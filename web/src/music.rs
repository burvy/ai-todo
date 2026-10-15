//! The calm piano under the "Everything + music" Sound preset.
//!
//! The playlist and the rule for when it plays are pure and unit-tested; playing it is
//! the DOM side. A browser will not start sound before a gesture, so `sync` is called
//! from the Sound choice itself and again on every press until the music has started.

use crate::settings::SoundPreset;

/// The recordings, as `assets/music/<name>.mp3`, played in order and round again.
pub const TRACKS: [&str; 2] = ["nocturne-op15-no1", "gymnopedie-no1"];

/// Only the Music preset plays them.
pub fn wanted(preset: SoundPreset) -> bool {
    preset == SoundPreset::Music
}

/// The track after `index`, wrapping to the first after the last.
pub fn next_track(index: usize) -> usize {
    (index + 1) % TRACKS.len()
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// The one audio element, kept so a later call resumes it rather than layering a second.
    static PLAYER: std::cell::RefCell<Option<web_sys::HtmlAudioElement>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(target_arch = "wasm32")]
fn track_url(index: usize) -> String {
    format!("assets/music/{}.mp3", TRACKS[index])
}

/// Make the music match `preset`: start or resume it under Music, cut it otherwise.
/// A browser that refuses to start before a gesture rejects the play promise; that is
/// dropped here and the next press calls this again.
#[cfg(target_arch = "wasm32")]
pub fn sync(preset: SoundPreset) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    PLAYER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if !wanted(preset) {
            if let Some(audio) = slot.take() {
                let _ = audio.pause();
            }
            return;
        }
        if let Some(audio) = slot.as_ref() {
            if audio.paused() {
                let _ = audio.play();
            }
            return;
        }
        let Some(audio) = leptos::prelude::window()
            .document()
            .and_then(|document| document.create_element("audio").ok())
            .and_then(|element| element.dyn_into::<web_sys::HtmlAudioElement>().ok())
        else {
            return;
        };
        audio.set_src(&track_url(0));
        audio.set_volume(crate::tuning::MUSIC_VOLUME);
        let next = std::rc::Rc::new(std::cell::Cell::new(0_usize));
        let on_end = {
            let audio = audio.clone();
            Closure::<dyn FnMut()>::new(move || {
                next.set(next_track(next.get()));
                audio.set_src(&track_url(next.get()));
                let _ = audio.play();
            })
        };
        audio.set_onended(Some(on_end.as_ref().unchecked_ref()));
        // The element is kept for as long as the page lives, and so is its handler.
        on_end.forget();
        let _ = audio.play();
        *slot = Some(audio);
    });
}

/// Off the browser there is nothing to play.
#[cfg(not(target_arch = "wasm32"))]
pub fn sync(_preset: SoundPreset) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_music_preset_plays_the_music() {
        for preset in SoundPreset::ALL {
            assert_eq!(wanted(preset), preset == SoundPreset::Music, "{preset:?}");
        }
    }

    #[test]
    fn the_playlist_wraps_round() {
        assert_eq!(next_track(0), 1);
        assert_eq!(next_track(TRACKS.len() - 1), 0);
    }

    #[test]
    fn every_track_ships_and_all_of_them_stay_under_three_megabytes() {
        let total: u64 = TRACKS
            .iter()
            .map(|name| {
                std::fs::metadata(format!("assets/music/{name}.mp3"))
                    .unwrap_or_else(|_| panic!("assets/music/{name}.mp3 is missing"))
                    .len()
            })
            .sum();
        assert!(total < 3_000_000, "the music is {total} bytes");
    }
}
