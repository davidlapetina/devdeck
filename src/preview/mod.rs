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
pub const MAX_IMAGE_PREVIEW_SIZE: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ImagePreview {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum PreviewContent {
    Empty,
    Directory { entries: Vec<String> },
    Image(ImagePreview),
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
    pub viewport_width: usize,
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
            viewport_width: 0,
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

    pub fn set_measurements(
        &mut self,
        viewport_width: usize,
        viewport_height: usize,
        rendered_line_count: usize,
    ) {
        self.viewport_width = viewport_width;
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
    height: u16,
    focused_link: Option<usize>,
) -> Vec<Line<'static>> {
    match &preview.content {
        PreviewContent::Empty => vec![Line::from("")],
        PreviewContent::Directory { entries } => render_directory(preview, entries),
        PreviewContent::Image(image) => render_image(preview, image, width, height),
        PreviewContent::Text { content, language } => syntax::highlight(content, Some(*language)),
        PreviewContent::Markdown { content } if render_markdown => {
            markdown::render_markdown_with_focus(
                content,
                width.saturating_sub(2) as usize,
                focused_link,
            )
            .lines
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

    let detected = detect_path(path, false);
    let max_preview_size = if detected == DetectedFileType::Image {
        MAX_IMAGE_PREVIEW_SIZE
    } else {
        MAX_PREVIEW_SIZE
    };

    if metadata.len() > max_preview_size {
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

    if detected == DetectedFileType::Image {
        return match image::load_from_memory(&bytes) {
            Ok(image) => {
                let rgba = image.to_rgba8();
                PreviewState {
                    path: Some(path.to_path_buf()),
                    file_type: DetectedFileType::Image,
                    size,
                    modified,
                    content: PreviewContent::Image(ImagePreview {
                        name: file_name(path),
                        width: rgba.width(),
                        height: rgba.height(),
                        rgba: rgba.into_raw(),
                    }),
                    ..PreviewState::default()
                }
            }
            Err(error) => PreviewState {
                path: Some(path.to_path_buf()),
                file_type: DetectedFileType::Image,
                size,
                modified,
                content: PreviewContent::Error {
                    message: format!("Unable to decode image: {error}"),
                },
                ..PreviewState::default()
            },
        };
    }

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

    match detected {
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
            file_type: detected,
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

fn render_image(
    preview: &PreviewState,
    image: &ImagePreview,
    width: u16,
    height: u16,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled("Image", Style::default().fg(Color::Cyan))),
        Line::from(""),
        Line::from(format!("Name: {}", image.name)),
        Line::from(format!("Dimensions: {} x {}", image.width, image.height)),
        Line::from(format!(
            "Size: {}",
            preview
                .size
                .map(format_size)
                .unwrap_or_else(|| "unknown".to_string())
        )),
        Line::from(format!("Modified: {}", format_modified(preview.modified))),
        Line::from(""),
    ];

    if image.width == 0 || image.height == 0 || image.rgba.is_empty() {
        lines.push(Line::from("Image has no pixels"));
        return lines;
    }

    let available_width = u32::from(width).max(1);
    let available_rows = u32::from(height).saturating_sub(lines.len() as u32).max(1);
    let available_pixel_height = available_rows.saturating_mul(2);
    let scale = (available_width as f64 / image.width as f64)
        .min(available_pixel_height as f64 / image.height as f64)
        .min(1.0);
    let output_width = ((image.width as f64 * scale).floor() as u32).max(1);
    let output_height = ((image.height as f64 * scale).floor() as u32).max(1);
    let output_rows = output_height.div_ceil(2);

    for row in 0..output_rows {
        let mut spans = Vec::with_capacity(output_width as usize);
        for col in 0..output_width {
            let top = sample_scaled_pixel(image, col, row * 2, output_width, output_height);
            let bottom = if row * 2 + 1 < output_height {
                sample_scaled_pixel(image, col, row * 2 + 1, output_width, output_height)
            } else {
                Rgb::BLACK
            };
            spans.push(Span::styled(
                "▀",
                Style::default()
                    .fg(Color::Rgb(top.red, top.green, top.blue))
                    .bg(Color::Rgb(bottom.red, bottom.green, bottom.blue)),
            ));
        }
        lines.push(Line::from(spans));
    }

    lines
}

#[derive(Debug, Clone, Copy)]
struct Rgb {
    red: u8,
    green: u8,
    blue: u8,
}

impl Rgb {
    const BLACK: Self = Self {
        red: 0,
        green: 0,
        blue: 0,
    };
}

fn sample_scaled_pixel(
    image: &ImagePreview,
    output_x: u32,
    output_y: u32,
    output_width: u32,
    output_height: u32,
) -> Rgb {
    let source_x = scale_coordinate(output_x, output_width, image.width);
    let source_y = scale_coordinate(output_y, output_height, image.height);
    let index = ((source_y * image.width + source_x) * 4) as usize;
    let red = image.rgba.get(index).copied().unwrap_or_default();
    let green = image.rgba.get(index + 1).copied().unwrap_or_default();
    let blue = image.rgba.get(index + 2).copied().unwrap_or_default();
    let alpha = image.rgba.get(index + 3).copied().unwrap_or(255);
    blend_with_black(red, green, blue, alpha)
}

fn scale_coordinate(output: u32, output_size: u32, source_size: u32) -> u32 {
    let scaled = (u64::from(output) * u64::from(source_size)) / u64::from(output_size);
    scaled.min(u64::from(source_size.saturating_sub(1))) as u32
}

fn blend_with_black(red: u8, green: u8, blue: u8, alpha: u8) -> Rgb {
    let alpha = u16::from(alpha);
    Rgb {
        red: ((u16::from(red) * alpha) / 255) as u8,
        green: ((u16::from(green) * alpha) / 255) as u8,
        blue: ((u16::from(blue) * alpha) / 255) as u8,
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

    #[test]
    fn loads_png_images_for_preview() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("pixel.png");
        fs::write(&path, tiny_png()).unwrap();

        let preview = PreviewState::load(&path, false);

        assert_eq!(preview.file_type, DetectedFileType::Image);
        let PreviewContent::Image(image) = preview.content else {
            panic!("expected image preview");
        };
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!(image.rgba.len(), 4);
    }

    fn tiny_png() -> &'static [u8] {
        &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99,
            0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ]
    }
}
