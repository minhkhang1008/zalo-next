//! GUI side of the local `@AI` assistant (see `src/ai.rs`, `docs/AI_TRO_LY.md`, ADR-0004).
//!
//! Assistant messages are kept in `State::messages`, a map **outside** `Thread.messages`, and drawn
//! between the real messages by timestamp. Because no Zalo code path ever sees them (seen
//! receipts, unread counts, persistence, merge, forward, quote, search...), INV-5 holds by
//! construction. All network and SQLite work goes through `WorkerCmd::Ai*` (INV-3).

use super::*;
use crate::ai::{self, AiMessage, AiRole, AskRequest, TranscriptLine};

/// Quick prompts offered while the composer starts with `@AI`.
const QUICK_PROMPTS: &[(&str, &str)] = &[
    ("Tóm tắt cuộc trò chuyện", "@AI Tóm tắt cuộc trò chuyện này"),
    (
        "Gợi ý câu trả lời",
        "@AI Gợi ý giúp tôi một câu trả lời cho tin nhắn gần nhất",
    ),
    (
        "Việc cần làm?",
        "@AI Liệt kê các việc cần làm và mốc thời gian được nhắc tới",
    ),
];

#[derive(Default)]
pub(super) struct State {
    /// Dialogue per conversation id, oldest first.
    pub messages: HashMap<String, Vec<AiMessage>>,
    /// Conversations whose stored dialogue was requested from the worker.
    pub loaded: HashSet<String>,
    /// Conversations waiting for an answer.
    pub thinking: HashSet<String>,
    /// Last request per conversation, for "Thử lại".
    pub last_req: HashMap<String, AskRequest>,
    /// A group request waiting for the one-time "share this conversation?" confirmation.
    pub pending_confirm: Option<AskRequest>,
    /// Action chosen in a bubble this frame; handled after the message list is drawn.
    pub pending_action: Option<AiAction>,
    /// Typed API key (never stored in settings; sent to the worker on "Lưu").
    pub key_input: String,
    pub key_set: bool,
    pub status: String,
    pub busy: bool,
    pub models: Vec<String>,
    /// Account the in-memory dialogue belongs to; a change drops it (each account has its own DB).
    pub account: String,
}

impl State {
    pub(super) fn new() -> Self {
        Self {
            key_set: ai::has_api_key(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum AiAction {
    Copy(String),
    Reply(String),
    Delete(String),
    Regenerate(String),
    Retry,
}

fn hhmm(ts_ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ts_ms)
        .single()
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_default()
}

fn day_time(ts_ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ts_ms)
        .single()
        .map(|d| d.format("%d/%m %H:%M").to_string())
        .unwrap_or_default()
}

/// Conversation text to share: text only (no files or links), oldest first, newest 800 messages,
/// capped at `ai::TRANSCRIPT_CAP_CHARS`.
pub(super) fn transcript_for(thread: &Thread) -> Vec<TranscriptLine> {
    let mut lines: Vec<TranscriptLine> = thread
        .messages
        .iter()
        .filter(|m| !is_system_notice_message(m))
        .rev()
        .take(800)
        .map(|m| {
            let who = if m.is_outgoing {
                "Tôi".to_string()
            } else if !m.sender.trim().is_empty() && !m.sender.chars().all(|c| c.is_ascii_digit()) {
                m.sender.clone()
            } else if !thread.is_group {
                thread.name.clone()
            } else {
                "Thành viên".to_string()
            };
            let text = if m.text.trim().is_empty() {
                "[tệp hoặc hình ảnh]".to_string()
            } else {
                m.text.trim().to_string()
            };
            TranscriptLine {
                time: day_time(normalize_ts_ms(m.ts)),
                who,
                text,
            }
        })
        .collect();
    lines.reverse();
    let mut total = 0usize;
    let mut keep_from = 0usize;
    for (i, l) in lines.iter().enumerate().rev() {
        total += l.text.chars().count() + l.who.chars().count() + 24;
        if total > ai::TRANSCRIPT_CAP_CHARS {
            keep_from = i + 1;
            break;
        }
    }
    lines.drain(..keep_from);
    lines
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "máy chủ AI".to_string())
}

impl ZaloGuiApp {
    /// Context sharing for a conversation: its own choice, else the global default (off).
    pub(super) fn ai_share_enabled(&self, thread_id: &str) -> bool {
        self.settings
            .ai_share_threads
            .get(thread_id)
            .copied()
            .unwrap_or(self.settings.ai_share_default)
    }

    /// True when Enter in the composer should go to the Assistant instead of Zalo.
    pub(super) fn ai_intercepts_send(&self) -> bool {
        self.settings.ai_enabled
            && self.pending_clipboard_image.is_none()
            && self.pending_drop_files.is_empty()
            && (ai::parse_ai_command(&self.input_message).is_some()
                || self
                    .replying_to
                    .as_ref()
                    .is_some_and(|q| ai::is_ai_id(&q.id)))
    }

    /// Take the composer text and start a request. Nothing is sent to Zalo.
    pub(super) fn ai_submit_from_composer(&mut self, thread_id: &str) {
        let replying_ai = self
            .replying_to
            .as_ref()
            .is_some_and(|q| ai::is_ai_id(&q.id));
        let text = self.input_message.trim().to_string();
        let prompt = match ai::parse_ai_command(&text) {
            Some(rest) => rest.to_string(),
            None if replying_ai => text,
            None => return,
        };
        if prompt.is_empty() {
            self.show_toast("Nhập câu hỏi sau @AI, ví dụ: @AI Rust là gì");
            return;
        }
        self.input_message.clear();
        let quoted = self
            .replying_to
            .take()
            .filter(|q| ai::is_ai_id(&q.id))
            .map(|q| (q.id, q.text));
        self.reply_auto_mention = None;
        self.ai_begin_ask(thread_id, prompt, quoted);
    }

    fn ai_build_request(
        &self,
        thread_id: &str,
        prompt: String,
        quoted: Option<(String, String)>,
        user_msg: Option<AiMessage>,
        user_msg_id: String,
    ) -> Option<AskRequest> {
        let thread = self.threads.iter().find(|t| t.id == thread_id)?;
        let share = self.ai_share_enabled(thread_id);
        Some(AskRequest {
            thread_id: thread_id.to_string(),
            thread_name: thread.name.clone(),
            is_group: thread.is_group,
            prompt,
            user_msg,
            user_msg_id,
            reply_to_id: quoted.as_ref().map(|(id, _)| id.clone()),
            quoted: quoted.map(|(_, text)| text),
            transcript: if share {
                transcript_for(thread)
            } else {
                Vec::new()
            },
            share,
            base_url: self.settings.ai_base_url.clone(),
            model: self.settings.ai_model.clone(),
        })
    }

    pub(super) fn ai_begin_ask(
        &mut self,
        thread_id: &str,
        prompt: String,
        quoted: Option<(String, String)>,
    ) {
        let id = ai::make_id();
        let user_msg = AiMessage {
            id: id.clone(),
            thread_id: thread_id.to_string(),
            role: AiRole::User,
            text: prompt.clone(),
            ts: ai::now_ms(),
            reply_to_id: quoted.as_ref().map(|(qid, _)| qid.clone()),
            model: None,
            note: None,
        };
        let Some(req) = self.ai_build_request(thread_id, prompt, quoted, Some(user_msg), id) else {
            return;
        };
        let needs_confirm =
            req.share && req.is_group && !self.settings.ai_group_confirmed.contains(thread_id);
        if needs_confirm {
            self.ai.pending_confirm = Some(req);
        } else {
            self.ai_dispatch(req);
        }
    }

    fn ai_dispatch(&mut self, req: AskRequest) {
        let thread_id = req.thread_id.clone();
        if let Some(m) = &req.user_msg {
            self.ai
                .messages
                .entry(thread_id.clone())
                .or_default()
                .push(m.clone());
        }
        self.ai.thinking.insert(thread_id.clone());
        self.force_scroll_to_bottom = true;
        self.ai.last_req.insert(thread_id.clone(), req.clone());
        match WORKER_TX.get() {
            Some(tx) if tx.send(WorkerCmd::AiAsk { req }).is_ok() => {}
            _ => {
                self.ai.thinking.remove(&thread_id);
                self.ai_push_error(&thread_id, "Tác vụ nền chưa sẵn sàng, hãy thử lại.".into());
            }
        }
    }

    fn ai_push_error(&mut self, thread_id: &str, text: String) {
        let list = self.ai.messages.entry(thread_id.to_string()).or_default();
        list.retain(|m| m.role != AiRole::Error);
        list.push(AiMessage {
            id: ai::make_id(),
            thread_id: thread_id.to_string(),
            role: AiRole::Error,
            text,
            ts: ai::now_ms(),
            reply_to_id: None,
            model: None,
            note: None,
        });
    }

    // ---- worker events ----------------------------------------------------------------------

    pub(super) fn ai_on_thread_loaded(&mut self, thread_id: String, mut messages: Vec<AiMessage>) {
        // Keep anything created while the load was in flight.
        let entry = self.ai.messages.entry(thread_id).or_default();
        let known: HashSet<String> = entry.iter().map(|m| m.id.clone()).collect();
        messages.retain(|m| !known.contains(&m.id));
        entry.extend(messages);
        entry.sort_by_key(|m| m.ts);
    }

    pub(super) fn ai_on_reply(&mut self, msg: AiMessage) {
        self.ai.thinking.remove(&msg.thread_id);
        let list = self.ai.messages.entry(msg.thread_id.clone()).or_default();
        list.retain(|m| m.role != AiRole::Error);
        list.push(msg);
        self.force_scroll_to_bottom = true;
    }

    pub(super) fn ai_on_failed(&mut self, thread_id: String, error: String) {
        self.ai.thinking.remove(&thread_id);
        self.ai_push_error(&thread_id, error);
    }

    pub(super) fn ai_on_test_result(&mut self, result: Result<String, String>) {
        self.ai.busy = false;
        self.ai.status = match result {
            Ok(note) => format!("✅ {note}"),
            Err(e) => format!("⚠ {e}"),
        };
    }

    pub(super) fn ai_on_models(&mut self, result: Result<Vec<String>, String>) {
        self.ai.busy = false;
        match result {
            Ok(models) => {
                self.ai.status =
                    format!("Có {} model. Chọn một model trong danh sách.", models.len());
                self.ai.models = models;
            }
            Err(e) => self.ai.status = format!("⚠ {e}"),
        }
    }

    // ---- actions chosen in bubbles ----------------------------------------------------------

    pub(super) fn ai_handle_pending_action(&mut self, ctx: &egui::Context) {
        if self.ai.account != self.user_uin {
            // Logout / account switch: the dialogue belongs to the previous account's database.
            self.ai.messages.clear();
            self.ai.loaded.clear();
            self.ai.thinking.clear();
            self.ai.last_req.clear();
            self.ai.pending_confirm = None;
            self.ai.account = self.user_uin.clone();
        }
        let Some(action) = self.ai.pending_action.take() else {
            return;
        };
        let Some(thread_id) = self
            .threads
            .get(self.selected_thread_index)
            .map(|t| t.id.clone())
        else {
            return;
        };
        match action {
            AiAction::Copy(text) => {
                ctx.copy_text(text);
                self.show_toast("Đã sao chép");
            }
            AiAction::Reply(id) => {
                let found = self
                    .ai
                    .messages
                    .get(&thread_id)
                    .and_then(|l| l.iter().find(|m| m.id == id))
                    .cloned();
                if let Some(m) = found {
                    // Replying to an AI message continues the dialogue with the Assistant; the
                    // composer sends it to `ai_submit_from_composer`, never to Zalo.
                    self.replying_to = Some(ChatMessage {
                        id: m.id,
                        sender: if m.role == AiRole::User {
                            "Bạn"
                        } else {
                            "AI"
                        }
                        .into(),
                        text: m.text,
                        ts: m.ts,
                        ..Default::default()
                    });
                }
            }
            AiAction::Delete(id) => {
                if let Some(list) = self.ai.messages.get_mut(&thread_id) {
                    list.retain(|m| m.id != id);
                }
                if ai::is_ai_id(&id) {
                    if let Some(tx) = WORKER_TX.get() {
                        let _ = tx.send(WorkerCmd::AiDelete { msg_id: id });
                    }
                }
            }
            AiAction::Regenerate(id) => {
                let list = self
                    .ai
                    .messages
                    .get(&thread_id)
                    .cloned()
                    .unwrap_or_default();
                let Some(pos) = list.iter().position(|m| m.id == id) else {
                    return;
                };
                let Some(question) = list[..pos]
                    .iter()
                    .rev()
                    .find(|m| m.role == AiRole::User)
                    .cloned()
                else {
                    return;
                };
                if let Some(l) = self.ai.messages.get_mut(&thread_id) {
                    l.retain(|m| m.id != id);
                }
                if let Some(tx) = WORKER_TX.get() {
                    let _ = tx.send(WorkerCmd::AiDelete { msg_id: id });
                }
                if let Some(req) = self.ai_build_request(
                    &thread_id,
                    question.text.clone(),
                    None,
                    None,
                    question.id,
                ) {
                    self.ai_dispatch(req);
                }
            }
            AiAction::Retry => {
                if let Some(mut req) = self.ai.last_req.get(&thread_id).cloned() {
                    req.user_msg = None; // already shown and stored
                                         // Rebuild the transcript so a retry sees the latest messages.
                    if let Some(fresh) = self.ai_build_request(
                        &thread_id,
                        req.prompt.clone(),
                        req.quoted
                            .clone()
                            .map(|q| (req.reply_to_id.clone().unwrap_or_default(), q)),
                        None,
                        req.user_msg_id.clone(),
                    ) {
                        req = fresh;
                    }
                    if let Some(l) = self.ai.messages.get_mut(&thread_id) {
                        l.retain(|m| m.role != AiRole::Error);
                    }
                    self.ai_dispatch(req);
                }
            }
        }
    }

    // ---- dialogs and composer hints ---------------------------------------------------------

    /// One-time confirmation before a group conversation is shared with the Assistant.
    pub(super) fn render_ai_confirm(&mut self, ctx: &egui::Context, pal: &ThemePalette) {
        let Some(req) = self.ai.pending_confirm.clone() else {
            return;
        };
        let host = host_of(&req.base_url);
        let mut decision: Option<u8> = None; // 1 allow, 2 without context, 3 cancel
        egui::Window::new("Chia sẻ nhóm với AI?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::none()
                    .fill(pal.bg_elevated)
                    .stroke(Stroke::new(1.0_f32, pal.border))
                    .rounding(Rounding::same(12.0))
                    .inner_margin(Margin::same(18.0)),
            )
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                ui.label(
                    RichText::new(format!(
                        "Để trả lời, AI cần đọc tin nhắn gần đây của nhóm \"{}\". Văn bản tin nhắn của tất cả thành viên sẽ được gửi tới {host}.",
                        req.thread_name
                    ))
                    .color(pal.text_primary)
                    .size(13.2),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Chỉ gửi văn bản (không gửi liên kết, ảnh hay tệp). Lựa chọn này được nhớ cho nhóm này và có thể tắt bất cứ lúc nào ở khung soạn tin.")
                        .color(pal.text_secondary)
                        .size(12.0),
                );
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.button(RichText::new("Cho phép và gửi").strong()).clicked() {
                        decision = Some(1);
                    }
                    if ui.button("Chỉ hỏi, không kèm cuộc trò chuyện").clicked() {
                        decision = Some(2);
                    }
                    if ui.button("Hủy").clicked() {
                        decision = Some(3);
                    }
                });
            });
        match decision {
            Some(1) => {
                self.settings
                    .ai_group_confirmed
                    .insert(req.thread_id.clone());
                self.settings
                    .ai_share_threads
                    .insert(req.thread_id.clone(), true);
                self.settings.save();
                self.ai.pending_confirm = None;
                self.ai_dispatch(req);
            }
            Some(2) => {
                self.settings
                    .ai_share_threads
                    .insert(req.thread_id.clone(), false);
                self.settings.save();
                self.ai.pending_confirm = None;
                let mut req = req;
                req.share = false;
                req.transcript.clear();
                self.ai_dispatch(req);
            }
            Some(_) => self.ai.pending_confirm = None,
            None => {}
        }
    }

    /// Hint row above the composer while it starts with `@AI`: privacy state + quick prompts.
    pub(super) fn render_ai_composer_hint(
        &mut self,
        ui: &mut egui::Ui,
        pal: &ThemePalette,
        thread_id: &str,
    ) {
        if !self.settings.ai_enabled || ai::parse_ai_command(&self.input_message).is_none() {
            return;
        }
        let share = self.ai_share_enabled(thread_id);
        let host = host_of(&self.settings.ai_base_url);
        egui::Frame::none()
            .fill(pal.bg_selected)
            .rounding(Rounding::same(8.0))
            .inner_margin(Margin::symmetric(10.0, 6.0))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!("{} AI · chỉ mình bạn thấy", lucide::SPARKLES))
                            .color(pal.primary_hover)
                            .size(12.2)
                            .strong(),
                    );
                    let (label, tip) = if share {
                        (
                            "AI đọc cuộc trò chuyện: Bật",
                            format!("Văn bản tin nhắn gần đây sẽ được gửi tới {host}. Bấm để tắt."),
                        )
                    } else {
                        (
                            "AI đọc cuộc trò chuyện: Tắt",
                            "AI chỉ thấy câu hỏi của bạn. Bấm để bật cho cuộc trò chuyện này."
                                .to_string(),
                        )
                    };
                    if ui
                        .small_button(RichText::new(label).size(11.8))
                        .on_hover_text(tip)
                        .clicked()
                    {
                        self.settings
                            .ai_share_threads
                            .insert(thread_id.to_string(), !share);
                        self.settings.save();
                    }
                    for (title, text) in QUICK_PROMPTS {
                        if ui.small_button(RichText::new(*title).size(11.8)).clicked() {
                            self.input_message = (*text).to_string();
                        }
                    }
                });
            });
        ui.add_space(4.0);
    }

    // ---- settings tab -----------------------------------------------------------------------

    pub(super) fn render_assistant_settings(
        &mut self,
        ui: &mut egui::Ui,
        pal: &ThemePalette,
        inner_w: f32,
    ) {
        let card = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
            egui::Frame::none()
                .fill(pal.bg_elevated)
                .stroke(Stroke::new(1.0_f32, pal.border))
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::symmetric(14.0, 10.0))
                .show(ui, |ui| {
                    let w = (inner_w - 28.0).max(280.0);
                    ui.set_min_width(w);
                    ui.set_max_width(w);
                    add(ui);
                });
        };
        let label = |ui: &mut egui::Ui, t: &str| {
            ui.label(RichText::new(t).color(pal.text_secondary).size(12.2));
        };
        let mut save_settings = false;
        let mut send: Vec<WorkerCmd> = Vec::new();

        ui.label(
            RichText::new("Trợ lý AI")
                .color(pal.text_primary)
                .size(13.5)
                .strong(),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new("Gõ @AI <câu hỏi> trong bất kỳ cuộc trò chuyện nào: AI trả lời như một người thứ ba, chỉ mình bạn thấy, không gửi gì tới Zalo.")
                .color(pal.text_secondary)
                .size(12.0),
        );
        ui.add_space(8.0);
        card(ui, &mut |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Bật trợ lý AI")
                        .color(pal.text_primary)
                        .size(13.2),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if render_toggle_switch(ui, &mut self.settings.ai_enabled, pal).changed() {
                        save_settings = true;
                    }
                });
            });
            ui.add_space(8.0);
            label(ui, "Base URL (tương thích OpenAI)");
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.ai_base_url)
                    .hint_text("https://api.example.com/v1")
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(6.0);
            label(ui, "API key");
            ui.add(
                egui::TextEdit::singleline(&mut self.ai.key_input)
                    .password(true)
                    .hint_text(if self.ai.key_set {
                        "Đã lưu (nhập key mới để thay)"
                    } else {
                        "sk-…"
                    })
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(6.0);
            label(ui, "Model");
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.ai_model)
                        .hint_text("gpt-4o-mini, deepseek-chat, llama3…")
                        .desired_width((inner_w - 190.0).max(160.0)),
                );
                if ui.button("Lấy danh sách").clicked() {
                    self.ai.busy = true;
                    self.ai.status = "Đang lấy danh sách model…".into();
                    send.push(WorkerCmd::AiListModels {
                        base_url: self.settings.ai_base_url.clone(),
                        model: self.settings.ai_model.clone(),
                    });
                }
            });
            if !self.ai.models.is_empty() {
                egui::ComboBox::from_id_salt("ai_model_pick")
                    .selected_text("Chọn model…")
                    .width(inner_w - 60.0)
                    .show_ui(ui, |ui| {
                        for m in self.ai.models.clone() {
                            if ui
                                .selectable_label(self.settings.ai_model == m, &m)
                                .clicked()
                            {
                                self.settings.ai_model = m;
                            }
                        }
                    });
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button(RichText::new("Lưu").strong()).clicked() {
                    save_settings = true;
                    if !self.ai.key_input.trim().is_empty() {
                        send.push(WorkerCmd::AiSaveKey {
                            key: std::mem::take(&mut self.ai.key_input),
                        });
                        self.ai.key_set = true;
                    }
                    self.ai.status = "Đã lưu cấu hình.".into();
                }
                if ui
                    .add_enabled(!self.ai.busy, egui::Button::new("Kiểm tra kết nối"))
                    .clicked()
                {
                    // Save first so the test uses exactly what is on screen.
                    save_settings = true;
                    if !self.ai.key_input.trim().is_empty() {
                        send.push(WorkerCmd::AiSaveKey {
                            key: std::mem::take(&mut self.ai.key_input),
                        });
                        self.ai.key_set = true;
                    }
                    self.ai.busy = true;
                    self.ai.status = "Đang kiểm tra kết nối…".into();
                    send.push(WorkerCmd::AiTestConnection {
                        base_url: self.settings.ai_base_url.clone(),
                        model: self.settings.ai_model.clone(),
                    });
                }
                if self.ai.key_set && ui.button("Xóa API key").clicked() {
                    send.push(WorkerCmd::AiSaveKey { key: String::new() });
                    self.ai.key_set = false;
                    self.ai.status = "Đã xóa API key.".into();
                }
            });
            if !self.ai.status.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(&self.ai.status)
                        .color(pal.text_primary)
                        .size(12.4),
                );
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new("API key lưu trong tệp riêng tư ai_secret.json (quyền 0600), không nằm trong app_settings.json. Biến môi trường CTDOZ_BASE_URL / CTDOZ_API_KEY / CTDOZ_MODEL luôn ưu tiên hơn cài đặt.")
                    .color(pal.text_muted)
                    .size(11.6),
            );
        });

        ui.add_space(14.0);
        ui.label(
            RichText::new("Quyền riêng tư")
                .color(pal.text_primary)
                .size(13.5)
                .strong(),
        );
        ui.add_space(6.0);
        card(ui, &mut |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Chia sẻ nội dung cuộc trò chuyện với AI").color(pal.text_primary).size(13.2));
                    ui.label(
                        RichText::new("Mặc định cho mọi cuộc trò chuyện. Khi bật, AI đọc văn bản các tin gần đây để trả lời và tóm tắt tốt hơn. Có thể đổi riêng từng cuộc ngay ở khung soạn tin khi gõ @AI.")
                            .color(pal.text_secondary)
                            .size(11.8),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if render_toggle_switch(ui, &mut self.settings.ai_share_default, pal).changed() {
                        save_settings = true;
                    }
                });
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("Nếu bật, văn bản tin nhắn sẽ được gửi tới {}. Liên kết, ảnh và tệp không bao giờ được gửi. Nhóm chat luôn hỏi xác nhận lần đầu.", host_of(&self.settings.ai_base_url)))
                    .color(pal.text_muted)
                    .size(11.6),
            );
        });

        if save_settings && !self.settings.save() {
            self.show_toast("Không thể lưu cài đặt: tác vụ nền chưa sẵn sàng.");
        }
        if let Some(tx) = WORKER_TX.get() {
            for cmd in send {
                let _ = tx.send(cmd);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Bubbles
// ---------------------------------------------------------------------------------------------

/// Draw one Assistant message (a question you asked, an answer, or an error). Returns the action
/// picked from its context menu / buttons.
pub(super) fn render_ai_bubble(
    ui: &mut egui::Ui,
    msg: &AiMessage,
    pal: &ThemePalette,
    textures: &HashMap<String, TextureHandle>,
    loading: &mut HashSet<String>,
) -> Option<AiAction> {
    let mut action = None;
    let is_user = msg.role == AiRole::User;
    let is_error = msg.role == AiRole::Error;
    let max_w = (ui.available_width() * 0.78).clamp(180.0, 560.0);
    let (fill, ink, meta, stroke_col) = if is_error {
        (
            pal.danger.linear_multiply(0.14),
            pal.text_primary,
            pal.text_secondary,
            pal.danger.linear_multiply(0.6),
        )
    } else if is_user {
        (
            pal.bubble_out_bg,
            pal.bubble_out_text,
            pal.bubble_out_meta,
            pal.primary.linear_multiply(0.55),
        )
    } else {
        (
            pal.bg_elevated,
            pal.bubble_in_text,
            pal.bubble_in_meta,
            pal.primary.linear_multiply(0.55),
        )
    };
    let layout = if is_user {
        Layout::right_to_left(Align::TOP)
    } else {
        Layout::left_to_right(Align::TOP)
    };
    ui.add_space(4.0);
    ui.with_layout(layout, |ui| {
        if !is_user {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
            ui.painter().circle_filled(
                rect.center(),
                15.0,
                if is_error { pal.danger } else { pal.primary },
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if is_error { "!" } else { "AI" },
                FontId::proportional(12.5),
                Color32::WHITE,
            );
            ui.add_space(4.0);
        }
        let resp = ui
            .vertical(|ui| {
                ui.set_max_width(max_w);
                let who = match msg.role {
                    AiRole::User => "Bạn → AI",
                    AiRole::Assistant => "AI",
                    AiRole::Error => "AI · lỗi",
                };
                ui.label(
                    RichText::new(format!("{who} · chỉ mình bạn thấy"))
                        .color(pal.text_muted)
                        .size(11.2),
                );
                egui::Frame::none()
                    .fill(fill)
                    .stroke(Stroke::new(1.0_f32, stroke_col))
                    .rounding(Rounding::same(12.0))
                    .inner_margin(Margin::symmetric(12.0, 8.0))
                    .show(ui, |ui| {
                        if msg.role == AiRole::Assistant {
                            render_markdown_lines(ui, &msg.text, ink, pal, textures, loading, None);
                        } else {
                            ui.add(
                                egui::Label::new(RichText::new(&msg.text).color(ink).size(13.6))
                                    .wrap()
                                    .selectable(true),
                            );
                        }
                        let mut foot = hhmm(msg.ts);
                        if let Some(note) = &msg.note {
                            foot.push_str(&format!(" · {note}"));
                        }
                        ui.label(RichText::new(foot).color(meta).size(10.8));
                        if is_error {
                            ui.add_space(2.0);
                            if ui.button("Thử lại").clicked() {
                                action = Some(AiAction::Retry);
                            }
                        }
                    })
                    .response
            })
            .inner;
        if !is_error {
            let msg_id = msg.id.clone();
            let text = msg.text.clone();
            resp.context_menu(|ui| {
                if ui.button("Sao chép").clicked() {
                    action = Some(AiAction::Copy(text.clone()));
                    ui.close_menu();
                }
                if ui.button("Trả lời").clicked() {
                    action = Some(AiAction::Reply(msg_id.clone()));
                    ui.close_menu();
                }
                if !is_user && ui.button("Tạo lại câu trả lời").clicked() {
                    action = Some(AiAction::Regenerate(msg_id.clone()));
                    ui.close_menu();
                }
                ui.separator();
                if ui
                    .button(RichText::new("Xóa (chỉ trên máy này)").color(pal.danger))
                    .clicked()
                {
                    action = Some(AiAction::Delete(msg_id.clone()));
                    ui.close_menu();
                }
            });
        }
    });
    action
}

/// "AI đang trả lời…" placeholder shown while a request is in flight.
pub(super) fn render_ai_thinking(ui: &mut egui::Ui, pal: &ThemePalette) {
    let dots = match (ui.input(|i| i.time) * 2.5) as u64 % 3 {
        0 => ".",
        1 => "..",
        _ => "...",
    };
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!("{} AI đang trả lời{dots}", lucide::SPARKLES))
            .color(pal.text_muted)
            .size(12.2),
    );
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(400));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(text: &str, sender: &str, out: bool, ts: i64) -> ChatMessage {
        ChatMessage {
            id: format!("m{ts}"),
            sender: sender.into(),
            text: text.into(),
            is_outgoing: out,
            ts,
            ..Default::default()
        }
    }

    #[test]
    fn transcript_labels_senders_hides_files_and_keeps_order() {
        let thread = Thread {
            id: "t".into(),
            name: "Sơn".into(),
            is_group: false,
            messages: vec![
                msg("chào", "1234567", false, 1_700_000_000_000),
                msg("hi", "Tôi", true, 1_700_000_060_000),
                msg("", "1234567", false, 1_700_000_120_000),
            ],
            ..Default::default()
        };
        let lines = transcript_for(&thread);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].who, "Sơn"); // numeric uid in a 1-1 chat -> conversation name
        assert_eq!(lines[1].who, "Tôi");
        assert_eq!(lines[2].text, "[tệp hoặc hình ảnh]");
        let group = Thread {
            is_group: true,
            ..thread
        };
        assert_eq!(transcript_for(&group)[0].who, "Thành viên");
    }

    #[test]
    fn transcript_is_capped_keeping_the_newest() {
        let big = "x".repeat(5_000);
        let thread = Thread {
            id: "t".into(),
            name: "G".into(),
            is_group: true,
            messages: (0..60)
                .map(|i| msg(&format!("{i} {big}"), "Ann", false, 1_700_000_000_000 + i))
                .collect(),
            ..Default::default()
        };
        let lines = transcript_for(&thread);
        let total: usize = lines.iter().map(|l| l.text.chars().count()).sum();
        assert!(total <= ai::TRANSCRIPT_CAP_CHARS);
        assert!(lines.last().unwrap().text.starts_with("59 "));
        assert!(lines.len() < 60);
    }
    #[test]
    fn bubbles_render_for_every_role_in_both_themes_without_panicking() {
        let textures: HashMap<String, TextureHandle> = HashMap::new();
        let mut loading: HashSet<String> = HashSet::new();
        for pal in [ThemePalette::dark(), ThemePalette::light()] {
            let ctx = egui::Context::default();
            let mut actions = 0;
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for (role, text) in [
                        (AiRole::User, "@AI Rust là gì"),
                        (
                            AiRole::Assistant,
                            "Rust là **ngôn ngữ** hệ thống.\n\n```rust\nfn main() {}\n```",
                        ),
                        (AiRole::Error, "API key sai hoặc đã hết hạn (HTTP 401)"),
                    ] {
                        let m = AiMessage {
                            id: ai::make_id(),
                            thread_id: "t".into(),
                            role,
                            text: text.into(),
                            ts: 1_700_000_000_000,
                            reply_to_id: None,
                            model: Some("m".into()),
                            note: Some("Đã đọc 12 tin gần nhất".into()),
                        };
                        if render_ai_bubble(ui, &m, &pal, &textures, &mut loading).is_some() {
                            actions += 1;
                        }
                    }
                    render_ai_thinking(ui, &pal);
                });
            });
            assert_eq!(actions, 0, "no action without a click");
        }
    }
}
