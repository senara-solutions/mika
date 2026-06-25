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

/// Truncate prose to at most `max_bytes`, preferring to end at the last
/// sentence boundary (`.` `!` `?`) within the budget. Falls back to
/// `safe_truncate` (char-boundary) if no sentence boundary exists or if
/// the best boundary would utilize less than 50% of the byte budget.
///
/// The 50% minimum-utilization floor prevents pathological cases where the
/// only sentence boundary is near the start of the string — returning 50
/// bytes of a 2000-byte budget silently discards context, which is worse
/// for LLM quality than a mid-sentence cut.
///
/// `\n` is intentionally excluded from the sentence-boundary set. Markdown
/// documents use hard-wrapped lines within sentences; a trailing `\n` from
/// line wrapping is not a reliable sentence boundary.
///
/// Use for LLM-prompt-bound truncation where mid-sentence cuts degrade
/// prompt quality. For log lines and error previews, use `safe_truncate`.
pub fn truncate_at_semantic_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let limit = s.floor_char_boundary(s.len().min(max_bytes));
    let min_utilization = max_bytes / 2;
    // Scan backwards for sentence-ending punctuation only (no \n)
    if let Some(end) = s[..limit].rfind(['.', '!', '?']) {
        // Include the sentence-ending character
        let boundary = end + s[end..].chars().next().map_or(0, |c| c.len_utf8());
        if boundary >= min_utilization {
            return &s[..boundary];
        }
    }
    // Fallback: char boundary (same as safe_truncate)
    safe_truncate(s, max_bytes)
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

    // -----------------------------------------------------------------------
    // truncate_at_semantic_boundary tests
    // -----------------------------------------------------------------------

    #[test]
    fn semantic_shorter_than_limit() {
        assert_eq!(truncate_at_semantic_boundary("hello.", 100), "hello.");
    }

    #[test]
    fn semantic_truncates_at_period() {
        let s = "First sentence. Second sentence that is longer.";
        // Budget 20: "First sentence. Sec" (19 bytes) — period at byte 15
        // 15 >= 20/2=10, so sentence boundary wins
        assert_eq!(truncate_at_semantic_boundary(s, 20), "First sentence.");
    }

    #[test]
    fn semantic_truncates_at_exclamation() {
        let s = "Watch out! This is a much longer continuation of the text.";
        // Budget 15: "Watch out! This" — exclamation at byte 10
        // 10 >= 15/2=7, so boundary wins
        assert_eq!(truncate_at_semantic_boundary(s, 15), "Watch out!");
    }

    #[test]
    fn semantic_truncates_at_question_mark() {
        let s = "Is this working? Let me check the rest of the output.";
        // Budget 20: "Is this working? Let" — question at byte 16
        // 16 >= 20/2=10, so boundary wins
        assert_eq!(truncate_at_semantic_boundary(s, 20), "Is this working?");
    }

    #[test]
    fn semantic_no_sentence_boundary_falls_back() {
        let s = "No sentence boundary here, just a long clause that keeps going";
        assert_eq!(truncate_at_semantic_boundary(s, 20), safe_truncate(s, 20));
    }

    #[test]
    fn semantic_newline_not_treated_as_boundary() {
        // Only \n boundaries — should fall back to safe_truncate
        let s = "Line one\nLine two\nLine three\nLine four continues here";
        assert_eq!(truncate_at_semantic_boundary(s, 25), safe_truncate(s, 25));
    }

    #[test]
    fn semantic_min_utilization_floor_rejects() {
        // Sentence boundary at byte 5 ("Done."), budget 100 → 5 < 100/2=50 → fallback
        let s = "Done. Then a very long continuation without any more sentence endings that goes on and on and on and on";
        assert_eq!(truncate_at_semantic_boundary(s, 100), safe_truncate(s, 100));
    }

    #[test]
    fn semantic_min_utilization_floor_accepts() {
        // Sentence boundary at ~55 bytes, budget 100 → 55 >= 50 → boundary wins
        let s = "This is a sentence that ends right around the middle here. Then more text follows after that point and keeps going";
        let result = truncate_at_semantic_boundary(s, 100);
        assert!(result.ends_with('.'));
        assert!(result.len() >= 50);
        assert!(result.len() <= 100);
    }

    #[test]
    fn semantic_multibyte_near_boundary() {
        // "Caf\u{00e9}." is 6 bytes (3 + 2 + 1), then " More text follows here"
        let s = "Caf\u{00e9}. More text follows here and continues on";
        // Budget 10: floor_char_boundary(10) = 10
        // rfind('.') at byte 5, boundary = 6
        // 6 >= 10/2=5, so sentence boundary wins
        assert_eq!(truncate_at_semantic_boundary(s, 10), "Caf\u{00e9}.");
    }

    #[test]
    fn semantic_empty_string() {
        assert_eq!(truncate_at_semantic_boundary("", 100), "");
    }

    #[test]
    fn semantic_zero_budget() {
        assert_eq!(truncate_at_semantic_boundary("hello.", 0), "");
    }
}
