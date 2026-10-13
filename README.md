# Rambl

Rambl turns spoken or typed ramblings into tasks with deadlines and priorities.

## Run locally

1. Build the web bundle, then the server (the server embeds `web/dist`):
   `cd web && trunk build --release && cd ..` then `cargo build --release -p rambl-server`.
2. Copy `rambl.env.example` to `rambl.env` and edit as needed.
3. Run `target/release/rambl-server.exe` (or `rambl-server` on Unix) from the folder holding `rambl.env`.
4. Open `http://127.0.0.1:3160` in a browser.

See **DEPLOY.md** for production deployment instructions and the `docs/` folder for detailed documentation.
