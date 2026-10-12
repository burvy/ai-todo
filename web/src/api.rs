use gloo_net::http::Request;
use leptos::prelude::*;
use todo_core::Snapshot;

pub fn timezone() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let options =
            js_sys::Intl::DateTimeFormat::new(&js_sys::Array::new(), &js_sys::Object::new())
                .resolved_options();
        js_sys::Reflect::get(&options, &"timeZone".into())
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_else(|| "UTC".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        "UTC".to_string()
    }
}

pub fn use_snapshot() -> ReadSignal<Snapshot> {
    let initial: Snapshot = serde_json::from_str(include_str!("../fixtures/snapshot.json"))
        .expect("fixture snapshot must be valid");
    let (snapshot, _) = signal(initial);
    snapshot
}

pub async fn fetch_snapshot(from: &str, days: u8) -> Result<Snapshot, gloo_net::Error> {
    Request::get(&format!("/api/v1/snapshot?from={from}&days={days}"))
        .header("X-Rambl-TZ", &timezone())
        .send()
        .await?
        .json()
        .await
}
