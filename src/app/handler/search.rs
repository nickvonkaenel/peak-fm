//! Find / grep / search mode key handling

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::input::Mode;

use super::{calculate_half_page_scroll, handle_preview_scroll_keys};

pub(super) fn handle_search(app: &mut App, key: KeyEvent) -> io::Result<()> {
    match key.code {
        KeyCode::Enter => {
            app.confirm_search();
        }
        KeyCode::Esc => {
            app.cancel_search();
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cancel_search();
        }
        KeyCode::Backspace => {
            app.search_pop_char();
        }
        KeyCode::Char(c) => {
            app.search_push_char(c);
        }
        _ => {}
    }

    Ok(())
}

pub(super) fn handle_find(app: &mut App, key: KeyEvent) -> io::Result<()> {
    // Handle preview scrolling early
    if handle_preview_scroll_keys(app, key) {
        return Ok(());
    }

    match (key.code, key.modifiers) {
        // Exit
        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.exit_find_mode();
        }
        // Navigate with Ctrl+j/k or arrows (wraps around)
        (KeyCode::Char('j'), KeyModifiers::CONTROL) | (KeyCode::Down, _) => {
            app.find_move(1, true);
        }
        (KeyCode::Char('k'), KeyModifiers::CONTROL) | (KeyCode::Up, _) => {
            app.find_move(-1, true);
        }
        // Half page scroll (Ctrl+d/u) - stops at boundaries
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
            let half_page = calculate_half_page_scroll();
            app.find_move(half_page as isize, false);
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            let half_page = calculate_half_page_scroll();
            app.find_move(-(half_page as isize), false);
        }
        // Toggle hidden directories (Ctrl+.)
        (KeyCode::Char('.'), KeyModifiers::CONTROL) => {
            app.find_toggle_hidden();
        }
        // Settings popup (Ctrl+g)
        (KeyCode::Char('g'), KeyModifiers::CONTROL) => {
            app.mode = Mode::Settings;
        }
        // Open selected item in editor (Ctrl+e)
        (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
            if let Some(path) = app
                .find_state
                .as_ref()
                .and_then(|s| s.selected_path().cloned())
            {
                app.open_path_in_editor(&path)?;
            }
        }
        // Reveal selected item in Finder (Ctrl+o)
        (KeyCode::Char('o'), KeyModifiers::CONTROL) => {
            if let Some(path) = app
                .find_state
                .as_ref()
                .and_then(|s| s.selected_path().cloned())
            {
                app.reveal_in_finder(&path);
            }
        }
        // Play audio (Ctrl+p)
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            app.toggle_audio_from_find();
        }
        // Select with Enter
        (KeyCode::Enter, _) => {
            if app.find_select()? {
                // File was opened, terminal needs reinit
                return Ok(());
            }
        }
        // Nvim mode: split/vsplit/tab shortcuts
        (KeyCode::Char('s'), KeyModifiers::CONTROL) if app.nvim_mode => {
            app.find_select_split();
        }
        (KeyCode::Char('v'), KeyModifiers::CONTROL) if app.nvim_mode => {
            app.find_select_vsplit();
        }
        (KeyCode::Char('t'), KeyModifiers::CONTROL) if app.nvim_mode => {
            app.find_select_tab();
        }
        // Navigate to file location without opening (Ctrl+n)
        (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
            app.find_navigate()?;
        }
        // Delete character
        (KeyCode::Backspace, _) | (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
            app.find_pop_char();
        }
        // Delete word (Ctrl+w)
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            app.find_delete_word();
        }
        // Clear query (Ctrl+l)
        (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
            app.find_clear();
        }
        // Type character
        (KeyCode::Char(c), mods) if !mods.contains(KeyModifiers::CONTROL) => {
            app.find_push_char(c);
        }
        _ => {}
    }

    Ok(())
}

pub(super) fn handle_grep(app: &mut App, key: KeyEvent) -> io::Result<()> {
    // Handle preview scrolling early
    if handle_preview_scroll_keys(app, key) {
        return Ok(());
    }

    match (key.code, key.modifiers) {
        // Exit
        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.exit_grep_mode();
        }
        // Open selected match with Enter
        (KeyCode::Enter, _) => {
            if app
                .grep_state
                .as_ref()
                .map(|s| !s.matches.is_empty())
                .unwrap_or(false)
                && app.grep_select()?
            {
                return Ok(());
            }
        }
        // Nvim mode: split/vsplit/tab shortcuts
        (KeyCode::Char('s'), KeyModifiers::CONTROL) if app.nvim_mode => {
            app.grep_select_split();
        }
        (KeyCode::Char('v'), KeyModifiers::CONTROL) if app.nvim_mode => {
            app.grep_select_vsplit();
        }
        (KeyCode::Char('t'), KeyModifiers::CONTROL) if app.nvim_mode => {
            app.grep_select_tab();
        }
        // Navigate to file location without opening (Ctrl+n)
        (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
            app.grep_navigate()?;
        }
        // Navigate with Ctrl+j/k or arrows (wraps around)
        (KeyCode::Char('j'), KeyModifiers::CONTROL) | (KeyCode::Down, _) => {
            app.grep_move(1, true);
        }
        (KeyCode::Char('k'), KeyModifiers::CONTROL) | (KeyCode::Up, _) => {
            app.grep_move(-1, true);
        }
        // Half page scroll - stops at boundaries
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
            let half_page = calculate_half_page_scroll();
            app.grep_move(half_page as isize, false);
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            let half_page = calculate_half_page_scroll();
            app.grep_move(-(half_page as isize), false);
        }
        // Delete character
        (KeyCode::Backspace, _) | (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
            app.grep_pop_char();
        }
        // Delete word
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            app.grep_delete_word();
        }
        // Clear query (Ctrl+l)
        (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
            app.grep_clear();
        }
        // Search settings (Ctrl+g)
        (KeyCode::Char('g'), KeyModifiers::CONTROL) => {
            app.mode = Mode::Settings;
        }
        // Type character
        (KeyCode::Char(c), mods) if !mods.contains(KeyModifiers::CONTROL) => {
            app.grep_push_char(c);
        }
        _ => {}
    }

    Ok(())
}
