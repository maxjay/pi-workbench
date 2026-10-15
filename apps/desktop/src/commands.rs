//! One command list supplies palette labels and actual keyboard bindings.
use crate::{
    lens::Since,
    workspace::{Finder, Workbench},
};
use floem::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    OpenFolder,
    Save,
    SaveAll,
    CloseFile,
    Reload,
    Split,
    Conversation,
    StopSession,
    NewSession,
    CloseSession,
    NextSession,
    PreviousSession,
    /// Ctrl+1..9 bring the nth open session to the front.
    Session(u8),
    /// What the changes lens compares against.
    Since(Since),
    ChangedOnly,
    Focus,
    NextChange,
    PreviousChange,
    RevertChange,
    AskAboutChange,
    MarkChangeSeen,
    QuickOpen,
    FindInFiles,
    Explorer,
    MoveExplorer,
    Refresh,
    ResetLayout,
    Quit,
    Palette,
}

pub const COMMANDS: &[Command] = &[
    Command::Palette,
    Command::QuickOpen,
    Command::FindInFiles,
    Command::Quit,
    Command::OpenFolder,
    Command::Save,
    Command::SaveAll,
    Command::CloseFile,
    Command::Reload,
    Command::Split,
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
    Command::Since(Since::Branch),
    Command::Since(Since::SessionStart),
    Command::Since(Since::LastTurn),
    Command::Since(Since::Head),
    Command::ChangedOnly,
    Command::Focus,
    Command::NextChange,
    Command::PreviousChange,
    Command::RevertChange,
    Command::AskAboutChange,
    Command::MarkChangeSeen,
    Command::Explorer,
    Command::MoveExplorer,
    Command::Refresh,
    Command::ResetLayout,
];

/// A key with Ctrl (Cmd on macOS), optionally Shift; or Alt alone.
#[derive(Clone, Copy)]
pub struct Shortcut {
    key: &'static str,
    shift: bool,
    alt: bool,
}

const fn ctrl(key: &'static str) -> Option<Shortcut> {
    Some(Shortcut {
        key,
        shift: false,
        alt: false,
    })
}

const fn ctrl_shift(key: &'static str) -> Option<Shortcut> {
    Some(Shortcut {
        key,
        shift: true,
        alt: false,
    })
}

const fn alt(key: &'static str) -> Option<Shortcut> {
    Some(Shortcut {
        key,
        shift: false,
        alt: true,
    })
}

impl Command {
    fn details(self) -> (&'static str, Option<Shortcut>) {
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
            Self::Palette => ("Toggle command palette", ctrl_shift("p")),
            Self::Quit => ("Close workspace window", ctrl("q")),
            Self::OpenFolder => ("Open folder in new window", None),
            Self::Save => ("Save active file", ctrl("s")),
            Self::SaveAll => ("Save all files", None),
            Self::CloseFile => ("Close active file", ctrl("w")),
            Self::Reload => ("Reload active file from disk", None),
            Self::Split => ("Toggle second editor pane", ctrl("\\")),
            Self::Conversation => ("Show conversation", ctrl("j")),
            Self::StopSession => ("Stop Pi in this session", None),
            Self::NewSession => ("New Pi session", ctrl("n")),
            Self::CloseSession => ("Close session", None),
            Self::NextSession => ("Next session", ctrl("tab")),
            Self::PreviousSession => ("Previous session", ctrl_shift("tab")),
            Self::Session(n) => {
                let index = usize::from(n.clamp(1, 9)) - 1;
                (SESSIONS[index], ctrl(DIGITS[index]))
            }
            Self::Since(Since::Branch) => ("Changes since base branch", None),
            Self::Since(Since::SessionStart) => ("Changes since session start", None),
            Self::Since(Since::LastTurn) => ("Changes since Pi's last turn", None),
            Self::Since(Since::Head) => ("Changes since last commit", None),
            Self::ChangedOnly => ("Show only changed files", ctrl_shift("g")),
            Self::Focus => ("Focus on changes", ctrl_shift("h")),
            Self::NextChange => ("Next change", alt("down")),
            Self::PreviousChange => ("Previous change", alt("up")),
            Self::RevertChange => ("Revert this change", None),
            Self::AskAboutChange => ("Ask Pi about this change", None),
            Self::MarkChangeSeen => ("Mark this change seen", None),
            Self::QuickOpen => ("Open file by name", ctrl("p")),
            Self::FindInFiles => ("Search in files", ctrl_shift("f")),
            Self::Explorer => ("Toggle explorer", ctrl("b")),
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

pub fn execute(state: &std::rc::Rc<Workbench>, command: Command) {
    match command {
        Command::OpenFolder => crate::open_folder(),
        Command::Save => state.save_active(),
        Command::SaveAll => state.documents.save_all(),
        Command::CloseFile => state.close_active(),
        Command::Reload => state.reload_active(),
        Command::Split => state.toggle_split(),
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
        Command::Since(since) => state.lens.set_since(since),
        Command::ChangedOnly => state.lens.toggle_changed_only(),
        Command::Focus => state.lens.toggle_focus(),
        Command::NextChange => state.next_change(true),
        Command::PreviousChange => state.next_change(false),
        Command::RevertChange => state.revert_change(),
        Command::AskAboutChange => state.ask_about_change(),
        Command::MarkChangeSeen => state.mark_change_seen(),
        Command::QuickOpen => state.open_finder(Finder::Files),
        Command::FindInFiles => state.open_finder(Finder::Search),
        Command::Explorer => state.toggle_explorer(),
        Command::MoveExplorer => state.move_explorer(),
        Command::Refresh => state.explorer.refresh(),
        Command::ResetLayout => state.reset_layout(),
        Command::Quit => state.request_quit(),
        Command::Palette => state.toggle_palette(),
    }
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
        let mut modifiers = if self.alt {
            Modifiers::ALT
        } else if self.key == "tab" {
            // Cmd+Tab is the macOS app switcher, so session cycling uses Ctrl everywhere.
            Modifiers::CONTROL
        } else {
            primary_modifier()
        };
        if self.shift {
            modifiers |= Modifiers::SHIFT;
        }
        modifiers
    }

    fn named(self) -> Option<NamedKey> {
        match self.key {
            "tab" => Some(NamedKey::Tab),
            "down" => Some(NamedKey::ArrowDown),
            "up" => Some(NamedKey::ArrowUp),
            _ => None,
        }
    }

    fn matches(self, key: &Key, modifiers: Modifiers) -> bool {
        // Ignore lock keys, but do not consume unrelated Ctrl+Alt/Shift combinations.
        let relevant =
            modifiers & (Modifiers::CONTROL | Modifiers::META | Modifiers::ALT | Modifiers::SHIFT);
        let key_matches = match (key, self.named()) {
            (Key::Named(named), Some(expected)) => *named == expected,
            (Key::Character(key), None) => key.eq_ignore_ascii_case(self.key),
            _ => false,
        };
        key_matches && relevant == self.modifiers()
    }

    fn label(self) -> String {
        let key = match self.key {
            "tab" => "Tab".to_string(),
            "down" => "Down".to_string(),
            "up" => "Up".to_string(),
            key => key.to_uppercase(),
        };
        let held = if self.alt {
            "Alt"
        } else if self.key == "tab" {
            "Ctrl"
        } else {
            "Ctrl/Cmd"
        };
        format!("{held}+{}{key}", if self.shift { "Shift+" } else { "" })
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
                let key = match binding.named() {
                    Some(named) => Key::Named(named),
                    None => Key::Character(binding.key.to_uppercase()),
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
            Some(Command::QuickOpen)
        );
        assert_eq!(
            for_key(
                &Key::Character("P".into()),
                primary_modifier() | Modifiers::SHIFT
            ),
            Some(Command::Palette)
        );
        assert_eq!(
            for_key(&Key::Named(NamedKey::ArrowDown), Modifiers::ALT),
            Some(Command::NextChange)
        );
        assert_eq!(
            for_key(&Key::Named(NamedKey::ArrowDown), Modifiers::empty()),
            None
        );
    }
}
