//! Git mode key handling

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::config::Config;
use crate::input::Mode;

use super::handle_confirm_scroll;

pub(super) fn handle_git(app: &mut App, key: KeyEvent) -> io::Result<()> {
    match (key.code, key.modifiers) {
        // Exit git menu
        (KeyCode::Esc, _)
        | (KeyCode::Char('q'), _)
        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.mode = Mode::Normal;
        }
        // Status (s)
        (KeyCode::Char('s'), _) => {
            let lines = app.git_status_lines(true);
            app.mode = Mode::GitStatus { lines, scroll: 0 };
        }
        // Pull (g)
        (KeyCode::Char('g'), _) => {
            app.git_pull()?;
            app.mode = Mode::Normal;
        }
        // Push (p)
        (KeyCode::Char('p'), _) => {
            app.git_push();
            app.mode = Mode::Normal;
        }
        // Commit staged (c)
        (KeyCode::Char('c'), m) if !m.contains(KeyModifiers::CONTROL) => {
            let status = app.git_status_lines(false);
            if status.is_empty() {
                app.set_status("No staged changes to commit");
                app.mode = Mode::Normal;
            } else {
                let auto_push = Config::load().git_auto_push;
                app.mode = Mode::GitCommit {
                    message: String::new(),
                    all: false,
                    auto_push,
                    status,
                };
            }
        }
        // Commit all (a)
        (KeyCode::Char('a'), _) => {
            let status = app.git_status_lines(true);
            if status.is_empty() {
                app.set_status("No changes to commit");
                app.mode = Mode::Normal;
            } else {
                let auto_push = Config::load().git_auto_push;
                app.mode = Mode::GitCommit {
                    message: String::new(),
                    all: true,
                    auto_push,
                    status,
                };
            }
        }
        _ => {}
    }

    Ok(())
}

pub(super) fn handle_git_status(
    app: &mut App,
    key: KeyEvent,
    lines: Vec<String>,
    scroll: usize,
) -> io::Result<()> {
    let max_scroll = lines.len().saturating_sub(10);

    // Handle scrolling
    if let Some(new_scroll) = handle_confirm_scroll(key, scroll, max_scroll) {
        app.mode = Mode::GitStatus {
            lines,
            scroll: new_scroll,
        };
        return Ok(());
    }

    match (key.code, key.modifiers) {
        // Exit
        (KeyCode::Esc, _)
        | (KeyCode::Char('q'), _)
        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.mode = Mode::Normal;
        }
        _ => {}
    }

    Ok(())
}

pub(super) fn handle_git_commit(
    app: &mut App,
    key: KeyEvent,
    message: String,
    all: bool,
    auto_push: bool,
    status: Vec<String>,
) -> io::Result<()> {
    match (key.code, key.modifiers) {
        // Cancel
        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.mode = Mode::Normal;
        }
        // Ctrl+P: toggle auto-push (persisted)
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            let new_auto_push = !auto_push;
            let mut config = Config::load();
            config.git_auto_push = new_auto_push;
            config.save();
            app.mode = Mode::GitCommit {
                message,
                all,
                auto_push: new_auto_push,
                status,
            };
        }
        // Enter: commit (and push if auto_push enabled)
        (KeyCode::Enter, _) => {
            if !message.is_empty() {
                app.git_commit(&message, all, auto_push);
            }
            app.mode = Mode::Normal;
        }
        // Backspace
        (KeyCode::Backspace, _) => {
            let mut msg = message;
            msg.pop();
            app.mode = Mode::GitCommit {
                message: msg,
                all,
                auto_push,
                status,
            };
        }
        // Type character
        (KeyCode::Char(c), _) => {
            let mut msg = message;
            msg.push(c);
            app.mode = Mode::GitCommit {
                message: msg,
                all,
                auto_push,
                status,
            };
        }
        _ => {}
    }

    Ok(())
}
