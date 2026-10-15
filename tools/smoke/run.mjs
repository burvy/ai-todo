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

// Stops one process and its children, by PID, and nothing else.
function killTree(pid) {
  try {
    if (windows) execFileSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
    else process.kill(pid, "SIGTERM");
  } catch {
    // Already gone.
  }
}

async function startServer(dataDir) {
  const port = await freePort();
  const child = spawn(serverBin, [], {
    env: {
      ...process.env,
      RAMBL_PORT: String(port),
      RAMBL_BIND: "127.0.0.1",
      RAMBL_DATA: join(dataDir, "rambl.db"),
    },
    stdio: "ignore",
  });
  await waitFor(async () => (await fetch(`http://127.0.0.1:${port}/`)).ok, 20000, "the server");
  return { child, port };
}

async function startBrowser(profileDir, size) {
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
    { stdio: "ignore" },
  );
  await waitFor(async () => (await fetch(`http://127.0.0.1:${port}/json/version`)).ok, 20000, "the browser");
  const target = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: "PUT" })).json();
  return { child, cdp: await Cdp.connect(target.webSocketDebuggerUrl) };
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

  return { evaluate, rect, count, has, drag, tap, type, enter, errors };
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

  async "Rambo dragged to the page goes home"(p) {
    await waitFor(() => p.has(".rambo"), 10000, "Rambo");
    const rambo = await p.rect(".rambo");
    const from = { x: rambo.x + rambo.w / 2, y: rambo.y + rambo.h / 2 };
    const to = { x: p.viewport.width * 0.5, y: p.viewport.height * 0.5 };
    await p.drag(from, to);
    await waitFor(() => p.has(".rambo.is-home"), 8000, "Rambo to go home");
  },
};

async function runSize(size) {
  const results = [];
  const dataDir = mkdtempSync(join(tmpdir(), `rambl-smoke-${size.name}-`));
  const profileDir = mkdtempSync(join(tmpdir(), `rambl-smoke-profile-${size.name}-`));
  let server = null;
  let browser = null;
  try {
    server = await startServer(dataDir);
    browser = await startBrowser(profileDir, size);
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
