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
    /// In-memory pre-read summaries (never persisted), by conversation id.
    pub digests: HashMap<String, ai::Digest>,
    /// Conversation being read in advance right now.
    pub preload_inflight: Option<String>,
    /// Earliest time the next pre-read may start (spreads the work out).
    pub preload_next_at: Option<std::time::Instant>,
    /// Conversations whose last pre-read failed, with the failure time (retry after 5 minutes).
    pub preload_failed: HashMap<String, std::time::Instant>,
    pub preload_error: String,
    pub preload_search: String,
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

/// Conversation messages to share, oldest first, newest 800 messages,
/// capped at `ai::TRANSCRIPT_CAP_CHARS`.
pub(super) fn transcript_for(thread: &Thread, after_ms: i64) -> Vec<TranscriptLine> {
    let mut lines: Vec<TranscriptLine> = thread
        .messages
        .iter()
        .filter(|m| !is_system_notice_message(m) && normalize_ts_ms(m.ts) > after_ms)
        .rev()
        .take(800)
        .map(|m| ai::line_from_chat(m, thread))
        .filter(|line| !line.text.is_empty() || line.attachment.is_some())
        .collect();
    lines.reverse();
    let mut total = 0usize;
    let mut keep_from = 0usize;
    for (i, l) in lines.iter().enumerate().rev() {
        total += [&l.id, &l.time, &l.who, &l.text]
            .into_iter()
            .chain(l.reply_to.iter())
            .chain(
                l.attachment
                    .iter()
                    .flat_map(|a| [&a.url, &a.title, &a.desc].into_iter().flatten()),
            )
            .map(|field| field.chars().count())
            .sum::<usize>()
            + 24;
        if total > ai::TRANSCRIPT_CAP_CHARS {
            keep_from = i + 1;
            break;
        }
    }
    lines.drain(..keep_from);
    lines
}

/// Next chosen conversation that needs a (re)read: no summary yet, or one that has fallen behind
/// by more than `DIGEST_REFRESH_AFTER_NEW` messages and is over ten minutes old.
pub(super) fn next_preload_target<'a>(
    chosen: &HashSet<String>,
    threads: &'a [Thread],
    digests: &HashMap<String, ai::Digest>,
    failed: &HashSet<String>,
    now_ms: i64,
) -> Option<&'a Thread> {
    threads.iter().find(|t| {
        if !chosen.contains(&t.id) || failed.contains(&t.id) {
            return false;
        }
        match digests.get(&t.id) {
            None => true,
            Some(d) => {
                let newer = t
                    .messages
                    .iter()
                    .filter(|m| normalize_ts_ms(m.ts) > d.covers_until_ms)
                    .count();
                newer > ai::DIGEST_REFRESH_AFTER_NEW && now_ms - d.built_at_ms > 600_000
            }
        }
    })
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "máy chủ AI".to_string())
}

/// What the user asked: the prompt, what it replies to, and composer attachments.
struct AskInput {
    prompt: String,
    /// `(id, text)` of the Assistant message being replied to.
    quoted: Option<(String, String)>,
    /// The Zalo message being replied to.
    quoted_msg: Option<TranscriptLine>,
    local: Vec<ai::AttachmentRef>,
}

/// Enter goes to the Assistant: `@AI ...`, or a reply to an Assistant message.
pub(super) fn ai_intercepts(enabled: bool, input: &str, replying_to_id: Option<&str>) -> bool {
    enabled && (ai::parse_ai_command(input).is_some() || replying_to_id.is_some_and(ai::is_ai_id))
}

const ATTACHMENT_LINE_PREFIX: &str = "\n📎 ";

/// The question as shown in its bubble: the prompt plus the names of composer attachments.
fn question_text(prompt: &str, local: &[ai::AttachmentRef]) -> String {
    if local.is_empty() {
        return prompt.to_string();
    }
    let names: Vec<String> = local.iter().map(ai::AttachmentRef::name).collect();
    format!("{prompt}{ATTACHMENT_LINE_PREFIX}{}", names.join(", "))
}

fn strip_attachment_line(text: &str) -> String {
    match text.rfind(ATTACHMENT_LINE_PREFIX) {
        Some(i) => text[..i].to_string(),
        None => text.to_string(),
    }
}

fn local_refs(refs: &[ai::AttachmentRef]) -> Vec<ai::AttachmentRef> {
    refs.iter()
        .filter(|r| matches!(r, ai::AttachmentRef::Local { .. }))
        .cloned()
        .collect()
}

/// Composer attachments always; with sharing on, also the photo/file of the Zalo message being
/// replied to and recent photos when the question is about images. No duplicates.
pub(super) fn request_attachments(
    local: Vec<ai::AttachmentRef>,
    quoted_msg: Option<&TranscriptLine>,
    transcript: &[TranscriptLine],
    prompt: &str,
    share: bool,
) -> Vec<ai::AttachmentRef> {
    let mut out = local;
    if share {
        let candidates = quoted_msg
            .and_then(ai::zalo_ref)
            .into_iter()
            .chain(ai::recent_photo_refs(transcript, prompt, share));
        for r in candidates {
            if !out.contains(&r) {
                out.push(r);
            }
        }
    }
    out
}

impl ZaloGuiApp {
    /// Context sharing for a conversation: its own choice, else the global default (off).
    pub(super) fn ai_share_enabled(&self, thread_id: &str) -> bool {
        self.settings
            .ai_share_threads
            .get(thread_id)
            .copied()
            .unwrap_or(self.ai_preload_active(thread_id) || self.settings.ai_share_default)
    }

    /// Pre-reading is on, consented to, and this conversation was chosen for it.
    fn ai_preload_active(&self, thread_id: &str) -> bool {
        self.settings.ai_enabled
            && self.settings.ai_preload_enabled
            && self.settings.ai_preload_consent
            && self.settings.ai_preload_threads.contains(thread_id)
    }

    /// True when Enter in the composer should go to the Assistant instead of Zalo. A pasted image or
    /// dropped file goes with the question to the Assistant (it used to be sent to Zalo with the
    /// `@AI ...` text as its caption).
    pub(super) fn ai_intercepts_send(&self) -> bool {
        ai_intercepts(
            self.settings.ai_enabled,
            &self.input_message,
            self.replying_to.as_ref().map(|q| q.id.as_str()),
        )
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
        // Composer attachments go to the Assistant with the question. The pasted image's temp file
        // is kept (not deleted) so Retry/Regenerate can read it again.
        let mut local = Vec::new();
        if let Some((path, _, _, _)) = self.pending_clipboard_image.take() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "ảnh dán.png".into());
            local.push(ai::AttachmentRef::Local {
                path,
                name,
                is_image: true,
            });
        }
        for file in self.pending_drop_files.drain(..) {
            local.push(ai::AttachmentRef::Local {
                name: file.name.clone(),
                is_image: file.is_image,
                path: file.path,
            });
        }
        let (quoted, quoted_msg) = match self.replying_to.take() {
            Some(q) if replying_ai => (Some((q.id, q.text)), None),
            Some(q) if self.ai_share_enabled(thread_id) => (
                None,
                self.threads
                    .iter()
                    .find(|t| t.id == thread_id)
                    .map(|thread| ai::line_from_chat(&q, thread)),
            ),
            _ => (None, None),
        };
        self.reply_auto_mention = None;
        self.ai_begin_ask(thread_id, prompt, quoted, quoted_msg, local);
    }

    fn ai_build_request(
        &self,
        thread_id: &str,
        input: AskInput,
        user_msg: Option<AiMessage>,
        user_msg_id: String,
    ) -> Option<AskRequest> {
        let AskInput {
            prompt,
            quoted,
            quoted_msg,
            local,
        } = input;
        let thread = self.threads.iter().find(|t| t.id == thread_id)?;
        let share = self.ai_share_enabled(thread_id);
        // With a fresh pre-read summary only the messages after it are sent; a summary that has
        // fallen far behind is ignored (and rebuilt in the background).
        let mut digest = None;
        let mut transcript = Vec::new();
        if share {
            if let Some(d) = self.ai.digests.get(thread_id) {
                let newer = transcript_for(thread, d.covers_until_ms);
                if newer.len() <= ai::DIGEST_REFRESH_AFTER_NEW * 2 {
                    digest = Some(d.clone());
                    transcript = newer;
                }
            }
            if digest.is_none() {
                transcript = transcript_for(thread, 0);
            }
        }
        let quoted_msg = quoted_msg.filter(|_| share);
        let attachments =
            request_attachments(local, quoted_msg.as_ref(), &transcript, &prompt, share);
        Some(AskRequest {
            thread_id: thread_id.to_string(),
            thread_name: thread.name.clone(),
            is_group: thread.is_group,
            prompt,
            user_msg,
            user_msg_id,
            reply_to_id: quoted.as_ref().map(|(id, _)| id.clone()),
            quoted: quoted.map(|(_, text)| text),
            quoted_msg,
            transcript,
            digest,
            share,
            base_url: self.settings.ai_base_url.clone(),
            model: self.settings.ai_model.clone(),
            attachments,
        })
    }

    fn ai_begin_ask(
        &mut self,
        thread_id: &str,
        prompt: String,
        quoted: Option<(String, String)>,
        quoted_msg: Option<TranscriptLine>,
        local: Vec<ai::AttachmentRef>,
    ) {
        let id = ai::make_id();
        let user_msg = AiMessage {
            id: id.clone(),
            thread_id: thread_id.to_string(),
            role: AiRole::User,
            text: question_text(&prompt, &local),
            ts: ai::now_ms(),
            reply_to_id: quoted.as_ref().map(|(qid, _)| qid.clone()),
            model: None,
            note: None,
        };
        let input = AskInput {
            prompt,
            quoted,
            quoted_msg,
            local,
        };
        if let Some(req) = self.ai_build_request(thread_id, input, Some(user_msg), id) {
            self.ai_dispatch(req);
        }
    }

    fn ai_dispatch(&mut self, req: AskRequest) {
        if req.share
            && req.is_group
            && !self.settings.ai_group_confirmed.contains(&req.thread_id)
            && !self.ai_preload_active(&req.thread_id)
        {
            self.ai.pending_confirm = Some(req);
            return;
        }
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

    pub(super) fn ai_on_digest(&mut self, thread_id: String, result: Result<ai::Digest, String>) {
        if self.ai.preload_inflight.as_deref() == Some(thread_id.as_str()) {
            self.ai.preload_inflight = None;
        }
        // Spread the work out: the next conversation starts a few seconds later.
        self.ai.preload_next_at =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
        if !self.ai_preload_active(&thread_id) {
            return; // switched off or deselected while it was running: drop the result
        }
        match result {
            Ok(d) => {
                self.ai.preload_failed.remove(&thread_id);
                self.ai.preload_error.clear();
                self.ai.digests.insert(thread_id, d);
            }
            Err(e) => {
                self.ai.preload_error = e;
                self.ai
                    .preload_failed
                    .insert(thread_id, std::time::Instant::now());
            }
        }
    }

    /// Drop every pre-read summary (switch off, deselect, logout).
    pub(super) fn ai_clear_digests(&mut self) {
        self.ai.digests.clear();
        self.ai.preload_inflight = None;
        self.ai.preload_failed.clear();
    }

    /// Start the next pre-read when enabled, consented and idle. One conversation at a time.
    fn ai_preload_tick(&mut self) {
        let active = self.settings.ai_enabled
            && self.settings.ai_preload_enabled
            && self.settings.ai_preload_consent
            && !self.settings.ai_preload_threads.is_empty();
        if !active {
            if !self.ai.digests.is_empty() || self.ai.preload_inflight.is_some() {
                self.ai_clear_digests();
            }
            return;
        }
        if !self.is_logged_in || self.ai.preload_inflight.is_some() {
            return;
        }
        let now = std::time::Instant::now();
        let start_at = *self
            .ai
            .preload_next_at
            .get_or_insert(now + std::time::Duration::from_secs(8));
        if now < start_at {
            return;
        }
        // Keep only digests of conversations that are still chosen.
        let chosen = self.settings.ai_preload_threads.clone();
        self.ai.digests.retain(|id, _| chosen.contains(id));
        let failed: HashSet<String> = self
            .ai
            .preload_failed
            .iter()
            .filter(|(_, at)| at.elapsed() < std::time::Duration::from_secs(300))
            .map(|(id, _)| id.clone())
            .collect();
        let Some(thread) = next_preload_target(
            &chosen,
            &self.threads,
            &self.ai.digests,
            &failed,
            ai::now_ms(),
        ) else {
            self.ai.preload_next_at = Some(now + std::time::Duration::from_secs(15));
            return;
        };
        let cmd = WorkerCmd::AiPreload {
            thread_id: thread.id.clone(),
            thread_name: thread.name.clone(),
            is_group: thread.is_group,
            base_url: self.settings.ai_base_url.clone(),
            model: self.settings.ai_model.clone(),
        };
        let id = thread.id.clone();
        if WORKER_TX.get().is_some_and(|tx| tx.send(cmd).is_ok()) {
            self.ai.preload_inflight = Some(id);
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
            self.ai.digests.clear();
            self.ai.preload_inflight = None;
            self.ai.preload_next_at = None;
            self.ai.preload_failed.clear();
            self.ai.account = self.user_uin.clone();
        }
        self.ai_preload_tick();
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
                // The last request for this question still knows its prompt, what it replied to
                // and its composer attachments; otherwise use the stored text alone.
                let input = match self
                    .ai
                    .last_req
                    .get(&thread_id)
                    .filter(|r| r.user_msg_id == question.id)
                {
                    Some(r) => AskInput {
                        prompt: r.prompt.clone(),
                        quoted: r
                            .quoted
                            .clone()
                            .map(|q| (r.reply_to_id.clone().unwrap_or_default(), q)),
                        quoted_msg: r.quoted_msg.clone(),
                        local: local_refs(&r.attachments),
                    },
                    None => AskInput {
                        prompt: strip_attachment_line(&question.text),
                        quoted: None,
                        quoted_msg: None,
                        local: Vec::new(),
                    },
                };
                if let Some(req) = self.ai_build_request(&thread_id, input, None, question.id) {
                    self.ai_dispatch(req);
                }
            }
            AiAction::Retry => {
                if let Some(req) = self.ai.last_req.get(&thread_id).cloned() {
                    // Rebuild the transcript so a retry sees the latest messages.
                    let input = AskInput {
                        prompt: req.prompt,
                        quoted: req.quoted.map(|q| (req.reply_to_id.unwrap_or_default(), q)),
                        quoted_msg: req.quoted_msg,
                        local: local_refs(&req.attachments),
                    };
                    let Some(req) = self.ai_build_request(&thread_id, input, None, req.user_msg_id)
                    else {
                        return;
                    };
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
                        "Để trả lời, AI cần đọc tin nhắn gần đây của nhóm \"{}\". Văn bản, liên kết, ảnh và tệp trong tin nhắn của tất cả thành viên có thể được gửi tới {host}.",
                        req.thread_name
                    ))
                    .color(pal.text_primary)
                    .size(13.2),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Lựa chọn này được nhớ cho nhóm này và có thể tắt bất cứ lúc nào ở khung soạn tin.")
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
                req.quoted_msg = None;
                req.digest = None;
                // Only what the user attached in the composer; nothing from the conversation.
                req.attachments = local_refs(&req.attachments);
                self.ai_dispatch(req);
            }
            Some(_) => {
                // Cancelled: give the question and its attachments back to the composer.
                self.ai.pending_confirm = None;
                if req.user_msg.is_some() {
                    if self.input_message.trim().is_empty() {
                        self.input_message = format!("@AI {}", req.prompt);
                    }
                    for r in local_refs(&req.attachments) {
                        if let ai::AttachmentRef::Local { path, .. } = r {
                            self.queue_pending_composer_file(ctx, path);
                        }
                    }
                }
            }
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
                            format!("Văn bản, liên kết, ảnh và tệp trong tin nhắn gần đây có thể được gửi tới {host}. Bấm để tắt."),
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
                        RichText::new("Mặc định cho mọi cuộc trò chuyện. Khi bật, AI đọc các tin gần đây để trả lời và tóm tắt tốt hơn. Có thể đổi riêng từng cuộc ngay ở khung soạn tin khi gõ @AI.")
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
                RichText::new(format!("Nếu bật, văn bản, liên kết, ảnh và tệp trong tin nhắn có thể được gửi tới {}. Nhóm chat luôn hỏi xác nhận lần đầu.", host_of(&self.settings.ai_base_url)))
                    .color(pal.text_muted)
                    .size(11.6),
            );
        });

        ui.add_space(14.0);
        ui.label(
            RichText::new("Đọc trước ngữ cảnh (tăng tốc)")
                .color(pal.text_primary)
                .size(13.5)
                .strong(),
        );
        ui.add_space(6.0);
        let host = host_of(&self.settings.ai_base_url);
        card(ui, &mut |ui| {
            ui.label(
                RichText::new(format!("Mặc định tắt. Khi mở ứng dụng, AI đọc trước các cuộc trò chuyện bạn chọn để trả lời nhanh hơn. Văn bản, liên kết, ảnh và tệp trong các tin nhắn đó có thể được gửi tới {host}. Bản tóm tắt chỉ được giữ trong bộ nhớ đến khi tắt ứng dụng."))
                    .color(pal.text_secondary)
                    .size(11.8),
            );
            ui.add_space(8.0);
            let mut consent = self.settings.ai_preload_consent;
            if ui
                .checkbox(
                    &mut consent,
                    RichText::new(format!("Tôi đồng ý cho AI ({host}) đọc nội dung các cuộc trò chuyện đã chọn bên dưới")).size(12.4),
                )
                .changed()
            {
                self.settings.ai_preload_consent = consent;
                if !consent {
                    self.settings.ai_preload_enabled = false;
                    self.ai_clear_digests();
                }
                save_settings = true;
            }
            ui.add_space(6.0);
            let can_enable = self.settings.ai_preload_consent
                && self.settings.ai_enabled
                && !self.settings.ai_preload_threads.is_empty();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Bật đọc trước ngữ cảnh")
                        .color(pal.text_primary)
                        .size(13.2),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled_ui(can_enable || self.settings.ai_preload_enabled, |ui| {
                        if render_toggle_switch(ui, &mut self.settings.ai_preload_enabled, pal)
                            .changed()
                        {
                            if self.settings.ai_preload_enabled {
                                self.ai.preload_next_at = None;
                                self.ai.preload_failed.clear();
                            } else {
                                self.ai_clear_digests();
                            }
                            save_settings = true;
                        }
                    });
                });
            });
            if !can_enable && !self.settings.ai_preload_enabled {
                let why = if !self.settings.ai_enabled {
                    "Hãy bật Trợ lý AI ở trên."
                } else if !self.settings.ai_preload_consent {
                    "Cần đồng ý cho AI đọc cuộc trò chuyện trước."
                } else {
                    "Chọn ít nhất một cuộc trò chuyện bên dưới."
                };
                ui.label(RichText::new(why).color(pal.text_muted).size(11.6));
            }
            ui.add_space(8.0);
            label(ui, "Chọn cuộc trò chuyện được đọc trước");
            ui.add(
                egui::TextEdit::singleline(&mut self.ai.preload_search)
                    .hint_text("Tìm theo tên…")
                    .desired_width(f32::INFINITY),
            );
            let needle = self.ai.preload_search.trim().to_lowercase();
            egui::ScrollArea::vertical()
                .id_salt("ai_preload_threads")
                .max_height(210.0)
                .show(ui, |ui| {
                    for t in self.threads.iter().filter(|t| {
                        !t.name.trim().is_empty()
                            && (needle.is_empty() || t.name.to_lowercase().contains(&needle))
                    }) {
                        let mut on = self.settings.ai_preload_threads.contains(&t.id);
                        let text = if t.is_group {
                            format!("{}  · nhóm", t.name)
                        } else {
                            t.name.clone()
                        };
                        if ui
                            .checkbox(&mut on, RichText::new(text).size(12.4))
                            .changed()
                        {
                            if on {
                                self.settings.ai_preload_threads.insert(t.id.clone());
                            } else {
                                self.settings.ai_preload_threads.remove(&t.id);
                                self.ai.digests.remove(&t.id);
                                if self.settings.ai_preload_threads.is_empty() {
                                    self.settings.ai_preload_enabled = false;
                                }
                            }
                            save_settings = true;
                        }
                    }
                });
            ui.add_space(6.0);
            let chosen = self.settings.ai_preload_threads.len();
            let ready = self.ai.digests.len();
            let mut status = format!("Đã chọn {chosen} cuộc trò chuyện");
            if self.settings.ai_preload_enabled {
                status.push_str(&format!(" · đã đọc trước {ready}/{chosen}"));
                if let Some(id) = &self.ai.preload_inflight {
                    let name = self
                        .threads
                        .iter()
                        .find(|t| &t.id == id)
                        .map_or("…", |t| t.name.as_str());
                    status.push_str(&format!(" · đang đọc \"{name}\"…"));
                }
            }
            ui.label(RichText::new(status).color(pal.text_primary).size(12.2));
            if !self.ai.preload_error.is_empty() {
                ui.label(
                    RichText::new(format!("⚠ {}", self.ai.preload_error))
                        .color(pal.danger)
                        .size(11.8),
                );
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if self.settings.ai_preload_enabled && ui.button("Đọc trước ngay").clicked() {
                    self.ai.preload_next_at = None;
                    self.ai.preload_failed.clear();
                    self.ai.preload_error.clear();
                }
                if !self.ai.digests.is_empty() && ui.button("Xóa dữ liệu đọc trước").clicked()
                {
                    self.ai_clear_digests();
                }
            });
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
        // The right-click target must be registered before the text: egui gives a click to the
        // widget registered last, so a menu target added after the bubble's labels (as
        // `Response::context_menu` does) swallowed clicks on links in the answer. Last frame's
        // rect is used, so the menu is ready from the second frame on.
        let menu_id = egui::Id::new(("ai_bubble_menu", &msg.id));
        let menu_target = ui
            .ctx()
            .data(|d| d.get_temp::<egui::Rect>(menu_id))
            .map(|rect| ui.interact(rect, menu_id, Sense::click()));
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
        ui.ctx().data_mut(|d| d.insert_temp(menu_id, resp.rect));
        if let (false, Some(target)) = (is_error, menu_target) {
            let msg_id = msg.id.clone();
            let text = msg.text.clone();
            target.context_menu(|ui| {
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
    fn old_assistant_consent_requires_confirmation_again() {
        let old: AppSettings = serde_json::from_value(serde_json::json!({
            "ai_group_confirmed": ["group-1"],
            "ai_preload_consent": true,
            "ai_preload_enabled": true,
            "ai_preload_threads": ["group-1"],
        }))
        .unwrap();
        assert!(old.ai_group_confirmed.is_empty());
        assert!(!old.ai_preload_consent);
        assert!(old.ai_preload_enabled && old.ai_preload_threads.contains("group-1"));

        let new: AppSettings = serde_json::from_value(serde_json::json!({
            "ai_group_confirmed_v2": ["group-1"],
            "ai_preload_consent_v2": true,
        }))
        .unwrap();
        assert!(new.ai_group_confirmed.contains("group-1"));
        assert!(new.ai_preload_consent);
    }

    #[test]
    fn transcript_labels_senders_includes_files_and_keeps_order() {
        let thread = Thread {
            id: "t".into(),
            name: "Sơn".into(),
            is_group: false,
            messages: vec![
                msg("chào", "1234567", false, 1_700_000_000_000),
                msg("hi", "Tôi", true, 1_700_000_060_000),
                ChatMessage {
                    link_src: Some("Tập tin Zalo".into()),
                    link_title: Some("agenda.pdf".into()),
                    link_url: Some("https://files.example/agenda.pdf".into()),
                    ..msg("", "1234567", false, 1_700_000_120_000)
                },
            ],
            ..Default::default()
        };
        let lines = transcript_for(&thread, 0);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].who, "Sơn"); // numeric uid in a 1-1 chat -> conversation name
        assert_eq!(lines[1].who, "Tôi");
        let (transcript, _) = ai::build_transcript(&lines, ai::CONTEXT_BUDGET_CHARS);
        assert!(transcript.contains("[Tệp: agenda.pdf — https://files.example/agenda.pdf]"));
        let group = Thread {
            is_group: true,
            ..thread
        };
        assert_eq!(transcript_for(&group, 0)[0].who, "Thành viên");
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
        let lines = transcript_for(&thread, 0);
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
    #[test]
    fn preload_picks_unread_chosen_threads_and_refreshes_only_stale_digests() {
        let thread = |id: &str, last_ts: i64, n: usize| Thread {
            id: id.into(),
            name: id.into(),
            messages: (0..n as i64)
                .map(|i| msg("x", "A", false, last_ts + i))
                .collect(),
            ..Default::default()
        };
        let threads = vec![
            thread("a", 1_700_000_000_000, 3),
            thread("b", 1_700_000_000_000, 3),
            thread("c", 1_700_000_000_000, 3),
        ];
        let chosen: HashSet<String> = ["b".to_string(), "c".to_string()].into();
        let mut digests: HashMap<String, ai::Digest> = HashMap::new();
        let none = HashSet::new();
        // Unchosen "a" is never read; the first chosen one without a summary comes first.
        assert_eq!(
            next_preload_target(&chosen, &threads, &digests, &none, 10)
                .unwrap()
                .id,
            "b"
        );
        let d = |covers: i64, built: i64| ai::Digest {
            text: "t".into(),
            covers_until_ms: covers,
            message_count: 3,
            built_at_ms: built,
        };
        digests.insert("b".into(), d(1_700_000_000_010, 0));
        assert_eq!(
            next_preload_target(&chosen, &threads, &digests, &none, 10)
                .unwrap()
                .id,
            "c"
        );
        // A failed thread is skipped (retry later); with everything summarised there is nothing to do.
        let failed: HashSet<String> = ["c".to_string()].into();
        assert!(next_preload_target(&chosen, &threads, &digests, &failed, 10).is_none());
        digests.insert("c".into(), d(1_700_000_000_010, 0));
        assert!(next_preload_target(&chosen, &threads, &digests, &none, 10).is_none());
        // Many new messages AND an old digest -> rebuild; a recent digest is kept.
        let busy = vec![thread("b", 1_700_000_001_000, 80)];
        digests.insert("b".into(), d(1_700_000_000_500, 0));
        assert_eq!(
            next_preload_target(&chosen, &busy, &digests, &none, 700_000)
                .unwrap()
                .id,
            "b"
        );
        assert!(next_preload_target(&chosen, &busy, &digests, &none, 60_000).is_none());
    }

    #[test]
    fn transcript_after_a_cutoff_returns_only_newer_messages() {
        let thread = Thread {
            id: "t".into(),
            name: "Sơn".into(),
            messages: vec![
                msg("cũ", "Sơn", false, 1_700_000_000_000),
                msg("mới", "Sơn", false, 1_700_000_100_000),
            ],
            ..Default::default()
        };
        assert_eq!(transcript_for(&thread, 1_700_000_050_000).len(), 1);
        assert_eq!(transcript_for(&thread, 1_700_000_050_000)[0].text, "mới");
        assert_eq!(transcript_for(&thread, 0).len(), 2);
    }

    #[test]
    fn at_ai_with_pending_attachments_goes_to_the_assistant() {
        // Pending images/files no longer decide this: "@AI …" always goes to the Assistant.
        assert!(ai_intercepts(true, "@AI ảnh này là gì", None));
        assert!(ai_intercepts(true, "trả lời tiếp", Some("ai_123")));
        assert!(!ai_intercepts(true, "ảnh đẹp", None));
        assert!(!ai_intercepts(true, "ảnh đẹp", Some("123456")));
        assert!(!ai_intercepts(false, "@AI ảnh này là gì", None));
    }

    #[test]
    fn question_text_lists_attachments_and_regenerate_strips_them() {
        let local = vec![ai::AttachmentRef::Local {
            path: "/tmp/a.png".into(),
            name: "a.png".into(),
            is_image: true,
        }];
        let text = question_text("mô tả", &local);
        assert_eq!(text, "mô tả\n📎 a.png");
        assert_eq!(strip_attachment_line(&text), "mô tả");
        assert_eq!(question_text("hỏi", &[]), "hỏi");
    }

    #[test]
    fn request_attachments_follow_sharing() {
        let photo = TranscriptLine {
            id: "p1".into(),
            attachment: Some(ai::Attachment {
                kind: ai::AttachmentKind::Photo,
                url: Some("https://f1.zdn.vn/p1.jpg".into()),
                title: None,
                desc: None,
            }),
            ..Default::default()
        };
        let local = vec![ai::AttachmentRef::Local {
            path: "/tmp/a.txt".into(),
            name: "a.txt".into(),
            is_image: false,
        }];
        // Sharing off: only what the user attached.
        let off = request_attachments(
            local.clone(),
            Some(&photo),
            std::slice::from_ref(&photo),
            "ảnh gì",
            false,
        );
        assert_eq!(off, local);
        // Sharing on: the replied photo once, even though it is also a recent photo.
        let on = request_attachments(
            local.clone(),
            Some(&photo),
            std::slice::from_ref(&photo),
            "ảnh gì",
            true,
        );
        assert_eq!(on.len(), 2);
        assert!(matches!(&on[1], ai::AttachmentRef::Zalo { message_id, .. } if message_id == "p1"));
    }
}
