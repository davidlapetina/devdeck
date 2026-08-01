use ratatui::{prelude::*, widgets::*};

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
        InputMode::Help => render_help(frame),
        InputMode::PromptOverlay => render_prompt(frame, app),
        InputMode::Repository | InputMode::Terminal | InputMode::CommandPrefix => {}
    }
}

fn render_search(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(format!(" Search: {} ", app.search.query))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let height = inner.height.saturating_sub(1) as usize;
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
    frame.render_widget(list, inner);
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
    let area = centered_rect(70, 45, frame.area());
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
    let lines = vec![
        Line::from(vec![
            Span::raw("Name:    "),
            Span::styled(app.launcher.name.clone(), field_style(LauncherField::Name)),
        ]),
        Line::from(vec![
            Span::raw("Command: "),
            Span::styled(
                app.launcher.command.clone(),
                field_style(LauncherField::Command),
            ),
        ]),
        Line::from(vec![
            Span::raw("Cwd:     "),
            Span::styled(app.launcher.cwd.clone(), field_style(LauncherField::Cwd)),
        ]),
        Line::from(""),
        Line::from("Tab/Shift-Tab field | Enter launch | Esc cancel"),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
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
        Line::from("Ctrl-b 1..9   Select tab"),
        Line::from("Ctrl-b n/p    Next/previous tab"),
        Line::from("Ctrl-b f      Files tab"),
        Line::from("Ctrl-b c      New temporary command tab"),
        Line::from("Ctrl-b x      Stop or close current terminal tab"),
        Line::from("Ctrl-b r      Restart current terminal tab"),
        Line::from("Ctrl-b e      Reload configuration"),
        Line::from("Ctrl-b ,      Rename temporary tab"),
        Line::from("Ctrl-b ?      Help"),
        Line::from("Ctrl-b Ctrl-b Send literal Ctrl-b"),
        Line::from("Ctrl-g        Prompt overlay for active terminal"),
        Line::from(""),
        Line::from("Files: v opens selected file in an editor tab, e opens externally."),
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
    let text = if app.prompt.text.is_empty() {
        " ".to_string()
    } else {
        app.prompt.text.clone()
    };
    let lines = vec![
        Line::from(text),
        Line::from(""),
        Line::from("Enter send | Alt-Enter newline | Esc cancel"),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
