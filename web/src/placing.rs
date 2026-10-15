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
    move_request: RwSignal<Option<TaskId>>,
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
        if move_request.with(Option::is_some) {
            let key = window_event_listener(leptos::ev::keydown, move |ev| {
                if ev.key() == "Escape" {
                    move_request.set(None);
                }
            });
            keys.set_value(Some(key));
        }
    });
    on_cleanup(drop_keys);
    let waiting = move || {
        let id = move_request.get()?;
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
    let open = move || move_request.with(Option::is_some);
    view! {
        <div class="placing-note" class:open=open inert=move || !open() role="status">
            {move || shown.get().map(|(title, offer)| view! {
                <span class="ask-question">"Where does "<q>{title}</q>" go? Tap an hour."</span>
                <span class="ask-choices">
                    {offer.map(|(slot, when)| view! {
                        <button type="button" class="ask-choice" on:click=move |_| place.run(slot)>
                            {format!("Next free: {when}")}
                        </button>
                    })}
                    <button type="button" class="ask-choice" on:click=move |_| move_request.set(None)>
                        "Leave it"
                    </button>
                </span>
            })}
        </div>
    }
}
