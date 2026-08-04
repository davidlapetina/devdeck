use ratatui::{prelude::*, widgets::*};

use crate::{
    app::{App, InputMode},
    preview::{format_modified, format_size},
    tabs::{TabContent, TerminalTabState},
};

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if let Some(message) = &app.status_message {
        let paragraph = Paragraph::new(message.clone())
            .style(Style::default().fg(Color::White).bg(Color::Black));
        frame.render_widget(paragraph, area);
        return;
    }

    if app.input_mode == InputMode::CommandPrefix {
        let paragraph = Paragraph::new(
            "COMMAND | 1..9 tab | n/p tab | c new | x stop | r restart | e reload | q quit | ? help",
        )
        .style(Style::default().fg(Color::Yellow).bg(Color::Black));
        frame.render_widget(paragraph, area);
        return;
    }

    match app.active_tab().map(|tab| &tab.content) {
        Some(TabContent::Repository) | None => render_files_status(frame, area, app),
        Some(TabContent::Terminal(_)) => render_terminal_status(frame, area, app),
    }
}

fn render_files_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let path = app
        .selected_path()
        .map(|path| app.relative_display(path))
        .unwrap_or_else(|| "-".to_string());
    let kind = app.preview.file_type.label();
    let size = app
        .preview
        .size
        .map(format_size)
        .unwrap_or_else(|| "-".to_string());
    let modified = format_modified(app.preview.modified);
    let watch = if app.watch_enabled {
        "watch"
    } else {
        "no-watch"
    };
    let markdown = if app.markdown_rendered {
        "md:render"
    } else {
        "md:raw"
    };

    let first_line =
        format!("Files | {path} | {kind} | {size} | {modified} | {watch} | {markdown}");
    let second_line =
        "j/k move  h/l expand  / search  a actions  m markdown  ]/[ links  v editor  e external  1..9/Tab tabs  c new  ? help  q quit"
            .to_string();

    let text = Text::from(vec![Line::from(first_line), Line::from(second_line)]);
    let paragraph = Paragraph::new(text).style(Style::default().fg(Color::White).bg(Color::Black));
    frame.render_widget(paragraph, area);
}

fn render_terminal_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(tab) = app.active_tab() else {
        return;
    };
    let Some(terminal) = tab.as_terminal() else {
        return;
    };

    let pid = terminal
        .session_id
        .and_then(|session_id| app.sessions.session(session_id))
        .and_then(|session| session.pid)
        .map(|pid| format!("pid {pid}"))
        .unwrap_or_else(|| "pid -".to_string());
    let dimensions = format!(
        "{}x{}",
        app.terminal_dimensions.cols, app.terminal_dimensions.rows
    );
    let state = match &terminal.state {
        TerminalTabState::NotStarted => "not started".to_string(),
        TerminalTabState::Starting => "starting".to_string(),
        TerminalTabState::Running => "running".to_string(),
        TerminalTabState::Exited { exit_code } => match exit_code {
            Some(code) => format!("exited {code}"),
            None => "exited".to_string(),
        },
        TerminalTabState::Failed { message } => format!("failed: {message}"),
    };
    let extra = if terminal.removed_from_config {
        " | removed from config"
    } else if terminal.requires_restart {
        " | restart required"
    } else {
        ""
    };
    let help = match terminal.state {
        TerminalTabState::Running => "Ctrl-b commands while running",
        TerminalTabState::NotStarted => "Enter start | 1..9/Tab tabs",
        TerminalTabState::Starting => "starting",
        TerminalTabState::Exited { .. } | TerminalTabState::Failed { .. } => {
            "Enter/r restart | x close/reset | 1..9/Tab tabs"
        }
    };
    let line = format!(
        "{} | {state} | {pid} | {dimensions} | {help}{extra}",
        tab.title
    );
    let paragraph = Paragraph::new(line).style(Style::default().fg(Color::White).bg(Color::Black));
    frame.render_widget(paragraph, area);
}
