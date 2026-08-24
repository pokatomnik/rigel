//! Shared model-facing vocabulary for built-in tools.
//!
//! Descriptions and errors use short, literal model-facing vocabulary.
//! Recoverable errors name one corrective action and use one code from
//! [`error_codes`].

use serde::Serialize;

pub(crate) mod error_codes {
    pub(crate) const INVALID_ARGUMENT: &str = "INVALID_ARGUMENT";
    pub(crate) const IO_ERROR: &str = "IO_ERROR";
    pub(crate) const NETWORK_ERROR: &str = "NETWORK_ERROR";
    pub(crate) const TIMEOUT: &str = "TIMEOUT";
    pub(crate) const SUBAGENT_ERROR: &str = "SUBAGENT_ERROR";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Completed,
    Fetched,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_values_are_stable_snake_case_strings() {
        let actions = [Action::Completed, Action::Fetched];
        let values = actions
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>();

        assert_eq!(
            values.ok(),
            Some(
                vec!["\"completed\"", "\"fetched\""]
                    .into_iter()
                    .map(String::from)
                    .collect()
            )
        );
    }
}
