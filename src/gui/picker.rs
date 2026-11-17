//! Sticker / Emoji / GIF picker: a floating panel above the composer, modelled on Zalo Web.
//!
//! Measured on chat.zalo.me (2026-11-17, BrowserSkill): the panel is about 344x424 px with three
//! tabs (STICKER, EMOJI, GIF) and an expand button; the sticker tab has a search pill, a scrolling
//! grid whose first section is "Gần đây" (recent), and a bottom bar with the recent clock, pack
//! icons, scroll arrows and a "+" button that opens the sticker store. Picking an emoji only
//! inserts it into the draft and keeps the panel open; picking a sticker or GIF sends it.
//!
//! All network work stays in the worker (`WorkerCmd::SearchStickers`, `SearchGifs`,
//! `FetchStickerImage`, `RasterizeEmoji`); this file only renders and sends commands.

use super::messages::system_emoji_texture;
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PickerTab {
    Sticker,
    Emoji,
    Gif,
}

/// How many stickers the "Gần đây" section remembers.
pub(super) const RECENT_STICKER_CAP: usize = 16;
/// How many emoji the emoji tab's "Gần đây" section remembers.
pub(super) const RECENT_EMOJI_CAP: usize = 24;
/// Edge of one emoji cell in the emoji tab.
const EMOJI_CELL: f32 = 38.0;
const PANEL_W: f32 = 344.0;
const PANEL_H: f32 = 424.0;
const PANEL_W_EXPANDED: f32 = 520.0;
const PANEL_H_EXPANDED: f32 = 600.0;
const HEADER_H: f32 = 40.0;
const SEARCH_H: f32 = 36.0;
const FOOTER_H: f32 = 46.0;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(200);

/// Unicode emoji shown in the emoji tab, grouped like the system keyboard.
pub(super) const EMOJI_GROUPS: &[(&str, &[&str])] = &[
    (
        "Mặt cười & cảm xúc",
        &[
            "😀", "😃", "😄", "😁", "😆", "😅", "🤣", "😂", "🙂", "🙃", "😉", "😊", "😇", "🥰",
            "😍", "🤩", "😘", "😗", "😚", "😋", "😛", "😜", "🤪", "😝", "🤗", "🤭", "🤫", "🤔",
            "😐", "😑", "😶", "😏", "😒", "🙄", "😬", "😌", "😔", "😪", "😴", "😷", "🤒", "🤕",
            "🤢", "🤮", "🥵", "🥶", "🥴", "😵", "🤯", "🤠", "🥳", "😎", "🤓", "😕", "😟", "🙁",
            "😮", "😯", "😲", "😳", "🥺", "😦", "😧", "😨", "😰", "😥", "😢", "😭", "😱", "😖",
            "😣", "😞", "😓", "😩", "😫", "🥱", "😤", "😡", "😠", "🤬", "😈", "👿", "💀", "💩",
            "🤡", "👻", "👽", "🤖",
        ],
    ),
    (
        "Cử chỉ & con người",
        &[
            "👍", "👎", "👌", "✌️", "🤞", "🤟", "🤘", "🤙", "👈", "👉", "👆", "👇", "☝️", "👋",
            "🤚", "🖐️", "✋", "🖖", "👏", "🙌", "👐", "🤲", "🤝", "🙏", "✍️", "💪", "🦾", "🧠",
            "👀", "👁️", "👅", "👄", "👶", "🧒", "👦", "👧", "🧑", "👨", "👩", "🧓", "👴", "👵",
            "🙋", "🙆", "🙅", "🤦", "🤷", "💁", "🙇",
        ],
    ),
    (
        "Trái tim & biểu tượng",
        &[
            "❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "🤎", "💔", "❣️", "💕", "💞", "💓",
            "💗", "💖", "💘", "💝", "💯", "💢", "💥", "💫", "💦", "💨", "🔥", "✨", "⭐", "🌟",
            "⚡", "☀️", "🌈", "❄️", "✅", "❌", "❓", "❗", "⭕", "🚫", "♻️", "🆗",
        ],
    ),
    (
        "Động vật & thiên nhiên",
        &[
            "🐶", "🐱", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯", "🦁", "🐮", "🐷", "🐸",
            "🐵", "🙈", "🙉", "🙊", "🐔", "🐧", "🐦", "🐤", "🦆", "🦅", "🦉", "🐺", "🐴", "🦄",
            "🐝", "🦋", "🐌", "🐞", "🐢", "🐍", "🐙", "🦀", "🐠", "🐬", "🐳", "🦈", "🌸", "🌹",
            "🌻", "🌼", "🌷", "🌳", "🌴", "🍀", "🍁", "🌙",
        ],
    ),
    (
        "Đồ ăn & thức uống",
        &[
            "🍎", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🍒", "🍑", "🥭", "🍍", "🥥", "🥝", "🍅",
            "🥑", "🌽", "🥕", "🍞", "🧀", "🍳", "🥓", "🍔", "🍟", "🍕", "🌭", "🥪", "🌮", "🍜",
            "🍝", "🍣", "🍤", "🍙", "🍚", "🍦", "🍰", "🎂", "🍩", "🍪", "🍫", "🍿", "☕", "🍵",
            "🧋", "🥤", "🍺", "🍻", "🥂", "🍷",
        ],
    ),
    (
        "Hoạt động & đồ vật",
        &[
            "⚽", "🏀", "🏈", "⚾", "🎾", "🏐", "🎱", "🏓", "🎯", "🎮", "🎲", "🎵", "🎶", "🎤",
            "🎧", "🎸", "🎹", "🎬", "📷", "📱", "💻", "⌚", "💡", "🔑", "🔒", "🎁", "🎉", "🎊",
            "🎈", "🏆", "🥇", "📌", "📎", "✏️", "📚", "💰", "💳", "🚗", "✈️", "🚀",
        ],
    ),
];

/// Move `item` to the front of `list`, dropping any earlier copy and keeping at most `cap`.
pub(super) fn push_recent<T, K: PartialEq>(
    list: &mut Vec<T>,
    item: T,
    cap: usize,
    key: impl Fn(&T) -> K,
) {
    let k = key(&item);
    list.retain(|x| key(x) != k);
    list.insert(0, item);
    list.truncate(cap);
}

/// Insert `ins` at char index `at` (clamped). Returns the new text and the caret char index
/// right after the insertion.
pub(super) fn insert_at_char(text: &str, at: usize, ins: &str) -> (String, usize) {
    let total = text.chars().count();
    let at = at.min(total);
    let byte = text
        .char_indices()
        .nth(at)
        .map(|(b, _)| b)
        .unwrap_or(text.len());
    let mut out = String::with_capacity(text.len() + ins.len());
    out.push_str(&text[..byte]);
    out.push_str(ins);
    out.push_str(&text[byte..]);
    (out, at + ins.chars().count())
}

fn panel_size(expanded: bool) -> Vec2 {
    if expanded {
        Vec2::new(PANEL_W_EXPANDED, PANEL_H_EXPANDED)
    } else {
        Vec2::new(PANEL_W, PANEL_H)
    }
}

/// Columns of the sticker grid for a panel of the given inner width.
fn sticker_cols(inner_w: f32) -> usize {
    ((inner_w / 78.0).floor() as usize).clamp(3, 8)
}

fn emoji_cols(inner_w: f32) -> usize {
    ((inner_w / EMOJI_CELL).floor() as usize).clamp(6, 14)
}

fn sticker_image_url(item: &crate::chat::StickerItem) -> String {
    if item.cate_id == 0 && (1..=30).contains(&item.sticker_id) {
        format!(
            "https://stc-chat.zdn.vn/images/stickers/default/thumb/{}.png",
            item.sticker_id
        )
    } else {
        format!(
            "https://api.zaloapp.com/api/emoticon/sticker/webpc?eid={}&size=130",
            item.sticker_id
        )
    }
}

fn draw_expand_icon(p: &egui::Painter, c: Pos2, size: f32, col: Color32, expanded: bool) {
    let h = size / 2.0;
    let head = size * 0.34;
    let st = Stroke::new(1.6_f32, col);
    // Two opposite diagonal arrows: heads on the outer corners when collapsed (expand),
    // heads near the centre when expanded (shrink).
    for s in [1.0_f32, -1.0] {
        let corner = Pos2::new(c.x + s * h, c.y - s * h);
        let near = Pos2::new(c.x + s * h * 0.15, c.y - s * h * 0.15);
        let tip = if expanded { near } else { corner };
        let tail = if expanded { corner } else { near };
        p.line_segment([tail, tip], st);
        let k = if expanded { -1.0 } else { 1.0 };
        p.line_segment([tip, Pos2::new(tip.x - s * k * head, tip.y)], st);
        p.line_segment([tip, Pos2::new(tip.x, tip.y + s * k * head)], st);
    }
}

fn draw_chevron(p: &egui::Painter, c: Pos2, left: bool, col: Color32) {
    let d = if left { 1.0 } else { -1.0 };
    let st = Stroke::new(2.0_f32, col);
    p.line_segment(
        [
            Pos2::new(c.x - 3.0 * d, c.y - 6.0),
            Pos2::new(c.x + 3.0 * d, c.y),
        ],
        st,
    );
    p.line_segment(
        [
            Pos2::new(c.x + 3.0 * d, c.y),
            Pos2::new(c.x - 3.0 * d, c.y + 6.0),
        ],
        st,
    );
}

fn draw_clock(p: &egui::Painter, c: Pos2, r: f32, col: Color32) {
    let st = Stroke::new(1.8_f32, col);
    p.circle_stroke(c, r, st);
    p.line_segment([c, Pos2::new(c.x, c.y - r * 0.6)], st);
    p.line_segment([c, Pos2::new(c.x + r * 0.45, c.y + r * 0.2)], st);
}

fn draw_plus(p: &egui::Painter, c: Pos2, r: f32, col: Color32) {
    let st = Stroke::new(2.0_f32, col);
    p.line_segment([Pos2::new(c.x - r, c.y), Pos2::new(c.x + r, c.y)], st);
    p.line_segment([Pos2::new(c.x, c.y - r), Pos2::new(c.x, c.y + r)], st);
}

impl ZaloGuiApp {
    /// Open the picker on a given tab.
    pub(super) fn open_picker(&mut self, tab: PickerTab) {
        self.sticker_picker_open = true;
        self.picker_tab = tab;
    }

    /// Put `text` into the draft at the caret and keep typing focus in the composer.
    fn insert_into_composer(&mut self, ctx: &egui::Context, thread_id: &str, text: &str) {
        let composer_id = Id::new(("chat_composer_input", thread_id));
        let state = egui::TextEdit::load_state(ctx, composer_id);
        let len = self.input_message.chars().count();
        let at = state
            .as_ref()
            .and_then(|s| s.cursor.char_range())
            .map(|r| r.primary.index.min(len))
            .unwrap_or(len);
        let (next_text, next_caret) = insert_at_char(&self.input_message, at, text);
        self.input_message = next_text;
        if let Some(mut st) = state {
            st.cursor.set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(next_caret),
            )));
            st.store(ctx, composer_id);
        }
        self.composer_focus_pending = true;
    }

    /// Floating picker panel. Call after the composer is laid out so it can anchor to it.
    pub(super) fn render_picker_panel(
        &mut self,
        ctx: &egui::Context,
        composer_rect: Rect,
        thread_id: &str,
        is_group: bool,
        pal: &ThemePalette,
    ) {
        if self.picker_tab == PickerTab::Sticker
            && self.sticker_store_packs.is_empty()
            && !self.sticker_store_loading
        {
            self.sticker_store_loading = true;
            if let Some(tx) = WORKER_TX.get() {
                let _ = tx.send(WorkerCmd::FetchStickerStore);
            }
        }

        let size = panel_size(self.picker_expanded);
        let inner_w = size.x - 24.0;
        let area = egui::Area::new(Id::new("composer_picker_panel"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(Pos2::new(
                composer_rect.left() + 8.0,
                composer_rect.top() - 6.0,
            ))
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(pal.bg_elevated)
                    .stroke(Stroke::new(1.0_f32, pal.border))
                    .rounding(Rounding::same(10.0))
                    .shadow(egui::epaint::Shadow {
                        offset: Vec2::new(0.0, 4.0),
                        blur: 18.0,
                        spread: 0.0,
                        color: Color32::from_black_alpha(if pal.is_dark { 120 } else { 40 }),
                    })
                    .inner_margin(Margin::symmetric(12.0, 0.0))
                    .show(ui, |ui| {
                        ui.set_width(inner_w);
                        ui.set_height(size.y);
                        ui.spacing_mut().item_spacing = Vec2::new(0.0, 0.0);
                        self.render_picker_header(ui, inner_w, pal);

                        let show_search = self.picker_tab != PickerTab::Emoji;
                        let footer = if self.picker_tab == PickerTab::Sticker {
                            FOOTER_H
                        } else {
                            0.0
                        };
                        let mut body_h = size.y - HEADER_H - footer - 6.0;
                        if show_search {
                            self.render_picker_search(ui, inner_w, pal);
                            body_h -= SEARCH_H + 12.0;
                        }
                        ui.add_space(6.0);
                        match self.picker_tab {
                            PickerTab::Sticker => {
                                self.render_picker_sticker_body(
                                    ui, inner_w, body_h, thread_id, is_group, pal,
                                );
                                self.render_picker_pack_bar(ui, inner_w, pal);
                            }
                            PickerTab::Emoji => self
                                .render_picker_emoji_body(ui, ctx, inner_w, body_h, thread_id, pal),
                            PickerTab::Gif => self.render_picker_gif_body(
                                ui, inner_w, body_h, thread_id, is_group, pal,
                            ),
                        }
                    });
            });

        // Esc or a press outside the panel and its toggle button closes it.
        let panel_rect = area.response.rect;
        let (esc, pressed_at) = ctx.input(|i| {
            (
                i.key_pressed(Key::Escape),
                if i.pointer.any_pressed() {
                    i.pointer.interact_pos()
                } else {
                    None
                },
            )
        });
        let outside = pressed_at
            .is_some_and(|p| !panel_rect.contains(p) && !self.picker_button_rect.contains(p));
        if (esc && !(self.ime_active || self.ime_just_committed)) || outside {
            self.sticker_picker_open = false;
        }
    }

    fn render_picker_header(&mut self, ui: &mut egui::Ui, inner_w: f32, pal: &ThemePalette) {
        let expand_w = 38.0;
        let tab_w = (inner_w - expand_w) / 3.0;
        ui.horizontal(|ui| {
            for (tab, label) in [
                (PickerTab::Sticker, "STICKER"),
                (PickerTab::Emoji, "EMOJI"),
                (PickerTab::Gif, "GIF"),
            ] {
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(tab_w, HEADER_H), Sense::click());
                let active = self.picker_tab == tab;
                let col = if active {
                    pal.primary
                } else if resp.hovered() {
                    pal.text_primary
                } else {
                    pal.text_secondary
                };
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(13.0),
                    col,
                );
                if active {
                    ui.painter().rect_filled(
                        Rect::from_min_size(
                            Pos2::new(rect.left() + 8.0, rect.bottom() - 2.0),
                            Vec2::new(rect.width() - 16.0, 2.0),
                        ),
                        Rounding::same(1.0),
                        pal.primary,
                    );
                }
                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    self.picker_tab = tab;
                }
            }
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(expand_w, HEADER_H), Sense::click());
            draw_expand_icon(
                ui.painter(),
                rect.center(),
                13.0,
                if resp.hovered() {
                    pal.text_primary
                } else {
                    pal.text_secondary
                },
                self.picker_expanded,
            );
            if resp
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(if self.picker_expanded {
                    "Thu gọn"
                } else {
                    "Mở rộng"
                })
                .clicked()
            {
                self.picker_expanded = !self.picker_expanded;
            }
        });
        let y = ui.cursor().top();
        ui.painter()
            .hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, pal.border));
    }

    fn render_picker_search(&mut self, ui: &mut egui::Ui, inner_w: f32, pal: &ThemePalette) {
        ui.add_space(6.0);
        let is_gif = self.picker_tab == PickerTab::Gif;
        let mut query = if is_gif {
            self.picker_gif_query.clone()
        } else {
            self.sticker_search_query.clone()
        };
        let hint = if is_gif {
            "Tìm kiếm GIF"
        } else {
            "Tìm kiếm sticker"
        };
        let mut cleared = false;
        let mut changed = false;
        egui::Frame::none()
            .fill(pal.bg_input)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(SEARCH_H / 2.0))
            .inner_margin(Margin::symmetric(12.0, 0.0))
            .show(ui, |ui| {
                ui.set_height(SEARCH_H);
                ui.set_width(inner_w - 24.0);
                ui.horizontal_centered(|ui| {
                    ui.label(
                        RichText::new(lucide::SEARCH)
                            .size(15.0)
                            .color(pal.text_muted),
                    );
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .hint_text(hint)
                            .frame(false)
                            .desired_width(inner_w - 24.0 - 60.0),
                    );
                    changed = resp.changed();
                    if !query.is_empty()
                        && ui
                            .add(
                                egui::Button::new(
                                    RichText::new(lucide::X).size(13.0).color(pal.text_muted),
                                )
                                .frame(false),
                            )
                            .on_hover_cursor(CursorIcon::PointingHand)
                            .clicked()
                    {
                        cleared = true;
                    }
                });
            });
        if cleared {
            query.clear();
            changed = true;
        }
        if is_gif {
            self.picker_gif_query = query;
        } else {
            if changed {
                self.last_sticker_search_edit_at = std::time::Instant::now();
            }
            self.sticker_search_query = query;
        }
        ui.add_space(6.0);
    }

    fn render_picker_sticker_body(
        &mut self,
        ui: &mut egui::Ui,
        inner_w: f32,
        body_h: f32,
        thread_id: &str,
        is_group: bool,
        pal: &ThemePalette,
    ) {
        let cols = sticker_cols(inner_w);
        let query = self.sticker_search_query.trim().to_string();

        // Debounced keyword search, same cadence as the old search tab.
        let mut trigger: Option<String> = None;
        if !query.is_empty() && query != self.last_sticker_search_dispatched {
            let elapsed = self.last_sticker_search_edit_at.elapsed();
            if elapsed >= SEARCH_DEBOUNCE {
                trigger = Some(query.clone());
            } else {
                ui.ctx()
                    .request_repaint_after(SEARCH_DEBOUNCE - elapsed + Duration::from_millis(10));
            }
        }
        if let Some(kw) = trigger {
            self.last_sticker_search_dispatched = kw.clone();
            self.sticker_search_loading = true;
            if let Some(tx) = WORKER_TX.get() {
                let _ = tx.send(WorkerCmd::SearchStickers { keyword: kw });
            }
        }

        egui::ScrollArea::vertical()
            .id_salt((
                "picker_sticker_scroll",
                self.sticker_tab_salt(),
                query.is_empty(),
            ))
            .max_height(body_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if !query.is_empty() {
                    if self.sticker_search_loading {
                        ui.add_space(36.0);
                        ui.vertical_centered(|ui| {
                            ui.spinner();
                        });
                    } else if self.sticker_search_results.is_empty() {
                        ui.add_space(36.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new("Không tìm thấy sticker phù hợp")
                                    .size(12.5)
                                    .color(pal.text_muted),
                            );
                        });
                    } else {
                        let items = self.sticker_search_results.clone();
                        self.render_sticker_cells(
                            ui, &items, cols, inner_w, thread_id, is_group, pal,
                        );
                    }
                    return;
                }
                match self.sticker_tab {
                    StickerTab::Store => {
                        self.render_picker_store(ui, inner_w, pal);
                    }
                    StickerTab::Pack(cid) => {
                        if let Some(items) = self.sticker_pack_items.get(&cid).cloned() {
                            self.render_sticker_cells(
                                ui, &items, cols, inner_w, thread_id, is_group, pal,
                            );
                        } else {
                            ui.add_space(36.0);
                            ui.vertical_centered(|ui| {
                                ui.spinner();
                            });
                        }
                    }
                    StickerTab::Onion => {
                        let items = preset_items(ONION_STICKERS);
                        self.render_sticker_cells(
                            ui, &items, cols, inner_w, thread_id, is_group, pal,
                        );
                    }
                    StickerTab::Popular | StickerTab::Search => {
                        let recent = self.settings.recent_stickers.clone();
                        if !recent.is_empty() {
                            section_title(ui, "Gần đây", pal);
                            self.render_sticker_cells(
                                ui, &recent, cols, inner_w, thread_id, is_group, pal,
                            );
                            ui.add_space(8.0);
                        }
                        section_title(ui, "Phổ biến", pal);
                        let items = preset_items(POPULAR_STICKERS);
                        self.render_sticker_cells(
                            ui, &items, cols, inner_w, thread_id, is_group, pal,
                        );
                    }
                }
            });
    }

    /// Fixed-column sticker grid (no scroll area of its own). A click sends the sticker.
    #[allow(clippy::too_many_arguments)]
    fn render_sticker_cells(
        &mut self,
        ui: &mut egui::Ui,
        items: &[crate::chat::StickerItem],
        cols: usize,
        inner_w: f32,
        thread_id: &str,
        is_group: bool,
        pal: &ThemePalette,
    ) {
        let gap = 4.0;
        let cell_w = ((inner_w - gap * (cols as f32 - 1.0)) / cols as f32).floor();
        let mut picked: Option<crate::chat::StickerItem> = None;
        for row in items.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
                for item in row {
                    let key = format!("stk_{}_{}", item.cate_id, item.sticker_id);
                    if !self.sticker_textures.contains_key(&key)
                        && self.loading_stickers.insert(key.clone())
                    {
                        if let Some(tx) = WORKER_TX.get() {
                            let _ = tx.send(WorkerCmd::FetchStickerImage {
                                key: key.clone(),
                                url: sticker_image_url(item),
                            });
                        }
                    }
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(cell_w), Sense::click());
                    if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, Rounding::same(8.0), pal.bg_hover);
                    }
                    if let Some(tex) = self.sticker_textures.get(&key) {
                        ui.painter().image(
                            tex.id(),
                            rect.shrink(5.0),
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    } else {
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            truncate_chars(&item.label, 8),
                            FontId::proportional(10.5),
                            pal.text_muted,
                        );
                    }
                    if resp
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(&item.label)
                        .clicked()
                    {
                        picked = Some(item.clone());
                    }
                }
            });
            ui.add_space(gap);
        }
        if let Some(item) = picked {
            self.sticker_picker_open = false;
            push_recent(
                &mut self.settings.recent_stickers,
                item.clone(),
                RECENT_STICKER_CAP,
                |s| (s.cate_id, s.sticker_id),
            );
            self.settings.save();
            if let Some(tx) = WORKER_TX.get() {
                let _ = tx.send(WorkerCmd::SendSticker {
                    to_id: thread_id.to_string(),
                    sticker_id: item.sticker_id,
                    cate_id: item.cate_id,
                    sticker_type: item.sticker_type,
                    is_group,
                });
            }
        }
    }

    /// Sticker store (all packs), shown in the body when "+" is chosen.
    fn render_picker_store(&mut self, ui: &mut egui::Ui, inner_w: f32, pal: &ThemePalette) {
        ui.add(
            egui::TextEdit::singleline(&mut self.sticker_store_filter)
                .hint_text("Lọc bộ sticker")
                .desired_width(inner_w),
        );
        ui.add_space(6.0);
        if self.sticker_store_loading && self.sticker_store_packs.is_empty() {
            ui.add_space(36.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
            });
            return;
        }
        let q = self.sticker_store_filter.trim().to_lowercase();
        let packs: Vec<crate::chat::StickerPackInfo> = self
            .sticker_store_packs
            .iter()
            .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q))
            .take(120)
            .cloned()
            .collect();
        let mut open_pack: Option<(u64, String)> = None;
        let card_w = ((inner_w - 8.0) / 2.0).floor();
        for row in packs.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                for pack in row {
                    let icon_key = format!("pack_ic_{}", pack.id);
                    if !pack.icon_url.is_empty()
                        && !self.sticker_textures.contains_key(&icon_key)
                        && self.loading_stickers.insert(icon_key.clone())
                    {
                        if let Some(tx) = WORKER_TX.get() {
                            let _ = tx.send(WorkerCmd::FetchStickerImage {
                                key: icon_key.clone(),
                                url: pack.icon_url.clone(),
                            });
                        }
                    }
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(card_w, 52.0), Sense::click());
                    ui.painter().rect_filled(
                        rect,
                        Rounding::same(8.0),
                        if resp.hovered() {
                            pal.bg_hover
                        } else {
                            pal.bg_input
                        },
                    );
                    let ic = Rect::from_min_size(
                        Pos2::new(rect.left() + 6.0, rect.top() + 6.0),
                        Vec2::splat(40.0),
                    );
                    if let Some(tex) = self.sticker_textures.get(&icon_key) {
                        ui.painter().image(
                            tex.id(),
                            ic,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    ui.painter().text(
                        Pos2::new(rect.left() + 52.0, rect.top() + 18.0),
                        egui::Align2::LEFT_CENTER,
                        truncate_chars(&pack.name, 14),
                        FontId::proportional(12.5),
                        pal.text_primary,
                    );
                    ui.painter().text(
                        Pos2::new(rect.left() + 52.0, rect.top() + 36.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{} nhãn", pack.total_image),
                        FontId::proportional(11.0),
                        pal.text_muted,
                    );
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        open_pack = Some((pack.id, pack.name.clone()));
                    }
                }
            });
            ui.add_space(8.0);
        }
        if let Some((cid, name)) = open_pack {
            self.open_sticker_pack(cid, name);
        }
    }

    /// Pin a pack to the bar, select it and request its stickers when not cached yet.
    fn open_sticker_pack(&mut self, cid: u64, name: String) {
        if !self.pinned_sticker_packs.iter().any(|(id, _)| *id == cid) {
            self.pinned_sticker_packs.insert(0, (cid, name.clone()));
        }
        self.sticker_tab = StickerTab::Pack(cid);
        if !self.sticker_pack_items.contains_key(&cid) && self.loading_sticker_packs.insert(cid) {
            if let Some(tx) = WORKER_TX.get() {
                let _ = tx.send(WorkerCmd::FetchStickerPack { cate_id: cid, name });
            }
        }
    }

    /// Bottom bar of the sticker tab: recent clock, pack icons (scrollable) and the store "+".
    fn render_picker_pack_bar(&mut self, ui: &mut egui::Ui, inner_w: f32, pal: &ThemePalette) {
        let y = ui.cursor().top();
        ui.painter()
            .hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, pal.border));
        let arrow_w = 22.0;
        let plus_w = 40.0;
        let scroll_w = inner_w - arrow_w * 2.0 - plus_w;
        let mut offset = self.picker_bar_offset;
        let mut new_tab: Option<StickerTab> = None;
        let mut open_pack: Option<(u64, String)> = None;
        let mut store_clicked = false;
        ui.horizontal(|ui| {
            let h = FOOTER_H;
            let (l_rect, l_resp) = ui.allocate_exact_size(Vec2::new(arrow_w, h), Sense::click());
            draw_chevron(ui.painter(), l_rect.center(), true, pal.text_secondary);
            if l_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                offset = (offset - 120.0).max(0.0);
            }
            let out = egui::ScrollArea::horizontal()
                .id_salt("picker_pack_bar")
                .horizontal_scroll_offset(offset)
                .max_width(scroll_w)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        // Recent / popular.
                        let active =
                            matches!(self.sticker_tab, StickerTab::Popular | StickerTab::Search);
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(36.0, h), Sense::click());
                        bar_highlight(ui, rect, active, resp.hovered(), pal);
                        draw_clock(
                            ui.painter(),
                            rect.center(),
                            9.0,
                            if active {
                                pal.primary
                            } else {
                                pal.text_secondary
                            },
                        );
                        if resp.on_hover_text("Gần đây").clicked() {
                            new_tab = Some(StickerTab::Popular);
                        }
                        // Built-in onion pack.
                        let onion_key = ONION_STICKERS
                            .first()
                            .map(|s| format!("stk_{}_{}", s.cate_id, s.sticker_id));
                        let active = self.sticker_tab == StickerTab::Onion;
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(36.0, h), Sense::click());
                        bar_highlight(ui, rect, active, resp.hovered(), pal);
                        if let Some(tex) = onion_key.and_then(|k| self.sticker_textures.get(&k)) {
                            ui.painter().image(
                                tex.id(),
                                Rect::from_center_size(rect.center(), Vec2::splat(28.0)),
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            draw_icon_sticker(
                                ui.painter(),
                                rect.center(),
                                16.0,
                                pal.text_secondary,
                            );
                        }
                        if resp.on_hover_text("Củ hành").clicked() {
                            new_tab = Some(StickerTab::Onion);
                        }
                        for (cid, name) in self.pinned_sticker_packs.clone() {
                            let icon_key = format!("pack_ic_{cid}");
                            if !self.sticker_textures.contains_key(&icon_key) {
                                if let Some(pack) =
                                    self.sticker_store_packs.iter().find(|p| p.id == cid)
                                {
                                    if !pack.icon_url.is_empty()
                                        && self.loading_stickers.insert(icon_key.clone())
                                    {
                                        if let Some(tx) = WORKER_TX.get() {
                                            let _ = tx.send(WorkerCmd::FetchStickerImage {
                                                key: icon_key.clone(),
                                                url: pack.icon_url.clone(),
                                            });
                                        }
                                    }
                                }
                            }
                            let active = self.sticker_tab == StickerTab::Pack(cid);
                            let (rect, resp) =
                                ui.allocate_exact_size(Vec2::new(36.0, h), Sense::click());
                            bar_highlight(ui, rect, active, resp.hovered(), pal);
                            if let Some(tex) = self.sticker_textures.get(&icon_key) {
                                ui.painter().image(
                                    tex.id(),
                                    Rect::from_center_size(rect.center(), Vec2::splat(28.0)),
                                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            } else {
                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    truncate_chars(&name, 2),
                                    FontId::proportional(11.0),
                                    pal.text_secondary,
                                );
                            }
                            if resp.on_hover_text(&name).clicked() {
                                open_pack = Some((cid, name));
                            }
                        }
                    });
                });
            offset = out.state.offset.x;
            let (r_rect, r_resp) = ui.allocate_exact_size(Vec2::new(arrow_w, h), Sense::click());
            draw_chevron(ui.painter(), r_rect.center(), false, pal.text_secondary);
            if r_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                offset += 120.0;
            }
            let (p_rect, p_resp) = ui.allocate_exact_size(Vec2::new(plus_w, h), Sense::click());
            let store_active = self.sticker_tab == StickerTab::Store;
            draw_plus(
                ui.painter(),
                p_rect.center(),
                7.0,
                if store_active || p_resp.hovered() {
                    pal.primary
                } else {
                    pal.text_secondary
                },
            );
            if p_resp
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text("Kho sticker")
                .clicked()
            {
                store_clicked = true;
            }
        });
        self.picker_bar_offset = offset;
        if let Some(tab) = new_tab {
            self.sticker_tab = tab;
            self.sticker_search_query.clear();
        }
        if let Some((cid, name)) = open_pack {
            self.sticker_search_query.clear();
            self.open_sticker_pack(cid, name);
        }
        if store_clicked {
            self.sticker_search_query.clear();
            self.sticker_tab = StickerTab::Store;
        }
    }

    fn render_picker_emoji_body(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        inner_w: f32,
        body_h: f32,
        thread_id: &str,
        pal: &ThemePalette,
    ) {
        let cols = emoji_cols(inner_w);
        let mut picked: Option<String> = None;
        let recent = self.settings.recent_emojis.clone();
        egui::ScrollArea::vertical()
            .id_salt("picker_emoji_scroll")
            .max_height(body_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut group = |ui: &mut egui::Ui, title: &str, list: &[String]| {
                    section_title(ui, title, pal);
                    for row in list.chunks(cols) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::ZERO;
                            for e in row {
                                let (rect, resp) =
                                    ui.allocate_exact_size(Vec2::splat(EMOJI_CELL), Sense::click());
                                if resp.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        Rounding::same(6.0),
                                        pal.bg_hover,
                                    );
                                }
                                match system_emoji_texture(
                                    e,
                                    &self.sticker_textures,
                                    &mut self.loading_stickers,
                                ) {
                                    Some(tex) => {
                                        ui.painter().image(
                                            tex.id(),
                                            Rect::from_center_size(
                                                rect.center(),
                                                Vec2::splat(26.0),
                                            ),
                                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                            Color32::WHITE,
                                        );
                                    }
                                    None => {
                                        ui.painter().text(
                                            rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            e,
                                            FontId::proportional(22.0),
                                            pal.text_secondary,
                                        );
                                    }
                                }
                                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                    picked = Some(e.clone());
                                }
                            }
                        });
                    }
                    ui.add_space(6.0);
                };
                if !recent.is_empty() {
                    group(ui, "Gần đây", &recent);
                }
                for (title, list) in EMOJI_GROUPS {
                    let owned: Vec<String> = list.iter().map(|s| s.to_string()).collect();
                    group(ui, title, &owned);
                }
            });
        if let Some(e) = picked {
            self.insert_into_composer(ctx, thread_id, &e);
            push_recent(&mut self.settings.recent_emojis, e, RECENT_EMOJI_CAP, |s| {
                s.clone()
            });
            self.settings.save();
        }
    }

    fn render_picker_gif_body(
        &mut self,
        ui: &mut egui::Ui,
        inner_w: f32,
        body_h: f32,
        thread_id: &str,
        is_group: bool,
        pal: &ThemePalette,
    ) {
        let gap = 6.0;
        let cell_w = ((inner_w - gap) / 2.0).floor();
        let cell = Vec2::new(cell_w, (cell_w * 0.75).floor());
        let mut picked: Option<crate::chat::GifItem> = None;
        egui::ScrollArea::vertical()
            .id_salt("picker_gif_scroll")
            .max_height(body_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section_title(
                    ui,
                    if self.picker_gif_query.trim().is_empty() {
                        "GIF xu hướng"
                    } else {
                        "Kết quả"
                    },
                    pal,
                );
                if self.gif_results.is_empty() {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        if self.gif_loading {
                            ui.spinner();
                        } else {
                            let msg = if self.gif_error.is_some() {
                                "Không tải được GIF, thử lại sau"
                            } else {
                                "Không tìm thấy GIF phù hợp"
                            };
                            ui.label(RichText::new(msg).size(12.5).color(pal.text_muted));
                        }
                    });
                    return;
                }
                for row in self.gif_results.chunks(2) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
                        for item in row {
                            let (rect, resp) = ui.allocate_exact_size(cell, Sense::click());
                            ui.painter()
                                .rect_filled(rect, Rounding::same(6.0), pal.bg_input);
                            let key = format!("thumb_{}", crate::crypto::md5_hex(&item.thumb));
                            match self.sticker_textures.get(&key) {
                                Some(tex) => {
                                    let sz = tex.size_vec2();
                                    let scale = (rect.width() / sz.x).max(rect.height() / sz.y);
                                    // Cover the cell, clipped to it.
                                    let shown = sz * scale;
                                    let uv_w = rect.width() / shown.x;
                                    let uv_h = rect.height() / shown.y;
                                    let uv = Rect::from_center_size(
                                        Pos2::new(0.5, 0.5),
                                        Vec2::new(uv_w, uv_h),
                                    );
                                    ui.painter().image(tex.id(), rect, uv, Color32::WHITE);
                                }
                                None => {
                                    if !item.thumb.is_empty()
                                        && self.loading_stickers.insert(key.clone())
                                    {
                                        if let Some(tx) = WORKER_TX.get() {
                                            let _ = tx.send(WorkerCmd::FetchStickerImage {
                                                key,
                                                url: item.thumb.clone(),
                                            });
                                        }
                                    }
                                }
                            }
                            if resp.hovered() {
                                ui.painter().rect_stroke(
                                    rect,
                                    Rounding::same(6.0),
                                    Stroke::new(2.0_f32, pal.primary),
                                );
                            }
                            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                picked = Some(item.clone());
                            }
                        }
                    });
                    ui.add_space(gap);
                }
            });
        if let Some(item) = picked {
            self.sticker_picker_open = false;
            if let Some(tx) = WORKER_TX.get() {
                let _ = tx.send(WorkerCmd::SendGif {
                    to_id: thread_id.to_string(),
                    is_group,
                    item,
                    src: crate::chat::GIF_SRC_CHAT_BAR,
                });
            }
        }
    }
}

fn section_title(ui: &mut egui::Ui, text: &str, pal: &ThemePalette) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(text)
            .size(14.0)
            .strong()
            .color(pal.text_primary),
    );
    ui.add_space(4.0);
}

fn bar_highlight(ui: &egui::Ui, rect: Rect, active: bool, hovered: bool, pal: &ThemePalette) {
    if active {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(0.0, 4.0)),
            Rounding::same(8.0),
            pal.bg_hover,
        );
        ui.painter().rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.left() + 6.0, rect.bottom() - 3.0),
                Vec2::new(rect.width() - 12.0, 2.0),
            ),
            Rounding::same(1.0),
            pal.primary,
        );
    } else if hovered {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(0.0, 4.0)),
            Rounding::same(8.0),
            pal.bg_hover,
        );
    }
}

fn preset_items(presets: &[PresetSticker]) -> Vec<crate::chat::StickerItem> {
    presets
        .iter()
        .map(|p| crate::chat::StickerItem {
            sticker_id: p.sticker_id,
            cate_id: p.cate_id,
            sticker_type: p.sticker_type,
            label: p.label.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_list_moves_duplicates_to_front_and_caps() {
        let mut v = vec![1, 2, 3];
        push_recent(&mut v, 3, 3, |x| *x);
        assert_eq!(v, vec![3, 1, 2]);
        push_recent(&mut v, 9, 3, |x| *x);
        assert_eq!(v, vec![9, 3, 1]);
    }

    #[test]
    fn insert_at_char_handles_multibyte_and_clamps() {
        assert_eq!(
            insert_at_char("xin chào", 3, "😀"),
            ("xin😀 chào".into(), 4)
        );
        assert_eq!(insert_at_char("à", 99, "😀"), ("à😀".into(), 2));
        assert_eq!(insert_at_char("", 0, "👍"), ("👍".into(), 1));
        // A ZWJ/skin-tone cluster counts as several chars, so the caret lands after all of them.
        let (t, caret) = insert_at_char("ab", 1, "👍🏽");
        assert_eq!(t, "a👍🏽b");
        assert_eq!(caret, 3);
    }

    #[test]
    fn emoji_groups_are_non_empty_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for (title, list) in EMOJI_GROUPS {
            assert!(!title.is_empty());
            assert!(!list.is_empty());
            for e in *list {
                assert!(!e.is_empty());
                assert!(seen.insert(*e), "duplicate emoji {e}");
            }
        }
    }

    #[test]
    fn grid_columns_follow_panel_width() {
        assert_eq!(sticker_cols(PANEL_W - 24.0), 4);
        assert!(sticker_cols(PANEL_W_EXPANDED - 24.0) > 4);
        assert!(emoji_cols(PANEL_W - 24.0) >= 8);
    }
}
