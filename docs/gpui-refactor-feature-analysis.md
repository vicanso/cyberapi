# CyberAPI 功能点分析（GPUI 重构参考）

> 基于 `v0.1.21`（commit `9e0bf44`）源码整理。
> 范围：只梳理**功能与业务逻辑**，不涉及 UI 外观；对现有代码做逻辑分层，把 View 相关部分单独剥离；不涉及 GPUI 的具体实现。

---

## 0. 阅读约定

- 功能编号：`F<模块>.<序号>`，例如 `F5.3`，方便重构时逐条追踪。
- 目标分层标记（第 3 节详细说明）：
  - **[D]** Domain：纯数据模型与纯函数，不做 IO
  - **[I]** Infrastructure：持久化、网络、文件系统、平台能力
  - **[S]** Service：应用服务 / 用例编排
  - **[St]** State：与 UI 框架无关的应用状态（类似 ViewModel）
  - **[V]** View：纯界面，本文只列出，不展开
- 引用格式：`文件:行号`，指向现有实现。

---

## 1. 项目概览

CyberAPI 是一个跨平台桌面 HTTP API 客户端（定位类似 Postman / Insomnia），主打安装包小、启动快、内存占用低。

| 项 | 现状 |
|---|---|
| 壳 | Tauri 1.x（`src-tauri/`） |
| 前端 | Vue 3 + Pinia + naive-ui + Monaco Editor（`src/`） |
| 后端 | Rust：hyper 0.14 + hyper-rustls、SeaORM + SQLite、cookie_store |
| 前端本地存储 | localforage（WebView 内的 IndexedDB） |
| 代码规模 | 约 14k 行（TS/TSX 约 12k，Rust 约 1.9k） |

### 1.1 当前运行时结构

```
┌─────────────────────────── WebView (TS) ───────────────────────────┐
│ views/*.tsx, components/*.tsx   ← UI，夹带了大量业务逻辑               │
│        │                                                            │
│ stores/*.ts (Pinia)             ← 状态 + 部分业务（缓存、树操作）        │
│        │                                                            │
│ commands/*.ts                   ← 业务核心：请求组装、模板函数、导入、    │
│        │                          响应解析、cURL；封装 IPC              │
│ helpers/*.ts                    ← 工具：拼音、cURL 解析、文件写入等      │
│        │                                                            │
│ localforage ── UI 状态、响应历史、设置                                │
│ @tauri-apps/api ── fs / dialog / clipboard / window / os / process   │
└────────┼────────────────────────────────────────────────────────────┘
         │ invoke (JSON IPC，body 以 base64 传输)
┌────────▼──────────────────────── Rust ──────────────────────────────┐
│ commands.rs        ← 27 个 Tauri command，薄封装                     │
│ schemas/*.rs       ← 表结构 + CRUD + 备份/恢复                        │
│ entities/*.rs      ← SeaORM 生成的实体                                │
│ http_request.rs    ← 实际发 HTTP、解压、耗时统计（解析 tracing 日志）   │
│ cookies.rs         ← 全局 CookieStore，持久化到 cookies.json           │
└─────────────────────────────────────────────────────────────────────┘
```

重构的关键点：现在**业务逻辑分散在 TS 和 Rust 两边**，TS 那边又有不少混在 View 组件里。换成 GPUI 后 IPC 层不复存在，所有非 View 逻辑都应收拢到 Rust 的 Domain / Service 层。

---

## 2. 现状分层分析

### 2.1 目录职责与实际内容

| 目录 / 文件 | 名义职责 | 实际包含的业务逻辑 | 重构归属 |
|---|---|---|---|
| `src-tauri/src/schemas/*` | 数据访问 | 建表 SQL、CRUD、递归查询目录子项、zip 备份/恢复 | [I] Repository + [S] Backup |
| `src-tauri/src/http_request.rs` | 发 HTTP | 请求组装（query、header、cookie）、TLS、超时、解压、耗时统计 | [I] HttpClient |
| `src-tauri/src/cookies.rs` | Cookie | Cookie 持久化、Set-Cookie 解析、过期处理 | [I] CookieStore |
| `src/commands/http_request.ts` | IPC 封装 | **发送管线**：body 处理、form/multipart 编码、模板函数、UA、Basic Auth、超时默认值、cURL 生成 | [D] + [S] RequestService |
| `src/commands/fn.ts` | — | **模板函数**的解析与执行 | [D] Template + [S] |
| `src/commands/http_response.ts` | — | 响应 body 分类、状态码文本、**响应历史**（存 localforage） | [D] + [S] History |
| `src/commands/import_api.ts` | — | Postman / Insomnia / Swagger / cURL / 自有格式的**导入转换** | [D] Importers + [S] |
| `src/commands/cookies.ts` | IPC 封装 | 解析 cookie_store 的内部 JSON 结构 | [I] |
| `src/commands/database.ts` | IPC 封装 | 启动时建表 + 写版本记录 | [S] Bootstrap |
| `src/stores/api_folder.ts` | 状态 | **`addChild`：树移动 / 插入的核心算法** | [D] Tree + [S] |
| `src/stores/api_setting.ts` | 状态 | **`fillValues`：环境变量替换、全局请求头注入**；选中项持久化；窗口标题 | [D] + [S] + [St] |
| `src/stores/api_collection.ts` | 状态 | 展开目录、顶层排序、活动 Tab 的持久化 | [S] + [St] |
| `src/stores/setting.ts` | 状态 | 应用设置读写、窗口尺寸策略 | [S] Settings + [St] |
| `src/stores/variable.ts` 等 | 状态 | 变量 CRUD 缓存、按名称取启用值 | [S] + [St] |
| `src/stores/local.ts` | 存储 | localforage 实例与 key 定义 | [I] KvStore |
| `src/stores/dialog.ts`, `header.ts` | 状态 | 弹窗开关、面包屑 | [St]/[V] |
| `src/helpers/curl.js` | 工具 | cURL 命令解析 | [D] |
| `src/helpers/pinyin.ts` | 工具 | 汉字转拼音 / 首字母 | [D] |
| `src/helpers/util.ts` | 工具 | 拼音匹配、JSON 判断与格式化、下载目录写文件（重名去重）、时间与耗时格式化、剪贴板 | [D] + [I] |

### 2.2 夹在 View 里的业务逻辑（重构时必须抽出）

| 位置 | 逻辑内容 | 抽到 |
|---|---|---|
| `src/views/Dashboard.tsx:96` `filterAndSort` | 项目过滤（名称 + 描述，支持拼音）与 5 种排序 | [D] `collection::filter_and_sort` |
| `src/views/Collection.tsx:156` `handleSend` | 发送 / 中止状态机、并发请求去重（reqID） | [S] RequestService + [St] |
| `src/views/Collection.tsx:123` | 分栏宽度计算规则（首栏为像素，次栏为比例） | [St] Layout |
| `src/components/APISettingTree/items.tsx:169` `convertToTreeItems` | **树构建、关键字过滤、顶层排序** | [D] Tree |
| `src/components/APISettingTree/items.tsx:355` `handleMove` | **拖拽落点 → 移动语义**（放进目录 / 插到前面 / 追加到末尾） | [D] Tree + [S] |
| `src/components/APISettingTree/items.tsx:558` | 重命名（按类型分发到目录或接口） | [S] |
| `src/components/APISettingTree/item_dropdown.tsx:89` | 导出目录（递归收集子项） | [S] Export |
| `src/components/APISettingTree/header.tsx:133` | 导出整个项目 | [S] Export |
| `src/components/ExDialog.tsx:138` `handleImport` | 导入编排：导入 → 挂到目标目录 → 重新加载 | [S] Import |
| `src/components/APIResponse/index.tsx:152` `fillValues` | 响应解析、预览判断、**附件自动保存到下载目录**、读缓存 | [D] + [S] |
| `src/components/APIResponse/index.tsx:228` | 生成 cURL，超过 2KB 改为写入剪贴板 | [S] |
| `src/components/APISettingParams/index.tsx:98` | 请求配置自动保存、防止写到已切换的接口 | [S] + [St] |
| `src/components/APISettingParams/uri.tsx:70` `cuttingURI` | 拆分 URI 里的环境变量前缀 `{{ENV}}` | [D] |
| `src/components/APISettingParams/uri.tsx:142` | 发送按钮防抖（200ms）、发送中再点即中止 | [St] |
| `src/components/APISettingParams/req_params.tsx:99` | 哪些 method 有 body、默认 Tab 规则、切换 Content-Type 时清空 body | [D] + [St] |
| `src/views/VariableSetting.tsx:85` | KV 列表的增 / 改 / 删 → 变量 CRUD 映射 | [S] + [St] |
| `src/components/ExCookieEditor.tsx:48` | Cookie 新增和编辑的字段规则、默认值 | [S] |
| `src/views/AppHeader.tsx:176` | 备份 / 恢复流程、切换语言后重启 | [S] |

---

## 3. 目标分层建议（与 UI 框架无关）

```
┌──────────────────────────────────────────────────────────────┐
│ L4  View (GPUI)                    —— 本文不展开                │
├──────────────────────────────────────────────────────────────┤
│ L3  App State / ViewModel                                     │
│     AppState, DashboardState, WorkspaceState(当前项目),         │
│     RequestEditorState, ResponseViewState, SendState …        │
│     • 只存状态和派生数据，通过事件 / 回调驱动 Service             │
├──────────────────────────────────────────────────────────────┤
│ L2  Application Services（用例编排，异步）                       │
│     Bootstrap, CollectionService, TreeService, RequestService, │
│     HistoryService, VariableService, CookieService,            │
│     ImportService, ExportService, BackupService,               │
│     SettingsService, PinService, LocalStateService             │
├──────────────────────────────────────────────────────────────┤
│ L1  Infrastructure（trait + 实现）                              │
│     Db Repository(SQLite), KvStore(替代 localforage),           │
│     HttpClient(计时 / 取消 / 解压), CookieStore,                 │
│     FileSystem(受限目录), Platform(剪贴板 / 文件对话框 /          │
│     窗口 / OS 信息 / 重启)                                       │
├──────────────────────────────────────────────────────────────┤
│ L0  Domain（纯 Rust，无 IO，可单测）                             │
│     model, tree, template(解析), request_builder, response,    │
│     curl(生成 / 解析), importers, search(拼音), sort            │
└──────────────────────────────────────────────────────────────┘
依赖方向：L4 → L3 → L2 → L1/L0；L1 → L0；L0 不依赖任何层
```

分层规则：

1. **L0 不做 IO**。模板函数执行时需要读文件、弹文件选择框、查历史响应、查变量，这些通过 trait 注入（如 `TemplateContext`），L0 只负责解析、调度和组合。
2. **L1 全部以 trait 暴露**，便于测试和替换。现在的 Web mock 模式（`isWebMode` + `commands/fake.ts`）可以用内存实现的 Repository / HttpClient 代替。
3. **L2 不引用任何 UI 类型**。需要用户确认的步骤（删除确认、覆盖确认）由 L3/L4 先完成，L2 只接收已确认的指令。
4. **L3 不依赖 GPUI 具体组件**，保存“选中了哪个接口、正在发送、当前 Tab”这类状态，View 只做渲染和事件转发。

### 3.1 现有代码 → 目标模块映射

| 目标模块 | 层 | 来源 |
|---|---|---|
| `model` | D | `src-tauri/src/schemas/*.rs` 结构体，`src/commands/*.ts` 中的 interface |
| `tree` | D | `items.tsx` `convertToTreeItems` / `handleMove`，`stores/api_folder.ts` `addChild`，`schemas/api_folder.rs` `list_api_folder_all_children` |
| `template` | D | `src/commands/fn.ts` |
| `request_builder` | D | `stores/api_setting.ts` `fillValues`，`commands/http_request.ts` `doHTTPRequest` 前半段，`http_request.rs` 组装部分 |
| `response` | D | `commands/http_response.ts` `getResponseBody` / `getStatusText`，`APIResponse/index.tsx` 中的文件名提取与预览判断 |
| `curl` | D | `commands/http_request.ts` `convertRequestToCURL`，`helpers/curl.js` |
| `importers` | D | `commands/import_api.ts` |
| `search` | D | `helpers/pinyin.ts`，`helpers/util.ts` `isMatchTextOrPinYin`，`Dashboard.tsx` `filterAndSort` |
| `db` | I | `src-tauri/src/schemas/*`，`entities/*` |
| `kv_store` | I | `src/stores/local.ts`（localforage） |
| `http_client` | I | `src-tauri/src/http_request.rs` |
| `cookie_store` | I | `src-tauri/src/cookies.rs`，`src/commands/cookies.ts` |
| `platform` | I | `@tauri-apps/api/*` 的所有调用（见第 7 节） |
| 各 Service | S | `stores/*.ts` 的 actions + 第 2.2 节中从 View 抽出的流程 |

---

## 4. 数据模型

### 4.1 SQLite（`{app_data_dir}/my_db.db`）

`app_data_dir` 在 macOS 上是 `~/Library/Application Support/com.bigtree.cyberapi/`。所有表的主键都是 `TEXT`，取值为 ULID；时间字段为字符串（前端写入 `dayjs().format()`，即带本地时区的 ISO 8601；后端兜底写入 UTC RFC 3339）。

| 表 | 字段 | 说明 |
|---|---|---|
| `api_collections`（项目） | `id, name, description, created_at, updated_at` | 顶层容器，环境变量等都按项目隔离 |
| `api_folders`（目录） | `id, collection, children, name, created_at, updated_at` | `children` 为**逗号分隔的子 ID 列表**（子目录 ID 或接口 ID 混排），顺序即显示顺序 |
| `api_settings`（接口） | `id, collection, name, category, setting, created_at, updated_at` | `category` 目前只有 `"http"`；`setting` 是 `HTTPRequest` 的 JSON 字符串 |
| `variables`（变量） | `id, category, collection, name, value, enabled, created_at, updated_at` | `category`：`env` / `customize` / `globalReqHeaders`；`enabled` 为字符串 `"1"` / `"0"` |
| `versions`（版本） | `id, version, created_at, updated_at` | 每次应用版本变化记录一条，预留给数据库迁移（尚未使用） |

建表语句见 `src-tauri/src/schemas/*.rs` 的 `get_*_create_sql`。没有外键、没有索引、没有迁移脚本。

> 建议：新实现**保持表结构兼容**，这样可以直接读取老用户的 `my_db.db`。

### 4.2 树结构的表示

- 树**只通过** `api_folders.children` 表达父子关系，接口本身不记录 parent。
- 顶层项（不属于任何目录的目录 / 接口）的**顺序不在数据库里**，存在本地 KV 的 `topTreeItem[collection]`（见 4.6）。
- 一个 ID 理论上可以出现在多个目录的 `children` 中；`addChild` 会先把它从所有目录中移除再插入，以保证唯一。

### 4.3 `HTTPRequest`（`api_settings.setting` 的内容）

```jsonc
{
  "method": "GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD",
  "uri": "{{ENV_NAME}}/users/v1/me",       // 可带环境变量前缀
  "contentType": "application/json | application/x-www-form-urlencoded | multipart/form-data | application/xml | text/plain",
  "body": "...",                           // 规则见下表
  "query":   [{ "key": "", "value": "", "enabled": true }],
  "headers": [{ "key": "", "value": "", "enabled": true }],
  "auth":    [{ "key": "user", "value": "pass", "enabled": true }]   // 仅第一条启用项生效，用于 Basic Auth
}
```

| contentType | body 存储格式 |
|---|---|
| JSON / XML / Plain | 原始文本；JSON 可以包含 `//` 开头的整行注释（发送前去掉） |
| Form / Multipart | `KVParam[]` 的 JSON 字符串；Multipart 中 value 以 `file://` 开头表示文件路径 |

`KVParam`：`{ key: string, value: string, enabled: bool }`，见 `src/commands/interface.ts`。

### 4.4 `HTTPResponse` / `HTTPStats`

| 字段 | 说明 |
|---|---|
| `id` | 响应 ID（ULID，写入历史时生成） |
| `api` | 所属接口 ID |
| `req` | 产生这次响应的**原始请求配置**（环境变量和全局头注入之前的版本） |
| `status` | 状态码；`-1` 表示请求中；请求失败时为空 |
| `headers` | `Map<小写name, string[]>` |
| `body` | base64（解压后的内容）；请求失败时为错误信息文本 |
| `bodySize` | 原始（压缩状态下）body 字节数 |
| `latency` | 总耗时（毫秒） |
| `stats` | `remoteAddr, isHttps, cipher, dnsLookup, tcp, tls, send, serverProcessing, contentTransfer, total`（毫秒） |

### 4.5 `Cookie`

`{ name, value, path, domain, expires }`。唯一性由 `(name, domain, path)` 决定。持久化文件为 `{app_data_dir}/cookies.json`（cookie_store 的 JSON 格式，只保存持久化 Cookie，Session Cookie 在重启后丢失）。

### 4.6 本地 KV 存储（现为 localforage / IndexedDB）

| Store | Key | Value | 用途 |
|---|---|---|---|
| `expandedSetting` | `{collectionId}` | `string[]` | 已展开的目录 ID |
| `topTreeItem` | `{collectionId}` | `string[]` | 顶层项的排序 |
| `tabActive` | `"activeTabs"` | `{ [apiId]: TabName }` | 每个接口最后打开的参数 Tab（空表示用默认 Tab） |
| `apiSetting` | `"selectedID"` | `string` | 当前选中的接口（**全局只有一个 key，不区分项目**） |
| `setting` | `"app"` | `AppSetting` | 应用设置（见 4.7） |
| `pinRequests` | `{collectionId}` | `{ id }[]` | 置顶接口 |
| `latestResponse` | `{apiId}` | `{ resp: HTTPResponse, createdAt }[]` | 响应历史，每个接口最多 10 条 |
| `lang` | `"lang"` | `string` | 界面语言 |

> 迁移提示：这部分数据在 WebView 的 IndexedDB 里，GPUI 版本无法直接读取。建议放弃迁移（只涉及 UI 状态、设置和响应历史），或者在新版本里统一放进 SQLite 的 KV 表 / JSON 文件。

### 4.7 `AppSetting`

| 字段 | 取值 / 默认 | 说明 |
|---|---|---|
| `theme` | `""`（跟随系统）/ `dark` / `light` | |
| `collectionSortType` | `lastModified`（默认）/ `nameAsc` / `nameDesc` / `olderFirst` / `newestFirst` | 首页项目排序 |
| `collectionColumnWidths` | `[300, 0.5]` | 第一个值是左栏像素宽度，第二个值是中栏占剩余宽度的比例，右栏自适应 |
| `resizeType` | `max`（默认）/ `custom` | 启动时最大化还是使用自定义尺寸 |
| `size` | `{ width ≥ 900, height ≥ 600 }` | 自定义窗口尺寸 |
| `latestRoute` | `{ name, params, query }` | 上次所在页面，启动时恢复 |
| `timeout` | `{ connect, write, read }`（秒） | `0` 表示使用默认值 10 / 120 / 300 |

---

## 5. 功能点清单

### M1 应用生命周期与窗口

| ID | 功能 | 规则 / 细节 | 现实现 | 层 |
|---|---|---|---|---|
| F1.1 | 启动流程 | 依次：显示 Splash → 建表 + 写版本记录 → 读取语言 → 恢复上次页面 → 读取应用信息 → 读取设置 → 应用窗口尺寸 → 关闭 Splash 并显示主窗口（Splash 最少显示 300ms） | `src/main.ts`，`src/root.tsx:35` | S |
| F1.2 | 建表与版本记录 | `CREATE TABLE IF NOT EXISTS` 建 5 张表；读取最新版本记录（按 `created_at` 倒序），和当前应用版本不同时插入一条新记录。这里预留了“版本变化 → 数据库迁移”的入口 | `src/commands/database.ts:32`，`schemas/database.rs:68` | S + I |
| F1.3 | 窗口尺寸策略 | `max` 时最大化；`custom` 且宽高都 > 0 时设置为逻辑尺寸；设置弹窗关闭时如有改动则立即应用 | `stores/setting.ts:190`，`views/AppSetting.tsx:39` | S |
| F1.4 | macOS 关闭即隐藏 | macOS 上点击关闭按钮时拦截关闭事件并隐藏应用，不退出 | `src/event.ts` | I（Platform） |
| F1.5 | 窗口标题 | 首页标题为 `Cyber API`；选中接口时为 `Cyber API - {接口名}` | `helpers/util.ts:21`，`stores/api_setting.ts:37` | St |
| F1.6 | 全局错误提示 | 未捕获错误以系统对话框展示 `组件名(消息): info`；Rust 错误统一为 `{category, message}`，前端格式化为 `[category]message` | `src/main.ts:53`，`commands/invoke.ts:54`，`src-tauri/src/error.rs` | S |
| F1.7 | 应用信息 | 应用版本、运行时版本、平台、OS、OS 版本、架构、User-Agent、数据目录 | `stores/app.ts` | S |
| F1.8 | 重启应用 | 切换语言、恢复备份、清除缓存后 3 秒自动重启 | `helpers/util.ts:119` | I |
| F1.9 | 页面导航 | 两个页面：首页（项目列表）和项目工作区（`collection=<id>`）；每次导航后保存为 `latestRoute`；面包屑为“首页 > 项目名” | `router/*.ts`，`stores/header.ts` | St |
| F1.10 | 生产环境禁用右键菜单 | 防止用户通过右键刷新页面（WebView 特有问题，GPUI 不需要） | `src/root.tsx:32` | V（可丢弃） |
| F1.11 | Web Mock 模式 | 浏览器中运行时用假数据代替所有后端调用，便于调试 UI | `commands/fake.ts`，`isWebMode()` | 可丢弃；改为内存版 Repository 用于测试 |

### M2 项目（Collection）管理

| ID | 功能 | 规则 / 细节 | 现实现 | 层 |
|---|---|---|---|---|
| F2.1 | 项目列表 | 读取全部项目 | `stores/api_collection.ts:136` | S |
| F2.2 | 新建项目 | 名称必填，描述可选；ID 为 ULID | `views/Dashboard.tsx:163` | S |
| F2.3 | 修改项目 | 修改名称 / 描述，并刷新 `updatedAt` | `stores/api_collection.ts:147` | S |
| F2.4 | 删除项目 | 需二次确认；**依次删除**该项目的接口、目录、项目本身（变量**不删除**，见第 8 节） | `src-tauri/src/commands.rs:89` | S |
| F2.5 | 关键字过滤 | 把名称和描述转小写后拼接，做子串匹配或拼音匹配（见 F15.1） | `views/Dashboard.tsx:96` | D |
| F2.6 | 排序 | 最近修改（`updatedAt` 倒序，默认）、名称升序、名称降序、创建时间升序、创建时间降序；按字符串比较；选择会持久化 | `views/Dashboard.tsx:111` | D + S |
| F2.7 | 进入项目 | 打开工作区后加载：置顶列表、目录、接口、环境变量、全局请求头、展开状态、顶层排序、活动 Tab；如果之前有选中的接口，加载它最近一次的响应 | `views/Collection.tsx:89` | S |
| F2.8 | 离开项目 | 清空置顶状态，清空选中的接口（会同时写入持久化） | `views/Collection.tsx:206` | St |

### M3 接口树（目录与接口）

#### F3.1 树构建 [D]

来源：`components/APISettingTree/items.tsx:169`

1. 为每个接口生成节点：从 `setting` JSON 字符串里**用正则**提取 `method` 和 `uri`（为了性能，没有完整解析 JSON），`uri` 去掉 `{{...}}` 前缀。
2. 为每个目录生成节点：`expanded = expandedFolders.contains(id)`。
3. 遍历每个目录的 `children`（逗号分隔，跳过空值和自身 ID，跳过不存在的 ID），挂上子节点，并记录 `parent`、`childIndex`、`isLastChild`。
4. 没有被任何目录引用的节点就是顶层节点。
5. 顶层按 `topTreeItems.indexOf(id)` 排序；不在列表中的排最前（index 为 -1），它们之间的顺序是：先接口（按 DB 顺序），后目录。

#### F3.2 关键字过滤 [D]

- 关键字转小写。节点满足以下任一条件即匹配：`method == 关键字的大写` 完全相等；`uri` 包含关键字；名称做文本或拼音匹配。
- 节点匹配时，它的全部子节点都显示；不匹配时递归检查子节点，只要有一个子节点可见，父节点就可见。
- 有关键字时，**所有目录强制展开显示**。

#### F3.3 展开与折叠 [S]

- 点击目录切换展开状态，按项目持久化到 `expandedSetting`（`stores/api_collection.ts:106`）。
- “全部折叠”会清空该项目的展开列表（`stores/api_collection.ts:60`）。

#### F3.4 新建接口 / 新建目录 [S]

来源：`components/APISettingTree/index.tsx:64`

- 新建接口：名称必填，`category = "http"`，`setting` 为空；如果指定了父目录，就追加到该目录的 `children`；创建后自动选中。
- 新建目录：名称必填；如果指定了父目录，就追加到父目录的 `children`。
- 入口：树头部的“新增”下拉、目录右键菜单、快捷键（F3.12）。

#### F3.5 重命名 [S]

- 接口：双击名称进入编辑，回车或失焦时保存，Esc 取消；名称为空时不保存（`items.tsx:558`）。
- 目录：通过菜单的“修改”弹窗修改名称（`item_dropdown.tsx:161`）。

#### F3.6 删除 [S]

- 删除接口：二次确认；删除后如果它是当前选中项，清空选中（`stores/api_setting.ts:160`）。
- 删除目录：二次确认；后端递归收集所有子目录和子接口后一起删除（`schemas/api_folder.rs:120`，`commands.rs:119`）。
- 注意：两者都**不会**从父目录的 `children` 中移除自身 ID，留下悬空引用（构建树时会被跳过）。

#### F3.7 拖拽移动 [D + S]

来源：`items.tsx:355`（落点语义），`stores/api_folder.ts:65`（`addChild`）

View 层负责把鼠标位置换算成 `(moveItem, targetItem, overType)`，其中 `overType ∈ {Over, Top, Bottom}`。领域语义如下：

```
if move == target: 忽略
parent = target.parent; before = target.id
if target 是可见列表中的最后一项 且 overType == Bottom: before = ""     // 追加到末尾
if target 是目录:
    if overType == Over:  parent = target.id; before = ""             // 放进该目录末尾
    else if 上一个可见项有 parent 且是其父目录的最后一个子项:
        parent = 上一项.parent; before = ""                           // 追加到那个目录末尾
addChild(parent, [move], before):
    从所有目录的 children 中移除 move
    if parent 目录存在: 插入到 before 之前（找不到 before 时追加到末尾）
    逐个保存受影响的目录
if parent == "":                                                       // 移动到顶层
    更新 topTreeItems：先移除 move，再插入到 before 之前，或追加到末尾
```

- `Top` 和 `Bottom` 在大多数情况下效果相同（都插到 target 前面）。
- **没有循环检测**：可以把目录拖进它自己的子孙目录，结果是整棵子树从树上消失（见第 8 节）。

#### F3.8 选中接口 [S + St]

- 选中后持久化 `selectedID`，更新窗口标题，并加载该接口最近一次的响应（`views/Collection.tsx:63`）。

#### F3.9 接口菜单操作

| 操作 | 说明 | 现实现 | 层 |
|---|---|---|---|
| 复制配置 | 把完整的 `APISetting` 记录序列化为 JSON 写入剪贴板，可以再通过“导入 → JSON”粘贴回来 | `item_dropdown.tsx:241` | S |
| 复制为 cURL | 注入环境变量和全局请求头，加载最新 Cookie，生成 cURL（见 F7.8）写入剪贴板 | `item_dropdown.tsx:203` | S |
| 置顶（Pin） | 见 M12 | `item_dropdown.tsx:224` | S |

#### F3.10 目录菜单操作

新建接口、新建目录、修改名称、导出该目录（F10.2）、导入到该目录（M9）、删除。

#### F3.11 树头部操作

关键字过滤、新建接口、新建目录、导出整个项目（F10.1）、导入到项目根（M9）、全部折叠。

#### F3.12 快捷键 [St]

| 快捷键 | 动作 | 说明 |
|---|---|---|
| `⌘ N` | 在根目录新建接口 | 只判断 `metaKey`，Windows / Linux 上不可用 |
| `⇧ ⌘ N` | 在根目录新建目录 | 同上 |
| URI 输入框中按 `Enter` | 发送请求 | URI 不为空时生效 |

来源：`helpers/hot_key.ts`，`APISettingTree/header.tsx:95`，`APISettingParams/uri.tsx:293`

### M4 请求编辑

| ID | 功能 | 规则 / 细节 | 现实现 | 层 |
|---|---|---|---|---|
| F4.1 | 加载请求配置 | 选中接口时把 `setting` 解析为 `HTTPRequest`，解析失败时使用空对象 | `stores/api_setting.ts:54` | S |
| F4.2 | Method | 可选 GET / POST / PUT / PATCH / DELETE / OPTIONS / HEAD，默认 GET；修改后立即保存 | `APISettingParams/uri.tsx:187` | D + S |
| F4.3 | URI 与环境变量前缀 | 保存格式为 `{{ENV}}` + 路径。编辑时拆成“环境变量选择器 + 路径输入框”（`cuttingURI`），保存时再拼回去；失焦时保存；会去掉首尾空格 | `uri.tsx:70`，`uri.tsx:122` | D + St |
| F4.4 | 选择环境变量 | 下拉中只列出**启用**的环境变量（显示 `名称 \| 值`），另有“新增”（打开环境变量弹窗）和“清除当前” | `uri.tsx:202` | St |
| F4.5 | 有 body 的 method | 只有 POST / PUT / PATCH 才显示 Body Tab，发送时也只有这三种会带 body | `req_params.tsx:99`，`http_request.ts:274` | D |
| F4.6 | 切换 Content-Type | 可选 JSON（默认）/ Form / Multipart / XML / Plain。切换时**清空 body**，body 不为空时需要先确认 | `req_params.tsx:222` | S + St |
| F4.7 | 文本 body 编辑 | JSON / XML / Plain 用文本编辑器编辑；JSON 可一键格式化；内容变化时去掉首尾空白后保存 | `req_params.tsx:199` | St |
| F4.8 | Form / Multipart body | 以 KV 列表编辑；Multipart 的每一行可以选择文件，value 写为 `file://{绝对路径}`（文件对话框默认打开下载目录） | `req_params.tsx:572`，`ExKeyValue.tsx:153` | S + St |
| F4.9 | Query / Header / Auth | 都用 KV 列表编辑；Auth 的 value 以密码形式显示；**只有第一条启用的 Auth 生效**，转换为 Basic Auth | `req_params.tsx:584` | St |
| F4.10 | KV 列表的通用行为 | 末尾总有一个空行，聚焦即新增一行；每行可以启用 / 禁用、删除（需确认）；保存时过滤掉 key 和 value 都为空的行；输入防抖 200ms | `components/ExKeyValue.tsx` | St |
| F4.11 | 自动保存 | URI / Method 立即保存；Body / Query / Header / Auth 防抖 300ms 后保存；保存前检查“修改发生时的接口 ID == 当前选中的 ID”，防止防抖期间切换接口导致写错 | `APISettingParams/index.tsx:98` | S + St |
| F4.12 | 活动 Tab 记忆 | 每个接口记住最后打开的 Tab；如果是默认 Tab（有 body 的 method 默认 Body，否则默认 Query），存为空值；切换 method 时自动切到对应的默认 Tab | `req_params.tsx:183`，`req_params.tsx:299`，`req_params.tsx:533` | St |
| F4.13 | 数量提示 | Query / Header / Auth / Form 的条目数显示在 Tab 上 | `req_params.tsx:111` | V |
| F4.14 | 参数栏宽度预设 | 中栏宽度可快速设为 30% / 50% / 70%，合法范围为 0.2–0.8 | `stores/setting.ts:163` | St |

### M5 请求发送管线（核心）

#### F5.1 发送 / 中止状态机 [St + S]

来源：`APISettingParams/uri.tsx:142`，`views/Collection.tsx:156`

```
Idle --点击发送--> Sending(reqId)        // 相隔不到 200ms 的第二次点击直接忽略
Sending --再次点击--> Idle               // “中止”：只是丢弃结果，后端请求仍在继续
Sending --完成--> Idle + 展示响应         // 只有 reqId 仍是当前请求时才更新界面
Sending --异常--> Idle + 错误提示
```

- 发送开始时，响应面板进入“请求中”状态（status = -1，并显示计时）。
- 没有选中接口时不能发送。

#### F5.2 请求预处理：变量注入 [D]

来源：`stores/api_setting.ts:61`

1. 复制一份 `originalReq` 留给历史记录。
2. 环境变量：用 `/\{\{([\S\s]+)\}\}/` 匹配 URI，取启用状态下**第一个**同名变量的值替换，只替换一次。
3. 全局请求头：把当前项目中所有启用的 `globalReqHeaders` 变量追加到 `headers` 末尾。

#### F5.3 Body 处理 [D]

来源：`commands/http_request.ts:254`

1. method 不是 POST / PUT / PATCH 时，body 和 contentType 都置空。
2. `convertBody`：如果 body 看起来像 JSON（去空白后首尾为 `{}` 或 `[]`），删除所有 `//` 开头的行；然后执行模板函数替换（M6）。
3. Form：把 KV JSON 解析出来，取启用项，key 和 value 都做 `encodeURIComponent`，用 `&` 拼接。
4. Multipart：取启用且 key 不为空的项；`file://` 开头的项读取二进制文件，MIME 按扩展名推断，文件名为完整路径；编码为 multipart（Content-Type 带 boundary）；body 以 base64 传给 Rust，Rust 发现 Content-Type 以 `multipart/form-data` 开头时先解码。

#### F5.4 Query / Header 处理 [D]

1. Query 和 Header 的 **value** 执行模板函数替换（key 不处理）。**URI 本身不支持模板函数。**
2. 追加 `User-Agent: CyberAPI/{appVer} ({os}; tauri/{ver}; {osVer}; {arch})`。
3. 存在启用的 Auth 时，追加 `Authorization: Basic base64(key:value)`。
4. 超时：`connect / write / read` 默认为 10 / 120 / 300 秒，设置中大于 0 的值覆盖默认值。

#### F5.5 HTTP 执行 [I]

来源：`src-tauri/src/http_request.rs:403`

| 步骤 | 行为 |
|---|---|
| Method | 不认识的值按 GET 处理（额外支持 CONNECT / TRACE） |
| URL | 解析 URI，把启用的 query 用 `append_pair` 追加上去（保留 URI 中已有的 query） |
| Header | 按顺序执行 `insert`（**同名 header 后写入的覆盖先写入的**）：请求自身的 header → 全局 header → UA → Authorization；然后强制设置 `Accept-Encoding: gzip, br`；没有设置 Content-Type 时用 contentType 补上；最后按 URL 从 CookieStore 取值设置 `Cookie`（会覆盖用户手写的 Cookie） |
| 连接 | 每次请求新建一个 Client；HTTPS 使用 rustls + 系统根证书；仅支持 HTTP/1.1；connect / read / write 分别设置超时 |
| 重定向 | **不跟随**，3xx 原样返回 |
| 响应头 | key 转小写，同名多值合并为数组；`set-cookie` 写入 CookieStore（F8.2） |
| 解压 | 按 `content-encoding` 精确匹配 `gzip` / `br` 后解压；`bodySize` 记录解压前的大小 |
| 返回 | body 以 base64 编码；`latency = stats.total` |

当前**不支持**的能力（便于决定重构范围）：代理、客户端证书、跳过 TLS 校验、HTTP/2、跟随重定向、deflate、流式 / 大文件响应、WebSocket / GraphQL / gRPC（`category` 字段预留了 graphQL）。

#### F5.6 耗时统计 [I]

- 统计项：DNS、TCP、TLS、发送、服务器处理（TTFB）、内容传输、总计，另有远端地址和 TLS 加密套件。
- 实现方式：注册一个 tracing Layer，**按字符串匹配 hyper / rustls 的日志**来打时间点，状态保存在全局单例里（`http_request.rs:308`）。并发请求时数据会串，hyper 升级后很可能失效。重构时需要换成真正的按请求计时（例如自定义 connector 或 hook）。

#### F5.7 错误处理 [S]

- 预处理阶段出错（读文件失败、模板参数错误等）：抛给调用方，界面提示错误。
- 网络阶段出错：生成一条响应，`body` 为错误信息，`status` 为空，`latency` 为已耗时间，**这条响应同样会写入历史**。
- `latency <= 0` 时修正为 1。

#### F5.8 写入历史 [S]

每次请求完成（包括失败）后，`resp.req = originalReq`，然后写入该接口的历史（F7.9）。

### M6 模板函数

来源：`src/commands/fn.ts`

#### 语法

- 写法：`{{fn(param)}}` 或 `{{fn2.fn1(param)}}`。可以用在 **body、query 的 value、header 的 value** 中。
- 匹配：外层用 `/\{\{([\s\S]+?)\}\}/g`，参数用 `/\(([\s\S]*?)\)/`。
- 函数链用 `.` 分隔，**从右往左**执行，上一个函数的结果作为下一个函数的输入。
- 参数用 `,` 分隔，去掉首尾空白和单 / 双引号；只有一个参数时是字符串，多个时是数组。
- 执行结果转为字符串，替换原文本（只替换**第一次**出现的相同文本）。
- 某个 `{{...}}` 解析不出参数时，**停止解析后面所有的表达式**。

#### 函数表

| 函数 | 别名 | 参数 | 行为 |
|---|---|---|---|
| `readTextFile` | `rtf` | `(file, dir?)` | 以文本读取文件；`dir` 只能是 `document` / `desktop` / `download`（默认） |
| `readFile` | `rf` | `(file, dir?)` | 以二进制读取文件，通常和 `base64` 串联使用 |
| `base64` | `b64` | 上一个函数输出的字节 | 编码为 base64 |
| `openFile` | `of` | — | 弹出文件选择框，返回所选文件的路径 |
| `get` | `g` | `(apiId, path)` | 读取指定接口**最近一次响应**的 JSON，按 lodash 风格的路径取值，例如 `data.0.name` |
| `timestamp` | `ts` | — | 当前 Unix 时间戳（秒） |
| `md5` | — | `(text)` | 输出 hex |
| `sha256` | — | `(text)` | 输出 hex |
| `value` | — | `(name)` | 读取当前项目中启用的自定义变量（customize）的值 |

不认识的函数名直接透传参数。

#### 输入提示（GPUI 版新增）

在请求体、Query / Header / 表单的值（包括它们的文本编辑模式）里输入 `{{` 时列出函数，附别名和说明（网页版没有本机文件，不列 `readTextFile` / `readFile` / `openFile`）；在 `value(` 里列出启用的自定义变量和它的值；在 `get(` 里列出项目的接口，附所在目录：离正在编辑的接口近的在前（同目录最先，然后是上一级和子目录，按经过的目录数算，一样近的按树的顺序），正在编辑的接口本身排最后；可按名称或拼音过滤，名称开头匹配的排在拼音 / 包含匹配之前；选中后写入接口 ID 和 `, `。之后从这个接口最近一次存进历史的响应（与发送时 `get` 读的是同一份）里逐级列出 JSON 字段：对象的键、列表的前 10 项，附值或“对象 / 列表，几项”；有下一级的写出 `键.` 并接着列出下一级，到值时补上 `)}}`。响应在第一次需要时读取并缓存，这个接口再次收到响应后重新读取。选中项替换已输入的部分，补上缺少的 `)` / `}}`，光标落在其后；带参数的函数只写到 `(`。URI、Basic Auth 和变量弹窗里的全局请求头不提示。

#### 重构建议

- 解析（L0）和执行（L2）分开。执行依赖抽象为 trait：`FileReader`（受限目录）、`FilePicker`、`ResponseLookup`、`VariableLookup`、`Clock`。
- 生成 cURL 时也会执行模板函数，`openFile` 会因此弹出对话框，需要决定这种场景下是否执行。

### M7 响应展示与历史

| ID | 功能 | 规则 / 细节 | 现实现 | 层 |
|---|---|---|---|---|
| F7.1 | Body 分类 | `content-type` 含 `application/json` → 解码并以 4 空格缩进格式化（解析失败会直接抛错）；匹配 `/text\|javascript/i` → 按文本解码；其余为二进制（保留 base64） | `http_response.ts:135` | D |
| F7.2 | 大小 | 优先取 `content-length`，否则用 base64 长度 × 3/4 估算；同时显示压缩前的大小和比例 | `http_response.ts:162`，`APIResponse/index.tsx:487` | D |
| F7.3 | 状态文本 | 内置状态码 → 原因短语映射表；按 2xx–3xx / 4xx / 5xx 分三级 | `http_response.ts:62`，`status_text.tsx` | D（分级颜色为 V） |
| F7.4 | 预览 | `content-type` 匹配 `/image\|pdf/i` 时进入预览模式（图片 / PDF） | `ExPreview.tsx:15` | D（渲染为 V） |
| F7.5 | 附件自动保存 | 响应头有 `content-disposition: ...filename="x"` 时，**自动**保存到下载目录；重名时依次尝试 `x-1` … `x-9`；从历史加载时不保存 | `APIResponse/index.tsx:213`，`helpers/util.ts:205` | S |
| F7.6 | 响应头 / Cookie | 响应头按名称首字母大写展示（`content-type` → `Content-Type`）；`set-cookie` 单独展示为 name=value | `APIResponse/index.tsx:430` | D + V |
| F7.7 | 请求信息 | 接口 ID（供 `get` 函数使用）、远端地址、TLS 套件、各阶段耗时 | `APIResponse/index.tsx:366` | V |
| F7.8 | 生成 cURL | 打开时才生成：重新加载 Cookie → 对 `req` 注入环境变量和全局头 → 生成。格式为 `curl -v -X{M} -d '{body}' -H 'k: v' … '{url}'`：JSON body 压缩为一行，Form body 做 urlencode，Cookie 取 domain 是 host 子串的那些；超过 2KB 时直接写入剪贴板并给出提示 | `http_request.ts:68`，`APIResponse/index.tsx:228` | D + S |
| F7.9 | 历史记录 | 每个接口最多保存 10 条（最新的在前，超出时删除最旧的）；每条包含完整的响应（含 body）和 `createdAt` | `http_response.ts:196` | S |
| F7.10 | 选择历史 | 历史列表显示状态码、大小、时间；选中某条后：①响应面板显示这条响应；②请求编辑区**回填为这条响应对应的请求配置**（不会立即保存，之后任何修改都会把它整体保存下来，相当于“恢复”） | `APIResponse/list.tsx`，`APISettingParams/index.tsx:87` | S + St |
| F7.11 | 清空历史 | 清空当前接口的历史 | `http_response.ts:220` | S |
| F7.12 | 切换接口时加载 | 选中接口时显示它的最近一次响应，没有则显示空白 | `views/Collection.tsx:63` | S |

### M8 变量（环境变量 / 自定义变量 / 全局请求头）

三类变量共用同一张表和同一套 CRUD，按 `(collection, category)` 隔离；读取时按名称排序。

| 类别 | `category` | 用途 | 生效方式 |
|---|---|---|---|
| 环境变量 | `env` | 定义 host 等，用于快速切换环境 | URI 前缀 `{{NAME}}`（F5.2）；同名的变量多个都启用时取第一个，所以 README 提醒不要同时启用同名变量 |
| 自定义变量 | `customize` | 保存常用值 | 模板函数 `{{value(name)}}`；**每次调用都查一次数据库** |
| 全局请求头 | `globalReqHeaders` | 给项目内所有请求统一加 header | 启用的项全部追加到请求头（F5.2） |

| ID | 功能 | 规则 | 现实现 | 层 |
|---|---|---|---|---|
| F8.1 | 列表 | 按名称排序；进入项目时加载环境变量和全局请求头，自定义变量在打开弹窗时加载 | `stores/variable.ts:50` | S |
| F8.2 | 新增 / 修改 / 删除 | 由 KV 编辑器的事件驱动：新增 → `add`，修改 → `update`（刷新 `updatedAt`），删除 → `remove`；**按列表下标对应到变量** | `views/VariableSetting.tsx:85` | S + St |
| F8.3 | 启用 / 禁用 | `enabled` 为 `"1"` / `"0"` | — | D |

### M9 Cookie 管理

| ID | 功能 | 规则 / 细节 | 现实现 | 层 |
|---|---|---|---|---|
| F9.1 | 持久化 | 全局（不分项目）的 CookieStore，每次变更后整体写回 `cookies.json`；Session Cookie 不持久化 | `cookies.rs` | I |
| F9.2 | 自动接收 | 响应中有 `Set-Cookie` 时：已过期的从存储中删除，其余按当前 URL 解析后存入 | `cookies.rs:102` | I |
| F9.3 | 请求时注入 | 按请求 URL 取匹配的 Cookie，拼成 `Cookie` 头 | `http_request.rs:478` | I |
| F9.4 | 列表 | 显示 name / value / path / domain / expires，按名称排序。现在是前端解析 cookie_store 内部的 JSON（`raw_cookie`、`path.0`、`domain.*`、`expires.AtUtc`），重构时应直接使用结构化数据 | `commands/cookies.ts:20` | I |
| F9.5 | 新增 | name / value / path / domain / expires 都必填；后端拼成 `name=value;Path=..;Domain=..;Expires=..`，以 `http://{domain}{path}` 作为来源 URL 解析存入 | `ExCookieEditor.tsx:48`，`cookies.rs:142` | S |
| F9.6 | 编辑 | 只能修改 value 和 expires；提供“永不过期”快捷选项（3000-01-01） | `ExCookieEditor.tsx` | S |
| F9.7 | 删除 / 清空 | 按 `(domain, path, name)` 删除；清空需确认 | `cookies.rs:80`，`cookies.rs:95` | S |

### M10 导入

来源：`src/commands/import_api.ts:350`，`src/components/ExDialog.tsx:138`

#### F10.1 入口与输入 [S + St]

- 入口：树头部（导入到项目根）、目录菜单（导入到该目录）。
- 打开导入窗口时读取剪贴板，内容像 JSON（首尾为 `{}` / `[]`）时自动格式化后预填。
- 两种输入：①文本（JSON 或 cURL）；②选择文件（读取为文本）。

#### F10.2 格式识别 [D]

按以下顺序判断：

1. 文本以 `curl` 开头 → cURL
2. 解析为 JSON 后：有 `swagger` 字段 → Swagger 2.0；有 `item` → Postman Collection；有 `resources` → Insomnia 导出；否则 → CyberAPI 自有格式。

#### F10.3 各格式的转换规则 [D]

| 格式 | 接口 | 目录 | 环境变量 | 未支持 / 限制 |
|---|---|---|---|---|
| cURL | 解析 `-X/--request`、`-H/--header`、`-d/--data/--data-ascii`（有 body 时 GET 自动改为 POST，默认 Form 类型）、`-u/--user`（转为 Basic 头）、`-A`、`-I`、`-b`、`--compressed`；URL 中的 query 拆到 query 列表；Content-Type 从 header 中取出（去掉 `;` 后的参数），没有时 body 像 JSON 就设为 JSON；名称固定为“未命名” | — | — | `--data-raw` / `--data-binary` / `-F` 不支持；`-b` 被错误地映射为 `Set-Cookie` 头 |
| Postman | 递归 `item`：有 `request` 的是接口；取 `method`、`url.raw`（query 拆开）、`request.query`、`body.raw`（首尾为 `{}` 时设为 JSON） | 没有 `request` 的 item 即目录，保留层级 | 顶层 `variable[]` | 不导入 headers 和 auth；只处理 raw body |
| Insomnia | `_type=request`：method、url（query 拆开）、`parameters`、`headers`、`body.text` | `_type=request_group`，根据 `parentId` 重建层级（先处理挂在 workspace 下的分组） | `_type=environment` 的 `data` | — |
| Swagger 2.0 | 遍历 `paths × method`：名称取 `summary` 或 `operationId`；`in=query` 的参数取 `example`；`in=body` 时根据 `$ref` 找到 definitions，按属性类型生成示例 JSON；URI 为 `{{<info.title 去空格>}}{path}` | 按 `tags[0]` 分组为目录 | 自动生成一个环境变量，值为 `{schemes[0]}://{host}{basePath}` | 不支持 OpenAPI 3；`$ref` 只解析一层；header / path 参数未处理 |
| CyberAPI | 数组或单个对象；`category == "http"` 的作为接口，其余作为目录；`collection` 改写为当前项目；**沿用原 ID** | 同左 | — | 导入到同一个库时 ID 冲突（见第 8 节） |

#### F10.4 写入与收尾 [S]

1. 找出“顶层 ID”：不在任何导入目录的 `children` 中的目录或接口。
2. 依次写入目录、接口、环境变量（name 和 value 都为空的跳过）。写入接口时如果遇到 `UNIQUE constraint failed`，换一个新 ID 重试一次（`commands/api_setting.ts:61`）。
3. 如果是导入到某个目录，用 `addChild` 把所有顶层 ID 挂到该目录下。
4. 重新加载目录、接口、环境变量。

#### F10.5 粘贴 cURL 新建接口（新增）[S + V]

- 在浏览器开发者工具里“复制为 cURL”后，回到工作区粘贴即新建接口：焦点不在输入框时按 ⌘V / Ctrl+V，或点树头部“+”菜单的“粘贴 cURL 新建接口”、目录菜单的“粘贴 cURL 到此目录”。
- **只在这些操作发生时读取剪贴板，不在后台或窗口获得焦点时读取。** 网页版的读取经浏览器的剪贴板接口，浏览器可能先询问用户。
- 输入框里的 ⌘V / Ctrl+V 仍是普通粘贴（URI 框粘贴 cURL 仍是覆盖当前接口，见 F4.3）。
- 键盘粘贴放在光标所在的目录（树有焦点时）或当前接口所在的目录，“+”菜单放在项目根；名称取 URL 路径的最后一段，新建后选中。剪贴板里不是 cURL 时只提示，不新建。

### M11 导出

| ID | 功能 | 规则 | 现实现 | 层 |
|---|---|---|---|---|
| F11.1 | 导出整个项目 | 该项目的所有目录和接口按 CyberAPI 格式（JSON 数组，4 空格缩进）写入下载目录，文件名为 `cyberapi-{项目名}.json`，重名时追加 `-1..-9` | `APISettingTree/header.tsx:133`，`helpers/util.ts:230` | S |
| F11.2 | 导出目录 | 递归收集该目录下的接口，文件名为 `cyberapi-{目录名}.json`。**存在缺陷**：目录记录没有被收集，导出结果只是扁平的接口列表，层级结构丢失 | `item_dropdown.tsx:89` | S |
| F11.3 | 不导出的内容 | 变量（环境变量 / 自定义变量 / 全局请求头）、顶层排序、置顶 | — | — |

### M12 置顶（Pin）

| ID | 功能 | 规则 | 现实现 | 层 |
|---|---|---|---|---|
| F12.1 | 置顶 | 按项目保存置顶接口的 ID 列表；已存在则忽略；只能置顶当前项目的接口 | `stores/pin_request.ts:35` | S |
| F12.2 | 展示与切换 | 显示为 `序号: 接口名` 的标签；点击（或 Cmd/Ctrl+序号）在编辑区打开该接口，树的选中项和展开的目录不变（GPUI 版：置顶是放在手边的接口，不打断树里正在看的位置；再点树里的行或关掉这个标签就回到树的选中项）；编辑区里的接口是置顶的时，对应标签高亮；已删除的接口不显示 | `views/AppHeader.tsx:131` | St |
| F12.3 | 取消置顶 | 关闭标签即从列表中移除 | `stores/pin_request.ts:47` | S |

### M13 数据备份与恢复

| ID | 功能 | 规则 | 现实现 | 层 |
|---|---|---|---|---|
| F13.1 | 备份 | 把 4 张表（项目、目录、接口、变量）分别导出为 `{表名}.json`，打包成 `~/Downloads/cyberapi-backup-YYYY-MM-DD.zip`（**同一天的备份会覆盖**）；不包含 versions、Cookie 和本地 KV | `schemas/database.rs:84` | S + I |
| F13.2 | 恢复 | 选择 `.zip` → 确认（提示会清空现有数据）→ **先清空** 4 张表，再按顺序逐表批量插入 → 3 秒后重启。整个过程没有事务 | `schemas/database.rs:121`，`views/AppHeader.tsx:189` | S + I |
| F13.3 | 入口 | 只在首页的功能菜单中提供 | `views/AppHeader.tsx:364` | V |

### M14 应用设置

| ID | 功能 | 规则 | 现实现 | 层 |
|---|---|---|---|---|
| F14.1 | 主题 | 跟随系统（空值）/ 深色 / 浅色；跟随系统时读取窗口的系统主题 | `stores/setting.ts:95`，`stores/setting.ts:143` | S + St |
| F14.2 | 窗口尺寸 | 见 F1.3；最小 900 × 600 | `views/AppSetting.tsx` | S |
| F14.3 | 请求超时 | connect / read / write（秒），0 表示默认值 | `stores/setting.ts:201` | S |
| F14.4 | 应用信息 | 见 F1.7 | `views/AppSetting.tsx:94` | V |
| F14.5 | 分栏宽度 | 拖动左栏时累加像素宽度；拖动中栏时累加“位移 / 剩余宽度”的比例，超过 1 时重置为 0.5；结果持久化 | `views/Collection.tsx:123` | St |
| F14.6 | 字号（新增） | 界面字号 12–18（默认 14，即 rem）、编辑器字号 11–20（默认 13，请求体和响应体）；0 表示默认值，超出可选范围的值按范围截断；修改后立即生效 | — | S + V |
| F14.7 | 编辑器自动换行（新增） | 请求体和响应体编辑器是否折行显示长行，默认开启（与以前一致） | — | S + V |
| F14.8 | 跟随重定向（新增） | 默认关闭（与旧版相同，3xx 原样返回）；开启后对 301 / 302 / 303 / 307 / 308 按 `Location` 逐跳重新发送，303 和 POST 的 301 / 302 改为 GET 并去掉 body，跨源时去掉 Authorization / Cookie / Host，每跳保存 `Set-Cookie` 并按新地址注入 Cookie；最多 10 次，超过即失败；历史只记最后一个响应 | — | D + S |
| F14.9 | 校验 HTTPS 证书（新增） | 默认校验；关闭后接受自签名、过期、域名不符的证书（仍校验握手签名）；证书校验失败时响应面板提示此设置 | — | S + I |
| F14.10 | HTTP 版本（新增） | 自动（默认）：HTTPS 经 ALPN 协商，服务器支持就用 HTTP/2，否则 HTTP/1.1，明文 HTTP 用 HTTP/1.1；仅 HTTP/1.1；仅 HTTP/2：TLS 协商不到 h2 即失败，明文走 h2c。实际版本显示在响应详情里，cURL 导出带上对应参数；响应体最多读 64 MB（解压后同样），超出截断并提示 | — | S + I |

### M15 通用能力

| ID | 功能 | 规则 | 现实现 | 层 |
|---|---|---|---|---|
| F15.1 | 拼音匹配 | 先做不区分大小写的子串匹配；不命中时把文本转为“全拼”和“首字母”两种形式再匹配。多音字会生成所有组合（组合数是各字读音数的乘积） | `helpers/util.ts:188`，`helpers/pinyin.ts` | D |
| F15.2 | 多语言 | 中文 / 英文 / 乌克兰语；默认取系统语言的前缀，不支持时用英文；切换后保存并在 3 秒后重启 | `i18n/index.ts`，`stores/local.ts:54`，`views/AppHeader.tsx:255` | S |
| F15.3 | 清除本地缓存 | 可分别清除：应用设置、置顶、响应历史；清除后 3 秒重启 | `views/StoreSetting.tsx`，`stores/local.ts:67` | S |
| F15.4 | 写入下载目录 | 目标文件已存在时依次尝试 `name-1.ext` … `name-9.ext`，都存在则报错 | `helpers/util.ts:205` | I |
| F15.5 | JSON 格式化 | 整体解析失败时按行尝试（兼容 NDJSON） | `helpers/util.ts:149` | D |
| F15.6 | 格式化工具 | 日期（今年显示 `MM-DD HH:mm`，否则显示 `YYYY-MM-DD`）、耗时（小于 1s 显示 ms，否则显示 s 并保留 2 位小数） | `helpers/util.ts:50`，`helpers/util.ts:131` | D |

---

## 6. 纯 View 层（剥离清单）

以下内容只和界面呈现有关，重构时**不作为业务逻辑迁移**，由 GPUI 重新实现。括号中注明需要 L3 提供的状态。

| 内容 | 现位置 | 需要的状态 |
|---|---|---|
| 各类弹窗的开关（设置、Cookie、环境变量、自定义变量、全局请求头、缓存） | `stores/dialog.ts`，`App.tsx` | 当前打开的弹窗 |
| 面包屑 | `stores/header.ts` | 当前页面 + 项目名 |
| 三栏布局与分栏拖拽手柄 | `components/ExColumn.tsx`，`views/Collection.tsx:225` | 列宽（F14.5） |
| 树的拖拽视觉（克隆节点、插入线、按坐标计算 `overType`） | `items.tsx:426-548` | 拖拽中状态 → 最终调用 F3.7 |
| 树缩进、目录图标、method 颜色（DELETE 红；POST / PUT / PATCH 绿；其他蓝） | `items.tsx:153` | — |
| 文本编辑器（Monaco）、JSON 高亮、只读响应编辑器 | `helpers/editor.ts`，`userWorker.ts` | 编辑内容 |
| 加载动画、请求计时器 | `ExLoading.tsx`，`ExTimer.tsx` | 发送状态 |
| 表单校验提示、二次确认弹层 | `ExForm.tsx`，`ExDeleteCheck.tsx` | — |
| 状态码颜色、大小与比例的展示格式 | `status_text.tsx`，`APIResponse/index.tsx:487` | — |
| 图片 / PDF 预览渲染 | `ExPreview.tsx` | 预览数据 |
| Cookie 表格列宽按内容估算 | `views/CookieSetting.tsx:98` | — |
| 首页卡片网格、排序下拉、语言下拉 | `views/Dashboard.tsx`，`views/AppHeader.tsx` | — |
| Splash 页面 | `splashscreen.html` | 启动阶段 |
| WebView 相关的 hack（禁用右键、resize 监听、DOM 工具） | `root.tsx`，`helpers/html.ts` | 丢弃 |

---

## 7. 平台能力依赖（Tauri API → 需要替代）

| 能力 | 现用法 | 使用位置 |
|---|---|---|
| 剪贴板读 / 写 | 复制 cURL、复制配置、导入预填 | `helpers/util.ts:104` |
| 文件选择框 | `openFile` 函数、Multipart 选文件、选择备份 zip | `fn.ts`，`ExKeyValue.tsx:158`，`AppHeader.tsx:200` |
| 读文件（受限目录） | `readTextFile` / `readFile` 只能读 Download / Document / Desktop；Multipart 按绝对路径读 | `fn.ts`，`http_request.ts:214` |
| 写文件到下载目录 | 导出配置、保存响应附件、备份 | `helpers/util.ts:205`，`database.rs:85` |
| 路径 | app data 目录、下载目录 | `stores/app.ts`，`ExKeyValue.tsx:161` |
| OS 信息 | 平台、类型、版本、架构（用于 UA 和应用信息） | `http_request.ts:242` |
| 应用版本 | UA、版本记录 | `database.ts:38` |
| 窗口 | 系统主题、最大化、设置尺寸、设置标题、显示 / 隐藏、关闭拦截 | `stores/setting.ts`，`commands/window.ts`，`event.ts` |
| 进程 | 重启 | `helpers/util.ts:119` |
| 原生消息框 | 启动错误、全局错误 | `main.ts`，`root.tsx` |
| 原生菜单 | 非 Windows 平台使用系统默认菜单（提供复制粘贴等快捷键） | `src-tauri/src/main.rs:34` |

---

## 8. 现有实现的问题与重构决策点

按影响程度排序。每条都需要在重构时决定是**保持兼容**还是**修正**。

### 8.1 正确性问题

| # | 问题 | 位置 | 建议 |
|---|---|---|---|
| 1 | 耗时统计依赖解析 tracing 日志字符串，状态是全局单例，并发时数据串 | `http_request.rs:308` | 改为按请求计时 |
| 2 | “中止”只是丢弃结果，后端请求仍在执行 | `views/Collection.tsx:158` | 实现真正的取消 |
| 3 | header 用 `insert` 覆盖：全局头覆盖请求自身的同名头；UA、`Accept-Encoding`、`Cookie` 强制覆盖用户设置的值 | `http_request.rs:451` | 明确优先级（建议：请求自身 > 全局 > 默认值） |
| 4 | 可以把目录拖进它自己的子孙目录，形成环后整棵子树从树上消失 | `items.tsx:355` | 移动前做循环检测 |
| 5 | 删除接口或目录时不清理父目录 `children` 中的引用；删除项目时不删除该项目的变量和本地 KV 数据 | `commands.rs:89`，`commands.rs:119` | 级联清理 |
| 6 | 导出目录时目录记录没有被收集，层级结构丢失 | `item_dropdown.tsx:92` | 修正 |
| 7 | 自有格式导入沿用原 ID：导入同一个库时接口会换新 ID 重试，但目录的 `children` 仍指向旧 ID；目录没有重试 | `import_api.ts:488` | 导入时整体重映射 ID |
| 8 | 备份恢复先清空再插入，没有事务，中途失败会丢失全部数据；同一天的备份文件会被覆盖 | `database.rs:121` | 使用事务；文件名带时间 |
| 9 | 新导入的顶层项没有写入 `topTreeItems`，总是排在最前面 | `import_api.ts:350` | 追加到排序末尾 |
| 10 | `content-type` 是 JSON 但内容不是合法 JSON 时，`JSON.parse` 抛错导致响应无法显示 | `http_response.ts:152` | 解析失败时按文本显示 |
| 11 | 响应头的 value 不是可见 ASCII 时 `to_str()?` 失败，整个请求报错 | `http_request.rs:534` | 做宽松解码 |
| 12 | 模板函数：参数不能包含 `)` 或 `,`；一个表达式解析失败后，后面的都不再解析；替换只替换第一处；`readFile` 不接 `base64` 时输出 `1,2,3…` 这样的字节串 | `fn.ts:61` | 重写为正式的解析器 |
| 13 | 先对 Form 的 JSON 文本整体做模板替换再解析，函数结果中如果含有引号会破坏 JSON | `http_request.ts:285` | 解析为 KV 后再逐项替换 |
| 14 | cURL 导入时把 `-b` 映射为 `Set-Cookie`；生成 cURL 时 body 中的单引号没有转义，Multipart 也没有生成 `-F` | `curl.js:118`，`http_request.ts:127` | 修正 |
| 15 | Multipart 上传的文件名是完整的本地路径 | `http_request.ts:220` | 改为只用文件名 |

### 8.2 设计限制

| # | 问题 | 建议 |
|---|---|---|
| 16 | 每次请求都新建 Client，没有连接复用；只支持 HTTP/1.1；不跟随重定向；没有代理 | 新的 HttpClient 设计时一并考虑 |
| 17 | 响应 body 先 base64 再走 IPC，历史记录中也保存完整的 base64 body | 没有 IPC 后直接使用 bytes；历史记录考虑限制大小或存成文件 |
| 18 | 树的 method / uri 用正则从 JSON 字符串中提取 | 解析一次后缓存，或在表中冗余存储 |
| 19 | `children` 用逗号分隔的字符串，并用 `includes` 做子串判断；顶层顺序存在 KV 而不是数据库 | 可以保持兼容；新增排序字段或单独的关系表更稳妥 |
| 20 | `selectedID` 全局只有一个，不区分项目 | 改为按项目保存 |
| 21 | 修改接口不会刷新所属项目的 `updatedAt`，所以首页“最近修改”排序只反映项目信息本身的修改 | 决定是否要级联刷新 |
| 22 | 快捷键只判断 `metaKey`，Windows / Linux 上不可用 | 按平台映射为 Ctrl / Cmd |
| 23 | 拼音多音字组合数是乘积，名称较长时组合数会爆炸 | 改为逐字匹配（DFS）或预先建立索引 |
| 24 | `enabled` 用字符串 `"0"` / `"1"`；时间用字符串存储 | 保持数据库兼容，Domain 层转为 `bool` / 时间类型 |
| 25 | URI 不支持模板函数；环境变量只能作为 URI 前缀，并且只替换一次 | 决定是否扩展为通用的变量插值 |
| 26 | 变量弹窗按“列表下标”对应变量（`VariableSetting.tsx:104`）；`handleUpdate`（`:128`）是未使用的死代码 | 改为按 ID 对应 |
| 27 | DELETE 等 method 不能带 body | 决定是否放开 |
| 28 | Postman 不导入 headers / auth；Swagger 只支持 2.0 | 按需补齐 |

### 8.3 数据兼容策略（建议）

1. **SQLite（`my_db.db`）**：表结构保持不变，新版本直接读取；如果需要加字段，用 `versions` 表实现迁移。
2. **`cookies.json`**：如果继续使用 cookie_store 库就保持兼容，否则写一个一次性转换。
3. **localforage 数据**：建议不迁移（UI 状态、设置和响应历史），在发布说明中告知用户。
4. **备份 zip 格式**：保持兼容，旧备份可以导入新版本。

---

## 附录 A：Tauri Command 清单（L1 / L2 接口参考）

| Command | 参数 | 返回 | 对应服务 |
|---|---|---|---|
| `close_splashscreen` | — | — | Bootstrap |
| `init_tables` | — | — | Bootstrap |
| `get_latest_version` / `add_version` | `Version` | `Version` | Bootstrap |
| `export_tables` | — | 文件名 | BackupService |
| `import_tables` | `file` | — | BackupService |
| `add/update/list/delete_api_collection` | `APICollection` / `id` | `APICollection` / 删除条数 | CollectionService |
| `add/update/list_api_folder` | `APIFolder` / `collection` | `APIFolder[]` | TreeService |
| `delete_api_folder` | `id` | `{folders, settings}`（被删除的 ID） | TreeService |
| `add/update/list_api_setting` | `APISetting` / `collection` | `APISetting[]` | TreeService |
| `delete_api_settings` | `ids` | — | TreeService |
| `add/update/delete/list_variable` | `Variable` / `ids` / `(collection, category)` | `Variable[]` | VariableService |
| `do_http_request` | `api, req, timeout` | `HTTPResponse` | RequestService |
| `list_cookie` / `add_cookie` / `delete_cookie` / `clear_cookie` | `Cookie` | `string[]`（cookie_store 内部 JSON） | CookieService |

## 附录 B：建议的实施顺序

1. **L0 Domain**：model、tree（构建 / 过滤 / 移动）、template、request_builder、response、curl、importers、search。这些都是纯函数，可以直接拿现有行为编写单元测试（先固化现状，再按第 8 节修正）。
2. **L1 Infrastructure**：兼容现有表结构的 Repository、KvStore、带计时和取消的 HttpClient、CookieStore、Platform trait。
3. **L2 Services**：按 M1–M15 的顺序实现用例，用内存版 L1 编写集成测试。
4. **L3 State**：Dashboard / Workspace / Editor / Response / Send 状态与事件。
5. **L4 GPUI View**：最后接入。
