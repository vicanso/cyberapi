# CyberAPI GPUI 架构设计

> 状态：已定稿（决策见第 10 节）。实现过程中如有变化，同步更新本文。
> 待实现的服务器项目同步和邀请注册见 [server-sync.md](./server-sync.md)。本文其余部分描述已经落地的行为。
> 功能编号（`F<模块>.<序号>`）和问题编号（`#N`）引用自 [gpui-refactor-feature-analysis.md](./gpui-refactor-feature-analysis.md)。
> 工程组织参照 `~/github/zedis`（ADR 9 / 10 / 17 / 18）。

---

## 1. 目标与约束

1. 同一份代码同时构建**桌面版**和**浏览器版**。
2. 桌面版的本地数据库用 **turso**，服务器端用 **sqlx**，同时支持 MySQL 和 PostgreSQL。
3. 数据的保存统一做成 **trait**，以后切换实现时不影响上层代码。
4. 参照 zedis 拆分 view，让 web 版和 GPUI 桌面版共用同一套 states 和 views。
5. 浏览器版支持**多账号**：账号属于团队，项目属于团队，项目也可以单独授权给账号（第 9 节）。

## 2. 总体思路

### 2.1 沿用 zedis 的部分

- **一份 GUI 代码，两个目标。** states 和 views 只写一份。浏览器版就是同一个 GUI crate 编译成 wasm，再去掉部分能力。`cyberapi-web` 保持轻薄，只负责装入实现，然后调用 `cyberapi_gui::launch`。
- **默认代码是桌面代码，浏览器版做减法。** 规则见根目录 `CLAUDE.md` 的“一份代码，两个目标”。
- **bridge 只帮浏览器做它做不到的事。** 浏览器受 CORS 限制，`Cookie`、`User-Agent`、`Host` 等是禁止设置的请求头，也拿不到 DNS、TCP、TLS 的分阶段耗时（F5.6），所以 HTTP 请求由 bridge 代发（D5）。请求的组装（变量注入、模板函数、body 编码）仍在 GUI 里完成，bridge 只发送已经组装好的请求。这和 zedis 的 bridge 只转发 RESP、不理解具体命令是同一个思路。

### 2.2 和 zedis 有意不同的部分：统一的 Store trait

zedis 的三层各用一种存储机制：

| 层 | 机制 |
|---|---|
| 桌面 | redb，wasm 构建时用 `mem_store` 按 cfg 替换 import |
| 浏览器 | `OnceLock<Arc<dyn Trait>>` 全局插槽 |
| bridge | `Arc<dyn Store>` trait |

CyberAPI 的数据就是业务本身（项目、接口树、变量、历史），所以只定义**一套 Store trait**，提供四个实现：

| 实现 | 运行位置 | 底层 |
|---|---|---|
| `TursoStore` | 桌面 | turso，兼容旧版的 `my_db.db` |
| `SqlStore` | bridge 服务器 | sqlx `AnyPool`，PostgreSQL / MySQL，按账号鉴权 |
| `BridgeStore` | 浏览器 | 用 HTTP 调用 bridge 的 REST API，bridge 内部再调 `SqlStore` |
| `MemoryStore` | 测试 | 内存，作为行为基准 |

启动时把选好的实现装进 gpui Global。states 只依赖 trait，因此 **states 和 views 里不需要任何与存储有关的 `cfg`**。

## 3. Crate 划分与依赖方向

```
crates/
  cyberapi-core          L0  纯逻辑：model、tree（构建 / 过滤 / 移动 + 环检测）、template 解析、
                             request_builder、response、curl、importers、拼音搜索、权限判定        [wasm ✓]
  cyberapi-store         L1  Store trait、StoreError、MemoryStore、一致性测试套件                  [wasm ✓]
  cyberapi-store-turso   L1  桌面实现（turso）                                                      [仅 native]
  cyberapi-store-sql     L1  服务器实现（sqlx，PostgreSQL / MySQL，含账号 / 团队 / 授权）            [仅 native]
  cyberapi-http          L1  HttpEngine trait（wasm ✓）+ native 引擎（hyper，按请求计时、
                             drop 即取消、Cookie 注入与接收）                                         [引擎仅 native]
  cyberapi-service       L2  用例：发送管线、导入、导出、备份。只依赖 trait                          [wasm ✓]
  cyberapi-ui                通用控件，对应 zedis-ui：不使用 i18n，文案由调用方传入                  [wasm ✓]
cyberapi-gui（根 crate） L3 states + L4 views；启动时装配实现
cyberapi-web             wasm 入口：装入 BridgeStore / BridgeHttpEngine，调用 launch
cyberapi-bridge          axum 服务：托管页面、登录、REST 存储 API、/v1/send 代发请求
```

依赖规则：

- 方向为 `core ← store / http ← service ← gui ← web`，以及 `bridge → core + store-sql + http（native）+ service`。
- bridge **永远不依赖 gui**，因为 gui 会链接窗口系统。
- `crates/*` 和 GUI crate 不得引用 `cyberapi-web` 或 `cyberapi-bridge`。
- `turso`、`sqlx`、`hyper` 只出现在各自 crate 的 native 依赖表中，wasm 构建在依赖层面就接触不到它们。

## 4. 存储层

### 4.1 Trait 的形状

```rust
// 需要 `Arc<dyn …>` 在运行时切换实现，而原生的 async fn in trait 不支持 dyn，
// 所以统一返回 boxed future。
pub type StoreFuture<T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'static>>;

pub trait CollectionStore: Send + Sync {
    /// 当前身份可见的项目，每项附带当前身份的角色（桌面版恒为 Manager）。
    fn list(&self) -> StoreFuture<Vec<Collection>>;
    fn get(&self, id: CollectionId) -> StoreFuture<Collection>;
    /// `owner` 为归属团队；桌面版忽略，浏览器版为空时归入个人空间。
    fn create(&self, input: NewCollection) -> StoreFuture<Collection>;
    fn update(&self, id: CollectionId, edit: CollectionEdit) -> StoreFuture<Collection>;
    /// 级联删除目录、接口、变量、各账号的界面状态（修正 #5）。
    fn delete(&self, id: CollectionId) -> StoreFuture<()>;
}

pub trait TreeStore: Send + Sync {
    /// 目录、接口和顶层顺序。
    fn load(&self, collection: CollectionId) -> StoreFuture<TreeData>;
    /// 新建的项与它造成的变更（`Inserted { item, change }`），调用方把 change 应用到自己的副本。
    fn create_folder(&self, collection: CollectionId, parent: Option<ItemId>, name: String) -> StoreFuture<Inserted<Folder>>;
    fn create_api(&self, collection: CollectionId, parent: Option<ItemId>, name: String) -> StoreFuture<Inserted<ApiSetting>>;
    fn rename(&self, collection: CollectionId, item: ItemId, name: String) -> StoreFuture<()>;
    fn save_request(&self, collection: CollectionId, api: ItemId, request: HttpRequest) -> StoreFuture<ApiSetting>;
    fn move_item(&self, collection: CollectionId, item: ItemId, to: Placement) -> StoreFuture<TreeChange>;
    fn delete(&self, collection: CollectionId, item: ItemId) -> StoreFuture<Removal>;
    /// 整份项目配置（名称、说明、目录、接口、顶层顺序、三类变量）和当前修订号，服务器项目拉取时用。
    fn config(&self, collection: CollectionId) -> StoreFuture<ConfigAt>;
    /// 在一个事务里用整份配置替换项目：id 原样保留，修订号必须等于 `base`（`None` 为强制），
    /// 否则返回 `Stale` 且不写入；写完修订号加一。编辑者可调用，名称和说明仍只由管理者修改。
    fn replace(&self, collection: CollectionId, config: ProjectConfig, base: Option<u64>) -> StoreFuture<u64>;
}

pub trait VariableStore: Send + Sync {
    /// 按名称排序（M8）。
    fn list(&self, collection: CollectionId, category: VariableCategory) -> StoreFuture<Vec<Variable>>;
    fn create(&self, input: NewVariable) -> StoreFuture<Variable>;
    /// 按 ID 对应（修正 #26）。
    fn update(&self, collection: CollectionId, id: VariableId, edit: VariableEdit) -> StoreFuture<Variable>;
    fn delete(&self, collection: CollectionId, ids: Vec<VariableId>) -> StoreFuture<()>;
}

/// 个人数据：应用设置（4.7）与每个项目的工作区状态（4.6，选中项按项目保存，修正 #20）。
pub trait PrefStore: Send + Sync {
    fn app(&self) -> StoreFuture<AppPrefs>;
    fn set_app(&self, prefs: AppPrefs) -> StoreFuture<()>;
    fn workspace(&self, collection: CollectionId) -> StoreFuture<WorkspacePrefs>;
    fn set_workspace(&self, collection: CollectionId, prefs: WorkspacePrefs) -> StoreFuture<()>;
}

/// 一个后端的全部 Store。一个后端实现全部 trait 时用 `Storage::from_backend(Arc::new(backend))`。
#[derive(Clone)]
pub struct Storage { /* 每个 trait 一个 Arc<dyn …> */ }
```

以上是第 2 步已经落地的部分（`crates/cyberapi-store`）。其余 trait 在用到它们的那一步再加，避免提前设计：

| Trait | 内容 | 加入的步骤 |
|---|---|---|
| `DirectoryStore` | 账号、团队、授权（9.4 节），只在浏览器版存在，不放进 `Storage` | 第 5 步 |
| `HistoryStore` | 每个接口保留 10 条响应历史（F7.9 / F7.11），依赖响应模型 | 第 7 步 |
| `CookieStore` | 列表 / 新增 / 编辑 / 删除 / 清空（M9） | 第 8 步 |
| `TreeStore::import`、`BackupStore` | 导入（整体重新映射 ID，修正 #7 / #9）；导出快照，在一个事务内恢复（修正 #8） | 第 9 步 |

### 4.2 设计要点

- **按意图建模，不暴露存储细节。** trait 提供的是 `move_item`，而不是“修改某个目录的 children”。实现内部的流程是：开启事务 → 读取快照（服务器端对相关行加 `FOR UPDATE`）→ 调用 `cyberapi_core::tree::plan_move`（纯函数，含环检测）→ 写回有变化的行。turso 和 sql 两个实现共用同一套算法，并借此修正以下问题：
  - #4：移动时形成环
  - #5：删除后留下悬空引用
  - #7：导入时 ID 冲突（整体重新映射 ID）
  - #9：导入项没有进入顶层排序
- **树的存储表示（D3）。** 所有后端都使用“目录的 `children` 列表 + 项目的顶层顺序”，移动算法只在 L0 写一份。
  - 桌面版沿用旧表的逗号分隔字符串。
  - 服务器把 children 存为 JSON 列。
  - **顶层顺序属于树结构**，由 `TreeStore` 负责，不再是界面状态：在服务器上由团队成员共享；在桌面版存入新表，首次迁移时从旧数据中补全。
- **共享数据与个人数据分开。**
  - **项目共享**（同一项目的成员看到同一份）：项目信息、目录、接口、变量、顶层顺序。
  - **按账号区分**（仍在库里的 `prefs`、响应历史、Cookie）：展开状态、选中项、活动 Tab、响应历史、Cookie、应用设置（含桌面窗口在各屏幕上的位置）。固定接口和 JSON 的默认展示不在库里，见 4.5。
  - 响应历史可能包含 token，所以只有本人可见。
- **每个修改项目内容的方法都带上项目 ID。** 树、请求、变量的方法都要求传入所属项目；不属于该项目的条目一律按“不存在”（`NotFound`）处理，和真的不存在没有区别。这样服务器只需要检查一次项目权限，也不会通过 ID 越权访问别的项目。
- **内容修改会刷新项目的修改时间（#21）。** 树、请求、变量的任何修改都会更新项目的 `updated_at`，所以首页“最近修改”排序反映的是整个项目的修改。个人数据（`PrefStore`）不算内容修改。
- **名称在存储层校验。** 项目、目录、接口的名称去掉首尾空白后不能为空，否则返回 `NameRequired`。规则写在 `cyberapi_store::rules`，每个后端调用同一个函数。
- **参数和 future 都拥有所有权，生命周期为 `'static`。** 既能直接交给 gpui 的 `background_spawn`，也方便跨 HTTP 序列化。trait 的方法清单就是 bridge REST API 的清单，数量有界。
- **身份在构造时绑定。** 服务器按登录账号构造 `SqlStore::scoped(account)`：
  - trait 方法本身不带账号参数，所以桌面版和服务器使用完全相同的 trait。
  - 每个方法内部按第 9 节判定权限，不满足时返回 `StoreError::Forbidden`。
  - 桌面版是唯一的本地用户，不做鉴权。
- **测试分两套。**
  - **一致性测试套件**：参照 zedis 的 `store/conformance.rs`，同一组用例分别运行在 Memory、Turso、PostgreSQL、MySQL 上，保证切换实现后行为不变。套件在 `cyberapi_store::conformance`（feature `conformance`），后端的测试调用 `conformance::run(&storage, &probe)`。
    - 有些规则通过 trait 看不出来，例如项目删除后，所有方法对它都返回 `NotFound`，无论它的行是删干净了还是残留着。为此每个后端实现一个只供测试用的 `Probe`，报告某个项目还剩多少行。
    - 套件的每条检查都会故意改坏 `MemoryStore` 的对应行为来验证，确认这条检查能发现问题。
  - **授权测试**：只运行在 `SqlStore` 上，覆盖第 9 节的每条规则。

### 4.3 桌面：turso

- 版本为 0.8.3（2026-10 的最新稳定版）。异步 API，不绑定特定运行时，在 gpui 的后台 executor 上运行。支持事务和批量执行。
- 必须设置 **`default-features = false`**：turso 的默认 feature 会把 mimalloc 装成全局分配器。
- 旧版的 5 张表（`api_collections`、`api_folders`、`api_settings`、`variables`、`versions`）**保持原结构**。Domain 层负责转换：`enabled` 的 `"1"` / `"0"` 转为 `bool`，字符串时间转为时间类型（#24）。
- 原来存放在 localforage 中的数据（设置、展开状态、置顶、活动 Tab、响应历史）以及顶层顺序、Cookie 改为新表，通过本应用自己的迁移来创建。
  - 迁移的版本号记在新表 `cyberapi_meta.schema_version` 中，不使用旧版的 `versions` 表；旧版的 `versions` 表原样保留，不再写入。
  - 迁移 1（第 3 步）创建 `top_orders`（顶层顺序，逗号分隔，与 `children` 相同）、`prefs`（个人文档，`app` 与 `workspace/<项目 ID>`），并为按项目的查询加索引。
  - 数据库的 schema 版本比当前程序更新时拒绝打开，不用旧规则改写新数据。
- 一个连接串行处理所有调用（桌面版只有一个用户），涉及多行的修改放在一个事务里，要么全部完成，要么全部不生效。
- 旧库里的行按 `rowid` 排序读取，即插入顺序，与旧版读出的顺序一致。旧版没有保存顶层顺序（它在 localforage 中，不迁移），所以复制过来的旧库第一次打开时顶层按“接口在前、目录在后”排列，和旧版清空缓存后看到的一样。
- turso 仍在快速迭代，这也是坚持使用 trait 的原因之一：如果遇到兼容问题，可以增加一个 rusqlite 实现来兜底。
- 兼容性测试使用一份真实旧库的**副本**，不修改原文件（ignored 测试 `opens_a_real_tauri_database`）。
  - 在一份真实旧库上验证过：21 个项目、162 个目录、628 个接口全部读出，每一项都出现在对应项目的树上。
  - 表里 129 个变量中有 90 个属于 10 个已经删除的项目，这是旧版 #5 留下的残留。它们在新版中同样不可见，暂不清理。

#### 旧数据迁移（D2、D4）

- **新的数据目录与 Tauri 版分开。** 目录名在实现时确定，但不能在大小写不敏感的文件系统上与 `com.bigtree.cyberapi` 撞名。
- **首次启动时复制旧数据**（`cyberapi_store_turso::copy_legacy_data`）。条件是新目录里还没有数据库，并且在 Tauri 1 的 `app_data_dir`（`tauri_data_dir()`：平台数据目录下的 `com.bigtree.cyberapi`，macOS 为 `~/Library/Application Support/com.bigtree.cyberapi/`）找到 `my_db.db`。此时：
  - **复制**数据库及其 `-wal` / `-shm` 文件，不移动、不修改原文件，所以旧版仍可继续使用。turso 打开文件时会把它切换为 WAL 模式，这是绝不能直接打开原文件的另一个原因。
  - 数据库先复制成临时文件名，最后再改名。复制中断时不会留下一个被误当成数据库的半截文件。新数据库一旦存在，就不会再复制。
  - `cookies.json` 先原样复制到新目录的 `legacy-cookies.json`。第 8 步实现 `CookieStore` 时再导入 turso 的 Cookie 表：cookie_store 的 JSON 由导入代码解析，只导入持久化 Cookie，导入后打标记防止重复。
- **localforage 中的数据不迁移**（见分析文档 4.6 / 8.3）。

### 4.4 服务器：sqlx（照搬 zedis ADR 18）

- sqlx 0.9，使用同一个 `AnyPool` 代码路径。每条语句写成 `Stmt { pg, my }` 两种 SQL，SQL 相同的用 `both!` 宏。方言由 `backend_name()` 识别。
- 迁移按方言分目录：`migrations/postgres`、`migrations/mysql`，用 `sqlx::migrate!` 内嵌到二进制，只增不改。
- MySQL 强制使用 `utf8mb4_bin`。
- 用 `--store postgres://…` 或 `--store mysql://…` 在**运行时**选择后端，不做成 cargo feature。
- 服务器的表结构**不需要兼容旧版**，可以使用正规的类型、外键和索引。表结构只需和 trait 的语义一致，与桌面版的表结构无关。

### 4.5 库里的数据、本机文件、只在内存里的状态

桌面版的库是本机 SQLite，文件名在数据目录里：发布版 `cyberapi.db`，`make dev`（debug）用同目录的 `cyberapi-dev.db`，不打开已安装应用的库。默认目录是平台数据目录下的 `CyberAPI`（Linux / FreeBSD 为 `cyberapi`）；macOS 为 `~/Library/Application Support/CyberAPI/`。`CYBERAPI_DATA_DIR` 只改这个目录，不改文件名。浏览器版没有这份文件，库在 bridge 的 PostgreSQL 或 MySQL 上，按登录账号隔离。

**数据库。** 项目内容团队成员看得到：项目、目录、接口、变量、顶层排序。读者自己的、不跟团队共享、但仍然进库：

| 内容 | 位置 |
|---|---|
| 主题、语言、超时、分栏宽度、上次打开的项目、首页分组与折叠 | `prefs` 的 `app` 文档 |
| 窗口大小、是否最大化，以及每块屏幕上一次的位置和大小 | 同上。只由桌面版写入；浏览器不记窗口 |
| 每个项目展开的目录、选中的接口、各接口停在哪个标签 | `prefs` 的 `workspace/<项目 ID>` |
| 响应历史 | `responses` |
| Cookie | `cookies`。桌面版首次启动把复制来的 `legacy-cookies.json` 导入此表，然后把文件改名为 `legacy-cookies.imported.json` |
| 账号、团队、成员、项目授权 | 只在 bridge 的库里。桌面版没有这些表 |

工作区文档里仍有 `pinned` 字段，旧文档才能解析。保存时先把固定列表写到本机，再把这个字段清空后写入库。本机写失败时列表留在文档里，避免丢掉。

**本机文件，不进库，也不进备份。** 桌面版放在数据目录、数据库旁边；浏览器版用 `localStorage`，按账号 id 分开（桌面只有一个读者，范围是空字符串）。

| 内容 | 桌面 | 浏览器 |
|---|---|---|
| 固定的接口（按项目，顺序即固定顺序） | `pins.json` | `cyberapi.pins` |
| JSON 默认以文本还是树打开 | `json-view.json` | `cyberapi.json_view` |

固定列表：本机还没有这份记录时，若库里的旧文档带有非空 `pinned`，采用一次，之后以本机为准。本机已是空列表表示读者全部取消固定，不再从旧文档填回。清除工作区状态时一并清掉该读者的固定列表；清除历史不清。删除项目时去掉该项目的固定列表。查看者也可以固定。

JSON 默认展示：恢复默认设置时写回「文本」，再保存其余设置。清除工作区或历史不动这个文件。

**只在内存。** 某个响应或某条请求上把 JSON 改成文本或树，只对当前这一次有效：响应跟这条响应走，换一条响应即失效；请求跟当前打开的接口走，换接口即失效。不写 `json-view.json`，也不改设置里的默认。

## 5. HTTP 执行

- 定义 `HttpEngine` trait，输入是组装完成的请求，输出是响应和耗时统计。drop 对应的 future 即取消请求，修正“中止只是丢弃结果”的问题（#2）。
- **native 引擎**（`cyberapi-http`）：
  - 基于 hyper，用自定义 connector 按请求计时，替代现在解析 tracing 日志的做法（#1）。
  - 明确请求头优先级：请求自身 > 全局请求头 > 默认值（#3）。
  - 对响应头做宽松解码（#11）。
  - 桌面版在私有的 tokio 运行时上运行，bridge 直接在 axum 的 tokio 上运行。
- **浏览器（D5）**：`BridgeHttpEngine` 把请求 POST 到 bridge 的 `/v1/send`，由 bridge 调用 native 引擎执行。
  - 行为和桌面版一致。代价是请求从服务器发出，访问不到用户本机的 `localhost`。
  - Cookie 使用发起请求的账号自己的 Cookie。
  - 以后可以再增加可选的“浏览器直连”模式（受 CORS 限制）。
  - bridge 需要限制可访问的目标地址，防止被当作 SSRF 跳板，例如默认拒绝内网和元数据地址，管理员可配置白名单。
- **发送管线**（L2 `cyberapi-service`）：变量注入（F5.2）、body 处理（F5.3）、Query / Header 处理（F5.4）、模板函数（M6）、写入历史（F5.8）。两端共用同一份代码。
- **模板函数的 IO 依赖**通过 trait 注入：`FileReader`、`FilePicker`、`ResponseLookup`、`VariableLookup`、`Clock`。浏览器版不提供 `readFile` 和 `openFile`。

## 6. View 拆分（照 zedis）

### 6.1 目录职责

| 目录 | 内容 |
|---|---|
| `src/states/` | gpui Entity，是数据的唯一来源：`AppState`（Global）、`DashboardState`、`WorkspaceState`（每个项目一个，含树、置顶、变量）、`RequestEditorState`、`ResponseState`、`SendState`；浏览器版另有 `AccountState`（当前账号、团队、授权） |
| `src/views/` | 只负责绘制和转发事件：dashboard、workspace（树 / 请求编辑 / 响应）、dialogs（设置、变量、Cookie、导入、缓存）；浏览器版另有账号与团队管理页 |
| `src/components/` | 依赖应用状态或 i18n 的共享组件 |
| `src/helpers/` | 快捷键、格式化、`this_build.rs` 能力查询等 |

### 6.2 数据流

1. view 调用 `state.update(cx, |s, cx| s.method(cx))`。
2. state 用 `cx.spawn` + `background_spawn` 调用 service 或 Store。
3. 回到主线程更新 state 的字段。
4. `cx.emit(...)` + `cx.notify()`。
5. 订阅了该 state 的 view 重新渲染。

### 6.3 规则

- **views 不直接访问 Store 或 HttpEngine，只能通过 states。** 参照 zedis 的 `tests/view_layering.rs`，用测试统计 `src/views` 中的违规调用来强制执行。
- 原版需要放进 state 的逻辑：
  - 请求编辑的防抖自动保存，以及“修改发生时的接口 ID 必须等于当前选中的 ID”这一保护（F4.11）
  - 发送 / 中止状态机和 reqId 去重（F5.1）
- **权限影响界面，但不由界面判定。** 项目附带当前身份的角色。界面按角色决定是否显示入口：只读时隐藏编辑、删除、拖拽和导入，请求编辑区只读。真正的拦截在 `SqlStore` 中完成。桌面版的角色恒为 Manager，所以这些判断在桌面版上不起作用，也不需要 `cfg`。

### 6.4 平台差异

- **能力查询**：`helpers/this_build.rs` 提供 `saves_files()`、`opens_files()`、`has_file_dialogs()`、`reads_clipboard_on_demand()`、`has_accounts()` 等。每个都是一对同签名、分别加 gate 的函数，桌面版在前，并有 native 测试固定桌面版的返回值。
- **文件读写成对实现**：桌面版写入下载目录或弹出选择框，浏览器版用下载 / 上传代替。
- **弹窗**：原版的设置、变量、Cookie 等本来就是弹窗，所以浏览器版不需要第二个窗口。
- **浏览器版专有**：登录页（由 `index.html` 负责，和 zedis 一样）、账号与团队管理、项目授权、新建项目时选择归属团队、退出登录。这些放在一个 wasm 专用模块中，整体用一个 gate。
- **浏览器版不提供的能力**：
  - 模板函数 `readFile` / `openFile`
  - Multipart 选择本地文件
  - 附件自动保存（F7.5）：改为下载
  - 窗口尺寸设置（F1.3）
  - zip 备份与恢复（M13）

### 6.5 i18n

- rust-i18n + 懒加载，语言为 en / zh / uk（F15.2）。
- 桌面版内嵌语言文件，浏览器版运行时 fetch。
- GPUI 可以直接切换语言，不再需要“3 秒后重启”（F1.8）。

## 7. 多人协作下的数据一致性（浏览器版）

同一个项目可能被多人同时编辑，需要处理两件事：写入冲突，以及看到别人的修改。

- **结构性修改在事务内完成。** 移动、新建、删除、导入都在服务器的事务中执行“读取快照 → `plan_move` → 写回”，并对涉及的目录行加 `FOR UPDATE`，不会丢失并发修改。
- **接口内容采用后写覆盖。** `save_request` 只写单个接口，覆盖范围很小，所以采用后写覆盖。如果实际使用中冲突明显，再加版本号做乐观锁。
- **项目修订号。** 每个项目维护一个修订号，任何共享数据变化都加一。页面打开某个项目时轮询修订号，参照 zedis 的 `pacing`：前台约 10 秒一次，后台标签页放慢；发生变化时重新加载树和变量。正在编辑的接口保留本地内容，不被覆盖。

## 8. 实施步骤

先打通一个**纵向切片**来验证 trait 设计，再横向补齐功能，避免 trait 全部写完才发现不适合通过 HTTP 传输。

| 步 | 内容 | 验证方式 |
|---|---|---|
| 1 | 拆分 workspace；`cyberapi-core` 的 model 和 tree（构建 / 过滤 / 移动 + 环检测） | 单元测试固化现有行为 |
| 2 | `cyberapi-store`：trait、错误类型、`MemoryStore`、一致性测试套件 | 套件在 Memory 上通过 |
| 3 | `cyberapi-store-turso`：兼容旧表 + 新表迁移 + 旧数据复制（D2、D4） | 一致性测试；读取旧库副本 |
| 4 | GUI 骨架：Storage Global、i18n、Dashboard（项目增删改、过滤、排序） | 桌面版可用 |
| 5 | **纵向切片**：`cyberapi-store-sql` + `cyberapi-bridge` + `BridgeStore`，只覆盖“项目”。账号、团队、授权的**表结构**在这一步就定下来；只实现最小的账号能力（setup token 创建管理员、登录、自动创建个人空间），保证 `scoped(account)` 从一开始就生效。浏览器版补上 zedis 的图标重绘观察器（`repaint_when_icons_land`）：图标是异步下载的，下载完成后没有东西触发重绘，第 4 步时空状态的图标就没显示出来 | 一致性测试跑 PostgreSQL / MySQL（docker）；授权测试；浏览器版 Dashboard 可用 |
| 6 | Workspace：树、展开、拖拽移动、置顶、偏好设置；浏览器版的修订号轮询 | 两端 |
| 7 | `cyberapi-http` 引擎 + 发送管线 + 请求编辑 + 响应展示 + 历史；bridge 的 `/v1/send` 和目标地址限制 | 两端 |
| 8 | 变量、Cookie、设置弹窗 | 两端 |
| 9 | 导入（cURL / Postman / Insomnia / Swagger / 自有格式）、导出、备份 | 用样例文件测试各 importer |
| 10 | 浏览器版的账号与团队管理：邀请账号、团队增删和成员管理、项目授权管理界面 | 授权测试；两个账号的端到端测试 |
| 11 | 部署打包：bridge 内嵌页面，产出发布包 | `make web-dist` / `make desktop-dist` |

第 5 步落地时的具体做法（与上表的设想一致，补充细节）：

- **协议只有一个存储端点。** 每个 Store trait 方法对应 `cyberapi_store_bridge::Call` 的一个变体，统一 POST 到 `/v1/store`，应答是 `{"ok": 值}` 或 `{"err": StoreError}`。trait 增加方法时只需加一个变体，不需要设计新路由。
- **登录在页面里完成，不在 wasm 里。** `index.html` 负责初始化 / 登录表单，登录成功后才加载 wasm，并把账号的设置随调用传入，第一帧就能用上语言和主题。
- **账号、团队、授权目前只有 `SqlStore` 的固有方法**（`create_account` / `create_team` / `set_member` / `set_grant`），供 bridge 和测试使用。面向用户的管理界面和 `DirectoryStore` trait 仍在第 10 步。
- **留到第 10 步加固的事项：** 登录频率限制；两个请求同时初始化管理员时可能各建一个（目前没有加锁）；会话过期行的清理；退出登录的界面入口（目前只有 `/v1/logout` 接口，会话失效时页面会自动回到登录表单）。

第 10 步的加固（前三项）：

- **初始化只成功一次。** `SqlStore::create_first_admin` 在一个事务里先插入 `store_meta` 的 `setup` 行（PostgreSQL `ON CONFLICT DO NOTHING`、MySQL `INSERT IGNORE`），再确认没有账号才建管理员。两个同时到达的初始化，后到的会等前一个提交，然后插不进去，返回 409。
- **登录频率限制**（bridge 进程内存，`throttle.rs`）：15 分钟内同一个用户名失败 5 次、同一个来源地址失败 30 次、初始化 token 试错 10 次，就返回 429 和 `Retry-After`，直到最早的一次失败满 15 分钟。用户名不区分大小写；登录成功清掉该用户名的计数。来源地址来自 `ConnectInfo`，放在反向代理后面时是代理的地址。
- **过期会话**：bridge 启动时和之后每小时删除 `expires_at` 已过的会话行。

第 7 步落地时的具体做法：

- **纯逻辑在 core，IO 在 service。** `cyberapi_core::template`（解析和求值，IO 需要的数据由调用方先按 `needs` 准备好）、`send::prepare`、`response`、`curl`；`cyberapi-service::send` 只依赖 trait，两端运行同一份发送管线。cURL 也走同一个准备过程，命令和实际发出的请求一致。
- **请求头优先级（#3）**：默认值（User-Agent）< 全局请求头 < Basic Auth < 请求自身的 header。
- **计时（#1）与取消（#2）**：`NativeEngine` 在 hyper 的连接级 API 上每个请求新建连接，DNS / TCP / TLS 各自计时，包装后的流记录写完请求和收到首字节的时间；drop future 即中止 tokio 任务、断开连接。每个解析到的地址分到一份连接超时，一个不通的地址不会耗尽全部时间。
- **历史记录**：`HistoryStore`，读者自己的（bridge 上按账号隔离），每个接口最近 10 条；写入不算项目修改。
- **bridge 的目标地址限制**：`TargetPolicy::PublicOnly` 在连接时检查解析后的地址（防 DNS rebinding），默认拒绝回环、私网、链路本地（含云元数据地址）、CGNAT、基准测试段（198.18.0.0/15，fake-IP 代理会把域名解析到这里）、IPv6 ULA，IPv6 中嵌入的 IPv4 按 IPv4 判断；`--allow-target` 放行网段。在使用 fake-IP 模式代理的机器上开发时需要放行 198.18.0.0/15。
- **暂未做**：响应体的流式处理（整个响应体仍一次读进内存）。后来补上的：Cookie（第 8 步）；附件自动保存（F7.5，桌面写入“下载”目录，浏览器版触发下载，从历史选中时不保存）；大响应体（#17）：历史只保存不超过 2 MiB 的响应体，更大的只记长度，面板最多显示 1 MiB 文本，并可以“保存响应体”。

第 11 步落地时的做法：

- **bridge 是浏览器版的全部部署。** release 构建用 `rust-embed` 把 `cyberapi-web/www` 编进二进制，页面和 API 同源（登录 cookie 保持 `SameSite=Strict`）。`scripts/web-bundle.sh --release` 同时生成 `.wasm.gz`，bridge 按 `Accept-Encoding` 发送；每个文件带内容哈希的 ETag 并要求重新验证（模块的文件名不随版本变）。debug 构建从磁盘读，开发时 `make web-bundle` 后无需重编 bridge。`make web-dist` 打出 `bridge + LICENSE + README.txt` 的 tarball。
- **桌面版用 cargo-bundle 出包**，沿用 Tauri 版的 identifier（`com.bigtree.cyberapi`），图标（`resources/icons/`）由 `icons/cyberapi.png` 生成（`make icons`），Windows 的 exe 由 `build.rs` 嵌入图标。macOS 的 `.app` 是 ad-hoc 签名，分发给他人还需要 Developer ID 签名和公证；Linux 出 `.deb`，Windows 目前只是 zip。包都在构建所在的平台上产出，不做交叉编译：CI 在三个平台上各自打包（`.github/workflows/package.yml`），版本 tag 挂到对应的 Release，`main` 每晚更新名为 `nightly` 的预发布。

## 9. 账号、团队与授权（浏览器版，D1）

桌面版没有账号概念。本节只适用于 bridge 和浏览器版。桌面登录 bridge、只同步项目配置的行为尚未实现，见 [server-sync.md](./server-sync.md)。

### 9.1 实体

| 实体 | 说明 |
|---|---|
| 账号（Account） | 用户名、Argon2id 密码哈希、显示名、是否为系统管理员、状态（启用 / 停用） |
| 团队（Team） | 名称；分为**普通团队**和**个人空间** |
| 团队成员（TeamMember） | `(team, account, role)`，团队角色为 `Owner` / `Admin` / `Member` |
| 项目（Collection） | 必须归属一个团队（`owner_team`） |
| 项目授权（Grant） | `(collection, grantee, role)`，`grantee` 可以是**账号**（单独授权）或**团队**（把项目授权给另一个团队） |

- **个人空间是一种特殊团队。** 每个账号注册时自动创建一个个人空间，只有本人一个成员，不能添加其他成员。这样“项目归属团队”在任何情况下都成立，个人项目不需要第二套规则。
- **团队管理权限。** 任何账号都可以创建普通团队，创建者成为 `Owner`。系统管理员负责管理账号（邀请、停用、重置密码），不会因此自动获得所有项目的访问权限。

### 9.2 项目角色

| 角色 | 能做什么 |
|---|---|
| `Viewer` | 查看树、接口、变量；**可以发送请求**（历史记在自己名下）；可以复制、导出 |
| `Editor` | 在 Viewer 基础上：编辑树（新建、重命名、移动、删除、导入）、编辑接口、编辑变量 |
| `Manager` | 在 Editor 基础上：修改项目信息、删除项目、管理授权、转移项目归属 |

### 9.3 有效角色的计算

某个账号对某个项目的有效角色，取以下各项中的**最高者**：

1. 在项目所属团队中的身份：团队 `Owner` / `Admin` → `Manager`；团队 `Member` → `Editor`。
2. 直接授予该账号的项目授权。
3. 授予该账号所在团队的项目授权。

什么都没有就是不可见：`list` 不返回这个项目，直接访问时返回 `NotFound`，不暴露项目是否存在。计算逻辑是 `cyberapi-core` 中的纯函数，单独测试；`SqlStore` 先查出上面三类事实，再调用这个函数得出角色。

### 9.4 `DirectoryStore`

账号、团队、授权的管理定义成单独的 trait，只有 `SqlStore` 实现、`BridgeStore` 转发，桌面版不装配：

```rust
pub trait DirectoryStore: Send + Sync {
    fn me(&self) -> StoreFuture<Me>;                                   // 当前账号及所在团队
    fn teams(&self) -> StoreFuture<Vec<Team>>;
    fn create_team(&self, name: String) -> StoreFuture<Team>;
    fn set_member(&self, team: TeamId, account: AccountId, role: Option<TeamRole>) -> StoreFuture<()>;
    fn grants(&self, collection: CollectionId) -> StoreFuture<Vec<Grant>>;
    fn set_grant(&self, collection: CollectionId, grantee: Grantee, role: Option<CollectionRole>) -> StoreFuture<()>;
    fn transfer(&self, collection: CollectionId, team: TeamId) -> StoreFuture<()>;
    // 账号的邀请、停用、重置密码只对系统管理员开放
}
```

第 10 步落地时的形状（`cyberapi-store`）：在上面的基础上加了团队的改名 / 删除、成员列表、账号列表（`accounts`，供选人）和管理员的 `create_account` / `update_account`（显示名、管理员、停用）/ `reset_password`，以及本人的 `change_password`。规则：

- 不在其中的团队一律 `NotFound`；个人空间不能加人、改名、删除。
- 改名要团队 `Admin` 以上，删除要 `Owner`，且团队不再拥有项目（先转移）；删除团队时一并删除授予它的授权。
- 成员：`Admin` 以上可以加人、改角色、移除，但只有 `Owner` 能设置或改动 `Owner`；任何人可以退出；最后一位 `Owner` 不能降级或退出（修改时锁住团队行，两个所有者同时退出也不会一个不剩）。
- 授权、转移要项目的 `Manager`；转移的目标团队必须是自己所在的团队。
- 账号管理只对系统管理员开放，每次调用重新读取管理员身份；管理员不能停用自己或取消自己的管理员身份；停用和重置密码会删除该账号的全部会话。

第 10 步的界面（浏览器版）：设置菜单里的团队（建 / 改名 / 删 / 退出，成员和角色）、账号管理（系统管理员：新建、管理员 / 停用开关、重置密码）、修改密码、退出登录；首页卡片菜单的“访问权限”（所属团队可转移，额外授权给账号或团队），新建项目时选团队，卡片显示所属团队和非管理者的角色。查看者在工作区里只读：不显示改树的操作，请求的修改不保存，变量弹窗只读。

### 9.5 登录与开通

参照 zedis ADR 17：

- 首个管理员通过一次性 setup token 创建。
- 之后的账号由管理员邀请。
- 登录后颁发 `HttpOnly` + `SameSite=Strict` 的 Cookie，页面与 API 同源。
- 登录页由 `index.html` 负责，登录成功后再加载 wasm。

## 10. 已定决策

| # | 问题 | 结论 |
|---|---|---|
| D1 | 浏览器版的定位 | 多账号。账号属于团队，项目属于团队，也可以单独授权给账号或其他团队（第 9 节） |
| D2 | 桌面版的旧数据 | 首次启动时把 Tauri 版的数据**复制**到新的数据目录，不与旧版共用文件（4.3 节） |
| D3 | 树结构的存储表示 | 所有后端都使用“目录 `children` 列表 + 顶层顺序”，移动算法只在 L0 写一份；服务器把 children 存为 JSON 列（4.2 节） |
| D4 | Cookie 的存放位置 | 桌面版首次导入 `cookies.json` 后存入 turso 表；浏览器版存在服务器，按账号隔离 |
| D5 | 浏览器版发送 HTTP 的方式 | 全部由 bridge 代发；以后可以增加可选的“浏览器直连”模式（第 5 节） |
| D6 | 项目角色的粒度 | 三级：`Viewer` / `Editor` / `Manager`（9.2 节）；需要更细的粒度时再扩展 |
| D7 | 团队 `Member` 的默认项目角色 | `Editor`（9.3 节） |
| D8 | 变量是否区分个人值 | 不区分，变量随项目共享，同项目成员都能看到（4.2 节） |
| D9 | 接口内容的并发写入 | 后写覆盖，不加乐观锁；冲突明显时再加版本号（第 7 节） |
