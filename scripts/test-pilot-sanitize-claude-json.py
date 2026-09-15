#!/usr/bin/env python3
"""Unit test for mika-pilot-sanitize-claude-json (mika#2313).

Verifies the allowlist IS the security boundary: given a ~/.claude.json that
mixes feature-flag/cache keys with account/credential keys, the emitter passes
ONLY the allowlisted keys and NO credential-shaped value reaches the output.
No pilot, no network — pure function of the emitter.
"""
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
EMITTER = os.path.join(HERE, "mika-pilot-sanitize-claude-json")

# A realistic mix: allowlisted keys + every forbidden class the sandbox must
# never see (mika#2039). The GrowthBook object carries a flag NAME containing
# "oauth" — a legitimate feature flag whose KEY must survive (it governs the
# prompt cache) even though the word "oauth" appears in it.
FIXTURE = {
    "hasCompletedOnboarding": True,
    "numStartups": 42,
    "cachedGrowthBookFeatures": {"tengu_mcp_local_oauth_blocked_hosts": {"defaultValue": []},
                                 "prompt_cache_enabled": {"defaultValue": True}},
    "cachedExperimentData": {"exp1": "variant_a"},
    "modelAccessCache": ["claude-opus-5"],
    # --- forbidden: must NOT appear in the output ---
    "oauthAccount": {"accessToken": "sk-ant-oat01-SECRETSECRETSECRET", "emailAddress": "x@y.z"},
    "userID": "user_ABC123",
    "machineID": "machine-deadbeef",
    "bridgeOauthDeadExpiresAt": 1234567890,
    "someFutureSecret": {"apiKey": "sk-ant-api03-LEAKLEAKLEAK"},
}

FORBIDDEN_KEYS = {"oauthAccount", "userID", "machineID",
                  "bridgeOauthDeadExpiresAt", "someFutureSecret"}
SECRET_SHAPES = ("sk-ant-", "eyJ", "user_ABC123", "machine-deadbeef", "x@y.z")


def main() -> int:
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
        json.dump(FIXTURE, fh)
        src = fh.name
    try:
        out = subprocess.run([EMITTER, src], capture_output=True, text=True, check=True).stdout
    finally:
        os.unlink(src)

    result = json.loads(out)
    failures = []

    # 1. No forbidden KEY survives.
    leaked = FORBIDDEN_KEYS & set(result)
    if leaked:
        failures.append(f"forbidden keys leaked: {sorted(leaked)}")

    # 2. No secret-shaped VALUE anywhere in the output.
    blob = json.dumps(result)
    leaked_vals = [s for s in SECRET_SHAPES if s in blob]
    if leaked_vals:
        failures.append(f"secret-shaped values leaked: {leaked_vals}")

    # 3. The prompt-cache-governing key (with its oauth-named flag) DID survive.
    if "cachedGrowthBookFeatures" not in result:
        failures.append("cachedGrowthBookFeatures dropped — the prompt cache would stay dead")
    elif "tengu_mcp_local_oauth_blocked_hosts" not in result["cachedGrowthBookFeatures"]:
        failures.append("a legitimate oauth-NAMED flag was stripped from cachedGrowthBookFeatures")

    # 4. Every emitted key is allowlisted (nothing outside the allowlist leaks).
    #    Re-derive the allowlist from the emitter to keep this test honest.
    from importlib.machinery import SourceFileLoader
    mod = SourceFileLoader("emitter", EMITTER).load_module()
    outside = set(result) - mod.ALLOWLIST
    if outside:
        failures.append(f"emitted keys outside the allowlist: {sorted(outside)}")

    if failures:
        print("FAIL:")
        for f in failures:
            print("  -", f)
        return 1
    print(f"ok: {len(result)} allowlisted keys emitted, 0 forbidden key, 0 secret-shaped value")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
