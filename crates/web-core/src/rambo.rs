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

/// What picking him up does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grabbed {
    /// He is carried: held still, he listens; a drag only carries him.
    Carried,
    /// Dazed, he writes: the drag is ink from his nib.
    Writing,
}

/// Picking him up, as `before`: `None` while his pen is busy or when a grab changes
/// nothing (splatting, on his way home, asking).
pub fn grabbed(before: &State) -> Option<Grabbed> {
    if mascot::pen_busy(before) {
        return None;
    }
    match mascot::transition(before, Event::Grab) {
        State::Dragging => Some(Grabbed::Carried),
        State::Inking => Some(Grabbed::Writing),
        _ => None,
    }
}

/// What his key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keyed {
    /// He was listening: he stops and is released.
    StopsListening,
    /// He is picked up and listens.
    Listens,
    /// Nothing, but the key is still his.
    Nothing,
}

/// Space or Enter on him, as `before`, `talking` or not; any other `key` is not his
/// (`None`), so the page keeps it.
pub fn keyed(key: &str, talking: bool, before: &State) -> Option<Keyed> {
    if key != " " && key != "Enter" {
        return None;
    }
    Some(if talking {
        Keyed::StopsListening
    } else if mascot::listens_on_grab(before) {
        Keyed::Listens
    } else {
        Keyed::Nothing
    })
}

/// Whether a bubble `height` tall would run off the top of the screen sitting on
/// something whose top is at `sits_on` (his status line, above him), keeping
/// `edge` pixels clear: then it opens below him instead.
pub fn opens_below(sits_on: f64, height: f64, edge: f64) -> bool {
    sits_on - height < edge
}

/// A box on the screen, in CSS px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

/// The nudge, in his own px, that keeps his line `edge` px inside a `screen` that wide:
/// slid in from a side, and below him (`his_bottom`) when it would run off the top.
/// It is worked out from where the line is `drawn`, nudged by `shift` and drawn at his
/// `scale`, so his lean, size and sink count, not only where his middle is; worked out
/// again on every frame he moves, the line never crosses an edge on the way.
pub fn fit_line(
    drawn: Rect,
    shift: (f64, f64),
    scale: f64,
    his_bottom: f64,
    screen: f64,
    edge: f64,
) -> (f64, f64) {
    let left = drawn.left - shift.0 * scale;
    let top = drawn.top - shift.1 * scale;
    let across = bubble_offset(left + drawn.width / 2.0, drawn.width, screen, edge);
    let down = if opens_below(top + drawn.height, drawn.height, edge) {
        his_bottom + edge - top
    } else {
        0.0
    };
    (across / scale, down / scale)
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
    fn he_is_carried_from_home_and_a_throw_and_writes_when_dazed() {
        assert_eq!(grabbed(&State::Home), Some(Grabbed::Carried));
        assert_eq!(grabbed(&State::Thrown), Some(Grabbed::Carried), "caught");
        assert_eq!(grabbed(&State::WritingRow), Some(Grabbed::Carried));
        assert_eq!(grabbed(&State::Dazed), Some(Grabbed::Writing));
    }

    #[test]
    fn a_busy_pen_or_a_grab_that_changes_nothing_is_not_a_grab() {
        for before in [
            State::Flying,
            State::WritingCaret,
            State::Splatting,
            State::Skittering,
            State::Covered,
        ] {
            assert_eq!(grabbed(&before), None, "{before:?}");
        }
    }

    #[test]
    fn space_or_enter_starts_and_stops_his_listening() {
        assert_eq!(keyed(" ", false, &State::Home), Some(Keyed::Listens));
        assert_eq!(keyed("Enter", false, &State::Home), Some(Keyed::Listens));
        assert_eq!(
            keyed(" ", true, &State::Dragging),
            Some(Keyed::StopsListening)
        );
        assert_eq!(
            keyed("Enter", false, &State::Dazed),
            Some(Keyed::Nothing),
            "dazed, he does not listen"
        );
        assert_eq!(keyed("a", false, &State::Home), None);
        assert_eq!(keyed("Escape", true, &State::Dragging), None);
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

    fn line_at(left: f64, top: f64) -> Rect {
        Rect {
            left,
            top,
            width: 200.0,
            height: 40.0,
        }
    }

    #[test]
    fn a_line_with_room_stays_where_it_is_drawn() {
        assert_eq!(
            fit_line(line_at(400.0, 300.0), (0.0, 0.0), 1.0, 500.0, 1000.0, 8.0),
            (0.0, 0.0)
        );
    }

    #[test]
    fn a_line_he_leans_past_an_edge_slides_back_in() {
        // Leaning, his line is drawn 11px past the right edge, wherever his middle is.
        let (across, down) = fit_line(line_at(803.0, 300.0), (0.0, 0.0), 1.0, 500.0, 1000.0, 8.0);
        assert_eq!((across, down), (-11.0, 0.0));
    }

    #[test]
    fn the_nudge_already_drawn_is_not_counted_twice() {
        // Drawn already nudged 11px in, at the edge: the nudge stays 11px, not 22.
        let fitted = fit_line(line_at(792.0, 300.0), (-11.0, 0.0), 1.0, 500.0, 1000.0, 8.0);
        assert_eq!(fitted, (-11.0, 0.0));
        // Moved back to the middle, the nudge goes.
        let middle = fit_line(line_at(389.0, 300.0), (-11.0, 0.0), 1.0, 500.0, 1000.0, 8.0);
        assert_eq!(middle, (0.0, 0.0));
    }

    #[test]
    fn near_the_top_it_drops_below_him_in_his_own_px() {
        // Drawn from 1px above the top, he 100px tall below it, at twice his size: it
        // goes 8px under his bottom, which is 108px on screen and 54px of his own.
        let (_, down) = fit_line(line_at(400.0, -1.0), (0.0, 0.0), 2.0, 99.0, 1000.0, 8.0);
        assert_eq!(down, 54.0);
        // Already dropped, it stays dropped by the same amount.
        let (_, again) = fit_line(line_at(400.0, 107.0), (0.0, 54.0), 2.0, 99.0, 1000.0, 8.0);
        assert_eq!(again, 54.0);
    }
}
