//! The home page's project list: keyword filter (F2.5) and sort (F2.6).

use crate::model::Collection;
use crate::search::matches_text_or_pinyin;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

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

fn compare(a: &Collection, b: &Collection, order: SortOrder) -> Ordering {
    match order {
        SortOrder::LastModified => b.updated_at.cmp(&a.updated_at),
        SortOrder::NameAsc => a.name.cmp(&b.name),
        SortOrder::NameDesc => b.name.cmp(&a.name),
        SortOrder::OldestFirst => a.created_at.cmp(&b.created_at),
        SortOrder::NewestFirst => b.created_at.cmp(&a.created_at),
    }
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
    fn keyword_searches_name_and_description() {
        let list = sample();
        assert_eq!(ids(&filter_and_sort(&list, "yonghu", SortOrder::NameAsc)), ["b"]);
        assert_eq!(ids(&filter_and_sort(&list, "ACCOUNT", SortOrder::NameAsc)), ["b"]);
        assert_eq!(ids(&filter_and_sort(&list, "zf", SortOrder::NameAsc)), ["c"]);
        assert!(filter_and_sort(&list, "nothing", SortOrder::NameAsc).is_empty());
    }

    #[test]
    fn sort_order_reads_the_old_setting_values() {
        let order: SortOrder = serde_json::from_str(r#""olderFirst""#).expect("old value");
        assert_eq!(order, SortOrder::OldestFirst);
    }
}
