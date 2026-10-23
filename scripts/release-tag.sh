#!/usr/bin/env bash
#
# Decide whether a tag may be released, and whether it is a release candidate
# (mika#2890).
#
#   release-tag.sh <tag> <Cargo.toml>        -> tag=, base=, prerelease= on stdout
#   release-tag.sh --workspace-version <Cargo.toml>  -> the workspace version alone
#
# Accepted tags are `vX.Y.Z` and `vX.Y.Z-rc.N`, and X.Y.Z must equal the
# `[workspace.package] version` of the manifest. Anything else is refused with a
# non-zero exit, before any job of .github/workflows/release.yml builds or signs.
#
# WHY THE CANDIDATE ANSWER MATTERS. install.sh installs `releases/latest`, and
# GitHub keeps a release out of `latest` only when it is flagged prerelease. So
# `prerelease=true` is what stops a candidate from being served to the family;
# release.yml turns it into `--prerelease --latest=false`. The decision lives
# here rather than in the workflow's YAML so that scripts/test-release-tag.sh
# can hold it offline.

set -euo pipefail

# Print the version of the `[workspace.package]` table, and nothing else: a
# dependency line such as `tokio = { version = "1" }` also carries `version =`.
workspace_version() {
    local manifest="$1"
    [ -f "$manifest" ] || { echo "release-tag: manifest not found: $manifest" >&2; return 1; }
    awk '
        /^\[/ { in_pkg = ($0 == "[workspace.package]"); next }
        in_pkg && /^version[[:space:]]*=/ {
            sub(/^version[[:space:]]*=[[:space:]]*"/, ""); sub(/".*$/, ""); print; exit
        }
    ' "$manifest"
}

if [ "${1:-}" = "--workspace-version" ]; then
    [ $# -eq 2 ] || { echo "usage: release-tag.sh --workspace-version <Cargo.toml>" >&2; exit 2; }
    v="$(workspace_version "$2")"
    [ -n "$v" ] || { echo "release-tag: no [workspace.package] version in $2" >&2; exit 1; }
    echo "$v"
    exit 0
fi

[ $# -eq 2 ] || { echo "usage: release-tag.sh <tag> <Cargo.toml>" >&2; exit 2; }
tag="$1"
manifest="$2"

if [[ "$tag" =~ ^v([0-9]+\.[0-9]+\.[0-9]+)(-rc\.[0-9]+)?$ ]]; then
    base="${BASH_REMATCH[1]}"
    if [ -n "${BASH_REMATCH[2]}" ]; then prerelease=true; else prerelease=false; fi
else
    echo "release-tag: '$tag' is neither vX.Y.Z nor vX.Y.Z-rc.N" >&2
    exit 1
fi

version="$(workspace_version "$manifest")"
if [ -z "$version" ]; then
    echo "release-tag: no [workspace.package] version in $manifest" >&2
    exit 1
fi
if [ "$base" != "$version" ]; then
    echo "release-tag: tag $tag is for $base but the workspace is at $version" >&2
    exit 1
fi

echo "tag=$tag"
echo "base=$base"
echo "prerelease=$prerelease"
