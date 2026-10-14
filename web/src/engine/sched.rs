//! The engine's pure core: which jobs run on a frame, what time they see, and when the
//! loop must wake next. Nothing here touches the browser, so it is tested natively;
//! [`super::run`] is the glue that asks the browser for the frames and timers.
// Natively nothing pumps the loop but these tests.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The longest frame a job is told about, in seconds: after a hidden tab or a stall, time
/// picks up where it left off instead of leaping.
const MAX_FRAME_S: f64 = 0.1;

/// The engine's fixed step, in seconds: physics steps by exactly this, however fast the
/// screen draws, so a throw goes the same on a 60 Hz phone and a 144 Hz monitor.
pub(crate) const ENGINE_DT_S: f64 = 1.0 / 120.0;

/// The most fixed steps one frame takes. A frame later than that drops the rest of the
/// backlog instead of stepping in a burst.
const MAX_STEPS: u32 = 8;

/// What a job sees on a frame: the loop's time in ms (the browser's animation clock,
/// never the wall clock), and how many fixed [`ENGINE_DT_S`] steps the time since the
/// last frame makes, with what was left over from earlier frames counted in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tick {
    pub(crate) now: f64,
    pub(crate) steps: u32,
}

/// Whether a job wants to run again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flow {
    Go,
    Stop,
}

/// What the loop needs next: a frame, a wake at a time in ms, or nothing at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Wake {
    Frame,
    At(f64),
    Sleep,
}

/// When a job runs.
enum When {
    /// On every frame until it stops.
    EveryFrame,
    /// Once `due` has come; again `every` ms later if it repeats, or never.
    At { due: Cell<f64>, every: Option<f64> },
}

pub(crate) struct Job {
    live: Rc<Cell<bool>>,
    when: When,
    body: RefCell<Box<dyn FnMut(Tick) -> Flow>>,
}

/// A job's handle. `stop` ends it from anywhere, from inside the job itself too, and it
/// never keeps the job's closure alive.
#[derive(Clone, Debug)]
pub(crate) struct Handle(Rc<Cell<bool>>);

impl Handle {
    pub(crate) fn stop(&self) {
        self.0.set(false);
    }
}

#[derive(Default)]
pub(crate) struct Loop {
    jobs: Vec<Rc<Job>>,
    /// The last frame's time, while frames follow one another; none once the loop has
    /// slept, so the first frame after it is a frame of no time.
    last: Option<f64>,
    /// Time not yet stepped, in seconds, always less than one fixed step.
    left_over: f64,
}

impl Loop {
    /// A job run on every frame until it returns [`Flow::Stop`].
    pub(crate) fn every_frame(&mut self, body: impl FnMut(Tick) -> Flow + 'static) -> Handle {
        self.add(When::EveryFrame, Box::new(body))
    }

    /// A job run once at `due` ms, and then every `every` ms while it returns [`Flow::Go`].
    pub(crate) fn at(
        &mut self,
        due: f64,
        every: Option<f64>,
        body: impl FnMut(Tick) -> Flow + 'static,
    ) -> Handle {
        let when = When::At {
            due: Cell::new(due),
            every,
        };
        self.add(when, Box::new(body))
    }

    fn add(&mut self, when: When, body: Box<dyn FnMut(Tick) -> Flow>) -> Handle {
        let live = Rc::new(Cell::new(true));
        self.jobs.push(Rc::new(Job {
            live: live.clone(),
            when,
            body: RefCell::new(body),
        }));
        Handle(live)
    }

    /// Opens a frame at `now`: the tick its jobs see and the jobs that are due. They are
    /// [`run`] with the loop let go, since a job may add or stop jobs; one added meanwhile
    /// first runs on the next frame.
    pub(crate) fn begin(&mut self, now: f64) -> (Tick, Vec<Rc<Job>>) {
        let dt = self
            .last
            .map_or(0.0, |last| ((now - last) / 1000.0).clamp(0.0, MAX_FRAME_S));
        self.last = Some(now);
        let owed = self.left_over + dt;
        // A hair under a whole step still counts as one, so 1/120 s is one step.
        let whole = (owed / ENGINE_DT_S + 1e-9).floor();
        let steps = (whole as u32).min(MAX_STEPS);
        self.left_over = if steps == MAX_STEPS {
            0.0
        } else {
            (owed - whole * ENGINE_DT_S).max(0.0)
        };
        let due = self
            .jobs
            .iter()
            .filter(|job| job.live.get() && job.is_due(now))
            .cloned()
            .collect();
        (Tick { now, steps }, due)
    }

    /// Closes a frame: stopped jobs go, and the loop says what it needs next.
    pub(crate) fn end(&mut self) -> Wake {
        self.jobs.retain(|job| job.live.get());
        let wake = self.wake();
        if wake != Wake::Frame {
            self.last = None;
            self.left_over = 0.0;
        }
        wake
    }

    /// A frame while any job runs every frame, else a wake at the earliest timer.
    pub(crate) fn wake(&self) -> Wake {
        let live = || self.jobs.iter().filter(|job| job.live.get());
        if live().any(|job| matches!(job.when, When::EveryFrame)) {
            return Wake::Frame;
        }
        live()
            .filter_map(|job| match &job.when {
                When::At { due, .. } => Some(due.get()),
                When::EveryFrame => None,
            })
            .min_by(f64::total_cmp)
            .map_or(Wake::Sleep, Wake::At)
    }
}

impl Job {
    fn is_due(&self, now: f64) -> bool {
        match &self.when {
            When::EveryFrame => true,
            When::At { due, .. } => due.get() <= now,
        }
    }
}

/// Runs a frame's jobs, in the order they were added. A job stopped by one before it in
/// the same frame does not run.
pub(crate) fn run(tick: Tick, jobs: &[Rc<Job>]) {
    for job in jobs {
        if !job.live.get() {
            continue;
        }
        let flow = (job.body.borrow_mut())(tick);
        match (&job.when, flow) {
            (_, Flow::Stop) | (When::At { every: None, .. }, _) => job.live.set(false),
            // A repeat that fell behind (a hidden tab) runs once and picks up from now,
            // never in a burst.
            (
                When::At {
                    due,
                    every: Some(every),
                },
                Flow::Go,
            ) => due.set((due.get() + every).max(tick.now + every)),
            (When::EveryFrame, Flow::Go) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame at `now`, as the browser glue runs it.
    fn frame(lp: &RefCell<Loop>, now: f64) -> Wake {
        let (tick, jobs) = lp.borrow_mut().begin(now);
        run(tick, &jobs);
        lp.borrow_mut().end()
    }

    fn log() -> Rc<RefCell<Vec<Tick>>> {
        Rc::default()
    }

    #[test]
    fn a_frame_job_sees_the_steps_between_frames_and_none_on_the_first() {
        let lp = RefCell::new(Loop::default());
        let seen = log();
        let s = seen.clone();
        lp.borrow_mut().every_frame(move |tick| {
            s.borrow_mut().push(tick);
            Flow::Go
        });
        assert_eq!(frame(&lp, 1000.0), Wake::Frame);
        assert_eq!(frame(&lp, 1016.0), Wake::Frame);
        assert_eq!(
            *seen.borrow(),
            [
                Tick {
                    now: 1000.0,
                    steps: 0
                },
                Tick {
                    now: 1016.0,
                    steps: 1
                }
            ]
        );
    }

    /// The fixed steps a frame job is given over frames at `times` ms.
    fn steps_over(times: &[f64]) -> Vec<u32> {
        let lp = RefCell::new(Loop::default());
        let seen = log();
        let s = seen.clone();
        lp.borrow_mut().every_frame(move |tick| {
            s.borrow_mut().push(tick);
            Flow::Go
        });
        for &t in times {
            frame(&lp, t);
        }
        seen.borrow().iter().map(|tick| tick.steps).collect()
    }

    #[test]
    fn fixed_steps_keep_pace_with_time_at_any_frame_rate() {
        // A second of 60 Hz frames and of 144 Hz frames is 120 steps either way.
        for hz in [60.0, 144.0] {
            let times: Vec<f64> = (0..=hz as usize).map(|i| i as f64 * 1000.0 / hz).collect();
            let total: u32 = steps_over(&times).iter().sum();
            assert!((119..=120).contains(&total), "{hz} Hz: {total}");
        }
        // A 60 Hz frame is two steps; none is lost.
        assert_eq!(
            steps_over(&[0.0, 1000.0 / 60.0, 2000.0 / 60.0, 50.0]),
            [0, 2, 2, 2]
        );
    }

    #[test]
    fn a_late_frame_takes_at_most_a_few_steps_and_drops_the_rest() {
        assert_eq!(
            steps_over(&[0.0, 500.0, 500.0 + 1000.0 / 120.0]),
            [0, MAX_STEPS, 1]
        );
    }

    #[test]
    fn after_a_stall_time_picks_up_and_does_not_leap() {
        let lp = RefCell::new(Loop::default());
        let seen = log();
        let s = seen.clone();
        lp.borrow_mut().every_frame(move |tick| {
            s.borrow_mut().push(tick);
            Flow::Go
        });
        frame(&lp, 0.0);
        frame(&lp, 30_000.0);
        assert_eq!(seen.borrow()[1].steps, MAX_STEPS);
    }

    #[test]
    fn the_loop_sleeps_once_the_last_job_stops_and_wakes_with_no_time_gone() {
        let lp = RefCell::new(Loop::default());
        let runs = Rc::new(Cell::new(0));
        let r = runs.clone();
        lp.borrow_mut().every_frame(move |_| {
            r.set(r.get() + 1);
            if r.get() == 2 { Flow::Stop } else { Flow::Go }
        });
        assert_eq!(frame(&lp, 0.0), Wake::Frame);
        assert_eq!(frame(&lp, 16.0), Wake::Sleep);
        assert_eq!(frame(&lp, 32.0), Wake::Sleep);
        assert_eq!(runs.get(), 2);
        let seen = log();
        let s = seen.clone();
        lp.borrow_mut().every_frame(move |tick| {
            s.borrow_mut().push(tick);
            Flow::Stop
        });
        frame(&lp, 5000.0);
        assert_eq!(seen.borrow()[0].steps, 0);
    }

    #[test]
    fn a_timer_runs_once_when_due_and_the_loop_waits_for_it_without_frames() {
        let lp = RefCell::new(Loop::default());
        let runs = Rc::new(Cell::new(0));
        let r = runs.clone();
        lp.borrow_mut().at(500.0, None, move |_| {
            r.set(r.get() + 1);
            Flow::Go
        });
        assert_eq!(lp.borrow().wake(), Wake::At(500.0));
        assert_eq!(frame(&lp, 499.0), Wake::At(500.0));
        assert_eq!(runs.get(), 0);
        assert_eq!(frame(&lp, 500.0), Wake::Sleep);
        assert_eq!(frame(&lp, 900.0), Wake::Sleep);
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn a_repeat_that_fell_behind_runs_once_and_picks_up_from_now() {
        let lp = RefCell::new(Loop::default());
        let runs = Rc::new(Cell::new(0));
        let r = runs.clone();
        lp.borrow_mut().at(100.0, Some(100.0), move |_| {
            r.set(r.get() + 1);
            Flow::Go
        });
        assert_eq!(frame(&lp, 100.0), Wake::At(200.0));
        assert_eq!(frame(&lp, 1050.0), Wake::At(1150.0));
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn the_loop_wakes_for_the_earliest_timer() {
        let mut lp = Loop::default();
        lp.at(900.0, None, |_| Flow::Stop);
        lp.at(300.0, Some(50.0), |_| Flow::Go);
        assert_eq!(lp.wake(), Wake::At(300.0));
    }

    #[test]
    fn a_stopped_job_never_runs_even_if_stopped_earlier_in_the_same_frame() {
        let lp = RefCell::new(Loop::default());
        let ran = Rc::new(Cell::new(false));
        let later: Rc<RefCell<Option<Handle>>> = Rc::default();
        let l = later.clone();
        lp.borrow_mut().every_frame(move |_| {
            if let Some(handle) = l.borrow().as_ref() {
                handle.stop();
            }
            Flow::Go
        });
        let r = ran.clone();
        *later.borrow_mut() = Some(lp.borrow_mut().every_frame(move |_| {
            r.set(true);
            Flow::Go
        }));
        frame(&lp, 0.0);
        assert!(!ran.get());
    }

    #[test]
    fn a_job_added_during_a_frame_first_runs_on_the_next() {
        let lp = Rc::new(RefCell::new(Loop::default()));
        let seen = log();
        let (inner, s) = (lp.clone(), seen.clone());
        lp.borrow_mut().at(0.0, None, move |_| {
            let s = s.clone();
            inner.borrow_mut().every_frame(move |tick| {
                s.borrow_mut().push(tick);
                Flow::Go
            });
            Flow::Stop
        });
        assert_eq!(frame(&lp, 0.0), Wake::Frame);
        assert!(seen.borrow().is_empty());
        frame(&lp, 16.0);
        assert_eq!(seen.borrow().len(), 1);
        assert_eq!(seen.borrow()[0].steps, 1);
    }
}
