use std::path::PathBuf;
use std::time::SystemTime;

use super::entry::{Entry, EntryId};

#[derive(Debug, Clone)]
pub struct BufferLine {
    pub id: Option<EntryId>,
    pub text: String,
    pub is_dir: bool,
    pub copy_from: Option<PathBuf>, // Source path for copy operations
    pub move_from: Option<PathBuf>, // Source path for move operations (takes precedence over copy_from)
    pub size: Option<u64>,          // File size in bytes
    pub modified: Option<SystemTime>, // Last modified time
    pub mode: Option<u32>,          // File permissions/mode
}

impl BufferLine {
    pub fn from_entry(entry: &Entry) -> Self {
        Self {
            id: Some(entry.id),
            text: entry.name.clone(),
            is_dir: entry.is_dir(),
            copy_from: None,
            move_from: None,
            size: entry.size,
            modified: entry.modified,
            mode: entry.mode,
        }
    }

    pub fn new_empty() -> Self {
        Self {
            id: None,
            text: String::new(),
            is_dir: false,
            copy_from: None,
            move_from: None,
            size: None,
            modified: None,
            mode: None,
        }
    }

    pub fn new_copy(name: String, is_dir: bool, source: PathBuf) -> Self {
        Self {
            id: None,
            text: name,
            is_dir,
            copy_from: Some(source),
            move_from: None,
            size: None,
            modified: None,
            mode: None,
        }
    }

    pub fn new_move(name: String, is_dir: bool, source: PathBuf) -> Self {
        Self {
            id: None,
            text: name,
            is_dir,
            copy_from: None,
            move_from: Some(source),
            size: None,
            modified: None,
            mode: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Buffer {
    pub path: PathBuf,
    pub lines: Vec<BufferLine>,
    pub snapshot: Vec<BufferLine>,
    pub dirty: bool,
    pub edit_cursor: usize, // Cursor position within current line text
    pub is_volumes: bool,   // True if this buffer shows volumes/drives
}

impl Buffer {
    pub fn new(path: PathBuf, entries: Vec<Entry>) -> Self {
        let lines: Vec<BufferLine> = entries.iter().map(BufferLine::from_entry).collect();
        Self {
            path,
            snapshot: lines.clone(),
            lines,
            dirty: false,
            edit_cursor: 0,
            is_volumes: false,
        }
    }

    /// Create a buffer for displaying volumes/drives
    pub fn new_volumes(entries: Vec<Entry>) -> Self {
        let lines: Vec<BufferLine> = entries.iter().map(BufferLine::from_entry).collect();
        Self {
            path: PathBuf::from("/"),
            snapshot: lines.clone(),
            lines,
            dirty: false,
            edit_cursor: 0,
            is_volumes: true,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn get_line(&self, idx: usize) -> Option<&BufferLine> {
        self.lines.get(idx)
    }

    pub fn get_line_mut(&mut self, idx: usize) -> Option<&mut BufferLine> {
        self.lines.get_mut(idx)
    }

    pub fn delete_line(&mut self, idx: usize) {
        if idx < self.lines.len() {
            self.lines.remove(idx);
            self.mark_dirty();
        }
    }

    pub fn insert_line(&mut self, idx: usize, line: BufferLine) {
        let idx = idx.min(self.lines.len());
        self.lines.insert(idx, line);
        self.mark_dirty();
    }
}
