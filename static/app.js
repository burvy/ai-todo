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
const micBtn     = $('#mic');
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
  weekStart: startOfWeek(new Date()),
  focusDate: null,
  speaking: true,
  freshIds: new Set(),
  working: false,   // a request is in flight
  editing: false,   // an inline editor is open
};

const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

/* ── Tiny helpers ───────────────────────────────────────────── */

function startOfWeek(date) {
  const d = new Date(date);
  d.setHours(0, 0, 0, 0);
  const shift = (d.getDay() + 6) % 7;   // Monday = 0
  d.setDate(d.getDate() - shift);
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

/* ── Mascot ─────────────────────────────────────────────────── */

function mood(next) {
  mascotTrack.dataset.mood = next;
}

function penHome() {
  mascotTrack.style.removeProperty('--pen-x');
  mascotTrack.style.removeProperty('--pen-y');
}

function penWalkTo(el) {
  if (!el || reduceMotion) return;
  const home = mascotTrack.getBoundingClientRect();
  const box  = el.getBoundingClientRect();
  const x = (box.left + Math.min(60, box.width * 0.4)) - home.left;
  const y = (box.top + box.height / 2) - 34 - home.top;
  mascotTrack.style.setProperty('--pen-x', `${Math.round(x)}px`);
  mascotTrack.style.setProperty('--pen-y', `${Math.round(y)}px`);
}

const wait = (ms) => new Promise((r) => setTimeout(r, ms));

/* ── Render the list ────────────────────────────────────────── */

function taskRow(task, isFresh) {
  const li = $('#taskTemplate').content.firstElementChild.cloneNode(true);
  li.dataset.id = task.id;

  const done = task.status === 'done';
  const tentative = Number(task.confidence) < PENCIL_BELOW;
  const overdue = task.deadline && !done && new Date(task.deadline) < new Date();

  if (isFresh) li.classList.add('enter');
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

  if (task.source) {
    const src = document.createElement('span');
    src.className = 'chip';
    src.textContent = task.source;
    src.style.opacity = '.6';
    meta.appendChild(src);
  }

  li.querySelector('.kill').setAttribute('aria-label', `Delete "${task.title}"`);

  return li;
}

function renderList() {
  const ordered = [...state.tasks].sort((a, b) => {
    if ((a.status === 'done') !== (b.status === 'done')) return a.status === 'done' ? 1 : -1;
    const ad = a.deadline ? new Date(a.deadline).getTime() : Infinity;
    const bd = b.deadline ? new Date(b.deadline).getTime() : Infinity;
    if (ad !== bd) return ad - bd;
    return a.priority - b.priority;
  });

  list.textContent = '';
  ordered.forEach((task) => list.appendChild(taskRow(task, state.freshIds.has(task.id))));

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

async function announceNewTasks() {
  const fresh = [...state.freshIds];
  if (!fresh.length) return;
  mood('writing');

  for (const id of fresh) {
    const row = list.querySelector(`.task[data-id="${CSS.escape(id)}"]`);
    if (row) penWalkTo(row.querySelector('.title-wrap'));
    await wait(reduceMotion ? 30 : WRITE_MS);
  }

  mood('idle');
  setTimeout(penHome, 420);
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

    for (const block of state.busy) {
      const start = new Date(block.start);
      if (!isSameDay(start, date)) continue;
      body.appendChild(paint(block.start, block.end, block.title, 'busy'));
    }

    for (const placement of state.placements) {
      const start = new Date(placement.start);
      if (!isSameDay(start, date)) continue;
      const task = state.byId.get(placement.task_id);
      const title = task ? task.title : 'task';
      body.appendChild(paint(
        placement.start, placement.end, title, 'place',
        task ? task.label : 'Other',
        task && task.status === 'done',
        task && Number(task.confidence) < PENCIL_BELOW,
      ));
    }

    day.appendChild(body);
    gridDays.appendChild(day);
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

function paint(startIso, endIso, title, kind, label, done, pencil) {
  const el = document.createElement('div');
  const spanMinutes = Math.max(0, minutesIntoDay(endIso) - minutesIntoDay(startIso));
  const top = ((minutesIntoDay(startIso) - DAY_START * 60) / ((DAY_END - DAY_START) * 60)) * 100;

  el.className = `block ${kind}` + (done ? ' done' : '') + (pencil ? ' pencil' : '');
  el.style.top = `${Math.max(0, top)}%`;
  el.style.height = `max(16px, ${(spanMinutes / ((DAY_END - DAY_START) * 60)) * 100}%)`;
  if (kind === 'place') el.style.setProperty('--block-color', LABELS[label] || LABELS.Other);
  el.title = `${title} · ${fmtTime(startIso)}–${fmtTime(endIso)}`;

  const name = document.createElement('span');
  name.className = 'block-title';
  name.textContent = title;
  el.appendChild(name);

  // a 30-minute block is only two text lines tall — keep it to the title
  if (spanMinutes >= 45) {
    const time = document.createElement('span');
    time.className = 'block-time';
    time.textContent = fmtRange(startIso, endIso);
    el.appendChild(time);
  }

  return el;
}

async function refreshCalendar() {
  const from = state.weekStart;
  const to = addDays(from, 7);
  try {
    const data = await getCalendar(from.toISOString(), to.toISOString());
    state.busy = data.busy || [];
    state.placements = data.placements || [];
  } catch {
    state.busy = [];
    state.placements = [];
  }
  renderPlanner();
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
  if (SpeechRec) micBtn.disabled = state.working;
}

/* ── Send a message ─────────────────────────────────────────── */

async function submit(text, source = 'text') {
  const body = text.trim();
  if (!body || state.working) return;

  setBusy(true);
  mood('writing');

  try {
    const result = await sendMessage(body, source);
    const before = new Set(state.byId.keys());
    state.tasks = result.tasks || [];
    indexTasks();
    if (result.focus_date) {
      state.focusDate = result.focus_date;
      state.weekStart = startOfWeek(new Date(result.focus_date + 'T00:00:00'));
    }
    state.placements = result.placements || [];

    state.freshIds = new Set(state.tasks.filter((t) => !before.has(t.id)).map((t) => t.id));
    renderList();
    renderPlanner();

    marginText.textContent = result.reply || '';
    marginNote.hidden = !result.reply;
    speak(result.reply);
    clearToast();

    await announceNewTasks();
    state.freshIds.clear();
    await refreshCalendar();
  } catch (err) {
    penHome();
    say('The server did not answer. Is it running?');
    console.error(err);
  } finally {
    setBusy(false);
    mood('idle');
  }
}

/* ── Push to talk ───────────────────────────────────────────── */

const SpeechRec = window.SpeechRecognition || window.webkitSpeechRecognition;
let recognition = null;
let listening = false;
let heardSomething = false;

function setupRecognition() {
  if (!SpeechRec) {
    micBtn.disabled = true;
    micBtn.title = 'This browser has no speech recognition. Type instead.';
    $('#composerHint').textContent = 'Enter sends · this browser has no speech recognition';
    return;
  }
  recognition = new SpeechRec();
  recognition.continuous = true;
  recognition.interimResults = true;
  recognition.lang = navigator.language || 'en-US';

  const baseText = () => (box.dataset.base || '');

  recognition.onstart = () => {
    listening = true;
    heardSomething = false;
    micBtn.setAttribute('aria-pressed', 'true');
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
  };

  recognition.onerror = (event) => {
    if (event.error === 'not-allowed' || event.error === 'service-not-allowed') {
      say('Microphone permission was refused.');
    } else if (event.error !== 'aborted' && event.error !== 'no-speech') {
      say('Speech recognition hiccup: ' + event.error);
    }
  };

  recognition.onend = () => {
    if (!listening) return;
    listening = false;
    micBtn.setAttribute('aria-pressed', 'false');
    mood('idle');
    const spoken = box.value.trim();
    if (heardSomething && spoken) {
      box.value = '';
      submit(spoken, 'voice');
    } else {
      box.value = baseText();
      autoGrow();
    }
  };
}

function startListening() {
  if (!recognition || listening) return;
  try { recognition.start(); } catch { /* already started */ }
}

function stopListening() {
  if (!recognition || !listening) return;
  try { recognition.stop(); } catch { /* not running */ }
}

/* hold the mic button */
['pointerdown'].forEach((evt) =>
  micBtn.addEventListener(evt, (event) => {
    event.preventDefault();
    micBtn.setPointerCapture?.(event.pointerId);
    startListening();
  }));
['pointerup', 'pointercancel'].forEach((evt) =>
  micBtn.addEventListener(evt, () => stopListening()));

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

box.addEventListener('input', autoGrow);
box.addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault();
    const text = box.value;
    box.value = '';
    box.dataset.base = '';
    autoGrow();
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
    li.querySelector('.tick').setAttribute('aria-pressed', String(done));
    li.classList.toggle('done', done);
    task.status = done ? 'done' : 'todo';
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

$('#weekPrev').addEventListener('click', async () => {
  state.weekStart = addDays(state.weekStart, -7);
  await refreshCalendar();
});
$('#weekNext').addEventListener('click', async () => {
  state.weekStart = addDays(state.weekStart, 7);
  await refreshCalendar();
});
weekLabel.addEventListener('click', async () => {
  state.weekStart = startOfWeek(new Date());
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