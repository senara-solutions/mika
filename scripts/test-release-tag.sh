#!/usr/bin/env bash
#
# Offline harness for scripts/release-tag.sh (mika#2890).
#
# The script decides whether a tag may be published and whether it is a release
# candidate. That second answer is what keeps a candidate away from the family:
# install.sh installs `releases/latest`, and only a release flagged prerelease is
# kept out of it. A classifier that answered `prerelease=false` for `-rc.N` would
# publish a candidate as the version everyone installs, so every row below is a
# case where a wrong answer reaches users or blocks the launch.
#
# Fixture manifests are shaped on the REAL root Cargo.toml: a `[workspace]`
# table before `[workspace.package]`, and a dependency carrying its own
# `version =` key after it. The second shape is the trap: a scan that takes the
# first or last `version =` of the file reads the wrong one.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/release-tag.sh"

PASS=0
FAIL=0

FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT

# Echo the path of a manifest whose workspace version is $1.
make_manifest() {
    local version="$1" dir
    dir="$(mktemp -d -p "$FIXTURE_ROOT")"
    printf '%s\n' \
        '[workspace]' \
        'members = ["crates/*"]' \
        'resolver = "2"' \
        '' \
        '[workspace.package]' \
        "version = \"$version\"" \
        'edition = "2024"' \
        '' \
        '[workspace.dependencies]' \
        'tokio = { version = "1", features = ["full"] }' \
        'serde = "1"' \
        > "$dir/Cargo.toml"
    echo "$dir/Cargo.toml"
}

# assert_class <name> <tag> <manifest> <expected prerelease value>
assert_class() {
    local name="$1" tag="$2" manifest="$3" want="$4" out got=0
    out="$(bash "$SCRIPT" "$tag" "$manifest" 2>/dev/null)" || got=$?
    if [ "$got" -eq 0 ] && printf '%s\n' "$out" | grep -qx "prerelease=$want" \
        && printf '%s\n' "$out" | grep -qx "tag=$tag"; then
        PASS=$((PASS + 1)); echo "ok   $name"
    else
        FAIL=$((FAIL + 1)); echo "FAIL $name (exit=$got, out=$(printf '%s' "$out" | tr '\n' ' '))"
    fi
}

# assert_refused <name> <args...>
assert_refused() {
    local name="$1" got=0
    shift
    bash "$SCRIPT" "$@" >/dev/null 2>&1 || got=$?
    if [ "$got" -ne 0 ]; then
        PASS=$((PASS + 1)); echo "ok   $name"
    else
        FAIL=$((FAIL + 1)); echo "FAIL $name (accepted, should have been refused)"
    fi
}

V100="$(make_manifest 1.0.0)"

assert_class   "final tag is not a prerelease"          v1.0.0       "$V100" false
assert_class   "rc tag is a prerelease"                 v1.0.0-rc.1  "$V100" true
assert_class   "two-digit rc is a prerelease"           v1.0.0-rc.12 "$V100" true
assert_refused "base differs from workspace version"    v1.0.1       "$V100"
assert_refused "rc of another version"                  v1.0.1-rc.1  "$V100"
assert_refused "unknown pre-release form"               v1.0.0-beta  "$V100"
assert_refused "rc without its number"                  v1.0.0-rc.   "$V100"
assert_refused "missing v prefix"                       1.0.0        "$V100"
assert_refused "trailing garbage"                       v1.0.0-rc.1x "$V100"
assert_refused "missing manifest argument"              v1.0.0
assert_refused "manifest file absent"                   v1.0.0       "$FIXTURE_ROOT/absent/Cargo.toml"

# A [workspace.package] table without its own `version`: the next table carries
# a line-leading `version = "1.0.0"`, which must not be read as the workspace's.
NOVER_DIR="$(mktemp -d -p "$FIXTURE_ROOT")"
printf '%s\n' '[workspace]' 'members = ["crates/*"]' '' '[workspace.package]' 'edition = "2024"' \
    '' '[workspace.metadata.release]' 'version = "1.0.0"' > "$NOVER_DIR/Cargo.toml"
assert_refused "workspace.package without a version"    v1.0.0       "$NOVER_DIR/Cargo.toml"

# The real manifest must classify a tag of its own version, or the release
# workflow refuses every tag on the day it is needed.
REAL_VERSION="$(bash "$SCRIPT" --workspace-version "$REPO_ROOT/Cargo.toml")"
assert_class   "real Cargo.toml: own version is final"  "v$REAL_VERSION" "$REPO_ROOT/Cargo.toml" false

echo
echo "release-tag: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
