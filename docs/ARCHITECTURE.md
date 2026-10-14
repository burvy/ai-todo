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
                  external_uid: Option<String> }   // iCalendar UID for imported events

pub struct TaskDraft { title, duration_min, when, label, priority, source }      // no id
pub struct Placement { task: TaskId, start: DateTime<Utc>, end: DateTime<Utc> }
pub struct Plan { placements: Vec<Placement>, unplaced: Vec<TaskId> }
pub struct Snapshot { tasks: Vec<Task>, plan: Plan, tz: String, generated: DateTime<Utc> }
pub struct ApiError { code: ErrorCode, message: String }
```

`When::At` always places at that instant; `Anytime` places in the first free slot. One task has at most one placement in v2 (no splitting).

**A `DueBy` task is a day-only task** ("call mom Friday"): it is placed **on its deadline's day**, in the free slot whose start is closest to the user's *usual time*, ending by the deadline and not before now. The usual time is the average local time of day of the user's own timed (`At`) tasks, finished ones included (series occurrences and calendar imports do not count: the user did not pick those times one by one); with none, it is `DaySettings.placement`, noon by default. If that day has no room, the task takes the earliest free slot before the deadline, and only if there is none it is unplaced. Overdue tasks (deadline now or past) are placed from now, first. `DaySettings` is the one per-user pair of times behind this: `deadline` (23:59 by default: where a day-only capture's deadline lands on its day) and `placement` (12:00 by default), both local `HH:MM`, stored per user (`v2_users.day_deadline`, `day_placement`; NULL is the default) and read and changed with `GET`/`PATCH /api/v1/settings`. A deadline of 00:00 is refused. Changing the deadline does not move existing tasks; it applies to what is captured or revised afterwards.

### 4.1 Task order and time semantics

New tasks receive the highest position in the list, so the list is newest first. The scheduler still fills time in list order; a deadline takes precedence over list position. `When::At` is an absolute instant, like a calendar event. `When::DueBy` stores the user's time zone at capture time as part of the due date, so the intended local deadline is preserved if the user later travels.

An unfinished task past its `DueBy` remains in the list marked **Overdue**. The scheduler places it in the next free slot, and Rambo teases the user about it.

## 5. Capture pipeline (`crates/capture`)

1. **Segment** (code): split text into clauses.
2. **Candidates** (code, `dates`): per clause, title spans, date phrases ("tomorrow", "Wednesday", "Oct 30"), time phrases ("at 3", "10am"), duration phrases, and relative phrases: "in 10 minutes", "in an hour", "in half an hour", "in 90 minutes" are `Value::After` (an instant, now plus that many minutes to the minute; the numbers may be spelled, up to sixty). It is unambiguous, so Jev is not asked: the draft is pinned `When::At` that instant whatever day or time Jev picks for the rest. "in 2 days", "in a week", "in a month" are ordinary days (calendar days, so a DST day is not 48 hours; a month is the same day next month, or the last day of a shorter one). Speech mishearings of "add a task" ("attittask", "add it task", "attit task", "at a task") are written as "add a task" before the lead-in is cut (`leadin::strip`). Each candidate already carries its resolved value from `LocalContext`. The title candidates include the clause without a lead-in such as "Add a task to" (`capture::leadin`, measured in `docs/experiments/title-leadin.md`); Jev chooses, and a judge with no opinion takes the first, which is the grammar's cut.
3. **Judge** (one Jev request per capture, all questions in parallel): per clause `is_task` (Noul), `title` (Choice of spans), `day` (Choice of date candidates + none), `time` (Choice of time candidates + none), `duration` (Choice), `label` (Choice), `priority` (Score). Jev gets the local time as text for context only.
4. **Resolve** (code): build `TaskDraft`. Day + time -> `When::At`; day only -> `DueBy(day at the user's deadline setting, 23:59 by default, local)`; neither -> `Anytime`.

Without a Jev key, `FakeJudge`'s deterministic rules pick the first candidate. Tests use it, so the pipeline is tested end to end without the network.

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
| `PATCH /api/v1/tasks/{id}` | any of `title, duration_min, when, done, label, priority`, plus `updated_at` (the version the page last saw) | `Snapshot`; `409 stale_update` if the task changed since `updated_at` |
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

- `Recurrence { weekdays: BTreeSet<Weekday>, until: Option<NaiveDate> }` — the rule. Empty `weekdays` never constructs; `until` is the last day an occurrence exists, inclusive. `chrono::Weekday` is `Eq` and `Hash` but not `Ord`, so this cannot be a `BTreeSet<Weekday>` as first sketched: the days are a sorted, duplicate-free `Vec<Weekday>` (Monday first), which is what `dates::RecurrenceMatch::weekdays` is too.
- `Series { id: SeriesId, user, title, duration_min: u16, rule: Recurrence, start_day: NaiveDate, time_of_day: Option<NaiveTime>, zone: String, external_uid: Option<String>, created, order }` — one per repeated thing. `time_of_day` comes from the capture or import; without it an occurrence floats (`Anytime`) and the planner gives it the first free slot it can find, like any flexible task (there is no per-week notion of "first"). `zone` is where the weekly time lives, like `due_tz` for a deadline. `order` is its list position, the same key as `Task::order`.
- `SeriesException { series: SeriesId, date: NaiveDate, kind: ExceptionKind }` with `ExceptionKind::{Done, Skipped, Moved { to: When, to_tz: Option<String> }}` — one per deviating occurrence, keyed by local date. A moved `DueBy` keeps the zone the move was made in, like `Task::due_tz`.
- `Occurrence { series: SeriesId, date: NaiveDate, when: When, status: OccurrenceStatus, due_tz: Option<String> }` — the pure expansion result, with `OccurrenceStatus::{Open, Done, Moved}`. Skipped days produce no occurrence at all.

**Expansion (pure, in `scheduler`).** `expand(series, window_start, window_end_exclusive, exceptions, ctx) -> Vec<Occurrence>`: for each day in the window whose weekday is in `rule.weekdays` and which is on/after `start_day` and on/before `until`, produce one `Occurrence`. Its `when` is `At(ctx.at(date, time))` in the series' zone (each occurrence is re-derived in the zone stored on the series, so DST shifts the UTC instant but never the local 07:00), or `Anytime` without a `time_of_day`. Exceptions reshape the output: a `skipped` date yields nothing; a `done` date yields the occurrence with `status: Done` (and no placement); a `moved` date yields the occurrence at its new `When` with `status: Moved` and the move's zone, its id still naming the original date so the exception row stays addressable. A zone that will not parse falls back to the request's zone rather than losing the series.

**Snapshot.** A snapshot expands every series for the window it was asked for and appends the occurrences as ephemeral `Task`s: skipped days never appear; done ones appear marked done; moved ones sit at their new time. Each row gets `id` = `"{series_id}:{date}"` (the original date for a moved one), `source` = a new `Source::Series`, `order` and `created` = the series'. The plan is built from these rows together with the stored tasks, so open occurrences occupy real slots and the week grid shows them with no new plan type. `Placement.task` already holds a `TaskId`, and an occurrence id is a valid `TaskId`. Occurrence ids never enter the `v2_tasks` table. A mutation answers with the default window (today and 7 days, the one the page draws); a client looking at another window re-fetches it.

**Store (migration 8).** Two tables, both user-scoped by `user_id` (the column name every v2 table shares):

- `v2_series`: `id TEXT` primary key, `user_id TEXT`, `title TEXT`, `duration_min INTEGER`, `weekdays TEXT` (e.g. `"124"` = Mon/Tue/Thu, `chrono::Weekday` numbers from Monday=1), `until TEXT` nullable (`YYYY-MM-DD`), `start_day TEXT`, `time_of_day TEXT` nullable (`HH:MM`), `zone TEXT`, `external_uid TEXT` nullable, `created TEXT`, `"order" INTEGER`. Index on `user_id`, unique on `(user_id, external_uid)`.
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

**ICS import.** In `ics_import.rs`, `parse_calendar` reads one export into three things: the one-shot events, the weekly series among them, and a note for every rule it could not follow. A `VEVENT` with `RRULE:FREQ=WEEKLY` becomes a series and is *not* imported as a task: `weekdays` from `BYDAY` (or DTSTART's own weekday when there is none), `until` = the `UNTIL` instant's date **in the event's TZID zone** (Outlook writes the local 23:59:59 as UTC, so `UNTIL=20261215T035959Z` is the 14th in New York), `COUNT` mapped to the date of its last occurrence (the earlier of the two ends wins when a rule names both), `start_day` = DTSTART's local date, `time_of_day` = DTSTART's local time (an all-day event floats instead), `zone` = the TZID, `external_uid` = the VEVENT UID so a re-import updates that series rather than adding a second one, keeping the id, list position and exceptions the user already has. `EXDATE` dates become skipped occurrences, so an excluded week stays excluded after a restart.

Any other `FREQ` (daily, monthly, yearly), an `INTERVAL` other than 1, an ordinal `BYDAY` (`2FR`), and any part we do not read (`BYSETPOS`, `BYMONTHDAY`, `BYWEEKNO`, …) import as today — one `At` task at the first occurrence — and the import answer lists the event and the reason under `unmapped`, so the client can say the rule was not kept rather than the day silently disappearing. `ImportResult` gains `series: Vec<SeriesId>` and `unmapped: Vec<String>`, both skipped when empty.

**Web UI.** *(planned)* — a series shows as a list row with the title and a recurrence chip (`Mon · Thu · until Dec 15`). Its occurrences look like ordinary rows in the list and the week grid, with the date in place of a due date. The task dialog gains days-of-week checkboxes and an until field for a series, and occurrence rows gain **Done**, **Not today** (skip) and **Move**. No new request paths beyond the endpoints above; `api.rs` still funnels everything into the one snapshot signal.

**Out of scope (stated so it stays out).** Monthly, yearly and daily rules; `BYSETPOS`/`BYMONTHDAY`; editing one occurrence's title or duration independently of the series (an occurrence inherits the series until it is moved); reordering occurrences relative to tasks beyond what `plan` already does; timezone changes for a series after creation (the series keeps its zone); series in the paid-tier or ad paths.

**Test plan.**

- `dates`: fixed-clock table rows for the scanner — `gym every Monday and Thursday`, `every weekday until December` (until = Nov 30 of the current year), `weekly on Tue and Fri at 6pm`, `standup every day at 9`, and the negatives (`a thing next Monday` is not a series). DST: a 07:00 America/New_York series expands at 11:00Z and 12:00Z across the March change.
- `scheduler`: `expand` invariants — no occurrence on a day outside the weekday set, none after `until`, an exception of every kind honoured, a moved occurrence at its new instant.
- `capture`: golden fixtures with `FakeJudge` — a recurrence clause produces a series draft, a one-shot clause stays a task, a mixed clause uses `start_day`.
- `server` (HTTP, `MemoryStore` + `SqliteStore` shared suite, `FixedClock`): create a series → the snapshot shows the expected occurrence rows; done/skip/move an occurrence → the snapshot reflects it; restart (fresh `App`) → done occurrences still done; move frees the original slot in the plan; weekly ICS import → a series, not one task; a non-weekly `RRULE` → one `At` task and `unmapped`; a made-up occurrence id (`other-series:2026-01-01`, a foreign series id, a date outside the rule) is refused.
- `web`: a fixture snapshot with a series renders the chip and the occurrence rows in the list and week grid.

## 8. Web (`web/`, Leptos CSR)

Plain and fast: a text box with a mic button (Web Speech API via `web-sys`), the task list, and a 7-day week grid. Light and dark follow the OS. The look is the parchment theme: tokens and rules in `docs/THEME.md`, all CSS in `web/theme.css` (linked from `web/index.html` with `<link data-trunk rel="css">` and copied into `web/dist`). The one texture is the paper itself (`web/assets/theme/paper.svg`, dark variant `paper-dark.svg`) on the page background; if a graphic is missing the flat paper colours carry the page. No animations or sounds, except the mascot below. One `api.rs` module makes every request and stores the returned `Snapshot` in one signal; the list and the week are derived from that signal only. Reorder by drag sends `PUT /tasks/order`. Built by `trunk build --release` into `web/dist`, which the server serves.

### 8.1 Rambo, the mascot

The pen character (`web/assets/rambo/`) is named **Rambo**. He rests in his nest, the dish at the end of the text box.

- **Nest and pen are separate:** the nest is drawn in two layers (`web/assets/nest/`): its back on the nest button, which sits in the page, behind Rambo; its front rim as a fixed layer of its own (`.nest-rim`) in front of him, placed on the nest's centre as he measures it. Their order is data in `:root` (`--layer-capture` < `--layer-rambo` < `--layer-nest-rim`); at home he rests `--nest-sink` below the centre, his tip inside the rim; Rambo is a `position: fixed` layer placed only by `--pen-x`/`--pen-y`, so carrying him away never carries the dish. At home he follows the nest: he measures it again on any scroll (window or panes), resize, page size change and once the fonts load (`web/src/rambo.rs:on_page_moved`), and on the way home he aims at where the nest is now. No CSS rule may give the button a transform without `--pen-x`/`--pen-y` (a test in `mascot.rs` checks theme.css).
- **Only to rows on screen:** the pen flies to write a new row only when that row shows on screen (`pen::on_screen`); a row on the other pane of a phone, or scrolled away, is shown without him.

- **Writing pen:** while typing, Rambo's pen follows the caret in the capture box. Two seconds after typing stops, the pen flies home. When new tasks are created, the pen visits each new row and writes its title under the nib, glyph by glyph, a glyph being a grapheme cluster so an accent or an emoji is one (`pen::glyphs_of`, `pen::schedule`, paced by `WRITE_PER_GLYPH_MS` between `WRITE_MIN_MS` and `WRITE_MAX_MS`), one span a glyph whose ink fades in over `INK_FADE_MS` as the nib passes its middle (`pen::stroke_at`, `pen::inked_count`), so wrapped lines need nothing of their own. His hand presses through each glyph, lifts over each word gap and wobbles a little, moving smoothly from one glyph into the next (`pen::hand`); he touches down when he lands (`pen::touch_down`) and flicks off as the last letter closes (`pen::flick`). The row's meta (duration, day and time) stays hidden while he writes and inks in after the flick. His lean while writing is the `--write-lean` token in theme.css. Anything the user does (a grab, typing, the row leaving the screen) pauses the ink, and he returns to finish it from home (`pen::beat`); a row is never written twice, and a reload shows titles whole.
- **Grab to listen:** there is no Mic button. Pointer-down on Rambo in his nest (or Space/Enter on his focusable circle) starts speech recognition. Releasing without dragging captures the transcript. Keyboard: Space/Enter toggles listening. A dazed Rambo never listens.
- **Moving:** every move from one spot to another (to the caret, to a row, home, the run home) is brisk hops, never a glide: each hop crouches, arcs and lands with a small settle (`web/src/hop.rs`: one hop per `HOP_SPAN_PX` of the way, at most `HOP_MAX`, each `HOP_MS`). He never moves instantly: every run of hops starts with a notice beat (`NOTICE_BEAT_MS`) where he stands, his face changing and his body perking up (`mascot::notice`, `mascot::notice_squash`), so the beat and the run together fit in `FLY_MS`. Landing at the end of a trip, and being moved by your finger, have no beat. Whoever moves him sets his spot (`pen_pos`); `rambo.rs` draws him hopping there, or simply there while he follows something exactly: your finger, his nib on a row, where he fell (`mascot::hops_there`). Moves under `HOP_MIN_PX` are a step. Flying home from the caret, he sets off for the nest at once (`capture.rs`). A hop's `Frame` keeps the ground point (`pos`) apart from the height above it (`lift`); he is drawn at `pos` raised by `lift`.
- **Drag:** pointer-based drag (not HTML5 drag). Dropped on empty space, Rambo is dazed where he lands (Dazed state; a fast drop splats first). Held, he rides just above the finger or cursor so it never covers him (`mascot::held_at`, `HELD_GAP_TOUCH_PX`, `HELD_GAP_POINTER_PX`); dragged while dazed his nib stays at the finger. Dragging him while dazed writes an ink trail on a canvas layer above the page (Inking state, `web/src/inklayer.rs`): a smooth curve through the pointer's samples (`ink::smooth`), laid as close dabs whose width follows the hand's speed (`ink::width_at`), each stroke filled as one so it never beads. Ink lies wet, dries a shade lighter and fades after `INK_HOLD_MS`, quick thin strokes first (`ink::age_alpha`); a faded stroke is dropped. After 8s untouched he hops home (Skittering state); dropped on his nest he goes home at once and the brain gets `PutBack` (a small cheer lift). Going home does not wipe the ink: it fades on its own. Dropped on a task row while listening, he asks what to do with it.
- **Dropped on a task or event** (list row or week block): he stays there and asks, a bit annoyed, what you want, in a small speech bubble ("Okay, what about *Lab report*?") with the task's actions: Edit, Move, Done, Delete, and Never mind. Picking one, or Never mind, sends him home.
- **Talking:** hold on him in his nest to talk; holding Space (focus outside a text field) talks too. While he is dazed he does not listen until he is home again.
- **PenRig overlay:** the travelling pen is a fixed-position `<div>` (`web/src/mascot.rs:PenRig`), moved via CSS custom properties `--pen-x` and `--pen-y`. The `.mascot-home` circle's image hides when the pen is away.
- **Testable:** his behaviour is a pure state machine (`Home`, `Dragging`, `Dazed`, `Inking`, `Splatting`, `Skittering`, `Asking(task)`) in `web/src/mascot.rs`, unit-tested without the DOM. The pen's timing and nib math live in `web/src/pen.rs` (8 tests). The `PenFlight` enum (`Parked`, `WritingCaret`, `FlyingToRow`, `WritingRow`, `FlyingHome`) tracks the pen's writing animation independently.
- **`prefers-reduced-motion`:** no pen movement; rows appear normally, Rambo reappears in his circle.
- **Lobotomized mode:** listens but no personality animation.
- **Touch:** `touch-action: none` on `.mascot-home` only; page swipe is unaffected.

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
