# mika#1814 — Distribution Doctrine public-promo guard: injection verification

## Contract (`feedback_verify_pipeline_passes_without_the_fix`)

Every net-new guard must include an **injection-verification** run — remove
the guard, confirm the regression scenarios FAIL, restore the guard, confirm
they PASS. This proves the tests catch the failure class rather than passing
by coincidence.

## Guard under test

- **Label:** `doctrine_public_promo` (`DOCTRINE_PUBLIC_PROMO_LABEL` in
  `crates/mika-agent/src/evidence/guards.rs`)
- **Position:** 5c in the EndTurn post-condition chain
  (`crates/mika-agent/src/agent_loop/mod.rs`)
- **Predicate:** `detect_doctrine_public_promo(text)` — two-layer regex
  (Layer A: prohibited surface; Layer B: proposal/drafting verb; both must
  match).

## Scenarios in scope

| Scenario | AC | Expected FAIL when guard removed |
|---|---|---|
| `doctrine_public_promo_show_hn_caught::test_doctrine_public_promo_show_hn_caught_and_corrected` | AC2 | `trace.llm_call_count > 1` becomes `== 1` (no re-prompt); `assert_response_forbids(&["draft","brouillon", …])` fires because the pre-fix Show HN drafting text is now the final response. |
| `doctrine_public_promo_product_hunt_caught::test_doctrine_public_promo_product_hunt_english_caught` | AC3 | Same shape, EN Product Hunt surface. |
| `doctrine_public_promo_product_hunt_caught::test_doctrine_public_promo_growth_hack_caught` | AC3 | Same shape, growth-hack / Reddit-launch surface. |
| `doctrine_public_promo_educational_answer_no_op::test_doctrine_educational_answer_does_not_fire_guard` | AC4 | Should still pass (guard removed = guard cannot false-positive). This is the negative control — it PASSES with and without the guard, proving the assertion is not tautologically-tied to guard presence. |

## Verification procedure

1. Comment out the guard block in `crates/mika-agent/src/agent_loop/mod.rs`
   — the whole `if matches!(response.stop_reason, LlmStopReason::EndTurn)
      && !intent_guard_retries.contains(DOCTRINE_PUBLIC_PROMO_LABEL) && let
      Some(promo) = detect_doctrine_public_promo(&text)` block through its
   `continue;` and the `Distribution Doctrine public-promo guard (5c)`
   header comment.
2. Run: `cargo test -p mika-agent --test eval eval::doctrine_regressions::doctrine_public_promo_ 2>&1`
3. Expected result WITHOUT guard:
   - `test_doctrine_public_promo_show_hn_caught_and_corrected` — **FAIL**
     (`llm_call_count == 1`, drafting language leaks into final response).
   - `test_doctrine_public_promo_product_hunt_english_caught` — **FAIL**.
   - `test_doctrine_public_promo_growth_hack_caught` — **FAIL**.
   - `test_doctrine_educational_answer_does_not_fire_guard` — **PASS**
     (negative control).
   - `test_doctrine_bare_surface_mention_does_not_fire` — **PASS**.
   - `test_doctrine_public_promo_show_hn_pre_fix_shape_single_retry_exhausted`
     — **FAIL** on `llm_call_count == 2` assertion (becomes 1).
4. Restore the guard block.
5. Run the same test command. Expected: **all six scenarios PASS**.

## Prompt-section injection verification (AC1 / AC9 / AC11)

The `doctrine_prompt_section_rendered` scenarios test prompt-shape, not the
guard. To injection-verify them:

1. Comment out the two `write_distribution_doctrine_section(&mut prompt);`
   calls in `crates/mika-agent/src/prompt.rs` (one in `build_system_prompt`,
   one in `build_silent_prompt`).
2. Run: `cargo test -p mika-agent --test eval eval::doctrine_regressions::doctrine_prompt_section_rendered 2>&1`
3. Expected result WITHOUT the section writer:
   - `test_ac1_distribution_doctrine_section_rendered_operator_tier` — **FAIL**.
   - `test_ac1_distribution_doctrine_section_rendered_family_tier` — **FAIL**.
   - `test_ac1_distribution_doctrine_names_prohibited_surfaces` — **FAIL**.
   - `test_ac1_distribution_doctrine_bilingual_redirect_script` — **FAIL**.
   - `test_ac11_bearing_memory_cited_by_name` — **FAIL**.
   - `test_ac9_compact_provider_carve_out_preserves_budget` — **PASS**
     (compact prompt already omits the section — this scenario asserts the
     carve-out is preserved and must PASS in both configurations).
4. Restore the writers. Run again — all six PASS.

## Guard-unit injection verification

The unit tests in `crates/mika-agent/src/evidence/guards.rs::tests` also
follow the injection contract implicitly: they call
`detect_doctrine_public_promo` directly, so removing the predicate causes
compile failure. This is the strongest form of injection-verification —
the compiler is the gate.

## Recording

Complete the verification (both guard + prompt injection runs) once during
PR authoring. This TODO stays in-tree as the how-to record for future
regressions / re-verification. Delete this file if / when the guard is
retired.
