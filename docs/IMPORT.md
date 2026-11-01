[中文](./IMPORT_zh.md) | English

# Import and export

Formats the app reads, what a second import keeps, and what an export writes. A project or folder that remembers its address and syncs later is [Sync from a source](./SOURCE.md). Back to [Features](./FEATURES.md).

## Formats

Paste, pick a file, or fetch an address (the web version fetches through its server, so the address has to be one requests may go to). From the tree header's **+**, or as a new project from the home page.

| Source | What comes in |
| --- | --- |
| CyberAPI | The app's own export. |
| Postman | A collection. Saved responses become examples. An environment file becomes named environments. Item variables come in. |
| Insomnia | Requests, folder auth, sub-environments. |
| Hoppscotch | One collection, an "export all", or an environment. |
| Bruno | A `.bru` request or environment, or an OpenCollection YAML export. `{{process.env.NAME}}` becomes `{{env(NAME)}}`. |
| Swagger 2 / OpenAPI 3 | JSON or YAML. Security schemes become the project's auth, values left for you. An operation's own scheme is written on that API. The first 2xx JSON response schema becomes an imported schema check. |
| HAR | Calls from a browser's developer tools. Images, scripts and styles a page loads are left out. One folder per host. |
| `.http` | VS Code REST Client and JetBrains HTTP Client (*0.4.1*). Requests parted by `###`. `@name = value` is a variable every environment shares. `http-client.env.json` or `rest-client.environmentVariables` become named environments. `{{$uuid}}`, `{{$timestamp}}`, `{{$randomInt 1 10}}`, `{{$processEnv NAME}}` and the like become this app's template functions. `# @prompt` asks as the request is sent. Response-handler scripts are left behind. `http-client.private.env.json` comes in as plain text, so keep it out of a project the team shares. |
| cURL | A pasted command becomes an API. |

Auth that has a matching kind comes along: Basic, Bearer, an API key, Digest, and OAuth 2.0 client credentials, password and authorization code (*0.3.1*). A kind the app does not have is left as "inherit".

If the project already signs in, that auth is kept. A named environment that is already there (names compared without case) only gains variables it did not have.

A line that is only an address is not a `.http` file. Fetch it instead.

## Where a request matches

Importing into a project counts the APIs already there. Two requests match when the method is the same and the path is the same.

- One leading `{{variable}}`, the scheme and host, the query, the fragment and a trailing slash are dropped before the path is compared.
- `{id}`, `:id` and `{{id}}` are the same placeholder. The rest of the path is case-sensitive.
- gRPC is not matched this way.
- If both addresses name a host in letters and those hosts differ, they are different APIs. A host that is still a variable can match.
- An API the project already has is not added again. When several requests in the file share that path, the first one that changes the API is the one applied.

## Updating what you already have

Importing again offers **Update those** (the default) or **Add only new ones**.

An update follows the import's address, parameters, path parameters and body fields. New ones come in. Ones the import dropped go. What you already set stays:

- A parameter's value, and whether it is switched on.
- A description the import leaves empty, on a parameter or on the API itself. A description the import does write replaces yours.
- The values inside a JSON body, where the field is still the same kind of value. A `{{template}}` may stand in for any value. An array you wrote stays whole. A body that is not plain JSON — comments, or a template where a value would be — stays as you wrote it (*0.4.1*). A non-JSON body of the same type stays. An empty body, or a body of another content type, is the import's.
- Path-parameter values. Your other variables stay, including ones the import does not list.
- The whole auth block, assertions you wrote, examples, extractions and the request's own settings.
- An imported schema check is replaced by the import's, unless you already check the body against a schema of your own. That one stands for it, and the import's schema row is not added.
- The request schema is the import's when the import has one, and yours when it does not.
- The name you gave the API stays.

New APIs go into folders of the same name. An environment variable that already has that name is not added again. An API you have open is rebuilt in the editor. A request the update does not change is not written.

## Adding only what is new

Choose this when the file should not touch APIs the project already has. Matching APIs stay as they are. The rest are added, into folders of the same name.

## Export

Export an API, a folder or a project as CyberAPI's own file.

A folder or a project can also be a Postman collection (v2.1) or OpenAPI 3 YAML:

- The variable in front of the address becomes `servers`.
- Bodies are examples, with a schema read off them.
- Auth is the kind only. Values are not written. OAuth 2.0 writes the flow, the token URL and the scopes. Authorization code also writes the authorization URL and the loopback callback (*0.3.1*).
- gRPC calls are not in the OpenAPI document.

## Copy as code

Any request can be copied as code. The menu groups the command line, scripting languages and compiled languages. cURL is first.

cURL, HTTPie, JavaScript `fetch`, Node.js axios, Python `requests`, PHP Guzzle, Go `net/http`, Rust reqwest, Java `HttpClient`, C# `HttpClient`, Swift `URLSession`.

Following redirects and checking certificates follow the settings. Digest is written in the library's own way where that library answers it. OAuth 2.0 writes the token you have now, or one line saying where to get one. gRPC is grpcurl. WebSocket is websocat.
