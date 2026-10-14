//! How date phrases are read. See `docs/TUNING.md`. The day-only deadline and the
//! working day are in `todo_core::tuning`.

use chrono::NaiveTime;

/// Said on a Friday, "Friday" means today until this local time and next week after
/// it. Safe range 12:00 to 21:00.
pub const SAME_DAY_CUTOFF: NaiveTime = NaiveTime::from_hms_opt(17, 0, 0).expect("17:00 is valid");
