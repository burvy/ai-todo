//! Calendar export: the plan as an iCalendar file (ARCHITECTURE.md section 6).
//! One event per placement. Only planned, open tasks appear.

use chrono::{DateTime, Utc};
use todo_core::Snapshot;

pub fn build_ics(snapshot: &Snapshot) -> String {
    let mut out = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Rambl//EN\r\nCALSCALE:GREGORIAN\r\n",
    );
    let stamp = format_utc(snapshot.generated);
    for placement in &snapshot.plan.placements {
        let title = snapshot
            .tasks
            .iter()
            .find(|task| task.id == placement.task)
            .map(|task| task.title.as_str())
            .unwrap_or("Task");
        out.push_str("BEGIN:VEVENT\r\n");
        push(&mut out, "UID", &format!("{}@rambl", placement.task.0));
        push(&mut out, "DTSTAMP", &stamp);
        push(&mut out, "DTSTART", &format_utc(placement.start));
        push(&mut out, "DTEND", &format_utc(placement.end));
        push(&mut out, "SUMMARY", &escape(title));
        out.push_str("END:VEVENT\r\n");
    }
    out.push_str("END:VCALENDAR\r\n");
    out
}

fn push(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push(':');
    out.push_str(value);
    out.push_str("\r\n");
}

fn format_utc(time: DateTime<Utc>) -> String {
    time.format("%Y%m%dT%H%M%SZ").to_string()
}

/// RFC 5545 text escaping for a property value.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use todo_core::{Label, Placement, Plan, Priority, Source, Task, TaskId, When};

    fn snapshot_with(title: &str) -> Snapshot {
        let start = Utc.with_ymd_and_hms(2026, 10, 12, 19, 0, 0).unwrap();
        let task = Task {
            id: TaskId("t1".into()),
            title: title.into(),
            duration_min: 60,
            when: When::At(start),
            done: false,
            order: 0,
            label: Label::Work,
            priority: Priority::Low,
            source: Source::Text,
            created: start,
            updated_at: start,
            external_uid: None,
        };
        Snapshot {
            tasks: vec![task],
            plan: Plan {
                placements: vec![Placement {
                    task: TaskId("t1".into()),
                    start,
                    end: start + chrono::Duration::minutes(60),
                }],
                unplaced: vec![],
            },
            tz: "America/New_York".into(),
            generated: start,
        }
    }

    #[test]
    fn one_event_per_placement_with_utc_times() {
        let ics = build_ics(&snapshot_with("Meet Maya"));
        assert!(ics.contains("UID:t1@rambl\r\n"));
        assert!(ics.contains("DTSTART:20261012T190000Z\r\n"));
        assert!(ics.contains("DTEND:20261012T200000Z\r\n"));
        assert!(ics.contains("SUMMARY:Meet Maya\r\n"));
    }

    #[test]
    fn summary_is_escaped() {
        let ics = build_ics(&snapshot_with("Lab, notes; draft"));
        assert!(ics.contains("SUMMARY:Lab\\, notes\\; draft\r\n"));
    }

    #[test]
    fn the_output_parses_as_a_calendar() {
        let ics = build_ics(&snapshot_with("Meet Maya"));
        let calendar: icalendar::Calendar = ics.parse().expect("valid iCalendar");
        assert_eq!(calendar.events().count(), 1);
    }
}
