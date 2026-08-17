#![allow(dead_code)]

pub(crate) fn subagent_prompt() -> &'static str {
    include_str!("./subagent.md")
}

pub(crate) fn task_prompt(task: String) -> String {
    let report_contract = subagent_prompt();
    format!("{task}\n\n{report_contract}")
}

#[cfg(test)]
mod tests {
    use super::{subagent_prompt, task_prompt};

    #[test]
    fn report_prompt_is_not_empty() {
        assert!(!subagent_prompt().trim().is_empty());
    }

    #[test]
    fn task_prompt_keeps_task_before_report_contract() {
        let prompt = task_prompt("inspect the repository".to_string());

        assert!(prompt.starts_with("inspect the repository"));
        for section in [
            "SUBAGENT REPORT",
            "STATUS:",
            "SUMMARY:",
            "ACTIONS:",
            "TOOL CALLS:",
            "CHANGES:",
            "VERIFICATION:",
            "ISSUES:",
            "NEXT STEPS:",
        ] {
            assert!(prompt.contains(section), "missing report section {section}");
        }
    }
}
