//! App Lock state and screens. The worker owns passcode hashing and persistence.

use std::time::{Duration, Instant, SystemTime};

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU8, Ordering};

use super::*;
use crate::app_lock::{Attempt, AutoLock, Config, Method};

#[cfg(target_os = "macos")]
pub(super) const LOAD_ERROR: &str =
    "Không đọc được cấu hình khóa ứng dụng; hãy xác thực bằng tài khoản macOS.";
#[cfg(not(target_os = "macos"))]
pub(super) const LOAD_ERROR: &str = "Không đọc được cấu hình khóa ứng dụng; khóa đang tắt.";
const WORKER_UNAVAILABLE: &str = "Tác vụ nền chưa sẵn sàng.";

#[cfg(target_os = "macos")]
static SYSTEM_AUTH_RESULT: AtomicU8 = AtomicU8::new(0);

pub(super) struct State {
    config: Config,
    saved: Config,
    auto: AutoLock,
    lock_requested: bool,
    lock_deferred: bool,
    candidate: String,
    focus_passcode: bool,
    unlock_pending: bool,
    unlock_retry_at: Option<Instant>,
    unlock_error: Option<String>,
    pub(super) load_error: Option<&'static str>,
    settings_error: Option<String>,
    pending_save: Option<Config>,
    #[cfg(not(target_os = "macos"))]
    passcode_setup: bool,
    #[cfg(not(target_os = "macos"))]
    new_passcode: String,
    #[cfg(not(target_os = "macos"))]
    repeated_passcode: String,
}

impl State {
    pub(super) fn new(config: Config, load_error: Option<&'static str>) -> Self {
        // The system-auth fallback on a load error has never been saved.
        let saved = if load_error.is_some() {
            Config::default()
        } else {
            config.clone()
        };
        Self {
            config,
            saved,
            auto: AutoLock::default(),
            lock_requested: false,
            lock_deferred: false,
            candidate: String::new(),
            focus_passcode: true,
            unlock_pending: false,
            unlock_retry_at: None,
            unlock_error: None,
            load_error,
            settings_error: None,
            pending_save: None,
            #[cfg(not(target_os = "macos"))]
            passcode_setup: false,
            #[cfg(not(target_os = "macos"))]
            new_passcode: String::new(),
            #[cfg(not(target_os = "macos"))]
            repeated_passcode: String::new(),
        }
    }

    fn finish_save(&mut self, config: Option<Config>, error: Option<&str>) {
        let Some(submitted) = self.pending_save.take() else {
            return;
        };
        self.settings_error = error.map(str::to_owned);
        if error.is_none() {
            // Plain saves acknowledge the submitted snapshot; passcode setup returns the hash.
            self.saved = config.unwrap_or(submitted);
            self.load_error = None;
            self.auto.note_input(Instant::now());
        }
        self.config = self.saved.clone();
    }

    fn can_unlock(&self, now: Instant) -> bool {
        !self.unlock_pending
            && self.pending_save.is_none()
            && self.unlock_retry_at.is_none_or(|retry_at| now >= retry_at)
    }

    fn take_lock_request(&mut self, auto: bool, call_is_connected: Option<bool>) -> bool {
        let manual = std::mem::take(&mut self.lock_requested);
        let deferred = std::mem::take(&mut self.lock_deferred);
        if !self.saved.usable() || !(manual || auto || deferred) {
            return false;
        }
        self.lock_deferred = !may_lock(call_is_connected);
        !self.lock_deferred
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
        let allow_shortcut = self.app_lock.saved.usable() && !app_lock::is_locked();
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
            .tick(&self.app_lock.saved, now, SystemTime::now());
        self.handle_app_lock_input(ctx);
        #[cfg(not(target_os = "macos"))]
        if !self.settings_modal_open || self.settings_tab != SettingsTab::Privacy {
            self.app_lock.clear_passcode_setup();
        }

        if app_lock::is_locked() {
            return;
        }
        let manual = self.app_lock.lock_requested;
        let call_is_connected = self.active_call.as_ref().map(|call| call.is_connected);
        if !self.app_lock.take_lock_request(auto, call_is_connected) {
            if manual && self.app_lock.lock_deferred {
                self.show_toast("Sẽ khóa ứng dụng khi cuộc gọi kết thúc.");
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
        self.app_lock.unlock_retry_at = None;
        self.app_lock.unlock_error = None;
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
        if save_error.is_none() {
            self.app_lock.saved = config.clone();
            self.app_lock.load_error = None;
        }
        self.app_lock.config = config;
        self.app_lock.unlock_pending = false;
        self.app_lock.focus_passcode = true;
        if let Some(mut message) = attempt_message(attempt) {
            if let Some(error) = save_error {
                let secs = match attempt {
                    Attempt::CoolingDown { secs } => secs,
                    _ => 30,
                };
                self.app_lock.unlock_retry_at =
                    Some(Instant::now() + Duration::from_secs(secs as u64));
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
        self.app_lock.finish_save(config, error.as_deref());
        if let Some(error) = error {
            self.show_toast(error);
        }
    }

    fn send_app_lock_save(&mut self, cmd: WorkerCmd) {
        self.app_lock.pending_save = Some(self.app_lock.config.clone());
        if WORKER_TX.get().is_some_and(|tx| tx.send(cmd).is_ok()) {
            self.app_lock.settings_error = None;
        } else {
            self.finish_app_lock_save(None, Some(WORKER_UNAVAILABLE.into()));
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
        let now = Instant::now();
        let can_unlock = self.app_lock.can_unlock(now);
        if let Some(retry_at) = self.app_lock.unlock_retry_at {
            if retry_at > now {
                ctx.request_repaint_after(retry_at - now);
            }
        }
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
                            if let Some(error) = self.app_lock.load_error {
                                ui.label(RichText::new(error).color(pal.danger));
                            }
                            ui.add_space(12.0);
                            if self
                                .active_call
                                .as_ref()
                                .is_some_and(|call| call.is_incoming_ringing)
                            {
                                ui.strong("Cuộc gọi đến");
                                if ui
                                    .add_enabled(
                                        can_unlock,
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
                                            mic_start_off: false,
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
                                            .add_enabled(can_unlock, egui::Button::new("Mở khóa"))
                                            .clicked();
                                    }
                                }
                                Method::Passcode => self.render_passcode_unlock(ui, can_unlock),
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
            && can_unlock
            // Only an explicit click starts the system prompt (owner's request): locking, waking or
            // bringing the window back never pops Touch ID by itself.
            && unlock_clicked
            && system_auth_window_is_active(ctx)
        {
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

    fn render_passcode_unlock(&mut self, ui: &mut egui::Ui, can_unlock: bool) {
        ui.add_enabled_ui(can_unlock, |ui| {
            let id = Id::new("app_lock_candidate");
            if std::mem::take(&mut self.app_lock.focus_passcode) {
                ui.memory_mut(|memory| memory.request_focus(id));
            }
            let response = password_field(ui, id, &mut self.app_lock.candidate, "Mã khóa");
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let clicked = ui.button("Mở khóa").clicked();
            if can_unlock && (enter || clicked) && !self.app_lock.candidate.is_empty() {
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
                ui.add_enabled_ui(self.app_lock.pending_save.is_none(), |ui| {
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
                if ui.add_enabled(self.app_lock.saved.usable(), egui::Button::new("Khóa ngay (Cmd+L / Ctrl+L)")).clicked() {
                    // Apply at the start of the next frame, before any unlocked content is drawn.
                    self.app_lock.lock_requested = true;
                    ui.ctx().request_repaint();
                }
                if self.app_lock.pending_save.is_some() {
                    ui.spinner();
                }
                if let Some(error) = self.app_lock.load_error {
                    ui.label(RichText::new(error).color(pal.danger));
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
        ui.add_enabled_ui(self.app_lock.pending_save.is_none(), |ui| {
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

fn may_lock(call_is_connected: Option<bool>) -> bool {
    call_is_connected != Some(true)
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
    fn only_a_connected_call_blocks_locking() {
        assert!(may_lock(None));
        assert!(may_lock(Some(false)));
        assert!(!may_lock(Some(true)));
    }

    #[test]
    fn deferred_lock_fires_when_a_connected_call_ends_or_fails() {
        let mut config = Config::default();
        config.enabled = true;
        config.method = Method::System;
        for (manual, call_after) in [(true, None), (false, Some(false))] {
            let mut state = State::new(config.clone(), None);
            state.lock_requested = manual;

            assert!(!state.take_lock_request(!manual, Some(true)));
            assert!(state.lock_deferred);
            assert!(!state.take_lock_request(false, Some(true)));
            assert!(state.take_lock_request(false, call_after));
            assert!(!state.take_lock_request(false, call_after));
        }
    }

    #[test]
    fn failed_call_overlay_allows_immediate_lock() {
        let mut config = Config::default();
        config.enabled = true;
        config.method = Method::System;
        let mut state = State::new(config, None);
        state.lock_requested = true;

        assert!(state.take_lock_request(false, Some(false)));
        assert!(!state.lock_deferred);
    }

    #[test]
    fn settings_save_failure_restores_last_saved_config() {
        let mut state = State::new(Config::default(), None);
        state.config.idle_minutes = 5;
        state.pending_save = Some(state.config.clone());
        state.finish_save(None, None);
        state.config.enabled = true;
        state.config.idle_minutes = 30;
        state.config.lock_on_sleep = false;
        state.pending_save = Some(state.config.clone());

        let error = "Không lưu được cấu hình khóa ứng dụng.";
        state.finish_save(None, Some(error));

        assert_eq!(state.config, state.saved);
        assert!(!state.config.enabled);
        assert_eq!(state.config.idle_minutes, 5);
        assert!(state.config.lock_on_sleep);
        assert_eq!(state.settings_error.as_deref(), Some(error));
        assert!(state.pending_save.is_none());
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
