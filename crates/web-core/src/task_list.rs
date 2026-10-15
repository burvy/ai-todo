//! The task list's model (ARCHITECTURE 8A.4, step 4): what the list's inputs mean,
//! decided here without the DOM so `cargo test` covers it. `web/src/tasks/mod.rs`
//! measures the page, asks these, and does what they answer.

use chrono::{DateTime, NaiveDate, Utc};
use todo_core::{Snapshot, Source, Task, TaskId, When};

use crate::row::Slot;

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
}
