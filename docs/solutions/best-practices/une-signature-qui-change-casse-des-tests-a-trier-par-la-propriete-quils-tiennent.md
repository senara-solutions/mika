---
title: "Une signature qui change casse des tests — triez-les par la propriété qu'ils tiennent, pas par la fonction qu'ils appellent"
date: 2026-09-28
last_updated: 2026-09-28
category: best-practices
module: mika-agent/skills/builtin_handlers
problem_type: best_practice
component: dev-loop
severity: medium
applies_when:
  - "Un correctif change le type de retour ou l'arité d'une fonction que des tests appellent directement"
  - "Un plan nomme les tests à supprimer ou migrer sans avoir recensé le symbole par grep"
  - "Un test porte le préfixe d'un autre ticket (ex. mika2118_…) dans un correctif qui ne le concerne pas"
  - "Une revue ou un implémenteur propose de supprimer « les tests d'une signature morte »"
  - "Une valeur se scinde en canaux (neutre / diagnostic, content / audit) et des tests assertaient l'ancien texte"
related_components:
  - testing_framework
tags: [testing, test-triage, signature-change, doctrine, plan-premises, mika-1964, mika-2407]
---

# Une signature qui change casse des tests — triez-les par la propriété qu'ils tiennent

## Contexte

mika#1964, livré par la PR #2555. Le handler `web_search` servait au LLM, via
`ToolOutput::error`, la chaîne rendue par `map_substrate_error(status, label) -> String`.
Deux branches nommaient `MIKA_SEARCH_UPSTREAM`, `MIKA_BRAVE_API_KEY` et « Ask the
operator to… », et ces noms atteignaient les tenants famille par le skill `web-search`.
Le correctif remplace la fonction par
`substrate_error_message(status, label) -> (String, String)`
(`crates/mika-agent/src/skills/builtin_handlers.rs:401`) : le premier membre est un
repli neutre pour `content`, le second le diagnostic opérateur, routé selon le tier
par `dispatch_substrate_unavailable` (appel à `builtin_handlers.rs:319`).

Changer la signature fait échouer la compilation de tous les tests qui appellent la
fonction nue. **Ces tests ne forment pas une population homogène**, et le compilateur
ne dit pas ce que chacun protégeait.

Le plan (`docs/plans/2026-09-22-001-chore-1964-agent-core-sweep-sibling-builtin-plan.md`)
n'en nommait que deux à sa rev 5 : le faux-vert et la branche wiremock 502. La
re-mesure de rev 6 (§10 « §3.5 nommait UN test là où la population en compte CINQ »)
en trouve **cinq**, dont **deux** qui cessent de compiler. Selon le plan (§3.5),
`map_substrate_error` comptait onze occurrences dans le fichier, dont quatre appels de
test — décompte qu'on ne peut plus re-mesurer, le symbole n'existant plus.

Le piège était le cinquième : `mika2118_substrate_404_names_the_selector_not_a_key`
(`builtin_handlers.rs:7753`). Ce n'est pas un test de `web_search`. Il garde la
doctrine de mika#2407 : sans `MIKA_SEARCH_UPSTREAM`, l'endpoint répond 404 quelle
que soit la clé, donc le message doit nommer le **sélecteur**, jamais une clé
absente — nommer une clé envoie l'opérateur au seul endroit qui ne peut pas l'aider.
Il appelait la fonction nue par commodité. Un implémenteur pressé l'aurait supprimé
comme « test d'une signature morte », et **le correctif aurait effacé la doctrine
qu'il prétend servir**.

C'est la classe M4 du même plan (§1), reproduite une génération plus tard : un garde
perdu quand un chemin bouge. Le cas d'origine, `web_search_family_tier_http_401_no_leak`,
construisait à la main l'objet qu'il prétendait vérifier et nommait un garde compagnon
qui n'a jamais existé ; il est resté vert à travers mika#1971, qui avait rouvert la
fuite. Le code mergé raconte ce cas-là (doc-comment de
`mika1964_web_search_family_tier_no_leak_on_substrate_failure`, en-tête de
`scripts/test-check-substrate-leak.sh`). Il ne raconte pas comment le cinquième test a
failli repartir de la même façon, ni la procédure qui l'a rattrapé : c'est l'objet de
ce doc.

## Recommandation

Quand un correctif change la signature d'une fonction que des tests appellent :

1. **Recenser avant d'écrire la conduite.** Grep du symbole sur tout l'arbre ; compter
   à part les appels de production et les appels de test. La liste du plan n'est pas
   la population, le grep l'est.
2. **Ajouter les tests qui assertent le texte sans appeler le symbole.** Ils compilent
   toujours et expriment un contrat périmé : ils ne se signalent pas d'eux-mêmes.
3. **Ne pas trier par le symbole appelé.** « Ils appellent tous X » ne dit pas lequel
   garder. Pour chaque test, noter la **propriété** tenue (taxonomie, contrat servi au
   LLM, doctrine opérateur, non-fuite sur un tier) et si un autre test de l'arbre la
   tient aussi.
4. **Une conduite par propriété**, parmi quatre :
   - **supprimer** — le test ne pilote pas le code réel (faux-vert) ou décrit un chemin
     disparu ; le remplacer par un test qui pilote le vrai handler ;
   - **migrer l'assertion sur le bon membre** — la propriété tient, mais elle vit
     désormais dans un membre précis du nouveau type ; asserter sur l'autre membre
     reviendrait à asserter la fuite qu'on vient de fermer ;
   - **inverser ou reformer** — le contrat servi a changé ; « inverser » s'applique
     **par tier** (voir l'écart ci-dessous) ;
   - **préserver l'intention en migrant** — garde de doctrine qui appelait la fonction
     par commodité : le migrer, et écrire dans son doc-comment que *le sujet a changé
     de canal sans changer de nature*.
5. **Un garde de doctrine touché devient une condition nommée du DoD**, pas une ligne
   de tableau. Le plan l'a fait (§7 : « `mika2118_substrate_404_names_the_selector_not_a_key`
   est TOUJOURS LÀ »).

**Signal d'alerte :** un test au préfixe d'un autre ticket (`mika2118_…` dans un
correctif `mika1964`) est presque toujours un garde de doctrine, pas un test local.

## Pourquoi c'est important

Face à un test qui ne compile plus, le réflexe est de le réparer au plus court ou de le
supprimer. Pour un test de la fonction elle-même, c'est sans conséquence. Pour un garde
de doctrine, la suppression efface une règle du dépôt que rien d'autre ne tient — et
**aucune assertion ne rougit** : la classe M4 est silencieuse par construction.

Le coût de la mesure est faible au regard de ce qu'elle rattrape. En rev 6, le
recensement a trouvé trois tests hors du plan : deux auraient été découverts au
`cargo test` seulement, et le troisième était la branche 404 que rev 5 ne traitait
pas — le cas de panne réellement mesuré le 2026-09-18 sur six tenants (plan §1 M3).

## Quand l'appliquer

- Changement de type de retour ou d'arité d'une fonction appelée directement par des
  tests, surtout quand la valeur se scinde en canaux.
- Renommage ou déplacement de chemin qui rend « morts » des tests d'un autre ticket.
- Toute re-mesure d'un plan groomé : recompter la population de tests touchée, pas
  seulement les positions.

## Exemples

### Procédure de recensement

```bash
F=crates/mika-agent/src/skills/builtin_handlers.rs
SYM=map_substrate_error

# 1. Population brute, tout l'arbre
grep -rn "$SYM" crates/ scripts/ docs/

# 2. Production vs tests (couper sur le MODULE de test, pas le premier #[cfg(test)])
T=$(grep -n '^mod tests {' "$F" | head -1 | cut -d: -f1)
awk -v t="$T" 'NR<t'  "$F" | grep -c "$SYM("    # appels prod
awk -v t="$T" 'NR>=t' "$F" | grep -n "$SYM("    # appels test

# 3. Remonter chaque appel de test à sa fonction englobante
grep -n "fn \|$SYM(" "$F" | grep -B1 "$SYM(" | grep 'fn '

# 4. Les tests qui assertent le TEXTE sans appeler le symbole
grep -n 'MIKA_SEARCH_UPSTREAM on mika-gateway\|rotate MIKA_BRAVE_API_KEY' "$F"
```

Colonnes du tableau de tri : **test** | **casse à la compilation ?** | **propriété
tenue** | **autre garde de cette propriété ?** | **conduite** | **remplaçant ou
doc-comment exigé**.

### La table de mika#1964 (plan rev 6 §3.5 → code mergé, PR #2555)

| test | propriété tenue | conduite | état dans le code |
|---|---|---|---|
| `web_search_family_tier_http_401_no_leak` | aucune (faux-vert M4) | supprimer | supprimé ; remplacé par `mika1964_web_search_family_tier_no_leak_on_substrate_failure` (`:5553`), qui pilote le vrai `web_search` sur 404 et 502, tier `Family`, et asserte l'absence des `FORBIDDEN_FAMILY_TIER_TOKENS` dans `content` plus la ligne `audit_events` |
| `test_map_substrate_error_taxonomy` | la taxonomie produit un message actionnable | migrer sur le membre diagnostic | nom conservé (`:5967`) ; assertions sur `diagnostic_*`, repli `== SEARCH_UNAVAILABLE_FALLBACK` |
| `test_web_search_maps_substrate_404_search_upstream_not_configured` | contrat servi au LLM, 404 | inverser (plan) → **reformer** (code) | `:6190` — tier opérateur : le diagnostic reste dans `content`, derrière le repli neutre |
| `test_web_search_maps_substrate_502_unauthorized` | contrat servi au LLM, 502 | idem | `:6238`, même forme |
| `mika2118_substrate_404_names_the_selector_not_a_key` | **doctrine mika#2407** | préserver l'intention | conservé (`:7753`) ; lit le membre diagnostic, asserte le sélecteur et l'absence de « missing key » ; son doc-comment dit que mika#1964 a changé son *sujet*, pas sa *nature* |

**L'écart entre plan et code est lui-même une leçon.** Le plan prévoyait d'inverser les
deux tests wiremock ; le code les reforme, parce que leur harnais est de tier opérateur,
où le diagnostic est légitimement replié dans `content`. La preuve de non-fuite vit dans
un test séparé, de tier famille. Un test de tier opérateur ne peut pas porter seul la
preuve de non-fuite famille ; le contrôle réciproque est
`mika1964_web_search_default_tier_keeps_the_operator_detail_readable` (`:5629`).

Le code ajoute aussi `mika1964_no_substrate_branch_leaks_through_its_neutral_fallback`
(`:6022`), qui asserte la propriété sur toutes les branches plutôt que le libellé de
chacune : une branche future sans test propre ne peut pas fuir.

## Voir aussi

- `docs/solutions/best-practices/2103-a-guard-that-knows-one-spelling-of-a-defect-protects-nothing-else.md` — même axiome (juger un garde sur la propriété), appliqué ici au tri des tests cassés.
- `docs/solutions/best-practices/trois-facons-dont-une-verification-passe-au-vert-sans-rien-mesurer-2026-09-22.md` — « un scan porte sur une population ».
- `docs/solutions/best-practices/a-guard-anchored-on-the-shape-of-its-subject-loses-sight-of-it-2026-08-30.md` — un garde qui devient aveugle après un refactor ; ici, il est supprimé parce qu'il a l'air mort.
- `docs/solutions/best-practices/secretstring-expose-at-boundary-pattern.md` — la migration pilotée par le compilateur trouve les sites, pas leur intention.
