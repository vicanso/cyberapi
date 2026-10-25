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
  <a href="#web-version-self-hosted">Web version</a> ·
  <a href="./docs/DEPLOY.md">Deployment guide</a> ·
  <a href="#development">Development</a>
</p>

---

## Features

- **Native and fast.** macOS, Windows and Linux, drawn on the GPU with GPUI. A project with thousands of APIs opens at once. Light and dark themes, English and Chinese, adjustable text sizes and line wrapping; the desktop window reopens where you left it, on each screen.
- **Projects, folders and APIs.** Search by name or pinyin, drag to reorder, duplicate, move to a folder, copy an API's config; pin the APIs you use most (⌘/Ctrl-1…9); group projects on the home page. ⌘/Ctrl-Enter sends, ⌘/Ctrl-N adds an API, ⌘/Ctrl-F finds an API and ⌘/Ctrl-L goes to its address, the arrow keys walk the tree, Delete and ⌘/Ctrl-D delete and duplicate there, and a list shows every shortcut.
- **Requests.** Query, headers, Basic auth, and JSON, form, multipart, XML or plain-text bodies; bulk-edit any list as text. A query typed into the URL moves to the Query table. Timeouts, HTTP/1.1 or HTTP/2, redirects followed or not, and self-signed certificates accepted when you say so.
- **GraphQL.** A GraphQL body keeps the query, its variables and the operation name apart, highlighted; it is sent as JSON by a POST or as parameters by a GET, and the response's `errors` are shown above it. Fetch an endpoint's schema and the query editor completes its fields, arguments and enum values. A subscription sent over HTTP shows its results as they come, from servers that answer it as an event stream (GraphQL Yoga, graphql-sse). Postman's and Insomnia's GraphQL requests import as GraphQL.
- **gRPC.** Unary and server-streaming calls: choose gRPC as the method, give the server's address, and pick a service's method from the server's reflection or from `.proto` files (desktop). The message is JSON — with an example to start from, and template functions — and the metadata are headers; the reply comes back as JSON with its status, and a stream of replies shows as it comes, each with its time, until it ends or you stop it. An API keeps the definitions its service needs, so it is sent anywhere, the web version included, without asking the server again. Copy a call as a grpcurl command. Client and bidirectional streaming are not supported yet.
- **Server-sent events.** An event stream (`text/event-stream`) shows as it comes — each event with its time, type, id and data, the one picked shown whole with the reconnection time it asked for — until the server ends it or you stop it; what came is kept in the history, as events or as it came. A language model's streamed reply, for instance.
- **Variables and functions.** Environments, custom variables and global headers per project. Template functions such as `{{g(api, data.token)}}` (a field of another API's latest response), `{{value(name)}}`, `{{timestamp()}}`, `{{now(%Y-%m-%d)}}`, `{{uuid()}}`, `{{random(1, 100)}}`, `{{randomString(16)}}`, `{{md5(…)}}`, `{{sha256(…)}}`, `{{hmacSha256(key, data)}}`, `{{urlEncode(…)}}`, `{{base64.readFile(…)}}`, `{{readTextFile(…)}}` and `{{openFile()}}` (a file picked as you send), with hints while you type.
- **Responses.** Time split into DNS, TCP, TLS, server and transfer; the request as it went out, every header said where it came from, secrets masked till shown; JSON raw as it came, indented or as a tree (NDJSON too); images shown in place, fitted or zoomed, with their format and size; a PDF in the system viewer, an HTML page in your browser (desktop); any other bytes as hex; a body without a content type told by its first bytes; copy a body or save it to a file, or narrow a JSON one with a JSONPath filter kept per API; a history per API, each entry with the method and path it was sent to, and picking one puts its request back. A large body stays quick: the panel shows its start, and a line too long to wrap scrolls sideways.
- **Cookies.** Kept from every response and sent with the requests they match, across projects; search, add, edit, delete or clear them.
- **cURL and imports.** Paste a cURL command to make an API, copy any request as cURL, HTTPie, JavaScript `fetch`, Python `requests` or Go `net/http`; import Postman, Insomnia, Swagger 2 / OpenAPI 3 (JSON or YAML) and cURL into a project, or as a new project from the projects page; export an API, a folder or a project.
- **Your data.** Back up and restore on the desktop, in the same format as the earlier Tauri app.
- **Teams.** A self-hosted web version — one binary or a Docker image, at the root or under a sub-path — with accounts by invitation, teams, and per-project roles (viewer, editor, manager); share a project with accounts or teams, or move it to another team. The desktop app signs in to the same server and works on shared projects offline, syncing when you choose and asking whose version to keep when someone else changed it meanwhile; APIs and folders can stay local, out of the sync.
- **AI assistants.** The web version is also an MCP server: Claude Code, Cursor or another MCP client signs in with a token you issue and reads your projects, APIs, variables and responses, with secrets masked; a token you allow to may also send saved APIs.

## Install

Download from [Releases](https://github.com/vicanso/cyberapi/releases). Each release lists its files in `SHA256SUMS`; `sha256sum -c SHA256SUMS --ignore-missing` checks the ones you downloaded.

| Platform | File | Notes |
|---|---|---|
| macOS (Apple silicon) | `cyberapi-<version>-aarch64-apple-darwin.dmg` | Signed and notarized. Drag CyberAPI to Applications. |
| macOS (Intel) | `cyberapi-<version>-x86_64-apple-darwin.dmg` | |
| Windows (x86_64) | `cyberapi-<version>-x86_64-pc-windows-msvc.zip` | Unzip and run `cyberapi.exe`. Not signed: SmartScreen may ask once. |
| Linux (x86_64, arm64) | `cyberapi_<version>_amd64.deb`, `cyberapi_<version>_arm64.deb` | Debian, Ubuntu and their relatives. |
| Linux (x86_64, aarch64) | `cyberapi-<version>-<arch>-unknown-linux-gnu.tar.gz` | The binary alone. Needs glibc 2.35 or newer. |

A `nightly` prerelease is built from `main` every night when it has moved. Its version is the next patch's, with the day and the commit, e.g. `0.2.2-nightly.20261024.137a858`, which the app's About and `cyberapi-bridge --version` show.

The desktop app looks for a newer release at start, at most once every two days; Settings → About turns that off, or checks at once. Nothing is downloaded until you ask, and the package is checked against the sum the release published. On macOS it then replaces the app in place, and a restart opens the new version; on Windows and Linux it is saved in your downloads folder and opened for you to install.

### Coming from the Tauri version

The macOS app has the same name and identifier as the earlier Tauri app, so installing it replaces the old one. On first launch it copies the old app's data, if there is one, and leaves the old files as they were. What is copied is the database (projects, folders, APIs, variables) and the cookies. What the old app kept in its web view is not: its settings (theme, language, timeouts), pinned APIs, open folders, chosen tabs, response history and the order of a project's top level. Those start afresh, and backups made by the old app restore as they always did.

The data lives in `CyberAPI` under the platform's data directory: `~/Library/Application Support` on macOS, `%APPDATA%` on Windows, and `~/.local/share` (as `cyberapi`) on Linux.

## Web version (self-hosted)

The web version is the same app compiled to WebAssembly and served by `cyberapi-bridge`, one binary that also keeps the accounts and projects in PostgreSQL or MySQL and sends the page's requests. It is an image on Docker Hub for linux/amd64 and linux/arm64, [`vicanso/cyberapi-web`](https://hub.docker.com/r/vicanso/cyberapi-web) (`latest`, each version, and `nightly` from `main`), and a tarball on each release.

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
