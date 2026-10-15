//! The title of a task: the clause without the phrases the code captured as its date,
//! time and duration, cleaned of filler and stranded function words. The lead-in
//! question ([`crate::leadin`]) is asked on what is left.

use crate::leadin;
use dates::Candidate;

/// Title spans the code proposes: the clause without its date, time and duration
/// phrases. The whole clause is offered only when stripping removed everything,
/// so a captured date never repeats inside the title.
pub fn title_spans(clause: &str, found: &[Candidate]) -> Vec<String> {
    leadin::options(&base_title(clause, found))
}

/// The clause without its date, time and duration phrases and its filler words: the
/// title before the lead-in question ([`leadin`]) is asked.
pub fn base_title(clause: &str, found: &[Candidate]) -> String {
    let mut stripped = String::new();
    let mut cursor = 0;
    for candidate in found {
        stripped.push_str(&clause[cursor..candidate.span.start]);
        cursor = candidate.span.end;
    }
    stripped.push_str(&clause[cursor..]);

    let stripped = clean_title(&stripped);
    if !stripped.is_empty() {
        return stripped;
    }
    let whole = clean_title(clause);
    if !whole.is_empty() {
        return whole;
    }
    clause.trim().to_owned()
}

fn is_edge_noise(c: char) -> bool {
    c.is_whitespace() || c.is_ascii_punctuation() || matches!(c, '‘' | '’' | '“' | '”')
}

/// Drop filler and edge punctuation, collapse spaces, capitalise the first letter.
///
/// Removing the date, time and duration phrases leaves the words that introduced
/// them stranded at the end ("...by the", "...before the exam on"), so trailing
/// function words and hedges are dropped too.
pub fn clean_title(text: &str) -> String {
    const FILLER: [&str; 16] = [
        "i need to ",
        "i have to ",
        "i should ",
        "i also have to ",
        "remind me to ",
        "don't forget to ",
        "also ",
        "um ",
        "uh ",
        "um, ",
        "uh, ",
        "okay, ",
        "okay ",
        "so, ",
        "so ",
        "right, ",
    ];
    /// Words that only make sense in front of the phrase they introduce.
    const TRAILING: [&str; 16] = [
        "a", "an", "the", "on", "by", "at", "for", "to", "of", "in", "with", "before", "after",
        "from", "or", "due",
    ];
    const TRAILING_HEDGES: [&str; 4] = ["maybe", "probably", "just", "only"];

    let mut words = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(is_edge_noise)
        .to_owned();
    loop {
        let lower = words.to_ascii_lowercase();
        match FILLER.iter().find(|filler| lower.starts_with(*filler)) {
            Some(filler) => words = words[filler.len()..].to_owned(),
            None => break,
        }
    }
    let mut words = words.trim_matches(is_edge_noise).to_owned();
    loop {
        let trimmed = words.trim_end_matches(is_edge_noise);
        let last = trimmed.rsplit(' ').next().unwrap_or_default();
        let lower = last.to_ascii_lowercase();
        if lower.is_empty() {
            break;
        }
        if TRAILING_HEDGES.contains(&lower.as_str()) {
            words = trimmed[..trimmed.len() - last.len()].trim_end().to_owned();
            continue;
        }
        if TRAILING.contains(&lower.as_str()) {
            words = trimmed[..trimmed.len() - last.len()].trim_end().to_owned();
            continue;
        }
        break;
    }
    let words = words.trim_matches(|c: char| {
        c.is_whitespace()
            || c == ','
            || c == '.'
            || c == '!'
            || c == '?'
            || c == ';'
            || c == ':'
            || c == '-'
            || c == '_'
            || c == '('
            || c == ')'
            || c == '"'
            || c == '\''
            || c == '‘'
            || c == '’'
    });
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use dates::candidates;
    use todo_core::LocalContext;

    fn ctx() -> LocalContext {
        LocalContext {
            now: Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
            tz: chrono_tz::America::New_York,
        }
    }

    #[test]
    fn clean_title_drops_filler_and_capitalises() {
        assert_eq!(
            clean_title("Okay, I need to get my life together"),
            "Get my life together"
        );
        assert_eq!(clean_title("  um, buy milk.  "), "Buy milk");
    }

    #[test]
    fn clean_title_drops_the_words_left_stranded_by_a_date() {
        // Stripping "Friday" out of "upload it by Friday" must not leave "by".
        assert_eq!(clean_title("upload it by"), "Upload it");
        assert_eq!(clean_title("buy groceries by the"), "Buy groceries");
        // Only the words left dangling at the end go; "before the exam" still reads.
        assert_eq!(
            clean_title("make flashcards before the exam on"),
            "Make flashcards before the exam"
        );
        // A hedge that trailed a hedged duration goes too.
        assert_eq!(
            clean_title("review chapters six and seven maybe"),
            "Review chapters six and seven"
        );
        // "Due" only stands in front of the date it introduced.
        assert_eq!(clean_title("lab report due"), "Lab report");
        assert_eq!(clean_title("taxes due by"), "Taxes");
    }

    #[test]
    fn clean_title_keeps_the_words_that_carry_the_meaning() {
        assert_eq!(
            clean_title("email professor chen about"),
            "Email professor chen about"
        );
        // A title may legitimately end in one of these when nothing was stripped.
        assert_eq!(clean_title("finish the paper"), "Finish the paper");
    }

    #[test]
    fn the_whole_clause_is_not_a_title_when_stripping_leaves_words() {
        // The date is captured separately, so it must not repeat in the title:
        // Jev gets only the stripped span to choose from.
        let spans = title_spans(
            "Call mom tomorrow at 3",
            &candidates("Call mom tomorrow at 3", &ctx()),
        );
        assert_eq!(spans, vec!["Call mom".to_string()]);

        let spans = title_spans(
            "Lab report due Wednesday",
            &candidates("Lab report due Wednesday", &ctx()),
        );
        assert_eq!(spans, vec!["Lab report".to_string()]);
    }

    #[test]
    fn a_clause_without_a_date_offers_only_itself() {
        let spans = title_spans("Buy milk", &candidates("Buy milk", &ctx()));
        assert_eq!(spans, vec!["Buy milk".to_string()]);
    }

    #[test]
    fn a_clause_that_is_all_date_falls_back_to_the_whole_clause() {
        // Nothing survives stripping, so the clause itself is the only span.
        let spans = title_spans("tomorrow at 3", &candidates("tomorrow at 3", &ctx()));
        assert_eq!(spans, vec!["Tomorrow at 3".to_string()]);
    }
}
