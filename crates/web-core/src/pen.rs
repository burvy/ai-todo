//! Rambo's writing schedule and nib arithmetic.
//!
//! Pure and unit-tested: no DOM, no clock, no timers. The view layer reads
//! these numbers and animates the pen; nothing here moves anything.
//!
//! A fresh row's title is inked under the nib: each glyph is given time in
//! proportion to the width it occupies on the page, and every word gap also
//! pays for the pen's lift. A glyph's ink appears as the nib passes its middle,
//! so a title that wraps is written line after line, the nib riding the
//! baseline of the glyph it is on. His hand moves with the strokes: a press at
//! each glyph, a lift over each gap, a little wobble. The pen lingers [`LINGER_MS`](crate::tuning::LINGER_MS) at the capture
//! caret after typing stops before flying home. Its timings are in
//! [`crate::tuning`].

use crate::mascot::State;
use crate::tuning::{
    INK_STEP_MAX_MS, WORD_GAP_W, WRITE_DIP_PX, WRITE_FLICK_DEG, WRITE_FLICK_MS, WRITE_FLICK_PX,
    WRITE_HOLD_MAX_MS, WRITE_LIFT_PX, WRITE_MAX_MS, WRITE_MIN_MS, WRITE_PER_GLYPH_MS,
    WRITE_TOUCHDOWN_MS, WRITE_TOUCHDOWN_PX, WRITE_WOBBLE_DEG,
};
use unicode_segmentation::UnicodeSegmentation;

/// A title's glyphs as the reader sees them: grapheme clusters, so an accent, an
/// emoji with a skin tone, a flag or a ZWJ family is one glyph, written in one
/// stroke and inked as one.
pub fn glyphs_of(text: &str) -> impl Iterator<Item = &str> {
    text.graphemes(true)
}

/// Whether `glyph` is a word gap, which the pen lifts across.
pub fn is_gap(glyph: &str) -> bool {
    glyph.chars().all(char::is_whitespace)
}

/// Even the narrowest glyph costs this much of the schedule (an `i` is
/// quick, not free), so a run of thin glyphs never flashes by.
pub const MIN_GLYPH_W: f64 = 6.0;

/// The pen's drawn height in CSS pixels; must mirror `.rambo img` in
/// theme.css. Scaled up 1.25x for the redesign.
pub const PEN_H: f64 = 65.0;
/// The scale the pen writes at (shrunk so it does not cover the words).
pub const WRITE_SCALE: f64 = 0.78;

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
    (glyphs as u32)
        .saturating_mul(WRITE_PER_GLYPH_MS)
        .clamp(WRITE_MIN_MS, WRITE_MAX_MS)
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
pub fn schedule(text: &str, width_of: impl Fn(&str) -> f64) -> Vec<Glyph> {
    let widths: Vec<f64> = glyphs_of(text).map(&width_of).collect();
    if widths.is_empty() {
        return Vec::new();
    }
    let weights: Vec<f64> = glyphs_of(text)
        .zip(&widths)
        .map(|(glyph, &width)| match is_gap(glyph) {
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

/// One line box of a title as the page lays it out, in viewport pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineBox {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

/// Where the pen is in the schedule: the glyph being stroked, and how far through its
/// stroke it is, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub index: usize,
    pub progress: f64,
}

/// The stroke `elapsed_ms` into the schedule. Past the end it is the last glyph,
/// finished; an empty title has none.
pub fn stroke_at(glyphs: &[Glyph], elapsed_ms: u32) -> Option<Stroke> {
    let last = glyphs.len().checked_sub(1)?;
    let index = glyphs
        .iter()
        .position(|glyph| glyph.ends_ms > elapsed_ms)
        .unwrap_or(last);
    let start = index
        .checked_sub(1)
        .map_or(0, |before| glyphs[before].ends_ms);
    let stroke = glyphs[index].ends_ms.saturating_sub(start).max(1);
    let progress = f64::from(elapsed_ms.saturating_sub(start)) / f64::from(stroke);
    Some(Stroke {
        index,
        progress: progress.clamp(0.0, 1.0),
    })
}

/// How many glyphs show `elapsed_ms` in: a glyph's ink appears as the nib passes
/// its middle, so the letter is never half there.
pub fn inked_count(glyphs: &[Glyph], elapsed_ms: u32) -> usize {
    stroke_at(glyphs, elapsed_ms).map_or(0, |at| at.index + usize::from(at.progress >= 0.5))
}

/// How his hand sits on the paper: the nib pressed down (positive) or lifted
/// (negative), in px, and his lean turned from its writing angle, in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hand {
    pub dip_px: f64,
    pub tilt_deg: f64,
}

impl Hand {
    pub const REST: Hand = Hand {
        dip_px: 0.0,
        tilt_deg: 0.0,
    };
}

/// A swell over `share` of a movement, 0 to 1: nothing at either end and all of
/// it in the middle, so a hand that follows it never jumps.
fn swell(share: f64) -> f64 {
    match share {
        share if share > 0.0 && share < 1.0 => (std::f64::consts::PI * share).sin(),
        _ => 0.0,
    }
}

/// His hand on `stroke`, `gap` when its glyph is a space. The nib presses in
/// through each glyph's stroke and comes back up as it ends; over a gap it lifts
/// and comes back down. The lean wobbles as he goes, keyed to where he is in the
/// title, so a title always writes the same way and never like a plotter. Every
/// part is continuous from one glyph into the next, so the hand moves, never
/// jumps.
pub fn hand(stroke: Stroke, gap: bool) -> Hand {
    let dip_px = match gap {
        true => -WRITE_LIFT_PX,
        false => WRITE_DIP_PX,
    } * swell(stroke.progress);
    Hand {
        dip_px,
        tilt_deg: WRITE_WOBBLE_DEG * ((stroke.index as f64 + stroke.progress) * 2.4).sin(),
    }
}

/// His hand `ms` after landing on the line: the nib touches down and settles back
/// over [`WRITE_TOUCHDOWN_MS`].
pub fn touch_down(ms: u32) -> Hand {
    Hand {
        dip_px: WRITE_TOUCHDOWN_PX * swell(f64::from(ms) / f64::from(WRITE_TOUCHDOWN_MS)),
        tilt_deg: 0.0,
    }
}

/// His hand `ms` into the flick off the paper as the last letter closes, from
/// `last`, his hand on that letter: up and turned away, then back to rest by
/// [`WRITE_FLICK_MS`].
pub fn flick(ms: u32, last: Hand) -> Hand {
    let share = (f64::from(ms) / f64::from(WRITE_FLICK_MS)).min(1.0);
    Hand {
        dip_px: last.dip_px * (1.0 - share) - WRITE_FLICK_PX * swell(share),
        tilt_deg: last.tilt_deg * (1.0 - share) + WRITE_FLICK_DEG * swell(share),
    }
}

/// What the pen does with a half-written row on this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Beat {
    /// Keep writing.
    Write,
    /// He is busy elsewhere (in your hand, at the caret, dazed): the ink waits.
    Hold,
    /// The row went off the screen under him: he flies home and waits there.
    FlyHome,
    /// He is home and the row is on screen again: he flies back to finish it.
    FlyBack,
    /// The row is gone, or has waited long enough: the rest simply appears.
    Finish,
}

/// The pen's next beat on a half-written row: interrupted, he stops; when he is
/// home again and can see the row, he goes back and carries on from the letter he
/// stopped at. `held_ms` is how long the ink has been waiting.
pub fn beat(state: &State, on_screen: bool, on_page: bool, held_ms: u32) -> Beat {
    match (state, on_screen) {
        _ if !on_page || held_ms >= WRITE_HOLD_MAX_MS => Beat::Finish,
        (State::WritingRow, true) => Beat::Write,
        (State::WritingRow, false) => Beat::FlyHome,
        (State::Home, true) => Beat::FlyBack,
        _ => Beat::Hold,
    }
}

/// Whether any of a box `(left, top, right, bottom)` shows in a `(width,
/// height)` viewport. The pen only flies to a line he can reach on screen: a
/// row on the other pane of a phone, or scrolled away, would send him off the
/// edge of the page to write it.
pub fn on_screen(
    (left, top, right, bottom): (f64, f64, f64, f64),
    (width, height): (f64, f64),
) -> bool {
    right > 0.0 && bottom > 0.0 && left < width && top < height
}

/// Where his centre sits so his nib lands on `(x, y)` while he writes a row, his
/// box `box_h` px tall (it is larger on a desktop than on a phone). He leans and
/// shrinks about his nib (the writing-a-row pose turns about it, `engine::look`), so
/// the nib is always the bottom of his upright box, half a box below the centre,
/// whatever the lean.
pub fn row_nib_center(x: f64, y: f64, box_h: f64) -> (f64, f64) {
    (x, y - box_h / 2.0)
}

/// Where the pen's centre sits so its nib lands on `(caret_x, caret_y)`:
/// the nib hangs half a scaled pen below the centre while writing.
pub fn writing_pen_center(caret_x: f64, caret_y: f64) -> (f64, f64) {
    (caret_x, caret_y - (PEN_H / 2.0) * WRITE_SCALE)
}

/// The text before a caret the browser reports in UTF-16 code units (`selectionStart`): an
/// emoji is two of them, so counting chars would put the caret too far along. A caret
/// inside a surrogate pair (never reported for typed text) stops before that character.
pub fn before_caret(text: &str, utf16_units: usize) -> &str {
    let mut seen = 0;
    for (at, ch) in text.char_indices() {
        if seen + ch.len_utf16() > utf16_units {
            return &text[..at];
        }
        seen += ch.len_utf16();
    }
    text
}

/// Where the nib is across the capture line when the caret has `advance` px of text before
/// it: past the box's left padding, back by however far the text has scrolled, and never
/// beyond the line's written width (a long line scrolls and the caret stays at its end).
/// All in px; `box_left` is the line's left edge on screen.
pub fn caret_x(
    box_left: f64,
    padding_left: f64,
    advance: f64,
    scroll: f64,
    content_width: f64,
) -> f64 {
    box_left + padding_left + (advance - scroll).clamp(0.0, content_width.max(0.0))
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

/// How far the pen advances for the time really passed since `last`, in whole ms, and the
/// time that step accounts for (the next `last`). The loop waits for the engine's next frame,
/// which may be later than one `INK_TICK_MS` (a fast screen is sooner, a slow one later), so
/// the pen is paced by the clock and not by counting waits; the part of a ms not yet paid is
/// kept in the returned time, so no rounding drifts. At least 1 ms, and never past
/// `INK_STEP_MAX_MS`: a tab that slept does not throw the pen ahead, it simply carries on.
pub fn step_between(last: f64, now: f64) -> (u32, f64) {
    let passed = (now - last).floor().max(1.0);
    if passed > f64::from(INK_STEP_MAX_MS) {
        (INK_STEP_MAX_MS, now)
    } else {
        (passed as u32, last + passed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::INK_TICK_MS;

    #[test]
    fn the_text_before_a_caret_counts_utf16_units_not_chars() {
        assert_eq!(before_caret("hello", 3), "hel");
        assert_eq!(before_caret("hello", 0), "");
        assert_eq!(before_caret("hello", 99), "hello");
        // An emoji is two units: a caret after it is at 2, and after "ab" at 4.
        assert_eq!(before_caret("🙂ab", 2), "🙂");
        assert_eq!(before_caret("🙂ab", 4), "🙂ab");
        assert_eq!(before_caret("a🙂b", 3), "a🙂");
        // Inside a pair it stops before the character, never splitting it.
        assert_eq!(before_caret("🙂ab", 1), "");
        // Accents are one unit each.
        assert_eq!(before_caret("café!", 4), "café");
    }

    #[test]
    fn the_nib_stands_at_the_end_of_the_words_on_the_paper() {
        // A line at x 100 with 28 px of paper before the words: an empty line has the nib at
        // the start of the words, and each px of text moves it right by as much.
        assert_eq!(caret_x(100.0, 28.0, 0.0, 0.0, 400.0), 128.0);
        assert_eq!(caret_x(100.0, 28.0, 90.0, 0.0, 400.0), 218.0);
        // A long line scrolls: the caret keeps its place relative to the text.
        assert_eq!(caret_x(100.0, 28.0, 700.0, 350.0, 400.0), 478.0);
        // It never leaves the written width, however far the text runs.
        assert_eq!(caret_x(100.0, 28.0, 900.0, 0.0, 400.0), 528.0);
        assert_eq!(caret_x(100.0, 28.0, -5.0, 0.0, 400.0), 128.0);
        assert_eq!(
            caret_x(100.0, 28.0, 30.0, 0.0, -10.0),
            128.0,
            "a box with no room"
        );
    }

    /// Every letter the same width: the easy case to reason about.
    fn even(text: &str) -> Vec<Glyph> {
        schedule(text, |glyph| if is_gap(glyph) { 6.0 } else { 10.0 })
    }

    #[test]
    fn write_total_scales_with_length_and_clamps() {
        assert_eq!(write_total_ms(0), WRITE_MIN_MS);
        assert_eq!(write_total_ms(3), WRITE_MIN_MS);
        assert_eq!(write_total_ms(10), 10 * WRITE_PER_GLYPH_MS);
        assert_eq!(write_total_ms(100), WRITE_MAX_MS);
        assert_eq!(write_total_ms(usize::MAX), WRITE_MAX_MS, "never overflows");
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
    fn wide_glyphs_take_longer_than_thin_ones() {
        let glyphs = schedule("mim", |glyph| if glyph == "m" { 20.0 } else { 4.0 });
        let strokes: Vec<u32> = glyphs
            .iter()
            .scan(0_u32, |spent, glyph| {
                let stroke = glyph.ends_ms - *spent;
                *spent = glyph.ends_ms;
                Some(stroke)
            })
            .collect();
        assert_eq!(strokes[0], strokes[2], "the two m's are one letter");
        // Proportional to width, not a flat share, so a wide m takes far
        // longer than a thin i. MIN_GLYPH_W floors the i, so the ratio is
        // about 3.3x rather than the raw width ratio.
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
                ends_ms: WRITE_MIN_MS,
                width: 12.0
            }]
        );
    }

    #[test]
    fn a_glyph_is_what_the_reader_sees_as_one() {
        // e + combining acute, a thumbs-up with a skin tone, a flag, a ZWJ family:
        // each is several chars and one glyph, written in one stroke.
        let title = "Caf\u{65}\u{301} \u{1F44D}\u{1F3FD}\u{1F1F3}\u{1F1FF}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let glyphs: Vec<&str> = glyphs_of(title).collect();
        assert_eq!(
            glyphs,
            [
                "C",
                "a",
                "f",
                "e\u{301}",
                " ",
                "\u{1F44D}\u{1F3FD}",
                "\u{1F1F3}\u{1F1FF}",
                "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            ]
        );
        assert_eq!(even(title).len(), glyphs.len(), "one stroke a glyph");
        assert!(is_gap(" ") && !is_gap("e\u{301}"));
    }

    #[test]
    fn empty_text_writes_nothing() {
        assert!(schedule("", |_| 5.0).is_empty());
        assert_eq!(stroke_at(&[], 10), None);
        assert_eq!(inked_count(&[], 10), 0);
    }

    #[test]
    fn the_nib_is_on_one_glyph_at_a_time() {
        let glyphs = even("Milk");
        let first = glyphs[0].ends_ms;
        assert_eq!(
            stroke_at(&glyphs, 0),
            Some(Stroke {
                index: 0,
                progress: 0.0
            })
        );
        assert_eq!(stroke_at(&glyphs, first / 2).map(|at| at.index), Some(0));
        assert_eq!(
            stroke_at(&glyphs, first).map(|at| at.index),
            Some(1),
            "on to the i"
        );
        assert_eq!(
            stroke_at(&glyphs, 10_000),
            Some(Stroke {
                index: 3,
                progress: 1.0
            }),
            "past the end: the k, finished"
        );
    }

    #[test]
    fn a_glyph_s_ink_appears_as_the_nib_passes_its_middle() {
        let glyphs = even("Milk");
        let first = glyphs[0].ends_ms;
        assert_eq!(
            inked_count(&glyphs, 0),
            0,
            "nothing before the first stroke"
        );
        assert_eq!(inked_count(&glyphs, first / 2 - 1), 0, "no half letters");
        assert_eq!(inked_count(&glyphs, first / 2 + 1), 1);
        assert_eq!(inked_count(&glyphs, 10_000), 4, "all of it after the last");
        let total = glyphs.last().expect("a line").ends_ms;
        let counts: Vec<usize> = (0..=total).map(|ms| inked_count(&glyphs, ms)).collect();
        assert!(
            counts.windows(2).all(|w| w[1] >= w[0]),
            "ink only ever advances"
        );
    }

    #[test]
    fn the_nib_presses_at_each_glyph_and_lifts_over_a_gap() {
        let at = |index, progress| Stroke { index, progress };
        assert_eq!(hand(at(3, 0.5), false).dip_px, WRITE_DIP_PX, "mid-stroke");
        assert!(hand(at(3, 1.0), false).dip_px.abs() < 1e-9, "up by the end");
        assert_eq!(
            hand(at(3, 0.5), true).dip_px,
            -WRITE_LIFT_PX,
            "highest over the middle of the gap"
        );
        // A wobble, not a sway: bounded, different between neighbours, and the
        // same each time the glyph is written.
        let tilts: Vec<f64> = (0..8).map(|i| hand(at(i, 0.5), false).tilt_deg).collect();
        assert!(tilts.iter().all(|tilt| tilt.abs() <= WRITE_WOBBLE_DEG));
        assert!(
            tilts.windows(2).all(|w| (w[1] - w[0]).abs() > 0.1),
            "{tilts:?}"
        );
        assert_eq!(hand(at(5, 0.5), false), hand(at(5, 0.5), false));
    }

    #[test]
    fn he_touches_down_and_settles() {
        assert_eq!(touch_down(0), Hand::REST);
        assert_eq!(
            touch_down(WRITE_TOUCHDOWN_MS / 2).dip_px,
            WRITE_TOUCHDOWN_PX
        );
        assert!(touch_down(WRITE_TOUCHDOWN_MS).dip_px.abs() < 1e-9);
        assert_eq!(touch_down(10 * WRITE_TOUCHDOWN_MS), Hand::REST);
    }

    #[test]
    fn he_flicks_up_and_away_and_comes_to_rest() {
        let last = Hand {
            dip_px: 0.0,
            tilt_deg: -1.5,
        };
        assert_eq!(flick(0, last), last, "it starts from his last letter");
        let top = flick(WRITE_FLICK_MS / 2, last);
        assert!(
            top.dip_px < 0.0 && top.tilt_deg > 0.0,
            "up and away: {top:?}"
        );
        let end = flick(WRITE_FLICK_MS, last);
        assert!(
            end.dip_px.abs() < 1e-9 && end.tilt_deg.abs() < 1e-9,
            "{end:?}"
        );
    }

    /// The most his hand moves between two frames over `hands`, in px and degrees.
    fn largest_step(hands: &[Hand]) -> (f64, f64) {
        hands.windows(2).fold((0.0, 0.0), |(dip, tilt), w| {
            (
                f64::max(dip, (w[1].dip_px - w[0].dip_px).abs()),
                f64::max(tilt, (w[1].tilt_deg - w[0].tilt_deg).abs()),
            )
        })
    }

    #[test]
    fn his_hand_moves_and_never_jumps() {
        // Frame by frame through a title, glyph boundaries and gaps included, then
        // the flick: a hand, not a jitter.
        let text = "Buy oat milk";
        let glyphs = even(text);
        let gaps: Vec<bool> = glyphs_of(text).map(is_gap).collect();
        let total = glyphs.last().expect("a line").ends_ms;
        let writing: Vec<Hand> = (0..=total)
            .step_by(INK_TICK_MS as usize)
            .filter_map(|ms| stroke_at(&glyphs, ms))
            .map(|at| hand(at, gaps[at.index]))
            .collect();
        let (dip, tilt) = largest_step(&writing);
        assert!(
            dip <= 1.0 && tilt <= 1.0,
            "{dip:.2}px, {tilt:.2}deg in a frame"
        );
        let last = *writing.last().expect("frames");
        let flicking: Vec<Hand> = std::iter::once(last)
            .chain(
                (0..=WRITE_FLICK_MS)
                    .step_by(INK_TICK_MS as usize)
                    .map(|ms| flick(ms, last)),
            )
            .collect();
        let (dip, tilt) = largest_step(&flicking);
        assert!(
            dip <= 1.0 && tilt <= 2.0,
            "{dip:.2}px, {tilt:.2}deg in a frame"
        );
    }

    #[test]
    fn a_word_gap_makes_the_ink_pause() {
        // The pen's time on a space is its width plus the lift across the gap,
        // so the ink runs slower over a word break than the break is wide — and
        // slower than a letter of the same place in the line.
        let glyphs = schedule("ab cd", |glyph| if is_gap(glyph) { 6.0 } else { 10.0 });
        let space = glyphs[2];
        let total = glyphs.last().expect("a line").ends_ms;
        let stroke = space.ends_ms - glyphs[1].ends_ms;
        assert!(
            f64::from(stroke) / f64::from(total)
                > space.width / glyphs.iter().map(|glyph| glyph.width).sum::<f64>(),
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
    fn interrupted_he_stops_and_goes_back_when_he_can() {
        // Writing, and the row on screen.
        assert_eq!(beat(&State::WritingRow, true, true, 0), Beat::Write);
        // Picked up, or typing at the caret, or dazed: the ink waits.
        for busy in [
            State::Dragging,
            State::WritingCaret,
            State::Dazed,
            State::Flying,
        ] {
            assert_eq!(beat(&busy, true, true, 0), Beat::Hold, "{busy:?}");
        }
        // The row scrolled away under him: he goes home rather than off screen.
        assert_eq!(beat(&State::WritingRow, false, true, 0), Beat::FlyHome);
        // Home, and the row is back in view: he goes back to it.
        assert_eq!(beat(&State::Home, true, true, 0), Beat::FlyBack);
        assert_eq!(beat(&State::Home, false, true, 0), Beat::Hold);
    }

    #[test]
    fn a_row_gone_or_waited_long_enough_simply_appears() {
        assert_eq!(beat(&State::WritingRow, true, false, 0), Beat::Finish);
        assert_eq!(
            beat(&State::Dragging, true, true, WRITE_HOLD_MAX_MS),
            Beat::Finish
        );
    }

    #[test]
    fn a_row_s_nib_is_the_bottom_of_his_box() {
        // Phone and desktop boxes: the lean never enters into it.
        assert_eq!(row_nib_center(100.0, 200.0, 64.0), (100.0, 168.0));
        assert_eq!(row_nib_center(100.0, 200.0, 80.0), (100.0, 160.0));
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

    #[test]
    fn the_pen_only_flies_to_a_line_on_the_screen() {
        let phone = (390.0, 844.0);
        assert!(on_screen((16.0, 120.0, 374.0, 160.0), phone));
        assert!(
            on_screen((-20.0, 120.0, 30.0, 160.0), phone),
            "half on is on"
        );
        // The list pane while the planner page is showing sits one page left.
        assert!(!on_screen((-374.0, 120.0, -16.0, 160.0), phone));
        assert!(
            !on_screen((-390.0, 120.0, 0.0, 160.0), phone),
            "touching is off"
        );
        // Scrolled above or below the window.
        assert!(!on_screen((16.0, -80.0, 374.0, -40.0), phone));
        assert!(!on_screen((16.0, 844.0, 374.0, 884.0), phone));
    }

    #[test]
    fn the_pen_advances_by_the_time_that_really_passed() {
        // A frame later at 144 Hz, 60 Hz and 30 Hz is what the pen is owed.
        assert_eq!(step_between(0.0, 7.0), (7, 7.0));
        assert_eq!(
            step_between(100.0, 116.7),
            (16, 116.0),
            "the 0.7 stays owed"
        );
        assert_eq!(step_between(116.0, 133.4), (17, 133.0));
    }

    #[test]
    fn a_step_is_at_least_one_and_a_slept_tab_is_capped() {
        assert_eq!(
            step_between(5.0, 5.0).0,
            1,
            "a clock that has not moved still moves"
        );
        assert_eq!(step_between(5.0, 4.0).0, 1, "and never backwards");
        assert_eq!(step_between(0.0, 60_000.0), (INK_STEP_MAX_MS, 60_000.0));
    }

    #[test]
    fn the_same_stretch_is_written_in_the_same_time_at_any_frame_rate() {
        // A pen owed 1,000 ms of writing, stepped by frames of each length, gets there in
        // the same real time give or take a frame.
        for frame in [7.0_f64, 16.7, 33.4] {
            let (mut now, mut last, mut written) = (0.0, 0.0, 0u32);
            while written < 1_000 {
                now += frame;
                let (step, next) = step_between(last, now);
                written += step;
                last = next;
            }
            assert!((now - 1_000.0).abs() <= frame, "{frame}: took {now} ms");
        }
    }
}
