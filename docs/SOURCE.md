[中文](./SOURCE_zh.md) | English

# Sync from a source

A project or a folder can remember the address its document lives at, and take that document in again later. This is for a bound source. A one-off import that updates matching APIs keeps more of what you set; that is [Import](./IMPORT.md). Back to [Features](./FEATURES.md).

## Binding a source

The address is stored with the project, so any editor can change it.

- On import, **Remember source** when you fetch an address.
- On the home page, in the new-project or edit-project dialog.
- In a folder's settings, for that folder's own document.
- In the sync dialog, **Save address only**.

**Sync from source…** is in the tree header's **⋯** menu, the command palette, and a folder's menu. The dialog shows the address first. **Fetch** is what goes and gets it. An address on this machine or a private network is pointed out, because any editor can change it.

A folder's source matches APIs inside that folder only, so two services in sibling folders do not collide. A project's source matches the folder it was imported into, or the whole project when it was imported at the top.

## Preview

Fetching does not write yet. The dialog says what would be added, what would be updated, and which APIs the source no longer has. Those stay in the project.

A name the document changed counts as an update. Schema changes that remove a field, change a type, or add a required field are listed on their own. A name change alone is not one of those. You confirm, and then it updates (*0.4.1*).

## What a sync writes

Matching is the same as [importing again](./IMPORT.md#where-a-request-matches): same method, same path. The first document entry that changes the API is the one applied. An open tab's unsaved edit is what the merge starts from.

Only the parameter values you typed, and auth secrets you already filled in, stay off the document. Everything else follows it.

Stays yours:

- A parameter's value, a form field's value, and a JSON value where the kind still matches. A `{{template}}` may stand for any value.
- An auth secret, when the sign-in kind is still the same one: Basic or Digest username and password, a bearer token, an API key's value, and an OAuth client's id, secret, username and password. An empty secret takes the document's.
- Assertions, extractions and request settings you added.
- An example whose name the document does not have.

Follows the document:

- The API's name, when the document's name is not empty.
- Whether a parameter is on, its description (an empty one clears yours), and the spelling of its key. A parameter the document dropped goes.
- The API's own description, including clearing it when the document's is empty.
- How it signs in. A different kind replaces the auth you had. For the same kind, the API key's name and place, and the OAuth grant, client auth, redirect port, token URL, authorization URL, scope, audience and resource, come from the document.
- Fields. A JSON object's keys follow the document. A value of a different kind is the document's. An array is paired by index: extra elements you had go, extra elements in the document come in. A body that is not JSON, JSON that cannot be read, or a body of another content type, is the document's.
- Variables, path parameters included: the document's keys, order, switches and descriptions. A matching key keeps your value. A variable only you had goes.
- The document's examples, in the document's order. The same name (ignoring surrounding space) is the document's example.
- The document's imported schema checks, which replace the ones a previous sync imported. A check you wrote stays, and it does not block the document's schema check.
- The request schema, including when the document has none.

Nothing is written when the result is unchanged, and the project's revision does not move.

## On a schedule

Tick **Periodically auto-sync on this device** for a source you trust. The interval is 5, 15, 30 or 60 minutes. The fetch runs in the background and merges new and updated APIs with the rules above, on its own. Nothing is written when the document has not changed.

If someone changes the address, that schedule stops.

Auto-sync and the read-only field fetch below are one or the other.

## Field descriptions only

**Periodically fetch field descriptions on this device** does not write the project. It keeps a read-only copy in memory and, by method and path, shows input and output descriptions on the Docs tab of an API you built by hand or have not synced. When the source has something new or updated, it says so, and you can sync.
