# ai-todo

> Every scheduling tool asks you to do data entry. This one just listens.

ai-todo turns a spoken ramble or dropped syllabus into tasks, then places those
tasks into available calendar time. The demo uses seeded busy blocks instead of
Google Calendar.

## Run the demo

1. Copy the Jev API key into a local `.env` file:

   ```text
   TYPESAFE_API_KEY=your-key-here
   ```

   Keep this file local; it is ignored by Git.
2. Start the server:

   ```powershell
   cargo run -p server
   ```

   The server listens on port `8080` by default. Set `PORT` to use another
   port.
3. Open the web client and submit a voice transcript or one of the files in
   `demo/`. The sample spoken inputs are in `demo/rambles.md`; the sample
   syllabus is `demo/syllabus.pdf`.

## API

## Deploy

Run `deploy\build-release.ps1` to build and stage `dist\ai-todo\`. Copy the
`deploy` directory to the server, then run `deploy\install-task.ps1` as
Administrator to register the startup task and allow TCP port 3160. The task
uses `C:\burvy\secrets\ai-todo\jev-key.env` for the Jev key and runs with
`PORT=3160` and `TZ_NAME=America/New_York` by default.

- `POST /api/capture` - accept JSON `{"text":"..."}` or a multipart PDF
  `file`; return `{"tasks":[Task]}`.
- `GET /api/tasks` - return `{"tasks":[Task]}`.
- `PATCH /api/tasks/:id` - update and return one `Task`.
- `DELETE /api/tasks/:id` - delete a task; returns `204` or `404`.
- `POST /api/schedule` - return
  `{"placements":[{"task_id":"...","start":"...","end":"..."}],"unscheduled":[{"task_id":"...","reason":"..."}]}`.
- `GET /api/calendar?from=...&to=...` - return
  `{"busy":[{"start":"...","end":"...","title":"..."}],"placements":[...]}`.

Each `Task` has `id`, `title`, `notes`, `duration_min`, `deadline` (nullable),
`priority` (0-3), `status`, `confidence` (0-1), and `source`
(`voice`, `pdf`, or `text`).

> The scheduled task runs as the low-privilege `LocalService` account, not SYSTEM, because the server faces the internet. Grant it read access to the key and app folder once (admin PowerShell):
> `icacls C:\burvy\secrets\ai-todo /grant "NT AUTHORITY\LOCAL SERVICE:(OI)(CI)R"` and `icacls C:\burvy\ai-todo /grant "NT AUTHORITY\LOCAL SERVICE:(OI)(CI)RX"`
