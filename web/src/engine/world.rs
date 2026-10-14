//! Rambo as an entity on the engine (ENGINE-PLAN step 2): where he is, how he gets
//! where his brain wants him, and the brain events his own movement and timers raise.
//!
//! Pure: once a frame the page reads its anchors (where the nest is, where his spot
//! is), steps the world by the engine's fixed steps and writes one [`Draw`] to his
//! element. The brain is [`mascot::transition`], unchanged; this decides only how his
//! body follows it, so behaviours end on state, never on a guessed CSS duration.

use super::sched::ENGINE_DT_S;
use crate::hop::Hop;
use crate::mascot::{self, Event, State};
use crate::tuning::RIDE_OMEGA;

/// A point on the page in px, or a velocity in px per second.
pub(crate) type Vec2 = (f64, f64);

/// The engine's fixed step, in ms of world time.
const STEP_MS: f64 = ENGINE_DT_S * 1000.0;

/// What the page measured this frame, in window px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Anchors {
    /// The middle of his nest.
    pub(crate) nest: Vec2,
    /// His spot: where whoever moves him wants him (above the finger while held, his
    /// nib's place while writing, where he fell).
    pub(crate) spot: Vec2,
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

/// How his body follows `state`, given what the page measured.
pub(crate) fn behaviour(state: &State, settled_home: bool, anchors: &Anchors) -> Drive {
    match state {
        State::Dragging => Drive::Ride(anchors.spot),
        State::Inking
        | State::WritingRow
        | State::Dazed
        | State::Splatting
        | State::Asking { .. } => Drive::Follow(anchors.spot),
        State::Home if settled_home => Drive::Follow(anchors.nest),
        State::Home | State::Skittering => Drive::HopTo(anchors.nest),
        State::Flying | State::WritingCaret => Drive::HopTo(anchors.spot),
    }
}

/// A run of hops under way: planned from where he set off, timed in world ms.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Run {
    hop: Hop,
    began: f64,
}

/// How he is drawn this frame: his middle raised by a hop's lift, and his squash
/// (positive squashed, negative stretched).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Draw {
    pub(crate) pos: Vec2,
    pub(crate) squash: f64,
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
                settled_home: true,
                draw: Draw {
                    pos: home,
                    squash: 0.0,
                },
            },
            now: 0.0,
        }
    }

    /// How he is drawn now.
    pub(crate) fn draw(&self) -> Draw {
        self.rambo.draw
    }

    /// An event for his brain from outside the world (a gesture, the capture box). His
    /// timers start again whenever his state changes.
    pub(crate) fn tell(&mut self, event: Event) {
        let rambo = &mut self.rambo;
        let next = mascot::transition(&rambo.brain, event);
        if next != rambo.brain {
            rambo.brain = next;
            rambo.since = self.now;
            rambo.settled_home = false;
        }
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
        if let Some(event) = self.timer_due() {
            self.tell(event);
        }
        let drive = behaviour(&self.rambo.brain, self.rambo.settled_home, anchors);
        if !matches!(drive, Drive::HopTo(_)) {
            self.rambo.run = None;
        }
        let (pos, vel, squash) = match drive {
            // Carried along by the page (a scroll, the nest moving) is not his own speed:
            // a grab right after must not fling him past the finger.
            Drive::Follow(at) => (at, (0.0, 0.0), 0.0),
            Drive::Ride(at) => {
                let (pos, vel) = ride(self.rambo.pos, self.rambo.vel, at);
                (pos, vel, 0.0)
            }
            Drive::HopTo(to) => return self.hop_towards(to),
        };
        self.rambo.pos = pos;
        self.rambo.vel = vel;
        self.rambo.draw = Draw { pos, squash };
    }

    /// The event one of his timers raises this step, if one has run out: the splat
    /// settles into a daze, and a daze left untouched sends him home.
    fn timer_due(&self) -> Option<Event> {
        let time_in_state = self.now - self.rambo.since;
        let after = match self.rambo.brain {
            State::Splatting => mascot::SPLAT_MS,
            State::Dazed => mascot::DAZED_MS,
            _ => return None,
        };
        // Steps of 1/120 s add up a hair short of whole ms.
        (time_in_state + 1e-6 >= f64::from(after)).then_some(Event::Return)
    }

    /// One step of a run of hops toward `to`, planned when the run starts. He lands on
    /// `to` as it is now, and landing in the nest after the run home makes him home.
    fn hop_towards(&mut self, to: Vec2) {
        let now = self.now;
        let rambo = &mut self.rambo;
        let run = match rambo.run {
            Some(run) => run,
            None => match Hop::plan(rambo.pos, to) {
                Some(hop) => *rambo.run.insert(Run { hop, began: now }),
                // Close enough that he simply steps there.
                None => return self.land(to),
            },
        };
        let elapsed = (now - run.began).max(0.0) as u32;
        if elapsed >= run.hop.ms() {
            return self.land(to);
        }
        let frame = run.hop.at(to, elapsed);
        rambo.vel = velocity(rambo.pos, frame.pos);
        rambo.pos = frame.pos;
        rambo.draw = Draw {
            pos: (frame.pos.0, frame.pos.1 - frame.lift),
            squash: frame.squash,
        };
    }

    /// He has arrived at `at`: still, unsquashed, and home if he was on his way home.
    fn land(&mut self, at: Vec2) {
        let rambo = &mut self.rambo;
        rambo.run = None;
        rambo.pos = at;
        rambo.vel = (0.0, 0.0);
        rambo.draw = Draw {
            pos: at,
            squash: 0.0,
        };
        if rambo.brain == State::Skittering {
            self.tell(Event::Return);
        }
        let rambo = &mut self.rambo;
        rambo.settled_home = rambo.brain == State::Home;
    }
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
        Anchors { nest: NEST, spot }
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
            spot: (0.0, 0.0),
        };
        world.advance(1, &scrolled);
        assert_eq!(world.draw().pos, scrolled.nest);
        assert_eq!(world.draw().squash, 0.0);
    }

    #[test]
    fn held_he_rides_toward_the_finger_behind_it_and_never_past_it() {
        let mut world = World::new(NEST);
        world.tell(Event::Grab);
        let spot = (100.0, 200.0);
        let mut last_gap = f64::INFINITY;
        for _ in 0..steps(400.0) {
            world.advance(1, &anchors(spot));
            let (x, y) = world.rambo.pos;
            let gap = (x - spot.0).hypot(y - spot.1);
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
            spot: (0.0, 0.0),
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
        assert_eq!(world.draw().pos, NEST);
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
            spot: fell,
        };
        world.advance(steps(f64::from(crate::tuning::FLY_MS) + 200.0), &moved);
        assert_eq!(world.rambo.brain, State::Home);
        assert_eq!(world.draw().pos, moved.nest);
    }

    #[test]
    fn a_run_cut_short_by_a_new_state_stops_hopping_and_follows_its_spot() {
        // Flying to a row, he arrives in its writing before the hops are done.
        let mut world = World::new(NEST);
        world.tell(Event::WriteRow);
        let row = (40.0, 120.0);
        world.advance(steps(100.0), &anchors(row));
        assert!(world.rambo.run.is_some(), "under way");
        world.tell(Event::WriteRow);
        assert_eq!(world.rambo.brain, State::WritingRow);
        world.advance(1, &anchors(row));
        assert_eq!(world.rambo.run, None);
        assert_eq!(
            world.draw(),
            Draw {
                pos: row,
                squash: 0.0
            }
        );
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
}
