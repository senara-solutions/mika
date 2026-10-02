---
title: Un préfixe LIKE ne délimite pas un numéro, et un scan de source ligne à ligne ne voit pas un LIKE minuscule ni coupé
date: 2026-10-03
category: logic-errors
module: mika-agent/db
problem_type: logic_error
component: database
symptoms:
  - "la sonde en vol de `…/issues/216` répond sur une tâche de `…/issues/2161`"
  - "le message (b) du feeder (`auto_feeder_pool_in_flight` depuis mika#2161) nomme la tâche d'un autre ticket comme celle qui coince le bassin"
  - "la porte 2c du ready_label_handler peut refuser un `labeled ready` au motif du pilote d'un autre ticket"
  - "un scan de source « aucun reference_url + LIKE » reste vert sur `reference_url like ?2` et sur une colonne séparée de son LIKE par un retour à la ligne"
root_cause: logic_error
resolution_type: code_fix
severity: medium
tags: [reference-url, sql-like, prefix-match, delimitation, source-scan, guard-fidelity, live-pilot, auto-pull, mika-2638]
---

# Un préfixe LIKE ne délimite pas un numéro, et un scan de source ligne à ligne ne voit pas un LIKE minuscule ni coupé

## Problem

Deux sondes SQL associaient une tâche à un ticket par `reference_url LIKE '<url du ticket>%'`. Un préfixe ne délimite pas un numéro : la sonde de `…/issues/216` matche aussi `…/issues/2160` à `…/issues/2169` et `…/issues/21600`. Le garde de source posé pour empêcher le retour du défaut avait, lui, deux trous que son propre message affirmait fermés.

## Symptoms

- `find_active_self_dev_task_for_issue` (lu par `auto_pull` Phase 0/2) rendait la tâche d'un voisin numérique ; depuis mika#2161 le message (b) « le bassin est coincé en vol » pouvait donc nommer la tâche d'un autre ticket.
- `find_dispatch_children_for_issue_url`, lecteur unique de `live_pilot` (mika#2279), répondait de même : la porte 2c pouvait refuser un `labeled ready` à cause du pilote d'un autre ticket, c'est-à-dire geler un ticket sain.
- Aucun test ne rougissait : un site par préfixe répond juste sur tout ticket sans voisin numérique, donc presque toujours.

## What Didn't Work

- **Énumérer `LIKE '<url>'` + `LIKE '<url>#%'` + `LIKE '<url>/%'`** (deuxième option de l'AC2) : garde deux élargissements gratuits du `LIKE` — `_` est un joker d'un caractère (`…/my_repo/…` matche `…/myXrepo/…`) et `LIKE` est insensible à la casse ASCII alors que `reference_url` est un `TEXT` sans `COLLATE NOCASE`.
- **Une garde « non-chiffre » en `GLOB '<url>[^0-9]*'`** : aucun précédent dans le dépôt, et SQLite nie une classe par `^`, pas par `!` — `[!0-9]` serait lu comme l'ensemble `{!, 0..9}`, un prédicat silencieusement inversé.
- **Une colonne `issue_number`** (troisième option de l'AC2) : n'existe pas sur `tasks` ; migration + backfill hors de proportion pour un P3.
- **Le premier scan de source D4** (`canonical_tokens.rs`), ligne par ligne, cherchait une ligne portant à la fois `reference_url` et le littéral `LIKE`. Trouvé en revue de code et confirmé par un validateur indépendant : il restait vert sur `AND reference_url like ?2` (les mots-clés SQLite ne sont pas sensibles à la casse) et sur `AND reference_url\n  LIKE ?2` (un littéral SQL reformaté sur deux lignes). Son message d'anti-vacuité disait pourtant « un retour à la ligne ne devrait pas suffire » : la normalisation `split_whitespace` portait sur **une** ligne déjà découpée, donc ne pouvait rien recoller.

## Solution

1. **Égalité stricte sur l'ensemble clos des variantes, par un seul site de définition.** `task_state::tasks::issue_url_variants` (`crates/mika-agent/src/task_state/tasks.rs:359`) rend `[url, url?phase=groom]` (en retirant un `?phase=groom` résiduel, idempotent). Les deux sondes passent de `reference_url LIKE ?2` à `reference_url IN (?2, ?3)` (`crates/mika-agent/src/db/tasks.rs:1747`, `:2276`), et les trois sites déjà délimités (`:1232`, `:4278`, `:4357`) passent par le même helper, sortie identique — cinq appelants pour une définition.

   ```rust
   // avant
   let prefix = format!("{}%", issue_url);
   // … AND reference_url LIKE ?2  — params![agent_id, prefix]
   // après
   let [exact, groom] = crate::task_state::tasks::issue_url_variants(issue_url);
   // … AND reference_url IN (?2, ?3)  — params![agent_id, exact, groom]
   ```

2. **Tests au niveau DB** (`crates/mika-agent/src/db/tests/reference_url_delimitee.rs`) : la sonde de #216 ignore #2161 sur les deux sites, contrôle positif sur #216, table `2160` / `21600` / `2161`, cas `_`, et les suffixes non déclarés `/` et `#issuecomment-1` épinglés comme **non** vus.

3. **Deux scans de source, et D4 en deux lectures.** D5 exige que tout corps de fonction de la couche DB comparant `reference_url IN (` appelle `issue_url_variants(`. D4 lit (1) ligne par ligne, et (2) la source **normalisée** — blancs fusionnés, casse repliée — en cherchant la colonne **immédiatement suivie** du mot `LIKE`, avec frontière de mot (sinon `reference_url likely_…` serait un faux positif). Une allowlist à une entrée (`reference_url LIKE '%github.com%'`, prédicat de domaine sans numéro) porte une assertion auto-nettoyante.

## Why This Works

L'index de dédup `idx_tasks_manual_active_ref_url` est déjà une égalité stricte sur `(agent_id, reference_url)` : le système tient donc déjà l'invariant « l'ensemble des variantes est clos et exact ». Une sonde plus permissive que son propre index était l'incohérence ; l'énumération exacte la fait coïncider avec lui. Le rétrécissement que cela implique (une ligne `…/issues/N/`, `…#…` ou en autre casse n'est plus vue) a été **mesuré** avant d'être accepté : sur la base de production le 2026-10-03, 974 lignes `self_dev` à URL d'issue — 896 sans suffixe, 77 en `?phase=groom`, **une** en `?v2` (issue #1204, tâche `completed` depuis 2026-06-06, donc hors du prédicat « actif » des deux sondes et invisible pour elles de toute façon), aucune en majuscules (requêtes en lecture seule faites pendant la revue de cette session, puis recomptées par le validateur d'ancrage). Le résidu `?v2` est la preuve qu'une forme non canonique *peut* être écrite : l'écrivain identifié est l'outil `create_task` (`crates/mika-agent/src/tools/create_task.rs`), qui ne normalise `reference_url` que par `.trim()` — le remède, s'il faut un jour, est au point d'écriture, pas au prédicat de lecture.

Pour D4 : un défaut de cette classe ne rend **aucune** décision fausse le jour où il est écrit, donc seul un scan de source peut le voir. Mais un scan qui lit ligne par ligne et compare en casse exacte porte sur la *mise en forme* du SQL, pas sur la *comparaison* — et SQLite ne distingue ni la casse des mots-clés ni les retours à la ligne. La seconde lecture ramène le prédicat sur ce que SQLite exécute.

## Prevention

- **Ne jamais comparer un identifiant numéroté par préfixe.** Délimiter par égalité stricte sur un ensemble clos, ou par un séparateur explicite (`@`, comme `hold_audit_key` et le faucheur), jamais par `LIKE 'x%'`. La maison avait déjà tranché cette classe trois fois (`degroom_marker_key`, mika#2347 : `#234` vs `#2343`) ; les deux sondes ne l'avaient pas appliquée.
- **Un ensemble de variantes a un seul site de définition**, et un scan qui exige que chaque comparaison passe par lui : deux requêtes libres d'épeler l'ensemble finiront par diverger (mika#2158, mika#2335).
- **Un scan de source sur du SQL lit la forme normalisée et casse repliée**, avec frontière de mot — pas une ligne brute en casse exacte. Et son message d'échec ne doit rien affirmer que son prédicat ne garantit pas.
- **Contrôle négatif par mutation, terme par terme** : pour D4 on a injecté dans la requête du site 1, *en gardant* le `IN` (sinon l'anti-vacuité rougit d'abord et masque le terme testé), une clause `AND reference_url like ?2` puis la même coupée sur deux lignes — les deux rougissent depuis le correctif.
- **Rétrécir un prédicat de lecture se mesure avant de s'accepter** : compter les formes réellement écrites en base (`ltrim(substr(reference_url, instr(reference_url,'/issues/')+8), '0123456789')` groupé) ; une troisième forme se règle au site d'écriture, pas en ré-élargissant la lecture.

## Related Issues

- mika#2638 (ce défaut), PR de rescue senara-solutions/mika#2651 (brouillon à cette date), plan `docs/plans/2026-10-02-002-fix-2638-reference-url-numero-delimite-plan.md`.
- mika#2161 (le message (b) qui a rendu le défaut visible), mika#2279 (`live_pilot`, porte 2c).
- `docs/solutions/architecture-patterns/2026-09-19-audit-scanners-sources-structurels.md` — même famille de scans de source ; la cécité ligne à ligne de D4 est un cas de la classe « silent-blindness » qu'il décrit.
- `docs/solutions/best-practices/un-cfg-test-ditem-aveugle-un-scan-qui-tronque-au-premier-litteral-2026-09-29.md` — un scan neuf vert du premier coup doit prouver qu'il regarde.
- `docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md` — discipline de fixtures et de contrôles négatifs pour un garde qui peut passer à vide.
- `docs/solutions/logic-errors/grooming-provenance-gate-reads-a-parent-row-a-sibling-mechanism-flips.md` — origine de la paire `url` / `url?phase=groom` et premier site en `IN (?2, ?3)`.
- `docs/solutions/architecture-patterns/la-note-dun-refus-ne-peut-pas-valoir-preuve-2026-09-29.md` — même famille : lecteur par sous-chaîne contre lecteur ancré.
