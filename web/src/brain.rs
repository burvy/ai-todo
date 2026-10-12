use chrono::{TimeZone, Utc};
use gloo_timers::callback::{Interval, Timeout};
use leptos::prelude::*;
use rambl_brain::{Action, Brain, Event, Mode, Seeded};
use std::cell::RefCell;
use std::rc::Rc;
use todo_core::{Snapshot, When};

fn brain_mode(value: crate::settings::RamblMode) -> Mode {
    match value {
        crate::settings::RamblMode::Interactive => Mode::Interactive,
        crate::settings::RamblMode::Instant => Mode::Instant,
        crate::settings::RamblMode::Lobotomized => Mode::Lobotomized,
    }
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
        move |_| {
            let current = snapshot.get();
            let old = previous.replace(current.clone());
            let now = Utc
                .timestamp_millis_opt(js_sys::Date::now() as i64)
                .single()
                .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().expect("unix epoch"));
            let event = if current.tasks.len() > old.tasks.len() {
                Some(Event::TaskAdded)
            } else if current.tasks.iter().any(|task| task.done)
                && current.tasks.iter().filter(|task| task.done).count()
                    > old.tasks.iter().filter(|task| task.done).count()
            {
                Some(Event::TaskDone)
            } else if current.tasks.is_empty() {
                Some(Event::ListEmpty)
            } else if current.tasks.iter().any(|task| {
                !task.done && matches!(task.when, When::DueBy(deadline) if deadline < now)
            }) {
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
        Interval::new(5000, move || {
            if mode.get_untracked() != crate::settings::RamblMode::Lobotomized
                && !window()
                    .document()
                    .is_some_and(|document| document.hidden())
            {
                step(&brain, &rng, Some(Event::Tick), set_speech, set_pose);
            }
        })
    };
    // The brain lives for the whole page, so the interval is never cancelled.
    tick.forget();

    view! { <span class="brain-speech" aria-live="polite"></span> }
}
