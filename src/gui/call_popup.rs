//! Incoming-call popup: a small always-on-top window in the top-right corner of the screen that
//! does not take focus, like a notification (owner request 2026-11-28). The full call window
//! opens only after the user answers, so a ringing call never covers the screen or steals the
//! keyboard from another app.

use eframe::egui::{
    self, Align, Button, Color32, CursorIcon, Key, Layout, Margin, RichText, Rounding, Sense,
    Stroke, Vec2,
};

use super::{background, paint_user_avatar, ThemePalette, WorkerCmd, ZaloGuiApp, WORKER_TX};

const POPUP_W: f32 = 400.0;
const POPUP_H: f32 = 128.0;
const POPUP_MARGIN: f32 = 16.0;
/// Top inset below the macOS menu bar.
const POPUP_TOP: f32 = 40.0;
/// Unique window title; the macOS code finds the native window by it.
const POPUP_TITLE: &str = "Zalo-next — Cuộc gọi đến";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IncomingChoice {
    Decline,
    /// Voice answer, or a video answer with the camera on when video calls are enabled.
    Answer,
    AnswerMicOff,
    AnswerCamOff,
    /// The app is locked: bring it forward so the lock screen can be unlocked.
    OpenApp,
}

/// The buttons offered for a ringing call, left to right.
pub(super) fn incoming_choices(
    is_video_call: bool,
    video_enabled: bool,
    locked: bool,
) -> &'static [IncomingChoice] {
    use IncomingChoice::*;
    if locked {
        &[Decline, OpenApp]
    } else if is_video_call && video_enabled {
        &[Decline, AnswerCamOff, Answer]
    } else {
        &[Decline, AnswerMicOff, Answer]
    }
}

/// `(cam_start_off, mic_start_off)` for an answering choice, `None` for the others. A video call
/// is always answered with the camera off while video calls are disabled; a voice call keeps
/// `cam_start_off = false`, exactly what the client sent for voice answers before.
pub(super) fn answer_flags(
    choice: IncomingChoice,
    is_video_call: bool,
    video_enabled: bool,
) -> Option<(bool, bool)> {
    let cam_off = is_video_call && (!video_enabled || choice == IncomingChoice::AnswerCamOff);
    match choice {
        IncomingChoice::Answer | IncomingChoice::AnswerCamOff => Some((cam_off, false)),
        IncomingChoice::AnswerMicOff => Some((cam_off, true)),
        IncomingChoice::Decline | IncomingChoice::OpenApp => None,
    }
}

fn choice_label(choice: IncomingChoice, is_video_call: bool, video_enabled: bool) -> &'static str {
    match choice {
        IncomingChoice::Decline => "Không nghe máy",
        IncomingChoice::AnswerMicOff => "Nghe nhưng tắt mic",
        IncomingChoice::AnswerCamOff => "Nghe, tắt camera",
        IncomingChoice::Answer if is_video_call && video_enabled => "Nghe có hình",
        IncomingChoice::Answer => "Nghe máy",
        IncomingChoice::OpenApp => "Mở Zalo-next",
    }
}

fn call_subtitle(is_video_call: bool, is_group: bool, video_enabled: bool) -> &'static str {
    match (is_group, is_video_call, video_enabled) {
        (true, _, _) => "Cuộc gọi nhóm đến",
        (false, true, true) => "Cuộc gọi video đến",
        (false, true, false) => "Cuộc gọi video đến · camera của bạn sẽ tắt",
        (false, false, _) => "Cuộc gọi thoại đến",
    }
}

impl ZaloGuiApp {
    /// Show the ringing call in its own small window. Runs before the lock screen so a locked
    /// app still shows that a call is ringing, without the caller's name.
    pub(super) fn render_incoming_call_popup(&mut self, ctx: &egui::Context, pal: &ThemePalette) {
        let Some(call) = self
            .active_call
            .clone()
            .filter(|call| call.is_incoming_ringing)
        else {
            self.incoming_popup_pinned = false;
            return;
        };
        let locked = crate::app_lock::is_locked();
        let video_enabled = self.settings.video_calls_enabled;
        let choices = incoming_choices(call.is_video, video_enabled, locked);
        let avatar = if locked {
            None
        } else {
            self.threads
                .iter()
                .find(|t| !call.partner_id.is_empty() && t.id == call.partner_id)
                .and_then(|t| t.avatar.clone())
        };
        // Ringing timeouts and remote cancels arrive as worker events; keep them flowing.
        ctx.request_repaint_after(std::time::Duration::from_millis(250));

        let monitor = ctx.input(|i| i.viewport().monitor_size);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(POPUP_TITLE)
            .with_inner_size([POPUP_W, POPUP_H])
            .with_resizable(false)
            .with_decorations(false)
            .with_always_on_top()
            .with_active(false)
            .with_taskbar(false);
        if let Some(size) = monitor.filter(|s| s.x > POPUP_W + 2.0 * POPUP_MARGIN) {
            builder = builder.with_position([size.x - POPUP_W - POPUP_MARGIN, POPUP_TOP]);
        }

        let mut picked: Option<IncomingChoice> = None;
        let viewport_id = egui::ViewportId::from_hash_of("zalo_incoming_call_popup");
        ctx.show_viewport_immediate(viewport_id, builder, |vp_ctx, _class| {
            if vp_ctx.input(|i| i.viewport().close_requested()) {
                picked = Some(IncomingChoice::Decline);
            }
            if vp_ctx.input(|i| i.key_pressed(Key::Escape)) {
                picked = Some(IncomingChoice::Decline);
            }
            if vp_ctx.input(|i| i.key_pressed(Key::Enter)) {
                picked = choices.last().copied();
            }
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::none()
                        .fill(pal.bg_elevated)
                        .stroke(Stroke::new(1.0_f32, pal.border))
                        .inner_margin(Margin::symmetric(16.0, 14.0)),
                )
                .show(vp_ctx, |ui| {
                    ui.horizontal(|ui| {
                        let (avatar_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                        let name = if locked {
                            "Cuộc gọi đến"
                        } else {
                            call.partner_name.as_str()
                        };
                        paint_user_avatar(
                            ui.painter(),
                            avatar_rect.center(),
                            22.0,
                            name,
                            call.is_group,
                            avatar.as_deref(),
                            &self.sticker_textures,
                            &mut self.loading_stickers,
                        );
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.add_space(2.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(name)
                                        .strong()
                                        .size(15.0)
                                        .color(pal.text_primary),
                                )
                                .truncate(),
                            );
                            let subtitle = if locked {
                                "Zalo-next đang khóa"
                            } else {
                                call_subtitle(call.is_video, call.is_group, video_enabled)
                            };
                            ui.label(RichText::new(subtitle).size(12.5).color(pal.text_secondary));
                        });
                    });
                    ui.add_space(12.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        for &choice in choices.iter().rev() {
                            let label = choice_label(choice, call.is_video, video_enabled);
                            let (fill, text, stroke) = match choice {
                                IncomingChoice::Decline => {
                                    (pal.danger, Color32::WHITE, Stroke::NONE)
                                }
                                IncomingChoice::Answer | IncomingChoice::OpenApp => {
                                    (pal.accent_online, Color32::WHITE, Stroke::NONE)
                                }
                                IncomingChoice::AnswerMicOff | IncomingChoice::AnswerCamOff => (
                                    pal.bg_hover,
                                    pal.text_primary,
                                    Stroke::new(1.0_f32, pal.border),
                                ),
                            };
                            let button =
                                Button::new(RichText::new(label).size(12.5).strong().color(text))
                                    .fill(fill)
                                    .stroke(stroke)
                                    .rounding(Rounding::same(8.0))
                                    .min_size(Vec2::new(0.0, 32.0));
                            if ui
                                .add(button)
                                .on_hover_cursor(CursorIcon::PointingHand)
                                .clicked()
                            {
                                picked = Some(choice);
                            }
                        }
                    });
                });
        });

        #[cfg(target_os = "macos")]
        if !self.incoming_popup_pinned {
            self.incoming_popup_pinned = keep_popup_visible_while_hidden();
        }

        let Some(choice) = picked else {
            return;
        };
        match choice {
            IncomingChoice::OpenApp => background::show_app(ctx),
            IncomingChoice::Decline => {
                let sent = WORKER_TX.get().is_some_and(|tx| {
                    tx.send(WorkerCmd::AnswerCall {
                        accept: false,
                        cam_start_off: false,
                        mic_start_off: false,
                    })
                    .is_ok()
                });
                if sent {
                    self.active_call = None;
                } else {
                    self.show_toast("Tác vụ nền chưa sẵn sàng.");
                }
            }
            IncomingChoice::Answer
            | IncomingChoice::AnswerMicOff
            | IncomingChoice::AnswerCamOff => {
                // Locked apps only offer Decline / OpenApp, so this never answers behind the lock.
                let Some((cam_start_off, mic_start_off)) =
                    answer_flags(choice, call.is_video, video_enabled)
                else {
                    return;
                };
                let sent = WORKER_TX.get().is_some_and(|tx| {
                    tx.send(WorkerCmd::AnswerCall {
                        accept: true,
                        cam_start_off,
                        mic_start_off,
                    })
                    .is_ok()
                });
                if !sent {
                    self.show_toast("Tác vụ nền chưa sẵn sàng.");
                    return;
                }
                if let Some(active) = self.active_call.as_mut() {
                    active.is_incoming_ringing = false;
                    active.cam_enabled = call.is_video && !cam_start_off;
                    active.mic_muted = mic_start_off;
                    active.state_label = "Đang kết nối cuộc gọi…".into();
                }
                // The user chose to take the call: bring the app (and the call window that opens
                // next frame) forward.
                background::show_app(ctx);
            }
        }
    }
}

/// Find the popup's native window and keep it on screen while the app is hidden (Cmd+H or the
/// window closed to the background), without activating the app. Returns `true` once done.
#[cfg(target_os = "macos")]
fn keep_popup_visible_while_hidden() -> bool {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{MainThreadMarker, NSString};

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    let title = NSString::from_str(POPUP_TITLE);
    // SAFETY: AppKit's window list and window properties are only touched on the main thread
    // (checked by `MainThreadMarker`); the window objects are retained by the returned array.
    unsafe {
        for window in app.windows().iter() {
            if window.title().isEqualToString(&title) {
                window.setCanHide(false);
                window.orderFrontRegardless();
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_disabled_offers_voice_choices_and_answers_without_camera() {
        let choices = incoming_choices(true, false, false);
        assert_eq!(
            choices,
            &[
                IncomingChoice::Decline,
                IncomingChoice::AnswerMicOff,
                IncomingChoice::Answer
            ]
        );
        assert_eq!(
            answer_flags(IncomingChoice::Answer, true, false),
            Some((true, false))
        );
        assert_eq!(
            answer_flags(IncomingChoice::AnswerMicOff, true, false),
            Some((true, true))
        );
    }

    #[test]
    fn voice_calls_keep_the_previous_answer_flags() {
        assert_eq!(
            answer_flags(IncomingChoice::Answer, false, false),
            Some((false, false))
        );
        assert_eq!(
            answer_flags(IncomingChoice::Answer, false, true),
            Some((false, false))
        );
        assert_eq!(
            answer_flags(IncomingChoice::AnswerMicOff, false, true),
            Some((false, true))
        );
    }

    #[test]
    fn video_enabled_offers_camera_choices() {
        assert_eq!(
            incoming_choices(true, true, false),
            &[
                IncomingChoice::Decline,
                IncomingChoice::AnswerCamOff,
                IncomingChoice::Answer
            ]
        );
        assert_eq!(
            answer_flags(IncomingChoice::Answer, true, true),
            Some((false, false))
        );
        assert_eq!(
            answer_flags(IncomingChoice::AnswerCamOff, true, true),
            Some((true, false))
        );
    }

    #[test]
    fn locked_app_never_answers_from_the_popup() {
        let choices = incoming_choices(false, false, true);
        assert_eq!(choices, &[IncomingChoice::Decline, IncomingChoice::OpenApp]);
        assert!(choices
            .iter()
            .all(|&choice| answer_flags(choice, false, false).is_none()));
    }
}
