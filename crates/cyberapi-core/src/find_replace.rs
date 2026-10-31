//! F15: project-wide find and replace across API URIs, query parameters,
//! headers, and bodies — previewed first, then applied via
//! `TreeStore::replace` and undoable like `undo::put_back` (F6).

use crate::ItemId;
use crate::model::{HttpMethod, HttpRequest, KvParam};
use crate::run::{self, Place};
use crate::send::form_fields;
use crate::sync::ProjectConfig;
use crate::tree::{Tree, TreeData};

/// Which parts of each API request are searched and replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindScope {
    pub uri: bool,
    pub query: bool,
    pub headers: bool,
    pub body: bool,
    pub case_sensitive: bool,
}

impl Default for FindScope {
    fn default() -> Self {
        Self {
            uri: true,
            query: true,
            headers: true,
            body: true,
            case_sensitive: true,
        }
    }
}

/// One API that matches the search and what its request becomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceHit {
    pub api: ItemId,
    pub place: Place,
    pub method: HttpMethod,
    pub count: usize,
    pub uri_matches: usize,
    pub query_matches: usize,
    pub header_matches: usize,
    pub body_matches: usize,
    pub before: HttpRequest,
    pub after: HttpRequest,
}

/// All APIs matched by a find-and-replace preview.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplacePlan {
    pub hits: Vec<ReplaceHit>,
    pub total_occurrences: usize,
}

impl ReplacePlan {
    /// The `(api_id, updated_request)` pairs to write when applying the plan.
    pub fn updates(&self) -> Vec<(ItemId, HttpRequest)> {
        self.hits
            .iter()
            .map(|hit| (hit.api.clone(), hit.after.clone()))
            .collect()
    }

    /// The `(api_id, original_request)` pairs to keep for undoing the plan.
    pub fn originals(&self) -> Vec<(ItemId, HttpRequest)> {
        self.hits
            .iter()
            .map(|hit| (hit.api.clone(), hit.before.clone()))
            .collect()
    }
}

/// Replaces all occurrences of `find` with `replace` in `text`, respecting
/// `case_sensitive`, and returns `(updated, occurrences)`.
fn replace_str(text: &str, find: &str, replace: &str, case_sensitive: bool) -> (String, usize) {
    if find.is_empty() || text.is_empty() {
        return (text.to_string(), 0);
    }
    if case_sensitive {
        let count = text.matches(find).count();
        if count == 0 {
            (text.to_string(), 0)
        } else {
            (text.replace(find, replace), count)
        }
    } else {
        let find_lower = find.to_lowercase();
        let find_chars: Vec<char> = find_lower.chars().collect();
        if find_chars.is_empty() {
            return (text.to_string(), 0);
        }
        let text_chars: Vec<(usize, char)> = text.char_indices().collect();
        let mut out = String::with_capacity(text.len());
        let mut count = 0;
        let mut i = 0;
        let mut last_byte = 0;
        while i + find_chars.len() <= text_chars.len() {
            let matches = text_chars[i..i + find_chars.len()]
                .iter()
                .map(|(_, ch)| ch.to_lowercase().next().unwrap_or(*ch))
                .eq(find_chars.iter().copied());
            if matches {
                let start_byte = text_chars[i].0;
                let end_byte = if i + find_chars.len() < text_chars.len() {
                    text_chars[i + find_chars.len()].0
                } else {
                    text.len()
                };
                out.push_str(&text[last_byte..start_byte]);
                out.push_str(replace);
                last_byte = end_byte;
                count += 1;
                i += find_chars.len();
            } else {
                i += 1;
            }
        }
        if count == 0 {
            (text.to_string(), 0)
        } else {
            out.push_str(&text[last_byte..]);
            (out, count)
        }
    }
}

fn replace_kv_list(list: &mut [KvParam], find: &str, replace: &str, case_sensitive: bool) -> usize {
    let mut total = 0;
    for kv in list {
        let (next_key, k_count) = replace_str(&kv.key, find, replace, case_sensitive);
        if k_count > 0 {
            kv.key = next_key;
            total += k_count;
        }
        let (next_val, v_count) = replace_str(&kv.value, find, replace, case_sensitive);
        if v_count > 0 {
            kv.value = next_val;
            total += v_count;
        }
    }
    total
}

/// Previews replacing `find` with `replace` across the APIs in `data` (under
/// `folder` when `Some`, or the whole project when `None`).
pub fn preview(data: &TreeData, folder: Option<&ItemId>, find: &str, replace: &str, scope: FindScope) -> ReplacePlan {
    if find.is_empty() || (!scope.uri && !scope.query && !scope.headers && !scope.body) {
        return ReplacePlan::default();
    }
    let tree = Tree::build(data);
    let ids = run::plan(data, folder);
    let mut hits = Vec::new();
    let mut total_occurrences = 0;

    for id in ids {
        let Some(api) = data.apis.iter().find(|api| api.id == id) else {
            continue;
        };
        let mut after = api.request.clone();
        let mut uri_matches = 0;
        let mut query_matches = 0;
        let mut header_matches = 0;
        let mut body_matches = 0;

        if scope.uri {
            let (next_uri, count) = replace_str(&after.uri, find, replace, scope.case_sensitive);
            if count > 0 {
                after.uri = next_uri;
                uri_matches = count;
            }
        }
        if scope.query {
            query_matches += replace_kv_list(&mut after.query, find, replace, scope.case_sensitive);
            query_matches += replace_kv_list(&mut after.variables, find, replace, scope.case_sensitive);
        }
        if scope.headers {
            header_matches += replace_kv_list(&mut after.headers, find, replace, scope.case_sensitive);
        }
        if scope.body {
            if after.content_type.is_key_value() {
                let mut fields = form_fields(&after.body);
                let count = replace_kv_list(&mut fields, find, replace, scope.case_sensitive);
                if count > 0 {
                    if let Ok(serialized) = serde_json::to_string(&fields) {
                        after.body = serialized;
                    }
                    body_matches = count;
                }
            } else {
                let (next_body, count) = replace_str(&after.body, find, replace, scope.case_sensitive);
                if count > 0 {
                    after.body = next_body;
                    body_matches = count;
                }
            }
        }

        let count = uri_matches + query_matches + header_matches + body_matches;
        if count > 0 {
            total_occurrences += count;
            hits.push(ReplaceHit {
                api: api.id.clone(),
                place: run::place(&tree, &api.id),
                method: api.request.method,
                count,
                uri_matches,
                query_matches,
                header_matches,
                body_matches,
                before: api.request.clone(),
                after,
            });
        }
    }

    ReplacePlan {
        hits,
        total_occurrences,
    }
}

/// Applies `updates` into `config` (used both for writing replacements and for
/// undoing them via `TreeStore::replace`). Returns `true` if any API in
/// `config` changed.
pub fn apply_requests(config: &mut ProjectConfig, updates: &[(ItemId, HttpRequest)]) -> bool {
    let mut changed = false;
    for (id, request) in updates {
        if let Some(api) = config.apis.iter_mut().find(|api| &api.id == id)
            && &api.request != request
        {
            api.request = request.clone();
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ApiSetting, ContentType};
    use crate::tree::fixture::sample;
    use pretty_assertions::assert_eq;

    #[test]
    fn preview_and_apply_replace_across_uri_headers_query_and_body() {
        let mut data = sample();
        let me = data
            .apis
            .iter_mut()
            .find(|api| api.id.as_str() == "me")
            .expect("me api");
        me.request.uri = "{{OLD_HOST}}/v1/users/me".into();
        me.request.query = vec![KvParam {
            key: "env".into(),
            value: "{{OLD_HOST}}".into(),
            enabled: true,
            description: String::new(),
        }];
        me.request.headers = vec![KvParam {
            key: "X-Origin".into(),
            value: "{{OLD_HOST}}".into(),
            enabled: true,
            description: String::new(),
        }];
        me.request.content_type = ContentType::Json;
        me.request.body = r#"{"callback": "{{OLD_HOST}}/hook"}"#.into();

        let plan = preview(&data, None, "{{OLD_HOST}}", "{{NEW_HOST}}", FindScope::default());
        assert_eq!(plan.hits.len(), 1);
        assert_eq!(plan.total_occurrences, 4);
        let hit = &plan.hits[0];
        assert_eq!(hit.uri_matches, 1);
        assert_eq!(hit.query_matches, 1);
        assert_eq!(hit.header_matches, 1);
        assert_eq!(hit.body_matches, 1);
        assert_eq!(hit.after.uri, "{{NEW_HOST}}/v1/users/me");
        assert_eq!(hit.after.query[0].value, "{{NEW_HOST}}");
        assert_eq!(hit.after.headers[0].value, "{{NEW_HOST}}");
        assert_eq!(hit.after.body, r#"{"callback": "{{NEW_HOST}}/hook"}"#);

        let mut config = ProjectConfig {
            folders: data.folders.clone(),
            apis: data.apis.clone(),
            top_order: data.top_order.clone(),
            settings: data.settings.clone(),
            ..Default::default()
        };
        assert!(apply_requests(&mut config, &plan.updates()));
        let updated_me: &ApiSetting = config.apis.iter().find(|api| api.id.as_str() == "me").expect("me");
        assert_eq!(updated_me.request.uri, "{{NEW_HOST}}/v1/users/me");

        // Undoing puts the originals back.
        assert!(apply_requests(&mut config, &plan.originals()));
        let restored_me: &ApiSetting = config.apis.iter().find(|api| api.id.as_str() == "me").expect("me");
        assert_eq!(restored_me.request.uri, "{{OLD_HOST}}/v1/users/me");
    }
}
