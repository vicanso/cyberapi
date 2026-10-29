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

/// `existing` updated from `imported`: its method, address, query,
/// headers and body; how it signs in, its assertions and its settings stay.
/// A6: but for the assertions an import wrote, which are the import's
/// again — unless the reader checks the body against a schema of their
/// own, which stands for it.
pub fn updated(existing: &HttpRequest, imported: &HttpRequest) -> HttpRequest {
    let own_schema = existing
        .assertions
        .iter()
        .any(|assertion| assertion.subject == Subject::Schema && !assertion.imported);
    let mut assertions: Vec<Assertion> = existing
        .assertions
        .iter()
        .filter(|assertion| !assertion.imported)
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
        body: imported.body.clone(),
        query: imported.query.clone(),
        headers: imported.headers.clone(),
        assertions,
        ..existing.clone()
    }
}

/// What importing `items` into `parent` of `tree` again does; `update`
/// says whether the APIs it has already are updated.
pub fn merge(tree: &TreeData, parent: Option<&ItemId>, items: &[ImportItem], update: bool) -> Merge {
    let mut known: HashMap<(HttpMethod, String), &crate::model::ApiSetting> = HashMap::new();
    for api in &tree.apis {
        if let Some(key) = key(&api.request) {
            known.entry(key).or_insert(api);
        }
    }
    let mut merge = Merge::default();
    let mut walk = Walk {
        tree,
        known: &known,
        update,
        merge: &mut merge,
    };
    walk.at_place(parent, items);
    merge
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
    /// them: left as they are. A gRPC call is never among them.
    pub missing: Vec<ItemId>,
}

/// B7: what importing `items` into `parent` of `tree` again, updating what
/// it has, would do.
pub fn preview(tree: &TreeData, parent: Option<&ItemId>, items: &[ImportItem]) -> Preview {
    let merge = merge(tree, parent, items, true);
    let mut keys = std::collections::HashSet::new();
    let mut stack: Vec<&ImportItem> = items.iter().collect();
    while let Some(item) = stack.pop() {
        match item {
            ImportItem::Api { request, .. } => keys.extend(key(request)),
            ImportItem::Folder { children, .. } => stack.extend(children),
        }
    }
    let (mut had, mut missing) = (0, Vec::new());
    for api in &tree.apis {
        match key(&api.request) {
            Some(key) if keys.contains(&key) => had += 1,
            Some(_) => missing.push(api.id.clone()),
            None => {}
        }
    }
    Preview {
        added: merge.added(),
        updated: merge.updates.len(),
        same: had - merge.updates.len(),
        missing,
    }
}

struct Walk<'a> {
    tree: &'a TreeData,
    known: &'a HashMap<(HttpMethod, String), &'a crate::model::ApiSetting>,
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
        let Some(found) = key(request).and_then(|key| self.known.get(&key)) else {
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
}
