use once_cell::sync::Lazy;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::RwLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

/// Global syntax set - uses two-face for extensive language support (same as bat)
static SYNTAX_SET: Lazy<SyntaxSet> = Lazy::new(two_face::syntax::extra_newlines);

/// All available themes
static THEMES: Lazy<BTreeMap<String, Theme>> = Lazy::new(|| {
    let mut themes = BTreeMap::new();

    // Load built-in themes
    let defaults = ThemeSet::load_defaults();
    for (name, theme) in defaults.themes {
        themes.insert(name, theme);
    }

    themes
});

/// Currently selected theme name
static CURRENT_THEME: Lazy<RwLock<String>> =
    Lazy::new(|| RwLock::new("base16-ocean.dark".to_string()));

/// Get list of available theme names
pub fn available_themes() -> Vec<String> {
    THEMES.keys().cloned().collect()
}

/// Get current theme name
pub fn current_theme() -> String {
    CURRENT_THEME.read().unwrap().clone()
}

/// Set current theme by name
pub fn set_theme(name: &str) {
    if THEMES.contains_key(name) {
        *CURRENT_THEME.write().unwrap() = name.to_string();
    }
}

/// Get the current theme
fn get_theme() -> &'static Theme {
    let name = CURRENT_THEME.read().unwrap();
    THEMES
        .get(name.as_str())
        .unwrap_or_else(|| THEMES.get("base16-ocean.dark").unwrap())
}

/// A highlighted line containing styled spans
#[derive(Debug, Clone)]
pub struct HighlightedLine {
    pub spans: Vec<(String, Style)>,
}

impl HighlightedLine {
    /// Convert to a ratatui Line without truncation (for wrapping)
    #[allow(dead_code)]
    pub fn to_line_full(&self) -> Line<'static> {
        let spans: Vec<Span> = self
            .spans
            .iter()
            .map(|(text, style)| Span::styled(text.clone(), *style))
            .collect();
        Line::from(spans)
    }

    /// Convert to a ratatui Line, truncating to fit width
    pub fn to_line(&self, max_width: usize) -> Line<'static> {
        let mut result_spans = Vec::new();
        let mut current_width = 0;

        for (text, style) in &self.spans {
            if current_width >= max_width {
                break;
            }

            let remaining = max_width - current_width;
            let char_count = text.chars().count();

            if char_count <= remaining {
                result_spans.push(Span::styled(text.clone(), *style));
                current_width += char_count;
            } else {
                // Truncate this span
                let truncated: String = text.chars().take(remaining.saturating_sub(1)).collect();
                result_spans.push(Span::styled(truncated, *style));
                result_spans.push(Span::styled("…".to_string(), Style::default()));
                break;
            }
        }

        Line::from(result_spans)
    }

    /// Convert to a ratatui Line with line number prefix
    pub fn to_line_numbered(
        &self,
        line_num: usize,
        max_width: usize,
        num_width: usize,
    ) -> Line<'static> {
        let mut result_spans = Vec::new();

        // Add line number with distinct color
        let num_str = format!("{:>width$} ", line_num, width = num_width);
        result_spans.push(Span::styled(
            num_str.clone(),
            Style::default().fg(Color::DarkGray),
        ));

        let mut current_width = num_str.len();

        for (text, style) in &self.spans {
            if current_width >= max_width {
                break;
            }

            let remaining = max_width - current_width;
            let char_count = text.chars().count();

            if char_count <= remaining {
                result_spans.push(Span::styled(text.clone(), *style));
                current_width += char_count;
            } else {
                let truncated: String = text.chars().take(remaining.saturating_sub(1)).collect();
                result_spans.push(Span::styled(truncated, *style));
                result_spans.push(Span::styled("…".to_string(), Style::default()));
                break;
            }
        }

        Line::from(result_spans)
    }
}

/// Syntax highlighter that caches the parsed syntax
pub struct Highlighter {
    syntax_name: Option<String>,
}

impl Highlighter {
    /// Create a highlighter for a given file path
    pub fn for_path(path: &Path) -> Self {
        let ext = path.extension().and_then(|e| e.to_str());

        let syntax = ext
            .and_then(|ext| SYNTAX_SET.find_syntax_by_extension(ext))
            .or_else(|| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| SYNTAX_SET.find_syntax_by_extension(name))
            });

        let syntax_name = syntax.map(|s| s.name.clone());

        Self { syntax_name }
    }

    /// Check if highlighting is available for this file
    #[allow(dead_code)]
    pub fn is_available(&self) -> bool {
        self.syntax_name.is_some()
    }

    /// Highlight lines of text
    pub fn highlight_lines(&self, lines: &[String]) -> Vec<HighlightedLine> {
        let Some(ref syntax_name) = self.syntax_name else {
            // No syntax - return plain text
            return lines
                .iter()
                .map(|line| HighlightedLine {
                    spans: vec![(line.clone(), Style::default())],
                })
                .collect();
        };

        let Some(syntax) = SYNTAX_SET.find_syntax_by_name(syntax_name) else {
            return lines
                .iter()
                .map(|line| HighlightedLine {
                    spans: vec![(line.clone(), Style::default())],
                })
                .collect();
        };

        let mut highlighter = HighlightLines::new(syntax, get_theme());

        lines
            .iter()
            .map(|line| {
                match highlighter.highlight_line(line, &SYNTAX_SET) {
                    Ok(ranges) => {
                        let spans: Vec<(String, Style)> = ranges
                            .into_iter()
                            .map(|(style, text)| {
                                let ratatui_style = syntect_to_ratatui_style(&style);
                                // Strip trailing newline from display text
                                let display_text = text.trim_end_matches('\n').to_string();
                                (display_text, ratatui_style)
                            })
                            .collect();
                        HighlightedLine { spans }
                    }
                    Err(_) => HighlightedLine {
                        spans: vec![(line.trim_end_matches('\n').to_string(), Style::default())],
                    },
                }
            })
            .collect()
    }

    /// Highlight a single line of text
    pub fn highlight_single_line(&self, line: &str) -> HighlightedLine {
        let Some(ref syntax_name) = self.syntax_name else {
            return HighlightedLine {
                spans: vec![(line.to_string(), Style::default())],
            };
        };

        let Some(syntax) = SYNTAX_SET.find_syntax_by_name(syntax_name) else {
            return HighlightedLine {
                spans: vec![(line.to_string(), Style::default())],
            };
        };

        let mut highlighter = HighlightLines::new(syntax, get_theme());
        let line_with_newline = format!("{}\n", line);

        match highlighter.highlight_line(&line_with_newline, &SYNTAX_SET) {
            Ok(ranges) => {
                let spans: Vec<(String, Style)> = ranges
                    .into_iter()
                    .map(|(style, text)| {
                        let ratatui_style = syntect_to_ratatui_style(&style);
                        let display_text = text.trim_end_matches('\n').to_string();
                        (display_text, ratatui_style)
                    })
                    .collect();
                HighlightedLine { spans }
            }
            Err(_) => HighlightedLine {
                spans: vec![(line.to_string(), Style::default())],
            },
        }
    }
}

/// Convert syntect style to ratatui style
fn syntect_to_ratatui_style(style: &syntect::highlighting::Style) -> Style {
    let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);

    let mut ratatui_style = Style::default().fg(fg);

    if style.font_style.contains(FontStyle::BOLD) {
        ratatui_style = ratatui_style.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        ratatui_style = ratatui_style.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        ratatui_style = ratatui_style.add_modifier(Modifier::UNDERLINED);
    }

    ratatui_style
}

/// Get a palette of colors from the current theme for icon mapping
pub fn get_theme_palette() -> Vec<(u8, u8, u8)> {
    let theme = get_theme();
    let mut colors = Vec::new();

    // Add foreground color
    if let Some(fg) = theme.settings.foreground {
        colors.push((fg.r, fg.g, fg.b));
    }

    // Extract colors from theme scopes
    for item in &theme.scopes {
        if let Some(fg) = item.style.foreground {
            let rgb = (fg.r, fg.g, fg.b);
            // Avoid duplicates
            if !colors.contains(&rgb) {
                colors.push(rgb);
            }
        }
    }

    // Ensure we have at least some colors
    if colors.is_empty() {
        colors.push((200, 200, 200)); // Default gray
    }

    colors
}

/// Map an RGB color to the nearest color in the theme palette
pub fn map_to_theme_color(r: u8, g: u8, b: u8) -> Color {
    let palette = get_theme_palette();

    // Find nearest color using simple Euclidean distance
    let mut best_match = palette[0];
    let mut best_distance = u32::MAX;

    for &(pr, pg, pb) in &palette {
        let dr = (r as i32 - pr as i32).unsigned_abs();
        let dg = (g as i32 - pg as i32).unsigned_abs();
        let db = (b as i32 - pb as i32).unsigned_abs();
        let distance = dr * dr + dg * dg + db * db;

        if distance < best_distance {
            best_distance = distance;
            best_match = (pr, pg, pb);
        }
    }

    Color::Rgb(best_match.0, best_match.1, best_match.2)
}
