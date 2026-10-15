//! The events queue (ARCHITECTURE 8A.1): one list of facts that views push and whoever
//! they are for takes. A fact says what happened (a row left the list, and where it
//! stood), never what to do about it, and carries what it needs: the core never measures
//! (rule 10), so a view reports geometry in the event.
//!
//! Pure: the page shares one queue through Leptos context (`web/src/events.rs`).

/// A box on the page, in viewport pixels: where a row stood, or where a scrap hangs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Spot {
    pub fn centre(self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// Something that happened on the page.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The row of task `id`, being marked done, stood at `from`: its strip at the pin
    /// flies from there.
    RowLeft { id: String, from: Spot },
    /// The strip of task `id`, being taped back, hung at `from`: its row flies back from
    /// there once the list shows it again.
    RowBack { id: String, from: Spot },
}

/// The facts not yet taken, oldest first.
#[derive(Clone, Debug, Default)]
pub struct Events {
    queue: Vec<Event>,
}

impl Events {
    pub fn push(&mut self, event: Event) {
        self.queue.push(event);
    }

    /// Take the oldest event `claim` answers for; the others stay, in order.
    pub fn take<T>(&mut self, mut claim: impl FnMut(&Event) -> Option<T>) -> Option<T> {
        let (at, taken) = self
            .queue
            .iter()
            .enumerate()
            .find_map(|(at, event)| claim(event).map(|taken| (at, taken)))?;
        self.queue.remove(at);
        Some(taken)
    }

    /// Take every event `claim` answers for, oldest first; the others stay, in order.
    pub fn take_all<T>(&mut self, mut claim: impl FnMut(&Event) -> Option<T>) -> Vec<T> {
        let mut taken = Vec::new();
        self.queue.retain(|event| match claim(event) {
            Some(answer) => {
                taken.push(answer);
                false
            }
            None => true,
        });
        taken
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// A claim on the row `id` leaving: where it stood.
pub fn row_left(id: &str) -> impl FnMut(&Event) -> Option<Spot> + '_ {
    move |event| match event {
        Event::RowLeft { id: left, from } if left == id => Some(*from),
        _ => None,
    }
}

/// A claim on the row `id` coming back: where its strip hung.
pub fn row_back(id: &str) -> impl FnMut(&Event) -> Option<Spot> + '_ {
    move |event| match event {
        Event::RowBack { id: back, from } if back == id => Some(*from),
        _ => None,
    }
}

/// A claim on any row coming back: which, and where its strip hung.
pub fn any_row_back(event: &Event) -> Option<(String, Spot)> {
    match event {
        Event::RowBack { id, from } => Some((id.clone(), *from)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(x: f64) -> Spot {
        Spot {
            x,
            y: 0.0,
            w: 10.0,
            h: 4.0,
        }
    }

    fn left(id: &str, x: f64) -> Event {
        Event::RowLeft {
            id: id.into(),
            from: spot(x),
        }
    }

    #[test]
    fn a_fact_is_taken_once_by_whoever_it_is_for() {
        let mut events = Events::default();
        events.push(left("a", 1.0));
        events.push(left("b", 2.0));
        assert_eq!(events.take(row_left("b")), Some(spot(2.0)));
        assert_eq!(events.take(row_left("b")), None, "taken once");
        assert_eq!(events.take(row_left("a")), Some(spot(1.0)));
        assert!(events.is_empty());
    }

    #[test]
    fn the_oldest_matching_fact_goes_first() {
        let mut events = Events::default();
        events.push(left("a", 1.0));
        events.push(left("a", 2.0));
        assert_eq!(events.take(row_left("a")), Some(spot(1.0)));
        assert_eq!(events.take(row_left("a")), Some(spot(2.0)));
    }

    #[test]
    fn taking_them_all_leaves_the_rest_in_order() {
        let mut events = Events::default();
        events.push(left("a", 1.0));
        events.push(left("b", 2.0));
        events.push(left("a", 3.0));
        events.push(left("c", 4.0));
        assert_eq!(events.take_all(row_left("a")), vec![spot(1.0), spot(3.0)]);
        assert_eq!(events.take(|_| Some(())), Some(()));
        assert_eq!(events.take(row_left("c")), Some(spot(4.0)));
        assert!(events.is_empty());
    }

    #[test]
    fn each_row_coming_back_is_taken_and_a_leaving_one_stays() {
        let mut events = Events::default();
        events.push(left("a", 1.0));
        events.push(Event::RowBack {
            id: "b".into(),
            from: spot(2.0),
        });
        events.push(Event::RowBack {
            id: "c".into(),
            from: spot(3.0),
        });
        assert_eq!(
            events.take_all(any_row_back),
            vec![("b".to_owned(), spot(2.0)), ("c".to_owned(), spot(3.0))]
        );
        assert_eq!(events.take(row_back("b")), None);
        assert_eq!(events.take(row_left("a")), Some(spot(1.0)));
    }

    #[test]
    fn the_middle_of_a_box_is_half_across_and_half_down() {
        assert_eq!(spot(2.0).centre(), (7.0, 2.0));
    }
}
