//! Print a simulated day of Rambl, so you can watch how he behaves without a browser.
//!
//! ```sh
//! cargo run -p rambl-brain --example sim
//! cargo run -p rambl-brain --example sim -- 99   # a different day
//! ```

use chrono::{DateTime, TimeZone, Utc};
use rambl_brain::{Action, Brain, Event, Mode, Seeded, step};

/// One simulated moment: the hour, what happened, what he did.
struct Moment {
    hour: u8,
    event: Option<Event>,
    now: DateTime<Utc>,
}

fn day() -> Vec<Moment> {
    let mut moments = Vec::new();
    let mut push = |hour: u8, event: Option<Event>| {
        moments.push(Moment {
            hour,
            event,
            now: Utc
                .with_ymd_and_hms(2026, 10, 13, u32::from(hour), 0, 0)
                .unwrap(),
        });
    };
    push(7, Some(Event::LocalHour(7)));
    push(7, Some(Event::ListEmpty));
    push(8, Some(Event::TaskAdded));
    push(9, Some(Event::TaskAdded));
    push(10, Some(Event::Tick));
    push(11, Some(Event::TaskDone));
    push(12, Some(Event::UserIdle(20)));
    push(13, Some(Event::Poked));
    push(
        14,
        Some(Event::Dropped {
            on_task: "Lab report".into(),
        }),
    );
    push(15, Some(Event::TaskOverdue));
    push(16, None);
    push(19, None);
    push(21, Some(Event::LocalHour(21)));
    push(23, Some(Event::LocalHour(23)));
    push(23, Some(Event::Poked));
    push(7, Some(Event::LocalHour(7)));
    push(8, None);
    moments
}

fn describe(actions: &[Action]) -> String {
    let mut parts = Vec::new();
    for action in actions {
        let text = match action {
            Action::Pose(name) => format!("pose {name}"),
            Action::Animate(name) => format!("animate {name}"),
            Action::MoveTo(spot) => format!("move to {spot:?}"),
            Action::Say(key) => format!(
                "say {:?}",
                rambl_brain::data::lines()
                    .resolve(key)
                    .unwrap_or("(no text)")
            ),
        };
        parts.push(text);
    }
    if parts.is_empty() {
        "nothing".to_owned()
    } else {
        parts.join(", ")
    }
}

fn run(mode: Mode, seed: u64) {
    println!("\n== {mode:?} mode, seed {seed} ==\n");
    let mut rng = Seeded::new(seed);
    let mut brain = Brain::new(mode);
    println!(
        "{:<6} {:<10} {:<26} what he did",
        "hour", "state", "moods (e/c/b)"
    );
    for moment in day() {
        let (next, actions) = step(&brain, moment.event.clone(), moment.now, &mut rng);
        let m = next.moods;
        let hour = format!("{:02}:00", moment.hour);
        let state = format!("{:?}", next.state);
        let moods = format!("{:.2} / {:.2} / {:.2}", m.energy, m.cheer, m.boredom);
        let did = describe(&actions);
        println!("{hour:<6} {state:<10} {moods:<26} {did}");
        brain = next;
    }
    println!(
        "\nended {:?}, energy {:.2}\n",
        brain.state, brain.moods.energy
    );
}

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(1);
    run(Mode::Interactive, seed);
    run(Mode::Instant, seed);
}
