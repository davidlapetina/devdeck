use ratatui::text::Line;

pub fn plain_lines(content: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<_> = content
        .split('\n')
        .map(|line| Line::from(line.trim_end_matches('\r').to_string()))
        .collect();

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    lines
}
