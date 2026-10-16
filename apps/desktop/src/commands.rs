//! One command list supplies palette labels and actual keyboard bindings.
use crate::{
    lens::Since,
    panes::Direction,
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
    SplitDown,
    ClosePane,
    MaximizePane,
    /// Moves focus to the pane on one side.
    FocusPane(Direction),
    /// Moves the focused tab to the pane on one side.
    MoveTab(Direction),
    NextTab,
    PreviousTab,
    /// Text size in the focused pane.
    ZoomIn,
    ZoomOut,
    ZoomReset,
    NewFile,
    /// The pet: into a window of its own and back, or out of sight.
    PetPopOut,
    PetShow,
    Conversation,
    StopSession,
    NewSession,
    NewWorktreeSession,
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
    StageChange,
    AskAboutChange,
    MarkChangeSeen,
    ShowFormatting,
    OpenSettings,
    QuickOpen,
    FindInFiles,
    Explorer,
    Terminal,
    NewTerminal,
    ColorTheme,
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
    Command::SplitDown,
    Command::ClosePane,
    Command::MaximizePane,
    Command::FocusPane(Direction::Left),
    Command::FocusPane(Direction::Right),
    Command::FocusPane(Direction::Up),
    Command::FocusPane(Direction::Down),
    Command::MoveTab(Direction::Left),
    Command::MoveTab(Direction::Right),
    Command::MoveTab(Direction::Up),
    Command::MoveTab(Direction::Down),
    Command::NextTab,
    Command::PreviousTab,
    Command::ZoomIn,
    Command::ZoomOut,
    Command::ZoomReset,
    Command::NewFile,
    Command::PetPopOut,
    Command::PetShow,
    Command::Conversation,
    Command::StopSession,
    Command::NewSession,
    Command::NewWorktreeSession,
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
    Command::StageChange,
    Command::AskAboutChange,
    Command::MarkChangeSeen,
    Command::ShowFormatting,
    Command::OpenSettings,
    Command::Explorer,
    Command::Terminal,
    Command::NewTerminal,
    Command::ColorTheme,
    Command::MoveExplorer,
    Command::Refresh,
    Command::ResetLayout,
];

/// A key with Ctrl (Cmd on macOS), Alt and Shift as given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    key: &'static str,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

const fn ctrl(key: &'static str) -> Option<Shortcut> {
    Some(Shortcut {
        key,
        ctrl: true,
        shift: false,
        alt: false,
    })
}

const fn ctrl_shift(key: &'static str) -> Option<Shortcut> {
    Some(Shortcut {
        key,
        ctrl: true,
        shift: true,
        alt: false,
    })
}

const fn ctrl_alt(key: &'static str) -> Option<Shortcut> {
    Some(Shortcut {
        key,
        ctrl: true,
        shift: false,
        alt: true,
    })
}

const fn ctrl_alt_shift(key: &'static str) -> Option<Shortcut> {
    Some(Shortcut {
        key,
        ctrl: true,
        shift: true,
        alt: true,
    })
}

/// Arrow key names for a direction.
const fn arrow(direction: Direction) -> &'static str {
    match direction {
        Direction::Left => "left",
        Direction::Right => "right",
        Direction::Up => "up",
        Direction::Down => "down",
    }
}

const fn alt(key: &'static str) -> Option<Shortcut> {
    Some(Shortcut {
        key,
        ctrl: false,
        shift: false,
        alt: true,
    })
}

fn named_key(key: &str) -> Option<NamedKey> {
    Some(match key {
        "tab" => NamedKey::Tab,
        "down" => NamedKey::ArrowDown,
        "up" => NamedKey::ArrowUp,
        "left" => NamedKey::ArrowLeft,
        "right" => NamedKey::ArrowRight,
        "enter" => NamedKey::Enter,
        "escape" | "esc" => NamedKey::Escape,
        "delete" | "del" => NamedKey::Delete,
        "backspace" => NamedKey::Backspace,
        "insert" => NamedKey::Insert,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" => NamedKey::PageUp,
        "pagedown" => NamedKey::PageDown,
        "f1" => NamedKey::F1,
        "f2" => NamedKey::F2,
        "f3" => NamedKey::F3,
        "f4" => NamedKey::F4,
        "f5" => NamedKey::F5,
        "f6" => NamedKey::F6,
        "f7" => NamedKey::F7,
        "f8" => NamedKey::F8,
        "f9" => NamedKey::F9,
        "f10" => NamedKey::F10,
        "f11" => NamedKey::F11,
        "f12" => NamedKey::F12,
        _ => return None,
    })
}

/// The settings file's name for a command, such as "QuickOpen" or "Session(2)".
fn setting_name(command: Command) -> String {
    format!("{command:?}")
}

/// A shortcut from the settings file. Key names are interned so shortcuts
/// stay `Copy`; there are only ever a handful.
fn configured(text: &str) -> Option<Shortcut> {
    use std::{collections::HashSet, sync::Mutex};
    static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut keys = workbench_core::settings::parse_keys(text)?;
    if keys.key == "space" {
        keys.key = " ".into();
    }
    // A key must be one character or a key name this app knows.
    let named = named_key(&keys.key).is_some();
    if !named && keys.key.chars().count() != 1 {
        return None;
    }
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    let names = names.get_or_insert_with(HashSet::new);
    let key = match names.get(keys.key.as_str()) {
        Some(key) => *key,
        None => {
            let key: &'static str = Box::leak(keys.key.into_boxed_str());
            names.insert(key);
            key
        }
    };
    Some(Shortcut {
        key,
        ctrl: keys.ctrl,
        shift: keys.shift,
        alt: keys.alt,
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
            Self::Palette => ("Show all commands", ctrl_shift("p")),
            Self::Quit => ("Close workspace window", ctrl("q")),
            Self::OpenFolder => ("Open folder in new window", None),
            Self::Save => ("Save active file", ctrl("s")),
            Self::SaveAll => ("Save all files", None),
            Self::CloseFile => ("Close tab", ctrl("w")),
            Self::Reload => ("Reload active file from disk", None),
            Self::Split => ("Split right", ctrl("\\")),
            Self::SplitDown => ("Split down", ctrl_shift("\\")),
            Self::ClosePane => ("Close pane", None),
            Self::MaximizePane => ("Maximise or restore pane", ctrl_shift("m")),
            Self::FocusPane(direction) => (
                match direction {
                    Direction::Left => "Focus the pane to the left",
                    Direction::Right => "Focus the pane to the right",
                    Direction::Up => "Focus the pane above",
                    Direction::Down => "Focus the pane below",
                },
                ctrl_alt(arrow(direction)),
            ),
            Self::MoveTab(direction) => (
                match direction {
                    Direction::Left => "Move tab to the left",
                    Direction::Right => "Move tab to the right",
                    Direction::Up => "Move tab up",
                    Direction::Down => "Move tab down",
                },
                ctrl_alt_shift(arrow(direction)),
            ),
            Self::NextTab => ("Next tab in the pane", ctrl("pagedown")),
            Self::PreviousTab => ("Previous tab in the pane", ctrl("pageup")),
            Self::ZoomIn => ("Zoom in (this pane)", ctrl("=")),
            Self::ZoomOut => ("Zoom out (this pane)", ctrl("-")),
            Self::ZoomReset => ("Reset zoom (this pane)", ctrl("0")),
            Self::NewFile => ("New file…", None),
            Self::PetPopOut => ("Pet: pop the front session's pet out, or back in", None),
            Self::PetShow => ("Pet: show or hide", None),
            Self::Conversation => ("Show Pi session", ctrl("j")),
            Self::StopSession => ("Stop Pi in this session", None),
            Self::NewSession => ("New Pi session", ctrl("n")),
            Self::NewWorktreeSession => ("New Pi session in its own worktree", None),
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
            Self::StageChange => ("Stage this change", None),
            Self::AskAboutChange => ("Ask Pi about this change", None),
            Self::MarkChangeSeen => ("Mark this change seen", None),
            Self::ShowFormatting => ("Show or hide formatting-only changes", None),
            Self::OpenSettings => ("Open settings file", ctrl(",")),
            Self::QuickOpen => ("Open file by name", ctrl("p")),
            Self::FindInFiles => ("Search in files", ctrl_shift("f")),
            Self::Explorer => ("Toggle explorer", ctrl("b")),
            Self::Terminal => ("Terminal, or back to where you were", ctrl("`")),
            Self::NewTerminal => ("New terminal", ctrl_shift("`")),
            Self::ColorTheme => ("Colour theme…", ctrl("t")),
            Self::MoveExplorer => ("Move explorer to opposite side", None),
            Self::Refresh => ("Refresh expanded folders", None),
            Self::ResetLayout => ("Reset panel layout", None),
        }
    }

    pub fn title(self) -> &'static str {
        self.details().0
    }
    /// The settings file can change or remove the built-in shortcut.
    pub fn shortcut(self) -> Option<Shortcut> {
        self.shortcut_in(&crate::settings::get().keys)
    }

    /// `""` removes the shortcut; one that cannot be read keeps the default.
    fn shortcut_in(self, keys: &std::collections::BTreeMap<String, String>) -> Option<Shortcut> {
        match keys.get(&setting_name(self)) {
            Some(text) if text.trim().is_empty() => None,
            Some(text) => configured(text).or(self.details().1),
            None => self.details().1,
        }
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
        Command::Split => state.split(Direction::Right),
        Command::SplitDown => state.split(Direction::Down),
        Command::ClosePane => state.close_pane(state.focused().get_untracked()),
        Command::MaximizePane => state.toggle_maximized(),
        Command::FocusPane(direction) => state.focus_direction(direction),
        Command::MoveTab(direction) => state.move_tab(direction),
        Command::NextTab => state.cycle_tab(true),
        Command::PreviousTab => state.cycle_tab(false),
        Command::ZoomIn => state.zoom(state.focused().get_untracked(), 1),
        Command::ZoomOut => state.zoom(state.focused().get_untracked(), -1),
        Command::ZoomReset => state.zoom(state.focused().get_untracked(), 0),
        Command::NewFile => state.new_file(),
        Command::PetPopOut => state.pets.toggle_out(),
        Command::PetShow => state.pets.toggle_hidden(),
        Command::Conversation => {
            state.show_conversation();
        }
        Command::StopSession => {
            if let Some(session) = state.sessions.active_untracked() {
                session.stop();
            }
        }
        Command::NewSession => state.new_session(),
        Command::NewWorktreeSession => state.new_worktree_session(),
        Command::CloseSession => {
            if let Some(session) = state.sessions.active_untracked() {
                state.close_session(session.key);
            }
        }
        Command::NextSession | Command::PreviousSession | Command::Session(_) => {
            match command {
                Command::NextSession => state.sessions.cycle(true),
                Command::PreviousSession => state.sessions.cycle(false),
                Command::Session(n) => state.sessions.select_index(usize::from(n) - 1),
                _ => {}
            }
            // The session switched to comes into view.
            if let Some(session) = state.sessions.active_untracked() {
                state.show_session(session.key);
            }
        }
        Command::Since(since) => state.lens.set_since(since),
        Command::ChangedOnly => state.lens.toggle_changed_only(),
        Command::Focus => state.lens.toggle_focus(),
        Command::NextChange => state.next_change(true),
        Command::PreviousChange => state.next_change(false),
        Command::RevertChange => state.revert_change(),
        Command::StageChange => state.stage_change(),
        Command::AskAboutChange => state.ask_about_change(),
        Command::MarkChangeSeen => state.mark_change_seen(),
        Command::ShowFormatting => state.lens.toggle_formatting(),
        Command::OpenSettings => state.open_settings(),
        Command::QuickOpen => state.open_finder(Finder::Files),
        Command::FindInFiles => state.open_finder(Finder::Search),
        Command::Explorer => state.toggle_explorer(),
        Command::Terminal => state.show_terminal(),
        Command::NewTerminal => state.new_terminal_here(),
        Command::ColorTheme => state.open_finder(Finder::Themes),
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
        let mut modifiers = Modifiers::empty();
        if self.ctrl {
            // Cmd+Tab is the macOS app switcher, so session cycling uses Ctrl everywhere.
            modifiers |= if self.key == "tab" {
                Modifiers::CONTROL
            } else {
                primary_modifier()
            };
        }
        if self.alt {
            modifiers |= Modifiers::ALT;
        }
        if self.shift {
            modifiers |= Modifiers::SHIFT;
        }
        modifiers
    }

    fn named(self) -> Option<NamedKey> {
        named_key(self.key)
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
            "pagedown" => "PageDown".to_string(),
            "pageup" => "PageUp".to_string(),
            key if self.named().is_some() => {
                let mut chars = key.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().chain(chars).collect()
                })
            }
            key => key.to_uppercase(),
        };
        let mut label = String::new();
        if self.ctrl {
            label.push_str(if self.key == "tab" {
                "Ctrl+"
            } else {
                "Ctrl/Cmd+"
            });
        }
        if self.alt {
            label.push_str("Alt+");
        }
        if self.shift {
            label.push_str("Shift+");
        }
        label + &key
    }
}

/// Shortcuts from the settings file win over built-in ones they collide with.
pub fn for_key(key: &Key, modifiers: Modifiers) -> Option<Command> {
    // "+" zooms in as "=" does, whether it takes Shift or is on the keypad.
    if let Key::Character(c) = key
        && c == "+"
    {
        return for_key(&Key::Character("=".into()), modifiers - Modifiers::SHIFT);
    }
    let settings = crate::settings::get();
    let overridden = |command: &Command| settings.keys.contains_key(&setting_name(*command));
    let matching = |command: &Command| {
        command
            .shortcut_in(&settings.keys)
            .is_some_and(|binding| binding.matches(key, modifiers))
    };
    let commands = || COMMANDS.iter().copied();
    commands()
        .filter(overridden)
        .find(matching)
        .or_else(|| commands().filter(|c| !overridden(c)).find(matching))
}

/// Problems with the settings file's shortcuts, to report rather than ignore.
pub fn check_keys(keys: &std::collections::BTreeMap<String, String>) -> Option<String> {
    let problems: Vec<String> = keys
        .iter()
        .filter_map(|(name, text)| {
            let known = COMMANDS.iter().any(|c| setting_name(*c) == *name);
            if !known {
                Some(format!("no command named {name:?}"))
            } else if !text.trim().is_empty() && configured(text).is_none() {
                Some(format!("cannot read the shortcut {text:?} for {name}"))
            } else {
                None
            }
        })
        .collect();
    (!problems.is_empty()).then(|| problems.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_shortcuts_win_and_bad_ones_are_reported() {
        use std::collections::BTreeMap;
        let keys = BTreeMap::from([
            ("Terminal".to_owned(), "ctrl+j".to_owned()),
            ("QuickOpen".to_owned(), "ctr+o".to_owned()),
            ("Nope".to_owned(), "ctrl+k".to_owned()),
            ("Save".to_owned(), String::new()),
        ]);
        assert_eq!(Command::QuickOpen.shortcut_in(&keys), ctrl("p"));
        assert_eq!(Command::Save.shortcut_in(&keys), None);
        assert_eq!(
            Command::Terminal.shortcut_in(&keys).map(Shortcut::label),
            Some("Ctrl/Cmd+J".to_owned())
        );
        let report = check_keys(&keys).unwrap();
        assert!(report.contains("\"Nope\"") && report.contains("ctr+o"));
        assert!(!report.contains("Save"));
        assert!(configured("ctrl+space").is_some() && configured("alt+esc").is_some());
        assert!(configured("ctrl+banana").is_none());
    }

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
        for shift in [Modifiers::empty(), Modifiers::SHIFT] {
            assert_eq!(
                for_key(&Key::Character("+".into()), primary_modifier() | shift),
                Some(Command::ZoomIn)
            );
        }
    }
}
