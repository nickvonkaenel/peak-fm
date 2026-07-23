#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Visual {
        anchor: usize,
    }, // Visual line selection mode
    VisualInsert {
        anchor: usize,
        edit_type: VisualEditType,
    }, // Multi-file insert
    Confirm(ConfirmAction),
    SyncConfirm {
        scroll: usize,
    }, // Sync confirmation popup with operation details
    QuitConfirm {
        scroll: usize,
    }, // Quit confirmation popup showing unsaved changes
    Search(SearchDirection),
    Find, // Full-screen fuzzy file search
    Grep, // Full-screen content search (ripgrep)
    Help,
    Settings, // Quick settings popup (Ctrl+g)
    ThemeSelect {
        selected: usize,
    },
    Sort {
        selected: usize,
        is_global: bool,
    }, // Sort options popup (Ctrl+s or leader+s/S)
    InfoSelect {
        selected: usize,
    }, // Display info selector popup (space+i)
    Leader,         // Leader key menu (space)
    PreviewOptions, // Preview options submenu (space+u)
    Git,            // Git commands submenu (space+g)
    GitCommit {
        message: String,
        all: bool,
        auto_push: bool,
        status: Vec<String>,
    }, // Git commit with message input
    GitStatus {
        lines: Vec<String>,
        scroll: usize,
    }, // Git status display
    Audio,          // Audio browser mode (fsf integration)
    Command,        // Shell command input line (`!`)
}

/// Type of edit being applied in visual insert mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualEditType {
    Start,     // i/I - insert at start of name
    BeforeExt, // a - insert before extension
    End,       // A - insert at end of name
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ConfirmAction {
    Sync,
    Quit,
    EmptyTrash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SearchDirection {
    Forward,
    Backward,
}

impl Mode {
    pub fn name(&self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual { .. } => "VISUAL",
            Mode::VisualInsert { .. } => "V-INSERT",
            Mode::Confirm(_) => "CONFIRM",
            Mode::SyncConfirm { .. } => "SYNC",
            Mode::QuitConfirm { .. } => "QUIT",
            Mode::Search(_) => "SEARCH",
            Mode::Find => "SEARCH",
            Mode::Grep => "GREP",
            Mode::Help => "HELP",
            Mode::Settings => "SETTINGS",
            Mode::ThemeSelect { .. } => "THEME",
            Mode::Sort { .. } => "SORT",
            Mode::InfoSelect { .. } => "INFO",
            Mode::Leader => "LEADER",
            Mode::PreviewOptions => "PREVIEW",
            Mode::Git => "GIT",
            Mode::GitCommit { .. } => "COMMIT",
            Mode::GitStatus { .. } => "STATUS",
            Mode::Audio => "AUDIO",
            Mode::Command => "COMMAND",
        }
    }
}
