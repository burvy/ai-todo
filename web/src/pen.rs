//! Rambo's writing schedule and nib arithmetic.
//!
//! Pure and unit-tested: no DOM, no clock, no timers. The view layer reads
//! these numbers and animates the pen; nothing here moves anything.
//!
//! A fresh row's title appears glyph by glyph under the nib: each glyph's
//! stroke time comes from its own measured width (wide glyphs take longer,
//! word gaps are pen lifts), so the nib's pace matches the text it writes.
//! The pen lingers [`LINGER_MS`] at the capture caret after typing stops
//! before flying home.

/// Average stroke time per glyph; sets the overall pace of a line.
pub const PER_GLYPH_MS: u32 = 90;
/// Shortest and longest a single title may take to write.
pub const MIN_WRITE_MS: u32 = 500;
pub const MAX_WRITE_MS: u32 = 2400;
/// Added width (in the same units as glyph widths) for the pen lift over a
/// space: the nib travels the gap and pauses a beat before the next word.
pub const WORD_GAP_W: f64 = 10.0;
/// Even the narrowest glyph costs this much of the schedule (an `i` is
/// quick, not free), so a run of thin glyphs never flashes by.
pub const MIN_GLYPH_W: f64 = 6.0;

/// Quiet time at the capture box before the pen flies home.
pub const LINGER_MS: u32 = 2000;
/// Flight time between two spots (caret, row, home).
pub const FLY_MS: u32 = 450;
/// The pen's drawn size in CSS pixels; must mirror `.rambo img` in theme.css.
/// Scaled up 1.25x for the redesign.
pub const PEN_W: f64 = 37.5;
pub const PEN_H: f64 = 65.0;
/// The scale the pen writes at (shrunk so it does not cover the words).
pub const WRITE_SCALE: f64 = 0.78;

/// How long a title with `glyphs` glyphs takes to write: pace by length,
/// clamped so short titles still read as writing and long ones finish.
pub fn write_total_ms(glyphs: usize) -> u32 {
    (glyphs as u32 * PER_GLYPH_MS).clamp(MIN_WRITE_MS, MAX_WRITE_MS)
}

/// The stroke time of every glyph in `text`, in milliseconds.
///
/// `width_of` supplies each glyph's advance width (the DOM measures this
/// with the title's real font; tests inject numbers). Time is shared in
/// proportion to width — a wide `m` takes proportionally longer than an `i`
/// — and every space also carries the [`WORD_GAP_W`] pen lift. The last
/// glyph absorbs rounding so the durations sum to exactly
/// [`write_total_ms`].
pub fn glyph_durations(text: &str, width_of: impl Fn(char) -> f64) -> Vec<u32> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let weights: Vec<f64> = chars
        .iter()
        .map(|&c| {
            let width = width_of(c).max(MIN_GLYPH_W);
            if c.is_whitespace() {
                width + WORD_GAP_W
            } else {
                width
            }
        })
        .collect();
    let total_ms = f64::from(write_total_ms(chars.len()));
    let weight_sum: f64 = weights.iter().sum();
    let mut durations: Vec<u32> = weights
        .iter()
        .map(|w| (total_ms * w / weight_sum).round() as u32)
        .collect();
    // Hand the rounding drift (either sign) to the last stroke so the line
    // finishes exactly on time.
    let drift = (total_ms as i64) - i64::from(durations.iter().sum::<u32>());
    let last = durations.len() - 1;
    durations[last] = (i64::from(durations[last]) + drift).max(0) as u32;
    durations
}

/// How many glyphs are on the paper `elapsed_ms` into the stroke schedule:
/// a glyph appears once its stroke has fully passed.
pub fn visible_prefix(durations: &[u32], elapsed_ms: u32) -> usize {
    let mut spent = 0_u32;
    for (index, duration) in durations.iter().enumerate() {
        match spent.saturating_add(*duration) {
            done if done <= elapsed_ms => spent = done,
            _ => return index,
        }
    }
    durations.len()
}

/// Where the pen's centre sits so its nib lands on `(caret_x, caret_y)`:
/// the nib hangs half a scaled pen below the centre while writing.
pub fn writing_pen_center(caret_x: f64, caret_y: f64) -> (f64, f64) {
    (caret_x, caret_y - (PEN_H / 2.0) * WRITE_SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_total_scales_with_length_and_clamps() {
        assert_eq!(write_total_ms(0), MIN_WRITE_MS);
        assert_eq!(write_total_ms(3), MIN_WRITE_MS);
        assert_eq!(write_total_ms(10), 900);
        assert_eq!(write_total_ms(100), MAX_WRITE_MS);
    }

    #[test]
    fn durations_sum_to_the_line_total() {
        for text in [
            "a",
            "hi",
            "Buy oat milk",
            "Something Much Longer To Write Out",
        ] {
            let durations = glyph_durations(text, |c| if c.is_whitespace() { 6.0 } else { 10.0 });
            assert_eq!(
                durations.iter().sum::<u32>(),
                write_total_ms(text.chars().count()),
                "{text}"
            );
        }
    }

    #[test]
    fn empty_text_writes_nothing() {
        assert!(glyph_durations("", |_| 5.0).is_empty());
    }

    #[test]
    fn wide_glyphs_take_longer_than_thin_ones() {
        let text = "mim";
        let durations = glyph_durations(text, |c| if c == 'm' { 20.0 } else { 4.0 });
        let [_, mid, _] = durations[..] else {
            panic!("three glyphs");
        };
        assert!(mid < durations[0], "thin i must be quicker: {durations:?}");
        assert!(durations[0].abs_diff(durations[2]) < 2, "{durations:?}");
    }

    #[test]
    fn spaces_carry_the_pen_lift() {
        // Same measured width, but the space also pays WORD_GAP_W.
        // Every glyph measures the same width; the space still costs more time.
        let with_space = glyph_durations("a a", |_| 6.0);
        let tight = glyph_durations("aia", |_| 6.0);
        assert!(
            with_space[1] > tight[1] + 5,
            "space must pause: {:?} vs {:?}",
            with_space,
            tight
        );
    }

    #[test]
    fn thin_glyphs_never_flash_by() {
        // All-zero widths still give every glyph a real stroke.
        let durations = glyph_durations("illili", |_| 0.0);
        assert!(durations.iter().all(|&d| d > 0), "{durations:?}");
    }

    #[test]
    fn single_glyph_takes_the_whole_window() {
        let durations = glyph_durations("W", |_| 12.0);
        assert_eq!(durations, vec![write_total_ms(1)]);
    }

    #[test]
    fn visible_prefix_grows_on_schedule() {
        let durations = [100_u32, 100, 100];
        assert_eq!(visible_prefix(&durations, 0), 0);
        assert_eq!(visible_prefix(&durations, 99), 0);
        assert_eq!(visible_prefix(&durations, 100), 1);
        assert_eq!(visible_prefix(&durations, 250), 2);
        assert_eq!(visible_prefix(&durations, 300), 3);
        assert_eq!(visible_prefix(&durations, 10_000), 3);
    }

    #[test]
    fn pen_centre_hangs_above_the_caret() {
        let (x, y) = writing_pen_center(200.0, 100.0);
        assert_eq!(x, 200.0);
        assert!((y - (100.0 - (PEN_H / 2.0) * WRITE_SCALE)).abs() < 1e-9);
    }
}
