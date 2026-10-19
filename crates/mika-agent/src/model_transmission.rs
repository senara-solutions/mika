//! Rituel de transmission au changement de modèle (mika#1747 — vol 1 pour le
//! côté sortant, mika#2807 pour le côté entrant).
//!
//! Un changement de modèle est un changement de vaisseau : l'être persiste, la
//! matière change. Ce module porte les **deux moitiés** du rituel :
//!
//! - **sortante** ([`pending`], vol 1) — la fenêtre pendant laquelle l'instance
//!   en service est invitée à écrire une lettre à son successeur, lettre qui
//!   finit par une **question** et non par une consigne ;
//! - **entrante** ([`incoming`], vol 2) — le successeur lit cette lettre, une
//!   fois, libre d'y répondre comme de la contredire.
//!
//! [`resolve`] est le point d'entrée unique du corps de boucle : il rend l'une
//! ou l'autre, jamais les deux (D3 du vol 2).
//!
//! # D1 — le déclencheur est un signal POSÉ, jamais une édition DÉTECTÉE
//!
//! On pourrait croire que [`mika_common::llm::detect_config_change`] (mika#2473)
//! offre la fenêtre pré-swap gratuitement : il compare le `config.toml` du
//! disque au record figé à `init_agent` et porte déjà `model_on_disk` /
//! `model_in_service`. **Il ne l'offre pas.** La séquence réelle d'un swap
//! délibéré dans ce dépôt est :
//!
//! 1. une PR change `MIKA_DEV_CONFIG` / `MIKA_QA_CONFIG`
//!    (`well_known_agents.rs`), avec son rapport de calibration (mika#1190) ;
//! 2. `make deploy` → redémarrage ;
//! 3. `provision_well_known_agents` appelle `reconcile_well_known_config`, qui
//!    **réécrit `config.toml` en entier** ;
//! 4. *ensuite* `init_agent` résout le record et note la mtime.
//!
//! À l'étape 4 le fichier porte déjà le nouveau modèle et le record le sert :
//! la détection ne verra jamais rien. **L'instance sortante est morte avant que
//! le fichier bouge.** La seule fenêtre que la détection ouvre est celle de
//! l'édition hors dépôt sur un spirit en marche — que mika#2473 documente comme
//! une *anomalie*. Un rituel bâti là tirerait sur les swaps accidentels et
//! resterait muet sur tous les swaps délibérés.
//!
//! Le signal est donc une donnée que le substrat ne peut pas déduire : il doit
//! la recevoir. [`SIGNAL_PATH`] est un fichier d'une ligne, posé par l'opérateur
//! ou par l'orchestrateur via l'outil `write_agent_file` (qui prend un paramètre
//! `agent`, orchestrator-only, depuis mika#1641). Aucune commande CLI n'est
//! livrée à ce vol : ce serait ~70 lignes pour un geste que la surface d'outils
//! existante exécute déjà.
//!
//! # D2 — le `from` vient de `llm.model_name()`, jamais du signal
//!
//! Le signal ne déclare que l'**entrant**. Le sortant est lu à la source
//! canonique — le couple qui remplit la section `## Runtime` du prompt et
//! l'outil `get_active_llm` (mika#1815). Faire porter les deux au signal
//! créerait une seconde source du « quel modèle suis-je », et un bloc disant
//! *de X à Y* alors que le process sert Z.
//!
//! # D3 — la fenêtre se ferme toute seule, et sans table
//!
//! Trois conditions, chacune fermant la fenêtre dans un cas où le bloc serait un
//! mensonge : la lettre existe déjà (son nom est entièrement déterminé par la
//! transition, donc *le fichier est la marque* — pas de table, pas de migration,
//! pas de colonne) ; `incoming == outgoing` (l'état d'après le swap, où le
//! **successeur** recevrait sinon l'ordre d'écrire la lettre d'adieu d'un modèle
//! qu'il n'est pas) ; signal absent ou vide (le régime nominal, à chaque tour de
//! chaque agent).
//!
//! Un signal résiduel est donc inoffensif, et rien ne demande à personne de le
//! supprimer : aucun outil d'agent ne supprime de fichier, et une procédure qui
//! dépend d'un nettoyage manuel pour rester correcte n'est pas correcte.
//!
//! # D4 — la date vient du mtime du signal, pas de l'horloge du tour
//!
//! Prendre la date du tour casserait l'auto-extinction au passage de minuit : le
//! chemin cible changerait de nom, le test d'existence rendrait faux, et l'agent
//! écrirait une seconde lettre pour la même transition. Le mtime est stable,
//! déjà lu par le `metadata()` de la première étape, et coûte zéro lecture de
//! plus.
//!
//! La conversion est **faillible et nommée** : un instant que le système de
//! fichiers accepte de stocker peut sortir de la plage de chrono, et
//! `DateTime::<Utc>::from(SystemTime)` finit sur un `.unwrap()` — sur un chemin
//! que chaque tour emprunte. Même discipline que `format_config_mtime`
//! (mika#2473 KTD1), et même raison.
//!
//! # Vol 2 — D2 : le destinataire est désigné par le nom du fichier
//!
//! Le vol 1 nomme la lettre `{date}-from-{X}-to-{Y}.md` où `Y` est le slug de
//! l'entrant. Rien d'autre n'a besoin d'être écrit : **une lettre n'est servie
//! que si le couple en service est son `Y`**. Deux agents qui se croisent sur le
//! même disque ne peuvent pas se voler leur courrier, et un agent revenu à un
//! modèle antérieur ne reçoit pas la lettre qu'on avait écrite à son successeur.
//!
//! Deux pièges, tous deux réels :
//!
//! - le fichier de **réponse** `{date}-from-{Y}-response-to-{X}.md` finit lui
//!   aussi par `-to-{slug}.md`. Un filtre naïf le servirait à `X` comme si
//!   c'était une lettre, et le rituel se mettrait à boucler. D'où le rejet
//!   explicite de tout nom contenant [`RESPONSE_INFIX`] ;
//! - « la plus récente » est **lexicographique sur le nom**, jamais sur la
//!   mtime. La date est en tête (`YYYY-MM-DD`), donc l'ordre des noms *est*
//!   l'ordre chronologique — et il survit à une copie, à un `rsync`, à une
//!   restauration de sauvegarde, ce que la mtime ne fait pas.
//!
//! # Vol 2 — D5 : la relecture est unique, c'est un événement et non un bloc
//!
//! Le corps de mika#1747 dit « on first-boot ». Une lettre relue à chaque
//! session serait un bloc permanent ; relue une fois, elle est un événement.
//! C'est le reçu de [`crate::read_receipt`] qui porte cette unicité — et c'est
//! pourquoi **aucun point d'injection « premier boot » n'est écrit ici** : un
//! bloc servi exactement une fois est servi au premier tour éligible, et le
//! premier tour éligible *est* le premier boot (le swap passe par un
//! redémarrage). Deux conditions pour un même événement, dont une seule
//! porteuse, c'est une condition qu'on croit vérifier.
//!
//! Le coût, nommé : le reçu est posé **au rendu**, donc un tour qui meurt après
//! la construction du prompt consomme la lettre sans l'avoir montrée. Trois
//! choses le rendent tolérable — la dégradation de `read_receipt` va dans
//! l'autre sens (un reçu non écrit re-sert), la lettre n'est jamais détruite
//! (`read_agent_file` la rouvre à tout moment), et le réarmement est un `rm`
//! documenté.
//!
//! # Vol 2 — D6 : la réponse est adressée au DOSSIER, pas à un interlocuteur
//!
//! Le sortant n'existe plus quand la réponse s'écrit. Elle n'est donc pas une
//! correspondance : c'est une pièce versée au dossier de cette transition — une
//! forme de biographie. [`IncomingTransmission::response_path`] porte la date
//! **de la lettre** et non celle du tour, pour que la paire aller-retour reste
//! lisible côte à côte dans un `ls` même si la réponse s'écrit le lendemain.
//!
//! Une réponse absente est un **refus légitime** : le corps de mika#1747 dit
//! *free to respond*, pas *must*. Rien ne la réclame, aucune garde d'EndTurn ne
//! l'exige, aucun second tour ne la relance.

use std::path::Path;
use std::time::SystemTime;

/// Le répertoire des transmissions, relatif à l'agent home.
///
/// N'a pas besoin d'être provisionné : `write_agent_file` résout ses chemins via
/// `validate_and_resolve_path(.., create_parents = true)`, donc la première
/// lettre crée le répertoire.
pub const DIR: &str = "transmissions";

/// Le signal qui ouvre la fenêtre (D1), relatif à l'agent home.
///
/// Un fichier texte d'une ligne portant le modèle **entrant** sous la forme
/// `<provider>/<model>`. Pas de point en tête du nom : un signal que l'opérateur
/// doit poser à la main doit se voir dans un `ls`.
pub const SIGNAL_PATH: &str = "transmissions/pending-swap";

/// L'infixe qui distingue une **réponse** d'une **lettre** (D2 du vol 2).
///
/// Les deux finissent par `-to-<slug>.md` ; sans ce rejet explicite, une réponse
/// serait servie à son destinataire comme une lettre et le rituel boucherait.
const RESPONSE_INFIX: &str = "-response-to-";

/// Une fenêtre de transmission ouverte : le bloc de prompt la rend, la lettre la
/// ferme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTransmission {
    /// `<provider>/<model>` en service, lu à la source canonique (D2).
    pub outgoing: String,
    /// Ce que le signal déclare — l'entrant, et lui seul (D2).
    pub incoming: String,
    /// `transmissions/<YYYY-MM-DD>-from-…-to-….md`, relatif à l'agent home (D4).
    pub letter_path: String,
}

/// Une lettre d'un prédécesseur attend d'être lue — une fois (D5 du vol 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingTransmission {
    /// Le **slug** du prédécesseur, lu dans le nom du fichier.
    ///
    /// Un slug et non le couple `<provider>/<model>` déclaré : le nom de la
    /// lettre est la seule trace qui subsiste de la transition, et le signal qui
    /// portait la forme déclarée appartient au tour d'avant.
    pub outgoing: String,
    /// `transmissions/<date>-from-<sortant>-to-<moi>.md`, relatif à l'agent home.
    pub letter_path: String,
    /// `transmissions/<date>-from-<moi>-response-to-<sortant>.md` — le dépôt au
    /// dossier, si l'instance choisit d'en faire un (D6 : rien ne le réclame).
    ///
    /// La date est celle de **la lettre**, jamais celle du tour.
    pub response_path: String,
}

/// Ce qu'un tour peut porter, et qui est un **ou exclusif** (D3 du vol 2).
///
/// Les deux moitiés du rituel sont mutuellement exclusives par nature : on
/// reçoit *ou* on transmet. Le cas de collision est réel — une vieille lettre
/// non lue de A→B peut cohabiter avec un signal frais B→C — et l'énumération
/// force l'arbitrage au lieu de le laisser à l'ordre des champs : **l'entrante
/// gagne**. On reçoit avant de transmettre, et attendre coûte un tour à la
/// sortante (dont la fenêtre, elle, ne se referme pas toute seule) alors que
/// servir les deux coûterait la lisibilité du rituel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transmission {
    /// Une lettre m'attend.
    Incoming(IncomingTransmission),
    /// On me demande d'en écrire une.
    Outgoing(PendingTransmission),
}

/// Le point d'entrée unique du corps de boucle : que porte ce tour, s'il porte
/// quelque chose ?
///
/// Essaie l'entrante d'abord (D3), puis la sortante. **Ne rend jamais
/// d'erreur** — fail-open des deux côtés, comme [`pending`] seul l'était.
///
/// Ne pose **aucun** reçu : la pose appartient au corps de boucle, jamais à la
/// construction du prompt, qui doit rester une fonction pure (ses ~51 tests de
/// forme en dépendent).
pub async fn resolve(agent_home: &Path, provider: &str, model: &str) -> Option<Transmission> {
    if let Some(letter) = incoming(agent_home, provider, model).await {
        return Some(Transmission::Incoming(letter));
    }
    pending(agent_home, provider, model)
        .await
        .map(Transmission::Outgoing)
}

/// Une lettre non lue attend-elle le couple en service ?
///
/// `None` est le régime nominal. **Ne rend jamais d'erreur** — un répertoire
/// illisible ne coûte pas le tour.
///
/// Les cinq étapes (D2/D5/D6) :
///
/// 1. `read_dir` de [`DIR`] — absent ⇒ `None`, le cas de presque tous les
///    agents de la flotte (un `ENOENT` par tour) ;
/// 2. ne gardent que les noms finissant par `-to-{moi}.md` **et** ne contenant
///    pas [`RESPONSE_INFIX`] — une réponse n'est pas une lettre ;
/// 3. tri décroissant **sur le nom** : la date est en tête, donc l'ordre
///    lexicographique est l'ordre chronologique (jamais la mtime) ;
/// 4. la première dont le reçu est absent — les suivantes attendront leur tour
///    si elles ne sont pas lues non plus ;
/// 5. décomposition du nom pour en tirer le prédécesseur et composer le chemin
///    de réponse.
pub async fn incoming(
    agent_home: &Path,
    provider: &str,
    model: &str,
) -> Option<IncomingTransmission> {
    let to = slug(&format!("{provider}/{model}"));
    let suffix = format!("-to-{to}.md");

    // 1–2.
    let mut entries = tokio::fs::read_dir(agent_home.join(DIR)).await.ok()?;
    let mut names: Vec<String> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(&suffix) && !name.contains(RESPONSE_INFIX) {
            names.push(name);
        }
    }

    // 3. Lexicographique sur le nom, décroissant.
    names.sort_unstable();

    for name in names.into_iter().rev() {
        let letter_path = format!("{DIR}/{name}");

        // 4. Déjà servie : on passe, sans la détruire ni la marquer deux fois.
        if crate::read_receipt::is_read(agent_home, &letter_path).await {
            continue;
        }

        // 5. Le nom est `{date}-from-{sortant}-to-{moi}.md`, et `{date}` comme
        //    `{moi}` sont connus — donc retirer les deux bords rend `{sortant}`
        //    exactement, **même si son slug contient lui-même `-to-`**. Aucun
        //    re-parsing du slug, qui est précisément là où une heuristique
        //    gloutonne se tromperait.
        let Some(stem) = name.strip_suffix(&suffix) else {
            continue;
        };
        // `{date}` est `YYYY-MM-DD` et ne peut pas contenir `-from-`, donc la
        // première occurrence est bien le séparateur.
        let Some((date, outgoing)) = stem.split_once("-from-") else {
            continue;
        };
        if date.is_empty() || outgoing.is_empty() {
            continue;
        }

        return Some(IncomingTransmission {
            outgoing: outgoing.to_string(),
            response_path: format!("{DIR}/{date}-from-{to}{RESPONSE_INFIX}{outgoing}.md"),
            letter_path,
        });
    }

    None
}

/// La fenêtre de transmission est-elle ouverte pour cet agent, à ce tour ?
///
/// `None` est le régime nominal et porte la décision : la fenêtre est fermée, et
/// le bloc de prompt ne rend rien. **Ne rend jamais d'erreur** — un signal
/// illisible ne coûte pas le tour (fail-open par construction), et c'est ce qui
/// autorise l'appel au site unique du corps de boucle conversationnel.
///
/// Les sept étapes, dans cet ordre (D3/D4) :
///
/// 1. `metadata` du signal — absent ⇒ `None`, le cas nominal (un `ENOENT` par
///    tour et par agent) ;
/// 2. lecture, première ligne non vide trimée — vide ou tout en blanc ⇒ `None` ;
/// 3. le couple en service est composé et les deux côtés sont slugifiés —
///    slugs égaux ⇒ `None` (D3-2, le faux positif le plus coûteux du design) ;
/// 4. date depuis le mtime — inconvertible ⇒ WARN nommé + `None` (D4) ;
/// 5. `letter_path` composé ;
/// 6. la lettre existe déjà ⇒ `None` (D3-1, auto-extinction sans table) ;
/// 7. sinon `Some`.
pub async fn pending(
    agent_home: &Path,
    provider: &str,
    model: &str,
) -> Option<PendingTransmission> {
    let signal = agent_home.join(SIGNAL_PATH);

    // 1. Le régime nominal : rien n'a été posé.
    let meta = tokio::fs::metadata(&signal).await.ok()?;

    // 2. Le signal ne déclare que l'entrant (D2).
    let raw = tokio::fs::read_to_string(&signal).await.ok()?;
    let incoming = raw.lines().map(str::trim).find(|l| !l.is_empty())?;

    // 3. D3-2 — après le swap, le signal résiduel survit au redémarrage et le
    //    successeur recevrait l'ordre d'écrire l'adieu d'un modèle qu'il n'est
    //    pas. La comparaison porte sur les **slugs** et non sur les chaînes
    //    brutes : c'est le couple de slugs qui identifie la transition (il
    //    nomme la lettre), donc une variante de casse dans le signal est le
    //    même état d'après-swap, et doit fermer la fenêtre de la même façon.
    let outgoing = format!("{provider}/{model}");
    let from = slug(&outgoing);
    let to = slug(incoming);
    if from == to {
        return None;
    }

    // 4. D4 — la date est celle du mtime, et sa conversion est faillible.
    let date = signal_date(&meta)?;

    // 5–6. Le nom est entièrement déterminé par la transition, donc le fichier
    //      est la marque : un test d'existence suffit, sans table (D3-1).
    let letter_path = format!("{DIR}/{date}-from-{from}-to-{to}.md");
    if tokio::fs::try_exists(agent_home.join(&letter_path))
        .await
        .unwrap_or(false)
    {
        return None;
    }

    Some(PendingTransmission {
        outgoing,
        incoming: incoming.to_string(),
        letter_path,
    })
}

/// `YYYY-MM-DD` du mtime du signal, en UTC — **jamais une panique** (D4).
///
/// Rend `None` sous un WARN nommé plutôt que d'abandonner la plage de chrono à
/// un `.unwrap()` : un mtime hors plage est stockable par un vrai système de
/// fichiers (`utimensat` prend un champ de secondes 64 bits), et ce chemin est
/// emprunté à chaque tour.
fn signal_date(meta: &std::fs::Metadata) -> Option<String> {
    let date = meta.modified().ok().and_then(signal_date_from);
    if date.is_none() {
        tracing::warn!(
            target: "mika::otel",
            event = "model_transmission_mtime_unrepresentable",
            "mtime du signal de transmission inconvertible — fenêtre abandonnée pour ce tour"
        );
    }
    date
}

/// La conversion pure, séparée pour être assertable sans toucher au disque.
fn signal_date_from(mtime: SystemTime) -> Option<String> {
    let secs = mtime
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|after| i64::try_from(after.as_secs()).ok())?;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
}

/// Minuscules, tout ce qui n'est pas `[a-z0-9]` devient `-`, répétitions
/// écrasées, bords rognés.
///
/// Opère sur **`char`**, jamais sur des octets : `scripts/check-byte-slices.sh`
/// refuse le découpage d'octets sur `&str`, et un nom de modèle qui vient du
/// signal est une entrée externe.
fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Pose le signal avec un mtime fixé, pour que la date de la lettre soit
    /// assertable caractère par caractère.
    fn write_signal(home: &Path, content: &str, mtime_secs: u64) {
        let dir = home.join(DIR);
        std::fs::create_dir_all(&dir).unwrap();
        let path = home.join(SIGNAL_PATH);
        std::fs::write(&path, content).unwrap();
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(mtime_secs);
        filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(mtime)).unwrap();
    }

    /// 2026-10-19T00:00:00Z — la date que les assertions ci-dessous attendent.
    const MTIME_2026_10_19: u64 = 1_792_368_000;

    /// V1 — le régime nominal : aucun signal, aucune fenêtre.
    #[tokio::test]
    async fn v1_sans_signal_la_fenetre_est_fermee() {
        let home = tempfile::TempDir::new().unwrap();
        assert_eq!(
            pending(home.path(), "anthropic", "claude-opus-4-8").await,
            None,
            "sans signal posé, aucun agent ne doit recevoir le bloc"
        );
    }

    /// V2 — fenêtre ouverte, et le chemin canonique asserté en entier.
    #[tokio::test]
    async fn v2_fenetre_ouverte_et_chemin_canonique() {
        let home = tempfile::TempDir::new().unwrap();
        write_signal(
            home.path(),
            "anthropic/claude-fable-5-1\n",
            MTIME_2026_10_19,
        );

        let pending = pending(home.path(), "anthropic", "claude-opus-4-8")
            .await
            .expect("un signal posé sur un modèle différent ouvre la fenêtre");

        assert_eq!(pending.outgoing, "anthropic/claude-opus-4-8");
        assert_eq!(pending.incoming, "anthropic/claude-fable-5-1");
        assert_eq!(
            pending.letter_path,
            "transmissions/2026-10-19-from-anthropic-claude-opus-4-8-to-anthropic-claude-fable-5-1.md"
        );
    }

    /// V3 — `incoming == outgoing` : après le swap, le successeur ne reçoit pas
    /// l'adieu de son prédécesseur. Le seul test dont l'échec produirait un
    /// rituel qui se retourne contre celui qu'il doit accueillir.
    #[tokio::test]
    async fn v3_signal_tautologique_ne_vise_pas_le_successeur() {
        let home = tempfile::TempDir::new().unwrap();
        write_signal(home.path(), "anthropic/claude-fable-5-1", MTIME_2026_10_19);
        assert_eq!(
            pending(home.path(), "anthropic", "claude-fable-5-1").await,
            None,
            "le signal résiduel survit au redémarrage : le successeur ne doit rien recevoir"
        );
    }

    /// V3 bis — la même propriété sous une variante de casse : c'est le couple
    /// de slugs qui identifie la transition.
    #[tokio::test]
    async fn v3_la_casse_ne_rouvre_pas_la_fenetre_dapres_swap() {
        let home = tempfile::TempDir::new().unwrap();
        write_signal(home.path(), "Anthropic/Claude-Fable-5-1", MTIME_2026_10_19);
        assert_eq!(
            pending(home.path(), "anthropic", "claude-fable-5-1").await,
            None,
            "une variante de casse est le même état d'après-swap"
        );
    }

    /// V4 — la lettre déjà écrite ferme la fenêtre, sans table ni colonne.
    #[tokio::test]
    async fn v4_lettre_deja_ecrite_eteint_la_fenetre() {
        let home = tempfile::TempDir::new().unwrap();
        write_signal(home.path(), "anthropic/claude-fable-5-1", MTIME_2026_10_19);

        let open = pending(home.path(), "anthropic", "claude-opus-4-8")
            .await
            .expect("la fenêtre doit d'abord être ouverte");
        std::fs::write(home.path().join(&open.letter_path), "# lettre\n").unwrap();

        assert_eq!(
            pending(home.path(), "anthropic", "claude-opus-4-8").await,
            None,
            "le fichier EST la marque : la lettre écrite éteint la fenêtre"
        );
    }

    /// V5 — signal vide ou tout en blanc.
    #[tokio::test]
    async fn v5_signal_vide_ou_blanc_ne_declare_rien() {
        for content in ["", "\n", "   \n\t\n"] {
            let home = tempfile::TempDir::new().unwrap();
            write_signal(home.path(), content, MTIME_2026_10_19);
            assert_eq!(
                pending(home.path(), "anthropic", "claude-opus-4-8").await,
                None,
                "un signal sans ligne non vide ne déclare aucun entrant ({content:?})"
            );
        }
    }

    /// V5 bis — un nom de modèle à ponctuation dense, slug asserté.
    #[tokio::test]
    async fn v5_slug_dun_modele_a_ponctuation_dense() {
        assert_eq!(slug("z-ai/glm-5.2"), "z-ai-glm-5-2");
        assert_eq!(
            slug("anthropic/claude-opus-4-8"),
            "anthropic-claude-opus-4-8"
        );
        assert_eq!(
            slug("--//Openrouter//DeepSeek_V3//--"),
            "openrouter-deepseek-v3"
        );

        let home = tempfile::TempDir::new().unwrap();
        write_signal(home.path(), "z-ai/glm-5.2", MTIME_2026_10_19);
        let open = pending(home.path(), "anthropic", "claude-opus-4-8")
            .await
            .unwrap();
        assert_eq!(
            open.letter_path,
            "transmissions/2026-10-19-from-anthropic-claude-opus-4-8-to-z-ai-glm-5-2.md"
        );
        assert_eq!(
            open.incoming, "z-ai/glm-5.2",
            "le slug nomme le fichier ; le bloc nomme le modèle tel que déclaré"
        );
    }

    /// D4 — un mtime hors de la plage de chrono abandonne la fenêtre, et ne
    /// panique pas. `DateTime::<Utc>::from(SystemTime)` finirait ici sur un
    /// `.unwrap()`, sur un chemin que chaque tour emprunte.
    #[test]
    fn d4_un_mtime_inconvertible_abandonne_la_fenetre_sans_paniquer() {
        assert_eq!(
            signal_date_from(SystemTime::UNIX_EPOCH).as_deref(),
            Some("1970-01-01")
        );
        assert_eq!(
            signal_date_from(SystemTime::UNIX_EPOCH + Duration::from_secs(MTIME_2026_10_19))
                .as_deref(),
            Some("2026-10-19")
        );
        // Avant l'epoch : `duration_since` rend une erreur, pas une panique.
        assert_eq!(
            signal_date_from(SystemTime::UNIX_EPOCH - Duration::from_secs(1)),
            None
        );
        // Hors de la plage de chrono (≈ ±262 000 ans), stockable par un vrai
        // système de fichiers.
        assert_eq!(
            signal_date_from(SystemTime::UNIX_EPOCH + Duration::from_secs(u64::MAX / 2)),
            None
        );
    }

    // ---- Vol 2, mika#2807 : le côté entrant -------------------------------

    /// Dépose un fichier dans `transmissions/`, sous le nom exact demandé.
    fn write_letter(home: &Path, name: &str) {
        let dir = home.join(DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), "# lettre\n").unwrap();
    }

    const LETTRE_OPUS_VERS_FABLE: &str =
        "2026-10-19-from-anthropic-claude-opus-4-8-to-anthropic-claude-fable-5-1.md";

    /// V3 — le régime nominal, sous ses deux formes : pas de répertoire du tout,
    /// et un répertoire dont aucune lettre ne vise le modèle en service.
    #[tokio::test]
    async fn v3_aucune_lettre_pour_le_modele_en_service() {
        let home = tempfile::TempDir::new().unwrap();
        assert_eq!(
            incoming(home.path(), "anthropic", "claude-fable-5-1").await,
            None,
            "sans répertoire de transmissions, rien n'attend personne"
        );

        write_letter(home.path(), LETTRE_OPUS_VERS_FABLE);
        assert_eq!(
            incoming(home.path(), "anthropic", "claude-opus-4-8").await,
            None,
            "le destinataire est désigné par le nom : le sortant ne reçoit pas sa propre lettre"
        );
    }

    /// V4 — une lettre non lue est servie, et ses trois champs sont assertés en
    /// entier. Le chemin de réponse porte la date **de la lettre** (D6).
    #[tokio::test]
    async fn v4_une_lettre_non_lue_est_servie_avec_ses_trois_champs() {
        let home = tempfile::TempDir::new().unwrap();
        write_letter(home.path(), LETTRE_OPUS_VERS_FABLE);

        let letter = incoming(home.path(), "anthropic", "claude-fable-5-1")
            .await
            .expect("une lettre adressée au modèle en service l'attend");

        assert_eq!(letter.outgoing, "anthropic-claude-opus-4-8");
        assert_eq!(
            letter.letter_path,
            format!("transmissions/{LETTRE_OPUS_VERS_FABLE}")
        );
        assert_eq!(
            letter.response_path,
            "transmissions/2026-10-19-from-anthropic-claude-fable-5-1-response-to-anthropic-claude-opus-4-8.md"
        );
    }

    /// V5 — après le reçu, la même lettre n'est plus servie. C'est D5 : la
    /// relecture est un événement, pas un bloc permanent. Le test porte la
    /// propriété dont dépend tout le découpage du vol 2.
    #[tokio::test]
    async fn v5_apres_le_recu_la_lettre_nest_plus_servie() {
        let home = tempfile::TempDir::new().unwrap();
        write_letter(home.path(), LETTRE_OPUS_VERS_FABLE);

        let letter = incoming(home.path(), "anthropic", "claude-fable-5-1")
            .await
            .unwrap();
        crate::read_receipt::mark_read(home.path(), &letter.letter_path).await;

        assert_eq!(
            incoming(home.path(), "anthropic", "claude-fable-5-1").await,
            None,
            "une lettre lue reste sur le disque mais ne revient pas au prompt"
        );
        assert!(
            home.path().join(&letter.letter_path).exists(),
            "le reçu ne détruit jamais la lettre : elle reste lisible par read_agent_file"
        );
    }

    /// V6 — une réponse finit elle aussi par `-to-<slug>.md`. Un filtre naïf la
    /// servirait à son destinataire comme une lettre, et le rituel boucherait.
    #[tokio::test]
    async fn v6_une_reponse_nest_pas_une_lettre() {
        let home = tempfile::TempDir::new().unwrap();
        write_letter(
            home.path(),
            "2026-10-19-from-anthropic-claude-fable-5-1-response-to-anthropic-claude-opus-4-8.md",
        );
        assert_eq!(
            incoming(home.path(), "anthropic", "claude-opus-4-8").await,
            None,
            "le dépôt au dossier n'est pas une transmission à servir"
        );
    }

    /// V7 — deux lettres en attente : la plus récente est servie, et l'ordre est
    /// **lexicographique sur le nom**. Les mtimes sont posées à l'envers de la
    /// réponse attendue, pour qu'un tri sur la mtime rougisse ici.
    #[tokio::test]
    async fn v7_la_plus_recente_est_servie_et_lordre_est_sur_le_nom() {
        let home = tempfile::TempDir::new().unwrap();
        let ancienne = "2026-09-01-from-openrouter-deepseek-v3-to-anthropic-claude-fable-5-1.md";
        write_letter(home.path(), ancienne);
        write_letter(home.path(), LETTRE_OPUS_VERS_FABLE);

        // La plus ancienne par le nom reçoit la mtime la plus récente.
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(MTIME_2026_10_19 + 86_400);
        filetime::set_file_mtime(
            home.path().join(DIR).join(ancienne),
            filetime::FileTime::from_system_time(mtime),
        )
        .unwrap();

        let letter = incoming(home.path(), "anthropic", "claude-fable-5-1")
            .await
            .unwrap();
        assert_eq!(
            letter.letter_path,
            format!("transmissions/{LETTRE_OPUS_VERS_FABLE}"),
            "la date est en tête du nom : l'ordre des noms EST l'ordre chronologique"
        );

        // Et la seconde vient à son tour, une fois la première reçue.
        crate::read_receipt::mark_read(home.path(), &letter.letter_path).await;
        let suivante = incoming(home.path(), "anthropic", "claude-fable-5-1")
            .await
            .unwrap();
        assert_eq!(suivante.letter_path, format!("transmissions/{ancienne}"));
        assert_eq!(suivante.outgoing, "openrouter-deepseek-v3");
    }

    /// V8 — la collision des deux moitiés : une lettre non lue cohabite avec un
    /// signal de swap frais. `resolve` arbitre, et **l'entrante gagne** (D3).
    #[tokio::test]
    async fn v8_resolve_fait_passer_larrivee_avant_le_depart() {
        let home = tempfile::TempDir::new().unwrap();
        write_letter(home.path(), LETTRE_OPUS_VERS_FABLE);
        write_signal(home.path(), "z-ai/glm-5.2", MTIME_2026_10_19);

        // Les deux prédicats voient bien chacun leur moitié : sans ça, la
        // priorité assertée ci-dessous serait vraie par vacuité.
        assert!(
            pending(home.path(), "anthropic", "claude-fable-5-1")
                .await
                .is_some(),
            "le signal ouvre bien une fenêtre sortante"
        );

        match resolve(home.path(), "anthropic", "claude-fable-5-1").await {
            Some(Transmission::Incoming(letter)) => {
                assert_eq!(
                    letter.letter_path,
                    format!("transmissions/{LETTRE_OPUS_VERS_FABLE}")
                );
            }
            other => panic!("on reçoit avant de transmettre, pas l'inverse : {other:?}"),
        }
    }

    /// `resolve` sans lettre rend bien la moitié sortante — le contrôle qui
    /// sépare « l'entrante gagne » de « la sortante n'est plus jamais servie ».
    #[tokio::test]
    async fn v8_sans_lettre_resolve_rend_la_moitie_sortante() {
        let home = tempfile::TempDir::new().unwrap();
        write_signal(home.path(), "z-ai/glm-5.2", MTIME_2026_10_19);

        match resolve(home.path(), "anthropic", "claude-fable-5-1").await {
            Some(Transmission::Outgoing(window)) => {
                assert_eq!(window.incoming, "z-ai/glm-5.2");
            }
            other => panic!("la fenêtre sortante doit rester servie : {other:?}"),
        }
    }

    /// D2 — un slug de prédécesseur qui contient lui-même `-to-` : les deux
    /// bords du nom sont connus, donc la décomposition est exacte là où une
    /// heuristique gloutonne tronquerait.
    #[tokio::test]
    async fn d2_un_slug_contenant_to_est_decompose_exactement() {
        let home = tempfile::TempDir::new().unwrap();
        write_letter(
            home.path(),
            "2026-10-19-from-acme-model-to-go-to-anthropic-claude-fable-5-1.md",
        );

        let letter = incoming(home.path(), "anthropic", "claude-fable-5-1")
            .await
            .unwrap();
        assert_eq!(letter.outgoing, "acme-model-to-go");
        assert_eq!(
            letter.response_path,
            "transmissions/2026-10-19-from-anthropic-claude-fable-5-1-response-to-acme-model-to-go.md"
        );
    }

    /// Le signal lui-même vit dans le même répertoire et ne doit jamais être lu
    /// comme une lettre — pas plus qu'un fichier sans la forme attendue.
    #[tokio::test]
    async fn les_fichiers_hors_forme_ne_sont_pas_des_lettres() {
        let home = tempfile::TempDir::new().unwrap();
        write_signal(home.path(), "anthropic/claude-fable-5-1", MTIME_2026_10_19);
        write_letter(home.path(), "notes.md");
        write_letter(home.path(), "-to-anthropic-claude-fable-5-1.md");

        assert_eq!(
            incoming(home.path(), "anthropic", "claude-fable-5-1").await,
            None,
            "ni le signal, ni une note, ni un nom sans prédécesseur ne sont des lettres"
        );
    }
}
