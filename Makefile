.PHONY: default docs

lint:
	cargo clippy
fmt:
	cargo fmt --all --
web-dev:
	cd web && npm run dev
web-build:
	cd web && npm run build
web-format:
	cd web && npm run format
docs:
	cp -rf docs web/dist/
	cp assets/* web/dist/assets/
dev:
	cargo run