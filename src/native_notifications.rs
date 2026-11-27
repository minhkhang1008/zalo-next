//! Native notification authorization and delivery for macOS and Windows.
//!
//! macOS uses the bundled `DailyNotifier` helper or falls back to `osascript` if it is absent.
//! Windows uses the WinRT toast API. Both platforms pass message text as data.
#[cfg(any(not(windows), test))]
use std::path::PathBuf;
#[cfg(not(windows))]
use std::process::Stdio;
use std::{path::Path, time::Duration};
#[cfg(not(windows))]
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Longest notification title forwarded to the OS, in characters.
const MAX_TITLE_CHARS: usize = 120;
/// Longest notification body forwarded to the OS, in characters.
const MAX_BODY_CHARS: usize = 240;
/// Deadline for a single delivery attempt.
const NOTIFY_DEADLINE: Duration = Duration::from_secs(20);

/// Authorization reported by the operating system helper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationStatus {
    /// The user has not chosen yet.
    NotDetermined,
    /// The user denied authorization.
    Denied,
    /// Alerts are authorized.
    Authorized,
    /// Quiet alerts are authorized.
    Provisional,
    /// Temporary authorization is available.
    Ephemeral,
    /// The operating system cannot report authorization.
    Unavailable,
}
impl AuthorizationStatus {
    /// Vietnamese label for the Settings panel.
    pub fn label(self) -> &'static str {
        match self {
            Self::NotDetermined => "Chưa yêu cầu quyền",
            #[cfg(windows)]
            Self::Denied => "Đã từ chối. Bạn có thể đổi trong Cài đặt > Hệ thống > Thông báo.",
            #[cfg(not(windows))]
            Self::Denied => "Đã từ chối. Bạn có thể đổi trong Cài đặt hệ thống macOS.",
            Self::Authorized => "Đã cấp quyền",
            Self::Provisional => "Được phép thông báo im lặng",
            Self::Ephemeral => "Được cấp quyền tạm thời",
            Self::Unavailable => "Trạng thái quyền chưa khả dụng",
        }
    }
}
/// Bounded helper failures; raw helper output never reaches the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationError {
    /// Permission requests require an explicit user gesture.
    ExplicitUserActionRequired,
    /// This platform or installation has no executable helper.
    HelperUnavailable,
    /// The notification operation exceeded its deadline.
    Timeout,
    /// The helper exited unsuccessfully or returned invalid/oversized output.
    InvalidResponse,
    /// The child pipe could not be read.
    Io,
    /// The operating system has not authorized notifications for the app.
    NotAuthorized,
    /// Windows could not register the app or use the toast API.
    #[cfg(windows)]
    WindowsApi,
}
impl std::fmt::Display for NotificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ExplicitUserActionRequired => "Hãy bấm nút yêu cầu quyền để tiếp tục.",
            Self::HelperUnavailable => "Chưa tìm thấy trợ giúp quyền thông báo macOS.",
            Self::Timeout => "Đọc quyền thông báo quá thời gian. Hãy thử lại.",
            Self::InvalidResponse => "Trợ giúp quyền thông báo trả kết quả không hợp lệ.",
            Self::Io => "Lỗi đọc kết quả quyền thông báo. Hãy thử lại.",
            #[cfg(windows)]
            Self::NotAuthorized => "Windows chưa cho phép ứng dụng gửi thông báo.",
            #[cfg(not(windows))]
            Self::NotAuthorized => "macOS chưa cho phép ứng dụng gửi thông báo.",
            #[cfg(windows)]
            Self::WindowsApi => "Không thể sử dụng thông báo Windows. Hãy thử lại.",
        })
    }
}
impl std::error::Error for NotificationError {}

#[cfg(any(not(windows), test))]
fn packaged_helper(executable: &Path) -> Option<PathBuf> {
    Some(
        executable
            .parent()?
            .parent()?
            .join("Helpers/DailyNotifier.app/Contents/MacOS/DailyNotifier"),
    )
}
#[cfg(not(windows))]
fn helper_path() -> PathBuf {
    let packaged = std::env::current_exe()
        .ok()
        .and_then(|path| packaged_helper(&path));
    packaged.filter(|path| path.is_file()).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("native/macos/build/DailyNotifier.app/Contents/MacOS/DailyNotifier")
    })
}
/// Spawn the helper, retrying briefly on `ETXTBSY`: a write handle to a freshly written executable
/// can still be held by a concurrently forked child that has not reached `exec` yet.
#[cfg(not(windows))]
async fn spawn_helper(
    path: &Path,
    command: &str,
) -> Result<tokio::process::Child, NotificationError> {
    spawn_with(path, &[command], false).await
}
#[cfg(not(windows))]
async fn spawn_with(
    path: &Path,
    args: &[&str],
    piped_stdin: bool,
) -> Result<tokio::process::Child, NotificationError> {
    let mut attempt = 0;
    loop {
        match tokio::process::Command::new(path)
            .args(args)
            .stdin(if piped_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 5 => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(10 * attempt)).await;
            }
            Err(_) => return Err(NotificationError::HelperUnavailable),
        }
    }
}
/// Run a child to completion under `deadline`, returning its bounded stdout on exit status 0.
#[cfg(not(windows))]
async fn collect_output(
    mut child: tokio::process::Child,
    stdin_payload: Option<Vec<u8>>,
    deadline: Duration,
) -> Result<String, NotificationError> {
    let operation = async {
        if let Some(payload) = stdin_payload {
            let mut stdin = child.stdin.take().ok_or(NotificationError::Io)?;
            stdin
                .write_all(&payload)
                .await
                .map_err(|_| NotificationError::Io)?;
            // Dropping stdin signals EOF to the helper.
            drop(stdin);
        }
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or(NotificationError::Io)?
            .take(4097)
            .read_to_end(&mut output)
            .await
            .map_err(|_| NotificationError::Io)?;
        if output.len() > 4096 {
            return Err(NotificationError::InvalidResponse);
        }
        if !child
            .wait()
            .await
            .map_err(|_| NotificationError::Io)?
            .success()
        {
            return Err(NotificationError::InvalidResponse);
        }
        String::from_utf8(output).map_err(|_| NotificationError::InvalidResponse)
    };
    tokio::time::timeout(deadline, operation)
        .await
        .map_err(|_| NotificationError::Timeout)?
}
#[cfg(not(windows))]
async fn invoke(
    path: &Path,
    command: &str,
    deadline: Duration,
) -> Result<AuthorizationStatus, NotificationError> {
    let child = spawn_helper(path, command).await?;
    parse_status(&collect_output(child, None, deadline).await?)
}
#[cfg(any(not(windows), test))]
fn parse_status(output: &str) -> Result<AuthorizationStatus, NotificationError> {
    match output.trim() {
        "notDetermined" => Ok(AuthorizationStatus::NotDetermined),
        "denied" => Ok(AuthorizationStatus::Denied),
        "authorized" => Ok(AuthorizationStatus::Authorized),
        "provisional" => Ok(AuthorizationStatus::Provisional),
        "ephemeral" => Ok(AuthorizationStatus::Ephemeral),
        "unavailable" => Ok(AuthorizationStatus::Unavailable),
        _ => Err(NotificationError::InvalidResponse),
    }
}
#[cfg(not(windows))]
async fn platform_invoke(
    command: &str,
    deadline: Duration,
) -> Result<AuthorizationStatus, NotificationError> {
    if !cfg!(target_os = "macos") {
        return Err(NotificationError::HelperUnavailable);
    }
    invoke(&helper_path(), command, deadline).await
}
/// Read authorization without asking for permission or posting a notification.
///
/// # Errors
/// Returns a typed error for unsupported platforms, missing helpers, deadlines or invalid output.
/// Windows reports authorization lookup failures as [`AuthorizationStatus::Unavailable`].
pub async fn status() -> Result<AuthorizationStatus, NotificationError> {
    #[cfg(windows)]
    {
        Ok(windows_toast::status().await)
    }
    #[cfg(not(windows))]
    {
        platform_invoke("--status", Duration::from_secs(10)).await
    }
}
/// On macOS, request authorization only after the user clicks the permission button.
///
/// Windows has no permission prompt, so this returns the current status.
///
/// # Errors
/// On macOS and unsupported platforms, returns `ExplicitUserActionRequired` when the
/// gesture is absent, or a typed helper error otherwise.
pub async fn request_if_user_action(
    explicit_user_action: bool,
) -> Result<AuthorizationStatus, NotificationError> {
    #[cfg(windows)]
    {
        let _ = explicit_user_action;
        status().await
    }
    #[cfg(not(windows))]
    {
        if !explicit_user_action {
            return Err(NotificationError::ExplicitUserActionRequired);
        }
        platform_invoke("--request", Duration::from_secs(120)).await
    }
}

/// Collapse control characters (including newlines) to spaces and cap the length, so the OS
/// banner stays one readable line and the helper's payload bounds always hold.
fn sanitize(text: &str, max_chars: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = flat.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    let mut cut: String = trimmed.chars().take(max_chars.saturating_sub(1)).collect();
    cut.push('…');
    cut
}
/// Opaque 64-hex request identifier required by the helper; it carries no message identity.
#[cfg(any(not(windows), test))]
fn opaque_id() -> String {
    use sha2::{Digest, Sha256};
    let mut seed = [0u8; 32];
    if getrandom::fill(&mut seed).is_err() {
        // Uniqueness only matters for not replacing an earlier banner.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        seed[..16].copy_from_slice(&nanos.to_le_bytes());
    }
    Sha256::digest(seed)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[cfg(any(not(windows), test))]
fn helper_payload(title: &str, body: &str, attachment: Option<&Path>) -> Vec<u8> {
    serde_json::json!({
        "title": title,
        "body": body,
        "id": opaque_id(),
        // The GUI already plays its own chime when sound is enabled.
        "sound": false,
        "attachment": attachment
            .filter(|path| path.is_absolute() && path.is_file())
            .and_then(Path::to_str),
    })
    .to_string()
    .into_bytes()
}
#[cfg(any(not(windows), test))]
fn parse_delivery(output: &str) -> Result<(), NotificationError> {
    match output.trim() {
        "submitted" => Ok(()),
        "notAuthorized" => Err(NotificationError::NotAuthorized),
        _ => Err(NotificationError::InvalidResponse),
    }
}
#[cfg(not(windows))]
async fn notify_via_helper(
    path: &Path,
    title: &str,
    body: &str,
    attachment: Option<&Path>,
) -> Result<(), NotificationError> {
    run_helper_delivery(path, "--notify", title, body, attachment).await
}
/// Self-drawn toast: shows the sender avatar as the only icon, which native banners cannot do.
/// The helper prints `submitted` once the panel is on screen and exits when it is dismissed.
#[cfg(not(windows))]
async fn toast_via_helper(
    path: &Path,
    title: &str,
    body: &str,
    attachment: Option<&Path>,
) -> Result<(), NotificationError> {
    run_helper_delivery(path, "--toast", title, body, attachment).await
}
#[cfg(not(windows))]
async fn run_helper_delivery(
    path: &Path,
    mode: &str,
    title: &str,
    body: &str,
    attachment: Option<&Path>,
) -> Result<(), NotificationError> {
    let child = spawn_with(path, &[mode], true).await?;
    parse_delivery(
        &collect_output(
            child,
            Some(helper_payload(title, body, attachment)),
            NOTIFY_DEADLINE,
        )
        .await?,
    )
}
/// AppleScript that reads title and body from `argv`, so message text is never parsed as code.
#[cfg(any(not(windows), test))]
const OSASCRIPT_LINES: [&str; 3] = [
    "on run argv",
    "display notification (item 2 of argv) with title (item 1 of argv)",
    "end run",
];
#[cfg(any(not(windows), test))]
fn osascript_args<'a>(title: &'a str, body: &'a str) -> Vec<&'a str> {
    let mut args = Vec::with_capacity(9);
    for line in OSASCRIPT_LINES {
        args.push("-e");
        args.push(line);
    }
    // `--` ends option parsing so text starting with `-` stays a plain argument.
    args.extend(["--", title, body]);
    args
}
#[cfg(not(windows))]
async fn notify_via_osascript(title: &str, body: &str) -> Result<(), NotificationError> {
    let child = spawn_with(
        Path::new("/usr/bin/osascript"),
        &osascript_args(title, body),
        false,
    )
    .await?;
    collect_output(child, None, NOTIFY_DEADLINE)
        .await
        .map(|_| ())
}
/// Post a native new-message notification with `title` (sender or group) and `body` (preview).
///
/// macOS uses the bundled `DailyNotifier` helper when its binary exists, otherwise `osascript`.
/// Windows delivers a WinRT toast.
///
/// # Errors
/// Returns a typed error for unsupported platforms, missing tools, deadlines, denied
/// authorization or invalid helper output.
pub async fn notify(title: &str, body: &str) -> Result<(), NotificationError> {
    notify_with_attachment(title, body, None).await
}

/// Post a native notification with an optional locally cached sender avatar attachment.
///
/// The packaged macOS helper and Windows toast support attachments. The AppleScript fallback
/// delivers only the text because AppleScript has no attachment API.
///
/// # Errors
/// Returns the same bounded delivery errors as [`notify`].
pub async fn notify_with_attachment(
    title: &str,
    body: &str,
    attachment: Option<&Path>,
) -> Result<(), NotificationError> {
    if !cfg!(any(target_os = "macos", windows)) {
        return Err(NotificationError::HelperUnavailable);
    }
    let title = sanitize(title, MAX_TITLE_CHARS);
    let body = sanitize(body, MAX_BODY_CHARS);
    let title = if title.is_empty() {
        crate::APP_DISPLAY_NAME.to_owned()
    } else {
        title
    };
    let body = if body.is_empty() {
        "Tin nhắn mới".to_owned()
    } else {
        body
    };
    #[cfg(windows)]
    {
        windows_toast::notify(title, body, attachment.map(Path::to_path_buf)).await
    }
    #[cfg(not(windows))]
    {
        let helper = helper_path();
        if helper.is_file() {
            // The toast needs no notification permission; fall back to the native banner if the
            // helper cannot draw it (older helper binary, no screen).
            if toast_via_helper(&helper, &title, &body, attachment)
                .await
                .is_ok()
            {
                return Ok(());
            }
            return notify_via_helper(&helper, &title, &body, attachment).await;
        }
        notify_via_osascript(&title, &body).await
    }
}

#[cfg(any(windows, test))]
fn windows_toast_template(with_image: bool) -> &'static str {
    // The GUI controls notification sounds separately.
    const TEXT: &str = r#"<toast>
        <visual><binding template="ToastGeneric"><text/><text/></binding></visual>
        <audio silent="true"/>
    </toast>"#;
    const TEXT_AND_IMAGE: &str = r#"<toast>
        <visual><binding template="ToastGeneric">
            <text/><text/><image placement="appLogoOverride" hint-crop="circle"/>
        </binding></visual>
        <audio silent="true"/>
    </toast>"#;
    if with_image {
        TEXT_AND_IMAGE
    } else {
        TEXT
    }
}

#[cfg(any(windows, test))]
fn local_attachment_uri(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let uri = reqwest::Url::from_file_path(path).ok()?;
    // A Windows UNC path also converts to a file URI. Reject its host before checking the file.
    if uri.has_host() || !path.is_file() {
        return None;
    }
    Some(uri.into())
}

#[cfg(any(windows, test))]
fn windows_authorization_status(setting: Option<i32>) -> AuthorizationStatus {
    // WinRT NotificationSetting discriminants. Unknown future values are unavailable.
    match setting {
        Some(0) => AuthorizationStatus::Authorized,
        Some(1..=4) => AuthorizationStatus::Denied,
        _ => AuthorizationStatus::Unavailable,
    }
}

#[cfg(windows)]
mod windows_toast {
    use super::{
        local_attachment_uri, windows_authorization_status, windows_toast_template,
        AuthorizationStatus, NotificationError, NOTIFY_DEADLINE,
    };
    use std::{path::PathBuf, time::Duration};
    use windows::{
        core::{self, h, w, Interface, HSTRING, PCWSTR},
        Data::Xml::Dom::{XmlDocument, XmlElement},
        Win32::{
            Foundation::ERROR_SUCCESS,
            System::{
                Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ},
                WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
            },
        },
        UI::Notifications::{ToastNotification, ToastNotificationManager, ToastNotifier},
    };

    const AUMID: &str = "com.ctdoteam1110.ctdozlite";

    struct WinRtApartment;

    impl Drop for WinRtApartment {
        fn drop(&mut self) {
            // SAFETY: The guard is created after successful initialization and stays on that thread.
            unsafe { RoUninitialize() };
        }
    }

    fn register_aumid() -> Result<(), NotificationError> {
        let key: Vec<u16> = r"Software\Classes\AppUserModelId\"
            .encode_utf16()
            .chain(AUMID.encode_utf16())
            .chain([0])
            .collect();
        let display_name: Vec<u16> = crate::APP_DISPLAY_NAME.encode_utf16().chain([0]).collect();
        // SAFETY: Both UTF-16 buffers are NUL-terminated and live throughout the call.
        // The REG_SZ byte count includes the terminating NUL.
        let result = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                w!("DisplayName"),
                REG_SZ.0,
                Some(display_name.as_ptr().cast()),
                std::mem::size_of_val(display_name.as_slice()) as u32,
            )
        };
        if result == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(NotificationError::WindowsApi)
        }
    }

    fn hstring(text: &str) -> core::Result<HSTRING> {
        HSTRING::from_wide(&text.encode_utf16().collect::<Vec<_>>())
    }

    async fn with_notifier<T: Send + 'static>(
        deadline: Duration,
        operation: impl FnOnce(&ToastNotifier) -> Result<T, NotificationError> + Send + 'static,
    ) -> Result<T, NotificationError> {
        let mut task = tokio::task::spawn_blocking(move || {
            // SAFETY: We pair every successful initialization, including S_FALSE, with
            // RoUninitialize on this thread after the WinRT objects have been dropped.
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
                .map_err(|_| NotificationError::WindowsApi)?;
            let _apartment = WinRtApartment;
            register_aumid()?;
            let notifier = hstring(AUMID)
                .and_then(|id| ToastNotificationManager::CreateToastNotifierWithId(&id))
                .map_err(|_| NotificationError::WindowsApi)?;
            operation(&notifier)
        });
        match tokio::time::timeout(deadline, &mut task).await {
            Ok(result) => result.map_err(|_| NotificationError::WindowsApi)?,
            Err(_) => {
                // This cancels queued work. A WinRT call already in progress runs to completion.
                task.abort();
                Err(NotificationError::Timeout)
            }
        }
    }

    fn read_setting(notifier: &ToastNotifier) -> Result<i32, NotificationError> {
        notifier
            .Setting()
            .map(|setting| setting.0)
            .map_err(|_| NotificationError::WindowsApi)
    }

    pub(super) async fn status() -> AuthorizationStatus {
        windows_authorization_status(
            with_notifier(Duration::from_secs(10), read_setting)
                .await
                .ok(),
        )
    }

    fn toast_document(
        title: &str,
        body: &str,
        image_uri: Option<&str>,
    ) -> core::Result<XmlDocument> {
        let document = XmlDocument::new()?;
        document.LoadXml(&hstring(windows_toast_template(image_uri.is_some()))?)?;
        let text = document.GetElementsByTagName(h!("text"))?;
        text.Item(0)?.SetInnerText(&hstring(title)?)?;
        text.Item(1)?.SetInnerText(&hstring(body)?)?;
        if let Some(uri) = image_uri {
            let image = document
                .GetElementsByTagName(h!("image"))?
                .Item(0)?
                .cast::<XmlElement>()?;
            image.SetAttribute(h!("src"), &hstring(uri)?)?;
        }
        Ok(document)
    }

    pub(super) async fn notify(
        title: String,
        body: String,
        attachment: Option<PathBuf>,
    ) -> Result<(), NotificationError> {
        with_notifier(NOTIFY_DEADLINE, move |notifier| {
            match windows_authorization_status(Some(read_setting(notifier)?)) {
                AuthorizationStatus::Authorized => (),
                AuthorizationStatus::Denied => return Err(NotificationError::NotAuthorized),
                _ => return Err(NotificationError::WindowsApi),
            }
            let image_uri = attachment.as_deref().and_then(local_attachment_uri);
            toast_document(&title, &body, image_uri.as_deref())
                .and_then(|document| ToastNotification::CreateToastNotification(&document))
                .and_then(|toast| notifier.Show(&toast))
                .map_err(|_| NotificationError::WindowsApi)
        })
        .await
    }
}

/// UI-owned sequence gate: at most one request is pending; stale completions are ignored.
#[derive(Default)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) struct AuthorizationState {
    next: u64,
    pub(crate) pending: Option<u64>,
    pub(crate) result: Option<Result<AuthorizationStatus, NotificationError>>,
}
impl AuthorizationState {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn begin(&mut self) -> Option<u64> {
        if self.pending.is_some() {
            return None;
        }
        self.next = self.next.checked_add(1)?;
        self.pending = Some(self.next);
        Some(self.next)
    }
    pub(crate) fn finish(
        &mut self,
        id: u64,
        result: Result<AuthorizationStatus, NotificationError>,
    ) {
        if self.pending == Some(id) {
            self.pending = None;
            self.result = Some(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_toast_templates_only_contain_fixed_markup() {
        for with_image in [false, true] {
            let template = windows_toast_template(with_image);
            assert!(template.contains(r#"template="ToastGeneric""#));
            assert_eq!(template.matches("<text/>").count(), 2);
            assert_eq!(template.contains("<image"), with_image);
            if with_image {
                assert!(template.contains(r#"placement="appLogoOverride" hint-crop="circle""#));
            }
            for forbidden in ["action", "protocol", "src=", "http"] {
                assert!(!template.contains(forbidden));
            }
        }
    }

    #[test]
    fn windows_toast_avatar_uri_encodes_existing_absolute_file() {
        let dir = crate::data_dir::PrivateTempDir::new("zalo-toast-avatar").unwrap();
        let path = dir.path().join("avatar #100%.png");
        std::fs::write(&path, b"avatar fixture").unwrap();

        let uri = local_attachment_uri(&path).unwrap();
        assert!(uri.starts_with("file:///"));
        assert!(uri.ends_with("/avatar%20%23100%25.png"));
    }

    #[test]
    fn windows_toast_avatar_uri_rejects_relative_non_file_and_remote_paths() {
        let dir = crate::data_dir::PrivateTempDir::new("zalo-toast-avatar").unwrap();
        for path in [
            Path::new("avatar.png"),
            dir.path(),
            &dir.path().join("missing.png"),
            Path::new("http://example.invalid/avatar.png"),
            Path::new("https://example.invalid/avatar.png"),
        ] {
            assert_eq!(local_attachment_uri(path), None);
        }
        #[cfg(windows)]
        assert_eq!(
            local_attachment_uri(Path::new(r"\\example.invalid\share\avatar.png")),
            None
        );
    }

    #[test]
    fn windows_toast_setting_maps_enabled_disabled_and_unavailable() {
        assert_eq!(
            windows_authorization_status(Some(0)),
            AuthorizationStatus::Authorized
        );
        for setting in 1..=4 {
            assert_eq!(
                windows_authorization_status(Some(setting)),
                AuthorizationStatus::Denied
            );
        }
        for setting in [None, Some(5)] {
            assert_eq!(
                windows_authorization_status(setting),
                AuthorizationStatus::Unavailable
            );
        }
    }

    #[test]
    fn parser_accepts_all_statuses_and_rejects_unknown() {
        for (raw, expected) in [
            ("notDetermined", AuthorizationStatus::NotDetermined),
            ("denied", AuthorizationStatus::Denied),
            ("authorized", AuthorizationStatus::Authorized),
            ("provisional", AuthorizationStatus::Provisional),
            ("ephemeral", AuthorizationStatus::Ephemeral),
            ("unavailable", AuthorizationStatus::Unavailable),
        ] {
            assert_eq!(parse_status(&format!("{raw}\n")), Ok(expected));
        }
        for invalid in ["", "unknown", "authorized\ndenied", "submitted"] {
            assert_eq!(
                parse_status(invalid),
                Err(NotificationError::InvalidResponse)
            );
        }
    }

    #[test]
    fn sanitize_flattens_controls_and_caps_length() {
        assert_eq!(sanitize("  a\nb\tc\u{7}  ", 50), "a b c");
        let long = "x".repeat(300);
        let out = sanitize(&long, 120);
        assert_eq!(out.chars().count(), 120);
        assert!(out.ends_with('…'));
        assert_eq!(sanitize("Tiếng Việt có dấu", 120), "Tiếng Việt có dấu");
    }

    #[test]
    fn osascript_passes_text_only_as_argv_after_double_dash() {
        let hostile = "\" \\ end run -- $(rm -rf ~) `x`";
        let args = osascript_args("-Nhóm \"A\"", hostile);
        let dash = args.iter().position(|a| *a == "--").unwrap();
        assert_eq!(&args[dash + 1..], &["-Nhóm \"A\"", hostile]);
        // Script lines are constants; no caller text appears before `--`.
        for (i, a) in args[..dash].iter().enumerate() {
            if i % 2 == 0 {
                assert_eq!(*a, "-e");
            } else {
                assert!(OSASCRIPT_LINES.contains(a));
            }
        }
    }

    #[test]
    fn helper_payload_is_json_with_opaque_hex_id() {
        let raw = helper_payload("A \"quoted\" name", "line", None);
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["title"], "A \"quoted\" name");
        assert_eq!(v["body"], "line");
        assert_eq!(v["sound"], false);
        let id = v["id"].as_str().unwrap();
        assert_eq!(id.len(), 64);
        assert!(id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert!(raw.len() <= 4096);
    }

    #[test]
    fn helper_payload_includes_only_existing_absolute_attachment() {
        let path = std::env::temp_dir().join(format!(
            "ctdozlite-notification-avatar-{}",
            std::process::id()
        ));
        std::fs::write(&path, b"png").unwrap();
        let raw = helper_payload("Sender", "Body", Some(&path));
        let value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(value["attachment"], path.to_str().unwrap());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn delivery_parser_maps_helper_output() {
        assert_eq!(parse_delivery("submitted\n"), Ok(()));
        assert_eq!(
            parse_delivery("notAuthorized"),
            Err(NotificationError::NotAuthorized)
        );
        assert_eq!(
            parse_delivery("failed"),
            Err(NotificationError::InvalidResponse)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mock_helper_receives_notify_payload_on_stdin() {
        let fixture =
            Fixture::new("[ \"$1\" = --notify ] || exit 9; cat > \"$0.stdin\"; echo submitted");
        assert_eq!(
            notify_via_helper(&fixture.0.join("helper"), "Sender", "Body", None).await,
            Ok(())
        );
        let stdin = std::fs::read(fixture.0.join("helper.stdin")).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&stdin).unwrap();
        assert_eq!(
            (v["title"].as_str(), v["body"].as_str()),
            (Some("Sender"), Some("Body"))
        );
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn permission_requires_explicit_action_before_spawn() {
        assert_eq!(
            request_if_user_action(false).await,
            Err(NotificationError::ExplicitUserActionRequired)
        );
    }

    #[test]
    fn state_prevents_duplicates_and_ignores_stale_results_with_retry() {
        let mut state = AuthorizationState::default();
        let first = state.begin().unwrap();
        assert_eq!(state.begin(), None);
        state.finish(first + 1, Ok(AuthorizationStatus::Authorized));
        assert_eq!(state.pending, Some(first));
        state.finish(first, Err(NotificationError::Timeout));
        assert_eq!(state.result, Some(Err(NotificationError::Timeout)));
        let second = state.begin().unwrap();
        state.finish(first, Ok(AuthorizationStatus::Denied));
        assert_eq!(state.pending, Some(second));
        state.finish(second, Ok(AuthorizationStatus::Authorized));
        assert_eq!(state.result, Some(Ok(AuthorizationStatus::Authorized)));
    }

    #[test]
    fn every_helper_error_clears_loading_and_allows_retry() {
        let mut state = AuthorizationState::default();
        for error in [
            NotificationError::HelperUnavailable,
            NotificationError::Timeout,
            NotificationError::InvalidResponse,
            NotificationError::Io,
            NotificationError::ExplicitUserActionRequired,
        ] {
            let id = state.begin().unwrap();
            state.finish(id, Err(error));
            assert_eq!(state.pending, None);
            assert_eq!(state.result, Some(Err(error)));
        }
        assert!(state.begin().is_some());
    }

    #[test]
    fn packaged_path_relocates_with_parent_app() {
        assert_eq!(packaged_helper(Path::new("/tmp/Relocated.app/Contents/MacOS/zalo-rust")), Some(PathBuf::from("/tmp/Relocated.app/Contents/Helpers/DailyNotifier.app/Contents/MacOS/DailyNotifier")));
    }

    #[cfg(unix)]
    struct Fixture(PathBuf);
    #[cfg(unix)]
    impl Fixture {
        fn new(body: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "ctdozlite-notification-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&dir).unwrap();
            let path = dir.join("helper");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(dir)
        }
        async fn run(&self, command: &str) -> Result<AuthorizationStatus, NotificationError> {
            invoke(&self.0.join("helper"), command, Duration::from_secs(3)).await
        }
    }
    #[cfg(unix)]
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mock_executable_valid_denied_and_exact_args() {
        let fixture = Fixture::new("printf '%s\\n' \"$@\" >> \"$0.args\"; case \"$1\" in --status) echo denied;; --request) echo authorized;; *) exit 9;; esac");
        assert_eq!(
            fixture.run("--status").await,
            Ok(AuthorizationStatus::Denied)
        );
        assert_eq!(
            fixture.run("--request").await,
            Ok(AuthorizationStatus::Authorized)
        );
        assert_eq!(
            std::fs::read_to_string(fixture.0.join("helper.args")).unwrap(),
            "--status\n--request\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mock_executable_rejects_malformed_utf8_oversize_and_failed_exit() {
        for body in [
            "echo bogus".to_owned(),
            "printf '\\377'".to_owned(),
            format!("printf '{}'", "x".repeat(4097)),
            "echo authorized; exit 1".to_owned(),
        ] {
            assert_eq!(
                Fixture::new(&body).run("--status").await,
                Err(NotificationError::InvalidResponse),
                "{body}"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mock_executable_timeout_and_missing_helper() {
        assert_eq!(
            invoke(
                &Fixture::new("exec sleep 5").0.join("helper"),
                "--status",
                Duration::from_millis(100)
            )
            .await,
            Err(NotificationError::Timeout)
        );
        let fixture = Fixture::new("echo authorized");
        std::fs::remove_file(fixture.0.join("helper")).unwrap();
        assert_eq!(
            fixture.run("--status").await,
            Err(NotificationError::HelperUnavailable)
        );
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    #[tokio::test]
    async fn unsupported_platform_does_not_launch_helper() {
        assert_eq!(status().await, Err(NotificationError::HelperUnavailable));
        assert_eq!(
            request_if_user_action(true).await,
            Err(NotificationError::HelperUnavailable)
        );
    }
}
