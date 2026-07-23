use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;

use ignore::{WalkBuilder, WalkState};

use crate::core::search::SearchEntry;
use crate::core::{Entry, EntryId, EntryKind};
use crate::paths::{
    IGNORE_FILE_NAME, LEGACY_IGNORE_FILE_NAME, TRASH_INDEX_FILE_NAME, TRASH_LOCK_FILE_NAME,
};

const MAX_SEARCH_DEPTH: usize = 10;

fn is_internal_bookkeeping_name(name: &str) -> bool {
    matches!(name, TRASH_INDEX_FILE_NAME | TRASH_LOCK_FILE_NAME)
}

/// Get file mode/permissions from metadata
#[cfg(unix)]
fn get_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode())
}

/// Get file mode/permissions from metadata (Windows stub)
#[cfg(windows)]
fn get_mode(metadata: &fs::Metadata) -> Option<u32> {
    // On Windows, we could read attributes but for now just return read-only status
    let readonly = metadata.permissions().readonly();
    Some(if readonly { 0o444 } else { 0o666 })
}

pub fn read_dir_filtered(path: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        let file_type = entry.file_type()?;

        let name = entry.file_name().to_string_lossy().into_owned();

        if is_internal_bookkeeping_name(&name) {
            continue;
        }

        // Skip hidden files if show_hidden is false
        if !show_hidden && name.starts_with('.') {
            continue;
        }

        let kind = if file_type.is_symlink() {
            let target = fs::read_link(&path).ok();
            EntryKind::Symlink(target.unwrap_or_default())
        } else if file_type.is_dir() {
            EntryKind::Directory
        } else {
            EntryKind::File
        };

        entries.push(Entry {
            id: EntryId::new(),
            name,
            kind,
            path,
            size: Some(metadata.len()),
            modified: metadata.modified().ok(),
            mode: get_mode(&metadata),
        });
    }

    // Sort: directories first, then by name
    entries.sort_by(|a, b| match (a.is_dir(), b.is_dir()) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}

const INITIAL_SCAN_LIMIT: usize = 2_048;

/// Scan a directory, returning initial results synchronously and a receiver for background results.
/// The first 2k files are scanned on the main thread for fast startup.
/// Background scanning uses parallel walking for faster I/O on multi-core systems.
pub fn spawn_recursive_scan(
    root: PathBuf,
    show_hidden: bool,
    recurse_hidden_dirs: bool,
    use_gitignore: bool,
) -> (Vec<SearchEntry>, Option<Receiver<SearchEntry>>) {
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(!show_hidden)
        .git_ignore(use_gitignore)
        .git_global(use_gitignore)
        .git_exclude(use_gitignore)
        .max_depth(Some(MAX_SEARCH_DEPTH));

    builder.add_custom_ignore_filename(IGNORE_FILE_NAME);
    builder.add_custom_ignore_filename(LEGACY_IGNORE_FILE_NAME);
    builder.add_custom_ignore_filename(".ignore");

    let mut walker = builder.build();
    let mut initial_entries = Vec::with_capacity(INITIAL_SCAN_LIMIT);

    // Scan first 1k files synchronously for fast startup
    while initial_entries.len() < INITIAL_SCAN_LIMIT {
        let result = match walker.next() {
            Some(r) => r,
            None => {
                // Finished scanning - no background thread needed
                return (initial_entries, None);
            }
        };

        if let Some(entry) = process_walk_entry(result, &root, show_hidden, recurse_hidden_dirs) {
            initial_entries.push(entry);
        }
    }

    // Check if there might be more files
    let has_more = walker.next().is_some();
    if !has_more {
        return (initial_entries, None);
    }

    // Collect paths of initial entries for deduplication in parallel walker
    let initial_paths: HashSet<PathBuf> = initial_entries.iter().map(|e| e.path.clone()).collect();

    // More files to scan - spawn parallel background walker
    // Use bounded channel for backpressure - scanner blocks if receiver falls behind
    let (tx, rx) = mpsc::sync_channel(20_000);

    thread::spawn(move || {
        // Build fresh parallel walker for background scanning
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(!show_hidden)
            .git_ignore(use_gitignore)
            .git_global(use_gitignore)
            .git_exclude(use_gitignore)
            .max_depth(Some(MAX_SEARCH_DEPTH));
        builder.add_custom_ignore_filename(IGNORE_FILE_NAME);
        builder.add_custom_ignore_filename(LEGACY_IGNORE_FILE_NAME);
        builder.add_custom_ignore_filename(".ignore");

        // Use available CPU cores for parallel I/O
        let num_threads = std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(4);
        builder.threads(num_threads);

        let parallel_walker = builder.build_parallel();

        // Wrap shared state in Arc for parallel access
        let tx = Arc::new(tx);
        let initial_paths = Arc::new(initial_paths);
        let root = Arc::new(root);

        parallel_walker.run(|| {
            let tx = Arc::clone(&tx);
            let initial_paths = Arc::clone(&initial_paths);
            let root = Arc::clone(&root);

            Box::new(move |result| {
                let entry = match result {
                    Ok(e) => e,
                    Err(_) => return WalkState::Continue,
                };

                let path = entry.path().to_path_buf();

                // Skip root directory
                if path == *root {
                    return WalkState::Continue;
                }

                // Skip entries already in initial results
                if initial_paths.contains(&path) {
                    return WalkState::Continue;
                }

                // Process entry - use cached file_type from DirEntry to avoid extra syscall
                let name = match path.file_name() {
                    Some(n) => n.to_string_lossy().into_owned(),
                    None => return WalkState::Continue,
                };
                if is_internal_bookkeeping_name(&name) {
                    return WalkState::Continue;
                }
                let is_hidden = name.starts_with('.');
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

                // Additional hidden directory filtering for recursion
                if is_dir && is_hidden && !recurse_hidden_dirs {
                    let is_in_root = path.parent() == Some(root.as_ref());
                    if !is_in_root {
                        return WalkState::Continue;
                    }
                }

                let display = path
                    .strip_prefix(root.as_ref())
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();

                let search_entry = SearchEntry {
                    path,
                    display,
                    is_dir,
                };

                if tx.send(search_entry).is_err() {
                    return WalkState::Quit;
                }

                WalkState::Continue
            })
        });
    });

    (initial_entries, Some(rx))
}

fn process_walk_entry(
    result: Result<ignore::DirEntry, ignore::Error>,
    root: &Path,
    _show_hidden: bool,
    recurse_hidden_dirs: bool,
) -> Option<SearchEntry> {
    let entry = result.ok()?;
    let path = entry.path().to_path_buf();

    // Skip the root directory itself
    if path == root {
        return None;
    }

    let name = path.file_name()?.to_string_lossy().into_owned();
    if is_internal_bookkeeping_name(&name) {
        return None;
    }
    let is_hidden = name.starts_with('.');
    // Use cached file_type from DirEntry to avoid extra syscall
    let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

    // Additional hidden directory filtering for recursion
    if is_dir && is_hidden && !recurse_hidden_dirs {
        let is_in_root = path.parent() == Some(root);
        if !is_in_root {
            return None;
        }
    }

    let display = path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .into_owned();

    Some(SearchEntry {
        path,
        display,
        is_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn directory_listing_hides_internal_bookkeeping_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(TRASH_INDEX_FILE_NAME), "{}").unwrap();
        fs::write(dir.path().join(TRASH_LOCK_FILE_NAME), "").unwrap();
        fs::write(dir.path().join("visible.txt"), "visible").unwrap();

        let entries = read_dir_filtered(dir.path(), true).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "visible.txt");
    }
}
