//! The planner's rules: where a slip sits on its day, which minute a drop lands on, what is
//! free, how a day is counted in the user's zone. No DOM; `web/src/week.rs` draws it.

use crate::settings::Settings;
use crate::tuning::{EDGE_MAX_PX, EDGE_ZONE_PX, HOUR_PX};
use chrono::{DateTime, Timelike, Utc};
use todo_core::{LocalContext, Placement, TaskId};

/// Where a task would start: a planner day (0 is today) and a minute of that day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub day: i64,
    pub minutes: u16,
}

/// Pixels per auto-scroll tick while a held slip is `distance_to_edge` from
/// the edge: zero outside the band, fastest right at it.
pub fn edge_nudge(distance_to_edge: f32) -> f32 {
    if distance_to_edge >= EDGE_ZONE_PX {
        0.0
    } else {
        ((EDGE_ZONE_PX - distance_to_edge) / EDGE_ZONE_PX) * EDGE_MAX_PX
    }
}

/// How far down the hour rows the current time sits. The rows start at start_hour.
pub fn now_line_top(now_minutes: u16, start_hour: u8, hour_px: f32) -> f32 {
    (f32::from(now_minutes) - f32::from(start_hour) * 60.0).max(0.0) / 60.0 * hour_px
}

/// Where the planner opens: about 08:00, or the current time if that is
/// already later — never in the middle of the night, never past the last
/// full hour we draw.
pub fn planner_open_minutes(now_minutes: u16, start_hour: u8, end_hour: u8) -> u16 {
    const OPEN_AT: u16 = 8 * 60;
    let top = u16::from(end_hour).saturating_sub(1) * 60;
    OPEN_AT
        .max(now_minutes)
        .clamp(u16::from(start_hour) * 60, top)
}

/// The handwritten label on a day column: short weekday and date in the
/// planner's zone, counted from today, so day 0 is "today".
pub fn day_label(ctx: &LocalContext, day: i64) -> String {
    (ctx.today() + chrono::Duration::days(day))
        .format("%a %-d")
        .to_string()
}

/// What a slip's row in the week grid is keyed on: the task AND its slot. Keying
/// on the task alone would leave a slip that moves or is re-planned inside its
/// own day painted at the old spot with the old time until the page reloads.
pub fn placement_key(placement: &Placement) -> (String, DateTime<Utc>, DateTime<Utc>) {
    (placement.task.0.clone(), placement.start, placement.end)
}

/// Top and height in pixels of a slip spanning `start`..`end`, in the
/// planner's zone: a day-crossing task is clipped at the working day's end.
pub fn placement_box(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    ctx: &LocalContext,
    settings: &Settings,
) -> (f32, f32) {
    let local_start = ctx.local(start);
    let local_end = ctx.local(end);
    let start_min = local_start.hour() as u16 * 60 + local_start.minute() as u16;
    let end_min = if local_end.date() > local_start.date() {
        u16::from(settings.end_hour) * 60
    } else {
        local_end.hour() as u16 * 60 + local_end.minute() as u16
    };
    placement_style(
        start_min,
        end_min,
        settings.start_hour,
        HOUR_PX,
        settings.end_hour,
    )
}

/// Placement top/height in pixels, clipped to planner bounds.
/// start/end are minutes-since-midnight in planner zone.
pub fn placement_style(
    start_minutes: u16,
    end_minutes: u16,
    planner_start_hour: u8,
    hour_px: f32,
    planner_end_hour: u8,
) -> (f32, f32) {
    let start = f32::from(start_minutes) - f32::from(planner_start_hour) * 60.0;
    let end = f32::from(end_minutes) - f32::from(planner_start_hour) * 60.0;
    let top = start.max(0.0) / 60.0 * hour_px;
    let max_bottom = (i32::from(planner_end_hour) - i32::from(planner_start_hour)) as f32 * hour_px;
    let bottom = (end / 60.0 * hour_px).min(max_bottom);
    let height = (bottom - top).max(0.0);
    (top, height)
}

/// The day column a placement falls in: its date in the planner's zone, counted from
/// today. Day 0 is today.
pub fn day_index(start: DateTime<Utc>, ctx: &LocalContext) -> i64 {
    (ctx.local(start).date() - ctx.today()).num_days()
}

/// The placements that fall on planner day `day`, in their original order.
pub fn placements_on_day(placements: &[Placement], day: i64, ctx: &LocalContext) -> Vec<Placement> {
    placements
        .iter()
        .filter(|placement| day_index(placement.start, ctx) == day)
        .cloned()
        .collect()
}

/// The minute a drop lands on, from a pixel offset inside a day column.
/// Round to the quarter hour, and never land at the very end of the day:
/// a zero-length task at 24:00 cannot start.
pub fn drop_minute(
    offset_px: f32,
    planner_start_hour: u8,
    planner_end_hour: u8,
    hour_px: f32,
) -> u16 {
    const SNAP: f32 = 15.0;
    let start_min = f32::from(planner_start_hour) * 60.0;
    let end_min = f32::from(planner_end_hour) * 60.0;
    let raw = start_min + offset_px.max(0.0) / hour_px * 60.0;
    let snapped = (raw / SNAP).round() * SNAP;
    snapped.clamp(start_min, (end_min - SNAP).max(start_min)) as u16
}

/// The instant a slot starts: planner day + minute of the day, read in the
/// planner's zone. None where the local time does not exist (DST jump).
pub fn drop_start(ctx: &LocalContext, day: i64, minutes: u16) -> Option<DateTime<Utc>> {
    use chrono::TimeZone as _;
    let date = ctx.today() + chrono::Duration::days(day);
    let naive = date.and_hms_opt(u32::from(minutes) / 60, u32::from(minutes) % 60, 0)?;
    ctx.tz
        .from_local_datetime(&naive)
        .earliest()
        .map(|l| l.with_timezone(&Utc))
}

/// Whether a new task could start at `slot`: the time exists that day, and no placed
/// task is under way then (one that ends at that minute is over).
pub fn free_at(placements: &[Placement], ctx: &LocalContext, slot: Slot) -> bool {
    drop_start(ctx, slot.day, slot.minutes).is_some_and(|instant| {
        !placements
            .iter()
            .any(|placement| (placement.start..placement.end).contains(&instant))
    })
}

/// What the capture box gets pre-filled with on a long-press of an empty hour:
/// the slot as a date phrase Jev already understands, plus a trailing space.
pub fn hour_prefill_text(ctx: &LocalContext, day: i64, minutes: u16) -> String {
    let date = ctx.today() + chrono::Duration::days(day);
    let day_phrase = match day {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        _ => date.format("%A").to_string().to_lowercase(),
    };
    format!(
        "{day_phrase} {:02}:{:02} ",
        u32::from(minutes) / 60,
        u32::from(minutes) % 60
    )
}

/// A slip's height once its task goes from `from` to `to` minutes long, from `height` px.
pub fn resized_px(height: f32, from: u16, to: u16) -> f32 {
    (height + (f32::from(to) - f32::from(from)) / 60.0 * HOUR_PX).max(0.0)
}

/// What an occurrence row's id is named from: `"{series_id}:{date}"`, the
/// original local date of the expansion. `None` for a plain task id, whose
/// move is a normal task PATCH.
pub fn occurrence_of(id: &TaskId) -> Option<(String, chrono::NaiveDate)> {
    let (series_id, date) = id.0.rsplit_once(':')?;
    let date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some((series_id.to_owned(), date))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::SHORT_SLIP_PX;
    use chrono::TimeZone;

    fn zone(name: &str, now: DateTime<Utc>) -> LocalContext {
        LocalContext {
            now,
            tz: name.parse().unwrap(),
        }
    }

    fn at_utc(year: u32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year as i32, month, day, hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn the_line_sits_where_the_clock_is_in_its_hour() {
        // 09:53 with the grid starting at midnight: nine full hours and 53 minutes down.
        let top = now_line_top(9 * 60 + 53, 0, HOUR_PX);
        assert!(
            (top - (9.0 * HOUR_PX + 53.0 / 60.0 * HOUR_PX)).abs() < 0.01,
            "{top}"
        );
    }

    #[test]
    fn the_line_counts_from_the_first_row_not_from_midnight() {
        // Grid starts at 08:00; 09:00 is one hour down.
        assert_eq!(now_line_top(9 * 60, 8, HOUR_PX), HOUR_PX);
        assert_eq!(
            now_line_top(7 * 60, 8, HOUR_PX),
            0.0,
            "before the grid, the line sits at the top"
        );
    }

    #[test]
    fn a_placement_lands_on_its_own_local_day() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        // 23:30 UTC on 12 Oct is 19:30 on 12 Oct in New York: today.
        assert_eq!(day_index(at_utc(2026, 10, 12, 23, 30), &ny), 0);
        let berlin = zone("Europe/Berlin", at_utc(2026, 10, 12, 12, 0));
        // The same instant is 01:30 on 13 Oct in Berlin: tomorrow.
        assert_eq!(day_index(at_utc(2026, 10, 12, 23, 30), &berlin), 1);
    }

    #[test]
    fn an_hour_is_free_unless_a_placed_task_is_under_way_then() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        // A seeded generator, so the run is the same every time.
        let mut seed = 7_u64;
        let mut next = |below: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % below
        };
        let slot = |day: i64, minutes: u16| Slot { day, minutes };
        for _ in 0..200 {
            let placements: Vec<Placement> = (0..next(5))
                .map(|n| {
                    let (day, minutes) = (next(4) as i64, (8 * 60 + next(36) * 15) as u16);
                    let start = drop_start(&ny, day, minutes).unwrap();
                    Placement {
                        task: todo_core::TaskId(format!("t{n}")),
                        start,
                        end: start + chrono::Duration::minutes(15 * (1 + next(8) as i64)),
                    }
                })
                .collect();
            for day in 0..4 {
                for minutes in (8 * 60..20 * 60).step_by(15) {
                    let instant = drop_start(&ny, day, minutes).unwrap();
                    let under_way = placements
                        .iter()
                        .any(|placement| placement.start <= instant && instant < placement.end);
                    assert_eq!(
                        free_at(&placements, &ny, slot(day, minutes)),
                        !under_way,
                        "day {day} at {minutes}: {placements:?}"
                    );
                }
            }
        }
        // With nothing placed every hour is free, but never a time the clock skips.
        assert!(free_at(&[], &ny, slot(0, 9 * 60)));
        let spring = zone("America/New_York", at_utc(2026, 3, 5, 12, 0));
        assert!(!free_at(&[], &spring, slot(3, 2 * 60 + 30)));
    }

    #[test]
    fn each_placement_appears_only_on_its_day() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        let at = |day: u32, hour: u32| Placement {
            task: todo_core::TaskId(format!("t{day}{hour}")),
            start: at_utc(2026, 10, day, hour, 0),
            end: at_utc(2026, 10, day, hour + 1, 0),
        };
        let placements = vec![at(12, 14), at(13, 9), at(14, 16), at(12, 18)];
        let ids = |day| -> Vec<String> {
            placements_on_day(&placements, day, &ny)
                .into_iter()
                .map(|placement| placement.task.0)
                .collect()
        };
        assert_eq!(ids(0), vec!["t1214", "t1218"]);
        assert_eq!(ids(1), vec!["t139"]);
        assert_eq!(ids(2), vec!["t1416"]);
        assert!(ids(3).is_empty());
    }

    #[test]
    fn a_day_is_counted_in_the_users_zone_not_utc() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        // 02:00 UTC on 13 Oct is still 22:00 on 12 Oct in New York.
        assert_eq!(day_index(at_utc(2026, 10, 13, 2, 0), &ny), 0);
    }

    #[test]
    fn a_placement_either_side_of_the_berlin_clock_change_keeps_its_day() {
        // Europe/Berlin goes from UTC+2 to UTC+1 at 01:00 UTC on 25 Oct 2026. A single
        // offset read on 19 Oct would put 22:30 UTC on 25 Oct on 26 Oct.
        let berlin = zone("Europe/Berlin", at_utc(2026, 10, 19, 12, 0));
        // 22:30 UTC on 25 Oct is 23:30 on 25 Oct in Berlin, after the change.
        assert_eq!(day_index(at_utc(2026, 10, 25, 22, 30), &berlin), 6);
        // 21:30 UTC on 24 Oct is 23:30 on 24 Oct in Berlin, before it.
        assert_eq!(day_index(at_utc(2026, 10, 24, 21, 30), &berlin), 5);
    }

    #[test]
    fn a_refused_stretch_puts_the_slip_back_to_the_height_it_had() {
        use crate::placing::{Stretch, stretched};
        let quarter = HOUR_PX / 4.0;
        for minutes in (15..=240).step_by(15) {
            let height = f32::from(minutes) / 60.0 * HOUR_PX;
            for stretch in [Stretch::Longer, Stretch::Shorter] {
                let to = stretched(minutes, stretch);
                let grown = resized_px(height, minutes, to);
                // What the bubble's stretch draws: a quarter hour, or nothing at a limit.
                let moved = (grown - height).abs();
                assert!(
                    moved == 0.0 || (moved - quarter).abs() < 1e-3,
                    "{minutes} {stretch:?}"
                );
                // And taking it back lands where it started.
                let back = resized_px(grown, to, minutes);
                assert!(
                    (back - height).abs() < 1e-3,
                    "{minutes} {stretch:?}: {back} vs {height}"
                );
            }
        }
    }

    #[test]
    fn the_planner_opens_at_eight_or_now_never_the_night() {
        // The brief: opens at about 08:00 — not 02:00.
        assert_eq!(planner_open_minutes(2 * 60, 0, 24), 8 * 60);
        // Later than eight: the clock wins.
        assert_eq!(planner_open_minutes(12 * 60 + 37, 0, 24), 12 * 60 + 37);
        // A working day that starts later clamps at its own first hour.
        assert_eq!(planner_open_minutes(2 * 60, 9, 18), 9 * 60);
        assert_eq!(planner_open_minutes(10 * 60, 9, 18), 10 * 60);
        // The last full hour is as far down as it opens.
        assert_eq!(planner_open_minutes(23 * 60 + 59, 0, 24), 23 * 60);
    }

    #[test]
    fn the_days_read_as_weekdays_in_the_planner_zone() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        // 2026-10-12 is a Monday; day 0 shows the date, later days the count.
        assert_eq!(day_label(&ny, 0), "Mon 12");
        assert_eq!(day_label(&ny, 1), "Tue 13");
        assert_eq!(day_label(&ny, 4), "Fri 16");
        // The label is drawn in the planner's zone, not UTC: 23:30 UTC is
        // still Sunday the 18th in New York.
        let late = zone("America/New_York", at_utc(2026, 10, 18, 23, 30));
        assert_eq!(day_label(&late, 0), "Sun 18");
    }

    #[test]
    fn a_drop_snaps_to_the_quarter_hour_and_never_lands_at_the_day_end() {
        // 1.5 px past the 08:00 row = dead on 08:00.
        assert_eq!(drop_minute(1.5, 8, 18, HOUR_PX), 8 * 60);
        // 16 px of 64 is a quarter hour.
        assert_eq!(drop_minute(16.0, 8, 18, HOUR_PX), 8 * 60 + 15);
        // Half of 64px is 30 minutes.
        assert_eq!(drop_minute(32.0, 8, 18, HOUR_PX), 8 * 60 + 30);
        assert_eq!(
            drop_minute(49.0, 8, 18, HOUR_PX),
            8 * 60 + 45,
            "49px is 45.9min → the nearest quarter hour"
        );
        // Far past the bottom: pinned just inside the planner's last slot.
        assert_eq!(drop_minute(10_000.0, 8, 18, HOUR_PX), 17 * 60 + 45);
        assert_eq!(drop_minute(-5.0, 8, 18, HOUR_PX), 8 * 60);
        // An end at midnight never snaps to 24:00 as a start.
        assert_eq!(drop_minute(10_000.0, 8, 24, HOUR_PX), 23 * 60 + 45);
    }

    #[test]
    fn the_paper_slides_faster_the_nearer_the_held_slip_gets_to_an_edge() {
        // Nothing at all outside the edge band.
        assert_eq!(edge_nudge(EDGE_ZONE_PX + 10.0), 0.0);
        assert_eq!(
            edge_nudge(EDGE_ZONE_PX),
            0.0,
            "the band's edge counts as out"
        );
        // Halfway into the band: half the page's fastest nudge.
        assert!((edge_nudge(EDGE_ZONE_PX / 2.0) - EDGE_MAX_PX / 2.0).abs() < 0.01,);
        // At the edge itself, the loudest nudge.
        assert!((edge_nudge(0.0) - EDGE_MAX_PX).abs() < 0.01);
    }

    #[test]
    fn a_drop_starts_at_the_wall_clock_time_in_the_planner_zone() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        // Day 0, 09:30 in New York = 13:30 UTC.
        assert_eq!(
            drop_start(&ny, 0, 9 * 60 + 30),
            Some(at_utc(2026, 10, 12, 13, 30))
        );
        // Day 2, 23:30 → still the same local day, two days ahead.
        assert_eq!(
            drop_start(&ny, 2, 23 * 60 + 30),
            Some(at_utc(2026, 10, 15, 3, 30))
        );
    }

    #[test]
    fn an_occurrence_names_its_series_and_original_date() {
        // A row the planner expanded from a weekly rule.
        assert_eq!(
            occurrence_of(&TaskId("standup:2026-10-13".into())),
            Some(("standup".to_owned(), "2026-10-13".parse().unwrap()))
        );
        // A series id may itself carry a colon: split at the LAST one.
        assert_eq!(
            occurrence_of(&TaskId("team:sync:2026-10-13".into())),
            Some(("team:sync".to_owned(), "2026-10-13".parse().unwrap()))
        );
        // Plain task ids are not occurrences.
        assert_eq!(occurrence_of(&TaskId("te86966".into())), None);
        // ... and neither is anything whose trailing part is not a date.
        assert_eq!(occurrence_of(&TaskId("standup:next-week".into())), None);
    }

    #[test]
    fn a_time_the_clock_skips_cannot_start() {
        // America/New_York jumps 02:00→03:00 on 2026-03-08; 02:30 does not exist.
        let ny = zone("America/New_York", at_utc(2026, 3, 5, 12, 0));
        // day 3 from 2026-03-05 = 2026-03-08 itself.
        assert_eq!(drop_start(&ny, 3, 2 * 60 + 30), None);
        // Same day, 03:30 exists after the jump.
        assert!(drop_start(&ny, 3, 3 * 60 + 30).is_some());
    }

    #[test]
    fn an_hour_long_press_prefills_a_date_phrase_jev_parses() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        let at = |day: i64, minutes: u16| hour_prefill_text(&ny, day, minutes);
        assert_eq!(at(0, 18 * 60), "today 18:00 ");
        assert_eq!(at(1, 9 * 60 + 30), "tomorrow 09:30 ");
        // 2026-10-16 is a Friday.
        assert_eq!(at(4, 10 * 60), "friday 10:00 ");
    }

    #[test]
    fn a_moved_or_replanned_placement_changes_its_key_so_the_block_is_redrawn() {
        // The same slip, moved half an hour later inside its own day.
        let first = Placement {
            task: TaskId("t".into()),
            start: at_utc(2026, 10, 12, 14, 0),
            end: at_utc(2026, 10, 12, 14, 30),
        };
        let moved = Placement {
            start: at_utc(2026, 10, 12, 14, 30),
            end: at_utc(2026, 10, 12, 15, 0),
            ..first.clone()
        };
        assert_ne!(placement_key(&first), placement_key(&moved));
        // An unchanged placement keeps its key, so the row is not rebuilt for nothing.
        assert_eq!(placement_key(&first), placement_key(&first.clone()));
    }

    #[test]
    fn a_thirty_minute_slip_is_short_an_hour_long_one_is_not() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        let settings = Settings::default();
        let slip = |start: DateTime<Utc>, end: DateTime<Utc>| Placement {
            task: TaskId("t".into()),
            start,
            end,
        };
        // 14:00 UTC is 10:00 in New York: a half hour, then a full hour.
        let half = slip(at_utc(2026, 10, 12, 14, 0), at_utc(2026, 10, 12, 14, 30));
        let whole = slip(at_utc(2026, 10, 12, 14, 0), at_utc(2026, 10, 12, 15, 0));
        assert_eq!(
            placement_box(half.start, half.end, &ny, &settings).1,
            HOUR_PX / 2.0
        );
        assert!(
            placement_box(half.start, half.end, &ny, &settings).1 < SHORT_SLIP_PX,
            "half an hour gets its words beside the slip"
        );
        assert!(
            placement_box(whole.start, whole.end, &ny, &settings).1 >= SHORT_SLIP_PX,
            "an hour holds its own words"
        );
    }

    #[test]
    fn placement_style_0930_start_8am_64px() {
        // 09:30 start => 90 min after 8:00 => 90/60*64 = 96; 45 min => 48
        let (top, height) = placement_style(9 * 60 + 30, 9 * 60 + 75, 8, HOUR_PX, 18);
        assert!((top - 96.0).abs() < 0.01, "top={}, expected 96", top);
        assert!(
            (height - 48.0).abs() < 0.01,
            "height={}, expected 48",
            height
        );
    }

    #[test]
    fn placement_style_clipped_at_end() {
        let (top, height) = placement_style(17 * 60, 19 * 60, 8, HOUR_PX, 18);
        assert!((top - 9.0 * HOUR_PX).abs() < 0.01);
        assert!((height - 1.0 * HOUR_PX).abs() < 0.01);
    }

    #[test]
    fn placement_style_midnight_crossing_clipped_to_end() {
        // Task starts at 23:30 and ends at 01:30 next day; planner ends 24:00 (18 not used here, planner 8-24)
        let (top, height) = placement_style(23 * 60 + 30, 25 * 60 + 30, 8, HOUR_PX, 24);
        assert!((top - 15.5 * HOUR_PX).abs() < 0.01); // 23:30 = 15h30 after 8:00
        assert!((height - 0.5 * HOUR_PX).abs() < 0.01); // 30 min until 24:00
    }

    #[test]
    fn planner_days_cross_month_and_year_end() {
        let ny = zone("America/New_York", at_utc(2026, 12, 31, 12, 0));
        assert_eq!(day_index(at_utc(2026, 12, 31, 23, 0), &ny), 0);
        assert_eq!(day_index(at_utc(2027, 1, 1, 1, 0), &ny), 0);
        assert_eq!(day_index(at_utc(2027, 1, 1, 5, 0), &ny), 1);
    }

    #[test]
    fn placement_at_exact_planner_start_hour() {
        let (top, height) = placement_style(8 * 60, 9 * 60, 8, HOUR_PX, 18);
        assert!((top - 0.0).abs() < 0.01, "start at 08:00 = 0, got {top}");
        assert!((height - HOUR_PX).abs() < 0.01, "1h = 64, got {height}");
    }

    #[test]
    fn one_minute_placement() {
        let (top, height) = placement_style(10 * 60, 10 * 60 + 1, 8, HOUR_PX, 18);
        assert!(
            (height - (1.0 / 60.0 * HOUR_PX)).abs() < 0.01,
            "1 min = 1.07, got {height}"
        );
        assert!((top - 2.0 * HOUR_PX).abs() < 0.01, "10:00 = 2h, got {top}");
    }

    #[test]
    fn six_hundred_minute_placement() {
        let (top, height) = placement_style(8 * 60, 18 * 60, 8, HOUR_PX, 18);
        assert!((top - 0.0).abs() < 0.01);
        assert!(
            (height - 10.0 * HOUR_PX).abs() < 0.01,
            "10h = 640, got {height}"
        );
    }
}
