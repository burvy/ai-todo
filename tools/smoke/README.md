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

Each check runs at desk size (1280x800, mouse) and at phone size (390x844, touch):

1. **load shows Rambo**: the page loads and Rambo is on the page.
2. **a typed capture writes a row**: type a sentence into the capture box and press Enter; a new row appears.
3. **a swipe finishes a row and the tape brings it back**: drag a row to the right; it goes, and the tape (undo) brings it back.
4. **Rambo dragged to the page goes home**: drag Rambo to the middle of the page and release; he goes home.
5. **no page errors**: no uncaught exception or console error during the run.

## How it runs

- A release server starts on a free port with a fresh data file in a temporary folder.
- Edge starts muted (`--mute-audio`) with its own profile in a temporary folder. The page is set to the window's focus and lifecycle state (`Emulation.setFocusEmulationEnabled`, `Page.setWebLifecycleState` active), so a headless page behaves as an open one.
- The server and the browser are stopped by their own process IDs in a `finally` block, and nothing else is stopped.
- The temporary folders are removed afterwards. A folder the browser still holds is reported, not treated as a failed check.

Exit status is 0 when every check passes and 1 otherwise. Each line of output is `PASS` or `FAIL` with the reason.

## Files

- `run.mjs`: the server, the browser, the checks, and the cleanup.
- `cdp.mjs`: a small Chrome DevTools Protocol client over Node's WebSocket.
