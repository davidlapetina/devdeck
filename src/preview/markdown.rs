use pulldown_cmark::{Event, Options, Parser, Tag};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, Copy)]
struct TextMode {
    bold: bool,
    italic: bool,
}

#[derive(Debug)]
struct ListFrame {
    ordered: bool,
    next: u64,
}

#[derive(Debug, Default)]
struct TableState {
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownLink {
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct MarkdownRender {
    pub lines: Vec<Line<'static>>,
    pub focused_link_line: Option<usize>,
}

#[derive(Debug, Clone)]
struct PendingSpan {
    text: String,
    style: Style,
    link_index: Option<usize>,
}

#[derive(Debug, Clone)]
struct LinkContext {
    target: String,
    index: usize,
}

#[derive(Debug, Clone)]
struct MarkdownAnchor {
    slug: String,
    line: usize,
}

pub fn render_markdown(content: &str, width: usize) -> Vec<Line<'static>> {
    render_markdown_with_focus(content, width, None).lines
}

pub fn render_markdown_with_focus(
    content: &str,
    width: usize,
    focused_link: Option<usize>,
) -> MarkdownRender {
    let renderer = render_markdown_full(content, width.max(1), focused_link);
    MarkdownRender {
        lines: renderer.lines,
        focused_link_line: renderer.focused_link_line,
    }
}

fn render_markdown_full(content: &str, width: usize, focused_link: Option<usize>) -> Renderer {
    let width = width.max(1);
    let mut renderer = Renderer {
        lines: Vec::new(),
        current: Vec::new(),
        mode: TextMode {
            bold: false,
            italic: false,
        },
        heading_level: None,
        quote_depth: 0,
        list_stack: Vec::new(),
        in_code_block: false,
        code_buffer: String::new(),
        link_target: None,
        link_count: 0,
        focused_link,
        focused_link_line: None,
        heading_text: None,
        anchors: Vec::new(),
        table: None,
        width,
    };

    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;

    for event in Parser::new_ext(content, options) {
        renderer.handle_event(event);
    }
    renderer.flush_current();

    if renderer.lines.is_empty() {
        renderer.lines.push(Line::from(""));
    }

    renderer
}

pub fn collect_links(content: &str) -> Vec<MarkdownLink> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;

    let mut in_table = false;
    let mut links = Vec::new();
    for event in Parser::new_ext(content, options) {
        match event {
            Event::Start(Tag::Table(_)) => in_table = true,
            Event::End(Tag::Table(_)) => in_table = false,
            Event::Start(Tag::Link(_, target, _)) if !in_table => links.push(MarkdownLink {
                target: target.to_string(),
            }),
            _ => {}
        };
    }
    links
}

pub fn focused_link_line(content: &str, width: usize, focused_link: usize) -> Option<usize> {
    render_markdown_with_focus(content, width, Some(focused_link)).focused_link_line
}

pub fn anchor_line(content: &str, width: usize, anchor: &str) -> Option<usize> {
    let requested = normalize_anchor(anchor);
    if requested.is_empty() {
        return None;
    }
    let renderer = render_markdown_full(content, width.max(1), None);
    renderer
        .anchors
        .into_iter()
        .find(|heading| heading.slug == requested)
        .map(|heading| heading.line)
}

struct Renderer {
    lines: Vec<Line<'static>>,
    current: Vec<PendingSpan>,
    mode: TextMode,
    heading_level: Option<u8>,
    quote_depth: usize,
    list_stack: Vec<ListFrame>,
    in_code_block: bool,
    code_buffer: String,
    link_target: Option<LinkContext>,
    link_count: usize,
    focused_link: Option<usize>,
    focused_link_line: Option<usize>,
    heading_text: Option<String>,
    anchors: Vec<MarkdownAnchor>,
    table: Option<TableState>,
    width: usize,
}

impl Renderer {
    fn handle_event(&mut self, event: Event<'_>) {
        if self.in_code_block {
            match event {
                Event::End(Tag::CodeBlock(_)) => self.end_code_block(),
                Event::Text(text) | Event::Code(text) => self.code_buffer.push_str(&text),
                Event::SoftBreak | Event::HardBreak => self.code_buffer.push('\n'),
                _ => {}
            }
            return;
        }

        if self.table.is_some() {
            self.handle_table_event(event);
            return;
        }

        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.push_text(&text),
            Event::Code(code) => self.push_span(
                code.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Event::Html(html) | Event::FootnoteReference(html) => self.push_text(&html),
            Event::SoftBreak | Event::HardBreak => self.flush_current(),
            Event::Rule => {
                self.flush_current();
                self.lines.push(Line::from(Span::styled(
                    "-".repeat(self.width.min(80)),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            Event::TaskListMarker(checked) => {
                self.push_span(
                    if checked { "[x] " } else { "[ ] " }.to_string(),
                    Style::default().fg(Color::Green),
                );
            }
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                if self.quote_depth > 0 && self.current.is_empty() && self.list_stack.is_empty() {
                    self.push_quote_prefix();
                }
            }
            Tag::Heading(level, _, _) => {
                self.flush_current();
                self.heading_level = Some(level as u8);
                self.heading_text = Some(String::new());
            }
            Tag::BlockQuote => {
                self.flush_current();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(_) => {
                self.flush_current();
                self.in_code_block = true;
                self.code_buffer.clear();
            }
            Tag::List(start) => {
                self.flush_current();
                self.list_stack.push(ListFrame {
                    ordered: start.is_some(),
                    next: start.unwrap_or(1),
                });
            }
            Tag::Item => {
                self.flush_current();
                self.push_line_prefix();
            }
            Tag::Emphasis => self.mode.italic = true,
            Tag::Strong => self.mode.bold = true,
            Tag::Link(_, target, _) => {
                let index = self.link_count;
                self.link_count += 1;
                self.link_target = Some(LinkContext {
                    target: target.to_string(),
                    index,
                });
            }
            Tag::Table(_) => {
                self.flush_current();
                self.table = Some(TableState::default());
            }
            Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::Strikethrough
            | Tag::Image(_, _, _) => {}
            Tag::FootnoteDefinition(_) => {}
        }
    }

    fn end_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.flush_current();
            }
            Tag::Heading(_, _, _) => {
                let line = self.lines.len();
                if let Some(text) = self.heading_text.take() {
                    let slug = slugify_heading(&text);
                    if !slug.is_empty() {
                        self.anchors.push(MarkdownAnchor { slug, line });
                    }
                }
                self.flush_current();
                self.heading_level = None;
            }
            Tag::BlockQuote => {
                self.flush_current();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            Tag::List(_) => {
                self.flush_current();
                self.list_stack.pop();
            }
            Tag::Item => self.flush_current(),
            Tag::Emphasis => self.mode.italic = false,
            Tag::Strong => self.mode.bold = false,
            Tag::Link(_, _, _) => {
                if let Some(link) = self.link_target.take() {
                    self.push_span(format!(" ({})", link.target), self.link_style(link.index));
                }
            }
            Tag::CodeBlock(_) => self.end_code_block(),
            Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::Strikethrough
            | Tag::Image(_, _, _) => {}
            Tag::FootnoteDefinition(_) => {}
        }
    }

    fn handle_table_event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(Tag::TableCell) => {
                if let Some(table) = &mut self.table {
                    table.current_cell.clear();
                }
            }
            Event::End(Tag::TableCell) => {
                if let Some(table) = &mut self.table {
                    table
                        .current_row
                        .push(table.current_cell.trim().to_string());
                    table.current_cell.clear();
                }
            }
            Event::End(Tag::TableHead) | Event::End(Tag::TableRow) => {
                if let Some(table) = &mut self.table {
                    if !table.current_row.is_empty() {
                        table.rows.push(std::mem::take(&mut table.current_row));
                    }
                }
            }
            Event::End(Tag::Table(_)) => {
                if let Some(table) = self.table.take() {
                    self.render_table(table);
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(table) = &mut self.table {
                    table.current_cell.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(table) = &mut self.table {
                    table.current_cell.push(' ');
                }
            }
            _ => {}
        }
    }

    fn push_line_prefix(&mut self) {
        self.push_quote_prefix();

        if let Some(frame) = self.list_stack.last_mut() {
            if frame.ordered {
                let prefix = format!("{}. ", frame.next);
                frame.next += 1;
                self.push_span(prefix, Style::default().fg(Color::Cyan));
            } else {
                self.push_span("- ".to_string(), Style::default().fg(Color::Cyan));
            }
        }
    }

    fn push_quote_prefix(&mut self) {
        if self.quote_depth > 0 {
            self.push_span(
                format!("{} ", ">".repeat(self.quote_depth)),
                Style::default().fg(Color::DarkGray),
            );
        }
    }

    fn push_text(&mut self, text: &str) {
        if let Some(heading_text) = &mut self.heading_text {
            heading_text.push_str(text);
        }
        self.push_span(text.to_string(), self.current_style());
    }

    fn push_span(&mut self, text: String, style: Style) {
        if !text.is_empty() {
            self.current.push(PendingSpan {
                text,
                style,
                link_index: self.link_target.as_ref().map(|link| link.index),
            });
        }
    }

    fn current_style(&self) -> Style {
        let mut style = Style::default();
        if let Some(level) = self.heading_level {
            style = style.fg(match level {
                1 => Color::Cyan,
                2 => Color::Green,
                _ => Color::Magenta,
            });
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.mode.bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.mode.italic {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if let Some(link) = &self.link_target {
            style = self.link_style(link.index);
        }
        style
    }

    fn link_style(&self, link_index: usize) -> Style {
        let style = Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED);
        if self.focused_link == Some(link_index) {
            style.fg(Color::Black).bg(Color::Yellow)
        } else {
            style
        }
    }

    fn flush_current(&mut self) {
        if self.current.is_empty() {
            return;
        }

        let spans = std::mem::take(&mut self.current);
        let start_line = self.lines.len();
        let (lines, focused_line) = wrap_spans(spans, self.width, self.focused_link);
        if self.focused_link_line.is_none() {
            self.focused_link_line = focused_line.map(|line| start_line + line);
        }
        self.lines.extend(lines);
    }

    fn end_code_block(&mut self) {
        self.in_code_block = false;
        let code = std::mem::take(&mut self.code_buffer);
        let style = Style::default().fg(Color::LightBlue);
        if code.is_empty() {
            self.lines.push(Line::from(""));
            return;
        }

        for line in code.split('\n') {
            self.lines.push(Line::from(Span::styled(
                format!("  {}", line.trim_end_matches('\r')),
                style,
            )));
        }
    }

    fn render_table(&mut self, table: TableState) {
        if table.rows.is_empty() {
            return;
        }

        let column_count = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        if column_count == 0 {
            return;
        }

        let mut widths = vec![0; column_count];
        for row in &table.rows {
            for (index, cell) in row.iter().enumerate() {
                widths[index] =
                    widths[index].max(cell.chars().filter_map(UnicodeWidthChar::width).sum());
            }
        }

        for (row_index, row) in table.rows.iter().enumerate() {
            let mut line = String::new();
            for column in 0..column_count {
                let cell = row.get(column).map(String::as_str).unwrap_or("");
                let padding = widths[column]
                    .saturating_sub(cell.chars().filter_map(UnicodeWidthChar::width).sum());
                if column > 0 {
                    line.push_str(" | ");
                }
                line.push_str(cell);
                line.push_str(&" ".repeat(padding));
            }
            self.lines.push(Line::from(Span::styled(
                line,
                if row_index == 0 {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            )));

            if row_index == 0 && table.rows.len() > 1 {
                let rule = widths
                    .iter()
                    .map(|width| "-".repeat((*width).max(3)))
                    .collect::<Vec<_>>()
                    .join("-+-");
                self.lines.push(Line::from(Span::styled(
                    rule,
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
    }
}

fn wrap_spans(
    spans: Vec<PendingSpan>,
    width: usize,
    focused_link: Option<usize>,
) -> (Vec<Line<'static>>, Option<usize>) {
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut current_width = 0;
    let mut current_focused = false;
    let mut focused_line = None;

    let push_line = |lines: &mut Vec<Line<'static>>,
                     current: &mut Vec<Span<'static>>,
                     current_focused: &mut bool,
                     focused_line: &mut Option<usize>| {
        if *current_focused && focused_line.is_none() {
            *focused_line = Some(lines.len());
        }
        lines.push(Line::from(std::mem::take(current)));
        *current_focused = false;
    };

    for span in spans {
        let style = span.style;
        let link_index = span.link_index;
        for ch in span.text.chars() {
            if ch == '\n' {
                push_line(
                    &mut lines,
                    &mut current,
                    &mut current_focused,
                    &mut focused_line,
                );
                current_width = 0;
                continue;
            }

            let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width + char_width > width && !current.is_empty() {
                push_line(
                    &mut lines,
                    &mut current,
                    &mut current_focused,
                    &mut focused_line,
                );
                current_width = 0;
            }

            if focused_link.is_some() && link_index == focused_link {
                current_focused = true;
            }
            current.push(Span::styled(ch.to_string(), style));
            current_width += char_width;
        }
    }

    push_line(
        &mut lines,
        &mut current,
        &mut current_focused,
        &mut focused_line,
    );
    (lines, focused_line)
}

fn normalize_anchor(anchor: &str) -> String {
    anchor.trim_start_matches('#').trim().to_ascii_lowercase()
}

fn slugify_heading(text: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;

    for ch in text.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            previous_dash = false;
        } else if ch.is_whitespace() || ch == '-' {
            if !slug.is_empty() && !previous_dash {
                slug.push('-');
                previous_dash = true;
            }
        }
    }

    if previous_dash {
        slug.pop();
    }

    slug
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_common_markdown_constructs() {
        let lines = render_markdown(
            "# Title\n\n- [x] task\n\n> quote\n\n| A | B |\n| - | - |\n| 1 | 2 |\n",
            40,
        );
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("Title")));
        assert!(rendered.iter().any(|line| line.contains("[x] task")));
        assert!(rendered.iter().any(|line| line.contains("> quote")));
        assert!(rendered.iter().any(|line| line.contains("A | B")));
    }

    #[test]
    fn collects_and_focuses_links() {
        let content = "[Docs](docs/readme.md)\n\n[Web](https://example.com)";
        let links = collect_links(content);

        assert_eq!(
            links,
            vec![
                MarkdownLink {
                    target: "docs/readme.md".to_string(),
                },
                MarkdownLink {
                    target: "https://example.com".to_string(),
                },
            ]
        );

        let rendered = render_markdown_with_focus(content, 80, Some(1));
        assert!(rendered.focused_link_line.is_some());
        assert!(rendered
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .any(|span| span.style.bg == Some(Color::Yellow)));
    }

    #[test]
    fn finds_heading_anchor_lines() {
        assert_eq!(
            anchor_line("# Hello, World!\n\nText", 80, "hello-world"),
            Some(0)
        );
    }

    #[test]
    fn wraps_to_the_requested_width() {
        let lines = render_markdown("long long long long", 8);
        assert!(lines.len() > 1);
    }
}
