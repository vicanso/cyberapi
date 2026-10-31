//! Importing again (B3): an API the project has already — the same method
//! to the same path — is updated or left as it is rather than added a
//! second time, and what is new goes into the folder of the same name the
//! project has, rather than a second folder.

use super::ImportItem;
use crate::ItemId;
use crate::assertions::{Assertion, Subject};
use crate::model::{HttpMethod, HttpRequest};
use crate::tree::TreeData;
use std::collections::HashMap;

/// What importing again does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Merge {
    /// What is new, and the folder it goes in (`None` the top level): where
    /// the import was asked to go, or a folder of the project an imported
    /// one is named as.
    pub additions: Vec<(Option<ItemId>, Vec<ImportItem>)>,
    /// The project's APIs the import has, each with what updating makes of
    /// its request; none when they are left as they are.
    pub updates: Vec<(ItemId, HttpRequest)>,
    /// How many of the import's APIs the project has already.
    pub matched: usize,
}

impl Merge {
    /// How many APIs and folders are added.
    pub fn added(&self) -> (usize, usize) {
        let bundle = super::ImportBundle {
            items: self.additions.iter().flat_map(|(_, items)| items.clone()).collect(),
            ..Default::default()
        };
        bundle.counts()
    }
}

/// A path as two APIs are told the same by: no variable before it, no
/// scheme and host, no query; each parameter — `{id}`, `:id`, `{{id}}` —
/// alike; no slash at its end.
pub fn path_key(uri: &str) -> String {
    let mut rest = uri.trim();
    if rest.starts_with("{{")
        && let Some(end) = rest.find("}}")
    {
        rest = &rest[end + 2..];
    }
    if let Some(at) = rest.find("://") {
        rest = &rest[at + 3..];
        rest = rest.find('/').map_or("", |slash| &rest[slash..]);
    }
    let path = rest.split(['?', '#']).next().unwrap_or_default();
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let parameter = (segment.starts_with('{') && segment.ends_with('}')) || segment.starts_with(':');
            if parameter { "{}" } else { segment }
        })
        .collect();
    format!("/{}", segments.join("/"))
}

/// What an API is matched by; a gRPC call is matched by nothing (its
/// address names a server, not a method).
fn key(request: &HttpRequest) -> Option<(HttpMethod, String)> {
    (request.method != HttpMethod::Grpc).then(|| (request.method, path_key(&request.uri)))
}

/// #84: resolves a leading `{{NAME}}` in `uri` from `primary` (or `fallback`
/// when `primary` lacks `NAME`).
pub fn resolve_with_vars(uri: &str, primary: &[(String, String)], fallback: &[(String, String)]) -> String {
    let uri = uri.trim();
    match crate::model::split_env_prefix(uri) {
        (Some(name), rest) => primary
            .iter()
            .chain(fallback.iter())
            .find(|(key, val)| key == name && !val.trim().is_empty())
            .map(|(_, val)| format!("{}{rest}", val.trim()))
            .unwrap_or_else(|| uri.to_string()),
        (None, _) => uri.to_string(),
    }
}

/// #84: the normalized `host[:port]` of a resolved URI, or `None` when it has
/// no literal host (a relative `/path` or an unresolved `{{VAR}}/path`).
pub fn host_key(resolved_uri: &str) -> Option<String> {
    let trimmed = resolved_uri.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') || trimmed.starts_with("{{") {
        return None;
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let parsed = url::Url::parse(&with_scheme).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if host.contains('{') || host.contains('}') {
        return None;
    }
    Some(match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

/// #84: whether `existing_uri` (resolved with `existing_vars`) and
/// `imported_uri` (resolved with `imported_vars`, falling back to
/// `existing_vars`) point to compatible hosts: `false` when both resolve to
/// literal hosts and those hosts differ.
pub fn hosts_compatible(
    existing_uri: &str,
    imported_uri: &str,
    existing_vars: &[(String, String)],
    imported_vars: &[(String, String)],
) -> bool {
    let existing_resolved = resolve_with_vars(existing_uri, existing_vars, &[]);
    let imported_resolved = resolve_with_vars(imported_uri, imported_vars, existing_vars);
    match (host_key(&existing_resolved), host_key(&imported_resolved)) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

/// A description as updating takes it: the import's, or the reader's when
/// the import gives none (F10).
fn kept_description(had: &str, given: &str) -> String {
    if given.trim().is_empty() {
        had.to_string()
    } else {
        given.to_string()
    }
}

/// The list as the import has it — its keys, in its order; a key it no
/// longer has goes — with what the reader set of a key it had kept: its
/// value and whether it is on. A description the import gives updates the
/// reader's; one it gives none for leaves it (F10).
fn merged_params(
    existing: &[crate::model::KvParam],
    imported: &[crate::model::KvParam],
    ignore_case: bool,
) -> Vec<crate::model::KvParam> {
    let same = |a: &str, b: &str| {
        if ignore_case {
            a.trim().eq_ignore_ascii_case(b.trim())
        } else {
            a.trim() == b.trim()
        }
    };
    imported
        .iter()
        .map(|kv| match existing.iter().find(|had| same(&had.key, &kv.key)) {
            Some(had) => crate::model::KvParam {
                value: had.value.clone(),
                enabled: had.enabled,
                description: kept_description(&had.description, &kv.description),
                ..kv.clone()
            },
            None => kv.clone(),
        })
        .collect()
}

/// The API's own variables after updating (F12 / D5): a path parameter
/// both addresses have keeps the reader's value, its description updated
/// as a parameter's is; one only the new address has comes from the
/// import; one only the old address had goes. The reader's other
/// variables stay.
fn merged_variables(existing: &HttpRequest, imported: &HttpRequest) -> Vec<crate::model::KvParam> {
    let old_names = crate::model::path_param_names(&existing.uri);
    let new_names = crate::model::path_param_names(&imported.uri);
    let mut variables: Vec<crate::model::KvParam> = existing
        .variables
        .iter()
        .filter(|kv| {
            let key = kv.key.trim();
            !old_names.iter().any(|name| name == key) || new_names.iter().any(|name| name == key)
        })
        .map(|kv| {
            match imported
                .variables
                .iter()
                .find(|given| given.key.trim() == kv.key.trim())
            {
                Some(given) => crate::model::KvParam {
                    description: kept_description(&kv.description, &given.description),
                    ..kv.clone()
                },
                None => kv.clone(),
            }
        })
        .collect();
    for given in &imported.variables {
        if !variables.iter().any(|kv| kv.key.trim() == given.key.trim()) {
            variables.push(given.clone());
        }
    }
    variables
}

/// Whether a JSON value the reader wrote stands where the import has
/// `given`: the same kind of value, or a template, which may stand for any.
fn same_kind(had: &serde_json::Value, given: &serde_json::Value) -> bool {
    use serde_json::Value;
    matches!(
        (had, given),
        (Value::String(_), Value::String(_))
            | (Value::Number(_), Value::Number(_))
            | (Value::Bool(_), Value::Bool(_))
            | (_, Value::Null)
    ) || matches!(had, Value::String(text) if text.contains("{{"))
}

/// A JSON body as the import has it — its fields, in its order; a field it
/// no longer has goes — with the reader's values where it has the same
/// field with the same kind of value. A list the reader wrote stays whole.
fn merged_json(had: &serde_json::Value, given: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match (had, given) {
        (Value::Object(had), Value::Object(given)) => Value::Object(
            given
                .iter()
                .map(|(key, value)| {
                    let value = match had.get(key) {
                        Some(mine) => merged_json(mine, value),
                        None => value.clone(),
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        (Value::Array(_), Value::Array(_)) => had.clone(),
        _ if same_kind(had, given) => had.clone(),
        _ => given.clone(),
    }
}

/// The body after updating. Form and multipart fields merge as the query
/// does. A JSON body takes the import's fields with the reader's values
/// (`merged_json`); one either side does not read as JSON — comments, a
/// template in place of a value — stays the reader's. Another body of the
/// same type stays the reader's unless it is empty; a body of another type
/// is the import's.
fn merged_body(existing: &HttpRequest, imported: &HttpRequest) -> String {
    if existing.content_type.is_key_value() && imported.content_type.is_key_value() {
        let fields = crate::send::form_fields(&imported.body);
        let kept = merged_params(&crate::send::form_fields(&existing.body), &fields, false);
        return if kept == fields {
            imported.body.clone()
        } else {
            serde_json::to_string(&kept).unwrap_or_else(|_| imported.body.clone())
        };
    }
    if existing.content_type != imported.content_type || existing.body.trim().is_empty() {
        return imported.body.clone();
    }
    if imported.content_type != crate::model::ContentType::Json {
        return existing.body.clone();
    }
    let parsed = (
        serde_json::from_str::<serde_json::Value>(&existing.body),
        serde_json::from_str::<serde_json::Value>(&imported.body),
    );
    match parsed {
        (Ok(had), Ok(given)) => {
            let merged = merged_json(&had, &given);
            // Written as it was when nothing changes.
            if merged == had {
                existing.body.clone()
            } else {
                crate::format::json(&merged)
            }
        }
        _ => existing.body.clone(),
    }
}

/// `existing` updated from `imported`: the structure follows the import —
/// method, address, its query, headers, path parameters and body fields,
/// the request schema — while what the reader set is kept: a parameter's
/// value and whether it is on, a JSON body's values, a description the
/// import gives none for (`merged_params`, `merged_variables`,
/// `merged_body`). The import's description of the API updates the
/// reader's unless it is empty. How it signs in, its other assertions, its
/// settings, examples and extractions stay. A6: the assertions an import
/// wrote are the import's again — unless the reader checks the body
/// against a schema of their own, which stands for it. One marked imported
/// that checks something else now was made the reader's (BUG-11).
pub fn updated(existing: &HttpRequest, imported: &HttpRequest) -> HttpRequest {
    let the_imports = |assertion: &Assertion| assertion.imported && assertion.subject == Subject::Schema;
    let own_schema = existing
        .assertions
        .iter()
        .any(|assertion| assertion.subject == Subject::Schema && !assertion.imported);
    let mut assertions: Vec<Assertion> = existing
        .assertions
        .iter()
        .filter(|assertion| !the_imports(assertion))
        .cloned()
        .collect();
    if !own_schema {
        assertions.extend(
            imported
                .assertions
                .iter()
                .filter(|assertion| assertion.imported)
                .cloned(),
        );
    }
    HttpRequest {
        method: imported.method,
        uri: imported.uri.clone(),
        content_type: imported.content_type,
        body: merged_body(existing, imported),
        query: merged_params(&existing.query, &imported.query, false),
        headers: merged_params(&existing.headers, &imported.headers, true),
        variables: merged_variables(existing, imported),
        description: kept_description(&existing.description, &imported.description),
        assertions,
        request_schema: imported
            .request_schema
            .clone()
            .or_else(|| existing.request_schema.clone()),
        ..existing.clone()
    }
}

/// What importing `items` into `parent` of `tree` again does; `update`
/// says whether the APIs it has already are updated.
pub fn merge(tree: &TreeData, parent: Option<&ItemId>, items: &[ImportItem], update: bool) -> Merge {
    merge_with_vars(tree, parent, items, update, false, &[], &[])
}

/// B9: what syncing `items` into `folder` from that folder's own source does:
/// only APIs inside `folder` (at any depth) are matched, so another folder's
/// APIs at the same path stay untouched.
pub fn merge_folder(tree: &TreeData, folder: &ItemId, items: &[ImportItem], update: bool) -> Merge {
    merge_with_vars(tree, Some(folder), items, update, true, &[], &[])
}

/// #84: merges `items` into `parent` of `tree`, also checking that resolved
/// hosts do not conflict when `existing_vars` / `imported_vars` are known.
pub fn merge_with_vars(
    tree: &TreeData,
    parent: Option<&ItemId>,
    items: &[ImportItem],
    update: bool,
    within_parent: bool,
    existing_vars: &[(String, String)],
    imported_vars: &[(String, String)],
) -> Merge {
    let within = within_parent
        .then_some(parent)
        .flatten()
        .map(|folder| apis_in(tree, folder));
    let mut known: HashMap<(HttpMethod, String), Vec<&crate::model::ApiSetting>> = HashMap::new();
    for api in &tree.apis {
        if within.as_ref().is_some_and(|within| !within.contains(&api.id)) {
            continue;
        }
        if let Some(key) = key(&api.request) {
            known.entry(key).or_default().push(api);
        }
    }
    let mut merge = Merge::default();
    let mut walk = Walk {
        tree,
        known: &known,
        existing_vars,
        imported_vars,
        update,
        merge: &mut merge,
    };
    walk.at_place(parent, items);
    merge
}

/// #84: up to five matched APIs' method and full URLs (`METHOD existing →
/// imported` when the URL changes, with resolved addresses shown beside
/// `{{var}}` templates), so the import dialog shows which existing APIs would
/// be updated before importing.
pub fn matched_summaries(tree: &TreeData, items: &[ImportItem]) -> Vec<String> {
    matched_summaries_with_vars(tree, None, items, &[], &[])
}

pub fn matched_summaries_with_vars(
    tree: &TreeData,
    parent: Option<&ItemId>,
    items: &[ImportItem],
    existing_vars: &[(String, String)],
    imported_vars: &[(String, String)],
) -> Vec<String> {
    const LIMIT: usize = 5;
    let _ = parent;
    let mut known: HashMap<(HttpMethod, String), Vec<&crate::model::ApiSetting>> = HashMap::new();
    for api in &tree.apis {
        if let Some(key) = key(&api.request) {
            known.entry(key).or_default().push(api);
        }
    }
    fn display_uri(uri: &str, resolved: &str) -> String {
        let uri = uri.trim();
        let resolved = resolved.trim();
        if resolved != uri && !resolved.is_empty() {
            format!("{uri} ({resolved})")
        } else {
            uri.to_string()
        }
    }
    struct Context<'a> {
        known: &'a HashMap<(HttpMethod, String), Vec<&'a crate::model::ApiSetting>>,
        existing_vars: &'a [(String, String)],
        imported_vars: &'a [(String, String)],
        summaries: Vec<String>,
        total: usize,
    }
    fn walk(items: &[ImportItem], cx: &mut Context<'_>) {
        for item in items {
            match item {
                ImportItem::Api { request, .. } => {
                    let Some(candidates) = key(request).and_then(|k| cx.known.get(&k)) else {
                        continue;
                    };
                    let Some(found) = candidates.iter().copied().find(|api| {
                        hosts_compatible(&api.request.uri, &request.uri, cx.existing_vars, cx.imported_vars)
                    }) else {
                        continue;
                    };
                    cx.total += 1;
                    if cx.summaries.len() < LIMIT {
                        let existing_resolved = resolve_with_vars(&found.request.uri, cx.existing_vars, &[]);
                        let imported_resolved = resolve_with_vars(&request.uri, cx.imported_vars, cx.existing_vars);
                        let existing_shown = display_uri(&found.request.uri, &existing_resolved);
                        let imported_shown = display_uri(&request.uri, &imported_resolved);
                        let line = if existing_shown == imported_shown {
                            format!("{} {existing_shown}", found.request.method.as_str())
                        } else {
                            format!("{} {existing_shown} → {imported_shown}", found.request.method.as_str())
                        };
                        if !cx.summaries.contains(&line) {
                            cx.summaries.push(line);
                        }
                    }
                }
                ImportItem::Folder { children, .. } => walk(children, cx),
            }
        }
    }
    let mut cx = Context {
        known: &known,
        existing_vars,
        imported_vars,
        summaries: Vec::new(),
        total: 0,
    };
    walk(items, &mut cx);
    if cx.total > cx.summaries.len() && cx.summaries.len() == LIMIT {
        cx.summaries.push(format!("… (+{})", cx.total - cx.summaries.len()));
    }
    cx.summaries
}

/// B7: what syncing from a source would do, told before it is done.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    /// The APIs and folders added.
    pub added: (usize, usize),
    /// The project's APIs the source changes.
    pub updated: usize,
    /// The project's APIs the source has as they are.
    pub same: usize,
    /// The project's APIs the source does not have, as the project keeps
    /// them: left as they are. A gRPC call is never among them, nor one
    /// outside the folder the source went into (Q-2).
    pub missing: Vec<ItemId>,
}

/// The APIs in `folder`, at any depth.
fn apis_in(tree: &TreeData, folder: &ItemId) -> std::collections::HashSet<ItemId> {
    let mut found = std::collections::HashSet::new();
    let mut stack = vec![folder];
    while let Some(id) = stack.pop() {
        let Some(folder) = tree.folders.iter().find(|found| &found.id == id) else {
            continue;
        };
        for child in &folder.children {
            if tree.folders.iter().any(|folder| &folder.id == child) {
                stack.push(child);
            } else {
                found.insert(child.clone());
            }
        }
    }
    found
}

/// B10: indexes every HTTP operation in `items` by `(method, path_key(uri))`
/// (first occurrence wins, depth-first), for live read-only field matching.
pub fn index_requests(items: &[ImportItem]) -> HashMap<(HttpMethod, String), HttpRequest> {
    let mut map = HashMap::new();
    let mut stack: Vec<&ImportItem> = items.iter().rev().collect();
    while let Some(item) = stack.pop() {
        match item {
            ImportItem::Api { request, .. } => {
                if let Some(k) = key(request) {
                    map.entry(k).or_insert_with(|| (**request).clone());
                }
            }
            ImportItem::Folder { children, .. } => {
                stack.extend(children.iter().rev());
            }
        }
    }
    map
}

/// B10: finds the matching source `HttpRequest` for `api_id` (whose current
/// editor state is `request`) across `cached` source indexes (`url -> index`),
/// walking enclosing folders from innermost to outermost and finally falling
/// back to the project's bound source when its target folder is `None` or
/// encloses `api_id`.
pub fn matched_request<'a>(
    tree: &TreeData,
    api_id: &ItemId,
    request: &HttpRequest,
    cached: &'a HashMap<String, HashMap<(HttpMethod, String), HttpRequest>>,
) -> Option<&'a HttpRequest> {
    if cached.is_empty() {
        return None;
    }
    let k = key(request)?;
    let mut enclosing: Vec<&crate::model::Folder> = Vec::new();
    let mut child = api_id;
    while let Some(parent) = tree.folders.iter().find(|folder| folder.children.contains(child)) {
        if enclosing.iter().any(|seen| seen.id == parent.id) {
            break;
        }
        enclosing.push(parent);
        child = &parent.id;
    }
    for folder in &enclosing {
        if let Some(url) = folder.settings.source.as_deref()
            && let Some(found) = cached.get(url).and_then(|index| index.get(&k))
        {
            return Some(found);
        }
    }
    if let Some(project_source) = &tree.settings.source {
        let within_scope = match &project_source.folder {
            None => true,
            Some(target) => {
                !tree.folders.iter().any(|folder| &folder.id == target)
                    || enclosing.iter().any(|folder| &folder.id == target)
            }
        };
        if within_scope && let Some(found) = cached.get(&project_source.url).and_then(|index| index.get(&k)) {
            return Some(found);
        }
    }
    None
}

/// B7: what importing `items` into `parent` of `tree` again, updating what
/// it has, would do. What the source lacks is looked for where it went: in
/// `parent`, or the whole project when that is the top level (Q-2) — a
/// project's own APIs beside a spec's folder are not the spec's.
pub fn preview(tree: &TreeData, parent: Option<&ItemId>, items: &[ImportItem]) -> Preview {
    preview_scoped(tree, parent, items, false)
}

/// B9: what syncing a folder from its own source would do, matching and
/// counting within that folder's subtree alone.
pub fn preview_folder(tree: &TreeData, folder: &ItemId, items: &[ImportItem]) -> Preview {
    preview_scoped(tree, Some(folder), items, true)
}

fn preview_scoped(tree: &TreeData, parent: Option<&ItemId>, items: &[ImportItem], within_parent: bool) -> Preview {
    let merge = merge_with_vars(tree, parent, items, true, within_parent, &[], &[]);
    let mut keys = std::collections::HashSet::new();
    let mut stack: Vec<&ImportItem> = items.iter().collect();
    while let Some(item) = stack.pop() {
        match item {
            ImportItem::Api { request, .. } => keys.extend(key(request)),
            ImportItem::Folder { children, .. } => stack.extend(children),
        }
    }
    let within = parent.map(|folder| apis_in(tree, folder));
    let (mut had, mut missing) = (0usize, Vec::new());
    for api in &tree.apis {
        if within_parent && within.as_ref().is_some_and(|within| !within.contains(&api.id)) {
            continue;
        }
        match key(&api.request) {
            Some(key) if keys.contains(&key) => had += 1,
            Some(_) if within.as_ref().is_none_or(|within| within.contains(&api.id)) => missing.push(api.id.clone()),
            _ => {}
        }
    }
    Preview {
        added: merge.added(),
        updated: merge.updates.len(),
        same: had.saturating_sub(merge.updates.len()),
        missing,
    }
}

struct Walk<'a> {
    tree: &'a TreeData,
    known: &'a HashMap<(HttpMethod, String), Vec<&'a crate::model::ApiSetting>>,
    existing_vars: &'a [(String, String)],
    imported_vars: &'a [(String, String)],
    update: bool,
    merge: &'a mut Merge,
}

impl Walk<'_> {
    /// The folders directly in `parent` (the top level for `None`).
    fn folders_in(&self, parent: Option<&ItemId>) -> Vec<&crate::model::Folder> {
        match parent {
            Some(id) => {
                let children = self
                    .tree
                    .folders
                    .iter()
                    .find(|folder| &folder.id == id)
                    .map(|folder| folder.children.as_slice())
                    .unwrap_or_default();
                self.tree
                    .folders
                    .iter()
                    .filter(|folder| children.contains(&folder.id))
                    .collect()
            }
            None => self
                .tree
                .folders
                .iter()
                .filter(|folder| {
                    !self
                        .tree
                        .folders
                        .iter()
                        .any(|other| other.children.contains(&folder.id))
                })
                .collect(),
        }
    }

    /// Whether the project has the API already; counted, and updated when
    /// asked.
    fn known(&mut self, request: &HttpRequest) -> bool {
        let Some(candidates) = key(request).and_then(|key| self.known.get(&key)) else {
            return false;
        };
        let Some(found) = candidates
            .iter()
            .copied()
            .find(|api| hosts_compatible(&api.request.uri, &request.uri, self.existing_vars, self.imported_vars))
        else {
            return false;
        };
        self.merge.matched += 1;
        // Once each: the first of the import's that has it; none when the
        // import would leave it as it is.
        if self.update && !self.merge.updates.iter().any(|(id, _)| id == &found.id) {
            let request = updated(&found.request, request);
            if request != found.request {
                self.merge.updates.push((found.id.clone(), request));
            }
        }
        true
    }

    /// Items going into a folder the project has: a folder among them named
    /// as one there is merged into it.
    fn at_place(&mut self, parent: Option<&ItemId>, items: &[ImportItem]) {
        let mut adding = Vec::new();
        for item in items {
            match item {
                ImportItem::Api { request, .. } => {
                    if !self.known(request) {
                        adding.push(item.clone());
                    }
                }
                ImportItem::Folder {
                    name,
                    children,
                    settings,
                } => {
                    let same = self
                        .folders_in(parent)
                        .into_iter()
                        .find(|folder| folder.name.trim() == name.trim())
                        .map(|folder| folder.id.clone());
                    match same {
                        Some(id) => self.at_place(Some(&id), children),
                        None => {
                            let kept = self.new_items(children);
                            // A folder whose APIs are all known is not made.
                            if kept.is_empty() && !children.is_empty() {
                                continue;
                            }
                            adding.push(ImportItem::Folder {
                                name: name.clone(),
                                children: kept,
                                settings: settings.clone(),
                            });
                        }
                    }
                }
            }
        }
        if !adding.is_empty() {
            self.merge.additions.push((parent.cloned(), adding));
        }
    }

    /// Items going into a folder being made: the APIs the project has are
    /// left out.
    fn new_items(&mut self, items: &[ImportItem]) -> Vec<ImportItem> {
        let mut kept = Vec::new();
        for item in items {
            match item {
                ImportItem::Api { request, .. } => {
                    if !self.known(request) {
                        kept.push(item.clone());
                    }
                }
                ImportItem::Folder {
                    name,
                    children,
                    settings,
                } => {
                    let inner = self.new_items(children);
                    if inner.is_empty() && !children.is_empty() {
                        continue;
                    }
                    kept.push(ImportItem::Folder {
                        name: name.clone(),
                        children: inner,
                        settings: settings.clone(),
                    });
                }
            }
        }
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assertions::Assertion;
    use crate::auth::Auth;
    use crate::model::{ApiSetting, Folder, KvParam};
    use pretty_assertions::assert_eq;

    #[test]
    fn paths_are_told_apart_by_what_matters() {
        assert_eq!(path_key("{{HOST}}/v1/users/{id}?page=1"), "/v1/users/{}");
        assert_eq!(path_key("https://api.test/v1/users/:id/"), "/v1/users/{}");
        assert_eq!(path_key("/v1/users/{{id}}#top"), "/v1/users/{}");
        assert_eq!(path_key("{{HOST}}"), "/");
        assert_eq!(path_key("http://api.test"), "/");
        assert_ne!(path_key("/v1/users"), path_key("/v1/Users"), "a path's case counts");
    }

    fn request(method: HttpMethod, uri: &str) -> HttpRequest {
        HttpRequest {
            method,
            uri: uri.into(),
            ..Default::default()
        }
    }

    fn api(id: &str, method: HttpMethod, uri: &str) -> ApiSetting {
        ApiSetting {
            id: id.into(),
            name: id.into(),
            request: request(method, uri),
            ..Default::default()
        }
    }

    fn folder(id: &str, name: &str, children: &[&str]) -> Folder {
        Folder {
            id: id.into(),
            name: name.into(),
            children: children.iter().map(|child| ItemId::from(*child)).collect(),
            ..Default::default()
        }
    }

    fn imported_api(name: &str, method: HttpMethod, uri: &str) -> ImportItem {
        ImportItem::Api {
            name: name.into(),
            request: Box::new(request(method, uri)),
        }
    }

    fn imported_folder(name: &str, children: Vec<ImportItem>) -> ImportItem {
        ImportItem::Folder {
            name: name.into(),
            children,
            settings: Default::default(),
        }
    }

    /// A project with `users` holding GET /users and GET /users/{id}, and
    /// POST /login at the top.
    fn project() -> TreeData {
        TreeData {
            folders: vec![folder("f-users", "users", &["a-list", "a-one"])],
            apis: vec![
                api("a-list", HttpMethod::Get, "{{HOST}}/users"),
                api("a-one", HttpMethod::Get, "{{HOST}}/users/{id}"),
                api("a-login", HttpMethod::Post, "{{HOST}}/login"),
            ],
            top_order: vec!["f-users".into(), "a-login".into()],
            ..Default::default()
        }
    }

    /// The spec again: the same APIs, a new one in `users`, a new folder
    /// with one known API and one new.
    fn spec() -> Vec<ImportItem> {
        vec![
            imported_folder(
                "users",
                vec![
                    imported_api("list", HttpMethod::Get, "{{API}}/users?page=1"),
                    imported_api("one", HttpMethod::Get, "{{API}}/users/:userId"),
                    imported_api("make", HttpMethod::Post, "{{API}}/users"),
                ],
            ),
            imported_api("login", HttpMethod::Post, "{{API}}/login"),
            imported_folder(
                "orders",
                vec![
                    imported_api("again", HttpMethod::Get, "{{API}}/users"),
                    imported_api("orders", HttpMethod::Get, "{{API}}/orders"),
                ],
            ),
            imported_folder("known", vec![imported_api("login", HttpMethod::Post, "{{API}}/login")]),
        ]
    }

    #[test]
    fn what_the_project_has_is_not_added_again() {
        let merge = merge(&project(), None, &spec(), false);
        assert_eq!(merge.matched, 5);
        assert!(merge.updates.is_empty(), "left as they are");
        assert_eq!(
            merge.additions,
            vec![
                (
                    Some("f-users".into()),
                    vec![imported_api("make", HttpMethod::Post, "{{API}}/users")]
                ),
                (
                    None,
                    vec![imported_folder(
                        "orders",
                        vec![imported_api("orders", HttpMethod::Get, "{{API}}/orders")]
                    )]
                ),
            ],
            "into the folder of the same name; a folder whose APIs are all known is not made"
        );
        assert_eq!(merge.added(), (2, 1));
        assert_eq!(
            matched_summaries(&project(), &spec()),
            [
                "GET {{HOST}}/users → {{API}}/users?page=1",
                "GET {{HOST}}/users/{id} → {{API}}/users/:userId",
                "POST {{HOST}}/login → {{API}}/login",
                "GET {{HOST}}/users → {{API}}/users",
            ]
        );

        // #84: when both sides resolve to literal hosts, only APIs pointing to
        // the same host match, and summaries display the resolved URLs.
        let existing_vars = [("HOST".into(), "http://127.0.0.1:8765".into())];
        let same_host_vars = [("API".into(), "http://127.0.0.1:8765".into())];
        let other_host_vars = [("API".into(), "http://other.example:9999".into())];
        assert_eq!(
            merge_with_vars(&project(), None, &spec(), false, false, &existing_vars, &same_host_vars).matched,
            5
        );
        assert_eq!(
            matched_summaries_with_vars(&project(), None, &spec(), &existing_vars, &same_host_vars)[0],
            "GET {{HOST}}/users (http://127.0.0.1:8765/users) → {{API}}/users?page=1 (http://127.0.0.1:8765/users?page=1)"
        );
        assert_eq!(
            merge_with_vars(
                &project(),
                None,
                &spec(),
                false,
                false,
                &existing_vars,
                &other_host_vars
            )
            .matched,
            0,
            "a different resolved host is not matched as the same API"
        );
    }

    /// A6: the import's assertions are the import's again; the reader's
    /// stay, and a schema of their own stands for the import's.
    #[test]
    fn the_imports_assertions_are_updated() {
        let schema = |kind: &str| Assertion::imported_schema(&serde_json::json!({ "type": kind }));
        let own = Assertion {
            expected: "200".into(),
            ..Default::default()
        };
        let existing = HttpRequest {
            assertions: vec![own.clone(), schema("array")],
            ..Default::default()
        };
        let incoming = HttpRequest {
            assertions: vec![schema("object")],
            ..Default::default()
        };
        assert_eq!(
            updated(&existing, &incoming).assertions,
            [own.clone(), schema("object")]
        );
        let edited = Assertion {
            imported: false,
            ..schema("string")
        };
        let theirs = HttpRequest {
            assertions: vec![edited.clone()],
            ..Default::default()
        };
        assert_eq!(updated(&theirs, &incoming).assertions, [edited]);

        // BUG-11: one an import wrote that checks the status now is the
        // reader's, and stays.
        let made_theirs = Assertion {
            subject: Subject::Status,
            op: crate::assertions::Op::Equals,
            expected: "200".into(),
            ..schema("array")
        };
        let changed = HttpRequest {
            assertions: vec![made_theirs.clone()],
            ..Default::default()
        };
        assert_eq!(updated(&changed, &incoming).assertions, [made_theirs, schema("object")]);
    }

    #[test]
    fn updating_takes_the_request_and_keeps_what_is_the_apis_own() {
        let mut tree = project();
        tree.apis[0].request.auth = Auth::Bearer { token: "t".into() };
        tree.apis[0].request.assertions = vec![Assertion::default()];
        tree.apis[0].request.headers = vec![KvParam {
            key: "X-Mine".into(),
            value: "1".into(),
            enabled: true,
            ..Default::default()
        }];
        let merge = merge(&tree, None, &spec(), true);
        assert_eq!(merge.matched, 5);
        let ids: Vec<&str> = merge.updates.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["a-list", "a-one", "a-login"], "once each, by the first");
        let list = &merge.updates[0].1;
        assert_eq!(list.uri, "{{API}}/users?page=1");
        assert!(list.headers.is_empty(), "the import's headers");
        assert_eq!(list.auth, Auth::Bearer { token: "t".into() });
        assert_eq!(list.assertions.len(), 1);
    }

    #[test]
    fn a_grpc_call_and_a_place_inside_a_folder() {
        let mut tree = project();
        tree.apis.push(api("a-grpc", HttpMethod::Grpc, "localhost:50051"));
        tree.top_order.push("a-grpc".into());
        let items = vec![imported_api("call", HttpMethod::Grpc, "localhost:50051")];
        assert_eq!(merge(&tree, None, &items, true).additions, vec![(None, items.clone())]);
        // Into `users`: the API it has is known; a new one goes there.
        let items = vec![
            imported_api("list", HttpMethod::Get, "/users"),
            imported_api("make", HttpMethod::Post, "/users"),
        ];
        let merge = merge(&tree, Some(&"f-users".into()), &items, false);
        assert_eq!(merge.matched, 1);
        assert_eq!(merge.additions, vec![(Some("f-users".into()), vec![items[1].clone()])]);
    }

    #[test]
    fn an_api_the_import_leaves_as_it_is_is_not_updated() {
        let items = vec![imported_api("login", HttpMethod::Post, "{{HOST}}/login")];
        let merge = merge(&project(), None, &items, true);
        assert_eq!(merge.matched, 1);
        assert!(merge.updates.is_empty(), "nothing to write");
    }

    #[test]
    fn a_preview_counts_what_a_sync_does_and_names_what_the_source_lacks() {
        let mut tree = project();
        tree.apis.push(api("a-mine", HttpMethod::Delete, "{{HOST}}/cache"));
        tree.apis.push(api("a-grpc", HttpMethod::Grpc, "localhost:50051"));
        let items = vec![
            imported_folder(
                "users",
                vec![
                    imported_api("list", HttpMethod::Get, "{{HOST}}/users"),
                    imported_api("one", HttpMethod::Get, "{{API}}/users/:userId"),
                    imported_api("make", HttpMethod::Post, "{{API}}/users"),
                ],
            ),
            imported_api("login", HttpMethod::Post, "{{HOST}}/login"),
        ];
        assert_eq!(
            preview(&tree, None, &items),
            Preview {
                added: (1, 0),
                updated: 1,
                same: 2,
                missing: vec!["a-mine".into()],
            }
        );
    }

    /// Q-2: a source that went into a folder lacks only what is in it.
    #[test]
    fn a_source_in_a_folder_lacks_only_what_is_in_it() {
        let mut tree = project();
        // Written here by hand, beside the spec's folder.
        tree.apis.push(api("a-mine", HttpMethod::Delete, "{{HOST}}/cache"));
        tree.top_order.push("a-mine".into());
        // And one deeper in the spec's folder.
        tree.folders.push(folder("f-admin", "admin", &["a-ban"]));
        tree.folders[0].children.push("f-admin".into());
        tree.apis
            .push(api("a-ban", HttpMethod::Post, "{{HOST}}/users/{id}/ban"));
        let items = vec![imported_api("list", HttpMethod::Get, "{{API}}/users")];
        let missing = preview(&tree, Some(&"f-users".into()), &items).missing;
        assert_eq!(missing, ["a-one".into(), "a-ban".into()] as [ItemId; 2]);
        let missing = preview(&tree, None, &items).missing;
        assert_eq!(missing.len(), 4, "at the top level, the whole project: {missing:?}");
    }

    /// F10: a parameter description the reader wrote stays when the import
    /// gives none for that key, and is updated when the import gives one.
    #[test]
    fn parameter_descriptions_stay_or_update() {
        let kv = |key: &str, value: &str, desc: &str| {
            KvParam {
                key: key.into(),
                value: value.into(),
                enabled: true,
                ..Default::default()
            }
            .with_description(desc)
        };
        let existing = HttpRequest {
            content_type: crate::model::ContentType::Form,
            body: serde_json::to_string(&[kv("user", "a", "Login name"), kv("role", "dev", "Own note")]).expect("JSON"),
            query: vec![kv("page", "1", "Own page note"), kv("size", "20", "Old size")],
            headers: vec![kv("X-Trace", "1", "Correlation id")],
            ..Default::default()
        };
        let incoming = HttpRequest {
            content_type: crate::model::ContentType::Form,
            body: serde_json::to_string(&[kv("user", "b", "Account handle"), kv("role", "ops", "")]).expect("JSON"),
            query: vec![kv("page", "2", ""), kv("size", "50", "Items per page")],
            headers: vec![kv("x-trace", "2", "")],
            ..Default::default()
        };
        let merged = updated(&existing, &incoming);
        assert_eq!(merged.query[0].description, "Own page note");
        assert_eq!(merged.query[1].description, "Items per page");
        assert_eq!(merged.headers[0].description, "Correlation id");
        let fields = crate::send::form_fields(&merged.body);
        assert_eq!(fields[0].description, "Account handle");
        assert_eq!(fields[1].description, "Own note");
    }

    /// Updating follows the import's structure and keeps what the reader
    /// set: a parameter's value and whether it is on, a JSON body's
    /// values, path parameters' values; a parameter the import no longer
    /// has goes, a new one comes with the import's example; descriptions
    /// of the API and of its path parameters update.
    #[test]
    fn updating_keeps_the_readers_values_and_follows_the_imports_structure() {
        let kv = |key: &str, value: &str, enabled: bool, desc: &str| {
            KvParam {
                key: key.into(),
                value: value.into(),
                enabled,
                ..Default::default()
            }
            .with_description(desc)
        };
        let existing = HttpRequest {
            method: HttpMethod::Post,
            uri: "{{HOST}}/users/{id}/orders/{old}".into(),
            content_type: crate::model::ContentType::Json,
            body: "{\n  \"name\": \"mine\",\n  \"age\": \"{{value(age)}}\",\n  \"gone\": 1,\n  \"tags\": [\"a\", \"b\"],\n  \"kind\": 3\n}".into(),
            description: "Old summary".into(),
            query: vec![kv("page", "7", false, ""), kv("legacy", "x", true, "")],
            headers: vec![kv("x-trace", "mine", true, "")],
            variables: vec![
                kv("id", "42", true, "Old id"),
                kv("old", "9", true, ""),
                kv("token", "t", true, "my own variable"),
            ],
            ..Default::default()
        };
        let imported = HttpRequest {
            method: HttpMethod::Post,
            uri: "{{API}}/users/{id}/orders/{orderId}".into(),
            content_type: crate::model::ContentType::Json,
            body:
                "{\"name\": \"string\", \"age\": 0, \"tags\": [\"string\"], \"kind\": \"string\", \"email\": \"a@b\"}"
                    .into(),
            description: "New summary".into(),
            query: vec![
                kv("page", "1", true, "Page number"),
                kv("size", "20", true, "Items per page"),
            ],
            headers: vec![kv("X-Trace", "", true, "")],
            variables: vec![kv("id", "", true, "User id"), kv("orderId", "", true, "Order id")],
            ..Default::default()
        };
        let merged = updated(&existing, &imported);
        assert_eq!(merged.uri, imported.uri);
        assert_eq!(merged.description, "New summary");
        assert_eq!(
            merged.query,
            [
                kv("page", "7", false, "Page number"),
                kv("size", "20", true, "Items per page")
            ],
            "the reader's value and switch stay; a removed one goes, a new one comes"
        );
        assert_eq!(merged.headers, [kv("X-Trace", "mine", true, "")]);
        assert_eq!(
            merged.variables,
            [
                kv("id", "42", true, "User id"),
                kv("token", "t", true, "my own variable"),
                kv("orderId", "", true, "Order id"),
            ],
            "a path parameter the address no longer has goes; the reader's own variables stay"
        );
        let body: serde_json::Value = serde_json::from_str(&merged.body).expect("JSON");
        assert_eq!(
            body,
            serde_json::json!({
                "name": "mine",
                "age": "{{value(age)}}",
                "tags": ["a", "b"],
                "kind": "string",
                "email": "a@b"
            }),
            "values kept, a template stands for any kind, a changed kind and new fields from the import"
        );
        let order: Vec<&str> = body.as_object().expect("object").keys().map(String::as_str).collect();
        assert_eq!(order, ["name", "age", "tags", "kind", "email"], "the import's order");

        // An empty import description leaves the reader's; a body that is
        // no JSON stays as written; nothing changed is written as it was.
        let quiet = HttpRequest {
            description: String::new(),
            ..imported.clone()
        };
        assert_eq!(updated(&existing, &quiet).description, "Old summary");
        let templated = HttpRequest {
            body: "{\n  // note\n  \"id\": {{id}}\n}".into(),
            ..existing.clone()
        };
        assert_eq!(updated(&templated, &imported).body, templated.body);
        let same = updated(&existing, &imported);
        assert_eq!(updated(&same, &imported).body, same.body);
        assert_eq!(updated(&same, &imported), same, "a second update changes nothing");
    }

    /// B9: syncing a folder's own source matches and updates only APIs in
    /// that folder, leaving a sibling service's `/health` untouched.
    #[test]
    fn a_folder_syncs_its_own_source_apart_from_sibling_folders() {
        let tree = TreeData {
            folders: vec![
                folder("f-users", "users", &["a-users-health", "a-users-list"]),
                folder("f-orders", "orders", &["a-orders-list"]),
            ],
            apis: vec![
                api("a-users-health", HttpMethod::Get, "{{USERS}}/health"),
                api("a-users-list", HttpMethod::Get, "{{USERS}}/users"),
                api("a-orders-list", HttpMethod::Get, "{{ORDERS}}/orders"),
            ],
            top_order: vec!["f-users".into(), "f-orders".into()],
            ..Default::default()
        };
        let orders_spec = vec![
            imported_api("health", HttpMethod::Get, "{{ORDERS}}/health"),
            imported_api("orders", HttpMethod::Get, "{{ORDERS}}/orders?limit=20"),
        ];
        let folder_id = ItemId::from("f-orders");
        assert_eq!(
            preview_folder(&tree, &folder_id, &orders_spec),
            Preview {
                added: (1, 0),
                updated: 1,
                same: 0,
                missing: Vec::new(),
            }
        );
        let merged = merge_folder(&tree, &folder_id, &orders_spec, true);
        assert_eq!(merged.updates.len(), 1);
        assert_eq!(merged.updates[0].0, ItemId::from("a-orders-list"));
        assert_eq!(
            merged.additions,
            vec![(Some(folder_id), vec![orders_spec[0].clone()])],
            "orders gets its own /health without touching users'"
        );
    }

    /// B10: live source matching walks enclosing folders innermost-first and
    /// falls back to the project's source within its target folder scope.
    #[test]
    fn matched_request_prefers_the_innermost_bound_folder_then_the_project() {
        let mut users = folder("f-users", "users", &["f-admin", "a-user"]);
        users.settings.source = Some("https://users.test/openapi.json".into());
        let mut admin = folder("f-admin", "admin", &["a-admin-user"]);
        admin.settings.source = Some("https://admin.test/openapi.json".into());
        let orders = folder("f-orders", "orders", &["a-order"]);

        let tree = TreeData {
            folders: vec![users, admin, orders],
            apis: vec![
                api("a-user", HttpMethod::Get, "{{HOST}}/users/{id}"),
                api("a-admin-user", HttpMethod::Get, "{{HOST}}/users/{id}"),
                api("a-order", HttpMethod::Get, "{{HOST}}/orders/{id}"),
            ],
            top_order: vec!["f-users".into(), "f-orders".into()],
            settings: crate::model::ProjectSettings {
                source: Some(crate::model::ImportSource {
                    url: "https://project.test/openapi.json".into(),
                    folder: None,
                }),
                ..Default::default()
            },
        };

        let spec_with_desc = |uri: &str, desc: &str| {
            let mut req = request(HttpMethod::Get, uri);
            req.description = desc.into();
            index_requests(&[ImportItem::Api {
                name: "op".into(),
                request: Box::new(req),
            }])
        };

        let mut cached = HashMap::new();
        cached.insert(
            "https://users.test/openapi.json".to_string(),
            spec_with_desc("/users/:userId", "users-spec"),
        );
        cached.insert(
            "https://admin.test/openapi.json".to_string(),
            spec_with_desc("/users/{uid}", "admin-spec"),
        );
        cached.insert(
            "https://project.test/openapi.json".to_string(),
            spec_with_desc("/orders/{orderId}", "project-spec"),
        );

        let admin_api = &tree.apis[1];
        assert_eq!(
            matched_request(&tree, &admin_api.id, &admin_api.request, &cached).map(|r| r.description.as_str()),
            Some("admin-spec"),
            "innermost folder source wins over outer folder source"
        );
        let user_api = &tree.apis[0];
        assert_eq!(
            matched_request(&tree, &user_api.id, &user_api.request, &cached).map(|r| r.description.as_str()),
            Some("users-spec")
        );
        let order_api = &tree.apis[2];
        assert_eq!(
            matched_request(&tree, &order_api.id, &order_api.request, &cached).map(|r| r.description.as_str()),
            Some("project-spec"),
            "falls back to project source when folder has none"
        );
    }
}
