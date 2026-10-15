//! The page's side of the events queue (`rambl_web_core::events`): one queue, shared with
//! every view through Leptos context, in place of a `thread_local` per channel.
//!
//! A channel moves here by adding its fact as an `Event` variant (with a claim like
//! `row_left`) in the core, then pushing it where the view used to note it and taking it
//! where the view used to read it.

use leptos::prelude::*;
use rambl_web_core::events::{Event, Events};

#[derive(Clone, Copy)]
pub struct Queue(StoredValue<Events>);

impl Queue {
    pub fn provide() -> Self {
        let queue = Queue(StoredValue::new(Events::default()));
        provide_context(queue);
        queue
    }

    pub fn push(self, event: Event) {
        self.0.update_value(|events| events.push(event));
    }

    /// Take the oldest event `claim` answers for.
    pub fn take<T>(self, claim: impl FnMut(&Event) -> Option<T>) -> Option<T> {
        self.0
            .try_update_value(|events| events.take(claim))
            .flatten()
    }

    /// Take every event `claim` answers for, oldest first.
    pub fn take_all<T>(self, claim: impl FnMut(&Event) -> Option<T>) -> Vec<T> {
        self.0
            .try_update_value(|events| events.take_all(claim))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    /// Rule 6 (ARCHITECTURE 8A.2): state shared between parts lives in the store or this
    /// queue, never a `thread_local`. These are the ones left; a migration deletes its
    /// line, and a new one fails here.
    #[test]
    fn no_new_thread_local_channels() {
        let left: &[(&str, usize)] = &[
            ("account.rs", 1),
            ("engine/link.rs", 1),
            ("engine/run.rs", 2),
            ("music.rs", 1),
            ("voice.rs", 1),
        ];
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut dirs = vec![src.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).expect("web/src is readable") {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let text = std::fs::read_to_string(&path).expect("a source file");
                    // Split so this test does not count itself; no brace, so the macro with no
                    // space before its brace counts too.
                    let count = text.matches(concat!("thread_local", "!")).count();
                    if count > 0 {
                        let name = path.strip_prefix(&src).expect("under src");
                        let name: Vec<_> = name.iter().map(|part| part.to_string_lossy()).collect();
                        found.push((name.join("/"), count));
                    }
                }
            }
        }
        found.sort();
        let left: Vec<_> = left
            .iter()
            .map(|(name, count)| (name.to_string(), *count))
            .collect();
        assert_eq!(found, left);
    }
}
