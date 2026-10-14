//! The done-task tack board (docs/DESIGN-DECISIONS.md, "Done tasks: the tack board").
//!
//! Ticking a task done cuts its line out as a paper strip that hangs from a pin,
//! swings and settles. Dragging a strip restores the task (drop on the list),
//! sets it aside (drop on the board) or crumple-deletes it (fling off the page
//! or onto the bin). Done-ness is server truth; where strips sit is
//! presentation only, saved per device in localStorage.
//!
//! The physics is a hand-written damped-pendulum integrator — no physics
//! library. Everything the browser animates runs through [`Phys::step`] with a
//! fixed timestep, so the unit tests below decide exactly how a strip moves.
//! Rambo's own choreography (cutting the strip out, grumpy tidying) belongs to
//! the mascot code; this board only reports [`Cue`]s on a signal it is handed.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use todo_core::{Snapshot, TaskId};
use wasm_bindgen::JsCast;

use crate::api;
use crate::settings;

/// What the board tells the mascot to react to. Fresh `Some(..)` per event;
/// the consumer's Effect reads and clears it.
#[derive(Clone, Debug, PartialEq)]
pub enum Cue {
    /// A task was ticked done: the strip swings in under the pin now, and
    /// Rambo does the cut-the-line-out choreography (the tick's pause has
    /// elapsed).
    CutAndTack { task_id: String },
    /// The pin was tapped: every set-aside strip goes back on the tack. Rambo
    /// does this grudgingly.
    TidyUp,
    /// A strip was flung off the page or dropped on the bin: crumple, delete.
    Crumpled,
}

// ---- the physics, pure ----

/// One strip's swing: angle from its rest direction, and angular velocity.
///
/// A tacked strip hangs from the pin (rest 0 = straight down); a strip lying
/// loose rests at its own small angle. Both are the same pendulum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phys {
    pub angle: f64,
    pub vel: f64,
}

/// Pendulum stiffness, chosen for a paper-strip swing period of about 0.8 s:
/// `sqrt(K)` is the small-angle angular frequency in rad/s.
const K: f64 = 61.0;
/// Damping: strong enough to settle inside about two and a half seconds,
/// weak enough to swing through three or four arcs first.
const C: f64 = 5.0;
/// The fixed physics timestep. The animation loop accumulates real frame time
/// and steps in multiples of this, so a strip moves identically at any fps.
pub const PHYS_DT: f64 = 1.0 / 120.0;

/// Below these the strip is at rest and can sleep (and its angle snaps to
/// `rest`, so a settled strip never renders a hair off zero).
const SETTLED_ANGLE: f64 = 1.0e-3;
const SETTLED_VEL: f64 = 2.0e-2;

impl Phys {
    /// Semi-implicit Euler step toward `rest`. Symplectic, so it does not gain
    /// energy at the timesteps used here.
    pub fn step(self, dt: f64, rest: f64) -> Phys {
        let vel = self.vel + (-(self.angle - rest).sin() * K - self.vel * C) * dt;
        let angle = self.angle + vel * dt;
        if (angle - rest).abs() < SETTLED_ANGLE && vel.abs() < SETTLED_VEL {
            return Phys {
                angle: rest,
                vel: 0.0,
            };
        }
        Phys { angle, vel }
    }

    pub fn settled(self, rest: f64) -> bool {
        self.angle == rest && self.vel == 0.0
    }

    /// Mechanical energy in units of `K`, for tests: it must never grow.
    #[cfg(test)]
    fn energy(self, rest: f64) -> f64 {
        0.5 * self.vel * self.vel / K + (1.0 - (self.angle - rest).cos())
    }
}

// ---- the drop decision, pure ----

/// A rectangle in viewport coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

/// What dropping a strip means, in testing order: the bin always wins; then
/// wherever the strip actually landed (the board, when it overlaps the list on
/// a phone, still counts as the board so a strip can be set aside onto it);
/// anything still moving fast enough was thrown away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAction {
    /// Back on the list: the task becomes not done.
    Restore,
    /// Loose on the board: it lies there until tidied up.
    SetAside,
    /// Flinged off the page or dropped on the bin: crumple and delete.
    Crumple,
}

/// A fling is any release at or above this speed, in CSS pixels per second —
/// a deliberate throw, not a drag that merely ended outside the board.
pub const FLING_PX_S: f64 = 900.0;

pub fn drop_action(
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    list: &Rect,
    board: &Rect,
    bin: &Rect,
) -> DropAction {
    if bin.contains(x, y) {
        return DropAction::Crumple;
    }
    if board.contains(x, y) {
        return DropAction::SetAside;
    }
    if list.contains(x, y) {
        return DropAction::Restore;
    }
    let speed = (vx * vx + vy * vy).sqrt();
    if speed >= FLING_PX_S {
        return DropAction::Crumple;
    }
    // Missed everything, slowly: snapped onto the board as set-aside.
    DropAction::SetAside
}

// ---- the board's own model, pure transforms ----

/// About this many strips hang or lie in view; older ones stack under the pin.
pub const MAX_HANGING: usize = 30;
/// The pause between the tick and the strip appearing. The mascot's
/// cut-the-strip choreography is timed against this same constant.
pub const PAUSE_MS: u32 = 650;
/// How long the paper takes to ball up and fade. Matches `tack-crumple` in
/// tack.css, and the strip is deleted once it is over.
pub const CRUMPLE_MS: u32 = 360;

#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    pub id: String,
    pub title: String,
    /// Where the strip hangs from the pin (board-local px) or where it lies.
    pub x: f64,
    pub y: f64,
    /// The angle it rests at when loose (unused while tacked).
    pub rest: f64,
    pub tacked: bool,
    pub stacked: bool,
    /// Paint order: newest on top, oldest first in the list.
    pub z: u32,
}

/// Deterministic small value for a strip, so a given task always hangs from
/// the same offset on the pin and lies at the same jaunty angle. FNV-1a.
pub fn hash_unit(id: &str, salt: u8) -> f64 {
    // The top 24 bits mapped to -1..=1.
    (f64::from(hash32(id, salt) >> 8) / 16_777_215.0 - 0.5) * 2.0
}

fn hash32(id: &str, salt: u8) -> u32 {
    let mut hash: u32 = 2166136261;
    for byte in id.bytes().chain([salt]) {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16777619);
    }
    hash
}

/// Which of the three torn-paper edges this strip is cut with, so the board
/// does not read as thirty identical bands.
pub fn tear_index(id: &str) -> usize {
    hash32(id, 4) as usize % 3
}

/// Where the pin sits, given the board's width.
pub fn pin_at(board_w: f64) -> (f64, f64) {
    (board_w / 2.0, 12.0)
}

/// The pin's attach slots centre a strip ([`STRIP_W`] / 2 to the left) a few
/// px around the pin so hanging strips do not perfectly overlap; `hash_unit`
/// makes each task's slot stable. Mirrors `--tack-strip-w` in tack.css.
const STRIP_W: f64 = 128.0;

fn attach_point(board_w: f64, id: &str) -> (f64, f64) {
    let (pin_x, pin_y) = pin_at(board_w);
    (pin_x + hash_unit(id, 1) * 10.0 - STRIP_W / 2.0, pin_y)
}

/// Where the `index`-th stacked strip lies: a small heap under the pin.
fn pile_at(board_w: f64, index: usize, id: &str) -> (f64, f64, f64) {
    let (pin_x, pin_y) = pin_at(board_w);
    let jitter = hash_unit(id, 3);
    (
        pin_x - 54.0 + (index % 4) as f64 * 28.0 + jitter * 6.0,
        pin_y + 118.0 + (index / 4) as f64 * 20.0 + jitter * 4.0,
        jitter * 6.0,
    )
}

/// Put every strip hanging from the pin back on its own slot of a board
/// `board_w` wide, so a board that changed width (a rotated phone) carries its
/// strips with it. Strips lying loose and strips in the pile keep their places.
pub fn reattach(strips: &mut [Strip], board_w: f64) {
    for strip in strips.iter_mut().filter(|s| s.tacked && !s.stacked) {
        (strip.x, strip.y) = attach_point(board_w, &strip.id);
    }
}

/// Newest strips hang; once more than [`MAX_HANGING`] are in view, the oldest
/// ones join the stack under the pin.
pub fn apply_cap(strips: &mut [Strip]) {
    let hanging = strips.iter().filter(|s| !s.stacked).count();
    if hanging <= MAX_HANGING {
        return;
    }
    let mut to_stack = hanging - MAX_HANGING;
    for strip in strips.iter_mut() {
        if to_stack == 0 {
            break;
        }
        if !strip.stacked {
            strip.stacked = true;
            strip.tacked = false;
            to_stack -= 1;
        }
    }
    renumber(strips);
}

/// Keep paint order consistent: oldest 1, newest on top.
fn renumber(strips: &mut [Strip]) {
    for (i, strip) in strips.iter_mut().enumerate() {
        strip.z = i as u32 + 1;
    }
}

/// What one pass over the server's done tasks tells the board to do.
#[derive(Clone, Debug, PartialEq)]
pub struct Pass {
    /// The strips that stay, with their titles brought up to date.
    pub kept: Vec<Strip>,
    /// Done tasks with no strip yet, in the order the server lists them.
    pub fresh: Vec<(String, String)>,
    /// Which of `fresh` the board watched go from listed to done: the ones that
    /// get the cut-out-and-tack ceremony rather than going up quietly.
    pub just_done: Vec<String>,
}

/// Server truth wins: a strip whose task is no longer done (restored, deleted,
/// gone) leaves the board, and a renamed task renames its strip.
///
/// `listed` is the ids the previous snapshot still showed as to-do. A fresh
/// task that is in there was ticked while the page was open — that is the
/// strip Rambo cuts out. One that is not was already done when the board
/// opened, so it goes straight up on the pin without the ceremony.
pub fn reconcile(strips: Vec<Strip>, done: &[(String, String)], listed: &HashSet<String>) -> Pass {
    let kept: Vec<Strip> = strips
        .into_iter()
        .filter_map(|mut strip| {
            let (_, title) = done.iter().find(|(id, _)| *id == strip.id)?;
            strip.title = title.clone();
            Some(strip)
        })
        .collect();
    let fresh: Vec<(String, String)> = done
        .iter()
        .filter(|(id, _)| !kept.iter().any(|strip| strip.id == *id))
        .cloned()
        .collect();
    let just_done = fresh
        .iter()
        .map(|(id, _)| id.clone())
        .filter(|id| listed.contains(id))
        .collect();
    Pass {
        kept,
        fresh,
        just_done,
    }
}

// ---- persistence, per device, presentation only ----

const STORAGE_KEY: &str = "rambl-tackboard-v1";

#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    v: u8,
    open: bool,
    strips: Vec<StoredStrip>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredStrip {
    id: String,
    x: f64,
    y: f64,
    rest: f64,
    tacked: bool,
    stacked: bool,
}

fn load_stored() -> Option<Stored> {
    let storage = web_sys::window()?.local_storage().ok().flatten()?;
    let raw = storage.get_item(STORAGE_KEY).ok().flatten()?;
    // A corrupt or future version is ignored, not fatal: the board rebuilds
    // from the server's list of done tasks.
    serde_json::from_str(&raw).ok()
}

fn save_stored(open: bool, strips: &[Strip]) {
    let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else {
        return;
    };
    let stored = Stored {
        v: 1,
        open,
        strips: strips
            .iter()
            .map(|s| StoredStrip {
                id: s.id.clone(),
                x: s.x,
                y: s.y,
                rest: s.rest,
                tacked: s.tacked,
                stacked: s.stacked,
            })
            .collect(),
    };
    if let Ok(json) = serde_json::to_string(&stored) {
        // Storage can refuse (private mode, quota): positions are only
        // presentation, so losing them is fine.
        let _ = storage.set_item(STORAGE_KEY, &json);
    }
}

// ---- the component ----

#[component]
pub fn TackBoard(
    snapshot: ReadSignal<Snapshot>,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
    view_settings: ReadSignal<settings::Settings>,
    set_cue: WriteSignal<Option<Cue>>,
) -> impl IntoView {
    let board_ref = NodeRef::<leptos::html::Section>::new();
    let strips_ref = NodeRef::<leptos::html::Div>::new();
    let bin_ref = NodeRef::<leptos::html::Div>::new();
    let strips = RwSignal::new(Vec::<Strip>::new());
    let (open, set_open) = signal(false);
    let (dragging, set_dragging) = signal(None::<String>);

    if let Some(stored) = load_stored() {
        set_open.set(stored.open);
        strips.set(
            stored
                .strips
                .into_iter()
                .map(|s| Strip {
                    id: s.id,
                    title: String::new(),
                    x: s.x,
                    y: s.y,
                    rest: s.rest,
                    tacked: s.tacked,
                    stacked: s.stacked,
                    z: 0,
                })
                .collect(),
        );
    }

    // Runtime-only state the animation loop owns; never reactive, so per-frame
    // physics costs the framework nothing. Strip elements are re-read from the
    // DOM after every render (`refresh_nodes`) so the loop can write transforms
    // straight to them without any node cache inside the render closure, which
    // `<For>` requires to be `Send`.
    let phys = Rc::new(RefCell::new(HashMap::<String, Phys>::new()));
    let nodes = Rc::new(RefCell::new(HashMap::<String, web_sys::HtmlElement>::new()));
    let board_w = Rc::new(Cell::new(180.0_f64));
    let loop_alive = Rc::new(Cell::new(false));
    // Strips whose ceremony is counting down, so a snapshot landing mid-pause
    // does not queue a second one.
    let pending = Rc::new(RefCell::new(HashSet::<String>::new()));

    let reduced = crate::mascot::prefers_reduced_motion();

    let measure = {
        let board_w = board_w.clone();
        move || {
            if let Some(el) = board_ref.get() {
                let w = el.get_bounding_client_rect().width();
                if w > 0.0 && w != board_w.get() {
                    board_w.set(w);
                    // The pin moved, so the strips hanging off it move with it.
                    strips.update(|list| reattach(list, w));
                }
            }
        }
    };

    // Where a strip renders right now, as a transform string. Stacked strips
    // place themselves in the pile; hanging ones swing by their physics.
    let strip_transform = {
        let board_w = board_w.clone();
        move |strip: &Strip, list: &[Strip], swing: Option<Phys>| -> String {
            if strip.stacked {
                let index = list.iter().filter(|s| s.stacked && s.z < strip.z).count();
                let (x, y, rest) = pile_at(board_w.get(), index, &strip.id);
                return format!(
                    "translate({x:.1}px, {y:.1}px) rotate({:.1}deg)",
                    rest.to_degrees()
                );
            }
            let angle = swing.map(|p| p.angle).unwrap_or(strip.rest);
            format!(
                "translate({x:.1}px, {y:.1}px) rotate({angle:.2}deg)",
                x = strip.x,
                y = strip.y,
                angle = angle.to_degrees()
            )
        }
    };

    // The one place a strip's placement reaches the DOM, so a frame of physics
    // and a re-render can never disagree about where a strip is. The strip
    // under the pointer is left out: it follows the pointer, not the pendulum.
    let place = {
        let phys = phys.clone();
        let nodes = nodes.clone();
        let strip_transform = strip_transform.clone();
        move |list: &[Strip], strip: &Strip| {
            if dragging.get_untracked().as_deref() == Some(strip.id.as_str()) {
                return;
            }
            let swing = phys.borrow().get(&strip.id).copied();
            let nodes = nodes.borrow();
            let Some(el) = nodes.get(&strip.id) else {
                return;
            };
            let transform = strip_transform(strip, list, swing);
            let _ = el.set_attribute("style", &placement(strip, &transform));
        }
    };

    // Best-effort sound, and the gear menu's mute is the only thing that stops
    // it: Ping's recording when it exists (art/sounds/SPEC.md —
    // `assets/sounds/<name>.mp3`, volume 0.3), silence when the board is muted
    // or the browser will not give us an audio element.
    let play = move |name: &str| {
        if !view_settings.with_untracked(|settings| settings.sounds) {
            return;
        }
        let Ok(audio) = web_sys::HtmlAudioElement::new() else {
            return;
        };
        audio.set_src(&format!("assets/sounds/{name}.mp3"));
        audio.set_volume(0.3);
        let _ = audio.play();
    };

    // Write transforms straight to the DOM: 30 strips at 60 fps must not go
    // through fine-grained reactivity.
    let sync_transforms = {
        let place = place.clone();
        move || {
            let list = strips.get_untracked();
            for strip in &list {
                place(&list, strip);
            }
        }
    };

    // Measure the board, re-read the strip elements and place everything. Run
    // after any render that added, removed or resized a strip; `after_render`
    // keeps it off the reactive path.
    let relayout = {
        let measure = measure.clone();
        let place = place.clone();
        let nodes = nodes.clone();
        Rc::new(move || {
            let measure = measure.clone();
            let place = place.clone();
            let nodes = nodes.clone();
            after_render(move || {
                measure();
                refresh_nodes(strips_ref, &nodes);
                let list = strips.get_untracked();
                for strip in &list {
                    place(&list, strip);
                }
            });
        })
    };

    /// The animation frame callback, held so it can re-arm itself until every
    /// strip has come to rest.
    type Frame = Rc<RefCell<Option<wasm_bindgen::prelude::Closure<dyn FnMut(f64)>>>>;

    // The animation loop: accumulate frame time, step every strip a whole
    // number of PHYS_DTs, write transforms, then sleep until an impulse wakes
    // it again.
    let start_loop = {
        let phys = phys.clone();
        let place = place.clone();
        let loop_alive = loop_alive.clone();
        Rc::new(move || {
            if reduced || loop_alive.get() {
                return;
            }
            loop_alive.set(true);
            let phys = phys.clone();
            let place = place.clone();
            let loop_alive = loop_alive.clone();
            let closure: Frame = Rc::new(RefCell::new(None));
            let tick = closure.clone();
            let mut last: Option<f64> = None;
            let mut acc = 0.0_f64;
            *closure.borrow_mut() = Some(wasm_bindgen::prelude::Closure::wrap(Box::new(
                move |now: f64| {
                    let dt = last
                        .map(|l| ((now - l) / 1000.0).clamp(0.0, 0.1))
                        .unwrap_or(0.0);
                    last = Some(now);
                    acc += dt;
                    let list = strips.get_untracked();
                    while acc >= PHYS_DT {
                        acc -= PHYS_DT;
                        let mut phys = phys.borrow_mut();
                        for strip in &list {
                            let rest = if strip.tacked { 0.0 } else { strip.rest };
                            let entry = phys.entry(strip.id.clone()).or_insert(Phys {
                                angle: rest,
                                vel: 0.0,
                            });
                            *entry = entry.step(PHYS_DT, rest);
                        }
                    }
                    let phys = phys.borrow();
                    let awake = list.iter().any(|strip| {
                        let rest = if strip.tacked { 0.0 } else { strip.rest };
                        phys.get(&strip.id).is_some_and(|p| !p.settled(rest))
                    });
                    for strip in &list {
                        place(&list, strip);
                    }
                    if awake
                        && let (Some(tick), Some(window)) =
                            (tick.borrow().as_ref(), web_sys::window())
                    {
                        window
                            .request_animation_frame(tick.as_ref().unchecked_ref())
                            .ok();
                    } else {
                        loop_alive.set(false);
                        *tick.borrow_mut() = None;
                    }
                },
            )));
            if let (Some(start), Some(window)) = (closure.borrow().as_ref(), web_sys::window()) {
                window
                    .request_animation_frame(start.as_ref().unchecked_ref())
                    .ok();
            }
        })
    };

    // A strip appears for a done task, either quietly (already done when the
    // page opened) or after the pause, swinging, with the cue for Rambo.
    let add_strip = {
        let phys = phys.clone();
        let board_w = board_w.clone();
        let start_loop = start_loop.clone();
        let sync = sync_transforms.clone();
        move |id: &str, title: &str, swing_in: bool| {
            let (x, y) = attach_point(board_w.get(), id);
            strips.update(|list| {
                list.push(Strip {
                    id: id.to_owned(),
                    title: title.to_owned(),
                    x,
                    y,
                    rest: hash_unit(id, 2) * 0.2,
                    tacked: true,
                    stacked: false,
                    z: 0,
                });
                apply_cap(list);
                renumber(list);
            });
            if swing_in {
                if !reduced {
                    phys.borrow_mut().insert(
                        id.to_owned(),
                        Phys {
                            angle: -1.1,
                            vel: 0.35,
                        },
                    );
                    start_loop();
                }
                play("pin");
                set_cue.set(Some(Cue::CutAndTack {
                    task_id: id.to_owned(),
                }));
            }
            sync();
        }
    };

    // Four places below need to re-measure and re-place; each takes its own handle
    // so none of them fights the others for it.
    let on_snapshot = relayout.clone();
    let on_open = relayout.clone();
    let on_crumple = relayout.clone();
    let on_toggle = relayout.clone();

    // Follow the server: strips live exactly as long as their task is done, and
    // a task the board watched get ticked is cut out and tacked after the
    // pause instead of appearing out of nowhere.
    Effect::new(move |_| {
        let snap = snapshot.get();
        let done: Vec<(String, String)> = snap
            .tasks
            .iter()
            .filter(|t| t.done)
            .map(|t| (t.id.0.clone(), t.title.clone()))
            .collect();
        let listed: HashSet<String> = snap
            .tasks
            .iter()
            .filter(|t| !t.done)
            .map(|t| t.id.0.clone())
            .collect();
        let Pass {
            mut kept,
            fresh,
            just_done,
        } = reconcile(strips.get_untracked(), &done, &listed);
        renumber(&mut kept);
        strips.set(kept);

        for (id, title) in fresh {
            if !just_done.contains(&id) {
                // Already done when the page opened: it goes straight up.
                add_strip(&id, &title, false);
                continue;
            }
            if !pending.borrow_mut().insert(id.clone()) {
                continue;
            }
            let add_strip = add_strip.clone();
            let pending = pending.clone();
            // The spec's moment's pause, then Rambo cuts the strip out and
            // tacks it: the strip swings in and the cue goes out together.
            Timeout::new(PAUSE_MS, move || {
                pending.borrow_mut().remove(&id);
                // Unticked again during the pause: nothing to cut out.
                if snapshot.with_untracked(|s| s.tasks.iter().any(|t| t.done && t.id.0 == id)) {
                    add_strip(&id, &title, true);
                }
            })
            .forget();
        }
        let relayout = on_snapshot.clone();
        relayout();
    });

    // Measure the board once mounted and whenever the phone pull-up opens
    // (which changes its width), then re-place every strip.
    let relayout = on_open.clone();
    Effect::new(move |_| {
        let _ = open.get();
        relayout();
    });

    // Persist presentation state whenever it changes.
    Effect::new(move |_| {
        save_stored(open.get(), &strips.get());
    });

    // ---- what the user can do ----

    // The tidy-up: every loose or stacked strip goes back on the tack, and
    // Rambo is asked to look put out about it.
    let tidy = {
        let phys = phys.clone();
        let board_w = board_w.clone();
        let start_loop = start_loop.clone();
        let sync = sync_transforms.clone();
        move |_| {
            let width = board_w.get();
            let mut any = false;
            let mut re_tack: Vec<(String, u32)> = Vec::new();
            strips.update(|list| {
                for (i, strip) in list.iter_mut().enumerate() {
                    if strip.tacked && !strip.stacked {
                        continue;
                    }
                    let (x, y) = attach_point(width, &strip.id);
                    strip.x = x;
                    strip.y = y;
                    strip.tacked = true;
                    strip.stacked = false;
                    any = true;
                    re_tack.push((strip.id.clone(), i as u32));
                }
                // Back on the tack, the board is over-full again, so the
                // longest-waiting strips go into the pile.
                apply_cap(list);
            });
            if !any {
                return;
            }
            if !reduced {
                // Staggered re-tacking: one strip snaps on every 90 ms.
                for (id, i) in re_tack {
                    let phys = phys.clone();
                    let start_loop = start_loop.clone();
                    Timeout::new(i * 90, move || {
                        phys.borrow_mut().insert(
                            id,
                            Phys {
                                angle: -0.8,
                                vel: 0.2,
                            },
                        );
                        start_loop();
                    })
                    .forget();
                }
            }
            set_cue.set(Some(Cue::TidyUp));
            play("pin");
            sync();
        }
    };

    // Dragging: the strip follows the pointer; the release decides. Velocity
    // is measured over the last ~120 ms of pointer samples.
    //
    // The three handlers sit on the container, not on each strip: the strip a
    // pointer event belongs to is read off the element it started on. That
    // keeps the render closure holding nothing but plain data, which `<For>`
    // requires, and gives the drag one pointer capture for the whole gesture.
    let samples: Rc<RefCell<Vec<(f64, f64, f64)>>> = Rc::new(RefCell::new(Vec::new()));

    let strip_under = move |ev: &leptos::ev::PointerEvent| -> Option<String> {
        let target = ev.target()?.dyn_into::<web_sys::Element>().ok()?;
        target
            .closest(".tack-strip")
            .ok()??
            .get_attribute("data-strip-id")
    };

    let strip_pointer_down = {
        let samples = samples.clone();
        let phys = phys.clone();
        move |ev: leptos::ev::PointerEvent| {
            // Only the primary button picks a strip up, and only one strip at
            // a time; a right-click opens nothing and must not start a drag
            // the release would then act on.
            if ev.button() != 0 || dragging.get_untracked().is_some() {
                return;
            }
            let Some(id) = strip_under(&ev) else {
                return;
            };
            ev.prevent_default();
            // Captured on the container, so the drag keeps reporting even once
            // the pointer leaves the board.
            if let Some(board) = ev
                .current_target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            {
                let _ = board.set_pointer_capture(ev.pointer_id());
            }
            samples.borrow_mut().clear();
            samples.borrow_mut().push((
                js_sys::Date::now(),
                f64::from(ev.client_x()),
                f64::from(ev.client_y()),
            ));
            phys.borrow_mut().remove(&id);
            set_dragging.set(Some(id));
        }
    };

    let strip_pointer_move = {
        let samples = samples.clone();
        let nodes = nodes.clone();
        move |ev: leptos::ev::PointerEvent| {
            if dragging.get_untracked().is_none() {
                return;
            }
            let (client_x, client_y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
            {
                let mut samples = samples.borrow_mut();
                samples.push((js_sys::Date::now(), client_x, client_y));
                while samples.len() > 2 && samples[1].0 - samples[0].0 > 120.0 {
                    samples.remove(0);
                }
            }
            let origin = board_ref
                .get()
                .map(|el| {
                    let r = el.get_bounding_client_rect();
                    (r.left(), r.top())
                })
                .unwrap_or((0.0, 0.0));
            let id = dragging.get_untracked();
            let nodes = nodes.borrow();
            if let Some(el) = id.as_deref().and_then(|id| nodes.get(id)) {
                let x = client_x - origin.0 - STRIP_W / 2.0;
                let y = client_y - origin.1 - 6.0;
                let _ = el.set_attribute(
                    "style",
                    &format!("transform: translate({x:.1}px, {y:.1}px) rotate(3deg); z-index: 99;"),
                );
            }
        }
    };

    let strip_pointer_up = {
        let phys = phys.clone();
        let nodes = nodes.clone();
        let board_w = board_w.clone();
        let start_loop = start_loop.clone();
        let sync = sync_transforms.clone();
        let relayout = on_crumple.clone();
        move |ev: leptos::ev::PointerEvent| {
            let Some(id) = dragging.get_untracked() else {
                return;
            };
            let (client_x, client_y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
            let samples = samples.borrow().clone();
            let (vx, vy) = match (samples.first(), samples.last()) {
                (Some((t0, x0, y0)), Some((t1, x1, y1))) if t1 > t0 => (
                    (x1 - x0) * 1000.0 / (t1 - t0),
                    (y1 - y0) * 1000.0 / (t1 - t0),
                ),
                _ => (0.0, 0.0),
            };

            let board_rect = board_ref
                .get()
                .map(|el| dom_rect(el.get_bounding_client_rect()))
                .unwrap_or(Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                });
            let list_rect = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|doc| {
                    doc.query_selector(".tasks-pane")
                        .ok()
                        .flatten()
                        .or_else(|| doc.query_selector("main").ok().flatten())
                })
                .map(|el| dom_rect(el.get_bounding_client_rect()))
                .unwrap_or(Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                });
            // An impossible rect when the bin is not on screen.
            let bin_rect = bin_ref
                .get()
                .map(|el| dom_rect(el.get_bounding_client_rect()))
                .unwrap_or(Rect {
                    x: -10.0,
                    y: -10.0,
                    w: 0.0,
                    h: 0.0,
                });

            set_dragging.set(None);

            match drop_action(
                client_x,
                client_y,
                vx,
                vy,
                &list_rect,
                &board_rect,
                &bin_rect,
            ) {
                DropAction::Restore => {
                    let body = snapshot.with_untracked(|s| crate::toggle_done_body(s, &id));
                    let sink = crate::EditSink {
                        set_snapshot,
                        set_error,
                        view_settings,
                    };
                    wasm_bindgen_futures::spawn_local(async move {
                        if let Some(body) = body {
                            // The snapshot effect reconciles the strip away
                            // once the server says the task is not done.
                            crate::patch_and_show(&TaskId(id), body, sink).await;
                        }
                    });
                    // Until the PATCH answers, the strip goes back to where it
                    // hung rather than hanging mid-air where it was dropped.
                    sync();
                }
                DropAction::SetAside => {
                    let x = (client_x - board_rect.x - STRIP_W / 2.0)
                        .clamp(4.0, (board_w.get() - STRIP_W - 4.0).max(4.0));
                    let y = (client_y - board_rect.y - 6.0).clamp(60.0, 520.0);
                    let rest = hash_unit(&id, 2) * 0.28;
                    strips.update(|list| {
                        if let Some(strip) = list.iter_mut().find(|s| s.id == id) {
                            strip.tacked = false;
                            strip.stacked = false;
                            strip.x = x;
                            strip.y = y;
                            strip.rest = rest;
                        }
                        renumber(list);
                    });
                    if !reduced {
                        // A flutter proportional to how hard it was thrown.
                        let impulse = (vx / 4000.0).clamp(-3.0, 3.0);
                        phys.borrow_mut().insert(
                            id,
                            Phys {
                                angle: rest + impulse * 0.15,
                                vel: impulse,
                            },
                        );
                        start_loop();
                    }
                    sync();
                }
                DropAction::Crumple => {
                    set_cue.set(Some(Cue::Crumpled));
                    play("crumple");
                    // Crumple animation first, delete under it; the strip
                    // leaves local state with the animation, and the server's
                    // list once the DELETE lands.
                    if let Some(el) = nodes.borrow().get(&id) {
                        let arr = js_sys::Array::new();
                        arr.push(&"crumpling".into());
                        let _ = el.class_list().add(&arr);
                    }
                    let nodes = nodes.clone();
                    let sync = sync.clone();
                    Timeout::new(CRUMPLE_MS, move || {
                        let arr = js_sys::Array::new();
                        arr.push(&"crumpling".into());
                        if let Some(el) = nodes.borrow().get(&id) {
                            let _ = el.class_list().remove(&arr);
                        }
                        strips.update(|list| {
                            list.retain(|s| s.id != id);
                            renumber(list);
                        });
                        sync();
                        let delete_id = id;
                        wasm_bindgen_futures::spawn_local(async move {
                            match api::delete_task(&TaskId(delete_id)).await {
                                Ok(snap) => set_snapshot.set(snap),
                                Err(_) => {
                                    set_error.set(Some("Could not delete that task.".to_owned()))
                                }
                            }
                        });
                    })
                    .forget();
                    relayout();
                }
            }
        }
    };

    let toggle_open = {
        let relayout = on_toggle.clone();
        move |_| {
            set_open.update(|o| *o = !*o);
            relayout();
        }
    };

    let done_count = move || strips.get().iter().filter(|s| !s.stacked).count();

    // A pointercancel ends the drag the same way any other release would.
    let cancelled = strip_pointer_up.clone();

    view! {
        <section
            node_ref=board_ref
            class:tack-board=true
            class:open=move || open.get()
            class:dragging=move || dragging.get().is_some()
            aria-label="Tack board of done tasks"
        >
            <button class="tack-pin" type="button" aria-label="Tidy up: put the set-aside strips back on the tack" on:click=tidy>
                <img class="tack-pin-up" src="assets/tackboard/pin.svg" alt="" />
                <img class="tack-pin-down" src="assets/tackboard/pin-pressed.svg" alt="" />
            </button>
            <div
                class="tack-strips"
                node_ref=strips_ref
                on:pointerdown=strip_pointer_down
                on:pointermove=strip_pointer_move
                on:pointerup=strip_pointer_up
                on:pointercancel=cancelled
            >
                <For each=move || strips.get() key=|s| s.id.clone() let:strip>
                    {let label = format!(
                        "Done: {}. Drag back to the list to restore, onto the bin to delete.",
                        strip.title
                    );
                     let tear = tear_index(&strip.id) + 1;
                     view! {
                        <div
                            class="tack-strip"
                            class:stacked=strip.stacked
                            class:aside=!strip.tacked && !strip.stacked
                            data-strip-id=strip.id.clone()
                            data-strip-tear=tear
                            role="button"
                            tabindex="0"
                            aria-label=label
                            title=strip.title.clone()
                        >
                            <s class="tack-strip-title">{strip.title.clone()}</s>
                        </div>
                     }}
                </For>
            </div>
            {move || {
                dragging.get().map(|_| view! {
                    <div class="tack-bin" node_ref=bin_ref role="img" aria-label="Bin — drop a strip here to crumple it"></div>
                })
            }}
            <button class="tack-tab" type="button" aria-expanded=move || open.get().to_string() on:click=toggle_open>
                "Done " <span class="tack-tab-count">{done_count}</span>
            </button>
        </section>
    }
}

/// Run once the current render has flushed to the DOM.
fn dom_rect(r: web_sys::DomRect) -> Rect {
    Rect {
        x: r.left(),
        y: r.top(),
        w: r.width(),
        h: r.height(),
    }
}

/// The inline style a placed strip carries: where it is, which way up, and how
/// far it is faded into the pile.
fn placement(strip: &Strip, transform: &str) -> String {
    let alpha = if strip.stacked { 0.92 } else { 1.0 };
    format!(
        "transform: {transform}; z-index: {}; opacity: {alpha};",
        strip.z
    )
}

/// Re-read the strip elements out of the DOM, keyed by the task each one
/// belongs to. Done after a render rather than during it, so the render closure
/// holds nothing but plain data.
fn refresh_nodes(
    strips_ref: NodeRef<leptos::html::Div>,
    nodes: &RefCell<HashMap<String, web_sys::HtmlElement>>,
) {
    let Some(container) = strips_ref.get() else {
        return;
    };
    let children = container.children();
    let mut found = HashMap::with_capacity(children.length() as usize);
    for index in 0..children.length() {
        let Some(element) = children
            .item(index)
            .and_then(|child| child.dyn_into::<web_sys::HtmlElement>().ok())
        else {
            continue;
        };
        if let Some(id) = element.get_attribute("data-strip-id") {
            found.insert(id, element);
        }
    }
    *nodes.borrow_mut() = found;
}

fn after_render(f: impl FnOnce() + 'static) {
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(0).await;
        f();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_swing_settles_and_never_gains_energy() {
        let mut p = Phys {
            angle: 1.2,
            vel: 0.0,
        };
        let mut last_energy = p.energy(0.0);
        let mut steps = 0_u32;
        while !p.settled(0.0) {
            p = p.step(PHYS_DT, 0.0);
            steps += 1;
            if steps.is_multiple_of(30) {
                let e = p.energy(0.0);
                assert!(e <= last_energy + 1e-12, "energy grew: {e} > {last_energy}");
                last_energy = e;
            }
            assert!(steps < 60_000, "never settled");
        }
        assert_eq!(p.angle, 0.0);
        // Firmly damped: settled inside about two and a half seconds, and not
        // so damped that it drops straight to rest.
        assert!(
            f64::from(steps) * PHYS_DT < 3.0,
            "took {}s",
            f64::from(steps) * PHYS_DT
        );
    }

    #[test]
    fn a_strip_at_rest_stays_exactly_at_rest() {
        let p = Phys {
            angle: 0.0,
            vel: 0.0,
        };
        assert_eq!(p.step(PHYS_DT, 0.0), p);
        let aside = Phys {
            angle: 0.21,
            vel: 0.0,
        };
        assert_eq!(aside.step(PHYS_DT, 0.21), aside);
    }

    /// How long a strip released from `angle` takes to swing through the
    /// bottom. The discriminator between a real pendulum and a linear spring:
    /// `sin` makes the period grow with the swing, so a wide release takes
    /// noticeably longer to get there than a narrow one.
    fn time_to_bottom(angle: f64) -> f64 {
        let mut p = Phys { angle, vel: 0.0 };
        for steps in 1..100_000 {
            p = p.step(PHYS_DT, 0.0);
            if p.angle <= 0.0 {
                return f64::from(steps) * PHYS_DT;
            }
        }
        panic!("a strip released at {angle} never came down");
    }

    #[test]
    fn the_swing_is_a_real_pendulum_not_a_linear_spring() {
        let small = time_to_bottom(0.17);
        let wide = time_to_bottom(std::f64::consts::FRAC_PI_2);
        assert!(
            wide > small * 1.1,
            "a strip swinging wide ({wide}s) should outlast one released gently ({small}s)"
        );
    }

    #[test]
    fn the_bin_wins_the_list_restores_and_a_fling_is_a_throw() {
        let list = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 700.0,
        };
        let board = Rect {
            x: 1100.0,
            y: 0.0,
            w: 190.0,
            h: 700.0,
        };
        let bin = Rect {
            x: 1130.0,
            y: 600.0,
            w: 60.0,
            h: 60.0,
        };
        // On the bin.
        assert_eq!(
            drop_action(1160.0, 630.0, 0.0, 0.0, &list, &board, &bin),
            DropAction::Crumple
        );
        // Dropped on the list (away from the board): restore, however fast.
        assert_eq!(
            drop_action(200.0, 300.0, 5000.0, 0.0, &list, &board, &bin),
            DropAction::Restore
        );
        // On the board, slow: set aside.
        assert_eq!(
            drop_action(1150.0, 300.0, 0.0, 0.0, &list, &board, &bin),
            DropAction::SetAside
        );
        // Off the board entirely, slow: snapped onto the board as set-aside.
        assert_eq!(
            drop_action(600.0, 300.0, 0.0, 0.0, &list, &board, &bin),
            DropAction::SetAside
        );
        // A fling: fast enough is a throw-away wherever it left the board.
        assert_eq!(
            drop_action(600.0, 300.0, 1200.0, 0.0, &list, &board, &bin),
            DropAction::Crumple
        );
        // Just under the threshold is not.
        assert_eq!(
            drop_action(600.0, 300.0, 899.9, 0.0, &list, &board, &bin),
            DropAction::SetAside
        );
        // When board and list overlap (phone, board pulled up over the list),
        // the board wins so the strip can still be set aside onto it.
        let overlapped_list = Rect {
            x: 0.0,
            y: 0.0,
            w: 390.0,
            h: 844.0,
        };
        let phone_board = Rect {
            x: 0.0,
            y: 500.0,
            w: 390.0,
            h: 344.0,
        };
        assert_eq!(
            drop_action(100.0, 600.0, 0.0, 0.0, &overlapped_list, &phone_board, &bin),
            DropAction::SetAside
        );
    }

    fn strip(id: &str, z: u32, stacked: bool) -> Strip {
        Strip {
            id: id.to_owned(),
            title: String::new(),
            x: 0.0,
            y: 0.0,
            rest: 0.0,
            tacked: !stacked,
            stacked,
            z,
        }
    }

    #[test]
    fn beyond_thirty_strips_the_oldest_stack_under_the_pin() {
        let mut strips: Vec<Strip> = (0..32)
            .map(|i| strip(&format!("t{i}"), i + 1, false))
            .collect();
        apply_cap(&mut strips);
        assert_eq!(strips.iter().filter(|s| !s.stacked).count(), MAX_HANGING);
        assert!(
            strips[0].stacked && strips[1].stacked,
            "oldest two stack first"
        );
        assert!(!strips[31].stacked, "newest stays in view");

        let mut all = (0..35)
            .map(|i| strip(&format!("t{i}"), i + 1, false))
            .collect::<Vec<_>>();
        apply_cap(&mut all);
        assert_eq!(all.iter().filter(|s| !s.stacked).count(), MAX_HANGING);
    }

    #[test]
    fn reconcile_follows_the_server() {
        let mut strips = vec![
            strip("a", 1, false),
            strip("b", 2, false),
            strip("gone", 3, false),
        ];
        strips[0].title = "old name".into();
        let done = vec![
            ("a".to_owned(), "new name".to_owned()),
            ("b".to_owned(), "B".to_owned()),
            ("new".to_owned(), "N".to_owned()),
        ];
        let Pass { kept, fresh, .. } = reconcile(strips, &done, &HashSet::new());
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].title, "new name", "titles follow the server");
        assert_eq!(fresh, vec![("new".to_owned(), "N".to_owned())]);
    }

    #[test]
    fn only_a_task_ticked_in_front_of_you_gets_cut_out() {
        let done = vec![
            ("already".to_owned(), "Done yesterday".to_owned()),
            ("just".to_owned(), "Ticked now".to_owned()),
        ];
        // Page load: the board has seen nothing but the tasks it was given, so
        // the done ones go straight up on the pin.
        let load = reconcile(Vec::new(), &done, &HashSet::new());
        assert_eq!(load.just_done, Vec::<String>::new());
        assert_eq!(load.fresh.len(), 2, "both still need a strip");

        // The user ticks a task the list was showing: that one is cut out.
        let listed = HashSet::from(["just".to_owned()]);
        let ticked = reconcile(Vec::new(), &done, &listed);
        assert_eq!(ticked.just_done, vec!["just".to_owned()]);
    }

    #[test]
    fn a_reattached_board_carries_its_hanging_strips() {
        let mut strips = vec![
            strip("a", 1, false),
            strip("loose", 2, false),
            strip("pile", 3, true),
        ];
        strips[1].tacked = false;
        let (loose_x, loose_y) = (strips[1].x, strips[1].y);
        reattach(&mut strips, 400.0);
        assert_eq!((strips[0].x, strips[0].y), attach_point(400.0, "a"));
        assert_eq!(
            (strips[1].x, strips[1].y),
            (loose_x, loose_y),
            "loose stays put"
        );
        assert_eq!(
            (strips[2].x, strips[2].y),
            (0.0, 0.0),
            "the pile is placed by the render"
        );
    }

    #[test]
    fn every_task_gets_a_stable_tear_and_it_is_one_of_three() {
        assert_eq!(tear_index("task-7"), tear_index("task-7"));
        assert!(tear_index("task-7") < 3);
        // Three tasks in a row, not all the same paper.
        let tears: HashSet<usize> = (0..9).map(|i| tear_index(&format!("t{i}"))).collect();
        assert_eq!(tears.len(), 3, "all three tears are used: {tears:?}");
    }

    #[test]
    fn a_placed_strip_carries_its_own_pile_fade() {
        let mut hanging = strip("a", 7, false);
        assert_eq!(
            placement(&hanging, "translate(0px, 0px)"),
            "transform: translate(0px, 0px); z-index: 7; opacity: 1;"
        );
        hanging.stacked = true;
        assert!(
            placement(&hanging, "translate(0px, 0px)").ends_with("opacity: 0.92;"),
            "a strip in the pile sits back"
        );
    }

    #[test]
    fn a_tasks_slot_on_the_pin_is_stable() {
        assert_eq!(attach_point(180.0, "task-7"), attach_point(180.0, "task-7"));
        assert_ne!(attach_point(180.0, "task-7"), attach_point(180.0, "task-8"));
        assert!(hash_unit("task-7", 1).abs() <= 1.0);
        assert!(hash_unit("", 9).abs() <= 1.0);
    }

    #[test]
    fn stored_strips_round_trip() {
        let stored = Stored {
            v: 1,
            open: true,
            strips: vec![
                StoredStrip {
                    id: "a".into(),
                    x: 1.0,
                    y: 2.0,
                    rest: 0.1,
                    tacked: true,
                    stacked: false,
                },
                StoredStrip {
                    id: "b".into(),
                    x: 3.0,
                    y: 4.0,
                    rest: -0.1,
                    tacked: false,
                    stacked: true,
                },
            ],
        };
        let json = serde_json::to_string(&stored).unwrap();
        let parsed: Stored = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.strips.len(), 2);
        assert!(parsed.open);
        assert!(parsed.strips[1].stacked);
    }
}
