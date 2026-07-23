//! Shared product names and on-disk path components.

use std::path::PathBuf;

pub const PRODUCT_NAME: &str = "Peak File Manager";
pub const APP_DIR_NAME: &str = "peak-fm";
pub const LEGACY_APP_DIR_NAME: &str = "fm";
pub const SETTINGS_FILE_NAME: &str = "settings";
pub const THEMES_DIR_NAME: &str = "themes";
pub const LAST_DIR_FILE_NAME: &str = "peak-fm-lastdir";
pub const FFMPEG_EDIT_FILE_NAME: &str = "peak-fm-ffmpeg.yaml";
pub const TRASH_INDEX_FILE_NAME: &str = ".peak-fm-origins.json";
pub const TRASH_LOCK_FILE_NAME: &str = ".peak-fm-origins.lock";
pub const IGNORE_FILE_NAME: &str = ".pkignore";
pub const LEGACY_IGNORE_FILE_NAME: &str = ".fmignore";

/// Directory containing user-editable Peak File Manager configuration.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        dirs::data_local_dir().map(|path| path.join(APP_DIR_NAME))
    }
    #[cfg(not(windows))]
    {
        dirs::home_dir().map(|path| path.join(".config").join(APP_DIR_NAME))
    }
}

pub fn themes_dir() -> Option<PathBuf> {
    config_dir().map(|path| path.join(THEMES_DIR_NAME))
}
