# Issue #55 起的处理计划

> 状态：A、B 两组和 C 组第 1–4 批及 #63 全部已完成（2026-10-31）。还搁置的是 #59 的从 URL 导入，以及第 4 批可选的环境整理工具。每个 issue 单独提交，提交信息带 issue 编号。

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
| #63 | JSON 响应在 Pretty / Tree 视图用 JSONPath 过滤，开关在响应状态行，条件按接口记住；响应状态行另有搜索按钮，支持在响应体和原始报文中查找并修复 Ctrl/⌘+F 首字母被覆盖的问题 | 45415d1 |
| #58 | Details 显示实际发出的请求：每个请求头及其来源（含引擎加的），请求体，密钥打码可切换 | 13fc381 |
| #69 | 复制为 cURL、HTTPie、JavaScript fetch、Python requests、Go net/http（响应的 `>_` 菜单和接口树子菜单） | 98a24a1 |
| #70 | GraphQL：“格式化”也缩进查询（保留注释）；响应的错误位置标在查询里；浏览 Schema，点字段写进查询 | 7ba0e05、4c148ad、38e7c25 |
| #60 | 桌面版的代理：环境变量（默认）/ 在设置里填写（HTTP、SOCKS5，可带账号和绕过列表）/ 不使用；本机地址始终直连；bridge 始终直连 | 03da11a |
| #71 | 接口的“设置”标签：超时、HTTP 版本、跟随重定向、校验证书，各项默认跟随全局设置；没跟随的 302 提示里可直接打开它；数据模型编号加到 2 | 5916e78 |
| #66a | 接口的认证：继承 / 无 / Basic / Bearer / API key（请求头或查询参数），值可用模板函数，密钥打码；旧的 Basic 列表读成 Basic、不再写回；数据模型编号加到 3（目录 / 项目的认证与继承是 #66b） | 7235d85 |
| #66b | 目录设置（认证、请求头）和项目认证，接口选“继承”时按树从近到远取，认证标签说明来自哪里；各存储、同步、备份、导出、MCP 都带上；数据模型编号加到 4 | 420b28d、cce2ba0 |
| #61a | 命名环境的存储与规则：环境属于项目，变量可以放进某个环境，发送时当前环境的同名变量优先、其余用共用的；当前环境属于读者；各存储、同步、备份、MCP（`send_api` 可指定环境）都带上；数据模型编号加到 5（界面是 #61b） | c82de97 |
| #61b | 标题栏切换环境；变量弹窗分层显示（共用 / 各环境），就地新建、重命名、删除环境，列出被覆盖的共用变量；地址栏手写的 `{{NAME}}` 前缀移到选择器 | c330561 |

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
| A3 + A2 | 运行器：目录或项目菜单的“运行…”，按树的顺序串行发送，可带 CSV / JSON 数据每行一遍（列用 `{{value(列名)}}` 读），请求要的 prompt 在运行弹窗里一次填好；每条结束即显示响应、断言结果，汇总通过 / 失败（有断言按断言，没有时 4xx / 5xx 或 gRPC 非 OK 算失败），运行结束后点一条打开对应接口；关掉弹窗即停止；发出的请求照常记进历史，运行结果不存。提取（`HttpRequest.extractions`，数据模型编号 8）写在断言标签页下面，只在一次运行里有效，后面的请求用 `{{value(名称)}}` 读（也作地址前缀的 `{{名称}}`）。规则在 `cyberapi_core::run` / `assertions::extract`，运行在 `cyberapi_service::run` | 已完成 |
| A4 | 命令行 `cyberapi-cli run`（新的 bin crate `crates/cyberapi-cli`，不依赖 GUI）：读项目文件（自己的导出保留 id，其他能导入的格式也行），项目放在内存里，不碰本机库也不连 bridge；`--env` / `--env-file` / `--var` / `--data` / `--prompt` / `--folder`，`env()` 读进程的环境变量；每条结束打印一行和没通过的原因，`--junit` 写 JUnit XML，退出码 0 / 1 / 2。还没有安装包，从源码构建（`make cli`），见 feature-research §9 | 已完成 |
| F2 | 命令面板：任何页面按 ⌘/Ctrl-K（网页版在 Windows / Linux 上 Ctrl-K 被浏览器占用，快捷键一览不列）；查找任意项目里的接口（当前项目的立即可用，其他项目按最近修改读最多 50 个）、项目、环境（切换）和命令（新建、导入、运行、最近发送、环境变量、返回项目、设置、Cookie、快捷键）；↑↓ 移动、回车选中。打开别的项目的接口时，项目打开并读完读者状态后再选中（`AppState::open_at` → `WorkspaceState::select_when_loaded`） | 已完成 |
| F3 | 说明与示例：接口的“说明”标签页（纯文本，Markdown 原样显示；示例列表，点开看响应体，可删除），历史菜单“存为示例…”（有状态码、文本、256 KB 以内）；目录的说明在目录设置里；项目已有说明。`HttpRequest.description` / `examples`、`FolderSettings.description`，数据模型编号 9。MCP 的 `get_api` 带上说明和示例（打码、截断）；Postman 导出说明和保存的响应、OpenAPI 导出 description 和响应示例；Postman（含保存的响应）、Insomnia、OpenAPI 导入读说明 | 已完成 |
| H1 | MCP 写入：签发令牌时单独勾选“允许修改”（`McpToken.can_write`，存在旁边的 `mcp_token_writes` 表，旧令牌读作不允许；签发在一个事务里），这样的令牌多出 `create_api`（目录可用路径或 id）和 `update_api`（给出的字段替换原来的，没给的不动，可改名），账号对项目至少是编辑者；不能删除。规则在 `cyberapi_core::mcp::written` / `find_folder` | 已完成 |
| G6 | Helm chart：`deploy/helm/cyberapi`（Deployment + Service + 可选 Ingress，数据库由用户提供：`store.url` 或自己的 Secret，不给时拒绝安装；初始化 token 同理；bridge 的选项都有对应 value；非 root、只读根文件系统，连上数据库才就绪）。版本随 `scripts/bump-version.sh` 一起改（顺带修好它备份不同目录同名文件的问题），`make helm-lint` 检查；用 helm 3.16 lint / template 验证过 | 已完成 |
| G4 | 项目历史版本：bridge 在每次整份替换（同步、上传、恢复）前把项目原样存一份（`project_versions` 表，每个项目 50 份，空项目不存），`VersionStore`（只有 `SqlStore` 实现，经 `Call::VersionsList` / `VersionsGet` 给页面和登录后的桌面）；项目菜单“历史版本…”列出，编辑者在没有未同步修改时可恢复（用旧配置再整份替换，现在的样子也被存下，可撤回），之后副本取服务器的 | 已完成 |
| C4 | OAuth 2.0（client credentials、password）：认证多一种 `Auth::OAuth2`（授权方式、令牌地址、客户端 ID / 密钥、密码方式的用户名和密码、范围、客户端用 Basic 头还是放在请求体里，都是模板；数据模型编号 10）。服务发送前申请令牌（`cyberapi_core::oauth` 拼请求、读应答；`cyberapi_service` 的 `Tokens` 按渲染后的全部字段缓存在内存里，应用一份，不存库、不同步；过期前 30 秒、或接口回 401 时重新申请，有刷新令牌时先用它；取不到时请求不发，原因作为响应记进历史）；内省、反射同样签名，复制为代码只用已有的令牌。Postman / Insomnia / Hoppscotch / Bruno 导入，Postman / OpenAPI 导出（OpenAPI 只写流程、令牌地址和范围）；MCP 打码。授权码 + PKCE 见「C4 后半」。之后（维护者 2026-10-27 定）：令牌像 Cookie 一样存在读者那里（`PrefStore::oauth_tokens`，按哈希存、不含密钥，命令行和 MCP 仍只在内存），令牌请求可带 `audience` / `resource`（数据模型编号 12） | 已完成 |
| F1 | 标签页：打开过的接口留成标签页（编辑列顶部，`views/workspace/tabs.rs`），每个标签的 `ApiState` 留在 `WorkspaceState.kept` 里，切回来时编辑内容、响应、正在发送的请求都还在；离开一个标签时保存它还在等的修改。列表按读者存在 `WorkspacePrefs.open`，下次打开项目时恢复（编辑器在第一次显示时才建）；新打开的放在当前标签之后，最多 `MAX_TABS`（16）个，超出时关最久没显示的（不关当前的和正在发送的）；接口被删除时标签跟着关，打开它旁边的。关闭按钮、中键、右键菜单（关闭 / 关闭其他 / 全部关闭），⌘/Ctrl-W（Mac 上菜单的 Close Window 先拿到 ⌘W，工作区接住它的动作：有标签时关标签，没有时照旧关窗口）、Ctrl-Tab / Ctrl-Shift-Tab。置顶不变。顺带修了同一接口两次整份保存可能乱序落地的问题（`ApiState` 的保存改成 `run_serial`） | 已完成 |
| E1 | WebSocket（先只做桌面版，见 feature-research §9）：新的方法 `HttpMethod::WebSocket`（存为 `"WS"`，数据模型编号 11），body 是下一条要发的消息。`cyberapi_core::ws`（地址换算、收发记录的 JSON 行格式和读回、websocat 命令）；引擎 `HttpEngine::open_socket`（默认“只有桌面版能开”），`NativeEngine` 在共用的 `dial`（建连接、计时、地址规则、代理隧道、客户端证书）上用 HTTP/1.1 升级并核对 `Sec-WebSocket-Accept`，之后 tokio-tungstenite 收发；服务 `cyberapi_service::socket`（`connect` 像发送一样准备，带 Cookie 和 OAuth；`message` 每次按当时的 body 求值；`keep` 在结束时把收发记录连同时间、结束原因存进历史）；`ApiState` 复用“发送中 + 实时流”：连上后响应区按事件流实时显示，再点一次是断开；请求编辑器的“连接 / 断开”和“消息”标签的“发送消息”，连上后 ⌘/Ctrl-Enter 发消息。运行器、命令行、MCP 的 `send_api` 遇到 WS 接口说明原因跳过（`SendFailure::WebSocket`），MCP 也不写它；Postman / OpenAPI 不导出 | 已完成 |
| D3 | 秘密变量：项目设置里记下哪些变量是秘密的（`ProjectSettings.secret_variables`，跟着项目同步、进备份，数据模型编号 13；不用改表），值是每个读者自己的（`PrefStore::secret_values`，按项目一份文档：桌面本机库、网页 bridge 上按账号；随项目删除，不进备份），变量自己那一行的值保持为空。`cyberapi_core::secrets`：`fill` 发送时填入、`mask_sent` 让历史里存的请求打码；`Sender.secrets` 由调用方从 `WorkspaceState::secrets` 填。变量弹窗每行一把锁（编辑者切换），秘密行的值打码、可切换显示，查看者也能填自己的；MCP `get_variables` 只给名字，`send_api` 用该账号自己的值；OpenAPI 导出不带秘密值 | 已完成 |
| G2 | Mock 服务（桌面版）：规则在 `cyberapi_core::mock`（`Routes::of(树)` 按方法和路径找接口，地址去掉变量、协议、主机，`{{id}}` / `:id` / `{id}` 匹配任意一段，固定段多的优先；回第一个示例或 `x-mock-example` 点名的；没有接口 404、方法不对 405、没有示例 404，都说明原因；CORS 预检和 `Access-Control-Allow-Origin: *`），服务器是 `cyberapi_http::MockServer`（hyper，只听 127.0.0.1，丢掉即停止），经 `this_build::start_mock` 在桌面的 tokio 运行时上启动（网页版没有）；`MockState` 由工作区持有，项目改动 300ms 后重建路由，记下最近 100 个请求；项目菜单“Mock 服务…”（`views/workspace/mock_dialog.rs`），运行时菜单里写出地址。bridge 上的 Mock 以后再做 | 已完成 |
| H1 后半 | MCP 能看到断言结果：`send_api` 的回答带上该接口每条启用的断言（`cyberapi_core::assertions::report`：是否通过，没通过时写出实际值，检查做不了时说明原因）；不新增工具，结果不写进历史 | 已完成 |
| E3 | GraphQL 经 WebSocket 订阅（桌面版）：地址写成 `ws://` / `wss://` 的 GraphQL 接口用 graphql-transport-ws，`next` / `error` 按事件流显示，结束时整段存进历史；HTTP 订阅仍走事件流。网页版不能连，运行器、命令行、MCP 跳过（`SendFailure::GraphQlSocket`） | 已完成 |
| E2 | gRPC 客户端流和双向流（桌面版）：`HttpEngine::open_call` 发流式请求体；连接后逐条发消息，客户端流有“结束发送”、之后收那一条回复，双向流边发边收，收发记录结束时存进历史。网页版不能连，运行器、命令行、MCP 跳过（`SendFailure::GrpcStream`）。一元调用和服务端流不变 | 已完成 |
| F1 后半 | 临时请求：项目菜单和命令面板的“新建临时请求”开一个不在树里的标签，用所在项目的变量和环境。内容在读者自己的 `WorkspacePrefs.scratches`（不进项目、不同步，重启后还在）；关掉标签就丢掉。编辑者“保存到项目…”选目录后才成为接口；查看者也可以新建 | 已完成 |
| C4 后半 | OAuth 授权码 + PKCE（桌面版）：授权方式多一种 `Grant::AuthorizationCode`（授权地址是模板，回调端口 `0` 为随机；数据模型编号 14）。点“授权”用 PKCE（S256）打开系统浏览器，在 `127.0.0.1` 上收一次回调（`cyberapi_http::CallbackListener`，复用 Mock 的 hyper），换来的令牌照现有方式存。网页版能保存设置，按钮说明只在桌面版可用。命令行、运行器、MCP 的 `send_api` 不打开浏览器：没有刷新令牌时回答先在桌面版授权。Postman（`authorization_code` / `authorization_code_with_pkce`）、OpenAPI（`authorizationCode`，Swagger `accessCode`）、Insomnia、Hoppscotch、Bruno 导入，Postman / OpenAPI 导出 | 已完成 |
| A5 | 前后置脚本 | 暂缓：维护者不想引入 JS 引擎（2026-10-27） |

## 0.4.0 之后

来自 [feature-research-0.4.md](./feature-research-0.4.md)（2026-10-29）。同上：不建 issue，提交信息写功能编号，需要定的写在那份文档里、按其中的默认做法做，一项一个提交。顺序按投入从小到大；可做可不做的 F7、A7、B8、D7 见那份文档第 4 节，没有排进来。

| 编号 | 内容 | 状态 |
|---|---|---|
| B7 | 来源绑定与一键同步：项目记住导入用的 `http(s)` 地址（`ProjectSettings.source`，跟着项目同步），“从来源同步…”取回后先显示新增 / 更新 / 来源里已没有的摘要，确认后按 B3 的“更新已有的”合并，来源里没有了的不删。来源在“从来源同步…”弹窗里查看、修改、不再记住（不另放进项目设置）；导入时记下导入到的目录，同步的新接口进那里；合并不再写入没有变化的接口；数据模型编号 15 | 已完成 |
| D6 | 生产环境标记：`ProjectSettings.protected_environments`；选中时环境切换器用醒目颜色，发送会改数据的请求（非 GET / HEAD / OPTIONS、mutation、gRPC、WebSocket 连接）和运行器开始前确认；MCP 在这个环境里拒绝这类请求，命令行不问。规则 `environments::asks_first`；数据模型编号 16 | 已完成 |
| G8 | 历史版本对比：和现在或和之后那一版比，按 id 列出接口、目录、设置、变量、环境的变化，接口的请求按行对比；规则在 core（`cyberapi_core::versions`），弹窗里就地切换，查看者也能对比 | 已完成 |
| D5 | 目录级、接口级变量：`FolderSettings.variables` / `HttpRequest.variables`，`{{value(名称)}}` 或地址前缀读，优先级 运行中的值 > 接口 > 目录（内层胜）> 当前环境 > 共用；经 `inherit::inherited` 进所有发送入口；Bruno、Postman 导入，Postman 导出；数据模型编号 17 | 已完成 |
| F6 | 本机数据保护：桌面版每天自动备份到 `backups/`（现有格式，留 7 份，可关）；树里删除接口 / 目录后 10 秒内可撤销，按原 id 和位置放回（`cyberapi_core::undo`，经 `TreeStore::replace` 写回，不加 store 方法），响应历史不恢复 | 已完成 |
| F5 | 复制 / 移动到其他项目：右键菜单选目标项目和目录，带上请求全部内容和目录设置，用新 id；移动是复制后删除，历史不跟过去；完成后列出目标项目里没有的变量（`cyberapi_core::transfer`） | 已完成 |
| A6 | 响应结构校验：断言多一种对象 Schema（JSON Schema，draft 2020-12，不取远程 `$ref`）；OpenAPI 导入时第一个 2xx 的 JSON schema 成为一条“来自导入”的断言（B7 同步时更新它），导出 OpenAPI 时用它作响应 schema；校验库要能编译到 wasm（`jsonschema` 关掉默认 feature）；数据模型编号 18 | 已完成 |
| D7 | 补全模板函数（第 4 节的可选项）：`base64Decode`（`b64d`）、`urlDecode`、`sha1`、`sha512`、`hmacSha1`、`hmacSha512`，`now` 可带偏移（`now(%Y-%m-%d, -1d)`，单位 s / m / h / d / w）；不改数据模型 | 已完成 |
| B8 | 导入 `.http` 文件（第 4 节的可选项）：REST Client / JetBrains HTTP Client 的 `###` 请求、`@变量`、多行 Query、表单和 multipart、GraphQL、WEBSOCKET、`HTTP/2`、`# @name` / `@no-redirect` / `@prompt`，系统变量换成模板函数；`http-client.env.json` 和 REST Client 的环境设置成为命名环境（`import/http_file.rs`）；命令行也能直接运行 `.http` 文件 | 已完成 |
| A7 | 压测（第 4 节的可选项，只有桌面版）：树里接口的右键菜单“压测…”，并发 1–50，按次数（≤ 10 万）或时长（≤ 600 秒），显示吞吐、错误率、平均 / 最快 / 最慢、P50 / P90 / P95 / P99 和各状态码、错误的数量，不写历史；规则在 `cyberapi_core::load`，服务 `cyberapi_service::load`；生产环境先确认 | 已完成 |
| F7 | 标签在新窗口打开（第 4 节的可选项，只有桌面版）：标签右键“在新窗口打开”，新窗口左右并排显示请求和响应，和主窗口共用同一个 `ApiState`；同一个接口同时只有一个编辑器（主窗口那一栏显示占位，可切换过去或移回）；关窗口即移回，关标签或离开项目时窗口一起关 | 已完成 |

## 0.4.1 之前的排查

2026-10-30 对 v0.4.0 之后的 11 项功能和整个应用的性能做了一次只读排查，结果在 [review-0.4.1.md](./review-0.4.1.md)：27 条 bug（`BUG-n`）、16 条性能问题（`PERF-n`）、5 个要先定做法的问题（`Q-n`），每条有位置、场景、原因和修复方向。建议先修 BUG-1（`now()` 的偏移溢出会让 bridge 进程退出）和 BUG-2…BUG-5（D6、F6、F7 的发送和数据问题），再修 BUG-6…BUG-12 和 PERF-1…PERF-4。修的时候一项一个提交，提交信息写编号，修完在那份文档的汇总表里标“已修复（提交）”。

## 0.4.1 之后的新功能

2026-10-30 评估并按从小到大的顺序逐项实现（一项一个提交，提交信息写功能编号；不确定或替维护者先定下的设计决策记在下方）：

| 编号 | 内容 | 状态 |
|---|---|---|
| F8 | 命令面板（⌘/Ctrl+K）按路径搜索接口：`FoundApi` 带上去掉 `{{ENV}}` 前缀的 `path`，支持按路径、多词（如 `POST /v1/orders`、项目名 + 路径）搜索，并在候选行接口名旁以次要颜色显示路径；不改数据模型 | 已完成 |
| F9 | 表单文件（Form File）补齐：修 `RequestEditor::reload` 漏同步 `form.options.files` 的 bug；`KvEditor` 在 `Multipart` 空白行也直接展示选文件按钮（空 key 默认填 `file`）并对 `file://` 值给出高亮标识与路径提示；OpenAPI / Swagger 导入 `type: "file"` / `format: "binary"` 的 multipart 字段为 `file://`，未选路径的 `file://` 发送时像 `{{openFile()}}` 一样弹框选文件，且 `file://` 路径支持模板；网页版选文件上传先记为待定（见下） | 已完成 |
| F10 | 键值入参说明（`KvParam.description`）：Query、请求头（含目录请求头）、Form / Multipart 请求体以及目录 / 接口变量每行可填说明（切换文本批量编辑时保留）；OpenAPI / Swagger、Postman、Insomnia 导入，Postman / OpenAPI 导出，MCP 读写带上说明；同步合并时不冲掉读者手写且上游为空的说明；数据模型编号 19 | 已完成 |
| B9 | 目录级 OpenAPI / 来源地址绑定（`FolderSettings.source`）：目录设置和目录右键菜单“从来源同步…”（含“仅保存地址”，无需先全量导入）以及“导入到此目录…”均可单独绑定、修改、清除目录的来源 URL，预览与合并（`merge_folder` / `preview_folder`）严格限定在该目录子树内以免不同微服务目录的同路径接口（如 `/health`）互相覆盖；数据模型编号 20 | 已完成 |
| F11 | 接口出入参字段说明（Schema 驱动 + 可直接在 CyberAPI 编辑）：保存请求体 JSON Schema（`HttpRequest.request_schema`，OpenAPI 导入/同步填入、导出优先使用、MCP 带出，数据模型编号 21）；从 `KvParam.description`、`request_schema` 和 A6 的响应 Schema 提取嵌套字段树，在接口“说明”标签页展示入参 / 出参字段说明，并允许编辑者直接在标签页里编辑请求与响应 Schema | 已完成 |
| B10 | 来源本机可选的定时只读拉取匹配与更新提醒：读者可在“从来源同步…”弹窗对当前信任的 URL 勾选“在本机定时拉取字段说明”（记在读者自己的 `WorkspacePrefs`，URL 被别人改动后自动失效以防 SSRF）；后台只读拉取并缓存到工作区内存（不写库、不加 `revision`），动态按 `方法 + 路径` 为手建或未同步的接口补齐出入参字段说明，发现上游有新增 / 更新时显示提示 | 已完成 |

### 实施时先替维护者定下的决策（均可改）

1. **网页版 `multipart` 选文件（F9 延伸）**：桌面端现已有完整的 `file://` 与 `{{openFile()}}` 能力；浏览器版发送经 bridge 接收 `PreparedRequest`（`body: Vec<u8>` 已是字节），若加浏览器选文件，需要越过 `gpui-kit` 的平台封装直接调用 `web-sys` 挂 `<input type="file">` 并在页面内存暂存字节（且不能把本机路径写进同步数据）。为守住 `AGENTS.md`“浏览器代码不散落在 view、不为浏览器改变桌面行为”的边界，F9 先把桌面端交互和 OpenAPI 二进制字段导入补齐，网页版文件选择记在这里留待单独评估是否加 `this_build::pick_file_bytes` 接缝。
2. **嵌套 JSON 字段说明的编辑方式（F11）**：不做 Apifox 式“手工逐行填多层嵌套字段表”的重型表单（极易与实际 JSON 请求体脱节），改为“扁平键值参数在表格直接填 `description`（F10）+ 嵌套 JSON 请求体与响应体用 JSON Schema 驱动字段说明树，并提供弹窗直接在 CyberAPI 编辑请求/响应 Schema（F11）”。
3. **定时拉取的安全与协同边界（B10）**：绝不在后台定时自动写项目库（避免多端同时推高 `revision` 产生草稿冲突，也避免共享项目里的恶意内网 URL 被所有队友桌面端自动探测）。定时拉取设计为“读者本机对具体 URL 显式开启的只读缓存”，既能实时匹配展示手建接口的出入参说明，又在检测到差异时提醒编辑者手动一键合并入库。

## 候选优化与后续规划（2026-10-31 整理）

按投入产出比（ROI）分为四组，守住“不引入重型 JS 引擎、本地优先、桌面与网页端共享核心层、保持原生极速”的原则：

### 一、日常调试体验打磨（小投入 S，高频有感）

| 编号 | 功能 | 规模 | 数据模型 | 内容与落点 |
|---|---|---|---|---|
| F12 | 路径参数（`:id` / `{id}`）可视化编辑与自动识别 | S–M | 可选 | 自动识别 URL 中的 `:param` / `{param}` 占位符，在 `Query` 标签页（或紧邻的 Path 分区）以键值行展示（含 `description` 说明，与 OpenAPI 的 `in: path` 参数双向导入导出），发送与生成代码时自动替换进 URL |
| A8 | 从响应体 JSON Tree 一键生成断言 / 提取 / `{{g(...)}}` 引用（已完成） | S | 不变 | 在 [`views/json_tree.rs`](../src/views/json_tree.rs) 节点操作中增加：① 复制为 `{{g(当前接口, 路径)}}`（跨接口引用 token 等）；② 添加为当前接口的 JSONPath 断言（`Assert`）；③ 添加为运行器提取变量（`Extract`） |
| F13 | 根据请求 Schema 一键生成 JSON 请求体骨架（已完成） | S | 不变 | 当接口存在 `request_schema`（来自 F11 手写/导入，或 B10 定时拉取匹配到的入参 Schema）时，在 JSON `Body` 工具栏 `Format` 旁提供“生成示例”按钮，通过 `schema_docs::sample_of_schema` 一键生成并填入 JSON 骨架 |
| F14 | 响应头与请求详情（Details）快速复制 / 过滤 | S | 不变 | 在响应 `Headers` / `Cookies` 及 `Details` 标签页支持单行或整表一键复制，并在条目较多时提供轻量关键字过滤 |

### 二、环境变量与文档协同（中小投入 S–M，团队提效）

| 编号 | 功能 | 规模 | 数据模型 | 内容与落点 |
|---|---|---|---|---|
| D8 | 环境变量多环境矩阵对比与整理（#61 延伸） | S–M | 不变 | 在 [`views/variables_dialog.rs`](../src/views/variables_dialog.rs) 增加“矩阵 / 对比”视图（行为变量名，列为 `共用 \| dev \| staging \| prod`），高亮缺失项，支持在单表内横向编辑各环境值及归并重复同名变量 |
| G3 | `bridge` 托管只读在线接口文档 | M | 不变 | `cyberapi-bridge` 为项目提供可选的只读文档页面，直接渲染项目树、接口说明、入参/出参字段说明表（F10 / F11）、响应示例（F3）与多语言代码片段（B6），方便前端或外部合作方免编辑器查阅 |
| G2b | `bridge` 团队共享 Mock 路由 | M | 不变 | 在 `cyberapi-bridge` 上挂载 `/v1/mock/<project-id>/...`，直接复用 [`cyberapi_core::mock::Routes`](../crates/cyberapi-core/src/mock.rs) 匹配项目的响应示例，让网页端和前端无需在本机起桌面端也能联调团队 Mock 接口 |

### 三、桌面端 AI 与工程化能力延伸（中投入 M，差异化亮点）

| 编号 | 功能 | 规模 | 数据模型 | 内容与落点 |
|---|---|---|---|---|
| H2 | 桌面版本机 MCP Server | M | 不变 | 桌面版在运行期间可按开关在 `127.0.0.1` 启动带本机 Token 校验的轻量 MCP HTTP 端点（复用 `cyberapi_http` 的 hyper 监听与 `cyberapi_core::mcp` 的工具定义），让未部署 `bridge` 的个人桌面用户也能在 Claude Code / Cursor 中直接读写和调测本机项目 |
| A4b | `cyberapi-cli` 发布预编译包 | S | 不变 | 在 [`.github/workflows/package.yml`](../.github/workflows/package.yml) 与 `scripts/checksums.sh` 中打包 `cyberapi-cli` 跨平台二进制产物，方便用户在 GitHub Actions / GitLab CI 中直接下载运行接口断言与 JSON Schema 契约测试 |

### 四、性能与细节尾项（来自 [`review-0.4.1.md`](./review-0.4.1.md) 遗留）

| 编号 | 功能 | 规模 | 数据模型 | 内容与落点 |
|---|---|---|---|---|
| PERF-3b | 大响应体 `Tree` 视图与首次断言检查后台化 | S–M | 不变 | 将 [`views/json_tree.rs`](../src/views/json_tree.rs)（最多 16 MiB）的首次 JSON 解析与节点构建以及 `ApiState::checks()` 的首次响应 JSON 解析放到 `background_spawn`，避免 5–16 MiB 响应切换 `Tree` 视图时主线程卡顿 |
| A7b / F7b | 压测动态模板按次求值与未保存修改读取 / 独立窗口多环境对照 | S–M | 不变 | 压测（A7）对打开着的接口先读编辑器最新内容（Q-5），并在请求包含 `uuid()` / `random()` / `now()` 等动态函数时支持按次求值；独立窗口（F7）评估支持同一接口切换不同环境并排对照 |
