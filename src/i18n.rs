//! Which language the interface speaks (F15.2).
//!
//! rust-i18n keeps one current locale for the whole process, and
//! gpui-component reads the same one, so setting it here translates the
//! components' own strings (dialog buttons, input menus) along with ours. The
//! components spell Simplified Chinese `zh-CN`, so this app does too.

use cyberapi_store::StoreError;
use rust_i18n::t;

/// The languages this build has strings for. English is the fallback.
pub const LOCALES: [&str; 2] = ["en", "zh-CN"];

/// The locale to use: the reader's choice when this build has it, the
/// system's language otherwise, English last.
pub fn resolve(preferred: Option<&str>) -> &'static str {
    preferred
        .and_then(supported)
        .or_else(|| sys_locale::get_locale().as_deref().and_then(supported))
        .unwrap_or("en")
}

/// A BCP 47 tag (`zh-Hans-CN`, `en-US`, `zh_CN`) as one of [`LOCALES`].
fn supported(tag: &str) -> Option<&'static str> {
    let tag = tag.trim().to_ascii_lowercase().replace('_', "-");
    let [english, chinese] = LOCALES;
    if tag.starts_with("zh") {
        Some(chinese)
    } else if tag == "en" || tag.starts_with("en-") {
        Some(english)
    } else {
        None
    }
}

pub fn apply(preferred: Option<&str>) {
    rust_i18n::set_locale(resolve(preferred));
}

/// What to tell the reader about a store error.
pub fn store_error(error: &StoreError) -> String {
    use cyberapi_store::Entity;
    match error {
        StoreError::NotFound { what, .. } => match what {
            Entity::Collection => t!("errors.collection_not_found").into(),
            Entity::Folder | Entity::Api | Entity::Item => t!("errors.item_not_found").into(),
            Entity::Variable => t!("errors.variable_not_found").into(),
            Entity::Team => t!("errors.team_not_found").into(),
        },
        StoreError::Forbidden { .. } => t!("errors.forbidden").into(),
        StoreError::NameRequired => t!("errors.name_required").into(),
        StoreError::Tree { source } => t!("errors.tree", reason = source.to_string()).into(),
        StoreError::Backend { message } => t!("errors.backend", message = message).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_map_to_the_locales_this_build_has() {
        assert_eq!(supported("zh-Hans-CN"), Some("zh-CN"));
        assert_eq!(supported("zh_TW"), Some("zh-CN"));
        assert_eq!(supported("en-GB"), Some("en"));
        assert_eq!(supported("de-DE"), None);
        assert_eq!(resolve(Some("zh-CN")), "zh-CN");
    }

    /// Every dotted key of a locale file.
    fn keys(source: &str) -> std::collections::BTreeSet<String> {
        fn walk(table: &toml::Table, prefix: &str, keys: &mut std::collections::BTreeSet<String>) {
            for (key, value) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                match value {
                    toml::Value::Table(inner) => walk(inner, &path, keys),
                    _ => {
                        keys.insert(path);
                    }
                }
            }
        }
        let table: toml::Table = source.parse().expect("a locale file is TOML");
        let mut found = std::collections::BTreeSet::new();
        walk(&table, "", &mut found);
        found
    }

    #[test]
    fn every_locale_has_every_key() {
        // A missing key silently falls back to English; this makes it loud.
        let english = keys(include_str!("../locales/en.toml"));
        let chinese = keys(include_str!("../locales/zh-CN.toml"));
        let missing: Vec<_> = english.symmetric_difference(&chinese).collect();
        assert!(missing.is_empty(), "keys in one locale only: {missing:?}");
        assert_ne!(
            t!("dashboard.title", locale = "zh-CN"),
            t!("dashboard.title", locale = "en")
        );
    }
}
