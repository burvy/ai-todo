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

Removed in v2: `static/` (after `web/` reaches parity), sounds, paper textures and every animation except the mascot's (section 8.1), `/api/message`, `/api/schedule`, spoken replies, `demo/` (its rambles move to `crates/capture/tests/fixtures/`), root `src/`, `Makefile` (one `run-local.ps1` + `run-local.sh` stay).

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

`When::At` always places at that instant; `DueBy` places before it; `Anytime` places in the first free slot. One task has at most one placement in v2 (no splitting).

### 4.1 Task order and time semantics

New tasks receive the highest position in the list, so the list is newest first. The scheduler still fills time in list order; a deadline takes precedence over list position. `When::At` is an absolute instant, like a calendar event. `When::DueBy` stores the user's time zone at capture time as part of the due date, so the intended local deadline is preserved if the user later travels.

An unfinished task past its `DueBy` remains in the list marked **Overdue**. The scheduler places it in the next free slot, and Rambl teases the user about it.

## 5. Capture pipeline (`crates/capture`)

1. **Segment** (code): split text into clauses.
2. **Candidates** (code, `dates`): per clause, title spans, date phrases ("tomorrow", "Wednesday", "Oct 30"), time phrases ("at 3", "10am"), duration phrases. Each candidate already carries its resolved value from `LocalContext`.
3. **Judge** (one Jev request per capture, all questions in parallel): per clause `is_task` (Noul), `title` (Choice of spans), `day` (Choice of date candidates + none), `time` (Choice of time candidates + none), `duration` (Choice), `label` (Choice), `priority` (Score). Jev gets the local time as text for context only.
4. **Resolve** (code): build `TaskDraft`. Day + time -> `When::At`; day only -> `DueBy(day 23:59:59 local)`; neither -> `Anytime`.

Without a Jev key, `FakeJudge`'s deterministic rules pick the first candidate. Tests use it, so the pipeline is tested end to end without the network.

## 6. HTTP API v1

All JSON. Every request carries the header `X-Rambl-TZ: America/New_York` (from `Intl.DateTimeFormat().resolvedOptions().timeZone`). Every mutating call returns the full `Snapshot`. Errors are `{ "error": { "code", "message" } }` with a fitting status.

**Accounts.** Rambl is multi-user. Every request carries either `Authorization: Bearer <session token>` for signed-in users, or `X-Rambl-Anon: <anon_id>` for anonymous users. The token or anon_id names the user; **every task read and write is scoped to that user, in the store, not in the handler** — `Store` methods take the user id, so a handler cannot forget it. In local mode (`GOOGLE_CLIENT_ID` unset), the server uses the fixed local user for all requests regardless of headers. Anonymous users have a daily Jev cap of `RAMBL_JEV_DAILY_CAP_ANON` (default 30); when exceeded, capture falls back to `FakeJudge` with `jev_fallback: true`.

Anonymous users do not sign in. Their account and tasks are deleted after 30 days without a visit; Rambl nudges them to sign in before then to keep their tasks.

| Method and path | Body | Returns |
|---|---|---|
| `POST /api/v1/auth/google` | `{credential}` (a Google ID token) | `{token, user}`. The token is checked against Google's `tokeninfo`; the audience must equal `GOOGLE_CLIENT_ID` and `email_verified` must be true. Sessions last 90 days. |
| `POST /api/v1/auth/signout` | none | 204. Ends this session only. |
| `GET /api/v1/me` | none | `{id, email, name, picture}` |
| `DELETE /api/v1/me` | none | 204. Removes the account, its sessions and every task it owns. |
| `GET /api/v1/snapshot?from=YYYY-MM-DD&days=7` | none | `Snapshot`. The window bounds the plan (today and 7 days by default); the task list is always the whole list, and pinned tasks keep their placement whatever the window is. |
| `POST /api/v1/capture` | `{text}` or multipart `file` (pdf, eml, txt) | `{created: [id], snapshot, jev_fallback}`. `jev_fallback` is true once the user is past `RAMBL_JEV_DAILY_CAP` captures today (200 by default), when Jev is not called and `FakeJudge` judges instead. |
| `POST /api/v1/tasks` | `TaskDraft` | `Snapshot` |
| `PATCH /api/v1/tasks/{id}` | any of `title, duration_min, when, done, label` | `Snapshot` |
| `POST /api/v1/tasks/{id}/revise` | `{instruction}` (goes through the capture pipeline for one clause) | `Snapshot` |
| `PUT /api/v1/tasks/order` | `{ids: [...]}` (full order) | `Snapshot` |
| `DELETE /api/v1/tasks/{id}` | none | `Snapshot` |
| `POST /api/v1/import` | multipart `file` (.ics) | `{created: [id], updated: [id], snapshot}`. Events become tasks (`At`, `source: Calendar`); same UID updates the existing task. Cancelled and transparent events are skipped. Overlapping `At` tasks are allowed and drawn side by side. |
| `GET /api/v1/calendar.ics` | none | iCalendar |

The server is the only planner: the client never computes placements.

### 6.1 Concurrent writes

Task edits use an `updated_at` field and last-write-wins per task. A reorder conflict is handled by re-sending the full order, rather than merging partial positions.

### Configuration

| Variable | Default | Meaning |
|---|---|---|
| `RAMBL_DATA` | `data/rambl.db` | The v2 SQLite file. |
| `RAMBL_DEFAULT_TZ` | `America/New_York` | Zone for requests with no `X-Rambl-TZ`. `TZ_NAME` is read behind it so `.env` files written for v1 keep working. |
| `GOOGLE_CLIENT_ID` | unset | The OAuth client ID tokens must be minted for. Unset means local mode: one user, no sign-in. |
| `RAMBL_JEV_DAILY_CAP` | `200` | Captures per user per day before capture falls back to `FakeJudge`. |
| `RAMBL_JEV_DAILY_CAP_ANON` | `30` | Captures per anonymous user per day before capture falls back to `FakeJudge` with `jev_fallback: true`. |

Working hours: 08:00 to 23:59:59 local time, every day.

## 7. Server (`server/`)

- `Store` trait: `tasks(user)`, `put(user, Task)`, `delete(user, id)`, `set_order(user, ids)`, `find_by_uid(user, uid)`, plus `upsert_user_and_session`, `delete_session`, `session_user`, `delete_user` and `captures_today`. The user id is a parameter rather than a filter applied afterwards, so scoping cannot be forgotten. `SqliteStore` for real, `MemoryStore` for tests; the same suite runs against both.
- `auth` module: Google sign-in, sessions and the `TokenVerifier` trait. Sign-in is verified **before** the store lock is taken — a handler holding a `MutexGuard` across an `await` has a non-`Send` future, which axum rejects.
- `App { store, judge, clock }`: one method per endpoint, each taking the user id. It loads once, mutates and returns the snapshot built from that same list inside one critical section, so a snapshot is never half-updated and a write is never read back twice.
- Handlers only parse input, build the `LocalContext` from the header and `clock`, call `App`, and map errors.
- Config from env: `TYPESAFE_API_KEY`, `PORT`, `RAMBL_DATA` (db path), `RAMBL_DEFAULT_TZ` (when the header is missing).

### 7.1 Recurrence (later)

Recurrence is design-only for now. A series stores weekly-on-these-days, an until-date, and exceptions. The scheduler expands occurrences; imported iCalendar `RRULE`s map to the same model.

## 8. Web (`web/`, Leptos CSR)

Plain and fast: a text box with a mic button (Web Speech API via `web-sys`), the task list, and a 7-day week grid. Light and dark follow the OS. No animations, sounds or textures, except the mascot below. One `api.rs` module makes every request and stores the returned `Snapshot` in one signal; the list and the week are derived from that signal only. Reorder by drag sends `PUT /tasks/order`. Built by `trunk build --release` into `web/dist`, which the server serves.

### 8.1 Rambl, the mascot

The pen character (`art/illustrations/pen.svg`) is named **Rambl**. He lives in a small circle next to the text box and is the talk button: tap to send, hold to talk.

- **Drag:** he can be picked up and dragged anywhere.
- **Fall:** dropped on empty space, he falls to the bottom of the window (short gravity animation, a small bounce), then skitters back to his circle.
- **Dropped on a task or event** (list row or week block): he stays there and asks, a bit annoyed, what you want, in a small speech bubble ("Okay, what about *Lab report*?") with the task's actions: Edit, Move, Done, Delete, and Never mind. Picking one, or Never mind, sends him home.
- **Talking always works:** wherever he is, hold on him to talk. His empty circle stays a working talk button while he is away, and holding Space (focus outside a text field) talks too. Dragging him off his circle never disables voice.
- **Testable:** his behaviour is a small state machine (`Home`, `Dragging`, `Falling`, `Returning`, `Asking(task)`) in `web/src/mascot.rs`, unit-tested without the DOM; the view only animates between states.
- `prefers-reduced-motion`: no fall or skitter; he reappears in his circle.

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
- Redesign animations, sounds and textures with a parchment look; the current stripped-down presentation is intermediate, not final.
