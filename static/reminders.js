(function () {
  const tools = document.querySelector('#masthead-tools, .masthead-tools');
  if (!tools) return;

  const button = document.createElement('a');
  button.href = '/api/calendar.ics';
  button.download = 'rambl.ics';
  button.textContent = 'Add to my calendar';
  button.style.cssText = 'background:none;border:0;color:inherit;cursor:pointer;font:inherit;text-decoration:underline;padding:.4rem .6rem;';
  tools.appendChild(button);

  const notifiedKey = 'ai-todo-notified-placements';
  const notified = new Set(JSON.parse(sessionStorage.getItem(notifiedKey) || '[]'));

  async function checkReminders() {
    if (!('Notification' in window) || Notification.permission !== 'granted') return;
    const [calendar, tasks] = await Promise.all([
      fetch('/api/calendar').then((response) => response.json()),
      fetch('/api/tasks').then((response) => response.json()),
    ]);
    const titles = new Map(tasks.tasks.map((task) => [task.id, task.title]));
    const now = Date.now();
    calendar.placements.forEach((placement) => {
      const start = new Date(placement.start).getTime();
      if (start < now || start > now + 5 * 60 * 1000) return;
      const id = `${placement.task_id}-${start}`;
      if (notified.has(id)) return;
      notified.add(id);
      sessionStorage.setItem(notifiedKey, JSON.stringify([...notified]));
      const title = titles.get(placement.task_id) || 'your next task';
      const at = new Date(start).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
      new Notification(`Up next: ${title} at ${at}`);
      if ('speechSynthesis' in window) window.speechSynthesis.speak(new SpeechSynthesisUtterance(`Up next: ${title} at ${at}`));
    });
  }

  button.addEventListener('click', () => {
    if ('Notification' in window && Notification.permission === 'default') Notification.requestPermission();
  });
  document.addEventListener('click', () => {
    if ('Notification' in window && Notification.permission === 'default') Notification.requestPermission();
  }, { once: true });
  setInterval(checkReminders, 30 * 1000);
})();
