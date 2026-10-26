中文 | [English](./DEPLOY.md)

# 部署网页版

网页版只有一个程序：`cyberapi-bridge`。它在同一个地址上提供页面（编译成 WebAssembly 的应用，内置在程序里）和页面用的 API；账号、团队、项目、响应历史、Cookie 和设置都存在 PostgreSQL 或 MySQL 里；页面准备好的请求也由它发出。它自己的磁盘上不存任何东西。

本文跟随 `main`。正式版还没有的内容标注了它从哪个版本开始提供（*0.2.2*），在那之前只有 `nightly` 构建里有；某个正式版自己的部署说明在它的 tag 下：`https://github.com/vicanso/cyberapi/blob/v<版本>/docs/DEPLOY_zh.md`。

- [需要什么](#需要什么)
- [先试一下](#先试一下)
- [Docker Compose](#docker-compose)
- [二进制加 systemd](#二进制加-systemd)
- [Kubernetes（Helm）](#kuberneteshelm)
- [选项](#选项)
- [第一个管理员](#第一个管理员)
- [反向代理和 HTTPS](#反向代理和-https)
- [请求能发往哪里](#请求能发往哪里)
- [账号、团队和邀请](#账号团队和邀请)
- [桌面版登录](#桌面版登录)
- [AI 助手（MCP）](#ai-助手mcp)
- [升级](#升级)
- [备份](#备份)

## 需要什么

- **数据库**：PostgreSQL 或 MySQL（CI 测试的是 PostgreSQL 18 和 MySQL 8.4）。给 bridge 一个空库和一个能在里面建表的用户：首次启动时它会建好表，之后每次启动都会把表结构升级到当前版本。
- **程序**：镜像 [`vicanso/cyberapi-web`](https://hub.docker.com/r/vicanso/cyberapi-web)，有 linux/amd64 和 linux/arm64（tag：各个版本号如 `0.3.0`、`latest`，以及 `main` 的 `nightly`）；或者 [Releases](https://github.com/vicanso/cyberapi/releases) 里的 `cyberapi-web-<版本>-x86_64-unknown-linux-gnu.tar.gz`（需要 glibc 2.39 或更新：Ubuntu 24.04、Debian 13）。
- **HTTPS**：除了在自己机器上试用，前面都要有 HTTPS。登录 Cookie 带 `Secure`，浏览器只在 HTTPS 下发送它。

## 先试一下

在自己的机器上，连一个已有的 PostgreSQL：

```bash
docker run --rm -p 127.0.0.1:7380:7380 \
  -e CYBERAPI_BRIDGE_STORE=postgres://user:password@host.docker.internal:5432/cyberapi \
  vicanso/cyberapi-web --insecure-cookie
```

`host.docker.internal` 是 Docker Desktop 里宿主机的名字；Linux 上要加 `--add-host=host.docker.internal:host-gateway`。打开 http://127.0.0.1:7380/，用日志里的 setup token 创建第一个管理员（[见下文](#第一个管理员)）。镜像名后面的参数会传给 bridge；`--insecure-cookie` 让纯 HTTP 下也能登录，只用于试用。

## Docker Compose

bridge 和一个 PostgreSQL 放在一起，只发布到本机回环地址，交给同一台机器上的反向代理：

```yaml
# compose.yaml
services:
  db:
    image: postgres:18
    environment:
      POSTGRES_USER: cyberapi
      POSTGRES_PASSWORD: change-me
      POSTGRES_DB: cyberapi
    volumes:
      - db:/var/lib/postgresql
    healthcheck:
      test: ["CMD", "pg_isready", "-U", "cyberapi"]
      interval: 5s
      retries: 20
    restart: unless-stopped

  cyberapi:
    image: vicanso/cyberapi-web:0.3.0
    environment:
      CYBERAPI_BRIDGE_STORE: postgres://cyberapi:change-me@db:5432/cyberapi
    ports:
      - "127.0.0.1:7380:7380"
    depends_on:
      db:
        condition: service_healthy
    restart: unless-stopped

volumes:
  db:
```

```bash
docker compose up -d
docker compose logs cyberapi | grep setup_token
```

像上面这样固定一个版本号，不要用 `latest`，这样升级是你决定的事（见[升级](#升级)）。用 MySQL 时地址是 `mysql://user:password@host:3306/cyberapi`。

镜像基于 distroless（没有 shell），以非 root 用户运行，在容器里监听 `0.0.0.0:7380`；`docker stop` 时会正常退出，正在处理的请求先完成。

## 二进制加 systemd

```bash
tar -xzf cyberapi-web-0.3.0-x86_64-unknown-linux-gnu.tar.gz
sudo install -m 755 cyberapi-web-0.3.0-x86_64-unknown-linux-gnu/cyberapi-bridge /usr/local/bin/
```

数据库地址放在只有 root 能读的文件里，不要写在命令行上：能列出进程的人都能看到命令行。

```bash
# /etc/cyberapi/bridge.env（chmod 600）
CYBERAPI_BRIDGE_STORE=postgres://cyberapi:change-me@127.0.0.1:5432/cyberapi
```

```ini
# /etc/systemd/system/cyberapi-bridge.service
[Unit]
Description=Cyber API web version
After=network-online.target
Wants=network-online.target

[Service]
EnvironmentFile=/etc/cyberapi/bridge.env
Environment=CYBERAPI_BRIDGE_LISTEN=127.0.0.1:7380
ExecStart=/usr/local/bin/cyberapi-bridge
DynamicUser=yes
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now cyberapi-bridge
journalctl -u cyberapi-bridge | grep setup_token
```

## Kubernetes（Helm）

*0.3.1*。仓库里有一个 chart：`deploy/helm/cyberapi`，包括镜像的一个 Deployment、一个 Service，需要的话还有一个 Ingress。它不创建数据库，请用你已有的。

```bash
git clone https://github.com/vicanso/cyberapi && cd cyberapi
kubectl create secret generic cyberapi-db \
  --from-literal=store-url='postgres://cyberapi:change-me@postgres:5432/cyberapi'
helm install cyberapi deploy/helm/cyberapi \
  --set store.existingSecret=cyberapi-db \
  --set ingress.enabled=true --set 'ingress.hosts[0].host=cyberapi.example.com' \
  --set 'ingress.hosts[0].paths[0].path=/' --set 'ingress.hosts[0].paths[0].pathType=Prefix'
```

- **数据库**：`store.existingSecret`（及 `store.existingSecretKey`，默认 `store-url`）指向你自己的 Secret；或者用 `store.url`，由 chart 写进它生成的 Secret。两者都没有时拒绝安装。
- **第一个管理员**：`setupToken.value` 或 `setupToken.existingSecret`；都没有时初始化 token 在日志里（`kubectl logs deploy/cyberapi | grep -i "setup token"`）。
- **上面的选项**都有对应的 value：`bridge.basePath`（Ingress 的路径要和它一致）、`bridge.allowTargets`（列表）、`bridge.publicOnly`、`bridge.insecureCookie`，其余用 `bridge.extraArgs` / `bridge.extraEnv`。
- **一个副本**（`replicaCount: 1`）：登录限流、MCP 调用限制和每个账号同时发送的请求数都是在各自进程里计数的。Pod 以非 root 用户、只读根文件系统运行，连上数据库后才算就绪。
- **TLS** 交给 Ingress（`ingress.tls`），这样登录 Cookie 保持 `Secure`。要用 chart 之外的版本就设 `image.tag`（`nightly` 跟随 `main`）。

## 选项

每个选项都是一个命令行参数，表中列出的也可以用环境变量设置。

| 参数 | 环境变量 | |
|---|---|---|
| `--store <url>` | `CYBERAPI_BRIDGE_STORE` | **必填。** `postgres://…` 或 `mysql://…`。建议用环境变量：它不会出现在进程列表里，bridge 的错误信息里也不会带出密码。 |
| `--listen <addr>` | `CYBERAPI_BRIDGE_LISTEN` | 监听地址，默认 `127.0.0.1:7380`；镜像里设为 `0.0.0.0:7380`。 |
| `--setup-token <token>` | `CYBERAPI_BRIDGE_SETUP_TOKEN` | 创建第一个管理员用的 token。不设置时随机生成并写进日志。 |
| `--allow-target <网段>` | `CYBERAPI_BRIDGE_ALLOW_TARGETS` | 请求还可以发往的网段，逗号分隔，如 `127.0.0.1/32`（见[请求能发往哪里](#请求能发往哪里)）。 |
| `--public-only` | `CYBERAPI_BRIDGE_PUBLIC_ONLY` | *0.2.2*。连内网也拒绝，用于开放到公网的 bridge。 |
| `--base-path <路径>` | `CYBERAPI_BRIDGE_BASE_PATH` | *0.2.2*。把页面和接口放在这个路径下，如 `/cyberapi`，登录 Cookie 也只在这个路径下有效（见[子路径](#反向代理和-https)）。默认为空，即根路径。 |
| `--insecure-cookie` | | 登录 Cookie 不带 `Secure`，用于在自己机器上走纯 HTTP。 |
| `--www <dir>` | | 用这个目录作为页面，代替内置的页面（开发用）。 |
| | `RUST_LOG` | 日志详细程度：默认 `info`，`debug` 更详细。日志写到标准输出。 |

`cyberapi-bridge --help` 也会列出这些选项。

## 第一个管理员

数据库里还没有任何账号时，bridge 每次启动都会在日志里打印 setup token：

```
INFO cyberapi_bridge: no account yet: make the first administrator with this setup token setup_token=…
```

打开页面，填写 token、用户名、显示名和密码，这个账号就是系统管理员。此后 token 不再起作用，新账号只能通过邀请创建。没有用 `CYBERAPI_BRIDGE_SETUP_TOKEN` 指定时，token 每次启动都会重新生成；输错 token 会被限速。

## 反向代理和 HTTPS

由代理负责 TLS，把所有请求转给 bridge。有两个设置要注意：请求体最大 64 MB（bridge 自己的上限），以及读超时要大于应用默认的请求超时 300 秒，免得慢接口的响应先被代理截断。

```nginx
server {
    listen 443 ssl;
    server_name cyberapi.example.com;
    ssl_certificate     /etc/ssl/cyberapi.example.com.pem;
    ssl_certificate_key /etc/ssl/cyberapi.example.com.key;

    client_max_body_size 64m;

    location / {
        proxy_pass http://127.0.0.1:7380;
        proxy_set_header Host $host;
        proxy_read_timeout 600s;
        proxy_send_timeout 600s;
    }
}
```

**事件流**（*0.2.2*）。响应是事件流（`text/event-stream`）的接口，由 `POST /v1/send/stream` 边收边转给页面。bridge 给这个响应加了 `X-Accel-Buffering: no`，nginx 会不缓冲地转发；其他代理也不能缓冲这个路径的响应（Caddy 和 Traefik 默认就是流式转发）。流在读者停止、超过请求的读取超时没有新数据、或达到 64 MB 时结束；持续期间占用该账号同时进行的 16 个请求之一，代理的读超时作用于两段数据之间的间隔。

**放在子路径下**（*0.2.2*）。要和其他应用共用一个域名、放在 `https://example.com/cyberapi/` 下，用 `--base-path /cyberapi`（或 `CYBERAPI_BRIDGE_BASE_PATH=/cyberapi`）启动 bridge，代理把路径原样转发：

```nginx
    location /cyberapi/ {
        proxy_pass http://127.0.0.1:7380;
        proxy_set_header Host $host;
        proxy_read_timeout 600s;
        proxy_send_timeout 600s;
    }
```

这样 bridge 只在 `/cyberapi/` 下响应，访问 `/cyberapi` 会跳到 `/cyberapi/`，登录 Cookie 也只在 `/cyberapi/` 下有效，同一域名下的其他应用收不到它。也可以不设前缀、由代理去掉前缀（`proxy_pass http://127.0.0.1:7380/;`、Caddy 的 `handle_path`、Traefik 的 `StripPrefix`），页面的地址都是相对于自身的，同样能用；只是这时 Cookie 对整个域名有效。

**登录失败按用户名（15 分钟 5 次）和客户端地址（15 分钟 30 次）分别计数。** 在代理后面，bridge 只看得到代理的地址，所以按地址的计数是所有人合在一起的：任何人累计失败 30 次，所有人都要等这段时间过去才能登录。请在代理上也按真实的客户端地址限制 `/v1/login`，例如用 nginx 的 `limit_req`。

## 请求能发往哪里

页面的请求由 bridge 在它所在的位置代发。默认（*0.2.2*；0.2.1 及以前只能访问公网）**公网和内网都能访问**（`10.0.0.0/8`、`172.16.0.0/12`、`192.168.0.0/16`、`100.64.0.0/10`、IPv6 `fc00::/7`），因为团队要测的接口大多在自己的内网里。只拒绝那些不会是被测接口、而代人发请求时又最容易出事的地址：**bridge 所在的本机**（回环 `127.0.0.0/8`、`::1`，那里的服务往往默认信任本机来的请求），以及**链路本地**（`169.254.0.0/16`、`fe80::/10`，其中包括云服务器的元数据地址 `169.254.169.254`，能拿到这台机器的云账号凭证），另外还有未指定、组播、广播和保留地址。检查的是域名解析后的地址，每次重定向都会再查一次。被拒绝时，报错会写出域名和它解析到的地址。

也就是说，能登录网页版的人，能访问 bridge 这台机器能访问的地方。请把它部署在要测的接口所在的网段，不要放进权限更大的网段。

- 要访问 bridge 本机上的服务，就放行它：`CYBERAPI_BRIDGE_ALLOW_TARGETS=127.0.0.1/32`。在容器里，`127.0.0.1` 是容器自己；宿主机上的接口要用宿主机的地址访问。
- 开放到公网的 bridge 可以用 `--public-only`（`CYBERAPI_BRIDGE_PUBLIC_ONLY=1`）连内网也一并拒绝，再用 `--allow-target` 放行你指定的网段：`CYBERAPI_BRIDGE_ALLOW_TARGETS=10.1.0.0/16,192.168.10.0/24`。

放行写的是地址（CIDR），不是域名：检查的是域名解析后的地址，只放行一个域名的话，它可以解析到任何地方。每个账号同时最多有 16 个请求在发送，再多的会立即回复“忙”，不会排队。

bridge 总是自己连接每台服务器，不经过代理：既不读它所在环境的 `HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY`，也不用桌面版的代理设置（页面的请求不带这项设置）。经过代理的话，上面的地址检查就只查到代理本身。

桌面版即使操作的是服务器项目，请求也是从它所在的电脑直接发出，以上限制对它不适用。

## 账号、团队和邀请

- **账号只能通过邀请创建。** 在页面的设置菜单（齿轮）里，系统管理员打开**账号管理…**，可以生成邀请链接（新账号只有自己的个人空间）、重置密码、停用账号。团队的所有者或管理员打开**团队…**，生成的链接会让对方以选定的角色加入该团队（只有所有者能邀请另一位所有者）。链接只能用一次，7 天内有效，使用前可以撤回。程序不发邮件：把链接复制给对方，对方打开后自己填写用户名、显示名和密码。
- **个人空间和团队。** 每个账号有一个别人不能加入的个人空间。团队有所有者、管理员和成员：成员可以编辑团队的项目，所有者和管理员负责管理。
- **共享项目。** 项目的**访问权限…**菜单可以把项目以查看者、编辑者或管理者的身份共享给其他账号或团队，也可以把项目移到另一个团队。查看者只能看，不能改。
- **登录会话。** 登录表单上有“保持登录 30 天”（*0.2.2*），默认勾选。取消勾选时，关闭浏览器即退出；即使浏览器一直开着，bridge 上的会话也最多保留 12 小时。桌面版的登录总是保持 30 天。过期的会话每小时清理一次。

系统管理员管理账号，但只能看到共享给他的项目。

## 桌面版登录

桌面版可以登录同一台服务器：**设置 → 服务器**，填写页面的地址（`https://cyberapi.example.com`，有子路径时带上子路径），再填用户名和密码。账号在页面上创建，桌面版只负责登录。

- 桌面版必须能访问这个地址，而且证书要被系统信任。桌面版总会校验这个证书，即使它自己的设置允许请求跳过证书校验。
- 登录后，首页会在本机项目旁边列出该账号的服务器项目。每个服务器项目都会复制一份到本机，在本机编辑（离线也可以），同步时再推到服务器；如果期间别人也改过，由你选择保留哪一份。
- 请求、响应历史和 Cookie 都留在本机；桌面版的登录信息存在数据目录的 `server.json` 里，不进项目库，也不进备份。

## AI 助手（MCP）

*0.2.2*。bridge 在 `/v1/mcp`（有子路径时在子路径下）提供 [Model Context Protocol](https://modelcontextprotocol.io) 服务，Claude Code、Cursor 等 MCP 客户端可以读取你的接口项目。它通过普通 HTTP 应答 JSON-RPC：一次请求一次应答，没有推送流，也没有会话；只提供工具（tools），没有 resources 和 prompts。

**令牌。** AI 助手用账号的 **MCP 令牌**登录，而不是账号密码。在设置菜单 → **MCP 令牌…** 里，按机器或客户端给令牌起名（如 `claude-laptop`），选有效期（30 天、90 天、1 年或直到吊销）后签发。令牌（`cat_…`）只显示这一次，同时给出添加到 Claude Code 的命令；bridge 只保存它的 SHA-256。令牌只能在 `/v1/mcp` 登录，不能登录页面，也不能调用页面用的接口；在同一个弹窗里吊销后，客户端下一次调用就会被拒绝。令牌随账号：账号被停用后，它的令牌都不能再登录。

```bash
claude mcp add --transport http cyberapi https://cyberapi.example.com/v1/mcp \
  --header "Authorization: Bearer ${CYBERAPI_MCP_TOKEN}"
```

**能做什么。** 令牌看到的就是它的账号能看到的，工具都是读取：`list_projects`、`list_apis`（项目的目录和接口，带路径和 id）、`get_api`（接口保存的请求，带上它从目录和项目继承的请求头和认证、它的断言、说明和示例（*0.3.1*））、`get_variables`（环境变量、自定义变量、全局请求头，各自注明所在的命名环境；以及项目的命名环境和该账号当前用的那个）和 `get_history`（接口最近的响应）。签发时勾选了**允许发送请求**的令牌还有 `send_api`：由 bridge 带着项目的变量发送一个已保存的接口（用参数指定的命名环境，没指定时用该账号在这个项目里最后选的环境，*0.2.2*；同样受[请求能发往哪里](#请求能发往哪里)和每个账号同时发送数量的限制），响应记入账号的历史；只能在账号至少是编辑者的项目里发送。勾选了**允许修改**的令牌（*0.3.1*）还有 `create_api`（在某个目录或顶层新建一个接口，带上给出的请求）和 `update_api`（用给出的部分——方法、地址、Query、请求头、请求体、说明——替换接口原来的，给了新名字就改名），同样只能在账号至少是编辑者的项目里用；不能删除任何东西。没有这一项的令牌，工具不会修改任何项目。

**会看到什么。** 看起来像密钥的值会显示为 `***`：`Authorization`、`Cookie` 之类的请求头，接口认证用的密码、令牌和 API Key，名字像 token、key、password、secret 的变量和表单 / JSON 字段；用模板写的值（`Bearer {{TOKEN}}`）按原样显示。响应体超过 8 KB 截断，整个结果超过 200 KB 截断，不是文本的响应体只给描述。每个账号每分钟最多调用 120 次，每次调用在 bridge 日志里记一行（`mcp call`，带账号、令牌名和工具名）。

## 升级

1. 先备份数据库（见[备份](#备份)）。
2. 改镜像的 tag 后执行 `docker compose up -d`，或者替换二进制后 `systemctl restart cyberapi-bridge`。

`cyberapi-bridge --version` 和它启动时打印的第一行日志会写出运行的是哪个构建：正式版是版本号和提交，如 `0.2.2 (abc1234)`；nightly 是带日期和提交的版本号，如 `0.2.2-nightly.20261024.137a858`（*0.2.2*）。新版本启动时会把表结构升级到当前版本。**旧版本的 bridge 无法在新版本升级过的数据库上启动**，所以要回退只能恢复升级前的备份。改动内容见 [`CHANGELOG.md`](../CHANGELOG.md)。登录会话在升级后仍然有效；已经打开的页面会一直用它加载时的版本，刷新一下即可。

**升级 bridge 时请一起升级团队里的桌面版**（*0.2.2*）。每个版本保存项目的方式有一个编号，版本在项目里新增了内容时编号就加一。保存方式比 bridge 旧的客户端（还没更新的桌面版，或者升级时一直开着的网页）仍然可以打开、查看服务器项目，但修改会被拒绝，并提示更新应用或刷新页面：否则它保存时会丢掉自己读不懂的内容，把别人加的设置一起抹掉。0.2.2 之前的客户端不报编号，按第一种方式算。

## 备份

服务器保存的一切都在数据库里，按备份任何数据库的方式备份即可（`pg_dump`、`mysqldump`，或云服务商的快照）。有两样东西不在数据库里：

- **在浏览器里对服务器项目做的、还没同步的修改**，在同步之前是这个浏览器本地存储里的草稿。
- **桌面版的服务器项目副本**和它自己的项目在桌面版的数据目录里，由桌面版自己的备份负责。

*0.3.1*。除了备份，服务器还为每个项目保留最近 50 个版本：每次被整份替换（同步、上传、恢复版本）之前的样子（`project_versions` 表）。编辑者可以在项目菜单的“历史版本…”里恢复，恢复时现在的样子也会存成一个版本。
