//! The settings the gear menu edits: their types, defaults, clamping and the
//! reading of what was stored. No DOM; the page reads and writes the stored
//! string (`web/src/settings.rs`).

use crate::clock::Clock;
use serde::{Deserialize, Serialize};
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
pub const MAX_DAYS: u8 = DEFAULT_WINDOW_DAYS as u8;

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
