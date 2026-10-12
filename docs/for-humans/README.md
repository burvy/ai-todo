# For humans

Everything only a person can do, collected in one checklist. Each line says what
to do and the exact path or link. Paths were checked against origin/main
(22c8f30); anything missing is marked **planned**.

## Art

- [ ] Redraw Rambl's poses by hand following `art/GUIDE.md`, replacing the placeholder SVGs in `art/illustrations/` and `web/assets/`.
- [ ] Add a "lobotomized" pose alongside the others in `art/illustrations/` (planned — no such file yet).
- [ ] Later, build the Rive animation file from the redrawn poses (planned — no `.rive` file exists yet).

## Rambl's voice

- [ ] Edit Rambl's reply lines in `crates/rambl-brain` (planned — the directory does not exist yet; its `README.md` will explain the files).

## Accounts and services

- [ ] Create the Google sign-in client (OAuth client ID) in Google Cloud: https://console.cloud.google.com/apis/credentials
- [ ] Turn on the Cloudflare Access lock in front of the site — temporary, until accounts ship: https://one.dash.cloudflare.com/
- [ ] Put the Jev key (`TYPESAFE_API_KEY`) in the server's `.env` — see Environment variables in [README.md](../../README.md) (gitignored: never on origin/main).

## Deploy

- [ ] Deploy per the [README Deploy section](../../README.md#deploy) (DEPLOY.md planned — not in the repo yet).

## Product decisions

- [ ] Work through the design review: https://claude.ai/code/artifact/a667d476-54d6-447b-8970-7accdea412fb
