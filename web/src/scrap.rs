//! The scrap a deleted row tears into (DESIGN-DIRECTION §9, "Undo is the scrap"): it flies
//! from where the row was to the pin at the right of the list's page and hangs there,
//! sideways, so a row never just vanishes. Tap it within [`SCRAP_UNDO_MS`] and the task is taped
//! back as it was; after that it slides away and the delete stands. Every delete
//! lays one, whichever control made it: the undo belongs to the delete, not the
//! gesture. Scraps pile up, newest at the bottom, up to [`SCRAP_MAX`]: two quick
//! deletes leave two, each taping back its own task. A scrap's clock only runs
//! while the page is in view.

use std::sync::atomic::{AtomicU32, Ordering};

use chrono::NaiveDate;
use leptos::prelude::*;
use todo_core::{Snapshot, Task, TaskId};
use wasm_bindgen::JsCast;

use crate::events::Queue;
use crate::store::Store;
use crate::tasks::occurrence_parts;
use crate::tuning::{
    BIN_CORNER_FRACTION, BIN_CRUMPLE_MS, PIN_EDGE_PX, PIN_EDGES, PIN_ROOM_PX, ROW_CLOSE_MS,
    SCRAP_MAX, SCRAP_SLIDE_MS, SCRAP_TICK_MS, SCRAP_UNDO_MS, STRIP_DRAG_PX, STRIP_GAP_PX,
    STRIP_H_PX,
};
use rambl_web_core::events::{Event, any_row_back, row_back, row_left};
use rambl_web_core::store::Command;

/// What taping a scrap back does.
#[derive(Clone, Debug, PartialEq)]
pub enum Undo {
    /// Store the task again, as it was.
    Task(Task),
    /// One day of a repeat was skipped: clear the skip.
    Day { series: String, date: NaiveDate },
}

pub use rambl_web_core::events::Spot;

/// How far, in px across and down, a scrap hung at `hung` starts from the middle of the
/// row that stood at `row`: the flight runs from this offset back to nothing.
pub fn fly_offset(row: Spot, hung: Spot) -> (f64, f64) {
    let ((row_x, row_y), (hung_x, hung_y)) = (row.centre(), hung.centre());
    (row_x - hung_x, row_y - hung_y)
}

/// Note where the row `id` stands now, for the strip that is about to hang at the pin.
pub fn note_departure(events: Queue, id: &str) {
    if let Some(from) = row_spot(id) {
        // A newer note of the same row replaces the older one.
        events.take_all(row_left(id));
        events.push(Event::RowLeft {
            id: id.to_owned(),
            from,
        });
    }
}

/// The tick on row `id` is answered: the noted departure is left for its strip, which
/// mounts after this runs (dropping it here sent every strip to the pin without its
/// flight), and dropped when no strip will take it: the tick was refused, or the strip
/// landed past what the pin mounts.
pub fn settle_departure(events: Queue, snapshot: &Snapshot, id: &str) {
    if !hung_at_the_pin(snapshot).iter().any(|hung| hung.id.0 == id) {
        events.take_all(row_left(id));
    }
}

/// Note where the strip of `id` hangs, for its row about to come back to the list.
fn note_return(events: Queue, id: &str, from: Spot) {
    // A newer note of the same row replaces the older one.
    events.take_all(row_back(id));
    events.push(Event::RowBack {
        id: id.to_owned(),
        from,
    });
}

/// The same for a strip taped back: the noted spot is left for the row, which the list
/// lays out a frame after this runs, and dropped when no row will come: the tape was
/// refused, or the task is gone.
fn settle_return(events: Queue, snapshot: &Snapshot, id: &str) {
    if !crate::edits::held(snapshot, id).is_some_and(|task| !task.done) {
        events.take_all(row_back(id));
    }
}

/// Whether a tick on `id` was refused: the row is still listed, not done.
pub fn tick_refused(snapshot: &Snapshot, id: &str) -> bool {
    crate::edits::held(snapshot, id).is_some_and(|task| !task.done)
}

/// Whether a point (`x`, `y`) in a `width` x `height` window is in the bin corner: the
/// bottom-left, where dragging a strip deletes its task for good.
fn in_bin_corner(x: f64, y: f64, width: f64, height: f64) -> bool {
    x <= width * BIN_CORNER_FRACTION && y >= height * (1.0 - BIN_CORNER_FRACTION)
}

/// The window's width and height in px, or nothing when there is no window.
fn viewport() -> Option<(f64, f64)> {
    let window = web_sys::window()?;
    Some((
        window.inner_width().ok()?.as_f64()?,
        window.inner_height().ok()?.as_f64()?,
    ))
}

/// How far below the top strip the strip at `rank` shows: each rank one edge deeper, up to
/// [`PIN_EDGES`]. Deeper ones sit behind the last edge, so none appears all at once.
fn edge_depth(rank: usize) -> usize {
    rank.min(PIN_EDGES)
}

/// A done task hanging at the pin: plain data, drawn as a strip. Its look and flight
/// come from the page's transform, so something else can drive them.
#[derive(Clone, Debug, PartialEq)]
struct Hung {
    id: TaskId,
    title: String,
}

/// The done tasks in the order they hang: newest first, the top of the stack.
fn done_newest_first(snapshot: &Snapshot) -> Vec<&Task> {
    let mut done: Vec<&Task> = snapshot.tasks.iter().filter(|task| task.done).collect();
    done.sort_by_key(|task| std::cmp::Reverse(task.updated_at));
    done
}

/// The done tasks mounted at the pin, newest first: the stack's edges, and two more so a
/// strip that moves up is already there. Older ones stay done and unmounted.
fn hung_at_the_pin(snapshot: &Snapshot) -> Vec<Hung> {
    done_newest_first(snapshot)
        .into_iter()
        .take(PIN_EDGES + 2)
        .map(|task| Hung {
            id: task.id.clone(),
            title: task.title.clone(),
        })
        .collect()
}

/// Where task `id` sits in the stack: 0 is the top. `None` when it is not done.
fn stack_rank(snapshot: &Snapshot, id: &TaskId) -> Option<usize> {
    done_newest_first(snapshot)
        .iter()
        .position(|task| task.id == *id)
}

/// Gives the keyboard to the top strip of the pin, if there is one.
fn focus_top_strip() {
    let top = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| {
            document
                .query_selector(".scrap-undo.done:not([disabled])")
                .ok()
                .flatten()
        })
        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok());
    if let Some(top) = top {
        let _ = top.focus();
    }
}

/// The scraps lying in the corner, oldest first. A newer delete lays a new scrap
/// beside the others, and each one's timers only ever clear their own (`ticket`).
#[derive(Clone, Debug, PartialEq)]
pub struct Scrap {
    pub title: String,
    pub undo: Undo,
    /// Sliding away: its time is up and it can no longer be tapped.
    pub leaving: bool,
    /// Tapped, and waiting for the server to take the task back: its clock stops,
    /// and it stays until the server says yes.
    pub taping: bool,
    /// Where the row it tore from stood, so it can fly from there to its place.
    pub from: Option<Spot>,
    ticket: u32,
}

/// The undo for deleting row `id`, read before the delete from the row as the list
/// holds it. `None` when the list does not hold it.
pub fn undo_for(snapshot: &Snapshot, id: &str) -> Option<(String, Undo)> {
    let task = snapshot.tasks.iter().find(|task| task.id.0 == id)?;
    let undo = match occurrence_parts(id) {
        Some((series, date)) => Undo::Day {
            series: series.to_owned(),
            date,
        },
        None => Undo::Task(task.clone()),
    };
    Some((task.title.clone(), undo))
}

/// Whether a timer laid with `ticket` still has its scrap in the corner.
fn owns(pile: &[Scrap], ticket: u32) -> bool {
    pile.iter().any(|scrap| scrap.ticket == ticket)
}

/// The corner with `scrap` laid at the bottom, and the tickets of the scraps it sent
/// away. Only scraps still lying count against [`SCRAP_MAX`]: the oldest one over is
/// marked leaving, so it slides off like any scrap whose time is up, and is removed
/// later. One being taped back is never pushed out from under the finger.
fn laid(mut pile: Vec<Scrap>, scrap: Scrap) -> (Vec<Scrap>, Vec<u32>) {
    pile.push(scrap);
    let mut pushed = Vec::new();
    while pile.iter().filter(|scrap| !scrap.leaving).count() > SCRAP_MAX {
        let Some(oldest) = pile
            .iter_mut()
            .find(|scrap| !scrap.leaving && !scrap.taping)
        else {
            break;
        };
        oldest.leaving = true;
        pushed.push(oldest.ticket);
    }
    (pile, pushed)
}

/// Change the scrap laid with `ticket`, if it is still there.
fn edit(pile: &mut [Scrap], ticket: u32, change: impl FnOnce(&mut Scrap)) {
    if let Some(scrap) = pile.iter_mut().find(|scrap| scrap.ticket == ticket) {
        change(scrap);
    }
}

/// The undo time left after one more tick. It only runs while the page is in view
/// and no tape-back is waiting: switch apps and come back, and the scrap is still
/// lying where it was.
fn after_tick(left_ms: u32, paused: bool) -> u32 {
    match paused {
        true => left_ms,
        false => left_ms.saturating_sub(SCRAP_TICK_MS),
    }
}

/// What lies in the corner once the server answers the tape-back of scrap `ticket`:
/// taken back, it is gone; refused, it is laid down again to try once more. The
/// other scraps are left as they are.
fn after_tape(mut pile: Vec<Scrap>, ticket: u32, taken_back: bool) -> Vec<Scrap> {
    if taken_back {
        pile.retain(|scrap| scrap.ticket != ticket);
    } else {
        edit(&mut pile, ticket, |scrap| scrap.taping = false);
    }
    pile
}

fn page_hidden() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .is_some_and(|document| document.hidden())
}

/// Lay the scrap of a row just deleted, under the scraps already there.
pub fn lay(scraps: RwSignal<Vec<Scrap>>, title: String, undo: Undo, from: Option<Spot>) {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let ticket = NEXT.fetch_add(1, Ordering::Relaxed);
    let scrap = Scrap {
        title,
        undo,
        leaving: false,
        taping: false,
        from,
        ticket,
    };
    let mut pushed = Vec::new();
    scraps.update(|pile| {
        let (next, away) = laid(std::mem::take(pile), scrap);
        (*pile, pushed) = (next, away);
    });
    for gone in pushed {
        wasm_bindgen_futures::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(SCRAP_SLIDE_MS).await;
            scraps.update(|pile| pile.retain(|scrap| scrap.ticket != gone));
        });
    }
    let still_ours = move || scraps.with_untracked(|pile| owns(pile, ticket));
    wasm_bindgen_futures::spawn_local(async move {
        let mut left_ms = SCRAP_UNDO_MS;
        while left_ms > 0 {
            gloo_timers::future::TimeoutFuture::new(SCRAP_TICK_MS).await;
            if !still_ours() {
                return;
            }
            let taping = scraps.with_untracked(|pile| {
                pile.iter()
                    .any(|scrap| scrap.ticket == ticket && scrap.taping)
            });
            left_ms = after_tick(left_ms, page_hidden() || taping);
        }
        scraps.update(|pile| edit(pile, ticket, |scrap| scrap.leaving = true));
        gloo_timers::future::TimeoutFuture::new(SCRAP_SLIDE_MS).await;
        scraps.update(|pile| pile.retain(|scrap| scrap.ticket != ticket));
    });
}

/// Where the list shows row `id`, if it does.
fn row_spot(id: &str) -> Option<Spot> {
    let row = web_sys::window()?
        .document()?
        .query_selector(&format!("li[data-id=\"{id}\"]"))
        .ok()??;
    let rect = row.get_bounding_client_rect();
    Some(Spot {
        x: rect.left(),
        y: rect.top(),
        w: rect.width(),
        h: rect.height(),
    })
}

/// Fly a scrap that has just been laid from the row it tore from to its place: it starts
/// at the row, turned flat like the row's own line, and the page's transition carries it
/// to where it hangs. Done in one task (move, read, release) so the start is never drawn
/// in the wrong place; with reduced motion it simply hangs. Either way it is pinned with
/// a tack's sound, as it arrives.
fn fly(scrap: &web_sys::HtmlElement, from: Spot) {
    if crate::mascot::prefers_reduced_motion() {
        crate::sound::play(crate::sound::Cue::Pin);
        return;
    }
    set_timeout(
        || crate::sound::play(crate::sound::Cue::Pin),
        std::time::Duration::from_millis(SCRAP_SLIDE_MS.into()),
    );
    let rect = scrap.get_bounding_client_rect();
    let hung = Spot {
        x: rect.left(),
        y: rect.top(),
        w: rect.width(),
        h: rect.height(),
    };
    let (dx, dy) = fly_offset(from, hung);
    let style = scrap.style();
    let _ = style.set_property("--fly-x", &format!("{dx:.1}px"));
    let _ = style.set_property("--fly-y", &format!("{dy:.1}px"));
    let classes = scrap.class_list();
    let _ = classes.add_1("flying");
    // Reading a layout value makes the browser settle the start before the release.
    let _ = scrap.offset_width();
    let _ = classes.remove_1("flying");
}

/// Where each list row sits, as its offset from the list's top in px, in list order.
/// An offset ignores transforms, so a row still sliding reads where it will rest.
pub fn row_tops() -> Vec<(String, i32)> {
    let list = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|doc| doc.query_selector(".task-list").ok().flatten());
    let Some(rows) = list.map(|list| list.children()) else {
        return Vec::new();
    };
    (0..rows.length())
        .filter_map(|n| rows.item(n)?.dyn_into::<web_sys::HtmlElement>().ok())
        .filter_map(|row| Some((row.get_attribute("data-id")?, row.offset_top())))
        .collect()
}

/// The rows that must slide to close the gap a removed row left, or open the room an
/// arriving one needs: how far each one moved (old top minus new top), for rows in both
/// lists that moved at all. Nothing slides when the same rows stand in a new order, so a
/// reorder keeps its own motion.
pub fn rows_to_slide(before: &[(String, i32)], after: &[(String, i32)]) -> Vec<(String, i32)> {
    let ids_changed = before.len() != after.len()
        || before
            .iter()
            .any(|(id, _)| after.iter().all(|(kept, _)| kept != id));
    if !ids_changed {
        return Vec::new();
    }
    after
        .iter()
        .filter_map(|(id, top)| {
            let (_, was) = before.iter().find(|(old, _)| old == id)?;
            (was != top).then(|| (id.clone(), was - top))
        })
        .collect()
}

/// Slide the rows that rest `moved` px lower than they were back up into place: each
/// starts where it stood, and a transition carries it home. With reduced motion they just
/// sit in their new places.
pub fn slide_rows(moved: &[(String, i32)]) {
    if crate::mascot::prefers_reduced_motion() {
        return;
    }
    let Some(doc) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let rows: Vec<web_sys::HtmlElement> = moved
        .iter()
        .filter_map(|(id, up)| {
            let row = doc
                .query_selector(&format!("li[data-id=\"{id}\"]"))
                .ok()??
                .dyn_into::<web_sys::HtmlElement>()
                .ok()?;
            let style = row.style();
            let _ = style.set_property("transition", "none");
            let _ = style.set_property("transform", &format!("translateY({up}px)"));
            Some(row)
        })
        .collect();
    if let Some(row) = rows.first() {
        // Reading a layout value makes the browser settle the starts before the release.
        let _ = row.offset_width();
    }
    for row in rows {
        let style = row.style();
        let _ = style.set_property(
            "transition",
            &format!("transform {ROW_CLOSE_MS}ms var(--ease-out, ease-out)"),
        );
        let _ = style.set_property("transform", "none");
    }
}

/// Fly each row that has just come back from the pin from where its strip hung. A strip
/// not yet answered for (the row is not in the list) is kept for the next look.
pub fn fly_rows_back(events: Queue) {
    let Some(doc) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let motion = !crate::mascot::prefers_reduced_motion();
    let noted = events.take_all(any_row_back);
    let mut flown = Vec::new();
    for (id, strip) in noted {
        let row = doc
            .query_selector(&format!("li[data-id=\"{id}\"]"))
            .ok()
            .flatten()
            .and_then(|row| row.dyn_into::<web_sys::HtmlElement>().ok());
        let Some(row) = row else {
            note_return(events, &id, strip);
            continue;
        };
        if !motion {
            continue;
        }
        let rect = row.get_bounding_client_rect();
        let at = Spot {
            x: rect.left(),
            y: rect.top(),
            w: rect.width(),
            h: rect.height(),
        };
        let (dx, dy) = fly_offset(strip, at);
        let style = row.style();
        let _ = style.set_property("transition", "none");
        let _ = style.set_property("transform", &format!("translate({dx:.1}px, {dy:.1}px)"));
        flown.push(row);
    }
    if let Some(row) = flown.first() {
        let _ = row.offset_width();
    }
    for row in flown {
        let style = row.style();
        let _ = style.set_property(
            "transition",
            &format!("transform {ROW_CLOSE_MS}ms var(--ease-out, ease-out)"),
        );
        let _ = style.set_property("transform", "none");
    }
}

/// A stable number for a task id, so the same strip always hangs the same way.
fn hash_ticket(id: &str) -> u32 {
    id.bytes().fold(7_u32, |sum, byte| {
        sum.wrapping_mul(31).wrapping_add(u32::from(byte))
    })
}

/// How far a scrap hangs off true, in degrees: each swings a little differently.
fn sway_deg(ticket: u32) -> i32 {
    (ticket * 37 % 9) as i32 - 4
}

/// Delete `task_id` with `delete` (which answers whether the server took it), then lay
/// its scrap. The undo is read from the list before the delete, so it tapes the row back
/// as the list held it.
pub async fn delete_and_lay<F>(
    task_id: &todo_core::TaskId,
    snapshot: ReadSignal<Snapshot>,
    scraps: RwSignal<Vec<Scrap>>,
    delete: F,
) -> bool
where
    F: std::future::Future<Output = bool>,
{
    let undo = snapshot.with_untracked(|held| undo_for(held, &task_id.0));
    // Where the row stands now, before the list loses it: the scrap flies from there.
    let from = row_spot(&task_id.0);
    let deleted = delete.await;
    if deleted {
        crate::sound::play(crate::sound::Cue::Delete);
        if let Some((title, undo)) = undo {
            lay(scraps, title, undo, from);
        }
    }
    deleted
}

/// Tape the scrap's task back, then show the list the server answers with. The
/// scrap stays until the server has the task again, so a refused tape-back keeps
/// the undo.
async fn tape_back(scrap: Scrap, scraps: RwSignal<Vec<Scrap>>, store: Store) {
    let command = match scrap.undo {
        Undo::Task(task) => Command::RestoreTask(task),
        Undo::Day { series, date } => Command::ClearOccurrence {
            series,
            date: date.to_string(),
        },
    };
    let taken = store.send(command).await.is_some();
    scraps.update(|pile| *pile = after_tape(std::mem::take(pile), scrap.ticket, taken));
}

/// The pin's side of the list's page, where the scraps hang. Nothing is drawn while there is
/// none. Each scrap is kept by its ticket, so a new one arriving does not replay the others.
#[component]
pub fn ScrapCorner(snapshot: ReadSignal<Snapshot>, scraps: RwSignal<Vec<Scrap>>) -> impl IntoView {
    // Edits go through the store, which shows the reply and explains a refusal.
    let store = expect_context::<Store>();
    let events = expect_context::<Queue>();
    view! {
        <div
            class="scrap-pile"
            style:--strip-h=format!("{STRIP_H_PX}px")
            style:--strip-gap=format!("{STRIP_GAP_PX}px")
            style:--scrap-ms=format!("{SCRAP_SLIDE_MS}ms")
            style:--pin-room=format!("{PIN_ROOM_PX}px")
            style:--edge-px=format!("{PIN_EDGE_PX}px")
            style:--stack-h=format!("{}px", STRIP_H_PX + PIN_EDGE_PX * PIN_EDGES as f64)
        >
            <For
                each=move || snapshot.with(hung_at_the_pin)
                key=|hung| hung.id.clone()
                children=move |hung| {
                    let hung_ref = NodeRef::<leptos::html::Button>::new();
                    let id = hung.id.0.clone();
                    Effect::new(move |flown: Option<bool>| {
                        let ready = hung_ref.get().inspect(|button| {
                            if flown != Some(true)
                                && let Some(from) = events.take(row_left(&id))
                            {
                                fly(button, from);
                            }
                        });
                        ready.is_some()
                    });
                    // Dragging the top strip: it follows the finger; in the bin corner it
                    // leans, and on release there it crumples and its task is deleted.
                    let grip = StoredValue::new(None::<(f64, f64)>);
                    let offset = RwSignal::new((0.0_f64, 0.0_f64));
                    let dragging = RwSignal::new(false);
                    let binned = RwSignal::new(false);
                    let just_dragged = StoredValue::new(false);
                    let task_id = StoredValue::new(hung.id.clone());
                    let deletable = occurrence_parts(&hung.id.0).is_none();
                    let rank_id = hung.id.clone();
                    let rank = Memo::new(move |_| snapshot.with(|held| stack_rank(held, &rank_id)));
                    let top = Memo::new(move |_| rank.get() == Some(0));
                    let label = format!("Tape \"{}\" back: not done after all", hung.title);
                    // Only the top strip carries its title; the edges under it are plain paper.
                    let title = hung.title.clone();
                    let style = format!("{}deg", sway_deg(hash_ticket(&hung.id.0)));
                    view! {
                        <button
                            type="button"
                            class="scrap-undo done"
                            node_ref=hung_ref
                            style:--hang=style
                            style:--drag-x=move || format!("{}px", offset.get().0)
                            style:--drag-y=move || format!("{}px", offset.get().1)
                            class:dragging=move || dragging.get()
                            class:over-bin=move || binned.get() && dragging.get()
                            class:binned=move || binned.get() && !dragging.get()
                            style:--depth=move || edge_depth(rank.get().unwrap_or(PIN_EDGES)).to_string()
                            style:z-index=move || (PIN_EDGES + 1 - edge_depth(rank.get().unwrap_or(PIN_EDGES))).to_string()
                            class:on-top=move || top.get()
                            aria-label=label
                            aria-hidden=move || (!top.get()).to_string()
                            disabled=move || !top.get()
                            title="Tap to tape it back"
                            on:pointerdown=move |event: leptos::ev::PointerEvent| {
                                if top.get_untracked() {
                                    grip.set_value(Some((f64::from(event.client_x()), f64::from(event.client_y()))));
                                    just_dragged.set_value(false);
                                    // Moves go to the strip even once the finger has left it.
                                    if let Some(button) = hung_ref.get_untracked() {
                                        let _ = button.set_pointer_capture(event.pointer_id());
                                    }
                                }
                            }
                            on:pointermove=move |event: leptos::ev::PointerEvent| {
                                let Some((x0, y0)) = grip.get_value() else { return };
                                let (x, y) = (f64::from(event.client_x()), f64::from(event.client_y()));
                                let (dx, dy) = (x - x0, y - y0);
                                if !dragging.get_untracked() {
                                    if dx.hypot(dy) < STRIP_DRAG_PX {
                                        return;
                                    }
                                    dragging.set(true);
                                }
                                offset.set((dx, dy));
                                let over = viewport().is_some_and(|(w, h)| in_bin_corner(x, y, w, h));
                                binned.set(over && deletable);
                            }
                            on:pointerup=move |_| {
                                if grip.get_value().is_none() {
                                    return;
                                }
                                grip.set_value(None);
                                if !dragging.get_untracked() {
                                    return;
                                }
                                dragging.set(false);
                                just_dragged.set_value(true);
                                if binned.get_untracked() {
                                    crate::sound::play(crate::sound::Cue::Crumple);
                                    let id = task_id.get_value();
                                    set_timeout(
                                        move || {
                                            wasm_bindgen_futures::spawn_local(async move {
                                                // Refused: the scrap comes back off the bin.
                                                if store.send(Command::DeleteTask(id)).await.is_none() {
                                                    binned.set(false);
                                                    offset.set((0.0, 0.0));
                                                }
                                            });
                                        },
                                        std::time::Duration::from_millis(BIN_CRUMPLE_MS),
                                    );
                                } else {
                                    offset.set((0.0, 0.0));
                                }
                            }
                            on:pointercancel=move |_| {
                                grip.set_value(None);
                                dragging.set(false);
                                binned.set(false);
                                offset.set((0.0, 0.0));
                            }
                            on:click=move |_| {
                                if just_dragged.get_value() {
                                    just_dragged.set_value(false);
                                    return;
                                }
                                let id = task_id.get_value();
                                let Some(action) = snapshot.with_untracked(|s| crate::tasks::tick_action(s, &id.0)) else {
                                    return;
                                };

                                // The row flies back from where this strip hangs.
                                if let Some(button) = hung_ref.get_untracked() {
                                    let rect = button.get_bounding_client_rect();
                                    note_return(events, &id.0, Spot {
                                        x: rect.left(),
                                        y: rect.top(),
                                        w: rect.width(),
                                        h: rect.height(),
                                    });
                                }
                                wasm_bindgen_futures::spawn_local(async move {
                                    crate::tasks::apply_tick(&id, action, store).await;
                                    snapshot.with_untracked(|s| settle_return(events, s, &id.0));
                                    // The strip is gone: keep the keyboard on the new top strip.
                                    crate::engine::next_frame(focus_top_strip);
                                });
                            }
                        >
                            <s>{title.clone()}</s>
                        </button>
                    }
                }
            />
            <For
                each=move || scraps.get()
                key=|scrap| scrap.ticket
                children=move |scrap| {
                    let ticket = scrap.ticket;
                    let state = move |read: fn(&Scrap) -> bool| {
                        scraps.with(|pile| pile.iter().any(|s| s.ticket == ticket && read(s)))
                    };
                    let label = format!("Tape \"{}\" back", scrap.title);
                    let hung = NodeRef::<leptos::html::Button>::new();
                    if let Some(from) = scrap.from {
                        Effect::new(move |flown: Option<bool>| {
                            let ready = hung.get().inspect(|button| {
                                if flown != Some(true) {
                                    fly(button, from);
                                }
                            });
                            ready.is_some()
                        });
                    }
                    view! {
                        <button
                            type="button"
                            class="scrap-undo"
                            node_ref=hung
                            class:leaving=move || state(|s| s.leaving)
                            class:taping=move || state(|s| s.taping)
                            style:--scrap-ms=format!("{SCRAP_SLIDE_MS}ms")
                            style:--hang=format!("{}deg", sway_deg(ticket))
                            aria-label=label
                            title="Tap to tape it back"
                            aria-busy=move || state(|s| s.taping).to_string()
                            disabled=move || state(|s| s.leaving || s.taping)
                            on:click={
                                let tapped = scrap.clone();
                                move |_| {
                                    scraps.update(|pile| edit(pile, ticket, |s| s.taping = true));
                                    wasm_bindgen_futures::spawn_local(tape_back(
                                        tapped.clone(),
                                        scraps,
                                        store,
                                    ));
                                }
                            }
                        >
                            <s>{scrap.title.clone()}</s>
                        </button>
                    }
                }
            />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api;

    /// A tick that went through leaves the row's departure for its strip, which mounts
    /// after the tick returns; one no strip will take is dropped.
    #[test]
    fn a_done_rows_departure_waits_for_its_strip_unless_the_tick_was_refused() {
        let mut snapshot = api::fixture_snapshot();
        let id = snapshot.tasks[0].id.0.clone();
        let from = Spot {
            x: 1.0,
            y: 2.0,
            w: 3.0,
            h: 4.0,
        };
        let left = || Event::RowLeft {
            id: id.clone(),
            from,
        };
        Owner::new().with(|| {
            let events = Queue::provide();
            snapshot.tasks[0].done = false;
            events.push(left());
            settle_departure(events, &snapshot, &id);
            assert_eq!(events.take(row_left(&id)), None, "refused: dropped");

            snapshot.tasks[0].done = true;
            events.push(left());
            settle_departure(events, &snapshot, &id);
            assert_eq!(
                events.take(row_left(&id)),
                Some(from),
                "done: kept for the strip"
            );

            // Done, but older than every strip the pin mounts: no strip will take it.
            let newer = snapshot.tasks[0].updated_at + chrono::Duration::minutes(1);
            for n in 0..PIN_EDGES + 2 {
                let mut other = snapshot.tasks[0].clone();
                other.id = TaskId(format!("newer-{n}"));
                other.updated_at = newer;
                snapshot.tasks.push(other);
            }
            events.push(left());
            settle_departure(events, &snapshot, &id);
            assert_eq!(events.take(row_left(&id)), None, "past what fits: dropped");

            snapshot.tasks.remove(0);
            events.push(left());
            settle_departure(events, &snapshot, &id);
            assert_eq!(events.take(row_left(&id)), None, "gone: dropped");
        });
    }

    /// The same for a strip taped back: its spot waits for the row, which the list lays
    /// out a frame after the tick returns; one no row will take is dropped.
    #[test]
    fn a_taped_back_rows_spot_waits_for_the_row_unless_the_tick_was_refused() {
        let mut snapshot = api::fixture_snapshot();
        let id = snapshot.tasks[0].id.0.clone();
        let from = Spot {
            x: 5.0,
            y: 6.0,
            w: 7.0,
            h: 8.0,
        };
        Owner::new().with(|| {
            let events = Queue::provide();
            snapshot.tasks[0].done = true;
            note_return(events, &id, from);
            settle_return(events, &snapshot, &id);
            assert_eq!(events.take(row_back(&id)), None, "refused: dropped");

            snapshot.tasks[0].done = false;
            note_return(events, &id, from);
            settle_return(events, &snapshot, &id);
            assert_eq!(
                events.take(row_back(&id)),
                Some(from),
                "back: kept for the row"
            );

            snapshot.tasks.remove(0);
            note_return(events, &id, from);
            settle_return(events, &snapshot, &id);
            assert_eq!(events.take(row_back(&id)), None, "gone: dropped");
        });
    }

    #[test]
    fn a_deleted_task_is_taped_back_unchanged() {
        let snapshot = api::fixture_snapshot();
        let task = snapshot
            .tasks
            .iter()
            .find(|task| occurrence_parts(&task.id.0).is_none())
            .expect("the fixture holds a plain task");
        let (title, undo) = undo_for(&snapshot, &task.id.0).expect("listed");
        assert_eq!(title, task.title);
        assert_eq!(
            undo,
            Undo::Task(task.clone()),
            "every field as the list held it"
        );
        assert!(undo_for(&snapshot, "gone").is_none());
    }

    #[test]
    fn a_repeat_s_day_is_put_back_on_its_series() {
        let mut snapshot = api::fixture_snapshot();
        let mut day = snapshot.tasks[0].clone();
        day.id = todo_core::TaskId("sgym:2026-10-15".into());
        snapshot.tasks.push(day);
        let (_, undo) = undo_for(&snapshot, "sgym:2026-10-15").expect("listed");
        assert_eq!(
            undo,
            Undo::Day {
                series: "sgym".into(),
                date: NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            }
        );
    }

    #[test]
    fn the_undo_clock_stops_while_the_page_is_hidden() {
        assert_eq!(after_tick(SCRAP_UNDO_MS, true), SCRAP_UNDO_MS);
        assert_eq!(
            after_tick(SCRAP_UNDO_MS, false),
            SCRAP_UNDO_MS - SCRAP_TICK_MS
        );
        assert_eq!(after_tick(SCRAP_TICK_MS / 2, false), 0, "never below zero");
    }

    fn scrap(ticket: u32) -> Scrap {
        Scrap {
            title: format!("Task {ticket}"),
            undo: Undo::Day {
                series: "s".into(),
                date: NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            },
            leaving: false,
            taping: false,
            from: None,
            ticket,
        }
    }

    fn tickets(pile: &[Scrap]) -> Vec<u32> {
        pile.iter().map(|scrap| scrap.ticket).collect()
    }

    #[test]
    fn quick_deletes_keep_their_own_scraps_newest_at_the_bottom() {
        let pile = lay_all([1, 2]);
        assert_eq!(tickets(&pile), [1, 2], "both are there, each with its task");
        assert_ne!(pile[0].title, pile[1].title);
    }

    fn lay_all(tickets: impl IntoIterator<Item = u32>) -> Vec<Scrap> {
        tickets
            .into_iter()
            .fold(Vec::new(), |pile, n| laid(pile, scrap(n)).0)
    }

    fn lying(pile: &[Scrap]) -> Vec<u32> {
        pile.iter()
            .filter(|scrap| !scrap.leaving)
            .map(|scrap| scrap.ticket)
            .collect()
    }

    #[test]
    fn a_scrap_over_the_most_slides_away_instead_of_popping() {
        let most = SCRAP_MAX as u32;
        let pile = lay_all(0..most);
        let (pile, pushed) = laid(pile, scrap(most));
        assert_eq!(pushed, [0], "the oldest is sent away");
        assert_eq!(pile.len(), SCRAP_MAX + 1, "still there to slide off");
        assert!(pile[0].leaving, "marked leaving, not removed");
        assert_eq!(lying(&pile).len(), SCRAP_MAX, "only the lying ones count");
        // A leaving scrap does not count: the next one pushes out the next oldest.
        let (pile, pushed) = laid(pile, scrap(most + 1));
        assert_eq!(pushed, [1]);
        assert_eq!(lying(&pile), (2..most + 2).collect::<Vec<_>>());
    }

    #[test]
    fn a_scrap_being_taped_back_is_not_pushed_out() {
        let most = SCRAP_MAX as u32;
        let mut pile = lay_all(0..most);
        pile[0].taping = true;
        let (pile, pushed) = laid(pile, scrap(most));
        assert_eq!(pushed, [1], "the oldest that is not mid tape-back goes");
        assert!(!pile[0].leaving, "the one being taped stays put");
    }

    #[test]
    fn a_refused_tape_back_keeps_the_scrap() {
        let tapped = Scrap {
            taping: true,
            ..scrap(3)
        };
        let pile = vec![scrap(2), tapped.clone()];
        assert_eq!(
            tickets(&after_tape(pile.clone(), 3, true)),
            [2],
            "taken back: only that scrap goes"
        );
        assert_eq!(
            after_tape(pile, 3, false),
            vec![
                scrap(2),
                Scrap {
                    taping: false,
                    ..tapped
                }
            ],
            "refused: laid down again to tap once more, the other untouched"
        );
        assert_eq!(
            after_tick(SCRAP_UNDO_MS, true),
            SCRAP_UNDO_MS,
            "paused while taping"
        );
    }

    #[test]
    fn a_scrap_flies_from_the_middle_of_its_row_to_where_it_hangs() {
        let row = Spot {
            x: 16.0,
            y: 200.0,
            w: 340.0,
            h: 50.0,
        };
        let hung = Spot {
            x: 340.0,
            y: 50.0,
            w: 30.0,
            h: 100.0,
        };
        // The row's middle is (186, 225), the hung scrap's (355, 100).
        assert_eq!(fly_offset(row, hung), (-169.0, 125.0));
        assert_eq!(
            fly_offset(hung, hung),
            (0.0, 0.0),
            "already there: no flight"
        );
    }

    fn done_snapshot(count: i64) -> Snapshot {
        let mut snapshot = api::fixture_snapshot();
        let template = snapshot.tasks[0].clone();
        snapshot.tasks = (0..count)
            .map(|n| {
                let mut task = template.clone();
                task.id = TaskId(format!("t{n}"));
                task.title = format!("Task {n}");
                task.done = true;
                task.updated_at = template.updated_at + chrono::Duration::minutes(n);
                task
            })
            .collect();
        snapshot
    }

    #[test]
    fn the_pin_stacks_every_done_task_newest_on_top() {
        let snapshot = done_snapshot(3);
        let ids: Vec<_> = hung_at_the_pin(&snapshot)
            .iter()
            .map(|h| h.id.0.clone())
            .collect();
        assert_eq!(ids, ["t2", "t1", "t0"]);
        assert_eq!(stack_rank(&snapshot, &TaskId("t2".into())), Some(0));
        assert_eq!(stack_rank(&snapshot, &TaskId("t0".into())), Some(2));
        assert_eq!(stack_rank(&snapshot, &TaskId("gone".into())), None);
    }

    #[test]
    fn only_a_few_edges_show_under_the_top_however_deep_the_stack() {
        assert_eq!(edge_depth(0), 0, "the top sits flat");
        assert_eq!(edge_depth(1), 1);
        assert_eq!(edge_depth(PIN_EDGES), PIN_EDGES);
        assert_eq!(edge_depth(40), PIN_EDGES, "deeper ones share the last edge");
    }

    #[test]
    fn the_pin_mounts_the_edges_and_two_more_and_no_further() {
        let snapshot = done_snapshot(10);
        assert_eq!(hung_at_the_pin(&snapshot).len(), PIN_EDGES + 2);
    }

    #[test]
    fn a_strip_that_is_not_in_the_stack_has_no_rank() {
        let snapshot = done_snapshot(3);
        assert_eq!(stack_rank(&snapshot, &TaskId("gone".into())), None);
    }

    #[test]
    fn only_done_tasks_hang() {
        let mut snapshot = done_snapshot(3);
        snapshot.tasks[1].done = false;
        let hung = hung_at_the_pin(&snapshot);
        assert!(hung.iter().all(|h| h.id.0 != "t1"));
    }

    #[test]
    fn rows_below_an_arriving_row_slide_down_to_make_room() {
        let row = |id: &str, top| (id.to_owned(), top);
        let before = [row("a", 0), row("b", 50)];
        let after = [row("n", 0), row("a", 50), row("b", 100)];
        assert_eq!(
            rows_to_slide(&before, &after),
            [row("a", -50), row("b", -50)]
        );
    }

    #[test]
    fn nothing_slides_when_the_same_rows_are_reordered() {
        let row = |id: &str, top| (id.to_owned(), top);
        let before = [row("a", 0), row("b", 50)];
        let after = [row("b", 0), row("a", 50)];
        assert!(rows_to_slide(&before, &after).is_empty());
    }

    #[test]
    fn the_bin_is_the_bottom_left_corner_and_nothing_else() {
        // A 390x844 phone: the corner is the left 30% of the width, the bottom 30% of the height.
        assert!(in_bin_corner(10.0, 800.0, 390.0, 844.0), "bottom left");
        assert!(!in_bin_corner(300.0, 800.0, 390.0, 844.0), "bottom right");
        assert!(!in_bin_corner(10.0, 100.0, 390.0, 844.0), "top left");
        assert!(in_bin_corner(0.0, 844.0, 390.0, 844.0), "the very corner");
    }

    #[test]
    fn each_scrap_swings_a_little_and_never_far() {
        assert!((0..200).all(|ticket| sway_deg(ticket).abs() <= 4));
        assert_ne!(sway_deg(1), sway_deg(2), "neighbours do not hang alike");
    }

    #[test]
    fn a_timer_only_clears_its_own_scrap() {
        let pile = vec![scrap(4)];
        assert!(owns(&pile, 4));
        // A timer laid for an older scrap finds it gone and leaves the rest be.
        assert!(!owns(&pile, 3));
        assert!(!owns(&[], 3), "already taped back");
    }
}
