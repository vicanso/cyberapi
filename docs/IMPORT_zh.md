中文 | [English](./IMPORT.md)

# 导入与导出

应用能读入的格式、再次导入时保留什么，以及导出写成什么。记住地址、以后再同步的项目或目录见[从来源同步](./SOURCE_zh.md)。返回[功能说明](./FEATURES_zh.md)。

## 格式

可以粘贴、选文件，或填地址取回（网页版经服务器取，地址要在“请求能发往哪里”的范围内）。从接口树顶栏的 **+** 导入到当前项目，或在首页导入为新项目。

| 来源 | 读进来的内容 |
| --- | --- |
| CyberAPI | 应用自己的导出。 |
| Postman | 集合。保存的响应成为示例。环境文件成为命名环境。条目上的变量也会进来。常见的 `test` 脚本转为断言和提取。 |
| Insomnia | 请求、目录认证、子环境。常见的 `afterResponseScript` 断言和变量提取也会转换。 |
| Hoppscotch | 单个集合、“全部导出”，或环境。 |
| Thunder Client | 集合（目录、`baseUrl`、认证、表单/GraphQL/二进制请求体、声明式测试与 `set-env-var` 提取）或环境导出。脚本与不支持的规则记入导入报告。 |
| Yaak | 导出 JSON（工作区、目录、HTTP / GraphQL / WebSocket / gRPC 请求、共享与命名环境、认证）。常见的 `${[ var ]}`、`${[ uuid.v4() ]}`、`${[ timestamp.unix() ]}` 与 `${[ timestamp.iso8601() ]}` 自动转换；其余 `${[ … ]}` 标签及不支持的认证与请求体记入导入报告。 |
| Apidog | 项目导出 JSON（`apidogProject` / `apifoxProject`：目录、环境与 Base URL、参数说明及 Schema 元信息、请求与响应 Schema、响应示例、认证、声明式后置断言/提取，以及常见 `customScript` 脚本静态转换）。未转换的脚本或规则记入导入报告。 |
| Bruno | `.bru` 请求或环境，或 OpenCollection 的 YAML 导出。`{{process.env.NAME}}` 变成 `{{env(NAME)}}`。常见的 `tests`、`script:post-response`、`vars:post-response` 和 `assert` 块转为断言和提取。 |
| Swagger 2 / OpenAPI 3 | JSON 或 YAML。安全方案成为项目认证，值由你填。操作自己的方案写在那个接口上。第一个 2xx 的 JSON 响应 Schema 成为一条来自导入的 Schema 检查。 |
| HAR | 浏览器开发者工具里的接口调用。页面加载的图片、脚本和样式不算。每个主机一个目录。 |
| `.http` | VS Code REST Client 和 JetBrains HTTP Client（*0.4.1*）。`###` 分隔请求。`@名称 = 值` 成为各环境共用的变量。`http-client.env.json` 或 `rest-client.environmentVariables` 成为命名环境。`{{$uuid}}`、`{{$timestamp}}`、`{{$randomInt 1 10}}`、`{{$processEnv NAME}}` 等换成本应用的模板函数。`# @prompt` 在发送时询问。响应处理脚本不导入。私有文件 `http-client.private.env.json` 的值以明文导入，不要放进团队共用的项目。 |
| cURL | 粘贴一条命令即成为一个接口。 |

对得上的认证会一起进来：Basic、Bearer、JWT Bearer、API Key、Digest、OAuth 2.0 的客户端凭据、密码、授权码（*0.3.1*）和隐式授权，以及 AWS SigV4（Postman、Insomnia、Bruno、Thunder Client、Yaak、Apidog）。没有对应种类的按继承处理。

项目已经有认证时，保留项目的。同名环境（不分大小写）只补上还没有的变量。二进制 / 文件请求体（Postman `mode: file`、Insomnia、HAR、Bruno、Thunder Client、Yaak、Apidog、OpenAPI 以及 cURL 的 `-T` / `--data-binary @`）导入为二进制请求体。

只有一行地址的文本不当作 `.http` 文件，用“获取”。

Postman、Bruno（`.bru` 与 OpenCollection）、Insomnia 和 Apidog `customScript` 里常见的测试与后置脚本（状态码、响应头、JSONPath、响应体文本与耗时检查，以及从响应 JSON、请求头、状态或正文提取变量）会不执行 JS 地按模式转为原生断言与提取；转不了的语句（含前置脚本）会以代码块追加到接口说明，并在下方列出。

导入或从来源同步完成后，如果有没能原样带进来或转不了的内容（未转换的脚本、不认识的模板表达式、不支持的认证类型或请求体），会弹出**导入报告**按位置列出（脚本可展开并复制原文）；全部原样导入时则不显示。

## 怎样算同一个接口

导入到已有项目时，会数出项目里已经有的接口。方法和路径相同的算同一个。

- 比较路径前会去掉开头的一个 `{{变量}}`、协议和主机、查询参数、片段，以及末尾的斜杠。
- `{id}`、`:id`、`{{id}}` 是同一种占位符。路径其余部分区分大小写。
- gRPC 不按这条匹配。
- 两边都写了字面主机、而且主机不同，就不是同一个接口。主机还是变量时仍可以匹配。
- 项目里已经有的接口不会再添加一份。文件里有多条路径相同的，第一条真正改动这个接口的会被用上。

## 更新已有的

再次导入时，默认更新对得上的接口。也可以改成只添加新的。

更新时，地址、参数、路径参数和请求体字段跟随导入的内容：新的进来，导入里没有的去掉。你已经设置的留下：

- 参数的值，以及这个参数开着还是关着。
- 路径参数、Query 和请求头的类型、是否必填、format 和枚举。跟导入的文档走，空了就清掉（*0.4.1*）。
- 导入没有写的说明，参数上和接口自己的都留下。导入写了的说明换上导入的。
- JSON 请求体里、字段种类没变的值。`{{模板}}` 可以占据任何值的位置。你写的整个数组保持原样。带注释、或用模板代替值、读不成纯 JSON 的请求体保持原样（*0.4.1*）。同类型的非 JSON 请求体也留下。空的请求体，或内容类型不同的请求体，用导入的。
- 路径参数的值。你的其他变量都留下，包括导入里没有的。
- 整份认证、你写的断言、示例、提取，以及接口自己的设置。
- 来自导入的 Schema 检查会被这次导入的换掉。你已经自己写了 Schema 检查时，以你的为准，导入的那条不加进来。
- 请求 Schema：导入有就用导入的，没有就留你的。
- 你给接口起的名字留下。

新接口放进同名目录。已有的同名环境变量不再添加。正开着的接口会重建编辑器。更新后没有变化的接口不写入。

## 只添加新的

文件不应当改动项目里已有的接口时选这个。对得上的接口保持原样，其余的加进去，放进同名目录。

## 导出

接口、目录或项目可以导出为本应用自己的文件。

目录和项目还可以导出为 Postman 集合（v2.1）、OpenAPI 3（YAML）、HAR 1.2（`.har`）、HTTP 文件（`.http`）或单文件离线 HTML 文档（`.html`）：

- OpenAPI：地址前面的变量成为 `servers`；请求体写成示例并带上从示例推断出的 schema；认证只写种类不写值（OAuth 2.0 写授权方式、令牌地址和范围，授权码另写授权地址和本机回调，*0.3.1*）；gRPC 和 WebSocket 调用不写进 OpenAPI 文档。
- HAR 1.2（`.har`）：尝试用当前环境的非秘密变量展开地址前缀，合并全局请求头、目录请求头和静态认证（Basic、Bearer、API Key），有示例时附带第一条响应，不导出秘密变量值，跳过 gRPC 和 WebSocket 调用。
- HTTP 文件（`.http`）：将共用的非秘密变量写为顶部的 `@name = value`，合并全局请求头、目录请求头和静态认证，保留接口说明、`# @no-redirect`、HTTP/2、GraphQL 和 WebSocket 请求，跳过 gRPC 调用；绝不写出秘密变量的值。

## 复制为代码

任何请求都可以复制为代码。菜单按命令行、脚本语言、编译型语言分组，cURL 在第一个。

cURL、HTTPie、JavaScript `fetch`、Node.js axios、Python `requests`、PHP Guzzle、Go `net/http`、Rust reqwest、Java `HttpClient`、C# `HttpClient`、Swift `URLSession`。

跟随重定向、是否校验证书照设置写出。库能自己应答 Digest 时，写成它的写法。OAuth 2.0 写当前的令牌；还没有令牌时写一句从哪里取。gRPC 一律是 grpcurl。WebSocket 一律是 websocat。
