use ratatui::{prelude::*, widgets::*};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::{App, InputMode, LauncherField, ReminderField},
    tabs::{ActivityState, TabContent, TerminalTabState},
};

pub mod layout;
pub mod preview;
pub mod status;
pub mod tabs;
pub mod terminal;
pub mod tree;

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let areas = layout::areas_for_files(frame.area(), app.maximized_files_pane);
    tabs::render(frame, areas.tabs, app);

    match app.active_tab().map(|tab| &tab.content) {
        Some(TabContent::Repository) | None => {
            if areas.files.width > 0 && areas.files.height > 0 {
                tree::render(frame, areas.files, app);
            }
            if areas.preview.width > 0 && areas.preview.height > 0 {
                preview::render(frame, areas.preview, app);
            }
        }
        Some(TabContent::Terminal(_)) => {
            let dimensions = layout::terminal_dimensions(areas.content);
            app.resize_active_terminal(dimensions);
            terminal::render(frame, areas.content, app);
        }
    }

    status::render(frame, areas.status, app);

    if app.search.active
        && matches!(
            app.active_tab().map(|tab| &tab.content),
            Some(TabContent::Repository)
        )
    {
        render_search(frame, app);
    }

    match app.input_mode {
        InputMode::TabLauncher => render_launcher(frame, app),
        InputMode::TabSwitcher => render_tab_switcher(frame, app),
        InputMode::RenameTab => render_rename(frame, app),
        InputMode::RenamePath => render_rename(frame, app),
        InputMode::FileActions => render_file_actions(frame, app),
        InputMode::ConfirmStop => {
            render_confirm(frame, "Stop terminal?", "Enter/y stop | Esc/n cancel")
        }
        InputMode::ConfirmRestart => {
            render_confirm(frame, "Restart terminal?", "Enter/y restart | Esc/n cancel")
        }
        InputMode::ConfirmQuit => {
            render_confirm(frame, "Quit DevDeck?", "Enter/y quit | Esc/n cancel")
        }
        InputMode::Help => render_help(frame, app),
        InputMode::PromptOverlay => render_prompt(frame, app),
        InputMode::AgentPrompt => render_agent_prompt(frame, app),
        InputMode::ReminderEditor => render_reminder_editor(frame, app),
        InputMode::Reminders => render_reminders(frame, app),
        InputMode::Repository | InputMode::Terminal | InputMode::CommandPrefix => {}
    }
}

fn render_tab_switcher(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(68, 62, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Switch Tab ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("Filter: "),
            Span::styled(
                app.tab_switcher.query.clone(),
                Style::default().fg(Color::Yellow),
            ),
        ])),
        chunks[0],
    );

    let matches = app.tab_switcher_matches();
    if matches.is_empty() {
        frame.render_widget(Paragraph::new("No matching tabs"), chunks[1]);
    } else {
        let height = chunks[1].height as usize;
        let selected = app.tab_switcher.selected.min(matches.len() - 1);
        let offset = selected.saturating_sub(height.saturating_sub(1));
        let items = matches
            .iter()
            .skip(offset)
            .take(height)
            .enumerate()
            .map(|(visible_index, index)| {
                let row = offset + visible_index;
                let tab = &app.tabs[*index];
                let current = if *index == app.active_tab { "●" } else { " " };
                let state = match &tab.content {
                    TabContent::Repository => "files".to_string(),
                    TabContent::Terminal(terminal) => {
                        let state = match terminal.state {
                            TerminalTabState::NotStarted => "idle",
                            TerminalTabState::Starting => "starting",
                            TerminalTabState::Running => "running",
                            TerminalTabState::Exited { .. } => "exited",
                            TerminalTabState::Failed { .. } => "failed",
                        };
                        let activity = match tab.activity {
                            ActivityState::None => "",
                            ActivityState::OutputActive { .. } => " · output",
                            ActivityState::OutputQuiet => " · quiet",
                        };
                        format!("{state}{activity}")
                    }
                };
                let style = if row == selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(
                    format!(
                        "{current} {:>2}  {} {state}",
                        index + 1,
                        fixed_display_width(&tab.title, 24)
                    ),
                    style,
                )))
            })
            .collect::<Vec<_>>();
        frame.render_widget(List::new(items), chunks[1]);
    }

    frame.render_widget(
        Paragraph::new("Type to filter · ↑/↓ select · Enter open · Esc cancel"),
        chunks[2],
    );
    set_input_cursor(frame, chunks[0], 0, "Filter: ", &app.tab_switcher.query);
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
    let title = if app.launcher_selected_path_parameter().is_some() {
        " Run Command For Selection "
    } else {
        " New Terminal Tab "
    };
    let block = Block::default()
        .title(title)
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
    if let Some(parameter) = app.launcher_selected_path_parameter() {
        lines.push(Line::from(vec![
            Span::raw("Arg:     "),
            Span::styled(parameter, Style::default().fg(Color::Cyan)),
        ]));
    }
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

fn render_file_actions(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(60, 44, frame.area());
    frame.render_widget(Clear, area);
    let title = app
        .selected_path()
        .map(|path| format!(" Actions: {} ", app.relative_display(path)))
        .unwrap_or_else(|| " File Actions ".to_string());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let text = vec![
        Line::from("r   Rename"),
        Line::from("n   Copy file/folder name"),
        Line::from("y   Copy relative path"),
        Line::from("Y   Copy absolute path"),
        Line::from("!   Run command with selected path argument"),
        Line::from("g   Run configured agent with prompt"),
        Line::from("v   Open file in editor tab"),
        Line::from("m   Mark/unmark for this session"),
        Line::from("M   Show/hide marked-only list"),
        Line::from("d   Add reminder with due date"),
        Line::from("l   List reminders"),
        Line::from(""),
        Line::from("Esc cancel"),
    ];
    frame.render_widget(Paragraph::new(text), inner);
}

fn render_rename(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(55, 25, frame.area());
    frame.render_widget(Clear, area);
    let title = if app.input_mode == InputMode::RenamePath {
        " Rename File "
    } else {
        " Rename Tab "
    };
    let block = Block::default()
        .title(title)
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

fn render_reminder_editor(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(68, 34, frame.area());
    frame.render_widget(Clear, area);
    let title = format!(
        " Reminder For {} ",
        app.relative_display(&app.reminder_editor.path)
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let field_style = |field| {
        if app.reminder_editor.field == field {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::default()
        }
    };
    let lines = vec![
        Line::from(vec![
            Span::raw("Title: "),
            Span::styled(
                app.reminder_editor.title.clone(),
                field_style(ReminderField::Title),
            ),
        ]),
        Line::from(vec![
            Span::raw("Due:   "),
            Span::styled(
                app.reminder_editor.due.clone(),
                field_style(ReminderField::Due),
            ),
        ]),
        Line::from(""),
        Line::from("Due format: YYYY-MM-DD"),
        Line::from("Tab field | Enter save | Esc cancel"),
    ];
    frame.render_widget(Paragraph::new(lines), inner);

    match app.reminder_editor.field {
        ReminderField::Title => {
            set_input_cursor(frame, inner, 0, "Title: ", &app.reminder_editor.title)
        }
        ReminderField::Due => {
            set_input_cursor(frame, inner, 1, "Due:   ", &app.reminder_editor.due)
        }
    }
}

fn render_reminders(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(82, 70, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Reminders ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let help_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(2),
        inner.width,
        2,
    );
    let list_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(2),
    );
    let height = list_area.height as usize;
    let selected = app.reminder_list_selected;
    let offset = selected.saturating_sub(height.saturating_sub(1));
    let today = chrono::Local::now().date_naive();

    let items = app
        .reminders
        .reminders
        .iter()
        .skip(offset)
        .take(height)
        .enumerate()
        .map(|(visible_index, reminder)| {
            let index = offset + visible_index;
            let status = App::reminder_status_label(reminder);
            let done = if reminder.done { "x" } else { " " };
            let path = reminder.path.to_string_lossy();
            let style = if index == selected {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else if reminder.done {
                Style::default().fg(Color::DarkGray)
            } else if reminder.due_date().is_some_and(|due| due < today) {
                Style::default().fg(Color::Red)
            } else if reminder.due_date().is_some_and(|due| due == today) {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(
                format!(
                    "[{done}] {} {status:>7}  {}  {}",
                    reminder.due, path, reminder.title
                ),
                style,
            )))
        })
        .collect::<Vec<_>>();

    if items.is_empty() {
        frame.render_widget(Paragraph::new("No reminders"), list_area);
    } else {
        frame.render_widget(List::new(items), list_area);
    }
    frame.render_widget(
        Paragraph::new("Up/Down move | Enter select path | Space done | x delete | q/Esc close"),
        help_area,
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

fn render_help(frame: &mut Frame<'_>, app: &mut App) {
    let (area, body, footer) = help_areas(frame.area());
    frame.render_widget(Clear, area);
    let text = help_lines(app);
    let paragraph = Paragraph::new(text.clone()).wrap(Wrap { trim: true });
    let total = paragraph.line_count(body.width);
    let viewport = body.height as usize;
    let max_scroll = total.saturating_sub(viewport);
    app.help_scroll = app.help_scroll.min(max_scroll);
    let start = if total == 0 { 0 } else { app.help_scroll + 1 };
    let end = app.help_scroll.saturating_add(viewport).min(total);
    let block = Block::default()
        .title(format!(" Help · {start}–{end}/{total} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(block, area);
    frame.render_widget(paragraph.scroll((app.help_scroll as u16, 0)), body);
    frame.render_widget(
        Paragraph::new("j/k scroll · PgUp/PgDn · g/G ends · Esc/Enter/q close")
            .style(Style::default().fg(Color::Yellow)),
        footer,
    );
}

fn help_areas(frame_area: Rect) -> (Rect, Rect, Rect) {
    let area = centered_rect(78, 70, frame_area);
    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    (area, chunks[0], chunks[1])
}

fn help_lines(app: &App) -> Vec<Line<'static>> {
    vec![
        Line::from("Files and inactive terminal tabs"),
        Line::from("1..9          Select tab by number"),
        Line::from("Tab/BackTab   Next/previous tab"),
        Line::from(format!(
            "{} / {}   Previous/next tab (configured)",
            app.previous_tab_key(),
            app.next_tab_key()
        )),
        Line::from("Alt-t         Search and switch tabs"),
        Line::from("Alt-l         Switch to most recently used tab"),
        Line::from("c             New terminal tab"),
        Line::from("Alt-m         Cycle mouse policy: auto / selection / navigation"),
        Line::from("?             Help"),
        Line::from("q             Quit with confirmation"),
        Line::from(""),
        Line::from("Terminal command prefix, used while a process is running"),
        Line::from("Ctrl-b 1..9   Select tab"),
        Line::from("Ctrl-b n/p    Next/previous tab"),
        Line::from("Ctrl-b t      Search and switch tabs"),
        Line::from("Ctrl-b l      Switch to most recently used tab"),
        Line::from("Ctrl-b f      Files tab"),
        Line::from("Ctrl-b c      New terminal tab"),
        Line::from("Ctrl-b x      Stop or close current terminal tab"),
        Line::from("Ctrl-b r      Restart current terminal tab"),
        Line::from("Ctrl-b e      Reload configuration"),
        Line::from("Ctrl-b m      Cycle mouse policy"),
        Line::from("Ctrl-b q      Quit with confirmation"),
        Line::from("Ctrl-b ,      Rename temporary tab"),
        Line::from("Ctrl-b ?      Help"),
        Line::from("Ctrl-b Ctrl-b Send literal Ctrl-b"),
        Line::from("Ctrl-g        Prompt overlay for active terminal"),
        Line::from(""),
        Line::from("Mouse auto: navigate Files, preserve native selection in terminal tabs."),
        Line::from("Navigation: click tabs/tree and wheel panes; Selection: never capture."),
        Line::from(""),
        Line::from(
            "Tab markers: > shell prompt, * recent output, . quiet output, ! exited/failed.",
        ),
        Line::from(""),
        Line::from("Files: v opens selected file in an editor tab, e opens externally."),
        Line::from("Files: a opens file actions for rename, path copy, command, and agent launch."),
        Line::from("Files: Space marks files, M filters to marked files, T lists reminders."),
        Line::from("Files: f focuses Preview; Esc returns to Tree; z maximizes/restores focus."),
        Line::from("Preview: j/k or arrows scroll; Ctrl-d/u pages; g/G goes top/bottom."),
        Line::from("Preview: w toggles wrap; h/l scroll horizontally when unwrapped."),
        Line::from("Preview: N toggles source line numbers; title shows position/progress."),
        Line::from("Files: ]/[ selects Markdown preview links, Enter opens the selected link."),
        Line::from("Esc, Enter, or q closes this help."),
    ]
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

fn render_agent_prompt(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(76, 58, frame.area());
    frame.render_widget(Clear, area);
    let title = app
        .selected_path()
        .map(|path| format!(" Agent For {} ", app.relative_display(path)))
        .unwrap_or_else(|| " Agent Prompt ".to_string());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let choices = app.agent_choice_labels();
    let choices_height = choices.len() as u16 + 2;
    let prompt_height = inner
        .height
        .saturating_sub(choices_height)
        .saturating_sub(2);
    let chunks = Layout::vertical([
        Constraint::Length(choices_height),
        Constraint::Length(prompt_height.max(3)),
        Constraint::Length(2),
    ])
    .split(inner);

    let mut lines = vec![Line::from(Span::styled(
        "Agent:",
        Style::default().fg(Color::Yellow),
    ))];
    lines.extend(choices.iter().enumerate().map(|(index, label)| {
        let marker = if index == app.agent_prompt.source_index {
            "> "
        } else {
            "  "
        };
        let style = if index == app.agent_prompt.source_index {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::default()
        };
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{marker}{label}"), style),
        ])
    }));
    frame.render_widget(Paragraph::new(lines), chunks[0]);

    let prompt_lines = app
        .agent_prompt
        .text
        .split('\n')
        .map(|line| Line::from(line.to_string()))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(prompt_lines)
            .block(Block::default().title(" Prompt ").borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
    frame.render_widget(
        Paragraph::new("Up/Down agent | Enter launch | Alt-Enter newline | Esc cancel"),
        chunks[2],
    );
    let prompt_inner = Rect::new(
        chunks[1].x.saturating_add(1),
        chunks[1].y.saturating_add(1),
        chunks[1].width.saturating_sub(2),
        chunks[1].height.saturating_sub(2),
    );
    set_multiline_input_cursor(frame, prompt_inner, &app.agent_prompt.text);
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

fn fixed_display_width(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut result = String::new();
    let value_width = UnicodeWidthStr::width(value);
    let content_width = if value_width > width {
        width.saturating_sub(UnicodeWidthChar::width('…').unwrap_or(1))
    } else {
        width
    };
    let mut used = 0;
    for ch in value.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > content_width {
            break;
        }
        result.push(ch);
        used += ch_width;
    }
    if value_width > width {
        result.push('…');
        used += UnicodeWidthChar::width('…').unwrap_or(1);
    }
    result.push_str(&" ".repeat(width.saturating_sub(used)));
    result
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

    #[test]
    fn fixed_width_field_aligns_and_truncates_unicode_by_display_cells() {
        let ascii = fixed_display_width("Shell", 8);
        let wide = fixed_display_width("日本語", 8);
        let truncated = fixed_display_width("日本語 terminal", 8);

        assert_eq!(UnicodeWidthStr::width(ascii.as_str()), 8);
        assert_eq!(UnicodeWidthStr::width(wide.as_str()), 8);
        assert_eq!(UnicodeWidthStr::width(truncated.as_str()), 8);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn help_reserves_visible_controls_on_an_80_by_24_terminal() {
        let (overlay, body, footer) = help_areas(Rect::new(0, 0, 80, 24));
        assert!(overlay.width >= 60);
        assert!(body.height > 0);
        assert_eq!(footer.height, 1);
        assert_eq!(footer.y, body.bottom());

        let long_help = Paragraph::new(
            (0..40)
                .map(|index| Line::from(format!("Help line {index}")))
                .collect::<Vec<_>>(),
        )
        .wrap(Wrap { trim: true });
        assert!(long_help.line_count(body.width) > body.height as usize);
    }
}
