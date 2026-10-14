//! Sending an edit to a task and showing what the server holds afterwards.

use leptos::prelude::*;
use todo_core::Snapshot;

use crate::{api, settings};

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

/// The body that moves `id` to the next priority, against the version held now.
pub(crate) fn next_priority_body(snapshot: &Snapshot, id: &str) -> Option<api::PatchTaskBody> {
    let task = held(snapshot, id)?;
    let next = match task.priority {
        todo_core::Priority::Low => todo_core::Priority::Medium,
        todo_core::Priority::Medium => todo_core::Priority::High,
        todo_core::Priority::High => todo_core::Priority::Low,
        todo_core::Priority::Critical => todo_core::Priority::High,
    };
    Some(
        api::PatchTaskBody::new()
            .priority(next)
            .updated_at(task.updated_at),
    )
}

/// What the page tells the user when an edit did not apply.
///
/// The wording follows why the edit failed, not whether the reload afterwards worked: a
/// refusal from the server is not "could not reach the server" just because the reload
/// that followed also failed.
fn refused_note(error: &api::ApiError, reloaded: bool) -> &'static str {
    match (error, reloaded) {
        (api::ApiError::Network(_), _) => {
            "Could not reach the server, so that change was not saved."
        }
        (
            api::ApiError::Server(todo_core::ApiError {
                code: todo_core::ErrorCode::StaleUpdate,
                ..
            }),
            true,
        ) => "That task changed elsewhere, so it was reloaded. Try again.",
        (
            api::ApiError::Server(todo_core::ApiError {
                code: todo_core::ErrorCode::StaleUpdate,
                ..
            }),
            false,
        ) => "That task changed elsewhere. Reload the page to see it.",
        (_, true) => "That change was not saved. The list was reloaded.",
        (_, false) => "That change was not saved.",
    }
}

/// Where an edit's result goes: the list it replaces, the note that explains a refusal,
/// and the view settings that say how many days the list covers.
#[derive(Clone, Copy)]
pub(crate) struct EditSink {
    pub(crate) set_snapshot: WriteSignal<Snapshot>,
    pub(crate) set_error: WriteSignal<Option<String>>,
    pub(crate) view_settings: ReadSignal<settings::Settings>,
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
    match api::patch_task(id, body).await {
        Ok(snapshot) => {
            // A note about an earlier refusal must not outlive the retry that worked.
            sink.set_error.set(None);
            sink.set_snapshot.set(snapshot);
            true
        }
        Err(error) => {
            let days = sink.view_settings.get_untracked().days;
            let reloaded = match api::fetch_snapshot(&api::today_date(), days).await {
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

        let bump = next_priority_body(&snapshot, &id).expect("task is listed");
        assert_eq!(bump.updated_at, Some(snapshot.tasks[0].updated_at));
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
    fn no_edit_is_built_for_a_task_that_is_gone() {
        let snapshot = api::fixture_snapshot();
        assert!(toggle_done_body(&snapshot, "nope").is_none());
        assert!(next_priority_body(&snapshot, "nope").is_none());
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
