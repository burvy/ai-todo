//! Sending an edit to a task and showing what the server holds afterwards.

use leptos::prelude::*;
use todo_core::Snapshot;

use crate::api;

/// The task `id` as the list holds it right now.
///
/// A row keeps the `Task` it was first drawn with, so an edit must read the task here,
/// at click time: its `updated_at` moves on with every accepted change, and sending the
/// old one is refused as stale.
pub(crate) fn held<'a>(snapshot: &'a Snapshot, id: &str) -> Option<&'a todo_core::Task> {
    snapshot.tasks.iter().find(|task| task.id.0 == id)
}

/// The body that flips `id` between done and not done, against the version held now.
pub(crate) fn toggle_done_body(snapshot: &Snapshot, id: &str) -> Option<api::PatchTaskBody> {
    let task = held(snapshot, id)?;
    Some(
        api::PatchTaskBody::new()
            .done(!task.done)
            .updated_at(task.updated_at),
    )
}

/// The note when a series write is refused: the server's own words say best
/// why (a day the rule never repeats, a series gone, the network down). A
/// series has no `updated_at` to go stale, so unlike a task edit there is
/// nothing to reload first.
pub(crate) fn series_note(error: &api::ApiError) -> String {
    match error {
        api::ApiError::Server(e) => e.message.clone(),
        other => other.to_string(),
    }
}

/// The browser's own confirm dialog — for the one destructive button, so a
/// whole series (every day still to come) is never deleted by a slip. The
/// native test build has no window and never confirms.
pub(crate) fn confirmed(message: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .is_some_and(|window| window.confirm_with_message(message).is_ok_and(|ok| ok))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = message;
        false
    }
}

/// What the page tells the user when an edit did not apply.
///
/// The wording follows why the edit failed, not whether the reload afterwards worked: a
/// refusal from the server is not "could not reach the server" just because the reload
/// that followed also failed.
fn refused_note(error: &api::ApiError, reloaded: bool) -> &'static str {
    crate::store::refusal(error).note(reloaded)
}

/// Where an edit's result goes: the list it replaces and the note that explains a refusal.
#[derive(Clone, Copy)]
pub(crate) struct EditSink {
    pub(crate) set_snapshot: WriteSignal<Snapshot>,
    pub(crate) set_error: WriteSignal<Option<String>>,
}

/// Sends an edit and shows what the server now holds. Returns whether the edit applied.
///
/// A refused edit (the task changed on another device, or the network failed) is not an
/// error to hide: the list is reloaded so the next click starts from the server's copy,
/// and a short note says why nothing happened.
pub(crate) async fn patch_and_show(
    id: &todo_core::TaskId,
    body: api::PatchTaskBody,
    sink: EditSink,
) -> bool {
    show(api::patch_task(id, body).await, sink).await
}

/// The same for a spoken change, which Jev turns into an edit on the server.
pub(crate) async fn revise_and_show(
    id: &todo_core::TaskId,
    body: api::ReviseBody,
    sink: EditSink,
) -> bool {
    show(api::revise_task(id, body).await, sink).await
}

/// The change for `id` that Rambo heard while its edit sheet was open, or `None`
/// when he heard nothing or the row is not a task the revise call can change: a
/// series row's sheet edits the whole series, which revise does not.
pub(crate) fn revise_body(snapshot: &Snapshot, id: &str, heard: &str) -> Option<api::ReviseBody> {
    let instruction = heard.trim();
    let task = revisable_task(snapshot, id)?;
    (!instruction.is_empty()).then(|| api::ReviseBody {
        instruction: instruction.to_owned(),
        updated_at: Some(task.updated_at),
    })
}

/// Whether a change Rambo heard for `heard_for` may be sent: only while that task's
/// sheet is still the one open. Closed, or showing another task, it is dropped.
pub(crate) fn heard_edit_applies(
    heard_for: Option<&todo_core::TaskId>,
    open_sheet: Option<&todo_core::TaskId>,
) -> bool {
    heard_for.is_some() && heard_for == open_sheet
}

/// Whether the sheet Rambo is listening for has just closed or moved to another
/// task, so the listening must end. A sheet not yet open (Edit was just chosen) is
/// not one that left.
pub(crate) fn sheet_left(
    heard_for: Option<&todo_core::TaskId>,
    was_open: Option<&todo_core::TaskId>,
    now_open: Option<&todo_core::TaskId>,
) -> bool {
    heard_for.is_some() && was_open == heard_for && now_open != heard_for
}

/// Whether Rambo should listen for a change to `id` when its sheet opens.
pub(crate) fn revisable(snapshot: &Snapshot, id: &str) -> bool {
    revisable_task(snapshot, id).is_some()
}

fn revisable_task<'a>(snapshot: &'a Snapshot, id: &str) -> Option<&'a todo_core::Task> {
    held(snapshot, id).filter(|task| task.source != todo_core::Source::Series)
}

async fn show(result: Result<Snapshot, api::ApiError>, sink: EditSink) -> bool {
    match result {
        Ok(snapshot) => {
            // A note about an earlier refusal must not outlive the retry that worked.
            sink.set_error.set(None);
            sink.set_snapshot.set(snapshot);
            true
        }
        Err(error) => {
            let reloaded = match api::fetch_snapshot(&api::today_date()).await {
                Ok(snapshot) => {
                    sink.set_snapshot.set(snapshot);
                    true
                }
                Err(_) => false,
            };
            sink.set_error
                .set(Some(refused_note(&error, reloaded).to_owned()));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row keeps the task it was first drawn with, so the second click on the same
    /// task must still carry the server's newest version and the current done state.
    #[test]
    fn an_edit_is_built_from_the_task_as_the_list_holds_it_now() {
        let mut snapshot = api::fixture_snapshot();
        let id = snapshot.tasks[0].id.0.clone();
        let first = toggle_done_body(&snapshot, &id).expect("task is listed");
        assert_eq!(first.done, Some(true));
        assert_eq!(first.updated_at, Some(snapshot.tasks[0].updated_at));

        // The server accepted that edit: the list now holds a newer version, done.
        snapshot.tasks[0].done = true;
        snapshot.tasks[0].updated_at += chrono::Duration::minutes(5);
        let second = toggle_done_body(&snapshot, &id).expect("task is listed");
        assert_eq!(second.done, Some(false), "must flip the current state");
        assert_eq!(second.updated_at, Some(snapshot.tasks[0].updated_at));
    }

    #[test]
    fn a_refused_edit_says_why() {
        let stale = api::ApiError::Server(todo_core::ApiError {
            code: todo_core::ErrorCode::StaleUpdate,
            message: String::new(),
        });
        assert!(refused_note(&stale, true).contains("changed elsewhere"));
        assert!(refused_note(&stale, false).contains("Reload the page"));
        let offline = api::ApiError::Network("offline".into());
        for reloaded in [true, false] {
            assert!(refused_note(&offline, reloaded).contains("Could not reach the server"));
        }
        // A server refusal is never reported as an unreachable server, reload or not.
        let refused = api::ApiError::Server(todo_core::ApiError {
            code: todo_core::ErrorCode::BadRequest,
            message: String::new(),
        });
        for reloaded in [true, false] {
            assert!(!refused_note(&refused, reloaded).contains("reach"));
            assert!(refused_note(&refused, reloaded).contains("not saved"));
        }
    }

    #[test]
    fn a_heard_change_revises_the_task_it_was_said_for() {
        let snapshot = api::fixture_snapshot();
        let task = snapshot
            .tasks
            .iter()
            .find(|task| task.source != todo_core::Source::Series)
            .expect("the fixture holds a plain task");
        let body = revise_body(&snapshot, &task.id.0, "  make it Friday at 3 ")
            .expect("a heard change on a listed task");
        assert_eq!(body.instruction, "make it Friday at 3");
        assert_eq!(
            body.updated_at,
            Some(task.updated_at),
            "stale edits are refused"
        );

        assert!(
            revise_body(&snapshot, &task.id.0, "  ").is_none(),
            "nothing heard"
        );
        assert!(
            revise_body(&snapshot, "nope", "make it Friday").is_none(),
            "task gone"
        );
        if let Some(row) = snapshot
            .tasks
            .iter()
            .find(|t| t.source == todo_core::Source::Series)
        {
            assert!(revise_body(&snapshot, &row.id.0, "make it Friday").is_none());
        }
    }

    #[test]
    fn a_heard_change_only_lands_on_its_own_open_sheet() {
        use todo_core::TaskId;
        let (a, b) = (TaskId("a".into()), TaskId("b".into()));
        assert!(heard_edit_applies(Some(&a), Some(&a)));
        assert!(!heard_edit_applies(Some(&a), None), "the sheet was closed");
        assert!(
            !heard_edit_applies(Some(&a), Some(&b)),
            "another task's sheet"
        );
        assert!(
            !heard_edit_applies(None, Some(&a)),
            "nothing was being listened for"
        );

        // Closing the sheet, or opening another task's, ends the listening.
        assert!(sheet_left(Some(&a), Some(&a), None));
        assert!(sheet_left(Some(&a), Some(&a), Some(&b)));
        // Edit was just chosen and the sheet is opening: keep listening.
        assert!(!sheet_left(Some(&a), None, Some(&a)));
        assert!(!sheet_left(Some(&a), None, None));
        // A refill keeps it open on the same task.
        assert!(!sheet_left(Some(&a), Some(&a), Some(&a)));
        assert!(!sheet_left(None, Some(&a), None), "not listening at all");
    }

    #[test]
    fn no_edit_is_built_for_a_task_that_is_gone() {
        let snapshot = api::fixture_snapshot();
        assert!(toggle_done_body(&snapshot, "nope").is_none());
    }

    #[test]
    fn a_refused_series_write_says_the_server_s_word() {
        let refused = api::ApiError::Server(todo_core::ApiError {
            code: todo_core::ErrorCode::BadRequest,
            message: "not an occurrence".into(),
        });
        assert_eq!(series_note(&refused), "not an occurrence");
        assert!(series_note(&api::ApiError::Network("offline".into())).contains("network error"));
    }
}
