//! One command list supplies palette labels and actual keyboard bindings.
use crate::workspace::Workbench;
use floem::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    OpenFolder,
    Save,
    SaveAll,
    CloseFile,
    Reload,
    Split,
    Changes,
    Conversation,
    StopSession,
    NewSession,
    CloseSession,
    NextSession,
    PreviousSession,
    /// Ctrl+1..9 bring the nth open session to the front.
    Session(u8),
    Explorer,
    MoveExplorer,
    Refresh,
    ResetLayout,
    Quit,
    Palette,
}

pub const COMMANDS: &[Command] = &[
    Command::Palette,
    Command::Quit,
    Command::OpenFolder,
    Command::Save,
    Command::SaveAll,
    Command::CloseFile,
    Command::Reload,
    Command::Split,
    Command::Changes,
    Command::Conversation,
    Command::StopSession,
    Command::NewSession,
    Command::CloseSession,
    Command::NextSession,
    Command::PreviousSession,
    Command::Session(1),
    Command::Session(2),
    Command::Session(3),
    Command::Session(4),
    Command::Session(5),
    Command::Session(6),
    Command::Session(7),
    Command::Session(8),
    Command::Session(9),
    Command::Explorer,
    Command::MoveExplorer,
    Command::Refresh,
    Command::ResetLayout,
];

impl Command {
    fn details(self) -> (&'static str, Option<Shortcut>) {
        let primary = |key| Some(Shortcut { key, shift: false });
        const SESSIONS: [&str; 9] = [
            "Go to session 1",
            "Go to session 2",
            "Go to session 3",
            "Go to session 4",
            "Go to session 5",
            "Go to session 6",
            "Go to session 7",
            "Go to session 8",
            "Go to session 9",
        ];
        const DIGITS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
        match self {
            Self::Palette => (
                "Toggle command palette",
                Some(Shortcut {
                    key: "p",
                    shift: true,
                }),
            ),
            Self::Quit => ("Close workspace window", primary("q")),
            Self::OpenFolder => ("Open folder in new window", None),
            Self::Save => ("Save active file", primary("s")),
            Self::SaveAll => ("Save all files", None),
            Self::CloseFile => ("Close active file", primary("w")),
            Self::Reload => ("Reload active file from disk", None),
            Self::Split => ("Toggle second editor pane", primary("\\")),
            Self::Changes => (
                "Toggle changes since last commit",
                Some(Shortcut {
                    key: "g",
                    shift: true,
                }),
            ),
            Self::Conversation => ("Show conversation", primary("j")),
            Self::StopSession => ("Stop Pi in this session", None),
            Self::NewSession => ("New Pi session", primary("n")),
            Self::CloseSession => ("Close session", None),
            Self::NextSession => ("Next session", primary("tab")),
            Self::PreviousSession => (
                "Previous session",
                Some(Shortcut {
                    key: "tab",
                    shift: true,
                }),
            ),
            Self::Session(n) => {
                let index = usize::from(n.clamp(1, 9)) - 1;
                (SESSIONS[index], primary(DIGITS[index]))
            }
            Self::Explorer => ("Toggle explorer", primary("b")),
            Self::MoveExplorer => ("Move explorer to opposite side", None),
            Self::Refresh => ("Refresh expanded folders", None),
            Self::ResetLayout => ("Reset panel layout", None),
        }
    }

    pub fn title(self) -> &'static str {
        self.details().0
    }
    pub fn shortcut(self) -> Option<Shortcut> {
        self.details().1
    }
    pub fn shortcut_label(self) -> String {
        self.shortcut().map(Shortcut::label).unwrap_or_default()
    }
}

pub fn execute(state: &Workbench, command: Command) {
    match command {
        Command::OpenFolder => crate::open_folder(),
        Command::Save => state.save_active(),
        Command::SaveAll => state.documents.save_all(),
        Command::CloseFile => state.close_active(),
        Command::Reload => state.reload_active(),
        Command::Split => state.toggle_split(),
        Command::Changes => state.toggle_changes(),
        Command::Conversation => state.show_conversation(),
        Command::StopSession => {
            if let Some(session) = state.sessions.active_untracked() {
                session.stop();
            }
        }
        Command::NewSession => {
            state.sessions.create();
            state.show_conversation();
        }
        Command::CloseSession => {
            if let Some(session) = state.sessions.active_untracked() {
                state.sessions.close(session.key);
            }
        }
        Command::NextSession => state.sessions.cycle(true),
        Command::PreviousSession => state.sessions.cycle(false),
        Command::Session(n) => state.sessions.select_index(usize::from(n) - 1),
        Command::Explorer => state.toggle_explorer(),
        Command::MoveExplorer => state.move_explorer(),
        Command::Refresh => state.explorer.refresh(),
        Command::ResetLayout => state.reset_layout(),
        Command::Quit => state.request_quit(),
        Command::Palette => state.toggle_palette(),
    }
}

/// All bindings use Ctrl on Windows/Linux and Cmd on macOS.
#[derive(Clone, Copy)]
pub struct Shortcut {
    key: &'static str,
    shift: bool,
}

fn primary_modifier() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::META
    } else {
        Modifiers::CONTROL
    }
}

impl Shortcut {
    fn modifiers(self) -> Modifiers {
        // Cmd+Tab is the macOS app switcher, so session cycling uses Ctrl everywhere.
        let base = if self.key == "tab" {
            Modifiers::CONTROL
        } else {
            primary_modifier()
        };
        base | if self.shift {
            Modifiers::SHIFT
        } else {
            Modifiers::empty()
        }
    }

    fn matches(self, key: &Key, modifiers: Modifiers) -> bool {
        // Ignore lock keys, but do not consume unrelated Ctrl+Alt/Shift combinations.
        let relevant =
            modifiers & (Modifiers::CONTROL | Modifiers::META | Modifiers::ALT | Modifiers::SHIFT);
        let key_matches = match key {
            Key::Named(NamedKey::Tab) => self.key == "tab",
            Key::Character(key) => key.eq_ignore_ascii_case(self.key),
            _ => false,
        };
        key_matches && relevant == self.modifiers()
    }

    fn label(self) -> String {
        if self.key == "tab" {
            return format!("Ctrl+{}Tab", if self.shift { "Shift+" } else { "" });
        }
        format!(
            "Ctrl/Cmd+{}{}",
            if self.shift { "Shift+" } else { "" },
            self.key.to_uppercase()
        )
    }
}

pub fn for_key(key: &Key, modifiers: Modifiers) -> Option<Command> {
    COMMANDS.iter().copied().find(|command| {
        command
            .shortcut()
            .is_some_and(|binding| binding.matches(key, modifiers))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_use_the_registry_and_reject_extra_modifiers() {
        for command in COMMANDS {
            if let Some(binding) = command.shortcut() {
                let key = if binding.key == "tab" {
                    Key::Named(NamedKey::Tab)
                } else {
                    Key::Character(binding.key.to_uppercase())
                };
                assert_eq!(for_key(&key, binding.modifiers()), Some(*command));
            }
        }
        assert_eq!(
            for_key(
                &Key::Character("s".into()),
                primary_modifier() | Modifiers::ALT
            ),
            None
        );
        assert_eq!(
            for_key(
                &Key::Character("s".into()),
                primary_modifier() | Modifiers::SHIFT
            ),
            None
        );
        assert_eq!(
            for_key(&Key::Character("p".into()), primary_modifier()),
            None
        );
        assert_eq!(
            for_key(
                &Key::Character("P".into()),
                primary_modifier() | Modifiers::SHIFT
            ),
            Some(Command::Palette)
        );
    }
}
