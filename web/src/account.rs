//! Signing in with Google. The server says whether it has a Google client id. When it
//! does, the page loads Google's button, trades the credential Google returns for a
//! session token, and keeps that token for later requests. With no client id, nothing
//! is shown.

use gloo_net::http::Request;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// Where the session token is kept: in this browser only.
const TOKEN_KEY: &str = "rambl_token";

/// The signed-in person, as far as the page needs them.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Me {
    pub email: String,
}

#[derive(Deserialize)]
struct Session {
    token: String,
    user: Me,
}

#[derive(Deserialize)]
struct Config {
    google_client_id: Option<String>,
}

#[derive(Serialize)]
struct Credential<'a> {
    credential: &'a str,
}

thread_local! {
    static USER: RwSignal<Option<Me>> = RwSignal::new(None);
    static CLIENT_ID: RwSignal<Option<String>> = RwSignal::new(None);
    static ERROR: RwSignal<Option<String>> = RwSignal::new(None);
}

fn user() -> RwSignal<Option<Me>> {
    USER.with(|signal| *signal)
}

fn client_id() -> RwSignal<Option<String>> {
    CLIENT_ID.with(|signal| *signal)
}

fn error() -> RwSignal<Option<String>> {
    ERROR.with(|signal| *signal)
}

fn storage() -> Option<web_sys::Storage> {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()?.local_storage().ok().flatten()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

/// The session token this browser holds, if someone is signed in.
pub fn token() -> Option<String> {
    storage()?.get_item(TOKEN_KEY).ok().flatten()
}

fn store_token(token: &str) {
    if let Some(storage) = storage() {
        let _ = storage.set_item(TOKEN_KEY, token);
    }
}

fn forget_token() {
    if let Some(storage) = storage() {
        let _ = storage.remove_item(TOKEN_KEY);
    }
}

/// The server refused the token, so the session is over and this browser forgets it.
pub fn session_ended() {
    if token().is_some() {
        forget_token();
        user().set(None);
    }
}

/// Asks for the Google client id and restores a saved session. Called once, when the
/// page loads.
pub fn start() {
    wasm_bindgen_futures::spawn_local(async {
        if let Some(id) = fetch_client_id().await {
            client_id().set(Some(id.clone()));
            load_google(id);
        }
        restore().await;
    });
}

async fn fetch_client_id() -> Option<String> {
    let config: Config = Request::get("/api/v1/config")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    config.google_client_id
}

async fn restore() {
    let Some(token) = token() else { return };
    let response = Request::get("/api/v1/me")
        .header("Authorization", &format!("Bearer {token}"))
        .send()
        .await;
    match response {
        Ok(response) if response.ok() => {
            if let Ok(me) = response.json::<Me>().await {
                user().set(Some(me));
            }
        }
        Ok(response) if response.status() == 401 => session_ended(),
        _ => {}
    }
}

/// Trades a Google credential for a session. Failures are shown, not silently dropped.
pub async fn sign_in(credential: &str) {
    let response = match Request::post("/api/v1/auth/google").json(&Credential { credential }) {
        Ok(request) => request.send().await,
        Err(why) => Err(why),
    };
    match response {
        Ok(response) if response.ok() => match response.json::<Session>().await {
            Ok(session) => {
                store_token(&session.token);
                user().set(Some(session.user));
                error().set(None);
            }
            Err(why) => error().set(Some(format!("Sign-in failed: {why}"))),
        },
        Ok(response) => error().set(Some(format!(
            "Google sign-in was refused (status {})",
            response.status()
        ))),
        Err(why) => error().set(Some(format!("Could not reach the server: {why}"))),
    }
}

/// Ends this browser's session. The server is told, but the page does not wait for it.
pub fn sign_out() {
    let Some(token) = token() else { return };
    wasm_bindgen_futures::spawn_local(async move {
        let _ = Request::post("/api/v1/auth/signout")
            .header("Authorization", &format!("Bearer {token}"))
            .send()
            .await;
    });
    forget_token();
    user().set(None);
}

#[cfg(target_arch = "wasm32")]
fn load_google(client_id: String) {
    use wasm_bindgen::closure::Closure;

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(script) = document.create_element("script") else {
        return;
    };
    let _ = script.set_attribute("src", "https://accounts.google.com/gsi/client");
    let _ = script.set_attribute("async", "");
    // The script arrives after the page has rendered the button's element.
    let on_load = Closure::once_into_js(move || initialize_google(&client_id));
    let _ = js_sys::Reflect::set(&script, &"onload".into(), &on_load);
    if let Some(head) = document.head() {
        let _ = head.append_child(&script);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_google(_client_id: String) {}

#[cfg(target_arch = "wasm32")]
fn initialize_google(client_id: &str) {
    use wasm_bindgen::{JsValue, closure::Closure};

    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(google) = js_sys::Reflect::get(&window, &"google".into()) else {
        return;
    };
    let Ok(accounts) = js_sys::Reflect::get(&google, &"accounts".into()) else {
        return;
    };
    let Ok(id) = js_sys::Reflect::get(&accounts, &"id".into()) else {
        return;
    };

    let on_credential = Closure::<dyn FnMut(JsValue)>::new(|response: JsValue| {
        let credential = js_sys::Reflect::get(&response, &"credential".into())
            .ok()
            .and_then(|value| value.as_string());
        if let Some(credential) = credential {
            wasm_bindgen_futures::spawn_local(async move { sign_in(&credential).await });
        }
    });
    let config = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&config, &"client_id".into(), &client_id.into());
    let _ = js_sys::Reflect::set(&config, &"callback".into(), on_credential.as_ref());
    // The callback lives as long as the page does.
    on_credential.forget();
    call(&id, "initialize", &[config.into()]);

    let Some(button) = window
        .document()
        .and_then(|document| document.get_element_by_id("google-button"))
    else {
        return;
    };
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"theme".into(), &"outline".into());
    let _ = js_sys::Reflect::set(&options, &"size".into(), &"large".into());
    call(&id, "renderButton", &[button.into(), options.into()]);
}

/// Calls `target.method(...args)` if the page has it.
#[cfg(target_arch = "wasm32")]
fn call(target: &wasm_bindgen::JsValue, method: &str, args: &[wasm_bindgen::JsValue]) {
    use wasm_bindgen::JsCast;

    let Ok(function) = js_sys::Reflect::get(target, &method.into()) else {
        return;
    };
    let Ok(function) = function.dyn_into::<js_sys::Function>() else {
        return;
    };
    let args: js_sys::Array = args.iter().collect();
    let _ = function.apply(target, &args);
}

/// The account block in the settings panel. It appears only when the server has a
/// Google client id.
#[component]
pub fn AccountSection() -> impl IntoView {
    view! {
        <Show when=move || client_id().get().is_some()>
            <div class="account">
                <div id="google-button" hidden=move || user().get().is_some()></div>
                <p class="account-user" hidden=move || user().get().is_none()>
                    "Signed in as "
                    {move || user().get().map(|me| me.email)}
                    " "
                    <button type="button" on:click=move |_| sign_out()>"Sign out"</button>
                </p>
                {move || error().get().map(|message| view! { <p class="error">{message}</p> })}
            </div>
        </Show>
    }
}

/// A small header link: the signed-in email with sign out, or "Sign in", which opens the
/// settings panel where the button is. Appears only when the server has a client id.
#[component]
pub fn HeaderLink() -> impl IntoView {
    view! {
        <Show when=move || client_id().get().is_some()>
            <p class="account-link">
                {move || match user().get() {
                    Some(me) => view! {
                        <span>{me.email}</span>
                        " "
                        <button type="button" on:click=move |_| sign_out()>"Sign out"</button>
                    }
                    .into_any(),
                    None => view! {
                        <button type="button" on:click=move |_| open_settings()>"Sign in"</button>
                    }
                    .into_any(),
                }}
            </p>
        </Show>
    }
}

fn open_settings() {
    #[cfg(target_arch = "wasm32")]
    if let Some(details) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector("details.settings").ok().flatten())
    {
        let _ = details.set_attribute("open", "");
    }
}
