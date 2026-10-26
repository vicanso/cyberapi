# Issue #55 起的处理计划

> 状态：A、B 两组和 C 组第 1–3 批已完成（2026-10-25），第 4 批也已完成（2026-10-25）。还搁置的是 #63 的搜索按钮（等组件库修好）和 #59 的从 URL 导入，以及第 4 批可选的环境整理工具。每个 issue 单独提交，提交信息带 issue 编号。

2026-10-25 把 [vicanso/cyberapi](https://github.com/vicanso/cyberapi/issues) #55 起的 issue 分了组：A 是 bug，B 是改动小、马上能做的优化，C 是要单独规划的功能。

## 已完成

| Issue | 改动 | 提交 |
|---|---|---|
| #55 | URL 框里打的 cURL 命令回车发送时，发往命令自己的地址 | f56f6a2 |
| #56 | “服务器反射”“获取 Schema”用正在输入的地址，地址为空时提示先填 | 162379f |
| #57 | 树形视图用完整的响应体（原来用的是文本视图截下的前 1 MB）；超过 `TREE_LIMIT`（16 MB）时说太大，不是 JSON 时换成响应的说法 | 66fc301 |
| #62 | DELETE 有 Body 标签并发送 body，body 为空时不发 body 和 Content-Type；cURL 命令的 body 方法不发送时保留并提示 | 8013268 |
| #72 | 事件流有 id 时多一列 ID；选中的事件显示 id 和 retry | 2c1da8c |
| #73 | gRPC 地址栏提示 `host:port`；gRPC 响应没有 Cookies 标签，Headers 叫元数据 | b486805 |
| #68 | 历史记录每条显示方法和路径；菜单说明选中会恢复请求（恢复本来就有，F7.10） | 46b885b |
| #64 | ⌘/Ctrl+F 搜索接口、⌘/Ctrl+L 跳到地址栏；树里 Delete（Mac ⌘⌫）删除、⌘/Ctrl+D 复制 | 65dd6dd |
| #67 | 模板函数 `uuid`、`random`、`randomString`、`now`、`hmacSha256`、`urlEncode` | d1b3849 |
| #65 | 首页（含空状态）“导入…”：导入的内容新建为项目，名称取集合自己的名字，项目打开、树加载完后导入 | aeed208 |
| #59 | YAML 写的 Swagger / OpenAPI 照 JSON 导入；剪贴板里的这种 YAML 也会预填（从 URL 导入推后） | 9192d62 |
| #63 | JSON 响应在 Pretty / Tree 视图用 JSONPath 过滤，开关在响应状态行，条件按接口记住 | 45415d1 |
| #58 | Details 显示实际发出的请求：每个请求头及其来源（含引擎加的），请求体，密钥打码可切换 | 13fc381 |
| #69 | 复制为 cURL、HTTPie、JavaScript fetch、Python requests、Go net/http（响应的 `>_` 菜单和接口树子菜单） | 98a24a1 |
| #70 | GraphQL：“格式化”也缩进查询（保留注释）；响应的错误位置标在查询里；浏览 Schema，点字段写进查询 | 7ba0e05、4c148ad、38e7c25 |
| #60 | 桌面版的代理：环境变量（默认）/ 在设置里填写（HTTP、SOCKS5，可带账号和绕过列表）/ 不使用；本机地址始终直连；bridge 始终直连 | 03da11a |
| #71 | 接口的“设置”标签：超时、HTTP 版本、跟随重定向、校验证书，各项默认跟随全局设置；没跟随的 302 提示里可直接打开它；数据模型编号加到 2 | 5916e78 |
| #66a | 接口的认证：继承 / 无 / Basic / Bearer / API key（请求头或查询参数），值可用模板函数，密钥打码；旧的 Basic 列表读成 Basic、不再写回；数据模型编号加到 3（目录 / 项目的认证与继承是 #66b） | 7235d85 |
| #66b | 目录设置（认证、请求头）和项目认证，接口选“继承”时按树从近到远取，认证标签说明来自哪里；各存储、同步、备份、导出、MCP 都带上；数据模型编号加到 4 | 420b28d、cce2ba0 |
| #61a | 命名环境的存储与规则：环境属于项目，变量可以放进某个环境，发送时当前环境的同名变量优先、其余用共用的；当前环境属于读者；各存储、同步、备份、MCP（`send_api` 可指定环境）都带上；数据模型编号加到 5（界面是 #61b） | c82de97 |
| #61b | 标题栏切换环境；变量弹窗分层显示（共用 / 各环境），就地新建、重命名、删除环境，列出被覆盖的共用变量；地址栏手写的 `{{NAME}}` 前缀移到选择器 | c330561 |

## 搁置：#63 的可见搜索按钮

gpui-kit 0.8.0 编辑器自带的查找栏在输入时会全选已输入的内容（打 “chatcmpl” 得到 “hatcmpl”），原因见组件库的 `InputBaseState::set_search_query` 和 `input/overlay.rs` 的 `SearchSignature`。加一个按钮只会把人引向这个问题。gpui-kit 升级后先在响应体的查找栏里打一个多字母的词，正常了再加按钮：聚焦响应体编辑器后 `window.dispatch_action(Box::new(Search), cx)`，提示用 `tooltip_with_action(.., &Search, Some("Input"))`。#63 的另一半（JSONPath 过滤）在第 2 批。

## 第 1 批：小而独立

### #67 更多模板函数（已完成）

加 `uuid()`、`random(min, max)`、`randomString(n)`、`now(format)`（ISO 8601 或自定义格式，UTC / 本地）、`hmacSha256(key, data)`、`urlEncode(text)`。

- 落点：`cyberapi_core::template`（求值）、`template_hints`（提示和选中后写入的内容）、两个 locale 的说明。
- core 没有时钟和随机源：随机数和本地时区偏移像 `Inputs.now` 一样由调用方准备好传进来（`Inputs` 加字段），测试里给固定值。
- `urlEncode` 复用 `send` 里的 `encode_uri_component`；HMAC 用纯 Rust 的 `hmac` crate（能编译到 wasm）。
- #48 之后未知函数会报错，新函数名不能和旧版写法冲突。

### #65 首页直接导入成新项目（已完成）

首页（含空状态）在“新建项目”旁加“导入…”，用导入内容新建一个项目，名称取集合自己的名字（Postman `info.name`、OpenAPI `info.title`、Insomnia 工作区名），取不到时让读者填。

- `ImportBundle` 加一个可选的名称，各格式的解析器填它。
- 新建项目走现有的新建流程：登录且选了团队时先在服务器创建再拉副本，然后按 `assign_ids` 写入。
- 导入弹窗预填剪贴板的规则不变：只在读者点开的那次读。

### #59 导入 YAML 的 OpenAPI（YAML 已完成，从 URL 导入推后）

`import::detect` 先试 JSON（错误信息更准），不是 JSON 再用 `serde-saphyr` 按 YAML 读成 `serde_json::Value`，后面的格式识别不变。

- 从 URL 导入推后：经 `HttpEngine` 取（桌面直连，网页经 bridge，受 `TargetPolicy` 限制），导入弹窗里文本是一个地址时提供“获取”。

## 第 2 批：看清发出去和收回来的

### #63 JSONPath 过滤响应（已完成）

响应体 Pretty / Tree 视图下方加过滤框（RFC 9535 JSONPath，用纯 Rust 的实现），只显示匹配的部分，结果是匹配值的数组；过滤条件按接口记住，属于读者自己（和 prefs 一样不进同步、不进备份）。

- 在完整响应体上过滤（同 #57，受 `TREE_LIMIT` 限制），解析放到后台，不卡界面。
- 过滤后的复制、保存是否用过滤结果，做之前定。

### #58 显示实际发出的请求（已完成）

Details 里加“请求”一节：最终的方法和 URL、实际发出的每个请求头（标出来源：全局请求头 / Cookie / 认证 / 默认）、模板求值后的 body。

- `HttpResponse` 加一个记录发出请求的字段，由 `cyberapi_service::send` 按 `PreparedRequest` 填（跟随重定向时记最后一跳）；旧的历史条目没有这个字段，按 `serde(default)` 读。
- 引擎自己加的头（`Accept-Encoding`，见 `cyberapi-http` 的 `native.rs`）也要算进去，否则和实际不符。
- `Authorization`、`Cookie`、API key 默认打码，点一下显示；body 的保存上限同 `HISTORY_BODY_LIMIT`。
- 网页版经 bridge 发送，`PreparedRequest` 在页面里就有，两端一样。

### #69 复制为代码（已完成）

`>_` 按钮和树的菜单从“复制 cURL”改为“复制为”菜单：cURL（默认）、HTTPie、JavaScript `fetch`、Python `requests`、Go `net/http`。

- 生成器放在 core，和 `curl::curl` 一样从 `PreparedRequest` 生成；multipart 和文件在各语言里的写法分别处理。
- gRPC 仍只有 grpcurl。

## 第 3 批：GraphQL 和网络

### #70 GraphQL：格式化、浏览 Schema、错误位置（已完成）

- 格式化查询：能完整解析时才格式化（写到一半的查询不动），缩进同 `format::JSON_INDENT`。
- 浏览 Schema：点“Schema：N 个类型”打开面板，列类型、字段、参数和说明（`graphql_schema` 已经取了 `description`），点击插入到光标处。数据来自 `WorkspaceState` 按 URI 缓存的 Schema。
- 错误位置：`graphql::response_errors` 已读出错误，再读 `locations` 的行列，在查询编辑器里标出来，参考请求体 JSON 错误标位置的做法。

### #60 代理（已完成）

设置里加代理：跟随系统 / 环境变量（`HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY`、`NO_PROXY`，默认）、手动（HTTP CONNECT、SOCKS5，可带认证和绕过列表）、不用代理。

- `NativeEngine` 每个请求自己建连接：HTTPS 走 CONNECT 隧道，HTTP 用绝对形式的请求行，SOCKS5 握手；DNS / TCP / TLS 分段计时经代理时的含义要重新定义并在 Details 说明。
- 只对桌面版有意义。bridge 要不要走出站代理是部署选项（命令行 / 环境变量），不是读者的设置，网页版设置里不显示。
- 客户端证书（mTLS）以后再说。

## 第 4 批：数据模型（#71、#66、#61）

三个都改请求或变量的数据模型，先一起设计，单独写一份设计说明再动手：见 [batch4-design.md](./batch4-design.md)，第 7 节的问题都按建议定了。

- 第一步（兼容准备，设计说明 §2.1）已完成：c134894；第二步 #71 已完成：5916e78；第三步 #66a 已完成：7235d85；第四步 #66b 已完成：420b28d、cce2ba0；第五步 #61a 已完成：c82de97；第六步 #61b 已完成：c330561。网页版还没有实际在用，此后不再为已发布的客户端做额外的兼容（设计说明 §2.1 末尾），Tauri 版的数据仍要能读。共同要顾及的：

- 旧版 Tauri 的库按原样打开：`api_settings.setting` 的 JSON 加字段要有默认值；新表只能追加到 `MIGRATIONS`（Turso）和 `migrations/{postgres,mysql}/`（bridge）。
- 服务器同步（`TreeStore::replace` 整份推送、`LocalItems::syncable`）、导出 / 导入（`import/own.rs`）、备份格式（`cyberapi_core::backup`）都要带上新字段，旧格式照常能读。
- 任何其他读者应该看到的修改都要加 `revision`。

### #71 单个接口覆盖超时、重定向、证书校验（已完成）

接口加一个“设置”标签（或弹出框），三项可分别覆盖，默认跟随全局设置。`ApiState` 发送时把接口的覆盖叠在 `SendPrefs` / 超时上；302 的提示改为指向这个接口的设置。

### #66 更多认证类型和目录级继承（已完成）

认证类型：无 / Basic / Bearer / API key（放请求头或 Query），密钥打码；接口可以选“继承目录 / 项目”，目录能设置认证（和请求头）。

- 现在 `HttpRequest.auth` 是 KV 列表、只用第一条作 Basic，要兼容读旧数据。
- 目录现在只有名称和子项，要有地方存目录的设置；继承按树从近到远取，由 core 算出。

### #61 命名环境（已完成，可选的整理工具没有做）

命名环境（dev / staging / prod…），工作区顶部切换并显示当前环境；自定义变量和全局请求头可以按环境取值。

- 现有数据的对应：同名的环境变量行各自成为该变量在一个环境里的值；当前“同名启用两条取第一条”的规则要换掉。
- 当前选的环境属于读者自己（prefs），环境的定义属于项目（同步、加 revision）。

## 第 5 批：0.3.0 之后，小而确定

来自 [feature-research-0.3.md](./feature-research-0.3.md) 第 6 节。不在 GitHub 上建 issue，进度只记在这里，提交信息写功能编号（F4、A1…）。每开始一项之前先和维护者确认范围。

| 编号 | 内容 | 状态 |
|---|---|---|
| F4 | 现有 issue 的遗留：#43 第 2 项（批量编辑的占位文字，Query 是 `page: 1`）79b1a76；#43 第 5 项（发送时上一份响应变暗）e2e8f4c；#51（响应区的“原始报文”标签页，像 `curl -v` 那样排版，规则是 `HttpResponse::exchange`）d77b7e4；#44：macOS 上放大后重开、还原、位置都正常，修了“窗口”菜单的缩放和最小化不起作用 6c20e88。Linux X11 上的还原尺寸没法在这里验证：GPUI 先按普通尺寸建窗口再立刻请求放大，有的窗口管理器会把放大后的尺寸当成普通尺寸；可以试的改法是 Linux 上先按普通尺寸打开、第一帧画完再放大 | 已完成（Linux 部分待验证） |
| A1 | 声明式断言：接口的“断言”标签页，每行对显示的响应实时检查，失败时写出实际值；状态行显示通过几条，点它打开标签页（原想在响应区另加一个标签页，响应区的标签已经放不下，结果就写在编辑器里）；只在接口上，不从目录继承；数据模型编号 6；MCP 的 `get_api` 带上断言 a93243f、e62e454 | 已完成 |
| B3 | 从 URL 导入（#59 推后的一半）：导入框里是地址时“获取”，经服务层的 `fetch_text` 取回；重新导入时合并：弹窗里选“只添加新的”（默认）或“更新已有的”，按方法加路径对上 | 已完成：5edd405（从 URL 导入）、829eec1（合并） |
| B4 | 导入时带上认证和环境：Postman 请求 / 目录 / 集合级与 Insomnia 的 Basic、Bearer、API key，OpenAPI 的 `securitySchemes`（值留空）；Postman 环境文件、Insomnia 子环境成为命名环境。已定：项目已有认证时保留项目的；同名环境只补上没有的变量 | 暂缓 |
| C1 | 签名类认证（AWS SigV4、OAuth 1.0、Hawk、JWT、Akamai EdgeGrid） | 暂缓：每家的签名都有些不同，维护者决定先不做（2026-10-26） |
| C2 | Digest 认证：认证里多一种 Digest（接口、目录、项目都能设，两端都能用）；每次先发一次拿到 401 的质询，再签名重发，MD5、SHA-256 及其 -sess，qop=auth；不答 auth-int 时说明原因；复制为 cURL 写 --digest；数据模型编号 7 | 已完成 |
| D1 | `prompt()` 模板函数：`prompt(说明, 预填)`，发送前一个弹框问齐，同一个说明只问一次，取消即不发送；填过的按项目记到应用退出（不存盘），下次预填，复制为代码、获取 Schema、gRPC 反射也用它；MCP 的发送遇到它报错 | 已完成 |
| D2 | `env()` 模板函数（仅桌面）：`env(名称)` 先读进程自己的环境变量，没有的再读登录 shell 导出的（从 Dock 打开的应用也认得 `.zshrc` 里的），结果缓存，找不到时重读；没设置就不发送并提示；网页版和 bridge 上的 MCP 不读 | 已完成 |
| C5 | 客户端证书与自定义 CA（桌面）：设置的“请求”分区按主机配置客户端证书（`api.example.com`、`*.example.com`，可加端口；PEM 证书 + 私钥，或 PKCS#12 文件 + 密码）和额外信任的 CA（PEM），只存路径、每次发送时读取，不同步；bridge 的引擎不读这些文件；复制为 cURL 时带上 | 已完成 |

应维护者要求插进来的：

| 编号 | 内容 | 状态 |
|---|---|---|
| 修复 | 首页显示期间按 `SERVER_POLL` 重新列服务器项目、窗口获得焦点和点刷新按钮时也列：团队新加的项目、别人改的接口数不用离开首页就能看到 126b92b | 已完成 |
| G1 | 本机项目上传成服务器项目：卡片菜单“上传到服务器…”，选团队或个人空间；同一个 id，原地变成副本，历史、固定、环境选择都保留；项目里的设置菜单没有加入口 470de4a 及之后 | 已完成 |
| 修复 | 带 Touch Bar 的 Mac 上关闭窗口（如设置窗口）就崩溃（`_NSTouchBarFinderObservation invalidate` 抛 `NSRangeException`），zedis 同样。原因在 accesskit_macos 0.26.3，上游 PR #835 已修、发布在 0.27.2，但 GPUI 后端还用不上；先用 `patches/accesskit_macos`（0.26.3 加 #835 的修复）。用 Xcode 的 Touch Bar 模拟器复现并验证，细节见 `patches/README.md`。诊断开关 `CYBERAPI_NO_ACCESSIBILITY` 在 272d8c1 | 已完成 2403289 |
| #74 | 接口记住的 JSONPath 过滤在新响应上没有匹配时，Pretty 显示整个响应体、Tree 显示空：编辑器记下的“显示的是过滤结果”不对，过滤在后台跑完后 Pretty 不刷新。现在没有匹配时两个视图都显示完整响应体，状态行用警告色说明 a3d64d1 | 已完成 |
| #75 | GraphQL 格式化后光标回到开头，从 Schema 浏览器插入的字段写到了 `query` 前面。格式化后光标留在原来的词旁；插入的字段在选择集里就写在光标处，否则写在光标所在的选择集或第一个操作的末尾 57900fa | 已完成 |
| #76 | 接口“设置”标签里的超时没有说明：加上每项的说明，写超时和读超时相加是一个总限时 a6fbf82 | 已完成 |
| #77 | 复制为 JavaScript fetch 时带的 `Cookie` 头会被浏览器丢掉：在这个头上面加一行注释 f3e50af | 已完成 |

## 第 6 批起：其余计划中的功能

来自 [feature-research-0.3.md](./feature-research-0.3.md) 第 6 节；维护者要求不再逐项确认，需要定的写进那份文档第 9 节，按那里的默认做法做。顺序按投入从小到大，一项一个提交。C1 维护者决定不做。

| 编号 | 内容 | 状态 |
|---|---|---|
| B4 | 导入时带上认证和环境：Postman 请求 / 目录 / 集合与 Insomnia 请求 / 目录的 Basic、Bearer、API key、Digest；OpenAPI / Swagger 的安全方案成为项目认证（值留空）；Postman 环境文件、Insomnia 子环境成为命名环境。项目已有认证时保留项目的，同名环境只补没有的变量 | 已完成 |
| B2 | 导出 Postman Collection v2.1：目录层级、Query / 请求头（含关掉的）、各种请求体、各层认证；共用变量成为集合变量；自己的 Postman 导入能原样读回 | 已完成 |
| B1 | 导出 OpenAPI 3.0（YAML）：地址前缀变量的值或主机成为 servers，`{{id}}` 路径段成为路径参数，Query / 请求头带示例，请求体带示例和推断的 schema，目录路径成为 tag，认证成为 securitySchemes（不写值） | 已完成 |
| B5 | 导入 HAR（每个调用一个接口，跳过页面资源、非 HTTP 地址和重复的，多个主机时各一个目录）、Hoppscotch 集合 / 全部导出 / 环境（`<<var>>` 换成 `{{var}}`）、Bruno 单个 `.bru` 请求或环境文件、OpenCollection YAML（整份导出或单个请求；环境成为命名环境）；各层认证照 B4；`{{process.env.X}}` 换成 `{{env(X)}}` | 已完成 |
| B6 | 复制为代码加 Node axios、PHP Guzzle、Rust reqwest（blocking）、Java HttpClient、C# HttpClient、Swift URLSession；菜单分三组（命令行、脚本、编译型），cURL 仍在第一个；跟随重定向、不校验证书照设置写，Digest 在 Guzzle、.NET、URLSession 里写成库自己的应答，其余写一行注释。axios、reqwest、URLSession 的代码对本地服务器实际跑过 | 已完成 |
| D4 | 历史：每个接口保留的条数在设置的“数据”里改（5–100，默认 10，`SendPrefs.history_size`；`HistoryStore::add` 带上条数，bridge 限在这个范围里，旧页面不带时按 10）；项目的“最近发送”（`HistoryStore::recent`，最近 50 条、不带响应体；树面板的项目菜单和响应区的历史菜单里打开，点一条打开接口并显示那条响应）；“对比两条响应”按行对比响应体（`cyberapi_core::diff`，JSON 先缩进，未变的折叠，过长时只比前 20000 行） | 已完成 |
| A3 + A2 | 运行器，运行中提取变量 | |
| A4 | `cyberapi run` 命令行 | |
| F2 | 命令面板 | |
| F3 | 接口、目录、项目的说明，响应存为示例 | |
| H1 | MCP 写入工具 | |
| G6 | Helm chart | |
| G4 | 项目历史版本与回滚（bridge） | |
| C4 | OAuth 2.0（client credentials、password） | |
| F1 | 标签页 | |
| E1 | WebSocket | |
