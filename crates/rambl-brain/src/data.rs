//! The brain's two data files, baked into the binary and parsed once at startup.
//!
//! The crate deliberately depends on nothing but chrono, so the reader here covers
//! only the slice of TOML these files use: `#` comments, `[section]` headers and
//! `key = "text"` pairs. A full parser would be dead weight for that.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

const LINES_TOML: &str = include_str!("../content/lines.toml");
const PERSONALITY_TOML: &str = include_str!("../content/personality.toml");

/// Something in a data file is wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataError {
    /// A section the brain needs is not there at all.
    Missing(String),
    /// A value that has to be a number is not one.
    NotANumber { key: String, value: String },
    /// A section the brain will ask for has no lines in it.
    NoLines(String),
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(section) => write!(f, "data file has no [{section}] section"),
            Self::NotANumber { key, value } => write!(f, "{key} should be a number, got {value:?}"),
            Self::NoLines(situation) => write!(f, "no lines for {situation:?}"),
        }
    }
}

impl std::error::Error for DataError {}

/// Everything Rambl can say, by situation. A line is addressed as `situation/index`,
/// which is what [`Action::Say`](crate::Action::Say) carries.
#[derive(Debug, Default)]
pub struct Lines {
    by_situation: BTreeMap<String, Vec<String>>,
}

impl Lines {
    /// Read a lines file: `[lines.name]` sections whose `lines` value is several
    /// lines separated by `|`.
    pub fn parse(text: &str) -> Result<Self, DataError> {
        let mut by_situation = BTreeMap::new();
        for (section, values) in tables(text) {
            let Some(name) = section.strip_prefix("lines.") else {
                continue;
            };
            let Some(raw) = values.get("lines") else {
                continue;
            };
            let lines: Vec<String> = raw
                .split('|')
                .map(|line| line.trim().to_owned())
                .filter(|line| !line.is_empty())
                .collect();
            by_situation.insert(name.to_owned(), lines);
        }
        Ok(Self { by_situation })
    }

    /// Every line written for a situation.
    pub fn all(&self, situation: &str) -> &[String] {
        self.by_situation
            .get(situation)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Every situation with lines in it.
    pub fn situations(&self) -> impl Iterator<Item = &str> {
        self.by_situation.keys().map(String::as_str)
    }

    /// The text behind a line key such as `task_done/2`.
    pub fn resolve(&self, key: &str) -> Option<&str> {
        let (situation, index) = key.split_once('/')?;
        let index: usize = index.parse().ok()?;
        self.all(situation).get(index).map(String::as_str)
    }

    /// How many situations have lines.
    pub fn count(&self) -> usize {
        self.by_situation.len()
    }
}

/// How Rambl feels and how quickly he changes: every number in
/// `content/personality.toml`, whose comments say what each one does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Personality {
    pub start_energy: f32,
    pub start_cheer: f32,
    pub start_boredom: f32,
    pub start_hour: u8,
    pub energy_drift: f32,
    pub cheer_decay: f32,
    pub boredom_drift: f32,
    pub cheer_gain: f32,
    pub cheer_loss: f32,
    pub night_drain: f32,
    pub task_added_relief: f32,
    pub task_done_relief: f32,
    pub poke_cheer: f32,
    pub put_back_cheer: f32,
    pub doze_energy: f32,
    pub idle_after: u32,
    pub wander_min_energy: f32,
    pub play_min_cheer: f32,
    pub home_base: f32,
    pub home_boredom: f32,
    pub home_energy: f32,
    pub wander_base: f32,
    pub wander_boredom: f32,
    pub wander_energy: f32,
    pub play_base: f32,
    pub play_cheer: f32,
    pub play_boredom: f32,
    pub wander_cooldown: u32,
    pub play_cooldown: u32,
    pub idle_line_cooldown: u32,
    pub night_start: u8,
    pub night_end: u8,
    /// How likely an idle moment is to come with a line at all.
    pub sass: f32,
}

impl Personality {
    pub fn parse(text: &str) -> Result<Self, DataError> {
        let all = tables(text);
        let start = all.get("start");
        let rates = all.get("rates");
        let events = all.get("events");
        let idle = all.get("idle");
        let cooldowns = all.get("cooldowns");
        let day = all.get("day");
        let voice = all.get("voice");
        Ok(Self {
            start_energy: number(start, "start", "energy")?,
            start_cheer: number(start, "start", "cheer")?,
            start_boredom: number(start, "start", "boredom")?,
            start_hour: hour(start, "start", "hour")?,
            energy_drift: number(rates, "rates", "energy_drift")?,
            cheer_decay: number(rates, "rates", "cheer_decay")?,
            boredom_drift: number(rates, "rates", "boredom_drift")?,
            cheer_gain: number(rates, "rates", "cheer_gain")?,
            cheer_loss: number(rates, "rates", "cheer_loss")?,
            night_drain: number(rates, "rates", "night_drain")?,
            task_added_relief: number(events, "events", "task_added_relief")?,
            task_done_relief: number(events, "events", "task_done_relief")?,
            poke_cheer: number(events, "events", "poke_cheer")?,
            put_back_cheer: number(events, "events", "put_back_cheer")?,
            doze_energy: number(events, "events", "doze_energy")?,
            idle_after: count(idle, "idle", "after")?,
            wander_min_energy: number(idle, "idle", "wander_min_energy")?,
            play_min_cheer: number(idle, "idle", "play_min_cheer")?,
            home_base: number(idle, "idle", "home_base")?,
            home_boredom: number(idle, "idle", "home_boredom")?,
            home_energy: number(idle, "idle", "home_energy")?,
            wander_base: number(idle, "idle", "wander_base")?,
            wander_boredom: number(idle, "idle", "wander_boredom")?,
            wander_energy: number(idle, "idle", "wander_energy")?,
            play_base: number(idle, "idle", "play_base")?,
            play_cheer: number(idle, "idle", "play_cheer")?,
            play_boredom: number(idle, "idle", "play_boredom")?,
            wander_cooldown: count(cooldowns, "cooldowns", "wander")?,
            play_cooldown: count(cooldowns, "cooldowns", "play")?,
            idle_line_cooldown: count(cooldowns, "cooldowns", "idle_line")?,
            night_start: hour(day, "day", "night_start")?,
            night_end: hour(day, "day", "night_end")?,
            sass: number(voice, "voice", "sass")?,
        })
    }

    /// True at the local hours Rambl dozes through.
    pub fn is_night(&self, hour: u8) -> bool {
        hour >= self.night_start || hour < self.night_end
    }
}

/// `[section]` -> `key = value`, with comments and quotes stripped.
fn tables(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut all: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_owned();
            all.entry(section.clone()).or_default();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        all.entry(section.clone()).or_default().insert(
            key.trim().to_owned(),
            value.trim().trim_matches('"').to_owned(),
        );
    }
    all
}

/// Drop a `#` comment. Lines in the data files keep `#` out of strings, so cutting
/// at the first one is enough.
fn strip_comment(line: &str) -> &str {
    line.split_once('#').map_or(line, |(before, _)| before)
}

fn value<'a>(
    table: Option<&'a BTreeMap<String, String>>,
    section: &str,
    key: &str,
) -> Result<&'a str, DataError> {
    table
        .and_then(|values| values.get(key))
        .map(String::as_str)
        .ok_or_else(|| DataError::Missing(format!("{section}.{key}")))
}

fn number(
    table: Option<&BTreeMap<String, String>>,
    section: &str,
    key: &str,
) -> Result<f32, DataError> {
    let raw = value(table, section, key)?;
    raw.parse().map_err(|_| DataError::NotANumber {
        key: format!("{section}.{key}"),
        value: raw.to_owned(),
    })
}

fn count(
    table: Option<&BTreeMap<String, String>>,
    section: &str,
    key: &str,
) -> Result<u32, DataError> {
    Ok(number(table, section, key)? as u32)
}

fn hour(
    table: Option<&BTreeMap<String, String>>,
    section: &str,
    key: &str,
) -> Result<u8, DataError> {
    Ok(number(table, section, key)? as u8)
}

/// Rambl's lines, parsed once.
pub fn lines() -> &'static Lines {
    static LINES: OnceLock<Lines> = OnceLock::new();
    LINES.get_or_init(|| Lines::parse(LINES_TOML).expect("content/lines.toml is valid"))
}

/// Rambl's personality, parsed once.
pub fn personality() -> &'static Personality {
    static PERSONALITY: OnceLock<Personality> = OnceLock::new();
    PERSONALITY.get_or_init(|| {
        Personality::parse(PERSONALITY_TOML).expect("content/personality.toml is valid")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_lines_parse_and_are_addressable() {
        let lines = lines();
        assert!(lines.count() >= 8, "only {} situations", lines.count());
        assert_eq!(lines.resolve("task_done/0"), Some("There it is."));
        for situation in lines.situations() {
            assert!(!lines.all(situation).is_empty(), "{situation} has no lines");
        }
    }

    #[test]
    fn a_missing_or_wrong_key_is_an_error() {
        let text = "[start]\nenergy = fast\n";
        let error = Personality::parse(text).unwrap_err();
        assert_eq!(
            error,
            DataError::NotANumber {
                key: "start.energy".into(),
                value: "fast".into()
            }
        );
        let error = Personality::parse("[start]\nenergy = 0.1\n").unwrap_err();
        assert_eq!(error, DataError::Missing("start.cheer".into()));
    }

    #[test]
    fn the_shipped_personality_is_sane() {
        let p = personality();
        assert!(p.energy_drift > 0.0 && p.energy_drift < 0.2);
        assert!(p.night_start > p.night_end, "night wraps past midnight");
        assert!((0.0..=1.0).contains(&p.sass));
        assert!(p.is_night(23) && p.is_night(2) && !p.is_night(13));
    }

    #[test]
    fn comments_and_quotes_are_handled() {
        let parsed = tables("# a comment\n[lines.hi] # trailing\nlines = \"a|b\" # note\n");
        assert_eq!(parsed["lines.hi"]["lines"], "a|b");
    }
}
