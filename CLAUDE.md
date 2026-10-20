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

- 根 crate `cyberapi-gui`：库 `cyberapi_gui`（`src/lib.rs`，全部应用代码）+ 只调用 `cyberapi_gui::run()` 的 bin `cyberapi`（`src/main.rs`）。`default-members = ["."]`，裸 `cargo build` 只构建桌面版；`members = ["crates/*", "cyberapi-web"]`。
  - 入口：桌面版的 `run()` 先打开数据库（`open_desktop_storage`：确定数据目录、复制旧数据、打开 `TursoStore`、读取设置），再调用 `launch(cx, storage, prefs)`；数据库打不开时调用 `launch_failed` 显示原因。浏览器版由 `cyberapi-web` 准备 `Storage` 后同样调用 `launch`。
  - `src/states/`：gpui entity。`AppStore`（Global，持有 `Storage` 和 `Entity<AppState>`）、`AppState`（路由、设置）、`DashboardState` 等。只有 states 调用 store，统一通过 `states::run_store`（后台执行，结果回到 entity）。修改失败时发事件，由 view 弹通知。
    - 树这类答案是“绝对值”的修改（“这个目录的 children 现在是 …”）用 `states::run_serial` 排队：一次一个，按发起顺序执行和应用，避免并行请求乱序导致副本和存储不一致。
    - states 的测试用 `#[gpui_kit::test]` + `TestAppContext` + `MemoryStore`（见 `states/workspace.rs`）。
  - `src/views/`：只负责绘制和转发事件，不调用 store。`root.rs` 按路由切换页面、同步主题和窗口标题。
  - `src/i18n.rs` + `locales/{en,zh-CN}.toml`：rust-i18n，当前语言与 gpui-component 共用，所以中文的 locale 代码用组件的 `zh-CN`。**新增文案时每个 locale 文件都要加**，测试 `every_locale_has_every_key` 会逐个 key 比对。store 错误统一用 `i18n::store_error` 转成提示文案。
  - 开发用环境变量（`src/paths.rs`）：`CYBERAPI_DATA_DIR` 覆盖数据目录（试运行时指向临时目录，避免写入真实数据），`CYBERAPI_LEGACY_DIR` 覆盖 Tauri 版的数据目录。默认数据目录是平台数据目录下的 `CyberAPI`（Linux 为 `cyberapi`），数据库为 `cyberapi.db`。
- `crates/cyberapi-core`：L0 领域层。model（项目 / 目录 / 接口 / 请求 / 变量，含旧库字段的转换）、`tree`（构建、关键字过滤后的显示行、移动 / 插入 / 删除的变更计划、拖放落点）、`search`（拼音匹配）、`collection`（首页过滤与排序）。无 IO、无时钟、无 gpui，必须能编译到 wasm。新的纯逻辑放这里，不放 GUI crate。树的规则只在这里写一份：store 按 `plan_*` 的结果写库，state 用同一个计划 `apply` 到自己的副本。
- `crates/cyberapi-store`：L1 存储接缝。`CollectionStore` / `TreeStore` / `VariableStore` / `PrefStore` 四个 trait（返回 `StoreFuture`，即 `'static` 的 boxed future）、`StoreError`（可序列化，浏览器版原样收到 bridge 的错误）、`Storage`（四个 `Arc<dyn …>` 的组合）、`rules`（各后端共用的校验）、`MemoryStore`（参考实现，仅 native）。trait 本身能编译到 wasm。
  - 新增或修改存储行为时，先在 `conformance` 里写检查，再改各后端；每个后端的测试都要跑 `conformance::run(&storage, &probe)`。
  - 新增一条检查后，要故意改坏 `MemoryStore` 的对应行为，确认这条检查会失败。检查不出问题的检查没有意义。
  - 后端返回的记录必须和它实际写入的一致（包括时间只精确到秒），套件会把返回值 `apply` 到副本后与 `load` 的结果比较。
- `crates/cyberapi-store-turso`：桌面版后端 `TursoStore`（仅 native，GUI 在 native 依赖表里引用）。
  - 旧版的 5 张表按原样创建，旧列的格式（逗号分隔的 `children`、`"1"`/`"0"`、文本时间）保持不变，所以 Tauri 版的 `my_db.db` 可以直接打开。
  - 本应用新增的表是 `schema.rs` 的 `MIGRATIONS`，按版本号记在 `cyberapi_meta.schema_version`。**只能追加，已发布的步骤不能改**。比当前程序更新的库拒绝打开。
  - 一个连接（`async_lock::Mutex`）串行处理所有调用；多行修改放在一个事务里。
  - `copy_legacy_data` 在首次启动时把 Tauri 版的数据**复制**过来。turso 打开文件时会把它切到 WAL 模式，所以绝不能直接打开 Tauri 版的原文件。
  - 用真实旧库测试时，用一份**副本**跑 ignored 测试 `opens_a_real_tauri_database`（`CYBERAPI_LEGACY_DB=副本路径`）。
  - turso 依赖要设 `default-features = false`，否则它会把 mimalloc 装成全局分配器。
- `crates/cyberapi-store-sql`：bridge 的后端 `SqlStore`（仅 native），sqlx `Any` 同一套代码跑 PostgreSQL / MySQL（`docs/architecture.md` §4.4）。
  - `SqlStore::scoped(account)` 返回该账号视角的 `Storage`：每次调用先按 §9.3 算出角色（`cyberapi_core::access::effective_role`），看不到的项目一律 `NotFound`，角色不够返回 `Forbidden`。账号、团队、成员、授权的管理方法是 `SqlStore` 的固有方法（`create_account` / `create_team` / `set_member` / `set_grant`）。
  - SQL 只写 PostgreSQL 的 `$1, $2…` 一份（`src/sql.rs`），MySQL 运行时换成 `?`；**每条语句的占位符必须从 1 开始按出现顺序编号**，测试 `placeholders_count_up_from_one_in_order` 会检查，新语句要加进它的 `ALL` 列表。两边写法不同的（upsert）写成 `Pair`。
  - 迁移在 `migrations/{postgres,mysql}/`，两套逐列对应，只追加；MySQL 的每条语句都要能重复执行（它的 DDL 不在事务里）。名称类排序在 Rust 里做，不依赖数据库的 collation。
  - 本地测试库：`make db-up` 用 Apple 的 `container` 起 PostgreSQL（127.0.0.1:55432）和 MySQL（127.0.0.1:53306），`make test-sql` 跑一致性测试和授权测试；没设置 `CYBERAPI_TEST_POSTGRES` / `CYBERAPI_TEST_MYSQL` 时这些测试跳过。改动 SQL 后端时必须跑 `make test-sql`。
- `crates/cyberapi-store-bridge`：bridge 协议的两端（能编译到 wasm）。每个 Store trait 方法对应一个 `Call`，统一 POST 到 `/v1/store`；服务端用 `dispatch(storage, call)` 执行，浏览器端用 `BridgeStore` 经 `Transport`（页面提供的 HTTP 客户端）发出。store 的拒绝（`StoreError`）作为正常应答 `{"err": …}` 原样传回；HTTP 状态码只表示传输层问题（401 未登录）。**给 Store trait 加方法时，同时在 `Call`、`dispatch`、`BridgeStore` 三处加上**，否则浏览器版用不了。
- `cyberapi-bridge`：浏览器版的服务端（axum，仅 native），不依赖 GUI crate。负责初始化管理员（setup token，只在没有任何账号时有效）、登录 / 登出（Argon2id，会话 cookie `HttpOnly; SameSite=Strict`，库里只存 token 的 sha256）、`/v1/store`（在登录账号的 `scoped` 存储上执行），并托管 `cyberapi-web/www`。测试每次新建一个 PG 数据库（`tests/bridge.rs`），其中一条通过内存 `Transport` 直接调用 Router，对 `BridgeStore` 跑一致性测试。本地运行：`make db-up && make web-bundle && make bridge`，打开 http://127.0.0.1:7380/，初始化 token 是 `dev`。
- 浏览器版的登录在 `www/index.html` 里完成：先查 `/v1/session`，未登录时按 `/v1/setup` 显示初始化或登录表单，登录后取 `prefsApp` 再加载 wasm，调用 `run(origin, ui_font, prefs)`。`cyberapi-web` 用 `FetchTransport`（gpui 的 fetch 客户端）创建 `BridgeStore`，收到 401 时刷新页面重新登录；`assets.rs` 是从 zedis 移植的图标重绘观察器。
- `cyberapi-web`：浏览器入口。把 GUI crate 编译成 wasm，只负责平台、UI 字体和资源来源；通过 `cyberapi_gui::launch` 进入应用。有自己的 `rust-toolchain.toml`（nightly + wasm32），必须在该目录下构建。`www/index.html` 拉取 `fonts/IBMPlexSans.ttf` 和 wasm 后调用 `run(origin, ui_font)`。
- UI 只依赖 `gpui-kit` 一个 crate：GPUI 是 `gpui_kit::*`，组件是 `gpui_kit::component`，图标是 `gpui_kit::assets`，平台是 `gpui_kit::platform`（wasm 上还有 `gpui_kit::web`）。不要再单独引入 `gpui-pre-*`。
  - **锁文件的两个精确版本**：gpui-kit 可选的 GPUI Fast 后端把 `cocoa` 锁在 `=0.26.0`、`unicode-properties` 锁在 `=0.1.3`，而锁文件也要满足可选依赖。不要用 `cargo update` 把这两个包升上去；打开 gpui-kit 的新 feature（例如 `test-support`）时如果报 “failed to select a version for `cocoa`”，说明锁文件里是更高的版本，需要重新生成锁文件（`cargo generate-lockfile`）。

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

完整设计（crate 划分、Store trait、turso / sqlx、view 拆分、实施步骤、待决定项）见 `docs/architecture.md`。

按 `docs/gpui-refactor-feature-analysis.md` 第 3 节：L0 Domain（纯函数，无 IO）→ L1 Infrastructure（trait + 实现）→ L2 Services → L3 State → L4 View（GPUI）。随功能落地拆出 `crates/*`（例如 L0 进 `crates/cyberapi-core`，不依赖 gpui，必须能编译到 wasm），届时把 `crates/*` 加入 workspace `members`。

## GPUI

- 启动顺序：先 `gpui_kit::init(cx)`，再用 `gpui_kit::open_window` 开窗（它会挂上 `Root`，弹窗、通知、tooltip 依赖它）。
- 不要臆造 API：以 `~/.cargo/registry/src/*/gpui-kit-0.8.0`、`gpui-component-0.8.0`、`gpui-pre-0.3.9` 的源码为准。
- 需要 `#[derive(IntoElement)]` 等派生宏时，在 `src/lib.rs` 加 `extern crate gpui_kit as gpui;`，宏展开出的 `gpui::…` 路径才能解析。
