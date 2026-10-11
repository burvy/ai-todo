/* ============================================================
   ai-todo — mobile sheet
   Talks to the same API as the desktop app. Every endpoint and
   field name here is exactly what docs/TEAM.md §3 and README
   specify, so this page keeps working when the backend lands.
   ============================================================ */

'use strict';

/* ── API ─────────────────────────────────────────────────────────
   One place to re-point. Leave empty for same origin (the server
   serves ./static). Set window.AI_TODO_API to call another host,
   e.g. 'https://ai-todo-api.burvy.dev'.                          */
const API = window.AI_TODO_API || '';

const PENCIL_BELOW = 0.6;    // confidence under this = "pencilled in"
const WRITE_MS = 600;         // how long the pen takes per title
const DAY_START = 8;
const DAY_END = 20;

const TAGS = {
  School:   'var(--tag-school)',
  Work:     'var(--tag-work)',
  Health:   'var(--tag-health)',
  Errands:  'var(--tag-errands)',
  Personal: 'var(--tag-personal)',
  Other:    'var(--tag-other)',
};

/* ── DOM ──────────────────────────────────────────────────── */

const $ = (s) => document.querySelector(s);
const list        = $('#taskList');
const emptyNote   = $('#emptyNote');
const box         = $('#messageBox');
const mascot      = $('#mascot');
const mascotHint  = $('#mascotHint');
const toast       = $('#toast');
const planner     = $('#planner');
const days        = $('#days');
const gutter      = $('#gutter');
const weekLabel   = $('#weekLabel');
const legend      = $('#legend');
const plannerEmpty = $('#plannerEmpty');

/* ── State ────────────────────────────────────────────────── */

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
  working: false,
  editing: false,
};

/* ── Helpers ──────────────────────────────────────────────── */

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
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}
function wait(ms) { return new Promise((r) => setTimeout(r, ms)); }

function fmtTime(iso) {
  return new Date(iso).toLocaleTimeString([], { hour: 'numeric' }).replace(/\s/g, '').toLowerCase();
}
/* compact range that fits a narrow day column: "9am – 10am" */
function fmtRange(startIso, endIso) {
  const s = new Date(startIso), e = new Date(endIso);
  const h12 = (h) => (h % 12 === 0 ? 12 : h % 12);
  const mer = (h) => (h < 12 ? 'am' : 'pm');
  const pad = (n) => String(n).padStart(2, '0');
  const a = s.getMinutes() ? `${h12(s.getHours())}:${pad(s.getMinutes())}${mer(s.getHours())}` : `${h12(s.getHours())}${mer(s.getHours())}`;
  const b = e.getMinutes() ? `${h12(e.getHours())}:${pad(e.getMinutes())}${mer(e.getHours())}` : `${h12(e.getHours())}${mer(e.getHours())}`;
  return `${a} – ${b}`;
}
function fmtDeadline(iso) {
  const d = new Date(iso);
  const today = new Date();
  if (isSameDay(d, today)) return `today ${fmtTime(iso)}`;
  if (isSameDay(d, addDays(today, 1))) return `tomorrow ${fmtTime(iso)}`;
  return `${d.toLocaleDateString([], { weekday: 'short' })} ${fmtTime(iso)}`;
}
function fmtDuration(min) {
  const m = Number(min) || 0;
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  const rest = m % 60;
  return rest ? `${h}h ${rest}m` : `${h}h`;
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
function say(message, ms = 3200) {
  toast.textContent = message;
  toast.hidden = !message;
  clearTimeout(toastTimer);
  if (message) toastTimer = setTimeout(() => { toast.hidden = true; }, ms);
}
function clearToast() {
  clearTimeout(toastTimer);
  toast.hidden = true;
}

/* ── The API layer — mirrors docs/TEAM.md §3 exactly ──────── */

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

const getTasks    = () => api('/api/tasks');                                            // → {tasks:[Task]}
const createTask  = (title) => api('/api/tasks', { method: 'POST', body: JSON.stringify({ title }) }); // → 201 Task
const patchTask   = (id, body) => api(`/api/tasks/${id}`, { method: 'PATCH', body: JSON.stringify(body) });
const deleteTask  = (id) => api(`/api/tasks/${id}`, { method: 'DELETE' });
const sendMessage = (text, source) => api('/api/message', { method: 'POST', body: JSON.stringify({ text, source }) });
const runSchedule = () => api('/api/schedule', { method: 'POST' });
const getCalendar = (from, to) => api(`/api/calendar?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`);

/* ── Mascot mood ──────────────────────────────────────────── */

function mood(next) {
  mascot.dataset.mood = next;
  mascot.setAttribute('aria-pressed', String(next === 'listening'));
  mascotHint.textContent =
    next === 'listening' ? 'listening…'
    : next === 'writing'  ? 'writing…'
    : 'hold the pen to talk';
}

/* ── Rendering ────────────────────────────────────────────── */

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

  const wrap = li.querySelector('.title-wrap');
  wrap.classList.add('editable');
  wrap.setAttribute('aria-label', `Edit the title of "${task.title}"`);
  wrap.querySelector('.title-text').textContent = task.title || '(untitled)';

  const meta = li.querySelector('.meta');

  const dur = document.createElement('span');
  dur.className = 'chip dur editable';
  dur.dataset.field = 'duration_min';
  dur.tabIndex = 0;
  dur.textContent = fmtDuration(task.duration_min);
  meta.appendChild(dur);

  const due = document.createElement('span');
  due.className = 'chip due editable' + (overdue ? ' overdue' : '');
  due.dataset.field = 'deadline';
  due.tabIndex = 0;
  due.textContent = task.deadline ? fmtDeadline(task.deadline) : 'no deadline';
  if (!task.deadline) due.style.opacity = '.55';
  meta.appendChild(due);

  if (task.label) {
    const tag = document.createElement('span');
    tag.className = 'chip tag';
    tag.style.setProperty('--tag-colour', TAGS[task.label] || TAGS.Other);
    const dot = document.createElement('i');
    tag.appendChild(dot);
    tag.appendChild(document.createTextNode(task.label));
    meta.appendChild(tag);
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

  emptyNote.hidden = ordered.length > 0;
  const open = ordered.filter((t) => t.status !== 'done').length;
  const pencilled = ordered.filter((t) => t.status !== 'done' && Number(t.confidence) < PENCIL_BELOW).length;
  list.setAttribute('aria-label',
    `${open} open${pencilled ? `, ${pencilled} pencilled in` : ''} of ${ordered.length} tasks`);
}

function indexTasks() { state.byId = new Map(state.tasks.map((t) => [t.id, t])); }

async function announceNewTasks() {
  const fresh = [...state.freshIds];
  if (!fresh.length) return;
  mood('writing');
  for (const id of fresh) {
    await wait(reduceMotion ? 30 : WRITE_MS);
  }
  mood('idle');
}

/* ── Planner ──────────────────────────────────────────────── */

function renderPlanner() {
  const weekStart = state.weekStart;
  const today = new Date();

  weekLabel.textContent =
    weekStart.toLocaleDateString([], { month: 'short', day: 'numeric' }) + ' – ' +
    addDays(weekStart, 6).toLocaleDateString([], { month: 'short', day: 'numeric' });

  gutter.textContent = '';
  const track = document.createElement('div');
  track.className = 'gutter-track';
  for (let h = DAY_START; h <= DAY_END; h++) {
    const label = document.createElement('span');
    label.style.top = `${((h - DAY_START) / (DAY_END - DAY_START)) * 100}%`;
    label.textContent = h === 12 ? '12' : ((h + 11) % 12) + 1;
    if (h === DAY_END) label.classList.add('last');
    track.appendChild(label);
  }
  gutter.appendChild(track);

  days.textContent = '';
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

    for (const block of state.busy) {
      if (!isSameDay(new Date(block.start), date)) continue;
      body.appendChild(paint(block.start, block.end, block.title, 'busy'));
    }
    for (const placement of state.placements) {
      if (!isSameDay(new Date(placement.start), date)) continue;
      const task = state.byId.get(placement.task_id);
      body.appendChild(paint(
        placement.start, placement.end,
        task ? task.title : 'task', 'place',
        task ? task.label : 'Other',
        task && task.status === 'done',
        task && Number(task.confidence) < PENCIL_BELOW,
      ));
    }

    day.appendChild(body);
    days.appendChild(day);
  }

  plannerEmpty.hidden = state.busy.length + state.placements.length > 0;

  legend.textContent = '';
  const used = [...new Set(state.tasks.map((t) => t.label).filter(Boolean))].sort();
  if (!used.length) used.push('Other');
  for (const label of used) {
    const item = document.createElement('li');
    const swatch = document.createElement('i');
    swatch.style.background = TAGS[label] || TAGS.Other;
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
  el.style.height = `max(14px, ${(spanMinutes / ((DAY_END - DAY_START) * 60)) * 100}%)`;
  if (kind === 'place') el.style.setProperty('--block-colour', TAGS[label] || TAGS.Other);
  el.title = `${title} · ${fmtTime(startIso)}–${fmtTime(endIso)}`;

  const name = document.createElement('span');
  name.className = 'block-title';
  name.textContent = title;
  el.appendChild(name);
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

async function replan() {
  try {
    const data = await runSchedule();
    state.placements = data.placements || [];
    state.unscheduled = data.unscheduled || [];
  } catch { /* the next message re-plans anyway */ }
  if (!planner.hidden) await refreshCalendar();
}

/* ── Data ─────────────────────────────────────────────────── */

async function loadTasks(fresh = []) {
  const data = await getTasks();
  state.tasks = data.tasks || [];
  indexTasks();
  state.freshIds = new Set(fresh);
  renderList();
  if (!planner.hidden) renderPlanner();
  await announceNewTasks();
  state.freshIds.clear();
}

/* ── Speech out ───────────────────────────────────────────── */

function speak(text) {
  if (!state.speaking || !text || !('speechSynthesis' in window)) return;
  const first = String(text).match(/[^.!?]*[.!?]?/g)?.filter(Boolean).slice(0, 2).join(' ') || text;
  window.speechSynthesis.cancel();
  const utterance = new SpeechSynthesisUtterance(first);
  utterance.rate = 1.04;
  window.speechSynthesis.speak(utterance);
}

/* ── Busy state ───────────────────────────────────────────── */

let busyCount = 0;
function setBusy(on) {
  busyCount = Math.max(0, busyCount + (on ? 1 : -1));
  state.working = busyCount > 0;
  $('#writingNote').hidden = !state.working;
  $('#scheduleBtn').disabled = state.working;
}

/* ── Send a message ───────────────────────────────────────── */

async function submit(text, source = 'text') {
  const body = text.trim();
  if (!body || state.working) return;

  setBusy(true);
  mood('writing');

  try {
    const before = new Set(state.byId.keys());
    const result = await sendMessage(body, source);

    state.tasks = result.tasks || [];
    indexTasks();
    if (result.focus_date) {
      state.focusDate = result.focus_date;
      state.weekStart = startOfDay(new Date(result.focus_date + 'T00:00:00'));
    }
    state.placements = result.placements || [];
    state.unscheduled = result.unscheduled || [];

    state.freshIds = new Set(state.tasks.filter((t) => !before.has(t.id)).map((t) => t.id));
    renderList();
    if (!planner.hidden) renderPlanner();

    speak(result.reply);
    clearToast();

    await announceNewTasks();
    state.freshIds.clear();
    if (!planner.hidden) await refreshCalendar();
  } catch (err) {
    say('The server did not answer. Is it running?');
    console.error(err);
  } finally {
    setBusy(false);
    mood('idle');
  }
}

/* ── Inline editing ───────────────────────────────────────── */

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
  } else {
    host = li.querySelector('.chip.dur');
    if (!host) return;
    input = document.createElement('input');
    input.type = 'number'; input.min = '5'; input.step = '15';
    input.className = 'inline-input num-input';
    input.value = task.duration_min ?? 30;
    input.setAttribute('aria-label', 'Minutes');
    read = () => { const n = Number(input.value); return n >= 5 ? Math.round(n) : null; };
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

    const blank = value === null && field !== 'deadline';
    if (!save || blank || value === task[field] || (field === 'title' && !value)) {
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

/* ── Task taps ────────────────────────────────────────────── */

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
    } catch {
      li.classList.remove('leaving');
      say('Could not delete that one.');
    }
    return;
  }

  if (event.target.closest('.tick')) {
    const done = task.status !== 'done';
    task.status = done ? 'done' : 'todo';
    try {
      await patchTask(id, { status: task.status });
      await replan();
    } catch {
      say('Could not save that.');
      task.status = done ? 'todo' : 'done';
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

list.addEventListener('keydown', (event) => {
  if (event.key !== 'Enter' && event.key !== ' ') return;
  const holder = event.target.closest('.title-wrap, .chip.editable');
  if (!holder || state.editing) return;
  event.preventDefault();
  beginEdit(holder.closest('.task'), holder.dataset.field || 'title');
});

/* ── Push to talk: the mascot is the microphone ───────────── */

const SpeechRec = window.SpeechRecognition || window.webkitSpeechRecognition;
let recognition = null;
let listening = false;
let heardSomething = false;
let startGuard = null;

function setupRecognition() {
  if (!SpeechRec) {
    mascotHint.textContent = 'no speech in this browser — type below';
    mascot.disabled = true;
    mascot.style.opacity = '.5';
    return;
  }
  recognition = new SpeechRec();
  recognition.continuous = true;
  recognition.interimResults = true;
  recognition.lang = navigator.language || 'en-US';

  const base = () => box.dataset.base || '';

  recognition.onstart = () => {
    clearTimeout(startGuard);
    startGuard = null;
    listening = true;
    heardSomething = false;
    box.dataset.base = box.value;
    mood('listening');
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
    const b = base();
    box.value = (b ? b.replace(/\s+$/, '') + ' ' : '') + final + interim;
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
    mascot.setAttribute('aria-pressed', 'false');
    const spoken = box.value.trim();
    if (heardSomething && spoken) {
      box.value = '';
      delete box.dataset.base;
      submit(spoken, 'voice');
    } else {
      box.value = base();
      delete box.dataset.base;
      mood('idle');
    }
  };
}

function startListening() {
  if (!recognition || listening) return;
  // React to the press straight away. Some browsers advertise speech support but
  // never fire `start` (no device, permission still pending), so don't hang.
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

/* the pen itself is the mic: press and hold it */
mascot.addEventListener('pointerdown', (event) => {
  event.preventDefault();
  // capture keeps the press alive if the finger slides off the nib; never let it
  // throw and swallow the start of the recording
  try { mascot.setPointerCapture?.(event.pointerId); } catch { /* fine without it */ }
  startListening();
});
['pointerup', 'pointercancel'].forEach((evt) =>
  mascot.addEventListener(evt, () => stopListening()));

/* or hold the space bar anywhere */
let spaceHeld = false;
document.addEventListener('keydown', (event) => {
  const typing = /^(INPUT|TEXTAREA)$/.test(document.activeElement?.tagName || '');
  if (event.code === 'Space' && !spaceHeld && !typing) {
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

/* ── The prompt ───────────────────────────────────────────── */

box.addEventListener('keydown', (event) => {
  if (event.key !== 'Enter') return;
  event.preventDefault();
  const text = box.value;
  box.value = '';
  submit(text, 'text');
});

$('#attach').addEventListener('click', () => {
  say('File capture is not wired up yet — talk or type instead.');
});

/* ── Schedule sheet ───────────────────────────────────────── */

function openPlanner() {
  planner.hidden = false;
  document.body.style.overflow = 'hidden';
  refreshCalendar();
}
function closePlanner() {
  planner.hidden = true;
  document.body.style.overflow = '';
}

$('#viewSchedule').addEventListener('click', openPlanner);
$('#plannerClose').addEventListener('click', closePlanner);
planner.addEventListener('click', (event) => {
  if (event.target === planner) closePlanner();
});
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && !planner.hidden) closePlanner();
});

$('#scheduleBtn').addEventListener('click', async () => {
  if (state.working) return;
  setBusy(true);
  mood('writing');
  try {
    await runSchedule();
    await refreshCalendar();
    const n = state.placements.length;
    say(n ? `Placed ${n} block${n === 1 ? '' : 's'}.` : 'Nothing left to place.');
  } catch (err) {
    say('The schedule did not come back.');
    console.error(err);
  } finally {
    setBusy(false);
    mood('idle');
  }
});

$('#weekPrev').addEventListener('click', () => {
  state.weekStart = addDays(state.weekStart, -7);
  refreshCalendar();
});
$('#weekNext').addEventListener('click', () => {
  state.weekStart = addDays(state.weekStart, 7);
  refreshCalendar();
});
weekLabel.addEventListener('click', () => {
  state.weekStart = startOfDay(new Date());
  state.focusDate = null;
  refreshCalendar();
});

/* ── Boot ─────────────────────────────────────────────────── */

const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

setupRecognition();
mood('idle');

getTasks()
  .then(async (data) => {
    state.tasks = data.tasks || [];
    indexTasks();
    renderList();
  })
  .catch(() => {
    emptyNote.hidden = false;
    emptyNote.querySelector('.empty-lead').textContent = 'The server is not answering.';
    emptyNote.querySelector('.empty-ramble').textContent = 'Start it with cargo run -p server';
  });