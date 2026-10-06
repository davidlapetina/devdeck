use std::{
    fs::{self, File},
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
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
pub const MAX_IMAGE_DIMENSION: u32 = 4096;
pub const MAX_IMAGE_DECODE_ALLOC: u64 = 64 * 1024 * 1024;
pub const MAX_DIRECTORY_PREVIEW_ENTRIES: usize = 512;
pub const MAX_IMAGE_RENDER_CELLS: usize = 16_384;

static NEXT_PREVIEW_GENERATION: AtomicU64 = AtomicU64::new(1);

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
    Directory {
        entries: Vec<String>,
        entries_truncated: bool,
    },
    Image(ImagePreview),
    Text {
        content: String,
        language: Language,
    },
    Markdown {
        content: String,
    },
    Binary {
        name: String,
    },
    TooLarge {
        max_bytes: u64,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct PreviewState {
    pub path: Option<PathBuf>,
    pub file_type: DetectedFileType,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub content: PreviewContent,
    pub scroll: usize,
    pub horizontal_scroll: usize,
    pub viewport_width: usize,
    pub viewport_height: usize,
    pub rendered_line_count: usize,
    pub rendered_column_count: usize,
    generation: u64,
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
            horizontal_scroll: 0,
            viewport_width: 0,
            viewport_height: 0,
            rendered_line_count: 0,
            rendered_column_count: 0,
            generation: NEXT_PREVIEW_GENERATION.fetch_add(1, Ordering::Relaxed),
        }
    }
}

impl PreviewState {
    pub fn generation(&self) -> u64 {
        self.generation
    }

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

    pub fn load_with_offsets(path: &Path, scroll: usize, horizontal_scroll: usize) -> Self {
        let mut state = load_preview(path);
        state.scroll = scroll;
        state.horizontal_scroll = horizontal_scroll;
        state
    }

    pub fn set_measurements(
        &mut self,
        viewport_width: usize,
        viewport_height: usize,
        rendered_line_count: usize,
        rendered_column_count: usize,
    ) {
        self.viewport_width = viewport_width;
        self.viewport_height = viewport_height;
        self.rendered_line_count = rendered_line_count;
        self.rendered_column_count = rendered_column_count;
        self.clamp_scroll();
        self.clamp_horizontal_scroll();
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

    pub fn max_horizontal_scroll(&self) -> usize {
        self.rendered_column_count
            .saturating_sub(self.viewport_width)
    }

    pub fn clamp_horizontal_scroll(&mut self) {
        self.horizontal_scroll = self.horizontal_scroll.min(self.max_horizontal_scroll());
    }

    pub fn scroll_columns(&mut self, delta: isize) {
        if delta.is_negative() {
            self.horizontal_scroll = self.horizontal_scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.horizontal_scroll = self.horizontal_scroll.saturating_add(delta as usize);
        }
        self.clamp_horizontal_scroll();
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
    show_source_line_numbers: bool,
) -> Vec<Line<'static>> {
    match &preview.content {
        PreviewContent::Empty => vec![Line::from("")],
        PreviewContent::Directory {
            entries,
            entries_truncated,
        } => render_directory(preview, entries, *entries_truncated),
        PreviewContent::Image(image) => render_image(preview, image, width, height),
        PreviewContent::Text { content, language } => {
            let lines = syntax::highlight(content, Some(*language));
            if show_source_line_numbers && matches!(preview.file_type, DetectedFileType::Source(_))
            {
                with_line_numbers(lines)
            } else {
                lines
            }
        }
        PreviewContent::Markdown { content } if render_markdown => {
            markdown::render_markdown_with_focus(content, width as usize, focused_link).lines
        }
        PreviewContent::Markdown { content } => {
            syntax::highlight(content, Some(Language::Markdown))
        }
        PreviewContent::Binary { name } => render_binary(preview, name),
        PreviewContent::TooLarge { max_bytes } => render_too_large(preview, *max_bytes),
        PreviewContent::Error { message } => vec![Line::from(vec![Span::styled(
            message.clone(),
            Style::default().fg(Color::Red),
        )])],
    }
}

fn with_line_numbers(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let digits = lines.len().max(1).to_string().len();
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let mut spans = Vec::with_capacity(line.spans.len() + 1);
            spans.push(Span::styled(
                format!("{:>digits$} │ ", index + 1),
                Style::default().fg(Color::DarkGray),
            ));
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
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
    // Directories are handled by read_dir. Files are opened before validation below so a path
    // replacement cannot swap a checked regular file for a FIFO/device before the read.
    let path_metadata = match fs::metadata(path) {
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

    if path_metadata.is_dir() {
        let (entries, entries_truncated) = directory_entries(path);
        return PreviewState {
            path: Some(path.to_path_buf()),
            file_type: DetectedFileType::Directory,
            size: Some(path_metadata.len()),
            modified: path_metadata.modified().ok(),
            content: PreviewContent::Directory {
                entries,
                entries_truncated,
            },
            ..PreviewState::default()
        };
    }

    let file = match open_preview_file(path) {
        Ok(file) => file,
        Err(error) => {
            return PreviewState {
                path: Some(path.to_path_buf()),
                file_type: DetectedFileType::Text(Language::Text),
                content: PreviewContent::Error {
                    message: format!("Unable to open file safely: {error}"),
                },
                ..PreviewState::default()
            }
        }
    };
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            return PreviewState {
                path: Some(path.to_path_buf()),
                file_type: DetectedFileType::Text(Language::Text),
                content: PreviewContent::Error {
                    message: format!("Unable to inspect opened file: {error}"),
                },
                ..PreviewState::default()
            }
        }
    };
    let modified = metadata.modified().ok();
    let size = Some(metadata.len());

    if !metadata.is_file() {
        return PreviewState {
            path: Some(path.to_path_buf()),
            file_type: DetectedFileType::Binary,
            size,
            modified,
            content: PreviewContent::Error {
                message: "Preview unavailable for non-regular file".to_string(),
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
            content: PreviewContent::TooLarge {
                max_bytes: max_preview_size,
            },
            ..PreviewState::default()
        };
    }

    let bytes = match read_bounded(&file, max_preview_size) {
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

    // The file can grow after the metadata check. The reader deliberately consumes at most one
    // byte beyond the threshold so that this race cannot cause an unbounded allocation.
    if bytes.len() as u64 > max_preview_size {
        let current_size = file
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or(bytes.len() as u64)
            .max(bytes.len() as u64);
        return PreviewState {
            path: Some(path.to_path_buf()),
            file_type: DetectedFileType::TooLarge,
            size: Some(current_size),
            modified,
            content: PreviewContent::TooLarge {
                max_bytes: max_preview_size,
            },
            ..PreviewState::default()
        };
    }

    if detected == DetectedFileType::Image {
        return match decode_image(&bytes) {
            Ok(image) => {
                let width = image.width();
                let height = image.height();
                let Some(rgba_len) = checked_rgba_len(width, height) else {
                    return image_limit_error(path, size, modified, "RGBA size overflow");
                };
                if rgba_len > MAX_IMAGE_DECODE_ALLOC {
                    return image_limit_error(
                        path,
                        size,
                        modified,
                        "RGBA output exceeds preview budget",
                    );
                }
                // Consume the DynamicImage so already-RGBA decoders can transfer their allocation
                // instead of retaining a second full decoded image during conversion.
                let rgba = image.into_rgba8();
                PreviewState {
                    path: Some(path.to_path_buf()),
                    file_type: DetectedFileType::Image,
                    size,
                    modified,
                    content: PreviewContent::Image(ImagePreview {
                        name: file_name(path),
                        width,
                        height,
                        rgba: rgba.into_raw(),
                    }),
                    ..PreviewState::default()
                }
            }
            Err(error) => image_limit_error(path, size, modified, &error.to_string()),
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

#[cfg(unix)]
fn open_preview_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_preview_file(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

fn read_bounded(file: &File, max_bytes: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024) as usize);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn image_decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_DECODE_ALLOC);
    limits
}

fn decode_image(bytes: &[u8]) -> image::ImageResult<image::DynamicImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(image_decode_limits());
    reader.decode()
}

fn checked_rgba_len(width: u32, height: u32) -> Option<u64> {
    u64::from(width)
        .checked_mul(u64::from(height))?
        .checked_mul(4)
}

fn image_limit_error(
    path: &Path,
    size: Option<u64>,
    modified: Option<SystemTime>,
    error: &str,
) -> PreviewState {
    PreviewState {
        path: Some(path.to_path_buf()),
        file_type: DetectedFileType::Image,
        size,
        modified,
        content: PreviewContent::Error {
            message: format!(
                "Unable to decode image: {error}. Limits: up to {} x {} pixels; {} decoder allocation budget (best effort)",
                MAX_IMAGE_DIMENSION,
                MAX_IMAGE_DIMENSION,
                format_size(MAX_IMAGE_DECODE_ALLOC)
            ),
        },
        ..PreviewState::default()
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
    // Metadata is rendered by the same wrapping Paragraph as the pixels. Count its actual
    // terminal rows so narrow previews cannot push the image below the viewport.
    let metadata_rows = ratatui::widgets::Paragraph::new(lines.clone())
        .wrap(ratatui::widgets::Wrap { trim: false })
        .line_count(width.max(1));
    let available_rows = u32::from(height).saturating_sub(metadata_rows as u32);
    if available_rows == 0 {
        return lines;
    }
    let available_pixel_height = available_rows.saturating_mul(2);
    let scale = (available_width as f64 / image.width as f64)
        .min(available_pixel_height as f64 / image.height as f64)
        .min(1.0);
    let (output_width, output_height) = bounded_image_output_size(image, scale);
    let output_rows = output_height.div_ceil(2);

    for row in 0..output_rows {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for col in 0..output_width {
            let top = sample_scaled_pixel(image, col, row * 2, output_width, output_height);
            let bottom = if row * 2 + 1 < output_height {
                sample_scaled_pixel(image, col, row * 2 + 1, output_width, output_height)
            } else {
                Rgb::BLACK
            };
            let style = Style::default()
                .fg(Color::Rgb(top.red, top.green, top.blue))
                .bg(Color::Rgb(bottom.red, bottom.green, bottom.blue));
            if let Some(last) = spans.last_mut().filter(|span| span.style == style) {
                last.content.to_mut().push('▀');
            } else {
                spans.push(Span::styled("▀", style));
            }
        }
        lines.push(Line::from(spans));
    }

    lines
}

fn bounded_image_output_size(image: &ImagePreview, initial_scale: f64) -> (u32, u32) {
    let source_cells = f64::from(image.width) * (f64::from(image.height) / 2.0);
    let budget_scale = if source_cells > MAX_IMAGE_RENDER_CELLS as f64 {
        (MAX_IMAGE_RENDER_CELLS as f64 / source_cells).sqrt()
    } else {
        1.0
    };
    let scale = initial_scale.min(budget_scale);
    let mut width = ((f64::from(image.width) * scale).floor() as u32).max(1);
    let mut height = ((f64::from(image.height) * scale).floor() as u32).max(1);
    while (width as usize).saturating_mul(height.div_ceil(2) as usize) > MAX_IMAGE_RENDER_CELLS {
        if width >= height && width > 1 {
            width -= 1;
        } else if height > 1 {
            height -= 1;
        } else {
            break;
        }
    }
    (width, height)
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

fn render_directory(
    preview: &PreviewState,
    entries: &[String],
    entries_truncated: bool,
) -> Vec<Line<'static>> {
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
        Line::from(if entries_truncated {
            format!("Entries: {}+", entries.len())
        } else {
            format!("Entries: {}", entries.len())
        }),
        Line::from(""),
    ];

    lines.extend(entries.iter().map(|entry| Line::from(entry.clone())));
    if entries_truncated {
        lines.push(Line::from(Span::styled(
            "… more entries not shown",
            Style::default().fg(Color::DarkGray),
        )));
    }
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

fn render_too_large(preview: &PreviewState, max_bytes: u64) -> Vec<Line<'static>> {
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
        Line::from(format!("Maximum preview size: {}", format_size(max_bytes))),
    ]
}

fn directory_entries(path: &Path) -> (Vec<String>, bool) {
    let Ok(read_dir) = fs::read_dir(path) else {
        return (Vec::new(), false);
    };
    let mut entries = Vec::with_capacity(MAX_DIRECTORY_PREVIEW_ENTRIES);
    let mut entries_truncated = false;
    for entry in read_dir
        .filter_map(Result::ok)
        .take(MAX_DIRECTORY_PREVIEW_ENTRIES + 1)
    {
        if entries.len() >= MAX_DIRECTORY_PREVIEW_ENTRIES {
            entries_truncated = true;
            break;
        }
        let is_dir = entry.file_type().map(|ty| ty.is_dir()).unwrap_or(false);
        let suffix = if is_dir { "/" } else { "" };
        entries.push(format!("{}{}", entry.file_name().to_string_lossy(), suffix));
    }
    entries.sort_by_key(|entry| entry.to_ascii_lowercase());
    (entries, entries_truncated)
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

        assert!(matches!(
            preview.content,
            PreviewContent::TooLarge {
                max_bytes: MAX_PREVIEW_SIZE
            }
        ));
        assert_eq!(preview.file_type, DetectedFileType::TooLarge);
    }

    #[test]
    fn image_decode_limits_reject_oversized_dimensions_and_allocations() {
        let limits = image_decode_limits();
        assert!(limits.check_dimensions(MAX_IMAGE_DIMENSION + 1, 1).is_err());
        assert!(limits.check_dimensions(1, MAX_IMAGE_DIMENSION + 1).is_err());

        let mut allocation_limits = image_decode_limits();
        assert!(allocation_limits
            .reserve(MAX_IMAGE_DECODE_ALLOC + 1)
            .is_err());
    }

    #[test]
    fn decoder_rejects_png_dimension_bomb_from_its_header() {
        let mut png = tiny_png().to_vec();
        png[16..20].copy_from_slice(&(MAX_IMAGE_DIMENSION + 1).to_be_bytes());
        let checksum = crc32(&png[12..29]);
        png[29..33].copy_from_slice(&checksum.to_be_bytes());

        assert!(matches!(
            decode_image(&png),
            Err(image::ImageError::Limits(_))
        ));
    }

    #[test]
    fn too_large_image_reports_the_image_input_limit() {
        let preview = PreviewState {
            size: Some(MAX_IMAGE_PREVIEW_SIZE + 1),
            content: PreviewContent::TooLarge {
                max_bytes: MAX_IMAGE_PREVIEW_SIZE,
            },
            ..PreviewState::default()
        };
        let rendered = render_lines(&preview, true, 80, 24, None, true);
        let text = rendered
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("20.0 MB"));
    }

    #[test]
    fn oversized_image_load_reports_actual_size_and_image_limit() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("large.png");
        let actual_size = MAX_IMAGE_PREVIEW_SIZE + 12_345;
        fs::File::create(&path)
            .unwrap()
            .set_len(actual_size)
            .unwrap();

        let preview = PreviewState::load(&path, false);

        assert_eq!(preview.size, Some(actual_size));
        assert!(matches!(
            preview.content,
            PreviewContent::TooLarge {
                max_bytes: MAX_IMAGE_PREVIEW_SIZE
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn image_symlink_cannot_bypass_compressed_input_limit() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let target = temp.path().join("large-target");
        let actual_size = MAX_IMAGE_PREVIEW_SIZE + 1;
        fs::File::create(&target)
            .unwrap()
            .set_len(actual_size)
            .unwrap();
        let path = temp.path().join("linked.png");
        symlink(&target, &path).unwrap();

        let preview = PreviewState::load(&path, false);

        assert_eq!(preview.size, Some(actual_size));
        assert!(matches!(
            preview.content,
            PreviewContent::TooLarge {
                max_bytes: MAX_IMAGE_PREVIEW_SIZE
            }
        ));
    }

    #[test]
    fn bounded_reader_never_consumes_more_than_one_byte_past_limit() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("growing.bin");
        fs::write(&path, vec![0_u8; 4096]).unwrap();

        let file = open_preview_file(&path).unwrap();
        let bytes = read_bounded(&file, 1024).unwrap();

        assert_eq!(bytes.len(), 1025);
    }

    #[cfg(unix)]
    #[test]
    fn opened_descriptor_is_safe_after_path_is_replaced() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let path = temp.path().join("race.txt");
        fs::write(&path, "safe").unwrap();
        let file = open_preview_file(&path).unwrap();
        fs::remove_file(&path).unwrap();
        symlink("/dev/null", &path).unwrap();

        let bytes = read_bounded(&file, 1024).unwrap();

        assert_eq!(bytes, b"safe");
    }

    #[cfg(unix)]
    #[test]
    fn fifo_preview_returns_without_waiting_for_a_writer() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};

        let temp = TempDir::new().unwrap();
        let path = temp.path().join("pipe.txt");
        let path_c = CString::new(path.as_os_str().as_bytes()).unwrap();
        let result = unsafe { libc::mkfifo(path_c.as_ptr(), 0o600) };
        if result != 0 {
            // Some CI sandboxes forbid creating special files; descriptor validation is still
            // covered by the /dev/null test above.
            return;
        }

        let preview = PreviewState::load(&path, false);

        assert!(matches!(preview.content, PreviewContent::Error { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn special_files_are_rejected_before_reading() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let path = temp.path().join("device.txt");
        symlink("/dev/null", &path).unwrap();

        let preview = PreviewState::load(&path, false);

        assert!(matches!(
            preview.content,
            PreviewContent::Error { ref message }
                if message.contains("non-regular")
        ));
    }

    #[test]
    fn directory_preview_caps_owned_entries_and_reports_omitted_count() {
        let temp = TempDir::new().unwrap();
        let total = MAX_DIRECTORY_PREVIEW_ENTRIES + 37;
        for index in 0..total {
            fs::write(temp.path().join(format!("entry-{index:04}")), []).unwrap();
        }

        let preview = PreviewState::load(temp.path(), false);
        let PreviewContent::Directory {
            entries,
            entries_truncated,
        } = &preview.content
        else {
            panic!("expected directory preview");
        };
        assert_eq!(entries.len(), MAX_DIRECTORY_PREVIEW_ENTRIES);
        assert!(*entries_truncated);

        let lines = render_lines(&preview, true, 80, 24, None, true);
        let rendered = lines
            .iter()
            .flat_map(|line| &line.spans)
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(rendered.contains("Entries: 512+"));
        assert!(rendered.contains("… more entries not shown"));
        assert!(lines.len() <= MAX_DIRECTORY_PREVIEW_ENTRIES + 6);
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

    #[test]
    fn terminal_image_output_has_a_hard_cell_bound_and_coalesces_runs() {
        let preview = PreviewState::default();
        let image = ImagePreview {
            name: "huge.png".to_string(),
            width: MAX_IMAGE_DIMENSION,
            height: MAX_IMAGE_DIMENSION,
            rgba: vec![255, 0, 0, 255],
        };

        let (width, height) = bounded_image_output_size(&image, 1.0);
        assert!(
            (width as usize).saturating_mul(height.div_ceil(2) as usize) <= MAX_IMAGE_RENDER_CELLS
        );

        let lines = render_image(&preview, &image, u16::MAX, u16::MAX);
        let image_lines = lines
            .iter()
            .filter(|line| line.spans.iter().any(|span| span.content.contains('▀')));
        let mut cells = 0usize;
        let mut spans = 0usize;
        for line in image_lines {
            cells += line.width();
            spans += line.spans.len();
        }
        assert!(cells <= MAX_IMAGE_RENDER_CELLS);
        assert!(spans <= cells);

        let solid = ImagePreview {
            name: "solid.png".to_string(),
            width: 64,
            height: 64,
            rgba: [255, 0, 0, 255].repeat(64 * 64),
        };
        let solid_lines = render_image(&preview, &solid, 64, 64);
        assert!(solid_lines
            .iter()
            .filter(|line| line.spans.iter().any(|span| span.content.contains('▀')))
            .all(|line| line.spans.len() == 1));
    }

    #[test]
    fn rgba_conversion_size_is_checked_explicitly() {
        assert_eq!(checked_rgba_len(1, 1), Some(4));
        assert_eq!(
            checked_rgba_len(MAX_IMAGE_DIMENSION, MAX_IMAGE_DIMENSION),
            Some(MAX_IMAGE_DECODE_ALLOC)
        );
    }

    #[test]
    fn clamps_vertical_and_horizontal_preview_offsets() {
        let mut preview = PreviewState {
            scroll: 99,
            horizontal_scroll: 99,
            ..PreviewState::default()
        };

        preview.set_measurements(8, 4, 10, 14);

        assert_eq!(preview.scroll, 6);
        assert_eq!(preview.horizontal_scroll, 6);
        preview.scroll_lines(-3);
        preview.scroll_columns(-4);
        assert_eq!(preview.scroll, 3);
        assert_eq!(preview.horizontal_scroll, 2);
    }

    #[test]
    fn unwindowed_model_offsets_reach_beyond_u16_for_large_previews() {
        let mut preview = PreviewState::default();
        preview.set_measurements(20, 10, 100_000, 100_000);
        preview.scroll_lines(90_000);
        preview.scroll_columns(90_000);

        assert_eq!(preview.scroll, 90_000);
        assert_eq!(preview.horizontal_scroll, 90_000);
        assert_eq!(preview.max_scroll(), 99_990);
        assert_eq!(preview.max_horizontal_scroll(), 99_980);
    }

    #[test]
    fn source_line_numbers_keep_unicode_content_and_align_gutter() {
        let preview = PreviewState {
            file_type: DetectedFileType::Source(Language::Rust),
            content: PreviewContent::Text {
                content: "界\nfn main() {}".to_string(),
                language: Language::Rust,
            },
            ..PreviewState::default()
        };

        let lines = render_lines(&preview, true, 80, 20, None, true);
        let text = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(text[0].starts_with("1 │ 界"));
        assert!(text[1].starts_with("2 │ fn main() {}"));
    }

    #[test]
    fn line_numbers_are_not_added_to_plain_text() {
        let preview = PreviewState {
            file_type: DetectedFileType::Text(Language::Text),
            content: PreviewContent::Text {
                content: "hello".to_string(),
                language: Language::Text,
            },
            ..PreviewState::default()
        };

        let lines = render_lines(&preview, true, 80, 20, None, true);
        assert_eq!(lines[0].spans[0].content, "hello");
    }

    #[test]
    fn narrow_image_preview_accounts_for_wrapped_metadata_rows() {
        let preview = PreviewState {
            size: Some(40_000),
            ..PreviewState::default()
        };
        let image = ImagePreview {
            name: "a-very-long-image-name.png".to_string(),
            width: 100,
            height: 100,
            rgba: vec![255; 100 * 100 * 4],
        };

        let lines = render_image(&preview, &image, 8, 8);

        assert!(lines
            .iter()
            .all(|line| line.spans.iter().all(|span| span.content != "▀")));
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

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
}
