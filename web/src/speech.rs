use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, Function, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{SpeechRecognition, Window};

/// The one calm note for a page that cannot listen at all — no speech API, or the
/// page is not secure (a phone on plain-http LAN). Shown instead of an error, while
/// Rambo keeps his listening pose (docs/REDESIGN.md: no scary errors).
pub const UNAVAILABLE_NOTE: &str =
    "Rambl can't listen in this browser. Keep typing and Rambo will write it down.";

/// Where one listening session is. Pure logic, unit-tested without a browser:
/// Chrome throws `InvalidStateError` when `start()` runs twice on a session, so
/// the machine gates what the browser may be asked to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum State {
    #[default]
    Idle,
    /// `start()` succeeded; Chrome may still be showing the microphone prompt.
    Starting,
    /// Audio is flowing (`onaudiostart` or the first result).
    Listening,
    /// `stop()` was asked for; waiting for Chrome's `onend`.
    Stopping,
}

/// Something that happened to a session while it was out in the browser.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Event {
    StartRequested,
    AudioStarted,
    Heard,
    StopRequested,
    Errored,
    Ended,
}

/// The four-state machine for one recognition session. Events a state cannot
/// take are ignored: a second `start()`, a stray `onend`, an error after the end.
#[derive(Debug, Default)]
struct Machine(State);

impl Machine {
    /// `start()` is only legal from `Idle` — anywhere else Chrome throws
    /// `InvalidStateError`.
    fn can_start(&self) -> bool {
        self.0 == State::Idle
    }

    /// `stop()` is only meaningful before Chrome's `onend` was seen.
    fn can_stop(&self) -> bool {
        matches!(self.0, State::Starting | State::Listening)
    }

    /// Apply `event`, ignoring the ones a state cannot take, and report where
    /// the machine ended up.
    fn handle(&mut self, event: Event) -> State {
        self.0 = match (self.0, event) {
            (State::Idle, Event::StartRequested) => State::Starting,
            (State::Starting, Event::AudioStarted | Event::Heard) => State::Listening,
            (State::Starting | State::Listening, Event::StopRequested | Event::Errored) => {
                State::Stopping
            }
            (_, Event::Ended) => State::Idle,
            _ => self.0,
        };
        self.0
    }
}

/// The calm, one-line note for a recognition error, or `None` to end quietly.
/// `aborted` is Chrome handing the microphone to a newer session, and unknown
/// codes say nothing the user could act on — neither is the user's problem.
fn calm_note(code: &str) -> Option<&'static str> {
    match code {
        "not-allowed" | "service-not-allowed" => Some("Microphone permission was denied."),
        "audio-capture" => Some("No microphone was found."),
        "no-speech" => Some("No speech was detected."),
        "network" => Some("Speech service could not be reached."),
        _ => None,
    }
}

pub struct Session {
    recognition: SpeechRecognition,
    machine: Rc<RefCell<Machine>>,
    _on_audio_start: Closure<dyn FnMut()>,
    _on_result: Closure<dyn FnMut(JsValue)>,
    _on_error: Closure<dyn FnMut(JsValue)>,
    _on_end: Closure<dyn FnMut()>,
}

impl Session {
    pub fn start(
        text: Rc<RefCell<String>>,
        set_text: impl Fn(String) + Copy + 'static,
        set_listening: impl Fn(bool) + Copy + 'static,
        set_error: impl Fn(Option<String>) + Copy + 'static,
    ) -> Result<Self, String> {
        let browser_window = web_sys::window().ok_or_else(|| UNAVAILABLE_NOTE.to_owned())?;
        let recognition = recognition(&browser_window)?;
        recognition.set_interim_results(true);
        let _ = recognition.set_continuous(false);

        let machine = Rc::new(RefCell::new(Machine::default()));
        let heard = Rc::new(RefCell::new(false));

        let machine_for_audio = machine.clone();
        let on_audio_start = Closure::wrap(Box::new(move || {
            machine_for_audio.borrow_mut().handle(Event::AudioStarted);
        }) as Box<dyn FnMut()>);

        let machine_for_result = machine.clone();
        let heard_for_result = heard.clone();
        let on_result = Closure::wrap(Box::new(move |event: JsValue| {
            let results = Reflect::get(&event, &JsValue::from_str("results"))
                .ok()
                .map(|value| Array::from(&value));
            let Some(results) = results else {
                return;
            };

            let mut transcript = String::new();
            for index in 0..results.length() {
                let result = results.get(index);
                let alternative = Reflect::get(&result, &JsValue::from(0)).ok();
                let Some(alternative) = alternative else {
                    continue;
                };
                let phrase = Reflect::get(&alternative, &JsValue::from_str("transcript"))
                    .ok()
                    .and_then(|value| value.as_string())
                    .unwrap_or_default();
                if !phrase.is_empty() {
                    if !transcript.is_empty() {
                        transcript.push(' ');
                    }
                    transcript.push_str(&phrase);
                }
            }
            if transcript.is_empty() {
                return;
            }
            machine_for_result.borrow_mut().handle(Event::Heard);
            *heard_for_result.borrow_mut() = true;
            *text.borrow_mut() = transcript.clone();
            set_text(transcript);
        }) as Box<dyn FnMut(JsValue)>);

        let set_error_for_error = set_error;
        let set_listening_for_error = set_listening;
        let machine_for_error = machine.clone();
        let on_error = Closure::wrap(Box::new(move |event: JsValue| {
            let code = Reflect::get(&event, &JsValue::from_str("error"))
                .ok()
                .and_then(|value| value.as_string())
                .unwrap_or_default();
            machine_for_error.borrow_mut().handle(Event::Errored);
            if let Some(note) = calm_note(&code) {
                set_error_for_error(Some(note.to_owned()));
            }
            set_listening_for_error(false);
        }) as Box<dyn FnMut(JsValue)>);

        let set_error_for_end = set_error;
        let set_listening_for_end = set_listening;
        let machine_for_end = machine.clone();
        let on_end = Closure::wrap(Box::new(move || {
            machine_for_end.borrow_mut().handle(Event::Ended);
            if !*heard.borrow() {
                set_error_for_end(Some("No speech was detected.".to_owned()));
            }
            set_listening_for_end(false);
        }) as Box<dyn FnMut()>);

        recognition.set_onaudiostart(Some(on_audio_start.as_ref().unchecked_ref()));
        recognition.set_onresult(Some(on_result.as_ref().unchecked_ref()));
        recognition.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        recognition.set_onend(Some(on_end.as_ref().unchecked_ref()));
        debug_assert!(
            machine.borrow().can_start(),
            "a session is started once, from Idle"
        );
        recognition
            .start()
            .map_err(|_| UNAVAILABLE_NOTE.to_owned())?;
        machine.borrow_mut().handle(Event::StartRequested);

        Ok(Self {
            recognition,
            machine,
            _on_audio_start: on_audio_start,
            _on_result: on_result,
            _on_error: on_error,
            _on_end: on_end,
        })
    }

    /// Ask Chrome to finish listening; a no-op unless audio may be flowing,
    /// so a plain click on Rambo (grab, never started) cannot trip Chrome.
    pub fn stop(&self) {
        if self.machine.borrow().can_stop() {
            self.machine.borrow_mut().handle(Event::StopRequested);
            self.recognition.stop();
        }
    }
}

/// Whether listening can work here: a browser with the API on a secure page.
/// Chrome keeps `webkitSpeechRecognition` visible on plain-http pages but never
/// starts it there, so the page must be secure too (127.0.0.1 is).
pub fn supported() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    if !window.is_secure_context() {
        return false;
    }
    constructor(&window).is_some()
}

fn recognition(window: &Window) -> Result<SpeechRecognition, String> {
    let constructor = constructor(window).ok_or_else(|| UNAVAILABLE_NOTE.to_owned())?;
    // DOM constructors must be run with `new`. A bare call (`Function::call0`)
    // throws "Please use the 'new' operator" in current Chrome/Edge, which is
    // what made every grab report "Could not start speech recognition".
    let value =
        Reflect::construct(&constructor, &Array::new()).map_err(|_| UNAVAILABLE_NOTE.to_owned())?;
    value.dyn_into().map_err(|_| UNAVAILABLE_NOTE.to_owned())
}

fn constructor(window: &Window) -> Option<Function> {
    ["SpeechRecognition", "webkitSpeechRecognition"]
        .iter()
        .find_map(|name| {
            Reflect::get(window.as_ref(), &JsValue::from_str(name))
                .ok()
                .and_then(|value| value.dyn_into().ok())
        })
}

#[cfg(test)]
mod tests {
    use super::{Event, Machine, State, calm_note};

    /// A machine walked into `state` through its real events, every step checked.
    fn machine_in(state: State) -> Machine {
        let mut machine = Machine::default();
        let reached = match state {
            State::Idle => State::Idle,
            State::Starting => machine.handle(Event::StartRequested),
            State::Listening => {
                machine.handle(Event::StartRequested);
                machine.handle(Event::AudioStarted)
            }
            State::Stopping => {
                machine.handle(Event::StartRequested);
                machine.handle(Event::AudioStarted);
                machine.handle(Event::StopRequested)
            }
        };
        assert_eq!(reached, state);
        machine
    }

    #[test]
    fn hold_talk_release_walks_the_whole_machine() {
        let mut machine = Machine::default();
        assert!(machine.can_start());
        assert_eq!(machine.handle(Event::StartRequested), State::Starting);
        assert_eq!(machine.handle(Event::AudioStarted), State::Listening);
        assert_eq!(machine.handle(Event::StopRequested), State::Stopping);
        assert_eq!(machine.handle(Event::Ended), State::Idle);
        assert!(machine.can_start());
    }

    #[test]
    fn the_microphone_prompt_holds_starting_until_audio() {
        // Chrome asks for permission after start(); grant or denial both arrive
        // as events before any audio.
        let mut granted = machine_in(State::Starting);
        assert_eq!(granted.handle(Event::Heard), State::Listening);

        let mut denied = machine_in(State::Starting);
        assert_eq!(denied.handle(Event::Errored), State::Stopping);
        assert_eq!(denied.handle(Event::Ended), State::Idle);
    }

    #[test]
    fn start_is_only_legal_from_idle() {
        assert!(Machine::default().can_start());
        for state in [State::Starting, State::Listening, State::Stopping] {
            assert!(!machine_in(state).can_start());
        }
    }

    #[test]
    fn stop_is_only_legal_before_the_end() {
        assert!(!machine_in(State::Idle).can_stop());
        assert!(!machine_in(State::Stopping).can_stop());
        for state in [State::Starting, State::Listening] {
            assert!(machine_in(state).can_stop());
        }
    }

    #[test]
    fn stray_events_do_not_move_the_machine() {
        let mut never_started = Machine::default();
        assert_eq!(never_started.handle(Event::Ended), State::Idle);
        assert_eq!(never_started.handle(Event::Errored), State::Idle);
        assert_eq!(never_started.handle(Event::Heard), State::Idle);

        // Chrome throws InvalidStateError on a second start(); the machine absorbs it.
        let mut double_start = machine_in(State::Starting);
        assert_eq!(double_start.handle(Event::StartRequested), State::Starting);

        // An error after the end changes nothing.
        let mut late_error = machine_in(State::Stopping);
        assert_eq!(late_error.handle(Event::Errored), State::Stopping);
        assert_eq!(late_error.handle(Event::Ended), State::Idle);

        // More audio while stopping does not restart anything.
        let mut stopped = machine_in(State::Stopping);
        assert_eq!(stopped.handle(Event::Heard), State::Stopping);
    }

    #[test]
    fn calm_notes_map_the_actionable_errors() {
        assert_eq!(
            calm_note("not-allowed"),
            Some("Microphone permission was denied.")
        );
        assert_eq!(
            calm_note("service-not-allowed"),
            Some("Microphone permission was denied.")
        );
        assert_eq!(calm_note("audio-capture"), Some("No microphone was found."));
        assert_eq!(calm_note("no-speech"), Some("No speech was detected."));
        assert_eq!(
            calm_note("network"),
            Some("Speech service could not be reached.")
        );
    }

    #[test]
    fn unactionable_errors_end_quietly() {
        // "aborted" is Chrome handing the microphone to a newer session.
        assert_eq!(calm_note("aborted"), None);
        assert_eq!(calm_note("bad-grammar"), None);
        assert_eq!(calm_note(""), None);
    }
}
