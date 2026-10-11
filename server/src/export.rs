use axum::{extract::State, http::header, response::Response};
use chrono::Utc;

use crate::AppState;

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\r', "")
        .replace('\n', "\\n")
}

fn utc_stamp(value: chrono::DateTime<Utc>) -> String {
    value.format("%Y%m%dT%H%M%SZ").to_string()
}

pub async fn calendar_ics(State(state): State<AppState>) -> Response {
    let store = state.store.lock().unwrap();
    let ics = build_ics(&store.tasks, &store.placements);
    Response::builder()
        .header(header::CONTENT_TYPE, "text/calendar; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=rambl.ics",
        )
        .body(ics.into())
        .unwrap()
}

fn build_ics(tasks: &[todo_core::Task], placements: &[crate::PlacementOut]) -> String {
    let mut ics =
        String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//ai-todo//planner//EN\r\n");
    for placement in placements {
        let Some(task) = tasks.iter().find(|task| task.id == placement.task_id) else {
            continue;
        };
        let description = task
            .deadline
            .map(|deadline| format!("Planned by Rambl; deadline {}", deadline.to_rfc3339()))
            .unwrap_or_else(|| "Planned by Rambl".into());
        let uid = format!("{}-{}@ai-todo", task.id, placement.start.timestamp());
        ics.push_str("BEGIN:VEVENT\r\n");
        ics.push_str(&format!("UID:{}\r\n", escape(&uid)));
        ics.push_str(&format!("DTSTAMP:{}\r\n", utc_stamp(chrono::Utc::now())));
        ics.push_str(&format!("DTSTART:{}\r\n", utc_stamp(placement.start)));
        ics.push_str(&format!("DTEND:{}\r\n", utc_stamp(placement.end)));
        ics.push_str(&format!("SUMMARY:{}\r\n", escape(&task.title)));
        ics.push_str(&format!("DESCRIPTION:{}\r\n", escape(&description)));
        ics.push_str("BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT10M\r\n");
        ics.push_str(&format!(
            "DESCRIPTION:{}\r\nEND:VALARM\r\n",
            escape(&task.title)
        ));
        ics.push_str("END:VEVENT\r\n");
    }
    ics.push_str("END:VCALENDAR\r\n");
    ics
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use todo_core::Source;

    #[test]
    fn escapes_ical_text() {
        assert_eq!(escape("a,b;c\\d\n"), "a\\,b\\;c\\\\d\\n");
    }

    #[test]
    fn exports_events_and_round_trips() {
        let tasks = vec![
            todo_core::Task {
                id: "t1".into(),
                title: "Write, plan".into(),
                notes: String::new(),
                duration_min: 30,
                deadline: None,
                priority: 1,
                status: "todo".into(),
                confidence: 1.0,
                label: "Work".into(),
                source: Source::Text,
                position: 0,
                pinned_start: None,
            },
            todo_core::Task {
                id: "t2".into(),
                title: "Read; notes".into(),
                notes: String::new(),
                duration_min: 30,
                deadline: None,
                priority: 1,
                status: "todo".into(),
                confidence: 1.0,
                label: "School".into(),
                source: Source::Text,
                position: 1,
                pinned_start: None,
            },
        ];
        let start = chrono::Utc
            .with_ymd_and_hms(2026, 10, 12, 14, 0, 0)
            .unwrap();
        let placements = vec![
            crate::PlacementOut {
                task_id: "t1".into(),
                start,
                end: start + chrono::Duration::minutes(30),
            },
            crate::PlacementOut {
                task_id: "t2".into(),
                start: start + chrono::Duration::hours(1),
                end: start + chrono::Duration::minutes(90),
            },
        ];
        let ics = build_ics(&tasks, &placements);
        assert_eq!(ics.matches("BEGIN:VEVENT").count(), 2);
        assert_eq!(ics.matches("BEGIN:VALARM").count(), 2);
        assert!(ics.contains("\r\n"));
        assert!(crate::ics::parse_ics(&ics).is_ok());
    }
}
