use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::core::SortOption;
use crate::paths::{APP_DIR_NAME, LEGACY_APP_DIR_NAME, PRODUCT_NAME};

/// Bumped when the on-disk settings format changes in a way that needs
/// migration. Files without a `version=` line are treated as legacy (v0) and
/// still parse, since every setting is keyed by name.
const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct Config {
    pub show_hidden: bool,
    pub wrap_preview: bool,
    pub line_numbers: bool,
    pub show_icons: bool,
    pub colored_icons: bool,
    pub theme_icons: bool, // When true, map icon colors to theme palette
    pub theme: String,
    /// When true, opening a file from search/grep navigates to the file's directory.
    /// When false (legacy), returns to search/grep mode after closing the file.
    pub search_navigate_on_open: bool,
    pub sort_option: SortOption,
    pub dir_sort_cache: HashMap<PathBuf, SortOption>,
    pub git_auto_push: bool,
    // Audio mode settings
    pub audio_autoplay: bool,
    pub audio_normalize: bool,
    pub audio_skip_silence: bool,
    pub audio_volume: f32,
    pub audio_analyzer_gradient: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_hidden: true,
            wrap_preview: false,
            line_numbers: false,
            show_icons: true,
            colored_icons: true,
            theme_icons: true,
            theme: "base16-ocean.dark".to_string(),
            search_navigate_on_open: true,
            sort_option: SortOption::Name,
            dir_sort_cache: HashMap::new(),
            git_auto_push: false,
            audio_autoplay: true,
            audio_normalize: false,
            audio_skip_silence: false,
            audio_volume: 1.0,
            audio_analyzer_gradient: false,
        }
    }
}

/// Parse a sort option from its on-disk token.
fn sort_from_str(value: &str) -> Option<SortOption> {
    Some(match value {
        "name" => SortOption::Name,
        "name_desc" => SortOption::NameDesc,
        "date" => SortOption::DateModified,
        "date_asc" => SortOption::DateModifiedAsc,
        "size" => SortOption::Size,
        "size_asc" => SortOption::SizeAsc,
        "ext" => SortOption::Extension,
        "ext_desc" => SortOption::ExtensionDesc,
        _ => return None,
    })
}

/// Serialize a sort option to its on-disk token.
fn sort_to_str(sort: SortOption) -> &'static str {
    match sort {
        SortOption::Name => "name",
        SortOption::NameDesc => "name_desc",
        SortOption::DateModified => "date",
        SortOption::DateModifiedAsc => "date_asc",
        SortOption::Size => "size",
        SortOption::SizeAsc => "size_asc",
        SortOption::Extension => "ext",
        SortOption::ExtensionDesc => "ext_desc",
    }
}

impl Config {
    fn config_path() -> Option<PathBuf> {
        #[cfg(windows)]
        {
            dirs::data_local_dir().map(|p| p.join(APP_DIR_NAME).join("settings"))
        }
        #[cfg(not(windows))]
        {
            dirs::home_dir().map(|p| p.join(".config").join(APP_DIR_NAME).join("settings"))
        }
    }

    fn legacy_config_path() -> Option<PathBuf> {
        #[cfg(windows)]
        {
            dirs::data_local_dir().map(|p| p.join(LEGACY_APP_DIR_NAME).join("settings"))
        }
        #[cfg(not(windows))]
        {
            dirs::home_dir().map(|p| p.join(".config").join(LEGACY_APP_DIR_NAME).join("settings"))
        }
    }

    /// Load the config, discarding any parse warnings. Used by the many
    /// read-modify-save call sites that only touch a single field.
    pub fn load() -> Self {
        Self::load_with_warnings().0
    }

    /// Load the config and return any non-fatal parse warnings (unknown keys,
    /// malformed values, a newer on-disk version). A missing or unreadable
    /// file is not a warning — it just yields defaults.
    pub fn load_with_warnings() -> (Self, Vec<String>) {
        let Some(path) = Self::config_path() else {
            return (Self::default(), Vec::new());
        };

        let contents = fs::read_to_string(&path).or_else(|_| {
            Self::legacy_config_path()
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
                .and_then(fs::read_to_string)
        });
        let Ok(contents) = contents else {
            return (Self::default(), Vec::new());
        };

        Self::parse(&contents)
    }

    /// Parse settings from the file contents. Pure (no I/O) so it can be
    /// tested directly. Unknown keys and malformed values are reported as
    /// warnings rather than silently dropped; the affected setting keeps its
    /// default so a single bad line never wipes out the rest.
    fn parse(contents: &str) -> (Self, Vec<String>) {
        let mut config = Self::default();
        let mut warnings = Vec::new();

        // Parse a boolean, warning (and keeping the default) on anything that
        // isn't exactly "true"/"false" instead of treating it as false.
        fn parse_bool(value: &str, key: &str, warnings: &mut Vec<String>) -> Option<bool> {
            match value {
                "true" => Some(true),
                "false" => Some(false),
                other => {
                    warnings.push(format!(
                        "invalid value '{}' for '{}' (expected true or false)",
                        other, key
                    ));
                    None
                }
            }
        }

        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(value) = line.strip_prefix("version=") {
                match value.parse::<u32>() {
                    Ok(v) if v > CONFIG_VERSION => warnings.push(format!(
                        "settings were written by a newer version of {} (v{} > v{}); \
                         some values may be ignored",
                        PRODUCT_NAME, v, CONFIG_VERSION
                    )),
                    Ok(_) => {}
                    Err(_) => warnings.push(format!("invalid version '{}'", value)),
                }
            } else if let Some(value) = line.strip_prefix("show_hidden=") {
                if let Some(v) = parse_bool(value, "show_hidden", &mut warnings) {
                    config.show_hidden = v;
                }
            } else if let Some(value) = line.strip_prefix("wrap_preview=") {
                if let Some(v) = parse_bool(value, "wrap_preview", &mut warnings) {
                    config.wrap_preview = v;
                }
            } else if let Some(value) = line.strip_prefix("line_numbers=") {
                if let Some(v) = parse_bool(value, "line_numbers", &mut warnings) {
                    config.line_numbers = v;
                }
            } else if let Some(value) = line.strip_prefix("show_icons=") {
                if let Some(v) = parse_bool(value, "show_icons", &mut warnings) {
                    config.show_icons = v;
                }
            } else if let Some(value) = line.strip_prefix("colored_icons=") {
                if let Some(v) = parse_bool(value, "colored_icons", &mut warnings) {
                    config.colored_icons = v;
                }
            } else if let Some(value) = line.strip_prefix("theme_icons=") {
                if let Some(v) = parse_bool(value, "theme_icons", &mut warnings) {
                    config.theme_icons = v;
                }
            } else if let Some(value) = line.strip_prefix("theme=") {
                config.theme = value.to_string();
            } else if let Some(value) = line.strip_prefix("search_navigate_on_open=") {
                if let Some(v) = parse_bool(value, "search_navigate_on_open", &mut warnings) {
                    config.search_navigate_on_open = v;
                }
            } else if let Some(value) = line.strip_prefix("sort_option=") {
                match sort_from_str(value) {
                    Some(s) => config.sort_option = s,
                    None => warnings.push(format!("unknown sort_option '{}'", value)),
                }
            } else if let Some(value) = line.strip_prefix("git_auto_push=") {
                if let Some(v) = parse_bool(value, "git_auto_push", &mut warnings) {
                    config.git_auto_push = v;
                }
            } else if let Some(value) = line.strip_prefix("audio_autoplay=") {
                if let Some(v) = parse_bool(value, "audio_autoplay", &mut warnings) {
                    config.audio_autoplay = v;
                }
            } else if let Some(value) = line.strip_prefix("audio_normalize=") {
                if let Some(v) = parse_bool(value, "audio_normalize", &mut warnings) {
                    config.audio_normalize = v;
                }
            } else if let Some(value) = line.strip_prefix("audio_skip_silence=") {
                if let Some(v) = parse_bool(value, "audio_skip_silence", &mut warnings) {
                    config.audio_skip_silence = v;
                }
            } else if let Some(value) = line.strip_prefix("audio_volume=") {
                match value.parse::<f32>() {
                    Ok(vol) => config.audio_volume = vol.clamp(0.0, 2.0),
                    Err(_) => warnings.push(format!("invalid audio_volume '{}'", value)),
                }
            } else if let Some(value) = line.strip_prefix("audio_analyzer_gradient=") {
                if let Some(v) = parse_bool(value, "audio_analyzer_gradient", &mut warnings) {
                    config.audio_analyzer_gradient = v;
                }
            } else if let Some(value) = line.strip_prefix("dir_sort:") {
                // Format: dir_sort:<path>=<sort_option>
                if let Some((path_str, sort_str)) = value.split_once('=') {
                    match sort_from_str(sort_str) {
                        Some(sort) => {
                            config.dir_sort_cache.insert(PathBuf::from(path_str), sort);
                        }
                        None => warnings.push(format!(
                            "unknown sort '{}' for directory '{}'",
                            sort_str, path_str
                        )),
                    }
                } else {
                    warnings.push(format!("malformed dir_sort entry: '{}'", line));
                }
            } else {
                warnings.push(format!("unknown setting: '{}'", line));
            }
        }

        (config, warnings)
    }

    /// Render the config to its on-disk text form.
    fn serialize(&self) -> String {
        let mut contents = format!(
            "version={}\nshow_hidden={}\nwrap_preview={}\nline_numbers={}\nshow_icons={}\ncolored_icons={}\ntheme_icons={}\ntheme={}\nsearch_navigate_on_open={}\nsort_option={}\ngit_auto_push={}\naudio_autoplay={}\naudio_normalize={}\naudio_skip_silence={}\naudio_volume={}\naudio_analyzer_gradient={}\n",
            CONFIG_VERSION,
            self.show_hidden,
            self.wrap_preview,
            self.line_numbers,
            self.show_icons,
            self.colored_icons,
            self.theme_icons,
            self.theme,
            self.search_navigate_on_open,
            sort_to_str(self.sort_option),
            self.git_auto_push,
            self.audio_autoplay,
            self.audio_normalize,
            self.audio_skip_silence,
            self.audio_volume,
            self.audio_analyzer_gradient,
        );

        // Append directory sort cache.
        for (path, sort) in &self.dir_sort_cache {
            contents.push_str(&format!(
                "dir_sort:{}={}\n",
                path.display(),
                sort_to_str(*sort)
            ));
        }

        contents
    }

    pub fn save(&self) {
        let Some(path) = Self::config_path() else {
            return;
        };

        // Create config directory if needed.
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let contents = self.serialize();

        // Write atomically: write to a temp file then rename over the target,
        // so a crash mid-write can't truncate or corrupt the existing config
        // (which would silently reset every setting on the next load).
        let tmp = path.with_extension("tmp");
        if fs::write(&tmp, contents.as_bytes()).is_ok() && fs::rename(&tmp, &path).is_err() {
            // Rename failed (e.g. cross-volume); fall back to a direct write
            // and clean up the temp file.
            let _ = fs::write(&path, contents.as_bytes());
            let _ = fs::remove_file(&tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_roundtrips_serialized_config() {
        let mut cfg = Config {
            show_hidden: false,
            theme: "gruvbox".to_string(),
            sort_option: SortOption::Size,
            audio_volume: 1.5,
            ..Config::default()
        };
        cfg.dir_sort_cache
            .insert(PathBuf::from("/tmp/x"), SortOption::DateModified);

        let (parsed, warnings) = Config::parse(&cfg.serialize());
        assert!(
            warnings.is_empty(),
            "clean config should not warn: {:?}",
            warnings
        );
        assert!(!parsed.show_hidden);
        assert_eq!(parsed.theme, "gruvbox");
        assert_eq!(parsed.sort_option, SortOption::Size);
        assert_eq!(parsed.audio_volume, 1.5);
        assert_eq!(
            parsed.dir_sort_cache.get(&PathBuf::from("/tmp/x")),
            Some(&SortOption::DateModified)
        );
    }

    #[test]
    fn malformed_bool_warns_and_keeps_default() {
        let (cfg, warnings) = Config::parse("show_hidden=yes\nline_numbers=true\n");
        // The bad value is rejected (default true retained), the good one applies.
        assert!(cfg.show_hidden, "bad bool should keep the default (true)");
        assert!(cfg.line_numbers, "valid bool should still apply");
        assert!(warnings.iter().any(|w| w.contains("show_hidden")));
    }

    #[test]
    fn unknown_key_and_sort_warn() {
        let (cfg, warnings) = Config::parse("frobnicate=1\nsort_option=banana\n");
        assert_eq!(cfg.sort_option, SortOption::Name, "bad sort keeps default");
        assert!(warnings.iter().any(|w| w.contains("unknown setting")));
        assert!(warnings.iter().any(|w| w.contains("sort_option")));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let (_cfg, warnings) = Config::parse("# a comment\n\n   \nshow_hidden=false\n");
        assert!(
            warnings.is_empty(),
            "comments/blanks must not warn: {:?}",
            warnings
        );
    }

    #[test]
    fn newer_version_warns() {
        let (_cfg, warnings) = Config::parse(&format!("version={}\n", CONFIG_VERSION + 1));
        assert!(warnings.iter().any(|w| w.contains("newer version")));
    }

    #[test]
    fn legacy_file_without_version_parses_clean() {
        let (cfg, warnings) = Config::parse("show_hidden=false\ntheme=dark\n");
        assert!(!cfg.show_hidden);
        assert_eq!(cfg.theme, "dark");
        assert!(warnings.is_empty());
    }
}
