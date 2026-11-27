//! Select writable storage before opening sessions, caches, logs, or databases.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

/// Data directory used on macOS: `~/Library/Application Support/ctdozlite`. The folder keeps the
/// legacy name on purpose, so existing logins and databases survive the Zalo-next rename.
pub fn macos_default_dir(home: &Path) -> PathBuf {
    home.join("Library/Application Support/ctdozlite")
}

/// Platform paths supplied by the caller, independently of the test or build host.
#[derive(Clone, Copy)]
pub enum Platform<'a> {
    MacOs(Option<&'a Path>),
    Windows(Option<&'a Path>),
    Other,
}

/// Resolve paths without changing process state, including in parallel tests.
///
/// # Errors
/// Returns an error for invalid storage options or a missing/relative bundle HOME.
pub fn resolve(
    args: &[String],
    cwd: &Path,
    executable: &Path,
    platform: Platform<'_>,
) -> Result<(PathBuf, bool)> {
    let mut explicit = None;
    let mut profile_name = None;
    let mut i = 0;
    while i < args.len() {
        let value = if args[i] == "--data-dir" {
            i += 1;
            Some(
                args.get(i)
                    .filter(|value| !value.starts_with('-'))
                    .context("--data-dir yêu cầu đường dẫn")?
                    .as_str(),
            )
        } else {
            args[i].strip_prefix("--data-dir=")
        };
        if let Some(value) = value {
            if value.is_empty() {
                bail!("--data-dir yêu cầu đường dẫn không rỗng");
            }
            if explicit.is_some() {
                bail!("--data-dir chỉ được chỉ định một lần");
            }
            explicit = Some(PathBuf::from(value));
            i += 1;
            continue;
        }
        let prof_val = if args[i] == "--profile" {
            i += 1;
            Some(
                args.get(i)
                    .filter(|value| !value.starts_with('-'))
                    .context("--profile yêu cầu tên hồ sơ tài khoản")?
                    .as_str(),
            )
        } else {
            args[i].strip_prefix("--profile=")
        };
        if let Some(prof) = prof_val {
            let safe: String = prof
                .trim()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if safe.is_empty() {
                bail!("--profile chỉ chấp nhận ký tự chữ, số, '-' hoặc '_'");
            }
            profile_name = Some(safe);
        }
        i += 1;
    }
    if let Some(path) = explicit {
        return Ok((
            if path.is_absolute() || path.to_str().is_some_and(|s| s.starts_with('/')) {
                path
            } else {
                cwd.join(path)
            },
            true,
        ));
    }
    let bundled = executable
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "MacOS"))
        .and_then(Path::parent)
        .filter(|p| p.file_name().is_some_and(|n| n == "Contents"))
        .and_then(Path::parent)
        .is_some_and(|p| p.extension().is_some_and(|e| e == "app"));
    // On macOS every run (the .app, `cargo run`, the TUI, helper CLIs) shares one private data
    // directory, so they all see the same session and message database. Only a developer run
    // without HOME falls back to the working directory; a bundle without HOME is an error.
    let (base, is_private) = match platform {
        Platform::MacOs(Some(home)) => {
            if !home.is_absolute() && !home.to_str().is_some_and(|s| s.starts_with('/')) {
                bail!("HOME phải là đường dẫn tuyệt đối");
            }
            (macos_default_dir(home), true)
        }
        Platform::MacOs(None) if bundled => bail!("Cần HOME để xác định Application Support"),
        Platform::Windows(Some(local_app_data)) if local_app_data.is_absolute() => {
            (local_app_data.join("ctdozlite"), true)
        }
        _ => (cwd.to_path_buf(), false),
    };
    if let Some(prof) = profile_name {
        return Ok((base.join("profiles").join(prof), true));
    }
    Ok((base, is_private))
}

/// Validate storage before any persistent state is opened.
///
/// # Errors
/// Returns an error if creation, permissions, or the writable probe fails.
pub fn prepare(path: &Path, private: bool) -> Result<()> {
    if private {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            let mut builder = std::fs::DirBuilder::new();
            builder
                .recursive(true)
                .mode(0o700)
                .create(path)
                .with_context(|| format!("Không tạo được thư mục dữ liệu {}", path.display()))?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).with_context(
                || format!("Không đặt được quyền thư mục dữ liệu {}", path.display()),
            )?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(path)
            .with_context(|| format!("Không tạo được thư mục dữ liệu {}", path.display()))?;
    }
    let (probe_path, probe) = create_private_file(path, ".storage-probe")
        .with_context(|| format!("Thư mục dữ liệu không ghi được: {}", path.display()))?;
    drop(probe);
    std::fs::remove_file(&probe_path).context("Không xóa được tệp kiểm tra quyền ghi")?;
    Ok(())
}

/// Select storage once before workers or persistent state are opened.
///
/// # Errors
/// Returns an error when process paths or writable storage cannot be selected.
pub fn initialize(args: &[String]) -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("Không xác định được thư mục làm việc")?;
    let executable =
        std::env::current_exe().context("Không xác định được đường dẫn chương trình")?;
    #[cfg(target_os = "macos")]
    let home = std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let platform = Platform::MacOs(home.as_deref());
    #[cfg(windows)]
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(windows)]
    let platform = Platform::Windows(local_app_data.as_deref());
    #[cfg(not(any(target_os = "macos", windows)))]
    let platform = Platform::Other;
    let (path, private) = resolve(args, &cwd, &executable, platform)?;
    let print_only = args.iter().any(|arg| arg == "--print-data-dir");
    let explicit = args
        .iter()
        .any(|arg| arg == "--data-dir" || arg.starts_with("--data-dir="));
    #[cfg(windows)]
    if print_only && !explicit {
        // Report the resolved path without creating the new per-user directory.
        return Ok(path);
    }
    let profile = args
        .iter()
        .any(|arg| arg == "--profile" || arg.starts_with("--profile="));
    #[cfg(windows)]
    let selected = if private && !print_only && !explicit {
        migrate_windows_storage(
            &cwd,
            &executable,
            &path,
            profile,
            || other_instance_running_after_wait(&executable),
            probe_legacy_db,
        )
    } else {
        path.clone()
    };
    #[cfg(not(windows))]
    let selected = path.clone();
    prepare(&selected, private && selected == path)?;
    if matches!(platform, Platform::MacOs(_)) && private && !print_only && !explicit && !profile {
        migrate_and_report(&cwd, &path);
    }
    // Legacy storage callers use relative paths; switch once before creating workers.
    std::env::set_current_dir(&selected)
        .with_context(|| format!("Không chọn được thư mục dữ liệu {}", selected.display()))?;
    Ok(selected)
}

fn migrate_and_report(source: &Path, target: &Path) -> MigrationReport {
    let report = migrate_legacy_cwd_storage(source, target);
    if !report.moved.is_empty() && report.errors.is_empty() {
        eprintln!(
            "ℹ Đã chuyển dữ liệu cũ từ {} sang {}: {} (bản gốc đổi tên thành *.bak).",
            source.display(),
            target.display(),
            report.moved.join(", ")
        );
    }
    for error in &report.errors {
        eprintln!("! Không chuyển được dữ liệu cũ: {error}");
    }
    report
}

#[cfg(any(windows, test))]
fn migrate_windows_storage(
    cwd: &Path,
    executable: &Path,
    target: &Path,
    profile: bool,
    other_instance_running: impl FnOnce() -> bool,
    probe_db: impl FnOnce(&Path) -> std::io::Result<()>,
) -> PathBuf {
    let source = std::iter::once(cwd)
        .chain(executable.parent())
        .filter_map(|base| {
            if profile {
                let profiles = base.join("profiles");
                let source = profiles.join(target.file_name()?);
                // A direct profile migration must skip the same links as a tree copy.
                if !is_legacy_item(&profiles, true) || !is_legacy_item(&source, true) {
                    return None;
                }
                Some(source)
            } else {
                Some(base.to_path_buf())
            }
        })
        .filter(|source| can_migrate_legacy_storage(source, target))
        .enumerate()
        .max_by_key(|(index, source)| (legacy_storage_priority(source), std::cmp::Reverse(*index)));
    let Some((_, source)) = source else {
        return target.to_path_buf();
    };
    if other_instance_running() {
        eprintln!("! Chưa thể xác nhận phiên bản khác đã đóng. Sẽ thử chuyển dữ liệu vào lần khởi động sau.");
        return source;
    }
    let db = source.join("zalo_history.db");
    if is_legacy_item(&db, false)
        && matches!(probe_db(&db), Err(error) if error.kind() != std::io::ErrorKind::NotFound)
    {
        eprintln!("! Không thể mở độc quyền cơ sở dữ liệu cũ. Sẽ thử chuyển dữ liệu vào lần khởi động sau.");
        return source;
    }
    let report = migrate_and_report(&source, target);
    if report.errors.is_empty() || !report.moved.is_empty() {
        target.to_path_buf()
    } else {
        source
    }
}

#[cfg(windows)]
fn other_instance_running_after_wait(executable: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use std::time::{Duration, Instant};

    let Some(name) = executable.file_name() else {
        return true;
    };
    let name: Vec<u16> = name.encode_wide().collect();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match snapshot_has_other_instance(&name) {
            Ok(false) => return false,
            Ok(true) => {}
            Err(error) => {
                eprintln!("! Không kiểm tra được các phiên bản đang chạy: {error:#}");
                return true;
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        std::thread::sleep(remaining.min(Duration::from_millis(250)));
    }
}

#[cfg(windows)]
fn snapshot_has_other_instance(name: &[u16]) -> windows::core::Result<bool> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES};
    use windows::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    // SAFETY: For a system-wide process snapshot the second argument is ignored.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)? };
    let own_pid = std::process::id();
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: The live snapshot and the writable entry with its size set satisfy ToolHelp.
    let mut next = unsafe { Process32FirstW(snapshot, &mut entry) };
    let mut found = false;
    while next.is_ok() {
        let end = entry
            .szExeFile
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(entry.szExeFile.len());
        // SAFETY: Both UTF-16 slices are valid for the lengths passed to Windows.
        if entry.th32ProcessID != own_pid
            && unsafe { CompareStringOrdinal(&entry.szExeFile[..end], name, true) } == CSTR_EQUAL
        {
            found = true;
            break;
        }
        // SAFETY: The same live snapshot and writable entry advance the enumeration.
        next = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    // SAFETY: This function owns the handle and closes it once after enumeration.
    let closed = unsafe { CloseHandle(snapshot) };
    if let Err(error) = next {
        if error.code() != ERROR_NO_MORE_FILES.to_hresult() {
            return Err(error);
        }
    }
    closed?;
    Ok(found)
}

#[cfg(windows)]
fn probe_legacy_db(path: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    // The updater may relaunch us before the old build closes its WAL-mode database.
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
        .map(drop)
}

/// Files that older builds wrote next to the working directory. SQLite sidecars travel with the
/// database so a WAL-mode history is copied consistently.
const LEGACY_FILES: &[&str] = &[
    "session.json",
    "zalo_history.db",
    "zalo_history.db-wal",
    "zalo_history.db-shm",
    "threads_cache.json",
    "app_settings.json",
    "theme.json",
    "custom_stickers.json",
    "autoreply_rules.json",
    "active_account",
    "sync_identity.json",
    "sync_rsa_identity.json",
    "sync_v2_keys.json",
];
/// Per-account data and named profiles.
const LEGACY_DIRS: &[&str] = &["accounts", "profiles"];
/// Legacy anchors, ordered from strongest to weakest, with their expected types.
const LEGACY_ANCHORS: &[(&str, bool)] = &[
    ("session.json", false),
    ("accounts", true),
    ("zalo_history.db", false),
];
const MIGRATION_STAGING: &str = ".migration-staging";
const PUBLISH_LIST: &str = "publish-list";

/// Result of [`migrate_legacy_cwd_storage`].
#[derive(Debug, Default)]
pub struct MigrationReport {
    /// Names committed to the new data directory.
    pub moved: Vec<String>,
    /// Migration or backup failures. An empty report means there was nothing to migrate.
    pub errors: Vec<String>,
}

/// Copy a legacy file or directory tree with owner-only permissions; returns the byte total.
fn copy_tree_private(source: &Path, destination: &Path) -> Result<u64> {
    if source.is_dir() {
        ensure_private_dir(destination)?;
        let mut total = 0;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let path = entry.path();
            // Symlinks are not followed: a link must never pull outside data into the store.
            if entry.file_type()?.is_symlink() {
                continue;
            }
            total += copy_tree_private(&path, &destination.join(entry.file_name()))?;
        }
        Ok(total)
    } else if source.is_file() {
        copy_private(source, destination)
    } else {
        bail!("không thể sao chép loại tệp này");
    }
}

fn tree_size(path: &Path) -> std::io::Result<u64> {
    if path.is_dir() {
        let mut total = 0;
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                continue;
            }
            total += tree_size(&entry.path())?;
        }
        Ok(total)
    } else {
        Ok(std::fs::metadata(path)?.len())
    }
}

/// Free `name.bak` (or `name.bak.N`) next to `path`.
fn backup_path(path: &Path) -> std::io::Result<PathBuf> {
    let base = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let base = base.unwrap_or_default();
    let mut n = 0;
    loop {
        let candidate = if n == 0 {
            path.with_file_name(format!("{base}.bak"))
        } else {
            path.with_file_name(format!("{base}.bak.{n}"))
        };
        if existing_entry(&candidate)?.is_none() {
            return Ok(candidate);
        }
        n += 1;
    }
}

fn same_directory(first: &Path, second: &Path) -> bool {
    first == second
        || match (first.canonicalize(), second.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
}

fn can_migrate_legacy_storage(source: &Path, target: &Path) -> bool {
    let staging = target.join(MIGRATION_STAGING);
    !same_directory(source, target)
        && ((is_legacy_item(&staging, true) && is_legacy_item(&staging.join(PUBLISH_LIST), false))
            || !LEGACY_ANCHORS
                .iter()
                .any(|(name, _)| std::fs::symlink_metadata(target.join(name)).is_ok()))
        && legacy_storage_priority(source) > 0
}

fn legacy_storage_priority(source: &Path) -> usize {
    LEGACY_ANCHORS
        .iter()
        .position(|(name, directory)| is_legacy_item(&source.join(name), *directory))
        .map_or(0, |index| LEGACY_ANCHORS.len() - index)
}

fn is_legacy_item(path: &Path, directory: bool) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| {
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        }
    })
}

fn existing_entry(path: &Path) -> std::io::Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn clear_migration_staging(target: &Path, staging: &Path) -> Result<()> {
    let Some(metadata) = existing_entry(staging)? else {
        return Ok(());
    };
    if !metadata.is_dir() {
        bail!("thư mục tạm chuyển dữ liệu không hợp lệ");
    }
    let list = staging.join(PUBLISH_LIST);
    if let Some(metadata) = existing_entry(&list)? {
        if !metadata.is_file() {
            bail!("danh sách tệp cần chuyển không hợp lệ");
        }
        let names = std::fs::read_to_string(&list)?;
        if !names
            .lines()
            .all(|name| LEGACY_FILES.contains(&name) || LEGACY_DIRS.contains(&name))
        {
            bail!("danh sách tệp cần chuyển không hợp lệ");
        }
        // A listed item missing from staging was published before the interruption.
        for name in names.lines() {
            let staged = staging.join(name);
            let published = target.join(name);
            if existing_entry(&staged)?.is_none() && existing_entry(&published)?.is_some() {
                std::fs::rename(&published, &staged)
                    .with_context(|| format!("Không thể hoàn tác việc chuyển {name}"))?;
            }
        }
        std::fs::remove_file(list)?;
    }
    std::fs::remove_dir_all(staging)?;
    Ok(())
}

fn stage_legacy_storage(source: &Path, target: &Path, staging: &Path) -> Result<Vec<&'static str>> {
    let mut staged = Vec::new();
    let staging_location = staging.canonicalize()?;
    let items = LEGACY_FILES
        .iter()
        .map(|&name| (name, false))
        .chain(LEGACY_DIRS.iter().map(|&name| (name, true)));
    for (name, directory) in items {
        let from = source.join(name);
        let Some(metadata) = existing_entry(&from)? else {
            continue;
        };
        if metadata.file_type().is_symlink() || existing_entry(&target.join(name))?.is_some() {
            continue;
        }
        let result = (|| -> Result<()> {
            if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
                bail!("loại tệp không hợp lệ");
            }
            if directory && staging_location.starts_with(from.canonicalize()?) {
                bail!("thư mục tạm nằm trong thư mục nguồn");
            }
            let to = staging.join(name);
            let expected = tree_size(&from)?;
            let copied = copy_tree_private(&from, &to)?;
            if copied != expected || tree_size(&to)? != expected {
                bail!("kích thước bản sao không khớp");
            }
            Ok(())
        })();
        result.with_context(|| format!("Không sao chép được {name}"))?;
        staged.push(name);
    }
    Ok(staged)
}

/// Move session, history and settings that older builds kept in a launch directory into the
/// per-user data directory.
///
/// Copies are staged and size-checked before any item is published. Originals become `*.bak`
/// once the new copies are in place. Existing target items are preserved.
pub fn migrate_legacy_cwd_storage(cwd: &Path, target: &Path) -> MigrationReport {
    let mut report = MigrationReport::default();
    if !can_migrate_legacy_storage(cwd, target) {
        return report;
    }
    let staging = target.join(MIGRATION_STAGING);
    let list = staging.join(PUBLISH_LIST);
    let mut published = Vec::new();
    let result = (|| -> Result<()> {
        ensure_private_dir(target)?;
        #[cfg(unix)]
        let _target_lock = {
            let directory = std::fs::File::open(target)?;
            directory.lock()?;
            directory
        };
        // Recheck after locking in case another startup just finished the migration.
        if !can_migrate_legacy_storage(cwd, target) {
            return Ok(());
        }
        clear_migration_staging(target, &staging).context("Không dọn được dữ liệu tạm cũ")?;
        if !can_migrate_legacy_storage(cwd, target) {
            return Ok(());
        }
        let mut backups = Vec::new();
        let migration = (|| -> Result<()> {
            publish_legacy_storage(cwd, target, &staging, &list, &mut published)?;
            for &name in &published {
                let original = cwd.join(name);
                let backup = backup_path(&original)?;
                std::fs::rename(&original, &backup)
                    .with_context(|| format!("Không đổi tên bản gốc của {name}"))?;
                backups.push((original, backup));
            }
            Ok(())
        })();
        if let Err(error) = migration {
            report.errors.push(format!("{error:#}"));
            for (original, backup) in backups.into_iter().rev() {
                if let Err(error) = std::fs::rename(backup, &original) {
                    report.errors.push(format!(
                        "Không khôi phục được bản gốc {}: {error}",
                        original.display()
                    ));
                }
            }
            if report.errors.len() > 1 {
                // Keep the complete target if a backup could not be restored to its source.
                return Ok(());
            }
            let published = std::mem::take(&mut published);
            if !published.is_empty() && existing_entry(&list)?.is_none() {
                ensure_private_dir(&staging)?;
                write_private_atomic(&list, published.join("\n").as_bytes())?;
            }
            clear_migration_staging(target, &staging)
                .context("Không hoàn tác được dữ liệu đã chuyển")?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        report.errors.push(format!("{error:#}"));
    }
    report.moved = published.into_iter().map(str::to_owned).collect();
    report
}

fn publish_legacy_storage(
    source: &Path,
    target: &Path,
    staging: &Path,
    list: &Path,
    published: &mut Vec<&'static str>,
) -> Result<()> {
    ensure_private_dir(staging)?;
    let staged = stage_legacy_storage(source, target, staging)?;
    write_private_atomic(list, staged.join("\n").as_bytes())?;
    for name in staged {
        let destination = target.join(name);
        if existing_entry(&destination)?.is_some() {
            bail!("đích đến của {name} đã tồn tại");
        }
        std::fs::rename(staging.join(name), destination)
            .with_context(|| format!("Không chuyển được {name} vào thư mục dữ liệu"))?;
        published.push(name);
    }
    std::fs::remove_dir_all(staging).context("Không xóa được thư mục tạm chuyển dữ liệu")?;
    Ok(())
}

fn create_private_file(parent: &Path, prefix: &str) -> std::io::Result<(PathBuf, std::fs::File)> {
    for _ in 0..100 {
        let path = parent.join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "Không tạo được tệp kiểm tra quyền ghi",
    ))
}

/// Create a directory if needed and enforce owner-only access on Unix.
pub fn ensure_private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700).create(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(path)?;
    Ok(())
}

/// Enforce owner read/write permissions for an existing file on Unix.
pub fn ensure_private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Create or truncate a file with owner-only permissions and write its complete contents.
pub fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        bail!("Parent directory does not exist: {}", parent.display());
    }
    ensure_private_dir(parent)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    ensure_private_file(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

/// Copy a sensitive file without inheriting permissive destination permissions.
pub fn copy_private(source: &Path, destination: &Path) -> Result<u64> {
    use std::io::Write;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    ensure_private_dir(parent)?;
    let mut input = std::fs::File::open(source)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(destination)?;
    ensure_private_file(destination)?;
    let copied = std::io::copy(&mut input, &mut output)?;
    output.flush()?;
    output.sync_all()?;
    Ok(copied)
}

/// Owner-only temporary directory that removes all contents when dropped.
pub struct PrivateTempDir {
    path: PathBuf,
}

impl PrivateTempDir {
    /// Create a uniquely named private directory under the system temporary directory.
    pub fn new(prefix: &str) -> Result<Self> {
        for _ in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "{prefix}-{}-{}",
                std::process::id(),
                NEXT_FILE.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => {
                    ensure_private_dir(&path)?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        bail!("Cannot create a unique private temporary directory")
    }

    /// Return the private directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PrivateTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Replace a session only after the complete private temporary file is flushed.
///
/// # Errors
/// Returns an error if creating, writing, flushing, or replacing the file fails.
pub fn write_private_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        bail!("Session directory does not exist: {}", parent.display());
    }
    ensure_private_dir(parent)?;
    let (temporary, mut file) = create_private_file(parent, ".session-tmp")
        .with_context(|| format!("Cannot create temporary session in {}", parent.display()))?;
    let result = (|| -> Result<()> {
        file.write_all(contents).context("Cannot write session")?;
        file.sync_all().context("Cannot flush session")?;
        drop(file);
        std::fs::rename(&temporary, path)
            .with_context(|| format!("Cannot replace session {}", path.display()))?;
        ensure_private_file(path)?;
        Ok(())
    })();
    if result.is_err() {
        // Preserve the original failure if best-effort temporary cleanup also fails.
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn private_helpers_enforce_modes_and_cleanup_temp_data() {
        use std::os::unix::fs::PermissionsExt;
        let root = fixture();
        let private_dir = root.join("private");
        ensure_private_dir(&private_dir).unwrap();
        let file = private_dir.join("secret.bin");
        write_private_file(&file, b"secret").unwrap();
        assert_eq!(
            std::fs::metadata(&private_dir)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let temp_path;
        {
            let temp = PrivateTempDir::new("ctdozlite-private-test").unwrap();
            temp_path = temp.path().to_path_buf();
            write_private_file(&temp_path.join("decrypted.db"), b"sqlite").unwrap();
            assert_eq!(
                std::fs::metadata(&temp_path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        assert!(!temp_path.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn fixture() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ctdozlite-storage-test-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn resolves_bundle_cli_and_explicit_paths() {
        let cwd = Path::new("/work");
        let exe = Path::new("/Applications/Zalo.app/Contents/MacOS/ctdozlite");
        assert_eq!(
            resolve(
                &[],
                cwd,
                exe,
                Platform::MacOs(Some(Path::new("/Users/fixture")))
            )
            .unwrap(),
            (
                PathBuf::from("/Users/fixture/Library/Application Support/ctdozlite"),
                true
            )
        );
        assert_eq!(
            resolve(&[], cwd, Path::new("/bin/ctdozlite"), Platform::MacOs(None)).unwrap(),
            (cwd.to_path_buf(), false)
        );
        assert_eq!(
            resolve(&[], cwd, exe, Platform::Other).unwrap(),
            (cwd.to_path_buf(), false)
        );
        assert_eq!(
            resolve(
                &["--data-dir".into(), "fixture".into()],
                cwd,
                exe,
                Platform::MacOs(None)
            )
            .unwrap(),
            (cwd.join("fixture"), true)
        );
        assert!(resolve(&[], cwd, exe, Platform::MacOs(None)).is_err());
        assert!(resolve(&["--data-dir".into()], cwd, exe, Platform::MacOs(None)).is_err());
        assert!(resolve(&["--data-dir=".into()], cwd, exe, Platform::MacOs(None)).is_err());
        assert!(resolve(
            &["--data-dir=a".into(), "--data-dir=b".into()],
            cwd,
            exe,
            Platform::MacOs(None)
        )
        .is_err());
        assert_eq!(
            resolve(
                &["--profile".into(), "acc_2".into()],
                cwd,
                Path::new("/bin/ctdozlite"),
                Platform::MacOs(None)
            )
            .unwrap(),
            (cwd.join("profiles").join("acc_2"), true)
        );
    }

    #[test]
    fn preparation_retains_existing_data_and_secures_directory() {
        let root = fixture();
        let path = root.join("storage");
        prepare(&path, true).unwrap();
        std::fs::write(path.join("session.json"), b"existing fixture").unwrap();
        prepare(&path, true).unwrap();
        assert_eq!(
            std::fs::read(path.join("session.json")).unwrap(),
            b"existing fixture"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 1);
        assert!(prepare(&path.join("session.json"), true).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn explicit_path_overrides_bundle_and_relative_home_is_rejected() {
        let cwd = Path::new("/work");
        let exe = Path::new("/Applications/Fixture.app/Contents/MacOS/ctdozlite");
        assert_eq!(
            resolve(
                &["--data-dir=/fixture-data".into()],
                cwd,
                exe,
                Platform::MacOs(None)
            )
            .unwrap(),
            (PathBuf::from("/fixture-data"), true)
        );
        assert!(resolve(
            &[],
            cwd,
            exe,
            Platform::MacOs(Some(Path::new("relative-home")))
        )
        .is_err());
        assert_eq!(
            resolve(
                &[],
                cwd,
                Path::new("/Fixture.app/Contents/MacOS-extra/ctdozlite"),
                Platform::MacOs(None)
            )
            .unwrap(),
            (cwd.to_path_buf(), false)
        );
    }

    #[test]
    fn legacy_preparation_keeps_existing_permissions_and_probe_files() {
        let root = fixture();
        let sentinel = root.join(format!(
            ".storage-probe-{}-{}",
            std::process::id(),
            NEXT_FILE.load(Ordering::Relaxed)
        ));
        std::fs::write(&sentinel, b"retained fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        prepare(&root, false).unwrap();
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"retained fixture");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn spaced_storage_option_rejects_flags_but_explicit_dash_paths_are_valid() {
        let cwd = Path::new("/work");
        let exe = Path::new("/bin/ctdozlite");
        for flag in ["-d", "-v", "-V", "--logout"] {
            assert!(
                resolve(
                    &["--data-dir".into(), flag.into()],
                    cwd,
                    exe,
                    Platform::MacOs(None)
                )
                .is_err(),
                "accepted {flag}"
            );
        }
        for args in [
            vec!["--data-dir=-d".into()],
            vec!["--data-dir".into(), "./-d".into()],
        ] {
            let (path, private) = resolve(&args, cwd, exe, Platform::MacOs(None)).unwrap();
            assert_eq!(path, cwd.join("-d"));
            assert!(private);
        }
    }
    #[test]
    fn atomic_write_is_private_and_failed_replace_retains_destination() {
        let root = fixture();
        let path = root.join("session.json");
        write_private_atomic(&path, b"first fixture").unwrap();
        write_private_atomic(&path, b"second fixture").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let occupied = root.join("occupied");
        std::fs::create_dir(&occupied).unwrap();
        std::fs::write(occupied.join("keep"), b"retained").unwrap();
        assert!(write_private_atomic(&occupied, b"replacement").is_err());
        assert_eq!(std::fs::read(occupied.join("keep")).unwrap(), b"retained");
        assert!(write_private_atomic(&root.join("missing/session.json"), b"fixture").is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn macos_non_bundled_runs_share_application_support() {
        let cwd = Path::new("/work/repo");
        let home = Path::new("/Users/fixture");
        let expected = PathBuf::from("/Users/fixture/Library/Application Support/ctdozlite");
        // `cargo run` / CLI / TUI use the same directory as the .app.
        assert_eq!(
            resolve(
                &[],
                cwd,
                Path::new("/work/repo/target/debug/zalo-next"),
                Platform::MacOs(Some(home))
            )
            .unwrap(),
            (expected.clone(), true)
        );
        // --profile nests inside it; --data-dir still wins.
        assert_eq!(
            resolve(
                &["--profile=acc2".into()],
                cwd,
                Path::new("/bin/zalo-next"),
                Platform::MacOs(Some(home))
            )
            .unwrap(),
            (expected.join("profiles").join("acc2"), true)
        );
        assert_eq!(
            resolve(
                &["--data-dir=/tmp/x".into()],
                cwd,
                Path::new("/bin/zalo-next"),
                Platform::MacOs(Some(home))
            )
            .unwrap(),
            (PathBuf::from("/tmp/x"), true)
        );
        // Linux keeps the working directory.
        assert_eq!(
            resolve(&[], cwd, Path::new("/bin/zalo-next"), Platform::Other).unwrap(),
            (cwd.to_path_buf(), false)
        );
        assert!(resolve(
            &[],
            cwd,
            Path::new("/bin/zalo-next"),
            Platform::MacOs(Some(Path::new("rel")))
        )
        .is_err());
    }

    fn write(path: &Path, contents: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn windows_default_and_profile_use_private_local_app_data() {
        let root = fixture();
        let cwd = root.join("cwd");
        let exe = root.join("bin/zalo-next.exe");
        let local_app_data = root.join("Local");
        let platform = Platform::Windows(Some(&local_app_data));
        let base = local_app_data.join("ctdozlite");
        assert_eq!(
            resolve(&[], &cwd, &exe, platform).unwrap(),
            (base.clone(), true)
        );
        assert_eq!(
            resolve(&["--profile=work".into()], &cwd, &exe, platform).unwrap(),
            (base.join("profiles").join("work"), true)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_missing_or_relative_local_app_data_falls_back_to_cwd() {
        let root = fixture();
        let exe = root.join("bin/zalo-next.exe");
        for local_app_data in [None, Some(Path::new("relative-app-data"))] {
            let platform = Platform::Windows(local_app_data);
            assert_eq!(
                resolve(&[], &root, &exe, platform).unwrap(),
                (root.clone(), false)
            );
            assert_eq!(
                resolve(&["--profile=work".into()], &root, &exe, platform).unwrap(),
                (root.join("profiles").join("work"), true)
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_storage_wins_on_all_platforms() {
        let root = fixture();
        let exe = root.join("bin/zalo-next");
        let absolute = root.join("absolute-data");
        for platform in [
            Platform::MacOs(Some(&root)),
            Platform::Windows(Some(&root)),
            Platform::Other,
        ] {
            for (arg, expected) in [
                (
                    format!("--data-dir={}", absolute.display()),
                    absolute.clone(),
                ),
                (
                    "--data-dir=relative-data".into(),
                    root.join("relative-data"),
                ),
            ] {
                assert_eq!(
                    resolve(&[arg, "--profile=work".into()], &root, &exe, platform).unwrap(),
                    (expected, true)
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_migration_prefers_cwd_then_uses_executable_directory() {
        let root = fixture();
        let cwd = root.join("cwd");
        let exe = root.join("bin/zalo-next.exe");
        let exe_dir = exe.parent().unwrap();
        let target = root.join("storage");
        write(&cwd.join("session.json"), b"cwd fixture");
        write(&exe_dir.join("session.json"), b"executable fixture");
        let missing_db = |_: &Path| Err(std::io::ErrorKind::NotFound.into());

        assert_eq!(
            migrate_windows_storage(&cwd, &exe, &target, false, || false, missing_db),
            target
        );
        assert_eq!(
            std::fs::read(target.join("session.json")).unwrap(),
            b"cwd fixture"
        );
        assert!(cwd.join("session.json.bak").is_file());
        assert!(exe_dir.join("session.json").is_file());

        let second_target = root.join("second-storage");
        assert_eq!(
            migrate_windows_storage(&cwd, &exe, &second_target, false, || false, missing_db),
            second_target
        );
        assert_eq!(
            std::fs::read(second_target.join("session.json")).unwrap(),
            b"executable fixture"
        );
        assert!(exe_dir.join("session.json.bak").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_migration_prefers_session_over_bootstrap_db() {
        let root = PrivateTempDir::new("ctdozlite-ranking-test").unwrap();
        let cwd = root.path().join("cwd");
        let exe = root.path().join("bin/zalo-next.exe");
        let exe_dir = exe.parent().unwrap();
        let target = root.path().join("storage");
        write(&cwd.join("zalo_history.db"), b"");
        write(&exe_dir.join("session.json"), b"session fixture");
        write(&exe_dir.join("zalo_history.db"), b"history fixture");

        assert_eq!(
            migrate_windows_storage(&cwd, &exe, &target, false, || false, |_| Ok(())),
            target
        );
        assert_eq!(
            std::fs::read(target.join("zalo_history.db")).unwrap(),
            b"history fixture"
        );
        assert!(cwd.join("zalo_history.db").is_file());
    }

    #[test]
    fn windows_migration_uses_locked_legacy_directory_and_retries_later() {
        let root = fixture();
        let cwd = root.join("cwd");
        let exe = root.join("bin/zalo-next.exe");
        let exe_dir = exe.parent().unwrap();
        let target = root.join("storage");
        prepare(&target, true).unwrap();
        for source in [cwd.as_path(), exe_dir] {
            write(&source.join("session.json"), b"legacy fixture");
            write(&source.join("zalo_history.db"), b"db fixture");
        }
        let locked = |_: &Path| Err(std::io::ErrorKind::PermissionDenied.into());
        assert_eq!(
            migrate_windows_storage(&cwd, &exe, &target, false, || false, locked),
            cwd
        );
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert!(cwd.join("session.json").is_file());
        assert!(exe_dir.join("session.json").is_file());

        let clean_cwd = root.join("empty-cwd");
        std::fs::create_dir(&clean_cwd).unwrap();
        assert_eq!(
            migrate_windows_storage(&clean_cwd, &exe, &target, false, || false, locked),
            exe_dir
        );
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert!(exe_dir.join("zalo_history.db").is_file());

        assert_eq!(
            migrate_windows_storage(&cwd, &exe, &target, false, || false, |_| Ok(())),
            target
        );
        assert!(target.join("zalo_history.db").is_file());
        assert!(cwd.join("zalo_history.db.bak").is_file());
        assert_eq!(
            migrate_windows_storage(&clean_cwd, &exe, &target, false, || false, locked),
            target
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_migration_uses_legacy_after_staging_fails() {
        let root = PrivateTempDir::new("ctdozlite-fallback-test").unwrap();
        let cwd = root.path().join("cwd");
        let exe = root.path().join("bin/zalo-next.exe");
        let target = root.path().join("storage");
        write(&cwd.join("session.json"), b"session fixture");
        write(&cwd.join("zalo_history.db-wal/unexpected"), b"bad WAL");
        write(&exe.parent().unwrap().join("session.json"), b"alternate");

        assert_eq!(
            migrate_windows_storage(&cwd, &exe, &target, false, || false, |_| Ok(())),
            cwd
        );
        assert!(!target.join("session.json").exists());
        assert!(cwd.join("session.json").is_file());
        assert!(exe.parent().unwrap().join("session.json").is_file());
    }

    #[test]
    fn windows_named_profile_migrates_from_executable_directory_after_wait_and_probe() {
        let root = PrivateTempDir::new("ctdozlite-profile-test").unwrap();
        let cwd = root.path().join("cwd");
        let exe = root.path().join("bin/zalo-next.exe");
        let local_app_data = root.path().join("Local");
        let (target, _) = resolve(
            &["--profile=work!".into()],
            &cwd,
            &exe,
            Platform::Windows(Some(&local_app_data)),
        )
        .unwrap();
        let source = exe.parent().unwrap().join("profiles/work");
        let db = source.join("zalo_history.db");
        write(&db, b"profile history");
        write(&cwd.join("session.json"), b"default login");
        write(
            &cwd.join("profiles/personal/session.json"),
            b"other profile",
        );

        assert_eq!(
            migrate_windows_storage(
                &cwd,
                &exe,
                &target,
                true,
                || true,
                |_| panic!("DB probe ran while another instance was still running"),
            ),
            source
        );
        assert!(!target.exists());
        let waited = std::cell::Cell::new(false);
        let check = || {
            waited.set(true);
            false
        };
        assert_eq!(
            migrate_windows_storage(&cwd, &exe, &target, true, check, |path| {
                assert!(waited.get());
                assert_eq!(path, db);
                Err(std::io::ErrorKind::PermissionDenied.into())
            }),
            source
        );
        assert!(!target.exists());
        assert!(db.is_file());

        assert_eq!(
            migrate_windows_storage(&cwd, &exe, &target, true, || false, |_| Ok(())),
            target
        );
        assert_eq!(
            std::fs::read(target.join("zalo_history.db")).unwrap(),
            b"profile history"
        );
        assert!(source.join("zalo_history.db.bak").is_file());
        assert!(cwd.join("session.json").is_file());
        assert!(cwd.join("profiles/personal/session.json").is_file());
        write(&source.join("session.json"), b"legacy profile login");
        assert_eq!(
            migrate_windows_storage(
                &cwd,
                &exe,
                &target,
                true,
                || panic!("Existing target profile must bypass the wait"),
                |_| panic!("Existing target profile must bypass the probe"),
            ),
            target
        );
        assert!(source.join("session.json").is_file());
    }

    #[test]
    fn migration_copies_legacy_cwd_data_and_keeps_backups() {
        let root = fixture();
        let (cwd, target) = (root.join("repo"), root.join("support"));
        write(&cwd.join("session.json"), b"session fixture");
        write(&cwd.join("zalo_history.db"), b"db fixture");
        write(&cwd.join("zalo_history.db-wal"), b"wal fixture");
        write(&cwd.join("accounts/1234.json"), b"account fixture");
        write(&cwd.join("accounts/1234_history.db"), b"account db");
        write(&cwd.join("profiles/work/session.json"), b"profile session");
        write(&cwd.join("unrelated.txt"), b"keep me");
        write(&cwd.join("README.md"), b"keep me too");
        prepare(&target, true).unwrap();

        let report = migrate_legacy_cwd_storage(&cwd, &target);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.moved.contains(&"session.json".to_string()));
        assert!(report.moved.contains(&"accounts".to_string()));
        assert_eq!(
            std::fs::read(target.join("session.json")).unwrap(),
            b"session fixture"
        );
        assert_eq!(
            std::fs::read(target.join("zalo_history.db-wal")).unwrap(),
            b"wal fixture"
        );
        assert_eq!(
            std::fs::read(target.join("accounts/1234.json")).unwrap(),
            b"account fixture"
        );
        // Originals survive as *.bak; unrelated files are untouched.
        assert_eq!(
            std::fs::read(cwd.join("session.json.bak")).unwrap(),
            b"session fixture"
        );
        assert!(cwd.join("accounts.bak/1234.json").is_file());
        assert_eq!(
            std::fs::read(target.join("profiles/work/session.json")).unwrap(),
            b"profile session"
        );
        assert!(cwd.join("profiles.bak/work/session.json").is_file());
        assert!(!cwd.join("session.json").exists());
        assert_eq!(
            std::fs::read(cwd.join("unrelated.txt")).unwrap(),
            b"keep me"
        );
        assert!(!target.join("unrelated.txt").exists() && !target.join("README.md").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(target.join("session.json")), 0o600);
            assert_eq!(mode(target.join("accounts")), 0o700);
            assert_eq!(mode(target.join("accounts/1234.json")), 0o600);
        }
        // Idempotent: a second launch has nothing left to do.
        let again = migrate_legacy_cwd_storage(&cwd, &target);
        assert!(again.moved.is_empty() && again.errors.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_leaves_sources_and_target_intact_after_a_staging_failure() {
        let root = PrivateTempDir::new("ctdozlite-staging-test").unwrap();
        let (cwd, target) = (root.path().join("repo"), root.path().join("support"));
        write(&cwd.join("session.json"), b"session fixture");
        write(&cwd.join("zalo_history.db"), b"db fixture");
        write(&cwd.join("zalo_history.db-wal/unexpected"), b"bad WAL");
        write(&target.join(".migration-staging/stale"), b"stale copy");
        write(&target.join("theme.json"), b"target theme");
        write(&cwd.join("theme.json"), b"source theme");

        let failed = migrate_legacy_cwd_storage(&cwd, &target);
        assert!(!failed.errors.is_empty());
        assert!(failed.moved.is_empty());
        assert!(cwd.join("session.json").is_file());
        assert!(cwd.join("zalo_history.db").is_file());
        assert!(cwd.join("zalo_history.db-wal/unexpected").is_file());
        assert!(!cwd.join("session.json.bak").exists());
        assert!(!cwd.join("zalo_history.db.bak").exists());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 1);
        assert!(!target.join(".migration-staging").exists());

        std::fs::remove_dir_all(cwd.join("zalo_history.db-wal")).unwrap();
        write(&cwd.join("zalo_history.db-wal"), b"wal fixture");
        let retried = migrate_legacy_cwd_storage(&cwd, &target);
        assert!(retried.errors.is_empty(), "{:?}", retried.errors);
        assert_eq!(retried.moved.len(), 3);
        assert_eq!(
            std::fs::read(target.join("zalo_history.db-wal")).unwrap(),
            b"wal fixture"
        );
        assert_eq!(
            std::fs::read(target.join("theme.json")).unwrap(),
            b"target theme"
        );
        assert!(cwd.join("theme.json").is_file());
        assert!(!target.join(".migration-staging").exists());
    }

    #[test]
    fn migration_retries_after_an_interrupted_publish() {
        let root = PrivateTempDir::new("ctdozlite-interrupted-test").unwrap();
        let (cwd, target) = (root.path().join("repo"), root.path().join("support"));
        let staging = target.join(MIGRATION_STAGING);
        write(&cwd.join("session.json"), b"session fixture");
        write(&cwd.join("zalo_history.db"), b"db fixture");
        write(&target.join("session.json"), b"interrupted session copy");
        write(&staging.join("zalo_history.db"), b"interrupted db copy");
        write(
            &staging.join(PUBLISH_LIST),
            b"session.json\nzalo_history.db",
        );

        let retried = migrate_legacy_cwd_storage(&cwd, &target);
        assert!(retried.errors.is_empty(), "{:?}", retried.errors);
        assert_eq!(retried.moved, ["session.json", "zalo_history.db"]);
        assert_eq!(
            std::fs::read(target.join("session.json")).unwrap(),
            b"session fixture"
        );
        assert_eq!(
            std::fs::read(target.join("zalo_history.db")).unwrap(),
            b"db fixture"
        );
        assert!(cwd.join("session.json.bak").is_file());
        assert!(!staging.exists());
    }

    #[test]
    fn migration_rejects_a_target_inside_the_legacy_profiles_tree() {
        let root = PrivateTempDir::new("ctdozlite-overlap-test").unwrap();
        let source = root.path();
        let target = source.join("profiles/target");
        write(&source.join("session.json"), b"default session");
        write(
            &source.join("profiles/work/session.json"),
            b"profile session",
        );

        let report = migrate_legacy_cwd_storage(source, &target);
        assert!(!report.errors.is_empty());
        assert!(report.moved.is_empty());
        assert!(source.join("session.json").is_file());
        assert!(source.join("profiles/work/session.json").is_file());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
    }

    #[test]
    fn migration_never_overwrites_or_mixes_existing_target_data() {
        let root = fixture();
        let (cwd, target) = (root.join("repo"), root.join("support"));
        write(&cwd.join("session.json"), b"old session");
        write(&target.join("session.json"), b"current session");
        let report = migrate_legacy_cwd_storage(&cwd, &target);
        assert!(report.moved.is_empty() && report.errors.is_empty());
        assert_eq!(
            std::fs::read(target.join("session.json")).unwrap(),
            b"current session"
        );
        assert_eq!(
            std::fs::read(cwd.join("session.json")).unwrap(),
            b"old session"
        );
        assert!(!cwd.join("session.json.bak").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_needs_a_session_or_history_anchor_and_a_different_directory() {
        let root = fixture();
        let (cwd, target) = (root.join("repo"), root.join("support"));
        // Only a stray theme file: not Zalo-next storage, leave it alone.
        write(&cwd.join("theme.json"), b"{}");
        std::fs::create_dir_all(&target).unwrap();
        let report = migrate_legacy_cwd_storage(&cwd, &target);
        assert!(report.moved.is_empty() && cwd.join("theme.json").exists());
        // Same directory: nothing to do even with an anchor present.
        write(&cwd.join("session.json"), b"s");
        assert!(migrate_legacy_cwd_storage(&cwd, &cwd).moved.is_empty());
        assert!(cwd.join("session.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn migration_skips_symlinked_entries() {
        let root = fixture();
        let (cwd, target) = (root.join("repo"), root.join("support"));
        write(&cwd.join("session.json"), b"session");
        let outside = root.join("outside.json");
        write(&outside, b"outside data");
        std::os::unix::fs::symlink(&outside, cwd.join("threads_cache.json")).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        let report = migrate_legacy_cwd_storage(&cwd, &target);
        assert_eq!(report.moved, vec!["session.json".to_string()]);
        assert!(!target.join("threads_cache.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
