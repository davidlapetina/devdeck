use ratatui::{prelude::*, widgets::*};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
            "COMMAND | 1..9 tab | n/p tab | t switcher | l last | c new | x stop | r restart | e reload | m mouse policy | q quit | ? help",
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
    let (first_line, second_line) = files_status_lines(app, area.width as usize);
    let text = Text::from(vec![Line::from(first_line), Line::from(second_line)]);
    let paragraph = Paragraph::new(text).style(Style::default().fg(Color::White).bg(Color::Black));
    frame.render_widget(paragraph, area);
}

fn files_status_lines(app: &App, width: usize) -> (String, String) {
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
    let mouse = format!("mouse:{}", app.mouse_policy.label());
    let pane = app.files_pane.label();
    let view = if app.preview_wrap_effective() {
        "wrap"
    } else {
        "nowrap"
    };

    // Interaction state comes first and is never displaced by long paths or metadata.
    let required = vec![
        "Files".to_string(),
        format!("focus:{pane}"),
        mouse,
        view.to_string(),
    ];
    let optional = vec![
        path,
        kind.to_string(),
        size,
        modified,
        watch.to_string(),
        markdown.to_string(),
    ];
    let first_line = fit_segments(required, optional, width);
    let controls = if app.files_pane == crate::app::FilesPane::Preview {
        vec![
            "Preview",
            "Alt-t tabs",
            "Alt-l last",
            "j/k scroll",
            "Ctrl-d/u page",
            "g/G ends",
            "w wrap",
            "h/l horizontal",
            "N numbers",
            "z max",
            "Esc tree",
            "]/[ links",
            "? help",
        ]
    } else {
        vec![
            "Tree",
            "Alt-t tabs",
            "Alt-l last",
            "j/k move",
            "h/l expand",
            "f preview",
            "z max",
            "/ search",
            "a actions",
            "v editor",
            "e external",
            "? help",
        ]
    };
    let second_line = fit_segments(
        controls[..1]
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        controls[1..]
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        width,
    );
    (first_line, second_line)
}

fn fit_segments(required: Vec<String>, optional: Vec<String>, width: usize) -> String {
    let mut line = required.join(" | ");
    if UnicodeWidthStr::width(line.as_str()) > width {
        return truncate_display(&line, width);
    }
    for segment in optional {
        let candidate = format!("{line} | {segment}");
        if UnicodeWidthStr::width(candidate.as_str()) <= width {
            line = candidate;
        }
    }
    line
}

fn truncate_display(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let target = width.saturating_sub(1);
    let mut used = 0;
    let mut result = String::new();
    for ch in value.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or_default();
        if used + char_width > target {
            break;
        }
        result.push(ch);
        used += char_width;
    }
    result.push('…');
    result
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
    let mouse = format!("mouse:{}", app.mouse_policy.label());
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
        TerminalTabState::Running => "Alt-m mouse policy | Ctrl-b commands",
        TerminalTabState::NotStarted => "Enter start | Alt-t tabs | Alt-l last | 1..9/Tab tabs",
        TerminalTabState::Starting => "starting",
        TerminalTabState::Exited { .. } | TerminalTabState::Failed { .. } => {
            "Enter/r restart | x close/reset | Alt-t tabs | Alt-l last | 1..9/Tab tabs"
        }
    };
    let line = format!(
        "{} | {state} | {pid} | {dimensions} | {mouse} | {help}{extra}",
        tab.title
    );
    let paragraph = Paragraph::new(line).style(Style::default().fg(Color::White).bg(Color::Black));
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prioritized_status_fits_small_terminal_width() {
        let line = fit_segments(
            vec![
                "Files".to_string(),
                "focus:preview".to_string(),
                "mouse:auto".to_string(),
                "nowrap".to_string(),
            ],
            vec!["a/very/long/path/that/must/not/displace/state.rs".to_string()],
            40,
        );
        assert!(UnicodeWidthStr::width(line.as_str()) <= 40);
        assert!(line.contains("focus:preview"));
        assert!(line.contains("mouse:auto"));
    }

    #[test]
    fn preview_controls_are_added_in_priority_order() {
        let line = fit_segments(
            vec!["Preview".to_string()],
            [
                "j/k scroll",
                "w wrap",
                "h/l horizontal",
                "N numbers",
                "z max",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            50,
        );
        assert!(UnicodeWidthStr::width(line.as_str()) <= 50);
        assert!(line.contains("j/k scroll"));
        assert!(line.contains("w wrap"));
        assert!(line.contains("h/l horizontal"));
    }
}
