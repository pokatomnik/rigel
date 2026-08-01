pub(crate) fn summarization() -> &'static str {
    include_str!("./summarization.md")
}

#[cfg(test)]
mod tests {
    use super::summarization;

    #[test]
    fn summarization_prompt_is_not_empty() {
        assert!(!summarization().trim().is_empty());
    }

    #[test]
    fn summarization_prompt_defines_the_compression_task() {
        let prompt = summarization();

        assert!(prompt.contains("compresses a long conversation"));
        assert!(prompt.contains("completely replace the message history"));
        assert!(prompt.contains("Limit the summary to 5–7 sentences"));
    }

    #[test]
    fn summarization_prompt_structures_the_output_sections() {
        let prompt = summarization();

        for section in ["FACTS:", "DECISIONS:", "PLAN / TASKS:", "QUESTIONS:"] {
            assert!(prompt.contains(section), "missing output section {section}");
        }
    }
}
