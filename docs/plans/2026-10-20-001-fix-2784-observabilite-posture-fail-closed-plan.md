# fix(agent) — rendre la posture fail-closed **observable**, avant de la resserrer (mika#2784, morceau 1/2)

> **Ticket :** senara-solutions/mika#2784 — *le chemin fail-closed échoue encore
> par dénylist — le durcir en allowlist*
> **Labels :** `p2-normal`, `ready`, `dispatch:loop`

---

## 0. Pourquoi ce ticket se groome aujourd'hui, et sur quel morceau

Le corps du ticket porte une section `## Dormant, ouvert (2026-10-20) : conditions
de réveil` qui découpe le périmètre en deux morceaux et donne à chacun sa
condition. Les deux conditions ont été vérifiées avant d'écrire une ligne de ce
plan :

| morceau | condition de réveil | état mesuré | verdict |
|---|---|---|---|
| 1 — observabilité (ligne de journal + test comportemental) | « au merge de #2781 » | `3a087b37 feat(agent): basculer les outils de mika-arch vers une allowlist (mika#2748) (#2781)` est **sur `main`**, et `prompt.rs:1119` porte bien `allowed: None` | **réveillé** |
| 2 — resserrement de `allowed` | « quand Vincent a tranché la question 1 (et la 2), par une décision écrite sur ce ticket » | `gh issue view 2784 --json comments` rend **0** commentaire. Aucune décision écrite. | **toujours dormant** |

**Ce plan livre exclusivement le morceau 1.** Il ne touche pas à `allowed`, qui
reste `None`. C'est la question 3 du ticket (« l'observabilité d'abord »), et
l'ordre n'est pas un confort : *resserrer une posture qu'on ne peut pas voir,
c'est rendre l'incident suivant plus difficile à lire, pas plus sûr*
(mika#2293).

`Tracked in: senara-solutions/mika#2784` — le morceau 2 reste **sur ce
ticket**, par son propre protocole de réveil (« le resserrement de `allowed`
peut se grommer. Sa condition préalable est la partie 1, livrée »). Conséquence
opératoire, portée en DoD : la PR de ce vol dit **`Refs #2784`**, jamais
`Closes #2784` — fermer le ticket ici effacerait la décision de doctrine en
attente.

---

## 1. État mesuré du code

### 1.1 Les trois entrées du chemin fail-closed

`prompt::fail_closed_identity()` (`crates/mika-agent/src/prompt.rs:1092`) a
**trois** appelants, tous dans le même fichier, et chacun correspond à une
population distincte avec sa propre remédiation :

| appelant | condition | événement journalisé aujourd'hui | sert fail-closed ? |
|---|---|---|---|
| `unreadable_identity_fail_closed` (`:1015`) | `ErrorKind::NotFound` | `identity_toml_absent` | oui, **tout** agent (mika#2027) |
| `unreadable_identity_fail_closed` (`:1026`) | autre erreur d'E/S | `identity_toml_unreadable` | oui, **tout** agent (mika#2027) |
| `parse_identity_or_fail_closed` (`:1062`) | TOML invalide **et** agent bien connu | `identity_toml_malformed` | oui (#811) |
| `parse_identity_or_fail_closed` (`:1074`) | TOML invalide, agent **non** bien connu | `identity_toml_malformed` | **non** — `Identity::default()`, donc permissif |

### 1.2 Le défaut d'observabilité, en deux parties

**(a) Aucun nom ne désigne la posture.** Les trois noms existants décrivent
l'**état du fichier**, pas la **posture servie**. Un opérateur qui veut compter
les agents qui tournent en fail-closed doit faire l'union de trois greps dont
l'un est ambigu — et ce n'est pas une lacune cosmétique : rien ne garantit qu'un
quatrième appelant futur de `fail_closed_identity()` pense à journaliser quoi
que ce soit, la fonction étant muette.

**(b) `identity_toml_malformed` couvre deux postures opposées.** Les lignes
`:1064` et `:1074` portent le **même** `event`, et servent l'une la posture
fail-closed, l'autre `Identity::default()` — c'est-à-dire *tous* les builtins et
*toutes* les skills. C'est exactement la forme mika#2205 : deux populations sous
un seul nom ne sont pas soustractibles. `docs/agent-internals/skills-system.md`
§ 44 décrit déjà la conséquence sans pouvoir la mesurer : « *a user-defined
agent with a broken `identity.toml` can look healthy while running fully
permissive — `identity_toml_malformed` is the only signal* ».

### 1.3 Ce que le champ `allowed` vaut, et qui le lit

`allowed: None` (`:1119`) porte déjà un commentaire de huit lignes nommant son
coût et renvoyant à ce ticket. **Aucune assertion ne lit ce champ sur ce
chemin** : les tests `mika2027_*` (`prompt.rs:3426`, `:3479`, `:3500`, `:3517`)
vérifient le sentinelle de skills et rien du tableau d'outils. Le morceau 2
pourra donc être écrit sans qu'aucun test ne rougisse — ce qui est précisément
ce que le ticket demande de fermer (« ni le prédicat ni la construction de
l'identité ne rougissent si le champ n'est jamais lu sur ce chemin-là »).

---

## 2. Décisions

### D1 — La ligne de posture est émise **dans** `fail_closed_identity()`, pas chez ses appelants

Le ticket dit « au moment où elle est **servie** ». La fonction qui construit
l'`Identity` est ce moment. Deux conséquences qu'un site d'appel n'achète pas :

- un quatrième appelant futur **ne peut pas** oublier la ligne, parce que la
  signature l'oblige à nommer sa cause ;
- la ligne est émise une fois par posture servie, jamais deux, jamais zéro.

**C'est aussi pourquoi ce plan ne livre pas de garde structurelle « site
unique ».** L'idiome maison serait un scan de source (forme mika#2425 / mika#1833),
mais ici le compilateur tient l'invariant plus fort : `fail_closed_identity` est
privée au module et prend un `FailClosedCause` **non-`Option`**, donc un site
d'appel sans cause ne compile pas. Un scan serait une redondance plus faible que
le type.

### D2 — Trois causes, pas deux — et c'est une extension assumée du vocabulaire du ticket

Le ticket écrit `parse_error | missing_file`. Le code a **trois** entrées
(§ 1.1). Fondre `unreadable` dans `missing_file` serait commettre, dans le nom
même qu'on introduit pour le réparer, le défaut mika#2205 qu'on répare — et les
deux remédiations divergent, ce que `docs/operator/agent-identity-reprovision.md`
§ 3 vs § 4 dit déjà : *absent* → re-provisionner depuis le gabarit de palier ;
*illisible* → corriger les permissions, **ne pas écraser le fichier**.

```rust
enum FailClosedCause { MissingFile, Unreadable, ParseError }
// → "missing_file" | "unreadable" | "parse_error"
```

### D3 — `agent_fail_closed_posture`, en ERROR, **additif** aux trois noms existants

Nom de l'événement : `agent_fail_closed_posture`. Il nomme la posture, pas le
fichier.

Niveau **ERROR**, comme les trois lignes existantes : un agent en fail-closed est
un incident, pas une information. Le coût assumé est **deux** lignes ERROR par
chargement fail-closed — et il est voulu, parce qu'elles répondent à deux
questions différentes : « qu'est-ce qui cloche dans le fichier ? » (nom existant,
cité par le runbook, par `docs/configuration.md:561` et par les commentaires de
`mika-cli/src/commands/agents.rs:1016,1402`) et « quels agents tournent en
posture dégradée ? » (nom nouveau).

**Les trois noms existants ne sont donc ni renommés ni supprimés.** Un renommage
casserait un runbook et trois sites de documentation pour un gain nul : la
soustractibilité est rendue par l'**existence** du second nom, pas par la
mutilation du premier.

Champs portés :

| champ | valeur | pourquoi |
|---|---|---|
| `agent` | nom issu de `agent_name_from_home` | demandé par le ticket ; déjà le nom partagé par les trois sites existants (`prompt.rs:1002`) |
| `cause` | `missing_file` \| `unreadable` \| `parse_error` | demandé par le ticket, étendu par D2 |
| `path` | chemin de l'`identity.toml` | aligne la ligne sur les trois existantes |
| `tools_allowlist` | `absent` (aujourd'hui) \| `present` | voir ci-dessous |

Le champ `tools_allowlist` est le seul ajout au-delà de la lettre du ticket, et
il se justifie seul : le jour où le morceau 2 posera `Some(...)`, **le même grep
dira si le resserrement est en vigueur dans le binaire servi**. Sans lui,
« le morceau 2 est déployé » et « le morceau 2 est écrit mais le binaire est
antérieur » rendent des octets identiques (classe mika#2340).

### D4 — Une ligne **par chargement**, sans déduplication

`load_identity` / `load_identity_async` ont ~25 appelants (CLI, `startup.rs`,
`task_engine`, `dispatcher`, `delegate_task`) : la posture est rechargée à chaque
tour, et la ligne suit. Pas de `OnceLock`, pas de compteur :

- la posture est une propriété du **chargement**, pas du processus —
  `mika agents reprovision` répare le fichier **sans redémarrage**, et une ligne
  « une fois par processus » resterait vraie après la réparation ;
- une déduplication rendrait l'**absence** de la ligne ambiguë (pas survenue, ou
  étouffée ?), ce qui est la forme mika#2331 ;
- le régime nominal est à **zéro** ligne, donc le volume est fonction d'un défaut
  présent, ce qui est le comportement correct ;
- la duplication par chargement est déjà une propriété connue et acceptée de ce
  chemin — `config_keys.rs:461` la décrit (« *twice, one line from
  `prompt::load_identity_async`* »).

### D5 — `posture` ajouté comme champ sur les deux lignes `identity_toml_malformed`

Deux lignes de code, et elles ferment § 1.2 (b) *sur la ligne elle-même* plutôt
qu'en obligeant l'opérateur à soustraire deux greps :

- `:1064` (agent bien connu) → `posture = "fail_closed"`
- `:1074` (agent utilisateur) → `posture = "default_fallback"`

L'`event` reste `identity_toml_malformed` dans les deux cas : c'est un événement
d'**état de fichier**, et à ce titre il est correct pour les deux branches.

### D6 — Le test comportemental passe par `missing_file`, et la raison est structurelle

`EvalHarness` donne à l'agent un `home_dir` qui est un `TempDir` à nom aléatoire
(`harness.rs:547`). Donc `find_well_known_agent(agent_name)` ne peut pas
correspondre, donc la branche `parse_error` de `parse_identity_or_fail_closed`
prend **toujours** le chemin `Identity::default()` dans le harnais : la cause
`parse_error` n'est pas atteignable en bout-en-bout sans un nom d'agent bien
connu, que le harnais n'expose pas.

Le test comportemental exerce donc le chemin **`missing_file`** — effacer
l'`identity.toml` que `build()` sème (`harness.rs:575`) — qui est de surcroît la
population la plus large (mika#2027 : *tout* agent). La cause `parse_error`
est couverte en test unitaire, où `fail_closed_identity` est appelable
directement et où un sous-répertoire nommé `mika-arch` satisfait
`find_well_known_agent`. Cette répartition est écrite ici pour que personne ne
la re-découvre en la croyant un oubli.

---

## 3. Livrables, dans l'ordre de commit

### Commit 1 — `crates/mika-agent/src/prompt.rs` : la cause et la ligne

1. `FailClosedCause` (enum privé, 3 variantes, `as_str()`), avec un doc-comment
   nommant D2 et les deux remédiations divergentes de `absent` vs `unreadable`.
2. `fail_closed_identity(agent: &str, path: &Path, cause: FailClosedCause)` :
   signature changée, un `tracing::error!(event = "agent_fail_closed_posture", …)`
   en tête, avec les quatre champs de D3. Doc-comment étendu : pourquoi la ligne
   est **ici** (D1), pourquoi elle est par chargement (D4), et pourquoi aucun
   scan « site unique » ne l'accompagne.
3. Les trois sites d'appel passent leur cause (`:1015` → `MissingFile`,
   `:1026` → `Unreadable`, `:1062` → `ParseError`).
4. `posture` ajouté aux deux `identity_toml_malformed` (D5).

**Plancher :** `cargo build -p mika-agent` et `cargo clippy` passent ; les quatre
tests `mika2027_*` existants passent **sans modification de leurs assertions**
(seuls leurs appels éventuels à `fail_closed_identity` sont adaptés s'ils
existent).

### Commit 2 — `crates/mika-agent/src/prompt.rs` : tests unitaires de la ligne

Dans le `mod tests` inline, sur l'idiome `compaction_handsoff.rs:340-490` —
helper local `capturing_subscriber()`, installation **obligatoirement** via
`crate::test_utils::test_helpers::install_capturing_subscriber` (site unique,
mika#2646), `#[serial_test::serial]` sur **chaque** test atteignant un callsite
capturé, voisins compris (c'est la moitié portante du dispositif) :

| test | ce qu'il tient |
|---|---|
| `mika2784_un_fichier_absent_nomme_la_posture_et_sa_cause` | `agent_fail_closed_posture`, `cause=missing_file`, nom de l'agent présent |
| extension de `mika2027_unreadable_identity_toml_also_fails_closed` | même ligne, `cause=unreadable` — distincte de `missing_file`, ce qui **est** D2 |
| `mika2784_un_toml_invalide_sur_agent_bien_connu_porte_parse_error` | `cause=parse_error` ; home dir = sous-répertoire nommé d'après un agent bien connu |
| `mika2784_le_regime_nominal_est_muet` | **contrôle négatif** : un `identity.toml` valide n'émet **aucune** ligne de posture. Sans lui, les trois tests ci-dessus passeraient sur un ERROR inconditionnel |
| `mika2784_un_agent_utilisateur_au_toml_invalide_nest_pas_fail_closed` | la ligne de posture est **absente**, et `identity_toml_malformed` porte `posture = "default_fallback"` — c'est l'assertion qui rend § 1.2 (b) soustractible |
| `mika2784_la_posture_ne_declare_aucune_allowlist_doutils` | **épingle** `tools.allowed == None` et `tools_allowlist = "absent"`, avec un message d'échec qui nomme mika#2784 morceau 2 et les questions 1 et 2 |

### Commit 3 — `crates/mika-agent/tests/eval/test_fail_closed_tool_surface_2784.rs`

Sur le modèle exact de `test_mika_arch_tool_allowlist_2748.rs` : un helper
`served_tool_names()` qui lit
`trace.captured_requests.first().tools`, et deux fonctions de test.

```
ADMITTED  = "send_message"   // hors dénylist par #811 (constitutif de l'agent)
WITHHELD  = "create_task"    // dans MIKA_ARCH_DISABLED_TOOLS, servi par default_tools()
```

| test | assertions |
|---|---|
| `mika2784_un_agent_fail_closed_ne_recoit_aucun_builtin_mutationnel` | `identity.toml` effacé après `build()` → `WITHHELD` **absent** du tableau servi, et `ADMITTED` **présent** (c'est la donnée que la question 1 arbitre : l'épingler rend le resserrement futur **visible** au lieu de silencieux) |
| `mika2784_avec_une_identite_valide_le_meme_builtin_est_servi` | **contrôle négatif**, porteur : sans lui, l'absence ci-dessus serait satisfaite par un registre qui n'a jamais porté `create_task` et par un filtre qui dépouille tout agent |

Déclaration du module dans `crates/mika-agent/tests/eval.rs` (voisin de la
ligne 174).

Le doc-comment de module dit, comme celui de 2748, **pourquoi un vrai tour et pas
le prédicat** : la construction de l'`Identity` et le prédicat de visibilité ont
chacun leurs tests, et tous deux restent verts si le chemin fail-closed ne
thread jamais ses champs jusqu'au crochet. Le tableau que le modèle reçoit est le
seul lieu qui le dit.

### Commit 4 — documentation (hors comptage de lignes)

- `docs/agent-internals/skills-system.md` § *Operator grep signals* : le
  quatrième nom, ses quatre champs, et la règle de lecture — *`identity_toml_*`
  dit l'état du fichier, `agent_fail_closed_posture` dit la posture ; le second
  est le compteur autoritatif*. Mise à jour du § 44 (l'asymétrie agent
  utilisateur) pour nommer `posture = "default_fallback"` comme le signal qui la
  rend enfin mesurable.
- `docs/operator/agent-identity-reprovision.md` § 1 : ligne ajoutée à la table
  des signaux, avec le fait que la ligne de posture est **par tour** et que sa
  disparition après `mika agents reprovision` est le signal de réparation
  (aucun redémarrage requis).
- `docs/configuration.md` § ~561 : le quatrième nom à côté des trois existants.
- `scripts/sync-agent-docs.sh` pour la copie crate-locale
  (`crates/mika-agent/docs/configuration.md`) — le job CI `docs-sync` échoue
  sinon.
- `crates/mika-agent/CLAUDE.md` : **rien**. Aucune variable d'environnement,
  aucune surface opérateur nouvelle ; le § *Audit Log* / *Observability* renvoie
  déjà à `docs/agent-internals/observability.md`, et le cap de taille des
  `CLAUDE.md` est une contrainte dure.

---

## 4. Contrat de vérification

| # | commande | attendu |
|---|---|---|
| V1 | `cargo build -p mika-agent` | vert |
| V2 | `cargo test -p mika-agent prompt::tests::mika2784 -- --nocapture` | 5 tests verts |
| V3 | `cargo test -p mika-agent prompt::tests::mika2027` | 4 tests existants verts, **assertions inchangées** |
| V4 | `cargo test -p mika-agent --test eval test_fail_closed_tool_surface_2784` | 2 tests verts |
| V5 | `cargo test` | aucune régression (~3463 tests) |
| V6 | `cargo clippy --all-targets -- -D warnings` | vert |
| V7 | `cargo fmt --check` | vert |
| V8 | `make verify-bundled-skills` | vert (aucune skill touchée, mais le gate est pré-merge) |
| V9 | `scripts/verify-pipeline.sh origin/main` | vert |
| V10 | `scripts/check-byte-slices.sh`, `scripts/check-suite-wiring.sh` | verts |

**V11 — contrôle d'anti-vacuité, exécuté à la main avant d'ouvrir la PR.**
Chaque test nouveau doit avoir été vu **rouge** une fois, en inversant
localement ce qu'il tient, puis rétabli :

| test | mutation qui doit le faire rougir |
|---|---|
| les trois tests de cause | retirer le `tracing::error!` de `fail_closed_identity` |
| `..._le_regime_nominal_est_muet` | rendre la ligne inconditionnelle |
| `..._nest_pas_fail_closed` | faire retourner `fail_closed_identity()` à la branche agent utilisateur |
| `..._la_posture_ne_declare_aucune_allowlist_doutils` | poser `allowed: Some(vec![])` |
| `..._ne_recoit_aucun_builtin_mutationnel` | vider la dénylist de `fail_closed_identity` |
| `..._le_meme_builtin_est_servi` | retirer `create_task` de `default_tools()` |

Le résultat de V11 est reporté **dans le corps de PR**, test par test. Un test
jamais vu rouge est une décoration.

---

## 5. Definition of Done

1. V1–V10 verts, V11 exécuté et reporté test par test dans le corps de PR.
2. `allowed` vaut toujours `None` dans `fail_closed_identity()` — **le diff ne
   doit pas y toucher**. Toute modification de ce champ sort du périmètre réveillé
   (§ 0) et est un échec de revue.
3. Aucun des trois `event` existants (`identity_toml_absent`,
   `identity_toml_unreadable`, `identity_toml_malformed`) n'est renommé ni
   supprimé (D3) : le runbook et trois sites de documentation les citent.
4. Les quatre tests `mika2027_*` passent avec leurs assertions **inchangées**.
5. Documentation du commit 4 livrée, `scripts/sync-agent-docs.sh` exécuté, job
   `docs-sync` vert.
6. Corps de PR : `Refs #2784`, **jamais** `Closes #2784` (§ 0). Il porte la
   ligne `Code review: delegated to mika-qa (mika#2694)`.
7. Corps de PR : une section nommant explicitement ce qui **reste** dormant — la
   question 1 (`send_message` en fail-closed), la question 2 (agent utilisateur
   au fichier absent), et le resserrement de `allowed` — et le fait que leur
   condition est une décision écrite de Vincent sur mika#2784.
8. Un commentaire posé sur mika#2784 après merge, enregistrant que le morceau 1
   a atterri, nommant la sonde (`grep agent_fail_closed_posture
   "$MIKA_SPIRIT_LOG_FILE"`), et rappelant que le morceau 2 attend la décision de
   doctrine.
9. **Sonde post-déploiement**, écrite dans le corps de PR : l'instrument n'est pas
   vivant avant `make deploy` (classe mika#2340). Après déploiement, le régime
   nominal donne **zéro** ligne ; l'absence de la ligne sur un agent dont un
   `identity_toml_*` est présent signifie **binaire antérieur à ce changement**,
   jamais « posture saine ».

---

## 6. Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria`. Les
critères ci-dessous sont dérivés du § *Périmètre proposé* du ticket, restreint au
morceau 1 par sa propre section de réveil (§ 0).

- **AC1** — Une ligne de journal nomme la posture fail-closed **au moment où elle
  est servie**, sous l'`event` `agent_fail_closed_posture`, émise depuis
  `fail_closed_identity()` et non depuis ses appelants.
- **AC2** — Cette ligne porte l'agent et la **cause**, la cause prenant trois
  valeurs disjointes (`missing_file`, `unreadable`, `parse_error`) — une par
  entrée réelle du chemin, aucune fusion de populations (mika#2205).
- **AC3** — Le régime nominal (un `identity.toml` valide) n'émet **aucune** ligne
  de posture, et un test le tient.
- **AC4** — Un agent **utilisateur** dont le TOML est invalide, qui n'est **pas**
  fail-closed, n'émet aucune ligne de posture, et sa ligne
  `identity_toml_malformed` porte `posture = "default_fallback"` : les deux
  populations de § 1.2 (b) deviennent soustractibles.
- **AC5** — Un test comportemental lit le **tableau d'outils réellement servi au
  modèle** pour un agent fail-closed, et atteste que `create_task` en est absent
  et `send_message` présent.
- **AC6** — Ce test est accompagné d'un **contrôle négatif** avec une identité
  valide servant le même `create_task`, sans lequel l'absence de AC5 serait
  satisfaite de façon vacue.
- **AC7** — `tools.allowed` est **épinglé** à `None` par une assertion dont le
  message nomme mika#2784 morceau 2 : le resserrement futur devient un changement
  **visible** d'un test, et non une mutation silencieuse d'un champ que personne
  ne lit.
- **AC8** — Aucune posture n'est modifiée par ce vol. `allowed`, la dénylist, le
  sentinelle de skills et `[context.summary].inject` sont inchangés.
- **AC9** — Les trois `event` existants sont conservés à l'identique, et la table
  des signaux du runbook `agent-identity-reprovision.md` porte le quatrième.

---

## Fire-Disposition

Ce plan livre des détecteurs : les six tests unitaires du commit 2 et les deux
tests eval du commit 3. Option retenue : **(a) exception nommée en allowlist —
avec une allowlist livrée VIDE**, et c'est démontrable plutôt que déclaratif.

**Pourquoi zéro exception est le compte exact.** Un détecteur a besoin d'une
exception quand il peut rencontrer une violation **préexistante**. Les huit tests
d'ici n'en rencontrent aucune, pour deux raisons distinctes :

- cinq d'entre eux portent sur la ligne de journal, qui **n'existe pas avant ce
  plan** : il n'y a pas de corpus antérieur où une violation pourrait se cacher ;
- les trois autres (`..._ne_recoit_aucun_builtin_mutationnel`, son contrôle
  négatif, `..._nest_pas_fail_closed`) attestent le comportement **actuel** de
  `main` — la dénylist mord déjà, `send_message` est déjà servi, l'agent
  utilisateur retombe déjà sur les défauts. Ils atterrissent **verts**, et c'est
  V11 (§ 4) qui empêche cette verdeur d'être vacue, en exigeant que chacun ait
  été vu rouge sous une mutation nommée.

**La seule candidate à une exception, nommée plutôt que passée sous silence.**
`mika2784_la_posture_ne_declare_aucune_allowlist_doutils` épingle `allowed ==
None`, c'est-à-dire précisément l'état que le morceau 2 est censé **changer**. Ce
n'est pas une exception mais une **assertion auto-nettoyante à expiration
explicite** : elle est conçue pour rougir le jour où le morceau 2 atterrit, et son
message d'échec porte le ticket (mika#2784), le numéro du morceau, et les deux
questions de doctrine (1 et 2) dont la réponse écrite est sa condition de
levée. Un implémenteur du morceau 2 qui la voit rouge lit *quoi décider*, pas
*quoi contourner* — et il ne peut pas la rendre verte sans supprimer ou réécrire
une assertion qui nomme la décision, ce qui est visible en diff.

**Aucun livrable désarmé, aucune halte.** Les huit tests atterrissent armés, dans
la suite par défaut (`cargo test`), sans `#[ignore]` et sans `cfg` conditionnel.
Le seul `#[serial_test::serial]` posé est une contrainte d'ordonnancement imposée
par la capture `tracing` (mika#2646), pas un désarmement : les tests s'exécutent
bien, simplement pas en parallèle.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/prompt.rs` — `FailClosedCause`, ligne de posture, 3 sites d'appel, champ `posture` (commit 1) | 70 |
| `crates/mika-agent/src/prompt.rs` — 6 tests unitaires + helper de capture (commit 2) | 105 |
| `crates/mika-agent/tests/eval/test_fail_closed_tool_surface_2784.rs` (commit 3) | 80 |
| `crates/mika-agent/tests/eval.rs` — déclaration du module | 1 |

Total estimé : 256 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 256 passe sous les deux bornes — 1000 par PR et
279 par vol — avec ~23 lignes de marge sur la plus serrée. Trois propriétés du
périmètre soutiennent cette estimation plutôt que de l'espérer :

- **un seul fichier de production est touché**, et `fail_closed_identity` n'y a
  que trois appelants, tous dans ce même fichier. Pas de propagation de
  signature au-delà du module, aucun site externe à adapter (la fonction est
  privée) ;
- **aucun changement de comportement**, donc aucun test existant à réécrire : la
  DoD 4 exige explicitement que les quatre `mika2027_*` passent avec leurs
  assertions intactes. Les refontes de suite sont la source habituelle de dérive
  entre estimé et réel ;
- **les deux fichiers de test suivent des gabarits mesurés** :
  `compaction_handsoff.rs:340-490` pour la capture `tracing` (helper de 15 lignes,
  tests de 15 à 25) et `test_mika_arch_tool_allowlist_2748.rs` pour l'eval
  (102 lignes pour deux tests + un helper, dont ~30 de doc-comment de module).
  Le commit 3 est le même gabarit avec un `WITHHELD` différent.

Le risque résiduel assumé est V11 : six mutations à appliquer puis rétablir, en
tours plutôt qu'en lignes. Elles sont séquentielles, bornées à un `cargo test`
ciblé chacune (jamais la suite complète), et la marge de 23 lignes sous la borne
est là pour elles.
