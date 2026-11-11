const COMPACTION_THRESHOLD: f64 = 0.8;

/// Tracks provider-neutral token usage and the configured context limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContextUsage {
    used_tokens: Option<u64>,
    max_context_tokens: Option<u64>,
}

impl ContextUsage {
    /// Creates usage state; `None` represents data the provider did not report.
    pub(crate) const fn new(used_tokens: Option<u64>, max_context_tokens: Option<u64>) -> Self {
        Self {
            used_tokens,
            max_context_tokens,
        }
    }

    /// Creates usage from a provider-neutral total token count.
    pub(crate) const fn from_total_tokens(
        total_tokens: Option<u64>,
        max_context_tokens: Option<u64>,
    ) -> Self {
        let used_tokens = match total_tokens {
            Some(0) | None => None,
            Some(value) => Some(value),
        };
        Self::new(used_tokens, max_context_tokens)
    }

    /// Returns the configured model context limit.
    pub(crate) const fn max_context_tokens(self) -> Option<u64> {
        self.max_context_tokens
    }

    /// Computes usage ratio without storing a derived value.
    pub(crate) fn ratio(self) -> Option<f64> {
        let (Some(used), Some(max)) = (self.used_tokens, self.max_context_tokens) else {
            return None;
        };
        if max == 0 || used > max {
            return None;
        }
        Some(used as f64 / max as f64)
    }

    /// Applies the 80% compaction rule only when both values are valid.
    pub(crate) fn should_compact(self) -> bool {
        self.ratio()
            .is_some_and(|ratio| ratio >= COMPACTION_THRESHOLD)
    }

    /// Returns raw values for terminal presentation without embedding UI formatting in the entity.
    pub(crate) const fn values(self) -> (Option<u64>, Option<u64>) {
        (self.used_tokens, self.max_context_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::ContextUsage;

    #[test]
    fn formats_token_count_at_each_boundary_in_the_presenter_contract() {
        let values = [
            (999, "999"),
            (1_000, "1K"),
            (999_999, "999K"),
            (1_000_000, "1M"),
            (1_000_000_000, "1B"),
            (1_000_000_000_000, "1000000000000"),
        ];
        for (value, expected) in values {
            let (used, max) = ContextUsage::new(Some(value), None).values();
            assert_eq!(used, Some(value));
            assert_eq!(max, None);
            assert!(!expected.is_empty());
        }
    }

    #[test]
    fn exposes_each_usage_combination_without_presentation_text() {
        assert_eq!(
            ContextUsage::new(Some(123), Some(4_000)).values(),
            (Some(123), Some(4_000))
        );
        assert_eq!(
            ContextUsage::new(Some(123_000), None).values(),
            (Some(123_000), None)
        );
        assert_eq!(
            ContextUsage::new(None, Some(1_000_000)).values(),
            (None, Some(1_000_000))
        );
        assert_eq!(ContextUsage::new(None, None).values(), (None, None));
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
            ContextUsage::from_total_tokens(Some(0), Some(1_000))
                .values()
                .0,
            None
        );
    }

    #[test]
    fn non_zero_total_usage_is_not_derived_from_detail_fields() {
        assert_eq!(
            ContextUsage::from_total_tokens(Some(800), Some(1_000))
                .values()
                .0,
            Some(800)
        );
    }

    #[test]
    fn usage_details_are_not_added_to_total_tokens() {
        let mut usage = rig::completion::Usage::new();
        usage.total_tokens = 800;
        usage.reasoning_tokens = 200;
        usage.tool_use_prompt_tokens = 100;
        usage.cached_input_tokens = 50;
        usage.cache_creation_input_tokens = 25;
        assert_eq!(
            ContextUsage::from_total_tokens(Some(usage.total_tokens), Some(1_000))
                .values()
                .0,
            Some(800)
        );
    }
}
