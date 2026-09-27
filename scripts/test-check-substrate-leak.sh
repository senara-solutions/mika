#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-substrate-leak.sh (mika#1964).
#
# "Delete the thing the test protects; confirm the test goes red."
#
# The founding defect of mika#1964 is a guard that did not exist while a test
# claimed it did: `web_search_family_tier_http_401_no_leak` named a companion
# source-scan, `web_search_no_raw_401_operator_error`, that was nowhere in the
# tree — and the test itself never called the handler, so it stayed green across
# the change (mika#1971) that reopened the leak. A lint nobody has watched go red
# is a decoration.
#
# THE FOUR NEGATIVE CONTROLS ARE THE DELIVERABLE. The positive cases below only
# prove the script parses; N1–N4 are what prove it bites, and each of them covers
# a failure the others cannot see.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-substrate-leak.sh"

PASS=0
FAIL=0

# Run the guard against scan root $1; assert its exit equals $2. $3 = case name.
assert_exit() {
    local root="$1" want="$2" name="$3"
    local got=0
    bash "$GUARD" "$root" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

# A throwaway perimeter holding one tools/*.rs file whose body is $1.
make_tool_fixture() {
    local dir
    dir="$(mktemp -d)"
    mkdir -p "$dir/tools"
    printf '%s\n' "$1" > "$dir/tools/probe.rs"
    echo "$dir"
}

# A throwaway perimeter holding skills/builtin_handlers.rs whose body is $1.
# Used for the truncation and good-faith cases, which key on that basename.
make_handlers_fixture() {
    local dir
    dir="$(mktemp -d)"
    mkdir -p "$dir/skills"
    printf '%s\n' "$1" > "$dir/skills/builtin_handlers.rs"
    echo "$dir"
}

# ── 1. The real, swept tree is clean.
assert_exit "$REPO_ROOT/crates/mika-agent/src" 0 "swept agent tree is clean"

# ── 2. N1 — THE EXACT LITERAL OF THE FOUNDING DEFECT, in a function SEPARATE
#       from any constructor.
#
# This is the case the ticket's own acceptance criterion cannot catch: it asks to
# reject "`ToolOutput::error(...)` calls with substrate-token strings", and this
# literal appears on no line carrying `ToolOutput::error`. If it goes green, the
# guard does not cover the defect that produced it, whatever the other rules do.
d="$(make_tool_fixture 'fn map_substrate_error(status: u16) -> String {
    "Search substrate rejected upstream credentials. \
     Ask the operator to rotate MIKA_BRAVE_API_KEY on mika-gateway."
        .to_string()
}

fn unrelated() -> ToolOutput {
    ToolOutput::error("bad input".to_string())
}')"
assert_exit "$d" 1 "N1: the mika#1964 literal is rejected in a function of its own"
rm -rf "$d"

# 2b. ...and the same literal reached through the coupled helper is NOT a
#     violation once annotated — otherwise the remedy itself is unusable and the
#     guard gets disarmed.
d="$(make_tool_fixture 'async fn handler(ctx: &ToolContext<'"'"'_>) -> ToolOutput {
    crate::tools::dispatch_substrate_unavailable(
        "Web search is unavailable.",
        // substrate-diagnostic: the operator channel.
        "Ask the operator to rotate MIKA_BRAVE_API_KEY on mika-gateway.",
        "web_search",
        ctx,
    )
    .await
}')"
assert_exit "$d" 0 "N1b: the annotated diagnostic of the coupled helper passes"
rm -rf "$d"

# ── 3. N2 — TRUNCATION. A leaking literal placed AFTER a
#       `#[cfg(not(test))]` / `#[cfg(test)]` pair must still be seen.
#
# The obvious cut — at the first `#[cfg(test)]` — is wrong on the one file that
# matters: builtin_handlers.rs carries such a pair on PROGRESS_TICKER_INTERVAL
# around line 600 while its test module starts near 5265, so the naive cut leaves
# ~4700 lines of production unscanned WHILE THE GUARD EXITS 0. This is the only
# case whose failure means "the guard is not looking where it believes it looks"
# rather than "a rule is too narrow".
d="$(make_handlers_fixture '#[cfg(not(test))]
const PROGRESS_TICKER_INTERVAL: Duration = Duration::from_secs(10);
#[cfg(test)]
const PROGRESS_TICKER_INTERVAL: Duration = Duration::from_millis(1);

async fn run_gws(ctx: &ToolContext) -> ToolOutput {
    ToolOutput::error("Set MIKA_GITHUB_TOKEN or configure a GitHub App.".to_string())
}

#[cfg(test)]
mod tests {
    const FORBIDDEN: &[&str] = &["MIKA_BRAVE_API_KEY", "config.toml"];
}')"
assert_exit "$d" 1 "N2: a literal after a cfg(not(test))/cfg(test) pair is still scanned"
rm -rf "$d"

# 3b. Its mirror — a literal INSIDE `mod tests` must NOT fire. The tests assert on
#     these very tokens (FORBIDDEN_FAMILY_TIER_TOKENS contains MIKA_BRAVE_API_KEY
#     and config.toml literally), so without this the guard is permanently red on
#     its own perimeter and gets allowlisted into uselessness.
d="$(make_handlers_fixture 'async fn run_gws(ctx: &ToolContext) -> ToolOutput {
    ToolOutput::success("ok".to_string())
}

#[cfg(test)]
mod tests {
    const FORBIDDEN: &[&str] = &["MIKA_BRAVE_API_KEY", "config.toml", "GH_TOKEN"];
    fn probe() { let _ = ToolOutput::substrate_unavailable("a", "MIKA_ROUTING_URL"); }
}')"
assert_exit "$d" 0 "N2b: literals inside the test module are out of the population"
rm -rf "$d"

# ── 4. N3 — GOOD FAITH. A production slice that lost its declared marker must go
#       red LOUDLY (exit 2), never green on the empty set.
d="$(make_handlers_fixture 'fn something_else() {}

#[cfg(test)]
mod tests {
    fn probe() {}
}')"
assert_exit "$d" 2 "N3: a production slice missing its good-faith marker exits 2"
rm -rf "$d"

# 4b. An empty perimeter is likewise exit 2, not a clean bill of health — a guard
#     with no population reads exactly like a clean tree (mika#2205).
d="$(mktemp -d)"
assert_exit "$d" 2 "N3b: an empty perimeter exits 2, not 0"
rm -rf "$d"

# ── 5. N4 — ANNOTATION SCOPE, BOTH DIRECTIONS.
#
# An annotation must cover the string-continuation lines that follow it: the
# offending literals are multi-line `format!`s whose lines end in `\`, so writing
# `//` inside one would put the comment inside the string.
d="$(make_tool_fixture 'fn remediation() -> String {
    // substrate-ok: a warn! field, never a tool-result content.
    format!(
        "Merge credential lacks write access. The fix is to install the mika \
         GitHub App on it, or to grant the configured PAT the repo scope. \
         Set MIKA_GITHUB_TOKEN if you use a PAT."
    )
}')"
assert_exit "$d" 0 "N4: an annotation covers the string continuation that follows it"
rm -rf "$d"

# 5b. ...and must NOT cover a literal beyond the window. Without this direction the
#     window is a hole no test measures: one annotation at the top of a file would
#     exempt everything below it.
d="$(make_tool_fixture 'fn annotated() -> String {
    // substrate-ok: this one is genuinely out of the population.
    "nothing to see".to_string()
}

fn filler_1() {}
fn filler_2() {}
fn filler_3() {}
fn filler_4() {}
fn filler_5() {}
fn filler_6() {}
fn filler_7() {}
fn filler_8() {}
fn filler_9() {}
fn filler_10() {}

fn leaks() -> String {
    "Set MIKA_ROUTING_URL on mika-spirit.".to_string()
}')"
assert_exit "$d" 1 "N4b: an annotation does NOT exempt a literal beyond the window"
rm -rf "$d"

# ── 6. Rule 1 — the bare constructor has a single production site.
#
# Allowlist shipped empty: when this fires the resolution is to remove the second
# site, never to exempt it (mika#2201). The coupled helper
# `dispatch_substrate_unavailable` is the one site, which is what makes the
# construct-then-route footgun inexpressible rather than merely detected.
d="$(make_tool_fixture 'async fn some_handler(ctx: &ToolContext<'"'"'_>) -> ToolOutput {
    let mut out = ToolOutput::substrate_unavailable("neutral", "detail");
    crate::tools::dispatch_substrate_diagnostic(&mut out, "some_tool", ctx).await;
    out
}')"
assert_exit "$d" 1 "Rule 1: a bare constructor outside the coupled helper is rejected"
rm -rf "$d"

# 6b. ...and inside the helper it is the expected shape.
d="$(make_tool_fixture 'pub async fn dispatch_substrate_unavailable(
    fallback: impl Into<String>,
    diagnostic: impl Into<String>,
    tool_name: &str,
    ctx: &ToolContext<'"'"'_>,
) -> ToolOutput {
    let mut out = ToolOutput::substrate_unavailable(fallback, diagnostic);
    dispatch_substrate_diagnostic(&mut out, tool_name, ctx).await;
    out
}')"
assert_exit "$d" 0 "Rule 1: the coupled helper is the one permitted site"
rm -rf "$d"

# ── 7. Comment lines are out of the population — documenting an env var is house
#      style, and a comment never reaches a tool-result content. Without this the
#      guard accuses dozens of doc-comments and is born red.
d="$(make_tool_fixture '/// Reads `MIKA_ROUTING_URL` from the settings cascade.
///
/// See `~/.mika/config.toml` and the `GH_TOKEN` relay.
//! Module-level: MIKA_INTERNAL_TOKEN is resolved on the gateway.
fn documented() {}')"
assert_exit "$d" 0 "comment lines (//, ///, //!) are not scanned"
rm -rf "$d"

# 7b. ...but a literal on a line that merely LOOKS commented is still caught — the
#      exclusion is on the first non-blank characters, not on containing "//".
d="$(make_tool_fixture 'fn tricky() -> String {
    "see https://example.com — Set MIKA_ROUTING_URL first".to_string()
}')"
assert_exit "$d" 1 "a URL inside a literal does not make the line a comment"
rm -rf "$d"

echo ""
echo "check-substrate-leak anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
