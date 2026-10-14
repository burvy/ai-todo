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
    OBLIGATIONS
        .iter()
        .any(|word| contains_word(lower, word.trim_end()))
        || is_instruction(lower)
}

/// Verbs of thinking, feeling and perceiving. "Can you believe it" comments on the
/// news, so a request may only step past an ask on one of these when the verb then
/// hands the errand on — see `is_action_handed_on`.
const MENTAL_VERBS: &[&str] = &[
    "believe", "imagine", "guess", "think", "hear", "feel", "remember", "tell", "know",
];

/// Words that open a statement, a question or a fragment, never an instruction.
const NOT_A_VERB: &[&str] = &[
    // pronouns
    "i",
    "i'm",
    "i've",
    "i'll",
    "i'd",
    "im",
    "you",
    "you're",
    "he",
    "she",
    "it",
    "it's",
    "we",
    "they",
    "me",
    "him",
    "her",
    "us",
    "them",
    "someone",
    "somebody",
    "something",
    "everyone",
    "everything",
    "nothing",
    "there",
    "there's",
    "here",
    "that's",
    // articles and determiners
    "the",
    "a",
    "an",
    "this",
    "that",
    "these",
    "those",
    "my",
    "your",
    "his",
    "its",
    "our",
    "their",
    "some",
    "any",
    "no",
    "every",
    "each",
    "all",
    "both",
    // prepositions
    "for",
    "of",
    "in",
    "on",
    "at",
    "by",
    "with",
    "about",
    "from",
    "to",
    "into",
    "after",
    "before",
    "during",
    "since",
    "until",
    // conjunctions and question words
    "although",
    "though",
    "because",
    "if",
    "when",
    "while",
    "or",
    "but",
    "which",
    "who",
    "what",
    "where",
    "why",
    "how",
    // auxiliaries and copulas
    "is",
    "are",
    "was",
    "were",
    "am",
    "be",
    "been",
    "do",
    "does",
    "did",
    "can",
    "could",
    "would",
    "will",
    // negation and hedges
    "not",
    "maybe",
];

/// Whether the clause is shaped like an instruction: "walk the dog", "water plants",
/// "can you fix the sink". No list of verbs can name every chore, but the words a
/// statement or a fragment opens with are a small closed set (pronouns, articles,
/// prepositions, auxiliaries, conjunctions), so a clause whose first real word is none
/// of those, nor a gerund or a past form, opens with the thing to do.
fn is_instruction(lower: &str) -> bool {
    /// Fillers and connectives a spoken task starts with before its verb.
    const LEAD: [&str; 16] = [
        "oh", "and", "also", "then", "so", "um", "uh", "okay", "ok", "right", "well", "please",
        "now", "plus", "just", "pls",
    ];
    /// Ways to ask someone else to do it: "can you", "could someone".
    const ASKS: [&str; 4] = ["can", "could", "would", "will"];
    const ASKED: [&str; 4] = ["you", "someone", "somebody", "u"];
    /// Phrases that hand the task to the app: "remind me to", "don't forget to".
    const HANDOFFS: [&[&str]; 3] = [
        &["remind", "me", "to"],
        &["don't", "forget", "to"],
        &["dont", "forget", "to"],
    ];
    let words: Vec<&str> = lower
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\''))
        .filter(|word| !word.is_empty())
        .collect();
    let mut rest = words.as_slice();
    loop {
        rest = match rest {
            [first, tail @ ..] if LEAD.contains(first) => tail,
            [ask, who, tail @ ..]
                if ASKS.contains(ask)
                    && ASKED.contains(who)
                    && verb_and_rest(&LEAD, tail).is_none_or(|words| match words {
                        [verb, rest @ ..] if MENTAL_VERBS.contains(verb) => {
                            is_action_handed_on(rest)
                        }
                        _ => true,
                    }) =>
            {
                tail
            }
            _ => match HANDOFFS.iter().find(|handoff| rest.starts_with(handoff)) {
                Some(handoff) => &rest[handoff.len()..],
                None => break,
            },
        };
    }
    // One word alone ("Thanks", "Groceries") is too little to call an instruction.
    rest.len() >= 2
        && rest.first().is_some_and(|first| {
            !NOT_A_VERB.contains(first) && !is_gerund(first) && !is_past_form(first)
        })
}

/// The words starting at the verb a request acts on: the first word after "can you"
/// that is not a lead-in, so "can you please fix the sink" still finds "fix".
fn verb_and_rest<'a>(lead: &[&str], words: &'a [&str]) -> Option<&'a [&'a str]> {
    let index = words.iter().position(|word| !lead.contains(word))?;
    Some(&words[index..])
}

/// Whether the mind verb in a request still hands on the errand: "remember to call
/// mom" and "tell mom to call" do, while what follows otherwise ("when we moved",
/// "that we are late", a bare "it") is a remark on the moment, not an errand.
fn is_action_handed_on(words: &[&str]) -> bool {
    /// The person the errand is passed to: a name ("mom") or an object pronoun
    /// ("tell her to call").
    const SOMEONE: &[&str] = &["me", "her", "him", "us", "them"];
    let is_action = |verb: &str| !NOT_A_VERB.contains(&verb) && !MENTAL_VERBS.contains(&verb);
    match words {
        ["to", verb, ..] => is_action(verb),
        [someone, "to", verb, ..] if SOMEONE.contains(someone) || !NOT_A_VERB.contains(someone) => {
            is_action(verb)
        }
        _ => false,
    }
}

/// "watching", "studying", but not "bring" or "string": an -ing word is a gerund when
/// what comes before the -ing has a vowel of its own.
fn is_gerund(word: &str) -> bool {
    word.strip_suffix("ing")
        .is_some_and(|stem| stem.chars().any(|c| "aeiouy".contains(c)))
}

/// "studied", "watched", but not "feed" or "need".
fn is_past_form(word: &str) -> bool {
    word.ends_with("ed") && !word.ends_with("eed") && word.len() > 3
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

    #[test]
    fn a_chore_is_a_task_whatever_its_verb() {
        // Owner's phone test on 236ba7a: "walk the dog", "fix the sink" and "water
        // plants" were refused while "buy bread" was taken, because only a fixed list
        // of verbs counted as something to do.
        for clause in [
            "walk the dog",
            "fix the sink",
            "water plants",
            "mow the lawn",
            "clean the bathroom",
            "feed the cat",
            "buy bread",
            "call mom",
            "pay rent",
            "email Sam",
            "Oh, and walk the dog",
            "can you fix the sink",
            "please water plants tomorrow",
            "remind me to feed the cat",
            "bring snacks",
        ] {
            assert!(!is_chit_chat(&clause.to_ascii_lowercase()), "{clause}");
        }
    }

    #[test]
    fn a_fragment_or_a_statement_is_still_not_a_task() {
        for clause in [
            "I think",
            "For biology",
            "Watching videos",
            "Studied all night",
            "The dog is asleep",
            "Which is boring",
        ] {
            assert!(is_chit_chat(&clause.to_ascii_lowercase()), "{clause}");
        }
    }

    /// A request is a task only when it asks for an action. A mind verb after
    /// the ask is the speaker reacting, unless it hands the action on: "remember
    /// to call mom" and "tell mom to call" are errands, "remember when" and
    /// "tell her that" are remarks.
    #[test]
    fn a_request_is_an_instruction_only_for_an_action_verb() {
        for clause in [
            "Can you believe it",
            "could you imagine that",
            "would you hear me out",
            "can you think about it",
            "can you remember when we moved",
            "can you tell her that we are late",
        ] {
            assert!(is_chit_chat(&clause.to_ascii_lowercase()), "{clause}");
        }
        for clause in [
            "can you call mom",
            "could you fix the sink",
            // "see" a shape in the world: a concrete look, not a state of mind
            // (Mario's review at 8ccee4c caught the regression).
            "can you see the sink",
            "can you remember to call mom",
            "can you tell mom to call",
            "will you tell her to call",
        ] {
            assert!(!is_chit_chat(&clause.to_ascii_lowercase()), "{clause}");
        }
    }

    #[tokio::test]
    async fn a_request_is_a_task_only_for_an_action_verb() {
        let questions = [
            question("Can you believe it", &["Can you believe it"], &[]),
            question("could you imagine that", &["could you imagine that"], &[]),
            question(
                "can you remember when we moved",
                &["can you remember when we moved"],
                &[],
            ),
            question(
                "can you remember to call mom",
                &["Can you remember to call mom"],
                &[],
            ),
            question(
                "can you tell mom to call",
                &["Can you tell mom to call"],
                &[],
            ),
        ];
        let answers = FakeJudge.judge(&questions).await.unwrap();
        assert_eq!(answers[0].is_task, 0.0);
        assert_eq!(answers[1].is_task, 0.0);
        assert_eq!(answers[2].is_task, 0.0);
        assert_eq!(answers[3].is_task, 1.0);
        assert_eq!(answers[4].is_task, 1.0);
    }

    #[tokio::test]
    async fn a_chit_chat_clause_is_answered_zero() {
        let questions = [question("I am so tired", &["I am so tired"], &[])];
        let answers = FakeJudge.judge(&questions).await.unwrap();
        assert_eq!(answers[0].is_task, 0.0);
    }
}
