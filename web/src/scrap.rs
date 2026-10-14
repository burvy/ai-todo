//! The scrap a swiped-out row leaves in the page corner (DESIGN-DIRECTION §9,
//! "Undo is the scrap"). Tap it within [`SCRAP_UNDO_MS`] and the task is taped
//! back as it was; after that it slides away and the delete stands. Every delete
//! lays one, whichever control made it: the undo belongs to the delete, not the
//! gesture. Its clock only runs while the page is in view.

use std::sync::atomic::{AtomicU32, Ordering};

use chrono::NaiveDate;
use leptos::prelude::*;
use todo_core::{Snapshot, Task};

use crate::api;
use crate::edits::series_note;
use crate::tasks::occurrence_parts;
use crate::tuning::{SCRAP_SLIDE_MS, SCRAP_TICK_MS, SCRAP_UNDO_MS};

/// What taping a scrap back does.
#[derive(Clone, Debug, PartialEq)]
pub enum Undo {
    /// Store the task again, as it was.
    Task(Task),
    /// One day of a repeat was skipped: clear the skip.
    Day { series: String, date: NaiveDate },
}

/// The scrap lying in the corner. A newer delete lays a new scrap over it, and
/// each one's timers only ever clear their own (`ticket`).
#[derive(Clone, Debug, PartialEq)]
pub struct Scrap {
    pub title: String,
    pub undo: Undo,
    /// Sliding away: its time is up and it can no longer be tapped.
    pub leaving: bool,
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

/// Whether a timer laid with `ticket` still owns what lies in the corner.
fn owns(scrap: Option<&Scrap>, ticket: u32) -> bool {
    scrap.is_some_and(|scrap| scrap.ticket == ticket)
}

/// The undo time left after one more tick. It only runs while the page is in view:
/// switch apps and come back, and the scrap is still lying where it was.
fn after_tick(left_ms: u32, hidden: bool) -> u32 {
    match hidden {
        true => left_ms,
        false => left_ms.saturating_sub(SCRAP_TICK_MS),
    }
}

fn page_hidden() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .is_some_and(|document| document.hidden())
}

/// Lay the scrap of a row just deleted, replacing any scrap already there.
pub fn lay(scraps: RwSignal<Option<Scrap>>, title: String, undo: Undo) {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let ticket = NEXT.fetch_add(1, Ordering::Relaxed);
    scraps.set(Some(Scrap {
        title,
        undo,
        leaving: false,
        ticket,
    }));
    let still_ours = move || scraps.with_untracked(|scrap| owns(scrap.as_ref(), ticket));
    wasm_bindgen_futures::spawn_local(async move {
        let mut left_ms = SCRAP_UNDO_MS;
        while left_ms > 0 {
            gloo_timers::future::TimeoutFuture::new(SCRAP_TICK_MS).await;
            if !still_ours() {
                return;
            }
            left_ms = after_tick(left_ms, page_hidden());
        }
        scraps.update(|scrap| {
            if let Some(scrap) = scrap.as_mut() {
                scrap.leaving = true;
            }
        });
        gloo_timers::future::TimeoutFuture::new(SCRAP_SLIDE_MS).await;
        if still_ours() {
            scraps.set(None);
        }
    });
}

/// Delete `task_id` with `delete`, then lay its scrap. The undo is read from the
/// list before the delete, so it tapes the row back as the list held it.
pub async fn delete_and_lay<F>(
    task_id: &todo_core::TaskId,
    snapshot: ReadSignal<Snapshot>,
    scraps: RwSignal<Option<Scrap>>,
    delete: F,
) -> Result<Snapshot, api::ApiError>
where
    F: std::future::Future<Output = Result<Snapshot, api::ApiError>>,
{
    let undo = snapshot.with_untracked(|held| undo_for(held, &task_id.0));
    let result = delete.await;
    if let (Ok(_), Some((title, undo))) = (&result, undo) {
        lay(scraps, title, undo);
    }
    result
}

/// Tape the scrap's task back, then show the list the server answers with.
async fn tape_back(
    undo: Undo,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
) {
    let result = match undo {
        Undo::Task(task) => api::restore_task(&task).await,
        Undo::Day { series, date } => api::clear_occurrence(&series, &date.to_string()).await,
    };
    match result {
        Ok(snapshot) => set_snapshot.set(snapshot),
        Err(error) => set_error.set(Some(series_note(&error))),
    }
}

/// The corner the scrap lies in. Nothing is drawn while there is no scrap.
#[component]
pub fn ScrapCorner(
    scraps: RwSignal<Option<Scrap>>,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
) -> impl IntoView {
    move || {
        scraps.get().map(|scrap| {
            let undo = scrap.undo.clone();
            let label = format!("Tape \"{}\" back", scrap.title);
            view! {
                <button
                    type="button"
                    class="scrap-undo"
                    class:leaving=scrap.leaving
                    style:--scrap-ms=format!("{SCRAP_SLIDE_MS}ms")
                    aria-label=label
                    disabled=scrap.leaving
                    on:click=move |_| {
                        scraps.set(None);
                        let undo = undo.clone();
                        wasm_bindgen_futures::spawn_local(tape_back(undo, set_snapshot, set_error));
                    }
                >
                    <s>{scrap.title.clone()}</s>
                    <small>"Tap to tape it back"</small>
                </button>
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_timer_only_clears_its_own_scrap() {
        let scrap = |ticket| Scrap {
            title: "Gym".into(),
            undo: Undo::Day {
                series: "s".into(),
                date: NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            },
            leaving: false,
            ticket,
        };
        assert!(owns(Some(&scrap(3)), 3));
        // A newer delete laid its own scrap: the older timer leaves it be.
        assert!(!owns(Some(&scrap(4)), 3));
        assert!(!owns(None, 3), "already taped back");
    }
}
