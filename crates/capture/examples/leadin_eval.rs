//! The title lead-in experiment (docs/experiments/title-leadin.md).
//!
//! `cargo run -p capture --example leadin_eval` scores the code rule on both eval sets.
//! With `TYPESAFE_API_KEY` set it also asks Jev to pick the title start, for each way of
//! listing the candidates, and prints its accuracy. With `--record` it writes Jev's
//! answers to `tests/fixtures/leadins.jev.json` so the tests can replay them offline.

use std::collections::BTreeMap;

use capture::{leadin, pipeline::base_title};
use chrono::{TimeZone, Utc};
use judge::{ClauseQuestion, JevJudge, Judge};
use todo_core::LocalContext;

const SETS: [(&str, &str); 2] = [
    ("design set", include_str!("../tests/fixtures/leadins.txt")),
    (
        "holdout set",
        include_str!("../tests/fixtures/leadins.holdout.txt"),
    ),
];

type Generator = fn(&str) -> Vec<String>;
const GENERATORS: [(&str, Generator); 1] = [("the grammar and link words", leadin::options)];

fn main() -> anyhow::Result<()> {
    let record = std::env::args().any(|arg| arg == "--record");
    let ctx = LocalContext {
        now: Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
        tz: chrono_tz::America::New_York,
    };
    let jev = JevJudge::from_env().ok();
    let runtime = tokio::runtime::Runtime::new()?;
    let mut recorded: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();

    for (set_name, text) in SETS {
        let cases: Vec<(&str, &str)> = text
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .filter_map(|line| line.split_once(" => "))
            .collect();
        // What the pipeline hands to the lead-in step: the clause without its date phrases.
        let baselines: Vec<String> = cases
            .iter()
            .map(|(clause, _)| base_title(clause, &dates::candidates(clause, &ctx)))
            .collect();
        let total = cases.len();
        let kept = cases
            .iter()
            .zip(&baselines)
            .filter(|((_, expected), baseline)| baseline == expected)
            .count();
        println!("\n== {set_name}: {total} cases");
        println!("today (clean_title only): {kept}/{total}");
        let mut ruled_ok = 0;
        for ((clause, expected), baseline) in cases.iter().zip(&baselines) {
            let ruled = leadin::strip(baseline).unwrap_or_else(|| baseline.clone());
            if ruled == *expected {
                ruled_ok += 1;
            } else {
                println!("  rule miss: {clause}\n      got: {ruled}\n      want: {expected}");
            }
        }
        println!("code rule:                {ruled_ok}/{total}");

        let Some(jev) = &jev else { continue };
        for (generator_name, generator) in GENERATORS {
            let questions: Vec<ClauseQuestion> = cases
                .iter()
                .zip(&baselines)
                .map(|((clause, _), baseline)| ClauseQuestion {
                    clause: (*clause).to_owned(),
                    titles: generator(baseline),
                    days: Vec::new(),
                    times: Vec::new(),
                    durations: Vec::new(),
                    local_now: "Monday 2026-10-12 09:00".into(),
                })
                .collect();
            let answers = runtime.block_on(jev.judge(&questions))?;
            let mut right = 0;
            let mut reachable = 0;
            let picks = recorded
                .entry(format!("{set_name} / {generator_name}"))
                .or_default();
            for ((question, answer), (clause, expected)) in
                questions.iter().zip(&answers).zip(&cases)
            {
                let pick = answer.title.unwrap_or(0).min(question.titles.len() - 1);
                picks.insert((*clause).to_owned(), pick);
                reachable += usize::from(question.titles.iter().any(|title| title == expected));
                if question.titles[pick] == *expected {
                    right += 1;
                } else {
                    println!(
                        "  jev miss ({generator_name}): {clause}\n      options: {:?}\n      got: {}\n      want: {expected}",
                        question.titles, question.titles[pick]
                    );
                }
            }
            println!(
                "Jev, candidates from {generator_name}: {right}/{total} (the right title was on offer in {reachable})"
            );
        }
    }
    if jev.is_none() {
        println!("\nTYPESAFE_API_KEY not set: Jev not run");
    } else if record {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/leadins.jev.json"
        );
        std::fs::write(path, serde_json::to_string_pretty(&recorded)? + "\n")?;
        println!("recorded {path}");
    }
    Ok(())
}
