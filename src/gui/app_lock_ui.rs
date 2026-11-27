//! App Lock state and screens. The worker owns passcode hashing and persistence.

use std::time::{Instant, SystemTime};

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU8, Ordering};

use super::*;
use crate::app_lock::{Attempt, AutoLock, Config, Method};

pub(super) const LOAD_ERROR: &str = "Không đọc được cấu hình khóa ứng dụng; khóa đang tắt.";
const WORKER_UNAVAILABLE: &str = "Tác vụ nền chưa sẵn sàng.";

#[cfg(target_os = "macos")]
static SYSTEM_AUTH_RESULT: AtomicU8 = AtomicU8::new(0);

pub(super) struct State {
    config: Config,
    auto: AutoLock,
    lock_requested: bool,
    candidate: String,
    focus_passcode: bool,
    unlock_pending: bool,
    unlock_error: Option<String>,
    pub(super) settings_error: Option<String>,
    pending_saves: usize,
    #[cfg(target_os = "macos")]
    system_prompt_started: bool,
    #[cfg(not(target_os = "macos"))]
    passcode_setup: bool,
    #[cfg(not(target_os = "macos"))]
    new_passcode: String,
    #[cfg(not(target_os = "macos"))]
    repeated_passcode: String,
}

impl State {
    pub(super) fn new(config: Config, load_error: Option<&str>) -> Self {
        Self {
            config,
            auto: AutoLock::default(),
            lock_requested: false,
            candidate: String::new(),
            focus_passcode: true,
            unlock_pending: false,
            unlock_error: None,
            settings_error: load_error.map(str::to_owned),
            pending_saves: 0,
            #[cfg(target_os = "macos")]
            system_prompt_started: false,
            #[cfg(not(target_os = "macos"))]
            passcode_setup: false,
            #[cfg(not(target_os = "macos"))]
            new_passcode: String::new(),
            #[cfg(not(target_os = "macos"))]
            repeated_passcode: String::new(),
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn clear_passcode_setup(&mut self) {
        self.passcode_setup = false;
        self.new_passcode.clear();
        self.repeated_passcode.clear();
    }
}

impl ZaloGuiApp {
    pub(super) fn handle_app_lock_input(&mut self, ctx: &egui::Context) {
        let allow_shortcut = self.app_lock.config.usable() && !app_lock::is_locked();
        let (had_input, shortcut) = ctx.input(|i| {
            let had_input = !i.events.is_empty() || i.pointer.delta() != Vec2::ZERO;
            // egui maps the platform's primary modifier to COMMAND on all operating systems.
            let shortcut = allow_shortcut
                && i.events.iter().any(|event| {
                    matches!(event, egui::Event::Key {
                    key: Key::L, pressed: true, repeat: false, modifiers, ..
                } if modifiers.matches_exact(egui::Modifiers::COMMAND))
                });
            (had_input, shortcut)
        });
        if had_input {
            self.app_lock.auto.note_input(Instant::now());
        }
        if shortcut {
            self.app_lock.lock_requested = true;
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }

    pub(super) fn tick_app_lock(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let auto = self
            .app_lock
            .auto
            .tick(&self.app_lock.config, now, SystemTime::now());
        self.handle_app_lock_input(ctx);
        #[cfg(not(target_os = "macos"))]
        if !self.settings_modal_open || self.settings_tab != SettingsTab::Privacy {
            self.app_lock.clear_passcode_setup();
        }

        if app_lock::is_locked() {
            return;
        }
        let manual = std::mem::take(&mut self.app_lock.lock_requested);
        if !self.app_lock.config.usable() || !(auto || manual) {
            return;
        }
        if !may_lock(
            self.active_call
                .as_ref()
                .map(|call| call.is_incoming_ringing),
        ) {
            if manual {
                self.show_toast("Không khóa trong cuộc gọi.");
            }
            return;
        }

        app_lock::set_locked(true);
        self.settings_modal_open = false;
        self.settings_blur_tex = None;
        self.settings_blur_req_at = None;
        self.close_image_viewer();
        self.profile_menu_open = false;
        self.forward_label_menu_open = false;
        self.sidebar_label_menu_open = false;
        self.sticker_picker_open = false;
        self.dismissed_sticker_suggest_for = self.input_message.trim().to_owned();
        if let Some((_, query)) = workspace::extract_active_mention_query(&self.input_message) {
            self.dismissed_mention_suggest_for = query.to_owned();
        }
        self.gif_dismissed_for = Some(self.input_message.clone());
        self.quick_message_dismissed_for = Some(self.input_message.clone());
        if let Some(url) = crate::chat::extract_first_url(&self.input_message) {
            self.dismissed_link_preview_for = url;
        }
        self.app_lock.candidate.clear();
        self.app_lock.focus_passcode = true;
        self.app_lock.unlock_error = None;
        #[cfg(target_os = "macos")]
        {
            self.app_lock.system_prompt_started = false;
        }
        #[cfg(not(target_os = "macos"))]
        self.app_lock.clear_passcode_setup();
        ctx.memory_mut(|memory| {
            memory.close_popup();
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        ctx.data_mut(|data| {
            data.remove_by_type::<egui::menu::BarState>();
            data.remove::<(String, Pos2, usize)>(Id::new("active_bubble_context_menu"));
        });
    }

    fn unlock_app(&mut self) {
        app_lock::set_locked(false);
        self.app_lock.auto.note_input(Instant::now());
        let before = self.app_lock.config.clone();
        self.app_lock.config.note_unlocked();
        if self.app_lock.config != before {
            self.save_app_lock();
        }
    }

    pub(super) fn finish_app_lock_attempt(
        &mut self,
        config: Config,
        attempt: Attempt,
        save_error: Option<String>,
    ) {
        self.app_lock.config = config;
        self.app_lock.unlock_pending = false;
        self.app_lock.focus_passcode = true;
        if let Some(mut message) = attempt_message(attempt) {
            if let Some(error) = save_error {
                message.push('\n');
                message.push_str(&error);
            }
            self.app_lock.unlock_error = Some(message);
        } else {
            self.unlock_app();
            if let Some(error) = save_error {
                self.show_toast(error);
            }
        }
    }

    pub(super) fn fail_app_lock_attempt(&mut self) {
        self.app_lock.unlock_pending = false;
        self.app_lock.focus_passcode = true;
        self.app_lock.unlock_error = Some("Không kiểm tra được mã khóa.".into());
    }

    pub(super) fn finish_app_lock_save(&mut self, config: Option<Config>, error: Option<String>) {
        self.app_lock.pending_saves = self.app_lock.pending_saves.saturating_sub(1);
        if let Some(config) = config {
            self.app_lock.config = config;
            self.app_lock.auto.note_input(Instant::now());
        }
        self.app_lock.settings_error = error.clone();
        if let Some(error) = error {
            self.show_toast(error);
        }
    }

    fn send_app_lock_save(&mut self, cmd: WorkerCmd) {
        if WORKER_TX.get().is_some_and(|tx| tx.send(cmd).is_ok()) {
            self.app_lock.pending_saves += 1;
            self.app_lock.settings_error = None;
        } else {
            self.app_lock.settings_error = Some(WORKER_UNAVAILABLE.into());
            self.show_toast(WORKER_UNAVAILABLE);
        }
    }

    fn save_app_lock(&mut self) {
        self.send_app_lock_save(WorkerCmd::AppLockSave {
            config: self.app_lock.config.clone(),
        });
    }

    pub(super) fn render_app_lock(&mut self, ctx: &egui::Context, pal: &ThemePalette) -> bool {
        if !app_lock::is_locked() {
            return false;
        }
        #[cfg(target_os = "macos")]
        match SYSTEM_AUTH_RESULT.swap(0, Ordering::Relaxed) {
            1 => {
                self.app_lock.unlock_pending = false;
                self.unlock_app();
                ctx.request_repaint();
                return false;
            }
            2 => {
                self.app_lock.unlock_pending = false;
                self.app_lock.unlock_error = Some("Không xác thực được. Hãy thử lại.".into());
            }
            _ => {}
        }

        #[cfg(target_os = "macos")]
        let mut unlock_clicked = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(pal.bg_chat_canvas))
            .show(ctx, |ui| {
                egui::Area::new(Id::new("app_lock_screen"))
                    .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, 16.0))
                    .movable(false)
                    .fade_in(false)
                    .show(ui.ctx(), |ui| {
                        ui.set_max_width(360.0);
                        ui.vertical_centered(|ui| {
                            ui.heading(
                                RichText::new(crate::APP_DISPLAY_NAME).color(pal.text_primary),
                            );
                            ui.label(
                                RichText::new("Zalo-next đang khóa").color(pal.text_secondary),
                            );
                            ui.add_space(12.0);
                            if self
                                .active_call
                                .as_ref()
                                .is_some_and(|call| call.is_incoming_ringing)
                            {
                                ui.strong("Cuộc gọi đến");
                                if ui
                                    .add_enabled(
                                        !self.app_lock.unlock_pending,
                                        egui::Button::new("Mở khóa để trả lời"),
                                    )
                                    .clicked()
                                {
                                    match self.app_lock.config.method {
                                        Method::System => {
                                            #[cfg(target_os = "macos")]
                                            {
                                                unlock_clicked = true;
                                            }
                                        }
                                        Method::Passcode => self.app_lock.focus_passcode = true,
                                    }
                                }
                                if ui.button("Từ chối").clicked() {
                                    let sent = WORKER_TX.get().is_some_and(|tx| {
                                        tx.send(WorkerCmd::AnswerCall {
                                            accept: false,
                                            cam_start_off: false,
                                        })
                                        .is_ok()
                                    });
                                    if sent {
                                        self.active_call = None;
                                    } else {
                                        self.app_lock.unlock_error =
                                            Some(WORKER_UNAVAILABLE.into());
                                    }
                                }
                                ui.add_space(12.0);
                            }

                            match self.app_lock.config.method {
                                Method::System => {
                                    #[cfg(target_os = "macos")]
                                    {
                                        unlock_clicked |= ui
                                            .add_enabled(
                                                !self.app_lock.unlock_pending,
                                                egui::Button::new("Mở khóa"),
                                            )
                                            .clicked();
                                    }
                                }
                                Method::Passcode => self.render_passcode_unlock(ui),
                            }
                            if self.app_lock.unlock_pending {
                                ui.spinner();
                            }
                            if let Some(error) = &self.app_lock.unlock_error {
                                ui.label(RichText::new(error).color(pal.danger));
                            }
                        });
                    });
            });

        #[cfg(target_os = "macos")]
        if self.app_lock.config.method == Method::System
            && !self.app_lock.unlock_pending
            && (unlock_clicked || !self.app_lock.system_prompt_started)
            && system_auth_window_is_active(ctx)
        {
            self.app_lock.system_prompt_started = true;
            self.app_lock.unlock_pending = true;
            self.app_lock.unlock_error = None;
            let repaint_ctx = ctx.clone();
            super::macos_auth::authenticate("mở khóa Zalo-next", move |ok| {
                SYSTEM_AUTH_RESULT.store(if ok { 1 } else { 2 }, Ordering::Relaxed);
                repaint_ctx.request_repaint();
            });
            ctx.request_repaint();
        }
        true
    }

    fn render_passcode_unlock(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(!self.app_lock.unlock_pending, |ui| {
            let id = Id::new("app_lock_candidate");
            if std::mem::take(&mut self.app_lock.focus_passcode) {
                ui.memory_mut(|memory| memory.request_focus(id));
            }
            let response = password_field(ui, id, &mut self.app_lock.candidate, "Mã khóa");
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let clicked = ui.button("Mở khóa").clicked();
            if (enter || clicked) && !self.app_lock.candidate.is_empty() {
                let cmd = WorkerCmd::AppLockTryPasscode {
                    config: self.app_lock.config.clone(),
                    candidate: std::mem::take(&mut self.app_lock.candidate),
                    now_unix: crate::login::unix_now(),
                };
                if WORKER_TX.get().is_some_and(|tx| tx.send(cmd).is_ok()) {
                    self.app_lock.unlock_pending = true;
                    self.app_lock.unlock_error = None;
                } else {
                    self.app_lock.unlock_error = Some(WORKER_UNAVAILABLE.into());
                    self.app_lock.focus_passcode = true;
                }
            }
        });
    }

    pub(super) fn render_app_lock_settings(&mut self, ui: &mut egui::Ui, pal: &ThemePalette) {
        ui.label(
            RichText::new("Khóa ứng dụng")
                .color(pal.text_primary)
                .size(13.5)
                .strong(),
        );
        ui.add_space(6.0);
        egui::Frame::none()
            .fill(pal.bg_elevated)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(8.0))
            .inner_margin(Margin::symmetric(14.0, 10.0))
            .show(ui, |ui| {
                let before = self.app_lock.config.clone();
                ui.add_enabled_ui(self.app_lock.pending_saves == 0, |ui| {
                    let mut enabled = self.app_lock.config.enabled;
                    #[cfg(not(target_os = "macos"))]
                    {
                        enabled |= self.app_lock.passcode_setup;
                    }
                    ui.horizontal(|ui| {
                        ui.label("Bật khóa ứng dụng");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if render_toggle_switch(ui, &mut enabled, pal).changed() {
                                self.set_app_lock_enabled(enabled);
                            }
                        });
                    });

                    ui.horizontal(|ui| {
                        ui.label("Tự khóa khi không dùng");
                        egui::ComboBox::from_id_salt("app_lock_idle")
                            .selected_text(idle_label(self.app_lock.config.idle_minutes))
                            .show_ui(ui, |ui| {
                                for minutes in app_lock::IDLE_CHOICES_MIN {
                                    ui.selectable_value(&mut self.app_lock.config.idle_minutes, minutes, idle_label(minutes));
                                }
                            });
                    });
                    ui.checkbox(&mut self.app_lock.config.lock_on_sleep, "Khóa khi máy ngủ");
                });
                if self.app_lock.config != before {
                    self.save_app_lock();
                }
                #[cfg(not(target_os = "macos"))]
                if self.app_lock.passcode_setup {
                    self.render_passcode_setup(ui);
                }
                if ui.add_enabled(self.app_lock.config.usable(), egui::Button::new("Khóa ngay (Cmd+L / Ctrl+L)")).clicked() {
                    // Apply at the start of the next frame, before any unlocked content is drawn.
                    self.app_lock.lock_requested = true;
                    ui.ctx().request_repaint();
                }
                if self.app_lock.pending_saves > 0 {
                    ui.spinner();
                }
                if let Some(error) = &self.app_lock.settings_error {
                    ui.label(RichText::new(error).color(pal.danger));
                }
                ui.label(RichText::new("Khóa ứng dụng chỉ che giao diện và nội dung thông báo; dữ liệu trên máy không được mã hóa.").color(pal.text_secondary));
            });
    }

    fn set_app_lock_enabled(&mut self, enabled: bool) {
        self.app_lock.settings_error = None;
        if !enabled {
            self.app_lock.config.enabled = false;
            #[cfg(not(target_os = "macos"))]
            self.app_lock.clear_passcode_setup();
            return;
        }
        #[cfg(target_os = "macos")]
        {
            if super::macos_auth::available() {
                self.app_lock.config.method = app_lock::platform_method();
                self.app_lock.config.enabled = true;
            } else {
                self.app_lock.settings_error = Some(
                    "Máy chưa đặt mật khẩu đăng nhập nên chưa dùng được khóa ứng dụng.".into(),
                );
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.app_lock.passcode_setup = true;
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn render_passcode_setup(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.app_lock.pending_saves == 0, |ui| {
            password_field(
                ui,
                Id::new("app_lock_new"),
                &mut self.app_lock.new_passcode,
                "Mã khóa mới",
            );
            password_field(
                ui,
                Id::new("app_lock_repeat"),
                &mut self.app_lock.repeated_passcode,
                "Nhập lại",
            );
            if ui.button("Đặt mã khóa").clicked() {
                if let Err(error) = app_lock::validate_passcode(&self.app_lock.new_passcode) {
                    self.app_lock.settings_error = Some(error.to_string());
                } else if self.app_lock.new_passcode != self.app_lock.repeated_passcode {
                    self.app_lock.settings_error = Some("Hai mã khóa không khớp.".into());
                } else {
                    let cmd = WorkerCmd::AppLockSetPasscode {
                        config: self.app_lock.config.clone(),
                        passcode: std::mem::take(&mut self.app_lock.new_passcode),
                    };
                    self.app_lock.clear_passcode_setup();
                    self.send_app_lock_save(cmd);
                }
            }
        });
    }
}

#[cfg(target_os = "macos")]
fn system_auth_window_is_active(ctx: &egui::Context) -> bool {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;

    ctx.input(|i| {
        i.focused && i.viewport().minimized != Some(true) && !i.viewport().close_requested()
    }) && MainThreadMarker::new().is_some_and(|mtm| {
        // SAFETY: The shared application is queried on the main thread.
        !unsafe { NSApplication::sharedApplication(mtm).isHidden() }
    })
}

fn password_field(ui: &mut egui::Ui, id: Id, text: &mut String, hint: &str) -> egui::Response {
    let output = egui::TextEdit::singleline(text)
        .id(id)
        .hint_text(hint)
        .password(true)
        .char_limit(app_lock::MAX_PASSCODE_CHARS + 1)
        .desired_width(220.0)
        .show(ui);
    // TextEdit normally retains previous text even after the field is cleared.
    let mut state = output.state;
    state.clear_undoer();
    state.store(ui.ctx(), id);
    output.response
}

fn may_lock(call_is_ringing: Option<bool>) -> bool {
    call_is_ringing != Some(false)
}

fn attempt_message(attempt: Attempt) -> Option<String> {
    match attempt {
        Attempt::Unlocked => None,
        Attempt::Wrong { left } => Some(format!("Sai mã khóa, còn {left} lần thử.")),
        Attempt::CoolingDown { secs } => Some(format!("Thử lại sau {secs} giây.")),
    }
}

fn idle_label(minutes: u32) -> String {
    if minutes == 0 {
        "Không tự khóa".into()
    } else {
        format!("Sau {minutes} phút")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_allowed_without_a_call_or_while_ringing() {
        assert!(may_lock(None));
        assert!(may_lock(Some(true)));
        assert!(!may_lock(Some(false)));
    }

    #[test]
    fn attempt_message_describes_wrong_passcodes_and_cooldown() {
        assert_eq!(
            attempt_message(Attempt::Wrong { left: 4 }).as_deref(),
            Some("Sai mã khóa, còn 4 lần thử.")
        );
        assert_eq!(
            attempt_message(Attempt::CoolingDown { secs: 30 }).as_deref(),
            Some("Thử lại sau 30 giây.")
        );
        assert_eq!(attempt_message(Attempt::Unlocked), None);
    }

    #[test]
    fn idle_choices_distinguish_disabled_from_a_timeout() {
        assert_eq!(idle_label(0), "Không tự khóa");
        assert_eq!(idle_label(15), "Sau 15 phút");
    }
}
