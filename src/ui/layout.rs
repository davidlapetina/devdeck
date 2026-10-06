use ratatui::prelude::*;

use crate::app::FilesPane;

#[derive(Debug, Clone, Copy)]
pub struct AppAreas {
    pub tabs: Rect,
    pub content: Rect,
    pub files: Rect,
    pub preview: Rect,
    pub status: Rect,
}

pub fn areas(area: Rect) -> AppAreas {
    areas_for_files(area, None)
}

pub fn areas_for_files(area: Rect, maximized: Option<FilesPane>) -> AppAreas {
    let vertical = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);
    let (files, preview) = match maximized {
        Some(FilesPane::Tree) => (
            vertical[1],
            Rect::new(vertical[1].right(), vertical[1].y, 0, 0),
        ),
        Some(FilesPane::Preview) => (Rect::new(vertical[1].x, vertical[1].y, 0, 0), vertical[1]),
        None => {
            let horizontal =
                Layout::horizontal([Constraint::Percentage(32), Constraint::Percentage(68)])
                    .split(vertical[1]);
            (horizontal[0], horizontal[1])
        }
    };

    AppAreas {
        tabs: vertical[0],
        content: vertical[1],
        files,
        preview,
        status: vertical[2],
    }
}

pub fn terminal_dimensions(content_area: Rect) -> crate::app::TerminalDimensions {
    crate::app::TerminalDimensions {
        rows: content_area.height.saturating_sub(2).max(1),
        cols: content_area.width.saturating_sub(2).max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximized_files_panes_take_the_full_content_area() {
        let area = Rect::new(0, 0, 100, 30);
        let split = areas_for_files(area, None);
        let tree = areas_for_files(area, Some(FilesPane::Tree));
        let preview = areas_for_files(area, Some(FilesPane::Preview));

        assert!(split.files.width > 0);
        assert!(split.preview.width > 0);
        assert_eq!(tree.files, tree.content);
        assert_eq!(tree.preview.width, 0);
        assert_eq!(preview.preview, preview.content);
        assert_eq!(preview.files.width, 0);
    }
}
