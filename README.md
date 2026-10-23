# Cyber API

A fast, native HTTP API client built with Rust and [GPUI](https://www.gpui.rs/), from one codebase for the desktop and the browser.

## Desktop

```bash
make dev            # run it
make bundle         # the macOS app, CyberAPI.app (cargo install --locked cargo-bundle@0.11.0)
make desktop-dist   # package it for this machine, a .dmg on macOS
```

On first launch the desktop app copies the data of the earlier Tauri app, if there is one; the old app's files are left as they were. What is copied is the database (projects, folders, APIs, variables) and the cookies. What the old app kept in its web view's storage is not: its settings (theme, language, timeouts), pinned APIs, open folders, chosen tabs, response history and the order of a project's top level. Those start afresh, and backups made by the old app restore as they always did.

## Browser

The browser build is the same app compiled to wasm, served by `cyberapi-bridge`, which keeps accounts and projects in PostgreSQL or MySQL and sends the requests for the page. Building it needs the nightly toolchain with the `wasm32-unknown-unknown` target (picked up from `cyberapi-web/rust-toolchain.toml`), `wasm-pack` (`cargo install wasm-pack`) and, for the release form, `wasm-opt` (binaryen).

```bash
make db-up        # PostgreSQL and MySQL in local containers
make web-bundle   # the page
make bridge       # http://127.0.0.1:7380/, setup token `dev`
```

`make web-dist` builds the deployment: one `cyberapi-bridge` binary with the page compiled in, packed with its README.

The same deployment is an image on Docker Hub, for linux/amd64 and linux/arm64: `vicanso/cyberapi-web` (`latest` and each version, `nightly` from `main`). `make docker` builds it from the sources.

```bash
docker run -p 7380:7380 -e CYBERAPI_BRIDGE_STORE=postgres://user:password@host/db vicanso/cyberapi-web
```

With no account yet the log shows the setup token (`CYBERAPI_BRIDGE_SETUP_TOKEN` chooses it). The login cookie is Secure: put TLS in front, or add `--insecure-cookie` after the image name to try it over plain HTTP.

## License

Apache-2.0
