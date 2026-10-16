//! Glitch, the workbench's pet. Every session has its own: it lives in the
//! session's view, shows how that session is doing, repeats the gist of its
//! replies and brings its questions. Popped out of the window it sits on
//! the desktop, still that session's, with news and questions from the
//! others too. A session's `/kill`, for when its context has gone bad, gets
//! its pet shot and thrown, the one animation kept for that.
use crate::{
    session::{Entry, EntryKind, Session, Status},
    sessions::Sessions,
};
use floem::{
    action::exec_after,
    reactive::{Effect, Memo, ReadSignal, RwSignal, Scope, SignalGet, SignalUpdate, SignalWith},
};
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::HashMap,
    f64::consts::{PI, TAU},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

/// One of the sprite sheet's animations. `Look` is one frame, a direction
/// from 0 (up) clockwise in sixteenths of a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Anim {
    Idle,
    RunRight,
    RunLeft,
    Wave,
    Jump,
    /// Shot from the right and thrown left; only `/kill` plays it.
    Shot,
    Waiting,
    Working,
    Review,
    Look(u8),
}

/// The sheet: 8 columns of 192×208 cells, a row per animation, and two
/// rows of look directions.
pub const CELL: (u32, u32) = (192, 208);

impl Anim {
    /// The sheet row.
    pub fn row(self) -> u32 {
        match self {
            Anim::Idle => 0,
            Anim::RunRight => 1,
            Anim::RunLeft => 2,
            Anim::Wave => 3,
            Anim::Jump => 4,
            Anim::Shot => 5,
            Anim::Waiting => 6,
            Anim::Working => 7,
            Anim::Review => 8,
            Anim::Look(direction) => 9 + u32::from(direction % 16 / 8),
        }
    }

    /// How long each frame shows, in milliseconds, as the artwork was timed.
    pub fn durations(self) -> &'static [u64] {
        match self {
            Anim::Idle => &[280, 110, 110, 140, 140, 320],
            Anim::RunRight | Anim::RunLeft => &[120, 120, 120, 120, 120, 120, 120, 220],
            Anim::Wave => &[140, 140, 140, 280],
            Anim::Jump => &[140, 140, 140, 140, 280],
            Anim::Shot => &[140, 140, 140, 140, 140, 140, 140, 240],
            Anim::Waiting => &[150, 150, 150, 150, 150, 260],
            Anim::Working => &[120, 120, 120, 120, 120, 220],
            Anim::Review => &[150, 150, 150, 150, 150, 280],
            Anim::Look(_) => &[180],
        }
    }

    /// Played through once.
    pub fn length(self) -> u64 {
        self.durations().iter().sum()
    }

    /// The sheet column showing `ms` after the animation started: looping,
    /// or holding the last frame once played through.
    pub fn column(self, ms: u64, looping: bool) -> u32 {
        if let Anim::Look(direction) = self {
            return u32::from(direction % 8);
        }
        let durations = self.durations();
        let mut at = if looping { ms % self.length() } else { ms };
        for (index, duration) in durations.iter().enumerate() {
            if at < *duration {
                return index as u32;
            }
            at -= duration;
        }
        durations.len() as u32 - 1
    }
}

/// The direction, in sixteenths of a turn clockwise from up, of a point
/// seen from another (y grows downwards).
pub fn direction(from: (f64, f64), to: (f64, f64)) -> u8 {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let angle = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
    ((angle / 22.5).round() as u8) % 16
}

/// Where a `/kill` has got to: the frame, how far the pet has been thrown,
/// and how much the window shakes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotFrame {
    pub anim: Anim,
    pub column: u32,
    /// Offset from where it was shot, in logical pixels; left is negative.
    pub thrown: (f64, f64),
    pub shake: (f64, f64),
}

/// The moments of a `/kill`, in milliseconds.
const HIT: u64 = 140;
const LAND: u64 = 420;
const SETTLE: u64 = 700;
const SHAKE_FOR: u64 = 520;
const LIE_STILL: u64 = 3000;
/// Getting up where it landed.
const GET_UP: u64 = 700;
/// Walking back, in logical pixels per millisecond.
const WALK: f64 = 0.35;

/// Where a flung pet may go, as offsets from where it was let go (or, in a
/// conversation, from its place there): the walls, the ceiling and the
/// floor it lands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub floor: f64,
}

impl Bounds {
    /// A hop where it stands: nothing in the way, and the floor under it.
    pub const HOP: Bounds = Bounds {
        left: f64::NEG_INFINITY,
        right: f64::INFINITY,
        top: f64::NEG_INFINITY,
        floor: 0.0,
    };
}

/// A pet in the air (or rolling, or walking home), in logical pixels and
/// pixels per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    /// Tells one flight from another.
    pub id: u64,
    pub at: (f64, f64),
    pub velocity: (f64, f64),
    pub bounds: Bounds,
    /// Walks back to (0, 0) once down, as in a conversation.
    pub walk_home: bool,
    /// The hardest it hit something since the last step, to complain about.
    pub hit: f64,
    /// How far round it has turned, and how fast it turns (radians a second).
    pub angle: f64,
    pub spin: f64,
}

/// Pulls it down, in pixels per second per second.
const GRAVITY: f64 = 2600.0;
/// Of its speed into a wall or the floor, how much comes back.
const BOUNCE: f64 = 0.55;
/// Slower than this on the floor, it stops bouncing.
const SETTLE_SPEED: f64 = 140.0;

impl Flight {
    pub fn on_floor(&self) -> bool {
        self.at.1 >= self.bounds.floor - 0.5 && self.velocity.1.abs() < f64::EPSILON
    }

    /// Down, still, upright, and home if it walks there: over.
    pub fn done(&self) -> bool {
        self.on_floor()
            && self.velocity.0.abs() < 1.0
            && self.spin == 0.0
            && self.angle == 0.0
            && (!self.walk_home || self.at.0.abs() < 1.0)
    }

    /// Tumbling while it flies; on the floor it slows and rights itself.
    fn turn(&mut self, dt: f64) {
        self.angle += self.spin * dt;
        if self.on_floor() {
            self.spin *= 0.4_f64.powf(dt * 30.0);
            if self.spin.abs() < 0.6 {
                self.spin = 0.0;
                let upright = (self.angle + PI).rem_euclid(TAU) - PI;
                self.angle = upright * 0.7_f64.powf(dt * 30.0);
                if self.angle.abs() < 0.02 {
                    self.angle = 0.0;
                }
            }
        }
    }

    /// Moves it on `dt` seconds: falling, bouncing off the walls, ceiling
    /// and floor, rolling to a stop, then walking home if it does.
    pub fn step(mut self, dt: f64) -> Flight {
        let dt = dt.clamp(0.0, 0.05);
        let b = self.bounds;
        self.hit = 0.0;
        self.turn(dt);
        if self.on_floor() && self.velocity.0.abs() < 1.0 {
            // Down and still: walk home, or stay.
            if self.walk_home {
                let step = WALK * 1000.0 * dt;
                self.at.0 = if self.at.0 < 0.0 {
                    (self.at.0 + step).min(0.0)
                } else {
                    (self.at.0 - step).max(0.0)
                };
            }
            return self;
        }
        self.velocity.1 += GRAVITY * dt;
        self.at.0 += self.velocity.0 * dt;
        self.at.1 += self.velocity.1 * dt;
        if self.at.0 < b.left {
            self.at.0 = b.left;
            self.hit = self.hit.max(self.velocity.0.abs());
            self.velocity.0 = self.velocity.0.abs() * BOUNCE;
            self.spin = -self.spin * 0.7;
        } else if self.at.0 > b.right {
            self.at.0 = b.right;
            self.hit = self.hit.max(self.velocity.0.abs());
            self.velocity.0 = -self.velocity.0.abs() * BOUNCE;
            self.spin = -self.spin * 0.7;
        }
        if self.at.1 < b.top {
            self.at.1 = b.top;
            self.hit = self.hit.max(self.velocity.1.abs());
            self.velocity.1 = self.velocity.1.abs() * BOUNCE;
        }
        if self.at.1 >= b.floor {
            self.at.1 = b.floor;
            self.hit = self.hit.max(self.velocity.1.abs());
            if self.velocity.1 < SETTLE_SPEED {
                self.velocity.1 = 0.0;
                // Rolling to a stop.
                self.velocity.0 *= 0.8_f64.powf(dt * 30.0);
                if self.velocity.0.abs() < 12.0 {
                    self.velocity.0 = 0.0;
                }
            } else {
                self.velocity.1 = -self.velocity.1 * BOUNCE;
                self.velocity.0 *= 0.85;
                self.spin *= 0.6;
            }
        }
        self
    }
}

/// Where a pet sits in the window to begin with: its left edge, and its
/// height above the window's bottom, at the foot of the sessions rail.
const HOME: (f64, f64) = (136.0, 34.0);

/// The smallest and largest a pet can be made, as multiples of the usual.
pub const GROWN: (f64, f64) = (0.5, 4.0);

/// Spinning a held pet round winds it up: the turns it makes add to the
/// throw, and it tumbles as it flies.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Windup {
    last: Option<f64>,
    /// Radians turned while held, one way positive, the other negative.
    pub turned: f64,
}

/// The fastest a pet can be thrown, in logical pixels a second.
const TOP_SPEED: f64 = 9000.0;

impl Windup {
    /// The pointer (or the pet) moved this far.
    pub fn moved(&mut self, dx: f64, dy: f64) {
        if dx.hypot(dy) < 1.5 {
            return;
        }
        let heading = dy.atan2(dx);
        if let Some(last) = self.last {
            let turn = (heading - last + PI).rem_euclid(TAU) - PI;
            // Doubling back is a shake, not a swing round.
            if turn.abs() < PI * 0.75 {
                self.turned += turn;
            }
        }
        self.last = Some(heading);
    }

    /// Whole turns wound up.
    pub fn turns(&self) -> f64 {
        self.turned.abs() / TAU
    }

    /// The throw for a release at `velocity`: faster for every turn wound
    /// up, and spinning with them (and a little from the throw itself).
    pub fn throw(&self, velocity: (f64, f64)) -> ((f64, f64), f64) {
        let boost = (1.0 + 0.8 * self.turns()).min(5.0);
        let (mut vx, mut vy) = (velocity.0 * boost, velocity.1 * boost);
        let speed = vx.hypot(vy);
        if speed > TOP_SPEED {
            vx *= TOP_SPEED / speed;
            vy *= TOP_SPEED / speed;
        }
        let spin = self.turned.signum() * self.turns().min(6.0) * 9.0 + vx / 400.0;
        ((vx, vy), spin)
    }
}

/// The whole `/kill`: shot from the right, thrown `distance` to the left
/// in an arc, a bounce and a slide, lying still, then getting up, and
/// walking back if `walk_home`. `None` once it is over.
pub fn shot_at(ms: u64, distance: f64, walk_home: bool) -> Option<ShotFrame> {
    let flight = distance.max(0.0);
    let slide = flight * 0.15;
    let landed = -(flight + slide);
    let shake = if (HIT..HIT + SHAKE_FOR).contains(&ms) {
        let t = (ms - HIT) as f64;
        let strength = 12.0 * (1.0 - t / SHAKE_FOR as f64);
        (
            strength * (t * 0.26).sin(),
            strength * 0.6 * (t * 0.33).cos(),
        )
    } else {
        (0.0, 0.0)
    };
    let shot = |thrown| {
        Some(ShotFrame {
            anim: Anim::Shot,
            column: Anim::Shot.column(ms, false),
            thrown,
            shake,
        })
    };
    if ms < HIT {
        return shot((0.0, 0.0));
    }
    if ms < LAND {
        let p = (ms - HIT) as f64 / (LAND - HIT) as f64;
        let x = -flight * (1.0 - (1.0 - p) * (1.0 - p));
        let y = -flight * 0.45 * 4.0 * p * (1.0 - p);
        return shot((x, y));
    }
    if ms < SETTLE {
        let p = (ms - LAND) as f64 / (SETTLE - LAND) as f64;
        let x = -flight - slide * p;
        let y = -flight * 0.1 * 4.0 * p * (1.0 - p);
        return shot((x, y));
    }
    if ms < LIE_STILL {
        return shot((landed, 0.0));
    }
    let since = ms - LIE_STILL;
    if since < GET_UP {
        return Some(ShotFrame {
            anim: Anim::Jump,
            column: Anim::Jump.column(since, false),
            thrown: (landed, 0.0),
            shake: (0.0, 0.0),
        });
    }
    let walking = (since - GET_UP) as f64 * WALK;
    if walk_home && walking < -landed {
        return Some(ShotFrame {
            anim: Anim::RunRight,
            column: Anim::RunRight.column(since - GET_UP, true),
            thrown: (landed + walking, 0.0),
            shake: (0.0, 0.0),
        });
    }
    None
}

/// What a pet says.
#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub id: u64,
    /// The session it is about, to open on a click.
    pub session: Option<u64>,
    pub text: String,
    /// Shown until, unless dismissed first.
    pub until: Instant,
}

/// How long an update stays up.
const UPDATE_FOR: Duration = Duration::from_secs(12);
/// How long a pet looks pleased after its session finishes.
const REVIEW_FOR: Duration = Duration::from_secs(8);
/// How much of a reply a pet repeats.
const GIST: usize = 140;

/// The pet popped out of the window: none, asked for but not made yet, or
/// open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PetWindow {
    Closed,
    Pending,
    Open(floem::window::WindowId),
}

/// What a pet is doing now: the moving parts the views draw from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub anim: Anim,
    pub column: u32,
    pub thrown: (f64, f64),
    pub shake: (f64, f64),
    /// Turned round this far, in radians, spinning or tumbling.
    pub angle: f64,
}

const STILL: Pose = Pose {
    anim: Anim::Idle,
    column: 0,
    thrown: (0.0, 0.0),
    shake: (0.0, 0.0),
    angle: 0.0,
};

/// The first line of a reply, without its markdown, cut short.
pub fn gist(reply: &str) -> Option<String> {
    let line = reply
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches(['#', '>', '-', '*', ' '])
                .trim()
        })
        .find(|line| !line.is_empty() && !line.starts_with("```"))?;
    let line = line.replace("**", "").replace('`', "");
    Some(if line.chars().count() > GIST {
        let cut: String = line.chars().take(GIST - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        line
    })
}

/// One session's pet. It lives in the session's view, or is popped out of
/// the window, where it also brings news and questions from the others.
pub struct Pet {
    pub key: u64,
    /// Its signals and memos, disposed of when its session closes.
    scope: Scope,
    sessions: Rc<Sessions>,
    clock: RwSignal<u64>,
    started: Instant,
    out: RwSignal<Option<u64>>,
    docked_scale: RwSignal<f64>,
    popped_scale: RwSignal<f64>,
    /// A one-off animation and when it began, after which the mood resumes.
    once: RwSignal<Option<(Anim, u64)>>,
    /// When the current `/kill` began.
    shot: RwSignal<Option<u64>>,
    /// In the air after a fling or a hop.
    flight: RwSignal<Option<Flight>>,
    /// Held by the pointer, this far from its place.
    pub held: RwSignal<Option<(f64, f64)>>,
    /// Held and swung round: how far round it has turned.
    pub swing: RwSignal<Option<f64>>,
    /// When it last complained, so it does not say "Ow" every bounce.
    complained: Cell<u64>,
    /// Its chat box is open.
    pub chat_open: RwSignal<bool>,
    /// Who its chat box talks to: `None` asks its session on the side
    /// (`/btw`), or a session it sends to as a prompt.
    pub target: RwSignal<Option<u64>>,
    /// Waiting on an answer to a side question.
    pub thinking: RwSignal<bool>,
    /// How far a `/kill` throws, as the view with room for it decides.
    pub throw_distance: Cell<f64>,
    /// Being dragged, moving right (`true`) or left.
    pub moving: RwSignal<Option<bool>>,
    /// The direction to look in, when the pointer is near.
    pub look: RwSignal<Option<u8>>,
    updates: RwSignal<Vec<Update>>,
    next_update: Cell<u64>,
    /// When its session last finished a run, while that still shows.
    finished: RwSignal<Option<Instant>>,
    /// The sprite as drawn in the window, to look from.
    pub sprite_view: Cell<Option<floem::ViewId>>,
    pose: OnceCell<Memo<Pose>>,
    mood: OnceCell<Memo<Anim>>,
    /// Waiting questions as (session, entry) keys, its own session's first.
    asking: OnceCell<Memo<Vec<(u64, u64)>>>,
}

impl Pet {
    fn new(pets: &Pets, key: u64) -> Rc<Self> {
        let scope = pets.scope.create_child();
        scope.enter(|| {
            let pet = Rc::new(Pet {
                key,
                scope,
                sessions: pets.sessions.clone(),
                clock: pets.clock,
                started: pets.started,
                out: pets.out,
                docked_scale: pets.docked_scale,
                popped_scale: pets.popped_scale,
                once: RwSignal::new(None),
                shot: RwSignal::new(None),
                flight: RwSignal::new(None),
                held: RwSignal::new(None),
                swing: RwSignal::new(None),
                complained: Cell::new(0),
                chat_open: RwSignal::new(false),
                target: RwSignal::new(None),
                thinking: RwSignal::new(false),
                throw_distance: Cell::new(240.0),
                moving: RwSignal::new(None),
                look: RwSignal::new(None),
                updates: RwSignal::new(Vec::new()),
                next_update: Cell::new(0),
                finished: RwSignal::new(None),
                sprite_view: Cell::new(None),
                pose: OnceCell::new(),
                mood: OnceCell::new(),
                asking: OnceCell::new(),
            });
            // Questions and mood are worked out when sessions change, not on
            // every tick of the clock.
            let this = Rc::downgrade(&pet);
            let asking = Memo::new(move |_| {
                this.upgrade()
                    .map(|pet| pet.find_questions())
                    .unwrap_or_default()
            });
            let _ = pet.asking.set(asking);
            let this = Rc::downgrade(&pet);
            let mood =
                Memo::new(move |_| this.upgrade().map_or(Anim::Idle, |pet| pet.compute_mood()));
            let _ = pet.mood.set(mood);
            let this = Rc::downgrade(&pet);
            let pose = Memo::new(move |_| this.upgrade().map_or(STILL, |pet| pet.compute_pose()));
            let _ = pet.pose.set(pose);
            pet.greet();
            pet
        })
    }

    /// The frame to draw, where the pet has been thrown to, and the shake.
    pub fn pose(&self) -> Memo<Pose> {
        *self.pose.get().expect("made in new")
    }

    /// Popped out of the window.
    pub fn is_out(&self) -> bool {
        self.out.get() == Some(self.key)
    }

    fn now(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn compute_pose(&self) -> Pose {
        let ms = self.clock.get();
        let still = |anim: Anim, column| Pose {
            anim,
            column,
            ..STILL
        };
        if let Some(began) = self.shot.get()
            && let Some(frame) = shot_at(
                ms.saturating_sub(began),
                self.throw_distance.get(),
                !self.is_out(),
            )
        {
            return Pose {
                anim: frame.anim,
                column: frame.column,
                thrown: frame.thrown,
                shake: frame.shake,
                angle: 0.0,
            };
        }
        if let Some(angle) = self.swing.get() {
            // Being whirled round: legs up, turning with it.
            return Pose {
                angle,
                ..still(Anim::Jump, 2)
            };
        }
        if let Some(at) = self.held.get() {
            // Dangling: legs up, as in the air.
            return Pose {
                thrown: at,
                ..still(Anim::Jump, 2)
            };
        }
        if let Some(flight) = self.flight.get() {
            let (anim, column) = if !flight.on_floor() {
                (Anim::Jump, 2)
            } else if flight.velocity.0.abs() >= 1.0 {
                (Anim::Jump, 4)
            } else if flight.walk_home && flight.at.0.abs() >= 1.0 {
                let anim = if flight.at.0 < 0.0 {
                    Anim::RunRight
                } else {
                    Anim::RunLeft
                };
                (anim, anim.column(ms, true))
            } else {
                (Anim::Jump, 4)
            };
            return Pose {
                thrown: flight.at,
                angle: flight.angle,
                ..still(anim, column)
            };
        }
        if let Some(right) = self.moving.get() {
            let anim = if right { Anim::RunRight } else { Anim::RunLeft };
            return still(anim, anim.column(ms, true));
        }
        if let Some((anim, began)) = self.once.get() {
            let since = ms.saturating_sub(began);
            if since < anim.length() {
                return still(anim, anim.column(since, false));
            }
        }
        let mood = self.mood.get().map_or(Anim::Idle, |mood| mood.get());
        if mood == Anim::Idle
            && let Some(direction) = self.look.get()
        {
            let look = Anim::Look(direction);
            return still(look, look.column(0, false));
        }
        still(mood, mood.column(ms, true))
    }

    /// What the pet does when nothing in particular is happening: how its
    /// session is doing, then any question waiting for you.
    fn compute_mood(&self) -> Anim {
        if self.thinking.get() {
            return Anim::Working;
        }
        match self.sessions.find_tracked(self.key).map(|s| s.status()) {
            Some(Status::NeedsInput) => return Anim::Waiting,
            Some(Status::Working) => return Anim::Working,
            _ => {}
        }
        if self.asking.get().is_some_and(|a| a.with(|a| !a.is_empty())) {
            return Anim::Waiting;
        }
        if self.finished.with(Option::is_some) {
            Anim::Review
        } else {
            Anim::Idle
        }
    }

    /// Waiting questions as keys: its own session's, and popped out, the
    /// other sessions' after them.
    fn find_questions(&self) -> Vec<(u64, u64)> {
        let out = self.is_out();
        let mut found: Vec<(u64, u64)> = self.sessions.open().with(|open| {
            open.iter()
                .filter(|session| out || session.key == self.key)
                .flat_map(|session| {
                    session.entries().with(|entries| {
                        entries
                            .iter()
                            .filter(|e| e.kind == EntryKind::Ask && e.answered.get().is_none())
                            .map(|e| (session.key, e.id))
                            .collect::<Vec<_>>()
                    })
                })
                .collect()
        });
        found.sort_by_key(|(session, _)| *session != self.key);
        found
    }

    /// The questions it brings, its own session's first. Tracks only the
    /// questions, not every change to every timeline.
    pub fn questions(&self) -> Vec<(Rc<Session>, Rc<Entry>)> {
        let keys = self.asking.get().map(|a| a.get()).unwrap_or_default();
        keys.into_iter()
            .filter_map(|(key, id)| {
                let session = self.sessions.find(key)?;
                let entry = session
                    .entries()
                    .with_untracked(|e| e.iter().find(|e| e.id == id).cloned())?;
                Some((session, entry))
            })
            .collect()
    }

    pub fn updates(&self) -> RwSignal<Vec<Update>> {
        self.updates
    }

    /// Something to say, about a session or nothing in particular.
    pub fn tell(&self, session: Option<u64>, text: String) {
        let id = self.next_update.get() + 1;
        self.next_update.set(id);
        self.updates.update(|updates| {
            updates.push(Update {
                id,
                session,
                text,
                until: Instant::now() + UPDATE_FOR,
            });
            // The latest few.
            let excess = updates.len().saturating_sub(4);
            updates.drain(..excess);
        });
    }

    pub fn dismiss(&self, id: u64) {
        self.updates
            .update(|updates| updates.retain(|u| u.id != id));
    }

    /// Its session finished a run: pleased for a while.
    fn finished_run(&self) {
        self.finished.set(Some(Instant::now()));
    }

    /// Clears what has had its time: old news, the pleased look, a /kill
    /// that is over.
    fn expire(&self) {
        let now = Instant::now();
        if self
            .updates
            .with_untracked(|updates| updates.iter().any(|u| u.until <= now))
        {
            self.updates
                .update(|updates| updates.retain(|u| u.until > now));
        }
        if self
            .finished
            .with_untracked(|f| f.is_some_and(|at| at.elapsed() >= REVIEW_FOR))
        {
            self.finished.set(None);
        }
        let over = self.shot.get_untracked().is_some_and(|began| {
            let walk_home = self.out.get_untracked() != Some(self.key);
            shot_at(
                self.now().saturating_sub(began),
                self.throw_distance.get(),
                walk_home,
            )
            .is_none()
        });
        if over {
            self.shot.set(None);
        }
    }

    /// The shake of a /kill under way.
    fn shake(&self) -> Option<(f64, f64)> {
        let began = self.shot.get_untracked()?;
        let walk_home = self.out.get_untracked() != Some(self.key);
        shot_at(
            self.now().saturating_sub(began),
            self.throw_distance.get(),
            walk_home,
        )
        .map(|frame| frame.shake)
    }

    /// Plays an animation once, then goes back to the mood.
    pub fn play(&self, anim: Anim) {
        if self.shot.get_untracked().is_none() {
            self.once.set(Some((anim, self.now())));
        }
    }

    pub fn greet(&self) {
        self.play(Anim::Wave);
    }

    /// Clicked: a jump where it stands, and the pleased look is
    /// acknowledged.
    pub fn poke(&self) {
        if self.finished.with_untracked(Option::is_some) {
            self.finished.set(None);
        }
        if self.shot.get_untracked().is_none() && self.flight.get_untracked().is_none() {
            self.launch((0.0, 0.0), (0.0, -720.0), 0.0, Bounds::HOP, false);
        }
    }

    /// Let go of at `at`, moving at `velocity`: it flies, bounces off what
    /// is in the way, and lands, walking home after if `walk_home`.
    pub fn launch(
        &self,
        at: (f64, f64),
        velocity: (f64, f64),
        spin: f64,
        bounds: Bounds,
        walk_home: bool,
    ) {
        if self.shot.get_untracked().is_some() {
            return;
        }
        // Thrown spinning, it carries on from where the swing left it.
        let angle = self.swing.get_untracked().unwrap_or(0.0);
        self.held.set(None);
        self.swing.set(None);
        self.once.set(None);
        self.flight.set(Some(Flight {
            id: self.now(),
            at,
            velocity,
            bounds,
            walk_home,
            hit: 0.0,
            angle,
            spin,
        }));
    }

    /// Its size, as a multiple of the usual.
    pub fn grown(&self) -> f64 {
        if self.out.get() == Some(self.key) {
            self.popped_scale.get()
        } else {
            self.docked_scale.get()
        }
    }

    /// Stops whatever it is in the middle of.
    fn settle(&self) {
        if self.busy() {
            self.shot.set(None);
            self.flight.set(None);
            self.swing.set(None);
            self.moving.set(None);
        }
    }

    /// Whether it is on the move: flying, thrown, swung or dragged, when
    /// the clock runs faster.
    fn busy(&self) -> bool {
        self.flight.with_untracked(Option::is_some)
            || self.shot.with_untracked(Option::is_some)
            || self.swing.with_untracked(Option::is_some)
            || self.moving.with_untracked(Option::is_some)
    }

    /// In the air, rolling or walking home.
    pub fn flight(&self) -> RwSignal<Option<Flight>> {
        self.flight
    }

    /// Moves a flight on, and complains about hard knocks.
    fn fly(&self, dt: f64) {
        let Some(flight) = self.flight.get_untracked() else {
            return;
        };
        // Ended a tick after it comes to rest, so where it lands is drawn.
        if flight.done() {
            self.flight.set(None);
            return;
        }
        let next = flight.step(dt);
        if next.hit > 1400.0 && self.now().saturating_sub(self.complained.get()) > 900 {
            self.complained.set(self.now());
            const OUCH: [&str; 6] = ["Ow!", "Hey!", "Easy!", "Why?!", "Rude.", "Ouch."];
            let pick = (self.now() / 7) as usize % OUCH.len();
            self.quip(OUCH[pick]);
        }
        self.flight.set(Some(next));
    }

    /// Says something that stays until dismissed (or for a long while):
    /// an answer to read.
    pub fn answer(&self, session: Option<u64>, text: String) {
        self.tell(session, text);
        let id = self.next_update.get();
        self.updates.update(|updates| {
            if let Some(update) = updates.iter_mut().find(|u| u.id == id) {
                update.until = Instant::now() + Duration::from_secs(600);
            }
        });
    }

    /// A short remark, gone in a moment.
    pub fn quip(&self, text: &str) {
        self.tell(None, text.to_string());
        let id = self.next_update.get();
        self.updates.update(|updates| {
            if let Some(update) = updates.iter_mut().find(|u| u.id == id) {
                update.until = Instant::now() + Duration::from_millis(1800);
            }
        });
    }

    /// Shot from the right and thrown. A kill while one is under way
    /// starts over.
    pub fn kill(&self) {
        self.once.set(None);
        self.moving.set(None);
        self.held.set(None);
        self.swing.set(None);
        self.flight.set(None);
        self.shot.set(Some(self.now()));
    }

    /// When the /kill under way began.
    pub fn shot(&self) -> RwSignal<Option<u64>> {
        self.shot
    }

    /// The pointer moved in the window: the pet looks at it when near.
    fn pointer_at(&self, at: floem::kurbo::Point) {
        let look = self
            .sprite_view
            .get()
            .filter(|view| view.is_valid())
            .map(|view| view.get_visual_rect())
            .filter(|rect| rect.area() > 0.0)
            .and_then(|rect| {
                // The face sits a little below the cell's middle.
                let face = (rect.center().x, rect.y0 + rect.height() * 0.62);
                look_from(face, (at.x, at.y))
            });
        if self.look.get_untracked() != look {
            self.look.set(look);
        }
    }

    /// Looks the way of a point on the screen, popped out.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn look_at(&self, face: (f64, f64), at: (f64, f64)) {
        let look = look_from(face, at);
        if self.look.get_untracked() != look {
            self.look.set(look);
        }
    }
}

/// Where to look for a point: towards it when near, ahead when far or on top.
fn look_from(face: (f64, f64), at: (f64, f64)) -> Option<u8> {
    let distance = (at.0 - face.0).hypot(at.1 - face.1);
    (distance > 28.0 && distance < 420.0).then(|| direction(face, at))
}

/// Whether a session's last prompt got a reply from Pi.
fn replied(session: &Session) -> bool {
    session.entries().with_untracked(|entries| {
        let last = |kind| entries.iter().rposition(|e| e.kind == kind);
        match (last(EntryKind::Assistant), last(EntryKind::User)) {
            (Some(reply), Some(prompt)) => reply > prompt,
            (Some(_), None) => true,
            _ => false,
        }
    })
}

/// A pet thrown out of the window: from where on the screen (logical
/// pixels), how fast, and spinning how fast.
pub type Launch = (floem::kurbo::Point, (f64, f64), f64);

/// The pets of a window's sessions, their shared clock, and which one, if
/// any, is popped out.
pub struct Pets {
    sessions: Rc<Sessions>,
    scope: Scope,
    started: Instant,
    /// Milliseconds since `started`, ticking while pets are shown.
    clock: RwSignal<u64>,
    ticking: Cell<bool>,
    pets: RefCell<HashMap<u64, Rc<Pet>>>,
    /// How big pets are drawn, in the window and out of it, as multiples of
    /// the usual; dragging a pet's corner changes it.
    pub docked_scale: RwSignal<f64>,
    pub popped_scale: RwSignal<f64>,
    /// The session whose pet is popped out of the window.
    pub out: RwSignal<Option<u64>>,
    pub hidden: RwSignal<bool>,
    /// The popped-out pet's window.
    pub window: Cell<PetWindow>,
    /// Pop out the front session's pet once there is one, as saved.
    pop_out_when_ready: Cell<bool>,
    statuses: RefCell<HashMap<u64, (Status, Instant)>>,
    /// How much the window shakes while a /kill lands.
    shake: RwSignal<(f64, f64)>,
    /// In the window: the pet's left edge, and its height above the
    /// window's bottom, in logical pixels.
    pub dock: RwSignal<(f64, f64)>,
    /// Thrown out of the window: from where on the screen (logical pixels),
    /// and how fast.
    launch_out: Cell<Option<Launch>>,
    /// The popped-out pet's speech window, to move with it (Windows).
    pub speech_hwnd: Cell<Option<isize>>,
}

impl Pets {
    pub fn new(scope: Scope, sessions: Rc<Sessions>) -> Rc<Self> {
        let pets = scope.enter(|| {
            Rc::new(Pets {
                sessions,
                scope,
                started: Instant::now(),
                clock: RwSignal::new(0),
                ticking: Cell::new(false),
                pets: RefCell::new(HashMap::new()),
                docked_scale: RwSignal::new(1.0),
                popped_scale: RwSignal::new(1.0),
                out: RwSignal::new(None),
                hidden: RwSignal::new(false),
                window: Cell::new(PetWindow::Closed),
                pop_out_when_ready: Cell::new(false),
                statuses: RefCell::new(HashMap::new()),
                shake: RwSignal::new((0.0, 0.0)),
                dock: RwSignal::new(HOME),
                launch_out: Cell::new(None),
                speech_hwnd: Cell::new(None),
            })
        });
        pets.watch();
        pets
    }

    /// A session's pet, made the first time it is wanted.
    pub fn pet(&self, key: u64) -> Rc<Pet> {
        if let Some(pet) = self.pets.borrow().get(&key) {
            return pet.clone();
        }
        let pet = Pet::new(self, key);
        self.pets.borrow_mut().insert(key, pet.clone());
        pet
    }

    pub fn shake(&self) -> ReadSignal<(f64, f64)> {
        self.shake.read_only()
    }

    /// Pops a session's pet out of the window, bringing back any other.
    pub fn pop_out(&self, key: u64) {
        self.hidden.set(false);
        if self.out.get_untracked() != Some(key) {
            self.out.set(Some(key));
        }
        self.pet(key).greet();
    }

    /// Thrown out of the window: it flies on from that point on the screen.
    pub fn fling_out(&self, key: u64, at: floem::kurbo::Point, velocity: (f64, f64), spin: f64) {
        self.launch_out.set(Some((at, velocity, spin)));
        self.hidden.set(false);
        self.out.set(Some(key));
    }

    /// The throw a pet popped out with, if it was thrown out.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn take_launch(&self) -> Option<Launch> {
        self.launch_out.take()
    }

    pub fn pop_in(&self) {
        if let Some(key) = self.out.get_untracked() {
            self.out.set(None);
            self.pet(key).greet();
        }
    }

    /// Pops the front session's pet out, or the popped-out one back in.
    pub fn toggle_out(&self) {
        match (
            self.out.get_untracked(),
            self.sessions.active_key().get_untracked(),
        ) {
            (Some(_), _) => self.pop_in(),
            (None, Some(front)) => self.pop_out(front),
            (None, None) => self.pop_out_when_ready.set(true),
        }
    }

    /// Brings a lost pet back: in the window, shown, the usual size, at the
    /// foot of the sessions rail.
    pub fn find(&self) {
        self.pop_in();
        self.hidden.set(false);
        self.docked_scale.set(1.0);
        self.popped_scale.set(1.0);
        self.dock.set(HOME);
        if let Some(front) = self.sessions.active_key().get_untracked() {
            let pet = self.pet(front);
            pet.flight().set(None);
            pet.greet();
        }
    }

    pub fn toggle_hidden(&self) {
        self.hidden.update(|h| *h = !*h);
    }

    /// As saved: popped out (the front session's pet, once there is one).
    pub fn restore(&self, popped: bool, hidden: bool) {
        self.hidden.set(hidden);
        self.pop_out_when_ready.set(popped);
    }

    /// Whether a pet is popped out, for saving.
    pub fn popped(&self) -> bool {
        self.out.get_untracked().is_some() || self.pop_out_when_ready.get()
    }

    /// Starts the clock, once the window the pets live in exists.
    pub fn start(self: &Rc<Self>) {
        if !self.ticking.replace(true) {
            Pets::tick(Rc::downgrade(self));
        }
    }

    /// Advances the clock while pets are shown: about thirty times a second
    /// at rest, over a hundred while one is on the move.
    fn tick(pets: Weak<Pets>) {
        let Some(this) = pets.upgrade() else { return };
        // Its window closed and its signals with it: the clock stops.
        let Some(hidden) = this.hidden.try_get_untracked() else {
            return;
        };
        if hidden {
            // Out of sight, nothing is under way: no shot, flight or swing
            // keeps the clock running fast.
            for pet in this.pets.borrow().values() {
                pet.settle();
            }
        } else {
            this.advance();
        }
        let busy = this.pets.borrow().values().any(|pet| pet.busy());
        drop(this);
        let every = if busy { 8 } else { 33 };
        exec_after(Duration::from_millis(every), move |_| Pets::tick(pets));
    }

    /// Moves the clock on now, as while a popped-out pet is dragged and
    /// the timers wait.
    pub fn advance(&self) {
        let pets: Vec<Rc<Pet>> = self.pets.borrow().values().cloned().collect();
        // A finished /kill ends before the clock moves on, so a pet is never
        // drawn back where it was shot from for a frame.
        let now = self.started.elapsed().as_millis() as u64;
        let dt = now.saturating_sub(self.clock.get_untracked()) as f64 / 1000.0;
        for pet in &pets {
            pet.expire();
            pet.fly(dt);
        }
        self.clock.set(now);
        let shake = pets
            .iter()
            .find_map(|pet| pet.shake())
            .unwrap_or((0.0, 0.0));
        if self.shake.get_untracked() != shake {
            self.shake.set(shake);
        }
    }

    /// The pointer moved in the window: docked pets near it look at it.
    pub fn pointer_at(&self, at: floem::kurbo::Point) {
        let out = self.out.get_untracked();
        for pet in self.pets.borrow().values() {
            if Some(pet.key) != out {
                pet.pointer_at(at);
            }
        }
    }

    /// Notices sessions finishing and stopping, tells their pets (and the
    /// popped-out one, about others), and passes Pi extensions' cues on.
    fn watch(self: &Rc<Self>) {
        let pets = Rc::downgrade(self);
        self.scope.enter(|| {
            Effect::new(move |_| {
                let Some(pets) = pets.upgrade() else { return };
                let open = pets.sessions.open().get();
                if pets.pop_out_when_ready.get()
                    && let Some(front) = pets.sessions.active_key().get()
                {
                    pets.pop_out_when_ready.set(false);
                    pets.pop_out(front);
                }
                let mut previous = pets.statuses.borrow_mut();
                let mut news = Vec::new();
                for session in &open {
                    let now = session.status();
                    let before = match previous.get(&session.key) {
                        Some(&(status, since)) if status == now => Some((status, since)),
                        _ => previous.insert(session.key, (now, Instant::now())),
                    };
                    match (before, now) {
                        // Stopped by you, or by Pi exiting, is not done.
                        // A reply is news; a run stopped (by you, or Pi exiting)
                        // or with none (an extension's command) is not.
                        (Some((Status::Working, _)), Status::Idle)
                            if !session.was_stopped() && replied(session) =>
                        {
                            news.push((session.clone(), true));
                        }
                        (Some((before, _)), Status::Stopped) if before != Status::Stopped => {
                            news.push((session.clone(), false));
                        }
                        _ => {}
                    }
                }
                previous.retain(|key, _| open.iter().any(|s| s.key == *key));
                drop(previous);
                // Pets of closed sessions go with them; all in one update.
                Effect::batch(|| {
                    let closed: Vec<u64> = pets
                        .pets
                        .borrow()
                        .keys()
                        .copied()
                        .filter(|key| !open.iter().any(|s| s.key == *key))
                        .collect();
                    for key in closed {
                        if let Some(pet) = pets.pets.borrow_mut().remove(&key) {
                            // Once its views have gone with it.
                            let scope = pet.scope;
                            exec_after(Duration::from_secs(2), move |_| scope.dispose());
                        }
                        if pets.out.get_untracked() == Some(key) {
                            pets.out.set(None);
                        }
                    }
                    for (session, done) in news {
                        pets.report(&session, done);
                    }
                });
            });
            let pets = Rc::downgrade(self);
            let cue = self.sessions.cue();
            Effect::new(move |_| {
                let Some(cue) = cue.get() else { return };
                let Some(pets) = pets.upgrade() else { return };
                let pet = pets.pet(cue.session);
                match cue.action.as_str() {
                    "kill" => pet.kill(),
                    "wave" => pet.play(Anim::Wave),
                    "jump" => pet.play(Anim::Jump),
                    other => {
                        if let Some(text) = other.strip_prefix("say:")
                            && !text.trim().is_empty()
                        {
                            pet.tell(Some(cue.session), text.trim().to_string());
                        }
                    }
                }
            });
        });
    }

    /// A session finished a run (`done`) or its Pi stopped: its pet says
    /// so, with the gist of the reply; the popped-out pet, if another's,
    /// passes it on.
    fn report(&self, session: &Session, done: bool) {
        let reply = session.entries().with_untracked(|entries| {
            entries
                .iter()
                .rev()
                .find(|e| e.kind == EntryKind::Assistant)
                .and_then(|e| e.body.with_untracked(|body| gist(body)))
        });
        let said = match (done, reply) {
            (true, Some(gist)) => gist,
            (true, None) => "Done.".to_string(),
            (false, _) => "Pi stopped.".to_string(),
        };
        let own = self.pet(session.key);
        if done {
            own.finished_run();
        }
        own.tell(Some(session.key), said.clone());
        if let Some(out) = self.out.get_untracked()
            && out != session.key
        {
            let title = session.title().get_untracked();
            self.pet(out)
                .tell(Some(session.key), format!("{title}: {said}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_follow_the_artworks_timing() {
        // Idle: 280, 110, 110, 140, 140, 320 ms.
        assert_eq!(Anim::Idle.column(0, true), 0);
        assert_eq!(Anim::Idle.column(279, true), 0);
        assert_eq!(Anim::Idle.column(280, true), 1);
        assert_eq!(Anim::Idle.column(1099, true), 5);
        assert_eq!(Anim::Idle.column(1100, true), 0, "loops");
        assert_eq!(Anim::Wave.column(10_000, false), 3, "holds its last frame");
        assert_eq!(Anim::Look(3).row(), 9);
        assert_eq!(Anim::Look(12).row(), 10);
        assert_eq!(Anim::Look(12).column(0, false), 4);
        assert_eq!(Anim::Shot.row(), 5);
    }

    #[test]
    fn looking_points_at_the_pointer() {
        let at = (100.0, 100.0);
        assert_eq!(direction(at, (100.0, 0.0)), 0, "up");
        assert_eq!(direction(at, (200.0, 100.0)), 4, "right");
        assert_eq!(direction(at, (100.0, 200.0)), 8, "down");
        assert_eq!(direction(at, (0.0, 100.0)), 12, "left");
        assert_eq!(direction(at, (200.0, 0.0)), 2, "up and right");
    }

    #[test]
    fn a_pet_repeats_the_gist_of_a_reply() {
        assert_eq!(
            gist("\n## **Done**: tests pass\nmore"),
            Some("Done: tests pass".into())
        );
        assert_eq!(gist("```rust\ncode\n```"), Some("code".into()));
        assert_eq!(gist("  \n"), None);
        let long = "word ".repeat(60);
        let cut = gist(&long).unwrap();
        assert!(cut.ends_with('…') && cut.chars().count() <= GIST);
    }

    #[test]
    fn a_flung_pet_bounces_off_walls_and_lands() {
        let bounds = Bounds {
            left: -300.0,
            right: 50.0,
            top: -400.0,
            floor: 0.0,
        };
        let mut flight = Flight {
            id: 1,
            at: (0.0, -100.0),
            velocity: (-2500.0, -300.0),
            bounds,
            walk_home: false,
            hit: 0.0,
            angle: 0.0,
            spin: 14.0,
        };
        let mut hit_wall = false;
        for _ in 0..600 {
            flight = flight.step(1.0 / 60.0);
            assert!(flight.at.0 >= -300.0 && flight.at.0 <= 50.0 && flight.at.1 <= 0.0);
            hit_wall |= flight.hit > 1000.0 && flight.at.0 == -300.0;
        }
        assert!(hit_wall, "it hits the left wall hard");
        assert!(flight.done(), "and comes to rest, upright: {flight:?}");
        // Walking home from where it landed.
        let mut home = Flight {
            walk_home: true,
            ..flight
        };
        for _ in 0..600 {
            home = home.step(1.0 / 60.0);
        }
        assert!(home.done() && home.at == (0.0, 0.0));
        // A hop comes back down where it started.
        let mut hop = Flight {
            at: (0.0, 0.0),
            velocity: (0.0, -720.0),
            bounds: Bounds::HOP,
            spin: 0.0,
            ..flight
        };
        let mut highest: f64 = 0.0;
        for _ in 0..120 {
            hop = hop.step(1.0 / 60.0);
            highest = highest.min(hop.at.1);
        }
        assert!(highest < -80.0 && hop.done() && hop.at.1 == 0.0);
    }

    #[test]
    fn swinging_a_pet_round_winds_up_the_throw() {
        let mut windup = Windup::default();
        // Two turns round a circle, a step at a time.
        for step in 0..=64 {
            let a = f64::from(step) * TAU / 32.0;
            windup.moved(-a.sin() * 10.0, a.cos() * 10.0);
        }
        assert!((windup.turns() - 2.0).abs() < 0.1, "{}", windup.turns());
        let ((vx, vy), spin) = windup.throw((1000.0, 0.0));
        assert!(vx > 2500.0 && vy == 0.0, "faster for the turns: {vx}");
        assert!(spin > 15.0, "and spinning: {spin}");
        // Back and forth winds nothing up.
        let mut wobble = Windup::default();
        for step in 0..40 {
            let dx = if step % 2 == 0 { 10.0 } else { -10.0 };
            wobble.moved(dx, 0.0);
        }
        assert!(wobble.turns() < 0.1);
        // However hard, there is a limit.
        let mut wound = Windup {
            turned: 100.0,
            ..Windup::default()
        };
        wound.moved(1.0, 1.0);
        let ((vx, vy), _) = wound.throw((8000.0, 0.0));
        assert!(vx.hypot(vy) <= TOP_SPEED + 1.0);
    }

    #[test]
    fn a_kill_shakes_throws_left_lands_and_gets_up() {
        let start = shot_at(0, 200.0, true).unwrap();
        assert_eq!(
            (start.anim, start.column, start.thrown),
            (Anim::Shot, 0, (0.0, 0.0))
        );
        // The hit: the window shakes.
        let hit = shot_at(150, 200.0, true).unwrap();
        assert_eq!(hit.column, 1);
        assert!(hit.shake.0.abs() + hit.shake.1.abs() > 0.0);
        // In the air, up and to the left.
        let flying = shot_at(280, 200.0, true).unwrap();
        assert!(flying.thrown.0 < -100.0 && flying.thrown.1 < -50.0);
        // Down, slid a little further, still.
        let lying = shot_at(2000, 200.0, true).unwrap();
        assert_eq!((lying.column, lying.thrown), (7, (-230.0, 0.0)));
        assert_eq!(lying.shake, (0.0, 0.0));
        // Up again, then walking home, then over.
        assert_eq!(shot_at(3100, 200.0, true).unwrap().anim, Anim::Jump);
        let walking = shot_at(3000 + 700 + 100, 200.0, true).unwrap();
        assert_eq!(walking.anim, Anim::RunRight);
        assert!(walking.thrown.0 > -230.0);
        assert!(shot_at(3000 + 700 + 1000, 200.0, true).is_none());
        // Popped out, it stays where it was thrown.
        assert!(shot_at(3000 + 700, 200.0, false).is_none());
    }
}
