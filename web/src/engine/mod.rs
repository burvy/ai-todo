//! The engine: one loop and one animation clock for everything on the page that moves
//! (ARCHITECTURE "The engine"). Jobs run on its frames (`on_tick`, `next_frame`) or
//! every so often in its time (`every`); the loop sleeps when none is waiting.

mod body;
mod dance;
mod fling;
mod glide;
mod link;
mod look;
mod nest;
mod run;
mod sched;
mod world;

pub(crate) use body::{Pull, Spring1};
pub(crate) use fling::Walls;
pub(crate) use glide::{glide, glide_keyed};
pub(crate) use link::{leaf, on_leaf, set_push};
#[cfg(target_arch = "wasm32")]
pub(crate) use link::{push, show};
pub(crate) use run::{after, cancel, every, next_frame, now, on_tick, wait};
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
            for token in &timers {
                if text.contains(token.as_str()) {
                    found.push(format!("{name}: {token}"));
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

    /// Keyframes that animate the inside of a drawing or the two allowed fades (ink and the
    /// patch), which ARCHITECTURE 8A.2 rule 4 lets CSS own.
    const DRAWING_KEYFRAMES: [&str; 5] = [
        "ink-dry",
        "rambo-fx-float",
        "rambo-splat-burst",
        "scribble-cloud",
        "scribble-out",
    ];

    /// Keyframes that still move a position, size or rotation, which rule 4 forbids: each
    /// goes when its mover joins the world (Rambo's sway, the strips' bin, the planner's
    /// slip-off, the page's arrivals). Delete a name here as its keyframes are deleted.
    const OWED_KEYFRAMES: [&str; 10] = [
        "bin-crumple",
        "landing-unfold",
        "paper-page-in",
        "rambo-breathe",
        "rambo-grumble",
        "row-fall",
        "row-make-room",
        "slip-off",
        "unfold-across",
        "unfold-down",
    ];

    #[test]
    fn css_keyframes_are_only_drawings_or_owed_to_the_world() {
        let css = include_str!("../../theme.css");
        let mut found: Vec<&str> = css
            .split("@keyframes ")
            .skip(1)
            .filter_map(|rest| rest.split_whitespace().next())
            .collect();
        found.sort_unstable();
        let mut known: Vec<&str> = DRAWING_KEYFRAMES
            .iter()
            .chain(&OWED_KEYFRAMES)
            .copied()
            .collect();
        known.sort_unstable();
        assert_eq!(
            found, known,
            "theme.css keyframes differ from the lists above: a new keyframe must be a drawing's own (rule 4), a removed one must leave OWED_KEYFRAMES"
        );
    }

    /// The properties a CSS `transition` may not animate (ARCHITECTURE 8A.2 rule 4): where a
    /// thing is, how big and how turned. Fades and colours are the style sheet's own. `all`
    /// counts, so a catch-all transition cannot hide a movement.
    const MOTION_PROPERTIES: [&str; 17] = [
        "all",
        "transform",
        "translate",
        "rotate",
        "scale",
        "left",
        "right",
        "top",
        "bottom",
        "inset",
        "width",
        "height",
        "max-height",
        "max-width",
        "margin",
        "padding",
        "clip-path",
    ];

    /// The transitions that still move or size something, as `selector: property`: each
    /// goes when its mover joins the world. Delete a line as its transition is deleted.
    const OWED_TRANSITIONS: [&str; 5] = [
        // Rambo's ask bubble (rambo.rs, his owner is moving it).
        ".ask-choice: scale",
        ".ask.folding: scale",
        ".ask.folding: translate",
        ".ask: scale",
        ".ask: translate",
    ];

    /// Every `selector: property` the style sheet's transitions animate that is a motion
    /// property, sorted, without comments.
    fn motion_transitions(css: &str) -> Vec<String> {
        let mut plain = String::new();
        let mut rest = css;
        while let Some(open) = rest.find("/*") {
            plain.push_str(&rest[..open]);
            rest = rest[open..].split_once("*/").map_or("", |(_, after)| after);
        }
        plain.push_str(rest);
        let mut found = Vec::new();
        // `transition: <property> <time> ...` (one part per property) and
        // `transition-property: <property>, ...`.
        for (key, shorthand) in [("transition:", true), ("transition-property:", false)] {
            for (at, _) in plain.match_indices(key) {
                if plain[..at].ends_with(|c: char| c.is_alphanumeric() || c == '-') {
                    continue;
                }
                let Some(open) = plain[..at].rfind('{') else {
                    continue;
                };
                let from = plain[..open].rfind(['{', '}']).map_or(0, |i| i + 1);
                let selector = plain[from..open]
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let value = plain[at + key.len()..].split(';').next().unwrap_or("");
                let (mut depth, mut start, mut parts) = (0_i32, 0, Vec::new());
                for (i, c) in value.char_indices() {
                    match c {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        ',' if depth == 0 => {
                            parts.push(&value[start..i]);
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                parts.push(&value[start..]);
                for part in parts {
                    let property = if shorthand {
                        part.split_whitespace().next()
                    } else {
                        Some(part.trim())
                    };
                    if let Some(property) = property.filter(|p| MOTION_PROPERTIES.contains(p)) {
                        found.push(format!("{selector}: {property}"));
                    }
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }

    #[test]
    fn the_transition_scan_reads_shorthands_longhands_and_all() {
        let css = ".a { transition: all 1s; }
                   .b { transition: opacity 1s, transform 1s cubic-bezier(0.1, 0.2, 0.3, 0.4); }
                   .c { transition-property: color, height; transition-duration: 1s; }
                   .d { transition: color 1s; transition: none; }
                   /* .e { transition: left 1s; } */
                   .f { -webkit-transition: top 1s; }";
        assert_eq!(
            motion_transitions(css),
            [".a: all", ".b: transform", ".c: height"]
        );
    }

    #[test]
    fn css_transitions_never_move_or_size_a_thing_unless_owed_to_the_world() {
        let found = motion_transitions(include_str!("../../theme.css"));
        let mut owed: Vec<String> = OWED_TRANSITIONS
            .iter()
            .map(|line| (*line).to_owned())
            .collect();
        owed.sort_unstable();
        assert_eq!(
            found, owed,
            "theme.css transitions that move or size something differ from OWED_TRANSITIONS: a new one must be a body in the world (rule 4), a removed one must leave the list"
        );
    }
}
