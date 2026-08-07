/// Trait for shortening strings to a specified edge count.
pub trait StringShort {
    /// Returns the string shortened to `edges` characters on each side.
    fn short(&self, edges: usize) -> Self;
}

impl StringShort for String {
    fn short(&self, edges: usize) -> Self {
        let char_count = self.chars().count();
        if char_count <= edges.saturating_mul(2) || edges == 0 {
            return self.clone();
        }

        let Some((prefix_end, _)) = self.char_indices().nth(edges) else {
            return self.clone();
        };
        let Some((suffix_start, _)) = self.char_indices().nth(char_count - edges) else {
            return self.clone();
        };
        format!("{}...{}", &self[..prefix_end], &self[suffix_start..])
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
    fn test_string_short_preserves_unicode_boundaries() {
        let s = String::from("привет мир");
        assert_eq!(s.short(3), "при...мир");
    }

    #[test]
    fn test_string_short_with_max_edges() {
        let s = String::from("привет");
        assert_eq!(s.short(usize::MAX), "привет");
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
}
