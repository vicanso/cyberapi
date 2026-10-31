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
| 命令行（macOS、Linux、Windows） | `cyberapi-cli-<版本>-<平台>.tar.gz` / `.zip` | 终端与 CI 使用的 `cyberapi-cli`。 |

## 命令行

`cyberapi-cli run` 不开窗口运行一个项目的接口（终端或 CI 里），和应用里的“运行…”一样。可从 [Releases](https://github.com/vicanso/cyberapi/releases) 下载对应平台的 `cyberapi-cli-<版本>-<平台>` 预编译包，或装好 Rust 从源码构建：`cargo install --path crates/cyberapi-cli`（或 `make cli`，产物在 cargo target 目录的 `release/` 下）。

```sh
cyberapi-cli run project.json --env-file staging.postman_environment.json --data users.csv --junit results.xml
```

项目是一个文件：CyberAPI 自己的导出（导出 → CyberAPI，可以放进 git），或任何应用能导入的格式——Postman 集合、Insomnia 导出、OpenAPI 文档、Bruno 和 Hoppscotch 文件。`--env` 选文件里的命名环境；`--env-file` 补上另一个文件的变量，没写 `--env` 时就用这个文件的环境；`--var 名称=值` 和 `--data 文件`（CSV 或 JSON 数组，每行运行一遍）给出的值用 `{{value(名称)}}` 读；`env(名称)` 读进程的环境变量，CI 的密钥就放在这里；`--prompt 说明=值` 回答 `prompt`。每个请求结束就打印一行，没通过的写出原因；`--junit` 把结果写成 JUnit XML。全部通过时退出码为 0，有失败时为 1，无法运行时为 2。请求读取的文件到项目文件所在的目录里找。

## Nightly 版

`main` 有新提交时，每晚构建一个 `nightly` 预发布版。它的版本号是下一个补丁版本加上日期和提交，如 `0.2.2-nightly.20261024.137a858`，应用的“关于”和 `cyberapi-bridge --version` 都会显示。

## 更新

桌面版启动时会检查有没有新版本，最多每两天一次；在“设置 → 关于”里可以关闭，或立即检查。只在你点击后才下载，下载后按发布时的校验和核对。macOS 上随即替换当前应用，重启后就是新版本；Windows 和 Linux 上把安装包存到下载目录，再交给系统打开安装。

## 从 Tauri 版升级

macOS 应用与之前的 Tauri 版同名、同标识符，安装后会替换旧版。首次启动时，如果有旧版的数据，会复制一份过来，旧版的文件保持不动。复制的是数据库（项目、目录、接口、变量）和 Cookie。旧版存在网页视图里的内容不会复制：设置（主题、语言、超时）、置顶的接口、展开的目录、选中的标签、响应历史，以及项目顶层的顺序。这些从头开始；旧版做的备份照常可以恢复。旧版第一个发布（app-v0.1.0，2022 年）的数据库里还留着一张环境变量的表，之后的版本没有删它；现在它会被改名为 `legacy_environments` 保留下来，不再挡住命名环境（0.3.0 里这张表让拉取服务器项目的副本一律报 “table environments has no column named position”）。

## 数据存放位置

数据在平台数据目录下的 `CyberAPI` 里：macOS 是 `~/Library/Application Support`，Windows 是 `%APPDATA%`，Linux 是 `~/.local/share`（目录名为 `cyberapi`）。

应用也在那里写日志，放在 `logs` 里：每天一个文件（`cyberapi.log.2026-10-27`），保留最近 7 天，记录应用做了什么以及界面上提示过的每个错误。**设置 → 本地数据 → 打开日志目录** 可以直接打开；报告问题时请一并发过来。写多少由 `RUST_LOG` 决定（不设时为 `info`）。
