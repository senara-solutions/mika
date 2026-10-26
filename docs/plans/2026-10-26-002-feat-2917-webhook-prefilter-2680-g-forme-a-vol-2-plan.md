# mika#2917 — (g′) forme (a), vol 2 : le skill écrit son état de dedup, et il en est le seul écrivain

> Vol 2 sur 3. Parent : senara-solutions/mika#2680, classe (g′), bearing ratifié
> par Vincent le 2026-10-26 : **filtrer**, dans la forme (a).
>
> **Le vol 1 est mergé** (#2918, commit `4502306c`). Le texte d'un `check_suite`
> porte désormais sa tête, `mika_common::github_event_format` en déclare la
> grammaire et la parse, et `Candidate::GreenPrBranch` la transporte **sans que
> rien ne la lise**. Ce plan est le vol suivant, pas un re-mesurage du premier.

## Contexte

Sur 4,9 jours, 51 tours `check_suite_pr_branch` (mika-dev et mika-qa) se
terminent par `stop_verdict_recorded` (35) ou `stop_already_reviewed_sha` (16),
pour 7,06 M tokens d'entrée (#2680, commentaire 6410232454). Chacun paie un tour
complet pour conclure qu'il n'y a rien à faire.

Le vol 1 a levé l'obstacle matériel : le payload nomme maintenant la PR et sa
tête, en ligne 2 et 3 du texte transmis. Il manque la seconde moitié de la
comparaison — **ce que le skill sait déjà**. Aujourd'hui, « cette tête est
revue » n'existe nulle part que dans GitHub, et la seule façon de l'apprendre
depuis le moteur serait un appel `gh`, que le bearing refuse (mika#2260).

Ce vol pose cette moitié, et **rien d'autre** : le skill écrit ce qu'il a conclu,
à un endroit que le moteur peut lire sans `gh`, et **personne ne le lit encore**.

## Ce qui est refusé (bearing, non négociable)

- **Aucun appel `gh` depuis `webhook_prefilter.rs`**, sous quelque identité que
  ce soit (mika#2260). Ce vol n'en ajoute aucun, et il **pose la garde** qui
  recense ceux du fichier — avant le vol 3, qui est le diff qu'elle juge.
- **Aucune copie de la logique de dedup du skill dans le Rust** : un seul site de
  vérité. Le Rust de ce vol ne compare rien ; il transcrit une conclusion que le
  modèle a prise, dans un vocabulaire fermé.
- **Le moteur n'écrit jamais cette ligne.** L'écriture n'a lieu que comme effet
  direct d'un appel d'outil du modèle, et une garde refuse qu'un second site
  d'écriture apparaisse.

## Où ce vol se situe

| vol | périmètre | plancher (ce que sa PR tient) | état |
|---|---|---|---|
| **1** | Pièce 1 : le gateway transmet `head_sha` et `pull_requests[]`, `mika-common` en déclare la grammaire et le parse, `classify` la transporte, `decide` ne la lit pas. | Les textes anciens **et** nouveaux se lisent identiquement par les quatre lecteurs ; aucune porte ne change. | **mergé** (#2918) |
| **2 — ce vol** | Pièce 2 : la constante `QA_REVIEWED_HEAD_TOOL` et son vocabulaire fermé, l'outil `record_reviewed_head`, le prompt de `qa-review-webhook-success` qui l'appelle, la garde de l'écrivain unique, le recensement des sites `gh`. | L'état est écrit et **personne ne le lit** ; l'écrivain est unique et recensé ; les deux gardes sont **vertes sur `main` avant** le diff du vol 3. | AC3 (moitié garde) |
| **3** | Pièce 3 : la classe (g′), le terme de lecture du trait `PrefilterState`, le décideur, l'interrupteur dédié, le corpus rejoué. | AC1, AC2, AC3 verts ; AC4 mesurable après merge. | AC1, AC2, AC3, AC4 |

**Pourquoi la garde est à ce vol et non au suivant.** Une garde qui atterrit avec
le code qu'elle juge n'a jamais rien refusé : elle passe la revue, elle passe la
CI par construction, et le défaut qu'elle aurait attrapé atterrit en production.
Ce dépôt a écrit cette doctrine lui-même (`suite-wiring-lint`, mika#2578 une fois
puis mika#2752 deux fois). Les deux gardes doivent donc être vertes sur `main`
**avant** le diff qu'elles jugent — ce que « au vol 2 » garantit.

Tracked in: senara-solutions/mika#2917 — ce ticket porte aussi le vol 3 (« le
pré-filtre compare et ne recalcule rien »), qui est le dernier.

## L'endroit, choisi et justifié

Le ticket délègue au groom un choix et lui demande de l'écrire : *« il écrit une
ligne `(repo, PR, head_sha, issue)` à un endroit que le moteur peut lire sans
`gh`. Le groom choisit cet endroit et le justifie par écrit. Le moteur ne
l'écrit jamais. »*

**L'endroit retenu : une ligne d'`audit_events` sous un `tool_name` dédié, écrite
par un outil que le skill appelle, lue au vol 3 par un seul terme du trait
`PrefilterState`.**

```
tool_name   = QA_REVIEWED_HEAD_TOOL  = "qa_reviewed_head"   (SOLE WRITER déclaré)
target_key  = ci_success_handler::head_key(repo, pr, head_sha)  →  pr:{repo}#{n}@{sha}
after_value = l'issue du tour : "review" | "stop_already_reviewed_sha" | "stop_verdict_recorded"
reasoning   = "request_id=<uuid>"    (l'identité de l'événement, quand le skill la porte)
agent_id    = injecté par AsyncDatabase, jamais passé par l'appelant
```

Les quatre champs du ticket y sont au complet : `repo`, `PR` et `head_sha` dans
la clé, `issue` dans `after_value`.

**Pourquoi là.**

1. **Aucun `gh`.** Le pré-filtre lira la base SQLite de son propre agent, par les
   accesseurs qui existent déjà (`AsyncDatabase::count_recent_audit_events_for_target`,
   `get_audit_event_rows_by_tool_name`).
2. **Aucune table, aucune migration.** Le précédent est explicite et vit dans ce
   dépôt : `crates/mika-agent/src/qa_head_supersession.rs`, dont la doc de module
   dit *« Pas de table, pas de migration : la lecture passe par
   `count_recent_audit_events_for_target`, et le registre est **par agent** »*, et
   dont `SYNC_OBSERVED_TOOL` porte une déclaration **SOLE WRITER** en
   doc-comment. C'est le même mécanisme, au même titre.
3. **Par agent sans que l'appelant le dise.** `AsyncDatabase::log_audit_event`
   clone son propre `self.agent_id` et ne l'accepte **pas** en argument (son
   premier paramètre est le `session_id`) ; `count_recent_audit_events_for_target`
   fait de même en lecture. L'état de mika-qa ne peut donc pas écarter un tour de
   mika-dev, et la propriété ne dépend d'aucune discipline d'appel — c'est
   exactement la prémisse que le seau `skipped_by_agent_class` de
   `measure-webhook-turn-cost` a été ajouté pour vérifier (#2680 vol 5).
4. **Aucune grammaire de clé nouvelle.** `ci_success_handler::head_key` existe,
   rend `pr:{repo}#{n}@{sha}`, et le pré-filtre la relit déjà telle quelle
   (`decision_core_holds`, sur le préfixe `pr:{repo}#{n}@`). On la réemploie. Une
   forme de clé retapée dans un second fichier est la classe de divergence
   programmée que mika#2158 a payée et que mika#2683 évite déjà.
5. **La décision reste entière dans le skill.** Le moteur ne calcule jamais
   « cette tête est revue » : au vol 3 il comparera la tête que le payload nomme à
   une ligne que le skill a écrite. Aucune copie de la logique de dedup en Rust.
6. **Le moteur ne l'écrit jamais.** L'écriture Rust n'a lieu que dans
   `Tool::execute`, donc comme effet direct d'un appel d'outil du modèle, et la
   garde de § *Fire-Disposition* (a.2) refuse qu'un second site apparaisse.

**Ce qui a été écarté, et pourquoi.**

- **`store_fact` / `memory_facts`.** Aucun outil nouveau, mais l'état de dedup
  entre dans la mémoire sémantique et ressort dans les résultats de
  `search_memory` de toutes les questions sans rapport. `memory_facts` n'a par
  ailleurs aucune grammaire de clé `(repo, PR, tête)` : il faudrait la retaper,
  ce que (4) refuse.
- **Une table dédiée (schéma v56 → v57).** Une migration pour un registre dont le
  lecteur rendra un booléen, quand le précédent du dépôt dit explicitement « pas
  de table, pas de migration ».
- **Un fichier sous `~/.mika/state/`.** Ni transactionnel, ni par agent sans
  discipline d'appel, et le pré-filtre y gagnerait une lecture de système de
  fichiers qu'il n'a nulle part ailleurs.
- **Surcharger l'attestation `Webhook-Outcome:` de mika#2855 et la persister
  depuis `emit_turn_outcome`.** C'est la forme la moins chère en lignes, et elle
  est refusée sur le bearing, pas sur le coût : (i) le site d'écriture devient un
  site **du moteur**, ce que le ticket exclut en une phrase ; (ii) elle
  transforme une ligne d'observabilité en entrée de refus, qui est précisément la
  faute que ce dépôt interdit ailleurs par un scan de sources
  (`LABELED_BY_LINE_PREFIX` : *« Hard invariant — informational only. **No
  refusal decision reads this value.** »*) ; (iii) `emit_turn_outcome` tire sur
  **tous** les tours webhook de tous les agents, donc l'élargir élargirait une
  population que mika#2855 a bornée.
- **Un outil de skill à handler `exec`** (la forme de `qa_pr_view`). Un
  sous-processus shell ne peut écrire dans la base de l'agent qu'en ouvrant
  `mika.db` en direct : un second écrivain hors du type Rust, avec un `agent_id`
  re-dérivé — ce que (3) rend impossible par construction dans la forme retenue.
  Refusé pour la même raison que l'operational ledger interdit le SQL direct.

**Coût assumé du choix.** `record_reviewed_head` est un builtin de
`default_tools()`, donc sa définition pèse sur le prompt de **tous** les agents.
Deux atténuations, et la seconde est vérifiée et non supposée : la définition est
courte (trois chaînes et un `enum` fermé de trois valeurs), et
`MIKA_ARCH_ALLOWED_TOOLS` est une **allowlist fermée** dont le test de roster
(`mika2748_lallowlist_de_mika_arch_est_le_roster`) ne contraint que le sens
allowlist → `BUILTIN_TOOL_NAMES` — un builtin absent de l'allowlist n'est donc
pas servi à mika-arch, et l'outil n'entre pas dans le prompt du relecteur.

## La perte de revue, et ce qui l'empêche

C'est le seul risque matériel du mécanisme, et il se décide **ici** : le
vocabulaire que ce vol fige est ce que le vol 3 comparera. Quatre propriétés, et
aucune n'est optionnelle.

1. **L'écriture est postérieure à l'action.** Le prompt place l'appel **après**
   la revue dispatchée, ou après le stop constaté — jamais avant. Une ligne
   écrite d'avance serait une tête déclarée traitée par une revue qui n'a pas eu
   lieu.
2. **Trois issues sur six écrivent.** `review`, `stop_already_reviewed_sha` et
   `stop_verdict_recorded` écrivent ; `stop_no_open_pr`, `stop_out_of_scope` et
   `review_failed` **n'écrivent rien**. Les deux premières n'ont pas de PR
   nommable ou sont hors périmètre ; `review_failed` est le cas où la revue n'a
   **pas** abouti, et c'est précisément celui où le tour suivant doit passer.
   Le vocabulaire fermé de l'outil rejette les trois autres valeurs : une issue
   qui n'écrit pas n'est pas représentable comme une écriture.
3. **La clé est la tête, pas la PR.** Un nouveau push produit une nouvelle
   `head_sha`, donc une clé que le registre ne porte pas : le tour passe. C'est
   le contrôle négatif d'AC1, et il est structurel — pas une branche de code.
4. **Fail-safe dans le sens de la revue.** Une écriture manquée (outil absent,
   base en erreur, modèle qui oublie l'appel) laisse le registre sans la ligne,
   donc le vol 3 fera passer le tour : le régime d'aujourd'hui, un tour payé.
   C'est l'asymétrie qui décide le point suivant.

**Ce que `required_tools` ne fera pas.**
`qa-review-webhook-success/skill.toml` ne déclarera **pas**
`record_reviewed_head` dans `required_tools`. La raison est documentée dans ce
dépôt : `docs/solutions/architecture-patterns/required-tools-terminal-failure-bypass-2026-04-20.md`
et `docs/solutions/runtime-errors/mika-qa-duplicate-pr-review-required-tools-gate-2026-04-26.md`
— un `required_tools` non résolu fait échouer le skill de façon **terminale**.
Sur un handler de webhook, une écriture manquée coûte un tour payé (propriété 4
ci-dessus) tandis qu'un échec terminal coûte une revue perdue. L'asymétrie
décide.

**Le risque résiduel, nommé.** Une revue dispatchée qui meurt sans émettre
`review_failed` laisse une ligne `review` sur une tête non revue, et le vol 3
écartera les `check_suite` suivants de cette tête. Trois filets, dont aucun n'est
livré par ce vol : un nouveau push passe (propriété 3), le scan de
réconciliation de mika#2334 repasse sur une PR sans revue, et l'interrupteur
dédié du vol 3 désarme la classe entière. Ce vol n'arme aucune porte, donc le
risque n'est pas encore matérialisable — mais le vocabulaire qui le borne est
figé ici, et c'est pourquoi il est écrit ici.

## Requirements

### R1 — Un module de vérité pour le registre : `qa_reviewed_head.rs`

Nouveau fichier `crates/mika-agent/src/qa_reviewed_head.rs`, sur le modèle exact
de `qa_head_supersession.rs` : la constante, le vocabulaire fermé, et les gardes
du module en `#[cfg(test)]`.

```rust
/// `audit_events.tool_name` du registre des têtes qu'un tour de
/// `qa-review-webhook-success` a conclues (mika#2917, vol 2).
/// Cible [`crate::server::ci_success_handler::head_key`] ; `after_value` vaut
/// l'une des trois valeurs de [`REVIEWED_HEAD_OUTCOMES`].
/// **SOLE WRITER** : [`crate::tools::record_reviewed_head`], par appel d'outil
/// du modèle. Aucun chemin du moteur ne l'écrit.
/// **Lecteur** : aucun dans ce vol ; le pré-filtre au vol 3.
pub(crate) const QA_REVIEWED_HEAD_TOOL: &str = "qa_reviewed_head";

/// Les issues de mika#2855 qui attestent qu'une tête est **traitée**. Les trois
/// autres (`stop_no_open_pr`, `stop_out_of_scope`, `review_failed`) n'écrivent
/// rien, et ce vocabulaire fermé est ce qui rend leur écriture non
/// représentable — voir § *La perte de revue*.
pub(crate) const REVIEWED_HEAD_OUTCOMES: &[&str] =
    &["review", "stop_already_reviewed_sha", "stop_verdict_recorded"];

/// L'issue telle que l'attestation la porte. `None` sur toute autre valeur :
/// l'outil rend alors une erreur et n'écrit pas.
pub(crate) fn reviewed_head_outcome(raw: &str) -> Option<&'static str>;
```

- `reviewed_head_outcome` est **pure** : elle rend le littéral canonique de
  `REVIEWED_HEAD_OUTCOMES` et non l'entrée de l'appelant, pour qu'un `after_value`
  ne puisse pas porter une graphie que le lecteur du vol 3 ne reconnaîtrait pas
  (casse, espaces). Comparaison sur `trim()` + `to_ascii_lowercase()`.
- Le module ne retape **pas** la grammaire de clé : il réexporte
  `ci_success_handler::head_key` et une garde interdit le littéral `"pr:"` +
  `format!` dans le fichier.
- Pas de kill-switch dans ce vol : il n'y a aucune porte à désarmer. L'interrupteur
  de la classe (g′) arrive au vol 3, avec le décideur qu'il désarme — un
  interrupteur qui ne commande rien se lit comme une protection qui existe.

### R2 — L'outil `record_reviewed_head`

Nouveau fichier `crates/mika-agent/src/tools/record_reviewed_head.rs`, sur le
patron de `tools/record_served_content.rs`.

```rust
input_schema = {
  "repo":     { "type": "string",  "description": "owner/name, exactement comme le webhook le nomme" },
  "pr":       { "type": "integer", "description": "le numéro de la PR que le check_suite nomme" },
  "head_sha": { "type": "string",  "description": "la tête sur laquelle ce tour a conclu" },
  "outcome":  { "type": "string",  "enum": ["review", "stop_already_reviewed_sha", "stop_verdict_recorded"] },
  "required": ["repo", "pr", "head_sha", "outcome"]
}
```

`execute` :

1. Valide les quatre champs. `repo` vide, `pr` absent ou nul, `head_sha` vide,
   `outcome` hors `REVIEWED_HEAD_OUTCOMES` → `ToolOutput::error` **sans aucune
   écriture**, avec un `hint` nommant le champ fautif. Un refus est fail-safe
   (propriété 4 du § *La perte de revue*).
2. Construit la clé par `ci_success_handler::head_key(repo, pr, head_sha)`.
3. Appelle `ctx.db.log_audit_event(session_id, QA_REVIEWED_HEAD_TOOL, &key, None,
   Some(outcome), reasoning, trace_id)`. L'`agent_id` n'est **pas** passé —
   `AsyncDatabase` l'injecte (§ *L'endroit*, point 3).
4. Rend un `ToolOutput` qui porte la clé écrite, pour que la trace d'outil dise
   sur quelle tête le tour a conclu.

**Aucun `gh`, aucun réseau, aucun sous-processus.** L'outil ne vérifie pas que la
PR existe ni que la tête est bien la sienne : il transcrit une conclusion du
modèle, et vérifier la recalculerait — ce que le bearing refuse.

### R3 — L'enregistrement, aux trois sites qu'un builtin demande

- `crates/mika-agent/src/tools/mod.rs` : `pub mod record_reviewed_head;` (ou la
  forme de déclaration que le fichier emploie pour ses voisins),
  `registry.register(Box::new(record_reviewed_head::RecordReviewedHeadTool));`
  dans `default_tools()`, et `"record_reviewed_head"` dans `BUILTIN_TOOL_NAMES`.
- `crates/mika-agent/src/lib.rs` : `mod qa_reviewed_head;`.
- **`MIKA_ARCH_ALLOWED_TOOLS` n'est pas touchée.** Le test de roster ne contraint
  que le sens allowlist → builtins ; l'absence est ce qui garde l'outil hors du
  prompt de mika-arch (§ *Coût assumé du choix*).
- `test_builtin_tool_names_parity` est la garde qui refuse d'oublier l'un des
  trois sites. Elle doit rester verte **sans exception**.

### R4 — Le prompt du skill appelle l'outil, aux trois issues et après l'action

`skills/bundled/qa-review-webhook-success/system_prompt.md` gagne une section, et
**rien n'en perd** : les six étapes de la décision et l'attestation
`Webhook-Outcome:` de mika#2855 restent mot pour mot ce qu'elles sont.

La section dit trois choses, et sa forme est contrainte par le § *La perte de
revue* :

1. **Quand.** Après l'étape 3, l'étape 4 ou l'étape 5 — donc après que le stop
   est constaté ou que la revue est dispatchée, jamais avant.
2. **Quoi.** `record_reviewed_head(repo, pr, head_sha, outcome)` où `outcome` est
   la **même valeur** que celle de la ligne `Webhook-Outcome:` du tour. Une table
   de trois lignes le dit, et nomme explicitement les trois issues qui
   **n'appellent pas** l'outil.
3. **Si l'appel échoue.** Ne pas réessayer, ne pas escalader, ne pas changer
   l'issue du tour : la ligne manquante coûte un tour payé au prochain
   `check_suite`, ce qui est le régime d'aujourd'hui. L'appel ne décide rien de
   ce tour-ci.

**Pourquoi le prompt et non un hook de post-action.** `post_action_hooks` existe,
mais un hook est un site **du moteur** : il écrirait la ligne sans appel d'outil
du modèle, ce que le ticket exclut en une phrase (« Le moteur ne l'écrit
jamais »), et il devrait deviner l'issue à partir de la trace — or les étapes 1 et
2 laissent la même trace, comme 3 et 4, ce que mika#2855 a déjà mesuré et écrit.

### R5 — Un seul prédicat d'écrivain, promu dans `source_scan`

La garde de § *Fire-Disposition* (a.2) a besoin du prédicat « quels fichiers de
production passent ce nom à `log_audit_event` ». Ce prédicat **existe déjà**,
inline dans le `mod tests` de `qa_head_supersession.rs` (`audit_writers`, fenêtre
de 4 lignes après l'ouverture de l'appel). Il est promu dans
`crates/mika-agent/src/source_scan.rs` à côté de `production_half`, `fn_bodies` et
`is_test_source_path`, et `qa_head_supersession` l'appelle au lieu de le porter.

Le recopier aurait été deux prédicats pour une question, dans deux fichiers, avec
deux fenêtres à faire diverger — la classe exacte que mika#2158 a payée. La
promotion **réduit** le total de lignes du vol, et c'est un effet secondaire, pas
la raison.

## Non-Requirements (hors de ce vol, délibérément)

- **La lecture.** Aucun terme du trait `PrefilterState` n'est ajouté, et aucun
  fichier du moteur ne nomme `QA_REVIEWED_HEAD_TOOL` hors de son module de vérité
  et de l'outil. Vol 3.
- **La comparaison, la classe (g′), le décideur, l'interrupteur dédié, le corpus
  rejoué.** Vol 3. Déclarer une `PrefilterClass` qu'aucun site ne peut atteindre
  ferait entrer dans `skipped_by_class` un seau qui ne se remplirait jamais, et un
  seau vide se lit comme « cette classe n'écarte rien » (mika#2205).
- **`measure-webhook-turn-cost`.** Le script n'est pas touché : `skipped_by_class`
  groupe dynamiquement sur `.class`, donc la classe du vol 3 y apparaîtra sans
  modification (AC4).
- **Les arrêts 1 et 2** (`stop_no_open_pr`, `stop_out_of_scope` : 4 tours,
  0,64 M). Hors flight par le ticket — et ici, hors du vocabulaire fermé.
- **La forme (b).** À n'ouvrir que si (a) échoue au groom.
- **Le retrait du dernier appel `gh` de `webhook_prefilter.rs`.** C'est
  l'achèvement de mika#2260, pas de ce vol. L'exception nommée de
  § *Fire-Disposition* (a.1) le référence.
- **Le filage de `head_sha` jusqu'à la clé de coalescence de la file v2.**
  `webhook_queue_v2` documente ce manque comme *« correctness-critical »* et le
  renvoie à un suivi ; le vol 1 l'a rendu réalisable et ne l'a pas réalisé. Ce
  vol non plus : `coalescing_key` reste `check_suite:{repo}:{branch}`.
  Tracked in: senara-solutions/mika#2680.

## Verification Contract

1. `cargo test -p mika-agent qa_reviewed_head` — le vocabulaire fermé, le rejet
   d'une graphie hors vocabulaire, la canonicalisation (`" REVIEW "` → `"review"`),
   et les deux gardes de § *Fire-Disposition* avec leurs contrôles négatifs et
   leurs anti-vacuités.
2. `cargo test -p mika-agent record_reviewed_head` — le schéma (quatre champs
   requis, `enum` fermé), les quatre refus de validation **sans écriture**, et la
   clé écrite égale à `head_key(repo, pr, head_sha)`.
3. `cargo test -p mika-agent tools::tests::test_builtin_tool_names_parity` — les
   trois sites d'enregistrement sont cohérents.
4. `cargo test -p mika-agent qa_head_supersession` — les gardes de mika#2671
   restent vertes **après** la promotion du prédicat de R5. C'est la moitié
   non-régression de ce requirement.
5. `cargo test -p mika-agent well_known_agents` — `mika2748_...est_le_roster`
   reste vert sans que l'allowlist de mika-arch soit touchée.
6. `make verify-bundled-skills` — le bundle `qa-review-webhook-success` reste
   structurellement complet après l'édition de son prompt, et son `skill.toml`
   ne gagne **aucun** `required_tools` (§ *La perte de revue*). Le gate doit
   rester vert sans exception.
7. `cargo test` (suite complète), `cargo clippy`, `cargo fmt --check`.
8. `scripts/check-byte-slices.sh` — aucun découpage d'octets sur `&str` n'est
   introduit (un `sha` est de l'ASCII, un `repo` ne l'est pas forcément).
9. `scripts/verify-pipeline.sh origin/main`.
10. **`suite-wiring-lint` : N/A, et c'est vérifiable.** Les livrables de test de
    ce vol sont des `#[cfg(test)] mod tests` compilés dans la crate, atteints par
    le `cargo test` que la CI lance. `scripts/check-suite-wiring.sh` ne juge que
    les suites présentes sous `scripts/` ou `skills/bundled/_shared/` ; ce vol
    n'en crée aucune, donc `scripts/suite-wiring.tsv` ne gagne **aucune** entrée
    et son `unwired=0` ne doit pas bouger.

## Definition of Done

- `QA_REVIEWED_HEAD_TOOL` et `REVIEWED_HEAD_OUTCOMES` vivent dans
  `crates/mika-agent/src/qa_reviewed_head.rs`, avec leur déclaration **SOLE
  WRITER** en doc-comment, et aucun autre fichier n'écrit leur littéral.
- `record_reviewed_head` est enregistré aux trois sites qu'un builtin demande, et
  `test_builtin_tool_names_parity` est vert.
- L'outil écrit **une** ligne `audit_events` par appel valide, ciblée par
  `ci_success_handler::head_key` — aucune clé retapée.
- Les quatre refus de validation n'écrivent rien, et chacun nomme son champ.
- `MIKA_ARCH_ALLOWED_TOOLS` n'est pas touchée, et l'outil n'est donc pas servi à
  mika-arch.
- Le prompt de `qa-review-webhook-success` appelle l'outil aux trois issues qui
  écrivent, **après** l'action, et nomme explicitement les trois qui n'écrivent
  pas. Les six étapes et l'attestation mika#2855 sont inchangées.
- `skill.toml` ne déclare pas `record_reviewed_head` en `required_tools`.
- Le prédicat d'écrivain a **un seul** site (`source_scan`), et les gardes de
  mika#2671 l'appellent.
- **Aucun lecteur.** Aucun terme de `PrefilterState`, aucune `PrefilterClass`,
  aucun terme `Release`, aucune migration de schéma, aucun appel `gh` ajoutés.
- Les deux gardes de § *Fire-Disposition* sont vertes sur `main` avant le vol 3.
- Le § *Verification Contract* passe en entier.

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2917. **AC1, AC2 et AC4
sont servis par le vol 3** ; ce vol sert la moitié garde d'AC3 et pose l'état que
les trois autres compareront. Son contrat de PR est le § *Definition of Done* et
la colonne « plancher » du § *Où ce vol se situe*.

- **AC1** : sur un corpus rejoué, chaque tour qui se serait terminé en
  `stop_verdict_recorded` ou `stop_already_reviewed_sha` est écarté, et **aucun**
  tour terminé en `review` ne l'est. Le contrôle négatif, une nouvelle tête sur
  une PR déjà revue, doit passer, dans le même test.
- **AC2** : `pull_requests` vide, plusieurs PR, état absent ou illisible : le
  tour passe. Un test par cas.
- **AC3** : le diff ne contient aucun appel `gh` dans `webhook_prefilter.rs`, et
  le Rust ne lit qu'une seule source pour l'état de dedup. Une garde de test
  l'impose.
- **AC4** : après déploiement, `measure-webhook-turn-cost` montre la classe
  nouvelle dans `skipped_by_class`. Les issues `stop_verdict_recorded` et
  `stop_already_reviewed_sha` des tours payés baissent, et `review` ne baisse
  pas. Cette mesure se fait après le merge et ne bloque pas la PR.

**Où chaque AC atterrit.**

| AC | vol | comment il est tenu |
|---|---|---|
| AC1 | 3 | Rejeu **ordonné** d'un corpus (l'état de chaque tour est l'accumulation de ce que les tours précédents ont écrit, jamais un état pré-rempli), plus le contrôle négatif dans le même test, plus une anti-vacuité : le corpus porte au moins un tour de chacune des trois issues. |
| AC2 | 3 | Un test par cas sur l'état bouchonné : `Empty`, `Many`, ligne absente, lecture en erreur. Les quatre rendent le tour complet. |
| AC3 | **2 (les deux gardes)** + 3 (le diff qu'elles jugent) | § *Fire-Disposition* (a.1) recense les sites `gh` de `webhook_prefilter.rs` à exception nommée ; (a.2) impose l'écrivain unique et le lecteur unique de `QA_REVIEWED_HEAD_TOOL`. Les deux sont vertes sur `main` avant le vol 3. |
| AC4 | 3, mesuré après merge | `skipped_by_class` groupe dynamiquement sur `.class`, donc la classe nouvelle y apparaît sans modification du script, et `skipped_by_agent_class` la ventile par agent. La moitié « les issues payées baissent » se lit sur `webhook_turn_outcome`, joint par `request_id`. |

## Fire-Disposition

Ce vol livre des détecteurs — leur chemin de succès est « aucune violation
trouvée » : les deux recensements de sources ci-dessous, et les tests de
validation de l'outil.

**Option retenue : (a), exception nommée en allowlist.**

### (a.1) Le recensement des sites `gh` de `webhook_prefilter.rs` — une exception

Le scan lit la moitié production du fichier (`source_scan::production_half`),
énumère ses corps de fonctions (`source_scan::fn_bodies`), et rend l'ensemble de
ceux qui nomment un jeton d'appel `gh` (`run_gh_subprocess`, `run_gh`, le littéral
`"gh"`). **Il existe une violation aujourd'hui**, donc le détecteur atterrit avec
une entrée et une seule :

```rust
/// Les sites de `webhook_prefilter.rs` qui appellent `gh`. Une entrée par site,
/// nommant la donnée précise qu'il lit et le ticket qui le retire.
///
/// - `pr_state` — lit le seul champ `state` d'une PR (`gh pr view --json state`,
///   via `tools::pr_merge_with_gate::run_gh_subprocess`) pour la classe (c)
///   `VerdictPrClosed`. **Antérieur à mika#2917** et hors du chemin (g′) : aucun
///   décideur de `GreenPrBranch` ne l'atteint. Son retrait est l'achèvement de
///   mika#2260, suivi par senara-solutions/mika#2260.
const GH_CALL_SITES_ALLOWED: &[&str] = &["pr_state"];
```

Trois propriétés, et aucune n'est optionnelle :

- **L'assertion est auto-nettoyante.** Elle compare l'**ensemble exact** des
  sites trouvés à `GH_CALL_SITES_ALLOWED` — pas un `contains`, pas un `>=`. Le
  jour où `pr_state` cesse d'appeler `gh`, l'entrée devient stale et le test
  rougit en nommant l'entrée à retirer. C'est le geste qui rend l'exception
  inutile qui la fait expirer — ce qui est ce qui empêche cette allowlist de
  devenir celle que mika#2309 condamne.
- **Contrôle négatif.** Le scan tourne sur une source fabriquée portant un appel
  `gh` dans une fonction hors allowlist, et doit le trouver. Sans lui, « le scan
  n'a rien trouvé » est indistinguable de « le scan n'a rien lu ».
- **Anti-vacuité.** Le test asserte que le scan a bien lu le fichier réel et y a
  énuméré au moins N corps de fonctions (mika#2205 : « zéro violation » sur un
  fichier non lu n'est pas une mesure).

### (a.2) L'écrivain unique et le lecteur unique — allowlist **vide**

Deux assertions, et elles n'ont pas la même forme, parce qu'elles ne mesurent pas
la même chose.

- **L'écrivain, ensemble exact.** Les fichiers de production sous
  `crates/mika-agent/src/` qui passent `QA_REVIEWED_HEAD_TOOL` à
  `log_audit_event` doivent être exactement `{ tools/record_reviewed_head.rs }`.
  C'est l'invariant du bearing — « le moteur ne l'écrit jamais » — et il est
  **stable à travers les vols 2 et 3** : le vol 3 ajoute un lecteur, pas un
  écrivain. Le prédicat est celui promu en R5, donc le même qui tient déjà
  mika#2671.
- **Le lecteur, borné à un.** Le nombre de fichiers de production qui nomment la
  constante **sans** l'écrire doit être **≤ 1**. Au vol 2 il vaut 0 ; au vol 3 il
  vaudra 1 (`server/webhook_prefilter.rs`). La forme est un `≤` et non un
  ensemble exact **délibérément** : un ensemble exact rougirait au vol 3 par
  construction, et un rouge attendu s'apprend à ignorer. Un `≤ 1` mord sur le seul
  défaut qu'AC3 nomme — « le Rust ne lit qu'une seule source » — et il mord aux
  deux vols.
- **Aucune exception, et l'allowlist reste vide.** Quand l'une des deux rougit, la
  résolution est de router l'accès par le module de vérité, jamais d'ajouter une
  entrée. Le contrôle négatif (une source fabriquée à deux écrivains, une autre à
  deux lecteurs) et l'anti-vacuité (le scan a lu N fichiers) sont les mêmes qu'en
  (a.1).
- **Aucun littéral recopié.** Comme la garde de mika#2671 le fait déjà pour ses
  trois noms, un balayage asserte qu'aucun fichier de production hors
  `qa_reviewed_head.rs` ne contient le littéral `"qa_reviewed_head"`.

### (a.3) Le reste des détecteurs de ce vol — armés, sans exception

Les tests de validation de l'outil (quatre refus sans écriture), la conformité du
prompt (il nomme l'outil aux trois issues qui écrivent et pas aux trois autres) et
la non-régression des gardes de mika#2671 après la promotion de R5 atterrissent
verts, sans allowlist : il n'existe aucune violation à excepter, et en créer une
vide serait une entrée inerte qui se lit comme une autorisation.

**Aucun détecteur n'est livré désarmé** (option b), et rien ne remonte à
l'opérateur (option c).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/qa_reviewed_head.rs` (constante, `REVIEWED_HEAD_OUTCOMES`, `reviewed_head_outcome`, doc de module) | 80 |
| `crates/mika-agent/src/qa_reviewed_head.rs` — tests (les deux gardes, deux contrôles négatifs, deux anti-vacuités, le balayage de littéraux, la conformité du prompt) | 120 |
| `crates/mika-agent/src/tools/record_reviewed_head.rs` (définition, `execute`, les quatre refus) | 95 |
| `crates/mika-agent/src/tools/record_reviewed_head.rs` — tests (schéma, refus sans écriture, clé écrite) | 45 |
| `crates/mika-agent/src/source_scan.rs` + `qa_head_supersession.rs` (prédicat d'écrivain promu, un seul site — net de la suppression inline) | 10 |
| `crates/mika-agent/src/tools/mod.rs` + `lib.rs` (`mod`, `register`, `BUILTIN_TOOL_NAMES`) | 6 |
| `skills/bundled/qa-review-webhook-success/system_prompt.md` (la section d'appel et sa table des six issues) | 25 |

Total estimé : 381 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient ici.** 381 < 429, avec 48 lignes de marge, et
381 < 1000. Les trois pièces de la forme (a) totalisaient ≈ 1 150 lignes et ne
tenaient dans aucune des deux bornes : le découpage en trois vols
indépendamment mergeables a été arrêté au vol 1 et tient toujours — le vol 1 a
coûté 365 estimées et a mergé en un pilote. Le vol 3 est nommé par la ligne
`Tracked in:` du § *Où ce vol se situe*. Le vol courant est le seul jugé ici.

**Ce qui tient la marge.** Le vol n'ajoute aucun lecteur, aucun décideur, aucun
terme `Release`, aucune `PrefilterClass`, aucune migration, et ne touche pas
`measure-webhook-turn-cost`. Trois postes de fin de pipeline sont absents : pas de
schéma à migrer, pas de `MIKA_ARCH_ALLOWED_TOOLS` à rouvrir, et le prédicat
d'écrivain est promu plutôt que réécrit. Un poste est présent et compté :
`make verify-bundled-skills` tourne, parce que ce vol édite un prompt de bundle —
mais il n'ajoute ni skill, ni `required_tools`, ni entrée d'allowlist d'identité,
donc le gate n'a rien de nouveau à résoudre.

## Ordre de commit

1. `source_scan` : le prédicat d'écrivain promu, et `qa_head_supersession` qui
   l'appelle au lieu de le porter. **Les gardes de mika#2671 restent vertes** —
   c'est un commit de non-régression pure, sans aucun livrable du vol.
2. `qa_reviewed_head.rs` : la constante, le vocabulaire fermé,
   `reviewed_head_outcome`, et les deux gardes de § *Fire-Disposition*. **Compile
   et passe seul**, sans écrivain : la garde (a.2) y asserte un écrivain vide,
   et c'est le seul instant du vol où elle le fait.
3. `tools/record_reviewed_head.rs` + les trois sites d'enregistrement. La garde
   (a.2) passe à `{ tools/record_reviewed_head.rs }` **dans le même commit** que
   l'écrivain qu'elle nomme — c'est l'unique mise à jour d'attendu du vol, et
   elle est locale à un commit.
4. Le prompt de `qa-review-webhook-success` et son test de conformité. **À partir
   d'ici l'état est écrit en production**, et rien ne le lit.

Chaque étape laisse l'arbre vert. L'étape 1 ne livre rien et protège mika#2671 ;
l'étape 4 est celle qui change le comportement d'un agent.

## Sonde post-déploiement (vol 2)

Ce vol n'arme aucune porte, donc il n'a pas de halte. Une seule lecture, et elle
sert au vol 3 : après `make deploy` — et un `make deploy` est **obligatoire** ici,
parce qu'un prompt de `skills/bundled/` n'atteint un agent que par cette
reconstruction (mika#2340) — sur un `check_suite` success réel d'une branche de
PR routé à mika-qa, une ligne `audit_events` doit apparaître sous
`tool_name = 'qa_reviewed_head'`, avec une cible `pr:{repo}#{n}@{sha}` et un
`after_value` égal à la ligne `Webhook-Outcome:` du même tour.

**Trois lectures, et elles ne disent pas la même chose.** Aucune ligne alors que
des tours tournent veut dire un prompt non déployé (classe mika#2340) — la sonde
est `cat ~/.mika/skills/.manifest-writer`, pas une relecture du registre. Des
lignes dont l'`after_value` diverge de l'attestation du tour veut dire un prompt
déployé mais mal suivi. Des lignes conformes veut dire que le vol 3 a sa seconde
moitié de comparaison, et qu'il peut partir.
