//! The page's one loop: the only code that asks the browser for animation frames or
//! timers, and the only animation clock. It keeps one [`Loop`] and pumps it: a frame
//! while any job runs every frame, one timeout for the earliest timer otherwise, and
//! nothing at rest.

#[cfg(target_arch = "wasm32")]
use super::sched::{self, Wake};
use super::sched::{Flow, Handle, Loop, Tick};
use std::cell::RefCell;

thread_local! {
    static LOOP: RefCell<Loop> = RefCell::default();
}

/// Runs `body` on every frame until it returns [`Flow::Stop`] or is stopped.
pub(crate) fn on_tick(body: impl FnMut(Tick) -> Flow + 'static) -> Handle {
    let handle = LOOP.with(|lp| lp.borrow_mut().every_frame(body));
    pump::poke();
    handle
}

/// Runs `body` once, on the next frame: after the page has laid out what changed.
pub(crate) fn next_frame(body: impl FnOnce() + 'static) {
    let mut body = Some(body);
    on_tick(move |_| {
        if let Some(body) = body.take() {
            body();
        }
        Flow::Stop
    });
}

/// Runs `body` once, `ms` from now, on the engine's clock.
pub(crate) fn after(ms: f64, body: impl FnOnce() + 'static) -> Handle {
    let mut body = Some(body);
    every(ms, move || {
        if let Some(body) = body.take() {
            body();
        }
        Flow::Stop
    })
}

/// Runs `body` every `ms` from now while it returns [`Flow::Go`].
pub(crate) fn every(ms: f64, mut body: impl FnMut() -> Flow + 'static) -> Handle {
    let handle = LOOP.with(|lp| lp.borrow_mut().at(now() + ms, Some(ms), move |_| body()));
    pump::poke();
    handle
}

/// The loop's time in ms: the browser's animation clock, the one frames are stamped
/// with and pointer events carry, never the wall clock.
#[cfg(target_arch = "wasm32")]
pub(crate) fn now() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now() -> f64 {
    0.0
}

/// One frame of the loop at `at`, with the loop let go while the jobs run.
#[cfg(target_arch = "wasm32")]
fn frame(at: f64) -> Wake {
    let (tick, jobs) = LOOP.with(|lp| lp.borrow_mut().begin(at));
    sched::run(tick, &jobs);
    drop(jobs);
    LOOP.with(|lp| lp.borrow_mut().end())
}

#[cfg(target_arch = "wasm32")]
mod pump {
    use super::{LOOP, Wake, frame, now};
    use std::cell::Cell;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    /// What the browser has been asked for.
    #[derive(Clone, Copy)]
    enum Asked {
        Nothing,
        Frame,
        Timeout { id: i32, at: f64 },
    }

    struct Pump {
        asked: Cell<Asked>,
        on_frame: Closure<dyn FnMut(f64)>,
        on_timeout: Closure<dyn FnMut()>,
    }

    thread_local! {
        static PUMP: Pump = Pump {
            asked: Cell::new(Asked::Nothing),
            on_frame: Closure::new(|at: f64| {
                PUMP.with(|pump| pump.asked.set(Asked::Nothing));
                ask(frame(at));
            }),
            on_timeout: Closure::new(|| {
                PUMP.with(|pump| pump.asked.set(Asked::Nothing));
                ask(frame(now()));
            }),
        };
    }

    /// A job was added: ask for what the loop needs now.
    pub(super) fn poke() {
        ask(LOOP.with(|lp| lp.borrow().wake()));
    }

    fn ask(wake: Wake) {
        let Some(window) = web_sys::window() else {
            return;
        };
        PUMP.with(|pump| {
            let asked = pump.asked.get();
            let next = match (asked, wake) {
                // A frame is coming, and it asks again at its end.
                (Asked::Frame, _) => return,
                (Asked::Timeout { at, .. }, Wake::At(due)) if at <= due => return,
                (_, Wake::Frame) => window
                    .request_animation_frame(pump.on_frame.as_ref().unchecked_ref())
                    .map_or(Asked::Nothing, |_| Asked::Frame),
                (_, Wake::At(due)) => window
                    .set_timeout_with_callback_and_timeout_and_arguments_0(
                        pump.on_timeout.as_ref().unchecked_ref(),
                        (due - now()).max(0.0).ceil() as i32,
                    )
                    .map_or(Asked::Nothing, |id| Asked::Timeout { id, at: due }),
                (_, Wake::Sleep) => Asked::Nothing,
            };
            if let Asked::Timeout { id, .. } = asked {
                window.clear_timeout_with_handle(id);
            }
            pump.asked.set(next);
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod pump {
    /// Natively nothing asks the browser for frames: the loop's rules are tested on
    /// [`super::Loop`] directly.
    pub(super) fn poke() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    /// One frame of the page's loop at `at`, run the way the browser's pump runs it.
    fn frame_at(at: f64) {
        let (tick, jobs) = LOOP.with(|lp| lp.borrow_mut().begin(at));
        super::super::sched::run(tick, &jobs);
        drop(jobs);
        LOOP.with(|lp| lp.borrow_mut().end());
    }

    #[test]
    fn after_runs_its_body_once_when_its_time_comes() {
        let runs = Rc::new(Cell::new(0));
        let counted = runs.clone();
        after(500.0, move || counted.set(counted.get() + 1));
        frame_at(499.0);
        assert_eq!(runs.get(), 0, "not before its time");
        frame_at(500.0);
        frame_at(1500.0);
        assert_eq!(runs.get(), 1, "and never again");
    }

    #[test]
    fn a_stopped_after_never_runs() {
        let runs = Rc::new(Cell::new(0));
        let counted = runs.clone();
        let handle = after(10.0, move || counted.set(counted.get() + 1));
        handle.stop();
        frame_at(100.0);
        assert_eq!(runs.get(), 0);
    }
}
