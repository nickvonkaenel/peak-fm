//! Grep mode - content search functionality (ripgrep)

use std::io;
use std::path::PathBuf;
use std::process::Command;

use crossterm::execute;
use crossterm::terminal::{Clear, ClearType};

use crate::core::preview::{FilePreview, Preview};
use crate::core::search::SearchModeState;
use crate::core::GrepModeState;
use crate::input::mode::Mode;

use super::{App, LastSearch, NvimAction};

impl App {
    // Grep mode methods

    pub fn enter_grep_mode(&mut self) {
        // Use search_lock_dir if set, otherwise use work_dir
        let search_root = self
            .search_lock_dir
            .clone()
            .unwrap_or_else(|| self.work_dir.clone());
        self.enter_grep_mode_in(&search_root);
    }

    /// Enter grep mode locked to the current directory only
    pub fn enter_grep_mode_cwd(&mut self) {
        self.enter_grep_mode_in(&self.cwd.clone());
    }

    fn enter_grep_mode_in(&mut self, search_root: &PathBuf) {
        self.grep_state = Some(GrepModeState::new(search_root.clone(), self.show_hidden));
        self.mode = Mode::Grep;
        self.preview = Preview::None;

        // Show search root in status if different from cwd
        if search_root != &self.cwd {
            self.set_status(format!(
                "grep in {}",
                search_root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ));
        } else {
            self.status.clear();
        }
    }

    pub fn exit_grep_mode(&mut self) {
        // Store state for resume
        if let Some(ref state) = self.grep_state {
            self.last_search = Some(LastSearch::Grep {
                search_root: state.search_root.clone(),
                query: state.query.clone(),
                selected: state.selected,
                scroll_offset: state.scroll_offset,
                matches: state.matches.clone(),
                show_hidden: state.show_hidden,
            });
        }

        self.grep_state = None;
        self.mode = Mode::Normal;
        self.status.clear();

        // Refresh preview for current selection in normal mode
        self.refresh_preview();

        // In pick mode, quit when exiting grep mode
        if self.pick_mode {
            self.should_quit = true;
        }
    }

    /// Resume the last search (Find or Grep)
    pub fn resume_last_search(&mut self) {
        let Some(last) = self.last_search.clone() else {
            self.set_status("No previous search");
            return;
        };

        match last {
            LastSearch::Find {
                search_root,
                query,
                selected,
                scroll_offset,
                files,
                show_hidden,
                use_gitignore,
                show_directories,
            } => {
                // Create new SearchModeState with the stored files
                let mut state = SearchModeState::new(
                    search_root.clone(),
                    files,
                    None, // No background scanner for resume
                    show_hidden,
                    use_gitignore,
                    show_directories,
                );

                // Restore query and selection
                state.set_query(query.clone());
                state.set_selection(selected, scroll_offset);

                // Need to tick nucleo to process the query
                state.tick();

                self.find_state = Some(state);
                self.mode = Mode::Find;
                self.zoxide_mode = false;

                let root_display = if search_root != self.cwd {
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
                self.set_status(format!("resumed{}", root_display));
            }
            LastSearch::Grep {
                search_root,
                query,
                selected,
                scroll_offset,
                matches,
                show_hidden,
            } => {
                // Create GrepModeState with restored results
                let state = GrepModeState::with_results(
                    search_root.clone(),
                    query,
                    matches,
                    selected,
                    scroll_offset,
                    show_hidden,
                );

                self.grep_state = Some(state);
                self.mode = Mode::Grep;
                self.preview = Preview::None;

                let root_display = if search_root != self.cwd {
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
                self.set_status(format!("resumed{}", root_display));
            }
        }
    }

    pub fn grep_push_char(&mut self, c: char) {
        let (searching, empty) = if let Some(ref mut state) = self.grep_state {
            state.push_char(c);
            (state.searching, state.matches.is_empty())
        } else {
            return;
        };

        if searching {
            self.set_status("Searching...");
        }
        if empty {
            self.preview = Preview::None;
        }
    }

    pub fn grep_pop_char(&mut self) {
        let (searching, query_short, empty) = if let Some(ref mut state) = self.grep_state {
            state.pop_char();
            (
                state.searching,
                state.query.len() < 2,
                state.matches.is_empty(),
            )
        } else {
            return;
        };

        if searching {
            self.set_status("Searching...");
        } else if query_short {
            self.status.clear();
        }
        if empty {
            self.preview = Preview::None;
        }
    }

    pub fn grep_delete_word(&mut self) {
        let (searching, query_short, empty) = if let Some(ref mut state) = self.grep_state {
            state.delete_word();
            (
                state.searching,
                state.query.len() < 2,
                state.matches.is_empty(),
            )
        } else {
            return;
        };

        if searching {
            self.set_status("Searching...");
        } else if query_short {
            self.status.clear();
        }
        if empty {
            self.preview = Preview::None;
        }
    }

    pub fn grep_clear(&mut self) {
        if let Some(ref mut state) = self.grep_state {
            state.clear();
        }
        self.preview = Preview::None;
        self.status.clear();
    }

    pub fn grep_move(&mut self, delta: isize, wrap: bool) {
        if let Some(ref mut state) = self.grep_state {
            state.move_selection(delta, wrap);
            self.refresh_grep_preview();
        }
    }

    pub fn grep_select(&mut self) -> io::Result<bool> {
        let selected = self.grep_state.as_ref().and_then(|s| {
            s.selected_match()
                .map(|m| (s.search_root.clone(), m.clone()))
        });

        if let Some((search_root, result)) = selected {
            let full_path = search_root.join(&result.path);
            if full_path.is_file() {
                if self.nvim_mode {
                    // In nvim mode, output path with line and column and quit
                    self.nvim_pick(NvimAction::Edit(
                        full_path,
                        Some((result.line_num, result.col_num)),
                    ));
                    return Ok(false);
                }
                let line_num = result.line_num;
                if self.search_navigate_on_open {
                    // New behavior: exit grep mode, navigate to file's directory, open file
                    let filename = full_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string());
                    let parent = full_path.parent().map(|p| p.to_path_buf());
                    self.exit_grep_mode();
                    if let Some(dir) = parent {
                        self.navigate_to(dir)?;
                    }
                    if let Some(name) = filename {
                        self.select_by_name(&name);
                    }
                    self.open_file_at_line(&full_path, line_num)?;
                    return Ok(true);
                } else {
                    // Legacy behavior: keep grep state, return to it after editor closes
                    self.open_file_at_line(&full_path, line_num)?;
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    /// Navigate to the selected file's directory without opening it
    pub fn grep_navigate(&mut self) -> io::Result<()> {
        let selected = self.grep_state.as_ref().and_then(|s| {
            s.selected_match()
                .map(|m| (s.search_root.clone(), m.clone()))
        });

        if let Some((search_root, result)) = selected {
            let full_path = search_root.join(&result.path);

            // Get filename for cursor positioning
            let filename = full_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string());

            // Get parent directory
            let target_dir = full_path.parent().map(|p| p.to_path_buf());

            // Exit grep mode and navigate
            self.exit_grep_mode();
            if let Some(dir) = target_dir {
                self.navigate_to(dir)?;
            }

            // Position cursor on the file
            if let Some(name) = filename {
                self.select_by_name(&name);
            }
        }

        Ok(())
    }

    /// Select file for horizontal split in grep mode (nvim mode only)
    pub fn grep_select_split(&mut self) {
        if !self.nvim_mode {
            return;
        }
        let selected = self.grep_state.as_ref().and_then(|s| {
            s.selected_match()
                .map(|m| (s.search_root.clone(), m.clone()))
        });
        if let Some((search_root, result)) = selected {
            let full_path = search_root.join(&result.path);
            if full_path.is_file() {
                self.nvim_pick(NvimAction::Split(
                    full_path,
                    Some((result.line_num, result.col_num)),
                ));
            }
        }
    }

    /// Select file for vertical split in grep mode (nvim mode only)
    pub fn grep_select_vsplit(&mut self) {
        if !self.nvim_mode {
            return;
        }
        let selected = self.grep_state.as_ref().and_then(|s| {
            s.selected_match()
                .map(|m| (s.search_root.clone(), m.clone()))
        });
        if let Some((search_root, result)) = selected {
            let full_path = search_root.join(&result.path);
            if full_path.is_file() {
                self.nvim_pick(NvimAction::Vsplit(
                    full_path,
                    Some((result.line_num, result.col_num)),
                ));
            }
        }
    }

    /// Select file for new tab in grep mode (nvim mode only)
    pub fn grep_select_tab(&mut self) {
        if !self.nvim_mode {
            return;
        }
        let selected = self.grep_state.as_ref().and_then(|s| {
            s.selected_match()
                .map(|m| (s.search_root.clone(), m.clone()))
        });
        if let Some((search_root, result)) = selected {
            let full_path = search_root.join(&result.path);
            if full_path.is_file() {
                self.nvim_pick(NvimAction::Tab(
                    full_path,
                    Some((result.line_num, result.col_num)),
                ));
            }
        }
    }

    pub(super) fn open_file_at_line(&mut self, path: &PathBuf, line: usize) -> io::Result<bool> {
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

        // Open in neovim at specific line with work_dir as working directory
        // Use --cmd to set directory before file loads (important for LSP)
        let work_dir_str = strip_windows_prefix(&self.work_dir.to_string_lossy());
        let status = Command::new("nvim")
            .arg("--cmd")
            .arg(format!("cd {}", work_dir_str))
            .arg(format!("+{}", line))
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
                self.set_status(format!("Error opening file: {}", e));
                Ok(true)
            }
        }
    }

    /// Poll grep results and update status
    pub fn poll_grep_results(&mut self) -> bool {
        let (added, just_finished, count) = if let Some(ref mut state) = self.grep_state {
            let was_searching = state.searching;
            let added = state.poll_results();
            let just_finished = was_searching && !state.searching;
            (added, just_finished, state.matches.len())
        } else {
            return false;
        };

        if added {
            self.refresh_grep_preview();
        }

        if just_finished {
            self.set_status(format!("{} matches", count));
        }

        added
    }

    pub(super) fn refresh_grep_preview(&mut self) {
        if let Some(ref state) = self.grep_state {
            if let Some(result) = state.selected_match() {
                // Use search_root since result.path is relative to it
                let full_path = state.search_root.join(&result.path);
                let line_num = result.line_num;
                // Load preview centered around the matching line
                let max_lines = (self.preview_height * 2).max(100);
                let file_preview = FilePreview::load_around_line(full_path, line_num, max_lines);
                self.preview = Preview::File(file_preview);
            } else {
                self.preview = Preview::None;
            }
        }
    }
}
