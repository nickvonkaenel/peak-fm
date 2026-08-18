//! Find and Grep mode rendering

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use devicons::FileIcon;

use crate::app::App;
use crate::core::Highlighter;

use super::{render_preview, render_status};

pub(super) fn render_find_mode(frame: &mut Frame, area: Rect, app: &mut App) {
    // Layout: status bar at bottom, content above
    let main_chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);

    let content_area = main_chunks[0];
    let status_area = main_chunks[1];

    // Render status bar
    render_status(frame, status_area, app);

    // Determine layout based on aspect ratio
    let width = content_area.width;
    let height = content_area.height;
    let aspect_ratio = width as f32 / height.max(1) as f32;
    let use_horizontal_preview = width < 80 && height >= 20 && aspect_ratio < 1.8;

    let (list_area, preview_area) = if use_horizontal_preview {
        // Vertical split: list on top, preview at bottom
        let chunks = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(content_area);
        (chunks[0], chunks[1])
    } else {
        // Horizontal split: list on left, preview on right
        let columns = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(content_area);
        (columns[0], columns[1])
    };

    let left_area = list_area;
    let right_area = preview_area;

    // Left side: input box at top + results list below
    let left_chunks = Layout::vertical([
        Constraint::Length(3), // Input box
        Constraint::Min(1),    // Results list
    ])
    .split(left_area);

    let input_area = left_chunks[0];
    let results_area = left_chunks[1];

    // Update results height for scrolling
    if let Some(ref mut state) = app.find_state {
        state.results_height = results_area.height.saturating_sub(2) as usize;
    }

    // Render input box
    let query = app
        .find_state
        .as_ref()
        .map(|s| s.query.as_str())
        .unwrap_or("");
    let input_text = format!("> {}", query);
    let title = if app.zoxide_mode {
        " Zoxide "
    } else {
        " Search "
    };
    let input_block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));
    let input_paragraph = Paragraph::new(input_text).block(input_block);
    frame.render_widget(input_paragraph, input_area);

    // Set cursor position in input box (cursor style set after frame draw)
    let cursor_x = input_area.x + 3 + query.len() as u16; // "> " prefix + query
    let cursor_y = input_area.y + 1;
    frame.set_cursor_position((cursor_x, cursor_y));

    // Render results list
    let results_label = if app.zoxide_mode {
        "Directories"
    } else {
        "Results"
    };
    let results_block = Block::default()
        .title(format!(
            " {} ({}) ",
            results_label,
            app.find_state
                .as_ref()
                .map(|s| s.matched_count())
                .unwrap_or(0)
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    if let Some(ref state) = app.find_state {
        let query_lower: Vec<char> = state.query.to_lowercase().chars().collect();
        let items: Vec<ListItem> = state
            .visible_results()
            .into_iter()
            .map(|(i, entry)| {
                let is_selected = i == state.selected;
                let sel_bg = Color::Rgb(55, 55, 55); // Dim background for selection

                let base_style = if is_selected {
                    Style::default().bg(sel_bg)
                } else {
                    Style::default()
                };

                let filename_style = if entry.is_dir {
                    base_style.fg(Color::Blue)
                } else {
                    base_style
                };

                let folder_style = if is_selected {
                    Style::default().bg(sel_bg).fg(Color::Rgb(96, 98, 104))
                } else {
                    Style::default().fg(Color::Rgb(96, 98, 104))
                };

                let highlight_style = if is_selected {
                    Style::default().bg(sel_bg).fg(Color::Rgb(255, 165, 87))
                } else {
                    Style::default().fg(Color::Rgb(255, 165, 87))
                };

                let prefix = if is_selected { "> " } else { "  " };
                let suffix = if entry.is_dir && !app.show_icons {
                    "/"
                } else {
                    ""
                };

                // Split into filename and parent folder
                let path = std::path::Path::new(&entry.display);
                let filename = path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| entry.display.clone());
                let parent = path
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .filter(|p| !p.is_empty());

                // Get file icon if enabled
                let file_icon_span = if app.show_icons {
                    let icon = FileIcon::from(filename.as_str());
                    let (icon_char, icon_color) = if icon.icon == '*' {
                        // Fallback for unrecognized files/directories
                        let fallback_icon = if entry.is_dir {
                            '\u{f115}' // Nerd Font: nf-fa-folder_open_o
                        } else {
                            '\u{f15b}' // Nerd Font: nf-fa-file
                        };
                        let color = if entry.is_dir {
                            Color::Blue // Blue for folders (matches directory text)
                        } else if app.colored_icons {
                            Color::Gray // Gray for unknown files
                        } else {
                            Color::Reset // Default text color
                        };
                        (fallback_icon, color)
                    } else {
                        // Use devicons icon
                        let color = if app.colored_icons {
                            if icon.color.len() == 7 && icon.color.starts_with('#') {
                                let r = u8::from_str_radix(&icon.color[1..3], 16).unwrap_or(128);
                                let g = u8::from_str_radix(&icon.color[3..5], 16).unwrap_or(128);
                                let b = u8::from_str_radix(&icon.color[5..7], 16).unwrap_or(128);
                                if app.theme_icons {
                                    crate::core::map_to_theme_color(r, g, b)
                                } else {
                                    Color::Rgb(r, g, b)
                                }
                            } else {
                                Color::Gray
                            }
                        } else {
                            // Monochrome: match directory/file text colors
                            if entry.is_dir {
                                Color::Blue
                            } else {
                                Color::Reset
                            }
                        };
                        (icon.icon, color)
                    };
                    let icon_style = if is_selected {
                        Style::default().fg(icon_color).bg(sel_bg)
                    } else {
                        Style::default().fg(icon_color)
                    };
                    Some(Span::styled(format!("{} ", icon_char), icon_style))
                } else {
                    None
                };

                let filename_with_suffix = format!("{}{}", filename, suffix);

                // Find fuzzy match positions in the full display path (for accurate matching)
                let full_display = format!("{}{}", entry.display, suffix);
                let display_lower: Vec<char> = full_display.to_lowercase().chars().collect();

                let mut match_indices = Vec::new();
                if !query_lower.is_empty() {
                    let mut query_idx = 0;
                    for (i, &c) in display_lower.iter().enumerate() {
                        if query_idx < query_lower.len() && c == query_lower[query_idx] {
                            match_indices.push(i);
                            query_idx += 1;
                        }
                    }
                }

                // Build spans: prefix, icon, filename (with highlights), folder (muted, with highlights)
                let mut spans = vec![Span::styled(prefix, base_style)];

                // Add file icon if enabled
                if let Some(icon_span) = file_icon_span {
                    spans.push(icon_span);
                }

                // Calculate where filename starts in the full path
                let filename_start = if let Some(ref p) = parent {
                    p.len() + 1 // +1 for the path separator
                } else {
                    0
                };

                // Render filename with highlights
                let filename_chars: Vec<char> = filename_with_suffix.chars().collect();
                if query_lower.is_empty() {
                    spans.push(Span::styled(filename_with_suffix.clone(), filename_style));
                } else {
                    let mut last_end = 0;
                    for &idx in &match_indices {
                        // Adjust index to be relative to filename
                        if idx >= filename_start && idx < filename_start + filename_chars.len() {
                            let rel_idx = idx - filename_start;
                            if rel_idx > last_end {
                                let segment: String =
                                    filename_chars[last_end..rel_idx].iter().collect();
                                spans.push(Span::styled(segment, filename_style));
                            }
                            spans.push(Span::styled(
                                filename_chars[rel_idx].to_string(),
                                highlight_style,
                            ));
                            last_end = rel_idx + 1;
                        }
                    }
                    if last_end < filename_chars.len() {
                        let segment: String = filename_chars[last_end..].iter().collect();
                        spans.push(Span::styled(segment, filename_style));
                    }
                }

                // Render folder path (muted) with highlights
                if let Some(folder) = parent {
                    let folder_chars: Vec<char> = folder.chars().collect();
                    if query_lower.is_empty() {
                        spans.push(Span::styled(format!(" {}", folder), folder_style));
                    } else {
                        spans.push(Span::styled(" ", base_style));
                        let mut last_end = 0;
                        for &idx in &match_indices {
                            // Check if this index is within the folder part
                            if idx < folder_chars.len() {
                                if idx > last_end {
                                    let segment: String =
                                        folder_chars[last_end..idx].iter().collect();
                                    spans.push(Span::styled(segment, folder_style));
                                }
                                // Use muted highlight for folder matches
                                let folder_highlight = if is_selected {
                                    Style::default().bg(sel_bg).fg(Color::Rgb(255, 165, 87))
                                } else {
                                    Style::default().fg(Color::Rgb(255, 165, 87))
                                };
                                spans.push(Span::styled(
                                    folder_chars[idx].to_string(),
                                    folder_highlight,
                                ));
                                last_end = idx + 1;
                            }
                        }
                        if last_end < folder_chars.len() {
                            let segment: String = folder_chars[last_end..].iter().collect();
                            spans.push(Span::styled(segment, folder_style));
                        }
                    }
                }

                let mut line = Line::from(spans);

                // Zoxide mode: show the frecency score right-aligned and dim
                if app.zoxide_mode {
                    if let Some(ref score) = entry.score {
                        let inner_width = results_area.width.saturating_sub(2) as usize;
                        let used = line.width();
                        let score_width = score.chars().count();
                        // Need at least one space between path and score
                        if used + score_width < inner_width {
                            let pad = inner_width - used - score_width;
                            line.spans.push(Span::styled(" ".repeat(pad), base_style));
                            line.spans.push(Span::styled(score.clone(), folder_style));
                        }
                    }
                }

                ListItem::new(line)
            })
            .collect();

        let list = List::new(items).block(results_block);
        frame.render_widget(list, results_area);
    } else {
        frame.render_widget(results_block, results_area);
    }

    // Update preview height
    app.preview_height = right_area.height.saturating_sub(2) as usize;

    // Get preview title from selected file
    let preview_title = app
        .find_state
        .as_ref()
        .and_then(|s| s.selected_entry())
        .and_then(|e| std::path::Path::new(&e.display).file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("Preview");

    // Render preview on right side
    render_preview(
        frame,
        right_area,
        &app.preview,
        preview_title,
        app.wrap_preview,
        app.line_numbers,
        None,
        app.show_icons,
        app.colored_icons,
        app.theme_icons,
    );
}

pub(super) fn render_grep_mode(frame: &mut Frame, area: Rect, app: &mut App) {
    // Layout: status bar at bottom, content above
    let main_chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);

    let content_area = main_chunks[0];
    let status_area = main_chunks[1];

    // Render status bar
    render_status(frame, status_area, app);

    // Determine layout based on aspect ratio
    let width = content_area.width;
    let height = content_area.height;
    let aspect_ratio = width as f32 / height.max(1) as f32;
    let use_horizontal_preview = width < 80 && height >= 20 && aspect_ratio < 1.8;

    let (list_area, preview_area) = if use_horizontal_preview {
        // Vertical split: list on top, preview at bottom
        let chunks = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(content_area);
        (chunks[0], chunks[1])
    } else {
        // Horizontal split: list on left, preview on right
        let columns = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(content_area);
        (columns[0], columns[1])
    };

    let left_area = list_area;
    let right_area = preview_area;

    // Left side: input box + results
    let left_chunks =
        Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(left_area);

    let input_area = left_chunks[0];
    let results_area = left_chunks[1];

    // Update results height for scrolling
    if let Some(ref mut state) = app.grep_state {
        state.results_height = results_area.height.saturating_sub(2) as usize;
    }

    // Render input box
    let query = app
        .grep_state
        .as_ref()
        .map(|s| s.query.as_str())
        .unwrap_or("");
    let input_text = format!("> {}", query);
    let input_block = Block::default()
        .title(" Grep ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));
    let input_paragraph = Paragraph::new(input_text).block(input_block);
    frame.render_widget(input_paragraph, input_area);

    // Set cursor position in input box (cursor style set after frame draw)
    let cursor_x = input_area.x + 3 + query.len() as u16;
    let cursor_y = input_area.y + 1;
    frame.set_cursor_position((cursor_x, cursor_y));

    // Render results list
    let match_count = app
        .grep_state
        .as_ref()
        .map(|s| s.matches.len())
        .unwrap_or(0);
    let results_block = Block::default()
        .title(format!(" Matches ({}) ", match_count))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    if let Some(ref state) = app.grep_state {
        // Compile regex once for all results (case-insensitive)
        // Smart case: case insensitive unless query contains uppercase
        let has_uppercase = state.query.chars().any(|c| c.is_uppercase());
        let query_regex = regex::RegexBuilder::new(&state.query)
            .case_insensitive(!has_uppercase)
            .build()
            .ok();
        let items: Vec<ListItem> = state
            .visible_results()
            .into_iter()
            .map(|(i, result)| {
                let is_selected = i == state.selected;
                let sel_bg = Color::Rgb(55, 55, 55); // Dim background for selection

                let base_style = if is_selected {
                    Style::default().bg(sel_bg)
                } else {
                    Style::default()
                };

                let highlight_style = if is_selected {
                    Style::default().bg(sel_bg).fg(Color::Rgb(255, 165, 87))
                } else {
                    Style::default().fg(Color::Rgb(255, 165, 87))
                };

                let prefix = if is_selected { "> " } else { "  " };

                // Extract filename and parent folder separately
                let filename = result
                    .path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| result.path.to_string_lossy().to_string());
                let parent = result
                    .path
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .filter(|p| !p.is_empty());
                let line_num_str = result.line_num.to_string();
                let content = result.line.trim();

                // Get file icon if enabled (grep results are always files)
                let file_icon_span = if app.show_icons {
                    let icon = FileIcon::from(filename.as_str());
                    // Use generic file icon if devicons returns asterisk (unknown)
                    let icon_char = if icon.icon == '*' {
                        '\u{f15b}' // Nerd Font: nf-fa-file
                    } else {
                        icon.icon
                    };
                    let icon_color = if app.colored_icons {
                        if icon.icon != '*' && icon.color.len() == 7 && icon.color.starts_with('#')
                        {
                            let r = u8::from_str_radix(&icon.color[1..3], 16).unwrap_or(128);
                            let g = u8::from_str_radix(&icon.color[3..5], 16).unwrap_or(128);
                            let b = u8::from_str_radix(&icon.color[5..7], 16).unwrap_or(128);
                            if app.theme_icons {
                                crate::core::map_to_theme_color(r, g, b)
                            } else {
                                Color::Rgb(r, g, b)
                            }
                        } else {
                            Color::Gray // Default color for unknown files
                        }
                    } else {
                        // Monochrome: use default file text color
                        Color::Reset
                    };
                    let icon_style = if is_selected {
                        Style::default().fg(icon_color).bg(sel_bg)
                    } else {
                        Style::default().fg(icon_color)
                    };
                    Some(Span::styled(format!("{} ", icon_char), icon_style))
                } else {
                    None
                };

                // Calculate max content length
                let parent_len = parent.as_ref().map(|p| p.len() + 1).unwrap_or(0); // +1 for space
                let fixed_len =
                    prefix.len() + filename.len() + 1 + parent_len + line_num_str.len() + 2;
                let max_content_len = results_area.width.saturating_sub(6) as usize;
                let available_for_content = max_content_len.saturating_sub(fixed_len);

                // Truncate content if needed
                let truncated_content = if content.chars().count() > available_for_content {
                    let truncated_str: String = content
                        .chars()
                        .take(available_for_content.saturating_sub(3))
                        .collect();
                    format!("{}...", truncated_str)
                } else {
                    content.to_string()
                };

                // Style definitions
                let filename_style = if is_selected {
                    Style::default().bg(sel_bg)
                } else {
                    Style::default()
                };

                let folder_style = if is_selected {
                    Style::default().bg(sel_bg).fg(Color::Rgb(96, 98, 104))
                } else {
                    Style::default().fg(Color::Rgb(96, 98, 104))
                };

                let line_num_style = if is_selected {
                    Style::default().bg(sel_bg).fg(Color::Magenta)
                } else {
                    Style::default().fg(Color::Magenta)
                };

                let separator_style = if is_selected {
                    Style::default().bg(sel_bg).fg(Color::Rgb(96, 98, 104))
                } else {
                    Style::default().fg(Color::Rgb(96, 98, 104))
                };

                // Start building spans: prefix icon filename folder line_num: content
                let mut spans = vec![Span::styled(prefix, base_style)];

                // Add file icon if enabled
                if let Some(icon_span) = file_icon_span {
                    spans.push(icon_span);
                }

                spans.push(Span::styled(filename, filename_style));

                // Add folder path if present
                if let Some(folder) = parent {
                    spans.push(Span::styled(format!(" {}", folder), folder_style));
                }

                spans.push(Span::styled(" ", base_style));
                spans.push(Span::styled(line_num_str, line_num_style));
                spans.push(Span::styled(": ", separator_style));

                // Apply syntax highlighting to content
                let full_path = state.search_root.join(&result.path);
                let highlighter = Highlighter::for_path(&full_path);
                let highlighted = highlighter.highlight_single_line(&truncated_content);

                // Find all pattern matches on the full content first
                let matches: Vec<(usize, usize)> = query_regex
                    .as_ref()
                    .map(|re| {
                        re.find_iter(&truncated_content)
                            .map(|m| (m.start(), m.end()))
                            .collect()
                    })
                    .unwrap_or_default();

                // Track position in full content as we iterate through syntax spans
                let mut content_pos = 0;
                for (text, style) in highlighted.spans {
                    let span_start = content_pos;
                    let span_end = content_pos + text.len();

                    // Check if any matches overlap with this span
                    let overlapping: Vec<_> = matches
                        .iter()
                        .filter(|(m_start, m_end)| *m_start < span_end && *m_end > span_start)
                        .collect();

                    if overlapping.is_empty() {
                        // No matches in this span - just apply syntax style
                        let combined_style = if is_selected { style.bg(sel_bg) } else { style };
                        spans.push(Span::styled(text, combined_style));
                    } else {
                        // Split this span at match boundaries
                        let mut last_end = 0; // relative to span start
                        for (m_start, m_end) in overlapping {
                            // Convert to span-relative positions
                            let rel_start = m_start.saturating_sub(span_start);
                            let rel_end = (*m_end - span_start).min(text.len());

                            if rel_start > last_end {
                                let combined_style =
                                    if is_selected { style.bg(sel_bg) } else { style };
                                spans.push(Span::styled(
                                    text[last_end..rel_start].to_string(),
                                    combined_style,
                                ));
                            }
                            if rel_start < text.len() {
                                spans.push(Span::styled(
                                    text[rel_start.max(last_end)..rel_end].to_string(),
                                    highlight_style,
                                ));
                            }
                            last_end = rel_end;
                        }
                        if last_end < text.len() {
                            let combined_style = if is_selected { style.bg(sel_bg) } else { style };
                            spans.push(Span::styled(text[last_end..].to_string(), combined_style));
                        }
                    }
                    content_pos = span_end;
                }

                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(items).block(results_block);
        frame.render_widget(list, results_area);
    } else {
        frame.render_widget(results_block, results_area);
    }

    // Update preview height
    app.preview_height = right_area.height.saturating_sub(2) as usize;

    // Get preview title from selected match
    let preview_title = app
        .grep_state
        .as_ref()
        .and_then(|s| s.matches.get(s.selected))
        .and_then(|m| m.path.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("Preview");

    // Render preview on right side with grep pattern for highlighting
    let grep_pattern = app.grep_state.as_ref().map(|s| s.query.as_str());
    render_preview(
        frame,
        right_area,
        &app.preview,
        preview_title,
        app.wrap_preview,
        app.line_numbers,
        grep_pattern,
        app.show_icons,
        app.colored_icons,
        app.theme_icons,
    );
}
