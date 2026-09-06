const FOLLOW_UP_INSTRUCTIONS: &str = include_str!("./goal_follow_up.md");

pub(crate) fn goal_follow_up(goal: &str) -> String {
    format!(
        "{}\n\nOriginal goal:\n{goal}",
        FOLLOW_UP_INSTRUCTIONS.trim_end()
    )
}

#[cfg(test)]
mod tests {
    use super::goal_follow_up;

    #[test]
    fn follow_up_keeps_goal_and_completion_contract() {
        let prompt = goal_follow_up("inspect the repository");

        assert!(prompt.contains("inspect the repository"));
        assert!(prompt.contains("Continue working"));
        assert!(prompt.contains("mark_goal_complete"));
        assert!(prompt.contains("truthful, non-empty report"));
        assert!(prompt.contains("call `ask_user`"));
        assert!(!prompt.contains("Do not call `ask_user`"));
        assert!(prompt.contains("direct terminal input"));
    }
}
