//! Photo list behind the image viewer: every photo of the open conversation, oldest first, so the
//! viewer can step through them like Zalo Web does.
//!
//! Behaviour measured on Zalo Web (chat.zalo.me, 2026-11-27, see `.agents/WORKLOG.md`): the viewer
//! lists all photos of the conversation (album members one by one), grouped by day in a vertical
//! strip; ↑/← go to an older photo, ↓/→ to a newer one, with no wrap-around; zoom resets on every
//! step.

use super::*;

/// One photo the viewer can show.
#[derive(Clone)]
pub(super) struct ViewerPhoto {
    pub msg_id: String,
    /// Sent by us; only then may a `local://` original be read without a click.
    pub outgoing: bool,
    pub thumbnail: MessageThumbnail,
    pub full_url: String,
    pub title: String,
    pub sender: String,
    /// `HH:MM` as the message list shows it.
    pub time: String,
    /// Local day number (days since 1970-01-01 in UTC+7), `None` when the message has no timestamp.
    pub day: Option<i64>,
}

/// Offset used for every day label in the chat (Vietnam, UTC+7).
const TZ_OFFSET_SEC: i64 = 7 * 3600;

/// Local day of a message timestamp (seconds or milliseconds).
pub(super) fn local_day(ts: i64) -> Option<i64> {
    let ms = normalize_ts_ms(ts);
    (ms > 0).then(|| (ms / 1000 + TZ_OFFSET_SEC).div_euclid(86_400))
}

/// Today's local day.
pub(super) fn today_local_day() -> i64 {
    (crate::login::unix_now() + TZ_OFFSET_SEC).div_euclid(86_400)
}

/// Day header of the thumbnail strip: "Hôm nay", "Hôm qua", `dd/mm` this year, `dd/mm/yyyy` before.
pub(super) fn day_label(day: i64, today: i64) -> String {
    if day == today {
        return "Hôm nay".to_string();
    }
    if day == today - 1 {
        return "Hôm qua".to_string();
    }
    let (y, m, d) = civil_from_days(day);
    let (this_year, _, _) = civil_from_days(today);
    if y == this_year {
        format!("{d:02}/{m:02}")
    } else {
        format!("{d:02}/{m:02}/{y}")
    }
}

/// "HH:MM · Hôm qua" for the viewer's header.
pub(super) fn when_label(photo: &ViewerPhoto, today: i64) -> String {
    match photo.day {
        Some(day) if photo.time.is_empty() => day_label(day, today),
        Some(day) => format!("{} · {}", photo.time, day_label(day, today)),
        None => photo.time.clone(),
    }
}

/// All photos of a conversation, oldest first, using the same rules as the chat bubbles
/// (`is_zalo_photo_message`, `photo_primary_url`, `photo_open_target`).
pub(super) fn collect_thread_photos(
    messages: &[ChatMessage],
    thread_name: &str,
) -> Vec<ViewerPhoto> {
    messages
        .iter()
        .filter(|msg| msg.text.trim() != "[Tin nhắn đã thu hồi]")
        .filter(|msg| is_zalo_photo_message(msg))
        .filter_map(|msg| {
            let thumb_url = photo_primary_url(msg)?;
            Some(ViewerPhoto {
                msg_id: msg.id.clone(),
                outgoing: msg.is_outgoing,
                thumbnail: MessageThumbnail::new(msg, thumb_url, "thumb_"),
                full_url: photo_open_target(msg, thumb_url),
                title: photo_title_label(msg),
                sender: message_sender_label(msg, thread_name),
                time: msg.time.clone(),
                day: local_day(msg.ts),
            })
        })
        .collect()
}

/// Whether the viewer may fetch this photo's original without a click: only when its thumbnail
/// loads by itself and the original is on a trusted Zalo media host, or is a local file of a
/// photo we sent (the same rule the bubbles use for `local://` thumbnails). Anything else waits
/// for the user's click, like the bubbles do.
pub(super) fn may_auto_fetch(photo: &ViewerPhoto) -> bool {
    photo.thumbnail.auto_loads()
        && ((photo.outgoing && photo.full_url.starts_with("local://"))
            || should_auto_load_message_thumbnail(&photo.full_url))
}

/// Position of the clicked photo: the message itself (with that thumbnail, since one message can
/// hold several photos), then by thumbnail key, then by the URL it opens.
pub(super) fn find_photo(
    photos: &[ViewerPhoto],
    msg_id: &str,
    tex_key: &str,
    full_url: &str,
) -> Option<usize> {
    let by_message = (!msg_id.is_empty())
        .then(|| {
            photos
                .iter()
                .position(|p| p.msg_id == msg_id && p.thumbnail.key == tex_key)
        })
        .flatten();
    by_message
        .or_else(|| photos.iter().rposition(|p| p.thumbnail.key == tex_key))
        .or_else(|| {
            (!full_url.is_empty())
                .then(|| photos.iter().rposition(|p| p.full_url == full_url))
                .flatten()
        })
}

/// Move `delta` photos from `index`, stopping at either end (no wrap-around, like Zalo).
pub(super) fn step_index(index: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    index.saturating_add_signed(delta).min(len - 1)
}

/// Gallery positions whose full-size texture the viewer keeps: the current photo and its
/// neighbours, so stepping either way is instant without holding every original in GPU memory.
pub(super) fn hd_keep_range(index: usize, len: usize) -> std::ops::RangeInclusive<usize> {
    const KEEP: usize = 1;
    let last = len.saturating_sub(1);
    index.saturating_sub(KEEP).min(last)..=(index + KEEP).min(last)
}

/// Viewer side column: width of the ▲/▼ buttons, of the thumbnail strip, and of one thumbnail.
pub(super) const VIEWER_NAV_W: f32 = 64.0;
pub(super) const VIEWER_STRIP_W: f32 = 104.0;
const THUMB: f32 = 76.0;
const THUMB_GAP: f32 = 8.0;
const DAY_HEADER_H: f32 = 26.0;

/// Vertical position of every strip row (content coordinates): an optional day header, then the
/// thumbnail. Pure arithmetic, so the strip can draw only the rows in view.
struct StripRow {
    header: Option<(i64, f32)>,
    cell_y: f32,
}

/// Row positions of the whole strip and its total height.
pub(super) struct StripLayout {
    rows: Vec<StripRow>,
    total_h: f32,
}

fn strip_layout(photos: &[ViewerPhoto]) -> StripLayout {
    let mut rows = Vec::with_capacity(photos.len());
    let mut y = 4.0_f32;
    let mut last_day: Option<Option<i64>> = None;
    for photo in photos {
        let mut header = None;
        if last_day != Some(photo.day) {
            last_day = Some(photo.day);
            if let Some(day) = photo.day {
                header = Some((day, y));
                y += DAY_HEADER_H;
            }
        }
        rows.push(StripRow { header, cell_y: y });
        y += THUMB + THUMB_GAP;
    }
    StripLayout { rows, total_h: y }
}

/// Texture coordinates that centre-crop a `w`×`h` image to a square.
fn square_crop_uv(w: usize, h: usize) -> Rect {
    let aspect = w.max(1) as f32 / h.max(1) as f32;
    if aspect > 1.0 {
        let half = 0.5 / aspect;
        Rect::from_min_max(Pos2::new(0.5 - half, 0.0), Pos2::new(0.5 + half, 1.0))
    } else {
        let half = 0.5 * aspect;
        Rect::from_min_max(Pos2::new(0.0, 0.5 - half), Pos2::new(1.0, 0.5 + half))
    }
}

impl ZaloGuiApp {
    /// The viewer's right column: ▲/▼ buttons and, unless hidden, a vertical strip of every photo
    /// in the conversation grouped under day headers (Zalo Web layout). Only the rows in view are
    /// laid out, fetched and drawn. Returns the photo the user picked, with `true` when it was a
    /// direct click on that photo.
    pub(super) fn render_image_viewer_side(
        &mut self,
        ctx: &egui::Context,
        pal: &ThemePalette,
        viewer: &mut ImageViewerModal,
        side_rect: Rect,
        strip_hidden: bool,
    ) -> Option<(usize, bool)> {
        let photo_count = viewer.photos.len();
        let mut picked: Option<(usize, bool)> = None;
        let nav_rect = Rect::from_min_size(
            side_rect.min,
            Vec2::new(VIEWER_NAV_W.min(side_rect.width()), side_rect.height()),
        );
        let strip_rect =
            Rect::from_min_max(Pos2::new(nav_rect.right(), side_rect.top()), side_rect.max);
        let bg_col = if pal.is_dark {
            Color32::from_rgba_unmultiplied(24, 26, 31, 244)
        } else {
            Color32::from_rgba_unmultiplied(24, 28, 36, 228)
        };
        egui::Area::new(Id::new("image_viewer_side"))
            .order(egui::Order::Foreground)
            .constrain(false)
            .fixed_pos(side_rect.min)
            .show(ctx, |ui| {
                // Claim the whole column so clicks on its empty parts do not reach the chat below.
                let _ = ui.allocate_rect(side_rect, Sense::click());
                let painter = ui.painter().with_clip_rect(side_rect);
                painter.rect_filled(nav_rect, Rounding::ZERO, bg_col);
                if !strip_hidden {
                    painter.rect_filled(strip_rect, Rounding::ZERO, pal.bg_sidebar);
                    painter.vline(
                        strip_rect.left(),
                        strip_rect.y_range(),
                        Stroke::new(1.0_f32, pal.border),
                    );
                }

                // ▲ older / ▼ newer, stacked in the middle like Zalo Web.
                let btn = 40.0_f32;
                let mid = nav_rect.center();
                for (older, center) in [
                    (true, Pos2::new(mid.x, mid.y - btn * 0.5 - 6.0)),
                    (false, Pos2::new(mid.x, mid.y + btn * 0.5 + 6.0)),
                ] {
                    let enabled = if older {
                        viewer.index > 0
                    } else {
                        viewer.index + 1 < photo_count
                    };
                    let r = Rect::from_center_size(center, Vec2::splat(btn));
                    let resp = ui.interact(
                        r,
                        Id::new(("image_viewer_step", older)),
                        if enabled {
                            Sense::click()
                        } else {
                            Sense::hover()
                        },
                    );
                    let fill = if enabled && resp.hovered() {
                        Color32::from_white_alpha(40)
                    } else {
                        Color32::from_white_alpha(18)
                    };
                    painter.circle_filled(r.center(), btn * 0.5, fill);
                    draw_lucide_icon(
                        &painter,
                        r.center(),
                        if older {
                            lucide::CHEVRON_UP
                        } else {
                            lucide::CHEVRON_DOWN
                        },
                        18.0,
                        if enabled {
                            Color32::WHITE
                        } else {
                            Color32::from_white_alpha(70)
                        },
                    );
                    let resp = resp.on_hover_text(if older {
                        "Ảnh cũ hơn (↑)"
                    } else {
                        "Ảnh mới hơn (↓)"
                    });
                    if enabled && resp.clicked() {
                        let delta = if older { -1 } else { 1 };
                        picked = Some((step_index(viewer.index, photo_count, delta), false));
                    }
                    if enabled && resp.hovered() {
                        ctx.set_cursor_icon(CursorIcon::PointingHand);
                    }
                }

                if strip_hidden {
                    return;
                }
                let inner = strip_rect.shrink2(Vec2::new(0.0, 8.0));
                let mut strip_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(inner)
                        .layout(Layout::top_down(Align::Min)),
                );
                strip_ui.set_clip_rect(inner);
                let today = today_local_day();
                let current_day = viewer.photos.get(viewer.index).and_then(|p| p.day);
                let layout = viewer
                    .strip_layout
                    .get_or_insert_with(|| std::sync::Arc::new(strip_layout(&viewer.photos)))
                    .clone();
                let (rows, total_h) = (&layout.rows, layout.total_h);
                let mut scroll = egui::ScrollArea::vertical()
                    .id_salt("image_viewer_strip")
                    .auto_shrink([false, false])
                    .max_height(inner.height());
                if std::mem::take(&mut viewer.scroll_strip) {
                    if let Some(row) = rows.get(viewer.index) {
                        let target = row.cell_y + THUMB * 0.5 - inner.height() * 0.5;
                        let max = (total_h - inner.height()).max(0.0);
                        scroll = scroll.vertical_scroll_offset(target.clamp(0.0, max));
                    }
                }
                scroll.show_viewport(&mut strip_ui, |ui, viewport| {
                    ui.set_height(total_h);
                    ui.set_width(inner.width());
                    let origin = ui.max_rect().min;
                    let x = (inner.width() - THUMB) * 0.5;
                    // Rows are sorted by y; start at the first one that reaches the viewport.
                    let first = rows.partition_point(|r| r.cell_y + THUMB < viewport.min.y);
                    for (i, row) in rows.iter().enumerate().skip(first) {
                        let row_top = row.header.map_or(row.cell_y, |(_, y)| y);
                        if row_top > viewport.max.y {
                            break;
                        }
                        let photo = &viewer.photos[i];
                        if let Some((day, y)) = row.header {
                            ui.painter().text(
                                origin + Vec2::new(inner.width() * 0.5, y + DAY_HEADER_H * 0.5),
                                egui::Align2::CENTER_CENTER,
                                day_label(day, today),
                                FontId::proportional(11.5),
                                if Some(day) == current_day {
                                    pal.text_primary
                                } else {
                                    pal.text_muted
                                },
                            );
                        }
                        let cell = Rect::from_min_size(
                            origin + Vec2::new(x, row.cell_y),
                            Vec2::splat(THUMB),
                        );
                        let resp =
                            ui.interact(cell, Id::new(("image_viewer_cell", i)), Sense::click());
                        let is_current = i == viewer.index;
                        let texture = photo
                            .thumbnail
                            .texture(ui.ctx(), &self.sticker_textures)
                            .filter(|t| t.size()[0] > 2 && t.size()[1] > 2)
                            .cloned();
                        let needs_click = texture.is_none()
                            && photo.thumbnail.needs_click(&self.failed_message_thumbnails);
                        let key = &photo.thumbnail.key;
                        let was_known = self.sticker_textures.contains_key(key)
                            || self.loading_stickers.contains(key);
                        photo.thumbnail.request(
                            ui.ctx(),
                            &self.sticker_textures,
                            &mut self.loading_stickers,
                            &self.failed_message_thumbnails,
                            needs_click && resp.clicked(),
                        );
                        if !was_known && self.loading_stickers.contains(key) {
                            viewer.thumbs_owned.push(key.clone());
                        }
                        let rounding = Rounding::same(6.0);
                        ui.painter().rect_filled(cell, rounding, pal.bg_elevated);
                        if let Some(tex) = texture {
                            let [tw, th] = tex.size();
                            egui::Image::new((tex.id(), cell.size()))
                                .uv(square_crop_uv(tw, th))
                                .rounding(rounding)
                                .paint_at(ui, cell);
                        } else if needs_click {
                            ui.painter().text(
                                cell.center(),
                                egui::Align2::CENTER_CENTER,
                                "Hiện ảnh",
                                FontId::proportional(11.0),
                                pal.text_secondary,
                            );
                        } else {
                            draw_icon_image(ui.painter(), cell.center(), 20.0, pal.text_muted);
                        }
                        if is_current {
                            ui.painter().rect_stroke(
                                cell.expand(2.0),
                                Rounding::same(8.0),
                                Stroke::new(2.0_f32, Color32::WHITE),
                            );
                        } else if resp.hovered() {
                            ui.painter().rect_stroke(
                                cell,
                                rounding,
                                Stroke::new(1.5_f32, pal.primary),
                            );
                        } else {
                            // Dim the others so the current photo stands out, like Zalo.
                            ui.painter()
                                .rect_filled(cell, rounding, Color32::from_black_alpha(70));
                        }
                        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() && !is_current {
                            picked = Some((i, true));
                        }
                    }
                });
            });
        picked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo_msg(id: &str, url: &str, ts: i64, outgoing: bool) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            sender: "Lan".to_string(),
            text: "[Hình ảnh]".to_string(),
            time: "10:15".to_string(),
            is_outgoing: outgoing,
            ts,
            link_url: Some(url.to_string()),
            link_src: Some("Hình ảnh Zalo".to_string()),
            ..Default::default()
        }
    }

    fn text_msg(id: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            sender: "Lan".to_string(),
            text: "xin chào".to_string(),
            ..Default::default()
        }
    }

    const A: &str = "https://photo-stal-1.zdn.vn/jpg/a.jpg";
    const B: &str = "https://photo-stal-1.zdn.vn/jpg/b.jpg";
    const C: &str = "https://photo-stal-1.zdn.vn/jpg/c.jpg";

    #[test]
    fn collects_photos_in_order_and_skips_text_and_recalled() {
        let mut recalled = photo_msg("r", C, 3, false);
        recalled.text = "[Tin nhắn đã thu hồi]".to_string();
        let messages = vec![
            photo_msg("1", A, 1_700_000_000_000, false),
            text_msg("t"),
            recalled,
            photo_msg("2", B, 1_700_000_100_000, true),
        ];
        let photos = collect_thread_photos(&messages, "Nhóm");
        assert_eq!(photos.len(), 2);
        assert_eq!(photos[0].full_url, A);
        assert_eq!(photos[0].sender, "Lan");
        assert_eq!(photos[1].full_url, B);
        assert_eq!(photos[1].sender, "Bạn");
        assert_eq!(
            photos[0].thumbnail.key,
            format!("thumb_{}", crate::crypto::md5_hex(A))
        );
    }

    #[test]
    fn album_members_are_separate_photos() {
        let messages = vec![
            photo_msg("1", A, 1_700_000_000_000, false),
            photo_msg("2", B, 1_700_000_000_000, false),
            photo_msg("3", C, 1_700_000_000_000, false),
        ];
        assert_eq!(collect_thread_photos(&messages, "Nhóm").len(), 3);
    }

    #[test]
    fn finds_clicked_photo_by_key_then_url() {
        let photos = collect_thread_photos(
            &[photo_msg("1", A, 1, false), photo_msg("2", B, 2, false)],
            "Nhóm",
        );
        let key_b = format!("thumb_{}", crate::crypto::md5_hex(B));
        assert_eq!(find_photo(&photos, "", &key_b, ""), Some(1));
        assert_eq!(find_photo(&photos, "", "thumb_other", A), Some(0));
        assert_eq!(
            find_photo(&photos, "", "thumb_other", "https://x.zdn.vn/z.jpg"),
            None
        );
        assert_eq!(find_photo(&photos, "", "thumb_other", ""), None);
    }

    #[test]
    fn same_thumbnail_in_two_messages_opens_the_clicked_one() {
        let photos = collect_thread_photos(
            &[
                photo_msg("1", A, 1, false),
                photo_msg("2", B, 2, false),
                photo_msg("3", A, 3, false),
            ],
            "Nhóm",
        );
        let key_a = format!("thumb_{}", crate::crypto::md5_hex(A));
        assert_eq!(find_photo(&photos, "1", &key_a, A), Some(0));
        assert_eq!(find_photo(&photos, "3", &key_a, A), Some(2));
        // Unknown message id: fall back to the newest photo with that thumbnail.
        assert_eq!(find_photo(&photos, "9", &key_a, A), Some(2));
    }

    #[test]
    fn local_originals_auto_load_only_for_our_own_photos() {
        let mut theirs = photo_msg("1", A, 1, false);
        theirs.link_thumb = Some(A.to_string());
        theirs.link_url = Some("local:///tmp/x.jpg".to_string());
        let mut ours = theirs.clone();
        ours.is_outgoing = true;
        let photos = collect_thread_photos(&[theirs, ours], "Nhóm");
        assert_eq!(photos.len(), 2);
        assert_eq!(photos[0].full_url, "local:///tmp/x.jpg");
        assert!(!may_auto_fetch(&photos[0]));
        assert!(may_auto_fetch(&photos[1]));
    }

    #[test]
    fn strip_layout_adds_one_header_per_day() {
        let day = 20_000_i64 * 86_400_000 + 3_600_000;
        let photos = collect_thread_photos(
            &[
                photo_msg("1", A, day, false),
                photo_msg("2", B, day + 60_000, false),
                photo_msg("3", C, day + 86_400_000, false),
            ],
            "Nhóm",
        );
        let StripLayout {
            rows,
            total_h: total,
        } = strip_layout(&photos);
        assert!(rows[0].header.is_some());
        assert!(rows[1].header.is_none());
        assert!(rows[2].header.is_some());
        assert!(rows.windows(2).all(|w| w[0].cell_y < w[1].cell_y));
        assert_eq!(total, rows[2].cell_y + THUMB + THUMB_GAP);
    }

    #[test]
    fn square_crop_keeps_the_middle() {
        let wide = square_crop_uv(200, 100);
        assert!((wide.width() - 0.5).abs() < 1e-6 && (wide.center().x - 0.5).abs() < 1e-6);
        assert!((wide.height() - 1.0).abs() < 1e-6);
        let tall = square_crop_uv(100, 400);
        assert!((tall.height() - 0.25).abs() < 1e-6 && (tall.center().y - 0.5).abs() < 1e-6);
    }

    #[test]
    fn stepping_stops_at_both_ends() {
        assert_eq!(step_index(0, 5, -1), 0);
        assert_eq!(step_index(0, 5, 1), 1);
        assert_eq!(step_index(4, 5, 1), 4);
        assert_eq!(step_index(3, 5, -1), 2);
        assert_eq!(step_index(0, 0, 1), 0);
    }

    #[test]
    fn keeps_neighbours_only() {
        assert_eq!(hd_keep_range(0, 5), 0..=1);
        assert_eq!(hd_keep_range(2, 5), 1..=3);
        assert_eq!(hd_keep_range(4, 5), 3..=4);
        assert_eq!(hd_keep_range(0, 1), 0..=0);
    }

    #[test]
    fn day_labels_follow_zalo() {
        // 2026-11-28 is day 20_785 since the epoch.
        let today = 20_785;
        assert_eq!(civil_from_days(today), (2026, 11, 28));
        assert_eq!(day_label(today, today), "Hôm nay");
        assert_eq!(day_label(today - 1, today), "Hôm qua");
        assert_eq!(day_label(today - 5, today), "23/11");
        assert_eq!(day_label(today - 365, today), "28/11/2025");
    }

    #[test]
    fn local_day_uses_utc_plus_seven() {
        // 2026-11-27 17:30 UTC is already 2026-11-28 00:30 in Vietnam.
        let ts_ms = (20_784_i64 * 86_400 + 17 * 3600 + 30 * 60) * 1000;
        assert_eq!(local_day(ts_ms), Some(20_785));
        assert_eq!(local_day(ts_ms / 1000), Some(20_785));
        assert_eq!(local_day(0), None);
    }
}
