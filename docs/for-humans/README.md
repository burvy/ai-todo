# For humans

Everything only a person can do, collected in one checklist. Each line says what
to do and the exact path or link. Paths were checked against origin/main
(22c8f30); anything missing is marked **planned**.

## Art

- [ ] Redraw Rambo's poses by hand following `art/GUIDE.md`, replacing the placeholder SVGs in `web/assets/rambo/`.
- [ ] Add a "lobotomized" pose alongside the others in `web/assets/rambo/`.
- [ ] Later, build the Rive animation file from the redrawn poses (planned — no `.rive` file exists yet).

## Rambo's voice

- [ ] Edit what Rambo says in `crates/rambl-brain/content/lines.toml` and how he behaves in `crates/rambl-brain/content/personality.toml`; `crates/rambl-brain/README.md` explains both. Preview a simulated day with `cargo run -p rambl-brain --example sim`.

## Accounts and services

- [ ] Create the Google sign-in client (OAuth client ID) in Google Cloud: https://console.cloud.google.com/apis/credentials
- [ ] Turn on the Cloudflare Access lock in front of the site — temporary, until accounts ship: https://one.dash.cloudflare.com/
- [ ] Put the Jev key (`TYPESAFE_API_KEY`) in the server's `rambl.env` — see Environment variables in [README.md](../../README.md) (gitignored: never on origin/main).

## Deploy

- [ ] Deploy per the [README Deploy section](../../README.md#deploy) (DEPLOY.md planned — not in the repo yet).

## Product decisions

- [ ] Work through the design review: https://claude.ai/code/artifact/a667d476-54d6-447b-8970-7accdea412fb
