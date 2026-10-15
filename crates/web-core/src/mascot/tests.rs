use super::look::art_folder;
use super::*;

/// Every pose the app can ask for. The path test walks this list, so a new
/// pose that ships without art fails the build.
const REACHABLE_POSES: [Pose; 10] = [
    Pose::Idle,
    Pose::Teasing,
    Pose::Listening,
    Pose::Writing,
    Pose::Flying,
    Pose::Dozing,
    Pose::Happy,
    Pose::Sulking,
    Pose::Worried,
    Pose::Lobotomized,
];

/// Where a page-relative art path lives in the checkout.
fn art_file(path: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../web")
        .join(path)
}

/// Every tool the gear menu offers.
const PENS: [Pen; 5] = [
    Pen::FountainPen,
    Pen::Pencil,
    Pen::Ballpoint,
    Pen::Quill,
    Pen::Marker,
];

#[test]
fn grab_listens_and_release_goes_home() {
    let state = transition(&State::Home, Event::Grab);
    assert_eq!(state, State::Dragging);
    assert_eq!(transition(&state, Event::Release), State::Home);
}

#[test]
fn a_gentle_drop_on_the_page_sends_him_home_undazed() {
    let state = transition(&State::Home, Event::Grab);
    assert_eq!(transition(&state, Event::DropEmpty), State::Home);
}

#[test]
fn only_a_smack_dazes_him_and_after_the_daze_he_skitters_home() {
    let thrown = transition(&transition(&State::Home, Event::Grab), Event::DropHard);
    assert_eq!(thrown, State::Thrown);
    assert!(is_off_nest(&thrown));
    assert_eq!(
        transition(&thrown, Event::Return),
        State::Home,
        "it just stopped"
    );
    assert_eq!(transition(&thrown, Event::Grab), State::Dragging, "caught");
    let splat = transition(&thrown, Event::Smack);
    assert_eq!(splat, State::Splatting);
    let dazed = transition(&splat, Event::Return);
    assert_eq!(dazed, State::Dazed);
    let skittering = transition(&dazed, Event::Return);
    assert_eq!(skittering, State::Skittering);
    assert_eq!(transition(&skittering, Event::Return), State::Home);
}

#[test]
fn only_the_first_drag_listens_a_dazed_drag_writes() {
    assert!(listens_on_grab(&State::Home));
    assert!(!listens_on_grab(&State::Dazed));
    let state = transition(&State::Dazed, Event::Grab);
    assert_eq!(state, State::Inking);
    assert_eq!(pose_for(&state, "", false), Pose::Writing);
    // Wherever a writing drag ends he is still dazed, task rows included.
    for drop in [
        Event::Release,
        Event::DropEmpty,
        Event::DropHard,
        Event::DropTask {
            task_id: "t1".into(),
            title: "Lab report".into(),
        },
    ] {
        assert_eq!(transition(&state, drop), State::Dazed);
    }
    // Not while he is on his way home or splatting either.
    assert!(!listens_on_grab(&State::Skittering));
    assert!(!listens_on_grab(&State::Splatting));
}

#[test]
fn putting_him_back_in_the_nest_ends_the_daze_and_is_forgiven() {
    let state = transition(&State::Inking, Event::DropNest);
    assert_eq!(state, State::Home);
    assert!(put_back(&State::Inking, &state));
    assert!(listens_on_grab(&state), "home again, he listens again");
    // Dropping a listening Rambo on his nest just sends him home.
    assert_eq!(transition(&State::Dragging, Event::DropNest), State::Home);
    assert!(!put_back(&State::Dragging, &State::Home));
    // Skittering home on his own is not being put back.
    assert!(!put_back(&State::Skittering, &State::Home));
}

#[test]
fn a_drop_on_or_just_beside_the_dish_puts_him_back() {
    let nest = (100.0, 100.0, 164.0, 164.0);
    assert!(over_nest((132.0, 132.0), nest));
    assert!(over_nest((100.0 - NEST_SLACK_PX, 170.0), nest));
    assert!(!over_nest((100.0 - NEST_SLACK_PX - 1.0, 132.0), nest));
    assert!(!over_nest((132.0, 300.0), nest));
}

#[test]
fn he_rests_inside_the_nest_box_whatever_its_size_or_place() {
    for (left, top, width, height) in [
        (286.0, 771.0, 104.0, 73.0),
        (1100.0, 24.0, 112.0, 96.0),
        (0.0, 0.0, 60.0, 50.0),
    ] {
        let (x, y) = nest_home((left, top, width, height));
        assert!((left..=left + width).contains(&x), "inside across");
        assert!((top..=top + height).contains(&y), "inside down");
        assert!(
            (x - (left + width / 2.0)).abs() < 1e-9,
            "in the middle across"
        );
        assert!(
            y < top + height / 2.0,
            "a little above the middle of the box"
        );
    }
}

#[test]
fn splatting_is_off_nest() {
    assert!(is_off_nest(&State::Splatting));
    assert!(is_off_nest(&State::Dragging));
    assert!(!is_off_nest(&State::Home));
}

#[test]
fn task_drop_preserves_target_then_choice_returns_home() {
    let state = transition(
        &State::Dragging,
        Event::DropTask {
            task_id: "t1".into(),
            title: "Lab report".into(),
        },
    );
    assert_eq!(
        state,
        State::Asking {
            task_id: "t1".into(),
            title: "Lab report".into()
        }
    );
    assert_eq!(transition(&state, Event::Choose), State::Home);
    assert_eq!(transition(&state, Event::Return), State::Home);
}

#[test]
fn talk_is_available_while_away() {
    let state = State::Dragging;
    assert_eq!(transition(&state, Event::Talk), state);
}

#[test]
fn he_hops_between_spots_and_follows_what_he_tracks() {
    for moving in [State::Flying, State::WritingCaret, State::Skittering] {
        assert!(hops_there(&moving, true), "{moving:?}");
    }
    for tracking in [
        State::Dragging,
        State::Inking,
        State::WritingRow,
        State::Dazed,
        State::Splatting,
    ] {
        assert!(!hops_there(&tracking, false), "{tracking:?}");
    }
    assert!(hops_there(&State::Home, false), "the way home is hops");
    assert!(
        !hops_there(&State::Home, true),
        "home, he rides the scrolling nest"
    );
}

/// An hour he was dropped on.
fn hour() -> NotedHour {
    use chrono::TimeZone as _;
    NotedHour {
        phrase: "today 11:15".into(),
        at: chrono::Utc
            .with_ymd_and_hms(2026, 10, 15, 15, 15, 0)
            .single()
            .expect("a plain instant"),
    }
}

/// Noting a task at [`hour`].
fn noting() -> State {
    State::Noting { at: hour() }
}

/// Every state, and every event, the guard tests below walk.
fn every_state() -> Vec<State> {
    vec![
        State::Home,
        State::Dragging,
        State::Dazed,
        State::Inking,
        State::Splatting,
        State::Skittering,
        State::Asking {
            task_id: "t1".into(),
            title: "Lab report".into(),
        },
        noting(),
        State::Flying,
        State::WritingCaret,
        State::WritingRow,
    ]
}

fn every_event() -> Vec<Event> {
    vec![
        Event::Grab,
        Event::Release,
        Event::DropEmpty,
        Event::DropHard,
        Event::DropHour(hour()),
        Event::Swept,
        Event::DropTask {
            task_id: "t1".into(),
            title: "Lab report".into(),
        },
        Event::DropNest,
        Event::Return,
        Event::Choose,
        Event::Talk,
        Event::WriteCaret,
        Event::WriteRow,
        Event::SkipRow,
        Event::FlyHome,
    ]
}

#[test]
fn no_move_starts_without_the_notice_beat() {
    // DESIGN-DIRECTION §9: he never moves instantly. Any change of state that
    // sets him hopping somewhere new shows a face first; only landing at the
    // end of a trip does not.
    for before in every_state() {
        for event in every_event() {
            let after = transition(&before, event.clone());
            let sets_off = after != before
                && hops_there(&after, false)
                && !matches!(
                    (&before, &after),
                    (State::Flying | State::Skittering, State::Home)
                );
            assert_eq!(
                notice(&before, &after).is_some(),
                sets_off,
                "{before:?} --{event:?}--> {after:?}"
            );
        }
    }
}

#[test]
fn he_reacts_to_what_sends_him() {
    assert_eq!(
        notice(&State::Home, &State::WritingCaret),
        Some(Pose::Teasing)
    );
    assert_eq!(
        notice(&State::WritingRow, &State::Flying),
        Some(Pose::Teasing)
    );
    assert_eq!(
        notice(&State::Dazed, &State::Skittering),
        Some(Pose::Worried),
        "he comes round with a start"
    );
    assert_eq!(notice(&State::Flying, &State::Home), None, "he has landed");
    assert_eq!(notice(&State::Home, &State::Dragging), None, "you move him");
}

#[test]
fn the_perk_rises_and_settles_before_the_hop() {
    let beat = crate::tuning::NOTICE_BEAT_MS;
    assert_eq!(notice_squash(0), 0.0);
    assert!(notice_squash(beat / 2) < 0.0, "stretched at the top");
    assert!(notice_squash(beat).abs() < 1e-9, "back to rest as he hops");
    assert!(notice_squash(beat * 3).abs() < 1e-9);
}

#[test]
fn held_he_rides_above_the_finger() {
    let finger = (100.0, 300.0);
    let (x, y) = held_at(finger, 64.0, true);
    assert_eq!(x, 100.0);
    assert!(
        y + 32.0 < finger.1,
        "his bottom edge is above the finger: {y}"
    );
    let (_, cursor_y) = held_at(finger, 64.0, false);
    assert!(
        cursor_y + 32.0 < finger.1 && cursor_y > y,
        "a cursor holds him lower"
    );
}

#[test]
fn writing_caret_then_fly_home() {
    let state = transition(&State::Home, Event::WriteCaret);
    assert_eq!(state, State::WritingCaret);
    let state = transition(&state, Event::FlyHome);
    assert_eq!(state, State::Flying);
    assert_eq!(transition(&state, Event::FlyHome), State::Home);
}

#[test]
fn writing_row_fly_through_flying_state() {
    let state = transition(&State::Home, Event::WriteRow);
    assert_eq!(state, State::Flying);
    let state = transition(&state, Event::WriteRow);
    assert_eq!(state, State::WritingRow);
    let state = transition(&state, Event::FlyHome);
    assert_eq!(state, State::Flying);
    assert_eq!(transition(&state, Event::FlyHome), State::Home);
}

#[test]
fn writing_a_row_never_eases_him() {
    // Only a flight eases (`only_a_flight_eases_him_from_place_to_place`), so a
    // row written without the flight's class follows the ink frame by frame.
    assert!(!rambo_class(&State::WritingRow, "").contains("is-flying"));
    assert!(rambo_class(&State::Flying, "").contains("is-flying"));
}

#[test]
fn he_speaks_once_he_is_home() {
    assert!(speaks_later(&State::WritingRow));
    assert!(speaks_later(&State::Flying));
    assert!(!speaks_later(&State::Home));
    assert!(
        !speaks_later(&State::Dragging),
        "picked up, he answers at once"
    );
}

#[test]
fn anything_the_user_does_interrupts_a_row() {
    assert!(!pen_busy(&State::WritingRow));
    assert_eq!(transition(&State::WritingRow, Event::Grab), State::Dragging);
    assert_eq!(
        transition(&State::WritingRow, Event::WriteCaret),
        State::WritingCaret
    );
}

#[test]
fn pen_busy_during_writing_flight() {
    assert!(!pen_busy(&State::Home));
    assert!(!pen_busy(&State::Dragging));
    assert!(pen_busy(&State::Flying));
    assert!(pen_busy(&State::WritingCaret));
}

#[test]
fn rambo_classes_cover_every_state() {
    assert_eq!(rambo_class(&State::Home, ""), "rambo is-home");
    assert!(rambo_class(&State::Dragging, "").contains("is-dragging"));
    assert!(rambo_class(&State::Splatting, "").contains("is-splatting"));
    assert!(rambo_class(&State::Dazed, "").contains("is-dazed"));
    assert!(rambo_class(&State::Inking, "").contains("is-inking"));
    assert!(rambo_class(&State::Skittering, "").contains("is-skittering"));
    assert!(rambo_class(&State::Flying, "").contains("is-flying"));
    assert!(rambo_class(&State::WritingCaret, "").contains("is-caret"));
    assert!(rambo_class(&State::WritingRow, "").contains("is-writing"));
    assert!(rambo_class(&State::Home, "listening").contains("pose-listening"));
}

#[test]
fn submit_while_writing_caret_writes_row_not_stuck() {
    // When the user presses Add while Rambo is at the caret, the pen queue
    // starts writing the new row directly from WritingCaret. He must never
    // stay stuck in WritingCaret.
    let state = transition(&State::WritingCaret, Event::WriteRow);
    assert_eq!(state, State::Flying);
    // After flying to the row, he writes it
    let state = State::WritingRow;
    // When done, fly home
    let state = transition(&state, Event::FlyHome);
    assert_eq!(state, State::Flying);
    // And arrives home
    let state = transition(&state, Event::FlyHome);
    assert_eq!(state, State::Home);
}

#[test]
fn a_row_he_cannot_see_is_skipped_without_a_flight() {
    // After Add he sits at the caret; a new row that is off screen (the
    // other pane on a phone, or scrolled away) leaves nothing to fly to,
    // and he must not be left hovering at a caret the user has left.
    assert_eq!(
        transition(&State::WritingCaret, Event::SkipRow),
        State::Home
    );
    // Already home, the skip changes nothing.
    assert_eq!(transition(&State::Home, Event::SkipRow), State::Home);
}

#[test]
fn every_drawing_is_shown_through_its_art_group() {
    // The page's tokens reach the art only through <use>. An <img> loads
    // the file as a document of its own, and Rambo then follows the
    // operating system instead of the theme. So the drawing every pose and
    // every tool can reach must name a .svg#art reference, and it must be
    // the very file that pose and tool draw in. Spec: FE-2.
    for pen in PENS {
        for pose in REACHABLE_POSES {
            let reference = pose.art(pen);
            assert!(
                reference.ends_with(".svg#art"),
                "{reference} is not a drawing shown through its art group"
            );
            assert_eq!(
                reference,
                format!("{}#art", pose.art_file(pen)),
                "{reference} points at another file than the art on disk"
            );
        }
    }
}

#[test]
fn every_tool_draws_from_its_own_folder() {
    assert_eq!(
        Pose::Idle.art_file(Pen::FountainPen),
        "assets/rambo/fountain/idle.svg"
    );
    assert_eq!(
        Pose::Writing.art_file(Pen::Quill),
        "assets/rambo/quill/writing.svg"
    );
    // The fountain pen is the one tool whose slug and folder differ.
    assert_eq!(Pen::FountainPen.slug(), "fountain-pen");
    assert_eq!(
        [
            Pen::FountainPen,
            Pen::Pencil,
            Pen::Ballpoint,
            Pen::Quill,
            Pen::Marker
        ]
        .map(art_folder),
        ["fountain", "pencil", "ballpoint", "quill", "marker"]
    );
}

#[test]
fn assets_have_every_pose_for_every_tool() {
    // The paths are built from strings, so nothing stops them drifting from
    // the art on disk. Every pose a mood or a state can reach must exist as
    // a file, for every tool the gear menu offers.
    for pen in PENS {
        for pose in REACHABLE_POSES {
            let file = art_file(&pose.art_file(pen));
            assert!(file.is_file(), "missing art: {file:?}");
        }
    }
}

#[test]
fn the_tool_in_settings_picks_the_drawing() {
    // The whole point of the pen picker: the tool the page stores is the
    // folder the drawing comes from.
    let state = State::Home;
    assert_eq!(
        art_for(Pen::Marker, &state, "", false),
        "assets/rambo/marker/idle.svg#art"
    );
    assert_eq!(
        art_for(Pen::Ballpoint, &state, "cheer", false),
        "assets/rambo/ballpoint/happy.svg#art"
    );
}

#[test]
fn fx_art_lives_beside_the_skins() {
    for fx in [Fx::SleepyZ, Fx::Sparks, Fx::ScribbleCloud] {
        let file = art_file(&fx.art_file());
        assert!(file.is_file(), "missing art: {file:?}");
    }
}

/// The pictures are shown through `<use>`, so they must expose `#art` and take the
/// page's colour: `currentColor` or a fixed colour would make them follow the
/// operating system (or stay black on a dark page).
#[test]
fn fx_art_takes_the_page_theme() {
    for fx in [Fx::SleepyZ, Fx::Sparks, Fx::ScribbleCloud] {
        let svg = std::fs::read_to_string(art_file(&fx.art_file())).expect("fx art exists");
        assert!(svg.contains("<g id=\"art\""), "{fx:?} has no #art group");
        // The colour goes in a `style` declaration: `var()` in a presentation attribute
        // (`fill="var(..)"`) is not honoured by every engine, and the picture, and the
        // mask made from it, would draw nothing.
        assert!(
            svg.contains("style=\"stroke: var(--rambo-fx"),
            "{fx:?} ignores --rambo-fx"
        );
        for paint in ["fill=\"var(", "stroke=\"var("] {
            assert!(
                !svg.contains(paint),
                "{fx:?}: `{paint}` is a presentation attribute"
            );
        }
        assert!(
            !svg.contains("currentColor"),
            "{fx:?} follows the OS, not the page"
        );
        // A `--` inside an XML comment makes the file unparseable and the picture blank.
        let comment = svg.split("-->").next().unwrap_or_default();
        assert!(
            !comment.trim_start_matches("<!--").contains("--"),
            "{fx:?}: `--` in the comment"
        );
    }
}

#[test]
fn what_he_is_doing_beats_how_he_feels() {
    // Being dragged is the held/listening pose, whatever the mood says.
    assert_eq!(pose_for(&State::Dragging, "cheer", false), Pose::Listening);
    assert_eq!(pose_for(&State::WritingRow, "sleep", false), Pose::Writing);
    assert_eq!(pose_for(&State::Flying, "", false), Pose::Flying);
    // Dropped on a task he asks: the quizzical tilt.
    assert_eq!(
        pose_for(
            &State::Asking {
                task_id: "t1".into(),
                title: "Lab report".into()
            },
            "",
            false
        ),
        Pose::Teasing
    );
}

#[test]
fn he_doze_s_at_night_and_wakes_when_you_type() {
    assert_eq!(pose_for(&State::Home, "sleep", false), Pose::Dozing);
    assert_eq!(fx_for(&State::Home, "sleep", false), Some(Fx::SleepyZ));
    // A keystroke wakes him even in the middle of the night.
    assert_eq!(pose_for(&State::Home, "sleep", true), Pose::Teasing);
    assert_eq!(fx_for(&State::Home, "sleep", true), None);
}

#[test]
fn he_peeks_and_celebrates_and_sulks() {
    assert_eq!(pose_for(&State::Home, "", true), Pose::Teasing);
    assert_eq!(pose_for(&State::Home, "cheer", false), Pose::Happy);
    assert_eq!(fx_for(&State::Home, "cheer", false), Some(Fx::Sparks));
    assert_eq!(pose_for(&State::Home, "worried", false), Pose::Sulking);
    // Nothing to sulk about, nothing to draw.
    assert_eq!(pose_for(&State::Home, "home", false), Pose::Idle);
    assert_eq!(fx_for(&State::Home, "home", false), None);
    // An unknown pose name is not a mood.
    assert_eq!(mood_for("dangle", false), Mood::Resting);
    assert_eq!(mood_for("cheer", false), Mood::Celebrating);
}

#[test]
fn tidying_up_makes_him_annoyed() {
    assert_eq!(Mood::Annoyed.pose(), Pose::Worried);
    assert_eq!(Mood::Annoyed.fx(), Some(Fx::ScribbleCloud));
}

#[test]
fn a_lobotomized_rambl_stands_vacant() {
    for state in [State::Home, State::Splatting] {
        assert_eq!(pose_for(&state, "lobotomized", false), Pose::Lobotomized);
    }
}

#[test]
fn no_mood_picture_follows_him_off_the_nest() {
    for state in [
        State::Dragging,
        State::Dazed,
        State::Inking,
        State::Splatting,
        State::Skittering,
        State::Asking {
            task_id: "t1".into(),
            title: "Lab report".into(),
        },
        State::Flying,
        State::WritingCaret,
        State::WritingRow,
    ] {
        assert_eq!(fx_for(&state, "sleep", false), None, "{state:?}");
    }
}

#[test]
fn the_element_class_still_carries_the_pose() {
    // The CSS keeps owning how a pose looks; the image src owns who Rambo is.
    assert!(rambo_class(&State::Home, "dozing").contains("pose-dozing"));
    assert!(!rambo_class(&State::Home, "").contains("pose-"));
}

#[test]
fn only_a_still_hold_listens() {
    assert!(held_still((100.0, 100.0), (100.0, 100.0)));
    assert!(
        held_still((100.0, 100.0), (105.0, 106.0)),
        "a shaky finger is still"
    );
    assert!(
        !held_still((100.0, 100.0), (110.0, 100.0)),
        "a drag is not a hold"
    );
}

#[test]
fn dazed_he_takes_every_hold_and_drag_whatever_is_under_the_finger() {
    use crate::gesture::Gesture;
    let tap = Gesture::Tap;
    let hold = Gesture::Hold;
    let drag = Gesture::Drag {
        pos: (10.0, 10.0),
        vel: (0.0, 0.0),
    };
    let drop = Gesture::Drop { pos: (10.0, 10.0) };
    let fling = Gesture::Fling { vel: (2.0, 0.0) };
    let row = Gesture::Down {
        target: crate::gesture::Target::Row("t1".into()),
        at: (10.0, 10.0),
    };
    let asking = State::Asking {
        task_id: "t1".into(),
        title: "Lab report".into(),
    };
    // (state, gesture, his whatever the target)
    let table = [
        (State::Dazed, &hold, true),
        (State::Dazed, &drag, true),
        (State::Dazed, &tap, false),
        (State::Dazed, &row, false),
        (State::Inking, &drag, true),
        (State::Inking, &drop, true),
        (State::Inking, &fling, true),
        (State::Inking, &Gesture::Cancel, true),
        (State::Home, &hold, false),
        (State::Home, &drag, false),
        (State::Dragging, &hold, false),
        (State::Splatting, &hold, false),
        (State::Skittering, &drag, false),
        (asking, &hold, false),
        (State::Flying, &drag, false),
        (State::WritingCaret, &hold, false),
        (State::WritingRow, &drag, false),
    ];
    for (state, gesture, his) in table {
        assert_eq!(wants(&state, gesture), his, "{state:?} {gesture:?}");
    }
}

#[test]
fn he_offers_edit_move_and_done_and_nothing_else() {
    let labels = |on| offered(on).iter().map(|c| c.label()).collect::<Vec<_>>();
    assert_eq!(labels(Asked::Row), ["Edit", "Move", "Done"]);
}

#[test]
fn on_a_slip_he_offers_its_time_and_a_repeat_only_moves() {
    let slip = offered(Asked::Slip { repeats: false });
    assert_eq!(
        slip,
        [Choice::Move, Choice::Longer, Choice::Shorter, Choice::Off]
    );
    assert_eq!(offered(Asked::Slip { repeats: true }), [Choice::Move]);
    // Editing or ticking the task is the list's business, never a slip's.
    for on in [
        Asked::Slip { repeats: false },
        Asked::Slip { repeats: true },
    ] {
        assert!(
            !offered(on)
                .iter()
                .any(|c| matches!(c, Choice::Edit | Choice::Done))
        );
    }
}

#[test]
fn a_tap_off_his_bubble_is_never_mind() {
    use crate::gesture::Gesture;
    let asking = State::Asking {
        task_id: "t1".into(),
        title: "Lab report".into(),
    };
    let drag = Gesture::Drag {
        pos: (0.0, 0.0),
        vel: (0.0, 0.0),
    };
    // (state, gesture, on his bubble, never mind)
    let table = [
        (&asking, &Gesture::Tap, false, true),
        (&asking, &Gesture::Tap, true, false),
        (&asking, &Gesture::Hold, false, false),
        (&asking, &drag, false, false),
        (&asking, &Gesture::Drop { pos: (0.0, 0.0) }, false, false),
        (&State::Home, &Gesture::Tap, false, false),
        (&State::Dazed, &Gesture::Tap, false, false),
        // Noting a task, the capture box is what he asks with.
        (&noting(), &Gesture::Tap, false, true),
        (&noting(), &Gesture::Tap, true, false),
        (&noting(), &Gesture::Hold, false, false),
        (&noting(), &drag, false, false),
    ];
    for (state, gesture, on_bubble, dismissed) in table {
        assert_eq!(
            asking_dismissed_by(state, gesture, on_bubble),
            dismissed,
            "{state:?} {gesture:?} on bubble: {on_bubble}"
        );
    }
    // Never mind sends him home.
    assert_eq!(transition(&asking, Event::Return), State::Home);
}

#[test]
fn only_a_listening_rambo_dropped_on_an_hour_notes_a_task_there() {
    for state in every_state() {
        let after = transition(&state, Event::DropHour(hour()));
        match state {
            State::Dragging => assert_eq!(after, noting()),
            // Dazed and writing, a drop is ink, never a task.
            _ => assert_eq!(after, state, "{state:?} ignores an hour"),
        }
    }
    // He stays on the hour, listening, away from his nest, until the task is sent
    // or never mind; a grab does not pick him off it, and a leaf closing over the
    // planner catches him under it like anywhere on the page.
    assert!(!hops_there(&noting(), false));
    assert!(is_off_nest(&noting()));
    assert_eq!(pose_for(&noting(), "happy", false), Pose::Listening);
    assert_eq!(transition(&noting(), Event::Return), State::Home);
    // The hour he holds is the one he was dropped on, and a grab keeps it.
    assert_eq!(
        transition(&State::Dragging, Event::DropHour(hour())),
        State::Noting { at: hour() }
    );
    assert_eq!(transition(&noting(), Event::Grab), noting());
    assert_eq!(transition(&noting(), Event::Covered), State::Covered);
}

#[test]
fn only_a_hold_while_dazed_draws_him_over_to_write() {
    use crate::gesture::Gesture;
    let gestures = [
        Gesture::Tap,
        Gesture::Hold,
        Gesture::Drag {
            pos: (40.0, 60.0),
            vel: (0.2, 0.0),
        },
        Gesture::Drop { pos: (40.0, 60.0) },
        Gesture::Cancel,
    ];
    let states = [
        State::Home,
        State::Dragging,
        State::Dazed,
        State::Inking,
        State::Splatting,
        State::Skittering,
        State::Flying,
        State::WritingCaret,
        State::WritingRow,
        State::Asking {
            task_id: "t1".into(),
            title: "Lab report".into(),
        },
    ];
    for state in &states {
        for gesture in &gestures {
            let drawn = drawn_over(state, gesture);
            let expected = *state == State::Dazed && matches!(gesture, Gesture::Hold);
            assert_eq!(drawn.is_some(), expected, "{state:?} {gesture:?}");
            // What draws him over is a gesture he wants, and it leaves him writing.
            if let Some(event) = drawn {
                assert!(wants(state, gesture));
                assert_eq!(transition(state, event), State::Inking);
            }
        }
    }
}

#[test]
fn a_leaf_closing_on_him_out_on_the_page_traps_him_and_he_runs_home_undazed() {
    for out_on_the_page in [State::Dazed, State::Inking, State::Splatting] {
        assert_eq!(transition(&out_on_the_page, Event::Covered), State::Covered);
    }
    let asking = State::Asking {
        task_id: "t".into(),
        title: "x".into(),
    };
    assert_eq!(transition(&asking, Event::Covered), State::Covered);
    assert_eq!(
        transition(&State::Covered, Event::Uncovered),
        State::Home,
        "out from under it he is not dazed: he hops home, not skitters"
    );
    // Not on a page: in the nest, in hand, or in transit.
    for elsewhere in [
        State::Home,
        State::Dragging,
        State::Flying,
        State::WritingRow,
    ] {
        assert_eq!(transition(&elsewhere, Event::Covered), elsewhere);
    }
    // Stuck: he cannot be picked up from under the leaf.
    assert_eq!(transition(&State::Covered, Event::Grab), State::Covered);
    assert!(is_off_nest(&State::Covered));
}
