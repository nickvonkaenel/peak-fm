//! Audio browser mode rendering — file browser, waveform, and frequency analyzer.

use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::audio::AudioModeState;
use crate::app::App;

/// Sanitize text by removing control characters and other unrenderable characters
fn sanitize_text(text: &str) -> String {
    text.chars()
        .filter(|c| {
            // Keep printable characters, spaces, and common Unicode
            // Filter out control characters (0x00-0x1F, 0x7F-0x9F) except tab/newline
            if c.is_control() && *c != '\t' && *c != '\n' {
                return false;
            }
            // Filter out replacement character and other problematic Unicode
            if *c == '\u{FFFD}' || *c == '\u{FFFE}' || *c == '\u{FFFF}' {
                return false;
            }
            // Filter out private use area characters
            let code = *c as u32;
            if (0xE000..=0xF8FF).contains(&code)
                || (0xF0000..=0xFFFFD).contains(&code)
                || (0x100000..=0x10FFFD).contains(&code)
            {
                return false;
            }
            true
        })
        .map(|c| if c == '\t' || c == '\n' { ' ' } else { c })
        .collect()
}

/// Wrap text to fit within a given width, breaking on word boundaries
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 || text.is_empty() {
        return vec![text.to_string()];
    }

    // Simple character-based wrapping - just break at max_width
    let chars: Vec<char> = text.chars().collect();
    let mut lines = Vec::new();
    let mut start = 0;

    while start < chars.len() {
        let end = (start + max_width).min(chars.len());
        lines.push(chars[start..end].iter().collect());
        start = end;
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}
/// Audio mode - full screen audio file browser (fsf integration)
pub(super) fn render_audio_mode(frame: &mut Frame, area: Rect, app: &mut App) {
    use ratatui::{
        layout::{Constraint, Layout},
        widgets::List,
    };

    // Get analyzer state
    let (show_waveform, show_analyzer) = if let Some(state) = &app.audio_state {
        (state.show_waveform, state.show_analyzer)
    } else {
        (true, false)
    };

    // Build layout constraints dynamically
    let mut layout_constraints = vec![Constraint::Min(5)]; // Content area

    // Hide waveform if terminal height is too small (< 25 lines)
    let available_height = area.height;
    let show_waveform_adjusted = show_waveform && available_height >= 25;

    if show_waveform_adjusted {
        // Scale waveform height based on terminal size
        // Min: 10 lines, Max: 15 lines
        // Scale based on available height (more height = taller waveform)
        let waveform_height = if available_height >= 50 {
            15 // Large terminal: 15 lines
        } else if available_height >= 40 {
            13 // Medium-large: 13 lines
        } else if available_height >= 30 {
            12 // Medium: 12 lines
        } else {
            10 // Small: 10 lines (minimum)
        };

        layout_constraints.push(Constraint::Length(waveform_height)); // Waveform (with embedded analyzer if wide enough)
    }

    layout_constraints.push(Constraint::Length(1)); // Status bar

    let main_chunks = Layout::vertical(layout_constraints).split(area);

    let content_area = main_chunks[0];
    let mut chunk_idx = 1;

    // Determine waveform area (analyzer will be embedded inside if enabled and wide enough)
    let waveform_area = if show_waveform_adjusted {
        let area = main_chunks[chunk_idx];
        chunk_idx += 1;
        Some(area)
    } else {
        None
    };

    let status_area = main_chunks[chunk_idx];

    // Determine layout mode for content based on terminal width
    let use_stacked_layout = content_area.width < 80;

    // Render status bar (matching file manager style)
    let mode_style = Style::default().fg(Color::Magenta);

    let (dir_display, status_msg, browse_mode, db_building) = if let Some(state) = &app.audio_state
    {
        let mut dir = state.scan_root.to_string_lossy().to_string();
        // Remove Windows extended-length path prefix \\?\
        if dir.starts_with(r"\\?\") {
            dir = dir[4..].to_string();
        }
        let msg = state.status_message.clone().unwrap_or_default();
        (dir, msg, state.browse_mode, state.db_building)
    } else {
        (String::new(), String::new(), false, false)
    };

    let mode_label = if browse_mode { " BROWSE " } else { " AUDIO " };

    let mut status_spans = vec![
        Span::styled(mode_label, mode_style),
        Span::raw(" "),
        Span::styled(&dir_display, Style::default().fg(Color::DarkGray)),
    ];

    // Show database building indicator
    if db_building {
        status_spans.push(Span::raw(" "));
        status_spans.push(Span::styled(
            "[Building DB]",
            Style::default().fg(Color::Cyan),
        ));
    }

    if !status_msg.is_empty() {
        status_spans.push(Span::raw(" "));
        status_spans.push(Span::styled(
            &status_msg,
            Style::default().fg(Color::Yellow),
        ));
    }
    let status = Line::from(status_spans);

    // Calculate space for right-aligned hint
    let hint = "Ctrl+i: info";
    let status_width: usize = status.spans.iter().map(|s| s.content.chars().count()).sum();
    let hint_width = hint.chars().count() + 1;
    let available = status_area.width as usize;

    let status_widget = Paragraph::new(status);
    frame.render_widget(status_widget, status_area);

    // Render hint on the right if there's space
    if available > status_width + hint_width + 2 {
        let hint_x = status_area.x + status_area.width - hint_width as u16;
        let hint_area = Rect::new(hint_x, status_area.y, hint_width as u16, 1);
        let hint_widget = Paragraph::new(Span::styled(
            format!("{} ", hint),
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(hint_widget, hint_area);
    }

    // Split content area based on layout mode
    let show_info = app
        .audio_state
        .as_ref()
        .map(|s| s.show_info)
        .unwrap_or(true);

    let (list_area, info_area_opt) = if show_info {
        if use_stacked_layout {
            // STACKED: Vertical split - list on top, info below
            // Calculate exact height needed for info section (always 25 rows)
            let metadata_height: u16 = 5;
            let controls_height: u16 = 20;
            let info_height = metadata_height + controls_height;

            let chunks = Layout::vertical([
                Constraint::Min(5),              // File list (gets remaining space)
                Constraint::Length(info_height), // Info section (exact height)
            ])
            .split(content_area);

            (chunks[0], Some(chunks[1]))
        } else {
            // SIDE-BY-SIDE: Horizontal split - list left, info right (max 30 columns)
            let chunks = Layout::horizontal([
                Constraint::Min(30), // File list (gets remaining space)
                Constraint::Max(30), // Info section (max 30 columns)
            ])
            .split(content_area);

            (chunks[0], Some(chunks[1]))
        }
    } else {
        (content_area, None)
    };

    // Split list area into search input + results list
    let list_chunks = Layout::vertical([
        Constraint::Length(3), // Input box
        Constraint::Min(1),    // Results list
    ])
    .split(list_area);

    let input_area = list_chunks[0];
    let results_area = list_chunks[1];

    // Update visible height for scrolling
    let visible_height = results_area.height.saturating_sub(2) as usize;
    if let Some(state) = &mut app.audio_state {
        state.adjust_scroll(visible_height);
    }

    // Render input box - highlighted in audio mode, dimmed in browse mode
    let query = app
        .audio_state
        .as_ref()
        .map(|s| s.search_query.as_str())
        .unwrap_or("");
    let input_text = format!("> {}", query);
    let search_color = if browse_mode {
        Color::Magenta
    } else {
        Color::Blue
    };
    let input_block = Block::default()
        .title(" Search ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(search_color));
    let input_paragraph = Paragraph::new(input_text).block(input_block);
    frame.render_widget(input_paragraph, input_area);

    // Set cursor position and style based on mode
    use crossterm::cursor::{Hide, SetCursorStyle, Show};
    use crossterm::execute;
    use std::io::stdout;

    // In browse mode, hide cursor. In audio mode, show blinking bar.
    if browse_mode {
        let _ = execute!(stdout(), Hide);
    } else {
        let _ = execute!(stdout(), Show, SetCursorStyle::BlinkingBar);
        // Position cursor in search input, clamped to input area bounds
        let cursor_x =
            (input_area.x + 3 + query.len() as u16).min(input_area.x + input_area.width - 1);
        let cursor_y = input_area.y + 1;
        frame.set_cursor_position((cursor_x, cursor_y));
    }

    // Render results list
    let (count, total) = app
        .audio_state
        .as_ref()
        .map(|s| (s.filtered_indices.len(), s.files.len()))
        .unwrap_or((0, 0));
    let title = if count == total {
        format!(" Audio Files ({}) ", total)
    } else {
        format!(" Audio Files ({}/{}) ", count, total)
    };
    // Results box - highlighted in browse mode, dimmed in audio mode
    let results_color = if browse_mode {
        Color::Blue
    } else {
        Color::Magenta
    };
    let results_block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(results_color));

    if let Some(state) = &app.audio_state {
        // Get visible slice
        let start = state.scroll_offset;
        let end = (start + visible_height).min(state.filtered_indices.len());

        let items: Vec<ratatui::widgets::ListItem> = state.filtered_indices[start..end]
            .iter()
            .enumerate()
            .map(|(vis_idx, &file_idx)| {
                let actual_idx = start + vis_idx;
                let is_selected = actual_idx == state.selected;

                let file = &state.files[file_idx];
                // Strip common audio extensions for cleaner display
                let display_name = file
                    .filename
                    .strip_suffix(".wav")
                    .or_else(|| file.filename.strip_suffix(".WAV"))
                    .or_else(|| file.filename.strip_suffix(".mp3"))
                    .or_else(|| file.filename.strip_suffix(".flac"))
                    .or_else(|| file.filename.strip_suffix(".aiff"))
                    .or_else(|| file.filename.strip_suffix(".aif"))
                    .unwrap_or(&file.filename);

                // Check if this file is currently playing or paused
                let is_this_file_active = state
                    .player
                    .as_ref()
                    .and_then(|p| p.current_file())
                    .map(|f| f == file.path)
                    .unwrap_or(false);
                let is_playing = state
                    .player
                    .as_ref()
                    .map(|p| p.is_playing())
                    .unwrap_or(false);
                let is_paused = state
                    .player
                    .as_ref()
                    .map(|p| p.is_paused())
                    .unwrap_or(false);

                // Show play/pause icon only when file is active, otherwise > for selected
                // Use U+FE0E variation selector to force text presentation (not emoji)
                let prefix = if is_selected {
                    if is_this_file_active && is_playing {
                        "▶\u{FE0E} " // Playing
                    } else if is_this_file_active && is_paused {
                        "⏸\u{FE0E} " // Paused
                    } else {
                        "> " // Selected but not playing
                    }
                } else {
                    "  "
                };

                // Selected file is always yellow, others are default
                let name_style = if is_selected {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };

                let mut spans = vec![
                    Span::styled(prefix, Style::default()),
                    Span::styled(display_name.to_string(), name_style),
                ];

                // Add description - dark gray (slightly darker when selected for contrast)
                if let Some(desc) = &file.description {
                    if !desc.is_empty() {
                        let clean_desc = sanitize_text(desc);
                        let desc_style = Style::default().fg(Color::DarkGray);
                        spans.push(Span::styled(format!(" {}", clean_desc), desc_style));
                    }
                }

                let line = Line::from(spans);
                ratatui::widgets::ListItem::new(line)
            })
            .collect();

        let list = List::new(items).block(results_block);
        frame.render_widget(list, results_area);
    } else {
        frame.render_widget(results_block, results_area);
    }

    // Info panel rendering (conditional on layout mode)
    if let Some(info_area) = info_area_opt {
        if use_stacked_layout {
            // STACKED LAYOUT: Horizontal two-column layout
            render_info_stacked(frame, app, info_area);
        } else {
            // SIDE-BY-SIDE LAYOUT: Vertical stack (current behavior)
            render_info_sidebar(frame, app, info_area);
        }
    }

    // Render waveform (with embedded analyzer if enabled and wide enough)
    if show_waveform_adjusted {
        if let Some(area) = waveform_area {
            render_waveform_fsf(frame, area, app, show_analyzer);
        }
    }
}

/// Render file section showing the full file path
fn render_file_section(frame: &mut Frame, state: &AudioModeState, area: Rect) {
    let file_block = Block::default()
        .title(" File ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    // Calculate available width for text wrapping (minus borders)
    let text_width = area.width.saturating_sub(3) as usize;

    let file_lines: Vec<Line> = if let Some(file) = state.selected_file() {
        let mut lines = vec![];
        let mut full_path = file.path.to_string_lossy().to_string();
        // Strip Windows extended path prefix (\\?\) if present
        if full_path.starts_with(r"\\?\") {
            full_path = full_path[4..].to_string();
        }
        for chunk in wrap_text(&full_path, text_width) {
            lines.push(Line::from(chunk.to_string()));
        }
        lines
    } else {
        vec![]
    };

    let file_paragraph = Paragraph::new(file_lines).block(file_block);
    frame.render_widget(file_paragraph, area);
}

/// Render description section showing file description from database or BWF metadata
fn render_description_section(frame: &mut Frame, state: &AudioModeState, area: Rect) {
    let desc_block = Block::default()
        .title(" Description ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    // Calculate available width for text wrapping (minus borders)
    let text_width = area.width.saturating_sub(3) as usize;

    let desc_lines: Vec<Line> = if let Some(file) = state.selected_file() {
        let mut lines = vec![];
        // Try file description first (from database)
        let description = if let Some(desc) = &file.description {
            Some(desc.clone())
        } else {
            // Fall back to metadata BWF description
            state
                .selected_metadata
                .as_ref()
                .and_then(|m| m.bwf_description.clone())
        };

        if let Some(desc) = description {
            if !desc.is_empty() {
                let clean_desc = sanitize_text(&desc);
                for chunk in wrap_text(&clean_desc, text_width) {
                    lines.push(Line::from(chunk));
                }
            }
        }
        lines
    } else {
        vec![]
    };

    let desc_paragraph = Paragraph::new(desc_lines).block(desc_block);
    frame.render_widget(desc_paragraph, area);
}

/// Render metadata section showing duration, sample rate, and channels
fn render_metadata_section(frame: &mut Frame, state: &AudioModeState, area: Rect) {
    let metadata_block = Block::default()
        .title(" Metadata ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let metadata_lines: Vec<Line> = if let Some(metadata) = &state.selected_metadata {
        let mut lines = vec![];

        if let Some(duration) = metadata.duration {
            let total_secs = duration.as_secs_f64();
            let mins = (total_secs / 60.0) as u64;
            let secs = total_secs % 60.0;
            lines.push(Line::from(vec![
                Span::styled("Duration: ", Style::default().fg(Color::Blue)),
                Span::raw(format!("{}:{:06.3}", mins, secs)),
            ]));
        }
        if let Some(sr) = metadata.sample_rate {
            lines.push(Line::from(vec![
                Span::styled("Sample Rate: ", Style::default().fg(Color::Blue)),
                Span::raw(format!("{} Hz", sr)),
            ]));
        }
        if let Some(channels) = metadata.channels {
            lines.push(Line::from(vec![
                Span::styled("Channels: ", Style::default().fg(Color::Blue)),
                Span::raw(format!("{}", channels)),
            ]));
        }

        lines
    } else {
        vec![]
    };

    let metadata_paragraph = Paragraph::new(metadata_lines).block(metadata_block);
    frame.render_widget(metadata_paragraph, area);
}

/// Helper function to create a shortcut line
fn shortcut<'a>(key: &'a str, label: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("{key:<width$}", key = key, width = 9),
            Style::default().fg(Color::Blue),
        ),
        Span::raw(label),
    ])
}

/// Render controls section showing keyboard shortcuts and settings
fn render_controls_section(frame: &mut Frame, _app: &App, state: &AudioModeState, area: Rect) {
    let controls_block = Block::default()
        .title(" Controls ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    // Extract status strings
    let autoplay_status = if state.autoplay { "ON" } else { "OFF" };
    let normalize_status = if state.normalize_waveform {
        // Calculate dB from linear gain: dB = 20 * log10(gain)
        let gain_db = 20.0 * state.normalize_gain.log10();
        // Show + for positive, - for negative (automatic)
        if gain_db >= 0.0 {
            format!("ON +{:.1} dB", gain_db)
        } else {
            format!("ON {:.1} dB", gain_db)
        }
    } else {
        "OFF".to_string()
    };
    let skip_silence_status = if state.skip_silence { "ON" } else { "OFF" };

    let controls_lines: Vec<Line> = if state.browse_mode {
        vec![
            shortcut("Enter", "Play"),
            shortcut("Space", "Play/Pause"),
            shortcut("s", "Stop"),
            shortcut("j/k", "Up/Down"),
            shortcut("d/u", "Half Page Down/Up"),
            shortcut("h/l", "Prev/Next Region"),
            shortcut("K/J", "Volume Up/Down"),
            shortcut("[/]", "Pitch Down/Up"),
            shortcut("/ ?", "Shuffle/Random"),
            shortcut("y", "Copy Path"),
            shortcut("r", "Open in Reaper"),
            shortcut("R", "Rebuild database"),
            shortcut("a", "Append to search"),
            shortcut("Esc/i", "Clear search"),
            Line::from(vec![
                Span::styled("A        ", Style::default().fg(Color::Blue)),
                Span::raw(format!("Autoplay ({})", autoplay_status)),
            ]),
            Line::from(vec![
                Span::styled("N        ", Style::default().fg(Color::Blue)),
                Span::raw(format!("Normalize ({})", &normalize_status)),
            ]),
            Line::from(vec![
                Span::styled("S        ", Style::default().fg(Color::Blue)),
                Span::raw(format!("Skip Silence ({})", skip_silence_status)),
            ]),
            shortcut("^c", "Exit"),
        ]
    } else {
        vec![
            shortcut("Enter", "Play + Browse mode"),
            shortcut("^p", "Play/Pause"),
            shortcut("^s", "Stop"),
            shortcut("^j/^k", "Up/Down"),
            shortcut("^d/^u", "Half Page Down/Up"),
            shortcut("^h/^l", "Prev/Next Region"),
            shortcut("K/J", "Volume Up/Down"),
            shortcut("[/]", "Pitch Down/Up"),
            shortcut("/ ?", "Shuffle/Random"),
            shortcut("^y", "Copy Path"),
            shortcut("^r", "Open in Reaper"),
            shortcut("R", "Rebuild database"),
            shortcut("[A-Z]", "Append to search"),
            shortcut("Esc", "Browse mode"),
            Line::from(vec![
                Span::styled("A        ", Style::default().fg(Color::Blue)),
                Span::raw(format!("Autoplay ({})", autoplay_status)),
            ]),
            Line::from(vec![
                Span::styled("N        ", Style::default().fg(Color::Blue)),
                Span::raw(format!("Normalize ({})", &normalize_status)),
            ]),
            Line::from(vec![
                Span::styled("S        ", Style::default().fg(Color::Blue)),
                Span::raw(format!("Skip Silence ({})", skip_silence_status)),
            ]),
            shortcut("^c", "Exit"),
        ]
    };

    let controls_paragraph = Paragraph::new(controls_lines).block(controls_block);
    frame.render_widget(controls_paragraph, area);
}

/// Render info panel in sidebar layout (vertical stack) - current side-by-side behavior
fn render_info_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(state) = &app.audio_state {
        let available_height = area.height;
        let controls_height: u16 = 20;
        let metadata_height: u16 = 5;
        let min_top_height: u16 = 8;

        // Prioritize file/description sections - hide controls first, then metadata
        let show_controls = available_height >= min_top_height + metadata_height + controls_height;
        let show_metadata = available_height >= min_top_height + metadata_height;

        // Build layout constraints based on what we're showing
        let constraints: Vec<Constraint> = if show_controls && show_metadata {
            vec![
                Constraint::Min(min_top_height),
                Constraint::Length(metadata_height),
                Constraint::Length(controls_height),
            ]
        } else if show_metadata {
            vec![
                Constraint::Min(min_top_height),
                Constraint::Length(metadata_height),
            ]
        } else {
            vec![Constraint::Min(min_top_height)]
        };

        let info_chunks = Layout::vertical(constraints).split(area);

        // File and Description sections (split the top area equally)
        let top_area = info_chunks[0];
        let top_chunks = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(top_area);

        render_file_section(frame, state, top_chunks[0]);
        render_description_section(frame, state, top_chunks[1]);

        // Metadata section (if space allows)
        if show_metadata {
            render_metadata_section(frame, state, info_chunks[1]);
        }

        // Controls section (if space allows)
        if show_controls {
            render_controls_section(frame, app, state, info_chunks[2]);
        }
    }
}

/// Render info panel in stacked layout (two columns side-by-side)
fn render_info_stacked(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(state) = &app.audio_state {
        // Calculate section heights (total 25 rows)
        let metadata_height: u16 = 5;
        let controls_height: u16 = 20;
        let show_controls = area.height >= metadata_height + controls_height;

        // Split horizontally into two columns (50/50) using full available height
        let columns = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        // LEFT COLUMN: File + Description (50/50 split)
        let left_chunks =
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(columns[0]);

        render_file_section(frame, state, left_chunks[0]);
        render_description_section(frame, state, left_chunks[1]);

        // RIGHT COLUMN: Metadata + Controls
        let right_chunks = Layout::vertical([
            Constraint::Length(metadata_height),
            Constraint::Length(controls_height),
        ])
        .split(columns[1]);

        render_metadata_section(frame, state, right_chunks[0]);

        if show_controls {
            render_controls_section(frame, app, state, right_chunks[1]);
        }
    }
}

/// Render waveform FSF-style: rectified, bottom-up, red/cyan, with title bar
/// If show_analyzer is true and width allows, embeds analyzer on the right 1/3
fn render_waveform_fsf(frame: &mut Frame, area: Rect, app: &App, show_analyzer: bool) {
    use std::time::Duration;

    const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    const MIN_WIDTH_FOR_ANALYZER: u16 = 90; // Minimum width to show embedded analyzer

    let state = match &app.audio_state {
        Some(s) => s,
        None => return,
    };

    // Build title bar content
    let (left_title, right_title) = if let Some(player) = &state.player {
        if let Some(file_path) = player.current_file() {
            // Determine icon based on playback state
            // Use U+FE0E variation selector to force text presentation (not emoji)
            let icon = if player.is_playing() {
                "▶\u{FE0E}"
            } else if player.is_paused() {
                "⏸\u{FE0E}"
            } else {
                "⏹\u{FE0E}"
            };

            // Get current position and total duration
            let current_pos = player.elapsed();
            let total_duration = if let Some(waveform) = &state.current_waveform {
                waveform
                    .duration
                    .or_else(|| state.current_metadata.as_ref().and_then(|m| m.duration))
            } else {
                state.current_metadata.as_ref().and_then(|m| m.duration)
            }
            .unwrap_or(Duration::from_secs(0));

            let current_time = format_time_precise(current_pos);
            let total_time = format_time_precise(total_duration);

            // Get filename
            let filename = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown");

            // Build right text first to know its width
            let right_text = format!(
                " Vol: {:+.0} dB | Pitch: {:+.0} st ",
                state.volume_db, state.pitch_semitones
            );
            let right_width = right_text.chars().count();

            // Calculate available width for left text
            let total_width = area.width as usize;
            let reserved_right = right_width + 2; // +2 for safety margin
            let available_left = total_width.saturating_sub(reserved_right);

            // Build left text prefix (without filename)
            let left_prefix = format!(" {} {} / {} - ", icon, current_time, total_time);
            let prefix_width = left_prefix.chars().count();

            // Calculate max filename width
            let max_filename_width = available_left.saturating_sub(prefix_width + 1); // +1 for trailing space

            // Truncate filename if needed
            let truncated_filename = if filename.chars().count() > max_filename_width {
                let mut chars: Vec<char> = filename
                    .chars()
                    .take(max_filename_width.saturating_sub(1))
                    .collect();
                chars.push('…');
                chars.into_iter().collect::<String>()
            } else {
                filename.to_string()
            };

            let left_text = format!("{}{} ", left_prefix, truncated_filename);

            (
                Line::from(left_text).left_aligned(),
                Line::from(right_text).right_aligned(),
            )
        } else {
            (
                Line::from(" Waveform ").left_aligned(),
                Line::from(format!(
                    " Vol: {:+.0} dB | Pitch: {:+.0} st ",
                    state.volume_db, state.pitch_semitones
                ))
                .right_aligned(),
            )
        }
    } else {
        (
            Line::from(" Waveform ").left_aligned(),
            Line::from(format!(
                " Vol: {:+.0} dB | Pitch: {:+.0} st ",
                state.volume_db, state.pitch_semitones
            ))
            .right_aligned(),
        )
    };

    // Get inner area (accounting for borders)
    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });

    // Determine if we should show the analyzer embedded
    let embed_analyzer = show_analyzer && inner.width >= MIN_WIDTH_FOR_ANALYZER;

    // Calculate widths for waveform and analyzer if embedded
    let (waveform_width, analyzer_width) = if embed_analyzer {
        let total_width = inner.width as usize;
        let analyzer_w = total_width / 3;
        let waveform_w = total_width - analyzer_w - 1; // -1 for separator
        (waveform_w, analyzer_w)
    } else {
        (inner.width as usize, 0)
    };

    // Build waveform content
    let waveform_lines: Vec<Line> = if let Some(waveform) = &state.current_waveform {
        if waveform.is_empty() {
            vec![]
        } else {
            let available_width = waveform_width;
            let total_peaks = waveform.peaks.len();
            let rows_count = inner.height.max(1) as usize;

            // Expected total peaks is 400, calculate completion ratio
            const EXPECTED_PEAKS: usize = 400;
            let completion_ratio = total_peaks as f32 / EXPECTED_PEAKS as f32;
            let filled_width = (available_width as f32 * completion_ratio) as usize;

            // Determine max peak for normalization
            let max_peak = if state.normalize_waveform {
                let mut max = f32::MIN;
                for &(min, max_val) in &waveform.peaks {
                    let val = max_val.abs().max(min.abs());
                    max = max.max(val);
                }
                max.max(0.001)
            } else {
                1.0
            };

            // Calculate current playback position
            let current_pos = if state.is_playing() || state.is_paused() {
                (state.get_progress() * available_width as f32) as usize
            } else {
                0
            };

            // Build waveform lines - rectified (absolute value), bottom to top
            let mut lines = Vec::new();

            // Render from top to bottom (reverse row order for display)
            for row in (0..rows_count).rev() {
                let mut chars = Vec::new();

                for i in 0..available_width {
                    // Only render up to filled width
                    if i >= filled_width {
                        chars.push(Span::styled(" ".to_string(), Style::default()));
                        continue;
                    }

                    // Map display position to peak index
                    let peak_idx = if filled_width > 0 {
                        (i * total_peaks) / filled_width
                    } else {
                        0
                    };

                    if peak_idx >= total_peaks {
                        chars.push(Span::styled(" ".to_string(), Style::default()));
                        continue;
                    }

                    let (min, max) = waveform.peaks[peak_idx];

                    // Use absolute value (rectified)
                    let value = max.abs().max(min.abs());

                    // Scale to [0, 1] range
                    let normalized_value = value / max_peak;

                    // Apply square root scaling for better visual distribution
                    let scaled_value = normalized_value.sqrt();

                    // Calculate height in eighths - filling from BOTTOM (row 0) to TOP
                    let total_eighths = (scaled_value * (rows_count * 8) as f32) as i32;
                    let row_eighths = row as i32 * 8;

                    let char_idx = if total_eighths >= row_eighths + 8 {
                        8 // Fully filled
                    } else if total_eighths > row_eighths {
                        (total_eighths - row_eighths).clamp(0, 8) as usize
                    } else {
                        0 // Empty
                    };

                    // Color logic
                    let color = if state.analyzer_gradient {
                        // Gradient mode
                        if i <= current_pos {
                            // Played: use position-based frequency gradient with amplitude-based brightness
                            let pos_ratio = if available_width > 0 {
                                i as f32 / available_width as f32
                            } else {
                                0.0
                            };
                            analyzer_color(pos_ratio, scaled_value, true)
                        } else {
                            // Unplayed: gray gradient based on amplitude
                            const MIN_GRAY: f32 = 100.0; // Dark gray
                            const MAX_GRAY: f32 = 209.0; // Light gray #d1d1d1
                            let gray_value = MIN_GRAY + scaled_value * (MAX_GRAY - MIN_GRAY);
                            let gray = gray_value.clamp(0.0, 255.0) as u8;
                            Color::Rgb(gray, gray, gray)
                        }
                    } else {
                        // Single color mode: played = Red, unplayed = Cyan
                        if i <= current_pos {
                            Color::Red
                        } else {
                            Color::Cyan
                        }
                    };

                    chars.push(Span::styled(
                        BLOCKS[char_idx].to_string(),
                        Style::default().fg(color),
                    ));
                }

                // If analyzer is embedded, add separator and analyzer visualization
                if embed_analyzer {
                    // Add vertical separator
                    chars.push(Span::styled("│", Style::default().fg(Color::DarkGray)));

                    // Add analyzer spans for this row
                    let is_bottom_row = row == 0;
                    let analyzer_spans =
                        build_analyzer_row(state, row, rows_count, analyzer_width, is_bottom_row);
                    chars.extend(analyzer_spans);
                }

                lines.push(Line::from(chars));
            }

            lines
        }
    } else {
        vec![]
    };

    let paragraph = Paragraph::new(waveform_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(left_title)
            .title(right_title),
    );

    frame.render_widget(paragraph, area);
}

/// Generate color based on frequency position and magnitude
/// If gradient is OFF: uses solid red (same as waveform progress color)
/// If gradient is ON: uses purple -> blue -> cyan -> fuchsia gradient based on frequency
fn analyzer_color(freq_position: f32, magnitude: f32, use_gradient: bool) -> Color {
    // Single color mode (default): solid red (same as waveform progress)
    if !use_gradient {
        return Color::Red;
    }
    // Vibrant frequency gradient colors
    // Low freq (0.0) = purple #8b5cf6
    // Mid-low (0.33) = blue #3b82f6
    // Mid-high (0.66) = cyan #06b6d4
    // High (1.0) = fuchsia #e879f9 (less pink, more magenta)

    let (base_r, base_g, base_b) = if freq_position < 0.33 {
        // Interpolate between purple and blue
        let t = freq_position / 0.33;
        let r = 139.0 + t * (59.0 - 139.0);
        let g = 92.0 + t * (130.0 - 92.0);
        let b = 246.0; // blue channel is 246 at both endpoints
        (r, g, b)
    } else if freq_position < 0.66 {
        // Interpolate between blue and cyan
        let t = (freq_position - 0.33) / 0.33;
        let r = 59.0 + t * (6.0 - 59.0);
        let g = 130.0 + t * (182.0 - 130.0);
        let b = 246.0 + t * (212.0 - 246.0);
        (r, g, b)
    } else {
        // Interpolate between cyan and fuchsia
        let t = (freq_position - 0.66) / 0.34;
        let r = 6.0 + t * (232.0 - 6.0);
        let g = 182.0 + t * (121.0 - 182.0);
        let b = 212.0 + t * (249.0 - 212.0);
        (r, g, b)
    };

    // Apply magnitude scaling (darker for quiet, brighter for loud)
    const MIN_BRIGHTNESS: f32 = 0.5; // 50% brightness minimum
    const MAX_BRIGHTNESS: f32 = 1.0; // 100% brightness maximum

    let brightness_scale = MIN_BRIGHTNESS + magnitude * (MAX_BRIGHTNESS - MIN_BRIGHTNESS);

    let r = (base_r * brightness_scale).clamp(0.0, 255.0) as u8;
    let g = (base_g * brightness_scale).clamp(0.0, 255.0) as u8;
    let b = (base_b * brightness_scale).clamp(0.0, 255.0) as u8;

    Color::Rgb(r, g, b)
}

struct FrequencyLabel {
    freq_hz: f32,
    label: &'static str,
    priority: u8, // 0 = always show, 1 = medium, 2 = wide only
}

const FREQUENCY_LABELS: [FrequencyLabel; 10] = [
    FrequencyLabel {
        freq_hz: 20.0,
        label: "20",
        priority: 0,
    },
    FrequencyLabel {
        freq_hz: 50.0,
        label: "50",
        priority: 2,
    },
    FrequencyLabel {
        freq_hz: 100.0,
        label: "100",
        priority: 1,
    },
    FrequencyLabel {
        freq_hz: 200.0,
        label: "200",
        priority: 2,
    },
    FrequencyLabel {
        freq_hz: 500.0,
        label: "500",
        priority: 1,
    },
    FrequencyLabel {
        freq_hz: 1000.0,
        label: "1k",
        priority: 0,
    },
    FrequencyLabel {
        freq_hz: 2000.0,
        label: "2k",
        priority: 1,
    },
    FrequencyLabel {
        freq_hz: 5000.0,
        label: "5k",
        priority: 1,
    },
    FrequencyLabel {
        freq_hz: 10000.0,
        label: "10k",
        priority: 0,
    },
    FrequencyLabel {
        freq_hz: 20000.0,
        label: "20k",
        priority: 0,
    },
];

fn build_analyzer_row(
    state: &AudioModeState,
    row: usize,
    rows_count: usize,
    width: usize,
    is_bottom_row: bool,
) -> Vec<Span<'static>> {
    const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    let mut spans = Vec::new();

    if let Some(frame_data) = &state.current_analyzer_frame {
        let bands = &frame_data.bands;

        if !bands.is_empty() {
            let min_freq = 20.0_f32;
            let max_freq = 20000.0_f32;
            let num_bands = bands.len();

            let freq_ratio_per_band = (max_freq / min_freq).powf(1.0 / num_bands as f32);

            // Determine visibility based on width (lowered thresholds)
            let max_priority = if width >= 80 {
                2 // Show all labels
            } else if width >= 50 {
                1 // Show priority 0 and 1
            } else {
                0 // Show only priority 0 (20, 1k, 10k, 20k)
            };

            // Build frequency label character map for bottom row
            let label_chars: Vec<Option<char>> = if is_bottom_row {
                let mut chars = vec![None; width];

                for label_def in &FREQUENCY_LABELS {
                    if label_def.priority > max_priority {
                        continue;
                    }

                    let freq = label_def.freq_hz;
                    if freq < min_freq || freq > max_freq {
                        continue;
                    }

                    // Calculate logarithmic position
                    let freq_ratio = (freq / min_freq).ln() / (max_freq / min_freq).ln();
                    let col = (freq_ratio * width as f32) as usize;

                    if col >= width {
                        continue;
                    }

                    // Center label at position
                    let label = label_def.label;
                    let label_len = label.len();
                    let start_col = col.saturating_sub(label_len / 2);

                    for (i, ch) in label.chars().enumerate() {
                        let target_col = start_col + i;
                        if target_col < width {
                            chars[target_col] = Some(ch);
                        }
                    }
                }

                chars
            } else {
                vec![None; width]
            };

            // Build frequency line position map (for all rows)
            // 0 = no line, 1 = dim line, 2 = bright line (100, 1k, 10k)
            let mut freq_line_cols = vec![0u8; width];
            for label_def in &FREQUENCY_LABELS {
                if label_def.priority > max_priority {
                    continue;
                }

                let freq = label_def.freq_hz;

                // Skip 20Hz line
                if freq == 20.0 {
                    continue;
                }

                if freq < min_freq || freq > max_freq {
                    continue;
                }

                // Calculate logarithmic position
                let freq_ratio = (freq / min_freq).ln() / (max_freq / min_freq).ln();
                let col = (freq_ratio * width as f32) as usize;

                if col < width {
                    // Bright lines for 100, 1k, 10k
                    let line_brightness = if freq == 100.0 || freq == 1000.0 || freq == 10000.0 {
                        2
                    } else {
                        1
                    };
                    freq_line_cols[col] = line_brightness;
                }
            }

            for col in 0..width {
                // Calculate the frequency this column represents (logarithmic)
                let col_freq_ratio = col as f32 / width as f32;
                let col_freq = min_freq * (max_freq / min_freq).powf(col_freq_ratio);

                // Find which band this frequency belongs to
                let band_idx_float = (col_freq / min_freq).ln() / freq_ratio_per_band.ln();
                let band_idx = band_idx_float.floor() as usize;

                // Get magnitude for this band
                let magnitude = if band_idx < bands.len() {
                    bands[band_idx]
                } else {
                    0.0
                };

                // Calculate height in eighths
                let total_eighths = (magnitude * (rows_count * 8) as f32) as i32;
                let row_eighths = row as i32 * 8;

                let char_idx = if total_eighths >= row_eighths + 8 {
                    8 // Fully filled
                } else if total_eighths > row_eighths {
                    (total_eighths - row_eighths).clamp(0, 8) as usize
                } else {
                    0 // Empty
                };

                // Determine display character: priority order:
                // 1. Analyzer bars if magnitude > 0
                // 2. Labels on bottom row if present
                // 3. Frequency lines if at frequency position and no magnitude
                // 4. Empty space
                let (display_char, color) = if char_idx == 0 && label_chars[col].is_some() {
                    // No magnitude here, show label
                    (label_chars[col].unwrap().to_string(), Color::DarkGray)
                } else if char_idx == 0 && freq_line_cols[col] > 0 {
                    // No magnitude, no label, but at frequency position - show line
                    let line_color = if freq_line_cols[col] == 2 {
                        Color::Rgb(70, 70, 70) // Brighter line for 100, 1k, 10k
                    } else {
                        Color::Rgb(40, 40, 40) // Dim line for others
                    };
                    ("│".to_string(), line_color)
                } else if char_idx > 0 {
                    // Show analyzer block with color
                    (
                        BLOCKS[char_idx].to_string(),
                        analyzer_color(col_freq_ratio, magnitude, state.analyzer_gradient),
                    )
                } else {
                    // Empty space
                    (
                        BLOCKS[0].to_string(),
                        analyzer_color(col_freq_ratio, 0.0, state.analyzer_gradient),
                    )
                };

                let style = Style::default().fg(color);
                spans.push(Span::styled(display_char, style));
            }
        } else {
            // No band data - show labels/lines
            let min_freq = 20.0_f32;
            let max_freq = 20000.0_f32;

            // Determine visibility based on width (lowered thresholds)
            let max_priority = if width >= 80 {
                2 // Show all labels
            } else if width >= 50 {
                1 // Show priority 0 and 1
            } else {
                0 // Show only priority 0 (20, 1k, 10k, 20k)
            };

            let mut label_chars = vec![None; width];
            let mut freq_line_cols = vec![0u8; width];

            for label_def in &FREQUENCY_LABELS {
                if label_def.priority > max_priority {
                    continue;
                }

                let freq = label_def.freq_hz;

                // Skip 20Hz line
                if freq == 20.0 {
                    if is_bottom_row {
                        // Still add label for 20Hz on bottom row
                        if freq >= min_freq && freq <= max_freq {
                            let freq_ratio = (freq / min_freq).ln() / (max_freq / min_freq).ln();
                            let col = (freq_ratio * width as f32) as usize;
                            if col < width {
                                let label = label_def.label;
                                let label_len = label.len();
                                let start_col = col.saturating_sub(label_len / 2);
                                for (i, ch) in label.chars().enumerate() {
                                    let target_col = start_col + i;
                                    if target_col < width {
                                        label_chars[target_col] = Some(ch);
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }

                if freq < min_freq || freq > max_freq {
                    continue;
                }

                // Calculate logarithmic position
                let freq_ratio = (freq / min_freq).ln() / (max_freq / min_freq).ln();
                let col = (freq_ratio * width as f32) as usize;

                if col >= width {
                    continue;
                }

                // Mark this column for frequency line
                let line_brightness = if freq == 100.0 || freq == 1000.0 || freq == 10000.0 {
                    2
                } else {
                    1
                };
                freq_line_cols[col] = line_brightness;

                // For bottom row, add labels
                if is_bottom_row {
                    let label = label_def.label;
                    let label_len = label.len();
                    let start_col = col.saturating_sub(label_len / 2);

                    for (i, ch) in label.chars().enumerate() {
                        let target_col = start_col + i;
                        if target_col < width {
                            label_chars[target_col] = Some(ch);
                        }
                    }
                }
            }

            // Render
            for col in 0..width {
                let (display_char, color) = if is_bottom_row && label_chars[col].is_some() {
                    (label_chars[col].unwrap().to_string(), Color::DarkGray)
                } else if freq_line_cols[col] > 0 {
                    let line_color = if freq_line_cols[col] == 2 {
                        Color::Rgb(70, 70, 70) // Brighter line for 100, 1k, 10k
                    } else {
                        Color::Rgb(40, 40, 40) // Dim line for others
                    };
                    ("│".to_string(), line_color)
                } else {
                    (" ".to_string(), Color::Reset)
                };

                spans.push(Span::styled(display_char, Style::default().fg(color)));
            }
        }
    } else {
        // No analyzer frame - show labels/lines
        let min_freq = 20.0;
        let max_freq = 20000.0;

        // Determine visibility based on width (lowered thresholds)
        let max_priority = if width >= 80 {
            2 // Show all labels
        } else if width >= 50 {
            1 // Show priority 0 and 1
        } else {
            0 // Show only priority 0 (20, 1k, 10k, 20k)
        };

        let mut label_chars = vec![None; width];
        let mut freq_line_cols = vec![0u8; width];

        for label_def in &FREQUENCY_LABELS {
            if label_def.priority > max_priority {
                continue;
            }

            let freq = label_def.freq_hz;

            // Skip 20Hz line
            if freq == 20.0 {
                if is_bottom_row {
                    // Still add label for 20Hz on bottom row
                    if freq >= min_freq && freq <= max_freq {
                        let freq_ratio = (freq / min_freq).ln() / (max_freq / min_freq).ln();
                        let col = (freq_ratio * width as f32) as usize;
                        if col < width {
                            let label = label_def.label;
                            let label_len = label.len();
                            let start_col = col.saturating_sub(label_len / 2);
                            for (i, ch) in label.chars().enumerate() {
                                let target_col = start_col + i;
                                if target_col < width {
                                    label_chars[target_col] = Some(ch);
                                }
                            }
                        }
                    }
                }
                continue;
            }

            if freq < min_freq || freq > max_freq {
                continue;
            }

            // Calculate logarithmic position
            let freq_ratio = (freq / min_freq).ln() / (max_freq / min_freq).ln();
            let col = (freq_ratio * width as f32) as usize;

            if col >= width {
                continue;
            }

            // Mark this column for frequency line
            let line_brightness = if freq == 100.0 || freq == 1000.0 || freq == 10000.0 {
                2
            } else {
                1
            };
            freq_line_cols[col] = line_brightness;

            // For bottom row, add labels
            if is_bottom_row {
                let label = label_def.label;
                let label_len = label.len();
                let start_col = col.saturating_sub(label_len / 2);

                for (i, ch) in label.chars().enumerate() {
                    let target_col = start_col + i;
                    if target_col < width {
                        label_chars[target_col] = Some(ch);
                    }
                }
            }
        }

        // Render
        for col in 0..width {
            let (display_char, color) = if is_bottom_row && label_chars[col].is_some() {
                (label_chars[col].unwrap().to_string(), Color::DarkGray)
            } else if freq_line_cols[col] > 0 {
                let line_color = if freq_line_cols[col] == 2 {
                    Color::Rgb(70, 70, 70) // Brighter line for 100, 1k, 10k
                } else {
                    Color::Rgb(40, 40, 40) // Dim line for others
                };
                ("│".to_string(), line_color)
            } else {
                (" ".to_string(), Color::Reset)
            };

            spans.push(Span::styled(display_char, Style::default().fg(color)));
        }
    }

    spans
}

fn format_time_precise(duration: std::time::Duration) -> String {
    let total_secs = duration.as_secs();
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    let centis = (duration.subsec_millis() / 10) as u64;
    format!("{:02}:{:02}.{:02}", mins, secs, centis)
}
