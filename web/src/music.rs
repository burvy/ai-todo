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

/// How loud a stretch of the music is: the root mean square of its samples (-1 to 1),
/// 0 for none. What Rambo hears to dance to (`engine::Dance`).
pub fn loudness(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    (sum / samples.len() as f64).sqrt()
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
    /// Where Rambo listens: the music runs through this analyser on its way out, and the
    /// context it lives in, resumed on a press like the music itself.
    static EAR: std::cell::RefCell<Option<(web_sys::AudioContext, web_sys::AnalyserNode)>> =
        const { std::cell::RefCell::new(None) };
}

/// How loud the music playing now is (0 to 1), or `None` when none plays.
#[cfg(target_arch = "wasm32")]
pub fn level() -> Option<f64> {
    let playing = PLAYER.with(|slot| slot.borrow().as_ref().is_some_and(|audio| !audio.paused()));
    if !playing {
        return None;
    }
    EAR.with(|ear| {
        let ear = ear.borrow();
        let (_, analyser) = ear.as_ref()?;
        let mut samples = vec![0.0_f32; analyser.fft_size() as usize];
        analyser.get_float_time_domain_data(&mut samples);
        Some(loudness(&samples))
    })
}

/// Off the browser no music plays.
#[cfg(not(target_arch = "wasm32"))]
pub fn level() -> Option<f64> {
    None
}

/// Runs `audio` through an analyser on its way out, so Rambo can hear it. Without Web
/// Audio the music still plays straight out, and he simply does not dance.
#[cfg(target_arch = "wasm32")]
fn listen_to(audio: &web_sys::HtmlAudioElement) {
    let Ok(context) = web_sys::AudioContext::new() else {
        return;
    };
    let (Ok(source), Ok(analyser)) = (
        context.create_media_element_source(audio),
        context.create_analyser(),
    ) else {
        return;
    };
    analyser.set_fft_size(1024);
    if source.connect_with_audio_node(&analyser).is_err()
        || analyser
            .connect_with_audio_node(&context.destination())
            .is_err()
    {
        return;
    }
    EAR.with(|ear| *ear.borrow_mut() = Some((context, analyser)));
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
        // A press is when a browser lets the ear's context run, as it lets music play.
        EAR.with(|ear| {
            if let Some((context, _)) = ear.borrow().as_ref() {
                let _ = context.resume();
            }
        });
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
        listen_to(&audio);
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
    fn loudness_is_the_root_mean_square_of_the_samples() {
        assert_eq!(loudness(&[]), 0.0);
        assert_eq!(loudness(&[0.0; 64]), 0.0);
        assert!((loudness(&[0.5, -0.5, 0.5, -0.5]) - 0.5).abs() < 1e-9);
        // A sine at full scale is 1/sqrt(2) loud, whatever its pitch.
        let sine: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.37).sin()).collect();
        assert!((loudness(&sine) - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.01);
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
