/* ============================================================
   ai-todo — parchment client
   Talks to the axum server in ../server/src/main.rs.
   Same origin by default (the server serves ./static).
   ============================================================ */

'use strict';

/* ── Config ─────────────────────────────────────────────────── */

const API = window.AI_TODO_API || '';   // e.g. 'https://ai-todo-api.burvy.dev'
const PENCIL_BELOW = 0.6;               // confidence under this = "pencilled in"
const WRITE_MS = 600;                   // time the pen takes per title
const DAY_START = 8, DAY_END = 20;      // planner hours

const LABELS = {
  School:   'var(--label-school)',
  Work:     'var(--label-work)',
  Health:   'var(--label-health)',
  Errands:  'var(--label-errands)',
  Personal: 'var(--label-personal)',
  Other:    'var(--label-other)',
};

/* ── DOM ────────────────────────────────────────────────────── */

const $ = (sel) => document.querySelector(sel);
const list       = $('#taskList');
const emptyNote  = $('#emptyNote');
const summary    = $('#taskSummary');
const box        = $('#messageBox');
const penBtn      = $('#penBtn');
const mascotTrack = $('#mascotTrack');
const marginNote = $('#marginNote');
const marginText = $('#marginText');
const gridDays   = $('#days');
const gutter     = $('#gutter');
const weekLabel  = $('#weekLabel');
const plannerEmpty = $('#plannerEmpty');
const legend     = $('#legend');
const toast      = $('#toast');

/* ── State ──────────────────────────────────────────────────── */

const state = {
  tasks: [],
  byId: new Map(),
  busy: [],
  placements: [],
  unscheduled: [],
  weekStart: startOfDay(new Date()),
  focusDate: null,
  speaking: true,
  freshIds: new Set(),
  working: false,   // a request is in flight
  editing: false,   // an inline editor is open
};

const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

/* ── Tiny helpers ───────────────────────────────────────────── */

/* The planner shows a rolling seven days from today, not Monday to Sunday.
   On a Sunday demo most placements land on the following Monday, which a
   Monday-anchored week would hide off screen. */
function startOfDay(date) {
  const d = new Date(date);
  d.setHours(0, 0, 0, 0);
  return d;
}

function addDays(date, n) {
  const d = new Date(date);
  d.setDate(d.getDate() + n);
  return d;
}

function isSameDay(a, b) {
  return a.getFullYear() === b.getFullYear() &&
         a.getMonth() === b.getMonth() &&
         a.getDate() === b.getDate();
}

function fmtTime(iso) {
  return new Date(iso).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}

function fmtDuration(min) {
  const m = Number(min) || 0;
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  const rest = m % 60;
  return rest ? `${h}h ${rest}m` : `${h}h`;
}

function fmtDeadline(iso) {
  const d = new Date(iso);
  const time = d.toLocaleTimeString([], { hour: 'numeric' }).replace(/\s/g, '').toLowerCase();
  const today = new Date();
  if (isSameDay(d, today)) return `today ${time}`;
  if (isSameDay(d, addDays(today, 1))) return `tomorrow ${time}`;
  return `${d.toLocaleDateString([], { weekday: 'short' })} ${time}`;
}

/* compact range that fits a narrow day column: "9am – 10am", "3pm – 3:30pm" */
function fmtRange(startIso, endIso) {
  const s = new Date(startIso), e = new Date(endIso);
  const h12 = (h) => (h % 12 === 0 ? 12 : h % 12);
  const mer = (h) => (h < 12 ? 'am' : 'pm');
  const pad = (n) => String(n).padStart(2, '0');
  const from = s.getMinutes() ? `${h12(s.getHours())}:${pad(s.getMinutes())}${mer(s.getHours())}`
                              : `${h12(s.getHours())}${mer(s.getHours())}`;
  const to = e.getMinutes() ? `${h12(e.getHours())}:${pad(e.getMinutes())}${mer(e.getHours())}`
                            : `${h12(e.getHours())}${mer(e.getHours())}`;
  return `${from} – ${to}`;
}

function toLocalInput(iso) {
  const d = new Date(iso);
  const pad = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function minutesIntoDay(iso) {
  const d = new Date(iso);
  return d.getHours() * 60 + d.getMinutes();
}

let toastTimer;
function say(message) {
  toast.textContent = message;
  toast.hidden = !message;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(clearToast, 3200);
}
function clearToast() {
  clearTimeout(toastTimer);
  toast.hidden = true;
}

/* ── API ────────────────────────────────────────────────────── */

async function api(path, options = {}) {
  const res = await fetch(API + path, {
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  if (res.status === 204) return null;
  if (!res.ok) {
    const error = new Error(`${options.method || 'GET'} ${path} -> ${res.status}`);
    error.status = res.status;
    throw error;
  }
  return res.json();
}

const getTasks      = () => api('/api/tasks');
const patchTask     = (id, body) => api(`/api/tasks/${id}`, { method: 'PATCH', body: JSON.stringify(body) });
const deleteTask    = (id) => api(`/api/tasks/${id}`, { method: 'DELETE' });
const sendMessage   = (text, source) => api('/api/message', { method: 'POST', body: JSON.stringify({ text, source }) });
const runSchedule   = () => api('/api/schedule', { method: 'POST' });
const getCalendar   = (from, to) => api(`/api/calendar?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`);

/* POST /api/tasks landed (f8cd143): 201 CREATED with the saved Task.
   The server runs clean_title and stamps confidence 1.0, so a manually
   added task always comes back in firm ink, never pencilled in. */
async function addTask(title) {
  return api('/api/tasks', { method: 'POST', body: JSON.stringify({ title }) });
}

/* ── The pen: three poses, and the trip up to write ──────────── */

function mood(next) {
  mascotTrack.dataset.mood = next;
  penBtn.setAttribute('aria-pressed', String(next === 'listening'));
}

/* Home is wherever the composer happens to be. Measure it while the pen has no
   transform on it, so repeated trips never compound. */
let penHomeRect = null;
function measurePenHome() {
  penHomeRect = penBtn.getBoundingClientRect();
}

function penGoHomeQuiet() {
  mascotTrack.classList.remove('is-flying');
  mascotTrack.style.removeProperty('--pen-x');
  mascotTrack.style.removeProperty('--pen-y');
}

/* Put the nib at a page point (viewport coordinates). */
function penAt(x, y) {
  if (!penHomeRect) measurePenHome();
  mascotTrack.classList.add('is-flying');
  mascotTrack.style.setProperty('--pen-x', `${Math.round(x - penHomeRect.left)}px`);
  mascotTrack.style.setProperty('--pen-y', `${Math.round(y - 22 - penHomeRect.top)}px`);
}

function penFlyTo(el) {
  if (!el) return;
  if (!penHomeRect) measurePenHome();
  const target = el.getBoundingClientRect();
  const x = (target.left + Math.min(70, target.width * 0.45)) - penHomeRect.left;
  const y = (target.top + target.height / 2) - 22 - penHomeRect.top;
  mascotTrack.classList.add('is-flying');
  mascotTrack.style.setProperty('--pen-x', `${Math.round(x)}px`);
  mascotTrack.style.setProperty('--pen-y', `${Math.round(y)}px`);
}

/* ── The pen writing along with you ──────────────────────────── */

/* While you type or speak, the pen comes to the text box, shrinks, and moves
   along the line you are on, so it looks like it is writing what you are.
   Two seconds of silence and it drifts back to its button.

   The 2s is the ask; the reset on send/blur is so it never sits parked on an
   empty box after you have already told it everything. */
const PEN_WRITE_LINGER_MS = 2000;
/* the pen's drawn size and the scale it writes at — these mirror .pen and
   .mascot-track.is-writing in styles.css, and place the nib on the caret */
const PEN_WIDTH = 30;
const PEN_HEIGHT = 52;
const PEN_SCALE = 0.78;
let penIdleTimer = null;

/* Where should the nib land? A textarea has no caret API, so mirror the text
   into an offscreen copy, put a marker where the caret is, and measure it.
   The marker lands on the caret's visual line even when the text has
   wrapped, which a plain "count the newlines" would get wrong. */
function caretPoint() {
  if (!penHomeRect) measurePenHome();
  const rect = box.getBoundingClientRect();
  if (!rect.width) return null;

  const style = getComputedStyle(box);
  const mirror = document.createElement('div');
  for (const prop of ['fontFamily', 'fontSize', 'fontWeight', 'fontStyle',
                      'letterSpacing', 'lineHeight', 'textTransform',
                      'paddingTop', 'paddingBottom', 'paddingLeft',
                      'paddingRight', 'borderRightWidth', 'borderLeftWidth',
                      'boxSizing', 'wordSpacing', 'tabSize', 'textIndent']) {
    mirror.style[prop] = style[prop];
  }
  mirror.style.position = 'absolute';
  mirror.style.visibility = 'hidden';
  mirror.style.whiteSpace = 'pre-wrap';
  mirror.style.overflowWrap = 'break-word';
  mirror.style.top = '0';
  mirror.style.left = '-9999px';
  mirror.style.width = rect.width + 'px';
  mirror.style.height = 'auto';

  mirror.textContent = box.value.slice(0, box.selectionStart ?? box.value.length);
  const marker = document.createElement('span');
  // a zero-width space so the marker has a box even at the end of a line
  marker.textContent = '';
  mirror.appendChild(marker);
  document.body.appendChild(mirror);

  const boxStyle = getComputedStyle(mirror);
  const markerRect = marker.getBoundingClientRect();
  const mirrorRect = mirror.getBoundingClientRect();
  const lineHeight = parseFloat(boxStyle.lineHeight) || rect.height;

  // caret position relative to the text box's own padding box
  const x = markerRect.left - mirrorRect.left
          + (parseFloat(boxStyle.paddingLeft) || 0);
  const y = markerRect.top - mirrorRect.top
          + (parseFloat(boxStyle.paddingTop) || 0)
          + lineHeight * 0.82;          // sit on the baseline, not the cap height
  mirror.remove();

  return {
    x: rect.left + x,
    y: rect.top + y,
    lineHeight,
    right: rect.right,
    left: rect.left,
  };
}

/* Park the pen with its nib on the caret and put it in writing pose.
   Safe to call on every keystroke: it only moves things that changed. */
function penWrites() {
  if (state.working) return;
  const point = caretPoint();
  if (!point) return;

  // The track is the pen button's box, and it shrinks about its own centre,
  // so the nib ends up (half the pen's height * scale) below the centre.
  const home = penBtn.getBoundingClientRect();
  const homeCx = home.left + home.width / 2;
  const homeCy = home.top + home.height / 2;
  const nibDrop = (PEN_HEIGHT / 2) * PEN_SCALE;

  const x = point.x - homeCx;
  // never let the pen walk past the right edge of the box
  const maxX = point.right - 6 - PEN_WIDTH * PEN_SCALE - homeCx;
  const y = (point.y - nibDrop) - homeCy;

  if (!mascotTrack.classList.contains('is-writing')) {
    mascotTrack.classList.add('is-writing');
    mood('writing');
  }
  mascotTrack.classList.add('is-flying');
  mascotTrack.style.setProperty('--pen-x', `${Math.round(Math.min(x, maxX))}px`);
  mascotTrack.style.setProperty('--pen-y', `${Math.round(y)}px`);

  clearTimeout(penIdleTimer);
  penIdleTimer = setTimeout(penGoHome, PEN_WRITE_LINGER_MS);
}

function penGoHome() {
  clearTimeout(penIdleTimer);
  penIdleTimer = null;
  mascotTrack.classList.remove('is-flying', 'is-writing');
  mascotTrack.style.removeProperty('--pen-x');
  mascotTrack.style.removeProperty('--pen-y');
  // a pen mid-write is an interruption: don't keep the writing pose on screen
  if (mascotTrack.dataset.mood === 'writing' && typeof listening !== 'undefined'
      && !listening) mood('idle');
}

const wait = (ms) => new Promise((r) => setTimeout(r, ms));

/* ── Render the list ────────────────────────────────────────── */

function taskRow(task, isFresh) {
  const li = $('#taskTemplate').content.firstElementChild.cloneNode(true);
  li.dataset.id = task.id;

  const done = task.status === 'done';
  const tentative = Number(task.confidence) < PENCIL_BELOW;
  const overdue = task.deadline && !done && new Date(task.deadline) < new Date();

  if (isFresh) {
    li.classList.add(reduceMotion ? 'enter' : 'pending');
    li.style.setProperty('--speed', `${WRITE_MS}ms`);
  }
  if (tentative) li.classList.add('pencil');
  if (done) li.classList.add('done');
  if (overdue) li.classList.add('overdue');

  const tick = li.querySelector('.tick');
  tick.setAttribute('aria-pressed', String(done));
  tick.setAttribute('aria-label', done ? `Mark "${task.title}" as not done` : `Mark "${task.title}" as done`);

  const titleWrap = li.querySelector('.title-wrap');
  titleWrap.classList.add('editable');
  titleWrap.tabIndex = 0;
  titleWrap.setAttribute('role', 'button');
  titleWrap.setAttribute('aria-label', `Edit the title of "${task.title}"`);
  titleWrap.querySelector('.title-text').textContent = task.title || '(untitled)';

  const meta = li.querySelector('.meta');

  const prio = document.createElement('span');
  prio.className = 'prio' + (task.priority === 0 ? ' critical' : '');
  prio.title = `Priority P${3 - Math.min(3, Number(task.priority) || 0)}`;
  const filled = 4 - Math.min(3, Number(task.priority) || 0);
  for (let i = 0; i < 4; i++) {
    const dot = document.createElement('i');
    if (i >= 4 - filled) dot.classList.add('on');
    prio.appendChild(dot);
  }
  meta.appendChild(prio);

  const dur = document.createElement('span');
  dur.className = 'chip dur editable';
  dur.dataset.field = 'duration_min';
  dur.tabIndex = 0;
  dur.title = 'Click to change how long this takes';
  dur.textContent = fmtDuration(task.duration_min);
  meta.appendChild(dur);

  if (task.deadline) {
    const due = document.createElement('span');
    due.className = 'chip due editable' + (overdue ? ' overdue' : '');
    due.dataset.field = 'deadline';
    due.tabIndex = 0;
    due.title = 'Click to change the deadline — clear it to remove';
    due.textContent = fmtDeadline(task.deadline);
    meta.appendChild(due);
  } else {
    const due = document.createElement('span');
    due.className = 'chip due editable';
    due.dataset.field = 'deadline';
    due.tabIndex = 0;
    due.textContent = 'no deadline';
    due.style.opacity = '.55';
    due.title = 'Click to add a deadline';
    meta.appendChild(due);
  }

  if (task.label) {
    const label = document.createElement('span');
    label.className = 'chip';
    label.textContent = task.label;
    meta.appendChild(label);
  }

  // the "text" / "voice" source chip is dropped: it was noise, and the space
  // goes to the title and the deadline instead (#2 on the task list)
  li.querySelector('.kill').setAttribute('aria-label', `Delete "${task.title}"`);

  return li;
}

/* Everything a row shows. A row whose signature has not changed is kept as
   the same DOM node, so adding tasks never replays old ticks or strikes. */
function rowSig(t) {
  return [t.title, t.status, t.deadline, t.duration_min, t.priority, t.label, t.confidence, t.position].join('|');
}

const orderOf = (t) => (Number.isFinite(t.position) ? t.position : Infinity);

function renderList() {
  const ordered = [...state.tasks].sort((a, b) => {
    if ((a.status === 'done') !== (b.status === 'done')) return a.status === 'done' ? 1 : -1;
    if (orderOf(a) !== orderOf(b)) return orderOf(a) - orderOf(b);
    const ad = a.deadline ? new Date(a.deadline).getTime() : Infinity;
    const bd = b.deadline ? new Date(b.deadline).getTime() : Infinity;
    if (ad !== bd) return ad - bd;
    return a.priority - b.priority;
  });

  const old = new Map([...list.children].map((li) => [li.dataset.id, li]));
  ordered.forEach((task, i) => {
    const sig = rowSig(task);
    let li = old.get(task.id);
    old.delete(task.id);
    if (!li || li.dataset.sig !== sig) {
      const fresh = state.freshIds.has(task.id) && !li;
      const next = taskRow(task, fresh);
      if (!fresh) next.classList.add('still');
      next.dataset.sig = sig;
      if (li) li.replaceWith(next);
      li = next;
    }
    if (list.children[i] !== li) list.insertBefore(li, list.children[i] || null);
  });
  old.forEach((li) => li.remove());

  const open = ordered.filter((t) => t.status !== 'done').length;
  const pencilled = ordered.filter((t) => t.status !== 'done' && Number(t.confidence) < PENCIL_BELOW).length;

  emptyNote.hidden = ordered.length > 0;
  if (!ordered.length) {
    summary.textContent = 'Nothing on the page yet.';
  } else if (!open) {
    summary.textContent = `${ordered.length} done. An empty page.`;
  } else {
    summary.textContent = `${open} open${pencilled ? ` · ${pencilled} pencilled in` : ''}`;
  }
}

async function announceNewTasks(alsoDone = []) {
  const fresh = [...state.freshIds];
  if (!fresh.length && !alsoDone.length) return;

  // this owns the pen for a while: drop any pending idle timer so it cannot
  // yank the pen home halfway through the flight below
  clearTimeout(penIdleTimer);
  penIdleTimer = null;
  returnToList();
  measurePenHome();
  mood('writing');

  for (const id of fresh) {
    const row = list.querySelector(`.task[data-id="${CSS.escape(id)}"]`);
    if (!row) continue;
    if (reduceMotion) { row.classList.add('enter'); continue; }
    const text = row.querySelector('.title-text');
    let r = text.getBoundingClientRect();
    penAt(r.left, r.top + r.height / 2);
    await wait(460);                        // the flight to the line
    r = text.getBoundingClientRect();
    row.classList.add('enter');             // the reveal starts under the nib
    mascotTrack.style.transition = `transform ${WRITE_MS}ms linear`;
    penAt(r.right, r.top + r.height / 2);
    await wait(WRITE_MS);
    mascotTrack.style.removeProperty('transition');
  }
  list.querySelectorAll('.task.pending:not(.enter)').forEach((row) => row.classList.add('enter'));

  // "I finished the lab report" gets the same trip, with a tick
  for (const id of alsoDone) {
    const row = list.querySelector(`.task[data-id="${CSS.escape(id)}"]`);
    penFlyTo(row ? row.querySelector('.tick') : null);
    await wait(reduceMotion ? 30 : WRITE_MS);
  }

  // fly home, then settle
  penGoHomeQuiet();
  await wait(reduceMotion ? 30 : 420);
  mood('idle');
}

/* ── Render the planner ─────────────────────────────────────── */

function renderPlanner() {
  const weekStart = state.weekStart;
  const today = new Date();

  weekLabel.textContent = weekStart.toLocaleDateString([], { month: 'short', day: 'numeric' }) +
    ' – ' + addDays(weekStart, 6).toLocaleDateString([], { month: 'short', day: 'numeric' });

  gutter.textContent = '';
  const track = document.createElement('div');
  track.className = 'gutter-track';
  for (let h = DAY_START; h <= DAY_END; h++) {
    const label = document.createElement('span');
    label.style.top = `${((h - DAY_START) / (DAY_END - DAY_START)) * 100}%`;
    label.textContent = h === 12 ? '12' : ((h + 11) % 12) + 1;
    if (h === DAY_END) label.classList.add('last');   // sits above the edge, not through it
    track.appendChild(label);
  }
  gutter.appendChild(track);

  gridDays.textContent = '';
  for (let i = 0; i < 7; i++) {
    const date = addDays(weekStart, i);
    const day = document.createElement('div');
    day.className = 'day';
    if (isSameDay(date, today)) day.classList.add('is-today');
    if (state.focusDate && isSameDay(date, new Date(state.focusDate + 'T00:00:00'))) {
      day.classList.add('is-focus');
    }

    const head = document.createElement('div');
    head.className = 'day-head';
    head.innerHTML =
      `<span class="dow">${date.toLocaleDateString([], { weekday: 'short' })}</span>` +
      `<span class="dom">${date.getDate()}</span>`;
    day.appendChild(head);

    const body = document.createElement('div');
    body.className = 'day-body';
    body.dataset.day = date.toDateString();

    // Gather the day's blocks first, then lay them out in lanes: anything
    // that overlaps in time shares the column instead of stacking.
    const entries = [];
    for (const block of state.busy) {
      const start = new Date(block.start);
      if (!isSameDay(start, date)) continue;
      entries.push({ start: block.start, end: block.end, title: block.title, kind: 'busy' });
    }
    for (const placement of state.placements) {
      const start = new Date(placement.start);
      if (!isSameDay(start, date)) continue;
      const task = state.byId.get(placement.task_id);
      entries.push({
        start: placement.start,
        end: placement.end,
        title: task ? task.title : 'task',
        kind: 'place',
        taskId: placement.task_id,
        pinned: !!(task && task.pinned_start),
        label: task ? task.label : 'Other',
        done: task && task.status === 'done',
        pencil: task && Number(task.confidence) < PENCIL_BELOW,
      });
    }
    entries.sort((a, b) => a.start.localeCompare(b.start) || a.end.localeCompare(b.end));

    entries.forEach((entry) => {
      body.appendChild(paint(
        entry.start, entry.end, entry.title, entry.kind,
        entry.label, entry.done, entry.pencil, entry.taskId, entry.pinned,
      ));
    });
    day.appendChild(body);
    gridDays.appendChild(day);
    // Must run after the day is in the document: lanes are measured from the
    // painted geometry, and offsetTop is 0 on a detached element.
    layOutLanes(body);
  }

  const placed = state.busy.length + state.placements.length;
  const stuck = state.unscheduled
    .map((entry) => ({ entry, task: state.byId.get(entry.task_id) }))
    .filter((item) => item.task);

  plannerEmpty.hidden = placed > 0 && stuck.length === 0;
  plannerEmpty.textContent = stuck.length
    ? 'No room on the page for: ' + stuck.map((i) => `${i.task.title} (${i.entry.reason.replace(/_/g, ' ')})`).join(', ') + '.'
    : 'Nothing is placed yet. Say a few tasks and the pen will find the gaps.';

  legend.textContent = '';
  const used = [...new Set(state.tasks.map((t) => t.label).filter(Boolean))].sort();
  if (!used.length) used.push('Other');
  for (const label of used) {
    const item = document.createElement('li');
    const swatch = document.createElement('i');
    swatch.style.background = LABELS[label] || LABELS.Other;
    item.appendChild(swatch);
    item.appendChild(document.createTextNode(label));
    legend.appendChild(item);
  }
}

/* Give blocks that visually overlap their own lane. Measures what was painted,
   so a block's minimum height counts as the overlap it really is. */
function layOutLanes(body) {
  const blocks = [...body.children].sort(
    (a, b) => a.offsetTop - b.offsetTop || a.offsetHeight - b.offsetHeight,
  );
  if (!blocks.length) return;

  let cluster = [];
  let clusterBottom = null;

  const flush = () => {
    if (!cluster.length) return;
    const laneBottoms = [];
    for (const el of cluster) {
      let lane = laneBottoms.findIndex((bottom) => bottom <= el.offsetTop);
      if (lane === -1) { lane = laneBottoms.length; laneBottoms.push(0); }
      laneBottoms[lane] = el.offsetTop + el.offsetHeight;
      el.dataset.laneIndex = String(lane);
    }
    const total = laneBottoms.length;
    for (const el of cluster) {
      if (total === 1) { delete el.dataset.laneIndex; continue; }
      const width = 100 / total;
      const lane = Number(el.dataset.laneIndex);
      el.style.left = `calc(${lane * width}% + 1px)`;
      el.style.right = 'auto';
      el.style.width = `calc(${width}% - 2px)`;
      el.dataset.lane = `${lane + 1}/${total}`;
    }
    cluster = [];
    clusterBottom = null;
  };

  for (const el of blocks) {
    if (clusterBottom !== null && el.offsetTop >= clusterBottom) flush();
    cluster.push(el);
    const bottom = el.offsetTop + el.offsetHeight;
    clusterBottom = clusterBottom === null || bottom > clusterBottom ? bottom : clusterBottom;
  }
  flush();
}

function paint(startIso, endIso, title, kind, label, done, pencil, taskId, pinned) {
  const el = document.createElement('div');
  const spanMinutes = Math.max(0, minutesIntoDay(endIso) - minutesIntoDay(startIso));
  const top = ((minutesIntoDay(startIso) - DAY_START * 60) / ((DAY_END - DAY_START) * 60)) * 100;

  el.className = `block ${kind}` + (done ? ' done' : '') + (pencil ? ' pencil' : '');
  el.style.top = `${Math.max(0, top)}%`;
  el.style.height = `max(16px, ${(spanMinutes / ((DAY_END - DAY_START) * 60)) * 100}%)`;
  if (kind === 'place') el.style.setProperty('--block-color', LABELS[label] || LABELS.Other);

  const full = `${title} · ${fmtTime(startIso)}–${fmtTime(endIso)}`;
  el.title = full;                       // hover, and the a11y name
  el.setAttribute('role', 'button');
  el.setAttribute('tabindex', '0');
  el.setAttribute('aria-label', full);
  if (taskId) el.dataset.taskId = taskId;
  el.addEventListener('click', () => {
    if (el.dataset.dragged) { delete el.dataset.dragged; return; }
    showBlock(full);
  });
  el.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); showBlock(full); }
  });

  const name = document.createElement('span');
  name.className = 'block-title';
  name.textContent = title;
  el.appendChild(name);

  if (pinned) {
    const pin = document.createElement('button');
    pin.type = 'button';
    pin.className = 'pin';
    pin.textContent = '📌';
    pin.title = 'Pinned here. Tap to unpin';
    pin.setAttribute('aria-label', `Unpin "${title}"`);
    el.appendChild(pin);
  }

  // a 30-minute block is only two text lines tall — keep it to the title
  if (spanMinutes >= 45) {
    const time = document.createElement('span');
    time.className = 'block-time';
    time.textContent = fmtRange(startIso, endIso);
    el.appendChild(time);
  }

  return el;
}

/* Tapping a block shows the whole title; a column is too narrow for it. */
function showBlock(text) {
  const note = $('#marginNote');
  const text_ = $('#marginText');
  if (!note || !text_) return;
  text_.textContent = text;
  note.hidden = false;
}

async function refreshCalendar() {
  const from = state.weekStart;
  const to = addDays(from, 7);
  let data = { busy: [], placements: [] };
  try {
    data = await getCalendar(from.toISOString(), to.toISOString());
  } catch { /* leave the page as it was rather than blanking the planner */ }
  state.busy = data.busy || [];
  state.placements = data.placements || [];
  renderPlanner();
  return data;
}

/* ── Data refresh ───────────────────────────────────────────── */

function indexTasks() {
  state.byId = new Map(state.tasks.map((t) => [t.id, t]));
}

async function loadTasks(fresh = []) {
  const data = await getTasks();
  state.tasks = data.tasks || [];
  indexTasks();
  state.freshIds = new Set(fresh);
  renderList();
  renderPlanner();
  await announceNewTasks();
  state.freshIds.clear();
}

async function replan() {
  try {
    const data = await runSchedule();
    state.placements = data.placements || [];
    state.unscheduled = data.unscheduled || [];
  } catch { /* server will re-plan on the next message */ }
  await refreshCalendar();
}

/* ── Speech out ─────────────────────────────────────────────── */

function speak(text) {
  if (!state.speaking || !text || !('speechSynthesis' in window)) return;
  const first = String(text).match(/[^.!?]*[.!?]?/g)?.filter(Boolean).slice(0, 2).join(' ') || text;
  window.speechSynthesis.cancel();
  const utterance = new SpeechSynthesisUtterance(first);
  utterance.rate = 1.04;
  utterance.pitch = 1.0;
  window.speechSynthesis.speak(utterance);
}

/* ── Loading state ──────────────────────────────────────────── */

let busyCount = 0;
function setBusy(on) {
  busyCount = Math.max(0, busyCount + (on ? 1 : -1));
  state.working = busyCount > 0;
  document.body.classList.toggle('is-busy', state.working);
  $('#writingNote').hidden = !state.working;
  scheduleBtn.disabled = state.working;
  // while the pen is away writing, take no new input from the user
  penBtn.disabled = state.working;
  box.readOnly = state.working;
}

/* ── Send a message ─────────────────────────────────────────── */

async function submit(text, source = 'text') {
  const body = text.trim();
  if (!body || state.working) return;

  setBusy(true);
  mood('writing');

  try {
    const result = await sendMessage(body, source);
    const before = new Map(state.tasks.map((t) => [t.id, t.status]));
    state.tasks = result.tasks || [];
    indexTasks();
    if (result.focus_date) {
      state.focusDate = result.focus_date;
      state.weekStart = startOfDay(new Date(result.focus_date + 'T00:00:00'));
    }
    state.placements = result.placements || [];

    state.freshIds = new Set(state.tasks.filter((t) => !before.has(t.id)).map((t) => t.id));
    const newlyDone = state.tasks
      .filter((t) => t.status === 'done' && before.has(t.id) && before.get(t.id) !== 'done')
      .map((t) => t.id);
    renderList();
    renderPlanner();

    marginText.textContent = result.reply || '';
    marginNote.hidden = !result.reply;
    speak(result.reply);
    clearToast();

    await announceNewTasks(newlyDone);
    state.freshIds.clear();
    await refreshCalendar();
  } catch (err) {
    penGoHomeQuiet();
    say('The server did not answer. Is it running?');
    console.error(err);
  } finally {
    setBusy(false);
    mood('idle');
  }
}

/* ── Push to talk: the pen is the mic ───────────────────────── */

const SpeechRec = window.SpeechRecognition || window.webkitSpeechRecognition;
let recognition = null;
let listening = false;
let heardSomething = false;
let startGuard = null;
/* true when this press produced speech, so the click that follows a
   press-and-hold does not also fire an empty send */
let spokeThisPress = false;

function setupRecognition() {
  if (!SpeechRec) {
    penBtn.dataset.noSpeech = 'true';
    penBtn.title = 'This browser has no speech recognition. Tap to send, type instead.';
    return;   // the composer hint was removed; the title carries this now
  }
  recognition = new SpeechRec();
  recognition.continuous = true;
  recognition.interimResults = true;
  recognition.lang = navigator.language || 'en-US';

  const baseText = () => (box.dataset.base || '');

  recognition.onstart = () => {
    clearTimeout(startGuard);
    startGuard = null;
    listening = true;
    heardSomething = false;
    mood('listening');
    box.dataset.base = box.value;
  };

  recognition.onresult = (event) => {
    let interim = '';
    let final = '';
    for (let i = event.resultIndex; i < event.results.length; i++) {
      const chunk = event.results[i];
      if (chunk.isFinal) final += chunk[0].transcript;
      else interim += chunk[0].transcript;
    }
    if (final) heardSomething = true;
    const base = baseText();
    box.value = (base ? base.replace(/\s+$/, '') + ' ' : '') + final + interim;
    autoGrow();
    // writing into the box by hand does not raise `input`, so call the pen
    // directly: it should be seen following along with what you are saying
    penWrites();
  };

  recognition.onerror = (event) => {
    if (event.error === 'not-allowed' || event.error === 'service-not-allowed') {
      say('Microphone permission was refused.');
    } else if (event.error !== 'aborted' && event.error !== 'no-speech') {
      say('Speech recognition hiccup: ' + event.error);
    }
  };

  recognition.onend = () => {
    clearTimeout(startGuard);
    startGuard = null;
    if (!listening) { mood('idle'); return; }
    listening = false;
    const spoken = box.value.trim();
    if (heardSomething && spoken) {
      spokeThisPress = true;
      box.value = '';
      delete box.dataset.base;
      penGoHome();            // spoken and sent: same as tapping send
      submit(spoken, 'voice');
    } else {
      box.value = baseText();
      delete box.dataset.base;
      autoGrow();
      penGoHome();            // nothing usable was said; un-write the box
      mood('idle');
    }
  };
}

function startListening() {
  if (!recognition || listening) return;
  // React to the press straight away. Some browsers advertise speech support
  // but never fire `start` (no device, permission still pending), so don't hang.
  mood('listening');
  startGuard = setTimeout(() => {
    startGuard = null;
    if (!listening) {
      mood('idle');
      say('The microphone did not start. Check the permission?', 4000);
    }
  }, 2000);
  try {
    recognition.start();
  } catch {
    clearTimeout(startGuard);
    startGuard = null;
    mood('idle');
  }
}

function stopListening() {
  if (!recognition) return;
  clearTimeout(startGuard);
  startGuard = null;
  if (!listening) return;
  try { recognition.stop(); } catch { /* not running */ }
}

/* The pen: press and hold to talk, tap to send. */
penBtn.addEventListener('pointerdown', (event) => {
  event.preventDefault();
  // capture keeps the press alive if the finger slides off the nib; never let
  // it throw and swallow the start of the recording
  try { penBtn.setPointerCapture?.(event.pointerId); } catch { /* fine without it */ }
  spokeThisPress = false;
  startListening();
});
['pointerup', 'pointercancel'].forEach((evt) =>
  penBtn.addEventListener(evt, () => stopListening()));

penBtn.addEventListener('click', () => {
  // a press-and-hold that produced speech already sent it
  if (spokeThisPress) { spokeThisPress = false; return; }
  if (state.working) return;
  const text = box.value;
  if (!text.trim()) return;
  box.value = '';
  delete box.dataset.base;
  autoGrow();
  submit(text, 'text');
});

/* hold the space bar */
let spaceHeld = false;
document.addEventListener('keydown', (event) => {
  const typingElsewhere = /^(INPUT|TEXTAREA)$/.test(document.activeElement?.tagName || '');
  if (event.code === 'Space' && !spaceHeld && !typingElsewhere) {
    spaceHeld = true;
    event.preventDefault();
    startListening();
  }
});
document.addEventListener('keyup', (event) => {
  if (event.code === 'Space' && spaceHeld) {
    spaceHeld = false;
    event.preventDefault();
    stopListening();
  }
});

/* ── Text box ───────────────────────────────────────────────── */

function autoGrow() {
  box.style.height = 'auto';
  box.style.height = Math.min(160, box.scrollHeight) + 'px';
}

/* Typing, dictation and voice all end up in the same box, so one input
   listener covers all three. The pen writes, then goes home by itself. */
box.addEventListener('input', () => {
  autoGrow();
  if (box.value.trim()) penWrites();
  else penGoHome();
});
box.addEventListener('blur', () => { if (!state.working) penGoHome(); });
box.addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault();
    const text = box.value;
    box.value = '';
    box.dataset.base = '';
    autoGrow();
    penGoHome();              // sent: nothing left to write
    submit(text, 'text');
  }
});

/* ── Inline editing (title, deadline, duration) ─────────────── */

const FIELD_LABEL = { title: 'Title', deadline: 'Deadline', duration_min: 'Length' };

function beginEdit(li, field) {
  if (state.editing) return;
  const task = state.byId.get(li.dataset.id);
  if (!task) return;

  let host, input, read;
  if (field === 'title') {
    host = li.querySelector('.title-wrap');
    input = document.createElement('input');
    input.className = 'inline-input title-input';
    input.value = task.title || '';
    input.setAttribute('aria-label', 'Task title');
    read = () => input.value.trim();
  } else if (field === 'deadline') {
    host = li.querySelector('.chip.due');
    if (!host) return;
    input = document.createElement('input');
    input.type = 'datetime-local';
    input.className = 'inline-input date-input';
    input.value = task.deadline ? toLocalInput(task.deadline) : '';
    input.setAttribute('aria-label', 'Deadline');
    read = () => (input.value ? new Date(input.value).toISOString() : null);
  } else if (field === 'duration_min') {
    host = li.querySelector('.chip.dur');
    if (!host) return;
    input = document.createElement('input');
    input.type = 'number'; input.min = '5'; input.step = '15';
    input.className = 'inline-input num-input';
    input.value = task.duration_min ?? 30;
    input.setAttribute('aria-label', 'Minutes');
    read = () => {
      const n = Number(input.value);
      return n >= 5 ? Math.round(n) : null;
    };
  }
  if (!host) return;

  state.editing = true;
  delete li.dataset.sig;
  li.classList.add('editing');
  host.replaceWith(input);
  input.focus();
  input.select?.();

  let closed = false;
  const finish = async (save) => {
    if (closed) return;
    closed = true;
    const value = read();
    li.classList.remove('editing');
    state.editing = false;

    const current = task[field];
    // for a deadline, an emptied field means "remove the deadline"
    const blank = value === null && field !== 'deadline';
    if (!save || blank || value === current || (field === 'title' && !value)) {
      renderList();
      return;
    }

    try {
      const updated = await patchTask(task.id, { [field]: value });
      Object.assign(task, updated);
      indexTasks();
      renderList();
      renderPlanner();
      await replan();
      say(`${FIELD_LABEL[field]} updated.`);
    } catch {
      say('Could not save that change.');
      renderList();
    }
  };

  input.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') { event.preventDefault(); finish(true); }
    if (event.key === 'Escape') { event.preventDefault(); finish(false); }
  });
  input.addEventListener('blur', () => finish(true));
}

/* ── List interactions ──────────────────────────────────────── */

list.addEventListener('click', async (event) => {
  const li = event.target.closest('.task');
  if (!li) return;
  const id = li.dataset.id;
  const task = state.byId.get(id);
  if (!task) return;

  if (event.target.closest('.kill')) {
    li.classList.add('leaving');
    try {
      await deleteTask(id);
      state.tasks = state.tasks.filter((t) => t.id !== id);
      indexTasks();
      setTimeout(() => { renderList(); renderPlanner(); }, 240);
      await replan();
    } catch { li.classList.remove('leaving'); say('Could not delete that one.'); }
    return;
  }

  if (event.target.closest('.tick')) {
    const done = task.status !== 'done';
    li.classList.remove('still');
    li.querySelector('.tick').setAttribute('aria-pressed', String(done));
    li.classList.toggle('done', done);
    task.status = done ? 'done' : 'todo';
    li.dataset.sig = rowSig(task);
    if (done && !reduceMotion) {
      measurePenHome();
      penFlyTo(li.querySelector('.tick'));
      setTimeout(penGoHomeQuiet, 520);
    }
    try {
      await patchTask(id, { status: task.status });
      await replan();
    } catch {
      say('Could not save that.');
    }
    renderList();
    renderPlanner();
    return;
  }

  if (state.editing) return;
  const field = event.target.closest('.chip.editable')?.dataset.field;
  if (field) { beginEdit(li, field); return; }
  if (event.target.closest('.title-wrap')) beginEdit(li, 'title');
});

/* keyboard route into the inline editor */
list.addEventListener('keydown', (event) => {
  if (event.key !== 'Enter' && event.key !== ' ') return;
  const holder = event.target.closest('.title-wrap, .chip.editable');
  if (!holder || state.editing) return;
  event.preventDefault();
  const li = holder.closest('.task');
  beginEdit(li, holder.dataset.field || 'title');
});

/* ── Add one task by hand ───────────────────────────────────── */

const addForm = $('#addForm');
const addBox  = $('#addBox');

addForm.addEventListener('submit', async (event) => {
  event.preventDefault();
  const title = addBox.value.trim();
  if (!title || state.working) return;

  addBox.value = '';
  setBusy(true);
  try {
    const created = await addTask(title);
    await loadTasks(created.id ? [created.id] : []);
    speak(`${created.title || title}. Added.`);
  } catch (err) {
    say('Could not add that one.');
    console.error(err);
  } finally {
    setBusy(false);
  }
});

/* ── Import a calendar (.ics) ───────────────────────────────── */

const icsFile = $('#icsFile');
const importBtn = document.querySelector('.import-btn');

const clashNote = $('#clashNote');
let clashFile = null;

async function uploadIcs(file, confirm = false) {
  const form = new FormData();
  form.append('file', file, file.name);
  if (confirm) form.append('confirm', 'true');
  const res = await fetch(API + '/api/calendar/import', { method: 'POST', body: form });
  if (res.status === 409) return { clash: await res.json().catch(() => ({})) };
  if (!res.ok) {
    const error = new Error(`POST /api/calendar/import -> ${res.status}`);
    error.status = res.status;
    throw error;
  }
  return { data: await res.json() };
}

function describeClash(c) {
  // server shape: {start, end, imported_title, existing_title}
  const title = c.imported_title || c.title || 'event';
  const when = c.start ? ` (${fmtDeadline(c.start)})` : '';
  return `${title}${when}${c.existing_title ? ` overlaps ${c.existing_title}` : ''}`;
}

function showClashes(file, body) {
  clashFile = file;
  $('#clashTitle').textContent = body.error || 'Some of these events overlap your calendar.';
  const ul = $('#clashList');
  ul.textContent = '';
  (body.conflicts || []).slice(0, 12).forEach((c) => {
    const li = document.createElement('li');
    li.textContent = typeof c === 'string' ? c : describeClash(c);
    ul.appendChild(li);
  });
  clashNote.hidden = false;
  $('#clashYes').focus();
}

async function importCalendar(file, confirm = false) {
  if (!file || state.working) return;
  setBusy(true);
  importBtn.classList.add('is-busy');
  try {
    const { data, clash } = await uploadIcs(file, confirm);
    if (clash) { showClashes(file, clash); return; }
    clashNote.hidden = true;
    // the route replaces the busy blocks; re-plan so the tasks move out of the way
    await runSchedule();
    await refreshCalendar();
    const n = (data && data.busy ? data.busy : state.busy).length;
    say(n ? `Imported ${n} event${n === 1 ? '' : 's'} from ${file.name}.`
          : `No events found in ${file.name}.`);
  } catch (err) {
    // A file the planner cannot read is the user's to fix, not a defect.
    const badFile = err.status === 400 || err.status === 422;
    if (!badFile) console.error(err);
    say(badFile ? 'That file is not a calendar the planner can read.' : 'Could not import that calendar.');
  } finally {
    importBtn.classList.remove('is-busy');
    setBusy(false);
  }
}

icsFile.addEventListener('change', () => {
  const file = icsFile.files && icsFile.files[0];
  icsFile.value = '';                       // let the same file be picked twice
  importCalendar(file);
});
$('#clashYes').addEventListener('click', () => {
  clashNote.hidden = true;
  importCalendar(clashFile, true);
});
$('#clashNo').addEventListener('click', () => { clashNote.hidden = true; clashFile = null; });

/* ── Import an email / PDF: its tasks are written like a ramble ── */

const docFile = $('#docFile');
docFile.addEventListener('change', async () => {
  const file = docFile.files && docFile.files[0];
  docFile.value = '';
  if (!file || state.working) return;
  const label = document.querySelector('label[for="docFile"]');
  setBusy(true);
  label.classList.add('is-busy');
  mood('writing');
  try {
    const before = new Set(state.tasks.map((t) => t.id));
    const form = new FormData();
    form.append('file', file, file.name);
    const res = await fetch(API + '/api/capture', { method: 'POST', body: form });
    if (!res.ok) {
      const error = new Error(`POST /api/capture -> ${res.status}`);
      error.status = res.status;
      throw error;
    }
    const data = await res.json();
    const fresh = (data.tasks || []).map((t) => t.id).filter((id) => !before.has(id));
    await loadTasks(fresh);
    await replan();
    say(fresh.length ? `Found ${fresh.length} task${fresh.length === 1 ? '' : 's'} in ${file.name}.`
                     : `No tasks found in ${file.name}.`);
  } catch (err) {
    if (!err.status || err.status >= 500) console.error(err);
    say(err.status && err.status < 500 ? 'The server could not read that file yet.' : 'Could not import that file.');
  } finally {
    label.classList.remove('is-busy');
    setBusy(false);
    mood('idle');
  }
});

/* ── Reorder the list: drag a row by its grip ──────────────── */

async function saveOrder() {
  const jobs = [];
  [...list.children].forEach((row, i) => {
    const task = state.byId.get(row.dataset.id);
    if (!task || task.position === i) return;
    task.position = i;
    row.dataset.sig = rowSig(task);
    jobs.push(patchTask(task.id, { position: i }).catch(() => null));
  });
  if (!jobs.length) return;
  await Promise.all(jobs);
  await replan();
}

list.addEventListener('pointerdown', (event) => {
  const grip = event.target.closest('.grip');
  if (!grip || state.working || state.editing) return;
  const li = grip.closest('.task');
  event.preventDefault();
  try { grip.setPointerCapture(event.pointerId); } catch { /* fine */ }
  li.classList.add('dragging');
  const move = (ev) => {
    const over = [...list.children].find((row) => {
      if (row === li) return false;
      const b = row.getBoundingClientRect();
      return ev.clientY >= b.top && ev.clientY <= b.bottom;
    });
    if (!over) return;
    const b = over.getBoundingClientRect();
    list.insertBefore(li, ev.clientY < b.top + b.height / 2 ? over : over.nextSibling);
  };
  const up = () => {
    grip.removeEventListener('pointermove', move);
    grip.removeEventListener('pointerup', up);
    grip.removeEventListener('pointercancel', up);
    li.classList.remove('dragging');
    saveOrder();
  };
  grip.addEventListener('pointermove', move);
  grip.addEventListener('pointerup', up);
  grip.addEventListener('pointercancel', up);
});

/* ── Reorder the planner: drag a note to a slot, tap the pin to free it ── */

async function pinTask(id, iso) {
  const task = state.byId.get(id);
  try {
    const updated = await patchTask(id, { pinned_start: iso });
    if (task) Object.assign(task, updated || {}, { pinned_start: iso });
  } catch { say('Could not move that one.'); }
  await replan();
}

gridDays.addEventListener('pointerdown', (event) => {
  const pin = event.target.closest('.pin');
  if (pin) {
    event.preventDefault();
    const id = pin.closest('.block').dataset.taskId;
    if (id && !state.working) pinTask(id, null);
    return;
  }
  const el = event.target.closest('.block.place');
  if (!el || !el.dataset.taskId || state.working) return;
  const sx = event.clientX, sy = event.clientY;
  let moved = false;
  try { el.setPointerCapture(event.pointerId); } catch { /* fine */ }
  const move = (ev) => {
    const dx = ev.clientX - sx, dy = ev.clientY - sy;
    if (!moved && Math.hypot(dx, dy) < 5) return;
    moved = true;
    el.classList.add('dragging');
    el.style.transform = `translate(${dx}px, ${dy}px)`;
  };
  const up = (ev) => {
    el.removeEventListener('pointermove', move);
    el.removeEventListener('pointerup', up);
    el.removeEventListener('pointercancel', up);
    if (!moved) return;
    el.dataset.dragged = '1';
    const top = el.getBoundingClientRect().top;
    el.classList.remove('dragging');
    el.style.transform = '';
    const body = [...gridDays.querySelectorAll('.day-body')].find((b) => {
      const r = b.getBoundingClientRect();
      return ev.clientX >= r.left && ev.clientX <= r.right;
    });
    if (!body) return;
    const r = body.getBoundingClientRect();
    const span = (DAY_END - DAY_START) * 60;
    let minutes = DAY_START * 60 + ((top - r.top) / r.height) * span;
    minutes = Math.round(minutes / 15) * 15;
    minutes = Math.max(DAY_START * 60, Math.min(DAY_END * 60 - 15, minutes));
    const start = new Date(body.dataset.day);
    start.setHours(0, minutes, 0, 0);
    pinTask(el.dataset.taskId, start.toISOString());
  };
  el.addEventListener('pointermove', move);
  el.addEventListener('pointerup', up);
  el.addEventListener('pointercancel', up);
});

/* ── Schedule it ────────────────────────────────────────────── */

const scheduleBtn = $('#scheduleBtn');
scheduleBtn.addEventListener('click', async () => {
  if (state.working) return;
  setBusy(true);
  mood('writing');
  try {
    await runSchedule();
    await refreshCalendar();
    const count = state.placements.length;
    say(count ? `Placed ${count} block${count === 1 ? '' : 's'} on the page.` : 'Nothing left to place.');
  } catch (err) {
    say('The schedule did not come back.');
    console.error(err);
  } finally {
    setBusy(false);
    mood('idle');
  }
});

$('#marginReplay').addEventListener('click', () => speak(marginText.textContent));

/* ── Header tools ───────────────────────────────────────────── */

const gearBtn = $('#gearBtn');
const settingsPop = $('#settingsPop');
function setSettings(open) {
  settingsPop.hidden = !open;
  gearBtn.setAttribute('aria-expanded', String(open));
}
gearBtn.addEventListener('click', (event) => {
  event.stopPropagation();
  setSettings(settingsPop.hidden);
});
document.addEventListener('click', (event) => {
  if (!settingsPop.hidden && !settingsPop.contains(event.target)) setSettings(false);
});
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && !settingsPop.hidden) { setSettings(false); gearBtn.focus(); }
});

const speakToggle = $('#speakToggle');
speakToggle.addEventListener('click', () => {
  state.speaking = !state.speaking;
  speakToggle.setAttribute('aria-pressed', String(state.speaking));
  if (!state.speaking) window.speechSynthesis?.cancel();
});

const themeToggle = $('#themeToggle');
themeToggle.addEventListener('click', () => {
  const night = document.documentElement.dataset.theme === 'night';
  document.documentElement.dataset.theme = night ? 'day' : 'night';
  themeToggle.querySelector('.tool-glyph').textContent = night ? '🌙' : '☀️';
  localStorage.setItem('ai-todo-theme', document.documentElement.dataset.theme);
});

/* ── Phone pages: To do | Planner ─────────────────────────────── */

/* At phone width the list and the planner are two snap pages side by side.
   The scrolling itself is CSS scroll-snap; this only keeps the tab strip
   honest and offers a tappable way across. */
const desk = $('#desk');
const pageTabs = $('#pageTabs');
const pageDots = [...document.querySelectorAll('.page-dots i')];

function showPage(index) {
  const page = desk.children[index];
  if (!page) return;
  desk.scrollTo({ left: page.offsetLeft, behavior: 'smooth' });
}

function markPage(index) {
  const tabs = pageTabs.querySelectorAll('.page-tab');
  tabs.forEach((tab, i) => {
    const on = i === index;
    tab.classList.toggle('is-on', on);
    if (on) tab.setAttribute('aria-current', 'page');
    else tab.removeAttribute('aria-current');
  });
  pageDots.forEach((dot, i) => dot.classList.toggle('on', i === index));
}

pageTabs.addEventListener('click', (event) => {
  const tab = event.target.closest('.page-tab');
  if (tab) showPage(Number(tab.dataset.page));
});

desk.addEventListener('scroll', () => {
  const width = desk.clientWidth || 1;
  markPage(Math.round(desk.scrollLeft / width) === 0 ? 0 : 1);
}, { passive: true });

/* New tasks have to be written on the list, so come back to it.
   `paged` is true only in the phone layout, where the two are side by side. */
function returnToList() {
  if (desk.scrollWidth > desk.clientWidth + 4) showPage(0);
}

$('#weekPrev').addEventListener('click', async () => {
  state.weekStart = addDays(state.weekStart, -7);
  await refreshCalendar();
});
$('#weekNext').addEventListener('click', async () => {
  state.weekStart = addDays(state.weekStart, 7);
  await refreshCalendar();
});
weekLabel.addEventListener('click', async () => {
  state.weekStart = startOfDay(new Date());
  state.focusDate = null;
  await refreshCalendar();
});

/* ── Boot ───────────────────────────────────────────────────── */

(function boot() {
  const savedTheme = localStorage.getItem('ai-todo-theme');
  if (savedTheme === 'night') {
    document.documentElement.dataset.theme = 'night';
    themeToggle.querySelector('.tool-glyph').textContent = '☀️';
  }

  setupRecognition();
  autoGrow();
  mood('idle');

  loadTasks()
    .then(replan)
    .catch(() => {
      emptyNote.textContent = 'The server is not answering yet. Start it with: cargo run -p server';
      summary.textContent = 'Offline';
    });
})();