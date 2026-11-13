//! `mika flight-verdicts` — l'adaptateur du registre des verdicts de vol
//! (mika#2922, vols 2 et 3).
//!
//! # Ce que ce module est
//!
//! Un **adaptateur mince**, et rien d'autre : il analyse les trois termes par
//! [`parse_judgment_terms`], inscrit par [`record_mpc_judgment`], et rend ce
//! qui s'est passé. Aucune écriture d'audit, aucune recomposition de clé,
//! aucune logique de domaine — ces trois gestes vivent dans
//! `mika_agent::flight_verdict_ledger`, qui est l'écrivain unique du
//! `tool_name` du jugement.
//!
//! Le vol 3 ajoute `concordance`, et la même minceur y tient : la lecture est
//! [`read_concordance`], le rendu est [`render_concordance_text`], et cet
//! adaptateur n'arbitre que la **forme** de sortie. Le lecteur unique des deux
//! registres vit lui aussi dans `flight_verdict_ledger`, et pour la même raison
//! de fond : ni l'écriture ni la lecture d'un `tool_name` ne peut vivre dans un
//! crate à qui une garde interdit de le nommer.
//!
//! `concordance` porte `--format` et `judge` n'en porte pas : le premier est un
//! verbe de **lecture**, qu'un script peut vouloir en JSON ; le second est un
//! verbe d'écriture, comme `tasks cancel` et `tasks rearm`.
//!
//! La raison n'est pas l'élégance. La garde d'unicité d'écrivain
//! (`mika2922_ac2_un_seul_ecrivain_du_jugement_de_mpc`) balaie le `src` de
//! **mika-agent** : un écrivain posé ici lui serait invisible, et sa verdeur
//! ne dirait plus rien. D'où la garde de ce côté-ci,
//! [`tests::mika2922_ladaptateur_nest_pas_un_ecrivain`], qui refuse dans ce
//! crate le geste par lequel un second écrivain naîtrait — nommer le
//! `tool_name` de l'un des deux registres, par son littéral ou par sa
//! constante — et son contrôle négatif
//! [`tests::mika2922_le_scan_de_ladaptateur_mord`].
//!
//! # Ce que ce module n'est pas
//!
//! Il ne tient aucune table de graphies. Les termes acceptés sont ceux que
//! serde écrit, et l'analyse en est l'inverse exact : une sixième cause de
//! mort ajoutée en amont est acceptée ici sans qu'une ligne bouge (classe
//! mika#2158). Un terme hors contrat est refusé par l'analyseur, qui nomme
//! les valeurs acceptées — ce module ne les connaît pas.
//!
//! # L'ordre des deux gestes
//!
//! L'analyse passe **avant** l'ouverture de la base : un terme mal tapé est
//! refusé sans qu'aucune connexion soit ouverte, et surtout sans qu'aucune
//! ligne soit inscrite. L'inverse rendrait l'erreur plus lente et, sur une
//! commande qui écrit, plus coûteuse à comprendre.

use anyhow::Result;
use mika_agent::flight_verdict_ledger::{
    MpcJudgmentOutcome, VerdictTerms, parse_judgment_terms, read_concordance, record_mpc_judgment,
    render_concordance_text,
};

use crate::cli::{FlightVerdictsArgs, FlightVerdictsCommand, OutputFormat};
use crate::init;

pub async fn run(args: FlightVerdictsArgs, agent_name: &str) -> Result<()> {
    match args.command {
        FlightVerdictsCommand::Judge {
            task_id,
            issue,
            cause,
            recommendation,
            note,
        } => {
            let record = parse_judgment_terms(&issue, cause.as_deref(), &recommendation)?;

            let ctx = init::init_db_only_for_agent(agent_name)?;
            let db = &ctx.async_db;

            match record_mpc_judgment(db, &task_id, &record, note.as_deref()).await? {
                MpcJudgmentOutcome::Inscribed => {
                    println!("\n  Jugement inscrit sur le vol {task_id} :");
                    print_terms(&record.terms());
                    println!();
                }
                MpcJudgmentOutcome::AlreadyRecorded { held } => {
                    println!("\n  Le vol {task_id} est déjà jugé — rien n'a été réécrit.");
                    match held {
                        Some(held) => {
                            let held_terms = held.terms();
                            println!("  Jugement en place :");
                            print_terms(&held_terms);
                            // Dire que les deux diffèrent, sans arbitrer : c'est
                            // l'opérateur qui sait lequel des deux est le bon, et
                            // la commande n'offre aucun moyen d'écraser.
                            if held_terms != record.terms() {
                                println!(
                                    "\n  Celui que tu viens de proposer en diffère. Pour le \
                                     corriger il faut lever la ligne en place, ce que cette \
                                     commande ne fait pas."
                                );
                            }
                        }
                        // Pas de `VerdictRecord` fabriqué pour combler : un
                        // jugement inventé est pire qu'un jugement illisible
                        // (motif mika#2331). Le WARN du registre nomme l'étape.
                        None => println!("  Il n'a pas pu être rendu — il reste inscrit."),
                    }
                    println!();
                }
            }
            Ok(())
        }
        FlightVerdictsCommand::Concordance { format } => {
            let ctx = init::init_db_only_for_agent(agent_name)?;
            let report = read_concordance(&ctx.async_db).await?;

            match format {
                // Le `Text` de cette commande est un **rendu**, et non la
                // sortie de secours en JSON joli qu'emploie
                // `commands/milestone.rs::emit` pour `read`/`assess` : le
                // tableau à cinq colonnes est la surface humaine de ce verbe,
                // il n'y en a pas d'autre ailleurs.
                OutputFormat::Text => print!("\n{}", render_concordance_text(&report)),
                // Le **même** type qu'en texte, sérialisé : un opérateur et un
                // script lisent le même rapport. Le champ divergent y est un
                // champ à part (`accord` + `field`), là où le texte le porte
                // dans sa colonne « accord ».
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
                OutputFormat::Yaml => print!("{}", yaml_serde::to_string(&report)?),
            }
            Ok(())
        }
    }
}

fn print_terms(terms: &VerdictTerms) {
    println!("    issue          {}", terms.issue);
    if let Some(cause) = &terms.cause {
        println!("    cause          {cause}");
    }
    println!("    recommandation {}", terms.recommendation);
}

#[cfg(test)]
mod tests {
    /// **AC2, la moitié de ce crate.** Ni le verdict du manager ni le jugement
    /// de MPC ne s'écrit d'ici.
    ///
    /// La garde côté mika-agent balaie le `src` de son propre crate et ne voit
    /// donc rien de celui-ci. Celle-ci couvre le même contrat sur l'autre
    /// crate, et le prédicat est **le `tool_name`, jamais `log_audit_event`** :
    /// ce crate écrit légitimement des lignes d'audit sous d'autres noms d'outil
    /// (`commands/tasks.rs`, `tui/app.rs`), et une garde qui les refuserait
    /// parlerait d'autre chose. Ce qui ferait un second écrivain **de ces deux
    /// registres** est de nommer leur `tool_name` ici — par le littéral recopié
    /// ou par la constante importée — et c'est exactement ce qui est refusé.
    ///
    /// Les quatre aiguilles sont **concaténées** plutôt qu'écrites d'un bloc,
    /// pour que la garde ne dépende pas de l'exactitude du masquage de
    /// `cfg(test)` : écrites entières, elles mordraient sur la garde elle-même.
    ///
    /// Aucune liste d'exception, et il n'y en a aucune à ajouter : la liste est
    /// **absente**, pas vide (doctrine mika#2201). Quand elle rougit, il y a
    /// deux remèdes selon le geste — une écriture se route par
    /// `record_mpc_judgment`, une **lecture** par l'accesseur du registre (le
    /// motif de mika#2267 : le lecteur ne recompose pas la clé du scribe, sinon
    /// il regarde où l'autre n'écrit pas).
    ///
    /// La dernière assertion est l'anti-vacuité, et elle porte sur ce
    /// fichier-ci : un balayage qui ne voit pas l'adaptateur ne protège pas
    /// l'adaptateur (classe mika#2205).
    #[test]
    fn mika2922_ladaptateur_nest_pas_un_ecrivain() {
        let aiguilles = [
            String::from("\"mpc_flight_") + "judgment\"",
            String::from("\"manager_flight_") + "verdict\"",
            String::from("MPC_FLIGHT_JUDGMENT_") + "TOOL",
            String::from("MANAGER_FLIGHT_VERDICT_") + "TOOL",
        ];

        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));

        let mut violations: Vec<String> = Vec::new();
        let mut vu_ladaptateur = false;
        scanner.for_each(|path, production| {
            let rel = path
                .strip_prefix(scanner.src_root())
                .unwrap_or(path)
                .display()
                .to_string();
            if rel == "commands/flight_verdicts.rs" {
                vu_ladaptateur = true;
            }
            for (idx, line) in production.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if aiguilles.iter().any(|a| line.contains(a.as_str())) {
                    violations.push(format!("{rel}:{} — {}", idx + 1, line.trim()));
                }
            }
        });

        assert!(
            violations.is_empty(),
            "mika#2922 : le registre des verdicts de vol a un seul écrivain par \
             tool_name, et il vit dans mika_agent::flight_verdict_ledger. \
             Site(s) nommant un de ces tool_name dans ce crate :\n{}\n\
             Une écriture se route par record_mpc_judgment, une lecture par \
             l'accesseur du registre.",
            violations.join("\n")
        );
        assert!(
            vu_ladaptateur,
            "anti-vacuité : le balayage ne voit pas commands/flight_verdicts.rs, \
             donc il ne dit rien de l'adaptateur"
        );
    }

    /// ...et le contrôle négatif : le balayage mord bel et bien.
    ///
    /// Sans lui, les quatre aiguilles concaténées pourraient être fausses — un
    /// underscore de travers, une casse — et la garde ci-dessus resterait verte
    /// pour la mauvaise raison (classe mika#2205). Le prédicat est reproduit à
    /// l'identique sur deux sources fabriquées : l'une porte le geste refusé,
    /// l'autre une écriture d'audit **légitime** sous un autre `tool_name`,
    /// c'est-à-dire exactement la population que la garde ne doit pas refuser.
    #[test]
    fn mika2922_le_scan_de_ladaptateur_mord() {
        let aiguilles = [
            String::from("\"mpc_flight_") + "judgment\"",
            String::from("\"manager_flight_") + "verdict\"",
            String::from("MPC_FLIGHT_JUDGMENT_") + "TOOL",
            String::from("MANAGER_FLIGHT_VERDICT_") + "TOOL",
        ];
        let mord = |source: &str| {
            source
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .any(|l| aiguilles.iter().any(|a| l.contains(a.as_str())))
        };

        assert!(
            mord(
                "    db.log_audit_event(sid, MPC_FLIGHT_JUDGMENT_TOOL, key, None, None, None, None).await?;"
            ),
            "le scan doit mordre sur un second écrivain par la constante"
        );
        assert!(
            mord("    let tool = \"mpc_flight_judgment\";"),
            "le scan doit mordre sur le littéral recopié"
        );
        assert!(
            !mord(
                "    db.log_audit_event(sid, \"task_cancel\", key, None, None, None, None).await?;"
            ),
            "le scan ne doit RIEN dire d'une écriture d'audit sous un autre tool_name"
        );
    }
}
