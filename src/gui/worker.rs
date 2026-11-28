use super::*;
use std::future::{poll_fn, Future};
use std::sync::Arc;
use std::task::Poll;
use tokio::sync::{watch, Mutex};

/// Thread (id, name, is_group) list the poll timer maps incoming messages onto; the GUI
/// republishes it whenever its threads change.
pub(super) static KNOWN_THREADS: std::sync::Mutex<Vec<(String, String, bool)>> =
    std::sync::Mutex::new(Vec::new());

/// GIFs fetched per `@gif` query (the web client allows up to 50).
const GIF_RESULT_LIMIT: u32 = 30;

fn persist_app_settings(path: &std::path::Path, json: &str) -> std::io::Result<()> {
    std::fs::write(path, json)
}

async fn run_poll_command(
    session: &Session,
    account: &str,
    poll_id: &str,
    option_ids: Option<&[String]>,
) -> Result<PollDetail, String> {
    if account.is_empty() || session.login.as_ref().map(|login| login.uin.as_str()) != Some(account) {
        return Err(POLL_ACCOUNT_CHANGED.into());
    }

    let vote = option_ids.map(|option_ids| crate::chat::vote_poll(session, poll_id, option_ids));
    poll_command_detail(vote, crate::chat::get_poll_detail(session, poll_id)).await
}

async fn poll_command_detail(
    vote: Option<impl Future<Output = anyhow::Result<Option<PollDetail>>>>,
    fetch_detail: impl Future<Output = anyhow::Result<PollDetail>>,
) -> Result<PollDetail, String> {
    let fallback = if let Some(vote) = vote {
        let details = vote
            .await
            .map_err(|error| poll_command_error(error, "Chưa xác nhận được lượt bình chọn."))?;
        if let Some(details) = details {
            return Ok(details);
        }
        "Đã gửi bình chọn, nhưng không thể tải lại kết quả."
    } else {
        "Không thể tải bình chọn."
    };
    fetch_detail
        .await
        .map_err(|error| poll_command_error(error, fallback))
}

const POLL_ACCOUNT_CHANGED: &str = "Tài khoản đã thay đổi.";

fn advance_poll_generation(generation: &watch::Sender<u64>) {
    generation.send_modify(|value| *value = value.wrapping_add(1));
}

struct PollCommand {
    account: String,
    poll_id: String,
    option_ids: Option<Vec<String>>,
    generation: u64,
}

/// Serialize poll requests and take credentials only when the command reaches the front.
fn spawn_poll_command(
    session: &Arc<Mutex<Session>>,
    queue: &Arc<Mutex<()>>,
    generation: &watch::Sender<u64>,
    gui_tx: &GuiTx,
    command: PollCommand,
) {
    let mut generation_rx = generation.subscribe();
    let received_generation = *generation_rx.borrow_and_update();
    let session = session.clone();
    let queue = queue.clone();
    let gui_tx = gui_tx.clone();
    tokio::spawn(async move {
        let PollCommand {
            account,
            poll_id,
            option_ids,
            generation,
        } = command;
        let _turn = queue.lock().await;
        let snapshot = {
            let current = session.lock().await;
            (generation == received_generation && generation == *generation_rx.borrow())
                .then(|| current.clone())
        };
        let result = if let Some(snapshot) = snapshot {
            let operation = run_poll_command(&snapshot, &account, &poll_id, option_ids.as_deref());
            run_in_poll_generation(&mut generation_rx, generation, operation)
                .await
                .map(PollDetail::bounded)
        } else {
            Err(POLL_ACCOUNT_CHANGED.into())
        };
        // Hold the generation read guard through the event send. The GUI checks it again when
        // consuming the event in case the account changed while the event was queued.
        let current_generation = generation_rx.borrow();
        let result = if generation == *current_generation {
            result
        } else {
            Err(POLL_ACCOUNT_CHANGED.into())
        };
        let event = if option_ids.is_some() {
            GuiEvent::PollVoted {
                account,
                poll_id,
                generation,
                result,
            }
        } else {
            GuiEvent::PollLoaded {
                account,
                poll_id,
                generation,
                result,
            }
        };
        let _ = gui_tx.tx.send(event);
        drop(current_generation);
        gui_tx.ctx.request_repaint();
    });
}

async fn run_in_poll_generation(
    generation_rx: &mut watch::Receiver<u64>,
    generation: u64,
    operation: impl Future<Output = Result<PollDetail, String>>,
) -> Result<PollDetail, String> {
    let active_generation = generation_rx.clone();
    tokio::pin!(operation);
    let current_operation = poll_fn(|cx| {
        // Serialize each poll of the request with generation changes without holding a
        // guard while the request is suspended.
        let current = active_generation.borrow();
        if generation == *current {
            operation.as_mut().poll(cx)
        } else {
            Poll::Ready(Err(POLL_ACCOUNT_CHANGED.into()))
        }
    });
    tokio::select! {
        biased;
        _ = generation_rx.changed() => Err(POLL_ACCOUNT_CHANGED.into()),
        result = current_operation => result,
    }
}

fn poll_command_error(error: anyhow::Error, fallback: &str) -> String {
    if let Some(unavailable) = error.downcast_ref::<crate::chat::PollUnavailable>() {
        unavailable.to_string()
    } else if error.downcast_ref::<crate::media_resources::MediaError>()
        == Some(&crate::media_resources::MediaError::TooLarge)
    {
        crate::session::GROUP_POLL_BODY_TOO_LARGE.to_owned()
    } else {
        fallback.to_owned()
    }
}

async fn run_storage_task<T: Send + 'static>(
    guard: Result<tokio::sync::OwnedMutexGuard<()>, tokio::sync::TryLockError>,
    operation: impl FnOnce(&std::path::Path) -> T + Send + 'static,
) -> Result<T, String> {
    let _guard = guard.map_err(|_| "Đang quét hoặc dọn bộ nhớ.".to_string())?;
    tokio::task::spawn_blocking(move || {
        let root = std::env::current_dir()
            .map_err(|_| "Không thể mở thư mục dữ liệu.".to_string())?;
        Ok(operation(&root))
    })
    .await
    .map_err(|_| "Tác vụ bộ nhớ bị gián đoạn.".to_string())?
}

/// Absolute cache directory for notification avatars; the native helper rejects relative paths.
fn notification_avatar_dir() -> Option<std::path::PathBuf> {
    Some(
        std::env::current_dir()
            .ok()?
            .join("media_cache/notification_avatar"),
    )
}

async fn cache_notification_avatar(
    session: &crate::session::Session,
    url: &str,
) -> Option<std::path::PathBuf> {
    const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;
    if !crate::session::is_trusted_zalo_url(url) {
        return None;
    }
    let dir = notification_avatar_dir()?;
    crate::data_dir::ensure_private_dir(&dir).ok()?;
    let path = dir.join(format!("{}.png", crate::crypto::md5_hex(url)));
    if path.is_file() {
        return Some(path);
    }
    let response = session.http.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = crate::media_resources::read_bounded_response(response, MAX_AVATAR_BYTES)
        .await
        .ok()?;
    let path_for_decode = path.clone();
    tokio::task::spawn_blocking(move || {
        let format = image::guess_format(&bytes).ok()?;
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes));
        reader.set_format(format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(32 * 1024 * 1024);
        reader.limits(limits);
        let decoded = reader.decode().ok()?.thumbnail(256, 256);
        let mut encoded = std::io::Cursor::new(Vec::new());
        decoded
            .write_to(&mut encoded, image::ImageFormat::Png)
            .ok()?;
        crate::data_dir::write_private_file(&path_for_decode, encoded.get_ref()).ok()?;
        Some(path_for_decode)
    })
    .await
    .ok()
    .flatten()
}

pub enum WorkerCmd {
    AppLockTryPasscode {
        config: app_lock::Config,
        candidate: String,
        now_unix: i64,
    },
    AppLockSetPasscode {
        config: app_lock::Config,
        passcode: String,
    },
    AppLockSave {
        config: app_lock::Config,
    },
    StartVideo {
        account: String,
        thread: String,
        message: String,
        generation: u64,
        url: String,
        start_at_seconds: f32,
        volume: f32,
    },
    StopVideo {
        generation: u64,
    },
    SetVideoVolume {
        generation: u64,
        volume: f32,
    },
    StartVoicePlayback {
        account: String,
        thread: String,
        message: String,
        generation: u64,
        url: String,
    },
    StopVoicePlayback {
        message: String,
    },
    VoiceStart {
        account: String,
        generation: u64,
        thread: String,
        is_group: bool,
    },
    VoiceStop {
        generation: u64,
        cancel: bool,
    },
    VoiceEncoded {
        account: String,
        generation: u64,
        thread: String,
        is_group: bool,
        result: Result<crate::voice::VoiceRecord, String>,
    },
    FetchDocPreview {
        account: String,
        generation: u64,
        url: String,
        name: String,
    },
    FetchPoll {
        account: String,
        generation: u64,
        thread_id: String,
        poll_id: String,
    },
    VotePoll {
        account: String,
        generation: u64,
        thread_id: String,
        poll_id: String,
        option_ids: Vec<String>,
    },
    Productivity {
        account: String,
        action: crate::productivity::Action,
    },
    /// Download or install the video tools after a user action.
    InstallVideoTools,
    /// Re-scan FFmpeg and media devices off the UI thread.
    RefreshVideoTools,
    /// Persist a settings snapshot without filesystem work on the UI thread.
    SaveAppSettings {
        json: String,
    },
    /// `@AI` question: persist, ask the Assistant and report back. Never sends anything to Zalo.
    AiAsk {
        req: crate::ai::AskRequest,
    },
    /// Load the local Assistant dialogue of one conversation.
    AiLoadThread {
        thread_id: String,
    },
    /// Read one chosen conversation in advance and summarise it (kept in memory by the GUI).
    AiPreload {
        thread_id: String,
        thread_name: String,
        is_group: bool,
        base_url: String,
        model: String,
    },
    AiDelete {
        msg_id: String,
    },
    AiTestConnection {
        base_url: String,
        model: String,
    },
    AiListModels {
        base_url: String,
        model: String,
    },
    /// Save (or, when empty, remove) the Assistant API key in the private key file.
    AiSaveKey {
        key: String,
    },
    /// Read OS authorization without prompting.
    ReadNotificationAuthorization {
        request_id: u64,
    },
    /// Request OS authorization (Settings click, or once at macOS startup).
    RequestNotificationAuthorization {
        request_id: u64,
    },
    /// Post a native OS notification for an incoming message.
    PostNativeNotification {
        title: String,
        body: String,
        avatar_url: Option<String>,
    },
    RequestQr,
    LoginPassword {
        phone: String,
        pass: String,
    },
    RefreshThreads,
    RefreshThreadsManual,
    PollMessages {
        known_threads: Vec<(String, String, bool)>,
    },
    LoadGroupHistory {
        group_id: String,
        is_group: bool,
        /// Messages the GUI already holds. SQLite is read `have + 300` deep, so each
        /// "Tải thêm" reaches further back instead of re-reading the same newest 500.
        have: usize,
    },
    /// Page group / Cloud history back from the server into SQLite: (id, name, is_group).
    SyncHistory {
        threads: Vec<(String, String, bool)>,
    },
    SendMessage {
        to_id: String,
        text: String,
        is_group: bool,
        /// Members tagged from the composer (group threads only; ignored for 1-1).
        mentions: Vec<crate::chat::MentionPick>,
    },
    SendQuote {
        to_id: String,
        text: String,
        quoted: ChatMessage,
        is_group: bool,
        /// Members tagged in the reply (group threads only; ignored for 1-1).
        mentions: Vec<crate::chat::MentionPick>,
    },
    Forward {
        target_id: String,
        target_is_group: bool,
        msg: ChatMessage,
        target_name: String,
        /// Optional text sent right after the forwarded message (share dialog note).
        note: Option<String>,
    },
    React {
        to_id: String,
        is_group: bool,
        msg: ChatMessage,
        reaction: String,
    },
    DeleteForMe {
        to_id: String,
        is_group: bool,
        msg: ChatMessage,
    },
    RecallMessage {
        to_id: String,
        is_group: bool,
        msg: ChatMessage,
    },
    SendSticker {
        to_id: String,
        sticker_id: u64,
        cate_id: u64,
        sticker_type: u32,
        is_group: bool,
    },
    /// Read the account's conversation labels ("Phân loại").
    FetchConvLabels,
    /// GIF search for the composer `@gif` command; an empty query means trending.
    SearchGifs {
        query: String,
        seq: u64,
    },
    SendGif {
        to_id: String,
        is_group: bool,
        item: crate::chat::GifItem,
        src: i32,
    },
    PickAndSendAttachment {
        to_id: String,
        is_group: bool,
        images_only: bool,
        caption: String,
        preset_path: Option<std::path::PathBuf>,
        original_hd: bool,
    },
    /// Pick, validate, and privately copy an image for the quick-message editor.
    PickQuickMessageImage {
        request_id: u64,
    },
    /// Remove an unreferenced image from the managed quick-message directory.
    DeleteQuickMessageImage {
        path: String,
    },
    ScanStorage,
    ClearStorage {
        target: ClearTarget,
        expected_account: String,
        /// The user's download folder: never cleared even when it sits in the data directory.
        protected: Vec<std::path::PathBuf>,
    },
    TriggerScreenshot,
    SendTyping {
        thread_id: String,
        is_group: bool,
    },
    FetchStickerImage {
        key: String,
        url: String,
    },
    /// Rasterise a colour emoji cluster from the system emoji font. Terminal events are
    /// `StickerImageReady` / `StickerImageFailed` with the same `key`.
    RasterizeEmoji {
        key: String,
        cluster: String,
    },
    /// Crop one tile from the shared Zalo emoticon sprite.
    FetchZaloEmojiTile {
        col: u8,
        row: u8,
    },
    FetchStickerStore,
    FetchStickerPack {
        cate_id: u64,
        name: String,
    },
    SearchStickers {
        keyword: String,
    },
    SuggestComposerStickers {
        keyword: String,
    },
    ParseLinkPreview {
        url: String,
    },
    LookupPhone {
        phone: String,
    },
    StartCall {
        partner_id: String,
        partner_name: String,
        partner_avatar: Option<String>,
        is_video: bool,
        is_group: bool,
    },
    AnswerCall {
        accept: bool,
        cam_start_off: bool,
    },
    SetCallMediaState {
        is_on_mic: bool,
        is_on_cam: bool,
    },
    InviteGroupCallMembers {
        user_ids: Vec<String>,
    },
    EndCall,
    AddFriend {
        user_id: String,
        user_name: String,
    },
    CheckFriendStatus {
        user_id: String,
    },
    AcceptFriend {
        user_id: String,
        user_name: String,
    },
    RejectFriend {
        user_id: String,
        user_name: String,
    },
    UndoFriend {
        user_id: String,
        user_name: String,
    },
    PersistThreads(Vec<Thread>),
    CheckForUpdates {
        silent: bool,
    },
    DownloadAndApplyUpdate {
        download_url: String,
        asset_name: String,
        target_version: String,
        expected_sha256: Option<String>,
    },
    Logout,
    /// Delete local message data (`zalo_history.db` + `threads_cache.json`) without
    /// touching credentials. Separate from [`WorkerCmd::Logout`] by design (#21).
    WipeLocalData,
    PinThread {
        thread_id: String,
        is_group: bool,
        pin: bool,
    },
    /// Reload pinned board items for one conversation (server is the source of truth).
    RefreshBoardPins {
        thread_id: String,
        is_group: bool,
    },
    /// Pin/unpin a single chat message through the group-board API (#119 follow-up).
    PinMessage {
        thread_id: String,
        is_group: bool,
        msg: crate::chat::ChatMessage,
        pin: bool,
        /// Board topic id, required to unpin a previously pinned message.
        topic_id: String,
    },
    MarkThreadUnread {
        thread_id: String,
        is_group: bool,
        unread: bool,
    },
    MuteThread {
        thread_id: String,
        is_group: bool,
        duration_sec: i64,
        mute: bool,
    },
    HideThread {
        thread_id: String,
        is_group: bool,
        hide: bool,
    },
    DeleteThread {
        thread_id: String,
        is_group: bool,
        last_cli_msg_id: String,
    },
    FetchGroupMembers {
        group_id: String,
    },
    RefreshMessageQueues {
        threads: Vec<Thread>,
    },
    SyncPhoneMessages {
        threads: Vec<Thread>,
        platform: crate::history_sync::PhonePlatform,
    },
    SetProxy {
        proxy_url: String,
    },
    DownloadFile {
        url: String,
        file_name: String,
        download_dir: String,
    },
    SendSeen {
        thread_id: String,
        is_group: bool,
        last_msg_id: String,
    },
    SwitchAccount {
        target_uin: String,
    },
}

pub(super) static WORKER_TX: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<WorkerCmd>> =
    std::sync::OnceLock::new();

pub(super) fn extract_msg_id(resp: &serde_json::Value) -> String {
    resp.get("data")
        .and_then(|d| d.get("msgId"))
        .and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

pub(super) fn resp_error_code(resp: &serde_json::Value) -> Option<i64> {
    resp.get("error_code")
        .and_then(serde_json::Value::as_i64)
        .filter(|c| *c != 0)
}

fn prepare_avatar_texture(image: image::DynamicImage) -> image::RgbaImage {
    let rgba = image.to_rgba8();
    let target_side = rgba.width().max(rgba.height()).clamp(32, 160);
    let longest_side = rgba.width().max(rgba.height()).max(1);
    let scale = target_side as f64 / longest_side as f64;
    let fitted_width = ((rgba.width() as f64 * scale).round() as u32).max(1);
    let fitted_height = ((rgba.height() as f64 * scale).round() as u32).max(1);
    let fitted = image::imageops::resize(
        &rgba,
        fitted_width,
        fitted_height,
        image::imageops::FilterType::Triangle,
    );
    let mut canvas = image::RgbaImage::new(target_side, target_side);
    let x = (target_side - fitted.width()) / 2;
    let y = (target_side - fitted.height()) / 2;
    image::imageops::overlay(&mut canvas, &fitted, i64::from(x), i64::from(y));
    canvas
}

/// Immediately decode any disk-cached sticker thumbnails (`stickers_cache/stk_{cid}_{sid}.png`)
/// and push `GuiEvent::StickerImageReady` *before* sending sticker list metadata, returning any
/// missing `(key, url, cache_path)` entries for concurrent network prefetching.
pub(super) fn preload_disk_sticker_thumbnails(
    items: &[crate::chat::StickerItem],
    gui_tx: &GuiTx,
    limit: usize,
) -> Vec<(String, String, std::path::PathBuf)> {
    let cache_dir = std::path::Path::new("stickers_cache");
    let _ = std::fs::create_dir_all(cache_dir);

    let mut missing = Vec::new();
    for item in items.iter().take(limit) {
        let key = format!("stk_{}_{}", item.cate_id, item.sticker_id);
        let cache_path = cache_dir.join(format!("{key}.png"));
        if let Ok(disk_bytes) = std::fs::read(&cache_path) {
            if let Ok(dyn_img) = image::load_from_memory(&disk_bytes) {
                let rgba = dyn_img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let pixels = rgba.as_flat_samples();
                let color_img = egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_slice());
                let _ = gui_tx.send(GuiEvent::StickerImageReady {
                    key,
                    image: color_img,
                });
                continue;
            } else {
                let _ = std::fs::remove_file(&cache_path);
            }
        }
        let url = if item.cate_id == 0 && (1..=30).contains(&item.sticker_id) {
            format!(
                "https://stc-chat.zdn.vn/images/stickers/default/thumb/{}.png",
                item.sticker_id
            )
        } else {
            format!(
                "https://api.zaloapp.com/api/emoticon/sticker/webpc?eid={}&size=130",
                item.sticker_id
            )
        };
        missing.push((key, url, cache_path));
    }
    missing
}

/// Concurrently download, cache, and decode missing sticker thumbnails so the popover and search
/// grid render smoothly without waiting for frame-by-frame lazy requests.
async fn prefetch_missing_sticker_thumbnails(
    http: &reqwest::Client,
    missing: Vec<(String, String, std::path::PathBuf)>,
    gui_tx: &GuiTx,
    egui_ctx: &egui::Context,
) {
    if missing.is_empty() {
        return;
    }
    let futs = missing.into_iter().map(|(key, url, cache_path)| {
        let client = http.clone();
        let tx = gui_tx.clone();
        let ctx = egui_ctx.clone();
        async move {
            if let Ok(resp) = client.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(b) = resp.bytes().await {
                        let vec = b.to_vec();
                        if let Ok(dyn_img) = image::load_from_memory(&vec) {
                            let _ = std::fs::write(&cache_path, &vec);
                            let rgba = dyn_img.to_rgba8();
                            let size = [rgba.width() as usize, rgba.height() as usize];
                            let pixels = rgba.as_flat_samples();
                            let color_img =
                                egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_slice());
                            let _ = tx.send(GuiEvent::StickerImageReady {
                                key,
                                image: color_img,
                            });
                            ctx.request_repaint();
                        }
                    }
                }
            }
        }
    });
    futures_util::future::join_all(futs).await;
}

async fn handle_sync_v2_batch(
    s: &Session,
    tx: &GuiTx,
    ctx: &egui::Context,
    url: &str,
    sync_id: &str,
    batch_type: u8,
    is_last: bool,
) {
    let result = async {
        if !matches!(batch_type, 1 | 2) {
            anyhow::bail!("Unsupported Sync V2 batch type");
        }
        if batch_type != 2 {
            // A data event may be scheduled before its metadata task has finished.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            while !crate::history_sync::has_sync_v2_session(sync_id) {
                if crate::history_sync::v2_transfer(sync_id).is_none()
                    || tokio::time::Instant::now() >= deadline
                {
                    anyhow::bail!("Chưa nhận metadata của phiên Sync V2.");
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        let _decode_guard = crate::history_sync::V2_BATCH_LOCK.lock().await;
        if crate::history_sync::v2_transfer(sync_id).is_none() {
            anyhow::bail!("Phiên Sync V2 đã kết thúc.");
        }
        if batch_type == 2 {
            let meta = crate::history_sync::import_sync_v2_meta(s, url).await?;
            crate::history_sync::record_sync_v2_session(sync_id, &meta.session_id);
            Ok(0)
        } else {
            crate::history_sync::import_sync_v2_messages(s, url).await
        }
    }
    .await;
    crate::history_sync::finish_v2_batch(sync_id, result, batch_type != 2 && is_last);
    if let Some(state) = crate::history_sync::v2_transfer(sync_id) {
        let note = if let Some(error) = state.failure {
            error
        } else if state.saw_last && state.pending == 0 {
            format!("Đã tải và lưu {} tin nhắn từ điện thoại.", state.imported)
        } else {
            format!(
                "Đang tải lịch sử điện thoại: đã lưu {} tin nhắn…",
                state.imported
            )
        };
        crate::history_sync::set_last_mobile_sync_status(Some(note.clone()));
        let _ = tx.send(GuiEvent::StatusUpdate(note));
        ctx.request_repaint();
    }
}

fn use_session_for_sticker_image(fetch_url: &str) -> bool {
    crate::session::is_trusted_zalo_url(fetch_url)
}

/// Full-resolution originals opened in the image viewer (`hd_` keys) can be much larger than
/// grid thumbnails, so they get their own, still bounded, body limit.
const ORIGINAL_IMAGE_LIMIT: usize = 64 * 1024 * 1024;

/// Longest side of an `hd_` texture. Well under the 8192–16384 px texture limit of the GPUs we
/// run on, and still sharper than any screen the viewer fills.
const HD_TEXTURE_MAX: u32 = 4096;

fn sticker_image_body_limit(key: &str) -> usize {
    if key.starts_with("hd_") {
        ORIGINAL_IMAGE_LIMIT
    } else {
        crate::media_resources::PREVIEW_LIMIT
    }
}

pub(super) fn spawn_session_worker(
    session: Session,
    gui_tx: GuiTx,
    egui_ctx: egui::Context,
    poll_generation: watch::Sender<u64>,
) {
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<WorkerCmd>();
    let _ = WORKER_TX.set(cmd_tx);

    let session = std::sync::Arc::new(tokio::sync::Mutex::new(session));
    let poll_state =
        std::sync::Arc::new(tokio::sync::Mutex::new(crate::chat::PollState::default()));
    let ctrl_poll_state = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::call::CtrlPollState::default(),
    ));

    std::thread::Builder::new()
        .name("zalo-worker".into())
        .spawn(move || {
            // Lightweight 2-thread async runtime to keep memory & CPU footprint minimal
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();

            rt.block_on(async move {
                if let Some(tx) = WORKER_TX.get() {
                    let _ = tx.send(WorkerCmd::RefreshVideoTools);
                }
                {
                    let s = session.lock().await.clone();
                    let _ = crate::login::get_server_info(&s).await;
                }
                // Message poll timer. It used to live in the GUI's update(), which forced a full
                // repaint every 1.2 s just to fire it. The PollMessages handler skips while a
                // long-poll is in flight and when logged out.
                tokio::spawn(async {
                    let mut tick = tokio::time::interval(Duration::from_millis(1200));
                    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    loop {
                        tick.tick().await;
                        let known_threads =
                            KNOWN_THREADS.lock().map(|k| k.clone()).unwrap_or_default();
                        if let Some(tx) = WORKER_TX.get() {
                            let _ = tx.send(WorkerCmd::PollMessages { known_threads });
                        }
                    }
                });

                // Scheduled messages worker (#38)
                {
                    let s_sched = session.clone();
                    let gui_tx_sched = gui_tx.clone();
                    let ctx_sched = egui_ctx.clone();
                    tokio::spawn(async move {
                        let mut tick = tokio::time::interval(Duration::from_secs(2));
                        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                        loop {
                            tick.tick().await;
                            let now_ts = crate::login::unix_now() as i64;
                            let due = crate::scheduler::claim_due_scheduled_messages(
                                "zalo_history.db",
                                now_ts,
                            )
                            .unwrap_or_default();
                            if due.is_empty() {
                                continue;
                            }
                            let s = s_sched.lock().await.clone();
                            if s.login.is_none() {
                                for item in due {
                                    let _ = crate::scheduler::mark_scheduled_failed(
                                        "zalo_history.db",
                                        item.id,
                                        "Chưa đăng nhập phiên Zalo",
                                    );
                                }
                                let _ = gui_tx_sched.send(GuiEvent::ScheduledListUpdated);
                                ctx_sched.request_repaint();
                                continue;
                            }
                            for item in due {
                                let client_id = crate::chat::next_client_message_id();
                                let send_res = if item.is_group {
                                    crate::chat::send_group_message_with_client_id(&s, &item.thread_id, &item.text, client_id)
                                        .await
                                } else {
                                    crate::chat::send_message_with_client_id(&s, &item.thread_id, &item.text, client_id).await
                                };
                                match send_res {
                                    Ok(resp) => {
                                        if let Some(code) = resp_error_code(&resp) {
                                            let _ = crate::scheduler::mark_scheduled_failed(
                                                "zalo_history.db",
                                                item.id,
                                                &format!("Mã lỗi Zalo {code}"),
                                            );
                                        } else {
                                            let msg_id = extract_msg_id(&resp);
                                            let _ = crate::scheduler::mark_scheduled_sent(
                                                "zalo_history.db",
                                                item.id,
                                            );
                                            let _ = gui_tx_sched.send(GuiEvent::MessageSent {
                                                client_message_id: client_id.to_string(),
                                                thread_id: item.thread_id.clone(),
                                                text: item.text.clone(),
                                                msg_id,
                                                mentions: Vec::new(),
                                            });
                                        }
                                    }
                                    Err(e) => {
                                        let _ = crate::scheduler::mark_scheduled_failed(
                                            "zalo_history.db",
                                            item.id,
                                            &e.to_string(),
                                        );
                                    }
                                }
                            }
                            let _ = gui_tx_sched.send(GuiEvent::ScheduledListUpdated);
                            ctx_sched.request_repaint();
                        }
                    });
                }
                {
                    let s_kw = session.clone();
                    let gui_tx_kw = gui_tx.clone();
                    let ctx_kw = egui_ctx.clone();
                    tokio::spawn(async move {
                        let s = s_kw.lock().await.clone();
                        if s.login.is_some() {
                            if let Some(tx) = WORKER_TX.get() {
                                let _ = tx.send(WorkerCmd::RefreshThreads);
                            }
                            if let Ok(kws) = crate::chat::fetch_sticker_suggest_keywords(&s).await {
                                if !kws.is_empty() {
                                    let _ = gui_tx_kw
                                        .send(GuiEvent::StickerSuggestKeywordsLoaded(kws));
                                    ctx_kw.request_repaint();
                                }
                            }
                        }
                    });
                }

                let call_bridge = std::sync::Arc::new(crate::call::CallBridge::spawn(
                    session.clone(),
                    gui_tx.clone(),
                    egui_ctx.clone(),
                ));

                // 1) Real-time `zpw_ws` WebSocket loop for instant chat messages + VoIP events
                {
                    let s_ws = session.clone();
                    let cb = call_bridge.clone();
                    let gui_tx_ws = gui_tx.clone();
                    let ctx_ws = egui_ctx.clone();
                    tokio::spawn(async move {
                        let mut ws_rx: Option<
                            tokio::sync::mpsc::UnboundedReceiver<crate::socket::WsEvent>,
                        > = None;
                        let mut ws_sek = String::new();
                        loop {
                            let s_snap = s_ws.lock().await.clone();
                            if s_snap.login.is_some() {
                                let cur_sek = s_snap.zpw_sek.clone().unwrap_or_default();
                                if !cur_sek.is_empty() && (ws_rx.is_none() || ws_sek != cur_sek) {
                                    ws_sek = cur_sek;
                                    ws_rx = Some(crate::socket::spawn(s_snap.clone()));
                                }
                                if let Some(rx) = ws_rx.as_mut() {
                                    match tokio::time::timeout(
                                        Duration::from_millis(1500),
                                        rx.recv(),
                                    )
                                    .await
                                    {
                                        Ok(Some(ev)) => {
                                            crate::reactions::trace_ws_payload(
                                                ev.cmd,
                                                ev.sub_cmd,
                                                &ev.payload,
                                            );
                                            let data_node = ev
                                                .payload
                                                .get("data")
                                                .unwrap_or(&ev.payload);
                                            for voip_ev in
                                                crate::call::extract_voip_controls(data_node)
                                            {
                                                cb.handle_control_event(voip_ev);
                                            }
                                            crate::chat::dispatch_file_done_controls(data_node);
                                            let mute_evs = crate::chat::extract_mute_controls(
                                                data_node,
                                                crate::login::unix_now() as i64,
                                            );
                                            if !mute_evs.is_empty() {
                                                let _ = gui_tx_ws
                                                    .send(GuiEvent::MuteEventsReceived(mute_evs));
                                                ctx_ws.request_repaint();
                                            }
                                            let (typing_evs, seen_evs) =
                                                crate::chat::extract_typing_and_seen_controls(data_node);
                                            if !typing_evs.is_empty() {
                                                let _ = gui_tx_ws.send(GuiEvent::TypingEventsReceived(typing_evs));
                                                ctx_ws.request_repaint();
                                            }
                                            if !seen_evs.is_empty() {
                                                let _ = gui_tx_ws.send(GuiEvent::SeenEventsReceived(seen_evs));
                                                ctx_ws.request_repaint();
                                            }
                                            let react_evs =
                                                crate::reactions::parse_reaction_events(data_node);
                                            if !react_evs.is_empty() {
                                                let _ = gui_tx_ws
                                                    .send(GuiEvent::ReactionEventsReceived(react_evs));
                                                ctx_ws.request_repaint();
                                            }

                                            for sync_ev in
                                                crate::chat::extract_mobile_sync_controls(data_node)
                                            {
                                                crate::history_sync::record_android_control(&s_snap, &sync_ev);
                                                match sync_ev {
                                                    crate::chat::MobileSyncControlEvent::TransferAfterLogin {
                                                        temp_key,
                                                    } => {
                                                        crate::history_sync::set_latest_mobile_temp_key(&temp_key);
                                                        let known = crate::chat::load_threads_cache();
                                                        if let Some(wtx) = WORKER_TX.get() {
                                                            let _ = wtx.send(
                                                                WorkerCmd::RefreshMessageQueues { threads: known },
                                                            );
                                                        }
                                                    }
                                                    crate::chat::MobileSyncControlEvent::WaitingConfirmOnPhone { .. } => {
                                                        let note = "📲 Đang chờ bạn bấm 'Đồng bộ' trên màn hình Zalo điện thoại…".to_string();
                                                        crate::history_sync::set_last_mobile_sync_status(Some(note.clone()));
                                                        let _ = gui_tx_ws.send(GuiEvent::StatusUpdate(note));
                                                        ctx_ws.request_repaint();
                                                    }
                                                    crate::chat::MobileSyncControlEvent::MobileBusyRestoring => {
                                                        let note = "⚠️ Điện thoại đang bận sao lưu/khôi phục phiên trước — hãy mở Zalo trên điện thoại xác nhận hoặc thử lại sau ít phút".to_string();
                                                        crate::history_sync::set_last_mobile_sync_status(Some(note.clone()));
                                                        let _ = gui_tx_ws.send(GuiEvent::StatusUpdate(note));
                                                        ctx_ws.request_repaint();
                                                    }
                                                    crate::chat::MobileSyncControlEvent::UserConfirmed => {
                                                        let note = "📲 Điện thoại đã xác nhận đồng bộ — đang nén và đóng gói tin nhắn…".to_string();
                                                        crate::history_sync::set_last_mobile_sync_status(Some(note.clone()));
                                                        let _ = gui_tx_ws.send(GuiEvent::StatusUpdate(note));
                                                        ctx_ws.request_repaint();
                                                    }
                                                    crate::chat::MobileSyncControlEvent::UserRejected => {
                                                        let note = "⚠️ Bạn đã từ chối yêu cầu đồng bộ tin nhắn trên điện thoại".to_string();
                                                        crate::history_sync::set_last_mobile_sync_status(Some(note.clone()));
                                                        let _ = gui_tx_ws.send(GuiEvent::StatusUpdate(note));
                                                        ctx_ws.request_repaint();
                                                    }
                                                    crate::chat::MobileSyncControlEvent::BackupReady { url, encrypted_key, .. } => {
    let snapshot = s_snap.clone();
    tokio::spawn(async move {
        crate::history_sync::receive_android_backup(&snapshot, &url, &encrypted_key).await;
    });
}
                                                    crate::chat::MobileSyncControlEvent::SyncV2Batch { url, sync_id, batch_type, is_last } => {
    if crate::history_sync::enqueue_v2_batch(&sync_id) {
        let s_sync = s_snap.clone();
        let tx_sync = gui_tx_ws.clone();
        let ctx_sync = ctx_ws.clone();
        tokio::spawn(async move {
            handle_sync_v2_batch(&s_sync, &tx_sync, &ctx_sync, &url, &sync_id, batch_type, is_last).await;
        });
    }
}
                                                }
                                            }
                                            if ev.is_chat_ish() {
                                                let mut ws_threads = Vec::new();
                                                if let Some(li) = s_snap.login.as_ref() {
                                                    if !li.send2me_id.is_empty() {
                                                        ws_threads.push(Thread {
                                                            presentation: Default::default(),
                                                            unread: Default::default(),
                                                            id: li.send2me_id.clone(),
                                                            name: "Cloud của tôi (Lưu trữ)"
                                                                .into(),
                                                            preview: String::new(),
                                                            time: String::new(),
                                                            avatar: None,
                                                            is_group: false,
                                                            messages: Vec::new(),
                                                        });
                                                    }
                                                }
                                                if let Ok(known) = KNOWN_THREADS.lock() {
                                                    for (id, name, is_group) in known.iter() {
                                                        if !ws_threads.iter().any(|t| &t.id == id) {
                                                            ws_threads.push(Thread {
                                                                presentation: Default::default(),
                                                                unread: Default::default(),
                                                                id: id.clone(),
                                                                name: name.clone(),
                                                                preview: String::new(),
                                                                time: String::new(),
                                                                avatar: None,
                                                                is_group: *is_group,
                                                                messages: Vec::new(),
                                                            });
                                                        }
                                                    }
                                                }
                                                crate::chat::merge_previews(
                                                    &mut ws_threads,
                                                    &ev.payload,
                                                );
                                                ws_threads.retain(|t| !t.messages.is_empty());
                                                if !ws_threads.is_empty() {
                                                    // Deliver incoming messages to UI immediately (0ms network wait)
                                                    let _ = gui_tx_ws.send(
                                                        GuiEvent::ThreadsUpdated(ws_threads.clone()),
                                                    );
                                                    ctx_ws.request_repaint();
                                                    let needs_resolve = ws_threads.iter().any(|t| {
                                                        t.name.is_empty()
                                                            || t.name.starts_with("Người dùng ")
                                                            || crate::chat::is_generic_thread_name(&t.name)
                                                    });
                                                    if needs_resolve {
                                                        let s_bg = s_snap.clone();
                                                        let gui_tx_bg = gui_tx_ws.clone();
                                                        let ctx_bg = ctx_ws.clone();
                                                        tokio::spawn(async move {
                                                            let mut resolved = ws_threads;
                                                            crate::chat::resolve_thread_names(
                                                                &s_bg,
                                                                &mut resolved,
                                                            )
                                                            .await;
                                                            let _ = gui_tx_bg.send(
                                                                GuiEvent::ThreadsUpdated(resolved),
                                                            );
                                                            ctx_bg.request_repaint();
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                        Ok(None) => {
                                            ws_rx = None;
                                            tokio::time::sleep(Duration::from_secs(2)).await;
                                        }
                                        Err(_) => {}
                                    }
                                } else {
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                }
                            } else {
                                ws_rx = None;
                                ws_sek.clear();
                                tokio::time::sleep(Duration::from_secs(3)).await;
                            }
                        }
                    });
                }

                // 2) Background HTTP long-poll fallback for VoIP & mute control events (`lp_control`)
                {
                    let s_ctrl = session.clone();
                    let cps = ctrl_poll_state.clone();
                    let cb = call_bridge.clone();
                    let gui_tx_ctrl = gui_tx.clone();
                    let ctx_ctrl = egui_ctx.clone();
                    tokio::spawn(async move {
                        loop {
                            let s_snap = s_ctrl.lock().await.clone();
                            if s_snap.login.is_some() {
                                let ctrl_res = {
                                    let mut st = cps.lock().await;
                                    crate::call::poll_control_events(&s_snap, &mut st).await
                                };
                                if let Ok((events, mute_evs, typing_evs, seen_evs)) = ctrl_res {
                                    for ev in events {
                                        cb.handle_control_event(ev);
                                    }
                                    if !mute_evs.is_empty() {
                                        let _ = gui_tx_ctrl
                                            .send(GuiEvent::MuteEventsReceived(mute_evs));
                                        ctx_ctrl.request_repaint();
                                    }
                                    if !typing_evs.is_empty() {
                                        let _ = gui_tx_ctrl
                                            .send(GuiEvent::TypingEventsReceived(typing_evs));
                                        ctx_ctrl.request_repaint();
                                    }
                                    if !seen_evs.is_empty() {
                                        let _ = gui_tx_ctrl
                                            .send(GuiEvent::SeenEventsReceived(seen_evs));
                                        ctx_ctrl.request_repaint();
                                    }
                                } else {
                                    tokio::time::sleep(Duration::from_secs(5)).await;
                                }
                            } else {
                                tokio::time::sleep(Duration::from_secs(3)).await;
                            }
                        }
                    });
                }

                // Periodic GitHub Releases update check (every 30 minutes). The first tick is
                // skipped because ZaloGuiApp::new already asks for a check at startup.
                {
                    let gui_tx_upd = gui_tx.clone();
                    let ctx_upd = egui_ctx.clone();
                    tokio::spawn(async move {
                        let mut ticker = tokio::time::interval(Duration::from_secs(30 * 60));
                        ticker.tick().await;
                        loop {
                            ticker.tick().await;
                            let res = fetch_github_latest_release().await;
                            let _ = gui_tx_upd.send(GuiEvent::UpdateCheckFinished {
                                result: res,
                                silent: true,
                            });
                            ctx_upd.request_repaint();
                        }
                    });
                }

                let mut current_qr_task: Option<tokio::task::JoinHandle<()>> = None;

                let mut voice_control:Option<(u64,std::sync::mpsc::Sender<bool>,std::sync::Arc<std::sync::atomic::AtomicBool>)>=None;
                let video_requests = crate::media_resources::RequestGeneration::default();
                let voice_playback_requests = crate::media_resources::RequestGeneration::default();
                let storage_operation = std::sync::Arc::new(tokio::sync::Mutex::new(()));
                // Poll commands run in order (an earlier fetch must not overwrite a vote's result)
                // without holding up the command loop; tokio's Mutex grants the lock FIFO.
                let poll_queue = std::sync::Arc::new(tokio::sync::Mutex::new(()));
                let zalo_emoji_sprite = std::sync::Arc::new(tokio::sync::OnceCell::new());
                while let Some(cmd) = cmd_rx.recv().await {
                    match cmd {
                        // Hashing takes a noticeable time (PBKDF2): run off the command loop.
                        WorkerCmd::AppLockTryPasscode { mut config, candidate, now_unix } => {
                            let gui_tx = gui_tx.clone();
                            tokio::spawn(async move {
                            let result = tokio::task::spawn_blocking(move || {
                                let attempt = config.try_passcode(&candidate, now_unix);
                                let save_error = app_lock::save(std::path::Path::new("."), &config)
                                    .err()
                                    .map(|_| "Không lưu được cấu hình khóa ứng dụng.".into());
                                GuiEvent::AppLockAttempt { config, attempt, save_error }
                            })
                            .await;
                            let _ = gui_tx.send(result.unwrap_or(GuiEvent::AppLockAttemptFailed));
                            });
                        }
                        WorkerCmd::AppLockSetPasscode { mut config, passcode } => {
                            let gui_tx = gui_tx.clone();
                            tokio::spawn(async move {
                            let result = tokio::task::spawn_blocking(move || -> Result<_, &'static str> {
                                config.set_passcode(&passcode)
                                    .map_err(|_| "Không đặt được mã khóa.")?;
                                config.enabled = true;
                                app_lock::save(std::path::Path::new("."), &config)
                                    .map_err(|_| "Không lưu được cấu hình khóa ứng dụng.")?;
                                Ok(config)
                            })
                            .await;
                            let (config, error) = match result {
                                Ok(Ok(config)) => (Some(config), None),
                                Ok(Err(error)) => (None, Some(error.into())),
                                Err(_) => (None, Some("Không đặt được mã khóa.".into())),
                            };
                            let _ = gui_tx.send(GuiEvent::AppLockSaved { config, error });
                            });
                        }
                        WorkerCmd::AppLockSave { config } => {
                            let result = tokio::task::spawn_blocking(move || {
                                app_lock::save(std::path::Path::new("."), &config)
                            })
                            .await;
                            let error = (!matches!(result, Ok(Ok(()))))
                                .then(|| "Không lưu được cấu hình khóa ứng dụng.".into());
                            let _ = gui_tx.send(GuiEvent::AppLockSaved { config: None, error });
                        }
                        WorkerCmd::StartVideo {
                            account,
                            thread,
                            message,
                            generation,
                            url,
                            start_at_seconds,
                            volume,
                        } => {
                            video_requests.cancel();
                            crate::video_player::stop_all();
                            let ticket = video_requests.capture();
                            let requests = video_requests.clone();
                            let session = session.clone();
                            let tx = gui_tx.clone();
                            let ctx = egui_ctx.clone();
                            tokio::spawn(async move {
                                let result = async {
                                    if !crate::session::is_trusted_zalo_media_url(&url) {
                                        return Err("Liên kết video không thuộc máy chủ Zalo tin cậy.".to_string());
                                    }
                                    if !crate::video_tools::available() {
                                        return Err("Chưa có FFmpeg. Mở Cài đặt để cài công cụ video.".to_string());
                                    }
                                    let cache = crate::media_resources::PrivateMediaCache::new(
                                        std::path::Path::new("media_cache/video"),
                                        &account,
                                    )
                                    .map_err(|_| "Không tạo được bộ nhớ đệm video riêng tư.".to_string())?;
                                    let key = crate::media_resources::MediaKey {
                                        account,
                                        thread,
                                        message,
                                        quality: crate::media_resources::MediaQuality::Original,
                                    };
                                    let path = cache
                                        .path_for(&key)
                                        .map_err(|_| "Không tạo được đường dẫn cache video.".to_string())?;
                                    if path.is_file() {
                                        let _ = cache.touch(&key);
                                    } else {
                                        let client = if crate::session::is_trusted_zalo_url(&url) {
                                            session.lock().await.clone().http
                                        } else {
                                            crate::media_resources::playback_client()
                                                .map_err(|_| "Không chuẩn bị được kết nối tải video.".to_string())?
                                        };
                                        let progress_tx = tx.clone();
                                        crate::media_resources::stream_original_with_progress(
                                            &client,
                                            &url,
                                            &path,
                                            &requests,
                                            ticket,
                                            move |downloaded, total| {
                                                let _ = progress_tx.send(GuiEvent::VideoLoadProgress {
                                                    generation,
                                                    downloaded,
                                                    total,
                                                });
                                            },
                                        )
                                        .await
                                        .map_err(|error| match error {
                                            crate::media_resources::MediaError::TooLarge => {
                                                "Video vượt giới hạn cache 512 MB.".to_string()
                                            }
                                            crate::media_resources::MediaError::Cancelled => {
                                                "Đã dừng tải video.".to_string()
                                            }
                                            other => {
                                                // Host only: the query carries a signed token.
                                                let host = reqwest::Url::parse(&url)
                                                    .ok()
                                                    .and_then(|u| u.host_str().map(str::to_string))
                                                    .unwrap_or_default();
                                                crate::logger::log_error(
                                                    "video download",
                                                    &format!("{other:?} host={host}"),
                                                );
                                                match other {
                                                    crate::media_resources::MediaError::Http(code) => {
                                                        format!("Không tải được video từ Zalo (HTTP {code}).")
                                                    }
                                                    crate::media_resources::MediaError::Timeout => {
                                                        "Hết thời gian tải video từ Zalo.".to_string()
                                                    }
                                                    _ => "Không tải được video từ Zalo.".to_string(),
                                                }
                                            }
                                        })?;
                                        let _ = cache.prune();
                                    }
                                    requests
                                        .check(ticket)
                                        .map_err(|_| "Đã dừng tải video.".to_string())?;
                                    crate::video_player::start(
                                        path,
                                        generation,
                                        start_at_seconds,
                                        volume,
                                        tx.clone(),
                                        ctx.clone(),
                                    );
                                    Ok::<(), String>(())
                                }
                                .await;
                                if let Err(error) = result {
                                    if requests.is_current(ticket) {
                                        let _ = tx.send(GuiEvent::VideoFailed { generation, error });
                                        ctx.request_repaint();
                                    }
                                }
                            });
                        }
                        WorkerCmd::StopVideo { generation } => {
                            video_requests.cancel();
                            crate::video_player::stop(generation);
                        }
                        WorkerCmd::SetVideoVolume { generation, volume } => {
                            crate::video_player::set_volume(generation, volume);
                        }
                        WorkerCmd::StartVoicePlayback {
                            account,
                            thread,
                            message,
                            generation,
                            url,
                        } => {
                            voice_playback_requests.cancel();
                            let ticket = voice_playback_requests.capture();
                            let requests = voice_playback_requests.clone();
                            let session = session.clone();
                            let tx = gui_tx.clone();
                            let ctx = egui_ctx.clone();
                            tokio::spawn(async move {
                                let result = async {
                                    if !crate::session::is_trusted_zalo_media_url(&url) {
                                        return Err("Liên kết ghi âm không thuộc máy chủ Zalo tin cậy.".to_string());
                                    }
                                    if !crate::video_tools::available() {
                                        return Err("Chưa có FFmpeg. Mở Cài đặt để cài công cụ media.".to_string());
                                    }
                                    let cache = crate::media_resources::PrivateMediaCache::new(
                                        std::path::Path::new("media_cache/voice"),
                                        &account,
                                    )
                                    .map_err(|_| "Không tạo được bộ nhớ đệm ghi âm riêng tư.".to_string())?;
                                    let key = crate::media_resources::MediaKey {
                                        account,
                                        thread,
                                        message: message.clone(),
                                        quality: crate::media_resources::MediaQuality::Original,
                                    };
                                    let path = cache
                                        .path_for(&key)
                                        .map_err(|_| "Không tạo được đường dẫn cache ghi âm.".to_string())?;
                                    if path.is_file() {
                                        let _ = cache.touch(&key);
                                    } else {
                                        let client = if crate::session::is_trusted_zalo_url(&url) {
                                            session.lock().await.clone().http
                                        } else {
                                            crate::media_resources::playback_client()
                                                .map_err(|_| "Không chuẩn bị được kết nối tải ghi âm.".to_string())?
                                        };
                                        crate::media_resources::stream_original(
                                            &client,
                                            &url,
                                            &path,
                                            &requests,
                                            ticket,
                                        )
                                        .await
                                        .map_err(|error| match error {
                                            crate::media_resources::MediaError::TooLarge => {
                                                "Ghi âm vượt giới hạn cache.".to_string()
                                            }
                                            crate::media_resources::MediaError::Cancelled => {
                                                "Đã dừng tải ghi âm.".to_string()
                                            }
                                            _ => "Không tải được ghi âm từ Zalo.".to_string(),
                                        })?;
                                        let _ = cache.prune();
                                    }
                                    requests
                                        .check(ticket)
                                        .map_err(|_| "Đã dừng tải ghi âm.".to_string())?;
                                    crate::video_player::start_audio(
                                        path,
                                        generation,
                                        message.clone(),
                                        tx.clone(),
                                        ctx.clone(),
                                    );
                                    Ok::<(), String>(())
                                }
                                .await;
                                if let Err(error) = result {
                                    if requests.is_current(ticket) {
                                        let _ = tx.send(GuiEvent::VoicePlaybackFinished {
                                            message,
                                            error: Some(error),
                                        });
                                        ctx.request_repaint();
                                    }
                                }
                            });
                        }
                        WorkerCmd::StopVoicePlayback { message } => {
                            voice_playback_requests.cancel();
                            crate::video_player::stop_audio(&message);
                            egui_ctx.request_repaint();
                        }
                        WorkerCmd::InstallVideoTools => {
                            use std::sync::atomic::Ordering;
                            if crate::video_tools::INSTALLING.swap(true, Ordering::SeqCst) { continue; }
                            crate::video_tools::CANCEL_INSTALL.store(false, Ordering::Relaxed);
                            let tx = gui_tx.clone();
                            tokio::spawn(async move {
                                let result = async {
                                    let data_dir = std::env::current_dir()?;
                                    let progress_tx = tx.clone();
                                    crate::video_tools::install(&data_dir, move |progress, status| {
                                        let _ = progress_tx.send(GuiEvent::VideoToolsProgress { progress, status });
                                    }).await?;
                                    tokio::task::spawn_blocking(|| crate::media::list_media_devices(true)).await?;
                                    Ok::<(), anyhow::Error>(())
                                }.await.map_err(|error| error.to_string());
                                crate::video_tools::INSTALLING.store(false, Ordering::SeqCst);
                                let _ = tx.send(GuiEvent::VideoToolsFinished { result });
                            });
                        }
                        WorkerCmd::RefreshVideoTools => {
                            let tx = gui_tx.clone();
                            tokio::spawn(async move {
                                let available = match std::env::current_dir() {
                                    Ok(dir) => crate::video_tools::refresh(&dir).await,
                                    Err(_) => false,
                                };
                                let _ = tokio::task::spawn_blocking(|| crate::media::list_media_devices(true)).await;
                                let _ = tx.send(GuiEvent::VideoToolsChecked { available });
                            });
                        }
                        WorkerCmd::AiAsk { req } => {
                            let tx = gui_tx.clone();
                            let session = session.clone();
                            tokio::spawn(async move {
                                // Only for downloading attachments from Zalo media CDNs; never sent to
                                // the AI host. Same cookie jar as the session, but redirects are off so a
                                // CDN URL cannot bounce the request (and its cookies) to a Zalo API host.
                                let jar = session.lock().await.jar.clone();
                                let zalo_http = ai_media_client(jar);
                                run_ai_ask(req, tx, "zalo_history.db".to_string(), zalo_http).await
                            });
                        }
                        WorkerCmd::AiLoadThread { thread_id } => {
                            let tx = gui_tx.clone();
                            tokio::spawn(async move {
                                let id = thread_id.clone();
                                let rows = tokio::task::spawn_blocking(move || {
                                    let store = crate::store::MessageStore::open("zalo_history.db")?;
                                    Ok::<_, anyhow::Error>(crate::ai::load_thread(&store.conn, &id, 300)?)
                                })
                                .await;
                                let messages = match rows {
                                    Ok(Ok(rows)) => rows,
                                    _ => Vec::new(),
                                };
                                let _ = tx.send(GuiEvent::AiThreadLoaded { thread_id, messages });
                            });
                        }
                        WorkerCmd::AiPreload { thread_id, thread_name, is_group, base_url, model } => {
                            let tx = gui_tx.clone();
                            tokio::spawn(async move {
                                let result = run_ai_preload(&thread_id, &thread_name, is_group, base_url, model, "zalo_history.db".to_string()).await;
                                let _ = tx.send(GuiEvent::AiDigest { thread_id, result });
                            });
                        }
                        WorkerCmd::AiDelete { msg_id } => {
                            tokio::spawn(async move {
                                let _ = tokio::task::spawn_blocking(move || {
                                    let store = crate::store::MessageStore::open("zalo_history.db")?;
                                    Ok::<_, anyhow::Error>(crate::ai::delete_message(&store.conn, &msg_id)?)
                                })
                                .await;
                            });
                        }
                        WorkerCmd::AiTestConnection { base_url, model } => {
                            let tx = gui_tx.clone();
                            tokio::spawn(async move {
                                let result = match crate::ai::AiConfig::resolve(&base_url, &model) {
                                    Err(e) => Err(e.to_string()),
                                    Ok(cfg) => {
                                        let turns = [crate::ai::Turn::new("user", "Trả lời đúng một từ: OK")];
                                        crate::ai::chat_completion(&cfg, &turns, 16)
                                            .await
                                            .map(|reply| format!("Kết nối thành công · {} · \"{}\"", cfg.model, reply.chars().take(40).collect::<String>()))
                                            .map_err(|e| e.to_string())
                                    }
                                };
                                let _ = tx.send(GuiEvent::AiTestResult(result));
                            });
                        }
                        WorkerCmd::AiListModels { base_url, model } => {
                            let tx = gui_tx.clone();
                            tokio::spawn(async move {
                                let result = match crate::ai::AiConfig::resolve(&base_url, if model.trim().is_empty() { "model" } else { &model }) {
                                    Err(e) => Err(e.to_string()),
                                    Ok(cfg) => crate::ai::list_models(&cfg).await.map_err(|e| e.to_string()),
                                };
                                let _ = tx.send(GuiEvent::AiModels(result));
                            });
                        }
                        WorkerCmd::AiSaveKey { key } => {
                            let tx = gui_tx.clone();
                            tokio::spawn(async move {
                                let saved = tokio::task::spawn_blocking(move || crate::ai::save_api_key(&key)).await;
                                let note = match saved {
                                    Ok(Ok(())) => "Đã lưu API key (tệp riêng tư, không nằm trong cài đặt).".to_string(),
                                    _ => "Không lưu được API key.".to_string(),
                                };
                                let _ = tx.send(GuiEvent::StatusUpdate(note));
                            });
                        }
                        WorkerCmd::SaveAppSettings { json } => {
                            let saved = tokio::task::spawn_blocking(move || {
                                persist_app_settings(std::path::Path::new("app_settings.json"), &json)
                            }).await;
                            if !matches!(saved, Ok(Ok(()))) {
                                let _ = gui_tx.send(GuiEvent::StatusUpdate(
                                    "Không thể lưu lựa chọn giao diện vào cài đặt.".into(),
                                ));
                            }
                        }
                        WorkerCmd::ReadNotificationAuthorization { request_id }
                        | WorkerCmd::RequestNotificationAuthorization { request_id } => {
                            let request = matches!(
                                cmd, WorkerCmd::RequestNotificationAuthorization { .. }
                            );
                            let tx = gui_tx.clone();
                            tokio::spawn(async move {
                                let result = if request {
                                    crate::native_notifications::request_if_user_action(true).await
                                } else {
                                    crate::native_notifications::status().await
                                };
                                // Closing the UI receiver is normal during shutdown.
                                let _ = tx.send(GuiEvent::NotificationAuthorization {
                                    request_id, result,
                                });
                            });
                        }
                        WorkerCmd::PostNativeNotification {
                            title,
                            body,
                            avatar_url,
                        } => {
                            let snapshot = session.lock().await.clone();
                            tokio::spawn(async move {
                                let attachment = match avatar_url {
                                    Some(url) => cache_notification_avatar(&snapshot, &url).await,
                                    None => None,
                                };
                                // Delivery is best-effort; a failure must never disturb chat flow.
                                if let Err(e) = crate::native_notifications::notify_with_attachment(
                                    &title,
                                    &body,
                                    attachment.as_deref(),
                                )
                                .await
                                {
                                    crate::logger::log_error(
                                        "native_notification",
                                        &format!("not delivered: {e:?}"),
                                    );
                                }
                            });
                        }
                        WorkerCmd::PersistThreads(threads_snapshot) => {
                            // Keep snapshots ordered with Saved mutations and account-file switches.
                            let result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                                let mut store = crate::store::MessageStore::open("zalo_history.db")?;
                                crate::store::persist_threads(&mut store, &threads_snapshot)?;
                                crate::chat::save_threads_cache(&threads_snapshot);
                                Ok(())
                            }).await;
                            if !matches!(result, Ok(Ok(()))) {
                                let _ = gui_tx.send(GuiEvent::StatusUpdate("Không thể lưu lịch sử trên máy.".into()));
                            }
                        }
                        WorkerCmd::Logout => {
                            advance_poll_generation(&poll_generation);
                            if let Some((_,control,cancelled))=voice_control.take(){cancelled.store(true,std::sync::atomic::Ordering::Relaxed);let _=control.send(true);}
                            crate::history_sync::PHONE_SYNC_CANCEL.store(true, std::sync::atomic::Ordering::Relaxed);
                            crate::history_sync::REFRESH_CANCEL.store(true, std::sync::atomic::Ordering::Relaxed);
                            let _phone_sync = crate::history_sync::PHONE_SYNC_LOCK.lock().await;
                            let _decoding = crate::history_sync::V2_BATCH_LOCK.lock().await;
                            let cur_uin = {
                                let s = session.lock().await;
                                s.login.as_ref().map(|l| l.uin.clone()).unwrap_or_default()
                            };
                            if !cur_uin.is_empty() {
                                let _ = crate::account_manager::save_current_account_snapshot(&cur_uin);
                            }
                            if let Some(task) = current_qr_task.take() {
                                task.abort();
                            }
                            let _ = std::fs::remove_file("session.json");
                            let _ = std::fs::remove_file("threads_cache.json");
                            let _ = std::fs::remove_file("zalo_history.db");
                            {
                                let mut s = session.lock().await;
                                s.logout();
                            }
                            {
                                let mut ps = poll_state.lock().await;
                                *ps = crate::chat::PollState::default();
                            }
                            let _ = gui_tx.send(GuiEvent::LoggedOut);
                            egui_ctx.request_repaint();
                        }
                        WorkerCmd::SwitchAccount { target_uin } => {
                            advance_poll_generation(&poll_generation);
                            if let Some((_,control,cancelled))=voice_control.take(){cancelled.store(true,std::sync::atomic::Ordering::Relaxed);let _=control.send(true);}
                            crate::history_sync::PHONE_SYNC_CANCEL.store(true, std::sync::atomic::Ordering::Relaxed);
                            crate::history_sync::REFRESH_CANCEL.store(true, std::sync::atomic::Ordering::Relaxed);
                            let _phone_sync = crate::history_sync::PHONE_SYNC_LOCK.lock().await;
                            let _decoding = crate::history_sync::V2_BATCH_LOCK.lock().await;
                            let cur_uin = {
                                let s = session.lock().await;
                                s.login.as_ref().map(|l| l.uin.clone()).unwrap_or_default()
                            };
                            if let Err(e) = crate::account_manager::switch_to_account(&cur_uin, &target_uin) {
                                let _ = gui_tx.send(GuiEvent::StatusUpdate(format!("⚠️ Lỗi chuyển tài khoản: {e}")));
                                egui_ctx.request_repaint();
                            } else {
                                let mut s = session.lock().await;
                                let mut next = s.clone();
                                next.logout();
                                if next.load_from_file("session.json").is_ok() {
                                    *s = next;
                                    let uin = s.login.as_ref().map(|l| l.uin.clone()).unwrap_or_default();
                                    let name = s.login.as_ref().map(|l| l.display_name.clone()).unwrap_or_default();
                                    let ava = s.login.as_ref().and_then(|l| if l.avatar.is_empty() { None } else { Some(l.avatar.clone()) });
                                    let mut threads = crate::chat::load_threads_cache();
                                    if let Ok(store) = crate::store::MessageStore::open("zalo_history.db") {
                                        let _ = store.hydrate_threads(&mut threads, 50);
                                    }
                                    let _ = gui_tx.send(GuiEvent::AccountSwitched {
                                        uin,
                                        name,
                                        avatar: ava,
                                        threads,
                                    });
                                }
                                egui_ctx.request_repaint();
                            }
                        }
                        WorkerCmd::WipeLocalData => {
                            let _ = std::fs::remove_file("zalo_history.db");
                            let _ = std::fs::remove_file("threads_cache.json");
                            let _ = gui_tx.send(GuiEvent::LocalDataWiped);
                            egui_ctx.request_repaint();
                        }
                        WorkerCmd::RefreshBoardPins {
                            thread_id,
                            is_group,
                        } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let result =
                                    crate::chat::list_board_topics(&s, &thread_id, is_group).await;
                                let (topics, error) = match result {
                                    Ok(list) => (list, None),
                                    Err(e) => (Vec::new(), Some(e.to_string())),
                                };
                                let _ = gui_tx_clone.send(GuiEvent::BoardPinsUpdated {
                                    thread_id,
                                    topics,
                                    error,
                                });
                            });
                        }
                        WorkerCmd::PinMessage {
                            thread_id,
                            is_group,
                            msg,
                            pin,
                            topic_id,
                        } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let result = if pin {
                                    crate::chat::pin_message(&s, &thread_id, is_group, &msg).await
                                } else {
                                    crate::chat::unpin_message(&s, &thread_id, is_group, &topic_id)
                                        .await
                                };
                                match result {
                                    Ok(v) => {
                                        // The server answers with the board topic; keep its id so a
                                        // later unpin targets the right item. `data` may be a JSON
                                        // object or a JSON string, depending on the endpoint.
                                        let data = match v.get("data") {
                                            Some(serde_json::Value::String(txt)) => {
                                                serde_json::from_str::<serde_json::Value>(txt)
                                                    .unwrap_or(serde_json::Value::Null)
                                            }
                                            Some(other) => other.clone(),
                                            None => serde_json::Value::Null,
                                        };
                                        let pick = |val: &serde_json::Value| {
                                            val.as_str()
                                                .map(str::to_string)
                                                .or_else(|| val.as_i64().map(|n| n.to_string()))
                                        };
                                        let topic = pick(&data)
                                            .or_else(|| data.pointer("/topic/id").and_then(pick))
                                            .or_else(|| data.pointer("/id").and_then(pick))
                                            .or_else(|| data.pointer("/topicId").and_then(pick))
                                            .unwrap_or_default();
                                        let _ = gui_tx_clone.send(GuiEvent::MessagePinResult {
                                            thread_id,
                                            msg_id: msg.id.clone(),
                                            pin,
                                            topic_id: topic,
                                            error: None,
                                        });
                                    }
                                    Err(e) => {
                                        let _ = gui_tx_clone.send(GuiEvent::MessagePinResult {
                                            thread_id,
                                            msg_id: msg.id.clone(),
                                            pin,
                                            topic_id: String::new(),
                                            error: Some(e.to_string()),
                                        });
                                    }
                                }
                            });
                        }
                        WorkerCmd::PinThread {
                            thread_id,
                            is_group,
                            pin,
                        } => {
                            let session_clone = session.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let _ = crate::chat::update_pinned_conversation(
                                    &s, &thread_id, is_group, pin,
                                )
                                .await;
                            });
                        }
                        WorkerCmd::MarkThreadUnread {
                            thread_id,
                            is_group,
                            unread,
                        } => {
                            let session_clone = session.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let _ =
                                    crate::chat::set_unread_mark(&s, &thread_id, is_group, unread)
                                        .await;
                            });
                        }
                        WorkerCmd::MuteThread {
                            thread_id,
                            is_group,
                            duration_sec,
                            mute,
                        } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let _ = crate::chat::set_mute_conversation(
                                    &s,
                                    &thread_id,
                                    is_group,
                                    duration_sec,
                                    mute,
                                )
                                .await;
                                if let Ok(mute_resp) = crate::chat::get_mute_conversations(&s).await
                                {
                                    if let Some(mute_map) = crate::unread::extract_mute_map(
                                        &mute_resp,
                                        crate::login::unix_now() as i64,
                                    ) {
                                        let _ = gui_tx_clone
                                            .send(GuiEvent::MuteSnapshotSynced(mute_map));
                                        ctx_clone.request_repaint();
                                    }
                                }
                            });
                        }
                        WorkerCmd::HideThread {
                            thread_id,
                            is_group,
                            hide,
                        } => {
                            let session_clone = session.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let _ = crate::chat::set_hidden_conversation(
                                    &s, &thread_id, is_group, hide,
                                )
                                .await;
                            });
                        }
                        WorkerCmd::DeleteThread {
                            thread_id,
                            is_group,
                            last_cli_msg_id,
                        } => {
                            let session_clone = session.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let _ = crate::chat::delete_conversation(
                                    &s,
                                    &thread_id,
                                    is_group,
                                    &last_cli_msg_id,
                                )
                                .await;
                            });
                        }
                        WorkerCmd::RequestQr => {
                            if let Some(task) = current_qr_task.take() {
                                task.abort();
                            }
                            let session_clone = session.clone();
                            let poll_clone = poll_state.clone();
                            let poll_generation = poll_generation.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();

                            let handle = tokio::spawn(async move {
                                {
                                    let mut s = session_clone.lock().await;
                                    advance_poll_generation(&poll_generation);
                                    s.logout();
                                }
                                {
                                    let mut ps = poll_clone.lock().await;
                                    *ps = crate::chat::PollState::default();
                                }
                                let _ = gui_tx_clone
                                    .send(GuiEvent::StatusUpdate("Đang xin mã QR mới…".into()));
                                ctx_clone.request_repaint();

                                let s_snap = { session_clone.lock().await.clone() };
                                let q_res = crate::qr::request_qr(&s_snap, "vi").await;

                                match q_res {
                                    Ok(q) => {
                                        if let Some(payload) = q.payload() {
                                            if let Ok(code) = qrcode::QrCode::new(payload.as_bytes())
                                            {
                                                let w = code.width();
                                                let quiet_zone = 4;
                                                let total_w = w + quiet_zone * 2;
                                                let mut pixels =
                                                    Vec::with_capacity(total_w * total_w * 4);
                                                for y in 0..total_w {
                                                    for x in 0..total_w {
                                                        let in_qr = x >= quiet_zone
                                                            && x < quiet_zone + w
                                                            && y >= quiet_zone
                                                            && y < quiet_zone + w;
                                                        let dark = in_qr
                                                            && (code[(
                                                                x - quiet_zone,
                                                                y - quiet_zone,
                                                            )] == qrcode::Color::Dark);
                                                        let c = if dark {
                                                            [15, 23, 42, 255]
                                                        } else {
                                                            [255, 255, 255, 255]
                                                        };
                                                        pixels.extend_from_slice(&c);
                                                    }
                                                }
                                                let img = egui::ColorImage::from_rgba_unmultiplied(
                                                    [total_w, total_w],
                                                    &pixels,
                                                );
                                                let _ = gui_tx_clone.send(GuiEvent::QrReady {
                                                    qr_login: q.clone(),
                                                    qr_image: img,
                                                });
                                                ctx_clone.request_repaint();

                                                let scan_res =
                                                    crate::qr::wait_for_scan(&s_snap, &q).await;

                                                if let Err(ref e) = scan_res {
                                                    // Expired / rejected QR: let the GUI fetch a new one.
                                                    let _ = gui_tx_clone.send(GuiEvent::QrExpired {
                                                        reason: e.to_string(),
                                                    });
                                                    ctx_clone.request_repaint();
                                                }
                                                if let Ok(scanned) = scan_res {
                                                    let name = scanned
                                                        .get("display_name")
                                                        .and_then(|v| v.as_str())
                                                        .unwrap_or("Zalo User");
                                                    let _ =
                                                        gui_tx_clone.send(GuiEvent::QrScanned {
                                                            name: name.to_string(),
                                                        });
                                                    ctx_clone.request_repaint();

                                                    let confirm_res =
                                                        crate::qr::confirm(&s_snap, &q).await;

                                                    if let Err(ref e) = confirm_res {
                                                        let _ = gui_tx_clone.send(GuiEvent::QrExpired {
                                                            reason: format!("chưa xác nhận đăng nhập: {e}"),
                                                        });
                                                        ctx_clone.request_repaint();
                                                    }
                                                    if let Ok(resp) = confirm_res {
                                                        let mut s = session_clone.lock().await;
                                                        advance_poll_generation(&poll_generation);
                                                        if let Some(sek) =
                                                            crate::qr::extract_zpw_sek(&resp)
                                                        {
                                                            s.set_zpw_sek(sek);
                                                        }
                                                        if let Some(data) = resp.get("data") {
                                                            crate::login::apply_login_config(
                                                                &mut s, data,
                                                            );
                                                        }
                                                        let _ = s.save_to_file("session.json");

                                                        if let Ok(info) =
                                                            crate::login::get_login_info(&s).await
                                                        {
                                                            if let Some(cfg) = info.get("data") {
                                                                crate::login::apply_login_config(
                                                                    &mut s, cfg,
                                                                );
                                                                s.apply_server_info(cfg);
                                                            }
                                                        }
                                                        let _ = s.sync_profile_me().await;
                                                        let _ = s.save_to_file("session.json");

                                                        let uin = s
                                                            .login
                                                            .as_ref()
                                                            .map(|l| l.uin.clone())
                                                            .unwrap_or_default();
                                                        if let Err(e) = crate::account_manager::activate_account_data(&uin) {
                                                    crate::logger::log_error("account::activate", &e.to_string());
                                                }
                                                let _ = crate::account_manager::save_current_account_snapshot(&uin);
                                                        let dname = s
                                                            .login
                                                            .as_ref()
                                                            .map(|l| l.display_name.clone())
                                                            .unwrap_or_default();
                                                        let u_ava = s
                                                            .login
                                                            .as_ref()
                                                            .map(|l| l.avatar.clone())
                                                            .filter(|a| !a.is_empty());
                                                        let _ = gui_tx_clone.send(
                                                            GuiEvent::LoginSuccess {
                                                                name: dname,
                                                                uin: uin.clone(),
                                                                avatar: u_ava,
                                                            },
                                                        );
                                                        ctx_clone.request_repaint();

                                                        let mut list: Vec<Thread> = Vec::new();
                                                        if let Ok(friends) =
                                                            crate::chat::get_friends(&s).await
                                                        {
                                                            for ft in friends {
                                                                if let Some(ex) = list
                                                                    .iter_mut()
                                                                    .find(|t| t.id == ft.id)
                                                                {
                                                                    if !ft.name.is_empty() {
                                                                        ex.name = ft.name;
                                                                    }
                                                                    if crate::chat::should_replace_avatar(
                                                                        ex.avatar.as_deref(),
                                                                        ft.avatar.as_deref(),
                                                                    ) {
                                                                        ex.avatar = ft.avatar;
                                                                    }
                                                                    ex.is_group =
                                                                        ex.is_group || ft.is_group;
                                                                } else {
                                                                    list.push(ft);
                                                                }
                                                            }
                                                        }
                                                        if let Ok(groups) =
                                                            crate::chat::get_groups(&s).await
                                                        {
                                                            for gt in groups {
                                                                if let Some(ex) = list
                                                                    .iter_mut()
                                                                    .find(|t| t.id == gt.id)
                                                                {
                                                                    if !gt.name.is_empty() {
                                                                        ex.name = gt.name;
                                                                    }
                                                                    if crate::chat::should_replace_avatar(
                                                                        ex.avatar.as_deref(),
                                                                        gt.avatar.as_deref(),
                                                                    ) {
                                                                        ex.avatar = gt.avatar;
                                                                    }
                                                                    ex.presentation.posting_restricted = gt.presentation.posting_restricted;
                                                                    ex.is_group = true;
                                                                } else {
                                                                    list.push(gt);
                                                                }
                                                            }
                                                        }
                                                        let db_path = crate::store::db_path_for_account(&uin);
                                                        crate::store::load_history_into_threads(
                                                            &db_path,
                                                            &mut list,
                                                            crate::store::MessageStore::PRELOAD_MSGS,
                                                        );
                                                        {
                                                            let mut ps = poll_clone.lock().await;
                                                            let _ = crate::chat::poll_new_messages(
                                                                &s, &mut ps, &mut list,
                                                            )
                                                            .await;
                                                        }
                                                        if let Ok(mute_resp) =
                                                            crate::chat::get_mute_conversations(&s)
                                                                .await
                                                        {
                                                            crate::unread::merge_mute_metadata(
                                                                &mut list, &mute_resp,
                                                            );
                                                            if let Some(mute_map) =
                                                                crate::unread::extract_mute_map(
                                                                    &mute_resp,
                                                                    crate::login::unix_now() as i64,
                                                                )
                                                            {
                                                                let _ = gui_tx_clone.send(
                                                                    GuiEvent::MuteSnapshotSynced(
                                                                        mute_map,
                                                                    ),
                                                                );
                                                            }
                                                        }
                                                        if let Ok(pinned_resp) = crate::chat::get_pinned(&s).await {
                                                            let pids = crate::chat::parse_pinned_conversations(&pinned_resp);
                                                            crate::chat::apply_pinned_threads(&mut list, &pids);
                                                            let _ = gui_tx_clone.send(GuiEvent::PinnedSnapshotSynced(pids));
                                                        }
                                                        let sync_snapshot = list.clone();
                                                        let _ = gui_tx_clone
                                                            .send(GuiEvent::ThreadsUpdated(list));
                                                        ctx_clone.request_repaint();

                                                        // Automatically sync Cloud/Group messages upon login without spamming phone prompt (#73)
                                                        if let Some(wtx) = WORKER_TX.get() {
                                                            let _ = wtx.send(
                                                                WorkerCmd::RefreshMessageQueues { threads: sync_snapshot },
                                                            );
                                                        }

                                                        let s_kw = s.clone();
                                                        let tx_kw = gui_tx_clone.clone();
                                                        let ctx_kw = ctx_clone.clone();
                                                        tokio::spawn(async move {
                                                            if let Ok(kws) = crate::chat::fetch_sticker_suggest_keywords(&s_kw).await {
                                                                if !kws.is_empty() {
                                                                    let _ = tx_kw.send(GuiEvent::StickerSuggestKeywordsLoaded(kws));
                                                                    ctx_kw.request_repaint();
                                                                }
                                                            }
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let _ = gui_tx_clone
                                            .send(GuiEvent::LoginFailed(e.to_string()));
                                        ctx_clone.request_repaint();
                                    }
                                }
                            });
                            current_qr_task = Some(handle);
                        }
                        ref cmd @ (WorkerCmd::RefreshThreads | WorkerCmd::RefreshThreadsManual) => {
                            let manual = matches!(cmd, WorkerCmd::RefreshThreadsManual);
                            let session_clone = session.clone();
                            let poll_clone = poll_state.clone();
                            let poll_generation = poll_generation.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let mut s = session_clone.lock().await.clone();
                                if s.login.is_some() {
                                    match s.session_status().await {
                                        crate::session::SessionStatus::Valid => {}
                                        crate::session::SessionStatus::Expired => {
                                            // Keep `session.json`: the old credentials stay
                                            // readable until a new login replaces the file (#21).
                                            {
                                                let mut sl = session_clone.lock().await;
                                                advance_poll_generation(&poll_generation);
                                                sl.mark_expired();
                                                let _ = sl.save_to_file("session.json");
                                                sl.logout();
                                            }
                                            let _ = gui_tx_clone.send(GuiEvent::SessionExpired(
                                                "Phiên đăng nhập (zpw_sek) đã hết hạn trên máy chủ Zalo"
                                                    .into(),
                                            ));
                                            ctx_clone.request_repaint();
                                            if let Some(tx) = WORKER_TX.get() {
                                                let _ = tx.send(WorkerCmd::RequestQr);
                                            }
                                            return;
                                        }
                                        crate::session::SessionStatus::Unreachable => {
                                            // Offline: keep the session, say so, and let the
                                            // next poll retry.
                                            crate::logger::log_error(
                                                "gui::refresh_threads",
                                                "không kết nối được máy chủ Zalo",
                                            );
                                            let _ = gui_tx_clone.send(GuiEvent::SessionUnreachable(
                                                "Không kết nối được máy chủ Zalo".into(),
                                            ));
                                            ctx_clone.request_repaint();
                                            return;
                                        }
                                    }
                                    if let Some((uid, name)) = s.sync_profile_me().await {
                                        {
                                            let mut sl = session_clone.lock().await;
                                            if let Some(ref mut li) = sl.login {
                                                if li.uin != uid {
                                                    advance_poll_generation(&poll_generation);
                                                }
                                                li.uin = uid.clone();
                                                if !name.is_empty() {
                                                    li.display_name = name.clone();
                                                }
                                            }
                                            let _ = sl.save_to_file("session.json");
                                        }
                                        let u_ava = s
                                            .login
                                            .as_ref()
                                            .map(|l| l.avatar.clone())
                                            .filter(|a| !a.is_empty());
                                        let _ = gui_tx_clone.send(GuiEvent::LoginSuccess {
                                            name,
                                            uin: uid,
                                            avatar: u_ava,
                                        });
                                    }
                                    let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(
                                        "⏳ Đang làm mới danh bạ & nhóm từ máy chủ Zalo (1/3)…".into(),
                                    ));
                                    ctx_clone.request_repaint();
                                    let mut list = crate::chat::load_threads_cache();
                                    let mut errors = Vec::new();
                                    match crate::chat::get_friends(&s).await {
                                        Ok(friends) => {
                                            for ft in friends {
                                                if let Some(ex) = list.iter_mut().find(|t| t.id == ft.id) {
                                                    if !ft.name.is_empty() {
                                                        ex.name = ft.name;
                                                    }
                                                    if crate::chat::should_replace_avatar(
                                                        ex.avatar.as_deref(),
                                                        ft.avatar.as_deref(),
                                                    ) {
                                                        ex.avatar = ft.avatar;
                                                    }
                                                    ex.is_group = ex.is_group || ft.is_group;
                                                } else {
                                                    list.push(ft);
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            crate::logger::log_error("gui::refresh_friends", &e.to_string());
                                            errors.push(format!("Danh bạ: {e}"));
                                        }
                                    }
                                    let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(
                                        "⏳ Đang tải danh sách nhóm Zalo (2/3)…".into(),
                                    ));
                                    ctx_clone.request_repaint();
                                    match crate::chat::get_groups(&s).await {
                                        Ok(groups) => {
                                            for gt in groups {
                                                if let Some(ex) = list.iter_mut().find(|t| t.id == gt.id) {
                                                    if !gt.name.is_empty() {
                                                        ex.name = gt.name;
                                                    }
                                                    if crate::chat::should_replace_avatar(
                                                        ex.avatar.as_deref(),
                                                        gt.avatar.as_deref(),
                                                    ) {
                                                        ex.avatar = gt.avatar;
                                                    }
                                                    ex.presentation.posting_restricted = gt.presentation.posting_restricted;
                                                    ex.is_group = true;
                                                } else {
                                                    list.push(gt);
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            crate::logger::log_error("gui::refresh_groups", &e.to_string());
                                            errors.push(format!("Nhóm: {e}"));
                                        }
                                    }
                                    let uin = s.login.as_ref().map(|l| l.uin.as_str()).unwrap_or_default();
                                    let db_path = crate::store::db_path_for_account(uin);
                                    crate::store::load_history_into_threads(
                                        &db_path,
                                        &mut list,
                                        crate::store::MessageStore::PRELOAD_MSGS,
                                    );
                                    let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(
                                        format!("⏳ Đang làm mới tin nhắn cho {} hội thoại (3/3)…", list.len()),
                                    ));
                                    ctx_clone.request_repaint();
                                    let mut fresh_ps = crate::chat::PollState::default();
                                    if let Err(e) =
                                        crate::chat::poll_new_messages(&s, &mut fresh_ps, &mut list)
                                            .await
                                    {
                                        crate::logger::log_error("gui::refresh_poll", &e.to_string());
                                        errors.push(format!("Tin nhắn: {e}"));
                                    }
                                    {
                                        let mut ps = poll_clone.lock().await;
                                        *ps = fresh_ps;
                                    }
                                    if let Ok(mute_resp) =
                                        crate::chat::get_mute_conversations(&s).await
                                    {
                                        crate::unread::merge_mute_metadata(&mut list, &mute_resp);
                                        if let Some(mute_map) = crate::unread::extract_mute_map(
                                            &mute_resp,
                                            crate::login::unix_now() as i64,
                                        ) {
                                            let _ = gui_tx_clone
                                                .send(GuiEvent::MuteSnapshotSynced(mute_map));
                                        }
                                    }
                                    if let Ok(pinned_resp) = crate::chat::get_pinned(&s).await {
                                        let pids = crate::chat::parse_pinned_conversations(&pinned_resp);
                                        crate::chat::apply_pinned_threads(&mut list, &pids);
                                        let _ = gui_tx_clone.send(GuiEvent::PinnedSnapshotSynced(pids));
                                    }
                                    let total_msgs: usize =
                                        list.iter().map(|t| t.messages.len()).sum();
                                    let n_threads = list.len();
                                    let _ = gui_tx_clone.send(GuiEvent::ThreadsUpdated(list));
                                    let _ = gui_tx_clone.send(GuiEvent::RefreshFinished {
                                        threads: n_threads,
                                        messages: total_msgs,
                                        errors,
                                        manual,
                                    });
                                    ctx_clone.request_repaint();
                                }
                            });
                        }
                        WorkerCmd::PollMessages { known_threads } => {
                            static IS_POLLING: std::sync::atomic::AtomicBool =
                                std::sync::atomic::AtomicBool::new(false);
                            if IS_POLLING.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                continue;
                            }

                            let session_clone = session.clone();
                            let poll_clone = poll_state.clone();
                            let poll_generation = poll_generation.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                if s.login.is_some() {
                                    let mut temp_threads: Vec<Thread> = known_threads
                                        .into_iter()
                                        .map(|(id, name, is_group)| Thread {
                                            presentation: Default::default(),
                                            unread: Default::default(),
                                            id,
                                            name,
                                            preview: String::new(),
                                            time: String::new(),
                                            avatar: None,
                                            is_group,
                                            messages: Vec::new(),
                                        })
                                        .collect();

                                    let mut poll_err: Option<anyhow::Error> = None;
                                    for _ in 0..3 {
                                        let poll_res = {
                                            let mut ps = poll_clone.lock().await;
                                            crate::chat::get_chat_from_polling(&s, &mut ps).await
                                        };
                                        match poll_res {
                                            Ok(lp_resp) => {
                                                crate::chat::merge_previews(
                                                    &mut temp_threads,
                                                    &lp_resp,
                                                );
                                                let data_node =
                                                    lp_resp.get("data").unwrap_or(&lp_resp);
                                                for sync_ev in crate::chat::extract_mobile_sync_controls(data_node) {
                                                    crate::history_sync::record_android_control(&s, &sync_ev);
                                                match sync_ev {
                                                        crate::chat::MobileSyncControlEvent::TransferAfterLogin { temp_key } => {
                                                            crate::history_sync::set_latest_mobile_temp_key(&temp_key);
                                                        }
                                                        crate::chat::MobileSyncControlEvent::WaitingConfirmOnPhone { .. } => {
                                                            let note = "📲 Đang chờ bạn bấm 'Đồng bộ' trên màn hình Zalo điện thoại…".to_string();
                                                            crate::history_sync::set_last_mobile_sync_status(Some(note.clone()));
                                                            let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(note));
                                                            ctx_clone.request_repaint();
                                                        }
                                                        crate::chat::MobileSyncControlEvent::MobileBusyRestoring => {
                                                            let note = "⏳ Điện thoại đang nén và chuẩn bị dữ liệu tin nhắn…".to_string();
                                                            crate::history_sync::set_last_mobile_sync_status(Some(note.clone()));
                                                            let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(note));
                                                            ctx_clone.request_repaint();
                                                        }
                                                        crate::chat::MobileSyncControlEvent::UserConfirmed => {
                                                            let note = "📲 Điện thoại đã xác nhận đồng bộ — đang nén và đóng gói tin nhắn…".to_string();
                                                            crate::history_sync::set_last_mobile_sync_status(Some(note.clone()));
                                                            let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(note));
                                                            ctx_clone.request_repaint();
                                                        }
                                                        crate::chat::MobileSyncControlEvent::UserRejected => {
                                                            let note = "⚠️ Bạn đã từ chối yêu cầu đồng bộ tin nhắn trên điện thoại".to_string();
                                                            crate::history_sync::set_last_mobile_sync_status(Some(note.clone()));
                                                            let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(note));
                                                            ctx_clone.request_repaint();
                                                        }
                                                        crate::chat::MobileSyncControlEvent::BackupReady { url, encrypted_key, .. } => {
    let snapshot = s.clone();
    tokio::spawn(async move {
        crate::history_sync::receive_android_backup(&snapshot, &url, &encrypted_key).await;
    });
}
                                                    crate::chat::MobileSyncControlEvent::SyncV2Batch { url, sync_id, batch_type, is_last } => {
    if crate::history_sync::enqueue_v2_batch(&sync_id) {
        let s_sync = s.clone();
        let tx_sync = gui_tx_clone.clone();
        let ctx_sync = ctx_clone.clone();
        tokio::spawn(async move {
            handle_sync_v2_batch(&s_sync, &tx_sync, &ctx_sync, &url, &sync_id, batch_type, is_last).await;
        });
    }
}
                                                    }
                                                }
                                                let more = data_node
                                                    .get("more")
                                                    .and_then(serde_json::Value::as_i64)
                                                    .unwrap_or(0);
                                                let has_reset = data_node
                                                    .get("resetLastActionId")
                                                    .and_then(serde_json::Value::as_str)
                                                    .map(|r| r != "0" && !r.is_empty())
                                                    .unwrap_or(false);
                                                if more == 0 && !has_reset {
                                                    break;
                                                }
                                            }
                                            Err(e) => {
                                                poll_err = Some(e);
                                                break;
                                            }
                                        }
                                    }

                                    match poll_err {
                                        Some(e) => {
                                            if crate::session::SessionStatus::from_error(&e)
                                                == crate::session::SessionStatus::Expired
                                            {
                                                {
                                                    let mut sl = session_clone.lock().await;
                                                    advance_poll_generation(&poll_generation);
                                                    sl.mark_expired();
                                                    let _ = sl.save_to_file("session.json");
                                                    sl.logout();
                                                }
                                                let _ = gui_tx_clone.send(GuiEvent::SessionExpired(
                                                    "Phiên đăng nhập (zpw_sek) đã hết hạn".into(),
                                                ));
                                                ctx_clone.request_repaint();
                                                if let Some(tx) = WORKER_TX.get() {
                                                    let _ = tx.send(WorkerCmd::RequestQr);
                                                }
                                            } else {
                                                // The session is untouched: report and retry on
                                                // the next poll instead of logging the user out.
                                                crate::logger::log_error(
                                                    "gui::poll_messages",
                                                    &e.to_string(),
                                                );
                                                let _ = gui_tx_clone.send(
                                                    GuiEvent::SessionUnreachable(e.to_string()),
                                                );
                                                ctx_clone.request_repaint();
                                            }
                                        }
                                        None => {
                                            temp_threads.retain(|t| !t.messages.is_empty());
                                            if !temp_threads.is_empty() {
                                                // Deliver polled messages to UI immediately without blocking on profile HTTP
                                                let _ = gui_tx_clone
                                                    .send(GuiEvent::ThreadsUpdated(temp_threads.clone()));
                                                ctx_clone.request_repaint();

                                                let needs_resolve = temp_threads.iter().any(|t| {
                                                    t.name.is_empty()
                                                        || t.name.starts_with("Người dùng ")
                                                        || crate::chat::is_generic_thread_name(&t.name)
                                                });
                                                if needs_resolve {
                                                    crate::chat::resolve_thread_names(
                                                        &s,
                                                        &mut temp_threads,
                                                    )
                                                    .await;
                                                    let _ = gui_tx_clone
                                                        .send(GuiEvent::ThreadsUpdated(temp_threads.clone()));
                                                    ctx_clone.request_repaint();
                                                }
                                                let rules = crate::api_mcp::load_autoreply_rules();
                                                if rules.iter().any(|r| r.enabled) {
                                                    for t in &temp_threads {
                                                        for m in &t.messages {
                                                            if !m.is_outgoing {
                                                                if let Some(matched) = crate::api_mcp::match_autoreply_rule(&rules, &m.text, t.is_group) {
                                                                    let reply_text = matched.reply_text;
                                                                    let client_id = crate::chat::next_client_message_id();
                                                                    let res = if t.is_group {
                                                                        crate::chat::send_group_message_with_client_id(&s, &t.id, &reply_text, client_id).await
                                                                    } else {
                                                                        crate::chat::send_message_with_client_id(&s, &t.id, &reply_text, client_id).await
                                                                    };
                                                                    if let Ok(resp) = res {
                                                                        if resp_error_code(&resp).is_none() {
                                                                            let msg_id = extract_msg_id(&resp);
                                                                            let _ = gui_tx_clone.send(GuiEvent::MessageSent {
                                                                                client_message_id: client_id.to_string(),
                                                                                thread_id: t.id.clone(),
                                                                                text: reply_text,
                                                                                msg_id,
                                                                                mentions: Vec::new(),
                                                                            });
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                IS_POLLING.store(false, std::sync::atomic::Ordering::SeqCst);
                            });
                        }
                        WorkerCmd::SyncHistory { threads } => {
                            use std::sync::atomic::Ordering;
                            if crate::history_sync::RUNNING.swap(true, Ordering::SeqCst) {
                                continue;
                            }
                            crate::history_sync::CANCEL.store(false, Ordering::Relaxed);
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let total = threads.len();
                                let mut added = 0;
                                let mut errors = Vec::new();
                                for (i, (id, name, is_group)) in threads.into_iter().enumerate() {
                                    if crate::history_sync::CANCEL.load(Ordering::Relaxed) {
                                        break;
                                    }
                                    let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                        "Đang đồng bộ {}/{total} · {name}",
                                        i + 1
                                    )));
                                    ctx_clone.request_repaint();
                                    let mut t = Thread {
                                        id,
                                        name,
                                        preview: String::new(),
                                        time: String::new(),
                                        avatar: None,
                                        is_group,
                                        messages: Vec::new(),
                                        ..Default::default()
                                    };
                                    match crate::history_sync::sync_thread(&s, &mut t).await {
                                        Ok(n) => added += n,
                                        Err(e) => errors.push(format!("{}: {}", t.name,
                                            crate::history_sync::history_sync_error_message(&e))),
                                    }
                                    // Failed threads must not bypass the page-level pacing.
                                    if i + 1 < total {
                                        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                                    }
                                }
                                let _ = gui_tx_clone.send(GuiEvent::SyncFinished {
                                    added,
                                    errors,
                                    cancelled: crate::history_sync::CANCEL.load(Ordering::Relaxed),
                                });
                                ctx_clone.request_repaint();
                                crate::history_sync::RUNNING.store(false, Ordering::SeqCst);
                            });
                        }
                        WorkerCmd::LoadGroupHistory {
                            group_id,
                            is_group,
                            have,
                        } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                if s.login.is_some() {
                                    let is_send2me = s
                                        .login
                                        .as_ref()
                                        .map(|l| !l.send2me_id.is_empty() && l.send2me_id == group_id)
                                        .unwrap_or(false);
                                    let t = Thread {
                                        presentation: Default::default(),
                                        unread: Default::default(),
                                        id: group_id.clone(),
                                        name: String::new(),
                                        preview: String::new(),
                                        time: String::new(),
                                        avatar: None,
                                        is_group: is_group && !is_send2me,
                                        messages: Vec::new(),
                                    };
                                    let mut t_list = vec![t];
                                    let uin = s.login.as_ref().map(|l| l.uin.as_str()).unwrap_or_default();
                                    let db_path = crate::store::db_path_for_account(uin);
                                    crate::store::load_history_into_threads(
                                        &db_path,
                                        &mut t_list,
                                        (have + 300).max(500),
                                    );
                                    let Some(mut t) =
                                        t_list.into_iter().find(|x| x.id == group_id)
                                    else {
                                        return;
                                    };
                                    if !is_group && !is_send2me {
                                        let mut one_list = vec![t.clone()];
                                        if let Ok(resp) = crate::chat::get_last_msgs(
                                            &s,
                                            std::slice::from_ref(&group_id),
                                        )
                                        .await
                                        {
                                            crate::chat::merge_previews(&mut one_list, &resp);
                                        }
                                        if let Ok(resp_all) =
                                            crate::chat::get_last_msgs(&s, &[]).await
                                        {
                                            crate::chat::merge_previews(&mut one_list, &resp_all);
                                        }
                                        if let Some(updated) =
                                            one_list.into_iter().find(|x| x.id == group_id)
                                        {
                                            t = updated;
                                        }
                                    }
                                    if is_group || is_send2me {
                                        if let Ok(cm_resp) = crate::chat::get_cloud_messages(
                                            &s,
                                            &group_id,
                                            "9223372036854775808",
                                            false,
                                            50,
                                        )
                                        .await
                                        {
                                            crate::chat::merge_group_history(&mut t, &cm_resp);
                                        }
                                        // Backfill up to 5 pages (250 messages) of older cloud history
                                        let mut cursor_id = t
                                            .messages
                                            .first()
                                            .map(|m| m.id.as_str())
                                            .filter(|id| {
                                                !id.is_empty()
                                                    && id.chars().all(|c| c.is_ascii_digit())
                                            })
                                            .unwrap_or("9223372036854775808")
                                            .to_string();
                                        for page in 1..=5 {
                                            let prev_len = t.messages.len();
                                            let Ok(cm_old) = crate::chat::get_cloud_messages(
                                                &s,
                                                &group_id,
                                                &cursor_id,
                                                true,
                                                50,
                                            )
                                            .await
                                            else {
                                                break;
                                            };
                                            crate::chat::merge_group_history(&mut t, &cm_old);
                                            let new_len = t.messages.len();
                                            let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(
                                                format!(
                                                    "⏳ Đang đồng bộ lịch sử cũ (trang {page}/5 — {new_len} tin nhắn)…"
                                                ),
                                            ));
                                            ctx_clone.request_repaint();
                                            let data_node = cm_old
                                                .pointer("/data/data")
                                                .or_else(|| cm_old.get("data"))
                                                .unwrap_or(&cm_old);
                                            let has_more = data_node
                                                .get("hasMore")
                                                .and_then(|v| v.as_i64())
                                                .unwrap_or(0);
                                            let next_cursor = data_node
                                                .get("lastMsgId")
                                                .and_then(|v| {
                                                    v.as_str().map(str::to_string).or_else(|| {
                                                        v.as_u64().map(|n| n.to_string())
                                                    })
                                                })
                                                .filter(|s| {
                                                    !s.is_empty()
                                                        && s != "0"
                                                        && s.chars().all(|c| c.is_ascii_digit())
                                                })
                                                .or_else(|| {
                                                    t.messages
                                                        .first()
                                                        .map(|m| m.id.clone())
                                                        .filter(|id| {
                                                            !id.is_empty()
                                                                && id
                                                                    .chars()
                                                                    .all(|c| c.is_ascii_digit())
                                                        })
                                                });
                                            let Some(nc) = next_cursor else {
                                                break;
                                            };
                                            if has_more == 0 || nc == cursor_id || new_len == prev_len {
                                                break;
                                            }
                                            cursor_id = nc;
                                        }
                                    }
                                    if is_group
                                        && !is_send2me
                                        && (crate::chat::is_generic_thread_name(&t.name)
                                            || t.avatar.as_deref().unwrap_or("").is_empty())
                                    {
                                        crate::chat::resolve_thread_names(
                                            &s,
                                            std::slice::from_mut(&mut t),
                                        )
                                        .await;
                                    }
                                    let cnt = t.messages.len();
                                    if !t.messages.is_empty() || !t.name.is_empty() {
                                        let uin = s.login.as_ref().map(|l| l.uin.as_str()).unwrap_or_default();
                                        let db_path = crate::store::db_path_for_account(uin);
                                        if let Ok(mut store) =
                                            crate::store::MessageStore::open(&db_path)
                                        {
                                            let _ = crate::store::persist_threads(
                                                &mut store,
                                                std::slice::from_ref(&t),
                                            );
                                        }
                                        let _ = gui_tx_clone.send(GuiEvent::ThreadsUpdated(vec![t]));
                                    }
                                    if cnt <= have {
                                        let _ = gui_tx_clone.send(GuiEvent::HistoryReachedStart(group_id.clone()));
                                    } else {
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "✅ Đã tải thêm {} tin nhắn cũ ({cnt} tin trong hội thoại)",
                                            cnt.saturating_sub(have)
                                        )));
                                    }
                                    ctx_clone.request_repaint();
                                }
                            });
                        }
                        WorkerCmd::SendMessage {
                            to_id,
                            text,
                            is_group,
                            mentions,
                        } => {
                            let session_clone = session.clone();
                            let poll_generation = poll_generation.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let client_id = crate::chat::next_client_message_id();
                                // `sendlink` has no verified mention support, so a group text with
                                // tags goes through `group/mention` (the URL stays plain text).
                                let has_tags = is_group && !mentions.is_empty();
                                let link_url = if has_tags {
                                    None
                                } else {
                                    crate::chat::extract_first_url(&text)
                                };
                                if let Some(found_url) = link_url {
                                    let meta = crate::chat::parse_link_metadata(&s, &found_url)
                                        .await
                                        .unwrap_or_else(|_| crate::chat::LinkMetadata {
                                            url: found_url.clone(),
                                            title: found_url.clone(),
                                            desc: String::new(),
                                            thumb: String::new(),
                                            src: "Liên kết".into(),
                                        });
                                    let _ =
                                        gui_tx_clone.send(GuiEvent::LinkPreviewReady(meta.clone()));
                                    match crate::chat::send_link_message_with_client_id(
                                        &s, &to_id, &text, &meta, is_group, client_id)
                                    .await
                                    {
                                        Ok(resp) => {
                                            let msg_id = extract_msg_id(&resp);
                                            let now = crate::ui::clock();
                                            let final_id = if msg_id.is_empty() {
                                                format!("out_{client_id}")
                                            } else {
                                                msg_id
                                            };
                                            let msg = ChatMessage {
                                                id: final_id,
                                                client_message_id: client_id.to_string(),
                                                sender: "Tôi".into(),
                                                text,
                                                time: now,
                                                is_outgoing: true,
                                                ts: (crate::login::unix_now() * 1000) as i64,
                                                link_url: Some(meta.url),
                                                link_title: if meta.title.is_empty() {
                                                    None
                                                } else {
                                                    Some(meta.title)
                                                },
                                                link_desc: if meta.desc.is_empty() {
                                                    None
                                                } else {
                                                    Some(meta.desc)
                                                },
                                                link_thumb: if meta.thumb.is_empty() {
                                                    None
                                                } else {
                                                    Some(meta.thumb)
                                                },
                                                link_src: if meta.src.is_empty() {
                                                    None
                                                } else {
                                                    Some(meta.src)
                                                },
                                                ..Default::default()
                                            };
                                            let _ = gui_tx_clone.send(GuiEvent::AttachmentSent {
                                                thread_id: to_id,
                                                msg,
                                            });
                                            ctx_clone.request_repaint();
                                            return;
                                        }
                                        Err(e) => {
                                            let err_s = e.to_string();
                                            crate::logger::log_error("gui::send_link", &err_s);
                                            let _ = gui_tx_clone.send(GuiEvent::SendFailed {
                                                thread_id: to_id,
                                                text,
                                                error: err_s,
                                            });
                                            ctx_clone.request_repaint();
                                            return;
                                        }
                                    }
                                }

                                let res = if is_group {
                                    crate::chat::send_group_mention_message_with_client_id(
                                        &s, &to_id, &text, client_id, &mentions,
                                    )
                                    .await
                                } else {
                                    crate::chat::send_message_with_client_id(&s, &to_id, &text, client_id).await
                                };
                                match res {
                                    Ok(resp) => {
                                        let msg_id = extract_msg_id(&resp);
                                        let _ = gui_tx_clone.send(GuiEvent::MessageSent {
                                            client_message_id: client_id.to_string(),
                                            mentions: if is_group {
                                                crate::chat::build_mention_info(&text, &mentions)
                                            } else {
                                                Vec::new()
                                            },
                                            thread_id: to_id,
                                            text,
                                            msg_id,
                                        });
                                        ctx_clone.request_repaint();
                                    }
                                    Err(e) => {
                                        let err_s = e.to_string();
                                        crate::logger::log_error("gui::send_message", &err_s);
                                        let _ = gui_tx_clone.send(GuiEvent::SendFailed {
                                            thread_id: to_id,
                                            text,
                                            error: err_s.clone(),
                                        });
                                        if crate::session::SessionStatus::from_error(&e)
                                            == crate::session::SessionStatus::Expired
                                        {
                                            {
                                                let mut sl = session_clone.lock().await;
                                                advance_poll_generation(&poll_generation);
                                                sl.mark_expired();
                                                let _ = sl.save_to_file("session.json");
                                                sl.logout();
                                            }
                                            let _ = gui_tx_clone.send(GuiEvent::SessionExpired(
                                                "Không thể gửi vì phiên zpw_sek đã hết hạn".into(),
                                            ));
                                            if let Some(tx) = WORKER_TX.get() {
                                                let _ = tx.send(WorkerCmd::RequestQr);
                                            }
                                        }
                                        ctx_clone.request_repaint();
                                    }
                                }
                            });
                        }
                        WorkerCmd::SendQuote {
                            to_id,
                            text,
                            quoted,
                            is_group,
                            mentions,
                        } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let client_id = crate::chat::next_client_message_id();
                                let picks: &[crate::chat::MentionPick] =
                                    if is_group { &mentions } else { &[] };
                                match crate::chat::send_quote_message_with_mentions(
                                    &s, &to_id, &text, &quoted, is_group, client_id, picks)
                                .await
                                {
                                    Ok(resp) => {
                                        let mentions = crate::chat::build_mention_info(&text, picks);
                                        let _ = gui_tx_clone.send(GuiEvent::QuoteSent {
                                            client_message_id: client_id.to_string(),
                                            thread_id: to_id,
                                            text,
                                            msg_id: extract_msg_id(&resp),
                                            quoted,
                                            mentions,
                                        });
                                    }
                                    Err(e) => {
                                        let err_s = e.to_string();
                                        crate::logger::log_error("gui::send_quote", &err_s);
                                        let _ = gui_tx_clone.send(GuiEvent::SendFailed {
                                            thread_id: to_id,
                                            text,
                                            error: err_s,
                                        });
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::Forward {
                            target_id,
                            target_is_group,
                            msg,
                            target_name,
                            note,
                        } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let client_id = crate::chat::next_client_message_id();
                                let res = crate::chat::forward_message_with_client_id(
                                    &s,
                                    &target_id,
                                    target_is_group,
                                    &msg, client_id)
                                .await;
                                match res {
                                    Ok(resp) if resp_error_code(&resp).is_none() => {
                                        let msg_id = extract_msg_id(&resp);
                                        if let Some(sid) = msg.sticker_id.filter(|v| *v > 0) {
                                            let _ = gui_tx_clone.send(GuiEvent::StickerSent {
                                                client_message_id: client_id.to_string(),
                                                thread_id: target_id.clone(),
                                                msg_id,
                                                sticker_id: sid,
                                                cate_id: msg.sticker_cate_id.unwrap_or(0),
                                                sticker_type: msg.sticker_type.unwrap_or(7),
                                            });
                                        } else if msg.link_url.is_some() {
                                            let clean_text = if let Some(stripped) = msg.text.strip_prefix("[Đã thu hồi] ") {
                                                stripped
                                            } else {
                                                &msg.text
                                            };
                                            let out_msg = ChatMessage {
                                                id: if msg_id.is_empty() { format!("out_{client_id}") } else { msg_id },
                                                client_message_id: client_id.to_string(),
                                                sender: "Tôi".into(),
                                                text: clean_text.to_string(),
                                                time: crate::ui::clock(),
                                                is_outgoing: true,
                                                ts: (crate::login::unix_now() * 1000) as i64,
                                                link_url: msg.link_url.clone(),
                                                link_thumb: msg.link_thumb.clone(),
                                                link_src: msg.link_src.clone(),
                                                ..Default::default()
                                            };
                                            let _ = gui_tx_clone.send(GuiEvent::AttachmentSent {
                                                thread_id: target_id.clone(),
                                                msg: out_msg,
                                            });
                                        } else {
                                            let clean_text = if let Some(stripped) = msg.text.strip_prefix("[Đã thu hồi] ") {
                                                stripped
                                            } else {
                                                &msg.text
                                            };
                                            let _ = gui_tx_clone.send(GuiEvent::MessageSent {
                                                client_message_id: client_id.to_string(),
                                                thread_id: target_id.clone(),
                                                text: clean_text.to_string(),
                                                msg_id,
                                                mentions: Vec::new(),
                                            });
                                        }
                                        // The share-dialog note goes after the forwarded message.
                                        if let Some(note) = note.filter(|n| !n.trim().is_empty()) {
                                            let note_id = crate::chat::next_client_message_id();
                                            let sent = if target_is_group {
                                                crate::chat::send_group_message_with_client_id(
                                                    &s, &target_id, &note, note_id,
                                                )
                                                .await
                                            } else {
                                                crate::chat::send_message_with_client_id(
                                                    &s, &target_id, &note, note_id,
                                                )
                                                .await
                                            };
                                            match sent {
                                                Ok(resp) => {
                                                    let _ = gui_tx_clone.send(GuiEvent::MessageSent {
                                                        client_message_id: note_id.to_string(),
                                                        thread_id: target_id.clone(),
                                                        text: note,
                                                        msg_id: extract_msg_id(&resp),
                                                        mentions: Vec::new(),
                                                    });
                                                }
                                                Err(e) => {
                                                    crate::logger::log_error(
                                                        "gui::forward_note",
                                                        &e.to_string(),
                                                    );
                                                }
                                            }
                                        }
                                        let _ = gui_tx_clone.send(GuiEvent::ForwardFinished {
                                            target_name: target_name.clone(),
                                            error: None,
                                        });
                                    }
                                    Ok(resp) => {
                                        let err_s = format!(
                                            "server error {}",
                                            resp_error_code(&resp).unwrap_or(0)
                                        );
                                        crate::logger::log_error("gui::forward", &err_s);
                                        let _ = gui_tx_clone.send(GuiEvent::ForwardFinished {
                                            target_name,
                                            error: Some(err_s),
                                        });
                                    }
                                    Err(e) => {
                                        let err_s = e.to_string();
                                        crate::logger::log_error("gui::forward", &err_s);
                                        let _ = gui_tx_clone.send(GuiEvent::ForwardFinished {
                                            target_name,
                                            error: Some(err_s),
                                        });
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::React {
                            to_id,
                            is_group,
                            msg,
                            reaction,
                        } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let res = crate::chat::send_reaction(
                                    &s, &to_id, is_group, &msg, &reaction,
                                )
                                .await;
                                match res {
                                    Ok(resp) if resp_error_code(&resp).is_none() => {
                                        let _ = gui_tx_clone.send(GuiEvent::Reacted {
                                            thread_id: to_id,
                                            msg_id: msg.id.clone(),
                                            ts: msg.ts,
                                            reaction,
                                        });
                                    }
                                    Ok(resp) => {
                                        let err_s = format!(
                                            "server error {}",
                                            resp_error_code(&resp).unwrap_or(0)
                                        );
                                        crate::logger::log_error("gui::react", &err_s);
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "Thả cảm xúc thất bại: {err_s}"
                                        )));
                                    }
                                    Err(e) => {
                                        crate::logger::log_error("gui::react", &e.to_string());
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "Thả cảm xúc thất bại: {e}"
                                        )));
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::DeleteForMe {
                            to_id,
                            is_group,
                            msg,
                        } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let local_only = msg.id.is_empty()
                                    || msg.id.starts_with("out_")
                                    || msg.id.starts_with("local_");
                                let res: anyhow::Result<serde_json::Value> = if local_only {
                                    Ok(serde_json::json!({}))
                                } else {
                                    crate::chat::delete_message_for_me(&s, &to_id, is_group, &msg)
                                        .await
                                };
                                match res {
                                    Ok(resp) if resp_error_code(&resp).is_none() => {
                                        let (tid, mid, ts) =
                                            (to_id.clone(), msg.id.clone(), msg.ts);
                                        let _ = tokio::task::spawn_blocking(move || {
                                            if let Ok(mut store) =
                                                crate::store::MessageStore::open("zalo_history.db")
                                            {
                                                let _ = store.delete_message(&tid, &mid, ts);
                                            }
                                        })
                                        .await;
                                        let _ = gui_tx_clone.send(GuiEvent::MessageDeleted {
                                            thread_id: to_id,
                                            msg_id: msg.id.clone(),
                                            ts: msg.ts,
                                        });
                                    }
                                    Ok(resp) => {
                                        let err_s = format!(
                                            "server error {}",
                                            resp_error_code(&resp).unwrap_or(0)
                                        );
                                        crate::logger::log_error("gui::delete", &err_s);
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "Xóa tin nhắn thất bại: {err_s}"
                                        )));
                                    }
                                    Err(e) => {
                                        crate::logger::log_error("gui::delete", &e.to_string());
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "Xóa tin nhắn thất bại: {e}"
                                        )));
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::RecallMessage {
                            to_id,
                            is_group,
                            msg,
                        } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let local_only = msg.id.is_empty()
                                    || msg.id.starts_with("out_")
                                    || msg.id.starts_with("local_");
                                let res: anyhow::Result<serde_json::Value> = if local_only {
                                    Ok(serde_json::json!({}))
                                } else {
                                    crate::chat::recall_message(&s, &to_id, is_group, &msg).await
                                };
                                match res {
                                    Ok(resp) if resp_error_code(&resp).is_none() => {
                                        let _ = gui_tx_clone.send(GuiEvent::MessageRecalled {
                                            thread_id: to_id,
                                            msg_id: msg.id.clone(),
                                            ts: msg.ts,
                                        });
                                    }
                                    Ok(resp) => {
                                        let err_s = format!(
                                            "server error {}",
                                            resp_error_code(&resp).unwrap_or(0)
                                        );
                                        crate::logger::log_error("gui::recall", &err_s);
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "Đã thu hồi cục bộ (Máy chủ: {err_s})"
                                        )));
                                    }
                                    Err(e) => {
                                        crate::logger::log_error("gui::recall", &e.to_string());
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "Không thể thu hồi trên máy chủ: {e}"
                                        )));
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::FetchConvLabels => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let res = crate::chat::fetch_conv_labels(&s)
                                    .await
                                    .map_err(|e| e.to_string());
                                if let Err(ref e) = res {
                                    crate::logger::log_error("chat::fetch_conv_labels", e);
                                }
                                let _ = gui_tx_clone.send(GuiEvent::ConvLabelsLoaded(res));
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::SearchGifs { query, seq } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let res = if query.trim().is_empty() {
                                    crate::chat::gif_trending(&s, 0, GIF_RESULT_LIMIT).await
                                } else {
                                    crate::chat::gif_search(&s, query.trim(), 0, GIF_RESULT_LIMIT)
                                        .await
                                };
                                let (items, error) = match res {
                                    Ok(items) => (items, None),
                                    Err(e) => {
                                        crate::logger::log_error("chat::gif_search", &e.to_string());
                                        (Vec::new(), Some(e.to_string()))
                                    }
                                };
                                let _ = gui_tx_clone.send(GuiEvent::GifResults { seq, items, error });
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::SendGif {
                            to_id,
                            is_group,
                            item,
                            src,
                        } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let client_id = crate::chat::next_client_message_id();
                                match crate::chat::send_gif_with_client_id(
                                    &s, &to_id, is_group, &item, src, client_id,
                                )
                                .await
                                {
                                    Ok((_resp, msg)) => {
                                        let _ = gui_tx_clone.send(GuiEvent::AttachmentSent {
                                            thread_id: to_id,
                                            msg,
                                        });
                                    }
                                    Err(e) => {
                                        let err_s = e.to_string();
                                        crate::logger::log_error("chat::send_gif", &err_s);
                                        let _ = gui_tx_clone.send(GuiEvent::SendFailed {
                                            thread_id: to_id,
                                            text: "[GIF]".into(),
                                            error: format!("Lỗi gửi GIF: {err_s}"),
                                        });
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::SendSticker {
                            to_id,
                            sticker_id,
                            cate_id,
                            sticker_type,
                            is_group,
                        } => {
                            let session_clone = session.clone();
                            let poll_generation = poll_generation.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            if cate_id == AMI_FAT_CAT_CATE_ID {
                                if let Some((_lbl, png_bytes, gif_bytes)) =
                                    get_ami_fat_cat_gif(sticker_id)
                                {
                                    tokio::spawn(async move {
                                        let idx = sticker_id.saturating_sub(999000);
                                        let _ = std::fs::create_dir_all("stickers_cache");
                                        let gif_path = std::path::PathBuf::from(format!(
                                            "stickers_cache/ami_fat_cat_{idx:02}.gif"
                                        ));
                                        if !gif_path.exists() {
                                            let _ = std::fs::write(&gif_path, gif_bytes);
                                        }
                                        let key = format!("stk_{cate_id}_{sticker_id}");
                                        if let Ok(dyn_img) = image::load_from_memory(png_bytes) {
                                            let rgba = dyn_img.to_rgba8();
                                            let size =
                                                [rgba.width() as usize, rgba.height() as usize];
                                            let pixels = rgba.as_flat_samples();
                                            let color_img =
                                                egui::ColorImage::from_rgba_unmultiplied(
                                                    size,
                                                    pixels.as_slice(),
                                                );
                                            let _ = gui_tx_clone
                                                .send(GuiEvent::StickerImageReady {
                                                    key,
                                                    image: color_img,
                                                });
                                        }
                                        let s = session_clone.lock().await.clone();
                                        let client_id = crate::chat::next_client_message_id();
                                        let ami_tag = format!("#ami_fat_cat_{idx:02}");
                                        match crate::chat::send_attachment(
                                            &s, &to_id, is_group, &gif_path, &ami_tag,
                                        )
                                        .await
                                        {
                                            Ok((_resp, cm)) => {
                                                let _ = gui_tx_clone.send(GuiEvent::StickerSent {
                                                    client_message_id: cm.client_message_id,
                                                    thread_id: to_id,
                                                    msg_id: cm.id,
                                                    sticker_id,
                                                    cate_id,
                                                    sticker_type,
                                                });
                                            }
                                            Err(e) => {
                                                let err_s = e.to_string();
                                                crate::logger::log_error(
                                                    "gui::send_ami_sticker",
                                                    &err_s,
                                                );
                                                let _ = gui_tx_clone.send(GuiEvent::StickerSent {
                                                    client_message_id: client_id.to_string(),
                                                    thread_id: to_id,
                                                    msg_id: format!("out_{client_id}"),
                                                    sticker_id,
                                                    cate_id,
                                                    sticker_type,
                                                });
                                            }
                                        }
                                        ctx_clone.request_repaint();
                                    });
                                    continue;
                                }
                            }
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let client_id = crate::chat::next_client_message_id();
                                let res = crate::chat::send_sticker_with_client_id(
                                    &s,
                                    &to_id,
                                    sticker_id,
                                    cate_id,
                                    sticker_type,
                                    is_group, client_id)
                                .await;
                                match res {
                                    Ok(resp) => {
                                        let msg_id = resp
                                            .get("data")
                                            .and_then(|d| d.get("msgId"))
                                            .and_then(|v| match v {
                                                serde_json::Value::String(s) => Some(s.clone()),
                                                serde_json::Value::Number(n) => Some(n.to_string()),
                                                _ => None,
                                            })
                                            .unwrap_or_default();
                                        let _ = gui_tx_clone.send(GuiEvent::StickerSent {
                                            client_message_id: client_id.to_string(),
                                            thread_id: to_id,
                                            msg_id,
                                            sticker_id,
                                            cate_id,
                                            sticker_type,
                                        });
                                        ctx_clone.request_repaint();
                                    }
                                    Err(e) => {
                                        let err_s = e.to_string();
                                        crate::logger::log_error("gui::send_sticker", &err_s);
                                        let _ = gui_tx_clone.send(GuiEvent::SendFailed {
                                            thread_id: to_id,
                                            text: String::new(),
                                            error: err_s.clone(),
                                        });
                                        if crate::session::SessionStatus::from_error(&e)
                                            == crate::session::SessionStatus::Expired
                                        {
                                            {
                                                let mut sl = session_clone.lock().await;
                                                advance_poll_generation(&poll_generation);
                                                sl.mark_expired();
                                                let _ = sl.save_to_file("session.json");
                                                sl.logout();
                                            }
                                            let _ = gui_tx_clone.send(GuiEvent::SessionExpired(
                                                "Không thể gửi nhãn dán vì phiên zpw_sek đã hết hạn"
                                                    .into(),
                                            ));
                                            if let Some(tx) = WORKER_TX.get() {
                                                let _ = tx.send(WorkerCmd::RequestQr);
                                            }
                                        }
                                        ctx_clone.request_repaint();
                                    }
                                }
                            });
                        }
                        WorkerCmd::FetchStickerImage { key, url } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            if key.starts_with("stk_99901_") {
                                if let Some(sid) = key
                                    .strip_prefix("stk_99901_")
                                    .and_then(|s| s.parse::<u64>().ok())
                                {
                                    if let Some((_lbl, png_bytes, _gif)) = get_ami_fat_cat_gif(sid)
                                    {
                                        if let Ok(dyn_img) = image::load_from_memory(png_bytes) {
                                            let rgba = dyn_img.to_rgba8();
                                            let size =
                                                [rgba.width() as usize, rgba.height() as usize];
                                            let pixels = rgba.as_flat_samples();
                                            let color_img =
                                                egui::ColorImage::from_rgba_unmultiplied(
                                                    size,
                                                    pixels.as_slice(),
                                                );
                                            let _ = gui_tx.send(GuiEvent::StickerImageReady {
                                                key,
                                                image: color_img,
                                            });
                                            egui_ctx.request_repaint();
                                        }
                                        continue;
                                    }
                                }
                            }
                            tokio::spawn(async move {
                                let bounded_video_poster = key.starts_with("video_thumb_");
                                let cache_dir = std::path::Path::new("stickers_cache");
                                let _ = std::fs::create_dir_all(cache_dir);
                                let cache_path = cache_dir.join(format!("{key}.png"));

                                // Bytes come from the disk cache, a local file or the network. They
                                // used to be decoded once to validate and again to use; now once, on
                                // the blocking pool so a large photo cannot stall the 2-thread runtime.
                                let cached_is_bounded = !bounded_video_poster
                                    || std::fs::metadata(&cache_path)
                                        .is_ok_and(|meta| {
                                            meta.len()
                                                <= crate::media_resources::PREVIEW_LIMIT as u64
                                        });
                                let cached_bytes = if cached_is_bounded {
                                    std::fs::read(&cache_path).ok()
                                } else {
                                    None
                                };
                                let (bytes_opt, from_network) =
                                    if let Some(disk_bytes) = cached_bytes {
                                        (Some(disk_bytes), false)
                                    } else if !url.starts_with("http://")
                                        && !url.starts_with("https://")
                                    {
                                        let file_path = url.strip_prefix("local://").unwrap_or(&url);
                                        (std::fs::read(file_path).ok(), false)
                                    } else {
                                        let fetch_url = crate::store::jxl_to_jpg_url(&url);
                                        let client = if use_session_for_sticker_image(&fetch_url) {
                                            Ok(session_clone.lock().await.http.clone())
                                        } else {
                                            crate::media_resources::credential_free_client()
                                        };
                                        let body = if let Ok(client) = client {
                                            match client
                                                .get(&fetch_url)
                                                .header("Accept", "image/avif,image/webp,image/apng,image/*,*/*;q=0.8")
                                                .timeout(std::time::Duration::from_secs(35))
                                                .send()
                                                .await
                                            {
                                                Ok(resp) => {
                                                    crate::media_resources::read_bounded_response(
                                                        resp,
                                                        sticker_image_body_limit(&key),
                                                    )
                                                    .await
                                                    .ok()
                                                }
                                                Err(_) => None,
                                            }
                                        } else {
                                            None
                                        };
                                        (body, true)
                                    };
                                let Some(bytes) = bytes_opt else {
                                    let _ = gui_tx_clone.send(GuiEvent::StickerImageFailed { key });
                                    ctx_clone.request_repaint();
                                    return;
                                };

                                let _ = tokio::task::spawn_blocking(move || {
                                    let decoded = if bounded_video_poster {
                                        let mut reader = image::ImageReader::new(
                                            std::io::Cursor::new(&bytes),
                                        );
                                        reader.set_format(image::guess_format(&bytes).unwrap_or(image::ImageFormat::Jpeg));
                                        let mut limits = image::Limits::default();
                                        limits.max_image_width = Some(8192);
                                        limits.max_image_height = Some(8192);
                                        limits.max_alloc = Some(64 * 1024 * 1024);
                                        reader.limits(limits);
                                        reader.decode()
                                    } else {
                                        image::load_from_memory(&bytes)
                                    };
                                    let Ok(mut dyn_img) = decoded else {
                                        if !from_network {
                                            // Corrupt cache entry: drop it so the next request refetches.
                                            let _ = std::fs::remove_file(&cache_path);
                                        }
                                        let _ = gui_tx_clone.send(GuiEvent::StickerImageFailed { key });
                                        ctx_clone.request_repaint();
                                        return;
                                    };
                                    if from_network {
                                        let _ = std::fs::write(&cache_path, &bytes);
                                    }
                                    // Bubbles draw images up to 360 pt wide; resize large photos to 720px max
                                    // so inline thumbnails stay crisp on Retina/HiDPI displays (#72).
                                    // The viewer's `hd_` textures keep up to `HD_TEXTURE_MAX` px: the GPU
                                    // refuses (and egui panics on) textures past its size limit, and the
                                    // viewer now preloads neighbouring originals. Copy/save read the
                                    // untouched bytes from the disk cache.
                                    if key.starts_with("hd_") {
                                        if dyn_img.width() > HD_TEXTURE_MAX
                                            || dyn_img.height() > HD_TEXTURE_MAX
                                        {
                                            dyn_img =
                                                dyn_img.resize(HD_TEXTURE_MAX, HD_TEXTURE_MAX, image::imageops::FilterType::Triangle);
                                        }
                                    } else if !key.starts_with("ava_")
                                        && (dyn_img.width() > 720 || dyn_img.height() > 720)
                                    {
                                        dyn_img = dyn_img.thumbnail(720, 720);
                                    }
                                    {
                                        let rgba = if key.starts_with("ava_") {
                                            prepare_avatar_texture(dyn_img)
                                        } else {
                                            dyn_img.to_rgba8()
                                        };
                                        let size =
                                            [rgba.width() as usize, rgba.height() as usize];
                                        let pixels = rgba.as_flat_samples();
                                        let color_img = egui::ColorImage::from_rgba_unmultiplied(
                                            size,
                                            pixels.as_slice(),
                                        );
                                        let _ = gui_tx_clone.send(GuiEvent::StickerImageReady {
                                            key,
                                            image: color_img,
                                        });
                                        ctx_clone.request_repaint();
                                    }
                                })
                                .await;
                            });
                        }
                        WorkerCmd::RasterizeEmoji { key, cluster } => {
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            // Font mapping and PNG decode are blocking work (INV-3).
                            tokio::task::spawn_blocking(move || {
                                let event = match crate::emoji_raster::rasterize(&cluster, 96) {
                                    Some(bmp) => {
                                        let edge = bmp.size as usize;
                                        GuiEvent::StickerImageReady {
                                            key,
                                            image: egui::ColorImage::from_rgba_unmultiplied(
                                                [edge, edge],
                                                &bmp.rgba,
                                            ),
                                        }
                                    }
                                    None => GuiEvent::StickerImageFailed { key },
                                };
                                let _ = gui_tx_clone.send(event);
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::FetchZaloEmojiTile { col, row } => {
                            let gui_tx = gui_tx.clone();
                            let sprite = zalo_emoji_sprite.clone();
                            tokio::spawn(async move {
                                use crate::zalo_emoticons;

                                let key = zalo_emoticons::texture_key(col, row);
                                let tile = sprite
                                    .get_or_init(zalo_emoticons::load_sprite_with_retry)
                                    .await
                                    .as_ref()
                                    .and_then(|image| zalo_emoticons::crop_tile(image, col, row));
                                let event = match tile {
                                    Some(tile) => {
                                        let edge = zalo_emoticons::TILE_SIZE as usize;
                                        GuiEvent::StickerImageReady {
                                            key,
                                            image: egui::ColorImage::from_rgba_unmultiplied(
                                                [edge, edge],
                                                tile.as_raw(),
                                            ),
                                        }
                                    }
                                    None => GuiEvent::StickerImageFailed { key },
                                };
                                let _ = gui_tx.send(event);
                            });
                        }
                        WorkerCmd::FetchStickerStore => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                match crate::chat::fetch_sticker_store_packs(&s).await {
                                    Ok(packs) => {
                                        let _ = gui_tx_clone
                                            .send(GuiEvent::StickerStoreLoaded(packs));
                                    }
                                    Err(e) => {
                                        crate::logger::log_error(
                                            "gui::fetch_sticker_store",
                                            &e.to_string(),
                                        );
                                        let _ = gui_tx_clone
                                            .send(GuiEvent::StickerStoreLoaded(Vec::new()));
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "Không tải được kho nhãn dán: {e}"
                                        )));
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::FetchStickerPack { cate_id, name } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                match crate::chat::fetch_sticker_pack_items(&s, cate_id, &name)
                                    .await
                                {
                                    Ok(items) => {
                                        let _ = gui_tx_clone.send(
                                            GuiEvent::StickerPackItemsLoaded {
                                                cate_id,
                                                name,
                                                items,
                                            },
                                        );
                                    }
                                    Err(e) => {
                                        crate::logger::log_error(
                                            "gui::fetch_sticker_pack",
                                            &e.to_string(),
                                        );
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "Không tải được bộ nhãn \"{name}\": {e}"
                                        )));
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::SearchStickers { keyword } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                match crate::chat::search_stickers_by_keyword(&s, &keyword).await {
                                    Ok(items) => {
                                        let missing = preload_disk_sticker_thumbnails(
                                            &items,
                                            &gui_tx_clone,
                                            24,
                                        );
                                        let _ = gui_tx_clone.send(GuiEvent::StickerSearchResults {
                                            keyword,
                                            items,
                                        });
                                        ctx_clone.request_repaint();
                                        prefetch_missing_sticker_thumbnails(
                                            &s.http,
                                            missing,
                                            &gui_tx_clone,
                                            &ctx_clone,
                                        )
                                        .await;
                                    }
                                    Err(e) => {
                                        crate::logger::log_error(
                                            "gui::search_stickers",
                                            &e.to_string(),
                                        );
                                        let _ = gui_tx_clone.send(GuiEvent::StickerSearchResults {
                                            keyword,
                                            items: Vec::new(),
                                        });
                                        ctx_clone.request_repaint();
                                    }
                                }
                            });
                        }
                        WorkerCmd::SuggestComposerStickers { keyword } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let items = crate::chat::search_stickers_by_keyword(&s, &keyword)
                                    .await
                                    .unwrap_or_default();
                                let missing =
                                    preload_disk_sticker_thumbnails(&items, &gui_tx_clone, 16);
                                let _ =
                                    gui_tx_clone.send(GuiEvent::ComposerStickerSuggestions {
                                        keyword,
                                        items,
                                    });
                                ctx_clone.request_repaint();
                                prefetch_missing_sticker_thumbnails(
                                    &s.http,
                                    missing,
                                    &gui_tx_clone,
                                    &ctx_clone,
                                )
                                .await;
                            });
                        }
                        WorkerCmd::ParseLinkPreview { url } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                if let Ok(meta) = crate::chat::parse_link_metadata(&s, &url).await {
                                    let _ = gui_tx_clone.send(GuiEvent::LinkPreviewReady(meta));
                                    ctx_clone.request_repaint();
                                }
                            });
                        }
                        WorkerCmd::PickAndSendAttachment {
                            to_id,
                            is_group,
                            images_only,
                            caption,
                            preset_path,
                            original_hd,
                        } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let picked_path = if let Some(p) = preset_path {
                                    Some(p)
                                } else {
                                    tokio::task::spawn_blocking(move || {
                                        pick_native_file_dialog(images_only)
                                    })
                                    .await
                                    .ok()
                                    .flatten()
                                };

                                let Some(path) = picked_path else {
                                    return;
                                };
                                let fname = path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("tập tin")
                                    .to_string();
                                let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                    "⏳ Đang tải lên và gửi '{fname}'…"
                                )));
                                ctx_clone.request_repaint();

                                let s = session_clone.lock().await.clone();
                                match crate::chat::send_attachment_ext(
                                    &s, &to_id, is_group, &path, &caption, original_hd,
                                )
                                .await
                                {
                                    Ok((_resp, cm)) => {
                                        let _ = gui_tx_clone.send(GuiEvent::AttachmentSent {
                                            thread_id: to_id,
                                            msg: cm,
                                        });
                                    }
                                    Err(e) => {
                                        let err_s = e.to_string();
                                        crate::logger::log_error("gui::send_attachment", &err_s);
                                        let _ = gui_tx_clone.send(GuiEvent::SendFailed {
                                            thread_id: to_id,
                                            text: caption,
                                            error: format!("Lỗi gửi '{fname}': {err_s}"),
                                        });
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::PickQuickMessageImage { request_id } => {
                            let gui_tx_clone = gui_tx.clone();
                            tokio::spawn(async move {
                                let result = tokio::task::spawn_blocking(move || {
                                    let Some(source) = pick_native_file_dialog(true) else {
                                        return Ok(None);
                                    };
                                    let root = std::env::current_dir()
                                        .map_err(|error| error.to_string())?;
                                    crate::quick_messages::import_image(&root, &source)
                                        .map(Some)
                                        .map_err(|error| error.to_string())
                                })
                                .await
                                .unwrap_or_else(|error| Err(error.to_string()));
                                let _ = gui_tx_clone.send(GuiEvent::QuickMessageImagePicked {
                                    request_id,
                                    result,
                                });
                            });
                        }
                        WorkerCmd::DeleteQuickMessageImage { path } => {
                            let gui_tx_clone = gui_tx.clone();
                            tokio::spawn(async move {
                                let result = tokio::task::spawn_blocking({
                                    let path = path.clone();
                                    move || -> Result<(), String> {
                                        let root = std::env::current_dir()
                                            .map_err(|error| error.to_string())?;
                                        crate::quick_messages::delete_image(&root, &path)
                                            .map_err(|error| error.to_string())
                                    }
                                })
                                .await
                                .unwrap_or_else(|error| Err(error.to_string()));
                                let _ = gui_tx_clone.send(GuiEvent::QuickMessageImageDeleted {
                                    error: result.err(),
                                });
                            });
                        }
                        command @ (WorkerCmd::ScanStorage | WorkerCmd::ClearStorage { .. }) => {
                            let guard = storage_operation.clone().try_lock_owned();
                            let account = session.lock().await.login.as_ref()
                                .map(|login| login.uin.clone())
                                .filter(|uin| !uin.is_empty());
                            let tx = gui_tx.clone();
                            tokio::spawn(async move {
                                let event = if let WorkerCmd::ClearStorage { target, expected_account, protected } = command {
                                    let result = if target == (ClearTarget::Media { current_account_only: true })
                                        && account.as_deref() != Some(expected_account.as_str())
                                    {
                                        drop(guard);
                                        Err("Tài khoản hiện tại đã thay đổi. Hãy quét lại bộ nhớ.".into())
                                    } else {
                                        run_storage_task(guard, move |root| {
                                            crate::storage_usage::clear(
                                                root,
                                                target,
                                                account.as_deref(),
                                                std::time::SystemTime::now(),
                                                &protected,
                                            )
                                        }).await
                                    };
                                    GuiEvent::StorageCleared(result)
                                } else {
                                    let result = run_storage_task(guard, move |root| {
                                        crate::storage_usage::scan(root, account.as_deref())
                                    }).await;
                                    GuiEvent::StorageScanned(result)
                                };
                                let _ = tx.send(event);
                            });
                        }
                        WorkerCmd::TriggerScreenshot => {
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::task::spawn_blocking(move || {
                                capture_screen_interactive(gui_tx_clone, ctx_clone);
                            });
                        }
                        WorkerCmd::StartCall {
                            partner_id,
                            partner_name,
                            partner_avatar,
                            is_video,
                            is_group,
                        } => {
                            if let Some((_,control,cancelled))=voice_control.take(){cancelled.store(true,std::sync::atomic::Ordering::Relaxed);let _=control.send(true);}
                            let kind_label = if is_video { "Gọi video" } else { "Gọi thoại" };
                            call_bridge.make_call(
                                &partner_id,
                                &partner_name,
                                partner_avatar.as_deref(),
                                is_video,
                                is_group,
                            );
                            let status_msg = format!(
                                "📞 Đang khởi tạo {kind_label} trực tiếp (Pure-Rust) tới {partner_name} ({partner_id})…"
                            );
                            let _ = gui_tx.send(GuiEvent::StatusUpdate(status_msg));
                            egui_ctx.request_repaint();
                        }
                        WorkerCmd::AnswerCall {
                            accept,
                            cam_start_off,
                        } => {
                            if let Some((_,control,cancelled))=voice_control.take(){cancelled.store(true,std::sync::atomic::Ordering::Relaxed);let _=control.send(true);}
                            call_bridge.answer_call(accept, cam_start_off);
                            egui_ctx.request_repaint();
                        }
                        WorkerCmd::SetCallMediaState {
                            is_on_mic,
                            is_on_cam,
                        } => {
                            call_bridge.set_media_state(is_on_mic, is_on_cam);
                            egui_ctx.request_repaint();
                        }
                        WorkerCmd::InviteGroupCallMembers { user_ids } => {
                            call_bridge.invite_group_members(user_ids);
                            egui_ctx.request_repaint();
                        }
                        WorkerCmd::EndCall => {
                            call_bridge.end_call();
                            let _ = gui_tx.send(GuiEvent::CallStateChanged("free".into()));
                            let _ = gui_tx.send(GuiEvent::StatusUpdate(
                                "Đã kết thúc cuộc gọi".into(),
                            ));
                            egui_ctx.request_repaint();
                        }
                        WorkerCmd::CheckFriendStatus { user_id } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let status_res =
                                    crate::chat::check_friendship_status(&s, &user_id).await;
                                let online_opt =
                                    crate::chat::get_user_online_status(&s, &user_id).await.ok();
                                if let Ok(status) = status_res {
                                    let _ = gui_tx_clone.send(GuiEvent::FriendStatusChecked {
                                        user_id,
                                        status,
                                        online: online_opt,
                                    });
                                    ctx_clone.request_repaint();
                                }
                            });
                        }
                        WorkerCmd::AcceptFriend { user_id, user_name } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                match crate::chat::accept_friend_request(&s, &user_id).await {
                                    Ok(_) => {
                                        let _ = gui_tx_clone.send(GuiEvent::FriendStatusChecked {
                                            user_id,
                                            status: crate::chat::FriendshipStatus::Friend,
                                            online: None,
                                        });
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "Đã đồng ý kết bạn với {user_name}!"
                                        )));
                                    }
                                    Err(e) => {
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "Không thể đồng ý kết bạn với {user_name}: {e}"
                                        )));
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::RejectFriend { user_id, user_name } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                if crate::chat::reject_friend_request(&s, &user_id)
                                    .await
                                    .is_ok()
                                {
                                    let _ = gui_tx_clone.send(GuiEvent::FriendStatusChecked {
                                        user_id,
                                        status: crate::chat::FriendshipStatus::Stranger,
                                        online: None,
                                    });
                                    let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                        "Đã từ chối lời mời kết bạn của {user_name}."
                                    )));
                                    ctx_clone.request_repaint();
                                }
                            });
                        }
                        WorkerCmd::UndoFriend { user_id, user_name } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                if crate::chat::undo_friend_request(&s, &user_id).await.is_ok() {
                                    let _ = gui_tx_clone.send(GuiEvent::FriendStatusChecked {
                                        user_id,
                                        status: crate::chat::FriendshipStatus::Stranger,
                                        online: None,
                                    });
                                    let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                        "Đã hủy lời mời kết bạn tới {user_name}."
                                    )));
                                    ctx_clone.request_repaint();
                                }
                            });
                        }
                        WorkerCmd::AddFriend { user_id, user_name } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                // Check if there is an incoming request we can accept directly
                                if crate::chat::accept_friend_request(&s, &user_id).await.is_ok() {
                                    let _ = gui_tx_clone.send(GuiEvent::FriendStatusChecked {
                                        user_id,
                                        status: crate::chat::FriendshipStatus::Friend,
                                        online: None,
                                    });
                                    let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                        "Đã đồng ý kết bạn với {user_name}! Bạn có thể gọi lại ngay."
                                    )));
                                    ctx_clone.request_repaint();
                                    return;
                                }
                                match crate::chat::send_friend_request(
                                    &s,
                                    &user_id,
                                    "Xin chào, kết bạn với mình nhé!",
                                )
                                .await
                                {
                                    Ok(_) => {
                                        let _ = gui_tx_clone.send(GuiEvent::FriendStatusChecked {
                                            user_id: user_id.clone(),
                                            status: crate::chat::FriendshipStatus::OutgoingPending,
                                            online: None,
                                        });
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "Đã gửi lời mời kết bạn tới {user_name} ({user_id})."
                                        )));
                                    }
                                    Err(e) => {
                                        let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(format!(
                                            "Không thể gửi lời mời kết bạn tới {user_name}: {e}"
                                        )));
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::LookupPhone { phone } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                match crate::chat::find_user_by_phone(&s, &phone).await {
                                    Ok(profile) => {
                                        let _ = gui_tx_clone
                                            .send(GuiEvent::PhoneLookupSuccess(profile));
                                    }
                                    Err(e) => {
                                        crate::logger::log_error(
                                            "gui::lookup_phone",
                                            &e.to_string(),
                                        );
                                        let _ = gui_tx_clone
                                            .send(GuiEvent::PhoneLookupFailed(e.to_string()));
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::LoginPassword { phone, pass } => {
                            let session_clone = session.clone();
                            let poll_clone = poll_state.clone();
                            let poll_generation = poll_generation.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let _ = gui_tx_clone.send(GuiEvent::StatusUpdate(
                                    "Đang xác thực tài khoản…".into(),
                                ));
                                ctx_clone.request_repaint();
                                let mut s = session_clone.lock().await;
                                advance_poll_generation(&poll_generation);
                                let login = crate::login::login_by_password(&mut s, &phone, &pass, "VN").await;
                                match login {
                                    Ok(_) => {
                                        let info = crate::login::get_login_info(&s).await;
                                        advance_poll_generation(&poll_generation);
                                        if let Ok(info) = info {
                                            if let Some(cfg) = info.get("data") {
                                                crate::login::apply_login_config(&mut s, cfg);
                                                s.apply_server_info(cfg);
                                                let _ = s.save_to_file("session.json");

                                                let uin = s
                                                    .login
                                                    .as_ref()
                                                    .map(|l| l.uin.clone())
                                                    .unwrap_or_default();
                                                if let Err(e) = crate::account_manager::activate_account_data(&uin) {
                                                    crate::logger::log_error("account::activate", &e.to_string());
                                                }
                                                let _ = crate::account_manager::save_current_account_snapshot(&uin);
                                                let name = s
                                                    .login
                                                    .as_ref()
                                                    .map(|l| l.display_name.clone())
                                                    .unwrap_or_default();
                                                let u_ava = s
                                                    .login
                                                    .as_ref()
                                                    .map(|l| l.avatar.clone())
                                                    .filter(|a| !a.is_empty());
                                                let _ = gui_tx_clone.send(GuiEvent::LoginSuccess {
                                                    name,
                                                    uin: uin.clone(),
                                                    avatar: u_ava,
                                                });
                                                ctx_clone.request_repaint();

                                                let mut list: Vec<Thread> = Vec::new();
                                                if let Ok(friends) =
                                                    crate::chat::get_friends(&s).await
                                                {
                                                    for ft in friends {
                                                        if let Some(ex) =
                                                            list.iter_mut().find(|t| t.id == ft.id)
                                                        {
                                                            if !ft.name.is_empty() {
                                                                ex.name = ft.name;
                                                            }
                                                            if crate::chat::should_replace_avatar(
                                                                ex.avatar.as_deref(),
                                                                ft.avatar.as_deref(),
                                                            ) {
                                                                ex.avatar = ft.avatar;
                                                            }
                                                            ex.is_group =
                                                                ex.is_group || ft.is_group;
                                                        } else {
                                                            list.push(ft);
                                                        }
                                                    }
                                                }
                                                if let Ok(groups) =
                                                    crate::chat::get_groups(&s).await
                                                {
                                                    for gt in groups {
                                                        if let Some(ex) =
                                                            list.iter_mut().find(|t| t.id == gt.id)
                                                        {
                                                            if !gt.name.is_empty() {
                                                                ex.name = gt.name;
                                                            }
                                                            if crate::chat::should_replace_avatar(
                                                                ex.avatar.as_deref(),
                                                                gt.avatar.as_deref(),
                                                            ) {
                                                                ex.avatar = gt.avatar;
                                                            }
                                                            ex.presentation.posting_restricted = gt.presentation.posting_restricted;
                                                            ex.is_group = true;
                                                        } else {
                                                            list.push(gt);
                                                        }
                                                    }
                                                }
                                                let db_path = crate::store::db_path_for_account(&uin);
                                                crate::store::load_history_into_threads(
                                                    &db_path,
                                                    &mut list,
                                                    crate::store::MessageStore::PRELOAD_MSGS,
                                                );
                                                {
                                                    let mut ps = poll_clone.lock().await;
                                                    *ps = crate::chat::PollState::default();
                                                    let _ = crate::chat::poll_new_messages(
                                                        &s, &mut ps, &mut list,
                                                    )
                                                    .await;
                                                }
                                                if let Ok(mute_resp) =
                                                    crate::chat::get_mute_conversations(&s).await
                                                {
                                                    crate::unread::merge_mute_metadata(
                                                        &mut list, &mute_resp,
                                                    );
                                                    if let Some(mute_map) =
                                                        crate::unread::extract_mute_map(
                                                            &mute_resp,
                                                            crate::login::unix_now() as i64,
                                                        )
                                                    {
                                                        let _ = gui_tx_clone.send(
                                                            GuiEvent::MuteSnapshotSynced(mute_map),
                                                        );
                                                    }
                                                }
                                                let sync_snapshot = list.clone();
                                                let _ = gui_tx_clone
                                                    .send(GuiEvent::ThreadsUpdated(list));
                                                ctx_clone.request_repaint();

                                                // Automatically sync Cloud/Group messages upon password login without spamming phone prompt (#73)
                                                if let Some(wtx) = WORKER_TX.get() {
                                                    let _ = wtx.send(
                                                        WorkerCmd::RefreshMessageQueues { threads: sync_snapshot },
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let _ = gui_tx_clone
                                            .send(GuiEvent::LoginFailed(e.to_string()));
                                        ctx_clone.request_repaint();
                                    }
                                }
                            });
                        }
                        WorkerCmd::CheckForUpdates { silent } => {
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let res = fetch_github_latest_release().await;
                                let _ = gui_tx_clone.send(GuiEvent::UpdateCheckFinished {
                                    result: res,
                                    silent,
                                });
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::DownloadAndApplyUpdate {
                            download_url,
                            asset_name,
                            target_version,
                            expected_sha256,
                        } => {
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let _ = gui_tx_clone.send(GuiEvent::UpdateDownloadStatus {
                                    downloading: true,
                                    message: format!(
                                        "Đang tải bản cập nhật v{target_version}…"
                                    ),
                                });
                                ctx_clone.request_repaint();
                                match apply_github_release_update(
                                    &download_url,
                                    &asset_name,
                                    &target_version,
                                    expected_sha256.as_deref(),
                                    &gui_tx_clone,
                                    &ctx_clone,
                                )
                                .await
                                {
                                    Ok(restarting) => {
                                        if restarting {
                                            let _ = gui_tx_clone.send(
                                                GuiEvent::UpdateAppliedAndRestarting(target_version),
                                            );
                                        } else {
                                            let _ = gui_tx_clone.send(
                                                GuiEvent::UpdateDownloadStatus {
                                                    downloading: false,
                                                    message: format!(
                                                        "Đã tải và mở bộ cài đặt {asset_name} (v{target_version})"
                                                    ),
                                                },
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        let _ = gui_tx_clone.send(GuiEvent::UpdateDownloadStatus {
                                            downloading: false,
                                            message: format!("Lỗi cập nhật: {e}"),
                                        });
                                    }
                                }
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::FetchGroupMembers { group_id } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                if let Ok((info_map, _)) = crate::chat::fetch_group_profiles(&s, &[group_id.clone()]).await {
                                    if let Some(info) = info_map.get(&group_id) {
                                        let mut member_uids: Vec<String> = Vec::new();
                                        if let Some(arr) = info.get("memVerList").and_then(serde_json::Value::as_array) {
                                            for item in arr {
                                                if let Some(s_val) = item.as_str() {
                                                    let uid = s_val.split('_').next().unwrap_or("").trim();
                                                    if !uid.is_empty() && !member_uids.contains(&uid.to_string()) {
                                                        member_uids.push(uid.to_string());
                                                    }
                                                }
                                            }
                                        }
                                        let creator_id = crate::chat::pick_str(info, &["creatorId"]).unwrap_or_default();
                                        if !creator_id.is_empty() && !member_uids.contains(&creator_id) {
                                            member_uids.insert(0, creator_id.clone());
                                        }
                                        let mut admin_ids: Vec<String> = Vec::new();
                                        if let Some(admins) = info.get("adminIds").and_then(serde_json::Value::as_array) {
                                            for a in admins {
                                                if let Some(aid) = a.as_str() {
                                                    let aid_s = aid.to_string();
                                                    if !member_uids.contains(&aid_s) {
                                                        member_uids.push(aid_s.clone());
                                                    }
                                                    if !admin_ids.contains(&aid_s) {
                                                        admin_ids.push(aid_s);
                                                    }
                                                }
                                            }
                                        }
                                        let mut profiles = serde_json::Map::new();
                                        for chunk in member_uids.chunks(50) {
                                            if let Ok(mp) = crate::chat::get_mini_profiles(&s, chunk).await {
                                                for (k, v) in mp {
                                                    profiles.insert(k, v);
                                                }
                                            }
                                        }
                                        let mut members = Vec::new();
                                        for uid in &member_uids {
                                            let mut m_name = "Thành viên".to_string();
                                            let mut m_ava = None;
                                            if let Some(p) = profiles.get(uid) {
                                                if let Some(n) = crate::chat::pick_str(p, &["displayName", "zaloName", "name"]) {
                                                    if !n.is_empty() {
                                                        m_name = n;
                                                    }
                                                }
                                                if let Some(a) = crate::chat::pick_str(p, &["avatar", "avt"]) {
                                                    if !a.is_empty() && !a.ends_with("/default") {
                                                        m_ava = Some(a);
                                                    }
                                                }
                                            }
                                            let role = if uid == &creator_id {
                                                "creator".to_string()
                                            } else if admin_ids.contains(uid) {
                                                "admin".to_string()
                                            } else {
                                                "member".to_string()
                                            };
                                            members.push(crate::chat::SidebarMember {
                                                id: uid.clone(),
                                                name: m_name,
                                                avatar: m_ava,
                                                role,
                                            });
                                        }
                                        let _ = gui_tx_clone.send(GuiEvent::GroupMembersLoaded {
                                            group_id,
                                            members,
                                        });
                                        ctx_clone.request_repaint();
                                        return;
                                    }
                                }
                                let _ = gui_tx_clone.send(GuiEvent::GroupMembersLoaded {
                                    group_id,
                                    members: Vec::new(),
                                });
                                ctx_clone.request_repaint();
                            });
                        }
                        command @ (WorkerCmd::SyncPhoneMessages { .. } | WorkerCmd::RefreshMessageQueues { .. }) => {
                            let (mut threads, platform) = match command {
                                WorkerCmd::SyncPhoneMessages { threads, platform } => (threads, Some(platform)),
                                WorkerCmd::RefreshMessageQueues { threads } => (threads, None),
                                _ => unreachable!(),
                            };
                            let send_phone_prompt = platform.is_some();
                            let running = if send_phone_prompt { &crate::history_sync::PHONE_SYNC_RUNNING } else { &crate::history_sync::REFRESH_RUNNING };
                            let cancel = if send_phone_prompt { &crate::history_sync::PHONE_SYNC_CANCEL } else { &crate::history_sync::REFRESH_CANCEL };
                            if send_phone_prompt {
                                crate::history_sync::REFRESH_CANCEL.store(true, std::sync::atomic::Ordering::Relaxed);
                            } else if crate::history_sync::PHONE_SYNC_RUNNING.load(std::sync::atomic::Ordering::Relaxed) {
                                continue;
                            }
                            use std::sync::atomic::Ordering;
                            if running.swap(true, Ordering::SeqCst)
                            {
                                let _ = gui_tx.send(GuiEvent::StatusUpdate(
                                    "⚠️ Tiến trình đồng bộ tin nhắn đang chạy ngầm, vui lòng chờ trong giây lát…".into(),
                                ));
                                egui_ctx.request_repaint();
                                continue;
                            }
                            cancel.store(false, Ordering::Relaxed);
                            let account_uin = session.lock().await.login.as_ref().map(|l| l.uin.clone()).unwrap_or_default();
                            let session_clone = session.clone();
                            let poll_clone = poll_state.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let _phone_sync = crate::history_sync::PHONE_SYNC_LOCK.lock().await;
                                let s = session_clone.lock().await.clone();
                                if s.login.as_ref().map(|l| l.uin.as_str()) != Some(account_uin.as_str()) {
                                    running.store(false, Ordering::SeqCst);
                                    return;
                                }
                                let gui_tx_prog = gui_tx_clone.clone();
                                let ctx_prog = ctx_clone.clone();
                                let progress_account = account_uin.clone();
                                let res = crate::history_sync::sync_phone_messages_opts(
                                    &s,
                                    &poll_clone,
                                    &mut threads,
                                    platform,
                                    move |progress, status| {
                                        if !send_phone_prompt { return; }
                                        let _ = gui_tx_prog.send(GuiEvent::PhoneSyncProgress {
                                            account_uin: progress_account.clone(),
                                            progress,
                                            status,
                                        });
                                        ctx_prog.request_repaint();
                                    },
                                )
                                .await;
                                if !send_phone_prompt {
                                    if session_clone.lock().await.login.as_ref().map(|l| l.uin.as_str()) == Some(account_uin.as_str()) {
                                        let _ = gui_tx_clone.send(GuiEvent::MessageQueuesRefreshed { account_uin: account_uin.clone(), threads });
                                        ctx_clone.request_repaint();
                                    }
                                    running.store(false, Ordering::SeqCst);
                                    return;
                                }
                                let cancelled = cancel.load(Ordering::Relaxed);
                                let phone_imported = if send_phone_prompt { crate::history_sync::last_phone_imported_count() } else { 0 };
                                let phone_complete = send_phone_prompt && crate::history_sync::last_phone_sync_complete();
                                match res {
                                    Ok((added, active_threads)) => {
                                        let _ = gui_tx_clone.send(GuiEvent::PhoneSyncFinished {
                                            account_uin: account_uin.clone(),
                                            threads,
                                            added,
                                            active_threads,
                                            phone_imported,
                                            phone_complete,
                                            cancelled,
                                            error: None,
                                        });
                                    }
                                    Err(e) => {
                                        let _ = gui_tx_clone.send(GuiEvent::PhoneSyncFinished {
                                            account_uin: account_uin.clone(),
                                            threads,
                                            added: 0,
                                            active_threads: 0,
                                            phone_imported,
                                            phone_complete,
                                            cancelled,
                                            error: if cancelled { None } else { Some(crate::history_sync::phone_sync_error_message(&e)) },
                                        });
                                    }
                                }
                                ctx_clone.request_repaint();
                                running.store(false, Ordering::SeqCst);
                            });
                        }
                        WorkerCmd::Productivity { account, action } => {
                            let active = session.lock().await.login.as_ref().map(|l|l.uin.clone()).unwrap_or_default();
                            if active != account || active.is_empty() {
                                let _=gui_tx.send(GuiEvent::Productivity{account,update:Err("Tài khoản đã thay đổi. Hãy thực hiện lại thao tác trong tài khoản hiện tại.".into())});
                                egui_ctx.request_repaint();continue;
                            }
                            if matches!(&action,crate::productivity::Action::Quit(_)){if let Some((_,control,cancelled))=voice_control.take(){cancelled.store(true,std::sync::atomic::Ordering::Relaxed);let _=control.send(true);}}

                            let update = tokio::task::spawn_blocking(move || -> anyhow::Result<crate::productivity::Update> {
                                let store = crate::store::MessageStore::open("zalo_history.db")?;
                                crate::productivity::execute(store, action, crate::login::unix_now() as i64)
                            }).await;
                            let result = match update { Ok(Ok(result)) => Ok(result), _ => Err("Không thể cập nhật dữ liệu cá nhân trên máy. Hãy thử lại.".to_owned()) };
                            let _ = gui_tx.send(GuiEvent::Productivity { account, update:result });
                            egui_ctx.request_repaint();
                        }
                        WorkerCmd::FetchPoll { account, poll_id, generation, .. } => {
                            let command = PollCommand { account, poll_id, option_ids: None, generation };
                            spawn_poll_command(&session, &poll_queue, &poll_generation, &gui_tx, command);
                        }
                        WorkerCmd::VotePoll { account, poll_id, option_ids, generation, .. } => {
                            let command = PollCommand { account, poll_id, option_ids: Some(option_ids), generation };
                            spawn_poll_command(&session, &poll_queue, &poll_generation, &gui_tx, command);
                        }
                        WorkerCmd::FetchDocPreview { account, generation, url, name } => {
                            let snapshot=session.lock().await.clone();
                            if snapshot.login.as_ref().map(|l|l.uin.as_str())!=Some(account.as_str()) {
                                let _=gui_tx.send(GuiEvent::DocPreview{account,generation,result:Err("Tài khoản đã thay đổi. Hãy mở lại tài liệu.".into())});
                                egui_ctx.request_repaint();continue;
                            }
                            let tx=gui_tx.clone(); let ctx=egui_ctx.clone();
                            tokio::spawn(async move {
                                let result=fetch_document_preview(&snapshot,&url,&name).await.map_err(|_|"Không thể xem trước tài liệu. Kiểm tra định dạng, giới hạn dung lượng hoặc tải để mở bên ngoài.".to_owned());
                                let _=tx.send(GuiEvent::DocPreview{account,generation,result});ctx.request_repaint();
                            });
                        }
                        WorkerCmd::VoiceStart {account,generation,thread,is_group} => {
                            if session.lock().await.login.as_ref().map(|l|l.uin.as_str())!=Some(account.as_str()){
                                let _=gui_tx.send(GuiEvent::VoiceFinished{account,generation,thread,result:Err("Tài khoản đã thay đổi. Hãy bắt đầu ghi âm lại.".into())});
                                egui_ctx.request_repaint();continue;
                            }
                            if let Some((_,control,cancelled))=voice_control.take(){cancelled.store(true,std::sync::atomic::Ordering::Relaxed);let _=control.send(true);}
                            let (control,cancelled)=spawn_voice_capture(account,generation,thread,is_group,gui_tx.clone(),egui_ctx.clone());
                            voice_control=Some((generation,control,cancelled));
                        }
                        WorkerCmd::VoiceStop {generation,cancel} => {
                            if let Some((current,control,cancelled))=voice_control.as_ref(){if *current==generation {if cancel{cancelled.store(true,std::sync::atomic::Ordering::Relaxed);}let _=control.send(cancel);}}
                        }
                        WorkerCmd::VoiceEncoded {account,generation,thread,is_group,result} => {
                            let mut result=result;
                            if voice_control.as_ref().is_some_and(|(current,_,_)|*current==generation){if let Some((_,_,cancelled))=voice_control.take(){if cancelled.load(std::sync::atomic::Ordering::Relaxed){result=Err("Đã hủy ghi âm.".into());}}}
                            let snapshot=session.lock().await.clone();
                            if snapshot.login.as_ref().map(|l|l.uin.as_str())!=Some(account.as_str()){continue;}
                            let result=match result {
                                Ok(record)=>crate::chat::send_voice_message(&snapshot,&thread,is_group,&record.path,&record.mime,record.duration_secs).await.map(|(_,message)|message).map_err(|_|"Chưa xác nhận gửi được ghi âm. Kiểm tra hội thoại trước khi thử lại.".to_owned()),
                                Err(error)=>Err(error),
                            };
                            let _=gui_tx.send(GuiEvent::VoiceFinished{account,generation,thread,result});egui_ctx.request_repaint();
                        }
                        WorkerCmd::SetProxy { proxy_url } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let res = {
                                    let mut s = session_clone.lock().await;
                                    let r = s
                                        .set_proxy(Some(&proxy_url))
                                        .map_err(|e| e.to_string());
                                    if r.is_ok() {
                                        let _ = s.save_to_file("session.json");
                                    }
                                    r
                                };
                                let _ = gui_tx_clone.send(GuiEvent::ProxyUpdated(res));
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::DownloadFile {
                            url,
                            file_name,
                            download_dir,
                        } => {
                            let session_clone = session.clone();
                            let gui_tx_clone = gui_tx.clone();
                            let ctx_clone = egui_ctx.clone();
                            tokio::spawn(async move {
                                let client = { session_clone.lock().await.clone().http };
                                let res = download_attachment_with_original_name(
                                    &client,
                                    &url,
                                    &file_name,
                                    &download_dir,
                                )
                                .await;
                                let _ = gui_tx_clone.send(GuiEvent::FileDownloaded(res));
                                ctx_clone.request_repaint();
                            });
                        }
                        WorkerCmd::SendTyping {
                            thread_id,
                            is_group,
                        } => {
                            let session_clone = session.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let _ = crate::chat::send_typing_indicator(&s, &thread_id, is_group).await;
                            });
                        }
                        WorkerCmd::SendSeen {
                            thread_id,
                            is_group,
                            last_msg_id,
                        } => {
                            let session_clone = session.clone();
                            tokio::spawn(async move {
                                let s = session_clone.lock().await.clone();
                                let _ = crate::chat::send_seen_v2(&s, &thread_id, is_group, &last_msg_id).await;
                            });
                        }
                    }
                }
            });
        })
        .expect("Failed to spawn zalo-worker thread");
}

/// Releases of this fork. Update checks only talk to GitHub (`api.github.com`,
/// `github.com/.../releases`) and never carry Zalo cookies; every download is size-limited and
/// verified against the release's SHA-256 digest before install.
const GITHUB_REPO: &str = "minhkhang1008/zalo-next";

pub(crate) async fn fetch_github_latest_release() -> Result<UpdateCheckResult, String> {
    let current_ver = env!("CARGO_PKG_VERSION").to_string();
    let client = crate::updater::github_update_client(GITHUB_REPO)
        .user_agent(format!("zalo-next/{current_ver}"))
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|e| format!("Khởi tạo HTTP client thất bại: {e}"))?;

    let latest_url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let mut rel_json: Option<serde_json::Value> = None;

    if let Ok(resp) = client
        .get(&latest_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                rel_json = Some(v);
            }
        }
    }

    if rel_json.is_none() {
        let list_url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases?per_page=5");
        if let Ok(resp) = client
            .get(&list_url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(arr) = resp.json::<serde_json::Value>().await {
                    if let Some(first) = arr
                        .as_array()
                        .and_then(|a| {
                            a.iter()
                                .find(|r| {
                                    !r.get("draft").and_then(|v| v.as_bool()).unwrap_or(false)
                                })
                                .or_else(|| a.first())
                        })
                        .cloned()
                    {
                        rel_json = Some(first);
                    }
                }
            }
        }
    }

    // Rate-limit resilient fallback: fetch static `latest.json` directly from GitHub Release CDN
    if rel_json.is_none() {
        let cdn_manifest_url =
            format!("https://github.com/{GITHUB_REPO}/releases/latest/download/latest.json");
        if let Ok(resp) = client.get(&cdn_manifest_url).send().await {
            if resp.status().is_success() {
                if let Ok(manifest) = resp.json::<serde_json::Value>().await {
                    rel_json = Some(manifest);
                }
            }
        }
    }

    let Some(rel) = rel_json else {
        return Err("Không truy cập được máy chủ GitHub Releases API".into());
    };

    let mut meta = crate::updater::parse_github_release_value(
        &rel,
        &current_ver,
        GITHUB_REPO,
        std::env::consts::OS,
        std::env::consts::ARCH,
    );

    if meta.expected_sha256.is_none() {
        if let (Some(s_url), Some(b_name)) = (&meta.sha256sums_url, &meta.asset_name) {
            if let Ok(resp) = client
                .get(s_url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
            {
                if let Ok(txt) = resp.text().await {
                    meta.expected_sha256 = crate::updater::parse_sha256sums_for_asset(&txt, b_name);
                }
            }
        }
    }

    Ok(UpdateCheckResult {
        current_version: meta.current_version,
        latest_version: meta.latest_version,
        has_update: meta.has_update,
        release_title: meta.release_title,
        release_notes: meta.release_notes,
        html_url: meta.html_url,
        download_url: meta.download_url,
        asset_name: meta.asset_name,
        asset_size: meta.asset_size,
        expected_sha256: meta.expected_sha256,
        published_at: meta.published_at,
    })
}

/// Stream-download the release asset from GitHub Releases with real-time progress reporting,
/// verify its SHA-256 checksum and PE/ELF magic where applicable via [`crate::updater`],
/// and atomically replace the running executable in-place with automatic rollback on failure.
async fn apply_github_release_update(
    download_url: &str,
    asset_name: &str,
    target_version: &str,
    expected_sha256: Option<&str>,
    gui_tx: &GuiTx,
    ctx: &egui::Context,
) -> Result<bool, String> {
    if !crate::updater::is_github_release_asset_url(download_url, GITHUB_REPO) {
        return Err(
            "Đường dẫn gói cập nhật không hợp lệ. Hãy tải thủ công từ trang phát hành trên GitHub."
                .into(),
        );
    }

    let client = crate::updater::github_update_client(GITHUB_REPO)
        .user_agent(format!("zalo-next/{target_version}"))
        .timeout(Duration::from_secs(240))
        .build()
        .map_err(|e| format!("Khởi tạo HTTP client thất bại: {e}"))?;

    let req = crate::updater::DownloadRequest {
        url: download_url,
        asset_name,
        expected_sha256,
        expected_size: None,
        limits: crate::updater::DownloadLimits::default(),
    };

    let release = crate::updater::download_release(&client, &req, |downloaded, total| {
        let _ = gui_tx.send(GuiEvent::UpdateDownloadProgress { downloaded, total });
        ctx.request_repaint();
    })
    .await?;

    let name_low = asset_name.to_lowercase();

    // macOS: replace the running .app and relaunch.
    #[cfg(target_os = "macos")]
    if name_low.ends_with(".dmg") {
        if let Some(bundle) = crate::updater::current_app_bundle() {
            let dmg = release.clone();
            let target = bundle.clone();
            let installed = tokio::task::spawn_blocking(move || {
                crate::updater::install_macos_app_from_dmg(&dmg, &target)
            })
            .await
            .map_err(|e| format!("Lỗi tác vụ cài đặt: {e}"))?;
            match installed {
                Ok(()) => {
                    crate::updater::relaunch_app_bundle_after_exit(&bundle)?;
                    return Ok(true);
                }
                Err(e) => {
                    // Fall through to the manual path (open the DMG) below.
                    crate::logger::log_error("updater::install_macos_app", &e);
                }
            }
        }
    }

    #[cfg(windows)]
    if crate::updater::try_install_windows_portable_release(
        &name_low,
        &release,
        std::env::current_exe,
        |installed_path| {
            let (_, args) = crate::updater::take_wait_for_pid(std::env::args().skip(1).collect());
            std::process::Command::new(installed_path)
                .arg("--wait-for-pid")
                .arg(std::process::id().to_string())
                .args(args)
                .spawn()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
    )? {
        return Ok(true);
    }

    #[cfg(target_os = "linux")]
    if name_low.ends_with(".appimage") {
        if let Ok(exe_path) = std::env::current_exe() {
            let (_, args) =
                crate::updater::take_wait_for_pid(std::env::args().skip(1).collect());
            if crate::updater::install_release(
                &release,
                &crate::updater::InstallTarget::Portable(exe_path),
                |installed_path| {
                    // The new process waits for this one's GUI lock (single_instance).
                    std::process::Command::new(installed_path)
                        .arg("--wait-for-pid")
                        .arg(std::process::id().to_string())
                        .args(&args)
                        .spawn()
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                },
            )
            .is_ok()
            {
                return Ok(true);
            }
        }
    }

    // Fallback / Installer mode: save to Downloads (or current directory) via transactional installer
    let dl_dir = std::env::var("USERPROFILE")
        .ok()
        .map(|p| std::path::PathBuf::from(p).join("Downloads"))
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join("Downloads"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let _ = std::fs::create_dir_all(&dl_dir);
    let dest_path = dl_dir.join(asset_name);

    crate::updater::install_release(
        &release,
        &crate::updater::InstallTarget::Installer(dest_path),
        |installer_path| {
            #[cfg(target_os = "windows")]
            {
                std::process::Command::new(installer_path)
                    .spawn()
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            #[cfg(target_os = "macos")]
            {
                std::process::Command::new("open")
                    .arg(installer_path)
                    .spawn()
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                std::process::Command::new("xdg-open")
                    .arg(installer_path)
                    .spawn()
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        },
    )?;
    Ok(false)
}

/// Download a Zalo file attachment (`share.file` / `chat.file` / `/renewMedia`) and save it
/// inside `download_dir` using the sender's original filename instead of `renewMedia.doc` (#74).
async fn download_attachment_with_original_name(
    client: &reqwest::Client,
    url: &str,
    file_name: &str,
    download_dir: &str,
) -> Result<(String, String), String> {
    let clean_name = crate::chat::resolve_attachment_file_name(Some(file_name), file_name, url);
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Không thể tải tập tin: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Máy chủ trả về mã HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Lỗi đọc dữ liệu tập tin: {e}"))?;

    let target_dir = if !download_dir.trim().is_empty() {
        std::path::PathBuf::from(download_dir.trim())
    } else {
        std::env::var("USERPROFILE")
            .ok()
            .map(|p| {
                std::path::PathBuf::from(p)
                    .join("Downloads")
                    .join("Zalo Received Files")
            })
            .or_else(|| {
                std::env::var("HOME").ok().map(|h| {
                    std::path::PathBuf::from(h)
                        .join("Downloads")
                        .join("Zalo Received Files")
                })
            })
            .unwrap_or_else(|| std::path::PathBuf::from("Downloads"))
    };

    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&target_dir)
            .map_err(|e| format!("Không thể tạo thư mục lưu file: {e}"))?;

        let (stem, ext) = match clean_name.rfind('.') {
            Some(dot_idx) if dot_idx > 0 && dot_idx + 1 < clean_name.len() => {
                (&clean_name[..dot_idx], Some(&clean_name[dot_idx + 1..]))
            }
            _ => (clean_name.as_str(), None),
        };

        let mut dest_path = target_dir.join(&clean_name);
        let mut final_name = clean_name.clone();
        let mut suffix_idx = 1_u32;
        while dest_path.exists() {
            final_name = match ext {
                Some(e) => format!("{stem} ({suffix_idx}).{e}"),
                None => format!("{stem} ({suffix_idx})"),
            };
            dest_path = target_dir.join(&final_name);
            suffix_idx += 1;
        }

        std::fs::write(&dest_path, &bytes).map_err(|e| format!("Không thể ghi tập tin: {e}"))?;
        Ok((final_name, dest_path.display().to_string()))
    })
    .await
    .map_err(|e| format!("Lỗi tác vụ lưu file: {e}"))?
}

pub(crate) fn capture_screen_interactive(gui_tx: crate::gui::GuiTx, ctx: egui::Context) {
    #[cfg(target_os = "windows")]
    {
        // Shell launch instead of `powershell`, which flashed a console window. Snipping Tool
        // leaves the snip on the clipboard; Windows cannot tell us which copy came from it, so the
        // user pastes it into the conversation they mean (no automatic attach).
        let text = if crate::gui::messages::windows_shell_open("ms-screenclip:") {
            "Đã mở công cụ chụp ảnh màn hình — sau khi chụp hãy nhấn Ctrl+V để dán"
        } else {
            "Không mở được công cụ chụp màn hình của Windows (Snipping Tool)"
        };
        let _ = gui_tx.send(crate::gui::GuiEvent::StatusUpdate(text.into()));
        ctx.request_repaint();
    }

    #[cfg(not(target_os = "windows"))]
    {
        let now = crate::login::unix_now();
        let temp_path = std::env::temp_dir().join(format!("ctdozlite_screenshot_{now}.png"));

        #[cfg(target_os = "macos")]
        let status = std::process::Command::new("/usr/sbin/screencapture")
            .args(["-i", "-x", temp_path.to_str().unwrap_or("")])
            .status();

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let status = {
            let temp_str = temp_path.to_str().unwrap_or("");
            std::process::Command::new("flameshot")
                .args(["gui", "-r", "-p", temp_str])
                .status()
                .or_else(|_| {
                    std::process::Command::new("gnome-screenshot")
                        .args(["-a", "-f", temp_str])
                        .status()
                })
                .or_else(|_| {
                    std::process::Command::new("spectacle")
                        .args(["-b", "-r", "-o", temp_str])
                        .status()
                })
                .or_else(|_| {
                    std::process::Command::new("scrot")
                        .args(["-s", temp_str])
                        .status()
                })
        };

        if let Ok(st) = status {
            if st.success() && temp_path.exists() {
                if let Ok(meta) = std::fs::metadata(&temp_path) {
                    if meta.len() > 0 {
                        if let Ok(bytes) = std::fs::read(&temp_path) {
                            if let Ok(dyn_img) = image::load_from_memory(&bytes) {
                                if let Some((final_path, w, h, col_img)) =
                                    crate::gui::messages::finalize_clipboard_rgba_image(
                                        dyn_img.to_rgba8(),
                                    )
                                {
                                    let _ = std::fs::remove_file(&temp_path);
                                    let _ = gui_tx.send(crate::gui::GuiEvent::ScreenshotCaptured {
                                        path: final_path,
                                        width: w,
                                        height: h,
                                        image: col_img,
                                    });
                                    ctx.request_repaint();
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }
        let _ = std::fs::remove_file(&temp_path);
        let _ = gui_tx.send(crate::gui::GuiEvent::StatusUpdate(
            "Đã huỷ chụp màn hình".into(),
        ));
        ctx.request_repaint();
    }
}

/// Persist the question, ask the Assistant and report the answer. Runs on the worker. Nothing is
/// sent to a Zalo endpoint (INV-5); `zalo_http` (the session client) is used only to download
/// attachments from Zalo media CDNs (`ai::media_needs_session`), and only their decoded content
/// goes to the AI host, over the Assistant's own client (INV-6).
async fn run_ai_ask(
    mut req: crate::ai::AskRequest,
    tx: GuiTx,
    db_path: String,
    zalo_http: Option<reqwest::Client>,
) {
    use crate::ai::{AiMessage, AiRole, Turn};
    let fail = |tx: &GuiTx, thread_id: &str, error: String| {
        let _ = tx.send(GuiEvent::AiFailed {
            thread_id: thread_id.to_string(),
            error,
        });
    };
    // 1. Persist the question and load local context on the blocking pool.
    let (user_msg, thread_id, exclude) = (
        req.user_msg.clone(),
        req.thread_id.clone(),
        req.user_msg_id.clone(),
    );
    let read_zalo_history = req.share && req.digest.is_none();
    let (thread_name, is_group) = (req.thread_name.clone(), req.is_group);
    let db_for_load = db_path.clone();
    let loaded = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let store = crate::store::MessageStore::open(&db_for_load)?;
        if let Some(m) = &user_msg {
            crate::ai::insert(&store.conn, m)?;
        }
        let dialogue = crate::ai::load_thread(&store.conn, &thread_id, 24)?;
        let stored = if read_zalo_history {
            match store.messages_for(&thread_id, 600) {
                Ok(rows) => crate::ai::stored_transcript(&rows, &thread_name, is_group),
                Err(err) => {
                    logger::log_error("AI conversation history", &err.to_string());
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        Ok((dialogue, stored))
    })
    .await;
    let (rows, stored) = loaded.ok().and_then(Result::ok).unwrap_or_default();
    if read_zalo_history {
        req.transcript = crate::ai::merge_transcripts(std::mem::take(&mut req.transcript), stored);
    }
    let history: Vec<Turn> = rows
        .iter()
        .filter(|m| m.id != exclude)
        .filter_map(|m| match m.role {
            AiRole::User => Some(Turn::new("user", m.text.clone())),
            AiRole::Assistant => Some(Turn::new("assistant", m.text.clone())),
            AiRole::Error => None,
        })
        .collect();
    // 2. Configuration (env overrides settings; key from env or the private key file).
    let (base_url, model) = (req.base_url.clone(), req.model.clone());
    let cfg =
        match tokio::task::spawn_blocking(move || crate::ai::AiConfig::resolve(&base_url, &model))
            .await
        {
            Ok(Ok(cfg)) => cfg,
            Ok(Err(e)) => return fail(&tx, &req.thread_id, e.to_string()),
            Err(_) => {
                return fail(
                    &tx,
                    &req.thread_id,
                    "Không đọc được cấu hình trợ lý AI".into(),
                )
            }
        };
    // 3. Attachments (bounded downloads from Zalo hosts or local files only).
    let extras = resolve_ai_extras(&req.attachments, zalo_http.as_ref()).await;
    // 4. Ask, with read-only tools over this conversation when sharing is on.
    let tool_ctx = std::sync::Arc::new(AiToolCtx {
        db_path: db_path.clone(),
        thread_id: req.thread_id.clone(),
        thread_name: req.thread_name.clone(),
        is_group: req.is_group,
        lines: req.transcript.clone(),
        zalo_http: zalo_http.clone(),
    });
    let answer = crate::ai::answer_with(
        &req,
        &history,
        &extras,
        |turns, tools, max_tokens| {
            let cfg = cfg.clone();
            async move { crate::ai::chat_request(&cfg, &turns, tools.as_ref(), max_tokens).await }
        },
        |call| run_ai_tool(call, tool_ctx.clone()),
    )
    .await;
    let answer = match answer {
        Ok(a) => a,
        Err(e) => return fail(&tx, &req.thread_id, cfg.redact(&e.to_string())),
    };
    let reply = AiMessage {
        id: crate::ai::make_id(),
        thread_id: req.thread_id.clone(),
        role: AiRole::Assistant,
        text: answer.text,
        ts: crate::ai::now_ms(),
        reply_to_id: Some(req.user_msg_id.clone()),
        model: Some(cfg.model.clone()),
        note: Some(answer.note),
    };
    let stored = reply.clone();
    let _ = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let store = crate::store::MessageStore::open(&db_path)?;
        crate::ai::insert(&store.conn, &stored)?;
        Ok(())
    })
    .await;
    let _ = tx.send(GuiEvent::AiReply(reply));
}

/// Client for the Assistant's downloads from Zalo media CDNs: the session's cookies, no redirects.
fn ai_media_client(jar: std::sync::Arc<reqwest::cookie::Jar>) -> Option<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::REFERER,
        reqwest::header::HeaderValue::from_static("https://chat.zalo.me/"),
    );
    reqwest::Client::builder()
        .cookie_provider(jar)
        .redirect(reqwest::redirect::Policy::none())
        .default_headers(headers)
        .connect_timeout(std::time::Duration::from_secs(15))
        .user_agent("zalo-next-media/0.2")
        .build()
        .ok()
}

/// Newest stored rows of the conversation that `search_messages`/`list_links` scan.
const AI_TOOL_SCAN_ROWS: usize = 5_000;

/// What the Assistant's tools may read: one conversation of the local history plus the lines the
/// GUI sent (newest messages may not be stored yet).
struct AiToolCtx {
    db_path: String,
    thread_id: String,
    thread_name: String,
    is_group: bool,
    lines: Vec<crate::ai::TranscriptLine>,
    zalo_http: Option<reqwest::Client>,
}

impl AiToolCtx {
    /// Run a read on the local history (blocking pool) and convert the rows.
    async fn stored<F>(&self, read: F) -> Result<Vec<crate::ai::TranscriptLine>, String>
    where
        F: FnOnce(&crate::store::MessageStore) -> anyhow::Result<Vec<crate::store::StoredMessage>>
            + Send
            + 'static,
    {
        let (db, name, is_group) = (
            self.db_path.clone(),
            self.thread_name.clone(),
            self.is_group,
        );
        tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let store = crate::store::MessageStore::open(&db)?;
            let rows = read(&store)?;
            Ok(crate::ai::stored_transcript(&rows, &name, is_group))
        })
        .await
        .map_err(|_| "Không đọc được lịch sử cục bộ.".to_string())?
        .map_err(|_| "Không đọc được lịch sử cục bộ.".to_string())
    }

    fn newest(
        &self,
        stored: Vec<crate::ai::TranscriptLine>,
        keep: impl Fn(&crate::ai::TranscriptLine) -> bool,
        limit: usize,
    ) -> Vec<crate::ai::TranscriptLine> {
        let gui: Vec<_> = self.lines.iter().filter(|l| keep(l)).cloned().collect();
        let stored: Vec<_> = stored.into_iter().filter(|l| keep(l)).collect();
        let mut merged = crate::ai::merge_transcripts(gui, stored);
        let extra = merged.len().saturating_sub(limit);
        merged.drain(..extra);
        merged
    }

    async fn find_line(&self, message_id: &str) -> Option<crate::ai::TranscriptLine> {
        if let Some(line) = self.lines.iter().find(|l| l.id == message_id) {
            return Some(line.clone());
        }
        let (thread, id) = (self.thread_id.clone(), message_id.to_string());
        self.stored(move |store| Ok(store.message_in_thread(&thread, &id)?.into_iter().collect()))
            .await
            .ok()?
            .into_iter()
            .next()
    }
}

/// Execute one tool call against the asking conversation only. Errors become text for the model.
async fn run_ai_tool(
    call: crate::ai::ToolCall,
    ctx: std::sync::Arc<AiToolCtx>,
) -> crate::ai::ToolOutput {
    use crate::ai::{render_tool_result, render_tool_text, ToolOutput, ToolRequest};
    let name = call.name.clone();
    let text = |t: String| ToolOutput {
        text: render_tool_text(&name, &t),
        images: Vec::new(),
    };
    let request = match crate::ai::parse_tool_call(&call) {
        Ok(r) => r,
        Err(e) => return text(e),
    };
    let thread = ctx.thread_id.clone();
    let lines = match request {
        // SQLite's LIKE/LOWER fold only ASCII, so Vietnamese matching runs in Rust over a bounded
        // scan of the newest stored rows, with the same rules for stored and in-memory lines.
        ToolRequest::Search { query, limit } => ctx
            .stored(move |s| s.messages_before(&thread, None, AI_TOOL_SCAN_ROWS))
            .await
            .map(|rows| ctx.newest(rows, |l| crate::ai::line_matches(l, &query), limit)),
        ToolRequest::Links { query, limit } => ctx
            .stored(move |s| s.messages_before(&thread, None, AI_TOOL_SCAN_ROWS))
            .await
            .map(|rows| {
                ctx.newest(
                    rows,
                    |l| {
                        crate::ai::line_has_link(l)
                            && query
                                .as_deref()
                                .is_none_or(|q| crate::ai::line_matches(l, q))
                    },
                    limit,
                )
            }),
        ToolRequest::Recent { limit, before_id } => {
            let before_ts = match &before_id {
                Some(id) => match ctx.find_line(id).await {
                    Some(line) => Some(line.ts),
                    None => return text(format!("Không có tin #{id} trong cuộc trò chuyện này.")),
                },
                None => None,
            };
            ctx.stored(move |s| s.messages_before(&thread, before_ts, limit))
                .await
                .map(|rows| ctx.newest(rows, |l| before_ts.is_none_or(|ts| l.ts < ts), limit))
        }
        ToolRequest::Around { message_id, radius } => {
            let id = message_id.clone();
            let rows = ctx
                .stored(move |s| s.messages_around(&thread, &id, radius))
                .await;
            match rows {
                Ok(rows) if !rows.is_empty() => Ok(rows),
                Ok(_) => match ctx.lines.iter().position(|l| l.id == message_id) {
                    Some(i) => Ok(ctx.lines
                        [i.saturating_sub(radius)..(i + radius + 1).min(ctx.lines.len())]
                        .to_vec()),
                    None => {
                        return text(format!(
                            "Không có tin #{message_id} trong cuộc trò chuyện này."
                        ))
                    }
                },
                Err(e) => Err(e),
            }
        }
        ToolRequest::Attachment { message_id } => {
            let Some(line) = ctx.find_line(&message_id).await else {
                return text(format!(
                    "Không có tin #{message_id} trong cuộc trò chuyện này."
                ));
            };
            let Some(attachment) = crate::ai::zalo_ref(&line) else {
                return text(format!(
                    "Tin #{message_id} không có ảnh hoặc tệp đọc được. Nội dung: {}",
                    render_tool_result(&name, std::slice::from_ref(&line))
                ));
            };
            return match resolve_ai_attachment(&attachment, ctx.zalo_http.as_ref()).await {
                AiResolved::Image(url) => ToolOutput {
                    text: render_tool_text(
                        &name,
                        &format!("Ảnh của tin #{message_id} được gửi kèm ngay sau kết quả này."),
                    ),
                    images: vec![url],
                },
                AiResolved::Text(body) => text(format!("Tệp {}:\n{body}", attachment.name())),
                AiResolved::Note(note) => text(note),
            };
        }
    };
    match lines {
        Ok(lines) => ToolOutput {
            text: render_tool_result(&name, &lines),
            images: Vec::new(),
        },
        Err(e) => text(e),
    }
}

/// One attachment made ready for the model.
enum AiResolved {
    Image(String),
    Text(String),
    /// Not sent; the reason (in Vietnamese) for the model and the note.
    Note(String),
}

async fn ai_attachment_bytes(
    attachment: &crate::ai::AttachmentRef,
    zalo_http: Option<&reqwest::Client>,
    limit: usize,
) -> Result<Vec<u8>, String> {
    match attachment {
        crate::ai::AttachmentRef::Local { path, .. } => {
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                let meta = std::fs::metadata(&path)
                    .map_err(|_| "tệp đính kèm không còn trên máy".to_string())?;
                if !meta.is_file() {
                    return Err("không phải tệp".to_string());
                }
                if meta.len() > limit as u64 {
                    return Err("tệp quá lớn".to_string());
                }
                use std::io::Read;
                let mut bytes = Vec::new();
                std::fs::File::open(&path)
                    .and_then(|f| f.take(limit as u64 + 1).read_to_end(&mut bytes))
                    .map_err(|_| "không đọc được tệp".to_string())?;
                if bytes.len() > limit {
                    return Err("tệp quá lớn".to_string());
                }
                Ok(bytes)
            })
            .await
            .map_err(|_| "không đọc được tệp".to_string())?
        }
        crate::ai::AttachmentRef::Zalo { url, kind, .. } => {
            let url = if *kind == crate::ai::AttachmentKind::Photo {
                crate::store::jxl_to_jpg_url(url)
            } else {
                url.clone()
            };
            if !crate::ai::may_fetch_media(&url) {
                return Err("chỉ tải ảnh/tệp từ máy chủ Zalo; đã gửi liên kết dạng chữ".to_string());
            }
            let cookie_free;
            let client = if crate::ai::media_needs_session(&url) {
                zalo_http.ok_or_else(|| "chưa đăng nhập Zalo".to_string())?
            } else {
                cookie_free = crate::media_resources::credential_free_client()
                    .map_err(|_| "không tạo được kết nối tải".to_string())?;
                &cookie_free
            };
            let fetch = async {
                let resp = client
                    .get(&url)
                    .timeout(std::time::Duration::from_secs(30))
                    .send()
                    .await
                    .map_err(|_| crate::media_resources::MediaError::Offline)?;
                // Redirects are off; refuse anything that is not the CDN URL we checked.
                if !crate::ai::may_fetch_media(resp.url().as_str()) {
                    return Err(crate::media_resources::MediaError::Http(0));
                }
                crate::media_resources::read_bounded_response(resp, limit).await
            };
            match tokio::time::timeout(std::time::Duration::from_secs(45), fetch).await {
                Ok(Ok(bytes)) => Ok(bytes),
                Ok(Err(crate::media_resources::MediaError::TooLarge)) => Err("tệp quá lớn".into()),
                Ok(Err(_)) => Err("không tải được".into()),
                Err(_) => Err("hết thời gian tải".into()),
            }
        }
    }
}

/// Download (Zalo hosts only) or read (composer files) one attachment and turn it into an image
/// data URL or text. Never fails: problems become a note.
async fn resolve_ai_attachment(
    attachment: &crate::ai::AttachmentRef,
    zalo_http: Option<&reqwest::Client>,
) -> AiResolved {
    let name = attachment.name();
    if attachment.is_image() {
        let bytes =
            match ai_attachment_bytes(attachment, zalo_http, crate::media_resources::PREVIEW_LIMIT)
                .await
            {
                Ok(b) => b,
                Err(e) => return AiResolved::Note(format!("Ảnh {name} không gửi được: {e}")),
            };
        return match tokio::task::spawn_blocking(move || crate::ai::image_data_url(&bytes)).await {
            Ok(Ok(url)) => AiResolved::Image(url),
            Ok(Err(e)) => AiResolved::Note(format!("Ảnh {name} không gửi được: {e}")),
            Err(_) => AiResolved::Note(format!("Ảnh {name} không gửi được")),
        };
    }
    let kind = crate::ai::document_kind(&name);
    if kind == crate::ai::DocumentKind::Unsupported {
        return AiResolved::Note(format!(
            "Tệp {name}: chưa đọc được nội dung loại tệp này, chỉ có tên và liên kết"
        ));
    }
    let bytes =
        match ai_attachment_bytes(attachment, zalo_http, crate::doc_preview::MAX_PREVIEW_BYTES)
            .await
        {
            Ok(b) => b,
            Err(e) => return AiResolved::Note(format!("Tệp {name} không đọc được: {e}")),
        };
    let text = match kind {
        crate::ai::DocumentKind::Pdf => crate::doc_preview::extract_pdf_isolated(bytes)
            .await
            .map_err(|e| e.to_string()),
        crate::ai::DocumentKind::Preview(ext) => {
            tokio::task::spawn_blocking(move || crate::doc_preview::extract_text(&bytes, &ext))
                .await
                .unwrap_or_else(|_| Err("không trích được nội dung".into()))
        }
        crate::ai::DocumentKind::PlainText => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        crate::ai::DocumentKind::Unsupported => Err(String::new()),
    };
    match text {
        Ok(t) if !t.trim().is_empty() => {
            AiResolved::Text(crate::ai::cap_file_text(&t, crate::ai::FILE_TEXT_CAP_CHARS))
        }
        Ok(_) => AiResolved::Note(format!("Tệp {name} không có chữ đọc được")),
        Err(e) => AiResolved::Note(format!("Tệp {name} không đọc được: {e}")),
    }
}

/// Resolve every attachment of a question within the per-question caps.
async fn resolve_ai_extras(
    refs: &[crate::ai::AttachmentRef],
    zalo_http: Option<&reqwest::Client>,
) -> crate::ai::Extras {
    let mut extras = crate::ai::Extras::default();
    let (mut image_bytes, mut file_chars, mut failed) = (0usize, 0usize, 0usize);
    for attachment in refs {
        if attachment.is_image() && extras.images.len() >= crate::ai::MAX_REQUEST_IMAGES {
            extras.notes.push(format!(
                "Ảnh {} không gửi: quá số ảnh cho một câu hỏi",
                attachment.name()
            ));
            continue;
        }
        if !attachment.is_image() && file_chars >= crate::ai::FILES_TOTAL_CAP_CHARS {
            extras.notes.push(format!(
                "Tệp {} không gửi: quá giới hạn nội dung tệp",
                attachment.name()
            ));
            continue;
        }
        match resolve_ai_attachment(attachment, zalo_http).await {
            AiResolved::Image(url) => {
                if image_bytes + url.len() > crate::ai::MAX_IMAGE_DATA_BYTES {
                    extras.notes.push(format!(
                        "Ảnh {} không gửi: quá dung lượng ảnh cho một câu hỏi",
                        attachment.name()
                    ));
                    continue;
                }
                image_bytes += url.len();
                extras.images.push(url);
            }
            AiResolved::Text(text) => {
                let room = crate::ai::FILES_TOTAL_CAP_CHARS - file_chars;
                let text = crate::ai::cap_file_text(&text, room);
                file_chars += text.chars().count();
                extras.files.push((attachment.name(), text));
            }
            AiResolved::Note(note) => {
                failed += 1;
                extras.notes.push(note);
            }
        }
    }
    let (images, files) = (extras.images.len(), extras.files.len());
    if images + files > 0 {
        let mut parts = Vec::new();
        if images > 0 {
            parts.push(format!("{images} ảnh"));
        }
        if files > 0 {
            parts.push(format!("{files} tệp"));
        }
        extras.user_notes.push(format!("kèm {}", parts.join(", ")));
    }
    if failed > 0 {
        extras
            .user_notes
            .push(format!("{failed} tệp/ảnh không gửi được"));
    }
    extras
}

#[cfg(test)]
mod tests {
    use super::prepare_avatar_texture;
    use image::{DynamicImage, Rgba, RgbaImage};

    #[tokio::test]
    async fn poll_vote_acknowledgement_fetches_detail() {
        use super::*;

        let refetched = PollDetail {
            poll_id: Some("poll-1".into()),
            joined: Some(true),
            ..Default::default()
        };
        let result = poll_command_detail(Some(async { Ok(None) }), async { Ok(refetched.clone()) })
            .await
            .unwrap();
        assert_eq!(result, refetched);
    }

    #[tokio::test]
    async fn poll_commands_reject_an_account_that_is_no_longer_active() {
        let mut session = crate::session::Session::new(false, false, false).unwrap();
        session.login = Some(crate::session::LoginInfo {
            uin: "active-account".into(),
            ..Default::default()
        });
        let options = ["option-1".to_owned()];
        for option_ids in [None, Some(options.as_slice())] {
            assert_eq!(
                super::run_poll_command(&session, "previous-account", "poll-1", option_ids).await,
                Err(super::POLL_ACCOUNT_CHANGED.into())
            );
        }
    }

    #[tokio::test]
    async fn queued_poll_vote_is_skipped_after_the_same_account_relogs_in() {
        use super::*;

        let mut session = Session::new(false, false, false).unwrap();
        // With no key or cookies, even a regression cannot send this synthetic vote.
        session.login = Some(crate::session::LoginInfo {
            uin: "user-1".into(),
            ..Default::default()
        });
        let session = Arc::new(Mutex::new(session));
        let queue = Arc::new(Mutex::new(()));
        let turn = queue.lock().await;
        let (generation, _generation_rx) = watch::channel(0);
        let (tx, rx) = std::sync::mpsc::channel();
        spawn_poll_command(
            &session,
            &queue,
            &generation,
            &GuiTx::new(tx, egui::Context::default()),
            PollCommand {
                account: "user-1".into(),
                poll_id: "poll-1".into(),
                option_ids: Some(vec!["option-1".into()]),
                generation: 0,
            },
        );
        advance_poll_generation(&generation);
        drop(turn);

        let event = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(2)))
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            GuiEvent::PollVoted { account, poll_id, generation: 0, result: Err(error) }
                if account == "user-1" && poll_id == "poll-1" && error == POLL_ACCOUNT_CHANGED
        ));
    }

    #[tokio::test]
    async fn stale_poll_vote_future_is_never_polled() {
        use super::*;

        let (generation, mut generation_rx) = watch::channel(0);
        advance_poll_generation(&generation);
        // Simulate a change observed immediately before the request gets its first poll.
        let _ = *generation_rx.borrow_and_update();
        let result = run_in_poll_generation(&mut generation_rx, 0, async {
            panic!("stale vote must never start");
        })
        .await;
        assert_eq!(result, Err(POLL_ACCOUNT_CHANGED.into()));
    }

    #[test]
    fn original_images_get_a_larger_but_bounded_body_limit() {
        assert_eq!(
            super::sticker_image_body_limit("thumb_abc"),
            crate::media_resources::PREVIEW_LIMIT
        );
        assert_eq!(
            super::sticker_image_body_limit("hd_abc"),
            super::ORIGINAL_IMAGE_LIMIT
        );
        assert!(super::ORIGINAL_IMAGE_LIMIT > crate::media_resources::PREVIEW_LIMIT);
    }

    #[test]
    fn privacy_regression_sticker_images_use_session_only_for_trusted_hosts() {
        assert!(super::use_session_for_sticker_image(
            "https://stc-chat.zdn.vn/sticker.png"
        ));
        for url in [
            "https://images.example.test/sticker.png",
            "https://dlmd.me/sticker.png",
            "http://stc-chat.zdn.vn/sticker.png",
        ] {
            assert!(!super::use_session_for_sticker_image(url), "{url}");
        }
    }

    #[test]
    fn saved_theme_survives_reopening_settings() {
        let path = std::env::temp_dir().join(format!(
            "ctdozlite-theme-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        assert_eq!(super::AppSettings::load_from(&path).theme_mode, "dark");
        for mode in ["dark", "light", "dark"] {
            let settings = super::AppSettings {
                theme_mode: mode.into(),
                ..Default::default()
            };
            super::persist_app_settings(&path, &serde_json::to_string(&settings).unwrap()).unwrap();
            assert_eq!(super::AppSettings::load_from(&path).theme_mode, mode);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn notification_avatar_dir_is_absolute_for_the_native_helper() {
        let dir = super::notification_avatar_dir().expect("current directory");
        assert!(dir.is_absolute(), "{dir:?}");
        assert!(dir.ends_with("media_cache/notification_avatar"));
    }

    #[test]
    fn avatar_texture_center_fits_the_complete_source_image() {
        let source = RgbaImage::from_fn(400, 100, |x, _| {
            if x < 200 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });

        let avatar = prepare_avatar_texture(DynamicImage::ImageRgba8(source));

        assert_eq!(avatar.dimensions(), (160, 160));
        assert_eq!(avatar.get_pixel(0, 80), &Rgba([255, 0, 0, 255]));
        assert_eq!(avatar.get_pixel(159, 80), &Rgba([0, 0, 255, 255]));
        assert_eq!(avatar.get_pixel(80, 0).0[3], 0);
        assert_eq!(avatar.get_pixel(80, 159).0[3], 0);
    }
}

async fn fetch_document_preview(
    s: &crate::session::Session,
    url: &str,
    name: &str,
) -> anyhow::Result<String> {
    anyhow::ensure!(
        crate::session::is_trusted_zalo_url(url),
        "Untrusted document URL"
    );
    anyhow::ensure!(
        crate::doc_preview::supports(name),
        "Unsupported document preview type"
    );
    let mut response = s.http.get(url).send().await?.error_for_status()?;
    let cap = crate::doc_preview::MAX_PREVIEW_BYTES;
    anyhow::ensure!(
        response.content_length().unwrap_or(0) <= cap as u64,
        "Document too large"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len().saturating_add(chunk.len()) <= cap,
            "Document too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_owned();
    if ext.eq_ignore_ascii_case("pdf") {
        return crate::doc_preview::extract_pdf_isolated(bytes).await;
    }
    tokio::task::spawn_blocking(move || {
        crate::doc_preview::extract_text(&bytes, &ext).map_err(anyhow::Error::msg)
    })
    .await?
}

fn spawn_voice_capture(
    account: String,
    generation: u64,
    thread: String,
    is_group: bool,
    tx: GuiTx,
    ctx: egui::Context,
) -> (
    std::sync::mpsc::Sender<bool>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let (control, rx) = std::sync::mpsc::channel();
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let capture_cancelled = cancelled.clone();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<crate::voice::VoiceRecord> {
            let recorder = crate::voice::VoiceRecorder::start()?;
            loop {
                let _ = tx.send(GuiEvent::VoiceProgress {
                    account: account.clone(),
                    generation,
                    seconds: recorder.elapsed().as_secs(),
                    peak: recorder.peak(),
                    encoding: false,
                });
                ctx.request_repaint();
                if let Some(error) = recorder.take_error() {
                    anyhow::bail!("{error}");
                }
                match rx.recv_timeout(Duration::from_millis(150)) {
                    Ok(false) => break,
                    Ok(true) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        recorder.cancel();
                        anyhow::bail!("Đã hủy ghi âm.");
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
                if recorder.elapsed().as_secs() >= 300 {
                    recorder.cancel();
                    anyhow::bail!("Ghi âm đã đạt 5 phút và được hủy. Hãy thu đoạn ngắn hơn.");
                }
            }
            let _ = tx.send(GuiEvent::VoiceProgress {
                account: account.clone(),
                generation,
                seconds: recorder.elapsed().as_secs(),
                peak: 0.0,
                encoding: true,
            });
            ctx.request_repaint();
            let record = recorder.stop_and_encode()?;
            anyhow::ensure!(
                !capture_cancelled.load(std::sync::atomic::Ordering::Relaxed),
                "Đã hủy ghi âm."
            );
            Ok(record)
        })()
        .map_err(|e| e.to_string());
        if let Some(worker) = WORKER_TX.get() {
            let _ = worker.send(WorkerCmd::VoiceEncoded {
                account,
                generation,
                thread,
                is_group,
                result,
            });
        }
    });
    (control, cancelled)
}

/// Read the chosen conversation from the local history and summarise it with the Assistant. The
/// result goes back to the GUI and is held in memory only; nothing is written anywhere.
async fn run_ai_preload(
    thread_id: &str,
    thread_name: &str,
    is_group: bool,
    base_url: String,
    model: String,
    db_path: String,
) -> Result<crate::ai::Digest, String> {
    let (id, name) = (thread_id.to_string(), thread_name.to_string());
    let lines = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let store = crate::store::MessageStore::open(&db_path)?;
        let rows = store.messages_for(&id, 500)?;
        Ok(crate::ai::stored_transcript(&rows, &name, is_group))
    })
    .await
    .map_err(|_| "Không đọc được lịch sử cục bộ".to_string())?
    .map_err(|e| format!("Không đọc được lịch sử cục bộ: {e}"))?;
    let covers_until = lines
        .iter()
        .map(|line| line.ts)
        .max()
        .ok_or_else(|| "Chưa có tin nhắn để đọc".to_string())?;
    let cfg = tokio::task::spawn_blocking(move || crate::ai::AiConfig::resolve(&base_url, &model))
        .await
        .map_err(|_| "Không đọc được cấu hình trợ lý AI".to_string())?
        .map_err(|e| e.to_string())?;
    crate::ai::build_digest(
        thread_id,
        thread_name,
        is_group,
        lines,
        covers_until,
        |turns, max_tokens| {
            let cfg = cfg.clone();
            async move { crate::ai::chat_completion(&cfg, &turns, max_tokens).await }
        },
    )
    .await
    .map_err(|e| cfg.redact(&e.to_string()))
}

#[cfg(test)]
mod ai_worker_tests {
    use super::*;
    use crate::ai::{AiRole, AskRequest};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serves `bodies` in order, one connection each, and records every request body.
    async fn fake_ai(bodies: Vec<&'static str>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://127.0.0.1:{}/v1",
            listener.local_addr().unwrap().port()
        );
        let handle = tokio::spawn(async move {
            let mut seen = Vec::new();
            for body in bodies {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    let n = sock.read(&mut chunk).await.unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len = text
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                seen.push(String::from_utf8_lossy(&buf).to_string());
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
            }
            seen
        });
        (url, handle)
    }

    fn request(url: &str, share: bool) -> AskRequest {
        let id = crate::ai::make_id();
        AskRequest {
            thread_id: "thread-1".into(),
            thread_name: "Sơn".into(),
            is_group: false,
            prompt: "Rust là gì".into(),
            user_msg: Some(crate::ai::AiMessage {
                id: id.clone(),
                thread_id: "thread-1".into(),
                role: AiRole::User,
                text: "Rust là gì".into(),
                ts: crate::ai::now_ms(),
                reply_to_id: None,
                model: None,
                note: None,
            }),
            user_msg_id: id,
            quoted: None,
            quoted_msg: None,
            reply_to_id: None,
            transcript: vec![crate::ai::TranscriptLine {
                time: "10:00".into(),
                who: "Sơn".into(),
                text: "hẹn 7h nhé".into(),
                ..Default::default()
            }],
            digest: None,
            share,
            base_url: url.into(),
            model: "test-model".into(),
            attachments: Vec::new(),
        }
    }

    #[tokio::test]
    async fn ask_persists_both_sides_replies_and_leaves_the_zalo_store_untouched() {
        let dir = std::env::temp_dir().join(format!(
            "zalo-next-ai-{}-{}",
            std::process::id(),
            crate::ai::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("history.db").to_string_lossy().to_string();
        let mut store = crate::store::MessageStore::open(&db).unwrap();
        let mut older = crate::store::StoredMessage::from(&crate::chat::ChatMessage {
            id: "msg-old".into(),
            sender: "Sơn".into(),
            ts: 1_700_000_000_000,
            link_url: Some("https://example.org/history".into()),
            ..Default::default()
        });
        older.thread_id = "thread-1".into();
        store.upsert_message(&older).unwrap();
        let stale = crate::store::StoredMessage {
            msg_id: "msg-new".into(),
            text: "bản cũ".into(),
            ts: older.ts + 1,
            raw_json: String::new(),
            ..older.clone()
        };
        store.upsert_message(&stale).unwrap();
        drop(store);
        let (url, server) = fake_ai(vec![
            r#"{"choices":[{"message":{"content":"Rust là ngôn ngữ hệ thống."}}]}"#,
        ])
        .await;
        let (tx, rx) = std::sync::mpsc::channel();
        let gui_tx = GuiTx::new(tx, egui::Context::default());
        let mut req = request(&url, true);
        req.transcript[0].id = stale.msg_id.clone();
        req.transcript[0].ts = stale.ts;
        run_ai_ask(req, gui_tx, db.clone(), None).await;
        let event = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        let GuiEvent::AiReply(reply) = event else {
            panic!("expected AiReply")
        };
        assert_eq!(reply.role, AiRole::Assistant);
        assert_eq!(reply.text, "Rust là ngôn ngữ hệ thống.");
        assert_eq!(reply.note.as_deref(), Some("Đã đọc 2 tin gần nhất"));
        // The request carried the shared conversation as data.
        let sent = server.await.unwrap().remove(0);
        assert!(sent.contains("hẹn 7h nhé") && sent.contains("cuoc_tro_chuyen"));
        assert!(
            sent.find("https://example.org/history").unwrap() < sent.find("hẹn 7h nhé").unwrap()
        );
        assert!(!sent.contains("bản cũ"));
        // Both sides are stored in the local table, and Zalo messages are unchanged.
        let store = crate::store::MessageStore::open(&db).unwrap();
        let rows = crate::ai::load_thread(&store.conn, "thread-1", 10).unwrap();
        assert_eq!(
            rows.iter().map(|m| m.role).collect::<Vec<_>>(),
            [AiRole::User, AiRole::Assistant]
        );
        assert_eq!(store.messages_for("thread-1", 10).unwrap(), [older, stale]);
        // Deleting the conversation removes the dialogue too.
        let mut store = store;
        store.delete_thread("thread-1").unwrap();
        assert!(crate::ai::load_thread(&store.conn, "thread-1", 10)
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn sharing_off_sends_no_conversation_text_and_errors_are_reported_not_stored() {
        let dir = std::env::temp_dir().join(format!(
            "zalo-next-ai-off-{}-{}",
            std::process::id(),
            crate::ai::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("history.db").to_string_lossy().to_string();
        let (url, server) = fake_ai(vec![r#"{"choices":[{"message":{"content":"ok"}}]}"#]).await;
        let (tx, rx) = std::sync::mpsc::channel();
        let gui_tx = GuiTx::new(tx, egui::Context::default());
        run_ai_ask(request(&url, false), gui_tx.clone(), db.clone(), None).await;
        let GuiEvent::AiReply(reply) = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap()
        else {
            panic!()
        };
        assert_eq!(reply.note.as_deref(), Some("AI không đọc cuộc trò chuyện"));
        let sent = server.await.unwrap().remove(0);
        assert!(
            !sent.contains("hẹn 7h nhé"),
            "conversation text leaked while sharing was off"
        );
        // Unreachable endpoint: the GUI gets a failure event and nothing extra is stored.
        run_ai_ask(
            request("http://127.0.0.1:1/v1", false),
            gui_tx,
            db.clone(),
            None,
        )
        .await;
        let GuiEvent::AiFailed { error, .. } =
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap()
        else {
            panic!()
        };
        assert!(!error.is_empty());
        let store = crate::store::MessageStore::open(&db).unwrap();
        let rows = crate::ai::load_thread(&store.conn, "thread-1", 10).unwrap();
        assert!(rows.iter().all(|m| m.role != AiRole::Error));
        let _ = std::fs::remove_dir_all(dir);
    }
    #[tokio::test]
    async fn preload_reads_local_history_and_returns_an_in_memory_digest() {
        let dir = std::env::temp_dir().join(format!(
            "zalo-next-ai-pre-{}-{}",
            std::process::id(),
            crate::ai::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("history.db").to_string_lossy().to_string();
        {
            let mut store = crate::store::MessageStore::open(&db).unwrap();
            for (i, text) in ["chào", "mai họp lúc 9h", "ok", ""].iter().enumerate() {
                store
                    .upsert_message(&crate::store::StoredMessage {
                        msg_id: format!("m{i}"),
                        thread_id: "thread-1".into(),
                        sender_id: "peer".into(),
                        sender_name: "Sơn".into(),
                        text: (*text).into(),
                        msg_type: "webchat".into(),
                        ts: 1_700_000_000_000 + i as i64,
                        is_outgoing: i == 2,
                        raw_json: if i == 3 {
                            r#"{"link_url":"https://example.org/agenda"}"#.into()
                        } else {
                            String::new()
                        },
                    })
                    .unwrap();
            }
        }
        let before: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let (url, server) = fake_ai(vec![
            r#"{"choices":[{"message":{"content":"Họp lúc 9h mai."}}]}"#,
        ])
        .await;
        let digest = run_ai_preload("thread-1", "Sơn", false, url, "test-model".into(), db)
            .await
            .unwrap();
        assert_eq!(digest.text, "Họp lúc 9h mai.");
        assert_eq!(digest.message_count, 4);
        assert_eq!(digest.covers_until_ms, 1_700_000_000_003);
        let sent = server.await.unwrap().remove(0);
        assert!(sent.contains("mai họp lúc 9h") && sent.contains("Tôi: ok"));
        assert!(sent.contains("[Liên kết: https://example.org/agenda]"));
        // Nothing but the existing history database exists afterwards: the digest is not stored.
        let after: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(before.len(), after.len());
        // An empty conversation is reported, not summarised.
        let err = run_ai_preload(
            "nobody",
            "X",
            false,
            "http://127.0.0.1:1/v1".into(),
            "m".into(),
            dir.join("history.db").to_string_lossy().to_string(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("Chưa có tin nhắn"));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tool_ctx(db: &str, lines: Vec<crate::ai::TranscriptLine>) -> std::sync::Arc<AiToolCtx> {
        std::sync::Arc::new(AiToolCtx {
            db_path: db.into(),
            thread_id: "A".into(),
            thread_name: "Nhóm".into(),
            is_group: true,
            lines,
            zalo_http: None,
        })
    }

    #[tokio::test]
    async fn tools_read_only_the_asking_conversation_plus_unsaved_gui_lines() {
        let dir = std::env::temp_dir().join(format!(
            "zalo-next-ai-tools-{}-{}",
            std::process::id(),
            crate::ai::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("history.db").to_string_lossy().to_string();
        {
            let mut store = crate::store::MessageStore::open(&db).unwrap();
            for (thread, id, ts, text, raw) in [
                (
                    "A",
                    "a1",
                    1_700_000_000_000i64,
                    "tài liệu đây",
                    r#"{"link_url":"https://docs.example/a","link_title":"Kế hoạch"}"#,
                ),
                ("A", "a2", 1_700_000_001_000, "ok", ""),
                (
                    "B",
                    "b1",
                    1_700_000_002_000,
                    "bí mật https://secret.example",
                    "",
                ),
                ("A", "a0", 1_699_999_999_000, "Hẹn ở Đà Nẵng", ""),
                (
                    "A",
                    "a5",
                    1_699_999_998_000,
                    "[Hình ảnh]",
                    r#"{"link_url":"https://f1.zdn.vn/p.jpg","link_src":"Hình ảnh Zalo"}"#,
                ),
            ] {
                store
                    .upsert_message(&crate::store::StoredMessage {
                        msg_id: id.into(),
                        thread_id: thread.into(),
                        sender_id: "1".into(),
                        sender_name: "An".into(),
                        text: text.into(),
                        msg_type: "webchat".into(),
                        ts,
                        is_outgoing: false,
                        raw_json: raw.into(),
                    })
                    .unwrap();
            }
        }
        let unsaved = crate::ai::TranscriptLine {
            id: "g1".into(),
            ts: 1_700_000_003_000,
            who: "Bình".into(),
            text: "link mới https://new.example/x".into(),
            ..Default::default()
        };
        let ctx = tool_ctx(&db, vec![unsaved]);
        let call = |name: &str, args: &str| crate::ai::ToolCall {
            id: "c".into(),
            name: name.into(),
            arguments: args.into(),
        };
        let links = run_ai_tool(call("list_links", "{}"), ctx.clone())
            .await
            .text;
        assert!(
            links.contains("https://docs.example/a") && links.contains("https://new.example/x")
        );
        assert!(
            !links.contains("secret.example"),
            "another conversation leaked: {links}"
        );
        let found = run_ai_tool(
            call("search_messages", r#"{"query":"bí mật"}"#),
            ctx.clone(),
        )
        .await
        .text;
        assert!(found.contains("Không tìm thấy"));
        // Vietnamese case folding works on stored rows (SQLite LOWER is ASCII-only).
        let folded = run_ai_tool(
            call("search_messages", r#"{"query":"đà nẵng"}"#),
            ctx.clone(),
        )
        .await
        .text;
        assert!(folded.contains("#a0"), "{folded}");
        // Photos are not links.
        assert!(!links.contains("f1.zdn.vn"), "{links}");
        let around = run_ai_tool(
            call("get_messages_around", r#"{"message_id":"b1"}"#),
            ctx.clone(),
        )
        .await
        .text;
        assert!(around.contains("Không có tin #b1"));
        let recent = run_ai_tool(
            call("get_recent_messages", r#"{"before_id":"a2"}"#),
            ctx.clone(),
        )
        .await
        .text;
        assert!(recent.contains("#a1") && !recent.contains("#a2") && !recent.contains("#g1"));
        let bad = run_ai_tool(call("search_messages", "{"), ctx.clone())
            .await
            .text;
        assert!(bad.contains("JSON"));
        // A link card has no photo/file to open; nothing is downloaded.
        let open = run_ai_tool(call("read_attachment", r#"{"message_id":"a1"}"#), ctx).await;
        assert!(open.images.is_empty() && open.text.contains("không có ảnh hoặc tệp"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn attachments_resolve_from_local_files_and_never_from_other_hosts() {
        let dir = std::env::temp_dir().join(format!(
            "zalo-next-ai-att-{}-{}",
            std::process::id(),
            crate::ai::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let notes = dir.join("ghi-chu.md");
        std::fs::write(&notes, "# Việc cần làm\n- gửi báo giá").unwrap();
        let png = dir.join("anh.png");
        image::RgbImage::from_pixel(40, 20, image::Rgb([1, 2, 3]))
            .save(&png)
            .unwrap();
        let refs = vec![
            crate::ai::AttachmentRef::Local {
                path: notes,
                name: "ghi-chu.md".into(),
                is_image: false,
            },
            crate::ai::AttachmentRef::Local {
                path: png,
                name: "anh.png".into(),
                is_image: true,
            },
            crate::ai::AttachmentRef::Local {
                path: dir.join("missing.txt"),
                name: "missing.txt".into(),
                is_image: false,
            },
            crate::ai::AttachmentRef::Zalo {
                message_id: "m".into(),
                kind: crate::ai::AttachmentKind::Photo,
                url: "https://images.example.org/cat.jpg".into(),
                name: None,
            },
        ];
        let extras = resolve_ai_extras(&refs, None).await;
        assert_eq!(
            extras.files,
            [(
                "ghi-chu.md".to_string(),
                "# Việc cần làm\n- gửi báo giá".to_string()
            )]
        );
        assert_eq!(extras.images.len(), 1);
        assert!(extras.images[0].starts_with("data:image/jpeg;base64,"));
        assert!(extras
            .notes
            .iter()
            .any(|n| n.contains("không còn trên máy")));
        assert!(extras
            .notes
            .iter()
            .any(|n| n.contains("chỉ tải ảnh/tệp từ máy chủ Zalo")));
        assert_eq!(
            extras.user_notes,
            ["kèm 1 ảnh, 1 tệp", "2 tệp/ảnh không gửi được"]
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
