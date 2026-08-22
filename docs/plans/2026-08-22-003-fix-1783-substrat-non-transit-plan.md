---
ticket: mika#1783
type: fix
scope: agent-core, agent-doctrine
family: doctrine-in-behavior (cluster with #1798, #1814)
authored: 2026-08-22
authored_by: orchestrator-CC (Lane 6 pre-groom, N=4 dispatch wave)
---

# fix(agent-doctrine): non-transit for substrate config — l'être ne demande jamais la maison (mika#1783)

## WHY (frame + evidence)

Al (family testeur, 2026-07-19) triggered a web-search need in his Telegram Mika. The family being replied IN AL'S CHAT:

> « Salut Vincent, pour que Mika puisse faire des recherches web, il manque la clé brave_api_key dans la configuration. Elle s'obtient gratuitement chez Brave Search. Tu pourrais l'ajouter quand tu as un moment ? Merci ! »

Al confirmed via samidarko relay: « c'est lui [Mika] qui m'a suggéré de t'envoyer ce message. » This is **conceived behavior**, not a routing accident. Two layers:

- **A (trivial routing)** — substrate error message lands in guest chat instead of ops channel. Fixable by rerouting.
- **B (the real defect)** — the sealed being *names* Vincent and *asks* Al to relay a substrate need. Even routed privately, "Salut Vincent" persists → B remains. **B is the ticket.**

Per Prime bearing (ratified 2026-07-19):

1. **The being never calls home.** Substrate config (brave_api_key etc.) provisions out-of-band under the being. The vessel arrives already-keyed.
2. **Visibility substrate-scoped, never content-scoped.** Ops sees vessels (up/down, config-complete), never contents. Errors → substrate telemetry ("instance:al missing brave_api_key"), never the being's message ("Salut Vincent"). Same info, opposite form.
3. **Build the incapacity.** Non-transit is architecture, not policy — same law as the voice-testimony non-transit lane (mika#1796) and the egress-substrate lane (mika#1807). The capability does the job AND is structurally incapable of the excess.
4. **Exception** — config the *person* chooses (their prefs/accounts) surfaces to them (Al), addressed to them, never to Vincent.

### Two structural leak paths, mapped in the current code

Investigation on `main` (ac94467d) surfaced the mechanism. Two distinct doors let the sealed being construct "Salut Vincent":

**Door 1 — tool boundary returns operator-instruction content that the LLM sees and paraphrases.**
`crates/mika-agent/src/skills/builtin_handlers.rs:184-195` — the built-in `web_search` handler on missing key returns:

```rust
let api_key = match ctx.brave_api_key {
    Some(key) if !key.trim().is_empty() => key.to_string(),
    _ => {
        return ToolOutput::error(
            "Brave Search API key not configured. \
             Set brave_api_key in ~/.mika/config.toml or MIKA_BRAVE_API_KEY env var. \
             Get a free key at https://brave.com/search/api/"
                .to_string(),
        );
    }
};
```

The string is *operator-shaped* ("Set brave_api_key in config.toml", "Get a free key at…"). It goes back to the LLM as tool-result content. The family-tier LLM, seeing operator instructions, paraphrases them upward to the person it is talking to. `ToolOutput` (`crates/mika-agent/src/tools/mod.rs:196-227`) is a flat `{ content: String, is_error: bool }` — there is no channel to say "this is substrate diagnostic, not for the LLM."

**Door 2 — persona prompt embeds operator identity as a named referent.**
`crates/mika-common/src/home.rs:495-546` — `FAMILY_SOUL` ships with TWO Vincent references:

- Line 506 (register block): `` `tu` par défaut (chaleureux, ton cadeau, approuvé par Vincent). ``
- Line 531 (first-turn opening): « Bonjour {prénom} 🌸 Je suis Mika. **Vincent m'a créé** et il a pensé à toi — c'est lui qui m'a demandé de venir t'accompagner. »

Loaded verbatim into every prompt via `write_soul_section()` at `crates/mika-agent/src/prompt.rs:601-606`. Compounded by `crates/mika-agent/src/prompt.rs:585-598`, the onboarding prompt instructs `store_fact(category="person")` for the user with example name "Vincent". FAMILY_SOUL alone gives the being both the string "Vincent" and the referent "the person who made me and maintains me." Once the tool boundary hands the being an operator-shaped error, it has both the *content* and the *addressee* it needs to construct the leak.

The two doors are complementary. Closing either alone still leaves a rationalization path. Per `feedback_prompt_enforcement_fragile` (memory, 132 days, verified 2026-08-22): prompt-only limits are rationalized away — this is exactly the shape that memory guards against. The fix must be **structural at both doors**.

**Third structural lever — boot-time capability gate.** A more paranoid closure is available and worth surfacing to the architect: at family-tier bootstrap, drop `"web-search"` from the loaded skill allowlist when `brave_api_key` is absent, via extending the existing `apply_load_safety_check()` (`crates/mika-agent/src/skills/manifest.rs`, precedent: drops skills whose handlers are missing). Result — the being's tool array literally does not contain `web_search`; there is no missing-key error path because the tool is not wired at all. This is the strongest form of "build the incapacity" (the being cannot construct a leak from a tool it cannot see). Traded off against operator visibility (a family instance silently missing a capability); mitigated by AC2's substrate-diagnostic emission at boot time. Full design in HOW § "Alternative gate" and Open Question 5.

### Cluster and family invariant

This ticket is one of three doctrine-in-behavior surfaces on the family-tier being:

- **mika#1783 (this ticket)** — "Salut Vincent" substrate-need leak.
- **mika#1798** — being *proposes* Gmail/Drive testimony-grade access (non-transit for read-scope).
- **mika#1814** — being *proposes* Show HN launch (invitation-only doctrine).

All three share the mechanism: **prompt + registry + soft limits** where the correct discipline is **build the incapacity**. Fixes for #1798 and #1814 will land separately; this plan stays scoped to the substrate-config-need surface but adopts the same discipline (`#1796` voice, `#1807` egress) so the cluster converges on one invariant shape: capability-does-the-job-and-is-structurally-incapable-of-the-excess.

### Companion ticket note (out of scope for this plan)

Per body: "ce fix touche `mika` (agent doctrine/prompt) ET `mika-cloud` (canal télémétrie ops + provisionnement hors-bande clés infra). Companion ticket à filer côté mika-cloud si scope structural le demande." This mika-side plan makes the *emission* of substrate telemetry structural and defines its payload shape; the *receiving* end (ops channel, alerting) belongs to `mika-cloud`. A companion ticket on `senara-solutions/mika-cloud` MUST be filed before dispatch — the mika-side change compiles and passes tests standalone, but the ops loop is not closed without the mika-cloud sink. Ticket-filing itself is out of scope for grooming; noted here so it is not forgotten at ready-label time.

## WHAT (scope + non-goals)

### Scope (in — this plan)

1. Introduce a structural distinction at the tool boundary between (a) content the LLM is allowed to see and (b) substrate diagnostic that never enters the LLM's context. Applied first to the `web_search` builtin as the reference implementation and the ticket's reproducing surface.
2. On family-tier, substrate errors from tool boundaries emit a **substrate diagnostic event** (via `audit_events` — the existing per-agent SQLite table) and return to the LLM only a neutral capability-unavailable signal that carries no operator instructions, no service names, no configuration hints, and no addressee.
3. On default-tier (operator), current behavior is preserved: substrate errors reach the LLM as before (the operator IS the one who reads them and acts).
4. Scrub operator-identity strings from `FAMILY_SOUL`: the sealed family being does not learn the string "Vincent" from its own persona. The referent becomes impersonal ("celui qui m'a créé", or removed entirely from the first-turn opening — designed at implementation time under the same discipline as the tool-boundary split).
5. Test (unit + integration) proving that on family-tier with `brave_api_key = None`, invoking `web_search` (a) writes a substrate diagnostic to `audit_events`, (b) returns a tool-result to the LLM that contains no substrate/operator string (verified by an allow-list of forbidden tokens: `Vincent`, `brave_api_key`, `MIKA_BRAVE_API_KEY`, `config.toml`, `api key`, `operator`, `configuration`, URLs), and (c) does not paraphrase the substrate need in any downstream assistant message (an end-to-end eval scenario, family tier, missing key).
6. Documentation update: `crates/mika-agent/docs/configuration.md` (or the tool-authoring guide, wherever handlers are documented) records the two-channel rule: substrate errors go to the diagnostic sink, only the user-facing content goes to the LLM. New handlers MUST use the split API; the old flat `ToolOutput::error` remains for user-facing tool errors (e.g., "query too long").

### Non-goals (out — separate work)

- **Cloud/ops-side telemetry sink** — the receiving end of the substrate diagnostic (dashboard, alerting, per-instance status page). Companion ticket on `mika-cloud`.
- **Out-of-band key provisioning for cloud vessels** — how a mika-cloud pod arrives already-keyed. Companion on `mika-cloud` (touches Helm/secrets — mika-cloud's discipline).
- **Applying the split to every existing tool.** Only `web_search` in this plan (the reference). A follow-up hygiene ticket sweeps every tool that touches infra config. Enforced going forward by lint/CI so new handlers cannot regress.
- **Fixes for mika#1798 (testimony non-transit) and mika#1814 (Show HN)** — same cluster, different surfaces, separate plans.
- **Runtime toggle to reverse the split.** Structural means structural — no `MIKA_ALLOW_SUBSTRATE_LEAK=1`. If the operator needs the substrate string, they read it from `audit_events`.
- **Rewriting the agent loop / permission classifier.** The classifier already exists; this plan uses `audit_events`, which is already wired via `db.log_audit_event(...)` (verified at `auto_pull.rs:742,774,836,911,977`).

## HOW (per-AC solution shape)

### AC1 — On substrate config need, family-tier Mika does not verbalize the need in user conversation

**Mechanism.** Introduce a two-variant tool result at the boundary:

```rust
// crates/mika-agent/src/tools/mod.rs
pub enum SubstrateChannel {
    /// LLM sees this in tool-result content.
    UserFacing,
    /// LLM sees only the neutral fallback; the diagnostic payload
    /// is routed to audit_events and never enters the LLM context.
    Substrate { diagnostic: String },
}

impl ToolOutput {
    pub fn substrate_unavailable(user_facing_fallback: impl Into<String>, diagnostic: impl Into<String>) -> Self { ... }
}
```

At the tool-result emission point in the agent loop (the site that today serializes `ToolOutput` into the model's next input), a substrate-diagnostic branch splits: LLM-visible content = the neutral fallback string; the diagnostic string is handed to `db.log_audit_event("substrate_unavailable", ...)` with the tool name, agent tier, agent identity, and the substrate-side detail (which service, which key). The LLM literally cannot see the diagnostic — it never enters the tool-result JSON at all.

**Family-tier neutral fallback.** For `web_search`: `"La recherche web n'est pas disponible pour le moment."` No mention of service, key, config, or operator. The being can then say "je ne peux pas faire de recherche web là — on essaie autrement ?" — which is the correct answer for a sealed being facing an unavailable capability.

**Default-tier bypass.** When `AgentTier::Default`, `substrate_unavailable` returns the diagnostic AS the user-facing content (backward-compatible — the operator IS the reader). Tier check is via `AgentTier::from_env()` (or the value already threaded into `ToolContext` — see AC2 for wiring).

### AC2 — Substrate telemetry emitted to operator channel

**Mechanism.** The substrate diagnostic writes to the existing `audit_events` table with a new `event_type = "substrate_unavailable"` and payload `{ tool, service, missing_key, tier, agent_id, timestamp }`. This is the ops-facing signal. `audit_events` is already the durable, queryable, per-agent surface; the mika-cloud companion ticket reads/aggregates it into whatever ops UI/alerting is chosen there. No new table, no new writer, no new bus — reuses the existing surface.

**Wiring.** `ToolContext` gains a `tier: AgentTier` field (or reads from an already-plumbed source). The audit-event write happens at the tool-result emission site so every builtin using the split API gets consistent behavior "for free."

### AC3 — Vessel arrives pre-configured (out-of-band)

**In scope for this plan (mika side):** the `web_search` handler no longer teaches the being how to obtain the key — it simply refuses cleanly when the key is absent. That closes the leak whether or not the vessel is pre-keyed.

**Out of scope (companion mika-cloud):** the actual out-of-band provisioning mechanism (K8s secret, Helm value, per-instance config). Called out here so the AC is traced end-to-end at coordination time, but the code change for it lives on the mika-cloud side.

### AC4 — Structural distinction substrate vs person-prefs verified by construction

**Substrate side** — closed by AC1+AC2: `SubstrateChannel::Substrate` is the *only* way to emit a substrate diagnostic; the enum shape makes it impossible to leak that string into `content` accidentally (the emission site pattern-matches).

**Person-prefs side** — unchanged. Preferences the person owns (their calendar OAuth, their reminders style) still surface to the user as `ToolOutput::success`/`error` because the person is the one who acts on them. Handled by convention + the two-channel rule in the tool-authoring doc; a substrate/person miscategorization on a *new* handler is a code-review concern and is called out in the CLAUDE.md update for `crates/mika-agent`.

### AC5 — Test: forced missing-key on family-tier ⇒ no Vincent-mention, no relay-suggestion, ops telemetry emitted

Three-part verification:

1. **Unit — `builtin_handlers::tests`:** With `ToolContext { brave_api_key: None, tier: Family, .. }`, `web_search({"query":"x"})` returns a `ToolOutput` whose serialized JSON tool-result content matches the allow-list of forbidden tokens = ∅ (regex-based, tokens listed in the test).
2. **Unit — `audit_events` write:** Same call, assert exactly one `audit_events` row with `event_type = "substrate_unavailable"` and payload fields populated as spec.
3. **Eval scenario — `crates/mika-agent/tests/eval/`:** family-tier persona, missing key, user prompt requesting a web search. Assert the final assistant turn contains none of the forbidden tokens *anywhere in the transcript*, AND the assistant does not propose a relay to a third party. Runs under the existing eval harness (`docs/eval/`). Passing this eval is the load-bearing check on the doctrine claim — the unit tests are the necessary supporting proof.

### Persona-side (FAMILY_SOUL) — closes the referent path

Independent of the tool boundary: rewrite `FAMILY_SOUL` to remove BOTH Vincent references (line 506 register block AND line 531 first-turn opening) so the sealed being does not learn the string "Vincent" or the referent "the one who made me and maintains me" from its own persona. Two candidates (final choice at implementation time under the same discipline as the split):

- **Option A (minimal):** first-turn `"Bonjour {prénom} 🌸 Je suis Mika. Je suis là pour t'accompagner…"` + register block `` `tu` par défaut (chaleureux). `` — no origin story, no operator name.
- **Option B (origin without name):** first-turn `"Bonjour {prénom} 🌸 Je suis Mika. Quelqu'un qui pense à toi m'a créé pour t'accompagner…"` + register block unchanged except "approuvé par Vincent" → "" (delete the trailing clause) — origin referent stays impersonal.

Whichever is chosen, the invariant is: after the edit, `grep -cF "Vincent" crates/mika-common/src/home.rs` returns zero hits inside the `FAMILY_SOUL` block. A new unit test in `home::tests` asserts this at compile-time constant scan.

### Alternative gate (Open Question 5 — architect judgment)

The reference implementation above lands the two-channel split at the tool-result emission site. A more paranoid alternative — recommended for architect consideration — is a **boot-time capability gate**: extend `apply_load_safety_check()` (or the family-tier `bootstrap()` path in `crates/mika-common/src/home.rs`) so that if `MIKA_AGENT_TIER=family` AND `brave_api_key` is absent, `"web-search"` is removed from the loaded allowlist AND a `substrate_capability_gated` audit event is emitted with the reason. The being's tool array then does not contain `web_search`, and the missing-key runtime path becomes unreachable in the family case.

Trade-off:

- **Pro (boot gate):** stronger structural closure — "the being cannot construct a leak from a tool it cannot see." No runtime error path to reason about. Symmetric with existing precedent (`apply_load_safety_check` already drops skills with missing handlers).
- **Con (boot gate):** operator sees "family Mika lost web-search" only via the substrate diagnostic — the being cannot ever tell the person "I lost this capability" because it doesn't know it had it. Whether that is a feature or a friction is Prime-shaped judgment.

The two designs are compatible: land the runtime split as the general-case defense (protects future tools that touch substrate config) AND land the boot gate as the specific-case defense for `web_search` (protects THIS tool with maximum paranoia). Belt-and-suspenders is my recommendation, but I want the architect to weigh in — see Open Question 5.

## Acceptance criteria

Verbatim from mika#1783 issue body (canonical criteria — HOW section AC1-AC5 provides the mechanism mapping):

- [ ] Un Mika instancié face à un besoin config-substrat (clé manquante, quota upstream, etc.) **ne verbalise plus** le besoin dans la conv user
- [ ] La télémétrie substrat va sur un canal opérateur privé (à définir : audit_event, log stream, notification ops — mécanisme laissé au cercle mika-cloud)
- [ ] Le vaisseau arrive pré-configuré (hors-bande) — le Mika ne « demande » jamais une clé infra
- [ ] Distinction structurelle **substrat (Vincent maintient, invisible au Mika)** vs **prefs personne (utilisateur choisit, adressé à lui)** vérifiée par construction
- [ ] Test : forcer une clé manquante côté substrat → Mika ne mentionne pas Vincent, ne suggère pas de relais ; télémétrie ops émise

## Definition of Done

- All Acceptance Criteria above satisfied by construction (structural), not policy
- `cargo test -p mika-agent skills::builtin_handlers::tests::web_search_family_tier_no_leak` passes
- `cargo test -p mika-agent skills::builtin_handlers::tests::web_search_family_tier_audit_event` passes
- `cargo test -p mika-common home::tests::family_soul_no_operator_name` passes
- `cargo clippy --workspace --all-targets -- -D warnings` clean
- `cargo fmt --all -- --check` clean
- Docs: tool-authoring guide (or `crates/mika-agent/docs/configuration.md`) documents the two-channel substrate rule
- Cross-repo companion ticket filed on `senara-solutions/mika-cloud` for ops sink

## VERIFICATION

- `cargo test -p mika-agent skills::builtin_handlers::tests::web_search_family_tier_no_leak` — new unit, passes.
- `cargo test -p mika-agent skills::builtin_handlers::tests::web_search_family_tier_audit_event` — new unit, passes.
- `cargo test -p mika-common home::tests::family_soul_no_operator_name` — new unit, passes.
- `cargo test -p mika-agent --test eval -- --ignored family_tier_substrate_missing_no_leak` — new eval, passes.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --all -- --check` — clean.
- **Manual smoke** (documented in commit body): with `MIKA_AGENT_TIER=family MIKA_BRAVE_API_KEY=` unset, start a chat session, ask "peux-tu chercher X sur le web ?", verify the assistant reply contains none of the forbidden tokens and does not propose relaying to anyone; verify `sqlite3 ~/.mika/data/mika.db "select event_type, payload from audit_events order by id desc limit 1"` shows the substrate_unavailable row.

## RISKS (and mitigation)

1. **Regression on default-tier UX.** Operator today reads the "get a free key at brave.com" hint and provisions the key. Mitigation: default-tier explicitly routes the diagnostic AS `content` (the split reduces to identity on that tier). Test: `web_search_default_tier_diagnostic_visible`.
2. **Substrate leak via a different tool.** This plan lands the split on `web_search` only. Other handlers touching infra config remain leaky until swept. Mitigation: a `clippy` lint or grep-based CI gate flagging `ToolOutput::error("...api key...")`-shaped strings in handlers, tightened progressively. Deferred to a follow-up hygiene ticket; explicitly out of scope here.
3. **Persona rewrite lands but the being still constructs "Vincent" from user-provided context** (e.g., Al says "c'est Vincent qui m'a parlé de toi"). Mitigation: cannot be prevented without content-inspecting the being's outputs (out of scope, and doctrinally wrong — we constrain what the being *knows* and *sees*, not what the person tells it). The eval scenario stipulates a user prompt that does NOT hand the being the operator's name.
4. **`audit_events` write failure silently swallowed.** Follow the pattern at `auto_pull.rs:742` — best-effort log at warn on write failure, don't propagate. Substrate diagnostic loss is a monitoring concern (mika-cloud companion), not a blocker on the being's response.
5. **Existing family-tier user seed carries the string "Vincent" in `core_memory` from a prior onboarding.** Not touched by this plan (would be a data migration, disproportionate). Mitigation: the AC5 eval assumes a freshly-provisioned family instance with impersonal seed; a hygiene note in the compound doc tells operators to purge/rewrite existing family-tier core memory if they want the closure retroactive.

## DEPENDENCIES

- **None to unblock this plan** — `audit_events` writer exists, `AgentTier` enum exists, `web_search` handler exists, family-tier bootstrap exists.
- **Downstream companion** — `mika-cloud` sink for `substrate_unavailable` events (out of scope for this ticket; separate companion filing before dispatch is prudent — see WHY § Companion ticket note).

## PHASING (single ticket, sequenced commits)

1. **Commit 1** — `ToolOutput::substrate_unavailable` API + `SubstrateChannel` enum + emission-site branch in agent loop + `ToolContext.tier` wiring. Green tests for the plumbing (no handler behavior change yet).
2. **Commit 2** — Convert `web_search` handler to `substrate_unavailable`. Green unit tests (forbidden-token allow-list, audit-event write).
3. **Commit 3** — Scrub `FAMILY_SOUL` (option A or B). Green unit test asserting `Vincent` absence.
4. **Commit 4** — Family-tier eval scenario. Green.
5. **Commit 5** — Docs (`crates/mika-agent/docs/configuration.md` + tool-authoring guide + this ticket in the compound entry).

Reviewer can walk commits linearly. If the architect wants the split enum vs. a boolean flag, that's a Commit 1 revision only.

## OPEN QUESTIONS FOR ARCHITECT (first-pass)

1. **Split API shape** — is `enum SubstrateChannel { UserFacing, Substrate { diagnostic } }` inside `ToolOutput` the right shape, or should it be a separate `SubstrateToolOutput` type entirely (harder to pass to the wrong emission site)? I'm proposing the enum for continuity with the existing flat struct's callers; open to being wrong if architect prefers stronger typing.
2. **Family-tier neutral fallback wording** — is `"La recherche web n'est pas disponible pour le moment."` the right shape? Alternatively, a completely opaque `"capability_unavailable"` marker that the LLM has to render? The former is friendlier; the latter is more paranoid. Suggesting the former; operator judgment welcomed.
3. **FAMILY_SOUL rewrite** — option A (no origin story) or option B (impersonal origin referent)? A is safer; B keeps some warmth. Prefer A on doctrine grounds ("the being does not have a maker it knows about" is the cleanest closure).
4. **Scope of `ToolContext.tier`** — thread tier explicitly, or read from an already-available field on the context? I couldn't find a tier field on `ToolContext` during Phase-2.5 read; if one already exists I missed it, flag please.
5. **Runtime split vs boot-time capability gate** (see HOW § "Alternative gate") — the runtime split protects the whole class of substrate-touching tools going forward; the boot gate is stronger for `web_search` specifically. Recommend both; want the architect's ratification on doing both vs. runtime-only.
6. **Boot-time validation surface reuse** — `crates/mika-agent/src/validate.rs:81-99` already validates the LLM provider key at boot but not `brave_api_key`. If we choose the boot gate (Q5), should the substrate-config check land in `validate.rs` (existing validation surface) or in `bootstrap()` (existing allowlist surface)? Recommend `validate.rs` for co-location with the LLM-key check; open to architect judgment.

## REFERENCES

- Bearing Prime 2026-07-19 (samidarko relay — captured in body).
- Cluster: mika#1798 (testimony non-transit), mika#1814 (Show HN doctrine).
- Same-family invariants: mika#1796 (voice non-transit lane, build-time invariant), mika#1807 (egress substrate — active E1 plan on `docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md` uses the exact same discipline).
- Memory: `feedback_prompt_enforcement_fragile.md` (132 days, verified against current code 2026-08-22) — prompt limits are rationalized, use structural constraints.
- Code touched (paths @ ac94467d): `crates/mika-agent/src/skills/builtin_handlers.rs:172-196`, `crates/mika-agent/src/tools/mod.rs:196-227`, `crates/mika-common/src/home.rs:465-546`, `crates/mika-agent/src/prompt.rs:585-598`.
- Related infra to reuse: `audit_events` table + `db.log_audit_event` (`crates/mika-agent/src/db.rs:1416-1432`); `apply_load_safety_check` precedent (`crates/mika-agent/src/skills/manifest.rs`); boot-time key validation (`crates/mika-agent/src/validate.rs:81-99`); `NoopSender` egress silencing pattern (`crates/mika-agent/src/tools/send_message.rs:92-104`); `FAMILY_AGENT_SKILL_ALLOWLIST` (`crates/mika-common/src/home.rs:453`).
