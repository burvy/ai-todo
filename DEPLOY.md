# Deploy Rambl

Every server is one exe and one settings file, staged the same way.
`deploy\build-release.ps1` (or `.\build.ps1 -Deploy -Servers rambl` in burvy-dev)
builds `rambl-server.exe` and writes `rambl.env.example` to `deploy\dist\rambl-release`.

## One-time server setup

Create a folder for Rambl (for example `C:\Rambl`) and copy `rambl-server.exe`
and `rambl.env.example` there. Copy `rambl.env.example` to `rambl.env` and fill in:

- `TYPESAFE_API_KEY`, the Jev key.
- `RAMBL_PORT=3160`, unless another port is wanted.
- `RAMBL_DEFAULT_TZ=America/New_York`.
- `GOOGLE_CLIENT_ID=138340425604-qagdhrfblj66i5s9i9icq07lb32sd2g2.apps.googleusercontent.com`,
  the public Google OAuth web client (its authorized origins are https://rambl.burvy.dev
  and http://localhost:3160). Leave it empty to turn sign-in off.
- `RAMBL_ADMIN_EMAILS=you@example.com,someone@example.com`, optional. Comma-separated
  list of the emails allowed to open the `/admin` page (it shows today's usage and can
  block users or addresses and change plans). Leave it empty to turn the page off.

`rambl.env` is read only from the exe's own folder, and real environment
variables override it. Tasks are stored in `data\rambl.db` next to the exe;
updates never touch that folder's `data\`. Backups are in `data\backups`.
Create a shortcut to `rambl-server.exe` in `shell:startup`. The firewall, router
and Cloudflare forwarding are already configured for port 3160.

Old installs are obsolete: the previous `C:\Rambl` build and `data\ai-todo.db`
can be deleted once the new server runs and the page loads.

## Every release

On the development machine, run `deploy\build-release.ps1`. Close the old server
window, rename `rambl-server.exe` in the Rambl folder to `rambl-server.old.exe`,
send the new `rambl-server.exe` from `deploy\dist\rambl-release` over RustDesk, and
double-click it. Open <https://rambl.burvy.dev> to verify the deployment.

## Rollback

Close the server window, remove the failed `rambl-server.exe`, rename
`rambl-server.old.exe` back to `rambl-server.exe`, and double-click it.

## Troubleshooting

- If the window exits, run `rambl-server.exe` from PowerShell to read the error.
- If the page is unavailable, check that port 3160 is free and the server
  window is still open.
- If sign-in or Jev fails, check the values in `rambl.env` without committing or
  sharing them.
