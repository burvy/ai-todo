use chrono::{DateTime, Utc};
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

pub mod api;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub duration_min: u16,
    pub when: When,
    pub done: bool,
    pub order: i64,
    pub label: String,
    pub priority: u8,
    pub source: Source,
    pub created: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum When {
    Anytime,
    DueBy(DateTime<Utc>),
    At(DateTime<Utc>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Source {
    Voice,
    Pdf,
    Text,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Busy {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub task: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub placements: Vec<Placement>,
    pub unplaced: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub tasks: Vec<Task>,
    pub plan: Plan,
    pub busy: Vec<Busy>,
    pub tz: String,
    pub generated: DateTime<Utc>,
}

#[component]
pub fn App() -> impl IntoView {
    let snapshot = api::use_snapshot();
    view! {
        <header><h1>"Rambl"</h1><CaptureBox /></header>
        <main>
            <TaskList snapshot=snapshot />
            <WeekGrid snapshot=snapshot />
        </main>
    }
}

#[component]
fn CaptureBox() -> impl IntoView {
    view! {
        <form class="capture" on:submit=|ev| ev.prevent_default()>
            <input placeholder="Say what you need to do…" aria-label="Ramble" />
            <button type="button" aria-label="Use microphone">"Mic"</button>
            <button type="submit">"Add"</button>
        </form>
    }
}

#[component]
fn TaskList(snapshot: ReadSignal<Snapshot>) -> impl IntoView {
    view! {
        <section aria-labelledby="tasks-heading">
            <h2 id="tasks-heading">"Tasks"</h2>
            <ul>
                <For each=move || snapshot.get().tasks key=|task| task.id.clone() let:task>
                    <li><strong>{task.title}</strong> " — " {task.duration_min} " min"
                        {move || task_time(&snapshot.get(), &task.id)}
                    </li>
                </For>
            </ul>
        </section>
    }
}

#[component]
fn WeekGrid(snapshot: ReadSignal<Snapshot>) -> impl IntoView {
    view! {
        <section aria-labelledby="week-heading">
            <h2 id="week-heading">"Week"</h2>
            <div class="week">
                <For each=move || week_days() key=|day| *day let:day>
                    <div class="day"><h3>{format!("Day {}", day + 1)}</h3>
                        <For each=move || snapshot.get().plan.placements.clone()
                            key=|placement| placement.task.clone() let:placement>
                            <div class="placement">{placement.task.clone()} " " {placement_time(&placement)}</div>
                        </For>
                        <div class="hour"></div><div class="hour"></div><div class="hour"></div>
                    </div>
                </For>
            </div>
        </section>
    }
}

fn task_time(snapshot: &Snapshot, id: &str) -> String {
    snapshot
        .plan
        .placements
        .iter()
        .find(|p| p.task == id)
        .map(|p| p.start.format("%a %H:%M").to_string())
        .unwrap_or_default()
}

fn placement_time(placement: &Placement) -> String {
    placement.start.format("%a %H:%M").to_string()
}

fn week_days() -> Vec<u8> {
    (0..7).collect()
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    leptos::mount::mount_to_body(App);
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn list_and_week_use_the_same_task_time() {
        let snapshot: Snapshot =
            serde_json::from_str(include_str!("../fixtures/snapshot.json")).unwrap();
        for task in &snapshot.tasks {
            let list_time = task_time(&snapshot, &task.id);
            let week_time = snapshot
                .plan
                .placements
                .iter()
                .find(|p| p.task == task.id)
                .map(placement_time)
                .unwrap_or_default();
            assert_eq!(list_time, week_time);
        }
    }
}
