//! Rambo's writing schedule and nib arithmetic.
//!
//! Pure and unit-tested: no DOM, no clock, no timers. The view layer reads
//! these numbers and animates the pen; nothing here moves anything.
//!
//! A fresh row's title is inked under the nib: each glyph is given time in
//! proportion to the width it occupies on the page, and every word gap also
//! pays for the pen's lift. The pen lingers [`LINGER_MS`] at the capture
//! caret after typing stops before flying home.

/// Average stroke time per glyph; sets the overall pace of a line.
pub const PER_GLYPH_MS: u32 = 90;
/// Shortest and longest a single title may take to write.
pub const MIN_WRITE_MS: u32 = 500;
pub const MAX_WRITE_MS: u32 = 2400;
/// Extra width, in the same units as glyph widths, that the pen lift over a
/// space costs: the nib hops the gap and pauses before the next word.
pub const WORD_GAP_W: f64 = 10.0;
/// Even the narrowest glyph costs this much of the schedule (an `i` is
/// quick, not free), so a run of thin glyphs never flashes by.
pub const MIN_GLYPH_W: f64 = 6.0;

/// Quiet time at the capture box before the pen flies home.
pub const LINGER_MS: u32 = 2000;
/// Flight time between two spots (caret, row, home).
pub const FLY_MS: u32 = 450;
/// The pen's drawn height in CSS pixels; must mirror `.rambo img` in
/// theme.css. Scaled up 1.25x for the redesign.
pub const PEN_H: f64 = 65.0;
/// The scale the pen writes at (shrunk so it does not cover the words).
pub const WRITE_SCALE: f64 = 0.78;
/// How often the ink and the nib move while a title is written: one frame,
/// so the ink never jumps a letter and the pen never looks stalled.
pub const INK_TICK_MS: u32 = 16;

/// A title crossed out takes at least [`SCRIBBLE_BASE_MS`], and every written
/// glyph adds [`SCRIBBLE_MS_PER_GLYPH_MS`] up to [`MAX_SCRIBBLE_MS`] — long
/// enough to read as crossing out, short enough not to hold up a delete.
pub const SCRIBBLE_BASE_MS: u32 = 420;
pub const SCRIBBLE_MS_PER_GLYPH_MS: u32 = 22;
pub const MAX_SCRIBBLE_MS: u32 = 1100;

/// The tool Rambo writes with when the gear menu has not chosen one: the
/// slug [`settings::Pen::slug`](crate::settings::Pen::slug) uses.
pub const DEFAULT_PEN: &str = "fountain-pen";

/// One glyph's place in a line's writing schedule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    /// Milliseconds from the start of the line until this glyph's stroke ends.
    pub ends_ms: u32,
    /// How wide the glyph is on the page, so the nib's speed is its pace.
    pub width: f64,
}

/// How long a title of `glyphs` glyphs takes to write: paced by length and
/// clamped so short titles still read as writing and long ones finish.
pub fn write_total_ms(glyphs: usize) -> u32 {
    (glyphs as u32 * PER_GLYPH_MS).clamp(MIN_WRITE_MS, MAX_WRITE_MS)
}

/// The whole line's schedule: every glyph's width and the moment its stroke
/// ends, in reading order.
///
/// `width_of` supplies each glyph's advance width — the DOM measures this
/// with the title's real font, tests inject numbers. Time is shared in
/// proportion to width, so a wide `m` takes proportionally longer than an `i`,
/// and a space pays the [`WORD_GAP_W`] pen lift on top of its own width. The
/// total is always [`write_total_ms`]: the last glyph absorbs the rounding
/// drift so the line finishes exactly on time.
pub fn schedule(text: &str, width_of: impl Fn(char) -> f64) -> Vec<Glyph> {
    let widths: Vec<f64> = text.chars().map(&width_of).collect();
    if widths.is_empty() {
        return Vec::new();
    }
    let weights: Vec<f64> = text
        .chars()
        .zip(&widths)
        .map(|(c, &width)| match c.is_whitespace() {
            true => width.max(MIN_GLYPH_W) + WORD_GAP_W,
            false => width.max(MIN_GLYPH_W),
        })
        .collect();
    let total_ms = write_total_ms(widths.len());
    let weight_sum: f64 = weights.iter().sum();
    // The running total is kept exact and only the ends are rounded, so every
    // glyph gets its own share of the line rather than the rounding of the
    // ones before it.
    let mut running = 0.0_f64;
    let mut ends_ms: Vec<u32> = weights
        .iter()
        .map(|weight| {
            running += f64::from(total_ms) * weight / weight_sum;
            running.round() as u32
        })
        .collect();
    // The last glyph takes whatever the rounding left over, so the line
    // finishes exactly on `total_ms` however each share happened to fall.
    if let Some(last) = ends_ms.last_mut() {
        *last = total_ms;
    }
    widths
        .into_iter()
        .zip(ends_ms)
        .map(|(width, ends_ms)| Glyph { ends_ms, width })
        .collect()
}

/// The line's measured width: where the nib ends up.
pub fn line_width(glyphs: &[Glyph]) -> f64 {
    glyphs.iter().map(|glyph| glyph.width).sum()
}

/// How much of the line is inked `elapsed_ms` in, as a fraction of its
/// measured width.
///
/// The line is laid out in full and only the ink is clipped, so this is both
/// how much of the title has been written and where the nib sits — the nib is
/// always exactly at the edge of the ink. The schedule gives each glyph time
/// in proportion to the width it also occupies, so the edge sweeps the words
/// and pauses over the gaps the pen lifts across.
pub fn inked(glyphs: &[Glyph], elapsed_ms: u32) -> f32 {
    let total_width = line_width(glyphs);
    if total_width <= 0.0 {
        return 1.0;
    }
    // The last glyph to have finished before `elapsed_ms`; `-1` means the nib
    // has not started writing yet.
    let mut done = 0_usize;
    let mut spent = 0_u32;
    for (index, glyph) in glyphs.iter().enumerate() {
        if glyph.ends_ms > elapsed_ms {
            break;
        }
        done = index + 1;
        spent = glyph.ends_ms;
    }
    let written: f64 = glyphs[..done].iter().map(|glyph| glyph.width).sum();
    let in_flight = glyphs[done..]
        .first()
        .map(|next| {
            let stroke = next.ends_ms.saturating_sub(spent).max(1);
            let partial = elapsed_ms.saturating_sub(spent) as f64 / f64::from(stroke);
            glyphs[done].width * partial.clamp(0.0, 1.0)
        })
        .unwrap_or(0.0);
    ((written + in_flight) / total_width) as f32
}

/// Where the pen's centre sits so its nib lands on `(caret_x, caret_y)`:
/// the nib hangs half a scaled pen below the centre while writing.
pub fn writing_pen_center(caret_x: f64, caret_y: f64) -> (f64, f64) {
    (caret_x, caret_y - (PEN_H / 2.0) * WRITE_SCALE)
}

/// The writing baseline inside a line box: leading is shared above and below,
/// so the baseline is the box's middle shifted by the font's own bias towards
/// the descenders.
pub fn baseline_y(top: f64, height: f64, ascent: f64, descent: f64) -> f64 {
    top + (height + (ascent - descent)) / 2.0
}

/// Which ink a row is written in: the tool Rambo is holding, named by the gear
/// menu's `data-pen` hook. Anything unrecognised falls back to the fountain
/// pen, so a new skin still writes instead of writing nothing.
pub fn ink_for_pen(tool: Option<&str>) -> &'static str {
    match tool {
        Some("pencil") => "pencil",
        Some("ballpoint") => "ballpoint",
        Some("quill") => "quill",
        Some("marker") => "marker",
        _ => DEFAULT_PEN,
    }
}

/// How long Rambo takes to cross `title` out. Spaces cost nothing: it is the
/// written glyphs that need crossing.
pub fn scribble_ms(title: &str) -> u32 {
    let glyphs = title.chars().filter(|c| !c.is_whitespace()).count() as u32;
    SCRIBBLE_BASE_MS
        .saturating_add(glyphs.saturating_mul(SCRIBBLE_MS_PER_GLYPH_MS))
        .min(MAX_SCRIBBLE_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every letter the same width: the easy case to reason about.
    fn even(text: &str) -> Vec<Glyph> {
        schedule(text, |c| if c.is_whitespace() { 6.0 } else { 10.0 })
    }

    #[test]
    fn write_total_scales_with_length_and_clamps() {
        assert_eq!(write_total_ms(0), MIN_WRITE_MS);
        assert_eq!(write_total_ms(3), MIN_WRITE_MS);
        assert_eq!(write_total_ms(10), 900);
        assert_eq!(write_total_ms(100), MAX_WRITE_MS);
    }

    #[test]
    fn the_schedule_ends_exactly_on_the_lines_total() {
        for text in [
            "a",
            "hi",
            "Buy oat milk",
            "Something Much Longer To Write Out",
        ] {
            let glyphs = even(text);
            assert_eq!(
                glyphs.last().map(|g| g.ends_ms),
                Some(write_total_ms(text.chars().count())),
                "{text}"
            );
            assert_eq!(glyphs.len(), text.chars().count(), "{text}");
        }
    }

    #[test]
    fn empty_text_writes_nothing() {
        assert!(schedule("", |_| 5.0).is_empty());
        assert_eq!(line_width(&[]), 0.0);
        assert_eq!(inked(&[], 0), 1.0, "no line is a finished line");
    }

    #[test]
    fn wide_glyphs_take_longer_than_thin_ones() {
        let glyphs = schedule("mim", |c| if c == 'm' { 20.0 } else { 4.0 });
        let strokes: Vec<u32> = glyphs
            .iter()
            .scan(0_u32, |spent, glyph| {
                let stroke = glyph.ends_ms - *spent;
                *spent = glyph.ends_ms;
                Some(stroke)
            })
            .collect();
        assert_eq!(strokes[0], strokes[2], "the two m's are one letter");
        // Proportional to width, not a flat share: an `m` five times wider
        // than an `i` takes far more than five times as long... no: at least
        // three times as long, with room for the rounding.
        assert!(
            strokes[0] >= 3 * strokes[1],
            "a wide m takes far longer than a thin i: {strokes:?}"
        );
        assert_eq!(strokes.iter().sum::<u32>(), write_total_ms(3));
    }

    #[test]
    fn thin_glyphs_never_flash_by() {
        // Zero-width glyphs still get a real stroke, so a run of i's and l's
        // reads as writing rather than as a flicker.
        let glyphs = schedule("illili", |_| 0.0);
        assert!(
            glyphs.windows(2).all(|w| w[1].ends_ms > w[0].ends_ms),
            "{glyphs:?}"
        );
        assert_eq!(glyphs.len(), 6);
        assert_eq!(glyphs.last().map(|g| g.ends_ms), Some(write_total_ms(6)));
    }

    #[test]
    fn single_glyph_takes_the_whole_window() {
        let glyphs = schedule("W", |_| 12.0);
        assert_eq!(
            glyphs,
            vec![Glyph {
                ends_ms: MIN_WRITE_MS,
                width: 12.0
            }]
        );
    }

    #[test]
    fn nothing_is_inked_before_the_first_stroke_and_all_of_it_after_the_last() {
        let glyphs = even("Milk");
        assert_eq!(inked(&glyphs, 0), 0.0);
        assert_eq!(inked(&glyphs, 10_000), 1.0);
    }

    #[test]
    fn the_ink_only_ever_advances() {
        let glyphs = even("Buy oat milk");
        let total = glyphs.last().expect("a line").ends_ms;
        let mut previous = 0.0_f32;
        for elapsed in (0..=total).step_by(3) {
            let drawn = inked(&glyphs, elapsed);
            assert!(drawn >= previous, "ink went backwards at {elapsed}");
            previous = drawn;
        }
        assert_eq!(previous, 1.0, "the line finishes written");
    }

    #[test]
    fn a_glyph_is_inked_before_the_nib_leaves_it() {
        // The nib sits at the edge of the ink, so by the time a glyph's own
        // stroke ends that glyph must be fully written: no letter may appear
        // after the pen has moved off it.
        let glyphs = even("Buy oat milk");
        let total = line_width(&glyphs);
        let mut written = 0.0_f64;
        for (index, glyph) in glyphs.iter().enumerate() {
            written += glyph.width;
            let inked = f64::from(inked(&glyphs, glyph.ends_ms)) * total;
            assert!(
                inked >= written - total * 1e-5,
                "glyph {index} of {glyphs:?} is unwritten ({inked:.3} of {written:.3}) when the nib leaves it"
            );
        }
    }

    #[test]
    fn a_word_gap_makes_the_ink_pause() {
        // The pen's time on a space is its width plus the lift across the gap,
        // so the ink runs slower over a word break than the break is wide — and
        // slower than a letter of the same place in the line.
        let glyphs = schedule("ab cd", |c| if c.is_whitespace() { 6.0 } else { 10.0 });
        let space = glyphs[2];
        let total = glyphs.last().expect("a line").ends_ms;
        let stroke = space.ends_ms - glyphs[1].ends_ms;
        assert!(
            f64::from(stroke) / f64::from(total) > space.width / line_width(&glyphs),
            "the space took {stroke}ms of {total}ms to cross {:.1}px",
            space.width
        );
        // And a letter in the same place costs less than the gap.
        let tight = schedule("abcd", |_| 10.0);
        assert!(
            stroke > tight[2].ends_ms - tight[1].ends_ms,
            "{glyphs:?} vs {tight:?}"
        );
    }

    #[test]
    fn the_ink_follows_the_page_and_not_the_clock() {
        // The pen lifts across a gap, so the space costs more time than it
        // covers: when its stroke ends the ink is only the two letters before
        // it. A line advanced by the clock alone would have got further.
        let glyphs = schedule("ab cd", |c| if c.is_whitespace() { 6.0 } else { 10.0 });
        let space = glyphs[2];
        let written = f64::from(inked(&glyphs, space.ends_ms)) * line_width(&glyphs);
        assert!(
            (written - 26.0).abs() < 1e-2,
            "the gap cost {written:.2}px of ink where the two letters are 20px and the space 6px"
        );
    }

    #[test]
    fn the_baseline_sits_below_the_middle_of_the_line_box() {
        // A 20px box whose font runs 16 above the baseline and 4 below: the
        // shared leading (none) puts the baseline 16px down.
        assert!((baseline_y(100.0, 20.0, 16.0, 4.0) - 116.0).abs() < 1e-9);
        // Loose line-height: the leading is shared, so the baseline only moves
        // down by half of it.
        assert!((baseline_y(0.0, 40.0, 16.0, 4.0) - 26.0).abs() < 1e-9);
    }

    #[test]
    fn the_tool_rambo_holds_decides_the_ink() {
        use crate::settings::Pen;
        for pen in [
            Pen::FountainPen,
            Pen::Pencil,
            Pen::Ballpoint,
            Pen::Quill,
            Pen::Marker,
        ] {
            // Every tool in the gear menu writes in its own ink, under the
            // slug that becomes the row's class.
            assert_eq!(ink_for_pen(Some(pen.slug())), pen.slug(), "{pen:?}");
        }
        // No tool chosen yet, or one nobody has heard of.
        assert_eq!(ink_for_pen(None), DEFAULT_PEN);
        assert_eq!(ink_for_pen(Some("paintbrush")), DEFAULT_PEN);
    }

    #[test]
    fn crossing_out_grows_with_the_title_and_never_overruns() {
        assert_eq!(scribble_ms(""), SCRIBBLE_BASE_MS);
        assert_eq!(scribble_ms("   "), SCRIBBLE_BASE_MS, "spaces cost nothing");
        let short = scribble_ms("Milk");
        let long = scribble_ms("Milk, and bread, and the eggs as well");
        assert!(short < long, "{short} !< {long}");
        assert!(long <= MAX_SCRIBBLE_MS, "{long}");
        assert_eq!(scribble_ms(&"x".repeat(500)), MAX_SCRIBBLE_MS);
    }
}
