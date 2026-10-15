//! Rambo's voice: the text-to-speech half of what he says.
//!
//! Voice and bubble are one thing (docs/DESIGN-DIRECTION.md 3): a line appears in
//! the paper bubble when his voice starts and goes when his voice is done. *What*
//! he says is not decided here Ã¢â‚¬â€ the brain decides that, and this only carries it.
//!
//! Two rules keep this from talking over the page: the gear menu's sound mute and
//! Rambo's own toggle both stop the voice, and he never speaks while the
//! microphone is listening, because his own voice would come back as a transcript.
//!
//! The rules are pure functions above the browser calls, so the behaviour is unit
//! tested without a browser; only the handle to `speechSynthesis` needs one.

use std::cell::RefCell;
use std::rc::Rc;

use crate::settings::SoundPreset;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

/// A little brisker than the browser default, so Rambo reads as a small cheerful pen
/// rather than a newsreader.
pub const RATE: f32 = 1.1;
/// Rambo is a young boy (ATD-206). A man's voice is raised this much (0 to 2, 1 is the
/// voice's own) so it reads as a boy's; safe range 1.3 to 1.7, higher turns cartoonish.
pub const PITCH: f32 = 1.5;
/// A voice that is already a boy's keeps nearly its own pitch. Safe range 1.0 to 1.2.
pub const PITCH_BOY: f32 = 1.05;

/// How long the bubble stays after his voice stops.
pub const BUBBLE_AFTER_VOICE_MS: u32 = 800;
/// With no voice the bubble is the whole message, so it holds on its own beat.
pub const BUBBLE_ALONE_MS: u32 = 1500;

/// How long the bubble holds once a line is dealt with.
///
/// A line with a voice ends when the voice does, plus a beat; a line with no voice
/// is over as soon as the bubble appears, so it holds longer — it is the only
/// channel the line has.
pub fn bubble_ms(spoke: bool) -> u32 {
    if spoke {
        BUBBLE_AFTER_VOICE_MS
    } else {
        BUBBLE_ALONE_MS
    }
}

/// What to run when a line has finished being spoken, so the bubble can go.
type OnEnd = Rc<RefCell<Option<Box<dyn Fn()>>>>;

/// Whether a line is heard as well as shown. Both switches are off-switches: the
/// gear menu's quiet desk, and Rambo's own `Rambo talks` toggle.
pub fn audible(sound: SoundPreset, rambl_talks: bool) -> bool {
    sound.allows_voice() && rambl_talks
}

/// Voices that are a boy's: Apple's Junior. Names are matched as whole words.
const BOY: [&str; 1] = ["Junior"];

/// Men's English voices, the younger-sounding first, for when no boy's voice is
/// offered: pitched up they read as a boy. "Male" catches the browsers that name a
/// voice by it ("Google UK English Male"). Any other English voice is acceptable;
/// these are preferences, not a whitelist.
const MALE: [&str; 10] = [
    "Guy", "Andrew", "Eric", "Aaron", "Alex", "Daniel", "Mark", "David", "George", "Male",
];

/// Whether `name` holds `word` as a whole word, in any case: "Female" is not "Male".
fn names(name: &str, word: &str) -> bool {
    name.split(|c: char| !c.is_alphanumeric())
        .any(|part| part.eq_ignore_ascii_case(word))
}

/// Whether the voice called `name` is a boy's.
fn is_boy(name: &str) -> bool {
    BOY.iter().any(|boy| names(name, boy))
}

/// Whether the voice called `name` is one he keeps once found: a boy's or a man's. Any
/// other voice was only the best offered so far, and a browser that fills its list in
/// late (Chrome) may still offer a better one.
fn is_kept(name: &str) -> bool {
    BOY.iter().chain(&MALE).any(|wanted| names(name, wanted))
}

/// The pitch to speak with in the voice called `name` (`None`: the browser's own
/// choice): a boy's voice near its own, any other raised so he sounds like a boy.
pub fn pitch_for(name: Option<&str>) -> f32 {
    if name.is_some_and(is_boy) {
        PITCH_BOY
    } else {
        PITCH
    }
}

/// One voice for the whole page: a boy's English voice if one is offered, else a
/// man's, else any English one.
///
/// `offered` is each voice's `(name, language)` as the browser reports it. Chrome
/// fills `getVoices()` in after the page loads, so this is called again until it
/// finds something; `None` means "let the browser choose", which every engine
/// handles and which is never a failure.
pub fn pick_voice<'a>(offered: &'a [(&'a str, &'a str)]) -> Option<&'a (&'a str, &'a str)> {
    let english = || {
        offered
            .iter()
            .filter(|(_, language)| language.to_ascii_lowercase().starts_with("en"))
    };
    BOY.iter()
        .chain(&MALE)
        .find_map(|wanted| english().find(|(name, _)| names(name, wanted)))
        .or_else(|| english().next())
}

/// The live handle to the browser's speech synthesiser. Only the browser ever
/// builds one; everywhere else the singleton stays empty and the bubble carries
/// the line on its own.
struct Speaker {
    synthesis: web_sys::SpeechSynthesis,
    /// The best voice offered so far: picked again before each line until it is one
    /// he keeps (`is_kept`).
    voice: Option<web_sys::SpeechSynthesisVoice>,
    /// Chrome drops an utterance that nothing references, so it is held here.
    speaking: Option<web_sys::SpeechSynthesisUtterance>,
    on_end: OnEnd,
    _on_end: Closure<dyn FnMut()>,
}

impl Speaker {
    fn new() -> Option<Self> {
        let synthesis = web_sys::window()?.speech_synthesis().ok()?;
        let on_end: OnEnd = Rc::new(RefCell::new(None));
        let read = on_end.clone();
        // One dispatch for the whole page: every utterance points its `onend`
        // here, and this calls whichever line is waiting.
        let on_end_closure = Closure::wrap(Box::new(move || {
            if let Some(ended) = read.borrow_mut().take() {
                ended();
            }
        }) as Box<dyn FnMut()>);
        Some(Self {
            synthesis,
            voice: None,
            speaking: None,
            on_end,
            _on_end: on_end_closure,
        })
    }

    /// The voice to speak the next line with. Chrome populates the list late, so an
    /// empty list is "not yet", not "none", and a woman's voice found early is only a
    /// stand-in: the list is read again before each line until a boy's or a man's
    /// voice is found, and that one is kept.
    fn voice(&mut self) -> Option<web_sys::SpeechSynthesisVoice> {
        let settled = self
            .voice
            .as_ref()
            .is_some_and(|voice| is_kept(&voice.name()));
        if !settled {
            let listed = self.synthesis.get_voices();
            let voices: Vec<web_sys::SpeechSynthesisVoice> = (0..listed.length())
                .filter_map(|index| listed.get(index).dyn_into().ok())
                .collect();
            let names = voices
                .iter()
                .map(|voice| (voice.name(), voice.lang()))
                .collect::<Vec<_>>();
            let offered = names
                .iter()
                .map(|(name, language)| (name.as_str(), language.as_str()))
                .collect::<Vec<_>>();
            let picked = pick_voice(&offered)
                .and_then(|(name, _)| voices.iter().find(|voice| voice.name() == *name).cloned());
            if picked.is_some() {
                self.voice = picked;
            }
        }
        self.voice.clone()
    }

    /// Speak `line`, calling `on_end` when the voice finishes. A new line always
    /// cuts the last one off: two overlapping voices read as a glitch.
    fn speak(&mut self, line: &str, on_end: impl Fn() + 'static) {
        self.stop();
        let Ok(utterance) = web_sys::SpeechSynthesisUtterance::new_with_text(line) else {
            return;
        };
        let voice = self.voice();
        if let Some(voice) = &voice {
            utterance.set_voice(Some(voice));
        }
        utterance.set_rate(RATE);
        utterance.set_pitch(pitch_for(voice.map(|voice| voice.name()).as_deref()));
        utterance.set_onend(Some(self._on_end.as_ref().unchecked_ref()));
        *self.on_end.borrow_mut() = Some(Box::new(on_end));
        self.synthesis.speak(&utterance);
        self.speaking = Some(utterance);
    }

    /// Cut him off. Called when the microphone starts listening, so his own voice
    /// cannot come back as a transcript.
    fn stop(&mut self) {
        self.synthesis.cancel();
        self.speaking = None;
        if let Some(ended) = self.on_end.borrow_mut().take() {
            ended();
        }
    }
}

thread_local! {
    /// The synthesiser for the whole page, built on first use.
    static SPEAKER: RefCell<Option<Speaker>> = const { RefCell::new(None) };
}

/// Speak `line` now, calling `on_end` when the voice finishes.
///
/// Returns whether there is a voice at all: `false` means there is nothing to
/// hear and the caller should time the bubble on its own. Every engine can go
/// quiet here Ã¢â‚¬â€ no synthesiser, muted, the toggle off, a line that arrived while
/// the microphone was listening Ã¢â‚¬â€ and none of those are errors.
pub fn say(line: &str, on_end: impl Fn() + 'static) -> bool {
    SPEAKER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Speaker::new();
        }
        match slot.as_mut() {
            Some(speaker) => {
                speaker.speak(line, on_end);
                true
            }
            None => false,
        }
    })
}

/// Stop mid-sentence, if he is speaking.
pub fn cancel() {
    SPEAKER.with(|slot| {
        if let Some(speaker) = slot.borrow_mut().as_mut() {
            speaker.stop();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        PITCH, PITCH_BOY, SoundPreset, audible, bubble_ms, is_kept, pick_voice, pitch_for,
    };

    #[test]
    fn either_switch_stops_him_and_they_are_alone_enough() {
        assert!(audible(SoundPreset::Touch, true), "both on: he speaks");
        assert!(
            audible(SoundPreset::Music, true),
            "music still lets him speak"
        );
        assert!(
            !audible(SoundPreset::Quiet, true),
            "a quiet desk: he does not"
        );
        assert!(!audible(SoundPreset::Off, true), "off: he does not");
        assert!(
            !audible(SoundPreset::Touch, false),
            "Rambo talks off: he does not"
        );
        assert!(!audible(SoundPreset::Quiet, false), "both off: he does not");
    }

    #[test]
    fn the_bubble_outlives_the_voice_and_stands_alone_without_one() {
        // A beat after the last word, so the bubble does not vanish under it.
        assert_eq!(bubble_ms(true), 800);
        // With no voice the bubble is the only channel, so it holds longer.
        assert!(bubble_ms(false) > bubble_ms(true));
    }

    #[test]
    fn only_a_boys_or_a_mans_voice_is_kept_so_a_late_list_can_still_win() {
        // Chrome lists its voices late: a woman's voice found first is picked again
        // before the next line, so Junior or a man still wins once listed.
        assert!(is_kept("Junior"));
        assert!(is_kept("Microsoft David - English (United States)"));
        assert!(is_kept("Google UK English Male"));
        assert!(!is_kept("Google UK English Female"));
        assert!(!is_kept("Samantha"));
        assert!(!is_kept("Some Reader"));
        // What is kept is always what the full list would pick first among its kind.
        let early = [("Samantha", "en-US")];
        let late = [("Samantha", "en-US"), ("Daniel", "en-GB")];
        assert!(!is_kept(pick_voice(&early).unwrap().0));
        assert!(is_kept(pick_voice(&late).unwrap().0));
    }

    #[test]
    fn a_boys_english_voice_beats_every_other() {
        let offered = [
            ("Samantha", "en-US"),
            ("Daniel", "en-GB"),
            ("Junior", "en-US"),
        ];
        assert_eq!(pick_voice(&offered), Some(&("Junior", "en-US")));
    }

    #[test]
    fn with_no_boy_a_mans_voice_beats_a_womans() {
        // Edge's girls (Ana, Maisie) and women are passed over for a man pitched up.
        let edge = [
            (
                "Microsoft Ana Online (Natural) - English (United States)",
                "en-US",
            ),
            (
                "Microsoft Aria Online (Natural) - English (United States)",
                "en-US",
            ),
            (
                "Microsoft Guy Online (Natural) - English (United States)",
                "en-US",
            ),
        ];
        assert_eq!(pick_voice(&edge), Some(&edge[2]));
        let windows = [
            ("Microsoft Zira - English (United States)", "en-US"),
            ("Microsoft David - English (United States)", "en-US"),
        ];
        assert_eq!(pick_voice(&windows), Some(&windows[1]));
        let chrome = [
            ("Google UK English Female", "en-GB"),
            ("Google UK English Male", "en-GB"),
        ];
        assert_eq!(pick_voice(&chrome), Some(&chrome[1]));
    }

    #[test]
    fn a_name_counts_only_as_a_whole_word() {
        // "Female" holds "male", "Guyana" holds "Guy": neither is a man's voice.
        let offered = [
            ("Google US English Female", "en-US"),
            ("Reader (Guyana)", "en-GY"),
            ("Samantha", "en-US"),
        ];
        assert_eq!(pick_voice(&offered), Some(&offered[0]));
        assert_eq!(pitch_for(Some("Juniors Reader")), PITCH);
    }

    #[test]
    fn he_sounds_like_a_boy_whichever_voice_speaks() {
        // A boy's voice is near its own pitch; a man's, a woman's, or the browser's
        // own choice is raised.
        assert_eq!(pitch_for(Some("Junior")), PITCH_BOY);
        assert_eq!(
            pitch_for(Some("Microsoft David - English (United States)")),
            PITCH
        );
        assert_eq!(pitch_for(Some("Samantha")), PITCH);
        assert_eq!(pitch_for(None), PITCH);
    }

    #[test]
    fn an_acceptable_english_voice_beats_the_browser_default() {
        let offered = [("Some Reader", "en-AU"), ("Ingrid", "de-DE")];
        assert_eq!(pick_voice(&offered), Some(&("Some Reader", "en-AU")));
    }

    #[test]
    fn no_english_voice_falls_back_to_the_browser_rather_than_a_foreign_one() {
        // French and Japanese readers are never better than no choice at all.
        assert_eq!(pick_voice(&[("Amelie", "fr-FR"), ("Kyoko", "ja-JP")]), None);
        assert_eq!(pick_voice(&[]), None);
    }

    #[test]
    fn a_wanted_voice_in_another_language_is_never_picked() {
        // A voice called "Daniel" speaking German must not win on the name.
        let offered = [("Daniel", "de-DE"), ("Samantha", "en-GB")];
        assert_eq!(pick_voice(&offered), Some(&("Samantha", "en-GB")));
    }

    #[test]
    fn the_browser_reports_languages_with_mixed_case_and_regions() {
        assert_eq!(
            pick_voice(&[("Samantha", "en-US"), ("Daniel", "EN-ie")]),
            Some(&("Daniel", "EN-ie"))
        );
    }
}
