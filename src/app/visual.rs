//! Visual mode - multi-line selection functionality

use crate::input::mode::{Mode, VisualEditType};

use super::App;

impl App {
    // Visual mode methods

    /// Get the selection range (start, end) from anchor and cursor
    pub fn visual_selection_range(&self, anchor: usize) -> (usize, usize) {
        let cursor = self.current.cursor;
        if anchor <= cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        }
    }

    pub fn enter_visual_mode(&mut self) {
        let anchor = self.current.cursor;
        self.mode = Mode::Visual { anchor };
        self.set_status("-- VISUAL --");
    }

    pub fn exit_visual_mode(&mut self) {
        self.mode = Mode::Normal;
        self.status.clear();
    }

    pub fn yank_visual_selection(&mut self, anchor: usize) {
        let (start, end) = self.visual_selection_range(anchor);

        self.yank.clear();
        for i in start..=end {
            if let Some(line) = self.current.buffer.lines.get(i) {
                if line.id.is_some() {
                    let path = self.current.buffer.path.join(&line.text);
                    self.yank.push((path, line.is_dir));
                }
            }
        }

        let count = self.yank.len();
        if count > 0 {
            self.set_status(format!("Yanked {} file(s)", count));
        }

        self.mode = Mode::Normal;
    }

    pub fn copy_visual_selection_to_clipboard(&mut self, anchor: usize) {
        let (start, end) = self.visual_selection_range(anchor);

        // Collect all file paths in the selection
        let mut paths: Vec<String> = Vec::new();
        for i in start..=end {
            if let Some(line) = self.current.buffer.lines.get(i) {
                if line.id.is_some() {
                    let path = self.current.buffer.path.join(&line.text);
                    if path.exists() {
                        // Get absolute path
                        let absolute_path = if path.is_absolute() {
                            path
                        } else {
                            std::env::current_dir()
                                .map(|cwd| cwd.join(&path))
                                .unwrap_or(path)
                        };
                        paths.push(absolute_path.to_string_lossy().to_string());
                    }
                }
            }
        }

        let count = paths.len();
        if count == 0 {
            self.set_status("No valid files to copy");
            self.mode = Mode::Normal;
            return;
        }

        // Copy to clipboard
        let result = super::clipboard::copy_paths_to_clipboard(&paths);
        match result {
            Ok(()) => {
                self.set_status(format!("Copied {} file(s) to clipboard", count));
            }
            Err(e) => {
                self.set_status(format!("Clipboard error: {}", e));
            }
        }

        self.mode = Mode::Normal;
    }

    pub fn copy_visual_selection_paths_to_clipboard(&mut self, anchor: usize) {
        let (start, end) = self.visual_selection_range(anchor);

        // Collect all absolute paths in the selection as text
        let mut paths: Vec<String> = Vec::new();
        for i in start..=end {
            if let Some(line) = self.current.buffer.lines.get(i) {
                if line.id.is_some() {
                    let path = self.current.buffer.path.join(&line.text);
                    // Get absolute path
                    let absolute_path = if path.is_absolute() {
                        path
                    } else {
                        std::env::current_dir()
                            .map(|cwd| cwd.join(&path))
                            .unwrap_or(path)
                    };
                    let path_str = absolute_path.to_string_lossy().to_string();
                    // Remove Windows extended-length path prefix \\?\
                    let path_str = path_str
                        .strip_prefix(r"\\?\")
                        .map(str::to_string)
                        .unwrap_or(path_str);
                    paths.push(path_str);
                }
            }
        }

        let count = paths.len();
        if count == 0 {
            self.set_status("No paths to copy");
            self.mode = Mode::Normal;
            return;
        }

        // Copy to clipboard
        let result = super::clipboard::copy_text_to_clipboard(&paths.join("\n"));
        match result {
            Ok(()) => {
                self.set_status(format!("Copied {} path(s) to clipboard", count));
            }
            Err(e) => {
                self.set_status(format!("Clipboard error: {}", e));
            }
        }

        self.mode = Mode::Normal;
    }

    pub fn delete_visual_selection(&mut self, anchor: usize) {
        self.save_undo_state();
        let (start, end) = self.visual_selection_range(anchor);

        // Yank all selected files before deleting
        self.yank.clear();
        self.yank_is_cut = true; // Mark as cut operation
        for i in start..=end {
            if let Some(line) = self.current.buffer.lines.get(i) {
                if line.id.is_some() {
                    let path = self.current.buffer.path.join(&line.text);
                    self.yank.push((path.clone(), line.is_dir));
                    self.marked_files.remove(&path);
                }
            }
        }

        // Delete lines from end to start (reverse order to maintain indices)
        let mut deleted = 0;
        for i in (start..=end).rev() {
            if i < self.current.buffer.lines.len() {
                self.current.buffer.delete_line(i);
                deleted += 1;
            }
        }

        // Adjust cursor if needed
        if self.current.cursor >= self.current.buffer.lines.len() {
            self.current.cursor = self.current.buffer.lines.len().saturating_sub(1);
        }

        self.mode = Mode::Normal;
        self.set_status(format!("Deleted {} line(s)", deleted));
        self.refresh_preview();
    }

    pub fn enter_visual_insert(&mut self, anchor: usize, edit_type: VisualEditType) {
        self.save_undo_state();
        self.visual_edit_text.clear();
        self.mode = Mode::VisualInsert { anchor, edit_type };

        let msg = match edit_type {
            VisualEditType::Start => "Insert at start (Enter to confirm)",
            VisualEditType::BeforeExt => "Insert before extension (Enter to confirm)",
            VisualEditType::End => "Insert at end (Enter to confirm)",
        };
        self.set_status(msg);
    }

    pub fn confirm_visual_insert(&mut self, anchor: usize, edit_type: VisualEditType) {
        let (start, end) = self.visual_selection_range(anchor);
        let text = self.visual_edit_text.clone();

        if text.is_empty() {
            self.set_status("No text entered");
            self.mode = Mode::Normal;
            return;
        }

        let mut renamed = 0;
        for i in start..=end {
            if let Some(line) = self.current.buffer.get_line_mut(i) {
                if line.id.is_some() {
                    let old_name = line.text.clone();
                    let new_name = match edit_type {
                        VisualEditType::Start => format!("{}{}", text, old_name),
                        VisualEditType::End => format!("{}{}", old_name, text),
                        VisualEditType::BeforeExt => {
                            // Insert before extension
                            if let Some(dot_pos) = old_name.rfind('.') {
                                let (base, ext) = old_name.split_at(dot_pos);
                                format!("{}{}{}", base, text, ext)
                            } else {
                                format!("{}{}", old_name, text)
                            }
                        }
                    };
                    line.text = new_name;
                    renamed += 1;
                }
            }
        }

        self.visual_edit_text.clear();
        self.mode = Mode::Normal;
        self.set_status(format!("Renamed {} file(s) - Ctrl+y to sync", renamed));
    }

    pub fn cancel_visual_insert(&mut self, anchor: usize) {
        self.visual_edit_text.clear();
        // Return to visual mode with the same anchor
        self.mode = Mode::Visual { anchor };
        self.set_status("-- VISUAL --");
    }

    pub fn copy_visual_selection_and_activate_reaper(&mut self, anchor: usize) {
        let (start, end) = self.visual_selection_range(anchor);

        // Collect all file paths in the selection
        let mut paths: Vec<String> = Vec::new();
        for i in start..=end {
            if let Some(line) = self.current.buffer.lines.get(i) {
                if line.id.is_some() {
                    let path = self.current.buffer.path.join(&line.text);
                    if path.exists() {
                        // Get absolute path
                        let absolute_path = if path.is_absolute() {
                            path
                        } else {
                            std::env::current_dir()
                                .map(|cwd| cwd.join(&path))
                                .unwrap_or(path)
                        };
                        paths.push(absolute_path.to_string_lossy().to_string());
                    }
                }
            }
        }

        let count = paths.len();
        if count == 0 {
            self.set_status("No valid files to copy");
            self.mode = Mode::Normal;
            return;
        }

        // Copy to clipboard
        let clipboard_result = super::clipboard::copy_paths_to_clipboard(&paths);
        if let Err(e) = clipboard_result {
            self.set_status(format!("Clipboard error: {}", e));
            self.mode = Mode::Normal;
            return;
        }

        // Activate Reaper
        let reaper_result = super::clipboard::activate_reaper();
        match reaper_result {
            Ok(()) => {
                self.set_status(format!("Copied {} file(s) & activated Reaper", count));
            }
            Err(e) => {
                self.set_status(format!("Reaper error: {}", e));
            }
        }

        self.mode = Mode::Normal;
    }
}
