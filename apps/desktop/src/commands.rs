use crate::state::{Confirm, Workbench};
use floem::prelude::*;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    OpenFolder,
    Save,
    SaveAll,
    CloseFile,
    Reload,
    Split,
    Explorer,
    MoveExplorer,
    Refresh,
    ResetLayout,
    Quit,
}

/// The toolbar and command palette use the same internal contribution surface.
pub struct CommandSpec {
    pub id: Command,
    pub title: &'static str,
    pub shortcut: &'static str,
}

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: Command::Quit,
        title: "Close workspace window",
        shortcut: "Ctrl/Cmd+Q",
    },
    CommandSpec {
        id: Command::OpenFolder,
        title: "Open folder in new window",
        shortcut: "",
    },
    CommandSpec {
        id: Command::Save,
        title: "Save active file",
        shortcut: "Ctrl/Cmd+S",
    },
    CommandSpec {
        id: Command::SaveAll,
        title: "Save all files",
        shortcut: "",
    },
    CommandSpec {
        id: Command::CloseFile,
        title: "Close active file",
        shortcut: "Ctrl/Cmd+W",
    },
    CommandSpec {
        id: Command::Reload,
        title: "Reload active file from disk",
        shortcut: "",
    },
    CommandSpec {
        id: Command::Split,
        title: "Toggle second editor pane",
        shortcut: "Ctrl/Cmd+\\",
    },
    CommandSpec {
        id: Command::Explorer,
        title: "Toggle explorer",
        shortcut: "Ctrl/Cmd+B",
    },
    CommandSpec {
        id: Command::MoveExplorer,
        title: "Move explorer to opposite side",
        shortcut: "",
    },
    CommandSpec {
        id: Command::Refresh,
        title: "Refresh expanded folders",
        shortcut: "",
    },
    CommandSpec {
        id: Command::ResetLayout,
        title: "Reset panel layout",
        shortcut: "",
    },
];

pub fn execute(state: &Rc<Workbench>, command: Command) {
    match command {
        Command::Quit => state.request_quit(),
        Command::OpenFolder => {
            floem::open_file(
                floem::FileDialogOptions::new()
                    .select_directories()
                    .title("Open workspace"),
                |selected| {
                    if let Some(info) = selected
                        && let Some(path) = info.path.first()
                    {
                        match crate::workspace_paths(path) {
                            Ok((root, layout)) => floem::new_window(
                                move |id| crate::ui::workbench(id, root, layout),
                                Some(crate::window_config()),
                            ),
                            Err(err) => eprintln!("Cannot open workspace: {err}"),
                        }
                    }
                },
            );
        }
        Command::Save => state.save_active(),
        Command::SaveAll => state.save_all(),
        Command::CloseFile => {
            if let Some(path) = state.active[state.focused.get_untracked()].get_untracked() {
                state.close_file(path);
            }
        }
        Command::Reload => {
            if let Some(path) = state.active[state.focused.get_untracked()].get_untracked() {
                if state.document(&path).is_some_and(|doc| doc.dirty()) {
                    state.confirm.set(Some(Confirm::Reload(path)));
                } else {
                    state.reload(path);
                }
            }
        }
        Command::Split => {
            state.layout.update(|layout| layout.split = !layout.split);
            if !state.layout.get_untracked().split {
                state.focused.set(0);
            } else if state.active[1].get_untracked().is_none() {
                state.active[1].set(state.active[0].get_untracked());
            }
        }
        Command::Explorer => state
            .layout
            .update(|layout| layout.explorer_visible = !layout.explorer_visible),
        Command::MoveExplorer => state
            .layout
            .update(|layout| layout.explorer_right = !layout.explorer_right),
        Command::Refresh => {
            state.read_directory(state.root.clone());
            for path in state.expanded.get_untracked() {
                state.read_directory(path);
            }
        }
        Command::ResetLayout => {
            state.layout.set(Default::default());
            state.focused.set(0);
        }
    }
}
