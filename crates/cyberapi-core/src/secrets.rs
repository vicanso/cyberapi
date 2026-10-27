//! D3: secret variables. A project marks which of its variables are secret
//! (`ProjectSettings::secret_variables`); their values are each reader's
//! own ([`SecretValues`], kept beside the reader's settings) and never in
//! the project, its sync, an export or a backup — the variable's own value
//! stays empty. Teammates each fill in theirs. Sending fills them in
//! ([`fill`]); the request the history keeps has them masked
//! ([`mask_sent`]).

use crate::VariableId;
use crate::model::Variable;
use crate::response::SentRequest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a secret value shows as where it is not to be read.
pub const MASK: &str = "••••";

/// A value this short is not masked inside other text: it would mask too
/// much that is not it.
const SHORTEST: usize = 3;

/// A reader's values of a project's secret variables, by variable.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretValues(pub BTreeMap<VariableId, String>);

impl SecretValues {
    /// Those of the variables in `secret` alone: one no longer secret
    /// takes its own value again.
    pub fn of(&self, secret: &[VariableId]) -> Self {
        Self(
            self.0
                .iter()
                .filter(|(id, _)| secret.contains(id))
                .map(|(id, value)| (id.clone(), value.clone()))
                .collect(),
        )
    }

    pub fn get(&self, id: &VariableId) -> Option<&str> {
        self.0.get(id).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `variables` with the reader's values in place of the secret ones'.
pub fn fill(variables: Vec<Variable>, values: &SecretValues) -> Vec<Variable> {
    if values.is_empty() {
        return variables;
    }
    variables
        .into_iter()
        .map(|variable| match values.get(&variable.id) {
            Some(value) => Variable {
                value: value.to_string(),
                ..variable
            },
            None => variable,
        })
        .collect()
}

/// `text` with every secret value in it masked, the longest first so a
/// value inside another is not left half shown.
pub fn mask(text: &str, values: &SecretValues) -> String {
    let mut secrets: Vec<&str> = values
        .0
        .values()
        .map(String::as_str)
        .filter(|value| value.chars().count() >= SHORTEST)
        .collect();
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    secrets
        .into_iter()
        .fold(text.to_string(), |text, secret| text.replace(secret, MASK))
}

/// The request as sent, its secret values masked: its address, header
/// values and body.
pub fn mask_sent(sent: SentRequest, values: &SecretValues) -> SentRequest {
    if values.is_empty() {
        return sent;
    }
    SentRequest {
        url: mask(&sent.url, values),
        headers: sent
            .headers
            .into_iter()
            .map(|header| crate::response::SentHeader {
                value: mask(&header.value, values),
                ..header
            })
            .collect(),
        body: sent.body.map(|body| mask(&body, values)),
        ..sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::response::SentHeader;
    use crate::{CollectionId, model::VariableCategory};
    use pretty_assertions::assert_eq;

    fn variable(id: &str, value: &str) -> Variable {
        Variable {
            id: VariableId::new(id),
            collection: CollectionId::new("c"),
            category: VariableCategory::Custom,
            name: id.into(),
            value: value.into(),
            enabled: true,
            environment: None,
            created_at: Default::default(),
            updated_at: Default::default(),
        }
    }

    fn values(pairs: &[(&str, &str)]) -> SecretValues {
        SecretValues(
            pairs
                .iter()
                .map(|(id, value)| (VariableId::new(*id), value.to_string()))
                .collect(),
        )
    }

    #[test]
    fn the_readers_values_fill_the_secret_ones() {
        let filled = fill(
            vec![variable("token", ""), variable("host", "h")],
            &values(&[("token", "s3cret")]),
        );
        assert_eq!(filled[0].value, "s3cret");
        assert_eq!(filled[1].value, "h");
        // A variable no longer secret takes none.
        let kept = values(&[("token", "s3cret"), ("gone", "x")]);
        assert_eq!(kept.of(&[VariableId::new("token")]), values(&[("token", "s3cret")]));
    }

    #[test]
    fn what_is_kept_of_the_request_has_them_masked() {
        let secrets = values(&[("key", "abc"), ("long", "abc-123"), ("short", "x")]);
        assert_eq!(mask("k=abc-123&j=abc&x=1", &secrets), "k=••••&j=••••&x=1");
        let sent = SentRequest {
            url: "https://a.test/?key=abc-123".into(),
            headers: vec![SentHeader {
                name: "X-Key".into(),
                value: "abc".into(),
                from: Default::default(),
            }],
            body: Some(r#"{"k":"abc"}"#.into()),
            ..Default::default()
        };
        let masked = mask_sent(sent, &secrets);
        assert_eq!(masked.url, "https://a.test/?key=••••");
        assert_eq!(masked.headers[0].value, "••••");
        assert_eq!(masked.body.as_deref(), Some(r#"{"k":"••••"}"#));
    }
}
