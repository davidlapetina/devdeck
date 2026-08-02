use ratatui::{prelude::*, widgets::*};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{App, InputMode, LauncherField},
    tabs::TabContent,
};

pub mod layout;
pub mod preview;
pub mod status;
pub mod tabs;
pub mod terminal;
pub mod tree;

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let areas = layout::areas(frame.area());
    tabs::render(frame, areas.tabs, app);

    match app.active_tab().map(|tab| &tab.content) {
        Some(TabContent::Repository) | None => {
            tree::render(frame, areas.files, app);
            preview::render(frame, areas.preview, app);
        }
        Some(TabContent::Terminal(_)) => {
            let dimensions = layout::terminal_dimensions(areas.content);
            app.resize_active_terminal(dimensions);
            terminal::render(frame, areas.content, app);
        }
    }

    status::render(frame, areas.status, app);

    if app.search.active {
        render_search(frame, app);
    }

    match app.input_mode {
        InputMode::TabLauncher => render_launcher(frame, app),
        InputMode::RenameTab => render_rename(frame, app),
        InputMode::ConfirmStop => {
            render_confirm(frame, "Stop terminal?", "Enter/y stop | Esc/n cancel")
        }
        InputMode::ConfirmRestart => {
            render_confirm(frame, "Restart terminal?", "Enter/y restart | Esc/n cancel")
        }
        InputMode::ConfirmQuit => {
            render_confirm(frame, "Quit DevDeck?", "Enter/y quit | Esc/n cancel")
        }
        InputMode::Help => render_help(frame),
        InputMode::PromptOverlay => render_prompt(frame, app),
        InputMode::Repository | InputMode::Terminal | InputMode::CommandPrefix => {}
    }
}

fn render_search(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Search ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let query_area = Rect::new(inner.x, inner.y, inner.width, inner.height.min(1));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("Search: "),
            Span::styled(app.search.query.clone(), Style::default().fg(Color::Yellow)),
        ])),
        query_area,
    );

    let results_area = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(1),
    );
    let height = results_area.height as usize;
    let selected = app.search.selected;
    let offset = selected.saturating_sub(height.saturating_sub(1));
    let items = app
        .search
        .results
        .iter()
        .skip(offset)
        .take(height)
        .enumerate()
        .map(|(visible_index, result)| {
            let index = offset + visible_index;
            let suffix = if result.is_dir { "/" } else { "" };
            let style = if index == selected {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(
                format!("{}{}", result.display_path, suffix),
                style,
            )))
        })
        .collect::<Vec<_>>();

    let list = List::new(items);
    frame.render_widget(list, results_area);
    set_input_cursor(frame, inner, 0, "Search: ", &app.search.query);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

fn render_launcher(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(76, 58, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" New Terminal Tab ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let field_style = |field| {
        if app.launcher.field == field {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::default()
        }
    };
    let source_style = || {
        if app.launcher.field == LauncherField::Source {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        }
    };
    let choices = app.launcher_choice_labels();
    let mut lines = vec![Line::from(Span::styled("Source:", source_style()))];
    lines.extend(choices.iter().enumerate().map(|(index, label)| {
        let marker = if index == app.launcher.source_index {
            "> "
        } else {
            "  "
        };
        let style = if index == app.launcher.source_index {
            field_style(LauncherField::Source)
        } else {
            Style::default()
        };
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{marker}{label}"), style),
        ])
    }));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("Name:    "),
        Span::styled(app.launcher.name.clone(), field_style(LauncherField::Name)),
    ]));
    lines.push(Line::from(vec![
        Span::raw("Command: "),
        Span::styled(
            app.launcher.command.clone(),
            field_style(LauncherField::Command),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw("Cwd:     "),
        Span::styled(app.launcher.cwd.clone(), field_style(LauncherField::Cwd)),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(
        "Tab/Shift-Tab field | Up/Down source | Enter launch | Esc cancel",
    ));
    frame.render_widget(Paragraph::new(lines), inner);

    let name_line = choices.len() as u16 + 2;
    match app.launcher.field {
        LauncherField::Source => {}
        LauncherField::Name => {
            set_input_cursor(frame, inner, name_line, "Name:    ", &app.launcher.name)
        }
        LauncherField::Command => set_input_cursor(
            frame,
            inner,
            name_line.saturating_add(1),
            "Command: ",
            &app.launcher.command,
        ),
        LauncherField::Cwd => set_input_cursor(
            frame,
            inner,
            name_line.saturating_add(2),
            "Cwd:     ",
            &app.launcher.cwd,
        ),
    }
}

fn render_rename(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(55, 25, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Rename Tab ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("Name: {}", app.rename.value)),
            Line::from(""),
            Line::from("Enter save | Esc cancel"),
        ]),
        inner,
    );
    set_input_cursor(frame, inner, 0, "Name: ", &app.rename.value);
}

fn render_confirm(frame: &mut Frame<'_>, title: &str, help: &str) {
    let area = centered_rect(45, 22, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(help.to_string()).alignment(Alignment::Center),
        inner,
    );
}

fn render_help(frame: &mut Frame<'_>) {
    let area = centered_rect(78, 70, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let text = vec![
        Line::from("Files and inactive terminal tabs"),
        Line::from("1..9          Select tab by number"),
        Line::from("Tab/BackTab   Next/previous tab"),
        Line::from("c             New terminal tab"),
        Line::from("?             Help"),
        Line::from("q             Quit with confirmation"),
        Line::from(""),
        Line::from("Terminal command prefix, used while a process is running"),
        Line::from("Ctrl-b 1..9   Select tab"),
        Line::from("Ctrl-b n/p    Next/previous tab"),
        Line::from("Ctrl-b f      Files tab"),
        Line::from("Ctrl-b c      New terminal tab"),
        Line::from("Ctrl-b x      Stop or close current terminal tab"),
        Line::from("Ctrl-b r      Restart current terminal tab"),
        Line::from("Ctrl-b e      Reload configuration"),
        Line::from("Ctrl-b q      Quit with confirmation"),
        Line::from("Ctrl-b ,      Rename temporary tab"),
        Line::from("Ctrl-b ?      Help"),
        Line::from("Ctrl-b Ctrl-b Send literal Ctrl-b"),
        Line::from("Ctrl-g        Prompt overlay for active terminal"),
        Line::from(""),
        Line::from("Tab markers: * recent output, . quiet after output, ! exited/failed."),
        Line::from(""),
        Line::from("Files: v opens selected file in an editor tab, e opens externally."),
        Line::from("Files: ]/[ selects Markdown preview links, Enter opens the selected link."),
        Line::from("Esc, Enter, or q closes this help."),
    ];
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), inner);
}

fn render_prompt(frame: &mut Frame<'_>, app: &App) {
    let title = app
        .active_tab()
        .map(|tab| format!(" Send input to {} ", tab.title))
        .unwrap_or_else(|| " Send input ".to_string());
    let area = centered_rect(72, 45, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);
    let lines = app
        .prompt
        .text
        .split('\n')
        .map(|line| Line::from(line.to_string()))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), chunks[0]);
    frame.render_widget(
        Paragraph::new("Enter send | Alt-Enter newline | Esc cancel"),
        chunks[2],
    );
    set_multiline_input_cursor(frame, chunks[0], &app.prompt.text);
}

fn set_input_cursor(frame: &mut Frame<'_>, area: Rect, line: u16, prefix: &str, value: &str) {
    if let Some(position) = input_cursor_position(area, line, prefix, value) {
        frame.set_cursor_position(position);
    }
}

fn set_multiline_input_cursor(frame: &mut Frame<'_>, area: Rect, value: &str) {
    if let Some(position) = multiline_input_cursor_position(area, value) {
        frame.set_cursor_position(position);
    }
}

fn input_cursor_position(area: Rect, line: u16, prefix: &str, value: &str) -> Option<Position> {
    if area.width == 0 || area.height == 0 {
        return None;
    }

    let x_offset = display_width(prefix).saturating_add(display_width(value));
    Some(Position::new(
        area.x
            .saturating_add(x_offset.min(area.width.saturating_sub(1))),
        area.y
            .saturating_add(line.min(area.height.saturating_sub(1))),
    ))
}

fn multiline_input_cursor_position(area: Rect, value: &str) -> Option<Position> {
    if area.width == 0 || area.height == 0 {
        return None;
    }

    let line_count = value.split('\n').count();
    let last_line = value.rsplit('\n').next().unwrap_or_default();
    let y_offset = (line_count.saturating_sub(1) as u16).min(area.height.saturating_sub(1));
    let x_offset = display_width(last_line).min(area.width.saturating_sub(1));
    Some(Position::new(
        area.x.saturating_add(x_offset),
        area.y.saturating_add(y_offset),
    ))
}

fn display_width(value: &str) -> u16 {
    UnicodeWidthStr::width(value).min(u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_cursor_lands_after_prefix_and_value() {
        let area = Rect::new(10, 5, 40, 4);

        let position = input_cursor_position(area, 1, "Command: ", "cargo test").unwrap();

        assert_eq!(position, Position::new(29, 6));
    }

    #[test]
    fn input_cursor_clamps_to_field_area() {
        let area = Rect::new(10, 5, 12, 2);

        let position = input_cursor_position(area, 7, "Command: ", "a very long command").unwrap();

        assert_eq!(position, Position::new(21, 6));
    }

    #[test]
    fn multiline_cursor_tracks_the_last_line() {
        let area = Rect::new(2, 3, 40, 8);

        let position = multiline_input_cursor_position(area, "first\nsecond").unwrap();

        assert_eq!(position, Position::new(8, 4));
    }

    #[test]
    fn multiline_cursor_handles_trailing_newline() {
        let area = Rect::new(2, 3, 40, 8);

        let position = multiline_input_cursor_position(area, "first\n").unwrap();

        assert_eq!(position, Position::new(2, 4));
    }
}
