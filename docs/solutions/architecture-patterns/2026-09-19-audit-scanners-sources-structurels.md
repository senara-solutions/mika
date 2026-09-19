---
module: mika-common
tags: [structural-guard, source-scan, cfg-test, production-boundary, audit, db-split]
problem_type: silent-blindness
issues: [2398, 2321, 2310]
date: 2026-09-19
---

# Audit des scanners de sources structurels

## Ce que l'audit mesure, et pourquoi la moitié de l'erreur ne se voit pas

Ce dépôt s'appuie sur une trentaine de **gardes structurelles** : des tests qui
balaient l'arbre source et refusent un motif — un second écrivain, un lecteur
hors module, une constante retirée. Elles existent parce qu'aucun test
comportemental ne peut voir la classe de défaut qu'elles visent : celle qui ne
rend **aucune décision fausse** et rend l'attribution impossible.

Toutes doivent séparer la production du test, sinon leur propre `mod tests` — qui
nomme le motif par construction — devient leur premier offenseur. **Treize le
faisaient à la main, de six façons différentes.** La conséquence n'est pas
symétrique :

- une garde qui **déborde** casse le CI et se fait remarquer dans l'heure ;
- une garde qui **a cessé de regarder** reste verte, et c'est exactement ce
  qu'elle promettait de ne jamais faire.

**Le chiffre qui ordonne le travail : une garde qui coupe à la première
occurrence de `#[cfg(test)]` perd 14 043 lignes de production non vides,
réparties sur 22 fichiers et 5 crates.** Ce n'est pas un défaut ponctuel : c'est
la propriété de la prémisse.

### Méthode, pour que le relevé soit rejouable

Toutes les mesures ci-dessous sont prises le **2026-09-19** dans le worktree
`feat/2398/audit-des-autres-scanners-de-sources`, sur **338 fichiers `.rs`** des
cinq crates (`mika-agent`, `mika-common`, `mika-gateway`, `mika-cli`,
`mika-a2a`). Deux instruments :

1. le recensement des réimplémentations —
   `(find|split|split_once|contains|starts_with|splitn)` appliqué à un littéral
   contenant `cfg(test)` ou `mod tests` ;
2. le lecteur `mika_common::source_guard` lui-même, appliqué fichier par fichier,
   la « perte » d'un fichier étant le nombre de ses lignes de **production non
   vides** situées après la première occurrence du marqueur.

Les chiffres sont **recalculés**, pas recopiés du plan : un chiffre transcrit est
un chiffre qui a déjà commencé à vieillir. Trois d'entre eux corrigent le plan —
c'est dit à chaque fois.

---

## 1. Le relevé par fichier (la zone aveugle d'une garde à périmètre arbre)

| lignes de production perdues | fichier | coupe à la ligne |
|---:|---|---:|
| 3 053 | `mika-agent/src/skills/builtin_handlers.rs` | 592 |
| 1 886 | `mika-agent/src/prompt.rs` | **142** |
| 1 859 | `mika-agent/src/auto_pull.rs` | 1 922 |
| 1 220 | `mika-cli/src/tui/app.rs` | 364 |
| 1 065 | `mika-agent/src/milestone_manager/spawn.rs` | 70 |
| 985 | `mika-gateway/src/github.rs` | 600 |
| 762 | `mika-common/src/llm/ollama.rs` | 29 |
| 747 | `mika-cli/src/tui/commands/handlers.rs` | 479 |
| 670 | `mika-gateway/src/routes.rs` | 2 185 |
| 579 | `mika-common/src/claude.rs` | 457 |
| 271 | `mika-gateway/src/egress_search/mod.rs` | 57 |
| 233 | `mika-agent/src/agent_loop/review_anchor.rs` | 90 |
| 221 | `mika-agent/src/server/permissions_stream.rs` | 550 |
| 148 | `mika-agent/src/skills/quoted_resources.rs` | 272 |
| 128 | `mika-agent/src/server/dashboard.rs` | 1 281 |
| 111 | `mika-agent/src/perimeter/mod.rs` | 48 |
| 43 | `mika-agent/src/server/tasks_stream.rs` | 213 |
| 16 | `mika-agent/src/milestone_manager/mod.rs` | 111 |
| 15 | `mika-agent/src/milestone_manager/reader.rs` | 483 |
| 13 | `mika-agent/src/agent_loop/mod.rs` | 8 621 |
| 10 | `mika-gateway/src/egress_search/brave.rs` | 265 |
| 8 | `mika-agent/src/lib.rs` | 46 |
| **14 043** | **22 fichiers** | |

> **Correction du plan.** Le plan annonçait 14 843 lignes sur 24 fichiers. L'écart
> vient de la définition : il comptait toutes les lignes après la coupe, régions de
> test comprises. Ici seules les lignes **de production** sont comptées — une ligne
> de test perdue par une troncature n'est pas une perte. 14 043 / 22 est le chiffre
> à citer.

**La distinction de périmètre décide de l'ampleur.** Une garde qui ne lit que son
propre fichier a une cécité bornée par ce fichier. Une garde qui balaie `src/`
hérite de la **somme** du tableau. Deux gardes étaient dans ce second cas :
`db.rs` (mika#2335) et `agent_loop/mod.rs` (mika#2305).

---

## 2. Les six formes que la prémisse ignore

La prémisse implicite — « les tests sont dans un `mod tests` inline, précédé d'un
`#[cfg(test)]` en fin de fichier, et il n'y en a qu'un » — est fausse de six
façons, **toutes présentes dans le dépôt**. Une septième a été trouvée pendant la
bascule.

| # | forme | occurrences mesurées | direction | couverte par |
|---|---|---:|---|---|
| 1 | helper `#[cfg(test)] fn` de niveau module | — (`auto_pull.rs:1922`) | cécité | KTD2 clause 3 |
| 2 | mention du marqueur **en prose** (doc-comment) | — (`prompt.rs:142`) | cécité | KTD2 clause 1 (ligne entière) |
| 3 | fichier intégralement de test, **sans marqueur** | **9** | faux positif | KTD3 |
| 4 | item `#[cfg(test)]` **mono-ligne**, sans bloc | **8** | sur-amputation | KTD2 clause 2 |
| 5 | `#[cfg(not(test))]` marquant de la **production** | **1** | inversion | KTD2 clause 1 |
| 6 | attribut `#[cfg(test)]` **indenté** dans un item de production | **13** | cécité *ou* faux positif | KTD2 clause 3 (indentation) |
| 7 | ligne de code **citée dans un littéral multi-ligne** | — (fixtures de ces gardes) | faux positif | lexer à état inter-lignes |

**(1) Le helper de niveau module.** `auto_pull.rs:1922` porte
`#[cfg(test)] fn select_feeder_candidates(…)`, qui se ferme ligne 1936 — 1 975
lignes avant le vrai `mod tests`. Toute garde en `find` coupe à 1922 et perd
1 859 lignes de production. C'est la direction dangereuse : la garde ne rougit
pas, elle ne voit plus.

**(2) La mention en prose — la plus coûteuse, pas la plus bénigne.**
`prompt.rs:142` est un doc-comment qui contient `` `#[cfg(test)]` `` en
décrivant où vit la denylist de mika#2292. C'est la **première** occurrence de la
chaîne dans le fichier : une garde en `find` y coupe `prompt.rs` et perd 1 886
lignes — la quasi-totalité du fichier qui porte les sections de prompt
code-managed. Le mécanisme est le plus vicieux de la famille : **la doc qui
explique une garde déplace une autre garde**, et la frontière cesse d'être une
propriété du code pour devenir une propriété de sa rédaction.

**(3) Les neuf fichiers intégralement de test.** Mesurés :
`mika-agent/src/{db/tests/harnais_porte.rs, milestone_manager/no_dispatch_test.rs,
perimeter/tests.rs, test_utils.rs}`,
`mika-common/src/{llm/mock.rs, source_guard.rs}`,
`mika-gateway/src/{egress_search/tests_e3_request_shape.rs,
egress_search/tests_e4_no_log.rs, voice/examples.rs}`. Deux d'entre eux ne
contiennent **aucune** occurrence de `cfg(test)` : l'attribut est sur la
déclaration `mod` chez le parent. Pour les sept formes d'amputation, l'amputation
est alors un no-op et le fichier entier est lu comme de la production.

**(4) Les huit items mono-ligne.** `skills/builtin_handlers.rs:592`
(`#[cfg(test)] const PROGRESS_TICKER_INTERVAL`) et **sept** déclarations de
module (`lib.rs:46`, `milestone_manager/mod.rs:111`, `perimeter/mod.rs:48`,
`egress_search/mod.rs:57` et `:364`, `llm/mod.rs:5`, `voice/mod.rs:81`). Ces
items n'ont pas d'accolade fermante : toute règle qui cherche « la prochaine `}`
en colonne 0 » **sur-ampute** de 481 lignes, dont 356 sur le seul
`egress_search/mod.rs:364`.

> **Correction du plan.** Il annonçait « 8 sites, dont … et six déclarations de
> module », ce qui fait 7. La mesure rend 8 = 1 `const` + 7 `mod`.

**(5) `cfg(not(test))`.** Un site, `builtin_handlers.rs:590`, immédiatement
au-dessus du (4). Une règle qui reconnaîtrait « un attribut `cfg` contenant
`test` » inverserait le sens sur cette ligne et masquerait 3 053 lignes de
production.

**(6) Les treize attributs indentés.** `mika-gateway/src/github.rs:600/606/612`
(un `impl ForwardResult` de production contenant trois `#[cfg(test)] fn`),
`mika-agent/src/{agent_loop/review_anchor.rs:90, bundled_skills.rs:1740,
server/permissions_stream.rs:550, server/tasks_stream.rs:213}`,
`mika-common/src/{claude.rs:463, config.rs:2348, github_app.rs:150/163/175}`,
`mika-cli/src/tui/app.rs:364`. Le plan précédent pariait sur zéro et en faisait
un angle mort « théorique ».

**(7) La forme trouvée en chemin.** Les fixtures de ces gardes citent des
fichiers Rust entiers, attribut d'ouverture **et accolade fermante en colonne 0**
comprises. Une règle ligne-à-ligne lit cette accolade comme la fermeture de la
région, termine le `mod tests` trop tôt et rend le reste comme de la production.
Trouvée en basculant `mika2305`, réparée par un lexer qui porte l'état de chaîne
d'une ligne à l'autre, épinglée par `mika2398_f8_*`.

---

## 3. Les treize réimplémentations, avec la direction de leur erreur

Recensement par scan (voir § Méthode). **Tous ont basculé** sur
`mika_common::source_guard`.

| Site (avant bascule) | Forme | Périmètre | Direction de l'erreur | État |
|---|---|---|---|---|
| `db.rs:14855` (mika#2335) | `src.find("#[cfg(test)]")` | **arbre** `mika-agent/src` | cécité, **toutes formes** | basculé |
| `agent_loop/mod.rs:14279` (mika#2305) | `src.find("#[cfg(test)]")` | **arbre** `mika-agent/src` | cécité, **toutes formes** | basculé |
| `auto_pull.rs:4458` | `split("#[cfg(test)]")[0]` | fichier propre | cécité, toutes formes | basculé |
| `auto_pull.rs:6880` (mika#2361) | `split_once("mod tests {")` | arbre, moins 2 chemins en dur | **faux positif** (helper de niveau module reste en production) | basculé + R5 |
| `prompt.rs:5871` (mika#2292) | `split_once("\nmod tests {")` | fichier propre | **faux positif**, idem | basculé |
| `agent_loop/mod.rs:8739` (mika#2342) | `split_once("\n#[cfg(test)]\nmod tests {")` | fichier propre | cécité sur (3) | basculé |
| `server/a2a.rs:1976` (mika#2363) | `split("\n#[cfg(test)]\n")` | fichier propre | cécité sur (1), (3), (4) | basculé |
| `server/deadline_verdict.rs:1248` (mika#2368) | `text.find("\n#[cfg(test)]")` | **arbre** | cécité sur (1), (3), (4) | basculé |
| `tests/eval/test_dispatch_fired_at_stamped.rs:223` (mika#2335) | `body.find("#[cfg(test)]")` | 3 fichiers nommés | **faux positif** (prédicat `contains` positif) | basculé |
| `tests/eval/test_recurring_trigger_wiring_2337.rs:70` (mika#2337) | `find("\n#[cfg(test)]\nmod ")` | **arbre** | cécité sur (3) | basculé |
| `tests/ac8_grep_discipline.rs:41` (mika#1733) | ligne-à-ligne + **comptage d'accolades** | **arbre** | **les deux**, selon la dérive du compteur | basculé |
| `mika-gateway/src/telegram.rs:2265` (mika#2291) | `source.find("\n#[cfg(test)]")` | fichier propre | cécité sur (1), (3), (4) | basculé |
| `mika-gateway/src/telegram_markdown.rs:800` (mika#2291) | `source.find("\n#[cfg(test)]")` | fichier propre | cécité sur (1), (3), (4) | basculé |

**Le préfixe `\n` n'est pas cosmétique**, et c'est ce qui rend la direction
mesurable par site. `find("#[cfg(test)]")` nu coupe sur une mention en prose
(forme 2) ; `find("\n#[cfg(test)]")` exige l'attribut en tête de ligne, donc il
est robuste à un doc-comment et à un attribut indenté, mais reste aveugle à un
helper de niveau module (1) et à un item mono-ligne (4).
`split_once("\n#[cfg(test)]\nmod tests {")` est le plus étroit : seule la forme
(3) le trompe. Deux formes se trompent dans la direction **inverse** :
`split_once("mod tests {")` laisse un helper `#[cfg(test)] fn` de niveau module
*dans* la production. **La bascule n'est donc pas uniforme : elle rend certaines
gardes voyantes et en resserre d'autres.**

### Ce que la bascule a effectivement révélé

| Garde basculée | Offenses préexistantes révélées | Lecture |
|---|---|---|
| `db.rs:14855` (arbre, 14 043 lignes aveugles) | **0** | le motif `update_manual_task_status` + `"in_progress"` n'a aucun site de production dans la zone aveugle |
| `agent_loop/mod.rs:14279` (arbre) | **0** | voir ci-dessous |
| les onze autres | **0** | périmètre fichier propre, cécité bornée |

> **Correction du plan — la simulation pré-vol était fausse.** Le plan annonçait
> que `mika2305_the_scope_has_a_single_decisional_reader` rougirait sur
> `prompt.rs:564-565` (`deserialize_history_scope`), et sa section
> *Fire-Disposition* prescrivait de rendre l'exclusion de la désérialisation
> explicite. **Elle l'est déjà.** Le prédicat `scope_match_sites` est
> *positionnel* : il exige `HistoryScope::` **dans le motif**, à gauche du `=>`.
> Les bras de `deserialize_history_scope` *produisent* le type à droite de leur
> flèche et tombent d'eux-mêmes, ce que leur doc-comment dit en toutes lettres.
> La correction prescrite était sans objet ; aucune ligne de `scope_match_sites`
> n'a été touchée, et son `assert_eq!(sites.len(), 2)` est inchangé.
>
> La simulation portait donc sur un prédicat plus grossier que le vrai. C'est
> exactement la classe de défaut que ce ticket ferme, appliquée à sa propre
> préparation : une mesure prise sur une approximation du code répond à une
> question voisine de celle qu'on pose.

**Aucune garde n'a vu son assertion, son message ou sa disposition affaiblis.**
Le seul changement de prédicat est celui de `ac8_grep_discipline`, qui abandonne
son compteur d'accolades — et c'est un resserrement, pas un affaiblissement.

---

## 4. Les gardes qui ne séparent rien du tout

Douze autres scanners lisent l'arbre source **sans aucune séparation
production/test**, ou n'excluent au mieux que leur propre fichier en entier.
Mécanisme vérifié site par site :

| Site | Ticket | Périmètre | Exclusion effective |
|---|---|---|---|
| `auto_pull_stop.rs:339` | mika#2329 | arbre | `path == this_module` |
| `ready_label.rs:1033` | mika#2315 | arbre | `path == this_module` |
| `grooming_marker.rs:685` | mika#2158 | arbre | `path == this_module` |
| `image_disposition.rs:580` | mika#1784 | arbre | `path == this_module` |
| `image_disposition.rs:527` | mika#1784 | 1 fichier | aucune |
| `tools/pr_merge_with_gate.rs:2393` | mika#2238 | arbre | `path == this_module` |
| `mika-common/src/llm/retry_gate.rs:518` | mika#2362 | arbre | `path == this_module` |
| `planning/policy.rs:160` | mika#2189 | arbre | **aucune** |
| `server/mod.rs:1947` | mika#2290 | arbre | **aucune** |
| `agent_loop/mod.rs:14214` | mika#2305 | 1 fichier | aucune |
| `task_engine/dispatcher.rs:3966` / `:4017` | mika#2205 | 1 fichier | corps de fonction |
| `tests/only_skills_arch_pass_2363.rs:193` | mika#2363 | `src/server` | aucune |
| `perimeter/tests.rs:600`, `tests/eval/manager_loop_resistance.rs:172` | mika#1948 | `src/milestone_manager` | aucune |

**Leur direction d'erreur est le faux positif**, pas la cécité : elles lisent le
`mod tests` de tous les autres fichiers comme de la production. Elles sont vertes
aujourd'hui parce que leur motif ne se trouve pas dans du code de test, ou parce
qu'elles filtrent les commentaires — **pas parce qu'elles séparent**.

Deux sites initialement rangés ici n'y appartiennent pas, après vérification :

- `mika-common/src/llm/mod.rs:1503` (mika#2342) **n'est pas un balayage d'arbre** :
  il lit trois rails nommés et asserte un `contains` **positif**. Élargir la zone
  lue ne peut que le rendre plus vert. Hors population.
- `mika-common/src/permission_authority.rs:372` **a été basculé**, hors des treize.
  C'est la moitié `mika-common` du contrôle AC8 dont la moitié `mika-agent`
  (`tests/ac8_grep_discipline.rs`) est dans les treize : un même invariant,
  répondant à « est-ce de la production ? » de deux façons différentes — l'une par
  comptage d'accolades, l'autre en n'excluant que son propre fichier. Laisser la
  paire asymétrique aurait été introduire la divergence que ce ticket ferme, à
  l'échelle d'un seul invariant (doctrine `feedback_structural_gate_audit_grep_all_callsites`,
  « une garde qui couvre trois appelants sur quatre est une garde qui ment »). La
  bascule est verte ; le whitelisting de son propre fichier par son nom a disparu,
  le lecteur masquant ce module de test comme n'importe quel autre.

**Décision pour les douze restants : laissés en l'état, et c'est un report
motivé.** Ils ne réimplémentent pas la frontière, donc la garde anti-récidive
(§ 7) ne les touche pas ; leur erreur est dans la direction bruyante ; et les
basculer serait un **resserrement** susceptible de faire rougir des gardes
voisines pour des raisons sans rapport avec ce ticket. R3 donne la priorité à la
cécité : elle ment en vert. **Ticket de suivi**, à faire une garde à la fois,
avec le même protocole de qualification qu'ici.

### Hors périmètre, et pourquoi

- **`bin/verify_bundled_skills.rs`, `tools/mod.rs:1291`, `tests/bundled_skills_*`,
  `tests/skills_load_path.rs`, `tests/qa_review_*`, `tests/review_anchor_prompt_contract.rs`,
  `tests/skill_prompt_user_prescription_guard_2024.rs`** — ils scannent
  `skills/bundled/`, pas l'arbre Rust. La prémisse `cfg(test)` ne les concerne pas.
- **`task_engine/worktree_activity.rs`** — production, scanne le worktree d'un
  pilote.
- **`server/openapi.rs`, `mika-gateway/src/openapi.rs`, `tests/rt005_analyze.rs`,
  `tests/eval/kg_provider_eval/*`** — lisent des YAML/JSON/fixtures.
- **`tests/eval/test_eval_modules_declared.rs:46`** — scanne `tests/eval`, où la
  notion de production n'a pas de sens.
- **Les scanners shell (`scripts/check-*.sh`, `verify-*.sh`)** — ils ne portent
  pas la prémisse `cfg(test)`. Leur classe de fragilité est l'exclusion par
  chemin, traitée en § 5.

---

## 5. Les exclusions par chemin en dur (R5)

| Site | Exclusion | Ce que mika#2321 lui fera | Traitement |
|---|---|---|---|
| `auto_pull.rs:6852` (mika#2361) | `[src/db.rs, src/async_db.rs]` | le jour où `Database` se répartit dans `db/*.rs`, ces chemins désignent des fichiers qui ne portent plus ce qu'ils excluaient, et les nouveaux entrent dans la population sans que personne l'ait décidé | **remplacée** par un critère qui suit le code : le fichier est exclu parce qu'il **définit** la méthode (`fn <needle>`), trouvé par scan |
| `scripts/check-secrets.sh:44` | `LARGE_FILE_ALLOWLIST` contient `db.rs` (plafond 1 Mo, commit `5a7a50fb`) | l'exception devient inutile une fois le découpage fait | **maintenue, datée** : la retirer avant le découpage remettrait le plafond en travers du chemin. Elle est le rappel de mika#2321 ; c'est à ce ticket de la retirer |
| `auto_pull_stop.rs`, `ready_label.rs`, `grooming_marker.rs` | `path == this_module` | rien — le fichier propre reste le fichier propre | **maintenues** : elles ne désignent pas un fichier tiers, donc elles ne peuvent pas se périmer en silence |

**Bound connue du nouveau critère, nommée plutôt que découverte :** un appelant
décisionnel ajouté dans un fichier qui définit aussi la méthode serait manqué. La
couche DB n'est pas un endroit où un appelant décisionnel a sa place, et ce
serait une anomalie en soi.

---

## 6. Le lecteur unique, et ce qu'il ne fait pas

`mika_common::source_guard`, derrière `#[cfg(any(test, feature = "test-utils"))]`
— la porte déjà en vigueur pour `MockLlmProvider` et `Settings::test_defaults()`.
Elle garantit R7 par construction : **rien de ceci n'entre dans un binaire de
production**. Aucun `Cargo.toml` n'a été modifié : `mika-agent` et `mika-gateway`
déclaraient déjà `mika-common` avec `features = ["test-utils"]` en
`[dev-dependencies]`.

La règle est une propriété **d'indentation**, pas un comptage d'accolades : les
`mod tests` de ce dépôt sont pleins de littéraux JSON et un compteur y dérive,
dans une direction ou dans l'autre, sans signal. L'indentation est garantie par
`rustfmt`, que le CI impose déjà (`cargo fmt --check`) — la règle s'appuie donc
sur un invariant **vérifié ailleurs** plutôt que sur une hypothèse nouvelle.

### Angles morts assumés, chacun mesuré

| Angle mort | Mesure au 2026-09-19 | Surface |
|---|---|---|
| région dont la ligne de fermeture est introuvable | **0** sur 338 fichiers | `SliceReport::unclosed` ; la région est alors **laissée visible**, jamais masquée jusqu'à la fin du fichier — le sur-masquage est la direction silencieuse |
| déclaration `mod X;` ne résolvant vers aucun fichier | **0** | `TestOnlyModules::unresolved()` ; `mika2398_every_test_only_declaration_resolves_to_a_file` (halt-and-surface, sans exemption) |
| chaîne `mod` de profondeur > 1 dont un maillon intermédiaire n'est pas `cfg(test)` | profondeur max mesurée = **1** | `mika2398_the_module_chain_depth_stays_within_the_measured_bound` |
| littéral de chaîne brute multi-ligne entre l'attribut et son item | borné à 64 lignes (`MAX_ITEM_HEADER_LINES`), puis `unclosed` | idem première ligne |

**La condition d'arrêt résiduelle du plan n'est pas atteinte** : aucun fichier de
production ne porte d'item de niveau module qui ne se ferme pas à sa propre
indentation. L'invariant `rustfmt` sur lequel KTD2 repose tient sur les 338
fichiers.

### Le contrôle de bonne foi sur l'axe dangereux

**8 `pub fn` masquées sur l'arbre réel**, toutes des helpers de test sous un
attribut `cfg(test)` **indenté** : `receiver_count` (×2), `App::new`,
`ClaudeClient::for_test`, `Settings::test_defaults`, `GitHubApp::{new,
seed_test_token, new_with_test_token}`.

Elles sont exclues **structurellement** — par KTD3 et par la clause
d'indentation — **jamais par une liste**. Le recensement est figé
(`MASKED_PUB_FN_CENSUS`), et la distinction mérite d'être écrite : *un
recensement figé n'est pas une allowlist*. Aucune de ces huit n'est exemptée de
quoi que ce soit ; elles passent le contrôle, et exempter d'un contrôle ce qui le
passe déjà crée une dispense morte que plus rien ne nettoie.

Ce que le figeage achète, dit sans surpromesse : **aucun prédicat mécanique ne
peut distinguer une fonction de production avalée d'un helper légitimement
masqué** sans re-dériver ce que le masqueur vient de décider, ce qui serait
circulaire. Ce qu'il achète est qu'une future permissivité de la règle ne puisse
pas passer inaperçue — une région qui se mettrait à déborder ajouterait des noms,
et le diff les nomme.

---

## 7. La garde anti-récidive

`mika2398_no_scanner_reimplements_the_production_test_boundary` balaie
`crates/*/src/**` **et** `crates/*/tests/**` et refuse toute ligne de production
qui compose la séparation à la main.

**Disposition : halt-and-surface, liste d'exceptions vide et interdite.** La
raison est chiffrée et non de principe : six exceptions sur treize laisseraient
une garde qui tolère la moitié de ce qu'elle interdit, et le jour où la
quatorzième arrive elle ne dirait plus rien — c'est la forme exacte du défaut
qu'elle ferme.

**Pourquoi un scan de source est le seul moyen.** Une quatorzième
réimplémentation ne rendrait **aucune** décision fausse le jour où elle est
écrite ; elle divergerait ensuite, comme les treize ont divergé — l'une d'elles
portant un commentaire disant qu'elle *recopiait* une autre, après quoi elle a
manqué ses deux élargissements. Aucune assertion comportementale ne peut voir ça.
Même famille que `grooming_marker::no_grooming_regex_outside_this_module` et
`auto_pull::mika2131_exclusion_skips_never_return_to_an_uncollected_debug`.

La garde est aussi **le premier client du lecteur** : un run vert prouve en
passant que le lecteur fonctionne sur le fichier le plus hostile qui soit, celui
qui nomme le motif qu'il interdit.

---

## 8. Le lien avec le découpage de `db.rs` (mika#2321)

`db.rs` pèse ~1,1 Mo et a franchi le plafond de 1 Mo de `scripts/check-secrets.sh`.
Le découpage frappera les gardes sur **deux** axes, tous deux traités ici :

- **Les exclusions par chemin en dur cessent de couvrir le code déplacé** —
  traité en § 5 (la seule qui désignait un fichier tiers a été remplacée par un
  critère qui suit le code).
- **Les formes (3) et (4) sont exactement ce que le découpage produit.**
  `db/tests/harnais_porte.rs` en est déjà un, né de mika#2310 précisément parce
  que `db.rs` frôlait le plafond ; chaque tranche future ajoutera un fichier
  test-only **et** la déclaration `#[cfg(test)] mod …;` mono-ligne qui va avec.
  Les deux sont couvertes par KTD3 et KTD2 clause 2.

**Ce ticket précède mika#2321.** Découper d'abord ferait rougir des gardes pour
une raison sans rapport avec le découpage, au moment le moins propice pour
distinguer les deux causes.

À noter : `db/tests/harnais_porte.rs` n'est **pas** atteint par une déclaration
`#[cfg(test)] mod X;` de niveau module — il est déclaré `#[path =
"harnais_porte.rs"] mod harnais_porte;` **à l'intérieur** du `mod tests` inline
de `db.rs`. C'est la raison pour laquelle KTD3 suit les déclarations à
l'intérieur des régions masquées, en traçant la chaîne de modules inline, et
honore `#[path]`. Une résolution qui n'aurait regardé que le niveau module aurait
laissé ce fichier lu comme de la production.

---

## 9. Ce qui n'a pas été traité, et la conséquence de son maintien

| Population | Conséquence du maintien | Suivi |
|---|---|---|
| les 14 gardes qui ne séparent rien (§ 4) | faux positifs latents : elles lisent le `mod tests` des autres fichiers comme de la production. Bruyant si cela mord, jamais silencieux | ticket de suivi |
| les scanners shell (`scripts/check-*.sh`) | l'exclusion par chemin s'y périme en silence comme partout ailleurs | ticket de suivi |
| `check-secrets.sh` / `LARGE_FILE_ALLOWLIST` (`db.rs`) | l'exception survit au découpage et masque un futur dépassement du plafond | à retirer **par** mika#2321 |
| chaînes `mod` de profondeur > 1 hors `cfg(test)` | un fichier test-only atteint par une telle chaîne serait lu comme de la production | aucune aujourd'hui (profondeur max = 1) ; re-mesuré par test |
| écrire les gardes manquantes | — | hors périmètre déclaré du ticket |

---

## La leçon, en une phrase

**Une garde qui répond elle-même à une question qu'une autre garde pose déjà ne
diverge pas le jour où elle est écrite — elle diverge après, et dans la direction
qui ne se voit pas.** La réponse à « quelle part de ce fichier est de la
production » est maintenant écrite à un seul endroit, et une quatorzième copie
est refusée au moment où elle est écrite plutôt que découverte quand elle aura
dérivé.
