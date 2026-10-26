# 0.3.0 之后：功能调研与建议

> 状态：调研稿（2026-10-25），只是建议，还没有排进计划，也没有写代码。定下来的条目再像 [issue-plan.md](./issue-plan.md) 一样分批、写设计、逐个提交。第 7 节是需要你拍板的问题。

## 1. 结论先说

0.3.0 已经覆盖了一个 API 客户端的基本盘：HTTP / GraphQL / gRPC 一元与服务端流 / SSE，认证与目录继承，命名环境，团队与离线同步，MCP。和同类工具比，最明显的缺口有五块：

1. **测试**：没有断言，没有集合运行器，也没有能放进 CI 的命令行。几乎所有竞品都有，团队最先问的就是它。
2. **互通**：能导入，但不能导出成 OpenAPI / Postman，不能从 URL 导入或重新同步，导入时也不带认证和环境。这直接决定别人愿不愿意换过来。
3. **认证**：只有 Basic / Bearer / API key。OAuth 2.0、Digest、AWS SigV4、客户端证书是企业内网和云 API 的门槛。
4. **协议**：没有 WebSocket；gRPC 只有一元和服务端流，其他支持 gRPC 的客户端四种调用都支持。
5. **工作方式**：没有标签页和临时请求；项目只存在数据库里，不能放进 git。

建议的顺序（第 6 节展开）：

- **第一批，投入小、需求明确：** 声明式断言、导出与导入补全、签名类认证、`prompt()` / `env()` 两个模板函数、客户端证书（桌面），以及现有 issue 的遗留项。
- **第二批，需求强、工作量中等：** 运行器和 `cyberapi run` 命令行、OAuth 2.0、WebSocket（以及随后的 gRPC 双向流）、标签页、接口说明与示例。
- **第三批，先定方向再做：** 文件存储（git 友好）、脚本、Mock、bridge 的 SSO 和项目历史版本。

同时要守住 CyberAPI 现在的优势，见第 8 节：不强制登录、本地优先、不收集数据、不加载远程代码、原生且快。

## 2. 怎么调研的

- **竞品功能**：Postman、Insomnia、Bruno、Hoppscotch、Yaak、HTTPie、Apidog、Kreya、Posting、RapidAPI for Mac（原 Paw）、Thunder Client、Requestly 的官方文档、更新日志和 README（2025–2026）。
- **用户需求**：上面这些开源项目 GitHub 上按 reaction 排序的 issue 和讨论，Hacker News 讨论，V2EX 上的国内讨论。reaction 数是 2026-10-25 查到的大约值。
- **项目自身**：本仓库 [vicanso/cyberapi](https://github.com/vicanso/cyberapi/issues) 还开着的 issue，以及设计文档里写着“以后再做”“这一版不做”的条目。
- **没能确认的**：Reddit 打不开，相关结论只来自引用它的 HN 和博客；Yaak 的投票数来自它的反馈页，是大约值；Apifox 一类国内工具“用户看重什么”的资料多出自厂商文章，只能当作“它卖什么”。详见附录 B。

评估每一项时看三件事：**需求强度**（有多少工具做了、有多少人在要）、**是否合 CyberAPI**（本地优先、原生、两端同一份代码、bridge 代发）、**工作量**。

工作量的记法：**S** 一两个提交、主要在 `cyberapi-core`；**M** 跨 core / service / store / 界面，几个提交；**L** 要先写设计文档，分多步做。

## 3. 和竞品对照

“✓”是 CyberAPI 已有，“部分”是有但不完整，“—”是没有。竞品一列只列做了的，缩写见表下。

| 能力 | CyberAPI | 做了的竞品 |
|---|---|---|
| WebSocket | — | PM IN BR HP YK AD KR RQ |
| gRPC 客户端流 / 双向流 | — | PM IN BR YK AD KR RQ（所有支持 gRPC 的都支持） |
| GraphQL 订阅 | 部分：只有 SSE | 多数经 WebSocket |
| Socket.IO / MQTT | — | PM HP AD RQ / PM HP RQ |
| 前后置脚本（JS） | — | PM IN BR HP AD KR TC RQ（YK HT PO 没有） |
| 声明式断言 | — | BR TC AD，KR 快照测试 |
| 集合运行器 / 数据驱动 | — | PM IN BR HP AD TC RQ KR |
| CI 命令行（JUnit 报告） | — | PM IN BR HP KR AD TC RQ |
| OAuth 2.0 | — | 几乎全部 |
| Digest / AWS SigV4 / NTLM / OAuth 1.0 | — | 大多数 |
| 客户端证书 / 自定义 CA | — | PM IN BR HP YK AD KR TC |
| 文件存储（可进 git） | — | BR YK IN PM RQ KR PO TC |
| 秘密变量（本机加密、不上传） | — | PM YK HT IN BR TC |
| Mock 服务 | — | PM IN BR HP AD |
| 生成 / 发布文档 | — | PM BR HP AD |
| 导出 OpenAPI | — | BR IN |
| 导入 HAR / Bruno / Hoppscotch | — | IN HP RQ / YK RQ PO / — |
| 标签页 | — | 几乎全部 |
| 命令面板 | 部分：项目内 ⌘/Ctrl-F 找接口 | HP PO |
| MCP 服务（读写自己的数据） | 部分：只读加发送 | HP（53 个工具）YK AD |
| AI 生成请求 / 测试 | — | PM BR HP AD RQ HT |
| 自托管团队版 | ✓ | HP（企业版功能另收费）IN BR（企业版） |
| 桌面登录自托管服务器、离线编辑 | ✓ | HP 要求多（[#3523](https://github.com/hoppscotch/hoppscotch/issues/3523) 76 个赞） |
| 原生、非 Electron | ✓ | KR（.NET）、PO（终端）；RA 已基本停止维护 |

缩写：PM Postman，IN Insomnia，BR Bruno，HP Hoppscotch，YK Yaak，HT HTTPie，AD Apidog，KR Kreya，PO Posting，RA RapidAPI for Mac，TC Thunder Client，RQ Requestly。

## 4. 用户最想要什么（需求信号）

按强弱排。链接是证据，数字是 reaction（r）或投票。

1. **本地优先、不要账号、数据归自己、能进 git。最强。** 例如：
   - Insomnia 强制登录 [#6577](https://github.com/Kong/insomnia/issues/6577)（346r）、[讨论 #6590](https://github.com/Kong/insomnia/discussions/6590)（416r）。
   - HN 上 [Bruno 发布](https://news.ycombinator.com/item?id=39653718)（1538 分）、[Postman 宕机](https://news.ycombinator.com/item?id=45645172)（537 分）。
   - Hoppscotch 排第一的 [#870 git 同步](https://github.com/hoppscotch/hoppscotch/issues/870)（107r）。
   - 国内 V2EX 的 [Apifox 挂了](https://v2ex.com/t/1119645)（90 回复，要“LOCAL FIRST”、git 管理）。

   CyberAPI 的本地库加可选的自托管服务器正合这个方向；缺的是一份能 diff 的文件格式。
2. **OpenAPI 往返：从 URL 导入、重新同步、导出。强，每个仓库都有。** 例如：
   - Postman [#1744 导出 OpenAPI](https://github.com/postmanlabs/postman-app-support/issues/1744)（397r）。
   - Bruno [#81](https://github.com/usebruno/bruno/issues/81)（132r）、[#3684 导入时覆盖](https://github.com/usebruno/bruno/issues/3684)（67r）。
   - HTTPie 桌面版前两名 [#14](https://github.com/httpie/desktop/issues/14)、[#202](https://github.com/httpie/desktop/issues/202)。
   - Insomnia [导出 OpenAPI](https://github.com/Kong/insomnia/discussions/3737)（115r）。
3. **脚本、串联、断言、运行器。强。** 例如：
   - Postman [#4193](https://github.com/postmanlabs/postman-app-support/issues/4193)（160r）。
   - Insomnia [前置脚本](https://github.com/Kong/insomnia/discussions/3806)（95 票）。
   - Yaak 反馈榜第二“Request Scripting”（64）。
   - Hoppscotch [#3728](https://github.com/hoppscotch/hoppscotch/issues/3728)（58r）。

   CyberAPI 的 `{{g(接口, 路径)}}` 解决了一部分串联。
4. **标签页、临时请求、多窗口。强。** 例如：
   - Insomnia 排第一的 [#1003 临时标签、不自动保存](https://github.com/Kong/insomnia/issues/1003)（93r）。
   - Postman [#9180 并排标签](https://github.com/postmanlabs/postman-app-support/issues/9180)（112r）。
   - Yaak 反馈榜第一“Multi-tab Support”（66）。

   注意两个方向都有人要：Postman [#4671](https://github.com/postmanlabs/postman-app-support/issues/4671)（100r）要更多自动保存，Insomnia #1003 要更少。
5. **更多协议，WebSocket 第一。强。** 例如：
   - Insomnia [WebSocket](https://github.com/Kong/insomnia/discussions/4477)（366r）。
   - Bruno [#1345](https://github.com/usebruno/bruno/issues/1345)（79r，2025 年已做）。
   - Socket.IO：Bruno [#5887](https://github.com/usebruno/bruno/issues/5887)（37r）。
6. **OAuth 2.0 和云认证。强。** 例如：
   - Bruno [#1003](https://github.com/usebruno/bruno/issues/1003)（130r）、[#2650 用系统浏览器登录](https://github.com/usebruno/bruno/issues/2650)（41r）。
   - Postman [#4209 AWS 配置](https://github.com/postmanlabs/postman-app-support/issues/4209)（82r）。
   - Hoppscotch [SigV4](https://github.com/hoppscotch/hoppscotch/issues/3169)（32r）。
7. **变量与秘密。中强。** 例如：
   - 发送时询问值：Postman [#285](https://github.com/postmanlabs/postman-app-support/issues/285)（373r）、Bruno [#1033](https://github.com/usebruno/bruno/issues/1033)（66r）。
   - 读系统环境变量：Postman [#1603](https://github.com/postmanlabs/postman-app-support/issues/1603)（113r）。
   - 按请求的变量：Insomnia [讨论 #3551](https://github.com/Kong/insomnia/discussions/3551)（171r）。

   嵌套环境（Bruno [#292](https://github.com/usebruno/bruno/issues/292)）CyberAPI 的“共用层 + 命名环境”已经覆盖。
8. **响应：保存为示例、jq。中。** 例如：
   - Bruno [#1235 示例](https://github.com/usebruno/bruno/issues/1235)（49r）。
   - Yaak“历史里保存请求数据”（47）、“examples”（23）。
   - Insomnia [#777 jq](https://github.com/Kong/insomnia/issues/777)（33r）。
9. **目录级请求头和复用。** 证实了 #66 做对了：Postman [#8179](https://github.com/postmanlabs/postman-app-support/issues/8179)（129r）、[#1535 跨集合复用请求](https://github.com/postmanlabs/postman-app-support/issues/1535)（152r）。
10. **多语言、插件、MCP、Vim 模式。中，在上升。** 例如：
    - Bruno 排第一的 [#322 多语言](https://github.com/usebruno/bruno/issues/322)（57r）、[#1529 插件](https://github.com/usebruno/bruno/issues/1529)（49r）、[#4806 MCP](https://github.com/usebruno/bruno/issues/4806)（26r）、[#2373 Vim](https://github.com/usebruno/bruno/issues/2373)（56r）。

**自托管团队的诉求：**

- Hoppscotch 上有票数的只有私有部署（[#1817](https://github.com/hoppscotch/hoppscotch/issues/1817)，109r）、桌面连自托管（[#3523](https://github.com/hoppscotch/hoppscotch/issues/3523)，76r）、通用 OIDC（[#3001](https://github.com/hoppscotch/hoppscotch/issues/3001)，39r）、不用 SMTP 的账号密码（[#3447](https://github.com/hoppscotch/hoppscotch/issues/3447)，34r）、Helm chart（[#3007](https://github.com/hoppscotch/hoppscotch/issues/3007)，25r）。其中前两项和账号密码 CyberAPI 已经有了。
- LDAP、SAML、审计日志几乎找不到社区投票，都是厂商企业版的卖点。

**国内开发者：**

- Apifox / Apipost 的卖点是一份数据同时做文档、调试、Mock 和测试，以及前后置操作：签名加密、提取变量、断言、读数据库。
- V2EX 上的抱怨集中在三点：
  - 卡：上千接口就卡，归咎于 Electron（[帖子](https://www.v2ex.com/t/1143902)）。
  - 云服务挂了就不能用（[Apifox](https://v2ex.com/t/1119645)、[Apipost](https://www.v2ex.com/t/841775)）。
  - 强制登录。
- 2026 年 3 月 Apifox 桌面客户端从 CDN 加载的统计脚本被劫持，窃取了 SSH 密钥、git 凭据、kubeconfig 等（[Wiz](https://threats.wiz.io/all-incidents/apifox-supply-chain-attack)、[腾讯云公告](https://cloud.tencent.com/announce/detail/2245)）。这对 CyberAPI“不加载远程代码”是很好的说明。

## 5. 候选功能逐项

每项写清：做什么、为什么、落在架构的哪里、工作量、要先定的事。凡是改项目里保存内容的（请求、目录、变量、新的请求类型），都要按 CLAUDE.md 的规矩把 `cyberapi_core::version::MODEL` 加一，并顾及 Store trait（`Call` / `dispatch` / `BridgeStore`）、同步、备份、导出、MCP；下面不再逐项重复。

### 5.1 测试

**A1 声明式断言（S–M）。**

- **做什么：** 接口上加“断言”标签，一行一条：状态码、响应头、JSONPath 取值（等于 / 包含 / 匹配 / 存在 / 类型）、耗时、响应体大小。发送后在响应面板显示通过与否，结果随响应存进历史。
- **为什么：** 不用写脚本就能测，Bruno、Thunder Client、Apidog 都有；它也是运行器和命令行的前提。
- **落点：**
  - 规则写在 core：新模块 `assert`，复用 `json_filter`（jsonpath-rust），两端行为一致。
  - 断言存进 `HttpRequest`，要给 MODEL 加一。
  - 结果放进 `HttpResponse`，属于历史，是读者自己的，不进同步。
  - MCP 可以加一个工具，让 AI 读断言结果。
- **要先定：** 断言能否写在目录上，让其中所有接口都检查（像 #66 的目录请求头那样继承）。

**A2 提取为变量（S）。**

- **做什么：** 断言旁边加“提取”：把某个 JSONPath 的值存成变量。
- **为什么：** 现有的 `{{g(接口, 路径)}}` 已经能在请求之间串联；提取主要是给运行器用，一次运行中后面的请求读前面的结果。
- **要先定：** 提取的值写到哪里。
  - 写进项目变量：会改项目、会同步，团队里互相覆盖，不建议。
  - 只在这次运行里有效：建议这样。
  - 写成读者自己的“运行时变量”。

**A3 运行器（M）。**

- **做什么：** 对目录或项目“全部运行”，按树的顺序依次发送，汇总通过 / 失败 / 耗时；可选 CSV / JSON 数据文件，每行跑一遍。
- **落点：**
  - 运行逻辑放在 `cyberapi-service`（`run`），两端共用。
  - 网页版逐个经 bridge 发送，受 `in_flight` 限制，只能串行或小并发。
- **为什么：** 8 个以上的竞品都有，是团队的第一需求。

**A4 `cyberapi run` 命令行（M）。**

- **做什么：** 无界面运行一个项目或目录，输出终端摘要和 JUnit XML，失败时退出码非零，可以放进 CI。
- **落点：** 新的 bin crate，复用 core / service / http，这在 Rust 里格外便宜。数据从哪来要定：
  - 导出文件（自有格式，最简单，可进 git）。
  - 本机库：只读打开要注意，turso 会把文件切到 WAL；不能和桌面版争锁，参见 #46。
  - bridge 上的项目：需要一种给机器用的令牌，现在的 MCP 令牌只认 `/v1/mcp`。
- **依赖：** A1，最好也有 A3。

**A5 前后置脚本（L，放到第三批，见第 7 节 Q1）。**

- **做什么：** JS 脚本，带 `pm.*` 兼容层，让 Postman 导入的集合继续能用。
- **落点：**
  - Boa 是纯 Rust、能编译到 wasm，两端可以同一个引擎。
  - 脚本在客户端跑：桌面本机；网页在页面里跑完再交给 bridge 发送。
  - MCP 的 `send_api` 在 bridge 上准备请求，那里跑脚本就要做多账号沙箱。
- **为什么放后面：** Yaak、HTTPie、Posting 3 没有脚本也过得很好；A1–A4 能覆盖大部分需求。

### 5.2 互通（导入导出）

都是 core 里的纯函数，能编译到 wasm，单项多为 **S–M**。它们的意义是降低换过来的成本，而换过来恰好是本地优先工具最大的机会：很多人在逃离 Postman / Insomnia / Apifox。

- **B1 导出 OpenAPI 3.x（M）：**
  - 环境变量前缀变成 `servers`，请求体变成示例，认证变成 `securitySchemes`。
  - 需求最强的一项，Postman #1744 有 397r。
- **B2 导出 Postman Collection v2.1（S–M）：** 让人能带着数据离开，这本身就是留住人的理由。
- **B3 从 URL 导入、重新导入时合并（S + M）：**
  - 从 URL 导入是 #59 推后的那一半：经 `HttpEngine` 取，网页受 `TargetPolicy` 限制。
  - 合并：按方法加路径对上已有接口，只更新变化的部分。要定的是，读者已经改过的接口是覆盖还是跳过。
- **B4 导入时带上认证和环境（S–M）：** [batch4-design.md](./batch4-design.md) 里写过“以后可以接着做”。
  - Postman / Insomnia 的 Bearer、API key，以及集合和目录级的认证，映射到 #66。
  - Postman 环境文件、Insomnia 的 base / sub 环境，映射到 #61 的命名环境。
  - OpenAPI 的 `securitySchemes` 映射到项目认证。
- **B5 新的导入格式：**
  - HAR（S）：浏览器开发者工具导出的请求。
  - Hoppscotch JSON（S）。
  - Bruno `.bru` 目录和 OpenCollection YAML（M）。
  - Yaak 导出（S–M）。
  - Paw / RapidAPI 文件（不确定能不能解析，见 Q9）。
- **B6 更多“复制为代码”目标（S 各）：** Rust reqwest、Java OkHttp / HttpClient、C# HttpClient、PHP Guzzle、Swift URLSession、Node axios。规则都在 `cyberapi_core::code`。

### 5.3 认证

- **C1 签名类认证（S 各）：** AWS SigV4、OAuth 1.0、Hawk、JWT 生成、Akamai EdgeGrid。
  - 落点：`cyberapi_core::auth` 的新分支，两端同一份。
  - 要注意签名时机：SigV4 要对最终的 Host、正文哈希和部分请求头签名，而引擎会自己补 `Host` / `Accept-Encoding` / `Content-Length`（见 `send::sent_request`），所以签名必须在这些头确定之后。
- **C2 Digest（S–M）：** 要先收到 401 才知道 nonce。在 `cyberapi_service::send` 里加一次重试，和跟随重定向的循环是同一种写法。
- **C3 NTLM（M，低优先）：** 握手绑在同一条连接上，而 `NativeEngine` 每个请求新建连接，要为它保留连接。企业内网才用，等有人要再说。
- **C4 OAuth 2.0（M–L）：**
  - client credentials 和 password 两种不需要浏览器（S–M）。
  - 授权码 + PKCE（M–L）：
    - 桌面：系统浏览器加本机回环监听（RFC 8252）。
    - 网页：回调落到 bridge 的一个路由，再回到页面。
  - 自动刷新令牌。
  - **令牌属于读者**：不同步、不进备份，像 Cookie 一样按账号存。
  - 和 #66 的继承配合：在项目或目录上配置一次。
- **C5 客户端证书与自定义 CA（桌面 S–M，网页 M）：**
  - 桌面：设置里按主机配证书和私钥文件，是读者自己的，像代理一样不同步。#60 时写过“mTLS 以后再说”。
  - 网页：证书要存在 bridge 上，按账号加密保存。安全上要多想一步，建议先只做桌面。

### 5.4 变量与秘密

- **D1 `prompt(说明)` 模板函数（S）：**
  - 发送时弹出输入框问值，和 `openFile()` 一样由调用方先问读者（`send::needs` 加一项）。
  - 两端都能用；MCP 和运行器里遇到它时报错，或者由参数给值。
- **D2 `env(NAME)` 读系统环境变量（S，仅桌面）：**
  - 走 `this_build` 的接缝。
  - **bridge 上绝不能有**：MCP 的 `send_api` 在服务器上准备请求，否则任何编辑者都能读出服务器的环境变量。
  - `.env` 文件可以作为这一项的延伸。
- **D3 秘密变量（M，要先定 Q3）：**
  - 变量加“秘密”标记：界面打码，Details 里打码（已有 `mcp::secret_name` 的规则可以并进来），MCP 永远不给值。
  - 难点是团队项目里秘密的**值**放在哪：
    - 每人本机：类似 Postman 的 Local Vault，同步只带名字、不带值。
    - 服务器加密保存。
    - 照旧共享：D8 说变量不区分个人值，要不要为秘密变量改。
  - 桌面可以放进系统钥匙串。
- **D4 历史（S–M）：**
  - 每个接口保留条数可设，现在固定 10 条（`HISTORY_LIMIT`）。
  - 项目级的“最近发送”列表。
  - 两条历史的对比（diff），只有 Kreya 有，是个差异点。

### 5.5 协议

- **E1 WebSocket（桌面 M，网页 M–L）：**
  - 新的请求类型，界面是连接 / 断开、消息编辑器、收发消息列表（可以复用 `event_stream.rs` 的列表）。
  - 桌面：`cyberapi-http` 在 hyper 的 upgrade 上接 tungstenite，同样按 DNS / TCP / TLS 计时。
  - 网页：浏览器 WebSocket 连到 bridge，bridge 再连目标，受 `TargetPolicy` 和每账号并发限制。这是 bridge 第一次需要**双向**中继，所以是工作量的大头。
  - 历史存连接的元数据和收发的消息。
- **E2 gRPC 客户端流和双向流（M，在 E1 之后）：** 网页需要同一个双向中继。现在的 `send_watched` 只能单向收。
- **E3 GraphQL 经 WebSocket 订阅（S，在 E1 之后）：** `graphql-transport-ws` 协议。
- **E4 Socket.IO、MQTT（M，低优先）：** 有人要，但不如上面三个；Socket.IO 可以在 E1 之上做。
- **不建议：** HTTP/3（只有 Kreya 有）、原始 TCP / UDP、SOAP。WSDL 导入可以在有人要时作为 B5 的一项。

### 5.6 工作方式

- **F1 标签页与临时请求（M–L，要先定 Q4）：**
  - 现在 `WorkspaceState` 只持有一个选中的 `ApiState`，标签页要改成多个，并在切换时保留各自的编辑状态和响应。
  - 临时请求是不属于任何项目、不自动保存的请求：粘贴 cURL 后先试，再决定存到哪。
  - 证据见第 4 节第 4 条，是多个竞品的第一名。
- **F2 命令面板（S–M）：** ⌘/Ctrl-K 跨项目找接口、切换环境、执行命令。#64 已经有项目内的 ⌘/Ctrl-F，可以在它上面扩展。
- **F3 接口说明与示例（M）：**
  - 接口、目录、项目加一段 Markdown 说明。
  - 可以把某次响应“存为示例”：示例跟着项目同步，历史不同步。
  - 用处有四个：给团队看、给 MCP 让 AI 理解接口、B1 导出 OpenAPI 时带上、以后的 Mock（G2）和文档（G3）直接用。
  - 要确认 gpui-component 0.8 的 Markdown 显示能力。
- **F4 现有 issue 的遗留（S 各）：**
  - **#51** 原始交换视图：像 `curl -v` 那样把状态行、响应头、原始响应体放在一起。现在有 JSON 原始视图和 Details，但没有完整的这一份。
  - **#43** 第 2 项：Query 批量编辑的占位文字还是 `Accept: application/json`，`locales/en.toml` 的 `editor.bulk_placeholder`，Query 和 Header 共用一个。
  - **#43** 第 5 项：发送新请求时，上一份响应没有变暗。
  - **#44**：最大化启动后点“还原”仍是全屏大小，窗口位置也没记住。要在当前版本上复测。
  - **#63** 的搜索按钮：等组件库修好查找栏，见 issue-plan。
  - 环境整理工具：把同名的几行变量分到读者指定的环境，#61 可选项。
  - #55–#73 已经做完但还开着，可以在你确认后逐个关闭。

### 5.7 团队与 bridge

- **G1 本地项目上传成服务器项目（M）：** [server-sync.md](./server-sync.md) §7 写着这一版不做。现在只能导出再导入，id、历史都断了。对“先自己用、再拉团队进来”的路径很重要。
- **G2 Mock 服务（M）：**
  - bridge 本来就是服务器，可以按 F3 的示例或 OpenAPI 在 `/v1/mock/<项目>/…` 回应。
  - 桌面要起一个本机 hyper 服务。
  - 国内用户尤其看重（Apifox 的卖点）。
- **G3 发布只读文档（M）：** bridge 给一个项目生成只读的 HTML 页面，依赖 F3 的说明。
- **G4 项目历史版本与回滚（M）：**
  - `TreeStore::replace` 本来就是整份替换，每次替换前把旧的 `ProjectConfig` 存一份，就能列出版本、对比、回滚。
  - 用户最恨的事之一就是数据丢失（第 8 节），这一项直接对着它。
- **G5 OIDC 单点登录（M）：** 自托管团队里唯一有真实票数的身份需求（Hoppscotch #3001）。SAML（L）、LDAP 和审计日志先不做，等有人要。
- **G6 Helm chart（S）：** 已经有多架构镜像，补一个 chart 门槛很低。
- **G7 定时监控（M，不确定）：** bridge 已经在服务器上发请求，按计划跑某个目录的断言、失败时通知，是 Postman、Apidog 收费的功能。依赖 A1 和 A3。

### 5.8 AI

- **H1 MCP 扩展（S–M）：**
  - 加写入工具：新建或修改接口、从 OpenAPI 片段建接口，受角色和令牌勾选限制，像现在的“允许发送”一样单独授权。
  - 加断言工具：配合 A1，让 AI 读断言结果。Hoppscotch 有 53 个工具，Yaak 能新建、修改、发送。
- **H2 桌面版本机 MCP（M，不确定）：**
  - 现在 MCP 只在 bridge 上，只用桌面的人用不到。
  - 可以在桌面开着时提供一个只监听本机、带令牌的 MCP 端点，直接读本机库。
  - 要考虑本机端口的安全，以及和 #46 的库锁问题。
- **H3 内置 AI（M，不确定，见 Q7）：** 自带密钥的“用自然语言写请求”“生成断言”，Bruno v4、Postman、Hoppscotch 都有。CyberAPI 已经能被外部 AI 通过 MCP 使用，内置是否值得要你定。

## 6. 建议的批次

| 批次 | 内容 | 理由 |
|---|---|---|
| 第 5 批（小而确定） | F4 遗留项；A1 断言；B3 从 URL 导入；B4 导入带认证和环境；C1 签名类认证；C2 Digest；D1 `prompt()`；D2 `env()`；C5 客户端证书（桌面） | 都是 S 到小 M，多数在 core，需求有证据，不需要大的设计决定 |
| 第 6 批（互通与测试） | B1 导出 OpenAPI；B2 导出 Postman；B5 HAR / Hoppscotch / Bruno 导入；A2 提取；A3 运行器；A4 命令行 | 让人愿意换过来，也让团队能把 CyberAPI 放进 CI |
| 第 7 批（体验与认证） | C4 OAuth 2.0；F1 标签页与临时请求；F2 命令面板；F3 说明与示例；D4 历史 | 需求强，但要先写设计（OAuth 回调、标签页的状态模型） |
| 第 8 批（协议） | E1 WebSocket；E2 gRPC 双向流；E3 GraphQL over WS | 共用 bridge 的双向中继，一起设计 |
| 第 9 批（团队） | G1 本地项目上传；G4 历史版本；G5 OIDC；G6 Helm；H1 MCP 写入 | 自托管团队的路径 |
| 待定 | A5 脚本；文件存储（见 Q2）；D3 秘密变量；G2 Mock；G3 文档；G7 监控；H2 / H3 | 方向性，先回答第 7 节 |

每一批照 issue-plan 的做法：先建 issue，写设计（改数据模型的写进单独的设计文档），一项一个提交，做完更新文档。

## 7. 需要你拍板的问题（不确定的都在这里）

- **Q1 要不要脚本？**
  - 方案一：只做声明式断言 + 提取 + 模板函数（Yaak 的路线），简单、安全、两端一致。
  - 方案二：再加 JS 脚本（Boa，带 `pm.*` 兼容），Postman 用户迁移更顺，但沙箱、bridge 上的执行、调试界面都是成本。

  我倾向先做方案一，看用户反馈再决定方案二。
- **Q2 要不要文件存储（项目存成一个目录的 YAML，可进 git）？** 这是 2026 年最强的趋势，Postman、Insomnia、Bruno、Yaak 都做了。
  - 做法上，Store trait 的接缝允许加一个新后端，跑同一套一致性测试。
  - 难点是它和服务器同步的关系：一个项目要么是文件，要么是服务器副本？团队已经用 git 协作时，还需要 bridge 吗？
  - 一个较轻的折中：“导出为目录”加“从目录导入”，配合 B3 的合并，先不做实时双向。
- **Q3 秘密变量的值放在哪？** 团队项目里，选项是每人本机、服务器加密、照旧共享。这会改 D8（变量不区分个人值）。
- **Q4 标签页之后，保存方式改不改？** 现在是自动保存。临时请求要不要默认不保存？要不要提供“手动保存”的设置？两边都有很多人要。
- **Q5 网页版要承担多少？** WebSocket / gRPC 双向流的中继、OAuth 回调、客户端证书，都要 bridge 多做事，也多了攻击面。可以选择其中一些只在桌面版提供，就像 `.proto` 导入和代理那样。
- **Q6 MCP 可以改项目吗？** 写入工具很有用，但 AI 改错了怎么办。如果先做 G4 历史版本，就有了回滚的底气。
- **Q7 要不要内置 AI？** 还是只靠 MCP 让外部 AI 来用？内置意味着要管密钥、费用和隐私说明。
- **Q8 要不要更多界面语言？** 旧版有乌克兰语，重写版只有中英。Bruno 排第一的 issue 就是多语言。成本主要是翻译的维护：每加一个 key，每个 locale 都要加，测试会逐个比对。
- **Q9 要不要把“原生的 Mac 客户端”作为卖点，接住 RapidAPI for Mac（Paw）的用户？**
  - 它的官方通知说 macOS 27 不再测试和支持，新闻标题说“立即放弃”，后者我只看到标题。
  - 如果要接住，就值得做 Paw 文件导入，但它的格式能不能解析还没查。
- **Q10 插件系统？** Insomnia、Yaak、Bruno 都有。对 CyberAPI，可能的形式是 wasm 插件（认证、导入器、模板函数），不加载远程代码。现在没有足够需求证据，建议不做。

## 8. 要守住的（用户最恨的事）

1. **强制登录，或离开厂商服务器就不能用。** Insomnia #6577、Postman 宕机、Apifox / Apipost 宕机都是例子。CyberAPI 桌面版不需要账号，要一直这样。
2. **升级或迁移丢数据。** 例如 Insomnia [#6579](https://github.com/Kong/insomnia/issues/6579)、Postman 移除 Scratch Pad。CyberAPI 复制旧数据、不改原文件，这个做法要保持；G4 历史版本能再加一层保险。
3. **关不掉的遥测。** 例如 Bruno [#337](https://github.com/usebruno/bruno/issues/337)。CyberAPI 除了检查更新（GitHub 上的 `latest.json`）不访问任何地址，也**不加载任何远程代码**（Apifox 的教训）。将来加 AI 或插件时也要守住。
4. **大项目慢、占内存。** 例如 Postman [#7294](https://github.com/postmanlabs/postman-app-support/issues/7294)、V2EX 上千接口就卡。GPUI 原生是最大的卖点；新功能，尤其标签页和运行器，不能把它拖慢。
5. **功能膨胀、许可或价格突变。** 例如 Thunder Client 加限制、Bruno 改收费。Apache-2.0 加自托管要保持清楚。
6. **强制或坏掉的自动更新。** 例如 Bruno [#7856](https://github.com/usebruno/bruno/issues/7856) 自己把自己卸载了。CyberAPI 的更新只在读者点击后下载并核对校验和，macOS 原地替换前会核对 bundle 标识；这一条要一直测。

## 9. 实施时先替你定下的（2026-10-26 起，都可以改）

维护者要求“需要你决定的先写进文档，尽可能多做”。下面每一条是动手前写下的：用的是哪种做法、为什么，以及要改的话改哪里。没写在这里的，按第 5 节的建议做。

- **B4 导入带认证和环境：** 照 issue-plan 里已经定的：项目已有认证时保留项目的；同名环境只补上没有的变量。OpenAPI 的 `securitySchemes` 只建结构，值留空。
- **B1 导出 OpenAPI：** 导出 3.0.3（比 3.1 被更多工具接受），JSON 或 YAML 由保存的扩展名决定。`servers` 取当前环境里地址前缀变量的值；请求体的示例取接口自己的请求体；认证只写类型，不写值（不把密钥带出去）。
- **B2 导出 Postman：** Collection v2.1；认证的值、变量的值照导出（这是读者自己的数据，导出给自己用），和现有的 CyberAPI 导出一致。
- **B5 新导入格式：** 先做 HAR、Hoppscotch、Bruno 单个 `.bru` 文件和 OpenCollection YAML。Bruno 整个目录要选文件夹，先不做；Paw 文件格式没查清，先不做。
- **B6 复制为代码：** 加 Rust reqwest、Node axios、Java HttpClient、C# HttpClient、PHP Guzzle、Swift URLSession。目标多了以后菜单分组，cURL 仍在第一个。
- **D4 历史：** 每个接口保留的条数可在设置里改（5–100，默认仍是 10），桌面和网页都是读者自己的设置；项目级“最近发送”列出最近 50 条；两条历史的对比先做响应体的文本对比。
- **A2 提取：** 只在一次运行里有效，不写回项目、不同步（第 5.1 节的建议）。
- **A3 运行器：** 串行发送；数据文件支持 CSV 和 JSON 数组，每行跑一遍；运行中发出的请求照常记进历史（每个接口仍只留最近的几条）；运行结果只显示、不存。
- **A4 `cyberapi run`：** 只读 CyberAPI 的导出文件（可进 git），不打开本机库（不和桌面版争锁），也不连 bridge（以后要的话再加机器令牌）。环境用 `--env` 选，输出终端摘要和 JUnit XML。
  - 实现时的补充：CyberAPI 自己的导出只有接口树（和旧版互通的记录数组），没有变量和环境，加进去旧版会读坏；所以命令行读任何能导入的文件（自己的导出保留 id），变量和环境由文件自带、`--env-file`（如 Postman 环境文件）和 `--var` 补。程序名是 `cyberapi-cli`（和桌面版的 `cyberapi` 不同名，cargo 不允许同名的两个 bin）。**待你定：** 要不要把 `cyberapi-cli` 加进 Release 的安装包（各平台一个二进制，要改 `package.yml` 和 `checksums.sh` 的 `describe`）；现在只能从源码构建。
- **F3 说明与示例：** 说明是纯文本（Markdown 原样显示，不渲染，组件库的渲染能力还没查），示例跟着项目同步，数据模型编号加一。
- **H1 MCP 写入（第 7 节 Q6）：** 令牌签发时单独勾选“允许修改”，账号对项目至少是编辑者；先只做新建和修改接口，不做删除。
- **C4 OAuth 2.0：** 令牌属于读者，像 Cookie 一样按账号存，不同步、不进备份。先做 client credentials 和 password 两种，不需要浏览器；授权码 + PKCE 之后再说。
  - 实现时的补充：令牌先只放在内存里（桌面一个应用一份、网页一个标签页一份、命令行一次运行一份、MCP 一次调用一份），没有像 Cookie 那样写进库：这两种授权方式随时能重新申请，重启后第一次发送多一次令牌请求；写进库就要多存一种密钥，网页版还会存到 bridge 上。缓存按渲染后的全部字段区分（含密钥，改了即重新申请）；过期前 30 秒或接口回 401 时重新申请，有刷新令牌时先用它。令牌请求在网页版同样经 bridge 发出、受“请求能发往哪里”限制。**待你定：** 要不要像 Cookie 一样持久保存（需要新的 store 和迁移）；要不要加 Auth0 等要求的 `audience` 这类附加参数。
- **F1 标签页（第 7 节 Q4）：** 保存方式不变，仍是自动保存；临时请求先不做。
  - 实现时的补充：单击树里的接口就开一个标签（不做“预览标签”），放在当前标签之后；最多 16 个，超出时关最久没显示的；标签列表按读者存（和展开的目录一样，进 prefs，网页版存在服务器上）。⌘W 在工作区里改为关闭当前标签（没有标签时才关窗口），和浏览器、Postman 一致。标签栏只在请求编辑列的顶部，不跨到响应列（三栏的可拖动宽度不用改）。置顶照旧，是常用接口的快捷入口。**待你定：** 要不要“预览标签”（单击替换同一个标签，编辑或双击后才固定）。
- **G4 项目历史版本：** 每次整份替换（同步）之前把旧的存一份，每个项目留最近 50 份；先只在 bridge 上做，桌面本机项目不做。

## 附录 A：竞品功能的主要来源

- 协议：[Postman protocols](https://learning.postman.com/docs/use/send-requests/protocols/protocols.md)、[Insomnia gRPC](https://developer.konghq.com/insomnia/grpc-requests/)、[Bruno gRPC streams](https://docs.usebruno.com/send-requests/grpc/grpc-streams.md)、[Yaak gRPC](https://yaak.app/docs/request-types/grpc-requests)、[Hoppscotch](https://docs.hoppscotch.io/llms.txt)、[Apidog](https://docs.apidog.com/llms.txt)、[Requestly](https://docs.requestly.com/llms.txt)、[Kreya](https://kreya.app/)。
- 测试：[Bruno 断言](https://docs.usebruno.com/testing/tests/assertions.md)、[Bruno 脚本沙箱](https://docs.usebruno.com/get-started/configure/javascript-sandbox.md)、[Hoppscotch 脚本](https://docs.hoppscotch.io/documentation/features/scripts.md)、[Hoppscotch CLI](https://docs.hoppscotch.io/documentation/clients/cli/overview.md)、[Kreya 快照测试](https://kreya.app/docs/scripting-and-tests/tests/snapshot-testing/)。
- 认证：[Postman OAuth 2.0](https://learning.postman.com/docs/use/send-requests/authorization/oauth-20.md)、[Yaak 客户端证书](https://yaak.app/docs/authentication/client-certificates)。
- 存储与秘密：[Postman Native Git](https://learning.postman.com/docs/use/native-git/overview.md)、[Insomnia 12.6](https://konghq.com/blog/product-releases/insomnia-12-6)、[Postman Vault](https://learning.postman.com/docs/use/postman-vault/postman-vault-secrets.md)、[Yaak 加密](https://yaak.app/docs/collaboration/secrets-encryption)、[Insomnia 外部密钥库](https://developer.konghq.com/insomnia/external-vault/)、[Postman 协作](https://learning.postman.com/docs/use/use-collections/collaborate-with-collections)。
- Mock、MCP：[Bruno Mock](https://docs.usebruno.com/mock-servers/overview.md)、[Hoppscotch MCP 工具](https://docs.hoppscotch.io/documentation/clients/mcp-server/tools.md)、[Yaak MCP](https://yaak.app/docs/getting-started/mcp-server)。

## 附录 B：没能确认的

- **Reddit：** 打不开，相关结论来自 HN、V2EX 和引用它的博客。
- **Yaak 的投票数：** 来自它的反馈页，是大约值；它在 GitHub 上关了 issue。
- **Insomnia 的讨论：** 由 issue 转成，reaction 和投票的计法不同。
- **Socket.IO：** Insomnia 支持 Socket.IO 只见于第三方目录，它的官方文档只列了 HTTP、SSE、GraphQL、gRPC、WebSocket。
- **OAuth 2.0 设备码授权：** 没找到任何客户端写明支持，但这不能证明没有。
- **GraphQL 订阅的传输方式：** Postman 和 Hoppscotch 没写清楚。
- **RapidAPI for Mac 的“立即放弃”：** 只看到新闻标题。官方通知只说 macOS 27 不再支持。
- **Bruno 官方 MCP 服务器：** 只见于第三方 MCP 目录。
- **国内工具的需求：** Mock、数据库操作、代码生成这些卖点多来自厂商或其关联文章，不是测量出的需求。
- **G4、H2 的成本：** “存旧版本”“本机 MCP 端点”是我按代码结构估的，没有原型。
- **F3 的 Markdown 显示：** 组件库对 Markdown 的支持程度没查。
