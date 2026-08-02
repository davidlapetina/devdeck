use ratatui::{prelude::*, widgets::*};

use crate::{app::App, preview};

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let block = Block::default()
        .title(" Preview ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    let focused_link = app
        .markdown_rendered
        .then_some(app.preview_link_index)
        .flatten();
    let lines = preview::render_lines(
        &app.preview,
        app.markdown_rendered,
        inner.width,
        focused_link,
    );
    app.preview
        .set_measurements(inner.width as usize, inner.height as usize, lines.len());

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((app.preview.scroll as u16, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}
