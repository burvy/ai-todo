//! Rambo's little sounds: on by default, gentle, and only ever on the
//! movements he is actually making.
//!
//! The cue table and the mute rule are pure and unit-tested; playing them is
//! the DOM side. A looping cue runs for exactly as long as the movement it
//! belongs to, which is what "the pen scratch is synced to writing" means
//! here: the nib moves for as long as the scratch lasts and not a frame more.

/// One of the sounds this crate owns. `assets/sounds/<asset()>.mp3` exists for
/// every cue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// The nib on paper. Looped for as long as the pen writes a row.
    Write,
    /// A title crossed out.
    Scribble,
}

impl Cue {
    /// The file under `assets/sounds/` this cue plays.
    pub fn asset(self) -> &'static str {
        match self {
            Cue::Write => "write",
            Cue::Scribble => "scribble",
        }
    }

    /// How loud the cue plays, 0..=1. Rambo is a pen, not a firework.
    pub fn volume(self) -> f64 {
        match self {
            Cue::Write => 0.28,
            Cue::Scribble => 0.42,
        }
    }
}

/// Where the gear menu's mute switch keeps the flag is
/// [`settings::Settings::sounds`](crate::settings::Settings::sounds); this
/// module only asks it whether to play.

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

/// Is the page muted right now? The gear menu may have flipped this since the
/// last sound, so it is read fresh rather than cached.
#[cfg(target_arch = "wasm32")]
pub fn muted() -> bool {
    !crate::settings::load().sounds
}

/// Off the browser there is nothing playing, so nothing is muted.
#[cfg(not(target_arch = "wasm32"))]
pub fn muted() -> bool {
    false
}

/// A ready-to-play element for `cue`, or `None` when the gear menu has the
/// sounds off.
#[cfg(target_arch = "wasm32")]
fn player(cue: Cue) -> Option<web_sys::HtmlAudioElement> {
    use wasm_bindgen::JsCast;
    if muted() {
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
        for cue in [Cue::Write, Cue::Scribble] {
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
}
