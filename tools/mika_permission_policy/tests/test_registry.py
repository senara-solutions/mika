"""Registry-shape tests for mika_permission_policy.get_policy()."""

from __future__ import annotations

from mika_permission_policy import get_policy


class TestRegistry:
    def test_returns_dict(self):
        policy = get_policy()
        assert isinstance(policy, dict)

    def test_ac3_initial_13_present(self):
        # mika#1708 AC3 initial binary set — all must be registered.
        policy = get_policy()
        for binary in (
            "grep", "awk", "sed", "cat", "ls", "find",
            "git", "gh", "cargo", "make", "sqlite3", "bash", "sh",
        ):
            assert binary in policy, f"AC3 binary missing: {binary}"

    def test_all_entries_callable(self):
        policy = get_policy()
        for binary, fn in policy.items():
            assert callable(fn), f"non-callable entry: {binary}"

    def test_all_entries_have_argv_cwd_signature(self):
        # Smoke: every entry accepts (argv, cwd) and returns a bool.
        policy = get_policy()
        for binary, fn in policy.items():
            result = fn([binary], "/tmp")
            assert isinstance(result, bool), (
                f"{binary} returned {type(result).__name__}, expected bool"
            )

    def test_grep_family_shares_impl(self):
        policy = get_policy()
        assert policy["egrep"] is policy["grep"]
        assert policy["fgrep"] is policy["grep"]

    def test_no_op_family_shares_impl(self):
        policy = get_policy()
        # echo/printf/true/false/: — all share is_safe_no_op
        assert policy["printf"] is policy["echo"]
        assert policy["true"] is policy["echo"]
        assert policy["false"] is policy["echo"]

    def test_test_bracket_share_impl(self):
        policy = get_policy()
        assert policy["["] is policy["test"]
