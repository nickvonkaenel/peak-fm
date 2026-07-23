use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

static ENTRY_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntryId(u64);

impl EntryId {
    pub fn new() -> Self {
        Self(ENTRY_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for EntryId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink(PathBuf),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Entry {
    pub id: EntryId,
    pub name: String,
    pub kind: EntryKind,
    pub path: PathBuf,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub mode: Option<u32>, // File permissions/mode
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, EntryKind::Directory)
    }

    #[allow(dead_code)]
    pub fn is_symlink(&self) -> bool {
        matches!(self.kind, EntryKind::Symlink(_))
    }
}
