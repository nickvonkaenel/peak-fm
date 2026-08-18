// Allow stylistic clippy lints that don't affect correctness
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::ptr_arg)] // &PathBuf vs &Path - not worth changing everywhere

mod app;
mod config;
mod core;
mod fs;
mod input;
mod paths;
mod ui;

use std::io::{self, stdout};
use std::path::PathBuf;
use std::time::Duration;

use crossterm::cursor::SetCursorStyle;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{enable_raw_mode, EnterAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::App;
use input::Mode;
use paths::{LAST_DIR_FILE_NAME, PRODUCT_NAME};

struct Args {
    path: PathBuf,
    search: bool,
    grep: bool,
    audio: bool,
    zoxide: bool,
    pick: bool,
    nvim: bool,
    select: Option<String>,
    cwd: Option<PathBuf>,
}

fn parse_args() -> io::Result<Args> {
    let mut args = std::env::args().skip(1);
    let mut path: Option<PathBuf> = None;
    let mut search = false;
    let mut grep = false;
    let mut audio = false;
    let mut zoxide = false;
    let mut pick = false;
    let mut nvim = false;
    let mut select: Option<String> = None;
    let mut cwd: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-s" | "--search" => search = true,
            "-g" | "--grep" => grep = true,
            "-f" | "--audio" => audio = true,
            "-z" | "--zoxide" => zoxide = true,
            "-p" | "--pick" => pick = true,
            "-n" | "--nvim" => nvim = true,
            "--select" => {
                select = args.next();
            }
            "--cwd" => {
                cwd = args.next().map(|p| {
                    let path = PathBuf::from(p);
                    path.canonicalize().unwrap_or(path)
                });
            }
            "-h" | "--help" => {
                eprintln!("{} {}", PRODUCT_NAME, env!("CARGO_PKG_VERSION"));
                eprintln!();
                eprintln!("Usage: pk [OPTIONS] [PATH]");
                eprintln!("       peak-fm [OPTIONS] [PATH]");
                eprintln!();
                eprintln!("Options:");
                eprintln!("  -s, --search    Start in search mode (fuzzy file search)");
                eprintln!("  -g, --grep      Start in grep mode (content search)");
                eprintln!("  -f, --audio     Start in audio mode (audio file browser)");
                eprintln!("  -z, --zoxide    Start in zoxide mode (directory jump list)");
                eprintln!("  -p, --pick      Picker mode: quit on Esc or after opening");
                eprintln!("  -n, --nvim      Emit a selection for a Neovim wrapper");
                eprintln!(
                    "                  Keys: Enter=edit, Ctrl+s=split, Ctrl+v=vsplit, Ctrl+t=tab"
                );
                eprintln!("  --select FILE   Pre-select a file by name");
                eprintln!("  --cwd DIR       Lock search/grep to this directory");
                eprintln!("  -h, --help      Show this help message");
                eprintln!("  -V, --version   Show the version");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("peak-fm {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ if !arg.starts_with('-') => {
                path = Some(PathBuf::from(arg));
            }
            _ => {
                eprintln!("Unknown option: {}", arg);
                std::process::exit(1);
            }
        }
    }

    let path = match path {
        Some(p) => p.canonicalize().unwrap_or(p),
        None => std::env::current_dir()?,
    };

    // If path is a file, use its parent as the directory and select the file
    let (path, select) = if path.is_file() {
        let filename = path.file_name().map(|s| s.to_string_lossy().to_string());
        let parent = path.parent().map(|p| p.to_path_buf()).unwrap_or(path);
        (parent, select.or(filename))
    } else {
        (path, select)
    };

    Ok(Args {
        path,
        search,
        grep,
        audio,
        zoxide,
        pick,
        nvim,
        select,
        cwd,
    })
}

pub fn run_cli() -> io::Result<()> {
    let args = parse_args()?;

    // Enable raw mode and alternate screen with true color support
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;

    // Create terminal with crossterm backend
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut app = App::new(args.path, args.pick, args.nvim, args.select, args.cwd)?;

    // Start in requested mode
    if args.search {
        app.enter_find_mode();
    } else if args.grep {
        app.enter_grep_mode();
    } else if args.audio {
        app.enter_audio_mode();
    } else if args.zoxide {
        app.enter_zoxide_mode();
    }

    let result = run(&mut terminal, &mut app);

    // Restore terminal
    use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);

    // Output nvim result if in nvim mode
    if let Some(action) = app.nvim_result {
        use app::NvimAction;

        // Helper to clean Windows extended-length path prefix
        let clean_path = |path: &PathBuf| -> String {
            let path_str = path.display().to_string();
            #[cfg(target_os = "windows")]
            {
                if let Some(stripped) = path_str.strip_prefix(r"\\?\") {
                    return stripped.to_string();
                }
            }
            path_str
        };

        // Include work_dir in output so neovim can update its working directory
        let cwd_str = clean_path(&app.work_dir);

        let output = match action {
            NvimAction::Edit(path, pos) => match pos {
                Some((line, col)) => {
                    format!("edit:{}:{}:{}:{}", clean_path(&path), line, col, cwd_str)
                }
                None => format!("edit:{}:::{}", clean_path(&path), cwd_str),
            },
            NvimAction::Split(path, pos) => match pos {
                Some((line, col)) => {
                    format!("split:{}:{}:{}:{}", clean_path(&path), line, col, cwd_str)
                }
                None => format!("split:{}:::{}", clean_path(&path), cwd_str),
            },
            NvimAction::Vsplit(path, pos) => match pos {
                Some((line, col)) => {
                    format!("vsplit:{}:{}:{}:{}", clean_path(&path), line, col, cwd_str)
                }
                None => format!("vsplit:{}:::{}", clean_path(&path), cwd_str),
            },
            NvimAction::Tab(path, pos) => match pos {
                Some((line, col)) => {
                    format!("tabedit:{}:{}:{}:{}", clean_path(&path), line, col, cwd_str)
                }
                None => format!("tabedit:{}:::{}", clean_path(&path), cwd_str),
            },
        };
        println!("{}", output);
    }

    // Write last directory to temp file for shell wrapper
    let lastdir_path = std::env::temp_dir().join(LAST_DIR_FILE_NAME);
    let cwd_str = app.cwd.to_string_lossy().to_string();
    // Remove Windows extended-length path prefix for shell compatibility
    #[cfg(target_os = "windows")]
    let cwd_str = cwd_str
        .strip_prefix(r"\\?\")
        .map(str::to_string)
        .unwrap_or(cwd_str);
    let _ = std::fs::write(lastdir_path, cwd_str.as_bytes());

    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        // Check if terminal needs to be reinitialized (after opening editor)
        if app.needs_reinit {
            // Re-enter raw mode and alternate screen
            enable_raw_mode()?;
            execute!(io::stdout(), EnterAlternateScreen)?;
            terminal.clear()?;
            app.needs_reinit = false;
            // Refresh preview in case file was modified
            app.refresh_preview();
        }

        // Poll background scanner in find mode
        app.poll_find_scanner();

        // Poll grep results
        app.poll_grep_results();

        // Check for debounced image loading
        app.check_pending_image();

        // Check for background git operations
        app.check_pending_git();

        // Poll audio mode for scan/waveform updates
        app.poll_audio_mode();

        app.clear_expired_status();
        terminal.draw(|frame| ui::render(frame, app))?;

        // Render any pending images after the frame
        ui::render_pending_image();

        // Set cursor style based on mode (after frame is rendered to avoid flicker)
        let cursor_style = if app.find_state.is_some()
            || app.grep_state.is_some()
            || app.audio_state.is_some()
            || matches!(app.mode, Mode::Insert | Mode::Search(_) | Mode::Command)
        {
            SetCursorStyle::BlinkingBar
        } else {
            SetCursorStyle::DefaultUserShape
        };
        let _ = execute!(stdout(), cursor_style);

        // Use 60 FPS (16ms) when audio is playing for smooth visualizations, otherwise 20 FPS (50ms)
        let is_audio_playing = app
            .audio_state
            .as_ref()
            .map(|s| s.is_playing())
            .unwrap_or(false);
        let poll_timeout = if is_audio_playing {
            Duration::from_millis(16) // ~60 FPS
        } else {
            Duration::from_millis(50) // ~20 FPS
        };

        if event::poll(poll_timeout)? {
            if let Event::Key(key) = event::read()? {
                // Only process key press events to avoid double input on Windows
                if key.kind == KeyEventKind::Press {
                    app::handler::handle_key(app, key)?;
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
