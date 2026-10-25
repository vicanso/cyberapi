# 第 4 批设计：接口级设置、认证类型与继承、命名环境

> 状态：草案，待确认（2026-10-25）。对应 [vicanso/cyberapi](https://github.com/vicanso/cyberapi/issues) #71、#66、#61，见 [issue-plan.md](./issue-plan.md) 第 4 批。确认前不动代码；第 7 节是需要决定的问题。

三个 issue 都要改项目的数据：#71 和 #66 改接口保存的请求，#66 还要让目录和项目带设置，#61 给变量加上“属于哪个环境”。它们共用一套兼容和存储的做法，所以放在一起设计。

## 1. 现状

| 数据 | 模型 | 桌面（Turso） | bridge（PG / MySQL） | 同步 / 导出 / 备份 |
|---|---|---|---|---|
| 接口的请求 | `HttpRequest`（JSON） | `api_settings.setting` | `apis.request` | `ProjectConfig.apis`；导出的 `setting`；备份 `api_settings.json` |
| 目录 | `Folder`（名称、子项） | `api_folders` | `folders` | `ProjectConfig.folders`；导出的 `children`；备份 `api_folders.json` |
| 项目 | `Collection`（名称、说明） | `api_collections` | `collections` | `ProjectConfig.name` / `description`；备份 `api_collections.json` |
| 变量 | `Variable`（类别、名称、值、启用） | `variables` | `variables` | `ProjectConfig.variables`；备份 `variables.json`（导出不含变量） |

现在的规则：

- **认证：** `HttpRequest.auth` 是键值列表，第一条启用的作为 Basic（F4.9），值不做模板求值。
- **发送设置：** 超时、跟随重定向、证书校验、HTTP 版本都是全局的（`AppPrefs.timeouts` / `SendPrefs`），每次发送时读最新值（#42）。
- **变量：** 环境变量只用作 URI 开头的 `{{HOST}}`；自定义变量由 `value(name)` 读取；全局请求头加在每个请求上。三类都是“同名的取第一条启用的”。读者现在切换 dev / prod 的办法，是建两条同名的 `HOST`，启用其中一条。

## 2. 共同的约束

### 2.1 旧版本的客户端不能把新数据抹掉

已经发布的客户端读请求、目录、变量时，会把不认识的字段丢掉。之后它一旦保存这条记录，或者同步整个项目（`TreeStore::replace` 整份推送），新字段就没了。会这样做的客户端：

- 桌面 0.2.x 打开服务器项目，同步时整份推送。
- 网页版：bridge 升级前就打开着的标签页，还在跑旧的 wasm。

要用两项措施，随第一项模型改动一起上线：

1. **保留不认识的字段。** `HttpRequest`、`Folder`、`Variable` 加上 `#[serde(flatten)] extra: serde_json::Map`，读进来什么，写回去还是什么。这只对从新版本开始的客户端有效，已发布的版本做不到。
2. **bridge 只接受不比它旧的写入者。** 桌面经 bridge 的请求和网页都带上 `X-CyberAPI-Version`。bridge 对 `/v1/store` 里会修改数据的调用，如果发来的版本比 `MIN_WRITER_VERSION` 旧（没带这个头也算旧），就拒绝：`StoreError::Refused { reason: Refusal::ClientTooOld }`。读不受影响。
   - **网页：** 收到这个拒绝就提示刷新页面。
   - **桌面：** 提示更新应用，在这之前只能只读地打开服务器项目。
   - **常量：** `MIN_WRITER_VERSION` 在改模型的那个版本里设为该版本；部署说明的“升级”一节写明，团队里的桌面版要一起升级。

本机自己的项目只有一个写入者，不受影响。备份恢复到旧版时，新字段会被忽略（旧版用 SeaORM 的 `from_json`，不认识的键直接跳过，不会报错），这在部署说明和更新日志里写一句即可。

### 2.2 存储只加新表，不改已有的表

- **不改旧表：** 桌面库里那 5 张旧版的表照原样保留（`MIGRATIONS` 第 2 步曾给 `api_collections` 加过列，但这次没必要）。
- **MySQL 的限制：** 迁移的每条语句都要能重复执行，而 MySQL 8 没有 `ADD COLUMN IF NOT EXISTS`。只用 `CREATE TABLE IF NOT EXISTS` 就不会碰到这个问题。

所以新数据都放在旁边的表里，桌面和 bridge 用同样的结构：

| 新表 | 内容 | 来自 |
|---|---|---|
| `folder_settings` | 目录 id → 设置的 JSON | #66 |
| `project_settings` | 项目 id → 设置的 JSON | #66 |
| `environments` | 环境：id、项目、名称、顺序、时间 | #61 |
| `variable_environments` | 变量 id → 环境 id（不在表里的是“所有环境共用”） | #61 |

接口请求的新字段（#71、#66）直接放在请求的 JSON 里，不需要新表。

### 2.3 每处都要带上新数据

每处改动都要按 CLAUDE.md 的规则做：先在 `conformance` 里写检查，故意改坏 `MemoryStore` 确认检查会失败，再改各后端；给 Store trait 加方法时同时改 `Call`、`dispatch`、`BridgeStore`。需要带上新数据的地方：

- **同步：** `ProjectConfig` 加字段，`TreeStore::replace` 和 `config` 一并读写；网页草稿（`drafts_local`）整份序列化，自动带上。
- **仅本地条目：** `local_only::fingerprint` 要把目录设置、环境、变量所属的环境算进去，否则只改这些时不会显示“未同步”；`syncable` / `restore` 照常剥掉、放回仅本地的目录。
- **备份：** 目录行加 `settings`、变量行加 `environment`、项目行加 `settings`，另加 `environments.json`。旧版恢复时会忽略这些。
- **导出 / 导入（自有格式）：** 目录记录带上 `settings`，`ImportItem::Folder` 随之带设置。导出本来就不含变量，环境也不导出。
- **MCP：**
  - `get_api` 给出接口实际生效的认证（含继承来的），密钥打码。
  - `get_variables` 按环境分组。
  - `send_api` 用该账号在这个项目上选的环境，也可以用参数指定。
- **修订号：** 目录设置、项目设置、环境的增删改、变量换环境都算共享的修改，要更新 `updated_at` 和 `revision`。

## 3. #71 接口级的发送设置

### 模型

`HttpRequest` 加：

```rust
/// What this API overrides of the settings' sending (#71); each left
/// unset follows the settings.
#[serde(default, skip_serializing_if = "RequestSettings::is_empty")]
pub settings: RequestSettings,

pub struct RequestSettings {
    /// 0 follows the settings, as `Timeouts` reads 0 there.
    pub timeouts: Timeouts,
    pub follow_redirects: Option<bool>,
    pub accept_invalid_certs: Option<bool>,
    pub http_version: Option<HttpVersion>,
}
```

### 规则

`cyberapi_core` 里写一个纯函数，按接口的设置叠在全局设置上，算出实际生效的值：超时按项，`0` 表示跟随全局；其余是 `Option`，`None` 跟随全局。

在 `cyberapi_service` 里只在一处使用它：`Sender::for_request(&HttpRequest) -> Sender`。`send_watched`、`code`（cURL 的 `-L`、`-k` 跟着变）、`introspect`、`reflect` 都先调用它。网页版同样生效：证书校验和 HTTP 版本随 `PreparedRequest` 交给 bridge，跟随重定向本来就在页面这边执行。

### 界面

请求编辑器加一个“设置”标签（`RequestTab::Settings`，按接口记住所在的标签）：

- **HTTP 版本：** 跟随设置（显示当前全局值）/ 自动 / 1.1 / 2。
- **三个超时：** 输入框，占位文字显示全局值，留空就是跟随。
- **跟随重定向、校验证书：** 三态，跟随设置 / 开 / 关。
- **gRPC 接口：** 只显示超时和证书两项，因为它固定走 HTTP/2，也不跟随重定向。

响应面板对 302 的提示改为：“在这个接口的‘设置’里打开跟随重定向，或在全局设置里打开”，并提供按钮直接切到这个标签。

### 兼容

旧客户端读不到 `settings`，会按全局设置发送。按 2.1，保存时这个字段会保留，旧客户端也写不进去。

## 4. #66 认证类型与继承

### 模型

```rust
/// How a request signs in (#66).
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Auth {
    /// The folder's, or the project's above it: what a request without
    /// one of its own does.
    Inherit,
    /// None, whatever is above.
    None,
    Basic { username: String, password: String },
    Bearer { token: String },
    ApiKey { name: String, value: String, place: KeyPlace }, // Header | Query
}
```

- **接口：** `HttpRequest` 加 `authentication: Option<Auth>`。
- **目录：** `FolderSettings { auth: Option<Auth>, headers: Vec<KvParam> }`。
- **项目：** `ProjectSettings { auth: Option<Auth> }`。项目级的请求头已经有了，就是全局请求头，不再另设一份。

### 读旧数据

- `authentication` 有值时，以它为准。
- 没有时，`auth` 列表里第一条启用的读成 `Basic`。
- 两者都没有时，读成 `Inherit`。

现在没有任何目录或项目带认证，所以 `Inherit` 最终就是不认证，和以前完全一样。新建的接口默认也是 `Inherit`。保存为 Basic 时，旧的 `auth` 列表同时写一份，旧客户端仍然能带上 Basic；Bearer 和 API key 旧客户端发不出去，靠 2.1 的版本门槛挡住它们的写入。

### 生效规则

`cyberapi_core` 里写纯函数：

```rust
fn effective_auth(api, folders from nearest, project) -> (Auth, Source)
fn effective_headers(...)
```

- **认证：** 从接口往上找，遇到第一个不是 `Inherit` 的就用它；一直到项目都没有，就不认证。`Source` 说明来自哪一层，界面显示“继承自 目录 xx / 项目”。
- **请求头：** 依次是全局请求头 → 外层目录 → 内层目录 → 接口自己的，同名的以里层为准。延续现在“请求自己的优先”（#3）的规则。
- **模板：** 认证里的值都做模板求值，可以写 `{{value(token)}}`。Basic 现在不求值，这是一处行为变化，见第 7 节。
- **API key 放在 Query 时：** 追加到 URL 的查询参数里。
- **其他出口：** “实际发出的请求”（#58）的来源标“认证”（`HeaderSource::Auth`），注明来自哪一层；cURL / 复制为代码因为都基于 `PreparedRequest`，自然带上。

### 存储与 API

`TreeStore` 加 `set_settings(collection, target: SettingsTarget, settings) -> …`：

- `SettingsTarget::Folder(id)` 写 `folder_settings` 表，`SettingsTarget::Project` 写 `project_settings` 表。
- 返回更新后的记录。
- 要更新修订号，查看者无权调用。
- `TreeData` 和 `ProjectConfig` 都带上这些设置。

### 界面

- **接口：** “认证”标签改为先选类型（继承 / 无 / Basic / Bearer / API key），再填对应字段。密钥打码，可以切换显示。选“继承”时显示实际生效的认证和它来自哪一层。
- **目录：** 右键菜单加“目录设置…”，弹窗里有认证和请求头（复用 `kv_editor`）。网页版的弹窗要按 CLAUDE.md 的写法：用 `ok_button`，二次确认用 `inline_confirm`。
- **项目：** 工作区设置菜单里，在“全局请求头”旁边加“项目认证…”。
- **查看者：** 都是只读。

### 以后可以接着做

Postman、Insomnia 的集合 / 目录 / 请求级认证，以及 OpenAPI 的 `securitySchemes`，导入时映射到这里。不放在这一批。

## 5. #61 命名环境

### 为什么不自动转换现有数据

计划里写过“同名的环境变量行各自成为一个环境的值”，但做不到：旧数据里只有 `HOST = http://dev` 和 `HOST = http://prod` 两行，既没有环境名，也说不出 `TOKEN` 的哪一行和 `HOST` 的哪一行属于同一个环境。所以现有变量原样保留，作为“所有环境共用”的那一层；环境是新加的一层，读者自己建。也可以加一个可选的手动整理工具：把同名的几行分到读者指定的环境里。见第 7 节。

### 模型

```rust
pub struct Environment { id: EnvironmentId, collection, name, position, created_at, updated_at }

// Variable 加：
/// The environment it belongs to; `None` is every environment's.
pub environment: Option<EnvironmentId>,
```

- **当前环境：** 属于读者自己，存在 `WorkspacePrefs.environment: Option<EnvironmentId>`，和置顶一样不同步、不进备份。团队里每个人可以停在不同的环境上。
- **环境的定义：** 属于项目，会同步，也要更新修订号。

### 生效规则

在 `cyberapi_core::send` 里写一份：对每个类别里的每个名字，先在当前环境里找第一条启用的，找不到再到共用的那层里找。

- **URI 前缀：** 用这条规则。
- **`value(name)`：** 用这条规则。
- **全局请求头：** 同名的以当前环境的为准，其余两层都带上。
- **没有当前环境时：** 只看共用的那层，和现在完全一样。
- **找不到 URI 前缀的变量时：** #50 的报错要写明当前是哪个环境。

具体做法：`Sender` 带上 `environment: Option<EnvironmentId>`，由 `ApiState` 从 `WorkspaceState` 取得；MCP 的 `send_api` 取该账号在这个项目上选的环境，或者用参数指定的环境名。

### 存储与 API

`VariableStore` 加：

- `environments(collection)`
- `create_environment`、`rename_environment`
- `delete_environment`：同时删掉这个环境里的变量
- `reorder_environments`

`NewVariable` 和 `VariableEdit` 都带上 `environment`，也就是说变量可以改到另一个环境。不另外加一个 trait，`Storage` 的结构保持不变。

### 界面

- **切换器：** 工作区标题栏，项目名旁边放“环境：dev ▾”，下拉列出“不使用环境”和各个环境。网页版同样有。
- **变量弹窗：** 顶部加一行环境选择：共用 / dev / prod / 新建…，可以重命名、删除。列表显示所选那层的变量；选某个环境时，共用层里被它覆盖的同名变量灰着显示在下方，方便对照。
- **地址栏的“Env”：** 仍然是选 URI 前缀变量（HOST）；当前环境决定 HOST 取哪个值。悬停提示写出当前生效的值。

### 以后可以接着做

Insomnia 的 base / sub 环境、Postman 的环境文件，导入时映射到命名环境。不放在这一批。

## 6. 实施顺序

每一步都是可以单独提交、单独发布的状态。每一步都要跑 `make test-sql`，因为都会动到 bridge 的存储。

1. **兼容的准备（2.1）：**
   - `extra` 字段，并测试来回读写不丢字段。
   - 版本头和 `MIN_WRITER_VERSION`：bridge 的测试覆盖旧版本被拒、同版本通过，网页版收到拒绝时提示刷新。
2. **#71 接口级设置：** 模型、生效规则的单元测试、`Sender::for_request`、“设置”标签、302 提示。
3. **#66a 接口的认证类型：** `Auth`、读旧数据的规则（测试旧的 Basic 列表）、请求的组装、认证标签、打码。
4. **#66b 目录 / 项目设置与继承：**
   - 先做存储：conformance → `MemoryStore` → Turso → SQL → bridge 的 `Call`。
   - 再做生效规则、两个弹窗、同步的指纹、备份和导出。
5. **#61a 环境的存储与规则：** 存储（同上的顺序）、生效规则、`Sender.environment`、MCP。
6. **#61b 环境的界面：** 切换器、变量弹窗；可选的手动整理工具。

## 7. 需要你决定的问题

| # | 问题 | 我的建议 |
|---|---|---|
| 1 | bridge 是否拒绝旧版本客户端的写入（2.1 第 2 项）？拒绝的话，团队里的桌面版要一起升级 | 拒绝。不拒绝的话，旧客户端一次同步就会抹掉别人的新设置 |
| 2 | 项目级认证要不要做？项目级的请求头已经有全局请求头了 | 做认证，请求头不再另设 |
| 3 | 接口没设置认证时默认是“继承” | 是。现在没有任何上层带认证，行为不变 |
| 4 | Basic 的用户名和密码改为做模板求值（行为变化：密码里有 `{{…}}(…)` 字样时会被当成函数） | 改，和其他认证类型一致 |
| 5 | 当前环境属于每个读者自己，而不是整个项目共用 | 属于读者自己 |
| 6 | 不自动把同名的环境变量转换成命名环境；要不要做手动整理工具 | 不自动转换；工具放在 #61b 之后、可选 |
| 7 | #71 的设置只在接口上，不做目录级继承 | 只在接口上。以后需要时再放进 `FolderSettings` |
