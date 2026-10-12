# AGENTS.md

Instructions for any AI coding agent working in this repository. Read this before your first edit.

Rambl is a speed-focused to-do list: you ramble, Jev (TypeSafe's model) helps turn it into tasks, a scheduler plans your week, and Rambl the pen mascot lives beside the capture box. Simple, fast and well tested beats clever.

## Read first

- `docs/ARCHITECTURE.md` is the spec. Where code and spec disagree, the spec wins. If your change alters behaviour, update the spec in the same PR.
- `docs/FEATURES.md` lists every feature and where it lives.
- `docs/jev.md` describes the Jev API.

## Where things are

| Path | What |
|---|---|
| `crates/core` | Shared types (`Task`, `When`, `Snapshot`, `ApiError`) |
| `crates/dates` | Date and time phrases, always in the user's time zone |
| `crates/judge` | Jev client and the offline `FakeJudge` |
| `crates/capture` | Ramble to tasks: split clauses, find candidates, ask the judge |
| `crates/scheduler` | Plans tasks into working hours |
| `crates/rambl-brain` | The mascot's behaviour; no web code |
| `server/src/v2` | HTTP API, storage, accounts |
| `web/` | Leptos page built with trunk |
| `static/`, old `server/src` routes | v1, being deleted; do not extend |

## Working rules

1. **Own branch, own worktree.** `git worktree add <path> origin/main -b <you>/<topic>`. Never edit the main checkout or commit to `main`.
2. **Stay in scope.** Touch only the files your task names. If the fix needs another area, stop and ask.
3. **Simple first.** Prefer deleting code to adding it. No new dependency without a reason in the PR.
4. **Time.** Never read the wall clock or assume UTC outside `core`'s `Clock` and `LocalContext`.
5. **Secrets.** Never commit `.env`, keys or `data/`. Never print a key.

## Definition of done (run all of it, once, at the end)

Make all your edits first, then run the gate once. Re-running it after every small edit wastes budget and trips the office's loop detector.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p rambl-web --target wasm32-unknown-unknown   # web code only compiles fully for the browser
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

## Report

Open one PR. In your done message give: the PR number and head commit, what changed (short), how you tested it, and anything a reviewer should look at. Say plainly if something failed or was skipped.
