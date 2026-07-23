use std::cmp::Ordering;

use crate::core::Entry;

/// Display info option for file list
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplayInfo {
    #[default]
    None,
    DateModified,
    Size,
    Mode,
    Extension,
}

impl DisplayInfo {
    #[allow(dead_code)]
    pub fn display_name(&self) -> &'static str {
        match self {
            DisplayInfo::None => "None",
            DisplayInfo::DateModified => "Date Modified",
            DisplayInfo::Size => "Size",
            DisplayInfo::Mode => "Mode",
            DisplayInfo::Extension => "Extension",
        }
    }
}

/// Sort option for directory entries
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOption {
    #[default]
    Name, // Ascending (A-Z)
    NameDesc,        // Descending (Z-A)
    DateModified,    // Descending (newest first)
    DateModifiedAsc, // Ascending (oldest first)
    Size,            // Descending (largest first)
    SizeAsc,         // Ascending (smallest first)
    Extension,       // Ascending (A-Z)
    ExtensionDesc,   // Descending (Z-A)
}

impl SortOption {
    /// Get display name for the sort option
    pub fn display_name(&self) -> &'static str {
        match self {
            SortOption::Name => "Name (A-Z)",
            SortOption::NameDesc => "Name (Z-A)",
            SortOption::DateModified => "Date Modified (Newest)",
            SortOption::DateModifiedAsc => "Date Modified (Oldest)",
            SortOption::Size => "Size (Largest)",
            SortOption::SizeAsc => "Size (Smallest)",
            SortOption::Extension => "Extension (A-Z)",
            SortOption::ExtensionDesc => "Extension (Z-A)",
        }
    }

    /// Get the key hint for the sort option
    #[allow(dead_code)]
    pub fn key_hint(&self) -> &'static str {
        match self {
            SortOption::Name => "n",
            SortOption::NameDesc => "N",
            SortOption::DateModified => "d",
            SortOption::DateModifiedAsc => "D",
            SortOption::Size => "s",
            SortOption::SizeAsc => "S",
            SortOption::Extension => "e",
            SortOption::ExtensionDesc => "E",
        }
    }

    /// Sort entries according to this option
    pub fn sort_entries(&self, entries: &mut [Entry]) {
        if entries.len() < 2 {
            return;
        }

        // Lowercase each name once up front. Comparisons reference these by
        // index rather than allocating a fresh lowercase String on every call
        // — a sort performs O(n log n) comparisons, so re-lowercasing inside
        // the comparator allocated heavily on large directories.
        let lower: Vec<String> = entries.iter().map(|e| e.name.to_lowercase()).collect();

        // Directories come first only when sorting by name/extension.
        let dirs_first = matches!(
            self,
            SortOption::Name
                | SortOption::NameDesc
                | SortOption::Extension
                | SortOption::ExtensionDesc
        );

        // Sort an index permutation so the precomputed keys can be reused,
        // then apply it to the entries in place.
        let mut order: Vec<usize> = (0..entries.len()).collect();
        order.sort_by(|&ia, &ib| {
            let (a, b) = (&entries[ia], &entries[ib]);
            let (a_name, b_name) = (lower[ia].as_str(), lower[ib].as_str());

            if dirs_first {
                match (a.is_dir(), b.is_dir()) {
                    (true, false) => return Ordering::Less,
                    (false, true) => return Ordering::Greater,
                    _ => {}
                }
            }

            match self {
                SortOption::Name => a_name.cmp(b_name),
                SortOption::NameDesc => b_name.cmp(a_name),
                SortOption::DateModified => compare_modified(b, a, b_name, a_name), // Newest first
                SortOption::DateModifiedAsc => compare_modified(a, b, a_name, b_name), // Oldest first
                SortOption::Size => compare_size(b, a, b_name, a_name), // Largest first
                SortOption::SizeAsc => compare_size(a, b, a_name, b_name), // Smallest first
                SortOption::Extension => compare_extension(a_name, b_name),
                SortOption::ExtensionDesc => compare_extension(b_name, a_name),
            }
        });

        apply_permutation(entries, order);
    }
}

/// Reorder `slice` in place so that `slice[i]` ends up holding the element
/// originally at `order[i]` (a gather permutation). Uses swaps, so it needs no
/// `Clone` bound.
fn apply_permutation<T>(slice: &mut [T], order: Vec<usize>) {
    for i in 0..slice.len() {
        // The element that belongs at `i` started at `order[i]`, but earlier
        // positions may have already swapped it forward — chase it through the
        // already-placed prefix until we find its current home.
        let mut src = order[i];
        while src < i {
            src = order[src];
        }
        slice.swap(i, src);
    }
}

fn compare_modified(a: &Entry, b: &Entry, a_name: &str, b_name: &str) -> Ordering {
    match (&a.modified, &b.modified) {
        (Some(a_time), Some(b_time)) => a_time.cmp(b_time),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a_name.cmp(b_name),
    }
}

fn compare_size(a: &Entry, b: &Entry, a_name: &str, b_name: &str) -> Ordering {
    match (&a.size, &b.size) {
        (Some(a_size), Some(b_size)) => a_size.cmp(b_size),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a_name.cmp(b_name),
    }
}

/// Compare by extension, with the (already lowercased) file name as tiebreak.
/// Names are passed in pre-lowercased; the extension of a lowercased name is
/// itself the lowercased extension, so no further allocation is needed.
fn compare_extension(a_name: &str, b_name: &str) -> Ordering {
    let a_ext = a_name.rsplit('.').next().unwrap_or("");
    let b_ext = b_name.rsplit('.').next().unwrap_or("");

    match a_ext.cmp(b_ext) {
        Ordering::Equal => a_name.cmp(b_name),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::entry::{EntryId, EntryKind};
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    fn entry(name: &str, dir: bool, size: u64, modified_secs: u64) -> Entry {
        Entry {
            id: EntryId::new(),
            name: name.to_string(),
            kind: if dir {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            path: PathBuf::from(name),
            size: Some(size),
            modified: Some(UNIX_EPOCH + Duration::from_secs(modified_secs)),
            mode: None,
        }
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn name_sort_is_case_insensitive_and_dirs_first() {
        let mut entries = vec![
            entry("banana.txt", false, 1, 1),
            entry("Apple", true, 1, 1),
            entry("apple.txt", false, 1, 1),
            entry("Zebra", true, 1, 1),
        ];
        SortOption::Name.sort_entries(&mut entries);
        // Directories first (case-insensitive), then files (case-insensitive).
        assert_eq!(
            names(&entries),
            vec!["Apple", "Zebra", "apple.txt", "banana.txt"]
        );
    }

    #[test]
    fn name_desc_reverses_within_groups() {
        let mut entries = vec![
            entry("a.txt", false, 1, 1),
            entry("dir_b", true, 1, 1),
            entry("c.txt", false, 1, 1),
            entry("dir_a", true, 1, 1),
        ];
        SortOption::NameDesc.sort_entries(&mut entries);
        assert_eq!(names(&entries), vec!["dir_b", "dir_a", "c.txt", "a.txt"]);
    }

    #[test]
    fn size_sort_does_not_force_dirs_first() {
        let mut entries = vec![
            entry("small.txt", false, 10, 1),
            entry("bigdir", true, 999, 1),
            entry("large.txt", false, 500, 1),
        ];
        SortOption::Size.sort_entries(&mut entries); // largest first
        assert_eq!(names(&entries), vec!["bigdir", "large.txt", "small.txt"]);
        SortOption::SizeAsc.sort_entries(&mut entries);
        assert_eq!(names(&entries), vec!["small.txt", "large.txt", "bigdir"]);
    }

    #[test]
    fn extension_sort_groups_by_extension_then_name() {
        let mut entries = vec![
            entry("b.rs", false, 1, 1),
            entry("a.txt", false, 1, 1),
            entry("a.rs", false, 1, 1),
        ];
        SortOption::Extension.sort_entries(&mut entries);
        assert_eq!(names(&entries), vec!["a.rs", "b.rs", "a.txt"]);
    }

    #[test]
    fn date_modified_sorts_newest_first() {
        let mut entries = vec![
            entry("old.txt", false, 1, 100),
            entry("new.txt", false, 1, 300),
            entry("mid.txt", false, 1, 200),
        ];
        SortOption::DateModified.sort_entries(&mut entries);
        assert_eq!(names(&entries), vec!["new.txt", "mid.txt", "old.txt"]);
    }

    #[test]
    fn sorting_zero_or_one_entry_is_a_noop() {
        let mut empty: Vec<Entry> = vec![];
        SortOption::Name.sort_entries(&mut empty);
        assert!(empty.is_empty());

        let mut one = vec![entry("only.txt", false, 1, 1)];
        SortOption::Name.sort_entries(&mut one);
        assert_eq!(names(&one), vec!["only.txt"]);
    }
}
