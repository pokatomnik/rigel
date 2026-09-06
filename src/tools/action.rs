use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Completed,
    Edited,
    Fetched,
    Read,
    Searched,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_values_are_stable_snake_case_strings() {
        let actions = [
            Action::Completed,
            Action::Edited,
            Action::Fetched,
            Action::Read,
            Action::Searched,
        ];
        let values = actions
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>();

        assert_eq!(
            values.ok(),
            Some(
                vec![
                    "\"completed\"",
                    "\"edited\"",
                    "\"fetched\"",
                    "\"read\"",
                    "\"searched\"",
                ]
                .into_iter()
                .map(String::from)
                .collect()
            )
        );
    }
}
