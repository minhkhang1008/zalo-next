//! Where the operating system keeps its font files.

use std::path::{Path, PathBuf};

/// Prefix used in the font candidate tables for files in the Windows font directory.
const WINDOWS_FONTS_PREFIX: &str = "C:\\Windows\\Fonts\\";

/// The Windows font directory. Windows is not always installed on `C:\Windows`, so this follows
/// `%WINDIR%` (or `%SystemRoot%`) and only falls back to the default location.
pub fn windows_fonts_dir() -> PathBuf {
    std::env::var_os("WINDIR")
        .or_else(|| std::env::var_os("SystemRoot"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
        .join("Fonts")
}

/// Resolve a font candidate path. On Windows, `C:\Windows\Fonts\…` is rewritten to the real
/// Windows font directory; every other path is returned as is.
pub fn resolve(path: &str) -> PathBuf {
    match path.strip_prefix(WINDOWS_FONTS_PREFIX) {
        Some(file) if cfg!(windows) => windows_fonts_dir().join(file),
        _ => Path::new(path).to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_windows_paths_are_unchanged() {
        assert_eq!(
            resolve("/System/Library/Fonts/Helvetica.ttc"),
            PathBuf::from("/System/Library/Fonts/Helvetica.ttc")
        );
        assert!(windows_fonts_dir().ends_with("Fonts"));
    }
}
