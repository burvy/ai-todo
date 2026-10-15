# Browser smoke suite

A checked-in set of browser checks for the page, run by hand. It is not part of CI and not run per PR.

## Run it

Once, from the repository root:

```sh
cargo build -p rambl-server --release
(cd web && trunk build --release)
node tools/smoke/run.mjs
```

Needs Node 22 or newer (it uses Node's built-in WebSocket; there are no packages to install) and Microsoft Edge.

Optional environment variables:

- `SMOKE_SERVER`: the server binary. Default: `target/release/rambl-server` (`.exe` on Windows).
- `SMOKE_EDGE`: the Edge executable. Default: the standard Edge path on Windows, `microsoft-edge` elsewhere.

## What it checks

Each check runs at desk size (1280x800, mouse), at a narrow desk window (640x800, mouse: the two pages stack and turn) and at phone size (390x844, touch):

1. **load shows Rambo**: the page loads and Rambo is on the page.
2. **a typed capture writes a row**: type a sentence into the capture box and press Enter; a new row appears.
3. **a swipe finishes a row and the tape brings it back**: drag a row to the right; it goes, and the tape (undo) brings it back.
4. **a swipe turns the page and the book flip is drawn, not blank, mid-turn**: where the pages stack (narrow desk, phone) swipe the list over to the planner, hold, and check the leaf is swung with the list on it, the planner shows beneath and the leaf's light is drawn; release, then turn back with the note at the foot and check the same. On the wide desk the two pages sit side by side, so it only checks both show.
5. **Rambo dragged to the page goes home**: drag Rambo to the middle of the page and release; he goes home.
6. **Rambo dropped on an empty planner hour takes a typed task**: drag Rambo onto the last empty hour on screen (a day still to come); he starts noting, and what is typed becomes a planner slip. On a phone the planner page is turned to first. This check needs a free hour on screen in a day still to come, so it fails on a day with no free hour in view.
7. **a row's edit sheet saves a title change**: open a row's edit sheet, type a new title, press Save; the sheet closes and the row shows the title.
8. **a gear setting is still set after a reload**: set the clock to 24-hour in the gear menu, reload the page, and open the menu again; 24-hour is still the active choice.
9. **no page errors**: no uncaught exception or console error during the run.

## How it runs

- A release server starts on a free port with a fresh data file in a temporary folder.
- Edge starts muted (`--mute-audio`) with its own profile in a temporary folder. The page is set to the window's focus and lifecycle state (`Emulation.setFocusEmulationEnabled`, `Page.setWebLifecycleState` active), so a headless page behaves as an open one.
- The server and the browser get only an allow-listed environment (`PATH`, `SystemRoot`, the temp and home folders, and the `RAMBL_` settings the suite sets). A key in the shell, such as `TYPESAFE_API_KEY`, is never passed on, so test captures cannot reach the real judge.
- The server and the browser are stopped by their own process IDs in a `finally` block, on Ctrl+C, and on exit. Nothing else is stopped.
- Everything the run writes (the data and profile folders, failure screenshots) goes in one run folder under the temp folder. When every check passes the folder is removed; otherwise its path is printed.

Exit status is 0 when every check passes and 1 otherwise. Each line of output is `PASS` or `FAIL` with the reason. A failed check also saves a screenshot in the run folder, whose path is printed at the end.

## Files

- `run.mjs`: the server, the browser, the checks, and the cleanup.
- `cdp.mjs`: a small Chrome DevTools Protocol client over Node's WebSocket.
