# Deploy Rambl

## One-time server setup

Create `C:\Rambl`, copy `server.exe` there, and copy `.env.example` to `.env`.
Fill in `TYPESAFE_API_KEY`; keep `PORT=3160` and
`RAMBL_DEFAULT_TZ=America/New_York`. Leave `GOOGLE_CLIENT_ID` empty until Google
sign-in is set up. Tasks are stored in `C:\Rambl\data\`, which updates never touch. Create a shortcut to `C:\Rambl\server.exe` in
`shell:startup`. The firewall, router, and Cloudflare forwarding are already
configured.

## Every release

On the development machine, run `deploy\build-release.ps1`. Close the old
server window, rename `C:\Rambl\server.exe` to
`C:\Rambl\server.old.exe`, send the new `server.exe` from
`deploy\dist\rambl-release` over RustDesk, and double-click it. Open
<https://rambl.burvy.dev> to verify the deployment.

## Rollback

Close the server window, remove the failed `server.exe`, rename
`server.old.exe` back to `server.exe`, and double-click it.

## Troubleshooting

- If the window exits, run `server.exe` from PowerShell to read the error.
- If the page is unavailable, check that port 3160 is free and the server
  window is still open.
- If sign-in or Jev fails, check `.env` values without committing or sharing
  them.
