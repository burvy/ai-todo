use chrono::{TimeZone, Utc};
use gloo_timers::callback::{Interval, Timeout};
use leptos::prelude::*;
use rambl_brain::{Action, Brain, Event, Mode, Seeded};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use todo_core::{Snapshot, When};

fn brain_mode(value: crate::settings::RamblMode) -> Mode {
    match value {
        crate::settings::RamblMode::Interactive => Mode::Interactive,
        crate::settings::RamblMode::Instant => Mode::Instant,
        crate::settings::RamblMode::Lobotomized => Mode::Lobotomized,
    }
}

/// Pure: which overdue task ids are newly overdue (tease), which to forget (done/deleted),
/// given current snapshot tasks, the already-teased set, and now.
fn overdue_changes(
    current: &Snapshot,
    teased: &HashSet<String>,
    now: chrono::DateTime<chrono::Utc>,
) -> (Vec<String>, Vec<String>) {
    let overdue_now: HashSet<String> = current
        .tasks
        .iter()
        .filter(|t| !t.done && matches!(t.when, When::DueBy(deadline) if deadline < now))
        .map(|t| t.id.0.clone())
        .collect();
    let current_ids: HashSet<String> = current.tasks.iter().map(|t| t.id.0.clone()).collect();
    let newly: Vec<String> = overdue_now.difference(teased).cloned().collect();
    let forget: Vec<String> = teased
        .iter()
        .filter(|id| !overdue_now.contains(*id) || !current_ids.contains(*id))
        .cloned()
        .collect();
    (newly, forget)
}

/// Pure: whether a task arrived between two snapshots. `previous` is `None`
/// until the server has answered once, because nothing can have been added to
/// nothing. Ids and not counts, so one task gone and one new still counts, and
/// a repeat-day occurrence (`{series}:{date}`, and a date holds no colon) is a
/// row of a task already there, never a new task.
fn task_added(current: &Snapshot, previous: Option<&Snapshot>) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    let known: HashSet<&str> = previous.tasks.iter().map(|t| t.id.0.as_str()).collect();
    current
        .tasks
        .iter()
        .any(|t| !t.id.0.contains(':') && !known.contains(t.id.0.as_str()))
}

fn step(
    brain: &Rc<RefCell<Brain>>,
    rng: &Rc<RefCell<Seeded>>,
    event: Option<Event>,
    audible: Signal<bool>,
    set_speech: WriteSignal<Option<String>>,
    set_pose: WriteSignal<String>,
) {
    let now = js_sys::Date::now();
    let now = Utc
        .timestamp_millis_opt(now as i64)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().expect("unix epoch"));
    let hour = js_sys::Date::new_0().get_hours() as u8;
    // He hears the hour when it changes, ahead of the event itself: every caller
    // passes an event, so folding the hour into a missing one meant he never heard
    // it and never slept. Not on every step, since the hour also settles his state.
    let told = (brain.borrow().hour != hour).then_some(Event::LocalHour(hour));
    let mut actions = Vec::new();
    for event in told.into_iter().map(Some).chain([event]) {
        let (next, more) = rambl_brain::step(&brain.borrow(), event, now, &mut *rng.borrow_mut());
        *brain.borrow_mut() = next;
        actions.extend(more);
    }
    for action in actions {
        match action {
            Action::Say(key) => {
                if let Some(line) = rambl_brain::data::lines().resolve(&key) {
                    let line = line.to_owned();
                    set_speech.set(Some(line.clone()));
                    // The bubble and the voice are one thing: with a voice it
                    // goes when the voice is done, plus a beat. Without one there
                    // is nothing to wait for, so it holds on its own — it is then
                    // the only channel the line has. Exactly one timer, either way.
                    let clear_later = move |after_ms: u32| {
                        let clear = set_speech;
                        Timeout::new(after_ms, move || clear.set(None)).forget();
                    };
                    let spoke = audible.get_untracked()
                        && crate::voice::say(&line, {
                            let clear = clear_later;
                            move || clear(crate::voice::bubble_ms(true))
                        });
                    if !spoke {
                        clear_later(crate::voice::bubble_ms(false));
                    }
                }
            }
            Action::Pose(pose) | Action::Animate(pose) => set_pose.set(pose.to_owned()),
            Action::MoveTo(_) => {}
        }
    }
}

#[component]
pub fn Controller(
    snapshot: ReadSignal<Snapshot>,
    mode: Signal<crate::settings::RamblMode>,
    event: ReadSignal<Option<Event>>,
    set_event: WriteSignal<Option<Event>>,
    /// Whether he may be heard at all: the gear menu's sound mute and his own
    /// `Rambo talks` toggle, already combined by the caller.
    audible: Signal<bool>,
    set_speech: WriteSignal<Option<String>>,
    set_pose: WriteSignal<String>,
) -> impl IntoView {
    let brain = Rc::new(RefCell::new(Brain::new(brain_mode(mode.get_untracked()))));
    let rng = Rc::new(RefCell::new(Seeded::new(js_sys::Date::now() as u64)));
    // The page hands every brain the snapshot it shows before the server
    // answers: empty, with no task of the user's in it. That one is not a load,
    // and treating it as one is what made him greet every page load with a new
    // task.
    let placeholder = snapshot.get_untracked();
    let previous = Rc::new(RefCell::new(placeholder.clone()));
    // The last snapshot the server answered, and so what an addition is
    // measured against. `None` until then.
    let loaded = Rc::new(RefCell::new(None::<Snapshot>));
    let teased = Rc::new(RefCell::new(HashSet::new()));

    Effect::new({
        let brain = brain.clone();
        move |_| {
            brain.borrow_mut().mode = brain_mode(mode.get());
        }
    });

    Effect::new({
        let brain = brain.clone();
        let rng = rng.clone();
        move |_| {
            if let Some(event) = event.get() {
                step(&brain, &rng, Some(event), audible, set_speech, set_pose);
                set_event.set(None);
            }
        }
    });

    Effect::new({
        let brain = brain.clone();
        let rng = rng.clone();
        let previous = previous.clone();
        let loaded = loaded.clone();
        let teased = teased.clone();
        move |_| {
            let current = snapshot.get();
            let old = previous.replace(current.clone());
            let added = if current == placeholder {
                false
            } else {
                task_added(&current, loaded.replace(Some(current.clone())).as_ref())
            };
            let now = Utc
                .timestamp_millis_opt(js_sys::Date::now() as i64)
                .single()
                .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().expect("unix epoch"));
            let (newly, forget) = overdue_changes(&current, &teased.borrow(), now);
            {
                let mut t = teased.borrow_mut();
                for id in forget {
                    t.remove(&id);
                }
                for id in &newly {
                    t.insert(id.clone());
                }
            }
            let event = if added {
                Some(Event::TaskAdded)
            } else if current.tasks.iter().any(|task| task.done)
                && current.tasks.iter().filter(|task| task.done).count()
                    > old.tasks.iter().filter(|task| task.done).count()
            {
                Some(Event::TaskDone)
            } else if current.tasks.is_empty() {
                Some(Event::ListEmpty)
            } else if !newly.is_empty() {
                Some(Event::TaskOverdue)
            } else {
                None
            };
            if event.is_some() {
                step(&brain, &rng, event, audible, set_speech, set_pose);
            }
        }
    });

    let tick = {
        let brain = brain.clone();
        let rng = rng.clone();
        let teased = teased.clone();
        Interval::new(5000, move || {
            if mode.get_untracked() != crate::settings::RamblMode::Lobotomized
                && !window()
                    .document()
                    .is_some_and(|document| document.hidden())
            {
                let now = Utc
                    .timestamp_millis_opt(js_sys::Date::now() as i64)
                    .single()
                    .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().expect("unix epoch"));
                let current = snapshot.get_untracked();
                let (newly, forget) = overdue_changes(&current, &teased.borrow(), now);
                {
                    let mut t = teased.borrow_mut();
                    for id in forget {
                        t.remove(&id);
                    }
                    for id in &newly {
                        t.insert(id.clone());
                    }
                }
                let event = if !newly.is_empty() {
                    Some(Event::TaskOverdue)
                } else {
                    Some(Event::Tick)
                };
                step(&brain, &rng, event, audible, set_speech, set_pose);
            }
        })
    };
    // The brain lives for the whole page, so the interval is never cancelled.
    tick.forget();

    view! { <span class="brain-speech" aria-live="polite"></span> }
}

#[cfg(test)]
mod overdue_tests {
    use super::*;
    use chrono::Utc;
    use todo_core::{Plan, Snapshot, Task, TaskId, When};

    fn snapshot_with_tasks(tasks: Vec<Task>) -> Snapshot {
        Snapshot {
            tasks,
            plan: Plan::default(),
            tz: "UTC".into(),
            generated: Utc::now(),
            series: Vec::new(),
        }
    }

    fn task_due(id: &str, minutes_past: i64) -> Task {
        let at = Utc::now() - chrono::Duration::minutes(minutes_past);
        Task {
            id: TaskId(id.into()),
            title: id.into(),
            duration_min: 30,
            when: When::DueBy(at),
            done: false,
            order: 0,
            label: todo_core::Label::Work,
            priority: todo_core::Priority::Medium,
            source: todo_core::Source::Text,
            created: at,
            updated_at: at,
            due_tz: None,
            external_uid: None,
        }
    }

    fn done_task(id: &str) -> Task {
        let mut t = task_due(id, 60);
        t.done = true;
        t
    }

    #[test]
    fn newly_overdue_is_teased() {
        let now = Utc::now();
        let current = snapshot_with_tasks(vec![task_due("a", 30)]);
        let teased = HashSet::new();
        let (newly, forget) = overdue_changes(&current, &teased, now);
        assert_eq!(newly, vec!["a".to_string()]);
        assert!(forget.is_empty());
    }

    #[test]
    fn already_teased_is_not_teased_again() {
        let now = Utc::now();
        let current = snapshot_with_tasks(vec![task_due("a", 30)]);
        let mut teased = HashSet::new();
        teased.insert("a".to_string());
        let (newly, forget) = overdue_changes(&current, &teased, now);
        assert!(newly.is_empty());
        assert!(forget.is_empty());
    }

    #[test]
    fn done_task_is_forgotten() {
        let now = Utc::now();
        let current = snapshot_with_tasks(vec![done_task("a")]);
        let mut teased = HashSet::new();
        teased.insert("a".to_string());
        let (newly, forget) = overdue_changes(&current, &teased, now);
        assert!(newly.is_empty());
        assert_eq!(forget, vec!["a".to_string()]);
    }

    #[test]
    fn deleted_task_is_forgotten() {
        let now = Utc::now();
        let current = snapshot_with_tasks(vec![]);
        let mut teased = HashSet::new();
        teased.insert("gone".to_string());
        let (newly, forget) = overdue_changes(&current, &teased, now);
        assert!(newly.is_empty());
        assert_eq!(forget, vec!["gone".to_string()]);
    }
}

#[cfg(test)]
mod added_tests {
    use super::*;
    use todo_core::{Plan, Task, TaskId};

    fn task(id: &str) -> Task {
        let now = Utc::now();
        Task {
            id: TaskId(id.into()),
            title: id.into(),
            duration_min: 30,
            when: When::Anytime,
            done: false,
            order: 0,
            label: todo_core::Label::Work,
            priority: todo_core::Priority::Medium,
            source: todo_core::Source::Text,
            created: now,
            updated_at: now,
            due_tz: None,
            external_uid: None,
        }
    }

    fn snapshot_of(ids: &[&str]) -> Snapshot {
        Snapshot {
            tasks: ids.iter().map(|id| task(id)).collect(),
            plan: Plan::default(),
            tz: "UTC".into(),
            generated: Utc::now(),
            series: Vec::new(),
        }
    }

    #[test]
    fn the_first_load_adds_nothing() {
        assert!(!task_added(&snapshot_of(&["a", "b"]), None));
    }

    #[test]
    fn a_new_task_is_an_addition() {
        let before = snapshot_of(&["a"]);
        assert!(task_added(&snapshot_of(&["a", "b"]), Some(&before)));
    }

    #[test]
    fn a_new_repeat_day_row_is_not_a_new_task() {
        let before = snapshot_of(&["a"]);
        let after = snapshot_of(&["a", "standup:2026-10-14"]);
        assert!(!task_added(&after, Some(&before)));
    }

    #[test]
    fn one_gone_and_one_new_is_still_an_addition() {
        let before = snapshot_of(&["a", "b"]);
        let after = snapshot_of(&["b", "c"]);
        assert_eq!(before.tasks.len(), after.tasks.len());
        assert!(task_added(&after, Some(&before)));
    }

    #[test]
    fn an_unchanged_snapshot_is_not_an_addition() {
        let snapshot = snapshot_of(&["a", "b"]);
        assert!(!task_added(&snapshot, Some(&snapshot)));
    }
}
