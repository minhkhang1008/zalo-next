//! Photo albums: several photos sent in one go are shown as one justified grid, like Zalo PC/Web.
//!
//! Wire evidence (live captures in `threads_cache.json`): every photo of an album arrives as its
//! own `chat.photo` message whose `content.params` JSON string carries `is_group_layout: 1`,
//! `group_layout_id`, `id_in_group` and `total_item_in_group` (plus `width`/`height`); the same
//! layout id is repeated in `extraData.groupMediaMsg.groupLayoutId`. Photos sent one by one lack
//! these fields and keep rendering as single large photos.
//!
//! Row split measured on Zalo Web with mixed-aspect test photos (see `.agents/WORKLOG.md`):
//! 2 → [2], 3 → [3], 4 → [2,2], 5 → [2,3], 6 → [2,2,2], 7 → [2,3,2], 10 → [2,3,2,3]. Every row is
//! scaled to the full width with all photos at the same height.

use super::*;
use serde_json::Value;

/// Album membership of one photo message.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AlbumInfo {
    pub layout_id: String,
    pub index: u32,
    pub total: u32,
    /// Original photo size from the wire, used until the texture is decoded.
    pub dims: Option<(f32, f32)>,
}

fn json_u64(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn json_f32(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Number(n) => n.as_f64().map(|f| f as f32),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Parse a JSON object that may arrive either as an object or as a JSON-encoded string.
fn object_or_string(v: &Value) -> Option<Value> {
    match v {
        Value::String(s) => serde_json::from_str(s).ok(),
        Value::Object(_) => Some(v.clone()),
        _ => None,
    }
}

/// Album metadata of a message, if it is one photo of a multi-photo send.
pub(super) fn album_info(msg: &ChatMessage) -> Option<AlbumInfo> {
    let content = object_or_string(msg.wire_ref.fields.get("content")?)?;
    let params = object_or_string(content.get("params")?)?;
    if json_u64(params.get("is_group_layout")) != Some(1) {
        return None;
    }
    let layout_id = json_u64(params.get("group_layout_id")).or_else(|| {
        let extra = object_or_string(msg.wire_ref.fields.get("extraData")?)?;
        json_u64(extra.get("groupMediaMsg")?.get("groupLayoutId"))
    })?;
    let dims = match (
        json_f32(params.get("width")),
        json_f32(params.get("height")),
    ) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some((w, h)),
        _ => None,
    };
    Some(AlbumInfo {
        layout_id: layout_id.to_string(),
        index: json_u64(params.get("id_in_group")).unwrap_or(0) as u32,
        total: json_u64(params.get("total_item_in_group")).unwrap_or(0) as u32,
        dims,
    })
}

/// Contiguous runs of album photos in `visible` (indices into `messages`).
///
/// Returns `(first_position, run_length, member_message_indices)` per run of at least two photos
/// with the same layout id and sender. Single photos are left alone.
pub(super) fn album_runs(
    messages: &[ChatMessage],
    visible: &[usize],
) -> Vec<(usize, usize, Vec<usize>)> {
    let mut runs = Vec::new();
    let mut pos = 0;
    while pos < visible.len() {
        let first = &messages[visible[pos]];
        let Some(info) = album_info(first) else {
            pos += 1;
            continue;
        };
        let mut members = vec![visible[pos]];
        let mut next = pos + 1;
        while next < visible.len() {
            let candidate = &messages[visible[next]];
            let same = album_info(candidate).is_some_and(|c| c.layout_id == info.layout_id)
                && candidate.is_outgoing == first.is_outgoing
                && candidate.sender == first.sender;
            if !same {
                break;
            }
            members.push(visible[next]);
            next += 1;
        }
        if members.len() > 1 {
            runs.push((pos, members.len(), members));
        }
        pos = next;
    }
    runs
}

/// Preferred sum of aspect ratios per row; at the 2-3 photos Zalo uses this keeps rows 190-280 px
/// tall in a ~600 px wide bubble.
const TARGET_ROW_ASPECT: f32 = 2.4;
const MAX_PHOTOS_PER_ROW: usize = 5;

/// Split photos into rows (a linear-partition DP on the row aspect sum).
///
/// Ties go to the alternating 2,3,2,3 pattern Zalo uses; the expected splits are pinned by tests.
pub(super) fn album_row_lengths(aspects: &[f32]) -> Vec<usize> {
    let n = aspects.len();
    if n == 0 {
        return Vec::new();
    }
    let a: Vec<f32> = aspects.iter().map(|v| v.clamp(0.4, 2.5)).collect();
    // best[i][p]: cost of laying out the first i photos in an even (p = 0) or odd (p = 1) number
    // of rows; `choice` remembers the length of the last row.
    let mut best = vec![[f32::INFINITY; 2]; n + 1];
    let mut choice = vec![[0usize; 2]; n + 1];
    best[0][0] = 0.0;
    for i in 1..=n {
        for len in 1..=MAX_PHOTOS_PER_ROW.min(i) {
            let sum: f32 = a[i - len..i].iter().sum();
            for prev_parity in 0..2 {
                let prev = best[i - len][prev_parity];
                if !prev.is_finite() {
                    continue;
                }
                // `prev_parity` is the parity of the rows already placed = index of this row.
                let preferred = if prev_parity == 0 { 2 } else { 3 };
                let mismatch = if len == preferred { 0.0 } else { 1e-4 };
                let cost = prev + (sum - TARGET_ROW_ASPECT).powi(2) + mismatch;
                let parity = 1 - prev_parity;
                if cost < best[i][parity] {
                    best[i][parity] = cost;
                    choice[i][parity] = len;
                }
            }
        }
    }
    let mut parity = if best[n][0] <= best[n][1] { 0 } else { 1 };
    let mut rows = Vec::new();
    let mut i = n;
    while i > 0 {
        let len = choice[i][parity];
        rows.push(len);
        i -= len;
        parity = 1 - parity;
    }
    rows.reverse();
    rows
}

/// Width of the album grid inside a bubble whose text area is `max_text_width` wide.
pub(super) fn album_width(max_text_width: f32) -> f32 {
    max_text_width.clamp(220.0, 480.0).floor()
}

const GAP: f32 = 2.0;
const CORNER: f32 = 12.0;

/// Draw the album grid and return the action of the clicked photo, if any.
pub(super) fn render_album_grid(
    ui: &mut egui::Ui,
    members: &[&ChatMessage],
    width: f32,
    pal: &ThemePalette,
    sticker_textures: &HashMap<String, TextureHandle>,
    loading_stickers: &mut HashSet<String>,
    sender_label: &str,
) -> Option<BubbleAction> {
    struct Cell {
        key: String,
        open_target: String,
        aspect: f32,
        loaded: bool,
        title: String,
        time: String,
    }
    let mut cells: Vec<Cell> = Vec::new();
    for msg in members {
        let Some(thumb_url) = photo_primary_url(msg) else {
            continue;
        };
        let key = format!("thumb_{}", crate::crypto::md5_hex(thumb_url));
        if !sticker_textures.contains_key(&key) && loading_stickers.insert(key.clone()) {
            if let Some(tx) = WORKER_TX.get() {
                let _ = tx.send(WorkerCmd::FetchStickerImage {
                    key: key.clone(),
                    url: thumb_url.to_string(),
                });
            }
        }
        let open_target = msg
            .link_url
            .as_deref()
            .filter(|u| !u.is_empty() && crate::chat::is_zalo_photo_cdn_url(u))
            .unwrap_or(thumb_url)
            .to_string();
        let tex_size = sticker_textures
            .get(&key)
            .map(|t| t.size())
            .filter(|[w, h]| *w > 2 && *h > 2);
        let aspect = match tex_size {
            Some([w, h]) => w as f32 / h as f32,
            None => album_info(msg)
                .and_then(|i| i.dims)
                .map(|(w, h)| w / h)
                .unwrap_or(1.0),
        };
        cells.push(Cell {
            key,
            open_target,
            aspect,
            loaded: tex_size.is_some(),
            title: "Hình ảnh".to_string(),
            time: msg.time.clone(),
        });
    }
    if cells.is_empty() {
        return None;
    }

    let aspects: Vec<f32> = cells.iter().map(|c| c.aspect).collect();
    let rows = album_row_lengths(&aspects);
    let row_heights: Vec<f32> = {
        let mut start = 0;
        rows.iter()
            .map(|&len| {
                let sum: f32 = aspects[start..start + len]
                    .iter()
                    .map(|a| a.clamp(0.4, 2.5))
                    .sum();
                start += len;
                ((width - GAP * (len as f32 - 1.0)) / sum).floor()
            })
            .collect()
    };
    let total_h = row_heights.iter().sum::<f32>() + GAP * (rows.len() as f32 - 1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, total_h), Sense::hover());

    let mut action = None;
    let mut cell_idx = 0;
    let mut y = rect.top();
    for (row_i, (&len, &row_h)) in rows.iter().zip(&row_heights).enumerate() {
        let sum: f32 = aspects[cell_idx..cell_idx + len]
            .iter()
            .map(|a| a.clamp(0.4, 2.5))
            .sum();
        let usable_w = width - GAP * (len as f32 - 1.0);
        let mut x = rect.left();
        for col in 0..len {
            let cell = &cells[cell_idx];
            let cell_w = if col + 1 == len {
                rect.right() - x
            } else {
                (usable_w * cell.aspect.clamp(0.4, 2.5) / sum).floor()
            };
            let cell_rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(cell_w, row_h));
            let rounding = Rounding {
                nw: if row_i == 0 && col == 0 { CORNER } else { 3.0 },
                ne: if row_i == 0 && col + 1 == len {
                    CORNER
                } else {
                    3.0
                },
                sw: if row_i + 1 == rows.len() && col == 0 {
                    CORNER
                } else {
                    3.0
                },
                se: if row_i + 1 == rows.len() && col + 1 == len {
                    CORNER
                } else {
                    3.0
                },
            };
            let resp = ui
                .interact(
                    cell_rect,
                    Id::new(("album_cell", &cell.key, cell_idx)),
                    Sense::click(),
                )
                .on_hover_cursor(CursorIcon::PointingHand);
            if let Some(tex) = sticker_textures.get(&cell.key).filter(|_| cell.loaded) {
                egui::Image::new(tex)
                    .rounding(rounding)
                    .paint_at(ui, cell_rect);
                // HD badge, top-left like Zalo.
                let badge =
                    Rect::from_min_size(cell_rect.min + Vec2::new(6.0, 6.0), Vec2::new(26.0, 15.0));
                ui.painter().rect_filled(
                    badge,
                    Rounding::same(4.0),
                    Color32::from_black_alpha(140),
                );
                ui.painter().text(
                    badge.center(),
                    egui::Align2::CENTER_CENTER,
                    "HD",
                    FontId::proportional(9.5),
                    Color32::WHITE,
                );
            } else {
                ui.painter().rect_filled(cell_rect, rounding, pal.bg_hover);
                draw_icon_image(ui.painter(), cell_rect.center(), 22.0, pal.text_muted);
            }
            if resp.hovered() {
                ui.painter()
                    .rect_filled(cell_rect, rounding, Color32::from_black_alpha(40));
            }
            if resp.clicked() {
                action = Some(BubbleAction::OpenImagePopup {
                    tex_key: cell.key.clone(),
                    full_url: cell.open_target.clone(),
                    title: cell.title.clone(),
                    sender: sender_label.to_string(),
                    time: cell.time.clone(),
                });
            }
            x += cell_w + GAP;
            cell_idx += 1;
        }
        y += row_h + GAP;
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Aspect ratios of the mixed landscape/portrait/square photos used on Zalo Web.
    const ASPECTS: [f32; 10] = [
        1.5,
        2.0 / 3.0,
        1.0,
        1.5,
        2.0 / 3.0,
        1.0,
        1.5,
        1.5,
        2.0 / 3.0,
        1.0,
    ];

    #[test]
    fn row_split_matches_zalo_web_measurements() {
        assert_eq!(album_row_lengths(&ASPECTS[..2]), vec![2]);
        assert_eq!(album_row_lengths(&ASPECTS[..3]), vec![3]);
        assert_eq!(album_row_lengths(&ASPECTS[..4]), vec![2, 2]);
        assert_eq!(album_row_lengths(&ASPECTS[..5]), vec![2, 3]);
        assert_eq!(album_row_lengths(&ASPECTS[..6]), vec![2, 2, 2]);
        assert_eq!(album_row_lengths(&ASPECTS[..7]), vec![2, 3, 2]);
        assert_eq!(album_row_lengths(&ASPECTS[..10]), vec![2, 3, 2, 3]);
    }

    #[test]
    fn row_split_covers_every_photo() {
        for n in 1..=40 {
            let aspects: Vec<f32> = (0..n).map(|i| 0.6 + (i % 5) as f32 * 0.3).collect();
            let rows = album_row_lengths(&aspects);
            assert_eq!(rows.iter().sum::<usize>(), n);
            assert!(rows.iter().all(|&r| (1..=MAX_PHOTOS_PER_ROW).contains(&r)));
        }
        assert!(album_row_lengths(&[]).is_empty());
    }

    fn photo(id: &str, layout: u64, idx: u32, total: u32, sender: &str) -> ChatMessage {
        let params = json!({
            "is_group_layout": 1,
            "group_layout_id": layout,
            "id_in_group": idx,
            "total_item_in_group": total,
            "width": 1500,
            "height": 1125,
        })
        .to_string();
        let mut msg = ChatMessage {
            id: id.into(),
            sender: sender.into(),
            ..Default::default()
        };
        msg.wire_ref
            .fields
            .insert("content".into(), json!({ "params": params }));
        msg
    }

    #[test]
    fn parses_album_params_from_live_shape() {
        let info = album_info(&photo("1", 1794398852905, 16, 22, "a")).unwrap();
        assert_eq!(info.layout_id, "1794398852905");
        assert_eq!((info.index, info.total), (16, 22));
        assert_eq!(info.dims, Some((1500.0, 1125.0)));
        // A photo without the group fields is a normal single photo.
        let mut single = ChatMessage::default();
        single.wire_ref.fields.insert(
            "content".into(),
            json!({ "params": r#"{"width":10,"height":10}"# }),
        );
        assert!(album_info(&single).is_none());
    }

    #[test]
    fn runs_group_only_contiguous_same_sender_same_layout() {
        let messages = vec![
            photo("1", 7, 0, 3, "a"),
            photo("2", 7, 1, 3, "a"),
            photo("3", 7, 2, 3, "a"),
            photo("4", 8, 0, 2, "a"),
            photo("5", 9, 0, 2, "a"),
            photo("6", 9, 1, 2, "b"),
        ];
        let visible: Vec<usize> = (0..messages.len()).collect();
        let runs = album_runs(&messages, &visible);
        assert_eq!(runs, vec![(0, 3, vec![0, 1, 2])]);
    }
}
