# mika#2497 — Le `target/` d'un worktree vif mais inactif est purgé

> **Parent umbrella :** mika#2491 (Défaut 5). Enfant du cadre substrat — 1 PR atomique.
> **Suivi HALT-2 de mika#2420**, nommé par ce ticket-là dans son propre corps.

## Le défaut, mesuré

Chaque pilote / QA reconstruit un `target/` Rust de 15–50 Go dans son worktree. Rien
ne le purge tant que le worktree **vit**. Nuit du 2026-09-22 : **+90 Go en 8 h**,
`/data` à **83 %**, worktrees à **165 Go** — à un cheveu de casser moteur, QA et
builds. Nettoyé à la main.

Le reaper terminal (mika#2420) ne voit pas cette population : son terme T4 exige
**« aucune PR ouverte »**. Un worktree dont la PR est ouverte lui est refusé sous le
motif `pr_open`, et son `target/` vit aussi longtemps que la PR.

## Ce que la lecture du code a déplacé — les deux questions de design du ticket, tranchées

Le ticket laisse deux questions ouvertes en interdisant de les supposer. Les deux se
tranchent sur des faits, et le premier explique aussi le parking.

### Q2 — Localisation : le fix vit côté `mika`, et ce n'est pas une préférence

`scripts/mika-platform-worktree-cleanup` vit dans le méta-dépôt **mika-platform**.
**Mesuré dans le bac à sable de ce grooming** : le worktree de dispatch ne matérialise
que le sous-dépôt `mika/` — le parent ne contient aucun autre sous-dépôt, et aucun
`../scripts/`. Un pilote dispatché sur `mika` **ne peut pas** éditer ce script : il
n'est pas dans son arbre, et la PR est sur un autre dépôt.

C'est aussi, rétrospectivement, **la cause du parking 4/4** : le premier plan dirigeait
le pilote vers des chemins que son bac à sable ne contient pas. Le remède n'est pas
d'élargir le bac à sable, c'est de mettre le mécanisme là où le pilote travaille.

Corollaire assumé : `mika-platform` garde son invariant zéro-issue et ne reçoit rien.

### Q1 — Chevauchement mika#2420 : disjonction, et asymétrie **inverse**

Les deux populations sont **disjointes par construction** — T4 du reaper est
« aucune PR ouverte », la population d'ici est « PR ouverte ». Il n'y a rien à
re-couvrir.

Mais la propriété qui décide de la conception est ailleurs, et elle doit être écrite
avant tout code :

> **Le reaper de mika#2420 supprime du travail potentiel. Ce livrable supprime du
> dérivé pur.**

Un `target/` ne porte aucun travail : il est intégralement reconstructible par
`cargo build`. Le coût d'un faux positif est donc **borné à du temps de rebuild**,
jamais à une perte. C'est l'exacte inverse de l'asymétrie de mika#2420, dont le plan
écrit *« un faux positif détruit des heures de travail, irréversiblement »*.

Conséquence directe : **les prédicats ne peuvent pas être les mêmes**, et un prédicat
plus permissif est ici légitime là où il serait fautif chez le reaper. Cette asymétrie
est ce qui autorise à toucher un worktree **vif**.

**Ce que l'asymétrie n'autorise PAS**, et c'est la vraie contrainte de sûreté :
supprimer `target/` **pendant** un `cargo build` casse ce build. Le danger n'est pas
la perte de données, c'est la **concurrence**. Toute la conception du prédicat porte
là-dessus, et nulle part ailleurs.

### Deux remèdes refusés, avec leur raison

**`CARGO_TARGET_DIR` partagé (refusé).** Séduisant — éviter la production plutôt que
purger — et refusé sur trois motifs. (a) Cargo prend un **verrou exclusif** sur son
répertoire de build : deux pilotes concurrents se **sérialisent**, ce qui couple la
boucle entière à un mutex de build au moment même où `MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT`
existe pour la découpler. (b) Un target partagé entre branches divergentes **accumule**
les artefacts de toutes les branches et invalide en cascade ; rien ne garantit qu'il
soit plus petit, et cargo ne fait aucun GC. (c) C'est un changement structurel de la
performance de build, **non mesuré**, dans un ticket « 1 PR atomique ». À rouvrir avec
une mesure, jamais par intuition.

**Purge à la fin de chaque dispatch, dans `dispatch-lib.sh` (refusé).** Elle détruit
le cache de build entre l'implement et les itérations QA / CI-fix qui suivent **sur le
même worktree** : chaque itération repartirait de zéro. On échangerait du disque contre
de la latence de boucle sur le chemin **nominal**, alors que le défaut mesuré est un
résidu **nocturne**. Le vocabulaire du HALT-2 de mika#2420 dit d'ailleurs *« `cargo clean`
sélectif sur les worktrees **inactifs** »* — c'est la sélectivité qui fait le remède.

## Surface retenue : un troisième bras du tick `worktree_reap`

`crates/mika-agent/src/worktree_reaper.rs`, dans le **même tick** que le reaper, **après**
lui. Quatre raisons, dans l'ordre de leur poids :

1. **L'ordre est nécessaire.** Ce que le reaper vient de retirer n'existe plus ; le
   considérer pour une purge serait au mieux un no-op, au pire une course.
2. **La population est déjà calculée.** Elle est exactement l'ensemble des refus du
   reaper portant le motif `pr_open` — une donnée en mémoire à la fin du tick, sans une
   seule requête de plus.
3. **Tout le coût est déjà payé** : `git worktree list --porcelain`, l'unique
   `gh pr list` par dépôt, l'énumération de `/proc`, la sentinelle STOP, le cap par
   tick, la déduplication des refus, les audits. Un scan séparé paierait tout une
   seconde fois pour répondre à une question voisine.
4. **Les gardes de chemin existent et sont éprouvées** : `is_managed_worktree_path`
   (syntaxique) et `canonical_path_is_managed` (symlink, juste avant la disposition).

Cadence inchangée : `WORKTREE_REAP_CRON = "0 */10 * * * *"`. Aucun nouveau
`PeriodicScan`, aucune nouvelle row récurrente, aucune migration de schéma.

## Le prédicat — cinq termes conjonctifs, tous fail-safe vers *conserver*

Sur chaque worktree refusé `pr_open` par le reaper du même tick :

| # | terme | source de vérité | illisible ⇒ |
|---|---|---|---|
| P1 | le chemin est managé après canonicalisation | `canonical_path_is_managed` | conserver |
| P2 | `<worktree>/target/` existe, est un **répertoire**, n'est pas un lien symbolique | `symlink_metadata` | conserver |
| P3 | aucun processus vivant n'a son cwd sous le worktree | `LiveCwds` du tick | conserver |
| P4 | inactivité : le mtime le plus récent de `target/` est plus vieux que la fenêtre | `stat`, profondeur bornée | conserver |
| P5 | **aucun cargo ne travaille ici** : le verrou de build est libre | `flock` non bloquant | conserver |

`LiveCwds::Unavailable` conserve tout, exactement comme chez le reaper.

### Pourquoi P5 existe, et pourquoi il n'est pas de la sur-ingénierie

P3 est **connu pour être troué**, et mika#2420 l'écrit lui-même : un processus peut
travailler dans un worktree sans y avoir son cwd (`cargo --manifest-path`, `git -C`, un
éditeur lancé ailleurs). Chez le reaper, ce trou était couvert **par la conjonction** —
un tel processus travaille sur une branche dont la PR est ouverte (exclu par T4) ou
produit des modifications non committées (exclu par T7).

**Ici, cette couverture disparaît : la PR est ouverte par définition de la population.**
Il faut donc un terme qui rende ce que T4 apportait au reaper. P5 est précisément
celui-là : il répond à « un cargo travaille-t-il dans ce répertoire », indépendamment
du cwd et indépendamment de tout délai.

P4 et P5 ne sont pas redondants et leur ordre est le motif de maison de mika#2184 :
**le proxy filtre d'abord** (P4, quelques `stat`, écarte la quasi-totalité de la
population), **la mesure directe tranche ensuite** (P5, juste avant la suppression, sur
le seul candidat retenu — le dernier point où le refus est encore gratuit).

### P4 : un mtime borné, jamais une marche complète

`measure_tree_size` est budgété à 400 000 entrées et 2 s — un `target/` de 40 Go les
dépasse, et une marche tronquée rendrait un mtime **faux dans la direction dangereuse**
(sous-estimer la récence, donc purger un arbre actif).

Le mtime retenu est donc le **maximum sur un ensemble borné et déclaré** : `target/`,
ses enfants directs, et les enfants de ceux-ci (**profondeur 2**). Quelques dizaines de
`stat`, coût constant. Cette profondeur attrape `target/debug/.fingerprint`,
`target/debug/build`, `target/debug/deps` et `target/debug/incremental`, dont les mtimes
bougent à chaque recompilation d'unité — c'est-à-dire le signal recherché.

**Un mtime illisible, ou dans le futur (dérive d'horloge), sort le worktree de la
population** ; il ne l'y fait jamais entrer.

### La suppression, et ses trois gardes

`std::fs::remove_dir_all` — `git worktree remove` n'est pas applicable, le worktree
étant conservé. Trois gardes, vérifiées **dans cet ordre**, immédiatement avant :

1. le chemin canonicalisé se termine par `/target` ;
2. il est sous `MANAGED_WORKTREE_SEGMENT` après canonicalisation ;
3. ce n'est pas un lien symbolique.

Un échec de suppression est journalisé et n'échoue jamais le tick.

## Réglages

Aucune migration, aucune clé de config ; quatre variables d'environnement, avec le
parse maison à trois paliers (absent/vide → défaut ; illisible, `0` ou négatif → défaut
**plus un WARN nommant la valeur entre guillemets**).

- **`MIKA_TARGET_PURGE`** — kill-switch, **défaut armé**. `0` désarme sans redéploiement.
- **`MIKA_TARGET_PURGE_DISPOSITION`** — `armed` (défaut) | `observe`. En observation le
  bras mesure, journalise et **ne supprime rien** (`target_purge_would_dispose`).
  Patron mika#2249 : *la détection est inconditionnelle, seule la disposition est
  gatée.* Une valeur non reconnue **reste armée** avec un WARN la nommant — un
  désarmement par coquille sur un frein de disque serait la panne silencieuse que tout
  ceci ferme.
- **`MIKA_TARGET_PURGE_IDLE_SECS`** — fenêtre P4, **défaut `14400` (4 h)**. Bornée des
  deux côtés : en dessous, une boucle QA → CI-fix active enchaîne en minutes et se ferait
  purger son cache entre deux itérations ; au-dessus, la fenêtre nocturne de 8 h qui a
  produit l'incident cesse d'être mordue. Quatre heures laissent un facteur confortable
  sur l'une et l'autre borne.
- **`MIKA_TARGET_PURGE_MAX_PER_TICK`** — **défaut `2`**, budget **distinct** de celui du
  reaper (un budget partagé ferait manger au reaper le sien, ou l'inverse). Un
  `remove_dir_all` de 40 Go est une tempête d'E/S : ce cap est ce qui l'étale, même
  raison que chez mika#2420.

### Pourquoi une disposition propre, mais une sentinelle STOP **partagée**

Disposition propre parce que les deux létalités diffèrent d'un ordre de grandeur :
coupler forcerait l'opérateur à régler les deux sur la plus prudente, et donc à perdre
la purge dès qu'il veut observer le reaper.

Sentinelle **partagée** (`~/.mika/state/worktree-reap-stop`, mika#2329) parce que le
critère de mika#2498 est *« une décision distincte mérite un fichier distinct »* — et
qu'ici la décision d'urgence est **la même** : « arrête ce qui supprime dans les
worktrees ». Un opérateur en incident pose un fichier et les deux bras s'arrêtent. Le
court-circuit existant est en tête de tick, donc il couvre le nouveau bras sans une
ligne.

### Armé par défaut, et l'argument est celui de l'asymétrie

mika#2420 a shippé armé en s'appuyant sur mika#2272 : *« zéro était l'absence de mesure,
pas la présence de prudence »*. L'argument est plus fort ici : le défaut est un p1
disque mesuré à 83 %, et **le pire cas d'un faux positif est un rebuild**, pas une
perte. Ce qui paie la prudence est nommé et concret : les cinq termes fail-safe, la
garde directe P5, le mode observe, la sentinelle partagée, le kill-switch — et une
sonde qui prescrit de **commencer en `observe`** (§ Sondes).

## Unités d'implémentation

**U1 — le prédicat, en fonctions pures.** `select_target_purges(...) -> TargetPurgeSelection`
sur le modèle exact de `screen_worktrees` / `ReapSelection` : candidats + refus nommés,
aucune E/S réseau, aucun accès disque hors de ce qu'on lui passe. Plus
`newest_mtime_bounded(root, depth) -> Option<SystemTime>` et
`cargo_build_lock_is_free(target) -> LockProbe` (trois états : libre / tenu /
**inévaluable**, ce dernier conservant).

**U2 — la collecte et la disposition.** Branchement à la fin de `reap_terminal_worktrees`,
sur les refus `pr_open` du même tick. Suppression gardée, mesure de taille
best-effort via `measure_tree_size` (`bytes_reclaimed` est un `Option` — **`null` n'est
jamais `0`**, doctrine mika#2331).

**U3 — les motifs, en format de fil séparé.** `ALL_PURGE_REFUSAL_REASONS`, **distincte**
de `ALL_REFUSAL_REASONS` : deux populations comptables qui doivent rester
soustractibles, comme `phantom_aged_out` / `phantom_sweep_spared`. Valeurs :
`no_target_dir`, `target_not_a_dir`, `live_process`, `process_scan_unreadable`,
`recently_active`, `mtime_unreadable`, `build_lock_held`, `build_lock_unreadable`,
`outside_managed_root`.

**U4 — les surfaces.** Journal et `audit_events` (§ Surfaces opérateur).

**U5 — les tests.** Tous en `tempfile::tempdir()`, patron déjà en usage cinq fois dans
ce même fichier. **Aucun test ne touche quoi que ce soit hors de son tmpdir**, ce qui
est l'exigence explicite du DoD.

## Contrainte d'implémentation — **zéro sondage de l'hôte**

C'est la cause du parking 4/4 et c'est une contrainte dure sur le pilote, pas un conseil.

**INTERDIT** pendant l'implémentation : lire `/var/log/mika`, `~/.mika`, la base
`mika.db`, `~/.mika/state/*`, ou tout chemin hors du worktree. Le classifier rend un
**deny terminal** sur ces chemins, et la session meurt.

**Et ce n'est pas nécessaire :** tout ce que livre ce plan est du code de dépôt et des
tests sur des arborescences **fabriquées en tmpdir**. La sentinelle n'est pas lue par ce
bras (le court-circuit existant est en amont) ; la base n'est écrite que par les
helpers d'audit déjà en place ; le journal n'est écrit que par `tracing`.

**La vérification en service est un geste opérateur post-merge** (mesure `df` / `du` sur
l'hôte réel), jamais un geste du pilote. Elle est écrite au § Sondes pour l'opérateur,
et le pilote n'en exécute aucune ligne.

## Fire-Disposition

Ce plan livre des détecteurs — un pin de format de fil et un scan SOLE WRITER, dont le
chemin de succès est « aucune violation trouvée ». Disposition retenue :
**(a) exception nommée en allowlist, allowlist livrée VIDE.**

C'est l'option la moins coûteuse ici parce qu'**il n'y a aucune violation préexistante à
exempter** : tous les noms introduits (`target_purged`, `target_purge_skipped`,
`target_purge_would_dispose`, les neuf motifs) sont neufs, donc les deux détecteurs
naissent verts sur un arbre propre. Livrer désarmé (b) protégerait donc de rien, et
halte-et-remontée (c) n'a pas d'objet.

Deux détecteurs :

1. **`mika2497_les_motifs_de_purge_sont_un_format_de_fil`** — épingle les neuf valeurs
   et leur site de définition unique, sur le modèle de
   `mika2420_les_motifs_sont_un_format_de_fil`. Ces valeurs atterrissent dans
   `audit_events.after_value` et l'opérateur en fait des `GROUP BY` : deux orthographes
   d'un motif couperaient une population sans le dire.
2. **`mika2497_le_nom_de_purge_a_un_seul_ecrivain`** — scan de source, `target_purged`
   n'est écrit qu'à un site. Allowlist `TARGET_PURGE_WRITERS_ALLOWED` **livrée vide**,
   et un test frère épingle qu'elle le reste. **Quand ce scan tire, la résolution est de
   retirer le second écrivain, jamais de l'allowlister** (doctrine mika#2201).

Le scan porte son **assertion auto-nettoyante** : il échoue si le nom n'est écrit
**nulle part**, parce qu'un scan visant un nom mort vérifie zéro chose et se lit
exactement comme un scan propre (classe mika#2205).

## Vérification

Tous les tests sont déterministes, hors réseau, et confinés à un `tempfile::tempdir()`.

**V1 — le cas du DoD, littéralement.** Un worktree factice en tmpdir portant un
`target/` factice daté au-delà de la fenêtre : la purge le retire, et **le reste du
worktree est intact** (fichiers suivis, `.git`, `src/`) — cette seconde moitié est ce
qui distingue « la purge marche » de « la purge supprime trop ».

**V2 — les cinq refus, un test chacun, chacun vu rouge** : pas de `target/` ; `target`
qui est un fichier ; un cwd vif sous le worktree ; un `target/` dont un descendant de
profondeur 2 est **récent** ; un verrou de build tenu.

**V3 — contrôles négatifs qui séparent « le prédicat mord » de « le prédicat est une
constante »** : un `target/` récent **d'une seconde** à l'intérieur de la fenêtre est
conservé ; le même à une seconde au-delà est purgé. Sans cette paire, un prédicat
toujours-faux passerait V2 en entier.

**V4 — les gardes de chemin** : un `target` qui est un lien symbolique vers un arbre
voisin est refusé, et **la cible existe toujours** après le tick (l'assertion porte sur
la cible, pas sur le refus — c'est le seul faux positif irréversible de ce livrable).
Un chemin hors de la racine managée est refusé.

**V5 — observe ne supprime rien** : en `observe`, `target/` est **toujours là** et
l'audit écrit `target_purge_would_dispose`, jamais `target_purged`. C'est la correction
que mika#2469 a dû apporter à son aîné ; elle est prise d'emblée ici.

**V6 — disjonction avec mika#2420** : un worktree à PR **terminale** ne passe pas par ce
bras (le reaper l'a déjà retiré), et un worktree à PR **ouverte** n'est jamais retiré
par le reaper. Le test asserte que les deux populations ne s'intersectent pas.

**V7 — les détecteurs** (§ Fire-Disposition), plus `cargo fmt`, `cargo clippy`,
`cargo test -p mika-agent`.

## Surfaces opérateur

```bash
# 1. Qu'a-t-on purgé, et combien ça a rendu ?
grep target_purge "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{worktree_path, branch, pr_number, idle_secs, bytes_reclaimed}'

# 2. CONTRÔLE POSITIF — le bras tourne-t-il seulement ?
grep target_purge_tick "$MIKA_SPIRIT_LOG_FILE" | tail
```

```sql
-- Ce que la boucle a purgé (SOLE WRITER)
SELECT target_key, created_at FROM audit_events
 WHERE tool_name = 'target_purged' ORDER BY created_at DESC;

-- La distribution des refus : la forme réelle de la population
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'target_purge_skipped' GROUP BY 1 ORDER BY 2 DESC;

-- Ce qui AURAIT été purgé, en observation
SELECT target_key, created_at FROM audit_events
 WHERE tool_name = 'target_purge_would_dispose' ORDER BY created_at DESC;
```

| événement | niveau | régime attendu | lecture |
|---|---|---|---|
| `target_purge` | INFO | **non vide, quelques-uns par jour** | chaque ligne est du disque rendu sans geste humain |
| `target_purge_tick` | INFO | émis **seulement quand le tick agit** | son silence avec zéro purge ne prouve rien — voir Halte 2 |
| `target_purge_would_dispose` | INFO | non vide **en `observe` seulement** | la population du dry-run |
| `target_purge_failed` | WARN | **vide** | toute occurrence est une suppression refusée par le système de fichiers |

Les refus sont dédupliqués par `(worktree, motif)` sur 24 h, comme chez le reaper
(doctrine mika#2131) : l'information durable est « ce worktree est tenu par ce motif »,
pas « il l'était encore à 14 h 32 » — la vivacité est le rôle de l'agrégat par tick.
**Zéro purge et zéro refus ⇒ zéro ligne.**

## Sondes post-déploiement — **gestes opérateur, jamais du pilote**

**S0 — commencer en `observe`.** Poser `MIKA_TARGET_PURGE_DISPOSITION=observe` et lire
la population que le bras *retirerait*. Le cap par tick (`2`) vaut aussi en observation :
laisser tourner `ceil(N / 2)` ticks — jusqu'à ce qu'un tick ne nomme plus de worktree
que `SELECT DISTINCT target_key` n'ait déjà rendu — puis armer. Armer après un seul tick
retirerait les N−2 autres sans les avoir jamais vus en dry-run.

**S1 — le symptôme (7 jours).** `df -h /data` : la montée nocturne cesse de rapprocher
le seuil. `du -sh` sur la racine des worktrees : le total se stabilise nettement sous
les 165 Go mesurés.

**S2 — l'attribution.** La requête `target_purged` doit être non vide, et croiser des
worktrees dont la PR était bien **ouverte** — c'est la preuve que la population visée
est celle qui est servie, et non celle de mika#2420.

**S3 — non-régression de la boucle.** Aucune plainte de rebuild intempestif sur une
itération QA → CI-fix. Le signal direct est la distribution de `target_purge_skipped` :
`recently_active` doit **dominer** — c'est la fenêtre qui protège le travail en cours.

### Les quatre haltes

**Halte 1 — un build cassé par la purge.** `touch ~/.mika/state/worktree-reap-stop`
**immédiatement**, puis diagnostiquer. C'est le seul mode de panne coûteux, et il ne se
règle pas en bougeant un seuil : établir lequel de P3, P4 ou P5 a lu vrai alors qu'il
était faux.

**Halte 2 — zéro purge et zéro ligne du tout.** On ne peut **rien** conclure. Vérifier
d'abord le contrôle positif (`target_purge_tick`), puis que le binaire servi porte le
correctif (classe mika#2340) — *une ligne absente ne prouve rien tant qu'on n'a pas
établi que le binaire qui tourne sait l'écrire*. Zéro purge avec un tick qui agit est
sain ; zéro des deux ne prouve rien.

**Halte 3 — `/data` remplit encore alors qu'aucun worktree inactif ne porte de
`target/`.** **Ne pas raccourcir la fenêtre par réflexe** : ce serait purger le cache de
travail en cours pour un problème de dimensionnement. La cause est alors le **pic de
production simultanée**, que ce livrable ne borne pas (voir ci-dessous). Ouvrir le suivi
`CARGO_TARGET_DIR` partagé **avec la mesure**, pas avec l'intuition.

**Halte 4 — `build_lock_unreadable` ou `mtime_unreadable` domine.** Le bras est
silencieusement inerte par fail-safe : il tourne et ne purge jamais rien, ce qui se lit
exactement comme un disque sain (classe mika#2205). Établir pourquoi ces signaux sont
illisibles **avant** de toucher au prédicat.

## Ce que ce travail n'achète PAS

**Il borne l'accumulation, il ne borne pas le pic.** Si les N worktrees de la nuit du 22
compilaient tous réellement, aucun d'eux n'était inactif et la purge n'aurait rien
attrapé **pendant** la montée — elle attrape le résidu après. Borner le pic demande soit
un `CARGO_TARGET_DIR` partagé (refusé ci-dessus, avec sa raison), soit une limite de
concurrence de dispatch. **C'est la limite honnête de ce livrable, et c'est la Halte 3.**

Il ne mesure pas le disque et n'a aucun seuil de remplissage : il ne sait pas que
`/data` est à 83 %, il sait qu'un `target/` est inactif. Un déclencheur par pression
disque serait un autre mécanisme, avec sa propre population.

## Definition of Done

- Le `target/` d'un worktree **vif (PR ouverte) mais inactif** est purgé par un
  mécanisme du dépôt `mika`, **testé sur un worktree factice en tmpdir**.
- Zéro sondage de l'hôte dans l'implémentation : le classifier n'émet **aucun** deny
  hors bac à sable pendant l'implement.
- La vérification en service (mesure disque) est documentée comme **geste opérateur
  post-merge**, et n'est exécutée par aucun pilote.
- `cargo fmt`, `cargo clippy` et `cargo test -p mika-agent` passent.
- Aucune régression du reaper mika#2420 : ses tests passent sans modification.

## Acceptance criteria

1. **AC1 — la purge a lieu.** Un worktree managé, à PR ouverte, sans processus vif,
   dont le `target/` n'a pas été écrit depuis plus que la fenêtre et dont le verrou de
   build est libre, voit son `target/` retiré ; le reste du worktree est intact.
2. **AC2 — les cinq refus tiennent.** Chacun de P1–P5 conserve le `target/` quand il est
   faux, et chacun est vu rouge par un test dédié.
3. **AC3 — l'illisible conserve.** Un mtime, un verrou ou une énumération `/proc`
   illisibles sortent le worktree de la population ; ils ne l'y font jamais entrer.
4. **AC4 — disjonction avec mika#2420.** Les deux populations ne s'intersectent pas, et
   aucun comportement du reaper terminal ne change.
5. **AC5 — `observe` ne supprime rien** et écrit `target_purge_would_dispose`, jamais
   `target_purged`.
6. **AC6 — le rollback est atteignable sans redéploiement** : `MIKA_TARGET_PURGE=0`
   désarme, la sentinelle partagée court-circuite le tick.
7. **AC7 — les motifs sont un format de fil** à site de définition unique, épinglés par
   test, dans une liste **distincte** de celle du reaper.
8. **AC8 — `target_purged` a un seul écrivain**, épinglé par un scan de source à
   allowlist vide portant son assertion auto-nettoyante.
9. **AC9 — les tests ne sortent pas de leur tmpdir**, y compris sur les chemins d'échec.
10. **AC10 — zéro deny hors bac à sable** pendant la session d'implémentation.

## Hors périmètre, délibérément

- **`CARGO_TARGET_DIR` partagé** — refusé ci-dessus avec ses trois motifs ; **suivi**,
  dont la précondition est la Halte 3.
- **Le pic de production simultanée** et toute limite de concurrence de dispatch.
- **`scripts/mika-platform-worktree-cleanup`** — vit dans `mika-platform`, structurellement
  hors d'atteinte du pilote (Q2), et ce dépôt garde son invariant zéro-issue.
- **Le reaper terminal mika#2420** — inchangé, population disjointe.
- **Un déclencheur par pression disque** (`df` sous seuil) — autre mécanisme, autre
  population, aucune mesure ne le demande aujourd'hui.
- **La purge des `target/` du checkout principal** — ce n'est pas un worktree managé,
  et P1 l'exclut par construction.
