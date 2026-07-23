//! Dialog rendering - settings, confirms, help screens

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::App;
use crate::input::ConfirmAction;

pub(super) fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    let in_search_mode = app.find_state.is_some();
    let in_grep_mode = app.grep_state.is_some();

    // Different help content for search/grep modes vs normal mode
    let (help_text, popup_height) = if in_search_mode || in_grep_mode {
        (
            vec![
                Line::from(Span::styled(
                    "Navigation",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from("  C-j/C-k  Move selection down/up"),
                Line::from("  ↓/↑      Move selection down/up"),
                Line::from("  C-d/C-u  Half page down/up"),
                Line::from("  C-f/C-b  Scroll preview down/up"),
                Line::from(""),
                Line::from(Span::styled(
                    "Actions",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from("  Enter    Open selected file"),
                Line::from("  C-n      Navigate to file location"),
                Line::from("  C-p      Play/pause audio (if audio file)"),
                Line::from(""),
                Line::from(Span::styled(
                    "Editing Query",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from("  Type     Add characters to query"),
                Line::from("  Bksp     Delete last character"),
                Line::from("  C-w      Delete last word"),
                Line::from("  C-l      Clear query"),
                Line::from(""),
                Line::from(Span::styled(
                    "Other",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from("  C-g      Open settings"),
                Line::from("  Esc      Exit search mode"),
            ],
            22.min(area.height.saturating_sub(4)),
        )
    } else {
        (
            vec![
                Line::from(Span::styled(
                    "Navigation",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from("  j/k      Move cursor down/up"),
                Line::from("  h/l      Navigate out/in (parent/open)"),
                Line::from("  Enter    Open file / play audio"),
                Line::from("  t/b      Jump to top/bottom"),
                Line::from("  -        Toggle previous directory"),
                Line::from("  C-d/C-u  Half page down/up"),
                Line::from("  C-j/C-k  Half page down/up"),
                Line::from("  C-f/C-b  Scroll preview down/up"),
                Line::from(""),
                Line::from(Span::styled(
                    "Editing",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from("  i/a/A    Insert at start/before ext/end"),
                Line::from("  c        Clear name and edit"),
                Line::from("  o/O      New file/dir below/above"),
                Line::from("  d/D      Delete line / delete marked"),
                Line::from("  y/Y      Yank file / yank marked"),
                Line::from("  p        Paste yanked files"),
                Line::from("  u/C-r    Undo / redo"),
                Line::from("  C-y      Sync changes to disk"),
                Line::from(""),
                Line::from(Span::styled(
                    "Search",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from("  s/S      Search files (all / cwd)"),
                Line::from("  g/G      Grep content (all / cwd)"),
                Line::from("  r        Resume last search"),
                Line::from("  /        Filter current list"),
                Line::from("  n/N      Next/prev match"),
                Line::from(""),
                Line::from(Span::styled(
                    "Visual Mode",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from("  v        Enter visual mode"),
                Line::from("  j/k/g/G  Navigate (extends selection)"),
                Line::from("  Esc      Exit visual mode"),
                Line::from("  Most commands work in visual mode!"),
                Line::from(""),
                Line::from(Span::styled(
                    "Other",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from("  .        Toggle hidden files"),
                Line::from("  w/#      Toggle wrap / line numbers"),
                Line::from("  ;        Mark/unmark file"),
                Line::from("  !        Shell command on selection (%f %d %n)"),
                Line::from("  space    Leader menu"),
                Line::from("  z        Zoxide jump"),
                Line::from("  e        Open in editor"),
                Line::from("  f        Open audio browser"),
                Line::from("  C-l      Toggle audio playback"),
                Line::from("  C-o      Reveal in Finder"),
                Line::from("  C-t      Set work dir / empty trash"),
                Line::from("  x/X      Restore from trash / restore marked"),
                Line::from("  C        Copy file to system clipboard"),
                Line::from("  R        Copy to clipboard + open REAPER"),
                Line::from("  C-g      Show this help"),
                Line::from("  Esc      Stop audio / clear marks / quit"),
                Line::from("  q        Quit"),
            ],
            54.min(area.height.saturating_sub(4)),
        )
    };

    // Center the popup
    let popup_width = 50.min(area.width.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let title = if in_grep_mode {
        " Grep Help "
    } else if in_search_mode {
        " Search Help "
    } else {
        " Help "
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(help_text).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_settings(frame: &mut Frame, area: Rect, app: &App) {
    let in_find_mode = app.find_state.is_some();
    let in_grep_mode = app.grep_state.is_some();

    // Build settings text with current state indicators
    let wrap_state = if app.wrap_preview { "on" } else { "off" };
    let line_num_state = if app.line_numbers { "on" } else { "off" };
    let navigate_state = if app.search_navigate_on_open {
        "on"
    } else {
        "off"
    };

    // Different settings for grep mode vs search mode
    let (settings_text, title) = if in_grep_mode {
        // Grep mode: only preview and navigate settings work
        (
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    " Behavior",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  n  ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!("Navigate on open   [{}]", navigate_state)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    " Preview",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  w  ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!("Word wrap          [{}]", wrap_state)),
                ]),
                Line::from(vec![
                    Span::styled("  l  ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!("Line numbers       [{}]", line_num_state)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  h  ", Style::default().fg(Color::Yellow)),
                    Span::raw("Help"),
                ]),
            ],
            " Grep Settings ",
        )
    } else {
        // Search mode: full settings
        let hidden_state = if in_find_mode {
            if app.find_show_hidden {
                "on"
            } else {
                "off"
            }
        } else if app.show_hidden {
            "on"
        } else {
            "off"
        };
        let gitignore_state = if app.find_use_gitignore { "on" } else { "off" };
        let directories_state = if app.find_show_directories {
            "on"
        } else {
            "off"
        };

        (
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    " Search",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  .  ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!("Include hidden     [{}]", hidden_state)),
                ]),
                Line::from(vec![
                    Span::styled("  i  ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!("Use .gitignore     [{}]", gitignore_state)),
                ]),
                Line::from(vec![
                    Span::styled("  d  ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!("Show directories   [{}]", directories_state)),
                ]),
                Line::from(vec![
                    Span::styled("  n  ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!("Navigate on open   [{}]", navigate_state)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    " Preview",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  w  ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!("Word wrap          [{}]", wrap_state)),
                ]),
                Line::from(vec![
                    Span::styled("  l  ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!("Line numbers       [{}]", line_num_state)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  h  ", Style::default().fg(Color::Yellow)),
                    Span::raw("Help"),
                ]),
            ],
            " Search Settings ",
        )
    };

    // Calculate popup size based on content
    let popup_width = 36.min(area.width.saturating_sub(4));
    let popup_height = (settings_text.len() as u16 + 2).min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(settings_text).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_sync_confirm(frame: &mut Frame, area: Rect, app: &App, scroll: usize) {
    // Get all operations grouped by directory
    let summary = app.global_operations.get_summary();
    let total_ops: usize = summary.iter().map(|(_, ops)| ops.len()).sum();
    let dir_count = summary.len();

    let header = if dir_count > 1 {
        format!("{} operation(s) in {} directories:", total_ops, dir_count)
    } else {
        format!("{} operation(s):", total_ops)
    };
    let footer = "Apply changes? y/n";

    let mut op_strings: Vec<(String, Color)> = Vec::new();

    // Group operations by directory
    for (dir, ops) in &summary {
        // Add directory header
        let dir_display = dir.to_string_lossy();
        #[cfg(target_os = "windows")]
        let dir_display = dir_display.strip_prefix(r"\\?\").unwrap_or(&dir_display);
        #[cfg(not(target_os = "windows"))]
        let dir_display = dir_display.as_ref();

        op_strings.push((format!("{}:", dir_display), Color::DarkGray));

        // Add operations for this directory
        for op in ops {
            let (icon, color, desc) = match op {
                crate::core::FsOperation::Create { path, is_dir } => {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    let kind = if *is_dir { "dir" } else { "file" };
                    ("+", Color::Green, format!("Create {} {}", kind, name))
                }
                crate::core::FsOperation::Delete { path } => {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    ("-", Color::Red, format!("Delete {}", name))
                }
                crate::core::FsOperation::Rename { from, to } => {
                    let from_name = from.file_name().unwrap_or_default().to_string_lossy();
                    let to_name = to.file_name().unwrap_or_default().to_string_lossy();
                    ("~", Color::Yellow, format!("{} → {}", from_name, to_name))
                }
                crate::core::FsOperation::Copy { from, to, .. } => {
                    let from_name = from.file_name().unwrap_or_default().to_string_lossy();
                    let to_name = to.file_name().unwrap_or_default().to_string_lossy();
                    ("c", Color::Cyan, format!("{} → {}", from_name, to_name))
                }
            };
            op_strings.push((format!("  {} {}", icon, desc), color));
        }

        // Add blank line between directories if there are more
        if dir_count > 1 {
            op_strings.push((String::new(), Color::Reset));
        }
    }

    // Calculate required width based on content
    let mut max_content_width = header.len();
    max_content_width = max_content_width.max(footer.len());
    for (s, _) in &op_strings {
        max_content_width = max_content_width.max(s.len());
    }

    // Add padding for borders (2) and some internal margin (4)
    let popup_width = ((max_content_width + 6) as u16).min(area.width.saturating_sub(8));

    // Calculate available height for ops (total - header - footer - spacing - borders)
    let max_popup_height = area.height.saturating_sub(4);
    // Fixed lines: header(1) + spacing(1) + spacing(1) + footer(1) + borders(2) = 6
    // When scrolling: add 2 for scroll indicators (up/down)
    let base_fixed = 6; // header + 2 spacing + footer + 2 borders
    let scroll_indicator_lines = 2; // reserve for both up and down indicators
    let max_ops_visible =
        (max_popup_height as usize).saturating_sub(base_fixed + scroll_indicator_lines);

    let needs_scroll = op_strings.len() > max_ops_visible;
    let ops_to_show = if needs_scroll {
        max_ops_visible
    } else {
        op_strings.len()
    };

    // Calculate actual content lines used
    let scroll_lines_used = if needs_scroll { 2 } else { 0 };
    let content_lines = 4 + ops_to_show + scroll_lines_used; // header + 2 spacing + footer + ops + scroll
    let popup_height = (content_lines as u16 + 2).min(max_popup_height);

    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    // Build the content lines
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        header,
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Always reserve space for scroll indicators when scrolling is possible (prevents shift)
    if needs_scroll {
        if scroll > 0 {
            lines.push(Line::from(Span::styled(
                format!("  ↑ {} more above", scroll),
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from("")); // Reserve space
        }
    }

    // Show visible ops with scroll offset
    for (text, color) in op_strings.iter().skip(scroll).take(ops_to_show) {
        if text.is_empty() {
            // Empty line separator
            lines.push(Line::from(""));
        } else {
            let icon_end = text.find(' ').unwrap_or(text.len()).min(text.len());
            lines.push(Line::from(vec![
                Span::styled(&text[..icon_end], Style::default().fg(*color)),
                Span::raw(&text[icon_end..]),
            ]));
        }
    }

    // Always reserve space for scroll indicators when scrolling is possible (prevents shift)
    let remaining = op_strings.len().saturating_sub(scroll + ops_to_show);
    if needs_scroll {
        if remaining > 0 {
            lines.push(Line::from(Span::styled(
                format!("  ↓ {} more below (j/k to scroll)", remaining),
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from("")); // Reserve space
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("Apply changes? "),
        Span::styled(
            "y",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("/"),
        Span::styled(
            "n",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    ]));

    let block = Block::default()
        .title(" Sync Changes ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_quit_confirm(frame: &mut Frame, area: Rect, app: &App, scroll: usize) {
    let ops = app.pending_ops();

    // Get the base path for relative display
    let base_path = &app.cwd;

    // Collect implicit directory creations from rename/copy/create operations
    let mut implicit_dirs: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    for op in &ops {
        let to_path = match op {
            crate::core::FsOperation::Rename { to, from, .. } => {
                // Only if moving to different directory
                if from.parent() != to.parent() {
                    Some(to)
                } else {
                    None
                }
            }
            crate::core::FsOperation::Copy { to, .. } => Some(to),
            crate::core::FsOperation::Create { path, .. } => Some(path),
            _ => None,
        };
        if let Some(to) = to_path {
            if let Some(parent) = to.parent() {
                // Check if parent is not the base directory and doesn't exist
                if parent != base_path && !parent.exists() {
                    implicit_dirs.insert(parent.to_path_buf());
                }
            }
        }
    }

    // Build content strings first to calculate width
    let total_ops = ops.len() + implicit_dirs.len();
    let header = format!("You have {} unsaved change(s):", total_ops);
    let footer1 = "Discard and quit? y/n";
    let footer2 = "Save and quit? S";

    let mut op_strings: Vec<(String, Color)> = Vec::new();

    // Show implicit directory creations first
    for dir in &implicit_dirs {
        // Show path relative to base
        let rel_path = dir.strip_prefix(base_path).unwrap_or(dir);
        op_strings.push((
            format!("  + Create dir {}", rel_path.display()),
            Color::Green,
        ));
    }

    for op in &ops {
        let (icon, color, desc) = match op {
            crate::core::FsOperation::Create { path, is_dir } => {
                // Show path relative to base for nested creates
                let rel_path = path.strip_prefix(base_path).unwrap_or(path);
                let kind = if *is_dir { "dir" } else { "file" };
                (
                    "+",
                    Color::Green,
                    format!("Create {} {}", kind, rel_path.display()),
                )
            }
            crate::core::FsOperation::Delete { path } => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                ("-", Color::Red, format!("Delete {}", name))
            }
            crate::core::FsOperation::Rename { from, to } => {
                // Check if this is a move to a different directory
                let from_parent = from.parent();
                let to_parent = to.parent();
                let from_name = from.file_name().unwrap_or_default().to_string_lossy();
                let to_name = to.file_name().unwrap_or_default().to_string_lossy();

                if from_parent != to_parent {
                    // Moving to different directory - show the relative path
                    let to_rel = if let Some(parent) = to_parent {
                        if let Some(parent_name) = parent.file_name() {
                            format!("{}/{}", parent_name.to_string_lossy(), to_name)
                        } else {
                            to_name.to_string()
                        }
                    } else {
                        to_name.to_string()
                    };
                    ("→", Color::Blue, format!("{} → {}", from_name, to_rel))
                } else {
                    // Same directory rename
                    ("~", Color::Yellow, format!("{} → {}", from_name, to_name))
                }
            }
            crate::core::FsOperation::Copy { from, to, .. } => {
                let from_name = from.file_name().unwrap_or_default().to_string_lossy();
                let to_name = to.file_name().unwrap_or_default().to_string_lossy();
                ("c", Color::Cyan, format!("{} → {}", from_name, to_name))
            }
        };
        op_strings.push((format!("  {} {}", icon, desc), color));
    }

    // Calculate required width based on content
    let mut max_content_width = header.len();
    max_content_width = max_content_width.max(footer1.len());
    max_content_width = max_content_width.max(footer2.len());
    for (s, _) in &op_strings {
        max_content_width = max_content_width.max(s.len());
    }

    // Add padding for borders (2) and some internal margin (4)
    let popup_width = ((max_content_width + 6) as u16).min(area.width.saturating_sub(8));

    // Calculate available height for ops (total - header - footer - spacing - borders)
    let max_popup_height = area.height.saturating_sub(4);
    // Fixed lines: header(1) + spacing(1) + spacing(1) + 2 footers(2) + borders(2) = 7
    // When scrolling: add 2 for scroll indicators (up/down)
    let base_fixed = 7; // header + 2 spacing + 2 footers + 2 borders
    let scroll_indicator_lines = 2; // reserve for both up and down indicators
    let max_ops_visible =
        (max_popup_height as usize).saturating_sub(base_fixed + scroll_indicator_lines);

    let needs_scroll = op_strings.len() > max_ops_visible;
    let ops_to_show = if needs_scroll {
        max_ops_visible
    } else {
        op_strings.len()
    };

    // Calculate actual content lines used
    let scroll_lines_used = if needs_scroll { 2 } else { 0 };
    let content_lines = 5 + ops_to_show + scroll_lines_used; // header + 2 spacing + 2 footers + ops + scroll
    let popup_height = (content_lines as u16 + 2).min(max_popup_height);

    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    // Build the content lines
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        header,
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Always reserve space for scroll indicators when scrolling is possible (prevents shift)
    if needs_scroll {
        if scroll > 0 {
            lines.push(Line::from(Span::styled(
                format!("  ↑ {} more above", scroll),
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from("")); // Reserve space
        }
    }

    // Show visible ops with scroll offset
    for (text, color) in op_strings.iter().skip(scroll).take(ops_to_show) {
        if text.is_empty() {
            // Empty line separator
            lines.push(Line::from(""));
        } else {
            let icon_end = text.find(' ').unwrap_or(text.len()).min(text.len());
            lines.push(Line::from(vec![
                Span::styled(&text[..icon_end], Style::default().fg(*color)),
                Span::raw(&text[icon_end..]),
            ]));
        }
    }

    // Always reserve space for scroll indicators when scrolling is possible (prevents shift)
    let remaining = op_strings.len().saturating_sub(scroll + ops_to_show);
    if needs_scroll {
        if remaining > 0 {
            lines.push(Line::from(Span::styled(
                format!("  ↓ {} more below (j/k to scroll)", remaining),
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from("")); // Reserve space
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("Discard and quit? "),
        Span::styled(
            "y",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw("/"),
        Span::styled(
            "n",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw("Save and quit? "),
        Span::styled(
            "S",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    let block = Block::default()
        .title(" Unsaved Changes ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_confirm(frame: &mut Frame, area: Rect, action: &ConfirmAction) {
    let (title, message) = match action {
        ConfirmAction::EmptyTrash => (
            " Empty Trash ",
            "All items in trash will be permanently deleted.\n\nThis action cannot be undone.",
        ),
        _ => (" Confirm ", "Are you sure?"),
    };

    // Calculate popup dimensions
    let popup_width = 48.min(area.width.saturating_sub(4));
    let popup_height = 9.min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let lines = vec![
        Line::from(""),
        Line::from(message.lines().next().unwrap_or("")),
        Line::from(""),
        Line::from(message.lines().nth(2).unwrap_or("")),
        Line::from(""),
        Line::from(vec![
            Span::raw("Confirm? "),
            Span::styled(
                "y",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw("/"),
            Span::styled(
                "n",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup_area);
}
