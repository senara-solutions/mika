//! # Markdown-Aware Document Chunker
//!
//! Deterministic, pure-function chunker for the lexical graph layer (#689).
//! Splits markdown documents into [`Chunk`]s suitable for entity extraction
//! and relationship linking.
//!
//! ## Algorithm
//!
//! 1. Strip YAML frontmatter (delimited by `---`) into its own chunk.
//! 2. Split the body on `## ` section headers.
//! 3. Window-split any section exceeding [`MAX_CHUNK_CHARS`] into overlapping
//!    windows of [`MAX_CHUNK_CHARS`] with [`OVERLAP_CHARS`] overlap.
//! 4. Assign monotonic [`Chunk::seq_id`] starting at 0.
//!
//! All size arithmetic uses **char counts**, not byte counts, so multibyte
//! UTF-8 sequences are handled correctly.

/// Maximum number of characters per chunk before window splitting.
const MAX_CHUNK_CHARS: usize = 2000;

/// Overlap in characters between consecutive window-split chunks.
const OVERLAP_CHARS: usize = 200;

/// A single chunk of document text with a monotonic sequence identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Monotonic sequence number starting at 0.
    pub seq_id: u32,
    /// The chunk text content.
    pub text: String,
}

/// Split a markdown document into deterministic, overlapping chunks.
///
/// Pure function — no I/O, no randomness. Same input always produces
/// the same output. An empty (or whitespace-only) document returns an
/// empty `Vec`.
pub fn chunk_doc(text: &str) -> Vec<Chunk> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let mut sections: Vec<String> = Vec::new();

    // Phase 1: separate YAML frontmatter from body.
    let body = if text.starts_with("---\n") || text.starts_with("---\r\n") {
        // Find the closing `---` that ends frontmatter.
        let after_opening = if text.starts_with("---\r\n") { 5 } else { 4 };
        if let Some(end_pos) = find_frontmatter_end(&text[after_opening..]) {
            let frontmatter = &text[..after_opening + end_pos];
            let frontmatter = frontmatter.trim();
            if !frontmatter.is_empty() {
                sections.push(frontmatter.to_string());
            }
            let rest = &text[after_opening + end_pos..];
            // Skip past the closing `---` line.
            skip_closing_delimiter(rest)
        } else {
            // No closing delimiter found — treat entire doc as body.
            text
        }
    } else {
        text
    };

    // Phase 2: split body on `## ` section headers.
    let body_sections = split_on_h2(body);
    for section in body_sections {
        let trimmed = section.trim();
        if !trimmed.is_empty() {
            sections.push(trimmed.to_string());
        }
    }

    // Phase 3: window-split oversized sections.
    let mut chunks = Vec::new();
    let mut seq_id: u32 = 0;
    for section in sections {
        let char_count = section.chars().count();
        if char_count <= MAX_CHUNK_CHARS {
            chunks.push(Chunk {
                seq_id,
                text: section,
            });
            seq_id += 1;
        } else {
            let windows = window_split(&section, MAX_CHUNK_CHARS, OVERLAP_CHARS);
            for window in windows {
                chunks.push(Chunk {
                    seq_id,
                    text: window,
                });
                seq_id += 1;
            }
        }
    }

    chunks
}

/// Find the position of the closing `---` delimiter within the frontmatter body.
///
/// `text` is the content *after* the opening `---\n`. Returns the byte offset
/// of the start of the closing `---` line, or `None` if not found.
fn find_frontmatter_end(text: &str) -> Option<usize> {
    for (i, line) in text.lines().enumerate() {
        // The closing delimiter must be `---` on its own line (possibly with
        // trailing whitespace).
        if line.trim() == "---" {
            // Compute byte offset of this line within `text`.
            let offset = text.lines().take(i).fold(0usize, |acc, l| {
                // Each line consumed `l.len()` bytes plus the newline.
                // `str::lines()` strips \n and \r\n, so we need to account
                // for the original separator.
                let sep = if text.as_bytes().get(acc + l.len()) == Some(&b'\r') {
                    2
                } else {
                    1
                };
                acc + l.len() + sep
            });
            return Some(offset);
        }
    }
    None
}

/// Skip past the closing `---` delimiter line (including its trailing newline).
fn skip_closing_delimiter(text: &str) -> &str {
    // `text` starts at the `---` closing line.
    if let Some(newline_pos) = text.find('\n') {
        &text[newline_pos + 1..]
    } else {
        // No newline after `---` — nothing left.
        ""
    }
}

/// Split body text on `## ` markdown headers.
///
/// Each section includes its `## ` header line. Text before the first `## `
/// becomes the first section (the preamble).
fn split_on_h2(text: &str) -> Vec<&str> {
    let mut sections = Vec::new();
    let mut last_start = 0;

    // We look for `\n## ` as the split point (section headers must be at
    // line start). The very first character could also be `## `.
    let bytes = text.as_bytes();
    let len = bytes.len();

    if text.starts_with("## ") {
        // First section starts at 0 with a header; we'll find the next split.
    }

    let mut i = 0;
    while i < len {
        // Check for `\n## ` pattern.
        if bytes[i] == b'\n' && i + 3 < len && &bytes[i + 1..i + 4] == b"## " {
            // Found a split point at i+1 (the `#` char).
            if i + 1 > last_start {
                sections.push(&text[last_start..i + 1]); // include the \n
            }
            last_start = i + 1;
            i += 4;
        } else {
            i += 1;
        }
    }

    // Remaining tail.
    if last_start < len {
        sections.push(&text[last_start..]);
    }

    sections
}

/// Split a string into windows of `max_chars` characters with `overlap` overlap.
///
/// All arithmetic is char-based for Unicode safety.
fn window_split(text: &str, max_chars: usize, overlap: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let total = chars.len();
    let mut windows = Vec::new();
    let mut start = 0;

    while start < total {
        let end = (start + max_chars).min(total);
        let window: String = chars[start..end].iter().collect();
        windows.push(window);
        if end == total {
            break;
        }
        let step = max_chars.saturating_sub(overlap);
        if step == 0 {
            break; // safety: avoid infinite loop
        }
        start += step;
    }

    windows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_small_doc() {
        let doc = "Hello, this is a small document.\n\nIt has no sections.";
        let chunks = chunk_doc(doc);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].seq_id, 0);
        assert_eq!(chunks[0].text, doc.trim());
    }

    #[test]
    fn section_split_three_sections() {
        let doc = "\
## Section One

Content of section one.

## Section Two

Content of section two.

## Section Three

Content of section three.
";
        let chunks = chunk_doc(doc);
        assert_eq!(chunks.len(), 3, "expected 3 chunks, got {chunks:?}");
        assert_eq!(chunks[0].seq_id, 0);
        assert_eq!(chunks[1].seq_id, 1);
        assert_eq!(chunks[2].seq_id, 2);
        assert!(chunks[0].text.starts_with("## Section One"));
        assert!(chunks[1].text.starts_with("## Section Two"));
        assert!(chunks[2].text.starts_with("## Section Three"));
    }

    #[test]
    fn window_split_oversized_section() {
        // Build a single section that exceeds MAX_CHUNK_CHARS.
        let filler = "a".repeat(MAX_CHUNK_CHARS + 500);
        let doc = format!("## Big Section\n\n{filler}");
        let chunks = chunk_doc(&doc);
        assert!(
            chunks.len() >= 2,
            "oversized section should produce multiple chunks, got {}",
            chunks.len()
        );

        // Verify overlap: the end of chunk 0 should overlap with the start of chunk 1.
        let c0_chars: Vec<char> = chunks[0].text.chars().collect();
        let c1_chars: Vec<char> = chunks[1].text.chars().collect();
        let c0_tail: String = c0_chars[c0_chars.len() - OVERLAP_CHARS..].iter().collect();
        let c1_head: String = c1_chars[..OVERLAP_CHARS].iter().collect();
        assert_eq!(c0_tail, c1_head, "overlap region should match");
    }

    #[test]
    fn frontmatter_handling() {
        let doc = "\
---
title: My Document
tags: [a, b]
---

Preamble text.

## First Section

Content here.

## Second Section

More content.
";
        let chunks = chunk_doc(doc);
        assert_eq!(chunks.len(), 4, "expected 4 chunks, got {chunks:?}");
        assert_eq!(chunks[0].seq_id, 0);
        assert!(
            chunks[0].text.starts_with("---"),
            "chunk 0 should be frontmatter"
        );
        assert!(
            chunks[0].text.contains("title: My Document"),
            "frontmatter should contain title"
        );
        assert_eq!(chunks[1].seq_id, 1);
        assert!(
            chunks[1].text.contains("Preamble text"),
            "chunk 1 should be preamble"
        );
        assert_eq!(chunks[2].seq_id, 2);
        assert!(chunks[2].text.starts_with("## First Section"));
        assert_eq!(chunks[3].seq_id, 3);
        assert!(chunks[3].text.starts_with("## Second Section"));
    }

    #[test]
    fn determinism() {
        let doc = "## A\n\nHello\n\n## B\n\nWorld";
        let run1 = chunk_doc(doc);
        let run2 = chunk_doc(doc);
        assert_eq!(run1, run2, "same input must produce identical output");
    }

    #[test]
    fn utf8_edge_multibyte_boundary() {
        // Build a string of multibyte chars that forces a split boundary.
        // U+1F600 (grinning face) is 4 bytes per char.
        let emoji = "\u{1F600}";
        let long_emoji = emoji.repeat(MAX_CHUNK_CHARS + 100);
        let doc = format!("## Emoji\n\n{long_emoji}");
        // Must not panic.
        let chunks = chunk_doc(&doc);
        assert!(
            chunks.len() >= 2,
            "should split oversized emoji section without panic"
        );
        // Verify each chunk is valid UTF-8 (guaranteed by String, but let's
        // also check char counts are within bounds).
        for chunk in &chunks {
            let cc = chunk.text.chars().count();
            assert!(
                cc <= MAX_CHUNK_CHARS,
                "chunk {} has {} chars, exceeds max",
                chunk.seq_id,
                cc
            );
        }
    }

    #[test]
    fn empty_doc_returns_empty() {
        assert!(chunk_doc("").is_empty());
        assert!(chunk_doc("   ").is_empty());
        assert!(chunk_doc("\n\n").is_empty());
    }
}
