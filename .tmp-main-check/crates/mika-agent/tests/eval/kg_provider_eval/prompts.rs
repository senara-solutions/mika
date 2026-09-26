//! Replicated extraction and resolution prompts for the KG provider eval (#762).
//!
//! These prompts mirror the production prompts in `kg/subject_extractor.rs` and
//! `kg/entity_resolver.rs`. They are replicated here (rather than imported) because
//! the production code builds prompts as methods on internal structs with private
//! dependencies. The eval needs standalone functions that accept plain data.

use mika_agent::kg::subject_extractor::{APPROVED_ENTITY_TYPES, APPROVED_RELATIONSHIP_TYPES};

// ---------------------------------------------------------------------------
// Extraction prompt
// ---------------------------------------------------------------------------

/// Build the extraction prompt from annotated document text.
///
/// Returns `(system_message, user_message)`.
pub fn build_extraction_prompt(annotated_text: &str) -> (String, String) {
    let approved_entity_types = APPROVED_ENTITY_TYPES.join(", ");
    let approved_rel_types = APPROVED_RELATIONSHIP_TYPES
        .iter()
        .map(|c| c.rel_type)
        .collect::<Vec<_>>()
        .join(", ");

    let system = format!(
        r#"You are a knowledge graph extraction agent. Extract named entities and fact triples from the following document.

Return ONLY valid JSON matching this schema:
{{
  "entities": [
    {{
      "type": "<entity_type>",
      "name": "<lowercase_underscore_name>",
      "description": "<brief description>",
      "chunk_indices": [<int>, ...],
      "confidence": <0.0-1.0>
    }}
  ],
  "relationships": [
    {{
      "from_type": "<entity_type>",
      "from_name": "<name>",
      "to_type": "<entity_type>",
      "to_name": "<name>",
      "type": "<relationship_type>",
      "chunk_indices": [<int>, ...],
      "confidence": <0.0-1.0>
    }}
  ]
}}

Approved entity types: {approved_entity_types}
Approved relationship types: {approved_rel_types}

Relationship type constraints:
- SOLVED_BY: from problem_type to solution_path
- USES: from solution_path to skill
- CALLS: from solution_path to tool
- INDICATES: from failure_mode to problem_type
- PREVENTS: from pattern to problem_type
- CAUSED_BY: from problem_type to problem_type
- MENTIONS: from any type to agent

Rules:
- Entity names must be lowercase with underscores (e.g., "state_drift", "self_dev")
- Entity names must NOT contain colons
- Each entity and relationship must reference chunk indices where evidence appears
- Only extract entities and relationships you are confident about
- If the document has no extractable entities, return {{"entities": [], "relationships": []}}
- Return ONLY the JSON object, no markdown fencing, no explanation"#
    );

    (system, annotated_text.to_string())
}

/// Build annotated text from document content with `[CHUNK N]` markers.
///
/// Mirrors the chunking logic in `kg/chunker.rs`:
/// 1. Strip YAML frontmatter (delimited by `---`)
/// 2. Split body on `## ` section headers
/// 3. Window-split oversized sections (>2000 chars) with 200-char overlap
/// 4. Prepend `[CHUNK N]` markers to each chunk
pub fn build_annotated_text(doc_content: &str) -> String {
    let chunks = chunk_doc_for_eval(doc_content);
    let mut annotated = String::with_capacity(doc_content.len() + chunks.len() * 12);
    for (i, chunk_text) in chunks.iter().enumerate() {
        if i > 0 {
            annotated.push('\n');
        }
        annotated.push_str(&format!("[CHUNK {i}] "));
        annotated.push_str(chunk_text);
    }
    annotated
}

/// Maximum number of characters per chunk before window splitting.
const MAX_CHUNK_CHARS: usize = 2000;

/// Overlap in characters between consecutive window-split chunks.
const OVERLAP_CHARS: usize = 200;

/// Simple document chunker that mirrors `kg/chunker.rs` logic.
fn chunk_doc_for_eval(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let mut sections: Vec<String> = Vec::new();

    // Phase 1: separate YAML frontmatter from body.
    let body = if text.starts_with("---\n") || text.starts_with("---\r\n") {
        let after_opening = if text.starts_with("---\r\n") { 5 } else { 4 };
        if let Some(end_pos) = text[after_opening..].find("\n---") {
            let frontmatter = text[..after_opening + end_pos].trim();
            if !frontmatter.is_empty() {
                sections.push(frontmatter.to_string());
            }
            // Skip past the closing `---` line.
            let rest = &text[after_opening + end_pos..];
            let after_delim = rest.find('\n').map(|p| p + 1).unwrap_or(rest.len());
            let after_delim = rest[after_delim..]
                .find('\n')
                .map(|p| after_delim + p + 1)
                .unwrap_or(rest.len());
            &rest[after_delim..]
        } else {
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
    for section in sections {
        let char_count = section.chars().count();
        if char_count <= MAX_CHUNK_CHARS {
            chunks.push(section);
        } else {
            // Window-split using char-based arithmetic.
            let chars: Vec<char> = section.chars().collect();
            let step = MAX_CHUNK_CHARS - OVERLAP_CHARS;
            let mut start = 0;
            while start < chars.len() {
                let end = (start + MAX_CHUNK_CHARS).min(chars.len());
                let chunk_text: String = chars[start..end].iter().collect();
                chunks.push(chunk_text);
                if end >= chars.len() {
                    break;
                }
                start += step;
            }
        }
    }

    chunks
}

/// Split text on `## ` section headers, keeping the header with its section.
fn split_on_h2(text: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if line.starts_with("## ") && !current.is_empty() {
            sections.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.is_empty() {
        sections.push(current);
    }

    sections
}

// ---------------------------------------------------------------------------
// Resolution / disambiguation prompt
// ---------------------------------------------------------------------------

/// Build the disambiguation prompt for entity resolution.
///
/// Returns `(system_message, user_message)`.
///
/// `candidates` is a list of `(entity_key, description)` pairs.
pub fn build_disambiguation_prompt(
    entity_key: &str,
    confidence: f64,
    chunk_context: &str,
    candidates: &[(String, String)],
) -> (String, String) {
    let system = r#"You are resolving an entity mention to a canonical knowledge graph node.
Given the entity extracted from prose and a list of candidate domain entities,
determine which candidate (if any) matches. Return null if NONE match —
do not force a pick.

Return ONLY valid JSON matching this schema:
{"match": "<entity_key>" | null, "confidence": 0.0-1.0}

Rules:
- "match" must be one of the candidate entity_key values, or null
- "confidence" is your confidence that the match is correct (0.0 to 1.0)
- If no candidate is a good match, return {"match": null, "confidence": 0.0}
- Consider both the entity name and the source prose context when deciding
- Return ONLY the JSON object, no markdown fencing, no explanation"#;

    let mut user = format!(
        "Extracted entity: {} (confidence: {:.2})\n",
        entity_key, confidence
    );

    if !chunk_context.is_empty() {
        let truncated = safe_truncate_str(chunk_context, 2000);
        user.push_str(&format!("\nSource prose:\n{}\n", truncated));
    }

    user.push_str("\nCandidates:\n");
    for (key, desc) in candidates {
        if desc.is_empty() {
            user.push_str(&format!("- {}\n", key));
        } else {
            user.push_str(&format!("- {} \u{2014} {}\n", key, desc));
        }
    }

    (system.to_string(), user)
}

/// UTF-8-safe string truncation (mirrors mika_common::text::safe_truncate).
fn safe_truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_extraction_prompt_contains_approved_types() {
        let (system, _user) = build_extraction_prompt("test text");
        assert!(system.contains("skill"));
        assert!(system.contains("problem_type"));
        assert!(system.contains("SOLVED_BY"));
        assert!(system.contains("MENTIONS"));
    }

    #[test]
    fn test_build_annotated_text_adds_chunk_markers() {
        let doc = "# Title\n\nSome content.\n\n## Section 1\n\nMore content.\n\n## Section 2\n\nFinal content.";
        let annotated = build_annotated_text(doc);
        assert!(annotated.contains("[CHUNK 0]"));
        assert!(annotated.contains("[CHUNK 1]"));
    }

    #[test]
    fn test_build_disambiguation_prompt_format() {
        let candidates = vec![
            (
                "skill:self-dev".to_string(),
                "Main self-dev skill".to_string(),
            ),
            ("skill:qa-review".to_string(), "".to_string()),
        ];
        let (system, user) =
            build_disambiguation_prompt("self_dev", 0.95, "some context", &candidates);
        assert!(system.contains("resolving an entity mention"));
        assert!(user.contains("self_dev"));
        assert!(user.contains("0.95"));
        assert!(user.contains("skill:self-dev"));
    }

    #[test]
    fn test_chunk_doc_strips_frontmatter() {
        let doc = "---\nmodule: test\ntags: [a, b]\n---\n\n# Title\n\nContent here.";
        let chunks = chunk_doc_for_eval(doc);
        assert!(!chunks.is_empty());
        // Frontmatter should be its own chunk
        assert!(chunks[0].contains("module: test"));
    }

    #[test]
    fn test_chunk_doc_splits_on_h2() {
        let doc = "## Section 1\n\nContent 1.\n\n## Section 2\n\nContent 2.";
        let chunks = chunk_doc_for_eval(doc);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn test_chunk_doc_empty() {
        let chunks = chunk_doc_for_eval("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_safe_truncate_ascii() {
        assert_eq!(safe_truncate_str("hello world", 5), "hello");
    }

    #[test]
    fn test_safe_truncate_multibyte() {
        // U+00E9 is 2 bytes in UTF-8
        let s = "caf\u{00e9}";
        assert_eq!(s.len(), 5); // 3 + 2
        let truncated = safe_truncate_str(s, 4);
        assert_eq!(truncated, "caf"); // Must not split in the middle of the e-acute
    }
}
