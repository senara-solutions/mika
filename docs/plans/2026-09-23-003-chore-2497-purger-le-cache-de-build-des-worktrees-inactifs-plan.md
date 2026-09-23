# mika#2497 — Le cache de build d'un worktree inactif ne survit plus à son inactivité

> **Parent umbrella :** mika#2491 (Défaut 5). Enfant du cadre substrat — 1 PR atomique.

---

## 1. Le défaut, mesuré

Chaque dev-pilot et chaque vérification de rescue reconstruisent un `target/`
Rust de 15 à 50 Go dans leur worktree. Rien ne le purge tant que le worktree
vit. La nuit du 2026-09-22 : **+90 Go en 8 h**, `/data` à **83 %**, les
worktrees pesant **165 Go**. Au-delà de 85 % les builds s'arrêtent et la boucle
entière se bloque. Résolu à la main — cinq `target/` de tickets mergés
supprimés, `/data` retombé à 50 %.

C'est la deuxième fois en deux jours que ce disque est purgé à la main : le
2026-09-20, trois vagues dans la même journée ont produit mika#2420. Ce
ticket-ci est la suite nommée de celui-là.

---

## 2. Ce que la lecture du code déplace dans le ticket

C'est le premier livrable, et il change ce qu'il faut écrire.

### R1 — La moitié « tickets terminés » est **déjà livrée**, et la ré-implémenter serait nuisible

Le ticket propose « purge des `target/` des worktrees de tickets **terminés**
(mergé/fermé) dans le cron de hygiène disque ». Cette population a déjà son
mécanisme : **mika#2420**, mergé le 2026-09-20, retire le **worktree entier**
d'une PR terminale — `target/` compris — toutes les dix minutes
(`WORKTREE_REAP_CRON = "0 */10 * * * *"`, `crates/mika-agent/src/server/mod.rs:119`).

Poser un second mécanisme sur cette population donnerait deux scans dont le
plus récent **masquerait le silence** du plus ancien. C'est le mode de panne que
la maison nomme déjà sous sa forme générale : *un filet qui porte le trafic
nominal a en plus effacé le signal qui permettrait de le voir* (mika#2334). Ce
plan ne ré-implémente donc pas cette moitié.

### R2 — L'incident du 22/09 n'est **pas attribué**, et ce plan ne fabrique pas son attribution

Le ticket dit « nettoyé à la main (5 `target/` de tickets **mergés**) ». Si le
faucheur mika#2420 tournait armé, ces cinq worktrees auraient dû partir en
≤ 10 min (cap `MIKA_WORKTREE_REAP_MAX_PER_TICK = 3`, six ticks par heure). Leur
présence à la main de l'opérateur est donc un **fait à expliquer**, et il y a
deux mondes :

| monde | cause | remède |
|---|---|---|
| **A** — le faucheur est inerte | binaire servi antérieur au correctif (classe mika#2340), `MIKA_WORKTREE_REAP_DISPOSITION=observe`, `MIKA_WORKTREE_REAP=0`, ou sentinelle STOP posée | **déployer / armer**, pas écrire du code |
| **B** — le faucheur a refusé | un motif de `ALL_REFUSAL_REASONS` — `dirty`, `unpushed_commits`, `live_process`, `too_young`… | dépend du motif ; `dirty`/`unpushed_commits` dominants est la HALTE 3 de mika#2420, un défaut **en amont** |

**Aucun de ces remèdes n'est « écrire un second scan ».** Et la mesure qui
tranche n'est pas disponible depuis un pilote de grooming : le sandbox bwrap ne
monte que son propre worktree (`ls` de la racine gérée rend une seule entrée) et
la politique de permission refuse `df` et `du`. Cette attribution est donc une
**précondition d'implémentation** (V0 ci-dessous), avec sa halte — jamais une
hypothèse de ce plan.

### R3 — Le livrable est **orthogonal aux deux mondes**, et c'est ce qui le rend livrable

La question du purgeur n'est pas « cette PR est-elle terminale ? » mais **« ce
cache de build sert-il encore à quelqu'un ? »**. Les deux questions ont des
populations qui se recouvrent sans s'inclure :

- un worktree que le faucheur **retire** perd son `target/` avec lui — le
  purgeur n'a rien à y faire ;
- un worktree que le faucheur **refuse** garde son travail et peut perdre son
  cache. Cela couvre l'angle mort (d) nommé par la HALTE 2 de mika#2420 — le
  `target/` des PR **ouvertes**, la population que ce ticket-ci était désigné
  pour fermer — **mais aussi** `dirty`, `unpushed_commits` et `detached_head`,
  c'est-à-dire le monde B tout entier.

**L'espace est donc récupéré dans les deux mondes, et le purgeur ne dépend pas
du fonctionnement du faucheur.** C'est aussi la raison pour laquelle son
prédicat **ne consomme pas** les motifs de refus du faucheur : ce serait un
couplage à un mécanisme dont l'inertie est précisément l'hypothèse à écarter, et
tout motif ajouté au faucheur changerait la population du purgeur en silence.

### R4 — La population est plus étroite que « tous les worktrees »

Un worktree de **grooming** ne compile pas : `ls -d target` dans le worktree de
ce plan même rend `No such file or directory`. Seuls les worktrees d'implémentation
et ceux passés par `_rescue_verify_run` (`dispatch-lib.sh:7188`, qui fait
`cd "$wt_dir"` avant `cargo clippy`) portent un `target/`. Un worktree sans
`target/` sort de la population sans produire ni ligne ni refus.

---

## 3. L'asymétrie s'inverse, et c'est le cœur de la conception

mika#2420 écrit, au sujet de son propre fail-safe uniforme vers *conserver* :
**« cet arbitrage est local ; il ne se transporte pas. »** Le transporter ici
produirait un purgeur trop timide pour servir. Les erreurs du purgeur ne sont
pas celles du faucheur :

| erreur | coût | réversible ? |
|---|---|---|
| faux négatif — on garde un cache mort | quelques dizaines de Go pendant dix minutes | oui, rattrapé au tick suivant |
| faux positif — worktree inactif, cache purgé, rebuild plus tard | **un rebuild** (temps CPU), zéro donnée perdue | oui, automatiquement |
| faux positif — **un build tourne**, le cache est arraché en vol | **un dispatch cassé**, avec des erreurs de compilation fantômes | le dispatch est relancé, mais le diagnostic est coûteux |

**Une seule erreur est chère, et c'est la troisième.** Le fail-safe n'est donc
pas uniforme comme chez le faucheur : il est **ciblé sur la liveness**. Tous les
termes de liveness pointent vers *conserver* quand ils sont indécidables ; rien
d'autre n'a besoin de cette prudence, parce que le purgeur ne touche ni au code,
ni aux commits, ni à `.git`, ni au worktree lui-même — **il supprime un cache
régénérable et rien d'autre**.

Corollaire à écrire au site : la troisième erreur doit être **attribuable**. Un
`cargo build` dont le `target/` disparaît en vol produit des erreurs
incompréhensibles ; sans une ligne datée nommant le chemin purgé, l'opérateur
ne peut pas joindre « ce dispatch a échoué à 14 h 32 » à « son cache a été purgé
à 14 h 31 ». C'est cette moitié d'observabilité qui rend le risque acceptable,
pas une conviction sur le prédicat.

---

## 4. Ce qui est livré

### U1 — Le prédicat de liveness : trois termes conjonctifs, tous fail-safe vers *conserver*

Dans `crates/mika-agent/src/worktree_reaper.rs`, une fonction pure
`select_targets_to_purge(entries, live, now, cfg) -> TargetPurgeSelection`.

- **L1 — aucun processus vivant n'a son répertoire courant sous le worktree.**
  Réutilise `LiveCwds`, déjà collecté **une fois par tick** par
  `collect_live_cwds()` (`worktree_reaper.rs:1087`) : aucun coût ajouté.
  `LiveCwds::Unreadable` sort le worktree de la population, jamais ne l'y fait
  entrer. Couvre le cas nominal : `_rescue_verify_run` fait `cd "$wt_dir"`, et un
  `cargo` lancé à la main a le worktree pour cwd.
- **L2 — `target/` n'a pas bougé depuis la fenêtre.** Max des mtimes sur
  `target/` à **profondeur ≤ 2** : `target/`, ses enfants directs
  (`debug/`, `release/`, `.rustc_info.json`, `CACHEDIR.TAG`), et les enfants
  directs de ceux-là (`deps/`, `incremental/`, `build/`, `.cargo-lock`). On
  **stat les répertoires sans y descendre** : un répertoire voit son mtime bouger
  dès qu'une entrée y est créée, supprimée ou renommée, ce que fait toute
  compilation. Coût borné à quelques dizaines de `stat`, sans budget d'entrées à
  épuiser.
- **L3 — le worktree **hors** `target/` n'a pas bougé depuis la fenêtre.**
  Réutilise `task_engine::worktree_activity::scan` (mika#2249), qui exclut
  `target/` par construction (`EXCLUDED_DIR_NAMES`) et répond donc exactement à
  « quelqu'un écrit-il du **code** ici ». C'est la ceinture de L1 contre son
  angle mort : un processus peut travailler dans un worktree sans avoir son cwd
  dedans (`git -C`, `cargo --manifest-path`), limite que mika#2449 a déjà dû
  nommer pour sa propre garde.

**Une marche tronquée sort le worktree de la population.** Un max partiel est un
**minorant** du vrai max : il fait paraître le worktree **plus inactif** qu'il
n'est, donc dans le sens de la purge. C'est la direction opposée à celle que le
site voisin peut se permettre, et elle doit être écrite au site sous peine
d'être « harmonisée » un jour par un relecteur.

### U2 — La disposition : `remove_dir_all`, pas `cargo clean`

`std::fs::remove_dir_all(<worktree>/target)`. Pourquoi pas `cargo clean`, que le
ticket propose : il exige un toolchain résolvable et un `Cargo.toml` lisible — ce
qu'un worktree en cours de rebase peut ne pas avoir — et prend le lock cargo,
donc il échoue précisément là où le retrait doit réussir. L'effet sur le disque
est identique.

Le retrait est borné par le **cap par tick** existant, pour la même raison que
chez le faucheur : un `remove_dir_all` sur quelques centaines de milliers de
fichiers est une tempête d'E/S.

### U3 — Les réglages, et lesquels sont partagés

| clé | défaut | portée |
|---|---|---|
| `MIKA_WORKTREE_TARGET_PURGE` | `1` | kill-switch **propre** : `0` désarme le purgeur sans toucher au faucheur |
| `MIKA_WORKTREE_TARGET_PURGE_DISPOSITION` | `armed` \| `observe` | disposition **propre** |
| `MIKA_WORKTREE_TARGET_PURGE_IDLE_SECS` | `21600` (6 h) | fenêtre d'inactivité, appliquée à **L2 et L3** |
| `MIKA_WORKTREE_TARGET_PURGE_MAX_PER_TICK` | `2` | cap **propre** sur les purges |
| `MIKA_WORKTREE_REAP_REPO_DIRS` | *hérité* | mêmes checkouts, une seule liste |
| sentinelle `worktree-reap-stop` | *partagée* | voir ci-dessous |

Les quatre clés numériques et booléennes suivent le parse maison à trois
paliers : absent/vide → défaut ; illisible, `0` ou négatif → défaut **plus un
WARN nommant la valeur entre guillemets**. Le `0` ne désarme pas — c'est le rôle
du kill-switch, et l'inverse ferait d'une faute de frappe un désarmement
silencieux sur une opération destructive. Une disposition non reconnue **reste
armée** avec un WARN, exactement comme celle du faucheur.

**La sentinelle STOP est partagée, les autres leviers ne le sont pas**, et les
deux moitiés de cet arbitrage sont raisonnées. Le critère de mika#2420 est
« une décision distincte mérite un fichier distinct », et mika#2498 l'a précisé
en élargissant le **sens** d'une sentinelle sans en créer une seconde : le
critère n'est pas « suis-je un scan distinct ? » mais « suis-je la même décision
d'opérateur ? ». Pendant un incident disque, le geste est un seul —
*arrête de supprimer des choses dans mes worktrees* — et un second fichier le
scinderait en deux, celui de la mémoire musculaire rendant alors le comportement
d'aujourd'hui en ayant l'air d'arrêter. Le kill-switch et la disposition, eux,
sont des décisions **durables** sur des blast radius différents : on peut
légitimement vouloir le faucheur armé et le purgeur en observation, ou l'inverse.

### U4 — Le choix de la fenêtre, et ce qui le contraint des deux côtés

**6 h.** Borne basse : un cycle de dispatch nominal (dev-pilot, puis build
callback QA, puis une itération éventuelle) tient dans quelques heures, et une
fenêtre plus courte purgerait un cache entre deux étapes du même cycle — ce qui
transformerait un problème d'espace en un problème de **temps de build sur le
chemin critique de la boucle**. Borne haute : le rythme mesuré est de ~8 %/h de
disque, donc une fenêtre de 24 h laisserait un cache mort occuper 45 Go pendant
une journée entière et ne fermerait pas le défaut.

La sonde S3 mesure la distribution réelle des âges au moment de la purge et
**décide** si ce chiffre tient. Il est choisi contre un ordre de grandeur mesuré,
pas contre une rondeur, et il est réglable sans redéploiement.

### U5 — Le site : greffé sur le faucheur, pas un cinquième scan

Le purgeur est une phase supplémentaire de `reap_terminal_worktrees`, pas un
scan récurrent de plus. Six mécanismes lui sont ainsi acquis sans une ligne :
l'énumération des worktrees (`parse_worktree_registry`), `LiveCwds` collecté une
fois par tick, le court-circuit STOP en tête de
`dispatcher::dispatch_worktree_reap`, la résolution de jeton PAT-first/App
(mika#2205), la déduplication des refus sur 24 h, et la cadence de dix minutes.

**Précédent maison direct** : mika#2449 a greffé sa sonde `main_checkout_dirty`
sur ce même faucheur plutôt que d'ouvrir un scan, avec le même argument.

L'ordre dans le tick est **purge après retrait** : un worktree que le faucheur
vient de retirer n'a plus de `target/` à purger, et l'inverse ferait mesurer puis
purger un arbre qui va disparaître dans la seconde.

### U6 — Le vocabulaire d'audit, format de fil

Quatre constantes d'un seul site, sur le modèle de `ALL_REFUSAL_REASONS` :

| `tool_name` | écrit quand |
|---|---|
| `worktree_target_purged` | un cache a été supprimé (disposition `armed`) |
| `worktree_target_would_purge` | un cache est éligible (disposition `observe`) — motif mika#2469 : **en observation, la ligne dit ce qu'elle ferait, jamais « purgé »** |
| `worktree_target_purge_skipped` | un refus, motif dans `after_value`, dédupliqué par `(worktree, motif)` sur 24 h |

Motifs de refus, énumérés en un seul lieu :
`live_process`, `target_recent`, `worktree_recent`, `activity_unreadable`,
`no_target`, `outside_managed_root`.

`activity_unreadable` est délibérément **distinct** de `target_recent` : écrire
« récent » pour un signal illisible serait une ligne d'audit **fausse**, et un
opérateur qui compte les transitoires compterait un blocage permanent parmi eux.
Même raison que les trois motifs `*_unreadable` du faucheur, et que
`unknown_provider` (mika#2328).

`no_target` sort la population de grooming (R4) sans bruit : elle n'est jamais
candidate, et une ligne par worktree de groom par tick serait le churn que la
doctrine mika#2131 borne.

### U7 — Documentation

Une entrée `CLAUDE.md` sous la section du faucheur terminal : les quatre clés,
la sentinelle partagée et pourquoi, les trois `tool_name`, les greps, les sondes
et leurs haltes. Plus une correction d'une phrase de l'entrée mika#2420 : sa
HALTE 2 prescrit « ouvrir le ticket de suivi » — ce ticket **est** ce suivi, et
la phrase doit le nommer plutôt que continuer d'appeler à l'ouvrir.

---

## 5. Alternatives refusées

**`cargo clean` au `git push` du pilote** (première proposition du ticket).
Trois refus superposés. *(a)* Le pilote **ne pousse pas** — `_push_branch` de
`dispatch-lib.sh` est le site unique de push pour un dispatch, par décision
mika#1407 ; le geste serait donc dans dispatch-lib, pas dans le pilote. *(b)*
Surtout, le push est l'instant **précédant immédiatement** le consommateur
suivant du cache : la PR s'ouvre, le build callback QA arrive, puis les
itérations `block[ci]` / `block[ac]`. Purger là, c'est garantir que chaque
itération reparte de zéro — on échange un problème d'espace contre un problème
de temps, sur le chemin critique de la boucle. *(c)* Un hook ne rattrape jamais
la population **déjà accumulée**, propriété qui a mis #1694 en échec et que
mika#2420 a dû écrire.

**`CARGO_TARGET_DIR` partagé entre worktrees.** Cargo prend un lock **exclusif**
sur son répertoire de sortie : deux pilotes concurrents (le cap mika#2160 autorise
N > 1) et une vérification de rescue se sérialiseraient sur ce lock, ce qui
transforme un problème d'espace en une sérialisation invisible des builds. Et un
`target/` partagé entre branches divergentes thrashe son propre cache — il
déplace le volume au lieu de le réduire, en ajoutant du rebuild. **Ticket de
suivi** (avec `sccache`, qui a un partage sans lock exclusif et est la vraie
réponse à la *cause* du volume) ; hors périmètre ici, et la raison est écrite.

**Consommer les motifs de refus du faucheur comme population.** Élégant — le
refus de l'un serait la population de l'autre — et refusé par R3 : ce serait un
couplage à un mécanisme dont l'inertie est l'hypothèse à écarter.

**Prendre le lock cargo (`flock` sur `target/{debug,release}/.cargo-lock`) comme
terme de liveness.** Donnerait « un build tourne **en ce moment** » sans
heuristique. Refusé comme redondant : à une fenêtre de 6 h, un build en cours a
nécessairement touché `target/` il y a moins de six heures, donc L2 le voit déjà ;
et prendre un verrou qu'on ne possède pas est une intrusion dans le protocole
d'un autre outil pour un terme que la conjonction couvre.

**Réutiliser `measure_tree_size` pour la récence.** Son budget est de 400 000
entrées : un `target/` de 40 Go le dépasse, la marche serait tronquée **à coup
sûr**, et avec la règle « tronqué → conserver » le purgeur ne purgerait jamais
rien. Un détecteur structurellement inerte se lit exactement comme un détecteur
sain (mika#2205) — c'est le piège que la profondeur ≤ 2 de L2 évite.

---

## 6. Fichiers touchés

| fichier | nature |
|---|---|
| `crates/mika-agent/src/worktree_reaper.rs` | le prédicat, la disposition, les constantes de fil, les audits, les tests |
| `crates/mika-agent/src/task_engine/worktree_activity.rs` | **aucune modification de comportement** — lecture seule depuis L3 ; un test de couplage y est ajouté |
| `CLAUDE.md` | U7 |
| `docs/plans/…-2497-…-plan.md` | ce plan |

`dispatcher.rs` n'est pas touché : le purgeur vit **dans**
`reap_terminal_worktrees`, dont le site d'appel, le STOP et le jeton sont déjà en
place.

---

## 7. Périmètre non couvert, nommé avec sa raison

- **La cause du volume** — l'absence de cache de compilation partagé. Ce travail
  borne la **durée de vie** d'un `target/`, il ne réduit pas le volume produit.
  Ticket de suivi (`sccache`), refus de `CARGO_TARGET_DIR` argumenté au § 5.
- **`dashboard/node_modules/`** — autre consommateur, d'un ordre de grandeur en
  dessous (~500 Mo contre 15–50 Go), et son coût de reconstruction est réseau et
  non CPU. Le ticket parle de `target/` ; l'élargir serait une seconde décision.
- **Les worktrees des autres dépôts de la plateforme** — couverts dès que leur
  checkout est dans `MIKA_WORKTREE_REAP_REPO_DIRS`, qui porte `mika` seul par
  défaut. Une ligne de configuration, délibérément non anticipée.
- **Le faucheur mika#2420 lui-même** — son prédicat, ses motifs et sa disposition
  sont inchangés. Si V0 établit le monde A, le remède est un geste de
  déploiement, pas une ligne de ce plan.
- **La HALTE 3 de mika#2420** (`dirty` / `unpushed_commits` dominants) — du
  travail non livré sur des PR mergées, donc un défaut de la recovery mika#1282,
  en amont. Le purgeur récupère leur espace ; il ne répare pas leur cause.

---

## 8. Surfaces opérateur

```bash
# Le purgeur mord-il ? (régime attendu : NON VIDE, quelques lignes par jour)
grep worktree_target_purged "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{worktree_path, branch, bytes_reclaimed, target_idle_secs, worktree_idle_secs}'

# En observation : ce qu'il FERAIT (mika#2469 — jamais « purgé »)
grep worktree_target_would_purge "$MIKA_SPIRIT_LOG_FILE" | jq -c '{worktree_path, bytes_reclaimed}'

# Pourquoi ce cache survit-il ? (distribution des motifs)
grep worktree_target_purge_skipped "$MIKA_SPIRIT_LOG_FILE" | jq -r .reason | sort | uniq -c

# CONTRÔLE POSITIF — le scan tourne-t-il seulement ?
grep worktree_reap_tick "$MIKA_SPIRIT_LOG_FILE" | tail -1
grep worktree_reap_no_checkout "$MIKA_SPIRIT_LOG_FILE" | tail -1   # attendu : vide sur gentux
```

```sql
-- Combien d'espace la boucle s'est-elle rendue, et sur quels worktrees ?
SELECT target_key, created_at, reasoning FROM audit_events
 WHERE tool_name = 'worktree_target_purged' ORDER BY created_at DESC;

-- La population que l'observation nommerait (disposition = observe)
SELECT target_key, created_at FROM audit_events
 WHERE tool_name = 'worktree_target_would_purge' ORDER BY created_at DESC;

-- Distribution des refus
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'worktree_target_purge_skipped' GROUP BY 1 ORDER BY 2 DESC;

-- ATTRIBUTION d'un dispatch cassé : le cache a-t-il été purgé sous ses pieds ?
SELECT created_at, target_key FROM audit_events
 WHERE tool_name = 'worktree_target_purged'
   AND created_at BETWEEN '<début du dispatch>' AND '<son échec>';
```

| événement | niveau | régime attendu | lecture |
|---|---|---|---|
| `worktree_target_purged` | INFO | **non vide, faible** | chaque ligne est de l'espace rendu sans geste manuel |
| `worktree_target_would_purge` | INFO | non vide en `observe`, **vide en `armed`** | la population de pré-vol |
| `worktree_target_purge_skipped` | INFO | non vide, dominé par `target_recent` | la distribution **est** la sonde S3 |
| `worktree_target_purge_failed` | WARN | **vide** | un `remove_dir_all` qui échoue |
| `worktree_target_activity_unreadable` | WARN | **vide** | un signal de liveness illisible, donc un purgeur devenu inerte |

---

## 9. Sondes post-déploiement, et leurs six haltes

### V0 — Attribution de l'incident, **avant toute ligne de code** (R2)

Sur l'hôte, établir dans quel monde on est :

```bash
grep -E 'worktree_reap_(tick|stop_armed|no_checkout|failed)' "$MIKA_SPIRIT_LOG_FILE" | tail -20
grep worktree_reap_skipped "$MIKA_SPIRIT_LOG_FILE" | jq -r .reason | sort | uniq -c
ls ~/.mika/state/worktree-reap-stop 2>/dev/null && echo "STOP POSÉ"
```
```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'worktree_reap_skipped' AND created_at > '2026-09-22' GROUP BY 1 ORDER BY 2 DESC;
SELECT count(*) FROM audit_events WHERE tool_name = 'worktree_reaped' AND created_at > '2026-09-22';
```

**HALTE 1 — aucune ligne `worktree_reap_tick` et aucun `worktree_reaped`.**
Monde A : le faucheur n'a pas tourné. **Le corriger est le premier geste, et il
n'est pas dans ce plan** — établir le déploiement (classe mika#2340), la
disposition, le kill-switch et la sentinelle **avant** d'écrire une ligne du
purgeur. Un purgeur livré par-dessus un faucheur inerte récupère de l'espace
tout en laissant croire que le faucheur marche.

**HALTE 2 — `dirty` ou `unpushed_commits` dominent la distribution des refus.**
C'est la HALTE 3 de mika#2420 : du travail non livré sur des PR mergées, donc un
défaut de la recovery mika#1282, **en amont**. Le purgeur récupérera leur espace
— c'est même son intérêt dans le monde B — mais **ne pas retirer ces worktrees à
l'aveugle** et ouvrir le suivi avec la distribution.

### V1 — Vérification empirique de L2, **avant de figer la profondeur**

Sur un worktree portant un `target/`, relever les mtimes à profondeur ≤ 2,
lancer un `cargo build` incrémental d'une seule ligne modifiée, relever à
nouveau.

**HALTE 3 — aucun mtime à profondeur ≤ 2 n'a bougé.** Le terme L2 est aveugle
aux builds incrémentaux et purgerait un cache en cours d'usage. **Ne pas élargir
la profondeur par réflexe** (le budget d'entrées revient et avec lui le piège du
§ 5) : le repli est de **conjoindre `target/{debug,release}/.cargo-lock`**, que
cargo touche à l'acquisition du lock, et de le documenter comme le signal
porteur plutôt que comme une ceinture.

### V2 — Le purgeur mord (48 h)

`worktree_target_purged` non vide, `/data` cesse de franchir 80 % en régime
nominal.

**HALTE 4 — la ligne est vide alors que des `target/` anciens subsistent.** Lire
la distribution de `worktree_target_purge_skipped` **avant** de toucher à la
fenêtre : un `activity_unreadable` dominant dit que le purgeur est inerte par
fail-safe et c'est le signal qu'il faut réparer, pas le seuil. Et vérifier que le
binaire servi porte le correctif (classe mika#2340) — *une ligne absente ne
prouve rien tant qu'on n'a pas établi que le binaire qui tourne sait l'écrire*.

### V3 — Aucun dispatch cassé (contrôle négatif, 7 jours)

Aucun dispatch en échec dont le worktree porte une ligne
`worktree_target_purged` **pendant** sa fenêtre d'exécution (la dernière requête
SQL du § 8 est exactement cette jointure).

**HALTE 5 — une correspondance est trouvée.** `touch ~/.mika/state/worktree-reap-stop`
**immédiatement**, puis diagnostiquer : c'est la seule erreur chère, et elle
nomme lequel des trois termes de liveness a rendu vrai ce qui ne l'était pas.
**Ne pas rallonger la fenêtre par réflexe** — si L1 est aveugle aux pilotes
sandboxés (le `cwd` d'un processus sous bwrap se résout dans son propre espace
de noms de montage), aucune valeur de fenêtre ne ferme la classe et c'est L1
qu'il faut réparer.

### V4 — La fenêtre est bien dimensionnée (S3, 7 jours)

Distribution de `target_idle_secs` au moment de la purge, et distribution des
refus `target_recent`.

**HALTE 6 — un volume notable de purges sur des caches âgés de peu plus de six
heures, suivies d'un rebuild dans l'heure.** La fenêtre est trop courte pour le
cycle réel : la relever, avec cette distribution comme justification. Le symptôme
inverse — `target_recent` écrasant tout et presque aucune purge — dit que la
population est intégralement active et que le défaut est ailleurs.

---

## 10. Contrat de vérification (tests)

**Comportementaux, sur la fonction pure** (`select_targets_to_purge`) :

1. Worktree inactif au-delà de la fenêtre, sans processus, avec `target/` →
   **candidat**.
2. `LiveCwds` porte un cwd sous le worktree → refus `live_process`.
3. `LiveCwds::Unreadable` → refus, **jamais candidat** (fail-safe).
4. `target/` touché dans la fenêtre → refus `target_recent`.
5. Code du worktree touché dans la fenêtre, `target/` ancien → refus
   `worktree_recent` (L3 est bien conjonctif, et non une ceinture inerte).
6. Marche de L3 tronquée → refus `activity_unreadable`, **jamais candidat** —
   c'est l'inversion de direction du § U1, et sans ce test elle se perdrait à la
   première relecture.
7. Pas de `target/` → refus `no_target`, et **aucune ligne de refus écrite**
   (R4 : la population de grooming est silencieuse).
8. Chemin hors de la racine gérée → refus `outside_managed_root`.
9. **Contrôle négatif de bonne foi** : avec tous les termes relâchés, le même
   jeu de worktrees rend des candidats — sans quoi les huit tests ci-dessus
   seraient verts sur un prédicat constamment faux.

**Sur la disposition :**

10. `observe` écrit `worktree_target_would_purge`, **aucune ligne**
    `worktree_target_purged`, et `target/` **est toujours là** — motif mika#2469.
11. `armed` supprime `target/` et **laisse intacts** `.git/`, les fichiers
    source et l'index — c'est l'assertion qui sépare ce purgeur du faucheur.

**Structurels :**

12. Les trois `tool_name` ont **un seul writer** dans le crate (scan de source,
    allowlist vide, modèle `mika2420_le_tool_name_daudit_a_un_seul_writer`).
13. Les motifs de refus sont un **format de fil** : liste figée, pas de doublon,
    renommer est une rupture à dater dans `CLAUDE.md` (modèle
    `ALL_REFUSAL_REASONS`).
14. **Couplage à `worktree_activity`** : `EXCLUDED_DIR_NAMES` contient `target`.
    Si ce voisin cessait d'exclure `target/`, L3 verrait l'activité de build comme
    de l'activité de code et le purgeur **ne purgerait plus jamais rien**, en
    silence. Miroir exact de la garde que mika#2420 porte déjà (`worktree_reaper.rs:2621`)
    pour l'exigence inverse.

---

## Fire-Disposition

Ce plan livre trois détecteurs — les tests 12, 13 et 14 ci-dessus, dont le
chemin de succès est « aucune violation trouvée ».

**Option retenue : (a) exception nommée en allowlist, livrée VIDE.**

Les trois portent sur une population **neuve** : les `tool_name`
(`worktree_target_purged`, `worktree_target_would_purge`,
`worktree_target_purge_skipped`) et les six motifs de refus n'existent nulle part
dans l'arbre avant cette PR, et `EXCLUDED_DIR_NAMES` contient déjà `target`.
**Il n'y a donc aucune violation existante à déclarer**, et les trois gardes
atterrissent **armées et vertes**.

L'allowlist du scan de source (test 12) est livrée vide et le reste, avec la
règle écrite au site, sur le modèle de `ACTOR_READING_PREDICATES_ALLOWED`
(mika#2323) : **quand la garde tire, on retire le second writer ; on n'y ajoute
pas une entrée.** Un test refuse qu'elle cesse d'être vide.

Aucune assertion auto-nettoyante n'est requise, puisqu'il n'y a pas d'exception à
périmer.

---

## Definition of Done

- Le `target/` d'un worktree inactif est purgé automatiquement, **quel que soit
  l'état de sa PR** — ce qui couvre la population du ticket (tickets terminés,
  quand le faucheur les refuse) **et** l'angle mort (d) de mika#2420 (PR
  ouvertes).
- Aucun `target/` n'est purgé pendant qu'un build tourne : les trois termes de
  liveness sont conjonctifs et chacun est fail-safe vers *conserver*.
- Le purgeur ne touche ni au code, ni aux commits, ni à `.git`, ni au worktree.
- Trois leviers : kill-switch propre, disposition propre, sentinelle STOP
  partagée avec le faucheur.
- Trois `tool_name` d'audit, six motifs de refus, chacun à écrivain unique et
  figé comme format de fil.
- V0 exécutée et son monde établi **avant** l'implémentation ; si monde A, le
  faucheur est réparé d'abord.
- V1 exécutée : la profondeur de L2 est validée empiriquement, ou le repli
  `.cargo-lock` est appliqué et documenté.
- `CLAUDE.md` porte l'entrée U7, HALTE 2 de mika#2420 corrigée pour nommer ce
  ticket au lieu d'appeler à l'ouvrir.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test -p mika-agent` verts.
- `scripts/verify-pipeline.sh origin/main` vert.

---

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de son DoD (« `target/` d'un worktree de ticket terminé
purgé automatiquement, testé ») et des exigences de ce plan.

- **AC1** — Un worktree géré dont le `target/` et le code n'ont pas bougé depuis
  plus de `MIKA_WORKTREE_TARGET_PURGE_IDLE_SECS`, et dans lequel aucun processus
  vivant n'a son répertoire courant, voit son `target/` supprimé par le scan,
  **sans que son code, ses commits, son `.git/` ou le worktree lui-même soient
  touchés**. Couvert par les tests 1 et 11.
- **AC2** — Un worktree dans lequel un build tourne, ou a tourné dans la
  fenêtre, ou dont un signal de liveness est illisible ou tronqué, n'est
  **jamais** purgé. Couvert par les tests 2 à 6, avec le contrôle négatif 9.
- **AC3** — En disposition `observe`, le scan mesure et journalise sous
  `worktree_target_would_purge` et **ne supprime rien** ; aucune ligne
  `worktree_target_purged` n'est écrite. Couvert par le test 10.
- **AC4** — `MIKA_WORKTREE_TARGET_PURGE=0` désarme le purgeur **sans** désarmer
  le faucheur terminal, et la sentinelle `worktree-reap-stop` court-circuite les
  deux d'un seul geste.
- **AC5** — Chaque purge écrit une ligne d'audit datée portant le chemin du
  worktree, de sorte qu'un dispatch en échec puisse être joint à la purge de son
  cache par une seule requête SQL (§ 8).
- **AC6** — Les trois `tool_name` et les six motifs de refus ont un écrivain
  unique et sont figés comme format de fil, gardés par les tests 12 et 13.
- **AC7** — Si `worktree_activity::EXCLUDED_DIR_NAMES` cessait de contenir
  `target`, le test 14 rougit au lieu de laisser le purgeur devenir inerte en
  silence.
