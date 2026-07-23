//! Normal mode key handling

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::core::is_audio_file;
use crate::fs;
use crate::input::{ConfirmAction, Mode, SearchDirection};

pub(super) fn handle_normal(app: &mut App, key: KeyEvent) -> io::Result<()> {
    match key.code {
        // Navigation
        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half_page = (app.current.height / 2).min(20);
            app.current.move_cursor(half_page as isize);
            app.refresh_preview();
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half_page = (app.current.height / 2).min(20);
            app.current.move_cursor(-(half_page as isize));
            app.refresh_preview();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.current.move_cursor(1);
            app.refresh_preview();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.current.move_cursor(-1);
            app.refresh_preview();
        }
        KeyCode::Char('l') => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                app.toggle_audio();
            } else {
                // Check if selected file is audio - if so, preview it instead of opening
                if let Some(path) = app.current.selected_path() {
                    if !path.is_dir() && is_audio_file(&path) {
                        app.toggle_audio();
                    } else if app.navigate_in()? {
                        return Ok(());
                    }
                }
            }
        }
        KeyCode::Right | KeyCode::Enter => {
            // Check if selected file is audio - if so, preview it instead of opening
            if let Some(path) = app.current.selected_path() {
                if !path.is_dir() && is_audio_file(&path) {
                    app.toggle_audio();
                } else if app.navigate_in()? {
                    return Ok(());
                }
            }
        }
        // Nvim mode: split/vsplit/tab shortcuts
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) && app.nvim_mode => {
            app.nvim_split();
        }
        // Sort menu moved to leader key (space+s)
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) && app.nvim_mode => {
            app.nvim_vsplit();
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) && app.nvim_mode => {
            app.nvim_tab();
        }
        KeyCode::Char('h') | KeyCode::Left => {
            app.navigate_out()?;
        }
        KeyCode::Char('-') => {
            app.toggle_prev_dir()?;
        }
        KeyCode::Char('.') => {
            app.toggle_hidden()?;
        }
        KeyCode::Char('e') => {
            app.open_cwd_in_editor()?;
            return Ok(());
        }
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(path) = app.current.selected_path() {
                app.reveal_in_finder(&path);
            }
        }
        // Audio mode (f) - opens audio browser in current directory
        KeyCode::Char('f') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.enter_audio_mode();
        }
        KeyCode::Char('z') => {
            app.enter_zoxide_mode();
        }
        KeyCode::Char('w') => {
            app.toggle_wrap();
        }
        KeyCode::Char('#') => {
            app.toggle_line_numbers();
        }
        // Trash toggle moved to leader key (space+t)
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // In trash: empty trash. Otherwise: set work directory to cwd
            let in_trash = fs::trash_dir().map(|t| app.cwd == t).unwrap_or(false);
            if in_trash {
                app.mode = Mode::Confirm(ConfirmAction::EmptyTrash);
            } else {
                app.set_work_dir_to_cwd();
            }
        }

        // Half page scroll (Ctrl+d/u)
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half_page = (app.current.height / 2).min(20);
            app.current.move_cursor(half_page as isize);
            app.refresh_preview();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half_page = (app.current.height / 2).min(20);
            app.current.move_cursor(-(half_page as isize));
            app.refresh_preview();
        }

        // Preview scroll (Ctrl+f/b)
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let page = if app.preview_height > 0 {
                app.preview_height / 2
            } else {
                10
            };
            app.scroll_preview(page.max(1) as isize);
        }
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let page = if app.preview_height > 0 {
                app.preview_height / 2
            } else {
                10
            };
            app.scroll_preview(-(page.max(1) as isize));
        }

        // Jump to top/bottom
        KeyCode::Home | KeyCode::Char('t') => {
            app.current.set_cursor(0);
            app.refresh_preview();
        }
        KeyCode::End | KeyCode::Char('b') => {
            let len = app.current.buffer.len();
            if len > 0 {
                app.current.set_cursor(len - 1);
                app.refresh_preview();
            }
        }
        // Shift+S: search in current directory only
        KeyCode::Char('S') => {
            app.enter_find_mode_cwd();
        }
        // Shift+G: grep in current directory only
        KeyCode::Char('G') => {
            app.enter_grep_mode_cwd();
        }

        // Editing
        KeyCode::Char('i') | KeyCode::Char('I') => {
            app.enter_insert_mode_start();
        }
        KeyCode::Char('a') => {
            app.enter_insert_mode_before_ext();
        }
        KeyCode::Char('A') => {
            app.enter_insert_mode_end();
        }
        KeyCode::Char('c') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.enter_insert_mode_clear();
        }
        KeyCode::Char('o') => {
            app.insert_line_below();
        }
        KeyCode::Char('O') => {
            app.insert_line_above();
        }
        KeyCode::Char('d') => {
            app.delete_line();
        }
        // Delete all marked files (Shift+D)
        KeyCode::Char('D') => {
            app.delete_marked();
        }

        // Restore from trash (only while browsing the trash directory)
        KeyCode::Char('x') if app.in_trash() => {
            app.restore_selected_from_trash();
        }
        KeyCode::Char('X') if app.in_trash() => {
            app.restore_marked_from_trash();
        }

        // Yank/Paste
        KeyCode::Char('y') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.yank_selected();
        }
        // Yank all marked files (Shift+Y)
        KeyCode::Char('Y') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.yank_marked();
        }
        KeyCode::Char(' ') => {
            // Enter leader mode
            app.mode = Mode::Leader;
        }
        // Run a shell command on the selection (! ...)
        KeyCode::Char('!') => {
            app.command_input.clear();
            app.mode = Mode::Command;
        }
        KeyCode::Char('p') => {
            app.paste();
        }

        // Undo (u)
        KeyCode::Char('u') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.undo();
        }
        // Redo (Ctrl+r)
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.redo();
        }

        // Sync (Ctrl+y)
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Capture current directory operations first
            app.capture_current_operations();

            // Check if there are any operations globally
            let total_ops = app.global_operations.total_count();
            if total_ops == 0 {
                app.status = "No changes to sync".to_string();
            } else {
                app.mode = Mode::SyncConfirm { scroll: 0 };
            }
        }
        // Copy file to clipboard (Shift+C)
        KeyCode::Char('C') => {
            app.copy_file_to_clipboard();
        }
        // Copy file to clipboard and activate Reaper (Shift+R)
        KeyCode::Char('R') => {
            app.copy_file_and_activate_reaper();
        }

        // Quit (q or Ctrl+c)
        KeyCode::Char('q') => {
            // Capture current operations first
            app.capture_current_operations();

            if app.global_operations.total_count() > 0 {
                app.mode = Mode::QuitConfirm { scroll: 0 };
            } else {
                app.should_quit = true;
            }
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Capture current operations first
            app.capture_current_operations();

            if app.global_operations.total_count() > 0 {
                app.mode = Mode::QuitConfirm { scroll: 0 };
            } else {
                app.should_quit = true;
            }
        }
        KeyCode::Esc => {
            // Stop audio if playing, clear marks if any exist, otherwise quit
            if app.is_audio_active() {
                app.stop_audio();
            } else if app.mark_count() > 0 {
                app.clear_marks();
                app.set_status("Marks cleared");
            } else {
                // Capture current operations first
                app.capture_current_operations();

                if app.global_operations.total_count() > 0 {
                    app.mode = Mode::QuitConfirm { scroll: 0 };
                } else {
                    app.should_quit = true;
                }
            }
        }

        // Mark/unmark files for batch operations
        KeyCode::Char(';') => {
            app.toggle_mark();
        }

        // Search
        KeyCode::Char('/') => {
            app.start_search(SearchDirection::Forward);
        }
        KeyCode::Char('n') => {
            app.search_next();
        }
        KeyCode::Char('N') => {
            app.search_prev();
        }

        // Help (Ctrl+g)
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = Mode::Help;
        }

        // Visual mode (line selection)
        KeyCode::Char('v') | KeyCode::Char('V') => {
            app.enter_visual_mode();
        }

        // Find mode (fuzzy file search)
        KeyCode::Char('s') => {
            app.enter_find_mode();
        }
        // Grep mode (content search)
        KeyCode::Char('g') => {
            app.enter_grep_mode();
        }
        // Resume last search
        KeyCode::Char('r') => {
            app.resume_last_search();
        }

        _ => {}
    }

    Ok(())
}
