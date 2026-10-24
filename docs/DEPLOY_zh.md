中文 | [English](./DEPLOY.md)

# 部署网页版

网页版只有一个程序：`cyberapi-bridge`。它在同一个地址上提供页面（编译成 WebAssembly 的应用，内置在程序里）和页面用的 API；账号、团队、项目、响应历史、Cookie 和设置都存在 PostgreSQL 或 MySQL 里；页面准备好的请求也由它发出。它自己的磁盘上不存任何东西。

- [需要什么](#需要什么)
- [先试一下](#先试一下)
- [Docker Compose](#docker-compose)
- [二进制加 systemd](#二进制加-systemd)
- [选项](#选项)
- [第一个管理员](#第一个管理员)
- [反向代理和 HTTPS](#反向代理和-https)
- [请求能发往哪里](#请求能发往哪里)
- [账号、团队和邀请](#账号团队和邀请)
- [桌面版登录](#桌面版登录)
- [升级](#升级)
- [备份](#备份)

## 需要什么

- **数据库**：PostgreSQL 或 MySQL（CI 测试的是 PostgreSQL 18 和 MySQL 8.4）。给 bridge 一个空库和一个能在里面建表的用户：首次启动时它会建好表，之后每次启动都会把表结构升级到当前版本。
- **程序**：镜像 [`vicanso/cyberapi-web`](https://hub.docker.com/r/vicanso/cyberapi-web)，有 linux/amd64 和 linux/arm64（tag：各个版本号如 `0.2.0`、`latest`，以及 `main` 的 `nightly`）；或者 [Releases](https://github.com/vicanso/cyberapi/releases) 里的 `cyberapi-web-<版本>-x86_64-unknown-linux-gnu.tar.gz`（需要 glibc 2.39 或更新：Ubuntu 24.04、Debian 13）。
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
    image: vicanso/cyberapi-web:0.2.0
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
tar -xzf cyberapi-web-0.2.0-x86_64-unknown-linux-gnu.tar.gz
sudo install -m 755 cyberapi-web-0.2.0-x86_64-unknown-linux-gnu/cyberapi-bridge /usr/local/bin/
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

## 选项

每个选项都是一个命令行参数，表中列出的也可以用环境变量设置。

| 参数 | 环境变量 | |
|---|---|---|
| `--store <url>` | `CYBERAPI_BRIDGE_STORE` | **必填。** `postgres://…` 或 `mysql://…`。建议用环境变量：它不会出现在进程列表里，bridge 的错误信息里也不会带出密码。 |
| `--listen <addr>` | `CYBERAPI_BRIDGE_LISTEN` | 监听地址，默认 `127.0.0.1:7380`；镜像里设为 `0.0.0.0:7380`。 |
| `--setup-token <token>` | `CYBERAPI_BRIDGE_SETUP_TOKEN` | 创建第一个管理员用的 token。不设置时随机生成并写进日志。 |
| `--allow-target <网段>` | `CYBERAPI_BRIDGE_ALLOW_TARGETS` | 请求还可以发往的网段，逗号分隔，如 `10.0.0.0/8,192.168.1.0/24`（见[请求能发往哪里](#请求能发往哪里)）。 |
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

**放在子路径下。** 页面用到的地址都是相对于页面的，所以可以和其他应用共用一个域名，放在 `https://example.com/cyberapi/` 下。由代理去掉前缀（注意两处结尾的斜杠）：

```nginx
    location /cyberapi/ {
        proxy_pass http://127.0.0.1:7380/;
        proxy_set_header Host $host;
        proxy_read_timeout 600s;
        proxy_send_timeout 600s;
    }
```

**登录失败按用户名（15 分钟 5 次）和客户端地址（15 分钟 30 次）分别计数。** 在代理后面，bridge 只看得到代理的地址，所以按地址的计数是所有人合在一起的：任何人累计失败 30 次，所有人都要等这段时间过去才能登录。请在代理上也按真实的客户端地址限制 `/v1/login`，例如用 nginx 的 `limit_req`。

## 请求能发往哪里

页面的请求由 bridge 代发，所以默认**只能发往公网地址**：回环、内网网段（`10.0.0.0/8`、`172.16.0.0/12`、`192.168.0.0/16`）、链路本地（包括云服务的元数据地址 `169.254.169.254`）等都会被拒绝。检查发生在 DNS 解析之后，每次重定向都会再查一次。服务器不能成为进入它所在网络的通道。

要测试自己内网的接口，就放行它们的网段：

```bash
CYBERAPI_BRIDGE_ALLOW_TARGETS=10.0.0.0/8,192.168.1.0/24
```

在容器里，`127.0.0.1` 是容器自己；宿主机上的接口要用宿主机的地址访问，并且要放行这个地址。每个账号同时最多有 16 个请求在发送，再多的会立即回复“忙”，不会排队。

桌面版即使操作的是服务器项目，请求也是从它所在的电脑直接发出，以上限制对它不适用。

## 账号、团队和邀请

- **账号只能通过邀请创建。** 在页面的设置菜单（齿轮）里，系统管理员打开**账号管理…**，可以生成邀请链接（新账号只有自己的个人空间）、重置密码、停用账号。团队的所有者或管理员打开**团队…**，生成的链接会让对方以选定的角色加入该团队（只有所有者能邀请另一位所有者）。链接只能用一次，7 天内有效，使用前可以撤回。程序不发邮件：把链接复制给对方，对方打开后自己填写用户名、显示名和密码。
- **个人空间和团队。** 每个账号有一个别人不能加入的个人空间。团队有所有者、管理员和成员：成员可以编辑团队的项目，所有者和管理员负责管理。
- **共享项目。** 项目的**访问权限…**菜单可以把项目以查看者、编辑者或管理者的身份共享给其他账号或团队，也可以把项目移到另一个团队。查看者只能看，不能改。
- **登录会话**有效期 30 天，过期的会话每小时清理一次。

系统管理员管理账号，但只能看到共享给他的项目。

## 桌面版登录

桌面版可以登录同一台服务器：**设置 → 服务器**，填写页面的地址（`https://cyberapi.example.com`，有子路径时带上子路径），再填用户名和密码。账号在页面上创建，桌面版只负责登录。

- 桌面版必须能访问这个地址，而且证书要被系统信任。桌面版总会校验这个证书，即使它自己的设置允许请求跳过证书校验。
- 登录后，首页会在本机项目旁边列出该账号的服务器项目。每个服务器项目都会复制一份到本机，在本机编辑（离线也可以），同步时再推到服务器；如果期间别人也改过，由你选择保留哪一份。
- 请求、响应历史和 Cookie 都留在本机；桌面版的登录信息存在数据目录的 `server.json` 里，不进项目库，也不进备份。

## 升级

1. 先备份数据库（见[备份](#备份)）。
2. 改镜像的 tag 后执行 `docker compose up -d`，或者替换二进制后 `systemctl restart cyberapi-bridge`。

新版本启动时会把表结构升级到当前版本。**旧版本的 bridge 无法在新版本升级过的数据库上启动**，所以要回退只能恢复升级前的备份。改动内容见 [`CHANGELOG.md`](../CHANGELOG.md)。登录会话在升级后仍然有效；已经打开的页面会一直用它加载时的版本，刷新一下即可。

## 备份

服务器保存的一切都在数据库里，按备份任何数据库的方式备份即可（`pg_dump`、`mysqldump`，或云服务商的快照）。有两样东西不在数据库里：

- **在浏览器里对服务器项目做的、还没同步的修改**，在同步之前是这个浏览器本地存储里的草稿。
- **桌面版的服务器项目副本**和它自己的项目在桌面版的数据目录里，由桌面版自己的备份负责。
