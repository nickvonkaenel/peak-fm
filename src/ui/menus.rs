//! Menu rendering - sort, info display, leader, preview options

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::App;
use crate::core::{DisplayInfo, SortOption};

pub(super) fn render_sort_menu(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    selected_idx: usize,
    is_global: bool,
) {
    // Center the popup
    let popup_width = 32.min(area.width.saturating_sub(4));
    let popup_height = 14.min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    // Helper to format a sort option line
    fn sort_line(
        key: &str,
        label: &str,
        current: SortOption,
        option: SortOption,
        is_highlighted: bool,
    ) -> Line<'static> {
        let is_active = current == option;
        let indicator = if is_active { " ●" } else { "" };
        let prefix = if is_highlighted { ">" } else { " " };

        let style = if is_highlighted {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Line::from(vec![
            Span::styled(format!(" {} ", prefix), style),
            Span::styled(format!("{}  ", key), style.fg(Color::Yellow)),
            Span::styled(format!("{:<18}{}", label, indicator), style),
        ])
    }

    let current = if is_global {
        app.sort_option
    } else {
        app.get_sort_for_dir(&app.cwd)
    };
    let sort_text = vec![
        Line::from(""),
        sort_line(
            "a",
            "Name (A-Z)",
            current,
            SortOption::Name,
            selected_idx == 0,
        ),
        sort_line(
            "A",
            "Name (Z-A)",
            current,
            SortOption::NameDesc,
            selected_idx == 1,
        ),
        Line::from(""),
        sort_line(
            "d",
            "Date (Newest)",
            current,
            SortOption::DateModified,
            selected_idx == 2,
        ),
        sort_line(
            "D",
            "Date (Oldest)",
            current,
            SortOption::DateModifiedAsc,
            selected_idx == 3,
        ),
        Line::from(""),
        sort_line(
            "s",
            "Size (Largest)",
            current,
            SortOption::Size,
            selected_idx == 4,
        ),
        sort_line(
            "S",
            "Size (Smallest)",
            current,
            SortOption::SizeAsc,
            selected_idx == 5,
        ),
        Line::from(""),
        sort_line(
            "e",
            "Extension (A-Z)",
            current,
            SortOption::Extension,
            selected_idx == 6,
        ),
        sort_line(
            "E",
            "Extension (Z-A)",
            current,
            SortOption::ExtensionDesc,
            selected_idx == 7,
        ),
    ];

    let title = if is_global {
        " Sort (Global) "
    } else {
        " Sort (Directory) "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(sort_text).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_info_select_menu(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    selected_idx: usize,
) {
    // Center the popup
    let popup_width = 32.min(area.width.saturating_sub(4));
    let popup_height = 12.min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    // Helper to format an info display option line
    fn info_line(
        key: &str,
        label: &str,
        current: DisplayInfo,
        option: DisplayInfo,
        is_highlighted: bool,
    ) -> Line<'static> {
        let is_active = current == option;
        let indicator = if is_active { " ●" } else { "" };
        let prefix = if is_highlighted { ">" } else { " " };

        let style = if is_highlighted {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Line::from(vec![
            Span::styled(format!(" {} ", prefix), style),
            Span::styled(format!("{}  ", key), style.fg(Color::Yellow)),
            Span::styled(format!("{:<18}{}", label, indicator), style),
        ])
    }

    let current = app.display_info;
    let info_text = vec![
        Line::from(""),
        info_line("n", "None", current, DisplayInfo::None, selected_idx == 0),
        Line::from(""),
        info_line(
            "d",
            "Date Modified",
            current,
            DisplayInfo::DateModified,
            selected_idx == 1,
        ),
        Line::from(""),
        info_line("s", "Size", current, DisplayInfo::Size, selected_idx == 2),
        Line::from(""),
        info_line("m", "Mode", current, DisplayInfo::Mode, selected_idx == 3),
        Line::from(""),
        info_line(
            "e",
            "Extension",
            current,
            DisplayInfo::Extension,
            selected_idx == 4,
        ),
    ];

    let block = Block::default()
        .title(" Info Display ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(info_text).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_leader_menu(frame: &mut Frame, area: Rect) {
    // Center the popup
    let popup_width = 28.min(area.width.saturating_sub(4));
    let popup_height = 18.min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let leader_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  .  ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle Hidden"),
        ]),
        Line::from(vec![
            Span::styled("  a  ", Style::default().fg(Color::Yellow)),
            Span::raw("Audio Browser"),
        ]),
        Line::from(vec![
            Span::styled("  c  ", Style::default().fg(Color::Yellow)),
            Span::raw("Themes"),
        ]),
        Line::from(vec![
            Span::styled("  f  ", Style::default().fg(Color::Yellow)),
            Span::raw("FFmpeg Editor"),
        ]),
        Line::from(vec![
            Span::styled("  g  ", Style::default().fg(Color::Yellow)),
            Span::raw("Git"),
        ]),
        Line::from(vec![
            Span::styled("  i  ", Style::default().fg(Color::Yellow)),
            Span::raw("Info Display"),
        ]),
        Line::from(vec![
            Span::styled("  l  ", Style::default().fg(Color::Yellow)),
            Span::raw("Lazygit"),
        ]),
        Line::from(vec![
            Span::styled("  p  ", Style::default().fg(Color::Yellow)),
            Span::raw("Commit all"),
        ]),
        Line::from(vec![
            Span::styled("  q  ", Style::default().fg(Color::Yellow)),
            Span::raw("Quit"),
        ]),
        Line::from(vec![
            Span::styled("  s  ", Style::default().fg(Color::Yellow)),
            Span::raw("Sort (Directory)"),
        ]),
        Line::from(vec![
            Span::styled("  S  ", Style::default().fg(Color::Yellow)),
            Span::raw("Sort (Global)"),
        ]),
        Line::from(vec![
            Span::styled("  t  ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle Trash"),
        ]),
        Line::from(vec![
            Span::styled("  T  ", Style::default().fg(Color::Yellow)),
            Span::raw("Empty Trash"),
        ]),
        Line::from(vec![
            Span::styled("  u  ", Style::default().fg(Color::Yellow)),
            Span::raw("Display Options"),
        ]),
        Line::from(""),
    ];

    let block = Block::default()
        .title(" Leader ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let paragraph = Paragraph::new(leader_text).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_preview_options_menu(frame: &mut Frame, area: Rect, app: &App) {
    // Center the popup
    let popup_width = 30.min(area.width.saturating_sub(4));
    let popup_height = 11.min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let wrap_indicator = if app.wrap_preview { " ●" } else { "" };
    let line_numbers_indicator = if app.line_numbers { " ●" } else { "" };
    let icons_indicator = if app.show_icons { " ●" } else { "" };
    let icon_colors_indicator = if app.colored_icons { " ●" } else { "" };
    let theme_icons_indicator = if app.theme_icons { " ●" } else { "" };

    let preview_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  w  ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("Toggle Wrap{}", wrap_indicator)),
        ]),
        Line::from(vec![
            Span::styled("  l  ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("Toggle Line Numbers{}", line_numbers_indicator)),
        ]),
        Line::from(vec![
            Span::styled("  i  ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("Toggle Icons{}", icons_indicator)),
        ]),
        Line::from(vec![
            Span::styled("  c  ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("Toggle Icon Colors{}", icon_colors_indicator)),
        ]),
        Line::from(vec![
            Span::styled("  t  ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("Toggle Theme Colors{}", theme_icons_indicator)),
        ]),
        Line::from(""),
    ];

    let block = Block::default()
        .title(" Display Options ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let paragraph = Paragraph::new(preview_text).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_git_menu(frame: &mut Frame, area: Rect) {
    // Center the popup
    let popup_width = 26.min(area.width.saturating_sub(4));
    let popup_height = 10.min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let git_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  s  ", Style::default().fg(Color::Yellow)),
            Span::raw("Status"),
        ]),
        Line::from(vec![
            Span::styled("  g  ", Style::default().fg(Color::Yellow)),
            Span::raw("Pull"),
        ]),
        Line::from(vec![
            Span::styled("  p  ", Style::default().fg(Color::Yellow)),
            Span::raw("Push"),
        ]),
        Line::from(vec![
            Span::styled("  c  ", Style::default().fg(Color::Yellow)),
            Span::raw("Commit staged"),
        ]),
        Line::from(vec![
            Span::styled("  a  ", Style::default().fg(Color::Yellow)),
            Span::raw("Commit all"),
        ]),
        Line::from(""),
    ];

    let block = Block::default()
        .title(" Git ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let paragraph = Paragraph::new(git_text).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_git_status(frame: &mut Frame, area: Rect, lines: &[String], scroll: usize) {
    // Calculate popup size
    let popup_width = 70.min(area.width.saturating_sub(4));
    let max_height = area.height.saturating_sub(4);
    let content_height = (lines.len() + 4) as u16; // +4 for padding and hints
    let popup_height = content_height.min(max_height).max(6);
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let visible_lines = (popup_height as usize).saturating_sub(4); // borders + padding
    let mut status_text = vec![Line::from("")];

    if lines.is_empty() {
        status_text.push(Line::from(Span::styled(
            "  Nothing to commit, working tree clean",
            Style::default().fg(Color::Green),
        )));
    } else {
        for line in lines.iter().skip(scroll).take(visible_lines) {
            let (icon, color) = if line.starts_with("M") || line.starts_with(" M") {
                ("~", Color::Yellow) // Modified
            } else if line.starts_with("A") || line.starts_with("??") {
                ("+", Color::Green) // Added/New
            } else if line.starts_with("D") || line.starts_with(" D") {
                ("-", Color::Red) // Deleted
            } else if line.starts_with("R") {
                ("→", Color::Blue) // Renamed
            } else {
                ("•", Color::Gray) // Other
            };

            // Extract filename (after status prefix)
            let filename = line.split_whitespace().last().unwrap_or(line);
            status_text.push(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                Span::raw(filename),
            ]));
        }

        // Show scroll indicator if needed
        let remaining = lines.len().saturating_sub(scroll + visible_lines);
        if remaining > 0 {
            status_text.push(Line::from(Span::styled(
                format!("  ↓ {} more (j/k to scroll)", remaining),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    status_text.push(Line::from(""));

    let block = Block::default()
        .title(format!(" Git Status ({} files) ", lines.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let paragraph = Paragraph::new(status_text).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_git_commit(
    frame: &mut Frame,
    area: Rect,
    message: &str,
    all: bool,
    auto_push: bool,
    status: &[String],
) {
    // Make popup wider - 80% of screen width, min 60, max 100
    let popup_width = ((area.width as f32 * 0.8) as u16)
        .clamp(60, 100)
        .min(area.width.saturating_sub(4));

    // Calculate how many lines the message will need (accounting for "> " prefix)
    let inner_width = popup_width.saturating_sub(4) as usize; // borders + padding
    let message_lines = if message.is_empty() {
        1
    } else {
        (message.len() + 2).div_ceil(inner_width).max(1) // +2 for "> " prefix
    };

    // Calculate height based on status lines and message lines
    let status_lines = status.len().clamp(1, 10); // At least 1 for "no changes" message
    let base_height = 7; // borders(2) + empty + empty + empty + hints + padding
    let popup_height =
        ((base_height + status_lines + message_lines) as u16).min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    // Wrap message text across multiple lines
    let mut commit_text = vec![Line::from("")];

    if message.is_empty() {
        commit_text.push(Line::from("> "));
    } else {
        let prefixed_message = format!("> {}", message);
        let mut remaining = prefixed_message.as_str();
        while !remaining.is_empty() {
            let (line, rest) = if remaining.len() <= inner_width {
                (remaining, "")
            } else {
                remaining.split_at(inner_width)
            };
            commit_text.push(Line::from(line.to_string()));
            remaining = rest;
        }
    }

    commit_text.push(Line::from(""));

    // Add status lines with color coding
    if status.is_empty() {
        commit_text.push(Line::from(Span::styled(
            "  No changes to commit",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for line in status.iter().take(10) {
            let (icon, color) = if line.starts_with("M") || line.starts_with(" M") {
                ("~", Color::Yellow) // Modified
            } else if line.starts_with("A") || line.starts_with("??") {
                ("+", Color::Green) // Added/New
            } else if line.starts_with("D") || line.starts_with(" D") {
                ("-", Color::Red) // Deleted
            } else if line.starts_with("R") {
                ("→", Color::Blue) // Renamed
            } else {
                ("•", Color::Gray) // Other
            };

            // Extract filename (after status prefix)
            let filename = line.split_whitespace().last().unwrap_or(line);
            commit_text.push(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                Span::raw(filename),
            ]));
        }
        if status.len() > 10 {
            commit_text.push(Line::from(Span::styled(
                format!("  ... and {} more", status.len() - 10),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    commit_text.push(Line::from(""));

    // Show hints with auto_push state
    let enter_hint = if auto_push {
        " commit & push  "
    } else {
        " commit  "
    };
    let push_state = if auto_push { "[on]" } else { "[off]" };
    let push_state_color = if auto_push {
        Color::Green
    } else {
        Color::DarkGray
    };

    commit_text.push(Line::from(vec![
        Span::styled("Enter:", Style::default().fg(Color::DarkGray)),
        Span::styled(enter_hint, Style::default().fg(Color::DarkGray)),
        Span::styled("Ctrl+p:", Style::default().fg(Color::DarkGray)),
        Span::styled(" auto-push ", Style::default().fg(Color::DarkGray)),
        Span::styled(push_state, Style::default().fg(push_state_color)),
    ]));

    let title = if all {
        " Commit All "
    } else {
        " Commit Staged "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let paragraph = Paragraph::new(commit_text).block(block);
    frame.render_widget(paragraph, popup_area);

    // Set cursor position (accounting for wrapped text)
    use crossterm::cursor::SetCursorStyle;
    use crossterm::execute;
    use std::io::stdout;
    let _ = execute!(stdout(), SetCursorStyle::BlinkingBar);

    // Calculate cursor position based on message length and wrapping
    let total_len = message.len() + 2; // +2 for "> " prefix
    let cursor_line = total_len / inner_width;
    let cursor_col = total_len % inner_width;
    let cursor_x = popup_area.x + 1 + cursor_col as u16;
    let cursor_y = popup_area.y + 2 + cursor_line as u16;
    frame.set_cursor_position((cursor_x, cursor_y));
}
