# Rambl architecture (v2)

The one design document. The hackathon build (v1: `static/` front end, `server/src/main.rs`) grew fast and broke in ways a design should prevent. v2 is a rewrite toward a small, testable base. Where code disagrees with this file, this file wins; change the file first.

## 1. What Rambl does

You type or say a ramble ("um call mom tomorrow at 3, lab report due Wednesday"). Rambl turns it into tasks, plans them into your week around your calendar, and keeps the list and the week in agreement. Import busy time from an `.ics` file, export the plan as `.ics`.

## 2. Failures v1 had, and the rule that prevents each

| v1 failure | Cause | v2 rule |
|---|---|---|
| "Tomorrow" became Tuesday on Sunday evening | Day math used the UTC date; Jev was told UTC time | All time goes through one `LocalContext { now, tz }`. The user's IANA zone comes from the client. Nothing reads `Utc::now()` or a `.date_naive()` of a UTC value outside `core::clock`. |
| Task time and calendar time disagreed | The list showed the deadline, the calendar showed the placement, and they were fetched separately | The server returns one `Snapshot` (tasks + plan + busy) from every call. The list and the week render from the same snapshot. A task's shown time is its placement. |
| AI put tasks on the wrong day or time | Jev's free-form date answer was re-parsed; code and Jev each did date logic | Jev only **selects** among candidates that code found in the text. Code alone turns the chosen candidate into a date (`dates` crate). |
| Titles lost words or kept filler | Ad hoc prefix stripping | Code proposes title spans; Jev chooses one; one tested cleaner removes filler. |
| Scheduler walked UTC days | Working hours stored as UTC time of day | Working hours are local; the scheduler resolves each local day with its own offset (kept from v1 #77). |
| Cancelled / transparent calendar events blocked time; mutex deadlock | Fixes landed late, no invariant tests | Kept fixes, each with a regression test. Locks are never held across a call that locks again (one `App` method per request). |

## 3. Workspace layout

```
crates/
  core/       types shared by server and web (wasm-safe: serde + chrono only)
  dates/      text -> date/time candidates; candidate + LocalContext -> instant. Pure.
  scheduler/  tasks + busy + hours + LocalContext -> Plan. Pure.
  judge/      trait Judge + JevJudge (HTTP) + FakeJudge (tests)
  capture/    ramble/PDF/email -> Vec<TaskDraft>, using dates + judge
server/       axum: Store trait (Sqlite, Memory), App service, thin handlers
web/          Leptos CSR app (trunk), talks only through web/src/api.rs
```

Removed in v2: `static/` (after `web/` reaches parity), the pen, sounds, paper textures and every animation, `/api/message`, `/api/schedule`, spoken replies, `demo/` (its rambles move to `crates/capture/tests/fixtures/`), root `src/`, `Makefile` (one `run-local.ps1` + `run-local.sh` stay).

## 4. Core types (`crates/core`)

```rust
pub struct LocalContext { pub now: DateTime<Utc>, pub tz: chrono_tz::Tz }
impl LocalContext { pub fn today(&self) -> NaiveDate; pub fn local(&self, t: DateTime<Utc>) -> NaiveDateTime; pub fn at(&self, d: NaiveDate, t: NaiveTime) -> DateTime<Utc>; }
pub trait Clock: Send + Sync { fn now(&self) -> DateTime<Utc>; }   // SystemClock, FixedClock

pub enum When { Anytime, DueBy(DateTime<Utc>), At(DateTime<Utc>) } // At = fixed start (pinned)

pub struct Task { id: TaskId, title: String, duration_min: u16, when: When, done: bool,
                  order: i64, label: Label, priority: Priority, source: Source, created: DateTime<Utc> }
pub struct TaskDraft { title, duration_min, when, label, priority, source }      // no id
pub struct Busy { start, end, title }
pub struct Placement { task: TaskId, start: DateTime<Utc>, end: DateTime<Utc> }
pub struct Plan { placements: Vec<Placement>, unplaced: Vec<TaskId> }
pub struct Snapshot { tasks: Vec<Task>, plan: Plan, busy: Vec<Busy>, tz: String, generated: DateTime<Utc> }
pub struct ApiError { code: ErrorCode, message: String }
```

`When::At` always places at that instant; `DueBy` places before it; `Anytime` places in the first free slot. One task has at most one placement in v2 (no splitting).

## 5. Capture pipeline (`crates/capture`)

1. **Segment** (code): split text into clauses.
2. **Candidates** (code, `dates`): per clause, title spans, date phrases ("tomorrow", "Wednesday", "Oct 30"), time phrases ("at 3", "10am"), duration phrases. Each candidate already carries its resolved value from `LocalContext`.
3. **Judge** (one Jev request per capture, all questions in parallel): per clause `is_task` (Noul), `title` (Choice of spans), `day` (Choice of date candidates + none), `time` (Choice of time candidates + none), `duration` (Choice), `label` (Choice), `priority` (Score). Jev gets the local time as text for context only.
4. **Resolve** (code): build `TaskDraft`. Day + time -> `When::At`; day only -> `DueBy(day 17:00 local)`; neither -> `Anytime`.

Without a Jev key, `FakeJudge`'s deterministic rules pick the first candidate. Tests use it, so the pipeline is tested end to end without the network.

## 6. HTTP API v1

All JSON. Every request carries the header `X-Rambl-TZ: America/New_York` (from `Intl.DateTimeFormat().resolvedOptions().timeZone`). Every mutating call returns the full `Snapshot`. Errors are `{ "error": { "code", "message" } }` with a fitting status.

| Method and path | Body | Returns |
|---|---|---|
| `GET /api/v1/snapshot?from=YYYY-MM-DD&days=7` | none | `Snapshot` |
| `POST /api/v1/capture` | `{text}` or multipart `file` (pdf, eml, txt) | `{created: [id], snapshot}` |
| `POST /api/v1/tasks` | `TaskDraft` | `Snapshot` |
| `PATCH /api/v1/tasks/{id}` | any of `title, duration_min, when, done, label` | `Snapshot` |
| `POST /api/v1/tasks/{id}/revise` | `{instruction}` (goes through the capture pipeline for one clause) | `Snapshot` |
| `PUT /api/v1/tasks/order` | `{ids: [...]}` (full order) | `Snapshot` |
| `DELETE /api/v1/tasks/{id}` | none | `Snapshot` |
| `POST /api/v1/busy/import` | multipart `file` (.ics), `confirm=true` to accept clashes | `Snapshot`, or 409 `{clashes}` |
| `GET /api/v1/calendar.ics` | none | iCalendar |

The server is the only planner: the client never computes placements.

## 7. Server (`server/`)

- `Store` trait: `tasks()`, `put(Task)`, `delete(id)`, `set_order(ids)`, `busy()`, `replace_busy(..)`. `SqliteStore` for real, `MemoryStore` for tests.
- `App { store, judge, clock, hours }`: one method per endpoint. It loads, mutates, re-plans and returns the snapshot inside one critical section, so a snapshot is never half-updated.
- Handlers only parse input, build the `LocalContext` from the header and `clock`, call `App`, and map errors.
- Config from env: `TYPESAFE_API_KEY`, `PORT`, `RAMBL_DATA` (db path), `RAMBL_DEFAULT_TZ` (when the header is missing).

## 8. Web (`web/`, Leptos CSR)

Plain and fast: a text box with a mic button (Web Speech API via `web-sys`), the task list, and a 7-day week grid. Light and dark follow the OS. No animations, sounds or textures. One `api.rs` module makes every request and stores the returned `Snapshot` in one signal; the list and the week are derived from that signal only. Reorder by drag sends `PUT /tasks/order`. Built by `trunk build --release` into `web/dist`, which the server serves.

## 9. Testing (the definition of done for every change)

- `dates`: a table of phrases at fixed instants, including Sunday 21:00 New York (Monday in UTC), both DST change days, month and year rollover, "next Tuesday" on a Tuesday, and every weekday from every weekday.
- `scheduler`: invariants for random inputs: no overlap with busy or each other, `At` tasks exactly at their start, `DueBy` tasks end by their due time when there is room, the order breaks ties.
- `capture`: golden fixtures (rambles in, drafts out) with `FakeJudge`; live Jev tests are `#[ignore]` and run by hand.
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
| 4 | delete `static/`, old routes and dead code; final docs pass | phase 3 |

Other docs: `README.md` (run and deploy), `docs/jev.md` (Jev API notes).
