use rig::completion::Usage;

const COMPACTION_THRESHOLD: f64 = 0.8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContextUsage {
    used_tokens: Option<u64>,
    max_context_tokens: Option<u64>,
}

impl ContextUsage {
    pub(crate) const fn new(used_tokens: Option<u64>, max_context_tokens: Option<u64>) -> Self {
        Self {
            used_tokens,
            max_context_tokens,
        }
    }

    pub(crate) fn from_usage(usage: Usage, max_context_tokens: Option<u64>) -> Self {
        let used_tokens = (usage.total_tokens != 0).then_some(usage.total_tokens);
        Self::new(used_tokens, max_context_tokens)
    }

    pub(crate) const fn used_tokens(self) -> Option<u64> {
        self.used_tokens
    }

    pub(crate) const fn max_context_tokens(self) -> Option<u64> {
        self.max_context_tokens
    }

    pub(crate) fn ratio(self) -> Option<f64> {
        let (Some(used), Some(max)) = (self.used_tokens, self.max_context_tokens) else {
            return None;
        };
        if max == 0 || used > max {
            return None;
        }
        Some(used as f64 / max as f64)
    }

    pub(crate) fn should_compact(self) -> bool {
        self.ratio()
            .is_some_and(|ratio| ratio >= COMPACTION_THRESHOLD)
    }

    pub(crate) fn display_pair(self) -> (String, String) {
        (
            Self::format_token_count(self.used_tokens()),
            Self::format_token_count(self.max_context_tokens()),
        )
    }

    pub(crate) fn display_prompt(self) -> String {
        match (self.used_tokens, self.max_context_tokens) {
            (Some(used), Some(max)) => format!(
                "{}/{} > ",
                Self::format_token_count(Some(used)),
                Self::format_token_count(Some(max))
            ),
            (Some(used), None) => format!("{} > ", Self::format_token_count(Some(used))),
            (None, Some(max)) => format!("?/{} > ", Self::format_token_count(Some(max))),
            (None, None) => "> ".to_string(),
        }
    }

    fn format_token_count(value: Option<u64>) -> String {
        let Some(value) = value else {
            return "?".to_string();
        };
        if value < 1_000 {
            return value.to_string();
        }
        if value < 1_000_000 {
            return format!("{}K", value / 1_000);
        }
        if value < 1_000_000_000 {
            return format!("{}M", value / 1_000_000);
        }
        if value < 1_000_000_000_000 {
            return format!("{}B", value / 1_000_000_000);
        }
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use rig::completion::Usage;

    use super::ContextUsage;

    #[test]
    fn formats_token_count_at_each_boundary() {
        let values = [
            (999, "999"),
            (1_000, "1K"),
            (999_999, "999K"),
            (1_000_000, "1M"),
            (1_000_000_000, "1B"),
            (1_000_000_000_000, "1000000000000"),
        ];
        for (value, expected) in values {
            assert_eq!(
                ContextUsage::new(Some(value), None).display_pair().0,
                expected
            );
        }
    }

    #[test]
    fn formats_all_known_and_unknown_combinations() {
        assert_eq!(
            ContextUsage::new(Some(123), Some(4_000)).display_pair(),
            ("123".to_string(), "4K".to_string())
        );
        assert_eq!(
            ContextUsage::new(Some(123_000), Some(1_000_000)).display_pair(),
            ("123K".to_string(), "1M".to_string())
        );
        assert_eq!(
            ContextUsage::new(None, Some(1_000_000)).display_pair(),
            ("?".to_string(), "1M".to_string())
        );
        assert_eq!(
            ContextUsage::new(Some(123_000), None).display_pair(),
            ("123K".to_string(), "?".to_string())
        );
        assert_eq!(
            ContextUsage::new(None, None).display_pair(),
            ("?".to_string(), "?".to_string())
        );
    }

    #[test]
    fn formats_prompt_for_each_usage_combination() {
        assert_eq!(
            ContextUsage::new(Some(123), Some(4_000)).display_prompt(),
            "123/4K > "
        );
        assert_eq!(
            ContextUsage::new(Some(123_000), None).display_prompt(),
            "123K > "
        );
        assert_eq!(
            ContextUsage::new(None, Some(1_000_000)).display_prompt(),
            "?/1M > "
        );
        assert_eq!(ContextUsage::new(None, None).display_prompt(), "> ");
    }

    #[test]
    fn computes_ratio_when_called_without_storing_it() {
        assert!(!ContextUsage::new(Some(799), Some(1_000)).should_compact());
        let usage = ContextUsage::new(Some(800), Some(1_000));
        assert_eq!(usage.ratio(), Some(0.8));
        assert!(usage.should_compact());
        assert!(ContextUsage::new(Some(801), Some(1_000)).should_compact());
    }

    #[test]
    fn refuses_to_compact_when_context_data_is_unknown_or_invalid() {
        assert!(!ContextUsage::new(None, Some(1_000)).should_compact());
        assert!(!ContextUsage::new(Some(800), None).should_compact());
        assert!(!ContextUsage::new(Some(1_001), Some(1_000)).should_compact());
        assert!(!ContextUsage::new(Some(800), Some(0)).should_compact());
    }

    #[test]
    fn zero_total_usage_is_unknown() {
        assert_eq!(
            ContextUsage::from_usage(Usage::new(), Some(1_000)).used_tokens(),
            None
        );
    }

    #[test]
    fn usage_details_are_not_added_to_total_tokens() {
        let mut usage = Usage::new();
        usage.total_tokens = 800;
        usage.reasoning_tokens = 200;
        usage.tool_use_prompt_tokens = 100;
        usage.cached_input_tokens = 50;
        usage.cache_creation_input_tokens = 25;

        assert_eq!(
            ContextUsage::from_usage(usage, Some(1_000)).used_tokens(),
            Some(800)
        );
    }
}
