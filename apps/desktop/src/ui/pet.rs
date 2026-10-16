//! Pets on screen: the sprite, cut from the sheet for the theme; what a pet
//! has to say; each session's pet in its view; and the one popped out onto
//! the desktop, which a `/kill` throws across the screen.
use super::{color, session::ask_card};
#[cfg(windows)]
use crate::pet::Bounds;
use crate::{
    pet::{CELL, GROWN, Pet, PetWindow, Pets, Windup},
    workspace::Workbench,
};
use floem::{
    kurbo::{Point, Rect, Size},
    peniko::{self, Blob, ImageAlphaType, ImageData},
    prelude::*,
    reactive::{Effect, Memo, ReadSignal},
    style::CursorStyle,
    views::{PlaceholderTextClass, TextInput, TextInputEnter, canvas},
    window::{WindowConfig, WindowId, WindowIdExt, WindowLevel},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::Arc,
};

const DARK: &[u8] = include_bytes!("../../assets/pet/glitch-dark.png");
const LIGHT: &[u8] = include_bytes!("../../assets/pet/glitch-light.png");

/// In a session's view a pet is drawn small; popped out, larger.
const DOCKED: f64 = 0.42;
const POPPED: f64 = 0.75;
const BUBBLE_WIDTH: f64 = 280.0;
/// Let go faster than this (logical pixels a second), a pet flies.
const FLING_SPEED: f64 = 350.0;

/// How fast something was going over its last moments, from where it was
/// when: pixels a second.
fn release_velocity(trail: &[(std::time::Instant, (f64, f64))]) -> (f64, f64) {
    let Some(&(last_at, last)) = trail.last() else {
        return (0.0, 0.0);
    };
    let window = std::time::Duration::from_millis(90);
    let Some(&(first_at, first)) = trail.iter().find(|(at, _)| last_at - *at <= window) else {
        return (0.0, 0.0);
    };
    let seconds = (last_at - first_at).as_secs_f64();
    if seconds < 0.005 {
        return (0.0, 0.0);
    }
    ((last.0 - first.0) / seconds, (last.1 - first.1) / seconds)
}

/// The empty top of a cell, above the pet's antenna, as a share of it.
#[cfg_attr(not(windows), allow(dead_code))]
const HEADROOM: f64 = 0.3;

/// A frame's pixels, scaled.
type Pixels = Rc<Vec<u8>>;

/// A frame cut from the sheet, and its key in the renderer's cache.
type Frame = (peniko::ImageBrush, Vec<u8>);

/// One theme's sheet, and the frames cut from it so far.
struct Sheet {
    image: image::RgbaImage,
    dark: bool,
    frames: RefCell<HashMap<(u32, u32), Frame>>,
    /// Frames scaled for the popped-out pet.
    #[cfg_attr(not(windows), allow(dead_code))]
    scaled: RefCell<HashMap<(u32, u32, u32, u32), Pixels>>,
}

impl Sheet {
    fn cell(&self, row: u32, column: u32) -> image::RgbaImage {
        let (width, height) = CELL;
        image::imageops::crop_imm(&self.image, column * width, row * height, width, height)
            .to_image()
    }

    fn frame(&self, row: u32, column: u32) -> Frame {
        self.frames
            .borrow_mut()
            .entry((row, column))
            .or_insert_with(|| {
                let (width, height) = CELL;
                // Nearest-neighbour, so the pixel art stays crisp when scaled.
                let brush = peniko::ImageBrush::new(ImageData {
                    data: Blob::new(Arc::new(self.cell(row, column).into_raw())),
                    format: peniko::ImageFormat::Rgba8,
                    alpha_type: ImageAlphaType::Alpha,
                    width,
                    height,
                })
                .with_quality(peniko::ImageQuality::Low);
                let key = format!("pet:{}:{row}:{column}", self.dark).into_bytes();
                (brush, key)
            })
            .clone()
    }

    /// A frame scaled to a size, nearest-neighbour, as RGBA pixels.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn pixels(&self, row: u32, column: u32, width: u32, height: u32) -> Rc<Vec<u8>> {
        self.scaled
            .borrow_mut()
            .entry((row, column, width, height))
            .or_insert_with(|| {
                Rc::new(
                    image::imageops::resize(
                        &self.cell(row, column),
                        width,
                        height,
                        image::imageops::FilterType::Nearest,
                    )
                    .into_raw(),
                )
            })
            .clone()
    }
}

/// RGBA pixels turned `angle` radians about their middle, on a square
/// canvas `side` wide that holds them at any angle.
#[cfg_attr(not(windows), allow(dead_code))]
fn turned(pixels: &[u8], width: u32, height: u32, side: u32, angle: f64) -> Vec<u8> {
    let mut out = vec![0u8; (side * side * 4) as usize];
    let (pad_x, pad_y) = ((side - width) / 2, (side - height) / 2);
    if angle == 0.0 {
        for y in 0..height {
            let from = (y * width * 4) as usize;
            let to = (((y + pad_y) * side + pad_x) * 4) as usize;
            out[to..to + (width * 4) as usize]
                .copy_from_slice(&pixels[from..from + (width * 4) as usize]);
        }
        return out;
    }
    let middle = f64::from(side) / 2.0;
    let (half_w, half_h) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
    // Each pixel of the canvas, turned back, lands on the one it shows.
    let (sin, cos) = (-angle).sin_cos();
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (f64::from(x) + 0.5 - middle, f64::from(y) + 0.5 - middle);
            let (sx, sy) = (dx * cos - dy * sin + half_w, dx * sin + dy * cos + half_h);
            if sx < 0.0 || sy < 0.0 || sx >= f64::from(width) || sy >= f64::from(height) {
                continue;
            }
            let from = ((sy as u32 * width + sx as u32) * 4) as usize;
            let to = ((y * side + x) * 4) as usize;
            out[to..to + 4].copy_from_slice(&pixels[from..from + 4]);
        }
    }
    out
}

/// The sheet for a dark or light theme, decoded once.
fn sheet(dark: bool) -> Rc<Sheet> {
    thread_local! {
        static SHEETS: RefCell<[Option<Rc<Sheet>>; 2]> = const { RefCell::new([None, None]) };
    }
    SHEETS.with_borrow_mut(|sheets| {
        sheets[usize::from(dark)]
            .get_or_insert_with(|| {
                let bytes = if dark { DARK } else { LIGHT };
                let image = image::load_from_memory(bytes)
                    .map(|image| image.into_rgba8())
                    .unwrap_or_default();
                Rc::new(Sheet {
                    image,
                    dark,
                    frames: RefCell::new(HashMap::new()),
                    scaled: RefCell::new(HashMap::new()),
                })
            })
            .clone()
    })
}

/// A pet, at a scale of its 192×208 cell. Ivory on dark themes would glare;
/// the charcoal one suits them.
fn sprite(pet: &Rc<Pet>, scale: f64) -> floem::views::Canvas {
    let pose = pet.pose();
    let grown_pet = pet.clone();
    canvas(move |cx, size| {
        let pose = pose.get();
        let sheet = sheet(color::is_dark());
        let (brush, key) = sheet.frame(pose.anim.row(), pose.column);
        cx.draw_img(
            floem_renderer::Img {
                img: brush,
                hash: &key,
            },
            Rect::from_origin_size(Point::ZERO, size),
        );
    })
    .style(move |s| {
        let scale = scale * grown_pet.grown();
        s.width(f64::from(CELL.0) * scale)
            .height(f64::from(CELL.1) * scale)
            .cursor(CursorStyle::Pointer)
    })
}

/// Where in its cell the pet itself is, as shares of the cell: left, top,
/// right, bottom. Its corners are what you drag to make it bigger.
const FIGURE: (f64, f64, f64, f64) = (0.22, 0.3, 0.78, 0.94);

/// What you say to a pet. By default a side question about its session
/// (`/btw`), answered in its bubble and kept out of the session; pick a
/// session on the left and it goes to that session instead.
fn chat(state: &Rc<Workbench>, pet: &Rc<Pet>) -> impl IntoView {
    let text = RwSignal::new(String::new());
    let (target, open) = (pet.target, pet.chat_open);
    let picking = RwSignal::new(false);
    let sessions = state.sessions.clone();
    let open_sessions = state.sessions.open();
    let name = move |key: Option<u64>| match key {
        None => "btw".to_string(),
        Some(key) => sessions
            .find_tracked(key)
            .map(|s| crate::session::shorten(&s.title().get()))
            .unwrap_or_else(|| "closed".into()),
    };
    let pill_name = name.clone();
    let pill = Label::derived(move || format!("{} ▾", pill_name(target.get())))
        .style(|s| {
            s.font_size(11)
                .padding_horiz(8)
                .padding_vert(5)
                .border_radius(6)
                .max_width(110)
                .text_ellipsis()
                .flex_shrink(0.0_f32)
                .color(color::text())
                .background(color::highlight())
                .cursor(CursorStyle::Pointer)
        })
        .on_event_stop(el::Click, move |_, _| picking.update(|p| *p = !*p));
    // The choices, above the row: by the way, or any open session.
    let options = dyn_stack(
        move || {
            let mut keys = vec![None];
            keys.extend(open_sessions.get().iter().map(|s| Some(s.key)));
            keys
        },
        |key| *key,
        move |key| {
            let label = match key {
                None => "btw: ask aside, the session left as it is".to_string(),
                Some(_) => format!("Send to {}", name(key)),
            };
            Label::new(label)
                .style(move |s| {
                    s.width_full()
                        .padding_horiz(8)
                        .padding_vert(4)
                        .border_radius(5)
                        .font_size(11)
                        .text_ellipsis()
                        .cursor(CursorStyle::Pointer)
                        .hover(|s| s.background(color::highlight()))
                        .apply_if(target.get() == key, |s| s.color(color::accent()))
                })
                .on_event_stop(el::Click, move |_, _| {
                    target.set(key);
                    picking.set(false);
                })
        },
    )
    .style(move |s| {
        s.flex_col()
            .width_full()
            .padding(4)
            .background(color::panel())
            .border(1)
            .border_color(color::border())
            .border_radius(8)
            .apply_if(!picking.get(), |s| s.hide())
    });
    let send_state = state.clone();
    let key = pet.key;
    let send = Rc::new(move || {
        let said = text.get_untracked();
        if !said.trim().is_empty() {
            text.set(String::new());
            send_state.tell_pet(key, said);
        }
    });
    let enter = send.clone();
    let input = TextInput::new(text)
        .placeholder("Say something…")
        .style(|s| {
            s.flex_grow(1.0_f32)
                .min_width(0)
                .padding_horiz(8)
                .padding_vert(5)
                .font_size(12)
                .border(1)
                .border_color(color::border())
                .border_radius(6)
                .background(color::bg())
                .color(color::text())
                .focus(|s| s.border_color(color::accent()))
                .class(PlaceholderTextClass, |s| s.color(color::muted()))
        })
        .on_event_stop(TextInputEnter::listener(), move |_, _| enter());
    let input_id = input.view_id();
    // Opening it puts the cursor there.
    Effect::new(move |_| {
        if open.get() {
            floem::action::exec_after_animation_frame(move |_| input_id.request_focus());
        }
    });
    let close = Label::new("×")
        .style(|s| {
            s.color(color::muted())
                .padding_horiz(4)
                .cursor(CursorStyle::Pointer)
                .hover(|s| s.color(color::text()))
        })
        .on_event_stop(el::Click, move |_, _| open.set(false));
    Stack::vertical((
        options,
        Stack::horizontal((pill, input, close)).style(|s| s.width_full().gap(6).items_center()),
    ))
    .style(move |s| {
        s.width(BUBBLE_WIDTH)
            .padding(8)
            .gap(6)
            .background(color::panel())
            .border(1)
            .border_color(color::border())
            .border_radius(10)
            .apply_if(!open.get(), |s| s.hide())
    })
    .on_event_cont(el::KeyDown, move |_, event| {
        if event.key == Key::Named(NamedKey::Escape) {
            open.set(false);
        }
    })
}

/// The corners to drag to make a docked pet bigger or smaller; it grows
/// up from its feet.
fn grips(state: &Rc<Workbench>, pet: &Rc<Pet>, hovered: Hover) -> impl IntoView {
    let pets = state.pets.clone();
    let grown = pets.docked_scale;
    let dock = pets.dock;
    let corner = |left: bool, top: bool| {
        let (state, pet) = (state.clone(), pet.clone());
        // Where the drag began, how big the pet was, and its feet (window).
        let start = Rc::new(Cell::new(None::<(Point, f64, Point)>));
        let (down, moving, up) = (start.clone(), start.clone(), start);
        let (down_state, move_state) = (state.clone(), state);
        let sprite_pet = pet.clone();
        Empty::new()
            .style(move |s| {
                let size = (
                    f64::from(CELL.0) * DOCKED * grown.get(),
                    f64::from(CELL.1) * DOCKED * grown.get(),
                );
                let x = size.0 * if left { FIGURE.0 } else { FIGURE.2 } - 4.0;
                let y = size.1 * if top { FIGURE.1 } else { FIGURE.3 } - 4.0;
                s.absolute()
                    .inset_left(x)
                    .inset_top(y)
                    .size(9, 9)
                    .border_radius(2)
                    .border(1)
                    .border_color(color::accent())
                    .background(color::panel())
                    .cursor(if left == top {
                        CursorStyle::NwseResize
                    } else {
                        CursorStyle::NeswResize
                    })
                    .apply_if(!hovered.get(), |s| s.hide())
            })
            .on_event_stop(el::PointerDown, move |cx, event| {
                if let Some(id) = event.pointer.pointer_id {
                    cx.request_pointer_capture(id);
                }
                let Some(sprite) = sprite_pet.sprite_view.get() else {
                    return;
                };
                let rect = sprite.get_visual_rect();
                let feet = Point::new(rect.center().x, rect.y1);
                down.set(Some((down_state.pointer(), grown.get_untracked(), feet)));
            })
            .on_event_stop(el::PointerMove, move |_, _| {
                let Some((from, was, feet)) = moving.get() else {
                    return;
                };
                let at = move_state.pointer();
                let before = from.distance(feet).max(8.0);
                let now = (was * at.distance(feet) / before).clamp(GROWN.0, GROWN.1);
                grown.set(now);
                // Its feet stay where they are.
                let width = f64::from(CELL.0) * DOCKED * now;
                let (_, bottom) = dock.get_untracked();
                dock.set(((feet.x - width / 2.0).max(0.0), bottom));
            })
            .on_event_stop(el::PointerUp, move |_, _| up.set(None))
    };
    Stack::new((
        corner(true, true),
        corner(false, true),
        corner(true, false),
        corner(false, false),
    ))
    // Takes no room of its own, so only the grips catch the pointer and
    // the pet can still be picked up.
    .style(|s| s.absolute().inset_left(0).inset_top(0).size(0, 0))
}

/// What the bubble shows: a question first, then the latest news.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Say {
    Question { session: u64, entry: u64 },
    Update(u64),
}

/// A pet's speech bubble: a question, answered right there, or what it has
/// to say about its session (or, popped out, another), which opens the
/// session when clicked.
fn bubble(state: &Rc<Workbench>, pet: &Rc<Pet>) -> Stack {
    let updates = pet.updates();
    let asking = pet.clone();
    let say = Memo::new(move |_| {
        if let Some((session, entry)) = asking.questions().first() {
            return Some(Say::Question {
                session: session.key,
                entry: entry.id,
            });
        }
        updates.with(|u| u.last().map(|u| Say::Update(u.id)))
    });
    let more_pet = pet.clone();
    let more = Memo::new(move |_| more_pet.questions().len() + updates.with(Vec::len));
    let (view_state, view_pet) = (state.clone(), pet.clone());
    let own = pet.key;
    let body = dyn_container(
        move || say.get(),
        move |say| match say {
            Some(Say::Question { session, entry }) => {
                let found = view_pet
                    .questions()
                    .into_iter()
                    .find(|(s, e)| s.key == session && e.id == entry);
                let Some((session, entry)) = found else {
                    return Empty::new().into_any();
                };
                let title = session.title();
                let open_state = view_state.clone();
                let key = session.key;
                // Another session's question says whose it is.
                let theirs = key != own;
                Stack::vertical((
                    Label::derived(move || format!("{} asks", title.get()))
                        .style(move |s| {
                            s.color(color::muted())
                                .font_size(11)
                                .cursor(CursorStyle::Pointer)
                                .hover(|s| s.color(color::accent()))
                                .apply_if(!theirs, |s| s.hide())
                        })
                        .on_event_stop(el::Click, move |_, _| open_state.show_session(key)),
                    ask_card(session, entry),
                ))
                .style(|s| s.gap(6).width_full())
                .into_any()
            }
            Some(Say::Update(id)) => {
                let Some(update) = view_pet
                    .updates()
                    .with_untracked(|u| u.iter().find(|u| u.id == id).cloned())
                else {
                    return Empty::new().into_any();
                };
                let (open_state, open_pet) = (view_state.clone(), view_pet.clone());
                let close_pet = view_pet.clone();
                Stack::horizontal((
                    Label::new(update.text.clone()).style(|s| {
                        s.flex_grow(1.0_f32)
                            .min_width(0)
                            .text_wrap()
                            .cursor(CursorStyle::Pointer)
                    }),
                    {
                        let text = update.text.clone();
                        super::session::copy_button(move || text.clone(), 1.0)
                    },
                    Label::new("×")
                        .style(|s| {
                            s.color(color::muted())
                                .padding_horiz(4)
                                .cursor(CursorStyle::Pointer)
                                .hover(|s| s.color(color::text()))
                        })
                        .on_event_stop(el::Click, move |_, _| close_pet.dismiss(id)),
                ))
                .style(|s| s.gap(6).width_full().items_start())
                .on_event_stop(el::Click, move |_, _| {
                    // One update, so the bubble changes once.
                    Effect::batch(|| {
                        if let Some(key) = update.session {
                            open_state.show_session(key);
                        }
                        open_pet.dismiss(id);
                    })
                })
                .into_any()
            }
            None => Empty::new().into_any(),
        },
    );
    Stack::vertical((
        body,
        Label::derived(move || format!("+{} more", more.get().saturating_sub(1))).style(move |s| {
            s.color(color::muted())
                .font_size(11)
                .apply_if(more.get() <= 1, |s| s.hide())
        }),
    ))
    .style(move |s| {
        s.width(BUBBLE_WIDTH)
            .padding(10)
            .gap(4)
            .font_size(12)
            .color(color::text())
            .background(color::panel())
            .border(1)
            .border_color(color::border())
            .border_radius(10)
            .box_shadow_blur(10)
            .box_shadow_color(Color::BLACK.with_alpha(0.25))
            .apply_if(say.get().is_none(), |s| s.hide())
    })
}

/// Hovered, and staying so for a moment after the pointer leaves, so it
/// can reach the pills that stick out past the pet.
#[derive(Clone, Copy)]
struct Hover {
    shown: RwSignal<bool>,
    /// Bumped on every enter and leave, so a late hide is dropped.
    turn: RwSignal<u64>,
}

impl Hover {
    fn new() -> Self {
        Self {
            shown: RwSignal::new(false),
            turn: RwSignal::new(0),
        }
    }
    fn get(&self) -> bool {
        self.shown.get()
    }
    fn enter(&self) {
        self.turn.update(|t| *t += 1);
        if !self.shown.get_untracked() {
            self.shown.set(true);
        }
    }
    fn leave(&self) {
        self.turn.update(|t| *t += 1);
        let (turn, shown, now) = (self.turn, self.shown, self.turn.get_untracked());
        floem::action::exec_after(std::time::Duration::from_millis(700), move |_| {
            if turn.try_get_untracked() == Some(now) {
                shown.try_update(|s| *s = false);
            }
        });
    }
}

/// A small pill shown on hover.
fn control(
    title: impl Fn() -> &'static str + 'static,
    action: impl Fn() + 'static,
) -> impl IntoView {
    Label::derived(move || title().to_string())
        .style(|s| {
            s.font_size(10)
                .padding_horiz(7)
                .padding_vert(2)
                .border_radius(8)
                .color(color::text())
                .background(color::panel())
                .border(1)
                .border_color(color::border())
                .cursor(CursorStyle::Pointer)
                .hover(|s| s.border_color(color::accent()))
        })
        .on_event_stop(el::PointerDown, |_, _| {})
        .on_event_stop(el::Click, move |_, _| action())
}

/// Pop out (or in) and hide, over the empty top of the sprite's cell, so
/// showing them moves nothing.
fn controls(pets: &Rc<Pets>, pet: &Rc<Pet>, hovered: Hover) -> impl IntoView {
    // Over the sprite's top, from its left edge.
    let (pop_pets, hide_pets) = (pets.clone(), pets.clone());
    let out = pets.out;
    let key = pet.key;
    let chat_open = pet.chat_open;
    Stack::horizontal((
        control(|| "Chat", move || chat_open.update(|o| *o = !*o)),
        control(
            move || {
                if out.get() == Some(key) {
                    "Pop in"
                } else {
                    "Pop out"
                }
            },
            move || {
                if pop_pets.out.get_untracked() == Some(key) {
                    pop_pets.pop_in()
                } else {
                    pop_pets.pop_out(key)
                }
            },
        ),
        control(|| "Hide", move || hide_pets.toggle_hidden()),
    ))
    .style(move |s| {
        s.absolute()
            .inset_top(0)
            .inset_left(0)
            .gap(4)
            .apply_if(!hovered.get(), |s| s.hide())
    })
    .on_event_cont(el::PointerEnter, move |_, _| hovered.enter())
    .on_event_cont(el::PointerLeave, move |_, _| hovered.leave())
}

/// The front session's pet in the window: switch sessions and it is that
/// session's. It starts at the foot of the sessions rail and goes wherever
/// it is dragged; thrown hard, it leaves the window and flies on across
/// the screen.
pub(super) fn docked(state: Rc<Workbench>, main: WindowId) -> impl IntoView {
    let pets = state.pets.clone();
    let front = state.sessions.active_key();
    let (out, hidden, dock) = (pets.out, pets.hidden, pets.dock);
    let grown = pets.docked_scale;
    let shown = Memo::new(move |_| {
        front
            .get()
            .filter(|key| out.get() != Some(*key) && !hidden.get())
    });
    // The window's size, to keep the pet and its bubble in it.
    let window = RwSignal::new(Size::ZERO);
    let view_state = state.clone();
    let view = dyn_container(
        move || shown.get(),
        move |key| match key {
            Some(key) => roaming(view_state.clone(), key, main, window).into_any(),
            None => Empty::new().into_any(),
        },
    );
    let view_id = view.view_id();
    view.style(move |s| {
        let (left, bottom) = dock.get();
        let size_now = window.get();
        let grown = DOCKED * grown.get();
        let pet = (f64::from(CELL.0) * grown, f64::from(CELL.1) * grown);
        // Kept in reach however small the window gets.
        let (left, bottom) = if size_now.width > 0.0 {
            (
                left.min(size_now.width - pet.0).max(0.0),
                bottom.min(size_now.height - pet.1).max(0.0),
            )
        } else {
            (left, bottom)
        };
        s.absolute()
            .inset_left(left)
            .inset_bottom(bottom)
            .z_index(25)
    })
    .on_event_cont(el::UpdatePhaseLayout, move |_, _| {
        let now = view_id.root().get_size().unwrap_or_default();
        if window.get_untracked() != now {
            window.set(now);
        }
    })
}

/// One session's pet roaming the window.
fn roaming(
    state: Rc<Workbench>,
    key: u64,
    main: WindowId,
    window: RwSignal<Size>,
) -> impl IntoView {
    let pets = state.pets.clone();
    let pet = pets.pet(key);
    let pose = pet.pose();
    let dock = pets.dock;
    let hovered = Hover::new();
    let grown = pets.docked_scale;
    let size = move || {
        let scale = DOCKED * grown.get_untracked();
        (f64::from(CELL.0) * scale, f64::from(CELL.1) * scale)
    };
    // A /kill throws it as far left as the window allows.
    let throw_pet = pet.clone();
    Effect::new(move |_| {
        let (left, _) = dock.get();
        throw_pet
            .throw_distance
            .set(((left - 6.0) / 1.15).clamp(0.0, 300.0));
    });

    // Grabbed: where the pointer was on it, whether it has moved, and where
    // it has been lately, to throw it with.
    struct Grab {
        offset: Point,
        moved: bool,
        trail: Vec<(std::time::Instant, (f64, f64))>,
        /// Swung round, it winds up.
        windup: Windup,
        pointer: Point,
    }
    let grab: Rc<RefCell<Option<Grab>>> = Rc::default();
    let body = sprite(&pet, DOCKED);
    let body_id = body.view_id();
    pet.sprite_view.set(Some(body_id));
    let window_size = move || body_id.root().get_size().unwrap_or_default();
    let (down_grab, move_grab, up_grab) = (grab.clone(), grab.clone(), grab);
    let (down_state, move_state) = (state.clone(), state.clone());
    let (move_pet, up_pet, up_pets) = (pet.clone(), pet.clone(), pets.clone());
    let body = body
        .on_event_stop(el::PointerDown, move |cx, event| {
            if let Some(id) = event.pointer.pointer_id {
                cx.request_pointer_capture(id);
            }
            let at = down_state.pointer();
            let (left, bottom) = dock.get_untracked();
            let top = window_size().height - bottom - size().1;
            *down_grab.borrow_mut() = Some(Grab {
                offset: Point::new(at.x - left, at.y - top),
                moved: false,
                trail: Vec::new(),
                windup: Windup::default(),
                pointer: at,
            });
        })
        .on_event_stop(el::PointerMove, move |_, _| {
            let mut grab = move_grab.borrow_mut();
            let Some(grab) = grab.as_mut() else { return };
            let at = move_state.pointer();
            let window = window_size();
            let size = size();
            let left = (at.x - grab.offset.x).clamp(0.0, (window.width - size.0).max(0.0));
            let top = (at.y - grab.offset.y).clamp(0.0, (window.height - size.1).max(0.0));
            let (before, _) = dock.get_untracked();
            if !grab.moved && (left - before).abs() < 2.0 {
                return;
            }
            grab.moved = true;
            grab.windup
                .moved(at.x - grab.pointer.x, at.y - grab.pointer.y);
            grab.pointer = at;
            if grab.windup.turns() > 0.15 {
                // Swung round, it turns with the swing.
                if move_pet.moving.get_untracked().is_some() {
                    move_pet.moving.set(None);
                }
                move_pet.swing.set(Some(grab.windup.turned));
            } else if (left - before).abs() > 0.5 {
                move_pet.moving.set(Some(left > before));
            }
            let now = std::time::Instant::now();
            grab.trail.push((now, (left, top)));
            grab.trail
                .retain(|(when, _)| now - *when < std::time::Duration::from_millis(200));
            dock.set((left, window.height - top - size.1));
        })
        .on_event_stop(el::PointerUp, move |_, _| {
            let Some(grab) = up_grab.borrow_mut().take() else {
                return;
            };
            up_pet.moving.set(None);
            if !grab.moved {
                up_pet.poke();
                return;
            }
            // Thrown hard, it leaves the window, from where it is on the
            // screen, faster and spinning for every turn it was swung; let
            // go gently, it stays where it is put.
            let (velocity, spin) = grab.windup.throw(release_velocity(&grab.trail));
            if !(cfg!(windows) && velocity.0.hypot(velocity.1) > FLING_SPEED) {
                up_pet.swing.set(None);
            } else {
                let sprite = body_id.get_visual_rect();
                let origin = main
                    .position_on_screen_including_frame()
                    .zip(main.position_of_content_on_screen())
                    .map(|(window, content)| {
                        Point::new(window.x + content.x, window.y + content.y)
                    });
                if let Some(origin) = origin {
                    let at = Point::new(origin.x + sprite.x0, origin.y + sprite.y0);
                    up_pets.fling_out(key, at, velocity, spin);
                }
            }
        });
    let figure = Stack::new((
        body,
        grips(&state, &pet, hovered),
        controls(&pets, &pet, hovered),
    ))
    .style(move |s| s.rotate(floem::unit::Angle::Rad(pose.get().angle)));
    // The bubble moves left to stay in the window near its right edge.
    let said = bubble(&state, &pet).style(move |s| {
        let (left, _) = dock.get();
        let room = window.get().width - left - BUBBLE_WIDTH - 8.0;
        s.translate_x(room.min(0.0).max(-left))
    });
    // Thrown or hopping, what it says goes with it.
    Stack::vertical((said, chat(&state, &pet), figure))
        .style(move |s| {
            let thrown = pose.get().thrown;
            s.items_start()
                .gap(4)
                .translate_x(thrown.0)
                .translate_y(thrown.1)
        })
        .on_event_cont(el::PointerEnter, move |_, _| hovered.enter())
        .on_event_cont(el::PointerLeave, move |_, _| hovered.leave())
}

/// The window's content shakes while a `/kill` lands.
pub(super) fn shake(state: &Rc<Workbench>) -> ReadSignal<(f64, f64)> {
    state.pets.shake()
}

/// Opens and closes the popped-out pet as it pops out and in.
pub(super) fn follow_popping(state: Rc<Workbench>, main: WindowId) {
    let pets = state.pets.clone();
    let (out, hidden) = (pets.out, pets.hidden);
    let current: Rc<RefCell<Option<Outside>>> = Rc::default();
    Effect::new(move |_| {
        let wanted = out.get().filter(|_| !hidden.get());
        let showing = current.borrow().as_ref().map(|o| o.key);
        if showing == wanted {
            return;
        }
        if let Some(old) = current.borrow_mut().take() {
            old.close(&pets);
        }
        if let Some(key) = wanted {
            match Outside::open(&state, main, key) {
                Some(outside) => *current.borrow_mut() = Some(outside),
                // It cannot go out; it stays in.
                None => pets.out.set(None),
            }
        }
    });
}

/// Where the popped-out pet first appears: low on the workbench, right of
/// the rail, with room to be thrown left (logical pixels). The content
/// bounds give the size; their place is not on screen.
fn start_point(main: WindowId, height: f64) -> Point {
    main.position_on_screen_including_frame()
        .zip(main.bounds_of_content_on_screen())
        .map(|(at, bounds)| Point::new(at.x + 260.0, at.y + bounds.height() - height - 40.0))
        .unwrap_or(Point::new(100.0, 100.0))
}

/// The popped-out pet and what it needs closing.
struct Outside {
    key: u64,
    scope: floem::reactive::Scope,
    /// Held for as long as it is out; dropping it closes it.
    #[cfg(windows)]
    _body: Rc<Body>,
}

impl Outside {
    fn close(self, pets: &Rc<Pets>) {
        self.scope.dispose();
        if let PetWindow::Open(id) = pets.window.replace(PetWindow::Closed) {
            floem::close_window(id);
        }
    }

    /// The pet's speech in a window of its own that fits it, or (`card`)
    /// the pet and its speech together.
    fn speech_window(state: &Rc<Workbench>, pet: &Rc<Pet>, at: Point, card: bool) {
        state.pets.window.set(PetWindow::Pending);
        let (state, pet) = (state.clone(), pet.clone());
        let size = if card {
            card_size(None)
        } else {
            Size::new(BUBBLE_WIDTH + 2.0, 40.0)
        };
        floem::new_window(
            move |id| {
                // Popped back in, or the workbench closed, before the window
                // was made: it closes straight away.
                if state.pets.window.get() != PetWindow::Pending
                    || state.pets.out.try_get_untracked().flatten() != Some(pet.key)
                {
                    floem::close_window(id);
                    return Empty::new().into_any();
                }
                state.pets.window.set(PetWindow::Open(id));
                if card {
                    card_view(state, pet, id).into_any()
                } else {
                    speech_view(state, pet, id).into_any()
                }
            },
            Some(
                WindowConfig::default()
                    .size(size)
                    .position(at)
                    .title("Glitch says")
                    .undecorated(true)
                    .show_titlebar(false)
                    .resizable(false)
                    .window_level(WindowLevel::AlwaysOnTop)
                    .with_win_os_config(|c| {
                        c.set_skip_taskbar(true)
                            .corner_preference(floem::window::WinOsCornerPreference::Round)
                    }),
            ),
        );
    }
}

/// The popped-out card where windows cannot be see-through: the pet with
/// its bubble above, growing upwards to fit.
fn card_size(content: Option<Size>) -> Size {
    let pet = Size::new(f64::from(CELL.0) * POPPED, f64::from(CELL.1) * POPPED);
    let content = content.unwrap_or(pet);
    Size::new(
        content.width.max(pet.width) + 16.0,
        content.height.max(pet.height) + 16.0,
    )
}

/// Keeps a window the size of its content, its bottom left where it is.
fn fit_window(id: WindowId, content: floem::ViewId, size: impl Fn(Option<Size>) -> Size) {
    let wanted = size(
        content
            .get_size()
            .filter(|s| s.width > 1.0 && s.height > 1.0),
    );
    let (Some(now), Some(at)) = (
        id.bounds_of_content_on_screen(),
        id.position_on_screen_including_frame(),
    ) else {
        return;
    };
    if (now.width() - wanted.width).abs() > 1.0 || (now.height() - wanted.height).abs() > 1.0 {
        id.set_content_size(wanted);
        id.set_outer_location(Point::new(at.x, at.y + now.height() - wanted.height));
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn card_view(state: Rc<Workbench>, pet: Rc<Pet>, id: WindowId) -> impl IntoView {
    let pets = state.pets.clone();
    let hovered = Hover::new();
    // Thrown from where each /kill finds the window.
    let from = Rc::new(Cell::new(None::<(u64, Point)>));
    let (shot, pose) = (pet.shot(), pet.pose());
    let throw_pet = pet.clone();
    Effect::new(move |_| {
        let thrown = pose.get().thrown;
        let Some(began) = shot.get() else {
            from.set(None);
            return;
        };
        let start = match from.get() {
            Some((shot, start)) if shot == began => start,
            _ => {
                let Some(start) = id.position_on_screen_including_frame() else {
                    return;
                };
                if let Some(screen) = id.monitor_bounds() {
                    throw_pet
                        .throw_distance
                        .set(((start.x - screen.x0) / 1.15).clamp(0.0, 420.0));
                }
                from.set(Some((began, start)));
                start
            }
        };
        id.set_outer_location(Point::new(start.x + thrown.0, start.y + thrown.1));
    });
    let pressed = Rc::new(Cell::new(None::<Point>));
    let (down, moved, up) = (pressed.clone(), pressed.clone(), pressed);
    let poke_pet = pet.clone();
    let body = sprite(&pet, POPPED)
        .on_event_stop(el::PointerDown, move |_, event| {
            down.set(Some(event.state.logical_point()));
        })
        .on_event_stop(el::PointerMove, move |_, event| {
            if let Some(from) = moved.get()
                && from.distance(event.current.logical_point()) > 3.0
            {
                moved.set(None);
                floem::action::drag_window();
            }
        })
        .on_event_stop(el::PointerUp, move |_, _| {
            if up.take().is_some() {
                poke_pet.poke();
            }
        });
    let content = Stack::vertical((
        bubble(&state, &pet),
        Stack::new((body, controls(&pets, &pet, hovered))),
    ))
    .style(|s| s.absolute().inset_left(8).inset_top(8).gap(6).items_start());
    let content_id = content.view_id();
    let fit_pet = pet.clone();
    Stack::new((content,))
        .style(|s| {
            s.size_full()
                .background(color::bg())
                .color(color::text())
                .font_size(13)
                .border(1)
                .border_color(color::border())
        })
        .on_event_cont(el::UpdatePhaseLayout, move |_, _| {
            if fit_pet.shot().get_untracked().is_none() {
                fit_window(id, content_id, card_size);
            }
        })
        .on_event_cont(el::PointerEnter, move |_, _| hovered.enter())
        .on_event_cont(el::PointerLeave, move |_, _| hovered.leave())
        .on_event_cont(el::WindowClosed, move |_, _| {
            // Closed by the system (Alt+F4, say): the pet comes back in.
            if pets.window.get() == PetWindow::Open(id) {
                pets.window.set(PetWindow::Closed);
                pets.out.try_update(|out| *out = None);
            }
        })
}

/// The popped-out pet's speech beside its see-through sprite: shown only
/// while it has something to say.
#[cfg_attr(not(windows), allow(dead_code))]
fn speech_view(state: Rc<Workbench>, pet: Rc<Pet>, id: WindowId) -> impl IntoView {
    // Laid out at its own size, not squeezed to the window's.
    let said = Stack::vertical((bubble(&state, &pet), chat(&state, &pet)))
        .style(|s| s.absolute().inset_left(0).inset_top(0).gap(4));
    let said_id = said.view_id();
    let pets = state.pets.clone();
    let speaking = {
        let pet = pet.clone();
        let (updates, open) = (pet.updates(), pet.chat_open);
        Memo::new(move |_| {
            open.get() || !pet.questions().is_empty() || updates.with(|u| !u.is_empty())
        })
    };
    #[cfg(windows)]
    pets.speech_hwnd.set(super::pet_window::native(id));
    // Shown without taking the keyboard from whatever you are typing in;
    // a click on it (to chat) gives it the keyboard.
    let shown_pets = pets.clone();
    Effect::new(move |_| {
        let show = speaking.get();
        #[cfg(windows)]
        if let Some(hwnd) = shown_pets.speech_hwnd.get() {
            super::pet_window::show_window(hwnd, show);
            return;
        }
        let _ = &shown_pets;
        id.set_visible(show);
    });
    Stack::new((said,))
        .style(|s| {
            s.size_full()
                .background(color::panel())
                .color(color::text())
                .font_size(13)
        })
        .on_event_cont(el::UpdatePhaseLayout, move |_, _| {
            fit_window(id, said_id, |content| {
                content.unwrap_or(Size::new(BUBBLE_WIDTH, 40.0))
            });
        })
        .on_event_cont(el::WindowClosed, move |_, _| {
            // Closed by the system (Alt+F4, say): the pet comes back in
            // rather than staying out without a word.
            if pets.window.get() == PetWindow::Open(id) {
                pets.window.set(PetWindow::Closed);
                pets.speech_hwnd.set(None);
                pets.out.try_update(|out| *out = None);
            }
        })
}

#[cfg(not(windows))]
impl Outside {
    fn open(state: &Rc<Workbench>, main: WindowId, key: u64) -> Option<Self> {
        let pet = state.pets.pet(key);
        let scope = floem::reactive::Scope::new();
        let at = start_point(main, card_size(None).height);
        scope.enter(|| Outside::speech_window(state, &pet, at, true));
        Some(Outside { key, scope })
    }
}

/// The popped-out pet's see-through window. It is square, wide enough to
/// turn the pet in, so the pet's own place is inset in it.
/// A resize under way: the feet that stay put, where the pointer started,
/// and the size it started from.
#[cfg(windows)]
type Resize = ((i32, i32), (i32, i32), f64);

#[cfg(windows)]
struct Body {
    layered: Rc<super::pet_window::Layered>,
    width: Cell<u32>,
    height: Cell<u32>,
    side: Cell<u32>,
    /// Being made bigger: where its feet stay (physical), the pointer and
    /// size it started from.
    resizing: Cell<Option<Resize>>,
}

#[cfg(windows)]
impl Body {
    /// The pixels per logical pixel of the screen it is on now, which
    /// changes as it is dragged from one screen to another.
    fn scale(&self) -> f64 {
        self.layered.scale()
    }
    /// Sized for a scale (the pet's size as a multiple of the usual).
    fn sized(&self, grown: f64) {
        let width = (f64::from(CELL.0) * POPPED * grown * self.scale()).round() as u32;
        let height = (f64::from(CELL.1) * POPPED * grown * self.scale()).round() as u32;
        self.width.set(width);
        self.height.set(height);
        self.side
            .set(f64::from(width).hypot(f64::from(height)).ceil() as u32 + 2);
    }
    fn width(&self) -> u32 {
        self.width.get()
    }
    fn height(&self) -> u32 {
        self.height.get()
    }
    fn pad(&self) -> (i32, i32) {
        (
            ((self.side.get() - self.width()) / 2) as i32,
            ((self.side.get() - self.height()) / 2) as i32,
        )
    }
    /// Whether a screen point is on one of the pet's corners.
    fn on_corner(&self, at: (i32, i32)) -> bool {
        let (x, y) = self.place();
        let (w, h) = (f64::from(self.width()), f64::from(self.height()));
        let corners = [
            (FIGURE.0, FIGURE.1),
            (FIGURE.2, FIGURE.1),
            (FIGURE.0, FIGURE.3),
            (FIGURE.2, FIGURE.3),
        ];
        let reach = 14.0 * self.scale();
        corners.iter().any(|(cx, cy)| {
            let corner = (f64::from(x) + w * cx, f64::from(y) + h * cy);
            (f64::from(at.0) - corner.0).abs() < reach && (f64::from(at.1) - corner.1).abs() < reach
        })
    }
    /// Its feet (physical).
    fn feet(&self) -> (i32, i32) {
        let (x, y) = self.place();
        (x + self.width() as i32 / 2, y + self.height() as i32)
    }
    /// The pet's own place on the screen: left, top (physical).
    fn place(&self) -> (i32, i32) {
        let (x, y, _, _) = self.layered.rect();
        let pad = self.pad();
        (x + pad.0, y + pad.1)
    }
    fn move_to(&self, at: (i32, i32)) {
        let pad = self.pad();
        self.layered.move_to((at.0 - pad.0, at.1 - pad.1));
    }
    fn paint(&self, pixels: &[u8], angle: f64) {
        let side = self.side.get();
        let canvas = turned(pixels, self.width(), self.height(), side, angle);
        self.layered.draw(&canvas, side, side, None);
    }
    /// Its speech, just above its antenna (the pet at `at`, physical).
    fn speech_follows(&self, pets: &Pets, at: (i32, i32)) {
        if let Some(hwnd) = pets.speech_hwnd.get() {
            let (left, now, _, height) = super::pet_window::window_rect(hwnd);
            let top = at.1 + (f64::from(self.height()) * HEADROOM) as i32 - height - 4;
            if (left, now) != (at.0, top) {
                super::pet_window::move_window(hwnd, (at.0, top));
            }
        }
    }
}

#[cfg(windows)]
impl Outside {
    /// The pet alone, see-through, on top of everything, with its speech
    /// in a small window above it that goes where it goes. `None` if the
    /// system would not make the window.
    fn open(state: &Rc<Workbench>, main: WindowId, key: u64) -> Option<Self> {
        use super::pet_window::{Events, Layered};
        let pets = state.pets.clone();
        let pet = pets.pet(key);
        let scope = floem::reactive::Scope::new();

        struct Handler {
            pets: std::rc::Weak<Pets>,
            pet: std::rc::Weak<Pet>,
            body: RefCell<Option<std::rc::Weak<Body>>>,
            /// Where it was while dragged, to throw it with, and how far
            /// round it has been swung.
            trail: RefCell<Vec<(std::time::Instant, (f64, f64))>>,
            windup: RefCell<Windup>,
        }
        impl Handler {
            fn body(&self) -> Option<Rc<Body>> {
                self.body.borrow().as_ref().and_then(|b| b.upgrade())
            }
        }
        impl Events for Handler {
            fn grip(&self, at: (i32, i32)) -> bool {
                self.body().is_some_and(|body| body.on_corner(at))
            }
            fn resize(&self, at: (i32, i32)) {
                let (Some(pets), Some(body)) = (self.pets.upgrade(), self.body()) else {
                    return;
                };
                let (feet, from, was) = match body.resizing.get() {
                    Some(started) => started,
                    None => {
                        let started = (body.feet(), at, pets.popped_scale.get_untracked());
                        body.resizing.set(Some(started));
                        started
                    }
                };
                let distance =
                    |p: (i32, i32)| f64::from(p.0 - feet.0).hypot(f64::from(p.1 - feet.1));
                let now = (was * distance(at) / distance(from).max(8.0)).clamp(GROWN.0, GROWN.1);
                pets.popped_scale.set(now);
            }
            fn resized(&self) {
                if let Some(body) = self.body() {
                    body.resizing.set(None);
                }
            }
            fn clicked(&self) {
                if let Some(pet) = self.pet.upgrade() {
                    pet.poke();
                }
            }
            fn dragged(&self, at: (i32, i32), dx: i32) {
                let (Some(pets), Some(pet), Some(body)) =
                    (self.pets.upgrade(), self.pet.upgrade(), self.body())
                else {
                    return;
                };
                let scale = body.scale();
                let here = (f64::from(at.0) / scale, f64::from(at.1) / scale);
                let now = std::time::Instant::now();
                {
                    let mut trail = self.trail.borrow_mut();
                    if let Some(&(_, last)) = trail.last() {
                        self.windup
                            .borrow_mut()
                            .moved(here.0 - last.0, here.1 - last.1);
                    }
                    trail.push((now, here));
                    trail.retain(|(when, _)| now - *when < std::time::Duration::from_millis(200));
                }
                let windup = *self.windup.borrow();
                if windup.turns() > 0.15 {
                    if pet.moving.get_untracked().is_some() {
                        pet.moving.set(None);
                    }
                    pet.swing.set(Some(windup.turned));
                } else if dx != 0 && pet.moving.get_untracked() != Some(dx > 0) {
                    pet.moving.set(Some(dx > 0));
                }
                // Its speech keeps up, though the app's timers wait while the
                // system moves the window.
                let pad = body.pad();
                body.speech_follows(&pets, (at.0 + pad.0, at.1 + pad.1));
                pets.advance();
            }
            fn dropped(&self) {
                let trail = std::mem::take(&mut *self.trail.borrow_mut());
                let windup = std::mem::take(&mut *self.windup.borrow_mut());
                let (Some(pets), Some(pet), Some(body)) =
                    (self.pets.upgrade(), self.pet.upgrade(), self.body())
                else {
                    return;
                };
                pet.moving.set(None);
                let (velocity, spin) = windup.throw(release_velocity(&trail));
                if velocity.0.hypot(velocity.1) > FLING_SPEED {
                    // Off it goes, inside the screen, onto the taskbar.
                    let scale = body.scale();
                    let (x, y) = body.place();
                    let (w, h) = (body.width() as i32, body.height() as i32);
                    let (left, top, right, bottom) = body.layered.work_area();
                    let bounds = Bounds {
                        left: f64::from(left - x) / scale,
                        right: f64::from(right - w - x) / scale,
                        top: f64::from(top - y) / scale,
                        floor: f64::from(bottom - h - y) / scale,
                    };
                    pet.launch((0.0, 0.0), velocity, spin, bounds, false);
                    return;
                }
                pet.swing.set(None);
                body.speech_follows(&pets, body.place());
            }
            fn chose(&self, index: usize) {
                if let (Some(pets), Some(pet)) = (self.pets.upgrade(), self.pet.upgrade()) {
                    match index {
                        0 => pet.chat_open.update(|o| *o = !*o),
                        1 => pets.pop_in(),
                        _ => pets.toggle_hidden(),
                    }
                }
            }
            fn menu(&self) -> Vec<&'static str> {
                vec!["Chat", "Pop in", "Hide"]
            }
        }

        let handler = Rc::new(Handler {
            pets: Rc::downgrade(&pets),
            pet: Rc::downgrade(&pet),
            body: RefCell::new(None),
            trail: RefCell::new(Vec::new()),
            windup: RefCell::new(Windup::default()),
        });
        let layered = Rc::new(Layered::open((0, 0), handler.clone())?);
        let body = Rc::new(Body {
            layered,
            width: Cell::new(0),
            height: Cell::new(0),
            side: Cell::new(0),
            resizing: Cell::new(None),
        });
        *handler.body.borrow_mut() = Some(Rc::downgrade(&body));
        let grown_now = pets.popped_scale.get_untracked();
        body.sized(grown_now);

        // Where it starts comes in the workbench's logical pixels; its own
        // window is in the physical pixels of whichever screen that is.
        let main_scale = main.scale();
        let launch = pets.take_launch();
        let start = launch.map_or_else(
            || start_point(main, f64::from(body.height()) / body.scale()),
            |l| l.0,
        );
        let home = (
            (start.x * main_scale).round() as i32,
            (start.y * main_scale).round() as i32,
        );
        body.move_to(home);
        // Now on that screen, sized for it.
        body.sized(grown_now);
        body.move_to(home);
        let pose_now = pet.pose().get_untracked();
        body.paint(
            &sheet(color::is_dark()).pixels(
                pose_now.anim.row(),
                pose_now.column,
                body.width(),
                body.height(),
            ),
            pose_now.angle,
        );
        // Thrown out of the window, it carries on from where it left it.
        if let Some((_, velocity, spin)) = launch {
            let scale = body.scale();
            let (width, height) = (body.width() as i32, body.height() as i32);
            let (left, top, right, bottom) = body.layered.work_area();
            let bounds = Bounds {
                left: f64::from(left - home.0) / scale,
                right: f64::from(right - width - home.0) / scale,
                top: f64::from(top - home.1) / scale,
                floor: f64::from(bottom - height - home.1) / scale,
            };
            pet.launch((0.0, 0.0), velocity, spin, bounds, false);
        }

        scope.enter(|| {
            let pose = pet.pose();
            // Drawn when the frame, its angle or the theme changes.
            let grown = pets.popped_scale;
            let look = Memo::new(move |_| {
                let pose = pose.get();
                (
                    pose.anim.row(),
                    pose.column,
                    (pose.angle * 100.0).round() as i64,
                    color::is_dark(),
                    (grown.get() * 1000.0).round() as i64,
                )
            });
            let paint_body = body.clone();
            let paint_pets = pets.clone();
            Effect::new(move |_| {
                let (row, column, angle, dark, size) = look.get();
                // Grown or shrunk, its feet stay where they were.
                let size = size as f64 / 1000.0;
                let feet = paint_body.feet();
                let resized = (f64::from(paint_body.width())
                    - (f64::from(CELL.0) * POPPED * size * paint_body.scale()).round())
                .abs()
                    > 0.5;
                if resized {
                    let feet = paint_body.resizing.get().map_or(feet, |r| r.0);
                    paint_body.sized(size);
                    paint_body.move_to((
                        feet.0 - paint_body.width() as i32 / 2,
                        feet.1 - paint_body.height() as i32,
                    ));
                }
                let pixels =
                    sheet(dark).pixels(row, column, paint_body.width(), paint_body.height());
                paint_body.paint(&pixels, angle as f64 / 100.0);
                if resized {
                    paint_body.speech_follows(&paint_pets, paint_body.place());
                }
            });
            // Moved while thrown by a /kill, flung or hopping, from where
            // each found it; its speech goes with it.
            let (shot, flight) = (pet.shot(), pet.flight());
            let from = Cell::new(None::<((bool, u64), (i32, i32))>);
            let (move_body, move_pets, throw_pet) = (body.clone(), pets.clone(), pet.clone());
            Effect::new(move |_| {
                let pose = pose.get();
                let motion = shot
                    .get()
                    .map(|began| (true, began))
                    .or_else(|| flight.with(|f| f.map(|f| (false, f.id))));
                let Some(motion) = motion else {
                    from.set(None);
                    return;
                };
                let start = match from.get() {
                    Some((was, start)) if was == motion => start,
                    _ => {
                        let start = move_body.place();
                        if motion.0 {
                            // As far left as its screen allows.
                            let room = f64::from(start.0 - move_body.layered.work_area().0);
                            throw_pet
                                .throw_distance
                                .set((room / move_body.scale() / 1.15).clamp(0.0, 420.0));
                        }
                        from.set(Some((motion, start)));
                        start
                    }
                };
                let scale = move_body.scale();
                let dx = ((pose.thrown.0 + pose.shake.0) * scale).round() as i32;
                let dy = ((pose.thrown.1 + pose.shake.1) * scale).round() as i32;
                let at = (start.0 + dx, start.1 + dy);
                move_body.move_to(at);
                move_body.speech_follows(&move_pets, at);
            });
            // When it starts or stops talking, or its chat opens, its speech
            // finds it at once, frame or no frame.
            let (talk_body, talk_pets, talk_pet) = (body.clone(), pets.clone(), pet.clone());
            let (updates, chat_open) = (pet.updates(), pet.chat_open);
            Effect::new(move |_| {
                updates.track();
                chat_open.track();
                let _ = talk_pet.questions();
                let (body, pets) = (talk_body.clone(), talk_pets.clone());
                floem::action::exec_after(std::time::Duration::from_millis(60), move |_| {
                    body.speech_follows(&pets, body.place());
                });
            });
            // Looks at the pointer anywhere on the screen; and, every frame,
            // its speech is just above it, however the speech has grown
            // (opening the chat, say) or wherever it went.
            let (look_body, look_pet, look_pets) = (body.clone(), pet.clone(), pets.clone());
            Effect::new(move |_| {
                pose.track();
                look_body.speech_follows(&look_pets, look_body.place());
                let (x, y) = look_body.place();
                let face = (
                    f64::from(x) + f64::from(look_body.width()) / 2.0,
                    f64::from(y) + f64::from(look_body.height()) * 0.62,
                );
                let at = super::pet_window::pointer();
                let scale = look_body.scale();
                look_pet.look_at(
                    (face.0 / scale, face.1 / scale),
                    (f64::from(at.0) / scale, f64::from(at.1) / scale),
                );
            });
            let speech_at = Point::new(
                f64::from(home.0) / main_scale,
                f64::from(home.1) / main_scale + f64::from(body.height()) / main_scale * HEADROOM
                    - 44.0,
            );
            Outside::speech_window(state, &pet, speech_at, false);
        });
        Some(Outside {
            key,
            scope,
            _body: body,
        })
    }
}
