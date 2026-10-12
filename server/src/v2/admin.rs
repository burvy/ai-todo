//! The local administration console: commands typed into the server's own terminal.
//!
//! It reads stdin, so only someone who can run the server can use it. It opens its own
//! handle on the database, so an answer reflects what the server is doing right now.

use std::io::BufRead;
use std::path::PathBuf;

use todo_core::{Clock, SystemClock};

use super::auth::Plan;
use super::store::{Block, SqliteStore, Store};

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Usage,
    Blocked,
    Block(Target),
    Unblock(Target),
    /// A user, by id or email, and the plan to move them to.
    Plan(String, Plan),
}

/// What a block or unblock applies to. A user may be named by id or by email, which is
/// resolved when the command runs.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    User(String),
    Ip(String),
}

pub fn parse_command(input: &str) -> Result<Command, &'static str> {
    const HELP: &str = "expected usage, blocked, block/unblock user <email|id>, block/unblock ip <addr>, or plan <email|id> <anon|free|paid>";
    let parts: Vec<_> = input.split_whitespace().collect();
    let command = match parts.as_slice() {
        ["usage"] => Command::Usage,
        ["blocked"] => Command::Blocked,
        ["block", "user", who] => Command::Block(Target::User((*who).into())),
        ["block", "ip", ip] => Command::Block(Target::Ip((*ip).into())),
        ["unblock", "user", who] => Command::Unblock(Target::User((*who).into())),
        ["unblock", "ip", ip] => Command::Unblock(Target::Ip((*ip).into())),
        ["plan", who, plan] => Command::Plan((*who).into(), parse_plan(plan).ok_or(HELP)?),
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
    }
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
    use super::super::store::MemoryStore;
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
}
