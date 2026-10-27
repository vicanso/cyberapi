[中文](./INSTALL_zh.md) | English

# Installing Cyber API

Back to the [README](../README.md).

## Downloads

Download from [Releases](https://github.com/vicanso/cyberapi/releases). Each release lists its files in `SHA256SUMS`; `sha256sum -c SHA256SUMS --ignore-missing` checks the ones you downloaded.

| Platform | File | Notes |
|---|---|---|
| macOS (Apple silicon) | `cyberapi-<version>-aarch64-apple-darwin.dmg` | Signed and notarized. Drag CyberAPI to Applications. |
| macOS (Intel) | `cyberapi-<version>-x86_64-apple-darwin.dmg` | |
| Windows (x86_64) | `cyberapi-<version>-x86_64-pc-windows-msvc.zip` | Unzip and run `cyberapi.exe`. Not signed: SmartScreen may ask once. |
| Linux (x86_64, arm64) | `cyberapi_<version>_amd64.deb`, `cyberapi_<version>_arm64.deb` | Debian, Ubuntu and their relatives. |
| Linux (x86_64, aarch64) | `cyberapi-<version>-<arch>-unknown-linux-gnu.tar.gz` | The binary alone. Needs glibc 2.35 or newer. |

## The command line

`cyberapi-cli run` runs a project's APIs without the window — in a terminal or CI — as the app's Run… does. There is no package of it yet: build it from the source with Rust installed, `cargo install --path crates/cyberapi-cli` (or `make cli`, which leaves it in the cargo target directory's `release/`).

```sh
cyberapi-cli run project.json --env-file staging.postman_environment.json --data users.csv --junit results.xml
```

The project is a file: CyberAPI's own export (Export → CyberAPI, which can live in git), or anything the app imports — a Postman collection, an Insomnia export, an OpenAPI document, Bruno and Hoppscotch files. `--env` picks one of its named environments; `--env-file` adds another file's variables, and its environment is the run's unless `--env` names one; `--var NAME=VALUE` and `--data FILE` (CSV or a JSON array, once for each row) give values read as `{{value(NAME)}}`; `env(NAME)` reads the process's environment, where CI keeps its secrets; `--prompt LABEL=VALUE` answers a `prompt`. Each send is printed as it ends, with what did not hold; `--junit` writes the run as JUnit XML. The exit code is 0 when every send held, 1 when one did not, 2 when nothing could run. A file a request reads is found beside the project file.

## Nightly builds

A `nightly` prerelease is built from `main` every night when it has moved. Its version is the next patch's, with the day and the commit, e.g. `0.2.2-nightly.20261024.137a858`, which the app's About and `cyberapi-bridge --version` show.

## Updates

The desktop app looks for a newer release at start, at most once every two days; Settings → About turns that off, or checks at once. Nothing is downloaded until you ask, and the package is checked against the sum the release published. On macOS it then replaces the app in place, and a restart opens the new version; on Windows and Linux it is saved in your downloads folder and opened for you to install.

## Coming from the Tauri version

The macOS app has the same name and identifier as the earlier Tauri app, so installing it replaces the old one. On first launch it copies the old app's data, if there is one, and leaves the old files as they were. What is copied is the database (projects, folders, APIs, variables) and the cookies. What the old app kept in its web view is not: its settings (theme, language, timeouts), pinned APIs, open folders, chosen tabs, response history and the order of a project's top level. Those start afresh, and backups made by the old app restore as they always did. A database from the old app's first release (app-v0.1.0, 2022) still holds a table of environment variables that later releases left behind; it is kept, renamed `legacy_environments`, and no longer stands in the way of named environments (in 0.3.0 that table made every server project's copy fail with “table environments has no column named position”).

## Where the data lives

The data lives in `CyberAPI` under the platform's data directory: `~/Library/Application Support` on macOS, `%APPDATA%` on Windows, and `~/.local/share` (as `cyberapi`) on Linux.

The app writes a log there too, in `logs`: one file a day (`cyberapi.log.2026-10-27`), the last 7 kept — what it did, and every failure it showed you. **Settings → Local data → Show logs** opens the folder; send the files along when you report a problem. `RUST_LOG` sets how much is written (`info` unless set).
