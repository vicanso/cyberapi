.PHONY: default

lint:
	cd src-tauri && cargo clippy 
fmt:
	cd src-tauri && cargo fmt --all --
dev:
	npm run tauri dev
icon:
	cargo tauri icon ./cyberapi.png
build:
	cargo tauri build
orm:
	cd src-tauri && sea-orm-cli generate entity --with-serde=both \
    -u "sqlite:///~/Library/Application Support/com.bigtree.cyberapi/my_db.db" \
    -o src/entities