# 0.4.2 之前的排查：bug、性能与功能取舍

2026-10-31。范围是 0.4.1 之后新增与改动的功能（F8–F16、B9–B11、D8、G2b、G3、H2、A4b、A7b/F7b、A8–A10、PERF-3b、#84，`5e73225`…`69fea4a`）里的 bug、性能热点，以及对功能必要性与交互简化的评估。本次只做代码审查与运行验证，没有修改功能代码。“核实”一栏：**已运行验证**是通过启动 `cyberapi-cli` / `cyberapi-bridge` 测试进程实测复现；**已核实**是对照代码完整走通调用链确认必现；**读代码确认**是沿路径读过确认逻辑缺陷。

编号：`BUG-n` 是错误行为或安全缺陷，`PERF-n` 是性能问题，`Q-n` 是功能必要性、删减或边界取舍建议。修的时候一项一个提交，提交信息写编号（`fix(BUG-1): …`）；修完在汇总表的“核实”一栏和正文标题后标“已修复”。

## 建议的顺序

1. **先修安全与数据泄露问题**：BUG-1（在线文档与 MCP 表单脱敏把 `"key"` 打码却明文输出 `"value"` 密码）、BUG-2（bridge `/v1/mock` 原样回传 `text/html` 与 `Set-Cookie` 造成同源存储型 XSS 与会话固定）、BUG-3（桌面本机 MCP 绕过服务器副本隔离与只读角色、且写入不标 `unsynced`）、BUG-4（变量矩阵视图在某环境补填秘密变量时明文写进同步库）。
2. **再修请求发送与编辑器状态错乱**：BUG-5 与 BUG-6（结合 **Q-1** 一起做：修 `:param` 路径参数遮蔽运行器数据行 / 提取值 / 环境变量，并删掉请求“设置”标签页里重复的接口变量编辑器、收敛路径参数同步逻辑）、BUG-7 与 BUG-8（项目内查找替换的并发保存竞态、误关独立窗口、损坏 gRPC / GraphQL 请求体 JSON）。
3. **接着修热门路径性能与剩余交互问题**：PERF-1（`/v1/mock` 与 MCP 令牌每次请求的多余 SQL 查询及整树重建）、PERF-2…PERF-4，以及 BUG-9…BUG-13。

## 汇总

| 编号 | 功能 | 严重度 | 一句话 | 核实 |
|---|---|---|---|---|
| BUG-1 | G3 / H1 | 高 | `mcp::masked_body` 对表单 JSON 数组把字段名 `"key"` 打码成 `"***"` 而明文暴露 `"value"`；在线文档（`/v1/docs`）把表单原始 JSON、gRPC base64 描述符原样当请求体输出，漏掉认证、多环境 `{{ENV}}` 过滤与 Query URL 编码 | 已核实；已修复 |
| BUG-2 | G2b | 高 | bridge `/v1/mock/{project}/{*rest}` 原样转发示例里的 `Content-Type: text/html` 与 `Set-Cookie` 等响应头，在 bridge 同源下可触发存储型 XSS 与会话 Cookie 注入 | 已核实；已修复 |
| BUG-3 | H2 | 高 | 桌面版本机 MCP 直接读写底层 `TursoStore`：暴露已退出登录的隐藏服务器副本、把只读查看者当成管理者放行改写、改写后不标 `unsynced`（下次轮询被服务器冲掉）且不通知打开中的工作区 | 已核实；已修复 |
| BUG-4 | D8 | 高 | 变量弹窗在“矩阵对比”视图下为秘密变量补填某环境值时按普通变量明文写入项目库（随同步泄露给团队）；切换单层/矩阵视图不刷新已有输入框的值；“归并重复”漏迁秘密标记并留孤儿 ID | 已核实；已修复 |
| BUG-5 | F12 / D5 | 中高 | `send::resolve_path_uri` 先查 `request.variables` 再查 `cx.template.custom`，导致 `:param` 的默认值遮蔽运行器 CSV/JSON 数据行、提取值（`extractions`）、CLI `--var` 与环境变量；落到 `custom` 分支时漏调 `render()` | 已运行验证；已修复 |
| BUG-6 | F12 / D5 | 中高 | `Query` 标签的 `Path parameters` 与 `Settings` 标签的 `Variables` 各自持有 `KvEditor` 读写同一份 `request.variables` 互相覆盖；地址栏边输入 `:u` → `:us` → `:user_id` 边留下幽灵变量并打断参数编辑框的 200ms 防抖 | 已核实；已修复 |
| BUG-7 | F15 / F7 | 中 | 项目内查找替换同时触发接口 `saves` 队列的 `save_now` 与工作区 `gate` 队列的 `tree().replace()` 产生写入竞态；调用 `start_again` 无条件关闭所有独立窗口；打开弹窗前未先 `flush` 当前编辑器 | 已核实；已修复 |
| BUG-8 | F15 | 中 | 项目内查找替换对 `HttpMethod::Grpc`（含 base64 描述符的 `GrpcBody` JSON）与 `ContentType::GraphQl`（`GraphQlBody` JSON）直接做裸字符串替换，易破坏外层 JSON 或损坏二进制描述符 | 已核实；已修复 |
| BUG-9 | B11 | 中 | `breaking_changes_of_schema` 在旧接口无 Schema 时把首次添加的所有必填项误报为破坏性变更 `AddedRequired`；可选父对象下的必填子项被误报为 `AddedRequired`；删父对象时对子孙字段重复报 `RemovedField` | 已核实；已修复 |
| BUG-10 | F11 / F13 / F16 | 中低 | `RequestReplaced` 使示例变少时 `example_sub` 越界导致示例预览跳回首条；在说明页改请求 Schema 或同步后未重新校验 `Body`；说明为空时只读查看者打开说明页仍进入编辑态并显示无效的编辑/预览按钮 | 读代码确认；已修复 |
| BUG-11 | F14 | 中低 | `KvEditor::set_list` 在批量文本模式（`bulk_mode == true`）下不更新 `bulk_editor`，选中历史记录或查找替换后切回键值模式会把旧文本写回 | 读代码确认；已修复 |
| BUG-12 | A7b | 中低 | 压测 `load::run` 对静态请求也每次重新 `prepare`，且最多 50 个并发 worker 每次发送都读写 `Storage::cookies()`，既留存了不该保留的响应 Cookie，又在桌面端争抢唯一的数据库连接锁 | 读代码确认；已修复 |
| BUG-13 | F7b | 低 | 独立窗口（`ApartView`）只在开窗时读一次 `compare_layout()`，不订阅 `AppState`，切换上下/左右布局后已打开的独立窗口不跟着变 | 读代码确认；已修复 |
| PERF-1 | G2b / G3 / H1 | 高 | `auth::session_token` 不跳过 `cat_` 前缀的 MCP 令牌，每次 Bearer 令牌调用 `/v1/mock`、`/v1/docs`、`/v1/mcp` 都白查一次 `session_account`；`/v1/mock` 每次请求都从数据库读整棵树并重建 `mock::Routes` | 已核实；已修复 |
| PERF-2 | H1 / H2 | 中 | MCP `get_history` 调 `HistoryStore::list` 拉取最多 100 条完整响应体（每条最多 2 MB），而实际只用列表摘要和最新一条的正文 | 已核实；已修复 |
| PERF-3 | D8 | 中 | 变量弹窗 `sync_rows` 在单层视图（`matrix == false`）下也无条件执行 `sync_matrix_cells`，每次按键或刷新都创建/遍历 `环境数 × 变量名数` 个隐藏的 `InputState` | 已核实；已修复 |
| PERF-4 | B10 | 中 | `WorkspaceState::source_updates` 在 `SourceView::render`（每帧）和打开右键菜单时在 UI 线程同步克隆整棵树并执行 `merge::preview`（含 Schema 破坏性变更分析） | 已核实；已修复 |
| Q-1 | F12 / D5 | 中 | **功能精简建议**：删除请求“设置”标签页里的“接口变量”编辑器，把 `HttpRequest.variables` 收敛为 `Query` 标签页下纯由 URL `:param` / `{param}` 驱动的“路径参数”（键名只读、紧随 URL 占位符） | 已按建议处理 |
| Q-2 | H2 | 中 | **功能边界建议**：桌面版本机 MCP 对服务器项目副本（`ServerCopy`）应只读（或严格按绑定角色 + `ServerCopy` 走同步标记），且隐藏非当前登录账号的副本 | 已按建议处理 |

---

## Bug 详情

### BUG-1 表单请求体脱敏反把 `"key"` 打码而明文暴露密码，且在线文档请求体/代码示例多处失真（G3 / H1，高）——已修复

- 位置：
  - `crates/cyberapi-core/src/mcp.rs:22-42`（`ENDING` 与 `is_sensitive`）、`:74-100`（`masked_body` / `mask_value`）
  - `crates/cyberapi-core/src/html_docs.rs:441-444`（`Request Body` 渲染）、`:630-721`（`sample_prepared`）
- 场景与原因：
  1. **安全漏洞（表单敏感值完全未脱敏，影响 `/v1/docs` 与 MCP `get_api`）**：当 `content_type` 是 `ContentType::Form` 或 `ContentType::Multipart` 时，`req.body` 存的是 `Vec<KvParam>` 序列化成的 JSON 数组：`[{"key":"password","value":"my-secret","enabled":true,"description":""}]`。`masked_body(&req.body)` 把它当作普通 JSON 对象树递归调用 `mask_value`，按对象的属性名调用 `is_sensitive(k)`：
     - 属性名 `"key"` 因为匹配 `ENDING` 里的 `"key"`（`mcp.rs:29`），被判断为敏感字段，于是字段名 `"password"` 被打码成 `"***"`！
     - 属性名 `"value"` 不在敏感词表里，于是真正的密码 `"my-secret"` **原封不动以明文输出**（变成 `[{"key":"***","value":"my-secret"}]`）！
  2. **在线文档（`/v1/docs`）`Request Body` 与代码示例把内部存储格式原样暴露**：
     - `html_docs.rs:441` 只看 `req.method.has_body() && !req.body.trim().is_empty()`，不看 `req.content_type`：`ContentType::Form` / `Multipart` 已经在上方 `Parameters` 表格渲染过，下面又把内部 `[{"key":"***","value":"..."}]` 原始 JSON 数组印在 `Request Body` 里；`ContentType::None` 但残留了旧 `body` 文本时也会印出来。
     - `HttpMethod::Grpc` 的 `req.body` 是 `GrpcBody` 的内部 JSON（含可能数百 KB 的 base64 编码 `.pb` 描述符 `definitions`），`ContentType::GraphQl` 的 `req.body` 是 `GraphQlBody` JSON：`masked_body` 既不会深入解析内部嵌套的 `message` / `variables` 字符串去打码敏感字段，还会在文档页面直接印出整坨 base64 描述符；同时 `sample_prepared`（`:696-718`）把打码后的 `masked_req` 传给 `code::code`，若 `GrpcBody` 或 `Form` JSON 里的键被改坏，生成的代码片段也会失真。
  3. **在线文档 `sample_prepared` 的三处偏差**：
     - `:635-643` 查找地址前缀 `{{ENV}}` 时只看 `v.category == VariableCategory::Environment && v.enabled && v.name == name`，**没有过滤 `v.environment.is_none()`**（也没有按所选环境过滤），当一个变量只在某个特定命名环境里有值或存在多环境同名行时，会取到排在前面的任意环境值。
     - `:666-676` 拼接 Query 参数 `format!("{}={}", kv.key.trim(), masked(&kv.key, &kv.value))` 没有做 URL 编码，含空格、`&`、`=` 时生成的示例 URL 和代码片段是非法 URL。
     - `:678-694` 只拼了继承的请求头与接口请求头，完全忽略了接口或目录配置的 `Auth`（如 Bearer、API Key、Basic），导致文档里的代码示例缺少认证头/参数，页面上也没有展示接口的认证方式。
- 修复方向：
  - 为请求体提供带 `(HttpMethod, ContentType)` 上下文的脱敏/展示函数（或在 `Form`/`Multipart` 时按 `KvParam.key` 判断 `is_sensitive(&kv.key)` 来打码 `kv.value`，对 `GrpcBody.message` 和 `GraphQlBody.variables` 解析内层 JSON 打码）。
  - `html_docs.rs` 的 `Request Body` 跳过 `ContentType::None`、`Form`、`Multipart`；对 `Grpc` 只展示服务/方法与格式化后的 `message`，对 `GraphQl` 分开展示 `query` 与脱敏后的 `variables`。
  - `sample_prepared` 优先取共用环境（`v.environment.is_none()`）或第一个环境的值，对 Query 键值使用 `encode_uri_component`，并按 `inh.auth` 补上打码后的示例认证头（如 `Authorization: Bearer ***`）及在文档中写出认证方式。
- 修复：新增 `mcp::masked_request_body(method, content_type, body)`，按请求方法与类型分别对 `Form`/`Multipart` 的 `KvParam.value`（按 `KvParam.key` 判断敏感词）、`GrpcBody.message`、`GraphQlBody.variables` 及 JSON 正文脱敏；`html_docs` 在 `Request Body` 中跳过 `None`/`Form`/`Multipart`，将 gRPC 渲染为服务方法 + 脱敏 `message`（不输出 base64 `definitions`）、GraphQL 渲染为 `query` + 脱敏 `variables`；`sample_prepared` 优先匹配共用环境变量、对 Query 参数做 `encode_uri_component`，并补齐继承或接口级 `Auth` 的示例认证头/Query 与文档展示。

### BUG-2 bridge 共享 Mock 路由（`/v1/mock`）原样回传示例响应头，存在同源存储型 XSS 与 Cookie 注入（G2b，高）——已修复

- 位置：`cyberapi-bridge/src/mock.rs:80-93`（`into_response`）。
- 场景与原因：
  - 桌面版的 Mock 服务监听在独立的随机端口（`127.0.0.1:<port>`），而 bridge 的 `/v1/mock/{project}/{*rest}` 挂载在 **与网页版前端及 `/v1/*` API 完全相同的 Origin** 下。
  - `into_response` 把 `Answer.headers`（直接来自项目中编辑者保存的 `ResponseExample.headers`）原样拷贝进 HTTP 响应头，没有任何过滤：
    1. **同源存储型 XSS**：任何对某项目有编辑权限的成员存一个包含 `Content-Type: text/html` 和 `<script>...</script>` 的响应示例，再把 `/v1/mock/<project>/page` 链接发给其他已登录的团队成员或管理员打开（由于浏览器自动携带同源的 `__Host-cyberapi` / `cyberapi_session` Cookie，第 53 行 `signed_in_or_token` 直接放行），脚本即可在 bridge 同源下执行，调用 `/v1/store` 或 `/v1/directory` 接管账号或签发令牌。
    2. **会话 Cookie 注入 / 覆盖**：示例响应头里如果包含 `Set-Cookie: cyberapi_session=...; Path=/`，访问该 Mock 路径的浏览器会被直接种下或覆盖 bridge 的会话 Cookie。
- 修复方向：
  - 在 `cyberapi-bridge/src/mock.rs` 的 `into_response` 中：
    1. 过滤掉危险的响应头：禁止从示例转发 `set-cookie`、`set-cookie2`、`content-security-policy`、`x-frame-options`、`strict-transport-security` 等。
    2. 强制加上沙箱与防嗅探响应头：`Content-Security-Policy: sandbox; default-src 'none'` 与 `X-Content-Type-Options: nosniff`，即使示例是 `text/html` 或 `image/svg+xml`，浏览器也在孤立沙箱中渲染、绝不执行脚本且不读写同源 Cookie / `localStorage`。
- 修复：`mock::into_response` 过滤 `set-cookie`、`set-cookie2`、`content-security-policy`、`content-security-policy-report-only`、`x-content-type-options`、`x-frame-options`、`strict-transport-security`、`clear-site-data`，并强制附加 `Content-Security-Policy: sandbox; default-src 'none'`、`X-Content-Type-Options: nosniff` 与 `X-Frame-Options: DENY`。

### BUG-3 桌面版本机 MCP Server 绕过服务器项目副本隔离、角色权限与未同步标记（H2，高）——已修复

- 位置：
  - `src/states/app.rs:78-82`、`src/states/mcp_local.rs:137-148`（直接把 `store.storage().clone()` 即底层 `TursoStore` 传给 `cyberapi_service::mcp::handle`）
  - `crates/cyberapi-service/src/mcp.rs:262-351`、`794-829`
  - 对照：`src/states/dashboard.rs:285-296`（`ServerState::hides`）、`src/states/copy.rs`（`ServerCopy::desktop`）
- 场景与原因：
  - 桌面版登录后会把服务器项目拉取到本机同一个 `TursoStore` 数据库中作为离线/工作副本（绑定信息记在 `server_copies` 表和 `ServerState` 中）。本机 MCP Server 直接操作裸 `TursoStore`（在 `TursoStore` 里 `CollectionStore::role` 对所有集合恒返回 `Ok(CollectionRole::Manager)`），完全不知道 `ServerState`、`ServerCopyStore` 和打开中的 `WorkspaceState`：
    1. **暴露已退出登录或其他账号的服务器副本**：桌面退出服务器登录或切换到另一个账号后，首页通过 `ServerState::hides` 隐藏不属于当前会话的服务器副本，但本机 MCP 的 `list_projects`、`search_apis`、`get_api` 会把这些隐藏项目全部列出来并允许读取。
    2. **越权修改与发送**：当前账号在服务器上只是某项目的只读查看者（`CollectionRole::Viewer`），本机 MCP 却按 `TursoStore` 返回的 `Manager` 放行 `create_api`、`update_api` 和生产环境外的修改。
    3. **修改不标 `unsynced`，被下次轮询静默覆盖**：通过本机 MCP 对服务器项目的桌面副本执行 `create_api` / `update_api` 时，只改了 `TursoStore` 的树，**没有调用 `ServerCopyStore::bind` 将 `unsynced` 设为 `true`**，也没有经过 `LocalItems` 处理；工作区或首页下一次 `SERVER_POLL`（30 秒）或重新打开项目时，看到 `unsynced == false`，就会在服务器有新修订时直接拉取覆盖，或者永远不会提示“未同步”，MCP 写入的内容悄无声息丢失。
    4. **打开中的工作区与标签页完全脱节**：当该项目正在桌面窗口中打开时，`WorkspaceState` 内存里持有 `TreeData` 副本、`kept` 里持有打开的 `ApiState`。本机 MCP 直接改了数据库，既不通知 `WorkspaceState` 重读树，也会在用户随后保存编辑器或关闭标签页时被 `ApiState::save_now` 用内存里的旧请求覆盖回去。
- 修复方向：见 **Q-2**。
- 修复：在 `cyberapi_service::mcp::LocalMcpContext` 中注入 `hidden_projects`、`server_bound_projects`、`role_overrides` 与 `on_project_mutated` 回调（由桌面版从 `ServerState` 动态提供）：隐藏非当前登录账号的服务器副本，对查看者角色拦截修改，对当前账号的服务器项目副本拒绝 `create_api` / `update_api` 并给出明确提示，对纯本机项目的修改通过 `LocalMcpEvent::ProjectChanged` 通知主线程刷新 `DashboardState` 与正在打开的 `WorkspaceState`（调用 `reload_after_external_write` 在 `self.gate` 队列内重读树并原地更新 `kept` 标签页的请求而不关闭独立窗口）。

### BUG-4 变量弹窗“矩阵对比”视图下补填秘密变量会明文写库，且视图切换与归并重复有状态缺陷（D8，高）——已修复

- 位置：
  - `src/views/variables_dialog.rs:452-569`（`sync_rows`、`sync_matrix_cells`）、`:303-336`（视图切换按钮）、`:571-618`（`save_matrix_cell`）
  - `src/states/variables.rs:206-226`（`create_at`）、`:300-317`（`deduplicate`）
- 场景与原因：
  1. **安全漏洞（矩阵视图补填秘密变量导致明文入库并同步给团队）**：
     - 在 `sync_matrix_cells`（`variables_dialog.rs:514-516`）中，每个单元格是否按秘密变量处理写的是：
       ```rust
       let var = state.find_at(name, env.as_ref());
       let id = var.map(|v| v.id.clone());
       let secret = var.is_some_and(|v| state.is_secret(&v.id));
       ```
     - 假设变量 `API_KEY` 在 `共用（Shared）` 层已创建并加锁标为秘密（`secret = true`），此时在 `prod` 环境列还没有对应的 `Variable` 行（`var == None`）。因此 `prod` 这一格算出的 `secret` 是 **`false`**！
     - 当用户在矩阵视图的 `prod` 列输入生产密钥时，输入框不仅**不打码**，失焦触发 `save_matrix_cell` 时因为 `!cell.secret` 且 `cell.id.is_none()`，会直接调用 `state.create_at("API_KEY", "sk-prod-12345", Some(prod))`（`:605-609`），把生产环境的密钥**作为普通明文变量写进项目数据库的 `variables` 表**（而不是把新建的 `VariableId` 标为秘密并把值写进本机私有的 `PrefStore::secret_values`）！一旦同步项目，生产密钥就会明文推送到服务器和所有团队成员。
  2. **单层视图与矩阵视图来回切换时，已有输入框保留旧文本**：
     - 在单层视图中修改某变量的值，`sync_rows`（`:462-477`）只在 `self.rows` 里找不到该 `id` 时才新建 `InputState`，对已存在的行**不会更新 `row.value_input` 的文本**；同样，在矩阵视图改完值切回单层视图（或反过来），另一套视图里早已创建的 `InputState` 仍显示修改前的旧值。
  3. **“归并重复”（`deduplicate`）丢失重复行的秘密标记并留下孤儿 ID**：
     - `states/variables.rs:300-317` 的 `deduplicate` 只保留每组 `(category, environment, name)` 的第一条，直接 `variables.retain(...)` 删掉后面的重复行，既没有从 `ProjectSettings.secret_variables` 和 `PrefStore::secret_values` 中清理被删掉的 `VariableId`，如果用户恰好把后面那条重复行标成了秘密，归并后秘密标记和秘密值都会丢失。
- 修复方向：
  - 矩阵视图中判断某一行变量是否为秘密应按**变量名**汇总（只要同名变量在任一环境或共用层是 `secret`，整行所有环境的单元格都应视为 `secret = true`）；在尚无 `Variable` 行的秘密单元格首次输入非空值时，应先创建 `value = ""` 的 `Variable`、将其 `id` 加入 `secret_variables`，再把实际值存入 `PrefStore::secret_values`。
  - `sync_rows` 与 `sync_matrix_cells` 对当前未处于焦点（`!input.read(cx).focus_handle(cx).is_focused(window)`）的已有 `InputState`，若其文本与最新 `shown_value` 不一致则调用 `set_value` 同步。
  - `deduplicate` 合并同组重复项时，若任一重复项为秘密则把保留项也标为秘密并迁移秘密值，同时从 `secret_variables` 与 `secret_values` 中移除被删掉的 `VariableId`。
- 修复：`VariablesState::is_name_secret` 按变量名跨环境汇总秘密状态，`set_secret` 同步同名行的秘密状态，`create_at` / `set_value` 对秘密变量名自动在数据库中存空字符串、把 `VariableId` 加入 `secret_variables` 并将真实值写入本机 `PrefStore::secret_values`；`sync_rows` 与 `sync_matrix_cells` 在 `syncing` 保护下对未聚焦的已有 `InputState` 同步最新 `shown_value` 与掩码状态；`deduplicate` 与 `delete_environment` 合并/删除时保留秘密标记与本机秘密值并清理孤儿 `VariableId`。

### BUG-5 `send::resolve_path_uri` 优先级倒置，`:param` 默认值遮蔽运行器数据行、提取值与环境变量（F12 / D5，中高）——已修复

- 位置：`crates/cyberapi-core/src/send.rs:263-296`（`resolve_path_uri`），对比 `crates/cyberapi-core/src/inherit.rs:51-75`（`Inherited::values`）。
- 场景与原因：
  - D5 规定的变量优先级（在 `Inherited::values` 和 `FEATURES_zh.md` 中明确写出）是：**运行中的值（CSV/JSON 数据行、前置接口提取值 `extractions`、CLI `--var`） > 接口变量 > 目录变量 > 当前命名环境 > 共用变量**。
  - 然而 F12 在 `RequestEditor::sync_path_params` 中会把 URL 里的 `:id` / `{id}` 自动写入 `request.variables`；而在发送准备阶段，`resolve_path_uri`（`send.rs:272-291`）先查 `request.variables`，找不到（或值为空）才查 `cx.template.custom`（即 `Inherited::values` 合并后的表）！
  - 这导致两个直接后果：
    1. **默认值遮蔽所有高层级动态值**：只要用户在 `Path parameters` 里给 `:id` 填了一个平时单接口调试用的默认值（如 `default_id`，或者从 OpenAPI / Postman 导入自带了示例值），在运行器（A3）跑 CSV/JSON 数据驱动测试、用前置接口提取的 `id`（A2）、用命令行 `cyberapi-cli run --data / --var`（A4），或者在环境变量里配了 `id` 时，URL 里的 `:id` 永远发的是 `request.variables` 里的静态默认值！
    2. **落到 `cx.template.custom` 分支时漏调 `render(v)`**：`send.rs:279` 对 `request.variables` 调了 `render(&kv.value)`，但 `:285-289` 落到 `cx.template.custom.get(name)` 时直接返回原始字符串，如果 `--var id="{{uuid()}}"` 含模板函数则不会被求值。
- 运行验证：
  - 使用编译出的 `cyberapi-cli` 对本地测试 HTTP 服务实测运行：接口 URL 为 `http://127.0.0.1:<port>/users/:id?q={{value(id)}}`，接口含路径变量 `id = default_id`，传入 CSV 数据文件 `id=csv_row_id` 执行 `cyberapi-cli run col.json --data data.csv`。
  - 实测服务端收到的请求路径为：`/users/default_id?q=csv_row_id`——同一个请求里，`?q={{value(id)}}` 正确取到了 CSV 数据行的 `csv_row_id`，而路径上的 `:id` 却被默认值 `default_id` 遮蔽！
- 修复方向：
  - `Inherited::values`（`inherit.rs:64-73`）在构建 `cx.template.custom` 时，本来就已经按“共用 → 当前环境 → 目录 → 接口（`request.variables`） → 运行中的 `values`”的正确顺序把非空值写进了 `cx.template.custom`！
  - 因此 `resolve_path_uri` 应该**先查 `cx.template.custom.get(name)`**（若非空则对其调 `render(v)` 并 `encode_uri_component`），或者区分哪些键来自更高优先级的运行时 `values`，避免 `request.variables` 反过来压过运行器数据行与提取值。
- 修复：`Inherited::values` 跳过空的接口路径参数值（避免空占位符抹掉同名的环境/目录变量），`raw_path_param` 优先查询运行期合并的值（`run::with_values` 与 `cx.template.custom`）再回退到接口路径参数、目录变量与环境变量，`resolve_path_uri` 对取出的值统一执行 `render(v)` 与 `encode_uri_component`。

### BUG-6 `Query` 标签的 `Path parameters` 与 `Settings` 标签的 `Variables` 双编辑器互相覆盖，且改 URL 留下幽灵变量并打断防抖（F12 / D5，中高）——已修复

- 位置：
  - `crates/cyberapi-core/src/model.rs:767-807`（`merge_path_variables`）
  - `src/views/workspace/request_editor.rs:312-319`、`:354-358`、`:1080-1081`、`:1162-1187`（`sync_path_params`）
  - `src/views/workspace/request_settings.rs:185-194`（`RequestSettingsView` 的 `variables` `KvEditor`）
- 场景与原因：
  1. **两个 `KvEditor` 同时绑定 `request.variables`，修改互相覆盖**：
     - `RequestEditor` 里的 `path_params`（在 `Query` 标签页上方显示）和 `RequestSettingsView` 里的 `variables`（在 `Settings` 标签页下方显示）读写的都是 `HttpRequest.variables`。
     - 在 `Query` 标签页修改路径参数值会调 `api.set_variables(...)`，但不会更新 `RequestSettingsView.variables`；随后切到 `Settings` 标签页改动那里的变量（或触发保存），`RequestSettingsView` 就会用自己手里旧的 `KvEditor` 列表调 `api.set_variables(...)`，把刚才在 `Query` 标签页填的路径参数值覆盖掉！反过来从 `Settings` 改完切回 `Query` 也会被 `path_params` 覆盖。
  2. **地址栏打字产生一串幽灵变量**：
     - 地址栏每敲一个字母都会立即触发 `sync_path_params` → `merge_path_variables(&request.uri, &request.variables)`。
     - `merge_path_variables`（`model.rs:797-805`）把所有当前不在 URL 里的旧变量当成“普通接口变量”追加保留在末尾。于是用户在地址栏输入 `:user_id` 的过程中，每敲一个键依次产生 `:u`、`:us`、`:use`、`:user`、`:user_`、`:user_i`、`:user_id`，**前 6 个半成品名字全部作为接口变量永久留在 `request.variables` 里**（切到 `Settings` 标签页或导出 JSON 时会看到一堆 `u`、`us`、`use` 垃圾行）！
  3. **改 URL 会取消 `path_params` 正在等待的 200ms 防抖**：
     - 用户在 `Path parameters` 值输入框打字后（`KvEditor` 有 200ms 防抖，`self.pending` 还没触发 `Change`），立即点进地址栏改了一个字符，`sync_path_params` 调用 `path_params.set_list(merged)`，而 `KvEditor::set_list`（`kv_editor.rs:64`）第一行就是 `self.pending = None`，直接丢掉了刚在路径参数框里输入但还没满 200ms 的值！
  4. **`path_params` 允许改键名和删行，但一改 URL 又复活**：
     - `path_params` 用的是普通 `KvEditor`，用户可以改它的 Key 或点删除按钮删掉一行；但只要地址栏里的 `:id` 还在，下一次 `sync_path_params` 又会自动再生成一行空的 `id`。
- 修复方向：结合 **Q-1** 一起彻底理顺（见下文 Q-1）。
- 修复：按 **Q-1** 移除 `RequestSettingsView` 中重复的接口变量编辑器；新增 `sync_path_variables` 只保留当前 URI 中 `path_param_names` 实际存在的占位符；`KvOptions { fixed_keys: true, .. }` 将路径参数表设为键名只读、固定行、无批量模式；`RequestEditor` 用 `path_drafts` 在内存中暂存编辑期间填过的键值，并在 `sync_path_params` / `save_uri` 时直接读取 `path_params.list(cx)` 保留尚未满 200ms 防抖的输入。

### BUG-7 项目内查找替换存在并发保存竞态、会强制关闭所有独立窗口，且打开前未 `flush` 当前编辑器（F15 / F7，中）——已修复

- 位置：
  - `src/states/workspace.rs:3369-3478`（`find_replace_preview`、`apply_find_replace`、`undo_replace`）
  - `src/states/workspace.rs:1487-1495`（`start_again`）
  - `src/views/workspace.rs:753-759`（`open_find_replace_dialog`）
- 场景与原因：
  1. **`api.save_now` 与 `tree().replace()` 在不同队列并发执行，产生竞态**：
     - `apply_find_replace`（`:3401-3403`）和 `undo_replace`（`:3451-3453`）先遍历 `self.kept` 对所有打开的标签调用 `api.update(cx, |api, cx| api.save_now(cx))`，然后**不等那些异步保存完成**，立即在工作区自己的 `self.gate` 队列里发起 `tree().replace(after, Some(revision))`（`:3410`）。
     - 每个 `ApiState` 的保存走的是该接口自己的 `self.saves` 队列（`states/api.rs:1469`），而 `tree().replace` 走的是工作区的 `self.gate` 队列。如果某个打开的接口恰好有未保存的防抖修改，`api.save_now` 排在后台执行：
       - 若 `save_now` 先落地，数据库 `revision` 加 1，紧接着执行的 `tree().replace(after, Some(旧 revision))` 就会在服务端/存储层因 `Stale` 失败！
       - 若 `tree().replace` 先落地，随后完成的 `save_now` 或 `start_again` 就会把替换前的旧请求重新写回数据库，冲掉查找替换的结果！
  2. **查找替换或撤销替换会直接关掉所有独立窗口（`ApartView`）**：
     - `apply_find_replace`（`:3419-3421`）和 `undo_replace`（`:3465-3467`）对每个受影响的接口调用 `this.start_again(id, cx)`。
     - `start_again`（`:1487-1495`）会从 `self.kept` 和 `self.apart` 中移除该接口，导致所有正在独立窗口中打开的受影响接口（`ApartView` 监听到不再处于 `apart`）**窗口直接被强制关闭**！而在同一个文件里，B7 的来源同步（`:3011-3018`）专门区分了已打开的 `kept` 标签：对已打开的接口调用 `api.replace_request(updated.request)` 原地更新请求，既保留标签页的响应历史，也不会关掉独立窗口。
  3. **打开查找替换弹窗前没有 `flush` 当前编辑器**：
     - `open_find_replace_dialog`（`views/workspace.rs:753-759`）直接打开弹窗，没有像打开运行器或压测弹窗那样先对 `self.request_editor` 调用 `flush(window, cx)`。用户在当前编辑器的输入框里刚改了文字（还在 `KvEditor` 200ms 防抖或地址栏未失焦），打开查找替换看到的预览还是旧文本，执行替换后还会丢掉刚打的字。
- 修复方向：
  - `open_find_replace_dialog` 打开前先 `flush` 主窗口（及独立窗口）的编辑器。
  - `apply_find_replace` 与 `undo_replace` 不要调异步的 `api.save_now(cx)`，而是先同步调用 `api.forget_unsaved()`（因为新树 `after` 已经由 `find_replace_preview` 从包含 `kept` 最新内存请求的 `tree_for_find_replace` 算出），再单走 `self.gate` 执行 `tree().replace`；成功后对 `self.kept` 里已打开的接口调用 `api.replace_request(...)` 原地刷新（保留响应历史与独立窗口），不再调用 `start_again`。
- 修复：`open_find_replace_dialog` 打开前先调用 `self.flush_request(cx)`；`ApiState` 新增 `forget_unsaved` 与 `set_stored_request` 丢弃未发出的防抖保存并原地同步请求，`apply_find_replace` 与 `undo_replace` 先同步丢弃防抖保存、单走 `self.gate` 执行 `tree().replace`，成功后对 `self.kept` 中受影响的接口调用 `set_stored_request` 原地刷新，不再调 `start_again` 关闭独立窗口。

### BUG-8 项目内查找替换对 gRPC 与 GraphQL 请求体做裸字符串替换，可能破坏内部 JSON 或描述符（F15，中）——已修复

- 位置：`crates/cyberapi-core/src/find_replace.rs:180-197`（`preview` 中对 `Scope::Body` 的处理）。
- 场景与原因：
  - 当勾选 `Body` 范围做项目内查找替换时，`find_replace.rs:181-196` 对任何满足 `!after.body.is_empty()` 的接口都直接执行 `replace_text(&after.body, find, replace, case_sensitive)`。
  - 但是：
    1. `HttpMethod::Grpc` 的 `after.body` 存的是 `GrpcBody` 序列化的 JSON（包含 `"service"`、`"method"`、`"message"` 以及 base64 编码的二进制 `.pb` 描述符 `"definitions"`）。裸字符串替换不仅可能改坏外层 JSON 语法，如果查找词是短词（如 `"id"`、`"v1"`、`"api"`）还会把 base64 编码的二进制描述符改坏，导致 `.pb` 定义无法解码！
    2. `ContentType::GraphQl` 的 `after.body` 存的是 `GraphQlBody` 序列化的 JSON（`{"query":"...","variables":"...","operation_name":...}`）。当替换词包含双引号、反斜杠或换行符，或者查找词恰好匹配到外层字段名 `"query"` / `"variables"` 时，直接对原始 JSON 字符串替换会破坏 `GraphQlBody` 的外层 JSON 结构。
    3. `ContentType::Form` / `Multipart` 的 `after.body` 也是 `Vec<KvParam>` 的 JSON 数组，裸替换同样可能匹配到 `"key"` / `"value"` / `"enabled"` 等内部 JSON 键名。
- 修复方向：
  - 在 `find_replace.rs` 处理 `Scope::Body` 时按 `(method, content_type)` 区分：
    - `HttpMethod::Grpc`：解析 `GrpcBody`，只对 `service`、`method`、`message` 做查找替换再序列化回去，绝不碰 `definitions`。
    - `ContentType::GraphQl`：解析 `GraphQlBody`，只对 `query`、`variables`、`operation_name` 字段的内容做查找替换再序列化回去。
    - `ContentType::Form` / `Multipart`：解析 `Vec<KvParam>`，只对每行的 `key`、`value`、`description` 做替换再序列化回去。
- 修复：`find_replace::replace_body` 按 `(method, content_type)` 结构化处理：gRPC 只替换 `GrpcBody` 的 `service`、`method`、`message`（保持 `descriptors` 不变），GraphQL 只替换 `GraphQlBody` 的 `query`、`variables`、`operation_name`，`Form`/`Multipart` 只替换 `Vec<KvParam>` 每项的 `key`、`value`、`description`。

### BUG-9 来源同步的破坏性变更检测（`breaking_changes_of_schema`）有三处误报/重复报（B11，中）——已修复

- 位置：`crates/cyberapi-core/src/schema_docs.rs:49-87`（`breaking_changes_of_schema`）。
- 场景与原因：
  1. **旧接口原本没有 Schema 时，首次加上 Schema 会把所有必填字段误报为破坏性变更**：
     - 一个手建的接口（或旧版导入时还没有 `request_schema` 的接口，`old_schema == ""`），在“从来源同步”时首次拿到上游的 `request_schema`。此时 `old_fields` 为空，`:71-83` 遍历 `new_fields` 时发现所有 `new_field.required` 在 `old_map` 中都不存在，于是把整个请求体的所有必填字段全部标为红色破坏性变更 `AddedRequired`！
  2. **可选父对象下的必填子字段被误报为 `AddedRequired`**：
     - 上游新增了一个**可选**的对象字段 `filter`（`filter.required == false`），而 `filter` 内部定义了当提供 `filter` 时必填的子字段 `filter.keyword`（`required == true`）。旧请求不传 `filter` 依然完全合法，并非破坏性变更；但 `collect_fields` 把 `filter.keyword` 拍平后 `new_field.required` 为 `true`，`:72` 直接报 `AddedRequired("filter.keyword")`。
  3. **删除父对象时对父对象及其所有子孙字段重复刷屏报警**：
     - 当上游删掉一个包含 10 个子属性的嵌套对象 `user` 时，`old_fields` 里既有 `user` 也有 `user.id`、`user.name` 等 10 个子字段，`:57-68` 会报出 11 条 `RemovedField`（`user` 以及它下面的每一个子字段）。
- 修复方向：
  - 若 `old_fields.is_empty()`（旧接口无有效 Schema），直接返回空列表（首次补充 Schema 不算破坏性变更）。
  - 检查 `AddedRequired` 时，只有从根到该字段的**所有祖先节点也都满足 `required == true`**（或祖先节点在 `old_map` 中已存在且路径必达）时才算破坏性新增必填项。
  - 检查 `RemovedField` 时，若某个字段的祖先路径（如 `user`）已经被判定为 `RemovedField`，跳过其子孙字段 `user.*` 的重复报错。
- 修复：`breaking_changes_of_schema` 在 `old_fields.is_empty()` 时直接返回空列表；跳过已被移除父级下的子孙 `RemovedField`；仅在所有祖先节点在旧 Schema 中已存在或同样为 `required` 时才报告顶层可达的 `AddedRequired`。

### BUG-10 说明标签页与请求 Schema 校验的三处细节缺陷（F11 / F13 / F16，中低）——已修复

- 位置：
  - `src/views/workspace/request_editor.rs:354-358`（`RequestReplaced` 分支）、`:947-985`（`validate_body_schema`）
  - `src/views/workspace/docs_editor.rs:46-75`、`:193-253`
- 场景与原因：
  1. **`example_sub` 在 `RequestReplaced` 时未重置/夹紧**：用户在“说明”标签页选中第 3 个响应示例（`self.example_sub = 2`），随后通过历史记录恢复或从来源同步触发 `ApiEvent::RequestReplaced`，新请求的 `examples` 只有 1 个。`render_docs`（`request_editor.rs:1915`）里 `.get(self.example_sub).or_else(|| examples.first())` 虽然不会 panic，但高亮和删除按钮逻辑会错位或跳回首条；应在 `RequestReplaced` 时 `self.example_sub = 0`。
  2. **在说明页修改请求 Schema 后不触发请求体校验刷新**：用户在 `Body` 标签页写好 JSON 请求体，切到“说明”标签页点“编辑请求 Schema”修改了 `request_schema`（或通过“从来源同步”更新了 `request_schema`），再切回 `Body` 标签页时，因为 `body_editor` 的文本没变，`validate_body_schema` 不会重新运行，工具栏上的 Schema 校验警告还是旧的结果。
  3. **说明为空时，只读查看者（`!editable()`）打开“说明”标签页仍进入 Markdown 编辑框**：`DocsEditor::new`（`docs_editor.rs:51`）写的是 `let editing_desc = desc.trim().is_empty();`，没有判断 `workspace.read(cx).editable()`；且 `:199-214` 在 `editing_desc == true` 时即使 `!has_desc && !editable` 也会渲染出“预览 / 编辑”按钮和输入框。只读查看者打开一个没有说明的接口时，看到的是一个可输入但永远不会保存的 Markdown 编辑框。
- 修复方向：
  - `RequestReplaced` 时重置 `self.example_sub = 0` 并调用 `this.validate_body_schema(cx)`；`DocsEditor` 保存请求 Schema 后通知或切回 `Body` 时校验一次。
  - `DocsEditor::new` 中 `let editing_desc = editable && desc.trim().is_empty();`，且只在 `editable` 为真时显示“编辑 / 预览”切换按钮。
- 修复：`DocsEditor` 在 `reload` 时将 `selected_example` 夹紧到新 `examples` 范围，`RequestEditor::reload` 与切换到 `Body` 标签页时重新执行 `check_json(cx)`；`DocsEditor` 在 `!editable` 时始终处于只读预览态，不再进入 Markdown 编辑框或显示无效切换按钮。

### BUG-11 `KvEditor::set_list` 在批量编辑模式下不更新文本框（F14，中低）——已修复

- 位置：`src/views/kv_editor.rs:62-133`（`set_list`）。
- 场景与原因：
  - 用户把 `Query` 或 `Headers` 的 `KvEditor` 切换到批量文本模式（`self.bulk_mode == true`），此时界面显示的是 `self.bulk_editor`。
  - 随后用户从历史记录点选一条旧响应恢复请求（触发 `ApiEvent::RequestReplaced` → `query.set_list(...)`），或执行了项目内查找替换：
  - `set_list` 只更新了 `self.rows` 和 `self.list`，**完全没有更新 `self.bulk_editor`**！不仅屏幕上看到的批量文本没有变，一旦用户接着在 `bulk_editor` 里打字或切回逐行模式（`toggle_bulk` 把 `bulk_editor` 的文本解析回 `rows`），就会把恢复/替换前的内容重新写回去。
- 修复方向：
  - `KvEditor::set_list` 在 `self.bulk_mode` 为真且 `self.bulk_editor` 已存在时，调用 `Self::serialize_bulk(&self.rows, self.options.descriptions)` 同步更新 `bulk_editor` 的文本（设置 `self.syncing = true` 避免触发 `Change`）。
- 修复：`KvEditor::set_list` 在 `self.bulk` 已存在时用 `kv_text(list)` 同步更新批量编辑器的文本与 `descriptions` 并清除 `self.report`，`toggle_bulk` 从批量模式切回表格模式时先将 `self.bulk` 置为 `None`。

### BUG-12 压测在并发 worker 中每次发送都读写 `CookieStore` 且对静态请求重复 `prepare`（A7b，中低）——已修复

- 位置：`crates/cyberapi-service/src/load.rs:95-146`（`run`）、`crates/cyberapi-service/src/send.rs:63-91`（`send_without_history`）。
- 场景与原因：
  - A7b 为了支持 `uuid()` / `random()` / `now()` 等按次动态求值的模板函数，把压测改成了在每个 worker 每次发送时调用 `send_without_history(&sender, &request)`。
  - 这带来三个问题：
    1. **违背文档并污染本机 Cookie**：`FEATURES_zh.md` 明确写明压测“响应设的 Cookie 不保留”，但 `send_without_history` 每次收到响应都会执行 `:83-85` 的 `sender.storage.cookies().put(set_cookies).await`，把压测服务器返回的 `Set-Cookie` 写进项目持久化 Cookie 库！
    2. **50 个并发 worker 争抢桌面端唯一的 Turso 连接锁**：每次发送前 `send_without_history` 调 `sender.storage.cookies().list(project).await`，收到响应后若带 `Set-Cookie` 又调 `cookies().put(...)`。在桌面端 `TursoStore` 只有单连接互斥锁，高并发压测时大量时间花在等 SQLite/Turso 锁查 Cookie 上。
    3. **不含动态模板的静态请求也被迫每次重新 `prepare`**：90% 以上的压测请求并不含 `uuid()` / `random()` / `now()` / `timestamp()`，每次循环都重新解析模板、重新查继承链与 OAuth 令牌纯属浪费。
- 修复方向：
  - 在压测开始前查一次 `cookies().list(project)` 留作只读快照，压测期间**不再每跳查库、也不写回 `cookies().put`**。
  - 开始前检查请求是否包含动态模板函数（或比较两次不同随机种子的 `prepare` 结果是否相同）：若为静态请求，只 `prepare` 一次并在所有 worker 间共享 `PreparedRequest`；只有确实含动态函数时才在内存中按次 `send::prepare`（复用开始前读好的只读 Cookie 快照与 OAuth 令牌）。
- 修复：`load::run` 在开始前通过 `prepare_for_load` 一次性拉取继承上下文、只读 Cookie 快照与 OAuth 令牌构造 `PreparedSender`，压测期间完全不读写 `Storage::cookies()`；通过 `send::has_dynamic_templates` 判断请求是否含动态模板函数，静态请求只 `prepare` 一次并在所有并发 worker 间克隆共享 `PreparedRequest`，动态请求也直接复用 `PreparedSender` 内存快照按次 `prepare_once` 再调 `engine.send`。

### BUG-13 独立窗口（`ApartView`）不响应上下 / 左右布局切换（F7b，低）——已修复

- 位置：`src/views/workspace/apart_window.rs:64-127`（`ApartView::new`）。
- 场景与原因：
  - 主窗口的 `WorkspaceView` 会 `observe` `AppState`，当用户在工具栏切换上下/左右布局（`toggle_compare_layout` → `AppState::set_compare_layout`）时，主窗口立即重排。
  - 而 `ApartView::new`（`:118`）只在打开独立窗口那一刻读了一次 `let compare_layout = store.app().read(cx).compare_layout();` 存进 `self.compare_layout` 字段，没有 `cx.observe(&app, ...)`。无论在主窗口还是在独立窗口自己的请求栏点切换布局按钮，已打开的独立窗口都不会重排。
- 修复方向：
  - 在 `ApartView::new` 中 `cx.observe(&app, |this, app, cx| { let next = app.read(cx).compare_layout(); if this.compare_layout != next { this.compare_layout = next; cx.notify(); } })`（或者在 `render` 中直接读 `AppStore::global(cx).app().read(cx).compare_layout()`）。
- 修复：在 `ApartView::new` 中通过 `cx.observe(&app, ...)` 监听 `AppState::compare_layout()` 变化并通知重绘。

---

## 性能问题详情

### PERF-1 bridge 每次 MCP 令牌请求白查一次会话表，且 `/v1/mock` 每次请求整棵读树并重建路由（G2b / G3 / H1，高）——已修复

- 位置：
  - `cyberapi-bridge/src/auth.rs:115-137`（`session_token`）、`:166-183`（`signed_in_or_token`）
  - `cyberapi-bridge/src/mock.rs:63-77`（`handle`）
- 情况：
  1. **`signed_in_or_token` 对每个 `Bearer cat_...` 令牌都先跑一次必败的 `session_account` SQL 查询**：
     - `signed_in_or_token` 先调 `signed_in(store, headers)`，失败才调 `token_account(store, headers, query)`。
     - 而 `session_token(headers)`（`auth.rs:124-136`）只要看到 `Authorization: Bearer <任何非空字符串>` 就把它当成桌面登录的会话令牌返回！当外部前端或 AI 客户端带 `Authorization: Bearer cat_xxxx` 高频调用 `/v1/mock`、`/v1/docs` 或 `/v1/mcp` 时，`signed_in` 先算一次 SHA-256 去 `sessions` 表跑一条必不命中的 SQL 查询，返回 `UNAUTHORIZED` 后才在 `token_account` 里再去 `mcp_tokens` 表查一次——每次请求白白多花一倍的认证 SQL 往返。
  2. **`/v1/mock` 每次请求都执行 `storage.tree().load(project)` 并重建 `Routes::of(&tree)`**：
     - `storage.tree().load` 会从 PostgreSQL / MySQL 读出该项目的所有目录、所有接口（含完整的请求 JSON、最多 256 KB × N 的响应示例）并反序列化，然后 `Routes::of(&tree)` 重新解析所有 URI 建立路由表。
     - 前端页面联调时往往一页并发几十个请求打向 `/v1/mock/<project>/...`，若项目有几百个接口，每次 Mock 请求都全量拉库 + 反序列化整棵树，数据库和 bridge CPU 立刻成为瓶颈。
- 修复方向：
  - `session_token` 中遇到 `token.starts_with(MCP_TOKEN_PREFIX)`（`"cat_"`）时直接返回 `None`，让 `cat_` 令牌一步直达 `token_account`。
  - 在 `Bridge` 内部按 `(CollectionId, revision)` 做一个轻量 LRU / 容量有界的 `Arc<mock::Routes>` 缓存：每次 `/v1/mock` 请求先用轻量查询验证可见性并取当前 `revision`（或复用带 `revision` 的缓存项），只要项目 `revision` 没变就直接复用已构建好的 `Arc<Routes>`，毫秒级直接返回示例。
- 修复：`auth::session_token` 跳过 `cat_`（`MCP_TOKEN_PREFIX`）开头的 Bearer 令牌；`Bridge` 新增 `mock::RouteCache`（最多缓存 64 个项目），每次 `/v1/mock` 先用轻量 `tree().revision(project)` 校验权限并取修订号，命中同 `revision` 时直接复用缓存的 `Arc<mock::Routes>`。

### PERF-2 MCP `get_history` 拉取最多 100 条完整响应体而非摘要（H1 / H2，中）——已修复

- 位置：`crates/cyberapi-service/src/mcp.rs:651-661`（`get_history`）。
- 情况：
  - PERF-9 已经为图形界面在 `HistoryStore` 上加了不带响应体的 `summaries(project, api)` 和按 ID 取单条的 `get(project, api, id)`。
  - 但 MCP 的 `get_history` 仍然调用 `storage.history().list(project, &api).await`，一次把该接口最多 100 条历史记录连同每条最多 2 MB 的响应体全部从数据库读出并反序列化，接着在 `:657-661` 把第 2～100 条的响应体全部丢掉，只留第 1 条的响应体（截到 16 KB）！
- 修复方向：
  - `mcp::get_history` 改为先调 `storage.history().summaries(project.clone(), &api).await`，若列表非空再对 `entries[0].id` 调一次 `storage.history().get(project, &api, &entries[0].id).await` 取出最新一条的响应体。
- 修复：`mcp::get_history` 改为调用 `storage.history().summaries(project.clone(), &api).await`，仅当列表非空时再对第一条调用 `storage.history().get` 读取完整响应体。

### PERF-3 变量弹窗在默认单层视图下也无条件构建与同步 `E × N` 个矩阵输入框（D8，中）——已修复

- 位置：`src/views/variables_dialog.rs:452-457`（`sync_rows`）。
- 情况：
  - `sync_rows` 在 `VariablesState` 每次发出事件（包括用户在单层视图每敲一个键、切换环境、新增/删除变量）时都会调用，而它第 456 行无条件调用 `self.sync_matrix_cells(window, cx)`。
  - 当用户从未点开“矩阵对比”（`self.matrix == false`）时，一个有 6 个环境、100 个变量的项目仍然会在后台创建并维护 `7 × 100 = 700` 个 GPUI `InputState` 实体及 700 个 `subscribe_in` 订阅，并在每次通知时双重循环遍历它们。
- 修复方向：
  - `sync_rows` 中只在 `self.matrix` 为 `true` 时才调用 `self.sync_matrix_cells(window, cx)`；从单层切换到矩阵视图（`:323`）时立即调用一次 `self.sync_matrix_cells(window, cx)`。
- 修复：`sync_rows` 仅在 `self.matrix` 为真时调用 `sync_matrix_cells`，并在切换到矩阵视图时调用 `sync_matrix_cells`。

### PERF-4 `source_updates` 在 UI 线程的 `render` 与菜单打开时同步克隆整棵树并跑合并预览（B10，中）——已修复

- 位置：
  - `src/states/workspace.rs:2893-2905`（`source_updates`）
  - 调用处：`src/views/workspace/source_dialog.rs:248`（`SourceView::render`）、`src/views/workspace/tree_panel.rs:644`、`1091`
- 情况：
  - B10 在后台拉取到远程 OpenAPI / 集合规范后存入 `WorkspaceState.polled_sources`。
  - 此后每当 `SourceView::render` 执行（弹窗打开且输入框光标每 500ms 闪烁一次就是每秒 2 帧）或用户在树面板点开项目/目录菜单时，都会在 **UI 主线程**同步调用 `workspace.source_updates(folder, cx)` → `merge::preview` / `merge::preview_folder`。
  - `merge::preview` 会克隆整棵 `TreeData`、运行完整的集合合并，并且 B11 加入破坏性变更检测后还会对每个更新接口解析新旧请求/响应 JSON Schema 并递归对比字段树！在大项目或大型 OpenAPI 规范下会导致明显的界面卡顿。
- 修复方向：
  - 在后台拉取完成（`poll_watched_sources` 收到 `bundle`）或本地树发生变化时，只计算一次 `(added, updated, breaking_count)` 缓存进 `PolledSource`（或者缓存按 `tree_builds` 计数失效的摘要结果），`source_updates` 直接 O(1) 读取缓存的数字。
- 修复：`WorkspaceState` 新增 `source_updates_cache: RefCell<HashMap<Option<ItemId>, Option<(usize, usize, usize)>>>`；当本地树或拉取的规范发生变更时调用 `invalidate_source_updates_cache` 清空缓存，`source_updates` 命中缓存时直接 O(1) 返回。

---

## 功能取舍与设计问题（Q-1 ～ Q-2）

### Q-1 是否删掉请求“设置”标签页里的“接口变量”编辑器，把 `HttpRequest.variables` 收敛为纯路径参数？（F12 / D5）——已按建议处理

- **现状与问题**：
  - D5 引入了目录级变量（`FolderSettings.variables`）和接口级变量（`HttpRequest.variables`），当时在接口编辑器的 `Settings` 标签页最下方放了一个 `Variables` 键值表格（`src/views/workspace/request_settings.rs:185-194`）。
  - F12 引入路径参数（`:id` / `{id}`）可视化编辑时，为了不改动数据模型，复用了 `HttpRequest.variables` 来存路径参数的值和说明，并在 `Query` 标签页上方放了一个 `Path parameters` 键值表格（`src/views/workspace/request_editor.rs:1833-1848`）。
  - 现在**同一个 `HttpRequest.variables` 同时在两个标签页有两个独立的 `KvEditor`**：
    1. 引发了 **BUG-6** 的双向状态覆盖、地址栏打字生成 `:u` / `:us` / `:use` 幽灵变量、打断防抖等一连串 bug。
    2. 从用户视角看，**单个接口内部根本不需要除路径参数以外的“私有自定义变量”**：同一个接口里如果在多处（如 Header 和 Body）用同一个变量，放到目录变量或项目环境变量里本来就更自然；而在 Postman / Insomnia / Apifox / Hoppscotch 等主流工具里，请求级变量（Request Variables）指的就是 **Path Variables（路径参数）**，并不存在另一套藏在请求 Settings 里的请求级键值表。
- **建议（删除冗余入口、简化交互）**：
  1. **删除 `RequestSettingsView`（接口“设置”标签页）里的“接口变量”编辑器**（保留目录设置弹窗里的“目录变量”，目录变量供目录下多个接口共享很有用）。
  2. **将 `HttpRequest.variables` 明确定位为该接口的路径参数表**（兼容已有数据和 Postman `url.variable` / OpenAPI `in: path` 导入导出）：
     - 在 `Query` 标签页的 `Path parameters` 分区展示，其行集合严格由当前 `request.uri` 中解析出的 `path_param_names(&request.uri)` 决定。
     - **键名（Key）设为只读**、隐藏删除按钮和末尾空行（因为一个参数是不是路径参数完全取决于 URL 里有没有写 `:id` 或 `{id}`，不该在表格里手改键名或删行）。
     - 当用户在地址栏改 URL 时（如把 `:u` 改成 `:user_id`），在 `RequestEditor` 内存中用一个小映射记住该接口编辑期间填过的 `参数名 -> (value, description)`，但 `request.variables` 只保存当前 URL 里实际存在的占位符名称——彻底消除 `:u`、`:us`、`:use` 幽灵变量！
     - 当 `Path parameters` 的值或说明输入框正在聚焦编辑时，地址栏没变就不调 `path_params.set_list`，避免冲掉 200ms 防抖。
- **处理**：已按建议删除接口“设置”标签页中的“接口变量”编辑器，将 `HttpRequest.variables` 收敛为 `Query` 标签页下键名只读、紧随 URL `:param` / `{param}` 占位符的“路径参数”表格，并同步更新中英文界面文案与 `FEATURES.md` / `FEATURES_zh.md`。

### Q-2 桌面版本机 MCP Server 对服务器项目副本（`ServerCopy`）的权限与同步边界（H2）——已按建议处理

- **现状与问题**：
  - 如 **BUG-3** 所述，桌面版本机 MCP Server 直接把 `TursoStore` 传给 `cyberapi_service::mcp::handle`，既会列出已退登账号的隐藏服务器项目副本，也会允许修改只读（Viewer）服务器副本，而且修改服务器副本后既不更新 `ServerCopyStore` 的 `unsynced = true` 标记、也不通知正在打开的 `WorkspaceState`，导致修改在下次轮询或关窗时静默丢失。
- **两个可选方案**：
  - **方案 A（推荐，最简单、零同步隐患）**：桌面版本机 MCP **只隐藏 `ServerState::hides` 的非当前账号副本**，并且**对带有 `ServerCopyStore` 绑定的服务器项目副本限制为只读（允许 `list_projects`、`get_project`、`search_apis`、`get_api`、`get_variables`、`get_environments`、`get_history`、`send_api`，拒绝 `create_api` / `update_api` 并提示“这是服务器同步项目，请在界面中修改或使用 bridge 的 MCP 端点”）**。
    - 理由：本机 MCP 的设计初衷（见 `issue-plan.md` H2：“让未部署 `bridge` 的个人桌面用户也能在 Claude Code / Cursor 中直接读写和调测本机项目”）本来就是服务纯本机项目的；服务器项目本身在 bridge 上就有原生的 `/v1/mcp` 端点。在桌面端禁止本机 MCP 直接绕过 `ServerCopy` 改写服务器副本，能一次性消除越权修改、漏标 `unsynced`、绕过 `LocalItems` 指纹以及与打开中工作区冲突的所有问题。
  - **方案 B（支持本机 MCP 改写服务器副本）**：在桌面版为本机 MCP 包一层感知 `ServerState` 与 `WorkspaceState` 的 `Storage` 适配器：按 `ServerState::hides` 过滤项目；按服务器绑定的角色（需在绑定中记录 role）拦截 Viewer 修改；每次 `create_api` / `update_api` 成功后自动读取并更新 `ServerCopyStore` 绑定为 `unsynced = true`，并通过主线程通知打开中的 `WorkspaceState` 重新加载树和刷新 `kept` 中的 `ApiState`。
- **建议**：采用 **方案 A**（过滤 `ServerState::hides`，对纯本机项目读写全开、并在纯本机项目正在窗口中打开时通知 `WorkspaceState` 刷新；对服务器项目副本允许读取和发送、拒绝 `create_api` / `update_api`）。
- **处理**：已按方案 A 实现，并更新 `FEATURES.md` / `FEATURES_zh.md` 的本机 MCP 说明。

