use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use super::buffer::Buffer;
use super::{compute_diff, topological_sort, BufferLine, FsOperation};

/// Signature for deduplicating operations
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
struct OperationSignature {
    kind: OperationKind,
    primary_path: PathBuf,
    secondary_path: Option<PathBuf>,
}

#[derive(Hash, Eq, PartialEq, Clone, Copy, Debug)]
enum OperationKind {
    Create,
    Delete,
    Rename,
    Copy,
}

impl OperationSignature {
    fn from_operation(op: &FsOperation) -> Self {
        match op {
            FsOperation::Create { path, .. } => Self {
                kind: OperationKind::Create,
                primary_path: path.clone(),
                secondary_path: None,
            },
            FsOperation::Delete { path } => Self {
                kind: OperationKind::Delete,
                primary_path: path.clone(),
                secondary_path: None,
            },
            FsOperation::Rename { from, to } => Self {
                kind: OperationKind::Rename,
                primary_path: from.clone(),
                secondary_path: Some(to.clone()),
            },
            FsOperation::Copy { from, to, .. } => Self {
                kind: OperationKind::Copy,
                primary_path: from.clone(),
                secondary_path: Some(to.clone()),
            },
        }
    }
}

/// Metadata for tracking operations
#[derive(Clone, Debug)]
struct OperationMetadata {
    origin_dir: PathBuf,
    timestamp: Instant,
}

/// Global store for pending filesystem operations across all directories
#[derive(Default)]
pub struct GlobalOperationStore {
    /// Operations grouped by directory
    operations_by_dir: HashMap<PathBuf, Vec<FsOperation>>,
    /// Metadata for tracking
    metadata: HashMap<OperationSignature, OperationMetadata>,
}

impl GlobalOperationStore {
    pub fn new() -> Self {
        Self {
            operations_by_dir: HashMap::new(),
            metadata: HashMap::new(),
        }
    }

    /// Capture operations from a buffer for a given directory
    pub fn capture_from_buffer(&mut self, dir: &PathBuf, buffer: &Buffer) {
        let ops = compute_diff(buffer);

        if ops.is_empty() {
            // No operations - remove this directory from the store if it exists
            self.operations_by_dir.remove(dir);
            // Clean up metadata for this directory
            self.metadata.retain(|_, meta| &meta.origin_dir != dir);
            return;
        }

        // Store operations for this directory
        self.operations_by_dir.insert(dir.clone(), ops.clone());

        // Update metadata
        let now = Instant::now();
        for op in ops {
            let sig = OperationSignature::from_operation(&op);
            self.metadata.insert(
                sig,
                OperationMetadata {
                    origin_dir: dir.clone(),
                    timestamp: now,
                },
            );
        }
    }

    /// Get all operations across all directories
    /// Resolves conflicts like delete+move becoming just a move
    pub fn all_operations(&self) -> Vec<FsOperation> {
        let mut all_ops = Vec::new();

        // Collect all operations from all directories
        for ops in self.operations_by_dir.values() {
            all_ops.extend(ops.clone());
        }

        // Resolve cross-directory move conflicts:
        // If we have Delete(A/file) and Rename(A/file -> B/file),
        // remove the Delete since Rename will handle the move
        let renames: Vec<PathBuf> = all_ops
            .iter()
            .filter_map(|op| {
                if let FsOperation::Rename { from, .. } = op {
                    Some(from.clone())
                } else {
                    None
                }
            })
            .collect();

        // Filter out Delete operations that have a corresponding Rename
        all_ops.retain(|op| {
            if let FsOperation::Delete { path } = op {
                !renames.contains(path)
            } else {
                true
            }
        });

        // Apply topological sort across all operations to respect dependencies
        // between renames and copies, even when they originate in different directories.
        topological_sort(&mut all_ops);

        all_ops
    }

    /// Get operations for a specific directory
    pub fn operations_for_dir(&self, dir: &PathBuf) -> Vec<FsOperation> {
        self.operations_by_dir.get(dir).cloned().unwrap_or_default()
    }

    /// Get total count of all pending operations
    pub fn total_count(&self) -> usize {
        self.operations_by_dir.values().map(|ops| ops.len()).sum()
    }

    /// Get count of operations for a specific directory
    pub fn count_for_dir(&self, dir: &PathBuf) -> usize {
        self.operations_by_dir
            .get(dir)
            .map(|ops| ops.len())
            .unwrap_or(0)
    }

    /// Clear all pending operations
    pub fn clear(&mut self) {
        self.operations_by_dir.clear();
        self.metadata.clear();
    }

    /// Restore buffer state from pending operations
    /// Reconstructs the edited state by applying pending operations to the fresh snapshot
    pub fn restore_to_buffer(&self, dir: &PathBuf, buffer: &mut Buffer) {
        let ops = self.operations_for_dir(dir);

        if ops.is_empty() {
            return;
        }

        // Start with fresh snapshot from filesystem
        let mut lines = buffer.snapshot.clone();

        // Apply each pending operation to reconstruct edited state
        for op in ops {
            match op {
                FsOperation::Delete { ref path } => {
                    // Remove the line that matches this path
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    lines.retain(|line| line.text != name);
                }
                FsOperation::Create { ref path, is_dir } => {
                    // Add a new empty line
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    let mut line = BufferLine::new_empty();
                    line.text = name;
                    line.is_dir = is_dir;
                    lines.push(line);
                }
                FsOperation::Rename { ref from, ref to } => {
                    let old_name = from.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let new_name = to
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();

                    let from_dir = from.parent();
                    let to_dir = to.parent();
                    let buf_dir = Some(buffer.path.as_path());

                    if from_dir == buf_dir && to_dir == buf_dir {
                        // Same-directory rename: update the existing line in place,
                        // keyed by full path equality (not just filename) so duplicate
                        // names from other operations cannot accidentally match.
                        if let Some(line) = lines.iter_mut().find(|l| l.text == old_name) {
                            line.text = new_name;
                        }
                    } else if to_dir == buf_dir {
                        // Cross-directory move INTO this directory: append a move line.
                        // The source path is preserved verbatim so compute_diff can
                        // round-trip it back to the same Rename operation.
                        let is_dir = from.is_dir();
                        let line = BufferLine::new_move(new_name, is_dir, from.clone());
                        lines.push(line);
                    } else if from_dir == buf_dir {
                        // Cross-directory move OUT of this directory: remove the
                        // source line so the buffer reflects the file leaving.
                        lines.retain(|l| l.text != old_name);
                    }
                    // else: rename doesn't involve this directory — ignore.
                }
                FsOperation::Copy {
                    ref from,
                    ref to,
                    is_dir,
                } => {
                    // Add a copy line
                    let name = to
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    let line = BufferLine::new_copy(name, is_dir, from.clone());
                    lines.push(line);
                }
            }
        }

        // Update buffer with reconstructed state
        buffer.lines = lines;
    }

    /// Get a summary of operations grouped by directory
    pub fn get_summary(&self) -> Vec<(PathBuf, Vec<FsOperation>)> {
        let mut summary: Vec<_> = self
            .operations_by_dir
            .iter()
            .map(|(dir, ops)| (dir.clone(), ops.clone()))
            .collect();

        // Sort by directory path for consistent display
        summary.sort_by(|a, b| a.0.cmp(&b.0));

        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::buffer::{Buffer, BufferLine};
    use crate::core::entry::EntryId;
    use std::path::PathBuf;

    fn line_with_id(name: &str, is_dir: bool) -> BufferLine {
        BufferLine {
            id: Some(EntryId::new()),
            text: name.to_string(),
            is_dir,
            copy_from: None,
            move_from: None,
            size: None,
            modified: None,
            mode: None,
        }
    }

    fn buffer_from(path: &str, entries: &[(&str, bool)]) -> Buffer {
        let lines: Vec<BufferLine> = entries
            .iter()
            .map(|(name, is_dir)| line_with_id(name, *is_dir))
            .collect();
        Buffer {
            path: PathBuf::from(path),
            snapshot: lines.clone(),
            lines,
            dirty: false,
            edit_cursor: 0,
            is_volumes: false,
        }
    }

    #[test]
    fn test_cross_directory_move_resolves_delete() {
        let mut store = GlobalOperationStore::new();
        let dir_a = PathBuf::from("/dir_a");
        let dir_b = PathBuf::from("/dir_b");

        // Simulate: Delete file in dir_a, then move it to dir_b
        // This creates a Delete in dir_a and a Rename in dir_b
        let file_in_a = dir_a.join("file.txt");
        let file_in_b = dir_b.join("file.txt");

        // Manually insert operations (simulating what would happen)
        store.operations_by_dir.insert(
            dir_a.clone(),
            vec![FsOperation::Delete {
                path: file_in_a.clone(),
            }],
        );

        store.operations_by_dir.insert(
            dir_b.clone(),
            vec![FsOperation::Rename {
                from: file_in_a.clone(),
                to: file_in_b.clone(),
            }],
        );

        // Get all operations - should resolve the conflict
        let ops = store.all_operations();

        // Should only have the Rename operation, Delete should be filtered out
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            FsOperation::Rename { from, to } => {
                assert_eq!(from, &file_in_a);
                assert_eq!(to, &file_in_b);
            }
            _ => panic!("Expected Rename operation"),
        }
    }

    /// Regression: when capturing then restoring a buffer for a cross-directory
    /// move INTO a directory that already has a same-named file, the old code
    /// matched by filename alone and silently overwrote the move with a no-op
    /// rename on the unrelated original file.
    #[test]
    fn restore_cross_dir_move_preserves_move_when_dest_dir_has_same_name() {
        let mut store = GlobalOperationStore::new();
        let dir_b = PathBuf::from("/dir_b");
        let move_op = FsOperation::Rename {
            from: PathBuf::from("/dir_a/file.txt"),
            to: dir_b.join("file.txt"),
        };
        store.operations_by_dir.insert(dir_b.clone(), vec![move_op]);

        // Buffer B already contains its own /dir_b/file.txt.
        let mut buf = buffer_from("/dir_b", &[("file.txt", false)]);
        store.restore_to_buffer(&dir_b, &mut buf);

        // After restoring, the buffer should contain BOTH lines: the original
        // file.txt (with id) and the pending move (without id, move_from set).
        assert_eq!(buf.lines.len(), 2);
        let original = buf
            .lines
            .iter()
            .find(|l| l.id.is_some())
            .expect("original line");
        assert_eq!(original.text, "file.txt");

        let move_line = buf
            .lines
            .iter()
            .find(|l| l.move_from.is_some())
            .expect("move line");
        assert_eq!(move_line.text, "file.txt");
        assert_eq!(
            move_line.move_from.as_ref().unwrap(),
            &PathBuf::from("/dir_a/file.txt")
        );

        // And recomputing the diff yields the same Rename — i.e. capture/restore
        // is round-trip stable.
        let ops = compute_diff(&buf);
        assert_eq!(
            ops.len(),
            1,
            "expected a single Rename on round-trip: {:?}",
            ops
        );
        match &ops[0] {
            FsOperation::Rename { from, to } => {
                assert_eq!(from, &PathBuf::from("/dir_a/file.txt"));
                assert_eq!(to, &PathBuf::from("/dir_b/file.txt"));
            }
            other => panic!("expected Rename, got {:?}", other),
        }
    }

    #[test]
    fn restore_same_dir_rename_round_trips() {
        let mut store = GlobalOperationStore::new();
        let dir = PathBuf::from("/dir");
        store.operations_by_dir.insert(
            dir.clone(),
            vec![FsOperation::Rename {
                from: dir.join("a.txt"),
                to: dir.join("b.txt"),
            }],
        );

        let mut buf = buffer_from("/dir", &[("a.txt", false), ("c.txt", false)]);
        store.restore_to_buffer(&dir, &mut buf);

        // The line that originally held "a.txt" now reads "b.txt" (keeping its id).
        let renamed = buf
            .lines
            .iter()
            .find(|l| l.text == "b.txt")
            .expect("renamed line should exist");
        assert!(renamed.id.is_some(), "renamed line should keep its id");

        // Re-computing the diff produces the same rename.
        let ops = compute_diff(&buf);
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            FsOperation::Rename { from, to } => {
                assert_eq!(from, &dir.join("a.txt"));
                assert_eq!(to, &dir.join("b.txt"));
            }
            other => panic!("expected Rename, got {:?}", other),
        }
    }

    /// When the user yanks a file in dir A, renames the original to C, and
    /// pastes a copy renamed to B, all_operations() should arrange them so
    /// the copy reads /dir/A before the rename consumes it.
    #[test]
    fn all_operations_orders_copy_before_consuming_rename() {
        let mut store = GlobalOperationStore::new();
        let dir = PathBuf::from("/dir");
        store.operations_by_dir.insert(
            dir.clone(),
            vec![
                FsOperation::Rename {
                    from: dir.join("A"),
                    to: dir.join("C"),
                },
                FsOperation::Copy {
                    from: dir.join("A"),
                    to: dir.join("B"),
                    is_dir: false,
                },
            ],
        );

        let ops = store.all_operations();
        let copy_idx = ops
            .iter()
            .position(|op| matches!(op, FsOperation::Copy { .. }))
            .unwrap();
        let rename_idx = ops
            .iter()
            .position(|op| matches!(op, FsOperation::Rename { .. }))
            .unwrap();
        assert!(
            copy_idx < rename_idx,
            "Copy must precede Rename when they share a source: {:?}",
            ops
        );
    }

    #[test]
    fn test_delete_without_rename_is_preserved() {
        let mut store = GlobalOperationStore::new();
        let dir_a = PathBuf::from("/dir_a");
        let file_in_a = dir_a.join("file.txt");

        // Just a regular delete without a corresponding rename
        store.operations_by_dir.insert(
            dir_a.clone(),
            vec![FsOperation::Delete {
                path: file_in_a.clone(),
            }],
        );

        let ops = store.all_operations();

        // Delete should be preserved
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            FsOperation::Delete { path } => {
                assert_eq!(path, &file_in_a);
            }
            _ => panic!("Expected Delete operation"),
        }
    }
}
