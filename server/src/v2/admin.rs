//! Commands for the local administration console.
#![allow(dead_code)]

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Usage,
    BlockUser(String),
    BlockIp(String),
    UnblockUser(String),
    UnblockIp(String),
    Blocked,
    Plan(String, super::auth::Plan),
}

pub fn parse_command(input: &str) -> Result<Command, &'static str> {
    let parts: Vec<_> = input.split_whitespace().collect();
    match parts.as_slice() {
        ["usage"] => Ok(Command::Usage),
        ["blocked"] => Ok(Command::Blocked),
        ["plan", value, "anon"] => Ok(Command::Plan((*value).into(), super::auth::Plan::Anon)),
        ["plan", value, "free"] => Ok(Command::Plan((*value).into(), super::auth::Plan::Free)),
        ["plan", value, "paid"] => Ok(Command::Plan((*value).into(), super::auth::Plan::Paid)),
        ["block", "user", value] if !value.is_empty() => Ok(Command::BlockUser((*value).into())),
        ["block", "ip", value] if !value.is_empty() => Ok(Command::BlockIp((*value).into())),
        ["unblock", "user", value] if !value.is_empty() => {
            Ok(Command::UnblockUser((*value).into()))
        }

        ["unblock", "ip", value] if !value.is_empty() => Ok(Command::UnblockIp((*value).into())),
        _ => Err("expected usage, blocked, block/unblock user <value>, or block/unblock ip <addr>"),
    }
}

pub fn start_console() {
    std::thread::spawn(|| {
        let stdin = std::io::stdin();
        for line in stdin.lines().map_while(Result::ok) {
            if let Ok(command) = parse_command(&line) {
                eprintln!("admin command received: {command:?}");
            } else {
                eprintln!("invalid admin command");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_commands() {
        assert_eq!(parse_command("usage"), Ok(Command::Usage));
        assert_eq!(
            parse_command("block user a@b.test"),
            Ok(Command::BlockUser("a@b.test".into()))
        );
        assert_eq!(
            parse_command("block ip 127.0.0.1"),
            Ok(Command::BlockIp("127.0.0.1".into()))
        );
        assert_eq!(
            parse_command("unblock user a@b.test"),
            Ok(Command::UnblockUser("a@b.test".into()))
        );
        assert_eq!(
            parse_command("unblock ip 127.0.0.1"),
            Ok(Command::UnblockIp("127.0.0.1".into()))
        );
        assert_eq!(parse_command("blocked"), Ok(Command::Blocked));
    }

    #[test]
    fn rejects_unknown_commands() {
        assert!(parse_command("block").is_err());
        assert!(parse_command("block user").is_err());
        assert!(parse_command("wat").is_err());
    }
}
