// Browser smoke suite: a release server on a free port with fresh data, Edge muted in its
// own profile, and four scenarios at desk and phone sizes. Run it with `node tools/smoke/run.mjs`
// (see README.md). Stops only the server and browser it started, by their own PIDs.

import { execFileSync, spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import net from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Cdp } from "./cdp.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..", "..");
const windows = process.platform === "win32";
const serverBin =
  process.env.SMOKE_SERVER ??
  join(repo, "target", "release", windows ? "rambl-server.exe" : "rambl-server");
const edgeBin =
  process.env.SMOKE_EDGE ??
  (windows
    ? "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe"
    : "microsoft-edge");

const SIZES = [
  { name: "desk", width: 1280, height: 800, phone: false },
  { name: "narrow desk", width: 640, height: 800, phone: false },
  { name: "phone", width: 390, height: 844, phone: true },
];

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

async function waitFor(check, ms, what) {
  const until = Date.now() + ms;
  for (;;) {
    const value = await check().catch(() => null);
    if (value) return value;
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}

function freePort() {
  return new Promise((resolvePort, reject) => {
    const probe = net.createServer();
    probe.unref();
    probe.on("error", reject);
    probe.listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolvePort(port));
    });
  });
}

// The processes this run started, by PID. Only these are ever stopped.
const owned = new Set();

// Stops one process and its children, by PID, and nothing else.
function killTree(pid) {
  owned.delete(pid);
  try {
    if (windows) execFileSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
    else process.kill(pid, "SIGTERM");
  } catch {
    // Already gone.
  }
}

// A child that failed to start has no PID, and its error must not crash the run: the
// wait that follows then times out and reports it.
function trackChild(child) {
  child.on("error", () => {});
  if (child.pid !== undefined) owned.add(child.pid);
}

// Ctrl+C or a normal exit stops whatever this run still owns.
function stopOwned() {
  for (const pid of [...owned]) killTree(pid);
}
process.on("SIGINT", () => {
  stopOwned();
  process.exit(130);
});
process.on("exit", stopOwned);

// Only what the server and Edge need from the shell. The rest of the environment is
// left out, so a key in the shell (for example TYPESAFE_API_KEY) can never reach the
// server and send test captures to the real judge.
const ALLOWED_ENV = ["PATH", "SystemRoot", "TEMP", "TMP", "USERPROFILE", "SYSTEMDRIVE", "HOME"];
function childEnv(extra) {
  const env = {};
  for (const name of ALLOWED_ENV) if (process.env[name] !== undefined) env[name] = process.env[name];
  return { ...env, ...extra };
}

// Each start returns the process at once and is owned from then on, so a failed wait
// still lets the caller's cleanup stop it by PID.
async function launchServer(dataDir) {
  const port = await freePort();
  const child = spawn(serverBin, [], {
    env: childEnv({
      RAMBL_PORT: String(port),
      RAMBL_BIND: "127.0.0.1",
      RAMBL_DATA: join(dataDir, "rambl.db"),
    }),
    stdio: "ignore",
  });
  trackChild(child);
  return { child, port };
}

async function waitForServer(server) {
  await waitFor(async () => (await fetch(`http://127.0.0.1:${server.port}/`)).ok, 20000, "the server");
}

async function launchBrowser(profileDir, size) {
  const port = await freePort();
  const child = spawn(
    edgeBin,
    [
      "--headless=new",
      "--mute-audio",
      "--no-first-run",
      "--no-default-browser-check",
      `--user-data-dir=${profileDir}`,
      `--remote-debugging-port=${port}`,
      `--window-size=${size.width},${size.height}`,
      "about:blank",
    ],
    { stdio: "ignore", env: childEnv({}) },
  );
  trackChild(child);
  return { child, port };
}

async function connectBrowser(browser) {
  await waitFor(async () => (await fetch(`http://127.0.0.1:${browser.port}/json/version`)).ok, 20000, "the browser");
  const target = await (await fetch(`http://127.0.0.1:${browser.port}/json/new?about:blank`, { method: "PUT" })).json();
  browser.cdp = await Cdp.connect(target.webSocketDebuggerUrl);
}

// The page as a scenario sees it: rectangles, element counts and real pointer input.
function page(cdp, size, errors) {
  const evaluate = async (expression) => {
    const result = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
    return result.result?.value;
  };
  const rect = (selector) =>
    evaluate(`(() => { const e = document.querySelector(${JSON.stringify(selector)});
      if (!e) return null; const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; })()`);
  const count = (selector) => evaluate(`document.querySelectorAll(${JSON.stringify(selector)}).length`);
  const has = (selector) => evaluate(`!!document.querySelector(${JSON.stringify(selector)})`);

  const pointer = async (phase, x, y) => {
    if (size.phone) {
      const type = { down: "touchStart", move: "touchMove", up: "touchEnd" }[phase];
      await cdp.send("Input.dispatchTouchEvent", {
        type,
        touchPoints: phase === "up" ? [] : [{ x, y, id: 1 }],
      });
    } else {
      const type = { down: "mousePressed", move: "mouseMoved", up: "mouseReleased" }[phase];
      await cdp.send("Input.dispatchMouseEvent", {
        type,
        x,
        y,
        button: "left",
        buttons: phase === "up" ? 0 : 1,
        clickCount: phase === "down" ? 1 : 0,
      });
    }
  };

  const drag = async (from, to, steps = 24) => {
    await pointer("down", from.x, from.y);
    for (let i = 1; i <= steps; i++) {
      await pointer("move", from.x + ((to.x - from.x) * i) / steps, from.y + ((to.y - from.y) * i) / steps);
      await sleep(16);
    }
    await pointer("up", to.x, to.y);
  };

  const tap = async (x, y) => {
    await pointer("down", x, y);
    await pointer("up", x, y);
  };

  const type = async (text) => {
    await cdp.send("Input.insertText", { text });
  };

  const enter = async () => {
    const key = { key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 };
    await cdp.send("Input.dispatchKeyEvent", { type: "keyDown", ...key, text: "\r" });
    await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...key });
  };

  return { evaluate, rect, count, has, pointer, drag, tap, type, enter, errors };
}

const scenarios = {
  async "load shows Rambo"(p) {
    await waitFor(() => p.has(".rambo"), 20000, "Rambo on the page");
    await sleep(500);
  },

  async "a typed capture writes a row"(p) {
    await waitFor(() => p.has('input[aria-label="Ramble"]'), 10000, "the capture box");
    const box = await p.rect('input[aria-label="Ramble"]');
    const before = await p.count("li.task-row");
    await p.tap(box.x + box.w / 2, box.y + box.h / 2);
    await p.type("buy walrus food and call heron vet");
    await p.enter();
    await waitFor(async () => (await p.count("li.task-row")) > before, 15000, "the new row");
  },

  async "a swipe finishes a row and the tape brings it back"(p) {
    await waitFor(() => p.has("li.task-row"), 10000, "a row");
    const before = await p.count("li.task-row");
    const row = await p.rect("li.task-row");
    const y = row.y + row.h / 2;
    await p.drag({ x: row.x + 20, y }, { x: row.x + row.w * 0.8, y });
    await waitFor(async () => (await p.count("li.task-row")) === before - 1, 8000, "the row to go");
    // The tape swings in after the row goes; tap it once it has settled.
    await sleep(900);
    const tape = await waitFor(() => p.rect(".scrap-undo.done"), 8000, "the tape");
    await p.tap(tape.x + tape.w / 2, tape.y + tape.h / 2);
    await waitFor(async () => (await p.count("li.task-row")) === before, 8000, "the row to come back");
  },

  async "a swipe turns the page and the book flip is drawn, not blank, mid-turn"(p) {
    await waitFor(() => p.has("li.task-row"), 10000, "a row");
    const stacked = await p.evaluate('matchMedia("(max-width: 700px)").matches');
    if (!stacked) {
      // Wide desk: the two pages lie side by side, so there is no page to turn.
      const list = await p.rect(".tasks-pane");
      const planner = await p.rect(".planner-page");
      if (!(list?.w > 0 && planner?.w > 0)) throw new Error("both pages should show side by side");
      return;
    }
    const main = await p.rect("main");
    const y = main.y + main.h / 2;
    const from = { x: main.x + main.w - 40, y };
    await p.pointer("down", from.x, from.y);
    for (let i = 1; i <= 10; i++) {
      await p.pointer("move", from.x - (main.w * 0.5 * i) / 10, y);
      await sleep(16);
    }
    const held = await p.evaluate(TURN_NOW);
    await p.pointer("up", from.x - main.w * 0.5, y);
    if (!held.flipping) throw new Error("the flip did not start: " + JSON.stringify(held));
    if (!(held.front.w > 0 && held.front.w < held.viewport)) throw new Error("the leaf is not swung: " + JSON.stringify(held.front));
    if (!held.frontSeen) throw new Error("the leaf is hidden: " + JSON.stringify(held));
    if (!held.underSeen) throw new Error("the page beneath is blank: " + JSON.stringify(held));
    if (!held.fxShown || !(held.fx.w > 0)) throw new Error("the leaf's light and shadow are not drawn");
    if (!held.leafHasContent) throw new Error("the leaf has no list on it");
    await waitFor(async () => (await p.evaluate(TURN_NOW)).pane === "1" && !(await p.evaluate(TURN_NOW)).flipping, 5000, "the turn to land on the planner");
    // Back, by the note at the page's foot: the same flip, with nobody holding the page.
    const note = await waitFor(() => p.evaluate(`(() => { const b = [...document.querySelectorAll("button")].find((e) => e.textContent.includes("task list on the previous page")); if (!b) return null; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`), 8000, "the note back");
    await p.tap(note.x, note.y);
    await sleep(150);
    const back = await p.evaluate(TURN_NOW);
    if (!back.flipping || !back.fxShown || !back.frontSeen || !back.underSeen) throw new Error("the turn back is not drawn: " + JSON.stringify(back));
    await waitFor(async () => (await p.evaluate(TURN_NOW)).pane === "0" && !(await p.evaluate(TURN_NOW)).flipping, 5000, "the turn to land on the list");
  },

  async "a slip dragged on the planner springs to its new time, and back where it sat"(p) {
    const stacked = await p.evaluate('matchMedia("(max-width: 700px)").matches');
    if (stacked) {
      // Where the pages stack, the planner is behind the list; the wide desk checks it.
      return;
    }
    // A slip in sight with room below it, from the tasks the checks above made.
    const slip = await waitFor(() => p.evaluate(SLIP_IN_SIGHT), 15000, "a slip on the planner");
    const top = () => p.evaluate(`document.querySelector(".week-scroll .placement[data-id='${slip.id}']")?.getBoundingClientRect().top ?? null`);
    const from = { x: slip.x, y: slip.y };
    // Picked up, carried off and brought back: dropped where it sat, the ghost springs back
    // to it and the slip stays.
    await p.pointer("down", from.x, from.y);
    for (const dy of [5, 10, 15, 20, 15, 10, 5, 1]) {
      await p.pointer("move", from.x, from.y + dy);
      await sleep(16);
    }
    if (!(await p.has(".drag-ghost"))) throw new Error("no ghost while the slip is carried");
    await p.pointer("up", from.x, from.y + 1);
    await waitFor(async () => !(await p.has(".drag-ghost")), 5000, "the ghost back on its slip");
    const back = await top();
    if (Math.abs(back - slip.top) > 3) throw new Error(`a slip dropped where it sat moved (${slip.top} to ${back})`);
    // Dragged two hours down: the ghost rides the pointer, is still drawn the moment it is
    // let go (it springs to the slot, it does not vanish), and the slip lands there.
    await p.pointer("down", from.x, from.y);
    for (let i = 1; i <= 15; i++) {
      await p.pointer("move", from.x, from.y + (slip.down * i) / 15);
      await sleep(16);
    }
    if (!(await p.has(".drag-ghost"))) throw new Error("no ghost while the slip is held");
    await p.pointer("up", from.x, from.y + slip.down);
    if (!(await p.has(".drag-ghost"))) throw new Error("the ghost vanished on letting go");
    await waitFor(async () => !(await p.has(".drag-ghost")) && (await top()) > slip.top + slip.down / 2, 8000, "the slip at its new time");
  },

  async "Rambo dragged to the page goes home"(p) {
    await waitFor(() => p.has(".rambo"), 10000, "Rambo");
    const rambo = await p.rect(".rambo");
    const from = { x: rambo.x + rambo.w / 2, y: rambo.y + rambo.h / 2 };
    const to = { x: p.viewport.width * 0.5, y: p.viewport.height * 0.5 };
    await p.drag(from, to);
    await waitFor(() => p.has(".rambo.is-home"), 8000, "Rambo to go home");
  },
};

// A slip on the planner in sight with room below it: its id, a point on it to hold, its
// top, and how far down two hours are.
const SLIP_IN_SIGHT = `(() => {
  const scroll = document.querySelector(".week-scroll");
  const hour = document.querySelector(".week-scroll .hour");
  if (!scroll || !hour) return null;
  const s = scroll.getBoundingClientRect();
  const down = 2 * hour.getBoundingClientRect().height;
  for (const e of document.querySelectorAll(".week-scroll .placement[data-id]")) {
    const r = e.getBoundingClientRect();
    if (r.top > s.top + 40 && r.bottom + down < s.bottom - 20 && r.left > s.left && r.right < s.right) {
      return { id: e.dataset.id, x: r.left + r.width / 2, y: r.top + 6, top: r.top, down };
    }
  }
  return null;
})()`;

// What the page turn shows right now: whether the book flip is under way, and whether what it
// draws is there (the leaf with the list on it, the page beneath it, the leaf's light).
const TURN_NOW = `(() => {
  const main = document.querySelector("main");
  const box = (e) => { const r = e ? e.getBoundingClientRect() : { width: 0, height: 0 }; return { w: r.width, h: r.height }; };
  // Drawn at all: has a size, is not hidden and is not see-through.
  const seen = (e) => { if (!e) return false; const c = getComputedStyle(e); const r = e.getBoundingClientRect();
    return r.width > 0 && r.height > 0 && c.display !== "none" && c.visibility === "visible" && Number(c.opacity) > 0; };
  const front = main.querySelector(".bend-front");
  const under = main.querySelector(".bend-under");
  const fx = main.querySelector(".bend-fx");
  return {
    flipping: main.classList.contains("flipping"),
    pane: main.dataset.pane,
    front: box(front),
    under: box(under),
    fx: box(fx),
    fxShown: seen(fx),
    frontSeen: seen(front),
    underSeen: seen(under),
    leafHasContent: !!front && [...front.querySelectorAll("li.task-row")].some(seen),
    viewport: window.innerWidth,
  };
})()`;

async function runSize(size) {
  const results = [];
  const dataDir = mkdtempSync(join(tmpdir(), `rambl-smoke-${size.name}-`));
  const profileDir = mkdtempSync(join(tmpdir(), `rambl-smoke-profile-${size.name}-`));
  let server = null;
  let browser = null;
  try {
    server = await launchServer(dataDir);
    await waitForServer(server);
    browser = await launchBrowser(profileDir, size);
    await connectBrowser(browser);
    const errors = [];
    const { cdp } = browser;
    cdp.on((method, params) => {
      if (method === "Runtime.exceptionThrown") errors.push(params.exceptionDetails?.exception?.description ?? "exception");
      if (method === "Log.entryAdded" && params.entry?.level === "error") errors.push(params.entry.text);
    });
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Log.enable");
    // Focus and lifecycle: a headless page is otherwise treated as hidden or unfocused.
    await cdp.send("Emulation.setFocusEmulationEnabled", { enabled: true });
    await cdp.send("Page.setWebLifecycleState", { state: "active" });
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width: size.width,
      height: size.height,
      deviceScaleFactor: 1,
      mobile: size.phone,
    });
    await cdp.send("Emulation.setTouchEmulationEnabled", size.phone ? { enabled: true, maxTouchPoints: 1 } : { enabled: false });
    await cdp.send("Page.navigate", { url: `http://127.0.0.1:${server.port}/` });

    const p = page(cdp, size, errors);
    p.viewport = { width: size.width, height: size.height };
    for (const [name, run] of Object.entries(scenarios)) {
      const label = `${size.name}: ${name}`;
      try {
        await run(p);
        results.push({ label, ok: true });
      } catch (error) {
        results.push({ label, ok: false, detail: String(error.message ?? error).slice(0, 300) });
      }
    }
    results.push({
      label: `${size.name}: no page errors`,
      ok: errors.length === 0,
      detail: errors.slice(0, 3).join(" | ").slice(0, 300),
    });
    cdp.close();
  } catch (error) {
    results.push({ label: `${size.name}: setup`, ok: false, detail: String(error.message ?? error).slice(0, 300) });
  } finally {
    if (browser) killTree(browser.child.pid);
    if (server) killTree(server.child.pid);
    // The browser releases its profile files a moment after it exits; a leftover temp
    // folder is not a failed check, so the cleanup is best-effort.
    await sleep(1000);
    for (const dir of [profileDir, dataDir]) {
      try {
        rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
      } catch (error) {
        console.log(`note  could not remove ${dir}: ${error.code ?? error.message}`);
      }
    }
  }
  return results;
}

let failed = 0;
for (const size of SIZES) {
  for (const result of await runSize(size)) {
    if (!result.ok) failed++;
    console.log(`${result.ok ? "PASS" : "FAIL"}  ${result.label}${result.detail ? `  (${result.detail})` : ""}`);
  }
}
console.log(failed === 0 ? "all smoke checks passed" : `${failed} smoke check(s) failed`);
process.exitCode = failed === 0 ? 0 : 1;
