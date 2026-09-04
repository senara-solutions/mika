#!/usr/bin/env bash
# CI lint: an image-push workflow may only push tags that are a function of the
# commit sha.
#
# THE PROPERTY, not a list of tag names:
#   The three ECR repositories this platform pushes to — mika-agent,
#   mika-gateway, mika-console — are declared IMMUTABLE. An IMMUTABLE
#   repository refuses to reassign a tag it already carries. A "moving" tag is
#   by definition a tag you reassign. So any tag in a `tags:` list that is not
#   derived from the commit sha is a tag the *second* build will fail to push —
#   and, when the tag already exists in the registry, the *first* one too.
#
# THE FOUNDING INCIDENT is mika#2143: the workflow shipped by PR#2093 pushed
# `:${{ github.sha }}` and `:latest` side by side, and `latest` had been sitting
# on mika-agent since 2026-06-09. The workflow could therefore never have
# succeeded once past authentication. The bug is not that someone chose a bad
# tag name; it is that a moving tag and an immutable registry are a
# contradiction in terms, and nothing in the repo said so out loud.
#
# WHEN YOU EXTEND THIS SCRIPT, EXTEND IT BY PROPERTY.
#   This guard deliberately does not grep for the string `latest`. A guard that
#   knows one *spelling* of a defect lets every other spelling through — that is
#   the mika#2103 lesson, learned the expensive way by check-byte-slices.sh.
#   `:stable`, `:main`, `:prod` and `:dev` are the same defect wearing a
#   different name, and each of them is caught here because the rule is
#   "derived from the sha", not "not called latest".
#
# KNOWN BOUNDARY, stated rather than left to be discovered: this guard reads
#   `tags:`. docker/build-push-action can also name an image through
#   `outputs: type=image,name=...`, which this parser does not model. That path
#   is not silently green, though — removing `tags:` makes the guard exit 3
#   ("nothing to check" is not "clean"), so the *substitution* is caught. Only
#   an `outputs:` added ALONGSIDE a sha-derived `tags:` would slip past. If that
#   ever becomes a real shape here, extend this by property, not by adding a
#   second grep for one more spelling.
#
# THE REMEDY, when this fires, is to delete the offending tag rather than to
# make the repository mutable. Immutability is a provenance guarantee — a
# deployed tag always designates the content it was created with — and it is
# not traded for a naming convenience. A consumer that wants "the newest build"
# resolves it: `aws ecr describe-images` sorted on `imagePushedAt`.
#
# Usage: check-image-tags-immutable.sh [workflow.yml]
#   Defaults to the repo's agent-image-build-push.yml. An explicit argument
#   lets the anti-vacuity harness point the guard at a fixture (see
#   scripts/test-check-image-tags-immutable.sh).
#
# Exit 0 clean, 1 on a moving tag, 2 if the workflow file cannot be read,
# 3 if no `tags:` list was found at all (a guard that finds nothing to check
# must say so rather than pass).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKFLOW="${1:-$REPO_ROOT/.github/workflows/agent-image-build-push.yml}"

if [[ ! -f "$WORKFLOW" ]]; then
    echo "ERROR: workflow file not readable: $WORKFLOW" >&2
    exit 2
fi

# Extract every entry of every `tags:` list in the file.
#
# All four YAML writings of a tag list are handled on purpose — a moving tag
# reintroduced in flow style would otherwise walk straight past a block-scalar
# parser, which is the same "one spelling" failure this guard exists to reject:
#
#   tags: |            (block scalar, the shape the workflow uses today)
#   tags:              (block sequence, `- item` lines)
#   tags: [a, b]       (flow sequence)
#   tags: single       (plain scalar on the same line)
extract_tags() {
    awk '
        # Inside a block: a line indented deeper than `tags:` is an entry.
        intag == 1 {
            if ($0 ~ /^[ \t]*$/) { next }
            match($0, /^[ \t]*/); ind = RLENGTH
            if (ind <= base) {
                intag = 0          # block closed; fall through to the rules below
            } else {
                v = $0
                sub(/^[ \t]+/, "", v); sub(/[ \t]+$/, "", v)
                sub(/^-[ \t]+/, "", v)                 # block-sequence dash
                # A `#` line inside the block is a comment a human wrote, not a
                # tag anyone means to push. Flagging it would be noise, and noise
                # is how a guard gets allowlisted into uselessness.
                if (v ~ /^#/) { next }
                gsub(/^["'"'"']|["'"'"']$/, "", v)     # surrounding quotes
                if (v != "") { print v }
                next
            }
        }

        /^[ \t]*tags:/ {
            found = 1
            match($0, /^[ \t]*/); base = RLENGTH
            rest = $0
            sub(/^[ \t]*tags:[ \t]*/, "", rest)
            sub(/[ \t]+$/, "", rest)

            # `tags: |`, `tags: |-`, `tags: >`, or a bare `tags:` — entries follow.
            if (rest == "" || rest ~ /^[|>][-+0-9]*$/) { intag = 1; next }

            # `tags: [a, b]`
            if (rest ~ /^\[.*\]$/) {
                sub(/^\[/, "", rest); sub(/\]$/, "", rest)
                n = split(rest, parts, ",")
                for (i = 1; i <= n; i++) {
                    v = parts[i]
                    gsub(/^[ \t]+|[ \t]+$/, "", v)
                    gsub(/^["'"'"']|["'"'"']$/, "", v)
                    if (v != "") { print v }
                }
                next
            }

            # `tags: <single value>`
            v = rest
            gsub(/^["'"'"']|["'"'"']$/, "", v)
            if (v != "") { print v }
            next
        }

        END { if (!found) { exit 3 } }
    ' "$1"
}

TAGS=""
awk_status=0
TAGS="$(extract_tags "$WORKFLOW")" || awk_status=$?

if [[ $awk_status -eq 3 ]]; then
    echo "ERROR: no \`tags:\` list found in $WORKFLOW" >&2
    echo "This guard has nothing to check, which is not the same as a clean file." >&2
    exit 3
elif [[ $awk_status -ne 0 ]]; then
    echo "ERROR: could not parse $WORKFLOW (awk exit $awk_status)" >&2
    exit 2
fi

if [[ -z "$TAGS" ]]; then
    echo "ERROR: the \`tags:\` list in $WORKFLOW is empty." >&2
    exit 3
fi

# A tag is acceptable only if the commit sha appears in it: two distinct
# commits then cannot produce the same tag, so no push can ever reassign one.
VIOLATIONS=0
while IFS= read -r tag; do
    [[ -z "$tag" ]] && continue
    if [[ "$tag" == *'github.sha'* || "$tag" == *'GITHUB_SHA'* ]]; then
        continue
    fi
    echo "ERROR: moving tag pushed to an IMMUTABLE registry: $tag"
    VIOLATIONS=$((VIOLATIONS + 1))
done <<< "$TAGS"

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "Found $VIOLATIONS tag(s) not derived from the commit sha in $WORKFLOW."
    echo "The ECR repositories are IMMUTABLE: a tag that does not move with the"
    echo "commit can be pushed once and never again, so the next merge fails."
    echo "This is mika#2143 — the workflow shipped with a \`latest\` alongside the"
    echo "sha tag, and \`latest\` already existed in the registry."
    echo ""
    echo "Fix: drop the tag. Do NOT make the repository mutable — immutability is"
    echo "the provenance guarantee, not an obstacle. A consumer wanting the newest"
    echo "build resolves it with \`aws ecr describe-images\` sorted on imagePushedAt."
    exit 1
fi

echo "All image tags are derived from the commit sha ($WORKFLOW)."
exit 0
