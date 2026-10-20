# mika#2855 — le vol d'instrument : l'issue du tour `qa-review-webhook-success`, attestée

Successeur (g′) de mika#2680. #2680 a chiffré puis **réfuté** la classe (g) :
`mika-qa` × `check_suite` success sur une branche de PR, 36 tours, 10 687 327
tokens d'entrée, 16,31 % de la fenêtre mesurée — et le tour n'est pas inutile
(la porte mika#2260 le préserve exprès, et `qa-review-webhook-success` est
l'unique entrée autonome de la revue QA sur CI vert). Seuls ses **arrêts
précoces** seraient filtrables, et leur nombre est **inconnu** : le registre dit
que le tour a eu lieu, jamais ce qu'il a fait.

**Ce vol n'écarte aucun tour. Il mesure.** Il ajoute un axe au registre et rien
d'autre.

---

## État des lieux, vérifié sur l'arbre servi

Trois faits de forme, chacun lu sur le code de la branche, parce que chacun
ferme une option de design.

**1. La ligne d'entrée du registre ne peut pas porter l'issue.**
`emit_turn_charged` (`crates/mika-agent/src/server/webhook_prefilter.rs:1169`)
est appelée depuis un seul site —
`crates/mika-agent/src/server/handlers.rs:2057`, dans le bras
`Gate::Charged(release)` — et ce site est **avant** `run_agent`
(`handlers.rs:2126`). Son message est littéralement *« tour webhook facturé — le
tour a lieu »*. Il n'existe, à cet instant, aucune issue à nommer. Il faut un
**second terme**, émis après le tour, corrélé par `request_id`.

**2. Le site du second terme existe déjà, et il voit les deux branches.**

```rust
// handlers.rs:2126-2129
let run = agent::run_agent(&params).await;
post_verdict_if_turn_did_not_conclude(state, a, conclusion_of(&run), &req, &session_id).await;

match run {
    Ok(mut output) => { … }
    Err(e) => { … }
```

Le `match` est **dissocié** de l'appel, délibérément, pour que le filet
mika#2276/mika#2289 soit atteint depuis `Ok` *et* depuis `Err`. Ce même
interstice est le seul endroit du moteur qui tienne à la fois `req.request_id`,
`a.db.agent_id()`, `original_github_text` (donc : ce tour est un tour webhook) et
le texte final du tour. Un seul site d'émission suffit, et c'est ce qui permet de
tenir un **seul écrivain** pour la nouvelle ligne.

**3. `qa-review` ne déclare aucune ligne suffixe, donc rien n'entre en
collision.** `grep -l required_suffix_lines skills/bundled/*/skill.toml` ne rend
que les trois skills `mika-arch-*`. Le chemin qa n'a aucun contrat de dernière
ligne à respecter aujourd'hui, et l'ajout d'un marqueur ne peut donc pas
redbuter une garde existante.

Et un fait de périmètre : le prompt du skill
(`skills/bundled/qa-review-webhook-success/system_prompt.md`) décrit six issues —
quatre arrêts (§ 1 à 4), la revue (§ 5), l'échec de revue (§ 6) — sans qu'aucune
ne laisse de trace nommée. Les étapes 1 et 2 se terminent sur un *« stop the
turn »* qui n'écrit rien du tout.

---

## Décision 1 — l'issue est **attestée par le modèle**, jamais déduite de la trace

Le ticket pose la question et donne le critère : *« Une déduction ne sait pas
nommer lequel des arrêts 1 à 4 a joué, ce que la réserve de Prime exige. Une
issue doit être attestée, pas devinée. »* La vérification sur le prompt confirme
que la déduction est structurellement impossible, et pas seulement imprécise :

| issue | ce que la trace d'outils porte |
|---|---|
| arrêt 1 (pas de PR ouverte) | un `run_gh pr list` |
| arrêt 2 (hors périmètre) | un `run_gh pr list` |
| arrêt 3 (déjà revue à ce SHA) | un `run_gh pr list` + un `run_gh pr view --json reviews` |
| arrêt 4 (verdict déjà posé) | un `run_gh pr list` + un `run_gh pr view --json reviews` |

Les arrêts 1 et 2 sont **indiscernables** sur la trace, et 3 et 4 le sont entre
eux : la décision vit dans le JSON que le modèle a lu, jamais dans la liste des
appels. Une déduction rendrait deux paires fusionnées — exactement la classe
mika#2205 (deux populations sous un nom, non soustractibles), et la réserve de
Prime demande le contraire : *« le champ nomme **lequel** des quatre arrêts ».*

**Donc : marqueur déclaratif, sur sa propre ligne, dans le texte final du tour.**
Forme retenue :

```
Webhook-Outcome: stop_already_reviewed_sha
```

Un **outil** d'attestation était l'autre option, refusée pour trois raisons
nommées : il ajoute une surface d'outil à tous les tours de l'agent pour un
usage d'un seul skill ; il faut l'ajouter à `MIKA_QA_IDENTITY`, donc toucher une
allowlist, donc sortir du périmètre « rien ne change hors l'attestation » (AC5) ;
et il coûte une étape de boucle par tour — un instrument qui renchérit le tour
dont il mesure le coût.

---

## Décision 2 — la garde `required_suffix_lines` n'est **pas** armée, et c'est le terme `outcome_absent` qui rend ce refus mesurable

Le dépôt a un mécanisme structurel pour forcer une ligne de fin :
`[output].required_suffix_lines` dans `skill.toml` arme la garde EndTurn #864
(`agent_loop/mod.rs:3987-4003`), qui re-prompte **une fois** quand aucune des
trois dernières lignes non vides ne matche. Le réflexe du dépôt
(`feedback_prompt_enforcement_fragile.md`, cité dans le texte de la garde
elle-même) est de préférer la garde au « MUST » de prompt.

**Ici, non**, et pour une raison propre à ce vol :

- **un re-prompt est un appel LLM supplémentaire sur le tour même que le vol
  mesure.** (g′) existe pour chiffrer le coût des arrêts de ce tour ; un
  instrument qui peut le doubler quand le modèle oublie sa ligne biaise la
  mesure dans le sens exact où elle est lue. Un instrument qui change la
  grandeur qu'il observe n'est pas un instrument.
- la garde est exhaustive sur la **dernière ligne**, et sur l'issue `review` le
  texte final appartient à `qa-review` (verdict, énumération par AC, preuves).
  Lui imposer un suffixe de forme, c'est un second contrat de sortie sur un
  skill que ce vol ne touche pas.

Le prix de ce refus est nommé, pas passé sous silence : le modèle peut ne rien
attester. C'est précisément ce que `outcome_absent` compte. **Si la première
fenêtre mesurée montre un taux d'absence élevé, armer la garde est le vol
suivant — et on saura alors ce qu'il coûte**, ce qu'on ne sait pas aujourd'hui.
L'ordre est : mesurer la désobéissance, puis décider de la contraindre.

---

## Décision 3 — huit termes, dont deux que le modèle ne peut pas écrire, et le type porte ce refus

Les six noms du ticket sont retenus tels quels — ils nomment les six sorties du
prompt, dans son ordre :

| terme | § du prompt |
|---|---|
| `stop_no_open_pr` | 1 — aucune PR ouverte sur le head SHA |
| `stop_out_of_scope` | 2 — dépôt / auteur / état hors périmètre |
| `stop_already_reviewed_sha` | 3 — une revue `mika-platform-qa` porte déjà ce `commit_id` |
| `stop_verdict_recorded` | 4 — un verdict d'autorité est déjà posé sur la tête |
| `review` | 5 — le skill `qa-review` a été appelé |
| `review_failed` | 6 — la revue n'a pas pu se faire (`send_message` + `blocked`) |

Deux termes de plus, que le moteur **dérive** et que le modèle ne peut pas
écrire :

- `outcome_absent` — aucune ligne `Webhook-Outcome:` dans le texte final (y
  compris : pas de texte du tout, branche `Err` du loop). C'est l'exigence AC2.
- `outcome_unreadable` — une ligne présente dont la valeur n'est **aucun** des
  six termes.

**`outcome_unreadable` est un raffinement d'AC1, et il est argumenté.** AC1
énumère sept valeurs et le ticket précise *« les noms sont à fixer au groom »*.
Replier une valeur illisible sur `outcome_absent` mettrait sous un seul nom deux
faits de signe opposé : « le modèle n'a rien dit » et « le modèle a dit quelque
chose que le contrat ne prévoit pas » — le premier appelle un durcissement de
prompt, le second appelle une correction du contrat ou de l'enum. Deux lectures,
deux suites, non soustractibles sous un nom partagé : classe mika#2205, la même
que la décision 1 refuse ailleurs.

**Le type porte la frontière**, il ne la documente pas :

```rust
/// Ce que le MODÈLE peut attester. Les six sorties du prompt, et rien d'autre.
pub(crate) enum Attested { StopNoOpenPr, StopOutOfScope, StopAlreadyReviewedSha,
                           StopVerdictRecorded, Review, ReviewFailed }

/// Ce que le MOTEUR met sur le fil. `Absent` et `Unreadable` ne sont pas
/// parsables : aucun texte ne peut les produire, seul le moteur les dérive.
pub(crate) enum Outcome { Attested(Attested), Absent, Unreadable }
```

`Attested: FromStr` est l'unique porte d'entrée depuis du texte, donc
`Outcome::Absent` est **inatteignable depuis une valeur écrite** — propriété du
type, pas d'une assertion. Même idiome que `Gate::{Skipped, Charged}` du vol 4 et
`Transmission::{Incoming, Outgoing}` de mika#2807.

---

## Décision 4 — règle de lecture : la **dernière** occurrence gagne, le parse est plus tolérant que le prompt

Le prompt demande la ligne **en dernière ligne** du texte. Le parse, lui, scanne
tout le texte et retient la **dernière** ligne dont le trim commence par
`Webhook-Outcome:`.

Deux raisons, toutes deux mesurables :

- **la dernière, et non la première** : un modèle qui raisonne à voix haute peut
  citer le contrat (« I will emit `Webhook-Outcome: review` once the review
  completes ») avant de l'honorer. La prose précède l'attestation ; prendre la
  première occurrence transformerait une intention en fait.
- **tolérant sur la position, strict sur la valeur** : exiger la position ferait
  rendre `outcome_absent` pour une attestation correcte mais mal placée, c'est-à-
  dire compter une obéissance comme une désobéissance. Inversement, une valeur
  hors des six n'est **jamais** devinée ni rapprochée : `outcome_unreadable`.

La comparaison du préfixe est **sensible à la casse** et le trim ne porte que sur
les blancs de bord. Un préfixe est un contrat, pas une intention à reconnaître au
lexique — le dépôt a déjà refusé la reconnaissance lexicale sous ce nom (D4 de
mika#2136).

---

## Décision 5 — la ligne de sortie porte `agent_id`, et ce n'est pas de la redondance

La ligne de sortie porte exactement trois champs : `agent_id`, `request_id`,
`outcome`. `agent_id` est déjà sur la ligne d'entrée, et le lecteur joint par
`request_id` : il semble redondant. **Il ne l'est pas**, et l'omettre casserait
le lecteur en silence.

`scripts/measure-webhook-turn-cost:204` filtre **chaque ligne du corpus** sur
`--agent` :

```
elif ($agent != "" and (($o.agent_id? // "") != $agent)) then empty
```

Une ligne de sortie sans `agent_id` serait écartée par tout appel `--agent
mika-qa` — c'est-à-dire par l'appel même que (g′) exige — et la jointure rendrait
`outcome: null` partout, sur un agrégat qui aurait l'air complet. Le champ est
donc une **condition de correction du lecteur**, pas un confort.

Le nom de l'événement est `webhook_turn_outcome`, sur `target: "mika::otel"`,
niveau INFO, **dégaté** de `MIKA_STORE_LLM_CALLS` comme son voisin d'entrée (même
raison : une fenêtre où la télémétrie a été réduite est précisément celle qu'on ne
veut pas perdre). **Aucune ligne d'audit** : `audit_events` reste réservé aux
décisions du moteur, et l'issue d'un tour est un fait de coût — le vol 1 de
mika#2680 a tranché cela pour la ligne d'entrée, et la sortie suit son voisin.

**Pas de scan « seul écrivain ».** Le dépôt impose ce scan aux `tool_name`
d'audit (`auth_boundary`, `agent_recurring_gate`), jamais aux noms d'événement
`tracing` : `WEBHOOK_TURN_CHARGED_EVENT`, le voisin immédiat et le précédent
direct de ce vol, porte « **SEUL ÉCRIVAIN** » en doc-comment et aucun scan. On
suit le précédent voisin plutôt que d'en ouvrir un nouveau dans un vol
d'instrument.

---

## Décision 6 — un seul écrivain, placé où le filet mika#2276 est placé

```rust
// handlers.rs, entre run_agent et le match — exactement là où le filet vit déjà
let run = agent::run_agent(&params).await;
post_verdict_if_turn_did_not_conclude(…).await;

// mika#2855 — le second terme du registre. UN SEUL SITE, et il voit les deux
// branches : sur `Err` il n'existe aucun `AgentOutput`, donc aucun texte, donc
// `outcome_absent` — un tour mort n'a rien attesté, et c'est vrai.
if original_github_text.is_some() {
    webhook_turn_outcome::emit_turn_outcome(
        a.db.agent_id(),
        &req.request_id,
        run.as_ref().ok().and_then(|o| o.text.as_deref()),
    );
}

match run { … }
```

Trois propriétés, chacune une raison de ce placement :

- **après `run_agent`, donc structurellement incapable d'influer sur le
  pré-filtre** (AC5). La porte a déjà rendu son `Gate` cent lignes plus haut.
- **avant l'envoi sur le canal**, comme le filet mika#2276 et pour la même
  raison : ce qui est mesuré ne doit pas dépendre de la réussite d'une
  délivrance. Un tour webhook est `NoChannel` de toute façon (`chat_id == 0`).
- **un seul site, parce que le `match` est déjà dissocié.** Émettre dans les deux
  bras donnerait deux écrivains pour une ligne, et le prochain éditeur qui
  ajoute un bras en oublierait un.

Le périmètre est `original_github_text.is_some()` — identique à celui de la ligne
d'entrée, donc **les deux moitiés du registre portent sur la même population**,
ce qui est la condition pour que la jointure du vol 2 soit complète. Conséquence
assumée : tout tour webhook émet une ligne de sortie, y compris les tours
mika-dev dont le prompt n'atteste rien, qui sortent `outcome_absent`. C'est
exact — ils n'attestent rien — et c'est aussi ce qui rend l'instrument adoptable
par un autre skill sans toucher au moteur.

**Le texte n'est pas nettoyé du marqueur.** Il n'est délivré à personne
(`NoChannel`), et un strip serait une mutation du texte que le filet mika#2276 et
`undelivered_send_line` lisent juste après.

---

## Décision 7 — la parité prompt ↔ enum est tenue par une assertion, pas par la vigilance

Les six termes sont écrits **deux fois** : dans l'enum Rust et dans le
`system_prompt.md` du skill. C'est exactement la classe que ce dépôt a payée deux
fois (mika#2158 `grooming_marker`, mika#2362 le seuil de retry recopié) : deux
copies qui dérivent, et la dérive est silencieuse — le parse rendrait
`outcome_unreadable` sur un terme que le prompt demande, et l'agrégat aurait
l'air plein.

Un test lit le prompt **embarqué** (`BUNDLED_SKILL_MANIFESTS`, donc ce que le
binaire sert, pas ce que l'arbre contient) et assert **dans les deux sens** :

1. chacun des six `Attested::as_str()` apparaît littéralement dans le prompt,
   précédé du préfixe ;
2. toute occurrence de `Webhook-Outcome:` dans le prompt porte une valeur qui
   `Attested::from_str` accepte — hors les lignes de l'allowlist.

La bidirectionnalité est ce qui empêche l'assertion de devenir l'allowlist que
mika#2309 condamne : ajouter un terme au prompt sans l'ajouter à l'enum rougit,
et retirer un terme de l'enum sans le retirer du prompt rougit aussi.

---

## Phases

### Phase 1 — le module, pur et testable sans bouchon

**Nouveau fichier** `crates/mika-agent/src/server/webhook_turn_outcome.rs`,
déclaré `pub(crate) mod webhook_turn_outcome;` dans `server/mod.rs` à côté de
`webhook_prefilter`.

Contenu :

- doc de module : pourquoi un second terme (le fait de forme de la décision 1),
  pourquoi attesté et non déduit (la table des traces indiscernables), pourquoi
  la garde #864 n'est pas armée (décision 2), et la raison de forme d'`agent_id`
  (décision 5) ;
- `const WEBHOOK_TURN_OUTCOME_EVENT: &str = "webhook_turn_outcome";` avec
  « **SEUL ÉCRIVAIN** : `emit_turn_outcome` » ;
- `const ATTESTATION_PREFIX: &str = "Webhook-Outcome:";` — source de vérité
  unique du préfixe, lue par le parse ET par le test de parité ;
- `enum Attested` (6 variants) + `as_str()` + `impl FromStr` + `const ALL: [Attested; 6]` ;
- `enum Outcome` (3 variants) + `as_str()` ;
- `fn parse_attestation(text: Option<&str>) -> Outcome` — **pure**, aucune
  lecture d'état, aucun appel réseau ;
- `fn emit_turn_outcome(agent_id: &str, request_id: &str, text: Option<&str>)` —
  appelle `parse_attestation`, émet l'`info!`. Un seul `info!`, trois champs, pas
  de bras conditionnel : aucun champ de cette ligne ne peut manquer (contrairement
  à `candidate` sur la ligne d'entrée).

Tests inline (`#[cfg(test)] mod tests`), un rouge par valeur :

| test | ce qu'il pine |
|---|---|
| `mika2855_les_six_termes_attestes_se_lisent` | les 6 `Attested`, un cas chacun — AC1 |
| `mika2855_un_tour_qui_natteste_rien_rend_absent` | texte sans ligne → `Absent` — **AC2** |
| `mika2855_un_tour_sans_texte_rend_absent` | `None` (branche `Err`) → `Absent`, jamais un arrêt — **AC2** |
| `mika2855_une_valeur_hors_contrat_nest_pas_une_absence` | `Webhook-Outcome: blah` → `Unreadable` ≠ `Absent` — décision 3 |
| `mika2855_la_derniere_occurrence_gagne` | prose citant le préfixe, puis attestation → la seconde — décision 4 |
| `mika2855_absent_nest_pas_parsable` | `Attested::from_str("outcome_absent")` et `("outcome_unreadable")` échouent — la frontière du type |
| `mika2855_le_prompt_embarque_et_lenum_ne_divergent_pas` | parité bidirectionnelle — décision 7 |

### Phase 2 — le site d'émission

`handlers.rs` : le bloc de la décision 6, entre
`post_verdict_if_turn_did_not_conclude` et le `match run`. Plus l'entrée de
module dans `server/mod.rs`.

Un test dans `server/mod.rs::tests`, **sur le harnais de capture existant**
(`Mika2680Event` + `mika2680_events_named`, `server/mod.rs:6493`) : la ligne
`webhook_turn_outcome` porte ses trois champs, et son `request_id` est celui de
la ligne `webhook_turn_charged` du même tour. Aucun nouveau harnais de
`tracing`.

### Phase 3 — le prompt du skill

`skills/bundled/qa-review-webhook-success/system_prompt.md` gagne une section
**« Outcome attestation (mika#2855) »** :

- la règle : *le texte final du tour finit par* `Webhook-Outcome: <valeur>`,
  *sur sa propre ligne, littéralement, pour **chaque** issue* ;
- la table des six valeurs, une par § du chemin de décision, avec le § cité ;
- la mention explicite que les six § existants restent inchangés : les arrêts
  arrêtent, la revue est dispatchée, l'échec notifie et passe `blocked` — la
  ligne est **ajoutée**, aucune décision ne change (AC5) ;
- la mention que `outcome_absent` est ce que le moteur écrit si la ligne manque,
  pour que la conséquence de l'oubli soit dans le prompt et pas seulement dans
  le code.

Aucun `skill.toml` touché : pas de `required_suffix_lines` (décision 2), pas de
nouveau `required_tools`, pas de nouveau mot-clé de déclenchement.

`skills/bundled/` n'est lu qu'au **build** : la ligne n'existe pour aucun agent
avant `make deploy`. La DoD le porte.

### Phase 4 — les documents

- `docs/eval/mika-2680/README.md` : section **(g′)** — l'axe `outcome` existe,
  comment il se lira une fois le vol 2 livré, et l'avertissement de contrôle
  positif (une fenêtre antérieure au déploiement ne porte aucune ligne de
  sortie ; c'est un binaire, pas une absence d'issue — motif mika#2340). AC6 est
  **servi ici en partie** : le protocole dit ce qui sera chiffré ; la commande
  qui le chiffre arrive au vol 2.
- `crates/mika-agent/CLAUDE.md` : trois phrases dans l'entrée voisine du
  registre — le second terme, son nom d'événement, et la lecture de son absence.

---

## Ce que ce vol ne touche pas

- **`skip_turn`, `decide`, `classify`, `Release`, les classes (a) à (h)** : zéro
  ligne. Le site d'émission est après `run_agent` ; il n'a aucun chemin vers la
  porte (AC5).
- **`scripts/measure-webhook-turn-cost` et son suite de test** : la jointure, le
  seau `by_agent_trigger_release_outcome`, le compte à part des tours sans ligne
  de sortie, et le corpus mêlant `state_unreadable` et un arrêt propre. Vol 2 —
  AC3, AC4, et la seconde moitié d'AC6. **Tracked in: senara-solutions/mika#2855
  (vol 2)**. Le vol 1 est mergeable et utile seul : la ligne est dans le journal
  et un `grep -F webhook_turn_outcome` la lit ; seule la jointure automatique
  manque. C'est exactement la séquence que mika#2680 a elle-même suivie — vol 1
  le registre, vol 2 le lecteur.
- **Le filtre de (g′)** : hors de ce ticket, et explicitement hors de ce vol. Les
  trois arbitrages nommés sur #2680 (le numéro de PR sous le PAT du relecteur,
  les deux sites de vérité du dedup, le taux de `StateUnreadable`) attendent la
  mesure.
- **L'armement de la garde #864** : refusé pour ce vol, décision 2, et
  conditionné à ce que `outcome_absent` dira.

---

## Definition of Done

- [ ] `webhook_turn_outcome.rs` existe, `parse_attestation` est pure, et
      `emit_turn_outcome` est le seul écrivain de `WEBHOOK_TURN_OUTCOME_EVENT`.
- [ ] Le site d'émission est unique, placé entre `run_agent` et le `match run`,
      et couvre `Ok` comme `Err`.
- [ ] Les sept tests de la phase 1 et le test d'émission de la phase 2 passent.
- [ ] Le prompt du skill porte la section d'attestation et ses six valeurs ;
      le test de parité bidirectionnelle passe sur le prompt **embarqué**.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`,
      `scripts/verify-pipeline.sh origin/main` verts.
- [ ] `docs/eval/mika-2680/README.md` et `crates/mika-agent/CLAUDE.md` à jour ;
      `docs-sync` vert.
- [ ] **Non fait par cette PR, et c'est la chaîne, pas un oubli :** la ligne
      n'existe pour aucun agent avant `make deploy`. `skills/bundled/` est lu au
      build (`build.rs` → `BUNDLED_SKILL_MANIFESTS`), puis ensemencé
      (`seed_bundled_skills_if_needed`), puis lu au tour suivant. Sonde
      post-déploiement : `grep -F webhook_turn_outcome $MIKA_SPIRIT_LOG_FILE | head`
      doit rendre une ligne après le premier `check_suite` vert servi à mika-qa.
      Son **absence** signifie « binaire antérieur au vol », jamais « aucune
      issue » (motif mika#2340).

---

## Acceptance criteria

Transcrits du corps de mika#2855, avec la portée de chaque AC dans ce vol
nommée, et les raffinements de nom que le ticket délègue au groom (*« les noms
sont à fixer au groom »*) appliqués.

- **AC1** — chaque tour webhook facturé émet en fin de tour une ligne
  `webhook_turn_outcome` portant `agent_id`, `request_id` et `outcome`.
  `outcome` vaut `stop_no_open_pr`, `stop_out_of_scope`,
  `stop_already_reviewed_sha`, `stop_verdict_recorded`, `review`,
  `review_failed`, `outcome_absent` ou **`outcome_unreadable`** (huitième terme,
  raffinement argumenté en décision 3). Il y a un rouge pour chaque valeur.
  **Servi par ce vol.**
- **AC2** — l'issue est attestée. Un test montre qu'un tour qui n'atteste rien
  produit `outcome_absent`, et pas un arrêt deviné ; un second le montre pour un
  tour sans texte du tout. `Outcome::Absent` n'est atteignable par aucun texte
  (propriété du type, décision 3). **Servi par ce vol.**
- **AC3, réserve de Prime** — un tour relâché par `Release::StateUnreadable` se
  distingue d'un arrêt propre dans la jointure ; le croisement `release` ×
  `outcome` est lisible et pinné par un corpus mêlant les deux. **Vol 2** — les
  deux axes sont orthogonaux par construction et leurs deux lignes portent le
  même `request_id` ; la lisibilité du croisement est une propriété de
  l'agrégat, donc du lecteur. **Tracked in: senara-solutions/mika#2855 (vol 2)**.
- **AC4** — `scripts/measure-webhook-turn-cost` joint la ligne de sortie par
  `request_id` ; l'agrégat gagne `by_agent_trigger_release_outcome` ; les tours
  sans ligne de sortie sont comptés à part, sans être perdus ni fusionnés ;
  `scripts/test-measure-webhook-turn-cost.sh` le couvre. **Vol 2. Tracked in:
  senara-solutions/mika#2855 (vol 2)**.
- **AC5** — aucun tour n'est écarté, et le comportement du skill est inchangé
  hors l'attestation ; les classes (a) à (h) du pré-filtre restent intactes.
  **Servi par ce vol** : zéro ligne touchée dans `webhook_prefilter.rs`, site
  d'émission après `run_agent`, aucun `skill.toml` modifié.
- **AC6** — le protocole de mesure de #2680 dit comment la prochaine fenêtre
  chiffrera (g′) : la part des arrêts par numéro sur les 36 tours de la cellule.
  **Partagé** : ce vol écrit la section du protocole et nomme l'axe ; la commande
  qui produit le chiffre est au vol 2.

---

## Fire-Disposition

Ce plan livre **un** détecteur : le test de parité prompt ↔ enum de la décision 7
(`mika2855_le_prompt_embarque_et_lenum_ne_divergent_pas`). Son chemin de succès
est « aucune divergence trouvée », donc le gate s'applique.

**Option retenue : (a) exception nommée en allowlist — allowlist livrée vide.**

Détail d'implémentation :

```rust
/// Lignes du prompt où `Webhook-Outcome:` apparaît SANS être une valeur du
/// contrat. LIVRÉE VIDE : le prompt est écrit dans ce même vol, donc aucune
/// occurrence hors contrat n'existe au moment de l'atterrissage.
///
/// Toute entrée future doit nommer la ligne exacte, référencer son ticket de
/// suivi, et disparaître quand la ligne disparaît — l'assertion ci-dessous
/// rougit sur une entrée stale.
const PROMPT_PARITY_ALLOWLIST: &[&str] = &[];
```

L'assertion auto-nettoyante, dans le même test :

```rust
// Une exception qui ne correspond plus à aucune ligne du prompt est une
// exception stale : elle rougit ici plutôt que de rester à couvrir un cas
// disparu. C'est ce qui empêche cette allowlist de devenir celle que
// mika#2309 condamne.
for exc in PROMPT_PARITY_ALLOWLIST {
    assert!(prompt.contains(exc),
        "exception stale dans PROMPT_PARITY_ALLOWLIST : {exc:?} n'est plus \
         dans le prompt embarqué — retirer l'entrée");
}
```

Plus le contrôle de bonne foi, parce qu'une allowlist vide sur un détecteur qui
ne détecte rien est indiscernable d'un détecteur inerte : une assertion construit
un prompt synthétique portant `Webhook-Outcome: stop_inexistant` et vérifie que
le prédicat de parité le **refuse**. Sans elle, le détecteur passerait vert en
étant cassé.

Le détecteur atterrit **armé** — pas de `#[ignore]` : il n'a aucune violation à
absorber, et il tourne dans `cargo test`, donc dans le job `Check` de CI.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/webhook_turn_outcome.rs` (prod : deux enums, parse pur, émetteur, doc de module) | 140 |
| tests inline du module (7 tests, dont parité + contrôle de bonne foi) | 140 |
| `crates/mika-agent/src/server/handlers.rs` + `server/mod.rs` (site d'émission, entrée de module) | 15 |
| `crates/mika-agent/src/server/mod.rs` tests (émission sur le harnais existant) | 20 |
| `skills/bundled/qa-review-webhook-success/system_prompt.md` (section d'attestation) | 35 |
| `crates/mika-agent/CLAUDE.md` | 0 (hors compte) |

Total estimé : 350 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient, et pourquoi la suite en est un second.** 350 passe
sous les deux bornes (1000 par PR, 429 par vol) avec ~18 % de marge sur la plus
serrée. Le périmètre est resserré par trois refus explicites, chacun chiffré
plutôt que deviné :

- **le scan « seul écrivain » n'est pas livré** (décision 5, ~45 lignes
  économisées) : le voisin direct `WEBHOOK_TURN_CHARGED_EVENT` n'en a pas, et
  ouvrir un précédent n'est pas le travail d'un vol d'instrument ;
- **aucun nouveau harnais de capture `tracing`** (phase 2, ~30 lignes
  économisées) : `Mika2680Event` et `mika2680_events_named` existent déjà dans le
  module de test voisin ;
- **le lecteur part au vol 2** (~160 lignes : ~70 dans
  `measure-webhook-turn-cost`, ~90 dans son suite de test). Un seul vol à ~510
  dépasserait la borne, et le découpage suit la coupure que mika#2680 a elle-même
  prise entre son vol 1 (le registre) et son vol 2 (le lecteur) : l'émetteur est
  mergeable et observable seul, le lecteur n'a de sens qu'une fois la ligne dans
  le journal.

Tracked in: senara-solutions/mika#2855 (vol 2 — jointure du lecteur, seau croisé,
AC3/AC4 et la commande d'AC6)

---

## Fichiers touchés

| fichier | nature |
|---|---|
| `crates/mika-agent/src/server/webhook_turn_outcome.rs` | **nouveau** — termes, parse pur, émetteur unique |
| `crates/mika-agent/src/server/mod.rs` | entrée de module + un test d'émission |
| `crates/mika-agent/src/server/handlers.rs` | site d'émission unique, après `run_agent` |
| `skills/bundled/qa-review-webhook-success/system_prompt.md` | section d'attestation, six valeurs |
| `docs/eval/mika-2680/README.md` | section (g′) du protocole (AC6, première moitié) |
| `crates/mika-agent/CLAUDE.md` | le second terme du registre, et la lecture de son absence |

Non touchés, et c'est une propriété : `webhook_prefilter.rs`,
`qa-review-webhook-success/skill.toml`, `well_known_agents.rs`,
`scripts/measure-webhook-turn-cost`, `scripts/test-measure-webhook-turn-cost.sh`.
