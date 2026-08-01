use ratatui::prelude::*;

#[derive(Debug, Clone, Copy)]
pub struct AppAreas {
    pub tabs: Rect,
    pub content: Rect,
    pub files: Rect,
    pub preview: Rect,
    pub status: Rect,
}

pub fn areas(area: Rect) -> AppAreas {
    let vertical = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);
    let horizontal = Layout::horizontal([Constraint::Percentage(32), Constraint::Percentage(68)])
        .split(vertical[1]);

    AppAreas {
        tabs: vertical[0],
        content: vertical[1],
        files: horizontal[0],
        preview: horizontal[1],
        status: vertical[2],
    }
}

pub fn terminal_dimensions(content_area: Rect) -> crate::app::TerminalDimensions {
    crate::app::TerminalDimensions {
        rows: content_area.height.saturating_sub(2).max(1),
        cols: content_area.width.saturating_sub(2).max(1),
    }
}
