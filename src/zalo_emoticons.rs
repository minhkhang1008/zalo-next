//! Zalo Web's text shortcuts and their pinned colour sprite (re-notes/51).

use crate::{endpoints::ZALO_EMOTICON_SPRITE, zalo_emoticon_table::EMOTICONS};
use anyhow::{ensure, Context, Result};
use image::RgbaImage;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{self, Read},
    ops::Range,
    path::Path,
    sync::{Arc, OnceLock},
    time::Duration,
};

/// A shortcut's byte range in the original text and its zero-based sprite coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub range: Range<usize>,
    pub col: u8,
    pub row: u8,
}

type Shortcut = (&'static str, u8, u8);
static SHORTCUTS_BY_PREFIX: OnceLock<HashMap<[u8; 2], Vec<Shortcut>>> = OnceLock::new();

/// Find Zalo shortcuts with ASCII case folding and Zalo's space, newline and adjacency rules.
pub fn find(text: &str) -> Vec<Match> {
    let lookup = SHORTCUTS_BY_PREFIX.get_or_init(|| {
        let mut buckets = HashMap::<_, Vec<_>>::new();
        for &(key, col, row) in EMOTICONS {
            let bytes = key.as_bytes();
            buckets
                .entry([bytes[0], bytes[1]])
                .or_default()
                .push((key, col, row));
        }
        for bucket in buckets.values_mut() {
            bucket.sort_unstable_by_key(|(key, _, _)| std::cmp::Reverse(key.len()));
        }
        buckets
    });

    let bytes = text.as_bytes();
    let mut candidates = Vec::new();
    let mut end = 0;
    for (start, ch) in text.char_indices() {
        if start < end || !ch.is_ascii() {
            continue;
        }
        let Some(&second) = bytes.get(start + 1) else {
            break;
        };
        let Some(bucket) =
            lookup.get(&[ch.to_ascii_lowercase() as u8, second.to_ascii_lowercase()])
        else {
            continue;
        };
        if let Some(&(key, col, row)) = bucket.iter().find(|(key, _, _)| {
            bytes[start..]
                .get(..key.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(key.as_bytes()))
        }) {
            end = start + key.len();
            candidates.push(Match {
                range: start..end,
                col,
                row,
            });
        }
    }

    // Zalo looks ahead to the next candidate, while the left neighbour must already be accepted.
    let mut accepted = Vec::new();
    let mut candidates = candidates.into_iter().peekable();
    let mut accepted_end = 0;
    while let Some(candidate) = candidates.next() {
        let Range { start, end } = candidate.range;
        let before = start == accepted_end || matches!(bytes[start - 1], b' ' | b'\n');
        let after = end == bytes.len()
            || matches!(bytes[end], b' ' | b'\n')
            || candidates
                .peek()
                .is_some_and(|next| next.range.start == end);
        if before && after {
            accepted_end = end;
            accepted.push(candidate);
        }
    }
    accepted
}

/// Key shared by the worker's tile event and the GUI's existing emoji texture cache.
pub(crate) fn texture_key(col: u8, row: u8) -> String {
    // The GUI keeps failed `emoji_` requests cached for the app run.
    format!("emoji_zemo_{col}_{row}")
}

const SPRITE_SHA256: &str = "39e30cb3dda96560788ff4af83bdfa48d0d550dd307b017e5bb435c1eb72c6dc";
const SPRITE_CACHE: &str = "stickers_cache/zalo_emoji_sprite.png";
const SPRITE_LIMIT: usize = 4 * 1024 * 1024;
pub(crate) const TILE_SIZE: u32 = 64;
const SPRITE_WIDTH: u32 = 51 * TILE_SIZE;
const SPRITE_HEIGHT: u32 = 41 * TILE_SIZE;

/// Load the verified sprite on the worker and make one delayed retry on failure.
pub(crate) async fn load_sprite_with_retry() -> Option<Arc<RgbaImage>> {
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(10 * 60)).await;
        }
        match load_sprite().await {
            Ok(sprite) => return Some(Arc::new(sprite)),
            Err(err) => crate::logger::log_error("zalo_emoticons::load_sprite", &err.to_string()),
        }
    }
    None
}

async fn load_sprite() -> Result<RgbaImage> {
    let path = Path::new(SPRITE_CACHE);
    match tokio::task::spawn_blocking(move || read_cached_sprite(path)).await? {
        Ok(Some(sprite)) => return Ok(sprite),
        Ok(None) => {}
        Err(err) => crate::logger::log_error("zalo_emoticons::read_cache", &err.to_string()),
    }

    ensure!(
        crate::session::is_trusted_zalo_url(ZALO_EMOTICON_SPRITE),
        "Untrusted Zalo emoji sprite URL"
    );
    let http = reqwest::Client::builder()
        .cookie_store(false)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(30))
        .build()?;
    let response = http.get(ZALO_EMOTICON_SPRITE).send().await?;
    let bytes = crate::media_resources::read_bounded_response(response, SPRITE_LIMIT).await?;
    tokio::task::spawn_blocking(move || {
        let sprite = decode_sprite(&bytes)?;
        let cache_dir = Path::new("stickers_cache");
        if let Err(err) = crate::data_dir::ensure_private_dir(cache_dir)
            .and_then(|()| crate::data_dir::write_private_atomic(path, &bytes))
        {
            crate::logger::log_error("zalo_emoticons::write_cache", &err.to_string());
        }
        Ok(sprite)
    })
    .await?
}

fn read_cached_sprite(path: &Path) -> Result<Option<RgbaImage>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::remove_file(path).context("Remove symlink in Zalo emoji sprite cache")?;
            return Ok(None);
        }
        Ok(meta) => ensure!(
            meta.is_file(),
            "Zalo emoji sprite cache is not a regular file"
        ),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context("Inspect Zalo emoji sprite cache"),
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(SPRITE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    match decode_sprite(&bytes) {
        Ok(sprite) => {
            crate::data_dir::ensure_private_file(path)?;
            Ok(Some(sprite))
        }
        Err(_) => {
            fs::remove_file(path).context("Remove invalid Zalo emoji sprite cache")?;
            Ok(None)
        }
    }
}

fn decode_sprite(bytes: &[u8]) -> Result<RgbaImage> {
    ensure!(
        bytes.len() <= SPRITE_LIMIT && format!("{:x}", Sha256::digest(bytes)) == SPRITE_SHA256,
        "Zalo emoji sprite failed size or SHA-256 validation"
    );
    // Only the exact pinned PNG reaches the decoder.
    let sprite = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?;
    ensure!(
        sprite.width() == SPRITE_WIDTH && sprite.height() == SPRITE_HEIGHT,
        "Unexpected Zalo emoji sprite dimensions"
    );
    Ok(sprite.into_rgba8())
}

/// Crop a tile by its zero-based coordinates, rejecting any cell outside the sheet.
pub(crate) fn crop_tile(sprite: &RgbaImage, col: u8, row: u8) -> Option<RgbaImage> {
    let x = u32::from(col) * TILE_SIZE;
    let y = u32::from(row) * TILE_SIZE;
    if x + TILE_SIZE > sprite.width() || y + TILE_SIZE > sprite.height() {
        return None;
    }
    Some(image::imageops::crop_imm(sprite, x, y, TILE_SIZE, TILE_SIZE).to_image())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_shortcuts_with_utf8_byte_offsets() {
        for (text, range) in [(":) chào", 0..2), ("chào :)", 6..8), ("a\n:)\nb", 2..4)] {
            assert_eq!(
                find(text),
                [Match {
                    range,
                    col: 41,
                    row: 7
                }]
            );
        }
    }

    #[test]
    fn rejects_shortcuts_embedded_in_words_and_urls() {
        for text in ["a:)b", "http://x.y/:)", "a\t:)", ":)\rb", "é:)", ":)é"] {
            assert!(find(text).is_empty(), "{text:?}");
        }
    }

    #[test]
    fn handles_adjacent_candidates_greedily() {
        let first = Match {
            range: 0..2,
            col: 41,
            row: 7,
        };
        let second = Match {
            range: 2..4,
            col: 41,
            row: 7,
        };
        assert_eq!(find(":):)"), [first.clone(), second]);
        assert_eq!(find(":):)x"), [first]);
        assert!(find("x:):)").is_empty());
    }

    #[test]
    fn prefers_longest_keys_and_folds_ascii_case() {
        for (text, col, row) in [
            (":-((", 42, 1),
            (":(", 41, 34),
            (":((", 41, 38),
            ("/-strong", 42, 33),
            ("_()_", 42, 40),
            (":D", 41, 0),
            (":smile:", 26, 24),
            (":SMILE:", 26, 24),
        ] {
            assert_eq!(
                find(text),
                [Match {
                    range: 0..text.len(),
                    col,
                    row
                }]
            );
        }
    }

    #[test]
    fn crops_the_requested_rgba_tile() {
        let sprite = RgbaImage::from_fn(3 * TILE_SIZE, 2 * TILE_SIZE, |x, y| {
            image::Rgba([x as u8, y as u8, 150, 120])
        });
        let tile = crop_tile(&sprite, 2, 1).unwrap();
        assert_eq!(tile.dimensions(), (TILE_SIZE, TILE_SIZE));
        assert_eq!(tile.get_pixel(0, 0), sprite.get_pixel(128, 64));
        assert_eq!(tile.get_pixel(63, 63), sprite.get_pixel(191, 127));
        assert!(crop_tile(&sprite, 3, 1).is_none());
        assert!(crop_tile(&sprite, 2, 2).is_none());
    }

    #[test]
    fn removes_an_unverified_cached_sprite() {
        let dir = crate::data_dir::PrivateTempDir::new("zalo-emoji-test").unwrap();
        let path = dir.path().join("sprite.png");
        fs::write(&path, b"invalid sprite").unwrap();
        assert!(read_cached_sprite(&path).unwrap().is_none());
        assert!(!path.exists());
    }
}
