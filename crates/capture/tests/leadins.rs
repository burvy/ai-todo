//! Title lead-ins ("Add a task to buy milk" is "Buy milk"), scored on the eval sets in
//! `tests/fixtures/leadins*.txt` (docs/experiments/title-leadin.md).
//!
//! Jev's choices were recorded once with `cargo run -p capture --example leadin_eval --
//! --record` and are replayed here, so the tests need no network. Re-record when the
//! candidates or the Jev question change: the recorded numbers are indexes into them.

use std::collections::BTreeMap;

use capture::{
    leadin,
    pipeline::{base_title, title_spans},
};
use chrono::{TimeZone, Utc};
use todo_core::LocalContext;

const DESIGN: &str = include_str!("fixtures/leadins.txt");
const HOLDOUT: &str = include_str!("fixtures/leadins.holdout.txt");
const RECORDED: &str = include_str!("fixtures/leadins.jev.json");
const CHOICE: &str = "the grammar and link words";

fn ctx() -> LocalContext {
    LocalContext {
        now: Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
        tz: chrono_tz::America::New_York,
    }
}

fn cases(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .filter_map(|line| line.split_once(" => "))
        .collect()
}

/// The clauses whose title is not the expected one, when `pick` chooses among the
/// candidates the pipeline offers.
fn misses(text: &str, pick: impl Fn(&str, &[String]) -> usize) -> Vec<&str> {
    cases(text)
        .into_iter()
        .filter(|(clause, expected)| {
            let titles = title_spans(clause, &dates::candidates(clause, &ctx()));
            titles[pick(clause, &titles)] != *expected
        })
        .map(|(clause, _)| clause)
        .collect()
}

fn recorded(set: &str) -> BTreeMap<String, usize> {
    let all: BTreeMap<String, BTreeMap<String, usize>> = serde_json::from_str(RECORDED).unwrap();
    all[&format!("{set} / {CHOICE}")].clone()
}

#[test]
fn the_eval_sets_are_big_enough_to_mean_something() {
    assert!(cases(DESIGN).len() >= 40);
    assert!(cases(HOLDOUT).len() >= 40);
}

#[test]
fn jevs_recorded_choices_get_every_design_clause_right() {
    let picks = recorded("design set");
    assert_eq!(
        misses(DESIGN, |clause, _| picks[clause]),
        Vec::<&str>::new()
    );
}

#[test]
fn jevs_recorded_choices_on_the_holdout_miss_only_two() {
    let picks = recorded("holdout set");
    assert_eq!(
        misses(HOLDOUT, |clause, _| picks[clause]),
        [
            "Hey, can you put a reminder in for me to renew the lease",
            "Add the task of mowing the lawn"
        ]
    );
}

#[test]
fn the_offline_judge_takes_the_first_candidate_which_is_the_grammars() {
    // `FakeJudge` answers `title: 0`.
    assert_eq!(misses(DESIGN, |_, _| 0), Vec::<&str>::new());
    assert!(
        misses(HOLDOUT, |_, _| 0).len() <= 13,
        "the grammar alone gets 33 of 46 on the holdout"
    );
}

#[test]
fn no_lead_in_is_cut_from_a_title_that_is_the_task() {
    // Whatever the judge picks, the grammar must not offer a cut for these as its first
    // candidate: the offline judge would take it.
    for (clause, expected) in cases(DESIGN).into_iter().chain(cases(HOLDOUT)) {
        if clause == expected {
            let base = base_title(clause, &dates::candidates(clause, &ctx()));
            assert_eq!(leadin::options(&base)[0], expected, "{clause}");
        }
    }
}
