use ignore::WalkBuilder;
use once_cell::sync::Lazy;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufReader, Cursor};
use std::path::Path;
use std::sync::RwLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use two_face::theme::EmbeddedThemeName;

use crate::paths::themes_dir;

/// Global syntax set - uses two-face for extensive language support (same as bat)
static SYNTAX_SET: Lazy<SyntaxSet> = Lazy::new(two_face::syntax::extra_newlines);

const NO_CLOWN_FIESTA_THEME_NAME: &str = "noclownfiesta";
const NO_CLOWN_FIESTA_THEME: &[u8] = include_bytes!("noclownfiesta.tmTheme");

pub const DEFAULT_THEME: &str = NO_CLOWN_FIESTA_THEME_NAME;

/// Popular themes already shipped by two-face. Keeping this list curated avoids
/// filling the picker with terminal-specific and deprecated variants.
const POPULAR_THEMES: &[EmbeddedThemeName] = &[
    EmbeddedThemeName::Dracula,
    EmbeddedThemeName::Github,
    EmbeddedThemeName::GruvboxDark,
    EmbeddedThemeName::GruvboxLight,
    EmbeddedThemeName::MonokaiExtended,
    EmbeddedThemeName::Nord,
    EmbeddedThemeName::OneHalfDark,
    EmbeddedThemeName::OneHalfLight,
];

struct ThemeCatalog {
    themes: BTreeMap<String, Theme>,
    warnings: Vec<String>,
}

/// Build the catalog once per process. Personal themes are loaded at startup,
/// after bundled themes, so a same-named personal file intentionally wins.
static THEME_CATALOG: Lazy<ThemeCatalog> =
    Lazy::new(|| build_theme_catalog(themes_dir().as_deref()));

fn build_theme_catalog(personal_theme_dir: Option<&Path>) -> ThemeCatalog {
    let mut themes = BTreeMap::new();
    let mut warnings = Vec::new();

    // Syntect's defaults include Solarized, InspiredGitHub and Base16 themes.
    let defaults = ThemeSet::load_defaults();
    for (name, theme) in defaults.themes {
        themes.insert(name, theme);
    }

    // Add a conservative selection from two-face's curated theme collection.
    let popular = two_face::theme::extra();
    for name in POPULAR_THEMES {
        themes.insert(name.as_name().to_string(), popular.get(*name).clone());
    }

    let mut reader = BufReader::new(Cursor::new(NO_CLOWN_FIESTA_THEME));
    let no_clown_fiesta =
        ThemeSet::load_from_reader(&mut reader).expect("bundled No Clown Fiesta theme must parse");
    themes.insert(NO_CLOWN_FIESTA_THEME_NAME.to_string(), no_clown_fiesta);

    if let Some(directory) = personal_theme_dir {
        load_personal_themes(directory, &mut themes, &mut warnings);
    }

    ThemeCatalog { themes, warnings }
}

fn load_personal_themes(
    directory: &Path,
    themes: &mut BTreeMap<String, Theme>,
    warnings: &mut Vec<String>,
) {
    match fs::metadata(directory) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            warnings.push(format!(
                "theme path is not a directory: {}",
                directory.display()
            ));
            return;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            warnings.push(format!(
                "could not inspect theme directory {}: {}",
                directory.display(),
                error
            ));
            return;
        }
    }

    let mut builder = WalkBuilder::new(directory);
    builder
        .follow_links(false)
        .hidden(false)
        .parents(false)
        .ignore(false)
        .git_global(false)
        .git_ignore(false)
        .git_exclude(false);

    let mut paths = Vec::new();
    for result in builder.build() {
        match result {
            Ok(entry)
                if entry
                    .file_type()
                    .is_some_and(|file_type| file_type.is_file())
                    && entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("tmTheme")) =>
            {
                paths.push(entry.into_path());
            }
            Ok(_) => {}
            Err(error) => warnings.push(format!(
                "could not inspect a theme path in {}: {}",
                directory.display(),
                error
            )),
        }
    }
    paths.sort();

    for path in paths {
        let Some(name) = theme_name_from_path(&path) else {
            warnings.push(format!("theme has an invalid filename: {}", path.display()));
            continue;
        };
        match ThemeSet::get_theme(&path) {
            Ok(theme) => {
                themes.insert(name, theme);
            }
            Err(error) => warnings.push(format!(
                "could not load theme {}: {}",
                path.display(),
                error
            )),
        }
    }
}

fn theme_name_from_path(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && !name.chars().any(char::is_control))
        .map(str::to_string)
}

/// Currently selected theme name
static CURRENT_THEME: Lazy<RwLock<String>> = Lazy::new(|| RwLock::new(DEFAULT_THEME.to_string()));

/// Get list of available theme names
pub fn available_themes() -> Vec<String> {
    THEME_CATALOG.themes.keys().cloned().collect()
}

/// Non-fatal failures encountered while loading personal theme files.
pub fn theme_load_warnings() -> Vec<String> {
    THEME_CATALOG.warnings.clone()
}

/// Get current theme name
pub fn current_theme() -> String {
    CURRENT_THEME.read().unwrap().clone()
}

/// Set current theme by name. Returns false when the name is not available.
pub fn set_theme(name: &str) -> bool {
    if THEME_CATALOG.themes.contains_key(name) {
        *CURRENT_THEME.write().unwrap() = name.to_string();
        true
    } else {
        false
    }
}

/// Get the current theme
fn get_theme() -> &'static Theme {
    let name = CURRENT_THEME.read().unwrap();
    THEME_CATALOG
        .themes
        .get(name.as_str())
        .or_else(|| THEME_CATALOG.themes.get(DEFAULT_THEME))
        .or_else(|| THEME_CATALOG.themes.values().next())
        .expect("Peak File Manager must have at least one syntax theme")
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const TEST_THEME: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>name</key>
  <string>Peak Test</string>
  <key>settings</key>
  <array>
    <dict>
      <key>settings</key>
      <dict>
        <key>background</key>
        <string>#000000</string>
        <key>foreground</key>
        <string>#010203</string>
      </dict>
    </dict>
  </array>
</dict>
</plist>
"##;

    #[test]
    fn catalog_includes_bundled_themes() {
        let catalog = build_theme_catalog(None);
        for name in [
            "Dracula",
            "GitHub",
            "gruvbox-dark",
            "gruvbox-light",
            "Monokai Extended",
            "Nord",
            "OneHalfDark",
            "OneHalfLight",
            NO_CLOWN_FIESTA_THEME_NAME,
        ] {
            assert!(catalog.themes.contains_key(name), "missing {name}");
        }

        let no_clown_fiesta = catalog.themes.get(NO_CLOWN_FIESTA_THEME_NAME).unwrap();
        assert_eq!(DEFAULT_THEME, NO_CLOWN_FIESTA_THEME_NAME);
        assert_eq!(no_clown_fiesta.name.as_deref(), Some("No Clown Fiesta"));

        let foreground = no_clown_fiesta.settings.foreground.unwrap();
        let background = no_clown_fiesta.settings.background.unwrap();
        assert_eq!((foreground.r, foreground.g, foreground.b), (209, 209, 209));
        assert_eq!((background.r, background.g, background.b), (18, 18, 18));
    }

    #[test]
    fn personal_themes_load_independently_and_override_bundled_names() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(
            dir.path()
                .join(format!("{NO_CLOWN_FIESTA_THEME_NAME}.tmTheme")),
            TEST_THEME,
        )
        .unwrap();
        fs::write(nested.join("personal.tmTheme"), TEST_THEME).unwrap();
        fs::write(dir.path().join("broken.tmTheme"), "not a plist").unwrap();
        fs::write(dir.path().join("ignored.txt"), TEST_THEME).unwrap();

        let catalog = build_theme_catalog(Some(dir.path()));

        let no_clown_fiesta = catalog.themes.get(NO_CLOWN_FIESTA_THEME_NAME).unwrap();
        let foreground = no_clown_fiesta.settings.foreground.unwrap();
        assert_eq!((foreground.r, foreground.g, foreground.b), (1, 2, 3));
        assert!(catalog.themes.contains_key("personal"));
        assert!(!catalog.themes.contains_key("ignored"));
        assert_eq!(catalog.warnings.len(), 1);
        assert!(catalog.warnings[0].contains("broken.tmTheme"));
    }

    #[test]
    fn invalid_personal_override_keeps_bundled_theme() {
        let dir = TempDir::new().unwrap();
        let override_path = dir
            .path()
            .join(format!("{NO_CLOWN_FIESTA_THEME_NAME}.tmTheme"));
        fs::write(&override_path, "not a plist").unwrap();

        let catalog = build_theme_catalog(Some(dir.path()));
        let no_clown_fiesta = catalog.themes.get(NO_CLOWN_FIESTA_THEME_NAME).unwrap();
        let foreground = no_clown_fiesta.settings.foreground.unwrap();

        assert_eq!((foreground.r, foreground.g, foreground.b), (209, 209, 209));
        assert_eq!(catalog.warnings.len(), 1);
        assert!(catalog.warnings[0].contains("noclownfiesta.tmTheme"));
    }

    #[test]
    fn theme_names_reject_settings_control_characters() {
        assert_eq!(
            theme_name_from_path(Path::new("my-theme.tmTheme")).as_deref(),
            Some("my-theme")
        );
        assert!(theme_name_from_path(Path::new("bad\nshow_hidden=false.tmTheme")).is_none());
        assert!(theme_name_from_path(Path::new("bad\rtheme=Dracula.tmTheme")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn personal_theme_walk_does_not_follow_nested_symlinks() {
        use std::os::unix::fs::symlink;

        let theme_dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("outside.tmTheme"), TEST_THEME).unwrap();
        symlink(outside.path(), theme_dir.path().join("linked")).unwrap();

        let catalog = build_theme_catalog(Some(theme_dir.path()));

        assert!(!catalog.themes.contains_key("outside"));
        assert!(catalog.warnings.is_empty());
    }
}
