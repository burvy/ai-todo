//! How Rambo gets from one spot to another: brisk hops, never a glide.
//!
//! Each hop crouches (he squashes), springs along an arc (he stretches) and lands
//! with a small settle; a longer way is more hops, never slower ones. Pure: the
//! page reads frames off it for anything that moves him (a flight to the caret,
//! the run home, and later the engine's throws that land). The run home is its own
//! way: shorter hops along a bowed path ([`Hop::plan_home`]).

use crate::tuning::{
    HOME_BEND, HOME_HOP_MAX, HOME_HOP_SPAN_PX, HOP_HEIGHT_MAX_PX, HOP_HEIGHT_PX, HOP_LONG_RISE,
    HOP_MAX, HOP_MIN_PX, HOP_MS, HOP_RISE, HOP_SPAN_PX, HOP_SQUASH,
};

/// A run of hops from `from`, planned for the distance it had to cover. The far
/// end is read on every frame ([`Hop::at`]), so a spot that moves while he hops
/// (the nest as the page scrolls) is still where he lands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hop {
    from: (f64, f64),
    hops: u32,
    /// How far the way bows to its left, as a share of its length at the middle.
    bend: f64,
}

/// One frame of a hop. `pos` is the ground point under him, moving along the way;
/// `lift` is how far above it he is, in px. They are kept apart so whatever asks
/// where he is (a hit test, his shadow, the nest) uses the ground, and only his
/// drawing rises: drawn at `(pos.0, pos.1 - lift)`. `squash` is positive squashed
/// (wider and shorter, crouching or landing), negative stretched (in the air).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub pos: (f64, f64),
    pub lift: f64,
    pub squash: f64,
}

/// One hop's shape: the share of the hop he crouches before take-off, and the
/// share at which he touches down again. The rest of the hop is the landing settle.
const TAKE_OFF: f64 = 0.2;
const TOUCH_DOWN: f64 = 0.8;

/// His squash through one hop, as `(share of the hop, share of HOP_SQUASH)`:
/// crouch, spring and stretch, land squashed, settle. Zero at both ends, so one hop
/// runs into the next without a jump.
const SQUASH_KEYS: [(f64, f64); 6] = [
    (0.0, 0.0),
    (TAKE_OFF, 1.0),
    (0.4, -0.6),
    (0.7, -0.2),
    (0.88, 0.7),
    (1.0, 0.0),
];

impl Hop {
    /// The hops from `from` to `to`, or none when the two are close enough that he
    /// simply steps there.
    pub fn plan(from: (f64, f64), to: (f64, f64)) -> Option<Hop> {
        Hop::planned(from, to, HOP_SPAN_PX, HOP_MAX, 0.0)
    }

    /// The way home from `from` to the nest at `to`: hop by hop, more and shorter
    /// hops than a flight, along a path that bows to one side, so it reads as a pen
    /// finding its way back, not a beeline.
    pub fn plan_home(from: (f64, f64), to: (f64, f64)) -> Option<Hop> {
        Hop::planned(from, to, HOME_HOP_SPAN_PX, HOME_HOP_MAX, HOME_BEND)
    }

    fn planned(from: (f64, f64), to: (f64, f64), span: f64, most: u32, bend: f64) -> Option<Hop> {
        let distance = (to.0 - from.0).hypot(to.1 - from.1);
        (distance >= HOP_MIN_PX).then(|| Hop {
            from,
            hops: ((distance / span).ceil() as u32).clamp(1, most),
            bend,
        })
    }

    /// How many hops the way takes.
    pub fn hops(&self) -> u32 {
        self.hops
    }

    /// How long the whole run takes, in ms.
    pub fn ms(&self) -> u32 {
        self.hops * HOP_MS
    }

    /// How fast he is coming down, in px/s, as each hop toward `to` touches down: the
    /// fall of its arc at the end of its flight.
    pub fn impact(&self, to: (f64, f64)) -> f64 {
        let span = (to.0 - self.from.0).hypot(to.1 - self.from.1) / f64::from(self.hops);
        let flight_ms = (TOUCH_DOWN - TAKE_OFF) * f64::from(HOP_MS);
        arc_height(span) * std::f64::consts::PI * 1000.0 / flight_ms
    }

    /// Where he is `elapsed_ms` into the run towards `to`. Past the end he has
    /// landed: at `to`, unsquashed.
    pub fn at(&self, to: (f64, f64), elapsed_ms: u32) -> Frame {
        if elapsed_ms >= self.ms() {
            return Frame {
                pos: to,
                lift: 0.0,
                squash: 0.0,
            };
        }
        let hop = elapsed_ms / HOP_MS;
        let share = f64::from(elapsed_ms % HOP_MS) / f64::from(HOP_MS);
        // How far through its own arc this hop is: still crouched, in the air, or
        // down and settling.
        let flight = ((share - TAKE_OFF) / (TOUCH_DOWN - TAKE_OFF)).clamp(0.0, 1.0);
        let along = (f64::from(hop) + flight) / f64::from(self.hops);
        let span = (to.0 - self.from.0).hypot(to.1 - self.from.1) / f64::from(self.hops);
        let height = arc_height(span);
        // Bowed to the left of the way: nothing at either end, most at the middle.
        let bow = self.bend * (std::f64::consts::PI * along).sin();
        let (dx, dy) = (to.0 - self.from.0, to.1 - self.from.1);
        Frame {
            pos: (
                self.from.0 + dx * along + dy * bow,
                self.from.1 + dy * along - dx * bow,
            ),
            lift: height * (std::f64::consts::PI * flight).sin(),
            squash: HOP_SQUASH * squash_at(share),
        }
    }
}

/// How high a hop `span` px long rises above the straight way, in px. A short hop
/// is a round arc ([`HOP_RISE`] of its length); past [`HOP_HEIGHT_PX`] the height
/// keeps growing, by [`HOP_LONG_RISE`] of each extra px, so a longer hop is always
/// higher — there is no flat stretch — and a long phone trip in few hops never
/// reads as a glide; no hop rises past [`HOP_HEIGHT_MAX_PX`].
fn arc_height(span: f64) -> f64 {
    let join = HOP_HEIGHT_PX / HOP_RISE;
    (span.min(join) * HOP_RISE + (span - join).max(0.0) * HOP_LONG_RISE).min(HOP_HEIGHT_MAX_PX)
}

/// His squash `share` of the way through one hop, read off [`SQUASH_KEYS`].
fn squash_at(share: f64) -> f64 {
    SQUASH_KEYS
        .windows(2)
        .find(|pair| share <= pair[1].0)
        .map_or(0.0, |pair| {
            let ((t0, v0), (t1, v1)) = (pair[0], pair[1]);
            v0 + (v1 - v0) * (share - t0) / (t1 - t0)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::{FLY_MS, INK_TICK_MS, NOTICE_BEAT_MS};

    const NEST: (f64, f64) = (40.0, 40.0);

    #[test]
    fn a_short_way_is_a_step_and_a_long_way_is_more_hops() {
        assert_eq!(Hop::plan(NEST, (NEST.0 + HOP_MIN_PX / 2.0, NEST.1)), None);
        let hops =
            |distance: f64| Hop::plan(NEST, (NEST.0 + distance, NEST.1)).map(|hop| hop.hops());
        assert_eq!(hops(HOP_MIN_PX), Some(1));
        assert_eq!(hops(HOP_SPAN_PX + 1.0), Some(2));
        assert_eq!(
            hops(100.0 * HOP_SPAN_PX),
            Some(HOP_MAX),
            "never more than HOP_MAX"
        );
    }

    #[test]
    fn the_way_home_is_more_shorter_hops_along_a_bow_never_a_beeline() {
        let far = (NEST.0 + 600.0, NEST.1 + 300.0);
        let flight = Hop::plan(far, NEST).expect("a long way");
        let home = Hop::plan_home(far, NEST).expect("a long way");
        assert!(home.hops() > flight.hops(), "hop by hop");
        assert!(home.hops() <= HOME_HOP_MAX);
        assert!(crate::tuning::HOME_RUN_MS >= NOTICE_BEAT_MS + home.ms());
        // Every hop lands off the straight line between where he was and the nest,
        // all to the same side, and the last lands in the nest.
        let (dx, dy) = (NEST.0 - far.0, NEST.1 - far.1);
        let off = |at: (f64, f64)| ((at.0 - far.0) * dy - (at.1 - far.1) * dx) / dx.hypot(dy);
        let landings: Vec<f64> = (1..home.hops())
            .map(|hop| off(home.at(NEST, hop * HOP_MS).pos))
            .collect();
        assert!(landings.iter().all(|&o| o > 10.0), "{landings:?}");
        assert_eq!(home.at(NEST, home.ms()).pos, NEST);
        // A flight still goes straight.
        assert!(off(flight.at(NEST, HOP_MS).pos).abs() < 1e-9);
    }

    #[test]
    fn every_run_fits_in_a_flight() {
        // Callers wait FLY_MS for him to arrive before they start what comes next,
        // and every run starts with his notice beat (`mascot::notice`).
        let far = Hop::plan(NEST, (5000.0, 5000.0)).expect("a long way");
        assert!(NOTICE_BEAT_MS + far.ms() <= FLY_MS, "{}ms", far.ms());
    }

    #[test]
    fn he_starts_where_he_was_and_lands_where_he_was_going() {
        let to = (400.0, 300.0);
        let hop = Hop::plan(NEST, to).expect("a hop");
        assert_eq!(
            hop.at(to, 0),
            Frame {
                pos: NEST,
                lift: 0.0,
                squash: 0.0
            }
        );
        assert_eq!(
            hop.at(to, hop.ms()),
            Frame {
                pos: to,
                lift: 0.0,
                squash: 0.0
            }
        );
        assert_eq!(hop.at(to, 10 * hop.ms()).pos, to);
    }

    #[test]
    fn he_crouches_springs_and_lands() {
        let to = (400.0, 40.0);
        let hop = Hop::plan(NEST, to).expect("a hop");
        let at = |share: f64| hop.at(to, (share * f64::from(HOP_MS)) as u32);
        assert!(at(TAKE_OFF).squash > 0.0, "crouched to take off");
        assert_eq!(at(TAKE_OFF).pos, NEST, "not off yet");
        assert_eq!(at(TAKE_OFF).lift, 0.0, "still on the ground");
        let top = at((TAKE_OFF + TOUCH_DOWN) / 2.0);
        assert!(top.squash < 0.0, "stretched in the air");
        assert!(top.lift > 0.0, "off the ground: {top:?}");
        assert_eq!(top.pos.1, NEST.1, "the ground stays on the way");
        assert!(at(0.88).squash > 0.0, "squashed as he lands");
    }

    #[test]
    fn a_hop_never_jumps_between_frames() {
        // Frame by frame through a long run: the hops join up, and he never moves
        // further in one frame than a brisk hop carries him.
        let to = (900.0, 500.0);
        let hop = Hop::plan(NEST, to).expect("a hop");
        let frames: Vec<Frame> = (0..=hop.ms())
            .step_by(INK_TICK_MS as usize)
            .map(|ms| hop.at(to, ms))
            .collect();
        let span = (to.0 - NEST.0).hypot(to.1 - NEST.1) / f64::from(hop.hops());
        let frame_share = f64::from(INK_TICK_MS) / f64::from(HOP_MS);
        for pair in frames.windows(2) {
            let drawn = |frame: &Frame| (frame.pos.0, frame.pos.1 - frame.lift);
            let (a, b) = (drawn(&pair[0]), drawn(&pair[1]));
            let moved = (b.0 - a.0).hypot(b.1 - a.1);
            assert!(
                moved <= 2.0 * span * frame_share / (TOUCH_DOWN - TAKE_OFF),
                "{pair:?}"
            );
            assert!(
                (pair[1].squash - pair[0].squash).abs() <= HOP_SQUASH,
                "{pair:?}"
            );
        }
    }

    #[test]
    fn a_longer_hop_rises_higher_up_to_a_ceiling() {
        let spans = (1..=2000).map(f64::from);
        let heights: Vec<f64> = spans.clone().map(arc_height).collect();
        assert!(
            heights.windows(2).all(|pair| pair[0] <= pair[1]),
            "a longer hop never rises lower"
        );
        for (span, &height) in spans.zip(heights.iter()) {
            assert!(height <= HOP_HEIGHT_MAX_PX, "{span}: {height}");
            assert!(
                height >= (span * HOP_LONG_RISE).min(HOP_HEIGHT_MAX_PX),
                "{span}: never flatter than a long hop: {height}"
            );
            // No flat band: below the cap a longer hop rises strictly higher.
            if height < HOP_HEIGHT_MAX_PX {
                assert!(arc_height(span + 1.0) > height, "{span}: {height}");
            }
        }
        // A phone's longest trip, capture box to top row, in two hops: each is a
        // 370 px hop that rises about twice the height where a short hop stops
        // rising — the old cap that made them read flat.
        let phone_hop = 740.0 / f64::from(HOP_MAX);
        let near = arc_height(phone_hop);
        assert!(
            (2.0 * HOP_HEIGHT_PX..2.0 * HOP_HEIGHT_PX + 2.0).contains(&near),
            "{near}"
        );
        // Long enough hops all top out at the cap itself.
        assert_eq!(arc_height(1000.0), HOP_HEIGHT_MAX_PX);
        assert_eq!(arc_height(2000.0), HOP_HEIGHT_MAX_PX);
    }

    #[test]
    fn the_impact_is_how_fast_he_falls_just_before_touching_down() {
        for to in [(60.0, 0.0), (240.0, 90.0), (900.0, 400.0)] {
            let hop = Hop::plan((0.0, 0.0), to).expect("far enough to hop");
            let touch_down = (hop.hops() - 1) * HOP_MS + (TOUCH_DOWN * f64::from(HOP_MS)) as u32;
            let (before, just_before) = (touch_down - 2, touch_down - 1);
            let fall = (hop.at(to, before).lift - hop.at(to, just_before).lift) * 1000.0;
            let impact = hop.impact(to);
            assert!(
                (fall - impact).abs() < impact * 0.05,
                "to {to:?}: falls at {fall:.0} px/s, impact says {impact:.0}"
            );
        }
    }

    #[test]
    fn he_lands_on_a_spot_that_moved_while_he_hopped() {
        let hop = Hop::plan(NEST, (400.0, 40.0)).expect("a hop");
        let moved = (400.0, -200.0);
        assert_eq!(hop.at(moved, hop.ms()).pos, moved);
    }
}
