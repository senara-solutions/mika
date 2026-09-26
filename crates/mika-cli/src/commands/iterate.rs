//! `mika iterate <repo>#<N> --context <texte>` (mika#2506 AC1).
//!
//! **Spirit dispatche, la CLI rend.** Ce module résout le `<repo>#<N>` de la
//! ligne de commande, POST sur `/api/v1/agents/{id}/iterate`, et rend la
//! réponse. Il ne dérive aucune branche, ne résout aucun skill, n'ouvre pas la
//! base et ne spawne aucun processus — la structure que
//! `iterate_tests::mika2506_la_cli_ne_dispatche_rien_localement` tient par un
//! scan de source à allowlist vide.
//!
//! La raison n'est pas stylistique : une CLI qui dispatcherait elle-même devrait
//! répliquer `SkillRegistry`, la résolution du répertoire de skill et celle du
//! jeton GitHub, et contredirait mika#1727 — qui a fait de la CLI un client mince
//! pour que l'exécution ait **un seul** propriétaire.

use anyhow::{Context, Result, bail};
use std::io::{self, Write};

use crate::cli::{IterateArgs, OutputFormat};

/// Le `<repo>#<N>` de la ligne de commande, découpé.
///
/// Rendu séparément de l'appel réseau pour que la grammaire soit testable sans
/// serveur — et parce qu'une cible malformée doit être refusée **avant** tout
/// appel, comme le rang 1 côté serveur.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct IterateTarget {
    pub repo: String,
    pub issue: u64,
}

/// Découpe `<repo>#<N>`.
///
/// Accepte `mika#2503` et `senara-solutions/mika#2503` — la **normalisation** du
/// propriétaire par défaut vit côté serveur
/// (`webhook_dispatch::normalize_owner_repo`), site unique de cette règle. La
/// recopier ici en ferait un second, et c'est la classe que mika#2158 a dû
/// refermer.
pub(crate) fn parse_target(raw: &str) -> Result<IterateTarget> {
    let raw = raw.trim();
    let Some((repo, number)) = raw.rsplit_once('#') else {
        bail!(
            "cible `{raw}` illisible : attendu `<repo>#<numéro-d-issue>` \
             (ex. `mika#2503`)"
        );
    };
    let repo = repo.trim();
    if repo.is_empty() {
        bail!("cible `{raw}` illisible : le dépôt est vide");
    }
    let issue: u64 = number.trim().parse().with_context(|| {
        format!("cible `{raw}` illisible : `{number}` n'est pas un numéro d'issue")
    })?;
    if issue == 0 {
        bail!("cible `{raw}` illisible : le numéro d'issue est `0`");
    }
    Ok(IterateTarget {
        repo: repo.to_string(),
        issue,
    })
}

pub async fn run(args: IterateArgs) -> Result<()> {
    run_with_out(args, &mut io::stdout()).await
}

pub(crate) async fn run_with_out(args: IterateArgs, out: &mut impl Write) -> Result<()> {
    let target = parse_target(&args.target)?;

    let base = crate::commands::dashboard::spirit_url();
    let url = format!(
        "{}/api/v1/agents/{}/iterate",
        base.trim_end_matches('/'),
        args.agent
    );
    // La route est sous auth INTERNE seulement — un dispatch pilote n'est pas
    // une lecture d'observabilité. `auth_token()` retombe sur
    // `MIKA_DASHBOARD_TOKEN`, que le serveur refusera : c'est le bon refus, et
    // il est lisible (401).
    let token = crate::commands::dashboard::auth_token()?;

    let response = reqwest::Client::new()
        .post(&url)
        .header("authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "repo": target.repo,
            "issue": target.issue,
            "iteration_context": args.context,
        }))
        // Deux allers-retours `gh` côté serveur (issue, puis PR), bornés à 60 s
        // chacun, plus le spawn. 180 s laisse de la marge sans attendre pour
        // toujours.
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await;

    let response = match response {
        Ok(resp) => resp,
        Err(e) => {
            bail!(
                "mika-spirit injoignable sur {base} : {e}\n\
                 Aucune itération n'a été lancée."
            );
        }
    };

    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .unwrap_or_else(|e| serde_json::json!({ "error": format!("réponse illisible: {e}") }));

    match args.format {
        OutputFormat::Json => writeln!(out, "{}", serde_json::to_string_pretty(&body)?)?,
        OutputFormat::Yaml => writeln!(out, "{}", serde_yaml::to_string(&body)?)?,
        OutputFormat::Text => render_text(status.as_u16(), &body, out)?,
    }

    if status.is_success() {
        Ok(())
    } else {
        // Un refus est un échec de commande : l'exit code doit le dire, sinon un
        // script lit « rien n'a été lancé » comme un succès.
        std::process::exit(1);
    }
}

fn render_text(status: u16, body: &serde_json::Value, out: &mut impl Write) -> Result<()> {
    let outcome = body["outcome"].as_str().unwrap_or("");
    if outcome == "dispatched" {
        writeln!(out, "Itération lancée.")?;
        writeln!(
            out,
            "  PR:       {}",
            body["pr_url"].as_str().unwrap_or("?")
        )?;
        writeln!(
            out,
            "  task:     {}",
            body["task_id"].as_str().unwrap_or("?")
        )?;
        writeln!(
            out,
            "  callback: {}",
            body["callback_task_id"].as_str().unwrap_or("?")
        )?;
        writeln!(
            out,
            "\nLe pilote pousse sur la branche existante — aucune nouvelle PR."
        )?;
        return Ok(());
    }

    // Le motif est le format de fil ; le détail est ce que l'opérateur lit.
    if let Some(reason) = body["refusal_reason"].as_str() {
        writeln!(out, "REFUSÉ ({reason})")?;
        if let Some(detail) = body["detail"].as_str() {
            writeln!(out, "  {detail}")?;
        }
        return Ok(());
    }

    // Ni succès ni refus structuré : 401, 404, 5xx, ou un corps que ce miroir ne
    // sait pas lire (skew de version). On nomme le statut plutôt que d'inventer
    // une cause.
    writeln!(
        out,
        "Non attesté (HTTP {status}) — aucune itération n'a été lancée."
    )?;
    if let Some(err) = body["error"].as_str() {
        writeln!(out, "  {err}")?;
    }
    Ok(())
}

#[cfg(test)]
mod iterate_tests {
    use super::*;

    #[test]
    fn mika2506_les_deux_formes_de_cible_sont_lues() {
        assert_eq!(
            parse_target("mika#2503").unwrap(),
            IterateTarget {
                repo: "mika".into(),
                issue: 2503
            }
        );
        assert_eq!(
            parse_target("senara-solutions/mika#2503").unwrap(),
            IterateTarget {
                repo: "senara-solutions/mika".into(),
                issue: 2503
            }
        );
        // `rsplit_once` : un `#` dans le nom de dépôt ne casse pas la lecture du
        // numéro, qui est toujours le dernier segment.
        assert_eq!(parse_target("  mika#2503  ").unwrap().issue, 2503);
    }

    #[test]
    fn mika2506_une_cible_malformee_est_refusee_avant_tout_appel() {
        for bad in ["mika", "mika#", "#2503", "mika#abc", "mika#0", ""] {
            assert!(
                parse_target(bad).is_err(),
                "`{bad}` doit être refusé plutôt qu'envoyé au serveur"
            );
        }
    }

    /// mika#2506 AC1 — **la CLI ne dispatche rien localement**, structurellement.
    ///
    /// # Pourquoi un scan de source et pas un test comportemental
    ///
    /// Le jour où quelqu'un ouvre la base ici et spawne le pilote, **aucune
    /// assertion ne devient fausse** : la commande marcherait, sur cette machine,
    /// avec ce `PATH` et ce `HOME`. Elle divergerait plus tard, en silence, du
    /// dispatch que spirit possède — sur `mark_parent_dispatched`, sur le bras
    /// `Deferred`, sur l'estampille `fired_at`. C'est la classe que seul un scan
    /// voit.
    ///
    /// # Aucune allowlist, et c'est la forme la plus forte
    ///
    /// Le scan porte sur **un seul fichier**, celui-ci, qui n'a rien à excepter.
    /// Une allowlist née vide serait un emplacement où déposer la prochaine
    /// infraction (mika#2323) ; ne pas en avoir retire l'emplacement. Quand il
    /// tire, on retire le site (doctrine mika#2201).
    ///
    /// Contrôle négatif : `mika2506_le_scan_mord_sur_un_dispatch_local`.
    #[test]
    fn mika2506_la_cli_ne_dispatche_rien_localement() {
        let source = include_str!("iterate.rs");

        // **Le périmètre s'arrête au module de test, et c'est un terme, pas une
        // commodité.** Sans lui le scan accuse ses propres fixtures — mesuré : le
        // contrôle négatif ci-dessous plante littéralement `db.create_task`, et
        // un scan sur le fichier entier le lit comme une infraction. C'est
        // l'idiome maison (`production_sources`, `is_test_source_path`).
        //
        // Anti-vacuité : la présence du marqueur est assertée. Si quelqu'un
        // extrait ce module vers un fichier voisin, `split_once` rend `None` et
        // ce test rougit — au lieu de scanner un fichier sans production et de
        // se lire comme un arbre propre (la prémisse que mika#2321 a dû corriger
        // sur trois gardes du dépôt).
        let marker = format!("#[cfg({})]", "test");
        let (production, _) = source.split_once(marker.as_str()).unwrap_or_else(|| {
            panic!(
                "mika#2506 — marqueur `{marker}` introuvable : le module de test a \
                 été extrait, et ce scan ne regarde plus de production. Repointer \
                 le périmètre plutôt que baisser l'assertion."
            )
        });
        assert!(
            production.contains("/api/v1/agents/"),
            "mika#2506 — la moitié production ne porte plus l'appel HTTP : le \
             périmètre est faux, le scan ne vérifie rien"
        );

        let offenders = local_dispatch_sites(production);

        assert!(
            offenders.is_empty(),
            "mika#2506 AC1 — la CLI dispatcherait localement :\n{offenders:#?}\n\n\
             RÉSOLUTION : retirer le site. La CLI POST sur \
             `/api/v1/agents/{{id}}/iterate` et rend la réponse ; spirit possède \
             le dispatch."
        );
    }

    /// Le contrôle de bonne foi : sans lui, « le scan compte les sites » est
    /// indistinguable de « le scan ne regarde rien » (classe mika#2205).
    #[test]
    fn mika2506_le_scan_mord_sur_un_dispatch_local() {
        let planted = "    let id = db.create_task(new_task).await?;\n";
        assert_eq!(
            local_dispatch_sites(planted).len(),
            1,
            "le scan doit accuser un site de dispatch local planté"
        );

        // Et il ne doit PAS accuser la prose qui en parle — c'est la moitié qui
        // le rend utilisable : ce module explique en commentaire pourquoi il
        // n'appelle rien de tout ça.
        let prose =
            "    // on n'appelle jamais db.create_task ici\n    //! ni spawn_long_running_exec\n";
        assert!(
            local_dispatch_sites(prose).is_empty(),
            "la prose À PROPOS d'un signal n'est pas une occurrence du signal \
             (leçon du Signal S, mika#2050)"
        );
    }

    /// Les sites, extraits pour que le contrôle négatif ci-dessus puisse les
    /// planter sur une source synthétique.
    fn local_dispatch_sites(source: &str) -> Vec<&str> {
        // Chaque aiguille est une façon de dispatcher dans CE processus.
        // Composées à l'exécution pour que les nommer dans la doc ci-dessus ne
        // se dénonce pas — la prose À PROPOS d'un signal n'est pas une
        // occurrence du signal (leçon du Signal S, mika#2050).
        let needles = [
            format!("spawn_long_running{}", "_exec"),
            format!("try_engine_dispatch{}", "_for"),
            format!("build_callback{}_task", ""),
            format!("validate_dispatch{}", "_readiness"),
            format!("extract_branch{}", "_name"),
            format!("create{}_task", ""),
        ];

        source
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                // Les lignes de commentaire sont retirées AVANT analyse : la
                // prose de ce module cite plusieurs de ces symboles pour
                // expliquer pourquoi ils ne sont pas appelés ici.
                !(t.starts_with("//!") || t.starts_with("//") || t.starts_with('*'))
            })
            .filter(|l| needles.iter().any(|n| l.contains(n.as_str())))
            .collect()
    }
}
