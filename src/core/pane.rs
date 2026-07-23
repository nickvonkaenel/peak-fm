use std::path::PathBuf;

use super::buffer::{Buffer, BufferLine};
use super::entry::Entry;

#[derive(Debug, Clone)]
pub struct Pane {
    pub buffer: Buffer,
    pub cursor: usize,
    pub scroll_offset: usize,
    pub height: usize,
}

impl Pane {
    pub fn new(path: PathBuf, entries: Vec<Entry>) -> Self {
        Self {
            buffer: Buffer::new(path, entries),
            cursor: 0,
            scroll_offset: 0,
            height: 20,
        }
    }

    /// Create a pane showing volumes/drives
    pub fn new_volumes(entries: Vec<Entry>) -> Self {
        Self {
            buffer: Buffer::new_volumes(entries),
            cursor: 0,
            scroll_offset: 0,
            height: 20,
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.buffer.len();
        if len == 0 {
            self.cursor = 0;
            return;
        }

        let new_pos = if delta < 0 {
            self.cursor.saturating_sub((-delta) as usize)
        } else {
            self.cursor.saturating_add(delta as usize)
        };
        self.cursor = new_pos.min(len.saturating_sub(1));
        self.ensure_visible();
    }

    pub fn set_cursor(&mut self, pos: usize) {
        let len = self.buffer.len();
        self.cursor = if len == 0 { 0 } else { pos.min(len - 1) };
        self.ensure_visible();
    }

    fn ensure_visible(&mut self) {
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        } else if self.cursor >= self.scroll_offset + self.height {
            self.scroll_offset = self.cursor.saturating_sub(self.height - 1);
        }
    }

    pub fn selected_line(&self) -> Option<&BufferLine> {
        self.buffer.get_line(self.cursor)
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        self.selected_line().map(|line| {
            if self.buffer.is_volumes {
                // For volumes, the text IS the full path
                PathBuf::from(&line.text)
            } else {
                self.buffer.path.join(&line.text)
            }
        })
    }

    pub fn delete_selected(&mut self) {
        self.buffer.delete_line(self.cursor);
        let len = self.buffer.len();
        if self.cursor >= len && len > 0 {
            self.cursor = len - 1;
        }
    }

    pub fn insert_below(&mut self) {
        let idx = if self.buffer.is_empty() {
            0
        } else {
            self.cursor + 1
        };
        self.buffer.insert_line(idx, BufferLine::new_empty());
        self.cursor = idx;
    }

    pub fn insert_above(&mut self) {
        let idx = self.cursor;
        self.buffer.insert_line(idx, BufferLine::new_empty());
        // cursor stays at same index, which is now the new line
    }
}
