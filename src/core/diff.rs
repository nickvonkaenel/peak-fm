use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::buffer::Buffer;
use super::entry::EntryId;

#[derive(Debug, Clone)]
pub enum FsOperation {
    Create {
        path: PathBuf,
        is_dir: bool,
    },
    Delete {
        path: PathBuf,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    Copy {
        from: PathBuf,
        to: PathBuf,
        is_dir: bool,
    },
}

impl FsOperation {
    pub fn order(&self) -> u8 {
        match self {
            FsOperation::Delete { .. } => 0,
            FsOperation::Rename { .. } => 1,
            FsOperation::Copy { .. } => 2,
            FsOperation::Create { .. } => 3,
        }
    }

    #[allow(dead_code)]
    pub fn description(&self) -> String {
        match self {
            FsOperation::Create { path, is_dir } => {
                let kind = if *is_dir { "dir" } else { "file" };
                format!("Create {}: {}", kind, path.display())
            }
            FsOperation::Delete { path } => {
                format!("Delete: {}", path.display())
            }
            FsOperation::Rename { from, to } => {
                format!(
                    "Rename: {} -> {}",
                    from.file_name().unwrap_or_default().to_string_lossy(),
                    to.file_name().unwrap_or_default().to_string_lossy()
                )
            }
            FsOperation::Copy { from, to, .. } => {
                format!(
                    "Copy: {} -> {}",
                    from.file_name().unwrap_or_default().to_string_lossy(),
                    to.file_name().unwrap_or_default().to_string_lossy()
                )
            }
        }
    }
}

pub fn compute_diff(buffer: &Buffer) -> Vec<FsOperation> {
    let mut ops = Vec::new();

    let snapshot_by_id: HashMap<EntryId, (&str, bool)> = buffer
        .snapshot
        .iter()
        .filter_map(|line| line.id.map(|id| (id, (line.text.as_str(), line.is_dir))))
        .collect();

    let current_by_id: HashMap<EntryId, (&str, bool)> = buffer
        .lines
        .iter()
        .filter_map(|line| line.id.map(|id| (id, (line.text.as_str(), line.is_dir))))
        .collect();

    let snapshot_ids: HashSet<EntryId> = snapshot_by_id.keys().copied().collect();
    let current_ids: HashSet<EntryId> = current_by_id.keys().copied().collect();

    // Deleted entries: in snapshot but not in current
    for id in snapshot_ids.difference(&current_ids) {
        if let Some((name, _)) = snapshot_by_id.get(id) {
            ops.push(FsOperation::Delete {
                path: buffer.path.join(name),
            });
        }
    }

    // New entries: lines with no ID
    for line in &buffer.lines {
        if line.id.is_none() && !line.text.is_empty() {
            if let Some(ref source) = line.move_from {
                // This is a move operation (rename) - skip if same path (no-op)
                let dest = buffer.path.join(&line.text);
                if source != &dest {
                    ops.push(FsOperation::Rename {
                        from: source.clone(),
                        to: dest,
                    });
                }
            } else if let Some(ref source) = line.copy_from {
                // This is a copy operation - skip if same path (no-op)
                let dest = buffer.path.join(&line.text);
                if source != &dest {
                    ops.push(FsOperation::Copy {
                        from: source.clone(),
                        to: dest,
                        is_dir: line.is_dir,
                    });
                }
            } else {
                // This is a create operation
                ops.push(FsOperation::Create {
                    path: buffer.path.join(&line.text),
                    is_dir: line.is_dir,
                });
            }
        }
    }

    // Renamed entries: same ID, different text
    for id in snapshot_ids.intersection(&current_ids) {
        let (old_name, _) = snapshot_by_id.get(id).unwrap();
        let (new_name, _) = current_by_id.get(id).unwrap();
        if old_name != new_name {
            ops.push(FsOperation::Rename {
                from: buffer.path.join(old_name),
                to: buffer.path.join(new_name),
            });
        }
    }

    topological_sort(&mut ops);
    ops
}

fn primary_path(op: &FsOperation) -> &Path {
    match op {
        FsOperation::Create { path, .. } => path,
        FsOperation::Delete { path } => path,
        FsOperation::Rename { from, .. } => from,
        FsOperation::Copy { from, .. } => from,
    }
}

/// Topologically sort operations so dependent ops run in a safe order.
///
/// Beyond the base order (Delete → Rename → Copy → Create), Copy operations
/// need special handling because their source must exist at execution time:
/// - A Copy whose source is the `from` of a Rename must run BEFORE that
///   Rename (otherwise the source has already moved away).
/// - A Copy whose source is the `to` of a Rename must run AFTER that Rename
///   (the source only exists after the rename has produced it).
pub fn topological_sort(ops: &mut Vec<FsOperation>) {
    let rename_sources: HashSet<PathBuf> = ops
        .iter()
        .filter_map(|op| {
            if let FsOperation::Rename { from, .. } = op {
                Some(from.clone())
            } else {
                None
            }
        })
        .collect();
    let rename_targets: HashSet<PathBuf> = ops
        .iter()
        .filter_map(|op| {
            if let FsOperation::Rename { to, .. } = op {
                Some(to.clone())
            } else {
                None
            }
        })
        .collect();

    ops.sort_by(|a, b| {
        let pa = priority(a, &rename_sources, &rename_targets);
        let pb = priority(b, &rename_sources, &rename_targets);
        pa.cmp(&pb)
            .then_with(|| primary_path(a).cmp(primary_path(b)))
    });
}

fn priority(
    op: &FsOperation,
    rename_sources: &HashSet<PathBuf>,
    rename_targets: &HashSet<PathBuf>,
) -> u8 {
    match op {
        FsOperation::Delete { .. } => 0,
        FsOperation::Copy { from, .. } => {
            if rename_sources.contains(from) {
                1 // Copy from a soon-to-be-renamed source: do BEFORE the rename
            } else if rename_targets.contains(from) {
                4 // Copy from a rename's destination: do AFTER the rename produces it
            } else {
                2 // Independent copy
            }
        }
        FsOperation::Rename { .. } => 3,
        FsOperation::Create { .. } => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::buffer::{Buffer, BufferLine};
    use crate::core::entry::EntryId;

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

    /// Build a buffer where the snapshot matches the initial lines.
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

    // ===== Basic diff scenarios =====

    #[test]
    fn empty_buffer_produces_no_ops() {
        let buf = buffer_from("/dir", &[]);
        assert!(compute_diff(&buf).is_empty());
    }

    #[test]
    fn unchanged_buffer_produces_no_ops() {
        let buf = buffer_from("/dir", &[("a.txt", false), ("sub", true)]);
        assert!(compute_diff(&buf).is_empty());
    }

    #[test]
    fn rename_via_text_edit() {
        let mut buf = buffer_from("/dir", &[("a.txt", false)]);
        buf.lines[0].text = "b.txt".into();
        let ops = compute_diff(&buf);
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            FsOperation::Rename { from, to } => {
                assert_eq!(from, &PathBuf::from("/dir/a.txt"));
                assert_eq!(to, &PathBuf::from("/dir/b.txt"));
            }
            other => panic!("expected Rename, got {:?}", other),
        }
    }

    #[test]
    fn rename_back_to_original_is_no_op() {
        let mut buf = buffer_from("/dir", &[("a.txt", false)]);
        buf.lines[0].text = "tmp".into();
        buf.lines[0].text = "a.txt".into();
        assert!(
            compute_diff(&buf).is_empty(),
            "renaming back to original name should produce no op"
        );
    }

    #[test]
    fn delete_via_line_removal() {
        let mut buf = buffer_from("/dir", &[("a.txt", false), ("b.txt", false)]);
        buf.lines.remove(0);
        let ops = compute_diff(&buf);
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            FsOperation::Delete { path } => assert_eq!(path, &PathBuf::from("/dir/a.txt")),
            other => panic!("expected Delete, got {:?}", other),
        }
    }

    #[test]
    fn create_via_new_line() {
        let mut buf = buffer_from("/dir", &[]);
        let mut new_line = BufferLine::new_empty();
        new_line.text = "new.txt".into();
        buf.lines.push(new_line);
        let ops = compute_diff(&buf);
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            FsOperation::Create { path, is_dir } => {
                assert_eq!(path, &PathBuf::from("/dir/new.txt"));
                assert!(!is_dir);
            }
            other => panic!("expected Create, got {:?}", other),
        }
    }

    #[test]
    fn copy_with_same_source_and_dest_is_skipped() {
        // User yanked file then pasted in same directory without renaming.
        let mut buf = buffer_from("/dir", &[("a.txt", false)]);
        buf.lines.push(BufferLine::new_copy(
            "a.txt".into(),
            false,
            PathBuf::from("/dir/a.txt"),
        ));
        assert!(
            compute_diff(&buf).is_empty(),
            "copy where source == dest should be skipped"
        );
    }

    #[test]
    fn move_with_same_source_and_dest_is_skipped() {
        let mut buf = buffer_from("/dir", &[("a.txt", false)]);
        // Drop the original line then add a move line back to same path.
        buf.lines.clear();
        buf.lines.push(BufferLine::new_move(
            "a.txt".into(),
            false,
            PathBuf::from("/dir/a.txt"),
        ));
        // The original 'a.txt' line is gone from current but still in snapshot —
        // diff sees a Delete + a no-op move. Verify the move is skipped while
        // the delete remains.
        let ops = compute_diff(&buf);
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            FsOperation::Delete { path } => assert_eq!(path, &PathBuf::from("/dir/a.txt")),
            other => panic!("expected Delete only, got {:?}", other),
        }
    }

    #[test]
    fn empty_line_does_not_become_create() {
        let mut buf = buffer_from("/dir", &[]);
        buf.lines.push(BufferLine::new_empty());
        assert!(
            compute_diff(&buf).is_empty(),
            "empty new lines should not produce a Create"
        );
    }

    // ===== Topological ordering — the core regression =====

    /// Regression: user yanks A, renames A → C in same buffer, pastes a copy
    /// (which originally bears name A), then renames the copy to B.
    /// Sync must execute the copy BEFORE the rename, otherwise the source
    /// path /dir/A no longer exists.
    #[test]
    fn copy_runs_before_rename_that_consumes_its_source() {
        let mut buf = buffer_from("/dir", &[("A", false)]);
        // Rename A → C
        buf.lines[0].text = "C".into();
        // Add a copy line (text already renamed to B by user) sourced from A
        buf.lines.push(BufferLine::new_copy(
            "B".into(),
            false,
            PathBuf::from("/dir/A"),
        ));

        let ops = compute_diff(&buf);
        let copy_idx = ops
            .iter()
            .position(|op| matches!(op, FsOperation::Copy { .. }));
        let rename_idx = ops
            .iter()
            .position(|op| matches!(op, FsOperation::Rename { .. }));
        let (copy_idx, rename_idx) = (copy_idx.expect("copy"), rename_idx.expect("rename"));
        assert!(
            copy_idx < rename_idx,
            "Copy from /dir/A must run before Rename /dir/A → /dir/C, got ops: {:?}",
            ops
        );
    }

    /// Reverse: user renames A → C first, then yanks C and pastes a copy.
    /// The copy's source path is /dir/C, which only exists after the rename
    /// completes — so Copy must run AFTER Rename.
    #[test]
    fn copy_runs_after_rename_that_produces_its_source() {
        let mut buf = buffer_from("/dir", &[("A", false)]);
        buf.lines[0].text = "C".into();
        // Copy line sourced from the post-rename path
        buf.lines.push(BufferLine::new_copy(
            "D".into(),
            false,
            PathBuf::from("/dir/C"),
        ));

        let ops = compute_diff(&buf);
        let copy_idx = ops
            .iter()
            .position(|op| matches!(op, FsOperation::Copy { .. }));
        let rename_idx = ops
            .iter()
            .position(|op| matches!(op, FsOperation::Rename { .. }));
        let (copy_idx, rename_idx) = (copy_idx.expect("copy"), rename_idx.expect("rename"));
        assert!(
            copy_idx > rename_idx,
            "Copy from /dir/C must run after Rename /dir/A → /dir/C, got ops: {:?}",
            ops
        );
    }

    #[test]
    fn delete_always_runs_first() {
        // Build a buffer with all four op kinds and ensure delete leads.
        let mut buf = buffer_from("/dir", &[("a", false), ("b", false)]);
        // Delete a
        buf.lines.remove(0);
        // Rename b → b2
        buf.lines[0].text = "b2".into();
        // Copy from external path
        buf.lines.push(BufferLine::new_copy(
            "copied".into(),
            false,
            PathBuf::from("/other/src"),
        ));
        // Create new file
        let mut new_line = BufferLine::new_empty();
        new_line.text = "new".into();
        buf.lines.push(new_line);

        let ops = compute_diff(&buf);
        assert!(matches!(ops[0], FsOperation::Delete { .. }));
        // Create should be last
        assert!(matches!(ops.last().unwrap(), FsOperation::Create { .. }));
    }

    // ===== Delete + recreate same path =====

    #[test]
    fn delete_then_create_same_name() {
        // User deletes a file then types a new file with the same name on a fresh line.
        let mut buf = buffer_from("/dir", &[("x.txt", false)]);
        buf.lines.remove(0);
        let mut new_line = BufferLine::new_empty();
        new_line.text = "x.txt".into();
        buf.lines.push(new_line);

        let ops = compute_diff(&buf);
        assert_eq!(ops.len(), 2);
        // Delete must come before Create so apply_operations can validate via
        // the pending_deletes set.
        assert!(matches!(ops[0], FsOperation::Delete { .. }));
        assert!(matches!(ops[1], FsOperation::Create { .. }));
    }

    // ===== topological_sort directly =====

    #[test]
    fn topological_sort_independent_copy_keeps_default_order() {
        let mut ops = vec![
            FsOperation::Create {
                path: PathBuf::from("/dir/new"),
                is_dir: false,
            },
            FsOperation::Copy {
                from: PathBuf::from("/elsewhere/src"),
                to: PathBuf::from("/dir/copied"),
                is_dir: false,
            },
            FsOperation::Delete {
                path: PathBuf::from("/dir/old"),
            },
        ];
        topological_sort(&mut ops);
        assert!(matches!(ops[0], FsOperation::Delete { .. }));
        assert!(matches!(ops[1], FsOperation::Copy { .. }));
        assert!(matches!(ops[2], FsOperation::Create { .. }));
    }
}
