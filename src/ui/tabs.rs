use ratatui::{prelude::*, widgets::*};

use crate::{
    app::App,
    tabs::{ActivityState, TabContent},
};

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let project = app
        .root_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("devdeck");
    let block = Block::default()
        .title(format!(" devdeck - {project} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut spans = Vec::new();
    for (index, tab) in app.tabs.iter().enumerate() {
        let activity = match tab.activity {
            ActivityState::None => "",
            ActivityState::UnreadOutput => "*",
        };
        let state = match &tab.content {
            TabContent::Repository => "",
            TabContent::Terminal(terminal)
                if matches!(
                    terminal.state,
                    crate::tabs::TerminalTabState::Exited { .. }
                        | crate::tabs::TerminalTabState::Failed { .. }
                ) =>
            {
                "!"
            }
            TabContent::Terminal(_) => "",
        };
        let label = format!("[{} {}{}{}] ", index + 1, tab.title, activity, state);
        let style = if index == app.active_tab {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        spans.push(Span::styled(label, style));
    }
    spans.push(Span::styled(
        "[+ c/Ctrl-b c]",
        Style::default().fg(Color::DarkGray),
    ));

    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}
