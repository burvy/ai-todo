//! The week planner's model (ARCHITECTURE 8A.4, step 4): what a slip's grip and a drop
//! on the planner do, decided here without the DOM so `cargo test` covers it.
//! `web/src/week.rs` measures the page, asks these, and does what they answer.

use chrono::{DateTime, Timelike, Utc};
use todo_core::{LocalContext, Snapshot, TaskId, When};

use crate::body::PatchTaskBody;
use crate::placing::{Stretch, stretched};
use crate::planner::{Slot, day_index, drop_start, occurrence_of, placement_style, snapped_minute};
use crate::store::Command;
use crate::tuning::{HOUR_PX, PRESS_SLOP_PX};

/// A slip lifted onto a finger: which task, its title, where the finger is, and where
/// on the slip it took hold. Where it may land is drawn from the planner under the
/// finger; the drop's edit guard is looked up when it lands, so a fresher edit wins.
#[derive(Clone, Debug, PartialEq)]
pub struct SlipDrag {
    pub id: TaskId,
    pub title: String,
    pub x: f64,
    pub y: f64,
    /// The finger's offset from the slip's top-left corner when it was lifted.
    pub grab: (f64, f64),
}

impl SlipDrag {
    /// The ghost's top-left corner in viewport pixels: the slip stays under the
    /// finger at the very spot it was taken.
    pub fn ghost_corner(&self) -> (f64, f64) {
        (self.x - self.grab.0, self.y - self.grab.1)
    }

    /// The point whose time the slip lands on: the ghost's top edge, in the day column
    /// under the finger, so a slip let go where it was taken lands where it sat however
    /// low on it the finger took hold.
    pub fn landing_point(&self) -> (f64, f64) {
        (self.x, self.y - self.grab.1)
    }
}

/// A slip's ghost let go: the slip it was, the slot it goes to (`None`: back to the
/// slip's own place), and where it is drawn on its way. Paper, not a picture: it springs
/// to where it goes and stays there until the slip is drawn under it.
#[derive(Clone, Debug, PartialEq)]
pub struct LetGoGhost {
    pub drag: SlipDrag,
    pub to: Option<Slot>,
    pub at: (f64, f64),
}

/// Where a slip placed at `minutes` starts on the screen, in a day whose hours box has
/// its top-left corner at `hours`, in a working day from `start_hour` to `end_hour`.
pub fn slot_corner(hours: (f64, f64), minutes: u16, start_hour: u8, end_hour: u8) -> (f64, f64) {
    let (top, _) = placement_style(minutes, minutes, start_hour, HOUR_PX, end_hour);
    (hours.0, hours.1 + f64::from(top))
}

/// Whether a finger that has not lifted its slip yet, having slid `slide` px in all, is
/// a scroll: past `PRESS_SLOP_PX` the hold is off and the paper pans; within it, a
/// tremble, and the hold stays armed.
pub fn press_scrolls(slide: f64) -> bool {
    slide > PRESS_SLOP_PX
}

/// Where a slip that starts at `start` goes when let go over `landing`: nowhere off
/// the planner, nor where it already sits. A drop lands on a quarter hour, so a slip
/// off the quarter (one planned at 9:04) sits on the quarter it rounds to.
pub fn slip_moves_to(
    start: DateTime<Utc>,
    ctx: &LocalContext,
    landing: Option<Slot>,
) -> Option<Slot> {
    let local = ctx.local(start);
    let current = Slot {
        day: day_index(start, ctx),
        minutes: snapped_minute(f32::from(local.hour() as u16 * 60 + local.minute() as u16)) as u16,
    };
    landing.filter(|slot| *slot != current)
}

/// What the page says when a slip is dropped on a time the clock skips.
pub const SKIPPED_TIME_NOTE: &str =
    "The clock skips that time that day (daylight change), so it cannot start then.";

/// What placing a task at a slot sends.
#[derive(Clone, Debug, PartialEq)]
pub enum Placed {
    /// This command.
    Send(Command),
    /// Nothing: the clock skips that time that day (`SKIPPED_TIME_NOTE`).
    SkippedTime,
    /// Nothing: the task is gone from the list.
    Gone,
}

/// Placing task `id` at `slot`. One day of a repeating series is a moved exception on
/// the series, never a task patch: its row is one expanded day of the whole rule, and
/// its id names the original local date. A task is patched to start then, back on the
/// planner if it was taken off, guarded by the edit time the page last saw, so the
/// server refuses the move if the task changed elsewhere meanwhile.
pub fn placed(snapshot: &Snapshot, ctx: &LocalContext, id: &TaskId, slot: Slot) -> Placed {
    let Some(instant) = drop_start(ctx, slot.day, slot.minutes) else {
        return Placed::SkippedTime;
    };
    let Some(updated_at) = edit_guard(snapshot, id) else {
        return Placed::Gone;
    };
    Placed::Send(match occurrence_of(id) {
        Some((series, date)) => Command::MoveOccurrence {
            series,
            date,
            to: When::At(instant),
        },
        None => Command::Patch(
            id.clone(),
            PatchTaskBody::new()
                .when(When::At(instant))
                .off_planner(false)
                .updated_at(updated_at),
        ),
    })
}

/// The edit time of task `id` as the page last saw it: the guard a change to it sends.
pub fn edit_guard(snapshot: &Snapshot, id: &TaskId) -> Option<DateTime<Utc>> {
    snapshot
        .tasks
        .iter()
        .find(|task| task.id == *id)
        .map(|task| task.updated_at)
}

/// A slip made a quarter hour longer or shorter: its length now and after, in minutes,
/// and the edit guard the change sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stretched {
    pub from: u16,
    pub to: u16,
    pub updated_at: DateTime<Utc>,
}

/// Stretching task `id`'s slip: `None` when the task is gone or its length is already
/// at the limit `stretch` goes toward.
pub fn stretch_of(snapshot: &Snapshot, id: &TaskId, stretch: Stretch) -> Option<Stretched> {
    let task = snapshot.tasks.iter().find(|task| task.id == *id)?;
    let to = stretched(task.duration_min, stretch);
    (to != task.duration_min).then_some(Stretched {
        from: task.duration_min,
        to,
        updated_at: task.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn fixture() -> Snapshot {
        serde_json::from_str(include_str!("../../../web/fixtures/snapshot.json")).expect("fixture")
    }

    /// The fixture's week, in its zone, on Thursday 15 October 2026.
    fn ctx() -> LocalContext {
        LocalContext {
            now: Utc
                .with_ymd_and_hms(2026, 10, 15, 13, 0, 0)
                .single()
                .expect("a plain instant"),
            tz: chrono_tz::America::New_York,
        }
    }

    fn id(id: &str) -> TaskId {
        TaskId(id.into())
    }

    #[test]
    fn a_dragged_slip_stays_under_the_finger_where_it_was_taken() {
        let drag = |x, y, grab| SlipDrag {
            id: id("t1"),
            title: "Lab report".into(),
            x,
            y,
            grab,
        };
        // Taken 12px in and 16px down, the ghost's corner keeps that offset wherever
        // the finger goes, including off the planner's left and top edges.
        for (x, y) in [(575.0, 174.0), (1745.0, 1000.0), (5.0, 3.0)] {
            let (left, top) = drag(x, y, (12.0, 16.0)).ghost_corner();
            assert_eq!((x - left, y - top), (12.0, 16.0));
        }
        assert_eq!(drag(40.0, 50.0, (0.0, 0.0)).ghost_corner(), (40.0, 50.0));
    }

    #[test]
    fn a_slot_starts_where_a_slip_placed_there_would() {
        let hours = (300.0, 120.0);
        assert_eq!(
            slot_corner(hours, 8 * 60, 8, 20),
            hours,
            "the day's first hour"
        );
        assert_eq!(
            slot_corner(hours, 9 * 60 + 30, 8, 20),
            (300.0, 120.0 + 1.5 * f64::from(HOUR_PX))
        );
        assert_eq!(
            slot_corner(hours, 7 * 60, 8, 20),
            hours,
            "before the day starts it is held at its top, as the slip is"
        );
    }

    #[test]
    fn a_tremble_keeps_the_hold_and_a_slide_is_a_scroll() {
        assert!(!press_scrolls(0.0));
        assert!(
            !press_scrolls(PRESS_SLOP_PX),
            "at the slop: still a tremble"
        );
        assert!(press_scrolls(PRESS_SLOP_PX + 0.5));
    }

    #[test]
    fn a_slip_let_go_where_it_sits_or_off_the_planner_goes_nowhere() {
        let ctx = ctx();
        let snapshot = fixture();
        let history = &snapshot.plan.placements[0];
        let start = history.start;
        let here = Slot {
            day: day_index(start, &ctx),
            minutes: 10 * 60,
        };
        assert_eq!(
            ctx.local(start).hour(),
            10,
            "the fixture's slip starts at 10:00"
        );
        assert_eq!(
            slip_moves_to(start, &ctx, Some(here)),
            None,
            "where it sits"
        );
        assert_eq!(slip_moves_to(start, &ctx, None), None, "off the planner");
        let later = Slot {
            minutes: here.minutes + 15,
            ..here
        };
        assert_eq!(slip_moves_to(start, &ctx, Some(later)), Some(later));
        let off_quarter = start + chrono::Duration::minutes(4);
        assert_eq!(
            slip_moves_to(off_quarter, &ctx, Some(here)),
            None,
            "a slip at 10:04 dropped on 10:00 is where it sits"
        );
        assert_eq!(slip_moves_to(off_quarter, &ctx, Some(later)), Some(later));
    }

    #[test]
    fn a_slip_lands_where_its_top_is_not_where_the_finger_is() {
        let held = SlipDrag {
            id: TaskId("t".into()),
            title: String::new(),
            x: 310.0,
            y: 250.0,
            grab: (10.0, 50.0),
        };
        assert_eq!(held.landing_point(), (310.0, 200.0));
        assert_eq!(held.landing_point().1, held.ghost_corner().1);
    }

    #[test]
    fn a_task_is_patched_to_start_there_back_on_the_planner() {
        let ctx = ctx();
        let snapshot = fixture();
        let slot = Slot {
            day: 1,
            minutes: 9 * 60,
        };
        let instant = drop_start(&ctx, slot.day, slot.minutes).expect("a plain hour");
        let guard = edit_guard(&snapshot, &id("history")).expect("in the fixture");
        assert_eq!(
            placed(&snapshot, &ctx, &id("history"), slot),
            Placed::Send(Command::Patch(
                id("history"),
                PatchTaskBody::new()
                    .when(When::At(instant))
                    .off_planner(false)
                    .updated_at(guard)
            ))
        );
    }

    #[test]
    fn one_day_of_a_series_moves_as_an_exception_on_the_series() {
        let ctx = ctx();
        let slot = Slot {
            day: 2,
            minutes: 18 * 60,
        };
        let Placed::Send(Command::MoveOccurrence { series, date, to }) =
            placed(&fixture(), &ctx, &id("sgym:2026-10-15"), slot)
        else {
            panic!("a moved day of the series")
        };
        assert_eq!(series, "sgym");
        assert_eq!(date.to_string(), "2026-10-15", "the day's original date");
        assert_eq!(
            to,
            When::At(drop_start(&ctx, slot.day, slot.minutes).expect("a plain hour"))
        );
    }

    #[test]
    fn a_time_the_clock_skips_sends_nothing() {
        // New York springs from 02:00 to 03:00 on Sunday 14 March 2027.
        let ctx = ctx();
        let jump = chrono::NaiveDate::from_ymd_opt(2027, 3, 14).expect("a date");
        let slot = Slot {
            day: (jump - ctx.today()).num_days(),
            minutes: 2 * 60 + 30,
        };
        assert_eq!(
            placed(&fixture(), &ctx, &id("history"), slot),
            Placed::SkippedTime
        );
    }

    #[test]
    fn a_gone_task_sends_nothing() {
        let slot = Slot {
            day: 1,
            minutes: 9 * 60,
        };
        assert_eq!(placed(&fixture(), &ctx(), &id("nope"), slot), Placed::Gone);
    }

    #[test]
    fn a_stretch_at_its_limit_changes_nothing() {
        let snapshot = fixture();
        let ink = id("ink");
        let longer = stretch_of(&snapshot, &ink, Stretch::Longer).expect("room to grow");
        assert_eq!((longer.from, longer.to), (30, 45));
        assert_eq!(Some(longer.updated_at), edit_guard(&snapshot, &ink));
        let mut short = snapshot.clone();
        short
            .tasks
            .iter_mut()
            .for_each(|task| task.duration_min = 15);
        assert_eq!(
            stretch_of(&short, &ink, Stretch::Shorter),
            None,
            "already shortest"
        );
        assert_eq!(stretch_of(&snapshot, &id("nope"), Stretch::Longer), None);
    }
}
