//! Shell command input line (`!`) key handling

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::input::Mode;

pub(super) fn handle_command(app: &mut App, key: KeyEvent) -> io::Result<()> {
    match (key.code, key.modifiers) {
        // Run the command (run_shell_command returns to Normal mode itself).
        (KeyCode::Enter, _) => {
            let template = std::mem::take(&mut app.command_input);
            app.run_shell_command(&template);
        }
        // Cancel.
        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.command_input.clear();
            app.mode = Mode::Normal;
        }
        // Clear the whole line.
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.command_input.clear();
        }
        (KeyCode::Backspace, _) => {
            app.command_input.pop();
        }
        (KeyCode::Char(c), mods) if !mods.contains(KeyModifiers::CONTROL) => {
            app.command_input.push(c);
        }
        _ => {}
    }

    Ok(())
}
