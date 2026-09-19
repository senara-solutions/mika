//! mika#2304 — the turn says which model served it, and a caller's model wins.
//!
//! # What this covers that its neighbours do not
//!
//! The unit tests in `server::a2a::tests` assert the wire key, the fail-soft
//! read and the fail-closed refusal; the ones in
//! `mika_common::llm::model_override` assert resolution. None of them runs a
//! turn, and the two properties this file pins only exist on a turn:
//!
//! * **T5** — every run that produces an `AgentOutput` carries an attestation.
//!   Without that, absence would be ambiguous between "this server predates the
//!   fix" and "no override was asked for", and the CLI's whole basis for
//!   refusing to print a local value (D3) collapses.
//! * **T9 / AC8** — a caller that named a model is not silently displaced by a
//!   matched skill's `[llm]` section. That is the founding false measurement of
//!   the ticket, one hop downstream.
//!
//! # Why the deadline path is asserted separately
//!
//! `AgentOutput` has three construction sites and only one of them —
//! `persist_deadline_fallback` — sits outside `run_agent_inner`'s own scope and
//! takes the value as a parameter. It is therefore the one that can be forgotten
//! with no other test going red (FD7). A turn cut off by its envelope *did* run
//! under a model, and it is precisely the population an operator investigates.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, LlmOverride, SkillInfo, SkillManifest, Triggers};
use mika_common::llm::mock::*;
use tokio::time::Instant;

use super::harness::EvalHarness;

// -- tracing capture (same shape as `test_context_scope_observability_2305.rs`) --

type Captured = Arc<Mutex<Vec<(String, HashMap<String, String>)>>>;

struct CapturingLayer {
    events: Captured,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = HashMap::new();
        let mut visitor = FieldVisitor(&mut fields);
        event.record(&mut visitor);
        let message = fields.remove("message").unwrap_or_default();
        if let Ok(mut events) = self.events.lock() {
            events.push((message, fields));
        }
    }
}

struct FieldVisitor<'a>(&'a mut HashMap<String, String>);

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

fn capture() -> (tracing::subscriber::DefaultGuard, Captured) {
    use tracing_subscriber::layer::SubscriberExt;
    let events: Captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturingLayer {
        events: Arc::clone(&events),
    });
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, events)
}

fn said(events: &Captured, needle: &str) -> bool {
    events
        .lock()
        .unwrap()
        .iter()
        .any(|(message, _)| message.contains(needle))
}

/// The INFO `resolve_skill_llm_override` writes when it stands down for a
/// caller-named model (mika#2304 D7).
const STOOD_DOWN: &str = "per-skill [llm] override stands down";
/// The INFO it writes when a skill's section wins — #463's nominal path.
const SKILL_WON: &str = "using per-skill LLM override";

/// A keyword-matched skill carrying an `[llm]` section.
///
/// `MatchReason::Keyword` is what makes it *qualify* for #463's override, which
/// is the state the precedence has to arbitrate. The provider it names is Ollama
/// pointed at a closed port: the point is never to reach it, only to be able to
/// tell from the outside which of the two providers the loop chose.
fn skill_with_llm_override(keyword: &str) -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: "override-carrier".to_string(),
                description: "mika#2304 fixture".to_string(),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers {
                keywords: vec![keyword.to_string()],
            },
            llm: LlmOverride {
                provider: Some("ollama".to_string()),
                model: Some("skill-declared-model".to_string()),
                from_db_override: false,
            },
            constraints: Constraints {
                required_tools: vec![],
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from("/skills/override-carrier"),
        keywords_lower: vec![keyword.to_string()],
        prompt_snippet: "You are running the override-carrier skill.".to_string(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

/// **T5 / AC6** — a turn with no override at all still attests.
///
/// This is the half that makes absence readable: with it, "no attestation"
/// means "this server did not say", full stop. Without it the client would have
/// to guess between a silent server and a turn nobody overrode, and guessing is
/// what D3 removes.
#[tokio::test]
async fn mika2304_a_plain_turn_attests_the_model_that_served_it() {
    let harness = EvalHarness::builder()
        .provider_name("openrouter")
        .model_name("moonshotai/kimi-k2.5")
        .responses(vec![text_response("Noted.")])
        .build()
        .await
        .unwrap();

    let trace = harness.run("ping").await.unwrap();

    assert_eq!(
        trace.output.effective_model.as_deref(),
        Some("openrouter/moonshotai/kimi-k2.5"),
        "every turn that produces an AgentOutput must name the model that served \
         it — absence is the client's only signal for 'this server did not say'"
    );
}

/// **T5, the site that can be forgotten alone (FD7)** — a turn cut off by its
/// envelope attests too.
///
/// `persist_deadline_fallback` is the third `AgentOutput` constructor and the
/// only one outside `run_agent_inner`'s scope, so it takes the value as a
/// parameter. If it were left at `None`, every other assertion in this file
/// would stay green while the population an operator most often investigates —
/// "what was it running when it ran out of time?" — went unanswerable.
#[tokio::test]
async fn mika2304_a_turn_cut_off_by_its_envelope_still_attests() {
    let harness = EvalHarness::builder()
        .provider_name("openrouter")
        .model_name("moonshotai/kimi-k2.5")
        .responses(vec![delayed_response(
            1_500,
            tool_call_response("get_current_time", serde_json::json!({})),
        )])
        .build()
        .await
        .unwrap();

    // One second of envelope against a 1.5 s call: the deadline is crossed inside
    // the call, and the loop leaves through `DeadlineExceeded` at the top of the
    // next iteration.
    let trace = harness
        .run_with_deadline("ping", Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();

    assert!(
        trace.output.deadline_exceeded.is_some(),
        "the fixture must actually reach the deadline path, or it attests nothing \
         about it"
    );
    assert_eq!(
        trace.output.effective_model.as_deref(),
        Some("openrouter/moonshotai/kimi-k2.5"),
        "a turn cut off by its envelope DID run under a model (FD7)"
    );
}

/// **T9 / AC8** — a caller-named model wins over a matched skill's `[llm]`.
///
/// The observation is the decision itself, not a downstream side effect: the
/// resolver says out loud which way it went, and the turn then runs under the
/// harness's provider — which only happens if the skill's section really stood
/// down.
#[tokio::test]
async fn mika2304_a_caller_named_model_wins_over_a_skill_llm_section() {
    let harness = EvalHarness::builder()
        .provider_name("openrouter")
        .model_name("moonshotai/kimi-k2.5")
        .caller_model_override(true)
        .skills(SkillRegistry::from_test_entries(vec![
            skill_with_llm_override("hijack"),
        ]))
        .responses(vec![text_response("Noted.")])
        .build()
        .await
        .unwrap();

    let (_guard, events) = capture();
    let trace = harness.run("please hijack this turn").await.unwrap();

    assert!(
        said(&events, STOOD_DOWN),
        "the resolver must say it stood down for the caller's model (mika#2304 D7)"
    );
    assert!(
        !said(&events, SKILL_WON),
        "a skill's [llm] section must not be applied over a caller-named model"
    );
    assert_eq!(
        trace.output.effective_model.as_deref(),
        Some("openrouter/moonshotai/kimi-k2.5"),
        "and the turn really ran under the caller's model, not the skill's"
    );
}

/// **T9's negative control**, and the half that makes the test above mean
/// something.
///
/// Same fixture, same keyword, only the caller's declaration removed: #463's
/// precedence applies and the skill's section wins. Without this, the test above
/// would pass on a registry whose skill never matched, on a resolver that always
/// stands down, and on a `[llm]` section the loop had stopped reading — three
/// ways for a green suite to describe a broken system.
///
/// The turn itself is expected to fail: the skill points at a closed port, which
/// is exactly how we can tell from the outside that the loop switched providers.
#[tokio::test]
async fn mika2304_without_a_caller_model_the_skill_llm_section_still_wins() {
    let mut harness = EvalHarness::builder()
        .provider_name("openrouter")
        .model_name("moonshotai/kimi-k2.5")
        .skills(SkillRegistry::from_test_entries(vec![
            skill_with_llm_override("hijack"),
        ]))
        .responses(vec![text_response("Noted.")])
        .build()
        .await
        .unwrap();
    // Port 1 is privileged and unbound: the provider builds, and any call to it
    // is refused immediately rather than hanging.
    harness.settings.llm_provider = mika_common::llm::ProviderKind::Ollama;
    harness.settings.ollama_base_url = Some("http://127.0.0.1:1".to_string());

    let (_guard, events) = capture();
    let _ = harness.run("please hijack this turn").await;

    assert!(
        said(&events, SKILL_WON),
        "with no caller-named model, #463's per-skill override is unchanged — if \
         this is silent the fixture never matched and the positive test above \
         proves nothing"
    );
    assert!(
        !said(&events, STOOD_DOWN),
        "nothing stood down: no caller named a model on this turn"
    );
}
