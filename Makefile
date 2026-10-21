.PHONY: dev debug check fmt lint test release desktop-dist icons nightly check-web web-bundle web-release web-serve web-dist db-up db-down test-sql bridge

# The desktop app.
dev:
	cargo run

debug:
	RUST_LOG=debug cargo run

check:
	cargo check

fmt:
	cargo fmt --all

# The desktop's gate: run it before calling any change done.
lint:
	cargo fmt --all --check
	cargo clippy --all-targets --all -- --deny=warnings

test:
	cargo test --all

release:
	cargo build --release

# The desktop package for this host (cargo-bundle): a .dmg on macOS, a .deb
# and a tarball on Linux, a zip on Windows, in <target dir>/desktop-dist.
desktop-dist:
	scripts/desktop-dist.sh

# Every platform's icon (resources/icons, the page's) made again from
# icons/cyberapi.png. Run on macOS.
icons:
	scripts/icons.sh

# The nightly build, started now and built even if the branch has not moved
# (.github/workflows/nightly.yml): of main, or of the pushed branch NIGHTLY_REF
# names. Needs `gh`, signed in.
NIGHTLY_REF ?= main

nightly:
	gh workflow run nightly.yml --ref $(NIGHTLY_REF) -f force=true

# The SQL store against real databases: `make db-up` starts PostgreSQL and
# MySQL in local containers (Apple's `container`), `make test-sql` runs the
# shared suite and the access rules on both. Without the variables those
# tests skip, so `make test` needs no database.
PG_URL ?= postgres://cyberapi:cyberapi@127.0.0.1:55432/cyberapi
MYSQL_URL ?= mysql://cyberapi:cyberapi@127.0.0.1:53306/cyberapi

db-up:
	scripts/db.sh up

db-down:
	scripts/db.sh down

test-sql:
	CYBERAPI_TEST_POSTGRES=$(PG_URL) CYBERAPI_TEST_MYSQL=$(MYSQL_URL) cargo test -p cyberapi-store-sql -p cyberapi-bridge

# The bridge on the local PostgreSQL, serving the last `make web-bundle`:
# open http://127.0.0.1:7380/. The first visit makes the administrator with
# the setup token `dev`. Requests may go to this machine, for local APIs.
bridge:
	CYBERAPI_BRIDGE_STORE=$(PG_URL) CYBERAPI_BRIDGE_SETUP_TOKEN=dev cargo run -p cyberapi-bridge -- --insecure-cookie --allow-target 127.0.0.0/8,::1/128

# The browser's gate, which sees nothing of the desktop (and `lint` sees
# nothing of wasm). Runs from cyberapi-web/, whose rust-toolchain.toml selects
# the nightly GPUI's web backend needs. Every crate under `crates/` that the
# page links is listed: they all build for wasm.
check-web:
	cd cyberapi-web && cargo check --target wasm32-unknown-unknown -p cyberapi-core -p cyberapi-store -p cyberapi-store-bridge -p cyberapi-http -p cyberapi-service -p cyberapi-gui -p cyberapi-web

# The browser bundle (`cargo install wasm-pack` once). `web-bundle` is the
# iteration build, `web-release` the shipped form (needs binaryen's wasm-opt).
# Both write cyberapi-web/www/, which `make bridge` serves with the API
# (`web-serve` serves the files alone, with no store behind them).
web-bundle:
	scripts/web-bundle.sh

web-release:
	scripts/web-bundle.sh --release

web-serve:
	python3 -m http.server 7381 --bind 127.0.0.1 --directory cyberapi-web/www

# The browser build's deployment: the release bridge with the release page
# compiled in, as one tarball in <target dir>/web-dist. Build on the platform
# it will run on.
web-dist:
	scripts/web-dist.sh
