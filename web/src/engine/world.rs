//! Rambo as an entity on the engine (ENGINE-PLAN step 2): where he is, how he gets
//! where his brain wants him, and the brain events his own movement and timers raise.
//!
//! Pure: once a frame the page reads its anchors (where the nest is, where his spot
//! is), steps the world by the engine's fixed steps and writes one [`Draw`] to his
//! element. The brain is [`mascot::transition`], unchanged; this decides only how his
//! body follows it, so behaviours end on state, never on a guessed CSS duration.

use super::dance::Dance;
use super::fling::{self, Body, Walls};
use super::look::{self, Look};
use super::nest::Nest;
use super::sched::ENGINE_DT_S;
use crate::hop::Hop;
use crate::mascot::{self, Event, Pose, State};
use crate::tuning::{self, NOTICE_BEAT_MS, RIDE_OMEGA};

/// A point on the page in px, or a velocity in px per second.
pub(crate) type Vec2 = (f64, f64);

/// The engine's fixed step, in ms of world time.
const STEP_MS: f64 = ENGINE_DT_S * 1000.0;

/// How near his spot he is, in px, once he has caught up with it after a change of
/// state: from then on he follows it exactly.
const CAUGHT_UP_PX: f64 = 0.5;

/// The page's leaf as the page measured it this frame: the one the turn swings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Leaf {
    /// How far it is open (`flip.rs`): 0 lying flat, 1 stood on its edge.
    pub(crate) open: f64,
    /// Whether it is swinging shut. Only a closing leaf catches him, and only one that has
    /// come down from beyond what he can lift (`COVERED_LIFT_REACH`): the leaf falling back
    /// from his own push is not a new catch.
    pub(crate) closing: bool,
    /// The page's top left corner in window px, its size, and the viewer's distance.
    pub(crate) origin: Vec2,
    pub(crate) size: Vec2,
    pub(crate) distance: f64,
}

impl Leaf {
    /// Whether the leaf lies over the window point `at`.
    fn covers(&self, at: Vec2) -> bool {
        crate::flip::leaf_covers(
            self.open,
            self.size,
            self.distance,
            (at.0 - self.origin.0, at.1 - self.origin.1),
        )
    }

    /// Where the leaf's free edge is on the screen, in window x: where he heads to push
    /// its corner up from underneath.
    fn free_edge_x(&self) -> f64 {
        self.origin.0
            + crate::flip::edge_px(self.size.0, crate::flip::angle(self.open), self.distance)
    }

    /// Whether it is closing over `at` now, from beyond his reach.
    fn catches(&self, at: Vec2) -> bool {
        self.closing && self.open > tuning::COVERED_LIFT_REACH && self.covers(at)
    }
}

/// What the page measured this frame, in window px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Anchors {
    /// The middle of his nest.
    pub(crate) nest: Vec2,
    /// His spot: where whoever moves him wants him (above the finger while held, his
    /// nib's place while writing, where he fell).
    pub(crate) spot: Vec2,
    /// The reader asked for less motion: he is simply where he goes, with no hops.
    pub(crate) reduced_motion: bool,
    /// The page's leaf as the turner last showed it: none before any turn, and after one
    /// the leaf lying where it landed (flat, not closing, or turned away).
    pub(crate) leaf: Option<Leaf>,
    /// The box his middle stays in when thrown: the screen less half his size.
    pub(crate) walls: Walls,
}

/// Ink his own movement lays, for the page to put on the paper: a splat thrown off
/// the edge he smacked into, and a dot where each hop home after a daze lands.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Splash {
    /// Strokes fanned out from where he hit, each a few points.
    Splat(Vec<Vec<Vec2>>),
    /// Where his middle came down.
    Dot(Vec2),
}

/// How his body follows his brain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Drive {
    /// Exactly there, every step: his nib on a row, where he fell, the nest once he is
    /// settled in it.
    Follow(Vec2),
    /// Pulled there by a critically damped spring: carried by a finger, he rides just
    /// behind it and never overshoots.
    Ride(Vec2),
    /// Hops there (`crate::hop`), the far end read every step so a moving nest is
    /// still where he lands.
    HopTo(Vec2),
    /// Slides on the throw's speed (`engine::fling`) until he stops or splats.
    Throw,
}

/// How his body follows `state`, given what the page measured: he hops between spots
/// (`mascot::hops_there`, the rule the notice beat is guarded by), rides the finger
/// that holds him, and is exactly on whatever else he tracks.
pub(crate) fn behaviour(state: &State, settled_home: bool, here: Vec2, anchors: &Anchors) -> Drive {
    let target = match state {
        State::Home | State::Skittering => anchors.nest,
        _ => anchors.spot,
    };
    match state {
        _ if mascot::hops_there(state, settled_home) => Drive::HopTo(target),
        State::Dragging => Drive::Ride(target),
        // Under the leaf he works his way along to its free corner, level with where he
        // was caught (not with a finger that may still be moving), and pushes it up from
        // there.
        State::Covered => Drive::Ride(
            anchors
                .leaf
                .map_or(here, |leaf| (leaf.free_edge_x(), here.1)),
        ),
        State::Thrown => Drive::Throw,
        _ => Drive::Follow(target),
    }
}

/// A run of hops under way: planned from where he set off, timed in world ms from
/// `began`. A run that sets off on a change of state starts with his notice beat: he
/// stands, makes `beat`'s face and perks up for [`NOTICE_BEAT_MS`], then hops.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Run {
    hop: Hop,
    began: f64,
    beat: Option<Pose>,
    /// The hops of the run that have landed so far.
    landed: u32,
}

impl Run {
    fn beat_ms(&self) -> u32 {
        self.beat.map_or(0, |_| NOTICE_BEAT_MS)
    }
}

/// How he is drawn this frame: his middle raised by a hop's lift, his squash
/// (positive squashed, negative stretched), and his face while he is in a notice beat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Draw {
    pub(crate) pos: Vec2,
    pub(crate) squash: f64,
    pub(crate) notice: Option<Pose>,
}

impl Draw {
    fn at(pos: Vec2) -> Self {
        Draw {
            pos,
            squash: 0.0,
            notice: None,
        }
    }
}

/// Rambo: his body (the ground point under him, and its velocity), his brain, and how
/// far into his current state and run he is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rambo {
    pub(crate) brain: State,
    pub(crate) pos: Vec2,
    pub(crate) vel: Vec2,
    /// When his brain entered its state, in world ms: his timers count from here.
    since: f64,
    run: Option<Run>,
    /// The face the last change of state owes: a caller often changes his state first
    /// and sets his spot a moment later, so the beat waits for the first run that sets
    /// off. A change that ends in a step leaves it owed; the next change replaces it.
    owed: Option<Pose>,
    /// Whether he is still catching up with the first letter of a row he has flown to:
    /// that new spot is reached by the ride spring, never by a jump. Every other state
    /// he follows is exact (inking, his nib stays on the finger).
    settling: bool,
    /// Whether he has come to rest in his nest, so the nest moving is the page
    /// scrolling under him and not a reason to hop.
    settled_home: bool,
    draw: Draw,
    /// His pose when his state last changed, which he eases out of.
    look_from: Look,
    /// His lean into his move, eased after his speed every step (`look::eased_sway`).
    sway: f64,
    look: Look,
    /// Where a throw left him, which his splat and daze stay on: his spot is still
    /// where the finger let go.
    fell: Option<Vec2>,
    /// How long he has written, in ms, since he last tapped his nib dry.
    written_ms: f64,
    /// Tapping his dry nib on the rim, home: when the taps began, and how many are done.
    tapping: Option<(f64, u32)>,
}

/// What the paper he lies on does to him when it moves: he stays (he is not on it), rides
/// along by how far it moved, or is flung at its speed in px per ms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Swept {
    Stays,
    Carried(Vec2),
    Flung(Vec2),
}

/// The page under him moved `delta` px on screen in `dt_ms` (Interview 1 item 2). Lying
/// on it, he holds on by friction: up to [`tuning::SWEEP_GRIP_PX_MS`] he rides along,
/// keeping his place on the paper; faster he slips and is flung with the paper's
/// speed, and the throw does the rest (an edge he hits hard enough smacks him). With
/// reduced motion he only ever rides along.
pub(crate) fn swept(state: &State, delta: Vec2, dt_ms: f64, reduced_motion: bool) -> Swept {
    if !mascot::lies_on_page(state) {
        return Swept::Stays;
    }
    let vel = (delta.0 / dt_ms, delta.1 / dt_ms);
    let slips = dt_ms > 0.0 && vel.0.hypot(vel.1) > tuning::SWEEP_GRIP_PX_MS;
    match slips && !reduced_motion {
        true => Swept::Flung(vel),
        false => Swept::Carried(delta),
    }
}

/// Everything the engine moves. Today that is Rambo; the nest, ink and the turn join it
/// in later steps.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct World {
    pub(crate) rambo: Rambo,
    /// World time in ms: the sum of the fixed steps taken.
    now: f64,
    /// Ink laid since the page last took it.
    ink: Vec<Splash>,
    /// His nest, which sags and rocks as he lands in it and leaves it.
    nest: Nest,
    /// How loud the music is that he hears now, if any plays.
    heard: Option<f64>,
    /// His dance to it.
    dance: Dance,
}

impl World {
    /// The world at load: Rambo at rest in his nest at `home`.
    pub(crate) fn new(home: Vec2) -> Self {
        World {
            rambo: Rambo {
                brain: State::Home,
                pos: home,
                vel: (0.0, 0.0),
                // Long settled in his pose.
                since: -crate::tuning::WRITE_BLEND_MS,
                run: None,
                owed: None,
                settling: false,
                settled_home: true,
                draw: Draw::at(home),
                look_from: look::pose(&State::Home, 0.0, false),
                look: look::pose(&State::Home, 0.0, false),
                sway: 0.0,
                fell: None,
                written_ms: 0.0,
                tapping: None,
            },
            now: 0.0,
            ink: Vec::new(),
            nest: Nest::default(),
            heard: None,
            dance: Dance::default(),
        }
    }

    /// How he is drawn now.
    pub(crate) fn draw(&self) -> Draw {
        self.rambo.draw
    }

    /// How he is posed now (`engine::look`).
    pub(crate) fn look(&self) -> Look {
        // At home he sits in the nest and rocks with it.
        let rock = match self.rambo.brain {
            State::Home => self.nest.offset().1,
            _ => 0.0,
        };
        Look {
            lean_deg: self.rambo.look.lean_deg
                + self.rambo.sway
                + rock
                + self.tap_lean()
                + self.dance.lean_deg(),
            ..self.rambo.look
        }
    }

    /// How far he tips toward the rim while he taps his nib: eased in before the first
    /// tap and out after the last, so he never snaps.
    fn tap_lean(&self) -> f64 {
        let Some((began, _)) = self.rambo.tapping else {
            return 0.0;
        };
        let ms = self.now - began;
        let edge = (ms.min(tapping_ms() - ms) / tuning::DRY_NIB_PAUSE_MS).clamp(0.0, 1.0);
        tuning::DRY_NIB_LEAN_DEG * edge * edge * (3.0 - 2.0 * edge)
    }

    /// How the nest is drawn now: sagged by `.0` px and rocked by `.1` degrees.
    pub(crate) fn nest_offset(&self) -> Vec2 {
        self.nest.offset()
    }

    /// Let go mid-flick at `vel`, in px per ms: the throw he slides on once his brain
    /// is [`State::Thrown`].
    pub(crate) fn throw(&mut self, vel: Vec2) {
        self.rambo.vel = (vel.0 * 1000.0, vel.1 * 1000.0);
    }

    /// The ink laid since the last call, for the page to put down.
    pub(crate) fn take_ink(&mut self) -> Vec<Splash> {
        std::mem::take(&mut self.ink)
    }

    /// An event for his brain from outside the world (a gesture, the capture box).
    pub(crate) fn tell(&mut self, event: Event) {
        let next = mascot::transition(&self.rambo.brain, event);
        self.adopt(next);
    }

    /// His brain is now `next`, decided outside the world. On a change his timers start
    /// again and he owes the notice beat for it; from flying to writing he catches up
    /// with the row's first letter.
    pub(crate) fn adopt(&mut self, next: State) {
        let rambo = &mut self.rambo;
        if next == rambo.brain {
            return;
        }
        if rambo.brain == State::Home && rambo.settled_home {
            // Out of the corner, always toward the page.
            self.nest.leave(-1.0);
        }
        rambo.owed = mascot::notice(&rambo.brain, &next);
        rambo.look_from = rambo.look;
        if !matches!(next, State::Splatting | State::Dazed) {
            rambo.fell = None;
        }
        rambo.settling = rambo.brain == State::Flying && next == State::WritingRow;
        rambo.brain = next;
        rambo.since = self.now;
        rambo.settled_home = false;
        // Anything he is asked to do stops the tapping; the nib stays as dry as it was.
        rambo.tapping = None;
    }

    /// Whether nothing in the world will change until something outside does (his
    /// state, his spot or the nest): he stands exactly on what his state tracks, with no
    /// run and no catching up, and no timer of his is running. The loop may sleep; the
    /// splat and the daze keep it awake until their timers end them, and a finger
    /// holding him keeps it awake while he rides after it.
    pub(crate) fn at_rest(&self) -> bool {
        let rambo = &self.rambo;
        let timed = matches!(
            rambo.brain,
            State::Thrown | State::Splatting | State::Dazed | State::Covered | State::Mending
        );
        !timed
            && look::still(&rambo.brain, self.now - rambo.since)
            && rambo.sway.abs() < look::SWAY_GONE_DEG
            && rambo.brain != State::Dragging
            && !mascot::hops_there(&rambo.brain, rambo.settled_home)
            && !rambo.settling
            && rambo.run.is_none()
            && rambo.tapping.is_none()
            && self.heard.is_none()
            && self.dance.still()
            && self.nest.at_rest()
    }

    /// The music plays this loud now (0 to 1), or none plays: he dances to it while he
    /// sits in his nest.
    pub(crate) fn hear(&mut self, level: Option<f64>) {
        self.heard = level;
    }

    /// `steps` fixed engine steps with the page as `anchors` measured it.
    pub(crate) fn advance(&mut self, steps: u32, anchors: &Anchors) {
        for _ in 0..steps {
            self.step(anchors);
        }
    }

    /// One fixed step: his own timers first (they may change his state), then his body
    /// follows what his state asks.
    fn step(&mut self, anchors: &Anchors) {
        self.now += STEP_MS;
        if let Some(event) = self.cover_event(anchors.leaf) {
            self.tell(event);
        }
        if let Some(event) = self.timer_due() {
            self.tell(event);
        }
        if anchors.reduced_motion {
            // With less motion the nest never sags or rocks.
            self.nest = Nest::default();
        }
        if matches!(self.rambo.brain, State::WritingRow | State::WritingCaret) {
            self.rambo.written_ms += STEP_MS;
        }
        self.tap_nib();
        // He dances only settled in his nest, not while he taps his nib, and never with less motion asked for.
        let dancing = self.rambo.brain == State::Home
            && self.rambo.settled_home
            && self.rambo.tapping.is_none()
            && !anchors.reduced_motion;
        if let Some(beat) = self.dance.hear(self.heard.filter(|_| dancing), STEP_MS) {
            self.nest.bounce(beat);
        }
        self.nest.step(STEP_MS / 1000.0);
        self.move_body(anchors);
        let rambo = &mut self.rambo;
        let ms = self.now - rambo.since;
        let to = look::pose(&rambo.brain, ms, anchors.reduced_motion);
        rambo.look = look::blended(&rambo.look_from, &to, ms, anchors.reduced_motion);
        let toward = look::sway_toward(&rambo.brain, rambo.vel, anchors.reduced_motion);
        rambo.sway = look::eased_sway(rambo.sway, toward, STEP_MS);
    }

    /// His body follows what his state asks.
    fn move_body(&mut self, anchors: &Anchors) {
        // He sits on the nest's sag, and is where a throw left him while it dazes him.
        let anchors = &Anchors {
            nest: (anchors.nest.0, anchors.nest.1 + self.nest.offset().0),
            spot: self.rambo.fell.unwrap_or(anchors.spot),
            ..*anchors
        };
        let drive = behaviour(
            &self.rambo.brain,
            self.rambo.settled_home,
            self.rambo.pos,
            anchors,
        );
        if !matches!(drive, Drive::HopTo(_)) {
            self.rambo.run = None;
        }
        let rambo = &mut self.rambo;
        let (pos, vel) = match drive {
            Drive::HopTo(to) if anchors.reduced_motion => return self.land(to),
            Drive::HopTo(to) => return self.hop_towards(to),
            Drive::Throw => return self.slide(anchors),
            Drive::Follow(at)
                if rambo.settling
                    && !anchors.reduced_motion
                    && distance(rambo.pos, at) > CAUGHT_UP_PX =>
            {
                ride(rambo.pos, rambo.vel, at)
            }
            Drive::Ride(at) if !anchors.reduced_motion => ride(rambo.pos, rambo.vel, at),
            // Carried along by the page (a scroll, the nest moving) is not his own speed:
            // a grab right after must not fling him past the finger. With reduced motion
            // he is simply on the finger.
            Drive::Follow(at) | Drive::Ride(at) => {
                rambo.settling = false;
                (at, (0.0, 0.0))
            }
        };
        rambo.pos = pos;
        rambo.vel = vel;
        rambo.draw = Draw::at(pos);
    }

    /// The push he gives the leaf's corner from underneath while he is caught: the page
    /// steps its spring with it (`Spring::step_with_push`), and it can lift the leaf a
    /// little but never turn it.
    pub(crate) fn leaf_push(&self) -> Option<crate::flip::Push> {
        (self.rambo.brain == State::Covered).then_some(crate::flip::Push {
            accel: tuning::COVERED_PUSH_ACCEL,
            reach: tuning::COVERED_LIFT_REACH,
        })
    }

    /// The event the leaf raises this step: it closes over him where he stands out on the
    /// page, or lifts away from where he is stuck.
    fn cover_event(&self, leaf: Option<Leaf>) -> Option<Event> {
        let at = self.rambo.pos;
        match self.rambo.brain {
            // No leaf in motion means it lies flat over him: only its lifting away, or the
            // timer, lets him out.
            State::Covered if leaf.is_some_and(|leaf| !leaf.covers(at)) => Some(Event::Uncovered),
            _ if leaf.is_some_and(|leaf| leaf.catches(at)) => Some(Event::Covered),
            _ => None,
        }
    }

    /// The event one of his timers raises this step, if one has run out: the splat
    /// settles into a daze, a daze left untouched sends him home, and the leaf lets go of
    /// him once he has pushed his way out.
    fn timer_due(&self) -> Option<Event> {
        let time_in_state = self.now - self.rambo.since;
        let (after, event) = match self.rambo.brain {
            State::Splatting => (mascot::SPLAT_MS, Event::Return),
            State::Dazed => (mascot::DAZED_MS, Event::Return),
            State::Covered => (mascot::COVERED_MS, Event::Uncovered),
            State::Mending => (mascot::MEND_MS, Event::Return),
            _ => return None,
        };
        // Steps of 1/120 s add up a hair short of whole ms.
        (time_in_state + 1e-6 >= f64::from(after)).then_some(event)
    }

    /// One step of a run of hops toward `to`, planned when the run starts. He lands on
    /// `to` as it is now, and landing in the nest after the run home makes him home.
    fn hop_towards(&mut self, to: Vec2) {
        let now = self.now;
        let rambo = &mut self.rambo;
        let run = match rambo.run {
            Some(run) => run,
            None => match Hop::plan(rambo.pos, to) {
                Some(hop) => *rambo.run.insert(Run {
                    hop,
                    began: now,
                    beat: rambo.owed.take(),
                    landed: 0,
                }),
                // Close enough that he simply steps there.
                None => return self.land(to),
            },
        };
        let since = (now - run.began).max(0.0) as u32;
        if let Some(face) = run.beat.filter(|_| since < run.beat_ms()) {
            // He notices where he stands before he goes.
            rambo.vel = (0.0, 0.0);
            rambo.draw = Draw {
                pos: rambo.pos,
                squash: mascot::notice_squash(since),
                notice: Some(face),
            };
            return;
        }
        let elapsed = since - run.beat_ms();
        if elapsed >= run.hop.ms() {
            return self.land(to);
        }
        // Running home from a daze, each hop that comes down on the page leaves a dot.
        let landed = elapsed / crate::tuning::HOP_MS;
        if landed > run.landed {
            if rambo.brain == State::Skittering {
                let dots = (run.landed + 1..=landed)
                    .map(|hop| Splash::Dot(run.hop.at(to, hop * crate::tuning::HOP_MS).pos));
                self.ink.extend(dots);
            }
            let rambo = &mut self.rambo;
            if let Some(run) = rambo.run.as_mut() {
                run.landed = landed;
            }
        }
        let rambo = &mut self.rambo;
        let frame = run.hop.at(to, elapsed);
        rambo.vel = velocity(rambo.pos, frame.pos);
        rambo.pos = frame.pos;
        rambo.draw = Draw {
            pos: (frame.pos.0, frame.pos.1 - frame.lift),
            squash: frame.squash,
            notice: None,
        };
    }

    /// One step of a throw: he slides and bounces off the edges. Into the corner, the
    /// nest catches him: it sags under him and he settles in, no splat. A smack splats
    /// him where he hit and throws ink off the edge; a throw that just stops sends him
    /// home.
    fn slide(&mut self, anchors: &Anchors) {
        let walls = &anchors.walls;
        let rambo = &mut self.rambo;
        let per_ms = (rambo.vel.0 / 1000.0, rambo.vel.1 / 1000.0);
        let (body, hit) = fling::step(
            Body {
                pos: rambo.pos,
                vel: per_ms,
            },
            walls,
            STEP_MS,
        );
        rambo.pos = body.pos;
        rambo.vel = (body.vel.0 * 1000.0, body.vel.1 * 1000.0);
        rambo.draw = Draw::at(body.pos);
        if caught(body.pos, anchors.nest) {
            let speed = per_ms.0.hypot(per_ms.1);
            rambo.vel = (0.0, 0.0);
            self.nest.catch(speed, per_ms.0);
            self.tell(Event::DropNest);
            // Caught, he drops straight in: no beat before the little hop.
            self.rambo.owed = None;
            return;
        }
        match hit {
            Some(hit) if hit.splats() => {
                rambo.vel = (0.0, 0.0);
                rambo.fell = Some(body.pos);
                let seed = format!("{}", self.now);
                self.ink
                    .push(Splash::Splat(fling::splat_strokes(&hit, &seed)));
                self.tell(Event::Smack);
            }
            _ if fling::at_rest(&body) => {
                rambo.vel = (0.0, 0.0);
                self.tell(Event::Return);
            }
            _ => {}
        }
    }

    /// The taps of his dry nib on the rim that have come due: each one knocks the nest,
    /// and he bobs on it as it springs back. Done, he rests.
    fn tap_nib(&mut self) {
        let Some((began, done)) = self.rambo.tapping else {
            return;
        };
        let ms = self.now - began;
        let due = ((ms - tuning::DRY_NIB_PAUSE_MS) / tuning::DRY_NIB_TAP_MS)
            .floor()
            .clamp(-1.0, f64::from(tuning::DRY_NIB_TAPS) - 1.0);
        let due = (due + 1.0) as u32;
        (done..due).for_each(|_| self.nest.tap());
        self.rambo.tapping = (ms < tapping_ms()).then_some((began, due));
    }

    /// He has arrived at `at`: still, unsquashed, and home if he was on his way home.
    fn land(&mut self, at: Vec2) {
        let rambo = &mut self.rambo;
        let sideways = rambo.vel.0;
        rambo.run = None;
        rambo.settling = false;
        rambo.pos = at;
        rambo.vel = (0.0, 0.0);
        rambo.draw = Draw::at(at);
        if rambo.brain == State::Skittering {
            self.tell(Event::Return);
        }
        let rambo = &mut self.rambo;
        rambo.settling = false;
        rambo.settled_home = rambo.brain == State::Home;
        if rambo.settled_home {
            // Still moving a little sideways as he comes down, he lands off its middle.
            self.nest.land(sideways * LANDING_LEAN);
            // Home after a long stretch of writing, he taps his dry nib on the rim.
            if mascot::nib_dry(rambo.written_ms) {
                rambo.written_ms = 0.0;
                rambo.tapping = Some((self.now, 0));
            }
        }
    }
}

/// How long his taps of a dry nib take, from landing to the end of the last tap's swing.
fn tapping_ms() -> f64 {
    tuning::DRY_NIB_PAUSE_MS + f64::from(tuning::DRY_NIB_TAPS + 1) * tuning::DRY_NIB_TAP_MS
}

/// How far off the nest's middle a landing pushes, in px per px/s he was still moving
/// sideways as he came down.
const LANDING_LEAN: f64 = 0.01;

/// Whether a throw at `pos` has reached the corner the nest fills: up to
/// `NEST_CATCH_PX` above and left of its middle, and anywhere past it.
fn caught(pos: Vec2, nest: Vec2) -> bool {
    pos.0 >= nest.0 - crate::tuning::NEST_CATCH_PX && pos.1 >= nest.1 - crate::tuning::NEST_CATCH_PX
}

fn distance(a: Vec2, b: Vec2) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// The velocity, in px per second, that took him from `from` to `to` in one step.
fn velocity(from: Vec2, to: Vec2) -> Vec2 {
    let per_s = 1.0 / ENGINE_DT_S;
    ((to.0 - from.0) * per_s, (to.1 - from.1) * per_s)
}

/// One step of the ride spring pulling him from `pos` at `vel` toward `to`: critically
/// damped at [`RIDE_OMEGA`], stepped semi-implicitly so it stays stable at the fixed step.
fn ride(pos: Vec2, vel: Vec2, to: Vec2) -> (Vec2, Vec2) {
    let axis = |x: f64, v: f64, target: f64| {
        let accel = RIDE_OMEGA * RIDE_OMEGA * (target - x) - 2.0 * RIDE_OMEGA * v;
        let v = v + accel * ENGINE_DT_S;
        (x + v * ENGINE_DT_S, v)
    };
    let (x, vx) = axis(pos.0, vel.0, to.0);
    let (y, vy) = axis(pos.1, vel.1, to.1);
    ((x, y), (vx, vy))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEST: Vec2 = (340.0, 780.0);

    /// A phone screen less half of him.
    const WALLS: Walls = Walls {
        left: 32.0,
        top: 32.0,
        right: 358.0,
        bottom: 812.0,
    };

    fn anchors(spot: Vec2) -> Anchors {
        Anchors {
            nest: NEST,
            spot,
            reduced_motion: false,
            leaf: None,
            walls: WALLS,
        }
    }

    /// Where he sits in a nest whose middle is `nest`: on its sag.
    fn in_nest(world: &World, nest: Vec2) -> Vec2 {
        (nest.0, nest.1 + world.nest_offset().0)
    }

    /// Long enough for his lean into a run and the nest under him to settle.
    const SETTLES_MS: f64 = 2000.0;

    /// Steps for `ms` of world time.
    fn steps(ms: f64) -> u32 {
        (ms / STEP_MS).ceil() as u32
    }

    /// A small seeded generator in -1..1, so the property runs are the same every time.
    struct Seeded(u64);

    impl Seeded {
        fn unit(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        }
    }

    #[test]
    fn the_paper_he_lies_on_carries_him_slowly_and_flings_him_fast() {
        let lying = [
            State::Dazed,
            State::Asking {
                task_id: "t1".into(),
                title: "Lab report".into(),
            },
        ];
        let elsewhere = [
            State::Home,
            State::Dragging,
            State::Thrown,
            State::Splatting,
            State::Skittering,
            State::Inking,
            State::Flying,
            State::WritingRow,
            State::Covered,
        ];
        let mut seeded = Seeded(12);
        for _ in 0..500 {
            let delta = (seeded.unit() * 80.0, seeded.unit() * 80.0);
            let dt_ms = 1.0 + seeded.unit().abs() * 100.0;
            let speed = delta.0.hypot(delta.1) / dt_ms;
            for state in &elsewhere {
                assert_eq!(swept(state, delta, dt_ms, false), Swept::Stays, "{state:?}");
            }
            for state in &lying {
                match swept(state, delta, dt_ms, false) {
                    // Holding on, he keeps his place on the paper exactly.
                    Swept::Carried(moved) => {
                        assert_eq!(moved, delta);
                        assert!(speed <= tuning::SWEEP_GRIP_PX_MS);
                    }
                    // Slipping, he goes the paper's way at its speed.
                    Swept::Flung(vel) => {
                        assert!(speed > tuning::SWEEP_GRIP_PX_MS);
                        assert!((vel.0 * dt_ms - delta.0).abs() < 1e-9);
                        assert!((vel.1 * dt_ms - delta.1).abs() < 1e-9);
                    }
                    Swept::Stays => panic!("{state:?} lies on the page"),
                }
                assert_eq!(
                    swept(state, delta, dt_ms, true),
                    Swept::Carried(delta),
                    "with reduced motion he only rides along"
                );
            }
        }
    }

    #[test]
    fn a_page_scrolled_hard_under_him_smacks_him_into_the_edge_and_a_gentler_one_does_not() {
        let fling = |vel: Vec2| {
            let at = (200.0, 500.0);
            let mut world = dropped_at(at);
            world.take_ink();
            let Swept::Flung(vel) = swept(
                &world.rambo.brain,
                (vel.0 * 16.0, vel.1 * 16.0),
                16.0,
                false,
            ) else {
                panic!("{vel:?} is past his grip");
            };
            world.throw(vel);
            world.tell(Event::Swept);
            assert_eq!(world.rambo.brain, State::Thrown);
            let mut seen = vec![world.rambo.brain.clone()];
            for _ in 0..steps(3000.0) {
                world.advance(1, &anchors(at));
                if seen.last() != Some(&world.rambo.brain) {
                    seen.push(world.rambo.brain.clone());
                }
            }
            (seen, world.take_ink())
        };
        // Scrolled up hard, the paper flings him into the top of the screen.
        let (seen, ink) = fling((0.0, -3.0));
        assert!(seen.contains(&State::Splatting), "{seen:?}");
        assert!(!ink.is_empty(), "the smack splats ink");
        // A little past his grip, he slides, stops short of the edge and hops home.
        let (seen, ink) = fling((0.0, -0.9));
        assert!(!seen.contains(&State::Splatting), "{seen:?}");
        assert_eq!(seen.last(), Some(&State::Home));
        assert_eq!(ink, Vec::new());
    }

    /// Written at the caret for `ms`, then sent home and settled there: how many times he
    /// tapped his nib on the rim, and whether he tipped toward it meanwhile.
    fn home_after_writing(ms: f64) -> (World, u32, bool) {
        let mut world = World::new(NEST);
        world.tell(Event::WriteCaret);
        world.advance(steps(ms), &anchors((60.0, 700.0)));
        world.tell(Event::FlyHome);
        world.tell(Event::FlyHome);
        assert_eq!(world.rambo.brain, State::Home);
        let (mut taps, mut tipped) = (0, false);
        for _ in 0..steps(f64::from(crate::tuning::FLY_MS) + SETTLES_MS) {
            world.advance(1, &anchors(NEST));
            if let Some((_, done)) = world.rambo.tapping {
                taps = taps.max(done);
                tipped |= world.look().lean_deg < -1.0;
            }
        }
        (world, taps, tipped)
    }

    #[test]
    fn after_a_long_stretch_of_writing_he_taps_his_dry_nib_on_the_rim_then_rests() {
        let (world, taps, tipped) = home_after_writing(tuning::DRY_NIB_AFTER_MS + 500.0);
        assert_eq!(taps, tuning::DRY_NIB_TAPS);
        assert!(tipped, "he tips toward the rim to tap");
        assert_eq!(world.rambo.written_ms, 0.0, "a fresh nib");
        assert!(world.at_rest(), "then the loop may sleep");
        assert_eq!(world.draw().pos, in_nest(&world, NEST));
    }

    #[test]
    fn only_a_dry_nib_is_tapped_and_only_once_he_is_home() {
        // Seeded writing lengths either side of the dry point.
        let mut seed = 3_u64;
        for _ in 0..6 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let ms = 2000.0 + (seed >> 33) as f64 % 20_000.0;
            let (_, taps, _) = home_after_writing(ms);
            // The world counts whole steps of writing, so judge by what it counted.
            let counted = f64::from(steps(ms)) * STEP_MS;
            let expected = match mascot::nib_dry(counted) {
                true => tuning::DRY_NIB_TAPS,
                false => 0,
            };
            assert_eq!(taps, expected, "after {ms} ms of writing");
        }
        // Writing is never interrupted by a tap: away from home there is none.
        let mut world = World::new(NEST);
        world.tell(Event::WriteCaret);
        for _ in 0..steps(tuning::DRY_NIB_AFTER_MS + 2000.0) {
            world.advance(1, &anchors((60.0, 700.0)));
            assert_eq!(world.rambo.tapping, None);
        }
    }

    #[test]
    fn picked_up_mid_tap_he_stops_and_the_nest_settles() {
        let mut world = World::new(NEST);
        world.tell(Event::WriteCaret);
        world.advance(
            steps(tuning::DRY_NIB_AFTER_MS + 500.0),
            &anchors((60.0, 700.0)),
        );
        world.tell(Event::FlyHome);
        world.tell(Event::FlyHome);
        for _ in 0..steps(f64::from(crate::tuning::FLY_MS) + 1000.0) {
            world.advance(1, &anchors(NEST));
            if world.rambo.tapping.is_some() {
                break;
            }
        }
        assert!(world.rambo.tapping.is_some(), "tapping");
        world.tell(Event::Grab);
        assert_eq!(world.rambo.tapping, None);
        world.advance(steps(SETTLES_MS), &anchors((200.0, 400.0)));
        assert_eq!(world.tap_lean(), 0.0, "no tap lean left");
        world.tell(Event::DropEmpty);
        world.advance(
            steps(f64::from(crate::tuning::FLY_MS) + SETTLES_MS),
            &anchors((200.0, 400.0)),
        );
        assert_eq!(
            world.rambo.tapping, None,
            "the nib was dried: no second round"
        );
        assert!(world.at_rest());
    }

    /// Dazed at `spot`, as a smack or a writing drag that ends there leaves him.
    fn dropped_at(spot: Vec2) -> World {
        let mut world = World::new(NEST);
        world.tell(Event::Grab);
        world.advance(steps(500.0), &anchors(spot));
        world.adopt(State::Dazed);
        world.advance(1, &anchors(spot));
        world
    }

    /// Carried to `from` and let go mid-flick at `vel` px per ms.
    fn thrown_from(from: Vec2, vel: Vec2) -> World {
        let mut world = World::new(NEST);
        world.tell(Event::Grab);
        world.advance(steps(500.0), &anchors(from));
        world.throw(vel);
        world.tell(Event::DropHard);
        world
    }

    #[test]
    fn a_gentle_drop_on_the_page_sends_him_hopping_home_with_no_ink() {
        let mut world = World::new(NEST);
        world.tell(Event::Grab);
        world.advance(steps(500.0), &anchors((120.0, 300.0)));
        world.tell(Event::DropEmpty);
        assert_eq!(world.rambo.brain, State::Home);
        world.advance(
            steps(f64::from(crate::tuning::FLY_MS) + 200.0),
            &anchors((120.0, 300.0)),
        );
        assert_eq!(world.draw().pos, in_nest(&world, NEST));
        world.advance(steps(SETTLES_MS), &anchors((120.0, 300.0)));
        assert!(world.at_rest());
        assert_eq!(world.take_ink(), Vec::new());
    }

    #[test]
    fn a_soft_throw_slides_on_stops_and_he_hops_home_undazed_with_no_ink() {
        let from = (150.0, 400.0);
        let mut world = thrown_from(from, (0.8, 0.0));
        let mut was = from;
        for _ in 0..steps(3000.0) {
            world.advance(1, &anchors(from));
            if world.rambo.brain != State::Thrown {
                break;
            }
            assert!(
                world.rambo.pos.0 >= was.0,
                "he slides on the way he was thrown"
            );
            was = world.rambo.pos;
        }
        assert!(was.0 > from.0 + 40.0, "he kept the throw's speed: {was:?}");
        assert_eq!(world.rambo.brain, State::Home, "stopped without a smack");
        assert_eq!(world.take_ink(), Vec::new(), "no smack, no ink");
    }

    #[test]
    fn smacked_into_an_edge_he_splats_ink_and_is_dazed_where_he_hit_then_skitters_home_dotting() {
        let from = (200.0, 300.0);
        let mut world = thrown_from(from, (3.0, 0.0));
        for _ in 0..steps(500.0) {
            world.advance(1, &anchors(from));
            if world.rambo.brain != State::Thrown {
                break;
            }
        }
        assert_eq!(world.rambo.brain, State::Splatting);
        let hit = world.rambo.pos;
        assert!(
            hit.0 > WALLS.right - 20.0,
            "against the right edge: {hit:?}"
        );
        let ink = world.take_ink();
        assert!(
            matches!(ink.as_slice(), [Splash::Splat(strokes)] if !strokes.is_empty()),
            "{ink:?}"
        );
        // His spot is still where the finger let go; he stays where he hit.
        let dazed_for = f64::from(mascot::SPLAT_MS + mascot::DAZED_MS);
        world.advance(steps(dazed_for) - 2, &anchors(from));
        assert_eq!(world.rambo.brain, State::Dazed);
        assert_eq!(world.draw().pos, hit);
        world.advance(2, &anchors(from));
        assert_eq!(world.rambo.brain, State::Skittering);
        world.advance(
            steps(f64::from(crate::tuning::FLY_MS) + 200.0),
            &anchors(from),
        );
        assert_eq!(world.rambo.brain, State::Home);
        assert_eq!(world.draw().pos, in_nest(&world, NEST));
        let dots: Vec<Vec2> = world
            .take_ink()
            .into_iter()
            .map(|splash| match splash {
                Splash::Dot(at) => at,
                other => panic!("only dots on the way home: {other:?}"),
            })
            .collect();
        assert!(!dots.is_empty(), "each hop on the page leaves a dot");
        for dot in &dots {
            assert!(
                distance(*dot, NEST) > 1.0 && distance(*dot, NEST) < distance(hit, NEST),
                "{dot:?} lies between where he hit and the nest"
            );
        }
    }

    #[test]
    fn a_torn_page_sends_him_to_tape_it_and_home_again_with_no_ink() {
        let tear = (120.0, 300.0);
        let mut world = World::new(NEST);
        world.tell(Event::Mend);
        assert_eq!(world.rambo.brain, State::Mending);
        // He hops there and waits at the tear until the tape is on.
        let there = steps(f64::from(crate::tuning::FLY_MS) * 2.0);
        world.advance(there, &anchors(tear));
        assert_eq!(world.draw().pos, tear);
        assert_eq!(world.rambo.brain, State::Mending, "still taping");
        world.advance(steps(f64::from(mascot::MEND_MS)), &anchors(tear));
        assert_eq!(world.rambo.brain, State::Home, "the tape is on");
        world.advance(there, &anchors(tear));
        assert_eq!(world.draw().pos, in_nest(&world, NEST), "and he hops home");
        world.advance(steps(SETTLES_MS), &anchors(tear));
        assert!(world.at_rest());
        assert_eq!(world.take_ink(), Vec::new(), "he is not dazed: no dots");
    }

    #[test]
    fn any_throw_into_the_corner_is_caught_by_the_nest_without_a_daze_or_ink() {
        // From all over the screen, hard and soft, aimed at the corner the nest fills.
        for &from in &[
            (60.0, 100.0),
            (200.0, 400.0),
            (100.0, 700.0),
            (320.0, 200.0),
        ] {
            for speed in [1.5, 2.5, 4.0] {
                let (dx, dy) = (NEST.0 - from.0, NEST.1 - from.1);
                let len = dx.hypot(dy);
                let mut world = thrown_from(from, (dx / len * speed, dy / len * speed));
                let mut deepest: f64 = 0.0;
                for _ in 0..steps(4000.0) {
                    world.advance(1, &anchors(from));
                    assert!(
                        !matches!(world.rambo.brain, State::Splatting | State::Dazed),
                        "{from:?} at {speed}: no daze in the nest"
                    );
                    deepest = deepest.max(world.nest_offset().0);
                }
                assert_eq!(world.rambo.brain, State::Home, "{from:?} at {speed}");
                assert!(deepest > 1.0, "the nest sagged under the catch: {deepest}");
                assert_eq!(world.take_ink(), Vec::new(), "{from:?} at {speed}: no ink");
                assert!(world.at_rest());
                assert_eq!(world.draw().pos, in_nest(&world, NEST));
            }
        }
    }

    #[test]
    fn a_throw_away_from_the_corner_is_not_caught() {
        let from = (200.0, 300.0);
        let mut world = thrown_from(from, (-3.0, 0.0));
        let mut deepest: f64 = 0.0;
        for _ in 0..steps(500.0) {
            world.advance(1, &anchors(from));
            deepest = deepest.max(world.nest_offset().0);
            if world.rambo.brain != State::Thrown {
                break;
            }
        }
        assert_eq!(world.rambo.brain, State::Splatting, "smacked the left edge");
        // Only his leaving it moved the nest (a little up): nothing came down in it.
        assert!(deepest < 0.5, "no catch: {deepest}");
    }

    #[test]
    fn the_nest_rocks_as_he_leaves_and_sags_as_he_comes_home_then_settles() {
        let mut world = World::new(NEST);
        world.tell(Event::Grab);
        world.advance(steps(100.0), &anchors((200.0, 400.0)));
        assert!(world.nest_offset().1.abs() > 0.1, "it rocked as he left");
        world.advance(steps(SETTLES_MS), &anchors((200.0, 400.0)));
        assert!(world.nest_offset().1.abs() < 0.05, "and settled");
        world.tell(Event::DropEmpty);
        let mut deepest: f64 = 0.0;
        for _ in 0..steps(f64::from(crate::tuning::FLY_MS) + 500.0) {
            world.advance(1, &anchors((200.0, 400.0)));
            deepest = deepest.max(world.nest_offset().0);
        }
        assert!(deepest > 1.0, "it sagged as he came down in it: {deepest}");
        world.advance(steps(SETTLES_MS), &anchors((200.0, 400.0)));
        assert!(world.at_rest(), "it settles, and the loop may sleep");
    }

    /// Piano notes struck every `every_ms` (loud at once, dying away), as he hears them
    /// for `ms` with the page as `anchors` measured it: how deep the nest bounced and
    /// how far he swayed each way.
    fn danced(world: &mut World, every_ms: f64, ms: f64, at: &Anchors) -> (f64, f64, f64) {
        let (mut sag, mut left, mut right) = (0.0_f64, 0.0_f64, 0.0_f64);
        for step in 0..steps(ms) {
            let into = (f64::from(step) * STEP_MS) % every_ms;
            world.hear(Some(0.3 * (-into / 250.0).exp()));
            world.advance(1, at);
            assert!(!world.at_rest(), "music keeps the loop awake");
            sag = sag.max(world.nest_offset().0);
            left = left.min(world.dance.lean_deg());
            right = right.max(world.dance.lean_deg());
        }
        (sag, left, right)
    }

    #[test]
    fn settled_in_his_nest_he_dances_to_the_music_and_stops_when_it_stops() {
        let mut world = World::new(NEST);
        world.advance(steps(SETTLES_MS), &anchors(NEST));
        let (sag, left, right) = danced(&mut world, 700.0, 6000.0, &anchors(NEST));
        assert!(sag > 1.0, "the nest bounces to the beat: {sag}");
        assert!(
            left < -1.0 && right > 1.0,
            "he sways both ways: {left} to {right}"
        );
        world.hear(None);
        world.advance(steps(5000.0), &anchors(NEST));
        assert!(
            world.at_rest(),
            "the music stopped: he settles and the loop may sleep"
        );
        assert!(world.dance.lean_deg().abs() < 0.05);
    }

    #[test]
    fn away_from_his_nest_or_with_less_motion_he_does_not_dance() {
        // Held, writing or dazed, the music plays on and he does not sway to it.
        let mut held = World::new(NEST);
        held.tell(Event::Grab);
        let (_, left, right) = danced(&mut held, 700.0, 3000.0, &anchors((200.0, 400.0)));
        assert_eq!((left, right), (0.0, 0.0));
        let mut calm = World::new(NEST);
        calm.advance(steps(SETTLES_MS), &anchors(NEST));
        let less = Anchors {
            reduced_motion: true,
            ..anchors(NEST)
        };
        let (sag, left, right) = danced(&mut calm, 700.0, 3000.0, &less);
        assert_eq!((sag, left, right), (0.0, 0.0, 0.0));
    }

    #[test]
    fn with_reduced_motion_the_nest_never_moves() {
        let mut world = World::new(NEST);
        let calm = Anchors {
            reduced_motion: true,
            ..anchors((200.0, 400.0))
        };
        world.tell(Event::Grab);
        world.advance(steps(100.0), &calm);
        world.tell(Event::DropEmpty);
        for _ in 0..steps(1000.0) {
            world.advance(1, &calm);
            assert_eq!(world.nest_offset(), (0.0, 0.0));
        }
    }

    #[test]
    fn caught_mid_slide_he_is_held_again() {
        let from = (150.0, 400.0);
        let mut world = thrown_from(from, (1.0, 0.0));
        world.advance(steps(100.0), &anchors(from));
        assert_eq!(world.rambo.brain, State::Thrown);
        world.tell(Event::Grab);
        assert_eq!(world.rambo.brain, State::Dragging);
        world.advance(steps(500.0), &anchors(from));
        assert!(distance(world.rambo.pos, from) < 1.0);
        assert!(!world.at_rest(), "held, never asleep");
    }

    #[test]
    fn settled_at_home_he_moves_with_the_nest_exactly_and_never_hops() {
        let mut world = World::new(NEST);
        assert_eq!(world.look().sink, 1.0, "sunk in the nest from the start");
        let scrolled = Anchors {
            nest: (340.0, 700.0),
            ..anchors((0.0, 0.0))
        };
        world.advance(1, &scrolled);
        assert_eq!(world.draw(), Draw::at(scrolled.nest));
        assert!(world.at_rest(), "the loop may sleep");
    }

    fn asking() -> State {
        State::Asking {
            task_id: "t1".into(),
            title: "buy eggs".into(),
        }
    }

    #[test]
    fn asking_on_his_spot_he_is_at_rest_for_as_long_as_he_asks() {
        let spot = (120.0, 300.0);
        let mut world = World::new(NEST);
        world.adopt(asking());
        world.advance(steps(1000.0), &anchors(spot));
        assert_eq!(
            world.draw(),
            Draw::at(spot),
            "he has caught up with his spot"
        );
        for _ in 0..steps(10_000.0) {
            world.advance(1, &anchors(spot));
            assert!(world.at_rest(), "nothing changes, so the loop may sleep");
        }
        assert_eq!(world.rambo.brain, asking());
    }

    #[test]
    fn dazed_he_is_never_at_rest_until_the_daze_ends_and_he_is_home() {
        let fell = (120.0, 300.0);
        let mut world = dropped_at(fell);
        while world.rambo.brain == State::Dazed {
            assert!(!world.at_rest(), "his daze timer must keep the loop awake");
            world.advance(1, &anchors(fell));
        }
        for _ in 0..steps(f64::from(crate::tuning::FLY_MS) + 2000.0) {
            if world.at_rest() {
                break;
            }
            world.advance(1, &anchors(fell));
        }
        assert!(world.at_rest(), "home and asleep");
        assert_eq!(world.rambo.brain, State::Home);
        assert_eq!(world.draw().pos, in_nest(&world, NEST));
    }

    #[test]
    fn a_state_set_from_outside_mid_run_ends_the_run_and_he_rests_on_the_new_spot() {
        let fell = (120.0, 300.0);
        let mut world = dropped_at(fell);
        world.advance(steps(f64::from(mascot::DAZED_MS)), &anchors(fell));
        assert_eq!(world.rambo.brain, State::Skittering);
        world.advance(steps(f64::from(NOTICE_BEAT_MS) + 100.0), &anchors(fell));
        assert!(world.rambo.run.is_some(), "mid-hop home");
        let spot = (200.0, 500.0);
        world.adopt(asking());
        world.advance(1, &anchors(spot));
        assert_eq!(
            world.draw(),
            Draw::at(spot),
            "on his spot at once, the run gone"
        );
        assert!(!world.at_rest(), "still easing into the asking pose");
        world.advance(steps(crate::tuning::WRITE_BLEND_MS), &anchors(spot));
        world.advance(steps(SETTLES_MS), &anchors(spot));
        assert!(world.at_rest());
        world.advance(steps(10_000.0), &anchors(spot));
        assert!(world.at_rest());
        assert_eq!(world.rambo.brain, asking(), "nothing in the world undid it");
        assert_eq!(world.draw(), Draw::at(spot));
    }

    #[test]
    fn inking_his_nib_is_on_the_finger_every_frame_from_the_first_move() {
        let fell = (120.0, 300.0);
        let mut world = dropped_at(fell);
        world.adopt(State::Inking);
        // The finger is 20 px off his middle when it grabs, then draws at 600 px/s.
        let mut finger = (fell.0 + 20.0, fell.1);
        for _ in 0..steps(1000.0) {
            finger.0 += 600.0 * ENGINE_DT_S;
            world.advance(1, &anchors(finger));
            assert_eq!(world.draw(), Draw::at(finger));
        }
    }

    #[test]
    fn with_reduced_motion_a_held_or_settling_rambo_is_on_his_spot() {
        let spot = (100.0, 200.0);
        let still = Anchors {
            reduced_motion: true,
            leaf: None,
            ..anchors(spot)
        };
        let mut world = World::new(NEST);
        world.tell(Event::Grab);
        world.advance(1, &still);
        assert_eq!(world.draw(), Draw::at(spot), "held, he is on the finger");
        world.adopt(State::Flying);
        world.advance(1, &still);
        world.adopt(State::WritingRow);
        let letter = (60.0, 420.0);
        world.advance(
            1,
            &Anchors {
                reduced_motion: true,
                leaf: None,
                ..anchors(letter)
            },
        );
        assert_eq!(
            world.draw(),
            Draw::at(letter),
            "no ride to the first letter"
        );
    }

    #[test]
    fn held_he_rides_toward_the_finger_behind_it_and_never_past_it() {
        let mut world = World::new(NEST);
        world.tell(Event::Grab);
        assert!(!world.at_rest());
        let spot = (100.0, 200.0);
        let mut last_gap = f64::INFINITY;
        for _ in 0..steps(400.0) {
            world.advance(1, &anchors(spot));
            let (x, y) = world.rambo.pos;
            let gap = distance((x, y), spot);
            assert!(
                gap <= last_gap + 1e-9,
                "never swings past: {gap} after {last_gap}"
            );
            assert!(x >= spot.0 - 1e-9 && y >= spot.1 - 1e-9, "never overshoots");
            last_gap = gap;
        }
        assert!(last_gap < 1.0, "he has caught up: {last_gap}");
    }

    #[test]
    fn grabbed_just_as_the_page_scrolls_he_still_never_passes_the_finger() {
        let mut world = World::new(NEST);
        world.advance(1, &anchors((0.0, 0.0)));
        // A quick flick scrolls the page 60 px in one step under him, settled in his nest.
        let scrolled = Anchors {
            nest: (NEST.0, NEST.1 - 60.0),
            ..anchors((0.0, 0.0))
        };
        world.advance(1, &scrolled);
        world.tell(Event::Grab);
        // The finger just above him: 20 px is nothing to the ride spring, but 7200 px/s
        // of borrowed speed would carry him past it.
        let spot = (NEST.0, NEST.1 - 80.0);
        for _ in 0..steps(400.0) {
            world.advance(1, &anchors(spot));
            assert!(
                world.rambo.pos.1 >= spot.1 - 1e-9,
                "past the finger: {:?}",
                world.rambo.pos
            );
        }
    }

    #[test]
    fn hopping_home_his_lean_follows_his_hops_and_never_jumps() {
        let fell = (40.0, 200.0);
        let mut world = dropped_at(fell);
        world.advance(steps(f64::from(mascot::DAZED_MS)), &anchors(fell));
        assert_eq!(world.rambo.brain, State::Skittering);
        let mut was = world.look().lean_deg;
        let mut most: f64 = 0.0;
        for _ in 0..steps(f64::from(crate::tuning::FLY_MS) + 1500.0) {
            world.advance(1, &anchors(fell));
            let lean = world.look().lean_deg;
            assert!(
                (lean - was).abs() < 1.0,
                "a jump in his lean: {was} -> {lean}"
            );
            most = most.max(world.rambo.sway.abs());
            was = lean;
        }
        assert!(most > 0.5, "he leans into the run: {most}");
        assert!(
            most < crate::tuning::LEAN_MAX_DEG,
            "never pinned at the most: {most}"
        );
        assert!(world.at_rest(), "home, upright and asleep");
        assert!(world.look().lean_deg.abs() < look::SWAY_GONE_DEG);
    }

    #[test]
    fn dropped_he_stays_dazed_where_he_fell_then_hops_home_and_is_home() {
        let fell = (120.0, 300.0);
        let mut world = dropped_at(fell);
        assert_eq!(world.rambo.brain, State::Dazed);
        world.advance(steps(f64::from(mascot::DAZED_MS)) - 2, &anchors(fell));
        assert_eq!(
            world.rambo.brain,
            State::Dazed,
            "still dazed just before the time"
        );
        assert_eq!(world.draw().pos, fell);
        world.advance(2, &anchors(fell));
        assert_eq!(world.rambo.brain, State::Skittering);
        let mut lifted = false;
        for _ in 0..steps(f64::from(crate::tuning::FLY_MS) + 200.0) {
            world.advance(1, &anchors(fell));
            lifted |= world.draw().pos.1 < world.rambo.pos.1;
        }
        assert!(lifted, "he hopped home, off the ground");
        assert_eq!(world.rambo.brain, State::Home);
        assert_eq!(world.draw().pos, in_nest(&world, NEST));
        world.advance(steps(SETTLES_MS), &anchors(fell));
        assert!(world.at_rest());
    }

    #[test]
    fn a_splat_settles_into_a_daze_where_he_fell() {
        let mut world = World::new(NEST);
        world.adopt(State::Splatting);
        world.advance(steps(f64::from(mascot::SPLAT_MS)), &anchors((50.0, 60.0)));
        assert_eq!(world.rambo.brain, State::Dazed);
        assert_eq!(world.draw().pos, (50.0, 60.0));
    }

    #[test]
    fn a_daze_starts_again_from_any_new_state() {
        // Picked up and dropped again halfway through a daze: a whole new daze.
        let fell = (120.0, 300.0);
        let mut world = dropped_at(fell);
        world.advance(steps(f64::from(mascot::DAZED_MS) / 2.0), &anchors(fell));
        world.tell(Event::Grab);
        world.tell(Event::Release);
        let before = world.rambo.brain.clone();
        world.advance(steps(f64::from(mascot::DAZED_MS) * 0.75), &anchors(fell));
        assert_eq!((before, &world.rambo.brain), (State::Dazed, &State::Dazed));
    }

    #[test]
    fn the_run_home_lands_on_the_nest_where_it_is_now() {
        let fell = (60.0, 200.0);
        let mut world = dropped_at(fell);
        world.advance(steps(f64::from(mascot::DAZED_MS)) + 1, &anchors(fell));
        assert_eq!(world.rambo.brain, State::Skittering);
        // The page scrolls while he hops: the nest moves up 80 px.
        let moved = Anchors {
            nest: (NEST.0, NEST.1 - 80.0),
            ..anchors(fell)
        };
        world.advance(steps(f64::from(crate::tuning::FLY_MS) + 200.0), &moved);
        assert_eq!(world.rambo.brain, State::Home);
        assert_eq!(world.draw().pos, in_nest(&world, moved.nest));
    }

    #[test]
    fn every_run_starts_with_his_notice_beat_where_he_stands() {
        let mut world = World::new(NEST);
        world.tell(Event::WriteCaret);
        let caret = (60.0, 700.0);
        let face = mascot::notice(&State::Home, &State::WritingCaret);
        assert!(face.is_some());
        for _ in 0..steps(f64::from(NOTICE_BEAT_MS)) - 1 {
            world.advance(1, &anchors(caret));
            assert_eq!(world.draw().pos, NEST, "he has not set off");
            assert_eq!(world.draw().notice, face);
        }
        world.advance(steps(50.0), &anchors(caret));
        assert_eq!(world.draw().notice, None);
        assert_ne!(world.rambo.pos, NEST, "now he hops");
    }

    #[test]
    fn the_beat_waits_for_the_first_run_that_sets_off() {
        // His state changes before anyone has set his spot: a step, and the beat stays
        // owed until the spot moves far enough to hop.
        let mut world = World::new(NEST);
        world.tell(Event::WriteCaret);
        world.advance(1, &anchors((NEST.0 - 2.0, NEST.1)));
        assert_eq!(world.draw().notice, None);
        world.advance(1, &anchors((60.0, 700.0)));
        assert!(world.draw().notice.is_some());
    }

    #[test]
    fn from_flying_to_writing_he_rides_to_the_first_letter_and_never_jumps() {
        // Flown to a row, its first letter is 20 px on (Dario on #337): no jump.
        let mut world = World::new(NEST);
        world.tell(Event::WriteRow);
        let row = (40.0, 120.0);
        world.advance(
            steps(f64::from(crate::tuning::FLY_MS) + 50.0),
            &anchors(row),
        );
        assert_eq!(world.rambo.pos, row, "landed");
        world.tell(Event::WriteRow);
        assert_eq!(world.rambo.brain, State::WritingRow);
        let letter = (row.0 + 16.0, row.1 + 12.0);
        let mut at = world.rambo.pos;
        for _ in 0..steps(300.0) {
            world.advance(1, &anchors(letter));
            assert!(
                distance(at, world.rambo.pos) < 6.0,
                "a jump: {at:?} to {:?}",
                world.rambo.pos
            );
            at = world.rambo.pos;
        }
        assert_eq!(world.draw().pos, letter, "then he follows it exactly");
    }

    #[test]
    fn a_run_cut_short_by_a_new_state_stops_hopping() {
        let mut world = World::new(NEST);
        world.tell(Event::WriteRow);
        let row = (40.0, 120.0);
        world.advance(steps(250.0), &anchors(row));
        assert!(world.rambo.run.is_some(), "under way");
        world.tell(Event::WriteRow);
        world.advance(1, &anchors(row));
        assert_eq!(world.rambo.run, None);
        assert_eq!(world.draw().squash, 0.0);
    }

    #[test]
    fn a_move_too_short_to_hop_is_a_step() {
        let mut world = World::new(NEST);
        world.tell(Event::WriteCaret);
        let caret = (NEST.0 - 2.0, NEST.1);
        world.advance(1, &anchors(caret));
        assert_eq!(world.draw().pos, caret);
        assert_eq!(world.rambo.run, None);
    }

    #[test]
    fn with_reduced_motion_he_is_simply_where_he_goes() {
        let fell = (60.0, 200.0);
        let mut world = dropped_at(fell);
        let still = Anchors {
            reduced_motion: true,
            leaf: None,
            ..anchors(fell)
        };
        world.advance(steps(f64::from(mascot::DAZED_MS)) + 1, &still);
        world.advance(1, &still);
        assert_eq!(world.rambo.brain, State::Home);
        assert_eq!(world.draw(), Draw::at(NEST));
    }

    fn leaf(open: f64, closing: bool) -> Leaf {
        Leaf {
            open,
            closing,
            origin: (0.0, 0.0),
            size: (390.0, 800.0),
            distance: crate::tuning::FLIP_PERSPECTIVE * 390.0,
        }
    }

    fn under(spot: Vec2, leaf: Option<Leaf>) -> Anchors {
        Anchors {
            leaf,
            ..anchors(spot)
        }
    }

    const FELL: Vec2 = (60.0, 300.0);

    #[test]
    fn a_leaf_closing_over_him_where_he_stands_on_the_page_catches_him() {
        // Out on the page: dazed after a smack, splatting, inking, or asking about a task.
        for on_the_page in [State::Dazed, State::Splatting, State::Inking, asking()] {
            let mut world = World::new(NEST);
            world.tell(Event::Grab);
            world.advance(steps(500.0), &anchors(FELL));
            world.adopt(on_the_page.clone());
            world.advance(1, &under(FELL, Some(leaf(0.7, true))));
            assert_eq!(world.rambo.brain, State::Covered, "from {on_the_page:?}");
        }
    }

    #[test]
    fn a_leaf_that_is_not_closing_over_him_does_not() {
        let caught_by = |leaf: Option<Leaf>, spot: Vec2| {
            let mut world = dropped_at(spot);
            world.advance(1, &under(spot, leaf));
            world.rambo.brain == State::Covered
        };
        assert!(caught_by(Some(leaf(0.7, true)), FELL), "closing over him");
        assert!(
            !caught_by(Some(leaf(0.7, false)), FELL),
            "opening: it is leaving him"
        );
        assert!(!caught_by(None, FELL), "lying still");
        assert!(
            !caught_by(Some(leaf(0.7, true)), (300.0, 300.0)),
            "past its free edge"
        );
        assert!(
            !caught_by(Some(leaf(0.7, true)), (60.0, 900.0)),
            "below the page"
        );
        assert!(
            !caught_by(Some(leaf(crate::tuning::COVERED_LIFT_REACH, true)), FELL),
            "the leaf falling back from his own push is not a new catch"
        );
        // At home he is in the nest, in the bar and on no page.
        let mut home = World::new(NEST);
        home.advance(1, &under(NEST, Some(leaf(0.7, true))));
        assert_eq!(home.rambo.brain, State::Home);
    }

    #[test]
    fn caught_he_pushes_the_leaf_up_a_little_and_never_turns_it() {
        use crate::flip::{Pull, Spring};
        let mut world = dropped_at(FELL);
        assert_eq!(world.leaf_push(), None, "no push while he is free");
        world.advance(1, &under(FELL, Some(leaf(0.7, true))));
        let push = world.leaf_push().expect("pushing while caught");
        assert!(
            push.reach < 0.5,
            "a corner lifted, not a turn: {}",
            push.reach
        );
        let pull = Pull {
            omega: crate::tuning::FLIP_FALL_OMEGA,
            zeta: crate::tuning::FLIP_FALL_DAMPING,
        };
        let mut open = Spring::new(crate::tuning::COVERED_LIFT_REACH / 3.0, 0.0);
        for _ in 0..240 {
            open.step_with_push(0.0, pull, Some(push), 1.0 / 60.0);
        }
        assert!(
            open.x > 0.05 && open.x <= push.reach + 1e-9,
            "lifted a little, never past the reach: {}",
            open.x
        );
    }

    #[test]
    fn he_pushes_his_way_out_after_his_time_and_hops_home_undazed() {
        let mut world = dropped_at(FELL);
        world.advance(1, &under(FELL, Some(leaf(0.7, true))));
        assert_eq!(world.rambo.brain, State::Covered);
        // The leaf now lies on him, pushed up a little: still over him, no longer closing.
        let lying = under(FELL, Some(leaf(0.02, false)));
        world.advance(steps(f64::from(mascot::COVERED_MS)) - 2, &lying);
        assert_eq!(
            world.rambo.brain,
            State::Covered,
            "still stuck just before the time"
        );
        let (x, y) = world.draw().pos;
        assert!(
            (x - Leaf::free_edge_x(&leaf(0.02, false))).abs() < 2.0 && y == FELL.1,
            "he has worked his way to the leaf's free corner, level with where he stood: {x}"
        );
        world.advance(2, &lying);
        assert_eq!(
            world.rambo.brain,
            State::Home,
            "out, not dazed and not skittering"
        );
        assert!(!world.at_rest(), "he is on his way: hopping to the nest");
        assert_eq!(world.leaf_push(), None);
        // The leaf falling flat behind him cannot catch him again, and he hops to the nest
        // and settles in it.
        world.advance(1, &under(FELL, Some(leaf(0.1, true))));
        assert_eq!(world.rambo.brain, State::Home);
        let falling = under(FELL, Some(leaf(0.0, false)));
        world.advance(steps(f64::from(crate::tuning::FLY_MS) + 400.0), &falling);
        assert_eq!(world.draw().pos, in_nest(&world, NEST));
        world.advance(steps(SETTLES_MS), &falling);
        assert!(world.at_rest());
    }

    #[test]
    fn if_the_leaf_lifts_away_he_is_out_at_once() {
        let mut world = dropped_at(FELL);
        world.advance(1, &under(FELL, Some(leaf(0.7, true))));
        assert_eq!(world.rambo.brain, State::Covered);
        world.advance(1, &under(FELL, Some(leaf(0.95, false))));
        assert_eq!(world.rambo.brain, State::Home, "he hops home, undazed");
        // No leaf shown at all, or one lying flat over him, does not let him go.
        let mut stuck = dropped_at(FELL);
        stuck.advance(1, &under(FELL, Some(leaf(0.7, true))));
        stuck.advance(steps(500.0), &under(FELL, None));
        assert_eq!(stuck.rambo.brain, State::Covered);
    }

    #[test]
    fn caught_while_inking_he_stays_level_with_where_he_was_caught_not_with_the_finger() {
        let mut world = dropped_at(FELL);
        world.tell(Event::Grab);
        assert_eq!(world.rambo.brain, State::Inking);
        world.advance(steps(300.0), &anchors((60.0, 400.0)));
        world.advance(1, &under((60.0, 400.0), Some(leaf(0.7, true))));
        assert_eq!(world.rambo.brain, State::Covered);
        let caught_y = world.rambo.pos.1;
        // The finger carries on down the page while he is stuck under the leaf.
        for y in [450.0, 520.0, 600.0] {
            world.advance(steps(200.0), &under((60.0, y), Some(leaf(0.02, false))));
            assert!(
                (world.rambo.pos.1 - caught_y).abs() < 1e-6,
                "level with where he was caught"
            );
        }
    }
}
