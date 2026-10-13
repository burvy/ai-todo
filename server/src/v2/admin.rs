//! The local administration console: commands typed into the server's own terminal.
//!
//! It reads stdin, so only someone who can run the server can use it. It opens its own
//! handle on the database, so an answer reflects what the server is doing right now.

use std::io::BufRead;
use std::path::PathBuf;

use todo_core::{Clock, SystemClock};

use super::auth::Plan;
use super::store::{Block, SqliteStore, Store, UsageOrder, UsageRow};

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Usage,
    Blocked,
    Block(Target),
    Unblock(Target),
    /// A user, by id or email, and the plan to move them to.
    Plan(String, Plan),
    /// Ranks 1-based, inclusive: `top 5 10` shows ranks five to ten.
    Top {
        from: usize,
        to: usize,
        order: UsageOrder,
    },
    /// Stop an account using the app. The same thing as a user block, named for what
    /// the operator means by it.
    Pause(String),
    /// Let a paused account back in.
    Unpause(String),
}

/// What a block or unblock applies to. A user may be named by id or by email, which is
/// resolved when the command runs.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    User(String),
    Ip(String),
}

pub fn parse_command(input: &str) -> Result<Command, &'static str> {
    const HELP: &str = "expected usage, blocked, top [N [M]] [today], block/unblock user <email|id>, \
block/unblock ip <addr>, pause/unpause <email|id>, or plan <email|id> <anon|free|paid>";
    const BAD_RANK: &str = "top takes ranks: a number from 1 upwards, e.g. `top 5` or `top 5 10`";
    let parts: Vec<_> = input.split_whitespace().collect();
    let command = match parts.as_slice() {
        ["usage"] => Command::Usage,
        ["blocked"] => Command::Blocked,
        ["block", "user", who] => Command::Block(Target::User((*who).into())),
        ["block", "ip", ip] => Command::Block(Target::Ip((*ip).into())),
        ["unblock", "user", who] => Command::Unblock(Target::User((*who).into())),
        ["unblock", "ip", ip] => Command::Unblock(Target::Ip((*ip).into())),
        ["plan", who, plan] => Command::Plan((*who).into(), parse_plan(plan).ok_or(HELP)?),
        ["pause", who] => Command::Pause((*who).into()),
        ["unpause", who] => Command::Unpause((*who).into()),
        ["top", rest @ ..] => {
            // `today` is an optional suffix, so it is taken off the end first and
            // what is left is the ranks.
            let (rest, order) = match rest.split_last() {
                Some((&"today", ranks)) => (ranks, UsageOrder::Day(today())),
                _ => (rest, UsageOrder::AllTime),
            };
            let mut numbers: Vec<usize> = rest
                .iter()
                .map(|rank| rank.parse::<usize>().map_err(|_| BAD_RANK))
                .collect::<Result<_, &str>>()?;
            if numbers.is_empty() {
                numbers.push(TOP);
            }
            if numbers.contains(&0) {
                return Err(BAD_RANK);
            }
            // `top 10 5` is the same request as `top 5 10`, not a mistake to refuse.
            Command::Top {
                from: numbers.iter().min().copied().unwrap_or(TOP),
                to: numbers.iter().max().copied().unwrap_or(TOP),
                order,
            }
        }
        _ => return Err(HELP),
    };
    Ok(command)
}

pub fn parse_plan(value: &str) -> Option<Plan> {
    match value {
        "anon" => Some(Plan::Anon),
        "free" => Some(Plan::Free),
        "paid" => Some(Plan::Paid),
        _ => None,
    }
}

/// Carry out one command and return what to print. An error is printed too, since the
/// operator needs to see why nothing happened.
pub fn run(store: &dyn Store, command: Command) -> Result<String, String> {
    match command {
        Command::Usage => {
            let usage = store
                .usage_today(SystemClock.now().date_naive(), TOP)
                .map_err(|error| error.to_string())?;
            let mut lines = vec!["users today:".to_owned()];
            lines.extend(usage.users.iter().map(|(id, n)| format!("  {n:>5}  {id}")));
            lines.push("addresses today:".to_owned());
            lines.extend(usage.ips.iter().map(|(ip, n)| format!("  {n:>5}  {ip}")));
            Ok(lines.join("\n"))
        }
        Command::Blocked => {
            let blocks = store.blocks().map_err(|error| error.to_string())?;
            if blocks.is_empty() {
                return Ok("nothing is blocked".into());
            }
            Ok(blocks
                .iter()
                .map(|block| match block {
                    Block::User(id) => format!("user {id}"),
                    Block::Ip(ip) => format!("ip {ip}"),
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        Command::Block(target) => {
            let block = resolve(store, target)?;
            store.block(&block).map_err(|error| error.to_string())?;
            Ok(format!("blocked {}", describe(&block)))
        }
        Command::Unblock(target) => {
            let block = resolve(store, target)?;
            let lifted = store.unblock(&block).map_err(|error| error.to_string())?;
            Ok(if lifted {
                format!("unblocked {}", describe(&block))
            } else {
                format!("{} was not blocked", describe(&block))
            })
        }
        Command::Plan(who, plan) => {
            let id = user_id(store, &who)?;
            store
                .set_user_plan(&id, plan)
                .map_err(|error| error.to_string())?;
            Ok(format!("{id} is now on the {} plan", plan.as_str()))
        }
        Command::Top { from, to, order } => top_rows(store, from, to, order),
        Command::Pause(who) => {
            let id = known_user(store, &who)?;
            store
                .block(&Block::User(id.clone()))
                .map_err(|error| error.to_string())?;
            Ok(format!("paused {id}"))
        }
        Command::Unpause(who) => {
            let id = known_user(store, &who)?;
            let lifted = store
                .unblock(&Block::User(id.clone()))
                .map_err(|error| error.to_string())?;
            Ok(if lifted {
                format!("unpaused {id}")
            } else {
                format!("{id} was not paused")
            })
        }
    }
}

/// The `top` answer: one line per account in the requested ranks.
///
/// The ranks asked for are read from the top of the ranking, so a range that runs past
/// the last account prints what there is and says how many there were.
fn top_rows(
    store: &dyn Store,
    from: usize,
    to: usize,
    order: UsageOrder,
) -> Result<String, String> {
    let ranked = store
        .usage_ranking(order, to)
        .map_err(|error| error.to_string())?;
    let paused: Vec<String> = store
        .blocks()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|block| match block {
            Block::User(id) => Some(id),
            Block::Ip(_) => None,
        })
        .collect();

    if ranked.is_empty() {
        return Ok("no accounts have made a Jev call yet".into());
    }
    let mut lines: Vec<String> = vec![format!(
        "{:<4} {:>8}  {:<28} {:<20} {:<6} {}",
        "rank", "jev", "email", "user", "plan", ""
    )];
    // The slice keeps ranks: an account at position one is rank one.
    lines.extend(
        ranked
            .iter()
            .enumerate()
            .skip(from.saturating_sub(1))
            .map(|(position, row)| usage_line(position + 1, row, paused.contains(&row.user))),
    );
    if ranked.len() < to {
        lines.push(format!(
            "(only {} user{})",
            ranked.len(),
            if ranked.len() == 1 { "" } else { "s" }
        ));
    }
    Ok(lines.join("\n"))
}

fn usage_line(rank: usize, row: &UsageRow, paused: bool) -> String {
    let email = if row.email.is_empty() {
        "anonymous"
    } else {
        row.email.as_str()
    };
    format!(
        "{:<4} {:>8}  {:<28} {:<20} {:<6} {}",
        rank,
        row.calls,
        email,
        row.user,
        row.plan.as_str(),
        if paused { "PAUSED" } else { "" }
    )
    .trim_end()
    .to_owned()
}

/// An account named by id or email, with an error when there is no such account.
///
/// [`user_id`] resolves an email but takes an id on faith, which is right for blocking
/// but not for pausing: a pause on an id nobody holds would sit there looking real.
fn known_user(store: &dyn Store, who: &str) -> Result<String, String> {
    let id = user_id(store, who)?;
    if store
        .account_exists(&id)
        .map_err(|error| error.to_string())?
    {
        return Ok(id);
    }
    Err(format!("no account has the id {id}"))
}

/// The local day a `today` ranking is about.
fn today() -> chrono::NaiveDate {
    SystemClock.now().date_naive()
}

/// Rows shown by `usage`.
pub const TOP: usize = 10;

pub fn resolve(store: &dyn Store, target: Target) -> Result<Block, String> {
    match target {
        Target::User(who) => Ok(Block::User(user_id(store, &who)?)),
        Target::Ip(ip) => Ok(Block::Ip(ip)),
    }
}

/// An email if the value has an `@`, else the id as given.
pub fn user_id(store: &dyn Store, who: &str) -> Result<String, String> {
    if !who.contains('@') {
        return Ok(who.to_owned());
    }
    store
        .user_id_for_email(&who.to_ascii_lowercase())
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("no signed-in user has the email {who}"))
}

pub fn describe(block: &Block) -> String {
    match block {
        Block::User(id) => format!("user {id}"),
        Block::Ip(ip) => format!("ip {ip}"),
    }
}

/// Read commands from stdin on a thread of their own. Started from `main` only, never
/// from code that tests call.
pub fn start_console(database: PathBuf) {
    std::thread::spawn(move || {
        let store = match SqliteStore::open(&database) {
            Ok(store) => store,
            Err(error) => {
                eprintln!("admin console is off: {error}");
                return;
            }
        };
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            match parse_command(&line) {
                Ok(command) => match run(&store, command) {
                    Ok(text) => println!("{text}"),
                    Err(text) => eprintln!("{text}"),
                },
                Err(help) => eprintln!("{help}"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::super::auth::Me;
    use super::super::store::{MemoryStore, SqliteStore};
    use super::*;

    fn signed_in(store: &MemoryStore, id: &str, email: &str) {
        store
            .upsert_user_and_session(
                &Me {
                    id: id.into(),
                    email: email.into(),
                    name: "Sam".into(),
                    picture: None,
                    plan: Plan::Free,
                },
                "token",
                SystemClock.now() + chrono::Duration::days(1),
            )
            .unwrap();
    }

    fn signed_in_sqlite(store: &SqliteStore, id: &str, email: &str) {
        store
            .upsert_user_and_session(
                &Me {
                    id: id.into(),
                    email: email.into(),
                    name: "Sam".into(),
                    picture: None,
                    plan: Plan::Free,
                },
                &format!("token-{}", id),
                SystemClock.now() + chrono::Duration::days(1),
            )
            .unwrap();
    }

    #[test]
    fn parses_all_commands() {
        assert_eq!(parse_command("usage"), Ok(Command::Usage));
        assert_eq!(parse_command("blocked"), Ok(Command::Blocked));
        assert_eq!(
            parse_command("block user a@b.test"),
            Ok(Command::Block(Target::User("a@b.test".into())))
        );
        assert_eq!(
            parse_command("block ip 127.0.0.1"),
            Ok(Command::Block(Target::Ip("127.0.0.1".into())))
        );
        assert_eq!(
            parse_command("unblock user google-1"),
            Ok(Command::Unblock(Target::User("google-1".into())))
        );
        assert_eq!(
            parse_command("unblock ip 127.0.0.1"),
            Ok(Command::Unblock(Target::Ip("127.0.0.1".into())))
        );
        assert_eq!(
            parse_command("plan a@b.test paid"),
            Ok(Command::Plan("a@b.test".into(), Plan::Paid))
        );
    }

    #[test]
    fn rejects_unknown_or_incomplete_commands() {
        for input in [
            "",
            "block",
            "block user",
            "block ip",
            "plan a@b.test gold",
            "plan a@b.test",
            "wat",
        ] {
            assert!(parse_command(input).is_err(), "{input:?} should not parse");
        }
    }

    #[test]
    fn block_and_unblock_by_email_act_on_the_user_id() {
        let store = MemoryStore::default();
        signed_in(&store, "google-1", "sam@example.com");

        run(
            &store,
            Command::Block(Target::User("Sam@Example.com".into())),
        )
        .expect("block by email");
        assert!(store.is_blocked("google-1", "203.0.113.1").unwrap());

        run(
            &store,
            Command::Unblock(Target::User("sam@example.com".into())),
        )
        .expect("unblock by email");
        assert!(!store.is_blocked("google-1", "203.0.113.1").unwrap());
    }

    #[test]
    fn an_unknown_email_is_reported_and_blocks_nothing() {
        let store = MemoryStore::default();
        let error = run(
            &store,
            Command::Block(Target::User("nobody@example.com".into())),
        )
        .expect_err("no such user");
        assert!(error.contains("nobody@example.com"), "got: {error}");
        assert!(store.blocks().unwrap().is_empty());
    }

    #[test]
    fn blocked_lists_what_is_blocked_and_unblocking_twice_says_so() {
        let store = MemoryStore::default();
        assert_eq!(run(&store, Command::Blocked).unwrap(), "nothing is blocked");

        run(&store, Command::Block(Target::Ip("203.0.113.9".into()))).unwrap();
        assert_eq!(run(&store, Command::Blocked).unwrap(), "ip 203.0.113.9");

        run(&store, Command::Unblock(Target::Ip("203.0.113.9".into()))).unwrap();
        assert_eq!(
            run(&store, Command::Unblock(Target::Ip("203.0.113.9".into()))).unwrap(),
            "ip 203.0.113.9 was not blocked"
        );
    }

    #[test]
    fn plan_changes_a_signed_in_users_plan_by_email() {
        let store = MemoryStore::default();
        signed_in(&store, "google-1", "sam@example.com");

        run(&store, Command::Plan("sam@example.com".into(), Plan::Paid)).expect("plan");
        assert_eq!(store.user_plan("google-1").unwrap(), Plan::Paid);
    }

    #[test]
    fn usage_counts_today_captures_by_user_and_address() {
        let store = MemoryStore::default();
        let today = SystemClock.now().date_naive();
        store
            .record_capture("google-1", "203.0.113.1", today)
            .unwrap();
        store
            .record_capture("google-1", "203.0.113.1", today)
            .unwrap();
        store
            .record_capture("google-2", "203.0.113.1", today)
            .unwrap();

        let text = run(&store, Command::Usage).unwrap();
        assert!(text.contains("2  google-1"), "got: {text}");
        assert!(text.contains("3  203.0.113.1"), "got: {text}");
    }

    // ---- top: the biggest Jev spenders ---------------------------------------

    /// Accounts that have spent Jev calls: one email, one anonymous, equal totals so
    /// the tie-break shows.
    fn store_with_spenders(store: &MemoryStore, calls: &[(&str, u32)]) {
        let today = SystemClock.now().date_naive();
        for (id, times) in calls {
            signed_in(store, id, &format!("{id}@example.com"));
            for _ in 0..*times {
                store.record_capture(id, "203.0.113.1", today).unwrap();
            }
        }
        store
            .ensure_anonymous("anon_visitor", SystemClock.now())
            .unwrap();
        store
            .record_capture("anon_visitor", "203.0.113.1", today)
            .unwrap();
    }

    #[test]
    fn parses_every_top_form() {
        let all = UsageOrder::AllTime;
        assert_eq!(
            parse_command("top"),
            Ok(Command::Top {
                from: TOP,
                to: TOP,
                order: all
            })
        );
        assert_eq!(
            parse_command("top 5"),
            Ok(Command::Top {
                from: 5,
                to: 5,
                order: all
            })
        );
        assert_eq!(
            parse_command("top 5 10"),
            Ok(Command::Top {
                from: 5,
                to: 10,
                order: all
            })
        );
    }

    /// `top 10 5` asks the same thing as `top 5 10`: ranks five to ten.
    #[test]
    fn a_reversed_top_range_is_the_same_request() {
        let today = UsageOrder::Day(SystemClock.now().date_naive());
        assert_eq!(
            parse_command("top 10 5"),
            Ok(Command::Top {
                from: 5,
                to: 10,
                order: UsageOrder::AllTime
            })
        );
        assert_eq!(
            parse_command("top 5 10 today"),
            Ok(Command::Top {
                from: 5,
                to: 10,
                order: today
            })
        );
        assert_eq!(
            parse_command("top 3 today"),
            Ok(Command::Top {
                from: 3,
                to: 3,
                order: today
            })
        );
    }

    #[test]
    fn a_top_rank_must_be_a_positive_number() {
        for input in ["top 0", "top 0 3", "top 3 0", "top x", "top 3 x", "top 3.5"] {
            let error = parse_command(input).expect_err(input);
            assert!(
                error.contains("number"),
                "{input:?} should say what a rank is"
            );
        }
        assert!(parse_command("top 1 2 tomorrow").is_err());
    }

    #[test]
    fn parses_pause_and_unpause() {
        assert_eq!(
            parse_command("pause sam@example.com"),
            Ok(Command::Pause("sam@example.com".into()))
        );
        assert_eq!(
            parse_command("unpause google-1"),
            Ok(Command::Unpause("google-1".into()))
        );
        assert!(parse_command("pause").is_err());
    }

    /// Most spend first, one line each: rank, calls, email, id, plan.
    #[test]
    fn top_ranks_the_biggest_spenders_first() {
        let store = MemoryStore::default();
        store_with_spenders(&store, &[("google-1", 5), ("google-2", 9), ("google-3", 5)]);

        let text = run(
            &store,
            Command::Top {
                from: 1,
                to: 3,
                order: UsageOrder::AllTime,
            },
        )
        .unwrap();
        let rows: Vec<&str> = text.lines().skip(1).collect();
        assert_eq!(rows.len(), 3, "got: {text}");
        assert!(
            rows[0].contains("google-2@example.com") && rows[0].contains(" 9 "),
            "the biggest spender is first: {rows:?}"
        );
        // Equal totals break by id, so the order never depends on the store.
        assert!(rows[1].contains("google-1"), "{rows:?}");
        assert!(rows[2].contains("google-3"), "{rows:?}");
        assert!(
            rows[0].starts_with("1 "),
            "the first line carries rank one: {rows:?}"
        );
    }

    /// A range reads from the top of the ranking, so rank two is the second account.
    #[test]
    fn top_takes_a_range_of_ranks() {
        let store = MemoryStore::default();
        store_with_spenders(&store, &[("a", 5), ("b", 4), ("c", 3), ("d", 2)]);

        let text = run(
            &store,
            Command::Top {
                from: 2,
                to: 3,
                order: UsageOrder::AllTime,
            },
        )
        .unwrap();
        let rows: Vec<&str> = text.lines().skip(1).collect();
        assert_eq!(rows.len(), 2, "got: {text}");
        assert!(
            rows[0].starts_with("2 ") && rows[0].contains("b@"),
            "{rows:?}"
        );
        assert!(
            rows[1].starts_with("3 ") && rows[1].contains("c@"),
            "{rows:?}"
        );
        assert!(
            !text.contains('d'),
            "a rank outside the range is not shown: {text}"
        );
    }

    /// A range past the last account prints what there is, and says how much.
    #[test]
    fn a_top_range_past_the_end_says_how_many_there_are() {
        let store = MemoryStore::default();
        store_with_spenders(&store, &[("a", 5)]);

        let text = run(
            &store,
            Command::Top {
                from: 1,
                to: 10,
                order: UsageOrder::AllTime,
            },
        )
        .unwrap();
        assert!(text.contains("(only 2 users)"), "got: {text}");
        assert!(
            text.lines()
                .filter(|line| line.starts_with(['0', '1', '2']))
                .count()
                == 2,
            "the two accounts that exist are printed: {text}"
        );

        let empty = MemoryStore::default();
        let text = run(
            &empty,
            Command::Top {
                from: 1,
                to: 5,
                order: UsageOrder::AllTime,
            },
        )
        .unwrap();
        assert!(text.contains("no accounts"), "got: {text}");
    }

    /// `today` ranks by today's calls, which is not the all-time total.
    #[test]
    fn top_today_ranks_by_todays_calls() {
        let store = MemoryStore::default();
        let yesterday = SystemClock.now().date_naive() - chrono::Duration::days(1);
        signed_in(&store, "google-1", "one@example.com");
        signed_in(&store, "google-2", "two@example.com");
        // google-1 spent a lot yesterday and nothing today; google-2 the reverse.
        for _ in 0..7 {
            store
                .record_capture("google-1", "203.0.113.1", yesterday)
                .unwrap();
        }
        for _ in 0..2 {
            store
                .record_capture("google-2", "203.0.113.1", yesterday)
                .unwrap();
        }
        store
            .record_capture("google-2", "203.0.113.1", SystemClock.now().date_naive())
            .unwrap();

        let all = run(
            &store,
            Command::Top {
                from: 1,
                to: 1,
                order: UsageOrder::AllTime,
            },
        )
        .unwrap();
        assert!(all.contains("one@example.com"), "all time: {all}");
        let today = run(
            &store,
            Command::Top {
                from: 1,
                to: 1,
                order: UsageOrder::Day(SystemClock.now().date_naive()),
            },
        )
        .unwrap();
        assert!(today.contains("two@example.com"), "today: {today}");
    }

    /// A paused account says so on its row.
    #[test]
    fn a_top_row_marks_a_paused_account() {
        let store = MemoryStore::default();
        store_with_spenders(&store, &[("google-1", 5)]);
        run(&store, Command::Pause("google-1".into())).unwrap();

        let text = run(
            &store,
            Command::Top {
                from: 1,
                to: 5,
                order: UsageOrder::AllTime,
            },
        )
        .unwrap();
        assert!(
            text.lines()
                .any(|line| line.contains("google-1") && line.ends_with("PAUSED")),
            "got: {text}"
        );
    }

    /// Pause and unpause are one mechanism with `block user`, and say what happened.
    #[test]
    fn pause_and_unpause_round_trip_on_memory_store() {
        let store = MemoryStore::new();
        signed_in(&store, "google-1", "sam@example.com");
        signed_in(&store, "google-2", "kim@example.com");

        assert_eq!(
            run(&store, Command::Pause("sam@example.com".into())).unwrap(),
            "paused google-1"
        );
        assert!(store.is_user_blocked("google-1").unwrap());
        assert!(!store.is_user_blocked("google-2").unwrap(), "one only");
        // Unblock lifts the same block pause set, so it works.
        assert_eq!(
            run(
                &store,
                Command::Unblock(Target::User("sam@example.com".into()))
            )
            .unwrap(),
            "unblocked user google-1"
        );
        assert!(!store.is_user_blocked("google-1").unwrap());

        assert_eq!(
            run(&store, Command::Unpause("kim@example.com".into())).unwrap(),
            "google-2 was not paused"
        );
    }

    /// Pause and unpause work on SqliteStore too.
    #[test]
    fn pause_and_unpause_round_trip_on_sqlite_store() {
        let store = SqliteStore::open_in_memory().unwrap();
        signed_in_sqlite(&store, "google-1", "sam@example.com");
        signed_in_sqlite(&store, "google-2", "kim@example.com");

        assert_eq!(
            run(&store, Command::Pause("sam@example.com".into())).unwrap(),
            "paused google-1"
        );
        assert!(store.is_user_blocked("google-1").unwrap());
        assert!(!store.is_user_blocked("google-2").unwrap(), "one only");
        assert_eq!(
            run(&store, Command::Unpause("sam@example.com".into())).unwrap(),
            "unpaused google-1"
        );
        assert!(!store.is_user_blocked("google-1").unwrap());

        assert_eq!(
            run(&store, Command::Unpause("kim@example.com".into())).unwrap(),
            "google-2 was not paused"
        );
    }

    /// Pausing an account nobody holds says so and blocks nothing.
    #[test]
    fn pausing_an_account_that_does_not_exist_says_so() {
        let store = MemoryStore::new();
        signed_in(&store, "google-1", "sam@example.com");

        let missing_email =
            run(&store, Command::Pause("nobody@example.com".into())).expect_err("no such account");
        assert!(
            missing_email.contains("nobody@example.com"),
            "got: {missing_email}"
        );
        let missing_id = run(&store, Command::Pause("google-99".into())).expect_err("no such id");
        assert!(missing_id.contains("google-99"), "got: {missing_id}");
        assert!(store.blocks().unwrap().is_empty(), "nothing was paused");
    }
}
