//! What a finger on the page means: one recogniser for every surface (VISION-AUDIT
//! item 3, ENGINE-PLAN §5).
//!
//! The page answers the same few gestures everywhere: a tap, a hold, a drag and a
//! fling. Each surface used to tell them apart for itself, with its own slop and
//! its own hold time, so a finger behaved one way on a row and another on a slip.
//! Here pointer samples go in and gestures come out, with one slop
//! ([`GESTURE_SLOP_PX`]) and one hold time ([`GESTURE_HOLD_MS`]). Who a gesture
//! belongs to is decided by whoever routes it (the target found at `Down`, and
//! first of all Rambo: [`crate::mascot::wants`]).
//!
//! It is pure: times are the pointer events' `timeStamp` in ms (the clock the
//! page's frames run on) and speeds are px per ms. Each sample names its finger
//! (the event's `pointerId`); only the finger that went down first counts until it
//! lifts. A finger that rests without moving sends no events, so a caller that
//! needs holds on time feeds a [`Pointer::Tick`] each frame while a finger is down
//! (the engine's tick, ENGINE-PLAN §1). Without ticks a hold is only noticed on
//! the next move, so it can read Hold then Drag where a ticked finger reads Drag.
//! Rambo's drop, the one caller today, reads only `Fling` and sends no ticks.
//!
//! The decisions are the ones `row::advance` made for a row, for everyone: a hold
//! is decided by time before travel, and a decided press stays decided.

use crate::tuning::{
    GESTURE_FLING_MIN_SPEED, GESTURE_HOLD_MS, GESTURE_RELEASE_MAX_SPEED, GESTURE_RELEASE_REST_MS,
    GESTURE_RELEASE_SAMPLE_MS, GESTURE_SLOP_PX,
};

/// A point on the page, or a velocity in px per ms.
pub type Vec2 = (f64, f64);

/// Which page of the book a finger is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leaf {
    List,
    Planner,
}

/// What was under the finger when it went down, found by hit test then and kept
/// for the whole gesture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Rambo,
    /// A task row on the list, by task id.
    Row(String),
    /// A task's slip on the planner, by task id.
    Slip(String),
    /// A done or deleted task's strip on the pin, by task id.
    Strip(String),
    /// Empty paper on a page.
    Page(Leaf),
    /// The planner's hour grid, between slips.
    Grid,
}

/// What the pointer did.
#[derive(Clone, Debug, PartialEq)]
pub enum Pointer {
    Down(Target),
    Move,
    Up,
    /// The browser took the pointer away (a scroll, a system gesture).
    Cancel,
    /// A frame passed with the finger down and still.
    Tick,
}

/// One pointer event, or one frame while a finger is down.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub pointer: Pointer,
    pub at: Vec2,
    pub t: f64,
    /// Which finger (the event's `pointerId`).
    pub id: i32,
}

/// What the finger means, as soon as it is known.
#[derive(Clone, Debug, PartialEq)]
pub enum Gesture {
    /// A finger went down on `target` at `at`. Nothing is decided yet.
    Down { target: Target, at: Vec2 },
    /// Let go quickly, without travelling.
    Tap,
    /// Held still for [`GESTURE_HOLD_MS`]. Moving on from here drags what was held.
    Hold,
    /// Moving, past the slop: the finger is at `pos`, going at `vel`.
    Drag { pos: Vec2, vel: Vec2 },
    /// Let go while moving fast: whatever it carried is thrown at `vel`.
    Fling { vel: Vec2 },
    /// Let go after a hold or a drag, without a throw: whatever it carried is put
    /// down at `pos`.
    Drop { pos: Vec2 },
    /// The gesture ended without meaning anything: undo whatever it started.
    Cancel,
}

/// How far a press has got.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Idle,
    /// Down, still inside the slop, not yet held.
    Pressed {
        from: Vec2,
        since: f64,
    },
    /// Held still long enough; still inside the slop.
    Held {
        from: Vec2,
    },
    Dragging,
}

/// Turns one finger's samples into gestures. Every sample from another finger
/// while one is down is ignored, so a second finger never ends, moves or speeds up
/// the first one's gesture.
#[derive(Clone, Debug)]
pub struct Recognizer {
    phase: Phase,
    /// The finger that is down.
    finger: i32,
    /// The finger's recent path (t, x, y), for its speed.
    recent: Vec<(f64, f64, f64)>,
}

impl Default for Recognizer {
    fn default() -> Self {
        Recognizer {
            phase: Phase::Idle,
            finger: 0,
            recent: Vec::new(),
        }
    }
}

fn outside_slop(from: Vec2, at: Vec2) -> bool {
    (at.0 - from.0).hypot(at.1 - from.1) >= GESTURE_SLOP_PX
}

impl Recognizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a finger is down.
    pub fn active(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// The finger whose gesture this is, while one is down.
    pub fn finger(&self) -> Option<i32> {
        self.active().then_some(self.finger)
    }

    /// One step: the gesture `sample` decides, if it decides one.
    pub fn advance(&mut self, sample: Sample) -> Option<Gesture> {
        let Sample { pointer, at, t, id } = sample;
        if self.active() && id != self.finger {
            return None;
        }
        if self.active() && pointer == Pointer::Move {
            self.remember(at, t);
        }
        match (self.phase, pointer) {
            (Phase::Idle, Pointer::Down(target)) => {
                self.phase = Phase::Pressed { from: at, since: t };
                self.finger = id;
                self.recent = vec![(t, at.0, at.1)];
                Some(Gesture::Down { target, at })
            }
            (Phase::Idle, _) | (_, Pointer::Down(_)) => None,
            (_, Pointer::Cancel) => {
                self.phase = Phase::Idle;
                Some(Gesture::Cancel)
            }
            (Phase::Pressed { from, since }, Pointer::Move | Pointer::Tick)
                if t - since >= GESTURE_HOLD_MS =>
            {
                self.phase = Phase::Held { from };
                Some(Gesture::Hold)
            }
            (Phase::Pressed { from, .. } | Phase::Held { from }, Pointer::Move)
                if outside_slop(from, at) =>
            {
                self.phase = Phase::Dragging;
                Some(self.drag(at, t))
            }
            (Phase::Dragging, Pointer::Move) => Some(self.drag(at, t)),
            (phase, Pointer::Up) => {
                self.phase = Phase::Idle;
                Some(match phase {
                    Phase::Pressed { since, .. } if t - since < GESTURE_HOLD_MS => Gesture::Tap,
                    Phase::Dragging => match release_velocity(&self.recent, t) {
                        vel if flung(vel) => Gesture::Fling { vel },
                        _ => Gesture::Drop { pos: at },
                    },
                    _ => Gesture::Drop { pos: at },
                })
            }
            _ => None,
        }
    }

    fn drag(&self, pos: Vec2, t: f64) -> Gesture {
        Gesture::Drag {
            pos,
            vel: release_velocity(&self.recent, t),
        }
    }

    /// Keeps the finger's moves over the last [`GESTURE_RELEASE_SAMPLE_MS`]: all its
    /// speed is ever read from.
    fn remember(&mut self, at: Vec2, t: f64) {
        self.recent.push((t, at.0, at.1));
        self.recent
            .retain(|&(when, ..)| t - when <= GESTURE_RELEASE_SAMPLE_MS);
    }
}

/// How fast the finger was going at `now`: its travel over the last
/// [`GESTURE_RELEASE_SAMPLE_MS`] of `samples` (its moves: t in ms, x, y), capped
/// at [`GESTURE_RELEASE_MAX_SPEED`]. A finger whose last move is more than
/// [`GESTURE_RELEASE_REST_MS`] old has stopped, and throws nothing however fast it
/// went before. The one throw detector on the page.
pub fn release_velocity(samples: &[(f64, f64, f64)], now: f64) -> Vec2 {
    if samples
        .last()
        .is_none_or(|&(t, ..)| now - t > GESTURE_RELEASE_REST_MS)
    {
        return (0.0, 0.0);
    }
    let mut recent = samples
        .iter()
        .filter(|(t, ..)| now - t <= GESTURE_RELEASE_SAMPLE_MS);
    let (Some(first), Some(last)) = (recent.next(), recent.next_back()) else {
        return (0.0, 0.0);
    };
    let dt = last.0 - first.0;
    if dt <= 0.0 {
        return (0.0, 0.0);
    }
    let vel = ((last.1 - first.1) / dt, (last.2 - first.2) / dt);
    let speed = vel.0.hypot(vel.1);
    let cap = (GESTURE_RELEASE_MAX_SPEED / speed).min(1.0);
    (vel.0 * cap, vel.1 * cap)
}

/// Whether letting go at `vel` throws what the finger carried; slower, it is put
/// down where it is.
pub fn flung(vel: Vec2) -> bool {
    vel.0.hypot(vel.1) >= GESTURE_FLING_MIN_SPEED
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(samples: impl IntoIterator<Item = (Pointer, Vec2, f64)>) -> Vec<Gesture> {
        fingers(
            samples
                .into_iter()
                .map(|(pointer, at, t)| (1, pointer, at, t)),
        )
    }

    /// Samples from more than one finger: (id, pointer, at, t).
    fn fingers(samples: impl IntoIterator<Item = (i32, Pointer, Vec2, f64)>) -> Vec<Gesture> {
        let mut recognizer = Recognizer::new();
        samples
            .into_iter()
            .filter_map(|(id, pointer, at, t)| recognizer.advance(Sample { pointer, at, t, id }))
            .collect()
    }

    fn down(at: Vec2) -> (Pointer, Vec2, f64) {
        (Pointer::Down(Target::Rambo), at, 0.0)
    }

    fn down_on_rambo() -> Gesture {
        Gesture::Down {
            target: Target::Rambo,
            at: (100.0, 100.0),
        }
    }

    /// Frames of a finger resting at `at` from `from` to `to` ms.
    fn resting(at: Vec2, from: f64, to: f64) -> Vec<(Pointer, Vec2, f64)> {
        let frames = ((to - from) / 16.0) as usize;
        (1..=frames)
            .map(|i| (Pointer::Tick, at, from + i as f64 * 16.0))
            .collect()
    }

    #[test]
    fn a_quick_press_without_travel_is_a_tap() {
        let at = (100.0, 100.0);
        let shaky = (103.0, 102.0);
        let gestures = feed([
            down(at),
            (Pointer::Move, shaky, 40.0),
            (Pointer::Up, shaky, 90.0),
        ]);
        assert_eq!(gestures, [down_on_rambo(), Gesture::Tap]);
    }

    #[test]
    fn a_still_finger_holds_on_time_without_moving() {
        let at = (100.0, 100.0);
        let mut samples = vec![down(at)];
        samples.extend(resting(at, 0.0, GESTURE_HOLD_MS + 32.0));
        samples.push((Pointer::Up, at, GESTURE_HOLD_MS + 40.0));
        assert_eq!(
            feed(samples),
            [down_on_rambo(), Gesture::Hold, Gesture::Drop { pos: at }]
        );
    }

    #[test]
    fn a_hold_decides_by_time_before_travel() {
        // A late first move: the hold came first, as in `row::advance`.
        let gestures = feed([
            down((100.0, 100.0)),
            (Pointer::Move, (160.0, 100.0), GESTURE_HOLD_MS + 5.0),
            (Pointer::Move, (170.0, 100.0), GESTURE_HOLD_MS + 21.0),
        ]);
        assert_eq!(gestures[1], Gesture::Hold);
        assert!(matches!(
            gestures[2],
            Gesture::Drag {
                pos: (170.0, 100.0),
                ..
            }
        ));
    }

    #[test]
    fn leaving_the_slop_is_a_drag_that_follows_the_finger() {
        let gestures = feed([
            down((100.0, 100.0)),
            (Pointer::Move, (100.0 + GESTURE_SLOP_PX * 0.5, 100.0), 16.0),
            (Pointer::Move, (100.0 + GESTURE_SLOP_PX, 100.0), 32.0),
            (Pointer::Move, (140.0, 100.0), 48.0),
        ]);
        let drags: Vec<Vec2> = gestures
            .iter()
            .filter_map(|g| match g {
                Gesture::Drag { pos, .. } => Some(*pos),
                _ => None,
            })
            .collect();
        assert_eq!(drags, [(100.0 + GESTURE_SLOP_PX, 100.0), (140.0, 100.0)]);
        // Once dragging, it never becomes a hold, however long it lasts.
        let mut long = vec![down((0.0, 0.0)), (Pointer::Move, (50.0, 0.0), 16.0)];
        long.extend(resting((50.0, 0.0), 16.0, GESTURE_HOLD_MS * 3.0));
        assert!(!feed(long).contains(&Gesture::Hold));
    }

    #[test]
    fn a_drag_knows_how_fast_the_finger_goes() {
        let gestures = feed((0..10).map(|i| match i {
            0 => down((0.0, 0.0)),
            _ => (
                Pointer::Move,
                (f64::from(i) * 16.0, 0.0),
                f64::from(i) * 16.0,
            ),
        }));
        let Some(Gesture::Drag { vel, .. }) = gestures.last() else {
            panic!("{gestures:?}");
        };
        assert!((vel.0 - 1.0).abs() < 1e-9 && vel.1 == 0.0, "{vel:?}");
    }

    #[test]
    fn letting_go_mid_flick_throws_and_letting_go_still_drops() {
        let flick: Vec<_> = (0..=6)
            .map(|i| match i {
                0 => down((0.0, 0.0)),
                _ => (
                    Pointer::Move,
                    (f64::from(i) * 32.0, 0.0),
                    f64::from(i) * 16.0,
                ),
            })
            .collect();
        let mut thrown = flick.clone();
        thrown.push((Pointer::Up, (192.0, 0.0), 96.0));
        let Some(Gesture::Fling { vel }) = feed(thrown).last().cloned() else {
            panic!("a fling");
        };
        assert!((vel.0 - 2.0).abs() < 1e-9, "{vel:?}");

        // The same flick, then the finger rests before letting go: put down.
        let mut stopped = flick;
        stopped.extend(resting(
            (192.0, 0.0),
            96.0,
            96.0 + GESTURE_RELEASE_SAMPLE_MS * 2.0,
        ));
        stopped.push((Pointer::Up, (192.0, 0.0), 300.0));
        assert_eq!(
            feed(stopped).last(),
            Some(&Gesture::Drop { pos: (192.0, 0.0) })
        );
    }

    #[test]
    fn a_cancel_ends_any_gesture_and_the_next_starts_fresh() {
        let at = (10.0, 10.0);
        let gestures = feed([
            down(at),
            (Pointer::Move, (60.0, 10.0), 16.0),
            (Pointer::Cancel, (60.0, 10.0), 32.0),
            (Pointer::Up, (60.0, 10.0), 40.0),
            (Pointer::Down(Target::Grid), at, 100.0),
            (Pointer::Up, at, 120.0),
        ]);
        assert_eq!(
            gestures[2..],
            [
                Gesture::Cancel,
                Gesture::Down {
                    target: Target::Grid,
                    at
                },
                Gesture::Tap
            ]
        );
    }

    #[test]
    fn a_second_finger_is_ignored() {
        let gestures = fingers([
            (1, Pointer::Down(Target::Rambo), (0.0, 0.0), 0.0),
            (1, Pointer::Move, (40.0, 0.0), 16.0),
            (
                2,
                Pointer::Down(Target::Page(Leaf::List)),
                (300.0, 300.0),
                20.0,
            ),
            (2, Pointer::Move, (400.0, 300.0), 24.0),
            (2, Pointer::Up, (400.0, 300.0), 28.0),
            (1, Pointer::Move, (50.0, 0.0), 300.0),
        ]);
        // The second finger's moves neither drag nor speed up the first, and its
        // lifting ends nothing: the first is still dragging, slowly.
        let Some(Gesture::Drag { pos, vel }) = gestures.last() else {
            panic!("{gestures:?}");
        };
        assert_eq!(*pos, (50.0, 0.0));
        assert!(!flung(*vel), "{vel:?}");
        assert_eq!(gestures.len(), 3, "{gestures:?}");
        let mut recognizer = Recognizer::new();
        recognizer.advance(Sample {
            pointer: Pointer::Down(Target::Rambo),
            at: (0.0, 0.0),
            t: 0.0,
            id: 7,
        });
        assert_eq!(recognizer.finger(), Some(7));
    }

    #[test]
    fn a_flick_that_rests_before_letting_go_is_put_down() {
        // 2 px per ms, then the finger stops and lets go a moment later: shorter
        // than the window the speed is read over, longer than a pointer's frame.
        let flick = |rest: f64| {
            let mut samples: Vec<_> = (0..=6)
                .map(|i| match i {
                    0 => down((0.0, 0.0)),
                    _ => (
                        Pointer::Move,
                        (f64::from(i) * 32.0, 0.0),
                        f64::from(i) * 16.0,
                    ),
                })
                .collect();
            samples.push((Pointer::Up, (192.0, 0.0), 96.0 + rest));
            feed(samples).last().cloned()
        };
        const { assert!(GESTURE_RELEASE_REST_MS * 1.5 < GESTURE_RELEASE_SAMPLE_MS) };
        assert_eq!(
            flick(GESTURE_RELEASE_REST_MS * 1.5),
            Some(Gesture::Drop { pos: (192.0, 0.0) })
        );
        assert!(matches!(flick(8.0), Some(Gesture::Fling { .. })));
    }

    #[test]
    fn nothing_happens_without_a_finger_down() {
        assert!(
            feed([
                (Pointer::Move, (5.0, 5.0), 0.0),
                (Pointer::Up, (5.0, 5.0), 9.0)
            ])
            .is_empty()
        );
    }

    #[test]
    fn the_release_speed_is_read_from_the_last_moments() {
        // Slow for a long while, then 2 px per ms across the last 60 ms.
        let samples = [
            (0.0, 0.0, 0.0),
            (200.0, 10.0, 0.0),
            (240.0, 50.0, 0.0),
            (300.0, 170.0, 0.0),
        ];
        let (vx, vy) = release_velocity(&samples, 300.0);
        assert!((vx - 2.0).abs() < 1e-9 && vy == 0.0, "{vx}, {vy}");
        assert_eq!(
            release_velocity(&samples, 300.0 + GESTURE_RELEASE_SAMPLE_MS + 1.0),
            (0.0, 0.0)
        );
        assert_eq!(release_velocity(&[], 0.0), (0.0, 0.0));
        let wild = release_velocity(&[(0.0, 0.0, 0.0), (10.0, 0.0, 500.0)], 10.0);
        assert!((wild.0.hypot(wild.1) - GESTURE_RELEASE_MAX_SPEED).abs() < 1e-9);
    }

    #[test]
    fn a_gentle_letting_go_is_a_drop_not_a_throw() {
        assert!(!flung((GESTURE_FLING_MIN_SPEED * 0.9, 0.0)));
        assert!(flung((0.0, GESTURE_FLING_MIN_SPEED)));
    }
}
