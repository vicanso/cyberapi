[中文](./DEPLOY_zh.md) | English

# Deploying the web version

The web version is one program, `cyberapi-bridge`. It serves the page (the app compiled to WebAssembly, built into the binary) and its API from one address, keeps the accounts, teams, projects, response history, cookies and settings in PostgreSQL or MySQL, and sends the requests the page prepares. It keeps nothing on its own disk.

This guide follows `main`. What a release does not have yet is marked with the version it comes in (*0.2.2*), and is in the `nightly` builds until then; a release's own guide is at its tag, `https://github.com/vicanso/cyberapi/blob/v<version>/docs/DEPLOY.md`.

- [What you need](#what-you-need)
- [Try it](#try-it)
- [Docker Compose](#docker-compose)
- [The binary, with systemd](#the-binary-with-systemd)
- [Kubernetes (Helm)](#kubernetes-helm)
- [Options](#options)
- [The first administrator](#the-first-administrator)
- [HTTPS behind a reverse proxy](#https-behind-a-reverse-proxy)
- [Where requests may go](#where-requests-may-go)
- [Accounts, teams and invitations](#accounts-teams-and-invitations)
- [Signing in from the desktop app](#signing-in-from-the-desktop-app)
- [AI assistants (MCP)](#ai-assistants-mcp)
- [Upgrading](#upgrading)
- [Backups](#backups)

## What you need

- **A database**: PostgreSQL or MySQL (CI tests PostgreSQL 18 and MySQL 8.4). Give the bridge an empty database and a user that can create tables in it: it creates the schema on its first start and brings it up to date on every start after.
- **The program**: the image [`vicanso/cyberapi-web`](https://hub.docker.com/r/vicanso/cyberapi-web) for linux/amd64 and linux/arm64 (tags: each version such as `0.3.0`, `latest`, and `nightly` from `main`), or `cyberapi-web-<version>-x86_64-unknown-linux-gnu.tar.gz` from [Releases](https://github.com/vicanso/cyberapi/releases) (glibc 2.39 or newer: Ubuntu 24.04, Debian 13).
- **HTTPS** in front of it for anything beyond a try on your own machine: the login cookie is `Secure`, which browsers send over HTTPS only.

## Try it

On your own machine, against a PostgreSQL you already have:

```bash
docker run --rm -p 127.0.0.1:7380:7380 \
  -e CYBERAPI_BRIDGE_STORE=postgres://user:password@host.docker.internal:5432/cyberapi \
  vicanso/cyberapi-web --insecure-cookie
```

`host.docker.internal` is the host as Docker Desktop names it; on Linux add `--add-host=host.docker.internal:host-gateway`. Open http://127.0.0.1:7380/ and make the first administrator with the setup token from the log ([below](#the-first-administrator)). Arguments after the image name go to the bridge; `--insecure-cookie` lets the login work over plain HTTP, for trying it only.

## Docker Compose

The bridge and a PostgreSQL beside it, published on this host's loopback for a reverse proxy on the same host:

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

Pin a version as above rather than `latest`, so an upgrade is a decision ([Upgrading](#upgrading)). With MySQL the address is `mysql://user:password@host:3306/cyberapi`.

The image runs as a non-root user on a distroless base (no shell), listens on `0.0.0.0:7380` inside the container, and stops cleanly on `docker stop`: requests under way finish first.

## The binary, with systemd

```bash
tar -xzf cyberapi-web-0.3.0-x86_64-unknown-linux-gnu.tar.gz
sudo install -m 755 cyberapi-web-0.3.0-x86_64-unknown-linux-gnu/cyberapi-bridge /usr/local/bin/
```

The database address goes in a file only root can read, not on the command line, where anyone who can list processes would see it:

```bash
# /etc/cyberapi/bridge.env  (chmod 600)
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

## Kubernetes (Helm)

*0.3.1.* A chart is in the repository, `deploy/helm/cyberapi`: one Deployment of the image, a Service and, if you want one, an Ingress. It does not make the database: give it one you have.

```bash
git clone https://github.com/vicanso/cyberapi && cd cyberapi
kubectl create secret generic cyberapi-db \
  --from-literal=store-url='postgres://cyberapi:change-me@postgres:5432/cyberapi'
helm install cyberapi deploy/helm/cyberapi \
  --set store.existingSecret=cyberapi-db \
  --set ingress.enabled=true --set 'ingress.hosts[0].host=cyberapi.example.com' \
  --set 'ingress.hosts[0].paths[0].path=/' --set 'ingress.hosts[0].paths[0].pathType=Prefix'
```

- **The database**: `store.existingSecret` (and `store.existingSecretKey`, `store-url` by default) names a Secret of yours; or `store.url` puts the URL in a Secret the chart writes. Without either it refuses to install.
- **The first administrator**: `setupToken.value` or `setupToken.existingSecret`; without one the token is in the log (`kubectl logs deploy/cyberapi | grep -i "setup token"`).
- **The options** above, as values: `bridge.basePath` (the ingress path should match it), `bridge.allowTargets` (a list), `bridge.publicOnly`, `bridge.insecureCookie`, and `bridge.extraArgs` / `bridge.extraEnv` for the rest.
- **One replica** (`replicaCount: 1`): the login throttle, the MCP limit and the requests an account has out at once are counted in each process. The pod runs as non-root with a read-only root filesystem; it is ready once it reaches its database.
- **TLS** is the Ingress's (`ingress.tls`): behind it the login cookie stays `Secure`. A version other than the chart's is `image.tag` (`nightly` follows `main`).

## Options

Each is a flag and, where given, an environment variable.

| Flag | Environment variable | |
|---|---|---|
| `--store <url>` | `CYBERAPI_BRIDGE_STORE` | **Required.** `postgres://…` or `mysql://…`. Prefer the variable: it is not shown in the process list, and the password is left out of the bridge's error messages. |
| `--listen <addr>` | `CYBERAPI_BRIDGE_LISTEN` | Where to listen. `127.0.0.1:7380` by default; the image sets `0.0.0.0:7380`. |
| `--setup-token <token>` | `CYBERAPI_BRIDGE_SETUP_TOKEN` | The token the first administrator is made with. Random, and logged, when not given. |
| `--allow-target <ranges>` | `CYBERAPI_BRIDGE_ALLOW_TARGETS` | Address ranges requests may also go to, comma-separated, e.g. `127.0.0.1/32` ([Where requests may go](#where-requests-may-go)). |
| `--public-only` | `CYBERAPI_BRIDGE_PUBLIC_ONLY` | *0.2.2.* Refuse private networks too, for a bridge open to the internet. |
| `--base-path <path>` | `CYBERAPI_BRIDGE_BASE_PATH` | *0.2.2.* Serve the page and the API under this path, e.g. `/cyberapi`, and keep the login cookie to it ([under a sub-path](#https-behind-a-reverse-proxy)). Empty by default: the root. |
| `--insecure-cookie` | | Leave `Secure` off the login cookie, for plain HTTP on your own machine. |
| `--www <dir>` | | Serve this directory as the page instead of the built-in one (development). |
| | `RUST_LOG` | How much to log: `info` by default, `debug` for more. Logs go to standard output. |

`cyberapi-bridge --help` lists them too.

## The first administrator

While the database has no account, the bridge logs a setup token on every start:

```
INFO cyberapi_bridge: no account yet: make the first administrator with this setup token setup_token=…
```

Open the page; it asks for the token, a name, a display name and a password, and makes that account a system administrator. From then on the token does nothing, and new accounts come only from invitations. The token is random on each start unless `CYBERAPI_BRIDGE_SETUP_TOKEN` sets it; wrong tokens are slowed down.

## HTTPS behind a reverse proxy

Let the proxy terminate TLS and pass everything to the bridge. Two settings matter: request bodies up to 64 MB (the bridge's own limit), and a read timeout above the app's default request timeout of 300 seconds, so a slow API's answer is not cut off by the proxy first.

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

**Event streams** (*0.2.2*). An API whose answer is an event stream (`text/event-stream`) is passed to the page as it comes, on `POST /v1/send/stream`. The bridge marks that answer `X-Accel-Buffering: no`, so nginx passes it on unbuffered; another proxy must not buffer responses on that path either (Caddy and Traefik stream by default). The stream ends when the reader stops it, when nothing comes for the request's read timeout, or at 64 MB; while it lasts it is one of the account's 16 requests in flight, and the proxy's read timeout applies to the gaps between its pieces.

**Under a sub-path** (*0.2.2*). To live at `https://example.com/cyberapi/` beside other applications, start the bridge with `--base-path /cyberapi` (or `CYBERAPI_BRIDGE_BASE_PATH=/cyberapi`) and let the proxy pass the path on as it is:

```nginx
    location /cyberapi/ {
        proxy_pass http://127.0.0.1:7380;
        proxy_set_header Host $host;
        proxy_read_timeout 600s;
        proxy_send_timeout 600s;
    }
```

The bridge then answers under `/cyberapi/` only, sends `/cyberapi` on to `/cyberapi/`, and keeps the login cookie to `/cyberapi/`, so the other applications on the host name never receive it. A proxy that strips the prefix instead (`proxy_pass http://127.0.0.1:7380/;`, Caddy's `handle_path`, Traefik's `StripPrefix`) also works with no base path, since the page addresses everything relative to itself; the cookie is then the whole host's.

**Failed sign-ins are counted per account name (5 in 15 minutes) and per client address (30 in 15 minutes).** Behind a proxy the bridge sees only the proxy's address, so the per-address count is everyone's together: 30 failed sign-ins from anyone hold back every sign-in until the window passes. Limit `/v1/login` at the proxy by the real client address as well, for example with nginx's `limit_req`.

## Where requests may go

The bridge sends the page's requests itself, from where it runs. By default (*0.2.2*; up to 0.2.1, public addresses only) it reaches **public addresses and private networks** (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `100.64.0.0/10`, IPv6 `fc00::/7`), since the APIs a team tests are mostly on its own network. It refuses only what is never an API under test and is where a request sent on someone's behalf does harm: **its own host** (loopback `127.0.0.0/8`, `::1`, where services trust whoever calls) and **its link** (link-local `169.254.0.0/16`, `fe80::/10`, among them the cloud metadata address `169.254.169.254`, which hands out the machine's credentials), besides unspecified, multicast, broadcast and reserved addresses. The check is on the address a name resolves to, and again on every redirect. A refused request says the name and the address it led to.

Whoever can sign in to the page can therefore reach what the bridge's machine can. Run it on the network of the APIs it is for, not in a more privileged one.

- To reach a service on the bridge's own host, allow it: `CYBERAPI_BRIDGE_ALLOW_TARGETS=127.0.0.1/32`. In a container, `127.0.0.1` is the container itself; an API on the host is reached at the host's address.
- For a bridge open to the internet, `--public-only` (`CYBERAPI_BRIDGE_PUBLIC_ONLY=1`) refuses the private networks as well, and `--allow-target` lets the ranges you name back: `CYBERAPI_BRIDGE_ALLOW_TARGETS=10.1.0.0/16,192.168.10.0/24`.

Ranges are written as addresses (CIDR), not names: the check is on what a name resolves to, so a name allowed by itself could lead anywhere. Each account has at most 16 requests under way at once; more are answered "busy" at once rather than queued.

The bridge connects to every server itself: it does not go through a proxy, neither one in its environment (`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`) nor the desktop app's proxy setting, which a page's requests do not carry. A proxy would see past the checks above, which are on the address the bridge connects to.

The desktop app sends its requests from the computer it runs on, even for server projects, so none of this applies there.

## Accounts, teams and invitations

- **Accounts come from invitations.** In the page's settings menu (the gear), a system administrator opens **Accounts…** to create an invitation link for a new account with only its own space, reset a password or disable an account. A team's owner or admin opens **Teams…** to create a link that also joins the team with a chosen role (only an owner can invite another owner). A link works once, for 7 days, and can be taken back until it is used. The program sends no email: copy the link to the person, who opens it and chooses a name, display name and password.
- **Spaces and teams.** Every account has a personal space no one else can join. Teams have owners, admins and members; members edit the team's projects, owners and admins manage them.
- **Sharing a project.** A project's **Access…** menu shares it with another account or team as Viewer, Editor or Manager, or moves it to another team. Viewers see but do not change it.
- **Sessions.** The sign-in forms have "Keep me signed in for 30 days" (*0.2.2*), ticked by default. Unticked, the login ends when the browser closes, and on the bridge within 12 hours even while it stays open. The desktop app's sign-in is always kept for 30 days. Expired sessions are removed every hour.

System administrators manage accounts but see only the projects shared with them.

## Signing in from the desktop app

The desktop app can sign in to the same server: **Settings → Server**, the page's address (`https://cyberapi.example.com`, or with its sub-path), then a name and password. Accounts are made in the page; the desktop only signs in.

- The address must be reachable from the desktop with a certificate the system trusts. The desktop always checks it, also when its own settings let requests skip certificate checks.
- Once signed in, the home page lists the account's server projects beside this computer's own. Each is copied to the computer, edited there, also offline, and sent to the server when you sync; if someone else changed it meanwhile, you choose whose version to keep.
- Requests, response history and cookies stay on the computer; the desktop's sign-in is kept in `server.json` in its data directory, not in the projects' database and not in backups.

## AI assistants (MCP)

*0.2.2.* The bridge is a [Model Context Protocol](https://modelcontextprotocol.io) server at `/v1/mcp` (below the sub-path, if any), so Claude Code, Cursor or another MCP client can read your API projects. It answers JSON-RPC over plain HTTP: one request, one reply, no stream and no session; it has tools, not resources or prompts.

**Tokens.** An assistant signs in with an **MCP token** of an account, never with its password. Open the settings menu → **MCP Tokens…**, name the token after the machine or the client (`claude-laptop`), pick how long it lasts (30 or 90 days, a year, or until revoked) and issue it. The token (`cat_…`) is shown once, with the line that adds it to Claude Code; the bridge keeps only its SHA-256. A token signs in at `/v1/mcp` and nowhere else — not the page, not the API the page uses — and revoking it in the same dialog shuts its client out at the next call. Tokens go with their account: a disabled account's sign nobody in.

```bash
claude mcp add --transport http cyberapi https://cyberapi.example.com/v1/mcp \
  --header "Authorization: Bearer ${CYBERAPI_MCP_TOKEN}"
```

**What it can do.** The token sees what its account sees, and its tools read: `list_projects`, `list_apis` (a project's folders and APIs, each with its path and id), `get_api` (an API's saved request, with the headers and auth it takes from its folders and project, its assertions, what it is for and its examples (*0.3.1*)), `get_variables` (environment variables, custom variables and global headers, each with the named environment it is in; the project's named environments, and the one the account sends in) and `get_history` (an API's latest responses). A token issued with **May send requests** also has `send_api`, which sends a saved API from the bridge with the project's variables — in the named environment it is given, or else the one the account last chose in the project (*0.2.2*), subject to [where requests may go](#where-requests-may-go) and to the account's requests at a time — and keeps the response in the account's history. When the API has assertions enabled, that answer also lists each one's result — whether it held, and when it did not the value that was there; a check that could not be made says why — and those results are not kept in the history (*0.3.1*). An authorization-code sign-in is not done from here: `send_api` does not open a browser, and with no refresh token the answer says to authorize in the desktop app first (*0.3.1*). It sends only in projects where the account is at least an editor. A token issued with **Allow changes** (*0.3.1*) also has `create_api` (an API in a folder or at the top level, with the request given) and `update_api` (what it is given of an API's request — method, URL, query, headers, body, description — replaces what it had; a new name renames it), in projects where the account is at least an editor; nothing deletes. Without it, nothing a tool does changes a project.

**What it is shown.** Values that look like secrets — headers such as `Authorization` and `Cookie`, the passwords, tokens and API keys an API signs in with, variables and form or JSON fields named like a token, a key, a password or a secret — come back as `***`; a value written with a template (`Bearer {{TOKEN}}`) is shown as written. Bodies are cut at 8 KB, a whole answer at 200 KB, and a body that is not text is described, not shown. An account makes at most 120 calls a minute, and every call is a line of the bridge's log (`mcp call`, with the account, the token's name and the tool).

## Upgrading

1. Back up the database ([Backups](#backups)).
2. Change the image tag and `docker compose up -d`, or replace the binary and `systemctl restart cyberapi-bridge`.

`cyberapi-bridge --version`, and the first line it logs, say which build runs: a release's version and commit, `0.2.2 (abc1234)`, or a nightly's version with its day and commit, `0.2.2-nightly.20261024.137a858` (*0.2.2*). The new version brings the schema up to date on start. **An older bridge does not start on a database a newer one has upgraded**, so going back means restoring the backup taken before. Read [`CHANGELOG.md`](../CHANGELOG.md) for what changed. Sessions survive the upgrade; a page already open keeps the version it loaded until it is reloaded, so reload it.

**Upgrade the team's desktop apps with the bridge** (*0.2.2*). Each version keeps projects in a shape it names with a number, which goes up when a version keeps something new in them. A client of an older shape than the bridge's — a desktop app not yet updated, or a page left open across the upgrade — may still open and read server projects, but its changes to them are refused with a note to update the app or reload the page: it would write a project without what it cannot read, dropping other people's settings. Clients from before 0.2.2 do not say their shape and count as the first.

## Backups

Everything the server keeps is in its database: back it up as you back up any database (`pg_dump`, `mysqldump`, or your provider's snapshots). Two things are not in it:

- **Unsynced edits made in a browser** to a server project are a draft in that browser's local storage until synced.
- **The desktop's copies** of server projects, and its own projects, are in the desktop's data directory; the desktop's own backup covers them.

*0.3.1.* Besides the backups, the server keeps each project's last 50 versions: what it was before each time it was replaced whole — a sync, an upload, a version put back (the `project_versions` table). An editor puts one back from the project menu, Versions…; that keeps the project as it was too.
