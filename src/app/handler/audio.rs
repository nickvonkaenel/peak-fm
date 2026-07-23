//! Audio browser mode key handling

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;

/// Execute audio state method if audio mode is active
macro_rules! audio_cmd {
    ($app:expr, $method:ident) => {
        if let Some(state) = &mut $app.audio_state {
            state.$method();
        }
    };
    ($app:expr, $method:ident, $($arg:expr),+) => {
        if let Some(state) = &mut $app.audio_state {
            state.$method($($arg),+);
        }
    };
}

/// Execute audio state method only in browse mode
macro_rules! browse_cmd {
    ($app:expr, $method:ident) => {
        if let Some(state) = &mut $app.audio_state {
            if state.browse_mode {
                state.$method();
            }
        }
    };
    ($app:expr, $method:ident, $($arg:expr),+) => {
        if let Some(state) = &mut $app.audio_state {
            if state.browse_mode {
                state.$method($($arg),+);
            }
        }
    };
}

/// Copy audio file path to clipboard and show status
fn audio_copy_to_clipboard(app: &mut App) {
    if let Some(state) = &mut app.audio_state {
        if let Some(file) = state.selected_file() {
            let path = file.path.to_string_lossy().to_string();
            match crate::app::clipboard::copy_path_to_clipboard(&path) {
                Ok(()) => state.set_status("Copied to clipboard".to_string()),
                Err(e) => state.set_status(format!("Clipboard error: {}", e)),
            }
        }
    }
}

/// Copy audio file path to clipboard, stop playback, and activate Reaper
fn audio_copy_and_activate_reaper(app: &mut App) {
    if let Some(state) = &mut app.audio_state {
        if let Some(file) = state.selected_file() {
            let path = file.path.to_string_lossy().to_string();
            match crate::app::clipboard::copy_path_to_clipboard(&path) {
                Ok(()) => {
                    state.stop();
                    let _ = crate::app::clipboard::activate_reaper();
                    state.set_status("Copied to clipboard, opening Reaper".to_string());
                }
                Err(e) => state.set_status(format!("Clipboard error: {}", e)),
            }
        }
    }
}

pub(super) fn handle_audio(app: &mut App, key: KeyEvent) -> io::Result<()> {
    // Get visible height for scroll calculations (roughly half terminal height)
    let (_, term_height) = crossterm::terminal::size().unwrap_or((80, 24));
    let visible_height = (term_height / 2).saturating_sub(4) as usize;

    // Check if we're in browse mode
    let browse_mode = app
        .audio_state
        .as_ref()
        .map(|s| s.browse_mode)
        .unwrap_or(false);

    match (key.code, key.modifiers) {
        // Ctrl+C always exits to normal mode
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.exit_audio_mode();
            if app.pick_mode {
                app.should_quit = true;
            }
        }

        // Esc: toggle between audio and browse modes
        (KeyCode::Esc, _) => {
            if let Some(state) = &mut app.audio_state {
                if state.browse_mode {
                    // Browse → Audio: clear search
                    state.clear_search();
                    state.browse_mode = false;
                } else {
                    // Audio → Browse
                    state.browse_mode = true;
                }
            }
        }

        // In browse mode, 'a' goes back to audio mode (keep search)
        (KeyCode::Char('a'), KeyModifiers::NONE) if browse_mode => {
            if let Some(state) = &mut app.audio_state {
                state.browse_mode = false;
            }
        }

        // In browse mode, 'i' goes back to audio mode and clears search
        (KeyCode::Char('i'), KeyModifiers::NONE) if browse_mode => {
            if let Some(state) = &mut app.audio_state {
                state.clear_search();
                state.browse_mode = false;
            }
        }

        // Browse mode navigation (no Ctrl needed)
        (KeyCode::Char('j'), KeyModifiers::NONE) if browse_mode => {
            browse_cmd!(app, move_down);
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) if browse_mode => {
            browse_cmd!(app, move_up);
        }
        (KeyCode::Char('d'), KeyModifiers::NONE) if browse_mode => {
            browse_cmd!(app, move_half_page_down, visible_height);
        }
        (KeyCode::Char('u'), KeyModifiers::NONE) if browse_mode => {
            browse_cmd!(app, move_half_page_up, visible_height);
        }
        (KeyCode::Char('h'), KeyModifiers::NONE) if browse_mode => {
            browse_cmd!(app, seek_prev_region);
        }
        (KeyCode::Char('l'), KeyModifiers::NONE) if browse_mode => {
            browse_cmd!(app, seek_next_region);
        }
        (KeyCode::Char('t'), KeyModifiers::NONE) if browse_mode => {
            browse_cmd!(app, move_to_top);
        }
        (KeyCode::Char('b'), KeyModifiers::NONE) if browse_mode => {
            browse_cmd!(app, move_to_bottom);
        }
        (KeyCode::Char('y'), KeyModifiers::NONE) if browse_mode => {
            audio_copy_to_clipboard(app);
        }
        (KeyCode::Char('r'), KeyModifiers::NONE) if browse_mode => {
            audio_copy_and_activate_reaper(app);
        }

        // Volume controls in browse mode (+/-)
        (KeyCode::Char('='), _) if browse_mode => {
            browse_cmd!(app, volume_up);
        }
        (KeyCode::Char('+'), _) if browse_mode => {
            browse_cmd!(app, volume_up);
        }
        (KeyCode::Char('-'), KeyModifiers::NONE) if browse_mode => {
            browse_cmd!(app, volume_down);
        }

        // Pitch controls in browse mode
        (KeyCode::Char('['), KeyModifiers::NONE) if browse_mode => {
            browse_cmd!(app, pitch_down);
        }
        (KeyCode::Char(']'), KeyModifiers::NONE) if browse_mode => {
            browse_cmd!(app, pitch_up);
        }
        (KeyCode::Char('{'), KeyModifiers::SHIFT) if browse_mode => {
            browse_cmd!(app, pitch_down_octave);
        }
        (KeyCode::Char('}'), KeyModifiers::SHIFT) if browse_mode => {
            browse_cmd!(app, pitch_up_octave);
        }

        // Space in browse mode always toggles play/pause
        (KeyCode::Char(' '), _) if browse_mode => {
            browse_cmd!(app, toggle_play_pause);
        }

        // Arrow keys work in both modes
        (KeyCode::Down, _) => {
            audio_cmd!(app, move_down);
        }
        (KeyCode::Up, _) => {
            audio_cmd!(app, move_up);
        }
        (KeyCode::Left, _) => {
            audio_cmd!(app, seek_prev_region);
        }
        (KeyCode::Right, _) => {
            audio_cmd!(app, seek_next_region);
        }

        // Audio mode: Ctrl+key navigation
        (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
            audio_cmd!(app, move_down);
        }
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
            audio_cmd!(app, move_up);
        }
        (KeyCode::Char('T'), KeyModifiers::SHIFT) => {
            audio_cmd!(app, move_to_top);
        }
        (KeyCode::Char('B'), KeyModifiers::SHIFT) => {
            audio_cmd!(app, move_to_bottom);
        }
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
            audio_cmd!(app, move_half_page_down, visible_height);
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            audio_cmd!(app, move_half_page_up, visible_height);
        }

        // Playback controls
        (KeyCode::Enter, _) => {
            if let Some(state) = &mut app.audio_state {
                state.play_selected();
                // Enter browse mode after playing
                state.browse_mode = true;
            }
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            audio_cmd!(app, toggle_play_pause);
        }
        (KeyCode::Char('s'), KeyModifiers::NONE) if browse_mode => {
            browse_cmd!(app, stop);
        }
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            audio_cmd!(app, stop);
        }

        // Volume controls (Shift+K/J)
        (KeyCode::Char('K'), KeyModifiers::SHIFT) => {
            audio_cmd!(app, volume_up);
            // Save volume to config
            if let Some(state) = &app.audio_state {
                let linear_volume = 10.0_f32.powf(state.volume_db / 20.0);
                app.audio_volume = linear_volume;
                let mut config = crate::config::Config::load();
                config.audio_volume = linear_volume;
                config.save();
            }
        }
        (KeyCode::Char('J'), KeyModifiers::SHIFT) => {
            audio_cmd!(app, volume_down);
            // Save volume to config
            if let Some(state) = &app.audio_state {
                let linear_volume = 10.0_f32.powf(state.volume_db / 20.0);
                app.audio_volume = linear_volume;
                let mut config = crate::config::Config::load();
                config.audio_volume = linear_volume;
                config.save();
            }
        }
        (KeyCode::Char('V'), KeyModifiers::SHIFT) => {
            audio_cmd!(app, reset_volume);
            // Save volume to config
            if let Some(state) = &app.audio_state {
                let linear_volume = 10.0_f32.powf(state.volume_db / 20.0);
                app.audio_volume = linear_volume;
                let mut config = crate::config::Config::load();
                config.audio_volume = linear_volume;
                config.save();
            }
        }

        // Pitch controls ([ and ])
        (KeyCode::Char('['), _) => {
            audio_cmd!(app, pitch_down);
        }
        (KeyCode::Char(']'), _) => {
            audio_cmd!(app, pitch_up);
        }
        // Pitch octave ({ and })
        (KeyCode::Char('{'), _) => {
            audio_cmd!(app, pitch_down_octave);
        }
        (KeyCode::Char('}'), _) => {
            audio_cmd!(app, pitch_up_octave);
        }
        // Reset pitch
        (KeyCode::Char('P'), KeyModifiers::SHIFT) => {
            audio_cmd!(app, reset_pitch);
        }

        // Search editing (audio mode only)
        (KeyCode::Backspace, _) if !browse_mode => {
            audio_cmd!(app, search_pop_char);
        }
        (KeyCode::Char('w'), KeyModifiers::CONTROL) if !browse_mode => {
            audio_cmd!(app, search_delete_word);
        }

        // Region navigation with Ctrl (audio mode)
        (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
            audio_cmd!(app, seek_prev_region);
        }
        (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
            audio_cmd!(app, seek_next_region);
        }

        // Copy file to clipboard (audio mode with Ctrl)
        (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
            audio_copy_to_clipboard(app);
        }

        // Copy file to clipboard and open Reaper (audio mode with Ctrl)
        (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
            audio_copy_and_activate_reaper(app);
        }
        (KeyCode::Char('i'), KeyModifiers::CONTROL) => {
            audio_cmd!(app, toggle_info);
        }

        // Toggles
        (KeyCode::Char('A'), KeyModifiers::SHIFT) => {
            audio_cmd!(app, toggle_autoplay);
            // Sync to app and save config
            if let Some(state) = &app.audio_state {
                app.audio_auto_play = state.autoplay;
                let mut config = crate::config::Config::load();
                config.audio_autoplay = state.autoplay;
                config.save();
            }
        }
        (KeyCode::Char('N'), KeyModifiers::SHIFT) => {
            audio_cmd!(app, toggle_normalize);
            // Sync to app and save config
            if let Some(state) = &app.audio_state {
                app.audio_normalize = state.normalize_waveform;
                let mut config = crate::config::Config::load();
                config.audio_normalize = state.normalize_waveform;
                config.save();
            }
        }
        (KeyCode::Char('S'), KeyModifiers::SHIFT) => {
            audio_cmd!(app, toggle_skip_silence);
            // Sync to app and save config
            if let Some(state) = &app.audio_state {
                app.audio_skip_silence = state.skip_silence;
                let mut config = crate::config::Config::load();
                config.audio_skip_silence = state.skip_silence;
                config.save();
            }
        }
        (KeyCode::Char('F'), KeyModifiers::SHIFT) => {
            let (term_width, _) = crossterm::terminal::size().unwrap_or((120, 24));
            if let Some(state) = &mut app.audio_state {
                state.toggle_analyzer(term_width as usize);
            }
        }
        // Toggle analyzer gradient mode (Ctrl+G)
        (KeyCode::Char('g'), KeyModifiers::CONTROL) => {
            if let Some(state) = &mut app.audio_state {
                state.toggle_analyzer_gradient();
                // Save gradient setting to config
                let mut config = crate::config::Config::load();
                config.audio_analyzer_gradient = state.analyzer_gradient;
                config.save();
            }
        }
        // Rebuild database (R)
        (KeyCode::Char('R'), KeyModifiers::SHIFT) => {
            audio_cmd!(app, rebuild_database);
        }

        // Shuffle results
        (KeyCode::Char('/'), _) => {
            audio_cmd!(app, shuffle);
        }

        // Jump to random result
        (KeyCode::Char('?'), _) => {
            audio_cmd!(app, jump_to_random);
        }

        // Type characters for search (audio mode only)
        (KeyCode::Char(c), mods)
            if !browse_mode
                && !mods.contains(KeyModifiers::CONTROL)
                && !mods.contains(KeyModifiers::ALT) =>
        {
            // Skip keys that have special functions (shift keys, symbols)
            if !matches!(
                c,
                'A' | 'N' | 'R' | 'P' | 'V' | 'T' | 'B' | 'K' | 'J' | '[' | ']' | '{' | '}'
            ) {
                if let Some(state) = &mut app.audio_state {
                    // Space has special handling
                    if c == ' ' {
                        // If search is empty or ends with space, toggle play/pause
                        if state.search_query.is_empty() || state.search_query.ends_with(' ') {
                            state.toggle_play_pause();
                        } else {
                            // Add space but don't re-filter (just append without update_filter)
                            state.search_query.push(' ');
                        }
                    } else {
                        state.search_push_char(c);
                    }
                }
            }
        }

        _ => {}
    }

    Ok(())
}
