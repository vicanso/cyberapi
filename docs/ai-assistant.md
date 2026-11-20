# H3：内置 AI 助手（设计稿）

> 状态：设计稿（2026-11-18）。第 9 节的四项决策维护者已于当天定下（团队共用模型配在 bridge 上、第四栏、每次发送都确认、OpenAI + Anthropic），正文已按此改写。来源：[issue-plan.md](./issue-plan.md) 的“借鉴 apiflow 与 Yaak”，参照 apiflow 的内置 Agent（变更集需要批准）。此前 H3 被否决的理由是“外部 AI 已经能通过 MCP 用 CyberAPI”；重新打开是因为：多数读者不会去配 MCP 客户端，而“用一句话 / 一段文档生成请求”“看懂这个响应”“为它写断言”都发生在工作区里，离开应用去问外部 AI 要来回复制。

## 1. 目标与边界

做：

- 工作区里的**对话面板**：带着当前项目、当前接口、当前响应作上下文问答。
- **模型由 bridge 管理员统一配置**（团队共用）：读者不需要自己申请和保管密钥。系统管理员在页面的“AI 模型…”里设（存在数据库的 `bridge_settings`，密钥不回传页面），或用启动选项给出，页面上设的优先。
- **用文字生成请求**：粘贴一段文档、curl 以外的描述或一句话，生成一个临时请求（F1 的临时请求，不直接进项目）。
- **为当前响应生成断言**：给出断言列表，读者勾选后加入。
- AI 要新建 / 修改接口时先成为**变更集**，读者看过差异、批准后才写入。

不做：

- AI 自动写入或自动发送（每次写入、每次发送都要读者点一次）。
- 删除接口、目录、变量（与 MCP 的 `create_api` / `update_api` 一样，不提供删除）。
- 把秘密变量的值、Cookie、OAuth 令牌、客户端证书发给模型。
- 保存对话记录（关掉面板或离开项目即丢弃）。
- 读者各自配置模型（以后若有需要再加）；没有登录服务器的桌面版没有 AI。
- 插件、远程工具、联网搜索。

## 2. 读者看到什么

- 标题栏（项目名右侧）多一个“AI”按钮，打开 / 关闭工作区最右侧的**助手栏**（第四栏，宽度可拖动，存进 `AppPrefs.columns`；窗口窄时盖在响应栏上）。
- 助手栏：消息列表（Markdown，代码块可复制）、输入框（⌘/Ctrl-Enter 发送）、“停止”、“清空”。上方一行写当前上下文：项目 / 接口 / 是否带上当前响应（可取消勾选）。
- 回答里出现的**变更集**是一张卡片：每个要新建 / 修改的接口一行，展开看请求的逐行差异（复用 `versions::request_text` + `diff::compare`，与历史版本对比同一套显示），按钮“全部应用”“逐个应用”“放弃”。查看者看得到卡片，但应用按钮不可用。
- 回答要**发送**某个接口时也是一张卡片（方法、地址、环境），点“发送”才发；生产环境照 D6 再确认一次。发送结果回到对话里（状态码、耗时、截断的响应体、断言结果），也照常记入该接口的历史。
- 请求编辑器的工具栏菜单多两项：“用文字生成请求…”“为响应生成断言”（后者在响应栏，有响应时可用）。两者都在助手栏里进行，不另开弹窗（网页版弹窗不能叠）。
- 服务器没有配置模型时不显示“AI”按钮；桌面版只在登录了配置了模型的服务器时显示。

## 3. 分层

| 层 | 放什么 |
|---|---|
| L0 `cyberapi_core::ai`（新） | 与协议无关的对话类型（消息、工具定义、工具调用、变更集）和 bridge 发给页面的事件（`AiEvent`：文本增量、工具调用、结束、错误）；把对话组装成两种协议的请求体（OpenAI Chat Completions、Anthropic Messages）；把两种协议的流式回答（复用 `sse::Parser`）读成 `AiEvent`；上下文裁剪（复用 `mcp::cut` / `MAX_RESULT_BYTES`）。纯函数，能编译到 wasm。 |
| L2 `cyberapi_service::assistant`（新） | 一轮对话：经 `AiModel`（页面上是 `BridgeStore` 旁的一个调用，见第 4 节）流式得到 `AiEvent` → 遇到读工具就执行、把结果接回去再问，最多 `MAX_STEPS`（8）步；提议工具和发送不执行，只产出变更集 / 发送卡片交给上层。读工具复用 `cyberapi_service::mcp` 的 `list_apis` / `get_api` / `get_variables` / `get_history`，打码规则不变。 |
| L3 `states/assistant.rs`（新） | `AssistantState`：由 `WorkspaceState` 持有，项目关闭即丢弃；对话、进行中的请求（drop 即取消）、待批准的变更集；批准时调用工作区已有的写入路径。 |
| L4 `views/workspace/assistant_panel.rs`（新） | 助手栏；变更集卡片、发送卡片。 |
| bridge | `--ai-url` / `--ai-model` / `--ai-key` / `--ai-protocol`（`CYBERAPI_BRIDGE_AI_*`），三项齐了才开启；`GET /v1/ai`（是否开启、模型名）和 `POST /v1/ai/chat`（流式，NDJSON 的 `AiEvent`）。 |

依赖方向不变：core / service 不依赖 gpui，两端共用；没有任何 `cfg(target_family = "wasm")` 分支进入 view。

## 4. 调用模型：bridge 上的团队模型

- **配置**：`--ai-url`（如 `https://api.openai.com/v1`、团队自己的模型网关）、`--ai-model`、`--ai-key`、`--ai-protocol`（`openai`，默认；或 `anthropic`），都可用环境变量 `CYBERAPI_BRIDGE_AI_URL` / `_MODEL` / `_KEY` / `_PROTOCOL` 给出；地址、模型、密钥三项齐了才开启。**密钥只用环境变量给**（k8s 里放 Secret，同 `CYBERAPI_BRIDGE_STORE`），启动日志只写地址和模型。按 AGENTS.md：改 bridge 选项时 `docs/DEPLOY.md` / `DEPLOY_zh.md` 和 Helm chart 的 values、模板一起改。
- **`GET /v1/ai`**：登录后可读，回答 `{ "model": "…" }` 或 `null`（没开启）；页面据此决定是否显示“AI”按钮。
- **`POST /v1/ai/chat`**：登录会话或桌面的 bearer 会话可用，MCP 令牌不可用（401）。请求体是与协议无关的对话（`core::ai` 的类型）；bridge 用 `core::ai` 组装成所配协议的请求、加上密钥、流式发出，再把模型的 SSE 读成 `AiEvent`，以 NDJSON 一行一个发回页面（同 `/v1/send/stream`：带 `X-Accel-Buffering: no`，页面断开即丢弃任务、断开上游）。
- **地址检查**：模型地址是管理员配置的，bridge 调用它时不经过读者请求用的 `TargetPolicy`，所以公司内网的模型网关不需要 `--allow-target`。读者无法改这个地址。
- **限流**：每个账号每分钟 `CHATS_PER_MINUTE`（20）轮；进行中的对话请求和发送请求一起算在每个账号同时发送的数量（`in_flight`）里，超过时回答一条错误事件；每轮最多 `max_tokens`（4096）输出。每次调用一行日志（账号、模型、用时、模型报告的 token 数），不记内容。
- **页面一侧**：`BridgeStore` 旁加 `BridgeAi`（`Transport::post_stream` 读 NDJSON，已有），实现 L2 的 `AiModel` trait；桌面登录服务器后用同一个实现经 `EngineTransport`（若它还没有 `post_stream`，H3b 补上）。没有登录的桌面没有 `AiModel`，不显示按钮。
- 模型调用不是项目的接口：不写历史、不带 Cookie、不做模板求值。

## 5. 密钥与数据

- 模型密钥在 bridge 进程的环境变量里，或（页面上设的）在数据库的 `bridge_settings` 里；不进页面、不进日志。`/v1/ai/settings` 只回 `hasKey`。
- 发给模型的内容见第 6 节，全部先经 MCP 的打码规则；秘密变量只给名字。管理员在 DEPLOY 文档里看到这一点：对话内容会发往所配置的模型服务。

## 6. 给模型什么

- 系统提示：CyberAPI 是什么、能用哪些工具、写入只能提议、不要编造接口；中英文按界面语言。
- 上下文（每轮重新取，按预算裁剪）：项目名和接口列表（`mcp::listing`，读 `WorkspaceState::tree_data_with_open`，即包含编辑器里未保存的修改和网页的未同步草稿）；当前接口的请求（`mcp` 的打码规则）；勾选时附当前响应（状态、头、体按 `MAX_TEXT_CHARS` 截断、JSON 先打码）；当前环境的变量名和非秘密值（秘密只给名字，同 MCP）。
- 读工具（模型自己决定调用）：`list_apis`、`get_api`、`get_variables`、`get_history`，参数与 MCP 相同，**读的是工作区自己的存储**（网页的服务器项目即页面里的工作副本）。
- 提议工具（不执行，产出卡片）：`propose_create_api`、`propose_update_api`（参数与 MCP 的 `create_api` / `update_api` 相同，经 `mcp::written` 算出新请求）、`propose_assertions`（断言列表）、`propose_send`（接口 + 可选环境）。

## 7. 变更集

- 一张变更集 = 若干条“新建接口（目录、名称、请求）”或“修改接口（id、新请求）”，在 `AssistantState` 里，带生成它时项目的修订号。
- 应用：新建走 `WorkspaceState::create_api` 再 `save_request`；修改走打开着的 `ApiState`（若打开）或 `save_request`，与读者手改同一路径，所以服务器项目随后照常显示“未同步”、仅本地条目照常处理。
- 冲突：应用前比较该接口现在的请求和生成卡片时看到的；不同则这一条标为“已被修改”，要读者重新生成或仍然覆盖。
- 查看者（`editable()` 为假）不能应用；网页的查看者照旧只能问。
- 应用后可以撤销：整张变更集应用前留下受影响接口的原样，通知里给“撤销”（同 F6 的 10 秒）。

## 8. 测试与验证

- `core::ai`：两种协议的请求体（快照）、流式解析（各存一份真实格式的 SSE 样本：纯文本、分段的工具调用参数、多个工具调用、中途断开、错误事件）、上下文裁剪的预算。
- `service::assistant`：用脚本化的假 `HttpEngine`（按顺序回放 SSE）测一轮对话：读工具被执行并接回、提议工具只产出卡片、`MAX_STEPS` 截断、取消。
- bridge：`/v1/ai` 与 `/v1/ai/chat` 的集成测试（`tests/bridge.rs`，本机起一个假模型服务回放两种协议的 SSE）：未登录 401、MCP 令牌 401、未开启时 `null` / 404、流式事件顺序、断开即停止、限流。
- 网页实测（主要验收）：本地起一个兼容 OpenAI 的假模型服务（脚本回放固定的流式回答，含一次工具调用和一次提议），bridge 的 `--ai-url` 指向它，有头 Chromium 走一遍：配置 → 提问 → 读工具 → 变更集卡片 → 应用 → 同步 → 撤销；浅色、深色截图。再用一个真实的兼容接口手动问一次；部署时用页面上的“AI 模型…”或 `CYBERAPI_BRIDGE_AI_*` 给出模型（密钥只用环境变量或页面设置）。桌面只跑 `make lint`，按维护者要求不做界面实测。

## 9. 已定的决策（2026-11-18）

1. **模型从哪来**：bridge 管理员配置团队共用的模型（第 4 节）。读者不各自配置；以后需要时再加个人配置。
2. **助手栏的位置**：工作区第四栏，可开关、可拖宽；窄窗口时盖住响应栏。
3. **发送**：每次都要点卡片上的“发送”；生产环境照 D6 再确认。
4. **协议**：OpenAI Chat Completions 兼容接口和 Anthropic Messages；其他经兼容网关。

## 10. 分步（一项一个提交）

| 编号 | 内容 | 验收 |
|---|---|---|
| H3a | 本文档，维护者确认第 9 节 | — |
| H3b | `core::ai`（类型、两种协议的请求与流式解析）；bridge 的 `--ai-*` 选项、`/v1/ai`、`/v1/ai/chat`、限流；DEPLOY 中英文与 Helm chart；页面的 `BridgeAi`；`service::assistant` 只做纯对话（无工具） | 单测与 bridge 集成测试；网页对假模型能流式对话、能停止 |
| H3c | 上下文与读工具；助手栏 | 假模型的一次工具调用在网页上走通 |
| H3d | 提议工具、变更集卡片、应用 / 冲突 / 撤销；发送卡片 | 网页实测应用后同步、撤销；查看者不能应用 |
| H3e | “用文字生成请求”“为响应生成断言”两个入口；FEATURES 中英文 | 网页实测两条入口；浅色、深色截图 |
