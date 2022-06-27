.PHONY: default

lint:
	cd src-tauri && cargo clippy
fmt:
	cd src-tauri && cargo fmt --all --
