# AGENTS.md

Instructions for any AI coding agent working in this repository. Read this before your first edit.

Rambl is a speed-focused to-do list: you ramble, Jev (TypeSafe's model) helps turn it into tasks, a scheduler plans your week, and Rambl the pen mascot lives beside the capture box. Fast, readable and well tested is the bar.

## Read first

- `docs/ARCHITECTURE.md` is the spec. Where code and spec disagree, the spec wins. If your change alters behaviour, update the spec in the same PR.
- `docs/FEATURES.md` lists every feature and where it lives.
- `docs/jev.md` describes the Jev API.
- `docs/TUNING.md` says where every tunable number lives. Never write a limit, timeout or default inline.

## Where things are

| Path | What |
|---|---|
| `crates/core` | Shared types (`Task`, `When`, `Snapshot`, `ApiError`) |
| `crates/dates` | Date and time phrases, always in the user's time zone |
| `crates/judge` | Jev client and the offline `FakeJudge` |
| `crates/capture` | Ramble to tasks: split clauses, find candidates, ask the judge |
| `crates/scheduler` | Plans tasks into working hours |
| `crates/rambl-brain` | The mascot's behaviour; no web code |
| `crates/web-core` | The web page's core: rules and numbers with no DOM (`tuning.rs` is here) |
| `server/src/v2` | HTTP API, storage, accounts |
| `web/` | Leptos page built with trunk |
| `static/`, old `server/src` routes | v1, being deleted; do not extend |

## Working rules

1. **Own branch, own worktree.** `git worktree add <path> origin/main -b <you>/<topic>`. Never edit the main checkout or commit to `main`.
2. **Stay in scope.** Touch only the files your task names. If the fix needs another area, stop and ask.
3. **Idiomatic Rust.** Prefer expressive, idiomatic code: iterators and combinators over index loops, enums and `match` over flags, types that make bad states unrepresentable, small pure functions. Clever is welcome when it makes the code shorter, clearer or easier to extend; it is not when only its author can follow it. Prefer deleting code to adding it, and add no dependency without a reason in the PR.
4. **Time.** Never read the wall clock or assume UTC outside `core`'s `Clock` and `LocalContext`.
5. **Secrets.** Never commit `.env`, keys or `data/`. Never print a key.

Claude agents: browser smoke checks are `node tools/smoke/run.mjs`, which drives a release build in Edge at desk and phone sizes. Run it after a change to the planner, rows, the edit sheet, the gear menu or Rambo. Agents that may not run servers or browsers (see STATUS.md) do not run it. It is not in CI (see `tools/smoke/README.md`).

## Definition of done (run all of it, once, at the end)

Make all your edits first, then run the gate once. Re-running it after every small edit wastes budget and trips the office's loop detector.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p rambl-web --target wasm32-unknown-unknown   # web code only compiles fully for the browser
cargo clippy -p rambl-web --target wasm32-unknown-unknown --all-targets -- -D warnings   # browser-only code lints here, not on the host
cd web && trunk build --release                              # only if you touched web/
```

Then verify your pushed branch from a **fresh** checkout, because ignored folders (for example any `data/`) never reach Git:

```sh
git worktree add <tmp> origin/<your-branch> && cd <tmp> && cargo test --workspace
```

## Test what you changed

- Add a test for every behaviour you add or fix. A bug fix comes with a test that fails without the fix; check that it does.
- Test the rule, not the implementation: inputs and outputs, invariants (no two placed tasks overlap, each user sees only their own tasks), golden files for capture.
- Fixed clocks and seeded randomness only; no network in tests (use `FakeJudge` and fake verifiers).

## Review your own diff before reporting

Read `git diff origin/main` top to bottom and check:

- [ ] Does exactly what the task asked, nothing extra.
- [ ] No dead code, debug prints, commented-out blocks or TODOs without an owner.
- [ ] Names say what things are; comments explain why, not what.
- [ ] Errors are handled, not unwrapped, on any path a user can reach.
- [ ] Spec and README updated if behaviour changed.
- [ ] Anything you were unsure about is listed in your report.

## Lessons from review (read these)

- **Report the real gate.** Paste the actual result of every gate command. Never say a check passed unless you ran it in your worktree, rebased on the latest `origin/main`.
- **Leptos: nothing after `view!`.** Helper `fn`s placed after `view! { ... }` inside a component turn the view into a statement (`expected (), found View`). Put helpers at module level.
- **Background tasks start from `main()` only.** Never spawn timers, consoles or backups from code that tests call (`app_from_env`, routers, constructors).
- **Durable state lives in the database,** never in a process-wide `HashSet`/`static` that a restart forgets.
- **Never trust ids from the client.** Validate anything a request sends as an identity against the database.
- **Docs describe code, not plans.** Mark anything not in code `(planned)`; check with grep before writing a path or function name.
- **Don't wait for permission.** When the job is clear, start and finish it; only ask when you are genuinely blocked. Report partial work honestly and say what is left.
- **Rebase before you report.** `git fetch && git rebase origin/main`, then run the gate.

## Report

Open one PR. In your done message give: the PR number and head commit, what changed (short), how you tested it, and anything a reviewer should look at. Say plainly if something failed or was skipped.

## Team agents: status and context (if you work on the hive floor)

- Read `Z:/Z---Misc/AITODO/hive/STATUS.md` at the start of every task and whenever you go idle. Work only on the job your row names; if your row is wrong, tell god.
- End every message to god (and every done report) with a line `ctx: NN%`: how full your context window is. Estimate if your tool does not show it.
- At 50% or more, put `[ctx NN%]` in the subject. Finish your current step, then wait: god asks the human to compact you. Never start a big task above 50%.
