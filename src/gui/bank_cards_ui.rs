//! Composer dialog for Zalo's saved bank accounts.

use super::*;
use crate::bank_cards::{
    upper_no_diacritics, valid_bank_number, valid_holder_name, vietqr_payload, Bank, BankAccount,
    DEFAULT_DELETE_ERROR,
};
use crate::session::is_trusted_zalo_url;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const TITLE: &str = "Gửi số tài khoản";
const COPY_NUMBER: &str = "Sao chép số tài khoản";
const ROW_HEIGHT: f32 = 62.0;
// The credit-card glyph in the bundled Lucide font.
const CREDIT_CARD: &str = "\u{e0aa}";

#[derive(Default)]
pub(super) struct BankCardsState {
    target: Option<(String, bool)>,
    view: View,
    pending: Option<(u64, RequestKind)>,
    error: Option<(RequestKind, String)>,
    banks: Arc<[Bank]>,
    accounts: Vec<BankAccount>,
    has_multiple: bool,
    logos: HashMap<String, Option<TextureHandle>>,
    qr_textures: HashMap<(u32, String), TextureHandle>,
}

#[derive(Default)]
enum View {
    #[default]
    List,
    Edit(Form),
    Delete(BankAccount),
    Qr {
        account: BankAccount,
        enlarged: bool,
    },
}

struct Form {
    account: BankAccount,
    editing: bool,
    bank_picker_open: bool,
    bank_query: String,
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
                        let selected = ui
                            .push_id(index, |ui| {
                                render_row(ui, pal, account, label, logo, enabled)
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
                self.bank_cards.view = View::List;
                if self.bank_cards.target.is_some() {
                    self.bank_cards.load(&self.user_uin);
                }
            }
            Ok(BankCardsUpdate::Sent) => {
                self.bank_cards.close();
                self.show_toast("Đã gửi số tài khoản");
            }
            Err(error) if self.bank_cards.target.is_some() => {
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
        let writing = state
            .pending
            .is_some_and(|(_, kind)| kind != RequestKind::Load);
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
            View::Qr { .. } => "Mã VietQR",
        };
        let width = if matches!(state.view, View::Qr { enlarged: true, .. }) {
            452.0_f32
        } else {
            402.0
        };
        let width = width.min(screen.width() - 56.0).max(200.0);
        let height = (screen.height() - 205.0).clamp(96.0, 360.0);
        let mut action = egui::Window::new(title)
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
                if let Some((_, kind)) = state.pending {
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
                    View::Edit(form) => render_form(ui, pal, form, &state.banks, writing, height),
                    View::Delete(account) => render_delete(ui, pal, account, writing),
                    View::Qr { account, enlarged } => {
                        let label = bank_label(account, bank_for(&state.banks, account.bin));
                        render_qr(
                            ui,
                            pal,
                            label,
                            account,
                            enlarged,
                            &mut state.qr_textures,
                            screen.height(),
                        )
                    }
                }
            })
            .and_then(|response| response.inner)
            .flatten();
        if !writing && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            action = Some(if matches!(state.view, View::List) {
                Action::Close
            } else {
                Action::Back
            });
        }

        match action {
            Some(Action::Close) => state.close(),
            Some(Action::Back) => {
                state.view = View::List;
                state.error = None;
            }
            Some(Action::Load) => state.load(&self.user_uin),
            Some(Action::Edit(account)) => {
                let editing = account.is_some();
                let mut account = account.unwrap_or_default();
                account.holder_name = upper_no_diacritics(&account.holder_name);
                state.view = View::Edit(Form {
                    account,
                    editing,
                    bank_picker_open: false,
                    bank_query: String::new(),
                });
                state.error = None;
            }
            Some(Action::ShowQr(account)) => {
                state.view = View::Qr {
                    account,
                    enlarged: false,
                };
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
                if let Some((thread_id, is_group)) = state.target.clone() {
                    state.request(&self.user_uin, RequestKind::Send, |request| {
                        WorkerCmd::SendBankCard {
                            request,
                            thread_id,
                            is_group,
                            account,
                        }
                    });
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

fn render_row(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    account: &BankAccount,
    label: &str,
    logo: Option<&TextureHandle>,
    enabled: bool,
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

    let send_rect = Rect::from_center_size(
        Pos2::new(rect.right() - 25.0, rect.center().y),
        Vec2::splat(34.0),
    );
    let send = ui
        .new_child(egui::UiBuilder::new().max_rect(send_rect))
        .add_enabled(
            enabled,
            egui::Button::new(RichText::new(lucide::SEND).size(17.0).color(Color32::WHITE))
                .fill(pal.primary)
                .min_size(send_rect.size()),
        );
    send.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, TITLE));

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
        .on_hover_text(TITLE)
        .clicked()
        .then(|| Action::Send(account.clone()));
    if enabled {
        row.context_menu(|ui| {
            if ui.button(COPY_NUMBER).clicked() {
                ui.ctx().copy_text(account.bank_number.clone());
                ui.close_menu();
            }
            if ui.button("Hiện mã VietQR").clicked() {
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
    writing: bool,
    height: f32,
) -> Option<Action> {
    ui.add_enabled_ui(!writing, |ui| {
        ui.label("Ngân hàng");
        let label = if form.account.bin == 0 {
            "Chọn ngân hàng"
        } else {
            bank_label(&form.account, bank_for(banks, form.account.bin))
        };
        let picker_clicked = ui.button(label).clicked();
        if picker_clicked {
            form.bank_picker_open = !form.bank_picker_open;
            form.bank_query.clear();
        }
        if form.bank_picker_open {
            let search = ui.add(
                egui::TextEdit::singleline(&mut form.bank_query)
                    .hint_text("Tìm ngân hàng")
                    .desired_width(f32::INFINITY),
            );
            if picker_clicked {
                search.request_focus();
            }
            let query = upper_no_diacritics(form.bank_query.trim());
            egui::ScrollArea::vertical()
                .id_salt("bank_picker")
                .max_height((height - 72.0).clamp(64.0, 150.0))
                .show(ui, |ui| {
                    let mut found = false;
                    for bank in banks.iter().filter(|bank| bank.bin != 0) {
                        let matches = query.is_empty()
                            || [&bank.short_name, &bank.name, &bank.search_keyword]
                                .iter()
                                .any(|name| upper_no_diacritics(name).contains(&query));
                        if matches {
                            found = true;
                            let label = if bank.short_name.is_empty() {
                                bank.name.clone()
                            } else {
                                format!("{} · {}", bank.short_name, bank.name)
                            };
                            if ui
                                .selectable_label(form.account.bin == bank.bin, label)
                                .clicked()
                            {
                                form.account.bin = bank.bin;
                                form.account.bank_name.clone_from(&bank.name);
                                form.account.bank_logo.clone_from(&bank.logo);
                                form.bank_picker_open = false;
                            }
                        }
                    }
                    if !found {
                        ui.label("Không tìm thấy ngân hàng.");
                    }
                });
        }

        ui.add_space(4.0);
        ui.label("Số tài khoản");
        ui.add(
            egui::TextEdit::singleline(&mut form.account.bank_number)
                .char_limit(128)
                .desired_width(f32::INFINITY),
        );
        let number_ok = valid_bank_number(&form.account.bank_number);
        if !number_ok && !form.account.bank_number.is_empty() {
            ui.colored_label(
                pal.danger,
                "Số tài khoản phải có 1–19 ký tự chữ hoặc số, không có khoảng trắng.",
            );
        }

        ui.add_space(4.0);
        ui.label("Tên chủ tài khoản");
        if ui
            .add(
                egui::TextEdit::singleline(&mut form.account.holder_name)
                    .char_limit(256)
                    .desired_width(f32::INFINITY),
            )
            .changed()
        {
            form.account.holder_name = upper_no_diacritics(&form.account.holder_name);
        }
        let name_ok = valid_holder_name(&form.account.holder_name);
        if !name_ok && !form.account.holder_name.is_empty() {
            ui.colored_label(
                pal.danger,
                "Tên phải có 5–50 ký tự chữ hoặc số, mỗi từ cách nhau một dấu cách.",
            );
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let save = ui.add_enabled(
                form.account.bin != 0 && number_ok && name_ok,
                egui::Button::new(RichText::new("Lưu").color(Color32::WHITE)).fill(pal.primary),
            );
            if save.clicked() {
                Some(Action::Save(form.account.clone(), form.editing))
            } else if ui.button("Quay lại").clicked() {
                Some(Action::Back)
            } else {
                None
            }
        })
        .inner
    })
    .inner
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
    label: &str,
    account: &BankAccount,
    enlarged: &mut bool,
    textures: &mut HashMap<(u32, String), TextureHandle>,
    viewport_height: f32,
) -> Option<Action> {
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(label).strong());
        ui.label(RichText::new(&account.bank_number).strong().size(19.0));
        ui.label(RichText::new(&account.holder_name).color(pal.text_secondary));
        ui.add_space(8.0);
        let key = (account.bin, account.bank_number.clone());
        if !textures.contains_key(&key) {
            if let Some(texture) = qr_texture(ui.ctx(), account) {
                textures.insert(key.clone(), texture);
            }
        }
        if let Some(texture) = textures.get(&key) {
            let side = if *enlarged { 392.0_f32 } else { 240.0 };
            let side = side
                .min(ui.available_width())
                .min((viewport_height - 220.0).max(80.0));
            let image = egui::Image::new(texture)
                .fit_to_exact_size(Vec2::splat(side))
                .sense(Sense::click());
            if ui
                .add(image)
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(if *enlarged {
                    "Thu nhỏ mã VietQR"
                } else {
                    "Phóng to mã VietQR"
                })
                .clicked()
            {
                *enlarged = !*enlarged;
            }
        } else {
            ui.colored_label(pal.danger, "Không thể tạo mã VietQR cho số tài khoản này.");
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(COPY_NUMBER).clicked() {
                ui.ctx().copy_text(account.bank_number.clone());
            }
            ui.button("Quay lại").clicked().then_some(Action::Back)
        })
        .inner
    })
    .inner
}

fn qr_texture(ctx: &egui::Context, account: &BankAccount) -> Option<TextureHandle> {
    if account.bin == 0 || !valid_bank_number(&account.bank_number) {
        return None;
    }
    let code = qrcode::QrCode::new(vietqr_payload(account.bin, &account.bank_number)).ok()?;
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
