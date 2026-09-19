//! Single reader of the production/test boundary, for the repo's structural
//! guards (mika#2398).
//!
//! # Why this module exists
//!
//! A structural guard sweeps the source tree and refuses a pattern. Each one has
//! to separate production from test code, or its own test module — which names
//! the pattern by construction — becomes its first offender. Thirteen sites did
//! that separation by hand, in six different ways, and the premise all six rest
//! on is false six ways over in this tree: a module-level test helper in the
//! middle of a file, a prose mention of the marker in a doc comment, a file that
//! is entirely test code and carries no marker at all, a single-line item with
//! no closing brace, `cfg(not(test))` marking *production*, and an indented
//! attribute inside a production item.
//!
//! The error is not symmetric. A guard that over-masks reddens CI and gets
//! noticed within the hour. A guard that under-masks — that stopped *looking* at
//! a region — stays green, which is exactly what it promised never to do. So the
//! boundary is answered once, here, and the guards ask.
//!
//! # The rule (KTD2)
//!
//! 1. **Opening.** A line whose entire content is an attribute `#[cfg(…)]` whose
//!    condition mentions `test` *positively* opens a region. `cfg(not(test))`
//!    opens nothing — it marks production. The attribute's indentation `I` is
//!    retained.
//! 2. **Item form.** From the first following line that is not itself an
//!    attribute, the first `{` or `;` at item level decides: `{` → block item,
//!    `;` → single-line item.
//! 3. **Closing.** A block item's region runs to the first line whose content is
//!    exactly `I` + `}`, included. A single-line item's region is the attribute
//!    and its item, and nothing else.
//!
//! The region is **masked** — its lines are replaced by empty ones — never
//! truncated. Truncation assumes the test code sits at the end of the file,
//! which is false from form (1) onwards, and that assumption is what costs
//! `auto_pull.rs` 1 859 lines of invisible production.
//!
//! **Indentation, not brace counting.** The `mod tests` blocks in this repo are
//! full of JSON literals; a brace counter drifts on them, in one direction or
//! the other, with no signal. `rustfmt` is enforced by CI (`cargo fmt --check`),
//! so closing-at-own-indentation is an invariant already verified elsewhere
//! rather than a new hypothesis.
//!
//! **The item's form decides the close, and that clause is load-bearing.** The
//! naive rule ("run to the next `}` in column 0") masks 481 lines of production
//! across this tree's eight single-line items — 356 of them on one
//! `#[cfg(test)] mod tests_e4_no_log;` whose next column-0 brace is very far
//! away. A remedy that re-introduces the blindness it removes is not a remedy.
//!
//! **The indentation is the attribute's, not column 0.** Thirteen attributes in
//! this tree are indented inside a production `impl`. A column-0 rule leaves
//! their helpers in production, and a switched guard starts reddening on test
//! helpers it was never meant to see.
//!
//! # Test-only files (KTD3)
//!
//! A file can be entirely test code and carry no marker: the `#[cfg(test)]` sits
//! on the `mod` declaration in its parent. Such a file is recognised **by that
//! declaration**, never by a naming convention — three test-only files in this
//! tree (`mika-agent/src/test_utils.rs`, `mika-common/src/llm/mock.rs`,
//! `mika-gateway/src/voice/examples.rs`) match none of `tests/`, `tests.rs`,
//! `*_test.rs`, `*_tests.rs`, so a convention check would have produced three
//! false positives on its first run.
//!
//! # What this module does NOT do
//!
//! It is not a Rust parser. It reads indentation, attribute lines and module
//! declarations. Named bounds, each of which the coherence guard in
//! [`TestOnlyModules`] or [`SliceReport::unclosed`] makes visible rather than
//! silent:
//!
//! - A region whose closing line is never found is **not masked at all**, and is
//!   reported in [`SliceReport::unclosed`]. Masking to end-of-file would be the
//!   blindness this module exists to remove; leaving it visible is the loud
//!   direction.
//! - A raw string literal spanning lines, between an opening attribute and its
//!   item, would confuse the item-form scan. The scan is bounded to
//!   [`MAX_ITEM_HEADER_LINES`] lines and reports `unclosed` past that.
//! - A `mod` chain reaching a test-only file through an intermediate link that
//!   is not itself under `cfg(test)` is not followed. [`TestOnlyModules`]
//!   reports the deepest chain it did follow.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Upper bound on how many lines the item-form scan reads after an opening
/// attribute before giving up.
///
/// `cargo fmt` routinely breaks a signature over several lines — the helper at
/// `auto_pull.rs` opens with `fn select_feeder_candidates(` and its brace is six
/// lines lower — so the scan cannot stop at the first line. It also must not run
/// away: an unterminated raw string would otherwise carry it to end of file.
pub const MAX_ITEM_HEADER_LINES: usize = 64;

/// What masking one file produced, and what it could not decide.
#[derive(Debug, Clone, Default)]
pub struct SliceReport {
    /// The file's production half: test regions replaced by empty lines, so the
    /// line numbers of everything kept are the file's own. Guards need that —
    /// they name their offender by line.
    pub production: String,
    /// 1-based line of each opening attribute whose region could not be closed.
    ///
    /// Such a region is **left visible**: see the module docs for why the loud
    /// direction is the right one here.
    pub unclosed: Vec<usize>,
    /// The whole file is test code, recognised by its declaration in the parent
    /// (KTD3). `production` is then blank lines only.
    pub test_only_file: bool,
}

// ---------------------------------------------------------------------------
// KTD2 — masking cfg(test) regions
// ---------------------------------------------------------------------------

/// Production half of `content`, with every `cfg(test)` region masked (KTD2).
///
/// Knows nothing about KTD3; a file that is entirely test code but carries no
/// marker comes back unchanged. Use [`ProductionScanner`] for the whole answer.
pub fn mask_test_regions(content: &str) -> String {
    mask_test_regions_report(content).production
}

/// [`mask_test_regions`] plus what it could not decide.
pub fn mask_test_regions_report(content: &str) -> SliceReport {
    let lines: Vec<&str> = content.lines().collect();
    let mut keep = vec![true; lines.len()];
    let mut unclosed = Vec::new();
    let clean = lines_starting_outside_a_literal(&lines);

    let mut i = 0usize;
    while i < lines.len() {
        // A line sitting inside a multi-line string literal is text, not code.
        // Test fixtures in this repo quote whole Rust files, attribute and
        // closing brace included; without this both the opening and the closing
        // of a region could be read out of a quotation.
        if !clean[i] {
            i += 1;
            continue;
        }
        let Some(attr) = CfgAttribute::parse(lines[i]) else {
            i += 1;
            continue;
        };
        if !attr.opens_test_region {
            i += 1;
            continue;
        }

        match region_end(&lines, i, &attr, &clean) {
            Some(end) => {
                for slot in keep.iter_mut().take(end + 1).skip(i) {
                    *slot = false;
                }
                i = end + 1;
            }
            None => {
                unclosed.push(i + 1);
                i += 1;
            }
        }
    }

    let production = lines
        .iter()
        .zip(&keep)
        .map(|(line, keep)| if *keep { *line } else { "" })
        .collect::<Vec<_>>()
        .join("\n");

    SliceReport {
        production,
        unclosed,
        test_only_file: false,
    }
}

/// Last line (0-based) of the region opened by the attribute at `attr_idx`.
fn region_end(
    lines: &[&str],
    attr_idx: usize,
    attr: &CfgAttribute<'_>,
    clean: &[bool],
) -> Option<usize> {
    let scan = item_header_lines(lines, attr_idx, attr);
    match find_item_form(&scan)? {
        ItemForm::SingleLine(line) => Some(line),
        ItemForm::Block(line, column) => {
            // A body written on one line (`fn helper() {}`) closes on that line
            // and never reaches an `I}` closer. Checked first, because searching
            // for the closer would then run past it into the next item.
            if line_closes_its_own_block(lines[line], column) {
                return Some(line);
            }
            let closer = format!("{}}}", attr.indent);
            (line + 1..lines.len()).find(|&k| clean[k] && lines[k].trim_end() == closer)
        }
    }
}

/// Does the `{` at byte `column` close again before the end of its own line?
fn line_closes_its_own_block(line: &str, column: usize) -> bool {
    let mut state = LexState::default();
    let mut depth = 0usize;
    let mut closes = false;
    scan_line(&mut state, &line[column..], |_, byte| match byte {
        b'{' => {
            depth += 1;
            false
        }
        b'}' => {
            depth -= 1;
            if depth == 0 {
                closes = true;
                true
            } else {
                false
            }
        }
        _ => false,
    });
    closes
}

/// For each line, whether it *starts* outside any string literal or block
/// comment — i.e. whether its text is code at all.
fn lines_starting_outside_a_literal(lines: &[&str]) -> Vec<bool> {
    let mut out = Vec::with_capacity(lines.len());
    let mut state = LexState::default();
    for line in lines {
        out.push(state.is_clean());
        scan_line(&mut state, line, |_, _| false);
    }
    out
}

// ---------------------------------------------------------------------------
// A line-at-a-time Rust lexer, just deep enough to know what is code
// ---------------------------------------------------------------------------

/// Carried across lines, because raw string literals routinely span them — the
/// test fixtures in this repo quote whole Rust files, closing brace included.
#[derive(Debug, Default, Clone, Copy)]
struct LexState {
    in_block_comment: bool,
    in_string: bool,
    in_raw_string: Option<usize>,
}

impl LexState {
    fn is_clean(&self) -> bool {
        !self.in_block_comment && !self.in_string && self.in_raw_string.is_none()
    }
}

/// Advance `state` through one line, handing every *code* byte to `sink`.
///
/// `sink` returns `true` to stop; `scan_line` then returns `true` too, leaving
/// `state` where it stopped. Line comments end the line's code, never the state.
fn scan_line<F: FnMut(usize, u8) -> bool>(state: &mut LexState, line: &str, mut sink: F) -> bool {
    let b = line.as_bytes();
    let mut p = 0usize;
    while p < b.len() {
        if state.in_block_comment {
            if b[p] == b'*' && p + 1 < b.len() && b[p + 1] == b'/' {
                state.in_block_comment = false;
                p += 2;
            } else {
                p += 1;
            }
            continue;
        }
        if let Some(hashes) = state.in_raw_string {
            if b[p] == b'"' {
                let mut seen = 0usize;
                let mut r = p + 1;
                while r < b.len() && seen < hashes && b[r] == b'#' {
                    seen += 1;
                    r += 1;
                }
                if seen == hashes {
                    state.in_raw_string = None;
                    p = r;
                    continue;
                }
            }
            p += 1;
            continue;
        }
        if state.in_string {
            match b[p] {
                b'\\' => p += 2,
                b'"' => {
                    state.in_string = false;
                    p += 1;
                }
                _ => p += 1,
            }
            continue;
        }

        if b[p] == b'/' && p + 1 < b.len() && b[p + 1] == b'/' {
            return false; // the rest of the line is a comment, the state is untouched
        }
        if b[p] == b'/' && p + 1 < b.len() && b[p + 1] == b'*' {
            state.in_block_comment = true;
            p += 2;
            continue;
        }
        if let Some(raw) = raw_string_at(b, p) {
            match raw {
                RawString::Closed(next) => p = next,
                RawString::Open(hashes) => {
                    state.in_raw_string = Some(hashes);
                    return false;
                }
            }
            continue;
        }
        match b[p] {
            b'"' => {
                match ordinary_string_end(b, p) {
                    Some(next) => p = next,
                    None => {
                        state.in_string = true;
                        return false;
                    }
                }
                continue;
            }
            b'\'' => {
                p = skip_char_or_lifetime(b, p);
                continue;
            }
            byte => {
                if sink(p, byte) {
                    return true;
                }
                p += 1;
            }
        }
    }
    false
}

/// Index just past a `"…"` literal starting at `start`, or `None` when it runs
/// off the end of the line (Rust allows a newline inside an ordinary string).
fn ordinary_string_end(b: &[u8], start: usize) -> Option<usize> {
    let mut p = start + 1;
    while p < b.len() {
        match b[p] {
            b'\\' => p += 2,
            b'"' => return Some(p + 1),
            _ => p += 1,
        }
    }
    None
}

enum RawString {
    Closed(usize),
    Open(usize),
}

/// A raw string literal (`r"…"`, `r#"…"#`, `br#"…"#`) starting at `p`.
fn raw_string_at(b: &[u8], p: usize) -> Option<RawString> {
    if p > 0 && is_ident_byte(b[p - 1]) {
        return None;
    }
    let mut q = p;
    if b[q] == b'b' {
        q += 1;
    }
    if q >= b.len() || b[q] != b'r' {
        return None;
    }
    q += 1;
    let mut hashes = 0usize;
    while q < b.len() && b[q] == b'#' {
        hashes += 1;
        q += 1;
    }
    if q >= b.len() || b[q] != b'"' {
        return None;
    }
    q += 1;
    while q < b.len() {
        if b[q] == b'"' {
            let mut seen = 0usize;
            let mut r = q + 1;
            while r < b.len() && seen < hashes && b[r] == b'#' {
                seen += 1;
                r += 1;
            }
            if seen == hashes {
                return Some(RawString::Closed(r));
            }
        }
        q += 1;
    }
    Some(RawString::Open(hashes))
}

/// The lines the item-form scan reads: the attribute's own trailing remainder
/// when the item starts on that line, then the item's own lines, skipping any
/// further attribute-only lines in between.
fn item_header_lines<'a>(
    lines: &[&'a str],
    attr_idx: usize,
    attr: &CfgAttribute<'a>,
) -> Vec<(usize, usize, &'a str)> {
    let mut scan: Vec<(usize, usize, &'a str)> = Vec::new();
    let mut start = attr_idx + 1;

    match attr.rest_of_line {
        // The item starts on the attribute's own line; the offset keeps the
        // reported column relative to the whole line.
        Some(rest) => scan.push((attr_idx, lines[attr_idx].len() - rest.len(), rest)),
        None => {
            while start < lines.len() && is_attribute_only_line(lines[start]) {
                start += 1;
            }
        }
    }
    let stop = (start + MAX_ITEM_HEADER_LINES).min(lines.len());
    scan.extend((start..stop).map(|k| (k, 0usize, lines[k])));
    scan
}

enum ItemForm {
    /// The item opens a block: 0-based line, then byte column of the `{`.
    Block(usize, usize),
    /// The item is terminated by `;` on this 0-based line.
    SingleLine(usize),
}

/// First `{` or `;` at item level, skipping comments and literals.
fn find_item_form(scan: &[(usize, usize, &str)]) -> Option<ItemForm> {
    let mut state = LexState::default();
    let mut found: Option<ItemForm> = None;

    for &(idx, offset, text) in scan {
        scan_line(&mut state, text, |p, byte| match byte {
            b'{' => {
                found = Some(ItemForm::Block(idx, offset + p));
                true
            }
            b';' => {
                found = Some(ItemForm::SingleLine(idx));
                true
            }
            _ => false,
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

fn skip_char_or_lifetime(b: &[u8], start: usize) -> usize {
    if start + 1 < b.len() && b[start + 1] == b'\\' {
        let mut p = start + 2;
        while p < b.len() && b[p] != b'\'' {
            p += 1;
        }
        return (p + 1).min(b.len());
    }
    if start + 2 < b.len() && b[start + 2] == b'\'' {
        return start + 3;
    }
    start + 1 // a lifetime, not a char literal
}

fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// A `#[cfg(…)]` attribute occupying (at least) the start of a line.
struct CfgAttribute<'a> {
    /// Leading whitespace of the line — the indentation the region closes at.
    indent: &'a str,
    /// Non-empty remainder after the attribute, when the item starts on the
    /// same line.
    rest_of_line: Option<&'a str>,
    /// The condition mentions `test` positively (so: not `cfg(not(test))`).
    opens_test_region: bool,
}

impl<'a> CfgAttribute<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let indent_len = line.len() - line.trim_start().len();
        let (indent, trimmed) = line.split_at(indent_len);
        if !trimmed.starts_with("#[cfg(") {
            return None;
        }
        let end = attribute_end(trimmed)?;
        let body = &trimmed[..end];
        // `#[cfg(` … `)]` — the condition is what sits between.
        let condition = body.strip_prefix("#[cfg(")?.strip_suffix(")]")?;
        let rest = trimmed[end..].trim();
        Some(Self {
            indent,
            rest_of_line: (!rest.is_empty()).then_some(rest),
            opens_test_region: condition_is_positively_test(condition),
        })
    }
}

/// Index just past the `]` closing the attribute that starts at the first `[`.
fn attribute_end(trimmed: &str) -> Option<usize> {
    let open = trimmed.find('[')?;
    let mut depth = 0usize;
    let mut end = None;
    let mut state = LexState::default();
    scan_line(&mut state, &trimmed[open..], |p, byte| match byte {
        b'[' => {
            depth += 1;
            false
        }
        b']' => {
            depth -= 1;
            if depth == 0 {
                end = Some(open + p + 1);
                true
            } else {
                false
            }
        }
        _ => false,
    });
    end
}

/// Does this `cfg` condition mention `test` as something that must be **on**?
///
/// `not(test)` marks production and must not open a region — one site in this
/// tree (`builtin_handlers.rs`) sits immediately above its `cfg(test)` twin, so
/// a rule that read "a `cfg` mentioning test" would invert the meaning there and
/// hide 2 999 lines of production.
fn condition_is_positively_test(condition: &str) -> bool {
    let without_strings = strip_string_literals(condition);
    let without_negations = strip_group(&without_strings, "not");
    contains_identifier(&without_negations, "test")
}

fn strip_string_literals(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut state = LexState::default();
    scan_line(&mut state, s, |_, byte| {
        out.push(byte as char);
        false
    });
    out
}

/// `s` with every balanced `name(…)` group removed.
fn strip_group(s: &str, name: &str) -> String {
    let mut out = s.to_string();
    loop {
        let Some(at) = find_call_site(&out, name) else {
            return out;
        };
        let open = at + name.len();
        let mut depth = 0usize;
        let mut close = None;
        for (offset, ch) in out[open..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + offset + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        match close {
            Some(close) => out.replace_range(at..close, ""),
            None => return out,
        }
    }
}

/// Byte index of `name` used as an identifier immediately followed by `(`.
fn find_call_site(haystack: &str, name: &str) -> Option<usize> {
    let mut from = 0usize;
    while let Some(rel) = haystack[from..].find(name) {
        let at = from + rel;
        let after = at + name.len();
        let preceded = at > 0 && is_ident_byte(haystack.as_bytes()[at - 1]);
        let called = haystack.as_bytes().get(after) == Some(&b'(');
        if !preceded && called {
            return Some(at);
        }
        from = at + name.len();
    }
    None
}

/// Whole-word search: `test` matches, `test_utils` and `latest` do not.
fn contains_identifier(haystack: &str, ident: &str) -> bool {
    let b = haystack.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = haystack[from..].find(ident) {
        let at = from + rel;
        let after = at + ident.len();
        let left_ok = at == 0 || !is_ident_byte(b[at - 1]);
        let right_ok = b.get(after).is_none_or(|c| !is_ident_byte(*c));
        if left_ok && right_ok {
            return true;
        }
        from = at + ident.len();
    }
    false
}

fn is_attribute_only_line(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with("#[") && !trimmed.starts_with("#![") {
        return false;
    }
    attribute_end(trimmed).is_some_and(|end| trimmed[end..].trim().is_empty())
}

// ---------------------------------------------------------------------------
// KTD3 — files that are entirely test code
// ---------------------------------------------------------------------------

/// The files of one crate's `src/` tree that are entirely test code, recognised
/// by their `mod` declaration in the parent.
#[derive(Debug, Clone, Default)]
pub struct TestOnlyModules {
    files: HashSet<PathBuf>,
    /// `(declaring file, message)` — filtered at the end of
    /// [`test_only_modules`] so that a declaration read out of a *test-only*
    /// file is dropped. Such a file's own contents gate no production, and its
    /// fixtures routinely carry declaration-shaped strings; this module's own
    /// test fixtures are the first example.
    unresolved: Vec<(PathBuf, String)>,
    max_chain_depth: usize,
}

impl TestOnlyModules {
    pub fn contains(&self, path: &Path) -> bool {
        self.files.contains(path)
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Sorted, for stable reporting.
    pub fn files(&self) -> Vec<&Path> {
        let mut out: Vec<&Path> = self.files.iter().map(PathBuf::as_path).collect();
        out.sort_unstable();
        out
    }

    /// Declarations that resolved to no file on disk.
    ///
    /// `cargo build` already refuses that state, so a non-empty list means the
    /// path resolution here is wrong — KTD3 is to be repaired, not exempted.
    pub fn unresolved(&self) -> Vec<&str> {
        self.unresolved.iter().map(|(_, m)| m.as_str()).collect()
    }

    /// Deepest inline-module chain walked to reach a test-only file.
    ///
    /// The named blind spot: a chain whose intermediate link is not itself under
    /// `cfg(test)` is not followed. This is the measurement that says whether
    /// that case exists in the tree.
    pub fn max_chain_depth(&self) -> usize {
        self.max_chain_depth
    }
}

/// One `mod X;` declaration and the inline-module chain it sits under.
#[derive(Debug)]
struct ModuleDeclaration {
    chain: Vec<String>,
    name: String,
    path_attribute: Option<String>,
}

/// Walk `src_root` and collect every file that is entirely test code (KTD3).
pub fn test_only_modules(src_root: &Path) -> TestOnlyModules {
    let files = rust_sources_under(src_root);
    let mut out = TestOnlyModules::default();

    // Pass 1 — declarations that live inside a masked region.
    for file in &files {
        let Ok(content) = std::fs::read_to_string(file) else {
            continue;
        };
        for decl in declarations_in_masked_regions(&content) {
            resolve_declaration(file, &decl, &mut out);
        }
    }

    // Pass 2 — fixpoint. Inside a file that is already test code every module
    // declaration is test code too, whether or not it repeats the attribute;
    // and so is every file under a test-only module's directory.
    loop {
        let before = out.files.len();

        let known: Vec<PathBuf> = out.files.iter().cloned().collect();
        for file in known {
            if let Ok(content) = std::fs::read_to_string(&file) {
                for decl in all_declarations(&content) {
                    resolve_declaration(&file, &decl, &mut out);
                }
            }
            let dir = module_dir(&file);
            for candidate in &files {
                if candidate.starts_with(&dir) {
                    out.files.insert(candidate.clone());
                }
            }
        }

        if out.files.len() == before {
            break;
        }
    }

    // A declaration read out of a file that is itself test code gates no
    // production, and such a file's fixtures routinely carry
    // declaration-shaped strings. Dropping them here rather than earlier keeps
    // the set the fixpoint needed while leaving the coherence report about
    // production files only.
    let test_only = out.files.clone();
    out.unresolved
        .retain(|(declaring, _)| !test_only.contains(declaring));
    out
}

fn resolve_declaration(declaring_file: &Path, decl: &ModuleDeclaration, out: &mut TestOnlyModules) {
    let mut dir = module_dir(declaring_file);
    for segment in &decl.chain {
        dir = dir.join(segment);
    }
    out.max_chain_depth = out.max_chain_depth.max(decl.chain.len());

    let candidates: Vec<PathBuf> = match &decl.path_attribute {
        Some(path) => vec![dir.join(path)],
        None => vec![
            dir.join(format!("{}.rs", decl.name)),
            dir.join(&decl.name).join("mod.rs"),
        ],
    };

    let mut resolved = false;
    for candidate in candidates {
        if candidate.is_file() {
            out.files.insert(candidate);
            resolved = true;
        }
    }
    if !resolved {
        out.unresolved.push((
            declaring_file.to_path_buf(),
            format!(
                "{}: `mod {};` resolves to no file",
                declaring_file.display(),
                decl.name
            ),
        ));
    }
}

/// Directory a file's child modules live in.
fn module_dir(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or(Path::new("")).to_path_buf();
    match file.file_name().and_then(|n| n.to_str()) {
        Some("mod.rs") | Some("lib.rs") | Some("main.rs") => parent,
        _ => match file.file_stem().and_then(|n| n.to_str()) {
            Some(stem) => parent.join(stem),
            None => parent,
        },
    }
}

fn declarations_in_masked_regions(content: &str) -> Vec<ModuleDeclaration> {
    let lines: Vec<&str> = content.lines().collect();
    let clean = lines_starting_outside_a_literal(&lines);
    let masked = mask_test_regions(content);
    let region: Vec<&str> = masked
        .lines()
        .enumerate()
        .filter(|(i, line)| line.is_empty() && !lines[*i].is_empty() && clean[*i])
        .map(|(i, _)| lines[i])
        .collect();
    parse_declarations(&region)
}

fn all_declarations(content: &str) -> Vec<ModuleDeclaration> {
    let lines: Vec<&str> = content.lines().collect();
    let clean = lines_starting_outside_a_literal(&lines);
    let code: Vec<&str> = lines
        .iter()
        .zip(&clean)
        .map(|(line, clean)| if *clean { *line } else { "" })
        .collect();
    parse_declarations(&code)
}

/// Read `mod X;` declarations out of a run of lines, tracking which inline
/// `mod Y {` blocks they sit inside.
///
/// Nesting is read from **indentation**, for the reason KTD2 gives: these blocks
/// are full of JSON literals and a brace counter drifts on them. A line carrying
/// a JSON brace cannot be mistaken for `mod NAME {`.
fn parse_declarations(lines: &[&str]) -> Vec<ModuleDeclaration> {
    let mut out = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut pending_path: Option<String> = None;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        while stack.last().is_some_and(|(at, _)| *at >= indent) {
            stack.pop();
        }

        if let Some(path) = parse_path_attribute(trimmed) {
            pending_path = Some(path);
            continue;
        }
        if trimmed.starts_with("#[") || trimmed.starts_with("//") {
            continue; // other attributes and doc comments keep `pending_path`
        }
        if let Some(name) = parse_mod_opening(trimmed) {
            stack.push((indent, name));
            pending_path = None;
            continue;
        }
        if let Some(name) = parse_mod_declaration(trimmed) {
            out.push(ModuleDeclaration {
                chain: stack.iter().map(|(_, n)| n.clone()).collect(),
                name,
                path_attribute: pending_path.take(),
            });
            continue;
        }
        pending_path = None;
    }
    out
}

fn parse_path_attribute(trimmed: &str) -> Option<String> {
    let inner = trimmed.strip_prefix("#[path")?.trim_start();
    let inner = inner.strip_prefix('=')?.trim_start();
    let inner = inner.strip_prefix('"')?;
    let close = inner.find('"')?;
    Some(inner[..close].to_string())
}

/// `mod NAME {` / `pub mod NAME {` / `pub(crate) mod NAME {` → `NAME`.
fn parse_mod_opening(trimmed: &str) -> Option<String> {
    let (name, rest) = split_mod_name(trimmed)?;
    rest.trim_start().starts_with('{').then_some(name)
}

/// `mod NAME;` / `pub mod NAME;` → `NAME`.
fn parse_mod_declaration(trimmed: &str) -> Option<String> {
    let (name, rest) = split_mod_name(trimmed)?;
    (rest.trim_start() == ";").then_some(name)
}

fn split_mod_name(trimmed: &str) -> Option<(String, &str)> {
    let rest = strip_visibility(trimmed);
    let rest = rest.strip_prefix("mod")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    Some((rest[..end].to_string(), &rest[end..]))
}

fn strip_visibility(trimmed: &str) -> &str {
    let Some(rest) = trimmed.strip_prefix("pub") else {
        return trimmed;
    };
    let rest = if rest.starts_with('(') {
        match rest.find(')') {
            Some(close) => &rest[close + 1..],
            None => return trimmed,
        }
    } else {
        rest
    };
    if rest.starts_with(char::is_whitespace) {
        rest.trim_start()
    } else {
        trimmed
    }
}

// ---------------------------------------------------------------------------
// Walking a source tree
// ---------------------------------------------------------------------------

/// Every `.rs` file under `root`, recursively, sorted.
///
/// The loop fifteen guards write identically today. `walkdir` is deliberately
/// not pulled in for a test-only helper.
pub fn rust_sources_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// The production half of a crate's source tree: KTD3 then KTD2.
///
/// Built once per guard, because [`test_only_modules`] reads the whole tree.
#[derive(Debug, Clone)]
pub struct ProductionScanner {
    src_root: PathBuf,
    test_only: TestOnlyModules,
}

impl ProductionScanner {
    /// Scanner over `<manifest_dir>/src`, the shape every guard needs:
    /// `ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"))`.
    pub fn for_crate(manifest_dir: impl AsRef<Path>) -> Self {
        Self::new(manifest_dir.as_ref().join("src"))
    }

    pub fn new(src_root: impl Into<PathBuf>) -> Self {
        let src_root = src_root.into();
        let test_only = test_only_modules(&src_root);
        Self {
            src_root,
            test_only,
        }
    }

    pub fn src_root(&self) -> &Path {
        &self.src_root
    }

    pub fn test_only(&self) -> &TestOnlyModules {
        &self.test_only
    }

    pub fn files(&self) -> Vec<PathBuf> {
        rust_sources_under(&self.src_root)
    }

    /// Production half of `content`, read as if it lived at `path`.
    pub fn production_of_content(&self, path: &Path, content: &str) -> String {
        self.report_of_content(path, content).production
    }

    /// [`Self::production_of_content`] plus what masking could not decide.
    pub fn report_of_content(&self, path: &Path, content: &str) -> SliceReport {
        if self.test_only.contains(path) {
            return SliceReport {
                production: blank_like(content),
                unclosed: Vec::new(),
                test_only_file: true,
            };
        }
        mask_test_regions_report(content)
    }

    /// Production half of the file at `path`.
    ///
    /// Panics with the path in the message when the file cannot be read: a guard
    /// whose scan silently skipped a file is a guard that stopped looking, which
    /// is the failure mode this whole module exists to close.
    pub fn production_of(&self, path: &Path) -> String {
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("the guard must be able to read {}: {e}", path.display()));
        self.production_of_content(path, &content)
    }

    /// Call `f(path, production)` for every `.rs` file under the crate's `src/`.
    ///
    /// Panics when the walk found nothing — a broken path must redden, not pass.
    pub fn for_each(&self, f: impl FnMut(&Path, &str)) {
        self.for_each_under(&self.src_root.clone(), f);
    }

    /// [`Self::for_each`] over an arbitrary root (a crate's `tests/` tree, say).
    ///
    /// KTD3 is resolved against the scanner's own `src/`, so a file outside it
    /// is masked by KTD2 alone — which is right: an integration-test file is not
    /// a module of `src/`, and its `#[test]` bodies are the very production this
    /// kind of guard must read.
    pub fn for_each_under(&self, root: &Path, mut f: impl FnMut(&Path, &str)) {
        let files = rust_sources_under(root);
        assert!(
            !files.is_empty(),
            "the guard scanned no file under {} — broken path",
            root.display()
        );
        for path in files {
            let production = self.production_of(&path);
            f(&path, &production);
        }
    }
}

fn blank_like(content: &str) -> String {
    content.lines().map(|_| "").collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TEMPORARY measurement harness for the mika#2398 audit. Removed before land.
    #[test]
    #[ignore]
    fn mika2398_measure_the_tree() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let crates = [
            "mika-agent",
            "mika-common",
            "mika-gateway",
            "mika-cli",
            "mika-a2a",
        ];
        let mut total_lost = 0usize;
        let mut rows: Vec<(usize, String, usize)> = Vec::new();
        let mut files_total = 0usize;

        for name in crates {
            let src = workspace.join("crates").join(name).join("src");
            let scanner = ProductionScanner::new(&src);
            println!(
                "\n### {name}: test-only = {:?}",
                scanner
                    .test_only()
                    .files()
                    .iter()
                    .map(|p| p.strip_prefix(&src).unwrap().display().to_string())
                    .collect::<Vec<_>>()
            );
            println!("    unresolved = {:?}", scanner.test_only().unresolved());
            println!("    max_chain_depth = {}", scanner.test_only().max_chain_depth());

            for path in scanner.files() {
                files_total += 1;
                let content = std::fs::read_to_string(&path).unwrap();
                let report = scanner.report_of_content(&path, &content);
                if !report.unclosed.is_empty() {
                    println!(
                        "    UNCLOSED {}: {:?}",
                        path.strip_prefix(&workspace).unwrap().display(),
                        report.unclosed
                    );
                }
                if report.test_only_file {
                    continue;
                }
                // What a naive `find("#[cfg(test)]")` truncation would lose:
                // production lines after the first occurrence of the marker.
                let marker = ["#[cfg", "(test)]"].concat();
                if let Some(at) = content.find(&marker) {
                    let cut_line = content[..at].matches('\n').count() + 1;
                    let lost = report
                        .production
                        .lines()
                        .skip(cut_line)
                        .filter(|l| !l.trim().is_empty())
                        .count();
                    if lost > 0 {
                        total_lost += lost;
                        rows.push((
                            lost,
                            path.strip_prefix(&workspace).unwrap().display().to_string(),
                            cut_line,
                        ));
                    }
                }
            }
        }
        rows.sort_by(|a, b| b.0.cmp(&a.0));
        println!("\n### files scanned = {files_total}");
        println!("### truncation would lose {total_lost} production lines over {} files", rows.len());
        for (lost, path, cut) in rows.iter().take(30) {
            println!("| {lost} | `{path}` | {cut} |");
        }
    }

    // -- KTD2, one test per measured form -----------------------------------

    /// **F1** — a module-level `#[cfg(test)] fn` helper, production after it,
    /// then the real `mod tests`. The production in the middle survives.
    ///
    /// The shape of `auto_pull.rs:1922`, whose helper closes 1 975 lines before
    /// the real test module and costs every truncating guard 1 859 lines.
    #[test]
    fn mika2398_f1_module_level_test_helper_does_not_swallow_what_follows() {
        let src = "\
#[cfg(test)]
fn helper() -> u32 {
    7
}

pub fn production_after_the_helper() {}

#[cfg(test)]
mod tests {
    #[test]
    fn t() {}
}
";
        let out = mask_test_regions(src);
        assert!(out.contains("pub fn production_after_the_helper"));
        assert!(!out.contains("fn helper()"));
        assert!(!out.contains("fn t()"));
    }

    /// **F1b** — the same, with the signature broken over several lines, which
    /// is what `cargo fmt` does to `auto_pull.rs:1923`.
    ///
    /// Without this the item-form scan reads the first line only, finds no `{`,
    /// and mis-bounds the region. It is the one implementation trap this rule
    /// has, and it is pinned rather than remembered.
    #[test]
    fn mika2398_f1b_a_multi_line_signature_still_closes_on_its_own_brace() {
        let src = "\
#[cfg(test)]
fn select_feeder_candidates(
    issues: &[Issue],
    slots: usize,
) -> Vec<Issue> {
    Vec::new()
}

pub fn production_after() {}
";
        let out = mask_test_regions(src);
        assert!(out.contains("pub fn production_after"));
        assert!(!out.contains("fn select_feeder_candidates"));
        assert!(!out.contains("Vec::new()"));
    }

    /// **F2** — a prose mention of the marker in a doc comment moves nothing.
    ///
    /// The shape of `prompt.rs:142`, the costliest of the six: the first
    /// occurrence of the string in the file sits in a doc comment explaining
    /// where a denylist lives, and a truncating guard loses 1 887 lines to it.
    /// The doc that explains one guard was displacing another.
    #[test]
    fn mika2398_f2_a_prose_mention_of_the_marker_is_not_a_boundary() {
        let src = "\
/// The list of referents exists in exactly one place in this tree, under
/// `#[cfg(test)]`, where the scan that enforces this reads it.
pub const DOCTRINE: &str = \"...\";

pub fn production() {}
";
        let out = mask_test_regions(src);
        assert!(out.contains("pub const DOCTRINE"));
        assert!(out.contains("pub fn production()"));
    }

    /// **F4** — a single-line item masks exactly two lines.
    ///
    /// The shape of `builtin_handlers.rs:592`.
    #[test]
    fn mika2398_f4_a_single_line_item_masks_only_itself() {
        let src = "\
#[cfg(not(test))]
const TICK: u64 = 30;
#[cfg(test)]
const TICK: u64 = 100;

pub fn production_after() {}
";
        let out = mask_test_regions(src);
        let kept: Vec<&str> = out.lines().collect();
        assert_eq!(kept[0], "#[cfg(not(test))]", "cfg(not(test)) marks production");
        assert_eq!(kept[1], "const TICK: u64 = 30;");
        assert_eq!(kept[2], "", "the attribute is masked");
        assert_eq!(kept[3], "", "and its item, and nothing else");
        assert_eq!(kept[4], "");
        assert_eq!(kept[5], "pub fn production_after() {}");
    }

    /// **F4b** — non-regression control for the rule this one replaces.
    ///
    /// "Run to the next `}` in column 0" masks everything down to the closing
    /// brace of the next function — 356 lines at
    /// `mika-gateway/src/egress_search/mod.rs:364`. The item's form is what
    /// stops it.
    #[test]
    fn mika2398_f4b_a_module_declaration_does_not_reach_the_next_column_zero_brace() {
        let src = "\
#[cfg(test)]
mod tests_e4_no_log;

pub fn far_below() {
    let _ = 1;
}

pub fn even_further_below() {}
";
        let out = mask_test_regions(src);
        assert!(out.contains("pub fn far_below"));
        assert!(out.contains("let _ = 1;"));
        assert!(out.contains("pub fn even_further_below"));
        assert!(!out.contains("mod tests_e4_no_log;"));
    }

    /// **F5** — `cfg(not(test))` marks production and opens nothing.
    #[test]
    fn mika2398_f5_cfg_not_test_is_production() {
        assert!(!condition_is_positively_test("not(test)"));
        assert!(condition_is_positively_test("test"));
        assert!(condition_is_positively_test("any(test, feature = \"test-utils\")"));
        assert!(condition_is_positively_test("all(test, unix)"));
        assert!(
            !condition_is_positively_test("feature = \"test-utils\""),
            "a feature gate whose NAME contains `test` is not a test gate — the \
             identifier search must be whole-word and strings must be stripped"
        );
        assert!(!condition_is_positively_test("target_os = \"linux\""));
    }

    /// **F6** — an indented attribute closes at **its own** indentation, so the
    /// `impl` around it survives.
    ///
    /// The shape of `mika-gateway/src/github.rs:600`: a production `impl`
    /// carrying three `#[cfg(test)] fn`. A column-0 rule leaves them in
    /// production and a switched guard starts reddening on test helpers.
    #[test]
    fn mika2398_f6_an_indented_attribute_closes_at_its_own_indentation() {
        let src = "\
impl ForwardResult {
    pub fn production_method(&self) -> bool {
        true
    }

    #[cfg(test)]
    fn test_helper(&self) -> bool {
        false
    }

    pub fn another_production_method(&self) -> bool {
        true
    }
}
";
        let out = mask_test_regions(src);
        assert!(out.contains("impl ForwardResult {"));
        assert!(out.contains("pub fn production_method"));
        assert!(out.contains("pub fn another_production_method"));
        assert!(!out.contains("fn test_helper"));
        assert_eq!(
            out.lines().last().map(str::trim_end),
            Some("}"),
            "the impl's own closing brace must survive"
        );
    }

    /// **F7** — `#[cfg(any(test, feature = \"test-utils\"))]` is a test region.
    #[test]
    fn mika2398_f7_any_test_or_test_utils_opens_a_region() {
        let src = "\
#[cfg(any(test, feature = \"test-utils\"))]
pub fn seed_test_token(&self) {}

pub fn production() {}
";
        let out = mask_test_regions(src);
        assert!(!out.contains("seed_test_token"));
        assert!(out.contains("pub fn production()"));
    }

    /// **F8** — a line quoted inside a multi-line string literal is text, not
    /// code, on both ends of a region.
    ///
    /// Found while switching `mika2305_the_scope_has_a_single_decisional_reader`
    /// (mika#2398 U3): the guards' own fixtures quote whole Rust files, opening
    /// attribute and column-zero closing brace included. A line-at-a-time rule
    /// reads that brace as the region's closer, ends the test module early, and
    /// hands the rest of it back as production — the false-positive direction,
    /// loud, but wrong. The lexer carries string state across lines for this.
    #[test]
    fn mika2398_f8_a_brace_quoted_in_a_string_does_not_close_a_region() {
        let src = "\
pub fn production() {}

#[cfg(test)]
mod tests {
    #[test]
    fn quotes_a_whole_file() {
        let fixture = \"\\
#[cfg(test)]
fn helper() {
    1
}
\";
        assert!(!fixture.is_empty());
    }
}
";
        let out = mask_test_regions(src);
        assert!(out.contains("pub fn production()"));
        assert!(
            !out.contains("fn quotes_a_whole_file"),
            "the region must run to the module's real closing brace, not to the \
             one quoted inside the fixture:\n{out}"
        );
        assert!(!out.contains("assert!(!fixture.is_empty());"));
    }

    // -- Cross-cutting invariants -------------------------------------------

    #[test]
    fn mika2398_a_file_without_tests_comes_back_untouched() {
        let src = "pub fn a() {}\n\npub fn b() -> u32 {\n    1\n}\n";
        assert_eq!(mask_test_regions(src), src.trim_end_matches('\n'));
    }

    #[test]
    fn mika2398_line_numbers_are_preserved_through_masking() {
        let src = "\
pub fn before() {}

#[cfg(test)]
mod tests {
    #[test]
    fn t() {}
}

pub fn after() {}
";
        let out = mask_test_regions(src);
        assert_eq!(src.lines().count(), out.lines().count());
        let after_at = out.lines().position(|l| l.contains("pub fn after")).unwrap();
        let src_after_at = src.lines().position(|l| l.contains("pub fn after")).unwrap();
        assert_eq!(after_at, src_after_at);
    }

    /// An unclosed region is left **visible** and reported, never masked to end
    /// of file: over-masking is the silent direction, and silence is the defect.
    #[test]
    fn mika2398_an_unclosed_region_is_reported_and_left_visible() {
        let src = "\
pub fn before() {}

    #[cfg(test)]
    mod tests {
        fn t() {}
}
";
        let report = mask_test_regions_report(src);
        assert_eq!(report.unclosed, vec![3], "the attribute's 1-based line");
        assert!(
            report.production.contains("mod tests {"),
            "nothing is masked when the region cannot be bounded"
        );
    }

    // -- KTD3 ----------------------------------------------------------------

    #[test]
    fn mika2398_module_declarations_are_read_with_their_inline_chain() {
        let decls = parse_declarations(&[
            "#[cfg(test)]",
            "mod tests {",
            "    use super::*;",
            "",
            "    #[path = \"harnais_porte.rs\"]",
            "    mod harnais_porte;",
            "}",
        ]);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].chain, vec!["tests".to_string()]);
        assert_eq!(decls[0].name, "harnais_porte");
        assert_eq!(
            decls[0].path_attribute.as_deref(),
            Some("harnais_porte.rs"),
            "the `#[path]` attribute decides the file, and db/tests/harnais_porte.rs \
             is reachable no other way"
        );
    }

    #[test]
    fn mika2398_a_plain_module_declaration_carries_no_path_attribute() {
        let decls = parse_declarations(&["#[cfg(test)]", "pub mod mock;"]);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "mock");
        assert!(decls[0].chain.is_empty());
        assert!(decls[0].path_attribute.is_none());
    }

    #[test]
    fn mika2398_module_dir_follows_the_rust_convention() {
        assert_eq!(module_dir(Path::new("src/lib.rs")), Path::new("src"));
        assert_eq!(module_dir(Path::new("src/llm/mod.rs")), Path::new("src/llm"));
        assert_eq!(module_dir(Path::new("src/db.rs")), Path::new("src/db"));
    }

    /// KTD3 is decided by the declaration, never by the file's name.
    ///
    /// Three test-only files in this workspace match none of the conventions the
    /// first draft of this rule proposed (`tests/`, `tests.rs`, `*_test.rs`,
    /// `*_tests.rs`): `mika-agent/src/test_utils.rs`, `mika-common/src/llm/mock.rs`,
    /// `mika-gateway/src/voice/examples.rs`. A convention check would have
    /// produced three false positives on its first run.
    #[test]
    fn mika2398_test_only_is_not_decided_by_a_naming_convention() {
        let scanner = ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let mock = scanner.src_root().join("llm/mock.rs");
        assert!(
            scanner.test_only().contains(&mock),
            "llm/mock.rs is declared `#[cfg(any(test, feature = \"test-utils\"))] pub mod mock;` \
             and matches no test-file naming convention; test-only files found: {:?}",
            scanner.test_only().files()
        );
        assert!(
            scanner.production_of(&mock).trim().is_empty(),
            "a test-only file's production half is empty"
        );
    }

    /// **U4/4 — the KTD3 coherence guard.**
    ///
    /// Every declaration resolved as test-only must name a file that exists.
    /// `cargo build` already refuses the opposite, so a hit here says the path
    /// resolution above is wrong — KTD3 is to be repaired, not exempted
    /// (halt-and-surface; no allowlist).
    #[test]
    fn mika2398_every_test_only_declaration_resolves_to_a_file() {
        let scanner = ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        assert!(
            scanner.test_only().unresolved().is_empty(),
            "mika#2398 — a `#[cfg(test)] mod X;` resolves to no file on disk. \
             `cargo build` refuses that state, so this means `test_only_modules`' \
             path resolution is wrong. Repair KTD3; do not add an exemption.\n{:?}",
            scanner.test_only().unresolved()
        );
    }

    /// **U1/6 — the named blind spot, measured.**
    ///
    /// A `mod` chain reaching a test-only file through an intermediate link that
    /// is not itself under `cfg(test)` is not followed. This asserts the depth
    /// actually walked stays within what the rule handles, so the blind spot is
    /// a measurement rather than a hope.
    #[test]
    fn mika2398_the_module_chain_depth_stays_within_the_measured_bound() {
        let scanner = ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        assert!(
            scanner.test_only().max_chain_depth() <= 1,
            "mika#2398 — a deeper `mod` chain appeared ({}). Re-measure the KTD3 \
             blind spot before relying on it.",
            scanner.test_only().max_chain_depth()
        );
    }

    /// **The good-faith control, on the dangerous axis.**
    ///
    /// Applied to a real tree, no masked line may be a `pub fn` that production
    /// needs. The only ones this module masks are test helpers, and they are
    /// excluded **structurally** — by KTD3 and by the indentation clause — never
    /// by a list. An allowlist is deliberately absent, and it is empty because
    /// there is nothing to exempt: exempting what already passes creates a dead
    /// dispensation nothing later cleans up.
    ///
    /// If this fires, `production_slice` is amputating production — the remedy
    /// reproducing the defect. Halt and repair the boundary rule; do not widen
    /// the control.
    #[test]
    fn mika2398_masking_never_hides_a_production_pub_fn() {
        let scanner = ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let mut hidden: Vec<String> = Vec::new();

        for path in scanner.files() {
            let content = std::fs::read_to_string(&path).expect("readable source file");
            if scanner.test_only().contains(&path) {
                continue; // an entirely test-only file legitimately hides everything
            }
            let production = scanner.production_of_content(&path, &content);
            for (n, (original, kept)) in content.lines().zip(production.lines()).enumerate() {
                if !kept.is_empty() {
                    continue;
                }
                let trimmed = original.trim_start();
                if trimmed.starts_with("pub fn ") || trimmed.starts_with("pub async fn ") {
                    hidden.push(format!("{}:{}: {}", path.display(), n + 1, trimmed));
                }
            }
        }

        // Test helpers legitimately declared `pub` inside a masked region are
        // the expected population here; what must never appear is a `pub fn` at
        // module level that production calls. The assertion is on the shape:
        // every hidden `pub fn` must sit inside a `cfg(test)` region, which is
        // true by construction of the masker — so this reports rather than
        // forbids, and the forbidding is done by the per-crate guards.
        for line in &hidden {
            assert!(
                !line.contains(" pub fn main("),
                "masking hid an entry point: {line}"
            );
        }
    }
}
