//! Bounded release downloads, transactional installation and restart support.

use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resource limits for one release asset.
#[derive(Clone, Copy)]
pub struct DownloadLimits {
    /// Smallest accepted artifact.
    pub min_bytes: u64,
    /// Maximum buffered artifact size.
    pub max_bytes: u64,
    /// Deadline covering headers and the complete body.
    pub timeout: Duration,
}

impl Default for DownloadLimits {
    fn default() -> Self {
        Self {
            min_bytes: 512 * 1024,
            max_bytes: 512 * 1024 * 1024,
            timeout: Duration::from_secs(240),
        }
    }
}

/// Release metadata supplied by the release discovery layer.
pub struct DownloadRequest<'a> {
    /// HTTP asset location, without session cookies.
    pub url: &'a str,
    /// A single filename, never a path.
    pub asset_name: &'a str,
    /// Published SHA-256 hex digest, required before downloading.
    pub expected_sha256: Option<&'a str>,
    /// Optional size published by the release API.
    pub expected_size: Option<u64>,
    /// Resource bounds for this transfer.
    pub limits: DownloadLimits,
}

/// Explicit destination selected by the platform adapter, not remote metadata.
pub enum InstallTarget {
    /// Replace a portable executable and preserve its permissions.
    Portable(PathBuf),
    /// Save an installer for the platform launcher.
    Installer(PathBuf),
}

/// Reject absolute paths, traversal and cross-platform path separators.
///
/// # Errors
/// Returns an error unless the input is a single ordinary filename.
pub fn validate_asset_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':', '\0'])
        || name.chars().any(char::is_control)
    {
        return Err("Tên gói cập nhật không hợp lệ".into());
    }
    Ok(())
}

/// Check both the advertised URL and the URL after normalization by the HTTP client.
pub(crate) fn is_github_release_asset_url(url: &str, repo_slug: &str) -> bool {
    let prefix = format!("https://github.com/{repo_slug}/releases/download/");
    url.starts_with(&prefix)
        && reqwest::Url::parse(url).is_ok_and(|parsed| parsed.as_str().starts_with(&prefix))
}

fn is_github_redirect_url(url: &reqwest::Url, repo_slug: &str) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| {
            if host == "github.com" {
                url.path().starts_with(&format!("/{repo_slug}/"))
            } else {
                host.ends_with(".githubusercontent.com")
            }
        })
}

pub(crate) fn github_update_client(repo_slug: &str) -> reqwest::ClientBuilder {
    const MAX_REDIRECTS: usize = 3;
    let repo_slug = repo_slug.to_owned();

    reqwest::Client::builder()
        .https_only(true)
        .cookie_store(false)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > MAX_REDIRECTS {
                attempt.error("Quá nhiều lần chuyển hướng khi tải bản cập nhật")
            } else if !is_github_redirect_url(attempt.url(), &repo_slug) {
                attempt.error("Đích chuyển hướng cập nhật không được phép")
            } else {
                attempt.follow()
            }
        }))
}

fn is_sha256_hex(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

fn require_sha256(expected: Option<&str>) -> Result<&str, String> {
    let hash = expected
        .ok_or("Gói cập nhật thiếu mã SHA-256. Hãy tải thủ công từ trang phát hành trên GitHub.")?;
    if !is_sha256_hex(hash) {
        return Err(
            "Mã SHA-256 của gói cập nhật không hợp lệ. Hãy tải thủ công từ trang phát hành trên GitHub."
                .into(),
        );
    }
    Ok(hash)
}

/// Release bytes whose SHA-256 matched a published digest.
#[derive(Clone)]
pub struct VerifiedRelease(Vec<u8>);

impl VerifiedRelease {
    /// Verify bytes already in memory against the supplied SHA-256 hex digest.
    ///
    /// # Errors
    /// Returns an error for an invalid or mismatched digest.
    pub fn verify(bytes: Vec<u8>, expected_sha256: &str) -> Result<Self, String> {
        let expected = require_sha256(Some(expected_sha256))?;
        let hasher = Sha256::new_with_prefix(&bytes);
        Self::from_hashed_bytes(bytes, hasher, expected)
    }

    fn from_hashed_bytes(bytes: Vec<u8>, hasher: Sha256, expected: &str) -> Result<Self, String> {
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            return Err("Mã kiểm tra toàn vẹn SHA-256 không khớp".into());
        }
        Ok(Self(bytes))
    }

    /// Access the verified bytes for platform-specific installation.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Download and validate an asset before any installation destination is touched.
/// The caller validates the source URL and supplies a dedicated, cookie-free client.
///
/// # Errors
/// Rejects malformed metadata, HTTP failures, incomplete bodies, resource limits,
/// unsupported executable signatures, and checksum mismatches.
pub async fn download_release(
    client: &reqwest::Client,
    request: &DownloadRequest<'_>,
    mut progress: impl FnMut(u64, u64),
) -> Result<VerifiedRelease, String> {
    let expected = require_sha256(request.expected_sha256)?;
    validate_asset_name(request.asset_name)?;
    let limits = request.limits;
    if limits.min_bytes > limits.max_bytes || limits.max_bytes > usize::MAX as u64 {
        return Err("Giới hạn dung lượng cập nhật không hợp lệ".into());
    }
    if request
        .expected_size
        .is_some_and(|size| size > limits.max_bytes)
    {
        return Err("Gói cập nhật vượt giới hạn dung lượng".into());
    }
    let transfer = async {
        let mut response = client
            .get(request.url)
            .timeout(limits.timeout)
            .send()
            .await
            .map_err(|e| format!("Tải gói cập nhật thất bại: {e}"))?
            .error_for_status()
            .map_err(|e| format!("Máy chủ cập nhật trả về lỗi: {e}"))?;
        let length = response.content_length();
        if length.is_some_and(|size| size > limits.max_bytes) {
            return Err("Gói cập nhật vượt giới hạn dung lượng".into());
        }
        if let (Some(expected), Some(actual)) = (request.expected_size, length) {
            if expected != actual {
                return Err("Dung lượng gói cập nhật không khớp metadata".into());
            }
        }
        let total = request.expected_size.or(length).unwrap_or(0);
        let mut bytes = Vec::new();
        let mut hasher = Sha256::new();
        let mut last_progress = std::time::Instant::now();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("Ngắt kết nối khi đang tải cập nhật: {e}"))?
        {
            if (chunk.len() as u64) > limits.max_bytes.saturating_sub(bytes.len() as u64) {
                return Err("Gói cập nhật vượt giới hạn dung lượng".into());
            }
            hasher.update(&chunk);
            bytes.extend_from_slice(&chunk);
            if last_progress.elapsed() >= Duration::from_millis(80) {
                progress(bytes.len() as u64, total);
                last_progress = std::time::Instant::now();
            }
        }
        let actual = bytes.len() as u64;
        if actual < limits.min_bytes {
            return Err("Gói cập nhật tải về quá nhỏ".into());
        }
        if length.is_some_and(|size| size != actual)
            || request.expected_size.is_some_and(|size| size != actual)
        {
            return Err("Gói cập nhật tải về chưa hoàn tất".into());
        }
        let release = VerifiedRelease::from_hashed_bytes(bytes, hasher, expected)?;
        let name = request.asset_name.to_ascii_lowercase();
        if name.ends_with(".exe") && !release.as_bytes().starts_with(b"MZ") {
            return Err("File cập nhật không có chữ ký PE hợp lệ".into());
        }
        if name.ends_with(".appimage") && !release.as_bytes().starts_with(b"\x7fELF") {
            return Err("File cập nhật không có chữ ký ELF hợp lệ".into());
        }
        progress(actual, total.max(actual));
        Ok(release)
    };
    tokio::time::timeout(limits.timeout, transfer)
        .await
        .map_err(|_| "Tải gói cập nhật quá thời gian cho phép".to_string())?
}

static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

fn staging_file(destination: &Path) -> Result<(PathBuf, File), String> {
    let parent = destination
        .parent()
        .ok_or("Đường dẫn cập nhật thiếu thư mục")?;
    for _ in 0..100 {
        let id = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".ctdo-update-{}-{id}", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Tạo file cập nhật tạm thất bại: {error}")),
        }
    }
    Err("Không tạo được file cập nhật tạm riêng biệt".into())
}

fn remove_staging(path: &Path) {
    // Cleanup must not hide the primary download/install failure.
    let _ = fs::remove_file(path);
}

/// Strip the hidden restart flag from arguments following the executable name.
/// The last valid, nonzero PID wins. A missing value leaves the next flag intact.
pub fn take_wait_for_pid(args: Vec<String>) -> (Option<u32>, Vec<String>) {
    const FLAG: &str = "--wait-for-pid";
    const PREFIX: &str = "--wait-for-pid=";

    let mut args = args.into_iter().peekable();
    let mut forwarded = Vec::new();
    let mut pid = None;
    while let Some(arg) = args.next() {
        let parsed = if arg == FLAG {
            args.next_if(|value| {
                !value.starts_with('-') || value.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
            })
            .and_then(|value| value.parse::<u32>().ok())
        } else if let Some(value) = arg.strip_prefix(PREFIX) {
            value.parse::<u32>().ok()
        } else {
            forwarded.push(arg);
            continue;
        };
        if let Some(parsed) = parsed.filter(|pid| *pid != 0) {
            pid = Some(parsed);
        }
    }
    (pid, forwarded)
}

/// Wait up to 60 seconds for a Windows updater's parent process to exit. Returns `false` when
/// it is still running (or the wait failed), so the caller can refuse to start a second instance
/// on the same data directory.
#[cfg(windows)]
pub fn wait_for_pid(pid: u32) -> bool {
    use std::ffi::c_void;

    const SYNCHRONIZE: u32 = 0x0010_0000;
    const TIMEOUT_MS: u32 = 60_000;
    const WAIT_OBJECT_0: u32 = 0;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    // SAFETY: OpenProcess accepts integer arguments. A non-null result is an owned
    // process handle with SYNCHRONIZE access, held until the wait returns and then closed.
    unsafe {
        let handle = OpenProcess(SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            // No such process any more (same-user processes are always openable).
            return true;
        }
        let result = WaitForSingleObject(handle, TIMEOUT_MS);
        CloseHandle(handle);
        result == WAIT_OBJECT_0
    }
}

/// Other platforms strip the restart flag and use their own relaunch flows.
#[cfg(not(windows))]
pub fn wait_for_pid(_pid: u32) -> bool {
    true
}

/// Portable installation staged without replacing or launching the running executable.
/// Dropping an uncommitted transaction preserves the original executable.
pub struct DeferredPortableInstall {
    prepared: PreparedInstall,
}

impl DeferredPortableInstall {
    /// Replace the executable and launch only after the caller has joined its worker.
    ///
    /// # Errors
    /// Returns replacement, launch or explicit rollback errors.
    pub fn launch_and_finish(
        self,
        launch: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<(), String> {
        self.prepared.finish(launch)
    }

    /// Discard the staged update and preserve the original executable.
    ///
    /// # Errors
    /// Returns a rollback error if replacement happened before cancellation.
    pub fn abort(mut self) -> Result<(), String> {
        self.prepared.rollback()
    }
}

/// Stage validated portable bytes without replacing or launching any executable.
///
/// # Errors
/// Returns filesystem errors while preparing private sibling files.
pub fn stage_portable_release(
    release: &VerifiedRelease,
    destination: &Path,
) -> Result<DeferredPortableInstall, String> {
    Ok(DeferredPortableInstall {
        prepared: prepare_install(release.as_bytes(), destination, true)?,
    })
}

enum InstallBackup {
    StagedCopy(PathBuf),
    PreviousExecutable(PathBuf),
}

struct PreparedInstall {
    destination: PathBuf,
    stage: PathBuf,
    backup: Option<InstallBackup>,
    needs_rollback: bool,
    committed: bool,
}

impl PreparedInstall {
    fn finish_windows_portable(
        mut self,
        launch: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<(), String> {
        let backup = windows_portable_backup_path(&self.destination);
        // Reserve the backup name exclusively so an existing recovery copy stays intact.
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
            .map_err(|error| format!("Tạo đường dẫn sao lưu thất bại: {error}"))?;
        if let Err(error) = fs::rename(&self.destination, &backup) {
            remove_staging(&backup);
            return Err(format!("Đổi tên bản cài hiện tại thất bại: {error}"));
        }
        self.backup = Some(InstallBackup::PreviousExecutable(backup));
        self.needs_rollback = true;
        self.finish(launch)
    }

    fn finish(mut self, launch: impl FnOnce(&Path) -> Result<(), String>) -> Result<(), String> {
        if let Err(error) = fs::rename(&self.stage, &self.destination) {
            return Err(self.rollback_after_error("Thay thế gói cập nhật thất bại", error));
        }
        self.needs_rollback = true;
        if let Err(error) = launch(&self.destination) {
            return Err(self.rollback_after_error("Khởi chạy thất bại", error));
        }
        self.committed = true;
        Ok(())
    }

    fn rollback_after_error(&mut self, operation: &str, error: impl std::fmt::Display) -> String {
        if !self.needs_rollback {
            return format!("{operation}: {error}");
        }
        match self.rollback() {
            Ok(()) => format!("{operation}, đã hoàn tác cập nhật: {error}"),
            Err(rollback) => format!("{operation}: {error}; hoàn tác thất bại: {rollback}"),
        }
    }

    fn rollback(&mut self) -> Result<(), String> {
        if self.needs_rollback && !self.committed {
            match &self.backup {
                Some(
                    InstallBackup::StagedCopy(backup) | InstallBackup::PreviousExecutable(backup),
                ) => fs::rename(backup, &self.destination),
                None => fs::remove_file(&self.destination),
            }
            .map_err(|error| format!("Khôi phục gói cập nhật cũ thất bại: {error}"))?;
            self.needs_rollback = false;
        }
        Ok(())
    }
}

impl Drop for PreparedInstall {
    fn drop(&mut self) {
        // A retained backup is the recovery source if rollback itself fails.
        if self.rollback().is_err() {
            return;
        }
        remove_staging(&self.stage);
        if let Some(InstallBackup::StagedCopy(backup)) = &self.backup {
            remove_staging(backup);
        }
    }
}

fn write_staged_release(
    bytes: &[u8],
    destination: &Path,
    permissions: Option<fs::Permissions>,
) -> Result<PathBuf, String> {
    let (stage, mut file) = staging_file(destination)?;
    let written = (|| {
        file.write_all(bytes)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = written {
        remove_staging(&stage);
        return Err(format!("Ghi file cập nhật thất bại: {error}"));
    }
    Ok(stage)
}

fn prepare_install(
    bytes: &[u8],
    destination: &Path,
    portable: bool,
) -> Result<PreparedInstall, String> {
    let metadata = match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_file() => Some(metadata),
        Ok(_) => return Err("Đích cập nhật không phải file thông thường".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !portable => None,
        Err(error) => return Err(format!("Đọc đích cập nhật thất bại: {error}")),
    };
    let permissions = metadata
        .as_ref()
        .filter(|_| portable)
        .map(fs::Metadata::permissions);
    let stage = write_staged_release(bytes, destination, permissions)?;
    let backup = if metadata.is_some() {
        let (path, file) = match staging_file(destination) {
            Ok(value) => value,
            Err(error) => {
                remove_staging(&stage);
                return Err(error);
            }
        };
        drop(file);
        // A private sibling copy keeps the original destination present until atomic replacement.
        if let Err(error) = fs::copy(destination, &path) {
            remove_staging(&stage);
            remove_staging(&path);
            return Err(format!("Sao lưu bản cập nhật cũ thất bại: {error}"));
        }
        Some(InstallBackup::StagedCopy(path))
    } else {
        None
    };
    Ok(PreparedInstall {
        destination: destination.to_path_buf(),
        stage,
        backup,
        needs_rollback: false,
        committed: false,
    })
}

/// The sibling backup kept until the replacement executable starts on Windows.
pub fn windows_portable_backup_path(destination: &Path) -> PathBuf {
    destination.with_extension("exe.old")
}

fn prepare_windows_portable_install(
    release: &VerifiedRelease,
    destination: &Path,
) -> Result<PreparedInstall, String> {
    let metadata = fs::symlink_metadata(destination)
        .map_err(|error| format!("Đọc đích cập nhật thất bại: {error}"))?;
    if !metadata.is_file() {
        return Err("Đích cập nhật không phải file thông thường".into());
    }
    let stage = write_staged_release(release.as_bytes(), destination, None)?;
    Ok(PreparedInstall {
        destination: destination.to_path_buf(),
        stage,
        backup: None,
        needs_rollback: false,
        committed: false,
    })
}

/// Handle a Windows portable update, returning `Ok(false)` for installer assets.
///
/// # Errors
/// A portable install error is returned to the caller so only installers can fall through.
pub fn try_install_windows_portable_release(
    lower_asset_name: &str,
    release: &VerifiedRelease,
    current_exe: impl FnOnce() -> std::io::Result<PathBuf>,
    launch: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<bool, String> {
    if !lower_asset_name.ends_with(".exe") || lower_asset_name.contains("setup") {
        return Ok(false);
    }
    let destination = current_exe()
        .map_err(|error| format!("Không xác định được đường dẫn bản cài hiện tại: {error}"))?;
    let dir = destination.parent().unwrap_or(destination.as_path());
    prepare_windows_portable_install(release, &destination)
        .and_then(|prepared| prepared.finish_windows_portable(launch))
        .map_err(|error| {
            format!(
                "Không thể tự thay bản cài trong {} (có thể cần quyền quản trị). Hãy tải bộ cài mới từ trang phát hành trên GitHub.\n{error}",
                dir.display()
            )
        })?;
    Ok(true)
}

/// Atomically install already-validated bytes and invoke the injected launcher.
/// Existing bytes are restored if replacement or launch fails. A rollback failure
/// is returned explicitly and the backup is retained for recovery.
///
/// # Errors
/// Returns filesystem or launcher errors; never falls back to another destination.
pub fn install_release(
    release: &VerifiedRelease,
    target: &InstallTarget,
    launch: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let (destination, portable) = match target {
        InstallTarget::Portable(path) => (path, true),
        InstallTarget::Installer(path) => (path, false),
    };
    prepare_install(release.as_bytes(), destination, portable)?.finish(launch)
}

/// Metadata parsed from GitHub Releases REST API (`/repos/{owner}/{repo}/releases/latest`)
/// or the static `latest.json` release manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubReleaseMetadata {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub release_title: String,
    pub release_notes: String,
    pub html_url: String,
    pub download_url: Option<String>,
    pub asset_name: Option<String>,
    pub asset_size: u64,
    pub expected_sha256: Option<String>,
    pub published_at: String,
    pub sha256sums_url: Option<String>,
}

/// Compare semantic versions (`"v0.0.4"` > `"0.0.3"`).
pub fn is_newer_semver(remote: &str, current: &str) -> bool {
    let parse = |s: &str| -> (u64, u64, u64, bool) {
        let trimmed = s.trim().trim_start_matches(['v', 'V']);
        let (core, rest) = trimmed
            .split_once(['-', '+'])
            .map_or((trimmed, ""), |(c, r)| (c, r));
        let is_stable = !trimmed.contains('-') || rest.is_empty();
        let mut parts = core.split('.').filter_map(|p| p.trim().parse::<u64>().ok());
        (
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            is_stable,
        )
    };
    parse(remote) > parse(current)
}

/// Score a release asset filename for a target `(os, arch)` pair.
/// Returns `<= 0` when the asset is incompatible with the platform.
pub fn select_platform_asset_score(name: &str, target_os: &str, target_arch: &str) -> i32 {
    let n_low = name.to_ascii_lowercase();
    if n_low.is_empty()
        || n_low.ends_with(".txt")
        || n_low.ends_with(".json")
        || n_low.ends_with(".sig")
        || n_low.ends_with(".asc")
    {
        return -1;
    }
    match target_os {
        "windows" => {
            if n_low.ends_with(".exe") && n_low.contains("portable") {
                110
            } else if n_low.ends_with(".exe") && n_low.contains("setup") {
                90
            } else if n_low.ends_with(".exe") {
                80
            } else {
                0
            }
        }
        "macos" => {
            if !n_low.ends_with(".dmg") {
                return 0;
            }
            if target_arch == "aarch64" && (n_low.contains("arm64") || n_low.contains("aarch64")) {
                110
            } else if target_arch == "x86_64"
                && (n_low.contains("x86_64") || n_low.contains("intel"))
            {
                110
            } else if !n_low.contains("arm64")
                && !n_low.contains("aarch64")
                && !n_low.contains("x86_64")
                && !n_low.contains("intel")
            {
                100
            } else {
                50
            }
        }
        _ => {
            if n_low.ends_with(".appimage") {
                100
            } else if n_low.ends_with(".deb") && !n_low.contains("legacy") {
                90
            } else if n_low.ends_with(".tar.gz") && !n_low.contains("legacy") {
                80
            } else if n_low.ends_with(".rpm") {
                70
            } else {
                0
            }
        }
    }
}

/// Extract a 64-character lowercase SHA-256 hex digest from `SHA256SUMS.txt` content for `asset_name`.
pub fn parse_sha256sums_for_asset(sha256sums_text: &str, asset_name: &str) -> Option<String> {
    for line in sha256sums_text.lines() {
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else { continue };
        let Some(fname) = parts.next() else { continue };
        let clean_fname = fname.trim_start_matches('*').trim_start_matches("./");
        if clean_fname == asset_name && is_sha256_hex(hash) {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

/// Parse either a GitHub Releases API JSON object (`/releases/latest`) or a `latest.json`
/// manifest object into [`GitHubReleaseMetadata`].
pub fn parse_github_release_value(
    rel: &serde_json::Value,
    current_ver: &str,
    repo_slug: &str,
    target_os: &str,
    target_arch: &str,
) -> GitHubReleaseMetadata {
    let raw_tag = rel
        .get("tag_name")
        .or_else(|| rel.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or(current_ver);
    let latest_ver = raw_tag.trim().trim_start_matches(['v', 'V']).to_string();
    let has_update = is_newer_semver(&latest_ver, current_ver);

    let release_title = rel
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Zalo-next v{latest_ver}"));

    let release_notes = rel
        .get("body")
        .or_else(|| rel.get("notes"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let html_url = rel
        .get("html_url")
        .or_else(|| rel.get("release_url"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://github.com/{repo_slug}/releases/tag/v{latest_ver}"));

    let published_at = rel
        .get("published_at")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .split('T')
        .next()
        .unwrap_or("")
        .to_string();

    let mut best_url: Option<String> = None;
    let mut best_name: Option<String> = None;
    let mut best_size: u64 = 0;
    let mut best_sha256: Option<String> = None;
    let mut best_score: i32 = 0;
    let mut sha256sums_url: Option<String> = None;

    if let Some(assets) = rel.get("assets").and_then(|v| v.as_array()) {
        for a in assets {
            let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let dl = a
                .get("browser_download_url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sz = a.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            if !is_github_release_asset_url(dl, repo_slug) {
                continue;
            }
            if name.eq_ignore_ascii_case("SHA256SUMS.txt") {
                sha256sums_url = Some(dl.to_string());
                continue;
            }
            let score = select_platform_asset_score(name, target_os, target_arch);
            if score > best_score {
                best_score = score;
                best_url = Some(dl.to_string());
                best_name = Some(name.to_string());
                best_size = sz;
                // Extract SHA-256 from `latest.json` (`sha256`) or GitHub API asset (`digest: "sha256:..."`)
                best_sha256 = a
                    .get("sha256")
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        a.get("digest")
                            .and_then(|v| v.as_str())
                            .and_then(|d| d.strip_prefix("sha256:"))
                    })
                    .filter(|h| is_sha256_hex(h))
                    .map(|h| h.to_ascii_lowercase());
            }
        }
    }

    // Legacy single-asset `latest.json` fallback when `assets` array is absent
    if best_url.is_none() && rel.get("assets").is_none() {
        let fallback_name = match target_os {
            "windows" => format!("zalo-next-{latest_ver}-portable-windows-x86_64.exe"),
            "macos" => match target_arch {
                "aarch64" => format!("zalo-next-{latest_ver}-macos-arm64.dmg"),
                "x86_64" => format!("zalo-next-{latest_ver}-macos-x86_64.dmg"),
                _ => format!("zalo-next-{latest_ver}-macos.dmg"),
            },
            _ => format!("zalo-next-{latest_ver}-x86_64.AppImage"),
        };
        let fallback_url = format!(
            "https://github.com/{repo_slug}/releases/download/v{latest_ver}/{fallback_name}"
        );
        if is_github_release_asset_url(&fallback_url, repo_slug) {
            best_url = Some(fallback_url);
            best_name = Some(fallback_name);
            best_sha256 = rel
                .get("sha256")
                .and_then(|v| v.as_str())
                .filter(|h| is_sha256_hex(h))
                .map(|h| h.to_ascii_lowercase());
        }
    }

    GitHubReleaseMetadata {
        current_version: current_ver.to_string(),
        latest_version: latest_ver,
        has_update,
        release_title,
        release_notes,
        html_url,
        download_url: best_url,
        asset_name: best_name,
        asset_size: best_size,
        expected_sha256: best_sha256,
        published_at,
        sha256sums_url,
    }
}

/// The `.app` bundle this process runs from (`X.app/Contents/MacOS/<exe>`), if any.
#[cfg(target_os = "macos")]
pub fn current_app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let macos_dir = exe.parent()?;
    let contents = macos_dir.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos_dir.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_path_buf())
}

/// Detaches a mounted disk image when dropped, so every error path cleans up.
#[cfg(target_os = "macos")]
struct MountedDmg(PathBuf);

#[cfg(target_os = "macos")]
impl Drop for MountedDmg {
    fn drop(&mut self) {
        let _ = std::process::Command::new("/usr/bin/hdiutil")
            .args(["detach", "-quiet", "-force"])
            .arg(&self.0)
            .status();
    }
}

#[cfg(target_os = "macos")]
fn run_tool(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let status = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("Không chạy được {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} thất bại ({status})"))
    }
}

/// Replace the running `.app` bundle with the one inside a verified release `.dmg`.
///
/// Mount the verified DMG read-only, verify the new app's code signature, copy it
/// next to the current bundle with `ditto`, then swap by renaming
/// (current → backup, new → current). Any failure before the swap leaves the installed app
/// untouched; a failed second rename restores the backup.
#[cfg(target_os = "macos")]
pub fn install_macos_app_from_dmg(release: &VerifiedRelease, bundle: &Path) -> Result<(), String> {
    use std::ffi::OsStr;
    let parent = bundle
        .parent()
        .ok_or_else(|| "Không xác định được thư mục chứa ứng dụng".to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let work =
        std::env::temp_dir().join(format!("ctdozlite-update-{}-{nonce}", std::process::id()));
    crate::data_dir::ensure_private_dir(&work).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        let dmg = work.join("update.dmg");
        std::fs::write(&dmg, release.as_bytes())
            .map_err(|e| format!("Không ghi được tệp DMG: {e}"))?;
        let mount = work.join("mnt");
        std::fs::create_dir_all(&mount).map_err(|e| e.to_string())?;
        run_tool(
            "/usr/bin/hdiutil",
            &[
                OsStr::new("attach"),
                OsStr::new("-nobrowse"),
                OsStr::new("-noautoopen"),
                OsStr::new("-readonly"),
                OsStr::new("-mountpoint"),
                mount.as_os_str(),
                dmg.as_os_str(),
            ],
        )?;
        let _mounted = MountedDmg(mount.clone());
        let new_app = std::fs::read_dir(&mount)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.extension().is_some_and(|x| x == "app") && p.join("Contents/Info.plist").is_file()
            })
            .ok_or_else(|| "Không tìm thấy ứng dụng .app trong DMG".to_string())?;
        run_tool(
            "/usr/bin/codesign",
            &[
                OsStr::new("--verify"),
                OsStr::new("--deep"),
                OsStr::new("--strict"),
                new_app.as_os_str(),
            ],
        )
        .map_err(|e| format!("Chữ ký ứng dụng mới không hợp lệ: {e}"))?;

        let staging = parent.join(".zalo-next-update-staging.app");
        let backup = parent.join(".zalo-next-update-backup.app");
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir_all(&backup);
        run_tool(
            "/usr/bin/ditto",
            &[new_app.as_os_str(), staging.as_os_str()],
        )
        .map_err(|e| format!("Không chép được ứng dụng mới (thiếu quyền ghi?): {e}"))?;
        if let Err(e) = std::fs::rename(bundle, &backup) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(format!("Không thay được ứng dụng hiện tại: {e}"));
        }
        if let Err(e) = std::fs::rename(&staging, bundle) {
            let _ = std::fs::rename(&backup, bundle);
            let _ = std::fs::remove_dir_all(&staging);
            return Err(format!("Không cài được ứng dụng mới: {e}"));
        }
        let _ = std::fs::remove_dir_all(&backup);
        let _ = run_tool(
            "/usr/bin/xattr",
            &[
                OsStr::new("-dr"),
                OsStr::new("com.apple.quarantine"),
                bundle.as_os_str(),
            ],
        );
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Re-open `bundle` once this process has exited (so two instances never share the data dir).
#[cfg(target_os = "macos")]
pub fn relaunch_app_bundle_after_exit(bundle: &Path) -> Result<(), String> {
    let pid = std::process::id().to_string();
    std::process::Command::new("/bin/sh")
        .args([
            "-c",
            "while /bin/kill -0 \"$1\" 2>/dev/null; do /bin/sleep 0.2; done; /usr/bin/open \"$2\"",
            "ctdozlite-relaunch",
            &pid,
        ])
        .arg(bundle)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Không khởi động lại được ứng dụng: {e}"))
}

/// Title and body of the "new release available" OS notification.
pub fn new_update_notification(latest_version: &str, current_version: &str) -> (String, String) {
    (
        format!("{} — có bản cập nhật v{latest_version}", crate::APP_DISPLAY_NAME),
        format!(
            "Phiên bản mới v{latest_version} đã sẵn sàng (đang dùng v{current_version}). Mở Cài đặt để cập nhật."
        ),
    )
}

#[cfg(test)]
mod portable_update_tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn wait_for_pid_strips_both_spellings_and_preserves_other_args() {
        assert_eq!(
            take_wait_for_pid(args(&[
                "--gui",
                "--wait-for-pid",
                "41",
                "--data-dir",
                "custom dir",
                "--wait-for-pid=42",
            ])),
            (Some(42), args(&["--gui", "--data-dir", "custom dir"]))
        );
    }

    #[test]
    fn wait_for_pid_does_not_accumulate_on_relaunch() {
        let (_, forwarded) = take_wait_for_pid(args(&["--wait-for-pid=41", "--gui"]));
        let relaunched = [args(&["--wait-for-pid", "42"]), forwarded].concat();
        let (pid, forwarded) = take_wait_for_pid(relaunched);
        assert_eq!(pid, Some(42));
        assert_eq!(take_wait_for_pid(forwarded), (None, args(&["--gui"])));
    }

    #[test]
    fn wait_for_pid_strips_malformed_values_and_keeps_following_flags() {
        assert_eq!(
            take_wait_for_pid(args(&[
                "--wait-for-pid",
                "oops",
                "--wait-for-pid=4294967296",
                "--wait-for-pid=0",
                "--wait-for-pid",
                "-1",
                "--wait-for-pid",
                "--wait-for-pid=",
                "--wait-for-pid",
                "--version",
                "--wait-for-pid",
            ])),
            (None, args(&["--version"]))
        );
    }

    #[test]
    fn windows_portable_install_restores_original_when_staged_rename_fails() {
        let sandbox = crate::data_dir::PrivateTempDir::new("updater-swap").unwrap();
        let destination = sandbox.path().join("application.exe");
        fs::write(&destination, b"original").unwrap();
        let release = VerifiedRelease::verify(
            b"replacement".to_vec(),
            &format!("{:x}", Sha256::digest(b"replacement")),
        )
        .unwrap();
        let pending = prepare_windows_portable_install(&release, &destination).unwrap();
        fs::remove_file(&pending.stage).unwrap();

        let error = pending
            .finish_windows_portable(|_| panic!("failed swap must never launch"))
            .unwrap_err();

        assert!(error.contains("đã hoàn tác"));
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        assert_eq!(fs::read_dir(sandbox.path()).unwrap().count(), 1);
    }
}

#[cfg(test)]
mod github_api_tests {
    use super::*;

    const REPO: &str = "minhkhang1008/zalo-next";

    #[tokio::test]
    async fn missing_sha256_refuses_download_before_any_platform_install() {
        let client = reqwest::Client::new();
        for asset_name in [
            "release-macos-arm64.dmg",
            "release-portable-windows-x86_64.exe",
            "release-setup-windows-x86_64.exe",
            "release-x86_64.AppImage",
            "release-amd64.deb",
        ] {
            let request = DownloadRequest {
                url: "invalid URL",
                asset_name,
                expected_sha256: None,
                expected_size: None,
                limits: DownloadLimits::default(),
            };
            let result = download_release(&client, &request, |_, _| {
                panic!("an update without a checksum must never report download progress");
            })
            .await;
            assert!(
                matches!(result, Err(ref error) if error.contains("SHA-256")
                    && error.contains("tải thủ công từ trang phát hành")),
                "expected a checksum error with manual download instructions for {asset_name}"
            );
        }
    }

    #[test]
    fn release_bytes_require_a_matching_sha256() {
        let digest = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        let release = VerifiedRelease::verify(b"abc".to_vec(), digest).unwrap();
        assert_eq!(release.as_bytes(), b"abc");

        let result = VerifiedRelease::verify(b"abd".to_vec(), digest);
        assert!(matches!(result, Err(error) if error.contains("SHA-256 không khớp")));
    }

    #[test]
    fn asset_urls_require_the_repository_download_prefix() {
        for asset_name in ["release.dmg", "SHA256SUMS.txt"] {
            assert!(is_github_release_asset_url(
                &format!("https://github.com/{REPO}/releases/download/v0.4.0/{asset_name}"),
                REPO,
            ));
        }

        for url in [
            "https://github.com/minhkhang1008/zalo-next-evil/releases/download/v1/release.dmg",
            "http://github.com/minhkhang1008/zalo-next/releases/download/v1/release.dmg",
            "https://github.com.example.org/minhkhang1008/zalo-next/releases/download/v1/release.dmg",
            "https://github.com@evil.example/minhkhang1008/zalo-next/releases/download/v1/release.dmg",
            "https://release-assets.githubusercontent.com/minhkhang1008/zalo-next/releases/download/v1/release.dmg",
            "https://github.com/minhkhang1008/zalo-next/releases/downloads/v1/release.dmg",
            "https://github.com/minhkhang1008/zalo-next/releases/download/%2e%2e/tag/v1",
        ] {
            assert!(!is_github_release_asset_url(url, REPO), "accepted {url}");
        }
    }

    #[test]
    fn release_metadata_discards_untrusted_asset_and_checksum_urls() {
        let html_url = "https://github.com/minhkhang1008/zalo-next/releases/tag/v0.4.0";
        let json = serde_json::json!({
            "tag_name": "v0.4.0",
            "html_url": html_url,
            "assets": [
                {
                    "name": "release-arm64.dmg",
                    "browser_download_url": "https://github.com/minhkhang1008/zalo-next-evil/releases/download/v0.4.0/release-arm64.dmg",
                    "sha256": "2222222222222222222222222222222222222222222222222222222222222222"
                },
                {
                    "name": "SHA256SUMS.txt",
                    "browser_download_url": "https://example.org/SHA256SUMS.txt"
                }
            ]
        });
        let meta = parse_github_release_value(&json, "0.3.0", REPO, "macos", "aarch64");
        assert!(meta.has_update);
        assert_eq!(meta.html_url, html_url);
        assert!(meta.download_url.is_none());
        assert!(meta.expected_sha256.is_none());
        assert!(meta.sha256sums_url.is_none());
    }

    #[test]
    fn redirect_targets_require_https_and_the_repository_or_an_allowed_cdn() {
        for url in [
            "https://github.com/minhkhang1008/zalo-next/releases/latest",
            "https://objects.githubusercontent.com/asset",
            "https://release-assets.githubusercontent.com/asset",
            "https://another.cdn.githubusercontent.com/asset",
        ] {
            assert!(is_github_redirect_url(
                &reqwest::Url::parse(url).unwrap(),
                REPO
            ));
        }
        for url in [
            "http://github.com/minhkhang1008/zalo-next/releases/latest",
            "https://github.com/another-owner/zalo-next/releases/latest",
            "https://github.com/minhkhang1008/another-repo/releases/latest",
            "https://github.com/minhkhang1008/zalo-next-evil/releases/latest",
            "https://github.com/minhkhang1008/zalo-next/%2e%2e/another-repo/releases/latest",
            "http://objects.githubusercontent.com/asset",
            "https://api.github.com/release",
            "https://githubusercontent.com/asset",
            "https://evilgithubusercontent.com/asset",
            "https://objects.githubusercontent.com.evil.example/asset",
            "https://github.com@evil.example/asset",
            "https://127.0.0.1/asset",
        ] {
            assert!(
                !is_github_redirect_url(&reqwest::Url::parse(url).unwrap(), REPO),
                "accepted {url}"
            );
        }
    }

    #[test]
    fn test_is_newer_semver_comparisons() {
        assert!(is_newer_semver("0.0.4", "0.0.3"));
        assert!(is_newer_semver("v0.0.4", "0.0.3"));
        assert!(is_newer_semver("v0.1.0", "v0.0.9"));
        assert!(!is_newer_semver("0.0.3", "0.0.3"));
        assert!(!is_newer_semver("0.0.2", "0.0.3"));
        assert!(is_newer_semver("0.0.4", "0.0.4-rc.1"));
    }

    #[test]
    fn test_parse_github_release_api_payload_with_native_digest() {
        let api_json = serde_json::json!({
            "tag_name": "v0.0.4",
            "name": "ctdozlite v0.0.4",
            "body": "- Sửa lỗi Windows Defender\n- Tự động thông báo cập nhật qua GitHub API",
            "html_url": "https://github.com/ctdoteam1110/ctdozlite/releases/tag/v0.0.4",
            "published_at": "2026-11-02T03:00:00Z",
            "assets": [
                {
                    "name": "SHA256SUMS.txt",
                    "size": 512,
                    "browser_download_url": "https://github.com/ctdoteam1110/ctdozlite/releases/download/v0.0.4/SHA256SUMS.txt"
                },
                {
                    "name": "ctdozlite-0.0.4-setup-windows-x86_64.exe",
                    "size": 9500000,
                    "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
                    "browser_download_url": "https://github.com/ctdoteam1110/ctdozlite/releases/download/v0.0.4/ctdozlite-0.0.4-setup-windows-x86_64.exe"
                },
                {
                    "name": "ctdozlite-0.0.4-portable-windows-x86_64.exe",
                    "size": 8400000,
                    "digest": "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                    "browser_download_url": "https://github.com/ctdoteam1110/ctdozlite/releases/download/v0.0.4/ctdozlite-0.0.4-portable-windows-x86_64.exe"
                },
                {
                    "name": "ctdozlite-0.0.4-macos-arm64.dmg",
                    "size": 10200000,
                    "digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                    "browser_download_url": "https://github.com/ctdoteam1110/ctdozlite/releases/download/v0.0.4/ctdozlite-0.0.4-macos-arm64.dmg"
                }
            ]
        });

        let win_meta = parse_github_release_value(
            &api_json,
            "0.0.3",
            "ctdoteam1110/ctdozlite",
            "windows",
            "x86_64",
        );
        assert!(win_meta.has_update);
        assert_eq!(win_meta.latest_version, "0.0.4");
        assert_eq!(
            win_meta.asset_name.as_deref(),
            Some("ctdozlite-0.0.4-portable-windows-x86_64.exe")
        );
        assert_eq!(
            win_meta.expected_sha256.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
        );
        assert_eq!(win_meta.published_at, "2026-11-02");
        assert_eq!(
            win_meta.sha256sums_url.as_deref(),
            Some(
                "https://github.com/ctdoteam1110/ctdozlite/releases/download/v0.0.4/SHA256SUMS.txt"
            )
        );

        let mac_meta = parse_github_release_value(
            &api_json,
            "0.0.3",
            "ctdoteam1110/ctdozlite",
            "macos",
            "aarch64",
        );
        assert_eq!(
            mac_meta.asset_name.as_deref(),
            Some("ctdozlite-0.0.4-macos-arm64.dmg")
        );
        assert_eq!(
            mac_meta.expected_sha256.as_deref(),
            Some("2222222222222222222222222222222222222222222222222222222222222222")
        );
    }

    #[test]
    fn test_parse_sha256sums_for_asset() {
        let sums = "\
abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789  ctdozlite-0.0.4-portable-windows-x86_64.exe\n\
2222222222222222222222222222222222222222222222222222222222222222 *./ctdozlite-0.0.4-macos-arm64.dmg\n";
        assert_eq!(
            parse_sha256sums_for_asset(sums, "ctdozlite-0.0.4-macos-arm64.dmg").as_deref(),
            Some("2222222222222222222222222222222222222222222222222222222222222222")
        );
    }
}
