//! Glitch, the workbench's pet. One lives in each window and watches every
//! session in it: it shows how the front session is doing, says when others
//! finish or stop, and brings you their questions. It sits in the window or
//! pops out into a little window of its own. A `/kill` gets it shot and
//! thrown, which is the one animation kept for that.
use crate::{
    session::{Entry, EntryKind, Session, Status},
    sessions::Sessions,
};
use floem::{
    action::exec_after,
    reactive::{Effect, Memo, RwSignal, Scope, SignalGet, SignalUpdate, SignalWith},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
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

/// What the pet tells you about a session.
#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub id: u64,
    pub session: Option<u64>,
    pub text: String,
    /// Shown until, unless dismissed first.
    pub until: Instant,
}

/// How long an update stays up.
const UPDATE_FOR: Duration = Duration::from_secs(12);
/// How long the pet looks pleased after the front session finishes.
const REVIEW_FOR: Duration = Duration::from_secs(8);
/// A run this short (a slash command an extension handled, say) is not news.
const NEWSWORTHY_RUN: Duration = Duration::from_secs(2);

/// What the pet is doing now: the moving parts the views draw from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub anim: Anim,
    pub column: u32,
    pub thrown: (f64, f64),
    pub shake: (f64, f64),
}

pub struct Pet {
    sessions: Rc<Sessions>,
    started: Instant,
    /// Milliseconds since `started`, ticking while the pet is shown.
    clock: RwSignal<u64>,
    pub popped: RwSignal<bool>,
    pub hidden: RwSignal<bool>,
    /// A one-off animation and when it began, after which the mood resumes.
    once: RwSignal<Option<(Anim, u64)>>,
    /// When the current `/kill` began.
    shot: RwSignal<Option<u64>>,
    /// How far a `/kill` throws, as the view with room for it decides.
    pub throw_distance: Cell<f64>,
    /// Being dragged, moving right (`true`) or left.
    pub moving: RwSignal<Option<bool>>,
    /// Where the pointer is, as a direction to look in, when it is near.
    pub look: RwSignal<Option<u8>>,
    updates: RwSignal<Vec<Update>>,
    next_update: Cell<u64>,
    /// When the front session last finished a run.
    finished: RwSignal<Option<(u64, Instant)>>,
    /// Each session's status, and since when.
    statuses: RefCell<HashMap<u64, (Status, Instant)>>,
    /// The pet's own window while popped out.
    pub window: Cell<Option<floem::window::WindowId>>,
    /// Docked: the sprite's left edge, and its height above the window's
    /// bottom, in logical pixels.
    pub dock: RwSignal<(f64, f64)>,
    /// The sprite as drawn in the window, to look from.
    pub sprite_view: Cell<Option<floem::ViewId>>,
    pose: std::cell::OnceCell<Memo<Pose>>,
    ticking: Cell<bool>,
}

impl Pet {
    pub fn new(scope: Scope, sessions: Rc<Sessions>) -> Rc<Self> {
        scope.enter(|| {
            let clock = RwSignal::new(0u64);
            let once = RwSignal::new(None::<(Anim, u64)>);
            let shot = RwSignal::new(None::<u64>);
            let moving = RwSignal::new(None::<bool>);
            let look = RwSignal::new(None::<u8>);
            let finished = RwSignal::new(None);
            let popped = RwSignal::new(false);
            let pet = Rc::new(Pet {
                sessions: sessions.clone(),
                started: Instant::now(),
                clock,
                popped,
                hidden: RwSignal::new(false),
                once,
                shot,
                throw_distance: Cell::new(240.0),
                moving,
                look,
                updates: RwSignal::new(Vec::new()),
                next_update: Cell::new(0),
                finished,
                statuses: RefCell::new(HashMap::new()),
                window: Cell::new(None),
                dock: RwSignal::new((136.0, 34.0)),
                sprite_view: Cell::new(None),
                pose: std::cell::OnceCell::new(),
                ticking: Cell::new(false),
            });
            let this = Rc::downgrade(&pet);
            let pose = Memo::new(move |_| match this.upgrade() {
                Some(pet) => pet.compute_pose(),
                None => Pose {
                    anim: Anim::Idle,
                    column: 0,
                    thrown: (0.0, 0.0),
                    shake: (0.0, 0.0),
                },
            });
            let _ = pet.pose.set(pose);
            pet.watch();
            pet.greet();
            pet
        })
    }

    /// Starts the clock, once the window it lives in exists.
    pub fn start(self: &Rc<Self>) {
        if !self.ticking.replace(true) {
            Pet::tick(Rc::downgrade(self));
        }
    }

    /// Advances the clock while the pet is shown, about thirty times a second.
    fn tick(pet: Weak<Pet>) {
        let Some(this) = pet.upgrade() else { return };
        if !this.hidden.get_untracked() {
            // A finished /kill ends before the clock moves on, so the pet is
            // never drawn back where it was shot from for a frame.
            this.expire_updates();
            this.clock.set(this.started.elapsed().as_millis() as u64);
        }
        drop(this);
        exec_after(Duration::from_millis(33), move |_| Pet::tick(pet));
    }

    /// The frame to draw, where the pet has been thrown to, and the shake.
    pub fn pose(&self) -> Memo<Pose> {
        *self.pose.get().expect("made in new")
    }

    fn now(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Notices sessions finishing, stopping, and asking, and cues from Pi
    /// extensions.
    fn watch(self: &Rc<Self>) {
        let pet = Rc::downgrade(self);
        Effect::new(move |_| {
            let Some(pet) = pet.upgrade() else { return };
            let open = pet.sessions.open().get();
            let front = pet.sessions.active_key().get();
            let mut previous = pet.statuses.borrow_mut();
            let mut news = Vec::new();
            for session in &open {
                let now = session.status();
                let before = match previous.get(&session.key) {
                    Some(&(status, since)) if status == now => Some((status, since)),
                    _ => previous.insert(session.key, (now, Instant::now())),
                };
                let title = session.title().get_untracked();
                match (before, now) {
                    (Some((Status::Working, since)), Status::Idle)
                        if since.elapsed() >= NEWSWORTHY_RUN =>
                    {
                        if Some(session.key) == front {
                            pet.finished.set(Some((session.key, Instant::now())));
                        }
                        news.push((session.key, format!("Done: {title}")));
                    }
                    (Some((before, _)), Status::Stopped) if before != Status::Stopped => {
                        news.push((session.key, format!("Pi stopped: {title}")));
                    }
                    _ => {}
                }
            }
            previous.retain(|key, _| open.iter().any(|s| s.key == *key));
            drop(previous);
            for (key, text) in news {
                pet.tell(Some(key), text);
            }
        });
        let pet = Rc::downgrade(self);
        let cue = self.sessions.cue();
        Effect::new(move |_| {
            let Some(cue) = cue.get() else { return };
            let Some(pet) = pet.upgrade() else { return };
            match cue.action.as_str() {
                "kill" => pet.kill(),
                "wave" => pet.play(Anim::Wave),
                "jump" => pet.play(Anim::Jump),
                other => {
                    if let Some(text) = other.strip_prefix("say:") {
                        pet.tell(Some(cue.session), text.trim().to_string());
                    }
                }
            }
        });
    }

    fn compute_pose(&self) -> Pose {
        let ms = self.clock.get();
        let still = |anim: Anim, column| Pose {
            anim,
            column,
            thrown: (0.0, 0.0),
            shake: (0.0, 0.0),
        };
        if let Some(began) = self.shot.get() {
            let walk_home = !self.popped.get();
            if let Some(frame) = shot_at(
                ms.saturating_sub(began),
                self.throw_distance.get(),
                walk_home,
            ) {
                return Pose {
                    anim: frame.anim,
                    column: frame.column,
                    thrown: frame.thrown,
                    shake: frame.shake,
                };
            }
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
        let mood = self.mood();
        if mood == Anim::Idle
            && let Some(direction) = self.look.get()
        {
            return still(
                Anim::Look(direction),
                Anim::Look(direction).column(0, false),
            );
        }
        still(mood, mood.column(ms, true))
    }

    /// What the pet does when nothing in particular is happening: how the
    /// front session is doing comes first, then any question waiting.
    pub fn mood(&self) -> Anim {
        let front = self.sessions.active();
        match front.as_ref().map(|s| s.status()) {
            Some(Status::NeedsInput) => return Anim::Waiting,
            Some(Status::Working) => return Anim::Working,
            _ => {}
        }
        if !self.questions().is_empty() {
            return Anim::Waiting;
        }
        let pleased = self.finished.with(|f| {
            f.is_some_and(|(key, at)| {
                Some(key) == front.as_ref().map(|s| s.key) && at.elapsed() < REVIEW_FOR
            })
        });
        if pleased { Anim::Review } else { Anim::Idle }
    }

    /// Questions waiting in any session, the front session's first.
    pub fn questions(&self) -> Vec<(Rc<Session>, Rc<Entry>)> {
        let front = self.sessions.active_key().get();
        let mut found: Vec<(Rc<Session>, Rc<Entry>)> = self
            .sessions
            .open()
            .get()
            .into_iter()
            .flat_map(|session| {
                session
                    .entries()
                    .get()
                    .into_iter()
                    .filter(|e| e.kind == EntryKind::Ask && e.answered.get().is_none())
                    .map(move |e| (session.clone(), e))
                    .collect::<Vec<_>>()
            })
            .collect();
        found.sort_by_key(|(session, _)| Some(session.key) != front);
        found
    }

    pub fn updates(&self) -> RwSignal<Vec<Update>> {
        self.updates
    }

    /// Something to say about a session (or about nothing in particular).
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

    fn expire_updates(&self) {
        let now = Instant::now();
        if self
            .updates
            .with_untracked(|updates| updates.iter().any(|u| u.until <= now))
        {
            self.updates
                .update(|updates| updates.retain(|u| u.until > now));
        }
        if self.shot.get_untracked().is_some_and(|began| {
            let walk_home = !self.popped.get_untracked();
            shot_at(
                self.now().saturating_sub(began),
                self.throw_distance.get(),
                walk_home,
            )
            .is_none()
        }) {
            self.shot.set(None);
        }
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

    /// Clicked: a hop, and the pleased look is acknowledged.
    pub fn poke(&self) {
        if self.finished.with_untracked(Option::is_some) {
            self.finished.set(None);
        }
        self.play(Anim::Jump);
    }

    /// Shot from the right and thrown. A kill while one is under way
    /// starts over.
    pub fn kill(&self) {
        self.once.set(None);
        self.moving.set(None);
        self.shot.set(Some(self.now()));
    }

    /// Whether a `/kill` is under way.
    pub fn shot(&self) -> RwSignal<Option<u64>> {
        self.shot
    }

    /// The pointer moved in the window: the pet looks at it when it is
    /// near, and not while popped out, where the window cannot see it.
    pub fn pointer_at(&self, at: floem::kurbo::Point) {
        let look = (!self.popped.get_untracked())
            .then(|| self.sprite_view.get())
            .flatten()
            .filter(|view| view.is_valid())
            .map(|view| view.get_visual_rect())
            .filter(|rect| rect.area() > 0.0)
            .and_then(|rect| {
                // The face sits a little below the cell's middle.
                let face = (rect.center().x, rect.y0 + rect.height() * 0.62);
                let distance = (at.x - face.0).hypot(at.y - face.1);
                (distance > 28.0 && distance < 420.0).then(|| direction(face, (at.x, at.y)))
            });
        if self.look.get_untracked() != look {
            self.look.set(look);
        }
    }

    pub fn toggle_popped(&self) {
        self.popped.update(|p| *p = !*p);
        self.hidden.set(false);
        self.greet();
    }

    pub fn toggle_hidden(&self) {
        self.hidden.update(|h| *h = !*h);
        if !self.hidden.get_untracked() {
            self.greet();
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
