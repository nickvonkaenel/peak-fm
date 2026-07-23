mod audio;
mod dialogs;
mod format;
mod menus;
mod modes;

use crossterm::cursor::SetCursorStyle;
use crossterm::execute;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};
use std::collections::HashSet;
use std::io::stdout;
use std::path::PathBuf;

use devicons::FileIcon;

use crate::app::App;
use crate::core::{
    available_themes, current_theme, is_audio_file, DisplayInfo, Pane, Preview, SortOption,
};
use crate::input::Mode;

use audio::render_audio_mode;
use dialogs::{
    render_confirm, render_help, render_quit_confirm, render_settings, render_sync_confirm,
};
use format::{format_mode, format_size, format_time};
use menus::{
    render_git_commit, render_git_menu, render_git_status, render_info_select_menu,
    render_leader_menu, render_preview_options_menu, render_sort_menu,
};
use modes::{render_find_mode, render_grep_mode};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Clear the entire screen with explicit background
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Reset)),
        area,
    );

    // Handle Find mode with different layout (also when Settings/Help popup is open during Find)
    if app.find_state.is_some() {
        render_find_mode(frame, area, app);

        // Popup overlays in Find mode
        if matches!(app.mode, Mode::Settings) {
            render_settings(frame, area, app);
        }
        if matches!(app.mode, Mode::Help) {
            render_help(frame, area, app);
        }
        return;
    }

    // Handle Grep mode with different layout
    if app.grep_state.is_some() {
        render_grep_mode(frame, area, app);

        // Popup overlays in Grep mode
        if matches!(app.mode, Mode::Settings) {
            render_settings(frame, area, app);
        }
        if matches!(app.mode, Mode::Help) {
            render_help(frame, area, app);
        }
        return;
    }

    // Handle Audio mode with full-screen layout
    if app.audio_state.is_some() {
        render_audio_mode(frame, area, app);

        // Popup overlays in Audio mode
        if matches!(app.mode, Mode::Help) {
            render_help(frame, area, app);
        }
        return;
    }

    // Main layout: content area + status bar
    let main_chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);

    let content_area = main_chunks[0];
    let status_area = main_chunks[1];

    // Responsive layout based on terminal dimensions
    // - Wide (>=100): parent (20%) | current (40%) | preview (40%)
    // - Medium (60-99): current (50%) | preview (50%)
    // - Narrow but tall: current on top, preview at bottom (horizontal split)
    // - Narrow (<60) and short: current only (100%)
    let width = content_area.width;
    let height = content_area.height;
    let aspect_ratio = width as f32 / height.max(1) as f32;

    // Use horizontal split (preview at bottom) when narrow but tall
    let use_horizontal_preview = width < 80 && height >= 20 && aspect_ratio < 1.8;

    let (show_parent, show_preview) = if use_horizontal_preview {
        (false, true)
    } else if width >= 100 {
        (true, true)
    } else if width >= 60 {
        (false, true)
    } else {
        (false, false)
    };

    // Different layout based on orientation
    let (columns, preview_area) = if use_horizontal_preview {
        // Vertical split: file list on top, preview at bottom
        let vert_chunks =
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(content_area);
        (vec![vert_chunks[0]], Some(vert_chunks[1]))
    } else {
        let constraints: Vec<Constraint> = match (show_parent, show_preview) {
            (true, true) => vec![
                Constraint::Percentage(20),
                Constraint::Percentage(40),
                Constraint::Percentage(40),
            ],
            (false, true) => vec![Constraint::Percentage(50), Constraint::Percentage(50)],
            (_, false) => vec![Constraint::Percentage(100)],
        };
        let cols = Layout::horizontal(constraints).split(content_area);
        let preview = if show_preview {
            Some(cols[cols.len() - 1])
        } else {
            None
        };
        (cols.to_vec(), preview)
    };

    // Column indices based on what's visible
    let (parent_col, current_col, preview_col): (Option<usize>, usize, Option<Rect>) =
        if use_horizontal_preview {
            (None, 0, preview_area)
        } else {
            match (show_parent, show_preview) {
                (true, true) => (Some(0), 1, Some(columns[2])),
                (false, true) => (None, 0, Some(columns[1])),
                (_, false) => (None, 0, None),
            }
        };

    // Update pane dimensions for scrolling calculation and image preview
    let pane_height = columns[current_col].height.saturating_sub(2) as usize;
    let preview_rect = preview_col;
    let preview_width = preview_rect
        .map(|r| r.width.saturating_sub(2) as usize)
        .unwrap_or(0);
    let preview_pane_height = preview_rect
        .map(|r| r.height.saturating_sub(2) as usize)
        .unwrap_or(pane_height);

    // Check if dimensions changed (e.g., first render or window resize)
    let dimensions_changed =
        app.preview_height != preview_pane_height || app.preview_width != preview_width;

    app.current.height = pane_height;
    app.preview_height = preview_pane_height;
    app.preview_width = preview_width;
    if let Some(ref mut parent) = app.parent {
        parent.height = pane_height;
    }
    if let Preview::Directory(ref mut pane) = app.preview {
        pane.height = preview_pane_height;
    }

    // Refresh preview if dimensions changed (for proper image scaling)
    if dimensions_changed {
        app.refresh_preview();
    }

    // Render panes
    let search_query = if !app.search_query.is_empty() {
        Some(app.search_query.as_str())
    } else {
        None
    };

    // Get visual selection range if in visual mode
    let visual_selection = match app.mode {
        Mode::Visual { anchor } | Mode::VisualInsert { anchor, .. } => {
            let cursor = app.current.cursor;
            if anchor <= cursor {
                Some((anchor, cursor))
            } else {
                Some((cursor, anchor))
            }
        }
        _ => None,
    };

    // Get current directory name to highlight in parent pane
    let current_dir_name = app.cwd.file_name().and_then(|n| n.to_str());

    // Render parent pane only if visible
    if let Some(parent_idx) = parent_col {
        let parent_sort = if let Some(ref parent) = app.parent {
            app.get_sort_for_dir(&parent.buffer.path)
        } else {
            app.sort_option
        };
        render_pane(
            frame,
            columns[parent_idx],
            app.parent.as_ref(),
            false,
            None,
            None,
            current_dir_name,
            None,
            app.display_info,
            parent_sort,
            app.show_icons,
            app.colored_icons,
            app.theme_icons,
        );
    }

    // Render current pane
    let current_sort = app.get_sort_for_dir(&app.cwd);
    render_pane(
        frame,
        columns[current_col],
        Some(&app.current),
        true,
        search_query,
        visual_selection,
        None,
        Some(&app.marked_files),
        app.display_info,
        current_sort,
        app.show_icons,
        app.colored_icons,
        app.theme_icons,
    );

    // Render preview pane only if visible
    let selected_path = app.current.selected_path();
    if let Some(preview_area) = preview_rect {
        // Get selected filename for preview title
        let preview_title = app
            .current
            .buffer
            .lines
            .get(app.current.cursor)
            .map(|l| l.text.as_str())
            .unwrap_or("Preview");
        render_preview(
            frame,
            preview_area,
            &app.preview,
            preview_title,
            app.wrap_preview,
            app.line_numbers,
            None,
            app.show_icons,
            app.colored_icons,
            app.theme_icons,
        );

        // Render audio player overlay for audio files
        render_audio_player(frame, preview_area, app, selected_path.as_deref());
    }

    // Set cursor position and style based on mode
    if matches!(app.mode, Mode::Insert) {
        // Use bar cursor in insert mode
        let _ = execute!(stdout(), SetCursorStyle::BlinkingBar);

        let pane = &app.current;
        let line_idx = pane.cursor;
        let edit_cursor = pane.buffer.edit_cursor;

        // Calculate prefix length (icon + mark indicator)
        let mut prefix_len: u16 = 0;
        if app.show_icons {
            prefix_len += 2; // icon + space
        }
        // Check if current file is marked
        if let Some(line) = pane.buffer.lines.get(line_idx) {
            let file_path = pane.buffer.path.join(&line.text);
            if app.marked_files.contains(&file_path) {
                prefix_len += 2; // mark icon + space
            }
        }

        // Calculate screen position
        // +1 for border, line position relative to scroll
        if line_idx >= pane.scroll_offset && line_idx < pane.scroll_offset + pane.height {
            let screen_line = (line_idx - pane.scroll_offset) as u16;
            let x = columns[current_col].x + 1 + prefix_len + edit_cursor as u16;
            let y = columns[current_col].y + 1 + screen_line;
            frame.set_cursor_position((x, y));
        }
    } else if matches!(app.mode, Mode::Search(_)) {
        // Use bar cursor in search mode, positioned in status bar
        let _ = execute!(stdout(), SetCursorStyle::BlinkingBar);
        // Status format: " SEARCH  /query" - cursor goes after query
        // " SEARCH " = 8 chars, then " " = 1 char, then status
        let prefix_len = format!(" {} ", app.mode.name()).len() + 1; // +1 for space after mode
                                                                     // Position cursor after prefix (/ or ?) and query, not after [No match] text
        let search_prefix_len = 1; // "/" or "?"
        let x =
            status_area.x + prefix_len as u16 + search_prefix_len + app.search_query.len() as u16;
        let y = status_area.y;
        frame.set_cursor_position((x, y));
    } else if matches!(app.mode, Mode::Command) {
        // Bar cursor in the shell-command line, after the "!" prefix and input.
        let _ = execute!(stdout(), SetCursorStyle::BlinkingBar);
        let prefix_len = format!(" {} ", app.mode.name()).len() + 1;
        let cmd_prefix_len = 1; // "!"
        let x = status_area.x + prefix_len as u16 + cmd_prefix_len + app.command_input.len() as u16;
        let y = status_area.y;
        frame.set_cursor_position((x, y));
    } else {
        // Use default cursor in normal mode
        let _ = execute!(stdout(), SetCursorStyle::DefaultUserShape);
    }

    // Status bar
    render_status(frame, status_area, app);

    // Help popup
    if matches!(app.mode, Mode::Help) {
        render_help(frame, area, app);
    }

    // Theme selector popup
    if let Mode::ThemeSelect { selected } = &app.mode {
        render_theme_select(frame, area, *selected);
    }

    // Settings popup
    if matches!(app.mode, Mode::Settings) {
        render_settings(frame, area, app);
    }

    // Sort menu popup
    if let Mode::Sort {
        selected,
        is_global,
    } = app.mode
    {
        render_sort_menu(frame, area, app, selected, is_global);
    }

    // Info select menu popup
    if let Mode::InfoSelect { selected } = app.mode {
        render_info_select_menu(frame, area, app, selected);
    }

    // Leader menu
    if matches!(app.mode, Mode::Leader) {
        render_leader_menu(frame, area);
    }

    // Preview options menu
    if matches!(app.mode, Mode::PreviewOptions) {
        render_preview_options_menu(frame, area, app);
    }

    // Git menu
    if matches!(app.mode, Mode::Git) {
        render_git_menu(frame, area);
    }

    // Git status popup
    if let Mode::GitStatus { ref lines, scroll } = app.mode {
        render_git_status(frame, area, lines, scroll);
    }

    // Git commit prompt
    if let Mode::GitCommit {
        ref message,
        all,
        auto_push,
        ref status,
    } = app.mode
    {
        render_git_commit(frame, area, message, all, auto_push, status);
    }

    // Sync confirmation popup
    if let Mode::SyncConfirm { scroll } = app.mode {
        render_sync_confirm(frame, area, app, scroll);
    }

    // Quit confirmation popup
    if let Mode::QuitConfirm { scroll } = app.mode {
        render_quit_confirm(frame, area, app, scroll);
    }

    // Confirm popup (e.g., empty trash)
    if let Mode::Confirm(ref action) = app.mode {
        render_confirm(frame, area, action);
    }
}

fn render_pane(
    frame: &mut Frame,
    area: Rect,
    pane: Option<&Pane>,
    active: bool,
    search_query: Option<&str>,
    visual_selection: Option<(usize, usize)>, // (start, end) for visual mode highlighting
    highlight_name: Option<&str>, // Name to highlight (for parent pane showing current dir)
    marked_files: Option<&HashSet<PathBuf>>, // Files marked for batch operations
    display_info: DisplayInfo,    // Display info option (for displaying size/date/mode)
    sort_option: SortOption,      // Current sort option (determines auto-shown info)
    show_icons: bool,             // Whether to show file type icons
    colored_icons: bool,          // Whether to use colored icons
    theme_icons: bool,            // Whether to map icon colors to theme palette
) {
    let border_style = if active {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::Rgb(96, 98, 104))
    };

    let Some(pane) = pane else {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style);
        frame.render_widget(block, area);
        return;
    };

    let title = if pane.buffer.is_volumes {
        "Volumes".to_string()
    } else {
        pane.buffer
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string())
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items: Vec<ListItem> = pane
        .buffer
        .lines
        .iter()
        .enumerate()
        .skip(pane.scroll_offset)
        .take(inner.height as usize)
        .map(|(idx, line)| {
            let is_cursor = idx == pane.cursor && active;
            let is_in_visual = active
                && visual_selection
                    .map(|(start, end)| idx >= start && idx <= end)
                    .unwrap_or(false);
            let is_modified = line.id.is_none()
                || line.copy_from.is_some()
                || (line.id.is_some()
                    && pane
                        .buffer
                        .snapshot
                        .iter()
                        .any(|s| s.id == line.id && s.text != line.text));

            // Check if this file is marked
            let file_path = pane.buffer.path.join(&line.text);
            let is_marked = marked_files
                .map(|m| m.contains(&file_path))
                .unwrap_or(false);

            let display = if line.is_dir && !line.text.ends_with('/') && !show_icons {
                format!("{}/", line.text) // Only add / when icons are disabled
            } else {
                line.text.clone()
            };

            // Get file icon if enabled
            let file_icon_span = if show_icons {
                let icon = FileIcon::from(line.text.as_str());
                let (icon_char, icon_color) = if icon.icon == '*' {
                    // Fallback for unrecognized files/directories
                    let fallback_icon = if line.is_dir {
                        '\u{f115}' // Nerd Font: nf-fa-folder_open_o
                    } else {
                        '\u{f15b}' // Nerd Font: nf-fa-file
                    };
                    let color = if line.is_dir {
                        Color::Blue // Blue for folders (matches directory text)
                    } else if colored_icons {
                        Color::Gray // Gray for unknown files
                    } else {
                        Color::Reset // Default text color
                    };
                    (fallback_icon, color)
                } else {
                    // Use devicons icon
                    let color = if colored_icons {
                        // Parse hex color like "#RRGGBB" to RGB
                        if icon.color.len() == 7 && icon.color.starts_with('#') {
                            let r = u8::from_str_radix(&icon.color[1..3], 16).unwrap_or(128);
                            let g = u8::from_str_radix(&icon.color[3..5], 16).unwrap_or(128);
                            let b = u8::from_str_radix(&icon.color[5..7], 16).unwrap_or(128);
                            if theme_icons {
                                // Map to nearest theme color
                                crate::core::map_to_theme_color(r, g, b)
                            } else {
                                Color::Rgb(r, g, b)
                            }
                        } else {
                            Color::Gray
                        }
                    } else {
                        // Monochrome: match directory/file text colors
                        if line.is_dir {
                            Color::Blue
                        } else {
                            Color::Reset
                        }
                    };
                    (icon.icon, color)
                };
                Some((icon_char, icon_color))
            } else {
                None
            };

            // Determine what info to show based on sort option first, then display_info
            let info_to_show = match sort_option {
                SortOption::DateModified | SortOption::DateModifiedAsc => DisplayInfo::DateModified,
                SortOption::Size | SortOption::SizeAsc => DisplayInfo::Size,
                SortOption::Extension | SortOption::ExtensionDesc => DisplayInfo::Extension,
                _ => display_info, // Fall back to user's display_info setting
            };

            // Format info suffix based on determined info to show
            let info_suffix: Option<String> = match info_to_show {
                DisplayInfo::None => None,
                DisplayInfo::DateModified => line.modified.map(format_time),
                DisplayInfo::Size => line.size.map(format_size),
                DisplayInfo::Mode => line.mode.map(format_mode),
                DisplayInfo::Extension => {
                    // Extract extension from filename
                    if line.is_dir {
                        None
                    } else {
                        line.text.rsplit_once('.').and_then(|(_, ext)| {
                            if ext.is_empty() {
                                None
                            } else {
                                Some(format!(".{}", ext))
                            }
                        })
                    }
                }
            };

            let mut base_style = if line.is_dir {
                Style::default().fg(Color::Blue)
            } else {
                Style::default()
            };

            if is_modified {
                base_style = base_style.fg(Color::Yellow);
            }

            // Visual selection highlighting
            if is_in_visual {
                base_style = base_style.bg(Color::Rgb(60, 60, 100));
            }

            // Cursor line gets special highlighting
            let sel_bg = Color::Rgb(55, 55, 55);
            if is_cursor {
                base_style = base_style.bg(sel_bg).add_modifier(Modifier::BOLD);
            }

            // Highlight matching name in parent pane (current directory)
            if let Some(name) = highlight_name {
                if line.text == name {
                    base_style = base_style.bg(Color::Rgb(35, 35, 35));
                }
            }

            // Build prefix spans: file icon (optional) + mark icon (optional)
            let mut prefix_spans: Vec<Span> = Vec::new();
            let mut prefix_len = 0;

            // Add file icon if enabled
            if let Some((icon_char, icon_color)) = file_icon_span {
                prefix_spans.push(Span::styled(
                    format!("{} ", icon_char),
                    Style::default().fg(icon_color),
                ));
                prefix_len += 2; // icon + space
            }

            // Add mark icon if marked
            if is_marked {
                prefix_spans.push(Span::styled("◆ ", Style::default().fg(Color::Magenta)));
                prefix_len += 2;
            }

            // Handle search highlighting
            if let Some(query) = search_query {
                let query_lower = query.to_lowercase();
                let display_lower = display.to_lowercase();

                if let Some(match_start) = display_lower.find(&query_lower) {
                    let match_end = match_start + query.len();
                    // Use reverse style for highlight to ensure visibility
                    let highlight_style = if is_cursor {
                        Style::default().bg(Color::Yellow).fg(Color::Black)
                    } else {
                        Style::default().bg(sel_bg)
                    };

                    let mut spans = prefix_spans.clone();
                    spans.push(Span::styled(display[..match_start].to_string(), base_style));
                    spans.push(Span::styled(
                        display[match_start..match_end].to_string(),
                        highlight_style,
                    ));
                    spans.push(Span::styled(display[match_end..].to_string(), base_style));
                    return ListItem::new(Line::from(spans));
                }
            }

            // No search match - render with optional info suffix
            let info_style = Style::default().fg(Color::DarkGray);
            let pane_width = inner.width as usize;

            if let Some(ref suffix) = info_suffix {
                let mut display_text = display.clone();
                let suffix_len = suffix.chars().count();
                let max_name_len = pane_width.saturating_sub(suffix_len + prefix_len + 2);

                // Truncate filename if needed
                let name_chars: Vec<char> = display_text.chars().collect();
                if name_chars.len() > max_name_len {
                    display_text = name_chars
                        .iter()
                        .take(max_name_len.saturating_sub(1))
                        .collect();
                    display_text.push('…');
                }

                let content_len = prefix_len + display_text.chars().count();
                let padding = pane_width.saturating_sub(content_len + suffix_len + 1);

                let mut spans = prefix_spans;
                spans.push(Span::styled(display_text, base_style));
                if padding > 0 {
                    spans.push(Span::raw(" ".repeat(padding)));
                } else {
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(suffix.clone(), info_style));
                ListItem::new(Line::from(spans))
            } else if !prefix_spans.is_empty() {
                let mut spans = prefix_spans;
                spans.push(Span::styled(display, base_style));
                ListItem::new(Line::from(spans))
            } else {
                ListItem::new(display).style(base_style)
            }
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Highlight pattern matches within a Line's spans using regex
fn highlight_pattern_in_line(line: Line<'static>, pattern: &str) -> Line<'static> {
    if pattern.is_empty() {
        return line;
    }

    // Smart case: case insensitive unless pattern contains uppercase
    let has_uppercase = pattern.chars().any(|c| c.is_uppercase());
    let re = match regex::RegexBuilder::new(pattern)
        .case_insensitive(!has_uppercase)
        .build()
    {
        Ok(r) => r,
        Err(_) => return line, // Invalid regex, return unchanged
    };

    let highlight_style = Style::default().fg(Color::Rgb(255, 165, 87));

    let new_spans: Vec<Span> = line
        .spans
        .into_iter()
        .flat_map(|span| {
            let text = span.content.to_string();
            let base_style = span.style;

            // Find all regex matches in this span
            let matches: Vec<(usize, usize)> =
                re.find_iter(&text).map(|m| (m.start(), m.end())).collect();

            if matches.is_empty() {
                return vec![Span::styled(text, base_style)];
            }

            // Split span at match boundaries
            let mut result = Vec::new();
            let mut last_end = 0;

            for (start, end) in matches {
                if start > last_end {
                    result.push(Span::styled(text[last_end..start].to_string(), base_style));
                }
                // Apply highlight style but keep background if set
                let match_style = if base_style.bg.is_some() {
                    highlight_style.bg(base_style.bg.unwrap())
                } else {
                    highlight_style
                };
                result.push(Span::styled(text[start..end].to_string(), match_style));
                last_end = end;
            }

            if last_end < text.len() {
                result.push(Span::styled(text[last_end..].to_string(), base_style));
            }

            result
        })
        .collect();

    Line::from(new_spans)
}

#[allow(static_mut_refs)]
pub(super) fn render_preview(
    frame: &mut Frame,
    area: Rect,
    preview: &Preview,
    title: &str,
    wrap: bool,
    line_numbers: bool,
    highlight_pattern: Option<&str>,
    show_icons: bool,
    colored_icons: bool,
    theme_icons: bool,
) {
    let border_style = Style::default().fg(Color::Rgb(96, 98, 104));

    match preview {
        Preview::Directory(pane) => {
            // Clear any lingering images
            unsafe {
                PENDING_IMAGE = None;
            }

            let block = Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style);

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let items: Vec<ListItem> = pane
                .buffer
                .lines
                .iter()
                .take(inner.height as usize)
                .map(|line| {
                    let display = if line.is_dir && !line.text.ends_with('/') && !show_icons {
                        format!("{}/", line.text) // Only add / when icons are disabled
                    } else {
                        line.text.clone()
                    };

                    let text_style = if line.is_dir {
                        Style::default().fg(Color::Blue)
                    } else {
                        Style::default()
                    };

                    if show_icons {
                        let icon = FileIcon::from(line.text.as_str());
                        let (icon_char, icon_color) = if icon.icon == '*' {
                            // Fallback for unrecognized files/directories
                            let fallback_icon = if line.is_dir {
                                '\u{f115}' // Nerd Font: nf-fa-folder_open_o
                            } else {
                                '\u{f15b}' // Nerd Font: nf-fa-file
                            };
                            let color = if line.is_dir {
                                Color::Blue
                            } else if colored_icons {
                                Color::Gray
                            } else {
                                Color::Reset
                            };
                            (fallback_icon, color)
                        } else {
                            let color = if colored_icons {
                                if icon.color.len() == 7 && icon.color.starts_with('#') {
                                    let r =
                                        u8::from_str_radix(&icon.color[1..3], 16).unwrap_or(128);
                                    let g =
                                        u8::from_str_radix(&icon.color[3..5], 16).unwrap_or(128);
                                    let b =
                                        u8::from_str_radix(&icon.color[5..7], 16).unwrap_or(128);
                                    if theme_icons {
                                        crate::core::map_to_theme_color(r, g, b)
                                    } else {
                                        Color::Rgb(r, g, b)
                                    }
                                } else {
                                    Color::Gray
                                }
                            } else if line.is_dir {
                                Color::Blue
                            } else {
                                Color::Reset
                            };
                            (icon.icon, color)
                        };
                        let spans = vec![
                            Span::styled(
                                format!("{} ", icon_char),
                                Style::default().fg(icon_color),
                            ),
                            Span::styled(display, text_style),
                        ];
                        ListItem::new(Line::from(spans))
                    } else {
                        ListItem::new(display).style(text_style)
                    }
                })
                .collect();

            let list = List::new(items);
            frame.render_widget(list, inner);
        }
        Preview::File(file_preview) => {
            // Clear any lingering images
            unsafe {
                PENDING_IMAGE = None;
            }

            let block = Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style);

            let inner = block.inner(area);
            frame.render_widget(block, area);

            if let Some(ref error) = file_preview.error {
                let text = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
                frame.render_widget(text, inner);
            } else if file_preview.is_binary {
                // Check if it's an audio file - if so, skip the binary message
                // (audio player will be rendered separately)
                let is_audio = std::path::Path::new(title)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| {
                        matches!(
                            e.to_lowercase().as_str(),
                            "mp3"
                                | "wav"
                                | "flac"
                                | "ogg"
                                | "m4a"
                                | "aac"
                                | "opus"
                                | "aiff"
                                | "wma"
                        )
                    })
                    .unwrap_or(false);

                if !is_audio {
                    let text =
                        Paragraph::new("[Binary file]").style(Style::default().fg(Color::DarkGray));
                    frame.render_widget(text, inner);
                }
            } else if file_preview.lines.is_empty() {
                let text =
                    Paragraph::new("[Empty file]").style(Style::default().fg(Color::DarkGray));
                frame.render_widget(text, inner);
            } else {
                let width = inner.width as usize;
                let first_line_num = file_preview.first_line_num;
                let last_line_num = first_line_num + file_preview.lines.len();
                let num_width = if line_numbers {
                    last_line_num.to_string().len().max(3)
                } else {
                    0
                };
                let gutter_width = if line_numbers { num_width + 1 } else { 0 }; // +1 for space

                let highlight_line = file_preview.highlight_line;

                if wrap {
                    // Wrap mode with optional line numbers
                    let mut output_lines: Vec<Line> = Vec::new();
                    let content_width = width.saturating_sub(gutter_width);

                    for (i, hl) in file_preview
                        .lines
                        .iter()
                        .enumerate()
                        .skip(file_preview.scroll_offset)
                    {
                        if output_lines.len() >= inner.height as usize {
                            break;
                        }

                        let line_num = first_line_num + i;
                        let is_highlighted = highlight_line == Some(line_num);
                        let wrapped = wrap_highlighted_line(
                            hl,
                            content_width,
                            line_num,
                            num_width,
                            line_numbers,
                        );
                        for mut line in wrapped {
                            if output_lines.len() >= inner.height as usize {
                                break;
                            }
                            if is_highlighted {
                                // Apply highlight background to all spans
                                let hl_bg = Color::Rgb(55, 55, 55);
                                line = Line::from(
                                    line.spans
                                        .into_iter()
                                        .map(|s| Span::styled(s.content, s.style.bg(hl_bg)))
                                        .collect::<Vec<_>>(),
                                );
                                // Also highlight pattern matches if provided
                                if let Some(pattern) = highlight_pattern {
                                    line = highlight_pattern_in_line(line, pattern);
                                }
                            }
                            output_lines.push(line);
                        }
                    }

                    let text = Paragraph::new(output_lines);
                    frame.render_widget(text, inner);
                } else {
                    // No wrap: truncate lines
                    let lines: Vec<Line> = file_preview
                        .lines
                        .iter()
                        .enumerate()
                        .skip(file_preview.scroll_offset)
                        .take(inner.height as usize)
                        .map(|(i, hl)| {
                            let line_num = first_line_num + i;
                            let is_highlighted = highlight_line == Some(line_num);
                            let mut line = if line_numbers {
                                hl.to_line_numbered(line_num, width, num_width)
                            } else {
                                hl.to_line(width)
                            };
                            if is_highlighted {
                                // Apply highlight background to all spans
                                let hl_bg = Color::Rgb(55, 55, 55);
                                line = Line::from(
                                    line.spans
                                        .into_iter()
                                        .map(|s| Span::styled(s.content, s.style.bg(hl_bg)))
                                        .collect::<Vec<_>>(),
                                );
                                // Also highlight pattern matches if provided
                                if let Some(pattern) = highlight_pattern {
                                    line = highlight_pattern_in_line(line, pattern);
                                }
                            }
                            line
                        })
                        .collect();

                    let text = Paragraph::new(lines);
                    frame.render_widget(text, inner);
                }
            }
        }
        Preview::Image(img) => {
            let block = Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style);

            let inner = block.inner(area);
            frame.render_widget(block, area);

            // Store the image position for rendering after the frame
            // Only update if the image data changed to avoid flickering
            unsafe {
                let needs_update = match &PENDING_IMAGE {
                    Some((old_img, _, _, _)) => old_img.data != img.data,
                    None => true,
                };
                if needs_update {
                    PENDING_IMAGE = Some((img.clone(), inner.x, inner.y, true));
                }
            }
        }
        Preview::Error(msg) => {
            // Clear any lingering images
            unsafe {
                PENDING_IMAGE = None;
            }

            let block = Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style);

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let text = Paragraph::new(msg.as_str()).style(Style::default().fg(Color::Red));
            frame.render_widget(text, inner);
        }
        Preview::None => {
            // Clear any lingering images
            unsafe {
                PENDING_IMAGE = None;
            }

            let block = Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style);
            frame.render_widget(block, area);
        }
    }
}

// Static storage for pending image render (written after frame)
// Tuple: (image, x, y, needs_redraw)
static mut PENDING_IMAGE: Option<(crate::core::ImagePreview, u16, u16, bool)> = None;
static mut HAD_IMAGE: bool = false;

/// Render any pending image after the frame has been drawn
#[allow(static_mut_refs)]
pub fn render_pending_image() {
    use std::io::Write;

    unsafe {
        match &PENDING_IMAGE {
            Some((ref img, x, y, needs_redraw)) => {
                if *needs_redraw {
                    let mut stdout = std::io::stdout();
                    // Clear previous images first using Kitty delete command
                    let _ = stdout.write_all(b"\x1b_Gq=2,a=d,d=A\x1b\\");
                    // Move cursor to image position and write the image
                    let _ = crossterm::execute!(stdout, crossterm::cursor::MoveTo(*x, *y));
                    let _ = stdout.write_all(&img.data);
                    let _ = stdout.flush();
                }
                HAD_IMAGE = true;
                // Mark as not needing redraw
                if let Some((_, _, _, ref mut nr)) = PENDING_IMAGE {
                    *nr = false;
                }
            }
            None => {
                // If we had an image before but not now, clear it
                if HAD_IMAGE {
                    let mut stdout = std::io::stdout();
                    let _ = stdout.write_all(b"\x1b_Gq=2,a=d,d=A\x1b\\");
                    let _ = stdout.flush();
                    HAD_IMAGE = false;
                }
            }
        }
    }
}

/// Render audio player in the preview area (at top, always shown for audio files)
fn render_audio_player(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    selected_path: Option<&std::path::Path>,
) {
    // Check if selected file is an audio file
    let is_audio = selected_path.map(is_audio_file).unwrap_or(false);
    if !is_audio {
        return;
    }

    // Player height: 3 lines (border + content + border)
    let player_height = 3u16;
    if area.height < player_height + 2 {
        return; // Not enough space
    }

    // Position at top of preview area, inside the border
    let player_area = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: player_height,
    };

    // Get player state if available
    let player = app.audio_player.as_ref();
    let is_playing = player.map(|p| p.is_playing()).unwrap_or(false);
    let is_paused = player.map(|p| p.is_paused()).unwrap_or(false);
    let is_active = is_playing || is_paused;

    // Build progress bar
    let progress = if is_active {
        player.and_then(|p| p.progress()).unwrap_or(0.0)
    } else {
        0.0
    };

    let elapsed = if is_active {
        player
            .map(|p| p.elapsed())
            .unwrap_or(std::time::Duration::ZERO)
    } else {
        std::time::Duration::ZERO
    };

    let duration = if is_active {
        player.and_then(|p| p.duration())
    } else {
        None
    };

    // Format time with two decimal places
    let format_time = |d: std::time::Duration| -> String {
        let total_secs = d.as_secs_f64();
        let mins = (total_secs / 60.0).floor() as u64;
        let secs = total_secs % 60.0;
        format!("{:02}:{:05.2}", mins, secs)
    };

    let time_str = match duration {
        Some(d) => format!(" {} / {} ", format_time(elapsed), format_time(d)),
        None if is_active => format!(" {} ", format_time(elapsed)),
        None => " --:--.-- / --:--.-- ".to_string(),
    };

    // Icon: show play (▶) by default, pause (⏸) when playing
    // Use U+FE0E variation selector to force text presentation (not emoji)
    let play_icon = if is_playing {
        "⏸\u{FE0E}"
    } else {
        "▶\u{FE0E}"
    };

    let icon_color = if is_playing {
        Color::Green
    } else if is_paused {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    // Calculate bar width: total width - borders(2) - icon(" X "=3) - time string
    let icon_width = 3; // " ▶ "
    let time_width = time_str.chars().count();
    let available_width = player_area.width.saturating_sub(2) as usize; // Inside borders
    let bar_width = available_width.saturating_sub(icon_width + time_width);

    let filled = (bar_width as f32 * progress) as usize;
    let empty = bar_width.saturating_sub(filled);

    // Progress bar: always show dim track, fill with bright color
    let bar_filled: String = "━".repeat(filled);
    let bar_track: String = "━".repeat(empty);

    // Time color: cyan when active, muted gray when inactive
    let time_color = if is_active {
        Color::Cyan
    } else {
        Color::Rgb(60, 60, 60)
    };

    // Create spans with colors
    let content = Line::from(vec![
        Span::styled(format!(" {} ", play_icon), Style::default().fg(icon_color)),
        Span::styled(bar_filled, Style::default().fg(Color::Magenta)),
        Span::styled(bar_track, Style::default().fg(Color::Rgb(60, 60, 60))),
        Span::styled(time_str, Style::default().fg(time_color)),
    ]);

    // Render without background color
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(96, 98, 104)));

    let para = Paragraph::new(content).block(block);

    // Clear the area first
    frame.render_widget(Clear, player_area);
    frame.render_widget(para, player_area);
}

/// Wrap a highlighted line, returning multiple Lines with proper gutter
fn wrap_highlighted_line(
    hl: &crate::core::highlight::HighlightedLine,
    content_width: usize,
    line_num: usize,
    num_width: usize,
    show_line_num: bool,
) -> Vec<Line<'static>> {
    if content_width == 0 {
        return vec![];
    }

    let mut result: Vec<Line> = Vec::new();
    let mut current_spans: Vec<Span> = Vec::new();
    let mut current_width = 0;
    let mut is_first_line = true;

    let gutter_style = Style::default().fg(Color::DarkGray);

    for (text, style) in &hl.spans {
        let mut remaining = text.as_str();

        while !remaining.is_empty() {
            let available = content_width.saturating_sub(current_width);
            if available == 0 {
                // Emit current line
                let mut line_spans = Vec::new();
                if show_line_num {
                    if is_first_line {
                        line_spans.push(Span::styled(
                            format!("{:>width$} ", line_num, width = num_width),
                            gutter_style,
                        ));
                        is_first_line = false;
                    } else {
                        line_spans.push(Span::styled(" ".repeat(num_width + 1), gutter_style));
                    }
                }
                line_spans.append(&mut current_spans);
                result.push(Line::from(line_spans));
                current_width = 0;
                continue;
            }

            let char_count = remaining.chars().count();
            if char_count <= available {
                current_spans.push(Span::styled(remaining.to_string(), *style));
                current_width += char_count;
                break;
            } else {
                // Split at available boundary
                let split_at: String = remaining.chars().take(available).collect();
                let rest: String = remaining.chars().skip(available).collect();
                current_spans.push(Span::styled(split_at, *style));
                current_width += available;
                remaining = Box::leak(rest.into_boxed_str()); // Need to extend lifetime
            }
        }
    }

    // Emit remaining content
    if !current_spans.is_empty() || is_first_line {
        let mut line_spans = Vec::new();
        if show_line_num {
            if is_first_line {
                line_spans.push(Span::styled(
                    format!("{:>width$} ", line_num, width = num_width),
                    gutter_style,
                ));
            } else {
                line_spans.push(Span::styled(" ".repeat(num_width + 1), gutter_style));
            }
        }
        line_spans.extend(current_spans);
        result.push(Line::from(line_spans));
    }

    result
}

fn render_theme_select(frame: &mut Frame, area: Rect, selected: usize) {
    let themes = available_themes();
    let current = current_theme();

    // Center the popup
    let popup_width = 40.min(area.width.saturating_sub(4));
    let popup_height = (themes.len() + 4).min(area.height.saturating_sub(4) as usize) as u16;
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "Use j/k to navigate, Enter to select",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    for (i, name) in themes.iter().enumerate() {
        let is_current = name == &current;
        let is_selected = i == selected;

        let prefix = if is_current { "* " } else { "  " };
        let style = if is_selected {
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else if is_current {
            Style::default().fg(Color::Green)
        } else {
            Style::default()
        };

        lines.push(Line::from(Span::styled(
            format!("{}{}", prefix, name),
            style,
        )));
    }

    let block = Block::default()
        .title(" Colors ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let mode_style = match app.mode {
        Mode::Normal => Style::default().fg(Color::Green),
        Mode::Insert => Style::default().fg(Color::Yellow),
        Mode::Visual { .. } => Style::default().fg(Color::Blue),
        Mode::VisualInsert { .. } => Style::default().fg(Color::LightBlue),
        Mode::Confirm(_) => Style::default().fg(Color::Red),
        Mode::SyncConfirm { .. } => Style::default().fg(Color::Yellow),
        Mode::QuitConfirm { .. } => Style::default().fg(Color::Red),
        Mode::Search(_) => Style::default().fg(Color::Cyan),
        Mode::Find => Style::default().fg(Color::Magenta),
        Mode::Help => Style::default().fg(Color::Cyan),
        Mode::Settings => Style::default().fg(Color::Cyan),
        Mode::ThemeSelect { .. } => Style::default().fg(Color::Magenta),
        Mode::Grep => Style::default().fg(Color::Green),
        Mode::Sort { .. } => Style::default().fg(Color::Cyan),
        Mode::InfoSelect { .. } => Style::default().fg(Color::Cyan),
        Mode::Leader => Style::default().fg(Color::Magenta),
        Mode::PreviewOptions => Style::default().fg(Color::Magenta),
        Mode::Git => Style::default().fg(Color::Green),
        Mode::GitStatus { .. } => Style::default().fg(Color::Green),
        Mode::GitCommit { .. } => Style::default().fg(Color::Green),
        Mode::Audio => Style::default().fg(Color::Magenta),
        Mode::Command => Style::default().fg(Color::Yellow),
    };

    // Simplified status bar during search mode
    if matches!(app.mode, Mode::Search(_)) {
        let status = Line::from(vec![
            Span::styled(format!(" {} ", app.mode.name()), mode_style),
            Span::raw(" "),
            Span::styled(&app.status, Style::default().fg(Color::Magenta)),
        ]);
        let paragraph = Paragraph::new(status);
        frame.render_widget(paragraph, area);
        return;
    }

    // Shell-command input line: " COMMAND  !<input>   %f file  %d dir  %n name"
    if matches!(app.mode, Mode::Command) {
        let status = Line::from(vec![
            Span::styled(format!(" {} ", app.mode.name()), mode_style),
            Span::raw(" "),
            Span::styled(
                format!("!{}", app.command_input),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                "   %f file  %d dir  %n name",
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        let paragraph = Paragraph::new(status);
        frame.render_widget(paragraph, area);
        return;
    }

    let dirty = if app.current.buffer.dirty { " [+]" } else { "" };
    let pending_total = app.global_operations.total_count();
    let pending_here = app.global_operations.count_for_dir(&app.cwd);
    let pending_str = if pending_total > 0 {
        if pending_here > 0 && pending_here < pending_total {
            format!(" ({} pending, {} here)", pending_total, pending_here)
        } else if pending_here == pending_total {
            format!(" ({} pending)", pending_total)
        } else {
            format!(" ({} pending, 0 here)", pending_total)
        }
    } else {
        String::new()
    };

    // Clean the cwd path for display. In grep/find modes, show the search root
    // (which may differ from app.cwd when searching from work_dir).
    let display_path = if let Some(ref state) = app.grep_state {
        &state.search_root
    } else if let Some(ref state) = app.find_state {
        &state.search_root
    } else {
        &app.cwd
    };
    let cwd_display = {
        let cwd_str = display_path.to_string_lossy().to_string();
        #[cfg(target_os = "windows")]
        {
            cwd_str
                .strip_prefix(r"\\?\")
                .map(str::to_string)
                .unwrap_or(cwd_str)
        }
        #[cfg(not(target_os = "windows"))]
        {
            cwd_str
        }
    };

    // Determine help hint based on mode
    let help_hint = if app.find_state.is_some() || app.grep_state.is_some() {
        "Ctrl+g: settings"
    } else {
        "Ctrl+g: help"
    };

    let status = Line::from(vec![
        Span::styled(format!(" {} ", app.mode.name()), mode_style),
        Span::raw(" "),
        Span::styled(cwd_display, Style::default().fg(Color::DarkGray)),
        Span::styled(dirty, Style::default().fg(Color::Red)),
        Span::styled(pending_str, Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(&app.status, Style::default().fg(Color::Yellow)),
    ]);

    // Calculate space for right-aligned hint
    let status_width: usize = status.spans.iter().map(|s| s.content.chars().count()).sum();
    let hint_width = help_hint.chars().count() + 1; // +1 for trailing space
    let available = area.width as usize;

    let paragraph = Paragraph::new(status);
    frame.render_widget(paragraph, area);

    // Render hint on the right if there's space
    if available > status_width + hint_width + 2 {
        let hint_x = area.x + area.width - hint_width as u16;
        let hint_area = Rect::new(hint_x, area.y, hint_width as u16, 1);
        let hint = Paragraph::new(Span::styled(
            format!("{} ", help_hint),
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(hint, hint_area);
    }
}
