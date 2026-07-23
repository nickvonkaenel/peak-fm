use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config, Injector, Nucleo, Utf32String};

/// Entry representing a file or directory found during recursive scan
#[derive(Debug, Clone)]
pub struct SearchEntry {
    /// Full path to the file/directory
    pub path: PathBuf,
    /// Display name (relative path from search root)
    pub display: String,
    /// Whether this is a directory
    pub is_dir: bool,
}

/// State for the search mode using nucleo's parallel matcher
pub struct SearchModeState {
    /// The search query entered by the user
    pub query: String,
    /// All files found (for lookup by index)
    pub all_files: Vec<SearchEntry>,
    /// Nucleo parallel fuzzy matcher
    nucleo: Nucleo<u32>,
    /// Injector for adding items to nucleo
    injector: Injector<u32>,
    /// Currently selected result index
    pub selected: usize,
    /// Scroll offset for results list
    pub scroll_offset: usize,
    /// Height of results pane (for scroll calculation)
    pub results_height: usize,
    /// The root directory being searched
    pub search_root: PathBuf,
    /// Whether to show hidden files/directories in search
    pub show_hidden: bool,
    /// Whether to respect .gitignore files
    pub use_gitignore: bool,
    /// Whether to show directories in search results
    pub show_directories: bool,
    /// Receiver for background scan results
    scan_receiver: Option<Receiver<SearchEntry>>,
    /// Whether scanning is still in progress
    pub scanning: bool,
    /// Whether to preserve original order (for zoxide frecency)
    pub preserve_order: bool,
}

impl SearchModeState {
    /// Create a new search mode state with initial files and optional background scanner
    pub fn new(
        search_root: PathBuf,
        initial_files: Vec<SearchEntry>,
        receiver: Option<Receiver<SearchEntry>>,
        show_hidden: bool,
        use_gitignore: bool,
        show_directories: bool,
    ) -> Self {
        let scanning = receiver.is_some();

        // Create nucleo matcher with 1 column (the display path)
        // Use None for notify since we'll poll manually
        let config = Config::DEFAULT.match_paths();
        let nucleo = Nucleo::new(config, Arc::new(|| {}), None, 1);
        let injector = nucleo.injector();

        let mut state = Self {
            query: String::new(),
            all_files: Vec::new(),
            nucleo,
            injector,
            selected: 0,
            scroll_offset: 0,
            results_height: 10,
            search_root,
            show_hidden,
            use_gitignore,
            show_directories,
            scan_receiver: receiver,
            scanning,
            preserve_order: false,
        };

        // Inject initial files (filter directories if not showing them)
        for entry in initial_files {
            if !entry.is_dir || show_directories {
                state.inject_entry(entry);
            }
        }

        state
    }

    /// Inject an entry into nucleo for matching
    fn inject_entry(&mut self, entry: SearchEntry) {
        let idx = self.all_files.len() as u32;
        let display = entry.display.clone();
        self.all_files.push(entry);

        self.injector.push(idx, |_, cols| {
            cols[0] = Utf32String::from(display.as_str());
        });
    }

    /// Poll for new entries from the background scanner.
    /// Returns true if new entries were added.
    pub fn poll_scanner(&mut self) -> bool {
        if self.scan_receiver.is_none() {
            return false;
        }

        let mut added = 0;
        let batch_size = 20_000;
        let mut entries_to_inject = Vec::new();
        let mut disconnected = false;

        // First collect entries without holding borrow
        if let Some(ref receiver) = self.scan_receiver {
            loop {
                if added >= batch_size {
                    break;
                }

                match receiver.try_recv() {
                    Ok(entry) => {
                        // Filter directories if not showing them
                        if !entry.is_dir || self.show_directories {
                            entries_to_inject.push(entry);
                            added += 1;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        // Now inject them
        for entry in entries_to_inject {
            self.inject_entry(entry);
        }

        if disconnected {
            self.scanning = false;
            self.scan_receiver = None;
        }

        added > 0
    }

    /// Tick nucleo to process matches. Call this regularly.
    /// Returns true if there are pending updates.
    pub fn tick(&mut self) -> bool {
        let status = self.nucleo.tick(10);
        status.changed
    }

    /// Replace the file list and start a new background scan
    pub fn set_scanner(
        &mut self,
        initial_files: Vec<SearchEntry>,
        receiver: Option<Receiver<SearchEntry>>,
    ) {
        // Recreate nucleo matcher
        let config = Config::DEFAULT.match_paths();
        self.nucleo = Nucleo::new(config, Arc::new(|| {}), None, 1);
        self.injector = self.nucleo.injector();
        self.all_files.clear();

        // Inject initial files (filter directories if not showing them)
        for entry in initial_files {
            if !entry.is_dir || self.show_directories {
                self.inject_entry(entry);
            }
        }

        // Re-apply current query if any
        if !self.query.is_empty() {
            self.nucleo.pattern.reparse(
                0,
                &self.query,
                CaseMatching::Ignore,
                Normalization::Smart,
                false,
            );
        }

        self.scan_receiver = receiver;
        self.scanning = self.scan_receiver.is_some();
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Replace entries directly (for zoxide mode where filtering is done externally)
    /// All entries are treated as matched, preserving their order.
    pub fn replace_entries(&mut self, entries: Vec<SearchEntry>) {
        // Recreate nucleo with empty pattern so all entries match
        let config = Config::DEFAULT.match_paths();
        self.nucleo = Nucleo::new(config, Arc::new(|| {}), None, 1);
        self.injector = self.nucleo.injector();
        self.all_files.clear();

        // Inject all entries
        for entry in entries {
            self.inject_entry(entry);
        }

        // Clear the pattern so all entries match
        self.nucleo
            .pattern
            .reparse(0, "", CaseMatching::Ignore, Normalization::Smart, false);

        // Tick to process
        self.nucleo.tick(10);

        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Append a character to the query
    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        // Use append=true for incremental matching when adding chars
        self.nucleo.pattern.reparse(
            0,
            &self.query,
            CaseMatching::Ignore,
            Normalization::Smart,
            true, // append mode - only filter existing matches
        );
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Remove the last character from the query
    pub fn pop_char(&mut self) {
        self.query.pop();
        // Must do full reparse when removing chars
        self.nucleo.pattern.reparse(
            0,
            &self.query,
            CaseMatching::Ignore,
            Normalization::Smart,
            false,
        );
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Delete the last word from the query (Ctrl+w behavior)
    pub fn delete_word(&mut self) {
        // Trim trailing spaces first
        while self.query.ends_with(' ') {
            self.query.pop();
        }
        // Then delete until we hit a space or the start
        while !self.query.is_empty() && !self.query.ends_with(' ') {
            self.query.pop();
        }
        self.nucleo.pattern.reparse(
            0,
            &self.query,
            CaseMatching::Ignore,
            Normalization::Smart,
            false,
        );
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Clear the entire query
    pub fn clear(&mut self) {
        self.query.clear();
        self.nucleo
            .pattern
            .reparse(0, "", CaseMatching::Ignore, Normalization::Smart, false);
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Set the query directly (for resuming previous search)
    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.nucleo.pattern.reparse(
            0,
            &self.query,
            CaseMatching::Ignore,
            Normalization::Smart,
            false,
        );
    }

    /// Set selection and scroll state (for resuming)
    pub fn set_selection(&mut self, selected: usize, scroll_offset: usize) {
        self.selected = selected;
        self.scroll_offset = scroll_offset;
    }

    /// Set preserve_order flag (for zoxide frecency)
    pub fn set_preserve_order(&mut self, preserve: bool) {
        self.preserve_order = preserve;
    }

    /// Get the number of matched results
    pub fn matched_count(&self) -> u32 {
        self.nucleo.snapshot().matched_item_count()
    }

    /// Get the total number of items
    #[allow(dead_code)]
    pub fn total_count(&self) -> u32 {
        self.nucleo.snapshot().item_count()
    }

    /// Move selection by delta, optionally wrapping around at boundaries
    pub fn move_selection(&mut self, delta: isize, wrap: bool) {
        let count = self.matched_count() as usize;
        if count == 0 {
            return;
        }

        let new_selected = if wrap {
            if delta < 0 {
                let abs_delta = (-delta) as usize;
                if abs_delta > self.selected {
                    // Wrap to bottom
                    count - 1 - (abs_delta - self.selected - 1) % count
                } else {
                    self.selected - abs_delta
                }
            } else {
                let new_pos = self.selected + delta as usize;
                if new_pos >= count {
                    // Wrap to top
                    (new_pos - count) % count
                } else {
                    new_pos
                }
            }
        } else {
            // Clamp to bounds
            if delta < 0 {
                self.selected.saturating_sub((-delta) as usize)
            } else {
                (self.selected + delta as usize).min(count - 1)
            }
        };

        self.selected = new_selected;

        // Adjust scroll to keep selection visible
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + self.results_height {
            self.scroll_offset = self.selected - self.results_height + 1;
        }
    }

    /// Get the currently selected entry, if any
    pub fn selected_entry(&self) -> Option<&SearchEntry> {
        let indices = self.get_matched_indices();
        indices
            .get(self.selected)
            .map(|&idx| &self.all_files[idx as usize])
    }

    /// Get the path of the currently selected entry
    pub fn selected_path(&self) -> Option<&PathBuf> {
        self.selected_entry().map(|e| &e.path)
    }

    /// Get all matched indices, optionally sorted by original order (for zoxide frecency)
    fn get_matched_indices(&self) -> Vec<u32> {
        let snapshot = self.nucleo.snapshot();
        let count = snapshot.matched_item_count();

        if self.preserve_order && !self.query.is_empty() {
            // Collect all matched original indices and sort by original order (frecency)
            let mut indices: Vec<u32> = (0..count)
                .filter_map(|i| snapshot.get_matched_item(i).map(|item| *item.data))
                .collect();
            indices.sort_unstable(); // Sort by original index = frecency order
            indices
        } else {
            // Use nucleo's default ordering (by match score)
            (0..count)
                .filter_map(|i| snapshot.get_matched_item(i).map(|item| *item.data))
                .collect()
        }
    }

    /// Get visible results for rendering
    pub fn visible_results(&self) -> Vec<(usize, &SearchEntry)> {
        let indices = self.get_matched_indices();
        let count = indices.len();

        let start = self.scroll_offset;
        let end = (start + self.results_height).min(count);

        (start..end)
            .map(|i| (i, &self.all_files[indices[i] as usize]))
            .collect()
    }
}
