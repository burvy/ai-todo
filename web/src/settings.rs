use crate::clock::Clock;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum RamblMode {
    #[default]
    Interactive,
    Instant,
    Lobotomized,
}

/// The pen Rambo writes with (DESIGN-DECISIONS). The choice is per device.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pen {
    #[default]
    FountainPen,
    Pencil,
    Ballpoint,
    Quill,
    Marker,
}

impl Pen {
    pub fn label(self) -> &'static str {
        match self {
            Pen::FountainPen => "Fountain pen",
            Pen::Pencil => "Pencil",
            Pen::Ballpoint => "Ballpoint",
            Pen::Quill => "Quill",
            Pen::Marker => "Marker",
        }
    }

    /// CSS/BEM-safe name: `data-pen` on Rambo, class names on his art.
    pub fn slug(self) -> &'static str {
        match self {
            Pen::FountainPen => "fountain-pen",
            Pen::Pencil => "pencil",
            Pen::Ballpoint => "ballpoint",
            Pen::Quill => "quill",
            Pen::Marker => "marker",
        }
    }
}

/// Parchment light is the default and the page never follows the OS: dark
/// exists only when the user picks it in the gear menu (owner QA, 2026-10-13).
/// `data-theme="light"` is baked into `web/index.html` so even the first
/// paint before the app loads is light.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    pub fn label(self) -> &'static str {
        match self {
            Theme::Light => "Light",
            Theme::Dark => "Dark",
        }
    }
}

fn sounds_on() -> bool {
    true
}

const DEFAULTS: Settings = Settings {
    // The working day the paper is for: eight until six (DESIGN-DECISIONS,
    // "Planner"). Opening on 08:00, not midnight, is what QA asked for.
    start_hour: 8,
    end_hour: 18,
    days: 7,
    mode: RamblMode::Interactive,
    theme: Theme::Light,
    sounds: true,
    pen: Pen::FountainPen,
    clock: Clock::Twelve,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub start_hour: u8,
    pub end_hour: u8,
    pub days: u8,
    #[serde(default)]
    pub mode: RamblMode,
    /// The pen Rambo writes with; new fields default so older stored
    /// settings keep parsing.
    #[serde(default)]
    pub pen: Pen,
    #[serde(default)]
    pub theme: Theme,
    /// Sounds are on by default (DESIGN-DECISIONS); the gear menu mutes.
    #[serde(default = "sounds_on")]
    pub sounds: bool,
    /// Times are written as `9 am` by default; `24:00` style is a gear choice.
    #[serde(default)]
    pub clock: Clock,
}

impl Default for Settings {
    fn default() -> Self {
        DEFAULTS
    }
}

impl Settings {
    pub fn clamped(self) -> Self {
        let start_hour = self.start_hour.min(23);
        let end_hour = self.end_hour.clamp(start_hour.saturating_add(1), 24);
        Self {
            start_hour,
            end_hour,
            days: self.days.clamp(1, 7),
            mode: self.mode,
            pen: self.pen,
            theme: self.theme,
            sounds: self.sounds,
            clock: self.clock,
        }
    }

    pub fn parse_or_default(value: &str) -> Self {
        serde_json::from_str(value)
            .map(|settings: Self| settings.clamped())
            .unwrap_or_default()
    }
}

/// The gear, top right: one small button holding every setting — the working
/// day, the paper (light/dark/system), sounds, Rambo's mode and pen, Import
/// .ics and the account. Nothing else on the page may show a settings control.
#[component]
pub fn GearMenu(
    settings: ReadSignal<Settings>,
    set_settings: WriteSignal<Settings>,
    set_snapshot: WriteSignal<crate::Snapshot>,
    set_error: WriteSignal<Option<String>>,
) -> impl IntoView {
    let (open, set_open) = signal(false);
    let file_ref = NodeRef::<leptos::html::Input>::new();

    let update = move |next: Settings| {
        let next = next.clamped();
        set_settings.set(next);
        save(next);
        apply_theme(next.theme);
    };

    // The stored choice wins over the OS from the first paint on.
    Effect::new(move |_| apply_theme(settings.get().theme));

    let set_theme = move |theme: Theme| {
        update(Settings {
            theme,
            ..settings.get()
        })
    };
    let toggle_sounds = move || {
        update(Settings {
            sounds: !settings.get().sounds,
            ..settings.get()
        })
    };
    let set_pen = move |pen: Pen| {
        update(Settings {
            pen,
            ..settings.get()
        })
    };
    let set_clock = move |clock: Clock| {
        update(Settings {
            clock,
            ..settings.get()
        })
    };

    let close = move |_| set_open.set(false);

    // Escape closes the sheet from wherever the focus sits; a press on the
    // page around the menu is caught by the backdrop.
    window_event_listener(leptos::ev::keydown, move |event| {
        if event.key() == "Escape" && open.get_untracked() {
            set_open.set(false);
        }
    });

    view! {
        <div class="gear-wrap">
            <button
                type="button"
                class="gear"
                aria-haspopup="menu"
                aria-expanded=move || if open.get() { "true" } else { "false" }
                on:click=move |_| set_open.update(|open| *open = !*open)
            >
                <svg
                    xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="22" height="22"
                    fill="none" stroke="currentColor" stroke-width="1.8"
                    stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
                >
                    <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/>
                    <circle cx="12" cy="12" r="3"/>
                </svg>
                <span class="sr-only">"Settings"</span>
            </button>
            {move || open.get().then(|| {
                view! {
                    <div class="gear-backdrop" on:click=close></div>
                    <div class="gear-menu" role="menu" aria-label="Settings">
                        <h3>"Paper"</h3>
                        <div class="gear-choices" role="group" aria-label="Theme">
                            <For each=|| [Theme::Light, Theme::Dark] key=|t| *t let:theme>
                                <button
                                    type="button"
                                    role="radio"
                                    aria-checked=move || (settings.get().theme == theme).to_string()
                                    class:active=move || settings.get().theme == theme
                                    on:click=move |_| set_theme(theme)
                                >{theme.label()}</button>
                            </For>
                        </div>
                        <h3>"Clock"</h3>
                        <div class="gear-choices" role="group" aria-label="Clock">
                            <For each=|| [Clock::Twelve, Clock::TwentyFour] key=|c| *c let:clock>
                                <button
                                    type="button"
                                    role="radio"
                                    aria-checked=move || (settings.get().clock == clock).to_string()
                                    class:active=move || settings.get().clock == clock
                                    on:click=move |_| set_clock(clock)
                                >{clock.label()}</button>
                            </For>
                        </div>
                        <h3>"Sounds"</h3>
                        <div class="gear-choices" role="group" aria-label="Sounds">
                            <button
                                type="button"
                                aria-pressed=move || settings.get().sounds.to_string()
                                class:active=move || settings.get().sounds
                                on:click=move |_| toggle_sounds()
                            >{move || if settings.get().sounds { "On" } else { "Off" }}
                            </button>
                        </div>
                        <h3>"Rambo"</h3>
                        <div class="gear-choices" role="group" aria-label="Rambo's pen">
                            <For each=|| [Pen::FountainPen, Pen::Pencil, Pen::Ballpoint, Pen::Quill, Pen::Marker] key=|p| *p let:pen>
                                <button
                                    type="button"
                                    role="radio"
                                    aria-checked=move || (settings.get().pen == pen).to_string()
                                    class:active=move || settings.get().pen == pen
                                    on:click=move |_| set_pen(pen)
                                >{pen.label()}</button>
                            </For>
                        </div>
                        <label>
                            "Mode "
                            <select on:change=move |event| {
                                let mode = match event_target_value(&event).as_str() {
                                    "instant" => RamblMode::Instant,
                                    "lobotomized" => RamblMode::Lobotomized,
                                    _ => RamblMode::Interactive,
                                };
                                update(Settings { mode, ..settings.get() });
                            }>
                                <option value="interactive" selected=move || settings.get().mode == RamblMode::Interactive>"Interactive"</option>
                                <option value="instant" selected=move || settings.get().mode == RamblMode::Instant>"Instant"</option>
                                <option value="lobotomized" selected=move || settings.get().mode == RamblMode::Lobotomized>"Lobotomized"</option>
                            </select>
                        </label>
                        <h3>"Working day"</h3>
                        <label>
                            "Start hour "
                            <input type="number" min="0" max="23" prop:value=move || settings.get().start_hour
                                on:change=move |event| {
                                    if let Ok(hour) = event_target_value(&event).parse() {
                                        update(Settings { start_hour: hour, ..settings.get() });
                                    }
                                } />
                        </label>
                        <label>
                            "End hour "
                            <input type="number" min="1" max="24" prop:value=move || settings.get().end_hour
                                on:change=move |event| {
                                    if let Ok(hour) = event_target_value(&event).parse() {
                                        update(Settings { end_hour: hour, ..settings.get() });
                                    }
                                } />
                        </label>
                        <label>
                            "Days "
                            <input type="number" min="1" max="7" prop:value=move || settings.get().days
                                on:change=move |event| {
                                    if let Ok(days) = event_target_value(&event).parse() {
                                        update(Settings { days, ..settings.get() });
                                    }
                                } />
                        </label>
                        <div class="gear-rule"></div>
                        <button
                            type="button"
                            class="gear-item"
                            on:click=move |_| {
                                if let Some(input) = file_ref.get() {
                                    input.click();
                                }
                            }
                        >"Import .ics"</button>
                        <input
                            node_ref=file_ref
                            type="file"
                            accept=".ics"
                            class="gear-file"
                            aria-label="Import calendar file"
                            on:change=move |event| {
                                let input = event_target::<web_sys::HtmlInputElement>(&event);
                                let file = input.files().and_then(|files| files.get(0));
                                input.set_value("");
                                let Some(file) = file else { return; };
                                let set_snapshot = set_snapshot;
                                let set_error = set_error;
                                wasm_bindgen_futures::spawn_local(async move {
                                    match crate::api::import_ics(file).await {
                                        Ok(response) => {
                                            set_snapshot.set(response.snapshot);
                                            set_error.set(None);
                                        }
                                        Err(e) => set_error.set(Some(format!("Import failed: {e}"))),
                                    }
                                });
                            }
                        />
                        <a class="gear-item" href="/api/v1/calendar.ics" download="rambl-calendar.ics">
                            "Download calendar (.ics)"
                        </a>
                        <crate::account::AccountSection />
                        <p class="version">
                            "Rambl "
                            {env!("CARGO_PKG_VERSION")}
                            " (built "
                            {env!("RAMBL_BUILD_DATE")}
                            ")"
                        </p>
                    </div>
                }
            })}
        </div>
    }
}

/// Light/Dark set `data-theme` on the page; the attribute is always present
/// (index.html bakes in the light default), so the OS is never followed.
fn apply_theme(theme: Theme) {
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

fn save(_settings: Settings) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Ok(Some(storage)) = leptos::prelude::window().local_storage() {
            if let Ok(value) = serde_json::to_string(&_settings) {
                let _ = storage.set_item("rambl-view-settings", &value);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = _settings;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_working_day_defaults_to_eight_until_six() {
        let settings = Settings::default();
        assert_eq!(settings.start_hour, 8);
        assert_eq!(settings.end_hour, 18);
    }

    #[test]
    fn clamps_hour_range_and_days() {
        assert_eq!(
            Settings {
                start_hour: 99,
                end_hour: 0,
                days: 0,
                ..Settings::default()
            }
            .clamped(),
            Settings {
                start_hour: 23,
                end_hour: 24,
                days: 1,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn invalid_storage_uses_defaults() {
        assert_eq!(Settings::parse_or_default("not json"), Settings::default());
    }

    #[test]
    fn valid_storage_is_clamped() {
        assert_eq!(
            Settings::parse_or_default(r#"{"start_hour":8,"end_hour":30,"days":9}"#),
            Settings {
                start_hour: 8,
                end_hour: 24,
                days: 7,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn older_settings_gain_the_new_defaults() {
        // Stored before pen/theme/sounds existed: the paper stays light-by-OS,
        // sounds stay on, and Rambo keeps the fountain pen.
        assert_eq!(
            Settings::parse_or_default(r#"{"start_hour":0,"end_hour":24,"days":7}"#),
            Settings {
                start_hour: 0,
                end_hour: 24,
                days: 7,
                ..Settings::default()
            }
        );
        assert_eq!(Settings::default().theme, Theme::Light);
        assert!(Settings::default().sounds);
        assert_eq!(Settings::default().pen, Pen::FountainPen);
    }

    #[test]
    fn a_stored_system_choice_falls_back_to_light() {
        // "System" no longer exists: the row is refused wholesale and the
        // defaults (light paper, sounds on, fountain pen) take over.
        assert_eq!(
            Settings::parse_or_default(r#"{"theme":"System"}"#),
            Settings::default()
        );
    }

    #[test]
    fn a_stored_pick_survives_the_round_trip() {
        let stored = Settings {
            theme: Theme::Dark,
            sounds: false,
            pen: Pen::Quill,
            ..Settings::default()
        };
        let json = serde_json::to_string(&stored).unwrap();
        assert_eq!(Settings::parse_or_default(&json), stored);
    }

    #[test]
    fn pen_names_are_css_safe_and_unique() {
        let slugs = [
            Pen::FountainPen,
            Pen::Pencil,
            Pen::Ballpoint,
            Pen::Quill,
            Pen::Marker,
        ]
        .map(Pen::slug);
        for slug in slugs {
            assert!(!slug.is_empty());
            assert!(slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
            assert_eq!(slugs.iter().filter(|s| **s == slug).count(), 1);
        }
    }
}
