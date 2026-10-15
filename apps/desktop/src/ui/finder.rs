//! The finder: open a file by name (Ctrl+P) or search file contents (Ctrl+Shift+F).
use super::{ACCENT, BG, BORDER, CAPTURE, CODE_FONT, MUTED, PANEL, TEXT};
use crate::workspace::{Finder, Workbench};
use floem::{
    event::EventPropagation,
    ext_event::create_ext_action,
    prelude::*,
    reactive::{Effect, Scope},
    style::CursorStyle,
    views::{PlaceholderTextClass, TextInput},
};
use std::{cell::RefCell, path::PathBuf, rc::Rc};
use workbench_text::{
    index::{FileMatcher, list_files},
    search::{Hit, Query, search},
};

const FILE_LIMIT: usize = 50_000;
const SHOWN: usize = 60;
const HIT_LIMIT: usize = 500;

/// One result row: a file to open, at a line for search hits.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Item {
    path: PathBuf,
    line: Option<usize>,
    title: String,
    detail: String,
}

fn row(
    item: Item,
    index: usize,
    selected: RwSignal<usize>,
    choose: Rc<dyn Fn(Item)>,
) -> impl IntoView {
    let picked = item.clone();
    Stack::horizontal((
        Label::new(item.title).style(|s| s.color(TEXT).flex_shrink(0.0_f32)),
        Label::new(item.detail).style(|s| {
            s.color(MUTED)
                .font_size(12)
                .min_width(0)
                .flex_grow(1.0_f32)
                .text_ellipsis()
                .font_family(CODE_FONT.to_string())
        }),
    ))
    .style(move |s| {
        s.width_full()
            .gap(10)
            .padding_horiz(12)
            .padding_vert(6)
            .items_center()
            .cursor(CursorStyle::Pointer)
            .hover(|s| s.background(BORDER))
            .apply_if(selected.get() == index, |s| {
                s.background(ACCENT.with_alpha(0.16))
            })
    })
    .on_event_stop(el::Click, move |_, _| choose(picked.clone()))
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

    // Opening lists files off the UI thread, so names are ready to match.
    let root = state.root().to_path_buf();
    Effect::new(move |_| {
        let Some(current) = mode.get() else { return };
        query.set(String::new());
        selected.set(0);
        hits.set(Vec::new());
        searched.set(String::new());
        message.set(String::new());
        if current == Finder::Files {
            let finish = create_ext_action(scope, move |list| files.set(list));
            let root = root.clone();
            std::thread::spawn(move || finish(list_files(&root, FILE_LIMIT)));
        }
    });

    let items = {
        let matcher = matcher.clone();
        move || -> Vec<Item> {
            match mode.get() {
                Some(Finder::Files) => {
                    let text = query.get();
                    files.with(|files| {
                        matcher
                            .borrow_mut()
                            .rank(&text, files, SHOWN)
                            .into_iter()
                            .map(|(path, _)| {
                                let title = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                let detail = path
                                    .parent()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default();
                                Item {
                                    path,
                                    line: None,
                                    title,
                                    detail,
                                }
                            })
                            .collect()
                    })
                }
                Some(Finder::Search) => hits.with(|hits| {
                    hits.iter()
                        .map(|hit| Item {
                            path: hit.path.clone(),
                            line: Some(hit.line),
                            title: format!("{}:{}", hit.path.display(), hit.line + 1),
                            detail: hit.text.trim().to_string(),
                        })
                        .collect()
                }),
                None => Vec::new(),
            }
        }
    };

    let choose: Rc<dyn Fn(Item)> = {
        let state = state.clone();
        Rc::new(move |item: Item| {
            let path = state.root().join(&item.path);
            state.dismiss_finder();
            match item.line {
                Some(line) => state.open_at(path, line),
                None => state.open_at(path, 0),
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
            let finish =
                create_ext_action(
                    scope,
                    move |result: anyhow::Result<Vec<Hit>>| match result {
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
                .background(BG)
                .color(TEXT)
                .border(1)
                .border_color(BORDER)
                .border_radius(6)
                .focus(|s| s.border_color(ACCENT))
                .class(PlaceholderTextClass, |s| s.color(MUTED))
        })
        .placeholder("Type to search");
    let input_id = input.view_id();
    Effect::new(move |_| {
        if mode.get().is_some() {
            floem::action::exec_after_animation_frame(move |_| input_id.request_focus());
        }
    });
    // Typing a new file query moves the selection back to the best match.
    Effect::new(move |_| {
        query.track();
        selected.set(0);
    });

    let keys_items = items.clone();
    let keys_choose = choose.clone();
    let list_choose = choose.clone();
    let list = dyn_stack(
        move || items().into_iter().enumerate().collect::<Vec<_>>(),
        |(index, item)| (*index, item.clone()),
        move |(index, item)| row(item, index, selected, list_choose.clone()),
    )
    .style(|s| s.flex_col().width_full())
    .scroll()
    .style(|s| s.width_full().max_height(420));

    let dismiss = state.clone();
    Stack::vertical((
        Label::derived(move || {
            match mode.get() {
                Some(Finder::Search) => "Search in files  ·  Enter to search",
                _ => "Open file",
            }
            .to_string()
        })
        .style(|s| s.color(ACCENT).font_size(12)),
        input,
        Label::derived(move || message.get()).style(move |s| {
            s.color(MUTED)
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
            .background(PANEL)
            .border(1)
            .border_color(BORDER)
            .border_radius(10)
            .z_index(30)
            .apply_if(mode.get().is_none(), |s| s.hide())
    })
    .on_event_with_config(el::KeyDown, CAPTURE, move |_, event| {
        let count = keys_items().len();
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
                } else if let Some(item) = keys_items().into_iter().nth(selected.get_untracked()) {
                    keys_choose(item);
                }
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
