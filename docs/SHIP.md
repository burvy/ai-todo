# Ship Rambl

The exact steps to put a new `rambl-server.exe` on the server machine over RustDesk,
keeping the old exe for rollback. Settings, the smoke table and troubleshooting are in
[DEPLOY.md](../DEPLOY.md); this page is the order to do things in.

The server folder is `C:\Rambl` below. Use whatever folder the server already runs from.

## 1. Build on the development machine

Build from a clean checkout of `origin/main`, never from a working checkout: the build
takes whatever the folder holds, branch and uncommitted edits included.

```powershell
cd "Z:\3. Rust Projects\ai-todo"
git fetch origin
git worktree add ..\ai-todo-release origin/main --detach   # first time only
cd ..\ai-todo-release
git checkout --detach origin/main                           # every later release
git status --short                                          # must print nothing

cd "..\burvy-dev"
git pull
.\build.ps1 -Deploy -Modules primer-wasm -Servers rambl,primer -RamblDir ..\ai-todo-release
```

Run it from a PowerShell 7 (`pwsh`) prompt. `-RamblDir` arrives with burvy-dev PR #3;
without it the build takes `..\ai-todo`, whatever branch that checkout is on. From cold
it takes about half an hour. It ends by opening `burvy-dev\deploy\`, which holds
`rambl-server.exe` with the web page compiled in, `primer-server.exe` and the site under
`burvy-dev\dist\`. Note the commit you built: `git -C ..\ai-todo-release log -1 --oneline`.

## 2. Check the exe starts from an empty folder

Before sending it anywhere, run it once on the development machine with a throwaway
settings file, on a port nothing else holds (another Rambl may already be on 3160):

```powershell
$l = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0); $l.Start()
$port = $l.LocalEndpoint.Port; $l.Stop()
$t = New-Item -ItemType Directory "$env:TEMP\rambl-smoke-$(Get-Date -f yyyyMMddHHmmss)"
Copy-Item "Z:\3. Rust Projects\burvy-dev\deploy\rambl-server.exe" $t
"RAMBL_PORT=$port`nRAMBL_DEFAULT_TZ=America/New_York" | Set-Content "$t\rambl.env"
$p = Start-Process "$t\rambl-server.exe" -WorkingDirectory $t -PassThru
Start-Sleep 3
curl.exe -s -o NUL -w "%{http_code}`n" http://127.0.0.1:$port/
curl.exe -s -o NUL -w "%{http_code}`n" http://127.0.0.1:$port/api/v1/config
Stop-Process -Id $p.Id
```

The exe's window must say `Rambl is listening on 127.0.0.1:<port>`, both lines must
print `200`, and `$t\data\rambl.db` must exist afterwards. A `200` alone proves nothing
if the port was already taken: something else answered. Delete `$t`.

## 3. Copy it over RustDesk

1. Open RustDesk and connect to the server machine.
2. Open **File Transfer** for that connection.
3. Left pane (this machine): go to `Z:\3. Rust Projects\burvy-dev\deploy\`.
4. Right pane (the server): go to `C:\Rambl\`, and create a folder `incoming` there if
   it does not exist.
5. Send `rambl-server.exe` into `C:\Rambl\incoming\`. Do not send it straight over the
   running exe: Windows refuses to replace an exe that is running, and a half-copied
   exe is worse than none.

## 4. Swap on the server (in the RustDesk remote session)

1. Back up the database while the server still runs: copy `C:\Rambl\data\rambl.db` to
   `C:\Rambl\data\rambl.db.<yyyy-mm-dd>`. The new exe migrates the file forward on
   start, and the startup backup in `data\backups` is taken after that migration, so it
   is not a pre-upgrade copy.
2. Close the Rambl server window.
3. If an old `rambl-server.old.exe` is still there from the last release, delete it.
4. Rename `C:\Rambl\rambl-server.exe` to `rambl-server.old.exe`. This is the rollback.
5. Move `C:\Rambl\incoming\rambl-server.exe` to `C:\Rambl\rambl-server.exe`.
6. Add any new setting the release notes name to `C:\Rambl\rambl.env`.
7. Double-click `rambl-server.exe`. The window prints
   `Rambl is listening on 127.0.0.1:3160 — open http://127.0.0.1:3160`.
   If it closes instead, run it from PowerShell in `C:\Rambl` to read the error.

The `shell:startup` shortcut points at `rambl-server.exe` by name, so it picks up the new
exe on the next reboot without changes.

## 5. Smoke it

From the development machine:

```powershell
curl.exe -s -o NUL -w "%{http_code}`n" https://rambl.burvy.dev/
curl.exe -s -o NUL -w "%{http_code}`n" https://rambl.burvy.dev/api/v1/config
curl.exe -s -o NUL -w "%{http_code}`n" https://rambl.burvy.dev/api/v1/snapshot
```

All three print `200`. Then open <https://rambl.burvy.dev> on a phone, capture one
ramble, and check it appears. The full table of checks is in
[DEPLOY.md](../DEPLOY.md#deploy-checklist).

## 6. Roll back if anything fails

In the RustDesk session:

1. Close the Rambl server window.
2. Rename `C:\Rambl\rambl-server.exe` to `rambl-server.failed.exe`.
3. Rename `C:\Rambl\rambl-server.old.exe` back to `rambl-server.exe`.
4. Double-click it.
5. If the old exe refuses the database (an exe older than #146 re-runs every
   migration and stops), close it, put the `data\rambl.db.<yyyy-mm-dd>` copy from step
   4.1 back as `data\rambl.db`, and start it again. Captures made since the swap are
   lost in that case.

Keep `rambl-server.old.exe` until the next release; it is the only rollback.
