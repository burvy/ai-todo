//! The task list's model (ARCHITECTURE 8A.4, step 4): what the list's inputs mean,
//! decided here without the DOM so `cargo test` covers it. `web/src/tasks/mod.rs`
//! measures the page, asks these, and does what they answer.

use chrono::{DateTime, NaiveDate, Utc, Weekday};
use todo_core::{Label, Priority, Series, Snapshot, Source, Task, TaskDraft, TaskId, When};

use crate::mascot::State;
use crate::row::{Gesture, Release, Slot};
use crate::settings::RamblMode;
use crate::tuning;

/// The order the server wants after moving `dragged` to `slot` beside `target`,
/// or `None` when the drop is not a reorder at all.
///
/// The server renumbers stored tasks only (`ids must list every task exactly once`),
/// and a snapshot lists series occurrences too — rows with `{series}:{date}` ids
/// that are expanded on the fly and share the series' row. A slip dragged from or
/// onto one is not a reorder, so the handler skips the call instead of failing 400.
pub fn reorder_ids(
    snapshot: &Snapshot,
    dragged: &TaskId,
    target: &TaskId,
    slot: Slot,
) -> Option<Vec<TaskId>> {
    if dragged == target {
        return None;
    }
    if snapshot
        .tasks
        .iter()
        .any(|task| task.source == Source::Series && (task.id == *dragged || task.id == *target))
    {
        return None;
    }
    let mut new_order: Vec<TaskId> = snapshot
        .tasks
        .iter()
        .filter(|task| task.source != Source::Series && task.id != *dragged)
        .map(|task| task.id.clone())
        .collect();
    // A drop that missed every row lands the slip at the end rather than
    // nowhere, which is where a finger that let go over empty paper expects it.
    let at = match (new_order.iter().position(|id| id == target), slot) {
        (Some(at), Slot::Before) => at,
        (Some(at), Slot::After) => at + 1,
        (None, _) => new_order.len(),
    };
    new_order.insert(at, dragged.clone());
    Some(new_order)
}

/// The two halves of an occurrence row's id, which the server mints as
/// `"{series_id}:{date}"` (docs/ARCHITECTURE.md 7.1). Ordinary task ids carry
/// no date, so they give `None`; a moved occurrence keeps its original date in
/// the id, which is exactly the day an exception names.
pub fn occurrence_parts(id: &str) -> Option<(&str, NaiveDate)> {
    let (series_id, date) = id.split_once(':')?;
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some((series_id, date))
}

/// Whether `task` is overdue at `now`: not done, and its deadline has passed.
pub fn is_overdue_at(task: &Task, now: DateTime<Utc>) -> bool {
    !task.done && matches!(task.when, When::DueBy(deadline) if deadline < now)
}

/// The rows the pen lets go of without writing them: the row it was on (if any), and,
/// when the queue is dropped, every row still waiting in it. A skipped row releases
/// only itself.
pub fn let_go(current: Option<&TaskId>, queue: &[TaskId], queue_dropped: bool) -> Vec<TaskId> {
    let waiting = if queue_dropped { queue } else { &[] };
    current.into_iter().chain(waiting).cloned().collect()
}

/// What a tap on a row does.
#[derive(Debug, PartialEq, Eq)]
pub enum Tapped {
    /// Its ink is still wet: the finger smears it, wherever on the row it lands.
    Smudge,
    /// Dry, on the title: the row opens to edit.
    Open,
    /// Dry, anywhere else on the row.
    Nothing,
}

/// What a tap on a row does: wet ink smears wherever it lands; dry, only the title opens.
pub fn tapped(wet: bool, on_title: bool) -> Tapped {
    match (wet, on_title) {
        (true, _) => Tapped::Smudge,
        (false, true) => Tapped::Open,
        (false, false) => Tapped::Nothing,
    }
}

/// Which kind of time the edit sheet's time field holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditWhen {
    Anytime,
    DueBy,
    At,
}

/// What the edit sheet shows when it opens on the row `id`, or `None` when the
/// list no longer holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct EditForm {
    pub title: String,
    pub duration: String,
    pub when: EditWhen,
    pub datetime: String,
    pub weekdays: Vec<Weekday>,
    pub until: String,
    pub time: String,
}

/// What the sheet says when Save is pressed with no title.
pub const MISSING_TITLE_NOTE: &str = "Give the task a name first.";

/// The id the sheet carries while it holds a task nobody has made yet (the + button). No
/// server id looks like it, and nothing is stored until Save.
pub const DRAFT_ID: &str = "draft";

pub fn is_draft(id: &TaskId) -> bool {
    id.0 == DRAFT_ID
}

/// The task a new sheet is drawn from: only what the sheet shows of a task (its title
/// line and its zone), none of it the server's. The clock fields are the epoch because
/// nothing here may read the wall clock, and nothing shows them.
pub fn blank_task() -> Task {
    Task {
        id: TaskId(DRAFT_ID.to_owned()),
        title: String::new(),
        duration_min: tuning::NEW_TASK_MINUTES,
        when: When::Anytime,
        done: false,
        order: 0,
        label: Label::Other,
        priority: Priority::Low,
        source: Source::Text,
        created: DateTime::UNIX_EPOCH,
        updated_at: DateTime::UNIX_EPOCH,
        due_tz: None,
        external_uid: None,
        off_planner: false,
    }
}

/// What the sheet shows when it opens on a new task: an empty title, the default length
/// and no day or repeat.
pub fn blank_form() -> EditForm {
    EditForm {
        title: String::new(),
        duration: tuning::NEW_TASK_MINUTES.to_string(),
        when: EditWhen::Anytime,
        datetime: String::new(),
        weekdays: Vec::new(),
        until: String::new(),
        time: String::new(),
    }
}

/// The task a Save on a new sheet makes, or `None` while it lacks a title or a length.
pub fn new_task_draft(title: &str, duration: Option<u16>, when: When) -> Option<TaskDraft> {
    let title = title.trim();
    let duration_min = duration.filter(|minutes| *minutes > 0)?;
    (!title.is_empty()).then(|| TaskDraft {
        title: title.to_owned(),
        duration_min,
        when,
        label: Label::Other,
        priority: Priority::Low,
        source: Source::Text,
    })
}

/// The sheet after a refill: each field takes the server's `fresh` value unless the
/// user has `typed` in it since the sheet showed `shown`.
pub fn keep_typed(shown: &EditForm, typed: EditForm, fresh: EditForm) -> EditForm {
    fn pick<T: PartialEq>(shown: &T, typed: T, fresh: T) -> T {
        if typed == *shown { fresh } else { typed }
    }
    EditForm {
        title: pick(&shown.title, typed.title, fresh.title),
        duration: pick(&shown.duration, typed.duration, fresh.duration),
        when: pick(&shown.when, typed.when, fresh.when),
        datetime: pick(&shown.datetime, typed.datetime, fresh.datetime),
        weekdays: pick(&shown.weekdays, typed.weekdays, fresh.weekdays),
        until: pick(&shown.until, typed.until, fresh.until),
        time: pick(&shown.time, typed.time, fresh.time),
    }
}

/// The series a row belongs to, when the row is an occurrence of one.
pub fn series_of<'a>(snapshot: &'a Snapshot, id: &str) -> Option<&'a Series> {
    let (series_id, _) = occurrence_parts(id)?;
    snapshot
        .series
        .iter()
        .find(|series| series.id.0 == series_id)
}

/// Whether a press on a row may lift it: not while Rambo is dazed, when every hold on
/// the page draws him to the finger instead.
pub fn row_lifts(rambo: &State) -> bool {
    crate::mascot::drawn_over(rambo, &crate::gesture::Gesture::Hold).is_none()
}

/// Whether the row keeps the finger from the page: only once it is lifted. A press
/// that is still a tap must not stop the click, and a scroll must not stop the scroll.
pub fn row_holds_finger(gesture: Option<Gesture>) -> bool {
    gesture == Some(Gesture::Lifted)
}

/// What letting go of a row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowLetGo {
    /// Nothing: a scroll, a swipe backed out of, a dry tap off the title, or a drop
    /// that is no reorder.
    Nothing,
    /// A tap on wet ink: the finger smears it.
    Smudge,
    /// A dry tap on the title: the row opens to edit.
    Open,
    /// The swipe went far enough: the task is done.
    Done,
    /// A drop in the list: the stored tasks in this order. A drop that starts or lands on
    /// a series occurrence is no reorder (`reorder_ids`), so nothing is sent rather than
    /// an order the server would refuse.
    Reorder(Vec<TaskId>),
}

/// Letting go of row `dragged` as `release` says, in `snapshot`. What it needs from the
/// page is looked up only when it decides anything: whether the row's ink is `wet` and
/// the finger was `on_title` for a tap, and the row and side it was `dropped` on for a
/// reorder (with none, nothing is sent).
pub fn row_released(
    release: Release,
    dragged: &TaskId,
    snapshot: &Snapshot,
    wet: impl FnOnce() -> bool,
    on_title: impl FnOnce() -> bool,
    dropped: impl FnOnce() -> Option<(TaskId, Slot)>,
) -> RowLetGo {
    match release {
        Release::Tap => match tapped(wet(), on_title()) {
            Tapped::Smudge => RowLetGo::Smudge,
            Tapped::Open => RowLetGo::Open,
            Tapped::Nothing => RowLetGo::Nothing,
        },
        Release::Scroll | Release::SnapBack => RowLetGo::Nothing,
        Release::Done => RowLetGo::Done,
        Release::Reorder => dropped()
            .and_then(|(target, slot)| reorder_ids(snapshot, dragged, &target, slot))
            .map_or(RowLetGo::Nothing, RowLetGo::Reorder),
    }
}

/// Whether Rambo writes the rows queued for his pen (rows the page has just made,
/// written in one at a time) at all: not with reduced motion and not lobotomized.
/// Then the queue is dropped, so his lines are not held for it.
pub fn pen_writes(mode: RamblMode, reduced_motion: bool) -> bool {
    !reduced_motion && mode != RamblMode::Lobotomized
}

/// Whether he is free to take the next queued row: at home, or still at the caret
/// after Add.
pub fn takes_next_row(state: &State) -> bool {
    matches!(state, State::Home | State::WritingCaret)
}

/// What he does with the next queued row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenStep {
    /// He flies to it and writes its title in.
    Write,
    /// It is off the screen (the other pane on a phone, or scrolled away): it is left
    /// as it is, and he stays home.
    Skip,
    /// He was taken elsewhere between rows: this row and every row still queued are
    /// let go unwritten.
    Stop,
}

/// The next queued row's step, with him as `state` and the row `in_view` or not.
pub fn pen_step(state: &State, in_view: bool) -> PenStep {
    match (takes_next_row(state), in_view) {
        (false, _) => PenStep::Stop,
        (true, false) => PenStep::Skip,
        (true, true) => PenStep::Write,
    }
}

/// Whether he flies home after a row: only while he is still writing it. A row
/// finished without him (he was taken elsewhere) leaves him where he is.
pub fn flies_home_after_row(state: &State) -> bool {
    *state == State::WritingRow
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Snapshot {
        serde_json::from_str(include_str!("../../../web/fixtures/snapshot.json")).expect("fixture")
    }

    fn id(name: &str) -> TaskId {
        TaskId(name.to_owned())
    }

    #[test]
    fn a_tap_on_a_dry_title_opens_the_row_and_wet_ink_only_smudges() {
        assert_eq!(tapped(false, true), Tapped::Open);
        assert_eq!(tapped(false, false), Tapped::Nothing);
        assert_eq!(
            tapped(true, true),
            Tapped::Smudge,
            "wet, the row does not open"
        );
        assert_eq!(tapped(true, false), Tapped::Smudge);
    }

    #[test]
    fn overdue_only_applies_to_incomplete_due_tasks() {
        let mut task = fixture().tasks[0].clone();
        let now = task.created + chrono::Duration::hours(1);
        task.when = When::DueBy(now - chrono::Duration::minutes(1));
        assert!(is_overdue_at(&task, now));
        task.done = true;
        assert!(!is_overdue_at(&task, now));
        task.done = false;
        task.when = When::At(now - chrono::Duration::minutes(1));
        assert!(!is_overdue_at(&task, now));
    }

    /// Reorder sends the server exactly the stored tasks, so the occurrence rows
    /// a snapshot carries are kept out of the list — and a drop that starts or
    /// lands on one is not a reorder at all.
    #[test]
    fn a_reorder_lists_stored_tasks_once_and_never_an_occurrence() {
        let mut snapshot = fixture();
        // An expanded day of a repeating series sits between the stored tasks.
        let mut occurrence = snapshot.tasks[0].clone();
        occurrence.id = TaskId("standup:2026-10-14".into());
        occurrence.source = Source::Series;
        snapshot.tasks.insert(1, occurrence);

        let first = snapshot.tasks[0].id.clone();
        let third = snapshot.tasks[2].id.clone();
        let order = reorder_ids(&snapshot, &third, &first, Slot::Before)
            .expect("stored slip onto stored slip");
        assert_eq!(
            order.first(),
            Some(&third),
            "the slip moves before its target"
        );
        assert_eq!(order.len(), 2, "only stored tasks, exactly once");
        assert!(!order.iter().any(|id| id.0.contains(':')));

        let occ = snapshot.tasks[1].id.clone();
        assert_eq!(
            reorder_ids(&snapshot, &occ, &first, Slot::Before),
            None,
            "dragged occurrence"
        );
        assert_eq!(
            reorder_ids(&snapshot, &first, &occ, Slot::Before),
            None,
            "dropped on occurrence"
        );
    }

    #[test]
    fn a_drop_below_its_target_moves_the_slip_down() {
        let snapshot = fixture();
        let first = snapshot.tasks[0].id.clone();
        let second = snapshot.tasks[1].id.clone();

        let below = reorder_ids(&snapshot, &first, &second, Slot::After)
            .expect("dropped on the lower half");
        assert_eq!(
            below.first(),
            Some(&second),
            "the slip sits after its target"
        );
        assert_eq!(below.get(1), Some(&first));

        let above = reorder_ids(&snapshot, &second, &first, Slot::Before)
            .expect("dropped on the upper half");
        assert_eq!(
            above.first(),
            Some(&second),
            "the slip sits before its target"
        );
    }

    #[test]
    fn a_slip_dropped_on_itself_keeps_its_place() {
        let snapshot = fixture();
        let first = snapshot.tasks[0].id.clone();
        assert_eq!(
            reorder_ids(&snapshot, &first, &first, Slot::Before),
            None,
            "a row dropped on itself is not a move, and re-sending the order would fight the user"
        );
    }

    #[test]
    fn a_slip_dropped_over_empty_paper_goes_to_the_end() {
        let snapshot = fixture();
        let first = snapshot.tasks[0].id.clone();
        // The finger let go where no row is: there is no target to land beside.
        let empty = TaskId("no-row-here".into());

        let order =
            reorder_ids(&snapshot, &first, &empty, Slot::Before).expect("a target the list lacks");
        assert_eq!(
            order.last(),
            Some(&first),
            "landing nowhere puts the slip at the end, where the finger left off"
        );
        let stored = snapshot
            .tasks
            .iter()
            .filter(|task| task.source != Source::Series)
            .count();
        assert_eq!(order.len(), stored, "still every stored task exactly once");
    }

    #[test]
    fn an_occurrence_id_names_its_series_and_date() {
        let (series, date) = occurrence_parts("sgym:2026-10-15").expect("a valid occurrence id");
        assert_eq!(series, "sgym");
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 10, 15).unwrap());
        // Ordinary task ids carry no date part.
        assert!(occurrence_parts("t1234").is_none());
        assert!(occurrence_parts("sgym:not-a-date").is_none());
    }

    /// A skipped row lets go of itself only: the rows still queued stay as they are.
    #[test]
    fn a_skipped_row_releases_only_itself() {
        let queue = [id("b")];
        assert_eq!(let_go(Some(&id("a")), &queue, false), vec![id("a")]);
    }

    /// A dropped queue lets go of the row he was on and every row waiting behind it.
    #[test]
    fn a_dropped_queue_releases_the_row_he_was_on_and_every_queued_row() {
        let queue = [id("b"), id("c")];
        assert_eq!(
            let_go(Some(&id("a")), &queue, true),
            vec![id("a"), id("b"), id("c")]
        );
    }

    /// Reduced motion drops the queue before any row is on the pen.
    #[test]
    fn a_dropped_queue_with_no_row_on_the_pen_releases_the_queue() {
        let queue = [id("b")];
        assert_eq!(let_go(None, &queue, true), vec![id("b")]);
        assert!(let_go(None, &[], true).is_empty());
    }

    #[test]
    fn a_new_sheet_starts_empty_with_the_default_length_and_no_day() {
        let form = blank_form();
        assert_eq!(form.title, "");
        assert_eq!(form.duration, tuning::NEW_TASK_MINUTES.to_string());
        assert_eq!(form.when, EditWhen::Anytime);
        assert!(form.weekdays.is_empty() && form.until.is_empty() && form.time.is_empty());
        assert!(is_draft(&blank_task().id));
        assert!(!is_draft(&TaskId("t1".into())));
    }

    #[test]
    fn a_new_task_needs_a_title_and_a_length_and_is_trimmed() {
        let made = new_task_draft("  Buy stamps ", Some(15), When::Anytime).expect("a task");
        assert_eq!((made.title.as_str(), made.duration_min), ("Buy stamps", 15));
        assert_eq!(made.source, Source::Text);
        assert!(new_task_draft("   ", Some(15), When::Anytime).is_none());
        assert!(new_task_draft("Buy stamps", None, When::Anytime).is_none());
        assert!(new_task_draft("Buy stamps", Some(0), When::Anytime).is_none());
    }

    #[test]
    fn a_refill_takes_the_servers_value_except_where_the_user_typed() {
        let shown = blank_form();
        let typed = EditForm {
            title: "Gym with Sam".into(),
            ..shown.clone()
        };
        let fresh = EditForm {
            title: "Gym (server)".into(),
            datetime: "2026-10-16T15:00".into(),
            ..shown.clone()
        };
        let refilled = keep_typed(&shown, typed, fresh);
        assert_eq!(refilled.title, "Gym with Sam", "typed meanwhile: kept");
        assert_eq!(
            refilled.datetime, "2026-10-16T15:00",
            "untouched: the server's"
        );
        assert_eq!(refilled.duration, shown.duration);
    }

    #[test]
    fn he_writes_queued_rows_unless_motion_is_reduced_or_he_is_lobotomized() {
        assert!(pen_writes(RamblMode::Interactive, false));
        assert!(pen_writes(RamblMode::Instant, false));
        assert!(!pen_writes(RamblMode::Lobotomized, false));
        assert!(!pen_writes(RamblMode::Interactive, true));
    }

    #[test]
    fn he_takes_a_row_from_home_or_the_caret_and_stops_when_taken_elsewhere() {
        assert_eq!(pen_step(&State::Home, true), PenStep::Write);
        assert_eq!(
            pen_step(&State::WritingCaret, true),
            PenStep::Write,
            "after Add"
        );
        assert_eq!(
            pen_step(&State::Home, false),
            PenStep::Skip,
            "off the screen"
        );
        for busy in [
            State::Dragging,
            State::Dazed,
            State::Flying,
            State::WritingRow,
        ] {
            assert!(!takes_next_row(&busy), "{busy:?}");
            assert_eq!(pen_step(&busy, true), PenStep::Stop, "{busy:?}");
        }
        // Taken elsewhere wins over off the screen: the rest are let go, not skipped.
        assert_eq!(pen_step(&State::Dragging, false), PenStep::Stop);
    }

    #[test]
    fn he_flies_home_only_from_the_row_he_is_still_writing() {
        assert!(flies_home_after_row(&State::WritingRow));
        assert!(
            !flies_home_after_row(&State::Dragging),
            "taken elsewhere: he stays"
        );
        assert!(!flies_home_after_row(&State::Home));
    }

    #[test]
    fn a_dazed_rambo_takes_the_hold_so_the_row_does_not_lift() {
        assert!(row_lifts(&State::Home));
        assert!(row_lifts(&State::Dragging));
        assert!(!row_lifts(&State::Dazed));
    }

    #[test]
    fn only_a_lifted_row_keeps_the_finger() {
        assert!(row_holds_finger(Some(Gesture::Lifted)));
        for gesture in [
            None,
            Some(Gesture::Pressed),
            Some(Gesture::Scrolling),
            Some(Gesture::Swiping),
        ] {
            assert!(!row_holds_finger(gesture), "{gesture:?}");
        }
    }

    #[test]
    fn letting_go_of_a_row_looks_up_only_what_decides_it() {
        let snapshot = fixture();
        let first = snapshot.tasks[0].id.clone();
        let never = || -> bool { panic!("not asked") };
        let nowhere = || -> Option<(TaskId, Slot)> { panic!("not asked") };
        let tap = |wet: bool, on_title: bool| {
            row_released(
                Release::Tap,
                &first,
                &snapshot,
                || wet,
                || on_title,
                nowhere,
            )
        };
        assert_eq!(tap(true, false), RowLetGo::Smudge);
        assert_eq!(tap(false, true), RowLetGo::Open);
        assert_eq!(tap(false, false), RowLetGo::Nothing);
        for release in [Release::Scroll, Release::SnapBack] {
            assert_eq!(
                row_released(release, &first, &snapshot, never, never, nowhere),
                RowLetGo::Nothing
            );
        }
        assert_eq!(
            row_released(Release::Done, &first, &snapshot, never, never, nowhere),
            RowLetGo::Done
        );
    }

    #[test]
    fn a_drop_reorders_unless_it_lands_on_itself() {
        let snapshot = fixture();
        let (first, second) = (snapshot.tasks[0].id.clone(), snapshot.tasks[1].id.clone());
        let never = || -> bool { panic!("not asked") };
        let drop_on = |target: &TaskId| {
            let target = target.clone();
            row_released(
                Release::Reorder,
                &first,
                &snapshot,
                never,
                never,
                move || Some((target, Slot::After)),
            )
        };
        assert_eq!(
            drop_on(&second),
            RowLetGo::Reorder(
                reorder_ids(&snapshot, &first, &second, Slot::After).expect("a move")
            )
        );
        assert_eq!(drop_on(&first), RowLetGo::Nothing, "on itself");
        assert_eq!(
            row_released(Release::Reorder, &first, &snapshot, never, never, || None),
            RowLetGo::Nothing
        );
    }
}
