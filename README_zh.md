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
  <a href="./docs/FEATURES_zh.md">完整功能</a> ·
  <a href="#网页版自行部署">网页版</a> ·
  <a href="./docs/DEPLOY_zh.md">部署指南</a> ·
  <a href="#开发">开发</a>
</p>

---

## 功能

**Cyber API** 是原生的 API 客户端，支持 HTTP、GraphQL、gRPC 和服务器推送事件。界面由 Zed 编辑器背后的 [GPUI](https://www.gpui.rs/) 在 GPU 上绘制，几千个接口的项目也是立即打开。

- ⚡ **原生、即开** —— macOS、Windows、Linux；不是 Electron，也不套网页。
- 🧩 **各种接口都能调** —— REST；GraphQL 按 Schema 补全；gRPC 从服务器反射或 `.proto` 文件取方法；WebSocket；事件流边收边显示。
- 🔍 **看清每一次请求** —— DNS、TCP、TLS、服务器各花多久；实际发出的请求原样可查；JSON 可看树形或用 JSONPath 过滤；每个接口都有历史。
- 🔗 **少复制粘贴** —— 在项目名旁切换环境，认证由目录往下继承，用 `{{g(接口, data.token)}}` 把上一个响应接进下一个请求。
- 👥 **团队协作** —— 同一份代码的网页版可自行部署；桌面版登录后离线编辑，想同步时再同步。
- 🤖 **接入 AI 助手** —— Claude Code、Cursor 等 MCP 客户端可以读取你的接口（敏感值打码），经你允许还能发送。

从 Postman、Insomnia、Hoppscotch、Bruno 或 Swagger 过来？**导入集合，或者直接粘贴 cURL 命令。**

📖 **[完整功能说明 →](./docs/FEATURES_zh.md)**

## 安装

从 [Releases](https://github.com/vicanso/cyberapi/releases) 下载 **macOS**、**Windows** 或 **Linux** 版，应用会自己检查更新。选哪个文件、nightly 版、从 Tauri 版迁移，见[安装说明](./docs/INSTALL_zh.md)。

## 网页版（自行部署）

网页版就是同一个应用编译成 WebAssembly，由 `cyberapi-bridge` 提供。这一个程序同时把账号和项目存进 PostgreSQL 或 MySQL，并替页面发送请求。Docker Hub 上有 linux/amd64 和 linux/arm64 的镜像 [`vicanso/cyberapi-web`](https://hub.docker.com/r/vicanso/cyberapi-web)（`latest`、各个版本号，以及 `main` 的 `nightly`），每个版本的 Release 里也有压缩包。

<p align="center">
  <img src="docs/images/architecture.svg" width="100%" alt="CyberAPI 的界面和 HTTP 引擎是同一份代码，编译进不止一个程序。桌面版是一个原生进程，两者都在其中：在进程内发送，项目存在本机的 SQLite 文件里。网页版把同一套界面以 WebAssembly 跑在浏览器标签页里，通过 HTTPS 访问 cyberapi-bridge；bridge 用同一个引擎发送，把团队的项目、账号和历史存在 PostgreSQL 或 MySQL 里。桌面版登录同一个 bridge 后在本机保存团队项目的离线副本并同步；MCP 客户端通过 bridge 读取这些项目。">
</p>

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
