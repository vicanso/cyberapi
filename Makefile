.PHONY: default

lint:
	cd src-tauri && cargo clippy
fmt:
	cd src-tauri && cargo fmt --all --
dev:
	npm run tauri dev
build:
	npm run tauri build