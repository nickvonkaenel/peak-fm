use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use crate::paths::{IGNORE_FILE_NAME, LEGACY_IGNORE_FILE_NAME};

/// A single grep match result
#[derive(Debug, Clone)]
pub struct GrepMatch {
    /// File path where match was found
    pub path: PathBuf,
    /// Line number (1-indexed)
    pub line_num: usize,
    /// Column number (1-indexed)
    pub col_num: usize,
    /// The matching line content
    pub line: String,
}

/// State for grep search mode
pub struct GrepModeState {
    /// The search query
    pub query: String,
    /// All matches found
    pub matches: Vec<GrepMatch>,
    /// Currently selected match index
    pub selected: usize,
    /// Scroll offset for results list
    pub scroll_offset: usize,
    /// Height of results pane
    pub results_height: usize,
    /// Root directory being searched
    pub search_root: PathBuf,
    /// Receiver for background grep results
    grep_receiver: Option<Receiver<GrepMatch>>,
    /// Whether grep is still running
    pub searching: bool,
    /// Whether to show hidden files
    pub show_hidden: bool,
    /// Whether to clear old results when first new result arrives
    pending_clear: bool,
}

impl GrepModeState {
    /// Create a new grep mode state
    pub fn new(search_root: PathBuf, show_hidden: bool) -> Self {
        Self {
            query: String::new(),
            matches: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            results_height: 10,
            search_root,
            grep_receiver: None,
            searching: false,
            show_hidden,
            pending_clear: false,
        }
    }

    /// Create a grep mode state with restored results (for resume)
    pub fn with_results(
        search_root: PathBuf,
        query: String,
        matches: Vec<GrepMatch>,
        selected: usize,
        scroll_offset: usize,
        show_hidden: bool,
    ) -> Self {
        Self {
            query,
            matches,
            selected,
            scroll_offset,
            results_height: 10,
            search_root,
            grep_receiver: None,
            searching: false,
            show_hidden,
            pending_clear: false,
        }
    }

    /// Start a new grep search with the current query
    pub fn execute_search(&mut self) {
        if self.query.is_empty() {
            self.matches.clear();
            self.searching = false;
            self.grep_receiver = None;
            self.pending_clear = false;
            return;
        }

        // Don't clear matches immediately - wait for first result to reduce flicker
        self.pending_clear = true;

        let (tx, rx) = mpsc::channel();
        self.grep_receiver = Some(rx);
        self.searching = true;

        let query = self.query.clone();
        let root = self.search_root.clone();
        let show_hidden = self.show_hidden;

        thread::spawn(move || {
            run_ripgrep(&query, &root, show_hidden, tx);
        });
    }

    /// Poll for new grep results
    pub fn poll_results(&mut self) -> bool {
        let receiver = match &self.grep_receiver {
            Some(r) => r,
            None => {
                // No active search - if pending_clear with no receiver, clear now
                if self.pending_clear {
                    self.matches.clear();
                    self.selected = 0;
                    self.scroll_offset = 0;
                    self.pending_clear = false;
                }
                return false;
            }
        };

        let mut added = 0;
        let batch_size = 1000;

        loop {
            if added >= batch_size {
                break;
            }

            match receiver.try_recv() {
                Ok(result) => {
                    // Clear old results on first new result to reduce flicker
                    if self.pending_clear {
                        self.matches.clear();
                        self.selected = 0;
                        self.scroll_offset = 0;
                        self.pending_clear = false;
                    }
                    self.matches.push(result);
                    added += 1;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.searching = false;
                    self.grep_receiver = None;
                    // If search finished with no results, clear now
                    if self.pending_clear {
                        self.matches.clear();
                        self.selected = 0;
                        self.scroll_offset = 0;
                        self.pending_clear = false;
                    }
                    break;
                }
            }
        }

        added > 0
    }

    /// Append a character to the query and auto-search
    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.auto_search();
    }

    /// Remove the last character from the query and auto-search
    pub fn pop_char(&mut self) {
        self.query.pop();
        self.auto_search();
    }

    /// Delete the last word from the query and auto-search
    pub fn delete_word(&mut self) {
        while self.query.ends_with(' ') {
            self.query.pop();
        }
        while !self.query.is_empty() && !self.query.ends_with(' ') {
            self.query.pop();
        }
        self.auto_search();
    }

    /// Clear the query and results
    pub fn clear(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        self.searching = false;
        self.grep_receiver = None;
        self.pending_clear = false;
    }

    /// Auto-search if query is not empty
    fn auto_search(&mut self) {
        if !self.query.is_empty() {
            self.execute_search();
        } else {
            // Clear results if query is empty
            self.matches.clear();
            self.selected = 0;
            self.scroll_offset = 0;
            self.searching = false;
            self.grep_receiver = None;
        }
    }

    /// Move selection by delta, optionally wrapping around at boundaries
    pub fn move_selection(&mut self, delta: isize, wrap: bool) {
        let count = self.matches.len();
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

        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + self.results_height {
            self.scroll_offset = self.selected - self.results_height + 1;
        }
    }

    /// Get the currently selected match
    pub fn selected_match(&self) -> Option<&GrepMatch> {
        self.matches.get(self.selected)
    }

    /// Get visible results for rendering
    pub fn visible_results(&self) -> Vec<(usize, &GrepMatch)> {
        let start = self.scroll_offset;
        let end = (start + self.results_height).min(self.matches.len());

        (start..end).map(|i| (i, &self.matches[i])).collect()
    }
}

/// Run ripgrep and send results to channel
fn run_ripgrep(pattern: &str, root: &PathBuf, show_hidden: bool, tx: mpsc::Sender<GrepMatch>) {
    let mut cmd = Command::new("rg");
    cmd.arg("--line-number")
        .arg("--column") // Include column number
        .arg("--with-filename")
        .arg("--no-heading")
        .arg("--color=never")
        .arg("--smart-case") // Case insensitive unless pattern has uppercase
        .arg("--max-count=100") // Limit matches per file
        .arg("--max-columns=500") // Limit line length
        .arg("-m")
        .arg("5000") // Total match limit
        .arg("-e")
        .arg(pattern);

    if show_hidden {
        cmd.arg("--hidden");
    }

    // Use Peak File Manager, legacy, and generic ignore files when present.
    for name in [IGNORE_FILE_NAME, LEGACY_IGNORE_FILE_NAME, ".ignore"] {
        let ignore_path = root.join(name);
        if ignore_path.exists() {
            // Remove Windows extended-length path prefix for ripgrep compatibility
            let ignore_str = ignore_path.to_string_lossy().to_string();
            #[cfg(target_os = "windows")]
            let ignore_str = ignore_str
                .strip_prefix(r"\\?\")
                .map(str::to_string)
                .unwrap_or(ignore_str);
            cmd.arg("--ignore-file").arg(&ignore_str);
        }
    }

    // Remove Windows extended-length path prefix for ripgrep compatibility
    let root_str = root.to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    let root_str = root_str
        .strip_prefix(r"\\?\")
        .map(str::to_string)
        .unwrap_or(root_str);

    cmd.arg(&root_str);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return,
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return,
    };

    let reader = BufReader::new(stdout);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        // Parse ripgrep output: path:line_num:content
        if let Some(result) = parse_rg_line(&line, root) {
            if tx.send(result).is_err() {
                break; // Receiver dropped
            }
        }
    }
}

/// Parse a ripgrep output line
fn parse_rg_line(line: &str, root: &PathBuf) -> Option<GrepMatch> {
    // Format with --column: path:line_num:col_num:content
    // On Windows, paths like "C:\path\file.rs:10:5:content" have a colon in the drive letter

    #[cfg(target_os = "windows")]
    let (path_str, rest) = {
        // Check if line starts with drive letter (e.g., "C:")
        if line.len() > 2
            && line.chars().nth(1) == Some(':')
            && line
                .chars()
                .next()
                .map(|c| c.is_ascii_alphabetic())
                .unwrap_or(false)
        {
            // Find the next colon after the drive letter
            if let Some(pos) = line[2..].find(':') {
                let path_end = pos + 2;
                (&line[..path_end], &line[path_end + 1..])
            } else {
                return None;
            }
        } else {
            // No drive letter, split normally
            if let Some(pos) = line.find(':') {
                (&line[..pos], &line[pos + 1..])
            } else {
                return None;
            }
        }
    };

    #[cfg(not(target_os = "windows"))]
    let (path_str, rest) = {
        if let Some(pos) = line.find(':') {
            (&line[..pos], &line[pos + 1..])
        } else {
            return None;
        }
    };

    // Now parse rest as line_num:col_num:content
    let mut parts = rest.splitn(3, ':');
    let line_num_str = parts.next()?;
    let col_num_str = parts.next()?;
    let content = parts.next().unwrap_or("");

    let path = PathBuf::from(path_str);
    let line_num: usize = line_num_str.parse().ok()?;
    let col_num: usize = col_num_str.parse().ok()?;

    // Create a clean root path without Windows extended-length prefix for comparison
    #[cfg(target_os = "windows")]
    let clean_root = {
        let root_str = root.to_string_lossy();
        match root_str.strip_prefix(r"\\?\") {
            Some(stripped) => PathBuf::from(stripped),
            None => root.clone(),
        }
    };
    #[cfg(not(target_os = "windows"))]
    let clean_root = root.clone();

    // Make path relative to root for display
    let display_path = path
        .strip_prefix(&clean_root)
        .unwrap_or(&path)
        .to_path_buf();

    Some(GrepMatch {
        path: display_path,
        line_num,
        col_num,
        line: content.to_string(),
    })
}
