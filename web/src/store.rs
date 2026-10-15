//! The page's side of the store (ARCHITECTURE 8A.1): the one place a view sends a change
//! to the server. The rules (what shows at once, which reply wins, what a refusal does)
//! are in `rambl_web_core::store`; this file only sends, and writes the page's signals.
//!
//! A call site moves here by adding its change as a `Command` (with its certain effect in
//! `Command::apply`) and its request in [`request`], then calling [`Store::dispatch`].

use crate::api;
use leptos::prelude::*;
use rambl_web_core::store::{Command, Outcome, Refusal, Reply, Store as Core};
use todo_core::{ErrorCode, Snapshot};

/// Why the server did not take a change, as the store's one refusal policy sees it.
pub(crate) fn refusal(error: &api::ApiError) -> Refusal {
    match error {
        api::ApiError::Network(_) => Refusal::Network,
        api::ApiError::Server(todo_core::ApiError {
            code: ErrorCode::StaleUpdate,
            ..
        }) => Refusal::Stale,
        // An internal error's message is the server's own text (storage, a panic), not
        // words for the user.
        api::ApiError::Server(todo_core::ApiError {
            code: ErrorCode::Internal,
            ..
        }) => Refusal::Refused(None),
        api::ApiError::Server(refused) => Refusal::Refused(
            Some(refused.message.trim())
                .filter(|why| !why.is_empty())
                .map(str::to_owned),
        ),
        api::ApiError::Decode(_) => Refusal::Refused(None),
    }
}

/// A server's answer split for the store: the list (or why there is none) goes to the
/// core, and what it said besides the list goes back to the caller, only if it took the
/// change.
fn split(
    answer: Result<(Snapshot, Reply), api::ApiError>,
) -> (Result<Snapshot, Refusal>, Option<Reply>) {
    match answer {
        Ok((snapshot, reply)) => (Ok(snapshot), Some(reply)),
        Err(error) => (Err(refusal(&error)), None),
    }
}

/// Send `command`; the list the server answers with, and anything more it says.
async fn request(command: Command) -> Result<(Snapshot, Reply), api::ApiError> {
    let listed = match command {
        Command::Capture(text) => {
            let captured = api::capture_text(&text).await?;
            return Ok((captured.snapshot, Reply::Created(captured.created)));
        }
        Command::Reorder(ids) => api::reorder_tasks(ids).await,
        Command::Patch(id, body) => api::patch_task(&id, body).await,
        Command::Revise(id, body) => api::revise_task(&id, body).await,
        Command::MoveOccurrence { series, date, to } => {
            api::move_series_occurrence(&series, date, to).await
        }
        Command::CreateTask(draft) => api::create_task(&draft).await,
        Command::RestoreTask(task) => api::restore_task(&task).await,
        Command::DeleteTask(id) => api::delete_task(&id).await,
        Command::CreateSeries(body) => api::create_series(body).await,
        Command::PatchSeries { series, patch } => api::patch_series(&series, patch).await,
        Command::DeleteSeries(series) => api::delete_series(&series).await,
        Command::MarkOccurrence { series, date, mark } => {
            api::mark_occurrence(&series, &date, mark.word()).await
        }
        Command::ClearOccurrence { series, date } => api::clear_occurrence(&series, &date).await,
    };
    listed.map(|snapshot| (snapshot, Reply::Nothing))
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

    /// The same, for a caller that acts on the result: what the server said besides the
    /// list if it took the change (an edit sheet closes only then; the pen writes in the
    /// tasks a capture made), or `None` if it did not.
    pub async fn send(self, command: Command) -> Option<Reply> {
        self.send_with(command, |_, _| {}).await
    }

    /// `send`, and `before_show` first: it gets what the server said and the list about to
    /// be shown, so a view can take its part (the pen writes in what a create made) before
    /// the rows move. It runs only for a change the server took.
    pub async fn send_with(
        self,
        command: Command,
        before_show: impl FnOnce(&Reply, &Snapshot),
    ) -> Option<Reply> {
        let ticket = self.core.try_update_value(|core| core.send(&command))?;
        self.show();
        let (listed, reply) = split(request(command).await);
        let outcome = self
            .core
            .try_update_value(|core| core.answer(ticket, listed))?;
        if let Some(reply) = &reply
            && let Some(shown) = self.core.try_with_value(|core| core.snapshot().clone())
        {
            before_show(reply, &shown);
        }
        match outcome {
            Outcome::Shown => {
                // A note about an earlier refusal must not outlive the change that worked.
                self.set_error.set(None);
                self.show();
            }
            // Taken, but a newer list is already shown.
            Outcome::Stale => {}
            Outcome::Refused(why) => {
                self.show();
                let reloaded = self.reload().await;
                self.set_error.set(Some(why.note(reloaded)));
            }
        }
        reply
    }

    /// Fetch the list again; whether a list came back.
    pub(crate) async fn reload(self) -> bool {
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
    fn a_capture_hands_back_what_it_made_only_when_taken() {
        let snapshot: Snapshot =
            serde_json::from_str(include_str!("../fixtures/snapshot.json")).expect("fixture");
        let made = Reply::Created(vec![snapshot.tasks[0].id.clone()]);
        assert_eq!(
            split(Ok((snapshot.clone(), made.clone()))),
            (Ok(snapshot), Some(made))
        );
        assert_eq!(
            split(Err(server(ErrorCode::StaleUpdate))),
            (Err(Refusal::Stale), None)
        );
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
            Refusal::Refused(None)
        );
        let mut said = server(ErrorCode::BadRequest);
        if let api::ApiError::Server(error) = &mut said {
            error.message = "a deadline cannot be at midnight".into();
        }
        assert_eq!(
            refusal(&said),
            Refusal::Refused(Some("a deadline cannot be at midnight".into()))
        );
    }

    #[test]
    fn an_internal_error_never_shows_the_servers_own_text() {
        let mut broke = server(ErrorCode::Internal);
        if let api::ApiError::Server(error) = &mut broke {
            error.message = "database is locked".into();
        }
        assert_eq!(refusal(&broke), Refusal::Refused(None));
    }
}
