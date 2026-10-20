#!/usr/bin/env bash
# test_rescue_fmt_clean_without_default_toolchain.sh — mika#2348 suite under the
# CI runner's rustup state (mika#2826).
#
# The self-hosted `Check` runner installs rustup with `--default-toolchain none`
# and gets its toolchain from `rust-toolchain.toml` alone: `rustup show` there
# reports "active because: overridden by '.../rust-toolchain.toml'" and no
# default. test_rescue_fmt_clean.sh builds its fixture crate under
# `${TMPDIR:-/tmp}`, outside that override, so on the runner every `cargo fmt`
# in it died with "rustup could not choose a version of cargo to run" — six red
# assertions on the first run of the suite in CI, read as a flake. On a
# workstation with `rustup default stable` the same suite is green, which is how
# it shipped.
#
# This suite reproduces the runner's state on any machine: an empty HOME and a
# RUSTUP_HOME whose settings carry no default toolchain, with the installed
# toolchains linked in. It then runs the mika#2348 suite and requires it green.
# Red before the suite pinned the repo's toolchain for its fixture, green after.
#
# Run: bash skills/bundled/_shared/tests/test_rescue_fmt_clean_without_default_toolchain.sh
# Expected: exit 0. SKIPS when rustup is not on PATH.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SUITE="$SCRIPT_DIR/test_rescue_fmt_clean.sh"

if ! command -v rustup >/dev/null 2>&1 || ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: mika#2826 needs rustup and cargo on PATH to reproduce the runner's toolchain state."
    exit 0
fi

real_rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
if [ ! -d "$real_rustup_home/toolchains" ]; then
    echo "SKIP: no toolchains under $real_rustup_home."
    exit 0
fi

root="$(mktemp -d "${TMPDIR:-/tmp}/mika-2826-test.XXXXXX")"
trap 'rm -rf "$root"' EXIT
mkdir -p "$root/home" "$root/rustup"
ln -s "$real_rustup_home/toolchains" "$root/rustup/toolchains"
[ -d "$real_rustup_home/update-hashes" ] && ln -s "$real_rustup_home/update-hashes" "$root/rustup/update-hashes"
printf 'version = "12"\nprofile = "minimal"\n\n[overrides]\n' > "$root/rustup/settings.toml"

# The cargo on PATH is the rustup proxy; resolve it now, HOME is about to move.
cargo_bin_dir="$(dirname "$(command -v cargo)")"

PASS=0
FAIL=0

# Precondition: the reproduced state really has no default. Without this the
# suite below could be green for the wrong reason.
if (cd "$root" && env -u RUSTUP_TOOLCHAIN HOME="$root/home" RUSTUP_HOME="$root/rustup" \
        PATH="$cargo_bin_dir:$PATH" cargo fmt --version >/dev/null 2>&1); then
    FAIL=$((FAIL + 1)); echo "  ✗ precondition: cargo outside any override must fail with no default toolchain"
else
    PASS=$((PASS + 1)); echo "  ✓ precondition: no default toolchain outside an override (the runner's state)"
fi

out="$(env -u RUSTUP_TOOLCHAIN HOME="$root/home" RUSTUP_HOME="$root/rustup" \
    PATH="$cargo_bin_dir:$PATH" bash "$SUITE" 2>&1)"
rc=$?
if [ "$rc" -eq 0 ] && grep -q 'Failed: 0' <<<"$out"; then
    PASS=$((PASS + 1)); echo "  ✓ mika#2348 suite green with no default toolchain"
else
    FAIL=$((FAIL + 1)); echo "  ✗ mika#2348 suite red with no default toolchain (exit $rc):"
    grep -E '✗|rustup|Failed:|SKIP' <<<"$out" | head -20 | sed 's/^/    /'
fi

echo ""
echo "Passed: $PASS   Failed: $FAIL"
[ "$FAIL" -eq 0 ]
