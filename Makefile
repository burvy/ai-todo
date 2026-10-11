# ai-todo — the commands a newcomer actually needs.
# `make run` is the one to start with; the rest are what CI runs.
#
# The server reads .env itself on startup and listens on port 8080 unless
# PORT says otherwise, so `make run` needs nothing extra.

run:
	cargo run -p server

fmt:
	cargo fmt --all

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

# fmt + lint + test: run before pushing a branch.
check: fmt lint test