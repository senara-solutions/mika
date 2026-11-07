//! grounding-rule enforcement, fabrication-guard predicates, tool-call audit trail.
//!
//! Per Foundation §6: this module owns the predicates that the agent loop's
//! guard-dispatch logic consults (assert_grounded, asserted_unavailability,
//! fabrication detection), plus the audit_events ledger that persists
//! tool-call provenance for the rewind path and dashboard surfaces.
//!
//! Guard *enforcement timing* (the reject-and-reprompt machinery at EndTurn)
//! lives in `crate::agent` (post-#1452 agent_loop/). This module exposes the
//! pure predicates that drive that machinery.

pub mod audit;
pub mod guards;

/// Le recensement d'AC4 de mika#3056 : quels outils servis atteignent la forge,
/// et lequel peut faire passer un ticket à `closed`.
///
/// Sous `#[cfg(test)]`, parce que ce module n'est pas un chemin d'exécution mais
/// une **assertion sur la surface servie** — rien en production ne le consulte,
/// et son contenu n'a de sens que comme attendu d'une garde. C'est l'idiome déjà
/// en place pour `gh_process::tests::GH_SPAWN_ALLOWLIST`, le recensement frère
/// (mika#2713/mika#2715) ; ici il vit dans son propre fichier parce que la
/// population qu'il recense est transversale et n'a pas de module d'accueil.
#[cfg(test)]
pub mod forge_close_census;

pub use audit::AuditEvent;
pub use guards::{
    ASSERT_GROUNDED_LABEL, AffirmativeStateClaim, DeliveryRecord, EQUIVALENCE_CLAIM_LABEL,
    EquivalenceClaim, GROUNDING_TOOLS, UNACKNOWLEDGED_SEND_FAILURE_LABEL, UndeliveredSends,
    UndeliveredStage, undelivered_send_correction, undelivered_sends,
};
