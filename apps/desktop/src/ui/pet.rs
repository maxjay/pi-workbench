//! Pets on screen: the sprite, cut from the sheet for the theme; what a pet
//! has to say; each session's pet in its view; and the one popped out onto
//! the desktop, which a `/kill` throws across the screen.
use super::{color, session::ask_card};
use crate::{
    pet::{CELL, Pet, PetWindow, Pets},
    workspace::Workbench,
};
use floem::{
    kurbo::{Point, Rect, Size},
    peniko::{self, Blob, ImageAlphaType, ImageData},
    prelude::*,
    reactive::{Effect, Memo, ReadSignal},
    style::CursorStyle,
    views::canvas,
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
/// The empty top of a cell, above the pet's antenna, as a share of it.
#[cfg_attr(not(windows), allow(dead_code))]
const HEADROOM: f64 = 0.3;

/// A frame cut from the sheet, and its key in the renderer's cache.
type Frame = (peniko::ImageBrush, Vec<u8>);

/// One theme's sheet, and the frames cut from it so far.
struct Sheet {
    image: image::RgbaImage,
    dark: bool,
    frames: RefCell<HashMap<(u32, u32), Frame>>,
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
    fn pixels(&self, row: u32, column: u32, width: u32, height: u32) -> Vec<u8> {
        image::imageops::resize(
            &self.cell(row, column),
            width,
            height,
            image::imageops::FilterType::Nearest,
        )
        .into_raw()
    }
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
                })
            })
            .clone()
    })
}

/// A pet, at a scale of its 192×208 cell. Ivory on dark themes would glare;
/// the charcoal one suits them.
fn sprite(pet: &Rc<Pet>, scale: f64) -> floem::views::Canvas {
    let pose = pet.pose();
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
        s.width(f64::from(CELL.0) * scale)
            .height(f64::from(CELL.1) * scale)
            .cursor(CursorStyle::Pointer)
    })
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
fn controls(pets: &Rc<Pets>, pet: &Rc<Pet>, hovered: RwSignal<bool>) -> impl IntoView {
    let (pop_pets, hide_pets) = (pets.clone(), pets.clone());
    let out = pets.out;
    let key = pet.key;
    Stack::horizontal((
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
            .inset_right(0)
            .gap(4)
            .apply_if(!hovered.get(), |s| s.hide())
    })
}

/// A session's pet in its view: at the bottom right of the conversation,
/// talking about this session, and thrown left across it by a `/kill`.
pub(super) fn docked(state: Rc<Workbench>, key: u64) -> impl IntoView {
    let pets = state.pets.clone();
    let pet = pets.pet(key);
    let pose = pet.pose();
    let (out, hidden) = (pets.out, pets.hidden);
    let hovered = RwSignal::new(false);

    let body = sprite(&pet, DOCKED).on_event_stop(el::Click, {
        let pet = pet.clone();
        move |_, _| pet.poke()
    });
    pet.sprite_view.set(Some(body.view_id()));
    let figure = Stack::new((body, controls(&pets, &pet, hovered))).style(move |s| {
        let thrown = pose.get().thrown;
        s.translate_x(thrown.0).translate_y(thrown.1)
    });
    let view = Stack::vertical((bubble(&state, &pet), figure)).style(move |s| {
        s.absolute()
            .inset_right(14)
            .inset_bottom(4)
            .items_end()
            .gap(4)
            .z_index(5)
            .apply_if(out.get() == Some(key) || hidden.get(), |s| s.hide())
    });
    // Thrown as far left as the conversation allows.
    let view_id = view.view_id();
    let throw_pet = pet.clone();
    view.on_event_cont(el::PointerEnter, move |_, _| hovered.set(true))
        .on_event_cont(el::PointerLeave, move |_, _| hovered.set(false))
        .on_event_cont(el::UpdatePhaseLayout, move |_, _| {
            // Popped out, the screen decides how far it flies.
            if out.get_untracked() == Some(key) {
                return;
            }
            let room = view_id
                .parent()
                .map(|parent| parent.get_visual_rect())
                .zip(throw_pet.sprite_view.get().map(|v| v.get_visual_rect()))
                .map_or(0.0, |(area, sprite)| sprite.x0 - area.x0 - 8.0);
            throw_pet
                .throw_distance
                .set((room / 1.15).clamp(0.0, 320.0));
        })
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
            *current.borrow_mut() = Some(Outside::open(&state, main, key));
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
    _sprite: Rc<super::pet_window::Layered>,
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
    let hovered = RwSignal::new(false);
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
        .on_event_cont(el::PointerEnter, move |_, _| hovered.set(true))
        .on_event_cont(el::PointerLeave, move |_, _| hovered.set(false))
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
    let said = bubble(&state, &pet).style(|s| s.absolute().inset_left(0).inset_top(0));
    let said_id = said.view_id();
    let pets = state.pets.clone();
    let speaking = {
        let pet = pet.clone();
        let updates = pet.updates();
        Memo::new(move |_| !pet.questions().is_empty() || updates.with(|u| !u.is_empty()))
    };
    Effect::new(move |_| id.set_visible(speaking.get()));
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
            if pets.window.get() == PetWindow::Open(id) {
                pets.window.set(PetWindow::Closed);
            }
        })
}

#[cfg(not(windows))]
impl Outside {
    fn open(state: &Rc<Workbench>, main: WindowId, key: u64) -> Self {
        let pet = state.pets.pet(key);
        let scope = floem::reactive::Scope::new();
        let at = start_point(main, card_size(None).height);
        scope.enter(|| Outside::speech_window(state, &pet, at, true));
        Outside { key, scope }
    }
}

#[cfg(windows)]
impl Outside {
    /// The pet alone, see-through, on top of everything, with its speech
    /// in a small window above it.
    fn open(state: &Rc<Workbench>, main: WindowId, key: u64) -> Self {
        use super::pet_window::{Events, Layered};
        let pets = state.pets.clone();
        let pet = pets.pet(key);
        let scope = floem::reactive::Scope::new();

        struct Handler {
            pets: std::rc::Weak<Pets>,
            pet: std::rc::Weak<Pet>,
            /// Moves the speech above the pet after a drag.
            place_speech: Box<dyn Fn()>,
        }
        impl Events for Handler {
            fn clicked(&self) {
                if let Some(pet) = self.pet.upgrade() {
                    pet.poke();
                }
            }
            fn dragged(&self, dx: i32) {
                if let (Some(pets), Some(pet)) = (self.pets.upgrade(), self.pet.upgrade()) {
                    if pet.moving.get_untracked() != Some(dx > 0) {
                        pet.moving.set(Some(dx > 0));
                    }
                    // The app's timers wait while the system moves the window.
                    pets.advance();
                }
            }
            fn dropped(&self) {
                if let Some(pet) = self.pet.upgrade() {
                    pet.moving.set(None);
                }
                (self.place_speech)();
            }
            fn chose(&self, index: usize) {
                if let Some(pets) = self.pets.upgrade() {
                    match index {
                        0 => pets.pop_in(),
                        _ => pets.toggle_hidden(),
                    }
                }
            }
            fn menu(&self) -> Vec<&'static str> {
                vec!["Pop in", "Hide"]
            }
        }

        // Floem works in logical pixels, the window in physical ones.
        let speech_at = Rc::new(Cell::new(Point::ZERO));
        let sprite_cell: Rc<RefCell<Option<std::rc::Weak<Layered>>>> = Rc::default();
        let place = {
            let (pets, sprite_cell, speech_at) =
                (Rc::downgrade(&pets), sprite_cell.clone(), speech_at.clone());
            move || {
                let Some(sprite) = sprite_cell.borrow().as_ref().and_then(|s| s.upgrade()) else {
                    return;
                };
                let scale = sprite.scale();
                let (x, y, _, height) = sprite.rect();
                let top = f64::from(y) / scale + f64::from(height) / scale * HEADROOM;
                let at = Point::new(f64::from(x) / scale, top);
                speech_at.set(at);
                if let Some(pets) = pets.upgrade()
                    && let PetWindow::Open(id) = pets.window.get()
                    && let Some(size) = id.bounds_of_content_on_screen()
                {
                    id.set_outer_location(Point::new(at.x, at.y - size.height() - 4.0));
                }
            }
        };
        let handler = Rc::new(Handler {
            pets: Rc::downgrade(&pets),
            pet: Rc::downgrade(&pet),
            place_speech: Box::new(place.clone()),
        });
        let sprite = Rc::new(Layered::open((0, 0), handler).expect("a layered window opens"));
        *sprite_cell.borrow_mut() = Some(Rc::downgrade(&sprite));
        let scale = sprite.scale();
        let (width, height) = (
            (f64::from(CELL.0) * POPPED * scale).round() as u32,
            (f64::from(CELL.1) * POPPED * scale).round() as u32,
        );
        let start = start_point(main, f64::from(height) / scale);
        let home = (
            (start.x * scale).round() as i32,
            (start.y * scale).round() as i32,
        );

        scope.enter(|| {
            // Drawn whenever the frame, the throw or the theme changes; moved
            // only while thrown, so a drag is left alone.
            let pose = pet.pose();
            let shot = pet.shot();
            let from = Cell::new(None::<(u64, (i32, i32))>);
            let first = Cell::new(Some(home));
            let (draw_sprite, throw_pet) = (sprite.clone(), pet.clone());
            let after_throw = place.clone();
            Effect::new(move |_| {
                let pose = pose.get();
                let pixels =
                    sheet(color::is_dark()).pixels(pose.anim.row(), pose.column, width, height);
                let at = match shot.get() {
                    Some(began) => {
                        let start = match from.get() {
                            Some((shot, start)) if shot == began => start,
                            _ => {
                                let (x, y, _, _) = draw_sprite.rect();
                                // As far left as the screen allows.
                                throw_pet
                                    .throw_distance
                                    .set((f64::from(x) / scale / 1.15).clamp(0.0, 420.0));
                                from.set(Some((began, (x, y))));
                                (x, y)
                            }
                        };
                        let dx = (pose.thrown.0 + pose.shake.0) * scale;
                        let dy = (pose.thrown.1 + pose.shake.1) * scale;
                        Some((start.0 + dx.round() as i32, start.1 + dy.round() as i32))
                    }
                    None => {
                        if from.take().is_some() {
                            after_throw();
                        }
                        first.take()
                    }
                };
                draw_sprite.draw(&pixels, width, height, at);
            });
            // Looks at the pointer anywhere on the screen.
            let (look_sprite, look_pet) = (sprite.clone(), pet.clone());
            Effect::new(move |_| {
                pose.track();
                let (x, y, w, h) = look_sprite.rect();
                let face = (
                    f64::from(x) + f64::from(w) / 2.0,
                    f64::from(y) + f64::from(h) * 0.62,
                );
                let at = super::pet_window::pointer();
                look_pet.look_at(
                    (face.0 / scale, face.1 / scale),
                    (f64::from(at.0) / scale, f64::from(at.1) / scale),
                );
            });
            place();
            Outside::speech_window(state, &pet, speech_at.get(), false);
        });
        Outside {
            key,
            scope,
            _sprite: sprite,
        }
    }
}
