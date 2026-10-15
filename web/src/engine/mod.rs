//! The engine: one loop and one animation clock for everything on the page that moves
//! (ARCHITECTURE "The engine"). Jobs run on its frames (`on_tick`, `next_frame`) or
//! every so often in its time (`every`); the loop sleeps when none is waiting.

mod body;
mod dance;
mod fling;
mod link;
mod look;
mod nest;
mod run;
mod sched;
mod world;

pub(crate) use body::{Pull, Spring1};
pub(crate) use fling::Walls;
pub(crate) use link::{leaf, on_leaf, set_push};
#[cfg(target_arch = "wasm32")]
pub(crate) use link::{push, show};
pub(crate) use run::{after, every, next_frame, now, on_tick, wait};
#[cfg(target_arch = "wasm32")]
pub(crate) use sched::{ENGINE_DT_S, Tick};
pub(crate) use sched::{Flow, Guard, Handle};
#[cfg(target_arch = "wasm32")]
pub(crate) use world::Leaf;
pub(crate) use world::{Anchors, Splash, Swept, World, swept};

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// Files that read the wall clock for the user's day (the planner's now line, what
    /// is overdue, the hour he hears), not to time a movement.
    const DAY_TIME: [&str; 3] = ["brain.rs", "tasks/mod.rs", "week.rs"];

    /// Files still on the browser's own timers until the scrap moves (core step 6).
    const TIMERS_PENDING: [&str; 1] = ["scrap.rs"];

    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read web/src").flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn nothing_outside_the_engine_asks_the_browser_for_frames_timers_or_the_time() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        sources(&src, &mut files);
        // Spelled in pieces so this file does not trip its own check.
        let loops = [
            ["request_animation", "_frame"].concat(),
            ["set_", "interval"].concat(),
            ["Interval", "::new"].concat(),
        ];
        let timers = [["gloo_", "timers"].concat(), ["set_", "timeout"].concat()];
        let clock = ["Date", "::now"].concat();
        let mut found = Vec::new();
        for path in files {
            let name = path
                .strip_prefix(&src)
                .expect("under src")
                .to_string_lossy()
                .replace('\\', "/");
            if name == "engine/run.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read a source file");
            for token in &loops {
                if text.contains(token.as_str()) {
                    found.push(format!("{name}: {token}"));
                }
            }
            if !TIMERS_PENDING.contains(&name.as_str()) {
                for token in &timers {
                    if text.contains(token.as_str()) {
                        found.push(format!("{name}: {token}"));
                    }
                }
            }
            if text.contains(clock.as_str()) && !DAY_TIME.contains(&name.as_str()) {
                found.push(format!("{name}: {clock}"));
            }
        }
        assert!(
            found.is_empty(),
            "a private loop, timer or clock outside the engine (use engine::on_tick, next_frame, every, after, wait or now): {found:?}"
        );
    }
}
