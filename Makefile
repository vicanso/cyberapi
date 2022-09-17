.PHONY: default

lint:
	cargo clippy
fmt:
	cargo fmt --all --
web-dev:
	cd web && npm run dev
web-build:
	cd web && npm run build
dev:
	cargo run