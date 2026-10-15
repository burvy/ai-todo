//! Moving a task Rambo was asked to move (the ask bubble's Move): the planner waits for a
//! tap on an hour, and Rambo offers the next free time as one tap.

use chrono::Duration;
use leptos::prelude::*;
use todo_core::{Snapshot, TaskId};

use crate::clock;
use crate::settings::Settings;
use crate::week::planner_context;
pub(crate) use rambl_web_core::placing::*;
pub(crate) use rambl_web_core::planner::Slot;

/// The note at the planner's top while a task waits to be placed: what to do, Rambo's
/// offer of the next free time, and a way to leave the task where it was. Esc leaves
/// it too.
#[component]
pub(crate) fn PlacingNote(
    snapshot: ReadSignal<Snapshot>,
    settings: ReadSignal<Settings>,
    /// The task waiting to be placed, held by the planner; Esc or "leave it" clears it.
    to_place: RwSignal<Option<TaskId>>,
    /// Puts the waiting task at a slot.
    place: Callback<Slot>,
) -> impl IntoView {
    let keys = StoredValue::new(None::<WindowListenerHandle>);
    let drop_keys = move || {
        if let Some(key) = keys.try_update_value(Option::take).flatten() {
            key.remove();
        }
    };
    Effect::new(move |_| {
        drop_keys();
        if to_place.with(Option::is_some) {
            let key = window_event_listener(leptos::ev::keydown, move |ev| {
                if ev.key() == "Escape" {
                    to_place.set(None);
                }
            });
            keys.set_value(Some(key));
        }
    });
    on_cleanup(drop_keys);
    let waiting = move || {
        let id = to_place.get()?;
        let snapshot = snapshot.get();
        let task = snapshot.tasks.iter().find(|task| task.id == id)?;
        let s = settings.get();
        let ctx = planner_context(&snapshot.tz);
        let offer = next_free(
            &snapshot.plan.placements,
            &id,
            task.duration_min,
            &ctx,
            (s.start_hour, s.end_hour),
            s.days,
        )
        .map(|slot| {
            let weekday = (ctx.today() + Duration::days(slot.day))
                .format("%a")
                .to_string();
            (
                slot,
                clock::format_day_time(&weekday, slot.minutes, s.clock),
            )
        });
        Some((task.title.clone(), offer))
    };
    // The note stays on the page and keeps its last words, so it can slide away with
    // them once the task has its place instead of vanishing.
    let shown = RwSignal::new(None);
    Effect::new(move |_| {
        if let Some(words) = waiting() {
            shown.set(Some(words));
        }
    });
    let open = move || to_place.with(Option::is_some);
    // How far the note is in sight: it slides up into it and back down out of it.
    let shown_by = RwSignal::new(0.0_f64);
    Effect::new(move |_| {
        let to = if open() { 1.0 } else { 0.0 };
        if shown_by.get_untracked() != to {
            crate::slide::show("placing-note", shown_by, to, || {});
        }
    });
    view! {
        <div
            class="placing-note"
            class:gone=move || shown_by.get() == 0.0
            style:--shown=move || format!("{:.4}", shown_by.get())
            inert=move || !open()
            role="status"
        >
            {move || shown.get().map(|(title, offer)| view! {
                <span class="ask-question">"Where does "<q>{title}</q>" go? Tap an hour."</span>
                <span class="ask-choices">
                    {offer.map(|(slot, when)| view! {
                        <button type="button" class="ask-choice" on:click=move |_| place.run(slot)>
                            {format!("Next free: {when}")}
                        </button>
                    })}
                    <button type="button" class="ask-choice" on:click=move |_| to_place.set(None)>
                        "Leave it"
                    </button>
                </span>
            })}
        </div>
    }
}
