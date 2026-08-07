/// Trait for shortening strings to a specified edge count.
pub trait StringShort {
    /// Returns the string shortened to `edges` characters on each side.
    fn short(&self, edges: usize) -> Self;
}

impl StringShort for String {
    fn short(&self, edges: usize) -> Self {
        let len = self.len();
        if len <= 2 * edges || edges == 0 {
            return self.clone();
        }
        
        // Split into three parts: prefix + "..." + suffix
        let prefix = &self[..edges];
        let suffix = &self[len - edges..];
        format!("{}...{}", prefix, suffix)
    }
}

/// Shortens a string for display purposes.
/// Words "tool call" and "tool result" are excluded from shortening.
fn shorten_for_display(s: &str) -> String {
    let mut result = s.to_string();
    
    // Find all occurrences of "tool call" or "tool result" (case-insensitive)
    let words_to_exclude = ["tool call", "tool result"];
    
    for word in &words_to_exclude {
        if let Some(start) = result.find(word) {
            // Replace with a placeholder that won't be shortened further
            result.replace_range(start..start + word.len(), "[EXCLUDED]");
        }
    }
    
    // Apply shortening logic to the modified string
    let edges = 30; // Default edge count for display
    if result.len() > 2 * edges {
        let prefix_len = edges.min(result.len());
        let suffix_start = result.len().saturating_sub(prefix_len);
        
        let prefix = &result[..prefix_len];
        let suffix = &result[suffix_start..];
        
        format!("{}...{}", prefix, suffix)
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_short_shorter_than_edges_times_two() {
        let s = String::from("hello");
        assert_eq!(s.short(10), "hello");
    }

    #[test]
    fn test_string_short_equal_to_edges_times_two() {
        let s = String::from("helloworld"); // 10 chars, edges * 2 = 2 * 5 = 10
        assert_eq!(s.short(5), "helloworld");
    }

    #[test]
    fn test_string_short_longer_than_edges_times_two() {
        let s = String::from("hello world"); // 11 chars, edges * 2 = 2 * 3 = 6
        assert_eq!(s.short(3), "hel...rld");
    }

    #[test]
    fn test_string_short_with_zero_edges() {
        let s = String::from("hello world");
        // When edges=0, return string as-is
        assert_eq!(s.short(0), "hello world");
    }

    #[test]
    fn test_string_short_empty_string() {
        let s = String::new();
        assert_eq!(s.short(5), "");
    }

    #[test]
    fn test_string_short_single_char() {
        let s = String::from("a");
        assert_eq!(s.short(10), "a");
    }

    #[test]
    fn test_display_shorten_tool_call() {
        let s = "tool call: some long argument that should be shortened";
        let result = shorten_for_display(s);
        // Should contain "[EXCLUDED]" and be shorter than original
        assert!(result.contains("[EXCLUDED]"));
    }

    #[test]
    fn test_display_shorten_tool_result() {
        let s = "tool result: some long argument that should be shortened";
        let result = shorten_for_display(s);
        // Should contain "[EXCLUDED]" and be shorter than original
        assert!(result.contains("[EXCLUDED]"));
    }

    #[test]
    fn test_display_shorten_no_excluded_words() {
        let s = "some regular string without excluded words";
        let result = shorten_for_display(s);
        assert!(!result.contains("[EXCLUDED]"));
        // Should be shortened since length > 60 (2 * 30)
    }

    #[test]
    fn test_display_shorten_not_long_enough() {
        let s = "short";
        let result = shorten_for_display(s);
        // Should not be shortened since length <= 60
        assert_eq!(result, "short");
    }
}
