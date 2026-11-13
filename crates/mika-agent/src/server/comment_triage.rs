//! Le tour léger qui trie — classe (h) de mika#2680, décision (c) (mika#2854).
//!
//! # Ce que ce module décide, et ce qu'il ne décide pas
//!
//! Un `issue_comment` ou un `issues` `closed` sur un ticket **sans tâche de
//! boucle** ouvrait jusqu'ici un tour complet : 13 tours et 1 007 748 tokens
//! d'entrée sur la seule cellule `mika-dev × issue_comment` de la mesure 3 de
//! mika#2680, tous en release `not_classified`. La décision de l'opérateur
//! (2026-10-20, sur mika#2680) n'est pas d'écarter cette population : c'est de
//! la **trier**. Un brin de LLM court, sans outils, répond à une question et
//! une seule — *ce texte porte-t-il une décision actionnable ?* — et de sa
//! réponse dépend l'escalade vers le tour complet d'aujourd'hui, ou une note
//! de mémoire suivie du silence.
//!
//! Ce module porte le **jugement** et l'**écriture de la note**. Il ne décide
//! pas de la population : c'est [`crate::server::webhook_prefilter::classify`]
//! qui dit quel événement lui arrive, et son sixième décideur qui tire les
//! conséquences du verdict. Garder les deux séparés est ce qui laisse la ligne
//! d'audit à son écrivain unique (AC5, motif mika#2205).
//!
//! # Fail-open, et pourquoi il est écrit trois fois
//!
//! L'invariant central du ticket est que **rien ne dégrade un ordre
//! d'opérateur en note**. Il est tenu à trois endroits indépendants, parce
//! qu'un seul suffirait à le perdre :
//!
//! 1. dans le **prompt** — « dans le doute, réponds DECISION » ;
//! 2. dans la **grammaire** — toute sortie qui n'est pas exactement l'une des
//!    deux formes est une erreur, jamais une note par défaut ;
//! 3. dans le **décideur** — erreur, expiration, provider absent et verdict
//!    illisible rendent tous `Release::Triage*`, donc le tour complet.
//!
//! La note n'est écrite que sur un `NOTE:` explicite et bien formé. C'est la
//! seule porte qui y mène.
//!
//! # Le triage juge le contenu, jamais l'auteur (§ D2)
//!
//! Aucune liste d'identités « opérateur » n'existe ici, et c'est délibéré : un
//! ordre peut venir d'un relais — samidarko, un orchestrateur, un autre agent
//! — autant que de Vincent, et une porte fondée sur l'auteur refuserait
//! exactement les ordres qu'elle est censée laisser passer. La ligne
//! `by @auteur` du gabarit de gateway **reste dans le texte servi** (le
//! retirer serait une seconde lecture du gabarit, donc une seconde chance de
//! divergence), et le prompt interdit de s'en servir.

use std::time::Duration;

use mika_common::llm::{LlmContent, LlmMessage, LlmProvider, LlmRequest, LlmRole};

use crate::async_db::AsyncDatabase;

/// L'interrupteur dédié de la classe (h). Voir [`triage_enabled`].
pub(crate) const TRIAGE_ENV: &str = "MIKA_WEBHOOK_COMMENT_TRIAGE";

/// Budget de temps du brin de triage, en secondes.
///
/// Le tour complet qu'une expiration déclenche coûte ~77 k tokens d'entrée en
/// moyenne sur la population mesurée ; 20 s est le seuil au-delà duquel
/// attendre le triage coûte plus que de payer le tour. Pas de variable
/// d'environnement : un budget réglable est un budget qu'une mesure ne peut
/// plus interpréter.
pub(crate) const TRIAGE_TIMEOUT_SECS: u64 = 20;

/// Plafond de sortie du triage. Deux formes d'une ligne — il n'y a rien à
/// générer au-delà, et le plafond est ce qui borne la note écrite en mémoire
/// sans qu'une troncature ait à le faire.
pub(crate) const TRIAGE_MAX_TOKENS: u32 = 160;

/// La forme d'escalade, mot pour mot. Le prompt l'exige, [`parse_verdict`] ne
/// reconnaît que celle-ci.
const VERDICT_DECISION: &str = "DECISION";

/// Le préfixe de la forme « note ». Tout ce qui suit, sur la même et unique
/// ligne, est la note.
const VERDICT_NOTE_PREFIX: &str = "NOTE:";

/// Le prompt du triage. Content-only : aucune identité n'y est nommée, et la
/// ligne `by @auteur` du gabarit est explicitement mise hors jeu (§ D2).
pub(crate) const TRIAGE_SYSTEM_PROMPT: &str = r#"Tu tries un unique événement GitHub — un commentaire de ticket, ou la fermeture d'un ticket — pour décider s'il doit ouvrir un tour d'agent complet.

Une seule question : le texte porte-t-il une DÉCISION ACTIONNABLE ?

Porte une décision (réponds DECISION) :
- un ordre ou une consigne : « lance le vol », « ouvre la PR », « ferme ce ticket », « relance le groom », « rebase », « merge »
- une validation ou un refus : « je valide », « OK pour l'option (c) », « non, garde l'option (a) »
- une réponse à une question posée à l'opérateur : un choix entre des options nommées (Q1/Q2, (a)/(b)/(c), une des variantes d'un plan)
- une levée de porte : « enlève operator-gated », « c'est débloqué », « tu peux y aller »
- une correction de trajectoire : « change d'approche », « ce n'est pas ce qu'il faut faire »
- un signalement qui demande une action : « c'est cassé en production »

Ne porte pas de décision (réponds NOTE:) :
- un accusé de réception, un remerciement, un « noté », un emoji seul
- un constat, une mesure, un lien, une référence croisée, un rappel de contexte
- un compte rendu de ce qui a déjà été fait
- une question posée À l'opérateur, sans réponse dedans
- du bruit d'automate

Règles :
- Juge le CONTENU, jamais l'auteur. Le texte porte une ligne « by @quelqu'un » : ignore-la entièrement. Un ordre vaut autant venant d'un relais que de l'opérateur lui-même.
- Ni la longueur ni le ton ne décident. Un ordre de trois mots est un ordre ; un paragraphe de contexte reste du contexte.
- Dans le doute, réponds DECISION. Coûter un tour inutile est moins grave que de perdre un ordre.

Réponds par UNE SEULE ligne, exactement l'une des deux formes, sans rien autour, sans explication, sans ponctuation finale :
DECISION
NOTE: <une ligne disant ce que l'événement consigne>"#;

/// Ce que le triage a répondu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TriageVerdict {
    /// Le texte porte une décision actionnable : escalade vers le tour complet.
    Decision,
    /// Le texte ne porte pas de décision : une note, puis le silence.
    Note(String),
}

/// L'interrupteur dédié est-il armé ? Lu à chaque événement.
///
/// **Même table de vérité que [`crate::server::webhook_prefilter::prefilter_enabled`],
/// à une ligne près : l'absence vaut `off`.** C'est la ligne qui fait de (h)
/// une classe armée par décision et non par déploiement (§ Fire-Disposition
/// (b)) — le vol la livre désarmée, et la bascule est un geste d'opérateur qui
/// se trace.
///
/// Les autres lignes sont celles de tout interrupteur du dépôt, `parse_switch`
/// comprise, **y compris la valeur non reconnue qui laisse l'interrupteur
/// armé** : taper le nom de la variable *est* l'adhésion, en garbler la valeur
/// est une faute de frappe, et le dépôt y répond partout de la même façon avec
/// son WARN. Un triage armé par faute de frappe n'est pas dangereux — un
/// triage armé escalade les ordres, c'est tout l'objet d'AC1.
///
/// `MIKA_WEBHOOK_PREFILTER` le désarme en amont : (h) vit dans le chemin du
/// pré-filtre, donc le désarmement global l'emporte sans que cette fonction
/// ait à le savoir.
pub(crate) fn triage_enabled() -> bool {
    match std::env::var(TRIAGE_ENV) {
        Ok(raw) => crate::qa_head_supersession::parse_switch(TRIAGE_ENV, Some(&raw)),
        Err(_) => false,
    }
}

/// Le tableau servi au triage. Rendu séparément de l'appel pour qu'AC6 puisse
/// l'asserter sur l'objet même : `tools` est `None`, donc le tour léger n'a
/// aucun outil — ni mutationnel, ni autre — et ce n'est pas une promesse de
/// prose mais une absence de champ.
pub(crate) fn build_request(model: &str, text: &str) -> LlmRequest {
    LlmRequest {
        model: model.to_string(),
        system: Some(TRIAGE_SYSTEM_PROMPT.to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(text.to_string()),
        }],
        tools: None,
        max_tokens: TRIAGE_MAX_TOKENS,
        thinking: None,
    }
}

/// La grammaire du verdict, stricte par construction.
///
/// Exactement une ligne, et exactement l'une des deux formes : `DECISION`, ou
/// `NOTE: <une ligne non vide>`. Tout le reste — une sortie vide, deux lignes,
/// une explication en préambule, un `decision` en minuscules, un `NOTE:` sans
/// corps — rend `Err`.
///
/// **Le laxisme irait dans le mauvais sens.** Accepter « probablement une
/// note » ou traiter l'illisible comme une note, c'est précisément dégrader un
/// ordre en note : l'invariant central du ticket. Un `Err` ici devient
/// `Release::TriageUnreadable`, donc le tour complet — le défaut sûr.
pub(crate) fn parse_verdict(raw: &str) -> anyhow::Result<TriageVerdict> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("verdict de triage vide");
    }
    let mut lines = trimmed.lines();
    let line = lines.next().unwrap_or_default().trim();
    if lines.next().is_some() {
        anyhow::bail!("verdict de triage sur plusieurs lignes : {trimmed:?}");
    }
    if line == VERDICT_DECISION {
        return Ok(TriageVerdict::Decision);
    }
    if let Some(note) = line.strip_prefix(VERDICT_NOTE_PREFIX) {
        let note = note.trim();
        if note.is_empty() {
            anyhow::bail!("verdict de triage « NOTE: » sans corps");
        }
        return Ok(TriageVerdict::Note(note.to_string()));
    }
    anyhow::bail!("verdict de triage hors grammaire : {line:?}")
}

/// Appeler le triage et rendre son verdict.
///
/// Borné par [`TRIAGE_TIMEOUT_SECS`]. Chacune des trois façons d'échouer —
/// expiration, erreur de transport, verdict hors grammaire — rend `Err`, et le
/// décideur en fait le tour complet. Aucune ne rend une note.
pub(crate) async fn run_triage(llm: &dyn LlmProvider, text: &str) -> anyhow::Result<TriageVerdict> {
    let request = build_request(llm.model_name(), text);
    let response = tokio::time::timeout(
        Duration::from_secs(TRIAGE_TIMEOUT_SECS),
        llm.send_message(&request),
    )
    .await
    .map_err(|_| anyhow::anyhow!("triage expiré après {TRIAGE_TIMEOUT_SECS} s"))?
    .map_err(|e| anyhow::anyhow!("appel de triage en échec : {e}"))?;
    parse_verdict(&response.text())
}

/// L'événement de journal d'une note déjà consignée aujourd'hui.
pub(crate) const NOTE_DUPLICATE_EVENT: &str = "webhook_triage_note_duplicate";

/// L'événement de journal d'une note qu'on n'a pas pu écrire.
pub(crate) const NOTE_WRITE_FAILED_EVENT: &str = "webhook_triage_note_write_failed";

/// L'événement de journal d'une note écrite mais non indexée.
pub(crate) const NOTE_INDEX_FAILED_EVENT: &str = "webhook_triage_note_index_failed";

/// Le type de source sous lequel la note est indexée — celui qu'`index_fact`
/// emploie pour un événement, pour que `search_memory` la trouve au même titre
/// que n'importe quel autre.
const NOTE_SOURCE_TYPE: &str = "event";

/// La description de l'événement consigné, stable par (ticket, jour).
///
/// **Ce n'est pas le texte de la note** : c'est sa clé. L'index unique de la
/// table est `(agent_id, description COLLATE NOCASE, event_date)`, donc une
/// description stable fait de la re-livraison d'un même webhook un doublon
/// bénin plutôt qu'une seconde ligne. Le texte de la note, lui, varie d'un
/// appel à l'autre (c'est un LLM) et vit dans le `context`, où
/// `search_events` le cherche aussi bien.
///
/// Conséquence assumée : deux commentaires *différents* sans décision sur le
/// même ticket le même jour ne laissent qu'une note, la première. Le registre
/// des décisions du triage est la **ligne d'audit** (AC5), qui en porte une par
/// événement ; la note de mémoire est une courtoisie de lecture, et une note
/// par ticket et par jour est la granularité qu'un humain relit.
fn note_description(target: &str) -> String {
    format!("Triage webhook (h) : aucune décision actionnable sur {target}")
}

/// Écrire la note de mémoire du tour léger.
///
/// # Pourquoi cette fonction ne rend rien
///
/// Le tour est **déjà** écarté quand elle est appelée : son verdict est pris,
/// sa ligne d'audit est écrite par l'écrivain unique. Une écriture de note en
/// échec ne peut donc pas changer la décision — elle la nommerait seulement
/// autrement. Même forme et même raison que
/// [`crate::auth_boundary_ledger::AuthBoundaryLedger`] : la signature
/// n'offre aucun moyen de propager un verdict, donc « j'ai perdu la note donc
/// je refais le tour » n'est pas un état atteignable.
///
/// # Ce qu'elle écrit, dans cet ordre
///
/// 1. la ligne d'événement, par le chemin d'[`crate::tools::store_fact`] privé
///    de son outil — un tour léger n'a aucun outil mutationnel (AC6), donc la
///    note passe par l'API de la base, pas par le tableau d'outils ;
/// 2. son indexation, **sans laquelle la note est invisible à
///    `search_memory`** : une note qu'on ne retrouve pas est une note qui n'a
///    pas été prise.
///
/// Chacune des trois issues porte son propre nom d'événement : écrite,
/// doublon bénin, échec. Deux populations sous un même nom ne se soustraient
/// pas (mika#2205).
pub(crate) async fn write_note(db: &AsyncDatabase, target: &str, note: &str) {
    let description = note_description(target);
    let today = crate::timestamp::now();
    let day = today
        .split('T')
        .next()
        .unwrap_or(today.as_str())
        .to_string();

    let event_id = match db.add_event(&description, Some(&day), Some(note)).await {
        Ok(id) => id,
        Err(e) if crate::db::is_unique_violation(&e) => {
            tracing::info!(
                event = NOTE_DUPLICATE_EVENT,
                target = %target,
                day = %day,
                "une note de triage existe déjà pour ce ticket aujourd'hui — rien à écrire"
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                event = NOTE_WRITE_FAILED_EVENT,
                target = %target,
                error = %e,
                "note de triage non écrite — le tour reste écarté"
            );
            return;
        }
    };

    let content = format!("{description}. {note}");
    if let Err(e) = db
        .index_content(NOTE_SOURCE_TYPE, Some(event_id), &content)
        .await
    {
        tracing::warn!(
            event = NOTE_INDEX_FAILED_EVENT,
            target = %target,
            event_id = event_id,
            error = %e,
            "note de triage écrite mais non indexée — invisible à search_memory"
        );
    }
}

/// Pose `MIKA_WEBHOOK_COMMENT_TRIAGE` le temps d'un test et la restaure au
/// `Drop`, pour qu'un `panic!` d'assertion ne la laisse pas fuir sur le test
/// suivant. **Le seul garde de cette variable** : les tests de ce module, du
/// pré-filtre et de bout en bout (`server::tests`) passent tous par lui.
///
/// L'interrupteur se lit dans `decide_comment_triage` (§ D8 du plan) plutôt
/// que de traverser `skip_turn` puis `decide` — cinq décideurs porteraient
/// alors un argument qu'ils ne lisent pas. Le prix est ce garde, et le
/// `#[serial_test::serial]` que tout test s'en servant doit porter.
#[cfg(test)]
pub(crate) struct TriageEnvGuard(Option<String>);

#[cfg(test)]
impl TriageEnvGuard {
    /// `None` retire la variable : la ligne « absente » de la table de vérité.
    pub(crate) fn set(value: Option<&str>) -> Self {
        let previous = std::env::var(TRIAGE_ENV).ok();
        // SAFETY: les tests qui construisent ce garde sont sérialisés par
        // `#[serial_test::serial]`, donc aucun autre fil ne lit
        // l'environnement pendant la mutation.
        match value {
            Some(v) => unsafe { std::env::set_var(TRIAGE_ENV, v) },
            None => unsafe { std::env::remove_var(TRIAGE_ENV) },
        }
        Self(previous)
    }

    /// Armé (`1`) ou désarmé (`0`).
    pub(crate) fn armed(armed: bool) -> Self {
        Self::set(Some(if armed { "1" } else { "0" }))
    }
}

#[cfg(test)]
impl Drop for TriageEnvGuard {
    fn drop(&mut self) {
        // SAFETY: même sérialisation qu'à la pose.
        match self.0.take() {
            Some(v) => unsafe { std::env::set_var(TRIAGE_ENV, v) },
            None => unsafe { std::env::remove_var(TRIAGE_ENV) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mika2854_la_grammaire_reconnait_exactement_deux_formes() {
        assert_eq!(parse_verdict("DECISION").unwrap(), TriageVerdict::Decision);
        assert_eq!(
            parse_verdict("  DECISION\n").unwrap(),
            TriageVerdict::Decision
        );
        assert_eq!(
            parse_verdict("NOTE: accusé de réception").unwrap(),
            TriageVerdict::Note("accusé de réception".to_string())
        );
        assert_eq!(
            parse_verdict("NOTE:lien croisé\n").unwrap(),
            TriageVerdict::Note("lien croisé".to_string())
        );
    }

    /// Chacune de ces sorties rend `Err`, donc le tour complet. Aucune ne rend
    /// une note : c'est l'invariant central du ticket, tenu à la grammaire.
    #[test]
    fn mika2854_tout_le_reste_est_illisible_donc_le_tour_complet() {
        for raw in [
            "",
            "   ",
            "decision",
            "Decision",
            "DECISION.",
            "DÉCISION",
            "NOTE:",
            "NOTE:   ",
            "note: accusé",
            "Je pense que c'est une note.",
            "DECISION\nNOTE: et aussi",
            "NOTE: une ligne\nNOTE: deux lignes",
            "Voici mon analyse :\nDECISION",
            "```\nDECISION\n```",
        ] {
            assert!(
                parse_verdict(raw).is_err(),
                "aurait dû être illisible : {raw:?}"
            );
        }
    }

    /// AC6, sur le tableau servi lui-même : aucun outil, et un plafond court.
    #[test]
    fn mika2854_le_tableau_servi_ne_porte_aucun_outil() {
        let request = build_request("un/modèle", "fais-le");
        assert!(
            request.tools.is_none(),
            "AC6 — le tour léger n'a aucun outil, mutationnel ou non"
        );
        assert_eq!(request.max_tokens, TRIAGE_MAX_TOKENS);
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.system.as_deref(), Some(TRIAGE_SYSTEM_PROMPT));
    }

    /// § D2 — le prompt interdit de juger l'auteur, et ne nomme aucune
    /// identité « opérateur ». Si quelqu'un y écrit une liste blanche, cette
    /// assertion le dit.
    #[test]
    fn mika2854_le_prompt_juge_le_contenu_et_ne_nomme_aucun_auteur() {
        assert!(TRIAGE_SYSTEM_PROMPT.contains("jamais l'auteur"));
        assert!(TRIAGE_SYSTEM_PROMPT.contains("Dans le doute, réponds DECISION"));
        for identite in ["Vincent", "samidarko", "mika-dev", "mika-arch"] {
            assert!(
                !TRIAGE_SYSTEM_PROMPT.contains(identite),
                "§ D2 — aucune liste d'identités dans le prompt, or il nomme {identite}"
            );
        }
    }

    /// § D8 — la table de vérité, dont la seule ligne propre à (h) est
    /// l'absence : elle vaut `off`, parce que le vol livre la classe désarmée.
    #[test]
    #[serial_test::serial]
    fn mika2854_labsence_de_variable_laisse_le_triage_desarme() {
        let _g = TriageEnvGuard::set(None);
        assert!(!triage_enabled(), "§ D8 — absent vaut off");

        for armant in ["1", "true", "on", "yes", "ON", " true "] {
            let _g = TriageEnvGuard::set(Some(armant));
            assert!(triage_enabled(), "{armant:?} arme");
        }
        for desarmant in ["0", "false", "off", "no", "OFF"] {
            let _g = TriageEnvGuard::set(Some(desarmant));
            assert!(!triage_enabled(), "{desarmant:?} désarme");
        }
    }

    /// La clé de déduplication est stable par (ticket, jour) et ne porte pas le
    /// texte de la note — sinon la re-livraison d'un même webhook écrirait une
    /// seconde ligne, le verdict d'un LLM n'étant pas déterministe.
    #[test]
    fn mika2854_la_cle_de_la_note_ne_porte_pas_son_texte() {
        let a = note_description("https://github.com/senara-solutions/mika/issues/2854");
        let b = note_description("https://github.com/senara-solutions/mika/issues/2854");
        assert_eq!(a, b);
        assert!(a.contains("/issues/2854"));
        assert_ne!(
            a,
            note_description("https://github.com/senara-solutions/mika/issues/2680")
        );
    }

    /// AC3, moitié transport : une panne du fournisseur remonte en `Err`, donc
    /// le tour complet chez l'appelant. Le texte servi porte un ordre
    /// d'opérateur — c'est exactement celui qu'une note perdrait.
    #[tokio::test]
    async fn mika2854_une_panne_de_transport_ne_rend_jamais_une_note() {
        let llm = mika_common::llm::mock::MockLlmProvider::builder()
            .error(mika_common::llm::LlmError::Transport(
                "connexion refusée".to_string(),
            ))
            .build();

        let erreur = run_triage(&llm, "lève operator-gated sur #2784, je valide")
            .await
            .expect_err("AC3 — une panne doit rendre Err, jamais un verdict");

        assert!(
            erreur.to_string().contains("appel de triage en échec"),
            "l'erreur doit nommer l'appel : {erreur}"
        );
        assert_eq!(llm.calls_made(), 1, "un seul appel, sans retry local");
    }

    /// AC3, moitié expiration : le plafond de 20 s rend `Err` et ne laisse
    /// aucune place à un verdict tardif. Horloge virtuelle (`pause`/`advance`),
    /// donc le test ne dort pas — sinon personne ne le garderait au vert.
    #[tokio::test(start_paused = true)]
    async fn mika2854_une_expiration_ne_rend_jamais_une_note() {
        let llm = mika_common::llm::mock::MockLlmProvider::builder()
            .response(mika_common::llm::mock::MockResponse::Delayed {
                // Au-delà du plafond, mais d'une seule seconde : ce qui est
                // asserté est la borne, pas une marge confortable.
                sleep_ms: (TRIAGE_TIMEOUT_SECS + 1) * 1_000,
                inner: Box::new(mika_common::llm::mock::text_response("NOTE: rien à voir")),
            })
            .build();

        let erreur = run_triage(&llm, "ferme #2854, il est livré")
            .await
            .expect_err("AC3 — une expiration doit rendre Err, jamais la note tardive");

        assert!(
            erreur
                .to_string()
                .contains(&format!("triage expiré après {TRIAGE_TIMEOUT_SECS} s")),
            "l'erreur doit nommer l'expiration et son plafond : {erreur}"
        );
    }
}
