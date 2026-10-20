.PHONY: dev debug check fmt lint test release check-web web-bundle web-release web-serve

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

# The browser's gate, which sees nothing of the desktop (and `lint` sees
# nothing of wasm). Runs from cyberapi-web/, whose rust-toolchain.toml selects
# the nightly GPUI's web backend needs.
check-web:
	cd cyberapi-web && cargo check --target wasm32-unknown-unknown -p cyberapi-gui -p cyberapi-web

# The browser bundle (`cargo install wasm-pack` once). `web-bundle` is the
# iteration build, `web-release` the shipped form (needs binaryen's wasm-opt).
# Both write cyberapi-web/www/, which `web-serve` serves as static files:
# open http://127.0.0.1:7380/.
web-bundle:
	scripts/web-bundle.sh

web-release:
	scripts/web-bundle.sh --release

web-serve:
	python3 -m http.server 7380 --bind 127.0.0.1 --directory cyberapi-web/www
