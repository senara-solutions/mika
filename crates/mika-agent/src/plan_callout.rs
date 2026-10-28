//! Le seul lecteur du callout `Plan` sous sa forme stricte (mika#2194, phase 1).
//!
//! # La décision, écrite ici pour que la prochaine divergence soit une régression
//!
//! **Ce module est la source de vérité du chemin porté par le callout `Plan`.**
//! [`crate::auto_pull`] et `dispatch-lib.sh` l'appellent et n'en portent **pas**
//! de copie. Un motif du callout écrit ailleurs qu'ici fait échouer
//! [`tests::mika2194_aucun_motif_de_callout_hors_de_ce_module`] côté Rust, et
//! son pendant shell dans `test-dispatch-lib.sh` — délibérément.
//!
//! La garde existe parce que la duplication était **mesurée**, pas redoutée :
//! le même jeton était lu par deux motifs, dans deux langues, qui n'avaient
//! jamais été exécutés sur la même entrée. Précédent direct et même remède :
//! [`crate::grooming_marker`] (mika#2158), dont la regex copiée portait le
//! commentaire « Mirrors … » et a ensuite raté deux élargissements.
//!
//! # Deux formes, parce que les deux appelants ne posent pas la même question
//!
//! - `auto_pull::plan_ownership` demande **à qui appartient ce plan**, donc il
//!   veut le chemin tel qu'écrit, préfixe de dépôt compris : c'est
//!   [`PlanCallout::raw`].
//! - `dispatch-lib` demande **quel fichier ouvrir**, et résout ensuite
//!   `"$WORKTREE_DIR/$PLAN_PATH"` où `$WORKTREE_DIR` est déjà la racine du
//!   sous-dépôt : `mika/docs/plans/x.md` y désignerait `…/mika/mika/docs/…`,
//!   qui n'existe pas. C'est [`PlanCallout::normalized`].
//!
//! Un lecteur qui n'en rendrait qu'une casserait un des deux appelants. Rendre
//! les deux est ce qui permet d'unifier l'**implémentation** sans toucher à la
//! **tolérance** — voir la borne B1 ci-dessous.
//!
//! # Ce que ce module ne fait PAS, et c'est une borne écrite
//!
//! Il ne resserre ni n'élargit aucun prédicat. En particulier il **ne touche
//! pas** `skills::executor::check_grooming_markers`, qui se contente de la
//! sous-chaîne `docs/plans/` non ancrée. Le doc-comment de
//! `auto_pull::is_groomed` l'interdit en toutes lettres :
//!
//! > **Ne le resserrez pas pour « harmoniser » les deux** — ce sens-là de
//! > l'alignement recréerait le défaut symétrique de celui que ce ticket ferme.
//!
//! Il y a **quatre** tolérances sur ce jeton, délibérément inégales — et non
//! trois, comme le plan de la phase 1 le disait : la quatrième a été trouvée en
//! implémentant, et **tranchée par mika#2608**.
//!
//! | lecteur | tolérance | question | statut |
//! |---|---|---|---|
//! | ce module | strict, ancré, fences au choix de l'appelant | quel chemin, sous quelle forme ? | **le lecteur unique** |
//! | `executor::check_grooming_markers` | sous-chaîne `docs/plans/`, non ancrée | ce corps a-t-il un plan ? (routage) | intouché — resserrement **interdit** par le doc-comment de `auto_pull::is_groomed` |
//! | `dispatch-lib::{_committed_plan_on_branch, _set_up_worktree}` | — | « quel chemin ce corps nomme-t-il ? » et « en porte-t-il un ? » | **délègue au lecteur unique** (phase 2, mika#2608) |
//! | `milestone_manager::reader::plan_callout_present` | `contains("**Plan:**")` **sans** le préfixe `> - `, plus une branche sur `docs/plans/` en prose | ce corps porte-t-il un callout ? (booléen, LECTURE seule) | tolérance **conservée, décidée** par mika#2608 |
//!
//! Ce module unifie l'**implémentation** de **quatre** lecteurs : `auto_pull`
//! (phase 1), `dispatch-lib::_extract_plan_path` (phase 1), puis
//! `dispatch-lib::_committed_plan_on_branch` et le `elif` de
//! `dispatch-lib::_set_up_worktree` (phase 2). Aucun d'eux n'en garde de copie,
//! et l'inventaire bash est désormais un **zéro** que `test-dispatch-lib.sh`
//! tient — le scan de la phase 1 comptait deux et disait en commentaire pourquoi
//! un zéro aurait été « rouge à la naissance ».
//!
//! Il n'unifie **aucune tolérance**, et un maillon futur qui écraserait les
//! quatre vers la plus stricte fermerait un défaut en ouvrant son miroir. La
//! quatrième ligne est là pour être lue avant d'essayer : la raison de la
//! conserver est **structurelle**, pas prudentielle — sa première branche est un
//! sur-ensemble strict de ce module, donc un appel strict-first y serait
//! *prouvablement inerte*, ce qui est la classe mika#2205 appliquée à un chemin
//! de code. La propriété est épinglée par
//! `milestone_manager::reader::tests::mika2608_la_branche_lache_est_un_surensemble_du_lecteur_strict`
//! sur le corpus doré, et la décision par son voisin
//! `…::mika2608_la_tolerance_lache_est_une_decision_epinglee`.
//!
//! # Trois deltas de tolérance que la phase 2 a produits, et leur direction
//!
//! Le `sed` de `_committed_plan_on_branch` était décrit comme « strict, ancré »
//! et ne l'était pas : il n'exigeait **pas** le littéral `docs/plans/`, tolérait
//! zéro espace ou plus après `**Plan:**`, et acceptait `../docs/plans/`. Les
//! trois sont des **resserrements**, et pour cette porte leur direction est
//! fail-open — un callout refusé fait que la porte ne tire pas, donc que le
//! grooming procède.
//!
//! Le premier ferme un **faux positif latent** : un corps portant
//! ``> - **Plan:** `README.md` `` en extrayait `README.md`, `cat-file -t` rendait
//! `blob` (tout dépôt a un README), la liaison mika#2034 ne réfutait rien — son
//! contrat est la réfutation — et la porte **tirait**, bloquant le ticket en
//! `already_groomed` de façon permanente. Corpus : `gate-non-plan-path.md`,
//! `gate-double-space.md`, `gate-foreign-prefix-resolves.md`.
//!
//! # Les trois divergences entre les anciens lecteurs, mesurées
//!
//! Deux n'étaient nommées ni dans le ticket ni dans le plan. La troisième est
//! documentée au site du canal (`crates/mika-cli/src/commands/plan_callout.rs`)
//! parce que c'est là qu'elle est bornée ; le tableau complet des trois vit dans
//! `docs/architecture/dispatch-lib-migration.md` § 4.
//!
//! ## 1. Les blocs clôturés — assumée, avec sa raison, côté bash
//!
//! Le Rust retire les blocs clôturés avant de matcher ([`strip_fenced_blocks`]);
//! le bash n'a **aucun** équivalent, et son commentaire de production le disait
//! déjà : « un faux positif est déjà rattrapé par le test `-f` qui suit ».
//! Ce n'était pas une méconnaissance, c'était une asymétrie **assumée**.
//!
//! Cette raison n'est vraie qu'à moitié : le `-f` rattrape un chemin
//! *inexistant*, pas un chemin *existant cité dans un bloc*. Population non
//! mesurée ; ce module ne la ferme pas — il la rend **nommable**, par
//! [`FenceHandling`].
//!
//! ## 2. Le backtick fermant — trouvée en écrivant ce module
//!
//! Le motif Rust exige le backtick de fermeture (`` `(…)` ``); le PCRE bash
//! s'arrêtait à `[^`]+` **sans borne droite**, donc lisait aussi un callout
//! dont le backtick final manque. Le lecteur unique garde la forme **stricte**,
//! ce qui est un resserrement du bash sur cette forme-là.
//!
//! **C'est dit plutôt que découvert**, et le sens du resserrement est celui de
//! la sûreté : desserrer aurait élargi la surface de faux positif de
//! `plan_ownership`, qui décide d'un **abandon** de ticket (mika#2020) — ce que
//! la borne B1 refuse dans les deux sens. La population visée est un callout
//! malformé, que le pipeline de grooming ne produit pas ; elle est mesurée par
//! le corpus (`backtick-unterminated.md`) plutôt que supposée vide.
//!
//! # Références
//!
//! - `docs/architecture/dispatch-lib-migration.md` — la doctrine de migration
//! - `crates/mika-agent/tests/fixtures/plan_callout_bodies/` — le corpus commun
//! - `crates/mika-agent/tests/plan_callout_parity.rs` — le lecteur Rust du corpus

use std::borrow::Cow;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Le littéral `docs/plans/` : ce qui sépare un chemin de plan d'un chemin de
/// `docs/brainstorms/` ou de `docs/solutions/`.
const PLAN_DIR_PREFIX: &str = "docs/plans/";

/// Le préfixe de la ligne que [`rewrite_callout_as_prior_plan`] écrit à la place
/// du run de callout d'un vol livré — **site unique** (mika#2952).
///
/// L'écrivain le consomme pour composer sa ligne et [`plan_reading`] pour la
/// reconnaître ; aucun des deux ne l'épelle. La constante existe parce que
/// mika#2952 a créé le lecteur que le § *L'idempotence est structurelle*
/// déclarait absent : l'orthographe de cette prose porte désormais un contrat,
/// et le tenir par une constante partagée plus un test d'aller-retour
/// ([`tests::mika2952_la_ligne_ecrite_par_la_porte_est_relue_par_le_lecteur`])
/// est structurel là où une convention de relecture serait prudentielle.
pub const PRIOR_PLAN_PREFIX: &str = "> Plan antérieur :";

/// Faut-il retirer les blocs clôturés avant de lire le callout ?
///
/// Un `enum` et **jamais un `bool`** : un booléen au site d'appel ne dit pas
/// lequel des deux sens il porte, et les deux appelants choisissent
/// différemment. Le compilateur force donc chaque appelant à nommer sa
/// politique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceHandling {
    /// Retirer les blocs clôturés avant de matcher.
    ///
    /// **Appelant : `auto_pull`.** Un callout *cité* dans un bloc de code ne
    /// doit pas satisfaire le prédicat qui garde une **promotion** — mika#2120
    /// en est la démonstration, son propre corps citant verbatim le gabarit de
    /// l'étape 19 de `mika-groom-ticket.md`.
    Strip,
    /// Lire le corps tel quel, blocs clôturés compris.
    ///
    /// **Appelant : `dispatch-lib`.** Ce n'est pas un échafaudage de migration
    /// mais le comportement **documenté et justifié** du bash : là-bas un faux
    /// positif est rattrapé par le test `-f` qui suit, et l'écart est assumé
    /// par écrit depuis mika#2120. Retirer cette variante serait corriger un
    /// comportement, pas déplacer une implémentation.
    Keep,
}

/// Le chemin porté par un callout `Plan`, dans les deux formes que ses deux
/// consommateurs attendent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanCallout {
    /// Le chemin tel qu'écrit dans le callout, préfixe de dépôt compris.
    ///
    /// Consommateur : `auto_pull::plan_ownership` — la question est
    /// l'appartenance du plan au ticket.
    pub raw: String,
    /// Le même, premier segment retiré s'il y en avait un.
    ///
    /// Consommateur : `dispatch-lib` — la question est quel fichier ouvrir,
    /// relativement à la racine du sous-dépôt.
    pub normalized: String,
}

/// Le motif du callout `Plan`, sous sa forme stricte.
///
/// # Ce que la permissivité couvre, et ce qu'elle ne couvre pas
///
/// Le callout existe en deux écritures : nue (`docs/plans/…`) et préfixée par
/// le dépôt (`mika/docs/plans/…`, `mika-cloud/docs/plans/…`). La seconde est
/// celle que `/mika-groom-ticket` étape 19 prescrivait, donc celle que le
/// pipeline produit quand personne ne lui demande l'autre — huit récidives
/// mesurées entre le 2026-09-01 et le 2026-09-03 (mika#2120). Un lecteur qui
/// n'accepte qu'une écriture n'applique pas une règle : il en ignore une que
/// son écrivain légitime produit.
///
/// Le préfixe accepté est **n'importe quel segment de tête**, pas la constante
/// `mika/` : le grooming écrit le préfixe du dépôt cible (`mika-cloud#220`).
///
/// La permissivité s'arrête là, et un prédicat élargi qui accepterait n'importe
/// quel chemin n'aurait rien réparé — il aurait ouvert la porte :
///
/// - le premier caractère du segment ne peut pas être un point, ce qui exclut
///   `../docs/plans/` et `./docs/plans/` ;
/// - le littéral `docs/plans/` exclut `docs/brainstorms/` et `docs/solutions/` ;
/// - un seul segment est autorisé, donc `a/b/docs/plans/` échoue ;
/// - le backtick de fermeture est **exigé** (voir § 2 de l'en-tête de module).
///
/// L'ancrage `(?m)^` est ce qui distingue le callout de la prose. Il ne suffit
/// pas à le distinguer de sa **citation**, d'où [`FenceHandling`].
fn callout_re() -> &'static Regex {
    static PLAN_CALLOUT_RE: OnceLock<Regex> = OnceLock::new();
    PLAN_CALLOUT_RE.get_or_init(|| {
        Regex::new(r"(?m)^> - \*\*Plan:\*\* `((?:[A-Za-z0-9_-][A-Za-z0-9._-]*/)?docs/plans/[^`]+)`")
            .expect("plan callout regex must compile")
    })
}

/// Le chemin de plan porté par le callout d'un corps d'issue, ou rien.
///
/// `fences` décide si les blocs clôturés sont retirés **avant** le match ; les
/// deux appelants ne choisissent pas la même politique, et la raison de chacun
/// est sur sa variante de [`FenceHandling`].
///
/// Le premier callout de la ligne la plus haute gagne : c'est la ligne, pas la
/// forme, qui décide de la priorité.
pub fn plan_callout(body: &str, fences: FenceHandling) -> Option<PlanCallout> {
    let readable: Cow<'_, str> = match fences {
        FenceHandling::Strip => strip_fenced_blocks(body),
        FenceHandling::Keep => Cow::Borrowed(body),
    };
    let raw = callout_re().captures(&readable)?[1].to_string();
    let normalized = normalize(&raw);
    Some(PlanCallout { raw, normalized })
}

/// Retire le premier segment du chemin s'il y en avait un.
///
/// Le motif garantit que `raw` est soit `docs/plans/…`, soit
/// `<segment>/docs/plans/…` : il n'y a donc pas de troisième cas à traiter, et
/// le `*) return 1` du `case` bash qu'il remplace était inatteignable.
fn normalize(raw: &str) -> String {
    if raw.starts_with(PLAN_DIR_PREFIX) {
        return raw.to_string();
    }
    match raw.split_once('/') {
        Some((_, rest)) => rest.to_string(),
        // Inatteignable sous le motif ci-dessus, et rendu tel quel plutôt que
        // paniqué : une entrée qu'on ne sait pas normaliser n'est pas une
        // raison d'abattre un tour d'agent.
        None => raw.to_string(),
    }
}

/// Réécrit le callout périmé d'un corps d'issue en une ligne « Plan antérieur »,
/// ou rend `None` si ce corps n'en porte pas (mika#2839).
///
/// # Pourquoi ce transform vit ici
///
/// mika#2194 fait de ce module le site **unique** du motif du callout, tenu par
/// [`tests::mika2194_aucun_motif_de_callout_hors_de_ce_module`] dont l'allowlist
/// est livrée vide et le reste. Un réécriveur ailleurs porterait le motif
/// ailleurs ; la réponse à un scan structurel est de router le site, pas de
/// l'exempter (mika#2201/#2323). La composition de la ligne de remplacement est
/// dans cette fonction pour la même raison : ce qu'on écrit dans un callout
/// relève du même vocabulaire que ce qu'on y lit.
///
/// # Line-wise, et sur le corps *original*
///
/// [`strip_fenced_blocks`] **supprime** des lignes : un offset trouvé sur son
/// résultat ne se remappe pas sur le corps d'entrée. Le parcours suit donc
/// l'état de fence ligne à ligne sur le corps original, et ne considère une
/// ligne de callout que hors fence — un callout *cité* dans un bloc de code
/// n'est pas réécrit. Le repli de `strip_fenced_blocks` est reproduit à
/// l'identique : une fence laissée ouverte ne masque rien, de sorte que la vue
/// de ce transform et celle de `plan_callout(_, Strip)` ne puissent pas
/// divergir — l'appelant compare sur la seconde (mika#2839 R9) et agit par la
/// première.
///
/// # Le **run** entier, pas la seule ligne `Plan`
///
/// `_write_canonical_callout` écrit trois lignes `> - ` contiguës — `Branch`,
/// `Plan`, `Grooming history`. Remplacer la seule ligne `Plan` laisserait
/// `Branch` en place, donc `auto_pull::extract_branch_name` rendrait toujours la
/// branche morte, donc la sonde `gh pr list --head` serait **encore repayée à
/// chaque pull** : la conséquence que mika#2839 existe pour supprimer. Le
/// transform étend donc l'intervalle sur le run contigu de lignes `> - `, quel
/// qu'en soit le nombre — un bloc écrit à la main sur quatre lignes est couvert.
///
/// Elle ne vérifie **pas** que la ligne `Branch` du run nomme `branch` : c'est
/// l'affaire de l'appelant, qui tient la comparaison de fraîcheur.
///
/// # L'idempotence est structurelle
///
/// Agir exige une ligne `> - **Plan:** …` ; après une réécriture elle n'existe
/// plus, donc un second passage rend `None` par construction. Aucun marqueur
/// « déjà réécrit » n'est ajouté au corps.
///
/// # La prose produite a un lecteur depuis mika#2952
///
/// Ce doc-comment disait « *un écrivain et zéro lecteur — rien ne peut dériver
/// de son orthographe* ». Ce n'est plus vrai : [`plan_reading`] reconnaît cette
/// ligne, et le bassin du manager en tire un refus d'admission nommé. Un
/// écrivain, un lecteur, et un test qui les joint —
/// [`tests::mika2952_la_ligne_ecrite_par_la_porte_est_relue_par_le_lecteur`] :
/// les deux côtés consomment [`PRIOR_PLAN_PREFIX`] sans l'épeler, et changer la
/// prose sans changer la constante devient rouge.
pub fn rewrite_callout_as_prior_plan(body: &str, branch: &str, merged_pr: u64) -> Option<String> {
    // `split_inclusive` et non `lines()` : les terminateurs sont conservés, donc
    // le texte autour du run est reproduit à l'octet près (`\r\n` compris) et un
    // corps sans saut de ligne final n'en gagne pas un.
    let lines: Vec<&str> = body.split_inclusive('\n').collect();

    let mut fenced = vec![false; lines.len()];
    let mut open: Option<char> = None;
    for (i, line) in lines.iter().enumerate() {
        match (open, fence_marker(line)) {
            (None, Some(c)) => {
                open = Some(c);
                fenced[i] = true;
            }
            (Some(c), Some(m)) if m == c => {
                fenced[i] = true;
                open = None;
            }
            _ => fenced[i] = open.is_some(),
        }
    }
    // Fence non fermée : le repli de `strip_fenced_blocks`, à l'identique.
    if open.is_some() {
        fenced.iter_mut().for_each(|f| *f = false);
    }

    let (at, plan) = lines.iter().enumerate().find_map(|(i, line)| {
        (!fenced[i])
            .then(|| callout_re().captures(line))
            .flatten()
            .map(|c| (i, c[1].to_string()))
    })?;

    let in_run = |i: usize| !fenced[i] && lines[i].starts_with("> - ");
    let mut start = at;
    while start > 0 && in_run(start - 1) {
        start -= 1;
    }
    let mut end = at;
    while end + 1 < lines.len() && in_run(end + 1) {
        end += 1;
    }

    let mut out = String::with_capacity(body.len() + PRIOR_PLAN_LINE_BUDGET);
    lines[..start].iter().for_each(|l| out.push_str(l));
    // Les trois faits de la ligne sont les trois faits **mesurés** : le chemin lu
    // dans le callout remplacé, la branche sur laquelle la mesure a porté, le
    // numéro de PR rendu par la sonde. Rien n'y affirme que le plan existe à ce
    // chemin sur `main` — la sonde a mesuré une suppression de branche, pas la
    // présence d'un fichier.
    out.push_str(&format!(
        "{PRIOR_PLAN_PREFIX} `{plan}` — la branche `{branch}` qui le portait a été \
         supprimée par le merge de #{merged_pr} dans `main`. Callout réécrit par la \
         porte d'auto-pull (mika#2839)."
    ));
    if lines[end].ends_with("\r\n") {
        out.push_str("\r\n");
    } else if lines[end].ends_with('\n') {
        out.push('\n');
    }
    lines[end + 1..].iter().for_each(|l| out.push_str(l));
    Some(out)
}

/// Réserve d'allocation pour la ligne de prose de [`rewrite_callout_as_prior_plan`].
///
/// Un budget, pas une borne : la ligne porte un chemin et une branche de
/// longueur libre, et `String` réalloue sans drame si elle le dépasse.
const PRIOR_PLAN_LINE_BUDGET: usize = 256;

/// Ce que le corps d'une issue dit de son plan, en **trois** états (mika#2952).
///
/// Un booléen portait deux populations sous un seul nom — « ce corps porte un
/// callout vivant » et « ce corps porte la trace d'un vol déjà livré » — et le
/// bassin du manager concluait « un vol est dû » sur les deux. Trois états parce
/// que les trois appellent trois gestes différents ; les confondre est la classe
/// mika#2205.
///
/// Orthogonal à `milestone_manager::reader::plan_callout_present`, que mika#2608
/// a figé et que mika#2952 **ne resserre pas** : la levée a pris la forme d'une
/// séparation. La propriété de sur-ensemble épinglée par mika#2608 est
/// précisément ce qui rend [`PlanReading::PriorOnly`] bien défini — `lâche \
/// stricte` est non vide, et c'est la population mesurée le 2026-10-28.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanReading {
    /// Le corps porte un callout `> - **Plan:** …` **strict**, hors bloc
    /// clôturé.
    ///
    /// Question répondue : « un vol est-il groomé et dû sur ce ticket ? » — oui.
    /// Geste correctif : aucun, le candidat est admissible. Cet état couvre
    /// aussi le découpage en N vols dont le vol 1 est livré et le vol suivant
    /// groomé : le corps porte alors les **deux** lignes, et c'est le callout
    /// frais qui décide.
    Live,
    /// Pas de callout strict, et au moins une ligne [`PRIOR_PLAN_PREFIX`] : le
    /// travail de vol est livré et la branche qui le portait a disparu.
    ///
    /// Question répondue : « reste-t-il un vol à faire ? » — non. Geste
    /// correctif : mesurer et fermer, ou re-groomer le vol suivant. **Aucun
    /// groom ne débloque cet état tel quel**, d'où le `false` que le refus tiré
    /// par le bassin rend à `remediable_by_grooming()`.
    PriorOnly,
    /// Ni l'un ni l'autre : le corps ne nomme aucun plan.
    ///
    /// Question répondue : « ce ticket est-il groomé ? » — non. Geste correctif :
    /// le groomer, ce que la relève de `milestone_manager::relief` propose déjà
    /// par `RefusalReason::NoGroomedPlan`.
    Absent,
}

/// L'état du plan dans un corps d'issue — le lecteur que mika#2952 crée pour le
/// bassin du manager.
///
/// L'ordre des deux questions **est** la sémantique : un callout strict gagne
/// toujours, donc un corps qui porte à la fois la trace d'un vol livré et un
/// callout frais rend [`PlanReading::Live`]. C'est la distinction « vol 1/2
/// livré » / « tout livré » que mika#2952 exige, et elle tient sur une
/// convention de la boucle plutôt que sur un compte de PR : quand un vol suivant
/// est dû, le grooming réécrit un callout canonique sur une branche neuve.
///
/// La politique de fences du premier test est [`FenceHandling::Strip`], celle
/// d'`auto_pull` : un callout *cité* dans un bloc clôturé n'est pas un plan
/// vivant (mika#2120). Le second test ne la rejoue pas, et l'écart ne change
/// aucun verdict — un corps dont la seule ligne antérieure est citée dans un
/// bloc rendrait sinon `Absent`, qui refuse l'admission tout autant. Seul le
/// **nom** du refus diffère, et nommer la ligne qu'on a lue est le moins faux
/// des deux.
pub fn plan_reading(body: &str) -> PlanReading {
    if plan_callout(body, FenceHandling::Strip).is_some() {
        return PlanReading::Live;
    }
    if body
        .lines()
        .any(|l| l.trim_start().starts_with(PRIOR_PLAN_PREFIX))
    {
        return PlanReading::PriorOnly;
    }
    PlanReading::Absent
}

/// Le marqueur de fence d'une ligne (```` ``` ```` ou `~~~`), ou rien.
fn fence_marker(line: &str) -> Option<char> {
    let trimmed = line.trim_start();
    let c = trimmed.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    (trimmed.chars().take_while(|&x| x == c).count() >= 3).then_some(c)
}

/// Rend le corps privé de ses blocs clôturés (```` ``` ```` et `~~~`), pour que
/// le gabarit d'un callout **cité** dans un ticket ne satisfasse pas le garde
/// qui le lit (mika#2120, AC6).
///
/// L'ancrage en début de ligne ne suffit pas à lui seul : une ligne citée dans
/// une fence commence elle aussi en colonne zéro. mika#2120 en est la
/// démonstration — son corps cite verbatim le gabarit de l'étape 19 de
/// `mika-groom-ticket.md`. Élargir un prédicat non ancré élargit sa surface de
/// faux positif, et le coût d'un faux positif est un créneau de dispatch mort à
/// `_find_issue_plan returned empty`, c'est-à-dire au pire endroit.
///
/// Une fence ouverte par ```` ``` ```` ne se ferme que sur ```` ``` ````,
/// jamais sur `~~~`.
///
/// **Repli sur fence non fermée : ne rien retirer.** Un corps dont une fence
/// n'est jamais refermée est ambigu, et les deux erreurs n'ont pas le même prix
/// — un faux positif coûte un créneau, un faux négatif a coûté quinze heures de
/// boucle (le relevé du 2026-08-31 qui a ouvert mika#2120). La doctrine citée
/// par ce ticket tranche dans le même sens : la *détection* doit être au moins
/// aussi permissive que le consommateur, la *décision* reste stricte. Voir
/// `docs/solutions/architecture-patterns/guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md`.
///
/// **Ce lecteur vit ici, et non plus dans `auto_pull`** (mika#2194) : le lecteur
/// unique du callout doit être autonome, sans quoi il dépendrait d'un de ses
/// propres appelants. Les tests de frontière de mika#2120 restent chez
/// `auto_pull`, où l'ancrage du ticket a sa valeur historique.
pub(crate) fn strip_fenced_blocks(body: &str) -> Cow<'_, str> {
    let mut open: Option<char> = None;
    let mut saw_fence = false;
    for line in body.lines() {
        match (open, fence_marker(line)) {
            (None, Some(c)) => {
                open = Some(c);
                saw_fence = true;
            }
            (Some(c), Some(m)) if m == c => open = None,
            _ => {}
        }
    }
    // Aucune fence, ou une fence laissée ouverte : on rend le corps tel quel.
    if !saw_fence || open.is_some() {
        return Cow::Borrowed(body);
    }

    let mut out = String::with_capacity(body.len());
    let mut open: Option<char> = None;
    for line in body.lines() {
        match (open, fence_marker(line)) {
            (None, Some(c)) => open = Some(c),
            (Some(c), Some(m)) if m == c => open = None,
            (None, None) => {
                out.push_str(line);
                out.push('\n');
            }
            _ => {}
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BARE: &str = "> - **Plan:** `docs/plans/2026-09-01-004-fix-2120-x-plan.md` (committed on branch @ `abc1234`)";
    const PREFIXED: &str = "> - **Plan:** `mika/docs/plans/2026-09-01-004-fix-2120-x-plan.md` (committed on branch @ `abc1234`)";
    const OTHER_REPO: &str = "> - **Plan:** `mika-cloud/docs/plans/2026-09-02-001-fix-220-x-plan.md` (committed on branch @ `abc1234`)";

    #[test]
    fn mika2194_forme_nue_les_deux_champs_sont_egaux() {
        let c = plan_callout(BARE, FenceHandling::Keep).expect("callout lu");
        assert_eq!(c.raw, "docs/plans/2026-09-01-004-fix-2120-x-plan.md");
        assert_eq!(c.normalized, c.raw);
    }

    #[test]
    fn mika2194_forme_prefixee_raw_garde_le_prefixe_normalized_le_retire() {
        let c = plan_callout(PREFIXED, FenceHandling::Keep).expect("callout lu");
        assert_eq!(c.raw, "mika/docs/plans/2026-09-01-004-fix-2120-x-plan.md");
        assert_eq!(c.normalized, "docs/plans/2026-09-01-004-fix-2120-x-plan.md");
    }

    /// Le préfixe accepté est n'importe quel segment de tête, pas `mika/` —
    /// le grooming écrit le préfixe du dépôt cible (mika-cloud#220).
    #[test]
    fn mika2194_le_prefixe_accepte_nest_pas_la_constante_mika() {
        let c = plan_callout(OTHER_REPO, FenceHandling::Keep).expect("callout lu");
        assert_eq!(
            c.raw,
            "mika-cloud/docs/plans/2026-09-02-001-fix-220-x-plan.md"
        );
        assert_eq!(c.normalized, "docs/plans/2026-09-02-001-fix-220-x-plan.md");
    }

    /// Contrôle négatif : élargir ne veut pas dire tout accepter.
    #[test]
    fn mika2194_controles_negatifs() {
        for body in [
            "> - **Plan:** `docs/brainstorms/2026-09-01-x.md` (committed @ `abc`)",
            "> - **Plan:** `mika/docs/solutions/2026-09-01-x.md` (committed @ `abc`)",
            "> - **Plan:** `../docs/plans/2026-09-01-004-fix-2120-x-plan.md` (committed @ `abc`)",
            "> - **Plan:** `./docs/plans/2026-09-01-004-fix-2120-x-plan.md` (committed @ `abc`)",
            "> - **Plan:** `a/b/docs/plans/2026-09-01-004-fix-2120-x-plan.md` (committed @ `abc`)",
            "The Plan: is to refactor the module",
        ] {
            assert!(
                plan_callout(body, FenceHandling::Keep).is_none(),
                "ce corps devait être refusé : {body}"
            );
        }
    }

    /// Ancré : la prose qui *parle* du callout n'en est pas un.
    #[test]
    fn mika2194_le_motif_est_ancre_en_debut_de_ligne() {
        let body = "voir la ligne > - **Plan:** `docs/plans/x-plan.md` du corps";
        assert!(plan_callout(body, FenceHandling::Keep).is_none());
    }

    /// C'est la ligne, pas l'écriture, qui décide de la priorité.
    #[test]
    fn mika2194_premier_callout_gagne_quelle_que_soit_lecriture() {
        let body = "> - **Plan:** `mika/docs/plans/first.md` (committed on branch @ aaa)\n\
                    > - **Plan:** `docs/plans/second.md` (committed on branch @ bbb)";
        let c = plan_callout(body, FenceHandling::Keep).expect("callout lu");
        assert_eq!(c.normalized, "docs/plans/first.md");
    }

    /// La divergence nommée n°1 : les deux politiques de fence ne rendent pas la
    /// même chose sur un callout cité — et c'est le contrat, pas un défaut.
    #[test]
    fn mika2194_les_deux_politiques_de_fence_divergent_sur_un_callout_cite() {
        let body = "prose\n```\n> - **Plan:** `docs/plans/x-plan.md`\n```\nfin";
        assert!(
            plan_callout(body, FenceHandling::Strip).is_none(),
            "Strip doit refuser un callout cité dans un bloc clôturé"
        );
        assert_eq!(
            plan_callout(body, FenceHandling::Keep)
                .expect("Keep doit lire un callout cité")
                .normalized,
            "docs/plans/x-plan.md"
        );
    }

    /// Une fence laissée ouverte ne retire rien : le repli de mika#2120.
    #[test]
    fn mika2194_fence_non_fermee_ne_retire_rien() {
        let body = "```\n> - **Plan:** `docs/plans/x-plan.md`\n";
        assert!(plan_callout(body, FenceHandling::Strip).is_some());
    }

    /// La divergence nommée n°2 : le backtick de fermeture est exigé, donc les
    /// **deux** politiques refusent un callout non terminé. C'est le
    /// resserrement du bash que l'en-tête de module nomme.
    #[test]
    fn mika2194_le_backtick_fermant_est_exige_dans_les_deux_politiques() {
        let body = "> - **Plan:** `docs/plans/x-plan.md";
        assert!(plan_callout(body, FenceHandling::Keep).is_none());
        assert!(plan_callout(body, FenceHandling::Strip).is_none());
    }

    // ───────── Réécriture en « Plan antérieur » (mika#2839) ─────────

    const DEAD_BRANCH: &str = "chore/2108/orienter-le-pilote-avant-le-corps";
    const PLAN_2108: &str =
        "docs/plans/2026-10-19-006-fix-2108-orienter-le-pilote-avant-le-corps-du-ticket-plan.md";

    /// Le corps canonique : les trois lignes que `_write_canonical_callout`
    /// écrit, puis de la prose.
    const CANONICAL: &str = concat!(
        "> - **Branch:** `chore/2108/orienter-le-pilote-avant-le-corps`\n",
        "> - **Plan:** `docs/plans/2026-10-19-006-fix-2108-orienter-le-pilote-avant-le-corps-du-ticket-plan.md` (committed on branch @ `3cbeec04`)\n",
        "> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: bcbf52ea\n",
        "\n",
        "## Le résidu\n",
        "\n",
        "Un paragraphe, avec un `backtick` et une > citation.\n",
    );

    const CANONICAL_REWRITTEN: &str = concat!(
        "> Plan antérieur : `docs/plans/2026-10-19-006-fix-2108-orienter-le-pilote-avant-le-corps-du-ticket-plan.md` — la branche `chore/2108/orienter-le-pilote-avant-le-corps` qui le portait a été supprimée par le merge de #2834 dans `main`. Callout réécrit par la porte d'auto-pull (mika#2839).\n",
        "\n",
        "## Le résidu\n",
        "\n",
        "Un paragraphe, avec un `backtick` et une > citation.\n",
    );

    /// AC1 — le run de trois lignes devient **une** ligne de prose, et tout le
    /// reste du corps est reproduit à l'octet près.
    #[test]
    fn mika2839_le_run_entier_devient_une_ligne_de_prose() {
        let out = rewrite_callout_as_prior_plan(CANONICAL, DEAD_BRANCH, 2834)
            .expect("le corps canonique doit être réécrit");
        assert_eq!(out, CANONICAL_REWRITTEN);
    }

    /// AC2 — l'idempotence est structurelle : la sortie ne porte plus de ligne
    /// `Plan`, donc un second passage ne trouve rien. Aucun marqueur
    /// « déjà réécrit » n'a été ajouté — la seule différence est le remplacement.
    #[test]
    fn mika2839_un_second_passage_rend_none_sans_marqueur() {
        let once =
            rewrite_callout_as_prior_plan(CANONICAL, DEAD_BRANCH, 2834).expect("1er passage");
        assert!(rewrite_callout_as_prior_plan(&once, DEAD_BRANCH, 2834).is_none());
    }

    /// AC3 — un corps dont le seul callout vit dans un bloc clôturé n'est pas
    /// touché. C'est `Strip` appliqué sans remappage d'offset (R2).
    #[test]
    fn mika2839_un_callout_cite_dans_une_fence_nest_pas_reecrit() {
        let body = "prose\n```\n> - **Plan:** `docs/plans/x-plan.md`\n```\nfin\n";
        assert!(rewrite_callout_as_prior_plan(body, DEAD_BRANCH, 2834).is_none());
    }

    /// Un corps sans aucune ligne de callout rend `None`.
    #[test]
    fn mika2839_sans_callout_rien_nest_reecrit() {
        assert!(
            rewrite_callout_as_prior_plan("## Titre\n\ndu texte.\n", DEAD_BRANCH, 2834).is_none()
        );
    }

    /// Le risque nommé par le plan : un bloc non canonique, écrit à la main sur
    /// quatre lignes. Le transform agit sur le run contigu qu'il trouve, quel
    /// qu'en soit le nombre de lignes.
    #[test]
    fn mika2839_un_run_non_canonique_de_quatre_lignes_est_remplace_entier() {
        let body = concat!(
            "> - **Issue:** senara-solutions/mika#2108\n",
            "> - **Branch:** `chore/2108/orienter-le-pilote-avant-le-corps`\n",
            "> - **Plan:** `docs/plans/x-plan.md`\n",
            "> - **Grooming history:** GROOMED\n",
            "\n",
            "suite\n",
        );
        let out = rewrite_callout_as_prior_plan(body, DEAD_BRANCH, 2834).expect("réécrit");
        assert_eq!(
            out.lines().filter(|l| l.starts_with("> - ")).count(),
            0,
            "aucune ligne du run ne doit survivre : {out}"
        );
        assert!(
            out.ends_with("\n\nsuite\n"),
            "la suite est préservée : {out}"
        );
    }

    /// V2 / AC4 — la chaîne de conséquences de R8, telle qu'elle est.
    ///
    /// **Un écart assumé avec la lettre de V2/AC4**, qui demandent
    /// `extract_plan_path` → `Some` sur la sortie. C'est impossible, et
    /// l'impossibilité est portée par R5 : `extract_plan_path` **est** le
    /// lecteur strict ancré de ce module, donc le faire rendre `Some` exigerait
    /// de laisser la ligne `> - **Plan:**` en place, donc d'annuler
    /// l'idempotence structurelle de R5 *et* de laisser `extract_branch_name`
    /// rendre la branche morte (R3). La justification que V2 et AC4 donnent
    /// eux-mêmes — « le substring `docs/plans/` subsiste », « et
    /// `check_grooming_markers` le voit toujours comme portant un plan » — est
    /// celle du lecteur **lâche**, pas du strict ; c'est elle qui est assertée
    /// ici, avec la chute du strict qui tarit la sonde.
    #[test]
    fn mika2839_la_sortie_tarit_le_lecteur_strict_et_garde_le_lache() {
        let out = rewrite_callout_as_prior_plan(CANONICAL, DEAD_BRANCH, 2834).expect("réécrit");
        assert!(
            plan_callout(&out, FenceHandling::Strip).is_none(),
            "le lecteur strict ne doit plus rien voir — c'est ce qui fait tomber \
             `extract_branch_name` et donc la sonde `gh pr list --head`"
        );
        assert!(
            !out.contains("> - **Branch:**"),
            "la ligne `Branch` doit avoir disparu avec le run (R3)"
        );
        assert!(
            out.contains(PLAN_2108) && out.contains("docs/plans/"),
            "le substring non ancré que lit `check_grooming_markers` subsiste"
        );
    }

    // ───────── Lecture à trois états du plan (mika#2952) ─────────

    /// **C'est le test qui tient le contrat d'orthographe.**
    ///
    /// L'écrivain compose, le lecteur relit. Les deux consomment
    /// [`PRIOR_PLAN_PREFIX`] sans l'épeler, donc changer la prose de la porte
    /// d'auto-pull sans changer la constante est impossible à merger : c'est ce
    /// qui remplace la phrase « zéro lecteur » que le doc-comment portait.
    #[test]
    fn mika2952_la_ligne_ecrite_par_la_porte_est_relue_par_le_lecteur() {
        assert_eq!(
            plan_reading(CANONICAL),
            PlanReading::Live,
            "le corps canonique porte un callout strict"
        );
        let out = rewrite_callout_as_prior_plan(CANONICAL, DEAD_BRANCH, 2834)
            .expect("le corps canonique doit être réécrit");
        assert_eq!(
            plan_reading(&out),
            PlanReading::PriorOnly,
            "la ligne que la porte vient d'écrire doit être relue : {out}"
        );
    }

    /// AC3-b au niveau de la grammaire : un découpage en N vols dont le vol 1
    /// est livré et le vol 2 groomé porte les **deux** lignes, et c'est le
    /// callout frais qui décide.
    #[test]
    fn mika2952_un_callout_vivant_a_cote_dune_ligne_anterieure_reste_live() {
        let body = format!("{CANONICAL_REWRITTEN}\n{CANONICAL}");
        assert!(
            body.contains(PRIOR_PLAN_PREFIX),
            "la trace du vol livré est bien là : {body}"
        );
        assert_eq!(plan_reading(&body), PlanReading::Live);
    }

    // Le corpus mesuré : extraits des corps de #2943, #2946 et #2680, relus sur
    // la forge le 2026-10-28. Les trois portent une ligne `Plan antérieur :` et
    // **aucune** ligne de callout strict — ni, pour les trois, de ligne
    // `Branch`, la vérification croisée que le § 2 du plan nomme.

    /// #2943 — ligne machine, écrite par la porte au merge de #2945.
    const CORPUS_2943: &str = concat!(
        "> Plan antérieur : `docs/plans/2026-10-28-001-fix-2943-filet-de-verdict-sur-coupure-max-steps-webhook-plan.md` — la branche `fix/2943/qa-review-un-tour-webhook-de-mika-qa` qui le portait a été supprimée par le merge de #2945 dans `main`. Callout réécrit par la porte d'auto-pull (mika#2839).\n",
        "\n",
        "## Le défaut\n",
        "\n",
        "Quand un tour **webhook** de mika-qa épuise son budget de pas, il se termine en `outcome_absent` sans verdict.\n",
    );

    /// #2946 — ligne machine, écrite par la porte au merge de #2949.
    const CORPUS_2946: &str = concat!(
        "> Plan antérieur : `docs/plans/2026-10-28-002-fix-2946-levaluateur-ne-compte-plus-un-groom-comme-un-vol-plan.md` — la branche `fix/2946/mika-manager-l-valuateur-compte-les` qui le portait a été supprimée par le merge de #2949 dans `main`. Callout réécrit par la porte d'auto-pull (mika#2839).\n",
        "\n",
        "## Le défaut\n",
        "\n",
        "L'évaluateur compte comme « vols terminés » des tâches qui n'en sont pas.\n",
    );

    /// #2680 — ligne **écrite à la main**, sans la clause « supprimée par le
    /// merge de #N », en milieu de corps, et dont la PR citée (#2833) a été
    /// *fermée sans merge*. C'est le cas qu'un ancrage sur la clause de merge
    /// aurait raté, et dont extraire un numéro de PR aurait menti (§ question 1).
    const CORPUS_2680: &str = concat!(
        "**Conclusion : (a) tient.** Elle coûte trois pièces : le gateway transmet `head_sha`.\n",
        "\n",
        "---\n",
        "> Plan antérieur : `docs/plans/2026-10-19-007-feat-2680-cellule-g-chiffree-et-refutee-plan.md` (vol 6). Il n'est pas sur `main` : il vit dans la PR #2833, fermée sans merge le 2026-10-20 (tête `816e05b8`, lisible par `refs/pull/2833/head`). Le résultat de ce vol est versé plus bas, dans « Vol 6 : résultat ».\n",
        "\n",
        "## Vol 6 : résultat\n",
    );

    /// Le corpus de régression : les trois corps que le bassin lisait comme
    /// portant un plan vivant rendent `PriorOnly`.
    #[test]
    fn mika2952_le_corpus_mesure_rend_prior_only() {
        for (ticket, body) in [
            ("#2943", CORPUS_2943),
            ("#2946", CORPUS_2946),
            ("#2680", CORPUS_2680),
        ] {
            assert_eq!(
                plan_reading(body),
                PlanReading::PriorOnly,
                "{ticket} doit se lire comme un vol livré"
            );
            // Vérification croisée : le signal corroborant du § 2, écarté comme
            // prédicat parce qu'il est aveugle par origine, mais vrai ici.
            assert!(
                !body.contains("> - **Branch:**"),
                "{ticket} ne porte plus de ligne `Branch`"
            );
        }
        // Contrôle négatif de bonne foi (mika#2205) : le prédicat n'est pas un
        // « tout rend PriorOnly ». Un corps qui ne nomme aucun plan rend
        // `Absent`, et c'est `NoGroomedPlan` qui le refusera, pas ce motif.
        assert_eq!(
            plan_reading("## Le défaut\n\nUn ticket que personne n'a groomé.\n"),
            PlanReading::Absent
        );
    }

    /// **Livrée vide, et elle le reste** — l'exception nommée de la
    /// Fire-Disposition de mika#2952, forme mika#2425 / mika#1833.
    ///
    /// Un tuple `(corps, ticket de suivi)`. Une future ligne `Plan antérieur :`
    /// dont la lecture devrait être exceptée s'inscrit ici **avec son ticket**,
    /// jamais par un élargissement du prédicat — ce qui est la différence entre
    /// une exception qui expire et l'allowlist que mika#2309 condamne.
    const MIKA2952_CORPUS_EXCEPTIONS: &[(&str, &str)] = &[];

    /// L'assertion auto-nettoyante de l'allowlist ci-dessus.
    #[test]
    fn mika2952_lallowlist_du_corpus_est_vide() {
        assert!(
            MIKA2952_CORPUS_EXCEPTIONS.is_empty(),
            "une exception a été inscrite : {MIKA2952_CORPUS_EXCEPTIONS:?}\n\n\
             RÉSOLUTION : chaque entrée doit citer son ticket de suivi, et ce test \
             doit alors être édité pour l'exiger nommément. Une allowlist qui \
             grossit sans ticket est celle que mika#2309 condamne."
        );
    }

    /// Un seul motif dans l'arbre de production : la garde structurelle de
    /// mika#2194 R5, côté Rust.
    ///
    /// Aucun test comportemental ne peut voir cette classe — un second motif ne
    /// rend **aucune** décision fausse le jour où il est écrit, il diverge
    /// ensuite, en silence, avec toutes les assertions au vert. C'est
    /// exactement la leçon de mika#2158.
    #[test]
    fn mika2194_aucun_motif_de_callout_hors_de_ce_module() {
        /// **Livrée vide, et elle le reste.**
        ///
        /// Quand ce scan tire, on **route le site vers ce module** ; on ne
        /// l'allowliste pas. Une allowlist née vide est un emplacement où
        /// déposer la prochaine infraction (mika#2323).
        const ALLOWED: &[&str] = &[];

        // L'aiguille est le motif **échappé pour une regex**, jamais le
        // littéral nu. La distinction est mesurée : le littéral
        // `> - **Plan:**` est porté par huit fichiers de production — des
        // fixtures, des messages d'opérateur, le `contains` lâche
        // d'`executor` — dont aucun n'est un second motif. Un scan sur le
        // littéral serait rouge à la naissance, donc désarmé.
        //
        // Composée à l'exécution pour que **ce fichier ne se dénonce pas
        // lui-même** — le motif de `worktree_reaper.rs`.
        let needle = format!(r"\*\*{}:\*\*", "Plan");

        // La frontière production/test est répondue par son lecteur unique
        // (mika#2398) plutôt que par une troncature au premier `#[cfg(test)]` :
        // cette prémisse-là est fausse six fois dans cet arbre, et sur
        // `auto_pull.rs` précisément elle laisse 1 859 lignes de production
        // non lues. Le scan masque, il ne tronque pas.
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let this_file = scanner.src_root().join("plan_callout.rs");

        let mut offenders: Vec<String> = Vec::new();
        let mut scanned = 0usize;
        scanner.for_each(|path, production| {
            if path == this_file {
                return;
            }
            let rel = path
                .strip_prefix(scanner.src_root())
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            if ALLOWED.contains(&rel.as_str()) {
                return;
            }
            scanned += 1;
            // Les commentaires sont retirés ensuite : une prose qui *décrit* le
            // motif n'en est pas un, et cet arbre en porte plusieurs pages.
            if crate::source_scan::strip_comment_lines(production).contains(&needle) {
                offenders.push(rel);
            }
        });

        assert!(
            scanned > 0,
            "aucun fichier examiné — un scan qui ne scanne rien est un \
             laissez-passer vide, pas un scan propre (mika#2103)"
        );
        assert!(
            offenders.is_empty(),
            "des sites portent le motif du callout `Plan` hors de \
             `plan_callout.rs` : {offenders:?}\n\n\
             RÉSOLUTION : appeler `plan_callout::plan_callout` depuis ce site, \
             et non ajouter une ligne d'allowlist. Un lecteur qu'on ne veut pas \
             router est un lecteur qu'il faut supprimer (mika#2201 § D5/D6)."
        );
    }

    /// Contrôle de bonne foi du scan ci-dessus : il **voit** un second motif.
    ///
    /// Sans ce contrôle, « aucun site hors de ce module » est indistinguable de
    /// « le prédicat ne regarde rien » — la classe mika#2205 appliquée à la
    /// garde elle-même.
    #[test]
    fn mika2194_le_scan_voit_un_second_motif() {
        let needle = format!(r"\*\*{}:\*\*", "Plan");
        let faux_site = format!(
            "fn lecteur_bis() {{\n    \
             Regex::new(r\"(?m)^> - {needle} `docs/plans/\").unwrap();\n}}\n"
        );
        assert!(
            crate::source_scan::strip_comment_lines(&faux_site).contains(&needle),
            "le prédicat du scan doit accuser un second motif écrit en clair"
        );
        // Et le contrôle négatif du même prédicat : une prose qui en parle
        // n'est pas un site.
        let prose = format!("// le motif est {needle}, et il vit ailleurs\n");
        assert!(
            !crate::source_scan::strip_comment_lines(&prose).contains(&needle),
            "un commentaire décrivant le motif ne doit pas être accusé"
        );
    }
}
