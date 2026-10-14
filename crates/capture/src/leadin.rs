//! Lead-ins on a task title: "Add a task to buy milk" is the task "Buy milk".
//!
//! Two ways to find where the real title starts, compared in
//! `docs/experiments/title-leadin.md`: a strict grammar ([`strip`]) and a list of
//! candidate start words for Jev to choose from ([`options`]).

/// Words that open a request to make a task ("add", "schedule", ...).
const MAKE_VERBS: [&str; 9] = [
    "add", "create", "make", "schedule", "set", "put", "log", "jot", "write",
];
/// What the request makes.
const TASK_NOUNS: [&str; 6] = ["task", "todo", "to-do", "reminder", "note", "item"];
/// Words that join the lead-in to the title: "a task *to* buy milk".
const LINKS: [&str; 9] = [
    "to", "that", "called", "named", "titled", "about", "for", "saying", "says",
];
/// Words that start a noun phrase, not an action: "a note *to the* landlord" is a note
/// addressed to someone, so "to" there is not a link.
const NOT_AN_ACTION: [&str; 14] = [
    "the", "a", "an", "my", "your", "his", "her", "their", "our", "this", "that", "me", "self",
    "us",
];
/// Words before a lead-in that mark the clause as a request to the app.
const CUES: [&str; 12] = [
    "remind", "note", "hey", "please", "can", "could", "would", "want", "need", "should", "let's",
    "new",
];
/// Ways to ask for it: "please add a task...", "can you add a task...".
const OPENERS: [&[&str]; 11] = [
    &["please"],
    &["can", "you"],
    &["could", "you"],
    &["would", "you"],
    &["i", "want", "you", "to"],
    &["i", "want", "to"],
    &["i'd", "like", "to"],
    &["i", "would", "like", "to"],
    &["i", "need", "to"],
    &["let's"],
    &["lets"],
];
/// What speech to text writes when it mishears "add a task": `(heard words, sure)`. A sure
/// one is unmistakable and cuts the lead-in even with no link word after it ("attittask
/// call mom"); the others are ordinary words too ("at a task force meeting"), so they
/// only count when a link word or a colon follows.
const HEARD_AS_ADD_A_TASK: [(&[&str], bool); 8] = [
    (&["attittask"], true),
    (&["attitask"], true),
    (&["attit", "task"], true),
    (&["add", "it", "task"], true),
    (&["add", "a", "task"], false),
    (&["at", "a", "task"], false),
    (&["at", "it", "task"], false),
    (&["ad", "a", "task"], false),
];

/// The words that open a request ("please", "can you", ...), as a count of words.
fn opener_len(words: &[&str]) -> usize {
    let is =
        |index: usize, word: &str| words.get(index).is_some_and(|heard| plain(heard).0 == word);
    let mut i = 0;
    while let Some(opener) = OPENERS
        .iter()
        .find(|opener| opener.iter().enumerate().all(|(n, word)| is(i + n, word)))
    {
        i += opener.len();
    }
    i
}

/// `title` with a mishearing of "add a task" at its start (or right after a "please" or
/// "can you") written as "add a task", and whether it was an unmistakable one.
fn heard_right(title: &str) -> (String, bool) {
    let words: Vec<&str> = title.split_whitespace().collect();
    let at = opener_len(&words);
    let heard = HEARD_AS_ADD_A_TASK.iter().find(|(heard, _)| {
        heard.len() <= words.len() - at
            && heard
                .iter()
                .enumerate()
                .all(|(n, word)| plain(words[at + n]).0 == *word)
    });
    let Some(&(heard, sure)) = heard else {
        return (title.to_owned(), false);
    };
    // "at a task: call mom": the colon stays on the noun, where the grammar looks for it.
    let noun = if words[at + heard.len() - 1].ends_with(':') {
        "task:"
    } else {
        "task"
    };
    let mut fixed: Vec<&str> = words[..at].to_vec();
    fixed.extend(["add", "a", noun]);
    fixed.extend(&words[at + heard.len()..]);
    (fixed.join(" "), sure)
}

/// Whether `word` joins a lead-in to the title, given the word after it. "to" before an
/// article or pronoun is an addressee ("a note to the landlord", "a note to self"), and
/// "for me" is a favour, not a title.
fn is_link(word: &str, next: Option<&str>) -> bool {
    LINKS.contains(&word)
        && !(word == "to" && next.is_some_and(|next| NOT_AN_ACTION.contains(&next)))
        && !(word == "for" && next.is_some_and(|next| ["me", "us"].contains(&next)))
}

/// How far into the clause a title may start.
const MAX_LEAD_IN_WORDS: usize = 10;

/// A word without edge punctuation, lowercased, and whether a colon ended it.
fn plain(word: &str) -> (String, bool) {
    let colon = word.ends_with(':');
    let text = word
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'')
        .to_lowercase();
    (text, colon)
}

fn capitalised(words: &[&str]) -> String {
    let text = words.join(" ");
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The title with its lead-in cut off, when the start matches the strict grammar:
/// `[please|can you|I want you to|...] [add|create|make|schedule|set up|put|log|jot down|write down] [a|an|the|another]
/// [new] task|todo|to-do|reminder|note|item [on my list] to|that|called|...`, or a bare
/// `task:` / `new task:` / `to do:` / `add to my list:` label. `None` leaves the title as is.
pub fn strip(title: &str) -> Option<String> {
    let (heard, sure) = heard_right(title);
    let title = heard.as_str();
    let words: Vec<&str> = title.split_whitespace().collect();
    let at = |index: usize| words.get(index).map(|word| plain(word));
    let is = |index: usize, set: &[&str]| {
        at(index).is_some_and(|(word, _)| set.contains(&word.as_str()))
    };
    let mut i = 0;
    while let Some(opener) = OPENERS.iter().find(|opener| {
        opener
            .iter()
            .enumerate()
            .all(|(n, word)| is(i + n, &[word]))
    }) {
        i += opener.len();
    }
    // "remind me to call mom" is the whole lead-in.
    if is(i, &["remind"]) && is(i + 1, &["me"]) && is(i + 2, &["to"]) {
        let rest = capitalised(&words[i + 3..]);
        return (!rest.is_empty()).then_some(rest);
    }
    let verb = is(i, &MAKE_VERBS);
    if verb {
        i += 1;
        // "set up", "jot down", "write down"
        if is(i, &["up", "down"]) {
            i += 1;
        }
    }
    let article = is(i, &["a", "an", "the", "another"]);
    if article {
        i += 1;
    }
    if is(i, &["new"]) {
        i += 1;
    }
    // "to do:" is two words.
    let noun_len = if is(i, &["to"]) && is(i + 1, &["do"]) {
        2
    } else {
        1
    };
    if !(is(i, &TASK_NOUNS) || noun_len == 2) {
        // "add to my list: ..."
        return (verb
            && is(i, &["to"])
            && is(i + 1, &["my"])
            && is(i + 2, &["list"])
            && at(i + 2).is_some_and(|(_, colon)| colon))
        .then(|| capitalised(&words[i + 3..]))
        .filter(|rest| !rest.is_empty());
    }
    let colon = at(i + noun_len - 1).is_some_and(|(_, colon)| colon);
    i += noun_len;
    // "a task on my list to ..."
    if is(i, &["on", "in", "to"]) && is(i + 1, &["my"]) && is(i + 2, &["list", "todo", "to-do"]) {
        i += 3;
    }
    let linked =
        at(i).is_some_and(|(link, _)| is_link(&link, at(i + 1).map(|(next, _)| next).as_deref()));
    if linked {
        i += 1;
        // "a task that says call Dana"
        if is(i, &["says", "saying"]) {
            i += 1;
        }
    }
    // Without a link word the lead-in must be unmistakable: a colon label. A task noun
    // is also an ordinary word ("Write note cards for the talk", "Make task force
    // plans"), so "add task buy cards" is left whole here and only offered to Jev.
    if !(linked || colon || sure) {
        return None;
    }
    let rest = capitalised(&words[i..]);
    (!rest.is_empty()).then_some(rest)
}

/// The titles a clause might have, for Jev to choose from. The grammar's answer comes
/// first when it has one, so a judge with no opinion (the offline one) takes it; then the
/// whole clause; then one more for each word that follows a link word ("to", "called",
/// ...) or a colon, among the first few words, when a request cue comes before it.
/// "Add milk to the list" offers "The list" and Jev says no.
pub fn options(title: &str) -> Vec<String> {
    let words: Vec<&str> = title.split_whitespace().collect();
    let mut options: Vec<String> = strip(title).into_iter().collect();
    if !options.iter().any(|option| option == title) {
        options.push(title.to_owned());
    }
    for start in 1..words.len().min(MAX_LEAD_IN_WORDS + 1) {
        let (before, colon) = plain(words[start - 1]);
        let next = plain(words[start]).0;
        // A link word or colon, or a request verb and a bare task noun ("add task buy
        // cards"): offered to Jev, never cut by the grammar.
        let verb_and_noun = TASK_NOUNS.contains(&before.as_str())
            && !LINKS.contains(&next.as_str())
            && start >= 2
            && MAKE_VERBS.contains(&plain(words[start - 2]).0.as_str());
        let after_link = is_link(&before, Some(&next)) || colon || verb_and_noun;
        let requests = words[..start].iter().any(|word| {
            let (word, _) = plain(word);
            [&MAKE_VERBS[..], &TASK_NOUNS[..], &CUES[..]]
                .iter()
                .any(|set| set.contains(&word.as_str()))
        });
        if after_link && requests {
            let option = capitalised(&words[start..]);
            if !options.contains(&option) {
                options.push(option);
            }
        }
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_lead_in_is_cut() {
        assert_eq!(strip("Add a task to buy milk").as_deref(), Some("Buy milk"));
        assert_eq!(
            strip("Create a new task called finish the lab report").as_deref(),
            Some("Finish the lab report")
        );
        assert_eq!(
            strip("New task: finish the paper").as_deref(),
            Some("Finish the paper")
        );
        assert_eq!(
            strip("Put a task on my list to clean the fridge").as_deref(),
            Some("Clean the fridge")
        );
    }

    #[test]
    fn a_mishearing_of_add_a_task_is_cut_like_the_real_thing() {
        for (heard, title) in [
            ("Attittask buy milk", "Buy milk"),
            ("attittask call mom", "Call mom"),
            ("attitask call mom", "Call mom"),
            ("Attit task email the TA", "Email the TA"),
            ("Add it task pay rent", "Pay rent"),
            ("add it task to pay rent", "Pay rent"),
            ("Please attittask water the plants", "Water the plants"),
            ("Can you attittask to book flights", "Book flights"),
            ("At a task to book flights", "Book flights"),
            ("at a task: renew the passport", "Renew the passport"),
            ("Ad a task to call the vet", "Call the vet"),
        ] {
            assert_eq!(strip(heard).as_deref(), Some(title), "{heard}");
        }
    }

    #[test]
    fn a_phrase_that_only_sounds_like_the_mishearing_is_kept() {
        for title in [
            "At a task force meeting we agreed",
            "Look at a task list with Priya",
            "Meet at a task board review",
            "Attit",
            "Attittask",
            "Add it to the list",
        ] {
            assert_eq!(strip(title), None, "{title}");
        }
    }

    #[test]
    fn a_task_that_only_starts_like_a_lead_in_is_kept() {
        for title in [
            "Add milk to the list",
            "Schedule a meeting with Sam",
            "Create a new task template for the team",
            "Make a to-do list for the move",
            "Task the interns with the cleanup",
            "Add a task",
            // A task noun is also an ordinary word.
            "Write note cards for the talk",
            "Put note cards in the bag",
            "Set reminder alarms for Friday",
            "Make todo list",
            "Make task force plans",
            "Add note to self to call Mom",
            "Add task buy a card",
        ] {
            assert_eq!(strip(title), None, "{title}");
        }
    }

    #[test]
    fn options_lead_with_the_grammar_then_offer_each_start_after_a_link_word() {
        assert_eq!(
            options("Add a task to buy milk"),
            ["Buy milk", "Add a task to buy milk"]
        );
        // "to the" is not a link, so "The list" is no longer even offered.
        assert_eq!(options("Add milk to the list"), ["Add milk to the list"]);
        assert_eq!(options("Buy milk"), ["Buy milk"]);
        // A request verb and a bare task noun: offered to Jev, not cut by the grammar.
        assert_eq!(
            options("Add task buy a card"),
            ["Add task buy a card", "Buy a card"]
        );
        // "to self" is an addressee, so it is not a start.
        assert_eq!(
            options("Add note to self to call Mom"),
            ["Add note to self to call Mom", "Call Mom"]
        );
    }
}
