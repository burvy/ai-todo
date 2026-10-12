//! Clause segmentation (ARCHITECTURE.md section 5, step 1). Code only.
//!
//! Ported from v1 `judge::split_sentences` and its filters: hedged self-talk and
//! policy lines are dropped, time fragments and pronoun clauses merge into the
//! clause before them, and compound sentences split on the first coordinating
//! marker where both sides have at least two words.
//!
//! Captures arrive as hard-wrapped text (a transcript or a PDF), so the lines are
//! joined back into sentences first: a newline only ends a clause after sentence
//! punctuation or a blank line. List items always start their own clause.

fn join_wrapped_lines(text: &str) -> String {
    let mut logical: Vec<String> = Vec::new();
    let mut starts_clause = true;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            starts_clause = true;
            continue;
        }
        if starts_clause || starts_list_item(line) {
            logical.push(line.to_owned());
            starts_clause = false;
        } else {
            let previous = logical.last_mut().expect("a clause is open");
            previous.push(' ');
            previous.push_str(line);
        }
        if ends_sentence(line) {
            starts_clause = true;
        }
    }
    logical.join("\n")
}

fn ends_sentence(line: &str) -> bool {
    line.trim_end()
        .chars()
        .next_back()
        .is_some_and(|last| matches!(last, '.' | '!' | '?' | '…'))
}

/// A list item (`- `, `* `, `+ `, `1. `, `2) `) always begins a new clause.
fn starts_list_item(line: &str) -> bool {
    if ["- ", "* ", "+ "].contains(&&line[..line.len().min(2)]) {
        return true;
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0
        && matches!(line[digits..].chars().next(), Some('.' | ')'))
        && line[digits + 1..].starts_with(' ')
}

fn split_sentences(text: &str) -> Vec<String> {
    let raw: Vec<String> = join_wrapped_lines(text)
        .split(['.', '!', '?', '\n'])
        .flat_map(split_compound)
        .collect();
    let mut merged: Vec<String> = Vec::new();
    for clause in raw {
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
            // "maybe an hour", "maybe Friday afternoon": a hedged time or duration
            // belongs to the obligation in front of it, not to a task of its own.
            "maybe ",
            "probably ",
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

/// Clauses of a capture, in order. Blank clauses are dropped.
pub fn split_clauses(text: &str) -> Vec<String> {
    split_sentences(text)
        .into_iter()
        .filter(|clause| !clause.trim().is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hard_wrap_is_joined_into_one_clause() {
        let joined = join_wrapped_lines("review chapters six and seven, maybe an\nhour");
        assert_eq!(joined, "review chapters six and seven, maybe an hour");
    }

    #[test]
    fn a_blank_line_starts_a_new_clause() {
        let joined = join_wrapped_lines("first thing\nwrapped\n\nsecond thing");
        assert_eq!(joined, "first thing wrapped\nsecond thing");
    }

    #[test]
    fn sentence_punctuation_ends_a_clause() {
        let joined = join_wrapped_lines("one thing done.\ntwo things left");
        assert_eq!(joined, "one thing done.\ntwo things left");
    }

    #[test]
    fn list_items_start_their_own_clause() {
        let joined = join_wrapped_lines("shopping:\n- printer ink\n- paper\n2. staples");
        assert_eq!(joined, "shopping:\n- printer ink\n- paper\n2. staples");
    }

    #[test]
    fn wrapping_no_longer_splits_a_sentence_in_two() {
        let clauses =
            split_clauses("please make a task to review chapters six and seven, maybe an\nhour");
        assert!(
            clauses
                .iter()
                .any(|clause| clause.contains("maybe an hour")),
            "wrapped tail stayed attached: {clauses:?}"
        );
        assert!(
            !clauses.iter().any(|clause| clause.trim() == "Hour"),
            "the wrap did not leave a bare fragment: {clauses:?}"
        );
    }

    #[test]
    fn policy_keywords_are_no_longer_dropped_from_segments() {
        let clauses = split_clauses("Pick up the dry cleaning, drops off at 5");
        // Segmentation splits on the comma; neither half may be dropped for "drops".
        assert!(
            clauses.contains(&"drops off at 5".to_string()),
            "'drops' clause was dropped: {clauses:?}"
        );
        assert!(
            clauses.contains(&"Pick up the dry cleaning".to_string()),
            "first clause was dropped: {clauses:?}"
        );
    }

    #[test]
    fn percent_is_no_longer_a_policy_keyword() {
        let clauses = split_clauses("Get the report to 100%");
        assert!(
            clauses.contains(&"Get the report to 100%".to_string()),
            "segment wrongly dropped the percent clause: {clauses:?}"
        );
    }
}
