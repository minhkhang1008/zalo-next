use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use eframe::egui::{self, Button, Label, RichText, Sense, TextureHandle, Vec2};

use crate::chat::{ContactCard, Thread};

use super::{paint_user_avatar, BubbleAction, ThemePalette};

fn format_phone(phone: &str) -> Cow<'_, str> {
    if phone.len() == 10 && phone.starts_with('0') && phone.bytes().all(|b| b.is_ascii_digit()) {
        format!("{} {} {}", &phone[..3], &phone[3..6], &phone[6..]).into()
    } else {
        phone.into()
    }
}

pub(super) fn render_contact_card(
    ui: &mut egui::Ui,
    card: &ContactCard,
    pal: &ThemePalette,
    textures: &HashMap<String, TextureHandle>,
    loading: &mut HashSet<String>,
    threads: &[Thread],
    current_uid: &str,
) -> Option<BubbleAction> {
    let phone = card.phone.as_deref();
    ui.horizontal(|ui| {
        let (avatar_rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
        // Cached and imported cards can reach the renderer without going through the wire parser.
        let avatar = card
            .avatar
            .as_deref()
            .filter(|url| crate::session::is_trusted_zalo_url(url));
        paint_user_avatar(
            ui.painter(),
            avatar_rect.center(),
            20.0,
            &card.name,
            false,
            avatar,
            textures,
            loading,
        );
        ui.vertical(|ui| {
            let color = pal.text_primary;
            let name = Label::new(RichText::new(&card.name).size(15.0))
                .truncate()
                .selectable(false);
            let (pos, galley, response) = name.layout_in_ui(ui);
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), &card.name)
            });
            response.on_hover_text(&card.name);
            // The bundled UI font has one weight. An offset pass gives the name a heavier stroke.
            ui.painter().galley(pos, galley.clone(), color);
            ui.painter()
                .galley(pos + Vec2::new(0.45, 0.0), galley, color);
            if let Some(phone) = phone {
                ui.add(
                    Label::new(RichText::new(format_phone(phone)).color(pal.text_secondary))
                        .truncate(),
                );
            }
        });
    });
    ui.separator();

    let has_uid = !card.uid.is_empty();
    let can_message = has_uid && threads.iter().any(|thread| thread.id == card.uid);
    let can_call = has_uid && card.uid != current_uid;
    ui.scope(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        ui.horizontal_wrapped(|ui| {
            let message = ui
                .add_enabled(can_message, Button::new("Nhắn tin"))
                .on_disabled_hover_text("Chưa có cuộc trò chuyện với người này");
            let call = ui.add_enabled(can_call, Button::new("Gọi điện"));
            let copy_clicked = phone.is_some() && ui.button("Sao chép số").clicked();

            if message.clicked() {
                Some(BubbleAction::MessageContact)
            } else if call.clicked() {
                Some(BubbleAction::CallContact)
            } else if copy_clicked {
                Some(BubbleAction::CopyContactPhone)
            } else {
                None
            }
        })
        .inner
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::format_phone;

    #[test]
    fn formats_ten_digit_vietnamese_numbers() {
        assert_eq!(format_phone("0793862568"), "079 386 2568");
        assert_eq!(format_phone("0241234567"), "024 123 4567");
    }

    #[test]
    fn leaves_other_phone_formats_as_given() {
        for phone in [
            "+84793862568",
            "079 386 2568",
            "1234567890",
            "012345",
            "0é1234567",
        ] {
            assert_eq!(format_phone(phone), phone);
        }
    }
}
