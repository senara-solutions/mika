# Le bras de purge cesse de lire *pourquoi* un worktree est conservé (mika#2619)

**Ticket :** senara-solutions/mika#2619
**Source :** mesure opérateur du 2026-10-01 — `/data` à **85 %** (299/371 Go),
redescendu à **67 %** après purge manuelle de trois répertoires de build
(**64 Go**). Aucun des trois n'était dans la population du bras de purge.
**Type :** élargissement de population + changement de cardinalité du bras de
purge `target/`.
**Périmètre :** `crates/mika-agent/src/worktree_reaper.rs` (production + tests
in-module) et la section mika#2497/#2511/#2482 du `CLAUDE.md` racine.
**Aucune migration, aucune variable d'environnement nouvelle, aucune valeur de
réglage déplacée** — ni `MIKA_TARGET_PURGE_IDLE_SECS` (14400), ni
`MIKA_TARGET_PURGE_MAX_PER_TICK` (2), ni `MIKA_WORKTREE_REAP_*`.
**Les cinq termes P1–P5 ne bougent pas**, verrou de build compris.
**Classe :** hygiène de la boucle — au-delà de 85 % les builds s'arrêtent et la
boucle entière se bloque.

> **Un détecteur est livré** (deux scans structurels et une assertion de
> partition) : la section `## Fire-Disposition` est donc **requise** et figure
> ci-dessous, option (a), allowlists livrées vides.

---

## Constat, et ce que la lecture du code en déplace

Le ticket pose trois worktrees et trois volumes. Reproduits comme classes, par
lecture du code plutôt que par confiance :

| worktree | motif du faucheur | volume | arrive-t-il au bras de purge ? |
|---|---|---|---|
| `refactor-2194-…` | `detached_head_pr_unknown` | 40 Go `target/` | **oui**, dans `screened.refusals` — seule la liste d'éligibilité le refuse |
| `fix-2105-…` | `unpushed_commits` | 17 Go sous `.pilot-scratch/ac6/` | **non** — ni le motif, ni le chemin, ni le vecteur |
| `fix-2616-…` | `too_young` | 8 Go `target/` | oui, et P4 le refuse légitimement (target récent) |

La ligne du milieu est la plus coûteuse à lire, et c'est elle qui fixe la forme
du plan : elle échoue à **trois** endroits indépendants, pas à un.

---

## Sept rectifications que la lecture du code impose au ticket

C'est le premier livrable du grooming : appliquer l'AC1 à la lettre produirait un
correctif **partiellement inerte**, et l'inertie serait invisible.

### R1 — élargir la liste à `dirty` / `unpushed_commits` est INERTE sans changer le vecteur passé

`purge_stale_target_dirs` est appelée avec `&screened.refusals`
(`worktree_reaper.rs:1891`), c'est-à-dire la sortie de `screen_worktrees`, qui
couvre T1–T6 : `outside_managed_root`, `pr_unknown`, `detached_head`,
`detached_head_pr_unknown`, `pr_open`, `pr_closed_at_unreadable`, `too_young`,
`process_scan_unreadable`, `live_process`.

`dirty`, `unpushed_commits` et `work_state_unreadable` sont poussés par
`apply_work_states` (T7), dont la sortie vit dans `selection.refusals` — **un
vecteur que le bras de purge ne reçoit jamais**. Ajouter ces trois motifs à
`PURGE_ELIGIBLE_REASONS` et rien d'autre produirait un bras qui se lit comme
élargi et ne purgerait pas un octet de plus, avec tous les tests verts.

C'est la classe mika#2205, et l'ironie est que le doc-comment de ce site **met
en garde contre exactement ça** (« filtrer le mauvais vecteur rendrait une
population vide, c'est-à-dire un bras qui se lit comme sain en ne faisant
rien ») — en l'énonçant dans la direction inverse de celle qui mord ici.

### R2 — un second trou, non nommé par le ticket : à budget faucheur nul, T7 n'est pas évalué du tout

La collecte des `WorkState` et l'appel à `apply_work_states` vivent **dans** un
bloc `if budget > 0 { … }` (`worktree_reaper.rs:1783`), ajouté par mika#2511
pour ne pas payer deux `git` par candidat quand le faucheur ne peut plus
disposer. Conséquence : quand le budget du faucheur est épuisé (défaut **3** par
tick), les survivants de T1–T6 ne sont **ni fauchés, ni refusés** — ils
n'apparaissent dans aucun vecteur, et leur répertoire de build échappe aux deux
bras.

Un worktree `dirty` ou `unpushed_commits` est donc invisible à la purge **deux
fois** : par la liste, et par le budget d'un autre bras. Le second ne se
réparerait pas tout seul en élargissant la première.

### R3 — AC4 demande de préserver une lisibilité qui n'existe pas

AC4 dit *« le motif de chaque purge **reste** lisible dans la ligne
`target_purged` »*. Il ne l'est pas : `TargetPurgeCandidate` ne porte pas le
motif — `screen_target_purges` le lit puis le jette — et `record_purged` écrit
`reasoning = "pr=… branch=… idle_secs=… bytes_reclaimed=… truncated=…
disposition=…"`. La ligne INFO non plus.

AC4 est donc un **ajout**, pas une préservation. Et c'est précisément ce qui
rend l'élargissement mesurable : sans ce champ, la population élargie serait
indistinguable de l'ancienne dans les deux surfaces publiées.

### R4 — `target_path_is_disposable` refuse structurellement tout chemin hors `/target`

La garde tardive exige `canonical.ends_with("/target")`
(`worktree_reaper.rs:3830`). Tout répertoire de build sous `.pilot-scratch/` est
refusé par elle **après** avoir franchi les cinq termes — donc AC2 ne peut pas
être satisfaite par une seule extension de population.

### R5 — la cardinalité « un worktree = un `target/` » est câblée dans trois clés

- `states: HashMap<String, TargetState>` est clé par **chemin de worktree** ;
- `target_path` est **dérivé** (`target_dir_of`), jamais découvert ;
- `purge_refusal_audit_key(worktree_path, reason)` est clé par
  `(worktree, motif)` — donc deux répertoires de build du même worktree refusés
  sous le même motif **se dédupliquent mutuellement**, et le second refus est
  perdu en silence.

AC2 impose N répertoires par worktree, donc ces trois clés doivent passer au
chemin du répertoire de build.

### R6 — un pilote vif protège structurellement TOUT son worktree, `.pilot-scratch/` compris

C'est ce qui rend AC2 sûre, et ça mérite d'être écrit avant d'écrire le code :
P3 refuse dès qu'un processus vivant a son cwd **sous** le worktree
(`cwds.iter().any(|cwd| cwd == root || cwd.starts_with(root))`). Un pilote
dispatché travaille à la racine de son worktree, donc `cwd == root`, donc le
worktree entier — et tous ses répertoires de build — sort de la population.

**La population AC2 est donc structurellement celle des worktrees sans pilote
vif.** Le prédicat ne se fie pas à cette propriété (P4 et P5 restent), mais elle
explique pourquoi élargir à `.pilot-scratch/` ne met pas en danger un brouillon
en cours d'usage.

### R7 — `.pilot-scratch/` est vidé au seed, donc la population AC2 est celle des worktrees NON repris

`_seed_pilot_scratch_dir` (`dispatch-lib.sh:2362-2371`) crée `.pilot-scratch/`
dans chaque worktree de dispatch **et le vide à chaque préparation**, appelé
depuis `_set_up_worktree`. Les 17 Go de `fix-2105` ont survécu parce que ce
worktree n'a plus été dispatché.

Corollaire utile : **la purge ne retire aucune garantie que le seed ne retire
déjà**. Et la tension apparente avec la règle de dispatch mika#2548 (« ne
supprime JAMAIS un brouillon de `.pilot-scratch/` ») n'existe pas : cette règle
s'adresse au **pilote**, contre un réflexe de rangement en cours de session ; la
purge est du code **moteur**, hors session, sur un répertoire inactif depuis au
moins quatre heures dans un worktree sans processus vivant, et son prédicat de
reconnaissance ne retient que des répertoires de **cache** (voir D4). Un fixture
de texte sous `.pilot-scratch/` n'entre dans aucune population.

---

## Six décisions

### D1 — la liste reste une allowlist, et une assertion de PARTITION ferme la classe

Le ticket propose « tout motif sauf `live_process` », c'est-à-dire d'inverser la
polarité. **Refusé**, pour une raison mesurable : le prédicat actuel est
fail-closed sur l'inconnu (`is_purge_eligible_reason("")` et
`("pr_unknown_")` rendent `false`, épinglé), et une denylist pure rendrait
`true` pour toute chaîne non listée.

Mais la polarité n'est pas la cause du défaut. La cause est qu'une liste de deux
noms a **pris du retard** : `detached_head_pr_unknown` est né de mika#2518 et
personne n'a pensé à l'y ajouter. Le remède qui ferme la **classe** plutôt que
l'occurrence est donc une **partition exacte et assertée** de
`ALL_REFUSAL_REASONS` en deux listes déclarées :

```rust
pub const PURGE_ELIGIBLE_REASONS:   &[&str] = &[ /* 9 */ ];
pub const PURGE_INELIGIBLE_REASONS: &[&str] = &[ /* 3 */ ];
```

`is_purge_eligible_reason` continue de lire la première — aucune sémantique
changée, fail-closed préservé — et un test exige que **chaque** motif de
`ALL_REFUSAL_REASONS` figure dans **exactement une** des deux. Un motif ajouté
demain fait rougir ce test jusqu'à ce que quelqu'un le classe : le défaut n'est
plus « purgeable par oubli » ni « non purgeable par oubli », c'est **« pas de
décision, pas de build »**.

**Les neuf éligibles**, avec ce que chacun ajoute :

| motif | ajouté ? | raison |
|---|---|---|
| `pr_open` | non | population d'origine |
| `pr_unknown` | non | mika#2482 |
| `detached_head_pr_unknown` | **oui** | les 40 Go de `refactor-2194` |
| `detached_head` | **oui** | anomalie git ; le `target/` reste du dérivé |
| `dirty` | **oui** | AC1 — exige aussi R1 |
| `unpushed_commits` | **oui** | les 17 Go de `fix-2105` — exige aussi R1 |
| `work_state_unreadable` | **oui** | `git` muet ne dit rien d'un cache de build |
| `pr_closed_at_unreadable` | **oui** | idem, sur la borne de grâce |
| `too_young` | **oui** | voir ci-dessous |

`too_young` est **quasi inerte et inclus par cohérence plutôt que par
exception** : une PR close depuis moins de 900 s a presque toujours un `target/`
plus récent que la fenêtre de 4 h, donc P4 le refuse. L'exclure demanderait
d'argumenter « le faucheur va le retirer en entier dans ≤ 10 min », ce qui est
vrai *tant que le faucheur est armé* — une dépendance que l'asymétrie fondatrice
ne demande pas. Un motif de moins à justifier.

**Les trois inéligibles**, chacun parce que son terme de purge le refuse déjà :

| motif | pourquoi exclu |
|---|---|
| `live_process` | **P3 re-décide**. L'inclure écrirait une ligne `target_purge_skipped` de motif `live_process` doublant celle du faucheur, sous un autre `tool_name` : du bruit sans information neuve. |
| `process_scan_unreadable` | Même tick, même cause : `live` vaut `Unavailable`, donc **P3 refuse tout le monde**. L'inclure produirait une ligne de refus par worktree de la population, toutes pour une seule cause déjà nommée côté faucheur. |
| `outside_managed_root` | **P1 refuse**, et ce motif doit rester vide côté faucheur (HALTE 4 du `CLAUDE.md`). Le faire traverser rendrait une HALTE illisible. |

Les trois exclusions sont des **redondances**, jamais des réserves de sûreté :
aucune ne protège un worktree que P1–P5 laisseraient passer. C'est ce qui
autorise à les écrire comme telles plutôt qu'à les regretter.

### D2 — le vecteur de purge est la concaténation, et T7 est évalué quand la purge en a besoin

Deux changements, et il faut les **deux** (R1 + R2) :

1. `purge_stale_target_dirs` reçoit **deux** slices, `screened_refusals` et
   `t7_refusals`, et les concatène en interne. Les fonctions pures
   (`screen_target_purges`, `apply_lock_probes`, `select_target_purges`) gardent
   leur signature `&[ReapRefusal]` **inchangée** : la concaténation est une
   affaire d'appelant, tous les tests existants restent valides, et un test pur
   peut passer un vecteur contenant un `dirty`.

2. La collecte des `WorkState` et `apply_work_states` sortent du
   `if budget > 0`, sous une condition élargie :

   ```rust
   let t7_needed = budget > 0 || (purge_cfg.enabled && purge_budget > 0);
   ```

   La **boucle de disposition du faucheur** reste, elle, sous `budget > 0`.
   C'est le motif `should_stop_repo_loop` de mika#2511, appliqué un cran plus
   bas : le bras qui a besoin du calcul le paie.

**Coût nommé, et c'est le seul que D2 crée :** deux sous-processus `git` par
candidat T1–T6 même quand le faucheur n'a plus de budget, à condition que la
purge soit armée et ait du budget. Borné par le nombre de worktrees dont la PR
est terminale, hors grâce, sans processus vivant — petit par construction, et
nul quand il n'y en a aucun.

**Changement de population comptable, à dater :** les refus `dirty` /
`unpushed_commits` / `work_state_unreadable` étaient **tronqués par le budget du
faucheur** ; ils seront désormais écrits à chaque tick où la purge tourne. Le
compte `worktree_reap_skipped` sur ces trois motifs **monte** après déploiement,
et ce n'est pas une dégradation : la population devient complète. Les lignes
antérieures ne sont pas réécrites (motif mika#2361).

**Trou résiduel, nommé et non fermé :** si le budget du faucheur s'épuise *en
cours* de boucle de disposition, les candidats `Clean` restants ne sont ni
fauchés ni refusés, donc leur répertoire de build échappe à ce tick. Transitoire
(≤ 10 min), borné, et fermer ce cas demanderait de pousser un refus synthétique
pour un worktree que rien ne refuse — une ligne d'audit fausse.

### D3 — N répertoires de build par worktree, découverts et jamais devinés

Une fonction pure, **lecteur unique** de la composition des chemins :

```rust
pub fn discover_build_dirs(worktree: &Path) -> Vec<PathBuf>
```

- `<worktree>/target` s'il existe (compatibilité : **aucune condition de
  marqueur**, pour ne pas rétrécir la population d'aujourd'hui) ;
- puis une marche **bornée** sous `<worktree>/.pilot-scratch/`, profondeur ≤ 3,
  retenant tout répertoire porteur d'un marqueur de cache (D4), **sans y
  descendre** une fois reconnu — un répertoire de build contient des
  sous-répertoires, et les énumérer serait une marche non bornée dans
  l'arborescence qu'on vient d'identifier comme un cache.
- Ordre déterministe (tri), pour que le budget soit consommé de façon
  reproductible et que les tests n'aient pas à ordonner.

Impacts, chacun un changement de clé :

- `states` passe de clé-worktree à **clé-répertoire-de-build** ;
- `TargetPurgeCandidate` garde son champ `target_path` — **champ de la ligne
  INFO, donc format de fil** — dont la sémantique s'élargit de « le `target/` du
  worktree » à « le répertoire de build » ; `worktree_path` est conservé, et un
  champ `keep_reason` est ajouté (D5) ;
- `purge_refusal_audit_key` prend le **chemin du répertoire de build** au lieu du
  chemin du worktree. Sans ce changement, deux répertoires du même worktree
  refusés sous le même motif se dédupliquent mutuellement. **Coût daté :** la
  clé passe de `target:<worktree>@<motif>` à
  `target:<worktree>/target@<motif>`. Les requêtes publiées dans le `CLAUDE.md`
  groupent par `after_value` (le motif) et ne sont **pas** affectées ; seule une
  requête `WHERE target_key = …` exacte l'est.
- **Le budget est par répertoire** : un worktree portant trois répertoires de
  build consomme trois unités. Cohérent avec ce que le cap borne — les tempêtes
  d'E/S viennent des suppressions, pas des worktrees.

### D4 — le marqueur est une déclaration de cache, et il complète le nom sans le remplacer

Reconnaissance d'un répertoire de build : présence, **à sa racine**, de
`.rustc_info.json` **ou** de `CACHEDIR.TAG`. Les deux sont nommés par le ticket ;
le premier est spécifique à cargo, le second est le standard par lequel un outil
déclare lui-même « ceci est un cache » — exactement l'information que
l'asymétrie fondatrice demande (du dérivé pur). Un répertoire qui n'en porte
aucun n'est **pas reconnu**, donc n'entre pas dans la population : la direction
sûre.

`target_path_is_disposable` devient : non-symlink **ET** sous la racine gérée
après canonicalisation **ET** (`ends_with("/target")` **OU** porte un marqueur,
re-vérifié après canonicalisation).

**La disjonction est voulue, dans les deux sens.** Garder `ends_with("/target")`
évite de rétrécir la population actuelle (un `target/` fraîchement créé, encore
sans marqueur, reste purgeable comme aujourd'hui). Ajouter le marqueur est ce
qui **remplace la preuve par le nom** pour tout le reste — et c'est une preuve
plus forte : un répertoire *nommé* `target` qui n'est pas un cache est
aujourd'hui supprimable, alors qu'un répertoire porteur d'un marqueur est un
cache par déclaration de son producteur.

Nouveau motif de refus, **en queue** de `ALL_PURGE_REFUSAL_REASONS` (un ajout,
jamais un renommage) : `PURGE_REASON_NOT_A_BUILD_DIR`, pour le refus **tardif**
d'un répertoire dont le marqueur a disparu entre la découverte et la garde.
Distinct de `outside_managed_root`, qui serait une ligne d'audit **fausse** — le
chemin est bien sous la racine gérée, c'est la preuve qui manque (doctrine
`pilot_stall_signal_unavailable`, mika#2277).

**Aucun refus n'est écrit pour un répertoire non reconnu à la découverte** : ce
serait une ligne par sous-répertoire de `.pilot-scratch/`, c'est-à-dire du bruit
sur une population qui n'a jamais été candidate (même raison que l'exclusion du
checkout primaire côté faucheur, HALTE 4).

### D5 — AC4 : `keep_reason` sur la ligne, un champ et pas un second `tool_name`

`TargetPurgeCandidate` gagne `keep_reason: &'static str`, propagé depuis
`ReapRefusal.reason`, rendu sur la ligne INFO (`keep_reason=…`) et dans
`record_purged`'s `reasoning` (`keep_reason=…`).

**Un champ, jamais un second `tool_name`** — le choix est déjà tranché deux fois
dans ce fichier (`RESOLUTION_BRANCH`, mika#2518 ; `ready_label_outcome`,
mika#2323) et le critère s'applique mot pour mot : les retraits sont faits par
**le même bras**, sous la **même conjonction** de cinq termes, avec la **même
létalité** ; seul le motif de conservation diffère. Créer un second `tool_name`
tronquerait en silence `SELECT … WHERE tool_name = 'target_purged'`, qui est une
requête publiée.

Ce champ est ce qui rend la population élargie **comptable séparément** :
`jq 'select(.keep_reason == "unpushed_commits")'` et
`reasoning LIKE '%keep_reason=detached_head_pr_unknown%'`.

### D6 — périmètre : AC1 à AC4 ensemble, parce que les pièces partagent un struct

Scinder AC2 en suivi ferait deux refactors successifs de `TargetPurgeCandidate`,
de `states` et des clés d'audit — et laisserait 17 Go des 64 mesurés invisibles.
AC4 ajoute un champ au même struct qu'AC2 modifie. Les quatre sont livrés
ensemble.

---

## Les éditions

Toutes dans `crates/mika-agent/src/worktree_reaper.rs`, plus une section du
`CLAUDE.md` racine.

### E1 — la partition des motifs (D1)

- `PURGE_ELIGIBLE_REASONS` passe de 2 à 9 entrées.
- `PURGE_INELIGIBLE_REASONS` est créée (3 entrées), chaque entrée portant **en
  doc-comment la raison de son exclusion** (AC1 : « si un motif est exclu,
  l'exclusion est écrite avec sa raison »).
- `is_purge_eligible_reason` : corps inchangé.
- Le doc-comment de `PURGE_ELIGIBLE_REASONS` est réécrit : il affirme aujourd'hui
  « le bras ne voyait que `pr_open` … de deux noms à trois », ce qui devient faux.

### E2 — le vecteur et le moment de T7 (D2)

- `purge_stale_target_dirs` : deux slices en entrée, concaténation interne.
- La boucle des dépôts : `t7_needed`, déstructuration de `screened` pour que
  `candidates` soit consommé par `apply_work_states` sans emprunter
  `refusals`, `t7_refusals` remonté hors du bloc.
- Le doc-comment de `purge_stale_target_dirs` — qui dit aujourd'hui
  « `screened.refusals` **et pas** `selection.refusals` » — est réécrit : les
  **deux**, et pourquoi chacun est nécessaire.

### E3 — la découverte des répertoires de build (D3, D4)

- `discover_build_dirs`, `is_cargo_build_dir` (le marqueur), constantes
  `PILOT_SCRATCH_DIRNAME`, `BUILD_DIR_SCAN_DEPTH`, `CARGO_INFO_MARKER`,
  `CACHEDIR_TAG_MARKER`.
- `inspect_target_dir` → `inspect_build_dir(dir, now)` : prend le répertoire
  **déjà découvert** plutôt que de composer `worktree.join("target")`.
  `target_dir_of` n'est plus appelé que par `discover_build_dirs`.
- `screen_target_purges` : la boucle devient, pour chaque refus éligible, une
  boucle sur ses répertoires découverts ; `states` est clé par répertoire.
- `target_path_is_disposable` : la disjonction de D4.
- `purge_refusal_audit_key` : paramètre `build_dir_path`.
- `PURGE_REASON_NOT_A_BUILD_DIR` + entrée en queue de
  `ALL_PURGE_REFUSAL_REASONS`.

### E4 — `keep_reason` (D5)

Champ sur `TargetPurgeCandidate` et sur `TargetPurgeRefusal`, rendu sur la ligne
INFO et dans `reasoning`.

### E5 — les tests existants qui changent de sens

Aucun test existant n'est **supprimé**. Trois voient leur énoncé évoluer, et
chacun est traité comme une décision datée plutôt qu'une mise à jour silencieuse :

- `mika2497_v4_seul_un_repertoire_nomme_target_est_disposable` — ses deux
  assertions **restent vraies** (ni `src` ni `src/target-ish` ne portent de
  marqueur), mais son titre devient trompeur. Renommé
  `…_la_preuve_est_le_nom_ou_le_marqueur`, et **augmenté** du cas qui porte la
  nouvelle moitié : un répertoire hors `/target` **avec** marqueur est
  disposable, un répertoire hors `/target` **sans** marqueur ne l'est pas.
- `mika2482_les_motifs_eligibles_a_la_purge_sont_un_format_de_fil` — la liste
  attendue passe de 2 à 9 entrées. Son message (« élargir cette liste change ce
  que la boucle supprime : le dater dans CLAUDE.md, jamais mettre ce test à jour
  en silence ») est **exécuté** : le `CLAUDE.md` est édité dans le même commit.
- `mika2497_v6_les_deux_populations_sont_disjointes` — reste vert, mais sa
  propriété devient **conditionnelle à T7** et doit être re-énoncée : les deux
  bras ne visent jamais le même worktree **au même tick** parce que la purge
  travaille les *refus* (T1–T7) et le faucheur les *survivants* de T7. Augmenté
  d'un cas `dirty`, qui est précisément la forme où l'ancienne formulation
  cesserait d'être évidente.

### E6 — `CLAUDE.md` racine

La section *« La purge `target/` d'un worktree vif »* : la population (9 motifs),
`keep_reason` sur la surface opérateur, les répertoires sous `.pilot-scratch/`,
le changement de clé d'audit des refus, la montée attendue du compte
`worktree_reap_skipped` sur les trois motifs T7, et les sondes ci-dessous.

---

## Contrat de vérification

### V1 — contrôle positif d'AC3 : la forme de `refactor-2194`

`select_target_purges` sur un refus `detached_head_pr_unknown`, `target/`
inactif au-delà de la fenêtre, verrou libre ⇒ **un candidat**, et sa
`keep_reason` vaut `detached_head_pr_unknown`.

### V2 — contrôle négatif d'AC3 : le verrou tenu gagne, quel que soit le motif

Pour **chacun des neuf** motifs éligibles, un `target/` dont un `.cargo-lock`
est tenu ⇒ **zéro candidat**, motif `build_lock_held`. Le test boucle sur
`PURGE_ELIGIBLE_REASONS` plutôt que d'énumérer : un dixième motif ajouté demain
est couvert sans qu'on y pense.

### V3 — la partition est exacte (D1)

Chaque entrée de `ALL_REFUSAL_REASONS` est dans **exactement une** des deux
listes ; les deux listes ne contiennent que des motifs réels ; les deux sont non
vides (contrôle de non-vacuité, sans lequel deux listes vides satisferaient la
partition).

### V4 — `dirty` traverse réellement la PRODUCTION (R1, et c'est le test qui compte)

Un test **async** sur `purge_stale_target_dirs` : un worktree dont le seul refus
est `dirty`, passé dans le vecteur T7 ⇒ son `target/` est purgé et la ligne
d'audit porte `keep_reason=dirty`.

Un test sur la fonction pure **ne suffirait pas** : il resterait vert si
l'appelant oubliait le second vecteur, ce qui est exactement la forme de R1.

### V5 — T7 est évalué quand le faucheur est à budget nul (R2)

`budget = 0`, purge armée avec budget ⇒ les refus T7 existent et la purge voit
un worktree `unpushed_commits`. Contrôle négatif : `budget = 0` **et** purge
désarmée ⇒ aucun `git` de `collect_work_state` n'est payé (assertion sur
l'absence de refus T7 écrits).

### V6 — un répertoire sous `.pilot-scratch/` est purgé (AC2)

Un worktree avec `target/` **et** `.pilot-scratch/ac6/a` et
`.pilot-scratch/ac6/b` porteurs d'un marqueur ⇒ trois candidats, trois
`target_path` distincts, trois clés d'audit distinctes. Contrôle négatif : un
`.pilot-scratch/draft/notes.md` et un `.pilot-scratch/vide/` **sans** marqueur
ne produisent ni candidat ni refus.

### V7 — la marche de découverte est bornée

Un répertoire reconnu à la profondeur 1 sous `.pilot-scratch/` ne voit pas ses
sous-répertoires énumérés (un sous-répertoire porteur d'un marqueur **à
l'intérieur** d'un répertoire déjà reconnu n'apparaît pas comme candidat
séparé) ; rien au-delà de `BUILD_DIR_SCAN_DEPTH` n'est découvert.

### V8 — la garde tardive, dans les deux sens

`target_path_is_disposable` : vrai pour `<wt>/target` sans marqueur (compat),
vrai pour `<wt>/.pilot-scratch/x` avec marqueur, **faux** pour
`<wt>/.pilot-scratch/x` sans marqueur, faux hors racine gérée, faux sur un lien
symbolique. Et un répertoire découvert dont le marqueur disparaît avant la garde
est refusé sous `not_a_build_dir`, **jamais** sous `outside_managed_root`.

### V9 — la déduplication des refus est par répertoire (R5)

Deux répertoires de build du même worktree refusés sous le **même** motif
produisent **deux** clés d'audit distinctes.

### V10 — `keep_reason` est sur les deux surfaces (AC4)

La ligne INFO porte le champ ; `reasoning` porte `keep_reason=<motif>` ; et pour
chacun des neuf motifs la valeur rendue est celle du refus d'origine.

### V11 — formats de fil

`ALL_PURGE_REFUSAL_REASONS` gagne `not_a_build_dir` **en queue** ; les dix
valeurs antérieures sont inchangées dans le même ordre.

### V12 — non-régression des invariants voisins

`mika2511_toute_suppression_est_precedee_de_lacquisition` reste vert sur la
boucle modifiée ; `mika2482_le_predicat_deligibilite_de_purge_a_un_site_unique`
reste vert ; `should_stop_repo_loop` est inchangé ;
`mika2511_v9/v10` (`purged` / `would_purge`) restent verts ;
`mika2497_le_vecteur_de_refus_est_celui_de_lecran` reste vert (sa seule
assertion — `pr_open` ne transite pas par `apply_work_states` — reste vraie).

### V13 — `cargo clippy --all-targets -- -D warnings` et `cargo fmt --check`

---

## Fire-Disposition

Le plan livre **trois détecteurs**, et la disposition est **(a) exception nommée
en allowlist — allowlists livrées vides**, forme canonique de ce module
(`mika2497_lallowlist_de_la_garde_est_vide`,
`mika2482_lallowlist_du_scan_deligibilite_est_vide`,
`mika2511_lallowlist_du_scan_est_vide`).

| détecteur | tire-t-il sur l'arbre ? | disposition |
|---|---|---|
| **D-1** assertion de partition `ALL_REFUSAL_REASONS` = éligibles ⊎ inéligibles (V3) | **non** — les 12 motifs sont classés dans le même commit | aucune exception à déclarer ; assertion de **non-vacuité** des deux listes, sans laquelle deux listes vides passeraient |
| **D-2** scan de source : la composition d'un chemin de répertoire de build a un **site unique** (`discover_build_dirs`) | **non** — `target_dir_of` est routé vers le découvreur dans le même commit | `BUILD_DIR_COMPOSITION_ALLOWED: &[&str] = &[]`, plus un test qui assert qu'elle **reste vide** (doctrine mika#2201 : quand le scan tire, on route le site, on ne l'allowliste pas) |
| **D-3** scan de source : tout refus de purge passe par `purge_refusal_audit_key` avec un chemin de **répertoire de build** | **non** | même forme, allowlist livrée vide + assertion de vacuité |

Chaque scan porte son **contrôle négatif vu rouge** (une fixture en ligne qui
doit faire échouer le scan) **et** son contrôle de bonne foi vu vert — sans le
second, « le scan lit les compositions » est indistinguable de « le scan rougit
sur toute mention », et c'est la paire que mika#2482 a déjà dû écrire pour son
propre scan (`…_est_vu_rouge_sur_un_second_filtre` /
`…_est_vert_sur_la_declaration_et_le_push`).

**Aucun détecteur n'est livré désarmé** (option b) et **aucune halte-et-remontée
n'est nécessaire** (option c) : aucun ne tire sur l'arbre à la livraison, ce qui
est établi par construction (les trois sites concernés sont réécrits dans le même
commit) et non supposé.

---

## Surfaces opérateur

```bash
# 1. Qu'a-t-on purgé, pour quel motif de conservation, et combien ça a rendu ?
grep target_purged "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{worktree_path, target_path, keep_reason, idle_secs, bytes_reclaimed}'

# 2. La population ÉLARGIE, comptée séparément (AC4)
grep target_purged "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.keep_reason != "pr_open" and .keep_reason != "pr_unknown")
           | {target_path, keep_reason, bytes_reclaimed}'

# 3. Les répertoires hors `target/` (AC2)
grep target_purged "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.target_path | test("\\.pilot-scratch/"))'

# 4. CONTRÔLE POSITIF — le bras tourne-t-il seulement ?
grep target_purge_tick "$MIKA_SPIRIT_LOG_FILE" | tail
```

```sql
-- La distribution des refus : INCHANGÉE en vocabulaire, plus `not_a_build_dir`
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'target_purge_skipped' GROUP BY 1 ORDER BY 2 DESC;

-- La population élargie, datée
SELECT target_key, created_at, reasoning FROM audit_events
 WHERE tool_name = 'target_purged' ORDER BY created_at DESC;
```

| surface | régime attendu | lecture |
|---|---|---|
| `target_purged` avec `keep_reason` hors `{pr_open, pr_unknown}` | **non vide** après déploiement | la mesure directe que l'élargissement mord — les 40 Go de `refactor-2194` sont dans cette population |
| `target_purged` avec `target_path` sous `.pilot-scratch/` | non vide, **faible** | les 17 Go de `fix-2105` ; faible parce que seuls les worktrees non repris en portent (R7) |
| `target_purge_skipped` / `recently_active` | **doit dominer** | la fenêtre protège le travail en cours — inchangé |
| `target_purge_skipped` / `not_a_build_dir` | **vide** | un répertoire découvert dont le marqueur a disparu entre découverte et garde |
| `target_purge_skipped` / `build_lock_held` ou `build_lock_raced` | non vide, faible | P5 mord sur la population élargie |
| `worktree_reap_skipped` / `dirty`, `unpushed_commits`, `work_state_unreadable` | **en hausse** | conséquence attendue de D2, non une dégradation : la population n'est plus tronquée par le budget du faucheur |
| `target_purge_failed` | **vide** | une suppression refusée par le système de fichiers |

---

## Sondes post-déploiement, et leurs cinq haltes

> **Préalable.** Ces mesures décrivent le **binaire servi**. Après `make deploy`,
> établir que le `mika-spirit` qui tourne porte le correctif avant toute
> conclusion (classe mika#2340). **Et ce sont des gestes d'opérateur sur l'hôte :**
> la base n'est pas montée dans le bac à sable de dispatch, donc aucune de ces
> requêtes n'est exécutable par le pilote qui écrit ce plan.

**S0 — commencer en `observe`.** Poser
`MIKA_TARGET_PURGE_DISPOSITION=observe` et lire la population qui *serait*
retirée (`target_purge_would_dispose`). Le cap par tick vaut **aussi** en
observation, et la population est désormais **par répertoire**, donc plus large :
laisser tourner jusqu'à ce qu'un tick ne nomme plus de répertoire que
`SELECT DISTINCT target_key` n'ait déjà rendu, puis armer. Armer après un seul
tick retirerait tout le reste sans dry-run.

**S1 — le symptôme (7 jours).** `df -h /data` cesse de rapprocher 85 % ; `du -sh`
sur la racine des worktrees se stabilise nettement sous les 299 Go mesurés.
*Halte 1 —* `/data` remplit encore alors qu'aucun répertoire inactif ne subsiste
⇒ la cause est le **pic de production simultanée**, que ce travail ne borne pas
(voir *Ce que ça n'achète pas*). **Ne pas raccourcir la fenêtre par réflexe** :
ce serait purger le cache de travail en cours pour un problème de
dimensionnement. Ouvrir le suivi `CARGO_TARGET_DIR` partagé **avec la mesure**.

**S2 — l'attribution (48 h).** La sonde 2 est non vide, et la sonde 3 l'est aussi
dès qu'un worktree non repris porte un répertoire de mesure.
*Halte 2 —* la sonde 2 est **vide** alors que des worktrees `detached_head` /
`unpushed_commits` existent : vérifier d'abord le **contrôle positif** (sonde 4).
Zéro purge avec un tick qui agit est sain ; zéro des deux ne prouve rien (classe
mika#2205). Si le tick agit et que la sonde 2 reste vide pour les motifs T7,
**le défaut R1 est revenu** — c'est le vecteur qu'il faut lire, pas la liste.

**S3 — contrôle négatif de la cardinalité (7 jours).** Aucun
`target_purge_skipped` dont le `target_key` ne porte pas de chemin de répertoire
de build.
*Halte 3 —* une clé au format ancien (`target:<worktree>@<motif>`, sans segment
de répertoire) signifie qu'un site compose encore la clé à l'ancienne : c'est le
scan D-3 qui n'a pas pris.

**S4 — contrôle négatif de sûreté (7 jours).** Aucune plainte de rebuild
intempestif sur une itération QA → CI-fix, et **aucun brouillon de
`.pilot-scratch/` perdu**.
*Halte 4 —* un fixture non-cache disparu sous `.pilot-scratch/` :
`MIKA_TARGET_PURGE=0` **immédiatement**, puis diagnostiquer le prédicat de
reconnaissance. C'est le seul mode de panne de ce travail qui coûte autre chose
que du temps de rebuild, et il ne se règle pas en bougeant un seuil.
*Halte 5 —* un build cassé pendant une purge : même geste, puis établir lequel de
P3, P4 ou P5 a lu vrai alors qu'il était faux. Le verrou tenu pendant la
suppression (mika#2511) a une fenêtre résiduelle **nommée et inchangée** — un
`cargo` qui démarre après l'unlink du `.cargo-lock` n'est pas vu.

**Halte transverse — les sondes muettes.** Zéro purge et zéro refus ne prouve
rien tant qu'aucun worktree de la population élargie n'a existé depuis le
déploiement. *Une garde que personne n'a exercée se lit exactement comme une
garde qui marche* (mika#2205).

---

## Ce que ce travail n'achète PAS

- **Il ne borne pas le pic.** Si N worktrees compilent simultanément, aucun n'est
  inactif et la purge n'attrape rien **pendant** la montée — elle attrape le
  résidu. Limite héritée de mika#2497, **inchangée**, et c'est la Halte 1.
- **Il ne mesure pas le disque.** Aucun seuil de remplissage : le bras ne sait pas
  que `/data` est à 85 %, il sait qu'un répertoire de build est inactif. Un
  déclencheur par pression disque est un autre mécanisme, qu'aucune mesure ne
  demande aujourd'hui.
- **Il ne fauche aucun worktree de plus.** Ce qui est retiré reste du **dérivé
  pur** : jamais le worktree, jamais une branche, jamais un commit. Un
  `unpushed_commits` garde ses commits ; un `dirty` garde son arbre de travail.
- **Il ne rattrape pas les 64 Go du 2026-10-01** : ils ont été purgés à la main et
  **rien ne rétro-écrit** une ligne d'audit décrivant un retrait qu'on n'a pas
  observé. La sonde est la **prochaine** occurrence.
- **Il ne ferme pas la fenêtre résiduelle de mika#2511** (unlink → fin de
  suppression), ni le trou nommé en D2 (budget faucheur épuisé en cours de
  boucle).
- **Il rend le champ lisible, pas surveillé.** Les seuls instruments sont les
  greps et les requêtes ci-dessus, et **leur silence ne prouve rien tant que
  personne ne les exécute**.

---

## Hors périmètre, délibérément

- **`CARGO_TARGET_DIR` partagé** — refusé par mika#2497 sur trois motifs écrits
  (sérialisation par verrou exclusif, accumulation inter-branches sans GC,
  changement non mesuré de la performance de build). **Suivi**, précondition :
  la Halte 1.
- **Une purge à la fin de chaque dispatch** (`dispatch-lib.sh`) — refusée par
  mika#2497 : elle détruit le cache entre l'implement et les itérations QA /
  CI-fix sur le même worktree, échangeant du disque contre de la latence sur le
  chemin **nominal**.
- **La fauche d'un `pr_unknown`, d'un `dirty` ou d'un `unpushed_commits`** — un
  `pr_unknown` sort à T3, donc T7 n'est **jamais** évalué sur lui ; un `dirty`
  porte un arbre de travail. Les sept termes du faucheur sont inchangés.
- **Les cinq termes P1–P5** — aucun n'est modifié, verrou de build compris.
- **`PURGE_REASON_BUILD_LOCK_RACED` et sa fenêtre** — mika#2511, inchangé.
- **Les répertoires de build hors `target/` et hors `.pilot-scratch/`** (un
  `CARGO_TARGET_DIR` posé ailleurs dans le worktree) : hors population. Armer une
  découverte sur tout le worktree serait une marche non bornée, et aucune mesure
  ne montre cette population.
- **Rendre `rm`/`rmdir` sous `.pilot-scratch/` survivable côté politique
  claude-pilot** — suivi déjà nommé par mika#2548, sans rapport avec le code
  moteur de ce plan.
- **Le vocabulaire du faucheur** (`ALL_REFUSAL_REASONS`) — aucun motif ajouté,
  retiré ni renommé. Seule leur **classification** est nouvelle.

---

## Definition of Done

1. `PURGE_ELIGIBLE_REASONS` porte 9 motifs, `PURGE_INELIGIBLE_REASONS` en porte 3
   avec la raison de chaque exclusion en doc-comment, et la partition de
   `ALL_REFUSAL_REASONS` est assertée (V3).
2. `purge_stale_target_dirs` reçoit les refus T1–T6 **et** T7, et T7 est évalué
   quand la purge en a besoin même à budget faucheur nul (V4, V5).
3. `discover_build_dirs` est le site unique de composition d'un chemin de
   répertoire de build ; `target/` et les répertoires reconnus sous
   `.pilot-scratch/` entrent dans la population avec les mêmes cinq termes
   (V6, V7).
4. `target_path_is_disposable` accepte le nom **ou** le marqueur, refuse le reste,
   et le refus tardif porte `not_a_build_dir` (V8).
5. Les clés d'audit des refus sont par répertoire de build (V9).
6. `keep_reason` est sur la ligne INFO **et** dans `reasoning`, pour les neuf
   motifs (V10).
7. Les trois détecteurs sont livrés avec leurs allowlists vides, leurs assertions
   de vacuité et leurs contrôles négatifs vus rouges (Fire-Disposition).
8. `cargo test -p mika-agent worktree_reaper` vert ; aucun test existant supprimé ;
   les trois tests dont l'énoncé évolue (E5) le font explicitement.
9. `cargo clippy --all-targets -- -D warnings` et `cargo fmt --check` propres.
10. La section mika#2497/#2511/#2482 du `CLAUDE.md` racine est éditée **dans le
    même commit** que l'élargissement de la liste — ce que le message du test de
    format de fil exige par écrit.

---

## Acceptance criteria

1. `PURGE_ELIGIBLE_REASONS` est élargie à `detached_head`,
   `detached_head_pr_unknown`, `unpushed_commits`, `dirty`,
   `work_state_unreadable`, `pr_closed_at_unreadable` et `too_young` ; les trois
   motifs exclus (`live_process`, `process_scan_unreadable`,
   `outside_managed_root`) le sont **avec leur raison écrite** ; les termes
   P1–P5 sont inchangés.
2. Les répertoires de build sous `.pilot-scratch/`, reconnus à la présence d'un
   `.rustc_info.json` ou d'un `CACHEDIR.TAG`, entrent dans la population avec les
   mêmes termes.
3. **Contrôle positif :** la forme de `refactor-2194` (HEAD détaché, `target/`
   inactif) est purgée. **Contrôle négatif :** un `target/` dont un `.cargo-lock`
   est tenu est conservé, **quel que soit le motif** — asserté sur les neuf.
4. Le motif de conservation du faucheur est lisible sur la ligne `target_purged`
   (champ `keep_reason`) et dans `audit_events.reasoning`, de sorte que la
   population élargie se compte séparément.
