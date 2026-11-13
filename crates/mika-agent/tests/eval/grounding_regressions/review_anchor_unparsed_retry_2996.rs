//! Scénario : la relance UNPARSED ne fait pas avancer un groom sans ancrage sur le plan (mika#2996)
//!
//! Contexte : la garde d'ancrage de mika#2037 ne s'arme que si le brief du tour fait au
//! moins `review_anchor_min_brief_chars` caractères. `_iterate_groom_loop`
//! (`skills/bundled/_shared/dispatch-lib.sh`) relance l'architecte quand sa réponse n'a pas
//! de disposition lisible — en première passe par mika#1823, en seconde par mika#2641 — puis
//! **accepte la disposition rendue à ce tour**. Le message de relance faisait ~380
//! caractères : la garde ne s'armait pas, et un `READY` non ancré faisait avancer la chaîne.
//!
//! Mesuré le 2026-10-31, trace `15ea28bd` : l'architecte, relancé sur le groom de mika#2995,
//! rend la revue d'un **autre** plan (mika#2752), close par `Disposition: READY`. Trail :
//! `UNPARSED`, `READY-after-retry`. La seconde passe a rattrapé l'erreur ce jour-là ; rien ne
//! garantit qu'elle le fera toujours, et le même chemin existe en seconde passe avec
//! `Verdict: GROOMED`, qui ramène le dispatch vers dev-pilot (mika#996).
//!
//! # Ce que ces tests font tourner, et pourquoi trois couches
//!
//! La décision d'avancer se prend en trois temps, sur deux langages :
//!
//! 1. le **shell** écrit le message de relance (`_first_pass_retry_prompt`,
//!    `_second_pass_retry_prompt`) ;
//! 2. le **moteur** reçoit ce message comme brief, et la garde d'ancrage décide si la
//!    disposition survit ;
//! 3. le **shell** lit la disposition rendue (`_parse_disposition`, `_parse_verdict`), et
//!    c'est ce mot qui route `_iterate_groom_loop` vers la passe suivante ou vers l'arrêt.
//!
//! Chaque test traverse les trois, avec les vraies fonctions du shell (sourcées depuis
//! `dispatch-lib.sh`) et le vrai `run_agent`. Un test qui construirait le message de relance
//! en Rust passerait le jour où le shell cesse d'y mettre le plan ; un test shell avec un
//! moteur simulé passerait le jour où la garde cesse de s'armer. Le contrat d'armement
//! (préfixes, compte, longueur de citation, seuil) est lu dans le **vrai** `skill.toml`.
//!
//! # Contrôle négatif dans le même appel
//!
//! Chaque test exécute deux tours sur le **même** message de relance : la réponse refusée,
//! puis une réponse ancrée sur le plan. Le refus seul se lirait aussi bien comme « la garde
//! refuse tout » ; l'acceptation seule comme « la garde ne s'arme pas ». C'est la paire qui
//! dit que la garde s'arme **et** qu'elle juge contre le plan.
//!
//! ## Tags
//! - `grounding:review-anchor-required` — succès après correctif
//! - `grounding:unanchored-ready` — l'échec d'avant correctif
//!
//! Référence : mika#2996, mika#2037, mika#1823, mika#2641.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{SkillInfo, SkillManifest, Triggers};

use super::*;

const FIXTURE_DIR: &str = "tests/fixtures/review_anchor_unparsed_retry_2996";
const PLAN_FIXTURE: &str = "plan-mika2995-61a3fb572.md";
const RESPONSE_15EA28BD: &str = "response-15ea28bd-msg123050.md";

/// Trois citations verbatim du plan, à trois endroits distincts — vérifiées par `grep -F`
/// contre la fixture à l'écriture du test.
const ANCHORS_ON_PLAN: &str = "A1: \"C'est la deuxième des trois voies qu'AC1 énumère. Le mécanisme existe déjà\" — la voie est nommée et justifiée.\n\
     A2: \"Trois faits qui bornent le coût, et qu'il faut lire avant de craindre une régression\" — le coût est borné avant d'être craint.\n\
     A3: \"La première branche est le geste de l'incident, décrit à l'agent sous forme de gabarit.\" — l'incident est rejoué tel quel.\n\
     \n\
     Aucun constat bloquant.\n";

fn crate_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    crate_dir().join("../..")
}

fn dispatch_lib() -> PathBuf {
    repo_root().join("skills/bundled/_shared/dispatch-lib.sh")
}

fn fixture_path(name: &str) -> PathBuf {
    crate_dir().join(FIXTURE_DIR).join(name)
}

fn read_fixture(name: &str) -> String {
    let path = fixture_path(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("lire {}: {e}", path.display()))
}

/// Le répertoire du binaire `mika`, à mettre en tête du `PATH` du bash de test.
///
/// Depuis mika#2194 (vol 5), `_parse_disposition` et `_parse_verdict` délèguent à
/// `mika arch-disposition`. Sans le binaire, chaque enveloppe prend son chemin de refus et
/// rend un mot vide : la lecture « sans garde » de la fixture rougirait sur une cause qui
/// n'est pas la sienne, et un `assert_ne!(…, "READY")` passerait sur rien.
///
/// `mika` appartient à `mika-cli`, qui dépend de ce crate : `env!("CARGO_BIN_EXE_mika")`
/// n'existe pas ici. Le binaire est donc cherché à côté de `mika-spirit`, dans le même
/// répertoire de cible, où le `cargo test` de l'espace de travail le construit avant de
/// lancer le moindre test. Un binaire absent, ou périmé au point d'ignorer la
/// sous-commande (`cargo test -p mika-agent` seul ne le reconstruit pas), arrête le test
/// avec son remède au lieu de le laisser lire un mot vide.
fn mika_bin_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_BIN_EXE_mika-spirit"))
        .parent()
        .expect("le binaire mika-spirit a un répertoire parent")
        .to_path_buf();
    let mika = dir.join("mika");
    let probe = Command::new(&mika)
        .args(["arch-disposition", "--help"])
        .output();
    match probe {
        Ok(out) if out.status.success() => dir,
        Ok(out) => panic!(
            "{} ne connaît pas `arch-disposition` (binaire périmé ?). Remède : \
             cargo build -p mika-cli --bin mika\nstderr: {}",
            mika.display(),
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => panic!(
            "binaire {} introuvable ({e}). Remède : cargo build -p mika-cli --bin mika",
            mika.display()
        ),
    }
}

/// Source `dispatch-lib.sh` dans un bash neuf et y exécute `script`, avec `args` en `$1…`.
///
/// Le source est fait comme `test-dispatch-lib.sh` le fait (`2>/dev/null || true`) : la
/// bibliothèque se charge hors de tout contexte de dispatch. Le `PATH` porte le binaire
/// `mika` en tête (voir [`mika_bin_dir`]).
fn bash_with_dispatch_lib(script: &str, args: &[&str]) -> String {
    let full = format!("source \"$0\" >/dev/null 2>&1 || true\n{script}");
    let path = match std::env::var_os("PATH") {
        Some(p) => {
            let mut dirs = vec![mika_bin_dir()];
            dirs.extend(std::env::split_paths(&p));
            std::env::join_paths(dirs).expect("PATH joignable")
        }
        None => mika_bin_dir().into_os_string(),
    };
    let out = Command::new("bash")
        .arg("-c")
        .arg(&full)
        .arg(dispatch_lib())
        .args(args)
        .env("PATH", path)
        .output()
        .expect("lancer bash");
    assert!(
        out.status.success(),
        "le script shell a échoué ({}): {}\nstderr: {}",
        out.status,
        script,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("sortie shell UTF-8")
}

/// Le message de relance que `_iterate_groom_loop` envoie, produit par la fonction de
/// production elle-même.
fn retry_message(function: &str) -> String {
    let plan = fixture_path(PLAN_FIXTURE);
    let plan = plan.to_str().expect("chemin UTF-8");
    bash_with_dispatch_lib(
        "f=$(\"$1\" \"$2\") || exit 3\ncat -- \"$f\"\nrm -f -- \"$f\"",
        &[function, plan],
    )
}

/// Le mot qui route la boucle : ce que le parseur de production lit dans la réponse finale.
fn routed_word(parser: &str, response: &str) -> String {
    bash_with_dispatch_lib("printf '%s' \"$2\" | \"$1\"", &[parser, response])
        .trim()
        .to_string()
}

/// Une entrée de registre dont le contrat `[output]` est celui du **vrai** manifeste.
///
/// `always_on` comme en production — la relance ne porte aucun mot-clé de déclenchement.
/// `[constraints]` est laissé par défaut : `required_tools = ["gh_read"]` est une autre garde,
/// dont le re-prompt consommerait les réponses simulées sans rien dire de celle-ci.
fn registry_from_manifest(skill: &str) -> SkillRegistry {
    let path = repo_root()
        .join("skills/bundled")
        .join(skill)
        .join("skill.toml");
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("lire {}: {e}", path.display()));
    let parsed: SkillManifest =
        toml::from_str(&raw).unwrap_or_else(|e| panic!("analyser {}: {e}", path.display()));
    assert!(
        !parsed.output.required_review_anchor_prefixes.is_empty(),
        "{skill} ne déclare plus de contrat d'ancrage : ce test ne mesurerait rien"
    );
    SkillRegistry::from_test_entries(vec![SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: skill.to_string(),
                description: format!("{skill} (contrat [output] du manifeste réel)"),
                version: "0.1.0".to_string(),
                always_on: true,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers { keywords: vec![] },
            llm: Default::default(),
            constraints: Default::default(),
            output: parsed.output,
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from(format!("/skills/{skill}")),
        keywords_lower: vec![],
        prompt_snippet: String::new(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }])
}

/// Début fixe du re-prompt correctif de la garde d'ancrage (`agent_loop/mod.rs`, mika#2037).
const ANCHOR_REPROMPT_HEAD: &str = "[mika-engine] The previous response carries a non-terminal \
     disposition without the review attestation";

/// Combien de fois l'architecte simulé répond la même chose : assez pour que les re-prompts
/// d'autres gardes ne fassent pas manquer de réponses au moteur.
///
/// **Le compte reste 4, et la raison de le garder n'est plus celle qui l'a fixé.** Mesuré à
/// l'écriture du correctif de mika#2996 : sur un brief qui porte ce plan, la garde de
/// sémantique de reprise (« A resume/continue instruction was received… ») re-promptait une
/// fois avant la garde d'ancrage, et elle tirait aussi en production sur le tour d'origine —
/// c'est l'UNPARSED du 2026-10-31, trace `3a066c59`, dont la réponse de 264 caractères
/// disait chercher des engagements « comme demandé par l'instruction de reprise ». **Depuis
/// mika#3002 elle ne tire plus ici** : les deux skills de ce fichier sont des
/// verdict-producers, le message de relance porte l'enveloppe du plan, et le déclencheur
/// n'est donc évalué que sur la région adressée à l'agent. Le compte garde sa marge parce
/// qu'il borne les re-prompts de **toutes** les gardes, pas d'une seule.
///
/// Les assertions portent, elles, sur le re-prompt d'ancrage et sur rien d'autre — c'est ce
/// qui a permis à ce fichier de rester vert sans retouche à travers mika#3002.
const REPLIES: usize = 4;

struct EngineTurn {
    /// La réponse finale du tour, celle que le shell lit.
    text: String,
    /// Nombre de requêtes LLM dont le dernier message est le re-prompt de la garde d'ancrage.
    anchor_reprompts: usize,
}

/// Un tour du moteur sur `message`, l'architecte simulé répondant toujours `reply`.
async fn engine_turn(skill: &str, message: &str, reply: &str) -> anyhow::Result<EngineTurn> {
    let harness = EvalHarness::builder()
        .responses((0..REPLIES).map(|_| text_response(reply)).collect())
        .skills(registry_from_manifest(skill))
        .build()
        .await?;
    let trace = harness.run(message).await?;
    assert_has_output(&trace);
    let anchor_reprompts = trace
        .captured_requests
        .iter()
        .filter(|req| {
            matches!(
                req.messages.last().map(|m| &m.content),
                Some(mika_common::llm::LlmContent::Text(t)) if t.starts_with(ANCHOR_REPROMPT_HEAD)
            )
        })
        .count();
    Ok(EngineTurn {
        text: trace.output.text.clone().unwrap_or_default(),
        anchor_reprompts,
    })
}

/// La réponse 15ea28bd avec sa dernière ligne non vide remplacée par `line`.
///
/// Pour la seconde passe : le cas est structurel et non observé (aucun
/// `GROOMED-after-retry` dans les 19 journaux de verdicts encore présents), donc la fixture
/// est la même revue d'un autre plan, close par la ligne que la seconde passe déclare.
fn with_last_line(body: &str, line: &str) -> String {
    let trimmed = body.trim_end();
    let (head, last) = trimmed
        .rsplit_once('\n')
        .expect("la fixture a plusieurs lignes");
    assert_eq!(
        last.trim(),
        "Disposition: READY",
        "fixture 15ea28bd altérée"
    );
    format!("{head}\n{line}\n")
}

/// Les deux lignes-marqueurs que le shell écrit sont celles que le moteur reconnaît.
///
/// `dispatch-lib.sh` les écrit en littéral. Si elles divergent des constantes Rust,
/// `anchor_reference` rend le message entier, et la garde accepte des ancres qui ne citent
/// que le texte correctif — sans qu'aucun des deux tests ci-dessous ne rougisse.
#[test]
fn mika2996_les_marqueurs_du_shell_sont_ceux_du_moteur() {
    use mika_agent::agent_loop::review_anchor::{
        RETRY_PLAN_BEGIN, RETRY_PLAN_END, anchor_reference,
    };
    let plan = read_fixture(PLAN_FIXTURE);
    for function in ["_first_pass_retry_prompt", "_second_pass_retry_prompt"] {
        let retry = retry_message(function);
        let begin = retry.lines().filter(|l| *l == RETRY_PLAN_BEGIN).count();
        let end = retry.lines().filter(|l| *l == RETRY_PLAN_END).count();
        assert_eq!(
            (begin, end),
            (1, 1),
            "{function} doit écrire chaque marqueur sur sa propre ligne, une fois"
        );
        assert_eq!(
            anchor_reference(&retry).trim(),
            plan.trim(),
            "{function} : la région vérifiée doit être exactement le plan"
        );
    }
}

/// AC2 + AC3, première passe : relance de mika#1823.
///
/// Négatif : la réponse réelle de la trace `15ea28bd` (une revue de mika#2752) ne doit pas
/// faire avancer la chaîne. Positif, sur le même message de relance : une revue ancrée sur
/// le plan avance.
#[tokio::test]
async fn mika2996_relance_de_premiere_passe_refuse_15ea28bd_et_laisse_passer_une_revue_ancree()
-> anyhow::Result<()> {
    let skill = "mika-arch-groom-ticket";
    let retry = retry_message("_first_pass_retry_prompt");
    let fixture = read_fixture(RESPONSE_15EA28BD);
    assert_eq!(
        routed_word("_parse_disposition", &fixture),
        "READY",
        "la fixture 15ea28bd doit se lire READY sans garde, sinon elle ne rejoue pas l'incident"
    );

    // Négatif — l'incident. L'architecte s'obstine : la garde re-prompte une fois, puis
    // refuse (mika#2037, fail-closed).
    let refused = engine_turn(skill, &retry, &fixture).await?;
    let refused_word = routed_word("_parse_disposition", &refused.text);
    assert_ne!(
        refused_word, "READY",
        "mika#2996 : la revue d'un autre plan rendue à la relance UNPARSED fait avancer la \
         chaîne ({} re-prompt(s) d'ancrage — 0 veut dire que la garde ne s'est pas armée).\n\
         Réponse finale :\n{}",
        refused.anchor_reprompts, refused.text
    );
    assert_eq!(
        refused_word, "ESCALATE",
        "le refus doit être lisible comme un arrêt explicite (mika#2338)"
    );
    assert_eq!(
        refused.anchor_reprompts, 1,
        "c'est la garde d'ancrage qui a refusé, après son unique re-prompt"
    );

    // Positif — même message de relance, revue ancrée sur le plan.
    let anchored = format!("{ANCHORS_ON_PLAN}\nDisposition: READY\n");
    let accepted = engine_turn(skill, &retry, &anchored).await?;
    assert_eq!(
        routed_word("_parse_disposition", &accepted.text),
        "READY",
        "une revue ancrée sur le plan doit avancer. Réponse finale :\n{}",
        accepted.text
    );
    assert_eq!(
        accepted.anchor_reprompts, 0,
        "une revue ancrée sur le plan ne doit pas être re-promptée par la garde d'ancrage"
    );

    Ok(())
}

/// AC2, seconde passe : relance de mika#2641, `Verdict: GROOMED`.
#[tokio::test]
async fn mika2996_relance_de_seconde_passe_refuse_un_groomed_non_ancre_et_laisse_passer_une_revue_ancree()
-> anyhow::Result<()> {
    let skill = "mika-arch-second-review";
    let retry = retry_message("_second_pass_retry_prompt");
    let unanchored = with_last_line(&read_fixture(RESPONSE_15EA28BD), "Verdict: GROOMED");
    assert_eq!(
        routed_word("_parse_verdict", &unanchored),
        "GROOMED",
        "la fixture doit se lire GROOMED sans garde"
    );

    let refused = engine_turn(skill, &retry, &unanchored).await?;
    let refused_word = routed_word("_parse_verdict", &refused.text);
    assert_ne!(
        refused_word, "GROOMED",
        "mika#2996 : un GROOMED non ancré rendu à la relance de seconde passe ramène le \
         dispatch vers dev-pilot ({} re-prompt(s) d'ancrage).\nRéponse finale :\n{}",
        refused.anchor_reprompts, refused.text
    );
    assert_eq!(refused_word, "ESCALATE");
    assert_eq!(refused.anchor_reprompts, 1);

    let anchored = format!("{ANCHORS_ON_PLAN}\nVerdict: GROOMED\n");
    let accepted = engine_turn(skill, &retry, &anchored).await?;
    assert_eq!(
        routed_word("_parse_verdict", &accepted.text),
        "GROOMED",
        "une revue ancrée sur le plan doit avancer. Réponse finale :\n{}",
        accepted.text
    );
    assert_eq!(accepted.anchor_reprompts, 0);

    Ok(())
}
