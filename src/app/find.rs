//! Find mode - fuzzy file search functionality

use std::io;
use std::path::PathBuf;
use std::process::Command;

use crate::core::image::load_image_async;
use crate::core::preview::{load_preview, Preview};
use crate::core::search::{SearchEntry, SearchModeState};
use crate::fs;
use crate::input::mode::Mode;

use super::{App, LastSearch, NvimAction};

impl App {
    // Find mode methods

    pub fn enter_find_mode(&mut self) {
        // Use search_lock_dir if set, otherwise use work_dir
        let search_root = self
            .search_lock_dir
            .clone()
            .unwrap_or_else(|| self.work_dir.clone());
        self.enter_find_mode_in(&search_root);
    }

    /// Enter find mode locked to the current directory only
    pub fn enter_find_mode_cwd(&mut self) {
        self.enter_find_mode_in(&self.cwd.clone());
    }

    fn enter_find_mode_in(&mut self, search_root: &PathBuf) {
        // Scan directory using Find mode's own hidden setting
        let show_hidden = self.find_show_hidden;
        let use_gitignore = self.find_use_gitignore;
        let show_directories = self.find_show_directories;

        // First 2k files scanned synchronously, rest in background
        let (initial_files, receiver) = fs::spawn_recursive_scan(
            search_root.clone(),
            show_hidden,
            show_hidden, // recurse_hidden_dirs matches show_hidden
            use_gitignore,
        );

        let file_count = initial_files.len();
        let scanning = receiver.is_some();

        self.find_state = Some(SearchModeState::new(
            search_root.clone(),
            initial_files,
            receiver,
            show_hidden,
            use_gitignore,
            show_directories,
        ));
        self.mode = Mode::Find;

        // Show search root in status if different from cwd
        let root_display = if search_root != &self.cwd {
            format!(
                " in {}",
                search_root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )
        } else {
            String::new()
        };

        if scanning {
            self.set_status(format!(
                "{} files{} (scanning...)",
                file_count, root_display
            ));
        } else {
            self.set_status(format!("{} files{}", file_count, root_display));
        }
    }

    pub fn exit_find_mode(&mut self) {
        self.exit_find_mode_impl(true);
    }

    fn exit_find_mode_impl(&mut self, quit_in_pick_mode: bool) {
        // Store state for resume (but not zoxide mode)
        if !self.zoxide_mode {
            if let Some(ref state) = self.find_state {
                self.last_search = Some(LastSearch::Find {
                    search_root: state.search_root.clone(),
                    query: state.query.clone(),
                    selected: state.selected,
                    scroll_offset: state.scroll_offset,
                    files: state.all_files.clone(),
                    show_hidden: state.show_hidden,
                    use_gitignore: state.use_gitignore,
                    show_directories: state.show_directories,
                });
            }
        }

        self.find_state = None;
        self.zoxide_mode = false;
        self.mode = Mode::Normal;
        self.status.clear();

        // Refresh preview for current selection in normal mode
        self.refresh_preview();

        // In pick mode, quit when exiting find mode (unless navigating to a directory)
        if self.pick_mode && quit_in_pick_mode {
            self.should_quit = true;
        }
    }

    pub fn enter_zoxide_mode(&mut self) {
        // Get directories from zoxide (with scores, excluding the current dir)
        let entries = match self.query_zoxide("") {
            Ok(entries) => entries,
            Err(e) => {
                self.set_status(format!("zoxide error: {}", e));
                return;
            }
        };

        let dir_count = entries.len();
        if dir_count == 0 {
            self.set_status("zoxide database is empty");
            return;
        }

        let mut state = SearchModeState::new(
            self.cwd.clone(),
            entries,
            None, // No background scanner for zoxide
            true, // show_hidden doesn't matter for zoxide
            true, // use_gitignore doesn't matter for zoxide
            true, // show_directories - zoxide is all directories
        );
        state.set_preserve_order(true); // Preserve frecency order when searching
        self.find_state = Some(state);
        self.zoxide_mode = true;
        self.mode = Mode::Find;
        self.set_status(format!("{} directories", dir_count));
    }

    /// Remove the selected directory from the zoxide database (zoxide mode only)
    pub fn zoxide_remove_selected(&mut self) {
        if !self.zoxide_mode {
            return;
        }

        let (path, selected, scroll_offset) = match self.find_state.as_ref() {
            Some(state) => match state.selected_entry() {
                // Use the display string (the exact line zoxide emitted) so
                // the removal matches the database entry
                Some(entry) => (entry.display.clone(), state.selected, state.scroll_offset),
                None => return,
            },
            None => return,
        };

        let output = Command::new("zoxide").args(["remove", &path]).output();

        match output {
            Ok(output) if output.status.success() => {
                self.refresh_zoxide_results();
                // refresh resets the cursor to the top; put it back near
                // where it was, clamped to the shorter list
                if let Some(ref mut state) = self.find_state {
                    state.set_selection(selected, scroll_offset);
                    state.move_selection(0, false);
                }
                self.refresh_find_preview();
                self.set_status(format!("Removed from zoxide: {}", path));
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                self.set_status(format!("zoxide remove failed: {}", err.trim()));
            }
            Err(e) => {
                self.set_status(format!("zoxide error: {}", e));
            }
        }
    }

    pub fn find_toggle_hidden(&mut self) {
        // Toggle the global setting
        self.find_show_hidden = !self.find_show_hidden;

        if let Some(ref mut state) = self.find_state {
            state.show_hidden = self.find_show_hidden;

            // Rescan with new setting
            let (initial_files, receiver) = fs::spawn_recursive_scan(
                self.cwd.clone(),
                state.show_hidden,
                state.show_hidden,
                state.use_gitignore,
            );

            let file_count = initial_files.len();
            let scanning = receiver.is_some();
            state.set_scanner(initial_files, receiver);

            let msg = if scanning {
                if state.show_hidden {
                    format!("{} files (scanning, showing hidden...)", file_count)
                } else {
                    format!("{} files (scanning...)", file_count)
                }
            } else if state.show_hidden {
                format!("{} files (showing hidden)", file_count)
            } else {
                format!("{} files", file_count)
            };
            self.set_status(msg);
            self.refresh_find_preview();
        } else {
            let msg = if self.find_show_hidden {
                "Hidden files enabled for search"
            } else {
                "Hidden files disabled for search"
            };
            self.set_status(msg);
        }
    }

    pub fn find_toggle_gitignore(&mut self) {
        // Toggle the global setting
        self.find_use_gitignore = !self.find_use_gitignore;

        if let Some(ref mut state) = self.find_state {
            state.use_gitignore = self.find_use_gitignore;

            // Rescan with new setting
            let (initial_files, receiver) = fs::spawn_recursive_scan(
                self.cwd.clone(),
                state.show_hidden,
                state.show_hidden,
                state.use_gitignore,
            );

            let file_count = initial_files.len();
            let scanning = receiver.is_some();
            state.set_scanner(initial_files, receiver);

            let msg = if scanning {
                if state.use_gitignore {
                    format!("{} files (scanning...)", file_count)
                } else {
                    format!("{} files (scanning, ignoring .gitignore...)", file_count)
                }
            } else if !state.use_gitignore {
                format!("{} files (ignoring .gitignore)", file_count)
            } else {
                format!("{} files", file_count)
            };
            self.set_status(msg);
            self.refresh_find_preview();
        } else {
            // Not in find mode, just toggle the setting
            let msg = if self.find_use_gitignore {
                "Gitignore enabled for search"
            } else {
                "Gitignore disabled for search"
            };
            self.set_status(msg);
        }
    }

    pub fn find_toggle_directories(&mut self) {
        // Toggle the global setting
        self.find_show_directories = !self.find_show_directories;

        if let Some(ref mut state) = self.find_state {
            state.show_directories = self.find_show_directories;

            // Rescan with new setting
            let (initial_files, receiver) = fs::spawn_recursive_scan(
                state.search_root.clone(),
                state.show_hidden,
                state.show_hidden,
                state.use_gitignore,
            );

            let file_count = initial_files.len();
            let scanning = receiver.is_some();
            state.set_scanner(initial_files, receiver);

            let msg = if scanning {
                if state.show_directories {
                    format!("{} files (scanning, showing directories...)", file_count)
                } else {
                    format!("{} files (scanning...)", file_count)
                }
            } else if state.show_directories {
                format!("{} files (showing directories)", file_count)
            } else {
                format!("{} files", file_count)
            };
            self.set_status(msg);
            self.refresh_find_preview();
        } else {
            let msg = if self.find_show_directories {
                "Directories enabled for search"
            } else {
                "Directories disabled for search"
            };
            self.set_status(msg);
        }
    }

    pub fn find_push_char(&mut self, c: char) {
        if let Some(ref mut state) = self.find_state {
            state.push_char(c);
        }
        if self.zoxide_mode {
            self.refresh_zoxide_results();
        }
        self.refresh_find_preview();
    }

    pub fn find_pop_char(&mut self) {
        if let Some(ref mut state) = self.find_state {
            state.pop_char();
        }
        if self.zoxide_mode {
            self.refresh_zoxide_results();
        }
        self.refresh_find_preview();
    }

    pub fn find_delete_word(&mut self) {
        if let Some(ref mut state) = self.find_state {
            state.delete_word();
        }
        if self.zoxide_mode {
            self.refresh_zoxide_results();
        }
        self.refresh_find_preview();
    }

    pub fn find_clear(&mut self) {
        if let Some(ref mut state) = self.find_state {
            state.clear();
        }
        if self.zoxide_mode {
            self.refresh_zoxide_results();
        }
        self.refresh_find_preview();
    }

    /// Run `zoxide query -l --score` (excluding the current directory) and
    /// parse the output into entries. `query` may be empty to list everything.
    fn query_zoxide(&self, query: &str) -> io::Result<Vec<SearchEntry>> {
        let mut cmd = Command::new("zoxide");
        cmd.args(["query", "-l", "--score"])
            .arg("--exclude")
            .arg(self.cwd.as_os_str());
        if !query.is_empty() {
            cmd.arg(query);
        }

        let output = cmd.output()?;
        if !output.status.success() {
            // zoxide exits non-zero when nothing matches; treat as empty
            return Ok(Vec::new());
        }

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(parse_zoxide_score_line)
            .collect())
    }

    /// Re-query zoxide with the current search query
    fn refresh_zoxide_results(&mut self) {
        let query = match self.find_state.as_ref() {
            Some(state) => state.query.clone(),
            None => return,
        };

        let entries = self.query_zoxide(&query).unwrap_or_default();

        // Update the state with new entries (skip nucleo, just show in order)
        if let Some(ref mut state) = self.find_state {
            state.replace_entries(entries);
        }
    }

    /// Boost the selected directory's zoxide score (zoxide mode only)
    pub fn zoxide_boost_selected(&mut self) {
        if !self.zoxide_mode {
            return;
        }

        let (path, selected, scroll_offset) = match self.find_state.as_ref() {
            Some(state) => match state.selected_entry() {
                Some(entry) => (entry.display.clone(), state.selected, state.scroll_offset),
                None => return,
            },
            None => return,
        };

        let output = Command::new("zoxide").args(["add", &path]).output();

        match output {
            Ok(output) if output.status.success() => {
                self.refresh_zoxide_results();
                if let Some(ref mut state) = self.find_state {
                    state.set_selection(selected, scroll_offset);
                    state.move_selection(0, false);
                }
                self.refresh_find_preview();
                self.set_status(format!("Boosted: {}", path));
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                self.set_status(format!("zoxide add failed: {}", err.trim()));
            }
            Err(e) => {
                self.set_status(format!("zoxide error: {}", e));
            }
        }
    }

    /// Remove zoxide entries whose directories no longer exist (zoxide mode only)
    pub fn zoxide_prune_dead(&mut self) {
        if !self.zoxide_mode {
            return;
        }

        // --all includes entries for directories that no longer exist
        let output = Command::new("zoxide")
            .args(["query", "-l", "--all"])
            .output();

        let dead: Vec<String> = match output {
            Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.is_empty() && !PathBuf::from(line).is_dir())
                .map(|line| line.to_string())
                .collect(),
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                self.set_status(format!("zoxide query failed: {}", err.trim()));
                return;
            }
            Err(e) => {
                self.set_status(format!("zoxide error: {}", e));
                return;
            }
        };

        if dead.is_empty() {
            self.set_status("No dead entries in zoxide database");
            return;
        }

        // Remove in chunks to stay well under command line length limits
        let mut removed = 0;
        for chunk in dead.chunks(50) {
            match Command::new("zoxide").arg("remove").args(chunk).output() {
                Ok(output) if output.status.success() => removed += chunk.len(),
                Ok(output) => {
                    let err = String::from_utf8_lossy(&output.stderr);
                    self.set_status(format!("zoxide remove failed: {}", err.trim()));
                    return;
                }
                Err(e) => {
                    self.set_status(format!("zoxide error: {}", e));
                    return;
                }
            }
        }

        let (selected, scroll_offset) = self
            .find_state
            .as_ref()
            .map(|s| (s.selected, s.scroll_offset))
            .unwrap_or((0, 0));
        self.refresh_zoxide_results();
        if let Some(ref mut state) = self.find_state {
            state.set_selection(selected, scroll_offset);
            state.move_selection(0, false);
        }
        self.refresh_find_preview();
        self.set_status(format!(
            "Pruned {} dead {} from zoxide",
            removed,
            if removed == 1 { "entry" } else { "entries" }
        ));
    }

    /// Poll the background scanner and tick nucleo for matching.
    /// Returns true if there were updates.
    pub fn poll_find_scanner(&mut self) -> bool {
        let (added, ticked, just_finished, count) = if let Some(ref mut state) = self.find_state {
            let was_scanning = state.scanning;
            let added = state.poll_scanner();
            let ticked = state.tick(); // Process matches in parallel
            let just_finished = was_scanning && !state.scanning;
            (added, ticked, just_finished, state.all_files.len())
        } else {
            return false;
        };

        if added || ticked {
            self.refresh_find_preview();
        }

        if just_finished {
            self.set_status(format!("{} files", count));
        }

        added || ticked
    }

    pub fn find_move(&mut self, delta: isize, wrap: bool) {
        if let Some(ref mut state) = self.find_state {
            state.move_selection(delta, wrap);
            self.refresh_find_preview();
        }
    }

    pub fn find_select(&mut self) -> io::Result<bool> {
        let selected_path = self
            .find_state
            .as_ref()
            .and_then(|s| s.selected_path().cloned());
        let is_zoxide = self.zoxide_mode;

        if let Some(path) = selected_path {
            if path.is_dir() {
                // Exit find mode and navigate to directory
                // Don't quit in picker mode - we're just navigating to a directory
                self.exit_find_mode_impl(false);
                self.navigate_to(path.clone())?;
                // In zoxide mode, also set work_dir to the selected directory
                if is_zoxide {
                    self.work_dir = path;
                    self.set_status(format!("work dir: {}", self.work_dir.display()));
                    // In pick mode, the directory is the selection - quit so the
                    // shell wrapper can cd to it
                    if self.pick_mode {
                        self.should_quit = true;
                    }
                }
            } else if self.nvim_mode {
                // In nvim mode, output path and quit
                self.nvim_pick(NvimAction::Edit(path, None));
            } else if self.search_navigate_on_open {
                // New behavior: exit find mode, navigate to file's directory, open file
                let filename = path.file_name().map(|n| n.to_string_lossy().to_string());
                let parent = path.parent().map(|p| p.to_path_buf());
                // Don't quit in picker mode when navigating - we'll quit after opening the file
                self.exit_find_mode_impl(false);
                if let Some(dir) = parent {
                    self.navigate_to(dir)?;
                }
                if let Some(name) = filename {
                    self.select_by_name(&name);
                }
                self.open_file(&path)?;
                return Ok(true);
            } else {
                // Legacy behavior: keep find state, return to it after editor closes
                self.open_file(&path)?;
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Navigate to the selected file's directory without opening it
    pub fn find_navigate(&mut self) -> io::Result<()> {
        let selected_path = self
            .find_state
            .as_ref()
            .and_then(|s| s.selected_path().cloned());

        if let Some(path) = selected_path {
            // Get filename for cursor positioning
            let filename = path.file_name().map(|n| n.to_string_lossy().to_string());

            // Determine target directory
            let target_dir = if path.is_dir() {
                path.clone()
            } else {
                path.parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or(path.clone())
            };

            // Exit find mode and navigate
            // Don't quit in picker mode - we're just navigating to a directory
            self.exit_find_mode_impl(false);
            self.navigate_to(target_dir)?;

            // Position cursor on the file
            if let Some(name) = filename {
                if !path.is_dir() {
                    self.select_by_name(&name);
                }
            }
        }

        Ok(())
    }

    /// Select file for horizontal split in find mode (nvim mode only)
    pub fn find_select_split(&mut self) {
        if !self.nvim_mode {
            return;
        }
        if let Some(path) = self
            .find_state
            .as_ref()
            .and_then(|s| s.selected_path().cloned())
        {
            if path.is_file() {
                self.nvim_pick(NvimAction::Split(path, None));
            }
        }
    }

    /// Select file for vertical split in find mode (nvim mode only)
    pub fn find_select_vsplit(&mut self) {
        if !self.nvim_mode {
            return;
        }
        if let Some(path) = self
            .find_state
            .as_ref()
            .and_then(|s| s.selected_path().cloned())
        {
            if path.is_file() {
                self.nvim_pick(NvimAction::Vsplit(path, None));
            }
        }
    }

    /// Select file for new tab in find mode (nvim mode only)
    pub fn find_select_tab(&mut self) {
        if !self.nvim_mode {
            return;
        }
        if let Some(path) = self
            .find_state
            .as_ref()
            .and_then(|s| s.selected_path().cloned())
        {
            if path.is_file() {
                self.nvim_pick(NvimAction::Tab(path, None));
            }
        }
    }

    pub(super) fn refresh_find_preview(&mut self) {
        // Stop any playing audio when selection changes
        self.stop_audio_silent();

        let show_hidden = self.find_show_hidden;
        let width = self.preview_width;
        let height = self.preview_height;

        if let Some(ref state) = self.find_state {
            if let Some(path) = state.selected_path() {
                // Check if it's an image - load asynchronously with short wait
                if crate::core::preview::is_image_file(path) {
                    let path = path.clone();
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
                            self.pending_image = None;
                            self.preview = Preview::None;
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            // Still loading, continue in background
                            self.pending_image = Some((path, rx));
                            self.preview = Preview::None;
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            self.pending_image = None;
                            self.preview = Preview::None;
                        }
                    }
                } else {
                    self.pending_image = None;
                    let sort = if path.is_dir() {
                        self.get_sort_for_dir(path)
                    } else {
                        self.sort_option
                    };
                    self.preview = load_preview(path, height, width, show_hidden, false, sort);
                }
            } else {
                self.pending_image = None;
                self.preview = Preview::None;
            }
        }
    }
}

/// Parse one line of `zoxide query -l --score` output: "  <score> <path>"
fn parse_zoxide_score_line(line: &str) -> Option<SearchEntry> {
    let trimmed = line.trim_start();
    let (score, rest) = trimmed.split_once(char::is_whitespace)?;
    let path_str = rest.trim_start();
    if path_str.is_empty() {
        return None;
    }
    Some(SearchEntry {
        path: PathBuf::from(path_str),
        display: path_str.to_string(),
        is_dir: true,
        score: Some(score.to_string()),
    })
}
