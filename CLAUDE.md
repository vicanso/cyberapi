# CLAUDE.md

CyberAPI 的 GPUI 重写版：跨平台 HTTP API 客户端（类似 Postman / Insomnia），同一份代码构建**桌面版**和**浏览器版**。功能范围与分层设计见 `docs/gpui-refactor-feature-analysis.md`（功能编号 `F<模块>.<序号>`，代码注释里引用这些编号）。工程结构参照 `~/github/zedis`。

## Commands

- 运行桌面版：`make dev`（`RUST_LOG=debug` 用 `make debug`）
- 桌面版检查：`make lint`（`cargo fmt --check` + `cargo clippy --all-targets --all -- --deny=warnings`），完成任何改动前必须通过
- 浏览器版检查：`make check-web`（在 `cyberapi-web/` 下用 nightly 对 wasm32 做 `cargo check`）
- 浏览器版构建：`make web-bundle`（开发）/ `make web-release`（发布，需要 `wasm-opt`），产物在 `cyberapi-web/www/wasm/`
- 浏览器版预览：`make web-serve`，打开 http://127.0.0.1:7380/

改动涉及任何带 target 条件编译的文件时，`make lint` 和 `make check-web` **都要跑**：前者看不到 wasm，后者看不到桌面。

## Workspace layout

- 根 crate `cyberapi-gui`：库 `cyberapi_gui`（`src/lib.rs`，全部应用代码）+ 只调用 `cyberapi_gui::run()` 的 bin `cyberapi`（`src/main.rs`）。`default-members = ["."]`，裸 `cargo build` 只构建桌面版。
- `cyberapi-web`：浏览器入口。把 GUI crate 编译成 wasm，只负责平台、UI 字体和资源来源；通过 `cyberapi_gui::launch` 进入应用。有自己的 `rust-toolchain.toml`（nightly + wasm32），必须在该目录下构建。`www/index.html` 拉取 `fonts/IBMPlexSans.ttf` 和 wasm 后调用 `run(origin, ui_font)`。
- UI 只依赖 `gpui-kit` 一个 crate：GPUI 是 `gpui_kit::*`，组件是 `gpui_kit::component`，图标是 `gpui_kit::assets`，平台是 `gpui_kit::platform`（wasm 上还有 `gpui_kit::web`）。不要再单独引入 `gpui-pre-*`。

## 一份代码，两个目标

桌面版是产品和参考实现，浏览器版是同一个 GUI crate 去掉部分能力后的 wasm 构建。目标：**为浏览器做的改动不能改变桌面版的行为。**

- **依赖方向单向**：`cyberapi-gui` ← `cyberapi-web`。GUI crate 和以后的 `crates/*` 不得引用 `cyberapi-web`；`cyberapi-web` 保持轻薄，只通过具名入口（目前是 `launch`）进入应用，需要更多时加入口，不要伸手进 views。
- **默认代码是桌面代码，浏览器做减法**：`#[cfg(not(target_family = "wasm"))]` 去掉桌面专属能力；`#[cfg(target_family = "wasm")]` 的浏览器代码属于 `cyberapi-web`，或放在有名字的 seam 后面，不要散落在 view 的 `render` 里。
- 一个函数需要两种行为时，写两个同签名函数各自加 gate（桌面在前），不要在一个函数体里交错 gate 语句。
- 只用属性形式的 `#[cfg(...)]`；`cfg!(...)` 会让两个分支都参与编译。按 OS 的 gate 要先排除 wasm（浏览器也是 “not linux”）。
- 能在 module 级 gate 就不要在 item 级，能在 item 级就不要在语句级。
- **依赖也分表**：普通 `[dependencies]` 里的 crate 必须能编译到 `wasm32-unknown-unknown`；仅桌面的放 `[target.'cfg(not(target_family = "wasm"))'.dependencies]`，仅浏览器的放 `[target.'cfg(target_family = "wasm")'.dependencies]`。

### 浏览器版的已知约束（待设计）

浏览器不能替代桌面版的 L1 基础设施：受 CORS 限制不能请求任意域名，`Cookie` / `User-Agent` / `Host` 等是禁止设置的请求头，拿不到 DNS / TCP / TLS 分阶段耗时（F5.6），也没有 SQLite 和本地文件系统。因此和 zedis 的 `zedis-bridge` 一样，浏览器版的请求执行和数据存储预计要经过一个服务端（`cyberapi-bridge`），切点放在 L1 的 trait（`HttpClient`、Repository 等）上：桌面版直连，浏览器版走 bridge。上层（L0 / L2 / L3 / View）两端共用。

## 分层（规划）

按 `docs/gpui-refactor-feature-analysis.md` 第 3 节：L0 Domain（纯函数，无 IO）→ L1 Infrastructure（trait + 实现）→ L2 Services → L3 State → L4 View（GPUI）。随功能落地拆出 `crates/*`（例如 L0 进 `crates/cyberapi-core`，不依赖 gpui，必须能编译到 wasm），届时把 `crates/*` 加入 workspace `members`。

## GPUI

- 启动顺序：先 `gpui_kit::init(cx)`，再用 `gpui_kit::open_window` 开窗（它会挂上 `Root`，弹窗、通知、tooltip 依赖它）。
- 不要臆造 API：以 `~/.cargo/registry/src/*/gpui-kit-0.8.0`、`gpui-component-0.8.0`、`gpui-pre-0.3.9` 的源码为准。
- 需要 `#[derive(IntoElement)]` 等派生宏时，在 `src/lib.rs` 加 `extern crate gpui_kit as gpui;`，宏展开出的 `gpui::…` 路径才能解析。
