//! Shared model-facing vocabulary for built-in tools.
//!
//! Descriptions and errors use `current directory` for the startup directory.
//! Descriptions are short, literal, and state the operation plus its path or
//! argument constraints. Success responses use one action value; collection
//! responses use `items`, `count`, and `truncated`. Recoverable errors name one
//! corrective action and use one code from [`error_codes`].

use serde::Serialize;

#[allow(dead_code)]
pub(crate) const CURRENT_DIRECTORY: &str = "current directory";

#[allow(dead_code)]
pub(crate) mod error_codes {
    pub(crate) const INVALID_ARGUMENT: &str = "INVALID_ARGUMENT";
    pub(crate) const PATH_NOT_FOUND: &str = "PATH_NOT_FOUND";
    pub(crate) const PATH_ALREADY_EXISTS: &str = "PATH_ALREADY_EXISTS";
    pub(crate) const PATH_OUTSIDE_CURRENT_DIRECTORY: &str = "PATH_OUTSIDE_CURRENT_DIRECTORY";
    pub(crate) const INVALID_PATH_TYPE: &str = "INVALID_PATH_TYPE";
    pub(crate) const PERMISSION_DENIED: &str = "PERMISSION_DENIED";
    pub(crate) const REVISION_CHANGED: &str = "REVISION_CHANGED";
    pub(crate) const TEXT_NOT_FOUND: &str = "TEXT_NOT_FOUND";
    pub(crate) const TEXT_NOT_UNIQUE: &str = "TEXT_NOT_UNIQUE";
    pub(crate) const BINARY_FILE: &str = "BINARY_FILE";
    pub(crate) const USER_REFUSED: &str = "USER_REFUSED";
    pub(crate) const IO_ERROR: &str = "IO_ERROR";
    pub(crate) const NETWORK_ERROR: &str = "NETWORK_ERROR";
    pub(crate) const TIMEOUT: &str = "TIMEOUT";
    pub(crate) const SUBAGENT_ERROR: &str = "SUBAGENT_ERROR";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Created,
    Updated,
    Deleted,
    Moved,
    Unchanged,
    NotFound,
    Completed,
    Fetched,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CollectionEnvelope<T> {
    pub(crate) items: Vec<T>,
    pub(crate) count: usize,
    pub(crate) truncated: bool,
}

impl<T> CollectionEnvelope<T> {
    pub(crate) fn new(items: Vec<T>, truncated: bool) -> Self {
        Self {
            count: items.len(),
            items,
            truncated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_values_are_stable_snake_case_strings() {
        let actions = [
            Action::Created,
            Action::Updated,
            Action::Deleted,
            Action::Moved,
            Action::Unchanged,
            Action::NotFound,
            Action::Completed,
            Action::Fetched,
        ];
        let values = actions
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>();

        assert_eq!(
            values.ok(),
            Some(
                vec![
                    "\"created\"",
                    "\"updated\"",
                    "\"deleted\"",
                    "\"moved\"",
                    "\"unchanged\"",
                    "\"not_found\"",
                    "\"completed\"",
                    "\"fetched\"",
                ]
                .into_iter()
                .map(String::from)
                .collect()
            )
        );
    }

    #[test]
    fn collection_envelope_counts_returned_items() {
        let envelope = CollectionEnvelope::new(vec!["a", "b"], true);

        assert_eq!(envelope.items, vec!["a", "b"]);
        assert_eq!(envelope.count, 2);
        assert!(envelope.truncated);
    }
}
