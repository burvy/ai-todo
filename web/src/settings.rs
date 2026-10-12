use leptos::prelude::*;
use serde::{Deserialize, Serialize};

const DEFAULTS: Settings = Settings {
    start_hour: 0,
    end_hour: 24,
    days: 7,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub start_hour: u8,
    pub end_hour: u8,
    pub days: u8,
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
        }
    }

    pub fn parse_or_default(value: &str) -> Self {
        serde_json::from_str(value)
            .map(|settings: Self| settings.clamped())
            .unwrap_or_default()
    }
}

#[component]
pub fn SettingsPanel(
    settings: ReadSignal<Settings>,
    set_settings: WriteSignal<Settings>,
) -> impl IntoView {
    let update = move |next: Settings| {
        let next = next.clamped();
        set_settings.set(next);
        save(next);
    };
    view! {
        <details class="settings">
            <summary>"View settings"</summary>
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
        </details>
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
    fn clamps_hour_range_and_days() {
        assert_eq!(
            Settings {
                start_hour: 99,
                end_hour: 0,
                days: 0
            }
            .clamped(),
            Settings {
                start_hour: 23,
                end_hour: 24,
                days: 1
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
                days: 7
            }
        );
    }
}
