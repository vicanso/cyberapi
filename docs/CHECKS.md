[中文](./CHECKS_zh.md) | English

# Checks, docs and runs

How an API says what it returns, how a response is checked, and how a folder or a project is run. Back to [Features](./FEATURES.md).

## Docs

- The Docs tab is Markdown, with a switch between editing and preview.
- It lists input and output fields: path parameters, query, headers and form or multipart rows, plus nested fields taken from the request JSON Schema and from a response JSON Schema assertion. Each field shows its type, whether it is required, and its description.
- Path, query and header rows also show the type, required flag, format and enum recorded from OpenAPI or Swagger (*0.4.1*). A row that never recorded them leaves those blank, and blank is not optional. Form and file rows do not show them. The four cannot be edited.
- **Request schema…** and **Response schema…** in that tab write a schema, or infer one from the current JSON body or response.
- The JSON body toolbar has **Sample**, which fills a body from the request schema. When the body does not conform, the toolbar and the moment before send say so. Sending is not blocked.
- **Keep as example** in the history menu saves the response on screen. Open it in Docs to read the body.
- A folder's description is in the folder's settings.
- Descriptions, schemas and examples go with the project: teammates see them, an assistant reads them through MCP, and Postman and OpenAPI exports carry them (the description, the request and response schemas, saved responses and response examples). They come in from Postman (saved responses included), Insomnia and OpenAPI.
- A server project also has a read-only page at `GET /v1/docs/<project-id>` (**Online documentation** in the project menu; on the web, also in the project card's menu, including for a viewer. Desktop project cards do not have it). Open it with a session or `?token=cat_…`. It renders the folder tree, descriptions, field tables, examples and code snippets in several languages, with secrets masked. The parameter table adds Type, Required, Format and Enum (*0.4.1*). Required is `required` or `optional` when it was recorded, and blank when it was not. A column that is empty for every parameter of that API is left out. When every path starts with the same `{{variable}}`, the page and the code samples write it as `BASE_URL`, and each environment's address is listed under the project description. Two different variables keep their names.
- On that page each sidebar folder collapses, shows how many APIs it holds, and stays hidden while a filter matches nothing inside it. Collapse and Expand apply to every folder. `/` or Ctrl/Cmd-K focuses the filter; Escape clears it. Copy path copies the path, Copy URL adds the selected base, and the code sample and the `#` beside the title can be copied too. When every path shares one `{{variable}}` and an environment stores a real address for it, the switcher at the top replaces `BASE_URL` in the samples. An empty optional query parameter is left out of those samples; a required one stays even with an empty value, and the parameter table still lists both. A schema table longer than 25 fields starts with the first 20. The response is `private, no-cache` and carries an ETag of the project revision, so an unchanged project can answer `304`.

## Assertions

No script. In the Assert tab each row checks one thing against the response on screen, and updates as you edit it.

- The subject is the status, a header, a value a JSONPath finds, the body, the time or the size.
- The check is equals, contains, matches a pattern, exists, greater or less than, or is of a type.
- A failure writes what was found. The status line says how many held. A click there opens the tab.
- An assistant that sends the API is told the same in that answer. The history does not keep those results (*0.3.1*).

## JSON Schema

A row can check that the body's JSON conforms to a JSON Schema (*0.4.1*).

- Write it in the dialog's editor. The default draft is 2020-12. A `$schema` picks another. A `$ref` stays inside the schema. Nothing is fetched.
- Where it does not conform, the first 3 places are listed.
- Importing OpenAPI or Swagger makes the schema of each operation's first 2xx JSON response such a row, marked as the import's. [Sync from a source](./SOURCE.md) replaces those imported rows with the document's. A check you wrote yourself stays beside it. Editing the imported row makes it yours: the next sync keeps that edit and brings the document's row back as well. Importing again by hand is different: your own schema check stands for the imported one, so the import's row is not added. See [Import](./IMPORT.md).
- Exporting OpenAPI writes this row as the response schema, in the OpenAPI 3.0 shape: `nullable`, exclusive bounds as flags, its own `$defs` written in. What 3.0 does not have, such as `if`, is left out.
- `cyberapi-cli run` checks it too, so a contract test can live in CI.

## Examples as checks

A row can require the answer to match one example kept on the API: the status and the body.

- JSON is compared by value. Spacing and key order do not matter. The row can list JSONPaths to ignore (a timestamp, an id). Those places are dropped from both sides before the comparison.
- Text is compared as written. Ignore paths do not apply to it.
- The example is the one saved on the API. Editing it changes the next check.

## Runs

**Run…** on a folder or a project sends its APIs one after another, in the tree's order.

- Set a delay between sends, how many times to retry a failure, and whether to stop at the first failure.
- Each send shows its answer and how its assertions held. The dialog tallies what held and what failed. With no assertions, an error status is a failure.
- Export the result as JUnit XML or a self-contained HTML report. Pick a send afterwards to open that API there.
- Give data — a CSV with a header row, or a JSON array of objects — pasted, or picked on the desktop. The APIs run once for each row. A column is `{{value(column)}}`.
- Under the assertions, **Extract** takes a value from the answer (a JSONPath, a header, the status) for later requests in the same run to read as `{{value(name)}}`. Log in once, then send the token.
- Those values belong to the run. Nothing is written to the project. What was sent is kept in each API's history, as any send is.
- In an environment marked as production, the run asks once before it starts.

## The command line

`cyberapi-cli run` is the same run without the window. How to install it, and `--env`, `--env-file`, `--var`, `--data` and `--prompt`, is in [Installing](./INSTALL.md#the-command-line).

```sh
cyberapi-cli run project.json --env staging --data users.csv --junit results.xml
```

- The file is CyberAPI's own export, or anything the app imports.
- Each send is printed as it ends. `--delay-ms`, `--retries` and `--bail` match the dialog. `--junit` writes JUnit XML. `--html` writes an HTML report.
- The exit code is non-zero when something did not hold.

## Load test

On the desktop, **Load test…** in an API's menu (*0.4.1*) sends that one API over and over.

- 1 to 50 senders at once. A number of times, up to 100,000, or a duration, up to 10 minutes.
- As it goes: sends per second, the error rate, the mean, the fastest and the slowest, P50 / P90 / P95 / P99, and how many of each status and error. No answer, or an error status, is a failure. Assertions are not checked.
- Variables, cookies and the token are read once at the start. `{{uuid()}}`, `{{random(…)}}`, `{{randomString(…)}}`, `{{now(…)}}` and `{{timestamp()}}` draw a new value on every send.
- Redirects are not followed. A Digest challenge is not answered. Nothing is written to the history, and cookies the answers set are not kept.
- In an environment marked as production, it asks first.
- The web version has none of this. Its sends go through the server, and an account may have only so many at once.
