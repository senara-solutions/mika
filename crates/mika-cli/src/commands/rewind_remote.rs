//! Le transport des routes rewind de mika-spirit
//! (`POST /api/v1/rewind/{resolve,preview,execute}`, mika#1727 vol V2c-1a-1).
//!
//! **Ce que ce module est** : les miroirs du fil, le vocabulaire d'erreur typé,
//! et un `post` unique qui traduit les statuts que les trois routes partagent.
//! L'URL de base et le jeton arrivent **par argument**, pour la raison déjà
//! écrite sur [`super::tasks::fetch_task_detail`] : un client qui résout ses
//! propres accès ne se pilote pas depuis un test. Rien ici ne lit
//! `MIKA_SPIRIT_URL` ni `MIKA_INTERNAL_TOKEN`.
//!
//! **Ce qu'il n'est pas encore** : ni porte (`resolve`, `preview`, `execute`),
//! ni conversion vers `mika_agent::rewind::RewindPreview`, donc aucun appelant
//! en production. V2c-1a-2 les câble, et retire avec elles les attributs
//! `expect(dead_code)` posés ici.

use serde::Deserialize;

// Les six miroirs du fil. Les réponses de `mika-agent/src/server/rewind.rs` ne
// dérivent que `Serialize`, et `crates/mika-agent/` est hors périmètre : ces
// structs ne sont pas un doublon évitable, elles *sont* le contrat de fil côté
// client. Aucune ne porte de règle de domaine — `action` reste une `String`, sa
// conversion vers `ReversalAction` arrive avec les portes.

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug, Deserialize)]
pub(crate) struct ResolveMirror {
    pub(crate) session_id: String,
    pub(crate) after_message_id: i64,
    pub(crate) trace_ids: Vec<String>,
}

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug, Deserialize)]
pub(crate) struct PreviewMirror {
    pub(crate) messages: Vec<MessageMirror>,
    pub(crate) reversals: Vec<ReversalMirror>,
    pub(crate) tasks: Vec<TaskMirror>,
    pub(crate) warnings: Vec<String>,
    pub(crate) blocked: bool,
    pub(crate) block_reason: Option<String>,
}

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug, Deserialize)]
pub(crate) struct MessageMirror {
    pub(crate) id: i64,
    pub(crate) role: String,
    pub(crate) content_snippet: String,
}

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug, Deserialize)]
pub(crate) struct ReversalMirror {
    pub(crate) audit_event_id: i64,
    pub(crate) tool_name: String,
    pub(crate) target_key: String,
    pub(crate) action: String,
    pub(crate) description: String,
}

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug, Deserialize)]
pub(crate) struct TaskMirror {
    pub(crate) task_id: String,
    pub(crate) label: String,
    pub(crate) actioned: bool,
}

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug, Deserialize)]
pub(crate) struct ResultMirror {
    pub(crate) messages_deleted: usize,
    pub(crate) reversals_applied: usize,
    pub(crate) reversals_skipped: usize,
    pub(crate) tasks_deleted: usize,
    pub(crate) tasks_cancelled: usize,
    pub(crate) warnings: Vec<String>,
    pub(crate) reversal_descriptions: Vec<String>,
    pub(crate) rewind_trace_id: String,
}

// Tout le code de test vit ici, **à la fin du fichier** : le scanner d'egress
// (`scripts/lib/egress_manifest_lint.py`) coupe la production au premier
// `#[cfg(test)]`, donc un item de production qui reprendrait après la coupure
// sortirait de son inventaire (`audit_cut_holes`).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mika1727_les_miroirs_se_lisent_depuis_la_forme_du_serveur() {
        let apercu: PreviewMirror = serde_json::from_str(
            r#"{"messages":[{"id":41,"role":"user","content_snippet":"pose la question"}],
                "reversals":[{"audit_event_id":7,"tool_name":"store_fact","target_key":"people:vincent",
                              "action":"Restore","description":"restaure la fiche"}],
                "tasks":[{"task_id":"t-1","label":"relire le plan","actioned":true}],
                "warnings":["un avertissement"],"blocked":true,"block_reason":"verrou"}"#,
        )
        .expect("la forme de RewindPreviewResponse");
        assert_eq!(apercu.messages.len(), 1);
        assert_eq!(apercu.messages[0].id, 41);
        assert_eq!(apercu.messages[0].role, "user");
        assert_eq!(apercu.messages[0].content_snippet, "pose la question");
        assert_eq!(apercu.reversals.len(), 1);
        assert_eq!(apercu.reversals[0].audit_event_id, 7);
        assert_eq!(apercu.reversals[0].tool_name, "store_fact");
        assert_eq!(apercu.reversals[0].target_key, "people:vincent");
        // `action` arrive sous sa forme `Debug` côté serveur (F3) et reste une
        // `String` ici : la table des trois graphies est le § 2.4, hors de ce vol.
        assert_eq!(apercu.reversals[0].action, "Restore");
        assert_eq!(apercu.reversals[0].description, "restaure la fiche");
        assert_eq!(apercu.tasks.len(), 1);
        assert_eq!(apercu.tasks[0].task_id, "t-1");
        assert_eq!(apercu.tasks[0].label, "relire le plan");
        assert!(apercu.tasks[0].actioned);
        assert_eq!(apercu.warnings, vec!["un avertissement".to_string()]);
        assert!(apercu.blocked);
        assert_eq!(apercu.block_reason.as_deref(), Some("verrou"));

        let resolu: ResolveMirror = serde_json::from_str(
            r#"{"session_id":"s-1","after_message_id":40,"trace_ids":["tr-1","tr-2"]}"#,
        )
        .expect("la forme de RewindResolveResponse");
        assert_eq!(resolu.session_id, "s-1");
        assert_eq!(resolu.after_message_id, 40);
        assert_eq!(
            resolu.trace_ids,
            vec!["tr-1".to_string(), "tr-2".to_string()]
        );

        let resultat: ResultMirror = serde_json::from_str(
            r#"{"messages_deleted":2,"reversals_applied":3,"reversals_skipped":1,
                "tasks_deleted":4,"tasks_cancelled":5,"warnings":["w"],
                "reversal_descriptions":["d"],"rewind_trace_id":"rw-1"}"#,
        )
        .expect("la forme de RewindResultResponse");
        assert_eq!(resultat.messages_deleted, 2);
        assert_eq!(resultat.reversals_applied, 3);
        assert_eq!(resultat.reversals_skipped, 1);
        assert_eq!(resultat.tasks_deleted, 4);
        assert_eq!(resultat.tasks_cancelled, 5);
        assert_eq!(resultat.warnings, vec!["w".to_string()]);
        assert_eq!(resultat.reversal_descriptions, vec!["d".to_string()]);
        assert_eq!(resultat.rewind_trace_id, "rw-1");
    }
}
