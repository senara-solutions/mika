use unicode_width::UnicodeWidthChar;

/// A single character position within unicode-width-aware wrapped text.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WrappedChar {
    /// Display row (0-based, increments at each wrap break).
    pub display_row: usize,
    /// Display column within the current display row (visual width, not char count).
    pub display_col: usize,
    /// Character index within the line (0-based).
    pub char_idx: usize,
    /// Byte offset within the line string.
    pub byte_offset: usize,
    /// The character itself.
    #[allow(dead_code)]
    pub ch: char,
    /// The display width of this character.
    pub char_width: usize,
    /// Cumulative display width of all characters up to (but not including) this one.
    /// This equals the sum of `char_width` for all previously yielded characters.
    pub cumulative_width: usize,
}

/// Iterator that walks characters in a string and yields their wrapped display positions.
///
/// Uses the wrapping predicate: `col + ch_w > width && col > 0` -- the same algorithm
/// previously duplicated across `visual_line_rows`, `wrap_input_with_cursor`,
/// `find_char_offset_in_wrapped_line`, and `screen_to_textarea_pos`.
pub(crate) struct WrappedChars<'a> {
    chars: std::str::CharIndices<'a>,
    width: usize,
    display_row: usize,
    display_col: usize,
    char_idx: usize,
    cumulative_width: usize,
}

impl<'a> WrappedChars<'a> {
    pub fn new(text: &'a str, width: usize) -> Self {
        Self {
            chars: text.char_indices(),
            width,
            display_row: 0,
            display_col: 0,
            char_idx: 0,
            cumulative_width: 0,
        }
    }
}

impl Iterator for WrappedChars<'_> {
    type Item = WrappedChar;

    fn next(&mut self) -> Option<Self::Item> {
        let (byte_offset, ch) = self.chars.next()?;
        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);

        // Apply the wrapping predicate
        if self.display_col + ch_w > self.width && self.display_col > 0 {
            self.display_row += 1;
            self.display_col = 0;
        }

        let item = WrappedChar {
            display_row: self.display_row,
            display_col: self.display_col,
            char_idx: self.char_idx,
            byte_offset,
            ch,
            char_width: ch_w,
            cumulative_width: self.cumulative_width,
        };

        self.display_col += ch_w;
        self.char_idx += 1;
        self.cumulative_width += ch_w;

        Some(item)
    }
}

/// Count visual rows a line occupies when wrapped at the given width, using character display widths.
pub(crate) fn visual_line_rows(line: &str, width: usize) -> usize {
    if width == 0 || line.is_empty() {
        return 1;
    }
    match WrappedChars::new(line, width).last() {
        Some(wc) => wc.display_row + 1,
        None => 1,
    }
}

/// Find the character column offset for a given position within a wrapped line.
///
/// Returns a cumulative display-width value (not a char index) matching the semantics
/// of `TextPosition.char_offset` ("by unicode width columns").
pub(crate) fn find_char_offset_in_wrapped_line(
    text: &str,
    width: usize,
    target_sub_line: usize,
    screen_col: usize,
) -> usize {
    if width == 0 || text.is_empty() {
        return 0;
    }

    let mut total_width = 0;
    for wc in WrappedChars::new(text, width) {
        if wc.display_row == target_sub_line && wc.display_col >= screen_col {
            return wc.cumulative_width;
        }
        if wc.display_row > target_sub_line {
            return wc.cumulative_width;
        }
        total_width = wc.cumulative_width + wc.char_width;
    }

    // Clamp to end of line
    total_width
}

/// Find the character index at a given display column within a segment.
///
/// `base_idx` is the starting char index for the segment within the full line.
/// Returns `base_idx + i` where `i` is the first char whose cumulative display column
/// exceeds `target_col`, or `base_idx + char_count` if past the end.
pub(crate) fn find_char_at_display_col(segment: &str, base_idx: usize, target_col: usize) -> usize {
    for wc in WrappedChars::new(segment, usize::MAX) {
        if wc.display_col + wc.char_width > target_col {
            return base_idx + wc.char_idx;
        }
    }
    base_idx + segment.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrapped_chars_basic() {
        let items: Vec<_> = WrappedChars::new("abcdef", 3).collect();
        assert_eq!(items.len(), 6);
        // First 3 chars on row 0
        assert_eq!(items[0].display_row, 0);
        assert_eq!(items[0].display_col, 0);
        assert_eq!(items[0].cumulative_width, 0);
        assert_eq!(items[2].display_row, 0);
        assert_eq!(items[2].display_col, 2);
        assert_eq!(items[2].cumulative_width, 2);
        // Next 3 chars wrap to row 1
        assert_eq!(items[3].display_row, 1);
        assert_eq!(items[3].display_col, 0);
        assert_eq!(items[3].cumulative_width, 3);
        assert_eq!(items[5].display_row, 1);
        assert_eq!(items[5].display_col, 2);
    }

    #[test]
    fn test_wrapped_chars_exact_width() {
        let items: Vec<_> = WrappedChars::new("abcde", 5).collect();
        // All fit on one row
        assert_eq!(items.last().unwrap().display_row, 0);
    }

    #[test]
    fn test_wrapped_chars_empty() {
        let items: Vec<_> = WrappedChars::new("", 5).collect();
        assert!(items.is_empty());
    }

    #[test]
    fn test_visual_line_rows_empty() {
        assert_eq!(visual_line_rows("", 80), 1);
    }

    #[test]
    fn test_visual_line_rows_short() {
        assert_eq!(visual_line_rows("hello", 80), 1);
    }

    #[test]
    fn test_visual_line_rows_exact_width() {
        assert_eq!(visual_line_rows("abcde", 5), 1);
    }

    #[test]
    fn test_visual_line_rows_wraps() {
        assert_eq!(visual_line_rows("abcdef", 5), 2);
    }

    #[test]
    fn test_visual_line_rows_zero_width() {
        assert_eq!(visual_line_rows("abc", 0), 1);
    }

    #[test]
    fn test_find_char_offset_first_sub_line() {
        let offset = find_char_offset_in_wrapped_line("abcdefghij", 5, 0, 3);
        assert_eq!(offset, 3);
    }

    #[test]
    fn test_find_char_offset_second_sub_line() {
        let offset = find_char_offset_in_wrapped_line("abcdefghij", 5, 1, 2);
        assert_eq!(offset, 7); // 5 (first sub-line) + 2
    }

    #[test]
    fn test_find_char_offset_empty() {
        let offset = find_char_offset_in_wrapped_line("", 5, 0, 0);
        assert_eq!(offset, 0);
    }

    #[test]
    fn test_find_char_at_display_col_basic() {
        assert_eq!(find_char_at_display_col("abcde", 0, 3), 3);
    }

    #[test]
    fn test_find_char_at_display_col_past_end() {
        assert_eq!(find_char_at_display_col("abc", 2, 10), 5); // base 2 + 3 chars
    }

    #[test]
    fn test_find_char_at_display_col_with_base() {
        assert_eq!(find_char_at_display_col("de", 3, 0), 3);
    }
}
