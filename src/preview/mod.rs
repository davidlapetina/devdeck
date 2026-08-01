use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::filesystem::file_type::{detect_path, DetectedFileType, Language};

pub mod markdown;
pub mod syntax;
pub mod text;

pub const MAX_PREVIEW_SIZE: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub enum PreviewContent {
    Empty,
    Directory { entries: Vec<String> },
    Text { content: String, language: Language },
    Markdown { content: String },
    Binary { name: String },
    TooLarge,
    Error { message: String },
}

#[derive(Debug, Clone)]
pub struct PreviewState {
    pub path: Option<PathBuf>,
    pub file_type: DetectedFileType,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub content: PreviewContent,
    pub scroll: usize,
    pub viewport_height: usize,
    pub rendered_line_count: usize,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            path: None,
            file_type: DetectedFileType::Text(Language::Text),
            size: None,
            modified: None,
            content: PreviewContent::Empty,
            scroll: 0,
            viewport_height: 0,
            rendered_line_count: 0,
        }
    }
}

impl PreviewState {
    pub fn load(path: &Path, preserve_scroll: bool) -> Self {
        let previous_scroll = if preserve_scroll { Some(0) } else { None };
        let mut state = load_preview(path);
        if let Some(scroll) = previous_scroll {
            state.scroll = scroll;
        }
        state
    }

    pub fn load_with_scroll(path: &Path, scroll: usize) -> Self {
        let mut state = load_preview(path);
        state.scroll = scroll;
        state
    }

    pub fn set_measurements(&mut self, viewport_height: usize, rendered_line_count: usize) {
        self.viewport_height = viewport_height;
        self.rendered_line_count = rendered_line_count;
        self.clamp_scroll();
    }

    pub fn max_scroll(&self) -> usize {
        self.rendered_line_count
            .saturating_sub(self.viewport_height)
    }

    pub fn clamp_scroll(&mut self) {
        let max_scroll = self.max_scroll();
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
    }

    pub fn scroll_lines(&mut self, delta: isize) {
        if delta.is_negative() {
            self.scroll = self.scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.scroll = self.scroll.saturating_add(delta as usize);
        }
        self.clamp_scroll();
    }

    pub fn scroll_page_down(&mut self) {
        let amount = self.viewport_height.saturating_sub(1).max(1);
        self.scroll = self.scroll.saturating_add(amount);
        self.clamp_scroll();
    }

    pub fn scroll_page_up(&mut self) {
        let amount = self.viewport_height.saturating_sub(1).max(1);
        self.scroll = self.scroll.saturating_sub(amount);
    }

    pub fn scroll_top(&mut self) {
        self.scroll = 0;
    }

    pub fn scroll_bottom(&mut self) {
        self.scroll = self.max_scroll();
    }
}

pub fn render_lines(
    preview: &PreviewState,
    render_markdown: bool,
    width: u16,
) -> Vec<Line<'static>> {
    match &preview.content {
        PreviewContent::Empty => vec![Line::from("")],
        PreviewContent::Directory { entries } => render_directory(preview, entries),
        PreviewContent::Text { content, language } => syntax::highlight(content, Some(*language)),
        PreviewContent::Markdown { content } if render_markdown => {
            markdown::render_markdown(content, width.saturating_sub(2) as usize)
        }
        PreviewContent::Markdown { content } => {
            syntax::highlight(content, Some(Language::Markdown))
        }
        PreviewContent::Binary { name } => render_binary(preview, name),
        PreviewContent::TooLarge => render_too_large(preview),
        PreviewContent::Error { message } => vec![Line::from(vec![Span::styled(
            message.clone(),
            Style::default().fg(Color::Red),
        )])],
    }
}

pub fn format_size(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.0} KB", size as f64 / KB)
    } else if size < 1024 * 1024 * 1024 {
        format!("{:.1} MB", size as f64 / MB)
    } else {
        format!("{:.1} GB", size as f64 / GB)
    }
}

fn load_preview(path: &Path) -> PreviewState {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return PreviewState {
                path: Some(path.to_path_buf()),
                file_type: DetectedFileType::Text(Language::Text),
                content: PreviewContent::Error {
                    message: format!("Unable to read file: {error}"),
                },
                ..PreviewState::default()
            }
        }
    };

    let modified = metadata.modified().ok();
    let size = Some(metadata.len());

    if metadata.is_dir() {
        return PreviewState {
            path: Some(path.to_path_buf()),
            file_type: DetectedFileType::Directory,
            size,
            modified,
            content: PreviewContent::Directory {
                entries: directory_entries(path),
            },
            ..PreviewState::default()
        };
    }

    if metadata.len() > MAX_PREVIEW_SIZE {
        return PreviewState {
            path: Some(path.to_path_buf()),
            file_type: DetectedFileType::TooLarge,
            size,
            modified,
            content: PreviewContent::TooLarge,
            ..PreviewState::default()
        };
    }

    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return PreviewState {
                path: Some(path.to_path_buf()),
                file_type: DetectedFileType::Text(Language::Text),
                size,
                modified,
                content: PreviewContent::Error {
                    message: format!("Unable to read file: {error}"),
                },
                ..PreviewState::default()
            }
        }
    };

    if bytes.contains(&0) {
        return PreviewState {
            path: Some(path.to_path_buf()),
            file_type: DetectedFileType::Binary,
            size,
            modified,
            content: PreviewContent::Binary {
                name: file_name(path),
            },
            ..PreviewState::default()
        };
    }

    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(_) => {
            return PreviewState {
                path: Some(path.to_path_buf()),
                file_type: DetectedFileType::Binary,
                size,
                modified,
                content: PreviewContent::Binary {
                    name: file_name(path),
                },
                ..PreviewState::default()
            }
        }
    };

    match detect_path(path, false) {
        DetectedFileType::Markdown => PreviewState {
            path: Some(path.to_path_buf()),
            file_type: DetectedFileType::Markdown,
            size,
            modified,
            content: PreviewContent::Markdown { content },
            ..PreviewState::default()
        },
        DetectedFileType::Source(language) | DetectedFileType::Text(language) => PreviewState {
            path: Some(path.to_path_buf()),
            file_type: detect_path(path, false),
            size,
            modified,
            content: PreviewContent::Text { content, language },
            ..PreviewState::default()
        },
        other => PreviewState {
            path: Some(path.to_path_buf()),
            file_type: other,
            size,
            modified,
            content: PreviewContent::Text {
                content,
                language: Language::Text,
            },
            ..PreviewState::default()
        },
    }
}

fn render_directory(preview: &PreviewState, entries: &[String]) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled("Directory", Style::default().fg(Color::Cyan))),
        Line::from(""),
        Line::from(format!(
            "Name: {}",
            preview
                .path
                .as_deref()
                .map(file_name)
                .unwrap_or_else(|| ".".to_string())
        )),
        Line::from(format!("Entries: {}", entries.len())),
        Line::from(""),
    ];

    lines.extend(entries.iter().map(|entry| Line::from(entry.clone())));
    lines
}

fn render_binary(preview: &PreviewState, name: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "Binary file",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(format!("Name: {name}")),
        Line::from(format!(
            "Size: {}",
            preview
                .size
                .map(format_size)
                .unwrap_or_else(|| "unknown".to_string())
        )),
        Line::from(format!("Modified: {}", format_modified(preview.modified))),
    ]
}

fn render_too_large(preview: &PreviewState) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "Preview unavailable",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(format!(
            "File size: {}",
            preview
                .size
                .map(format_size)
                .unwrap_or_else(|| "unknown".to_string())
        )),
        Line::from(format!(
            "Maximum preview size: {}",
            format_size(MAX_PREVIEW_SIZE)
        )),
    ]
}

fn directory_entries(path: &Path) -> Vec<String> {
    let mut entries = match fs::read_dir(path) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| {
                let is_dir = entry.file_type().map(|ty| ty.is_dir()).unwrap_or(false);
                let suffix = if is_dir { "/" } else { "" };
                format!("{}{}", entry.file_name().to_string_lossy(), suffix)
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    entries.sort_by_key(|entry| entry.to_ascii_lowercase());
    entries
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| path.as_os_str().to_str().unwrap_or(""))
        .to_string()
}

pub fn format_modified(modified: Option<SystemTime>) -> String {
    modified
        .map(|modified| {
            let datetime: chrono::DateTime<chrono::Local> = modified.into();
            datetime.format("%Y-%m-%d %H:%M").to_string()
        })
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn refuses_to_load_files_over_the_preview_limit() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("large.txt");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_PREVIEW_SIZE + 1).unwrap();

        let preview = PreviewState::load(&path, false);

        assert!(matches!(preview.content, PreviewContent::TooLarge));
        assert_eq!(preview.file_type, DetectedFileType::TooLarge);
    }

    #[test]
    fn treats_invalid_utf8_as_binary() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("bytes.bin");
        fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();

        let preview = PreviewState::load(&path, false);

        assert!(matches!(preview.content, PreviewContent::Binary { .. }));
        assert_eq!(preview.file_type, DetectedFileType::Binary);
    }
}
