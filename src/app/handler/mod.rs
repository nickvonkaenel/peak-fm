mod audio;
mod command;
mod git;
mod menus;
mod normal;
mod search;
mod visual;

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::App;
use crate::fs;
use crate::input::{ConfirmAction, Mode};

use normal::handle_normal;

// Helper functions

/// Handle preview scrolling (Ctrl+f/Ctrl+b) common across find/grep modes
pub(super) fn handle_preview_scroll_keys(app: &mut App, key: KeyEvent) -> bool {
    let page = if app.preview_height > 0 {
        app.preview_height / 2
    } else {
        10
    };

    match (key.code, key.modifiers) {
        (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
            app.scroll_preview(page.max(1) as isize);
            true
        }
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
            app.scroll_preview(-(page.max(1) as isize));
            true
        }
        _ => false,
    }
}

/// Calculate half-page scroll amount based on terminal height
pub(super) fn calculate_half_page_scroll() -> usize {
    let (_, term_height) = crossterm::terminal::size().unwrap_or((80, 24));
    let results_height = (term_height / 2).saturating_sub(2) as usize;
    (results_height / 2).max(1)
}

/// Update scroll value with a delta, respecting bounds
fn update_scroll(current: usize, delta: isize, max: usize) -> usize {
    if delta > 0 {
        (current + delta as usize).min(max)
    } else {
        current.saturating_sub(delta.unsigned_abs())
    }
}

/// Handle scroll keys for confirmation dialogs
/// Returns Some(new_scroll) if key was handled, None otherwise
pub(super) fn handle_confirm_scroll(
    key: KeyEvent,
    current_scroll: usize,
    max_scroll: usize,
) -> Option<usize> {
    match (key.code, key.modifiers) {
        // Half page down
        (KeyCode::Char('d'), m) if m.contains(KeyModifiers::CONTROL) => {
            Some(update_scroll(current_scroll, 5, max_scroll))
        }
        // Single line down
        (KeyCode::Char('j'), _) | (KeyCode::Down, _) => {
            Some(update_scroll(current_scroll, 1, max_scroll))
        }
        // Half page up
        (KeyCode::Char('u'), m) if m.contains(KeyModifiers::CONTROL) => {
            Some(update_scroll(current_scroll, -5, max_scroll))
        }
        // Single line up
        (KeyCode::Char('k'), _) | (KeyCode::Up, _) => {
            Some(update_scroll(current_scroll, -1, max_scroll))
        }
        _ => None,
    }
}

/// Handle menu navigation (up/down with wrapping)
/// Returns Some(new_index) if navigation key was pressed, None otherwise
pub(super) fn handle_menu_navigation(
    key: KeyEvent,
    current: usize,
    total_items: usize,
) -> Option<usize> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some((current + 1) % total_items),
        KeyCode::Char('k') | KeyCode::Up => Some(if current == 0 {
            total_items - 1
        } else {
            current - 1
        }),
        _ => None,
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> io::Result<()> {
    match app.mode.clone() {
        Mode::Normal => handle_normal(app, key),
        Mode::Insert => handle_insert(app, key),
        Mode::Visual { anchor } => visual::handle_visual(app, key, anchor),
        Mode::VisualInsert { anchor, edit_type } => {
            visual::handle_visual_insert(app, key, anchor, edit_type)
        }
        Mode::Confirm(action) => handle_confirm(app, key, action),
        Mode::SyncConfirm { scroll } => handle_sync_confirm(app, key, scroll),
        Mode::QuitConfirm { scroll } => handle_quit_confirm(app, key, scroll),
        Mode::Search(_) => search::handle_search(app, key),
        Mode::Find => search::handle_find(app, key),
        Mode::Grep => search::handle_grep(app, key),
        Mode::Help => handle_help(app, key),
        Mode::Settings => menus::handle_settings(app, key),
        Mode::ThemeSelect { selected } => menus::handle_theme_select(app, key, selected),
        Mode::Sort {
            selected,
            is_global,
        } => menus::handle_sort(app, key, selected, is_global),
        Mode::InfoSelect { selected } => menus::handle_info_select(app, key, selected),
        Mode::Leader => menus::handle_leader(app, key),
        Mode::PreviewOptions => menus::handle_preview_options(app, key),
        Mode::Git => git::handle_git(app, key),
        Mode::GitStatus { lines, scroll } => git::handle_git_status(app, key, lines, scroll),
        Mode::GitCommit {
            message,
            all,
            auto_push,
            status,
        } => git::handle_git_commit(app, key, message, all, auto_push, status),
        Mode::Audio => audio::handle_audio(app, key),
        Mode::Command => command::handle_command(app, key),
    }
}

fn handle_insert(app: &mut App, key: KeyEvent) -> io::Result<()> {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.exit_insert_mode();
        }
        // Enter adds a new line below and stays in insert mode
        (KeyCode::Enter, _) => {
            app.save_undo_state();
            app.current.insert_below();
            app.current.buffer.edit_cursor = 0;
        }
        // Ctrl shortcuts
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            app.delete_word();
        }
        (KeyCode::Char('l'), KeyModifiers::CONTROL)
        | (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.clear_line();
        }
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
            app.delete_to_end();
        }
        (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
            app.move_edit_cursor_to(0);
        }
        (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
            app.move_edit_cursor_to_end();
        }
        (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
            app.delete_char();
        }
        (KeyCode::Char(c), _) => {
            app.insert_char(c);
        }
        (KeyCode::Backspace, _) => {
            app.delete_char();
        }
        (KeyCode::Left, _) => {
            app.move_edit_cursor(-1);
        }
        (KeyCode::Right, _) => {
            app.move_edit_cursor(1);
        }
        _ => {}
    }

    Ok(())
}

fn handle_confirm(app: &mut App, key: KeyEvent, action: ConfirmAction) -> io::Result<()> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            app.mode = Mode::Normal;
            match action {
                ConfirmAction::Sync => {
                    app.remove_empty_lines();
                    let _ = app.sync();
                }
                ConfirmAction::Quit => {
                    app.should_quit = true;
                }
                ConfirmAction::EmptyTrash => {
                    match fs::empty_trash() {
                        Ok(count) => {
                            app.set_status(format!("Deleted {} item(s) from trash", count));
                            // Refresh if we're in trash directory
                            if let Ok(trash) = fs::trash_dir() {
                                if app.cwd == trash {
                                    let _ = app.refresh_current_dir();
                                }
                            }
                        }
                        Err(e) => {
                            app.set_status(format!("Error emptying trash: {}", e));
                        }
                    }
                }
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.set_status("Cancelled");
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = Mode::Normal;
            app.set_status("Cancelled");
        }
        _ => {}
    }

    Ok(())
}

fn handle_sync_confirm(app: &mut App, key: KeyEvent, scroll: usize) -> io::Result<()> {
    let ops_count = app.pending_ops().len();

    // Calculate max visible ops based on terminal height
    // Fixed lines: header(1) + spacing(2) + footer(1) + borders(2) + scroll indicators(2) = 8
    let (_, term_height) = crossterm::terminal::size().unwrap_or((80, 24));
    let max_popup_height = term_height.saturating_sub(4) as usize;
    let max_ops_visible = max_popup_height.saturating_sub(8);
    let max_scroll = ops_count.saturating_sub(max_ops_visible);

    // Handle scrolling
    if let Some(new_scroll) = handle_confirm_scroll(key, scroll, max_scroll) {
        app.mode = Mode::SyncConfirm { scroll: new_scroll };
        return Ok(());
    }

    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            app.mode = Mode::Normal;
            app.remove_empty_lines();
            let _ = app.sync();
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.set_status("Sync cancelled");
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = Mode::Normal;
            app.set_status("Sync cancelled");
        }
        _ => {}
    }

    Ok(())
}

fn handle_quit_confirm(app: &mut App, key: KeyEvent, scroll: usize) -> io::Result<()> {
    let ops_count = app.pending_ops().len();

    // Calculate max visible ops based on terminal height
    // Fixed lines: header(1) + spacing(2) + footers(2) + borders(2) + scroll indicators(2) = 9
    let (_, term_height) = crossterm::terminal::size().unwrap_or((80, 24));
    let max_popup_height = term_height.saturating_sub(4) as usize;
    let max_ops_visible = max_popup_height.saturating_sub(9);
    let max_scroll = ops_count.saturating_sub(max_ops_visible);

    // Handle scrolling
    if let Some(new_scroll) = handle_confirm_scroll(key, scroll, max_scroll) {
        app.mode = Mode::QuitConfirm { scroll: new_scroll };
        return Ok(());
    }

    match key.code {
        // 'y' to quit and discard changes
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            app.should_quit = true;
        }
        // Shift+S to sync and then quit
        KeyCode::Char('S') => {
            app.mode = Mode::Normal;
            app.remove_empty_lines();
            if app.sync().is_ok() {
                app.should_quit = true;
            }
        }
        // 'n' or Esc to cancel
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.set_status("Quit cancelled");
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = Mode::Normal;
            app.set_status("Quit cancelled");
        }
        _ => {}
    }

    Ok(())
}

fn handle_help(app: &mut App, key: KeyEvent) -> io::Result<()> {
    // Determine what mode to return to when closing help
    let return_mode = if app.find_state.is_some() {
        Mode::Find
    } else if app.grep_state.is_some() {
        Mode::Grep
    } else {
        Mode::Normal
    };

    match key.code {
        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter => {
            app.mode = return_mode;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = return_mode;
        }
        _ => {}
    }

    Ok(())
}
