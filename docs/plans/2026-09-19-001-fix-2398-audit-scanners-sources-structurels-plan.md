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
- **Autorité :** ce plan > le titre du ticket mika#2398 > le commentaire opérateur du 18/09. Le corps du ticket est **vide** — le titre porte tout l'énoncé, et l'anatomie ci-dessous est établie **par mesure dans le worktree au 2026-09-19**, pas par lecture du ticket.
- **Conditions d'arrêt :** les deux que la version précédente de ce plan posait ont été **atteintes en pré-vol et levées par correction de la règle**, pas par exception (voir KTD2 § *La règle naïve sur-ampute* et KTD3 § *La convention de nom est fausse*). Il en reste une de ce type, et elle est étroite : s'arrêter et rapporter si U1 trouve un fichier de production dont un item de niveau module ne se ferme pas à sa propre indentation — la règle reposerait alors sur un invariant `rustfmt` que le dépôt ne respecte pas. Les haltes propres aux détecteurs sont nommées une à une en § *Fire-Disposition*, qui est la section décisionnelle à lire avant de faire taire un rougissement.
- **Profil d'exécution :** Rust, `crates/mika-agent` + `crates/mika-common`, tests inline ; aucun changement de schéma, aucune surface runtime, aucun comportement de production modifié.
- **Livraison :** le pipeline `/mika` sur la branche `feat/2398/audit-des-autres-scanners-de-sources` ouvre la PR qui ferme mika#2398.

---

## Product Contract

### Summary

Ce dépôt s'appuie sur une vingtaine de **gardes structurelles** : des tests qui balaient l'arbre source et refusent un motif (un second écrivain, un lecteur hors module, une constante retirée). Elles existent parce qu'un test comportemental ne peut pas voir la classe de défaut qu'elles visent — celle qui ne rend aucune décision fausse et rend l'attribution impossible.

Toutes doivent séparer la production du test, sinon leur propre `mod tests` — qui nomme le motif par construction — devient leur premier offenseur. Elles le font de **cinq façons différentes**, ou pas du tout, et la règle qu'elles appliquent est fausse sur **six** formes de code que ce dépôt contient déjà. La conséquence n'est pas symétrique : une garde qui déborde casse le CI et se fait remarquer dans l'heure ; une garde qui a cessé de regarder reste verte, et c'est exactement ce qu'elle promettait de ne jamais faire.

### Problem Frame

Mesuré le 2026-09-19 dans le worktree, sur les cinq crates (`mika-agent`, `mika-common`, `mika-gateway`, `mika-a2a`, `mika-cli`), **337 fichiers `.rs`**.

**Le chiffre qui ordonne tout le travail.** Une garde qui coupe à `find("#[cfg(test)]")` perd **14 843 lignes de production non vides**, réparties sur **24 fichiers** et **5 crates**. Ce n'est pas un défaut ponctuel d'`auto_pull.rs` : c'est la propriété de la prémisse.

| lignes de production perdues | fichier | coupe à la ligne |
|---:|---|---:|
| 2 999 | `skills/builtin_handlers.rs` | 592 |
| 1 887 | `prompt.rs` | **142** |
| 1 859 | `auto_pull.rs` | 1 922 |
| 1 229 | `mika-cli/src/tui/app.rs` | 364 |
| 1 065 | `milestone_manager/spawn.rs` | 70 |
| 1 000 | `mika-gateway/src/github.rs` | 600 |
| 762 | `mika-common/src/llm/ollama.rs` | 29 |
| 747 | `mika-cli/src/tui/commands/handlers.rs` | 479 |
| 650 | `mika-gateway/src/routes.rs` | 2 185 |
| 594 | `mika-common/src/claude.rs` | 457 |
| … | 14 autres fichiers | |

**Treize implémentations de la même amputation, six sémantiques distinctes.** Le recensement est
mesuré par scan sur les cinq crates au 2026-09-19 (`(find|split|split_once|contains|starts_with|splitn)`
appliqué à un littéral contenant `cfg(test)` ou `mod tests`) :

| Site | Forme | Périmètre scanné | Direction de l'erreur |
|---|---|---|---|
| `db.rs:14855` | `src.find("#[cfg(test)]")` | **tout `mika-agent/src`** | cécité, **toutes formes** |
| `agent_loop/mod.rs:14279` | `src.find("#[cfg(test)]")` | **tout `mika-agent/src`** | cécité, **toutes formes** |
| `tests/eval/test_dispatch_fired_at_stamped.rs:223` | `body.find("#[cfg(test)]")` | 3 fichiers nommés | **faux positif** (prédicat `contains` positif) |
| `auto_pull.rs:4458` | `split("#[cfg(test)]")`, garde `[0]` | son propre fichier (`include_str!`) | cécité, toutes formes |
| `auto_pull.rs:6880` | `split_once("mod tests {")` | son propre fichier | **faux positif** (un helper de niveau module reste en production) |
| `prompt.rs:5871` | `split_once("\nmod tests {")` | son propre fichier | **faux positif**, idem |
| `agent_loop/mod.rs:8739` | `split_once("\n#[cfg(test)]\nmod tests {")` | son propre fichier | cécité sur (3) seulement |
| `tests/eval/test_recurring_trigger_wiring_2337.rs:70` | `find("\n#[cfg(test)]\nmod ")` | arbre source | cécité sur (3) seulement |
| `server/a2a.rs:1976` | `split("\n#[cfg(test)]\n")` | son propre fichier | cécité sur (1), (4), (3) |
| `server/deadline_verdict.rs:1248` | `text.find("\n#[cfg(test)]")` | son propre fichier | cécité sur (1), (4), (3) |
| `mika-gateway/src/telegram.rs:2265` | `source.find("\n#[cfg(test)]")` | son propre fichier | cécité sur (1), (4), (3) |
| `mika-gateway/src/telegram_markdown.rs:800` | `source.find("\n#[cfg(test)]")` | son propre fichier | cécité sur (1), (4), (3) |
| `tests/ac8_grep_discipline.rs:41` | ligne-à-ligne + comptage d'accolades | arbre source | **les deux**, selon la dérive du compteur |

> **Note de correction — le recensement précédent en manquait six, et en classait deux à l'opposé.**
> La version précédente de ce plan annonçait « cinq implémentations » pour sept sites, et rangeait
> `server/deadline_verdict.rs` et `tests/eval/test_recurring_trigger_wiring_2337.rs` parmi les gardes
> « qui n'amputent rien du tout ». Les deux amputent (aux lignes `1248` et `70`, dans le corps du test
> dont le plan citait l'attribut `#[test]`). Six sites étaient absents, dont **deux hors `mika-agent`**
> (`mika-gateway/src/telegram.rs`, `telegram_markdown.rs`). Conséquence directe et chiffrée sur le
> périmètre de travail : la population que la garde anti-récidive U4 refuse est de **13**, pas de 7 —
> voir § *Fire-Disposition*, où c'est ce qui décide de l'étendue de U3.

**Le préfixe `\n` n'est pas cosmétique, et c'est ce qui rend la direction de l'erreur mesurable par
site.** `find("#[cfg(test)]")` nu coupe sur une mention en prose (forme 2) ; `find("\n#[cfg(test)]")`
exige l'attribut en tête de ligne, donc il est **robuste** à un doc-comment (précédé de `///`) et à un
attribut indenté (forme 6), mais reste aveugle à un helper de niveau module (1) et à un item mono-ligne
(4). `split_once("\n#[cfg(test)]\nmod tests {")` est le plus étroit : seule la forme (3) le trompe.
Deux formes se trompent dans la direction **inverse** — `split_once("mod tests {")` laisse un helper
`#[cfg(test)] fn` de niveau module *dans* la production et produit donc un faux positif, pas une
cécité. La bascule ne peut donc pas être uniforme : elle rend certaines gardes voyantes et en
**resserre** d'autres.

La distinction de périmètre décide la priorité : une garde qui ne lit que son propre fichier a une
cécité bornée par ce fichier ; une garde qui balaie `src/` hérite de la **somme** du tableau des 24
fichiers. `db.rs:14855` (mika#2335, disposition *halt-and-surface*) et `agent_loop/mod.rs:14279`
(mika#2305) sont dans ce second cas, et ce sont les deux dont la bascule a été simulée en pré-vol
(§ *Fire-Disposition*).

À quoi s'ajoutent une douzaine de gardes qui **n'amputent rien du tout** et n'excluent, au mieux, que
leur propre fichier en entier : `ready_label.rs:1030`, `auto_pull_stop.rs:339`, `planning/policy.rs:143`,
`grooming_marker.rs:677`, `image_disposition.rs:579`, `server/mod.rs:1947`,
`tools/pr_merge_with_gate.rs:2392`, `task_engine/dispatcher.rs:3966` et `:4017`,
`mika-common/src/llm/mod.rs:1467`, `mika-common/src/llm/retry_gate.rs:518`,
`mika-common/src/permission_authority.rs:372`, `tests/only_skills_arch_pass_2363.rs:193`.

**La prémisse — « les tests sont dans un `mod tests` inline, précédé d'un `#[cfg(test)]` en fin de fichier, et il n'y en a qu'un » — est fausse de six façons, toutes présentes dans le dépôt :**

**(1) Un helper `#[cfg(test)] fn` au milieu du fichier → cécité massive.** `auto_pull.rs:1922` porte `#[cfg(test)] fn select_feeder_candidates(...)`, qui se ferme ligne **1936**, à 1 975 lignes avant le vrai `mod tests` (attribut 3911). Toute garde en `find` coupe le fichier à 1922 : **1 859 lignes de production non vides** (1937→3910) deviennent invisibles. C'est la direction dangereuse — la garde ne rougit pas, elle ne voit plus.

> **Note de correction.** La version précédente de ce plan annonçait « 1 989 lignes de production réelles ». 1 989 est la distance brute entre les deux attributs : elle inclut les 15 lignes du helper lui-même et les lignes vides. Le compte de lignes de production non vides est **1 859**. L'écart est mineur ; qu'il ait existé dans un plan dont le sujet est *« les gardes mentent sur leur population »* ne l'est pas.

**(2) Une mention du marqueur *en prose* déplace la frontière — et c'est la forme la plus coûteuse, pas la plus bénigne.** `prompt.rs:142` est un doc-comment qui contient `` `#[cfg(test)]` `` en décrivant où vit la denylist de mika#2292. C'est la **première** occurrence de la chaîne dans le fichier, donc toute garde en `find` coupe `prompt.rs` à la ligne 142 et perd **1 887 lignes** — la quasi-totalité du fichier qui porte les sections de prompt code-managed. `agent_loop/mod.rs:8621` et `db.rs:14780` sont les cas bénins (15 et 1 ligne) que le plan précédent prenait pour la règle. Le mécanisme est le plus vicieux de la famille : **la doc qui explique une garde déplace une autre garde**, et la frontière cesse d'être une propriété du code pour devenir une propriété de sa rédaction.

**(3) Un fichier intégralement de test ne porte aucun marqueur → faux positifs.** `db/tests/harnais_porte.rs` et `milestone_manager/no_dispatch_test.rs` contiennent **zéro** occurrence de `cfg(test)` : l'attribut est sur la déclaration `mod` chez le parent. Pour les sept formes d'amputation ci-dessus, l'amputation est alors un **no-op** et le fichier entier est lu comme de la production.

**(4) Un item `#[cfg(test)]` *mono-ligne*, sans bloc.** **8 sites**, dont `skills/builtin_handlers.rs:592` (`#[cfg(test)] const PROGRESS_TICKER_INTERVAL = …;`) et six déclarations de module (`#[cfg(test)] mod tests;`, `#[cfg(any(test, feature = "test-utils"))] pub mod mock;`). Ces items n'ont pas d'accolade fermante : toute règle qui cherche « la prochaine `}` » pour borner la région **sur-ampute**. Forme absente du plan précédent, et c'est elle qui invalidait sa règle (voir KTD2).

**(5) `#[cfg(not(test))]` marque de la *production*.** 1 site, `skills/builtin_handlers.rs:590`, immédiatement au-dessus du (4). Une règle qui reconnaîtrait « un attribut `cfg` contenant `test` » inverserait le sens sur cette ligne et masquerait du code de production.

**(6) Un attribut `#[cfg(test)]` *indenté*, à l'intérieur d'un item de production.** **13 sites**, 8 fichiers, 3 crates — le plan précédent pariait sur zéro et en faisait un angle mort « théorique ». `mika-gateway/src/github.rs:598` est le cas net : un `impl ForwardResult` de production contenant trois `#[cfg(test)] fn` indentées. Les autres : `agent_loop/review_anchor.rs:90`, `bundled_skills.rs:1740`, `server/permissions_stream.rs:550`, `server/tasks_stream.rs:213`, `mika-common/src/claude.rs:463`, `mika-common/src/config.rs:2348`, `mika-common/src/github_app.rs:150`/`:163`/`:175`, `mika-cli/src/tui/app.rs:364`.

**Le lien avec le découpage de `db.rs` (mika#2321), qui est la raison d'être de ce ticket.** `db.rs` pèse 1 108 301 octets et a franchi le plafond de 1 Mo de `scripts/check-secrets.sh` ; le commit `5a7a50fb` lui a posé une exception nommée dans `LARGE_FILE_ALLOWLIST` avec mika#2321 pour tracker. Ce découpage frappera les gardes sur **deux** axes, et les deux sont dans le périmètre de ce plan :

- **Les exclusions par chemin en dur cessent de couvrir le code déplacé.** `auto_pull.rs:6852` porte `let definitions = [src_root.join("db.rs"), src_root.join("async_db.rs")]`. Le jour où `Database` se répartit dans `db/*.rs`, ces exclusions désignent un fichier qui ne porte plus ce qu'elles excluaient, et les nouveaux fichiers entrent dans la population sans que personne l'ait décidé.
- **Les formes (3) et (4) sont exactement ce que le découpage produit.** `db/tests/harnais_porte.rs` en est déjà un, né de mika#2310 précisément parce que `db.rs` frôlait le plafond ; chaque tranche future ajoutera un fichier test-only **et** la déclaration `#[cfg(test)] mod …;` mono-ligne qui va avec.

**Conséquence d'ordonnancement :** ce ticket **précède** mika#2321. Découper d'abord ferait rougir des gardes pour une raison sans rapport avec le découpage, au moment le moins propice pour distinguer les deux causes.

### Requirements

- **R1.** Un document d'audit recense **tous** les scanners structurels du dépôt, et pour chacun : ce qu'il détecte, son **périmètre de scan** (fichier propre / arbre), la forme de séparation production/test qu'il applique (ou son absence), les chemins en dur qu'il porte, et la **direction de son erreur** (cécité / faux positif / exact).
- **R2.** Un lecteur unique répond à « quelle part de ce fichier est de la production », correct sur les **six** formes mesurées ci-dessus.
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

- **KTD1 — Le lecteur unique vit dans `mika-common`, derrière `#[cfg(any(test, feature = "test-utils"))]`.** Les gardes concernées sont réparties sur `mika-agent`, `mika-common`, `mika-gateway` et `mika-cli` ; seul `mika-common` est en amont des quatre. La porte `test-utils` est le précédent déjà en vigueur pour du code de test partagé entre crates (`MockLlmProvider`, `Settings::test_defaults()`), et elle garantit R7 par construction : rien de ceci n'entre dans un binaire de production. **Écarté :** un `dev-dependency` dédié ou un module dupliqué par crate — dupliquer le lecteur unique serait reproduire, une couche plus haut, le défaut que ce ticket ferme.

- **KTD2 — La frontière est une propriété d'*indentation*, la région est *masquée* (jamais tronquée), et sa fin dépend de la *forme de l'item*.**

  Règle, en trois clauses :
  1. **Ouverture.** Une ligne dont le contenu entier est un attribut `#[cfg(…)]` dont la condition contient `test` **en positif** ouvre une région. `#[cfg(not(test))]` n'ouvre **rien** (forme 5). L'indentation de l'attribut, notée `I`, est retenue.
  2. **Forme de l'item.** À partir de la première ligne suivante qui n'est pas elle-même un attribut, on cherche le premier `{` ou le premier `;` **au niveau de l'item**. Le premier des deux rencontré décide : `{` → item à bloc ; `;` → item mono-ligne (forme 4).
  3. **Fermeture.** Item à bloc : la région court jusqu'à la première ligne égale à `I` + `}`, incluse. Item mono-ligne : la région est l'attribut et son item, et **rien d'autre**.

  La région est remplacée par des lignes vides — les numéros de ligne restent exacts, ce dont chaque garde a besoin pour nommer son offenseur.

  - **La règle naïve sur-ampute, et c'est une condition d'arrêt atteinte en pré-vol.** La version précédente de ce plan écrivait « jusqu'à la première ligne `}` en colonne 0 ». Mesurée sur l'arbre, cette règle masque **481 lignes de production non vides** sur les 8 items mono-ligne de la forme (4) — dont **356 sur le seul `egress_search/mod.rs:364`** (`#[cfg(test)] mod tests_e4_no_log;`, dont la prochaine `}` en colonne 0 est très loin). Une règle qui masque de la production **réintroduit la cécité que le ticket ferme**, par son propre remède. La clause 2 la supprime.
  - **L'indentation de l'attribut, pas la colonne 0.** La règle « colonne 0 » laissait les 13 attributs indentés de la forme (6) en production : une garde basculée se serait mise à voir `github.rs`'s `fn is_retryable`, `Settings::test_defaults`, `GitHubApp::seed_test_token` — des helpers de test — et à rougir dessus. R6 interdit de faire taire un tel rougissement par une exception ; le généraliser à l'indentation de l'attribut le supprime pour un coût nul, puisque la clause 3 était déjà paramétrée par une indentation.
  - **Pourquoi pas le comptage d'accolades :** les `mod tests` de ce dépôt sont pleins de littéraux JSON, et une accolade dans une chaîne ferait dériver le compteur — dans une direction ou dans l'autre, sans signal. L'indentation est garantie par `rustfmt`, que le CI impose déjà (`cargo fmt --check`), donc la règle s'appuie sur un invariant **déjà vérifié ailleurs** plutôt que sur une hypothèse nouvelle.
  - **Pourquoi masquer et non tronquer :** tronquer suppose que le test est en queue de fichier. C'est faux dès la forme (1), et c'est précisément l'hypothèse qui coûte les 1 859 lignes mesurées.
  - **Piège d'implémentation, mesuré en pré-vol.** La clause 2 doit chercher le `{`/`;` **au-delà de la première ligne de l'item**, pas sur elle seule : `cargo fmt` casse régulièrement les signatures sur plusieurs lignes (`auto_pull.rs:1923` est `fn select_feeder_candidates(` — l'accolade est six lignes plus bas). Un prototype qui n'a testé que la première ligne a classé ce helper comme mono-ligne et a mal borné sa région. C'est le seul piège rencontré ; il est écrit ici pour qu'il ne soit pas rencontré deux fois.

- **KTD3 — Un fichier intégralement de test est reconnu par *sa déclaration chez le parent*, jamais par une convention de nom.**

  Pour chaque déclaration `mod X;` / `pub mod X;` masquée par la clause KTD2 (forme 4), le fichier `X.rs` ou `X/mod.rs` du répertoire du déclarant a une production **vide**. Idem pour un fichier sous un répertoire lui-même déclaré ainsi.

  - **La convention de nom est fausse, et c'est la seconde condition d'arrêt atteinte en pré-vol.** La version précédente prescrivait « un répertoire `tests/`, ou un nom `tests.rs` / `*_test.rs` / `*_tests.rs` », plus une garde vérifiant la convention **dans les deux sens**. Mesure : **trois fichiers test-only ne matchent aucun de ces motifs** — `mika-agent/src/test_utils.rs` (déclaré `#[cfg(any(test, feature = "test-utils"))] pub mod test_utils;` à `lib.rs:46`), `mika-common/src/llm/mock.rs` (`llm/mod.rs:5`) et `mika-gateway/src/voice/examples.rs` (`voice/mod.rs:81`). La garde bidirectionnelle aurait donc rougi sur trois fichiers parfaitement légitimes, dès son premier run.
  - **Ce n'est pas le « parseur de modules déguisé » que le plan précédent écartait.** L'analyse requise est exactement celle que la clause KTD2/2 fait déjà pour la forme (4) : les huit items mono-ligne mesurés **sont** ces déclarations. Résoudre `mod X;` en `X.rs` ou `X/mod.rs` dans le répertoire du déclarant est une jointure de chemins, pas une analyse syntaxique — et le coût marginal par rapport à KTD2 est nul.
  - **Écarté :** une liste déclarée de fichiers — elle se périme en silence à chaque tranche de mika#2321, et se périmer en silence est le défaut d'origine.
  - **Angle mort assumé et nommé :** un fichier test-only atteint par une chaîne de `mod` de profondeur > 1 dont un maillon intermédiaire n'est pas lui-même `#[cfg(test)]`. Le dépôt n'en contient aucun aujourd'hui (les trois cas mesurés sont de profondeur 1) ; U1 le re-mesure et le dit.

- **KTD4 — La direction de l'erreur ordonne le travail, et l'audit la mesure avant toute bascule.** Cécité d'abord (R3) : elle ment en vert. Faux positif ensuite : il casse le CI, donc il est déjà su. « Exact » enfin : bascule pour l'uniformité, sans urgence. Cet ordre est ce qui permet à U3 de livrer une valeur réelle même si U1 révèle un périmètre plus large qu'attendu.

- **KTD5 — Un rougissement provoqué par une bascule est un *résultat*, pas une régression.** Une garde rendue voyante peut se mettre à voir un offenseur qui existait depuis toujours dans la zone qu'elle avait cessé de lire — c'est littéralement ce que l'audit cherche. R6 interdit de le faire taire : il se qualifie. S'il est réel et que sa correction ne tient pas dans ce périmètre, il part en ticket nommé et la garde est laissée rouge **le temps du même PR seulement** — une garde rouge mergée est une garde qu'on désarme la semaine suivante.

- **KTD6 — La garde anti-récidive scanne la source, et c'est le seul moyen.** Une sixième réimplémentation de l'amputation ne rendrait **aucune** décision fausse le jour où elle est écrite ; elle divergerait ensuite, comme les cinq actuelles ont divergé. Aucune assertion comportementale ne peut voir ça. Même famille, même justification que `grooming_marker::no_grooming_regex_outside_this_module` et `auto_pull::mika2131_exclusion_skips_never_return_to_an_uncollected_debug`. La garde est elle-même le premier client du lecteur unique, donc elle prouve en s'exécutant que le lecteur fonctionne sur le fichier le plus hostile qui soit : celui qui nomme le motif qu'il interdit.

### High-Level Technical Design

```
mika-common::source_guard   (#[cfg(any(test, feature = "test-utils"))])
  ├─ production_slice(path, content) -> String
  │     ├─ KTD3 : fichier déclaré test-only chez son parent → production vide
  │     └─ KTD2 : masque chaque région cfg(test)
  │               ouverture = attribut cfg positif, indentation I
  │               forme     = premier `{` ou `;` au niveau de l'item
  │               fermeture = `I}` (bloc) | fin de l'item (mono-ligne)
  │               → lignes remplacées par du vide, numéros préservés
  ├─ test_only_modules(crate_root) -> HashSet<PathBuf>   (KTD3, une passe)
  └─ scan_src_tree(crate_root, |path, production| …)
        └─ parcours récursif de src/, filtre *.rs, appelle production_slice

                 ▲                      ▲                      ▲
                 │                      │                      │
   gardes « cécité » (U3)   gardes « exact/FP » (U1→suivi)   garde anti-récidive (U4)
```

Les gardes gardent chacune leur motif, leur message et leur disposition. Ce qui est mutualisé est **uniquement** la réponse à « quelle part de ce fichier est de la production », c'est-à-dire exactement la question que sept sites répondent aujourd'hui de quatre façons.

### Assumptions

- `rustfmt` est appliqué à tout le code du dépôt (imposé par le job `cargo fmt --check` du CI), donc un item se ferme par une accolade à sa propre indentation. **La règle de KTD2 repose sur cet invariant et sur rien d'autre.** Si U1 trouve un fichier qui le viole, c'est la condition d'arrêt résiduelle (Goal Capsule).
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
1. Recenser les scanners : tout site sous `crates/*/src/**` et `crates/*/tests/**` combinant `env!("CARGO_MANIFEST_DIR")` et une lecture de l'arbre source. Exclure ceux nommés en Scope Boundaries, en disant pourquoi — un inventaire dont on ne sait pas ce qu'il a écarté ne se relit pas. **Recouper avec le scan des réimplémentations** (`(find|split|split_once|contains|starts_with|splitn)` sur un littéral `cfg(test)` / `mod tests`), qui doit rendre les treize sites du tableau : la première rédaction de ce plan en avait manqué six en recensant à la main, et deux crates entières avec.
2. Pour chacun : nom du test, ticket d'origine, motif détecté, **périmètre de scan** (fichier propre / arbre — c'est ce qui décide de l'ampleur), forme de séparation production/test, chemins en dur, **direction de l'erreur**, décision (basculer maintenant / basculer plus tard / laisser, avec le motif).
3. Reporter le tableau de la Problem Frame (24 fichiers, 14 843 lignes) et le **recalculer** à la date de l'audit plutôt que le recopier : un chiffre transcrit d'un plan est un chiffre qui a déjà commencé à vieillir.
4. Relever les six formes à la date de l'audit : mono-lignes (8 mesurés), indentés (13), `cfg(not(test))` (1), test-only sans marqueur (2), test-only hors convention de nom (3), mentions en prose. Pour chacune, dire si la clause KTD2/KTD3 correspondante la couvre.
5. Relever les exclusions par chemin en dur et, pour chacune, ce que mika#2321 lui fera (R5).
6. Nommer la profondeur maximale de chaîne `mod` menant à un fichier test-only (angle mort KTD3). Si elle vaut 1, le dire et clore.

**Test scenarios :** aucun — U1 est un document. Sa vérification est en aval : U3 ne bascule que ce que U1 a classé, et un site absent de l'inventaire ne peut pas être basculé.

**Verification :** chaque garde de l'inventaire est atteignable par son nom de test ; la somme des sites classés couvre les sites recensés à l'étape 1.

### U2. Le lecteur unique

**Goal :** une réponse, testée sur les six formes mesurées, à « quelle part de ce fichier est de la production ».

**Requirements :** R2, R7

**Dependencies :** U1 (étapes 4 et 6 : les angles morts sont connus avant d'être assumés)

**Files :**
- `crates/mika-common/src/source_guard.rs` (nouveau)
- `crates/mika-common/src/lib.rs` (déclaration du module, derrière `#[cfg(any(test, feature = "test-utils"))]`)

**Approach :**
1. `test_only_modules(crate_root) -> HashSet<PathBuf>` — KTD3, une passe sur l'arbre qui collecte les déclarations `mod X;` masquées et résout leurs chemins.
2. `production_slice(path, content, &test_only) -> String` — KTD3 puis KTD2, lignes masquées et non retirées.
3. `scan_src_tree(crate_root, f)` — le parcours récursif que quinze gardes réécrivent aujourd'hui à l'identique, avec l'assertion `scanned > 0` qu'elles portent déjà (un chemin cassé doit rougir, pas passer).
4. Documenter au site, en une phrase chacun : pourquoi l'indentation et pas les accolades, pourquoi la forme de l'item décide la fermeture (avec le chiffre de 481 lignes), pourquoi la déclaration parent et pas le nom (avec les trois fichiers), et quel cas n'est pas traité.

**Patterns to follow :** `mika_common::llm::mock` (porte `test-utils`) ; la forme de boucle de `ready_label.rs:1036` et `planning/policy.rs:163`, qui sont déjà identiques à la variable près.

**Test scenarios :** un par forme mesurée, chacun ancré sur le site réel qui l'a produite.
- **F1** — helper `#[cfg(test)] fn` de niveau module, suivi de production, suivi de `mod tests` → la production du milieu est conservée. *L'écrire en premier.*
- **F1b** — le même, avec une **signature multi-lignes** (la forme d'`auto_pull.rs:1923`) → la région se ferme sur l'accolade de l'item, pas sur le premier `;` rencontré. C'est le piège de KTD2 ; sans ce test il revient.
- **F2** — doc-comment contenant `` `#[cfg(test)]` `` avant toute production (la forme de `prompt.rs:142`) → production intégralement conservée.
- **F3** — chemin déclaré test-only chez le parent, contenu sans aucun marqueur → production vide.
- **F4** — `#[cfg(test)] const X = …;` suivi de production (la forme de `builtin_handlers.rs:592`) → **seules** les deux lignes sont masquées.
- **F4b** — `#[cfg(test)] mod X;` dont la prochaine `}` en colonne 0 est très loin (la forme d'`egress_search/mod.rs:364`) → 356 lignes de production conservées. **Contrôle de non-régression de la règle précédente.**
- **F5** — `#[cfg(not(test))] const X = …;` → conservé comme production.
- **F6** — `#[cfg(test)] fn` indentée dans un `impl` de production (la forme de `github.rs:600`) → la fn est masquée, l'`impl` et ses autres membres sont conservés.
- **F7** — `#[cfg(any(test, feature = "test-utils"))]` en colonne 0 → masqué comme `#[cfg(test)]`.
- **Invariants transverses** — fichier sans aucun test → intact ; numéros de ligne inchangés après masquage.
- **Contrôle de bonne foi sur l'axe dangereux** — appliqué à l'arbre réel, aucune ligne masquée ne commence par `pub fn` / `pub async fn` en dehors des fichiers et helpers que U1 a classés test-only. Ce contrôle a été exécuté en pré-vol : 24 occurrences, **toutes légitimes** (`test_utils.rs`, `Settings::test_defaults`, `GitHubApp::seed_test_token`, `receiver_count`, `for_test`). Le figer évite qu'une future permissivité passe inaperçue.

**Verification :** `cargo test -p mika-common` ; le lecteur appliqué aux fichiers réels `builtin_handlers.rs`, `prompt.rs`, `auto_pull.rs`, `github.rs`, `egress_search/mod.rs` rend une frontière égale à celle relevée en U1.

**Note de portée, mesurée :** les deux crates que U3 touche hors `mika-agent` déclarent déjà
`mika-common = { workspace = true, features = ["test-utils"] }` en `[dev-dependencies]`
(`mika-agent/Cargo.toml:83`, `mika-gateway/Cargo.toml:68`). Aucun `Cargo.toml` n'est donc modifié, ce
qui est la moitié mesurable de R7. `mika-cli` ne l'a pas et n'en a pas besoin : aucun des treize sites
n'y est.

### U3. Bascule des gardes aveugles

**Goal :** les gardes dont la cécité est mesurée voient à nouveau ce qu'elles promettent de voir.

**Requirements :** R3, R5, R6, R7

**Dependencies :** U1, U2

**Files :** **les treize sites du tableau de la Problem Frame**, et non les seuls « cécité ». Ce
périmètre est imposé par la Fire-Disposition de U4, pas par R3 : U4 refuse la *réimplémentation*, pas
la cécité, donc tout site laissé en place la fait rougir et bloque le PR. Le tri cécité / faux positif
de KTD4 continue d'ordonner le **travail** ; il ne réduit plus la liste.

- `crates/mika-agent/src/db.rs` (~14855) — arbre
- `crates/mika-agent/src/agent_loop/mod.rs` (~14279 — arbre ; ~8739 — fichier propre)
- `crates/mika-agent/src/auto_pull.rs` (~4458 et ~6880, dont les exclusions `db.rs` / `async_db.rs` de ~6852, R5)
- `crates/mika-agent/src/prompt.rs` (~5871)
- `crates/mika-agent/src/server/a2a.rs` (~1976)
- `crates/mika-agent/src/server/deadline_verdict.rs` (~1248)
- `crates/mika-agent/tests/eval/test_dispatch_fired_at_stamped.rs` (~223)
- `crates/mika-agent/tests/eval/test_recurring_trigger_wiring_2337.rs` (~70)
- `crates/mika-agent/tests/ac8_grep_discipline.rs` (~41)
- `crates/mika-gateway/src/telegram.rs` (~2265)
- `crates/mika-gateway/src/telegram_markdown.rs` (~800)

**Approach :**
1. Une garde à la fois : remplacer l'amputation locale par `production_slice`, exécuter la garde seule, **lire le résultat avant de passer à la suivante**. Grouper les bascules ferait d'un rougissement une énigme à N causes.
2. Chaque rougissement se qualifie selon R6/KTD5. La qualification s'écrit dans le document U1 : c'est le rendu de l'audit, pas une note de passage.
3. Pour les exclusions par chemin en dur d'`auto_pull.rs:6852`, appliquer R5 : soit un critère qui suit le code (le site de définition, pas le fichier qui l'héberge), soit le maintien **daté** avec ce que mika#2321 lui fera.

**Fire-Disposition :** la classe « gardes basculées » est traitée en § *Fire-Disposition* (U3), avec la
simulation pré-vol des deux gardes à périmètre arbre, le seul rougissement mesuré
(`mika2305` sur `prompt.rs:564-565`), son traitement dans le périmètre, et l'interdiction explicite de
l'exception par fichier.

**Execution note — l'ordre a changé, et le précédent reposait sur un chiffre faux.** Commencer par **`db.rs:14855`** : c'est une garde à périmètre *arbre*, donc sa zone aveugle est la somme du tableau de la Problem Frame — **14 843 lignes**, dont `builtin_handlers.rs` (2 999) et `prompt.rs` (1 887) qu'elle ne voit pas du tout. Le plan précédent prescrivait de commencer par `auto_pull.rs:4458` au motif que sa zone aveugle était « la plus vaste » ; `auto_pull.rs:4458` scanne son **propre fichier** et sa cécité est bornée à 1 859 lignes, soit le troisième rang. La bascule la plus susceptible de rapporter quelque chose est celle qui regarde le plus de code.

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
3. Disposition en cas de déclenchement : **halt-and-surface, sans liste d'exceptions** — formalisée en § *Fire-Disposition* (U4/1), avec sa population comptée de **13 sites** et la conséquence qu'elle impose au périmètre de U3. Une garde qui tolère « N sites dont six sont nommés » ne dit plus rien le jour où le septième arrive — c'est la forme exacte du défaut qu'elle ferme.
4. La garde KTD3 de cohérence : tout fichier résolu comme test-only par `test_only_modules` doit exister, et tout fichier du répertoire d'un module test-only l'est aussi. **Pas de garde sur une convention de nom** — la mesure de KTD3 montre qu'elle produirait trois faux positifs immédiats. Disposition en § *Fire-Disposition* (U4/4).

**Test scenarios :**
- Le dépôt en l'état post-U3 → zéro offenseur.
- Un fichier de fixture réintroduisant `src.find("#[cfg(test)]")` → détecté, et le message nomme `production_slice` comme remède.
- La garde ne se détecte pas elle-même (contrôle de bonne foi : sans la recomposition `concat!`, elle rougirait — l'asserter fige la raison de la contorsion).
- Une déclaration `#[cfg(test)] mod X;` dont le fichier `X.rs` est absent → détectée.

**Verification :** `cargo test -p mika-common` ; la garde rougit sur la fixture de récidive et reste verte sur l'arbre réel.

---

## Fire-Disposition

Requis par le Fire-Disposition Gate (mika#1574), soulevé par mika-arch en première passe (F1). Ce plan
porte **quatre** livrables de classe détecteur, et non deux : U4/1 (la garde anti-récidive), U4/4 (la
garde de cohérence KTD3), U2 (le contrôle de bonne foi sur l'axe dangereux) et — la classe que F1
nomme en second — **les gardes existantes que U3 bascule**, qui sont des détecteurs dont ce plan change
la population d'entrée sans toucher leur prédicat.

Tous sont des `#[test]` ordinaires, donc **bloquants en CI par le job `cargo test` existant** : aucun ne
peut lander vert mais inerte.

Chaque disposition ci-dessous est écrite sur une population **comptée dans l'arbre au 2026-09-19**, et
la mesure a contredit le plan sur deux points, reportés en Problem Frame et en U3. C'est la précaution
qui donne son sens au gate : sur un plan dont le sujet est *« les gardes mentent sur leur population »*,
annoncer des exceptions sans avoir compté ce qu'on exempte serait reproduire le défaut dans le remède.

### U4/1 — la garde anti-récidive

**Population comptée : 13 sites** (le tableau de la Problem Frame), sur cinq fichiers de plus et deux
crates de plus que le recensement précédent. Ils sont **tous** des violations préexistantes : la garde
refuse la réimplémentation, et les treize *sont* la réimplémentation.

**Disposition : (c) halt-and-surface, avec une liste d'exceptions vide et interdite.** La conséquence
est un élargissement de U3 aux treize sites, et c'est le prix assumé de cette option — U4 dépend de U3
et ne peut pas être verte tant qu'un site subsiste.

Le détail d'implémentation du halt : si un des treize résiste à la bascule, ou si un quatorzième
apparaît entre cette rédaction et le land, le poseur **s'arrête et remonte à l'opérateur** plutôt que
d'ajouter une entrée d'exemption. La raison est mesurable et non de principe : six exceptions sur treize
laisseraient une garde qui tolère la moitié de ce qu'elle interdit, et le jour où le quatorzième arrive
elle ne dirait plus rien — c'est la forme exacte du défaut qu'elle ferme (l'argument est déjà écrit en
U4/Approach 3 ; la mesure de 13 est ce qui le rend chiffré). **Écarté : (a) exception nommée par site**
pour cette raison. **Écarté : (b) land disabled** — un `#[ignore]` sur la garde qui protège le lecteur
unique laisserait U2 sans gardien pendant que U3 crée douze nouveaux appelants, soit le moment le moins
propice.

### U4/4 — la garde de cohérence KTD3

**Population : non comptée à la rédaction, et c'est U1/6 qui la compte** (tout fichier résolu comme
test-only doit exister ; tout fichier du répertoire d'un module test-only l'est aussi). Le pré-vol a
établi que les trois fichiers test-only hors convention de nom sont de profondeur 1 et se résolvent,
mais il n'a pas balayé l'arbre pour une déclaration `mod X;` orpheline.

**Disposition : (c) halt-and-surface.** Une déclaration `#[cfg(test)] mod X;` dont le fichier est absent
est un état que `cargo build` refuse déjà ; si la garde en trouve un, la bonne lecture est que la
résolution de chemin de `test_only_modules` est fausse — donc que **KTD3 est à réparer**, pas à
exempter. C'est R6 appliqué à la lettre : un faux positif dit que la règle de frontière est fausse.
**Écarté : (a)** — une exception nommée ici masquerait un lecteur cassé sous une entrée d'allowlist.

### U2 — le contrôle de bonne foi sur l'axe dangereux

**Population comptée : 24 occurrences**, et **zéro violation.** Les 24 lignes masquées commençant par
`pub fn` / `pub async fn` sont toutes légitimes (`test_utils.rs`, `Settings::test_defaults`,
`GitHubApp::seed_test_token`, `receiver_count`, `for_test`) et sont exclues **structurellement**, par
`test_only_modules` (KTD3) et par la clause d'indentation de KTD2 — jamais par une liste.

**Disposition : (a) allowlist nommée, avec une liste vide.** Le contrôle est bloquant dès le land, sans
exemption ni période de grâce, parce qu'il n'y a rien à exempter. **Aucune exemption n'est écrite pour
les 24 occurrences** : elles passent le contrôle, et exempter d'un contrôle ce qui le passe déjà crée
une dispense morte que plus rien ne nettoie — précisément la dette que le sous-point (3) de l'option (a)
cherche à éviter. La distinction à retenir pour un futur lecteur : **une exclusion structurelle n'est pas
une allowlist**, et c'est ce qui fait que la liste peut rester vide sans que le contrôle soit
permissif.

**Si le contrôle fire malgré tout : (c) halt-and-surface.** Une `pub fn` de production masquée signifie
que `production_slice` ampute de la production — c'est-à-dire que le remède a reproduit le défaut. Ni
exemption ni élargissement : arrêt, et réparation de la règle de frontière en U2.

### U3 — les gardes basculées, sur des offenses préexistantes

C'est la classe que F1 nomme en second, et la seule dont la population ne pouvait pas être supposée : la
bascule rend visible une zone que personne n'a lue. Elle a donc été **simulée en pré-vol** sur les deux
gardes à périmètre *arbre*, celles dont la zone aveugle est la somme des 24 fichiers.

| Garde basculée | Offenses préexistantes révélées | Lecture |
|---|---|---|
| `db.rs:14855` (mika#2335) | **0** — le motif `update_manual_task_status` + `"in_progress"` n'a aucun site de production dans la zone aveugle ; les 21 occurrences hors production sont toutes dans un `mod tests` que `production_slice` masque | reste verte |
| `agent_loop/mod.rs:14279` (mika#2305) | **1** — `prompt.rs:564-565`, aujourd'hui invisible parce que `prompt.rs` est coupé à la **ligne 142** par le doc-comment de mika#2292 | **rougit**, et c'est un résultat |
| `test_dispatch_fired_at_stamped.rs:223` | **0 par construction** — son prédicat est un `contains` *positif* (« le chemin de dispatch contient encore `mark_parent_dispatched` »), donc élargir la zone lue ne peut que la rendre plus verte, jamais plus rouge | reste verte |
| les dix autres | non simulées — périmètre *fichier propre*, cécité bornée | qualification en U3/2 |

**Le seul rougissement mesuré, et pourquoi il n'est pas un offenseur.** `mika2305_the_scope_has_a_single_decisional_reader`
asserte `sites.len() == 2` et son doc-comment nomme les deux attendus : « the decision
(`scoped_session_id`) and the rendering (`history_scope_label`) ». Son propre nom de test dit
« outside **deserialization** ». Or le site que la bascule révèle est `deserialize_history_scope`
(`prompt.rs:556-570`) — de la désérialisation, donc exactement ce que l'intention écrite exclut. **La
garde excluait la désérialisation par accident de cécité, pas par prédicat** : c'est le doc-comment de
mika#2292 à `prompt.rs:142` qui la coupait, et la Problem Frame le documentait déjà comme la forme (2)
la plus coûteuse — *« la doc qui explique une garde déplace une autre garde »*. Ici elle ne déplaçait
pas seulement une frontière : elle tenait une assertion en vie.

**Disposition : (c) halt-and-surface par défaut, et cet offenseur-ci est traité dans le périmètre.**
Traitement : rendre l'exclusion de la désérialisation **explicite** dans `scope_match_sites`. Ce n'est
pas un affaiblissement au sens de R6/AC — l'assertion (`== 2`), le message et la disposition de la garde
restent **inchangés** ; ce qui change est son prédicat de collecte, mis en conformité avec son propre
énoncé écrit. La nuance est écrite ici parce que sans elle le poseur se croit pris entre « ne rien
toucher à la garde » et « la laisser rouge », et choisirait probablement le contournement.

**Ce qui est interdit, et qui est la tentation la plus proche : (a) une exception par fichier ou par
zone.** Exempter `prompt.rs` de la garde `mika2305` ré-aveuglerait la garde sur exactement la zone que
la bascule vient de rendre visible — annulant le ticket dans le geste censé le livrer. Si une exception
nommée devient nécessaire pour un autre site, elle doit porter sur l'**offenseur exact** (chemin + ligne
+ motif), avec ticket de suivi et assertion auto-nettoyante ; à défaut de pouvoir la formuler à cette
granularité, c'est (c).

**Écarté pour cette classe : (b) land disabled.** Un `#[ignore]` sur une garde **existante et verte**
est une perte nette de couverture : elle protégeait déjà quelque chose avant ce ticket. L'option (b) est
réservée par la doctrine au cas où la violation existante est elle-même dangereuse à laisser non
signalée — ce n'est pas le cas ici, où la violation est un site de désérialisation parfaitement
légitime.

**La règle de qualification, pour les dix gardes non simulées.** Chaque rougissement se qualifie selon
R6/KTD5 et s'écrit dans le document U1 : offenseur réel → correction dans le périmètre, ou ticket nommé
et halt ; faux positif → la règle de frontière est fausse et se répare en U2. **Aucune garde n'est
mergée rouge** (KTD5 : « une garde rouge mergée est une garde qu'on désarme la semaine suivante »), ce
qui fait de (c) la seule issue quand la correction ne tient pas dans le périmètre.

**Halte nommée.** Si la bascule d'une garde à périmètre *arbre* révèle plus de trois offenses réelles
distinctes, s'arrêter et remonter : l'audit a trouvé plus que ce que ce ticket peut porter, et le
découper est une décision d'opérateur, pas une décision de poseur.

---

## Verification Contract

- `cargo test -p mika-agent -p mika-common -p mika-gateway -p mika-cli` — les gardes basculées restent vertes avec leurs assertions d'origine ; les tests de U2 et U4 passent.
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- `cargo build --release` — R7 : le lecteur est derrière `#[cfg(any(test, feature = "test-utils"))]` et n'entre dans aucun binaire.
- **Contrôle de non-régression de périmètre :** `git diff --stat` ne montre aucun fichier de production hors des déclarations de module. Si une ligne de production a changé, c'est que la bascule a débordé.
- **Contrôle de l'audit :** chaque site basculé en U3 figure dans le document U1 ; chaque site de U1 classé « cécité » est basculé ou porte un motif écrit de report.

## Definition of Done

- U1–U4 livrées, tests verts, clippy et fmt propres.
- Aucune garde n'a vu son assertion, son message ou sa disposition affaiblis pour repasser au vert.
- La disposition retenue en § *Fire-Disposition* a été **tenue** : aucune liste d'exceptions sur U4, aucune exemption par fichier sur une garde basculée, et tout halt annoncé a effectivement remonté à l'opérateur plutôt que d'être contourné.
- Chaque rougissement apparu pendant U3 est qualifié dans le document d'audit : offenseur réel (corrigé, ou ticket nommé) ou défaut de la règle de frontière (réparé en U2).
- Le document d'audit nomme ce qui **n'a pas** été traité et pourquoi : les chaînes `mod` de profondeur > 1, les scanners shell, les exclusions par chemin maintenues.
- Le corps de PR mène par le POURQUOI — les 14 843 lignes de production invisibles sur 24 fichiers, les cinq formes pour une question, le doc-comment de `prompt.rs` qui coûte 1 887 lignes à une garde voisine — et porte `Closes #2398`.

## Acceptance criteria

- [ ] Un document d'audit recense tous les scanners structurels du dépôt avec, par site : motif détecté, **périmètre de scan**, forme de séparation production/test, chemins en dur, direction de l'erreur, décision.
- [ ] `production_slice` conserve la production située **après** un helper `#[cfg(test)] fn` de niveau module, y compris quand sa signature est multi-lignes (formes 1 et 1b, tests dédiés).
- [ ] `production_slice` ignore une mention de `#[cfg(test)]` en commentaire ou en prose (forme 2, test dédié ancré sur `prompt.rs`).
- [ ] `production_slice` rend une production vide pour un fichier reconnu test-only **par sa déclaration chez le parent** (forme 3, test dédié).
- [ ] `production_slice` masque **exactement** l'attribut et son item pour un item mono-ligne, et conserve les 356 lignes de production qui suivent `egress_search/mod.rs:364` (forme 4, test dédié — contrôle de non-régression de la règle « premier `}` en colonne 0 »).
- [ ] `production_slice` conserve un item sous `#[cfg(not(test))]` comme production (forme 5, test dédié).
- [ ] `production_slice` masque une `#[cfg(test)] fn` indentée sans masquer l'`impl` de production qui la contient (forme 6, test dédié ancré sur `github.rs`).
- [ ] Les numéros de ligne des lignes de production sont inchangés après masquage d'une région de test.
- [ ] Aucune ligne masquée sur l'arbre réel n'est une `pub fn` de production : les seules occurrences sont celles que U1 a classées test-only.
- [ ] **Les treize sites** du tableau de la Problem Frame appellent `production_slice`, et chacun passe avec son assertion, son message et sa disposition **inchangés** (un prédicat de collecte mis en conformité avec l'énoncé écrit de la garde n'est pas un affaiblissement — voir § *Fire-Disposition* / U3).
- [ ] Une garde anti-récidive refuse toute réimplémentation locale de la séparation production/test, sans liste d'exceptions, et rougit sur une fixture de récidive.
- [ ] Le plan porte une section `## Fire-Disposition` qui, pour **chacun** des quatre livrables de classe détecteur (U4/1, U4/4, U2, U3), nomme une des trois options canoniques de mika#1574 avec sa population comptée et son détail d'implémentation.
- [ ] La garde `mika2305_the_scope_has_a_single_decisional_reader` passe après bascule en excluant la désérialisation **par prédicat explicite** et non par cécité, avec son `assert_eq!(sites.len(), 2)` inchangé.
- [ ] Aucune exception n'est posée par fichier ni par zone sur une garde basculée ; toute exemption éventuelle porte sur un offenseur exact (chemin + ligne + motif) avec ticket de suivi et assertion auto-nettoyante.
- [ ] Aucune garde ne repose sur une convention de nom de fichier pour décider qu'un fichier est test-only.
- [ ] `cargo build --release` réussit et le lecteur n'entre dans aucun binaire de production ; aucun fichier de production ne change hors déclarations de module.
- [ ] Le document nomme les populations non traitées (chaînes `mod` de profondeur > 1, scanners shell, exclusions par chemin maintenues) avec, pour chacune, la conséquence de son maintien.

## Sources

- Issue senara-solutions/mika#2398 — titre « Audit des autres scanners de sources structurels (prémisse cfg(test) inline) », **corps vide** ; commentaire opérateur du 18/09 (« Suite du découpage db.rs (#2321) »).
- **Mesures du 2026-09-19** dans le worktree, sur 337 fichiers `.rs` des cinq crates : 14 843 lignes de production non vides perdues sur 24 fichiers ; 8 items `cfg(test)` mono-ligne ; 13 attributs `cfg(test)` indentés ; 1 `cfg(not(test))` ; 2 fichiers test-only sans marqueur ; 3 fichiers test-only hors convention de nom ; 481 lignes sur-amputées par la règle « premier `}` en colonne 0 ».
- mika#2321 — découpage de `db.rs` ; `scripts/check-secrets.sh:44` (exception `LARGE_FILE_ALLOWLIST`, posée par le commit `5a7a50fb`).
- mika#2310 — `crates/mika-agent/src/db/tests/harnais_porte.rs`, premier fichier test-only né de la pression du plafond ; son commentaire de tête dit pourquoi.
- **Les treize formes d'amputation**, mesurées par scan le 2026-09-19 : `db.rs:14855`, `agent_loop/mod.rs:14279` et `:8739`, `auto_pull.rs:4458` et `:6880`, `prompt.rs:5871`, `server/a2a.rs:1976`, `server/deadline_verdict.rs:1248`, `tests/eval/test_dispatch_fired_at_stamped.rs:223`, `tests/eval/test_recurring_trigger_wiring_2337.rs:70`, `tests/ac8_grep_discipline.rs:41`, `mika-gateway/src/telegram.rs:2265`, `mika-gateway/src/telegram_markdown.rs:800`.
- **La simulation de bascule du 2026-09-19** (§ Fire-Disposition / U3) : 0 offense sur le motif de `db.rs:14855` (`update_manual_task_status` + `"in_progress"` : 21 occurrences hors production, toutes masquées par `production_slice`) ; **1** offense sur celui d'`agent_loop/mod.rs:14279` (`HistoryScope::` en bras de match — `prompt.rs:564-565`, `deserialize_history_scope`, invisible aujourd'hui parce que `prompt.rs` est coupé à la ligne 142) ; 0 par construction pour `test_dispatch_fired_at_stamped.rs:223`, dont le prédicat est un `contains` positif.
- `docs/solutions/best-practices/fire-disposition-doctrine.md` — les trois options canoniques (a)/(b)/(c) et le gate mika#1574 qui les exige.
- `crates/mika-agent/Cargo.toml:83` et `crates/mika-gateway/Cargo.toml:68` — `mika-common` avec `features = ["test-utils"]` déjà en `[dev-dependencies]` dans les deux crates que U3 touche.
- Les faux-amers mesurés : `auto_pull.rs:1922`→`1936` (helper `#[cfg(test)] fn`), `prompt.rs:142` (mention en prose, mika#2292, −1 887 lignes), `agent_loop/mod.rs:8621` et `db.rs:14780` (mentions en prose bénignes), `builtin_handlers.rs:590`/`:592` (`cfg(not(test))` + mono-ligne adjacents), `github.rs:598`–`619` (`impl` de production à trois `fn` de test indentées), `egress_search/mod.rs:364` (mono-ligne, −356 lignes si mal bornée), `lib.rs:46` + `test_utils.rs`, `llm/mod.rs:5` + `mock.rs`, `voice/mod.rs:81` + `examples.rs` (test-only hors convention de nom).
- Les gardes sans amputation : `ready_label.rs:1030` (mika#2315), `planning/policy.rs:143` (mika#2189), `grooming_marker.rs:677` (mika#2158), `auto_pull.rs:6849` (mika#2361), `auto_pull_stop.rs:339` (mika#2329), `deadline_verdict.rs:1228` (mika#2368), `pr_merge_with_gate.rs:2392` (mika#2238), `dispatcher.rs:3966`/`:4017` (mika#2205), `llm/mod.rs:1467` (mika#2342), `image_disposition.rs:579` (mika#1784).
- Doctrine : `feedback_structural_gate_audit_grep_all_callsites` (« une garde qui couvre trois appelants sur quatre est une garde qui ment »), citée au site de `planning/policy.rs`.
- `mika_common::llm::mock` et `Settings::test_defaults()` — précédent de la porte `test-utils` pour du code de test partagé entre crates.

## Revision history

- **rev 2 (2026-09-19) — adressé F1** (section `## Fire-Disposition` manquante pour les deliverables de
  classe détecteur ; citation : review-guide.md § Fire-Disposition Gate, mika#1574).

  Ajout de la section `## Fire-Disposition`, placée entre les Implementation Units et le Verification
  Contract. Elle couvre **quatre** livrables et non les deux que F1 nommait : U4/1 (garde anti-récidive)
  et U4/4 (garde de cohérence KTD3) comme demandé, plus U2 (le contrôle de bonne foi sur l'axe
  dangereux, qui est aussi un scan bloquant) et U3 (les gardes basculées, la classe que F1 nomme en
  second). Options retenues : **(c)** pour U4/1 avec liste d'exceptions vide et interdite, **(c)** pour
  U4/4, **(a) avec liste vide** pour U2, **(c) par défaut avec traitement dans le périmètre** pour U3.
  Chaque option porte sa population comptée, son détail d'implémentation, et les options écartées avec
  leur motif.

  Trois mesures ont été prises en pré-vol pour ne pas écrire la section sur une population supposée, et
  **deux d'entre elles ont contredit le plan** — répercuté hors de la seule section :

  1. **Le recensement des amputations était incomplet : 13 sites, pas 7.** Six sites étaient absents
     (`prompt.rs:5871`, `server/a2a.rs:1976`, `server/deadline_verdict.rs:1248`,
     `tests/eval/test_recurring_trigger_wiring_2337.rs:70`, `mika-gateway/src/telegram.rs:2265`,
     `telegram_markdown.rs:800`), dont deux hors `mika-agent`, et deux d'entre eux étaient rangés à
     l'opposé, parmi les gardes « qui n'amputent rien ». Tableau de la Problem Frame refait avec une
     colonne *direction de l'erreur*, note de correction ajoutée, liste des gardes sans amputation
     corrigée, U1/1 doté du scan qui aurait attrapé l'omission, Sources mises à jour. **Conséquence sur
     le périmètre :** U3 passe de 7 à 13 sites, parce que U4 refuse la réimplémentation et non la
     cécité — un site laissé en place la fait rougir et bloque le PR. C'est la disposition (c) de U4/1
     qui impose cet élargissement, et le dire est tout l'objet du gate.
  2. **La bascule fait rougir exactement une garde, et ce n'est pas un offenseur.**
     `mika2305_the_scope_has_a_single_decisional_reader` asserte `sites.len() == 2` ; la bascule révèle
     `prompt.rs:564-565` (`deserialize_history_scope`), aujourd'hui invisible parce que `prompt.rs` est
     coupé à la **ligne 142** par le doc-comment de mika#2292 — la forme (2) que la Problem Frame
     décrivait déjà comme la plus coûteuse. Le nom du test dit « outside deserialization » : la garde
     excluait la désérialisation **par accident de cécité, pas par prédicat**. Traité dans le périmètre
     en rendant l'exclusion explicite, sans toucher l'assertion, le message ni la disposition.
     L'exception par fichier est explicitement interdite : elle ré-aveuglerait la garde sur la zone que
     la bascule vient de rendre visible.
  3. **Aucun `Cargo.toml` n'est modifié.** `mika-agent` et `mika-gateway` — les deux seules crates que
     U3 touche — déclarent déjà `mika-common = { features = ["test-utils"] }` en `[dev-dependencies]`.
     Noté en U2/Verification, ce qui rend R7 vérifiable plutôt que postulé.

  Trois AC ajoutés (existence et complétude de la section ; `mika2305` passant par prédicat explicite ;
  interdiction de l'exemption par fichier), un AC resserré (« les treize sites » au lieu de « chaque
  garde classée cécité »), une ligne ajoutée à la Definition of Done. **Aucun AC affaibli** — la
  résolution de F1 en a rendu un strictement plus exigeant.

  Rien n'a été laissé en « Could not address ».
