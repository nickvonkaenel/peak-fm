use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::highlight::{HighlightedLine, Highlighter};
use super::image::{is_image, ImagePreview};
use super::pane::Pane;

const MAX_PREVIEW_LINES: usize = 100;
const MAX_LINE_LENGTH: usize = 500;

/// Check if a file is an audio file by extension
pub fn is_audio_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    matches!(
        ext.as_str(),
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" | "aiff" | "wma"
    )
}

#[derive(Debug, Clone)]
pub enum Preview {
    Directory(Pane),
    File(FilePreview),
    Image(ImagePreview),
    Error(String),
    None,
}

#[derive(Debug, Clone)]
pub struct FilePreview {
    pub path: PathBuf,
    pub lines: Vec<HighlightedLine>,
    pub is_binary: bool,
    pub error: Option<String>,
    pub scroll_offset: usize,
    pub highlight_line: Option<usize>, // Line to highlight (1-indexed)
    pub first_line_num: usize,         // First line number in the preview (1-indexed)
    pub fully_loaded: bool,            // True if entire file has been loaded
    pub lines_on_disk: usize,          // Number of lines loaded from disk so far
}

impl FilePreview {
    pub fn load(path: PathBuf, max_lines: usize) -> Self {
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                return Self {
                    path,
                    lines: Vec::new(),
                    is_binary: false,
                    error: Some(format!("Cannot open: {}", e)),
                    scroll_offset: 0,
                    highlight_line: None,
                    first_line_num: 1,
                    fully_loaded: true,
                    lines_on_disk: 0,
                };
            }
        };

        let reader = BufReader::new(file);
        let mut raw_lines = Vec::with_capacity(max_lines);
        let mut is_binary = false;

        for (i, line_result) in reader.lines().enumerate() {
            if i >= max_lines {
                break;
            }

            match line_result {
                Ok(line) => {
                    // Check for binary content (null bytes or too many control chars)
                    if line.bytes().any(|b| b == 0) {
                        is_binary = true;
                        break;
                    }

                    // Sanitize: replace tabs with spaces (4 spaces for proper alignment), filter control characters
                    let sanitized: String = line
                        .chars()
                        .flat_map(|c| {
                            if c == '\t' {
                                "    ".chars().collect::<Vec<_>>()
                            } else if c.is_control() {
                                vec![' ']
                            } else {
                                vec![c]
                            }
                        })
                        .collect();

                    // Truncate long lines (by character count, not bytes, for Unicode safety)
                    // Note: we add \n back because syntect's extra_newlines syntax set
                    // expects lines to include trailing newlines for proper state tracking
                    if sanitized.chars().count() > MAX_LINE_LENGTH {
                        let truncated: String = sanitized.chars().take(MAX_LINE_LENGTH).collect();
                        raw_lines.push(format!("{}...\n", truncated));
                    } else {
                        raw_lines.push(format!("{}\n", sanitized));
                    }
                }
                Err(_) => {
                    // Likely encoding error, treat as binary
                    is_binary = true;
                    break;
                }
            }
        }

        // Apply syntax highlighting if not binary
        let num_lines = raw_lines.len();
        let fully_loaded = num_lines < max_lines; // If we got fewer than requested, file is fully loaded

        let lines = if is_binary {
            Vec::new()
        } else {
            let highlighter = Highlighter::for_path(&path);
            highlighter.highlight_lines(&raw_lines)
        };

        Self {
            path,
            lines,
            is_binary,
            error: None,
            scroll_offset: 0,
            highlight_line: None,
            first_line_num: 1,
            fully_loaded,
            lines_on_disk: num_lines,
        }
    }

    /// Load preview centered around a specific line (for grep results)
    /// Note: This doesn't support lazy loading since it's meant to show context around a grep match
    pub fn load_around_line(path: PathBuf, target_line: usize, max_lines: usize) -> Self {
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                return Self {
                    path,
                    lines: Vec::new(),
                    is_binary: false,
                    error: Some(format!("Cannot open: {}", e)),
                    scroll_offset: 0,
                    highlight_line: None,
                    first_line_num: 1,
                    fully_loaded: true,
                    lines_on_disk: 0,
                };
            }
        };

        // Calculate start line so target appears centered in the visible area
        // Visible area is roughly max_lines/2, so put target at max_lines/4 from start
        let first_line_num = target_line.saturating_sub(max_lines / 4).max(1);

        let reader = BufReader::new(file);
        let mut raw_lines = Vec::with_capacity(max_lines);
        let mut is_binary = false;

        for (i, line_result) in reader.lines().enumerate() {
            let line_num = i + 1;

            // Skip lines before our window
            if line_num < first_line_num {
                continue;
            }

            // Stop after enough lines
            if raw_lines.len() >= max_lines {
                break;
            }

            match line_result {
                Ok(line) => {
                    if line.bytes().any(|b| b == 0) {
                        is_binary = true;
                        break;
                    }

                    let sanitized: String = line
                        .chars()
                        .flat_map(|c| {
                            if c == '\t' {
                                "    ".chars().collect::<Vec<_>>()
                            } else if c.is_control() {
                                vec![' ']
                            } else {
                                vec![c]
                            }
                        })
                        .collect();

                    if sanitized.chars().count() > MAX_LINE_LENGTH {
                        let truncated: String = sanitized.chars().take(MAX_LINE_LENGTH).collect();
                        raw_lines.push(format!("{}...\n", truncated));
                    } else {
                        raw_lines.push(format!("{}\n", sanitized));
                    }
                }
                Err(_) => {
                    is_binary = true;
                    break;
                }
            }
        }

        let lines = if is_binary {
            Vec::new()
        } else {
            let highlighter = Highlighter::for_path(&path);
            highlighter.highlight_lines(&raw_lines)
        };

        Self {
            path,
            lines,
            is_binary,
            error: None,
            scroll_offset: 0,
            highlight_line: Some(target_line),
            first_line_num,
            fully_loaded: true, // Grep context view doesn't support lazy loading
            lines_on_disk: raw_lines.len(),
        }
    }

    /// Load more lines from the file (for lazy loading during scroll)
    pub fn load_more(&mut self, additional_lines: usize) {
        if self.fully_loaded || self.is_binary || self.error.is_some() {
            return;
        }

        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => {
                self.fully_loaded = true;
                return;
            }
        };

        let reader = BufReader::new(file);
        let mut new_raw_lines = Vec::with_capacity(additional_lines);
        let skip_count = self.lines_on_disk;
        let mut actual_lines_read = 0;

        for (i, line_result) in reader.lines().enumerate() {
            // Skip lines we've already loaded
            if i < skip_count {
                continue;
            }

            // Stop after enough new lines
            if actual_lines_read >= additional_lines {
                break;
            }

            match line_result {
                Ok(line) => {
                    // Check for binary content
                    if line.bytes().any(|b| b == 0) {
                        self.is_binary = true;
                        self.fully_loaded = true;
                        return;
                    }

                    // Sanitize: replace tabs with spaces, filter control characters
                    let sanitized: String = line
                        .chars()
                        .flat_map(|c| {
                            if c == '\t' {
                                "    ".chars().collect::<Vec<_>>()
                            } else if c.is_control() {
                                vec![' ']
                            } else {
                                vec![c]
                            }
                        })
                        .collect();

                    // Truncate long lines
                    if sanitized.chars().count() > MAX_LINE_LENGTH {
                        let truncated: String = sanitized.chars().take(MAX_LINE_LENGTH).collect();
                        new_raw_lines.push(format!("{}...\n", truncated));
                    } else {
                        new_raw_lines.push(format!("{}\n", sanitized));
                    }
                    actual_lines_read += 1;
                }
                Err(_) => {
                    // Encoding error, treat as binary
                    self.is_binary = true;
                    self.fully_loaded = true;
                    return;
                }
            }
        }

        // Check if we reached end of file
        if actual_lines_read < additional_lines {
            self.fully_loaded = true;
        }

        if !new_raw_lines.is_empty() {
            // Apply syntax highlighting to new lines
            let highlighter = Highlighter::for_path(&self.path);
            let new_highlighted = highlighter.highlight_lines(&new_raw_lines);

            // Append to existing lines
            self.lines.extend(new_highlighted);
            self.lines_on_disk += actual_lines_read;
        }
    }
}

/// Load a preview, optionally skipping expensive image loading
pub fn load_preview(
    path: &PathBuf,
    height: usize,
    width: usize,
    show_hidden: bool,
    load_images: bool,
    sort_option: crate::core::SortOption,
) -> Preview {
    if path.is_dir() {
        match crate::fs::read_dir_filtered(path, show_hidden) {
            Ok(mut entries) => {
                sort_option.sort_entries(&mut entries);
                Preview::Directory(Pane::new(path.clone(), entries))
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                Preview::Error("Permission denied".to_string())
            }
            Err(_) => Preview::None,
        }
    } else if path.is_file() {
        // Check if it's an image
        if is_image(path) {
            if load_images {
                match ImagePreview::load(path, width as u16, height as u16) {
                    Ok(img) => return Preview::Image(img),
                    Err(_) => return Preview::None,
                }
            } else {
                // Return None - caller will load later after debounce
                return Preview::None;
            }
        }

        let max_lines = height.max(MAX_PREVIEW_LINES);
        Preview::File(FilePreview::load(path.clone(), max_lines))
    } else {
        Preview::None
    }
}

/// Check if a path is an image file
pub fn is_image_file(path: &PathBuf) -> bool {
    path.is_file() && is_image(path)
}
