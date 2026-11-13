# mika#2888 — Les agents bien connus épinglent `allow_authoring = false`, et leur spécification passe sous CODEOWNERS

> **Ce vol n'ouvre ni ne ferme l'écriture de skills.** Il rend **mécanique** une
> réserve qui n'était jusqu'ici qu'une valeur dans un littéral TOML : quatre
> `false` que rien n'oblige à rester `false`. mika#1580 reste la seule entrée
> vers l'auteur de skills, et le jour où elle décide d'ouvrir, c'est le test livré
> ici qui est modifié — dans la PR de #1580, avec la décision de Vincent citée.

---

## Le constat, et ce qui le rend décidable

`skills.allow_authoring` est **déjà** code-owned : la clé est nommée dans
`CODE_OWNED_IDENTITY_SECTIONS` (`well_known_agents.rs:711`), donc le réconciliateur
remplace au démarrage la valeur du disque par celle du spec. Ce qui n'est épinglé
nulle part, c'est **la valeur que le spec déclare**.

Les deux détecteurs qui encadrent déjà cette constante se prononcent sur la
*structure*, jamais sur le *contenu* :

- `test_code_owned_sections_have_reconciler_coverage` (`:4263`) exige qu'**au moins
  un** spec émette la clé. Un seul suffit — trois agents pourraient perdre la clé
  sans que le test bouge.
- `test_no_code_owned_drift_outside_constant` (`:4291`) exige l'inverse : tout
  chemin émis doit être couvert par la constante. Il ne lit pas les valeurs.

Entre les deux il y a exactement un trou, et c'est celui de ce ticket : un
`allow_authoring = true` écrit dans n'importe lequel des quatre littéraux passe
les deux détecteurs, passe `cargo test`, et **se propage à l'agent sur disque au
démarrage suivant** — précisément parce que la clé *est* code-owned. La force du
réconciliateur est ici le vecteur, pas la protection.

Les quatre sites, lus sur le fichier :

| agent | source d'identité | site du `false` |
|---|---|---|
| mika-dev | `Static(MIKA_DEV_IDENTITY)` | `:134` |
| mika-qa | `Static(MIKA_QA_IDENTITY)` | `:318` |
| mika-arch | `Computed(build_mika_arch_identity)` | `:633` (dans le `format!`) |
| mika-test | `Static(MIKA_TEST_IDENTITY)` | `:1650` |

Un seul des quatre n'est pas un littéral statique. C'est ce qui décide la forme du
test, en D1.

---

## D1 — Le test rend l'identité, il ne grep pas le fichier

Trois formes étaient disponibles. Deux sont écartées pour la même raison : elles
ne voient pas mika-arch.

**Écartée — grep sur le source.** Un scan textuel de `well_known_agents.rs`
comptant les occurrences de `allow_authoring = false` est aveugle à la
correspondance agent↔valeur : quatre occurrences ne disent pas qu'elles sont chez
quatre agents distincts, et il ne peut pas nommer l'agent fautif (AC1).

**Écartée — assertion sur les constantes.** Lire `MIKA_DEV_IDENTITY` et ses
siblings couvre trois agents sur quatre : mika-arch n'a **pas** de constante
d'identité, son TOML naît d'un `format!` dans `build_mika_arch_identity(settings)`.
Et surtout, cette forme viole l'exigence du corps du ticket — « le test énumère les
agents à partir de la même source que le reconciler » : elle énumère des
constantes, pas des agents, donc un cinquième agent bien connu ajouté demain
n'entre dans le test que par une édition du test.

**Retenue — rendre, puis marcher le chemin pointé.** Le test itère
`WELL_KNOWN_AGENTS` (`:644`), appelle `render_identity_content(spec, &settings)`
(`:1015`), parse le TOML, et marche `skills.allow_authoring` avec `get_path`
(`:736`). C'est, terme pour terme, le chemin du réconciliateur
(`provision_well_known_agents` → `reconcile_well_known_identity`, `:855` itère la
même constante avec le même marcheur). Trois propriétés en découlent :

1. **Un nouvel agent bien connu est couvert sans édition du test** —
   `WELL_KNOWN_AGENTS` est la seule liste à éditer, et c'est la même que celle du
   réconciliateur. C'est l'exigence explicite du corps du ticket.
2. **mika-arch est couvert par sa voie `Computed`** — la valeur testée est celle
   que l'agent reçoit vraiment, pas celle qu'un littéral laisse croire.
3. **Le test est au bon endroit** : dans le `mod tests` de
   `well_known_agents.rs`, où `get_path`, `render_identity_content` et
   `test_settings_with_kg_roots()` (`:1761`) sont déjà à portée. `get_path` est
   privé au module — c'est une raison de plus de ne pas poser le test ailleurs.

### Un `Err` de rendu doit rougir, pas être sauté

Les deux détecteurs voisins écrivent `Err(_) => continue`. **Le nôtre ne le fait
pas.** `render_identity_content` ne retourne `Err` que sur la voie `Computed` de
mika-arch quand `kg_docs_roots` est absent ; or le test fournit
`test_settings_with_kg_roots()`, qui le peuple. Un `Err` sous ces settings-là est
donc un défaut, et `continue` le transformerait en couverture silencieusement
perdue : mika-arch sauté, test vert, réserve non épinglée chez l'agent qui lit le
plus de code. Le test `expect`, et le message nomme l'agent.

C'est le seul écart délibéré au patron des deux voisins, et il va dans le sens du
resserrement.

---

## D2 — Le prédicat est extrait, et c'est ce qui arme AC2 et AC3

Le ticket demande deux recettes de rouge-avant : passer une valeur à `true`
(AC2), et ajouter un agent sans la clé (AC3). Jouées à la main, elles attestent
le détecteur **le jour de la PR** et rien après. AC3 est en plus coûteuse à jouer :
elle demande d'ajouter une cinquième entrée à `WELL_KNOWN_AGENTS`, donc un
`WellKnownAgent` complet, donc une manipulation qu'on ne veut pas laisser derrière
soi.

Le plan extrait donc le prédicat dans un helper de test pur :

```rust
/// Retourne `None` quand la spécification épingle bien la réserve, et
/// `Some(<message nommant l'agent et ce qui cloche>)` sinon.
///
/// Deux formes de violation, et une seule porte : la clé absente et la clé à
/// `true` sont toutes deux des manquements à la réserve de mika#1580. L'absence
/// compte parce que `allow_authoring` est code-owned (`CODE_OWNED_IDENTITY_SECTIONS`)
/// : une spécification qui n'émet rien laisse le réconciliateur sans valeur à
/// imposer, et la valeur effective retombe sur le défaut de
/// `SkillsIdentityConfig::authoring_enabled()` — `false` aujourd'hui, mais un
/// défaut n'est pas une réserve, et il se change à un autre endroit du code.
fn allow_authoring_pin_violation(agent_name: &str, rendered_toml: &str) -> Option<String>
```

Le helper prend le TOML **rendu** en `&str`, pas un spec. C'est ce qui le rend
exerçable sur un fixture, et donc ce qui arme les deux recettes :

| recette | forme armée | fixture |
|---|---|---|
| AC2 — valeur à `true` | test unitaire sur le helper | `[skills]\nallow_authoring = true` |
| AC3 — agent sans la clé | test unitaire sur le helper | `[skills]\nnudge_enabled = false` (pas de `allow_authoring`) |

Les deux tests assertent aussi que le message **contient le nom de l'agent**
passé — AC1 n'est pas « le test échoue », c'est « il nomme le fautif », et une
assertion sur le texte du message est la seule chose qui le vérifie.

Les recettes manuelles restent jouées une fois pendant l'implémentation (le ticket
les demande en ces termes : « puis on revient en arrière »), et leur résultat est
écrit dans l'en-tête de doctrine du test. Mais ce qui survit à la PR, ce sont les
deux fixtures.

---

## D3 — La ligne CODEOWNERS, et ce qu'elle ajoute vraiment

`.github/CODEOWNERS` porte déjà deux périmètres de même classe — la forge-gate
(`/crates/mika-agent/src/perimeter/`, …) et la surface de flux sortant
(`/docs/egress/`) — plus sa propre auto-référence. La ligne ajoutée est :

```
/crates/mika-agent/src/well_known_agents.rs         @samidarko
```

Elle entre sous son propre bloc commenté, dans le style des deux existants, qui
dit **pourquoi un test ne suffit pas** : le test livré ici vit dans le fichier
qu'il protège. Une PR qui ouvre l'écriture de skills édite le littéral *et* le
test dans le même diff, et repart verte. La ligne CODEOWNERS est le seul mécanisme
qui met un humain sur ce diff-là. Le test dit *qu'une* valeur a changé ; CODEOWNERS
décide *si* elle avait le droit.

Par AC4, le merge de cette PR passe par samidarko — `/.github/CODEOWNERS` se
possède lui-même, donc l'ajout déclenche sa propre règle. C'est attendu, et ce
n'est pas un obstacle à lever : c'est la démonstration que la ligne mord.

---

## D4 — Ce que ce vol ne touche pas

- **Aucune valeur de `allow_authoring` ne change.** Les quatre restent `false`.
- **`CODE_OWNED_IDENTITY_SECTIONS` est inchangée** — `skills.allow_authoring` y
  est déjà (`:711`). Rien à ajouter, rien à retirer.
- **`SkillsIdentityConfig` (`prompt.rs:519`) et sa porte d'exécution
  (`tools/skill_manage.rs:78`) sont inchangées.** Ce vol épingle une
  déclaration, il ne retouche pas le gate runtime.
- **`skill_nudge.rs` est inchangé.** Il lit `allow_authoring` comme
  présupposition ; sa logique ne dépend pas de l'épinglage.
- **Les deux détecteurs voisins (`:4263`, `:4291`) ne sont pas modifiés.** Le
  nouveau test est un troisième, pas une réécriture des deux premiers — ils
  répondent à des questions différentes (structure vs valeur) et doivent rester
  lisibles séparément.
- **Aucune porte d'environnement, aucune migration de schéma, aucun `docs/`
  touché** hormis ce plan.

---

## Phase unique — l'ordre de commit

Un seul vol, un seul commit logique. L'ordre n'est pas libre : les deux recettes
de rouge-avant se jouent **après** que le détecteur existe et **avant** qu'on le
déclare vert.

1. **`.github/CODEOWNERS`** — la ligne et son bloc de commentaire. Première, parce
   qu'elle est indépendante et qu'elle doit exister dans le diff que samidarko
   relit.
2. **Le helper `allow_authoring_pin_violation`** dans le `mod tests` de
   `well_known_agents.rs`, avec son doc-comment (les deux formes de violation et
   pourquoi l'absence compte).
3. **Le détecteur** `test_well_known_agents_pin_allow_authoring_false` — itère
   `WELL_KNOWN_AGENTS`, rend via `render_identity_content` avec
   `test_settings_with_kg_roots()`, `expect` sur l'`Err` en nommant l'agent,
   parse, applique le helper, collecte **toutes** les violations avant d'assert
   (un seul run doit nommer les quatre fautifs s'il y en a quatre, pas le premier).
4. **Les deux tests de fixture** — AC2 (`true`) et AC3 (clé absente), chacun
   assertant que le message nomme l'agent passé.
5. **Recette manuelle de rouge-avant AC2** : passer `:134` à `true`, voir le
   détecteur rougir en nommant `mika-dev`, revenir en arrière, revoir vert.
   Résultat écrit dans l'en-tête de doctrine.
6. **Recette manuelle de rouge-avant AC3** : la même sur mika-arch via la voie
   `Computed` — retirer la ligne du `format!` à `:633`, voir rougir en nommant
   `mika-arch`, revenir. C'est la recette qui atteste que la voie non-littérale
   est bien couverte, et c'est pour cela qu'elle se joue sur mika-arch plutôt que
   sur un `Static`.
7. **`cargo fmt`**, **`cargo clippy`**, **`cargo test -p mika-agent`**, les lints
   du dépôt, **`scripts/verify-pipeline.sh origin/main`**.

Plancher : les étapes 1–4 sont le livrable ; 5–6 n'ajoutent pas de code mais
leur résultat va dans le doc-comment, donc elles précèdent le commit.

---

## Fire-Disposition

Ce vol livre un détecteur — trois tests dont le chemin de succès est « aucune
violation trouvée ». Disposition : **(a) exception nommée en allowlist, livrée
vide, et le détecteur l'assert vide.**

```rust
/// **Livrée vide, et le détecteur plus bas en dépend.**
///
/// Il n'y a rien à excuser à la livraison : les quatre spécifications de
/// `WELL_KNOWN_AGENTS` déclarent déjà `allow_authoring = false` — mika-dev
/// `:134`, mika-qa `:318`, mika-arch `:633` (voie `Computed`), mika-test
/// `:1650`. Le détecteur est donc **armé et vert** au merge, sans aucune
/// dérogation.
///
/// **Quand ce détecteur tire, on ne l'excepte pas.** Une entrée ici serait une
/// ouverture de l'écriture de skills décidée dans une PR de test, ce qui est
/// exactement ce que mika#1580 réserve à sa propre PR. La seule issue légitime
/// est l'une des deux : rendre la valeur à `false`, ou modifier ce détecteur
/// dans la PR de mika#1580 en y citant la décision de Vincent (doctrine
/// mika#2201 — un site qu'on ne veut pas armer est un site à supprimer, pas à
/// excepter).
const ALLOW_AUTHORING_PIN_EXCEPTIONS: &[&str] = &[];
```

L'assertion auto-nettoyante est portée par le détecteur lui-même : il assert
`ALLOW_AUTHORING_PIN_EXCEPTIONS.is_empty()` avec un message qui renvoie au
doc-comment ci-dessus et à mika#1580. Une entrée ajoutée sans la décision citée
fait donc rougir **le détecteur**, pas seulement la revue — et l'exception ne peut
pas devenir stale, puisqu'il n'y en a aucune.

Les deux tests de fixture (AC2, AC3) n'ont pas d'allowlist : ils s'exécutent sur
des TOML construits dans le test, hors de tout périmètre réel.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/well_known_agents.rs` — helper + doc-comments | 40 |
| `crates/mika-agent/src/well_known_agents.rs` — détecteur + assertion d'allowlist vide | 35 |
| `crates/mika-agent/src/well_known_agents.rs` — deux tests de fixture (AC2, AC3) | 30 |
| `.github/CODEOWNERS` — ligne + bloc de commentaire | 8 |

Total estimé : 113 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 113 estimées contre une borne de 279 : il reste
plus du double de marge. Et la marge est plus large que le nombre ne le dit, pour
trois raisons structurelles — aucun fichier nouveau (tout atterrit dans un
`mod tests` existant et dans un `CODEOWNERS` existant), aucune surface de
production touchée donc aucun appelant à mettre à jour, et aucune migration ni
porte d'environnement. Le seul coût en tours au-delà de l'écriture est celui des
deux recettes manuelles de rouge-avant (étapes 5–6), qui sont deux éditions d'une
ligne, deux `cargo test -p mika-agent well_known_agents`, et deux retours en
arrière. L'estimation du ticket — S — tient.

Rien n'est renvoyé à un suivi : les quatre AC sont intégralement dans ce vol.

---

## Acceptance criteria

Transcrits du corps de mika#2888.

- **AC1** : le test passe sur main, et nomme l'agent fautif s'il échoue.
- **AC2** : rouge-avant. Passer une des valeurs à `true` fait rougir le test, puis
  on revient en arrière.
- **AC3** : un agent bien connu ajouté sans cette clé fait aussi rougir le test.
- **AC4** : la ligne CODEOWNERS est présente. Le merge de cette PR passe par
  samidarko, puisque CODEOWNERS se possède lui-même.

### Comment chacun est vérifié

- **AC1** → le détecteur de l'étape 3 est vert sur main (`cargo test -p mika-agent
  well_known_agents`), et les deux tests de fixture assertent que le message de
  violation **contient le nom de l'agent** passé au helper. Le détecteur collecte
  toutes les violations avant d'assert, donc un run nomme tous les fautifs, pas le
  premier.
- **AC2** → deux fois. Armé : test de fixture `allow_authoring = true` → le helper
  retourne `Some`. Manuel : recette de l'étape 5 sur `:134`, résultat écrit dans
  l'en-tête de doctrine.
- **AC3** → deux fois. Armé : test de fixture sans la clé → le helper retourne
  `Some`. Manuel : recette de l'étape 6 sur la voie `Computed` de mika-arch
  (`:633`), qui atteste en plus que l'agent non-littéral est couvert. L'énumération
  depuis `WELL_KNOWN_AGENTS` est ce qui rend un futur cinquième agent couvert sans
  édition du test.
- **AC4** → `grep -n 'well_known_agents.rs' .github/CODEOWNERS` retourne la ligne ;
  le check de revue requise sur la PR nomme samidarko.

---

## Definition of Done

- [ ] `allow_authoring_pin_violation` livré dans le `mod tests` de
      `well_known_agents.rs`, avec son doc-comment nommant les **deux** formes de
      violation (clé absente, clé à `true`) et pourquoi l'absence compte.
- [ ] Le détecteur énumère `WELL_KNOWN_AGENTS` — la même source que le
      réconciliateur — et marche `skills.allow_authoring` avec `get_path`.
- [ ] L'`Err` de `render_identity_content` **rougit** et nomme l'agent ; aucun
      `Err(_) => continue`.
- [ ] Le détecteur collecte toutes les violations avant d'assert.
- [ ] `ALLOW_AUTHORING_PIN_EXCEPTIONS` livrée vide, et le détecteur assert qu'elle
      l'est, avec le renvoi à mika#1580 dans le message.
- [ ] Deux tests de fixture (AC2, AC3) verts, chacun assertant que le message
      nomme l'agent.
- [ ] `.github/CODEOWNERS` porte `/crates/mika-agent/src/well_known_agents.rs
      @samidarko`, sous un bloc commenté qui dit pourquoi le test ne suffit pas.
- [ ] Les deux recettes manuelles de rouge-avant jouées, retours en arrière
      effectués, résultats écrits dans l'en-tête de doctrine du détecteur.
- [ ] Diff vide sur les six surfaces de D4 — en particulier `CODE_OWNED_IDENTITY_SECTIONS`,
      `SkillsIdentityConfig`, `skill_manage.rs`, `skill_nudge.rs` et les deux
      détecteurs voisins (`:4263`, `:4291`).
- [ ] Les quatre valeurs de `allow_authoring` valent toujours `false` à la fin du
      diff.
- [ ] `cargo fmt`, `cargo clippy`, `cargo test -p mika-agent`, les lints du dépôt
      et `scripts/verify-pipeline.sh origin/main` verts.
- [ ] `Code review: delegated to mika-qa (mika#2694)` dans le corps de la PR.

---

## Risques

### R1 — Le test vit dans le fichier qu'il protège

C'est la limite reconnue, et c'est la raison d'être de la ligne CODEOWNERS (D3).
Une PR peut éditer le littéral et le test ensemble et repartir verte. Aucun
arrangement de code ne ferme cela — déplacer le test dans un autre fichier le
couperait de `get_path` (privé au module) et ne fermerait rien, puisque la même PR
pourrait éditer les deux fichiers. Le mécanisme de fermeture est humain **par
conception** : la ligne CODEOWNERS met samidarko sur tout diff du fichier, et le
corps du ticket nomme la seule issue légitime (la PR de #1580, décision citée).
Mitigation partielle dans le code : le doc-comment du détecteur et celui de
l'allowlist disent tous deux, en clair, que modifier ce test hors de #1580 est la
violation.

### R2 — Un cinquième agent bien connu dont le rendu échoue

Un futur agent à identité `Computed` qui exigerait un setting absent de
`test_settings_with_kg_roots()` ferait rougir le détecteur sur son `expect`, pas
sur la réserve. Le rouge serait exact — l'agent n'est pas attesté — mais son
message doit le dire sans ambiguïté, sinon il se lit comme une violation de la
réserve. Mitigation : le message de l'`expect` distingue explicitement « le rendu
a échoué pour l'agent X » de « l'agent X n'épingle pas la réserve », et nomme le
settings à compléter. C'est un rouge qu'on veut : il signale un agent que le
détecteur ne couvre pas encore.

### R3 — La boucle autonome ne peut pas merger cette PR

Par AC4, `/.github/CODEOWNERS` se possède lui-même, donc la ligne ajoutée fait
entrer la PR dans son propre périmètre de revue humaine : le bot ne mergera pas
seul. Ce n'est pas un risque à mitiger, c'est le comportement attendu et déclaré
par l'AC. Conséquence opérationnelle à porter dans le corps de la PR : la PR
attend une approbation de samidarko, et une absence de merge automatique ne doit
pas être lue comme un échec de pipeline.
