//! Moving a task Rambo was asked to move (the ask bubble's Move): the planner waits for a
//! tap on an hour, and Rambo offers the next free time as one tap.

use chrono::Duration;
use leptos::prelude::*;
use todo_core::{LocalContext, Placement, Snapshot, TaskId};

use crate::clock;
use crate::settings::Settings;
use crate::week::{Slot, drop_start, planner_context};

/// The step a free time is looked for in, and the shortest stretch it makes room for.
const QUARTER: u16 = 15;

/// The earliest slot from now on where `minutes` fit: a quarter-hour start inside the
/// working hours `start_hour..end_hour` of one of the planner's `days`, overlapping no
/// placement but `moving`'s own. `None` when the planner has no such room.
pub(crate) fn next_free(
    placements: &[Placement],
    moving: &TaskId,
    minutes: u16,
    ctx: &LocalContext,
    (start_hour, end_hour): (u8, u8),
    days: u8,
) -> Option<Slot> {
    let minutes = minutes.max(QUARTER);
    let length = Duration::minutes(i64::from(minutes));
    let (open, close) = (u16::from(start_hour) * 60, u16::from(end_hour) * 60);
    let free = |start: chrono::DateTime<chrono::Utc>| {
        start >= ctx.now
            && placements
                .iter()
                .filter(|placement| placement.task != *moving)
                .all(|placement| placement.end <= start || placement.start >= start + length)
    };
    (0..i64::from(days))
        .flat_map(|day| {
            (open..close.saturating_sub(minutes).saturating_add(1))
                .step_by(usize::from(QUARTER))
                .map(move |minutes| Slot { day, minutes })
        })
        .find(|slot| drop_start(ctx, slot.day, slot.minutes).is_some_and(free))
}

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
    view! {
        {move || waiting().map(|(title, offer)| view! {
            <div class="placing-note" role="status">
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
            </div>
        })}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone as _, Utc};

    fn berlin(now: DateTime<Utc>) -> LocalContext {
        LocalContext {
            now,
            tz: chrono_tz::Europe::Berlin,
        }
    }

    fn slot(day: i64, minutes: u16) -> Slot {
        Slot { day, minutes }
    }

    fn at(ctx: &LocalContext, slot: Slot) -> DateTime<Utc> {
        drop_start(ctx, slot.day, slot.minutes).expect("a real local time")
    }

    fn placed(task: &str, start: DateTime<Utc>, minutes: i64) -> Placement {
        Placement {
            task: TaskId(task.into()),
            start,
            end: start + Duration::minutes(minutes),
        }
    }

    /// A small seeded generator, so each run of the property sees the same weeks.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % n
        }
    }

    #[test]
    fn the_next_free_time_is_free_in_hours_ahead_and_the_earliest_there_is() {
        let moving = TaskId("moving".into());
        let mut rng = Lcg(7);
        for _ in 0..300 {
            // Monday 2026-10-12, somewhere between 06:00 and 21:59 in Berlin.
            let now = Utc.with_ymd_and_hms(2026, 10, 12, 4, 0, 0).unwrap()
                + Duration::minutes(rng.below(16 * 60) as i64);
            let ctx = berlin(now);
            let hours = (8 + rng.below(2) as u8, 17 + rng.below(3) as u8);
            let days = 1 + rng.below(4) as u8;
            let minutes = [15, 30, 45, 60, 90, 120][rng.below(6) as usize];
            let mut placements: Vec<_> = (0..rng.below(14))
                .map(|i| {
                    let start = at(
                        &ctx,
                        slot(rng.below(4) as i64, 8 * 60 + rng.below(40) as u16 * 15),
                    );
                    placed(&format!("t{i}"), start, 15 * (1 + rng.below(8) as i64))
                })
                .collect();
            // Its own slip never blocks it.
            placements.push(placed("moving", now, 600));
            let fits = |slot: Slot| {
                let start = at(&ctx, slot);
                slot.minutes >= u16::from(hours.0) * 60
                    && slot.minutes + minutes <= u16::from(hours.1) * 60
                    && start >= now
                    && placements.iter().filter(|p| p.task != moving).all(|p| {
                        p.end <= start || p.start >= start + Duration::minutes(i64::from(minutes))
                    })
            };
            // Every quarter hour of every day, earliest first.
            let earliest = (0..i64::from(days))
                .flat_map(|day| (0..24 * 4).map(move |q| slot(day, q * 15)))
                .find(|slot| fits(*slot));
            let offer = next_free(&placements, &moving, minutes, &ctx, hours, days);
            assert_eq!(offer, earliest, "{placements:?} {hours:?} {minutes}");
        }
    }

    #[test]
    fn a_full_planner_offers_nothing() {
        let ctx = berlin(Utc.with_ymd_and_hms(2026, 10, 12, 6, 0, 0).unwrap());
        let busy = [placed("other", at(&ctx, slot(0, 8 * 60)), 9 * 60)];
        let moving = TaskId("moving".into());
        assert_eq!(next_free(&busy, &moving, 30, &ctx, (8, 17), 1), None);
        assert_eq!(
            next_free(&busy, &moving, 30, &ctx, (8, 17), 2),
            Some(slot(1, 8 * 60)),
            "the next day's first hour"
        );
    }

    #[test]
    fn a_task_with_no_length_still_gets_a_quarter_hour() {
        let ctx = berlin(Utc.with_ymd_and_hms(2026, 10, 12, 6, 0, 0).unwrap());
        // 08:00 to 16:50 busy: a 0-minute task needs a whole quarter, so 16:50 will not do.
        let busy = placed("other", at(&ctx, slot(0, 8 * 60)), 8 * 60 + 50);
        let moving = TaskId("moving".into());
        assert_eq!(next_free(&[busy], &moving, 0, &ctx, (8, 17), 1), None);
    }
}
