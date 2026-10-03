//! Le lecteur unique de la grammaire de lien d'un check GitHub Actions
//! (mika#2617 U3, plan R14).
//!
//! Un check GitHub Actions porte un `link` de la forme
//! `https://github.com/{owner}/{repo}/actions/runs/{run_id}/job/{job_id}`.
//! Deux consommateurs en dérivent des choses différentes :
//!
//! | consommateur | ce qu'il en tire | pour quoi faire |
//! |---|---|---|
//! | [`crate::server::ci_failure_handler`] | `(run_id, job_id)` en `&str` | `gh run view <run> --job <job> --log-failed` |
//! | [`crate::merge_gate_rerun`] | `run_id` en `u64` | `gh run rerun <run> --failed` |
//!
//! ## Pourquoi ce module existe, et pourquoi il n'est pas un jumeau
//!
//! R2 et la première rédaction d'U3 proposaient d'écrire
//! `extract_actions_run_id` à côté de [`parse_check_link`] : un second lecteur
//! de la **même** grammaire, dans le même crate, sur une maison qui tient le
//! lecteur unique (mika#2158, mika#2484, mika#2624). Le plan de mika#2617
//! invoque lui-même ce principe à sa rectification R11 pour refuser un second
//! lecteur du head SHA — être incohérent d'une rectification à l'autre serait
//! pire que les deux choix.
//!
//! Le remède est donc la **promotion** : `parse_check_link` quitte
//! `ci_failure_handler`, garde ses tests et sa sémantique au caractère près, et
//! U3 en **dérive** son `run_id`. La seule chose ajoutée est la conversion
//! `&str → u64` et le refus d'un `run_id` non numérique.
//!
//! ## Le comportement hérité, nommé plutôt que redécouvert
//!
//! [`parse_check_link`] **exige le segment `/job/`** et rend `None` sans lui —
//! ce que son propre test `parse_check_link_no_job_segment` épingle depuis
//! #594. Un lien de *run* sans job (forme qu'un check non-Actions ou une API
//! future pourrait produire) rend donc `None`, et U3 le lit comme « aucune
//! relance ». C'est le bon sens de défaut, et il est **hérité plutôt que
//! décidé** : à dire, pour qu'un futur lecteur ne le prenne pas pour un oubli.

/// Parse un lien de check GitHub Actions en `(run_id, job_id)`.
///
/// Forme attendue :
/// `https://github.com/{owner}/{repo}/actions/runs/{run_id}/job/{job_id}`.
///
/// **Sémantique inchangée depuis #594**, déplacée ici verbatim par mika#2617
/// U3 (plan R14) : le segment `/job/` est obligatoire, et les deux moitiés
/// doivent être non vides. Aucun des deux identifiants n'est validé
/// numériquement — c'est [`run_id_from_link`] qui ajoute cette exigence, parce
/// que `gh run view --job` accepte un nom de job là où `gh run rerun` veut un
/// identifiant de run.
pub(crate) fn parse_check_link(url: &str) -> Option<(&str, &str)> {
    // Split on "/job/" to separate run path from job_id
    let (before_job, job_id) = url.rsplit_once("/job/")?;
    // Extract run_id: last segment before "/job/"
    let run_id = before_job.rsplit('/').next()?;
    if run_id.is_empty() || job_id.is_empty() {
        return None;
    }
    Some((run_id, job_id))
}

/// Le `run_id` numérique d'un lien de check, ou `None`.
///
/// Rend `None` sur quatre populations distinctes, qui appellent toutes la même
/// conduite (aucune relance) et que l'appelant lit sous un seul nom
/// (`RerunOutcome::NoActionsRun`) :
///
/// 1. **lien absent** — `gh` n'a pas rendu de `link` pour ce check ;
/// 2. **lien non-Actions** — un check externe (Netlify, un service tiers) dont
///    l'URL ne porte pas la grammaire `actions/runs/…/job/…` ;
/// 3. **lien de run sans `/job/`** — le comportement hérité ci-dessus ;
/// 4. **`run_id` non numérique** — `gh run rerun` prend un identifiant de run,
///    et lui passer autre chose serait un appel qu'on sait voué à l'échec.
///
/// Les quatre sont **des refus, jamais des erreurs** : il n'y a rien à réparer
/// dans un check qui n'est pas un run Actions.
pub(crate) fn run_id_from_link(link: Option<&str>) -> Option<u64> {
    let (run_id, _job_id) = parse_check_link(link?)?;
    run_id.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─────────────────────────────────────────────────────────────────────
    // Les trois tests de #594, déplacés verbatim avec la fonction.
    //
    // Ils ne sont pas réécrits : ce sont eux qui attestent que la promotion
    // n'a rien changé à la sémantique. Les renommer ou les resserrer aurait
    // transformé un déplacement en réécriture, et plus rien n'aurait dit que
    // `ci_failure_handler` lit toujours ce qu'il lisait.
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn parse_check_link_valid() {
        let url = "https://github.com/org/repo/actions/runs/12345/job/67890";
        let (run_id, job_id) = parse_check_link(url).unwrap();
        assert_eq!(run_id, "12345");
        assert_eq!(job_id, "67890");
    }

    #[test]
    fn parse_check_link_no_job_segment() {
        let url = "https://github.com/org/repo/actions/runs/12345";
        assert!(parse_check_link(url).is_none());
    }

    #[test]
    fn parse_check_link_empty_returns_none() {
        assert!(parse_check_link("").is_none());
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2617 U3 — la dérivation du `run_id` (plan § 4)
    // ─────────────────────────────────────────────────────────────────────

    /// Le cas nominal : le lien d'un check Actions rend son run.
    #[test]
    fn mika2617_run_id_from_link_nominal() {
        assert_eq!(
            run_id_from_link(Some(
                "https://github.com/senara-solutions/mika/actions/runs/18200000001/job/51900000001"
            )),
            Some(18_200_000_001)
        );
    }

    /// Lien absent — `gh` n'a rendu aucun `link` pour ce check.
    #[test]
    fn mika2617_run_id_from_link_absent() {
        assert_eq!(run_id_from_link(None), None);
        assert_eq!(run_id_from_link(Some("")), None);
    }

    /// Check non-Actions : l'URL ne porte pas la grammaire.
    #[test]
    fn mika2617_run_id_from_link_not_an_actions_run() {
        assert_eq!(
            run_id_from_link(Some("https://app.netlify.com/sites/x/deploys/abc123")),
            None
        );
        assert_eq!(
            run_id_from_link(Some("https://github.com/org/repo/pull/2614/checks")),
            None
        );
    }

    /// `run_id` non numérique : `gh run rerun` prend un identifiant de run, pas
    /// un nom. Le refuser ici évite un appel qu'on sait voué à l'échec.
    #[test]
    fn mika2617_run_id_from_link_non_numeric_is_refused() {
        assert_eq!(
            run_id_from_link(Some("https://github.com/o/r/actions/runs/latest/job/1")),
            None
        );
        assert_eq!(
            run_id_from_link(Some("https://github.com/o/r/actions/runs/12_345/job/1")),
            None
        );
    }

    /// **Comportement hérité, épinglé comme tel** (plan R14) : un lien de run
    /// sans segment `/job/` rend `None`, donc aucune relance.
    ///
    /// Sans ce test, le jour où quelqu'un trouve ce comportement surprenant il
    /// le lirait comme un oubli de la promotion plutôt que comme la sémantique
    /// de #594 conservée à dessein.
    #[test]
    fn mika2617_run_id_from_link_run_without_job_segment_is_inherited_none() {
        assert_eq!(
            run_id_from_link(Some("https://github.com/o/r/actions/runs/12345")),
            None,
            "hérité de #594 : `parse_check_link` exige `/job/`. Le défaut est le bon \
             sens (aucune relance), et il est épinglé pour ne pas passer pour un oubli."
        );
    }
}
