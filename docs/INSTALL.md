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

## Nightly builds

A `nightly` prerelease is built from `main` every night when it has moved. Its version is the next patch's, with the day and the commit, e.g. `0.2.2-nightly.20261024.137a858`, which the app's About and `cyberapi-bridge --version` show.

## Updates

The desktop app looks for a newer release at start, at most once every two days; Settings → About turns that off, or checks at once. Nothing is downloaded until you ask, and the package is checked against the sum the release published. On macOS it then replaces the app in place, and a restart opens the new version; on Windows and Linux it is saved in your downloads folder and opened for you to install.

## Coming from the Tauri version

The macOS app has the same name and identifier as the earlier Tauri app, so installing it replaces the old one. On first launch it copies the old app's data, if there is one, and leaves the old files as they were. What is copied is the database (projects, folders, APIs, variables) and the cookies. What the old app kept in its web view is not: its settings (theme, language, timeouts), pinned APIs, open folders, chosen tabs, response history and the order of a project's top level. Those start afresh, and backups made by the old app restore as they always did.

## Where the data lives

The data lives in `CyberAPI` under the platform's data directory: `~/Library/Application Support` on macOS, `%APPDATA%` on Windows, and `~/.local/share` (as `cyberapi`) on Linux.
