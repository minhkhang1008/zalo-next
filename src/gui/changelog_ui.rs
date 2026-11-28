//! Release notes shown in Settings → Cập nhật and in the update dialog: the GitHub Release body
//! grouped by its sections (Tính năng mới, Sửa lỗi, …) instead of one flat list.

use eframe::egui::{self, Color32, Margin, RichText, Rounding, Stroke};

use super::modals::render_markdown_bullet_line;
use super::{lucide, ThemePalette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SectionKind {
    Added,
    Fixed,
    Changed,
    Security,
    Removed,
    Other,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Section {
    pub kind: SectionKind,
    pub title: String,
    pub items: Vec<String>,
}

impl Section {
    fn new(kind: SectionKind, title: &str) -> Self {
        Self {
            kind,
            title: title.to_string(),
            items: Vec::new(),
        }
    }
}

fn section_kind(heading: &str) -> SectionKind {
    let h = heading.to_lowercase();
    if h.contains("tính năng") || h.contains("added") || h.contains("mới") {
        SectionKind::Added
    } else if h.contains("sửa lỗi") || h.contains("fixed") {
        SectionKind::Fixed
    } else if h.contains("bảo mật") || h.contains("security") {
        SectionKind::Security
    } else if h.contains("gỡ bỏ") || h.contains("đã bỏ") || h.contains("removed") {
        SectionKind::Removed
    } else if h.contains("thay đổi") || h.contains("changed") {
        SectionKind::Changed
    } else {
        SectionKind::Other
    }
}

/// "Sửa lỗi (Fixed)" → "Sửa lỗi": the CHANGELOG puts the English name in brackets.
fn section_title(heading: &str) -> String {
    let title = heading.trim_start_matches('#').trim();
    match title.find(" (") {
        Some(open) if title.ends_with(')') => title[..open].trim().to_string(),
        _ => title.to_string(),
    }
}

/// "**Title (`src/x.rs`):** long text" → "Title"; other bullets are kept as written.
fn short_title(bullet: &str) -> String {
    let Some(rest) = bullet.strip_prefix("**") else {
        return bullet.to_string();
    };
    let Some(end) = rest.find("**") else {
        return bullet.to_string();
    };
    let mut title = rest[..end].trim().trim_end_matches(':').trim().to_string();
    // Drop a trailing file-reference group such as "(`src/chat.rs`, `src/gui/x.rs`)".
    if let Some(open) = title.rfind(" (`") {
        if title.ends_with(')') {
            title.truncate(open);
        }
    }
    title
}

fn is_noise(bullet: &str) -> bool {
    let low = bullet.to_lowercase();
    low.is_empty()
        || low.starts_with("bộ cài đặt:")
        || low.starts_with("bản chạy ngay")
        || low.starts_with("có gì mới:")
        || low.starts_with("cài đặt:")
        || ["sudo apt", "sudo dnf", ".deb", ".rpm", ".dmg", ".appimage"]
            .iter()
            .any(|needle| low.contains(needle))
        // GitHub's auto-generated commit lines.
        || (low.contains(" by @") && low.contains(" in https://"))
}

/// Group a GitHub Release body into sections. Skips the release title, code blocks, install
/// instructions, link-only pointers and duplicate items; empty sections are dropped.
pub(super) fn parse_release_notes(markdown: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut in_code = false;
    let mut skipping = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            let low = trimmed.to_lowercase();
            skipping = low.contains("bản cài đặt")
                || low.contains("hướng dẫn cài")
                || low.contains("installation");
            // "## Zalo-next v0.4.0" names the release, not a section.
            let is_release_title = trimmed.starts_with("## ")
                && trimmed
                    .split_whitespace()
                    .last()
                    .and_then(|word| word.strip_prefix('v'))
                    .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
            if !skipping && !is_release_title {
                sections.push(Section::new(section_kind(trimmed), &section_title(trimmed)));
            }
            continue;
        }
        if skipping {
            continue;
        }
        let Some(bullet) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .map(str::trim)
        else {
            continue;
        };
        if is_noise(bullet) {
            continue;
        }
        let item = short_title(bullet);
        if sections.is_empty() {
            sections.push(Section::new(SectionKind::Other, "Điểm mới"));
        }
        if let Some(section) = sections.last_mut() {
            if !section.items.contains(&item) {
                section.items.push(item);
            }
        }
    }
    sections.retain(|section| !section.items.is_empty());
    sections
}

fn section_style(kind: SectionKind, pal: &ThemePalette) -> (&'static str, Color32) {
    match kind {
        SectionKind::Added => (lucide::SPARKLES, pal.accent_online),
        SectionKind::Fixed => (lucide::CHECK, pal.warning),
        SectionKind::Changed => (lucide::SLIDERS, pal.primary_hover),
        SectionKind::Security => (lucide::SHIELD_CHECK, pal.primary_hover),
        SectionKind::Removed => (lucide::TRASH_2, pal.danger),
        SectionKind::Other => (lucide::INFO, pal.text_secondary),
    }
}

/// Draw the sections. With `per_section` set, each section shows that many items and a
/// "+N mục khác" line; the full list stays on GitHub.
pub(super) fn render_release_notes(
    ui: &mut egui::Ui,
    sections: &[Section],
    pal: &ThemePalette,
    per_section: Option<usize>,
) {
    for (index, section) in sections.iter().enumerate() {
        if index > 0 {
            ui.add_space(10.0);
        }
        let (icon, color) = section_style(section.kind, pal);
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon).size(13.0).color(color));
            ui.label(
                RichText::new(&section.title)
                    .size(12.6)
                    .strong()
                    .color(pal.text_primary),
            );
            egui::Frame::none()
                .fill(color.linear_multiply(0.16))
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::symmetric(6.0, 1.0))
                .stroke(Stroke::NONE)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(section.items.len().to_string())
                            .size(11.0)
                            .color(color),
                    );
                });
        });
        ui.add_space(4.0);
        let shown = per_section.unwrap_or(usize::MAX).min(section.items.len());
        ui.indent(("release_notes_section", index), |ui| {
            for item in &section.items[..shown] {
                render_markdown_bullet_line(ui, item, 12.0, color, pal.text_secondary, pal);
                ui.add_space(3.0);
            }
            let hidden = section.items.len() - shown;
            if hidden > 0 {
                ui.label(
                    RichText::new(format!("+{hidden} mục khác"))
                        .size(11.5)
                        .color(pal.text_muted),
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTES: &str = "## Zalo-next v0.4.0\n\n### Tính năng mới\n\
- Tùy biến giao diện\n\
- **Tìm và gửi GIF bằng lệnh `@gif` (`src/chat.rs`, `src/gui/workspace.rs`):** `@gif` hiện GIF…\n\
- Tùy biến giao diện\n\
- Có gì mới: [CHANGELOG.md](https://x)\n\
\n### Sửa lỗi (Fixed)\n\
- Cuộc gọi đến\n\
* feat(ui): stuff by @someone in https://github.com/x/pull/1\n\
\n### Bảo mật\n\
\n## Bản cài đặt\n\
- Bộ cài đặt: zalo-next.dmg\n\
\n```bash\n- not a bullet\n```\n\
Chi tiết: [CHANGELOG.md](https://x)\n";

    #[test]
    fn groups_items_by_section_and_skips_noise() {
        let sections = parse_release_notes(NOTES);
        assert_eq!(
            sections,
            vec![
                Section {
                    kind: SectionKind::Added,
                    title: "Tính năng mới".into(),
                    items: vec![
                        "Tùy biến giao diện".into(),
                        "Tìm và gửi GIF bằng lệnh `@gif`".into()
                    ],
                },
                Section {
                    kind: SectionKind::Fixed,
                    title: "Sửa lỗi".into(),
                    items: vec!["Cuộc gọi đến".into()],
                },
            ]
        );
    }

    #[test]
    fn bullets_without_a_heading_go_to_a_generic_section() {
        let sections = parse_release_notes("- Một thay đổi\n- Thêm nữa\n");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].kind, SectionKind::Other);
        assert_eq!(sections[0].items.len(), 2);
    }

    #[test]
    fn maps_english_and_vietnamese_headings() {
        assert_eq!(section_kind("### Added"), SectionKind::Added);
        assert_eq!(
            section_kind("### Security (Bảo mật)"),
            SectionKind::Security
        );
        assert_eq!(section_kind("### Thay đổi"), SectionKind::Changed);
        assert_eq!(section_kind("### Đã gỡ bỏ"), SectionKind::Removed);
        assert_eq!(section_kind("### Đã bỏ"), SectionKind::Removed);
    }

    #[test]
    fn only_the_versioned_title_is_skipped() {
        let sections =
            parse_release_notes("## Zalo-next v1.2.3\n## Sửa lỗi và cải tiến\n- Một lỗi\n");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].kind, SectionKind::Fixed);
        assert_eq!(sections[0].title, "Sửa lỗi và cải tiến");
        assert_eq!(section_title("### Thay đổi (Changed)"), "Thay đổi");
    }
}
