use ratatui::{prelude::*, widgets::*};

use crate::app::{App, FileListMode};

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let title = match app.file_list_mode {
        FileListMode::Tree => " Files ",
        FileListMode::Marked => " Marked Files ",
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner_height = area.height.saturating_sub(2) as usize;
    let selected = app.selected_index;
    let offset = selected.saturating_sub(inner_height.saturating_sub(1));

    let items = app
        .visible_entries
        .iter()
        .skip(offset)
        .take(inner_height)
        .enumerate()
        .map(|(visible_index, entry)| {
            let index = offset + visible_index;
            let indent = "  ".repeat(entry.depth);
            let marker = if entry.is_dir {
                if app.file_list_mode == FileListMode::Marked {
                    "  "
                } else if app.expanded_directories.contains(&entry.path) {
                    "v "
                } else {
                    "> "
                }
            } else {
                "  "
            };
            let mark = if app.is_marked(&entry.path) { "*" } else { " " };
            let reminder = if app.active_reminder_count(&entry.path) > 0 {
                "!"
            } else if entry.is_dir && app.has_active_reminder_under(&entry.path) {
                "r"
            } else {
                " "
            };
            let suffix = if entry.is_dir { "/" } else { "" };
            let style = if index == selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else if entry.is_dir {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(
                format!("{indent}{mark}{reminder}{marker}{}{suffix}", entry.name),
                style,
            )))
        })
        .collect::<Vec<_>>();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}
