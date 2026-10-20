//! Checks every backend makes the same way.

use crate::StoreError;

/// A project's or an item's name: required, kept without surrounding blanks.
pub fn required_name(name: &str) -> Result<String, StoreError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(StoreError::NameRequired);
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_required() {
        assert_eq!(required_name("  users  "), Ok("users".to_string()));
        assert_eq!(required_name(" \t"), Err(StoreError::NameRequired));
    }
}
