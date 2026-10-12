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

fn step(
    brain: &Rc<RefCell<Brain>>,
    rng: &Rc<RefCell<Seeded>>,
    event: Option<Event>,
    set_speech: WriteSignal<Option<String>>,
    set_pose: WriteSignal<String>,
) {
    let now = js_sys::Date::now();
    let now = Utc
        .timestamp_millis_opt(now as i64)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().expect("unix epoch"));
    let hour = js_sys::Date::new_0().get_hours() as u8;
    let mut current = brain.borrow().clone();
    let (next, actions) = rambl_brain::step(
        &current,
        event.or(Some(Event::LocalHour(hour))),
        now,
        &mut *rng.borrow_mut(),
    );
    current = next;
    *brain.borrow_mut() = current;
    for action in actions {
        match action {
            Action::Say(key) => {
                if let Some(line) = rambl_brain::data::lines().resolve(&key) {
                    set_speech.set(Some(line.to_owned()));
                    let clear = set_speech;
                    Timeout::new(3500, move || clear.set(None)).forget();
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
    set_speech: WriteSignal<Option<String>>,
    set_pose: WriteSignal<String>,
) -> impl IntoView {
    let brain = Rc::new(RefCell::new(Brain::new(brain_mode(mode.get_untracked()))));
    let rng = Rc::new(RefCell::new(Seeded::new(js_sys::Date::now() as u64)));
    let previous = Rc::new(RefCell::new(snapshot.get_untracked()));
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
                step(&brain, &rng, Some(event), set_speech, set_pose);
                set_event.set(None);
            }
        }
    });

    Effect::new({
        let brain = brain.clone();
        let rng = rng.clone();
        let previous = previous.clone();
        let teased = teased.clone();
        move |_| {
            let current = snapshot.get();
            let old = previous.replace(current.clone());
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
            let event = if current.tasks.len() > old.tasks.len() {
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
                step(&brain, &rng, event, set_speech, set_pose);
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
                step(&brain, &rng, event, set_speech, set_pose);
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
