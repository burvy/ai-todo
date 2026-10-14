//! Golden test: each ramble in `fixtures/rambles.md` goes through the pipeline with
//! `FakeJudge` at a fixed instant, and the drafts must match `fixtures/rambles.expected.txt`.
//!
//! To accept a change in output, run `UPDATE_GOLDEN=1 cargo test -p capture --test golden`
//! and review the diff of the expected file.

use std::fmt::Write as _;

use capture::capture_text;
use chrono::{TimeZone, Utc};
use judge::FakeJudge;
use todo_core::{DaySettings, LocalContext, Source, TaskDraft};

const RAMBLES: &str = "tests/fixtures/rambles.md";
const EXPECTED: &str = "tests/fixtures/rambles.expected.txt";

/// Sunday 2026-10-11 21:00 in New York, the same night the dates tests use.
fn ctx() -> LocalContext {
    LocalContext {
        now: Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
        tz: chrono_tz::America::New_York,
    }
}

/// The ramble bodies: each `## ` heading starts one ramble.
fn rambles(text: &str) -> Vec<String> {
    text.split("\n## ")
        .skip(1)
        .map(|section| {
            section
                .split_once('\n')
                .map(|(_, body)| body)
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

fn render(drafts: &[TaskDraft]) -> String {
    let mut out = String::new();
    for draft in drafts {
        writeln!(
            out,
            "{} | {} min | {:?} | {:?} | {:?}",
            draft.title, draft.duration_min, draft.when, draft.label, draft.priority
        )
        .unwrap();
    }
    out
}

#[tokio::test]
async fn rambles_match_the_golden_output() {
    let source = std::fs::read_to_string(RAMBLES).expect("fixture exists");
    let mut actual = String::new();
    for (index, ramble) in rambles(&source).iter().enumerate() {
        writeln!(actual, "## ramble {}", index + 1).unwrap();
        let drafts = capture_text(
            &FakeJudge,
            ramble,
            Source::Voice,
            DaySettings::default().deadline,
            &ctx(),
        )
        .await
        .expect("fake judge never fails");
        actual.push_str(&render(&drafts));
    }

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(EXPECTED, &actual).expect("write expected file");
        return;
    }
    let expected = std::fs::read_to_string(EXPECTED).expect("expected file exists");
    assert_eq!(actual, expected);
}

#[test]
fn the_fixture_has_all_its_rambles() {
    let source = std::fs::read_to_string(RAMBLES).expect("fixture exists");
    assert_eq!(rambles(&source).len(), 4);
}
