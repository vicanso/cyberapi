中文 | [English](./INSTALL.md)

# 安装 Cyber API

返回 [README](../README_zh.md)。

## 下载

从 [Releases](https://github.com/vicanso/cyberapi/releases) 下载。每个版本附带 `SHA256SUMS`，`sha256sum -c SHA256SUMS --ignore-missing` 可以校验已下载的文件。

| 平台 | 文件 | 说明 |
|---|---|---|
| macOS（Apple 芯片） | `cyberapi-<版本>-aarch64-apple-darwin.dmg` | 已签名并公证。把 CyberAPI 拖进“应用程序”。 |
| macOS（Intel） | `cyberapi-<版本>-x86_64-apple-darwin.dmg` | |
| Windows（x86_64） | `cyberapi-<版本>-x86_64-pc-windows-msvc.zip` | 解压后运行 `cyberapi.exe`。未签名，SmartScreen 可能会提示一次。 |
| Linux（x86_64、arm64） | `cyberapi_<版本>_amd64.deb`、`cyberapi_<版本>_arm64.deb` | Debian、Ubuntu 及其衍生版。 |
| Linux（x86_64、aarch64） | `cyberapi-<版本>-<架构>-unknown-linux-gnu.tar.gz` | 只有程序本身。需要 glibc 2.35 或更新。 |

## Nightly 版

`main` 有新提交时，每晚构建一个 `nightly` 预发布版。它的版本号是下一个补丁版本加上日期和提交，如 `0.2.2-nightly.20261024.137a858`，应用的“关于”和 `cyberapi-bridge --version` 都会显示。

## 更新

桌面版启动时会检查有没有新版本，最多每两天一次；在“设置 → 关于”里可以关闭，或立即检查。只在你点击后才下载，下载后按发布时的校验和核对。macOS 上随即替换当前应用，重启后就是新版本；Windows 和 Linux 上把安装包存到下载目录，再交给系统打开安装。

## 从 Tauri 版升级

macOS 应用与之前的 Tauri 版同名、同标识符，安装后会替换旧版。首次启动时，如果有旧版的数据，会复制一份过来，旧版的文件保持不动。复制的是数据库（项目、目录、接口、变量）和 Cookie。旧版存在网页视图里的内容不会复制：设置（主题、语言、超时）、置顶的接口、展开的目录、选中的标签、响应历史，以及项目顶层的顺序。这些从头开始；旧版做的备份照常可以恢复。

## 数据存放位置

数据在平台数据目录下的 `CyberAPI` 里：macOS 是 `~/Library/Application Support`，Windows 是 `%APPDATA%`，Linux 是 `~/.local/share`（目录名为 `cyberapi`）。
