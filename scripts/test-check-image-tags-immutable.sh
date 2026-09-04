#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-image-tags-immutable.sh (mika#2143).
#
# "Delete the thing the test protects; confirm the test goes red."
#
# A guard nobody has watched go red is a decoration — mika#2103 is the incident
# where a lint stayed green through 26 production panics because it knew one
# spelling of the defect. This suite pins the *property* ("every tag is a
# function of the commit sha") across all four YAML writings of a tag list, so
# that reintroducing a moving tag in any of them goes red.
#
# The battery carries accented fixtures on purpose. This repository writes its
# plans, tickets and logs in French, and its worktrees and paths follow: a
# fixture set that is ASCII-only tests a population that does not exist here.
# Both the accented path and the accented content must survive the guard
# intact — the failure message is asserted to reproduce the accents verbatim.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-image-tags-immutable.sh"

PASS=0
FAIL=0

# Run the guard against workflow $1; assert its exit equals $2. $3 = case name.
assert_exit() {
    local file="$1" want="$2" name="$3"
    local got=0
    bash "$GUARD" "$file" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

# Assert the guard's output for workflow $1 contains the literal string $2.
assert_output_contains() {
    local file="$1" needle="$2" name="$3"
    local out
    out="$(bash "$GUARD" "$file" 2>&1 || true)"
    if [[ "$out" == *"$needle"* ]]; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (output did not contain: $needle)"
        FAIL=$((FAIL + 1))
    fi
}

# A throwaway fixture workflow whose body is $1, written at relative path $2
# (default a plain ASCII name) under a fresh temp dir. Echoes the file path.
make_fixture() {
    local body="$1" rel="${2:-wf.yml}" dir
    dir="$(mktemp -d)"
    mkdir -p "$dir/$(dirname "$rel")"
    printf '%s\n' "$body" > "$dir/$rel"
    echo "$dir/$rel"
}

# ── 1. POSITIVE CONTROL — the real, corrected workflow passes.
assert_exit "$REPO_ROOT/.github/workflows/agent-image-build-push.yml" 0 \
    "the repo's own agent-image workflow is clean"

# ── 2. NEGATIVE CONTROL — the exact mika#2143 defect, in the shape it shipped.
#    If this ever goes green, the class is unprotected regardless of the rest.
f="$(make_fixture '          tags: |
            ${{ env.ECR_REGISTRY }}/${{ env.ECR_REPOSITORY }}:${{ github.sha }}
            ${{ env.ECR_REGISTRY }}/${{ env.ECR_REPOSITORY }}:latest')"
assert_exit "$f" 1 "block scalar: the mika#2143 \`latest\` defect is rejected"
rm -rf "$(dirname "$f")"

# ── 3. The rule is "derived from the sha", NOT "not called latest". A moving
#    tag under any other name is the same defect and must bite the same.
for moving in stable main prod dev v1; do
    f="$(make_fixture "          tags: |
            registry/repo:\${{ github.sha }}
            registry/repo:$moving")"
    assert_exit "$f" 1 "block scalar: moving tag \`:$moving\` is rejected (property, not token)"
    rm -rf "$(dirname "$f")"
done

# ── 4. The other three YAML writings of a tag list. A parser that only knows
#    the block scalar would let a reintroduced moving tag walk past — the
#    "one spelling" failure this guard exists to refuse.
f="$(make_fixture '          tags: [registry/repo:${{ github.sha }}, registry/repo:latest]')"
assert_exit "$f" 1 "flow sequence: moving tag is rejected"
rm -rf "$(dirname "$f")"

f="$(make_fixture '          tags:
            - registry/repo:${{ github.sha }}
            - registry/repo:latest')"
assert_exit "$f" 1 "block sequence: moving tag is rejected"
rm -rf "$(dirname "$f")"

f="$(make_fixture '          tags: registry/repo:latest')"
assert_exit "$f" 1 "plain scalar: moving tag is rejected"
rm -rf "$(dirname "$f")"

# ── 5. ...and each writing stays green when every tag is sha-derived, or the
#    guard is a false-positive generator and gets disabled instead of obeyed.
f="$(make_fixture '          tags: |
            registry/repo:${{ github.sha }}')"
assert_exit "$f" 0 "block scalar: sha-only list passes"
rm -rf "$(dirname "$f")"

f="$(make_fixture '          tags: [registry/repo:${{ github.sha }}]')"
assert_exit "$f" 0 "flow sequence: sha-only list passes"
rm -rf "$(dirname "$f")"

f="$(make_fixture '          tags: |
            registry/repo:${GITHUB_SHA}
            registry/repo:${GITHUB_SHA}-amd64')"
assert_exit "$f" 0 "shell-form \${GITHUB_SHA} and a sha-derived suffix both pass"
rm -rf "$(dirname "$f")"

# ── 6. A guard that finds nothing to check must SAY so, not pass. A silently
#    empty parse is indistinguishable from a clean file, and that is exactly how
#    a green check comes to mean nothing.
f="$(make_fixture '          push: true
          platforms: linux/amd64')"
assert_exit "$f" 3 "a workflow with no \`tags:\` list exits 3, not 0"
rm -rf "$(dirname "$f")"

f="$(make_fixture '          tags: |

          push: true')"
assert_exit "$f" 3 "an empty \`tags:\` block exits 3, not 0"
rm -rf "$(dirname "$f")"

assert_exit "/nonexistent/path/to/wf.yml" 2 "an unreadable workflow exits 2, not 0"

# ── 7. ACCENTED FIXTURES — our actual population.
#
# The path carries accents, a space and an apostrophe; the moving tag and an
# in-block comment carry accents too. A guard that mangles UTF-8, or that
# splits an unquoted path, cannot pass these. The failure message is asserted
# to reproduce the accented tag verbatim: an error that garbles the offending
# value is an error nobody can act on.
ACCENTED_BAD='          tags: |
            # le tag immuable, dérivé du commit
            dépôt/mika-agent:${{ github.sha }}
            dépôt/mika-agent:dernière'
f="$(make_fixture "$ACCENTED_BAD" "dépôt d'images/agent-image-build-push.yml")"
assert_exit "$f" 1 "accented path + accented moving tag is rejected"
assert_output_contains "$f" "dépôt/mika-agent:dernière" \
    "the failure message reproduces the accented tag verbatim"
# ...and the accented French comment inside the block is NOT mistaken for a tag.
assert_output_contains "$f" "Found 1 tag(s)" \
    "the accented in-block comment is not counted as a tag"
rm -rf "$(dirname "$f")"

# The accent alone must not manufacture a violation, or the guard is unusable
# on exactly the paths this repo produces.
ACCENTED_OK='          tags: |
            # un seul tag, dérivé du commit — pas de tag mouvant
            dépôt/mika-agent:${{ github.sha }}'
f="$(make_fixture "$ACCENTED_OK" "dépôt d'images/agent-image-build-push.yml")"
assert_exit "$f" 0 "accented path + accented comment, sha-only list, passes"
rm -rf "$(dirname "$f")"

echo ""
echo "check-image-tags-immutable anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
