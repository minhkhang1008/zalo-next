//! App Lock: hide the app behind a lock screen until the owner proves who they are.
//!
//! This is a UI lock, not encryption. `session.json` and the message database stay readable to
//! anyone who can read the data directory; the lock only keeps a person at the keyboard from
//! seeing chats or notification text while the owner is away.
//!
//! * macOS: the system prompt (Touch ID or the account password, `LocalAuthentication`).
//!   No secret is stored.
//! * Other platforms: a passcode of at least [`MIN_PASSCODE_CHARS`] characters, kept as a
//!   salted PBKDF2-HMAC-SHA256 hash in [`CONFIG_FILE`] (owner-only file).
//!
//! The UI thread only reads the in-memory config; loading and saving go through the worker.

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const CONFIG_FILE: &str = "app_lock.json";
pub const MIN_PASSCODE_CHARS: usize = 6;
/// Upper bound so a pasted novel cannot make hashing slow.
pub const MAX_PASSCODE_CHARS: usize = 128;
/// OWASP 2023 guidance for PBKDF2-HMAC-SHA256.
const ITERATIONS: u32 = 600_000;
/// Refuse a stored config that was weakened by hand (tests use a small count to stay fast).
const MIN_ITERATIONS: u32 = if cfg!(test) { 1_000 } else { 100_000 };
const SALT_LEN: usize = 16;
const HASH_LEN: usize = 32;
/// Wrong passcodes allowed before the first cooldown.
const FREE_ATTEMPTS: u32 = 5;
const FIRST_LOCKOUT_SECS: i64 = 30;
const MAX_LOCKOUT_SECS: i64 = 15 * 60;
/// A wall-clock jump this much larger than the monotonic clock means the machine slept.
pub const SLEEP_JUMP: Duration = Duration::from_secs(45);
/// Idle timeouts offered in Settings, in minutes (0 = only manual and sleep).
pub const IDLE_CHOICES_MIN: [u32; 4] = [0, 5, 15, 30];

static LOCKED: AtomicBool = AtomicBool::new(false);

/// Whether the app is locked right now. Read by every path that shows message content outside
/// the main window (OS notifications, the call window, the tray).
pub fn is_locked() -> bool {
    LOCKED.load(Ordering::Relaxed)
}

pub fn set_locked(locked: bool) {
    LOCKED.store(locked, Ordering::Relaxed);
}

/// How the owner unlocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// macOS Touch ID / account password.
    System,
    /// Passcode hashed in [`CONFIG_FILE`].
    Passcode,
}

/// The method this build offers.
pub fn platform_method() -> Method {
    if cfg!(target_os = "macos") {
        Method::System
    } else {
        Method::Passcode
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub enabled: bool,
    pub method: Method,
    /// Minutes without input before locking; 0 disables the idle lock.
    pub idle_minutes: u32,
    pub lock_on_sleep: bool,
    salt_b64: String,
    hash_b64: String,
    iterations: u32,
    failed_attempts: u32,
    lockout_until_unix: i64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            enabled: false,
            method: platform_method(),
            idle_minutes: 15,
            lock_on_sleep: true,
            salt_b64: String::new(),
            hash_b64: String::new(),
            iterations: ITERATIONS,
            failed_attempts: 0,
            lockout_until_unix: 0,
        }
    }
}

/// Result of a passcode attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    Unlocked,
    /// Wrong passcode; `left` more tries before a cooldown.
    Wrong {
        left: u32,
    },
    /// Too many wrong tries: wait this many seconds.
    CoolingDown {
        secs: i64,
    },
}

impl Config {
    /// Whether a passcode is set (meaningful for [`Method::Passcode`]).
    pub fn has_passcode(&self) -> bool {
        !self.hash_b64.is_empty() && !self.salt_b64.is_empty()
    }

    /// Whether this config can actually lock: enabled, and either the system method or a
    /// passcode is set.
    pub fn usable(&self) -> bool {
        self.enabled && (self.method == Method::System || self.has_passcode())
    }

    pub fn idle_timeout(&self) -> Option<Duration> {
        (self.idle_minutes > 0).then(|| Duration::from_secs(u64::from(self.idle_minutes) * 60))
    }

    /// Set a new passcode and switch to [`Method::Passcode`]; resets the attempt counter.
    pub fn set_passcode(&mut self, passcode: &str) -> Result<()> {
        self.set_passcode_with(passcode, ITERATIONS)
    }

    fn set_passcode_with(&mut self, passcode: &str, iterations: u32) -> Result<()> {
        validate_passcode(passcode)?;
        let mut salt = [0u8; SALT_LEN];
        getrandom::fill(&mut salt).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
        let hash = derive(passcode, &salt, iterations);
        self.salt_b64 = B64.encode(salt);
        self.hash_b64 = B64.encode(hash);
        self.iterations = iterations;
        self.method = Method::Passcode;
        self.failed_attempts = 0;
        self.lockout_until_unix = 0;
        Ok(())
    }

    /// Check a passcode at `now_unix`. Updates the attempt counter; the caller persists the
    /// config afterwards so a restart does not reset the cooldown.
    pub fn try_passcode(&mut self, candidate: &str, now_unix: i64) -> Attempt {
        if now_unix < self.lockout_until_unix {
            return Attempt::CoolingDown {
                secs: self.lockout_until_unix - now_unix,
            };
        }
        if self.matches(candidate) {
            self.failed_attempts = 0;
            self.lockout_until_unix = 0;
            return Attempt::Unlocked;
        }
        self.failed_attempts = self.failed_attempts.saturating_add(1);
        if self.failed_attempts < FREE_ATTEMPTS {
            return Attempt::Wrong {
                left: FREE_ATTEMPTS - self.failed_attempts,
            };
        }
        // 30 s after the 5th wrong try, doubling with each further one, capped at 15 min.
        let doublings = (self.failed_attempts - FREE_ATTEMPTS).min(10);
        let secs = (FIRST_LOCKOUT_SECS << doublings).min(MAX_LOCKOUT_SECS);
        self.lockout_until_unix = now_unix.saturating_add(secs);
        Attempt::CoolingDown { secs }
    }

    /// The system method succeeded: clear the passcode counters as well.
    pub fn note_unlocked(&mut self) {
        self.failed_attempts = 0;
        self.lockout_until_unix = 0;
    }

    fn matches(&self, candidate: &str) -> bool {
        if candidate.chars().count() > MAX_PASSCODE_CHARS || self.iterations < MIN_ITERATIONS {
            return false;
        }
        let (Ok(salt), Ok(expected)) = (B64.decode(&self.salt_b64), B64.decode(&self.hash_b64))
        else {
            return false;
        };
        if salt.len() != SALT_LEN || expected.len() != HASH_LEN {
            return false;
        }
        constant_time_eq(&derive(candidate, &salt, self.iterations), &expected)
    }

    /// Normalise a loaded config: a passcode config without a usable hash cannot lock (it would
    /// lock the owner out), and the system method is not available off macOS.
    fn sanitized(mut self) -> Self {
        if self.method == Method::System && platform_method() != Method::System {
            self.enabled = false;
        }
        if self.method == Method::Passcode
            && (!self.has_passcode() || self.iterations < MIN_ITERATIONS)
        {
            self.enabled = false;
        }
        if !IDLE_CHOICES_MIN.contains(&self.idle_minutes) {
            self.idle_minutes = 15;
        }
        self
    }
}

/// Passcode rules shown to the user when they are broken.
pub fn validate_passcode(passcode: &str) -> Result<()> {
    let count = passcode.chars().count();
    if count < MIN_PASSCODE_CHARS {
        bail!("Mã khóa cần ít nhất {MIN_PASSCODE_CHARS} ký tự.");
    }
    if count > MAX_PASSCODE_CHARS {
        bail!("Mã khóa tối đa {MAX_PASSCODE_CHARS} ký tự.");
    }
    if passcode.trim() != passcode {
        bail!("Mã khóa không được bắt đầu hoặc kết thúc bằng khoảng trắng.");
    }
    Ok(())
}

fn derive(passcode: &str, salt: &[u8], iterations: u32) -> [u8; HASH_LEN] {
    let mut out = [0u8; HASH_LEN];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(passcode.as_bytes(), salt, iterations, &mut out);
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Load `dir/app_lock.json`. A missing file is the default (disabled); a corrupt file is an
/// error so the caller can tell the owner instead of silently dropping their lock.
pub fn load(dir: &Path) -> Result<Config> {
    let path = dir.join(CONFIG_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e).with_context(|| format!("đọc {CONFIG_FILE}")),
    };
    let config: Config =
        serde_json::from_slice(&bytes).with_context(|| format!("{CONFIG_FILE} hỏng"))?;
    Ok(config.sanitized())
}

/// Save atomically as an owner-only file.
pub fn save(dir: &Path, config: &Config) -> Result<()> {
    let json = serde_json::to_vec_pretty(config)?;
    crate::data_dir::write_private_atomic(&dir.join(CONFIG_FILE), &json)
}

/// Detects when the app should lock by itself.
///
/// Idle time and sleep are measured from frame timestamps: `Instant` (monotonic, and on macOS
/// paused while the machine sleeps) against `SystemTime` (wall clock, keeps running).
#[derive(Debug)]
pub struct AutoLock {
    last_input: std::time::Instant,
    last_tick: Option<(std::time::Instant, std::time::SystemTime)>,
}

impl Default for AutoLock {
    fn default() -> Self {
        Self::new(std::time::Instant::now())
    }
}

impl AutoLock {
    pub fn new(now: std::time::Instant) -> Self {
        Self {
            last_input: now,
            last_tick: None,
        }
    }

    pub fn note_input(&mut self, now: std::time::Instant) {
        self.last_input = now;
    }

    /// Call once per frame. Returns true when the app should lock now.
    pub fn tick(
        &mut self,
        config: &Config,
        now: std::time::Instant,
        wall: std::time::SystemTime,
    ) -> bool {
        let slept = self.last_tick.is_some_and(|(mono, prev_wall)| {
            let mono = now.saturating_duration_since(mono);
            let wall = wall.duration_since(prev_wall).unwrap_or_default();
            wall > mono + SLEEP_JUMP
        });
        self.last_tick = Some((now, wall));
        if !config.usable() {
            return false;
        }
        let idle = config
            .idle_timeout()
            .is_some_and(|limit| now.saturating_duration_since(self.last_input) >= limit);
        idle || (slept && config.lock_on_sleep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Instant, SystemTime};

    fn passcode_config() -> Config {
        let mut config = Config {
            enabled: true,
            ..Config::default()
        };
        // Fewer iterations keep the tests fast; `matches` accepts any count >= MIN_ITERATIONS.
        config
            .set_passcode_with("hunter22", MIN_ITERATIONS)
            .unwrap();
        config
    }

    #[test]
    fn passcode_rules() {
        assert!(validate_passcode("12345").is_err());
        assert!(validate_passcode("123456").is_ok());
        assert!(validate_passcode("mật khẩu").is_ok());
        assert!(validate_passcode(" 123456").is_err());
        assert!(validate_passcode(&"x".repeat(MAX_PASSCODE_CHARS + 1)).is_err());
    }

    #[test]
    fn right_passcode_unlocks_and_resets_the_counter() {
        let mut config = passcode_config();
        assert_eq!(config.try_passcode("wrong!", 0), Attempt::Wrong { left: 4 });
        assert_eq!(config.try_passcode("hunter22", 0), Attempt::Unlocked);
        assert_eq!(config.failed_attempts, 0);
    }

    #[test]
    fn repeated_wrong_passcodes_cool_down_with_backoff_even_for_the_right_one() {
        let mut config = passcode_config();
        for left in (1..FREE_ATTEMPTS).rev() {
            assert_eq!(config.try_passcode("nope!!", 100), Attempt::Wrong { left });
        }
        assert_eq!(
            config.try_passcode("nope!!", 100),
            Attempt::CoolingDown { secs: 30 }
        );
        // During the cooldown even the right passcode is refused.
        assert_eq!(
            config.try_passcode("hunter22", 110),
            Attempt::CoolingDown { secs: 20 }
        );
        assert_eq!(
            config.try_passcode("nope!!", 130),
            Attempt::CoolingDown { secs: 60 }
        );
        assert_eq!(config.try_passcode("hunter22", 190), Attempt::Unlocked);
        for _ in 0..40 {
            config.try_passcode("nope!!", i64::MAX / 2);
        }
        assert!(config.lockout_until_unix - i64::MAX / 2 <= MAX_LOCKOUT_SECS);
    }

    #[test]
    fn stored_hash_never_contains_the_passcode_and_round_trips() {
        let dir = crate::data_dir::PrivateTempDir::new("app-lock").unwrap();
        let config = passcode_config();
        save(dir.path(), &config).unwrap();
        let raw = std::fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        assert!(!raw.contains("hunter22"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(CONFIG_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let mut loaded = load(dir.path()).unwrap();
        assert_eq!(loaded, config);
        assert_eq!(loaded.try_passcode("hunter22", 0), Attempt::Unlocked);
    }

    #[test]
    fn weakened_or_broken_configs_never_lock_the_owner_out() {
        let dir = crate::data_dir::PrivateTempDir::new("app-lock-weak").unwrap();
        assert_eq!(load(dir.path()).unwrap(), Config::default());

        let mut weak = passcode_config();
        weak.iterations = 1;
        save(dir.path(), &weak).unwrap();
        assert!(!load(dir.path()).unwrap().enabled);

        let mut no_hash = passcode_config();
        no_hash.hash_b64.clear();
        save(dir.path(), &no_hash).unwrap();
        assert!(!load(dir.path()).unwrap().usable());

        std::fs::write(dir.path().join(CONFIG_FILE), b"{not json").unwrap();
        assert!(load(dir.path()).is_err());
    }

    #[test]
    fn auto_lock_on_idle_and_on_sleep() {
        let config = Config {
            enabled: true,
            method: Method::System,
            idle_minutes: 5,
            ..Config::default()
        };
        let start = Instant::now();
        let wall = SystemTime::now();
        let mut auto = AutoLock::new(start);
        assert!(!auto.tick(&config, start, wall));
        let four_min = Duration::from_secs(240);
        assert!(!auto.tick(&config, start + four_min, wall + four_min));
        auto.note_input(start + four_min);
        let eight_min = Duration::from_secs(480);
        assert!(!auto.tick(&config, start + eight_min, wall + eight_min));
        let ten_min = Duration::from_secs(600);
        assert!(auto.tick(&config, start + ten_min, wall + ten_min));

        // Sleep: the wall clock jumps an hour while the monotonic clock barely moves.
        let mut auto = AutoLock::new(start);
        let no_idle = Config {
            idle_minutes: 0,
            ..config.clone()
        };
        assert!(!auto.tick(&no_idle, start, wall));
        let one_sec = Duration::from_secs(1);
        assert!(auto.tick(&no_idle, start + one_sec, wall + Duration::from_secs(3600)));

        // Disabled config never locks.
        let off = Config::default();
        let mut auto = AutoLock::new(start);
        assert!(!auto.tick(&off, start + ten_min, wall + ten_min));
    }
}
