use crate::{
    app::{App, FilesPane},
    filesystem::file_type::DetectedFileType,
    preview::{self, PreviewState},
};
use ratatui::{prelude::*, widgets::*};
use std::collections::VecDeque;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const WRAP_CHECKPOINT_ROWS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreviewRenderKey {
    generation: u64,
    width: u16,
    height: u16,
    render_markdown: bool,
    focused_link: Option<usize>,
    line_numbers: bool,
    wrap: bool,
}

#[derive(Debug)]
struct CachedPreview {
    key: PreviewRenderKey,
    lines: Vec<Line<'static>>,
    widest: usize,
    wrapped: Option<WrappedLayout>,
    prewrapped: bool,
}

/// Cached wrapping metadata. `row_prefix` lets a draw binary-search directly to the logical line
/// containing the viewport. The optional width-one indexes cover the pathological (but valid)
/// case of a multi-megabyte logical line in a one-column viewport without storing one entry per
/// rendered row.
#[derive(Debug)]
struct WrappedLayout {
    row_prefix: Vec<usize>,
    line_indexes: Vec<LineWrapIndex>,
}

#[derive(Debug, Clone, Copy, Default)]
struct SourceCursor {
    span: usize,
    byte: usize,
}

#[derive(Debug, Clone)]
struct OwnedGrapheme {
    symbol: String,
    style: Style,
}

#[derive(Debug, Clone, Default)]
struct WrapState {
    pending_line: Vec<OwnedGrapheme>,
    pending_word: Vec<OwnedGrapheme>,
    pending_whitespace: VecDeque<OwnedGrapheme>,
    line_width: usize,
    word_width: usize,
    whitespace_width: usize,
    non_whitespace_previous: bool,
    rows: usize,
}

#[derive(Debug, Clone)]
struct WrapCheckpoint {
    row: usize,
    cursor: SourceCursor,
    state: WrapState,
}

#[derive(Debug)]
struct LineWrapIndex {
    rows: usize,
    checkpoints: Vec<WrapCheckpoint>,
}

#[derive(Debug, Default)]
pub(crate) struct PreviewRenderCache {
    entry: Option<CachedPreview>,
    #[cfg(test)]
    builds: usize,
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let focused_link = app
        .markdown_rendered
        .then_some(app.preview_link_index)
        .flatten();
    let wrap = app.preview_wrap_effective();
    let sticky_gutter = !wrap
        && app.preview_line_numbers
        && matches!(app.preview.file_type, DetectedFileType::Source(_));
    let key = PreviewRenderKey {
        generation: app.preview.generation(),
        width: inner.width,
        height: inner.height,
        render_markdown: app.markdown_rendered,
        focused_link,
        line_numbers: app.preview_line_numbers,
        wrap,
    };
    ensure_render_cache(
        &mut app.preview_render_cache,
        key,
        &app.preview,
        focused_link,
    );
    let cached = app
        .preview_render_cache
        .entry
        .as_ref()
        .expect("preview cache is populated before rendering");
    let gutter_width = sticky_gutter.then(|| source_gutter_width(cached.lines.len()));
    let content_width = inner.width.saturating_sub(gutter_width.unwrap_or_default());
    let rendered_line_count = if wrap {
        if cached.prewrapped {
            cached.lines.len()
        } else {
            cached
                .wrapped
                .as_ref()
                .and_then(|layout| layout.row_prefix.last())
                .copied()
                .unwrap_or_default()
        }
    } else {
        cached.lines.len()
    };
    // Window content before rendering so the public usize scroll position is never truncated by
    // Paragraph's u16 scroll offset, including genuinely wrapped previews taller than 65,535 rows.
    app.preview.set_measurements(
        if sticky_gutter {
            content_width as usize
        } else {
            inner.width as usize
        },
        inner.height as usize,
        rendered_line_count,
        cached.widest,
    );

    let (start, end, total, percent) = preview_progress(&app.preview);
    let focus = if app.files_pane == FilesPane::Preview {
        " • focus"
    } else {
        ""
    };
    let mode = if wrap { "wrap" } else { "nowrap" };
    let title = format!(" Preview{focus} · {mode} · {start}–{end}/{total} · {percent}% ");
    let border = if app.files_pane == FilesPane::Preview {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border));
    frame.render_widget(block, area);

    if wrap {
        if cached.prewrapped {
            let visible = cached
                .lines
                .iter()
                .skip(app.preview.scroll)
                .take(inner.height as usize)
                .cloned()
                .collect::<Vec<_>>();
            frame.render_widget(Paragraph::new(visible), inner);
            return;
        }
        let visible = wrapped_window(
            &cached.lines,
            cached
                .wrapped
                .as_ref()
                .expect("wrapped cache includes layout metadata"),
            inner.width,
            app.preview.scroll,
            inner.height as usize,
        );
        frame.render_widget(Paragraph::new(visible), inner);
        return;
    }
    let visible = cached
        .lines
        .iter()
        .skip(app.preview.scroll)
        .take(inner.height as usize)
        .cloned()
        .collect::<Vec<_>>();
    if let Some(gutter_width) = gutter_width {
        let chunks =
            Layout::horizontal([Constraint::Length(gutter_width), Constraint::Min(0)]).split(inner);
        let (gutters, content): (Vec<_>, Vec<_>) =
            visible.into_iter().map(split_source_gutter).unzip();
        frame.render_widget(Paragraph::new(gutters), chunks[0]);
        frame.render_widget(
            Paragraph::new(crop_lines(
                content,
                app.preview.horizontal_scroll,
                chunks[1].width as usize,
            )),
            chunks[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new(crop_lines(
                visible,
                app.preview.horizontal_scroll,
                inner.width as usize,
            )),
            inner,
        );
    }
}

fn ensure_render_cache(
    cache: &mut PreviewRenderCache,
    key: PreviewRenderKey,
    preview_state: &PreviewState,
    focused_link: Option<usize>,
) {
    if cache.entry.as_ref().is_some_and(|cached| cached.key == key) {
        return;
    }

    // Release the potentially multi-megabyte previous render before building its replacement.
    // This keeps resize/mode invalidation from transiently retaining two complete previews.
    drop(cache.entry.take());

    let lines = preview::render_lines(
        preview_state,
        key.render_markdown,
        key.width,
        key.height,
        focused_link,
        key.line_numbers,
    );
    let widest = if !key.wrap
        && key.line_numbers
        && matches!(preview_state.file_type, DetectedFileType::Source(_))
    {
        widest_without_source_gutter(&lines)
    } else {
        lines.iter().map(Line::width).max().unwrap_or_default()
    };
    let prewrapped = key.wrap
        && key.render_markdown
        && matches!(
            preview_state.content,
            crate::preview::PreviewContent::Markdown { .. }
        );
    let wrapped = (key.wrap && !prewrapped).then(|| wrapped_layout(&lines, key.width));
    cache.entry = Some(CachedPreview {
        key,
        lines,
        widest,
        wrapped,
        prewrapped,
    });
    #[cfg(test)]
    {
        cache.builds += 1;
    }
}

#[cfg(test)]
fn measure_lines(lines: &[Line<'static>], width: u16, wrap: bool) -> (usize, usize) {
    let widest = lines.iter().map(Line::width).max().unwrap_or_default();
    let line_count = if wrap {
        wrapped_layout(lines, width)
            .row_prefix
            .last()
            .copied()
            .unwrap_or_default()
    } else {
        lines.len()
    };
    (line_count, widest)
}

fn preview_progress(preview: &PreviewState) -> (usize, usize, usize, usize) {
    let total = preview.rendered_line_count;
    if total == 0 {
        return (0, 0, 0, 0);
    }
    if preview.viewport_height == 0 {
        return (0, 0, total, 0);
    }
    let start = preview.scroll.saturating_add(1).min(total);
    let end = preview
        .scroll
        .saturating_add(preview.viewport_height)
        .min(total);
    let percent = end.saturating_mul(100) / total;
    (start, end, total, percent)
}

fn source_gutter_width(line_count: usize) -> u16 {
    (line_count.max(1).to_string().len() + 3).min(u16::MAX as usize) as u16
}

fn widest_without_source_gutter(lines: &[Line<'static>]) -> usize {
    lines
        .iter()
        .map(|line| line.spans.iter().skip(1).map(Span::width).sum())
        .max()
        .unwrap_or_default()
}

fn split_source_gutter(mut line: Line<'static>) -> (Line<'static>, Line<'static>) {
    let gutter = if line.spans.is_empty() {
        Span::raw("")
    } else {
        line.spans.remove(0)
    };
    let mut content = Line::from(line.spans);
    content.style = line.style;
    content.alignment = line.alignment;
    (Line::from(gutter), content)
}

fn crop_lines(lines: Vec<Line<'static>>, offset: usize, width: usize) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| crop_line(&line, offset, width))
        .collect()
}

fn crop_line(line: &Line<'static>, offset: usize, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut current_style = None;
    let mut current_text = String::new();
    let mut position = 0usize;
    let right = offset.saturating_add(width);

    for grapheme in line.styled_graphemes(Style::default()) {
        let grapheme_width = UnicodeWidthStr::width(grapheme.symbol);
        let end = position.saturating_add(grapheme_width);
        if end <= offset {
            position = end;
            continue;
        }
        if position >= right {
            break;
        }
        if position < offset || end > right {
            let overlap_start = position.max(offset);
            let overlap_end = end.min(right);
            let padding = overlap_end.saturating_sub(overlap_start);
            append_styled_text(
                &mut spans,
                &mut current_style,
                &mut current_text,
                " ".repeat(padding).as_str(),
                grapheme.style,
            );
            position = end;
            if end >= right {
                break;
            }
            continue;
        }
        append_styled_text(
            &mut spans,
            &mut current_style,
            &mut current_text,
            grapheme.symbol,
            grapheme.style,
        );
        position = end;
    }
    if let Some(style) = current_style {
        spans.push(Span::styled(current_text, style));
    }
    Line::from(spans)
}

fn append_styled_text(
    spans: &mut Vec<Span<'static>>,
    current_style: &mut Option<Style>,
    current_text: &mut String,
    text: &str,
    style: Style,
) {
    if *current_style != Some(style) {
        if let Some(previous) = current_style.take() {
            spans.push(Span::styled(std::mem::take(current_text), previous));
        }
        *current_style = Some(style);
    }
    current_text.push_str(text);
}

/// Return an arbitrary usize-sized vertical window using the same word wrapping state machine as
/// Ratatui's `WordWrapper` with `trim: false`.
fn wrapped_window(
    lines: &[Line<'static>],
    layout: &WrappedLayout,
    width: u16,
    start: usize,
    height: usize,
) -> Vec<Line<'static>> {
    wrapped_window_with_work(lines, layout, width, start, height).0
}

#[derive(Debug, Default, PartialEq, Eq)]
struct WrapWindowWork {
    logical_lines: usize,
    source_units: usize,
}

fn wrapped_window_with_work(
    lines: &[Line<'static>],
    layout: &WrappedLayout,
    width: u16,
    start: usize,
    height: usize,
) -> (Vec<Line<'static>>, WrapWindowWork) {
    if width == 0 || height == 0 {
        return (Vec::new(), WrapWindowWork::default());
    }

    let Some(total) = layout.row_prefix.last().copied() else {
        return (Vec::new(), WrapWindowWork::default());
    };
    if start >= total
        || layout.row_prefix.len() != lines.len().saturating_add(1)
        || layout.line_indexes.len() != lines.len()
    {
        return (Vec::new(), WrapWindowWork::default());
    }

    let first_line = layout
        .row_prefix
        .partition_point(|row| *row <= start)
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1));
    let mut visible = Vec::with_capacity(height.min(1024));
    let mut work = WrapWindowWork::default();
    for (line_index, line) in lines.iter().enumerate().skip(first_line) {
        work.logical_lines = work.logical_lines.saturating_add(1);
        let local_start = start.saturating_sub(layout.row_prefix[line_index]);
        let remaining = height.saturating_sub(visible.len());
        let (window, source_units) = wrapped_line_window(
            line,
            &layout.line_indexes[line_index],
            width,
            local_start,
            remaining,
        );
        work.source_units = work.source_units.saturating_add(source_units);
        visible.extend(window);
        if visible.len() >= height {
            break;
        }
    }
    (visible, work)
}

fn wrapped_layout(lines: &[Line<'static>], width: u16) -> WrappedLayout {
    let mut prefix = Vec::with_capacity(lines.len().saturating_add(1));
    let mut line_indexes = Vec::with_capacity(lines.len());
    prefix.push(0usize);
    for line in lines {
        let index = build_line_wrap_index(line, width);
        let next = prefix
            .last()
            .copied()
            .unwrap_or_default()
            .saturating_add(index.rows);
        prefix.push(next);
        line_indexes.push(index);
    }
    WrappedLayout {
        row_prefix: prefix,
        line_indexes,
    }
}

fn build_line_wrap_index(line: &Line<'static>, width: u16) -> LineWrapIndex {
    if width == 0 {
        return LineWrapIndex {
            rows: 0,
            checkpoints: Vec::new(),
        };
    }
    let mut state = WrapState::default();
    let mut checkpoints = vec![WrapCheckpoint {
        row: 0,
        cursor: SourceCursor::default(),
        state: state.clone(),
    }];
    let mut next_checkpoint = WRAP_CHECKPOINT_ROWS;
    visit_source(line, SourceCursor::default(), |grapheme, next_cursor| {
        state.push(grapheme, width as usize);
        if state.rows >= next_checkpoint {
            checkpoints.push(WrapCheckpoint {
                row: state.rows,
                cursor: next_cursor,
                state: state.clone(),
            });
            next_checkpoint = state.rows.saturating_add(WRAP_CHECKPOINT_ROWS);
        }
        true
    });
    state.finish(|_| true);
    LineWrapIndex {
        rows: state.rows,
        checkpoints,
    }
}

fn wrapped_line_window(
    line: &Line<'static>,
    index: &LineWrapIndex,
    width: u16,
    start: usize,
    height: usize,
) -> (Vec<Line<'static>>, usize) {
    if width == 0 || height == 0 || start >= index.rows {
        return (Vec::new(), 0);
    }
    let checkpoint = &index.checkpoints[index
        .checkpoints
        .partition_point(|checkpoint| checkpoint.row <= start)
        .saturating_sub(1)
        .min(index.checkpoints.len().saturating_sub(1))];
    let mut state = checkpoint.state.clone();
    let mut rows = Vec::with_capacity(height.min(1024));
    let mut source_units = 0usize;
    let completed = visit_source(line, checkpoint.cursor, |grapheme, _| {
        source_units = source_units.saturating_add(1);
        let emitted = state.push(grapheme, width as usize);
        if let Some(emitted) = emitted {
            let row = state.rows.saturating_sub(1);
            if row >= start {
                rows.push(line_from_owned(emitted, line.alignment));
            }
        }
        rows.len() < height
    });
    if completed && rows.len() < height {
        let mut final_row = state.rows;
        state.finish(|emitted| {
            if final_row >= start {
                rows.push(line_from_owned(emitted, line.alignment));
            }
            final_row = final_row.saturating_add(1);
            rows.len() < height
        });
    }
    (rows, source_units)
}

impl WrapState {
    fn push(&mut self, grapheme: OwnedGrapheme, max_width: usize) -> Option<Vec<OwnedGrapheme>> {
        let symbol_width = UnicodeWidthStr::width(grapheme.symbol.as_str());
        if symbol_width > max_width {
            return None;
        }
        let is_whitespace = is_wrap_whitespace(&grapheme.symbol);
        let word_found = self.non_whitespace_previous && is_whitespace;
        let untrimmed_overflow = self.pending_line.is_empty()
            && self
                .word_width
                .saturating_add(self.whitespace_width)
                .saturating_add(symbol_width)
                > max_width;

        if word_found || untrimmed_overflow {
            self.pending_line.extend(self.pending_whitespace.drain(..));
            self.line_width = self.line_width.saturating_add(self.whitespace_width);
            self.pending_line.append(&mut self.pending_word);
            self.line_width = self.line_width.saturating_add(self.word_width);
            self.whitespace_width = 0;
            self.word_width = 0;
        }

        let line_full = self.line_width >= max_width;
        let pending_word_overflow = symbol_width > 0
            && self
                .line_width
                .saturating_add(self.whitespace_width)
                .saturating_add(self.word_width)
                >= max_width;
        let mut emitted = None;
        if line_full || pending_word_overflow {
            let mut remaining_width = max_width.saturating_sub(self.line_width);
            emitted = Some(std::mem::take(&mut self.pending_line));
            self.rows = self.rows.saturating_add(1);
            self.line_width = 0;

            while let Some(front) = self.pending_whitespace.front() {
                let width = UnicodeWidthStr::width(front.symbol.as_str());
                if width > remaining_width {
                    break;
                }
                self.whitespace_width = self.whitespace_width.saturating_sub(width);
                remaining_width = remaining_width.saturating_sub(width);
                self.pending_whitespace.pop_front();
            }
            if is_whitespace && self.pending_whitespace.is_empty() {
                return emitted;
            }
        }

        if is_whitespace {
            self.whitespace_width = self.whitespace_width.saturating_add(symbol_width);
            self.pending_whitespace.push_back(grapheme);
        } else {
            self.word_width = self.word_width.saturating_add(symbol_width);
            self.pending_word.push(grapheme);
        }
        self.non_whitespace_previous = !is_whitespace;
        emitted
    }

    fn finish(&mut self, mut emit: impl FnMut(Vec<OwnedGrapheme>) -> bool) {
        let mut emitted_final = false;
        if self.pending_line.is_empty()
            && self.pending_word.is_empty()
            && !self.pending_whitespace.is_empty()
        {
            self.rows = self.rows.saturating_add(1);
            emitted_final = true;
            if !emit(Vec::new()) {
                return;
            }
        }
        self.pending_line.extend(self.pending_whitespace.drain(..));
        self.pending_line.append(&mut self.pending_word);
        if !self.pending_line.is_empty() {
            self.rows = self.rows.saturating_add(1);
            emit(std::mem::take(&mut self.pending_line));
        } else if !emitted_final && self.rows == 0 {
            self.rows = 1;
            emit(Vec::new());
        }
    }
}

fn is_wrap_whitespace(symbol: &str) -> bool {
    symbol == "\u{200b}" || symbol != "\u{00a0}" && symbol.chars().all(char::is_whitespace)
}

/// Visit styled graphemes from an O(1)-seekable span/byte cursor. The cursor always points to a
/// Unicode grapheme boundary generated by this function, so slicing is safe.
fn visit_source(
    line: &Line<'static>,
    cursor: SourceCursor,
    mut visit: impl FnMut(OwnedGrapheme, SourceCursor) -> bool,
) -> bool {
    for span_index in cursor.span..line.spans.len() {
        let span = &line.spans[span_index];
        let start = if span_index == cursor.span {
            cursor.byte.min(span.content.len())
        } else {
            0
        };
        let content = &span.content[start..];
        let style = line.style.patch(span.style);
        for (relative, symbol) in content.grapheme_indices(true) {
            if symbol == "\n" {
                continue;
            }
            let byte = start + relative;
            let after = byte + symbol.len();
            let next_cursor = if after == span.content.len() {
                SourceCursor {
                    span: span_index + 1,
                    byte: 0,
                }
            } else {
                SourceCursor {
                    span: span_index,
                    byte: after,
                }
            };
            if !visit(
                OwnedGrapheme {
                    symbol: symbol.to_string(),
                    style,
                },
                next_cursor,
            ) {
                return false;
            }
        }
    }
    true
}

fn line_from_owned(graphemes: Vec<OwnedGrapheme>, alignment: Option<Alignment>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for grapheme in graphemes {
        if let Some(last) = spans.last_mut().filter(|span| span.style == grapheme.style) {
            last.content.to_mut().push_str(&grapheme.symbol);
        } else {
            spans.push(Span::styled(grapheme.symbol, grapheme.style));
        }
    }
    let mut line = Line::from(spans);
    line.alignment = alignment;
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_wrap_measurement_reaches_the_true_final_rendered_line() {
        let lines = vec![Line::from("abcd efgh ij")];
        let (rendered, width) = measure_lines(&lines, 6, true);
        assert_eq!(
            rendered, 3,
            "word boundaries require more rows than width/6"
        );
        assert_eq!(width, 12);

        let mut preview = PreviewState::default();
        preview.set_measurements(6, 1, rendered, 12);
        preview.scroll_bottom();

        assert_eq!(preview.scroll, 2);
        assert_eq!(preview_progress(&preview), (3, 3, 3, 100));
    }

    #[test]
    fn visible_range_reports_the_whole_viewport() {
        let mut preview = PreviewState::default();
        preview.set_measurements(80, 20, 100, 80);
        preview.scroll = 80;
        assert_eq!(preview_progress(&preview), (81, 100, 100, 100));
    }

    #[test]
    fn zero_height_viewport_reports_no_visible_range() {
        let mut preview = PreviewState::default();
        preview.set_measurements(80, 0, 100, 80);

        assert_eq!(preview_progress(&preview), (0, 0, 100, 0));
    }

    #[test]
    fn wrapped_previews_beyond_u16_rows_reach_the_tail_and_full_progress() {
        let lines = vec![Line::from("x ".repeat(70_000))];
        let (rendered, _) = measure_lines(&lines, 1, true);
        let layout = wrapped_layout(&lines, 1);
        assert!(rendered > u16::MAX as usize);

        let mut preview = PreviewState::default();
        preview.set_measurements(1, 1, rendered, 2);
        preview.scroll_bottom();
        assert!(preview.scroll > u16::MAX as usize);
        assert_eq!(preview_progress(&preview).3, 100);

        let tail = wrapped_window(&lines, &layout, 1, preview.scroll, 1);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].width(), 1);
    }

    #[test]
    fn usize_word_wrap_window_matches_ratatui_measurement() {
        let lines = vec![
            Line::from("abcd efgh ij"),
            Line::from("  whitespace and 界 text"),
            Line::from(""),
        ];
        let rendered = Paragraph::new(lines.clone())
            .wrap(Wrap { trim: false })
            .line_count(6);
        let layout = wrapped_layout(&lines, 6);
        assert_eq!(layout.row_prefix.last().copied().unwrap(), rendered);
        assert_eq!(
            wrapped_window(&lines, &layout, 6, 0, usize::MAX).len(),
            rendered
        );
    }

    #[test]
    fn cached_wrap_counts_match_ratatui_for_edge_cases() {
        let fixtures = vec![
            Line::from(""),
            Line::from("trailing "),
            Line::from("  leading whitespace"),
            Line::from("word-that-is-longer-than-the-width"),
            Line::from("zero\u{200b}width and non\u{00a0}breaking"),
            Line::from("wide 界 characters"),
            Line::from(vec![
                Span::styled("styled ", Style::default().fg(Color::Red)),
                Span::styled("segments", Style::default().fg(Color::Blue)),
            ]),
        ];
        for width in 1..=12 {
            for line in &fixtures {
                let expected = Paragraph::new(line.clone())
                    .wrap(Wrap { trim: false })
                    .line_count(width);
                assert_eq!(
                    build_line_wrap_index(line, width).rows,
                    expected,
                    "width={width}, line={line:?}"
                );
            }
        }
    }

    #[test]
    fn sparse_window_rows_and_styles_match_ratatui_across_checkpoints() {
        use ratatui::buffer::Buffer;

        let lines = vec![Line::from(vec![
            Span::styled(
                "alpha  e\u{301} 界 beta ".repeat(80),
                Style::default().fg(Color::Red),
            ),
            Span::styled("tail words ".repeat(80), Style::default().fg(Color::Blue)),
        ])];
        for width in [1, 2, 7, 80] {
            let layout = wrapped_layout(&lines, width);
            let total = layout.row_prefix[1];
            for start in [0, 63, 64, total.saturating_sub(10)] {
                if start >= total || start > u16::MAX as usize {
                    continue;
                }
                let height = (total - start).min(10) as u16;
                let area = Rect::new(0, 0, width, height);
                let mut expected = Buffer::empty(area);
                Paragraph::new(lines.clone())
                    .wrap(Wrap { trim: false })
                    .scroll((start as u16, 0))
                    .render(area, &mut expected);

                let visible = wrapped_window(&lines, &layout, width, start, height as usize);
                let mut actual = Buffer::empty(area);
                Paragraph::new(visible).render(area, &mut actual);

                assert_eq!(actual, expected, "width={width}, start={start}");
            }
        }
    }

    #[test]
    fn deep_wrapped_windows_binary_search_the_cached_logical_line_prefix() {
        let lines = (0..100_000)
            .map(|index| Line::from(format!("line {index}")))
            .collect::<Vec<_>>();
        let layout = wrapped_layout(&lines, 80);

        let (visible, work) = wrapped_window_with_work(&lines, &layout, 80, 99_990, 5);

        assert_eq!(visible.len(), 5);
        assert_eq!(work.logical_lines, 5);
    }

    #[test]
    fn two_megabyte_width_one_line_has_sparse_index_and_bounded_deep_work() {
        let byte_count = crate::preview::MAX_PREVIEW_SIZE as usize;
        let lines = vec![Line::from("x".repeat(byte_count))];
        let layout = wrapped_layout(&lines, 1);

        assert_eq!(layout.row_prefix, [0, byte_count]);
        assert!(
            layout.line_indexes[0].checkpoints.len()
                <= byte_count.div_ceil(WRAP_CHECKPOINT_ROWS) + 1
        );

        let (visible, work) = wrapped_window_with_work(&lines, &layout, 1, byte_count - 24, 24);
        assert_eq!(visible.len(), 24);
        assert!(visible.iter().all(|line| line.spans[0].content == "x"));
        assert_eq!(work.logical_lines, 1);
        assert!(work.source_units <= WRAP_CHECKPOINT_ROWS + 24);
    }

    #[test]
    fn sparse_window_preserves_styles_across_checkpoints() {
        let lines = vec![Line::from(vec![
            Span::styled("a".repeat(80), Style::default().fg(Color::Red)),
            Span::styled("b".repeat(80), Style::default().fg(Color::Blue)),
        ])];
        let layout = wrapped_layout(&lines, 1);
        let visible = wrapped_window(&lines, &layout, 1, 76, 12);

        assert_eq!(visible.len(), 12);
        assert!(visible[..4]
            .iter()
            .all(|line| line.spans[0].style.fg == Some(Color::Red)));
        assert!(visible[4..]
            .iter()
            .all(|line| line.spans[0].style.fg == Some(Color::Blue)));
    }

    #[test]
    fn sparse_checkpoints_bound_deep_whitespace_work_at_common_widths() {
        let byte_count = crate::preview::MAX_PREVIEW_SIZE as usize;
        let line = Line::from("x ".repeat(byte_count / 2));
        for width in [1, 2, 80] {
            let lines = vec![line.clone()];
            let layout = wrapped_layout(&lines, width);
            let total = *layout.row_prefix.last().unwrap();
            let start = total.saturating_sub(20);
            let (visible, work) = wrapped_window_with_work(&lines, &layout, width, start, 20);

            assert_eq!(visible.len(), total.min(20));
            assert!(work.source_units <= (WRAP_CHECKPOINT_ROWS + 20) * width as usize * 3);
            assert!(layout.line_indexes[0].checkpoints.len() <= total / WRAP_CHECKPOINT_ROWS + 2);
        }
    }

    #[test]
    fn sparse_unicode_tail_matches_ratatui_and_scans_bounded_work() {
        let line = Line::from("e\u{301} 界 ".repeat(20_000));
        let lines = vec![line.clone()];
        let width = 2;
        let layout = wrapped_layout(&lines, width);
        let expected = Paragraph::new(line)
            .wrap(Wrap { trim: false })
            .line_count(width);
        assert_eq!(layout.row_prefix[1], expected);

        let start = expected.saturating_sub(16);
        let (visible, work) = wrapped_window_with_work(&lines, &layout, width, start, 16);
        assert_eq!(visible.len(), expected.min(16));
        assert!(work.source_units <= (WRAP_CHECKPOINT_ROWS + 16) * 5);
    }

    #[test]
    fn unchanged_preview_reuses_render_and_measurement_cache() {
        let mut preview = PreviewState::default();
        preview.file_type = DetectedFileType::Source(crate::filesystem::file_type::Language::Rust);
        preview.content = crate::preview::PreviewContent::Text {
            content: "fn main() {}\n".repeat(100),
            language: crate::filesystem::file_type::Language::Rust,
        };
        let key = PreviewRenderKey {
            generation: preview.generation(),
            width: 80,
            height: 20,
            render_markdown: true,
            focused_link: None,
            line_numbers: true,
            wrap: true,
        };
        let mut cache = PreviewRenderCache::default();

        ensure_render_cache(&mut cache, key.clone(), &preview, None);
        preview.scroll = 42;
        ensure_render_cache(&mut cache, key.clone(), &preview, None);

        assert_eq!(cache.builds, 1);
        assert_eq!(
            cache
                .entry
                .as_ref()
                .unwrap()
                .wrapped
                .as_ref()
                .unwrap()
                .row_prefix
                .len(),
            102
        );

        let mut width_changed = key.clone();
        width_changed.width = 79;
        ensure_render_cache(&mut cache, width_changed, &preview, None);
        let mut mode_changed = key.clone();
        mode_changed.wrap = false;
        ensure_render_cache(&mut cache, mode_changed, &preview, None);
        let mut preference_changed = key.clone();
        preference_changed.line_numbers = false;
        ensure_render_cache(&mut cache, preference_changed, &preview, None);
        let mut focus_changed = key.clone();
        focus_changed.focused_link = Some(0);
        ensure_render_cache(&mut cache, focus_changed, &preview, Some(0));

        let replacement = PreviewState::default();
        let mut selection_changed = key;
        selection_changed.generation = replacement.generation();
        ensure_render_cache(&mut cache, selection_changed, &replacement, None);
        assert_eq!(cache.builds, 6);
    }

    #[test]
    fn rendered_markdown_skips_redundant_wrapped_layout_allocation() {
        let mut preview = PreviewState::default();
        preview.file_type = DetectedFileType::Markdown;
        preview.content = crate::preview::PreviewContent::Markdown {
            content: "# Heading\n\nA wrapped paragraph".to_string(),
        };
        let key = PreviewRenderKey {
            generation: preview.generation(),
            width: 12,
            height: 10,
            render_markdown: true,
            focused_link: None,
            line_numbers: true,
            wrap: true,
        };
        let mut cache = PreviewRenderCache::default();

        ensure_render_cache(&mut cache, key, &preview, None);
        let cached = cache.entry.as_ref().unwrap();

        assert!(cached.prewrapped);
        assert!(cached.wrapped.is_none());
    }

    #[test]
    fn arbitrary_large_unicode_horizontal_offset_is_sliced_without_u16_loss() {
        let line = Line::from(format!("{}界tail", "x".repeat(70_000)));
        let cropped = crop_line(&line, 70_000, 6);
        assert_eq!(
            cropped
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "界tail"
        );
        assert_eq!(cropped.width(), 6);
    }

    #[test]
    fn horizontal_crop_pads_and_stops_at_right_edge_wide_grapheme() {
        let cropped = crop_line(&Line::from("a界b"), 0, 2);
        let text = cropped
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text, "a ");
        assert_eq!(cropped.width(), 2);
    }

    #[test]
    fn horizontal_crop_inside_wide_grapheme_preserves_absolute_columns() {
        let cropped = crop_line(&Line::from("界ab"), 1, 2);
        let text = cropped
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text, " a");
        assert_eq!(cropped.width(), 2);
    }

    #[test]
    fn source_gutter_is_separate_from_horizontally_scrolled_content() {
        let line = Line::from(vec![
            Span::styled("1 │ ", Style::default().fg(Color::DarkGray)),
            Span::raw("0123456789"),
        ]);
        let (gutter, content) = split_source_gutter(line);
        let cropped = crop_line(&content, 6, 4);
        assert_eq!(gutter.spans[0].content, "1 │ ");
        assert_eq!(cropped.spans[0].content, "6789");
    }
}
