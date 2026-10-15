//! The store (ARCHITECTURE 8A.1): the one owner of the page's `Snapshot`. A view sends a
//! [`Command`]; the store applies what is certain at once, and keeps a reply only if it
//! is newer than what it shows, so replies that arrive out of order never put an older
//! list back on screen. Every refusal follows one policy: show the last list the server
//! confirmed, reload, and say why ([`Refusal::note`]).
//!
//! Pure: the page (`web/src/store.rs`) does the sending and the signals.

use todo_core::{Snapshot, Source, TaskId};

/// What a view asks the store to change.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// The stored tasks in this order, every one exactly once.
    Reorder(Vec<TaskId>),
}

impl Command {
    /// Change `snapshot` as the server will, where the result is certain, so the list
    /// shows it before the reply comes.
    fn apply(&self, snapshot: &mut Snapshot) {
        match self {
            Command::Reorder(ids) => reorder(snapshot, ids),
        }
    }
}

/// The stored tasks put in `ids`' order, in the places stored tasks held: anything not
/// named (a series occurrence) keeps its place. Each named task's `order` becomes its
/// position, as the server numbers them.
fn reorder(snapshot: &mut Snapshot, ids: &[TaskId]) {
    let slots: Vec<usize> = snapshot
        .tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| task.source != Source::Series)
        .map(|(slot, _)| slot)
        .collect();
    let mut named: Vec<_> = ids
        .iter()
        .filter_map(|id| {
            let slot = slots.iter().find(|&&slot| &snapshot.tasks[slot].id == id)?;
            Some(snapshot.tasks[*slot].clone())
        })
        .collect();
    if named.len() != slots.len() || ids.len() != slots.len() {
        // Not every stored task named exactly once: the server will refuse it, so show
        // nothing yet.
        return;
    }
    for (position, task) in named.iter_mut().enumerate() {
        task.order = position as i64;
    }
    for (slot, task) in slots.into_iter().zip(named) {
        snapshot.tasks[slot] = task;
    }
}

/// One request the store has sent, in the order they were sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ticket(u64);

/// Why the server did not take a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The server could not be reached.
    Network,
    /// The task changed elsewhere since this page last saw it.
    Stale,
    /// Anything else the server refused.
    Refused,
}

impl Refusal {
    /// What the page says about it: why the change failed, not whether the reload after
    /// it worked, so a refusal is never called "could not reach the server" just because
    /// the reload also failed.
    pub fn note(self, reloaded: bool) -> &'static str {
        match (self, reloaded) {
            (Refusal::Network, _) => "Could not reach the server, so that change was not saved.",
            (Refusal::Stale, true) => "That task changed elsewhere, so it was reloaded. Try again.",
            (Refusal::Stale, false) => "That task changed elsewhere. Reload the page to see it.",
            (Refusal::Refused, true) => "That change was not saved. The list was reloaded.",
            (Refusal::Refused, false) => "That change was not saved.",
        }
    }
}

/// What came of a reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It is the newest list: it is shown.
    Shown,
    /// A newer list is already shown: this one is dropped.
    Stale,
    /// The command was refused: the last confirmed list is shown again; reload, and say
    /// why.
    Refused(Refusal),
}

/// The list the page shows, and how fresh the newest confirmed one is.
#[derive(Clone, Debug)]
pub struct Store {
    /// What is shown: the newest confirmed list with any certain changes on top.
    shown: Snapshot,
    /// The newest list the server confirmed, and the request it answered.
    confirmed: Snapshot,
    confirmed_by: Ticket,
    /// The last ticket handed out.
    issued: Ticket,
}

impl Store {
    pub fn new(snapshot: Snapshot) -> Self {
        Store {
            shown: snapshot.clone(),
            confirmed: snapshot,
            confirmed_by: Ticket(0),
            issued: Ticket(0),
        }
    }

    /// The list to show.
    pub fn snapshot(&self) -> &Snapshot {
        &self.shown
    }

    /// A command is being sent: what is certain shows at once.
    pub fn send(&mut self, command: &Command) -> Ticket {
        command.apply(&mut self.shown);
        self.next_ticket()
    }

    /// A reload is being sent: nothing changes until it answers.
    pub fn reload(&mut self) -> Ticket {
        self.next_ticket()
    }

    fn next_ticket(&mut self) -> Ticket {
        self.issued = Ticket(self.issued.0 + 1);
        self.issued
    }

    /// A list the page already put on screen some other way (the first load, or a call
    /// site not yet sent through the store). It is taken as confirmed only if it was made
    /// later than the one confirmed; otherwise the store's list goes back on screen, and
    /// this returns it. The store's own lists come back here too, and return nothing.
    pub fn offer(&mut self, written: Snapshot) -> Option<Snapshot> {
        if written.generated > self.confirmed.generated {
            self.shown = written.clone();
            self.confirmed = written;
            None
        } else {
            (written != self.shown).then(|| self.shown.clone())
        }
    }

    /// The server answered `ticket`. A list is kept only if it is newer than the one
    /// confirmed: made later, or as late and asked for later.
    pub fn answer(&mut self, ticket: Ticket, reply: Result<Snapshot, Refusal>) -> Outcome {
        match reply {
            Ok(snapshot) => {
                let newer =
                    (snapshot.generated, ticket) > (self.confirmed.generated, self.confirmed_by);
                if !newer {
                    return Outcome::Stale;
                }
                self.shown = snapshot.clone();
                self.confirmed = snapshot;
                self.confirmed_by = ticket;
                Outcome::Shown
            }
            Err(refusal) => {
                self.shown = self.confirmed.clone();
                Outcome::Refused(refusal)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn fixture() -> Snapshot {
        serde_json::from_str(include_str!("../../../web/fixtures/snapshot.json")).expect("fixture")
    }

    /// The stored tasks' ids, in list order: what a reorder names.
    fn ids(snapshot: &Snapshot) -> Vec<TaskId> {
        snapshot
            .tasks
            .iter()
            .filter(|task| task.source != Source::Series)
            .map(|task| task.id.clone())
            .collect()
    }

    fn later(snapshot: &Snapshot, seconds: i64) -> Snapshot {
        let mut next = snapshot.clone();
        next.generated += Duration::seconds(seconds);
        next
    }

    #[test]
    fn a_reorder_shows_at_once_and_numbers_the_tasks_by_place() {
        let start = fixture();
        assert!(ids(&start).len() >= 2, "the fixture lists stored tasks");
        let mut order = ids(&start);
        order.reverse();
        let mut store = Store::new(start.clone());
        store.send(&Command::Reorder(order.clone()));
        assert_eq!(ids(store.snapshot()), order);
        let orders: Vec<i64> = store
            .snapshot()
            .tasks
            .iter()
            .filter(|task| task.source != Source::Series)
            .map(|task| task.order)
            .collect();
        assert_eq!(orders, (0..orders.len() as i64).collect::<Vec<_>>());
    }

    #[test]
    fn a_reorder_that_leaves_a_task_out_shows_nothing_until_the_server_answers() {
        let start = fixture();
        let mut store = Store::new(start.clone());
        store.send(&Command::Reorder(ids(&start)[1..].to_vec()));
        assert_eq!(store.snapshot(), &start);
    }

    #[test]
    fn an_older_reply_arriving_last_never_replaces_a_newer_one() {
        let start = fixture();
        let mut store = Store::new(start.clone());
        let first = store.reload();
        let second = store.reload();
        let newer = later(&start, 2);
        assert_eq!(store.answer(second, Ok(newer.clone())), Outcome::Shown);
        assert_eq!(store.answer(first, Ok(later(&start, 1))), Outcome::Stale);
        assert_eq!(store.snapshot(), &newer);
    }

    #[test]
    fn of_two_lists_made_in_the_same_instant_the_later_request_wins() {
        let start = fixture();
        let mut store = Store::new(start.clone());
        let first = store.reload();
        let second = store.reload();
        let mut a = later(&start, 1);
        a.tz = "Europe/London".into();
        let mut b = later(&start, 1);
        b.tz = "Asia/Tokyo".into();
        assert_eq!(store.answer(second, Ok(b.clone())), Outcome::Shown);
        assert_eq!(store.answer(first, Ok(a)), Outcome::Stale);
        assert_eq!(store.snapshot(), &b);
    }

    #[test]
    fn a_refused_command_shows_the_last_confirmed_list_again() {
        let start = fixture();
        let mut order = ids(&start);
        order.reverse();
        let mut store = Store::new(start.clone());
        let ticket = store.send(&Command::Reorder(order));
        assert_ne!(store.snapshot(), &start, "shown at once");
        assert_eq!(
            store.answer(ticket, Err(Refusal::Refused)),
            Outcome::Refused(Refusal::Refused)
        );
        assert_eq!(store.snapshot(), &start, "and taken back when refused");
    }

    #[test]
    fn a_list_from_elsewhere_is_taken_only_when_newer() {
        let start = fixture();
        let mut store = Store::new(start.clone());
        let newer = later(&start, 5);
        assert_eq!(store.offer(newer.clone()), None);
        assert_eq!(store.snapshot(), &newer);
        assert_eq!(store.offer(newer.clone()), None, "its own list is not news");
        // A reply to a request sent before it is now stale.
        let ticket = store.reload();
        assert_eq!(store.answer(ticket, Ok(later(&start, 1))), Outcome::Stale);
    }

    #[test]
    fn an_older_list_written_elsewhere_is_put_back_by_the_newer_one() {
        // A reorder answered at T2 races an edit not yet sent through the store, whose
        // reply was made at T1 but lands last: the page must end on the T2 list.
        let start = fixture();
        let mut order = ids(&start);
        order.reverse();
        let mut store = Store::new(start.clone());
        let ticket = store.send(&Command::Reorder(order.clone()));
        let mut reordered = later(&start, 2);
        reorder(&mut reordered, &order);
        assert_eq!(store.answer(ticket, Ok(reordered.clone())), Outcome::Shown);
        assert_eq!(store.offer(later(&start, 1)), Some(reordered.clone()));
        assert_eq!(store.snapshot(), &reordered);
    }

    #[test]
    fn a_refusal_says_why_it_failed_not_how_the_reload_went() {
        assert_eq!(Refusal::Network.note(true), Refusal::Network.note(false));
        assert!(Refusal::Stale.note(true).contains("reloaded"));
        assert!(!Refusal::Refused.note(false).contains("reloaded"));
    }
}
