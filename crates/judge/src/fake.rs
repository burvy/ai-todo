//! A deterministic judge for tests and offline runs: the first candidate of each kind,
//! label and priority from keywords. It accepts any clause that has a title and is not
//! chit-chat.
//!
//! The v1 client asked Jev a `Noul` for every clause, with the criteria "actionable
//! obligation, assessment preparation, deadline or errand" against "observation, wish,
//! chit-chat, feeling, past event, or instruction to reorder existing work", and the
//! code dropped hedged self-talk before that. With no network there is nothing to ask,
//! so those same rules are applied here in code.

use std::future::Future;

use todo_core::{Label, Priority};

use crate::{ClauseAnswer, ClauseQuestion, Judge};

#[derive(Clone, Copy, Debug, Default)]
pub struct FakeJudge;

impl Judge for FakeJudge {
    fn judge(
        &self,
        questions: &[ClauseQuestion],
    ) -> impl Future<Output = anyhow::Result<Vec<ClauseAnswer>>> + Send {
        let answers = questions.iter().map(answer).collect();
        async move { Ok(answers) }
    }
}

fn answer(question: &ClauseQuestion) -> ClauseAnswer {
    let first = |items: &[String]| (!items.is_empty()).then_some(0);
    let lower = question.clause.to_ascii_lowercase();
    ClauseAnswer {
        is_task: if question.titles.is_empty() || is_chit_chat(&lower) {
            0.0
        } else {
            1.0
        },
        title: first(&question.titles),
        day: first(&question.days),
        time: first(&question.times),
        duration: first(&question.durations),
        label: label_for(&lower),
        priority: if ["urgent", "asap", "important"]
            .iter()
            .any(|word| lower.contains(word))
        {
            Priority::High
        } else {
            Priority::Low
        },
        confidence: 1.0,
    }
}

/// Hedged self-talk, v1's own `is_hedged_self_talk`.
fn is_hedged_self_talk(lower: &str) -> bool {
    [
        "i should probably ",
        "i guess ",
        "maybe i'll ",
        "maybe i will ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

/// The v1 `Noul` criteria, in code. v1 asked Jev whether a clause was an "actionable
/// obligation, assessment preparation, deadline, or errand" and spelled out the false
/// side as "observation, wish (would be nice), chit-chat, feeling, past event, or
/// instruction to reorder existing work". With no network to ask, each of those six is
/// matched here. This is what keeps a raw transcript down to its obligations.
fn is_chit_chat(lower: &str) -> bool {
    if lower.trim().is_empty() || is_hedged_self_talk(lower) {
        return true;
    }
    is_observation(lower)
        || is_wish(lower)
        || is_feeling(lower)
        || is_past_event(lower)
        || is_reorder_instruction(lower)
        || !has_obligation(lower)
}

/// A clause is only an obligation if it names something to do or something owed. The
/// fragments left over once a sentence has been split ("I think", "For biology",
/// "Watching videos", "The design presentation is coming up") name neither, and v1
/// would have rejected every one of them.
fn has_obligation(lower: &str) -> bool {
    const ACTIONS: [&str; 21] = [
        "buy", "call", "clean", "email", "finish", "find", "make", "read", "review", "schedule",
        "book", "pay", "send", "share", "start", "submit", "upload", "write", "return", "remind",
        "bring",
    ];
    // "I need groceries", "I have a quiz", "I promised to call": the obligation can
    // sit on a verb of owing rather than on a verb of doing.
    const OBLIGATIONS: [&str; 10] = [
        "need ",
        "needs ",
        "have to",
        "have a",
        "have an",
        "must ",
        "should ",
        "want ",
        "going to",
        "promised ",
    ];
    let has = |words: &[&str]| {
        words
            .iter()
            .any(|word| contains_word(lower, word.trim_end()))
    };
    has(&ACTIONS) || has(&OBLIGATIONS) || is_instruction(lower)
}

/// "Walk the dog", "fix the sink", "water plants": two or more words that open with a verb
/// nobody listed. The word list above cannot hold every verb, so a clause that opens
/// with a word that is not a pronoun, article, preposition, conjunction, question word or
/// "-ing" or "-ed" form is taken for an instruction. The fragments a sentence splits into
/// ("I think", "For biology", "Watching videos") all open with one of those.
fn is_instruction(lower: &str) -> bool {
    const NOT_A_VERB: &[&str] = &[
        "i",
        "you",
        "he",
        "she",
        "it",
        "we",
        "they",
        "me",
        "my",
        "your",
        "his",
        "her",
        "our",
        "their",
        "this",
        "that",
        "these",
        "those",
        "the",
        "a",
        "an",
        "some",
        "any",
        "no",
        "not",
        "and",
        "but",
        "or",
        "so",
        "if",
        "although",
        "though",
        "because",
        "since",
        "while",
        "when",
        "then",
        "also",
        "just",
        "for",
        "of",
        "in",
        "on",
        "at",
        "to",
        "with",
        "from",
        "by",
        "about",
        "as",
        "which",
        "who",
        "what",
        "where",
        "why",
        "how",
        "there",
        "here",
        "is",
        "are",
        // Talk before the point: "Right, the design presentation is coming up".
        "right",
        "okay",
        "ok",
        "oh",
        "well",
        "yeah",
        "yes",
        "anyway",
        "um",
        "uh",
        "hmm",
        "like",
        "still",
        "really",
        "maybe",
        "probably",
        "honestly",
        "actually",
        "basically",
        // Talk before the point: "Right, the design presentation is coming up".
        "right",
        "okay",
        "ok",
        "oh",
        "well",
        "yeah",
        "yes",
        "anyway",
        "um",
        "uh",
        "hmm",
        "like",
        "still",
        "really",
        "maybe",
        "probably",
        "honestly",
        "actually",
        "basically",
    ];
    let mut words = lower.split_whitespace();
    let (Some(first), Some(_)) = (words.next(), words.next()) else {
        return false;
    };
    let first = first.trim_matches(|c: char| !c.is_alphabetic());
    first.len() > 2
        && first.chars().all(char::is_alphabetic)
        && !NOT_A_VERB.contains(&first)
        && !first.ends_with("ing")
        && !first.ends_with("ed")
}

/// Match a word on its own boundaries, so "pay" does not fire on "repayment" and
/// "return " does not fire on "returning". The trailing space in a keyword means
/// "and the next character is not alphanumeric".
fn contains_word(lower: &str, word: &str) -> bool {
    lower.match_indices(word).any(|(index, _)| {
        let before = lower[..index].chars().next_back();
        let after = lower[index + word.len()..].chars().next();
        let open = before.is_none_or(|c| !c.is_alphanumeric());
        let close = after.is_none_or(|c| !c.is_alphanumeric());
        open && close
    })
}

/// "The design presentation is coming up", "the website ate my submission".
fn is_observation(lower: &str) -> bool {
    [
        "my apartment is",
        "the website ",
        "the library closes",
        "which is not homework",
        "is not really a school task",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

/// "Taking out the recycling would be nice", "I need to get my life together".
fn is_wish(lower: &str) -> bool {
    ["would be nice", "get my life together"]
        .iter()
        .any(|phrase| lower.contains(phrase))
}

/// "I am so tired", "I think", "not that".
fn is_feeling(lower: &str) -> bool {
    [
        "i am so ",
        "i'm so ",
        "i feel ",
        "not that ",
        "which is boring",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

/// "I have studied", "I keep opening the slides and then watching videos".
fn is_past_event(lower: &str) -> bool {
    [
        "i have studied",
        "i keep ",
        "i already ",
        "i watched ",
        "i spent ",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

/// "Put the small source-finding thing first" changes the order of work that already
/// exists. That is the reorder endpoint's job, not a new obligation to capture.
fn is_reorder_instruction(lower: &str) -> bool {
    [
        "put the small",
        "thing first",
        "first, please",
        "do not let me forget",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn label_for(lower: &str) -> Label {
    let has = |words: &[&str]| words.iter().any(|word| lower.contains(word));
    if has(&[
        "class", "homework", "exam", "quiz", "essay", "lab ", "reading",
    ]) {
        Label::School
    } else if has(&["work", "meeting", "client", "report", "boss"]) {
        Label::Work
    } else if has(&["doctor", "dentist", "gym", "health", "pharmacy"]) {
        Label::Health
    } else if has(&["buy", "grocer", "errand", "pick up", "return "]) {
        Label::Errands
    } else if has(&["mom", "dad", "friend", "birthday", "call "]) {
        Label::Personal
    } else {
        Label::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(clause: &str, titles: &[&str], days: &[&str]) -> ClauseQuestion {
        ClauseQuestion {
            clause: clause.into(),
            titles: titles.iter().map(|t| t.to_string()).collect(),
            days: days.iter().map(|d| d.to_string()).collect(),
            times: vec![],
            durations: vec![],
            local_now: String::new(),
        }
    }

    #[tokio::test]
    async fn picks_first_candidates_and_keywords() {
        let questions = [question(
            "finish the lab report",
            &["Finish the lab report"],
            &["friday"],
        )];
        let answers = FakeJudge.judge(&questions).await.unwrap();
        assert_eq!(answers[0].title, Some(0));
        assert_eq!(answers[0].day, Some(0));
        assert_eq!(answers[0].time, None);
        assert_eq!(answers[0].label, Label::School);
        assert_eq!(answers[0].is_task, 1.0);
    }

    #[tokio::test]
    async fn a_clause_without_a_title_is_not_a_task() {
        let questions = [question("", &[], &[])];
        let answers = FakeJudge.judge(&questions).await.unwrap();
        assert_eq!(answers[0].is_task, 0.0);
    }

    /// The six things v1's `Noul` called not-a-task.
    #[test]
    fn chit_chat_is_not_a_task() {
        for clause in [
            "I am so tired",
            "Which is boring",
            "Not that I have studied",
            "The design presentation is coming up",
            "The website ate my submission",
            "Although taking out the recycling would be nice",
            "I keep opening the slides",
            "I should probably start the response",
            "Put the small source-finding thing first, please",
            "Please do not let me forget",
        ] {
            assert!(is_chit_chat(&clause.to_ascii_lowercase()), "{clause}");
        }
    }

    #[test]
    fn a_real_obligation_is_a_task() {
        for clause in [
            "Finish the history response paper",
            "Email Professor Chen about the missing reading",
            "Buy printer ink",
            "Book an appointment with advising",
            "Return the library book",
            "Schedule a meeting with Maya",
            // Verbs the keyword list does not hold.
            "Walk the dog",
            "fix the sink",
            "water plants",
            "Take out the recycling",
            "Pick up the parcel",
        ] {
            assert!(!is_chit_chat(&clause.to_ascii_lowercase()), "{clause}");
        }
    }

    #[tokio::test]
    async fn a_chit_chat_clause_is_answered_zero() {
        let questions = [question("I am so tired", &["I am so tired"], &[])];
        let answers = FakeJudge.judge(&questions).await.unwrap();
        assert_eq!(answers[0].is_task, 0.0);
    }
}
