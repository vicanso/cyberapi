[中文](./FEATURES_zh.md) | English

# Cyber API features

What the desktop app and the web version do. Where only one of them does something, it says so. Back to the [README](../README.md).

Longer guides: [checks and runs](./CHECKS.md), [import and export](./IMPORT.md), [sync from a source](./SOURCE.md). Pictures of the desktop app: [gallery](./GALLERY.md).

A version in italics is in nightly builds until that release.

## Native and fast

- macOS, Windows and Linux, drawn on the GPU with GPUI. A project with thousands of APIs opens at once.
- Light and dark themes, English and Chinese, adjustable text size and line wrapping.
- The desktop window reopens where you left it, on each screen.

## Projects, folders and APIs

- Search by name or pinyin. Drag to reorder, duplicate, move into a folder, or copy an API's config.
- Find and replace across a project or a folder — URIs, query parameters, headers and bodies. Preview the matches, then replace. Undo from the notice for 10 seconds.
- Pin the APIs you use most (⌘/Ctrl-1…9). Pins sit in the title bar and are not repeated in the tab bar.
- Each unpinned API you open stays as a tab across the request and the response, up to 16. Edits, responses and a request still on its way are there when you come back. Close one with its button, a middle click or ⌘/Ctrl-W (the others, or all, from its menu). Ctrl-Tab / Ctrl-Shift-Tab switches. The tabs are open again next time. Hide that bar under Editor in the settings (*0.3.1*); the pins stay.
- On the desktop, **Open in a new window** (*0.4.1*) puts that API's request and response side by side, with its own environment switcher, so you can send against another environment without changing the main window. Its tab stays and says where it went. Closing the window (or ⌘W) brings it back. Closing the tab or leaving the project closes the window.
- **New scratch request** (project menu or command palette) opens a tab on this project's variables and environment. It stays in your own settings across a restart and is dropped when you close the tab. **Save to project…** picks a folder and it becomes an API there (*0.3.1*).
- Group projects on the home page.
- Copy or move an API or a folder to another project — the project, then the folder. The whole request and a folder's settings go along. The response history does not. Variables the other project lacks are named afterwards (*0.4.1*).
- Undo a delete from its notice within 10 seconds. It comes back where it was, with what it held and its pins. Its APIs' response history does not (*0.4.1*).
- ⌘/Ctrl-Enter sends, ⌘/Ctrl-N adds an API, ⌘/Ctrl-F finds one, ⌘/Ctrl-L goes to the address. Arrow keys walk the tree. Delete and ⌘/Ctrl-D delete and duplicate. A list shows every shortcut.
- ⌘/Ctrl-K opens the command palette on any page: find an API in any project (name, path, folder, method, pinyin), open a project, switch the environment, or run a command (new API, import, run, recent responses, settings).

## Requests

- Query, headers and a body: JSON, form, multipart, XML or plain text. A query typed into the address moves to the Query table. Any key-value list can be edited in bulk as text.
- Path parameters appear above Query when the address has `:param` or `{param}`. They are filled in as the request goes out and as code is generated, and they travel with OpenAPI `in: path` parameters and Postman URL variables.
- Each row of path parameters, query, headers, form bodies and folder variables can carry a description (*0.4.1*). Bulk edit keeps it. OpenAPI, Postman and Insomnia bring it in. Postman, OpenAPI and MCP take it out. What a later import or a source sync does with an empty description is in [Import](./IMPORT.md) and [Sync from a source](./SOURCE.md).
- Auth on the API, or inherited from its folder or the project: Basic, a bearer token, an API key in a header or the query, Digest, or OAuth 2.0 (client credentials or password). Values can be template functions. An audience and a resource go along when the server asks. The token is kept as cookies are — yours, never in the project, its sync or a backup — and is asked for again when it expires or the API answers 401, the refresh token first. The client signs in with a Basic header or in the body.
- On the desktop, OAuth 2.0 can also be the authorization code with PKCE (*0.3.1*). **Authorize** opens the system browser and listens on `127.0.0.1`, on a random port unless you set one. The web version can save that setup and says the browser step is desktop-only. A run, the command line and an assistant do not open a browser; they say to authorize in the desktop app first.
- A folder's headers go out with every API in it.
- Timeouts, HTTP/1.1 or HTTP/2, whether to follow redirects, and whether to accept a self-signed certificate. An API can override any of these in its own Settings tab.
- On the desktop, requests can go through an HTTP or SOCKS5 proxy (the settings, or `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY`). Client certificates (PEM, or PKCS#12 with a password) are presented to the hosts they are for, and extra CAs — a company's own — can be trusted.
- Multipart on the desktop: pick a local file per row, kept as a `file://` path. An imported `file://` field with no path asks for one as you send. In the web version the file stays in the page's memory. What syncs is a marker, and the bytes are not sent along to other clients.
- The web version sends from its server, and only to public addresses (*0.3.1*). A private address is refused, and the failure says why.

## GraphQL

- The query, the variables and the operation name are edited apart, with highlighting. POST sends JSON. GET sends query parameters.
- **Format** indents the query (comments kept) and the variables.
- The response's `errors` are shown above the body and marked in the query where they point.
- Fetch the endpoint's schema and the editor completes fields, arguments and enum values. Browse the schema and click a field to put it in the query.
- A subscription sent over HTTP shows results as they arrive when the server answers with an event stream (GraphQL Yoga, graphql-sse).
- An address of `ws://` or `wss://` subscribes with graphql-transport-ws. **Subscribe** opens it. Each `next` or `error` shows as it comes, and the conversation is kept in the history when it ends. Desktop only, as WebSocket is. An HTTP subscription is unchanged.
- GraphQL requests from Postman, Insomnia, Hoppscotch and Bruno stay GraphQL.

## gRPC

- Unary calls and server streams. Pick gRPC, give `host:port`, and choose the method from server reflection or, on the desktop, from `.proto` files.
- The message is JSON (start from an example; template functions work). Metadata are headers. The reply is JSON, with the call's status.
- A server stream shows each reply as it arrives, with its time, until it ends or you stop it.
- The API keeps the definitions that service needs, so it can be sent anywhere, including the web version, without asking the server again. Copy a call as grpcurl.
- Client streams and bidirectional streams connect on the desktop. **Connect** opens the call. **Send message** sends the editor's message, templates run each time. **Finish sending** closes a client stream and the one reply comes after. A bidirectional stream sends and receives together. The conversation is kept when it ends.
- The web version can edit those methods. A run, the command line and an assistant skip them.

## WebSocket

- Method WS, address `ws://` or `wss://` (`http(s)://` is taken as the matching WebSocket). **Connect** uses the API's query, headers, cookies and auth, including an OAuth 2.0 token, through the proxy and with that host's client certificate.
- **Send message** (or ⌘/Ctrl-Enter) sends the Message tab. Templates run on each send. Messages show as they go, each with its time. Binary messages are base64.
- **Disconnect**, or the server closing, keeps the conversation in the history. A refused upgrade is shown as an ordinary response.
- A conversation is kept up to 64 MB, as a response body is. Past that the connection closes and the history says it was cut. A streaming gRPC call and a GraphQL subscription are the same (*0.4.1*).
- Copy as websocat. Desktop only: the web server does not relay WebSockets, and a run, the command line and an assistant skip them.

## Server-sent events

An event stream (`text/event-stream`) shows as it arrives: time, type, id and data. Pick one to read it whole, including the retry it asked for. Stop when you want, or when the server ends it. What arrived is kept, as events or as the raw body. A language model's streamed reply is the usual case.

## Variables and functions

- Environment variables, custom variables and global headers, shared by every environment or set per named environment (dev, staging, prod). Switch beside the project's name. The choice is yours, not the team's.
- The variables dialog can show one layer, or a **matrix** across Shared and every environment: missing cells are highlighted, and duplicate names can be folded into one.
- An editor can mark an environment as production (*0.4.1*). The switcher turns red. A request that may change something — anything but GET, HEAD, OPTIONS and a GraphQL query, including gRPC and WebSocket — asks first. You can skip the question for that project until the app quits. Running a folder or the project asks once. An assistant sends only what would not be asked about.
- A folder can have variables of its own (*0.4.1*), read as `{{value(name)}}` or as `{{NAME}}` at the start of an address. The nearest wins: a run's data and extracted values, the API's path parameters, the inner folder, outer folders, the environment, then the shared variables. A template written inside one of these values is not evaluated. Keep a value that changes with the environment in the environment.
- The lock on a row makes that variable secret. The project keeps the name. The value is yours: never synced, exported or backed up, and each teammate, viewers included, fills in their own. It is masked on screen and in the request the history keeps. An assistant sees the name.
- Hints while you type. `{{g(api, data.token)}}` reads a field of another API's latest response. Also `{{value(name)}}`, `{{timestamp()}}`, `{{now(%Y-%m-%d)}}`, `{{uuid()}}`, `{{random(1, 100)}}`, `{{randomString(16)}}`, `{{md5(…)}}`, `{{sha256(…)}}`, `{{hmacSha256(key, data)}}`, `{{urlEncode(…)}}`.
- From *0.4.1*: `{{now(%Y-%m-%d, -1d)}}` (shift by s / m / h / d / w), `{{sha1(…)}}`, `{{sha512(…)}}`, `{{hmacSha1(…)}}`, `{{hmacSha512(…)}}`, `{{base64Decode(…)}}`, `{{urlDecode(…)}}`, `{{base64.readFile(…)}}`, `{{readTextFile(…)}}`, `{{openFile()}}` (pick a file as you send), `{{prompt(label)}}` (asked as you send, such as a one-time code, remembered until the app quits), `{{env(NAME)}}` (this machine's environment, including what the shell's startup files export; desktop).

## Responses

- Time split into DNS, TCP, TLS, the server and transfer.
- The request as it went out. Each header says where it came from. Secrets stay masked until you show them. Headers, cookies and the sent headers can be filtered and copied per row or as a table. The whole exchange can be read as `curl -v` prints it.
- JSON raw, indented, or as a tree (NDJSON too). A node's menu copies the value, the JSON or a JSONPath, copies `{{g(api, path)}}`, or adds a JSONPath assertion or a run extraction.
- Images in place, fitted or zoomed, with format and size. A PDF in the system viewer. HTML in the browser, on the desktop. Other bytes as hex. A body with no content type is recognized from its first bytes.
- Copy a body or save it. Search the body or the wire text from the status bar or ⌘/Ctrl-F, with match case and next / previous. Narrow JSON with a JSONPath kept per API.
- A history per API, 5 to 100 entries (10 by default), set under Data. Each entry shows the method and path. Picking one puts that request back in the editor. Compare two responses line by line; JSON is indented first, and unchanged parts fold.
- **Recent responses** on the project lists the newest 50 across its APIs. Pick one to open that API there.
- A large body stays quick. The panel shows the first 1–2 MiB of a text body and of the wire view, 1 by default, set under Data (*0.3.1*). A line too long to wrap scrolls sideways.

## Docs, checks and runs

An API's Docs tab says what it is for, lists its fields, and keeps example responses. Assertions check the answer as you edit them. **Run…** sends a folder or the project in order, and `cyberapi-cli run` does the same in a terminal or in CI. The desktop app can also load-test one API.

[Checks, docs and runs](./CHECKS.md) — fields and schemas, assertions, examples, runs, the command line, load tests.

## Cookies

Saved from each response and sent with the requests they match, across every project. Search, add, edit, delete or clear them.

## Import, export and code

Paste a cURL command to make an API, or copy any request as cURL, HTTPie, JavaScript `fetch`, Node axios, Python `requests`, PHP Guzzle, Go `net/http`, Rust reqwest, Java or C# `HttpClient`, or Swift `URLSession`.

Import Postman, Insomnia, Hoppscotch, Bruno, Swagger 2 / OpenAPI 3, HAR, `.http` files and cURL. Importing again updates matching APIs by default and keeps the values you typed. A project or folder can remember the address it came from and sync later.

[Import and export](./IMPORT.md) — formats, what an update keeps, export. [Sync from a source](./SOURCE.md) — a bound document, including a scheduled sync that follows the document and leaves your values and secrets.

## Mock server

- On the desktop, **Mock server…** answers on this machine from the project's examples (port 4010 unless you choose another).
- A server project also has `/v1/mock/<project-id>/…` on the bridge, shown in that dialog. Call it with a session or an MCP token (`Authorization: Bearer cat_…`, `X-CyberAPI-Token`, or `?token=cat_…`).
- The method and the path pick the API. The path is the address without its variable, scheme and host. `{{id}}`, `:id` and `{id}` match any one segment, and more fixed segments win. The answer is the first example, or the one named by `x-mock-example`. No example, or no API, is a 404 that says why.
- Any origin may call it. It follows the project as you edit. The desktop dialog lists what the local server answered.

## Your data

On the desktop, back up and restore in the same format as the earlier Tauri app. The app also backs up once a day into `backups` in the data folder and keeps the newest 7. Turn that off, or open the folder, under Local data (*0.4.1*).

## Teams

- One binary or one Docker image, at the root or under a sub-path. Accounts come from invitations. Roles on a project are viewer, editor and manager. Share a project with accounts or teams, or move it to another team.
- The desktop app signs in to the same server. The home page lists the team's projects as they appear. Upload a local project and keep its history. Edit a shared project offline and sync when you choose. If someone else changed it meanwhile, you choose which copy to keep.
- An API or a folder can stay local and out of that sync.
- The server keeps the last 50 versions of a project, taken before each sync. An editor restores one from **Versions…**. The copy that was replaced is kept too, so a restore can be undone.
- Compare a version with now, or with the version after it (*0.4.1*): folders and APIs added, removed, renamed, moved or edited, and variables and environments that changed. Open an edited API to see where its request differs.
- When the server is set up for OpenID Connect (*0.4.1*), the sign-in page and the desktop offer single sign-on. An invited account whose name is that person's email is linked on the first sign-in. The password still works.

## AI assistants

The web server, and the desktop app on `127.0.0.1`, are MCP servers. Claude Code, Cursor and other MCP clients sign in with a token and read projects, APIs, variables and responses, with secrets masked.

- Allowed to send: it may send saved APIs. If the API has assertions, that reply lists each result and, where one failed, what was found. Those results are not kept in the history (*0.3.1*).
- Allowed to change: it may create and edit APIs where you can edit. It cannot delete. On the desktop, local projects go through the local server. A server project goes through the server's MCP endpoint.
