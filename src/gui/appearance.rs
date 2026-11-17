//! Settings → Giao diện: font, UI scale, motion and colour customisation.
//!
//! The theme cards (Sáng / Tối / Hệ thống) stay in `modals.rs`; everything below them lives here
//! so the settings modal does not grow further. All values are stored in `AppSettings` and take
//! effect through `theme_dirty` (palette, zoom, animation time) and `theme::setup_custom_fonts_*`.

use super::*;

/// Colour presets offered under "Bảng màu gợi ý" (name, `#RRGGBB` primary).
const COLOR_PRESETS: &[(&str, &str)] = &[
    ("Zalo Blue", "#0068FF"),
    ("Atom Blue", "#528BFF"),
    ("Emerald", "#059669"),
    ("Violet", "#7C3AED"),
    ("Rose", "#E11D48"),
    ("Amber", "#D97706"),
    ("Slate", "#475569"),
];

/// UI zoom steps shown as buttons (percent).
const SCALE_STEPS: &[u32] = &[90, 100, 110, 125, 150];

/// Animation speed presets (multiplier, label).
const SPEED_STEPS: &[(f32, &str)] = &[
    (0.75, "0.75x Chậm"),
    (1.0, "1.0x Chuẩn"),
    (1.5, "1.5x Nhanh"),
];

/// Outcome of one colour row.
enum ColorEdit {
    Keep,
    Set(String),
    Reset,
}

fn section_title(ui: &mut egui::Ui, pal: &ThemePalette, text: &str) {
    ui.add_space(16.0);
    ui.label(
        RichText::new(text)
            .color(pal.text_primary)
            .size(13.5)
            .strong(),
    );
    ui.add_space(6.0);
}

fn settings_card(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    inner_w: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::none()
        .fill(pal.bg_elevated)
        .stroke(Stroke::new(1.0_f32, pal.border))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(14.0, 10.0))
        .show(ui, |ui| {
            let w = (inner_w - 28.0).max(280.0);
            ui.set_min_width(w);
            ui.set_max_width(w);
            add_contents(ui);
        });
}

/// Pill button that is filled with the primary colour while selected.
fn choice_button(ui: &mut egui::Ui, pal: &ThemePalette, label: &str, selected: bool) -> bool {
    let (fill, ink) = if selected {
        (pal.primary, Color32::WHITE)
    } else {
        (pal.bg_hover, pal.text_primary)
    };
    ui.add(
        egui::Button::new(RichText::new(label).color(ink).size(12.6))
            .fill(fill)
            .stroke(Stroke::new(
                1.0_f32,
                if selected { pal.primary } else { pal.border },
            ))
            .rounding(Rounding::same(7.0)),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
    .clicked()
}

/// One "label ........ [swatch] #RRGGBB [reset]" row.
fn color_row(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    label: &str,
    current: Color32,
    overridden: bool,
) -> ColorEdit {
    let mut edit = ColorEdit::Keep;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(pal.text_primary).size(13.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if overridden
                && ui
                    .add(
                        egui::Button::new(RichText::new(lucide::REFRESH).size(13.0))
                            .rounding(Rounding::same(7.0)),
                    )
                    .on_hover_text("Về màu mặc định")
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .clicked()
            {
                edit = ColorEdit::Reset;
            }
            ui.label(
                RichText::new(format_hex_color(current))
                    .monospace()
                    .color(pal.text_secondary)
                    .size(12.4),
            );
            let mut rgb = [current.r(), current.g(), current.b()];
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                edit = ColorEdit::Set(format_hex_color(Color32::from_rgb(rgb[0], rgb[1], rgb[2])));
            }
        });
    });
    edit
}

fn preview_bubble(
    ui: &mut egui::Ui,
    text: &str,
    time: &str,
    bg: Color32,
    ink: Color32,
    meta: Color32,
    outgoing: bool,
) {
    let layout = if outgoing {
        Layout::right_to_left(Align::TOP)
    } else {
        Layout::left_to_right(Align::TOP)
    };
    ui.with_layout(layout, |ui| {
        egui::Frame::none()
            .fill(bg)
            .rounding(Rounding::same(10.0))
            .inner_margin(Margin::symmetric(12.0, 8.0))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(text).color(ink).size(13.4));
                    ui.label(RichText::new(time).color(meta).size(11.0));
                });
            });
    });
}

impl ZaloGuiApp {
    /// Persist `settings` and re-apply the theme next frame; warn when the worker is not ready.
    fn appearance_changed(&mut self) {
        self.theme_dirty = true;
        if !self.settings.save() {
            self.show_toast("Không thể lưu giao diện: tác vụ nền chưa sẵn sàng.");
        }
    }

    fn set_custom_color(slot: &mut Option<String>, edit: ColorEdit) -> bool {
        match edit {
            ColorEdit::Keep => false,
            ColorEdit::Set(hex) => {
                *slot = Some(hex);
                true
            }
            ColorEdit::Reset => {
                *slot = None;
                true
            }
        }
    }

    /// Font, scale, motion, colour presets/custom colours, live preview and "restore defaults".
    pub(super) fn render_appearance_display_settings(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        pal: &ThemePalette,
        inner_w: f32,
    ) {
        let mut changed = false;
        let mut fonts_changed = false;

        // --- Font & scale -------------------------------------------------------------------
        section_title(ui, pal, "Phông chữ & Kích thước hiển thị");
        settings_card(ui, pal, inner_w, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Phông chữ hiển thị")
                            .color(pal.text_primary)
                            .size(13.2),
                    );
                    ui.label(
                        RichText::new("Tối ưu hiển thị đầy đủ ký tự tiếng Việt")
                            .color(pal.text_secondary)
                            .size(11.8),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let choices = theme::available_font_choices();
                    let current_label = choices
                        .iter()
                        .find(|(key, _)| *key == self.settings.font_family)
                        .map_or("Mặc định (Be Vietnam Pro)", |(_, label)| *label);
                    egui::ComboBox::from_id_salt("appearance_font_family")
                        .selected_text(current_label)
                        .show_ui(ui, |ui| {
                            for (key, label) in choices {
                                if ui
                                    .selectable_label(self.settings.font_family == key, label)
                                    .clicked()
                                    && self.settings.font_family != key
                                {
                                    self.settings.font_family = key.to_string();
                                    fonts_changed = true;
                                }
                            }
                        });
                });
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
            ui.label(
                RichText::new("Thu phóng giao diện")
                    .color(pal.text_primary)
                    .size(13.2),
            );
            ui.label(
                RichText::new("Điều chỉnh kích thước hiển thị chữ và biểu tượng")
                    .color(pal.text_secondary)
                    .size(11.8),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let cur = (self.settings.normalized_font_scale() * 100.0).round() as u32;
                for pct in SCALE_STEPS {
                    if choice_button(ui, pal, &format!("{pct}%"), cur == *pct) {
                        self.settings.font_scale = *pct as f32 / 100.0;
                        changed = true;
                    }
                }
            });
        });

        // --- Motion -------------------------------------------------------------------------
        section_title(ui, pal, "Hiệu ứng chuyển động");
        settings_card(ui, pal, inner_w, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Bật hiệu ứng chuyển động mượt mà khi tương tác")
                        .color(pal.text_primary)
                        .size(13.2),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if render_toggle_switch(ui, &mut self.settings.animations_enabled, pal)
                        .changed()
                    {
                        changed = true;
                    }
                });
            });
            if self.settings.animations_enabled {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Tốc độ hiệu ứng:")
                            .color(pal.text_secondary)
                            .size(12.4),
                    );
                    let cur = self.settings.normalized_animation_speed();
                    for (speed, label) in SPEED_STEPS {
                        if choice_button(ui, pal, label, (cur - speed).abs() < 0.01) {
                            self.settings.animation_speed = *speed;
                            changed = true;
                        }
                    }
                });
            }
        });

        // --- Colours ------------------------------------------------------------------------
        section_title(ui, pal, "Tùy biến màu sắc giao diện & Tin nhắn");
        settings_card(ui, pal, inner_w, |ui| {
            ui.label(
                RichText::new("Bảng màu gợi ý:")
                    .color(pal.text_secondary)
                    .size(12.4),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (name, hex) in COLOR_PRESETS {
                    let swatch = parse_hex_color(hex).unwrap_or(pal.primary);
                    let selected = self
                        .settings
                        .custom_primary_color
                        .as_deref()
                        .is_some_and(|c| c.eq_ignore_ascii_case(hex));
                    let resp = ui.add(
                        egui::Button::new(
                            RichText::new(format!("   {name}"))
                                .color(pal.text_primary)
                                .size(12.6),
                        )
                        .fill(if selected {
                            pal.bg_selected
                        } else {
                            pal.bg_hover
                        })
                        .stroke(Stroke::new(
                            1.0_f32,
                            if selected { pal.primary } else { pal.border },
                        ))
                        .rounding(Rounding::same(8.0))
                        .min_size(Vec2::new(0.0, 28.0)),
                    );
                    let dot = Pos2::new(resp.rect.left() + 13.0, resp.rect.center().y);
                    ui.painter().circle_filled(dot, 6.0, swatch);
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        self.settings.custom_primary_color = Some((*hex).to_string());
                        changed = true;
                    }
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);
            ui.label(
                RichText::new("Tùy chỉnh màu chi tiết:")
                    .color(pal.text_secondary)
                    .size(12.4),
            );
            ui.add_space(4.0);

            let edit = color_row(
                ui,
                pal,
                "Màu chủ đạo:",
                pal.primary,
                self.settings.custom_primary_color.is_some(),
            );
            changed |= Self::set_custom_color(&mut self.settings.custom_primary_color, edit);
            let edit = color_row(
                ui,
                pal,
                "Màu điểm nhấn & trạng thái:",
                pal.accent_online,
                self.settings.custom_accent_color.is_some(),
            );
            changed |= Self::set_custom_color(&mut self.settings.custom_accent_color, edit);
            let edit = color_row(
                ui,
                pal,
                "Bong bóng tin nhắn gửi đi:",
                pal.bubble_out_bg,
                self.settings.custom_bubble_out_bg.is_some(),
            );
            changed |= Self::set_custom_color(&mut self.settings.custom_bubble_out_bg, edit);
            let edit = color_row(
                ui,
                pal,
                "Bong bóng tin nhắn nhận:",
                pal.bubble_in_bg,
                self.settings.custom_bubble_in_bg.is_some(),
            );
            changed |= Self::set_custom_color(&mut self.settings.custom_bubble_in_bg, edit);

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);
            ui.label(
                RichText::new("Xem trước giao diện:")
                    .color(pal.text_secondary)
                    .size(12.4),
            );
            ui.add_space(4.0);
            egui::Frame::none()
                .fill(pal.bg_chat_canvas)
                .stroke(Stroke::new(1.0_f32, pal.border))
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::same(10.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    preview_bubble(
                        ui,
                        "Chào bạn! Màu sắc và font chữ mới hiển thị thế nào?",
                        "10:45",
                        pal.bubble_in_bg,
                        pal.bubble_in_text,
                        pal.bubble_in_meta,
                        false,
                    );
                    ui.add_space(6.0);
                    preview_bubble(
                        ui,
                        "Rất đẹp mắt, giao diện cập nhật tức thì!",
                        "10:46 · Đã gửi",
                        pal.bubble_out_bg,
                        pal.bubble_out_text,
                        pal.bubble_out_meta,
                        true,
                    );
                });

            ui.add_space(10.0);
            if ui
                .button(RichText::new("Khôi phục mặc định").size(12.8))
                .on_hover_text(
                    "Đặt lại màu, phông chữ, thu phóng và hiệu ứng (giữ chế độ Sáng/Tối)",
                )
                .clicked()
            {
                self.settings.custom_primary_color = None;
                self.settings.custom_accent_color = None;
                self.settings.custom_bubble_out_bg = None;
                self.settings.custom_bubble_in_bg = None;
                self.settings.font_scale = 1.0;
                self.settings.animations_enabled = true;
                self.settings.animation_speed = 1.0;
                if self.settings.font_family != "default" {
                    self.settings.font_family = "default".into();
                    fonts_changed = true;
                }
                changed = true;
            }
        });

        if fonts_changed {
            setup_custom_fonts_with_choice(ctx, &self.settings.font_family);
            changed = true;
        }
        if changed {
            self.appearance_changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_valid_hex_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for (name, hex) in COLOR_PRESETS {
            assert!(
                parse_hex_color(hex).is_some(),
                "{name} has an invalid colour {hex}"
            );
            assert!(
                seen.insert(hex.to_ascii_lowercase()),
                "duplicate preset {hex}"
            );
        }
    }

    #[test]
    fn custom_colors_override_palette_and_keep_text_readable() {
        let mut pal = ThemePalette::dark();
        pal.apply_custom_colors(
            Some("#D97706"),
            Some("#112233"),
            Some("#FFFFFF"),
            Some("#101010"),
        );
        assert_eq!(format_hex_color(pal.primary), "#D97706");
        assert_eq!(format_hex_color(pal.accent_online), "#112233");
        // Light custom bubble gets dark text; dark custom bubble keeps light text.
        assert!(contrast_ratio(pal.bubble_out_text, pal.bubble_out_bg) >= 4.5);
        assert!(contrast_ratio(pal.bubble_in_text, pal.bubble_in_bg) >= 4.5);
        // Invalid hex is ignored instead of corrupting the palette.
        let before = ThemePalette::dark();
        let mut after = ThemePalette::dark();
        after.apply_custom_colors(Some("not-a-colour"), None, None, None);
        assert_eq!(before.primary, after.primary);
    }

    #[test]
    fn scale_and_speed_are_clamped() {
        let mut s = AppSettings {
            font_scale: f32::NAN,
            ..Default::default()
        };
        assert_eq!(s.normalized_font_scale(), 1.0);
        s.font_scale = 9.0;
        assert_eq!(s.normalized_font_scale(), 3.0);
        s.animation_speed = -1.0;
        assert_eq!(s.normalized_animation_speed(), 1.0);
        s.animation_speed = 10.0;
        assert_eq!(s.normalized_animation_speed(), 2.0);
    }

    #[test]
    fn system_theme_follows_os_and_defaults_dark() {
        assert!(!resolve_dark_mode("light", Some(egui::Theme::Dark)));
        assert!(resolve_dark_mode("dark", Some(egui::Theme::Light)));
        assert!(!resolve_dark_mode("system", Some(egui::Theme::Light)));
        assert!(resolve_dark_mode("system", Some(egui::Theme::Dark)));
        assert!(resolve_dark_mode("system", None));
    }

    #[test]
    fn old_settings_files_without_appearance_fields_still_load() {
        let s: AppSettings = serde_json::from_str(r#"{"theme_mode":"light"}"#).unwrap();
        assert_eq!(s.font_family, "default");
        assert_eq!(s.normalized_font_scale(), 1.0);
        assert!(s.animations_enabled);
        assert!(s.custom_primary_color.is_none());
    }
}
