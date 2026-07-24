//! State-transition tests for the `App` layer.
//!
//! These exercise the buffer/diff/operation-store plumbing that drives
//! yank/cut/paste, deletion, and cross-directory operation persistence —
//! the stateful logic that has historically had no coverage. They assert on
//! *staged* operations (`pending_ops()` and the global store) and never call
//! `sync()`, so nothing touches the real filesystem or trash.

use super::*;
use crate::core::FsOperation;
use std::fs as stdfs;
use tempfile::TempDir;

/// Build an `App` rooted at a fresh temp directory containing `files`.
/// Each entry is `(name, is_dir)`. Returns the `TempDir` guard (keep it alive
/// for the duration of the test) and the constructed `App`.
fn app_with(files: &[(&str, bool)]) -> (TempDir, App) {
    let tmp = TempDir::new().expect("create temp dir");
    for (name, is_dir) in files {
        let path = tmp.path().join(name);
        if *is_dir {
            stdfs::create_dir(&path).expect("create dir");
        } else {
            stdfs::write(&path, b"contents").expect("write file");
        }
    }

    let mut app =
        App::new(tmp.path().to_path_buf(), false, false, None, None).expect("construct app");
    // Hidden files don't exist in these temp dirs, but pin the setting so the
    // visible entry set is independent of the developer's saved config.
    app.show_hidden = false;
    app.refresh_current_dir().expect("refresh");
    (tmp, app)
}

/// The path the app uses internally for `name` in the current directory.
/// `App::new` canonicalizes the cwd (on Windows this adds the `\\?\` prefix),
/// so derive expected paths from `app.cwd` rather than the raw temp path.
fn cwd_path(app: &App, name: &str) -> PathBuf {
    app.cwd.join(name)
}

#[test]
fn app_defers_audio_device_initialization() {
    let (_tmp, app) = app_with(&[]);
    assert!(
        app.audio_player.is_none(),
        "opening a file manager should not open the system audio device"
    );
}

#[test]
fn delete_stages_a_delete_op() {
    let (_tmp, mut app) = app_with(&[("a.txt", false), ("b.txt", false)]);
    app.select_by_name("a.txt");

    app.delete_line();

    let ops = app.pending_ops();
    assert_eq!(ops.len(), 1, "exactly one op expected, got {:?}", ops);
    match &ops[0] {
        FsOperation::Delete { path } => assert_eq!(path, &cwd_path(&app, "a.txt")),
        other => panic!("expected Delete, got {:?}", other),
    }
    // Deleting is also a cut: the file should be on the clipboard as a cut.
    assert!(app.yank_is_cut);
    assert_eq!(app.yank, vec![(cwd_path(&app, "a.txt"), false)]);
}

#[test]
fn delete_then_paste_same_dir_restores_with_no_pending_op() {
    let (_tmp, mut app) = app_with(&[("a.txt", false)]);
    app.select_by_name("a.txt");

    app.delete_line();
    assert_eq!(app.pending_ops().len(), 1, "delete should be staged");

    // Pasting the just-cut file back into its origin directory restores the
    // original line, cancelling the delete entirely.
    app.paste();
    assert!(
        app.pending_ops().is_empty(),
        "restore should cancel the delete, got {:?}",
        app.pending_ops()
    );
}

#[test]
fn yank_then_paste_into_other_dir_stages_a_copy() {
    let (_tmp, mut app) = app_with(&[("a.txt", false), ("sub", true)]);
    let source = cwd_path(&app, "a.txt");
    let sub = cwd_path(&app, "sub");

    app.select_by_name("a.txt");
    app.yank_selected();
    assert!(!app.yank_is_cut, "yank is a copy, not a cut");

    app.navigate_to(sub.clone()).expect("enter sub");
    app.paste();

    let ops = app.pending_ops();
    assert_eq!(ops.len(), 1, "expected one copy, got {:?}", ops);
    match &ops[0] {
        FsOperation::Copy { from, to, is_dir } => {
            assert_eq!(from, &source);
            assert_eq!(to, &sub.join("a.txt"));
            assert!(!is_dir);
        }
        other => panic!("expected Copy, got {:?}", other),
    }
}

#[test]
fn cut_then_paste_into_other_dir_resolves_to_single_rename() {
    let (_tmp, mut app) = app_with(&[("a.txt", false), ("sub", true)]);
    let source = cwd_path(&app, "a.txt");
    let sub = cwd_path(&app, "sub");

    // Cut in the source directory (stages a Delete there)...
    app.select_by_name("a.txt");
    app.delete_line();
    assert_eq!(app.pending_ops().len(), 1, "delete staged in source dir");

    // ...then paste into the subdirectory (stages a move there).
    app.navigate_to(sub.clone()).expect("enter sub");
    app.paste();

    // Capture the subdir's pending move into the global store, then confirm
    // the cross-directory resolver collapses Delete(source) + move into a
    // single Rename rather than deleting and re-creating the file.
    app.capture_current_operations();
    let ops = app.global_operations.all_operations();
    assert_eq!(
        ops.len(),
        1,
        "delete should be absorbed by the move, got {:?}",
        ops
    );
    match &ops[0] {
        FsOperation::Rename { from, to } => {
            assert_eq!(from, &source);
            assert_eq!(to, &sub.join("a.txt"));
        }
        other => panic!("expected Rename, got {:?}", other),
    }
}

#[test]
fn pending_ops_persist_across_directory_navigation() {
    let (_tmp, mut app) = app_with(&[("a.txt", false), ("b.txt", false), ("sub", true)]);
    let sub = cwd_path(&app, "sub");
    let deleted = cwd_path(&app, "a.txt");

    app.select_by_name("a.txt");
    app.delete_line();
    assert_eq!(app.pending_ops().len(), 1);

    // Navigating away captures the staged op into the global store...
    app.navigate_to(sub).expect("enter sub");
    assert!(app.pending_ops().is_empty(), "sub dir has no pending ops");

    // ...and navigating back restores it to the buffer.
    app.navigate_to(app.cwd.parent().unwrap().to_path_buf())
        .expect("return to parent");
    let ops = app.pending_ops();
    assert_eq!(ops.len(), 1, "op should be restored, got {:?}", ops);
    match &ops[0] {
        FsOperation::Delete { path } => assert_eq!(path, &deleted),
        other => panic!("expected restored Delete, got {:?}", other),
    }
    assert!(
        !app.current.buffer.lines.iter().any(|l| l.text == "a.txt"),
        "restored delete should keep a.txt out of the visible list"
    );
}

#[test]
fn yank_marked_collects_all_marked_files_and_clears_marks() {
    let (_tmp, mut app) = app_with(&[("a.txt", false), ("b.txt", false)]);

    app.select_by_name("a.txt");
    app.toggle_mark();
    app.select_by_name("b.txt");
    app.toggle_mark();
    assert_eq!(app.mark_count(), 2);

    app.yank_marked();

    assert_eq!(app.yank.len(), 2, "both marked files should be yanked");
    assert!(!app.yank_is_cut, "marked yank is a copy");
    assert_eq!(app.mark_count(), 0, "marks should be cleared after yank");
    let yanked: std::collections::HashSet<_> = app.yank.iter().map(|(p, _)| p.clone()).collect();
    assert!(yanked.contains(&cwd_path(&app, "a.txt")));
    assert!(yanked.contains(&cwd_path(&app, "b.txt")));
}

#[test]
fn undo_restores_buffer_after_delete() {
    let (_tmp, mut app) = app_with(&[("a.txt", false), ("b.txt", false)]);
    app.select_by_name("a.txt");

    app.delete_line();
    assert!(!app.current.buffer.lines.iter().any(|l| l.text == "a.txt"));

    app.undo();
    assert!(
        app.current.buffer.lines.iter().any(|l| l.text == "a.txt"),
        "undo should bring a.txt back"
    );
    assert!(
        app.pending_ops().is_empty(),
        "undo should clear the staged delete"
    );
}

// ===== Shell command (`!`) substitution helpers =====

#[test]
fn shell_quote_wraps_and_escapes() {
    #[cfg(not(windows))]
    {
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(shell_quote("with space"), "'with space'");
        // Embedded single quote -> '\'' .
        assert_eq!(shell_quote("a'b"), r#"'a'\''b'"#);
    }
    #[cfg(windows)]
    {
        assert_eq!(shell_quote("plain"), "\"plain\"");
        assert_eq!(shell_quote("with space"), "\"with space\"");
    }
}

#[test]
fn expand_command_substitutes_placeholders() {
    let dir = PathBuf::from("/work");
    let files = vec![PathBuf::from("/work/a.txt"), PathBuf::from("/work/b c.txt")];

    // %f -> all targets, quoted and space-joined.
    let out = expand_command("echo %f", &files, &dir);
    assert_eq!(
        out,
        format!(
            "echo {} {}",
            shell_quote("/work/a.txt"),
            shell_quote("/work/b c.txt")
        )
    );

    // %n -> base names; %d -> dir.
    assert_eq!(
        expand_command("x %n in %d", &files, &dir),
        format!(
            "x {} {} in {}",
            shell_quote("a.txt"),
            shell_quote("b c.txt"),
            shell_quote("/work")
        )
    );

    // %% -> literal %, unknown %z left untouched.
    assert_eq!(expand_command("100%% %z", &files, &dir), "100% %z");

    // No placeholder -> verbatim.
    assert_eq!(expand_command("ls -la", &files, &dir), "ls -la");
}

#[test]
fn expand_command_with_no_targets_yields_empty_f() {
    let dir = PathBuf::from("/work");
    let files: Vec<PathBuf> = Vec::new();
    assert_eq!(expand_command("echo [%f]", &files, &dir), "echo []");
}

#[test]
fn text_file_detection_rejects_binary_content() {
    let temp = TempDir::new().expect("create temp dir");
    let text = temp.path().join("notes.txt");
    let binary = temp.path().join("data.bin");
    stdfs::write(&text, "hello\nworld\n").expect("write text fixture");
    stdfs::write(&binary, [0_u8, 1, 2, 3, 4]).expect("write binary fixture");

    assert!(App::is_text_file(&text));
    assert!(!App::is_text_file(&binary));
}
