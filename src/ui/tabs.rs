use ratatui::{prelude::*, widgets::*};

use crate::{
    app::App,
    session::TerminalPromptState,
    tabs::{ActivityState, TabContent, TerminalTabState},
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
        let state = match &tab.content {
            TabContent::Repository => "",
            TabContent::Terminal(terminal)
                if matches!(
                    terminal.state,
                    TerminalTabState::Exited { .. } | TerminalTabState::Failed { .. }
                ) =>
            {
                "!"
            }
            TabContent::Terminal(terminal)
                if terminal
                    .session_id
                    .and_then(|session_id| app.sessions.session(session_id))
                    .is_some_and(|session| {
                        session.prompt_state == TerminalPromptState::AtPrompt
                    }) =>
            {
                ">"
            }
            TabContent::Terminal(_) => "",
        };
        let activity = if state.is_empty() {
            match tab.activity {
                ActivityState::None => "",
                ActivityState::OutputActive { .. } => "*",
                ActivityState::OutputQuiet => ".",
            }
        } else {
            ""
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
