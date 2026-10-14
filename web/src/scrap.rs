//! The scrap a swiped-out row leaves in the page corner (DESIGN-DIRECTION §9,
//! "Undo is the scrap"). Tap it within [`SCRAP_UNDO_MS`] and the task is taped
//! back as it was; after that it slides away and the delete stands. Every delete
//! lays one, whichever control made it: the undo belongs to the delete, not the
//! gesture. Scraps pile up, newest at the bottom, up to [`SCRAP_MAX`]: two quick
//! deletes leave two, each taping back its own task. A scrap's clock only runs
//! while the page is in view.

use std::sync::atomic::{AtomicU32, Ordering};

use chrono::NaiveDate;
use leptos::prelude::*;
use todo_core::{Snapshot, Task};

use crate::api;
use crate::edits::series_note;
use crate::tasks::occurrence_parts;
use crate::tuning::{SCRAP_MAX, SCRAP_SLIDE_MS, SCRAP_TICK_MS, SCRAP_UNDO_MS};

/// What taping a scrap back does.
#[derive(Clone, Debug, PartialEq)]
pub enum Undo {
    /// Store the task again, as it was.
    Task(Task),
    /// One day of a repeat was skipped: clear the skip.
    Day { series: String, date: NaiveDate },
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
pub fn lay(scraps: RwSignal<Vec<Scrap>>, title: String, undo: Undo) {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let ticket = NEXT.fetch_add(1, Ordering::Relaxed);
    let scrap = Scrap {
        title,
        undo,
        leaving: false,
        taping: false,
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

/// Delete `task_id` with `delete`, then lay its scrap. The undo is read from the
/// list before the delete, so it tapes the row back as the list held it.
pub async fn delete_and_lay<F>(
    task_id: &todo_core::TaskId,
    snapshot: ReadSignal<Snapshot>,
    scraps: RwSignal<Vec<Scrap>>,
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

/// Tape the scrap's task back, then show the list the server answers with. The
/// scrap stays until the server has the task again, so a refused tape-back keeps
/// the undo.
async fn tape_back(
    scrap: Scrap,
    scraps: RwSignal<Vec<Scrap>>,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
) {
    let result = match scrap.undo {
        Undo::Task(task) => api::restore_task(&task).await,
        Undo::Day { series, date } => api::clear_occurrence(&series, &date.to_string()).await,
    };
    scraps.update(|pile| *pile = after_tape(std::mem::take(pile), scrap.ticket, result.is_ok()));
    match result {
        Ok(snapshot) => set_snapshot.set(snapshot),
        Err(error) => set_error.set(Some(series_note(&error))),
    }
}

/// The corner the scraps lie in. Nothing is drawn while there is none. Each scrap is
/// kept by its ticket, so a new one sliding in does not replay the others' entrance.
#[component]
pub fn ScrapCorner(
    scraps: RwSignal<Vec<Scrap>>,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <div class="scrap-pile">
            <For
                each=move || scraps.get()
                key=|scrap| scrap.ticket
                children=move |scrap| {
                    let ticket = scrap.ticket;
                    let state = move |read: fn(&Scrap) -> bool| {
                        scraps.with(|pile| pile.iter().any(|s| s.ticket == ticket && read(s)))
                    };
                    let label = format!("Tape \"{}\" back", scrap.title);
                    view! {
                        <button
                            type="button"
                            class="scrap-undo"
                            class:leaving=move || state(|s| s.leaving)
                            class:taping=move || state(|s| s.taping)
                            style:--scrap-ms=format!("{SCRAP_SLIDE_MS}ms")
                            aria-label=label
                            aria-busy=move || state(|s| s.taping).to_string()
                            disabled=move || state(|s| s.leaving || s.taping)
                            on:click={
                                let tapped = scrap.clone();
                                move |_| {
                                    scraps.update(|pile| edit(pile, ticket, |s| s.taping = true));
                                    wasm_bindgen_futures::spawn_local(tape_back(
                                        tapped.clone(),
                                        scraps,
                                        set_snapshot,
                                        set_error,
                                    ));
                                }
                            }
                        >
                            <s>{scrap.title.clone()}</s>
                            <small>"Tap to tape it back"</small>
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
    fn a_timer_only_clears_its_own_scrap() {
        let pile = vec![scrap(4)];
        assert!(owns(&pile, 4));
        // A timer laid for an older scrap finds it gone and leaves the rest be.
        assert!(!owns(&pile, 3));
        assert!(!owns(&[], 3), "already taped back");
    }
}
