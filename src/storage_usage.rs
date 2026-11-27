//! Measure data-directory storage and clear the allowlisted cache and log paths.

use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::{self, Metadata},
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const MAX_DEPTH: usize = 8;
const MIN_AGE: Duration = Duration::from_secs(10 * 60);

/// A storage group displayed by the storage manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    System,
    Cache,
    Media,
    Logs,
    Foreign,
}

/// Logical size and regular-file count for a storage group or media folder.
#[derive(Debug, PartialEq, Eq)]
pub struct UsageEntry {
    pub category: Category,
    pub label: String,
    pub bytes: u64,
    pub files: u64,
    pub current_account: bool,
}

/// Storage usage within the recognized layout.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub entries: Vec<UsageEntry>,
    pub total: u64,
}

/// Storage that the caller can explicitly request to clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearTarget {
    Cache,
    Media { current_account_only: bool },
    Logs,
    Foreign,
}

/// Results of a best-effort clear. Counts cover files and truncated logs.
/// Skips also include links and subtrees excluded by the depth limit.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ClearReport {
    pub freed: u64,
    pub removed: u64,
    pub skipped: u64,
    pub errors: Vec<String>,
}

/// Measure accessible, recognized entries under the supplied data directory.
pub fn scan(data_dir: &Path, current_account: Option<&str>) -> Usage {
    let Ok(root) = canonical_root(data_dir) else {
        return Usage::default();
    };
    let mut walk = Walk::new(current_account, None);
    walk.layout(&root, Layout::Root, 0);
    walk.usage
        .entries
        .sort_by_key(|entry| match entry.category {
            Category::System => (0, false),
            Category::Cache => (1, false),
            Category::Media => (2, !entry.current_account),
            Category::Logs => (3, false),
            Category::Foreign => (4, false),
        });
    walk.usage
}

/// Clear eligible files at least ten minutes old, preserving the cache roots.
/// A current-account-only clear requires an account ID.
pub fn clear(
    data_dir: &Path,
    target: ClearTarget,
    current_account: Option<&str>,
    now: SystemTime,
) -> ClearReport {
    if target
        == (ClearTarget::Media {
            current_account_only: true,
        })
        && current_account.is_none_or(str::is_empty)
    {
        return ClearReport::default();
    }
    let Ok(root) = canonical_root(data_dir) else {
        return ClearReport {
            errors: vec!["Không thể mở thư mục dữ liệu để dọn dẹp.".into()],
            ..ClearReport::default()
        };
    };
    let mut walk = Walk::new(
        current_account,
        Some(Cleanup {
            root: &root,
            target,
            now,
        }),
    );
    walk.layout(&root, Layout::Root, 0);
    walk.report
}

fn canonical_root(path: &Path) -> io::Result<PathBuf> {
    // Strip trailing separators or "." so the final component is checked as a link.
    let path: PathBuf = path.components().collect();
    if !real_directory(&fs::symlink_metadata(&path)?) {
        return Err(io::Error::from(ErrorKind::InvalidInput));
    }
    let canonical = path.canonicalize()?;
    if !real_directory(&fs::symlink_metadata(&canonical)?) {
        return Err(io::Error::from(ErrorKind::InvalidInput));
    }
    Ok(canonical)
}

fn unchanged_path(path: &Path) -> io::Result<()> {
    if path.canonicalize()? == path {
        Ok(())
    } else {
        Err(io::Error::from(ErrorKind::PermissionDenied))
    }
}

fn real_directory(meta: &Metadata) -> bool {
    meta.is_dir() && !platform::is_link(meta)
}

fn is_account_hash(name: &OsStr) -> bool {
    let bytes = name.as_encoded_bytes();
    bytes.len() == 64
        && bytes
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

#[derive(Clone, Copy)]
enum Layout {
    Root,
    Accounts,
    Account,
    MediaCache,
    MediaKind(MediaKind),
}

#[derive(Clone, Copy)]
enum MediaKind {
    Video,
    Voice,
}

enum EntryKind {
    Layout(Layout),
    Tree(Category),
    File(Category),
    MediaAccount(MediaKind, bool),
    SystemAccountEntry,
}

impl Layout {
    fn classify(self, name: &OsStr, account_hash: Option<&str>) -> Option<EntryKind> {
        use EntryKind::{File, MediaAccount, SystemAccountEntry, Tree};
        let kind = match self {
            Self::Root => match name.to_str() {
                Some("accounts") => EntryKind::Layout(Self::Accounts),
                Some("media_cache") => EntryKind::Layout(Self::MediaCache),
                Some("profiles" | "video_tools") => Tree(Category::System),
                Some("stickers_cache") => Tree(Category::Cache),
                Some("cache" | "media") => Tree(Category::Foreign),
                Some(
                    "session.json"
                    | "zalo_history.db"
                    | "zalo_history.db-wal"
                    | "zalo_history.db-shm"
                    | "app_settings.json"
                    | "ai_secret.json"
                    | "sync_rsa_identity.json"
                    | "autoreply_rules.json"
                    | "threads_cache.json"
                    | "active_account"
                    | "theme.json"
                    | "ctdozlite.lock"
                    | "app_lock.json",
                ) => File(Category::System),
                _ if name.as_encoded_bytes().starts_with(b"quick_messages") => {
                    File(Category::System)
                }
                _ if name == "log.txt" || name.as_encoded_bytes().ends_with(b".log") => {
                    File(Category::Logs)
                }
                _ => return None,
            },
            Self::Accounts => EntryKind::Layout(Self::Account),
            Self::Account if name == "media_cache" => Tree(Category::Foreign),
            Self::Account => SystemAccountEntry,
            Self::MediaCache => match name.to_str() {
                Some("video") => EntryKind::Layout(Self::MediaKind(MediaKind::Video)),
                Some("voice") => EntryKind::Layout(Self::MediaKind(MediaKind::Voice)),
                Some("notification_avatar") => Tree(Category::Cache),
                _ => return None,
            },
            Self::MediaKind(kind) if is_account_hash(name) => {
                MediaAccount(kind, account_hash.is_some_and(|hash| name == hash))
            }
            Self::MediaKind(_) => return None,
        };
        Some(kind)
    }
}

impl EntryKind {
    fn selected(&self, target: ClearTarget) -> bool {
        match *self {
            Self::Layout(Layout::Accounts | Layout::Account) => target == ClearTarget::Foreign,
            Self::Layout(Layout::MediaCache) => {
                matches!(target, ClearTarget::Cache | ClearTarget::Media { .. })
            }
            Self::Layout(Layout::MediaKind(_)) => matches!(target, ClearTarget::Media { .. }),
            Self::Tree(Category::Cache) => target == ClearTarget::Cache,
            Self::Tree(Category::Foreign) => target == ClearTarget::Foreign,
            Self::File(Category::Logs) => target == ClearTarget::Logs,
            Self::MediaAccount(_, is_current) => {
                matches!(target, ClearTarget::Media { current_account_only } if !current_account_only || is_current)
            }
            _ => false,
        }
    }
}

struct Cleanup<'a> {
    root: &'a Path,
    target: ClearTarget,
    now: SystemTime,
}

struct Walk<'a> {
    account_hash: Option<String>,
    cleanup: Option<Cleanup<'a>>,
    usage: Usage,
    report: ClearReport,
}

impl<'a> Walk<'a> {
    fn new(current_account: Option<&str>, cleanup: Option<Cleanup<'a>>) -> Self {
        Self {
            account_hash: current_account
                .filter(|account| !account.is_empty())
                .map(|account| format!("{:x}", Sha256::digest(account.as_bytes()))),
            cleanup,
            usage: Usage::default(),
            report: ClearReport::default(),
        }
    }

    fn layout(&mut self, dir: &Path, layout: Layout, depth: usize) {
        let Some(entries) = self.read_dir(dir, depth) else {
            return;
        };
        for entry in entries {
            let Some(entry) = self.result(entry, "Không thể đọc một mục trong thư mục.")
            else {
                continue;
            };
            let Some(kind) = layout.classify(&entry.file_name(), self.account_hash.as_deref())
            else {
                continue;
            };
            if self
                .cleanup
                .as_ref()
                .is_some_and(|cleanup| !kind.selected(cleanup.target))
            {
                continue;
            }
            let path = entry.path();
            let Some(meta) = self.metadata(&path) else {
                continue;
            };
            match kind {
                EntryKind::Layout(layout) if meta.is_dir() => self.layout(&path, layout, depth + 1),
                EntryKind::Layout(Layout::Account) if self.cleanup.is_none() && meta.is_file() => {
                    self.measure(Category::System, meta.len());
                }
                EntryKind::Tree(category) if meta.is_dir() => {
                    self.tree(&path, category, depth + 1, true)
                }
                EntryKind::File(category) if meta.is_file() => self.file(&path, &meta, category),
                EntryKind::MediaAccount(kind, current) if meta.is_dir() => {
                    self.media(&path, kind, current, depth + 1);
                }
                EntryKind::SystemAccountEntry if meta.is_dir() => {
                    self.tree(&path, Category::System, depth + 1, true);
                }
                EntryKind::SystemAccountEntry if meta.is_file() => {
                    self.measure(Category::System, meta.len())
                }
                _ => {}
            }
        }
    }

    fn tree(&mut self, path: &Path, category: Category, depth: usize, keep_root: bool) {
        let Some(entries) = self.read_dir(path, depth) else {
            return;
        };
        for entry in entries {
            let Some(entry) = self.result(entry, "Không thể đọc một mục trong thư mục.")
            else {
                continue;
            };
            let child = entry.path();
            let Some(meta) = self.metadata(&child) else {
                continue;
            };
            if meta.is_dir() {
                self.tree(&child, category, depth + 1, false);
            } else if meta.is_file() {
                self.file(&child, &meta, category);
            }
        }
        if !keep_root {
            if let Some(cleanup) = &self.cleanup {
                let result = cleanup
                    .remove(path, true)
                    .or_else(|error| match error.kind() {
                        ErrorKind::DirectoryNotEmpty | ErrorKind::NotFound => Ok(()),
                        _ => Err(error),
                    });
                self.result(result, "Không thể xóa thư mục trống.");
            }
        }
    }

    fn media(&mut self, path: &Path, kind: MediaKind, current: bool, depth: usize) {
        let Some(entries) = self.read_dir(path, depth) else {
            return;
        };
        let mut usage = UsageEntry {
            category: Category::Media,
            label: if current {
                "Video/ghi âm đã tải (tài khoản này)"
            } else {
                "Video/ghi âm đã tải (tài khoản khác)"
            }
            .into(),
            bytes: 0,
            files: 0,
            current_account: current,
        };
        for entry in entries {
            let Some(entry) = self.result(entry, "Không thể đọc một mục trong thư mục.")
            else {
                continue;
            };
            let path = entry.path();
            let Some(meta) = self.metadata(&path) else {
                continue;
            };
            if !meta.is_file()
                || (matches!(kind, MediaKind::Video)
                    && path.extension() != Some(OsStr::new("media")))
            {
                continue;
            }
            if self.cleanup.is_some() {
                self.file(&path, &meta, Category::Media);
            } else {
                usage.add(meta.len());
            }
        }
        if self.cleanup.is_none() {
            self.usage.total = self.usage.total.saturating_add(usage.bytes);
            self.usage.entries.push(usage);
        }
    }

    fn read_dir(&mut self, path: &Path, depth: usize) -> Option<fs::ReadDir> {
        if depth >= MAX_DEPTH {
            self.report.skipped += 1;
            return None;
        }
        self.result(
            unchanged_path(path).and_then(|()| fs::read_dir(path)),
            "Không thể đọc thư mục.",
        )
    }

    fn metadata(&mut self, path: &Path) -> Option<Metadata> {
        let meta = self.result(fs::symlink_metadata(path), "Không thể kiểm tra một mục.")?;
        if platform::is_link(&meta) {
            self.report.skipped += 1;
            return None;
        }
        Some(meta)
    }

    fn file(&mut self, path: &Path, meta: &Metadata, category: Category) {
        let Some(cleanup) = &self.cleanup else {
            self.measure(category, meta.len());
            return;
        };
        let result = if category == Category::Logs {
            cleanup.truncate_log(path, meta)
        } else {
            cleanup.remove_file(path, meta)
        };
        match self.result(
            result,
            if category == Category::Logs {
                "Không thể làm trống tệp nhật ký."
            } else {
                "Không thể xóa tệp."
            },
        ) {
            Some(Some(bytes)) => {
                self.report.freed = self.report.freed.saturating_add(bytes);
                self.report.removed += 1;
            }
            Some(None) => self.report.skipped += 1,
            None => {}
        }
    }

    fn measure(&mut self, category: Category, bytes: u64) {
        let entry = if let Some(index) = self
            .usage
            .entries
            .iter()
            .position(|entry| entry.category == category)
        {
            &mut self.usage.entries[index]
        } else {
            let label = match category {
                Category::System => "Dữ liệu hệ thống",
                Category::Cache => "Bộ nhớ đệm",
                Category::Logs => "Nhật ký",
                Category::Foreign => "Dữ liệu ctdozlite cũ",
                Category::Media => return,
            };
            self.usage.entries.push(UsageEntry {
                category,
                label: label.into(),
                bytes: 0,
                files: 0,
                current_account: false,
            });
            let index = self.usage.entries.len() - 1;
            &mut self.usage.entries[index]
        };
        entry.add(bytes);
        self.usage.total = self.usage.total.saturating_add(bytes);
    }

    fn result<T>(&mut self, result: io::Result<T>, message: &str) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                if self.cleanup.is_some() && error.kind() != ErrorKind::NotFound {
                    self.report.errors.push(message.into());
                }
                None
            }
        }
    }
}

impl UsageEntry {
    fn add(&mut self, bytes: u64) {
        self.bytes = self.bytes.saturating_add(bytes);
        self.files += 1;
    }
}

impl Cleanup<'_> {
    /// Refuse any path that does not resolve to itself inside the data directory, so a symlink
    /// swapped into an ancestor after the walk cannot redirect a deletion. The data directory is
    /// owner-only (0700), which bounds who could race this check.
    fn check_location(&self, path: &Path) -> io::Result<()> {
        if !path.starts_with(self.root) || path == self.root {
            return Err(io::Error::from(ErrorKind::PermissionDenied));
        }
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::from(ErrorKind::InvalidInput))?;
        unchanged_path(parent)
    }

    fn old_enough(&self, meta: &Metadata) -> io::Result<bool> {
        Ok(self
            .now
            .duration_since(meta.modified()?)
            .is_ok_and(|age| age >= MIN_AGE))
    }

    fn remove(&self, path: &Path, directory: bool) -> io::Result<()> {
        self.check_location(path)?;
        let latest = fs::symlink_metadata(path)?;
        if platform::is_link(&latest) {
            return Err(io::Error::from(ErrorKind::PermissionDenied));
        }
        if directory {
            fs::remove_dir(path)
        } else {
            fs::remove_file(path)
        }
    }

    fn remove_file(&self, path: &Path, meta: &Metadata) -> io::Result<Option<u64>> {
        if !self.old_enough(meta)? {
            return Ok(None);
        }
        self.check_location(path)?;
        let latest = fs::symlink_metadata(path)?;
        if platform::is_link(&latest) || !latest.is_file() || !self.old_enough(&latest)? {
            return Ok(None);
        }
        fs::remove_file(path)?;
        Ok(Some(latest.len()))
    }

    /// Truncate in place: a running writer may hold the log open.
    fn truncate_log(&self, path: &Path, meta: &Metadata) -> io::Result<Option<u64>> {
        if !self.old_enough(meta)? {
            return Ok(None);
        }
        self.check_location(path)?;
        let before = fs::symlink_metadata(path)?;
        // A hard link could make a "log" share its contents with a protected file.
        if platform::is_link(&before) || !before.is_file() || !platform::one_link(&before) {
            return Ok(None);
        }
        let file = fs::OpenOptions::new().write(true).open(path)?;
        let opened = file.metadata()?;
        if !platform::same_file(&before, &opened) || !self.old_enough(&opened)? {
            return Ok(None);
        }
        file.set_len(0)?;
        Ok(Some(opened.len()))
    }
}

/// Portable link and identity checks (std only, no FFI).
mod platform {
    use std::fs::Metadata;

    pub(super) fn is_link(meta: &Metadata) -> bool {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return true;
            }
        }
        meta.file_type().is_symlink()
    }

    pub(super) fn one_link(meta: &Metadata) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            meta.nlink() == 1
        }
        #[cfg(not(unix))]
        {
            let _ = meta;
            true
        }
    }

    /// Whether two metadata snapshots describe the same file (Unix device and inode).
    pub(super) fn same_file(first: &Metadata, second: &Metadata) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            first.dev() == second.dev() && first.ino() == second.ino()
        }
        #[cfg(not(unix))]
        {
            let _ = (first, second);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_dir::PrivateTempDir;

    const CURRENT: &str = "user-1";
    const OTHER: &str = "user-2";
    const HOUR: Duration = Duration::from_secs(3600);

    fn fixture() -> PrivateTempDir {
        PrivateTempDir::new("zalo-storage-usage").unwrap()
    }

    fn write(root: &Path, name: impl AsRef<Path>, bytes: &[u8]) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    fn media_dir(kind: &str, account: &str) -> PathBuf {
        let hash = format!("{:x}", Sha256::digest(account.as_bytes()));
        Path::new("media_cache").join(kind).join(hash)
    }

    #[test]
    fn scan_counts_each_category_and_media_folder() {
        let dir = fixture();
        let root = dir.path();
        write(root, "session.json", b"abc");
        write(root, "quick_messages.json", b"d");
        write(root, "accounts/user-1_history.db", b"ef");
        write(root, "accounts/user-1/other/item", b"g");
        write(root, "accounts/user-1/media_cache/old", b"hijk");
        write(root, "profiles/user-1/session.json", b"l");
        write(root, "video_tools/ffmpeg", b"mnop");
        write(root, "stickers_cache/thumbs/sticker.png", b"qrst");
        write(root, "media_cache/notification_avatar/avatar.png", b"uvw");
        write(
            root,
            media_dir("video", CURRENT).join("current.media"),
            b"12345",
        );
        write(root, media_dir("voice", CURRENT).join("recording"), b"123");
        write(root, media_dir("voice", OTHER).join("recording"), b"123456");
        write(root, "call.log", b"1234567");
        write(root, "log.txt", b"1");
        write(root, "cache/legacy", b"12");
        write(root, "media/legacy", b"123");
        write(root, "log-directory.log/file", b"ignored");
        write(root, "media_cache/video/unknown/fake.media", b"ignored");
        write(
            root,
            media_dir("video", CURRENT).join("saved.mp4"),
            b"ignored",
        );
        write(root, "unknown/stuff", b"ignored");

        let usage = scan(root, Some(CURRENT));
        let mut sizes = usage
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.category,
                    entry.bytes,
                    entry.files,
                    entry.current_account,
                )
            })
            .collect::<Vec<_>>();
        sizes.sort_by_key(|entry| entry.1);
        assert_eq!(
            sizes,
            [
                (Category::Media, 3, 1, true),
                (Category::Media, 5, 1, true),
                (Category::Media, 6, 1, false),
                (Category::Cache, 7, 2, false),
                (Category::Logs, 8, 2, false),
                (Category::Foreign, 9, 3, false),
                (Category::System, 12, 6, false),
            ]
        );
        assert_eq!(usage.total, 50);
        for entry in usage
            .entries
            .iter()
            .filter(|entry| entry.category == Category::Media)
        {
            let label = if entry.current_account {
                "Video/ghi âm đã tải (tài khoản này)"
            } else {
                "Video/ghi âm đã tải (tài khoản khác)"
            };
            assert_eq!(entry.label, label);
        }
    }

    #[test]
    fn clear_cache_skips_recent_clipboard_and_removes_empty_subdirs() {
        let dir = fixture();
        let root = dir.path();
        let old = write(root, "stickers_cache/thumbs/sticker.png", b"old");
        let avatar = write(root, "media_cache/notification_avatar/avatar.png", b"saved");
        let recent = write(root, "stickers_cache/clipboard_recent.png", b"recent");
        let now = SystemTime::now();
        for path in [&old, &avatar] {
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(now - HOUR))
                .unwrap();
        }

        let report = clear(root, ClearTarget::Cache, None, now);
        assert_eq!(
            report,
            ClearReport {
                freed: 8,
                removed: 2,
                skipped: 1,
                errors: vec![]
            }
        );
        assert!(recent.is_file());
        assert!(!old.exists());
        assert!(!old.parent().unwrap().exists());
        assert!(!avatar.exists());
        assert!(avatar.parent().unwrap().is_dir());
        assert!(root.join("stickers_cache").is_dir());
    }

    #[test]
    fn clear_logs_truncates_files_in_place() {
        let dir = fixture();
        let root = dir.path();
        let log = write(root, "active.log", b"hello");
        let writer = fs::File::options().append(true).open(&log).unwrap();
        let text = write(root, "log.txt", b"hi");
        let nested = write(root, "profiles/test/nested.log", b"keep");
        let inside_log_named_dir = write(root, "directory.log/inner", b"keep");

        let report = clear(root, ClearTarget::Logs, None, SystemTime::now() + HOUR);
        assert_eq!(
            report,
            ClearReport {
                freed: 7,
                removed: 2,
                skipped: 0,
                errors: vec![]
            }
        );
        assert_eq!(writer.metadata().unwrap().len(), 0);
        assert_eq!(fs::metadata(log).unwrap().len(), 0);
        assert_eq!(fs::metadata(text).unwrap().len(), 0);
        assert_eq!(fs::read(nested).unwrap(), b"keep");
        assert_eq!(fs::read(inside_log_named_dir).unwrap(), b"keep");
    }

    #[test]
    fn clear_current_media_preserves_other_accounts_and_unrecognized_files() {
        let dir = fixture();
        let root = dir.path();
        let video = write(
            root,
            media_dir("video", CURRENT).join("video.media"),
            b"video",
        );
        let voice = write(
            root,
            media_dir("voice", CURRENT).join("recording"),
            b"voice",
        );
        let other_video = write(root, media_dir("video", OTHER).join("video.media"), b"keep");
        let other_voice = write(root, media_dir("voice", OTHER).join("recording"), b"keep");
        let saved = write(root, media_dir("video", CURRENT).join("saved.mp4"), b"keep");
        let misplaced = write(root, "media_cache/video/unknown/video.media", b"keep");
        let target = ClearTarget::Media {
            current_account_only: true,
        };
        let now = SystemTime::now() + HOUR;

        assert_eq!(clear(root, target, None, now), ClearReport::default());
        assert!(video.is_file());
        assert_eq!(
            clear(root, target, Some(CURRENT), now),
            ClearReport {
                freed: 10,
                removed: 2,
                skipped: 0,
                errors: vec![]
            },
        );
        for path in [video, voice] {
            assert!(!path.exists());
            assert!(path.parent().unwrap().is_dir());
        }
        for path in [&other_video, &other_voice, &saved, &misplaced] {
            assert_eq!(fs::read(path).unwrap(), b"keep");
        }

        let all_media = ClearTarget::Media {
            current_account_only: false,
        };
        assert_eq!(
            clear(root, all_media, None, now),
            ClearReport {
                freed: 8,
                removed: 2,
                skipped: 0,
                errors: vec![]
            },
        );
        assert!(!other_video.exists());
        assert!(!other_voice.exists());
        assert!(misplaced.is_file());
        assert!(saved.is_file());
    }

    #[test]
    fn clear_foreign_empties_only_legacy_roots() {
        let dir = fixture();
        let root = dir.path();
        let cache = write(root, "cache/subdir/old", b"old");
        let media = write(root, "media/old", b"old");
        let account_cache = write(root, "accounts/user-1/media_cache/old", b"old");
        let db = write(root, "accounts/user-1_history.db", b"keep");
        let other = write(root, "accounts/user-1/other", b"keep");
        let nested_nonlegacy = write(root, "accounts/user-1/dir/media_cache/keep", b"keep");
        let avatar = write(root, "media_cache/notification_avatar/keep", b"keep");

        let report = clear(root, ClearTarget::Foreign, None, SystemTime::now() + HOUR);
        assert_eq!(
            report,
            ClearReport {
                freed: 9,
                removed: 3,
                skipped: 0,
                errors: vec![]
            }
        );
        for path in [cache, media, account_cache] {
            assert!(!path.exists());
        }
        for path in [db, other, nested_nonlegacy, avatar] {
            assert_eq!(fs::read(path).unwrap(), b"keep");
        }
        for path in ["cache", "media", "accounts/user-1/media_cache"] {
            assert_eq!(fs::read_dir(root.join(path)).unwrap().count(), 0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn clear_cache_skips_symlinks_inside_and_at_cache_roots() {
        use std::os::unix::fs::symlink;

        let dir = fixture();
        let outside = fixture();
        let root = dir.path();
        let outside_file = write(outside.path(), "new.jpg", b"keep");
        let old = write(root, "stickers_cache/old.png", b"remove");
        let link = root.join("stickers_cache/link.png");
        symlink(&outside_file, &link).unwrap();
        symlink(outside.path(), root.join("media_cache")).unwrap();

        let report = clear(root, ClearTarget::Cache, None, SystemTime::now() + HOUR);
        assert_eq!(
            report,
            ClearReport {
                freed: 6,
                removed: 1,
                skipped: 2,
                errors: vec![]
            }
        );
        assert!(!old.exists());
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(outside_file).unwrap(), b"keep");
        assert_eq!(scan(root, None).total, 0);
    }

    #[test]
    fn system_and_unknown_files_survive_every_clear() {
        let dir = fixture();
        let root = dir.path();
        let protected = [
            "session.json",
            "zalo_history.db",
            "zalo_history.db-wal",
            "zalo_history.db-shm",
            "accounts/user-1.json",
            "accounts/user-1_history.db",
            "accounts/user-1_threads.json",
            "app_settings.json",
            "ai_secret.json",
            "sync_rsa_identity.json",
            "autoreply_rules.json",
            "threads_cache.json",
            "active_account",
            "quick_messages.json",
            "quick_messages_backup.log",
            "theme.json",
            "ctdozlite.lock",
            "app_lock.json",
            "profiles/profile/session.json",
            "profiles/profile/cache/member",
            "video_tools/ffmpeg",
            "ctdozlite.sock",
            "unknown/data.log",
            "media_cache/unknown/kept",
        ];
        for path in protected {
            write(root, path, b"keep");
        }
        for target in [
            ClearTarget::Cache,
            ClearTarget::Media {
                current_account_only: false,
            },
            ClearTarget::Logs,
            ClearTarget::Foreign,
        ] {
            assert_eq!(
                clear(root, target, Some(CURRENT), SystemTime::now() + HOUR),
                ClearReport::default(),
            );
            for path in protected {
                assert_eq!(fs::read(root.join(path)).unwrap(), b"keep", "{path}");
            }
        }
    }

    #[test]
    fn clear_skips_subtrees_beyond_the_depth_limit() {
        let dir = fixture();
        let root = dir.path();
        let deep = Path::new("stickers_cache").join("sub/".repeat(MAX_DEPTH - 1));
        let kept = write(root, deep.join("keep"), b"keep");
        let shallow = write(root, "stickers_cache/remove", b"remove");

        let report = clear(root, ClearTarget::Cache, None, SystemTime::now() + HOUR);
        assert_eq!(
            report,
            ClearReport {
                freed: 6,
                removed: 1,
                skipped: 1,
                errors: vec![]
            }
        );
        assert_eq!(fs::read(kept).unwrap(), b"keep");
        assert!(!shallow.exists());
    }

    #[cfg(unix)]
    #[test]
    fn clear_logs_skips_links_to_protected_files() {
        use std::os::unix::fs::symlink;

        let dir = fixture();
        let root = dir.path();
        let system = write(root, "session.json", b"keep");
        symlink(&system, root.join("soft.log")).unwrap();
        fs::hard_link(&system, root.join("hard.log")).unwrap();

        let report = clear(root, ClearTarget::Logs, None, SystemTime::now() + HOUR);
        assert_eq!(
            report,
            ClearReport {
                skipped: 2,
                ..ClearReport::default()
            }
        );
        assert_eq!(fs::read(system).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn clear_rejects_a_symlink_supplied_as_data_dir() {
        use std::os::unix::fs::symlink;

        let dir = fixture();
        let outside = fixture();
        let file = write(outside.path(), "stickers_cache/keep", b"keep");
        let alias = dir.path().join("alias");
        symlink(outside.path(), &alias).unwrap();
        let alias = alias.join(".");

        let report = clear(&alias, ClearTarget::Cache, None, SystemTime::now() + HOUR);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.removed, 0);
        assert_eq!(scan(&alias, None), Usage::default());
        assert_eq!(fs::read(file).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn deletion_rechecks_the_location_of_its_parent() {
        use std::os::unix::fs::symlink;

        let dir = fixture();
        let outside = fixture();
        let root = dir.path().canonicalize().unwrap();
        let file = write(&root, "stickers_cache/file", b"keep");
        let meta = fs::symlink_metadata(&file).unwrap();
        let cleanup = Cleanup {
            root: &root,
            target: ClearTarget::Cache,
            now: SystemTime::now() + HOUR,
        };

        // The cache folder is swapped for a link to somewhere else after the walk.
        let moved = outside.path().join("moved");
        fs::rename(root.join("stickers_cache"), &moved).unwrap();
        symlink(&moved, root.join("stickers_cache")).unwrap();

        assert!(cleanup.remove_file(&file, &meta).is_err());
        assert_eq!(fs::read(moved.join("file")).unwrap(), b"keep");
    }
}
