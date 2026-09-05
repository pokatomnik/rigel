//! Shared model-facing vocabulary for built-in tools.
//!
//! Descriptions and errors use short, literal model-facing vocabulary.
//! Recoverable errors name one corrective action and use one code from
//! [`error_codes`].

use serde::Serialize;

pub mod error_codes;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Completed,
    Fetched,
    Read,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_values_are_stable_snake_case_strings() {
        let actions = [Action::Completed, Action::Fetched, Action::Read];
        let values = actions
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>();

        assert_eq!(
            values.ok(),
            Some(
                vec!["\"completed\"", "\"fetched\"", "\"read\""]
                    .into_iter()
                    .map(String::from)
                    .collect()
            )
        );
    }
}
