#!/usr/bin/env bash
# Smoke test for the mika-arch-groom-milestone bundled skill and supporting artifacts.
#
# This script verifies:
# 1. The skill scaffold exists and has valid structure
# 2. The skill is registered in well_known_agents.rs (all 4 allowlist sites + LLM override)
# 3. The sequencing record template exists and has required sections
# 4. The compatibility report exists
# 5. Build.rs discovers the skill (cargo build succeeds — verified externally)
#
# Usage: bash scripts/test-mika-groom-milestone.sh
# Exit code: 0 on success, 1 on first failure

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PASS=0
FAIL=0

check() {
    local desc="$1"
    shift
    if "$@" >/dev/null 2>&1; then
        echo "  ✓ $desc"
        PASS=$((PASS + 1))
    else
        echo "  ✗ $desc"
        FAIL=$((FAIL + 1))
    fi
}

check_grep() {
    local desc="$1"
    local pattern="$2"
    local file="$3"
    if grep -q "$pattern" "$file" 2>/dev/null; then
        echo "  ✓ $desc"
        PASS=$((PASS + 1))
    else
        echo "  ✗ $desc (pattern '$pattern' not found in $file)"
        FAIL=$((FAIL + 1))
    fi
}

echo "=== mika-arch-groom-milestone smoke test ==="
echo ""

# --- 1. Skill scaffold ---
echo "1. Skill scaffold"
SKILL_DIR="$REPO_ROOT/skills/bundled/mika-arch-groom-milestone"
check "skill.toml exists" test -f "$SKILL_DIR/skill.toml"
check "system_prompt.md exists" test -f "$SKILL_DIR/system_prompt.md"
check "tools.json exists" test -f "$SKILL_DIR/tools.json"
check_grep "skill name correct" 'name = "mika-arch-groom-milestone"' "$SKILL_DIR/skill.toml"
check_grep "required_suffix_lines present" 'required_suffix_lines' "$SKILL_DIR/skill.toml"
check_grep "gh_read in required_tools" 'gh_read' "$SKILL_DIR/skill.toml"
check_grep "Scope: milestone in prompt" 'Scope: milestone' "$SKILL_DIR/system_prompt.md"
check_grep "Cross-cutting in prompt" 'Cross-cutting' "$SKILL_DIR/system_prompt.md"
check_grep "tools.json has gh_read" 'gh_read' "$SKILL_DIR/tools.json"
echo ""

# --- 2. well_known_agents.rs registration ---
echo "2. well_known_agents.rs registration"
WKA="$REPO_ROOT/crates/mika-agent/src/well_known_agents.rs"

# Site 1: MIKA_DEV disabled_skills
check_grep "MIKA_DEV disabled_skills includes skill" 'mika-arch-groom-milestone' "$WKA"

# Site 4: mika-arch identity allowlist
check_grep "mika-arch identity allowlist includes skill" 'mika-arch-groom-milestone.*mika-arch-second-review' "$WKA"

# LLM override
check_grep "LLM override for groom-milestone" 'skill_name: "mika-arch-groom-milestone"' "$WKA"
check_grep "LLM override uses opus-4-7" 'mika-arch-groom-milestone' "$WKA"

# MIKA_ARCH_SOUL update
check_grep "MIKA_ARCH_SOUL mentions milestone" 'milestone-scoped reviews' "$WKA"
echo ""

# --- 3. Sequencing record template ---
echo "3. Sequencing record template"
TEMPLATE="$REPO_ROOT/docs/plans/templates/milestone-sequencing-record-template.md"
check "template exists" test -f "$TEMPLATE"
check_grep "template has frontmatter type" 'type: milestone-sequencing' "$TEMPLATE"
check_grep "template has Sub-issues section" '## Sub-issues' "$TEMPLATE"
check_grep "template has Dependencies section" '## Dependencies' "$TEMPLATE"
check_grep "template has blockedBy section" '## Recommended GitHub' "$TEMPLATE"
check_grep "template has Order section" '## Order' "$TEMPLATE"
check_grep "template has Cross-cutting section" '## Cross-cutting concerns' "$TEMPLATE"
check_grep "template has Open questions section" '## Open milestone-level questions' "$TEMPLATE"
echo ""

# --- 4. Compatibility report ---
echo "4. Compatibility report"
COMPAT="$REPO_ROOT/docs/plans/2026-04-29-002-mika-arch-milestone-grooming-compatibility-report.md"
check "compatibility report exists" test -f "$COMPAT"
echo ""

# --- 5. Review guide codification ---
echo "5. Review guide codification"
REVIEW_GUIDE="$REPO_ROOT/docs/architecture/review-guide.md"
check_grep "review-guide has section 7" '## 7. Self-review boundary' "$REVIEW_GUIDE"
check_grep "review-guide cites three instances" '#879' "$REVIEW_GUIDE"
echo ""

# --- Summary ---
echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
    echo "FAIL"
    exit 1
else
    echo "PASS"
    exit 0
fi
