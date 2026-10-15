# Tuning

Every number that changes how Rambl behaves lives in one documented place, so it can be
found, understood and changed without reading the code around it. This page is the
convention, and the index of where each kind of value lives.

## Where values live

| Kind of value | Where | Example |
|---|---|---|
| Used by one crate | that crate's `src/tuning.rs` | `server/src/v2/tuning.rs`: `SESSION_LENGTH` |
| Shared by two or more crates | `todo_core::tuning` (`crates/core/src/tuning.rs`), the crate every other depends on | `TASK_MAX_DURATION_MIN` |
| Set per deployment | an environment variable; its default is a `DEFAULT_*` constant in `tuning.rs` | `RAMBL_JEV_DAILY_CAP` → `DEFAULT_JEV_DAILY_CAP` |
| Chosen by each user | a stored user setting; its default is a constant in `tuning.rs` | `DEFAULT_DAY_DEADLINE`, the default for each user's day-only deadline |
| Content and personality a non-programmer edits | a TOML file in the crate's `content/` folder, compiled in | `crates/rambl-brain/content/personality.toml` |
| Web behaviour (timings, thresholds) | `crates/web-core/src/tuning.rs` (started: the time picker's minute step and resting hour, Rambo's writing pace; the rest is planned with the front-end restructure) | `MINUTE_STEP` |
| Web look (colours, spacing, radii, animation lengths) | CSS custom properties on `:root` in `web/theme.css`; colours are there today, the rest is planned with the front-end restructure | `--ink`, `--accent` |

A value is never written twice. When a second crate needs it, it moves down to
`todo_core::tuning` and both use it from there; a copy that drifts is how a capture and
an edit end up with different limits.

## How a value is written

```rust
/// An anonymous account unused for this long is deleted with its tasks, after the
/// daily backup. Safe range 7 to 365 days.
pub const ANONYMOUS_IDLE_LIMIT: chrono::Duration = chrono::Duration::days(30);

/// Captures a signed-in user may run per day before capture stops using Jev.
/// `RAMBL_JEV_DAILY_CAP`.
pub const DEFAULT_JEV_DAILY_CAP: u32 = 200;
```

- **A doc comment on every value**: what it controls, its unit, a safe range, and the
  environment variable that overrides it, if any.
- **The unit is in the type or the name.** Prefer `Duration`, `NaiveTime` and other
  typed values; where the type reaches, a typed value wins over a suffix. A bare
  number ends its name with its unit: `_MS` (milliseconds), `_SECS`, `_MIN`
  (minutes), `_DAYS`, `_CHARS`, `_BYTES`, `_ROWS`, `_PX` (CSS pixels), `_PERCENT`.
- **Relations are checked at compile time** where two values must agree, for example
  `const _: () = assert!(TASK_MIN_DURATION_MIN <= TASK_DEFAULT_DURATION_MIN);`.
- **Grouped by what they affect**, with a `// ---- heading` per group, so related values
  sit together.

### On the web

`crates/web-core/src/tuning.rs` holds the paper pickers' numbers so far. It follows the same rules; a
behaviour value lives there and a look value is a CSS token:

```rust
// ---- the capture box

/// How long each whisper hint shows before the next. Safe range 3000 to 15000 ms.
pub const WHISPER_ROTATE_MS: u32 = 6000;
/// Drag a slip at least this far before it counts as a drag, not a click. Safe
/// range 3 to 12 px.
pub const DRAG_START_PX: f64 = 6.0;
```

```css
:root {
  --radius-slip: 6px;   /* corner of a task slip; safe range 0 to 12px */
}
```

## What is not a tuning value

Facts that cannot change stay where they are used, named when the meaning is not
obvious:

- the calendar and clock: 7 days a week, 12 months, 60 minutes an hour, 23:59
- formats and protocols: iCalendar units, IPv4's 32-bit prefix, `HH:MM`
- database shape: column positions, migration text, `SCHEMA_VERSION`
- tests: fixtures and expected values. A test that checks a limit uses the constant,
  so changing the limit does not break the test that guards it.

## Changing a value

1. Edit the constant in its `tuning.rs` (or the TOML file) and nowhere else.
2. Run the gate in `AGENTS.md`. A failing test usually means a test assumed the old
   value; make it use the constant instead.
3. If the change is something a user or operator can see, update `docs/ARCHITECTURE.md`
   (the environment table names every variable and its default) and the release notes.

## Index

| Module | What it covers |
|---|---|
| `crates/core/src/tuning.rs` | Task length (shortest, longest, default); the working day the planner fills; the defaults for the day-only settings (deadline 23:59, placement 12:00) |
| `server/src/v2/tuning.rs` | Port, sessions, anonymous expiry, daily caps, request size limits, snapshot window, calendar import limits, database and backup timing, admin rows |
| `crates/capture/src/tuning.rs` | How sure Jev must be that a clause is a task; how short a time fragment is; how an email's quoted header is told from a signature |
| `crates/dates/src/tuning.rs` | When "Friday", said on a Friday, stops meaning today |
| `crates/scheduler` | No values of its own: it plans with the working day and task lengths from `todo_core::tuning` |
| `crates/rambl-brain/content/personality.toml` | Everything that shapes the mascot: starting moods, how fast they drift, what each event does to them, when he gets restless and how he picks what to do |
| `crates/judge` | No values of its own: Jev's answer scales follow the label and priority lists in `judge/src/lib.rs` |
| `crates/web-core/src/tuning.rs`, `web/theme.css` `:root` | (planned) front-end restructure |
