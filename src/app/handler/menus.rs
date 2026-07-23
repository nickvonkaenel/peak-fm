//! Menu-mode key handling (leader, sort, settings, theme, info, preview options)

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::config::Config;
use crate::core::{available_themes, set_theme, SortOption};
use crate::fs;
use crate::input::{ConfirmAction, Mode};

use super::handle_menu_navigation;

pub(super) fn handle_theme_select(app: &mut App, key: KeyEvent, selected: usize) -> io::Result<()> {
    let themes = available_themes();
    let len = themes.len();

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('T') => {
            // Revert to original theme
            if let Some(ref original) = app.original_theme {
                set_theme(original);
                app.refresh_preview();
            }
            app.original_theme = None;
            app.mode = Mode::Normal;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Revert to original theme
            if let Some(ref original) = app.original_theme {
                set_theme(original);
                app.refresh_preview();
            }
            app.original_theme = None;
            app.mode = Mode::Normal;
        }
        KeyCode::Enter | KeyCode::Char('l') => {
            if let Some(name) = themes.get(selected) {
                app.select_theme(name);
            }
            // Clear original theme (committing to new theme)
            app.original_theme = None;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let new_selected = if selected + 1 >= len { 0 } else { selected + 1 };
            // Apply theme immediately for preview
            if let Some(name) = themes.get(new_selected) {
                set_theme(name);
                app.refresh_preview();
            }
            app.mode = Mode::ThemeSelect {
                selected: new_selected,
            };
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let new_selected = if selected == 0 {
                len.saturating_sub(1)
            } else {
                selected - 1
            };
            // Apply theme immediately for preview
            if let Some(name) = themes.get(new_selected) {
                set_theme(name);
                app.refresh_preview();
            }
            app.mode = Mode::ThemeSelect {
                selected: new_selected,
            };
        }
        _ => {}
    }

    Ok(())
}

pub(super) fn handle_settings(app: &mut App, key: KeyEvent) -> io::Result<()> {
    // Track which search mode we came from
    let was_in_find = app.find_state.is_some();
    let was_in_grep = app.grep_state.is_some();

    // Helper to return to the correct mode
    let return_mode = if was_in_find {
        Mode::Find
    } else if was_in_grep {
        Mode::Grep
    } else {
        Mode::Find // Fallback (shouldn't happen since settings only opens from search modes)
    };

    match key.code {
        // Close popup
        KeyCode::Esc | KeyCode::Char('q') => {
            app.mode = return_mode;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = return_mode;
        }
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = return_mode;
        }

        // Open help
        KeyCode::Char('h') => {
            app.mode = Mode::Help;
        }

        // Toggle word wrap
        KeyCode::Char('w') => {
            app.toggle_wrap();
            app.mode = return_mode;
        }

        // Toggle line numbers
        KeyCode::Char('l') => {
            app.toggle_line_numbers();
            app.mode = return_mode;
        }

        // Toggle hidden files
        KeyCode::Char('.') => {
            if was_in_find {
                app.find_toggle_hidden();
            } else {
                let _ = app.toggle_hidden();
            }
            app.mode = return_mode;
        }

        // Toggle gitignore
        KeyCode::Char('i') => {
            app.find_toggle_gitignore();
            app.mode = return_mode;
        }

        // Toggle directories
        KeyCode::Char('d') => {
            app.find_toggle_directories();
            app.mode = return_mode;
        }

        // Toggle navigate on open
        KeyCode::Char('n') => {
            app.toggle_search_navigate_on_open();
            app.mode = return_mode;
        }

        _ => {}
    }

    Ok(())
}

pub(super) fn handle_sort(
    app: &mut App,
    key: KeyEvent,
    selected: usize,
    is_global: bool,
) -> io::Result<()> {
    // Sort options list (matches render order)
    let sort_options = [
        SortOption::Name,
        SortOption::NameDesc,
        SortOption::DateModified,
        SortOption::DateModifiedAsc,
        SortOption::Size,
        SortOption::SizeAsc,
        SortOption::Extension,
        SortOption::ExtensionDesc,
    ];

    // Handle menu navigation
    if let Some(new_selected) = handle_menu_navigation(key, selected, sort_options.len()) {
        app.mode = Mode::Sort {
            selected: new_selected,
            is_global,
        };
        return Ok(());
    }

    match (key.code, key.modifiers) {
        // Exit sort menu
        (KeyCode::Esc, _)
        | (KeyCode::Char('q'), _)
        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.mode = Mode::Normal;
        }
        // Select current option
        (KeyCode::Enter, _) | (KeyCode::Char('l'), _) => {
            if selected < sort_options.len() {
                if is_global {
                    app.set_sort_option(sort_options[selected])?;
                } else {
                    app.set_dir_sort_option(sort_options[selected])?;
                }
            }
        }
        // Name (A-Z) - 'a' for alphabetical
        (KeyCode::Char('a'), _) => {
            if is_global {
                app.set_sort_option(SortOption::Name)?;
            } else {
                app.set_dir_sort_option(SortOption::Name)?;
            }
        }
        // Name (Z-A) - descending
        (KeyCode::Char('A'), _) => {
            if is_global {
                app.set_sort_option(SortOption::NameDesc)?;
            } else {
                app.set_dir_sort_option(SortOption::NameDesc)?;
            }
        }
        // Date modified (newest first) - descending is common
        (KeyCode::Char('d'), _) => {
            if is_global {
                app.set_sort_option(SortOption::DateModified)?;
            } else {
                app.set_dir_sort_option(SortOption::DateModified)?;
            }
        }
        // Date modified (oldest first) - ascending
        (KeyCode::Char('D'), _) => {
            if is_global {
                app.set_sort_option(SortOption::DateModifiedAsc)?;
            } else {
                app.set_dir_sort_option(SortOption::DateModifiedAsc)?;
            }
        }
        // Size (largest first) - descending is common
        (KeyCode::Char('s'), _) => {
            if is_global {
                app.set_sort_option(SortOption::Size)?;
            } else {
                app.set_dir_sort_option(SortOption::Size)?;
            }
        }
        // Size (smallest first) - ascending
        (KeyCode::Char('S'), _) => {
            if is_global {
                app.set_sort_option(SortOption::SizeAsc)?;
            } else {
                app.set_dir_sort_option(SortOption::SizeAsc)?;
            }
        }
        // Extension (A-Z) - ascending is common for alphabetical
        (KeyCode::Char('e'), _) => {
            if is_global {
                app.set_sort_option(SortOption::Extension)?;
            } else {
                app.set_dir_sort_option(SortOption::Extension)?;
            }
        }
        // Extension (Z-A) - descending
        (KeyCode::Char('E'), _) => {
            if is_global {
                app.set_sort_option(SortOption::ExtensionDesc)?;
            } else {
                app.set_dir_sort_option(SortOption::ExtensionDesc)?;
            }
        }
        _ => {}
    }

    Ok(())
}

pub(super) fn handle_info_select(app: &mut App, key: KeyEvent, selected: usize) -> io::Result<()> {
    use crate::core::DisplayInfo;

    let display_options = [
        DisplayInfo::None,
        DisplayInfo::DateModified,
        DisplayInfo::Size,
        DisplayInfo::Mode,
        DisplayInfo::Extension,
    ];

    // Handle menu navigation
    if let Some(new_selected) = handle_menu_navigation(key, selected, display_options.len()) {
        app.mode = Mode::InfoSelect {
            selected: new_selected,
        };
        return Ok(());
    }

    match (key.code, key.modifiers) {
        // Exit info menu
        (KeyCode::Esc, _)
        | (KeyCode::Char('q'), _)
        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.mode = Mode::Normal;
        }
        // Select current option
        (KeyCode::Enter, _) | (KeyCode::Char('l'), _) => {
            if selected < display_options.len() {
                app.display_info = display_options[selected];
                app.mode = Mode::Normal;
                app.refresh_current_dir()?;
            }
        }
        // Quick keys
        (KeyCode::Char('n'), _) => {
            app.display_info = DisplayInfo::None;
            app.mode = Mode::Normal;
            app.refresh_current_dir()?;
        }
        (KeyCode::Char('d'), _) => {
            app.display_info = DisplayInfo::DateModified;
            app.mode = Mode::Normal;
            app.refresh_current_dir()?;
        }
        (KeyCode::Char('s'), _) => {
            app.display_info = DisplayInfo::Size;
            app.mode = Mode::Normal;
            app.refresh_current_dir()?;
        }
        (KeyCode::Char('m'), _) => {
            app.display_info = DisplayInfo::Mode;
            app.mode = Mode::Normal;
            app.refresh_current_dir()?;
        }
        (KeyCode::Char('e'), _) => {
            app.display_info = DisplayInfo::Extension;
            app.mode = Mode::Normal;
            app.refresh_current_dir()?;
        }
        _ => {}
    }

    Ok(())
}

pub(super) fn handle_leader(app: &mut App, key: KeyEvent) -> io::Result<()> {
    match (key.code, key.modifiers) {
        // Exit leader mode
        (KeyCode::Esc, _)
        | (KeyCode::Char(' '), _)
        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.mode = Mode::Normal;
        }
        // Quit (q)
        (KeyCode::Char('q'), _) => {
            app.capture_current_operations();
            if app.global_operations.total_count() > 0 {
                app.mode = Mode::QuitConfirm { scroll: 0 };
            } else {
                app.should_quit = true;
            }
        }
        // Audio browser (a)
        (KeyCode::Char('a'), _) => {
            app.enter_audio_mode();
        }
        // Colors/Theme (c)
        (KeyCode::Char('c'), m) if !m.contains(KeyModifiers::CONTROL) => {
            app.open_theme_selector();
        }
        // FFmpeg editor (f)
        (KeyCode::Char('f'), _) => {
            app.open_ffmpeg_editor()?;
            app.mode = Mode::Normal;
        }
        // Info display (i)
        (KeyCode::Char('i'), _) => {
            app.mode = Mode::InfoSelect { selected: 0 };
        }
        // Lazygit (l)
        (KeyCode::Char('l'), _) => {
            app.launch_lazygit()?;
            app.mode = Mode::Normal;
        }
        // Per-directory sort (s)
        (KeyCode::Char('s'), m) if !m.contains(KeyModifiers::SHIFT) => {
            app.mode = Mode::Sort {
                selected: 0,
                is_global: false,
            };
        }
        // Global sort (Shift+S)
        (KeyCode::Char('S'), _) => {
            app.mode = Mode::Sort {
                selected: 0,
                is_global: true,
            };
        }
        // Toggle trash (t)
        (KeyCode::Char('t'), m) if !m.contains(KeyModifiers::SHIFT) => {
            app.toggle_trash()?;
            app.mode = Mode::Normal;
        }
        // Empty trash (Shift+T)
        (KeyCode::Char('T'), _) => {
            let in_trash = fs::trash_dir().map(|t| app.cwd == t).unwrap_or(false);
            if in_trash {
                app.mode = Mode::Confirm(ConfirmAction::EmptyTrash);
            } else {
                app.set_status("Navigate to trash to empty it");
                app.mode = Mode::Normal;
            }
        }
        // Preview options (u)
        (KeyCode::Char('u'), _) => {
            app.mode = Mode::PreviewOptions;
        }
        // Git menu (g)
        (KeyCode::Char('g'), _) => {
            app.mode = Mode::Git;
        }
        // Toggle hidden (.)
        (KeyCode::Char('.'), _) => {
            app.toggle_hidden()?;
            app.mode = Mode::Normal;
        }
        // Commit all (p)
        (KeyCode::Char('p'), _) => {
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

pub(super) fn handle_preview_options(app: &mut App, key: KeyEvent) -> io::Result<()> {
    match (key.code, key.modifiers) {
        // Exit preview options menu
        (KeyCode::Esc, _)
        | (KeyCode::Char('q'), _)
        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.mode = Mode::Normal;
        }
        // Toggle wrap (w)
        (KeyCode::Char('w'), _) => {
            app.toggle_wrap();
            app.mode = Mode::Normal;
        }
        // Toggle line numbers (l)
        (KeyCode::Char('l'), _) => {
            app.toggle_line_numbers();
            app.mode = Mode::Normal;
        }
        // Toggle icons (i)
        (KeyCode::Char('i'), _) => {
            app.toggle_icons();
            app.mode = Mode::Normal;
        }
        // Toggle icon colors (c)
        (KeyCode::Char('c'), m) if !m.contains(KeyModifiers::CONTROL) => {
            app.toggle_icon_colors();
            app.mode = Mode::Normal;
        }
        // Toggle theme icon colors (t)
        (KeyCode::Char('t'), _) => {
            app.toggle_theme_icons();
            app.mode = Mode::Normal;
        }
        _ => {}
    }

    Ok(())
}
