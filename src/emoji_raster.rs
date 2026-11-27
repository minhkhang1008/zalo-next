//! Colour emoji rasterisation from the operating system's emoji font.
//!
//! egui draws text through a single-channel (coverage-only) glyph atlas, so it can never show
//! colour glyphs: every emoji comes out as a tinted monochrome outline. This module extracts the
//! embedded colour bitmap (`sbix` on macOS, `CBDT` on Linux) for an emoji cluster, or paints its
//! `COLR` vector layers (Segoe UI Emoji on Windows, see `colr`), so the GUI can draw it as a
//! texture instead.
//!
//! Must only run off the UI thread (it touches a ~180 MB memory-mapped font file and decodes
//! PNG); the GUI reaches it through `WorkerCmd::RasterizeEmoji`.

mod colr;

use std::path::PathBuf;
use std::sync::OnceLock;

use memmap2::Mmap;
use rustybuzz::ttf_parser::{GlyphId, RasterImageFormat};

/// Candidate colour emoji fonts on macOS and Linux, in priority order. Missing files are skipped.
const EMOJI_FONT_PATHS: [&str; 5] = [
    "/System/Library/Fonts/Apple Color Emoji.ttc",
    "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf",
    "/usr/share/fonts/google-noto-color-emoji-fonts/Noto-COLRv1.ttf",
];

/// Candidate files in priority order, each with whether it may be memory-mapped. On Windows this
/// is Segoe UI Emoji (`COLR` layers); its directory comes from `%WINDIR%`, which could point at a
/// writable folder, so that file is read into memory instead of mapped (~15 MB).
fn emoji_font_paths() -> Vec<(PathBuf, bool)> {
    let mut paths = Vec::new();
    if cfg!(windows) {
        paths.push((
            crate::system_fonts::windows_fonts_dir().join("seguiemj.ttf"),
            false,
        ));
    }
    paths.extend(EMOJI_FONT_PATHS.iter().map(|p| (PathBuf::from(p), true)));
    paths
}

/// Font bytes held for the whole process.
enum FontData {
    Mapped(Mmap),
    Owned(Vec<u8>),
}

impl FontData {
    fn bytes(&self) -> &[u8] {
        match self {
            FontData::Mapped(map) => map,
            FontData::Owned(bytes) => bytes,
        }
    }
}

/// Largest bitmap edge we keep; bigger strikes are downscaled to bound texture memory.
const MAX_EDGE: u32 = 160;

/// A decoded, square, straight-alpha RGBA bitmap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmojiBitmap {
    pub size: u32,
    pub rgba: Vec<u8>,
}

fn emoji_font() -> Option<&'static [u8]> {
    static FONT: OnceLock<Option<FontData>> = OnceLock::new();
    FONT.get_or_init(|| {
        emoji_font_paths().iter().find_map(|(path, mappable)| {
            if !mappable {
                return std::fs::read(path).ok().map(FontData::Owned);
            }
            let file = std::fs::File::open(path).ok()?;
            // SAFETY: only fixed system font paths (root-owned on macOS/Linux) are mapped; they
            // are not modified while the app runs, and the map is never written and lives for
            // the whole process ('static OnceLock).
            unsafe { Mmap::map(&file) }.ok().map(FontData::Mapped)
        })
    })
    .as_ref()
    .map(FontData::bytes)
}

/// Rasterise one emoji cluster (a single emoji, ZWJ sequence, flag or skin-tone variant).
///
/// `pixels_per_em` picks the closest embedded strike at or above that size (bitmap fonts) or the
/// em size to paint at (`COLR` fonts). Returns `None` when no colour emoji font is installed or the
/// font has no colour glyph for this cluster.
pub fn rasterize(cluster: &str, pixels_per_em: u16) -> Option<EmojiBitmap> {
    let data = emoji_font()?;
    rasterize_from_font(data, cluster, pixels_per_em)
}

fn rasterize_from_font(font: &[u8], cluster: &str, pixels_per_em: u16) -> Option<EmojiBitmap> {
    if cluster.is_empty() {
        return None;
    }
    let face = rustybuzz::Face::from_slice(font, 0)?;

    // Shaping applies the font's ligature rules (morx/GSUB) so multi-codepoint sequences such
    // as 👨‍👩‍👧, 🇻🇳 or 👍🏽 collapse into their single combined glyph.
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(cluster);
    let shaped = rustybuzz::shape(&face, &[], buffer);
    let gid = shaped
        .glyph_infos()
        .iter()
        .map(|g| g.glyph_id)
        .find(|&id| id != 0)?;
    let gid = GlyphId(u16::try_from(gid).ok()?);

    if let Some(raster) = face.glyph_raster_image(gid, pixels_per_em) {
        if raster.format != RasterImageFormat::PNG {
            return None;
        }
        let img = image::load_from_memory(raster.data).ok()?.into_rgba8();
        return Some(square_and_bound(img));
    }

    // Vector colour font. Paint every shaped glyph, so a sequence the font has no single glyph
    // for (a flag in Segoe UI Emoji is two regional-indicator letters) looks as it does natively.
    let mut pen = 0.0;
    let mut glyphs = Vec::new();
    for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        if let Ok(id) = u16::try_from(info.glyph_id) {
            if id != 0 {
                glyphs.push(colr::PlacedGlyph {
                    id: GlyphId(id),
                    x: pen + pos.x_offset as f32,
                });
            }
        }
        pen += pos.x_advance as f32;
    }
    // The vector canvas grows with the requested size; keep it within the texture cap.
    let size = u32::from(pixels_per_em).min(MAX_EDGE);
    let img = colr::render(&face, &glyphs, pen, size)?;
    Some(square_and_bound(img))
}

/// Centre the bitmap on a transparent square canvas and cap its edge at `MAX_EDGE`.
fn square_and_bound(img: image::RgbaImage) -> EmojiBitmap {
    let (w, h) = img.dimensions();
    let edge = w.max(h).max(1);
    let mut canvas = image::RgbaImage::new(edge, edge);
    image::imageops::overlay(
        &mut canvas,
        &img,
        i64::from((edge - w) / 2),
        i64::from((edge - h) / 2),
    );
    let canvas = if edge > MAX_EDGE {
        image::imageops::resize(
            &canvas,
            MAX_EDGE,
            MAX_EDGE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        canvas
    };
    EmojiBitmap {
        size: canvas.width(),
        rgba: canvas.into_raw(),
    }
}

/// Stable texture-cache key for an emoji cluster (codepoints in hex).
pub fn texture_key(cluster: &str) -> String {
    let hex: Vec<String> = cluster.chars().map(|c| format!("{:x}", c as u32)).collect();
    format!("emoji_{}", hex.join("-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_key_is_codepoint_based() {
        assert_eq!(texture_key("🐧"), "emoji_1f427");
        assert_eq!(texture_key("❤️"), "emoji_2764-fe0f");
    }

    #[test]
    fn square_and_bound_pads_and_caps() {
        let bmp = square_and_bound(image::RgbaImage::new(20, 10));
        assert_eq!(bmp.size, 20);
        assert_eq!(bmp.rgba.len(), 20 * 20 * 4);
        let big = square_and_bound(image::RgbaImage::new(400, 400));
        assert_eq!(big.size, MAX_EDGE);
    }

    #[test]
    fn garbage_font_data_yields_none() {
        assert!(rasterize_from_font(b"not a font", "🐧", 96).is_none());
        assert!(rasterize_from_font(b"", "", 96).is_none());
    }

    /// Renders `COLR` emoji from the font file named by `ZALO_COLR_TEST_FONT` (e.g. a copy of
    /// Segoe UI Emoji or Noto-COLRv1.ttf); skipped when unset. With `ZALO_COLR_TEST_OUT` set to a
    /// directory, also writes a contact sheet there for checking the result by eye.
    #[test]
    fn rasterizes_colr_font_from_env() {
        let Some(path) = std::env::var_os("ZALO_COLR_TEST_FONT") else {
            return;
        };
        let font = std::fs::read(&path).expect("read ZALO_COLR_TEST_FONT");
        let clusters = [
            "🐧",
            "💌",
            "❤️",
            "😀",
            "😂",
            "🥰",
            "👍",
            "👍🏽",
            "👨‍👩‍👧",
            "🔥",
            "🌈",
            "🎉",
            "🍜",
            "🐱",
            "🚀",
            "🇻🇳",
        ];
        let mut sheet = image::RgbaImage::new(96 * clusters.len() as u32, 96);
        for (i, cluster) in clusters.iter().enumerate() {
            // Flags are optional: Segoe UI Emoji draws them as monochrome letters.
            let Some(bmp) = rasterize_from_font(&font, cluster, 96) else {
                assert_eq!(*cluster, "🇻🇳", "no bitmap for {cluster}");
                continue;
            };
            assert!(bmp.rgba.iter().skip(3).step_by(4).any(|&a| a > 0));
            let img = image::RgbaImage::from_raw(bmp.size, bmp.size, bmp.rgba).expect("rgba");
            let img = image::imageops::resize(&img, 96, 96, image::imageops::FilterType::Triangle);
            image::imageops::overlay(&mut sheet, &img, i as i64 * 96, 0);
        }
        assert_ne!(
            rasterize_from_font(&font, "👍🏽", 96),
            rasterize_from_font(&font, "👍", 96)
        );
        if let Some(out) = std::env::var_os("ZALO_COLR_TEST_OUT") {
            let out = std::path::Path::new(&out).join("colr_sheet.png");
            sheet.save(&out).expect("save sheet");
        }
    }

    /// Exercises the real system font; skipped where it is not installed.
    #[test]
    fn rasterizes_single_and_sequence_emoji_when_font_present() {
        // Every supported Windows ships Segoe UI Emoji, so a miss there is a path bug, not a
        // missing font; fail loudly so a green Windows CI run proves the COLR path ran.
        #[cfg(windows)]
        assert!(emoji_font().is_some(), "Segoe UI Emoji not found");
        if emoji_font().is_none() {
            return;
        }
        for cluster in ["🐧", "💌", "❤️", "👍🏽", "👨‍👩‍👧"] {
            let bmp = rasterize(cluster, 96).unwrap_or_else(|| panic!("no bitmap for {cluster}"));
            assert!(bmp.size >= 32, "{cluster} too small: {}", bmp.size);
            assert!(bmp.rgba.iter().skip(3).step_by(4).any(|&a| a > 0));
        }
        // Shaping must pick the combined glyph, not the first component.
        assert_ne!(rasterize("👨‍👩‍👧", 96), rasterize("👨", 96));
        assert_ne!(rasterize("👍🏽", 96), rasterize("👍", 96));
        // Segoe UI Emoji has no colour flags (Windows draws them as plain letters), so a flag
        // may fall back to text there; where it is drawn it must be the flag, not one letter.
        if let Some(flag) = rasterize("🇻🇳", 96) {
            assert_ne!(Some(flag), rasterize("🇻", 96));
        }
    }
}
