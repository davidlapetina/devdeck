use ratatui::{prelude::*, widgets::*};
use unicode_width::UnicodeWidthStr;

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
    for (index, _) in app.tabs.iter().enumerate() {
        let label = tab_label(app, index);
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

pub fn hit_test(area: Rect, app: &App, column: u16, row: u16) -> Option<usize> {
    let inner = Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if row != inner.y || column < inner.x || column >= inner.x.saturating_add(inner.width) {
        return None;
    }

    let mut x = inner.x;
    for index in 0..app.tabs.len() {
        let width =
            UnicodeWidthStr::width(tab_label(app, index).as_str()).min(u16::MAX as usize) as u16;
        let end = x.saturating_add(width);
        if column >= x && column < end {
            return Some(index);
        }
        x = end;
    }
    None
}

fn tab_label(app: &App, index: usize) -> String {
    let tab = &app.tabs[index];
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
                .is_some_and(|session| session.prompt_state == TerminalPromptState::AtPrompt) =>
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
    format!("[{} {}{}{}] ", index + 1, tab.title, activity, state)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::TempDir;

    use super::*;
    use crate::{
        config::{ResolvedConfig, TerminalProfile, WorkspaceConfig},
        tabs::{Tab, TabId},
    };

    fn app(temp: &TempDir) -> App {
        App::new(
            temp.path().to_path_buf(),
            false,
            false,
            ResolvedConfig {
                workspace: WorkspaceConfig::default(),
                tabs: Vec::new(),
            },
        )
        .unwrap()
    }

    fn terminal_profile(name: &str) -> TerminalProfile {
        TerminalProfile {
            name: name.to_string(),
            command: "sh".to_string(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            auto_start: false,
            restart_on_exit: false,
        }
    }

    #[test]
    fn hit_test_returns_tab_under_rendered_label() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        app.tabs.push(Tab::terminal_tab(
            TabId(2),
            terminal_profile("Shell"),
            false,
        ));
        let area = Rect::new(0, 0, 80, 3);
        let first_tab_width = UnicodeWidthStr::width("[1 Files] ") as u16;

        assert_eq!(hit_test(area, &app, 1, 1), Some(0));
        assert_eq!(hit_test(area, &app, 1 + first_tab_width, 1), Some(1));
        assert_eq!(hit_test(area, &app, 1, 2), None);
    }
}
