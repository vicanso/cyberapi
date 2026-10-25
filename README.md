[中文](./README_zh.md) | English

<p align="center">
  <img src="./icons/cyberapi.png" alt="Cyber API" width="128">
</p>

<h1 align="center">Cyber API</h1>

<p align="center">
  <strong>A fast, native HTTP API client in Rust and <a href="https://www.gpui.rs/">GPUI</a>: one codebase for the desktop and a self-hosted web version.</strong>
</p>

<p align="center">
  <a href="https://github.com/vicanso/cyberapi/actions/workflows/ci.yml"><img src="https://github.com/vicanso/cyberapi/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License"></a>
  <a href="https://github.com/vicanso/cyberapi/releases"><img src="https://img.shields.io/github/downloads/vicanso/cyberapi/total" alt="Downloads"></a>
  <a href="https://hub.docker.com/r/vicanso/cyberapi-web"><img src="https://img.shields.io/docker/pulls/vicanso/cyberapi-web" alt="Docker pulls"></a>
</p>

<p align="center">
  <a href="#features">Features</a> ·
  <a href="#install">Install</a> ·
  <a href="./docs/FEATURES.md">All features</a> ·
  <a href="#web-version-self-hosted">Web version</a> ·
  <a href="./docs/DEPLOY.md">Deployment guide</a> ·
  <a href="#development">Development</a>
</p>

---

## Features

**Cyber API** is a native API client for HTTP, GraphQL, gRPC and server-sent events. [GPUI](https://www.gpui.rs/) — the engine behind the Zed editor — draws it on the GPU, so a project with thousands of APIs opens at once.

- ⚡ **Native and instant** — macOS, Windows and Linux; no Electron, no web view.
- 🧩 **Every kind of API** — REST; GraphQL with schema completion; gRPC from server reflection or `.proto` files; event streams shown as they arrive.
- 🔍 **See what really happened** — DNS, TCP, TLS and server time; the request exactly as it went out; JSON as a tree or narrowed by JSONPath; a history for every API.
- 🔗 **Less copy and paste** — switch environments beside the project's name, let folders pass their auth down, and chain responses with `{{g(api, data.token)}}`.
- 👥 **Built for teams** — a self-hosted web version from the same code; the desktop app signs in, works offline and syncs when you choose.
- 🤖 **Ready for AI assistants** — Claude Code, Cursor and other MCP clients read your APIs, secrets masked, and send them when you allow.

Coming from Postman, Insomnia or Swagger? **Import the collection, or just paste a cURL command.**

📖 **[Every feature in detail →](./docs/FEATURES.md)**

## Install

Download for **macOS**, **Windows** or **Linux** from [Releases](https://github.com/vicanso/cyberapi/releases); the app checks for new versions itself. Which file to pick, nightly builds, and moving over from the Tauri version: [the install notes](./docs/INSTALL.md).

## Web version (self-hosted)

The web version is the same app compiled to WebAssembly and served by `cyberapi-bridge`, one binary that also keeps the accounts and projects in PostgreSQL or MySQL and sends the page's requests. It is an image on Docker Hub for linux/amd64 and linux/arm64, [`vicanso/cyberapi-web`](https://hub.docker.com/r/vicanso/cyberapi-web) (`latest`, each version, and `nightly` from `main`), and a tarball on each release.

<p align="center">
  <img src="docs/images/architecture.svg" width="100%" alt="The CyberAPI UI and the HTTP engine are code written once and compiled into more than one program. The desktop app is one native process holding both: it sends in-process and keeps projects in a local SQLite file. The web version runs the same UI as WebAssembly in a browser tab and calls cyberapi-bridge over HTTPS; the bridge sends with the same engine and keeps the team's projects, accounts and histories in PostgreSQL or MySQL. The desktop app signs in to the same bridge, keeps offline copies of the team's projects and syncs them; MCP clients read them through the bridge.">
</p>

```bash
docker run -p 7380:7380 \
  -e CYBERAPI_BRIDGE_STORE=postgres://user:password@host:5432/cyberapi \
  vicanso/cyberapi-web
```

With no account yet the log shows a setup token; open the page and make the first administrator with it. The administrator invites everyone else. The login cookie is `Secure`, so put HTTPS in front of it, or add `--insecure-cookie` after the image name to try it over plain HTTP.

**[The deployment guide](./docs/DEPLOY.md)** covers Docker Compose, the binary with systemd, HTTPS behind nginx (also under a sub-path), every option, which addresses requests may reach, accounts and invitations, signing in from the desktop, upgrades and backups.

## Development

Rust (the version in `rust-toolchain.toml` is picked up by rustup). On Linux, first: `libfontconfig1-dev libxcb-xfixes0-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libssl-dev`.

```bash
make dev            # run the desktop app (a debug build uses its own database)
make lint           # fmt check and clippy, as CI
make test           # every test
make desktop-dist   # package for this machine (cargo install --locked cargo-bundle@0.11.0)
```

The web version needs the nightly toolchain with the `wasm32-unknown-unknown` target (picked up from `cyberapi-web/rust-toolchain.toml`) and `wasm-pack` (`cargo install wasm-pack`); its release form also needs `wasm-opt` (binaryen).

```bash
make db-up          # PostgreSQL and MySQL in local containers (Apple's `container`)
make web-bundle     # build the page
make bridge         # http://127.0.0.1:7380/, setup token `dev`
make check-web      # the wasm build's check
make web-dist       # the deployment tarball; `make docker` builds the image
```

How the code is laid out, and why, is in [docs/architecture.md](./docs/architecture.md); how server projects sync, in [docs/server-sync.md](./docs/server-sync.md) (both in Chinese). Releases are cut with `make version` and a `v*` tag; see `CHANGELOG.md`.

## License

[Apache-2.0](./LICENSE)
