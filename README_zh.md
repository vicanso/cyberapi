中文 | [English](./README.md)

<p align="center">
  <img src="./icons/cyberapi.png" alt="Cyber API" width="128">
</p>

<h1 align="center">Cyber API</h1>

<p align="center">
  <strong>用 Rust 和 <a href="https://www.gpui.rs/">GPUI</a> 写的原生 HTTP API 客户端：同一份代码构建桌面版和可自行部署的网页版。</strong>
</p>

<p align="center">
  <a href="https://github.com/vicanso/cyberapi/actions/workflows/ci.yml"><img src="https://github.com/vicanso/cyberapi/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License"></a>
  <a href="https://github.com/vicanso/cyberapi/releases"><img src="https://img.shields.io/github/downloads/vicanso/cyberapi/total" alt="Downloads"></a>
  <a href="https://hub.docker.com/r/vicanso/cyberapi-web"><img src="https://img.shields.io/docker/pulls/vicanso/cyberapi-web" alt="Docker pulls"></a>
</p>

<p align="center">
  <a href="#功能">功能</a> ·
  <a href="#安装">安装</a> ·
  <a href="#网页版自行部署">网页版</a> ·
  <a href="./docs/DEPLOY_zh.md">部署指南</a> ·
  <a href="#开发">开发</a>
</p>

---

## 功能

- **原生、快。** 支持 macOS、Windows、Linux，由 GPUI 在 GPU 上绘制。几千个接口的项目也是立即打开。浅色、深色主题，中文、英文界面，字号和自动换行可调；桌面版窗口在每块屏幕上都按上次离开时的位置打开。
- **项目、目录和接口。** 按名称或拼音搜索，拖动调整顺序，创建副本、移动到目录、复制接口配置；常用接口可以置顶（⌘/Ctrl-1…9）；首页的项目可以分组。⌘/Ctrl+Enter 发送，⌘/Ctrl+N 新建接口，方向键在接口树中移动，快捷键一览列出全部快捷键。
- **请求。** Query、请求头、Basic 认证，请求体支持 JSON、表单、multipart、XML、纯文本；任何键值列表都能切换成文本批量编辑。在地址栏里写的查询参数会移到 Query 表。可设置超时、HTTP/1.1 或 HTTP/2、是否跟随重定向，需要时可以接受自签名证书。
- **GraphQL。** GraphQL 请求体分开编辑查询、变量和操作名，带语法高亮；POST 以 JSON 发送，GET 以查询参数发送，响应里的 `errors` 显示在响应体上方。获取接口的 Schema 后，编辑查询时会补全字段、参数和枚举值。Postman、Insomnia 里的 GraphQL 请求导入后仍是 GraphQL。
- **变量和函数。** 每个项目有环境变量、自定义变量和全局请求头。模板函数如 `{{g(接口, data.token)}}`（取另一个接口最近一次响应里的字段）、`{{value(name)}}`、`{{timestamp()}}`、`{{md5(…)}}`、`{{sha256(…)}}`、`{{base64.readFile(…)}}`、`{{readTextFile(…)}}`、`{{openFile()}}`（发送时选择文件），输入时有提示。
- **响应。** 耗时分为 DNS、TCP、TLS、服务器处理和传输；JSON 可看原始内容、美化后的缩进或树形（也支持 NDJSON）；图片直接显示，可适应面板或缩放，并显示格式和尺寸；PDF 用系统查看器打开，HTML 页面可在浏览器中打开（桌面版）；其他二进制内容以十六进制显示；没有 Content-Type 的响应体按文件头识别；响应体可以复制或保存成文件；每个接口有历史记录。大响应也不卡：只显示开头部分，超长的一行改为横向滚动。
- **Cookie。** 从每个响应中保存，随匹配的请求发送，所有项目共用；可以搜索、新增、修改、删除或全部清空。
- **cURL 和导入。** 粘贴 cURL 命令即新建接口，任何请求都能复制为 cURL；可导入 Postman、Insomnia、Swagger 2 / OpenAPI 3 和 cURL；可按接口、目录或项目导出。
- **数据。** 桌面版可备份和恢复，格式与之前的 Tauri 版相同。
- **团队。** 可自行部署的网页版：一个程序或一个 Docker 镜像，可放在根路径或子路径下；账号通过邀请创建，有团队和项目级的角色（查看者、编辑者、管理者），项目可以共享给其他账号或团队，也可以移到别的团队。桌面版可以登录同一台服务器，离线编辑共享项目，在你选择的时候同步；期间别人也改过时，由你选择保留哪一份；接口和目录也可以设为仅本地，不参与同步。
- **AI 助手。** 网页版同时是 MCP 服务器：Claude Code、Cursor 等 MCP 客户端用你签发的令牌读取项目、接口、变量和响应，敏感值打码；允许发送的令牌还能发送已保存的接口。

## 安装

从 [Releases](https://github.com/vicanso/cyberapi/releases) 下载。每个版本附带 `SHA256SUMS`，`sha256sum -c SHA256SUMS --ignore-missing` 可以校验已下载的文件。

| 平台 | 文件 | 说明 |
|---|---|---|
| macOS（Apple 芯片） | `cyberapi-<版本>-aarch64-apple-darwin.dmg` | 已签名并公证。把 CyberAPI 拖进“应用程序”。 |
| macOS（Intel） | `cyberapi-<版本>-x86_64-apple-darwin.dmg` | |
| Windows（x86_64） | `cyberapi-<版本>-x86_64-pc-windows-msvc.zip` | 解压后运行 `cyberapi.exe`。未签名，SmartScreen 可能会提示一次。 |
| Linux（x86_64、arm64） | `cyberapi_<版本>_amd64.deb`、`cyberapi_<版本>_arm64.deb` | Debian、Ubuntu 及其衍生版。 |
| Linux（x86_64、aarch64） | `cyberapi-<版本>-<架构>-unknown-linux-gnu.tar.gz` | 只有程序本身。需要 glibc 2.35 或更新。 |

`main` 有新提交时，每晚构建一个 `nightly` 预发布版。它的版本号是下一个补丁版本加上日期和提交，如 `0.2.2-nightly.20261024.137a858`，应用的“关于”和 `cyberapi-bridge --version` 都会显示。

桌面版启动时会检查有没有新版本，最多每两天一次；在“设置 → 关于”里可以关闭，或立即检查。只在你点击后才下载，下载后按发布时的校验和核对。macOS 上随即替换当前应用，重启后就是新版本；Windows 和 Linux 上把安装包存到下载目录，再交给系统打开安装。

### 从 Tauri 版升级

macOS 应用与之前的 Tauri 版同名、同标识符，安装后会替换旧版。首次启动时，如果有旧版的数据，会复制一份过来，旧版的文件保持不动。复制的是数据库（项目、目录、接口、变量）和 Cookie。旧版存在网页视图里的内容不会复制：设置（主题、语言、超时）、置顶的接口、展开的目录、选中的标签、响应历史，以及项目顶层的顺序。这些从头开始；旧版做的备份照常可以恢复。

数据在平台数据目录下的 `CyberAPI` 里：macOS 是 `~/Library/Application Support`，Windows 是 `%APPDATA%`，Linux 是 `~/.local/share`（目录名为 `cyberapi`）。

## 网页版（自行部署）

网页版就是同一个应用编译成 WebAssembly，由 `cyberapi-bridge` 提供。这一个程序同时把账号和项目存进 PostgreSQL 或 MySQL，并替页面发送请求。Docker Hub 上有 linux/amd64 和 linux/arm64 的镜像 [`vicanso/cyberapi-web`](https://hub.docker.com/r/vicanso/cyberapi-web)（`latest`、各个版本号，以及 `main` 的 `nightly`），每个版本的 Release 里也有压缩包。

```bash
docker run -p 7380:7380 \
  -e CYBERAPI_BRIDGE_STORE=postgres://user:password@host:5432/cyberapi \
  vicanso/cyberapi-web
```

还没有任何账号时，日志里会打印一个 setup token；打开页面，用它创建第一个管理员，其他人由管理员邀请。登录 Cookie 带 `Secure`，所以前面要有 HTTPS；只想用纯 HTTP 试一下时，在镜像名后面加 `--insecure-cookie`。

**[部署指南](./docs/DEPLOY_zh.md)** 介绍了 Docker Compose、二进制加 systemd、nginx 反向代理下的 HTTPS（包括子路径）、全部选项、请求能发往哪些地址、账号和邀请、桌面版登录、升级和备份。

## 开发

需要 Rust（rustup 会按 `rust-toolchain.toml` 选版本）。Linux 上先安装：`libfontconfig1-dev libxcb-xfixes0-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libssl-dev`。

```bash
make dev            # 运行桌面版（debug 构建用自己的数据库）
make lint           # 格式检查和 clippy，与 CI 相同
make test           # 全部测试
make desktop-dist   # 为本机打包（cargo install --locked cargo-bundle@0.11.0）
```

网页版需要带 `wasm32-unknown-unknown` 目标的 nightly 工具链（按 `cyberapi-web/rust-toolchain.toml` 自动选择）和 `wasm-pack`（`cargo install wasm-pack`）；发布构建还需要 `wasm-opt`（binaryen）。

```bash
make db-up          # 在本地容器里启动 PostgreSQL 和 MySQL（Apple 的 `container`）
make web-bundle     # 构建页面
make bridge         # http://127.0.0.1:7380/，setup token 为 `dev`
make check-web      # wasm 构建的检查
make web-dist       # 部署用的压缩包；`make docker` 构建镜像
```

代码的组织方式和原因见 [docs/architecture.md](./docs/architecture.md)，服务器项目如何同步见 [docs/server-sync.md](./docs/server-sync.md)。发布用 `make version` 和 `v*` tag，见 `CHANGELOG.md`。

## 许可证

[Apache-2.0](./LICENSE)
