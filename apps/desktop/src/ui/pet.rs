//! The pet on screen: its sprite, cut from the sheet for the theme; what it
//! has to say; and where it lives, docked in the window or popped out into
//! a small window of its own that a `/kill` throws across the screen.
use super::{color, session::ask_card};
use crate::{
    pet::{CELL, Pet, PetWindow},
    workspace::Workbench,
};
use floem::{
    kurbo::{Point, Rect, Size},
    peniko::{self, Blob, ImageAlphaType, ImageData},
    prelude::*,
    reactive::{Effect, Memo},
    style::CursorStyle,
    views::canvas,
    window::{WinOsCornerPreference, WindowConfig, WindowId, WindowIdExt, WindowLevel},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

const DARK: &[u8] = include_bytes!("../../assets/pet/glitch-dark.png");
const LIGHT: &[u8] = include_bytes!("../../assets/pet/glitch-light.png");

/// Docked, the pet is drawn at half size; popped out, a little larger.
const DOCKED: f64 = 0.5;
const POPPED: f64 = 0.75;
const BUBBLE_WIDTH: f64 = 280.0;

/// Space around the pet and its bubble in the popped-out card.
const CARD_PADDING: f64 = 8.0;

/// A frame cut from the sheet, and its key in the renderer's cache.
type Frame = (peniko::ImageBrush, Vec<u8>);

/// One theme's sheet, and the frames cut from it so far.
struct Sheet {
    image: image::RgbaImage,
    dark: bool,
    frames: RefCell<HashMap<(u32, u32), Frame>>,
}

impl Sheet {
    fn frame(&self, row: u32, column: u32) -> Frame {
        self.frames
            .borrow_mut()
            .entry((row, column))
            .or_insert_with(|| {
                let (width, height) = CELL;
                let cell = image::imageops::crop_imm(
                    &self.image,
                    column * width,
                    row * height,
                    width,
                    height,
                )
                .to_image();
                // Nearest-neighbour, so the pixel art stays crisp when scaled.
                let brush = peniko::ImageBrush::new(ImageData {
                    data: Blob::new(Arc::new(cell.into_raw())),
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

/// The pet itself, at a scale of its 192×208 cell.
fn sprite(pet: &Rc<Pet>, scale: f64) -> floem::views::Canvas {
    let pose = pet.pose();
    canvas(move |cx, size| {
        let pose = pose.get();
        // Ivory on dark themes would glare; the charcoal one suits them.
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

/// The pet's speech bubble: a session's question, answered right there,
/// or news from any session, which opens it when clicked.
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
                Stack::vertical((
                    Label::derived(move || format!("{} asks", title.get()))
                        .style(|s| {
                            s.color(color::muted())
                                .font_size(11)
                                .cursor(CursorStyle::Pointer)
                                .hover(|s| s.color(color::accent()))
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

/// A small pill shown on hover: pop out or in, or hide.
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

/// The pills, over the empty top of the sprite's cell so showing them
/// moves nothing.
fn controls(pet: &Rc<Pet>, hovered: RwSignal<bool>) -> impl IntoView {
    let (pop_pet, hide_pet) = (pet.clone(), pet.clone());
    let popped = pet.popped;
    Stack::horizontal((
        control(
            move || if popped.get() { "Pop in" } else { "Pop out" },
            move || pop_pet.toggle_popped(),
        ),
        control(|| "Hide", move || hide_pet.toggle_hidden()),
    ))
    .style(move |s| {
        s.absolute()
            .inset_top(2)
            .inset_left(0)
            .gap(4)
            .apply_if(!hovered.get(), |s| s.hide())
    })
}

/// The pet docked in the window: it sits at the bottom of the sessions
/// rail, drags anywhere, and is thrown left across the window by a `/kill`.
pub(super) fn docked(state: Rc<Workbench>) -> impl IntoView {
    let pet = state.pet.clone();
    let pose = pet.pose();
    let dock = pet.dock;
    let (popped, hidden) = (pet.popped, pet.hidden);
    let hovered = RwSignal::new(false);
    // Grabbed at this offset from the sprite's corner, and moved yet.
    let grabbed = RwSignal::new(None::<(Point, bool)>);
    let size = (f64::from(CELL.0) * DOCKED, f64::from(CELL.1) * DOCKED);
    // It is thrown as far as the window allows, up to a good distance.
    let throw_pet = pet.clone();
    Effect::new(move |_| {
        let (left, _) = dock.get();
        throw_pet
            .throw_distance
            .set(((left - 6.0) / 1.15).clamp(0.0, 300.0));
    });

    let body = sprite(&pet, DOCKED);
    let body_id = body.view_id();
    pet.sprite_view.set(Some(body_id));
    let window_size = move || body_id.root().get_size().unwrap_or_default();
    // The window's size, to keep the pet and its bubble in it.
    let window = RwSignal::new(Size::ZERO);
    let (press_state, move_state, up_pet) = (state.clone(), state.clone(), pet.clone());
    let move_pet = pet.clone();
    let body = body
        .on_event_stop(el::PointerDown, move |cx, event| {
            if let Some(id) = event.pointer.pointer_id {
                cx.request_pointer_capture(id);
            }
            let at = press_state.pointer();
            let (left, bottom) = dock.get_untracked();
            let window = window_size();
            let top = window.height - bottom - size.1;
            grabbed.set(Some((Point::new(at.x - left, at.y - top), false)));
        })
        .on_event_stop(el::PointerMove, move |_, _| {
            let Some((offset, _)) = grabbed.get_untracked() else {
                return;
            };
            let at = move_state.pointer();
            let window = window_size();
            let left = (at.x - offset.x).clamp(0.0, (window.width - size.0).max(0.0));
            let top = (at.y - offset.y).clamp(0.0, (window.height - size.1).max(0.0));
            let (before, _) = dock.get_untracked();
            if (left - before).abs() > 0.5 {
                move_pet.moving.set(Some(left > before));
            }
            dock.set((left, window.height - top - size.1));
            grabbed.set(Some((offset, true)));
        })
        .on_event_stop(el::PointerUp, move |_, _| {
            let moved = grabbed.get_untracked().is_some_and(|(_, moved)| moved);
            grabbed.set(None);
            up_pet.moving.set(None);
            if !moved {
                up_pet.poke();
            }
        })
        .on_event_stop(el::LostPointerCapture, move |_, _| grabbed.set(None));

    // Only the pet is thrown; what it was saying stays put.
    let figure = Stack::new((body, controls(&pet, hovered))).style(move |s| {
        let thrown = pose.get().thrown;
        s.translate_x(thrown.0).translate_y(thrown.1)
    });
    // The bubble moves left to stay in the window when the pet is near its
    // right edge.
    let said = bubble(&state, &pet).style(move |s| {
        let (left, _) = dock.get();
        let room = window.get().width - left - BUBBLE_WIDTH - 8.0;
        s.translate_x(room.min(0.0).max(-left))
    });
    Stack::vertical((said, figure))
        .style(move |s| {
            let (left, bottom) = dock.get();
            let size_now = window.get();
            // Kept in reach however small the window gets.
            let (left, bottom) = if size_now.width > 0.0 {
                (
                    left.min(size_now.width - size.0).max(0.0),
                    bottom.min(size_now.height - size.1).max(0.0),
                )
            } else {
                (left, bottom)
            };
            s.absolute()
                .inset_left(left)
                .inset_bottom(bottom)
                .items_start()
                .gap(4)
                .z_index(25)
                .apply_if(popped.get() || hidden.get(), |s| s.hide())
        })
        .on_event_cont(el::PointerEnter, move |_, _| hovered.set(true))
        .on_event_cont(el::PointerLeave, move |_, _| hovered.set(false))
        .on_event_cont(el::UpdatePhaseLayout, move |_, _| {
            let now = window_size();
            if window.get_untracked() != now {
                window.set(now);
            }
        })
}

/// Opens and closes the pet's own window as it pops out and in, next to
/// the workbench's window.
pub(super) fn follow_popping(state: Rc<Workbench>, main: WindowId) {
    let pet = state.pet.clone();
    let (popped, hidden) = (pet.popped, pet.hidden);
    Effect::new(move |_| {
        let wanted = popped.get() && !hidden.get();
        match (wanted, pet.window.get()) {
            (true, PetWindow::Closed) => {
                pet.window.set(PetWindow::Pending);
                let size = card_size(None);
                // Low on the workbench, right of the rail, with room to be
                // thrown left. (Content bounds give the size, not the place.)
                let position = main
                    .position_on_screen_including_frame()
                    .zip(main.bounds_of_content_on_screen())
                    .map(|(at, bounds)| {
                        Point::new(at.x + 260.0, at.y + bounds.height() - size.height - 40.0)
                    })
                    .unwrap_or(Point::new(100.0, 100.0));
                let state = state.clone();
                floem::new_window(
                    move |id| {
                        // Popped back in, or the workbench closed, before
                        // the window was made: it closes straight away.
                        let pet = &state.pet;
                        let wanted = pet.popped.try_get_untracked().unwrap_or(false)
                            && !pet.hidden.try_get_untracked().unwrap_or(true)
                            && pet.window.get() == PetWindow::Pending;
                        if !wanted {
                            floem::close_window(id);
                            return Empty::new().into_any();
                        }
                        pet.window.set(PetWindow::Open(id));
                        window_view(state, id).into_any()
                    },
                    Some(
                        WindowConfig::default()
                            .size(size)
                            .position(position)
                            .title("Glitch")
                            .undecorated(true)
                            .show_titlebar(false)
                            .resizable(false)
                            .window_level(WindowLevel::AlwaysOnTop)
                            .with_win_os_config(|c| {
                                c.set_skip_taskbar(true)
                                    .corner_preference(WinOsCornerPreference::Round)
                            }),
                    ),
                );
            }
            (false, PetWindow::Open(id)) => {
                pet.window.set(PetWindow::Closed);
                floem::close_window(id);
            }
            (false, PetWindow::Pending) => pet.window.set(PetWindow::Closed),
            _ => {}
        }
    });
}

/// The popped-out card: the pet alone, or with what it has to say above.
fn card_size(content: Option<Size>) -> Size {
    let pet = Size::new(f64::from(CELL.0) * POPPED, f64::from(CELL.1) * POPPED);
    let content = content.unwrap_or(pet);
    Size::new(
        content.width.max(pet.width) + 2.0 * CARD_PADDING,
        content.height.max(pet.height) + 2.0 * CARD_PADDING,
    )
}

/// The popped-out window: a small card always on top, with the pet at the
/// bottom and its bubble above, growing upwards to fit. The pet drags the
/// window about and runs while it moves; a `/kill` throws the whole window
/// across the screen.
fn window_view(state: Rc<Workbench>, id: WindowId) -> impl IntoView {
    let pet = state.pet.clone();
    let hovered = RwSignal::new(false);
    // Where the window was when shot, and which shot, to throw it from.
    let shot_from = Rc::new(Cell::new(None::<(u64, Point)>));
    let shot = pet.shot();
    let pose = pet.pose();
    let throw_pet = pet.clone();
    let from = shot_from.clone();
    Effect::new(move |_| {
        let thrown = pose.get().thrown;
        let Some(began) = shot.get() else {
            from.set(None);
            return;
        };
        let start = match from.get() {
            Some((shot, start)) if shot == began => start,
            // A new shot, perhaps while the last was under way.
            _ => {
                let Some(start) = id.position_on_screen_including_frame() else {
                    return;
                };
                // As far left as the screen allows.
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

    // Pressing and moving drags the window; a press alone pokes the pet.
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
        Stack::new((body, controls(&pet, hovered))),
    ))
    .style(|s| {
        s.absolute()
            .inset_left(CARD_PADDING)
            .inset_top(CARD_PADDING)
            .gap(6)
            .items_start()
    });
    let content_id = content.view_id();
    // The card fits what it holds, keeping its bottom left where it is.
    let fit_pet = pet.clone();
    let fit = move || {
        if fit_pet.shot().get_untracked().is_some() {
            return;
        }
        let wanted = card_size(content_id.get_size());
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
    };

    // It runs the way the window is dragged.
    let last_x = Rc::new(Cell::new(None::<f64>));
    let settle = Rc::new(Cell::new(0u64));
    let run_pet = Rc::downgrade(&pet);
    let closed_pet = Rc::downgrade(&pet);
    Stack::new((content,))
        .style(|s| {
            s.size_full()
                .background(color::bg())
                .color(color::text())
                .font_size(13)
                .border(1)
                .border_color(color::border())
        })
        .on_event_cont(el::UpdatePhaseLayout, move |_, _| fit())
        .on_event_cont(el::PointerEnter, move |_, _| hovered.set(true))
        .on_event_cont(el::PointerLeave, move |_, _| hovered.set(false))
        .on_event_cont(el::WindowClosed, move |_, _| {
            // Closed by the system (Alt+F4, say) rather than by popping in.
            if let Some(pet) = closed_pet.upgrade()
                && pet.window.get() == PetWindow::Open(id)
            {
                pet.window.set(PetWindow::Closed);
                pet.popped.try_update(|p| *p = false);
            }
        })
        .on_event_cont(el::WindowMoved, move |_, at| {
            let Some(run_pet) = run_pet.upgrade() else {
                return;
            };
            if run_pet.shot().get_untracked().is_some() {
                return;
            }
            if let Some(before) = last_x.replace(Some(at.x))
                && (at.x - before).abs() > 0.5
            {
                run_pet.moving.set(Some(at.x > before));
                let serial = settle.get() + 1;
                settle.set(serial);
                let (pet, settle) = (Rc::downgrade(&run_pet), settle.clone());
                floem::action::exec_after(Duration::from_millis(180), move |_| {
                    if settle.get() == serial
                        && let Some(pet) = pet.upgrade()
                    {
                        pet.moving.try_update(|m| *m = None);
                    }
                });
            }
        })
}

/// The window's content shakes while a `/kill` lands.
pub(super) fn shake(pet: &Rc<Pet>) -> Memo<(f64, f64)> {
    let pose = pet.pose();
    Memo::new(move |_| pose.get().shake)
}
