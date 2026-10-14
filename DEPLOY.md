# Deploy Rambl

Every server is one exe and one settings file, staged the same way.
`deploy\build-release.ps1` (or `.\build.ps1 -Deploy -Servers rambl` in burvy-dev)
builds `rambl-server.exe` and writes `rambl.env.example` to `deploy\dist\rambl-release`.

## One-time server setup

Create a folder for Rambl (for example `C:\Rambl`) and copy `rambl-server.exe`
and `rambl.env.example` there. Copy `rambl.env.example` to `rambl.env` and fill in:

- `TYPESAFE_API_KEY`, the Jev key.
- `RAMBL_PORT=3160`, unless another port is wanted.
- `RAMBL_BIND`, optional, and normally left empty. The server listens on `127.0.0.1`
  only, which is enough because the Cloudflare tunnel runs on this machine. Loopback
  keeps the server off the LAN and stops Windows asking for firewall access every time
  a new build path runs the exe. Set it to `0.0.0.0` to serve the rest of the LAN as
  well, or to one address to serve only that one. A value that is not an IP address
  stops the server with an error naming the variable and the value, rather than quietly
  falling back.
- If you do widen the bind, set `RAMBL_TRUSTED_PROXIES` to cloudflared's own address in
  the same edit. The tunnel will no longer be arriving over loopback, and a peer's
  `CF-Connecting-IP` is only read from a trusted proxy. List the tunnel and nothing
  else: a client's address or network in that list would let that client name itself
  and walk past the per-address limits and blocks, which is the one thing the list
  exists to prevent.
- `RAMBL_DEFAULT_TZ=America/New_York`.
- `RAMBL_SIGNINS_PER_IP`, optional, default `60`. Google sign-in attempts one client
  address may make per day; past it the server answers 429. Raise it only for a shared
  address such as an office network.
- `GOOGLE_CLIENT_ID=138340425604-qagdhrfblj66i5s9i9icq07lb32sd2g2.apps.googleusercontent.com`,
  the public Google OAuth web client (its authorized origins are https://rambl.burvy.dev
  and http://localhost:3160). Leave it empty to turn sign-in off.
- `RAMBL_LOCAL_LOGIN=1`, **test sites only, never in production.** Shows "Continue locally"
  in the settings panel, which signs in as one fixed local user with no Google account.
  Only a peer on this machine or the private LAN (10/8, 172.16/12, 192.168/16, `::1`,
  `fc00::/7`) may use it, and never a request carrying `CF-Connecting-IP` or
  `X-Forwarded-For`, so the Cloudflare tunnel cannot reach it. The server refuses to start
  with it while `RAMBL_TRUSTED_PROXIES` is set, and prints a warning at startup.
- `RAMBL_ADMIN_EMAILS=you@example.com,someone@example.com`, optional. Comma-separated
  list of the emails allowed to open the `/admin` page (it shows today's usage and can
  block users or addresses and change plans). Leave it empty to turn the page off.
- `RAMBL_TRUSTED_PROXIES`, optional, and normally unset. Cloudflare's
  `CF-Connecting-IP` header is only believed when the connection itself came from a
  trusted proxy: loopback always is, because the Cloudflare tunnel runs on this
  machine, and this variable adds any other proxy as a comma-separated list of
  addresses or networks (`10.0.0.1, 10.0.0.0/8`). From any other peer the server uses
  the socket address and ignores the header, so someone reaching the server directly
  cannot name their own address and walk past the per-address limits and blocks. An
  entry that is not a real address or network is ignored.

`rambl.env` is read only from the exe's own folder, and real environment
variables override it. Tasks are stored in `data\rambl.db` next to the exe;
updates never touch that folder's `data\`. Backups are in `data\backups`.
Create a shortcut to `rambl-server.exe` in `shell:startup`. The firewall and Cloudflare tunnel are already configured for port 3160.

The window prints the address it actually bound, for example `Rambl is listening on
127.0.0.1:3160 — open http://127.0.0.1:3160`. Anything other than `127.0.0.1` there
means `RAMBL_BIND` is set, and the site will only reach the tunnel if that address
includes the one cloudflared connects from.

Old installs are obsolete: the previous `C:\Rambl` build and `data\ai-todo.db`
can be deleted once the new server runs and the page loads.

## Every release

Follow [docs/SHIP.md](docs/SHIP.md): it is the one ordered procedure (build from a
clean `origin/main` checkout, smoke the exe, send it over RustDesk, swap, smoke the
site, roll back). The checklist below holds the settings and the checks it refers to.
`deploy\build-release.ps1` stages the same exe and `rambl.env.example` in
`deploy\dist\rambl-release` when you build from this repo alone, without burvy-dev.

The notes for what changed, and the PR behind each item, are in
[docs/RELEASE-NOTES.md](docs/RELEASE-NOTES.md).

## Deploy checklist

1. **Build.** On the development machine, `deploy\build-release.ps1`, or from
   burvy-dev the full command:

   ```powershell
   .uild.ps1 -Deploy -Modules primer-wasm -Servers rambl,primer
   ```

   Either leaves `rambl-server.exe` and `rambl.env.example` in
   `deploy\dist\rambl-release`. Run it with PowerShell 7 (`pwsh`); on Windows
   PowerShell 5.1 the script stops on the build output that trunk writes to stderr.

2. **New settings.** In the server's `rambl.env`:

   - `RAMBL_ADMIN_EMAILS=you@example.com`, a comma-separated list of the emails
     allowed to open `/admin`. Unset or empty answers 404 to everyone, which is the
     reason an admin page that "disappeared" is almost always this line.
   - `RAMBL_TRUSTED_PROXIES=`, left empty. cloudflared runs on the same machine and
     reaches the server over loopback, which is trusted without being listed. Only fill
     it in if a proxy runs on another host, as a comma-separated list of addresses or
     networks (`10.0.0.1, 10.0.0.0/8`). Entries that are not a real address or network
     are ignored.
   - `RAMBL_BIND=`, left empty. The server listens on `127.0.0.1`, which is all the
     tunnel on this machine needs; that keeps the server off the LAN. Set `0.0.0.0`
     only if the server should also serve the LAN, and set `RAMBL_TRUSTED_PROXIES` to
     cloudflared's own address in the same edit: the tunnel will no longer arrive over
     loopback, and a peer's `CF-Connecting-IP` is only read from a trusted proxy. Never
     list a client's address or network there — that would let the client name itself.
     A value that is not an IP address stops the server instead of falling back.

3. **Before swapping the exe**, copy `C:\Rambl\data\rambl.db` to `rambl.db.<date>`.
   The file only moves forward: the new exe applies the migrations it has not seen.
   The startup backup in `data\backups` is written after the migration, so it is not a
   pre-upgrade copy.

4. **Swap and start.** Close the server window, rename `rambl-server.exe` to
   `rambl-server.old.exe`, copy the new one in, double-click it. The window prints
   `Rambl is listening on 127.0.0.1:3160 — open http://127.0.0.1:3160`, or whatever
   `RAMBL_BIND` asked for. If it closes instead, run the exe from PowerShell to read
   the error; a refused `RAMBL_BIND` is one reason to do that.

5. **Smoke it** from the development machine (change the port if you are testing
   locally):

   ```powershell
   curl.exe -s -o NUL -w "%{http_code}`n" https://rambl.burvy.dev/
   curl.exe -s -o NUL -w "%{http_code}`n" https://rambl.burvy.dev/api/v1/config
   curl.exe -s -o NUL -w "%{http_code}`n" https://rambl.burvy.dev/api/v1/snapshot
   curl.exe -s -o NUL -w "%{http_code}`n" https://rambl.burvy.dev/admin
   ```

   | Check | Expected |
   |---|---|
   | `/` | `200` and a page titled Rambl |
   | `/api/v1/config` | `200`, `{"google_client_id":null}` on a server with sign-in off |
   | `/api/v1/snapshot` | `200` with a plan window |
   | `/admin` | `404` with `RAMBL_ADMIN_EMAILS` empty, `200` with it set |
   | `C:\Rambl\data\rambl.db` | exists, and grows with use |
   | `C:\Rambl\data\backups` | one `rambl-<date>.db` from startup, one more each day |

6. **Check the admin page shows real visitor addresses.** See below.

7. **Rollback** if any check fails: see [Rollback](#rollback).

## Checking that the admin page shows real visitor addresses

The admin page's usage list shows the address each capture came from. It is only
meaningful if Cloudflare's `CF-Connecting-IP` is being read, and that happens only when
the request arrived through the tunnel.

1. Capture one ramble from a phone on mobile data, not on the office wifi, so the
   request can only have come through Cloudflare.
2. Open <https://rambl.burvy.dev/admin> and sign in with an email in
   `RAMBL_ADMIN_EMAILS`. Today's usage lists captures by address.
3. The row for that capture should be your phone's public address (the mobile
   carrier's range), not `127.0.0.1` and not the machine's `192.168.x.x`.

| What you see | What it means | What to do |
|---|---|---|
| The phone's public address | The tunnel is forwarding the header and the server believes it. Nothing to set. | none |
| `127.0.0.1` | The request came from the tunnel over loopback but the header was not used, or the capture was made against `localhost` or the LAN rather than through Cloudflare. | Confirm the capture came from `https://rambl.burvy.dev` on mobile data, not from `http://localhost:3160`. If it did, check the exe is the current build, since this is what #168 changed. |
| `192.168.x.x` or another LAN address | The request bypassed the tunnel and came straight to the server. | Only possible if `RAMBL_BIND` was widened. Set it back to empty (loopback) unless you meant to serve the LAN. |
| No row for that capture at all | Nothing was captured, or the day has rolled over. | Capture again, then check `/api/v1/snapshot` from the same account. |

If the tunnel ever moves to another machine, put that machine's address in
`RAMBL_TRUSTED_PROXIES` and restart the server. Nothing else changes.

## Rollback

Close the server window, remove the failed `rambl-server.exe`, rename
`rambl-server.old.exe` back to `rambl-server.exe`, and double-click it.

The database file only moves forward, so two cases matter:

- **An exe from this release or later**: it opens the migrated file, changes nothing,
  and does not read the columns it does not know. The site works; a deadline set by the
  new build reads in the browser's zone until you roll forward again.
- **An exe older than the anonymous-expiry release (#146)**: it re-ran every migration
  on open, so it will refuse the migrated file and stop. Put back the
  `rambl.db.<date>` copy taken in step 3, then start it again.

## Troubleshooting

- If the window exits, run `rambl-server.exe` from PowerShell to read the error. A
  refused `RAMBL_BIND` is one of the few reasons to do this.
- If the page is unavailable, check that port 3160 is free and the server
  window is still open.
- If the page works on `http://localhost:3160` but not through Cloudflare, the server
  is bound somewhere the tunnel cannot reach: check the address in the window against
  `RAMBL_BIND`.
- If `/admin` answers 404, check `RAMBL_ADMIN_EMAILS` in `rambl.env`, that it is not
  empty, that the signed-in email is in it, and that the server was restarted after
  the edit.
- If `/admin` shows `127.0.0.1` for a capture made through the tunnel, see
  [Checking that the admin page shows real visitor addresses](#checking-that-the-admin-page-shows-real-visitor-addresses).
- If sign-in or Jev fails, check the values in `rambl.env` without committing or
  sharing them.
