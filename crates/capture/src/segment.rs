//! Clause segmentation (ARCHITECTURE.md section 5, step 1). Code only.
//!
//! Ported from v1 `judge::split_sentences` and its filters: hedged self-talk and
//! policy lines are dropped, time fragments and pronoun clauses merge into the
//! clause before them, and compound sentences split on the first coordinating
//! marker where both sides have at least two words.

fn split_sentences(text: &str) -> Vec<String> {
    let raw: Vec<String> = text
        .split(['.', '!', '?', '\n'])
        .flat_map(split_compound)
        .collect();
    let mut merged: Vec<String> = Vec::new();
    for clause in raw {
        if is_hedged_self_talk(&clause) {
            continue;
        }
        if is_policy_line(&clause) {
            continue;
        }
        if is_time_fragment(&clause)
            && let Some(previous) = merged.last_mut()
        {
            previous.push(' ');
            previous.push_str(clause.trim());
            continue;
        }
        if (is_pronoun_clause(&clause) || is_pronoun_object_action(&clause))
            && let Some(previous) = merged.last_mut()
        {
            previous.push(' ');
            previous.push_str(clause.trim());
            continue;
        }
        merged.push(clause);
    }
    merged
}

fn is_hedged_self_talk(clause: &str) -> bool {
    let lower = clause.trim().to_ascii_lowercase();
    [
        "i should probably ",
        "i guess ",
        "maybe i'll ",
        "maybe i will ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn is_pronoun_clause(clause: &str) -> bool {
    [
        "it ", "that ", "this ", "them ", "it,", "that,", "this,", "them,",
    ]
    .iter()
    .any(|prefix| clause.trim().to_ascii_lowercase().starts_with(prefix))
}

fn is_pronoun_object_action(clause: &str) -> bool {
    let words: Vec<&str> = clause.split_whitespace().collect();
    let words = words.strip_prefix(&["and"]).unwrap_or(&words);
    words.len() >= 2
        && [
            "upload", "send", "share", "email", "finish", "review", "read", "submit", "return",
        ]
        .contains(
            &words[0]
                .trim_matches(|c: char| !c.is_ascii_alphabetic())
                .to_ascii_lowercase()
                .as_str(),
        )
        && ["it", "that", "this", "them"].contains(
            &words[1]
                .trim_matches(|c: char| !c.is_ascii_alphabetic())
                .to_ascii_lowercase()
                .as_str(),
        )
}

fn is_time_fragment(clause: &str) -> bool {
    let lower = clause.trim().to_ascii_lowercase();
    lower.split_whitespace().count() <= 5
        && [
            "by ",
            "before ",
            "today",
            "tomorrow",
            "next ",
            "this weekend",
            "weekend",
            "just ",
            "only ",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn split_compound(sentence: &str) -> Vec<String> {
    let sentence = sentence.trim();
    let lower = sentence.to_ascii_lowercase();
    let mut candidates = Vec::new();
    for marker in [",", " oh and ", " also ", " and ", " then ", " plus "] {
        let mut offset = 0;
        while let Some(relative) = lower[offset..].find(marker) {
            let index = offset + relative;
            let left = sentence[..index].trim();
            let right = sentence[index + marker.len()..].trim();
            let right_first = right
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_ascii_lowercase();
            let bare_coordination = marker == " and "
                && !is_action_word(&right_first)
                && !["i", "you", "we", "they", "a", "an", "the"].contains(&right_first.as_str());
            if word_count_without_filler(left) >= 2
                && word_count_without_filler(right) >= 2
                && !bare_coordination
            {
                candidates.push((index, marker.len(), left.to_owned(), right.to_owned()));
            }
            offset = index + marker.len();
        }
    }
    if let Some((_, _, left, right)) = candidates.into_iter().min_by_key(|(index, _, _, _)| *index)
    {
        let mut clauses = split_compound(&left);
        clauses.extend(split_compound(&right));
        return clauses;
    }
    if sentence.is_empty() {
        Vec::new()
    } else {
        vec![sentence.to_owned()]
    }
}

fn is_action_word(word: &str) -> bool {
    [
        "buy", "call", "clean", "email", "finish", "find", "make", "read", "review", "schedule",
        "book", "pay", "send", "share", "start", "submit", "upload", "write",
    ]
    .contains(
        &word
            .trim_matches(|c: char| !c.is_ascii_alphabetic())
            .to_ascii_lowercase()
            .as_str(),
    )
}

fn word_count_without_filler(sentence: &str) -> usize {
    sentence
        .split_whitespace()
        .map(|word| word.trim_matches(|character: char| !character.is_ascii_alphanumeric()))
        .filter(|word| {
            !["ugh", "uh", "um", "like", "okay", "so"].contains(&word.to_ascii_lowercase().as_str())
        })
        .filter(|word| !word.is_empty())
        .count()
}

/// Check if a clause is a policy line (not a task).
/// Policy lines contain: late work, grading, attendance, "unless", percent penalties.
pub fn is_policy_line(clause: &str) -> bool {
    let lower = clause.to_lowercase();
    let policy_keywords = [
        "late work",
        "grading",
        "attendance",
        "unless",
        "percent",
        "%",
        "penalty",
        "penalties",
        "late penalty",
        "extension",
        "arranged in advance",
        "drops",
        "drop policy",
        "late loses",
        "loses percent",
    ];
    policy_keywords.iter().any(|kw| lower.contains(kw))
}

/// Clauses of a capture, in order. Blank clauses are dropped.
pub fn split_clauses(text: &str) -> Vec<String> {
    split_sentences(&join_wrapped_lines(text))
        .into_iter()
        .filter(|clause| !clause.trim().is_empty())
        .collect()
}

/// Joins hard line wraps so a sentence broken across lines stays whole. A newline
/// stays a break only after sentence punctuation or at a blank line.
pub fn join_wrapped_lines(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\n' {
            out.push(ch);
        } else if chars.peek() == Some(&'\n')
            || out.trim_end().ends_with(['.', '!', '?'])
            || chars.peek().is_none()
        {
            out.push('\n');
        } else {
            out.push(' ');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hard_wrap_mid_sentence_is_joined() {
        assert_eq!(
            split_clauses(
                "Maybe an
hour on the lab report."
            ),
            ["Maybe an hour on the lab report"]
        );
    }

    #[test]
    fn a_newline_after_punctuation_or_a_blank_line_still_breaks() {
        assert_eq!(
            split_clauses(
                "Call mom.
Buy milk

Write the essay"
            ),
            ["Call mom", "Buy milk", "Write the essay"]
        );
    }
}
