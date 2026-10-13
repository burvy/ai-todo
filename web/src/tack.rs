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

use std::cell::RefCell;
use std::collections::HashSet;
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
/// Damping: strong enough to settle in a couple of seconds, weak enough to
/// swing a few times first.
const C: f64 = 1.15;
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
        let mut angle = self.angle + vel * dt;
        let mut vel = vel;
        if (angle - rest).abs() < SETTLED_ANGLE && vel.abs() < SETTLED_VEL {
            angle = rest;
            vel = 0.0;
        }
        Phys { angle, vel }
    }

    pub fn settled(self, rest: f64) -> bool {
        self.angle == rest && self.vel == 0.0
    }

    /// Mechanical energy in units of `K`, for tests: it must never grow.
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
    let mut hash: u32 = 2166136261;
    for byte in id.bytes().chain([salt]) {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16777619);
    }
    // The top 24 bits mapped to -1..=1.
    (f64::from(hash >> 8) / 16_777_215.0 - 0.5) * 2.0
}

/// Where the pin sits, given the board's width.
pub fn pin_at(board_w: f64) -> (f64, f64) {
    (board_w / 2.0, 12.0)
}

/// The pin's attach slots centre a strip ([`STRIP_W`] / 2 to the left) a few
/// px around the pin so hanging strips do not perfectly overlap; `hash_unit`
/// makes each task's slot stable.
const STRIP_W: f64 = 140.0;

fn attach_point(board_w: f64, id: &str) -> (f64, f64) {
    let (pin_x, pin_y) = pin_at(board_w);
    (pin_x + hash_unit(id, 1) * 10.0 - STRIP_W / 2.0, pin_y)
}

/// Where the `index`-th stacked strip lies: a small heap under the pin.
fn pile_at(board_w: f64, index: usize, id: &str) -> (f64, f64, f64) {
    let (pin_x, pin_y) = pin_at(board_w);
    let jitter = hash_unit(id, 3);
    (
        pin_x - 62.0 + (index % 4) as f64 * 32.0 + jitter * 7.0,
        pin_y + 118.0 + (index / 4) as f64 * 20.0 + jitter * 4.0,
        jitter * 7.0,
    )
}

/// Newest strips hang; once more than [`MAX_HANGING`] are in view, the oldest
/// ones join the stack under the pin.
pub fn apply_cap(strips: &mut Vec<Strip>) {
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

/// Server truth wins: a strip whose task is no longer done (restored, deleted,
/// gone) leaves the board, and a renamed task renames its strip. Returns the
/// strips to keep and the done tasks that have no strip yet, oldest-first.
pub fn reconcile(
    strips: Vec<Strip>,
    done: &[(String, String)],
) -> (Vec<Strip>, Vec<(String, String)>) {
    let mut kept = strips;
    kept.retain(|strip| done.iter().any(|(id, _)| *id == strip.id));
    for strip in &mut kept {
        if let Some((_, title)) = done.iter().find(|(id, _)| *id == strip.id) {
            strip.title = title.clone();
        }
    }
    let missing = done
        .iter()
        .filter(|(id, _)| !kept.iter().any(|strip| strip.id == *id))
        .cloned()
        .collect();
    (kept, missing)
}

/// Best-effort sound: Ping's recording when it exists, silence otherwise
/// (art/sounds/SPEC.md — `assets/sounds/<name>.mp3`, volume 0.3). The gear
/// menu's mute toggle should grow a gate around this later.
pub fn play_sound(name: &str) {
    let Ok(audio) = web_sys::HtmlAudioElement::new() else {
        return;
    };
    audio.set_src(&format!("assets/sounds/{name}.mp3"));
    audio.set_volume(0.3);
    let _ = audio.play();
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
    let Some(storage) = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
    else {
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
    // physics costs the framework nothing. Strip nodes are cached by id so the
    // loop writes transforms straight to the DOM.
    let phys = Rc::new(RefCell::new(std::collections::HashMap::<String, Phys>::new()));
    let nodes =
        Rc::new(RefCell::new(std::collections::HashMap::<String, web_sys::HtmlElement>::new()));
    let board_w = Rc::new(std::cell::Cell::new(180.0_f64));
    let loop_alive = Rc::new(std::cell::Cell::new(false));

    let reduced = crate::mascot::prefers_reduced_motion();

    // Tasks already done when the board first renders get strips without
    // ceremony; only ones that flip done while the page is open get the
    // pause, the swing-in and the cue for Rambo.
    let seen_done = Rc::new(RefCell::new(HashSet::<String>::new()));
    let first_run = Rc::new(std::cell::Cell::new(true));

    let measure = {
        let board_ref = board_ref;
        let board_w = board_w.clone();
        move || {
            if let Some(el) = board_ref.get() {
                let w = el.get_bounding_client_rect().width();
                if w > 0.0 {
                    board_w.set(w);
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
                let index = list
                    .iter()
                    .filter(|s| s.stacked && s.z < strip.z)
                    .count();
                let (x, y, rest) = pile_at(board_w.get(), index, &strip.id);
                return format!(
                    "translate({x:.1}px, {y:.1}px) rotate({:.1}deg)",
                    rest.to_degrees()
                );
            }
            let angle = swing
                .map(|p| p.angle)
                .unwrap_or(strip.rest);
            format!(
                "translate({x:.1}px, {y:.1}px) rotate({angle:.2}deg)",
                x = strip.x,
                y = strip.y,
                angle = angle.to_degrees()
            )
        }
    };

    // Write transforms straight to the DOM: 30 strips at 60 fps must not go
    // through fine-grained reactivity.
    let sync_transforms = {
        let phys = phys.clone();
        let nodes = nodes.clone();
        let strip_transform = strip_transform.clone();
        move || {
            let list = strips.get_untracked();
            let phys = phys.borrow();
            let mut nodes = nodes.borrow_mut();
            nodes.retain(|id, _| list.iter().any(|s| &s.id == id));
            for strip in &list {
                if let Some(el) = nodes.get(&strip.id) {
                    let swing = strip
                        .tacked
                        .then(|| phys.get(&strip.id).copied())
                        .flatten();
                    let transform = strip_transform(strip, &list, swing);
                    let _ = el.set_attribute(
                        "style",
                        &format!("transform: {transform}; z-index: {};", strip.z),
                    );
                }
            }
        }
    };

    // The animation loop: accumulate frame time, step every strip a whole
    // number of PHYS_DTs, write transforms, then sleep until an impulse wakes
    // it again.
    let start_loop = {
        let phys = phys.clone();
        let nodes = nodes.clone();
        let board_w = board_w.clone();
        let loop_alive = loop_alive.clone();
        Rc::new(move || {
            if reduced || loop_alive.get() {
                return;
            }
            loop_alive.set(true);
            let phys = phys.clone();
            let nodes = nodes.clone();
            let board_w = board_w.clone();
            let loop_alive = loop_alive.clone();
            let closure: Rc<RefCell<Option<wasm_bindgen::prelude::Closure<dyn FnMut(f64)>>>> =
                Rc::new(RefCell::new(None));
            let tick = closure.clone();
            let mut last: Option<f64> = None;
            let mut acc = 0.0_f64;
            *closure.borrow_mut() =
                Some(wasm_bindgen::prelude::Closure::wrap(Box::new(move |now: f64| {
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
                            let entry = phys
                                .entry(strip.id.clone())
                                .or_insert(Phys { angle: rest, vel: 0.0 });
                            *entry = entry.step(PHYS_DT, rest);
                        }
                    }
                    let phys = phys.borrow();
                    let mut awake = false;
                    let mut nodes = nodes.borrow_mut();
                    nodes.retain(|id, _| list.iter().any(|s| &s.id == id));
                    for strip in &list {
                        let rest = if strip.tacked { 0.0 } else { strip.rest };
                        let swing = phys.get(&strip.id).copied();
                        if swing.is_some_and(|p| !p.settled(rest)) {
                            awake = true;
                        }
                        if let Some(el) = nodes.get(&strip.id) {
                            let transform = strip_transform(strip, &list, swing);
                            let _ = el.set_attribute(
                                "style",
                                &format!(
                                    "transform: {transform}; z-index: {};",
                                    strip.z
                                ),
                            );
                        }
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
                })));
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
        let set_cue = set_cue;
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
                    phys.borrow_mut()
                        .insert(id.to_owned(), Phys { angle: -1.1, vel: 0.35 });
                    start_loop();
                }
                play_sound("pin");
                set_cue.set(Some(Cue::CutAndTack { task_id: id.to_owned() }));
            }
            sync();
        }
    };

    // Follow the server: reconcile strips with the tasks that are done now,
    // and celebrate the ones that just became done.
    Effect::new(move |_| {
        let snap = snapshot.get();
        let done: Vec<(String, String)> = snap
            .tasks
            .iter()
            .filter(|t| t.done)
            .map(|t| (t.id.0.clone(), t.title.clone()))
            .collect();
        let (mut kept, missing) = reconcile(strips.get_untracked(), &done);
        renumber(&mut kept);
        strips.set(kept);

        if first_run.get() {
            first_run.set(false);
            seen_done
                .borrow_mut()
                .extend(missing.iter().map(|(id, _)| id.clone()));
            for (id, title) in &missing {
                add_strip(id, title, false);
            }
            sync_transforms();
            return;
        }

        for (id, title) in missing {
            if seen_done.borrow().contains(&id) {
                continue;
            }
            seen_done.borrow_mut().insert(id.clone());
            let add_strip = add_strip.clone();
            // The spec's moment's pause, then Rambo cuts the strip out and
            // tacks it: the strip swings in and the cue goes out together.
            Timeout::new(PAUSE_MS, move || {
                add_strip(&id, &title, true);
            })
            .forget();
        }
        sync_transforms();
    });

    // Measure the board once mounted and whenever the phone pull-up opens
    // (which changes its width), then re-place every strip.
    Effect::new(move |_| {
        let _ = open.get();
        let measure = measure.clone();
        let sync = sync_transforms.clone();
        after_render(move || {
            measure();
            sync();
        });
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
        let set_cue = set_cue;
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
                renumber(list);
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
                        phys.borrow_mut()
                            .insert(id, Phys { angle: -0.8, vel: 0.2 });
                        start_loop();
                    })
                    .forget();
                }
            }
            set_cue.set(Some(Cue::TidyUp));
            play_sound("pin");
            sync();
        }
    };

    // Dragging: the strip follows the pointer; the release decides. Velocity
    // is measured over the last ~120 ms of pointer samples.
    let samples: Rc<RefCell<Vec<(f64, f64, f64)>>> = Rc::new(RefCell::new(Vec::new()));

    let strip_pointer_down = {
        let samples = samples.clone();
        let phys = phys.clone();
        move |ev: leptos::ev::PointerEvent| {
            let Some(target) = ev
                .current_target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            else {
                return;
            };
            let Some(id) = target.get_attribute("data-strip-id") else {
                return;
            };
            ev.prevent_default();
            let _ = target.set_pointer_capture(ev.pointer_id());
            samples.borrow_mut().clear();
            samples
                .borrow_mut()
                .push((js_sys::Date::now(), f64::from(ev.client_x()), f64::from(ev.client_y())));
            phys.borrow_mut().remove(&id);
            set_dragging.set(Some(id));
        }
    };

    let strip_pointer_move = {
        let samples = samples.clone();
        let nodes = nodes.clone();
        let board_ref = board_ref;
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
            if let Some(el) = id.as_deref().and_then(|id| nodes.borrow().get(id)) {
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
        let snapshot = snapshot;
        let set_snapshot = set_snapshot;
        let set_error = set_error;
        let view_settings = view_settings;
        let set_cue = set_cue;
        let phys = phys.clone();
        let nodes = nodes.clone();
        let board_ref = board_ref;
        let bin_ref = bin_ref;
        let board_w = board_w.clone();
        let start_loop = start_loop.clone();
        let sync = sync_transforms.clone();
        move |ev: leptos::ev::PointerEvent| {
            let Some(id) = dragging.get_untracked() else {
                return;
            };
            let (client_x, client_y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
            let samples = samples.borrow().clone();
            let (vx, vy) = match (samples.first(), samples.last()) {
                (Some((t0, x0, y0)), Some((t1, x1, y1))) if t1 > t0 => {
                    ((x1 - x0) * 1000.0 / (t1 - t0), (y1 - y0) * 1000.0 / (t1 - t0))
                }
                _ => (0.0, 0.0),
            };

            let board_rect = board_ref
                .get()
                .map(|el| dom_rect(el.get_bounding_client_rect()))
                .unwrap_or(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
            let list_rect = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|doc| {
                    doc.query_selector(".tasks-pane")
                        .ok()
                        .flatten()
                        .or_else(|| doc.query_selector("main").ok().flatten())
                })
                .map(|el| dom_rect(el.get_bounding_client_rect()))
                .unwrap_or(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
            // An impossible rect when the bin is not on screen.
            let bin_rect = bin_ref
                .get()
                .map(|el| dom_rect(el.get_bounding_client_rect()))
                .unwrap_or(Rect { x: -10.0, y: -10.0, w: 0.0, h: 0.0 });

            set_dragging.set(None);

            match drop_action(client_x, client_y, vx, vy, &list_rect, &board_rect, &bin_rect) {
                DropAction::Restore => {
                    let body = snapshot.with_untracked(|s| crate::toggle_done_body(s, &id));
                    let sink = crate::EditSink { set_snapshot, set_error, view_settings };
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
                            Phys { angle: rest + impulse * 0.15, vel: impulse },
                        );
                        start_loop();
                    }
                    sync();
                }
                DropAction::Crumple => {
                    set_cue.set(Some(Cue::Crumpled));
                    play_sound("crumple");
                    // Crumple animation first, delete under it; the strip
                    // leaves local state with the animation, and the server's
                    // list once the DELETE lands.
                    if let Some(el) = nodes.borrow().get(&id) {
                        let arr = js_sys::Array::new();
                        arr.push(&"crumpling".into());
                        let _ = el.class_list().add(&arr);
                    }
                    let nodes = nodes.clone();
                    let set_error = set_error;
                    let set_snapshot = set_snapshot;
                    Timeout::new(360, move || {
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
                                Err(_) => set_error.set(Some(
                                    "Could not delete that task.".to_owned(),
                                )),
                            }
                        });
                    })
                    .forget();
                }
            }
        }
    };

    let toggle_open = {
        let measure = measure.clone();
        move |_| {
            set_open.update(|o| *o = !*o);
            let measure = measure.clone();
            let sync = sync_transforms.clone();
            after_render(move || {
                measure();
                sync();
            });
        }
    };

    let done_count = move || strips.get().iter().filter(|s| !s.stacked).count();

    view! {
        <section
            node_ref=board_ref
            class:tack-board=true
            class:open=move || open.get()
            class:dragging=move || dragging.get().is_some()
            aria-label="Tack board of done tasks"
        >
            <button class="tack-pin" type="button" aria-label="Tidy up: put the set-aside strips back on the tack" on:click=tidy>
                <span class="tack-pin-head" aria-hidden="true"></span>
            </button>
            <div class="tack-strips">
                <For each=move || strips.get() key=|s| s.id.clone() let:strip>
                    {let strip_id = strip.id.clone();
                     let label = format!(
                        "Done: {}. Drag back to the list to restore, onto the bin to delete.",
                        strip.title
                    );
                     let title = strip.title.clone();
                     let nodes = nodes.clone();
                     let r = NodeRef::<leptos::html::Div>::new();
                     let track = move || {
                         if let Some(el) = r.get() {
                             // An HtmlDivElement upcast to HtmlElement: the
                             // cache holds one type for every strip.
                             let el: web_sys::HtmlElement = el.unchecked_into();
                             nodes.borrow_mut().insert(strip_id.clone(), el);
                         }
                     };
                     track();
                     after_render(track.clone());
                     view! {
                        <div
                            node_ref=r
                            class="tack-strip"
                            class:stacked=strip.stacked
                            class:aside=!strip.tacked && !strip.stacked
                            data-strip-id=strip.id.clone()
                            role="button"
                            tabindex="0"
                            aria-label=label
                            title=title
                            on:pointerdown=strip_pointer_down.clone()
                            on:pointermove=strip_pointer_move.clone()
                            on:pointerup=strip_pointer_up.clone()
                            on:pointercancel=strip_pointer_up.clone()
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
    Rect { x: r.left(), y: r.top(), w: r.width(), h: r.height() }
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
        let mut p = Phys { angle: 1.2, vel: 0.0 };
        let mut last_energy = p.energy(0.0);
        let mut steps = 0_u32;
        while !p.settled(0.0) {
            p = p.step(PHYS_DT, 0.0);
            steps += 1;
            if steps % 30 == 0 {
                let e = p.energy(0.0);
                assert!(e <= last_energy + 1e-12, "energy grew: {e} > {last_energy}");
                last_energy = e;
            }
            assert!(steps < 60_000, "never settled");
        }
        assert_eq!(p.angle, 0.0);
        // Firmly damped: settled inside about six seconds.
        assert!(f64::from(steps) * PHYS_DT < 6.0);
    }

    #[test]
    fn a_strip_at_rest_stays_exactly_at_rest() {
        let p = Phys { angle: 0.0, vel: 0.0 };
        assert_eq!(p.step(PHYS_DT, 0.0), p);
        let aside = Phys { angle: 0.21, vel: 0.0 };
        assert_eq!(aside.step(PHYS_DT, 0.21), aside);
    }

    #[test]
    fn the_swing_is_a_real_pendulum_not_a_linear_spring() {
        // A big swing has a longer period than a small one (sin, not its
        // argument): after a quarter of the small-swing period, a strip
        // released from 90° is still well on its way down.
        let mut big = Phys { angle: std::f64::consts::FRAC_PI_2, vel: 0.0 };
        let quarter = (0.2 / PHYS_DT) as usize;
        for _ in 0..quarter {
            big = big.step(PHYS_DT, 0.0);
        }
        assert!(big.angle > 0.9, "linear spring behaviour: {}", big.angle);
    }

    #[test]
    fn the_bin_wins_the_list_restores_and_a_fling_is_a_throw() {
        let list = Rect { x: 0.0, y: 0.0, w: 400.0, h: 700.0 };
        let board = Rect { x: 1100.0, y: 0.0, w: 190.0, h: 700.0 };
        let bin = Rect { x: 1130.0, y: 600.0, w: 60.0, h: 60.0 };
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
        let overlapped_list = Rect { x: 0.0, y: 0.0, w: 390.0, h: 844.0 };
        let phone_board = Rect { x: 0.0, y: 500.0, w: 390.0, h: 344.0 };
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
        let mut strips: Vec<Strip> =
            (0..32).map(|i| strip(&format!("t{i}"), i + 1, false)).collect();
        apply_cap(&mut strips);
        assert_eq!(strips.iter().filter(|s| !s.stacked).count(), MAX_HANGING);
        assert!(strips[0].stacked && strips[1].stacked, "oldest two stack first");
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
        let (kept, missing) = reconcile(strips, &done);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].title, "new name", "titles follow the server");
        assert_eq!(missing, vec![("new".to_owned(), "N".to_owned())]);
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
