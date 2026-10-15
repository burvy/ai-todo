//! The page's little sounds: gentle, and only ever on the movements that are
//! actually happening.
//!
//! What plays is chosen by the gear menu's `SoundPreset`. The cue table (`plays`)
//! is pure and unit-tested; playing a cue is the DOM side. A looping cue runs for exactly as long as the movement it
//! belongs to, which is what "the pen scratch is synced to writing" means
//! here: the nib moves for as long as the scratch lasts and not a frame more.

use crate::settings::SoundPreset;

/// One of the sounds this crate owns. `assets/sounds/<asset()>.mp3` exists for
/// every cue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// The nib on paper. Looped for as long as the pen writes a row.
    Write,
    /// A title crossed out.
    Scribble,
    /// A finger or click lands on a button.
    Tap,
    /// A page turns.
    Page,
    /// A ramble is sent.
    Send,
    /// A strip is crumpled into the bin.
    Crumple,
    /// His dry nib knocks on the nest's rim, once a tap.
    Knock,
}

impl Cue {
    /// Every cue, for the tests that check the table and the files.
    pub const ALL: [Cue; 7] = [
        Cue::Write,
        Cue::Scribble,
        Cue::Tap,
        Cue::Page,
        Cue::Send,
        Cue::Crumple,
        Cue::Knock,
    ];

    /// Sounds that answer a touch with no job of their own: what a person who asked the
    /// page for less motion has asked to lose along with it.
    fn is_ambient(self) -> bool {
        matches!(self, Cue::Tap | Cue::Page)
    }
}

/// The event-to-sound table: does `preset` play `cue`? Off plays nothing. A quiet desk keeps to the noises
/// paper and ink really make (the nib, its knock on the rim, the scribble, the page, the
/// crumple); the other
/// two answer every touch (the music of `Music` is a separate track, not a cue).
/// Reduced motion drops the cues that only decorate a movement.
pub fn plays(preset: SoundPreset, cue: Cue, reduced_motion: bool) -> bool {
    let wanted = match preset {
        SoundPreset::Off => false,
        SoundPreset::Quiet => matches!(
            cue,
            Cue::Write | Cue::Knock | Cue::Scribble | Cue::Page | Cue::Crumple
        ),
        SoundPreset::Touch | SoundPreset::Music => true,
    };
    wanted && !(reduced_motion && cue.is_ambient())
}

impl Cue {
    /// The file under `assets/sounds/` this cue plays.
    pub fn asset(self) -> &'static str {
        match self {
            Cue::Write => "write",
            Cue::Scribble => "scribble",
            Cue::Tap => "tick",
            Cue::Page => "page",
            Cue::Send => "send",
            Cue::Crumple => "crumple",
            // A small hard knock, as a button's tick is: no file of its own yet.
            Cue::Knock => "tick",
        }
    }

    /// How loud the cue plays, 0..=1. Rambo is a pen, not a firework.
    pub fn volume(self) -> f64 {
        match self {
            Cue::Write => 0.28,
            Cue::Scribble => 0.42,
            Cue::Tap => 0.2,
            Cue::Page => 0.3,
            Cue::Send => 0.35,
            Cue::Crumple => 0.4,
            Cue::Knock => 0.25,
        }
    }
}

/// A cue that keeps going until the movement it belongs to is over.
#[cfg(target_arch = "wasm32")]
pub struct Loop {
    audio: web_sys::HtmlAudioElement,
}

#[cfg(target_arch = "wasm32")]
impl Loop {
    /// Cut the sound where the nib stops.
    pub fn stop(self) {
        // A cue that has already ended has nothing left to pause; the page is
        // not told about it because the pen has moved on either way.
        let _ = self.audio.pause();
    }
}

/// A ready-to-play element for `cue`, or `None` when the gear menu's preset does not
/// play it. The preset is read fresh each time: the gear menu may have flipped it since
/// the last sound.
#[cfg(target_arch = "wasm32")]
fn player(cue: Cue) -> Option<web_sys::HtmlAudioElement> {
    use wasm_bindgen::JsCast;
    if !plays(
        crate::settings::load().sound,
        cue,
        crate::mascot::prefers_reduced_motion(),
    ) {
        return None;
    }
    let audio = leptos::prelude::window()
        .document()?
        .create_element("audio")
        .ok()?
        .dyn_into::<web_sys::HtmlAudioElement>()
        .ok()?;
    audio.set_src(&format!("assets/sounds/{}.mp3", cue.asset()));
    audio.set_preload("auto");
    audio.set_volume(cue.volume());
    Some(audio)
}

/// Play a cue once. A browser that will not play before the first gesture
/// rejects the promise, and there is nothing a user could do about it, so the
/// rejection is dropped here rather than surfaced.
#[cfg(target_arch = "wasm32")]
pub fn play(cue: Cue) {
    let Some(audio) = player(cue) else { return };
    let _ = audio.play();
}

/// Off the browser there is nothing to play.
#[cfg(not(target_arch = "wasm32"))]
pub fn play(_cue: Cue) {}

/// Start a cue looping, so a caller can keep it going for exactly as long as
/// the movement it belongs to.
#[cfg(target_arch = "wasm32")]
pub fn looping(cue: Cue) -> Option<Loop> {
    let audio = player(cue)?;
    audio.set_loop(true);
    let _ = audio.play();
    Some(Loop { audio })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cue_names_a_shipped_file() {
        assert_eq!(Cue::Write.asset(), "write");
        assert_eq!(Cue::Scribble.asset(), "scribble");
        assert_eq!(Cue::Tap.asset(), "tick");
        for cue in Cue::ALL {
            // The file really is in the tree: a renamed asset would otherwise
            // only show up as a silent animation.
            assert!(
                std::path::Path::new(&format!("assets/sounds/{}.mp3", cue.asset())).exists(),
                "assets/sounds/{}.mp3 is missing",
                cue.asset()
            );
            assert!((0.0..=1.0).contains(&cue.volume()), "{cue:?} is too loud");
            assert!(cue.volume() <= 0.5, "Rambo is gentle by default");
        }
    }

    #[test]
    fn off_plays_nothing() {
        for cue in Cue::ALL {
            assert!(!plays(SoundPreset::Off, cue, false), "{cue:?} while off");
        }
    }

    #[test]
    fn a_quiet_desk_keeps_to_paper_and_ink_and_the_others_answer_every_touch() {
        for cue in [Cue::Tap, Cue::Send] {
            assert!(
                !plays(SoundPreset::Quiet, cue, false),
                "{cue:?} on a quiet desk"
            );
        }
        for cue in [
            Cue::Write,
            Cue::Knock,
            Cue::Scribble,
            Cue::Page,
            Cue::Crumple,
        ] {
            assert!(
                plays(SoundPreset::Quiet, cue, false),
                "{cue:?} is a real noise"
            );
        }
        for cue in Cue::ALL {
            assert!(
                plays(SoundPreset::Touch, cue, false),
                "{cue:?} should answer"
            );
            assert!(
                plays(SoundPreset::Music, cue, false),
                "{cue:?} should answer"
            );
        }
    }

    #[test]
    fn reduced_motion_drops_only_the_ambient_cues() {
        for preset in SoundPreset::ALL
            .into_iter()
            .filter(|p| *p != SoundPreset::Off)
        {
            assert!(!plays(preset, Cue::Tap, true) && !plays(preset, Cue::Page, true));
            for cue in [Cue::Write, Cue::Knock, Cue::Scribble, Cue::Crumple] {
                assert!(
                    plays(preset, cue, true),
                    "{cue:?} is feedback, not decoration"
                );
            }
        }
    }
}
