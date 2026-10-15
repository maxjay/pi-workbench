//! The quick picker, as in VS Code: files by name (Ctrl+P), commands
//! (Ctrl+Shift+P, or `>` in the file picker), Pi's slash commands (`/`),
//! colour themes with a live preview, and search in files.
use super::{CAPTURE, CODE_FONT, color};
use crate::{
    commands::{self, COMMANDS, Command},
    workspace::{Finder, Workbench},
};
use floem::{
    event::EventPropagation,
    ext_event::create_ext_action,
    prelude::*,
    reactive::{Effect, Memo, Scope},
    style::CursorStyle,
    views::{PlaceholderTextClass, TextInput},
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};
use workbench_text::{
    index::{FileMatcher, list_files},
    search::{Hit, Query, search},
};

const FILE_LIMIT: usize = 50_000;
const SHOWN: usize = 60;
const HIT_LIMIT: usize = 500;

/// What choosing a row does.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Pick {
    File(PathBuf, Option<usize>),
    Command(Command),
    /// A Pi slash command, by name without the slash.
    Pi(String),
    /// A theme by name, or the path of a `.tmTheme` file.
    Theme(String),
    /// Creates the file typed, or opens it if it exists.
    NewFile(String),
}

/// One row: what it does, and what it shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Item {
    pick: Pick,
    title: String,
    detail: String,
    /// Right-aligned: a shortcut, where a command comes from, or "current".
    hint: String,
}

fn row(
    item: Item,
    index: usize,
    selected: RwSignal<usize>,
    choose: Rc<dyn Fn(Item)>,
) -> impl IntoView {
    let picked = item.clone();
    let view = Stack::horizontal((
        Label::new(item.title).style(|s| s.color(color::text()).flex_shrink(0.0_f32)),
        Label::new(item.detail).style(|s| {
            s.color(color::muted())
                .font_size(12)
                .min_width(0)
                .flex_grow(1.0_f32)
                .text_ellipsis()
                .font_family(CODE_FONT.to_string())
        }),
        Label::new(item.hint).style(|s| s.color(color::muted()).font_size(11).flex_shrink(0.0_f32)),
    ))
    .style(move |s| {
        s.width_full()
            .gap(10)
            .padding_horiz(12)
            .padding_vert(6)
            .items_center()
            .cursor(CursorStyle::Pointer)
            .hover(|s| s.background(color::border()))
            .apply_if(selected.get() == index, |s| {
                s.background(color::highlight())
            })
    })
    .on_event_stop(el::Click, move |_, _| choose(picked.clone()));
    // The selected row stays in view as the arrows move it.
    let id = view.view_id();
    Effect::new(move |_| {
        if selected.get() == index {
            id.scroll_to(None);
        }
    });
    view
}

/// Whether every word of `query` appears in `text`, ignoring case.
fn matches(query: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word))
}

fn source_label(source: &str) -> String {
    match source {
        "extension" => "Pi extension".into(),
        "prompt" => "Pi prompt".into(),
        "skill" => "Pi skill".into(),
        other => format!("Pi {other}"),
    }
}

/// `.tmTheme` files put in the `themes` folder beside the settings file.
fn theme_files() -> Vec<PathBuf> {
    let Some(dir) = workbench_core::settings::path()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("themes")))
    else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("tmtheme"))
        })
        .collect();
    files.sort();
    files
}

pub(super) fn finder(state: Rc<Workbench>) -> impl IntoView {
    let mode = state.finder();
    let query = RwSignal::new(String::new());
    let selected = RwSignal::new(0usize);
    let files = RwSignal::new(Vec::<PathBuf>::new());
    let hits = RwSignal::new(Vec::<Hit>::new());
    let message = RwSignal::new(String::new());
    // The query the current hits answer; Enter searches again once it differs.
    let searched = RwSignal::new(String::new());
    let matcher = Rc::new(RefCell::new(FileMatcher::new()));
    let scope = Scope::current();
    // Bumped on every opening and search, so late background results from an
    // earlier one are dropped.
    let generation = Rc::new(Cell::new(0u64));
    // Text to keep when typing `>` or `/` moves the file picker to commands.
    let carried: Rc<RefCell<Option<String>>> = Rc::default();

    let root = state.root().to_path_buf();
    let open_generation = generation.clone();
    let open_carried = carried.clone();
    let open_state = state.clone();
    Effect::new(move |_| {
        let current = mode.get();
        open_generation.set(open_generation.get() + 1);
        // Setting the query runs the effect that picks the selection, so the
        // selection is not touched after it.
        query.set(
            open_carried
                .borrow_mut()
                .take()
                .or_else(|| open_state.take_prefill())
                .unwrap_or_default(),
        );
        hits.set(Vec::new());
        files.set(Vec::new());
        searched.set(String::new());
        message.set(String::new());
        match current {
            // Opening lists files off the UI thread, so names are ready to match.
            Some(Finder::Files) => {
                let (check, expected) = (open_generation.clone(), open_generation.get());
                let finish = create_ext_action(scope, move |list| {
                    if check.get() == expected {
                        files.set(list)
                    }
                });
                let root = root.clone();
                std::thread::spawn(move || finish(list_files(&root, FILE_LIMIT)));
            }
            // Pi extensions can add commands while it runs.
            Some(Finder::Commands) => {
                if let Some(session) = open_state.sessions.active_untracked() {
                    session.refresh_commands();
                }
            }
            _ => {}
        }
    });

    let item_state = state.clone();
    let items = Memo::new({
        let matcher = matcher.clone();
        move |_| -> Vec<Item> {
            let text = query.get();
            match mode.get() {
                Some(Finder::Files) => files.with(|files| {
                    matcher
                        .borrow_mut()
                        .rank(&text, files, SHOWN)
                        .into_iter()
                        .map(|(path, _)| Item {
                            title: path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            detail: path
                                .parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default(),
                            hint: String::new(),
                            pick: Pick::File(path, None),
                        })
                        .collect()
                }),
                Some(Finder::Search) => hits.with(|hits| {
                    hits.iter()
                        .map(|hit| Item {
                            pick: Pick::File(hit.path.clone(), Some(hit.line)),
                            title: format!("{}:{}", hit.path.display(), hit.line + 1),
                            detail: hit.text.trim().to_string(),
                            hint: String::new(),
                        })
                        .collect()
                }),
                Some(Finder::Commands) => {
                    // `/` narrows to Pi's commands.
                    let (pi_only, words) = match text.strip_prefix('/') {
                        Some(rest) => (true, rest),
                        None => (false, text.as_str()),
                    };
                    let own = COMMANDS
                        .iter()
                        .filter(|_| !pi_only)
                        .filter(|c| !matches!(c, Command::Palette))
                        .filter(|c| matches(words, c.title()))
                        .map(|c| Item {
                            pick: Pick::Command(*c),
                            title: c.title().to_owned(),
                            detail: String::new(),
                            hint: c.shortcut_label(),
                        });
                    let pi: Vec<Item> = item_state
                        .sessions
                        .active()
                        .map(|session| {
                            session.commands().with(|commands| {
                                commands
                                    .iter()
                                    .filter(|c| {
                                        matches(words, &format!("/{} {}", c.name, c.description))
                                    })
                                    .map(|c| Item {
                                        pick: Pick::Pi(c.name.clone()),
                                        title: format!("/{}", c.name),
                                        detail: c.description.clone(),
                                        hint: source_label(&c.source),
                                    })
                                    .collect()
                            })
                        })
                        .unwrap_or_default();
                    own.chain(pi).take(200).collect()
                }
                Some(Finder::NewFile) => match item_state.describe_new_file(&text) {
                    Ok((path, exists)) => vec![Item {
                        pick: Pick::NewFile(text.clone()),
                        title: if exists { "Open" } else { "Create" }.into(),
                        detail: path.display().to_string(),
                        hint: if exists {
                            "exists".into()
                        } else {
                            "Enter".into()
                        },
                    }],
                    Err(problem) => vec![Item {
                        pick: Pick::NewFile(text.clone()),
                        title: problem,
                        detail: String::new(),
                        hint: String::new(),
                    }],
                },
                Some(Finder::Themes) => {
                    let chosen = color::chosen_name();
                    let named = workbench_text::theme::names()
                        .into_iter()
                        .map(|name| (name.clone(), name));
                    let files = theme_files().into_iter().map(|path| {
                        let name = path
                            .file_stem()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        (name, path.to_string_lossy().into_owned())
                    });
                    named
                        .chain(files)
                        .filter(|(name, _)| matches(&text, name))
                        .map(|(name, load)| Item {
                            hint: if name == chosen {
                                "current".into()
                            } else {
                                String::new()
                            },
                            detail: String::new(),
                            title: name,
                            pick: Pick::Theme(load),
                        })
                        .collect()
                }
                None => Vec::new(),
            }
        }
    });

    // Themes preview as the selection moves.
    let preview_state = state.clone();
    Effect::new(move |_| {
        if mode.get() != Some(Finder::Themes) {
            return;
        }
        let picked = items.with(|items| items.get(selected.get()).map(|i| i.pick.clone()));
        if let Some(Pick::Theme(name)) = picked {
            match color::load(&name) {
                Ok(theme) => color::preview(theme),
                Err(err) => preview_state.report(format!("{err:#}")),
            }
        }
    });

    // `>` switches the file picker to commands and `/` to Pi's commands, as
    // in VS Code; typing a new query moves the selection to the best match.
    let switch_state = state.clone();
    let switch_carried = carried.clone();
    Effect::new(move |_| {
        let text = query.get();
        if mode.get_untracked() == Some(Finder::Files)
            && let Some(first) = text.chars().next()
            && (first == '>' || first == '/')
        {
            let rest = if first == '>' {
                text[1..].to_owned()
            } else {
                text.clone()
            };
            *switch_carried.borrow_mut() = Some(rest);
            switch_state.open_finder(Finder::Commands);
            return;
        }
        // The theme list opens on the current theme; filtering picks the
        // first match, which then previews.
        let current = (mode.get_untracked() == Some(Finder::Themes) && text.is_empty())
            .then(|| {
                let chosen = color::chosen_name();
                let files = theme_files().into_iter().filter_map(|path| {
                    path.file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                });
                workbench_text::theme::names()
                    .into_iter()
                    .chain(files)
                    .position(|name| name.eq_ignore_ascii_case(&chosen))
            })
            .flatten();
        selected.set(current.unwrap_or(0));
    });

    let choose: Rc<dyn Fn(Item)> = {
        let state = state.clone();
        Rc::new(move |item: Item| match item.pick {
            Pick::File(path, line) => {
                let path = state.root().join(&path);
                state.dismiss_finder();
                match line {
                    Some(line) => state.open_at(path, line),
                    // A file opened by name keeps where it was left.
                    None => state.open(path, state.focused().get_untracked()),
                }
            }
            Pick::Command(command) => {
                state.dismiss_finder();
                commands::execute(&state, command);
            }
            Pick::Pi(name) => {
                state.dismiss_finder();
                state.start_pi_command(&name);
            }
            Pick::Theme(name) => {
                state.choose_theme(&name);
                state.dismiss_finder();
            }
            Pick::NewFile(text) => {
                if state.describe_new_file(&text).is_ok() {
                    state.dismiss_finder();
                    state.create_file(&text);
                }
            }
        })
    };

    let run_search = {
        let state = state.clone();
        move || {
            let text = query.get_untracked();
            if text.trim().is_empty() {
                return;
            }
            searched.set(text.clone());
            message.set("Searching…".into());
            generation.set(generation.get() + 1);
            let (check, expected) = (generation.clone(), generation.get());
            let finish =
                create_ext_action(
                    scope,
                    move |result: anyhow::Result<Vec<Hit>>| match result {
                        _ if check.get() != expected => {}
                        Ok(found) => {
                            message.set(match found.len() {
                                0 => "No matches".into(),
                                n if n >= HIT_LIMIT => format!("First {n} matches"),
                                n => format!("{n} matches"),
                            });
                            hits.set(found);
                            selected.set(0);
                        }
                        Err(err) => message.set(format!("{err:#}")),
                    },
                );
            let root = state.root().to_path_buf();
            std::thread::spawn(move || {
                let query = Query {
                    text,
                    ..Query::default()
                };
                finish(search(&root, &query, HIT_LIMIT))
            });
        }
    };

    let input = TextInput::new(query)
        .style(|s| {
            s.width_full()
                .padding(10)
                .background(color::bg())
                .color(color::text())
                .border(1)
                .border_color(color::border())
                .border_radius(6)
                .focus(|s| s.border_color(color::accent()))
                .class(PlaceholderTextClass, |s| s.color(color::muted()))
        })
        .placeholder("Type to search");
    let input_id = input.view_id();
    Effect::new(move |_| {
        if mode.get().is_some() {
            floem::action::exec_after_animation_frame(move |_| input_id.request_focus());
        }
    });

    let keys_choose = choose.clone();
    let list_choose = choose.clone();
    let list = dyn_stack(
        move || items.get().into_iter().enumerate().collect::<Vec<_>>(),
        |(index, item)| (*index, item.clone()),
        move |(index, item)| row(item, index, selected, list_choose.clone()),
    )
    .style(|s| s.flex_col().width_full())
    .scroll()
    .style(|s| s.width_full().max_height(420).flex_shrink(0.0_f32));

    let dismiss = state.clone();
    let back = state.clone();
    Stack::vertical((
        Label::derived(move || {
            match mode.get() {
                Some(Finder::Search) => "Search in files  ·  Enter to search",
                Some(Finder::Commands) => "Commands  ·  / for Pi's commands",
                Some(Finder::Themes) => {
                    "Colour theme  ·  arrows preview, Enter keeps, Escape restores"
                }
                Some(Finder::NewFile) => "New file  ·  a path in the workspace; folders are made",
                _ => "Open file  ·  > for commands, / for Pi's commands",
            }
            .to_string()
        })
        .style(|s| s.color(color::accent()).font_size(12)),
        input,
        Label::derived(move || message.get()).style(move |s| {
            s.color(color::muted())
                .font_size(11)
                .apply_if(message.with(String::is_empty), |s| s.hide())
        }),
        list,
    ))
    .style(move |s| {
        s.absolute()
            .inset_top(60)
            .inset_left_pct(20.0)
            .width_pct(60.0)
            .padding(14)
            .gap(8)
            .background(color::panel())
            .border(1)
            .border_color(color::border())
            .border_radius(10)
            .z_index(30)
            .apply_if(mode.get().is_none(), |s| s.hide())
    })
    .on_event_with_config(el::KeyDown, CAPTURE, move |_, event| {
        let count = items.with_untracked(Vec::len);
        match &event.key {
            Key::Named(NamedKey::ArrowDown) => {
                selected.update(|s| *s = (*s + 1).min(count.saturating_sub(1)));
                EventPropagation::Stop
            }
            Key::Named(NamedKey::ArrowUp) => {
                selected.update(|s| *s = s.saturating_sub(1));
                EventPropagation::Stop
            }
            Key::Named(NamedKey::Enter) => {
                let stale = query.with_untracked(|q| searched.with_untracked(|s| q != s));
                if mode.get_untracked() == Some(Finder::Search) && stale {
                    run_search();
                } else if let Some(item) =
                    items.with_untracked(|items| items.get(selected.get_untracked()).cloned())
                {
                    keys_choose(item);
                }
                EventPropagation::Stop
            }
            // Deleting past the start of a command query goes back to files.
            Key::Named(NamedKey::Backspace)
                if mode.get_untracked() == Some(Finder::Commands)
                    && query.with_untracked(String::is_empty) =>
            {
                back.open_finder(Finder::Files);
                EventPropagation::Stop
            }
            Key::Named(NamedKey::Escape) => {
                dismiss.dismiss_finder();
                EventPropagation::Stop
            }
            _ => EventPropagation::Continue,
        }
    })
}
