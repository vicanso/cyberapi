# Issue #55 起的处理计划

> 状态：A、B 两组已完成（2026-10-25），C 组分四批待做，按批次顺序进行。每个 issue 单独提交，提交信息带 issue 编号。

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

### #63 JSONPath 过滤响应

响应体 Pretty / Tree 视图下方加过滤框（RFC 9535 JSONPath，用纯 Rust 的实现），只显示匹配的部分，结果是匹配值的数组；过滤条件按接口记住，属于读者自己（和 prefs 一样不进同步、不进备份）。

- 在完整响应体上过滤（同 #57，受 `TREE_LIMIT` 限制），解析放到后台，不卡界面。
- 过滤后的复制、保存是否用过滤结果，做之前定。

### #58 显示实际发出的请求

Details 里加“请求”一节：最终的方法和 URL、实际发出的每个请求头（标出来源：全局请求头 / Cookie / 认证 / 默认）、模板求值后的 body。

- `HttpResponse` 加一个记录发出请求的字段，由 `cyberapi_service::send` 按 `PreparedRequest` 填（跟随重定向时记最后一跳）；旧的历史条目没有这个字段，按 `serde(default)` 读。
- 引擎自己加的头（`Accept-Encoding`，见 `cyberapi-http` 的 `native.rs`）也要算进去，否则和实际不符。
- `Authorization`、`Cookie`、API key 默认打码，点一下显示；body 的保存上限同 `HISTORY_BODY_LIMIT`。
- 网页版经 bridge 发送，`PreparedRequest` 在页面里就有，两端一样。

### #69 复制为代码

`>_` 按钮和树的菜单从“复制 cURL”改为“复制为”菜单：cURL（默认）、HTTPie、JavaScript `fetch`、Python `requests`、Go `net/http`。

- 生成器放在 core，和 `curl::curl` 一样从 `PreparedRequest` 生成；multipart 和文件在各语言里的写法分别处理。
- gRPC 仍只有 grpcurl。

## 第 3 批：GraphQL 和网络

### #70 GraphQL：格式化、浏览 Schema、错误位置

- 格式化查询：能完整解析时才格式化（写到一半的查询不动），缩进同 `format::JSON_INDENT`。
- 浏览 Schema：点“Schema：N 个类型”打开面板，列类型、字段、参数和说明（`graphql_schema` 已经取了 `description`），点击插入到光标处。数据来自 `WorkspaceState` 按 URI 缓存的 Schema。
- 错误位置：`graphql::response_errors` 已读出错误，再读 `locations` 的行列，在查询编辑器里标出来，参考请求体 JSON 错误标位置的做法。

### #60 代理

设置里加代理：跟随系统 / 环境变量（`HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY`、`NO_PROXY`，默认）、手动（HTTP CONNECT、SOCKS5，可带认证和绕过列表）、不用代理。

- `NativeEngine` 每个请求自己建连接：HTTPS 走 CONNECT 隧道，HTTP 用绝对形式的请求行，SOCKS5 握手；DNS / TCP / TLS 分段计时经代理时的含义要重新定义并在 Details 说明。
- 只对桌面版有意义。bridge 要不要走出站代理是部署选项（命令行 / 环境变量），不是读者的设置，网页版设置里不显示。
- 客户端证书（mTLS）以后再说。

## 第 4 批：数据模型（#71、#66、#61）

三个都改请求或变量的数据模型，先一起设计，单独写一份设计说明再动手。共同要顾及的：

- 旧版 Tauri 的库按原样打开：`api_settings.setting` 的 JSON 加字段要有默认值；新表只能追加到 `MIGRATIONS`（Turso）和 `migrations/{postgres,mysql}/`（bridge）。
- 服务器同步（`TreeStore::replace` 整份推送、`LocalItems::syncable`）、导出 / 导入（`import/own.rs`）、备份格式（`cyberapi_core::backup`）都要带上新字段，旧格式照常能读。
- 任何其他读者应该看到的修改都要加 `revision`。

### #71 单个接口覆盖超时、重定向、证书校验

接口加一个“设置”标签（或弹出框），三项可分别覆盖，默认跟随全局设置。`ApiState` 发送时把接口的覆盖叠在 `SendPrefs` / 超时上；302 的提示改为指向这个接口的设置。

### #66 更多认证类型和目录级继承

认证类型：无 / Basic / Bearer / API key（放请求头或 Query），密钥打码；接口可以选“继承目录 / 项目”，目录能设置认证（和请求头）。

- 现在 `HttpRequest.auth` 是 KV 列表、只用第一条作 Basic，要兼容读旧数据。
- 目录现在只有名称和子项，要有地方存目录的设置；继承按树从近到远取，由 core 算出。

### #61 命名环境

命名环境（dev / staging / prod…），工作区顶部切换并显示当前环境；自定义变量和全局请求头可以按环境取值。

- 现有数据的对应：同名的环境变量行各自成为该变量在一个环境里的值；当前“同名启用两条取第一条”的规则要换掉。
- 当前选的环境属于读者自己（prefs），环境的定义属于项目（同步、加 revision）。
