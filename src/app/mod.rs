pub mod audio;
mod clipboard;
mod find;
mod grep;
pub mod handler;
mod media;
mod search;
#[cfg(test)]
mod tests;
mod visual;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};

use std::sync::mpsc::Receiver;

use crate::config::Config;
use crate::core::image::{load_image_async, ImageLoadResult};
use crate::core::player::AudioPlayer;
use crate::core::{
    available_themes, compute_diff, current_theme, load_preview, set_theme, theme_load_warnings,
    BufferLine, DisplayInfo, FsOperation, GlobalOperationStore, GrepMatch, GrepModeState, Pane,
    Preview, SearchEntry, SearchModeState, SortOption, DEFAULT_THEME,
};
use crate::fs;
use crate::input::{Mode, SearchDirection};

/// Action for neovim picker mode output
/// Position is (line, column) - both 1-indexed
#[derive(Debug, Clone)]
pub enum NvimAction {
    Edit(PathBuf, Option<(usize, usize)>), // Open in current buffer, optional (line, col)
    Split(PathBuf, Option<(usize, usize)>), // Open in horizontal split
    Vsplit(PathBuf, Option<(usize, usize)>), // Open in vertical split
    Tab(PathBuf, Option<(usize, usize)>),  // Open in new tab
}

/// Stored state for resuming the last search
#[derive(Clone)]
pub enum LastSearch {
    Find {
        search_root: PathBuf,
        query: String,
        selected: usize,
        scroll_offset: usize,
        files: Vec<SearchEntry>,
        show_hidden: bool,
        use_gitignore: bool,
        show_directories: bool,
    },
    Grep {
        search_root: PathBuf,
        query: String,
        selected: usize,
        scroll_offset: usize,
        matches: Vec<GrepMatch>,
        show_hidden: bool,
    },
}

pub struct App {
    pub parent: Option<Pane>,
    pub current: Pane,
    pub preview: Preview,
    pub cwd: PathBuf,
    pub _start_dir: PathBuf, // Original directory Peak File Manager was opened with
    pub prev_dir: PathBuf,   // Previous directory for `-` toggle
    pub work_dir: PathBuf,   // Working directory for searches (changeable with Ctrl+T)
    pub mode: Mode,
    pub should_quit: bool,
    pub status: String,
    pub status_time: Option<Instant>, // When status was set (for auto-clear)
    pub yank: Vec<(PathBuf, bool)>,   // Vec of (source_path, is_dir) for multi-file yank
    pub yank_is_cut: bool, // True if yank came from delete (cut), false if from yank (copy)
    pub preview_height: usize,
    pub preview_width: usize,
    pub needs_reinit: bool,
    pub search_query: String,
    pub command_input: String, // Shell command being typed in Command mode (`!`)
    pub search_direction: SearchDirection,
    pub show_hidden: bool,
    pub wrap_preview: bool,
    pub line_numbers: bool,
    pub show_icons: bool,
    pub colored_icons: bool,
    pub theme_icons: bool, // Use theme-mapped colors for icons
    pub find_state: Option<SearchModeState>,
    pub find_show_hidden: bool, // Independent hidden setting for Find mode
    pub find_use_gitignore: bool,
    pub find_show_directories: bool, // Whether to show directories in search (off by default)
    pub search_navigate_on_open: bool, // Navigate to file's directory when opening from search/grep
    pub zoxide_mode: bool,           // True when find_state contains zoxide directories
    pub grep_state: Option<GrepModeState>,
    pub last_search: Option<LastSearch>, // Stored state for resuming last search with 'r'
    pub visual_edit_text: String,        // Text being entered in visual insert mode
    pub pick_mode: bool,                 // Picker mode: quit on Esc or after opening
    pub nvim_mode: bool,                 // Neovim mode: output path instead of opening
    pub nvim_result: Option<NvimAction>, // Result to output when quitting in nvim mode
    pub search_lock_dir: Option<PathBuf>, // Lock search/grep to this directory
    pub audio_player: Option<AudioPlayer>,
    pub audio_auto_play: bool, // Auto-play mode: continue playing when switching audio files (audio browser mode)
    pub audio_normalize: bool, // Normalize waveform amplitude
    pub audio_skip_silence: bool, // Auto-skip silence during playback
    pub audio_volume: f32,     // Audio volume (0.0 to 4.0 linear, i.e. up to +12 dB, default 1.0)
    pub audio_analyzer_gradient: bool, // Analyzer gradient mode (colorful vs single color)
    pub audio_state: Option<audio::AudioModeState>, // Audio browser mode state
    pub normal_mode_autoplay: bool, // Auto-play audio in normal mode (not persistent, defaults to false)
    pub dir_cursors: HashMap<PathBuf, usize>, // Remember cursor positions per directory
    pub dir_sort_cache: HashMap<PathBuf, SortOption>, // Remember sort options per directory
    pub pending_image: Option<(PathBuf, Receiver<ImageLoadResult>)>, // Async image loading
    pub pending_git: Option<Receiver<String>>, // Async git operation result
    pub marked_files: HashSet<PathBuf>, // Files marked for batch operations
    pub global_operations: GlobalOperationStore, // Global pending filesystem operations
    pub undo_history: Vec<(Vec<BufferLine>, usize)>, // (lines, cursor) for undo
    pub redo_history: Vec<(Vec<BufferLine>, usize)>, // (lines, cursor) for redo
    pub sort_option: SortOption,
    pub display_info: DisplayInfo,
    pub _leader_time: Option<Instant>, // When leader key was pressed (reserved for future use)
    pub original_theme: Option<String>, // Original theme before entering theme selector (for preview)
    pub original_is_dir: bool,          // Was the line a directory when we entered insert mode?
}

/// Quote a string for the system shell so paths with spaces/specials survive.
#[cfg(not(windows))]
fn shell_quote(s: &str) -> String {
    // Single-quote and escape embedded single quotes as the classic '\'' .
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(windows)]
fn shell_quote(s: &str) -> String {
    // cmd.exe: wrap in double quotes, escaping any embedded ones.
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// Expand the `%f`/`%d`/`%n`/`%%` placeholders in a `!` command template.
/// `%f` = target file paths, `%n` = their base names (both shell-quoted and
/// space-joined), `%d` = the directory, `%%` = a literal `%`. An unknown `%x`
/// is left untouched.
fn expand_command(template: &str, files: &[PathBuf], dir: &Path) -> String {
    let join_quoted = |items: Vec<String>| {
        items
            .iter()
            .map(|s| shell_quote(s))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let files_str = || {
        join_quoted(
            files
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        )
    };
    let names_str = || {
        join_quoted(
            files
                .iter()
                .map(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                })
                .collect(),
        )
    };

    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('f') => {
                chars.next();
                out.push_str(&files_str());
            }
            Some('n') => {
                chars.next();
                out.push_str(&names_str());
            }
            Some('d') => {
                chars.next();
                out.push_str(&shell_quote(&dir.to_string_lossy()));
            }
            Some('%') => {
                chars.next();
                out.push('%');
            }
            // Unknown placeholder: leave the '%' as-is.
            _ => out.push('%'),
        }
    }
    out
}

impl App {
    pub fn new(
        start_path: PathBuf,
        pick_mode: bool,
        nvim_mode: bool,
        select: Option<String>,
        search_lock_dir: Option<PathBuf>,
    ) -> io::Result<Self> {
        let (config, mut startup_warnings) = Config::load_with_warnings();
        let show_hidden = config.show_hidden;
        let wrap_preview = config.wrap_preview;
        let line_numbers = config.line_numbers;
        let show_icons = config.show_icons;
        let colored_icons = config.colored_icons;
        let theme_icons = config.theme_icons;
        let search_navigate_on_open = config.search_navigate_on_open;
        let sort_option = config.sort_option;
        let dir_sort_cache = config.dir_sort_cache.clone();
        let audio_auto_play = config.audio_autoplay;
        let audio_normalize = config.audio_normalize;
        let audio_skip_silence = config.audio_skip_silence;
        let audio_volume = config.audio_volume;
        let audio_analyzer_gradient = config.audio_analyzer_gradient;

        // Set the theme from config. An unavailable personal theme falls back
        // safely while still surfacing a useful startup warning.
        if !set_theme(&config.theme) {
            let _ = set_theme(DEFAULT_THEME);
            startup_warnings.push(format!(
                "configured theme '{}' is unavailable",
                config.theme
            ));
        }
        startup_warnings.extend(theme_load_warnings());

        let cwd = start_path.canonicalize()?;
        let mut entries = fs::read_dir_filtered(&cwd, show_hidden)?;
        sort_option.sort_entries(&mut entries);
        let current = Pane::new(cwd.clone(), entries);

        let parent = if fs::is_at_root(&cwd) {
            // At filesystem root - show volumes in parent pane
            Some(Pane::new_volumes(fs::list_volumes()))
        } else {
            cwd.parent().and_then(|p| {
                fs::read_dir_filtered(p, show_hidden).ok().map(|mut e| {
                    sort_option.sort_entries(&mut e);
                    Pane::new(p.to_path_buf(), e)
                })
            })
        };

        let mut app = Self {
            parent,
            current,
            preview: Preview::None,
            _start_dir: cwd.clone(),
            prev_dir: cwd.clone(),
            work_dir: cwd.clone(),
            cwd,
            mode: Mode::Normal,
            should_quit: false,
            status: String::new(),
            status_time: None,
            yank: Vec::new(),
            yank_is_cut: false,
            preview_height: 20,
            preview_width: 40,
            needs_reinit: false,
            search_query: String::new(),
            command_input: String::new(),
            search_direction: SearchDirection::Forward,
            show_hidden,
            wrap_preview,
            line_numbers,
            show_icons,
            colored_icons,
            theme_icons,
            find_state: None,
            find_show_hidden: false, // Default to hiding hidden files in search
            find_use_gitignore: true, // Default to respecting .gitignore
            find_show_directories: false, // Default to hiding directories in search
            search_navigate_on_open,
            zoxide_mode: false,
            grep_state: None,
            last_search: None,
            visual_edit_text: String::new(),
            pick_mode,
            nvim_mode,
            nvim_result: None,
            search_lock_dir,
            // Open the platform audio device only when playback is requested.
            // This keeps startup and state-only tests out of native backends.
            audio_player: None,
            audio_auto_play,
            audio_normalize,
            audio_skip_silence,
            audio_volume,
            audio_analyzer_gradient,
            audio_state: None,
            normal_mode_autoplay: false,
            dir_cursors: HashMap::new(),
            dir_sort_cache,
            pending_image: None,
            pending_git: None,
            marked_files: HashSet::new(),
            global_operations: GlobalOperationStore::new(),
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            sort_option,
            display_info: DisplayInfo::default(),
            _leader_time: None,
            original_theme: None,
            original_is_dir: false,
        };

        // Pre-select a file if specified
        if let Some(filename) = select {
            app.select_by_name(&filename);
        }

        // Surface config and personal-theme problems instead of silently
        // falling back.
        if !startup_warnings.is_empty() {
            app.set_status(format!(
                "Startup: {} issue(s) — e.g. {}",
                startup_warnings.len(),
                startup_warnings[0]
            ));
        }

        app.refresh_preview();
        Ok(app)
    }

    /// Select a file by name in the current directory
    pub fn select_by_name(&mut self, name: &str) {
        for (i, line) in self.current.buffer.lines.iter().enumerate() {
            if line.text == name {
                self.current.set_cursor(i);
                break;
            }
        }
    }

    /// Save current buffer state to undo history (call before making changes)
    pub fn save_undo_state(&mut self) {
        let state = (self.current.buffer.lines.clone(), self.current.cursor);
        self.undo_history.push(state);
        // Clear redo history when new changes are made
        self.redo_history.clear();
        // Limit history size to prevent memory issues
        const MAX_HISTORY: usize = 100;
        if self.undo_history.len() > MAX_HISTORY {
            self.undo_history.remove(0);
        }
    }

    /// Undo the last change
    pub fn undo(&mut self) {
        if let Some((lines, cursor)) = self.undo_history.pop() {
            // Save current state to redo history
            let current_state = (self.current.buffer.lines.clone(), self.current.cursor);
            self.redo_history.push(current_state);

            // Restore previous state
            self.current.buffer.lines = lines;
            self.current.cursor = cursor.min(self.current.buffer.lines.len().saturating_sub(1));
            self.current.buffer.mark_dirty();
            self.set_status("Undo");
            self.refresh_preview();
        } else {
            self.set_status("Nothing to undo");
        }
    }

    /// Redo the last undone change
    pub fn redo(&mut self) {
        if let Some((lines, cursor)) = self.redo_history.pop() {
            // Save current state to undo history
            let current_state = (self.current.buffer.lines.clone(), self.current.cursor);
            self.undo_history.push(current_state);

            // Restore redo state
            self.current.buffer.lines = lines;
            self.current.cursor = cursor.min(self.current.buffer.lines.len().saturating_sub(1));
            self.current.buffer.mark_dirty();
            self.set_status("Redo");
            self.refresh_preview();
        } else {
            self.set_status("Nothing to redo");
        }
    }

    /// Clear undo/redo history (call when changing directories)
    pub fn clear_undo_history(&mut self) {
        self.undo_history.clear();
        self.redo_history.clear();
    }

    pub fn refresh_preview(&mut self) {
        // Stop any playing audio when selection changes
        self.stop_audio_silent();

        let show_hidden = self.show_hidden;
        let width = self.preview_width;
        let height = self.preview_height;

        if let Some(path) = self.current.selected_path() {
            // Auto-play audio if in auto-play mode (normal mode uses separate setting)
            if self.normal_mode_autoplay && crate::core::is_audio_file(&path) {
                self.play_audio_file(&path);
            }
            // Check if it's an image - load asynchronously with short wait
            if crate::core::preview::is_image_file(&path) {
                let rx = load_image_async(path.clone(), width as u16, height as u16);
                // Wait a short time for quick loads to avoid flicker
                match rx.recv_timeout(std::time::Duration::from_millis(16)) {
                    Ok(result) if result.path == path => {
                        // Loaded quickly, use immediately
                        self.pending_image = None;
                        if let Some(img) = result.image {
                            self.preview = Preview::Image(img);
                        } else {
                            self.preview = Preview::None;
                        }
                    }
                    Ok(_) => {
                        // Path mismatch (shouldn't happen), ignore
                        self.pending_image = None;
                        self.preview = Preview::None;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        // Still loading, continue in background
                        self.pending_image = Some((path, rx));
                        self.preview = Preview::None;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        // Error loading
                        self.pending_image = None;
                        self.preview = Preview::None;
                    }
                }
            } else {
                self.pending_image = None;
                let sort = if path.is_dir() {
                    self.get_sort_for_dir(&path)
                } else {
                    self.sort_option
                };
                self.preview = load_preview(&path, height, width, show_hidden, false, sort);
            }
        } else {
            self.pending_image = None;
            self.preview = Preview::None;
        }
    }

    /// Poll for async image loading completion
    pub fn check_pending_image(&mut self) {
        use std::sync::mpsc::TryRecvError;

        if let Some((ref expected_path, ref rx)) = self.pending_image {
            match rx.try_recv() {
                Ok(result) => {
                    // Only use the result if we're still looking at the same file
                    if &result.path == expected_path {
                        if let Some(img) = result.image {
                            self.preview = Preview::Image(img);
                        }
                    }
                    self.pending_image = None;
                }
                Err(TryRecvError::Empty) => {
                    // Still loading, keep waiting
                }
                Err(TryRecvError::Disconnected) => {
                    // Thread finished without sending (error case)
                    self.pending_image = None;
                }
            }
        }
    }

    /// Poll for async git operation completion
    pub fn check_pending_git(&mut self) {
        use std::sync::mpsc::TryRecvError;

        if let Some(ref rx) = self.pending_git {
            match rx.try_recv() {
                Ok(status_msg) => {
                    self.set_status(status_msg);
                    self.pending_git = None;
                }
                Err(TryRecvError::Empty) => {
                    // Still running, keep waiting
                }
                Err(TryRecvError::Disconnected) => {
                    // Thread finished without sending (error case)
                    self.pending_git = None;
                }
            }
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.status_time = Some(Instant::now());
    }

    pub fn clear_expired_status(&mut self) {
        // Don't auto-clear status in Confirm mode - wait for user response
        if matches!(self.mode, Mode::Confirm(_)) {
            return;
        }

        // Don't auto-clear status while audio is playing
        if self.is_audio_playing() {
            return;
        }

        if let Some(time) = self.status_time {
            if time.elapsed().as_secs() >= 2 {
                self.status.clear();
                self.status_time = None;
            }
        }
    }

    pub fn scroll_preview(&mut self, delta: isize) {
        match &mut self.preview {
            Preview::Directory(pane) => {
                pane.move_cursor(delta);
            }
            Preview::File(file_preview) => {
                let total_lines = file_preview.lines.len();
                if total_lines == 0 {
                    return;
                }
                if delta < 0 {
                    file_preview.scroll_offset =
                        file_preview.scroll_offset.saturating_sub((-delta) as usize);
                } else {
                    // Allow scrolling until last line is visible
                    let max_offset = total_lines.saturating_sub(1);
                    file_preview.scroll_offset =
                        (file_preview.scroll_offset + delta as usize).min(max_offset);

                    // Lazy loading: when approaching end of loaded content, load more
                    if !file_preview.fully_loaded {
                        let visible_end = file_preview.scroll_offset + self.preview_height;
                        let load_threshold = 20; // Load more when within 20 lines of end

                        if visible_end + load_threshold >= file_preview.lines.len() {
                            file_preview.load_more(200); // Load 200 more lines
                        }
                    }
                }
            }
            Preview::Image(_) => {}
            Preview::Error(_) => {}
            Preview::None => {}
        }
    }

    pub fn navigate_in(&mut self) -> io::Result<bool> {
        let Some(path) = self.current.selected_path() else {
            return Ok(false);
        };

        if path.is_dir() {
            // Capture current directory operations before navigating away
            self.capture_current_operations();

            // Try to read directory, handle permission errors gracefully
            let entries = match fs::read_dir_filtered(&path, self.show_hidden) {
                Ok(e) => e,
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    self.set_status(format!("Permission denied: {}", path.display()));
                    return Ok(false);
                }
                Err(e) => return Err(e),
            };

            // Save cursor position for current directory before leaving
            // (but not when in volumes view - that's not a real directory)
            if !self.current.buffer.is_volumes {
                self.dir_cursors
                    .insert(self.cwd.clone(), self.current.cursor);
            }

            let mut entries = entries;
            let sort = self.get_sort_for_dir(&path);
            sort.sort_entries(&mut entries);

            // Check if we're navigating from volumes view
            let from_volumes = self.current.buffer.is_volumes;

            // Current becomes parent
            let old_current =
                std::mem::replace(&mut self.current, Pane::new(path.clone(), entries));

            // When navigating from volumes view into a volume, show volumes as parent
            // since we're at the root of that volume
            self.parent = if from_volumes {
                let volumes = fs::list_volumes();
                // Position cursor on the volume we just entered
                let path_str = path.to_string_lossy();
                let cursor_pos = volumes.iter().position(|v| v.name == path_str).unwrap_or(0);
                let mut pane = Pane::new_volumes(volumes);
                pane.set_cursor(cursor_pos);
                Some(pane)
            } else {
                Some(old_current)
            };
            self.cwd = path.clone();

            // Restore operations for this directory if any exist
            self.restore_operations_to_buffer();

            // Restore cursor position if we've been here before
            if let Some(&saved_cursor) = self.dir_cursors.get(&path) {
                self.current.set_cursor(saved_cursor);
            }

            self.refresh_preview();
            Ok(false)
        } else if path.is_file() {
            // In nvim mode, output path and quit instead of opening
            if self.nvim_mode {
                self.nvim_pick(NvimAction::Edit(path, None));
                return Ok(false);
            }
            // Open file - returns true if terminal restore needed
            self.open_file(&path)
        } else {
            Ok(false)
        }
    }

    pub fn open_file(&mut self, path: &PathBuf) -> io::Result<bool> {
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let is_csv = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase() == "csv")
            .unwrap_or(false);

        if is_csv {
            ratatui::restore();
            let _ = execute!(io::stdout(), Clear(ClearType::All));
            let status = Command::new("csvlens")
                .arg("--ignore-case")
                .arg(path)
                .status();
            if self.pick_mode {
                self.should_quit = true;
                return Ok(true);
            }
            self.needs_reinit = true;
            return match status {
                Ok(_) => Ok(true),
                Err(e) => {
                    self.set_status(format!("Error opening {}: {}", filename, e));
                    Ok(true)
                }
            };
        }

        if Self::is_text_file(path) {
            // Restore terminal before opening editor
            ratatui::restore();
            // Clear screen for clean handoff to external program (fixes Windows artifacts)
            let _ = execute!(io::stdout(), Clear(ClearType::All));

            // Helper to strip Windows extended-length path prefix
            #[cfg(target_os = "windows")]
            fn strip_windows_prefix(path: &str) -> String {
                path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
            }
            #[cfg(not(target_os = "windows"))]
            fn strip_windows_prefix(path: &str) -> String {
                path.to_string()
            }

            // Make path relative to work_dir if possible (helps LSP find project root)
            let path_str = if let Ok(relative) = path.strip_prefix(&self.work_dir) {
                relative.to_string_lossy().to_string()
            } else {
                strip_windows_prefix(&path.to_string_lossy())
            };

            // Open in neovim with work_dir as working directory (set by zoxide or Ctrl+T)
            // Use --cmd to set directory before file loads (important for LSP)
            let work_dir_str = strip_windows_prefix(&self.work_dir.to_string_lossy());
            let status = Command::new("nvim")
                .arg("--cmd")
                .arg(format!("cd {}", work_dir_str))
                .arg(&path_str)
                .current_dir(&self.work_dir)
                .status();

            // In pick mode, quit after editor closes
            if self.pick_mode {
                self.should_quit = true;
                return Ok(true);
            }

            // Signal that terminal needs to be reinitialized
            self.needs_reinit = true;

            match status {
                Ok(_) => Ok(true),
                Err(e) => {
                    self.set_status(format!("Error opening {}: {}", filename, e));
                    Ok(true) // Still need reinit
                }
            }
        } else {
            // Open with system default application
            #[cfg(target_os = "macos")]
            let result = Command::new("open")
                .arg(path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            #[cfg(target_os = "linux")]
            let result = Command::new("xdg-open")
                .arg(path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            #[cfg(target_os = "windows")]
            let result = Command::new("cmd")
                .args(["/C", "start", ""])
                .arg(path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            // In pick mode, quit after opening
            if self.pick_mode {
                self.should_quit = true;
                return Ok(false);
            }

            match result {
                Ok(exit_status) => {
                    if exit_status.success() {
                        self.set_status(format!("Opened: {}", filename));
                    } else {
                        self.set_status(format!("No handler for: {}", filename));
                    }
                }
                Err(e) => {
                    self.set_status(format!("Error opening {}: {}", filename, e));
                }
            }
            Ok(false) // No terminal reinit needed
        }
    }

    fn is_text_file(path: &Path) -> bool {
        let Ok(mut file) = File::open(path) else {
            return false;
        };
        let mut buffer = [0_u8; 8192];
        let Ok(bytes_read) = file.read(&mut buffer) else {
            return false;
        };
        let sample = &buffer[..bytes_read];
        if sample.is_empty() {
            return true;
        }
        if sample.contains(&0) || std::str::from_utf8(sample).is_err() {
            return false;
        }

        let suspicious_controls = sample
            .iter()
            .filter(|&&byte| byte < 0x20 && !matches!(byte, b'\n' | b'\r' | b'\t' | 0x0c))
            .count();
        suspicious_controls.saturating_mul(100) <= sample.len()
    }

    /// Pick a file for nvim mode output
    pub fn nvim_pick(&mut self, action: NvimAction) {
        self.nvim_result = Some(action);
        self.should_quit = true;
    }

    /// Pick current file for edit in nvim
    #[allow(dead_code)]
    pub fn nvim_edit(&mut self) {
        if let Some(path) = self.current.selected_path() {
            if path.is_file() {
                self.nvim_pick(NvimAction::Edit(path, None));
            }
        }
    }

    /// Pick current file for horizontal split in nvim
    pub fn nvim_split(&mut self) {
        if let Some(path) = self.current.selected_path() {
            if path.is_file() {
                self.nvim_pick(NvimAction::Split(path, None));
            }
        }
    }

    /// Pick current file for vertical split in nvim
    pub fn nvim_vsplit(&mut self) {
        if let Some(path) = self.current.selected_path() {
            if path.is_file() {
                self.nvim_pick(NvimAction::Vsplit(path, None));
            }
        }
    }

    /// Pick current file for new tab in nvim
    pub fn nvim_tab(&mut self) {
        if let Some(path) = self.current.selected_path() {
            if path.is_file() {
                self.nvim_pick(NvimAction::Tab(path, None));
            }
        }
    }

    pub fn navigate_to(&mut self, path: PathBuf) -> io::Result<()> {
        if !path.is_dir() || path == self.cwd {
            return Ok(());
        }

        // Capture current directory operations before navigating away
        self.capture_current_operations();

        // Clear undo history when changing directories
        self.clear_undo_history();

        let mut entries = fs::read_dir_filtered(&path, self.show_hidden)?;
        let sort = self.get_sort_for_dir(&path);
        sort.sort_entries(&mut entries);
        self.current = Pane::new(path.clone(), entries);
        self.cwd = path.clone();

        // Restore operations for this directory if any exist
        self.restore_operations_to_buffer();

        // Set parent pane - show volumes if at root
        self.parent = if fs::is_at_root(&path) {
            Some(Pane::new_volumes(fs::list_volumes()))
        } else {
            path.parent().and_then(|p| {
                fs::read_dir_filtered(p, self.show_hidden)
                    .ok()
                    .map(|mut e| {
                        let sort = self.get_sort_for_dir(p);
                        sort.sort_entries(&mut e);
                        Pane::new(p.to_path_buf(), e)
                    })
            })
        };

        self.refresh_preview();
        Ok(())
    }

    /// Refresh the current directory (reload file list)
    pub fn refresh_current_dir(&mut self) -> io::Result<()> {
        let mut entries = fs::read_dir_filtered(&self.cwd, self.show_hidden)?;
        let sort = self.get_sort_for_dir(&self.cwd);
        sort.sort_entries(&mut entries);
        let cursor = self.current.cursor.min(entries.len().saturating_sub(1));
        self.current = Pane::new(self.cwd.clone(), entries);
        self.current.set_cursor(cursor);
        self.refresh_preview();
        Ok(())
    }

    /// Toggle between current directory and previous directory
    pub fn toggle_prev_dir(&mut self) -> io::Result<()> {
        if self.prev_dir == self.cwd {
            return Ok(());
        }
        let target = self.prev_dir.clone();
        self.prev_dir = self.cwd.clone();
        self.navigate_to(target)
    }

    pub fn navigate_to_trash(&mut self) -> io::Result<()> {
        match fs::trash_dir() {
            Ok(trash_path) => {
                self.prev_dir = self.cwd.clone();
                self.navigate_to(trash_path)?;
                self.set_status("Trash — press x to restore, X to restore marked");
                Ok(())
            }
            Err(e) => {
                self.set_status(format!("Cannot open trash: {}", e));
                Ok(())
            }
        }
    }

    /// True when the current directory is the trash directory.
    pub fn in_trash(&self) -> bool {
        fs::trash_dir().map(|t| t == self.cwd).unwrap_or(false)
    }

    /// Restore the selected trashed entry to its original location, or into the
    /// directory we came from when the origin is unknown.
    pub fn restore_selected_from_trash(&mut self) {
        if !self.in_trash() {
            self.set_status("Not in trash");
            return;
        }
        let entry = match self.current.selected_path() {
            Some(p) => p,
            None => return,
        };
        let fallback = self.prev_dir.clone();
        match fs::restore_from_trash(&entry, &fallback) {
            Ok(dest) => {
                self.marked_files.remove(&entry);
                let _ = self.refresh_current_dir();
                let name = dest
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                self.set_status(format!("Restored: {}", name));
            }
            Err(e) => self.set_status(format!("Restore failed: {}", e)),
        }
    }

    /// Restore all marked trashed entries. Falls back to the selected entry
    /// when nothing is marked.
    pub fn restore_marked_from_trash(&mut self) {
        if !self.in_trash() {
            self.set_status("Not in trash");
            return;
        }
        if self.marked_files.is_empty() {
            self.restore_selected_from_trash();
            return;
        }
        let fallback = self.prev_dir.clone();
        let entries: Vec<PathBuf> = self.marked_files.iter().cloned().collect();
        let mut restored = 0;
        let mut failed = 0;
        for entry in entries {
            match fs::restore_from_trash(&entry, &fallback) {
                Ok(_) => restored += 1,
                Err(_) => failed += 1,
            }
        }
        self.clear_marks();
        let _ = self.refresh_current_dir();
        if failed > 0 {
            self.set_status(format!("Restored {} file(s), {} failed", restored, failed));
        } else {
            self.set_status(format!("Restored {} file(s)", restored));
        }
    }

    pub fn toggle_trash(&mut self) -> io::Result<()> {
        // Check if we're currently in trash
        if let Ok(trash_path) = fs::trash_dir() {
            if self.cwd == trash_path {
                // In trash - go back to previous directory
                let target = self.prev_dir.clone();
                self.prev_dir = self.cwd.clone();
                return self.navigate_to(target);
            }
        }
        // Not in trash - navigate to trash
        self.navigate_to_trash()
    }

    /// Set the work directory to the current directory (for Ctrl+T)
    pub fn set_work_dir_to_cwd(&mut self) {
        self.work_dir = self.cwd.clone();
        let name = self.cwd.file_name().unwrap_or_default().to_string_lossy();
        self.set_status(format!("work dir: {}", name));
    }

    pub fn navigate_out(&mut self) -> io::Result<()> {
        // Capture current directory operations before navigating away
        self.capture_current_operations();

        // At root - switch to volumes view
        if fs::is_at_root(&self.cwd) {
            // If already showing volumes, do nothing
            if self.current.buffer.is_volumes {
                return Ok(());
            }

            // Save cursor position before switching to volumes view
            self.dir_cursors
                .insert(self.cwd.clone(), self.current.cursor);

            // Switch current pane to volumes view
            let volumes = fs::list_volumes();

            // Find cursor position for the current volume (cwd)
            let cwd_str = self.cwd.to_string_lossy();
            let cursor_pos = volumes.iter().position(|v| v.name == cwd_str).unwrap_or(0);

            let mut pane = Pane::new_volumes(volumes);
            pane.set_cursor(cursor_pos);
            self.current = pane;
            self.parent = None;
            self.refresh_preview();
            return Ok(());
        }

        let Some(parent_path) = self.cwd.parent().map(|p| p.to_path_buf()) else {
            return Ok(());
        };

        // Clear undo history when changing directories
        self.clear_undo_history();

        // Save cursor position for current directory before leaving
        self.dir_cursors
            .insert(self.cwd.clone(), self.current.cursor);

        let mut entries = fs::read_dir_filtered(&parent_path, self.show_hidden)?;
        let sort = self.get_sort_for_dir(&parent_path);
        sort.sort_entries(&mut entries);

        // Use saved cursor position if available, otherwise find old cwd in parent
        let cursor_pos = self
            .dir_cursors
            .get(&parent_path)
            .copied()
            .unwrap_or_else(|| {
                let old_cwd_name = self.cwd.file_name().and_then(|n| n.to_str());
                entries
                    .iter()
                    .position(|e| Some(e.name.as_str()) == old_cwd_name)
                    .unwrap_or(0)
            });

        let mut new_current = Pane::new(parent_path.clone(), entries);
        new_current.set_cursor(cursor_pos);

        self.current = new_current;
        self.cwd = parent_path.clone();

        // Restore operations for this directory if any exist
        self.restore_operations_to_buffer();

        // Set parent pane - show volumes if now at root
        self.parent = if fs::is_at_root(&parent_path) {
            Some(Pane::new_volumes(fs::list_volumes()))
        } else {
            parent_path.parent().and_then(|p| {
                fs::read_dir_filtered(p, self.show_hidden)
                    .ok()
                    .map(|mut e| {
                        let sort = self.get_sort_for_dir(p);
                        sort.sort_entries(&mut e);
                        Pane::new(p.to_path_buf(), e)
                    })
            })
        };

        self.refresh_preview();
        Ok(())
    }

    pub fn pending_ops(&self) -> Vec<FsOperation> {
        compute_diff(&self.current.buffer)
    }

    /// Capture current directory operations before navigating away
    pub fn capture_current_operations(&mut self) {
        if self.is_dirty() {
            self.global_operations
                .capture_from_buffer(&self.cwd, &self.current.buffer);
        }
    }

    /// Restore operations for current directory if any exist
    fn restore_operations_to_buffer(&mut self) {
        if self.global_operations.count_for_dir(&self.cwd) > 0 {
            self.global_operations
                .restore_to_buffer(&self.cwd, &mut self.current.buffer);
        }
    }

    pub fn sync(&mut self) -> io::Result<()> {
        // 1. Capture current directory operations first
        self.capture_current_operations();

        // 2. Get ALL operations globally
        let ops = self.global_operations.all_operations();

        if ops.is_empty() {
            self.set_status("No changes to sync");
            return Ok(());
        }

        // 3. Validate all operations (including cross-directory conflicts)
        if let Err(e) = fs::validate_global_operations(&ops) {
            self.set_status(format!("Validation error: {}", e));
            return Err(io::Error::new(io::ErrorKind::InvalidInput, e));
        }

        // 4. Apply operations to filesystem
        if let Err(e) = fs::apply_operations(&ops) {
            self.set_status(format!("Sync error: {}", e));
            return Err(e);
        }

        // 4. Clear global operations and undo/redo history after successful sync
        // (undo history holds pre-sync buffer states that no longer match disk reality)
        self.global_operations.clear();
        self.clear_undo_history();

        // 5. Refresh current directory while preserving selection
        let selected_path = self.current.selected_path();
        let mut entries = fs::read_dir_filtered(&self.cwd, self.show_hidden)?;
        let sort = self.get_sort_for_dir(&self.cwd);
        sort.sort_entries(&mut entries);
        self.current = Pane::new(self.cwd.clone(), entries);

        // Try to restore selection to the same file/directory
        if let Some(path) = selected_path {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                for (i, line) in self.current.buffer.lines.iter().enumerate() {
                    if line.text == name {
                        self.current.cursor = i;
                        break;
                    }
                }
            }
        }

        // 6. Refresh parent pane
        self.parent = if fs::is_at_root(&self.cwd) {
            Some(Pane::new_volumes(fs::list_volumes()))
        } else {
            self.cwd.parent().and_then(|p| {
                fs::read_dir_filtered(p, self.show_hidden)
                    .ok()
                    .map(|mut e| {
                        let sort = self.get_sort_for_dir(p);
                        sort.sort_entries(&mut e);
                        Pane::new(p.to_path_buf(), e)
                    })
            })
        };

        // 7. Force refresh preview to ensure directory contents are updated
        self.refresh_preview();
        self.set_status(format!("Synced {} operation(s)", ops.len()));

        Ok(())
    }

    pub fn delete_line(&mut self) {
        self.save_undo_state();
        // Yank before deleting
        if let Some(line) = self.current.selected_line() {
            if line.id.is_some() {
                let path = self.current.buffer.path.join(&line.text);
                self.yank = vec![(path.clone(), line.is_dir)];
                self.yank_is_cut = true; // Mark as cut operation
                self.marked_files.remove(&path);
            }
        }
        self.current.delete_selected();
    }

    pub fn insert_line_below(&mut self) {
        self.save_undo_state();
        self.current.insert_below();
        self.original_is_dir = false; // New line, not an original directory
        self.mode = Mode::Insert;
        self.current.buffer.edit_cursor = 0;
    }

    pub fn insert_line_above(&mut self) {
        self.save_undo_state();
        self.current.insert_above();
        self.original_is_dir = false; // New line, not an original directory
        self.mode = Mode::Insert;
        self.current.buffer.edit_cursor = 0;
    }

    pub fn enter_insert_mode_start(&mut self) {
        self.save_undo_state();
        let cursor = self.current.cursor;

        // Capture original directory state
        if let Some(line) = self.current.buffer.get_line(cursor) {
            self.original_is_dir = line.is_dir;
        } else {
            self.original_is_dir = false;
        }

        // i/I - cursor at start
        self.current.buffer.edit_cursor = 0;
        self.mode = Mode::Insert;
    }

    pub fn enter_insert_mode_before_ext(&mut self) {
        self.save_undo_state();
        let cursor = self.current.cursor;

        // Capture original directory state
        if let Some(line) = self.current.buffer.get_line(cursor) {
            self.original_is_dir = line.is_dir;
        } else {
            self.original_is_dir = false;
        }

        // a - cursor before file extension
        let cursor_pos = self
            .current
            .buffer
            .get_line(cursor)
            .map(|line| {
                // Find position before extension (last dot, if any)
                if let Some(dot_pos) = line.text.rfind('.') {
                    // Don't count hidden files (starting with .)
                    if dot_pos > 0 {
                        return dot_pos;
                    }
                }
                line.text.len()
            })
            .unwrap_or(0);
        self.current.buffer.edit_cursor = cursor_pos;
        self.mode = Mode::Insert;
    }

    pub fn enter_insert_mode_end(&mut self) {
        self.save_undo_state();
        let cursor = self.current.cursor;

        // Capture original directory state
        if let Some(line) = self.current.buffer.get_line(cursor) {
            self.original_is_dir = line.is_dir;
        } else {
            self.original_is_dir = false;
        }

        // A - cursor at end
        let cursor_pos = self
            .current
            .buffer
            .get_line(cursor)
            .map(|line| line.text.len())
            .unwrap_or(0);
        self.current.buffer.edit_cursor = cursor_pos;
        self.mode = Mode::Insert;
    }

    pub fn enter_insert_mode_clear(&mut self) {
        self.save_undo_state();

        // Since we're clearing and creating new, it's not an original directory
        self.original_is_dir = false;

        // c - clear line and enter insert mode
        let cursor = self.current.cursor;
        if let Some(line) = self.current.buffer.get_line_mut(cursor) {
            line.text.clear();
            line.is_dir = false;
        }
        self.current.buffer.edit_cursor = 0;
        self.current.buffer.mark_dirty();
        self.mode = Mode::Insert;
    }

    pub fn insert_char(&mut self, c: char) {
        let cursor = self.current.cursor;
        let edit_cursor = self.current.buffer.edit_cursor;

        if let Some(line) = self.current.buffer.get_line_mut(cursor) {
            // Prevent adding "/" at the end if this was originally a directory
            // and it already ends with "/" (would create "dirname//")
            let is_end_position = edit_cursor >= line.text.len();
            let would_add_trailing_slash = c == '/' && is_end_position;
            let already_has_slash = line.text.ends_with('/');

            if self.original_is_dir && would_add_trailing_slash && already_has_slash {
                // Don't insert - would create duplicate slash
                return;
            }

            let pos = edit_cursor.min(line.text.len());
            line.text.insert(pos, c);

            // Update is_dir dynamically based on trailing slash
            line.is_dir = line.text.ends_with('/');

            self.current.buffer.edit_cursor = edit_cursor + 1;
            self.current.buffer.mark_dirty();
        }
    }

    pub fn delete_char(&mut self) {
        let cursor = self.current.cursor;
        let edit_cursor = self.current.buffer.edit_cursor;

        if edit_cursor > 0 {
            let pos = edit_cursor - 1;
            if let Some(line) = self.current.buffer.get_line_mut(cursor) {
                if pos < line.text.len() {
                    // For original directories, protect the trailing slash
                    let is_trailing_slash = pos == line.text.len() - 1 && line.text.ends_with('/');

                    if self.original_is_dir && is_trailing_slash {
                        // Don't delete the trailing slash of original directories
                        return;
                    }

                    line.text.remove(pos);
                    // Update is_dir based on trailing slash
                    line.is_dir = line.text.ends_with('/');
                    self.current.buffer.edit_cursor = pos;
                    self.current.buffer.mark_dirty();
                }
            }
        }
    }

    pub fn move_edit_cursor(&mut self, delta: isize) {
        let len = self
            .current
            .buffer
            .get_line(self.current.cursor)
            .map(|line| line.text.len())
            .unwrap_or(0);

        let cur = self.current.buffer.edit_cursor;
        if delta < 0 {
            self.current.buffer.edit_cursor = cur.saturating_sub((-delta) as usize);
        } else {
            self.current.buffer.edit_cursor = (cur + delta as usize).min(len);
        }
    }

    pub fn move_edit_cursor_to(&mut self, pos: usize) {
        let len = self
            .current
            .buffer
            .get_line(self.current.cursor)
            .map(|line| line.text.len())
            .unwrap_or(0);
        self.current.buffer.edit_cursor = pos.min(len);
    }

    pub fn move_edit_cursor_to_end(&mut self) {
        let len = self
            .current
            .buffer
            .get_line(self.current.cursor)
            .map(|line| line.text.len())
            .unwrap_or(0);
        self.current.buffer.edit_cursor = len;
    }

    pub fn delete_word(&mut self) {
        let cursor = self.current.cursor;
        let edit_cursor = self.current.buffer.edit_cursor;

        if edit_cursor == 0 {
            return;
        }

        if let Some(line) = self.current.buffer.get_line_mut(cursor) {
            // Find start of word (skip trailing spaces, then skip word chars)
            let text = &line.text[..edit_cursor];
            let trimmed_end = text.trim_end_matches(' ').len();
            let word_start = text[..trimmed_end]
                .rfind([' ', '/', '.'])
                .map(|i| i + 1)
                .unwrap_or(0);

            // For original directories, stop before the trailing slash
            let final_word_start = if self.original_is_dir && line.text.ends_with('/') {
                word_start.max(line.text.len() - 1)
            } else {
                word_start
            };

            line.text.drain(final_word_start..edit_cursor);
            line.is_dir = line.text.ends_with('/');
            self.current.buffer.edit_cursor = final_word_start;
            self.current.buffer.mark_dirty();
        }
    }

    pub fn clear_line(&mut self) {
        let cursor = self.current.cursor;
        if let Some(line) = self.current.buffer.get_line_mut(cursor) {
            if self.original_is_dir {
                // For original directories, leave just the trailing slash
                line.text = "/".to_string();
                line.is_dir = true;
                self.current.buffer.edit_cursor = 0;
            } else {
                line.text.clear();
                line.is_dir = false;
                self.current.buffer.edit_cursor = 0;
            }
            self.current.buffer.mark_dirty();
        }
    }

    pub fn delete_to_end(&mut self) {
        let cursor = self.current.cursor;
        let edit_cursor = self.current.buffer.edit_cursor;

        if let Some(line) = self.current.buffer.get_line_mut(cursor) {
            line.text.truncate(edit_cursor);

            // For original directories, ensure the trailing slash is preserved
            if self.original_is_dir && !line.text.ends_with('/') {
                line.text.push('/');
            }

            line.is_dir = line.text.ends_with('/');
            self.current.buffer.mark_dirty();
        }
    }

    pub fn yank_selected(&mut self) {
        if let Some(line) = self.current.selected_line() {
            if line.id.is_some() {
                let path = self.current.buffer.path.join(&line.text);
                self.yank = vec![(path, line.is_dir)];
                self.yank_is_cut = false; // Mark as copy operation
                self.set_status(format!("Yanked: {}", line.text));
            }
        }
    }

    /// Yank all marked files
    pub fn yank_marked(&mut self) {
        if self.marked_files.is_empty() {
            self.set_status("No files marked");
            return;
        }

        self.yank.clear();
        for path in &self.marked_files {
            let is_dir = path.is_dir();
            self.yank.push((path.clone(), is_dir));
        }

        self.yank_is_cut = false; // Mark as copy operation
        let count = self.yank.len();
        self.set_status(format!("Yanked {} file(s)", count));
        self.clear_marks();
    }

    pub fn paste(&mut self) {
        if self.yank.is_empty() {
            return;
        }

        self.save_undo_state();
        let mut idx = self.current.cursor + 1;
        let count = self.yank.len();
        let mut restored = 0;
        let mut copied = 0;

        for (src_path, is_dir) in &self.yank {
            if let Some(name) = src_path.file_name().and_then(|n| n.to_str()) {
                // Check if this file was deleted from the current directory
                // by looking for it in the snapshot but not in current lines
                let is_from_current_dir =
                    src_path.parent() == Some(self.current.buffer.path.as_path());

                let restored_line = if is_from_current_dir {
                    // Find the original line in snapshot
                    self.current
                        .buffer
                        .snapshot
                        .iter()
                        .find(|snap_line| snap_line.text == name && snap_line.id.is_some())
                        .and_then(|snap_line| {
                            // Check if this ID is not already in current lines (i.e., it was deleted)
                            let id = snap_line.id?;
                            let already_exists =
                                self.current.buffer.lines.iter().any(|l| l.id == Some(id));
                            if already_exists {
                                None
                            } else {
                                Some(snap_line.clone())
                            }
                        })
                } else {
                    None
                };

                if let Some(line) = restored_line {
                    // Restore the original line (undo the delete)
                    self.current.buffer.insert_line(idx, line);
                    restored += 1;
                } else {
                    // Create a copy or move operation
                    let name = name.to_string();
                    let new_line = if self.yank_is_cut {
                        BufferLine::new_move(name, *is_dir, src_path.clone())
                    } else {
                        BufferLine::new_copy(name, *is_dir, src_path.clone())
                    };
                    self.current.buffer.insert_line(idx, new_line);
                    copied += 1;
                }
                idx += 1;
            }
        }

        // Move cursor to first pasted item, clamped to buffer bounds
        let new_cursor = self.current.cursor + 1;
        self.current.cursor = new_cursor.min(self.current.buffer.lines.len().saturating_sub(1));

        // Set appropriate status message
        let operation = if self.yank_is_cut { "move" } else { "copy" };
        if count == 1 {
            let name = self.yank[0]
                .0
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            if restored == 1 {
                self.set_status(format!("Restored: {}", name));
            } else {
                self.set_status(format!("Pasted: {} (will {} on sync)", name, operation));
            }
        } else if restored > 0 && copied > 0 {
            self.set_status(format!(
                "Restored {} file(s), will {} {} file(s)",
                restored, operation, copied
            ));
        } else if restored > 0 {
            self.set_status(format!("Restored {} file(s)", restored));
        } else {
            self.set_status(format!(
                "Pasted {} file(s) (will {} on sync)",
                copied, operation
            ));
        }
    }

    pub fn is_dirty(&self) -> bool {
        !self.pending_ops().is_empty()
    }

    pub fn remove_empty_lines(&mut self) {
        // Remove empty lines that have no ID (newly created but left blank)
        self.current.buffer.lines.retain(|line| {
            // Keep lines that have an ID (existing files) or have non-empty text
            line.id.is_some() || !line.text.is_empty()
        });

        // Adjust cursor if needed
        let len = self.current.buffer.len();
        if len == 0 {
            self.current.cursor = 0;
        } else if self.current.cursor >= len {
            self.current.cursor = len - 1;
        }
    }

    pub fn exit_insert_mode(&mut self) {
        self.restore_empty_lines();
        self.remove_empty_lines();
        self.original_is_dir = false; // Reset state
        self.mode = Mode::Normal;
    }

    fn restore_empty_lines(&mut self) {
        // Restore empty lines that have an ID back to their original name
        for line in &mut self.current.buffer.lines {
            if line.text.is_empty() {
                if let Some(id) = line.id {
                    // Find original name from snapshot
                    if let Some(original) = self
                        .current
                        .buffer
                        .snapshot
                        .iter()
                        .find(|s| s.id == Some(id))
                    {
                        line.text = original.text.clone();
                        line.is_dir = original.is_dir;
                    }
                }
            }
        }
    }

    pub fn toggle_hidden(&mut self) -> io::Result<()> {
        self.show_hidden = !self.show_hidden;

        // Save setting
        let mut config = Config::load();
        config.show_hidden = self.show_hidden;
        config.save();

        // Refresh current directory
        let mut entries = fs::read_dir_filtered(&self.cwd, self.show_hidden)?;
        let sort = self.get_sort_for_dir(&self.cwd);
        sort.sort_entries(&mut entries);

        // Try to preserve cursor position on same file
        let current_name = self.current.selected_line().map(|l| l.text.clone());

        self.current = Pane::new(self.cwd.clone(), entries);

        // Restore cursor position if possible
        if let Some(name) = current_name {
            if let Some(pos) = self
                .current
                .buffer
                .lines
                .iter()
                .position(|l| l.text == name)
            {
                self.current.set_cursor(pos);
            }
        }

        // Refresh parent - show volumes if at root
        self.parent = if fs::is_at_root(&self.cwd) {
            Some(Pane::new_volumes(fs::list_volumes()))
        } else {
            self.cwd.parent().and_then(|p| {
                fs::read_dir_filtered(p, self.show_hidden)
                    .ok()
                    .map(|mut e| {
                        let sort = self.get_sort_for_dir(p);
                        sort.sort_entries(&mut e);
                        Pane::new(p.to_path_buf(), e)
                    })
            })
        };

        self.refresh_preview();

        let msg = if self.show_hidden {
            "Showing hidden files"
        } else {
            "Hiding hidden files"
        };
        self.set_status(msg);

        Ok(())
    }

    pub fn open_cwd_in_editor(&mut self) -> io::Result<bool> {
        let path = self.cwd.to_string_lossy().to_string();
        // Remove Windows extended-length path prefix for nvim compatibility
        #[cfg(target_os = "windows")]
        let path = path
            .strip_prefix(r"\\?\")
            .map(str::to_string)
            .unwrap_or(path);
        self.spawn_tui_command("nvim", &[&path])
    }

    pub fn open_path_in_editor(&mut self, path: &PathBuf) -> io::Result<bool> {
        let path_str = path.to_string_lossy().to_string();
        // Remove Windows extended-length path prefix for nvim compatibility
        #[cfg(target_os = "windows")]
        let path_str = path_str
            .strip_prefix(r"\\?\")
            .map(str::to_string)
            .unwrap_or(path_str);
        self.spawn_tui_command("nvim", &[&path_str])
    }

    pub fn reveal_in_finder(&mut self, path: &PathBuf) {
        #[cfg(target_os = "macos")]
        let result = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        #[cfg(target_os = "linux")]
        let result = std::process::Command::new("xdg-open")
            .arg(path.parent().unwrap_or(path))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        #[cfg(target_os = "windows")]
        let result = std::process::Command::new("explorer")
            .arg("/select,")
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        match result {
            Ok(_) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.set_status(format!("Revealed: {}", name));
            }
            Err(e) => {
                self.set_status(format!("Error revealing: {}", e));
            }
        }
    }

    /// Launch lazygit in work directory if .git exists
    pub fn launch_lazygit(&mut self) -> io::Result<bool> {
        let git_dir = self.work_dir.join(".git");
        if !git_dir.exists() {
            self.set_status("No .git directory in work dir");
            return Ok(false);
        }

        self.spawn_tui_command_in("lazygit", &[], &self.work_dir.clone())
    }

    /// Spawn a TUI command without flashing back to the main terminal.
    /// Keeps the alternate screen, just disables raw mode for the child process.
    fn spawn_tui_command(&mut self, cmd: &str, args: &[&str]) -> io::Result<bool> {
        self.spawn_tui_command_in(cmd, args, &self.cwd.clone())
    }

    /// Spawn a TUI command in a specific directory
    fn spawn_tui_command_in(
        &mut self,
        cmd: &str,
        args: &[&str],
        dir: &PathBuf,
    ) -> io::Result<bool> {
        // Disable raw mode but stay in alternate screen (no flash)
        disable_raw_mode()?;
        // Clear screen for clean handoff to external program (fixes Windows artifacts)
        execute!(io::stdout(), Clear(ClearType::All))?;

        let status = Command::new(cmd).args(args).current_dir(dir).status();

        // In pick mode, quit after command exits
        if self.pick_mode {
            self.should_quit = true;
            return Ok(true);
        }

        // Re-enable raw mode
        enable_raw_mode()?;

        // Signal that terminal needs to be reinitialized (redrawn)
        self.needs_reinit = true;

        match status {
            Ok(_) => Ok(true),
            Err(e) => {
                self.set_status(format!("Error opening {}: {}", cmd, e));
                Ok(true)
            }
        }
    }

    /// The file(s) a shell command should act on: the marked set if any
    /// (sorted for a stable order), otherwise the current selection.
    fn target_paths(&self) -> Vec<PathBuf> {
        if !self.marked_files.is_empty() {
            let mut paths: Vec<PathBuf> = self.marked_files.iter().cloned().collect();
            paths.sort();
            paths
        } else if let Some(path) = self.current.selected_path() {
            vec![path]
        } else {
            Vec::new()
        }
    }

    /// Run a user-typed shell command (from `!`), expanding `%f`/`%d`/`%n`/`%%`
    /// against the target file(s), then suspending the TUI while it runs.
    pub fn run_shell_command(&mut self, template: &str) {
        let template = template.trim().to_string();
        self.mode = Mode::Normal;
        if template.is_empty() {
            return;
        }

        let files = self.target_paths();
        let dir = self.cwd.clone();
        let expanded = expand_command(&template, &files, &dir);

        // Suspend the TUI (leave raw mode, clear) for a clean handoff, like
        // spawn_tui_command_in, but run through the system shell so pipes,
        // globs and redirection work.
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), Clear(ClearType::All));

        #[cfg(windows)]
        let status = Command::new("cmd")
            .arg("/C")
            .arg(&expanded)
            .current_dir(&dir)
            .status();
        #[cfg(not(windows))]
        let status = Command::new("sh")
            .arg("-c")
            .arg(&expanded)
            .current_dir(&dir)
            .status();

        // Always pause so command output stays visible before redrawing.
        {
            use std::io::Write;
            let mut out = io::stdout();
            let _ = write!(out, "\r\n[Press Enter to continue]");
            let _ = out.flush();
            let mut buf = String::new();
            let _ = io::stdin().read_line(&mut buf);
        }

        let _ = enable_raw_mode();
        self.needs_reinit = true;

        if self.pick_mode {
            self.should_quit = true;
            return;
        }

        match status {
            Ok(_) => {
                // The command may have changed the directory; reflect it.
                let _ = self.refresh_current_dir();
                self.set_status(format!("Ran: {}", template));
            }
            Err(e) => self.set_status(format!("Command failed: {}", e)),
        }
    }

    pub fn toggle_wrap(&mut self) {
        self.wrap_preview = !self.wrap_preview;

        // Save setting
        let mut config = Config::load();
        config.wrap_preview = self.wrap_preview;
        config.save();

        let msg = if self.wrap_preview {
            "Wrap enabled"
        } else {
            "Wrap disabled"
        };
        self.set_status(msg);
    }

    pub fn toggle_line_numbers(&mut self) {
        self.line_numbers = !self.line_numbers;

        // Save setting
        let mut config = Config::load();
        config.line_numbers = self.line_numbers;
        config.save();

        let msg = if self.line_numbers {
            "Line numbers enabled"
        } else {
            "Line numbers disabled"
        };
        self.set_status(msg);
    }

    pub fn toggle_icons(&mut self) {
        self.show_icons = !self.show_icons;

        // Save setting
        let mut config = Config::load();
        config.show_icons = self.show_icons;
        config.save();

        let msg = if self.show_icons {
            "Icons enabled"
        } else {
            "Icons disabled"
        };
        self.set_status(msg);
    }

    pub fn toggle_icon_colors(&mut self) {
        self.colored_icons = !self.colored_icons;

        // Save setting
        let mut config = Config::load();
        config.colored_icons = self.colored_icons;
        config.save();

        let msg = if self.colored_icons {
            "Icon colors enabled"
        } else {
            "Icon colors disabled"
        };
        self.set_status(msg);
    }

    pub fn toggle_theme_icons(&mut self) {
        self.theme_icons = !self.theme_icons;

        // Save setting
        let mut config = Config::load();
        config.theme_icons = self.theme_icons;
        config.save();

        let msg = if self.theme_icons {
            "Theme icon colors enabled"
        } else {
            "Theme icon colors disabled"
        };
        self.set_status(msg);
    }

    pub fn toggle_search_navigate_on_open(&mut self) {
        self.search_navigate_on_open = !self.search_navigate_on_open;

        // Save setting
        let mut config = Config::load();
        config.search_navigate_on_open = self.search_navigate_on_open;
        config.save();

        let msg = if self.search_navigate_on_open {
            "Navigate on open enabled"
        } else {
            "Navigate on open disabled (return to search)"
        };
        self.set_status(msg);
    }

    /// Get the effective sort option for a directory (cached or global default)
    pub fn get_sort_for_dir(&self, path: &Path) -> SortOption {
        self.dir_sort_cache
            .get(path)
            .copied()
            .unwrap_or(self.sort_option)
    }

    pub fn set_sort_option(&mut self, option: SortOption) -> io::Result<()> {
        self.sort_option = option;

        // Save setting
        let mut config = Config::load();
        config.sort_option = option;
        config.save();

        // Refresh current directory with new sort order
        self.refresh_current_dir()?;

        // Also refresh parent - show volumes if at root
        self.parent = if fs::is_at_root(&self.cwd) {
            Some(Pane::new_volumes(fs::list_volumes()))
        } else {
            self.cwd.parent().and_then(|p| {
                fs::read_dir_filtered(p, self.show_hidden)
                    .ok()
                    .map(|mut e| {
                        let sort = self.get_sort_for_dir(p);
                        sort.sort_entries(&mut e);
                        Pane::new(p.to_path_buf(), e)
                    })
            })
        };

        self.set_status(format!("Sort (Global): {}", option.display_name()));
        self.mode = Mode::Normal;
        Ok(())
    }

    pub fn set_dir_sort_option(&mut self, option: SortOption) -> io::Result<()> {
        // Save sort option for current directory
        self.dir_sort_cache.insert(self.cwd.clone(), option);

        // Save to config
        let mut config = Config::load();
        config.dir_sort_cache = self.dir_sort_cache.clone();
        config.save();

        // Refresh current directory with new sort order
        self.refresh_current_dir()?;

        // Also refresh parent - show volumes if at root
        self.parent = if fs::is_at_root(&self.cwd) {
            Some(Pane::new_volumes(fs::list_volumes()))
        } else {
            self.cwd.parent().and_then(|p| {
                fs::read_dir_filtered(p, self.show_hidden)
                    .ok()
                    .map(|mut e| {
                        let parent_sort = self.get_sort_for_dir(p);
                        parent_sort.sort_entries(&mut e);
                        Pane::new(p.to_path_buf(), e)
                    })
            })
        };

        self.set_status(format!("Dir sort: {}", option.display_name()));
        self.mode = Mode::Normal;
        Ok(())
    }

    pub fn open_theme_selector(&mut self) {
        let themes = available_themes();
        let current = current_theme();
        let selected = themes.iter().position(|t| t == &current).unwrap_or(0);

        // Store original theme for preview revert
        self.original_theme = Some(current.clone());

        self.mode = Mode::ThemeSelect { selected };
    }

    pub fn select_theme(&mut self, name: &str) {
        if !set_theme(name) {
            self.set_status(format!("Theme unavailable: {}", name));
            self.mode = Mode::Normal;
            return;
        }

        // Save to config
        let mut config = Config::load();
        config.theme = name.to_string();
        if let Err(error) = config.save_checked() {
            self.refresh_preview();
            self.set_status(format!(
                "Theme active, but settings were not saved: {}",
                error
            ));
            self.mode = Mode::Normal;
            return;
        }

        // Refresh preview to show new theme
        self.refresh_preview();

        self.set_status(format!("Theme: {}", name));
        self.mode = Mode::Normal;
    }

    // File marking methods

    /// Toggle mark on the currently selected file
    pub fn toggle_mark(&mut self) {
        if let Some(path) = self.current.selected_path() {
            if self.marked_files.contains(&path) {
                self.marked_files.remove(&path);
            } else {
                self.marked_files.insert(path);
            }
        }
    }

    /// Clear all marked files
    pub fn clear_marks(&mut self) {
        self.marked_files.clear();
    }

    /// Check if a path is marked
    #[allow(dead_code)]
    pub fn is_marked(&self, path: &PathBuf) -> bool {
        self.marked_files.contains(path)
    }

    /// Get the number of marked files
    pub fn mark_count(&self) -> usize {
        self.marked_files.len()
    }

    /// Delete all marked files from the buffer
    pub fn delete_marked(&mut self) {
        if self.marked_files.is_empty() {
            self.set_status("No files marked");
            return;
        }

        self.save_undo_state();

        // Yank all marked files before deleting
        self.yank.clear();
        self.yank_is_cut = true; // Mark as cut operation
        for path in &self.marked_files {
            let is_dir = path.is_dir();
            self.yank.push((path.clone(), is_dir));
        }

        let count = self.marked_files.len();

        // Find indices of lines to delete (in reverse order to avoid shifting issues)
        let mut indices_to_delete: Vec<usize> = self
            .current
            .buffer
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                let path = self.current.buffer.path.join(&line.text);
                self.marked_files.contains(&path)
            })
            .map(|(idx, _)| idx)
            .collect();

        // Sort in reverse order so we delete from end to start
        indices_to_delete.sort_by(|a, b| b.cmp(a));

        for idx in indices_to_delete {
            self.current.buffer.delete_line(idx);
        }

        // Adjust cursor if needed
        if self.current.cursor >= self.current.buffer.lines.len() {
            self.current.cursor = self.current.buffer.lines.len().saturating_sub(1);
        }

        self.clear_marks();
        self.set_status(format!("Deleted {} file(s)", count));
        self.refresh_preview();
    }

    // Git methods

    /// Run git pull in the current directory
    pub fn git_pull(&mut self) -> io::Result<()> {
        let output = Command::new("git")
            .args(["pull"])
            .current_dir(&self.cwd)
            .output()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("Already up to date") {
                self.set_status("Already up to date");
            } else {
                self.set_status("Pull successful");
                // Refresh directory in case files changed
                let _ = self.refresh_current_dir();
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            self.set_status(format!(
                "Pull failed: {}",
                stderr.lines().next().unwrap_or("unknown error")
            ));
        }

        Ok(())
    }

    /// Run git push in the current directory
    pub fn git_push(&mut self) {
        use std::sync::mpsc;
        use std::thread;

        let cwd = self.cwd.clone();
        let (tx, rx) = mpsc::channel();

        self.set_status("Pushing...");
        self.pending_git = Some(rx);

        thread::spawn(move || {
            let output = Command::new("git")
                .args(["push"])
                .current_dir(&cwd)
                .output();

            let msg = match output {
                Ok(out) if out.status.success() => "Push successful".to_string(),
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    format!(
                        "Push failed: {}",
                        stderr.lines().next().unwrap_or("unknown error")
                    )
                }
                Err(e) => format!("Push failed: {}", e),
            };
            let _ = tx.send(msg);
        });
    }

    /// Get git status for display in commit dialog
    /// Returns a list of status lines (e.g., "M  src/main.rs", "A  new_file.txt")
    pub fn git_status_lines(&self, all: bool) -> Vec<String> {
        // For staged commits (-m), show only staged changes
        // For all commits (-am), show all changes to tracked files
        let args = if all {
            vec!["status", "--porcelain"]
        } else {
            vec!["diff", "--cached", "--name-status"]
        };

        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.work_dir)
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|l| l.to_string())
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    // Audio mode methods

    /// Enter audio browser mode
    pub fn enter_audio_mode(&mut self) {
        // Use current directory as the scan root
        let scan_root = self.cwd.clone();
        let mut state = audio::AudioModeState::new(
            scan_root,
            self.audio_auto_play,
            self.audio_normalize,
            self.audio_skip_silence,
            self.audio_volume,
            self.audio_analyzer_gradient,
        );
        state.start_scan();

        // Start analyzer with current terminal width
        let (term_width, _) = crossterm::terminal::size().unwrap_or((120, 24));
        state.start_analyzer(term_width as usize);

        self.audio_state = Some(state);
        self.mode = Mode::Audio;
    }

    /// Exit audio browser mode
    pub fn exit_audio_mode(&mut self) {
        // Stop any playing audio and analyzer
        if let Some(state) = &mut self.audio_state {
            if let Some(player) = &state.player {
                player.stop();
            }
            state.stop_analyzer();
            // Sync volume back so re-entering audio mode keeps it
            self.audio_volume = 10.0_f32.powf(state.volume_db / 20.0);
        }
        self.audio_state = None;
        self.mode = Mode::Normal;
    }

    /// Poll audio mode state updates
    pub fn poll_audio_mode(&mut self) {
        if let Some(state) = &mut self.audio_state {
            state.poll_scan();
            state.poll_waveform();
            state.poll_analyzer();
            state.clear_expired_status();
            state.check_and_skip_silence();
        }
    }

    /// Run git commit with the given message (in background)
    /// If `all` is true, adds all files (including new) and commits
    /// If `all` is false, uses -m (commit only staged changes)
    /// If `auto_push` is true, pushes after successful commit
    pub fn git_commit(&mut self, message: &str, all: bool, auto_push: bool) {
        use std::sync::mpsc;
        use std::thread;

        let cwd = self.cwd.clone();
        let message = message.to_string();
        let (tx, rx) = mpsc::channel();

        let status_msg = if auto_push {
            "Committing and pushing..."
        } else {
            "Committing..."
        };
        self.set_status(status_msg);
        self.pending_git = Some(rx);

        thread::spawn(move || {
            // Add all files if requested
            if all {
                let _ = Command::new("git")
                    .args(["add", "-A"])
                    .current_dir(&cwd)
                    .output();
            }

            // Run commit
            let output = Command::new("git")
                .args(["commit", "-m", &message])
                .current_dir(&cwd)
                .output();

            let commit_result = match output {
                Ok(out) if out.status.success() => Ok(()),
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let error_msg = if stderr.is_empty() {
                        stdout.lines().next().unwrap_or("unknown error")
                    } else {
                        stderr.lines().next().unwrap_or("unknown error")
                    };
                    Err(format!("Commit: {}", error_msg))
                }
                Err(e) => Err(format!("Commit failed: {}", e)),
            };

            // If commit succeeded and auto_push is enabled, push
            let msg = match commit_result {
                Ok(()) => {
                    if auto_push {
                        let push_output = Command::new("git")
                            .args(["push"])
                            .current_dir(&cwd)
                            .output();

                        match push_output {
                            Ok(out) if out.status.success() => {
                                "Commit and push successful".to_string()
                            }
                            Ok(out) => {
                                let stderr = String::from_utf8_lossy(&out.stderr);
                                format!(
                                    "Committed, but push failed: {}",
                                    stderr.lines().next().unwrap_or("unknown error")
                                )
                            }
                            Err(e) => format!("Committed, but push failed: {}", e),
                        }
                    } else {
                        "Commit successful".to_string()
                    }
                }
                Err(e) => e,
            };

            let _ = tx.send(msg);
        });
    }
}
