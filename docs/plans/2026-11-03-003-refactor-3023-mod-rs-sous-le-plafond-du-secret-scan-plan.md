# mika#3023 — `agent_loop/mod.rs` repasse sous le plafond du Secret Scan, par extraction de son `mod tests`

**Ticket :** senara-solutions/mika#3023
**Labels :** `p2-normal`, `ready`, `dispatch:loop`, `loop-substrate`
**Base :** `origin/main` 3693614d

---

## 1. Le défaut, re-mesuré sur la base

Mesures prises sur la tête du worktree (= `origin/main` 3693614d) :

| grandeur | valeur |
|---|---|
| `crates/mika-agent/src/agent_loop/mod.rs` | 22 446 lignes, **1 046 935 octets** |
| plafond `LARGE_FILE_LIMIT` (`scripts/check-secrets.sh:9`) | 1 048 576 |
| marge restante | **1 641 octets** |
| `LARGE_FILE_ALLOWLIST` (`scripts/check-secrets.sh:42-44`) | **vide** |
| bloc `#[cfg(test)] mod tests` (l. 12 561 → 22 446) | 9 886 lignes, **404 539 octets** (38,6 %) |
| moitié production (l. 1 → 12 560) | 12 560 lignes, **642 396 octets** |
| `#[test]` / `#[tokio::test]` au premier niveau du bloc | 418 |

Les trois mesures du ticket sont confirmées à l'octet. Le défaut n'est pas une
hypothèse : **le prochain vol qui ajoute plus de 1 641 octets à ce fichier sort
une PR rouge sur le seul check requis**, quel que soit son contenu —
c'est arrivé deux fois (#3009, #3010), à 201 tours et ~23 $ par pilote, et la
remise en route a demandé une main humaine.

Un point de mécanique qui borne le vol et qu'il faut avoir lu avant de choisir
la structure : **les deux détecteurs de `check-secrets.sh` sont diff-scopés**
(commentaire l. 39-41 — « only files the PR itself adds/modifies are scanned »).
La PR de ce vol présente donc exactement deux fichiers au scan : `mod.rs`
rétréci à ~642 Ko, et le nouveau fichier de tests à ~405 Ko. Les deux passent,
et aucune entrée d'allowlist n'est requise — ce qui est la forme littérale de
l'AC1.

---

## 2. La décision : un **répertoire** `agent_loop/tests/`, un seul fichier à la naissance

### 2a. Pourquoi un répertoire et pas un `agent_loop/tests.rs`

Ce n'est pas une préférence de style, c'est la raison 1 du précédent mika#2321,
et elle tient à l'identique ici. `perimeter::rules` classe :

- **Mechanical** tout chemin portant un segment `/tests/` (`rules.rs:177`) ;
- **DecisionCore** un `some_module/tests.rs` isolé — par choix fail-closed
  explicite et commenté (`perimeter/tests.rs:175-180` : « the file itself lives
  next to production code and might be accidentally cfg-flipped »).

Or une PR DECISION-CORE n'est pas dé-draftée automatiquement (mika#2286). Un
fichier `agent_loop/tests.rs` ferait donc de **chaque PR future touchant un
test de la boucle** un draft en attente d'un geste humain de vérification —
c'est-à-dire exactement le coût que ce ticket existe pour retirer, déplacé d'un
check rouge vers un draft. Le répertoire l'évite par construction.

### 2b. Pourquoi un seul fichier à la naissance, et non un découpage thématique

Le précédent mika#2321 a réparti `db` par thème pour sa raison 3 (croissance
d'une région append-only). Cette raison est réelle mais **n'est pas l'urgence
de ce ticket**, et les deux grandeurs ne sont pas du même ordre :

| | marge avant le plafond |
|---|---|
| `agent_loop/mod.rs` aujourd'hui | **1 641 octets** |
| `agent_loop/tests/mod.rs` à sa naissance | **644 037 octets** |

Au débit mesuré par le ticket (4 PR de la boucle sur ce fichier depuis le
2026-11-01, ~3 Ko par PR), 644 Ko représentent plus de 200 PR. Le découpage
thématique, lui, est un arbitrage de **418 `#[test]`** et de leurs helpers
partagés — le volume qui tue un vol — et le bloc n'offre que 32 marqueurs de
section de premier niveau, dont un intervalle de 6 136 lignes sans aucune
frontière (l. 13 776 → 19 912). L'arbitrage y est à faire, pas à lire.

Deux propriétés rendent le report **gratuit** plutôt que reporté-avec-dette :

1. **La classification est déjà transitive.** `source_guard::test_only_modules`
   a une passe 2 à point fixe (`source_guard.rs:717-730`) : dans un fichier
   déjà classé test-only, toute déclaration de module l'est aussi, « and so is
   **every file under a test-only module's directory** ». Dès que `mod.rs`
   déclare `#[cfg(test)] mod tests;`, le répertoire entier est test-only pour
   toujours : un découpage ultérieur ne demande **aucune** retouche de garde.
2. **Aucun nom de test ne change dans ce vol.** Un `mod tests` inline et un
   `mod tests;` sur fichier donnent le même chemin de module —
   `agent_loop::tests::<nom>`. Les 8 citations de `agent_loop::tests::…` qui
   existent sous `crates/` (toutes en commentaire : `canonical_tokens.rs:3062`,
   `tool_execution/dispatch.rs:550`, `evidence/guards.rs:6494`,
   `skills/manifest.rs:238`, et 4 sous `tests/eval/`) restent exactes. Un
   découpage thématique, lui, les périmerait toutes les 8 et renommerait 418
   chemins — raison de plus pour ne pas le mélanger à ce vol.

**Suite renvoyée.** Le découpage thématique est **hors périmètre** de ce vol
(§ 8), pas un second vol dont celui-ci dépendrait : les 108 lignes écrites du
§ 9 passent très au-dessous de la borne par vol, donc il n'y a pas de découpage
en vols à opérer ici. L'implémentation ouvre un ticket de suivi — « répartir
`agent_loop/tests/` par thème » — et en cite le numéro sur une ligne
`Tracked in:` du corps de PR, puisque `pr-body-validation.yml` exige un
référent réel pour toute phrase de report. Un numéro n'est pas inventé ici : le
ticket n'existe pas encore au moment du grooming.

### 2c. La forme exacte de la déclaration

```rust
#[cfg(test)]
mod tests;
```

Sans visibilité, et ce n'est pas un détail. Deux raisons :

- **Personne n'importe `agent_loop::tests`.** Vérifié : les 8 occurrences sous
  `crates/` sont des mentions en prose. C'est la différence avec `db`, dont
  `tests/mod.rs` doit documenter que `skills::executor` importe
  `crate::db::tests::*` et que ce chemin « doit survivre octet pour octet ».
  Ici il n'y a aucun contrat inter-modules à préserver, donc `pub(crate)` serait
  une extension de visibilité sans lecteur.
- **Cette orthographe garde l'ancre de `source_scan::production_half`
  satisfaite.** L'ancre est le littéral `"\n#[cfg(test)]\nmod tests"`
  (`source_scan.rs:195`) ; `pub(crate) mod tests;` ne la matche pas — c'est le
  cas de `db.rs:6855-6856`, bénin là-bas puisque le fichier n'a plus rien à
  couper. Garder l'ancre vivante ici laisse la coupure se comporter après le
  déplacement comme elle se comportait avant, ce qui est une invariance de
  moins à vérifier.

---

## 3. Le geste, et l'interdiction qui le borne

### 3a. Ordre des opérations

1. **Relever le compte de tests AVANT** (AC2) :
   `cargo test -p mika-agent -- --list` sur la tête intacte. Noter le nombre.
2. **Créer `crates/mika-agent/src/agent_loop/tests/mod.rs`** avec son en-tête
   `//!` (§ 3c), puis y verser le **corps** du module — lignes 12 563 à 22 445
   du `mod.rs` actuel, soit l'intérieur des accolades, sans le `#[cfg(test)]`
   ni le `mod tests {` ni son `}` de fermeture.
   Extraction par `sed -n '12563,22445p'` redirigé dans le fichier cible (une
   redirection dont la cible est sous le worktree est permise).
3. **Tronquer `mod.rs`** aux lignes 1 à 12 560, puis y ajouter la déclaration
   du § 2c.
4. **`cargo fmt`** — et c'est ici que la dé-indentation se fait (§ 3b).
5. **`cargo build` puis `cargo test -p mika-agent`.** Traiter chaque rouge
   selon la grille du § 4.
6. **Relever le compte de tests APRÈS** et le comparer au compte de l'étape 1.
7. `cargo clippy`, `cargo fmt --check`, `cargo test -p mika-common mika2398`
   (AC4), `scripts/verify-pipeline.sh origin/main`.

Un seul commit de travail est attendu : le déplacement est atomique — un état
intermédiaire où `mod.rs` est tronqué mais le fichier de tests absent ne
compile pas.

### 3b. L'interdiction : **aucune dé-indentation textuelle**

Le corps extrait porte 4 espaces d'indentation de trop à chaque ligne. La
tentation est `sed 's/^    //'`. **Elle est interdite, et c'est le seul mode de
panne silencieux de ce vol.**

Le bloc contient des littéraux de chaîne multi-lignes dont l'indentation est
**le contenu testé**. Exemple en arbre, `mod.rs:21322-21336` :

```rust
        let helper_first = "\
#[cfg(test)]
fn helper() -> u32 {
    1
}
";
```

Un `sed 's/^    //'` transformerait `    1` en `1` à l'intérieur du littéral.
La fixture change, et rien ne garantit que le test qui la lit rougisse : une
fixture d'indentation altérée peut rester acceptée par le prédicat qu'elle
exerce, et la garde continue alors de passer en ayant cessé de prouver ce
qu'elle prouvait. C'est précisément la classe que ce dépôt refuse ailleurs — un
vert qui ne mesure plus rien.

**La dé-indentation est faite par `cargo fmt`**, qui travaille sur la grammaire
et ne modifie jamais le contenu d'un littéral de chaîne. C'est la seule forme
du geste qui soit sûre par construction, et non sûre par vérification.

Vérification associée, bon marché : après `cargo fmt`, `git diff --numstat` sur
le fichier de tests doit montrer un diff de reformatage (lignes ajoutées ≈
lignes supprimées ≈ 9 886) et `cargo test -p mika-agent` doit être vert avec le
compte de l'étape 1. Si `cargo fmt` échoue ou boucle sur un fichier de 405 Ko,
**ne pas basculer sur `sed`** : s'arrêter et remonter (§ 7, risque R1).

### 3c. L'en-tête du nouveau module

`tests/mod.rs` porte un `//!` qui dit trois choses, chacune parce qu'elle
s'oublie autrement :

1. **Pourquoi un répertoire** — le § 2a en deux phrases, avec la citation de
   `perimeter/tests.rs:175-180` et mika#2286. Sans elle, un futur éditeur
   « simplifie » en `tests.rs` et chaque PR de test redevient un draft.
2. **Pourquoi un seul fichier aujourd'hui** — les deux marges du § 2b et le
   renvoi du découpage thématique, pour qu'un relecteur n'ait pas à chercher si
   le découpage a été oublié ou décidé.
3. **Ce que la déclaration `mod tests;` sans visibilité achète** — § 2c, les
   deux raisons.

---

## 4. AC3 : la grille de classement des sites, et ce qu'elle décide

Le ticket nomme 14 sites qui lisent `agent_loop/mod.rs` par chemin, dans 5
fichiers. Le recensement se refait sur la tête, par une commande que le corps
de PR cite :

```
grep -rn "agent_loop/mod.rs" crates/mika-agent/src crates/mika-agent/tests crates/mika-common/src
```

Si le compte diffère de 14, **la mesure refaite est la référence** (le 14 du
ticket est une mesure du 2026-11-01) et l'écart est déclaré dans le corps avec
sa cause. Un compte reproduit sans être re-mesuré est une citation, pas une
mesure.

Le classement n'est pas un exercice de prose : **la classe décide du geste**, et
pour deux des trois classes l'arbitre est la suite de tests.

### Classe (i) — passe par le lecteur canonique → **indifférent par construction**

Le site lit via `ProductionScanner::production_of` /
`source_scan::production_half` / `source_scan::production_files()`.

Aucune retouche, et ce n'est pas une espérance : `production_of` rend
`blank_like(content)` — des lignes vides, numérotation préservée — pour tout
fichier que `test_only` reconnaît (`source_guard.rs:1000-1008`), et
`test_only_modules` reconnaît `tests/mod.rs` **dès la déclaration du § 2c**, par
résolution de la déclaration et non par convention de nommage. Côté
`source_scan`, `is_test_source_path` classe test tout chemin portant un segment
`tests` (`source_scan.rs:79`), donc `production_files()` l'écarte aussi.

Sites déjà identifiés dans cette classe : `server/a2a.rs:2729`, et les quatre
`production_of(&scanner.src_root().join("agent_loop/mod.rs"))` du bloc lui-même
(l. 12 818, 13 078, 13 109, 13 962), qui excluaient déjà la moitié test et
continueront d'exclure un fichier vide.

### Classe (ii) — lit brut, asserte une **présence** ou un **comptage** → la suite de tests arbitre

Le site fait `include_str!` ou `read_to_string` sans filtre, puis
`assert!(…contains(…))` ou `assert_eq!(n, N)`. Si l'aiguille vivait dans la
moitié test, le test **rougit** à l'étape 5 — la panne est bruyante. Le geste
est de constater où vit l'aiguille et, si elle est dans les tests, de faire
suivre le site vers `agent_loop/tests/mod.rs`.

Sites déjà identifiés : `server/a2a.rs:2904` (`include_str!`, présence de
`attest_effective_model(effective_llm)`), `image_disposition.rs:527`
(comptage == 1 de `image_disposition::decide(`), `qa_reviewed_head.rs:1961`
(comptage == 2 de conditions d'armement), `mod.rs:20703` (présence de
`history_scope_label`). Les quatre visent *a priori* de la production ; c'est
l'étape 5 qui le prouve, pas ce plan.

Trois des 8 sites comptés dans `mod.rs` ne sont ni l'un ni l'autre : ce sont des
**chemins passés en argument à un helper sur une fixture en mémoire**
(l. 21 155, 21 558, 21 996). Ils ne lisent aucun fichier, donc ils sont
indifférents, et le corps de PR le dit ainsi plutôt que de les ranger de force.

### Classe (iii) — lit brut, asserte une **absence** → le seul danger silencieux

Un `assert!(!…contains(needle))` sur `mod.rs` devient vrai **à tort** si
l'aiguille migre dans `tests/mod.rs`. Le site passe au vert sans rien prouver.

Chaque site de cette classe est nommé dans le corps de PR et **re-visé sur le
lecteur canonique** (`ProductionScanner` ou `source_scan::production_files()`),
ce qui le fait basculer en classe (i). Jamais une exemption par fichier :
`source_scan.rs:46-52` refuse ce geste en toutes lettres (« Élargir les
aiguilles ou poser une allowlist par fichier est **refusé** »), l'AC1 exige que
`LARGE_FILE_ALLOWLIST` reste vide, et l'AC3 exige qu'aucun site ne passe au vert
par une allowlist.

### Le périmètre que le ticket ne nomme pas, et qu'il faut nommer

Au-delà des 14 sites, **~17 gardes du crate énumèrent `src/**/*.rs` par un walk
maison**, sans passer par `ProductionScanner` ni par `is_test_source_path` :
`grooming_marker.rs:759`, `image_disposition.rs:580`, `auto_pull_stop.rs:450`,
`wip_rescue.rs:3428`, `worktree_reaper.rs:6051/6872/7893/9881`,
`ready_label.rs:1247`, `evidence/guards.rs:6757`, `planning/policy.rs:197`,
`tools/pr_merge_with_gate.rs:4717/5947`, `server/mod.rs:2390`,
`task_engine/engine.rs:9893/9987/10118`, `milestone_manager/liveness.rs:1056`.

Pour celles-là, `tests/mod.rs` sera lu **comme de la production** : 405 Ko de
fixtures qui posent légitimement des motifs interdits. Ce n'est pas un trou
neuf — elles traversent déjà `db/tests/*.rs` et `perimeter/tests.rs`, soit
~463 Ko de fixtures depuis mika#2321, et elles sont vertes. Ce vol ajoute 405 Ko
dans le même trou, et une aiguille donnée peut s'y trouver.

L'instrument est `cargo test -p mika-agent` à l'étape 5, et la résolution est
celle de la classe (iii) : router la garde vers le lecteur canonique, qui est le
lecteur unique que `source_scan` réclame pour lui-même (« un seul lecteur, pas
une copie par garde », mika#2158). C'est de la **vérification**, pas du code
écrit : le § *Taille estimée* en porte une borne haute plutôt qu'un compte.

---

## 5. Contrat de vérification

| ce qui est vérifié | comment |
|---|---|
| AC1 — taille de `mod.rs` | `wc -c crates/mika-agent/src/agent_loop/mod.rs` ≤ 750 000. Attendu ~642 430 (642 396 de production + la déclaration), marge ~107 570 |
| AC1 — allowlist vide | `scripts/check-secrets.sh:42-44` inchangé, et le diff de la PR ne touche pas `scripts/check-secrets.sh` |
| AC1 — les deux fichiers passent le scan | `scripts/check-secrets.sh --changed origin/main` vert ; les détecteurs étant diff-scopés, seuls `mod.rs` (~642 Ko) et `tests/mod.rs` (~405 Ko) sont présentés |
| AC2 — compte de tests identique | `cargo test -p mika-agent -- --list`, avant et après ; les deux comptes dans le corps de PR |
| AC3 — les sites classés | le recensement par la commande du § 4, et une ligne par site avec sa classe |
| AC4 — mika2398 | `cargo test -p mika-common mika2398` vert |
| aucune fixture altérée | `cargo test -p mika-agent` vert **avec le compte de l'étape 1** ; + `git diff --numstat` sur le fichier de tests montre un reformatage, non une réécriture de contenu |
| hygiène | `cargo clippy`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` |

---

## 6. Acceptance criteria

Transcrits du corps de senara-solutions/mika#3023 :

- **AC1.** Sur la tête de la PR, `agent_loop/mod.rs` pèse au plus 750 000
  octets, et `LARGE_FILE_ALLOWLIST` reste vide.
- **AC2.** Le nombre de tests de `mika-agent` est identique avant et après,
  compté avec `cargo test -p mika-agent -- --list`. Les deux comptes figurent
  dans le corps de la PR.
- **AC3.** Chacun des 14 sites ci-dessus est classé dans le corps de la PR : il
  lit la production, il lit les tests, ou il est indifférent. Ceux qui lisent
  les tests suivent le déplacement. Aucun ne passe au vert par une allowlist.
- **AC4.** `cargo test -p mika-common mika2398` est vert.

---

## 7. Risques

**R1 — `cargo fmt` échoue ou ne termine pas sur 405 Ko.** C'est le seul geste
non négociable du plan, et il n'a pas de substitut sûr (§ 3b). Conduite :
s'arrêter, ne **pas** basculer sur `sed`, et remonter à l'opérateur avec la
sortie de `cargo fmt`. Un repli acceptable, s'il est confirmé par une mesure :
laisser le corps à son indentation d'origine et poser un `#[rustfmt::skip]` —
mais il se discute, il ne se décide pas en vol.

**R2 — une garde de la classe (iii) non détectée.** Une absence qui devient
vraie à tort ne rougit pas. La borne est que la classe (iii) est *énumérable* :
ce sont les sites qui lisent `mod.rs` brut et assertent une négation, et le
recensement du § 4 les expose tous par lecture du prédicat. Le corps de PR doit
dire, pour chaque site brut, s'il asserte une présence, un comptage ou une
absence — c'est cette ligne-là qui ferme R2, pas la suite verte.

**R3 — le compte de tests change.** Un `#[test]` perdu dans la troncature. La
sonde est l'AC2, et elle est exacte : `-- --list` compte, elle ne juge pas. Les
chemins de module ne changeant pas (§ 2b point 2), un écart signale une perte,
jamais un renommage.

---

## 8. Hors périmètre

- **Les régions de production qui ne font que grossir** (hors vol, ticket).
- **Un avertissement du groom quand le fichier cible approche du plafond** —
  l'autre question du commentaire 4 de mika#2983, laissée ouverte par le ticket.
- **Le découpage thématique de `agent_loop/tests/`** — § 2b, renvoyé.
- **Aucun détecteur n'est livré, donc le gate Fire-Disposition est N/A.** Ce
  vol déplace du code de test ; il n'ajoute ni test, ni lint, ni garde CI, ni
  assertion dont le chemin de succès serait « aucune violation trouvée ». Les
  retouches possibles des classes (ii)/(iii) **re-visent le périmètre de gardes
  existantes** sur le lecteur canonique du dépôt ; elles n'en créent aucune et
  n'élargissent aucun prédicat. En particulier, ce plan ne livre **pas** de test
  asservissant la taille de `mod.rs` à un plafond : ce serait un détecteur, et
  le ticket le range dans la question laissée ouverte.

---

## 9. Taille estimée

Les deux comptes sont séparés, comme le ticket l'exige : le Plan-Size Gate lit
la ligne `Total estimé`, et un déplacement qui y entrerait la ferait lire comme
du code écrit.

### Code écrit

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/agent_loop/mod.rs` — déclaration `#[cfg(test)] mod tests;` | 3 |
| `crates/mika-agent/src/agent_loop/tests/mod.rs` — en-tête `//!` (§ 3c) | 45 |
| retouches des sites de classe (ii)/(iii) qui changent de cible — borne haute, 6 sites | 60 |

Total estimé : 108 lignes

Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

### Code déplacé, sans modification

**9 886 lignes** (404 539 octets) : le corps du `mod tests` de
`agent_loop/mod.rs`, versé tel quel dans `agent_loop/tests/mod.rs`. Aucune ligne
n'est réécrite — seule l'indentation change, et par `cargo fmt` (§ 3b), jamais
à la main ni par `sed`. Le diff le fera apparaître comme ~9 886 ajoutées et
~9 886 supprimées ; c'est un déplacement, pas une rédaction.

### Pourquoi un seul vol

108 lignes écrites, soit **25 % de la borne par vol** de 429 et 11 % du seuil par
PR de 1 000. Le volume de ce ticket est en octets déplacés, pas en lignes
écrites, et le déplacement se fait par trois commandes (`sed`, troncature,
`cargo fmt`) qui coûtent des tours constants et non proportionnels au volume.
Le précédent mika#2321 l'a mesuré : son volet B (#2395) a déplacé ~22 000 lignes
en une PR, soit plus du double d'ici.

La part du vol qui coûte réellement des tours est l'étape 5 — traiter les rouges
des classes (ii)/(iii) et des ~17 gardes à walk maison (§ 4). C'est pour cela
que le découpage thématique est renvoyé (§ 2b) : il ajouterait un arbitrage de
418 tests et le renommage de 8 citations à un vol dont le budget de tours est
déjà engagé sur du diagnostic.

---

## 10. Definition of Done

- [ ] `crates/mika-agent/src/agent_loop/tests/mod.rs` existe, porte l'en-tête
      du § 3c, et contient les 9 886 lignes du bloc de tests.
- [ ] `crates/mika-agent/src/agent_loop/mod.rs` se termine par
      `#[cfg(test)]` / `mod tests;` et pèse ≤ 750 000 octets (AC1).
- [ ] `scripts/check-secrets.sh` n'est pas touché ; `LARGE_FILE_ALLOWLIST` est
      vide (AC1).
- [ ] Les deux comptes `cargo test -p mika-agent -- --list` sont égaux et tous
      deux cités dans le corps de PR (AC2).
- [ ] Le recensement du § 4 est dans le corps de PR, un site par ligne avec sa
      classe (i)/(ii)/(iii) et, pour les sites bruts, la nature de l'assertion
      (présence / comptage / absence) (AC3, R2).
- [ ] `cargo test -p mika-common mika2398` vert (AC4).
- [ ] `cargo build`, `cargo test`, `cargo clippy`, `cargo fmt --check` verts.
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] Le ticket de suivi du découpage thématique est ouvert (§ 2b) et son
      numéro est cité sur une ligne `Tracked in:` du corps de PR.
- [ ] Corps de PR : `Closes senara-solutions/mika#3023` et
      `Code review: delegated to mika-qa (mika#2694)`.
