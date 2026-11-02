//! `mika arch-disposition --pass <first|second> --response-file <path>` (mika#2194, vol 5).
//!
//! Le canal par lequel `dispatch-lib.sh::_parse_disposition` et
//! `::_parse_verdict` interrogent le lecteur unique des cinq tiers
//! ([`mika_agent::arch_disposition::decide`]) au lieu de porter une seconde
//! échelle de tiers en bash.
//!
//! # Ce module est un CANAL, et il ne contient aucune règle de tier
//!
//! Les cinq tiers vivent dans `mika-agent` depuis le vol 4, avec leur corpus
//! doré et ses deux lecteurs. Ici il n'y a qu'une lecture de fichier, un appel à
//! `decide`, et un formatage. C'est voulu : **la seule chose qu'un canal peut
//! faire de mal, c'est perdre une information en la formatant** — d'où le test
//! de corpus ci-dessous, qui relit les 29 lignes d'attendus depuis l'autre
//! crate.
//!
//! # Le corps passe par un FICHIER, jamais par un argument
//!
//! Une réponse d'architecte porte des retours à la ligne, des backticks et des
//! `$` : la passer en argv ré-introduirait la classe de panne de portée de
//! guillemets dans le geste même qui prétend la fermer (cpp#157, l'une des trois
//! classes que mika#2194 cite). Le refus est **structurel** — il n'y a pas de
//! variante positionnelle à cette sous-commande, donc il n'y a rien à
//! contourner.
//!
//! # Trois codes de sortie, et deux lignes TOUJOURS
//!
//! | code | stdout | sens |
//! |---|---|---|
//! | `0` | ligne 1 = le jeton, ligne 2 = le tier | un jeton a été lu |
//! | `1` | ligne 1 **vide**, ligne 2 = `0` ou `-` | **aucun jeton** — UNPARSED |
//! | `≥2` | vide, motif sur stderr | **je n'ai pas pu regarder** |
//!
//! Les deux lignes sont écrites même au code `1` : un lecteur bash qui fait
//! `sed -n '2p'` sur une sortie d'une seule ligne lit le vide et ne peut pas le
//! distinguer d'un tier absent. La règle « une colonne vide porte `-`, jamais
//! rien » du corpus s'applique ici à la **ligne 2 seule** — la ligne 1 est
//! légitimement vide au code `1`, c'est sa signification.
//!
//! Et la ligne 2 sépare deux populations que le code `1` réunit : `0` dit que le
//! moteur a **retiré** la disposition (mika#2037, une absence décidée), `-` dit
//! que personne n'a rien trouvé. Deux populations sous un seul nom ne sont pas
//! soustractibles (classe mika#2205).
//!
//! # Le chemin court, avant toute résolution
//!
//! Cette sous-commande sort dans le `match &cli.command` de tête de `main.rs`,
//! à côté de `Token`, `CredentialHelper` et `PlanCallout`, et fait strictement
//! moins que `Token` : ni `dotenv`, ni `Settings`, ni `home`, ni base, ni
//! réseau. C'est la réponse prise d'avance à la sonde S3 du corps du ticket —
//! « si S3 dépasse environ 500 ms, le remède est le placement dans `main.rs`,
//! jamais un cache ». La bascule ajoute un démarrage de process par passe
//! d'architecte, et environ 87 à travers les suites de tests (§ 4 du plan du
//! vol 5) : c'est ce placement qui rend ce coût acceptable.
//!
//! # La forme du fragment est contrainte par un lecteur existant
//!
//! Le bloc de parité de `test-dispatch-lib.sh` extrait le fragment avec
//! `sed -n "s/^.*(matched: '\(.*\)')$/\1/p"`, parce que c'est la forme que le
//! bash écrivait sur stderr au tier 2. L'AC2 exige que les attentes des tests
//! existants ne changent pas, donc [`tier_2_note`] se termine par
//! `(matched: '<fragment>')` — le fragment étant le texte apparié **tel qu'il
//! figure dans l'entrée**, casse comprise, et non le motif de la table.
//! `Decision::fragment` est déjà une tranche de l'entrée : il n'y a rien à
//! recalculer, seulement à ne pas le perdre à l'écriture.

use std::path::Path;

use mika_agent::arch_disposition::{ArchPass, Decision, decide};

/// Le code de sortie « je n'ai pas pu regarder ».
///
/// Un seul site de définition : c'est un **format de fil** lu par les deux
/// enveloppes de `dispatch-lib.sh`, qui distinguent `1` (aucun jeton) de `≥2`
/// (refus bruyant, annexé au `RESULT` du callback). Deux littéraux pourraient
/// diverger.
pub const EXIT_UNREADABLE: i32 = 2;

/// Le préfixe des motifs de refus, repris par les surfaces opérateur.
const REFUSAL_PREFIX: &str = "REFUSED (arch-disposition, mika#2194)";

/// Le résultat de la sous-commande, avant de devenir un code de sortie.
///
/// Un `enum` plutôt qu'un `Result<Option<_>, _>` : les trois issues sont trois
/// réponses différentes à la question posée, et « aucun jeton » n'est ni une
/// erreur ni un succès partiel. C'est ce qui rend les trois codes — l'AC3
/// comprise — exerçables sans lancer de process.
#[derive(Debug, PartialEq, Eq)]
pub enum DispositionOutcome {
    /// Un jeton a été lu. Code `0`.
    Read {
        /// `READY` | `ITERATE` | `ESCALATE` | `GROOMED` — ligne 1 de stdout.
        token: &'static str,
        /// `0b` | `1a` | `1b` | `2` — ligne 2 de stdout.
        tier: &'static str,
        /// Le fragment décisif, tier 2 uniquement.
        fragment: Option<String>,
    },
    /// Aucun jeton — UNPARSED. Code `1`, ligne 1 vide, ligne 2 = `0` ou `-`.
    Unparsed {
        /// `0` (le moteur a retiré la disposition) ou `-` (aucun tier n'a matché).
        tier: &'static str,
    },
    /// On n'a pas pu regarder. Code `≥2`, stdout vide, motif sur stderr.
    Unreadable {
        reason: &'static str,
        detail: String,
    },
}

/// La stderr du tier 2, dont la FORME est un contrat (voir l'en-tête).
///
/// Une fonction plutôt qu'un `eprintln!` en ligne : la forme est lue par un
/// `sed` existant, donc elle est assertable, et elle l'est — un `format!` noyé
/// dans [`run`] ne le serait pas sans capturer un flux.
fn tier_2_note(token: &str, fragment: &str) -> String {
    format!("arch-disposition: mapped paraphrased disposition → {token} (matched: '{fragment}')")
}

/// Traduit la décision du lecteur unique en issue de canal.
///
/// Aucun arbitrage ici : `token: None` est un UNPARSED quel que soit le tier qui
/// l'a décidé, et le tier traverse **tel quel** vers la ligne 2. Le tier 0 passe
/// donc par ce bras avec `tier = "0"`, ce qui est exactement le contrat.
fn as_outcome(decision: Decision<'_>) -> DispositionOutcome {
    match decision.token {
        Some(token) => DispositionOutcome::Read {
            token,
            tier: decision.tier.as_str(),
            fragment: decision.fragment.map(str::to_owned),
        },
        None => DispositionOutcome::Unparsed {
            tier: decision.tier.as_str(),
        },
    }
}

/// Lit le fichier de réponse et décide.
///
/// L'entrée-sortie est **ici** et non dans [`run`], parce que « je n'ai pas pu
/// regarder » est une issue du canal : la population « fichier illisible » est
/// créée par la migration — avant elle, le texte arrivait par un tube — et
/// l'AC3 exige qu'elle soit épinglée aux deux polarités. La mettre derrière un
/// `println!` la rendrait inexerçable sans process.
pub fn outcome(response_file: &Path, pass: ArchPass) -> DispositionOutcome {
    match std::fs::read_to_string(response_file) {
        Ok(text) => as_outcome(decide(&text, pass)),
        Err(e) => DispositionOutcome::Unreadable {
            reason: "response_file_unreadable",
            detail: format!("{} — {e}", response_file.display()),
        },
    }
}

/// Écrit le canal et rend le code de sortie.
///
/// Ne rend pas `anyhow::Result` : les trois issues **sont** des codes, et
/// laisser `main` écraser une erreur en `1` effacerait la distinction entre
/// « aucun jeton » et « je n'ai pas pu regarder » — c'est-à-dire le livrable de
/// l'AC3.
pub fn run(response_file: &Path, pass: ArchPass) -> i32 {
    match outcome(response_file, pass) {
        DispositionOutcome::Read {
            token,
            tier,
            fragment,
        } => {
            println!("{token}");
            println!("{tier}");
            if let Some(fragment) = fragment {
                eprintln!("{}", tier_2_note(token, &fragment));
            }
            0
        }
        DispositionOutcome::Unparsed { tier } => {
            // Ligne 1 vide, ligne 2 le tier : deux lignes, toujours.
            println!();
            println!("{tier}");
            1
        }
        DispositionOutcome::Unreadable { reason, detail } => {
            eprintln!("{REFUSAL_PREFIX} {reason}: {detail}");
            EXIT_UNREADABLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le corpus doré traverse le canal sans perdre une information.
    ///
    /// Ce test lit le corpus de `mika-agent` depuis `mika-cli`, comme
    /// `plan_callout.rs` le fait pour le sien : il atteste que la sous-commande
    /// répond comme le lecteur unique, plutôt que comme une troisième
    /// interprétation. Les trois colonnes assertées sont les trois sorties du
    /// canal — jeton (ligne 1), tier (ligne 2), fragment (stderr).
    ///
    /// Il porte aussi la couverture des **cinq tiers sur la ligne 2** : la
    /// dernière assertion exige que les six valeurs du vocabulaire fermé aient
    /// été exercées, donc un corpus qui perdrait un tier rougit ici au lieu de
    /// passer pour complet (classe mika#2205).
    #[test]
    fn mika2194_le_canal_rend_le_corpus_dore_tier_compris() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../mika-agent/tests/fixtures/arch_paraphrase_texts");
        let tsv = corpus.join("expectations.tsv");
        let content = std::fs::read_to_string(&tsv)
            .unwrap_or_else(|e| panic!("corpus introuvable : {} ({e})", tsv.display()));

        let mut checked = 0usize;
        let mut tiers_seen: Vec<&str> = Vec::new();
        for line in content.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            assert_eq!(cols.len(), 6, "format du TSV : {line:?}");
            let (file, pass, token, fragment, tier) = (cols[0], cols[1], cols[2], cols[3], cols[4]);
            let pass = match pass {
                "first" => ArchPass::First,
                "second" => ArchPass::Second,
                other => panic!("{file}: passe inconnue {other:?}"),
            };

            match outcome(&corpus.join(file), pass) {
                DispositionOutcome::Read {
                    token: got_token,
                    tier: got_tier,
                    fragment: got_fragment,
                } => {
                    assert_eq!(got_token, token, "{file} ({tier}): jeton divergent");
                    assert_eq!(got_tier, tier, "{file}: tier divergent");
                    // `fragment ≠ « - » ⟺ tier = « 2 »`, la règle du corpus.
                    match got_fragment {
                        Some(f) => {
                            assert_eq!(f, fragment, "{file}: fragment divergent");
                            assert!(
                                tier_2_note(got_token, &f)
                                    .ends_with(&format!("(matched: '{fragment}')")),
                                "{file}: la forme lue par le `sed` du bloc de parité est perdue"
                            );
                        }
                        None => assert_eq!(fragment, "-", "{file}: fragment attendu, aucun rendu"),
                    }
                }
                DispositionOutcome::Unparsed { tier: got_tier } => {
                    assert_eq!(
                        token, "-",
                        "{file}: aucun jeton alors que {token} est attendu"
                    );
                    assert_eq!(got_tier, tier, "{file}: tier divergent");
                }
                other => panic!("{file}: un corps du corpus n'est jamais refusé, lu {other:?}"),
            }

            if !tiers_seen.contains(&tier) {
                tiers_seen.push(tier);
            }
            checked += 1;
        }

        assert!(
            checked > 0,
            "aucun cas exercé — le canal n'a été comparé à rien (anti-vacuité)"
        );
        tiers_seen.sort_unstable();
        assert_eq!(
            tiers_seen,
            vec!["-", "0", "0b", "1a", "1b", "2"],
            "la ligne 2 n'a pas été exercée sur tout son vocabulaire fermé"
        );
        println!("mika#2194 — canal CLI comparé au lecteur sur {checked} cas");
    }

    /// AC3, les **deux polarités dans le même corps** : le refus sort quand on
    /// n'a pas pu regarder, et rien ne sort sur une réponse saine.
    ///
    /// Les deux ensemble, parce qu'un test qui ne vérifie que le refus ne
    /// distingue pas « refuse quand il faut » de « refuse toujours » — et un
    /// canal qui refuserait chaque passe d'architecte passerait tous les autres
    /// tests de ce fichier au vert.
    #[test]
    fn mika2194_un_fichier_illisible_est_refuse_et_une_reponse_saine_ne_lest_pas() {
        let dir = tempfile::tempdir().expect("répertoire temporaire");

        let absent = dir.path().join("reponse-qui-nexiste-pas.md");
        match outcome(&absent, ArchPass::First) {
            DispositionOutcome::Unreadable { reason, detail } => {
                assert_eq!(reason, "response_file_unreadable");
                assert!(
                    detail.contains("reponse-qui-nexiste-pas.md"),
                    "le motif doit nommer le fichier, lu {detail:?}"
                );
            }
            other => panic!("attendu un refus nommé, lu {other:?}"),
        }

        let saine = dir.path().join("reponse.md");
        std::fs::write(&saine, "Disposition: READY\n\nShip it.\n").expect("écriture du corps");
        match outcome(&saine, ArchPass::First) {
            DispositionOutcome::Read { token, tier, .. } => {
                assert_eq!(token, "READY");
                assert_eq!(tier, "1a");
            }
            other => panic!("une réponse saine ne doit jamais être refusée, lu {other:?}"),
        }
    }

    /// Le code `1` porte deux populations, et c'est la ligne 2 qui les sépare.
    ///
    /// `0` = le moteur a retiré la disposition (une absence **décidée**),
    /// `-` = aucun tier n'a matché. Le corpus les exerce tous deux ; ce test
    /// épingle la **traduction** que fait le canal, qui est le seul endroit où
    /// elles pourraient se confondre en une seule ligne vide.
    #[test]
    fn mika2194_le_code_1_distingue_le_tier_0_du_tier_absent() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../mika-agent/tests/fixtures/arch_paraphrase_texts");

        assert_eq!(
            outcome(
                &corpus.join("tier-0-marqueur-en-tete-de-ligne.md"),
                ArchPass::First
            ),
            DispositionOutcome::Unparsed { tier: "0" }
        );
        assert_eq!(
            outcome(&corpus.join("french-inert-2641.md"), ArchPass::First),
            DispositionOutcome::Unparsed { tier: "-" }
        );
    }
}
