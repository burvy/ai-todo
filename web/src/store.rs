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
        Command::Patch(id, body) => api::patch_task(&id, body).await,
        Command::Revise(id, body) => api::revise_task(&id, body).await,
        Command::CreateTask(draft) => api::create_task(&draft).await,
        Command::DeleteTask(id) => api::delete_task(&id).await,
        Command::CreateSeries(body) => api::create_series(body).await,
        Command::PatchSeries { series, patch } => api::patch_series(&series, patch).await,
        Command::DeleteSeries(series) => api::delete_series(&series).await,
        Command::MarkOccurrence { series, date, mark } => {
            api::mark_occurrence(&series, &date, mark.word()).await
        }
        Command::ClearOccurrence { series, date } => api::clear_occurrence(&series, &date).await,
    }
}

/// The store, shared with every view through Leptos context.
#[derive(Clone, Copy)]
pub struct Store {
    core: StoredValue<Core>,
    /// The page's list and its note. Call sites not yet moved here still write these
    /// themselves; the list they write is offered to the store (`provide`).
    pub(crate) set_snapshot: WriteSignal<Snapshot>,
    pub(crate) set_error: WriteSignal<Option<String>>,
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

    /// Send `command` and forget it: what is certain shows at once, the server's answer
    /// replaces it, and a refusal shows the last confirmed list, reloads, and says why.
    pub fn dispatch(self, command: Command) {
        wasm_bindgen_futures::spawn_local(async move {
            self.send(command).await;
        });
    }

    /// The same, for a caller that acts on the result: whether the server took it (an
    /// edit sheet closes only then).
    pub async fn send(self, command: Command) -> bool {
        let Some(ticket) = self.core.try_update_value(|core| core.send(&command)) else {
            return false;
        };
        self.show();
        let reply = request(command).await.map_err(|error| refusal(&error));
        let Some(outcome) = self
            .core
            .try_update_value(|core| core.answer(ticket, reply))
        else {
            return false;
        };
        match outcome {
            Outcome::Shown => {
                // A note about an earlier refusal must not outlive the change that worked.
                self.set_error.set(None);
                self.show();
                true
            }
            // Taken, but a newer list is already shown.
            Outcome::Stale => true,
            Outcome::Refused(why) => {
                self.show();
                let reloaded = self.reload().await;
                self.set_error.set(Some(why.note(reloaded).to_owned()));
                false
            }
        }
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

    /// The note names why the edit failed, not whether the reload after it worked.
    #[test]
    fn a_refused_edit_says_why() {
        let stale = server(ErrorCode::StaleUpdate);
        assert!(refusal(&stale).note(true).contains("changed elsewhere"));
        assert!(refusal(&stale).note(false).contains("Reload the page"));
        let offline = api::ApiError::Network("offline".into());
        for reloaded in [true, false] {
            assert!(
                refusal(&offline)
                    .note(reloaded)
                    .contains("Could not reach the server")
            );
        }
        // A server refusal is never reported as an unreachable server, reload or not.
        let refused = server(ErrorCode::BadRequest);
        for reloaded in [true, false] {
            assert!(!refusal(&refused).note(reloaded).contains("reach"));
            assert!(refusal(&refused).note(reloaded).contains("not saved"));
        }
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
