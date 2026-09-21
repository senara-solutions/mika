# mika#2419 — la porte QA doit hériter de l'exclusion d'auteur automatique, pas du label

- **Ticket :** senara-solutions/mika#2419
- **Type :** fix (substrat de porte)
- **Priorité :** p2, tier-2
- **HEAD de lecture :** `7c6f787b` (le ticket a été fiché à `455b2706`)

---

## Contexte

### Le symptôme mesuré

PR mika#2415 (dependabot, bump `Cargo.lock` seul) :

- label `pipeline-exempt` posé ~07:05Z ;
- check CI **`Pipeline Artifacts` = vert** ;
- verdict `mika-platform-qa` à **07:36:55Z = `block[pipeline]`**, REASON
  « Dependabot Cargo.lock-only PR : missing plan document in docs/plans/ ».

Une PR de dépendance légitimement exemptée reste bloquée et exige un geste
opérateur à chaque bump. Le symptôme est réel et le ticket a raison de le ficher.

### Quatre affirmations du ticket que la lecture du code réfute

**C'est le livrable principal de ce grooming.** Le correctif littéral demandé
— « le rôle QA doit refléter la même exemption que la CI : label `pipeline-exempt`
⇒ pas de `block[pipeline]` » — est faux sur ses deux moitiés, et l'appliquer tel
quel élargirait une garde sur la foi d'une parité qui n'existe pas.

**R1 — La CI n'a jamais honoré le label sur cette PR : le job est entièrement
sauté.** `.github/workflows/ci.yml:336` porte
`!startsWith(github.head_ref, 'dependabot/')`, avec son motif écrit juste
au-dessus :

> « Dependabot n'écrit ni plan doc ni trailer `Pipeline-Exempt:` (mika#860) —
> activer dependabot.yml sans cette ligne a bloqué 11 PRs vertes par ailleurs
> (mika#2010). »

`verify-pipeline.sh` **n'a pas tourné** sur #2415. Le vert observé n'est pas le
vert d'un guard qui honore un label : c'est l'absence de guard. La prémisse
« la porte CI honore l'exemption » est donc fausse, et avec elle la notion de
« même exemption » que QA devrait refléter.

**R2 — Même exécuté, `pipeline-exempt` n'aurait rien exempté pour cette classe.**
L'en-tête du script (l.46-51) qualifie l'asymétrie de *load-bearing* :

> « The `documentation` issue label and the `pipeline-exempt` PR label exempt the
> source-required check **ONLY**. […] Only the `Pipeline-Exempt: code-only`
> trailer bypasses the code-only rejection. »

`Cargo.lock` n'est ni sous `docs/`, ni `.github/`, ni `.claude/worktrees/` : il
tombe dans `SOURCE_BUCKET`. Une PR `Cargo.lock`-seul est donc **code-only**
(`!docs && source`), classe que le label n'exempte pas — par conception, pour
préserver la protection mika-platform#17. Implémenter le correctif du ticket
ferait refléter à QA une exemption que la CI n'applique pas ici, c'est-à-dire
créerait la divergence que le ticket veut fermer, dans l'autre sens.

**R3 — `calibration/roles/mika_qa.rs` n'est pas la surface servie.** C'est le
harnais des 5 scénarios de `make calibrate-mika-qa`. Les lignes citées par le
ticket (157, 275, 345-350) sont des **prompts de fixtures inline** (« Include
DEPTH, REASON, DIFF ANALYSIS, and PLAN-AC VERIFICATION sections ») qui décrivent
au modèle *évalué* la réponse attendue d'un scénario. Les éditer changerait ce
qu'on **mesure**, jamais ce que mika-qa **fait**. La surface servie est
`skills/bundled/qa-review/system_prompt.md`, compilée dans
`BUNDLED_SKILL_MANIFESTS` par `build.rs`. Un correctif posé dans le harnais
serait inerte en production — classe mika#2340, dans sa variante la plus
trompeuse, puisque la suite de calibration passerait au vert.

**R4 — Le carve-out dependabot existe déjà, complet, et depuis 25 jours.**
`system_prompt.md` **Step 1.6** (mika#1729, PR #1995, mergé le 2026-08-26)
détecte la PR (`author == "dependabot[bot]"` ou `"app/dependabot"`), **saute
explicitement Step 2 et Step 2.5**, et substitue une revue de dépendance
(requête indépendante à la GitHub Advisory Database) que la CI ne fait pas. Le
`grep` du ticket ne l'a pas vu parce qu'il portait sur le fichier de
calibration. Et son discriminant est bien **lisible** : `qa_pr_view.sh:34`
expose `author` dans `SAFE_FIELDS` — ce n'est donc pas la classe du champ
inexploitable (`isDraft`, absent lui, et déjà fiché ailleurs).

### La racine réelle

La connaissance « un auteur automatique n'est pas soumis au gate plan » vit à
**deux endroits, dont aucun n'est le guard que QA exécute** :

| Porteur | Nature | Lu par la CI | Lu par QA |
|---|---|---|---|
| `ci.yml` `if: !startsWith(head_ref, 'dependabot/')` | exclusion de branche | ✅ | ❌ invisible |
| `system_prompt.md` Step 1.6 | routage LLM | ❌ | ⚠️ non déterministe |
| `scripts/verify-pipeline.sh` | **le guard exécuté** | ✅ | ✅ | → **0 occurrence de `dependabot`** |

Depuis mika#2172 (PR #2217, 2026-09-07), QA ne paraphrase plus la règle : il
**exécute** `verify-pipeline.sh` dans un worktree détaché, et la disposition de
Step 2C est pré-spécifiée, sans jugement — « Any guard exits non-zero →
`block[pipeline]` ». C'est un progrès (une prose dérive, un script ne dérive pas
de lui-même), mais il déplace la parité : QA hérite désormais fidèlement de
`verify-pipeline.sh`, et **de rien de ce que `ci.yml` décide au-dessus de lui**.

Chaîne causale de #2415, vérifiée ligne à ligne :

1. Step 1.6 aurait dû router → **raté** ;
2. Step 2 exécute le guard → `Cargo.lock` ⇒ code-only ⇒ `REJECT` exit 1 ;
3. Step 2C ⇒ `block[pipeline]`, sortie du guard citée verbatim.

Le REASON observé corrobore le point 1 : le modèle **nomme** « Dependabot
Cargo.lock-only PR » tout en appliquant la classe pipeline. Il avait
l'information et n'a pas routé — `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`,
la classe que mika#2120 a mesurée à neuf récurrences sous prompt contre zéro
quand la règle est portée par le substrat.

**Corollaire qui décide du remède :** renforcer Step 1.6 reproduirait exactement
le mode de panne. La règle doit descendre dans le seul artefact que les deux
portes exécutent.

### Ce qui rend le correctif suffisant

Step 2.5 est **déjà correct** depuis mika#2172 : sans callout de plan et guards
verts, il émet `PLAN-AC VERIFICATION: skipped` et porte l'interdiction explicite
« Do NOT emit `block[pipeline]` ». Donc un guard qui sort 0 fait disparaître le
blocage **même lorsque Step 1.6 est raté** — il n'y a pas de second gate à
réparer en aval.

---

## Requirements

- **R1.** `scripts/verify-pipeline.sh` n'applique pas ses deux rejets de bucket
  (docs-only, code-only) lorsque la PR est ouverte par un auteur automatique
  reconnu.
- **R2.** Le discriminant est l'**auteur** (`.pull_request.user.login` de
  `GITHUB_EVENT_PATH`), jamais le préfixe de branche.
- **R3.** **Fail-closed** : event absent, fichier illisible, champ absent ou
  login vide ⇒ **aucune exemption**. Un run local n'exempte donc jamais.
- **R4.** L'exemption est **visible** dans la sortie du script et nomme son
  mécanisme, comme les trois exemptions existantes.
- **R5.** Le guard exécuté par QA reçoit effectivement le champ d'auteur : le
  JSON synthétique de Step 2B le porte. Sans R5, R1 est inerte côté QA.
- **R6.** Test négatif obligatoire : dependabot + `Cargo.lock` seul ⇒ exit 0,
  **échouant avant le correctif**, plus un contrôle négatif prouvant que
  l'exemption est ciblée.
- **R7.** Aucune exemption existante n'est élargie, aucune protection retirée :
  l'asymétrie docs-only/code-only du label `pipeline-exempt` est laissée
  intacte.

---

## Approche / Conception

### Le point de changement : le guard, pas le prompt

Un seul lieu porte la règle, et c'est celui que les deux portes exécutent.
Conséquences :

- **CI** : comportement inchangé. Le job reste sauté par `ci.yml` ; s'il tournait,
  le script sortirait 0. Aucune PR aujourd'hui verte ne devient rouge.
- **QA** : hérite de l'exclusion **déterministiquement**, sans clause de prompt,
  y compris quand le modèle rate Step 1.6.

### Pourquoi l'auteur et non le préfixe de branche

`ci.yml` discrimine sur `github.head_ref` parce qu'une expression `if:` n'a rien
d'autre à sa disposition. Le script a mieux, et le préfixe est usurpable : un
humain qui nomme sa branche `dependabot/foo` franchirait le gate plan. Un
`user.login` ne s'usurpe pas. La liste retenue est `dependabot[bot]` et
`app/dependabot` — les deux formes que `gh` rend, déjà reconnues par Step 1.6.

### Portée de l'exemption : les deux checks de bucket, pas un `exit 0`

L'exemption est posée **après** le check de section `## Acceptance criteria`
(mika#1600) et **avant** les deux rejets de bucket. `ci.yml` saute tout, mais il
le fait parce qu'une expression `if:` n'a aucune granularité — pas parce que la
granularité serait indésirable. Une exemption doit être la plus étroite qui
répare le défaut mesuré : si une PR automatique portait un plan, sa section AC
reste vérifiée.

### Visibilité, et le refus documenté qui ne s'applique pas

Les lignes 52-62 du script refusent explicitement l'auto-exemption **par
chemin**, dont le premier motif est que « le vert silencieux à plusieurs chemins
d'exemption érode la visibilité structurelle ». Un relecteur peut légitimement
l'invoquer ici, donc il faut y répondre : l'exemption proposée n'est pas par
chemin mais par **auteur** (le classement, pas son artefact), et elle émet
`info: [pipeline-exempt: automated-author] …` sur le modèle des trois existantes
— l'opérateur continue de voir d'un coup d'œil quelle porte a laissé passer.

### La duplication de liste, assumée et bornée

`ci.yml` garde 3 préfixes de branche, le script porte 2 logins. Aucun lint de
parité n'est proposé : les deux surfaces répondent à deux questions distinctes
(« faut-il dépenser un runner ? » contre « cette PR est-elle soumise au
gate ? ») et comparer des préfixes de branche à des logins comparerait deux
natures. L'asymétrie de risque est ce qui rend l'arbitrage acceptable et elle
est nommée : script **plus strict** que `ci.yml` ⇒ faux `block` visible
immédiatement ; script **plus permissif** ⇒ dangereux, mais borné par une liste
explicite de deux logins qu'aucun chemin ne construit dynamiquement. Un
commentaire croisé est posé dans les deux fichiers.

### R5 : sans le champ, le correctif est inerte — et l'inertie serait invisible

Le JSON synthétique de Step 2B ne porte aujourd'hui que `number` et `labels` :

```
printf '%s' '{"pull_request":{"number":<number>,"labels":[…]}}' > "$W.ev"
```

Il faut y ajouter `"user":{"login":"<author>"}`, alimenté par le champ `author`
que `qa_pr_view` retourne déjà. **C'est la moitié qui compte** : sans elle, le
script apprend une règle que son unique appelant QA ne lui donne jamais les
moyens d'appliquer — classe mika#2205, un réglage qui n'atteint pas son lecteur.
Et cette inertie serait silencieuse : toute la suite de tests du script
resterait verte. D'où la garde structurelle de la phase 3.

---

## Phases d'implémentation

### Phase 1 — Le guard apprend l'auteur automatique

`scripts/verify-pipeline.sh` :

1. Étendre l'en-tête documentaire : quatrième mécanisme d'exemption, sa portée
   (les deux rejets de bucket), son caractère fail-closed, et le renvoi à
   `ci.yml` pour l'asymétrie des deux listes.
2. Après le bloc mika#1600 et avant le check docs-only, résoudre
   `AUTOMATED_AUTHOR` depuis `GITHUB_EVENT_PATH` via
   `jq -r '.pull_request.user.login // empty'`, en tolérant l'absence de fichier
   et l'échec de `jq` (⇒ chaîne vide ⇒ pas d'exemption).
3. Comparer à la liste `dependabot[bot]` / `app/dependabot` par égalité exacte
   (jamais par sous-chaîne : `not-dependabot[bot]` ne doit pas matcher).
4. Si match : émettre `info: [pipeline-exempt: automated-author] <login>: bucket
   checks skipped (mirrors ci.yml pipeline-artifacts branch exclusion, mika#2010)`
   et court-circuiter les deux blocs de rejet.

### Phase 2 — Le guard QA reçoit l'auteur

`skills/bundled/qa-review/system_prompt.md`, Step 2B :

1. Ajouter `"user":{"login":"<author>"}` au JSON synthétique, et `author` à la
   liste des champs extraits de `qa_pr_view` juste au-dessus.
2. Ajouter à la table de disposition 2C la ligne correspondante, pour que
   l'opérateur lise la sortie `automated-author` sans la confondre avec un
   `GUARD-ABSENT`.
3. Une phrase à Step 1.6 : le routage reste la voie nominale (il porte le
   `DEP-REVIEW:`, que le guard ne remplace pas) ; l'exclusion du guard est le
   filet déterministe quand ce routage est raté. **Aucune injonction ajoutée** —
   renforcer l'injonction est précisément ce que R4 du contexte écarte.

### Phase 3 — Tests

`scripts/verify-pipeline-test.sh` (harnais existant, cas A-E + trailers) :

- **F1 — le cas mesuré.** dependabot + `Cargo.lock` seul ⇒ exit 0. **Doit
  échouer avant la phase 1.**
- **F2 — contrôle négatif.** Auteur humain + `Cargo.lock` seul ⇒ exit 1. Sans
  lui, F1 ne prouve pas que l'exemption est ciblée — il pourrait passer parce
  que le gate a été désarmé pour tout le monde.
- **F3 — fail-closed.** `GITHUB_EVENT_PATH` absent + code-only ⇒ exit 1.
- **F4 — seconde forme.** `app/dependabot` ⇒ exit 0.
- **F5 — étroitesse de la portée.** dependabot + plan présent sans section
  `## Acceptance criteria` ⇒ exit 1 (le check mika#1600 survit à l'exemption).
- **F6 — pas de match par sous-chaîne.** Login `not-dependabot[bot]` +
  code-only ⇒ exit 1.

- **F7 — le fail-closed ne doit pas être un plantage.** Fichier d'event présent
  mais **malformé** (JSON invalide) + code-only ⇒ exit **1** avec le rejet
  code-only, jamais un code de sortie de `jq`. Voir la section
  `## Fire-Disposition` : le script tourne sous `set -euo pipefail`, donc une
  résolution écrite naïvement transformerait une PR qui passe aujourd'hui en
  `block[pipeline]`. C'est le seul chemin par lequel ce correctif pourrait
  casser du préexistant.

Garde structurelle (Rust, à placer près des scans de source existants) :
assertion que le bloc Step 2B de `system_prompt.md` porte `user.login`. Un test
comportemental ne peut pas attraper cette régression — le retrait du champ ne
rendrait aucune décision fausse, il rendrait l'exemption **inerte**, avec F1-F6
au vert. C'est exactement la classe que les scans de source maison existent pour
tenir. Modèle en vigueur : `qa_build_callback.rs::mika2355_the_scope_header_quotes_the_engine_marker`
(`include_str!` sur `../../skills/bundled/<skill>/system_prompt.md` depuis un
`#[cfg(test)] mod tests`).

---

## Fire-Disposition

*(Exigée par le Fire-Disposition Gate — mika#1574,
`docs/solutions/best-practices/fire-disposition-doctrine.md`. Ce plan livre
trois détecteurs : le harnais F1-F7, la garde structurelle Rust de la phase 3,
et — si l'on considère qu'un guard modifié est un détecteur livré — le
`verify-pipeline.sh` de la phase 1. Cette section dit ce que chacun fait face au
**préexistant**.)*

> **Disposition retenue : option (a) — exception nommée, `allowlist : zéro
> entrée`.** La vacuité est **mesurée, pas supposée** ; chaque mesure est
> rejouable en une commande, et la seule par laquelle elle pourrait être fausse
> est nommée puis fermée par un test (F7).

### Les trois détecteurs et leur population préexistante

| Détecteur | Ce sur quoi il tire | Préexistant, mesuré le 2026-09-20 sur `7c6f787b` |
|---|---|---|
| **F1-F7** (`scripts/verify-pipeline-test.sh`) | ses propres fixtures — dépôts jetables qu'il construit (`mktemp -d` + `git init`, l.26-28) | **néant par construction** : le harnais ne lit aucun fichier suivi du dépôt |
| **Garde structurelle Rust** | un fichier, un bloc : Step 2B de `skills/bundled/qa-review/system_prompt.md` (l.204) | **zéro entrée** — voir ci-dessous |
| **`verify-pipeline.sh` modifié** | les PR que la CI et QA font passer par lui | **aucune violation préexistante possible** — voir ci-dessous |

### La garde structurelle — rouge aujourd'hui, verte à l'atterrissage, et c'est le cas nominal

`grep -n "user.login" skills/bundled/qa-review/system_prompt.md` rend **vide** :
la ligne 204 porte `{"pull_request":{"number":…,"labels":[…]}}` et rien d'autre.
La garde serait donc rouge sur l'arbre d'aujourd'hui — et elle est verte à
l'atterrissage parce que **la phase 2 du même PR ajoute le champ qu'elle
asserte**. La population n'est pas « tolérée », elle est **mise en conformité
par le même diff** ; il n'y a rien à mettre en allowlist, et y écrire une entrée
poserait une ligne que rien ne pourrait plus faire rougir — strictement pire
qu'une table vide.

Cette rougeur pré-correctif n'est d'ailleurs pas un coût : c'est le **contrôle
positif** de la garde, du même ordre que « F1 doit échouer avant la phase 1 » au
contrat de vérification. Une garde qui serait déjà verte avant la phase 2
n'attesterait rien.

**Emplacement obligatoire de l'allowlist si elle cessait un jour d'être vide :**
à l'intérieur du `#[cfg(test)] mod tests`, jamais dans un chemin que le
chargeur de production consulte (doctrine mika#1574 § *When the allowlist is
structural*).

### Le guard modifié — l'exemption n'ajoute qu'une sortie 0, sauf sur un chemin, qui est fermé

Les deux blocs de rejet (`verify-pipeline.sh` l.238-273) ne font
qu'**incrémenter `ERRORS`** ; l'unique `exit 1` est en l.284-286, conditionné à
`ERRORS > 0`. Court-circuiter ces blocs pour un auteur reconnu ne peut donc
**que** retirer des erreurs : l'ensemble des PR qui passeraient de exit 0 à
exit 1 est vide par construction, et aucune PR aujourd'hui verte ne devient
rouge. Côté CI la population est vide pour une seconde raison, indépendante :
`ci.yml:336` saute entièrement le job sur `dependabot/`, donc aucun run n'est
même exécuté (R1 du contexte).

**La seule exception, et c'est pourquoi F7 existe.** Le script tourne sous
`set -euo pipefail` (l.88). Une résolution d'auteur écrite naïvement —
`AUTHOR=$(jq -r '.pull_request.user.login' "$GITHUB_EVENT_PATH")` — **avorte le
script** quand `jq` est absent ou quand le fichier est malformé, avec le code de
sortie de `jq`. Une PR qui passe aujourd'hui sortirait alors non-zéro, que Step
2C lit sans jugement comme `block[pipeline]` : le correctif produirait
exactement le symptôme qu'il ferme. La résolution doit donc reprendre mot pour
mot le motif défensif déjà employé deux lignes plus haut dans le même fichier
(l.217 et l.230) : `… 2>/dev/null || echo ""`. R3 dit « fail-closed ⇒ aucune
exemption » ; cette section précise que *fail-closed* signifie **poursuivre sans
exempter**, jamais **avorter**. F7 et AC3 (« exit 1 », littéralement, et non
« non-zéro ») en sont les deux pins.

### Halte

Si l'un des trois détecteurs tire sur du préexistant que cette section n'a pas
prévu, **la résolution est de chercher l'écrivain, jamais d'ouvrir
l'allowlist** : une entrée ajoutée pour faire passer un rouge inattendu
aveugle précisément la classe que le détecteur existe pour tenir.

---

## Contrat de vérification

| Quoi | Comment |
|---|---|
| F1 échoue avant, passe après | `git stash` du patch phase 1, `bash scripts/verify-pipeline-test.sh` |
| Aucune régression du harnais | cas A-E + trailers verts avant/après |
| CI inchangée | `ci.yml` non modifié fonctionnellement ; le job reste sauté sur `dependabot/` |
| Le champ atteint le guard | garde structurelle phase 3 + lecture du bloc 2B |
| Lint | `cargo fmt`, `cargo clippy -D warnings`, `make verify-bundled-skills` |

**Sonde post-déploiement, avec sa halte.** Au prochain bump dependabot :
verdict ≠ `block[pipeline]`. Si un `block[pipeline]` réapparaît sur une PR
dependabot, **ne pas élargir l'exemption par réflexe** — lire d'abord la section
`PIPELINE:` du verdict, qui cite la sortie du guard verbatim : si elle ne porte
pas la ligne `automated-author`, le champ n'a pas atteint le script (phase 2 non
déployée — classe mika#2340) et c'est le déploiement qu'il faut établir, pas le
prédicat.

---

## Definition of Done

- [ ] `verify-pipeline.sh` porte le quatrième mécanisme, fail-closed, documenté
      dans son en-tête.
- [ ] Le JSON synthétique de Step 2B porte `user.login`.
- [ ] F1-F7 écrits, F1 vérifié rouge avant le patch.
- [ ] La résolution d'auteur ne peut pas avorter le script sous `set -e` : motif
      `2>/dev/null || echo ""` employé, F7 vert (voir `## Fire-Disposition`).
- [ ] Garde structurelle sur la présence de `user.login` dans le prompt, placée
      dans un `#[cfg(test)] mod tests`, vérifiée **rouge** sur l'arbre
      pré-phase-2 (contrôle positif).
- [ ] `cargo fmt` / `cargo clippy -D warnings` / `make verify-bundled-skills`
      verts.
- [ ] Le corps de PR porte la rectification du diagnostic (R1-R4), pour que le
      ticket ne soit pas refermé sur une prémisse fausse.

## Acceptance criteria

- [ ] Une PR ouverte par `dependabot[bot]` ne portant que `Cargo.lock` obtient
      exit 0 de `scripts/verify-pipeline.sh`, avec une ligne
      `info: [pipeline-exempt: automated-author]` en sortie.
- [ ] La même PR ouverte par un auteur humain obtient exit 1 avec le rejet
      code-only inchangé.
- [ ] Sans `GITHUB_EVENT_PATH`, une PR code-only obtient exit 1 (aucune
      exemption n'est accordée sans information d'auteur).
- [ ] `app/dependabot` est reconnu au même titre que `dependabot[bot]` ; un
      login qui contient l'un d'eux en sous-chaîne ne l'est pas.
- [ ] Une PR d'auteur automatique portant un plan sans section
      `## Acceptance criteria` obtient toujours exit 1 (mika#1600 survit).
- [ ] Le JSON synthétique de Step 2B contient `user.login`, alimenté par le
      champ `author` de `qa_pr_view`, et une garde structurelle échoue si ce
      champ disparaît.
- [ ] Le test F1 échoue sur l'arbre pré-correctif et passe après.
- [ ] Aucun comportement CI ne change : `ci.yml` conserve son exclusion de
      branche et aucune PR aujourd'hui verte ne devient rouge.
- [ ] L'asymétrie docs-only / code-only du label `pipeline-exempt` est inchangée
      (aucune ligne du bloc `EXEMPT_PR_LABEL_DOCS` modifiée).

---

## Hors périmètre, délibérément

- **Le comportement du modèle sur Step 1.6.** Ce plan ne le répare pas : il rend
  son échec inoffensif. Ajouter une injonction reproduirait le mode de panne
  mesuré (mika#2120).
- **L'élargissement du label `pipeline-exempt` au check code-only**, que le
  correctif littéral du ticket impliquerait. L'asymétrie est *load-bearing* et
  protège mika-platform#17 ; la fermer demanderait son propre ticket et sa
  propre mesure.
- **Un scénario de calibration dependabot dans `mika_qa.rs`.** Légitime, mais
  c'est une **mesure**, pas un correctif — et le ticket l'a précisément confondue
  avec la surface servie. Ticket de suivi si l'on veut mesurer la fidélité du
  routage Step 1.6 ; ce plan rend cette fidélité non critique.
- **`isDraft` absent de `qa_pr_view.sh`**, qui rend le Step 1.5.4 inexécutable
  dans son propre périmètre. Défaut réel, déjà nommé comme ticket de suivi,
  sans rapport avec celui-ci.
- **Un lint de parité `ci.yml` ↔ script** : voir l'arbitrage assumé ci-dessus.

---

## Revision history

- **rev 2 (2026-09-20)** — addressed F1 (BLOCKING : détecteurs livrés sans
  section `## Fire-Disposition`, gate mika#1574) en ajoutant cette section
  entre les phases d'implémentation et le contrat de vérification, sans
  renuméroter ni réécrire aucune section existante. Option canonique **(a) —
  exception nommée, `allowlist : zéro entrée`** retenue, comme le finding
  l'indiquait, avec sa condition de disponibilité **mesurée plutôt que
  supposée** (2026-09-20, `7c6f787b`) : le harnais `verify-pipeline-test.sh`
  construit ses fixtures sous `mktemp -d` + `git init` (l.26-28) et ne lit
  aucun fichier suivi ; `grep -n "user.login" skills/bundled/qa-review/system_prompt.md`
  rend vide, donc la garde structurelle est rouge aujourd'hui et verte à
  l'atterrissage parce que la phase 2 du même PR ajoute le champ qu'elle
  asserte — ce qui fait de cette rougeur le **contrôle positif** de la garde,
  non une population à tolérer. L'emplacement obligatoire de l'allowlist si
  elle cessait d'être vide (`#[cfg(test)] mod tests`, jamais le chemin de
  production) est nommé, ainsi que la halte : un rouge inattendu se traite en
  cherchant l'écrivain, jamais en ouvrant l'allowlist.
  La seconde moitié du finding — la disposition du **guard modifié lui-même**
  vis-à-vis des PR préexistantes — est traitée nommément, et sa conclusion
  (« aucune violation préexistante possible, l'exemption ne fait qu'ajouter un
  chemin de sortie 0 ») est **vérifiée plutôt que reprise** : les deux blocs de
  rejet (l.238-273) n'incrémentent que `ERRORS`, l'unique `exit 1` étant en
  l.284-286. La vérification a trouvé **un** chemin par lequel la conclusion
  serait fausse, et c'est le seul écart de fond à la lettre du finding : le
  script tourne sous `set -euo pipefail` (l.88), donc une résolution d'auteur
  écrite sans le motif défensif déjà employé aux l.217 et 230 avorterait le
  script sur un `jq` absent ou un event malformé, faisant sortir non-zéro une
  PR aujourd'hui verte — c'est-à-dire produisant le `block[pipeline]` que ce
  plan ferme. Ce chemin est fermé par un test **F7** ajouté à la phase 3 et par
  deux cases de DoD. Aucun AC n'est ajouté ni affaibli : AC3 pinne déjà le code
  de sortie **1** (et non « non-zéro »), ce qui est exactement l'assertion
  qu'exige ce chemin.
  *Citations préservées : Fire-Disposition Gate / mika#1574 /
  `docs/solutions/best-practices/fire-disposition-doctrine.md` ; précédents de
  résolution mika#2423 et mika#2049.*
- Aucun autre contenu modifié. Les neuf AC existants sont inchangés — la
  disposition retenue est une contrainte d'implémentation, couverte par AC3
  (fail-closed avec son code de sortie littéral) et AC6 (garde structurelle sur
  la disparition du champ).
