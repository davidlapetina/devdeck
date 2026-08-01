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

pub fn render_markdown(content: &str, width: usize) -> Vec<Line<'static>> {
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

    renderer.lines
}

struct Renderer {
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    mode: TextMode,
    heading_level: Option<u8>,
    quote_depth: usize,
    list_stack: Vec<ListFrame>,
    in_code_block: bool,
    code_buffer: String,
    link_target: Option<String>,
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
            Tag::Link(_, target, _) => self.link_target = Some(target.to_string()),
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
            Tag::Paragraph | Tag::Heading(_, _, _) => {
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
                if let Some(target) = self.link_target.take() {
                    self.push_span(
                        format!(" ({target})"),
                        Style::default()
                            .fg(Color::Blue)
                            .add_modifier(Modifier::UNDERLINED),
                    );
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
        self.push_span(text.to_string(), self.current_style());
    }

    fn push_span(&mut self, text: String, style: Style) {
        if !text.is_empty() {
            self.current.push(Span::styled(text, style));
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
        style
    }

    fn flush_current(&mut self) {
        if self.current.is_empty() {
            return;
        }

        let spans = std::mem::take(&mut self.current);
        self.lines.extend(wrap_spans(spans, self.width));
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

fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut current_width = 0;

    for span in spans {
        let style = span.style;
        let content = span.content.into_owned();
        for ch in content.chars() {
            if ch == '\n' {
                lines.push(Line::from(std::mem::take(&mut current)));
                current_width = 0;
                continue;
            }

            let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width + char_width > width && !current.is_empty() {
                lines.push(Line::from(std::mem::take(&mut current)));
                current_width = 0;
            }

            current.push(Span::styled(ch.to_string(), style));
            current_width += char_width;
        }
    }

    lines.push(Line::from(current));
    lines
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
    fn wraps_to_the_requested_width() {
        let lines = render_markdown("long long long long", 8);
        assert!(lines.len() > 1);
    }
}
