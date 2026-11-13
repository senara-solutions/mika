# mika#2796 — un contrôle négatif porte sur les champs dérivés de la requête, jamais sur une chaîne qui contient un UUID

- **Ticket :** senara-solutions/mika#2796
- **Labels :** `bug`, `p3-nice-to-have`, `agent-core`, `ready`, `dispatch:loop`
- **Type :** `test`
- **Branche :** `test/2796/eval-le-contr-le-n-gatif-de-test`

---

## Contexte, et ce que la mesure dit exactement

`eval::test_operator_iterate_2506::le_nombre_qui_voyage_est_le_numero_d_issue`
tient **le** terme de conception de mika#2506 : le nombre qui voyage dans l'`input`
de dispatch est le numéro d'**issue**, jamais celui de la PR — parce que
`dispatch-lib.sh` lit `prompt: "<repo>#<N>"` comme un numéro d'issue
(`gh issue view "$ISSUE_NUM"`, puis `derive-branch-name --issue "$ISSUE_NUM"`).

Le test est juste. Son contrôle négatif ne l'est pas.

### La mesure

CI requis de la PR mika#2787, run 39160328822, 2026-10-18 :

```
le numéro de PR ne doit apparaître nulle part dans l'input de dispatch … :
{"iteration_context":"rebase puis corrige le lint SIGPIPE","prompt":"mika#2503",
 "skill":"dev-pilot","task_id":"fcc4cf43-9b62-4b0d-8281-06046a7f2504"}
```

Les quatre champs de cet `input` sont exactement ceux que le code de production
compose (`server/verdict_handler.rs:1084-1089`) :

```rust
let dispatch_input = serde_json::json!({
    "skill": target_skill,
    "prompt": format!("{repo_name}#{number}"),
    "task_id": task_id,
    "iteration_context": iteration_context,
});
```

`prompt` porte `mika#2503` — le numéro d'**issue**, ce que le test exige.
`iteration_context` porte le texte de l'opérateur, verbatim. **L'input était
correct.** Ce qui a rougi, c'est que le `task_id` — un UUID v4 frappé par
`db::create_task`, sans aucun rapport avec la requête — se terminait par
`…7f2504`, et que le contrôle négatif lit la chaîne **entière** :

```rust
let serialized = input.to_string();
assert!(!serialized.contains("2504"), …);          // tests/eval/test_operator_iterate_2506.rs:376-381
```

### Ce que ça coûte, et pourquoi ce n'est pas « un flake »

Un UUID v4 porte 32 chiffres hexadécimaux, dont ~16 décimaux en moyenne,
répartis en segments. La probabilité qu'une sous-chaîne décimale de 4 chiffres
donnée y apparaisse est de l'ordre de 1/700 par run. Sur un dépôt qui ouvre
plusieurs PR par jour et relance la CI à chaque push, c'est un rouge tous les
quelques jours — un rouge qui **ne dit rien de la PR qu'il bloque**, et qui coûte
une relance plus un tour d'agent pour établir qu'il ne dit rien.

Le défaut n'est pas l'aléa, il est dans le prédicat : il s'applique à une chaîne
dont une partie **ne peut pas**, par construction, transporter le numéro de PR, et
dont la valeur est déjà vérifiée **exactement** trois lignes plus haut
(`assert_eq!(input["task_id"].as_str(), Some(task_id.as_str()))`, ligne 367).

### Trois faits relevés au grooming, qui resserrent le périmètre

1. **La population est de un.** `grep -rn '!.*to_string().contains'` sur
   `crates/` ne rend qu'un seul contrôle négatif portant sur un blob sérialisé
   qui contient un identifiant aléatoire : celui-ci. L'autre forme voisine,
   `skills/manifest.rs:870` (`!serialized.contains("[llm]")`), porte sur un
   littéral à crochets dans un manifeste déterministe — aucun identifiant
   aléatoire, aucun chiffre.
2. **Les contrôles *positifs* voisins sur sous-chaîne décimale sont hors
   risque, et c'est vérifié, pas supposé.** `ready_label_handler.rs:3016`
   (`digest.contains("2055")`), `ci_success_handler.rs:1266`,
   `merge_gate_rerun.rs:799` : tous portent sur un digest **déterministe**
   construit sur place, sans UUID. Le risque dual — un contrôle positif
   satisfait par hasard, donc *vert* à tort — n'a pas de sujet dans ce dépôt
   aujourd'hui. Il n'est donc pas traité ici (voir § *Hors périmètre*).
3. **Le `task_id` n'est pas injectable, et la variante que le ticket suggère est
   hors d'atteinte sans toucher la production.** `NewTask` ne porte pas de champ
   d'identifiant ; `Database::create_task` frappe l'UUID en interne. « Fixer dans
   le test un `task_id` qui ne contient pas de chiffres ambigus » demanderait un
   siège d'injection dans le chemin de production pour réparer une assertion de
   test. Refusé — c'est le prédicat qui est faux, pas l'UUID.

---

## La direction retenue

Un seul geste, en trois pièces, toutes dans
`crates/mika-agent/tests/eval/test_operator_iterate_2506.rs`. Aucune ligne de
production n'est touchée.

### Pièce 1 — le prédicat devient une fonction, et il porte sur les champs

Le contrôle négatif cesse d'être une ligne d'`assert!` noyée dans un test et
devient une fonction locale au fichier de test, avec une liste d'exclusion
nommée :

```rust
/// Les clés de l'`input` de dispatch dont la valeur est **frappée par le moteur**
/// et ne dérive en rien de la requête de l'opérateur. mika#2796 : exclues du
/// contrôle négatif, parce qu'un UUID v4 porte ~16 chiffres décimaux et qu'une
/// sous-chaîne de 4 chiffres donnée y tombe environ une fois sur 700.
const CLES_NON_DERIVEES_DE_LA_REQUETE: [&str; 1] = ["task_id"];

/// Rend, sous la forme `clé=valeur`, les champs de l'`input` de dispatch dont la
/// valeur porte `nombre` — hors clés non dérivées de la requête. Vide = aucune
/// fuite.
fn champs_portant_le_nombre(input: &serde_json::Value, nombre: &str) -> Vec<String>
```

Corps : `as_object()` (panique si la forme change), assertion `contains_key` sur
chaque clé exclue (l'exclusion ne peut pas devenir muette en silence), puis
`to_string().contains(nombre)` sur **chaque sous-arbre** des clés restantes.

Trois propriétés, chacune porteuse :

- **Le balayage porte sur les sous-arbres, pas sur les scalaires.** L'`input` est
  plat aujourd'hui (quatre champs scalaires), mais un champ qui deviendrait un
  objet reste couvert sans qu'on y pense : c'est le `to_string()` du sous-arbre
  qui est lu. C'est la lecture du « tout champ libre » du ticket.
- **Le défaut est d'inclure.** La liste est une **exclusion**, pas une liste de
  champs à regarder : un cinquième champ ajouté au `json!` de production entre
  dans le contrôle le jour où il est écrit, sans qu'il faille s'en souvenir.
  Une liste d'inclusion (`prompt` + `iteration_context`) aurait la forme que
  mika#2309 condamne — un contrôle qui cesse de couvrir ce qu'il devait couvrir,
  sans rougir.
- **Le message nomme le champ coupable**, pas seulement le blob : `porteurs` est
  rendu tel quel dans le message d'échec, l'`input` complet en annexe. Un rouge
  futur dira *lequel* des champs transporte le nombre.

### Pièce 2 — le test porteur appelle le prédicat, et sa doc dit la limite

Lignes 375-381 remplacées par :

```rust
// Contrôle négatif : aucun champ DÉRIVÉ DE LA REQUÊTE ne porte le numéro de PR.
// Le `task_id` est exclu (mika#2796) — son UUID n'a aucun rapport avec la
// requête, et sa valeur est vérifiée exactement ligne 367.
let porteurs = champs_portant_le_nombre(&input, "2504");
assert!(
    porteurs.is_empty(),
    "le numéro de PR ne doit apparaître dans aucun champ dérivé de la requête \
     — `dispatch-lib` le lirait comme un numéro d'issue : {porteurs:?} \
     (input complet : {input})"
);
```

Le doc-comment du test (lignes 324-332) dit aujourd'hui « ne doit apparaître
**nulle part** dans l'`input` ». Amendé : « dans aucun champ **dérivé de la
requête** », avec la raison en une phrase et le renvoi à mika#2796. Une doc qui
promet plus que l'assertion est ce qui fait qu'on « répare » l'assertion dans le
mauvais sens à la lecture suivante.

### Pièce 3 — trois tests qui mesurent le prédicat lui-même

Le prédicat est un détecteur : son chemin de succès est « aucune violation
trouvée ». Il lui faut donc ses propres mesures, et elles sont synchrones
(`#[test]`, ni base ni tokio — le sujet est une `serde_json::Value`).

| test | ce qu'il établit |
|---|---|
| `mika2796_un_task_id_portant_les_chiffres_du_numero_de_pr_ne_declenche_rien` | **Le rouge-avant du ticket, figé.** L'`input` exact du run 39160328822, UUID `fcc4cf43-9b62-4b0d-8281-06046a7f2504` verbatim ⇒ `porteurs` vide. Sous le prédicat d'avant (`!input.to_string().contains("2504")`) ce test échoue : c'est le flake, rendu déterministe et nommé. |
| `mika2796_le_controle_rougit_quand_le_nombre_voyage_dans_un_champ_libre` | **Le contrôle reste capable de rougir** (exigence du ticket). Deux cas sur la forme de production : `iteration_context` semé de `2504` ⇒ `["iteration_context=…"]` ; `prompt` mis à `mika#2504` ⇒ `["prompt=…"]`. L'égalité est sur la liste **entière**, pas sur sa non-vacuité : un prédicat qui rendrait tous les champs rougirait aussi « correctement » sans rien discriminer. |
| `mika2796_lexclusion_du_task_id_rougit_si_la_cle_disparait` | **L'assertion auto-nettoyante.** `#[should_panic]` sur un `input` privé de `task_id`. Si la clé est renommée en production, l'exclusion devient muette, le contrôle re-balaie l'UUID et le flake revient ; ce test fait rougir le renommage à la place. |

### Ce que la direction ferme, et ce qu'elle ne ferme pas

Ferme : le flake, sa cause, et la régression par renommage. Le terme de
conception de mika#2506 reste mesuré, sur un périmètre **plus franc** qu'avant
(les champs nommés, pas une chaîne où la couverture était accidentelle).

Ne ferme pas : la famille des contrôles **positifs** sur sous-chaîne décimale
(fait 2 ci-dessus). Leur risque est de signe opposé — vert à tort — et sans sujet
mesurable aujourd'hui dans ce dépôt. L'étendre ici serait un balayage structurel
sur `crates/**` pour une population vide : exactement la garde que mika#2578 et
mika#2752 condamnent.

---

## Hors périmètre, et pourquoi

1. **Un siège d'injection du `task_id` dans la production.** Fait 3 : la variante
   « fixer un `task_id` sans chiffres ambigus » demande un champ d'identifiant
   dans `NewTask` ou un paramètre à `create_task`, c'est-à-dire élargir une
   surface de production pour réparer une assertion de test. Le défaut est dans
   le prédicat ; c'est le prédicat qui change.
2. **Un lint CI « pas de `contains` sur un blob portant un UUID ».** Population
   de un (fait 1). Une règle de lint pour un seul site est une garde qui ne
   refuse rien et qu'on paie à chaque run ; la doctrine de ce dépôt est de
   documenter le motif et d'armer le lint quand la population le justifie. D'où
   le § suivant.
3. **Les contrôles positifs sur sous-chaîne décimale.** Fait 2 — vérifiés
   déterministes, pas de sujet. Pas de ticket de suivi ouvert : il n'y a rien à
   suivre tant qu'aucun digest n'embarque d'identifiant aléatoire.
4. **Toute modification de `server/verdict_handler.rs` ou de
   `server/iterate_dispatch.rs`.** Le code de production est correct — c'est la
   conclusion de la mesure, pas une hypothèse.

---

## Livrables et ordre de commit

Un seul commit de travail, puis le commit de doc.

**Commit 1 — `test(eval): le contrôle négatif de mika#2506 porte sur les champs dérivés de la requête (mika#2796)`**

1. `crates/mika-agent/tests/eval/test_operator_iterate_2506.rs`
   - `CLES_NON_DERIVEES_DE_LA_REQUETE` + `champs_portant_le_nombre`, posés avec
     les autres helpers du fichier (après `run_nominal`, avant la bannière
     `V5`) ;
   - lignes 375-381 remplacées par l'appel au prédicat ;
   - doc-comment du test porteur amendé (« dérivé de la requête ») ;
   - bloc `# Rouge-avant` de l'en-tête du module (lignes 16-30) : une recette
     d'injection de plus — *remettre `input.to_string().contains("2504")` ⇒
     `mika2796_un_task_id_portant_les_chiffres_du_numero_de_pr_ne_declenche_rien`
     rougit, et elle seule* ;
   - les trois `#[test]` de la pièce 3, sous une bannière
     `// mika#2796 — le prédicat du contrôle négatif, mesuré`.

**Commit 2 — `docs(solutions): un contrôle négatif exclut les identifiants aléatoires (mika#2796)`**

2. `docs/solutions/best-practices/un-controle-negatif-exclut-les-identifiants-aleatoires-2026-10-19.md`
   — frontmatter YAML (`module: mika-agent`, `tags: [tests, flake, negative-control]`,
   `problem_type: flaky_test`), la mesure du run 39160328822, le calcul de
   probabilité, la règle (*un contrôle négatif porte sur les champs dérivés de
   l'entrée, jamais sur un blob qui contient un identifiant frappé par le
   moteur*), la forme retenue (exclusion nommée + assertion auto-nettoyante), et
   la raison pour laquelle ce n'est pas un lint. C'est le document qu'on cite le
   jour où la population passe de un à trois.

Pas de `CLAUDE.md` touché : le contrat du test ne change pas, seule sa précision
d'assertion le fait. Pas de `scripts/sync-agent-docs.sh` : rien sous
`crates/mika-agent/docs/`.

---

## Verification Contract

| # | geste | attendu |
|---|---|---|
| V1 | `cargo test -p mika-agent --test eval test_operator_iterate_2506` | 14 tests verts (11 existants + 3 nouveaux) |
| V2 | **Rouge-avant du flake** : dans `champs_portant_le_nombre`, retirer le filtre d'exclusion (balayer *toutes* les clés) | `mika2796_un_task_id_portant_les_chiffres_du_numero_de_pr_ne_declenche_rien` rougit, **et elle seule** |
| V3 | **Rouge-avant de la capacité à rougir** : faire rendre `Vec::new()` inconditionnellement par `champs_portant_le_nombre` | `mika2796_le_controle_rougit_quand_le_nombre_voyage_dans_un_champ_libre` rougit, **et elle seule** |
| V4 | **Rouge-avant de l'auto-nettoyage** : retirer l'assertion `contains_key` | `mika2796_lexclusion_du_task_id_rougit_si_la_cle_disparait` rougit, **et elle seule** |
| V5 | **Le terme de mika#2506 reste mesuré** : dans `try_engine_dispatch_for`, passer le numéro de PR au lieu de `req.issue` | `le_nombre_qui_voyage_est_le_numero_d_issue` rougit, en nommant `prompt=` dans son message |
| V6 | `cargo test -p mika-agent --test eval` | suite eval entière verte |
| V7 | `cargo clippy --all-targets -- -D warnings` puis `cargo fmt --check` | propre |
| V8 | `scripts/verify-pipeline.sh origin/main` | vert (U2 : `## Acceptance criteria` présent dans ce plan) |

V2 à V5 sont des injections manuelles, à défaire avant le commit. V5 est le
contrôle qui compte le plus : il atteste que la réparation du prédicat n'a pas
coûté sa portée au test.

---

## Risques

| risque | probabilité | mitigation |
|---|---|---|
| L'exclusion de `task_id` masque une vraie fuite — le moteur écrirait le numéro de PR *dans* le `task_id` | nulle par construction | `task_id` est un UUID v4 frappé par `create_task`, et sa valeur est vérifiée **exactement** ligne 367 (`assert_eq!` contre le `task_id` rendu par l'outcome). Le champ exclu du balayage est le seul dont la valeur est pinnée au byte. |
| Un cinquième champ de production échappe au contrôle | faible | Le prédicat est une **exclusion** : tout champ non nommé est balayé dès son écriture. C'est la pièce 1, propriété 2. |
| Le renommage de `task_id` rend l'exclusion muette et le flake revient | faible | `contains_key` + `mika2796_lexclusion_du_task_id_rougit_si_la_cle_disparait`. |
| La réparation réduit la portée du test porteur sans qu'on le voie | faible | V5 : le rouge-avant de mika#2506 est rejoué, et le message doit nommer `prompt=`. |
| On « répare » le prédicat en sens inverse à la lecture suivante, sur la foi du doc-comment | moyenne (c'est exactement comment ce défaut naît) | Le doc-comment du test est amendé dans le même commit, et le bloc `# Rouge-avant` du module porte la recette d'injection. |

---

## Definition of Done

- `champs_portant_le_nombre` et `CLES_NON_DERIVEES_DE_LA_REQUETE` existent dans
  `tests/eval/test_operator_iterate_2506.rs`, le second avec la raison de
  l'exclusion et la référence mika#2796 dans son doc-comment.
- `le_nombre_qui_voyage_est_le_numero_d_issue` n'appelle plus
  `input.to_string().contains(…)` ; son doc-comment dit « dérivé de la requête ».
- Les trois `#[test]` de la pièce 3 existent et sont verts.
- Le bloc `# Rouge-avant` du module porte la recette d'injection mika#2796.
- V1 à V8 verts, injections V2–V5 vérifiées puis défaites.
- Le document de `docs/solutions/best-practices/` est écrit.
- Aucun fichier sous `crates/mika-agent/src/` modifié.

---

## Acceptance criteria

Le corps de mika#2796 ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de son § *Travail attendu*, point par point.

- **AC1** — Le contrôle négatif de
  `le_nombre_qui_voyage_est_le_numero_d_issue` ne porte plus sur l'`input`
  sérialisé entier : il porte sur les champs susceptibles de transporter le
  numéro de PR, le `task_id` exclu. Attesté par V1 et par l'absence de
  `input.to_string().contains` dans le fichier.
- **AC2** — Le flake est figé en test déterministe : l'`input` exact du run
  39160328822, UUID verbatim, ne déclenche pas le contrôle. Attesté par
  `mika2796_un_task_id_portant_les_chiffres_du_numero_de_pr_ne_declenche_rien`
  et par le rouge-avant V2.
- **AC3** — Le contrôle reste capable de rougir : un test frère sème le numéro de
  PR dans `iteration_context` et exige que le contrôle le trouve — et nomme le
  champ. Attesté par
  `mika2796_le_controle_rougit_quand_le_nombre_voyage_dans_un_champ_libre` et par
  le rouge-avant V3.
- **AC4** — L'exclusion du `task_id` ne peut pas devenir muette en silence : sa
  disparition ou son renommage fait rougir un test. Attesté par
  `mika2796_lexclusion_du_task_id_rougit_si_la_cle_disparait` et par V4.
- **AC5** — Le terme de conception de mika#2506 reste mesuré à portée inchangée :
  faire voyager le numéro de PR dans `prompt` fait toujours rougir le test
  porteur. Attesté par V5.
- **AC6** — Aucune ligne de production n'est modifiée. La variante « fixer un
  `task_id` non ambigu » du ticket est refusée **et nommée** dans le plan, avec
  sa raison (pas de siège d'injection sans élargir `NewTask`).
- **AC7** — Le motif est documenté dans `docs/solutions/best-practices/` avec la
  mesure, la règle et la raison pour laquelle ce n'est pas un lint.

---

## Fire-Disposition

Ce plan livre un détecteur : `champs_portant_le_nombre`, dont le chemin de succès
est « aucune violation trouvée ». Option retenue : **(a) exception nommée en
allowlist**, et il y a exactement **une** exception.

**L'entrée.** `const CLES_NON_DERIVEES_DE_LA_REQUETE: [&str; 1] = ["task_id"]` —
grep-visible (`grep -n CLES_NON_DERIVEES crates/mika-agent/tests/eval/`), elle
nomme la **donnée précise** (la clé `task_id` de l'`input` de dispatch, et non
« les UUID » en général), et son doc-comment porte la référence mika#2796 avec la
raison mesurée : un UUID v4 porte ~16 chiffres décimaux, une sous-chaîne de 4
chiffres donnée y tombe ~1 fois sur 700.

**L'assertion auto-nettoyante.** `champs_portant_le_nombre` asserte
`obj.contains_key("task_id")` avant de filtrer. L'exception ne devient stale que
d'une seule façon : si la clé est **renommée** ou **retirée** du `json!` de
production, l'exclusion ne désigne plus rien et le balayage reviendrait sur
l'UUID — ce que l'assertion refuse en paniquant, avec un message qui dit quoi
faire. `mika2796_lexclusion_du_task_id_rougit_si_la_cle_disparait`
(`#[should_panic]`) pinne ce comportement, et V4 atteste qu'il rougit pour la
bonne raison.

**Pourquoi l'exception est permanente et pourquoi ce n'est pas un report.** La
valeur de `task_id` est un identifiant aléatoire frappé par le moteur : elle ne
dérivera jamais de la requête, quel que soit l'avenir du code. L'exception n'est
donc pas une violation qu'on remet à plus tard, c'est une propriété de la donnée
— et le champ exclu du **balayage** est le seul du quadruplet dont la valeur est
pinnée **exactement** par ailleurs (ligne 367), donc l'exclusion ne peut
dissimuler aucune fuite. Il n'y a rien à armer plus tard, donc aucun ticket de
suivi n'est ouvert : mika#2796 est la référence de l'entrée, et la fermeture de
mika#2796 est l'état final.

**Aucune autre violation n'est tolérée.** Le détecteur atterrit **armé**, sur le
chemin nominal, sans `#[ignore]`, et vert sur `main` du premier coup — parce que
le code de production composait déjà le bon numéro. Les trois autres champs
(`skill`, `prompt`, `iteration_context`) sont balayés, et tout cinquième champ le
sera dès son écriture.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `tests/eval/test_operator_iterate_2506.rs` — const + prédicat (doc comprise) | 38 |
| `tests/eval/test_operator_iterate_2506.rs` — contrôle du test porteur, net | 5 |
| `tests/eval/test_operator_iterate_2506.rs` — 3 `#[test]` (doc comprise) | 78 |
| `tests/eval/test_operator_iterate_2506.rs` — doc du test porteur + bloc `# Rouge-avant` | 14 |

Total estimé : 135 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 135 lignes contre une borne de 279 :
la moitié de la marge. Le périmètre est d'**un seul fichier**, de test, qui
compile déjà, dans une suite dont la commande de run est connue
(`cargo test -p mika-agent --test eval test_operator_iterate_2506`, quelques
secondes). Aucune ligne de production n'est touchée, donc aucun chemin de
migration, aucune sonde à déployer, aucun `make deploy`. Les quatre rouge-avant
(V2–V5) sont des injections d'une ligne chacune dans le fichier qu'on vient
d'écrire — c'est le poste de tours le plus lourd du vol, et il est borné à quatre
cycles edit-test-revert.
