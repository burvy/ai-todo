//! Clean writing schedule for the Rambo pen — pure Rust, unit-tested.
//! Design token: WRITE_MS = 600 ms per title line; linger 2000 ms after last write.

/// Schedule entry: when a title starts/ends writing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteSlot {
    pub title: String,
    pub start_ms: u32,
    pub duration_ms: u32,
}

/// Build slots from a list of new task titles.
/// Pure: no DOM, no clock, just arithmetic.
pub fn schedule_titles(titles: &[String], write_ms: u32, offset_ms: u32) -> Vec<WriteSlot> {
    let mut slots = Vec::with_capacity(titles.len());
    for (i, t) in titles.iter().enumerate() {
        let start = offset_ms + (i as u32) * write_ms;
        slots.push(WriteSlot {
            title: t.clone(),
            start_ms: start,
            duration_ms: write_ms,
        });
    }
    slots
}

/// Total write time for a batch.
pub fn total_write_ms(titles_len: usize, write_ms: u32) -> u32 {
    if titles_len == 0 {
        0
    } else {
        (titles_len as u32 - 1) * write_ms + write_ms + 2000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_titles_sequential() {
        let s = schedule_titles(&["A".into(), "B".into()], 600, 0);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].start_ms, 0);
        assert_eq!(s[1].start_ms, 600);
    }

    #[test]
    fn linger_after_last() {
        assert_eq!(total_write_ms(2, 600), 600 + 600 + 2000);
    }

    #[test]
    fn reduced_motion_ignored_at_schedule_level() {
        // The schedule is arithmetic; reduced-motion is a DOM/display decision
        // handled in the view layer, not here.
        let s = schedule_titles(&["X".into()], 600, 100);
        assert_eq!(s[0].start_ms, 100);
    }
}
