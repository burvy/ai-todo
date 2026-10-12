use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, Function, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{SpeechRecognition, Window};

pub struct Session {
    recognition: SpeechRecognition,
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
        let browser_window =
            web_sys::window().ok_or_else(|| "Speech recognition is not available.".to_string())?;
        let recognition = recognition(&browser_window)?;
        recognition.set_interim_results(true);
        let _ = recognition.set_continuous(false);

        let heard = Rc::new(RefCell::new(false));
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
            *heard_for_result.borrow_mut() = true;
            *text.borrow_mut() = transcript.clone();
            set_text(transcript);
        }) as Box<dyn FnMut(JsValue)>);

        let set_error_for_error = set_error;
        let set_listening_for_error = set_listening;
        let on_error = Closure::wrap(Box::new(move |event: JsValue| {
            let code = Reflect::get(&event, &JsValue::from_str("error"))
                .ok()
                .and_then(|value| value.as_string())
                .unwrap_or_default();
            let message = match code.as_str() {
                "not-allowed" | "service-not-allowed" => {
                    "Microphone permission was denied.".to_string()
                }
                "no-speech" => "No speech was detected.".to_string(),
                _ => "Speech recognition failed.".to_string(),
            };
            set_error_for_error(Some(message));
            set_listening_for_error(false);
        }) as Box<dyn FnMut(JsValue)>);

        let set_error_for_end = set_error;
        let set_listening_for_end = set_listening;
        let on_end = Closure::wrap(Box::new(move || {
            if !*heard.borrow() {
                set_error_for_end(Some("No speech was detected.".to_string()));
            }
            set_listening_for_end(false);
        }) as Box<dyn FnMut()>);

        recognition.set_onresult(Some(on_result.as_ref().unchecked_ref()));
        recognition.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        recognition.set_onend(Some(on_end.as_ref().unchecked_ref()));
        recognition
            .start()
            .map_err(|_| "Could not start speech recognition.".to_string())?;

        Ok(Self {
            recognition,
            _on_result: on_result,
            _on_error: on_error,
            _on_end: on_end,
        })
    }

    pub fn stop(&self) {
        self.recognition.stop();
    }
}

pub fn supported() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    constructor(&window).is_some()
}

fn recognition(window: &Window) -> Result<SpeechRecognition, String> {
    let constructor = constructor(window).ok_or_else(|| {
        "Speech recognition is not available in this browser.".to_string()
    })?;
    let value = constructor
        .call0(&JsValue::from(window))
        .map_err(|_| "Could not start speech recognition.".to_string())?;
    value
        .dyn_into()
        .map_err(|_| "Could not start speech recognition.".to_string())
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
