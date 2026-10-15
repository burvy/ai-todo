pub use rambl_web_core::settings::{
    MAX_DAYS, PageTurn, Pen, RamblMode, Settings, SoundPreset, Theme,
};

/// Light/Dark set `data-theme` on the page; the attribute is always present
/// (index.html bakes in the light default), so the OS is never followed.
pub(crate) fn apply_theme(theme: Theme) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        let document = leptos::prelude::document();
        let html = document
            .document_element()
            .and_then(|el| el.dyn_into::<web_sys::HtmlHtmlElement>().ok());
        if let Some(html) = html {
            match theme {
                Theme::Light => html.set_attribute("data-theme", "light"),
                Theme::Dark => html.set_attribute("data-theme", "dark"),
            }
            .expect("setting data-theme on the page's <html> cannot fail");
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = theme;
    }
}

pub fn load() -> Settings {
    #[cfg(target_arch = "wasm32")]
    {
        leptos::prelude::window()
            .local_storage()
            .ok()
            .flatten()
            .and_then(|storage| storage.get_item("rambl-view-settings").ok().flatten())
            .map(|value| Settings::parse_or_default(&value))
            .unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Settings::default()
    }
}

pub(crate) fn save(settings: Settings) {
    #[cfg(target_arch = "wasm32")]
    if let Ok(Some(storage)) = leptos::prelude::window().local_storage()
        && let Ok(value) = serde_json::to_string(&settings)
    {
        let _ = storage.set_item("rambl-view-settings", &value);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = settings;
}
