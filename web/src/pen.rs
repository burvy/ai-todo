//! Rambo's writing schedule and nib arithmetic.
//!
//! Pure and unit-tested: no DOM, no clock, no timers. The view layer reads
//! these numbers and animates the pen; nothing here moves anything.
//!
//! The pen writes a fresh row's title left to right over [`WRITE_MS`], one row
//! after another, and lingers [`LINGER_MS`] at the capture caret after typing
//! stops before flying home — the v1 behaviour, rebuilt.

/// How long writing one title line takes.
pub const WRITE_MS: u32 = 600;
/// Quiet time at the capture box before the pen flies home.
pub const LINGER_MS: u32 = 2000;
/// Flight time between two spots (caret, row, home).
pub const FLY_MS: u32 = 450;
/// The pen's drawn size in CSS pixels; must mirror `.pen-rig-pen` in index.html.
pub const PEN_W: f64 = 30.0;
pub const PEN_H: f64 = 52.0;
/// The scale the pen writes at (shrunk so it does not cover the words).
pub const WRITE_SCALE: f64 = 0.78;

/// One title's writing window, relative to the start of a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteSlot {
    pub start_ms: u32,
    pub duration_ms: u32,
}

/// `count` titles written back to back: each starts `write_ms` after the last.
pub fn schedule_slots(count: usize, write_ms: u32, offset_ms: u32) -> Vec<WriteSlot> {
    (0..count as u32)
        .map(|i| WriteSlot {
            start_ms: offset_ms + i * write_ms,
            duration_ms: write_ms,
        })
        .collect()
}

/// Milliseconds from the first nib touch until the last line is finished.
pub fn total_write_ms(count: usize, write_ms: u32) -> u32 {
    (count as u32).saturating_mul(write_ms)
}

/// How much of the line the nib has covered, clamped to 0..=1.
pub fn nib_progress(elapsed_ms: u32, duration_ms: u32) -> f32 {
    if duration_ms == 0 {
        return 1.0;
    }
    (elapsed_ms as f32 / duration_ms as f32).clamp(0.0, 1.0)
}

/// The nib's x along a line: `left` when the write starts, `right` at the end.
pub fn nib_x(left: f64, right: f64, progress: f32) -> f64 {
    left + (right - left) * f64::from(progress)
}

/// Where the pen's centre sits so its nib lands on `(caret_x, caret_y)`:
/// the nib hangs half a scaled pen below the centre while writing.
pub fn writing_pen_center(caret_x: f64, caret_y: f64) -> (f64, f64) {
    (caret_x, caret_y - (PEN_H / 2.0) * WRITE_SCALE)
}

/// The pen's centre so its nib (bottom point) touches `point_y` at `scale`.
pub fn nib_center(point_x: f64, point_y: f64, scale: f64) -> (f64, f64) {
    (point_x, point_y - (PEN_H / 2.0) * scale)
}

/// Clamp the nib so the pen never walks past the box's right edge
/// (v1 kept a 6px margin and one pen width of room).
pub fn clamp_nib_x(x: f64, box_left: f64, box_right: f64) -> f64 {
    let max = box_right - 6.0 - PEN_W * WRITE_SCALE;
    x.clamp(box_left, max.max(box_left))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_run_back_to_back() {
        let slots = schedule_slots(3, 600, 0);
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0], WriteSlot { start_ms: 0, duration_ms: 600 });
        assert_eq!(slots[1], WriteSlot { start_ms: 600, duration_ms: 600 });
        assert_eq!(slots[2], WriteSlot { start_ms: 1200, duration_ms: 600 });
    }

    #[test]
    fn slots_honour_offset() {
        let slots = schedule_slots(2, 600, 450);
        assert_eq!(slots[0].start_ms, 450);
        assert_eq!(slots[1].start_ms, 1050);
    }

    #[test]
    fn empty_batch_writes_nothing() {
        assert!(schedule_slots(0, 600, 0).is_empty());
        assert_eq!(total_write_ms(0, 600), 0);
    }

    #[test]
    fn total_is_the_last_line_finished() {
        assert_eq!(total_write_ms(3, 600), 1800);
    }

    #[test]
    fn nib_progress_clamps() {
        assert_eq!(nib_progress(0, 600), 0.0);
        assert!((nib_progress(300, 600) - 0.5).abs() < 1e-6);
        assert_eq!(nib_progress(900, 600), 1.0);
        assert_eq!(nib_progress(10, 0), 1.0);
    }

    #[test]
    fn nib_sweeps_left_to_right() {
        assert_eq!(nib_x(100.0, 300.0, 0.0), 100.0);
        assert_eq!(nib_x(100.0, 300.0, 1.0), 300.0);
        assert_eq!(nib_x(100.0, 300.0, 0.5), 200.0);
    }

    #[test]
    fn pen_centre_hangs_above_the_caret() {
        let (x, y) = writing_pen_center(200.0, 100.0);
        assert_eq!(x, 200.0);
        assert!((y - (100.0 - (PEN_H / 2.0) * WRITE_SCALE)).abs() < 1e-9);
    }

    #[test]
    fn nib_stays_inside_the_box() {
        // right edge guard: pen width at write scale plus a 6px margin
        assert_eq!(clamp_nib_x(500.0, 0.0, 400.0), 400.0 - 6.0 - PEN_W * WRITE_SCALE);
        assert_eq!(clamp_nib_x(-50.0, 0.0, 400.0), 0.0);
        assert_eq!(clamp_nib_x(200.0, 0.0, 400.0), 200.0);
    }
}
