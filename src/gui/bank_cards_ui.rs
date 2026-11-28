//! Saved bank accounts and native rendering for bank cards in chat.

use super::*;
use crate::bank_cards::{
    upper_no_diacritics, valid_bank_number, valid_holder_name, vietqr_payload, Bank, BankAccount,
    ReceivedBankCard, DEFAULT_DELETE_ERROR, SEND_UNCONFIRMED,
};
use crate::chat::BANK_CARD_PREVIEW;
use crate::session::is_trusted_zalo_url;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const TITLE: &str = "Gửi số tài khoản";
pub(super) const COPY_NUMBER: &str = "Sao chép số tài khoản";
pub(super) const SHOW_QR: &str = "Hiện mã VietQR";
const ROW_HEIGHT: f32 = 62.0;
// The credit-card glyph in the bundled Lucide font.
const CREDIT_CARD: &str = "\u{e0aa}";

#[derive(Default)]
pub(super) struct BankCardsState {
    target: Option<(String, bool)>,
    view: View,
    pending: Option<(u64, RequestKind)>,
    pending_send: Option<SendAttempt>,
    unconfirmed_sends: HashMap<(String, bool), HashSet<String>>,
    error: Option<(RequestKind, String)>,
    banks: Arc<[Bank]>,
    accounts: Vec<BankAccount>,
    has_multiple: bool,
    logos: HashMap<String, Option<TextureHandle>>,
    qr_textures: HashMap<(u32, String), TextureHandle>,
    received: HashMap<String, ReceivedCard>,
}

enum ReceivedCard {
    Loading,
    Ready {
        card: ReceivedBankCard,
        qr: Option<TextureHandle>,
    },
    Failed(String),
}

struct QrView {
    bank_name: String,
    number: String,
    holder_name: Option<String>,
    texture: Option<TextureHandle>,
    enlarged: bool,
    from_message: bool,
}

#[derive(Default)]
enum View {
    #[default]
    List,
    Edit(Form),
    Delete(BankAccount),
    Qr(QrView),
}

impl View {
    fn is_message_qr(&self) -> bool {
        matches!(
            self,
            Self::Qr(QrView {
                from_message: true,
                ..
            })
        )
    }
}

struct Form {
    ctx: egui::Context,
    id: Id,
    account: BankAccount,
    editing: bool,
    bank_picker_open: bool,
    bank_query: String,
    bank_touched: bool,
    number_touched: bool,
    holder_touched: bool,
    save_attempted: bool,
}

impl Form {
    fn new(ctx: &egui::Context, account: Option<BankAccount>) -> Self {
        static NEXT_FORM_ID: AtomicU64 = AtomicU64::new(1);
        let editing = account.is_some();
        let mut account = account.unwrap_or_default();
        account.holder_name = upper_no_diacritics(&account.holder_name);
        Self {
            ctx: ctx.clone(),
            id: Id::new((
                "bank_account_form",
                NEXT_FORM_ID.fetch_add(1, Ordering::Relaxed),
            )),
            account,
            editing,
            bank_picker_open: false,
            bank_query: String::new(),
            bank_touched: false,
            number_touched: false,
            holder_touched: false,
            save_attempted: false,
        }
    }

    fn clear_editor(&self, field: &str) {
        let id = self.id.with(field);
        self.ctx
            .data_mut(|data| data.remove::<egui::text_edit::TextEditState>(id));
        self.ctx.memory_mut(|memory| memory.surrender_focus(id));
    }

    fn close_picker(&mut self) {
        self.bank_picker_open = false;
        self.bank_touched = true;
        self.bank_query.clear();
        self.clear_editor("search");
    }
}

impl Drop for Form {
    fn drop(&mut self) {
        // egui keeps the undo buffer in context memory even after the form leaves the view.
        for field in ["number", "holder", "search"] {
            self.clear_editor(field);
        }
        self.ctx
            .memory_mut(|memory| memory.surrender_focus(self.id.with("picker")));
    }
}

struct SendAttempt {
    target: (String, bool),
    account_id: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestKind {
    Load,
    Save,
    Delete,
    Send,
}

enum Action {
    Close,
    Back,
    Load,
    Edit(Option<BankAccount>),
    ShowQr(BankAccount),
    ConfirmDelete(BankAccount),
    Save(BankAccount, bool),
    Delete(BankAccount),
    Send(BankAccount),
}

impl BankCardsState {
    fn close(&mut self) {
        self.target = None;
        self.view = View::List;
        self.error = None;
    }

    fn request(
        &mut self,
        uin: &str,
        kind: RequestKind,
        command: impl FnOnce(BankRequest) -> WorkerCmd,
    ) {
        if self.pending.is_some() {
            return;
        }
        static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let request = BankRequest {
            uin: uin.to_owned(),
            id,
        };
        if WORKER_TX
            .get()
            .is_some_and(|tx| tx.send(command(request)).is_ok())
        {
            self.error = None;
            self.pending = Some((id, kind));
        } else {
            self.error = Some((kind, "Không thể kết nối dịch vụ số tài khoản.".into()));
        }
    }

    fn load(&mut self, uin: &str) {
        self.request(uin, RequestKind::Load, |request| {
            WorkerCmd::LoadBankAccounts { request }
        });
    }

    fn complete_send(&mut self, result: &Result<BankCardsUpdate, String>) {
        let Some(SendAttempt { target, account_id }) = self.pending_send.take() else {
            return;
        };
        match result {
            Err(error) if error == SEND_UNCONFIRMED => {
                self.unconfirmed_sends
                    .entry(target)
                    .or_default()
                    .insert(account_id);
            }
            Ok(BankCardsUpdate::Sent) => {
                if let Some(accounts) = self.unconfirmed_sends.get_mut(&target) {
                    accounts.remove(&account_id);
                    if accounts.is_empty() {
                        self.unconfirmed_sends.remove(&target);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn on_logo(
        &mut self,
        ctx: &egui::Context,
        url: String,
        image: Option<egui::ColorImage>,
    ) {
        if let Some(texture) = self.logos.get_mut(&url) {
            *texture =
                image.map(|image| ctx.load_texture("bank_logo", image, TextureOptions::LINEAR));
        }
    }

    pub(super) fn on_received_card(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        result: Result<ReceivedBankCard, String>,
    ) {
        if let Some(entry @ ReceivedCard::Loading) = self.received.get_mut(key) {
            *entry = match result {
                Ok(card) => ReceivedCard::Ready {
                    qr: qr_texture(ctx, &card.qr_payload),
                    card,
                },
                Err(error) => ReceivedCard::Failed(error),
            };
        }
    }

    pub(super) fn received_card(&self, key: &str) -> Option<&ReceivedBankCard> {
        match self.received.get(key)? {
            ReceivedCard::Ready { card, .. } => Some(card),
            _ => None,
        }
    }

    fn load_received_card(&mut self, uin: &str, key: &str, url: &str) {
        let sent = WORKER_TX.get().is_some_and(|tx| {
            tx.send(WorkerCmd::LoadReceivedBankCard {
                uin: uin.into(),
                key: key.into(),
                url: url.into(),
            })
            .is_ok()
        });
        let entry = if sent {
            ReceivedCard::Loading
        } else {
            ReceivedCard::Failed("Không tải được thẻ ngân hàng.".into())
        };
        self.received.insert(key.into(), entry);
    }

    pub(super) fn render_received_card(
        &mut self,
        ui: &mut egui::Ui,
        pal: &ThemePalette,
        uin: &str,
        key: &str,
        url: &str,
    ) -> Option<messages::BubbleAction> {
        let retry = match self.received.get(key) {
            Some(ReceivedCard::Ready { card, qr }) => {
                let bounds = Rect::from_min_size(
                    ui.next_widget_position(),
                    Vec2::new(ui.available_width(), 94.0),
                );
                let logo = card
                    .logo
                    .as_deref()
                    .filter(|_| ui.is_rect_visible(bounds))
                    .and_then(|url| bank_logo_texture(&mut self.logos, uin, url));
                return render_received_details(ui, pal, card, logo, qr.as_ref());
            }
            Some(ReceivedCard::Failed(error)) => {
                ui.label(BANK_CARD_PREVIEW);
                ui.small(error);
                ui.button("Thử lại").clicked()
            }
            entry => {
                let row = ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Đang tải thẻ ngân hàng…");
                });
                entry.is_none() && ui.is_rect_visible(row.response.rect)
            }
        };
        if retry {
            self.load_received_card(uin, key, url);
        }
        None
    }

    pub(super) fn show_received_qr(&mut self, thread_id: &str, is_group: bool, key: &str) {
        if let Some(ReceivedCard::Ready { card, qr }) = self.received.get(key) {
            self.target = Some((thread_id.into(), is_group));
            self.view = View::Qr(QrView {
                bank_name: card.bank_name.clone(),
                number: card.number.clone(),
                holder_name: None,
                texture: qr.clone(),
                enlarged: false,
                from_message: true,
            });
            self.error = None;
        }
    }

    fn render_list(
        &mut self,
        ui: &mut egui::Ui,
        pal: &ThemePalette,
        uin: &str,
        height: f32,
    ) -> Option<Action> {
        if self
            .error
            .as_ref()
            .is_some_and(|(kind, _)| *kind == RequestKind::Load)
        {
            return ui.button("Thử lại").clicked().then_some(Action::Load);
        }
        if self
            .pending
            .is_some_and(|(_, kind)| kind == RequestKind::Load)
        {
            return None;
        }

        let enabled = self.pending.is_none();
        let retries = self
            .target
            .as_ref()
            .and_then(|target| self.unconfirmed_sends.get(target));
        if retries.is_some_and(|ids| {
            self.accounts
                .iter()
                .any(|account| ids.contains(&account.account_id))
        }) {
            ui.label(RichText::new(SEND_UNCONFIRMED).color(pal.warning));
        }
        let mut action = None;
        if self.accounts.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(18.0);
                ui.label(RichText::new("Chưa có số tài khoản").color(pal.text_secondary));
                ui.add_space(18.0);
            });
        } else {
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .max_height(height)
                .show_rows(ui, ROW_HEIGHT, self.accounts.len(), |ui, rows| {
                    for index in rows {
                        let account = &self.accounts[index];
                        let bank = bank_for(&self.banks, account.bin);
                        let label = bank_label(account, bank);
                        let logo =
                            bank_logo_texture(&mut self.logos, uin, bank_logo(account, bank));
                        let retry = retries.is_some_and(|ids| ids.contains(&account.account_id));
                        let selected = ui
                            .push_id(index, |ui| {
                                render_row(ui, pal, account, label, logo, enabled, retry)
                            })
                            .inner;
                        if selected.is_some() {
                            action = selected;
                        }
                    }
                });
        }
        ui.add_space(8.0);
        if ui
            .add_enabled(enabled, egui::Button::new("Thêm số tài khoản"))
            .clicked()
        {
            action = Some(Action::Edit(None));
        }
        action
    }
}

impl ZaloGuiApp {
    pub(super) fn bank_cards_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        pal: &ThemePalette,
        thread_id: &str,
        is_group: bool,
    ) {
        let response = ui.add_sized(
            [34.0, 34.0],
            egui::Button::new(
                RichText::new(CREDIT_CARD)
                    .size(18.0)
                    .color(pal.text_secondary),
            )
            .frame(false),
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, TITLE));
        if response
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(TITLE)
            .clicked()
        {
            self.sticker_picker_open = false;
            self.composer_focus_pending = false;
            ui.ctx().memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
            self.bank_cards.target = Some((thread_id.into(), is_group));
            self.bank_cards.view = View::List;
            self.bank_cards.load(&self.user_uin);
        }
    }

    pub(super) fn on_bank_cards_update(
        &mut self,
        request_id: u64,
        result: Result<BankCardsUpdate, String>,
    ) {
        let Some((id, kind)) = self.bank_cards.pending else {
            return;
        };
        if id != request_id {
            return;
        }
        self.bank_cards.pending = None;
        let mut manage_open =
            self.bank_cards.target.is_some() && !self.bank_cards.view.is_message_qr();
        if kind == RequestKind::Send {
            manage_open &= self
                .bank_cards
                .pending_send
                .as_ref()
                .is_some_and(|send| self.bank_cards.target.as_ref() == Some(&send.target));
            self.bank_cards.complete_send(&result);
        }
        match result {
            Ok(BankCardsUpdate::Loaded {
                banks,
                accounts,
                has_multiple,
            }) => {
                let state = &mut self.bank_cards;
                state.qr_textures.retain(|(bin, number), _| {
                    accounts
                        .iter()
                        .any(|a| a.bin == *bin && a.bank_number == *number)
                });
                state.logos.retain(|url, _| {
                    accounts
                        .iter()
                        .any(|account| bank_logo(account, bank_for(&banks, account.bin)) == url)
                        || state.received.values().any(|entry| {
                            matches!(entry, ReceivedCard::Ready { card, .. }
                                if card.logo.as_ref() == Some(url))
                        })
                });
                state.banks = banks;
                state.accounts = accounts;
                state.has_multiple = has_multiple;
            }
            Ok(update @ (BankCardsUpdate::Saved | BankCardsUpdate::Deleted)) => {
                self.show_toast(match update {
                    BankCardsUpdate::Saved => "Đã lưu số tài khoản",
                    _ => "Đã xóa số tài khoản",
                });
                if manage_open {
                    self.bank_cards.view = View::List;
                    self.bank_cards.load(&self.user_uin);
                }
            }
            Ok(BankCardsUpdate::Sent) => {
                if manage_open {
                    self.bank_cards.close();
                }
                self.show_toast("Đã gửi số tài khoản");
            }
            Err(error) if manage_open && error == SEND_UNCONFIRMED => {}
            Err(error) if manage_open => {
                self.bank_cards.error = Some((kind, error));
            }
            Err(error) if kind != RequestKind::Load => self.show_toast(error),
            Err(_) => {}
        }
    }

    pub(super) fn render_bank_cards(&mut self, ctx: &egui::Context, pal: &ThemePalette) {
        let Some((thread_id, is_group)) = &self.bank_cards.target else {
            return;
        };
        if self.contacts_view_open
            || !self
                .threads
                .get(self.selected_thread_index)
                .is_some_and(|thread| &thread.id == thread_id && thread.is_group == *is_group)
        {
            self.bank_cards.close();
            return;
        }

        let state = &mut self.bank_cards;
        let pending = state.pending.filter(|_| !state.view.is_message_qr());
        let writing = pending.is_some_and(|(_, kind)| kind != RequestKind::Load);
        let escape = !writing
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape));
        let escape_action = if escape {
            match &mut state.view {
                View::Edit(form) if form.bank_picker_open => {
                    form.close_picker();
                    ctx.memory_mut(|memory| memory.request_focus(form.id.with("picker")));
                    None
                }
                View::List => Some(Action::Close),
                view if view.is_message_qr() => Some(Action::Close),
                _ => Some(Action::Back),
            }
        } else {
            None
        };
        let screen = ctx.screen_rect();
        egui::Area::new(Id::new("bank_cards_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_black_alpha(100));
            });

        let title = match &state.view {
            View::List => TITLE,
            View::Edit(form) if form.editing => "Sửa số tài khoản",
            View::Edit(_) => "Thêm số tài khoản",
            View::Delete(_) => "Xóa số tài khoản",
            View::Qr(_) => "Mã VietQR",
        };
        let width = if matches!(state.view, View::Qr(QrView { enlarged: true, .. })) {
            452.0_f32
        } else {
            402.0
        };
        let width = width.min(screen.width() - 56.0).max(200.0);
        let height = (screen.height() - 205.0).clamp(96.0, 360.0);
        let action = egui::Window::new(title)
            .id(Id::new("bank_cards_dialog"))
            .title_bar(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .resizable(false)
            .movable(false)
            .max_height(screen.height() - 48.0)
            .vscroll(true)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(pal.bg_elevated)
                    .stroke(Stroke::new(1.0_f32, pal.border))
                    .inner_margin(Margin::same(16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                let mut close = false;
                ui.horizontal(|ui| {
                    ui.heading(title);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        close = ui
                            .add_enabled(!writing, egui::Button::new(lucide::X).frame(false))
                            .on_hover_text("Đóng")
                            .clicked();
                    });
                });
                ui.separator();
                if close {
                    return Some(Action::Close);
                }
                if let Some((_, error)) = &state.error {
                    ui.colored_label(pal.danger, error);
                }
                if let Some((_, kind)) = pending.filter(|(_, kind)| {
                    *kind != RequestKind::Save || !matches!(state.view, View::Edit(_))
                }) {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(match kind {
                            RequestKind::Load => "Đang tải số tài khoản…",
                            RequestKind::Save => "Đang lưu…",
                            RequestKind::Delete => "Đang xóa…",
                            RequestKind::Send => "Đang gửi…",
                        });
                    });
                }

                match &mut state.view {
                    View::List => state.render_list(ui, pal, &self.user_uin, height),
                    View::Edit(form) => render_form(
                        ui,
                        pal,
                        form,
                        &state.banks,
                        &mut state.logos,
                        &self.user_uin,
                        writing,
                    ),
                    View::Delete(account) => render_delete(ui, pal, account, writing),
                    View::Qr(qr) => render_qr(ui, pal, qr, screen.height()),
                }
            })
            .and_then(|response| response.inner)
            .flatten();

        match if escape { escape_action } else { action } {
            Some(Action::Close) => state.close(),
            Some(Action::Back) => {
                state.view = View::List;
                state.error = None;
            }
            Some(Action::Load) => state.load(&self.user_uin),
            Some(Action::Edit(account)) => {
                state.view = View::Edit(Form::new(ctx, account));
                state.error = None;
            }
            Some(Action::ShowQr(account)) => {
                let key = (account.bin, account.bank_number.clone());
                let texture = state.qr_textures.get(&key).cloned().or_else(|| {
                    if account.bin == 0 || !valid_bank_number(&account.bank_number) {
                        return None;
                    }
                    let payload = vietqr_payload(account.bin, &account.bank_number);
                    let texture = qr_texture(ctx, &payload)?;
                    state.qr_textures.insert(key, texture.clone());
                    Some(texture)
                });
                state.view = View::Qr(QrView {
                    bank_name: bank_label(&account, bank_for(&state.banks, account.bin)).into(),
                    number: account.bank_number,
                    holder_name: Some(account.holder_name),
                    texture,
                    enlarged: false,
                    from_message: false,
                });
                state.error = None;
            }
            Some(Action::ConfirmDelete(account)) => {
                if account.is_default && state.has_multiple {
                    state.error = Some((RequestKind::Delete, DEFAULT_DELETE_ERROR.into()));
                } else {
                    state.view = View::Delete(account);
                    state.error = None;
                }
            }
            Some(Action::Save(account, editing)) => {
                state.request(&self.user_uin, RequestKind::Save, |request| {
                    if editing {
                        WorkerCmd::UpdateBankAccount { request, account }
                    } else {
                        WorkerCmd::CreateBankAccount { request, account }
                    }
                })
            }
            Some(Action::Delete(account)) => {
                state.request(&self.user_uin, RequestKind::Delete, |request| {
                    WorkerCmd::DeleteBankAccount { request, account }
                })
            }
            Some(Action::Send(account)) => {
                if let Some(target) = state.target.clone().filter(|_| state.pending.is_none()) {
                    let (thread_id, is_group) = target.clone();
                    let account_id = account.account_id.clone();
                    state.request(&self.user_uin, RequestKind::Send, |request| {
                        WorkerCmd::SendBankCard {
                            request,
                            thread_id,
                            is_group,
                            account,
                        }
                    });
                    if state.pending.is_some() {
                        state.pending_send = Some(SendAttempt { target, account_id });
                    }
                }
            }
            None => {}
        }
    }
}

fn bank_for(banks: &[Bank], bin: u32) -> Option<&Bank> {
    banks.iter().find(|bank| bin != 0 && bank.bin == bin)
}

fn bank_label<'a>(account: &'a BankAccount, bank: Option<&'a Bank>) -> &'a str {
    [
        bank.map_or("", |bank| bank.short_name.as_str()),
        account.bank_name.as_str(),
        bank.map_or("", |bank| bank.name.as_str()),
    ]
    .into_iter()
    .find(|name| !name.is_empty())
    .unwrap_or("Ngân hàng")
}

fn bank_logo<'a>(account: &'a BankAccount, bank: Option<&'a Bank>) -> &'a str {
    if account.bank_logo.is_empty() {
        bank.map_or("", |bank| bank.logo.as_str())
    } else {
        &account.bank_logo
    }
}

fn bank_logo_texture<'a>(
    logos: &'a mut HashMap<String, Option<TextureHandle>>,
    uin: &str,
    url: &str,
) -> Option<&'a TextureHandle> {
    if !url.starts_with("https://") || !is_trusted_zalo_url(url) {
        return None;
    }
    logos
        .entry(url.to_owned())
        .or_insert_with(|| {
            if let Some(tx) = WORKER_TX.get() {
                let _ = tx.send(WorkerCmd::LoadBankLogo {
                    uin: uin.to_owned(),
                    url: url.to_owned(),
                });
            }
            None
        })
        .as_ref()
}

fn render_received_details(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    card: &ReceivedBankCard,
    logo: Option<&TextureHandle>,
    qr: Option<&TextureHandle>,
) -> Option<messages::BubbleAction> {
    let details = |ui: &mut egui::Ui| {
        if let Some(logo) = logo {
            ui.add(egui::Image::new(logo).max_size(Vec2::new(64.0, 28.0)));
        }
        ui.label(RichText::new(&card.bank_name).color(pal.text_secondary));
        ui.label(
            RichText::new(&card.number)
                .strong()
                .size(19.0)
                .color(pal.text_primary),
        );
    };
    let thumbnail = |ui: &mut egui::Ui, texture: &TextureHandle| {
        let response = ui.add(
            egui::Image::new(texture)
                .fit_to_exact_size(Vec2::splat(88.0))
                .sense(Sense::click()),
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, SHOW_QR));
        response
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(SHOW_QR)
            .clicked()
            && !ui.input(|input| input.modifiers.ctrl || input.modifiers.command)
    };

    let width = ui.available_width();
    let opened = match qr {
        Some(qr) if width >= 200.0 => {
            ui.horizontal(|ui| {
                let details_width = width - 88.0 - ui.spacing().item_spacing.x;
                ui.allocate_ui_with_layout(
                    Vec2::new(details_width, 0.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(details_width);
                        details(ui);
                    },
                );
                thumbnail(ui, qr)
            })
            .inner
        }
        Some(qr) => {
            details(ui);
            thumbnail(ui, qr)
        }
        None => {
            details(ui);
            false
        }
    };
    opened.then_some(messages::BubbleAction::ShowBankCardQr)
}

fn render_row(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    account: &BankAccount,
    label: &str,
    logo: Option<&TextureHandle>,
    enabled: bool,
    retry: bool,
) -> Option<Action> {
    let (rect, row) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    if row.hovered() || row.context_menu_opened() {
        ui.painter().rect_filled(rect, 8.0, pal.bg_hover);
    }
    let logo_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 28.0, rect.center().y),
        Vec2::splat(42.0),
    );
    ui.painter().rect_filled(logo_rect, 8.0, pal.bg_selected);
    if let Some(logo) = logo {
        let size = logo.size_vec2();
        let bounds = Rect::from_center_size(logo_rect.center(), size * (38.0 / size.max_elem()));
        ui.painter().image(
            logo.id(),
            bounds,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        let monogram: String = label.chars().take(9).collect();
        let size = (60.0 / monogram.chars().count().max(1) as f32).clamp(7.0, 11.0);
        ui.painter().text(
            logo_rect.center(),
            egui::Align2::CENTER_CENTER,
            monogram,
            FontId::proportional(size),
            pal.text_primary,
        );
    }

    let send_width = if retry { 96.0 } else { 34.0 };
    let send_rect = Rect::from_center_size(
        Pos2::new(rect.right() - 8.0 - send_width / 2.0, rect.center().y),
        Vec2::new(send_width, 34.0),
    );
    let send_label = if retry { "Vẫn gửi lại" } else { TITLE };
    let button_text = if retry {
        RichText::new(send_label)
    } else {
        RichText::new(lucide::SEND).size(17.0)
    };
    let send = ui
        .new_child(egui::UiBuilder::new().max_rect(send_rect))
        .add_enabled(
            enabled,
            egui::Button::new(button_text.color(Color32::WHITE))
                .fill(pal.primary)
                .min_size(send_rect.size()),
        );
    send.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, send_label));

    let mut details = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                Pos2::new(logo_rect.right() + 10.0, rect.top() + 9.0),
                Pos2::new(send_rect.left() - 8.0, rect.bottom() - 5.0),
            ))
            .layout(Layout::top_down(Align::Min)),
    );
    details.horizontal(|ui| {
        ui.add(
            egui::Label::new(RichText::new(&account.bank_number).size(15.0).strong()).truncate(),
        );
        if account.is_default {
            egui::Frame::none()
                .fill(pal.bg_selected)
                .rounding(5.0)
                .inner_margin(Margin::symmetric(5.0, 2.0))
                .show(ui, |ui| {
                    ui.label(RichText::new("Mặc định").size(10.5).color(pal.text_primary));
                });
        }
    });
    details.add(
        egui::Label::new(
            RichText::new(format!("{label} • {}", account.holder_name)).color(pal.text_secondary),
        )
        .truncate(),
    );

    let mut action = send
        .on_hover_text(send_label)
        .clicked()
        .then(|| Action::Send(account.clone()));
    if enabled {
        row.context_menu(|ui| {
            if ui.button(COPY_NUMBER).clicked() {
                ui.ctx().copy_text(account.bank_number.clone());
                ui.close_menu();
            }
            if ui.button(SHOW_QR).clicked() {
                action = Some(Action::ShowQr(account.clone()));
                ui.close_menu();
            }
            if ui.button("Sửa").clicked() {
                action = Some(Action::Edit(Some(account.clone())));
                ui.close_menu();
            }
            if ui.button("Xóa").clicked() {
                action = Some(Action::ConfirmDelete(account.clone()));
                ui.close_menu();
            }
        });
    }
    action
}

fn render_form(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    form: &mut Form,
    banks: &[Bank],
    logos: &mut HashMap<String, Option<TextureHandle>>,
    uin: &str,
    saving: bool,
) -> Option<Action> {
    let number_id = form.id.with("number");
    let holder_id = form.id.with("holder");
    ui.add_space(8.0);
    let valid = ui
        .add_enabled_ui(!saving, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            render_bank_picker(ui, pal, form, banks, logos, uin);

            ui.add_space(9.0);
            let number_ok = render_account_field(
                ui,
                pal,
                number_id,
                &mut form.account.bank_number,
                &mut form.number_touched,
                form.save_attempted,
                AccountField::Number,
            );
            ui.add_space(9.0);
            let holder_ok = render_account_field(
                ui,
                pal,
                holder_id,
                &mut form.account.holder_name,
                &mut form.holder_touched,
                form.save_attempted,
                AccountField::Holder,
            );
            form.account.bin != 0 && number_ok && holder_ok
        })
        .inner;

    ui.add_space(12.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let save_label = if saving {
            "Đang lưu…"
        } else if form.editing {
            "Lưu thay đổi"
        } else {
            "Lưu"
        };
        // The muted button can reveal validation hints, but only valid data can be saved.
        let primary = valid || saving;
        let save = ui.add_enabled(
            !saving,
            egui::Button::new(RichText::new(save_label).strong().color(if primary {
                Color32::WHITE
            } else {
                pal.text_secondary
            }))
            .fill(if primary { pal.primary } else { pal.bg_hover })
            .stroke(Stroke::NONE)
            .min_size(Vec2::new(
                if saving || form.editing { 128.0 } else { 76.0 },
                34.0,
            ))
            .rounding(8.0),
        );
        if saving {
            egui::Spinner::new()
                .size(14.0)
                .color(Color32::WHITE)
                .paint_at(
                    ui,
                    Rect::from_center_size(
                        Pos2::new(save.rect.left() + 18.0, save.rect.center().y),
                        Vec2::splat(14.0),
                    ),
                );
        }
        let cancel = ui.add_enabled(
            !saving,
            egui::Button::new(RichText::new("Hủy").color(pal.text_primary))
                .fill(pal.bg_elevated)
                .stroke(Stroke::new(1.0_f32, pal.border))
                .min_size(Vec2::new(64.0, 34.0))
                .rounding(8.0),
        );
        if save.clicked() {
            form.save_attempted = true;
            ui.ctx().request_repaint();
            valid.then(|| Action::Save(form.account.clone(), form.editing))
        } else if cancel.clicked() {
            Some(Action::Back)
        } else {
            None
        }
    })
    .inner
}

enum AccountField {
    Number,
    Holder,
}

fn render_account_field(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    id: Id,
    text: &mut String,
    touched: &mut bool,
    save_attempted: bool,
    field: AccountField,
) -> bool {
    let (label, hint, limit, error) = match field {
        AccountField::Number => (
            "Số tài khoản",
            "Nhập số tài khoản",
            128,
            "Số tài khoản phải có 1–19 ký tự chữ hoặc số, không có khoảng trắng.",
        ),
        AccountField::Holder => (
            "Tên chủ tài khoản",
            "VD: NGUYEN VAN A",
            256,
            "Tên phải có 5–50 ký tự chữ hoặc số, mỗi từ cách nhau một dấu cách.",
        ),
    };
    let label = form_label(ui, pal, label);
    let input = FormInput::show(
        ui,
        pal,
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text(RichText::new(hint).color(pal.text_muted))
            .char_limit(limit),
        None,
    );
    let valid = match field {
        AccountField::Number => valid_bank_number(text),
        AccountField::Holder => {
            if input.response.changed() {
                *text = upper_no_diacritics(text);
            }
            valid_holder_name(text)
        }
    };
    *touched |= input.response.changed() || input.response.lost_focus();
    let invalid = !valid && (*touched || save_attempted);
    input.paint(ui, pal, invalid);
    input.response.labelled_by(label.id);

    if matches!(field, AccountField::Number) {
        if invalid {
            validation_hint(ui, pal, error);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{}/19", text.chars().count()))
                    .size(11.0)
                    .color(if invalid {
                        pal.danger
                    } else {
                        pal.text_secondary
                    }),
            );
        });
    } else if invalid {
        validation_hint(ui, pal, error);
    }
    valid
}

fn form_label(ui: &mut egui::Ui, pal: &ThemePalette, label: &str) -> egui::Response {
    ui.label(
        RichText::new(label)
            .size(12.0)
            .strong()
            .color(pal.text_secondary),
    )
}

fn validation_hint(ui: &mut egui::Ui, pal: &ThemePalette, hint: &str) {
    ui.label(RichText::new(hint).size(11.0).color(pal.danger));
}

struct FormInput {
    response: egui::Response,
    rect: Rect,
    background: egui::layers::ShapeIdx,
}

impl FormInput {
    fn show(
        ui: &mut egui::Ui,
        pal: &ThemePalette,
        edit: egui::TextEdit<'_>,
        icon: Option<&str>,
    ) -> Self {
        let background = ui.painter().add(egui::Shape::Noop);
        let mut margin = Margin::symmetric(12.0, 4.0);
        if icon.is_some() {
            margin.left = 34.0;
        }
        let response = ui.add(
            edit.frame(false)
                .interactive(ui.is_enabled())
                .margin(margin)
                .desired_width(f32::INFINITY)
                .min_size(Vec2::new(0.0, 36.0))
                .vertical_align(Align::Center)
                .font(FontId::proportional(13.5))
                .text_color(pal.text_primary),
        );
        let rect = response.rect + margin;
        if let Some(icon) = icon {
            ui.painter().text(
                Pos2::new(rect.left() + 17.0, rect.center().y),
                egui::Align2::CENTER_CENTER,
                icon,
                FontId::proportional(15.0),
                pal.text_secondary,
            );
        }
        Self {
            response,
            rect,
            background,
        }
    }

    fn paint(&self, ui: &egui::Ui, pal: &ThemePalette, invalid: bool) {
        let border = form_field_border(pal, self.response.has_focus(), invalid);
        ui.painter().set(
            self.background,
            egui::Shape::rect_filled(self.rect, 8.0, pal.bg_input),
        );
        ui.painter()
            .rect_stroke(self.rect, 8.0, Stroke::new(1.0_f32, border));
    }
}

fn form_field_border(pal: &ThemePalette, focused: bool, invalid: bool) -> Color32 {
    if invalid {
        pal.danger
    } else if focused {
        pal.primary
    } else {
        pal.border
    }
}

fn render_bank_picker(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    form: &mut Form,
    banks: &[Bank],
    logos: &mut HashMap<String, Option<TextureHandle>>,
    uin: &str,
) {
    let label = form_label(ui, pal, "Ngân hàng");
    let bank_selected = form.account.bin != 0;
    let bank = bank_for(banks, form.account.bin);
    let (short_name, full_name) = if bank_selected {
        (
            bank_label(&form.account, bank),
            bank.map_or(form.account.bank_name.as_str(), |bank| bank.name.as_str()),
        )
    } else {
        ("Chọn ngân hàng", "")
    };
    let logo = bank_logo_texture(logos, uin, bank_logo(&form.account, bank));
    let invalid = !bank_selected && (form.bank_touched || form.save_attempted);
    let picker = render_bank_choice(
        ui,
        pal,
        logo,
        (short_name, full_name),
        BankChoiceStyle::Picker {
            id: form.id.with("picker"),
            open: form.bank_picker_open,
            placeholder: !bank_selected,
            invalid,
        },
    )
    .labelled_by(label.id);
    if picker.clicked() {
        if form.bank_picker_open {
            form.close_picker();
        } else {
            form.bank_picker_open = true;
            ui.memory_mut(|memory| memory.request_focus(form.id.with("search")));
        }
    }
    if invalid {
        validation_hint(ui, pal, "Chọn ngân hàng.");
    }
    if !form.bank_picker_open {
        return;
    }

    let picked = egui::Frame::none()
        .fill(pal.bg_elevated)
        .stroke(Stroke::new(1.0_f32, pal.border))
        .rounding(8.0)
        .inner_margin(Margin::same(6.0))
        .show(ui, |ui| {
            const SEARCH_HINT: &str = "Tìm theo tên hoặc mã ngân hàng";
            let search = FormInput::show(
                ui,
                pal,
                egui::TextEdit::singleline(&mut form.bank_query)
                    .id(form.id.with("search"))
                    .hint_text(RichText::new(SEARCH_HINT).color(pal.text_muted))
                    .char_limit(200),
                Some(lucide::SEARCH),
            );
            search.paint(ui, pal, false);
            let query = upper_no_diacritics(form.bank_query.trim());
            let results: Vec<_> = banks
                .iter()
                .filter(|bank| bank.bin != 0 && bank_matches_query(bank, &query))
                .take(100)
                .collect();
            if results.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Không tìm thấy ngân hàng phù hợp").color(pal.text_secondary),
                );
                return None;
            }
            let height = (ui.ctx().screen_rect().height() - 277.0).clamp(64.0, 150.0);
            egui::ScrollArea::vertical()
                .id_salt(form.id.with("bank_results"))
                .max_height(height)
                .auto_shrink([false, true])
                .show_rows(ui, BANK_CHOICE_HEIGHT, results.len(), |ui, rows| {
                    let mut picked = None;
                    for index in rows {
                        let bank = results[index];
                        let logo = bank_logo_texture(logos, uin, &bank.logo);
                        let title = if bank.short_name.is_empty() {
                            &bank.name
                        } else {
                            &bank.short_name
                        };
                        if render_bank_choice(
                            ui,
                            pal,
                            logo,
                            (title, &bank.name),
                            BankChoiceStyle::Row {
                                selected: form.account.bin == bank.bin,
                            },
                        )
                        .clicked()
                        {
                            picked = Some(bank);
                        }
                    }
                    picked
                })
                .inner
        })
        .inner;
    if let Some(bank) = picked {
        form.account.bin = bank.bin;
        form.account.bank_name.clone_from(&bank.name);
        form.account.bank_logo.clone_from(&bank.logo);
        form.close_picker();
        picker.request_focus();
    }
}

fn bank_matches_query(bank: &Bank, query: &str) -> bool {
    query.is_empty()
        || [&bank.short_name, &bank.name, &bank.search_keyword]
            .into_iter()
            .any(|text| upper_no_diacritics(text).contains(query))
        || (query.bytes().all(|byte| byte.is_ascii_digit()) && bank.bin.to_string().contains(query))
}

const BANK_CHOICE_HEIGHT: f32 = 44.0;

enum BankChoiceStyle {
    Picker {
        id: Id,
        open: bool,
        placeholder: bool,
        invalid: bool,
    },
    Row {
        selected: bool,
    },
}

fn render_bank_choice(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    logo: Option<&TextureHandle>,
    (title, subtitle): (&str, &str),
    style: BankChoiceStyle,
) -> egui::Response {
    let height = match style {
        BankChoiceStyle::Picker { .. } => 40.0,
        BankChoiceStyle::Row { .. } => BANK_CHOICE_HEIGHT,
    };
    let (auto_id, rect) = ui.allocate_space(Vec2::new(ui.available_width(), height));
    let (id, icon, icon_color, placeholder) = match style {
        BankChoiceStyle::Picker {
            id,
            open,
            placeholder,
            ..
        } => (
            id,
            if open {
                lucide::CHEVRON_UP
            } else {
                lucide::CHEVRON_DOWN
            },
            pal.text_secondary,
            placeholder,
        ),
        BankChoiceStyle::Row { selected } => (
            auto_id,
            if selected { lucide::CHECK } else { "" },
            pal.primary,
            false,
        ),
    };
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title)
    });
    let (fill, stroke) = match style {
        BankChoiceStyle::Picker { open, invalid, .. } => (
            if response.hovered() {
                pal.bg_hover
            } else {
                pal.bg_input
            },
            Stroke::new(
                1.0_f32,
                form_field_border(pal, open || response.has_focus(), invalid),
            ),
        ),
        BankChoiceStyle::Row { selected } => (
            if response.hovered() {
                pal.bg_hover
            } else if selected {
                pal.bg_selected
            } else {
                Color32::TRANSPARENT
            },
            if response.has_focus() {
                Stroke::new(1.0_f32, pal.primary)
            } else {
                Stroke::NONE
            },
        ),
    };

    ui.painter().rect(rect, 8.0, fill, stroke);
    let logo_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 22.0, rect.center().y),
        Vec2::splat(24.0),
    );
    if let Some(texture) = logo {
        let size = texture.size_vec2();
        let bounds = Rect::from_center_size(logo_rect.center(), size * (24.0 / size.max_elem()));
        ui.painter().image(
            texture.id(),
            bounds,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        ui.painter().rect_filled(logo_rect, 5.0, pal.bg_selected);
        ui.painter().text(
            logo_rect.center(),
            egui::Align2::CENTER_CENTER,
            CREDIT_CARD,
            FontId::proportional(14.0),
            pal.text_secondary,
        );
    }
    if !icon.is_empty() {
        ui.painter().text(
            Pos2::new(rect.right() - 17.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icon,
            FontId::proportional(15.0),
            icon_color,
        );
    }

    let two_lines = !subtitle.is_empty() && title != subtitle;
    let mut text = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                Pos2::new(logo_rect.right() + 9.0, rect.top()),
                Pos2::new(rect.right() - 32.0, rect.bottom()),
            ))
            .layout(Layout::top_down(Align::Min)),
    );
    text.spacing_mut().item_spacing.y = 0.0;
    text.add_space(if two_lines { 5.0 } else { 13.0 });
    let (title_pos, galley, _) = egui::Label::new(RichText::new(title).size(13.5))
        .truncate()
        .selectable(false)
        .layout_in_ui(&mut text);
    if !placeholder {
        // The bundled font is regular. A one-pixel offset adds weight without changing fonts.
        let offset = Vec2::new(1.0 / ui.ctx().pixels_per_point(), 0.0);
        text.painter().galley_with_override_text_color(
            title_pos + offset,
            galley.clone(),
            pal.text_primary.gamma_multiply(0.5),
        );
    }
    text.painter().galley_with_override_text_color(
        title_pos,
        galley,
        if placeholder {
            pal.text_secondary
        } else {
            pal.text_primary
        },
    );
    if two_lines {
        text.add(
            egui::Label::new(RichText::new(subtitle).size(11.0).color(pal.text_secondary))
                .truncate()
                .selectable(false),
        );
    }
    response
}

fn render_delete(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    account: &BankAccount,
    writing: bool,
) -> Option<Action> {
    ui.label(format!(
        "Xóa số tài khoản {} khỏi danh sách đã lưu?",
        account.bank_number
    ));
    ui.add_space(8.0);
    ui.add_enabled_ui(!writing, |ui| {
        ui.horizontal(|ui| {
            if ui.button("Hủy").clicked() {
                Some(Action::Back)
            } else if ui.button(RichText::new("Xóa").color(pal.danger)).clicked() {
                Some(Action::Delete(account.clone()))
            } else {
                None
            }
        })
        .inner
    })
    .inner
}

fn render_qr(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    qr: &mut QrView,
    viewport_height: f32,
) -> Option<Action> {
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(&qr.bank_name).strong());
        ui.label(RichText::new(&qr.number).strong().size(19.0));
        if let Some(holder) = &qr.holder_name {
            ui.label(RichText::new(holder).color(pal.text_secondary));
        }
        ui.add_space(8.0);
        if let Some(texture) = &qr.texture {
            let side = if qr.enlarged { 392.0_f32 } else { 240.0 };
            let side = side
                .min(ui.available_width())
                .min((viewport_height - 220.0).max(80.0));
            let image = egui::Image::new(texture)
                .fit_to_exact_size(Vec2::splat(side))
                .sense(Sense::click());
            if ui
                .add(image)
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(if qr.enlarged {
                    "Thu nhỏ mã VietQR"
                } else {
                    "Phóng to mã VietQR"
                })
                .clicked()
            {
                qr.enlarged = !qr.enlarged;
            }
        } else {
            ui.colored_label(pal.danger, "Không thể tạo mã VietQR cho số tài khoản này.");
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(COPY_NUMBER).clicked() {
                ui.ctx().copy_text(qr.number.clone());
            }
            if qr.from_message {
                ui.button("Đóng").clicked().then_some(Action::Close)
            } else {
                ui.button("Quay lại").clicked().then_some(Action::Back)
            }
        })
        .inner
    })
    .inner
}

fn qr_texture(ctx: &egui::Context, payload: &str) -> Option<TextureHandle> {
    let code = qrcode::QrCode::new(payload).ok()?;
    let width = code.width();
    let quiet_zone = 4;
    let side = width + quiet_zone * 2;
    let mut image = egui::ColorImage::new([side, side], Color32::WHITE);
    for (index, color) in code.to_colors().into_iter().enumerate() {
        if color == qrcode::Color::Dark {
            let x = index % width + quiet_zone;
            let y = index / width + quiet_zone;
            image.pixels[y * side + x] = Color32::BLACK;
        }
    }
    Some(ctx.load_texture("vietqr", image, TextureOptions::NEAREST))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_card_forms_use_fresh_ids_and_remove_editor_state_on_close_or_reset() {
        let ctx = egui::Context::default();
        let mut state = BankCardsState::default();
        let fields = ["number", "holder", "search"];

        let first = Form::new(&ctx, None);
        let first_ids = fields.map(|field| first.id.with(field));
        for id in first_ids {
            egui::TextEdit::store_state(&ctx, id, Default::default());
            assert!(egui::TextEdit::load_state(&ctx, id).is_some());
        }
        state.view = View::Edit(first);
        state.close();
        assert!(first_ids
            .iter()
            .all(|&id| egui::TextEdit::load_state(&ctx, id).is_none()));

        let second = Form::new(&ctx, None);
        let second_ids = fields.map(|field| second.id.with(field));
        for id in second_ids {
            assert!(!first_ids.contains(&id));
            egui::TextEdit::store_state(&ctx, id, Default::default());
        }
        state.view = View::Edit(second);
        state = BankCardsState::default();
        assert!(matches!(state.view, View::List));
        assert!(second_ids
            .iter()
            .all(|&id| egui::TextEdit::load_state(&ctx, id).is_none()));
    }

    #[test]
    fn bank_card_send_confirmation_survives_reopening_and_a_rejected_retry() {
        let target = ("thread-1".to_owned(), false);
        let mut state = BankCardsState {
            target: Some(target.clone()),
            ..Default::default()
        };
        let attempt = || SendAttempt {
            target: target.clone(),
            account_id: "1".into(),
        };
        state.pending_send = Some(attempt());
        state.complete_send(&Err(SEND_UNCONFIRMED.into()));
        state.close();
        assert!(state.unconfirmed_sends[&target].contains("1"));
        assert!(!state.unconfirmed_sends[&target].contains("2"));

        state.pending_send = Some(attempt());
        state.complete_send(&Err("Zalo từ chối yêu cầu.".into()));
        assert!(state.unconfirmed_sends[&target].contains("1"));

        state.pending_send = Some(attempt());
        state.complete_send(&Ok(BankCardsUpdate::Sent));
        assert!(!state.unconfirmed_sends.contains_key(&target));
    }

    #[test]
    fn bank_card_picker_rows_accept_clicks_on_the_bank_name() {
        let ctx = egui::Context::default();
        let pal = ThemePalette::light();
        let frame = |events| {
            let mut clicked = false;
            let mut name_pos = Pos2::ZERO;
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let row = render_bank_choice(
                            ui,
                            &pal,
                            None,
                            ("TEST", "Ngân hàng mẫu"),
                            BankChoiceStyle::Row { selected: false },
                        );
                        name_pos = row.rect.min + Vec2::new(48.0, 27.0);
                        clicked = row.clicked();
                    });
                },
            );
            (name_pos, clicked)
        };
        let (pos, _) = frame(Vec::new());
        let click = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(pos), click(true)]);
        let (_, clicked) = frame(vec![click(false)]);
        assert!(clicked);
    }
}
