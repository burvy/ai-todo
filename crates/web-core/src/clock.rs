//! How times are written on the page (DESIGN-DIRECTION "12-hour clock is the
//! default, 24-hour is a gear-menu choice"): one helper, used by the planner's
//! hour gutter, the task rows, the planner slips and the edit dialog.

use serde::{Deserialize, Serialize};

/// The clock the paper writes its times in. The 12-hour one is the default;
/// the gear menu is where the choice lives and is remembered per device.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum Clock {
    #[default]
    Twelve,
    TwentyFour,
}

impl Clock {
    pub fn label(self) -> &'static str {
        match self {
            Clock::Twelve => "12-hour",
            Clock::TwentyFour => "24-hour",
        }
    }

    /// A whole hour and fractional minutes as minutes since midnight.
    pub fn twelve_hour(self) -> bool {
        matches!(self, Clock::Twelve)
    }
}

/// A minute of the day as the clock writes it: `9 am`, `12 pm`, `2:30 pm`,
/// or `09:00`, `14:30`, on the 24-hour paper. Midnight is `12 am`, noon
/// `12 pm`, as the calendar books say; minutes are always shown.
pub fn format_time(minutes_in_day: u16, clock: Clock) -> String {
    let minutes_in_day = minutes_in_day % (24 * 60);
    if clock.twelve_hour() {
        let hour24 = minutes_in_day / 60;
        let minute = minutes_in_day % 60;
        let mut hour12 = hour24 % 12;
        if hour12 == 0 {
            hour12 = 12;
        }
        let half = if hour24 < 12 { "am" } else { "pm" };
        if minute == 0 {
            format!("{hour12} {half}")
        } else {
            format!("{hour12}:{minute:02} {half}")
        }
    } else {
        format!("{:02}:{:02}", minutes_in_day / 60, minutes_in_day % 60)
    }
}

/// A day name and a time together, as the rows and the slips show it:
/// `Mon 9 am`, `Mon 14:00`.
pub fn format_day_time(weekday: &str, minutes_in_day: u16, clock: Clock) -> String {
    format!("{weekday} {}", format_time(minutes_in_day, clock))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_twelve_hour_clock_names_midnight_and_noon_by_the_book() {
        assert_eq!(format_time(0, Clock::Twelve), "12 am");
        assert_eq!(format_time(12 * 60, Clock::Twelve), "12 pm");
        assert_eq!(format_time(23 * 60 + 59, Clock::Twelve), "11:59 pm");
        assert_eq!(format_time(24 * 60, Clock::Twelve), "12 am");
    }

    #[test]
    fn whole_hours_drop_the_minutes_half_hours_keep_them() {
        assert_eq!(format_time(8 * 60, Clock::Twelve), "8 am");
        assert_eq!(format_time(9 * 60 + 30, Clock::Twelve), "9:30 am");
        assert_eq!(format_time(13 * 60, Clock::Twelve), "1 pm");
        assert_eq!(format_time(17 * 60 + 30, Clock::Twelve), "5:30 pm");
    }

    #[test]
    fn the_twenty_four_hour_clock_pads_the_hour() {
        assert_eq!(format_time(0, Clock::TwentyFour), "00:00");
        assert_eq!(format_time(8 * 60, Clock::TwentyFour), "08:00");
        assert_eq!(format_time(9 * 60 + 30, Clock::TwentyFour), "09:30");
        assert_eq!(format_time(17 * 60 + 30, Clock::TwentyFour), "17:30");
    }

    #[test]
    fn a_day_and_a_time_read_as_one_line() {
        assert_eq!(format_day_time("Mon", 9 * 60, Clock::Twelve), "Mon 9 am");
        assert_eq!(
            format_day_time("Tue", 14 * 60, Clock::TwentyFour),
            "Tue 14:00"
        );
    }
}
