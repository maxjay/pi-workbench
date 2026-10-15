//! Native window composition and styling. User actions are implemented in workspace.rs.
mod editor;
mod explorer;
mod finder;
mod lens_doc;
mod overlays;
mod rail;
mod session;
mod terminal;

use crate::{
    commands::{self, Command},
    lens::Since,
    workspace::{Integrations, Pane, Workbench},
};
use editor::pane;
use explorer::explorer;
use floem::{
    context::{EventCallbackConfig, Phases},
    event::EventPropagation,
    prelude::*,
    style::CursorStyle,
    window::WindowId,
};
use overlays::{command_palette, confirmation};
use pi_client::Launch;
use std::{path::PathBuf, rc::Rc};

const BG: Color = Color::from_rgb8(21, 24, 30);
const PANEL: Color = Color::from_rgb8(28, 32, 40);
const BORDER: Color = Color::from_rgb8(47, 54, 66);
const TEXT: Color = Color::from_rgb8(218, 224, 233);
const MUTED: Color = Color::from_rgb8(143, 156, 176);
const ACCENT: Color = Color::from_rgb8(128, 190, 175);
/// Raised cards inside panels.
const SURFACE: Color = Color::from_rgb8(34, 39, 49);
const WARN: Color = Color::from_rgb8(229, 184, 100);
const INFO: Color = Color::from_rgb8(122, 162, 247);
/// Line backgrounds in the changes view and tool previews.
const ADDED: Color = Color::from_rgba8(80, 170, 110, 46);
const REMOVED: Color = Color::from_rgba8(210, 90, 90, 46);
const ADDED_TEXT: Color = Color::from_rgb8(134, 207, 150);
const REMOVED_TEXT: Color = Color::from_rgb8(236, 135, 135);
/// CSS-style family list; the first installed font wins.
pub(crate) const CODE_FONT: &str =
    "'Cascadia Code', 'JetBrains Mono', Consolas, 'DejaVu Sans Mono', monospace";
pub(crate) const CODE_SIZE: usize = 14;
/// Floem's editor background, which its style API does not let us change.
const EDITOR_BG: Color = Color::from_rgb8(40, 44, 52);

/// How the lens marks who added a line.
fn origin_color(origin: crate::origins::Origin) -> Color {
    match origin {
        crate::origins::Origin::Pi => ACCENT,
        crate::origins::Origin::You => INFO,
        crate::origins::Origin::Others => WARN,
    }
}
const CAPTURE: EventCallbackConfig = EventCallbackConfig {
    phases: Phases::CAPTURE,
};

/// Syntax colours; `true` draws the token in italics.
fn token_style(token: workbench_text::highlight::Token) -> Option<(Color, bool)> {
    use workbench_text::highlight::Token;
    Some(match token {
        Token::Keyword => (Color::from_rgb8(198, 146, 233), false),
        Token::String => (Color::from_rgb8(163, 205, 133), false),
        Token::Comment => (Color::from_rgb8(110, 120, 138), true),
        Token::Function => (Color::from_rgb8(120, 178, 245), false),
        Token::Type => (Color::from_rgb8(232, 197, 128), false),
        Token::Number | Token::Constant => (Color::from_rgb8(222, 160, 110), false),
        Token::Operator => (Color::from_rgb8(120, 200, 210), false),
        Token::Attribute | Token::Tag => (Color::from_rgb8(230, 120, 130), false),
        Token::Variable | Token::Punctuation | Token::Plain => return None,
    })
}

/// Editor colours from the app palette, shared by every editor.
fn editor_colors(style: floem::views::EditorCustomStyle) -> floem::views::EditorCustomStyle {
    floem::views::editor::text::default_dark_color(style)
        .cursor_color(ACCENT)
        .selection_color(ACCENT.with_alpha(0.25))
        .current_line_color(Color::from_rgb8(46, 51, 61))
        .gutter_current_color(Color::from_rgb8(46, 51, 61))
        .gutter_dim_color(Color::from_rgb8(84, 94, 110))
        .indent_guide_color(BORDER)
}

/// Editor styling shared by every code view.
fn code_styling() -> floem::views::editor::text::SimpleStyling {
    floem::views::editor::text::SimpleStyling::builder()
        .font_family(floem::text::FamilyOwned::parse_list(&crate::settings::code_font()).collect())
        .font_size(crate::settings::code_size())
        .line_height(1.6)
        .build()
}

fn button(title: &'static str, action: impl Fn() + 'static) -> impl IntoView {
    Button::new(title).action(action).style(|s| {
        s.padding_horiz(10)
            .padding_vert(5)
            .border(0)
            .border_radius(6)
            .background(Color::TRANSPARENT)
            .color(MUTED)
            .hover(|s| s.background(BORDER).color(TEXT))
    })
}

/// A toolbar button that stays highlighted while its feature is on.
fn toggle_button(
    state: Rc<Workbench>,
    title: &'static str,
    command: Command,
    on: impl Fn() -> bool + 'static,
) -> impl IntoView {
    button(title, move || commands::execute(&state, command)).style(move |s| {
        s.apply_if(on(), |s| {
            s.background(ACCENT.with_alpha(0.16))
                .color(ACCENT)
                .hover(|s| s.background(ACCENT.with_alpha(0.26)).color(ACCENT))
        })
    })
}

fn command_button(state: Rc<Workbench>, title: &'static str, command: Command) -> impl IntoView {
    button(title, move || commands::execute(&state, command))
}

fn separator() -> impl IntoView {
    Empty::new().style(|s| s.width(1).height(20).margin_horiz(4).background(BORDER))
}

fn toolbar(state: Rc<Workbench>) -> impl IntoView {
    let root = state
        .root()
        .file_name()
        .unwrap_or(state.root().as_os_str())
        .to_string_lossy()
        .into_owned();
    let layout = state.layout();
    let (changed_only, focus) = (state.lens.changed_only(), state.lens.focus());
    Stack::horizontal((
        Label::new("π").style(|s| {
            s.color(ACCENT)
                .font_size(20)
                .font_family(CODE_FONT.to_string())
        }),
        Label::new(root).style(|s| {
            s.color(TEXT)
                .font_bold()
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        command_button(state.clone(), "Open folder", Command::OpenFolder),
        command_button(state.clone(), "Save", Command::Save),
        separator(),
        toggle_button(state.clone(), "Explorer", Command::Explorer, move || {
            layout.with(|l| l.explorer_visible)
        }),
        toggle_button(state.clone(), "Split", Command::Split, move || {
            layout.with(|l| l.split)
        }),
        toggle_button(state.clone(), "Changed", Command::ChangedOnly, move || {
            changed_only.get()
        }),
        toggle_button(state.clone(), "Focus", Command::Focus, move || focus.get()),
        separator(),
        command_button(state, "Commands", Command::Palette),
    ))
    .style(|s| {
        s.width_full()
            .height(48)
            .padding_horiz(14)
            .gap(4)
            .items_center()
            .flex_shrink(0.0_f32)
            .background(PANEL)
            .border_bottom(1)
            .border_color(BORDER)
    })
}

fn divider(state: Rc<Workbench>, sidebar: bool, width: RwSignal<f64>) -> impl IntoView {
    let dragging = RwSignal::new(None::<(f64, f64)>);
    let layout = state.layout();
    Empty::new()
        .style(move |s| {
            s.width(5)
                .height_full()
                .flex_shrink(0.0_f32)
                .background(BORDER)
                .cursor(CursorStyle::ColResize)
                .hover(|s| s.background(ACCENT))
                .apply_if(
                    layout.with(|l| {
                        if sidebar {
                            !l.explorer_visible
                        } else {
                            !l.split
                        }
                    }),
                    |s| s.hide(),
                )
        })
        .on_event_stop(el::PointerDown, move |cx, event| {
            if let Some(id) = event.pointer.pointer_id {
                cx.request_pointer_capture(id);
                let x = (cx.world_transform.inverse() * event.state.logical_point()).x;
                dragging.set(Some((
                    x,
                    layout.with_untracked(|l| {
                        if sidebar {
                            l.explorer_width
                        } else {
                            l.split_fraction
                        }
                    }),
                )));
            }
        })
        .on_event_stop(el::PointerMove, move |cx, event| {
            if let Some((start_x, start_value)) = dragging.get_untracked() {
                let x = (cx.world_transform.inverse() * event.current.logical_point()).x;
                let layout = layout.get_untracked();
                if sidebar {
                    let direction = if layout.explorer_right { -1.0 } else { 1.0 };
                    state.resize_explorer(start_value + direction * (x - start_x));
                } else {
                    let explorer_width = if layout.explorer_visible {
                        layout.explorer_width + 5.0
                    } else {
                        0.0
                    };
                    let available = (width.get_untracked() - explorer_width - 5.0).max(200.0);
                    state.resize_split(start_value + (x - start_x) / available);
                }
            }
        })
        .on_event_stop(el::PointerUp, move |_, _| dragging.set(None))
        .on_event_stop(el::LostPointerCapture, move |_, _| dragging.set(None))
}

pub fn workbench(window: WindowId, root: PathBuf, layout_path: PathBuf) -> impl IntoView {
    let (settings, settings_error) = crate::settings::current();
    let integrations = Integrations {
        pi: Launch::from_settings(settings.pi.command(), &settings.pi.args),
    };
    let state = Workbench::new(root, layout_path, integrations, move || {
        floem::close_window(window)
    });
    state.restore();
    if let Some(err) = settings_error {
        state.report(err);
    }
    state.sessions.refresh_recent();
    state.sessions.create();
    state.lens.start();
    let (synced, changed, tree) = (
        state.documents.clone(),
        state.lens.clone(),
        state.explorer.clone(),
    );
    // Changes from outside (a terminal, a formatter) fold in like Pi's.
    if let Some(watcher) =
        crate::watcher::watch(floem::reactive::Scope::current(), state.root(), move || {
            synced.sync_from_disk("disk");
            changed.refresh();
            tree.refresh();
        })
    {
        state.keep(watcher);
    }
    let layout = state.layout();
    let status = state.status();
    let focused = state.focused();
    let width = RwSignal::new(1280.0);
    let close_state = state.clone();
    let keyboard_state = state.clone();
    let panes = Stack::horizontal((
        pane(state.clone(), Pane::Primary),
        divider(state.clone(), false, width),
        pane(state.clone(), Pane::Secondary),
    ))
    .style(move |s| s.size_full().min_width(0).min_height(0));
    let editors = Stack::vertical((panes, terminal::terminal_panel(state.clone())))
        .style(move |s| s.size_full().min_width(0).min_height(0));
    let body = Stack::horizontal((
        explorer(state.clone()),
        divider(state.clone(), true, width),
        editors,
    ))
    .style(move |s| {
        s.size_full().min_width(0).min_height(0).flex_direction(
            if layout.with(|l| l.explorer_right) {
                floem::taffy::FlexDirection::RowReverse
            } else {
                floem::taffy::FlexDirection::Row
            },
        )
    });
    let (dot_sessions, label_sessions) = (state.sessions.clone(), state.sessions.clone());
    let position_state = state.clone();
    let lens_open = RwSignal::new(false);
    let (summary_lens, menu_state) = (state.lens.clone(), state.clone());
    let lens_summary = Label::derived(move || {
        let label = summary_lens.label(summary_lens.since().get());
        summary_lens.changes().with(|changes| {
            let added: usize = changes.iter().map(|c| c.added).sum();
            let removed: usize = changes.iter().map(|c| c.removed).sum();
            match changes.len() {
                0 => format!("No changes since {label}"),
                1 => format!("Changes since {label}  ·  1 file  +{added} −{removed}"),
                n => format!("Changes since {label}  ·  {n} files  +{added} −{removed}"),
            }
        })
    })
    .style(|s| {
        s.color(ACCENT)
            .padding_horiz(8)
            .padding_vert(2)
            .border_radius(5)
            .cursor(floem::style::CursorStyle::Pointer)
            .hover(|s| s.background(BORDER))
            .flex_shrink(0.0_f32)
    })
    .on_event_stop(el::Click, move |_, _| {
        lens_open.update(|open| *open = !*open)
    });
    // Who added the changed lines, as the editors colour them.
    let legend_changes = state.lens.changes();
    let legend = Stack::horizontal(
        [
            ("Pi", crate::origins::Origin::Pi),
            ("you", crate::origins::Origin::You),
            ("others", crate::origins::Origin::Others),
        ]
        .map(|(name, origin)| {
            Stack::horizontal((
                Empty::new().style(move |s| {
                    s.size(8, 8)
                        .border_radius(2)
                        .background(origin_color(origin))
                }),
                Label::new(name).style(|s| s.color(MUTED).font_size(11)),
            ))
            .style(|s| s.gap(4).items_center())
        }),
    )
    .style(move |s| {
        s.gap(10)
            .items_center()
            .flex_shrink(0.0_f32)
            .apply_if(legend_changes.with(Vec::is_empty), |s| s.hide())
    });
    let footer = Stack::horizontal((
        lens_summary,
        legend,
        Label::derived(move || status.get()).style(|s| {
            s.color(MUTED)
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        Empty::new().style(move |s| {
            let status = dot_sessions.active().map(|s| s.status());
            s.size(7, 7)
                .border_radius(4)
                .background(status.map_or(MUTED, session::status_color))
        }),
        Label::derived(move || {
            label_sessions
                .active()
                .map_or("No session".into(), |session| {
                    let model = session.model().get().map(|m| m.name).unwrap_or_default();
                    format!("Pi {}  {model}", session::status_label(session.status()))
                })
        })
        .style(|s| s.color(MUTED)),
        Label::derived(move || {
            let pane = focused.get();
            if pane == Pane::Primary && position_state.conversation().get() {
                return String::new();
            }
            let (line, col) = (
                position_state.cursor_line(pane).get(),
                position_state.cursor_col(pane).get(),
            );
            format!("Ln {}, Col {}  ·  UTF-8", line + 1, col + 1)
        })
        .style(|s| s.color(MUTED).margin_left(8)),
    ))
    .style(|s| {
        s.width_full()
            .height(28)
            .padding_horiz(12)
            .gap(10)
            .font_size(11)
            .items_center()
            .flex_shrink(0.0_f32)
            .background(PANEL)
            .border_top(1)
            .border_color(BORDER)
    });
    Stack::new((
        Stack::vertical((
            toolbar(state.clone()),
            Stack::horizontal((rail::rail(state.clone()), body))
                .style(|s| s.size_full().min_width(0).min_height(0)),
            footer,
        ))
        .style(|s| s.size_full()),
        lens_menu(menu_state, lens_open),
        finder::finder(state.clone()),
        command_palette(state.clone()),
        confirmation(state),
    ))
    .style(|s| s.size_full().background(BG).color(TEXT).font_size(13))
    .on_event_cont(el::WindowCloseRequested, move |cx, _| {
        cx.prevent_default();
        close_state.request_quit();
    })
    .on_event_cont(el::WindowResized, move |_, size| width.set(size.width))
    .on_event_with_config(el::KeyDown, CAPTURE, move |cx, event| {
        if event.key == Key::Named(NamedKey::Escape) {
            keyboard_state.dismiss_palette();
            keyboard_state.dismiss_finder();
            keyboard_state.dismiss_confirmation();
            return EventPropagation::Continue;
        }
        if let Some(command) = commands::for_key(&event.key, event.modifiers) {
            // While the finder has the keyboard, only finder keys apply.
            let finding = keyboard_state.finder().get_untracked().is_some();
            if finding && !matches!(command, Command::QuickOpen | Command::FindInFiles) {
                return EventPropagation::Continue;
            }
            // The shell gets Ctrl keys such as Ctrl+P and Ctrl+W, except these.
            let in_terminal = keyboard_state.terminal_focused().get_untracked();
            if in_terminal
                && !matches!(
                    command,
                    Command::Terminal
                        | Command::Palette
                        | Command::NextSession
                        | Command::PreviousSession
                        | Command::Session(_)
                )
            {
                return EventPropagation::Continue;
            }
            commands::execute(&keyboard_state, command);
            cx.prevent_default();
            return EventPropagation::Stop;
        }
        EventPropagation::Continue
    })
}

/// What the changes lens compares against, chosen from the status bar.
fn lens_menu(state: Rc<Workbench>, open: RwSignal<bool>) -> impl IntoView {
    let since = state.lens.since();
    let items = Since::ALL.map(|choice| {
        let (label_state, pick_state) = (state.clone(), state.clone());
        Label::derived(move || {
            let mark = if since.get() == choice {
                "●  "
            } else {
                "    "
            };
            format!("{mark}{}", label_state.lens.label(choice))
        })
        .style(|s| {
            s.width_full()
                .padding_horiz(12)
                .padding_vert(7)
                .color(TEXT)
                .cursor(floem::style::CursorStyle::Pointer)
                .hover(|s| s.background(BORDER))
        })
        .on_event_stop(el::Click, move |_, _| {
            pick_state.lens.set_since(choice);
            open.set(false);
        })
        .into_any()
    });
    Stack::vertical((
        Label::new("Compare everything with").style(|s| {
            s.color(MUTED)
                .font_size(11)
                .padding_horiz(12)
                .padding_vert(6)
        }),
        Stack::vertical_from_iter(items).style(|s| s.width_full()),
    ))
    .style(move |s| {
        s.absolute()
            .inset_bottom(32)
            .inset_left(12)
            .width(260)
            .padding_vert(4)
            .background(PANEL)
            .border(1)
            .border_color(BORDER)
            .border_radius(8)
            .z_index(20)
            .apply_if(!open.get(), |s| s.hide())
    })
}
