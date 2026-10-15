# Rambl architecture (v2)

The one design document. The hackathon build (v1: `static/` front end, `server/src/main.rs`, now deleted) grew fast and broke in ways a design should prevent. v2 is a rewrite toward a small, testable base. Where code disagrees with this file, this file wins; change the file first.

## 1. What Rambl does

You type or say a ramble ("um call mom tomorrow at 3, lab report due Wednesday"). Rambl turns it into tasks, plans them into your week around your fixed events, and keeps the list and the week in agreement. Import events from an `.ics` file (they become ordinary tasks you can edit and move), export the plan as `.ics`.

## 2. Failures v1 had, and the rule that prevents each

| v1 failure | Cause | v2 rule |
|---|---|---|
| "Tomorrow" became Tuesday on Sunday evening | Day math used the UTC date; Jev was told UTC time | All time goes through one `LocalContext { now, tz }`. The user's IANA zone comes from the client. Nothing reads `Utc::now()` or a `.date_naive()` of a UTC value outside `core::clock`. |
| Task time and calendar time disagreed | The list showed the deadline, the calendar showed the placement, and they were fetched separately | The server returns one `Snapshot` (tasks + plan) from every call. The list and the week render from the same snapshot. A task's shown time is its placement. |
| AI put tasks on the wrong day or time | Jev's free-form date answer was re-parsed; code and Jev each did date logic | Jev only **selects** among candidates that code found in the text. Code alone turns the chosen candidate into a date (`dates` crate). |
| Titles lost words or kept filler | Ad hoc prefix stripping | Code proposes title spans; Jev chooses one; one tested cleaner removes filler. |
| Scheduler walked UTC days | Working hours stored as UTC time of day | Working hours are local; the scheduler resolves each local day with its own offset (kept from v1 #77). |
| Imported calendar events were grey, fixed blocks that could not be edited or moved | A separate "busy" type | There is no busy or grey state. An imported event is a normal `Task` (`source: Calendar`, `when: At(start)`, duration from the event) that can be edited, moved, reordered, done or deleted like any other. Re-importing updates by the event's UID. |
| Cancelled / transparent calendar events blocked time; mutex deadlock | Fixes landed late, no invariant tests | Kept fixes, each with a regression test. Locks are never held across a call that locks again (one `App` method per request). |

## 3. Workspace layout

```
crates/
  core/       types shared by server and web (wasm-safe: serde + chrono only)
  dates/      text -> date/time candidates; candidate + LocalContext -> instant. Pure.
  scheduler/  tasks + hours + LocalContext -> Plan (`At` tasks are fixed, the rest flow around them). Pure.
  judge/      trait Judge + JevJudge (HTTP) + FakeJudge (tests)
  capture/    ramble/PDF/email -> Vec<TaskDraft>, using dates + judge
  web-core/   the web page's core (section 8A): its rules and numbers, no DOM, tested natively.
              So far `tuning`, `clock` and `hop`, re-exported by `web/` under their old paths.
server/       axum: Store trait (Sqlite, Memory), App service, thin handlers
web/          Leptos CSR app (trunk), talks only through web/src/api.rs
```

Removed in v2: `static/` (after `web/` reaches parity), sounds and every animation except the mascot's (section 8.1), `/api/message`, `/api/schedule`, spoken replies, `demo/` (its rambles move to `crates/capture/tests/fixtures/`), root `src/`, `Makefile` (one `run-local.ps1` + `run-local.sh` stay).

## 4. Core types (`crates/core`)

```rust
pub struct LocalContext { pub now: DateTime<Utc>, pub tz: chrono_tz::Tz }
impl LocalContext { pub fn today(&self) -> NaiveDate; pub fn local(&self, t: DateTime<Utc>) -> NaiveDateTime; pub fn at(&self, d: NaiveDate, t: NaiveTime) -> DateTime<Utc>; }
pub trait Clock: Send + Sync { fn now(&self) -> DateTime<Utc>; }   // SystemClock, FixedClock

pub enum When { Anytime, DueBy(DateTime<Utc>), At(DateTime<Utc>) } // At = fixed start (pinned)

pub struct Task { id: TaskId, title: String, duration_min: u16, when: When, done: bool,
                  order: i64, label: Label, priority: Priority, source: Source, created: DateTime<Utc>,
                  updated_at: DateTime<Utc>, due_tz: Option<String>,
                  external_uid: Option<String>,   // iCalendar UID for imported events
                  off_planner: bool }             // the user took it off the week (4.2)

pub struct TaskDraft { title, duration_min, when, label, priority, source }      // no id
pub struct Placement { task: TaskId, start: DateTime<Utc>, end: DateTime<Utc> }
pub struct Plan { placements: Vec<Placement>, unplaced: Vec<TaskId>, off_planner: Vec<TaskId> }
pub struct Snapshot { tasks: Vec<Task>, plan: Plan, tz: String, generated: DateTime<Utc> }
pub struct ApiError { code: ErrorCode, message: String }
```

`When::At` always places at that instant; `Anytime` places in the first free slot. One task has at most one placement in v2 (no splitting).

**A `DueBy` task is a day-only task** ("call mom Friday"): it is placed **on its deadline's day**, in the free slot whose start is closest to the user's *usual time*, ending by the deadline and not before now. The usual time is the average local time of day of the user's own timed (`At`) tasks, finished ones included (series occurrences and calendar imports do not count: the user did not pick those times one by one); with none, it is `DaySettings.placement`, noon by default. If that day has no room, the task takes the earliest free slot before the deadline, and only if there is none it is unplaced. Overdue tasks (deadline now or past) are placed from now, first. `DaySettings` is the one per-user pair of times behind this: `deadline` (23:59 by default: where a day-only capture's deadline lands on its day) and `placement` (12:00 by default), both local `HH:MM`, stored per user (`v2_users.day_deadline`, `day_placement`; NULL is the default) and read and changed with `GET`/`PATCH /api/v1/settings`. A deadline of 00:00 is refused. Changing the deadline does not move existing tasks; it applies to what is captured or revised afterwards.

### 4.1 Task order and time semantics

New tasks receive the highest position in the list, so the list is newest first. The scheduler still fills time in list order; a deadline takes precedence over list position. `When::At` is an absolute instant, like a calendar event. `When::DueBy` stores the user's time zone at capture time as part of the due date, so the intended local deadline is preserved if the user later travels.

An unfinished task past its `DueBy` remains in the list marked **Overdue**. The scheduler places it in the next free slot, and Rambo teases the user about it.

### 4.2 Off the planner

`Task::off_planner` is the user saying *not this week*: the task stays in the list with its
deadline and its label, and the scheduler gives it no slot until the flag is cleared. It is a
flag and not a `When` variant, because the task keeps its `when` — a `DueBy` task taken off the
planner is still due then, it just is not being scheduled towards it.

The scheduler reports these tasks in `Plan::off_planner`, apart from `unplaced`, because the two
mean opposite things to the UI: `unplaced` is "wanted on the week, no room left", and
`off_planner` is "not asked for". Every unfinished task the planner is given lands in exactly one
of the three buckets.

The flag outranks a pin: an `At` task taken off the planner leaves the week rather than staying
where it was pinned, because "off the planner" that still shows on the planner is a lie. A task
that is already done is only done — the flag does not report it anywhere.

Set it with `PATCH /api/v1/tasks/{id}` and `{"off_planner": true|false}`; an absent field leaves
it alone. It is stored as `v2_tasks.off_planner` (`INTEGER NOT NULL DEFAULT 0`), and `0` is what
every task written before the column existed reads as, so an old install plans exactly what it
planned before.

## 5. Capture pipeline (`crates/capture`)

1. **Segment** (code): split text into clauses.
2. **Candidates** (code, `dates`): per clause, title spans, date phrases ("tomorrow", "Wednesday", "Oct 30"), time phrases ("at 3", "10am"), duration phrases, and relative phrases: "in 10 minutes", "in an hour", "in half an hour", "in 90 minutes" are `Value::After` (an instant, now plus that many minutes to the minute; the numbers may be spelled, up to sixty). It is unambiguous, so Jev is not asked: the draft is pinned `When::At` that instant whatever day or time Jev picks for the rest. "in 2 days", "in a week", "in a month" are ordinary days (calendar days, so a DST day is not 48 hours; a month is the same day next month, or the last day of a shorter one). Speech mishearings of "add a task" ("attittask", "add it task", "attit task", "at a task") are written as "add a task" before the lead-in is cut (`leadin::strip`). Each candidate already carries its resolved value from `LocalContext`. The title candidates include the clause without a lead-in such as "Add a task to" (`capture::leadin`, measured in `docs/experiments/title-leadin.md`); Jev chooses, and a judge with no opinion takes the first, which is the grammar's cut.
3. **Judge** (one Jev request per capture, all questions in parallel): per clause `is_task` (Noul), `title` (Choice of spans), `day` (Choice of date candidates + none), `time` (Choice of time candidates + none), `duration` (Choice), `label` (Choice), `priority` (Score). Jev gets the local time as text for context only.
4. **Resolve** (code): build `TaskDraft`. Day + time -> `When::At`; day only -> `DueBy(day at the user's deadline setting, 23:59 by default, local)`; neither -> `Anytime`.

Without a Jev key, `FakeJudge`'s deterministic rules pick the first candidate. It takes a clause as a task when it names an obligation ("need", "have to", ...) or is shaped like an instruction: past fillers ("oh, and", "please", "can you", "remind me to"), its first word is not a pronoun, article, preposition, conjunction, auxiliary or negation, nor a gerund or past form ("walk the dog" is a task, "watching videos" is not); then v1's chit-chat rules drop observations, wishes, feelings and past events. Tests use it, so the pipeline is tested end to end without the network.

## 6. HTTP API v1

All JSON. Every request carries the header `X-Rambl-TZ: America/New_York` (from `Intl.DateTimeFormat().resolvedOptions().timeZone`). Every mutating call returns the full `Snapshot`. Errors are `{ "error": { "code", "message" } }` with a fitting status.

**Accounts.** Rambl is multi-user. Every request carries either `Authorization: Bearer <session token>` for signed-in users, or `X-Rambl-Anon: <anon_id>` for anonymous users. The token or anon_id names the user; **every task read and write is scoped to that user, in the store, not in the handler** — `Store` methods take the user id, so a handler cannot forget it. In local mode (`GOOGLE_CLIENT_ID` unset), the server uses the fixed local user for all requests regardless of headers. Each account has a persisted plan: anonymous, free, or paid. Daily Jev caps are `RAMBL_JEV_DAILY_CAP_ANON` (default 30), `RAMBL_JEV_DAILY_CAP` (default 200), and `RAMBL_JEV_DAILY_CAP_PAID` (default 1000); the admin console changes plans with `plan <email|id> <anon|free|paid>`, and the `/admin` page does the same over `POST /api/v1/admin/plan` for anyone whose email is in `RAMBL_ADMIN_EMAILS`. When a cap is exceeded, or the user or client address is blocked, capture falls back to `FakeJudge` with `jev_fallback: true`.

**Client address.** The address a request came from is the socket peer, unless the peer is a trusted proxy — loopback, plus whatever `RAMBL_TRUSTED_PROXIES` lists — and then it is the `CF-Connecting-IP` header, and only if that parses as an address. Anything reaching the server directly therefore cannot name its own address and so cannot step past the per-address limits and blocks. The server listens on `127.0.0.1` unless `RAMBL_BIND` says otherwise, so "directly" means a caller on this machine, or an operator who has deliberately widened the listener.

Anonymous users do not sign in. Their account and tasks are deleted after 30 days without a visit (`last_activity_at`, refreshed by a visit at most once an hour and cleared by the daily backup task, only after that backup succeeded); signed-in accounts are never deleted. *(planned)* Rambo nudges them to sign in before then.

| Method and path | Body | Returns |
|---|---|---|
| `GET /api/v1/config` | none | `{google_client_id}`, the client id from `GOOGLE_CLIENT_ID`, or null. When null the page shows no sign-in. |
| `POST /api/v1/auth/google` | `{credential}` (a Google ID token) | `{token, user}`. The token is checked against Google's `tokeninfo`; the audience must equal `GOOGLE_CLIENT_ID` and `email_verified` must be true. Sessions last 90 days. |
| `POST /api/v1/auth/signout` | none | 204. Ends this session only. |
| `GET /api/v1/me` | none | `{id, email, name, picture}` |
| `GET /api/v1/settings` | none | `{deadline, placement}`, this user's day-only times as `HH:MM` (defaults `23:59` and `12:00`). |
| `PATCH /api/v1/settings` | `{deadline?, placement?}`, each `HH:MM` | The settings that now hold. Absent fields stay; `deadline` `00:00` is a 400 and changes nothing. |
| `DELETE /api/v1/me` | none | 204. Removes the account, its sessions and every task it owns. |
| `GET /api/v1/snapshot?from=YYYY-MM-DD&days=7` | none | `Snapshot`. The window bounds the plan (today and 7 days by default); the task list is always the whole list, and pinned tasks keep their placement whatever the window is. |
| `POST /api/v1/capture` | `{text}` or multipart `file` (pdf, eml, txt) | `{created: [id], snapshot, jev_fallback}`. `jev_fallback` is true once the user is past `RAMBL_JEV_DAILY_CAP` captures today (200 by default), when Jev is not called and `FakeJudge` judges instead. |
| `POST /api/v1/tasks` | `TaskDraft` | `Snapshot` |
| `PATCH /api/v1/tasks/{id}` | any of `title, duration_min, when, done, label, priority, off_planner` (4.2), plus `updated_at` (the version the page last saw) | `Snapshot`; `409 stale_update` if the task changed since `updated_at` |
| `POST /api/v1/tasks/{id}/revise` | `{instruction}` (goes through the capture pipeline for one clause) | `Snapshot` |
| `POST /api/v1/tasks/restore` | `Task` as it was before its delete (the scrap undo); stored under a fresh id with the create checks; its place is kept within one step of the list, an unknown deadline zone is re-anchored, a future `created` is clamped to now and `external_uid` is dropped; a repeat's day is refused | `Snapshot` |
| `PUT /api/v1/tasks/order` | `{ids: [...]}` (full order) | `Snapshot` |
| `DELETE /api/v1/tasks/{id}` | none | `Snapshot` |
| `POST /api/v1/import` | multipart `file` (.ics) **or** JSON `{tasks: [Task]}` (matching `GET /api/v1/export`) | `{created: [id], updated: [id], snapshot}`. ICS: events become tasks (`At`, `source: Calendar`); same UID updates the existing task. A moved instance of a repeating event (its series' UID plus a `RECURRENCE-ID`) is matched on both, and a UID repeated within one file keeps its last copy. Cancelled and transparent events are skipped. Overlapping `At` tasks are allowed and drawn side by side. JSON: every imported task gets a fresh id, the caller as owner, and newest-first ordering, and must meet the same title and duration limits as any task. Bad JSON or an invalid task returns 400 and writes nothing (all or nothing). Body capped at 2 MiB, max 500 tasks per import. |
| `GET /api/v1/calendar.ics` | none | iCalendar |
| `GET /api/v1/export` | none | `{tasks: [Task]}` as an attachment download (`rambl-export-YYYY-MM-DD.json`, `Cache-Control: no-store`). The date is the caller's own, from the request zone, so it is the caller's claim rather than the server's clock. Every task the caller owns, with all fields; scoped to the caller like every read. A caller with no credentials is a fresh anonymous account, so the download is 200 with an empty list rather than 401. |
| `GET /admin` | none | The admin page (HTML). 404 when `RAMBL_ADMIN_EMAILS` is unset, and for any signed-in user whose email is not in it. |
| `GET /api/v1/admin/usage` | none | Today's captures by user (id, plan, captures) and by address, plus totals per `anon`/`free`/`paid` plan. 404 for anyone who is not an admin. |
| `GET /api/v1/admin/blocks` | none | Everything currently blocked, `[{kind: "user"|"ip", value}]`. 404 for anyone who is not an admin. |
| `POST /api/v1/admin/block` | `{kind: "user"\|"ip", value}` | Block a user (id or email) or an address. 404 for anyone who is not an admin. |
| `POST /api/v1/admin/unblock` | `{kind: "user"\|"ip", value}` | Lift a block placed the same way. 404 for anyone who is not an admin. |
| `POST /api/v1/admin/plan` | `{who: email\|id, plan: anon\|free\|paid}` | Move a user to another plan. 404 for anyone who is not an admin. |

The server is the only planner: the client never computes placements.

### 6.1 Concurrent writes

Task edits use an `updated_at` field and last-write-wins per task. The server sets `updated_at` to its own clock on every accepted change. A `PATCH` or `revise` that carries an `updated_at` older than the stored one is refused with `409 stale_update` and changes nothing, so the page reloads and redoes the edit; one equal to or newer than the stored value, or none at all, is accepted. The check is in `App`, so both stores behave the same. A reorder conflict is handled by re-sending the full order, rather than merging partial positions.

### Configuration

| Variable | Default | Meaning |
|---|---|---|
| `RAMBL_DATA` | `data/rambl.db` | The v2 SQLite file. |
| (config file) | `rambl.env` | Read from the working folder (the exe's folder on a server). Real environment variables win. |
| `RAMBL_DEFAULT_TZ` | `America/New_York` | Zone for requests with no `X-Rambl-TZ`. `TZ_NAME` is read behind it. |
| `RAMBL_PORT` | `3160` | HTTP port. `PORT` is read when it is unset. |
| `RAMBL_BIND` | `127.0.0.1` | Address to listen on. Loopback, because the Cloudflare tunnel on this machine is the way in: it keeps the server off the LAN and stops Windows prompting for firewall access every time a new build path runs the exe. Set `0.0.0.0` or one address to serve more, which does not by itself let a caller name their own address (a peer's `CF-Connecting-IP` is only read from a trusted proxy) but does mean a tunnel running elsewhere has to be named in `RAMBL_TRUSTED_PROXIES`, by its own address and never a client's network. A value that is not an IP address stops the server rather than falling back. |
| `GOOGLE_CLIENT_ID` | unset | The OAuth client ID tokens must be minted for. Unset means local mode: one user, no sign-in. Set but blank stops startup with an error, so a public server never falls into local mode by accident. |
| `RAMBL_JEV_DAILY_CAP` | `200` | Captures per user per day before capture falls back to `FakeJudge`. |
| `RAMBL_JEV_DAILY_CAP_ANON` | `30` | Captures per anonymous user per day before capture falls back to `FakeJudge` with `jev_fallback: true`. |
| `RAMBL_JEV_DAILY_CAP_PAID` | `1000` | Captures per paid user per day before capture falls back to `FakeJudge`. |
| `RAMBL_ANON_IDS_PER_IP` | `20` | New anonymous ids one client address may be issued per day. |
| `RAMBL_SIGNINS_PER_IP` | `60` | Google sign-in attempts one client address may make per day; past it the answer is 429. |
| `RAMBL_LOCAL_LOGIN` | unset | `1` adds `POST /api/v1/auth/local` and the page's "Continue locally" button: a session for the fixed user `local-login`, with no Google. Test sites only. The server will not start with it while `RAMBL_TRUSTED_PROXIES` is set, prints a warning when it does start, and the endpoint answers `403` unless the socket peer is loopback or private-LAN (10/8, 172.16/12, 192.168/16, `::1`, `fc00::/7`) and the request has no `CF-Connecting-IP` or `X-Forwarded-For`. `GET /api/v1/config` reports `local_login` under the same test. |
| `RAMBL_ADMIN_EMAILS` | unset | Comma-separated emails allowed to use `/admin` and its API. Unset or empty disables the admin page (404). |
| `RAMBL_TRUSTED_PROXIES` | unset | Comma-separated proxy addresses or networks (for example `10.0.0.1, 10.0.0.0/8`) whose `CF-Connecting-IP` names the real client. Loopback is always trusted, which covers a tunnel on the same machine. From any other peer the header is ignored and the socket address is used, so a client reaching the server directly cannot name its own address. |

Working hours: 08:00 to 23:59:59 local time, every day.

## 7. Server (`server/`)

- `Store` trait: `tasks(user)`, `put(user, Task)`, `delete(user, id)`, `set_order(user, ids)`, `find_by_uid(user, uid)`, plus the account calls, `captures_today`, `record_capture`, `block`/`is_blocked` and `usage_today`. The user id is a parameter rather than a filter applied afterwards, so scoping cannot be forgotten. Task ids are random (128 bits) and unique across all users; `put` refuses an id another user already holds rather than taking it over. `SqliteStore` for real, `MemoryStore` for tests; the same suite runs against both.
- `auth` module: Google sign-in, sessions and the `TokenVerifier` trait. Sign-in is verified **before** the store lock is taken — a handler holding a `MutexGuard` across an `await` has a non-`Send` future, which axum rejects.
- `App { store, judge, clock }`: one method per endpoint, each taking the user id. It loads once, mutates and returns the snapshot built from that same list inside one critical section, so a snapshot is never half-updated and a write is never read back twice.
- Handlers only parse input, build the `LocalContext` from the header and `clock`, call `App`, and map errors.
- Config from env: `TYPESAFE_API_KEY`, `PORT`, `RAMBL_BIND` (address to listen on), `RAMBL_DATA` (db path), `RAMBL_DEFAULT_TZ` (when the header is missing).

### 7.1 Recurring tasks

A **series** repeats weekly on chosen days, optionally until a date. The stored data never lists every week; the server **expands occurrences** for the window the planner asked for (`GET /api/v1/snapshot?from=&days=`) at snapshot time and shows them as normal task rows. Marking an occurrence done, skipping it or moving it writes an **exception** row, so the expansion stays correct after a restart. The store, `App`, the endpoints, the ICS import and their tests are in code; the capture wiring and the web UI are *(planned)*.

**An undone past occurrence is not carried forward.** A routine does not pile up: a Monday that came and went is gone, not an overdue row waiting to be caught up. Only one-shot tasks carry that rule (a `DueBy` in the past becomes overdue).

**Types (`core`).**

- `Recurrence { weekdays: Vec<Weekday>, interval: u32, until: Option<NaiveDate> }` — the rule. `interval` is 1 or more: every `interval` weeks (weekly) or every `interval` days (daily). Non-empty `weekdays` means a weekly rule; empty `weekdays` means a daily rule. `until` is the last day an occurrence exists, inclusive. `chrono::Weekday` is `Eq` and `Hash` but not `Ord`, so this cannot be a `BTreeSet<Weekday>` as first sketched: the days are a sorted, duplicate-free `Vec<Weekday>` (Monday first), which is what `dates::RecurrenceMatch::weekdays` is too.
- `Series { id: SeriesId, user, title, duration_min: u16, rule: Recurrence, start_day: NaiveDate, time_of_day: Option<NaiveTime>, zone: String, external_uid: Option<String>, created, order }` — one per repeated thing. `time_of_day` comes from the capture or import; without it an occurrence floats (`Anytime`) and the planner gives it the first free slot it can find, like any flexible task (there is no per-week notion of "first"). `zone` is where the weekly time lives, like `due_tz` for a deadline. `order` is its list position, the same key as `Task::order`.
- `SeriesException { series: SeriesId, date: NaiveDate, kind: ExceptionKind }` with `ExceptionKind::{Done, Skipped, Moved { to: When, to_tz: Option<String> }}` — one per deviating occurrence, keyed by local date. A moved `DueBy` keeps the zone the move was made in, like `Task::due_tz`.
- `Occurrence { series: SeriesId, date: NaiveDate, when: When, status: OccurrenceStatus, due_tz: Option<String> }` — the pure expansion result, with `OccurrenceStatus::{Open, Done, Moved}`. Skipped days produce no occurrence at all.

**Expansion (pure, in `scheduler`).** `expand(series, window_start, window_end_exclusive, exceptions, ctx) -> Vec<Occurrence>`: for each day in the window that `rule.covers(start_day, date)` returns true for, produce one `Occurrence`. For a weekly rule, the day's weekday must be in `rule.weekdays` and the week number (from `start_day`) must be a multiple of `interval`. For a daily rule, the day offset from `start_day` must be a multiple of `interval`. Each occurrence is on/after `start_day` and on/before `until`. Its `when` is `At(ctx.at(date, time))` in the series' zone (each occurrence is re-derived in the zone stored on the series, so DST shifts the UTC instant but never the local 07:00), or `Anytime` without a `time_of_day`. Exceptions reshape the output: a `skipped` date yields nothing; a `done` date yields the occurrence with `status: Done` (and no placement); a `moved` date yields the occurrence at its new `When` with `status: Moved` and the move's zone, its id still naming the original date so the exception row stays addressable. A zone that will not parse falls back to the request's zone rather than losing the series.

**Snapshot.** A snapshot expands every series for the window it was asked for and appends the occurrences as ephemeral `Task`s: skipped days never appear; done ones appear marked done; moved ones sit at their new time. Each row gets `id` = `"{series_id}:{date}"` (the original date for a moved one), `source` = a new `Source::Series`, `order` and `created` = the series'. The plan is built from these rows together with the stored tasks, so open occurrences occupy real slots and the week grid shows them with no new plan type. `Placement.task` already holds a `TaskId`, and an occurrence id is a valid `TaskId`. Occurrence ids never enter the `v2_tasks` table. A mutation answers with the default window (today and 7 days, the one the page draws); a client looking at another window re-fetches it.

**Store (migration 8).** Two tables, both user-scoped by `user_id` (the column name every v2 table shares):

- `v2_series`: `id TEXT` primary key, `user_id TEXT`, `title TEXT`, `duration_min INTEGER`, `weekdays TEXT` (e.g. `"124"` = Mon/Tue/Thu, `chrono::Weekday` numbers from Monday=1; empty for daily rules), `interval INTEGER` (weeks for weekly, days for daily; default 1), `until TEXT` nullable (`YYYY-MM-DD`), `start_day TEXT`, `time_of_day TEXT` nullable (`HH:MM`), `zone TEXT`, `external_uid TEXT` nullable, `created TEXT`, `"order" INTEGER`. Index on `user_id`, unique on `(user_id, external_uid)`.
- `v2_series_exceptions`: `series_id TEXT`, `user_id TEXT`, `date TEXT`, `kind TEXT` (`"done"` / `"skipped"` / `"moved"`), `moved_when TEXT` nullable (the moved-to `When` as JSON), `moved_tz TEXT` nullable, primary key `(user_id, series_id, date)`.

`SCHEMA_VERSION` 7 → 8, `MIGRATION_8` creates both tables. `Store` gains `series`, `put_series`, `delete_series` (drops the series' exceptions), `exceptions`, `put_exception` (an upsert, so marking the same day twice is one row) and `delete_exception` (clearing one puts the day back to the rule). Series and exceptions are scoped and deleted exactly as tasks are: `delete_user`, `expire_anonymous` and `adopt_anonymous` all carry them. `App` methods: `create_series`, `update_series`, `delete_series`, `patch_occurrence`, `clear_occurrence`, each returning a snapshot in the same critical section as every other mutation.

**API (JSON, under `/api/v1`, as every other v2 route).**

- `POST /api/v1/series` / `PATCH /api/v1/series/{id}` / `DELETE /api/v1/series/{id}` — the series body; `updated_at`-style optimistic concurrency is not needed because only the series row itself changes. A rule that names no day, or one that ends before it starts, is a `400` rather than a series that could never fire. Absent fields in a `PATCH` stay as they are; `time_of_day` and `until` can be cleared by sending `null` (an all-day, never-ending series again).
- `PATCH /api/v1/series/{id}/dates/{date}` with `{"kind": "done" | "skipped" | "moved", "when": <When object>}` (`when` only for `moved`, and a move without one is a `400`); `DELETE` on the same path clears the exception. The date must parse and must be a day this series' rule actually covers (`Recurrence::covers`) — not merely one inside the window — so an exception can never be written for a day the series never repeats. A client id is never trusted as an identity: the series is looked up among the caller's own rows, so another person's id is a `404` (the same rule as everywhere else). A task endpoint handed an occurrence id (`"{series_id}:{date}"`) answers `400` and names this endpoint, rather than reporting a task that is not there.
- `Snapshot` gains `series: Vec<Series>` alongside the ephemeral occurrence rows, so the web UI can draw the rule without a second call.

**Capture.** *(planned — the scanner itself is in code, `dates::recurrence`; wiring it into `capture` is what is left)* It runs on each clause **before** `dates::candidates`, carrying the days, an optional time, an optional end and an optional start day, plus the byte spans it consumed so the title can drop them. It accepts:

- `every <weekday>` / `<weekday>s` / `every <weekday> and <weekday>…` / `weekly on <weekday>[, <weekday>]` — the existing `WEEKDAYS` name table (`crates/dates/src/phrases.rs`) supplies the names; plural stripping (`mondays`, `tuesdays`) and three-letter prefixes (`tue`, `fri`) are added there, not in `capture`.
- `every weekday` = Mon–Fri; `every day` = all seven.
- `until <day phrase>` — anything the existing `day()` resolver already accepts ("until December 15", "until Friday"), plus two phrases the scanner adds and `day()` cannot hold: a bare month after `until` ("until December") means the last day of the month **before** it, and `end of the month` = the last day of the current month. A bare month stays out of `day()`, where it would make "December" a date of its own, and "end of the month" is four words, longer than any window `candidates` reads.
- `at 7am`, `at 19:00` etc. reuse the existing time candidates.
- `starting <day>`, `from <day>`, `beginning on <day>` — a one-shot date beside a marker is the first week of the series.
- No match — "every", "always", a rhythm this scanner does not read ("every month", "every other Monday") or a weekday with no `every`/`weekly on` marker — leaves the clause to the normal one-shot path ("a thing next Monday" is not a series).

The match wins over one-shot day phrases: `gym every Monday and Thursday` produces a series, not two `At` tasks. Jev still judges that the clause is a task; the scanner decides it repeats. A clause with both a one-shot date and a recurrence marker ("submit the report every Friday starting October 30") is a series whose `start_day` is that date.

**Done / skip / move.**

- **Done:** `PATCH …/dates/{date}` `done` — the row stays in the snapshot marked done while its date is inside the window; once the date falls out of it the row scrolls away like any done task. The exception row is what keeps that state across restarts.
- **Skip ("not today"):** `skipped` — the row disappears from this week; nothing else changes.
- **Move:** `moved` with a new `When` — the original slot is freed (the day's occurrence is emitted at the new time instead, `status: Moved`) and a normal-looking row appears there. The dialog is the same one the web app uses for a task edit, writing through the occurrence endpoint instead of `PATCH /api/v1/tasks/{id}`.
- Editing the series (`PATCH /api/v1/series/{id}`) changes future occurrences only; past exceptions keep their dates.

**ICS import.** In `ics_import.rs`, `parse_calendar` reads one export into three things: the one-shot events, the series among them, and a note for every rule it could not follow. A `VEVENT` with `RRULE:FREQ=WEEKLY` becomes a series and is *not* imported as a task: `weekdays` from `BYDAY` (or DTSTART's own weekday when there is none), `interval` from `INTERVAL` (default 1), `until` = the `UNTIL` instant's date **in the event's TZID zone** (Outlook writes the local 23:59:59 as UTC, so `UNTIL=20261215T035959Z` is the 14th in New York), `COUNT` mapped to the date of its last occurrence (the earlier of the two ends wins when a rule names both), `start_day` = DTSTART's local date, `time_of_day` = DTSTART's local time (an all-day event floats instead), `zone` = the TZID, `external_uid` = the VEVENT UID so a re-import updates that series rather than adding a second one, keeping the id, list position and exceptions the user already has. `EXDATE` dates become skipped occurrences, so an excluded week stays excluded after a restart. `FREQ=DAILY` (with optional `INTERVAL`) also becomes a series: `weekdays` is empty, `interval` is the day step. `INTERVAL` counts calendar weeks (Monday first, from the week that holds DTSTART), so a start on a Wednesday with `BYDAY=MO,WE` every 2 weeks fires Wed 14, Mon 26, Wed 28; `COUNT` is turned into its `until` by `Recurrence::nth_occurrence`, which agrees with `covers`. `FREQ=DAILY;BYDAY=…` with no `INTERVAL` is the weekly rule on those days (weekdays only stays weekdays only); with an `INTERVAL` it has no equivalent and is reported. An `INTERVAL` that is not a whole number from 1 to `IMPORT_MAX_INTERVAL` (1000), and a `WKST` other than `MO` on a weekly rule with an `INTERVAL`, are reported and the event imports as one task.

Any other `FREQ` (monthly, yearly), an ordinal `BYDAY` (`2FR`), and any part we do not read (`BYSETPOS`, `BYMONTHDAY`, `BYWEEKNO`, …) import as today — one `At` task at the first occurrence — and the import answer lists the event and the reason under `unmapped`, so the client can say the rule was not kept rather than the day silently disappearing. `ImportResult` gains `series: Vec<SeriesId>` and `unmapped: Vec<String>`, both skipped when empty.

**Web UI.** *(planned)* — a series shows as a list row with the title and a recurrence chip (`Mon · Thu · until Dec 15`). Its occurrences look like ordinary rows in the list and the week grid, with the date in place of a due date. The task dialog gains days-of-week checkboxes and an until field for a series, and occurrence rows gain **Done**, **Not today** (skip) and **Move**. No new request paths beyond the endpoints above; `api.rs` still funnels everything into the one snapshot signal.

**Out of scope (stated so it stays out).** Monthly and yearly rules; `BYSETPOS`/`BYMONTHDAY`; editing one occurrence's title or duration independently of the series (an occurrence inherits the series until it is moved); reordering occurrences relative to tasks beyond what `plan` already does; timezone changes for a series after creation (the series keeps its zone); series in the paid-tier or ad paths.

**Test plan.**

- `dates`: fixed-clock table rows for the scanner — `gym every Monday and Thursday`, `every weekday until December` (until = Nov 30 of the current year), `weekly on Tue and Fri at 6pm`, `standup every day at 9`, and the negatives (`a thing next Monday` is not a series). DST: a 07:00 America/New_York series expands at 11:00Z and 12:00Z across the March change.
- `scheduler`: `expand` invariants — no occurrence on a day outside the weekday set, none after `until`, an exception of every kind honoured, a moved occurrence at its new instant.
- `capture`: golden fixtures with `FakeJudge` — a recurrence clause produces a series draft, a one-shot clause stays a task, a mixed clause uses `start_day`.
- `server` (HTTP, `MemoryStore` + `SqliteStore` shared suite, `FixedClock`): create a series → the snapshot shows the expected occurrence rows; done/skip/move an occurrence → the snapshot reflects it; restart (fresh `App`) → done occurrences still done; move frees the original slot in the plan; weekly ICS import → a series, not one task; a non-weekly `RRULE` → one `At` task and `unmapped`; a made-up occurrence id (`other-series:2026-01-01`, a foreign series id, a date outside the rule) is refused.
- `web`: a fixture snapshot with a series renders the chip and the occurrence rows in the list and week grid.

## 8. Web (`web/`, Leptos CSR)

Plain and fast: a text box with a mic button (Web Speech API via `web-sys`), the task list, and a 7-day week grid. Light and dark follow the OS. The look is the parchment theme: tokens and rules in `docs/THEME.md`, all CSS in `web/theme.css` (linked from `web/index.html` with `<link data-trunk rel="css">` and copied into `web/dist`). The one texture is the paper itself (`web/assets/theme/paper.svg`, dark variant `paper-dark.svg`) on the page background; if a graphic is missing the flat paper colours carry the page. No animations or sounds, except the mascot below. One `api.rs` module makes every request and stores the returned `Snapshot` in one signal; the list and the week are derived from that signal only. Reorder by drag sends `PUT /tasks/order`. Built by `trunk build --release` into `web/dist`, which the server serves.

### 8.0 The engine (`web/src/engine/`)

Everything on the page that moves runs on one loop with one animation clock. `engine::on_tick` runs a job on every frame until it returns `Flow::Stop`, `engine::next_frame` runs one once the page has laid out, and `engine::every` runs one every so many ms. A job sees `Tick { now, steps }`. `now` is the browser's animation time in ms: what frames and pointer events are stamped with, never the wall clock. `steps` is how many fixed `ENGINE_DT_S` (1/120 s) steps have come due: time left over carries to the next frame, a late frame takes at most 8, and the first frame after the loop has slept takes none. Physics steps by exactly that (the page turn's spring does), so motion is the same at any frame rate. The loop asks the browser for frames only while a job runs on every frame and for one timeout for the earliest timer otherwise, so at rest it costs nothing. The rules are pure and tested natively (`engine/sched.rs`); `engine/run.rs` is the only code that asks the browser for frames or timers, and a test fails on `request_animation_frame`, `set_interval`, `Interval::new` or `Date::now` anywhere else. `Date::now` stays only where the user's day is read (the planner's now line, what is overdue, the hour the brain hears). On the loop today: the ink, the page turns, Rambo's hops, the brain's tick, the planner's now line and its edge slide. Writing a row (`tasks/write.rs`) still steps on its own timers until it moves onto the loop (planned). `engine/world.rs` holds Rambo as an entity of the engine, and `rambo.rs` draws what it says. The brain stays `mascot::transition`; the rest of the page sets his state, and the world's own timers and landings set it back. `behaviour` gives each state a drive: `HopTo` a target with `hop.rs` (where `mascot::hops_there`), landing where the target is now; `Ride`, a critically damped spring after the finger at `RIDE_OMEGA`, while held; `Throw`, the slide of a throw (`engine/fling.rs`) inside the screen less half his size (`Anchors::walls`); or `Follow` his spot exactly (a splat and its daze stay where the throw left him) (inking, his nib stays on the finger). From flying to writing a row he reaches the row's first letter on the ride spring, never by a jump. With reduced motion he is on his spot in every state: no hops, no ride. Every run that sets off on a change of state starts with his notice beat (`mascot::notice`, `NOTICE_BEAT_MS`), and the splat and the daze are timed in world time. His pose comes from the entity too (`engine/look.rs`, `look::pose`): sunk `--nest-sink` into the nest at home; held, tilted like a pen in a hand with a slow pulse; dazed, tipped over; the splat a wobble that dies into the daze's tilt; hopping or carried, leaning into where he goes (`LEAN_PER_PX_S`, at most `LEAN_MAX_DEG`), the lean eased after his speed every step over `LEAN_EASE_MS` (`look::eased_sway`) so it never jumps as a hop lands; caught under the leaf, upright; writing, smaller at `--write-lean`, a row turning about his nib and wearing his hand (`--nib-dip`, `--nib-tilt`). On every change of state he eases in and out of the pose he had into the new one over `POSE_BLEND_MS` (`WRITE_BLEND_MS` into or out of the writing pose, a whole `--write-lean` turn), so nothing snaps (flying into writing a row used to step 18 px); with reduced motion the pose changes at once, with no pulse and no wobble. He is a job on the loop only while something can change on its own: when he stands exactly on what he follows (settled in his nest, asking, inking, writing a row) the loop sleeps until his state or his spot changes; the splat and the daze keep it running until their timers end them, and a change of pose until it has eased in.

### 8.0.1 One body: physics for Rambo and the paper (in progress)

**Problem.** Motion today is picked per state: `world.rs:Drive` swaps between `Follow`, `Ride` (a spring), `HopTo` (`hop.rs`, a scripted arc) and `Throw` (`fling.rs`), so momentum is lost at every switch, as when a drag ends in a scripted hop. Beside the world there were two springs (the nest's and the page leaf's; now both are `Spring1`), smoothstep glides, about 20 wasm timers outside the engine clock, CSS lengths kept in step with Rust by hand, and reduced motion checked in a dozen places (hive/research/motion-inventory.md).

**Rule.** Anything that moves physically is a `Body`: position, velocity and an optional height above the page. Every engine step it gets the forces its owner set, and one integrator (semi-implicit Euler at `ENGINE_DT_S`) moves it. States never move a body; they only set its forces and constraints. Velocity is never reset by a change of state, so momentum always carries over.

**Module** (`web/src/engine/body.rs`, pure and tested natively, about 150 lines). In code now: `Pull` and `Spring1`. The rest is planned:
- `Body { pos, vel, z, vz }`, `Body::step(&Forces, walls, dt)`, `Body::impulse(dv)`.
- `Forces`: an optional `Spring { to, omega, zeta }` (for a moving `to`, such as the pointer or the nest), `friction`, `gravity` (on `z` only: the page is the ground) and an optional `Pin(to)` for exact tracking (the nib on a row). A pinned body still reports the velocity it moved at, so letting go of a pin carries it on.
- `Spring1 { x, v }` (in code): one scalar spring, `step(target, pull, accel, dt)`, `kick(dv, cap)`, `settled(target, near, slow)`. The nest's sag and rock (`engine/nest.rs`), his dance sway (`engine/dance.rs`) and the page leaf (`flip::fall`, which adds the leaf's walls and the push from underneath) all step it; the ribbon's spring-back is planned.
- `Motion::Reduced`: the one reduced-motion switch, read once by the world. A reduced body snaps to its spring's target or pin and never takes an impulse.

**Rambo's states only set forces:**

| State | Forces |
|---|---|
| Dragging | stiff, critically damped spring to `held_at(pointer)` |
| Let go (any speed) | none: he slides on his velocity with friction and the walls. A smack at `FLING_SPLAT_SPEED` splats as today. Once slow, he is sent home. A gentle drop is just a slow throw, so there is no separate hop. |
| Home run | after `HOME_WAIT_MS`, a series of impulses: each hop is the `vz` and ground velocity that lands on the next waypoint of a bowed path, blended with the velocity he still has. Touching down (`z` reaches 0 while falling) leaves an ink dot, kicks the nest when he lands in it, and fires the next hop. |
| Home (settled) | spring to the nest anchor; idle sway as small random kicks on a `Spring1` tilt |
| Writing (caret or row) | `Pin` to the nib point |
| Dazed, asking, noting | friction only (he lies where he stopped); the paper's sweep stays a velocity |
| Covered | spring along the leaf's free edge |

Squash, stretch and lean are read from `vel` and `vz` (`look.rs`), not scripted per hop.

**Paper strips** (the pinned scraps, list rows lifted or released, the slip ghost) become bodies in the same world: held, a spring to the pointer; let go, a spring to their slot or the pin. That removes their `TimeoutFuture` lays and flies and the inline transform that fights a CSS transition (inventory items 1 and 3 for `scrap.rs` and `row.rs`).

**Kept as they are:** CSS keyframes for drawing-only effects (breathing, mood pictures, the scribble picture), and ink ageing. The human allows fades only for ink and the patch (the tear and its patch were cut, #413).

**Order of small PRs.** Each PR is green on its own and leaves behaviour the same unless it says otherwise.
1. (done) `body.rs` with `Pull` and `Spring1` and their tests. `nest.rs`, `dance.rs` and `flip.rs` are ported onto them and the two old springs are deleted. No visible change. `Body` and `Forces` arrive with their first user in PR 2, and `Motion` in PR 4, so no code waits unused.
2. Rambo's ground motion: `Drive` becomes `Forces` on one `Body`. The drag spring, release keeping velocity (a gentle drop slides and stops) and the pin for writing. `fling.rs` keeps the walls and the splat rule and loses its own integrator.
3. The hop home as impulses with `z` and gravity, and ink dots on landing. Delete `hop.rs` and `Run`. The parked `emilio/hop-home` branch has the bowed path and the wait.
4. Idle sway and one reduced-motion switch through `Motion`.
5. Paper strips: the pinned scraps' lay and fly, and a lifted row's release, as bodies. Delete their wasm timers and inline transitions.
6. The remaining sequencing timers (capture, lamp, glyph pacing, `take_off`) move onto `engine::every`. This is clock work, not physics, so it is listed only to close inventory item 1.

**Not in scope:** the page turn's geometry (it only takes `Spring1`) and the ribbon's length (PR 1 takes its spring-back if it is small).

**Risk.** Release-keeps-velocity changes the feel of a gentle drop: Fable reviews PRs 2 and 3. The freeze question stays open; if it comes from a mechanism switch, PR 2 removes the switch.

### 8.1 Rambo, the mascot

The pen character (`web/assets/rambo/`) is named **Rambo**. He rests in his nest, the dish at the end of the text box.

- **Nest and pen are separate:** the nest is drawn in two layers (`web/assets/nest/`): its back on the nest button, which sits in the page, behind Rambo; its front rim as a fixed layer of its own (`.nest-rim`) in front of him, placed on the nest's centre as he measures it. Their order is data in `:root` (`--layer-capture` < `--layer-rambo` < `--layer-nest-rim`); at home he rests `--nest-sink` below the centre, his tip inside the rim; Rambo is a `position: fixed` layer whose one transform the engine writes (`engine::Look::transform`: where he is, then his pose), so carrying him away never carries the dish. At home he follows the nest: he measures it again on any scroll (window or panes), resize, page size change, a change to the root's style, where the phone's keyboard inset lives (a `MutationObserver`, so he follows `--keyboard-inset` whatever sets it, not only the viewport events that usually do), and once the fonts load (`web/src/rambo.rs:on_page_moved`), and on the way home he aims at where the nest is now. On a phone the keyboard covers the bottom of the screen: `capture.rs:ride_the_keyboard` sets `--keyboard-inset` on the root from the visual viewport (`keyboard_inset`: the layout's height below the visible part, 0 when pinch-zoomed), and the fixed capture bar sits that far up, so the nest, its rim and Rambo ride up above the keyboard and back down when it closes. No CSS rule may move the button: no transform, animation, translate, rotate or scale on it (a test in `mascot.rs` checks theme.css). Moods animate only the drawing inside it. While he listens (`talking`), `.listening-dim` turns the light down over the whole screen at `--layer-dim`, between the page's notes and him (`--listening-dim`, eased over `--listening-dim-ms`, at once with reduced motion); it has `pointer-events: none`, so drops and taps still land on what is under it.
- **Only to rows on screen:** the pen flies to write a new row only when that row shows on screen (`pen::on_screen`); a row on the other pane of a phone, or scrolled away, is shown without him.

- **Writing pen:** while typing, Rambo's pen follows the caret in the capture box. Two seconds after typing stops, the pen flies home. When new tasks are created, the pen visits each new row and writes its title under the nib, glyph by glyph, a glyph being a grapheme cluster so an accent or an emoji is one (`pen::glyphs_of`, `pen::schedule`, paced by `WRITE_PER_GLYPH_MS` between `WRITE_MIN_MS` and `WRITE_MAX_MS`), one span a glyph whose ink fades in over `INK_FADE_MS` as the nib passes its middle (`pen::stroke_at`, `pen::inked_count`), so wrapped lines need nothing of their own. His hand presses through each glyph, lifts over each word gap and wobbles a little, moving smoothly from one glyph into the next (`pen::hand`); he touches down when he lands (`pen::touch_down`) and flicks off as the last letter closes (`pen::flick`). The row's meta (duration, day and time) stays hidden while he writes and inks in after the flick. His lean while writing is the `--write-lean` token in theme.css, worn through his pose (`engine::look`). Anything the user does (a grab, typing, the row leaving the screen) pauses the ink, and he returns to finish it from home (`pen::beat`); a row is never written twice, and a reload shows titles whole.
- **Grab to listen:** there is no Mic button. Pointer-down on Rambo in his nest (or Space/Enter on his focusable circle) starts speech recognition. Releasing without dragging captures the transcript. Keyboard: Space/Enter toggles listening. A dazed Rambo never listens.
- **Gestures:** `crates/web-core/src/gesture.rs` is the page's one gesture recogniser (VISION-AUDIT item 3). `Recognizer::advance(Sample)` turns pointer samples (down on a `Target`, move, up, cancel, and a `Tick` each frame while a finger rests, from callers that need holds on time: Rambo's watch of a dazed page sends them), each naming its finger: only the first finger down counts until it lifts, into `Gesture::{Down, Tap, Hold, Drag, Fling, Drop, Cancel}`, with one slop (`GESTURE_SLOP_PX`) and one hold time (`GESTURE_HOLD_MS`); a hold is decided by time before travel, as `row::advance` decides it. `release_velocity` reads the finger's speed over its last `GESTURE_RELEASE_SAMPLE_MS` of moves, and is zero once the finger has rested `GESTURE_RELEASE_REST_MS` before letting go; it is the one throw detector; `flung` is a release at `GESTURE_FLING_MIN_SPEED` or more. The brain is asked before any target: `mascot::wants` gives a dazed Rambo every hold and drag, whatever is under the finger. Rambo's drop uses it today; rows, slips, the turn and the pin still run their own (planned: each moves over in its own PR).
- **Moving:** every move from one spot to another (to the caret, to a row, home, the run home) is brisk hops, never a glide: each hop crouches, arcs and lands with a small settle (`crates/web-core/src/hop.rs`: one hop per `HOP_SPAN_PX` of the way, at most `HOP_MAX`, each `HOP_MS`; a short hop arcs `HOP_RISE` of its length, and past `HOP_HEIGHT_PX` the height keeps growing, by `HOP_LONG_RISE` of each extra px — no flat stretch, a longer hop is always higher — up to `HOP_HEIGHT_MAX_PX`, so a long trip in two hops never reads as a glide). He never moves instantly: every run of hops starts with a notice beat (`NOTICE_BEAT_MS`) where he stands, timed by the engine's tick (`engine::on_tick`), his face changing and his body perking up (`mascot::notice`, `mascot::notice_squash`), so the beat and the run together fit in `FLY_MS`. Landing at the end of a trip, and being moved by your finger, have no beat. Whoever moves him sets his spot (`pen_pos`); the engine's world (`engine/world.rs`) hops him there, or keeps him exactly there while he follows something: his nib on a row, where he fell (`mascot::hops_there`). Moves under `HOP_MIN_PX` are a step. Flying home from the caret, he sets off for the nest at once (`capture.rs`). Flying to a new row, his spot is the start of its title on every engine frame (`write::follow_row`): the row is still moving into place, so he lands where it is, not where it was when he set off. A hop's `Frame` keeps the ground point (`pos`) apart from the height above it (`lift`); he is drawn at `pos` raised by `lift`.
- **Drag:** pointer-based drag (not HTML5 drag). Dropped gently on empty space, he hops home; he is never dazed by a drop. Let go mid-flick (a `Fling` from `crates/web-core/src/gesture.rs`), he is thrown (Thrown state): he keeps the flick's speed, slides across the screen slowed by the paper's friction (`FLING_FRICTION`) and curving a little toward the bottom (`FLING_GRAVITY`), and bounces off its edges with `FLING_BOUNCE` of his speed (`web/src/engine/fling.rs`). Only a hit on an edge at `FLING_SPLAT_SPEED` or more is a smack: ink splats off the edge (`fling::splat_strokes`), he splats, is dazed where he hit for `DAZED_MS`, then skitters home in small hops, each hop that lands on the page leaving an ink dot that fades like any ink. A throw that just stops sends him home undazed, and grabbing him mid-slide holds him again. The paper drags him too (Interview 1 item 2): `rambo.rs:on_paper_scrolled` hears every scroller (capture phase) and gives how far its paper moved on screen since its last scroll event and how long that took; if he lies on that paper (`mascot::lies_on_page`: dazed or asking, and the first thing under him is inside the scroller), `engine::swept` decides by friction: up to `SWEEP_GRIP_PX_MS` he rides along (his spot moves with the paper), faster he is flung at the paper's speed (`Event::Swept`, a `Thrown` like a flick's, so the walls, the nest's catch and a smack all apply). With reduced motion he only rides along. A throw that comes within `NEST_CATCH_PX` of the nest's corner (right of and below the nest's centre less that) is caught first, before any wall: the nest takes the hit (`Event::DropNest`, Thrown to Home), with no daze, no ink and no notice beat. The nest is an entity of the world too (`web/src/engine/nest.rs:Nest`): two `Spring1`s (`engine/body.rs`), a sag in px and a rock in degrees (`NEST_OMEGA`, `NEST_DAMPING`, never past `NEST_SAG_MAX_PX` or `NEST_ROCK_MAX_DEG`), kicked when he lands in it (by how fast he is still falling: a hop's touchdown speed, `Hop::impact`, times `NEST_IMPACT_SAG`, never less than `NEST_LAND_KICK`; tipped toward the side he lands on), when it catches a throw (the same give for the throw's speed, tipped the way he went) and when he leaves it (springing up and rocking the other way), and stepped with the world. At home he sits on the sag and turns with the rock; `rambo.rs` writes `--nest-sag` and `--nest-rock` on both of its layers (the nest button and `.nest-rim`), whose transform in `theme.css` moves the drawing, never the button. With reduced motion the nest never moves, and the loop sleeps only once it has settled. His nib runs dry too (Interview 3, idea 2): the world counts the ms he spends writing (rows and the caret); landing home with `mascot::nib_dry` true (`DRY_NIB_AFTER_MS`), he starts tapping, staying `Home`: after `DRY_NIB_PAUSE_MS` each of `DRY_NIB_TAPS` taps, `DRY_NIB_TAP_MS` apart, knocks the nest (`Nest::tap`), so he bobs on its spring, and is heard (`World::take_knocks`, played by `rambo.rs` as `sound::Cue::Knock`), while he tips `DRY_NIB_LEAN_DEG` toward the rim, eased in and out. Any change of state stops it, and the count starts again from dry. With the Everything + music preset he dances there (`web/src/engine/dance.rs:Dance`): `music.rs` runs the music through a Web Audio analyser, `music::level` is its loudness (the RMS of its samples, divided by `MUSIC_VOLUME` so it is the recording's, not the volume's) while it plays, and every frame `rambo.rs` hands it to the world (`World::hear`). The dance follows that loudness fast (`DANCE_FAST_MS`) and slow (`DANCE_SLOW_MS`); a beat is the fast one rising past `DANCE_ONSET_RATIO` of the slow one, again only after it has fallen back and at least `DANCE_MIN_GAP_MS` after the last, so a held note moves him once. A beat pushes the swing he is already in (the side his sway is moving to, the other side only from still; a damped spring, never past `DANCE_LEAN_MAX_DEG`, added to his pose's tilt) and bounces the nest (`Nest::bounce`), and the spring is loose (`DANCE_OMEGA`, `DANCE_DAMPING`) so the swing carries from note to note and a run of notes swells it; with no music to hear the sway is critically damped and stops within a few seconds. He dances only when `World::can_dance`: settled at home, not tapping his nib, and not with reduced motion. Only then does `rambo.rs` read the analyser; otherwise the world hears nothing and his loop may sleep. While he dances the loop runs at the full frame rate for as long as the music plays: that is the Everything + music preset's cost, not a leak. While the preset is on, a timer every `DANCE_LISTEN_MS` wakes the loop if music plays and he could dance. The splat and the dots go through the one ink path: the world hands them out (`World::take_ink`) and `rambo.rs` lays them as strokes. With reduced motion he is never thrown. Held, he rides just above the finger or cursor so it never covers him (`mascot::held_at`, `HELD_GAP_TOUCH_PX`, `HELD_GAP_POINTER_PX`); dragged while dazed his nib stays at the finger. Dragging him while dazed writes an ink trail (Inking state, `web/src/inklayer.rs`). Dazed, a hold anywhere else on the page draws him to the finger, which then writes as if it had picked him up (`mascot::drawn_over`): only while he is dazed, `rambo.rs` watches the page's first finger with the recogniser, ticked by the engine so a still finger holds on time; whatever had the finger's Down gets a `pointercancel` and lets go, a row or a planner slip does not lift while he is dazed, and while he writes a touch does not pan the page. A tap or a slide before the hold stays the page's. Ink begun over the list page while it lies open lies on that page (`ink::Surface::List`): it is held in the page's own coordinates, counting the rows the page has scrolled inside itself. It is painted on a screen-tall sheet that sticks inside a layer as tall as all the rows, so it scrolls and turns with the paper; ink begun anywhere else lies on the floor, a fixed canvas above the page and below him (`Surface::Floor`). The ink is a smooth curve through the pointer's samples (`ink::smooth`), laid as close dabs whose width follows the hand's speed (`ink::width_at`), each stroke filled as one so it never beads. Ink lies wet, dries a shade lighter and fades after `INK_HOLD_MS`, quick thin strokes first (`ink::age_alpha`); a faded stroke is dropped. After 3 s untouched (`DAZED_MS`) he hops home (Skittering state); dropped on his nest he goes home at once and the brain gets `PutBack` (a small cheer lift). Going home does not wipe the ink: it fades on its own. Dropped on a task row while listening, he asks what to do with it. Dropped on an empty planner hour (`week::hour_at`, the one hour hit-test, also the bubble's Move; free when `week::free_at` finds no placement under way then), he is `Noting`: he stays there and listens, the capture box takes the focus, and what is sent is made as a task at that hour's instant, as worded (`capture::NotedHour`, `capture::noted`, `api::create_task`), never through the capture pipeline, whose judge may decide words that ask for nothing are no task; sending sends him home, and a tap off the capture box or Esc is never mind (`mascot::asking_dismissed_by`), emptying the box. A page leaf closing over him where he stands on the page (`Event::Covered`) leaves him `Covered` under it for `COVERED_MS`: the world gives the leaf a push from underneath (`World::leaf_push`, `COVERED_PUSH_ACCEL`, never past `COVERED_LIFT_REACH`) and he then pushes his way out, undazed (only a hard throw dazes him), and hops home (`Uncovered` makes him `Home`, not `Skittering`, so he leaves no dots). The world decides it from the leaf the page measures (`engine::Leaf`); the page turner shows the leaf and steps its spring with the push through `engine::link`, a thread-local seam that goes when the turn joins the `World` (planned, ENGINE-PLAN section 2).
- **Dropped on a task or event** (list row or week block): he stays there and asks, a bit annoyed, what you want, in a small speech bubble ("What about *Lab report*?") with three paper-scrap buttons, Edit, Move and Done (`mascot::Choice`); there is no Delete (deleting is the scribble-out). A tap anywhere off the bubble, him included, or Esc, is never mind (`mascot::asking_dismissed_by`, read by `gesture.rs` from listeners that live only while he asks); picking a choice or never mind sends him home. The bubble never fades: it grows out of him, and once he has his answer it folds back into him over `ASK_FOLD_MS` before it is taken away; near the top of the screen it opens below him. Move turns the page to the planner, where the task waits (`move_request`, `placing::PlacingNote`): a tap on an hour starts it there, snapped to the quarter hour, or a tap on his offer of the next free time (`placing::next_free`: the earliest quarter hour from now, inside working hours, where the task's length overlaps no other placement); Leave it, Esc or turning back to the list leaves it where it was. Both write through `week::place_at`, as a dropped slip does. Dropped on a slip in the planner (`week::slip_at`, which, like every planner hit-test, looks past him to what lies beneath) he asks about its time instead (`mascot::Asked::Slip`, `mascot::offered`): Move (the same wait for an hour), Longer and Shorter, a quarter hour each way (`placing::stretched`: never past the longest a task may be, never under a quarter hour), saved as the task's length through `week::stretch`. The slip stretches at once and, redrawn from the saved plan, eases on from the height it had reached. Take off sets the task's `off_planner` (`week::take_off`): the slip slides sideways out of its day over `SLIP_OFF_MS`, and the week is drawn from the saved plan once both are done (refused, it slides back); `week::place_at` sends `off_planner: false`, so placing a task again puts it back. One day of a repeating series takes its length from the series and is planned with it, so it is only offered Move. The first choice takes the keyboard focus, and Esc gives it back to him.
- **Talking:** hold on him in his nest to talk; holding Space (focus outside a text field) talks too. While he is dazed he does not listen until he is home again.
- **Testable:** his behaviour is a pure state machine (`Home`, `Dragging`, `Thrown`, `Dazed`, `Inking`, `Splatting`, `Covered`, `Skittering`, `Asking(task)`) in `crates/web-core/src/mascot.rs`, unit-tested without the DOM. The pen's timing and nib math live in `web/src/pen.rs` (8 tests). The `PenFlight` enum (`Parked`, `WritingCaret`, `FlyingToRow`, `WritingRow`, `FlyingHome`) tracks the pen's writing animation independently.
- **`prefers-reduced-motion`:** no pen movement; rows appear normally, Rambo reappears in his circle.
- **Lobotomized mode:** listens but no personality animation.
- **Touch:** `touch-action: none` on `.mascot-home` only; page swipe is unaffected.

## 8A. Target: one core, thin views (planned)

The web front end is about 31,000 lines in 50 files (`web/src`, plus about 3,100 lines of `theme.css`). It works, but state, motion and timing are spread across the views: 22 write sites with four error policies, Rambo's state kept twice, `thread_local` channels between the list and the strips (and, until #413, the tear), two springs, about 20 timers off the engine clock, and CSS and Rust lengths kept in step by hand. The sources are `hive/research/motion-inventory.md` (Mario), `state-map.md` (Claudio), `module-map.md` (Dario), `css-hygiene.md` and `codebase-brief.md`. They add four more findings: `App` hand-wires about 20 signals (Rambo takes 22 props); DOM logic hides behind 158 wasm `cfg` gates and about 40 host stubs, untested; three mega-components (`Rambo` about 1,280 lines, `TaskList` about 950, `WeekGrid` about 640); and module cycles (`scrap` with `rip_ui` and `tasks`, and the engine depending on `flip`). This section is the shape to converge on. The one-body physics design (§8.0.1, added by #405) is one part of it.

### 8A.1 Shape

```
            commands ─────────────►┌──────────────── core (no DOM, native tests) ────────────────┐
  views                            │ store   Snapshot + generation; Command → send, apply, reconcile│
  (Leptos,                         │ clock   the one engine clock: frames and `after(ms, Event)`    │
   thin)   ◄──── signals (read) ───│ events  one queue: what happened (row left, strip pinned, …)   │
                                   │ world   Rambo (brain + body), the bodies (strips, ghost, leaf), │
                                   │         the nest, ink; stepped on the clock, reads the store    │
                                   │ body    Pull, Spring1, Body, Forces; Motion (reduced or full)   │
                                   └────────────────────────────────────────────────────────────────┘
```

- **store** (`crates/web-core/src/store.rs`, with the page's side in `web/src/store.rs`; in code for `Reorder` and, from the edit sheet and the row delete, `CreateTask`, `DeleteTask`, `CreateSeries`, `PatchSeries`, `DeleteSeries`, `MarkOccurrence`, `ClearOccurrence`, all pessimistic: nothing shows until the server answers; `Capture` is in the store but the capture box still calls `api::capture_text` (planned); the other commands planned): the one owner of the `Snapshot`. Views never call `api::*` or `set_snapshot`. They send a `Command` (`Capture`, `Patch`, `Tick`, `Delete`, `Restore`, `Reorder`, `Place`, `Stretch`, `TakeOff`, the series ops, `DayTimes`). The store sends it (`Store::send` answers the caller with what the server said besides the list, `Reply::Created(ids)` for a capture, or `None` if it was refused), applies it optimistically where the result is certain (tick, reorder, take off), and on the reply keeps a snapshot only if its `generated` is newer than the one it holds. What it shows is always the newest confirmed list with every command still unanswered laid on top again, in the order sent, so a newer list never flickers a change on its way out and back; laying a command on a list that already holds it changes nothing. Every refusal follows one policy: show the last confirmed list (with the other unanswered commands still on top, so a refusal takes back only its own change), reload, then an event the world turns into Rambo's bubble (his words come from him, never loose text in a corner). Until the events queue (step 3) exists, the note goes to the page's error line. A list written by a call site not yet moved to the store is taken only if it is newer than the last confirmed one; an older one is replaced on screen by the store's list (`Store::offer`). Ties in `generated` go to the later request. That fixes problems 1, 2 and 3 of the state map.
- **clock** (today's `engine/run.rs` and `engine/sched.rs`): frames, and timers as `after(ms, Event)`. Nothing else in `web/` may call `Timeout`, `TimeoutFuture` or `set_timeout`, and the existing test that forbids a private loop grows to cover them.
- **events** (`crates/web-core/src/events.rs`, shared by the page through Leptos context in `web/src/events.rs`; in code for `RowLeft`, the rest planned): one queue of facts (`Event`) that views push and whoever a fact is for takes with a claim (`Events::take`, `take_all`; `row_left(id)` is the first). A fact carries the geometry it needs (`Spot`), so the core never measures. Once the world owns Rambo (step 4) it drains the queue each step. `RowLeft` replaced `scrap::DEPARTURES`; `scrap::RETURNS`, the request signals (`edit_request`, `move_request`, `open_sheet`) and the store's refusals (as an event Rambo speaks, carrying the server's wording) are planned. A test (`events::tests::no_new_thread_local_channels`) lists the `thread_local`s left, so a new one fails and each migration deletes its line.
- **world** (today's `engine/world.rs`): the only owner of Rambo's state. `mascot::transition` stays the one rule. `mascot_state` becomes a read-only mirror the world writes once per step, so the two-way sync at `rambo.rs:232` and the three bypass writes go away. Bodies for paper strips, the drag ghost and the page leaf live here too.
- **body** (the one-body design, §8.0.1, added by #405): all motion as forces on bodies and springs, stepped by one integrator, with one reduced-motion switch.

**Crates.** The core is a native crate, `web-core` (planned), holding the pure halves: `body`, `sched`, `world`, `mascot`'s state machine, `ink`, `gesture`'s recogniser, `pen`, the planner's model (`week`'s geometry and drag rules), the list's model (row order, hold, release) and the page leaf (`flip`'s geometry). `rambl-web` keeps only views and the browser seam (`engine/run.rs`, `api.rs`, audio). The `cfg(target_arch = "wasm32")` gates and host stubs go: what is in `web-core` is tested natively, and what is in `rambl-web` is a view by construction. The engine sits at the bottom; nothing in it imports a view or a page-turn style.

**Wiring.** The core is provided once through Leptos context (`provide_context(Core)`). Views take it with `use_context` instead of 20-odd props, so adding state never means threading a new prop through `App`.

The views are list (`tasks/`, `row.rs`), planner (`week.rs`, `placing.rs`, the bookmark), capture, strips (`scrap.rs` without its timers and physics), the page flip, Rambo's drawing (`rambo.rs`, `look.rs`), the ink layer, the gear, sheets and pickers, and account. Sound, voice and music are outputs: they play cues the world hands out, as knocks do today.

### 8A.2 Rules every feature follows

1. A view reads signals and sends commands or events. It owns no task data, no timer and no motion.
2. Every write goes through the store. No `spawn_local(api::…)` in a view.
3. Every timer is the engine clock's. Every movement is a body or a `Spring1` stepped by the world.
4. CSS may animate only the inside of a drawing (mood pictures, a scribble's strokes) and the two allowed fades, ink and the patch. Rambo's breathing and grumbling move to the world (a `Spring1` sway), so they pause when he is held and obey the one reduced-motion switch. Position, size and rotation come from the core and are never a CSS `transition` or keyframe. A test lists the allowed keyframes.
5. A length or duration shared by CSS and Rust (for example `--slip-off-ms` and `SLIP_OFF_MS`, or the 700 px phone breakpoint repeated in 7 `@media` blocks) lives in `tuning.rs` and is written to a CSS variable by the core. It is never typed twice.
6. No `thread_local` or `static` for state shared between parts. Use the store or the events queue.
7. Randomness is seeded: the existing hash (`row::hash_unit`, as the splat uses) keyed on a seed the world holds, which is fixed in tests. No new dependency is needed, and the world stays deterministic under test.
8. One reduced-motion switch (`body::Motion`), read by the world only.
9. A view component stays small: its logic lives in a model in `web-core` with tests, as `rambl-brain`/`brain.rs` and `mascot`'s pure state machine already do.
10. The core never measures. Views report geometry as events (row rects, the column pitch, the viewport, the keyboard inset), and the core reads only those. This keeps `web-core` testable natively and closes the bug class of #381 (measuring through a live transform) and #383 (the bar rises with the keyboard and Rambo stays behind).

### 8A.3 Cut list (the human: favour a small core)

| Cut or simplify | Code it removes (on main e83cf0c) | What replaces it |
|---|---|---|
| **Page rip**: tear, hole, queue, tape, band, the planner sliding under the tear, Rambo's `Mending` | about 2,150 lines (#413): `rip.rs` 987, `rip_ui.rs` 695, the hole hand-off and the row hold and release in `scrap.rs` (about 80; its departure notes and row slide stay), tear CSS (about 150), 17 `RIP_*`/`TAPE_*`/`TEAR_*` tunables, Rambo's `Mending`, `ROW_OUT`/`HOLE_OPEN`, the `transitionend` workaround. #401 is superseded. | Nothing. A deleted row's scrap still flies to the pin; the rows below close up by the plain slide (`scrap::slide_rows`). On a phone the planner is reached by the page turn. This also breaks the `scrap`/`rip_ui`/`tasks` cycle. |
| **One page-turn style** (the gear's `Page turn` choice) | about 1,100 to 1,300 lines: the other style's pure and DOM halves, `turner.rs`, `turn_util.rs`, the `PageTurn` setting and the unused half of `engine/link` | One turn, rebuilt as a `Spring1` leaf in the world. **The human picks which**: the book flip (`flip*`, the default, simpler to explain) or the corner bend (`fold*`, lighter). Page turning is a reported bug (spec item 11), so one clean implementation is the goal. |
| **Hop scripts** | `hop.rs` 278, `Run` and `Drive::HopTo` in `world.rs` (about 200) | Impulses on Rambo's body (the one-body design, PR 3) |
| **Wet-ink smudge** | `smudge.rs` 133 and its CSS | Nothing (ink still dries and fades) |
| **Ribbon pull and stretch** | the pull, stretch and spring-back half of `ribbon.rs` (about 100) and its CSS transition | A static bookmark at today; a tap glides back (a `Spring1`) |
| **Sheet timers and transitions** (the edit sheet, the gear, the pickers' page, the lamp's sheet) | the three `set_timeout`s in `lamp.rs` and the sheets' CSS transitions | Not a cut of the feature: every "sheet laid on the page" is one `Spring1` to its y in the world, like the page leaf, so the theme still swaps physically under a sheet of the other paper |
| **Two capture texts** | `capture_text` and the copying Effect (state map 8) | One text signal in the store |
| **Working hours in two places** | `Settings.start_hour`/`end_hour` in localStorage, beside the server's constants | One server-side day setting the planner reads (server change, own step) |

Candidates for a human call, not cut here: the dance (`engine/dance.rs` 189, music analyser), the voice (`voice.rs`, `speech.rs`) and the custom pickers (`pickers.rs` 686, versus native inputs styled as paper).

The cuts above remove about 4,000 lines, about a seventh of the front end, before any refactor. The server tidy-up from the module map (the `http.rs` and `store.rs` tests and the test-only `MemoryStore` out of the prod files) is done (#408) and is not part of this core.

### 8A.4 Migration (each step is one or a few small PRs; main stays green; behaviour unchanged unless the step is a cut)

0. **Free now:** delete the dead code the module map lists (about 120 lines).
1. **Cuts first** (each its own PR): the whole page rip (#401 closed; a simple tear may be rebuilt on the core later, only if the human asks), then the smudge and the ribbon pull. Once the human picks, the other page-turn style is only deleted here; the kept leaf is rebuilt in step 6. Less code to move in every later step.
1b. **Layers:** create `web-core` and move the already-pure modules into it (`body`, `sched`, `mascot`'s machine, `ink`, `pen`, `gesture`'s recogniser, `flip` or `fold`). Take the engine off the page-turn module and provide the core through context. These are pure moves with no behaviour change.
2. **Store and commands.** Add `core::store` with the generation check and the one error policy (in progress: the store and `Reorder` are in code). Then move the write sites over one file at a time (`edits.rs`, `tasks/mod.rs`, `week.rs`, `capture.rs`, `scrap.rs`, `settings.rs`). This fixes the out-of-order and silent-reorder bugs.
3. **Events queue.** Replace `DEPARTURES`/`RETURNS` (and anything the rip left) with core events (in progress: the queue and `RowLeft` are in code). Each further channel is one small PR: add its `Event` variant and a claim in the core with a test, push it where the view noted it, take it where the view read it, and delete the old channel and its line in the `thread_local` list.
4. **Rambo, one owner, and the three mega-components split.** The world owns his state; `mascot_state` is its mirror; the bypass writes go through `transition`. Then `Rambo`, `TaskList` and `WeekGrid` each become a `web-core` model with tests plus a thin view, one PR per component.
5. **Clock only.** Move every `Timeout`/`TimeoutFuture`/`set_timeout` (capture, write pacing, row hold, scrap lay and fly, `take_off`, scribble) onto `after(ms, Event)`, and extend the test to forbid them.
6. **Physics** (the one-body design, §8.0.1, which #405 adds): PR 1 is #405 (`Spring1`; it fits this design unchanged). Then Rambo's body; the hop home stays hop by hop (a wait, then one impulse per landing event, a dot where each lands, never one flight); `Motion`; then the kept page leaf rebuilt as a `Spring1` in the world, which is where the page-turn bug (spec item 11) is fixed; then the sheets as `Spring1`s.
7. **Paper as bodies.** Pinned strips, a lifted row's release and the drag ghost become world bodies, keyed by task id. Delete their inline transitions.
8. **CSS motion audit.** Add the allowed-keyframes test (rule 4) and move every shared length into `tuning.rs` (rule 5).
9. **After the freeze**, on this core: Rambo's seeded variation in hops, wandering and doodling on empty space, and turning sad when the page is full.

Each step ends with the full gate and a note in this section moving it from planned to done.

## 9. Testing (the definition of done for every change)

- `dates`: a table of phrases at fixed instants, including Sunday 21:00 New York (Monday in UTC), both DST change days, month and year rollover, "next Tuesday" on a Tuesday, and every weekday from every weekday.
- `scheduler`: invariants for random inputs: flexible tasks never overlap `At` tasks or each other, `At` tasks exactly at their start, `DueBy` tasks end by their due time when there is room, the order breaks ties.
- `capture`: golden fixtures (rambles in, drafts out) with `FakeJudge`; live Jev tests are `#[ignore]` and run by hand.
- `import`: a committed synthetic `.ics` fixture shaped like a real export (TZID times, LOCATION, 16 events in one week): every event becomes an editable `At` task at the right local time, re-import updates instead of duplicating, a moved imported task keeps its new time. burvy's real sample (`Z:\App Data\Downloads\calendar_this_week.ics`) is personal: use it only in a local `#[ignore]` test via the env var `RAMBL_SAMPLE_ICS`, never commit it.
- `server`: HTTP tests (`tower::ServiceExt`) with `MemoryStore`, `FixedClock` and `FakeJudge`. A shared check runs on every returned snapshot: each open task with a placement shows that placement, and no placement exists for a done or deleted task.
- `web`: `wasm-bindgen-test` that the list and week render the same times from a fixture snapshot.
- CI: `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`, `trunk build`.

## 10. Plan of work

| Phase | Work | Depends on |
|---|---|---|
| 1 | `core` types + `dates` crate with its full test table | this doc |
| 1 | `judge` trait + `FakeJudge`; `capture` pipeline on top | core signatures |
| 1 | `web` scaffold rendering a fixture `Snapshot` | core types |
| 2 | `server` v1: Store, App, handlers, HTTP tests | phase 1 crates |
| 3 | `web` wired to API v1, mic, reorder; deploy scripts build `web/dist` | phase 2 |
| 4 | delete `static/`, old routes and dead code; final docs pass (done: v1 server, `judge::v1`, `core::v1` and the legacy `scheduler::schedule` are gone; the v2 app plans with `scheduler::plan`) | phase 3 |

Other docs: `README.md` (run and deploy), `docs/jev.md` (Jev API notes).

## Later

- Paid tier with a larger Jev allowance (users already have a `plan` field).
- Ads to cover Jev.
- Rambl as an installable app.
- Give the mascot and page the rest of the parchment treatment: animations and sounds to match the texture (`docs/THEME.md`), and the pose and icon graphics from `web/assets/theme/README.md`.
