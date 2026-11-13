---
module: db
tags: [sqlite, vacuum, auto_vacuum, corruption, migration]
problem_type: data_corruption
date: 2026-10-16
---

# Un `VACUUM` au démarrage a abîmé `mika.db` — et ce n'était pas un compactage

## Le constat

Déploiement du 2026-10-16, moteur `613d74f1`. La migration v55→v56 (mika#2658)
s'est jouée au démarrage de mika-spirit, `VACUUM` compris.

| instant | observation |
|---|---|
| 08:50:20Z | sauvegarde automatique avant migration |
| 08:50:33Z → 08:50:38Z | `v56: vacuum start`, `v56: vacuum complete`, `database migrated to v56` |
| 08:50:45Z | 19 WARN `file is not a database` sur les balayages de démarrage |
| 09:00:00Z | `failed to claim task … database disk image is malformed` |
| 09:01:44Z | nouvelle salve sur tout le tick du moteur |
| 09:05Z | `PRAGMA quick_check` rend `ok` |
| 09:15Z | `quick_check` KO — 4 arbres (`tasks`, `idx_tasks_agent_status`, `sqlite_autoindex_sessions_1`, `idx_sessions_task_id`) |
| 11:00Z | 25 arbres : `tasks`, `sessions`, `messages`, `llm_calls`, `tool_calls`, `audit_events`, `a2a_task_map` et leurs index |

Erreurs relevées : `btreeInitPage() returns error code 11`,
`2nd reference to page`, `Bad ptr map entry`, et une plage
`Page 110606… never used`.

**Le fichier lui-même était abîmé**, pas seulement des connexions. C'est le
`quick_check` de 09:15Z qui tranche : il tournait en lecture seule, depuis une
connexion neuve, dans un processus séparé.

## La lecture qui manquait : ce `VACUUM` convertissait le format

La première hypothèse était « un `VACUUM` concurrent laisse des connexions dans
un état antérieur ». Elle n'explique pas une pointer map incohérente. Deux des
signatures — `Bad ptr map entry`, `2nd reference to page` — nomment la *pointer
map*, une structure qui **n'existe que dans un fichier `auto_vacuum` ≠ 0**.

Or :

- `mika.db` est née en `auto_vacuum=NONE` ;
- `Database::open` demande `PRAGMA auto_vacuum = INCREMENTAL` ;
- sur une base déjà peuplée, cette demande est **inopérante** : elle reste en
  attente jusqu'au prochain `VACUUM`.

Donc ce `VACUUM` n'était pas un compactage. C'était une **conversion de format**
du fichier vivant, qui lui ajoute des pages de pointer map — pendant que le tick
du moteur et une connexion par agent écrivaient dessus. Le fait est épinglé par
`db::migrations::tests::conversion_auto_vacuum` :

```text
base peuplée en auto_vacuum=NONE        → PRAGMA auto_vacuum = 0
PRAGMA auto_vacuum = INCREMENTAL        → PRAGMA auto_vacuum = 0   (en attente)
VACUUM;                                 → PRAGMA auto_vacuum = 2
```

**La cause exacte de la corruption n'est pas établie** — on n'a pas l'instant de
l'entrelacement fautif. Mais on n'a pas besoin de la connaître pour retirer
l'acte, et on sait que l'acte n'était pas celui qu'on croyait.

## Le chemin de démarrage portait deux `VACUUM`, pas un

C'est le point qu'un diagnostic pressé rate, et le ticket d'origine ne nommait
que le premier :

| site | quand | autres connexions ouvertes |
|---|---|---|
| `migrate_v55_to_v56` (`db/migrations.rs`) | pendant le premier `Database::open`, hors transaction, après l'estampille v56 | **aucune** dans ce processus |
| `db.vacuum()` dans `startup_cleanup` (`server/mod.rs`) | tâche `tokio::spawn` fire-and-forget lancée **après** `ready` | **toutes** — une par agent, plus le tick du moteur |

Le second était le plus exposé des deux, et il n'était **pas gardé par une
version de schéma** : il repartait à chaque démarrage dès que la compaction des
`audit_events` avait supprimé au moins une ligne. Il explique mieux que la
migration le « 25 s après le démarrage » du premier `file is not a database`.

Leçon transférable : *un correctif dont le périmètre est un fichier nommé dans
le ticket laisse en place les sites que personne n'a cherchés.* Ici le scan
structurel porte sur `crates/mika-agent/src/**/*.rs`, pas sur
`db/migrations.rs`.

## La récupération, le 2026-10-16

`sqlite3 .recover` sur une **copie** a produit un fichier propre :
`quick_check` et `integrity_check` `ok`, intégrité FTS5 `ok`. Par rapport à la
sauvegarde v55, les seules lignes absentes correspondaient à l'élagage du
démarrage (`llm_calls` 39, `tool_calls` 56). Une table `lost_and_found` portait
4 009 cellules orphelines à 2 champs, venues de la plage jamais utilisée.

Et le contrefactuel qui ferme le dossier : **rejouer la v56 sans `VACUUM` sur la
sauvegarde v55 prend 6 s, et `quick_check` est `ok` après.**

## Le correctif (vol 1 de mika#2735)

1. Les deux `VACUUM` du chemin de démarrage sont retirés.
2. `Database::vacuum` et `AsyncDatabase::vacuum` sont **supprimées**, pas
   laissées sans appelant : une méthode orpheline aurait demandé une exemption
   au détecteur, c'est-à-dire une allowlist déguisée.
3. Une source scan (`db::migrations::tests::aucun_vacuum_au_demarrage`) refuse
   tout `VACUUM` dans la production du crate. Registre livré **vide**, avec
   assertion d'entrée morte et quatre contrôles négatifs.
4. `VACUUM INTO` est exempté **par prédicat**, pas par fichier : ce n'est pas le
   même acte (il écrit un fichier neuf), et le vol 3 en posera un dans `db.rs`,
   donc sur le chemin de démarrage.

Le `PRAGMA auto_vacuum = INCREMENTAL` de `Database::open` est **laissé en
place** : hors périmètre, et inoffensif tant qu'aucun `VACUUM` ne tourne sous
concurrence — ce que la garde assure.

Pas encore livré, et nommé comme tel : la commande opérateur sous exclusivité
explicite (`mika db vacuum`, vol 2) et la sauvegarde de pré-migration par
`VACUUM INTO` au lieu de `std::fs::copy` sur le fichier vivant (vol 3).

## Coût assumé, parce qu'il est permanent

Sans compactage automatique, le fichier garde sa taille après une migration qui
supprime une colonne. Donc `database size exceeds threshold`
(`DB_SIZE_WARN_BYTES`, `server/mod.rs`) **sonne à chaque démarrage** jusqu'au
geste opérateur, au lieu d'être éteint par le `VACUUM`. C'est un signal
permanent sur la taille, pas une panne en cours.

Le geste, moteur arrêté, est le runbook `docs/deployment.md` § 4b *Schema
Migrations and File Rewrites*. Il répond aussi à la question que l'incident
posait : **les processus d'agents peuvent-ils tenir la base pendant une
migration ?** Oui aujourd'hui, parce qu'aucune migration ne réécrit plus le
fichier. Non pour une migration qui le réécrit — et l'ordre de démarrage est
écrit là.

## Ce qui n'est pas livré, et pourquoi on le dit

L'AC1 de la première liste du ticket demandait un contrôle négatif
comportemental : « sans le correctif, au moins une connexion du pool échoue ».
Il n'est **pas** livré. La reproduction dépend d'un entrelacement dont la cause
exacte n'est pas établie : un test vert ne prouverait rien, et un test rouge
serait instable. Ce qui le remplace est fait de deux pièces déterministes — la
garde structurelle, dont le contrôle négatif *est* déterministe, et le fait de
conversion de format ci-dessus.

C'est un arbitrage, pas un oubli : une garde qui refuse le retour de l'acte vaut
mieux qu'un test de concurrence instable qu'on finirait par marquer `#[ignore]`.
