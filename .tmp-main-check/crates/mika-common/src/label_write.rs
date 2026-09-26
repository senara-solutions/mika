//! Écritures de label GitHub — le token et ce qu'il faut dire quand il échoue
//! (mika#2228).
//!
//! # Pourquoi une identité distincte du PAT
//!
//! ADR-008 impose l'identité PAT machine **là où GitHub lit l'auteur** de
//! l'action : approuver ou merger une PR, où `mika-qa` approuvant une PR
//! `mika-dev` sous l'identité App partagée est refusé par GitHub lui-même.
//! Poser un label n'est pas de cette forme — la timeline d'une issue enregistre
//! qui a posé `ready`, mais aucune règle de la forge ne lit cet auteur. mika#2205
//! avait déjà tranché ce point pour les mêmes deux scans, en repli.
//!
//! Ce module inverse la priorité **pour la seule classe des écritures de label**,
//! parce que le repli n'a jamais été atteint : le PAT résolu (samidarko, fuité
//! dans l'environnement du spirit, cf mika#2218) **authentifie** — les scans
//! tournent, `resolve_github_token` rend un token, rien n'a l'air cassé — mais
//! n'a pas `issues: write`. Mesuré le 2026-09-07 : 29 `--add-label` refusés sur
//! le marqueur `operator-gated` d'`auto_pull` et 5 sur la promotion `ready` de
//! l'auto-feeder, tous sous le même message,
//! `Resource not accessible by personal access token (addLabelsToLabelable)`.
//! Le token d'installation de l'App, lui, porte la permission.
//!
//! # Ce que ce module refuse de faire
//!
//! Il ne retombe **pas** sur le PAT quand le token App échoue. Le repli existe
//! pour l'App *absente*, pas pour l'App *insuffisante* : un PAT qui vient d'être
//! mesuré sans le scope échouerait pareil, et la seconde erreur écraserait la
//! première dans les logs en laissant croire à un problème de réseau. Trois
//! états, trois lectures distinctes :
//!
//! | état | source résolue | événement |
//! |---|---|---|
//! | App absente | [`LabelWriteTokenSource::Pat`] | aucun (comportement historique) |
//! | App présente, permission OK | [`LabelWriteTokenSource::App`] | aucun |
//! | App présente, permission absente | [`LabelWriteTokenSource::App`] | `label_write_app_token_insufficient` (ERROR) |

use std::fmt;

/// D'où vient un token d'écriture de label (mika#2228).
///
/// Nécessaire, et pas seulement descriptif : c'est ce qui permet de distinguer
/// « l'App n'est pas configurée, on écrit avec le PAT comme avant » de
/// « l'App est configurée mais son installation n'a pas `Issues: write` ».
/// Sans cette provenance les deux échecs se lisent pareil, et c'est précisément
/// la confusion que l'AC5 de mika#2228 interdit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelWriteTokenSource {
    /// Token d'installation GitHub App — le chemin nominal depuis mika#2228.
    App,
    /// PAT résolu, parce qu'aucune App n'est configurée (ou que l'échange a
    /// échoué). Comportement d'avant mika#2228, conservé comme repli.
    Pat,
}

impl LabelWriteTokenSource {
    /// Étiquette stable pour les champs de log et les lignes d'audit.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Pat => "pat",
        }
    }
}

/// Un token résolu pour une écriture de label, avec sa provenance.
///
/// `Debug` est écrit à la main et **rédige le token** : la structure circule
/// jusque dans les couches de dispatch, où un `?token` involontaire dans un
/// `warn!` suffirait à recopier un credential dans `server.log` (cf. la même
/// discipline sur `Settings`, root CLAUDE.md § Secrets).
#[derive(Clone)]
pub struct LabelWriteToken {
    token: String,
    source: LabelWriteTokenSource,
}

impl LabelWriteToken {
    /// Construit un token de provenance connue.
    pub fn new(token: String, source: LabelWriteTokenSource) -> Self {
        Self { token, source }
    }

    /// Valeur à passer à `GH_TOKEN`.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Provenance résolue.
    pub fn source(&self) -> LabelWriteTokenSource {
        self.source
    }

    /// Signale un échec d'écriture de label (mika#2228, AC5).
    ///
    /// Émet `label_write_app_token_insufficient` en ERROR **uniquement** quand
    /// le token vient de l'App et que l'erreur a la forme d'un refus de
    /// permission. Les autres échecs (réseau, `gh` absent, issue fermée) restent
    /// la responsabilité de l'appelant, qui les journalise déjà : les redoubler
    /// ici ferait de ce nom d'événement un synonyme de « une écriture de label a
    /// raté », et il doit rester le nom d'**une** cause, celle que la checklist
    /// opérateur sait réparer.
    ///
    /// `scope` nomme le chemin appelant (`auto_pull`, `wip_rescue`) ; `target`
    /// est le numéro d'issue ou de PR.
    pub fn report_write_failure(&self, scope: &str, target: u64, label: &str, error: &str) {
        if self.source != LabelWriteTokenSource::App {
            return;
        }
        if !is_permission_denied(error) {
            return;
        }
        tracing::error!(
            target: "mika::github_auth",
            event = "label_write_app_token_insufficient",
            scope,
            resource = target,
            label,
            error,
            "le token d'installation GitHub App n'a pas le droit d'écrire des labels ; \
             donner `Issues: Read and write` à l'installation de l'App (aucun repli PAT \
             n'est tenté : le PAT mesuré n'a pas ce scope non plus)"
        );
    }
}

impl fmt::Debug for LabelWriteToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LabelWriteToken")
            .field("token", &"[REDACTED]")
            .field("source", &self.source)
            .finish()
    }
}

/// Est-ce que cette sortie d'erreur `gh` est un refus de permission ?
///
/// Volontairement lexicale et volontairement large **du côté du refus** : le
/// coût d'un faux positif est une ligne ERROR de plus nommant la bonne
/// remédiation, celui d'un faux négatif est le retour au silence que mika#2228
/// existe pour finir. Les formes retenues sont celles que `gh` produit
/// réellement — la GraphQL (`Resource not accessible by …`, mesurée 34 fois le
/// 2026-09-07), la REST (`HTTP 403`), et le refus d'écriture de dépôt
/// (`Must have admin rights`, chemin `gh label create`).
///
/// La casse est ignorée ; `gh` n'est pas cohérent d'un chemin à l'autre.
pub fn is_permission_denied(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("resource not accessible")
        || lower.contains("http 403")
        || lower.contains("403 forbidden")
        || lower.contains("must have admin rights")
        || lower.contains("insufficient permission")
        || lower.contains("integration is not permitted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphql_refusal_measured_on_2026_09_07_is_recognised() {
        // Verbatim des 29 lignes `auto_pull_refusal_marker_unavailable`.
        let err = "gh issue edit --add-label failed for #1949: \
                   GraphQL: Resource not accessible by personal access token \
                   (addLabelsToLabelable)";
        assert!(is_permission_denied(err));
    }

    #[test]
    fn app_side_refusal_wording_is_recognised_too() {
        // Le même refus, vu depuis un token d'installation : c'est CETTE forme
        // que l'AC5 doit attraper, et elle ne dit pas « personal access token ».
        assert!(is_permission_denied(
            "GraphQL: Resource not accessible by integration (addLabelsToLabelable)"
        ));
        assert!(is_permission_denied("HTTP 403: Forbidden"));
        assert!(is_permission_denied(
            "gh exit 1: Must have admin rights to Repository."
        ));
    }

    #[test]
    fn ordinary_failures_are_not_permission_refusals() {
        // Ces trois-là doivent rester la responsabilité de l'appelant, sinon
        // `label_write_app_token_insufficient` cesse de nommer une cause.
        assert!(!is_permission_denied(
            "gh issue edit --add-label failed for #1949: 'operator-gated' not found"
        ));
        assert!(!is_permission_denied("gh timed out after 30s"));
        assert!(!is_permission_denied(
            "could not resolve host: api.github.com"
        ));
    }

    #[test]
    fn debug_never_prints_the_token() {
        let t = LabelWriteToken::new("ghs_supersecret".to_string(), LabelWriteTokenSource::App);
        let rendered = format!("{t:?}");
        assert!(
            !rendered.contains("ghs_supersecret"),
            "LabelWriteToken Debug must redact the token, got: {rendered}"
        );
        assert!(rendered.contains("REDACTED"));
        assert!(rendered.contains("App"));
    }

    #[test]
    fn source_labels_are_stable() {
        assert_eq!(LabelWriteTokenSource::App.as_str(), "app");
        assert_eq!(LabelWriteTokenSource::Pat.as_str(), "pat");
    }
}
