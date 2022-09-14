.PHONY: default

lint:
	cd src-tauri && cargo clippy
fmt:
	cd src-tauri && cargo fmt --all --
dev:
	npm run tauri dev
icon:
	npx @tauri-apps/tauricon ./cyberapi.svg
build:
	cargo tauri build