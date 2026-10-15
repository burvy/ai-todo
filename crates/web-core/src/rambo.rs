//! Rambo's model (ARCHITECTURE 8A.4, step 4): what the page's inputs do to him, decided
//! here without the DOM so `cargo test` covers it. `web/src/rambo.rs` measures the page,
//! asks these, and does what they answer.

use crate::gesture::Vec2;
use crate::mascot::{self, Asked, Event, NotedHour, State};

/// A task under the finger, as the page shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskUnder {
    pub id: String,
    pub title: String,
}

/// A slip on the planner under the finger.
#[derive(Clone, Debug, PartialEq)]
pub struct SlipUnder {
    pub task: TaskUnder,
    /// One day of a repeating series.
    pub repeats: bool,
}

/// What lies where he is let go, as the page found it there.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Under {
    /// His nest.
    pub nest: bool,
    /// An empty planner hour.
    pub hour: Option<NotedHour>,
    /// A task row.
    pub row: Option<TaskUnder>,
    /// A slip on the planner.
    pub slip: Option<SlipUnder>,
}

/// How he was let go.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LetGo {
    /// The finger moved since it picked him up.
    pub moved: bool,
    /// He was listening.
    pub listening: bool,
    /// The flick's speed, if he was let go mid-flick.
    pub flick: Option<Vec2>,
    /// The page asks for reduced motion.
    pub reduced_motion: bool,
}

/// What letting go of him does.
#[derive(Clone, Debug, PartialEq)]
pub enum Released {
    /// He was not held: nothing.
    NotHeld,
    /// Let go where he was picked up: he is released, and a carried Rambo who was not
    /// listening sends what is typed (`submit`).
    Tapped { submit: bool },
    /// Let go somewhere else. `event` tells him where; `asked` is what he will ask
    /// about; `put_back` is whether he was put back in his nest from a daze (his brain
    /// hears it); `throw` is the speed he is thrown at.
    Dropped {
        event: Event,
        asked: Option<Asked>,
        put_back: bool,
        throw: Option<Vec2>,
    },
}

/// Letting go of him, held as `before`, the way `how` says, over what lies there
/// (`under`, looked up only when it decides anything). The nest wins over everything;
/// carried, an empty hour, then a row, then a slip; otherwise he lands on the page.
pub fn released(before: &State, how: LetGo, under: impl FnOnce() -> Under) -> Released {
    let carried = *before == State::Dragging;
    if !carried && *before != State::Inking {
        return Released::NotHeld;
    }
    if !how.moved {
        return Released::Tapped {
            submit: carried && !how.listening,
        };
    }
    let under = under();
    let dropped = |event: Event, asked: Option<Asked>| Released::Dropped {
        event,
        asked,
        put_back: false,
        throw: None,
    };
    if under.nest {
        let after = mascot::transition(before, Event::DropNest);
        return Released::Dropped {
            event: Event::DropNest,
            asked: None,
            put_back: mascot::put_back(before, &after),
            throw: None,
        };
    }
    if carried {
        if let Some(at) = under.hour {
            return dropped(Event::DropHour(at), None);
        }
        if let Some(TaskUnder { id, title }) = under.row {
            return dropped(Event::DropTask { task_id: id, title }, Some(Asked::Row));
        }
        if let Some(SlipUnder {
            task: TaskUnder { id, title },
            repeats,
        }) = under.slip
        {
            return dropped(
                Event::DropTask { task_id: id, title },
                Some(Asked::Slip { repeats }),
            );
        }
    }
    let (event, throw) = on_page(before, how.flick, how.reduced_motion);
    Released::Dropped {
        event,
        asked: None,
        put_back: false,
        throw,
    }
}

/// Let go on the page: carried and let go mid-flick, he is thrown at the flick's
/// speed, unless motion is reduced; otherwise he drops.
fn on_page(before: &State, flick: Option<Vec2>, reduced_motion: bool) -> (Event, Option<Vec2>) {
    match flick {
        Some(vel) if *before == State::Dragging && !reduced_motion => (Event::DropHard, Some(vel)),
        _ => (Event::DropEmpty, None),
    }
}

/// Whether a bubble `height` tall would run off the top of the screen sitting on
/// something whose top is at `sits_on` (his status line, above him), keeping
/// `edge` pixels clear: then it opens below him instead.
pub fn opens_below(sits_on: f64, height: f64, edge: f64) -> bool {
    sits_on - height < edge
}

/// How far to slide a bubble of `width`, centred on `centre_x`, so it keeps `edge`
/// pixels from both sides of a `screen` that wide. A bubble too wide for the screen
/// starts at the left edge, where its question begins.
pub fn bubble_offset(centre_x: f64, width: f64, screen: f64, edge: f64) -> f64 {
    let left = centre_x - width / 2.0;
    let fitted = if width > screen - 2.0 * edge {
        edge
    } else {
        left.clamp(edge, screen - edge - width)
    };
    fitted - left
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hour() -> NotedHour {
        NotedHour {
            phrase: "today 11:15".into(),
            at: chrono::DateTime::UNIX_EPOCH,
        }
    }

    fn task(id: &str, title: &str) -> TaskUnder {
        TaskUnder {
            id: id.into(),
            title: title.into(),
        }
    }

    /// Everything at once under the finger: the nest, an hour, a row and a slip.
    fn everything() -> Under {
        Under {
            nest: true,
            hour: Some(hour()),
            row: Some(task("t1", "Lab report")),
            slip: Some(SlipUnder {
                task: task("t2", "Gym"),
                repeats: true,
            }),
        }
    }

    /// Let go where he was picked up.
    const TAP: LetGo = LetGo {
        moved: false,
        listening: false,
        flick: None,
        reduced_motion: false,
    };

    /// Let go somewhere else, gently.
    const MOVED: LetGo = LetGo { moved: true, ..TAP };

    fn event_of(released: Released) -> Event {
        match released {
            Released::Dropped { event, .. } => event,
            other => panic!("not a drop: {other:?}"),
        }
    }

    #[test]
    fn only_a_held_rambo_is_let_go() {
        for before in [State::Home, State::Dazed, State::Thrown, State::Flying] {
            let released = released(&before, MOVED, || {
                panic!("nothing is looked up for {before:?}")
            });
            assert_eq!(released, Released::NotHeld);
        }
    }

    #[test]
    fn let_go_where_he_was_picked_up_he_is_tapped_and_sends_what_is_typed() {
        let nothing = || panic!("a tap looks nothing up");
        assert_eq!(
            released(&State::Dragging, TAP, nothing),
            Released::Tapped { submit: true }
        );
        let listening = LetGo {
            listening: true,
            ..TAP
        };
        assert_eq!(
            released(&State::Dragging, listening, nothing),
            Released::Tapped { submit: false },
            "listening, what he heard is what is sent"
        );
        assert_eq!(
            released(&State::Inking, TAP, nothing),
            Released::Tapped { submit: false },
            "writing, a tap is ink"
        );
    }

    #[test]
    fn the_nest_wins_and_a_dazed_rambo_put_back_is_heard() {
        assert_eq!(
            released(&State::Dragging, MOVED, everything),
            Released::Dropped {
                event: Event::DropNest,
                asked: None,
                put_back: false,
                throw: None
            }
        );
        let Released::Dropped { put_back, .. } = released(&State::Inking, MOVED, everything) else {
            panic!("a drop")
        };
        assert!(put_back, "writing, he was dazed: put back");
    }

    #[test]
    fn carried_an_hour_comes_before_a_row_and_a_row_before_a_slip() {
        let carried = |under: Under| released(&State::Dragging, MOVED, || under);
        let no_nest = Under {
            nest: false,
            ..everything()
        };
        assert_eq!(event_of(carried(no_nest.clone())), Event::DropHour(hour()));
        let row_and_slip = Under {
            hour: None,
            ..no_nest
        };
        assert_eq!(
            carried(row_and_slip.clone()),
            Released::Dropped {
                event: Event::DropTask {
                    task_id: "t1".into(),
                    title: "Lab report".into()
                },
                asked: Some(Asked::Row),
                put_back: false,
                throw: None
            }
        );
        let slip = Under {
            row: None,
            ..row_and_slip
        };
        assert_eq!(
            carried(slip),
            Released::Dropped {
                event: Event::DropTask {
                    task_id: "t2".into(),
                    title: "Gym".into()
                },
                asked: Some(Asked::Slip { repeats: true }),
                put_back: false,
                throw: None
            }
        );
    }

    #[test]
    fn writing_he_asks_nothing_and_lands_on_the_page() {
        let under = Under {
            nest: false,
            ..everything()
        };
        let flicked = LetGo {
            flick: Some((3.0, 0.0)),
            ..MOVED
        };
        assert_eq!(
            event_of(released(&State::Inking, flicked, || under)),
            Event::DropEmpty
        );
    }

    #[test]
    fn let_go_mid_flick_he_is_thrown_unless_motion_is_reduced() {
        let flick = Some((2.0, -1.0));
        let flicked = LetGo { flick, ..MOVED };
        assert_eq!(
            released(&State::Dragging, flicked, Under::default),
            Released::Dropped {
                event: Event::DropHard,
                asked: None,
                put_back: false,
                throw: flick
            }
        );
        let reduced = LetGo {
            reduced_motion: true,
            ..flicked
        };
        assert_eq!(
            event_of(released(&State::Dragging, reduced, Under::default)),
            Event::DropEmpty
        );
        assert_eq!(
            event_of(released(&State::Dragging, MOVED, Under::default)),
            Event::DropEmpty
        );
    }

    #[test]
    fn a_bubble_without_room_above_him_opens_below() {
        // A 120px bubble, 8px from the top: room from his top at 128px down.
        assert!(!opens_below(400.0, 120.0, 8.0));
        assert!(!opens_below(128.0, 120.0, 8.0));
        assert!(opens_below(127.0, 120.0, 8.0));
        assert!(opens_below(-20.0, 120.0, 8.0));
        // Room above him, but not above the line he is saying: it opens below.
        let (his_top, line_h) = (150.0, 39.0);
        assert!(!opens_below(his_top, 120.0, 8.0));
        assert!(opens_below(his_top - line_h, 120.0, 8.0));
    }

    // Rambo at x, a 200px bubble, a 1000px screen, 16px from each edge.

    #[test]
    fn a_bubble_that_fits_stays_centred() {
        assert_eq!(bubble_offset(500.0, 200.0, 1000.0, 16.0), 0.0);
    }

    #[test]
    fn near_the_left_edge_it_slides_right() {
        // It spans -70 to 130, so it moves 86 to start at 16.
        assert_eq!(bubble_offset(30.0, 200.0, 1000.0, 16.0), 86.0);
        assert_eq!(bubble_offset(10.0, 200.0, 1000.0, 16.0), 106.0);
    }

    #[test]
    fn near_the_right_edge_it_slides_left() {
        // It spans 870 to 1070, so it moves 86 to end at 984.
        assert_eq!(bubble_offset(970.0, 200.0, 1000.0, 16.0), -86.0);
    }

    #[test]
    fn a_bubble_wider_than_the_screen_starts_at_the_left_edge() {
        // It spans -100 to 1100; it starts at 16 and runs off the right instead.
        assert_eq!(bubble_offset(500.0, 1200.0, 1000.0, 16.0), 116.0);
        // Exactly as wide as the room between the edges: it fits.
        assert_eq!(bubble_offset(500.0, 968.0, 1000.0, 16.0), 0.0);
    }
}
