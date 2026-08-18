//! Visual and visual-insert mode key handling

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::input::{Mode, VisualEditType};

pub(super) fn handle_visual(app: &mut App, key: KeyEvent, anchor: usize) -> io::Result<()> {
    match key.code {
        // Exit visual mode
        KeyCode::Esc | KeyCode::Char('v') | KeyCode::Char('V') => {
            app.exit_visual_mode();
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.exit_visual_mode();
        }

        // Navigation (extends selection)
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
        KeyCode::Char('g') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.current.set_cursor(0);
            app.refresh_preview();
        }
        KeyCode::Char('G') => {
            let len = app.current.buffer.len();
            if len > 0 {
                app.current.set_cursor(len - 1);
                app.refresh_preview();
            }
        }

        // Half page scroll
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

        // Yank selected
        KeyCode::Char('y') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.yank_visual_selection(anchor);
        }

        // Copy selected files to clipboard
        KeyCode::Char('C') => {
            app.copy_visual_selection_to_clipboard(anchor);
        }

        // Copy selected files to clipboard and activate Reaper
        KeyCode::Char('R') => {
            app.copy_visual_selection_and_activate_reaper(anchor);
        }

        // Copy selected paths as text to clipboard
        KeyCode::Char('P') => {
            app.copy_visual_selection_paths_to_clipboard(anchor);
        }

        // Delete selected
        KeyCode::Char('d') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.delete_visual_selection(anchor);
        }

        // Insert modes for multi-file editing
        KeyCode::Char('i') | KeyCode::Char('I') => {
            app.enter_visual_insert(anchor, VisualEditType::Start);
        }
        KeyCode::Char('a') => {
            app.enter_visual_insert(anchor, VisualEditType::BeforeExt);
        }
        KeyCode::Char('A') => {
            app.enter_visual_insert(anchor, VisualEditType::End);
        }

        // Insert new line (single line, exits visual)
        KeyCode::Char('o') => {
            app.exit_visual_mode();
            app.insert_line_below();
        }
        KeyCode::Char('O') => {
            app.exit_visual_mode();
            app.insert_line_above();
        }

        // Quit
        KeyCode::Char('q') => {
            app.exit_visual_mode();
            app.capture_current_operations();

            if app.global_operations.total_count() > 0 {
                app.mode = Mode::QuitConfirm { scroll: 0 };
            } else {
                app.should_quit = true;
            }
        }

        // Help menu
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = Mode::Help;
        }

        // Sync changes
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.exit_visual_mode();
            app.capture_current_operations();

            let total_ops = app.global_operations.total_count();
            if total_ops == 0 {
                app.status = "No changes to sync".to_string();
            } else {
                app.mode = Mode::SyncConfirm { scroll: 0 };
            }
        }

        // Mark/unmark all selected files
        KeyCode::Char(';') => {
            let (start, end) = app.visual_selection_range(anchor);
            let mut toggled = 0;

            for i in start..=end {
                if let Some(line) = app.current.buffer.lines.get(i) {
                    if line.id.is_some() {
                        let path = app.current.buffer.path.join(&line.text);
                        if app.marked_files.contains(&path) {
                            app.marked_files.remove(&path);
                        } else {
                            app.marked_files.insert(path);
                        }
                        toggled += 1;
                    }
                }
            }

            app.set_status(format!("Toggled marks for {} file(s)", toggled));
        }

        // Toggle hidden files
        KeyCode::Char('.') => {
            app.exit_visual_mode();
            app.toggle_hidden()?;
        }

        // Toggle wrap
        KeyCode::Char('w') => {
            app.toggle_wrap();
        }

        // Toggle line numbers
        KeyCode::Char('#') => {
            app.toggle_line_numbers();
        }

        // Scroll preview
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

        // Leader menu (exit visual mode first)
        KeyCode::Char(' ') => {
            app.exit_visual_mode();
            app.mode = Mode::Leader;
        }

        _ => {}
    }

    Ok(())
}

pub(super) fn handle_visual_insert(
    app: &mut App,
    key: KeyEvent,
    anchor: usize,
    edit_type: VisualEditType,
) -> io::Result<()> {
    match (key.code, key.modifiers) {
        // Confirm edit
        (KeyCode::Enter, _) => {
            app.confirm_visual_insert(anchor, edit_type);
        }
        // Cancel
        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.cancel_visual_insert(anchor);
        }
        // Delete char
        (KeyCode::Backspace, _) | (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
            app.visual_edit_text.pop();
        }
        // Delete word
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            // Delete last word
            while app.visual_edit_text.ends_with(' ') {
                app.visual_edit_text.pop();
            }
            while !app.visual_edit_text.is_empty() && !app.visual_edit_text.ends_with(' ') {
                app.visual_edit_text.pop();
            }
        }
        // Clear line
        (KeyCode::Char('l'), KeyModifiers::CONTROL)
        | (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.visual_edit_text.clear();
        }
        // Type character
        (KeyCode::Char(c), mods) if !mods.contains(KeyModifiers::CONTROL) => {
            app.visual_edit_text.push(c);
        }
        _ => {}
    }

    Ok(())
}
