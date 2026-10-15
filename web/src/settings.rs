use crate::clock::Clock;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use todo_core::DaySettings;
use todo_core::tuning::DEFAULT_WINDOW_DAYS;

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum RamblMode {
    #[default]
    Interactive,
    Instant,
    Lobotomized,
}

/// The pen Rambo writes with (DESIGN-DIRECTION). The choice is per device.
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

/// How a page turns on a phone: the whole page swinging like a book's (`Realistic`, the
/// default) or the corner-fold bend (`Fast`), which is cheaper to draw.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageTurn {
    #[default]
    Realistic,
    Fast,
}

impl PageTurn {
    pub fn label(self) -> &'static str {
        match self {
            PageTurn::Realistic => "Realistic",
            PageTurn::Fast => "Fast",
        }
    }
}

/// How much the page makes noise, the gear menu's Sound choice. It replaces the old
/// on/off flag: off became `Off`, on became `Touch`.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoundPreset {
    /// Silence: nothing plays and Rambo does not speak.
    Off,
    /// A quiet desk: only what paper and ink really make a noise doing, and Rambo keeps
    /// his voice to himself.
    Quiet,
    /// Every touch answers: the pen, the page, the tack, the bin.
    #[default]
    Touch,
    /// Everything, with calm music under it.
    Music,
}

impl SoundPreset {
    pub const ALL: [SoundPreset; 4] = [
        SoundPreset::Off,
        SoundPreset::Quiet,
        SoundPreset::Touch,
        SoundPreset::Music,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SoundPreset::Off => "Off",
            SoundPreset::Quiet => "A quiet desk",
            SoundPreset::Touch => "Every touch answers",
            SoundPreset::Music => "Everything + music",
        }
    }

    /// Whether Rambo may speak aloud: not when it is off or at a quiet desk.
    pub fn allows_voice(self) -> bool {
        !matches!(self, SoundPreset::Off | SoundPreset::Quiet)
    }
}

const DEFAULTS: Settings = Settings {
    // The working day the paper is for: eight until six (DESIGN-DIRECTION,
    // "Planner"). Opening on 08:00, not midnight, is what QA asked for.
    start_hour: 8,
    end_hour: 18,
    days: 7,
    mode: RamblMode::Interactive,
    theme: Theme::Light,
    sound: SoundPreset::Touch,
    rambo_talks: true,
    pen: Pen::FountainPen,
    clock: Clock::Twelve,
    page_turn: PageTurn::Realistic,
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
    /// What makes noise (`SoundPreset`). Settings stored with the old on/off flag are
    /// read by `parse_or_default`.
    #[serde(default)]
    pub sound: SoundPreset,
    /// Times are written as `9 am` by default; `24:00` style is a gear choice.
    #[serde(default)]
    pub clock: Clock,
    /// Rambo talks on by default (docs/DESIGN-DIRECTION.md 3). The bubble shows
    /// either way; this only decides whether there is a voice with it.
    #[serde(default = "rambo_talks_on")]
    pub rambo_talks: bool,
    /// How a page turns on a phone (`PageTurn`); a stored setting from before this
    /// existed gets the book flip.
    #[serde(default)]
    pub page_turn: PageTurn,
}

fn rambo_talks_on() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        DEFAULTS
    }
}

/// The most planner columns. The page fetches one window of this many days, so it cannot
/// show more days than it has.
const MAX_DAYS: u8 = DEFAULT_WINDOW_DAYS as u8;

impl Settings {
    pub fn clamped(self) -> Self {
        let start_hour = self.start_hour.min(23);
        let end_hour = self.end_hour.clamp(start_hour.saturating_add(1), 24);
        Self {
            start_hour,
            end_hour,
            days: self.days.clamp(1, MAX_DAYS),
            mode: self.mode,
            pen: self.pen,
            theme: self.theme,
            sound: self.sound,
            clock: self.clock,
            rambo_talks: self.rambo_talks,
            page_turn: self.page_turn,
        }
    }

    pub fn parse_or_default(value: &str) -> Self {
        serde_json::from_str(value)
            .ok()
            .map(with_migrated_sound)
            .and_then(|stored| serde_json::from_value::<Self>(stored).ok())
            .map(Self::clamped)
            .unwrap_or_default()
    }
}

/// Settings stored before the presets carry `"sounds": true|false`; that becomes the
/// preset it stood for unless a preset is already stored.
fn with_migrated_sound(mut stored: serde_json::Value) -> serde_json::Value {
    if let Some(object) = stored.as_object_mut()
        && let Some(old) = object.remove("sounds").and_then(|flag| flag.as_bool())
    {
        let preset = if old {
            SoundPreset::Touch
        } else {
            SoundPreset::Off
        };
        object
            .entry("sound")
            .or_insert_with(|| serde_json::json!(preset));
    }
    stored
}

/// One of the two times the server keeps per user for a task with a day and no time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DayTime {
    /// When "by Friday" is due on that day.
    Deadline,
    /// Where on its day a task goes when there are no times of the user's own to learn from.
    Placement,
}

impl DayTime {
    fn of(self, settings: &DaySettings) -> chrono::NaiveTime {
        match self {
            DayTime::Deadline => settings.deadline,
            DayTime::Placement => settings.placement,
        }
    }

    /// `settings` with this one time changed.
    fn set(self, settings: DaySettings, time: chrono::NaiveTime) -> DaySettings {
        match self {
            DayTime::Deadline => DaySettings {
                deadline: time,
                ..settings
            },
            DayTime::Placement => DaySettings {
                placement: time,
                ..settings
            },
        }
    }

    /// The patch that changes only this time, so a change made elsewhere is not undone.
    fn patch(self, time: chrono::NaiveTime) -> crate::api::DaySettingsBody {
        let written = Some(time.format("%H:%M").to_string());
        match self {
            DayTime::Deadline => crate::api::DaySettingsBody {
                deadline: written,
                placement: None,
            },
            DayTime::Placement => crate::api::DaySettingsBody {
                deadline: None,
                placement: written,
            },
        }
    }
}

/// The two day-only times, kept by the server: a change is saved at once and the plan
/// is read again, since the planner places tasks by them. A refused change (the server
/// does not take a 00:00 deadline) puts the old time back and says why.
#[component]
fn DayTimes(
    clock: Signal<Clock>,
    set_snapshot: WriteSignal<crate::Snapshot>,
    set_error: WriteSignal<Option<String>>,
) -> impl IntoView {
    // The defaults show until the server's own answer comes in, so nothing jumps.
    let held = RwSignal::new(DaySettings::default());
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(loaded) = crate::api::fetch_day_settings().await {
            held.set(loaded);
        }
    });
    let change = move |which: DayTime, written: String| {
        // These always have a time: clearing one is not an option the picker offers.
        let Ok(time) = chrono::NaiveTime::parse_from_str(&written, "%H:%M") else {
            return;
        };
        let before = held.get_untracked();
        held.set(which.set(before, time));
        wasm_bindgen_futures::spawn_local(async move {
            match crate::api::patch_day_settings(&which.patch(time)).await {
                Ok(saved) => {
                    held.set(saved);
                    if let Ok(plan) = crate::api::fetch_snapshot(&crate::api::today_date()).await {
                        set_snapshot.set(plan);
                    }
                }
                Err(error) => {
                    held.set(before);
                    set_error.set(Some(format!("Could not save that time: {error}")));
                }
            }
        });
    };
    let shown = move |which: DayTime| {
        Signal::derive(move || which.of(&held.get()).format("%H:%M").to_string())
    };
    view! {
        <div class="gear-row">
            <span class="gear-row-name">"Due by"</span>
            <crate::pickers::PaperTime
                label="Due by"
                value=shown(DayTime::Deadline)
                set=Callback::new(move |written| change(DayTime::Deadline, written))
                clock
                clearable=false
            />
        </div>
        <div class="gear-row">
            <span class="gear-row-name">"Placed near"</span>
            <crate::pickers::PaperTime
                label="Placed near"
                value=shown(DayTime::Placement)
                set=Callback::new(move |written| change(DayTime::Placement, written))
                clock
                clearable=false
            />
        </div>
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
    let set_sound = move |sound: SoundPreset| {
        update(Settings {
            sound,
            ..settings.get()
        });
        // A quiet desk cuts him off mid-sentence too, not just the next one.
        if !sound.allows_voice() {
            crate::voice::cancel();
        }
    };
    let toggle_rambo_talks = move || {
        let next = Settings {
            rambo_talks: !settings.get().rambo_talks,
            ..settings.get()
        };
        update(next);
        // Turning him off mid-sentence should stop the sentence, not the next one.
        if !next.rambo_talks || !next.sound.allows_voice() {
            crate::voice::cancel();
        }
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

    let set_page_turn = move |page_turn: PageTurn| {
        update(Settings {
            page_turn,
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
                        <h3>"Page turn"</h3>
                        <div class="gear-choices" role="group" aria-label="Page turn">
                            <For each=|| [PageTurn::Realistic, PageTurn::Fast] key=|t| *t let:page_turn>
                                <button
                                    type="button"
                                    role="radio"
                                    aria-checked=move || (settings.get().page_turn == page_turn).to_string()
                                    class:active=move || settings.get().page_turn == page_turn
                                    on:click=move |_| set_page_turn(page_turn)
                                >{page_turn.label()}</button>
                            </For>
                        </div>
                        <h3>"Sound"</h3>
                        <div class="gear-choices" role="group" aria-label="Sound">
                            <For each=|| SoundPreset::ALL key=|preset| *preset let:preset>
                                <button
                                    type="button"
                                    aria-pressed=move || (settings.get().sound == preset).to_string()
                                    class:active=move || settings.get().sound == preset
                                    on:click=move |_| set_sound(preset)
                                >{preset.label()}</button>
                            </For>
                        </div>
                        <label>
                            "Rambo talks "
                            <button
                                type="button"
                                aria-pressed=move || settings.get().rambo_talks.to_string()
                                class:active=move || settings.get().rambo_talks
                                on:click=move |_| toggle_rambo_talks()
                            >{move || if settings.get().rambo_talks { "On" } else { "Off" }}
                            </button>
                        </label>
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
                            <input type="number" min="1" max=MAX_DAYS.to_string() prop:value=move || settings.get().days
                                on:change=move |event| {
                                    if let Ok(days) = event_target_value(&event).parse() {
                                        update(Settings { days, ..settings.get() });
                                    }
                                } />
                        </label>
                        <h3>"A day with no time"</h3>
                        <DayTimes
                            clock=Signal::derive(move || settings.get().clock)
                            set_snapshot
                            set_error
                        />
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
                        // A plain link would fetch the file without this browser's
                        // session or anonymous id, and so export someone else's (empty)
                        // calendar; the API call carries both.
                        <button
                            type="button"
                            class="gear-item"
                            on:click=move |_| {
                                wasm_bindgen_futures::spawn_local(async move {
                                    match crate::api::export_calendar().await {
                                        Ok(calendar) => save_calendar(&calendar),
                                        Err(e) => set_error.set(Some(format!("Download failed: {e}"))),
                                    }
                                });
                            }
                        >"Download calendar (.ics)"</button>
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
/// Hands the exported calendar to the browser as a file download.
#[cfg(target_arch = "wasm32")]
fn save_calendar(calendar: &str) {
    use wasm_bindgen::JsCast;
    let parts = js_sys::Array::of1(&calendar.into());
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("text/calendar");
    let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options) else {
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return;
    };
    if let Some(link) = document()
        .create_element("a")
        .ok()
        .and_then(|element| element.dyn_into::<web_sys::HtmlAnchorElement>().ok())
    {
        link.set_href(&url);
        link.set_download("rambl-calendar.ics");
        link.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

#[cfg(not(target_arch = "wasm32"))]
fn save_calendar(_calendar: &str) {}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u32, minute: u32) -> chrono::NaiveTime {
        chrono::NaiveTime::from_hms_opt(hour, minute, 0).unwrap()
    }

    #[test]
    fn a_day_time_changes_only_its_own_field() {
        let settings = DaySettings::default();
        let moved = DayTime::Deadline.set(settings, at(17, 0));
        assert_eq!(moved.deadline, at(17, 0));
        assert_eq!(moved.placement, settings.placement);
        let moved = DayTime::Placement.set(settings, at(9, 30));
        assert_eq!(moved.placement, at(9, 30));
        assert_eq!(moved.deadline, settings.deadline);
        assert_eq!(DayTime::Placement.of(&moved), at(9, 30));
    }

    #[test]
    fn the_patch_for_a_day_time_names_only_that_field_in_hh_mm() {
        let deadline = serde_json::to_string(&DayTime::Deadline.patch(at(17, 5))).unwrap();
        assert_eq!(deadline, r#"{"deadline":"17:05"}"#);
        let placement = serde_json::to_string(&DayTime::Placement.patch(at(9, 0))).unwrap();
        assert_eq!(placement, r#"{"placement":"09:00"}"#);
    }

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
        assert_eq!(Settings::default().sound, SoundPreset::Touch);
        assert_eq!(Settings::default().pen, Pen::FountainPen);
    }

    #[test]
    fn a_stored_setting_from_before_the_page_turn_choice_gets_the_book_flip() {
        let stored = Settings::parse_or_default(r#"{"start_hour":8,"end_hour":18,"days":7}"#);
        assert_eq!(stored.page_turn, PageTurn::Realistic);
        let fast = Settings {
            page_turn: PageTurn::Fast,
            ..Settings::default()
        };
        let round_trip = Settings::parse_or_default(&serde_json::to_string(&fast).unwrap());
        assert_eq!(round_trip.page_turn, PageTurn::Fast, "a choice is kept");
    }

    #[test]
    fn rambo_talks_defaults_on_and_a_stored_choice_is_kept() {
        // Settings stored before the voice existed have no key for it, so the
        // `#[serde(default)]` has to give Rambo his voice rather than take it away.
        let stored_before_the_voice =
            Settings::parse_or_default(r#"{"start_hour":8,"end_hour":18}"#);
        assert!(stored_before_the_voice.rambo_talks, "voice on by default");

        // Once the reader has turned it off, that is the answer that sticks.
        let turned_off = Settings::parse_or_default(
            r#"{"start_hour":8,"end_hour":18,"days":7,"rambo_talks":false}"#,
        );
        assert!(!turned_off.rambo_talks, "a stored choice is kept");

        // Turning it back on is stored like any other choice.
        assert!(
            Settings::parse_or_default(
                r#"{"start_hour":8,"end_hour":18,"days":7,"rambo_talks":true}"#
            )
            .rambo_talks,
            "and so is turning it back on"
        );
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
    fn the_old_sound_flag_becomes_a_preset() {
        let stored = |flag: &str| {
            Settings::parse_or_default(&format!(
                r#"{{"start_hour":8,"end_hour":18,"days":7,"sounds":{flag}}}"#
            ))
            .sound
        };
        assert_eq!(stored("false"), SoundPreset::Off, "silence stays silence");
        assert_eq!(stored("true"), SoundPreset::Touch, "on is every touch");
        let both = r#"{"start_hour":8,"end_hour":18,"days":7,"sounds":false,"sound":"Music"}"#;
        assert_eq!(
            Settings::parse_or_default(both).sound,
            SoundPreset::Music,
            "a stored preset wins"
        );
    }

    #[test]
    fn a_stored_pick_survives_the_round_trip() {
        let stored = Settings {
            theme: Theme::Dark,
            sound: SoundPreset::Music,
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
