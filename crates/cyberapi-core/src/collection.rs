//! The home page's project list: keyword filter (F2.5), sort (F2.6) and
//! the reader's own groups.

use crate::CollectionId;
use crate::model::Collection;
use crate::search::matches_text_or_pinyin;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// How the home page orders projects. The names are the old app's setting
/// values, so a saved choice reads back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SortOrder {
    /// Most recently modified first.
    #[default]
    #[serde(rename = "lastModified")]
    LastModified,
    #[serde(rename = "nameAsc")]
    NameAsc,
    #[serde(rename = "nameDesc")]
    NameDesc,
    #[serde(rename = "olderFirst")]
    OldestFirst,
    #[serde(rename = "newestFirst")]
    NewestFirst,
}

/// The projects the keyword finds — in the name and description, by text or
/// pinyin — in the chosen order. Ties keep the input order.
pub fn filter_and_sort<'a>(collections: &'a [Collection], keyword: &str, order: SortOrder) -> Vec<&'a Collection> {
    let keyword = keyword.trim();
    let mut found: Vec<&Collection> = collections
        .iter()
        .filter(|collection| {
            keyword.is_empty()
                || matches_text_or_pinyin(
                    &format!(
                        "{}{}",
                        collection.name.to_lowercase(),
                        collection.description.to_lowercase()
                    ),
                    keyword,
                )
        })
        .collect();
    found.sort_by(|a, b| compare(a, b, order));
    found
}

/// A group of the project list, and its projects in the list's order.
#[derive(Debug, PartialEq)]
pub struct Group<'a> {
    /// `None` for the projects in no group.
    pub name: Option<String>,
    pub collections: Vec<&'a Collection>,
}

/// The projects the keyword finds, in the groups the reader put them in
/// (`groups`, a project's group by its id): the named groups by name, then
/// the projects in none. The chosen order holds inside each group; a group
/// the keyword leaves empty is not there.
pub fn grouped<'a>(
    collections: &'a [Collection],
    groups: &BTreeMap<CollectionId, String>,
    keyword: &str,
    order: SortOrder,
) -> Vec<Group<'a>> {
    let mut found: Vec<Group<'a>> = Vec::new();
    for collection in filter_and_sort(collections, keyword, order) {
        let name = group_of(groups, &collection.id);
        match found.iter_mut().find(|group| group.name.as_deref() == name) {
            Some(group) => group.collections.push(collection),
            None => found.push(Group {
                name: name.map(str::to_string),
                collections: vec![collection],
            }),
        }
    }
    found.sort_by(|a, b| match (&a.name, &b.name) {
        (Some(a), Some(b)) => by_text(a, b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    });
    found
}

/// The group a project is in: a name with something in it.
pub fn group_of<'a>(groups: &'a BTreeMap<CollectionId, String>, id: &CollectionId) -> Option<&'a str> {
    groups.get(id).map(|name| name.trim()).filter(|name| !name.is_empty())
}

/// Every group's name, once each, by name.
pub fn group_names(groups: &BTreeMap<CollectionId, String>) -> Vec<String> {
    let mut names: Vec<String> = groups
        .values()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect();
    names.sort_by(|a, b| by_text(a, b));
    names.dedup();
    names
}

fn compare(a: &Collection, b: &Collection, order: SortOrder) -> Ordering {
    match order {
        SortOrder::LastModified => b.updated_at.cmp(&a.updated_at),
        SortOrder::NameAsc => by_name(a, b),
        SortOrder::NameDesc => by_name(b, a),
        SortOrder::OldestFirst => a.created_at.cmp(&b.created_at),
        SortOrder::NewestFirst => b.created_at.cmp(&a.created_at),
    }
}

/// Names ignore case — the old app compared code units, so every capitalised
/// name came before every lower-case one — and fall back to the exact text
/// so the order is total.
fn by_name(a: &Collection, b: &Collection) -> Ordering {
    by_text(&a.name, &b.name)
}

fn by_text(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CollectionId, Timestamp};
    use pretty_assertions::assert_eq;

    fn collection(id: &str, name: &str, description: &str, created: &str, updated: &str) -> Collection {
        Collection {
            id: CollectionId::from(id),
            name: name.into(),
            description: description.into(),
            created_at: Timestamp::parse_lenient(created),
            updated_at: Timestamp::parse_lenient(updated),
            ..Default::default()
        }
    }

    fn ids(found: &[&Collection]) -> Vec<String> {
        found.iter().map(|c| c.id.to_string()).collect()
    }

    fn sample() -> Vec<Collection> {
        vec![
            collection("a", "Orders", "", "2022-01-01T00:00:00Z", "2022-03-01T00:00:00Z"),
            collection(
                "b",
                "用户中心",
                "accounts",
                "2022-02-01T00:00:00Z",
                "2022-01-15T00:00:00Z",
            ),
            collection(
                "c",
                "Billing",
                "支付",
                "2021-12-01T00:00:00+08:00",
                "2022-04-01T00:00:00Z",
            ),
        ]
    }

    #[test]
    fn default_order_is_last_modified_first() {
        let list = sample();
        assert_eq!(ids(&filter_and_sort(&list, "", SortOrder::default())), ["c", "a", "b"]);
    }

    #[test]
    fn every_order() {
        let list = sample();
        assert_eq!(ids(&filter_and_sort(&list, "", SortOrder::NameAsc)), ["c", "a", "b"]);
        assert_eq!(ids(&filter_and_sort(&list, "", SortOrder::NameDesc)), ["b", "a", "c"]);
        assert_eq!(
            ids(&filter_and_sort(&list, "", SortOrder::OldestFirst)),
            ["c", "a", "b"]
        );
        assert_eq!(
            ids(&filter_and_sort(&list, "", SortOrder::NewestFirst)),
            ["b", "a", "c"]
        );
    }

    #[test]
    fn names_ignore_case() {
        let list = vec![
            collection("upper", "IPLocation", "", "", ""),
            collection("lower", "browser", "", "", ""),
            collection("mixed", "ImageOptim", "", "", ""),
        ];
        assert_eq!(
            ids(&filter_and_sort(&list, "", SortOrder::NameAsc)),
            ["lower", "mixed", "upper"]
        );
        assert_eq!(
            ids(&filter_and_sort(&list, "", SortOrder::NameDesc)),
            ["upper", "mixed", "lower"]
        );
    }

    #[test]
    fn keyword_searches_name_and_description() {
        let list = sample();
        assert_eq!(ids(&filter_and_sort(&list, "yonghu", SortOrder::NameAsc)), ["b"]);
        assert_eq!(ids(&filter_and_sort(&list, "ACCOUNT", SortOrder::NameAsc)), ["b"]);
        assert_eq!(ids(&filter_and_sort(&list, "zf", SortOrder::NameAsc)), ["c"]);
        assert!(filter_and_sort(&list, "nothing", SortOrder::NameAsc).is_empty());
    }

    #[test]
    fn groups_by_name_then_the_rest_each_in_the_order() {
        let mut list = sample();
        list.push(collection(
            "d",
            "Docs",
            "",
            "2022-01-01T00:00:00Z",
            "2022-05-01T00:00:00Z",
        ));
        let groups: BTreeMap<CollectionId, String> = [
            (CollectionId::from("a"), "work".to_string()),
            (CollectionId::from("c"), "Home".to_string()),
            (CollectionId::from("d"), " work ".to_string()),
            // Blank is no group.
            (CollectionId::from("b"), "  ".to_string()),
        ]
        .into_iter()
        .collect();
        let shown = |keyword: &str| {
            grouped(&list, &groups, keyword, SortOrder::LastModified)
                .into_iter()
                .map(|group| (group.name, ids(&group.collections)))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shown(""),
            [
                (Some("Home".to_string()), vec!["c".to_string()]),
                (Some("work".to_string()), vec!["d".to_string(), "a".to_string()]),
                (None, vec!["b".to_string()]),
            ]
        );
        // A group the keyword empties is gone.
        assert_eq!(shown("orders"), [(Some("work".to_string()), vec!["a".to_string()])]);
        assert_eq!(group_names(&groups), ["Home", "work"]);
    }

    #[test]
    fn sort_order_reads_the_old_setting_values() {
        let order: SortOrder = serde_json::from_str(r#""olderFirst""#).expect("old value");
        assert_eq!(order, SortOrder::OldestFirst);
    }
}
