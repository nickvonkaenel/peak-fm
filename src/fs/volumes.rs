//! Cross-platform volume/drive enumeration
//!
//! Provides functionality to list available drives/volumes when at the filesystem root.

use std::path::{Path, PathBuf};

use crate::core::{Entry, EntryId, EntryKind};

/// Check if a path is at the filesystem root
pub fn is_at_root(path: &Path) -> bool {
    #[cfg(windows)]
    {
        // On Windows, root is a drive letter like "C:\"
        // parent() returns None for drive roots
        path.parent().is_none()
    }

    #[cfg(not(windows))]
    {
        path == Path::new("/")
    }
}

/// Get list of available volumes/drives as Entry structs for display in pane
pub fn list_volumes() -> Vec<Entry> {
    enumerate_volumes()
}

/// Platform-specific volume enumeration - macOS
#[cfg(target_os = "macos")]
fn enumerate_volumes() -> Vec<Entry> {
    let mut volumes = Vec::new();

    // Always include root filesystem
    volumes.push(Entry {
        id: EntryId::new(),
        name: "/".to_string(),
        kind: EntryKind::Directory,
        path: PathBuf::from("/"),
        size: None,
        modified: None,
        mode: None,
    });

    // List /Volumes contents
    if let Ok(entries) = std::fs::read_dir("/Volumes") {
        for entry in entries.flatten() {
            let path = entry.path();

            // Skip symlinks that point to root (e.g., "Macintosh HD" -> "/")
            if let Ok(target) = std::fs::read_link(&path) {
                if target == Path::new("/") {
                    continue;
                }
            }

            // Only include directories (mounted volumes)
            if path.is_dir() {
                // Use full path as name so selected_path() works correctly
                let name = path.to_string_lossy().to_string();
                volumes.push(Entry {
                    id: EntryId::new(),
                    name,
                    kind: EntryKind::Directory,
                    path,
                    size: None,
                    modified: None,
                    mode: None,
                });
            }
        }
    }

    volumes
}

/// Platform-specific volume enumeration - Linux
#[cfg(target_os = "linux")]
fn enumerate_volumes() -> Vec<Entry> {
    let mut volumes = Vec::new();

    // Root filesystem
    volumes.push(Entry {
        id: EntryId::new(),
        name: "/".to_string(),
        kind: EntryKind::Directory,
        path: PathBuf::from("/"),
        size: None,
        modified: None,
        mode: None,
    });

    // Home directory
    volumes.push(Entry {
        id: EntryId::new(),
        name: "/home".to_string(),
        kind: EntryKind::Directory,
        path: PathBuf::from("/home"),
        size: None,
        modified: None,
        mode: None,
    });

    // Check /mnt for mounted drives
    if let Ok(entries) = std::fs::read_dir("/mnt") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = format!("/mnt/{}", entry.file_name().to_string_lossy());
                volumes.push(Entry {
                    id: EntryId::new(),
                    name,
                    kind: EntryKind::Directory,
                    path,
                    size: None,
                    modified: None,
                    mode: None,
                });
            }
        }
    }

    // Check /media/$USER for user-mounted media
    if let Ok(user) = std::env::var("USER") {
        let media_path = PathBuf::from("/media").join(&user);
        if let Ok(entries) = std::fs::read_dir(&media_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = format!("/media/{}/{}", user, entry.file_name().to_string_lossy());
                    volumes.push(Entry {
                        id: EntryId::new(),
                        name,
                        kind: EntryKind::Directory,
                        path,
                        size: None,
                        modified: None,
                        mode: None,
                    });
                }
            }
        }
    }

    volumes
}

/// Platform-specific volume enumeration - Windows
#[cfg(target_os = "windows")]
fn enumerate_volumes() -> Vec<Entry> {
    let mut volumes = Vec::new();

    // Check all possible drive letters A-Z
    for letter in b'A'..=b'Z' {
        let drive_letter = letter as char;
        let drive_path = format!("{}:\\", drive_letter);
        let path = PathBuf::from(&drive_path);

        if path.exists() {
            // Use full path with backslash as name - "C:" alone refers to
            // current directory on that drive, not the root
            volumes.push(Entry {
                id: EntryId::new(),
                name: drive_path.clone(),
                kind: EntryKind::Directory,
                path,
                size: None,
                modified: None,
                mode: None,
            });
        }
    }

    volumes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_at_root_unix() {
        #[cfg(not(windows))]
        {
            assert!(is_at_root(Path::new("/")));
            assert!(!is_at_root(Path::new("/home")));
            assert!(!is_at_root(Path::new("/home/user")));
        }
    }

    #[test]
    fn test_list_volumes_not_empty() {
        let volumes = list_volumes();
        assert!(
            !volumes.is_empty(),
            "Should have at least one volume (root)"
        );
    }
}
