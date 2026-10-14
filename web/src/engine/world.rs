//! Rambo as an entity on the engine (ENGINE-PLAN step 2): where he is, how he gets
//! where his brain wants him, and the brain events his own movement and timers raise.
//!
//! Pure: once a frame the page reads its anchors (where the nest is, where his spot
//! is), steps the world by the engine's fixed steps and writes one [`Draw`] to his
//! element. The brain is [`mascot::transition`], unchanged; this decides only how his
//! body follows it, so behaviours end on state, never on a guessed CSS duration.

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
}

/// Everything the engine moves. Today that is Rambo; the nest, ink and the turn join it
/// in later steps.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct World {
    pub(crate) rambo: Rambo,
    /// World time in ms: the sum of the fixed steps taken.
    now: f64,
}

impl World {
    /// The world at load: Rambo at rest in his nest at `home`.
    pub(crate) fn new(home: Vec2) -> Self {
        World {
            rambo: Rambo {
                brain: State::Home,
                pos: home,
                vel: (0.0, 0.0),
                since: 0.0,
                run: None,
                owed: None,
                settling: false,
                settled_home: true,
                draw: Draw::at(home),
            },
            now: 0.0,
        }
    }

    /// How he is drawn now.
    pub(crate) fn draw(&self) -> Draw {
        self.rambo.draw
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
        rambo.owed = mascot::notice(&rambo.brain, &next);
        rambo.settling = rambo.brain == State::Flying && next == State::WritingRow;
        rambo.brain = next;
        rambo.since = self.now;
        rambo.settled_home = false;
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
            State::Splatting | State::Dazed | State::Covered
        );
        !timed
            && rambo.brain != State::Dragging
            && !mascot::hops_there(&rambo.brain, rambo.settled_home)
            && !rambo.settling
            && rambo.run.is_none()
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
        let frame = run.hop.at(to, elapsed);
        rambo.vel = velocity(rambo.pos, frame.pos);
        rambo.pos = frame.pos;
        rambo.draw = Draw {
            pos: (frame.pos.0, frame.pos.1 - frame.lift),
            squash: frame.squash,
            notice: None,
        };
    }

    /// He has arrived at `at`: still, unsquashed, and home if he was on his way home.
    fn land(&mut self, at: Vec2) {
        let rambo = &mut self.rambo;
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
    }
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

    fn anchors(spot: Vec2) -> Anchors {
        Anchors {
            nest: NEST,
            spot,
            reduced_motion: false,
            leaf: None,
        }
    }

    /// Steps for `ms` of world time.
    fn steps(ms: f64) -> u32 {
        (ms / STEP_MS).ceil() as u32
    }

    fn dropped_at(spot: Vec2) -> World {
        let mut world = World::new(NEST);
        world.tell(Event::Grab);
        world.advance(steps(500.0), &anchors(spot));
        world.tell(Event::DropEmpty);
        world.advance(1, &anchors(spot));
        world
    }

    #[test]
    fn settled_at_home_he_moves_with_the_nest_exactly_and_never_hops() {
        let mut world = World::new(NEST);
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
        assert_eq!(world.draw(), Draw::at(NEST));
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
        assert!(world.at_rest(), "on his spot at once, the run gone");
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
        assert_eq!(world.draw(), Draw::at(NEST));
        assert!(world.at_rest());
    }

    #[test]
    fn a_splat_settles_into_a_daze_where_he_fell() {
        let mut world = World::new(NEST);
        world.tell(Event::Grab);
        world.tell(Event::DropHard);
        assert_eq!(world.rambo.brain, State::Splatting);
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
        assert_eq!(world.draw().pos, moved.nest);
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
        for brain_to_covered in [Event::DropEmpty, Event::DropHard] {
            let mut world = World::new(NEST);
            world.tell(Event::Grab);
            world.advance(steps(500.0), &anchors(FELL));
            world.tell(brain_to_covered);
            world.advance(1, &under(FELL, Some(leaf(0.7, true))));
            assert_eq!(world.rambo.brain, State::Covered);
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
        assert_eq!(world.draw().pos, NEST);
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
