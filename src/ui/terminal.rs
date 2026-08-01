use ratatui::{prelude::*, widgets::*};
use tui_term::widget::PseudoTerminal;

use crate::{app::App, tabs::TerminalTabState};

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(tab) = app.active_tab() else {
        return;
    };
    let block = Block::default()
        .title(format!(" {} ", tab.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let Some(terminal_tab) = tab.as_terminal() else {
        frame.render_widget(block, area);
        return;
    };

    match &terminal_tab.state {
        TerminalTabState::Running => {
            if let Some(session_id) = terminal_tab.session_id {
                if let Some(session) = app.sessions.session(session_id) {
                    let pseudo_terminal = PseudoTerminal::new(session.terminal.screen())
                        .block(block)
                        .style(Style::default().fg(Color::White).bg(Color::Black));
                    frame.render_widget(pseudo_terminal, area);
                    return;
                }
            }
            render_message(frame, area, block, "Terminal session unavailable");
        }
        TerminalTabState::NotStarted => {
            render_message(
                frame,
                area,
                block,
                "Session not started\n\nPress Enter to start\n1..9 switch tabs | Tab next | ? help",
            );
        }
        TerminalTabState::Starting => {
            render_message(frame, area, block, "Starting...");
        }
        TerminalTabState::Exited { exit_code } => {
            let state = match exit_code {
                Some(code) => format!("Session exited {code}"),
                None => "Session exited".to_string(),
            };
            let close_help = if tab.temporary {
                "x close temporary tab"
            } else {
                "x reset to not started"
            };
            render_message(
                frame,
                area,
                block,
                &format!("{state}\n\nPress Enter or r to restart\n{close_help}\n1..9 switch tabs | Tab next | ? help"),
            );
        }
        TerminalTabState::Failed { message } => {
            render_message(
                frame,
                area,
                block,
                &format!(
                    "Unable to start session\n\n{message}\n\nPress Enter or r to retry\n1..9 switch tabs | Tab next | ? help"
                ),
            );
        }
    }
}

fn render_message(frame: &mut Frame<'_>, area: Rect, block: Block<'_>, message: &str) {
    let paragraph = Paragraph::new(message.to_string())
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}
