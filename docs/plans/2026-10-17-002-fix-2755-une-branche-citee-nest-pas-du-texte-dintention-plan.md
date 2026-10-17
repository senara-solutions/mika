# mika#2755 — Une branche citée n'est pas du texte d'intention (vol 1)

> Plan content-only produit par `/mika-groom-plan-only`. Ticket : senara-solutions/mika#2755.
> Labels portés : `bug`, `p2-normal`, `ready`, `dispatch:loop`, `loop-substrate`.

---

## 1. Contexte : deux contraintes opposées sur le même tour, et une évasion

Le ticket décrit un `check_suite` vert du 2026-10-16 ≈ 23:49Z qui a coûté
13 étapes LLM à mika-dev et laissé une tâche `in_progress` sans process ni
enfant. La chaîne est reproductible site par site.

### 1.1 Le match

Le gateway émet, pour un `check_suite`, un unique gabarit
(`crates/mika-gateway/src/github.rs:554`) :

```rust
format!("[GitHub] Check suite {conclusion} on {repo_name} (branch: {branch})")
```

Le texte reçu est donc :

```
[GitHub] Check suite success on senara-solutions/mika (branch: fix/2750/dispatch-un-groom-ready-label-qui-crashe)
```

`match_skills` (`crates/mika-agent/src/skills/matcher.rs:118`) lowercase ce texte
entier et le soumet à l'alternation de chaque skill. Le mot-clé `groom` de
`skills/bundled/dev-groom/skill.toml` produit le motif `\bgroom\b` — et dans
`…-un-groom-ready-…` les deux voisins sont des tirets, donc **des non-word
chars : les deux bornes `\b` sont satisfaites**. La word-boundary de mika#1878
ne protège pas ici, et ce n'est pas un défaut de mika#1878 : le mot `groom` est
bel et bien un mot dans cette chaîne. Le défaut est que cette chaîne n'est pas
du texte.

dev-groom entre donc dans le tour avec `MatchReason::Keyword`, et
`collect_required_tools` (`agent_loop/mod.rs:8188-8193`) n'armait jusqu'ici les
`required_tools` que sur ce motif-là (#463). L'exigence
`run_claude_pilot_groom` est posée.

### 1.2 La contradiction

Sur le même tour, une seconde contrainte du moteur est armée et dit l'inverse.
`INTENT_GUARDS` porte `webhook_no_unauthorized_dispatch`
(`agent_loop/mod.rs:10271`), dont le prédicat satisfait est
(`agent_loop/mod.rs:10483`) :

```rust
!summaries.iter().any(|s|
    (s.name == "run_claude_pilot" || s.name == "run_claude_pilot_groom") && s.success)
```

c'est-à-dire : sur un `[GitHub]` qui n'est pas un `Issue labeled ready`, appeler
`run_claude_pilot_groom` **avec succès** est une violation (mika#841 couche 1,
trois incidents documentés #798 / #838 / #910).

Le tour était donc tenu par deux exigences incompatibles : *appelle cet outil*
et *n'appelle pas cet outil*. Le `WARN … missing_names=["run_claude_pilot_groom"]`
du ticket est la première qui tire ; la tâche manuelle
`Groom mika#2750 (retry — engine correction)` en `action_type=none`,
`status=in_progress`, est **l'évasion** — la seule sortie qui ressemble à une
action de grooming sans franchir l'interdit. Elle n'a ni process ni enfant
callback, d'où sa forme de vol bloqué que le reaper de mika#2750 ne ramasse pas
(son prédicat lit le couple parent/callback, et il n'y a pas de callback).

### 1.3 Ce que la mesure établit, et ce qu'elle n'établit pas

Elle établit **la cause de cette instance** : une donnée citée par un événement
a été lue comme une intention. Elle n'établit pas que le masquage de cette
donnée ferme toute la classe — un commentaire GitHub contenant le mot « groom »
rejouerait la même contradiction par une autre porte. C'est pourquoi ce ticket
se découpe en deux vols (§ 2.3) : celui-ci ferme la cause mesurée et rend l'AC
du ticket mot pour mot ; le second ferme la contradiction elle-même.

---

## 2. La décision

### 2.1 Le principe : la grammaire d'un événement, pas ses données

Un texte de `format_event_text` est composé de deux matières :

- une **forme** — `[GitHub] Check suite success on`, `[GitHub] PR opened:` — qui
  dit ce qui s'est passé. C'est elle que les skills-handlers lisent : les
  keywords de `self-dev-webhook-ci` (`"check suite"`), de
  `self-dev-webhook-ready-label` (`"issue labeled ready"`), de
  `self-dev-webhook-qa` (`"pr review"`) portent tous sur la forme ;
- des **données citées** — un slug de branche, un numéro, un login — qui
  identifient l'objet. Un slug de branche est dérivé mécaniquement d'un titre de
  ticket par `scripts/derive-branch-name` ; personne ne l'a écrit pour être lu.

La décision tient en une phrase : **une branche citée par un événement webhook
n'est pas du texte d'intention, et le matcher de keywords ne doit pas la voir.**

C'est la piste 1 du ticket dans sa forme bornée (« ou au moins pas contre le nom
de branche qu'il cite »). Le périmètre est délibérément la branche, pas « tout
texte d'événement » : `qa-review` déclare
`keywords = ["review", "pr", "qa", "pull request"]` **et**
`required_tools = ["qa_pr_view", "run_gh", "run_shell", "build_mika"]`. Son
armement nominal vient du gabarit `[GitHub] PR review (…) on …`, c'est-à-dire de
la forme. Désarmer les `required_tools` sur « tout événement webhook » — la
piste 2 prise au pied de la lettre — désarmerait le chemin de revue de mika-qa.
Le ticket propose deux pistes ; la mesure en élimine une.

### 2.2 Ce que le masquage ne fait PAS : il ne désarme pas dev-groom

Point qui conditionne l'innocuité du vol. `dev-groom` est déclaré dépendance de
`self-dev` (`skills/bundled/self-dev/skill.toml:26`), et `self-dev` est
`always_on`. Sur un tour webhook, dev-groom continue donc d'arriver dans
l'ensemble matché — en `MatchReason::Dependency`, exactement comme le fait déjà
le tour ready-label épinglé par
`matcher.rs::test_webhook_ready_label_loads_dev_groom_via_dependency` (ligne
457). Son prompt reste injecté, `run_claude_pilot_groom` reste dans le tool map
du tour, et le dispatch ready-label — qui est de toute façon structurel depuis
mika#1384 — n'est touché par rien.

Le vol ne retire pas une capacité : il remet une provenance à l'endroit. Le seul
effet observable est que `collect_required_tools`, qui ne lit que `Keyword`, ne
voit plus un match né d'un slug.

### 2.3 Le découpage en vols

| vol | affirmation | état |
|---|---|---|
| **vol 1 (ce plan)** | une branche citée n'est pas du texte d'intention | ce plan |
| vol 2 | `required_tools` et `webhook_no_unauthorized_dispatch` ne peuvent pas se contredire sur un même tour : l'ensemble d'outils que la seconde interdit est lu au même site par la première, qui s'y désarme et le logue | `Tracked in:` ci-dessous |

Tracked in: senara-solutions/mika#2755 (vol 2 — le désarmement de la contradiction `required_tools` ↔ `webhook_no_unauthorized_dispatch`)

Chaque vol est mergeable seul. Le vol 1 rend l'AC minimal du ticket mot pour
mot et ne dépend de rien. Le vol 2 est une généralisation qui reste exacte que
le vol 1 soit livré ou non — il lit l'ensemble `{run_claude_pilot,
run_claude_pilot_groom}` au seul site qui le définit aujourd'hui
(`webhook_no_unauthorized_dispatch_satisfied`) plutôt que d'en écrire une
seconde copie, qui serait la divergence programmée que ce dépôt a déjà payée
deux fois (mika#2158, mika#2362).

---

## 3. Les livrables, site par site

### D1 — `crates/mika-common/src/github_event_format.rs` : la grammaire possède son propre masque

Le module est aujourd'hui purement déclaratif — il porte `BRANCH_MARKER`,
`CHECK_SUITE_EVENT_PREFIX` et les dix autres préfixes, et son doc-module dit
explicitement pourquoi ils vivent là : « un `starts_with(CONST)` ne porte pas de
littéral, donc la garde des jetons canoniques (mika#2201) ne voit aucun nouveau
site de match ». La fonction de masquage consomme exactement ces constantes et
n'a de sens qu'avec elles : elle appartient à ce module, pas au matcher.

**D1a — une constante.** `GITHUB_EVENT_TEXT_PREFIX: &str = "[GitHub] "`, le
préfixe que les douze formes déclarées partagent, y compris l'attrape-tout
`[GitHub] {event_type}.{action} on {repo}`. Doc-comment nommant le seul rôle
qu'elle tient : décider si un texte est un événement webhook avant d'y toucher.

**D1b — une fonction pure.**

```rust
pub fn mask_cited_branch(text: &str) -> Cow<'_, str>
```

Contrat, à écrire dans le doc-comment :

1. **Trois refus, tous fail-safe vers le comportement actuel** — le texte ne
   commence pas par `GITHUB_EVENT_TEXT_PREFIX` ; sa première ligne ne contient
   pas `BRANCH_MARKER` ; sa première ligne ne se termine pas par `)`. Dans ces
   trois cas : `Cow::Borrowed(text)`, rien n'est masqué, aucun comportement ne
   change. Un masque qui se trompe de périmètre coûte plus cher que l'absence
   de masque.
2. **Le découpage.** La branche se lit entre `BRANCH_MARKER` et le `)` qui ferme
   la **première ligne** — le même idiome que
   `webhook_prefilter::check_suite_branch` (`find` + `strip_suffix(')')`), et la
   raison est la même : les deux seuls gabarits qui citent une branche la
   placent en fin de première ligne (`check_suite` fin de texte, `pull_request`
   avant le `\n{url}`).
3. **`find`, pas `rfind`, et c'est délibéré.** Un titre de PR qui contiendrait
   lui-même ` (branch: ` ferait masquer le titre *et* le slug. C'est le sens
   fail-safe : on masque plus de données citées, jamais moins.
4. **Le slug seul est retiré, le marqueur reste.** Le texte rendu porte
   `… (branch: )`. Conserver le marqueur garde le texte lisible en debug et
   n'ouvre aucune nouvelle frontière ; un keyword `branch` — qu'aucun skill du
   dépôt ne déclare — resterait d'ailleurs satisfait avant comme après.
5. **Portée.** Le texte rendu ne sert qu'à l'évaluation des keywords. Aucun autre
   consommateur ne le voit : `context::resolve_contexts`,
   `review_filter::apply_review_filter`, les `INTENT_GUARDS` et le prompt
   continuent de lire `params.user_message` intact.

### D2 — `crates/mika-agent/src/skills/matcher.rs` : un seul site d'application

En tête de `match_skills`, avant le `to_lowercase()` :

```rust
let intent = mika_common::github_event_format::mask_cited_branch(user_message);
let message_lower = intent.to_lowercase();
```

Le reste de la fonction est inchangé. Le placement **dans** `match_skills`, et
non chez ses deux appelants (`run_agent` ligne 5452, `run_silent` ligne 7552),
est le point de la décision : les deux modes en héritent, et un troisième
appelant futur aussi. C'est la différence entre un invariant et une convention.

Doc-comment à ajouter sous le paragraphe « Word-boundary matching (mika#1878) » :
une section « Données citées (mika#2755) » qui nomme le cas mesuré, dit que la
word-boundary ne pouvait pas le tenir (`groom` est un mot entre deux tirets), et
renvoie à `mask_cited_branch` pour le contrat.

### D3 — `scripts/canonical-tokens.tsv` : une ligne

`GITHUB_EVENT_TEXT_PREFIX` est une constante nommée `*_PREFIX` portant le
littéral `[GitHub] ` : le survey l'émettra, et
`canonical_tokens::tests::mika2201_every_match_site_is_declared` exige qu'elle
soit déclarée. Une ligne, alignée sur les douze voisines :

```
[GitHub]	B	crates/mika-common/src/github_event_format.rs::GITHUB_EVENT_TEXT_PREFIX	exact:prefix
```

Le site consommateur de D2 n'en demande aucune : il lit par constante, sans
littéral.

### D4 — tests de `mask_cited_branch` (`mod tests` neuf dans D1)

Le module n'a pas de `#[cfg(test)]` aujourd'hui ; ce vol lui en donne un. Sept
cas, chacun pour une affirmation distincte :

1. le message du ticket **mot pour mot** → le slug a disparu, `groom` n'est plus
   dans le texte rendu ;
2. gabarit `pull_request` avec `\n{url}` en seconde ligne → le slug a disparu,
   **l'URL est intacte** (le masque s'arrête à la première ligne) ;
3. un texte humain commençant par `groom mika#2750 (branch: x)` → `Borrowed`,
   rien n'est masqué (refus 1) ;
4. un `[GitHub]` sans `BRANCH_MARKER` (`issue_comment`) → `Borrowed` (refus 2) ;
5. une première ligne tronquée ne finissant pas par `)` → `Borrowed` (refus 3) ;
6. titre de PR contenant lui-même ` (branch: ` → le masque va jusqu'au `)`
   final, donc il mange le titre : le sens fail-safe de `find`, épinglé plutôt
   que subi ;
7. slug accentué / multi-octets → aucun panic, aucun découpage au milieu d'un
   caractère (la garde `byte-slice-lint` du dépôt, #764, vit sur cette classe).

### D5 — tests de `match_skills` (`matcher.rs`)

Trois cas, dont deux contrôles négatifs sans lesquels le premier ne prouve rien :

1. **le cas du ticket** — scène `self-dev`(always_on, dépend de dev-pilot et
   dev-groom) + `dev-pilot` + `dev-groom`(keywords `groom`, `groom ticket`), le
   message `check_suite` mot pour mot. Assertion : dev-groom est présent et sa
   `reason` est `Dependency`, **pas** `Keyword` ;
2. **le chemin nominal n'a pas bougé** — même scène, message `groom mika#2750` :
   dev-groom arrive en `Keyword` ;
3. **mika-qa n'a pas bougé** — scène `qa-review` (keywords `review`, `pr`, `qa`),
   message `[GitHub] PR review (approved) on senara-solutions/mika#2754 (titre) by @mika-qa` :
   `Keyword`. C'est l'assertion qui distingue ce vol de la piste 2 large.

### D6 — le test de l'AC (`agent_loop/mod.rs`, `mod tests`)

Le critère du ticket porte sur `required_tools`, pas sur `MatchReason` : il faut
donc épingler la chaîne complète, au site où elle décide.
`collect_required_tools` est une fonction libre du module, accessible depuis ses
tests inline, et le builder `make_skill_entry_with_constraints` (ligne 12337)
pose `keywords = [name]` — un skill nommé `"groom"` portant
`required_tools = ["run_claude_pilot_groom"]` reproduit la scène sans ajouter un
seul builder.

```
match_skills(&skills, MESSAGE_DU_TICKET)  →  collect_required_tools(…)  →  ∅
```

plus la contre-épreuve sur `"groom mika#2750"`, qui doit rendre
`{"run_claude_pilot_groom"}`. Un test qui ne rendrait que l'ensemble vide
passerait aussi sur un matcher cassé.

### D7 — le test cross-crate (`crates/mika-gateway/src/github.rs`, `mod tests`)

Voisin de `mika2680_chaque_prefixe_declare_est_celui_que_le_gateway_emet`, qui
fournit déjà `mika2680_blank_event()` et `mika2680_pr()`. Deux moitiés :

- **producteur → masque** : les textes réellement produits par
  `format_event_text` pour `check_suite` et `pull_request` perdent bien leur
  slug en passant par `mask_cited_branch` ;
- **l'assertion auto-nettoyante** : pour chacun des autres types d'événement que
  le gateway met en forme (`issues` incluant `labeled`, `issue_comment`,
  `pull_request_review`, l'attrape-tout `push`), le texte produit ne contient
  **pas** `BRANCH_MARKER`. Le jour où un troisième gabarit se met à citer une
  branche, ce test rougit et nomme le gabarit — c'est la disposition de tir du
  § suivant.

---

## Fire-Disposition

Le vol livre des tests et des assertions, dont le chemin de succès est « aucune
violation trouvée » : le gate s'applique. **Option (a) — exception nommée en
allowlist — et l'allowlist est vide par construction.**

- **Violations existantes : zéro, et c'est vérifiable avant d'écrire une ligne.**
  Le seul détecteur qui balaie un corpus est D7, et son corpus est l'ensemble des
  bras de `format_event_text` — six bras, inspectés : seuls `pull_request`
  (ligne 505) et `check_suite` (ligne 554) écrivent `(branch: …)`. Aucune entrée
  d'exception n'est donc écrite, et aucune ne doit l'être : « on déclare, on
  n'allowliste pas » (`canonical_tokens.rs`, § du même nom).
- **L'assertion auto-nettoyante est la seconde moitié de D7.** Elle n'est pas
  décorative : elle est la seule chose qui empêche le masque de devenir faux en
  silence. Si un septième bras du gateway se met demain à citer une branche —
  un `workflow_run`, un `deployment_status` — D7 rougit en nommant le gabarit,
  et le remède est d'étendre `mask_cited_branch`, pas d'ajouter une exception.
  Un masque incomplet ne rend aucune décision fausse : il rend un slug à
  nouveau visible, exactement la régression que ce vol ferme, et sans ce test
  elle repasserait toutes les autres assertions au vert.
- **D4, D5, D6 ne sont pas des détecteurs à corpus** : ce sont des assertions de
  comportement sur des entrées qu'elles construisent elles-mêmes. Elles n'ont
  rien à allowlister et ne reçoivent pas d'entrée.
- **Les deux gardes existantes du dépôt que ce vol réveille** —
  `canonical_tokens` (D3) et `byte-slice-lint` (#764, couvert par D4 cas 7) —
  sont livrées armées et satisfaites par le vol lui-même. Aucune n'est
  désarmée, aucune n'est exemptée.

Aucun livrable n'atterrit avec `#[ignore]`, et rien ne remonte à l'opérateur.

---

## 4. Contrat de vérification

| # | affirmation | comment elle est tenue |
|---|---|---|
| V1 | le message du ticket n'arme plus `run_claude_pilot_groom` | D6, cas 1 — `collect_required_tools` rend `∅` sur le message mot pour mot |
| V2 | …et le chemin nominal du grooming est intact | D6, cas 2 — `"groom mika#2750"` rend `{"run_claude_pilot_groom"}` |
| V3 | dev-groom reste joignable sur le tour webhook | D5, cas 1 — `reason == Dependency`, le prompt et le tool map sont inchangés |
| V4 | mika-qa n'est pas désarmé | D5, cas 3 — `qa-review` en `Keyword` sur un `PR review` |
| V5 | le masque ne touche que ce qu'il doit | D4, cas 2 à 7 — trois refus, l'URL intacte, le fail-safe de `find`, l'UTF-8 |
| V6 | le masque suit le producteur | D7 — textes réels de `format_event_text`, et l'assertion auto-nettoyante sur les autres gabarits |
| V7 | l'inventaire des jetons reste exhaustif | D3 + `cargo test -p mika-agent canonical_tokens` |

Commandes, dans l'ordre :

```
cargo test -p mika-common github_event_format
cargo test -p mika-agent --lib skills::matcher
cargo test -p mika-agent --lib agent_loop
cargo test -p mika-agent --lib canonical_tokens
cargo test -p mika-gateway --lib github
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
scripts/verify-pipeline.sh origin/main
```

**Rouge-avant exigé pour V1.** Le test de D6 doit être vu échouer sur l'arbre
non modifié — c'est-à-dire avant D2. Un test d'AC qui n'a jamais été rouge
n'atteste de rien : il pourrait passer parce que la scène est mal construite
(mauvais keyword, skill mal nommé) plutôt que parce que le masque fonctionne.
L'ordre de commit du § 5 rend ce rouge-avant mécanique.

---

## 5. Ordre de commit

1. **D1 + D4** — la fonction pure et ses sept cas. Compile et passe seule ;
   aucun comportement du moteur n'a bougé.
2. **D6 seul, attendu ROUGE** — le test d'AC sur l'arbre où `match_skills` ne
   masque encore rien. Constater le rouge et le reporter dans le corps de PR.
3. **D2 + D3** — l'application dans `match_skills` et la ligne TSV. D6 passe au
   vert ; c'est le commit qui porte le correctif.
4. **D5 + D7** — les contrôles négatifs et le test cross-crate.
5. Suite complète, clippy, fmt, `verify-pipeline.sh`.

Le plancher de ce vol est l'étape 3 : en deçà, rien n'est corrigé. Les étapes 4
et 5 ne sont pas facultatives pour autant — sans D5 cas 3, le vol n'a pas montré
qu'il ne désarmait pas mika-qa.

---

## 6. Risques

| risque | évaluation |
|---|---|
| un skill du dépôt dépendait d'un keyword présent **uniquement** dans un slug | inventorié avant d'écrire : les keywords des sept skills-handlers webhook (`self-dev-webhook-ci` `"check suite"`, `-ready-label` `"issue labeled ready"`, `-qa` `"pr review"`, `-callback` `"callback"`, `qa-review` `"review"`/`"pr"`, `self-dev` `"implement"`, `resolve-pr-conflicts` `"rebase"`) portent tous sur la **forme** du gabarit, jamais sur une branche. V4 épingle le cas le plus coûteux |
| un humain colle un texte commençant par `[GitHub] ` | le masque s'applique alors à un message humain. Conséquence bornée : un slug de branche cité dans ce collage cesse de déclencher un skill — ce qui est le comportement voulu. Risque nommé plutôt que réduit : le distinguer demanderait un canal de provenance que le moteur n'a pas (le `channel_type` des webhooks est `"a2a"`, comme tout appel `message/send`) |
| le découpage `find` mange un titre de PR | délibéré et épinglé (D4 cas 6). Fail-safe : masquer plus de données citées, jamais moins |
| `(branch: )` vide perturbe un consommateur aval | impossible par construction : le texte masqué ne quitte pas `match_skills`. Épinglé par le fait qu'aucun autre site ne reçoit la `Cow` |
| la tâche fantôme déjà en base | hors périmètre code, § 7 |

---

## 7. Hors périmètre

- **La tâche fantôme existante** (`Groom mika#2750 (retry — engine correction)`,
  `in_progress`, `action_type=none`). C'est une donnée de production, pas un
  défaut de code : ce vol l'empêche de renaître, il ne la ferme pas. Elle doit
  être passée en terminal à la main par l'opérateur ; aucun reaper existant ne
  la voit (celui de mika#2750 raisonne sur le couple parent/callback et elle n'a
  pas d'enfant, celui de mika#1712 exige `process_id IS NULL`). À mentionner
  dans le corps de PR pour que le geste ne se perde pas.
- **Le désarmement de la contradiction** `required_tools` ↔
  `webhook_no_unauthorized_dispatch` — vol 2, nommé au § 2.3.
- **Le titre de ticket comme donnée citée.** Le même raisonnement s'appliquerait
  au `— {titre}` des gabarits `issues` et `pull_request`, mais c'est une
  population beaucoup plus large (un titre *est* du texte humain, contrairement
  à un slug) et le ticket ne la nomme pas. Un masque de titre désarmerait
  `qa-review` sur un `PR opened` dont le titre dit « review ». Ne pas y toucher
  sans mesure.
- **Le pré-filtre webhook** (`webhook_prefilter`). Un `check_suite` vert sur une
  branche de PR n'est pas dans sa population ; l'élargir est un autre ticket.

---

## Definition of Done

- [ ] `mask_cited_branch` et `GITHUB_EVENT_TEXT_PREFIX` livrés dans
      `crates/mika-common/src/github_event_format.rs`, avec le contrat des trois
      refus écrit dans le doc-comment.
- [ ] `match_skills` applique le masque en un seul site, avant le
      `to_lowercase()`, et son doc-comment porte la section « Données citées
      (mika#2755) ».
- [ ] La ligne `GITHUB_EVENT_TEXT_PREFIX` est déclarée dans
      `scripts/canonical-tokens.tsv`.
- [ ] D4 (7 cas), D5 (3 cas), D6 (2 cas), D7 (2 moitiés) sont livrés et verts.
- [ ] Le rouge-avant de D6 a été constaté à l'étape 2 de l'ordre de commit et
      est reporté dans le corps de PR.
- [ ] `cargo test`, `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` passent.
- [ ] Le corps de PR nomme le résidu opérationnel du § 7 (la tâche fantôme à
      fermer à la main) et la ligne `Tracked in:` du vol 2.
- [ ] `Code review: delegated to mika-qa (mika#2694)` figure dans le corps de PR.

## Acceptance criteria

Transcrit du § « Critère d'acceptation minimal » du ticket :

- **AC1** — Un événement `check_suite` sur une branche dont le slug contient
  `groom` ne produit **ni `required_tools` actif**, **ni re-prompt**, **ni tâche
  manuelle**.
- **AC2** — Un test épingle le cas avec le message du ticket **mot pour mot** :
  `[GitHub] Check suite success on senara-solutions/mika (branch: fix/2750/dispatch-un-groom-ready-label-qui-crashe)`.

Correspondance avec le contrat de vérification : AC1 est tenu par V1 — l'ensemble
`required_tools` rendu vide est la cause des deux autres termes (sans exigence
active, la garde de `agent_loop/mod.rs:2165` ne peut pas re-prompter, et sans
re-prompt le modèle n'est pas poussé vers l'évasion qui a créé la tâche
manuelle). AC2 est tenu par D6 cas 1 et D4 cas 1, qui portent tous deux le
message littéral.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-common/src/github_event_format.rs` (D1 : const + `mask_cited_branch` + doc-comments) | 58 |
| `crates/mika-agent/src/skills/matcher.rs` (D2 : appel + doc) | 22 |
| `scripts/canonical-tokens.tsv` (D3) | 1 |
| tests (D4 : `mod tests` neuf dans `github_event_format.rs`, 7 cas) | 85 |
| tests (D5 : `matcher.rs`, 3 cas) | 55 |
| tests (D6 : `agent_loop/mod.rs`, 2 cas) | 45 |
| tests (D7 : `mika-gateway/src/github.rs`, cross-crate) | 50 |

Total estimé : 316 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 316 < 429 et 316 < 1000 : les deux
bornes passent, la plus serrée de 113 lignes. La marge est réelle parce que la
production ne pèse que 81 lignes — une constante, une fonction pure de ~25
lignes de corps, et un appel de deux lignes. Les six septièmes du volume sont du
test, et chaque fichier de test a son compte de cas arrêté dans le plan (7, 3,
2, 2), pas laissé au jugement du pilote.

Le poste qui pourrait déborder est D4. Sa borne de sécurité : **sept cas, un
helper de construction de message, aucun second helper.** Si le fichier dépasse
nettement 85 lignes, c'est que chaque cas s'est mis à reconstruire sa scène —
le remède est de factoriser la construction, jamais de retirer un cas. Les cas
3 à 5 sont les trois refus du contrat et les retirer viderait le fail-safe de
son attestation ; le cas 7 est exigé par la classe que `byte-slice-lint` (#764)
surveille.

Le découpage en deux vols est ce qui fait tenir ce total : le désarmement de la
contradiction `required_tools` ↔ `webhook_no_unauthorized_dispatch` (§ 2.3) est
estimé à lui seul autour de 140 lignes, ce qui porterait l'ensemble à ~456,
au-dessus de la borne. Il part en vol 2.
