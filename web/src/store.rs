//! The page's side of the store (ARCHITECTURE 8A.1): the one place a view sends a change
//! to the server. The rules (what shows at once, which reply wins, what a refusal does)
//! are in `rambl_web_core::store`; this file only sends, and writes the page's signals.
//!
//! A call site moves here by adding its change as a `Command` (with its certain effect in
//! `Command::apply`) and its request in [`request`], then calling [`Store::dispatch`].

use crate::api;
use leptos::prelude::*;
use rambl_web_core::store::{Command, Outcome, Refusal, Store as Core};
use todo_core::{ErrorCode, Snapshot};

/// Why the server did not take a change, as the store's one refusal policy sees it.
pub(crate) fn refusal(error: &api::ApiError) -> Refusal {
    match error {
        api::ApiError::Network(_) => Refusal::Network,
        api::ApiError::Server(todo_core::ApiError {
            code: ErrorCode::StaleUpdate,
            ..
        }) => Refusal::Stale,
        _ => Refusal::Refused,
    }
}

async fn request(command: Command) -> Result<Snapshot, api::ApiError> {
    match command {
        Command::Reorder(ids) => api::reorder_tasks(ids).await,
    }
}

/// The store, shared with every view through Leptos context.
#[derive(Clone, Copy)]
pub struct Store {
    core: StoredValue<Core>,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
}

impl Store {
    /// A store over the page's list. Call sites not yet moved here still write
    /// `snapshot` themselves; every list they write is offered to the store, which takes
    /// it only if it is newer than what the server last confirmed to it, and otherwise
    /// puts its own list back on screen.
    pub fn provide(
        snapshot: ReadSignal<Snapshot>,
        set_snapshot: WriteSignal<Snapshot>,
        set_error: WriteSignal<Option<String>>,
    ) -> Self {
        let store = Store {
            core: StoredValue::new(Core::new(snapshot.get_untracked())),
            set_snapshot,
            set_error,
        };
        Effect::new(move |_| {
            let written = snapshot.get();
            // The write-back equals the store's list, which it then accepts: no loop.
            if let Some(back) = store
                .core
                .try_update_value(|core| core.offer(written))
                .flatten()
            {
                store.set_snapshot.set(back);
            }
        });
        provide_context(store);
        store
    }

    /// Send `command`: what is certain shows at once, the server's answer replaces it,
    /// and a refusal shows the last confirmed list, reloads, and says why.
    pub fn dispatch(self, command: Command) {
        let ticket = self.core.try_update_value(|core| core.send(&command));
        let Some(ticket) = ticket else { return };
        self.show();
        wasm_bindgen_futures::spawn_local(async move {
            let reply = request(command).await.map_err(|error| refusal(&error));
            let Some(outcome) = self
                .core
                .try_update_value(|core| core.answer(ticket, reply))
            else {
                return;
            };
            match outcome {
                Outcome::Shown => {
                    // A note about an earlier refusal must not outlive the change that worked.
                    self.set_error.set(None);
                    self.show();
                }
                Outcome::Stale => {}
                Outcome::Refused(why) => {
                    self.show();
                    let reloaded = self.reload().await;
                    self.set_error.set(Some(why.note(reloaded).to_owned()));
                }
            }
        });
    }

    /// Fetch the list again; whether a list came back.
    async fn reload(self) -> bool {
        let Some(ticket) = self.core.try_update_value(|core| core.reload()) else {
            return false;
        };
        let Ok(snapshot) = api::fetch_snapshot(&api::today_date()).await else {
            return false;
        };
        if self
            .core
            .try_update_value(|core| core.answer(ticket, Ok(snapshot)))
            == Some(Outcome::Shown)
        {
            self.show();
        }
        true
    }

    fn show(self) {
        if let Some(shown) = self.core.try_with_value(|core| core.snapshot().clone()) {
            self.set_snapshot.set(shown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(code: ErrorCode) -> api::ApiError {
        api::ApiError::Server(todo_core::ApiError {
            code,
            message: String::new(),
        })
    }

    #[test]
    fn each_failure_meets_the_one_refusal_policy() {
        assert_eq!(
            refusal(&api::ApiError::Network("down".into())),
            Refusal::Network
        );
        assert_eq!(refusal(&server(ErrorCode::StaleUpdate)), Refusal::Stale);
        assert_eq!(
            refusal(&api::ApiError::Decode("junk".into())),
            Refusal::Refused
        );
    }
}
