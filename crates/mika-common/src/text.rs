/// Truncate a string slice to at most `max_bytes`, rounding down to the
/// nearest char boundary. Never panics on multi-byte UTF-8.
///
/// Unlike `db::truncate_chars` (which counts characters and appends "..."),
/// this function preserves byte-budget semantics — the returned slice is
/// always `<= max_bytes` bytes long, with no suffix appended. Use this for
/// log line widths, prompt size budgets, and error message previews.
pub fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    let end = s.floor_char_boundary(s.len().min(max_bytes));
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_shorter_than_limit() {
        assert_eq!(safe_truncate("hello", 10), "hello");
    }

    #[test]
    fn ascii_longer_than_limit() {
        assert_eq!(safe_truncate("hello world", 5), "hello");
    }

    #[test]
    fn ascii_exact_limit() {
        assert_eq!(safe_truncate("hello", 5), "hello");
    }

    #[test]
    fn em_dash_inside_boundary() {
        // "abc" is 3 bytes, "\u{2014}" (em-dash) is bytes 3..6
        // Requesting 5 bytes: byte 5 is inside the em-dash, rounds down to 3
        assert_eq!(safe_truncate("abc\u{2014}def", 5), "abc");
    }

    #[test]
    fn em_dash_end_boundary() {
        // Requesting 6 bytes: byte 6 is exactly the end of the em-dash
        assert_eq!(safe_truncate("abc\u{2014}def", 6), "abc\u{2014}");
    }

    #[test]
    fn em_dash_start_boundary() {
        // Requesting 3 bytes: byte 3 is the start of the em-dash, valid boundary
        assert_eq!(safe_truncate("abc\u{2014}def", 3), "abc");
    }

    #[test]
    fn empty_string() {
        assert_eq!(safe_truncate("", 100), "");
    }

    #[test]
    fn zero_max_bytes() {
        assert_eq!(safe_truncate("hello", 0), "");
    }

    #[test]
    fn max_bytes_exceeds_length() {
        assert_eq!(safe_truncate("short", 1000), "short");
    }

    #[test]
    fn all_multibyte_chars() {
        // Each char is 3 bytes: \u{2014}=3, \u{2192}=3, \u{2501}=3 → total 9 bytes
        let s = "\u{2014}\u{2192}\u{2501}";
        assert_eq!(s.len(), 9);

        // Inside first char
        assert_eq!(safe_truncate(s, 1), "");
        assert_eq!(safe_truncate(s, 2), "");
        // End of first char
        assert_eq!(safe_truncate(s, 3), "\u{2014}");
        // Inside second char
        assert_eq!(safe_truncate(s, 4), "\u{2014}");
        assert_eq!(safe_truncate(s, 5), "\u{2014}");
        // End of second char
        assert_eq!(safe_truncate(s, 6), "\u{2014}\u{2192}");
        // Full string
        assert_eq!(safe_truncate(s, 9), s);
    }

    #[test]
    fn four_byte_emoji_at_boundary() {
        // "ab" = 2 bytes, 😀 = 4 bytes (U+1F600), "cd" = 2 bytes → total 8 bytes
        let s = "ab\u{1F600}cd";
        assert_eq!(s.len(), 8);

        // Inside emoji
        assert_eq!(safe_truncate(s, 3), "ab");
        assert_eq!(safe_truncate(s, 4), "ab");
        assert_eq!(safe_truncate(s, 5), "ab");
        // End of emoji
        assert_eq!(safe_truncate(s, 6), "ab\u{1F600}");
        // Full string
        assert_eq!(safe_truncate(s, 8), s);
    }
}
