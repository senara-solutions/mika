---
title: Audit des scanners de sources structurels — la prémisse « cfg(test) inline » et ce qu'elle cache - Plan
type: fix
date: 2026-09-19
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
issue: senara-solutions/mika#2398
---

# Audit des scanners de sources structurels — la prémisse « cfg(test) inline » et ce qu'elle cache - Plan

## Goal Capsule

- **Objectif :** qu'une garde structurelle de ce dépôt dise la vérité sur la population qu'elle prétend couvrir. Aujourd'hui plusieurs d'entre elles décident « ceci est de la production » à partir d'une prémisse fausse, et la moitié de l'erreur va dans la direction qui ne se voit pas : la garde reste **verte** en ayant cessé de regarder.
- **Moyens :** (1) l'audit lui-même — le relevé mesuré, site par site, avec la direction de l'erreur ; (2) un lecteur unique de la frontière production/test ; (3) la bascule des gardes dont la cécité est mesurée ; (4) une garde anti-récidive.
- **Autorité :** ce plan > le titre du ticket mika#2398 > le commentaire opérateur du 18/09. Le corps du ticket est **vide** — le titre porte tout l'énoncé, et l'anatomie ci-dessous est établie par mesure dans le dépôt, pas par lecture du ticket.
- **Conditions d'arrêt :** s'arrêter et rapporter si l'audit U1 montre que la règle de frontière retenue en KTD2 produit un faux positif sur un fichier existant que la règle prétend classer — la règle serait alors fausse, et changer 20 gardes en s'appuyant dessus déplacerait le défaut au lieu de le fermer.
- **Profil d'exécution :** Rust, `crates/mika-agent` + `crates/mika-common`, tests inline ; aucun changement de schéma, aucune surface runtime, aucun comportement de production modifié.
- **Livraison :** le pipeline `/mika` sur la branche `feat/2398/audit-des-autres-scanners-de-sources` ouvre la PR qui ferme mika#2398.

---

## Product Contract

### Summary

Ce dépôt s'appuie sur une vingtaine de **gardes structurelles** : des tests qui balaient l'arbre source et refusent un motif (un second écrivain, un lecteur hors module, une constante retirée). Elles existent parce qu'un test comportemental ne peut pas voir la classe de défaut qu'elles visent — celle qui ne rend aucune décision fausse et rend l'attribution impossible.

Toutes doivent séparer la production du test, sinon leur propre `mod tests` — qui nomme le motif par construction — devient leur premier offenseur. Elles le font de **cinq façons différentes**, ou pas du tout, et la règle qu'elles appliquent est fausse sur trois formes de code que ce dépôt contient déjà. La conséquence n'est pas symétrique : une garde qui déborde casse le CI et se fait remarquer dans l'heure ; une garde qui a cessé de regarder reste verte, et c'est exactement ce qu'elle promettait de ne jamais faire.

### Problem Frame

Mesuré le 2026-09-19 dans le worktree, sur `crates/*/src/**/*.rs`.

**Cinq implémentations de la même amputation, quatre sémantiques distinctes :**

| Site | Forme |
|---|---|
| `auto_pull.rs:6880` | `split_once("mod tests {")` — et **uniquement pour son propre fichier** |
| `auto_pull.rs:4458` | `split("#[cfg(test)]")`, garde `[0]` |
| `db.rs:14855` | `src.find("#[cfg(test)]")` |
| `agent_loop/mod.rs:14279` | `src.find("#[cfg(test)]")` |
| `agent_loop/mod.rs:8739` | `split_once("\n#[cfg(test)]\nmod tests {")` |
| `tests/eval/test_dispatch_fired_at_stamped.rs:223` | `body.find("#[cfg(test)]")` |
| `tests/ac8_grep_discipline.rs:41` | ligne-à-ligne, `contains("#[cfg(test)]") \|\| contains("#[cfg(any(test,")` |

À quoi s'ajoutent une quinzaine de gardes qui **n'amputent rien du tout** et n'excluent, au mieux, que leur propre fichier en entier : `ready_label.rs:1030` (`no_ready_label_write_outside_this_module`), `auto_pull_stop.rs:339`, `planning/policy.rs:143` (`no_bare_agent_timeout_constant_remains`), `grooming_marker.rs:677`, `image_disposition.rs:579`, `server/deadline_verdict.rs:1228`, `server/mod.rs:1947`, `tools/pr_merge_with_gate.rs:2392`, `task_engine/dispatcher.rs:3966` et `:4017`, `mika-common/src/llm/mod.rs:1467`, `mika-common/src/llm/retry_gate.rs:518`, `mika-common/src/permission_authority.rs:372`, `tests/only_skills_arch_pass_2363.rs:193`, `tests/eval/test_recurring_trigger_wiring_2337.rs:190`.

**La prémisse — « les tests sont dans un `mod tests` inline, précédé d'un `#[cfg(test)]`, et il n'y en a qu'un » — est fausse de trois façons, toutes présentes dans le dépôt :**

**(1) Un helper `#[cfg(test)] fn` au milieu du fichier → cécité massive.** `auto_pull.rs:1922` porte `#[cfg(test)] fn select_feeder_candidates(...)`, un helper de test à 1 989 lignes avant le vrai `mod tests` (ligne 3912). Toute garde en `find("#[cfg(test)]")` coupe `auto_pull.rs` à la ligne 1922 : **1 989 lignes de production réelles sont invisibles** pour elle. C'est la direction dangereuse — la garde ne rougit pas, elle ne voit plus. Vingt fichiers portent plus d'un `#[cfg(test)]` ; `agent_loop/mod.rs` en porte 8, `auto_pull.rs` 5, `db.rs` 3.

**(2) Une mention du marqueur **en prose** déplace la frontière.** `agent_loop/mod.rs:8621` est un commentaire de doc contenant `` `#[cfg(test)] mod tests` `` ; `db.rs:14780` est un commentaire contenant `` `#[cfg(test)]` ``. `find("#[cfg(test)]")` s'arrête sur le **commentaire**, pas sur l'attribut. La perte est ici faible (15 lignes, 1 ligne), mais le mécanisme est le plus vicieux de la famille : **la doc qui explique la garde déplace la garde**, et la frontière cesse d'être une propriété du code pour devenir une propriété de sa rédaction.

**(3) Un fichier intégralement de test ne porte aucun marqueur → faux positifs.** `db/tests/harnais_porte.rs` et `milestone_manager/no_dispatch_test.rs` contiennent **zéro** occurrence de `cfg(test)` : l'attribut est sur la déclaration `mod` chez le parent. Pour les sept formes d'amputation ci-dessus, l'amputation est alors un **no-op** et le fichier entier est lu comme de la production.

**Le lien avec le découpage de `db.rs` (mika#2321), qui est la raison d'être de ce ticket.** `db.rs` pèse 1 108 301 octets et a franchi le plafond de 1 Mo de `scripts/check-secrets.sh` ; le commit `5a7a50fb` lui a posé une exception nommée dans `LARGE_FILE_ALLOWLIST` avec mika#2321 pour tracker. Ce découpage frappera les gardes sur **deux** axes, et les deux sont dans le périmètre de ce plan :

- **Les exclusions par chemin en dur cessent de couvrir le code déplacé.** `auto_pull.rs:6852` porte `let definitions = [src_root.join("db.rs"), src_root.join("async_db.rs")]`. Le jour où `Database` se répartit dans `db/*.rs`, ces exclusions désignent un fichier qui ne porte plus ce qu'elles excluaient, et les nouveaux fichiers entrent dans la population sans que personne l'ait décidé.
- **La forme (3) est exactement ce que le découpage produit.** `db/tests/harnais_porte.rs` en est déjà un, né de mika#2310 précisément parce que `db.rs` frôlait le plafond. Chaque tranche future en ajoutera.

**Conséquence d'ordonnancement :** ce ticket **précède** mika#2321. Découper d'abord ferait rougir des gardes pour une raison sans rapport avec le découpage, au moment le moins propice pour distinguer les deux causes.

### Requirements

- **R1.** Un document d'audit recense **tous** les scanners structurels du dépôt, et pour chacun : ce qu'il détecte, la forme de séparation production/test qu'il applique (ou son absence), les chemins en dur qu'il porte, et la **direction de son erreur** (cécité / faux positif / exact).
- **R2.** Un lecteur unique répond à « quelle part de ce fichier est de la production », correct sur les trois formes (1), (2), (3) mesurées ci-dessus.
- **R3.** Toute garde dont l'audit établit une **cécité** bascule sur ce lecteur. La cécité est prioritaire sur le faux positif : une garde aveugle ment en vert.
- **R4.** Une garde anti-récidive refuse qu'un nouveau scanner réimplémente la séparation production/test au lieu d'appeler le lecteur unique.
- **R5.** Aucune exclusion par chemin en dur ne survit sans être soit remplacée par un critère qui suit le code, soit **nommée** dans l'audit avec la conséquence datée de son maintien.
- **R6.** Chaque nouveau rougissement provoqué par une bascule est **qualifié** avant la fin du travail : offenseur réel (→ ticket nommé, ou correction si elle tient dans le périmètre) ou faux positif (→ la règle de frontière est fausse et se répare, elle ne se contourne pas par une exception).
- **R7.** Aucun comportement de production ne change. Le diff ne touche que du code de test, plus le document d'audit.

### Scope Boundaries

- **Hors périmètre :** le découpage de `db.rs` lui-même (mika#2321). Ce plan rend les gardes robustes **à** ce découpage ; il n'en déplace aucune ligne.
- **Hors périmètre :** les scanners qui ne lisent pas l'arbre source du dépôt — `task_engine/worktree_activity.rs` (production, scanne le worktree d'un pilote), les tests reaper qui fabriquent une arborescence en tmpdir, `bin/verify_bundled_skills.rs` (scanne `skills/bundled/`). Leur prémisse n'est pas celle du titre.
- **Hors périmètre :** les scanners **shell** (`scripts/check-*.sh`, `verify-*.sh`). Ils ne portent pas la prémisse `cfg(test)` — leur classe de fragilité est l'exclusion par chemin, qui est nommée en R5 mais dont le traitement complet est son propre ticket. L'exception `db.rs` de `check-secrets.sh` est relevée par l'audit, pas modifiée ici : elle est un rappel de mika#2321, et la retirer avant le découpage remettrait le plafond en travers du chemin.
- **Hors périmètre :** écrire les gardes manquantes. L'audit peut constater qu'un invariant mériterait une garde ; le dire suffit.
- **Hors périmètre :** un parseur Rust. Voir KTD2 — la règle retenue est une propriété d'indentation, bornée et testable, pas une analyse syntaxique.

---

## Planning Contract

### Key Technical Decisions

- **KTD1 — Le lecteur unique vit dans `mika-common`, derrière `#[cfg(any(test, feature = "test-utils"))]`.** Les gardes concernées sont réparties sur `mika-agent`, `mika-common` et, potentiellement, `mika-gateway` ; seul `mika-common` est en amont des trois. La porte `test-utils` est le précédent déjà en vigueur pour du code de test partagé entre crates (`MockLlmProvider`, `Settings::test_defaults()`), et elle garantit R7 par construction : rien de ceci n'entre dans un binaire de production. **Écarté :** un `dev-dependency` dédié ou un module dupliqué par crate — dupliquer le lecteur unique serait reproduire, une couche plus haut, le défaut que ce ticket ferme.

- **KTD2 — La frontière est une propriété d'**indentation**, et la région est **masquée**, jamais tronquée.** Règle : un attribut `#[cfg(test)]` (ou `#[cfg(any(test, ...))]`) **en colonne 0** ouvre une région qui court jusqu'à la première ligne `}` en colonne 0 incluse, ou la fin du fichier. Cette région est remplacée par des lignes vides — les numéros de ligne restent donc exacts, ce dont chaque garde a besoin pour nommer son offenseur. Les trois formes mesurées tombent d'un coup : (1) le helper `#[cfg(test)] fn` de `auto_pull.rs:1922` est en colonne 0, sa région se ferme sur son propre `}` et **la production reprend après** ; (2) l'exigence « colonne 0 » exclut les mentions en prose, qui sont toutes indentées ou préfixées `///`/`//` ; (3) traité séparément en KTD3.
  - **Pourquoi pas le comptage d'accolades :** les `mod tests` de ce dépôt sont pleins de littéraux JSON, et une accolade dans une chaîne ferait dériver le compteur — dans une direction ou dans l'autre, sans signal. L'indentation est garantie par `rustfmt`, que le CI impose déjà (`cargo fmt --check`), donc la règle s'appuie sur un invariant **déjà vérifié ailleurs** plutôt que sur une hypothèse nouvelle.
  - **Pourquoi masquer et non tronquer :** tronquer suppose que le test est en queue de fichier. C'est faux dès la forme (1), et c'est précisément l'hypothèse qui coûte les 1 989 lignes mesurées.
  - **Coût nommé :** un `#[cfg(test)]` **indenté** (dans un `impl`, un `mod` imbriqué) n'est pas masqué. Ce cas n'est pas traité, il est **détecté** : le lecteur le signale, et U1 relève combien il y en a. En ajouter le traitement sans mesure serait ajouter une règle à une famille dont le défaut est justement d'avoir trop de règles non mesurées.

- **KTD3 — Un fichier intégralement de test est reconnu par convention de chemin, et la convention est **vérifiée**, pas supposée.** Sous `src/`, un fichier situé dans un répertoire `tests/`, ou nommé `tests.rs` / `*_test.rs` / `*_tests.rs`, a une production **vide**. Les trois fichiers mesurés s'y conforment (`db/tests/harnais_porte.rs`, `perimeter/tests.rs`, `milestone_manager/no_dispatch_test.rs`). Une garde tient la convention honnête dans les deux sens : tout fichier matchant doit être déclaré derrière un `#[cfg(test)]` chez son parent, et un fichier ainsi déclaré doit matcher la convention. **Écarté :** une liste déclarée de fichiers — elle se périme en silence à chaque tranche de mika#2321, et se périmer en silence est le défaut d'origine. **Écarté aussi :** déduire le statut en remontant la chaîne `mod` depuis `lib.rs`, qui est un parseur de modules déguisé et dont le coût dépasse ce que le ticket achète.

- **KTD4 — La direction de l'erreur ordonne le travail, et l'audit la mesure avant toute bascule.** Cécité d'abord (R3) : elle ment en vert. Faux positif ensuite : il casse le CI, donc il est déjà su. « Exact » enfin : bascule pour l'uniformité, sans urgence. Cet ordre est ce qui permet à U3 de livrer une valeur réelle même si U1 révèle un périmètre plus large qu'attendu.

- **KTD5 — Un rougissement provoqué par une bascule est un **résultat**, pas une régression.** Une garde rendue voyante peut se mettre à voir un offenseur qui existait depuis toujours dans la zone qu'elle avait cessé de lire — c'est littéralement ce que l'audit cherche. R6 interdit de le faire taire : il se qualifie. S'il est réel et que sa correction ne tient pas dans ce périmètre, il part en ticket nommé et la garde est laissée rouge **le temps du même PR seulement** — une garde rouge mergée est une garde qu'on désarme la semaine suivante.

- **KTD6 — La garde anti-récidive scanne la source, et c'est le seul moyen.** Une sixième réimplémentation de l'amputation ne rendrait **aucune** décision fausse le jour où elle est écrite ; elle divergerait ensuite, comme les cinq actuelles ont divergé. Aucune assertion comportementale ne peut voir ça. Même famille, même justification que `grooming_marker::no_grooming_regex_outside_this_module` et `auto_pull::mika2131_exclusion_skips_never_return_to_an_uncollected_debug`. La garde est elle-même le premier client du lecteur unique, donc elle prouve en s'exécutant que le lecteur fonctionne sur le fichier le plus hostile qui soit : celui qui nomme le motif qu'il interdit.

### High-Level Technical Design

```
mika-common::source_guard   (#[cfg(any(test, feature = "test-utils"))])
  ├─ production_slice(path, content) -> String
  │     ├─ KTD3 : chemin test-only  → chaîne de lignes vides (production vide)
  │     └─ KTD2 : masque chaque région #[cfg(test)] en colonne 0
  │               → numéros de ligne préservés
  └─ scan_src_tree(crate_root, |path, production| …)
        └─ parcours récursif de src/, filtre *.rs, appelle production_slice

                 ▲                      ▲                      ▲
                 │                      │                      │
   gardes « cécité » (U3)   gardes « exact/FP » (U1→suivi)   garde anti-récidive (U4)
```

Les gardes gardent chacune leur motif, leur message et leur disposition. Ce qui est mutualisé est **uniquement** la réponse à « quelle part de ce fichier est de la production », c'est-à-dire exactement la question que sept sites répondent aujourd'hui de quatre façons.

### Assumptions

- `rustfmt` est appliqué à tout le code du dépôt (imposé par le job `cargo fmt --check` du CI), donc un item de niveau module est en colonne 0 et se ferme par un `}` en colonne 0. **La règle de KTD2 repose sur cet invariant et sur rien d'autre.** Si U1 trouve un fichier qui le viole, c'est une condition d'arrêt (Goal Capsule) et non une exception à ajouter.
- Les gardes actuelles sont, pour la plupart, vertes aujourd'hui — y compris celles qui sont aveugles. Une bascule peut donc en faire rougir ; KTD5 dit comment le lire.
- Le découpage de `db.rs` (mika#2321) n'a pas commencé sur la couche `Database` elle-même : `db/` ne contient à ce jour que `kg_schema.rs`, `operational.rs` et `tests/harnais_porte.rs`. Le plan est donc écrit contre l'arbre actuel, et sa valeur est précisément d'être livré avant.

---

## Implementation Units

### U1. L'audit

**Goal :** le relevé mesuré que le titre du ticket demande — et la décision par site, prise avant d'écrire une ligne de lecteur.

**Requirements :** R1, R5

**Dependencies :** aucune

**Files :**
- `docs/solutions/architecture-patterns/2026-09-19-audit-scanners-sources-structurels.md` (nouveau)

**Approach :**
1. Recenser les scanners : tout site sous `crates/*/src/**` et `crates/*/tests/**` combinant `env!("CARGO_MANIFEST_DIR")` et une lecture de l'arbre source. Exclure ceux nommés en Scope Boundaries, en disant pourquoi — un inventaire dont on ne sait pas ce qu'il a écarté ne se relit pas.
2. Pour chacun : nom du test, ticket d'origine, motif détecté, forme de séparation production/test, chemins en dur, **direction de l'erreur**, décision (basculer maintenant / basculer plus tard / laisser, avec le motif).
3. Chiffrer la cécité là où elle est mesurable, comme les 1 989 lignes d'`auto_pull.rs` le sont : un nombre rend la priorité indiscutable là où un adjectif la laisse négociable.
4. Relever les `#[cfg(test)]` **indentés** (l'angle mort assumé de KTD2) : combien, où. Si le compte est nul, l'angle mort est théorique et le dire est la conclusion ; s'il ne l'est pas, c'est un ticket de suivi nommé, pas un élargissement de la règle en cours de route.
5. Relever les exclusions par chemin en dur et, pour chacune, ce que mika#2321 lui fera (R5).

**Test scenarios :** aucun — U1 est un document. Sa vérification est en aval : U3 ne bascule que ce que U1 a classé, et un site absent de l'inventaire ne peut pas être basculé.

**Verification :** chaque garde de l'inventaire est atteignable par son nom de test ; la somme des sites classés couvre les sites recensés à l'étape 1.

### U2. Le lecteur unique

**Goal :** une réponse, testée sur les trois formes mesurées, à « quelle part de ce fichier est de la production ».

**Requirements :** R2, R7

**Dependencies :** U1 (pour l'étape 4 : l'angle mort est connu avant d'être assumé)

**Files :**
- `crates/mika-common/src/source_guard.rs` (nouveau)
- `crates/mika-common/src/lib.rs` (déclaration du module, derrière `#[cfg(any(test, feature = "test-utils"))]`)

**Approach :**
1. `is_test_only_path(path) -> bool` — KTD3, sur le chemin relatif à `src/`.
2. `production_slice(path, content) -> String` — KTD3 puis KTD2, lignes masquées et non retirées.
3. `scan_src_tree(crate_root, f)` — le parcours récursif que quinze gardes réécrivent aujourd'hui à l'identique, avec l'assertion `scanned > 0` qu'elles portent déjà (un chemin cassé doit rougir, pas passer).
4. Documenter au site, en une phrase chacun : pourquoi l'indentation et pas les accolades, pourquoi masquer et pas tronquer, et quel cas n'est pas traité.

**Patterns to follow :** `mika_common::llm::mock` (porte `test-utils`) ; la forme de boucle de `ready_label.rs:1036` et `planning/policy.rs:163`, qui sont déjà identiques à la variable près.

**Test scenarios :**
- `mod tests` en queue → production = tout ce qui précède, lignes suivantes vides.
- Helper `#[cfg(test)] fn` au milieu, suivi de production, suivi de `mod tests` → **la production du milieu est conservée**. C'est le test qui ferme la forme (1) ; l'écrire en premier.
- Commentaire `///` contenant `` `#[cfg(test)]` `` puis production puis le vrai `mod tests` → la production entre les deux est conservée (forme (2)).
- `#[cfg(any(test, feature = "test-utils"))]` en colonne 0 → masqué comme `#[cfg(test)]`.
- Chemin `db/tests/harnais_porte.rs`, contenu sans aucun marqueur → production vide (forme (3)).
- Chemin `perimeter/tests.rs`, `milestone_manager/no_dispatch_test.rs` → production vide.
- Fichier sans aucun test → production = fichier intact.
- Les numéros de ligne d'une ligne de production après une région masquée sont inchangés.
- **Contrôle négatif :** un `#[cfg(test)]` indenté n'est pas masqué — le comportement est asserté tel qu'il est, pour que sa correction future soit un changement visible et non une dérive.

**Verification :** `cargo test -p mika-common` ; le lecteur appliqué aux fichiers réels `auto_pull.rs`, `agent_loop/mod.rs`, `db.rs` rend une frontière égale à celle relevée à la main en U1.

### U3. Bascule des gardes aveugles

**Goal :** les gardes dont la cécité est mesurée voient à nouveau ce qu'elles promettent de voir.

**Requirements :** R3, R5, R6, R7

**Dependencies :** U1, U2

**Files :** les sites que U1 classe « cécité ». Sur l'état mesuré au 19/09, au moins :
- `crates/mika-agent/src/db.rs` (~14855)
- `crates/mika-agent/src/agent_loop/mod.rs` (~14279 et ~8739)
- `crates/mika-agent/src/auto_pull.rs` (~4458 et ~6880, dont les exclusions `db.rs` / `async_db.rs` de ~6852, R5)
- `crates/mika-agent/tests/eval/test_dispatch_fired_at_stamped.rs` (~223)
- `crates/mika-agent/tests/ac8_grep_discipline.rs` (~41)

**Approach :**
1. Une garde à la fois : remplacer l'amputation locale par `production_slice`, exécuter la garde seule, **lire le résultat avant de passer à la suivante**. Grouper les bascules ferait d'un rougissement une énigme à N causes.
2. Chaque rougissement se qualifie selon R6/KTD5. La qualification s'écrit dans le document U1 : c'est le rendu de l'audit, pas une note de passage.
3. Pour les exclusions par chemin en dur d'`auto_pull.rs:6852`, appliquer R5 : soit un critère qui suit le code (le site de définition, pas le fichier qui l'héberge), soit le maintien **daté** avec ce que mika#2321 lui fera.

**Execution note :** écrire d'abord la bascule d'`auto_pull.rs:4458` et exécuter `mika2131_exclusion_skips_never_return_to_an_uncollected_debug` : c'est la garde dont la zone aveugle mesurée est la plus vaste, donc celle qui a le plus de chances de rapporter quelque chose.

**Test scenarios :** les tests existants de chaque garde restent verts **sans modification de leur message ni de leur disposition**. Une garde dont il faut changer l'assertion pour qu'elle repasse au vert n'a pas été basculée : elle a été affaiblie.

**Verification :** `cargo test -p mika-agent -p mika-common` ; chaque rougissement apparu est tracé dans U1 avec sa qualification.

### U4. La garde anti-récidive

**Goal :** qu'une sixième réimplémentation de l'amputation soit refusée au moment où elle est écrite, et pas découverte quand elle aura divergé.

**Requirements :** R4

**Dependencies :** U2, U3 (la garde ne peut pas être verte tant que les cinq formes existent)

**Files :**
- `crates/mika-common/src/source_guard.rs` (la garde vit avec le lecteur qu'elle protège)

**Approach :**
1. Balayer `crates/*/src/**` et `crates/*/tests/**` via `scan_src_tree`, refuser toute ligne de production qui compose une séparation production/test à la main : `find(` / `split(` / `split_once(` / `contains(` appliqué à un littéral contenant `cfg(test)` ou `mod tests`.
2. Le motif est écrit en morceaux recomposés (`concat!`), comme `planning/policy.rs:143` le fait déjà — sans quoi la garde est son premier offenseur. Ce contournement est aujourd'hui une contorsion locale répétée ; ici il est **le seul restant**, ce qui est une amélioration en soi.
3. Disposition en cas de déclenchement, écrite au site : **halt-and-surface, sans liste d'exceptions.** Une garde qui tolère « N sites dont cinq sont nommés » ne dit plus rien le jour où le sixième arrive — c'est la forme exacte du défaut qu'elle ferme.
4. La garde KTD3 de cohérence de la convention (un fichier test-only est déclaré derrière un `#[cfg(test)]` chez son parent, et réciproquement).

**Test scenarios :**
- Le dépôt en l'état post-U3 → zéro offenseur.
- Un fichier de fixture réintroduisant `src.find("#[cfg(test)]")` → détecté, et le message nomme `production_slice` comme remède.
- La garde ne se détecte pas elle-même (contrôle de bonne foi : sans la recomposition `concat!`, elle rougirait — l'asserter fige la raison de la contorsion).
- Un fichier test-only non déclaré derrière `#[cfg(test)]` chez son parent → détecté.

**Verification :** `cargo test -p mika-common` ; la garde rougit sur la fixture de récidive et reste verte sur l'arbre réel.

---

## Verification Contract

- `cargo test -p mika-agent -p mika-common` — les gardes basculées restent vertes avec leurs assertions d'origine ; les tests de U2 et U4 passent.
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- `cargo build --release` — R7 : le lecteur est derrière `#[cfg(any(test, feature = "test-utils"))]` et n'entre dans aucun binaire.
- **Contrôle de non-régression de périmètre :** `git diff --stat` ne montre aucun fichier de production hors des déclarations de module. Si une ligne de production a changé, c'est que la bascule a débordé.
- **Contrôle de l'audit :** chaque site basculé en U3 figure dans le document U1 ; chaque site de U1 classé « cécité » est basculé ou porte un motif écrit de report.

## Definition of Done

- U1–U4 livrées, tests verts, clippy et fmt propres.
- Aucune garde n'a vu son assertion, son message ou sa disposition affaiblis pour repasser au vert.
- Chaque rougissement apparu pendant U3 est qualifié dans le document d'audit : offenseur réel (corrigé, ou ticket nommé) ou défaut de la règle de frontière (réparé en U2).
- Le document d'audit nomme ce qui **n'a pas** été traité et pourquoi : les `#[cfg(test)]` indentés, les scanners shell, les exclusions par chemin maintenues.
- Le corps de PR mène par le POURQUOI — les 1 989 lignes de production invisibles, les cinq formes pour une question, la mention en prose qui déplace la frontière — et porte `Closes #2398`.

## Acceptance criteria

- [ ] Un document d'audit recense tous les scanners structurels du dépôt avec, par site : motif détecté, forme de séparation production/test, chemins en dur, direction de l'erreur, décision.
- [ ] `production_slice` conserve la production située **après** un helper `#[cfg(test)] fn` de niveau module (forme (1), test dédié).
- [ ] `production_slice` ignore une mention de `#[cfg(test)]` en commentaire ou en prose indentée (forme (2), test dédié).
- [ ] `production_slice` rend une production vide pour un fichier reconnu test-only par convention de chemin (forme (3), test dédié sur les trois fichiers réels).
- [ ] Les numéros de ligne des lignes de production sont inchangés après masquage d'une région de test.
- [ ] Chaque garde classée « cécité » en U1 appelle `production_slice` et passe avec son assertion, son message et sa disposition **inchangés**.
- [ ] Une garde anti-récidive refuse toute réimplémentation locale de la séparation production/test, sans liste d'exceptions, et rougit sur une fixture de récidive.
- [ ] La convention « fichier test-only » est vérifiée dans les deux sens par une garde.
- [ ] `cargo build --release` réussit et le lecteur n'entre dans aucun binaire de production ; aucun fichier de production ne change hors déclarations de module.
- [ ] Le document nomme les populations non traitées (`#[cfg(test)]` indentés, scanners shell, exclusions par chemin maintenues) avec, pour chacune, la conséquence de son maintien.

## Sources

- Issue senara-solutions/mika#2398 — titre « Audit des autres scanners de sources structurels (prémisse cfg(test) inline) », **corps vide** ; commentaire opérateur du 18/09 (« Suite du découpage db.rs (#2321) »).
- mika#2321 — découpage de `db.rs` ; `scripts/check-secrets.sh:44` (exception `LARGE_FILE_ALLOWLIST`, posée par le commit `5a7a50fb`).
- mika#2310 — `crates/mika-agent/src/db/tests/harnais_porte.rs`, premier fichier test-only né de la pression du plafond ; son commentaire de tête dit pourquoi.
- Les sept formes d'amputation : `auto_pull.rs:4458` et `:6880`, `db.rs:14855`, `agent_loop/mod.rs:14279` et `:8739`, `tests/eval/test_dispatch_fired_at_stamped.rs:223`, `tests/ac8_grep_discipline.rs:41`.
- Les faux-amers mesurés : `auto_pull.rs:1922` (helper `#[cfg(test)] fn`), `agent_loop/mod.rs:8621` et `db.rs:14780` (mentions en prose).
- Les gardes sans amputation : `ready_label.rs:1030` (mika#2315), `planning/policy.rs:143` (mika#2189), `grooming_marker.rs:677` (mika#2158), `auto_pull.rs:6849` (mika#2361), `auto_pull_stop.rs:339` (mika#2329), `deadline_verdict.rs:1228` (mika#2368), `pr_merge_with_gate.rs:2392` (mika#2238), `dispatcher.rs:3966`/`:4017` (mika#2205), `llm/mod.rs:1467` (mika#2342), `image_disposition.rs:579` (mika#1784).
- Doctrine : `feedback_structural_gate_audit_grep_all_callsites` (« une garde qui couvre trois appelants sur quatre est une garde qui ment »), citée au site de `planning/policy.rs`.
- `mika_common::llm::mock` et `Settings::test_defaults()` — précédent de la porte `test-utils` pour du code de test partagé entre crates.
