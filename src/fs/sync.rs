use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::FsOperation;
#[cfg(not(test))]
use crate::paths::APP_DIR_NAME;
use crate::paths::{TRASH_INDEX_FILE_NAME, TRASH_LOCK_FILE_NAME};

/// Name of the hidden bookkeeping file inside the trash that maps each trashed
/// entry to the absolute path it was deleted from, so it can be restored.
const TRASH_INDEX_NAME: &str = TRASH_INDEX_FILE_NAME;
const TRASH_LOCK_NAME: &str = TRASH_LOCK_FILE_NAME;

/// Serializes read-modify-write access to the trash origin index. Trashing,
/// restoring and emptying all load the index, mutate it and write it back; if
/// two of those interleave (e.g. deleting several files at once) one save can
/// clobber the other's changes (lost update). Holding this lock across each
/// whole load→mutate→save keeps threads in this process ordered. A file lock
/// below extends the same protection across multiple Peak processes.
static TRASH_INDEX_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// Maximum depth for recursive directory operations (prevents stack overflow)
const MAX_RECURSION_DEPTH: usize = 50;

/// Protected system paths that cannot be deleted (Unix/Linux/macOS)
#[cfg(not(windows))]
const PROTECTED_PATHS: &[&str] = &[
    "/",
    "/bin",
    "/sbin",
    "/usr",
    "/etc",
    "/var",
    "/tmp",
    "/dev",
    "/proc",
    "/sys",
    "/lib",
    "/lib64",
    "/boot",
    "/root",
    "/home",
    "/opt",
    // macOS specific
    "/System",
    "/Library",
    "/Applications",
    "/Users",
    "/Volumes",
    "/private",
    "/cores",
];

/// Protected system paths that cannot be deleted (Windows)
/// Note: These are checked case-insensitively
#[cfg(windows)]
const PROTECTED_PATHS: &[&str] = &[
    "C:\\",
    "C:\\Windows",
    "C:\\Windows\\System32",
    "C:\\Windows\\SysWOW64",
    "C:\\Program Files",
    "C:\\Program Files (x86)",
    "C:\\Users",
    "C:\\ProgramData",
    "C:\\Recovery",
    "C:\\$Recycle.Bin",
];

/// Get the trash directory path
#[cfg(not(test))]
pub fn trash_dir() -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        // On Windows, use LOCALAPPDATA or USERPROFILE
        let base = std::env::var("LOCALAPPDATA")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map_err(|_| {
                io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA/USERPROFILE not set")
            })?;
        let trash_path = PathBuf::from(base).join(APP_DIR_NAME).join("trash");
        fs::create_dir_all(&trash_path)?;
        Ok(trash_path)
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var("HOME")
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "HOME not set"))?;
        let trash_path = PathBuf::from(home)
            .join(".local")
            .join("share")
            .join(APP_DIR_NAME)
            .join("trash");
        fs::create_dir_all(&trash_path)?;
        Ok(trash_path)
    }
}

/// Unit tests use a thread-local temporary trash so they never touch a user's
/// real files and parallel tests cannot interfere with one another.
#[cfg(test)]
pub fn trash_dir() -> io::Result<PathBuf> {
    thread_local! {
        static TEST_TRASH_ROOT: std::cell::RefCell<Option<tempfile::TempDir>> =
            const { std::cell::RefCell::new(None) };
    }

    TEST_TRASH_ROOT.with(|root| {
        let mut root = root.borrow_mut();
        if root.is_none() {
            *root = Some(tempfile::tempdir()?);
        }
        let trash_path = root
            .as_ref()
            .expect("test trash root was initialized")
            .path()
            .join("trash");
        fs::create_dir_all(&trash_path)?;
        Ok(trash_path)
    })
}

/// Check if a path is protected
fn is_protected_path(path: &Path) -> bool {
    // Check the path itself
    if is_protected_path_str(path) {
        return true;
    }

    // Also check what symlinks resolve to (if applicable)
    if let Ok(resolved) = path.canonicalize() {
        if resolved != path && is_protected_path_str(&resolved) {
            return true;
        }
    }

    false
}

/// Internal helper to check path strings against protected list
fn is_protected_path_str(path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    #[cfg(windows)]
    let path_str = path_str.to_uppercase();

    // Check exact matches
    for protected in PROTECTED_PATHS {
        #[cfg(windows)]
        let protected_cmp = protected.to_uppercase();
        #[cfg(not(windows))]
        let protected_cmp = protected.to_string();

        if path_str == protected_cmp || path_str == format!("{}\\", protected_cmp) {
            return true;
        }
    }

    // Check if it's a direct child of protected directories
    if let Some(parent) = path.parent() {
        let parent_str = parent.to_string_lossy();
        #[cfg(windows)]
        let parent_str = parent_str.to_uppercase();

        for protected in PROTECTED_PATHS {
            #[cfg(windows)]
            let protected_cmp = protected.to_uppercase();
            #[cfg(not(windows))]
            let protected_cmp = protected.to_string();

            if parent_str == protected_cmp {
                // Allow deleting in temp directories
                #[cfg(not(windows))]
                if *protected == "/tmp" {
                    return false;
                }
                return true;
            }
        }
    }

    // Windows: also protect drive roots (D:\, E:\, etc.)
    #[cfg(windows)]
    {
        let path_upper = path.to_string_lossy().to_uppercase();
        // Check if it's a drive root like "D:\" or "D:"
        if path_upper.len() <= 3
            && path_upper
                .chars()
                .next()
                .map(|c| c.is_ascii_alphabetic())
                .unwrap_or(false)
            && (path_upper.ends_with(":\\") || path_upper.ends_with(":"))
        {
            return true;
        }
    }

    false
}

/// Check if a path contains dangerous traversal patterns
fn has_path_traversal(path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    // Check for parent directory traversal
    if path_str.contains("..") {
        return true;
    }

    // Check for null bytes (could be used to bypass checks)
    if path_str.contains('\0') {
        return true;
    }

    false
}

/// Check if path is a symlink
fn is_symlink(path: &Path) -> bool {
    path.symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Move a file or directory, falling back to copy + delete for cross-device
/// moves (external drives, different filesystems).
fn move_path(src: &Path, dst: &Path) -> io::Result<()> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device_error(&e) => {
            if src.is_dir() {
                copy_dir_recursive(src, dst)?;
                fs::remove_dir_all(src)?;
            } else {
                fs::copy(src, dst)?;
                fs::remove_file(src)?;
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Move a file/directory to trash instead of deleting
/// Falls back to copy + delete for cross-device moves (external drives)
fn move_to_trash(path: &Path) -> io::Result<()> {
    let original = absolute_path(path);

    // Lock the complete move-and-index operation so another Peak process
    // cannot restore or empty the entry between those two steps.
    with_trash_index_lock(|| {
        let trash = trash_dir()?;

        // Include a random process-independent nonce so simultaneous Peak
        // processes cannot choose the same destination.
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let nonce = rand::random::<u128>();

        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let trash_name = format!("{}-{}_{}", timestamp, nonce, file_name);
        let trash_path = trash.join(&trash_name);
        move_path(path, &trash_path)?;

        // Remember where this came from so it can be restored later.
        // Best-effort: a failure to update the index must not fail a move that
        // has already succeeded.
        let mut index = load_trash_index();
        index.insert(trash_name, original.to_string_lossy().to_string());
        let _ = save_trash_index(&index);
        Ok(())
    })
}

/// Make `path` absolute without requiring it to exist (canonicalize won't work
/// once the file has been moved away).
fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(path)
    } else {
        path.to_path_buf()
    }
}

fn trash_index_path() -> io::Result<PathBuf> {
    Ok(trash_dir()?.join(TRASH_INDEX_NAME))
}

fn with_trash_index_lock<T>(operation: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    let _thread_guard = TRASH_INDEX_LOCK.lock();
    let lock_path = trash_dir()?.join(TRASH_LOCK_NAME);
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    fs2::FileExt::lock_exclusive(&lock_file)?;

    let result = operation();
    match result {
        Ok(value) => {
            fs2::FileExt::unlock(&lock_file)?;
            Ok(value)
        }
        Err(error) => {
            let _ = fs2::FileExt::unlock(&lock_file);
            Err(error)
        }
    }
}

/// Load the trash origin index (trash entry name -> original absolute path).
/// Any error is treated as an empty index — the index is an optimization, not
/// a source of truth.
fn load_trash_index() -> HashMap<String, String> {
    trash_index_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_trash_index(index: &HashMap<String, String>) -> io::Result<()> {
    let path = trash_index_path()?;
    let data = serde_json::to_string(index).map_err(io::Error::other)?;
    // Write atomically so a crash mid-write can't leave invalid JSON, which
    // would make the whole index (every recorded origin) unreadable.
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, data.as_bytes()).is_ok() && fs::rename(&tmp, &path).is_ok() {
        return Ok(());
    }
    let _ = fs::remove_file(&tmp);
    fs::write(path, data)
}

/// Strip a numeric timestamp/nonce prefix added when trashing, so the original
/// file name can be recovered when no origin was recorded.
fn strip_trash_prefix(name: &str) -> String {
    if let Some(pos) = name.find('_') {
        let prefix = &name[..pos];
        let is_generated_prefix = prefix
            .split('-')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
        if pos > 0 && is_generated_prefix {
            return name[pos + 1..].to_string();
        }
    }
    name.to_string()
}

/// Return `target` if free, otherwise append ` (restored)`, ` (restored 2)`, …
/// before the extension until an unused path is found, so restoring never
/// clobbers an existing file.
fn unique_destination(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }
    let parent = target.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = target.extension().map(|e| e.to_string_lossy().to_string());
    for n in 1..10_000 {
        let suffix = if n == 1 {
            " (restored)".to_string()
        } else {
            format!(" (restored {})", n)
        };
        let name = match &ext {
            Some(ext) => format!("{}{}.{}", stem, suffix, ext),
            None => format!("{}{}", stem, suffix),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    target.to_path_buf()
}

/// Restore a trashed entry to its original location (or into `fallback_dir`
/// when the origin is unknown — e.g. files trashed before origins were
/// recorded). Returns the path the entry was restored to.
pub fn restore_from_trash(entry: &Path, fallback_dir: &Path) -> io::Result<PathBuf> {
    let trash_name = entry
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid trash entry"))?;

    // Bookkeeping files are not restorable entries.
    if matches!(trash_name.as_str(), TRASH_INDEX_NAME | TRASH_LOCK_NAME) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a restorable entry",
        ));
    }

    // Hold the process-wide file lock across the whole restore so a concurrent
    // Peak process cannot lose this entry's origin or clobber our removal.
    with_trash_index_lock(|| {
        let mut index = load_trash_index();
        let target = match index.get(&trash_name) {
            Some(orig) => PathBuf::from(orig),
            None => fallback_dir.join(strip_trash_prefix(&trash_name)),
        };

        // The original directory may have been removed since the delete.
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        let dest = unique_destination(&target);
        move_path(entry, &dest)?;

        // Drop the bookkeeping entry now that it has been restored.
        if index.remove(&trash_name).is_some() {
            let _ = save_trash_index(&index);
        }

        Ok(dest)
    })
}

/// Check if an error is a cross-device link error
#[cfg(unix)]
fn is_cross_device_error(e: &io::Error) -> bool {
    // EXDEV = 18 on most Unix systems
    e.raw_os_error() == Some(libc::EXDEV)
}

#[cfg(windows)]
fn is_cross_device_error(e: &io::Error) -> bool {
    // ERROR_NOT_SAME_DEVICE = 17
    e.raw_os_error() == Some(17)
}

/// Validate global operations with cross-directory conflict detection
/// This checks for operations that would target paths inside deleted directories
pub fn validate_global_operations(ops: &[FsOperation]) -> Result<(), String> {
    // First run standard validation
    validate_operations(ops)?;

    // Collect all directories that will be deleted
    let deleted_dirs: Vec<PathBuf> = ops
        .iter()
        .filter_map(|op| {
            if let FsOperation::Delete { path } = op {
                if path.is_dir() {
                    Some(path.clone())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Check if any Create/Rename/Copy operation targets a path inside a deleted directory
    for op in ops {
        let target_path = match op {
            FsOperation::Create { path, .. } => Some(path),
            FsOperation::Rename { to, .. } => Some(to),
            FsOperation::Copy { to, .. } => Some(to),
            FsOperation::Delete { .. } => None,
        };

        if let Some(target) = target_path {
            // Check if this target is inside any deleted directory
            for deleted_dir in &deleted_dirs {
                // Use starts_with to check if target is inside deleted_dir
                // But exclude exact matches (deleting and creating the same path is already validated)
                if target != deleted_dir && target.starts_with(deleted_dir) {
                    let dir_name = deleted_dir
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| deleted_dir.display().to_string());
                    let target_name = target
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| target.display().to_string());

                    return Err(format!(
                        "Cannot create '{}': parent directory '{}' is scheduled for deletion",
                        target_name, dir_name
                    ));
                }
            }
        }
    }

    Ok(())
}

/// Validate operations before applying them
pub fn validate_operations(ops: &[FsOperation]) -> Result<(), String> {
    // Collect all paths that will be deleted - these won't conflict with creates/copies
    let pending_deletes: HashSet<PathBuf> = ops
        .iter()
        .filter_map(|op| {
            if let FsOperation::Delete { path } = op {
                Some(path.clone())
            } else {
                None
            }
        })
        .collect();

    for op in ops {
        match op {
            FsOperation::Create { path, is_dir } => {
                // Check for path traversal
                if has_path_traversal(path) {
                    return Err("Cannot create: path contains invalid characters".to_string());
                }

                // Strip trailing slash for path checking
                let check_path = path.to_string_lossy();
                let check_path = check_path.trim_end_matches('/');
                let check_path = Path::new(check_path);

                // Skip existence check if this path will be deleted first
                if check_path.exists() && !pending_deletes.contains(check_path) {
                    let name = check_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.display().to_string());

                    if check_path.is_dir() && !*is_dir {
                        return Err(format!(
                            "Cannot create file '{}': a directory with that name exists",
                            name
                        ));
                    } else if check_path.is_file() && *is_dir {
                        return Err(format!(
                            "Cannot create directory '{}': a file with that name exists",
                            name
                        ));
                    } else {
                        return Err(format!("'{}' already exists", name));
                    }
                }
            }
            FsOperation::Copy { from, to, is_dir } => {
                // Check for path traversal
                if has_path_traversal(to) {
                    return Err("Cannot copy: destination contains invalid characters".to_string());
                }

                // Check if trying to copy a directory into itself
                if *is_dir {
                    if let (Ok(from_canon), Some(to_parent)) = (
                        from.canonicalize(),
                        to.parent().and_then(|p| p.canonicalize().ok()),
                    ) {
                        if to_parent.starts_with(&from_canon) {
                            return Err("Cannot copy directory into itself".to_string());
                        }
                    }
                }

                let check_path = to.to_string_lossy();
                let check_path = check_path.trim_end_matches('/');
                let check_path = Path::new(check_path);

                // Skip existence check if this path will be deleted first
                if check_path.exists() && !pending_deletes.contains(check_path) {
                    let name = check_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| to.display().to_string());

                    if check_path.is_dir() && !*is_dir {
                        return Err(format!(
                            "Cannot copy file '{}': a directory with that name exists",
                            name
                        ));
                    } else if check_path.is_file() && *is_dir {
                        return Err(format!(
                            "Cannot copy directory '{}': a file with that name exists",
                            name
                        ));
                    } else {
                        return Err(format!("'{}' already exists", name));
                    }
                }
            }
            FsOperation::Rename { from, to } => {
                // Check for path traversal
                if has_path_traversal(to) {
                    return Err(
                        "Cannot rename: destination contains invalid characters".to_string()
                    );
                }

                // Check if source is a symlink pointing to a protected path
                if is_symlink(from) {
                    if let Ok(target) = fs::read_link(from) {
                        if is_protected_path(&target) {
                            return Err(
                                "Cannot rename: symlink points to protected path".to_string()
                            );
                        }
                    }
                }

                // Skip existence check if this path will be deleted first
                if to.exists() && !pending_deletes.contains(to) {
                    let name = to
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| to.display().to_string());
                    return Err(format!("Cannot rename: '{}' already exists", name));
                }
            }
            FsOperation::Delete { path } => {
                if is_protected_path(path) {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.display().to_string());
                    return Err(format!("Cannot delete '{}': protected system path", name));
                }
            }
        }
    }
    Ok(())
}

pub fn apply_operations(ops: &[FsOperation]) -> io::Result<()> {
    // Validate first
    if let Err(e) = validate_operations(ops) {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, e));
    }

    for op in ops {
        match op {
            FsOperation::Delete { path } => {
                if let Err(e) = delete_path(path) {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
                    return Err(io::Error::new(
                        e.kind(),
                        format_file_error("delete", name, &e),
                    ));
                }
            }
            FsOperation::Rename { from, to } => {
                // Create parent directory if it doesn't exist
                if let Some(parent) = to.parent() {
                    if !parent.as_os_str().is_empty() && !parent.exists() {
                        fs::create_dir_all(parent)?;
                    }
                }
                if let Err(e) = fs::rename(from, to) {
                    let name = from.file_name().and_then(|n| n.to_str()).unwrap_or("file");
                    return Err(io::Error::new(
                        e.kind(),
                        format_file_error("rename", name, &e),
                    ));
                }
            }
            FsOperation::Create { path, is_dir } => {
                // Strip trailing slash for actual path operations
                let clean_path = path.to_string_lossy();
                let clean_path = clean_path.trim_end_matches('/');
                let clean_path = Path::new(clean_path);

                let result = if *is_dir {
                    fs::create_dir_all(clean_path)
                } else {
                    if let Some(parent) = clean_path.parent() {
                        if !parent.as_os_str().is_empty() {
                            fs::create_dir_all(parent)?;
                        }
                    }
                    fs::File::create(clean_path).map(|_| ())
                };

                if let Err(e) = result {
                    let name = clean_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("file");
                    return Err(io::Error::new(
                        e.kind(),
                        format_file_error("create", name, &e),
                    ));
                }
            }
            FsOperation::Copy { from, to, is_dir } => {
                // Skip if source and destination are the same (prevents data loss)
                if from == to {
                    continue;
                }
                // Also check canonical paths to catch symlink/relative path cases
                if let (Ok(from_canon), Ok(to_canon)) = (from.canonicalize(), to.canonicalize()) {
                    if from_canon == to_canon {
                        continue;
                    }
                }

                let clean_to = to.to_string_lossy();
                let clean_to = clean_to.trim_end_matches('/');
                let clean_to = Path::new(clean_to);

                let result = if *is_dir {
                    copy_dir_recursive(from, clean_to)
                } else {
                    if let Some(parent) = clean_to.parent() {
                        if !parent.as_os_str().is_empty() {
                            fs::create_dir_all(parent)?;
                        }
                    }
                    fs::copy(from, clean_to).map(|_| ())
                };

                if let Err(e) = result {
                    let name = from.file_name().and_then(|n| n.to_str()).unwrap_or("file");
                    return Err(io::Error::new(
                        e.kind(),
                        format_file_error("copy", name, &e),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Format a user-friendly error message for file operations
fn format_file_error(operation: &str, filename: &str, error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::PermissionDenied => {
            // Check for Windows-specific locked file error
            #[cfg(windows)]
            {
                if let Some(code) = error.raw_os_error() {
                    // ERROR_SHARING_VIOLATION = 32
                    // ERROR_LOCK_VIOLATION = 33
                    if code == 32 || code == 33 {
                        return format!(
                            "Cannot {} '{}': file is locked or in use by another process",
                            operation, filename
                        );
                    }
                }
            }
            format!("Cannot {} '{}': permission denied", operation, filename)
        }
        io::ErrorKind::NotFound => {
            format!("Cannot {} '{}': file not found", operation, filename)
        }
        io::ErrorKind::AlreadyExists => {
            format!("Cannot {} '{}': file already exists", operation, filename)
        }
        _ => {
            format!("Cannot {} '{}': {}", operation, filename, error)
        }
    }
}

fn delete_path(path: &Path) -> io::Result<()> {
    // If already in trash, permanently delete and drop its origin record.
    if is_in_trash(path) {
        let trash = trash_dir()?;
        let trash_canonical = trash.canonicalize().unwrap_or(trash);
        let is_top_level = path
            .parent()
            .map(|parent| {
                parent
                    .canonicalize()
                    .unwrap_or_else(|_| absolute_path(parent))
                    == trash_canonical
            })
            .unwrap_or(false);
        let trash_name = is_top_level
            .then(|| path.file_name())
            .flatten()
            .map(|name| name.to_string_lossy().to_string());

        if trash_name
            .as_deref()
            .is_some_and(|name| matches!(name, TRASH_INDEX_NAME | TRASH_LOCK_NAME))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot delete trash bookkeeping",
            ));
        }

        with_trash_index_lock(|| {
            remove_trash_entry(path)?;
            if let Some(name) = &trash_name {
                let mut index = load_trash_index();
                if index.remove(name).is_some() {
                    let _ = save_trash_index(&index);
                }
            }
            Ok(())
        })
    } else {
        // Move to trash instead of permanent deletion
        // Note: move_to_trash moves the symlink itself, not the target
        move_to_trash(path)
    }
}

/// Permanently remove an entry that lives inside the trash, removing symlinks
/// as the link itself rather than following them.
fn remove_trash_entry(path: &Path) -> io::Result<()> {
    if is_symlink(path) {
        // On Unix, symlinks are removed with remove_file regardless of target.
        #[cfg(unix)]
        return fs::remove_file(path);

        // Windows distinguishes file vs directory symlinks.
        #[cfg(windows)]
        {
            let meta = path.symlink_metadata()?;
            if meta.is_dir() {
                return fs::remove_dir(path);
            } else {
                return fs::remove_file(path);
            }
        }
    }

    // Not a symlink - remove based on actual type
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Check if a path is inside the trash directory
fn is_in_trash(path: &Path) -> bool {
    if let Ok(trash) = trash_dir() {
        if let Ok(canonical_path) = path.canonicalize() {
            if let Ok(canonical_trash) = trash.canonicalize() {
                return canonical_path.starts_with(&canonical_trash);
            }
        }
        // Fallback: check string prefix
        let path_str = path.to_string_lossy();
        let trash_str = trash.to_string_lossy();
        return path_str.starts_with(trash_str.as_ref());
    }
    false
}

/// Empty the trash directory
pub fn empty_trash() -> io::Result<usize> {
    let trash = trash_dir()?;

    // Hold the process-wide file lock: emptying removes the index and entries,
    // and must not race with a concurrent trash or restore operation. Keep the
    // lock file itself in place because deleting a locked file is not portable.
    with_trash_index_lock(|| {
        let mut count = 0;
        for entry in fs::read_dir(&trash)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let is_index = name == TRASH_INDEX_NAME;
            if name == TRASH_LOCK_NAME {
                continue;
            }
            if path.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
            // The origin index is bookkeeping, not a user file — don't count it.
            if !is_index {
                count += 1;
            }
        }

        Ok(count)
    })
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    let mut visited = HashSet::new();
    copy_dir_recursive_impl(src, dst, &mut visited, 0)
}

fn copy_dir_recursive_impl(
    src: &Path,
    dst: &Path,
    visited: &mut HashSet<PathBuf>,
    depth: usize,
) -> io::Result<()> {
    // Prevent excessive recursion
    if depth > MAX_RECURSION_DEPTH {
        return Err(io::Error::other("Maximum directory depth exceeded"));
    }

    // Get canonical path to detect symlink loops
    let canonical = src.canonicalize().unwrap_or_else(|_| src.to_path_buf());
    if visited.contains(&canonical) {
        // Already visited this directory (symlink loop) - skip silently
        return Ok(());
    }
    visited.insert(canonical);

    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        // Handle symlinks specially
        if is_symlink(&src_path) {
            // Copy symlinks as symlinks (preserve the link)
            #[cfg(unix)]
            {
                let target = fs::read_link(&src_path)?;
                std::os::unix::fs::symlink(&target, &dst_path)?;
            }
            #[cfg(windows)]
            {
                let target = fs::read_link(&src_path)?;
                let meta = src_path.symlink_metadata()?;
                if meta.is_dir() {
                    std::os::windows::fs::symlink_dir(&target, &dst_path)?;
                } else {
                    std::os::windows::fs::symlink_file(&target, &dst_path)?;
                }
            }
        } else if src_path.is_dir() {
            copy_dir_recursive_impl(&src_path, &dst_path, visited, depth + 1)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ============ Path traversal detection tests ============

    #[test]
    fn test_path_traversal_detects_parent_dir() {
        assert!(has_path_traversal(Path::new("../etc/passwd")));
        assert!(has_path_traversal(Path::new("foo/../bar")));
        assert!(has_path_traversal(Path::new("foo/bar/..")));
    }

    #[test]
    fn test_path_traversal_detects_null_bytes() {
        assert!(has_path_traversal(Path::new("foo\0bar")));
    }

    #[test]
    fn test_path_traversal_allows_safe_paths() {
        assert!(!has_path_traversal(Path::new("foo/bar")));
        assert!(!has_path_traversal(Path::new("/absolute/path")));
        assert!(!has_path_traversal(Path::new("relative.txt")));
        assert!(!has_path_traversal(Path::new("file.with.dots.txt")));
    }

    // ============ Protected path tests ============

    #[cfg(not(windows))]
    #[test]
    fn test_protected_paths_unix() {
        assert!(is_protected_path(Path::new("/")));
        assert!(is_protected_path(Path::new("/bin")));
        assert!(is_protected_path(Path::new("/usr")));
        assert!(is_protected_path(Path::new("/etc")));
        assert!(is_protected_path(Path::new("/System"))); // macOS
    }

    #[cfg(not(windows))]
    #[test]
    fn test_non_protected_paths_unix() {
        assert!(!is_protected_path(Path::new("/home/user/documents")));
        assert!(!is_protected_path(Path::new("/tmp/foo"))); // /tmp itself is protected, but children aren't
    }

    #[cfg(windows)]
    #[test]
    fn test_protected_paths_windows() {
        assert!(is_protected_path(Path::new("C:\\")));
        assert!(is_protected_path(Path::new("C:\\Windows")));
        assert!(is_protected_path(Path::new("C:\\Windows\\System32")));
    }

    // ============ Validation tests ============

    #[test]
    fn test_validate_create_with_path_traversal() {
        let ops = vec![FsOperation::Create {
            path: PathBuf::from("../evil.txt"),
            is_dir: false,
        }];
        let result = validate_operations(&ops);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid characters"));
    }

    #[test]
    fn test_validate_rename_with_path_traversal() {
        let ops = vec![FsOperation::Rename {
            from: PathBuf::from("safe.txt"),
            to: PathBuf::from("../evil.txt"),
        }];
        let result = validate_operations(&ops);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid characters"));
    }

    #[test]
    fn test_validate_copy_with_path_traversal() {
        let ops = vec![FsOperation::Copy {
            from: PathBuf::from("safe.txt"),
            to: PathBuf::from("../evil.txt"),
            is_dir: false,
        }];
        let result = validate_operations(&ops);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid characters"));
    }

    #[cfg(not(windows))]
    #[test]
    fn test_validate_delete_protected_path() {
        let ops = vec![FsOperation::Delete {
            path: PathBuf::from("/bin"),
        }];
        let result = validate_operations(&ops);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("protected system path"));
    }

    // ============ File operation tests ============

    #[test]
    fn test_create_file() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("new_file.txt");

        let ops = vec![FsOperation::Create {
            path: file_path.clone(),
            is_dir: false,
        }];

        apply_operations(&ops).unwrap();
        assert!(file_path.exists());
        assert!(file_path.is_file());
    }

    #[test]
    fn test_create_directory() {
        let temp = TempDir::new().unwrap();
        let dir_path = temp.path().join("new_dir");

        let ops = vec![FsOperation::Create {
            path: dir_path.clone(),
            is_dir: true,
        }];

        apply_operations(&ops).unwrap();
        assert!(dir_path.exists());
        assert!(dir_path.is_dir());
    }

    #[test]
    fn test_create_nested_directory() {
        let temp = TempDir::new().unwrap();
        let nested_path = temp.path().join("a/b/c");

        let ops = vec![FsOperation::Create {
            path: nested_path.clone(),
            is_dir: true,
        }];

        apply_operations(&ops).unwrap();
        assert!(nested_path.exists());
        assert!(nested_path.is_dir());
    }

    #[test]
    fn test_rename_file() {
        let temp = TempDir::new().unwrap();
        let old_path = temp.path().join("old.txt");
        let new_path = temp.path().join("new.txt");

        fs::write(&old_path, "content").unwrap();

        let ops = vec![FsOperation::Rename {
            from: old_path.clone(),
            to: new_path.clone(),
        }];

        apply_operations(&ops).unwrap();
        assert!(!old_path.exists());
        assert!(new_path.exists());
        assert_eq!(fs::read_to_string(&new_path).unwrap(), "content");
    }

    #[test]
    fn test_copy_file() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("source.txt");
        let dst = temp.path().join("dest.txt");

        fs::write(&src, "content").unwrap();

        let ops = vec![FsOperation::Copy {
            from: src.clone(),
            to: dst.clone(),
            is_dir: false,
        }];

        apply_operations(&ops).unwrap();
        assert!(src.exists());
        assert!(dst.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "content");
    }

    #[test]
    fn test_copy_directory() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("src_dir");
        let dst_dir = temp.path().join("dst_dir");

        fs::create_dir(&src_dir).unwrap();
        fs::write(src_dir.join("file.txt"), "content").unwrap();

        let ops = vec![FsOperation::Copy {
            from: src_dir.clone(),
            to: dst_dir.clone(),
            is_dir: true,
        }];

        apply_operations(&ops).unwrap();
        assert!(src_dir.exists());
        assert!(dst_dir.exists());
        assert!(dst_dir.join("file.txt").exists());
        assert_eq!(
            fs::read_to_string(dst_dir.join("file.txt")).unwrap(),
            "content"
        );
    }

    #[test]
    fn test_validate_create_existing_file() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("existing.txt");
        fs::write(&file_path, "").unwrap();

        let ops = vec![FsOperation::Create {
            path: file_path,
            is_dir: false,
        }];

        let result = validate_operations(&ops);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn test_validate_rename_to_existing() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("source.txt");
        let dst = temp.path().join("existing.txt");
        fs::write(&src, "").unwrap();
        fs::write(&dst, "").unwrap();

        let ops = vec![FsOperation::Rename { from: src, to: dst }];

        let result = validate_operations(&ops);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn test_copy_to_existing_file_fails_validation() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("source.txt");
        let dst = temp.path().join("existing.txt");
        fs::write(&src, "source").unwrap();
        fs::write(&dst, "existing").unwrap();

        let ops = vec![FsOperation::Copy {
            from: src,
            to: dst,
            is_dir: false,
        }];

        // Validation should fail because destination exists
        let result = validate_operations(&ops);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn test_copy_to_existing_file_succeeds_with_pending_delete() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("source.txt");
        let dst = temp.path().join("existing.txt");
        fs::write(&src, "source").unwrap();
        fs::write(&dst, "existing").unwrap();

        // Delete the existing file, then copy to same location
        let ops = vec![
            FsOperation::Delete { path: dst.clone() },
            FsOperation::Copy {
                from: src,
                to: dst,
                is_dir: false,
            },
        ];

        // Validation should pass because destination will be deleted first
        let result = validate_operations(&ops);
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_at_existing_path_succeeds_with_pending_delete() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("existing.txt");
        fs::write(&file_path, "content").unwrap();

        // Delete the existing file, then create at same location
        let ops = vec![
            FsOperation::Delete {
                path: file_path.clone(),
            },
            FsOperation::Create {
                path: file_path,
                is_dir: false,
            },
        ];

        // Validation should pass because path will be deleted first
        let result = validate_operations(&ops);
        assert!(result.is_ok());
    }

    #[test]
    fn test_rename_to_existing_path_succeeds_with_pending_delete() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("source.txt");
        let dst = temp.path().join("existing.txt");
        fs::write(&src, "source").unwrap();
        fs::write(&dst, "existing").unwrap();

        // Delete the existing file, then rename to same location
        let ops = vec![
            FsOperation::Delete { path: dst.clone() },
            FsOperation::Rename { from: src, to: dst },
        ];

        // Validation should pass because destination will be deleted first
        let result = validate_operations(&ops);
        assert!(result.is_ok());
    }

    #[test]
    fn test_apply_copy_with_pending_delete() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("source.txt");
        let dst = temp.path().join("existing.txt");
        fs::write(&src, "new content").unwrap();
        fs::write(&dst, "old content").unwrap();

        // Delete then copy - should replace the file
        let ops = vec![
            FsOperation::Delete { path: dst.clone() },
            FsOperation::Copy {
                from: src.clone(),
                to: dst.clone(),
                is_dir: false,
            },
        ];

        let result = apply_operations(&ops);
        assert!(result.is_ok());
        assert!(dst.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "new content");
        // Source should still exist (it's a copy, not move)
        assert!(src.exists());
    }

    #[test]
    fn test_multiple_deletes_with_paste() {
        let temp = TempDir::new().unwrap();
        let file_a = temp.path().join("file_a.txt");
        let file_b = temp.path().join("file_b.txt");
        fs::write(&file_a, "content a").unwrap();
        fs::write(&file_b, "content b").unwrap();

        // Simulate: delete file A, then delete file B
        // Both files should be deleted (moved to trash)
        let ops = vec![
            FsOperation::Delete {
                path: file_a.clone(),
            },
            FsOperation::Delete {
                path: file_b.clone(),
            },
        ];

        let result = apply_operations(&ops);
        assert!(result.is_ok());

        // Both files should no longer exist in original location
        assert!(!file_a.exists(), "file_a should be deleted");
        assert!(!file_b.exists(), "file_b should be deleted");

        // Verify both are in trash by checking trash contains files with these names
        let trash = trash_dir().unwrap();
        let mut found_a = false;
        let mut found_b = false;

        for entry in fs::read_dir(&trash).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains("file_a.txt") {
                found_a = true;
                assert_eq!(fs::read_to_string(entry.path()).unwrap(), "content a");
            }
            if name.contains("file_b.txt") {
                found_b = true;
                assert_eq!(fs::read_to_string(entry.path()).unwrap(), "content b");
            }
        }

        assert!(found_a, "file_a should be in trash");
        assert!(found_b, "file_b should be in trash");
    }

    // ============ Cross-directory validation tests ============

    #[test]
    fn test_validate_global_operations_delete_dir_then_create_inside() {
        let temp = TempDir::new().unwrap();
        let dir_path = temp.path().join("mydir");
        let file_in_dir = dir_path.join("file.txt");

        fs::create_dir(&dir_path).unwrap();

        // Try to delete directory and create a file inside it
        let ops = vec![
            FsOperation::Delete {
                path: dir_path.clone(),
            },
            FsOperation::Create {
                path: file_in_dir,
                is_dir: false,
            },
        ];

        let result = validate_global_operations(&ops);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("parent directory"));
        assert!(err.contains("scheduled for deletion"));
    }

    #[test]
    fn test_validate_global_operations_delete_dir_then_rename_into() {
        let temp = TempDir::new().unwrap();
        let dir_path = temp.path().join("mydir");
        let src_file = temp.path().join("source.txt");
        let dst_file = dir_path.join("dest.txt");

        fs::create_dir(&dir_path).unwrap();
        fs::write(&src_file, "content").unwrap();

        // Try to delete directory and rename a file into it
        let ops = vec![
            FsOperation::Delete {
                path: dir_path.clone(),
            },
            FsOperation::Rename {
                from: src_file,
                to: dst_file,
            },
        ];

        let result = validate_global_operations(&ops);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("parent directory"));
    }

    #[test]
    fn test_validate_global_operations_delete_dir_then_copy_into() {
        let temp = TempDir::new().unwrap();
        let dir_path = temp.path().join("mydir");
        let src_file = temp.path().join("source.txt");
        let dst_file = dir_path.join("dest.txt");

        fs::create_dir(&dir_path).unwrap();
        fs::write(&src_file, "content").unwrap();

        // Try to delete directory and copy a file into it
        let ops = vec![
            FsOperation::Delete {
                path: dir_path.clone(),
            },
            FsOperation::Copy {
                from: src_file,
                to: dst_file,
                is_dir: false,
            },
        ];

        let result = validate_global_operations(&ops);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("parent directory"));
    }

    #[test]
    fn test_validate_global_operations_delete_and_recreate_same_dir() {
        let temp = TempDir::new().unwrap();
        let dir_path = temp.path().join("mydir");

        fs::create_dir(&dir_path).unwrap();

        // Deleting and recreating the same path should be allowed
        let ops = vec![
            FsOperation::Delete {
                path: dir_path.clone(),
            },
            FsOperation::Create {
                path: dir_path,
                is_dir: true,
            },
        ];

        let result = validate_global_operations(&ops);
        assert!(result.is_ok());
    }

    // ============ Chained operation scenarios ============

    /// Regression: yank file A, rename A → C, paste a copy renamed to B.
    /// The Copy must succeed even though there's also a Rename consuming A.
    /// Topological ordering (Copy before Rename) is what makes this work.
    #[test]
    fn chained_yank_rename_paste_copy_succeeds() {
        let temp = TempDir::new().unwrap();
        let path_a = temp.path().join("A");
        fs::write(&path_a, "content of A").unwrap();

        let mut ops = vec![
            FsOperation::Rename {
                from: path_a.clone(),
                to: temp.path().join("C"),
            },
            FsOperation::Copy {
                from: path_a.clone(),
                to: temp.path().join("B"),
                is_dir: false,
            },
        ];
        crate::core::topological_sort(&mut ops);

        apply_operations(&ops).expect("chained rename + copy should succeed");

        assert!(
            !path_a.exists(),
            "A should no longer exist at original path"
        );
        assert!(temp.path().join("C").exists(), "A should be renamed to C");
        assert!(temp.path().join("B").exists(), "B should be the copy");
        assert_eq!(
            fs::read_to_string(temp.path().join("C")).unwrap(),
            "content of A"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("B")).unwrap(),
            "content of A"
        );
    }

    /// Symmetric case: rename A → C, then copy from C to D (user yanked
    /// after renaming, source path is the new name).
    #[test]
    fn chained_rename_then_copy_from_renamed_succeeds() {
        let temp = TempDir::new().unwrap();
        let path_a = temp.path().join("A");
        fs::write(&path_a, "content").unwrap();

        let mut ops = vec![
            FsOperation::Copy {
                from: temp.path().join("C"),
                to: temp.path().join("D"),
                is_dir: false,
            },
            FsOperation::Rename {
                from: path_a.clone(),
                to: temp.path().join("C"),
            },
        ];
        crate::core::topological_sort(&mut ops);

        apply_operations(&ops).expect("rename then copy-from-new-name should succeed");

        assert!(temp.path().join("C").exists());
        assert!(temp.path().join("D").exists());
        assert_eq!(
            fs::read_to_string(temp.path().join("D")).unwrap(),
            "content"
        );
    }

    /// User deletes a file then creates a new file with the same name — must
    /// be applied in the right order, with validation accepting it via the
    /// pending_deletes set.
    #[test]
    fn delete_then_create_same_name_chain() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("file.txt");
        fs::write(&path, "old").unwrap();

        let ops = vec![
            FsOperation::Delete { path: path.clone() },
            FsOperation::Create {
                path: path.clone(),
                is_dir: false,
            },
        ];

        validate_operations(&ops).expect("delete-then-create should validate");
        apply_operations(&ops).expect("delete-then-create should apply");

        assert!(path.exists());
        // The new file should be empty (Create produces an empty file)
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
    }

    /// Swap rename (A → B, B → A) is fundamentally unsafe without an
    /// intermediate. Confirm we error out cleanly rather than corrupting
    /// either file.
    #[test]
    fn swap_rename_errors_cleanly() {
        let temp = TempDir::new().unwrap();
        let a = temp.path().join("A");
        let b = temp.path().join("B");
        fs::write(&a, "A").unwrap();
        fs::write(&b, "B").unwrap();

        let ops = vec![
            FsOperation::Rename {
                from: a.clone(),
                to: b.clone(),
            },
            FsOperation::Rename {
                from: b.clone(),
                to: a.clone(),
            },
        ];

        let result = validate_operations(&ops);
        assert!(result.is_err(), "validation must reject A↔B swap");

        // Files must be untouched
        assert_eq!(fs::read_to_string(&a).unwrap(), "A");
        assert_eq!(fs::read_to_string(&b).unwrap(), "B");
    }

    /// Delete a directory, rename a file into it (which validation must reject),
    /// and verify nothing is touched on disk.
    #[test]
    fn delete_dir_then_rename_into_it_is_caught_before_apply() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("d");
        let src = temp.path().join("src.txt");
        fs::create_dir(&dir).unwrap();
        fs::write(&src, "x").unwrap();

        let ops = vec![
            FsOperation::Delete { path: dir.clone() },
            FsOperation::Rename {
                from: src.clone(),
                to: dir.join("moved.txt"),
            },
        ];

        let result = validate_global_operations(&ops);
        assert!(result.is_err());
        assert!(
            dir.exists(),
            "directory must not be removed when validation fails"
        );
        assert!(
            src.exists(),
            "source file must not be moved when validation fails"
        );
    }

    /// A chain of multiple deletes + a copy + a create in one batch should
    /// all execute and validate against the pending_deletes set.
    #[test]
    fn mixed_chain_delete_copy_create_executes_in_order() {
        let temp = TempDir::new().unwrap();
        let old1 = temp.path().join("old1");
        let old2 = temp.path().join("old2");
        let src = temp.path().join("src");
        fs::write(&old1, "old1").unwrap();
        fs::write(&old2, "old2").unwrap();
        fs::write(&src, "source").unwrap();

        // Plan: delete old1 and old2, copy src → old1 (taking its place),
        // and create a brand new file 'new'.
        let ops = vec![
            FsOperation::Delete { path: old1.clone() },
            FsOperation::Delete { path: old2.clone() },
            FsOperation::Copy {
                from: src.clone(),
                to: old1.clone(),
                is_dir: false,
            },
            FsOperation::Create {
                path: temp.path().join("new"),
                is_dir: false,
            },
        ];

        validate_operations(&ops).expect("chain should validate");
        apply_operations(&ops).expect("chain should apply");

        assert!(old1.exists(), "old1 was replaced by copy of src");
        assert_eq!(fs::read_to_string(&old1).unwrap(), "source");
        assert!(!old2.exists(), "old2 should be moved to trash");
        assert!(src.exists(), "copy source must be preserved");
        assert!(temp.path().join("new").exists());
    }

    #[test]
    fn test_validate_global_operations_nested_directory_delete() {
        let temp = TempDir::new().unwrap();
        let parent_dir = temp.path().join("parent");
        let child_dir = parent_dir.join("child");
        let file_in_child = child_dir.join("file.txt");

        fs::create_dir_all(&child_dir).unwrap();

        // Delete parent directory and try to create file in nested child
        let ops = vec![
            FsOperation::Delete {
                path: parent_dir.clone(),
            },
            FsOperation::Create {
                path: file_in_child,
                is_dir: false,
            },
        ];

        let result = validate_global_operations(&ops);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("parent directory"));
    }

    // ============ Trash restore tests ============

    #[test]
    fn strip_trash_prefix_removes_timestamp() {
        assert_eq!(strip_trash_prefix("1700000000000_note.txt"), "note.txt");
        assert_eq!(
            strip_trash_prefix("1700000000000-123456_note.txt"),
            "note.txt"
        );
        assert_eq!(strip_trash_prefix("42_a_b.txt"), "a_b.txt");
        // No numeric prefix: leave the name untouched.
        assert_eq!(strip_trash_prefix("note.txt"), "note.txt");
        assert_eq!(strip_trash_prefix("_leading"), "_leading");
    }

    #[test]
    fn unique_destination_avoids_clobbering() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("file.txt");

        // Free path is returned unchanged.
        assert_eq!(unique_destination(&target), target);

        // Occupied path gets a " (restored)" suffix before the extension.
        fs::write(&target, b"x").unwrap();
        let alt = unique_destination(&target);
        assert_eq!(alt, dir.path().join("file (restored).txt"));

        // ...and the next collision bumps the counter.
        fs::write(&alt, b"x").unwrap();
        assert_eq!(
            unique_destination(&target),
            dir.path().join("file (restored 2).txt")
        );
    }

    #[test]
    fn restore_returns_file_to_recorded_origin() {
        let dir = TempDir::new().unwrap();
        // A distinctive name to avoid matching stray entries in the real trash.
        let original = dir.path().join("peak_restore_roundtrip_marker.txt");
        fs::write(&original, b"payload").unwrap();

        // Trash it: the file leaves its original location and its origin is
        // recorded in the index.
        move_to_trash(&original).unwrap();
        assert!(!original.exists(), "file should be gone after trashing");

        // Locate the trashed entry in the real trash directory.
        let trash = trash_dir().unwrap();
        let entry = fs::read_dir(&trash)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| {
                p.file_name()
                    .map(|n| {
                        n.to_string_lossy()
                            .ends_with("_peak_restore_roundtrip_marker.txt")
                    })
                    .unwrap_or(false)
            })
            .expect("trashed entry should be present");

        // Restore: origin is recorded, so the fallback dir is irrelevant.
        let dest = restore_from_trash(&entry, Path::new("/nonexistent")).unwrap();
        assert_eq!(dest, original, "should restore to the recorded origin");
        assert!(original.exists(), "file should be back at its origin");
        assert_eq!(fs::read_to_string(&original).unwrap(), "payload");
        assert!(!entry.exists(), "trash entry should be consumed by restore");
    }

    #[test]
    fn restore_refuses_bookkeeping_files() {
        let trash = trash_dir().unwrap();
        let dir = TempDir::new().unwrap();
        for name in [TRASH_INDEX_NAME, TRASH_LOCK_NAME] {
            let err = restore_from_trash(&trash.join(name), dir.path()).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn permanent_delete_refuses_bookkeeping_files() {
        let trash = trash_dir().unwrap();
        for name in [TRASH_INDEX_NAME, TRASH_LOCK_NAME] {
            let path = trash.join(name);
            fs::write(&path, "").unwrap();
            let err = delete_path(&path).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
            assert!(path.exists());
        }
    }

    #[test]
    fn permanent_delete_prunes_origin_index() {
        let dir = TempDir::new().unwrap();
        let original = dir.path().join("peak_prune_marker_unique.txt");
        fs::write(&original, b"x").unwrap();
        move_to_trash(&original).unwrap();

        let trash = trash_dir().unwrap();
        let entry = fs::read_dir(&trash)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| {
                p.file_name()
                    .map(|n| {
                        n.to_string_lossy()
                            .ends_with("_peak_prune_marker_unique.txt")
                    })
                    .unwrap_or(false)
            })
            .expect("trashed entry should be present");
        let trash_name = entry.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            load_trash_index().contains_key(&trash_name),
            "origin recorded"
        );

        // Permanently deleting it from the trash prunes its index entry.
        delete_path(&entry).unwrap();
        assert!(!entry.exists());
        assert!(
            !load_trash_index().contains_key(&trash_name),
            "origin should be pruned after permanent delete"
        );
    }
}
