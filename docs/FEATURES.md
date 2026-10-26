[中文](./FEATURES_zh.md) | English

# Cyber API features

Everything the desktop app and the web version do; where only one of them does something, it says so. Back to the [README](../README.md).

## Native and fast

macOS, Windows and Linux, drawn on the GPU with GPUI. A project with thousands of APIs opens at once. Light and dark themes, English and Chinese, adjustable text sizes and line wrapping; the desktop window reopens where you left it, on each screen.

## Projects, folders and APIs

Search by name or pinyin, drag to reorder, duplicate, move to a folder, copy an API's config; pin the APIs you use most (⌘/Ctrl-1…9); group projects on the home page. ⌘/Ctrl-Enter sends, ⌘/Ctrl-N adds an API, ⌘/Ctrl-F finds an API and ⌘/Ctrl-L goes to its address, the arrow keys walk the tree, Delete and ⌘/Ctrl-D delete and duplicate there, and a list shows every shortcut.

## Requests

Query, headers, auth — Basic, a bearer token, an API key in a header or the query, or Digest (cameras and routers ask for it), its values templates, set on the API or inherited from its folder or the project — and JSON, form, multipart, XML or plain-text bodies; bulk-edit any list as text. A folder's headers go with every API in it. A query typed into the URL moves to the Query table. Timeouts, HTTP/1.1 or HTTP/2, redirects followed or not, and self-signed certificates accepted when you say so; an API can set any of these for itself in its Settings tab. On the desktop, requests go through an HTTP or SOCKS5 proxy, set in the settings or taken from `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY`; client certificates (PEM, or PKCS#12 with its password) are presented to the hosts they are for, and authorities besides the system's — a company's own, say — trusted.

## GraphQL

A GraphQL body keeps the query, its variables and the operation name apart, highlighted; it is sent as JSON by a POST or as parameters by a GET, and the response's `errors` are shown above it and marked in the query where they point. Format indents the query (its comments kept) and the variables. Fetch an endpoint's schema and the query editor completes its fields, arguments and enum values; browse it — types, fields, arguments and what they are for — and click a field to put it in the query. A subscription sent over HTTP shows its results as they come, from servers that answer it as an event stream (GraphQL Yoga, graphql-sse). Postman's, Insomnia's, Hoppscotch's and Bruno's GraphQL requests import as GraphQL.

## gRPC

Unary and server-streaming calls: choose gRPC as the method, give the server's address, and pick a service's method from the server's reflection or from `.proto` files (desktop). The message is JSON — with an example to start from, and template functions — and the metadata are headers; the reply comes back as JSON with its status, and a stream of replies shows as it comes, each with its time, until it ends or you stop it. An API keeps the definitions its service needs, so it is sent anywhere, the web version included, without asking the server again. Copy a call as a grpcurl command. Client and bidirectional streaming are not supported yet.

## Server-sent events

An event stream (`text/event-stream`) shows as it comes — each event with its time, type, id and data, the one picked shown whole with the reconnection time it asked for — until the server ends it or you stop it; what came is kept in the history, as events or as it came. A language model's streamed reply, for instance.

## Variables and functions

Environment variables, custom variables and global headers per project, shared by every environment or set per named environment (dev, staging, prod…), which you switch beside the project's name — your choice alone, not your team's. Template functions such as `{{g(api, data.token)}}` (a field of another API's latest response), `{{value(name)}}`, `{{timestamp()}}`, `{{now(%Y-%m-%d)}}`, `{{uuid()}}`, `{{random(1, 100)}}`, `{{randomString(16)}}`, `{{md5(…)}}`, `{{sha256(…)}}`, `{{hmacSha256(key, data)}}`, `{{urlEncode(…)}}`, `{{base64.readFile(…)}}`, `{{readTextFile(…)}}` and `{{openFile()}}` (a file picked as you send) and `{{prompt(label)}}` (a value asked for as you send, such as a one-time code, remembered until the app quits) and `{{env(NAME)}}` (this machine's environment variable, those your shell's startup files export included; desktop), with hints while you type.

## Responses

Time split into DNS, TCP, TLS, server and transfer; the request as it went out, every header said where it came from, secrets masked till shown, and the whole exchange as `curl -v` prints it; JSON raw as it came, indented or as a tree (NDJSON too); images shown in place, fitted or zoomed, with their format and size; a PDF in the system viewer, an HTML page in your browser (desktop); any other bytes as hex; a body without a content type told by its first bytes; copy a body or save it to a file, or narrow a JSON one with a JSONPath filter kept per API; a history per API, each entry with the method and path it was sent to, and picking one puts its request back. A large body stays quick: the panel shows its start, and a line too long to wrap scrolls sideways.

## Assertions

Check an answer without a script: in an API's Assert tab each row checks its status, a header, a value a JSONPath finds, the body, the time it took or its size — equals, contains, matches a pattern, exists, more or less than, is of a type. Each row is checked against the response shown as you edit it, with what was found when it fails, and the status line says how many held; a click there opens the tab.

## Cookies

Kept from every response and sent with the requests they match, across projects; search, add, edit, delete or clear them.

## cURL and imports

Paste a cURL command to make an API, copy any request as cURL, HTTPie, JavaScript `fetch`, Python `requests` or Go `net/http`; import Postman, Insomnia, Hoppscotch (a collection, an "export all" or an environment), Bruno (a `.bru` request or environment file, or an OpenCollection YAML export), Swagger 2 / OpenAPI 3 (JSON or YAML), a HAR file from a browser's developer tools (its calls, without the images, scripts and styles a page loads, a folder for each host) and cURL into a project, or as a new project from the projects page, with how they sign in (Basic, Bearer, API key, Digest) and their environments (a Postman or Hoppscotch environment file, Insomnia's sub-environments or an OpenCollection's environments as named environments; Bruno's `{{process.env.NAME}}` as `{{env(NAME)}}`; an OpenAPI document's security schemes as the project's auth, values for you to fill) — pasted, from a file, or fetched from an address (the web version's through its server, where requests may go); importing again adds only what is new — into the folders of the same name — or also updates the APIs the project has, the same method to the same path, keeping how they sign in, their assertions and settings; export an API, a folder or a project — a folder or a project also as a Postman collection (v2.1) or an OpenAPI 3 document (YAML: the address's variable as its servers, bodies as examples with a schema read off them, how they sign in but never the values).

## Your data

Back up and restore on the desktop, in the same format as the earlier Tauri app.

## Teams

A self-hosted web version — one binary or a Docker image, at the root or under a sub-path — with accounts by invitation, teams, and per-project roles (viewer, editor, manager); share a project with accounts or teams, or move it to another team. The desktop app signs in to the same server — listing the team's projects as they come — uploads a project of its own there, keeping its history, and works on shared projects offline, syncing when you choose and asking whose version to keep when someone else changed it meanwhile; APIs and folders can stay local, out of the sync.

## AI assistants

The web version is also an MCP server: Claude Code, Cursor or another MCP client signs in with a token you issue and reads your projects, APIs, variables and responses, with secrets masked; a token you allow to may also send saved APIs.
