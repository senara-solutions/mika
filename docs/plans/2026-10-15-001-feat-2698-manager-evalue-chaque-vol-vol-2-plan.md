# mika#2698 — vol 2 : l'interrupteur d'écriture et le geste unique

- **Ticket :** senara-solutions/mika#2698 (Phase 2 (b) du manager de milestone)
- **Branche :** `feat/2698/vol-2-interrupteur-ecriture`, empilée sur `feat/2698/mika-manager-phase-2-b-valuer-chaque-vol`
- **Vol précédent :** vol 1 livré en `e1c70f56`, PR #2704 OUVERTE sur `main`
- **Plafond de tours :** 200 (label `loop-substrate`, `PILOT_LABEL_TURN_CEILINGS`)
- **Borne par vol :** 431 lignes estimées — `(200 − 35 − 50) × 7,5 / 2`, règle `_plan_size_rule` de mika#2693
- **Estimation de ce vol :** 440 lignes hors `docs/`, dont 205 de tests (rapport tests/code ×0,87)

## Pourquoi ce plan existe

Le commentaire opérateur du 2026-10-14 sur le ticket constate trois choses sur
le vol 1 : le diff a pesé ×2,25 l'estimation (2 180 lignes contre 970), le plan
ne portait **ni `Plafond de tours :` ni `Borne par vol :`**, et la borne par
vol — en service depuis mika#2693 mais postérieure au groom — aurait imposé un
découpage plus fin que « vol 1 / vol 2 ». Ce plan porte les deux lignes en
tête, et il découpe le reliquat du vol 2 en **deux** vols pour tenir la borne.

## Découpage du reliquat

Le § *Découpage en vols* du plan du vol 1 attribuait au vol 2 cinq livrables.
À 431 lignes de borne, ils n'y tiennent pas ensemble — l'estimation honnête du
lot complet est d'environ 650 lignes. Le lot se coupe proprement en deux parce
que les deux moitiés ne se touchent pas : l'une écrit sur GitHub, l'autre lit un
entier de plus dans un callback.

| Vol | Livrables | ACs | Lignes estimées |
|---|---|---|---|
| **2 (celui-ci)** | `MIKA_MANAGER_EVALUATE_WRITE` (OFF par défaut), le geste `gh issue comment` unique, l'idempotence par checkpoint, l'exception nommée dans l'allowlist de `no_dispatch_test.rs` + contrôle négatif | **AC1** (forme nominale), **AC2** | 440 |
| **3** | la ligne `Turn ceiling: N` côté producteur (`dispatch-lib.sh`), sa lecture dans `turn_ceiling`, le rendu `N/plafond`, la déclaration dans `canonical-tokens.tsv` ; puis `last_commit_turn` lu du log de session pilote | — (complète le verdict du vol 1) | ≈ 230 |

AC3 a été livré en entier par le vol 1. Le vol 3 est nommé et chiffré ici ; il
sera ouvert comme sous-ticket avec un `Tracked in:` depuis la PR de ce vol.
`last_commit_turn` y va plutôt qu'ici parce que c'est la seule donnée du verdict
sans source structurée existante : elle demande de parser
`$PILOT_LOG_DIR/<task-id>.log`, soit un lecteur et ses fixtures.

## Ce que ce vol construit

### 1. `milestone_manager/verdict_poster.rs` (nouveau)

L'interrupteur et le geste, dans un fichier à eux pour que l'exception de
l'allowlist nomme **un** fichier et pas un répertoire.

- `ENV_EVALUATE_WRITE = "MIKA_MANAGER_EVALUATE_WRITE"`, et `write_is_armed()`
  son **lecteur unique** : vrai pour `1` / `true` (insensible à la casse),
  faux pour l'absence et pour toute autre valeur. OFF par défaut veut dire que
  l'absence est l'état normal, non un cas d'erreur.
- `render_verdict_comment(verdict) -> String` : le corps du commentaire. Il
  porte un marqueur HTML `<!-- mika-manager:verdict:<task_id> -->` en tête —
  non pas pour être relu (ce serait une lecture réseau de plus, hors du mandat
  « un seul geste »), mais pour qu'un doublon soit *reconnaissable* par
  l'opérateur si le checkpoint est perdu.
- `post_verdicts(armed, runner, repo, verdicts, already_posted) -> Vec<String>`
  rend les `task_id` nouvellement postés. `armed` est un **paramètre**, pas une
  lecture d'environnement : la fonction est alors déterministe et testable en
  parallèle, et il reste exactement un lecteur de la variable.
- Le geste est déclaré **à plat** :

  ```rust
  const GESTE: &str = "gh issue comment";
  ```

  et le vecteur d'arguments en est **dérivé** (`GESTE.split_whitespace()`
  moins le `gh`). Ce détour n'est pas de la coquetterie : le vol 1 a documenté
  que la forme éclatée `.args(["issue", "comment"])` **échappe** au grep plat du
  garde (`mika2698_le_scan_rougit_sur_une_source_fautive`). Un posteur écrit
  naïvement n'aurait donc besoin d'**aucune** exception, et l'entrée nommée que
  le ticket exige serait une fiction. En dérivant les arguments du jeton, ce qui
  s'exécute est exactement ce que le garde voit.

### 2. L'idempotence (`cadence.rs`)

- `MilestoneCheckpoint` gagne `posted_flight_task_ids: Vec<String>` avec
  `#[serde(default)]` — un checkpoint d'avant ce vol se relit en liste vide.
- La condition de sauvegarde passe de `should_deliver` à
  `should_deliver || !newly_posted.is_empty()`. Sans quoi un cycle qui poste
  sans livrer ne retiendrait rien et re-posterait au cycle suivant.
- Dans le cas « posté sans livrer », les trois champs existants sont
  **préservés** tels quels (digest, `last_delivered_at`, `last_observed_at`) :
  l'événement `state_changed` en attente ne doit pas être avalé par une
  sauvegarde faite pour une autre raison.

### 3. Le câblage, sans cascade de signatures

Le posteur a besoin d'un exécutant `gh`. Il y en a déjà un injectable dans le
module : `GhRunner` / `ProcessGhRunner` (mika#1975), utilisé par `reader.rs`.
On ajoute `CycleContext::with_gh_runner`, construit dans
`run_manager_cycle_with_auth` depuis `cfg.github_token`. Rien ne bouge dans
`server/mod.rs` ni dans `spawn.rs` — une injection de plus depuis le serveur
aurait coûté une vingtaine de lignes de plomberie pour aucune capacité de test
supplémentaire.

### 4. L'exception nommée (`no_dispatch_test.rs`)

```rust
const EXEMPTED_OCCURRENCES: &[(&str, &str, &str)] = &[(
    "verdict_poster.rs",
    "gh issue comment",
    "mika#2698 vol 2 — le seul geste d'écriture autorisé, derrière MIKA_MANAGER_EVALUATE_WRITE",
)];
```

Le test `mika2698_lallowlist_dexceptions_est_vide_et_le_reste` exige, dans son
propre message, de changer **en même temps** que la branche ON de
l'interrupteur et que le docstring du module. Il est donc réécrit dans ce même
commit : il épingle désormais **exactement une** entrée, pour **exactement** ce
fichier et ce jeton — ni `gh issue edit`, ni `gh pr comment`, ni une exception
par répertoire.

### 5. Le contrôle négatif de l'AC1

Un garde dont le succès est « rien trouvé » ne distingue pas « ça marche » de
« le code est mort » (classe mika#2205). Trois tests le tiennent :

- `write_is_armed()` est faux quand la variable est absente ;
- `armed = false` ⇒ **zéro** appel au runner (l'AC1 comportemental) ;
- `armed = true` ⇒ **exactement un** appel, et ses arguments commencent par
  `issue comment` — le contrôle négatif, sans lequel « zéro appel » serait
  satisfait par un posteur qui ne poste jamais.

## Acceptance criteria

Les deux critères de mika#2698 que ce vol porte (AC3 a été livré par le vol 1) :

- AC1 (forme nominale) : avec l'interrupteur `MIKA_MANAGER_EVALUATE_WRITE` OFF, aucun appel d'écriture `gh`. Garantie structurelle par l'extension de `no_dispatch_test.rs`, avec un contrôle négatif (§ 5).
- AC2 : avec l'interrupteur ON, un seul commentaire par vol, idempotent sur le rejeu du cycle (checkpoint `posted_flight_task_ids`).

## Contrat de vérification

`cargo test -p mika-agent milestone_manager`, `cargo test -p mika-agent perimeter`,
la suite complète `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check`, `scripts/verify-pipeline.sh origin/main`,
`make verify-bundled-skills`, et les lints du dépôt
(`check-canonical-tokens.sh`, `check-dispatch-seats-declared.sh`,
`check-pilot-turn-ceiling-labels.sh`).

Revue de code déléguée à mika-qa (mika#2694) : ni `ce-simplify-code` ni
`ce-code-review` dans ce vol.

## Ce que ce vol ne fait pas

- Il n'ouvre pas l'interrupteur. Le seuil d'ouverture est un geste opérateur
  (10 vols concordants, ou 7 jours) décrit par le ticket.
- Il ne lit aucun commentaire existant sur GitHub. Un seul geste, et c'est une
  écriture.
- Il ne ferme pas mika#2698 : le vol 3 reste dû.
