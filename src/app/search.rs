//! In-buffer search functionality (/? patterns)

use crate::input::mode::{Mode, SearchDirection};

use super::App;

impl App {
    pub fn start_search(&mut self, direction: SearchDirection) {
        self.search_query.clear();
        self.search_direction = direction;
        self.mode = Mode::Search(direction);
        self.status = match direction {
            SearchDirection::Forward => "/".to_string(),
            SearchDirection::Backward => "?".to_string(),
        };
    }

    pub fn search_push_char(&mut self, c: char) {
        self.search_query.push(c);
        let prefix = match self.search_direction {
            SearchDirection::Forward => "/",
            SearchDirection::Backward => "?",
        };
        self.status = format!("{}{}", prefix, self.search_query);
        // Live search - jump to first match as you type (respects direction)
        match self.search_direction {
            SearchDirection::Forward => self.search_forward(true),
            SearchDirection::Backward => self.search_backward(true),
        }
    }

    pub fn search_pop_char(&mut self) {
        self.search_query.pop();
        let prefix = match self.search_direction {
            SearchDirection::Forward => "/",
            SearchDirection::Backward => "?",
        };
        self.status = format!("{}{}", prefix, self.search_query);
        if !self.search_query.is_empty() {
            match self.search_direction {
                SearchDirection::Forward => self.search_forward(true),
                SearchDirection::Backward => self.search_backward(true),
            }
        }
    }

    pub fn confirm_search(&mut self) {
        self.mode = Mode::Normal;
        if self.search_query.is_empty() {
            self.status.clear();
        }
    }

    pub fn cancel_search(&mut self) {
        self.search_query.clear();
        self.mode = Mode::Normal;
        self.status.clear();
    }

    pub fn search_next(&mut self) {
        // n - go in the same direction as the original search
        if !self.search_query.is_empty() {
            match self.search_direction {
                SearchDirection::Forward => self.search_forward(false),
                SearchDirection::Backward => self.search_backward(false),
            }
        }
    }

    pub fn search_prev(&mut self) {
        // N - go in the opposite direction of the original search
        if !self.search_query.is_empty() {
            match self.search_direction {
                SearchDirection::Forward => self.search_backward(false),
                SearchDirection::Backward => self.search_forward(false),
            }
        }
    }

    fn search_forward(&mut self, from_current: bool) {
        let query = self.search_query.to_lowercase();
        let len = self.current.buffer.len();
        if len == 0 || query.is_empty() {
            return;
        }

        let start = if from_current {
            self.current.cursor
        } else {
            (self.current.cursor + 1) % len
        };

        let prefix = match self.search_direction {
            SearchDirection::Forward => "/",
            SearchDirection::Backward => "?",
        };

        // Search forward from start
        for i in 0..len {
            let idx = (start + i) % len;
            if let Some(line) = self.current.buffer.get_line(idx) {
                if line.text.to_lowercase().contains(&query) {
                    self.current.set_cursor(idx);
                    self.refresh_preview();
                    self.status = format!("{}{}", prefix, self.search_query);
                    return;
                }
            }
        }

        self.status = format!("{}{} [No match]", prefix, self.search_query);
    }

    fn search_backward(&mut self, from_current: bool) {
        let query = self.search_query.to_lowercase();
        let len = self.current.buffer.len();
        if len == 0 || query.is_empty() {
            return;
        }

        let start = if from_current {
            self.current.cursor
        } else if self.current.cursor == 0 {
            len - 1
        } else {
            self.current.cursor - 1
        };

        let prefix = match self.search_direction {
            SearchDirection::Forward => "/",
            SearchDirection::Backward => "?",
        };

        // Search backward from start
        for i in 0..len {
            let idx = (start + len - i) % len;
            if let Some(line) = self.current.buffer.get_line(idx) {
                if line.text.to_lowercase().contains(&query) {
                    self.current.set_cursor(idx);
                    self.refresh_preview();
                    self.status = format!("{}{}", prefix, self.search_query);
                    return;
                }
            }
        }

        self.status = format!("{}{} [No match]", prefix, self.search_query);
    }
}
