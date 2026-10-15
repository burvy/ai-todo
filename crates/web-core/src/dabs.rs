//! How a stroke of ink becomes dabs on the page: the pure half of the ink layer.
//!
//! [`batches`] takes the strokes laid on a surface and the frame's clock and returns
//! the dabs to paint, grouped by ink and strength, faintest first. The page's canvas
//! only fills what it is given (`web/src/inklayer.rs`), so the rule is tested here.

use crate::ink::{Dot, Stroke, age_alpha, dryness};
use crate::settings::Pen;

/// The token a dab is painted in: a CSS token, so the ink follows the page's
/// palette in light and dark without anything here knowing a colour.
///
/// DESIGN-DIRECTION §6: the fountain pen and the marker write in `--ink`, the
/// ballpoint in `--ink-ballpoint`, the quill in `--ink-quill`, and the tail of a
/// stroke in `--rambo-tint` whatever the tool.
pub fn token_for(pen: Pen, tail: bool) -> &'static str {
    if tail {
        return "--rambo-tint";
    }
    match pen {
        Pen::FountainPen | Pen::Pencil | Pen::Marker => "--ink",
        Pen::Ballpoint => "--ink-ballpoint",
        Pen::Quill => "--ink-quill",
    }
}

/// A stroke as it lies on the page: its dabs, worked out once, and when it was laid.
#[derive(Clone, Debug, PartialEq)]
pub struct Laid {
    pub dots: Vec<Dot>,
    pub pen: Pen,
    pub at: f64,
    /// The nib's full width, against which a dab's thinness is measured: thin ink
    /// fades first.
    pub radius: f64,
    /// How uneven the tool's ink turns as it dries.
    pub dry_grain: f64,
}

impl Laid {
    pub fn of(stroke: &Stroke) -> Self {
        let nib = crate::ink::Nib::for_pen(stroke.pen);
        Laid {
            dots: stroke.dots(),
            pen: stroke.pen,
            at: stroke.laid_at(),
            radius: nib.radius,
            dry_grain: nib.dry_grain,
        }
    }
}

/// Dabs that share one ink and one strength, filled together in one path. A path is
/// filled once however many of its dabs overlap, so neighbouring dabs never bead
/// where they meet and the line reads as one continuous stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub token: &'static str,
    pub alpha: f64,
    pub dabs: Vec<(f64, f64, f64)>,
}

/// How many strengths of ink are told apart, so a frame is a few dozen fills and not
/// one per dab. Fine enough that no step shows.
const ALPHA_LEVELS: f64 = 48.0;

/// The page's ink at `now` (the frames' clock, in ms): every dab, aged, grouped by
/// ink and strength. Ink that has faded away is left out.
pub fn batches(laid: &[Laid], now: f64) -> Vec<Batch> {
    let mut out: Vec<Batch> = Vec::new();
    for stroke in laid {
        for dot in &stroke.dots {
            // Each dab ages from when the nib laid it, so a stroke fades the way it
            // was drawn; thin ink goes first, and dry ink pools a little unevenly.
            let age = now - dot.t;
            let thinness = 1.0 - dot.radius / stroke.radius.max(f64::EPSILON);
            let pooled = 1.0 + stroke.dry_grain * dot.pooling * dryness(age);
            let strength = dot.alpha * age_alpha(age, thinness) * pooled;
            let alpha = (strength.clamp(0.0, 1.0) * ALPHA_LEVELS).round() / ALPHA_LEVELS;
            if alpha <= 0.0 {
                continue;
            }
            let token = token_for(stroke.pen, dot.tail);
            let dab = (dot.x, dot.y, dot.radius);
            match out
                .iter_mut()
                .find(|b| b.token == token && b.alpha == alpha)
            {
                Some(batch) => batch.dabs.push(dab),
                None => out.push(Batch {
                    token,
                    alpha,
                    dabs: vec![dab],
                }),
            }
        }
    }
    // Faintest first: each batch is laid over the ones before it in place of them
    // (`paint`), so where two strengths meet the stronger shows, never their sum.
    out.sort_by(|a, b| a.alpha.total_cmp(&b.alpha));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ink::Sample;
    use crate::tuning::{INK_FADE_OUT_MS, INK_HOLD_MS};

    fn stroke(pen: Pen) -> Stroke {
        Stroke {
            samples: (0..=40)
                .map(|i| Sample {
                    x: f64::from(i) * 5.0,
                    y: 40.0,
                    t: f64::from(i) * 50.0,
                })
                .collect(),
            pen,
            on: crate::ink::Surface::Floor,
        }
    }

    #[test]
    fn every_tool_writes_in_its_own_ink() {
        assert_eq!(token_for(Pen::FountainPen, false), "--ink");
        assert_eq!(token_for(Pen::Pencil, false), "--ink");
        assert_eq!(token_for(Pen::Marker, false), "--ink");
        assert_eq!(token_for(Pen::Ballpoint, false), "--ink-ballpoint");
        assert_eq!(token_for(Pen::Quill, false), "--ink-quill");
    }

    #[test]
    fn the_tail_is_rambo_tint_whichever_tool_drew_it() {
        for pen in [
            Pen::FountainPen,
            Pen::Ballpoint,
            Pen::Pencil,
            Pen::Marker,
            Pen::Quill,
        ] {
            assert_eq!(token_for(pen, true), "--rambo-tint", "{pen:?}");
        }
    }

    #[test]
    fn a_stroke_of_one_ink_is_filled_in_a_few_paths_with_every_dab() {
        let laid = Laid::of(&stroke(Pen::FountainPen));
        let at = laid.at;
        let on_page = batches(std::slice::from_ref(&laid), at);
        let dabs: usize = on_page.iter().map(|b| b.dabs.len()).sum();
        assert_eq!(dabs, laid.dots.len(), "nothing lost");
        // Laid over two seconds, the stroke has aged along its length, yet it is
        // still a bounded number of fills, the freshest dab is full, and the tail is
        // Rambo's own.
        assert!(
            on_page.len() <= 2 * ALPHA_LEVELS as usize,
            "{} fills",
            on_page.len()
        );
        assert!(
            on_page
                .iter()
                .any(|b| b.token == "--rambo-tint" && b.alpha == 1.0)
        );
    }

    #[test]
    fn fresh_ink_is_full_and_faded_ink_paints_nothing() {
        let laid = Laid::of(&stroke(Pen::FountainPen));
        let fresh = batches(std::slice::from_ref(&laid), laid.at);
        assert!(fresh.iter().any(|b| b.alpha == 1.0));
        let later = batches(
            std::slice::from_ref(&laid),
            laid.at + INK_HOLD_MS + INK_FADE_OUT_MS,
        );
        assert!(later.is_empty(), "{later:?}");
    }

    #[test]
    fn a_frame_is_a_few_dozen_fills_even_mid_fade_in_graphite() {
        let laid = Laid::of(&stroke(Pen::Pencil));
        let mid_fade = laid.at + INK_HOLD_MS + INK_FADE_OUT_MS * 0.3;
        let on_page = batches(std::slice::from_ref(&laid), mid_fade);
        assert!(!on_page.is_empty());
        assert!(
            on_page.len() <= 2 * ALPHA_LEVELS as usize,
            "{} fills",
            on_page.len()
        );
    }

    #[test]
    fn the_faintest_ink_is_laid_first_so_the_strongest_wins() {
        let laid = Laid::of(&stroke(Pen::FountainPen));
        let now = laid.at + INK_HOLD_MS + INK_FADE_OUT_MS * 0.3;
        let order: Vec<f64> = batches(std::slice::from_ref(&laid), now)
            .iter()
            .map(|b| b.alpha)
            .collect();
        assert!(
            order.len() > 1 && order.windows(2).all(|w| w[0] <= w[1]),
            "{order:?}"
        );
    }

    #[test]
    fn a_stroke_fades_the_way_it_was_drawn() {
        // Laid over two seconds: mid-fade its start is fainter than its end.
        let laid = Laid::of(&stroke(Pen::Ballpoint));
        let first = laid.dots[laid.dots.len() / 8];
        let last = laid.dots[laid.dots.len() * 7 / 8];
        assert!(last.t - first.t > 1000.0);
        let now = first.t + INK_HOLD_MS + INK_FADE_OUT_MS * 0.5;
        let on_page = batches(std::slice::from_ref(&laid), now);
        let strength_at = |dot: &Dot| {
            on_page
                .iter()
                .find(|b| b.dabs.contains(&(dot.x, dot.y, dot.radius)))
                .map_or(0.0, |b| b.alpha)
        };
        assert!(
            strength_at(&first) < strength_at(&last),
            "the start goes first"
        );
        // Still a few dozen fills, not one per dab.
        assert!(
            on_page.len() <= 2 * ALPHA_LEVELS as usize,
            "{}",
            on_page.len()
        );
    }

    #[test]
    fn a_stroke_lies_in_the_ink_of_the_tool_that_drew_it() {
        let quill = Laid::of(&stroke(Pen::Quill));
        let on_page = batches(std::slice::from_ref(&quill), quill.at);
        assert!(
            on_page
                .iter()
                .all(|b| b.token == "--ink-quill" || b.token == "--rambo-tint")
        );
    }
}
