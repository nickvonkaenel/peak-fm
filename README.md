# Peak File Manager

Peak File Manager (`pk`, also available as `peak-fm`) is a fast, keyboard-first
terminal file manager written in Rust. It combines Miller columns with a
staged, text-editing-inspired workflow: edit directory entries in memory,
review the resulting filesystem operations, then apply them together.

This is an early `0.1.0` release. The core file manager is usable, but several
tool choices are intentionally opinionated and currently hard-coded. Those
integrations are documented below; making them configurable is part of the
roadmap.

## Highlights

- Parent, current, and preview Miller columns
- Vim-style navigation, inline filename editing, visual selection, and marks
- Staged create, rename, delete, copy, and move operations with global review
- Syntax-highlighted text previews and Kitty-protocol image previews
- Built-in fuzzy file finding plus ripgrep-powered content search
- Built-in audio browser, playback, waveform, metadata, and spectrum views
- Git actions, lazygit, shell commands, zoxide, CSV, FFmpeg, and REAPER workflows

## Current requirements and integrations

### Build requirements

- Rust 1.88 or newer and Cargo
- On Debian/Ubuntu Linux: `libasound2-dev`
- On Fedora Linux: `alsa-lib-devel`

The ALSA development package is needed because audio support is currently built
unconditionally on Linux.

### Terminal recommendations

- A true-color terminal is recommended.
- A Nerd Font is recommended for file icons. Icons can be disabled in the
  display options.
- Inline images require a terminal that implements the Kitty graphics
  protocol. The rest of the application works without it.

### External commands

Basic navigation and filesystem editing do not require the tools below. Each
command enables a specific integration:

| Command or application | Current use |
|---|---|
| `nvim` | Opens text files and directories; also edits the FFmpeg job template |
| `csvlens` | Opens `.csv` files with `csvlens --ignore-case` |
| `rg` | Powers content grep (`g`, `G`, and `--grep`) |
| `zoxide` | Powers directory jumping with `z` |
| `git` | Built-in status, pull, push, and commit actions |
| `lazygit` | Opens the lazygit TUI with `Space`, then `l` |
| `ffprobe` and `ffmpeg` | Media inspection and the FFmpeg editor |
| `open`, `xdg-open`, or `cmd` | Opens non-text files with the platform default |
| `explorer` or the platform opener | Reveals a selected file |
| `xclip` or `wl-copy` | File clipboard support on Linux |
| REAPER | Optional copy-and-focus workflow; Linux focus uses `wmctrl` or `xdotool` |

Audio browsing and playback are built in. They do not require FastSoundFinder
or FFmpeg.

## Themes

Press `Space`, then `c` to open the syntax-theme picker. Use `j`/`k` or the
arrow keys for live preview, `Page Up`/`Page Down` for larger jumps, `Enter` to
save the selection, or `Esc` to cancel.

Peak File Manager includes the legacy `noclownfiesta` TextMate adaptation
previously shipped by `fm`, based on
[No Clown Fiesta by Gustaf Rydholm](https://github.com/aktersnurra/no-clown-fiesta.nvim).
It is the default and retains the same settings key. The audited upstream
revision declares no license, so the bundled adaptation is not covered by
Peak's MIT license; see
[Syntax and Theme Licenses and Provenance Notices](SYNTAX_THEME_LICENSES.md).

Peak also includes Syntect's Base16, InspiredGitHub, and Solarized themes, plus
a curated set from `two-face`: Dracula, GitHub, Gruvbox dark and light, Monokai
Extended, Nord, and One Half dark and light.
[Upstream provenance for those assets is maintained by `two-face`](https://github.com/CosmicHorrorDev/two-face/blob/v0.4.5/generated/acknowledgements_full.md).

To add a personal TextMate theme:

1. Create the theme directory if it does not exist:
   - Unix and macOS: `~/.config/peak-fm/themes`
   - Windows: `%LOCALAPPDATA%\peak-fm\themes`
2. Copy one or more `.tmTheme` files into it. Subdirectories are also scanned.
3. Restart `pk`, then open `Space`, `c` and select the theme.

To author a theme, create a standard TextMate property-list file. This is a
minimal starting point:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>name</key>
  <string>My Theme</string>
  <key>settings</key>
  <array>
    <dict>
      <key>settings</key>
      <dict>
        <key>background</key><string>#101216</string>
        <key>foreground</key><string>#d8dee9</string>
      </dict>
    </dict>
    <dict>
      <key>scope</key><string>comment</string>
      <key>settings</key>
      <dict><key>foreground</key><string>#6b7280</string></dict>
    </dict>
  </array>
</dict>
</plist>
```

The filename without `.tmTheme` is the name stored in settings and shown in
the picker. A personal theme with the same filename as a bundled theme
overrides the bundled copy. Malformed files are skipped and reported in the
startup status.

Themes currently control syntax-preview colors and, when `theme_icons=true`,
the palette used to remap icon colors. They do not recolor the entire TUI or
terminal background, so dark variants are intended for dark terminals and
light variants for light terminals.

## Installation

Peak File Manager is currently installed from source:

```bash
cd path/to/peak-fm
cargo install --path . --locked
```

Cargo installs both commands:

```text
pk          # preferred short command
peak-fm     # full-name fallback if pk collides with another command
```

For a local build without installing:

```bash
cargo build --release --locked
```

## Usage

```text
pk [OPTIONS] [PATH]
peak-fm [OPTIONS] [PATH]
```

| Option | Description |
|---|---|
| `-s`, `--search` | Start in fuzzy file search |
| `-g`, `--grep` | Start in content grep |
| `-f`, `--audio` | Start in the audio browser |
| `-p`, `--pick` | Exit after opening a selection, or on `Esc` |
| `-n`, `--nvim` | Emit a selection action for a custom Neovim wrapper |
| `--select FILE` | Preselect a file by name |
| `--cwd DIR` | Lock the search/grep work root to a directory |
| `-h`, `--help` | Print command help |
| `-V`, `--version` | Print the version |

Passing a file as `PATH` opens its parent directory and selects that file.

The `--nvim` mode is an integration mode: it emits an edit, split, vertical
split, or tab action after the TUI exits instead of spawning Neovim itself.
Its output format is experimental.

## Quick start

1. Run `pk` or `pk path/to/directory`.
2. Navigate with `j`, `k`, `h`, and `l`.
3. Edit names with `i`, `a`, or `A`; create entries with `o`; stage deletions
   with `d`.
4. Press `Ctrl+y` to review and apply all pending operations.
5. Press `Ctrl+g` for context-sensitive help or `Space` for the leader menu.

## Keybindings

### Navigation

| Key | Action |
|---|---|
| `j` / `k`, arrows | Move down / up |
| `h` / `l`, left / right | Parent directory / enter or open |
| `Enter` | Enter a directory, open a file, or play selected audio |
| `t` / `b`, Home / End | Jump to top / bottom |
| `Ctrl+d` / `Ctrl+u` | Half page down / up |
| `Ctrl+f` / `Ctrl+b` | Scroll the preview down / up |
| `-` | Toggle the previous directory |
| `.` | Toggle hidden files |
| `Ctrl+t` | Set the current directory as the work root |

### Editing and staged operations

| Key | Action |
|---|---|
| `i` / `I` | Edit the selected name from the start |
| `a` | Edit before the extension |
| `A` | Edit at the end |
| `c` | Clear the name and enter insert mode |
| `o` / `O` | Create an entry below / above; end with `/` for a directory |
| `d` / `D` | Stage deletion of the selection / all marked files |
| `y` / `Y` | Yank the selection / all marked files |
| `p` | Paste yanked or cut files |
| `u` / `Ctrl+r` | Undo / redo buffer edits |
| `v` | Enter visual selection mode |
| `;` | Toggle a mark on the selected file |
| `Ctrl+y` | Review and apply all pending operations |

### Search

| Key | Action |
|---|---|
| `/` | Filter the current directory list |
| `n` / `N` | Next / previous filter match |
| `s` / `S` | Fuzzy find from the work root / current directory |
| `g` / `G` | Content grep from the work root / current directory |
| `r` | Resume the previous find or grep |
| `z` | Jump with zoxide |

### Tools and other actions

| Key | Action |
|---|---|
| `f` | Open the built-in audio browser at the current directory |
| `Ctrl+l` | Play / pause audio in normal mode |
| `e` | Open the current directory in Neovim |
| `Ctrl+o` | Reveal the selected file with the platform file manager |
| `C` | Copy the selected file to the system file clipboard |
| `R` | Copy the selected file and focus REAPER |
| `!` | Run a shell command against the selection or marked files |
| `w` / `#` | Toggle preview wrapping / line numbers |
| `Ctrl+g` | Open context-sensitive help |
| `q` | Quit, confirming if staged operations remain |

### Leader menu

Press `Space`, then:

| Key | Action |
|---|---|
| `.` | Toggle hidden files |
| `a` | Open the audio browser |
| `c` | Select a syntax theme |
| `f` | Open the FFmpeg editor for the selected media file |
| `g` | Open the Git menu |
| `i` | Choose file information shown in the list |
| `l` | Open lazygit |
| `p` | Commit all Git changes |
| `q` | Quit |
| `s` / `S` | Set directory / global sort order |
| `t` | Toggle the private trash directory |
| `T` | Permanently empty the trash while browsing it |
| `u` | Open display options |

The Git menu provides status (`s`), pull (`g`), push (`p`), commit staged (`c`),
and commit all (`a`). In the commit prompt, `Ctrl+p` toggles persistent
auto-push and `Enter` commits.

The audio browser renders its own control reference in the UI.

## Filesystem workflow

Directory edits are staged in memory. Peak File Manager compares the edited
buffer with its original snapshot and keeps pending operations when you move
between directories. `Ctrl+y` opens a global confirmation before anything is
applied.

Deleting an entry outside Peak File Manager's private trash moves it into that
trash when the operation is synced. While browsing the trash:

- `x` restores the selected entry.
- `X` restores all marked entries.
- `Ctrl+t` or `Space`, then `T` requests permanent deletion.

Emptying the trash is irreversible.

## Find, grep, and audio scan behavior

Fuzzy find is built in, scans at most ten directory levels, hides hidden files
and directories by default, and respects Git ignore rules. Both find and grep
also recognize `.pkignore`, the legacy `.fmignore`, and `.ignore`.

`s` and `g` search from the work root; `S` and `G` search only from the current
directory. Content grep requires `rg`.

Audio mode recursively scans the current directory, skips hidden entries, and
does not apply Git ignore rules. Starting it at a very broad directory can
therefore produce a large scan. A hashed SQLite index is stored per canonical
audio scan root.

## Shell commands

Press `!` to enter a shell command. Marked files are used when marks exist;
otherwise the selected entry is used.

| Placeholder | Expansion |
|---|---|
| `%f` | All target paths, shell-quoted |
| `%n` | All target basenames, shell-quoted |
| `%d` | Current directory, shell-quoted |
| `%%` | A literal `%` |

Commands run through `sh -c` on Unix and `cmd /C` on Windows.

## Change directory on exit

The application writes its last directory to the system temporary directory as
`peak-fm-lastdir`.

For Bash or Zsh:

```bash
pk() {
    command pk "$@"
    local lastdir
    lastdir="$(cat "${TMPDIR:-/tmp}/peak-fm-lastdir" 2>/dev/null)" || return
    if [ -d "$lastdir" ] && [ "$lastdir" != "$PWD" ]; then
        builtin cd -- "$lastdir"
    fi
}
```

If `pk` already belongs to another program, give the function another name and
call `command peak-fm "$@"` inside it.

For PowerShell:

```powershell
function pk {
    & pk.exe @args
    $lastdir = Get-Content "$env:TEMP\peak-fm-lastdir" -ErrorAction SilentlyContinue
    if ($lastdir -and (Test-Path -LiteralPath $lastdir) -and ($lastdir -ne $PWD.Path)) {
        Set-Location -LiteralPath $lastdir
    }
}
```

## Configuration and state

Settings use a `key=value` file:

- Unix and macOS: `~/.config/peak-fm/settings`
- Windows: `%LOCALAPPDATA%\peak-fm\settings`

If the new file does not exist, settings from the previous private-build
location are read as a compatibility fallback. The next settings change is
written to the new location.

| Setting | Default | Description |
|---|---:|---|
| `show_hidden` | `true` | Show hidden entries |
| `wrap_preview` | `false` | Wrap text previews |
| `line_numbers` | `false` | Show preview line numbers |
| `show_icons` | `true` | Show file type icons |
| `colored_icons` | `true` | Color file type icons |
| `theme_icons` | `true` | Map icon colors to the syntax theme |
| `theme` | `noclownfiesta` | Syntax highlighting theme |
| `search_navigate_on_open` | `true` | Navigate to a search result's directory after opening |
| `sort_option` | `name` | Default sort order |
| `git_auto_push` | `false` | Push automatically after a commit |
| `audio_autoplay` | `true` | Start playback when an audio file is selected |
| `audio_normalize` | `false` | Normalize audio waveform playback gain |
| `audio_skip_silence` | `false` | Skip detected silence during playback |
| `audio_volume` | `1.0` | Linear volume, clamped from `0.0` to `2.0` |
| `audio_analyzer_gradient` | `false` | Use a gradient in the spectrum analyzer |

Per-directory sort overrides are stored as managed `dir_sort:<path>=<sort>`
entries.

Other state locations:

- Trash: `~/.local/share/peak-fm/trash` on Unix/macOS,
  `%LOCALAPPDATA%\peak-fm\trash` on Windows
- Personal themes: `~/.config/peak-fm/themes` on Unix/macOS,
  `%LOCALAPPDATA%\peak-fm\themes` on Windows
- Waveforms: `~/.cache/peak-fm/waveforms`
- Audio indexes: the platform config directory under `peak-fm/audio`
- Last directory: the system temp directory's `peak-fm-lastdir`

## Current limitations and roadmap

- Editor, CSV viewer, Git TUI, media tools, and platform commands are hard-coded.
- Kitty is the only supported inline-image protocol.
- The custom Neovim selection output is experimental.
- External command configuration, editor fallbacks, additional image
  protocols, and less opinionated platform integrations are planned.

## Development

```bash
cargo fmt --check
cargo check --locked --all-targets
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings -D clippy::all
cargo build --locked --release --bins
```

CI runs formatting, debug builds, tests, Clippy, and command smoke tests on
Linux, macOS, and Windows. Linux also verifies the Cargo release package.

## License

Peak File Manager's original code is available under the [MIT License](LICENSE).
That license does not apply to `src/core/noclownfiesta.tmTheme`. Bundled
third-party syntax and theme assets—and their attribution or license
status—are documented in
[Syntax and Theme Licenses and Provenance Notices](SYNTAX_THEME_LICENSES.md).
Personal themes are loaded from the user's machine, are not distributed with
Peak File Manager, and remain subject to their own terms.
