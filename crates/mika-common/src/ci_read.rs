//! Lectures de CI — le token qui les fait, et ce qu'il en dit (mika#2963).
//!
//! # Pourquoi une classe d'opération de plus
//!
//! `gh pr checks <n> --json name,state,bucket,link` résout ce `--json` par le
//! champ GraphQL `statusCheckRollup`, qu'un PAT fine-grained ne peut pas lire :
//! GitHub n'accorde pas la permission Checks à ce type de jeton. Mesuré dans
//! `/var/log/mika/server.log` du 2026-10-07 au 2026-10-29 : 65 échecs sur
//! mika-cloud, 11 sur mika-platform, 1 sur control-monitor, tous sous
//! `GraphQL: Resource not accessible by personal access token
//! (node.statusCheckRollup…)`. Même après l'élargissement du PAT par Vincent le
//! 2026-10-29 à 14:45Z, `check-runs` rend 403. Le garde mika#2455 s'abstient
//! alors : 55 `qa_ci_coherence_abstained` avec `reason "gh_failed"` du
//! 2026-10-21 au 2026-10-29, et un `pass` QA passe sans contrôle CI.
//!
//! Le jeton d'installation de l'App, lui, porte `checks=read` (installation
//! 123032070, dépôts sélectionnés dont control-monitor, mika-cloud et
//! mika-platform, vérifiés par Vincent le 2026-10-29 à 15:16Z).
//!
//! # Lire la CI est non identitaire
//!
//! ADR-008 impose l'identité PAT machine **là où GitHub lit l'auteur** de
//! l'action : approuver ou merger une PR. Aucune règle de la forge ne lit
//! l'auteur d'une **lecture** de checks — personne ne sait, ni ne demande, qui
//! a regardé. C'est la même famille que les écritures de label de mika#2228
//! ([`crate::label_write`]), et la priorité s'y inverse pour la même raison.
//!
//! # Ce que ce module fait, et ce qu'il ne fait pas
//!
//! Il porte le jeton et sa provenance, et il **dit** laquelle a servi
//! ([`CiReadToken::report_read`], AC4). Il ne décide rien : la classification
//! des checks reste `classify_checks`, et le gate de merge reste fermé en cas
//! de doute — un échec d'échange de jeton se lit comme un échec de lecture,
//! jamais comme du vert.
//!
//! Contrairement à [`crate::label_write`], le repli au PAT **est** tenté quand
//! l'échange App échoue, et c'est correct ici : sur un déploiement sans App le
//! PAT est la seule identité disponible, et une lecture au PAT dégrade vers
//! exactement le régime d'avant ce ticket — échec sur `statusCheckRollup`, gate
//! fermé sur `GhCliFailure`. Un refus honnête, pas une lecture partielle.

use std::fmt;

/// D'où vient un token de lecture de CI (mika#2963).
///
/// Nécessaire et pas seulement descriptif : c'est le terme d'AC4. Sans lui, un
/// compte de zéro `qa_ci_coherence_abstained` ne distingue pas « l'App lit, la
/// réparation a pris » de « le repli PAT lit, l'échange App échoue en silence et
/// le défaut dort » — exactement le défaut d'instrument que mika#2205 nomme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiReadTokenSource {
    /// Token d'installation GitHub App — le chemin nominal depuis mika#2963.
    App,
    /// PAT résolu, parce qu'aucune App n'est configurée ou que l'échange a
    /// échoué. Comportement d'avant mika#2963, conservé comme repli.
    Pat,
}

impl CiReadTokenSource {
    /// Étiquette stable pour les champs de log et les lignes d'audit.
    ///
    /// **Format de fil** : la sonde AC5 du plan groupe les lectures CI sur ces
    /// deux mots. Une seconde orthographe scinderait une population sans le
    /// dire. Épinglé par `source_labels_are_stable`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Pat => "pat",
        }
    }
}

/// Un token résolu pour une lecture de CI, avec sa provenance.
///
/// `Debug` est écrit à la main et **rédige le token** : la structure circule
/// jusque dans `ToolContext`, où un `?ctx` involontaire dans un `warn!`
/// suffirait à recopier un credential dans `server.log` (même discipline que
/// [`crate::label_write::LabelWriteToken`], root CLAUDE.md § Secrets).
#[derive(Clone)]
pub struct CiReadToken {
    token: String,
    source: CiReadTokenSource,
}

impl CiReadToken {
    /// Construit un token de provenance connue.
    pub fn new(token: impl Into<String>, source: CiReadTokenSource) -> Self {
        Self {
            token: token.into(),
            source,
        }
    }

    /// Valeur à passer à `GH_TOKEN`.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Provenance résolue.
    pub fn source(&self) -> CiReadTokenSource {
        self.source
    }

    /// Dit quel jeton vient de faire une lecture CI (mika#2963, AC4).
    ///
    /// Émis **à chaque lecture**, depuis le site unique
    /// `pr_merge_with_gate::run_gh_checks_raw` — donc les cinq consommateurs
    /// l'héritent sans une ligne chez eux.
    ///
    /// # Pourquoi `info!` et non `debug!`
    ///
    /// C'est un choix contre la doctrine mika#2131 (« rien n'est émis sur le
    /// chemin nominal »), assumé parce qu'AC4 l'exige nommément et cite
    /// mika#2205 : le silence d'un repli doit se distinguer du silence d'un
    /// lecteur sain. Un `debug!` ne serait pas comptable par la sonde AC5 — et
    /// un compte de zéro abstentions accompagné d'un compte nul de lectures ne
    /// prouve rien, c'est un lecteur qui ne tourne pas.
    ///
    /// Le jeton n'apparaît jamais : cette méthode n'a accès qu'à
    /// [`Self::source`], et le `Debug` rédigé ferme la seconde porte.
    pub fn report_read(&self, repo: &str, pr: u64) {
        tracing::info!(
            target: "mika::github_auth",
            event = "ci_read_token_source",
            source = self.source.as_str(),
            repo,
            pr,
            "lecture CI faite avec ce jeton (mika#2963)"
        );
    }
}

impl fmt::Debug for CiReadToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CiReadToken")
            .field("token", &"[REDACTED]")
            .field("source", &self.source)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_the_token() {
        let t = CiReadToken::new("ghs_supersecret", CiReadTokenSource::App);
        let rendered = format!("{t:?}");
        assert!(
            !rendered.contains("ghs_supersecret"),
            "CiReadToken Debug must redact the token, got: {rendered}"
        );
        assert!(rendered.contains("REDACTED"));
        assert!(rendered.contains("App"));
    }

    #[test]
    fn source_labels_are_stable() {
        // Format de fil : la sonde AC5 groupe sur ces deux mots.
        assert_eq!(CiReadTokenSource::App.as_str(), "app");
        assert_eq!(CiReadTokenSource::Pat.as_str(), "pat");
    }

    #[test]
    fn the_token_is_readable_and_carries_its_source() {
        let app = CiReadToken::new("ghs_install", CiReadTokenSource::App);
        assert_eq!(app.token(), "ghs_install");
        assert_eq!(app.source(), CiReadTokenSource::App);

        let pat = CiReadToken::new("github_pat_x".to_string(), CiReadTokenSource::Pat);
        assert_eq!(pat.token(), "github_pat_x");
        assert_eq!(pat.source(), CiReadTokenSource::Pat);
    }
}
