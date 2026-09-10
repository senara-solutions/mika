---
module: mika-agent/task_engine
tags: [reaper, pilot-silent-stall, liveness, false-positive, loop-substrate]
problem_type: wrong-predicate
category: bug
type: fix
issue: senara-solutions/mika#2277
---

# fix mika#2277 — le signal de vie du reaper D1 est le silence sur TOUTES les surfaces du pilote

**Issue :** senara-solutions/mika#2277 (p1, bug)
**Branche :** `bug/2277/task-engine-reaper-faux-positif-d1-le`
**Lignée :** mika#2249 (D1) → mika#2272/PR#2275 (armement) → **ce ticket** (le prédicat)

## Le défaut, mesuré

Le 2026-09-10 à 07:16:10Z — premier déploiement armé de #2275 — le reaper a
fauché **les deux seuls pilotes en vol**, tous deux vivants et productifs.

| row callback | ticket | pid | `worktree_idle_secs` | dernière écriture log pilote | dernière écriture transcript |
|---|---|---|---|---|---|
| `11cf415b` | #2023 | 192622 | 2758 ≥ 2700 | 07:12:30Z (T−3 min 40 s) | 07:12:30Z (T−3 min 40 s) |
| `4df0f177` | #2266 | 193912 | 2744 ≥ 2700 | 07:12:39Z (T−3 min 31 s) | 07:12:39Z (T−3 min 31 s) |

Sources : `audit_events` id 695392/695393 ; `ls` sur `/var/log/claude-pilot/` et
`~/.mika/data/pilot-transcripts/`.

La mécanique de #2275 est correcte — bonne row (`pending`, celle qui porte le
pid), bon pid, kill effectif, audit écrit avec `disposition_armed=true`. **Le
prédicat est faux.** « Aucune écriture dans le worktree » n'est pas « pilote
muet » : un pilote en phase lecture / exploration / planification ne touche
aucun fichier du worktree pendant que le SDK lui rend des tours en continu.

### Ce que la mesure réfute *en plus* du prédicat

`DEFAULT_PILOT_STALL_REAP_AGE_SECONDS` (`config.rs:1289`) documente une fenêtre
utilisable `]1800 s, 3480 s[`, dérivée d'un contrôle négatif : la course saine
`c3f9a2f9` avec un écart inter-écriture worktree de **24 minutes**, que 2700 s
« dégage d'un facteur deux ».

Ce contrôle négatif est réfuté. Les deux pilotes du 09-10 étaient sains avec des
écarts d'écriture worktree de **46 minutes**. Sur le signal worktree seul, la
borne haute des écarts sains n'est pas 24 min : c'est la durée d'une phase de
lecture, qui n'a pas de borne connue. **Il n'existe donc aucune fenêtre sûre
mesurée pour un prédicat worktree-seul** — ce n'est pas la fenêtre qu'il faut
élargir, c'est le prédicat qu'il faut changer.

### Le signal qui sépare, lui, mesuré

Les transcripts pilote (`~/.mika/data/pilot-transcripts/<task_id>.jsonl`,
mika#1705) horodatent **chaque tour LLM**. Mesure sur les deux faux positifs :

| row | tours | premier | dernier | écart inter-tours **max** |
|---|---|---|---|---|
| `11cf415b` | 31 | 06:30:16Z | 07:12:30Z | **387 s** |
| `4df0f177` | 27 | 06:30:30Z | 07:12:39Z | **376 s** |

387 s contre une fenêtre de 2700 s : la marge est d'un facteur **7**. Là où le
signal worktree n'a pas de séparation mesurée, le signal « le pilote consomme
encore des tours LLM » en a une large. C'est aussi la définition exacte de la
classe que D1 vise (mika#1901 : le flux SDK se tait), et non un proxy.

### Population du reaper à ce jour

`audit_events` ne contient que **deux** rows `pilot_silent_stall` — les deux
faux positifs ci-dessus. Aucun vrai positif n'a jamais été fauché. Les trois
cas fondateurs de mika#2249 ne se laissent pas re-mesurer sur ce signal : leurs
transcripts n'ont jamais été ingérés (`pilot_transcripts` est vide avant le
09-10), et les mtimes de leurs logs pilote (`b3accf50` : 19:38:21Z pour une
disposition manuelle à 19:43:58Z ; `969e91bb` : 20:40:02Z pour 20:43:19Z)
montrent un log **actif** à T−5 min et T−3 min. Deux des trois ont été disposés
à la main « après CHECK mtime-worktree » — c'est-à-dire diagnostiqués avec le
prédicat que ce ticket réfute.

Conséquence assumée, écrite ici pour qu'elle soit arbitrée plutôt que
découverte : **le prédicat corrigé peut n'avoir aucune population**. C'est
précisément ce que AC3 gate — le re-armement exige une preuve sur un vrai
stall, pas une échéance.

## La conception

Le kill n'a lieu que si le pilote est muet sur **toutes** ses surfaces
observables. Le prédicat devient une conjonction, chaque terme mesuré
indépendamment, et **aucun terme n'est vrai par défaut**.

| terme | surface | source du chemin | statut |
|---|---|---|---|
| 1–4 | pid utilisable, process vivant, worktree déclaré + lisible | existant (`engine.rs:1733-1783`) | inchangé |
| 5 | `worktree_idle > fenêtre` | `metadata.dispatch_worktree_file` | inchangé |
| **6** | `transcript_idle > fenêtre` | `metadata.pilot_transcript_expected` | **nouveau** |
| **7** | `pilot_log_idle > fenêtre` | `<pilot_log_dir>/<task.id>.log` | **nouveau** |
| 8 | row toujours sur une surface vivante (re-lecture) | existant | inchangé |

### Pourquoi ces deux sources de chemin, et pas une déclaration nouvelle

- **Le transcript est déjà estampillé sur la row que le reaper scanne.**
  `inject_pilot_transcript_env` (`executor.rs:167`) écrit
  `metadata.pilot_transcript_expected` sur le même `task_id` que
  `dispatch_worktree_file`. Vérifié sur les deux rows fauchées : le champ y est.
  Zéro nouvelle variable d'environnement, zéro nouveau bind sandbox, zéro
  changement dans `dispatch-lib.sh`.
- **Le log pilote se dérive, et sa dérivation ne peut échouer que dans le sens
  sûr.** `dispatch-lib.sh:2116` pose `LOG_ID="$TASK_ID"` et `:2598` écrit
  `$_PILOT_LOG_DIR/${LOG_ID}.log` ; vérifié empiriquement — les fichiers
  `11cf415b-….log` et `4df0f177-….log` portent l'id de la row callback. Le
  répertoire suit `${PILOT_LOG_DIR:-/var/log/claude-pilot}` (`dispatch-lib.sh:247`).
  mika#2165 a documenté le coût d'un override que seuls les lecteurs honorent —
  ici ce coût est borné : si le moteur regarde le mauvais répertoire, le fichier
  est absent, le signal est indisponible, et la règle ci-dessous met le dispatch
  **hors population**. Une divergence ne peut produire que de l'inertie, jamais
  un faux positif.

### La règle de sûreté, énoncée une fois

> Un signal indisponible n'est **jamais** un terme satisfait.

Fichier absent, illisible, sans mtime, mtime dans le futur, clé de metadata
absente : le dispatch sort de la population (`continue`), il ne devient pas
candidat. C'est la doctrine que `worktree_activity.rs` porte déjà en
en-tête — « un reaper qui tue ne doit jamais faire feu sur une absence de
preuve » — appliquée aux deux nouvelles surfaces sans exception.

Cette règle a une conséquence qu'il faut rendre visible : couper
`MIKA_LOG_PILOT_TRANSCRIPTS` désarme le reaper en silence. D'où AC4.

## Acceptance criteria

**AC1 — le prédicat de kill exige le silence sur toutes les surfaces.**
`reap_silently_stalled_pilots` ne dispose d'un pilote que si, en plus des termes
1–5 existants : (a) le transcript déclaré est muet depuis plus que la fenêtre,
**et** (b) le log pilote dérivé est muet depuis plus que la fenêtre. Un
transcript ou un log actif ⇒ pilote vivant ⇒ **aucun kill**, quel que soit le
mtime du worktree. Toute surface dont le signal est indisponible (clé absente,
fichier absent, illisible, sans mtime, mtime futur) met le dispatch **hors
population** — jamais un terme satisfait par défaut.

**AC2 (PORTE — tests négatifs OBLIGATOIRES).** Dans le harness eval, avec
contrôle positif et contrôles négatifs **dans la même suite** :

- **Contrôle positif :** worktree, transcript et log tous muets au-delà de la
  fenêtre, process vivant ⇒ SIGTERM émis, row `failed`, audit
  `pilot_silent_stall` écrit avec `transitioned=true`.
- **Négatif N1 (la forme de l'incident) :** transcript **frais**, log **frais**,
  worktree intact depuis > 2700 s ⇒ **aucune** détection armée, **aucun**
  SIGTERM (le process fixture est encore vivant après le tick), row toujours
  `pending`, **aucun** row d'audit `pilot_silent_stall`.
- **Négatif N2 :** transcript frais seul (log et worktree muets) ⇒ aucun kill.
- **Négatif N3 :** log frais seul (transcript et worktree muets) ⇒ aucun kill.
- **Négatif N4 :** signal indisponible — `pilot_transcript_expected` absent de
  la metadata, tout le reste muet ⇒ aucun kill.

N2 et N3 existent parce qu'un ET de trois termes ne se prouve pas en
neutralisant le tout : chaque terme doit être montré load-bearing
séparément. Le pilote qui implémente écrit ces tests, les lance, et **colle la
sortie dans le corps de la PR**.

**AC3 — re-armement gaté.** `MIKA_PILOT_STALL_REAP_ENABLED` reste à `0` dans
`~/.mika/.env` tant que AC2 n'est pas vert. Le défaut du code
(`DEFAULT_PILOT_STALL_REAP_ENABLED`) reste `true` — c'est le knob d'exploitation
qui tient le désarmement, pas une seconde bascule dans le code. La PR ne
retire pas le knob et n'en propose pas le retrait ; la remise à `1` est un geste
opérateur, après le déploiement du prédicat corrigé et une preuve sur un vrai
stall.

**AC4 — l'inertie est visible.** Quand un dispatch sort de la population parce
qu'un signal de vie est indisponible (et non parce qu'il est actif), le reaper
émet un `warn!` nommant le `task_id` et la surface manquante, au plus une fois
par dispatch. Sans quoi couper une feature d'observabilité désarme
silencieusement un mécanisme de sûreté — et la leçon de #2272 est précisément
qu'un compteur à zéro peut être l'absence de mesure et non l'absence de défaut.

**AC5 — l'audit dit ce qui a été mesuré.** Le row `pilot_silent_stall` et le
`warn!` portent les trois âges (`worktree_idle_secs`, `transcript_idle_secs`,
`pilot_log_idle_secs`) et non le seul worktree. Le row d'audit est la seule
surface par laquelle quiconque lit ce mécanisme ; il doit permettre de rejouer
la décision.

## Périmètre

**Dedans :** `crates/mika-agent/src/task_engine/engine.rs` (le prédicat, l'audit,
le `warn!`), `crates/mika-agent/src/task_engine/worktree_activity.rs` ou un
module frère (la sonde mtime d'un fichier unique), `crates/mika-common/src/config.rs`
(le défaut `PILOT_LOG_DIR` côté Rust, symétrique à `dispatch-lib.sh:247`), et le
harness eval `crates/mika-agent/tests/eval/`.

**Dehors :** `dispatch-lib.sh` et `skills/executor.rs` — aucune nouvelle
déclaration n'est nécessaire, les deux chemins sont déjà obtenables. La cause
profonde du stall SDK (D2, mika#1901) reste hors périmètre, comme dans #2249. Le
re-armement du knob est un geste opérateur, pas un livrable de la PR.

**Forge-gate :** `task_engine` = décision-core → PR **human-gated**, **Vincent
merge**. Pas d'auto-merge.

## Risques et questions ouvertes

1. **Le prédicat corrigé peut n'avoir aucune population.** Aucun cas mesuré à ce
   jour ne présente un transcript ou un log muet ≥ 2700 s. Si c'est durable, le
   reaper est un mécanisme correct sans gibier, et la classe #2249 demande une
   sonde différente. AC4 rend cette inertie lisible plutôt que silencieuse ; le
   verdict se prendra sur les rows d'audit, pas sur une intuition.
2. **Une fenêtre unique pour trois surfaces.** 2700 s est très large pour le
   transcript (facteur 7 sur l'écart max mesuré). Resserrer la fenêtre du
   transcript ferait détecter un vrai stall plus tôt, mais la population des
   écarts inter-tours n'est mesurée que sur deux courses. v1 garde une seule
   fenêtre ; le resserrement demande une mesure dédiée et son propre ticket.
3. **Deux des trois cas fondateurs de #2249 sont possiblement des faux
   positifs de diagnostic** (disposés à la main sur le prédicat worktree, log
   actif à T−5 min). Cela ne change pas ce fix, mais cela veut dire que la
   classe « stall SDK » est moins bien établie que n=3 le laissait croire. À
   porter dans le ticket de suivi si la question 1 se confirme.

## Fire-Disposition

Deux livrables de ce plan sont de classe détecteur. Ce que chacun fait **quand il
tire** est fixé ici, avant l'implémentation, plutôt que découvert au premier feu.

### Les tests négatifs N1–N4 (AC2)

- **Disposition : halt-and-surface.** Un N1..N4 rouge arrête la suite et rend la
  ligne d'assertion échouée. Il n'y a pas de mode « avertir et continuer » : ces
  quatre tests sont la porte de AC2, et un prédicat trop permissif est
  exactement le défaut que ce ticket répare.
- **Violations préexistantes : aucune, par construction.** Le prédicat conjonctif
  n'existe pas encore ; N1..N4 naissent avec lui. Il n'y a donc rien à
  grand-parenter, et aucune tolérance transitoire à prévoir.
- **Propagation : gate CI.** Les quatre tests vivent dans la suite eval de
  `mika-agent` et échouent la CI de la PR comme n'importe quel test. Le contrôle
  positif tombe sous la même règle : un contrôle positif rouge veut dire que le
  reaper ne fauche plus rien, ce qui est un défaut de même gravité qu'un faux
  positif.
- **Preuve exigée à la PR :** la sortie de `cargo test -p mika-agent` couvrant le
  contrôle positif et N1..N4 est collée dans le corps de la PR.

### Le `warn!` d'inertie (AC4)

- **Disposition : emit-and-continue.** Le `warn!` est de l'observabilité : il
  nomme le `task_id` et la surface dont le signal est indisponible, puis le
  dispatch sort de la population comme prévu. Il ne bloque pas le tick, ne
  change pas le statut de la row, et ne devient jamais lui-même une cause de
  disposition. Un mécanisme de sûreté qui s'arrête parce qu'il n'arrive pas à
  mesurer serait un pire défaut que celui qu'il signale.
- **Cadence : au plus une fois par dispatch.** Le reaper passe à chaque
  `DB_SCAN_INTERVAL_TICKS` ; sans borne, un dispatch aux transcripts coupés
  produirait une ligne par tick pendant toute sa vie. La borne suit le motif
  déjà en place pour le détecteur de transcript vide (`PILOT_TRANSCRIPT_REPORTED_KEY`,
  mika#2040 AC7) : une clé de metadata estampillée après le premier rapport.
- **Violations préexistantes : attendues, et c'est le but.** Tout dispatch lancé
  avec `MIKA_LOG_PILOT_TRANSCRIPTS` désactivé, ou avant que ce fix ne soit
  déployé, tombera hors population et émettra la ligne. Ce n'est pas du bruit à
  supprimer : c'est la mesure de combien de la flotte le reaper ne voit pas.
  Aucune tolérance, aucun grand-parentage — la ligne doit sortir dès le premier
  dispatch concerné.
- **Ce qui n'est pas un feu :** un dispatch écarté parce qu'une surface est
  **active** (le pilote est vivant) est le fonctionnement nominal et reste
  silencieux au niveau `warn`. Seule l'indisponibilité du signal se dit.

## Vérification

- `cargo test -p mika-agent` — la suite eval, contrôle positif + N1..N4 verts.
- `cargo clippy --all-targets -- -D warnings`.
- Relire le row d'audit produit par le contrôle positif : il doit porter les
  trois âges (AC5).
- Confirmer que `MIKA_PILOT_STALL_REAP_ENABLED=0` est toujours dans
  `~/.mika/.env` à la fin de la PR (AC3) — la PR ne le retire pas.
