---
title: Un faucheur qui lit un indice au lieu de lire le processus — et pourquoi allonger son seuil ne le répare pas
date: 2026-09-04
category: best-practices
module: crates/mika-agent/src/task_engine, crates/mika-agent/src/db.rs, crates/mika-common/src/config.rs
tags: [reaper, watchdog, liveness, pid-reuse, fail-open, destructive-write, threshold, telemetry]
problem_type: silent-logic-error
component: task-engine
severity: high
applies_when:
  - "Un balayeur périodique écrit un état terminal (failed/expired) à partir d'un critère indirect"
  - "On envisage d'allonger un seuil pour arrêter des faux positifs"
  - "Une garde est insérée avant une écriture destructrice et doit choisir sa direction d'échec"
  - "On lit la vivacité d'un processus depuis un couple (pid, start_time) stocké en base"
related: [mika#2156, mika#1712, mika#959, mika#1687, mika#2169]
---

# Un faucheur qui lit un indice au lieu de lire le processus

## Le défaut, dans sa forme générale

`sweep_null_pid_phantoms` marquait `failed` toute fiche de suivi dont
`updated_at` dépassait un seuil. Ses trois autres critères — `action_type='none'`,
`process_id IS NULL`, `status IN ('in_progress','blocked')` — sont satisfaits
**par construction** pour toute fiche saine : le processus réel vit sur une
ligne de rappel séparée. Il ne restait donc que l'âge comme discriminant.

Et l'âge ne mesurait pas ce qu'on croyait. `updated_at` de la fiche **n'est
jamais rafraîchi pendant que le dispatch travaille**. Le seuil mesurait « temps
depuis la dernière écriture sur cette ligne », pas « temps depuis le dernier
signe de vie du travail ». Ces deux quantités ne coïncident que lorsque le
travail est mort — c'est-à-dire exactement dans le cas où le faucheur n'a pas
besoin d'être juste.

Mesure fondatrice : une session écrivait encore **2 h 08 après** avoir été
déclarée `phantom_aged_out`.

## La leçon qui se transporte

**Un indice corrélé à la mort n'est pas une mesure de la mort.** Quand un
mécanisme écrit un état terminal, son discriminant doit interroger la chose
elle-même — ici le processus, via `(pid, process_start_time)` — et non une
propriété de la fiche qui décrit la chose.

Le corollaire est celui qui coûte le plus cher à redécouvrir :

> **Allonger le seuil ne répare pas un mauvais discriminant.** Ça déplace la
> frontière entre faux positifs et faux négatifs sans en supprimer aucun. Ici,
> allonger seul aurait laissé traîner plus longtemps les fiches réellement
> mortes *sans* sauver les sessions lentes.

Le seuil n'est légitime qu'**une fois le vrai discriminant en place**, et son
rôle change alors de nature : il ne décide plus du sort du travail vivant, mais
seulement de la vitesse de nettoyage des orphelines. C'est ce changement de rôle
qui rend un chiffre plus généreux défendable, pas le chiffre lui-même.

## Le piège de conception, mesuré

L'erreur naturelle est d'écrire la garde sur la **présence** de l'enfant de
rappel plutôt que sur sa **vivacité mesurée**. Elle est fatale :

```sql
SELECT count(*) FROM tasks WHERE result='phantom_aged_out';            -- 181
SELECT count(DISTINCT p.id) FROM tasks p
  JOIN tasks c ON c.parent_task_id = p.id AND c.process_id IS NOT NULL
  WHERE p.result='phantom_aged_out';                                   -- 177
```

**177 des 181 fauches historiques avaient un enfant porteur de PID.** Une garde
écrite sur la présence aurait désarmé le faucheur à 98 %. Le statut de l'enfant
ne sauve pas non plus : 1146 des 1147 enfants porteurs de PID sont `delivered`,
et rien dans le code ne prouve que `delivered` implique un processus mort.

**Le test qui attrape cette erreur est le cas symétrique** : une fiche *avec* un
enfant dont le processus est mort doit être fauchée quand même. Sans lui, la
version fausse passe.

## Direction d'échec : « fail-open » ne veut rien dire tant qu'on n'a pas nommé l'action

Le mot est ambigu et l'ambiguïté est dangereuse. Une garde posée **avant une
écriture destructrice** a deux dispositions possibles, et « ouvert » désigne la
mauvaise dans la moitié des lectures :

| Incertitude | Disposition retenue | Pourquoi |
|---|---|---|
| Aucun enfant / tous morts | **faucher** | c'est la raison d'être du faucheur (comportement d'avant) |
| Enfant sans `process_start_time` exploitable | **faucher, et le compter** | le couple qui identifie une *instance* est incomplet, un PID recyclé lirait « vivant » |
| **La lecture elle-même a échoué** | **passer la ligne, ne rien écrire** | un échec de mesure n'est pas une preuve de mort |

Le troisième cas est celui qu'on écrit mal par réflexe. Une lecture qui échoue
ne dit rien ; la traiter comme un « non » revient à laisser un signal illisible
autoriser la transition qu'on cherchait justement à empêcher. Différer coûte une
passe ; deviner coûte l'incident. Une garde à **trois valeurs**
(`Live` / `NoneLive` / `Unknown`) rend cette distinction impossible à perdre au
prochain refactor — une `Option` l'écrase.

## Le piège SQL qui transforme une ligne en panne globale

`json_extract` de SQLite **lève une erreur dure** sur une colonne non-JSON, il
ne rend pas NULL :

```
$ sqlite3 :memory: "SELECT json_extract('pas du json','\$.a');"
Error in 2nd command line argument: malformed JSON    (rc=1)

$ sqlite3 :memory: "SELECT CASE WHEN json_valid('pas du json')
                    THEN json_extract('pas du json','\$.a') END;"
                                                       (rc=0, NULL)
```

Sans la garde `json_valid`, **une seule ligne sœur malformée fait échouer la
recherche pour le parent entier** — et le parent perd alors la réponse que son
*autre* enfant, vivant, aurait donnée. La dégradation par ligne promise dans le
code Rust n'arrive jamais : l'erreur se produit en SQL, avant le `match`.

Règle : quand une requête agrège plusieurs lignes et qu'une seule mal formée
peut faire tomber l'ensemble, la tolérance doit être **dans la requête**, pas en
aval.

## Ce qu'une mesure a corrigé dans le raisonnement (et pas l'inverse)

En écrivant le test qui devait épingler le nouveau seuil, il est apparu qu'une
fiche **sans aucun enfant** ne l'atteint jamais : le faucheur voisin de parents
sans enfant la prend à **1800 s** avec `stuck_in_progress_no_callback_child`.

Le seuil relevé ne gouverne donc que les fiches **qui ont un enfant**. Deux
conséquences qu'aucun raisonnement n'avait produites :

1. Le chiffre a une portée plus étroite que ce que son argumentaire annonçait.
2. Une fiche **en attente de créneau** — dont l'unique enfant est une enveloppe
   différée sans `process_id` — n'est protégée par *aucune* garde de vivacité,
   seulement par le seuil. La garde ne peut pas l'interroger.

**Généralisation :** avant d'ajuster le seuil d'un faucheur, énumérer *tous* les
mécanismes qui peuvent terminaliser la même forme de ligne. Un seuil n'a
d'effet que sur la population qu'aucun faucheur plus rapide ne prend d'abord.
Le test qui a révélé ça n'a rien coûté ; c'est l'assertion sur `result` — pas
seulement sur `status` — qui a nommé le coupable :

```rust
assert_eq!(o.result.as_deref(), Some("phantom_aged_out"),
    "control must be reaped by THIS sweeper, not by a sibling reaper");
```

Un contrôle qui vérifie `status == "failed"` sans vérifier **par quoi** peut
être vert alors que le mécanisme testé n'a jamais tourné.

## La transition retenue doit laisser une trace au même endroit que la transition écrite

Le chemin destructeur écrivait une ligne `audit_events`. Le chemin qui *retient*
la transition n'écrivait qu'un log. Résultat : un désarmement systématique de la
garde serait invisible sur la surface SQL que les opérateurs interrogent — le
compte de fauches passerait simplement à zéro, ce qui est indiscernable d'un
régime sain.

Règle : **les deux branches d'une décision se lisent sur la même surface.** Ici,
un `tool_name` distinct (`phantom_sweep_spared`) conserve la sémantique de
comptage de `phantom_aged_out` (« lignes réellement transitées ») tout en
rendant l'épargne interrogeable — et permet au test d'affirmer l'épargne
*positivement* au lieu d'affirmer seulement l'absence d'une fauche.

Corollaire de télémétrie : le chemin le plus susceptible de **ramener
silencieusement le faucheur à son comportement d'avant** doit être compté, pas
seulement journalisé — et pas en `debug!` quand le service tourne en `info`.

## Ce que les injections ont prouvé

Chaque garde a été vérifiée en la retirant :

| Injection | Test qui rougit |
|---|---|
| `dispatch_liveness` rend toujours `NoneLive` | les 3 tests d'épargne |
| `continue` → `break` dans la boucle enfants | l'épargne multi-enfants |
| retour anticipé après le premier enfant non vivant | l'épargne multi-enfants |
| constante ramenée à 3600 | le cas entre ancien et nouveau seuil |
| garde `json_valid` retirée | le frère au JSON malformé |

**Le test multi-enfants n'attrapait rien tant que l'ordre n'était pas
déterministe.** `ORDER BY id` sur des UUID rendait l'enfant vivant premier une
fois sur quatre ; le test passait toujours, mais n'aurait piégé un
court-circuit que par chance. Forcer les identifiants est ce qui rend
l'injection significative.

## Résidus nommés, non traités ici

- `(pid, start_time)` n'identifie une instance **que dans le boot qui l'a
  enregistrée** (`/proc/<pid>/stat` champ 22 = tics depuis le démarrage). Borné,
  non permanent : le `timeout_at` de l'enfant mène à `kill_orphan_processes`,
  qui efface le `process_id`.
- La fenêtre d'attente de créneau (ci-dessus) demande un second prédicat
  d'épargne sur l'enveloppe différée — changement de conception, pas de réglage.
- Le faucheur voisin `stuck_pending_no_deferred_wrapper` discrimine sur le
  **statut** de l'enfant, c'est-à-dire précisément le critère que la mesure des
  1146 `delivered` invalide ici.
