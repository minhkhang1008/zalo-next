//! Composer formatting spans and the layout primitives shared with message bubbles.

use std::ops::Range;

use eframe::egui::{self, Color32, FontId, Id, Key, Modifiers, Stroke, TextFormat};

use crate::chat::{validate_text_styles, ListKind, TextStyle, TextStyleFormat};

use super::theme::{contrast_ratio, ThemePalette};

pub(super) const INDENT_WIDTH: f32 = 16.0;
const FORMAT_TOOLTIP: &str = "Định dạng tin nhắn (Cmd/Ctrl+Shift+X)";

#[derive(Default)]
pub(super) struct RichComposer {
    pub(super) open: bool,
    pub(super) styles: Vec<TextStyle>,
    text: String,
    pending: Option<(usize, TextStyleFormat)>,
    backspace: bool,
}

fn push_span(spans: &mut Vec<TextStyle>, start: usize, end: usize, format: TextStyleFormat) {
    if start >= end {
        return;
    }
    if let Some(last) = spans
        .last_mut()
        .filter(|last| last.end == start && last.format == format)
    {
        last.end = end;
    } else {
        spans.push(TextStyle { start, end, format });
    }
}

/// Include plain gaps and combine overlapping wire spans, such as a list with a bold word.
pub(super) fn style_runs(text: &str, styles: &[TextStyle]) -> Vec<TextStyle> {
    let mut ordered: Vec<_> = styles
        .iter()
        .copied()
        .filter(|span| span.fits(text))
        .collect();
    ordered.sort_by_key(|span| span.start);
    let mut boundaries = vec![0, text.len()];
    for span in &ordered {
        boundaries.extend([span.start, span.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut runs = Vec::new();
    let mut active: Vec<TextStyle> = Vec::new();
    let mut remaining = ordered.into_iter().peekable();
    for pair in boundaries.windows(2) {
        let start = pair[0];
        active.retain(|span| span.end > start);
        while let Some(span) = remaining.next_if(|span| span.start <= start) {
            active.push(span);
        }
        let mut format = TextStyleFormat::default();
        for span in &active {
            format.overlay(span.format);
        }
        push_span(&mut runs, start, pair[1], format);
    }
    runs
}

pub(super) fn runs_in(
    runs: &[TextStyle],
    range: Range<usize>,
) -> impl Iterator<Item = TextStyle> + '_ {
    let Range { start, end } = range;
    let first = runs.partition_point(|span| span.end <= start);
    runs[first..]
        .iter()
        .copied()
        .take_while(move |span| start < end && span.start < end)
        .map(move |mut span| {
            span.start = span.start.max(start);
            span.end = span.end.min(end);
            span
        })
}

#[derive(Debug)]
pub(super) struct Paragraph {
    pub range: Range<usize>,
    pub prefix: Option<String>,
    pub indent: u8,
}

pub(super) fn paragraphs(text: &str, runs: &[TextStyle]) -> Vec<Paragraph> {
    let mut start = 0;
    let mut number = 0;
    text.split_inclusive('\n')
        .map(|line| {
            let end = start + line.len();
            let format = paragraph_style(runs, start..end);
            let paragraph = Paragraph {
                range: start..end,
                prefix: list_prefix(format.list, &mut number),
                indent: format.indent,
            };
            start = end;
            paragraph
        })
        .collect()
}

fn list_prefix(list: Option<ListKind>, number: &mut usize) -> Option<String> {
    if list == Some(ListKind::Numbered) {
        *number += 1;
        return Some(format!("{number}. "));
    }
    *number = 0;
    (list == Some(ListKind::Bullet)).then(|| "• ".to_owned())
}

fn paragraph_style(runs: &[TextStyle], range: Range<usize>) -> TextStyleFormat {
    let mut paragraph = TextStyleFormat::default();
    for span in runs_in(runs, range) {
        paragraph.list = paragraph.list.or(span.format.list);
        paragraph.indent = paragraph.indent.max(span.format.indent);
    }
    paragraph
}

pub(super) fn text_format(
    style: TextStyleFormat,
    base: &TextFormat,
    background: Option<Color32>,
    bold_ink: Color32,
    accent: Option<Color32>,
) -> TextFormat {
    let mut format = base.clone();
    format.font_id.size += style
        .size
        .filter(|size| matches!(size, 13 | 15 | 18 | 20))
        .map_or(0.0, |size| f32::from(size) - 15.0);
    // Match the Markdown renderer's proportional-font emphasis.
    if style.bold {
        format.font_id.size += 0.4;
        format.color = bold_ink;
    }
    if let Some([r, g, b]) = style.color {
        let color = Color32::from_rgb(r, g, b);
        format.color = background.map_or(color, |bg| readable_color(color, bg));
    }
    if let Some(color) = accent {
        format.color = color;
    }
    format.italics = style.italic;
    if style.underline {
        format.underline = Stroke::new(1.0_f32, format.color);
    }
    if style.strike {
        format.strikethrough = Stroke::new(1.0_f32, format.color);
    }
    format
}

fn readable_color(color: Color32, bg: Color32) -> Color32 {
    let toward = if contrast_ratio(Color32::WHITE, bg) >= contrast_ratio(Color32::BLACK, bg) {
        255.0
    } else {
        0.0
    };
    (0..=16)
        .map(|step| {
            let mix = step as f32 / 16.0;
            let channel = |value| (f32::from(value) * (1.0 - mix) + toward * mix).round() as u8;
            Color32::from_rgb(channel(color.r()), channel(color.g()), channel(color.b()))
        })
        .find(|&candidate| contrast_ratio(candidate, bg) >= 4.5)
        .unwrap_or(color)
}

fn style_at(styles: &[TextStyle], byte: usize, prefer_left: bool) -> TextStyleFormat {
    let first = styles.partition_point(|span| {
        if prefer_left {
            span.end < byte
        } else {
            span.end <= byte
        }
    });
    styles
        .get(first)
        .filter(|span| {
            if prefer_left {
                span.start < byte
            } else {
                span.start <= byte
            }
        })
        .map_or_else(TextStyleFormat::default, |span| span.format)
}

fn byte_at(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte, _)| byte)
}

fn selection(ctx: &egui::Context, id: Id, text: &str) -> Range<usize> {
    egui::TextEdit::load_state(ctx, id)
        .and_then(|state| state.cursor.char_range())
        .map(|range| {
            let [start, end] = range.sorted();
            byte_at(text, start.index)..byte_at(text, end.index)
        })
        .unwrap_or(text.len()..text.len())
}

fn changed_ranges(
    old: &str,
    new: &str,
    anchor: Option<usize>,
    backspace: bool,
) -> (Range<usize>, Range<usize>) {
    let mut prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum();
    if backspace && old.len() > new.len() {
        let removed = old.len() - new.len();
        if let Some((start, end)) = anchor.and_then(|end| Some((end.checked_sub(removed)?, end))) {
            // Only use the caret if removing exactly this range produces the edited text.
            if start <= prefix && old.is_char_boundary(start) && old.get(end..) == new.get(start..)
            {
                return (start..end, start..start);
            }
        }
    }
    if let Some(anchor) = anchor.filter(|&a| a <= prefix && old.is_char_boundary(a)) {
        prefix = anchor;
    }
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum();
    (prefix..old.len() - suffix, prefix..new.len() - suffix)
}

/// Shift or clip runs around the changed range and insert the effective typing format.
fn edit_styles(
    styles: &[TextStyle],
    old: Range<usize>,
    new: Range<usize>,
    inserted: TextStyleFormat,
) -> Vec<TextStyle> {
    let mut result = Vec::new();
    for span in styles.iter().take_while(|span| span.start < old.start) {
        push_span(
            &mut result,
            span.start,
            span.end.min(old.start),
            span.format,
        );
    }
    if inserted != TextStyleFormat::default() {
        push_span(&mut result, new.start, new.end, inserted);
    }
    for span in styles.iter().filter(|span| span.end > old.end) {
        let start = new.end + (span.start.max(old.end) - old.end);
        let end = new.end + (span.end - old.end);
        push_span(&mut result, start, end, span.format);
    }
    result
}

#[derive(Clone, Copy)]
enum FormatAction {
    Bold,
    Italic,
    Underline,
    Strike,
    Size(Option<u8>),
    Color(Option<[u8; 3]>),
    List(ListKind),
    Indent(bool),
    Clear,
}

impl FormatAction {
    fn edit_inline(self, format: &mut TextStyleFormat, active: TextStyleFormat) {
        match self {
            Self::Bold => format.bold = !active.bold,
            Self::Italic => format.italic = !active.italic,
            Self::Underline => format.underline = !active.underline,
            Self::Strike => format.strike = !active.strike,
            Self::Size(size) => format.size = size,
            Self::Color(color) => format.color = color,
            Self::Clear => {
                let (list, indent) = (format.list, format.indent);
                *format = TextStyleFormat {
                    list,
                    indent,
                    ..Default::default()
                };
            }
            Self::List(_) | Self::Indent(_) => {}
        }
    }

    fn edit_paragraph(self, format: &mut TextStyleFormat, active: TextStyleFormat) {
        match self {
            Self::List(list) => format.list = (Some(list) != active.list).then_some(list),
            Self::Indent(true) => format.indent = (active.indent + 1).min(5),
            Self::Indent(false) => format.indent = active.indent.saturating_sub(1),
            Self::Clear => {
                format.list = None;
                format.indent = 0;
            }
            _ => {}
        }
    }
}

impl RichComposer {
    pub(super) fn restore(&mut self, text: &str, mut styles: Vec<TextStyle>) {
        validate_text_styles(text, &mut styles);
        self.styles = style_runs(text, &styles);
        self.styles
            .retain(|span| span.format != TextStyleFormat::default());
        self.pending = None;
        self.text.clear();
        self.text.push_str(text);
    }

    pub(super) fn clear(&mut self) {
        self.text.clear();
        self.styles.clear();
        self.pending = None;
    }

    pub(super) fn sync(&mut self, text: &str, hint: Option<usize>) {
        let backspace = hint.is_some() && std::mem::take(&mut self.backspace);
        if text == self.text {
            return;
        }
        if text.is_empty() {
            self.clear();
            return;
        }
        if !self.styles.is_empty() || self.pending.is_some() {
            let anchor = hint.or(self.pending.map(|(at, _)| at));
            let (old, new) = changed_ranges(&self.text, text, anchor, backspace);
            let pending = self.pending.take().filter(|(at, _)| *at == old.start);
            let format = pending.map_or_else(
                || style_at(&self.styles, old.start, old.is_empty()),
                |(_, format)| format,
            );
            if pending.is_some() {
                self.pending = Some((new.end, format));
            }
            self.styles = edit_styles(&self.styles, old, new, format);
        }
        self.text.clear();
        self.text.push_str(text);
    }

    fn active(&self, selected: &Range<usize>) -> TextStyleFormat {
        let caret = selected.start;
        let mut inline = self
            .pending
            .filter(|(at, _)| selected.is_empty() && *at == caret)
            .map_or_else(
                || style_at(&self.styles, caret, selected.is_empty()),
                |(_, format)| format,
            );
        let line = touched_lines(&self.text, caret..caret);
        let paragraph = paragraph_style(&self.styles, line.clone());
        if !line.is_empty() {
            inline.list = paragraph.list;
            inline.indent = paragraph.indent;
        }
        inline
    }

    fn apply(&mut self, selected: Range<usize>, action: FormatAction) {
        let active = self.active(&selected);
        let line_action = matches!(action, FormatAction::List(_) | FormatAction::Indent(_));
        if selected.is_empty() {
            let mut pending = active;
            action.edit_inline(&mut pending, active);
            action.edit_paragraph(&mut pending, active);
            self.pending = Some((selected.start, pending));
        } else {
            self.pending = None;
        }
        let mut runs = style_runs(&self.text, &self.styles);
        if !line_action && !selected.is_empty() {
            let range = selected.clone();
            runs = edit_formats(&runs, self.text.len(), range, |format| {
                action.edit_inline(format, active);
            });
        }
        if line_action || matches!(action, FormatAction::Clear) {
            let lines = touched_lines(&self.text, selected);
            let mut start = lines.start;
            let previous = runs;
            let mut edited = Vec::new();
            for span in runs_in(&previous, 0..lines.start) {
                push_span(&mut edited, span.start, span.end, span.format);
            }
            for line in self.text[lines.clone()].split_inclusive('\n') {
                let end = start + line.len();
                let paragraph = paragraph_style(&previous, start..end);
                for mut span in runs_in(&previous, start..end) {
                    // Toggle a list consistently across the selection and indent each line
                    // relative to its own level.
                    let state = if matches!(action, FormatAction::List(_)) {
                        active
                    } else {
                        paragraph
                    };
                    action.edit_paragraph(&mut span.format, state);
                    push_span(&mut edited, span.start, span.end, span.format);
                }
                start = end;
            }
            for span in runs_in(&previous, lines.end..self.text.len()) {
                push_span(&mut edited, span.start, span.end, span.format);
            }
            runs = edited;
        }
        runs.retain(|span| span.format != TextStyleFormat::default());
        self.styles = runs;
    }

    pub(super) fn render_toolbar(&mut self, ui: &mut egui::Ui, id: Id) {
        let focused = ui.memory(|memory| memory.has_focus(id));
        if focused && shortcut(ui, Key::X, true) {
            self.open = !self.open;
        }
        if !self.open {
            return;
        }

        let selected = selection(ui.ctx(), id, &self.text);
        let active = self.active(&selected);
        let mut action = None;
        if focused {
            for (key, op) in [
                (Key::B, FormatAction::Bold),
                (Key::I, FormatAction::Italic),
                (Key::U, FormatAction::Underline),
            ] {
                if shortcut(ui, key, false) {
                    action = Some(op);
                }
            }
        }
        ui.horizontal_wrapped(|ui| {
            for (label, tip, enabled, op) in [
                ("B", "In đậm (Cmd/Ctrl+B)", active.bold, FormatAction::Bold),
                (
                    "I",
                    "In nghiêng (Cmd/Ctrl+I)",
                    active.italic,
                    FormatAction::Italic,
                ),
                (
                    "U",
                    "Gạch chân (Cmd/Ctrl+U)",
                    active.underline,
                    FormatAction::Underline,
                ),
                ("S", "Gạch ngang", active.strike, FormatAction::Strike),
            ] {
                let mut label = egui::RichText::new(label);
                label = match op {
                    FormatAction::Bold => label.strong(),
                    FormatAction::Italic => label.italics(),
                    FormatAction::Underline => label.underline(),
                    FormatAction::Strike => label.strikethrough(),
                    _ => label,
                };
                if ui
                    .selectable_label(enabled, label)
                    .on_hover_text(tip)
                    .clicked()
                {
                    action = Some(op);
                }
            }
            let sizes = [
                (13, "Nhỏ 13"),
                (15, "Thường 15"),
                (18, "Lớn 18"),
                (20, "Rất lớn 20"),
            ];
            let current_size = active.size.unwrap_or(15);
            let size_label = sizes
                .iter()
                .find(|(size, _)| *size == current_size)
                .map_or("Cỡ chữ", |(_, label)| *label);
            ui.menu_button(size_label, |ui| {
                for (size, label) in sizes {
                    if ui.selectable_label(current_size == size, label).clicked() {
                        action = Some(FormatAction::Size((size != 15).then_some(size)));
                        ui.close_menu();
                    }
                }
            })
            .response
            .on_hover_text("Cỡ chữ");
            let colors = [
                (None, "Mặc định"),
                (Some([0xdb, 0x34, 0x2e]), "Đỏ"),
                (Some([0xf2, 0x78, 0x06]), "Cam"),
                (Some([0xf7, 0xb5, 0x03]), "Vàng"),
                (Some([0x15, 0xa8, 0x5f]), "Xanh lá"),
            ];
            let current_color = colors
                .iter()
                .find(|(color, _)| *color == active.color)
                .map_or("Màu chữ", |(_, label)| *label);
            ui.menu_button(current_color, |ui| {
                for (color, label) in colors {
                    if ui.selectable_label(active.color == color, label).clicked() {
                        action = Some(FormatAction::Color(color));
                        ui.close_menu();
                    }
                }
            })
            .response
            .on_hover_text("Màu chữ");
            for (list, label, tip) in [
                (ListKind::Bullet, "•", "Danh sách dấu đầu dòng"),
                (ListKind::Numbered, "1.", "Danh sách đánh số"),
            ] {
                if ui
                    .selectable_label(active.list == Some(list), label)
                    .on_hover_text(tip)
                    .clicked()
                {
                    action = Some(FormatAction::List(list));
                }
            }
            if ui
                .add_enabled(active.indent > 0, egui::Button::new("Thụt lề −"))
                .on_hover_text("Giảm thụt lề")
                .clicked()
            {
                action = Some(FormatAction::Indent(false));
            }
            if ui
                .add_enabled(active.indent < 5, egui::Button::new("Thụt lề +"))
                .on_hover_text("Tăng thụt lề")
                .clicked()
            {
                action = Some(FormatAction::Indent(true));
            }
            if ui.button("Xóa định dạng").clicked() {
                action = Some(FormatAction::Clear);
            }
        });
        if let Some(action) = action {
            self.apply(selected, action);
            ui.memory_mut(|memory| memory.request_focus(id));
        }
    }

    pub(super) fn toggle_button(&mut self, ui: &mut egui::Ui, id: Id) {
        if ui
            .selectable_label(self.open, "Định dạng")
            .on_hover_text(FORMAT_TOOLTIP)
            .clicked()
        {
            self.open = !self.open;
            ui.memory_mut(|memory| memory.request_focus(id));
            ui.ctx().request_repaint();
        }
    }

    pub(super) fn caret_changed(&mut self, ctx: &egui::Context, id: Id) {
        if let Some((at, _)) = self.pending {
            if selection(ctx, id, &self.text) != (at..at) {
                self.pending = None;
            }
        }
    }

    pub(super) fn edit_hint(&mut self, ctx: &egui::Context, id: Id) -> Option<usize> {
        self.backspace = false;
        if self.styles.is_empty() && self.pending.is_none() {
            return None;
        }
        let selected = selection(ctx, id, &self.text);
        // egui lays out the edited text before it saves the new caret.
        self.backspace = selected.is_empty()
            && ctx.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Key {
                        key: Key::Backspace,
                        pressed: true,
                        ..
                    } => Some(true),
                    egui::Event::Key {
                        key: Key::Delete,
                        pressed: true,
                        ..
                    } => Some(false),
                    _ => None,
                }) == Some(true)
            });
        Some(selected.start)
    }
}

fn edit_formats(
    runs: &[TextStyle],
    len: usize,
    range: Range<usize>,
    edit: impl Fn(&mut TextStyleFormat),
) -> Vec<TextStyle> {
    let mut result = Vec::new();
    for (part, apply) in [
        (0..range.start, false),
        (range.clone(), true),
        (range.end..len, false),
    ] {
        for mut span in runs_in(runs, part) {
            if apply {
                edit(&mut span.format);
            }
            push_span(&mut result, span.start, span.end, span.format);
        }
    }
    result
}

fn touched_lines(text: &str, selected: Range<usize>) -> Range<usize> {
    let start = text[..selected.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    if !selected.is_empty() && text[..selected.end].ends_with('\n') {
        return start..selected.end;
    }
    let end = text[selected.end..]
        .find('\n')
        .map_or(text.len(), |index| selected.end + index + 1);
    start..end
}

fn shortcut(ui: &egui::Ui, key: Key, shift: bool) -> bool {
    ui.input_mut(|input| {
        input.consume_key(
            Modifiers {
                shift,
                ..Modifiers::COMMAND
            },
            key,
        )
    })
}

pub(super) fn composer_job(
    ui: &egui::Ui,
    text: &str,
    styles: &[TextStyle],
    mentions: &[Range<usize>],
    pal: &ThemePalette,
    size: f32,
) -> egui::text::LayoutJob {
    let base = TextFormat::simple(FontId::proportional(size), pal.text_primary);
    let tag_color = super::messages::mention_link_color(pal);
    let background = pal.is_dark.then_some(pal.bg_input);
    let runs = style_runs(text, styles);
    let mut job = egui::text::LayoutJob::default();
    for paragraph in paragraphs(text, &runs) {
        let mut leading = f32::from(paragraph.indent) * INDENT_WIDTH;
        if let Some(prefix) = paragraph.prefix {
            leading += prefix_width(ui, &prefix, &base);
        }
        for run in runs_in(&runs, paragraph.range) {
            let mut start = run.start;
            let in_run = mentions.partition_point(|tag| tag.end <= start);
            for tag in mentions[in_run..]
                .iter()
                .take_while(|tag| tag.start < run.end)
            {
                let tag_start = tag.start.max(start);
                let tag_end = tag.end.min(run.end);
                if start < tag_start {
                    job.append(
                        &text[start..tag_start],
                        std::mem::take(&mut leading),
                        text_format(run.format, &base, background, pal.text_primary, None),
                    );
                }
                job.append(
                    &text[tag_start..tag_end],
                    std::mem::take(&mut leading),
                    text_format(
                        run.format,
                        &base,
                        background,
                        pal.text_primary,
                        Some(tag_color),
                    ),
                );
                start = tag_end;
            }
            if start < run.end {
                job.append(
                    &text[start..run.end],
                    std::mem::take(&mut leading),
                    text_format(run.format, &base, background, pal.text_primary, None),
                );
            }
        }
    }
    if text.is_empty() {
        job.append("", 0.0, base);
    }
    job
}

fn prefix_width(ui: &egui::Ui, prefix: &str, base: &TextFormat) -> f32 {
    ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(prefix.to_owned(), base.font_id.clone(), base.color)
            .size()
            .x
    })
}

/// Draw list prefixes in the gutter so the editable galley has exactly the source characters.
pub(super) fn paint_prefixes(
    ui: &egui::Ui,
    output: &egui::text_edit::TextEditOutput,
    composer: &RichComposer,
    color: Color32,
    size: f32,
) {
    if composer.styles.is_empty() {
        return;
    }
    let runs = style_runs(&composer.text, &composer.styles);
    let painter = ui.painter().with_clip_rect(output.text_clip_rect);
    let mut char_index = 0;
    for paragraph in paragraphs(&composer.text, &runs) {
        if let Some(prefix) = paragraph.prefix {
            let cursor = output.galley.from_ccursor(egui::text::CCursor {
                index: char_index,
                prefer_next_row: true,
            });
            let bounds = output.galley.pos_from_cursor(&cursor);
            let x = f32::from(paragraph.indent) * INDENT_WIDTH;
            painter.text(
                output.galley_pos + egui::vec2(x, bounds.center().y),
                egui::Align2::LEFT_CENTER,
                prefix,
                FontId::proportional(size),
                color,
            );
        }
        char_index += composer.text[paragraph.range].chars().count();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold(start: usize, end: usize) -> TextStyle {
        TextStyle {
            start,
            end,
            format: TextStyleFormat {
                bold: true,
                ..Default::default()
            },
        }
    }

    #[test]
    fn styled_spans_survive_insert_delete_and_replacement() {
        let mut composer = RichComposer::default();
        composer.restore("xế😀y", vec![bold(1, 8)]);
        composer.sync("xế😀!y", None);
        assert_eq!(composer.styles, [bold(1, 9)]);
        composer.sync("xế!y", None);
        assert_eq!(composer.styles, [bold(1, 5)]);
        composer.sync("xĐ!y", None);
        assert_eq!(composer.styles, [bold(1, 4)]);
        composer.sync("xy", None);
        assert!(composer.styles.is_empty());
    }

    fn delete_at_editor_caret(key: Key, caret: usize, character: char) -> Vec<TextStyle> {
        let ctx = egui::Context::default();
        let id = Id::new("deletion-test-editor");
        let mut text = character.to_string().repeat(4);
        let width = character.len_utf8();
        let mut composer = RichComposer::default();
        composer.restore(&text, vec![bold(width, 2 * width)]);
        let mut state = egui::text_edit::TextEditState::default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(caret),
            )));
        state.store(&ctx, id);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| {
                ctx.memory_mut(|memory| memory.request_focus(id));
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut hint = composer.edit_hint(ctx, id);
                    let mut initial_layout = true;
                    let mut layouter = |ui: &egui::Ui, text: &str, width: f32| {
                        let hint = if std::mem::take(&mut initial_layout) {
                            None
                        } else {
                            hint.take()
                        };
                        composer.sync(text, hint);
                        ui.fonts(|fonts| {
                            fonts.layout(text.to_owned(), FontId::default(), Color32::WHITE, width)
                        })
                    };
                    egui::TextEdit::singleline(&mut text)
                        .id(id)
                        .layouter(&mut layouter)
                        .show(ui);
                    composer.caret_changed(ctx, id);
                });
            },
        );
        assert_eq!(text, character.to_string().repeat(3));
        composer.styles
    }

    #[test]
    fn backspace_uses_the_character_before_the_caret_in_repeated_text() {
        for character in ['a', 'ế', '😀'] {
            assert!(delete_at_editor_caret(Key::Backspace, 2, character).is_empty());
            let width = character.len_utf8();
            assert_eq!(
                delete_at_editor_caret(Key::Backspace, 1, character),
                [bold(0, width)]
            );
        }
    }

    #[test]
    fn delete_uses_the_character_after_the_caret_in_repeated_text() {
        for character in ['a', 'ế', '😀'] {
            assert!(delete_at_editor_caret(Key::Delete, 1, character).is_empty());
            let width = character.len_utf8();
            assert_eq!(
                delete_at_editor_caret(Key::Delete, 2, character),
                [bold(width, 2 * width)]
            );
        }
    }

    #[test]
    fn pending_off_overrides_inherited_style_during_typing() {
        let mut composer = RichComposer::default();
        composer.restore("aa", vec![bold(0, 2)]);
        composer.apply(2..2, FormatAction::Bold);
        composer.sync("aaa", None);
        composer.sync("aaaa", None);
        assert_eq!(composer.styles, [bold(0, 2)]);
        composer.sync("", None);
        assert!(composer.styles.is_empty() && composer.pending.is_none());
    }

    #[test]
    fn formatting_selection_splits_runs_without_losing_other_attributes() {
        let mut composer = RichComposer::default();
        composer.restore("ế😀x", vec![bold(0, 8)]);
        composer.apply(3..7, FormatAction::Italic);
        assert_eq!(composer.styles.len(), 3);
        assert!(composer.styles[1].format.bold && composer.styles[1].format.italic);
        composer.apply(3..7, FormatAction::Clear);
        assert_eq!(composer.styles, [bold(0, 3), bold(7, 8)]);
    }

    #[test]
    fn line_formatting_uses_full_lines_and_keeps_next_line_unchanged() {
        let mut composer = RichComposer::default();
        composer.restore("một\nhai\nba", Vec::new());
        let end = composer.text.rfind("ba").unwrap();
        composer.apply(1..end, FormatAction::List(ListKind::Numbered));
        composer.apply(1..end, FormatAction::Indent(true));
        let lines = paragraphs(
            &composer.text,
            &style_runs(&composer.text, &composer.styles),
        );
        assert_eq!(lines[0].prefix.as_deref(), Some("1. "));
        assert_eq!(lines[1].prefix.as_deref(), Some("2. "));
        assert_eq!(lines[1].indent, 1);
        assert_eq!(lines[2].prefix, None);
    }

    #[test]
    fn composer_shortcuts_format_the_saved_selection() {
        let ctx = egui::Context::default();
        let id = Id::new("shortcut-test-editor");
        let text = "ế😀x";
        let mut composer = RichComposer::default();
        composer.restore(text, Vec::new());
        let mut state = egui::text_edit::TextEditState::default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(1),
                egui::text::CCursor::new(2),
            )));
        state.store(&ctx, id);
        let events = [(Key::X, true), (Key::B, false)]
            .into_iter()
            .map(|(key, shift)| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers {
                    shift,
                    ..Modifiers::COMMAND
                },
            })
            .collect();
        let _ = ctx.run(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ctx| {
                ctx.memory_mut(|memory| memory.request_focus(id));
                egui::CentralPanel::default().show(ctx, |ui| composer.render_toolbar(ui, id));
            },
        );
        assert!(composer.open);
        assert_eq!(composer.styles, [bold(3, 7)]);
    }

    #[test]
    fn selection_uses_egui_character_offsets() {
        let ctx = egui::Context::default();
        let id = Id::new("test-editor");
        let mut state = egui::text_edit::TextEditState::default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(1),
                egui::text::CCursor::new(2),
            )));
        state.store(&ctx, id);
        assert_eq!(selection(&ctx, id, "ế😀x"), 3..7);
    }
}
