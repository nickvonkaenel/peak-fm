# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Project Overview

`peak-fm` (Peak File Manager) is a fast, vim-like terminal file manager written in Rust. The primary executable is `pk`, with `peak-fm` provided as a collision-safe full-name command. It uses a Miller column interface (parent | current | preview) with vim-style keybindings and supports inline file editing, visual mode for bulk operations, syntax highlighting, audio playback with waveform visualization, fuzzy search, and git integration.

## Build and Development Commands

```bash
# Build and install the binary
cargo install --path .

# Build for development
cargo build

# Build for release
cargo build --release

# Run tests
cargo test

# Run with specific flags
cargo run -- [OPTIONS] [PATH]
# Options:
#   -s, --search   Start in search mode
#   -g, --grep     Start in grep mode
#   -f, --audio    Start in audio mode
#   -p, --pick     Picker mode (quit on Esc or after opening)
#   -n, --nvim     Neovim mode (output path for nvim integration)
#   --select FILE  Pre-select a file
#   --cwd DIR      Lock search/grep to directory
```

## Architecture Overview

### Core Module Structure

The application follows a modular architecture with clear separation of concerns:

**`src/main.rs` and `src/bin/peak-fm.rs`**: Thin executable wrappers for the `pk` and `peak-fm` commands.

**`src/lib.rs`**: CLI parsing, terminal initialization (crossterm), and the main event loop. Handles terminal lifecycle (raw mode, alternate screen) and event polling with variable refresh rates (60 FPS when audio is playing, 20 FPS otherwise).

**`src/app/mod.rs`**: Central application state (`App` struct) containing all UI state, mode management, panes, preview, search/grep/audio states, undo/redo history, marked files, and global operations. Acts as the coordinating hub between all subsystems.

**`src/core/`**: Core business logic independent of UI:
- `buffer.rs`: Buffer model with lines, IDs, snapshots for diff computation
- `pane.rs`: Directory pane with cursor, entries, and buffer
- `preview.rs`: Preview loading logic (files, directories, images)
- `diff.rs`: Computes filesystem operations from buffer changes
- `operations.rs`: Global operation store that tracks pending changes across directories
- `highlight.rs`: Syntax highlighting using syntect
- `audio/`: Audio playback, analysis, waveform generation, and database caching
- `sort.rs`: File sorting options and display info

**`src/fs/`**: Filesystem operations:
- `scan.rs`: Directory reading with filtering and recursive scanning
- `sync.rs`: Applies filesystem operations (create, delete, rename, copy, move), validates operations, manages trash
- `volumes.rs`: Cross-platform volume/drive detection

**`src/input/`**: Input handling and mode definitions:
- `mode.rs`: Enum defining all application modes (Normal, Insert, Visual, Search, Find, Grep, Audio, Leader, etc.)

**`src/app/handler/`**: Key event routing and mode-specific input handlers split by mode for maintainability

**`src/ui/`**: UI rendering using ratatui:
- `mod.rs`: Main render function orchestrating all panels
- `modes.rs`: Mode-specific UI rendering
- `menus.rs`: Leader menu, sort menu, settings menu rendering
- `dialogs.rs`: Confirmation dialogs, git status, sync preview
- `format.rs`: Text formatting utilities

### Key Concepts

**Buffer-Based Editing Model**: Changes are staged in a buffer until synced with `Ctrl+y`. The buffer maintains:
- Current state (`lines`): current view of files/dirs with modifications
- Snapshot (`snapshot`): original state from disk
- IDs: unique identifiers linking buffer lines to filesystem entries
- Diff computation: compares current vs snapshot to generate operations

**Global Operation Store**: Tracks pending filesystem operations across multiple directories. When navigating away from a directory, operations are captured; when returning, operations are restored to the buffer. All operations are validated globally before sync to prevent conflicts.

**Multi-Clipboard System**: The `yank` field stores multiple files. Operations can be either copy (yank `y`) or cut (delete `d`). Paste (`p`) intelligently restores deleted files from the same directory or creates copy/move operations.

**Directory State Persistence**:
- `dir_cursors`: HashMap remembering cursor positions per directory
- `dir_sort_cache`: Per-directory sort options override global sort
- Operations persist when navigating between directories

**Mode System**: The `Mode` enum in `src/input/mode.rs` defines all interactive modes. Each mode has dedicated input handlers in `src/app/handler/` and rendering in `src/ui/modes.rs`.

**Search Modes**:
- **Find** (`s`/`S`): Fuzzy file search using `nucleo` matcher with background scanning
- **Grep** (`g`/`G`): Content search using ripgrep with background result streaming
- **Zoxide** (`z`): Directory jumping using zoxide integration
- Both support resuming last search with `r`

**Audio System** (`src/core/audio/`):
- Player with seek, volume, speed control
- Decoded, downsampled waveforms cached with Bincode
- FFT-based real-time frequency analyzer for visualization
- Per-root audio metadata indexes stored in SQLite
- Smart silence detection for auto-skip

**Async Operations**: Image loading, git operations (push, commit), and audio waveform generation use background threads with `mpsc` channels polled in the main loop.

**Theme System**: Syntax highlighting themes come from Syntect defaults, a curated subset of `two-face`, and personal `.tmTheme` files loaded from the platform themes directory. The existing `Space`, then `c` picker previews and persists selections. Personal files override same-named bundled themes. Icon colors can map to the selected theme palette when `theme_icons` is enabled.

`syntect` and `two-face` are pinned because their embedded assets, versions, and notices move together. After intentionally updating either dependency, update the version in the notice generator, run `cargo run --locked --example generate_third_party_licenses`, and review the regenerated `SYNTAX_THEME_LICENSES.md`.

Personal theme files must not be copied into the repository or a release package without a separate provenance review and an explicit, compatible redistribution license.

## Development Guidelines

### Adding New File Operations

1. Define the operation in `src/core/diff.rs` (e.g., `FsOperation` enum)
2. Implement validation in `src/fs/sync.rs::validate_global_operations`
3. Implement execution in `src/fs/sync.rs::apply_operations`
4. Buffer changes are automatically captured by the diff system

### Adding New Modes

1. Add variant to `Mode` enum in `src/input/mode.rs`
2. Add input handler in `src/app/handler/` (create new file if complex)
3. Add rendering in `src/ui/modes.rs`
4. Update `Mode::name()` for status line display

### Working with Settings

Settings are persisted in `~/.config/peak-fm/settings` (Unix) or `%LOCALAPPDATA%/peak-fm/settings` (Windows). The config system uses a simple key=value format. To add a new setting:

1. Add field to `Config` struct in `src/config.rs`
2. Add to `Default::default()` implementation
3. Add parsing in `Config::load()`
4. Add serialization in `Config::save()`
5. Update `App::new()` to read and use the setting

### Audio File Handling

Audio files are detected by extension in `src/core/preview.rs::is_audio_file()`. Each canonical scan root gets a hashed SQLite index beneath the platform config directory's `peak-fm/audio` folder. Waveforms are cached separately beneath `.cache/peak-fm/waveforms`, keyed by BLAKE3 hash for integrity.

### Platform-Specific Code

Windows-specific code uses `#[cfg(target_os = "windows")]`. Common differences:
- Path prefix stripping: Windows extended-length paths (`\\?\`) need normalization
- File opening: Uses `cmd /C start` instead of `open`/`xdg-open`
- Clipboard: Uses `clipboard-win` crate

### Testing Notes

- Use `tempfile` crate for filesystem tests
- Test buffer operations with snapshot comparison
- Test diff computation with known before/after states

## Important Patterns

- **Preview refresh**: Always call `app.refresh_preview()` after cursor movement or directory changes
- **Undo state**: Call `app.save_undo_state()` BEFORE modifying buffer
- **Navigation**: Use `app.navigate_to()` for directory changes (handles parent pane refresh)
- **Status messages**: Use `app.set_status()` with auto-clear after 2 seconds
- **Dirty checking**: Use `app.is_dirty()` to check for unsaved changes before quit
- **Operations capture**: Call `app.capture_current_operations()` before leaving a directory

## File Locations

- Config: `~/.config/peak-fm/settings` (Unix), `%LOCALAPPDATA%\peak-fm\settings` (Windows)
- Trash: `~/.local/share/peak-fm/trash` (Unix), `%LOCALAPPDATA%\peak-fm\trash` (Windows)
- Personal themes: `~/.config/peak-fm/themes` (Unix), `%LOCALAPPDATA%\peak-fm\themes` (Windows)
- Audio indexes: platform config directory under `peak-fm/audio/`
- Waveform cache: `~/.cache/peak-fm/waveforms`
- Last dir: `$TMPDIR/peak-fm-lastdir` (Unix), `%TEMP%\peak-fm-lastdir` (Windows)
