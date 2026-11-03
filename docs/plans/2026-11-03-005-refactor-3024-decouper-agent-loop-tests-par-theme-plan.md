# refactor(3024) — découper `agent_loop/tests/mod.rs` par thème

**Ticket :** senara-solutions/mika#3024
**Source :** mika#3023, hors vol déclaré par son plan § 8 et sa *Definition of Done* § 10.
**Précédent structurant :** mika#2321 (`db/tests/`).

---

## 1. Le défaut, mesuré

mika#3023 a sorti le `mod tests` inline d'`agent_loop/mod.rs` vers un module
répertoire `agent_loop/tests/` qui ne porte **qu'un fichier**.

| mesure, au HEAD de ce plan (`e4873c6af`) | valeur |
|---|---|
| `agent_loop/tests/mod.rs` | 372 321 octets, **9 855 lignes** |
| `#[test]` / `#[tokio::test]` dans ce fichier | **418** |
| marqueurs de section de premier niveau (`// ====`, `// ----`) | 20 |
| `agent_loop/mod.rs` | 642 482 octets — **406 094 sous** `LARGE_FILE_LIMIT=1048576` |

Deux choses se lisent sur ces nombres, et elles orientent tout le vol :

1. **Le motif est la lisibilité, pas une marge.** 372 Ko sont à 36 % du plafond
   du Secret Scan. Aucun vol de la boucle n'est bloqué par l'état actuel, et le
   ticket le dit. Ce vol n'achète donc **aucune marge** ; il achète un fichier
   qu'on peut ouvrir.
2. **Le fichier croît déjà.** mika#3023 l'a livré à 9 849 lignes ; il en porte
   9 855 deux merges plus tard (mika#3022, mika#3027). C'est une région
   append-only — un test y naît, n'en sort pas — exactement le profil que
   mika#2321 a invoqué pour `db`.

---

## 2. Ce que le ticket demande, et la contradiction qu'il porte

Le ticket énonce trois interdits (§ *Ce que le découpage ne doit pas faire*).
Le premier et le troisième sont tenables tels quels. **Le deuxième ne l'est
pas**, et un plan qui ne le dirait pas ferait découvrir la chose à
l'implémenteur au tour 40.

### 2a. « Ne pas changer un chemin de module » est inexécutable avec des sous-modules

Le ticket écrit : *« les tests sont atteints par `agent_loop::tests::<nom>`
quel que soit le fichier qui les porte »*. C'était **vrai pour mika#3023** —
un `mod tests` inline et un `mod tests;` sur **un seul** fichier `tests/mod.rs`
donnent le même chemin, et le corps de PR #3025 l'a vérifié. Ce n'est **pas**
vrai pour un découpage en `tests/<thème>.rs` : en Rust, un fichier déclaré par
`mod gardes_de_source;` **est** un module, et ses tests s'appellent
`agent_loop::tests::gardes_de_source::<nom>`.

Le précédent que le ticket invoque l'a payé : les tests de `db` s'appellent
aujourd'hui `db::tests::reapers::<nom>`. Le découpage et l'invariance du chemin
sont **mutuellement exclusifs**.

**Et la contrainte est portante**, pas cosmétique. Le `<nom>` ne change pas,
donc `cargo test -p mika-agent <nom>` continue de fonctionner ; mais le filtre
`cargo test` est une **sous-chaîne**, et `agent_loop::tests::mika2473` n'est
plus une sous-chaîne de `agent_loop::tests::gardes_de_source::mika2473…`. Une
citation de la forme longue rendrait alors `0 passed; N filtered out` **et
sortirait 0** — très exactement la panne que
`docs/solutions/best-practices/trois-facons-dont-une-verification-passe-au-vert-sans-rien-mesurer-2026-09-22.md`
documente, et dont l'un des exemples **est** `agent_loop::tests::mika2473_…`.
Laisser les citations telles quelles, c'est armer cette panne.

### 2b. L'option qui honorerait la lettre, et pourquoi elle est refusée

Une seule construction découpe le fichier **sans** insérer de segment :
`include!("gardes_de_source.rs")` dans `tests/mod.rs`. Le contenu est splicé
textuellement dans `agent_loop::tests`, donc :

- les 418 chemins restent **octet pour octet** ceux d'aujourd'hui ;
- il n'y a **aucune** question de visibilité entre tranches — un seul module,
  donc tous les helpers se voient, et l'arbitrage du § 3c disparaît ;
- aucune citation à réécrire.

**Elle est refusée pour une raison et une seule : `rustfmt` ne traverse pas
`include!`.** `cargo fmt` cesserait de formater les ~9 800 lignes concernées, et
`cargo fmt --check` **passerait au vert** sur un fichier qui dérive. C'est un
vert qui ne mesure plus rien, dans un dépôt qui refuse cette classe partout
ailleurs — et le coût serait payé par tous les vols futurs, pas par celui-ci.
Elle est de plus sans précédent ici, là où `mod` sur fichier en a un (`db`).

### 2c. La résolution

**Sous-modules réels, comme `db/tests/`** — l'instruction primaire du ticket —
**et les citations réécrites en nom nu**, qui est la forme stable :

| forme | survit à ce vol | survit au prochain re-découpage |
|---|---|---|
| `agent_loop::tests::mika2473_…` | non | non |
| `agent_loop::tests::gardes_de_source::mika2473_…` | oui | **non** |
| `cargo test -p mika-agent mika2473_…` (nom nu) | oui | **oui** |

Les citations partent donc en **nom nu**, jamais en chemin long : réécrire vers
la forme longue serait reconduire la fragilité d'un cran. Le détail des sites
est au § 6.

---

## 3. La partition

### 3a. Le principe : des tranches **contiguës**

Chaque fichier est **une ou deux plages de lignes consécutives** du fichier
actuel. C'est ce qui rend le vol mécanique, et cela pour deux raisons :

1. **L'extraction est une commande par fichier**, pas une couture ligne à ligne
   — donc un coût en tours constant par fichier, pas proportionnel au volume.
2. **La propriété d'un helper devient évidente** : il vit dans la tranche qui
   le suit. Le découpage non contigu obligerait à arbitrer, helper par helper,
   quel thème le possède — c'est ce coût que `db/tests/mod.rs` a évité en
   gardant *tous* les helpers à la racine, et que la contiguïté rend inutile.

Le fichier s'y prête : son ordre est déjà thématique (un ticket y a déposé ses
tests en bloc), et les 20 marqueurs de section sont des frontières d'item
propres — vérifié aux 5 frontières sondées (`1905`, `2646`, `3232`, `3487`,
`4485`) : chacune tombe sur un `#[test]`, un `fn` ou un commentaire de section.

### 3b. La table

**Les plages sont données au HEAD de ce plan et dériveront.** L'ancre
contraignante est le **nom du premier item** de la tranche ; l'implémenteur
redérive la ligne par `grep -n`. La coupe emporte les lignes d'attribut et de
doc-comment qui précèdent l'item.

| # | fichier `agent_loop/tests/…` | plages (HEAD) | ≈ lignes | thème | premier item |
|---|---|---|---|---|---|
| 1 | `gardes_de_source.rs` | 61–855, 8272–8989, 9480–9513 | 1547 | les scans `production_of` et les balayages de crate : mika#2879, #2342, #1910, #2247, #1883, #2473, #2305 (lecteur unique), #2425, #2321 | `FREQUENCY_PROMISE_EVENT_PREFIX` |
| 2 | `verdicts_et_dispositions.rs` | 856–1262 | 407 | mika#2338 (disposition attestée), #2794 (re-prompt correctif), #2296 (budget de raisonnement) | `arch_suffix_union` |
| 3 | `filtre_doutils_et_mode_de_boucle.rs` | 1263–1804 | 542 | mika#1324 (filtre structurel CI), propriétés de `LoopMode`, mika#2515 (chaque sortie décide du signal) | `tool_def` |
| 4 | `onboarding_et_amorce.rs` | 1805–1905 | 101 | `check_onboarding`, `seed_user_person` | `test_check_onboarding_true_when_no_core_memory` |
| 5 | `skills_outils_et_budgets.rs` | 1906–2646 | 741 | carte outil→skill, `max_skill_timeout`, mika#2276, visibilité d'outils, mika#2593, #2748, #1952 (contrat de preuve) | `make_skill_entry` |
| 6 | `injection_de_skills_et_historique.rs` | 2647–3232 | 586 | variantes de prompt d'`inject_skills`, `format_step_exceeded`, métadonnées de message, `strip_prior_images` | `test_inject_skills_skips_empty_snippets` |
| 7 | `callbacks_et_declencheurs.rs` | 3233–3487, 4486–5060 | 830 | `format_callback_framing`, garde de santé de tâche, surcharges LLM de skill, mika#2355, dispatch différé | `test_format_callback_framing_completed` |
| 8 | `outils_requis.rs` | 3488–4485 | 998 | `collect_required_tools`, `filter_available_required_tools`, mika#2755, #2363, #2757, #2814 (armement) | `test_collect_required_tools_empty_when_no_constraints` |
| 9 | `detection_dans_le_texte.rs` | 5061–5746 | 686 | appels d'outil en prose, revendication d'achèvement, clôture de jalon et son `argv` | `test_detect_text_based_tool_call_function_tag` |
| 10 | `classification_derreurs_et_reprise.rs` | 5747–6358 | 612 | entrée informationnelle, sortie persistable, erreurs terminales/réessayables, détection de revue de PR, intention de reprise, mika#3002 | `test_detect_informational_input_fyi` |
| 11 | `tours_webhook.rs` | 6359–7061 | 703 | ready-label, garde zéro-outil, mika#2517, #2667, dispatch non autorisé, compte du registre d'intentions | `ready_label_trigger_matches_canonical_marker` |
| 12 | `resume_et_usage_du_tour.rs` | 7062–7715 | 654 | résumé plafonné en mode silencieux, provider compact, `build_turn_usage`, mika#2331, #1671 | `seed_summary` |
| 13 | `fenetre_de_contexte.rs` | 7716–8271, 9514–9655 | 698 | mika#2295 (attribution et plafond d'octets), #2783 (recensement par outil), #2305 (champ), #2474 | `window_msg` |
| 14 | `perimetre_mcp.rs` | 8990–9479 | 490 | mika#2814 (helper de publication qui ensemence), #1959 (grades MCP) | `lecteur_declare` |
| 15 | `heritage_dequipe.rs` | 9656–9855 | 200 | mika#1926 (l'enfant hérite des stop-topics du parent conversationnel) | `team_turn_system_prompt` |

Somme ≈ 9 795 lignes ; le reste (1–60) est l'en-tête et les `use` de
`tests/mod.rs`. Quatre thèmes nommés par le ticket sont les fichiers **1, 11,
13, 14** ; les gardes de fin de tour sont **3** et le contrat de prompt **6**.

**Deux bornes sur la forme**, pour que la table ne dérive pas en liste :
aucun fichier au-dessus de **~1 600 lignes** (le plus gros de `db/tests/` en
porte 3 472, donc la borne est volontairement plus serrée que le précédent), et
**au plus 16 fichiers**. L'implémenteur peut fusionner deux tranches adjacentes
si une frontière se révèle malpropre ; il ne peut pas en ajouter.

### 3c. Les helpers partagés : **le compilateur est l'arbitre**

La règle de visibilité que mika#2321 a dégagée se relit ici en une phrase :
*deux frères ne voient pas les items privés l'un de l'autre, un enfant voit
ceux de son parent.* Conséquence opératoire :

- un helper utilisé par **une seule** tranche voyage avec elle — gratuit, il
  reste enfant d'`agent_loop` et ne demande aucune signature élargie ;
- un helper utilisé par **plusieurs** tranches doit remonter dans
  `tests/mod.rs`, où toutes le voient.

**Il n'y a rien à arbitrer à la main.** Un helper resté dans la mauvaise
tranche produit un `cannot find function …` dans la tranche qui l'appelle : le
compilateur **énumère** exhaustivement les helpers à promouvoir, et la liste est
close quand `cargo build --tests` est vert. Symétriquement, un helper promu que
personne n'appelle produit un `dead_code` — donc rien ne reste à la racine
« au cas où ».

L'exemple est déjà dans la table : `scope_reader_sites` (≈ 8302) vit dans la
tranche 1, et `mika2321_scope_reader_sites_keeps_production_and_drops_test_paths`
(≈ 9480) est son test. C'est pourquoi la plage `9480–9513` est rattachée au
fichier **1** et non au **14** : la tranche suit le helper, pas l'ordre du
fichier. C'est le seul rattachement non contigu décidé d'avance ; les autres,
s'il y en a, sortiront du compilateur.

**Aucune visibilité n'est élargie dans ce vol.** Si une promotion demandait un
`pub(crate)`, c'est le signe que la tranche a été mal coupée : recouper, ne pas
élargir.

---

## 4. Le geste

### 4a. Ordre des opérations

1. **Relever le compte de tests AVANT** : `cargo test -p mika-agent -- --list`,
   somme des binaires. Attendu `7209` / `8392` (corps de PR #3025) — mais
   **relevé, jamais posé de mémoire** : deux merges ont touché le fichier
   depuis.
2. **Sonder le primitif d'extraction** (§ 4b) — une commande, un tour. Le
   résultat décide de la suite.
3. **Pour chaque fichier de la table** : créer le fichier avec son en-tête
   `//!` + `use super::*;` (§ 4d), puis y verser ses plages.
4. **Tronquer `tests/mod.rs`** à son en-tête et ses `use`, et y ajouter les 15
   `mod <thème>;` par ordre alphabétique (forme de `db/tests/mod.rs`).
5. **`cargo fmt`**, puis `cargo build --tests -p mika-agent`.
6. **Promouvoir les helpers que le compilateur nomme** (§ 3c). Boucler 5–6
   jusqu'au vert.
7. **Relever le compte APRÈS** et le comparer à l'étape 1. Égalité stricte.
8. **Réécrire les citations** (§ 6).
9. `cargo clippy`, `cargo fmt --check`, `cargo test -p mika-agent`,
   `cargo test -p mika-common mika2398` (cohérence KTD3),
   `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`.

Un seul commit de travail : un état où `tests/mod.rs` est tronqué et une
tranche absente ne compile pas.

### 4b. Le primitif d'extraction, et le fait de politique qui le borne

Le plan de mika#3023 affirmait *« une redirection dont la cible est sous le
worktree est permise »*. **Cette affirmation est fausse dans la posture de
cette session de grooming.** Trois sondes, trois refus
(`no matching policy rule -- denied by default`) :

```
sed -n '1,3p' Cargo.toml       > .pilot-scratch/probe-out.txt    → REFUSÉ
head -3 Cargo.toml             > .pilot-scratch/probe-c.txt      → REFUSÉ
awk 'NR<=3{print > "…"}' Cargo.toml                              → REFUSÉ
git show HEAD:Cargo.toml       > .pilot-scratch/probe-a.toml      → PERMIS
```

Seule la forme `git show` l'est (règle `bash-git-show-redirect`, cpp#35), et
elle n'extrait qu'un fichier entier à une révision — pas une plage.

**Ce que l'implémenteur fait de ce fait.** La posture d'un pilote `dev-pilot`
peut être plus large que celle d'un `dev-groom` — mika#3023 a réellement
déplacé 9 886 lignes, donc *quelque* primitif lui était ouvert. L'étape 2
tranche en un tour :

```
sed -n '1,3p' Cargo.toml > .pilot-scratch/probe-sed.txt
```

- **Si elle passe** : l'extraction est une commande par fichier,
  `sed -n '61,855p;8272,8989p;9480,9513p' <src> >> <dst>` (plusieurs plages dans
  une seule expression, en `>>` pour conserver l'en-tête déjà écrit). 15
  commandes, coût en tours constant.
- **Si elle est refusée** : ne pas improviser, et surtout **ne pas** faire
  passer 9 800 lignes par le contexte via Read + Write — c'est la mort au
  plafond que mika#2636 existe pour empêcher. La voie de repli est un
  découpeur Rust jetable, `crates/mika-agent/examples/split_agent_loop_tests.rs`
  (~60 lignes : une table `(fichier, plages)`, `fs::read_to_string`,
  `fs::write`), lancé par `cargo run -p mika-agent --example
  split_agent_loop_tests`, puis **supprimé dans le même commit**. La table de
  plages qu'il porte est celle du § 3b ; elle est consignée dans le corps de PR
  pour que la revue voie la partition sans le fichier.

### 4c. L'interdiction héritée : aucune dé-indentation textuelle

mika#3023 a dû dé-indenter de 4 espaces, et a interdit `sed 's/^    //'` parce
que des littéraux multi-lignes portent leur indentation **comme contenu testé**
(exemple en arbre : `helper_first` ≈ `tests/mod.rs:9480-9490`). Une fixture
d'indentation altérée peut rester acceptée par le prédicat qu'elle exerce : le
test reste vert en ayant cessé de prouver.

**Ici le danger est éteint par construction** : le fichier est déjà au niveau
0, donc une tranche extraite n'a **aucune** indentation à corriger.
L'interdiction est reconduite quand même : aucune transformation `sed`/`s///`
sur le contenu, jamais. `sed` n'est utilisé qu'en **sélection** (`-n 'a,bp'`).
Vérification bon marché : `git diff --numstat` doit montrer, sur l'ensemble du
répertoire, **ajouté ≈ supprimé ≈ 9 800** — un écart signale une réécriture.

### 4d. Les en-têtes

**Chaque tranche** porte un `//!` de ~8 lignes, sur la forme de
`db/tests/reapers.rs` : le thème en une phrase, ce qui y est rangé, et le
rappel de visibilité (*« enfant de `agent_loop::tests`, donc descendant
d'`agent_loop` : les items privés d'`agent_loop` et les helpers promus restent
visibles via `use super::*` »*). Le corps est `use super::*;` et rien d'autre —
aucun `use` recopié : les imports de `tests/mod.rs` (lignes 56–60) traversent
les deux globs, comme dans `db/tests/`.

**`tests/mod.rs`** garde ses deux premières sections (*« Pourquoi un répertoire,
et pas un `agent_loop/tests.rs` »* — `perimeter::rules`/mika#2286 — et *« Ce que
la déclaration `mod tests;` sans visibilité achète »* — l'ancre
`"\n#[cfg(test)]\nmod tests"` de `source_scan::production_half`). Les deux
restent vraies et portantes : **ne pas les abréger.**

Sa troisième section, *« Pourquoi un seul fichier aujourd'hui »*, devient fausse
et est **remplacée** par : la partition et son principe de contiguïté (§ 3a), la
règle d'arbitrage des helpers (§ 3c), et le **chemin de module qui a gagné un
segment** avec la conséquence sur les filtres `cargo test` (§ 2a) — pour qu'un
futur éditeur n'ait pas à redécouvrir que la forme citable est le nom nu.

---

## 5. Ce que le découpage ne change pas, et pourquoi c'est prouvé

| ce qui pourrait casser | pourquoi non |
|---|---|
| les ~15 scans de la **moitié production d'`agent_loop/mod.rs`** | ils lisent `scanner.src_root().join("agent_loop/mod.rs")`, un chemin absolu dérivé de `CARGO_MANIFEST_DIR` — jamais `file!()`. Ce fichier n'est pas touché par ce vol. |
| les scans **à portée crate** (`for_each`, `for_each_under` : mika#2473, #2425, #1883) | `source_guard::test_only_modules` a une passe 2 à point fixe : *« and so is every file under a test-only module's directory »* (`source_guard.rs:717-718`). `tests/` est déjà test-only, donc chaque nouveau fichier l'est **par son répertoire**, et de surcroît par la résolution de `mod <thème>;` que la même passe fait dans un fichier déjà classé. Double couverture, **aucune garde à retoucher**. |
| un `use` externe de `agent_loop::tests::*` | il n'en existe aucun. Les occurrences sous `crates/` sont 3 commentaires ; les autres sont de la prose dans `docs/`. C'est la différence avec `db`, dont `skills::executor` importe six items — ici il n'y a **aucun contrat inter-modules** à préserver. |
| `perimeter::rules` → PR en DECISION-CORE | la classification Mechanical porte sur le **segment** `/tests/`, pas sur le nombre de fichiers. Inchangée. |
| `scripts/check-secrets.sh` | `LARGE_FILE_ALLOWLIST` **reste vide** et le script n'est pas touché. Le découpage réduit les tailles, il n'exempte rien. |
| le job `docs-sync` | la liste `DOCS` de `scripts/sync-agent-docs.sh` tient 9 fichiers nommés (`architecture.md`, `configuration.md`, …). **Aucun** des documents du § 6 n'y est. Rien à synchroniser. |
| le compte de tests | égalité stricte à l'étape 7 — c'est l'AC1. |

Le mode de panne que mika#3023 a trouvé (KTD3 perdant la **dernière** ligne du
fichier, donc un `mod tests;` en dernier item invisible, donc ~9 800 lignes de
test servies comme production à toutes les gardes **en silence**) est réparé.
L'étape 9 rejoue `cargo test -p mika-common mika2398` pour le constater plutôt
que le supposer : c'est la garde de cohérence qui était restée verte pendant la
panne, donc le contrôle à ne pas sauter.

---

## 6. Les citations à réécrire

Sept sites vivants citent un test par son chemin long. **Historique exclu** :
les occurrences sous `docs/plans/` sont des archives et ne se réécrivent pas.

| site | forme actuelle | forme cible |
|---|---|---|
| `docs/solutions/best-practices/trois-facons-…-2026-09-22.md:48,55` | `agent_loop::tests::mika2473_…`, `agent_loop::tests::mika2342` | nom nu |
| `docs/agent-internals/agent-loop.md:340` | `agent_loop::tests::mika2342_every_llm_call_is_wrapped_in_a_timeout` | nom nu |
| `docs/agent-internals/skills-system.md:107` | `agent_loop::tests::mika2363_escalation_still_has_a_declared_target_under_restriction` | nom nu |
| `docs/eval/mika-1910/README.md:159` | `agent_loop::tests::mika1910_every_unmeasured_site_declares_itself` | nom nu |
| `docs/operator/environment-variables.md:3600` | `agent_loop::tests::mika2305_the_scope_has_a_single_decisional_reader` | nom nu |
| `crates/mika-agent/src/canonical_tokens.rs:3062` | commentaire « plutôt que dans `agent_loop::tests` » | inchangé — désigne le module, pas un test |
| `crates/mika-agent/src/tool_execution/dispatch.rs:550` | `agent_loop::tests::mika3003_le_registre_…` | nom nu |
| `crates/mika-agent/src/evidence/guards.rs:6494` | `agent_loop::tests::mika2879_les_deux_sites_5e_emettent_la_nature` | nom nu |

Le site de `trois-facons-…` est le plus important des huit : c'est le document
qui **enseigne** la panne du filtre vide, et il la porterait.

**Contrôle positif, un tour, à faire après la réécriture** : lancer l'un des
noms nus (`cargo test -p mika-agent mika2473_the_freshness_check_sits_in_the_one_funnel`)
et vérifier `1 passed` — et non `0 passed; N filtered out`. Un essai d'absence
exige un contrôle positif sur son instrument.

---

## 7. Risques

| # | risque | signe | conduite |
|---|---|---|---|
| R1 | **le primitif d'extraction est refusé** (§ 4b) | `no matching policy rule` à l'étape 2 | basculer sur le découpeur Rust jetable. **Ne jamais** verser 9 800 lignes par Read+Write. |
| R2 | **une frontière coupe un item** | erreur de syntaxe au premier `cargo build` | redériver l'ancre par `grep -n`, emporter attributs et doc-comments. Le compilateur est immédiat et bruyant : ce risque n'est pas silencieux. |
| R3 | **helpers partagés plus nombreux que prévu** | plusieurs rondes 5–6 | la liste est close et énumérée par le compilateur (§ 3c). Si une ronde demandait un `pub(crate)`, **recouper** plutôt qu'élargir. |
| R4 | **le budget de tours se tend** | > ~60 tours de travail consommés | s'arrêter au **plancher** du § 9 : un découpage *partiel* compile et se merge seul (les tests non déplacés restent dans `tests/mod.rs`, qui est un état valide). Consigner le reste en commentaire sur mika#3024 et dans le corps de PR — **jamais** comme un ticket neuf (gel jusqu'au 01/12). |
| R5 | **`cargo fmt` réécrit plus que de l'indentation** | `git diff --numstat` montre ajouté ≫ supprimé | § 4c : s'arrêter et remonter. Ne pas « réparer » à la main. |
| R6 | **un test devient rouge** | `cargo test` rouge à l'étape 9 | aucune réponse par allowlist. Diagnostiquer le site contre la grille du § 5 ; si la cause est un scan qui voit désormais une fixture comme de la production, c'est une **vraie** régression de classification, à remonter. |

---

## 8. Hors vol, et la porte Fire-Disposition

**La porte Fire-Disposition est N/A, et ce n'est pas un oubli.** Ce vol ne livre
**aucun détecteur neuf** : il déplace 418 tests dont ~15 sont des scans
structurels **préexistants**, dont les populations et les prédicats sont
inchangés — un test déplacé n'est pas un test livré. C'est la disposition que le
plan de mika#3023 a prise sur le même terrain (« ce plan ne livre pas de test
asservissant la taille de `mod.rs` à un plafond : ce serait un détecteur »), et
elle a été groomée ainsi.

En particulier, **ce plan ne livre pas** :

- une garde plafonnant la taille d'un fichier d'`agent_loop/tests/` — ce serait
  un détecteur, et le ticket range explicitement le plafond hors motif ;
- une entrée dans `LARGE_FILE_ALLOWLIST`, qui reste vide ;
- une garde vérifiant que les citations en prose résolvent vers un test réel —
  utile, mais c'est un détecteur, et son périmètre est le dépôt entier, pas ce
  fichier. Si elle doit naître, elle naît ailleurs, après le 01/12.

Aucun ticket de suivi n'est ouvert (gel). Le reste éventuel part en commentaire
sur mika#3024.

---

## 9. Taille estimée

Les deux comptes sont séparés, comme pour mika#3023 : le Plan-Size Gate lit la
ligne `Total estimé`, et un déplacement qui y entrerait s'y lirait comme du code
écrit.

### Code écrit

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/agent_loop/tests/mod.rs` — en-tête `//!` réécrit (§ 4d) + 15 `mod <thème>;` | 70 |
| 15 × `crates/mika-agent/src/agent_loop/tests/<thème>.rs` — en-tête `//!` + `use super::*;`, ~10 lignes chacun | 150 |
| 3 commentaires de code citant un chemin long (`canonical_tokens.rs`, `tool_execution/dispatch.rs`, `evidence/guards.rs`) | 6 |

Total estimé : 226 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

### Code déplacé, sans modification

**≈ 9 795 lignes** réparties en 18 plages : les corps de 418 `#[test]` et leurs
helpers, versés tels quels. Aucune ligne n'est réécrite — et contrairement à
mika#3023, **aucune ré-indentation n'est même nécessaire** (§ 4c). Le diff
montrera ~9 800 ajoutées et ~9 800 supprimées ; c'est un déplacement.

### Pourquoi un seul vol

226 lignes écrites, soit **81 % de la borne par vol** de 279 et 23 % du seuil
par PR de 1 000. Le volume est en lignes **déplacées**, et le déplacement coûte
un nombre de tours **constant par fichier** (une commande d'extraction, un
en-tête), non proportionnel aux 9 800 lignes : 15 fichiers × ~2 tours ≈ 30
tours, plus les rondes de promotion du § 3c. Le précédent le mesure : mika#3023
a déplacé 9 886 lignes en un vol, et le volet B de mika#2321 (#2395) en a
déplacé ~22 000.

**Et le vol porte son plancher**, parce que 81 % n'est pas une marge
confortable (R4). Un découpage partiel est un état valide et mergeable seul :
tout test non déplacé reste dans `tests/mod.rs`. Le plancher est donc

> **fichiers 1, 11, 13, 14** — `gardes_de_source`, `tours_webhook`,
> `fenetre_de_contexte`, `perimetre_mcp` — soit les quatre thèmes que le ticket
> nomme, ≈ 3 440 lignes sorties de `tests/mod.rs`,

et la cible est les 15. Entre les deux, on s'arrête sur une frontière de
fichier, jamais au milieu d'une tranche, et on consigne le reste au § R4.

---

## 10. Definition of Done

- [ ] `crates/mika-agent/src/agent_loop/tests/` porte entre 5 et 16 fichiers en
      plus de `mod.rs`, chacun ≤ ~1 600 lignes, chacun avec son en-tête `//!`
      et `use super::*;`.
- [ ] `tests/mod.rs` ne contient plus **aucun** `#[test]` : seulement son
      en-tête, ses `use`, ses `mod <thème>;` et les helpers promus.
- [ ] Les deux sections portantes de l'en-tête de `tests/mod.rs` (répertoire vs
      `tests.rs` ; `mod tests;` sans visibilité) sont intactes ; la section
      « un seul fichier » est remplacée par la partition, la règle des helpers
      et la conséquence sur le chemin de module.
- [ ] `cargo test -p mika-agent -- --list` rend **le même compte** avant et
      après, et les deux comptes sont cités dans le corps de PR.
- [ ] Aucun `pub`/`pub(crate)` ajouté dans `agent_loop` ni dans `agent_loop/tests`.
- [ ] `scripts/check-secrets.sh` n'est pas touché ; `LARGE_FILE_ALLOWLIST` est vide.
- [ ] Les 7 citations vivantes du § 6 sont en nom nu, et le contrôle positif du
      § 6 est joué et cité.
- [ ] `cargo test -p mika-common mika2398` vert (cohérence KTD3).
- [ ] `cargo clippy`, `cargo fmt --check`, `cargo test -p mika-agent`,
      `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main` verts.
- [ ] Le corps de PR porte la table de partition effective (fichier → plages →
      nombre de tests), et nomme tout écart à la table du § 3b avec sa cause.
- [ ] `Code review: delegated to mika-qa (mika#2694)` dans le corps de PR ; pas
      d'invocation de `ce-simplify-code` ni de `ce-code-review`.
- [ ] Si le vol s'arrête au plancher : le reste est en commentaire sur
      mika#3024 **et** dans le corps de PR. Aucun ticket neuf.

---

## 11. Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de ses trois interdits et de son objectif.

- **AC1 — le contenu d'aucun test ne change.** `cargo test -p mika-agent --
  --list` rend un compte strictement identique avant et après (attendu `7209`
  lib / `8392` total, à relever et non à poser), et `cargo test -p mika-agent`
  est vert sans échec. Le diff du répertoire `agent_loop/tests/` montre
  ajouté ≈ supprimé, signature d'un déplacement et non d'une réécriture.
- **AC2 — `agent_loop/tests/mod.rs` ne porte plus de test.** Il se réduit à son
  en-tête, ses `use`, ses déclarations `mod <thème>;` et les helpers que
  plusieurs tranches partagent. Chaque fichier thématique pèse ≤ ~1 600 lignes.
- **AC3 — aucune visibilité n'est élargie.** Zéro `pub` ou `pub(crate)` ajouté
  dans `agent_loop` ou `agent_loop/tests` : la partition garde tous les tests
  enfants du même parent, ce qui est la propriété qui rend l'opération
  mécanique.
- **AC4 — le changement de chemin de module est assumé, pas subi.** Les 7
  citations vivantes du § 6 sont réécrites en **nom nu** (jamais en chemin
  long), le contrôle positif du § 6 est joué et son résultat cité, et l'en-tête
  de `tests/mod.rs` consigne que la forme citable est le nom nu.
- **AC5 — aucune garde n'est touchée, ni exemptée.** `scripts/check-secrets.sh`
  est hors diff et `LARGE_FILE_ALLOWLIST` est vide ; aucune allowlist, aucun
  recensement, aucune liste d'exemption du dépôt n'est modifié pour obtenir du
  vert ; `cargo test -p mika-common mika2398` est vert.
- **AC6 — la partition est auditable.** Le corps de PR porte la table effective
  (fichier → plages → nombre de tests) et nomme tout écart à la table du § 3b
  avec sa cause. Si un découpeur jetable a été utilisé (§ 4b), il ne survit pas
  au commit et sa table de plages est dans le corps de PR.
