# mika#2808 — Le `target/` d'un worktree est purgé quand son dernier consommateur a fini

> **Suivi de mika#2497** (purge du `target/` d'un worktree vif mais inactif), conservé
> entier. Ce plan ne retire ni n'affaiblit aucun de ses cinq termes.

## Le défaut, mesuré

Le 2026-10-19 à 00:00Z, `/data` est monté de 27 % à 42 % en trois heures — ~5 points
par heure — aucun pilote en vol. `.claude/worktrees` pesait 268 G pour 35 worktrees,
dont neuf `target/` de 22 à 47 G laissés par les vols et les builds QA du 18/10. Un
`cargo clean` manuel sur ces neuf a ramené `/data` à 14 %.

## Recadrage du constat — deux mécanismes portent le nom de mika#2497, et c'est le second qui compte

Le corps du ticket attribue la récupération de `target/` à
`scripts/mika-platform-worktree-cleanup` (seuils `DISK_PRESSURE_PCT` 75 %,
`STALE_DAYS` 14). Ce script existe — son enveloppe de commande est lisible dans ce
worktree (`.claude/commands/mika-platform-worktree-cleanup.md`, lignes 43-44) — mais il
vit dans le méta-dépôt `mika-platform`, **structurellement hors d'atteinte du pilote**
(le bac à sable de dispatch ne matérialise que le sous-dépôt `mika/` ; vérifié en
listant `/data/workspace/mika-platform/`, qui ne contient aucun `scripts/`). Ce n'est
pas lui que mika#2497 a livré.

**mika#2497 a atterri dans ce dépôt**, comme un troisième bras du tick
`worktree_reap` : `purge_stale_target_dirs` dans
`crates/mika-agent/src/worktree_reaper.rs`, cadence `0 */10 * * * *`, armé par défaut,
cinq termes P1–P5, étendu à neuf motifs d'éligibilité par mika#2619. Ses seuils ne sont
ni 75 % ni 14 jours : **aucun seuil de pression disque**, et une fenêtre d'inactivité
de **4 h** (`MIKA_TARGET_PURGE_IDLE_SECS = 14400`).

Ce recadrage change la question du ticket. Le défaut n'est pas « un mécanisme de
pression ne se déclenche qu'à 75 % » : c'est que **la fenêtre de 4 h est le seul signal
de fin de consommation dont le bras dispose**, et qu'en régime de plusieurs vols par
jour cette fenêtre est l'incident à elle seule. Neuf worktrees × 20–50 G retenus 4 h
après leur dernier build, c'est exactement les 268 G mesurés.

## La question du ticket, tranchée : le point d'accroche est le **verdict QA**

Le ticket laisse trois candidats et interdit de supposer : *sortie du build QA, verdict
QA ou fin de vol sans PR*. Il énonce aussi sa propre lecture — « le dernier consommateur
est le build QA (`long_running:build_mika`) ». **La lecture du prompt de revue
contredit cette lecture, et c'est le fait qui tranche.**

`skills/bundled/qa-review/system_prompt.md` ligne 420, vérification d'un critère
d'acceptation **comportemental** :

> **Behavioral** — invoke `build_mika` (Step 3e.3 path) to compile the worktree, then
> run the AC's commands against `<worktree>/target/release/mika ...`

Le binaire produit par le build est donc lu **après** l'arrivée du callback, à l'étape
3e.4, et le verdict n'est posté qu'à l'étape 5 (`run_gh pr review`, lignes 654-666).
L'ordre est verrouillé par le prompt lui-même : ligne 46, *« If `build_mika` was called
and the callback has NOT yet arrived, you MUST NOT proceed to Steps 4 or 5 »*.

Conséquence : **purger à la sortie du handler de build casserait la vérification des AC
comportementaux** — le `target/release/mika` que la revue est sur le point d'invoquer
n'existerait plus. La sortie du build n'est pas la fin de la consommation ; la revue
postée l'est.

**Décision : le point d'accroche est le verdict QA, et la fin de vol sans PR.** Les deux
branches de l'AC1/AC2, et aucune troisième.

## Pourquoi un **état lu** et non un **signal émis** — la seconde décision du plan

Le réflexe serait d'émettre un signal au site du verdict et un autre en fin de vol. Deux
raisons le refusent, et la seconde est dirimante.

**1. Deux écrivains pour une garde, c'est la classe de divergence que ce dépôt referme
en boucle.** Le site du verdict est en Rust ; le seul site qui voit une fin de vol est
`dispatch-lib.sh`, en shell. Chacun devrait porter sa propre copie de la garde `/proc`
de l'AC3 — deux lecteurs d'une même question, qui répondront différemment le jour où
l'un bouge (mika#2158, deux regex de grooming divergentes pendant des mois).

**2. Un signal shell a un trou **exactement** sur la clause « quelle qu'en soit la
raison ».** `dispatch-lib`'s post-flight ne tourne pas quand le groupe de processus du
pilote est tué — la supersession tue le pgid (mika#2263), `dispatch-lib` inclus. Un
AC2 fondé sur un signal émis serait muet sur la mort violente, c'est-à-dire sur la
population même que le ticket nomme.

**Le remède : le bras de purge lit l'état, au tick, depuis la source de vérité.** « Le
dernier consommateur a fini » n'est pas un événement à graver, c'est un **état
observable** :

| motif de conservation du faucheur | ce que l'état dit | fenêtre |
|---|---|---|
| `pr_open` **et** la branche porte ≥ 1 revue soumise | la QA a rendu son verdict (AC1) | courte |
| `pr_unknown` | aucune PR pour cette branche ⇒ aucune QA ne viendra (AC2) | courte |
| `detached_head_pr_unknown` | aucune PR à ce SHA ⇒ idem (AC2) | courte |
| `pr_open` sans revue | un build QA peut être en vol | **4 h, inchangée** |
| les six autres motifs éligibles | la question n'est pas répondable | **4 h, inchangée** |
| liste de revues illisible, ou `branch: None` | terme illisible ⇒ conserve | **4 h, inchangée** |

Un pilote tué violemment laisse son worktree sans PR ou avec une PR sans revue : le
premier cas tombe dans la ligne 2 (purgé), le second attend 4 h comme aujourd'hui. Le
trou du signal émis est fermé par construction, parce que rien n'a eu à être émis.

### `latestReviews`, et pourquoi pas `reviewDecision`

Les verdicts bloquants de mika-qa sont postés en **`--comment`**, pas en
`--request-changes` (`qa-review/system_prompt.md`, table des lignes 662-666 :
`hold[review]`, `block[ac]`, `block[dependency]`, `block` → Comment ; seul `pass` →
Approve). Une revue `COMMENT` laisse `reviewDecision` à `REVIEW_REQUIRED`.
**`reviewDecision` serait aveugle précisément à la population qui retient un worktree le
plus longtemps.** Le champ correct est `latestReviews`, non vide.

### Le champ voyage par un **second appel étroit**, et c'est une décision de sûreté

Mesuré sur `senara-solutions/mika` depuis ce worktree, le 2026-10-19 :

| appel | taille | latence |
|---|---|---|
| l'appel actuel du faucheur (`--state all --limit 300`, 6 champs) | 73 Ko | **2,1 s** |
| le même, `latestReviews` ajouté | 1,21 Mo | **7,05 s** |
| un second appel étroit (`--state open`, `number,headRefName,latestReviews`) | 57 Ko | **0,79 s** |

Enrichir l'appel du faucheur multiplierait sa latence par 3,4 sur un appel dont l'échec
**saute le dépôt entier pour le tick** (`worktree_reap_failed stage=pr_list`), faucheur
compris, avec `GH_TIMEOUT` à 30 s. Un second appel étroit coûte 0,79 s, et son échec ne
dégrade **que** le bras de purge — qui retombe alors sur la fenêtre de 4 h, c'est-à-dire
sur le comportement d'aujourd'hui.

## Surface retenue

`crates/mika-agent/src/worktree_reaper.rs`, dans le bras existant. **Aucun nouveau
`PeriodicScan`, aucune migration, aucun nouveau `tool_name`, aucun terme P1–P5 touché.**
Le plan ajoute **une fenêtre**, jamais une garde — et c'est ce qui satisfait l'AC3 par
héritage : la garde `/proc` (P3), la garde de chemin (P1 + `canonical_path_is_managed`),
la sonde de verrou cargo (P5) et le périmètre « `target/` seulement, jamais l'arbre de
travail » sont ceux de mika#2497, inchangés.

### Ancrages vérifiés dans l'arbre — ce plan n'invente aucune API

| ce que le plan prescrit | symbole | site |
|---|---|---|
| le bras hôte | `purge_stale_target_dirs` | `worktree_reaper.rs:4280` |
| **l'index des PR est déjà un paramètre du bras** | `prs: &PrIndex` | `worktree_reaper.rs:4286` |
| le prédicat P4 à paramétrer | `idle_secs < cfg.idle_secs` | `worktree_reaper.rs:3555` |
| la config à étendre | `TargetPurgeConfig` | `worktree_reaper.rs:3261` |
| le parse maison à trois paliers | `parse_positive_i64` | `worktree_reaper.rs:581` |
| l'appel `gh` à dupliquer en étroit | `list_prs` | `worktree_reaper.rs:1523` |
| le patron « absence ⇒ conserve » | `states` / `TargetState` | `worktree_reaper.rs:3535` |
| le patron « champ additif ⇒ `#[serde(default)]` » | `PrSnapshot::head_ref_oid` (R-4) | `worktree_reaper.rs:770` |
| la partition exacte à imiter | `mika2619_la_partition_des_motifs_est_exacte` | `worktree_reaper.rs` |
| la liste à partitionner | `PURGE_ELIGIBLE_REASONS` (9 motifs) | `worktree_reaper.rs:3135` |
| le modèle du test de bout en bout | `mika2497_v1_le_target_inactif_est_purge_et_le_reste_intact` | `worktree_reaper.rs:6643` |
| la résolution par branche | `PrIndex::by_branch` | `worktree_reaper.rs:826` |
| le champ de branche, **`Option`** | `ReapRefusal::branch` | `worktree_reaper.rs:914` |

## Implémentation — ordre de commit

**C1 — l'état, en fonctions pures.**
- `ReviewSnapshot { state: String }`, `#[serde(default)]` sur le champ.
- `ConsumerState { Done, Pending, Unknown }`, `Default` = `Unknown`.
- `consumer_state_for(reason: &str, has_pr: bool, branch_reviewed: Option<bool>) -> ConsumerState`
  — pure, aucun réseau, aucun disque. Implémente la table ci-dessus.
- `CONSUMER_ANSWERABLE_REASONS` / `CONSUMER_UNANSWERABLE_REASONS` : partition déclarée
  de `PURGE_ELIGIBLE_REASONS`, sur le modèle mika#2619.

**C2 — l'appel étroit.**
- `list_open_pr_reviews(repo, token) -> Result<HashMap<String, bool>, String>` :
  `gh pr list --state open --json number,headRefName,latestReviews --limit 300`, rendu
  comme `branche → au moins une revue`. Le `body` de chaque revue est **jeté à la
  désérialisation** (seul `state` est retenu), donc l'empreinte mémoire est bornée.
- `Err` ⇒ WARN `target_purge_reviews_unreadable`, et **le bras continue** avec une carte
  vide ⇒ tous les `pr_open` en `Unknown` ⇒ fenêtre de 4 h. Le faucheur n'est pas touché.

**C3 — la fenêtre par candidat.**
- `TargetPurgeConfig.consumer_done_idle_secs`, env
  `MIKA_TARGET_PURGE_CONSUMER_DONE_IDLE_SECS`, défaut **`900`** (15 min).
- `screen_target_purges` prend un sixième paramètre
  `consumer: &HashMap<String, ConsumerState>` (clé = chemin du worktree). P4 devient :
  fenêtre = `consumer_done_idle_secs` si `Done`, `cfg.idle_secs` sinon. **Une entrée
  absente vaut `Unknown`**, même invariant fail-safe que `states`.
- `purge_stale_target_dirs` construit la carte avant la sélection, depuis `prs`
  (déjà en main) et C2.
- Deux champs **additifs** sur `TargetPurgeCandidate` : `window_secs: i64` et
  `consumer_state: &'static str`, repris sur la ligne INFO et sur
  `record_purged`. Un opérateur doit pouvoir dire **quelle fenêtre a mordu** —
  sans quoi `target_purged` ne distingue plus les deux populations. Aucun second
  `tool_name` : la purge est faite par le même bras, sous la même conjonction, avec la
  même létalité (doctrine `keep_reason`, mika#2619 AC4).

**C4 — les tests** (voir § Contrat de vérification).

**C5 — `docs/operator/environment-variables.md`** : la nouvelle variable à la suite des
quatre `MIKA_TARGET_PURGE*` (ligne ~7118), et la mise en garde de la ligne 7288 amendée
pour dire que la fenêtre de 4 h **n'a pas été raccourcie**.

## Pourquoi 900 s, et pourquoi la fenêtre n'est pas nulle

P3 (`/proc`) et P5 (le verrou cargo) sont ce qui protège la **correction** ; la fenêtre
protège le **cache** d'une boucle encore active. Une fois le consommateur attesté fini,
il reste un cache qui vaut quelque chose dans un seul cas : un verdict bloquant suivi
d'une itération dispatchée dans le même worktree. 900 s couvre le délai réaliste entre
le webhook `pull_request_review.submitted` et l'arrivée de cette itération — et quand
elle arrive, c'est **P3 qui la protège**, pas la fenêtre : sa session habite le
worktree, le bras refuse sous `live_process`. La fenêtre n'est donc pas nulle parce
qu'une fenêtre nulle supprimerait la marge entre le `gh pr review` qui réussit et la fin
réelle du tour QA (`update_task_status`, `send_message`).

## Coûts assumés, nommés

1. **Un verdict bloquant peut coûter une recompilation à froid.** Borné : `target/` est
   du dérivé pur, intégralement reconstructible — l'asymétrie fondatrice de mika#2497.
   Atténué par P3 comme ci-dessus, jamais supprimé.
2. **La latence est d'un tick plus la fenêtre (≈ 15–25 min), pas zéro.** Le ticket écrit
   « dès que » ; ce plan livre 25 min au lieu de 4 h 10. À 5 points/h, c'est ≤ 1,3 point
   de disque résident par worktree au lieu de ~21. Raccourcir le cron de 10 à 5 min
   doublerait le coût `gh pr list` du faucheur pour gagner 5 min : refusé, et nommé ici
   plutôt que laissé à découvrir.
3. **Un second appel `gh` par dépôt et par tick** : +0,79 s mesuré, hors du chemin
   critique du faucheur.
4. **La fenêtre de 4 h n'est pas touchée.** La note opérateur de
   `environment-variables.md:7288` met en garde contre un raccourcissement réflexe de
   `MIKA_TARGET_PURGE_IDLE_SECS` — *« ce serait purger le cache de travail en cours pour
   un problème de course »*. Ce plan la laisse à 14400 et ajoute une **seconde fenêtre,
   attestée**, qui ne s'applique que là où le moteur peut établir qu'aucun consommateur
   ne reste.

## Ce que le plan ne touche pas

- **`scripts/mika-platform-worktree-cleanup`** — hors d'atteinte du bac à sable, autre
  dépôt, invariant zéro-issue préservé. Sa récupération par pression disque reste le
  filet de dernier recours, et le ticket la conserve explicitement.
- **Les cinq termes P1–P5**, le kill-switch `MIKA_TARGET_PURGE`, la disposition
  `observe`, le cap `MIKA_TARGET_PURGE_MAX_PER_TICK`, la sentinelle STOP, les clés
  d'audit, les trois `tool_name`.
- **Le faucheur terminal de mika#2420** et sa population disjointe.
- **`dispatch-lib.sh`**, le handler `build-mika`, le prompt `qa-review` : aucun signal à
  émettre, donc aucun site de production à modifier.

## Contrat de vérification

Tous les tests sont `#[cfg(test)] mod tests` dans `worktree_reaper.rs`, sur un worktree
**factice** sous `tempfile::tempdir()` portant le segment `/.claude/worktrees/`, sur le
modèle de `mika2497_v1`. **Aucun ne sonde l'hôte** : ni `/proc` réel (les cwd vivants
sont injectés), ni `gh`, ni la base de l'hôte.

| # | test | ce qu'il établit |
|---|---|---|
| U1 | `mika2808_le_verdict_qa_raccourcit_la_fenetre` | `pr_open` + branche revue + `target/` inactif 60 s ⇒ **purgé**, et `src/`, `Cargo.toml`, `.git` du worktree **intacts** |
| U2 | `mika2808_un_worktree_habite_nest_jamais_purge` | même entrée, un cwd vivant à la racine ⇒ refus `live_process`, `target/` **intact** (AC3) |
| U3 | `mika2808_un_vol_sans_pr_purge_aussi` | `pr_unknown` + `target/` inactif 60 s ⇒ **purgé** (AC2) |
| U4 | `mika2808_pr_ouverte_sans_revue_garde_les_quatre_heures` | **contrôle négatif** : `pr_open` sans revue + inactif 60 s ⇒ refus `recently_active`. Ce test est ce qui distingue « une fenêtre attestée » d'« un élargissement en blanc » |
| U5 | `mika2808_la_partition_des_motifs_est_exacte` | les deux listes couvrent `PURGE_ELIGIBLE_REASONS` **exactement une fois chacune** — un motif ajouté demain rougit ici jusqu'à ce que quelqu'un le classe |
| U6 | `mika2808_une_liste_de_revues_illisible_conserve` | carte vide (échec `gh`) ⇒ tous les `pr_open` en `Unknown` ⇒ fenêtre de 4 h ⇒ refus |
| U7 | `mika2808_le_champ_de_revues_absent_ne_casse_pas_le_parsing` | JSON sans `latestReviews` ⇒ désérialisation **réussie** (doctrine R-4 : un champ additif ne doit pas pouvoir éteindre la fonction qu'il enrichit) |

**Sonde post-déploiement** (l'hôte, pas le pilote — la population réelle n'est pas
mesurable depuis un bac à sable, cf. `environment-variables.md:7490`) :

```sql
SELECT count(*) FROM audit_events WHERE tool_name = 'target_purged'
  AND created_at >= datetime('now','-1 day');
```
puis, sur les lignes du jour, `jq 'select(.consumer_state == "done")'` — **halte** : un
compte nul en `consumer_state = "done"` alors que des PR ont été revues dans la journée
veut dire que l'appel étroit échoue en silence ; chercher
`target_purge_reviews_unreadable`. Démarrer en `MIKA_TARGET_PURGE_DISPOSITION=observe`
un tick (sonde S0 de mika#2497), lire `target_purge_would_dispose`, puis armer.

## Definition of Done

- [ ] C1–C5 livrés, dans cet ordre de commit.
- [ ] `cargo test -p mika-agent` vert ; U1–U7 présents et verts.
- [ ] `cargo clippy --all-targets -- -D warnings` vert ; `cargo fmt --check` vert.
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] `docs/operator/environment-variables.md` porte la nouvelle variable et l'amendement
      de la note de la ligne 7288 ; aucune valeur existante déplacée.
- [ ] Le corps de PR nomme les deux coûts assumés 1 et 2 et la latence réelle (≈ 25 min,
      pas « dès que »).
- [ ] `cargo clean` au push ; un seul push.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; ceux-ci sont
dérivés de sa section « Travail attendu », point par point.

- **AC1 — le `target/` est purgé quand son dernier consommateur a fini, et le point
  d'accroche est tranché.** Le plan nomme le **verdict QA** (et non la sortie du
  handler de build), avec son fondement : `qa-review/system_prompt.md` ligne 420 lit
  `<worktree>/target/release/mika` après le callback de build et avant l'étape 5.
  Vérifié par **U1** : un worktree dont la PR porte ≥ 1 revue soumise voit son `target/`
  purgé après 900 s d'inactivité au lieu de 14 400 s.
- **AC2 — un vol qui se termine sans PR, quelle qu'en soit la raison, purge aussi son
  `target/`.** Couvert par les motifs `pr_unknown` et `detached_head_pr_unknown`, qui
  attestent qu'aucune PR n'existe pour cette branche / ce SHA. Vérifié par **U3**. La
  clause « quelle qu'en soit la raison » est satisfaite parce qu'aucun signal n'est émis
  par le producteur : une mort violente du pgid du pilote (mika#2263) ne peut donc pas
  la trouer.
- **AC3 — la purge ne touche jamais un worktree habité, et ne touche que `target/`.**
  Garde `/proc/*/cwd` héritée de P3, inchangée ; gardes de chemin P1 +
  `canonical_path_is_managed` inchangées ; le retrait porte sur le répertoire de build
  seul. Vérifié par **U2** (habité ⇒ intact) et par la seconde moitié de **U1** (l'arbre
  de travail survit à la purge).
- **AC4 — le test porte sur un worktree factice et ne sonde jamais l'hôte.** U1–U7
  tournent sur `tempfile::tempdir()`, avec cwd vivants injectés et liste de revues
  injectée ; aucun appel `gh`, aucun accès à `/proc` réel, aucune requête sur la base de
  l'hôte.
- **AC5 — aucune régression du mécanisme conservé.** `MIKA_TARGET_PURGE_IDLE_SECS` reste
  à 14400, les cinq termes P1–P5 sont inchangés, et **U4** l'atteste négativement : une
  PR ouverte sans revue est encore refusée sous `recently_active`.
- **AC6 — un terme illisible conserve.** Échec de l'appel étroit ⇒ fenêtre de 4 h
  (**U6**) ; champ `latestReviews` absent ⇒ parsing réussi et dégradation bornée au
  nouveau chemin (**U7**).

## Fire-Disposition

Ce plan livre des détecteurs : U1–U7, dont **U5** (partition exacte de
`PURGE_ELIGIBLE_REASONS`) et **U4** (contrôle négatif) sont des détecteurs structurels
au sens de mika#1574 — leur chemin de succès est « aucune violation trouvée ».

**Option retenue : (a) exception nommée en allowlist — avec une allowlist vide par
construction.**

- **Aucune violation existante.** U5 partitionne les neuf motifs de
  `PURGE_ELIGIBLE_REASONS` dans le **même commit** qui les déclare : la liste est
  classée au moment où le détecteur atterrit, donc il est vert sur l'arbre tel qu'il
  est. Il n'y a rien à mettre en allowlist, et inventer une entrée vide serait un faux
  grep-visible.
- **L'assertion auto-nettoyante est le détecteur lui-même.** U5 exige que chaque motif
  de `PURGE_ELIGIBLE_REASONS` apparaisse **exactement une fois** dans l'une des deux
  listes. Un dixième motif ajouté demain — le cas réel de mika#2619, où
  `detached_head_pr_unknown` est né de mika#2518 et n'a jamais été ajouté, pour 40 G —
  fait rougir U5 jusqu'à ce que quelqu'un le classe. Pas de décision, pas de build.
- **Aucun détecteur ne débarque désarmé** : ni `#[ignore]`, ni `#[cfg(skip)]`. Les sept
  tests sont verts au commit de livraison, vérifié par `cargo test -p mika-agent`.
- **Aucune halte-et-remontée** : les deux questions que le ticket laissait ouvertes (le
  point d'accroche, et signal émis contre état lu) sont tranchées ci-dessus sur des faits
  lus dans l'arbre, pas sur une préférence.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `worktree_reaper.rs` — `ReviewSnapshot`, `ConsumerState`, `consumer_state_for`, les deux listes de partition (C1) | 95 |
| `worktree_reaper.rs` — `list_open_pr_reviews` + sa dégradation nommée (C2) | 45 |
| `worktree_reaper.rs` — `consumer_done_idle_secs` + parse env (C3) | 35 |
| `worktree_reaper.rs` — fenêtre par candidat dans `screen_target_purges`, construction de la carte dans `purge_stale_target_dirs`, deux champs additifs sur `TargetPurgeCandidate` + `record_purged` (C3) | 60 |
| tests U1–U7 (`worktree_reaper.rs`, `#[cfg(test)]`) (C4) | 190 |

Total estimé : 425 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 425 contre une borne de 429 : c'est serré,
et c'est délibérément le périmètre maximal admissible sans découpage. Trois propriétés
le tiennent :

1. **Aucun nouveau module, aucun nouveau site.** Tout atterrit dans un bras existant
   (`purge_stale_target_dirs`) dont les cinq termes, les clés d'audit, le kill-switch,
   le cap et les gardes sont déjà écrits et déjà testés. Le travail est l'ajout d'une
   **fenêtre**, pas d'un mécanisme.
2. **`prs: &PrIndex` est déjà un paramètre du bras** (`worktree_reaper.rs:4286`) : il n'y
   a aucun plomberie de données à tirer à travers les appelants, ce qui est la source
   habituelle du dépassement sur ce module.
3. **Les 190 lignes de test sont sept tests courts** bâtis sur un helper existant
   (`purge_select`, `purge_dirs`, `purge_states`, `worktree_reaper.rs:6594-6635`), pas
   sept harnais neufs.

Si l'implémentation découvre que la densité documentaire du module pousse C1 au-delà de
son estimation, **la coupe est C2+C3 reportés** : livrer C1 (l'état, en fonctions pures,
avec U5 et U7) seul est mergeable et inerte — `ConsumerState` n'est lu par personne
tant que C3 n'existe pas. La suite serait alors `Tracked in:` sur un ticket de phase 2
nommé dans le corps de PR. Ce n'est pas le plan nominal ; c'est sa soupape, écrite
d'avance pour qu'elle ne soit pas improvisée au tour 180.
