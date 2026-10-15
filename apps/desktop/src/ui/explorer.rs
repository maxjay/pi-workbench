//! The explorer, read through the changes lens: changed files stand out with
//! their line counts, the rest fade, and the tree can collapse to just the changes.
use super::{CODE_FONT, color, command_button};
use crate::{commands::Command, lens::TurnFiles, workspace::Workbench};
use floem::{prelude::*, style::CursorStyle};
use std::{path::PathBuf, rc::Rc};
use workbench_core::git::{FileChange, FileStatus};

fn counts(change: &FileChange) -> String {
    if change.binary {
        return "binary".into();
    }
    match change.status {
        FileStatus::Added if change.removed == 0 => format!("+{}", change.added),
        FileStatus::Deleted => format!("−{}", change.removed),
        _ => format!("+{} −{}", change.added, change.removed),
    }
}

fn status_color(status: FileStatus) -> Color {
    match status {
        FileStatus::Added => color::added_text(),
        FileStatus::Deleted => color::removed_text(),
        FileStatus::Modified => color::accent(),
    }
}

fn tree(state: Rc<Workbench>) -> impl IntoView {
    let rows = state.clone();
    let make_row = state.clone();
    virtual_stack(
        move || {
            rows.explorer
                .rows()
                .into_iter()
                .collect::<floem::imbl::Vector<_>>()
        },
        |row| row.entry.path.clone(),
        move |row| {
            let action_state = make_row.clone();
            let render_state = make_row.clone();
            let changes = make_row.lens.changes();
            let path = row.entry.path;
            let (render_path, change_path) = (path.clone(), path.clone());
            let title = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let directory = row.entry.directory;
            // A file's own change, or how many changed files a folder holds.
            let change = move || {
                changes.with(|changes| {
                    if directory {
                        let inside = changes
                            .iter()
                            .filter(|c| c.path.starts_with(&change_path))
                            .count();
                        (inside > 0).then(|| (format!("{inside}"), color::accent()))
                    } else {
                        changes
                            .iter()
                            .find(|c| c.path == change_path)
                            .map(|c| (counts(c), status_color(c.status)))
                    }
                })
            };
            let (label_change, count_change, color_change) =
                (change.clone(), change.clone(), change);
            Stack::horizontal((
                Label::derived(move || {
                    let marker = if directory {
                        if render_state.explorer.is_expanded(&render_path) {
                            "− "
                        } else {
                            "+ "
                        }
                    } else {
                        "  "
                    };
                    format!("{marker}{title}")
                })
                .style(move |s| {
                    let changed = label_change().is_some();
                    s.flex_grow(1.0_f32).min_width(0).text_ellipsis().color(
                        if changed || directory {
                            color::text()
                        } else {
                            color::muted().with_alpha(0.75)
                        },
                    )
                }),
                Label::derived(move || count_change().map(|(c, _)| c).unwrap_or_default()).style(
                    move |s| {
                        let color = color_change()
                            .map(|(_, color)| color)
                            .unwrap_or(color::muted());
                        s.font_size(11)
                            .font_family(CODE_FONT.to_string())
                            .color(color)
                            .flex_shrink(0.0_f32)
                    },
                ),
            ))
            .style(move |s| {
                s.height(28)
                    .width_full()
                    .gap(6)
                    .items_center()
                    .padding_left((12 + row.depth * 14) as f64)
                    .padding_right(10)
                    .cursor(CursorStyle::Pointer)
                    .hover(|s| s.background(color::border()))
            })
            .on_event_stop(el::Click, move |_, _| {
                if directory {
                    action_state.explorer.toggle(path.clone());
                } else {
                    action_state.open(path.clone(), action_state.focused().get_untracked());
                }
            })
        },
    )
    .style(|s| s.flex_col().width_full())
    .scroll()
    .style(|s| s.size_full().min_height(0))
}

/// One row of the changed-files list.
#[derive(Clone)]
enum Row {
    /// A group: a Pi turn and its prompt, or the changes no turn made.
    Heading(String, String),
    File(FileChange),
}

/// The changed files, grouped by the Pi turn that changed them when the
/// active session has finished turns, newest first. Only files that still
/// differ from the baseline are listed, with their counts against it.
fn rows(changes: &[FileChange], turns: &[TurnFiles]) -> Vec<(usize, Row)> {
    if turns.is_empty() {
        return changes.iter().cloned().map(|c| (0, Row::File(c))).collect();
    }
    let mut rows = Vec::new();
    let mut by_turn = std::collections::HashSet::new();
    for (group, turn) in turns.iter().rev().enumerate() {
        let files: Vec<&FileChange> = changes
            .iter()
            .filter(|change| turn.files.iter().any(|f| f.path == change.path))
            .collect();
        if files.is_empty() {
            continue;
        }
        let prompt = turn.prompt.lines().next().unwrap_or_default().trim();
        rows.push((
            group,
            Row::Heading(format!("Turn {}", turn.number), prompt.into()),
        ));
        for file in files {
            by_turn.insert(file.path.clone());
            rows.push((group, Row::File(file.clone())));
        }
    }
    let rest: Vec<_> = changes
        .iter()
        .filter(|c| !by_turn.contains(&c.path))
        .cloned()
        .collect();
    if !rest.is_empty() {
        let group = turns.len();
        rows.push((group, Row::Heading("Other changes".into(), String::new())));
        rows.extend(rest.into_iter().map(|c| (group, Row::File(c))));
    }
    rows
}

/// Every changed file, by path or by Pi turn.
fn changed_files(state: Rc<Workbench>) -> impl IntoView {
    let changes = state.lens.changes();
    let turns = state.lens.turn_files();
    let root = state.root().to_path_buf();
    let open_state = state.clone();
    Stack::vertical((
        Label::new("Nothing has changed").style(move |s| {
            s.color(color::muted())
                .padding(14)
                .apply_if(!changes.with(Vec::is_empty), |s| s.hide())
        }),
        dyn_stack(
            move || changes.with(|changes| turns.with(|turns| rows(changes, turns))),
            // Keyed by the whole row, so new counts rebuild it.
            |(group, row)| match row {
                Row::Heading(title, detail) => {
                    (*group, PathBuf::from(title), 0, 0, false, detail.clone())
                }
                Row::File(change) => (
                    *group,
                    change.path.clone(),
                    change.added,
                    change.removed,
                    change.binary,
                    String::new(),
                ),
            },
            move |(_, row)| {
                let change = match row {
                    Row::Heading(title, detail) => return heading(title, detail).into_any(),
                    Row::File(change) => change,
                };
                let state = open_state.clone();
                let relative = change
                    .path
                    .strip_prefix(&root)
                    .unwrap_or(&change.path)
                    .display()
                    .to_string();
                let (name, folder) = match relative.rsplit_once(['/', '\\']) {
                    Some((folder, name)) => (name.to_string(), folder.to_string()),
                    None => (relative.clone(), String::new()),
                };
                let path: PathBuf = change.path.clone();
                let deleted = change.status == FileStatus::Deleted;
                Stack::horizontal((
                    Label::new(name).style(move |s| {
                        s.color(color::text())
                            .flex_shrink(0.0_f32)
                            .apply_if(deleted, |s| s.color(color::removed_text()))
                    }),
                    Label::new(folder).style(|s| {
                        s.color(color::muted())
                            .font_size(11)
                            .flex_grow(1.0_f32)
                            .min_width(0)
                            .text_ellipsis()
                    }),
                    Label::new(counts(&change)).style(move |s| {
                        s.font_size(11)
                            .font_family(CODE_FONT.to_string())
                            .color(status_color(change.status))
                            .flex_shrink(0.0_f32)
                    }),
                ))
                .style(|s| {
                    s.height(28)
                        .width_full()
                        .gap(8)
                        .items_center()
                        .padding_horiz(12)
                        .cursor(CursorStyle::Pointer)
                        .hover(|s| s.background(color::border()))
                })
                .on_event_stop(el::Click, move |_, _| {
                    if !deleted {
                        state.open(path.clone(), state.focused().get_untracked());
                    }
                })
                .into_any()
            },
        )
        .style(|s| s.flex_col().width_full()),
    ))
    .style(|s| s.width_full())
    .scroll()
    .style(|s| s.size_full().min_height(0))
}

fn heading(title: String, detail: String) -> impl IntoView {
    Stack::horizontal((
        Label::new(title).style(|s| s.color(color::accent()).font_size(11).flex_shrink(0.0_f32)),
        Label::new(detail).style(|s| {
            s.color(color::muted())
                .font_size(11)
                .min_width(0)
                .flex_grow(1.0_f32)
                .text_ellipsis()
        }),
    ))
    .style(|s| {
        s.width_full()
            .gap(8)
            .items_center()
            .padding_horiz(12)
            .padding_top(12)
            .padding_bottom(4)
    })
}

pub(super) fn explorer(state: Rc<Workbench>) -> impl IntoView {
    let layout = state.layout();
    let changed_only = state.lens.changed_only();
    let files = Stack::new((
        tree(state.clone()).style(move |s| s.apply_if(changed_only.get(), |s| s.hide())),
        changed_files(state.clone()).style(move |s| s.apply_if(!changed_only.get(), |s| s.hide())),
    ))
    .style(|s| s.size_full().min_height(0));
    Stack::vertical((
        Stack::horizontal((
            Label::derived(move || {
                if changed_only.get() {
                    "CHANGED"
                } else {
                    "FILES"
                }
                .to_string()
            })
            .style(|s| s.color(color::muted()).font_size(11).flex_grow(1.0_f32)),
            command_button(state.clone(), "↔", Command::MoveExplorer),
            command_button(state, "Refresh", Command::Refresh),
        ))
        .style(|s| {
            s.width_full()
                .height(42)
                .padding_horiz(10)
                .gap(6)
                .items_center()
        }),
        files,
    ))
    .style(move |s| {
        s.width(layout.with(|l| l.explorer_width))
            .height_full()
            .min_height(0)
            .flex_shrink(0.0_f32)
            .background(color::panel())
            .apply_if(!layout.with(|l| l.explorer_visible), |s| s.hide())
    })
}
