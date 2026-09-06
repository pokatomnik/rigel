use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Completed,
    Edited,
    Fetched,
    Matched,
    Read,
    Searched,
    Written,
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
            Action::Matched,
            Action::Read,
            Action::Searched,
            Action::Written,
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
                    "\"matched\"",
                    "\"read\"",
                    "\"searched\"",
                    "\"written\"",
                ]
                .into_iter()
                .map(String::from)
                .collect()
            )
        );
    }
}
