# Plan — mika#2516 : le check d'Acceptance criteria tolère un préfixe de numérotation

**Ticket :** mika#2516
**Type :** fix (substrat CI)
**Module :** `scripts/verify-pipeline.sh`
**Classe :** n=3 d'une famille documentée dans ce dépôt (voir §0.4)

---

## 0. Ce que la lecture du code déplace dans le ticket

Trois rectifications, chacune mesurée, et chacune change le livrable. Elles sont
en tête parce qu'appliquer la lettre du ticket produirait un correctif qui
répare un faux négatif en en créant deux autres.

### 0.1 — Il n'y a PAS « d'autres sections exactes que le check exige »

Le ticket propose : « Idem pour les autres sections exactes que le check
exige. » **Il n'y en a aucune.** `scripts/verify-pipeline.sh` fait 360 lignes et
n'exige qu'**un seul** en-tête littéral, `## Acceptance criteria`, au bloc
mika#1600 (lignes 138–161). Tout le reste du script raisonne sur des **chemins**
(`^docs/plans/`, `^docs/solutions/`, `^.github/`), des **trailers de commit**
(`Pipeline-Exempt:`), des **labels** et le **login** de l'auteur — jamais sur un
en-tête de plan.

Élargir à « les autres sections » serait inventer un périmètre. Le livrable est
donc étroitement borné à un motif, à deux sites.

### 0.2 — La regex proposée par le ticket RESSERRE le check

Le ticket propose `^## (\d+\.\s+)?Acceptance criteria$`. Le `$` final est une
régression : le matcher actuel est `grep -qi '^## Acceptance criteria'`,
**sans ancrage de fin**. Mesuré (sonde G) :

```
  ## Acceptance criteria (revised)   →  grep actuel : MATCH   (passe aujourd'hui)
                                     →  regex du ticket : NO-MATCH (rougirait demain)
```

*(la ligne d'exemple ci-dessus est indentée à dessein : le check ne strippe pas
les blocs clôturés — voir §7, limite trouvée en chemin.)*

Un plan portant un suffixe après l'en-tête passe le check aujourd'hui et
échouerait après le correctif. **La permissivité existante est préservée
telle quelle** : on ajoute un préfixe optionnel, on n'ajoute pas d'ancrage.

### 0.3 — Il y a DEUX matchers du même motif, et corriger l'un seul déplace le défaut

C'est le piège central, et il est invisible à la lecture du ticket. Le motif est
écrit **deux fois**, à sept lignes d'écart :

| ligne | rôle | expression |
|---|---|---|
| 147 | présence | `grep -qi '^## Acceptance criteria'` |
| 154 | non-vacuité | `sed -n '/^## Acceptance criteria/I,/^## /{…}'` |

Mesuré (sonde C) — en corrigeant **le `grep` seul**, sur un en-tête
`## 11. Acceptance criteria` :

```
grep tolérant  → MATCH        (la branche « missing » n'est plus prise)
sed strict     → sortie VIDE  (l'adresse de départ ne matche rien)
message rendu  → FAIL: Plan '…' has empty '## Acceptance criteria' section.
```

Le faux négatif n'est pas réparé : il est **déplacé**, et son message devient
**trompeur** — il accuse une section vide alors que la section est pleine. Un
implémenteur qui ne corrigerait qu'un site produirait un état strictement pire
que l'actuel, où le message dirigeait au moins vers la bonne ligne.

Ce n'est pas une hypothèse de conception : **ce fichier a déjà rencontré ce
piège une fois**. mika#1639 a dû corriger ces deux sites ensemble pour la
casse, et son doc de solution consacre un paragraphe à expliquer lequel des
deux avait besoin du flag `I`. Six mois plus tard, mêmes deux sites, même
exigence de co-mutation, et toujours rien qui la tienne. C'est ce qui justifie
le détecteur structurel du §3.2 — la co-mutation est à n=2, pas à n=1.

### 0.4 — La classe est documentée dans ce dépôt, et ceci en est le n=3

`docs/solutions/workflow-issues/verify-pipeline-ac-heading-case-insensitive-2026-06-30.md`
§ « La classe récurrente » énumère déjà :

- **n=1 — forme d'en-tête** (mika#1381/#771/#1600 → #1602) : `**Ticket:**` vs `**Issue:**`
- **n=2 — casse d'en-tête** (mika#1639) : `Acceptance Criteria` vs `Acceptance criteria`
- **n=3 — préfixe de numérotation** (mika#2516) : `## 11. Acceptance criteria` ← *ce ticket*

Et il énonce la règle, mot pour mot :

> Quand un gate matche un en-tête qu'un LLM rédige, rendre le match tolérant à
> la variation naturelle que le modèle produira (casse, ponctuation
> environnante, synonyme évident), et épingler le comportement par une fixture.
> **Corriger le matcher (contrôle), pas le prompt (documentation)** — le prompt
> est la mauvaise couche pour un invariant structurel.

Ce plan applique la règle existante à son troisième axe. Il n'invente pas de
doctrine ; il en exécute une, et il ajoute la garde que les deux occurrences
précédentes n'avaient pas posée.

### 0.5 — La preuve est dans l'arbre, pas seulement dans le ticket

`docs/plans/2026-09-24-002-fix-2511-tenir-le-verrou-pendant-la-suppression-plan.md`
porte onze en-têtes numérotés (`## 0.` … `## 10.`) et **un seul** non numéroté :
`## Acceptance criteria`, ligne 640. C'est le fix-1-ligne du commit `6af5e7b5`,
visible dans l'arbre. Le motif n'a pas besoin d'être reconstruit — il est lisible
tel quel, et il sert de fixture réelle au §3.1.

---

## 1. Requirements

**R1.** Un plan dont l'en-tête AC porte un préfixe de numérotation
(`## 11. Acceptance criteria`, `## 0. Acceptance Criteria`) **passe** le check
Pipeline Artifacts, sans fix manuel.

**R2.** Les **deux** matchers du bloc mika#1600 bougent ensemble. Un plan
numéroté ne doit jamais atterrir sur le message « empty section » (§0.3).

**R3.** La **décision** reste stricte : une section AC absente, ou présente et
vide, **échoue toujours**. La tolérance porte sur la détection de l'en-tête,
jamais sur la vacuité du contenu.

**R4.** Aucune régression de la permissivité acquise : la casse (mika#1639) et
l'absence d'ancrage de fin (§0.2) sont préservées à l'identique.

**R5.** Les messages d'échec sont **inchangés, à la lettre**. Ce sont un format
de fil : `verify-pipeline-test.sh` les assert par sous-chaîne (lignes 459, 484,
602) et qa-review lit le code de sortie du script sans jugement (Step 2C,
`block[pipeline]`). Un message reformulé casserait des assertions sans rapport
avec ce ticket.

**R6.** Une troisième expression littérale du motif ne peut pas être ajoutée en
silence (garde structurelle, §3.2).

---

## 2. Conception

### 2.1 — Le découpage : détection permissive, décision stricte

La doctrine du dépôt
(`docs/solutions/architecture-patterns/guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md`)
donne le découpage exact, et il s'applique ici sans adaptation :

| question | direction | site |
|---|---|---|
| *Détection* — « cette ligne est-elle l'en-tête de la section AC ? » | **permissive** | l'expression, aux deux sites |
| *Décision* — « la section est-elle non vide ? » | **stricte** | le test `[[ -z "$AC_CONTENT" ]]`, inchangé |

Tout le correctif vit dans la colonne « détection ». Pas une ligne de la colonne
« décision » ne bouge.

### 2.2 — Une constante, deux interpolations

Le motif cesse d'être écrit deux fois. Une constante est définie une fois, juste
au-dessus du bloc mika#1600, et interpolée aux deux sites :

```bash
# mika#2516 — motif de DÉTECTION de l'en-tête AC, site unique.
#
# Permissif par conception (doctrine « détection permissive, décision stricte ») :
#   - casse repliée par `-i` / le flag `I` (mika#1639, n=2)
#   - préfixe de numérotation optionnel, `/ce:plan` numérotant ses en-têtes (n=3)
#   - PAS d'ancrage `$` : `## Acceptance criteria (revised)` passe aujourd'hui
#     et doit continuer de passer (mika#2516 §0.2)
#
# La DÉCISION reste stricte : une section vide échoue, ici comme avant.
# Ne pas réécrire ce motif ailleurs — voir la garde de lecteur unique dans
# scripts/verify-pipeline-test.sh (mika#2516).
AC_HEADING_RE='^##[[:space:]]+([0-9]+\.?[[:space:]]+)?Acceptance criteria'
```

Sites d'usage :

```bash
if ! grep -qiE "$AC_HEADING_RE" "$PLAN_FILE"; then
  …
  AC_CONTENT=$(sed -nE "/$AC_HEADING_RE/I,/^## /{ /^## /d; /^[[:space:]]*\$/d; p; }" "$PLAN_FILE")
```

### 2.3 — Quatre pièges d'implémentation, tous mesurés

**(a) `grep -qi` → `grep -qiE`.** Le motif passe en ERE ; `[[:space:]]+`,
`( … )?` et `[0-9]+` y sont des quantificateurs, pas des littéraux. Sans le
`-E`, le motif matche la chaîne littérale et ne trouve rien — un désarmement
silencieux.

**(b) `sed -n` → `sed -nE`.** Même raison, côté adresse de départ. GNU sed
accepte le flag `I` d'adresse **en mode `-E`** — vérifié sur GNU sed 4.10
(sonde D/F).

**(c) L'adresse de fin et le `d` interne ne bougent pas.** `/^## /` matche
n'importe quel en-tête `## `, numéroté compris, puisque le préfixe `## ` y est
inchangé. Le doc mika#1639 avait déjà établi ce point — le citer évite de
« corriger » un site qui n'a pas de défaut.

**(d) Le `$` de `/^[[:space:]]*$/d` doit être échappé.** L'expression sed passe
de quotes simples à quotes doubles pour permettre l'interpolation ; bash y
développerait `$/` . Écrire `\$`. Vérifié : la sonde d'interpolation rend le bon
contenu aux trois fixtures.

Corollaire de (d), à écrire dans le commentaire : **la constante est définie en
quotes simples et interpolée en quotes doubles.** Définie en quotes doubles,
son `\.` resterait littéral mais le `$`… n'existe pas dans la constante — le
risque est asymétrique et ne concerne que le corps du `sed`.

### 2.4 — Bornes du motif, et ce qu'il refuse encore

Mesures de la sonde :

| en-tête | verdict | pourquoi |
|---|---|---|
| `## 11. Acceptance criteria` | **passe** | le défaut mesuré |
| `## 11 Acceptance criteria` | **passe** | point optionnel, variation proche non coûteuse |
| `## Acceptance Criteria` | **passe** | non-régression mika#1639 |
| `## Acceptance criteria (revised)` | **passe** | non-régression §0.2 |
| `## Notes on Acceptance criteria` | **refusé** | le texte reste ancré après le préfixe |
| `## 11. Acceptance criteria` + section vide | **refusé** | la décision reste stricte (R3) |

La dernière ligne est celle qui compte : c'est la garde du § « When NOT to
over-widen » du doc mika#1639 — *« ne pas relâcher un gate au-delà du point où
il prouve encore ce pour quoi il existe »*.

Le préfixe est borné à `<chiffres>[.]` — la forme que `/ce:plan` produit
réellement (mesurée sur le plan de #2514). Les formes `## 1.1 …` ou
`## Phase 3 — …` ne sont **pas** couvertes : elles ne sont pas mesurées, et
élargir sur une hypothèse est ce que le § « When NOT to over-widen » refuse. Si
l'une apparaît, elle rougira le check — visiblement, avec le bon message — et ce
sera un n=4 à traiter avec la même méthode.

### 2.5 — Les trois autres lecteurs du motif ne bougent pas, et c'est une décision

Quatre gates lisent « la section AC » sur le trajet d'un plan :

| gate | lecteur | strict ? | action |
|---|---|---|---|
| `mika-arch-groom-ticket` (1ʳᵉ passe) | LLM | non | **inchangé** |
| `mika-arch-second-review` (2ᵉ passe) | LLM | non | **inchangé** |
| `scripts/verify-pipeline.sh` (CI) | **shell** | **oui** | **corrigé** |
| `qa-review` Step ~285 | LLM | non | **inchangé** |

Un seul est strict, et c'est le seul qui a produit le défaut — trois occurrences
en un jour, toutes côté CI, aucune côté architecte ni côté QA. Un lecteur LLM
reconnaît `## 11. Acceptance criteria` comme la section AC sans qu'on le lui
dise.

Ajouter une clause « l'en-tête peut être numéroté » à ces trois prompts serait
du prompt-enforcement sur un invariant structurel — exactement ce que la règle
du §0.4 et `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
condamnent. **Ne rien y toucher est le livrable, pas une omission.**

---

## 3. Contrat de vérification

### 3.1 — Fixtures comportementales (`scripts/verify-pipeline-test.sh`)

Quatre cas, ajoutés à la suite du bloc « Acceptance criteria section tests
(mika#1600) », sur le modèle exact de la fixture mika#1639 qui le précède
(repo git temporaire, `run_verify`, `assert_pass` / `assert_fail`).

| # | fixture | attendu | ce qu'elle atteste |
|---|---|---|---|
| **T1** | `## 11. Acceptance criteria` + contenu | `assert_pass` | R1 **et R2** — voir ci-dessous |
| **T2** | `## 11. Acceptance criteria` + section **vide** | `assert_fail`, sous-chaîne `empty '## Acceptance criteria' section` | R3 — la décision reste stricte sur un en-tête numéroté |
| **T3** | `## Acceptance Criteria` (titre, non numéroté) | `assert_pass` | R4 — non-régression mika#1639 |
| **T4** | `## Notes on Acceptance criteria` seul | `assert_fail`, sous-chaîne `missing '## Acceptance criteria' section` | borne haute — la tolérance n'a pas avalé le refus |

**T1 est la fixture porteuse, et c'est elle qui tient le §0.3.** Sur un correctif
à moitié appliqué (le `grep` tolérant, le `sed` resté strict), la section pleine
et numérotée rend le message « empty » : `assert_pass` rougit. T1 distingue donc
« les deux matchers ont bougé » de « le `grep` a bougé », ce qu'aucune autre
fixture ne fait.

T2 ne peut pas jouer ce rôle : sur le même correctif à moitié appliqué, elle
**passerait** — son attendu *est* le message « empty ». Elle atteste R3, pas la
co-mutation. Les deux sont nécessaires, et le contrôle négatif du §3.3 est ce
qui prouve que T1 remplit bien le sien.

### 3.2 — Garde structurelle : un seul lecteur du motif

Le comportement ne peut pas voir la régression que le §0.3 décrit : réécrire le
motif en dur à un troisième site ne rend **aucune** décision fausse le jour où
on l'écrit — les quatre fixtures restent vertes, et la divergence n'apparaît
qu'au prochain axe de variation, des mois plus tard. C'est exactement la forme
qui a produit ce ticket.

Garde, dans `verify-pipeline-test.sh`, à la suite des fixtures :

1. **Anti-vacuité d'abord.** La définition `AC_HEADING_RE=` doit exister dans
   `scripts/verify-pipeline.sh`, **exactement une fois**. Zéro occurrence ⇒
   échec : un scan qui vise un nom mort se lit exactement comme un arbre propre
   (classe mika#2205).
2. **Deux usages attendus**, `grep -qiE "$AC_HEADING_RE"` et l'adresse sed
   interpolée. Le compte est **asserté**, pas « au moins ».
3. **Aucune forme littérale résiduelle.** Les deux écritures historiques
   (`'^## Acceptance criteria'` en argument de `grep`, `/^## Acceptance criteria/`
   en adresse `sed`) ne doivent plus apparaître **en position de commande**.
   Le prédicat exclut les lignes de commentaire (`^[[:space:]]*#`) et les
   `echo` — sans quoi il rougirait sur les messages d'erreur et sur le
   commentaire d'en-tête, qui citent légitimement le motif. Le bloc de
   commentaire du §2.2 cite lui-même le nom de la section : le scan doit rester
   vert dessus, et c'est son contrôle de bonne foi.

**Allowlist :** `AC_HEADING_LITERAL_ALLOWED`, **livrée vide**, avec un test
frère qui refuse qu'elle cesse de l'être sans une entrée nommée, datée et
référencée. Quand ce scan tire, **la résolution est de router le site vers la
constante, jamais d'ajouter une ligne d'allowlist** (doctrine mika#2201).

### 3.3 — Contrôle négatif, à voir ROUGE avant de livrer

Trois dépouillements, chacun exécuté et **observé rouge**, puis restauré :

| dépouillement | doit faire rougir |
|---|---|
| corriger le `grep` seul, laisser le `sed` strict | **T1** (message « empty » là où « pass » est attendu) — atteste le §0.3 |
| retirer le `-E` du `grep` | T1, T4 |
| retirer la définition `AC_HEADING_RE` | la garde §3.2, par son terme d'anti-vacuité |

Le premier est le contrôle porteur : sans lui, rien n'atteste que la suite
distingue un correctif complet d'un correctif à moitié appliqué.

### 3.4 — Vérification en service

`bash scripts/verify-pipeline-test.sh` doit rendre 0, **et le compte total de
tests doit avoir augmenté de quatre** — un compte inchangé signifie que les
fixtures n'ont pas été enregistrées dans le compteur, donc qu'une suite
silencieusement plus courte se lit comme une suite verte.

---

## 4. Fire-Disposition

Ce plan livre des détecteurs : quatre fixtures comportementales (§3.1) et un
scan structurel (§3.2). La règle mika#2306 s'applique.

**Option retenue : (a) — exception nommée en allowlist, livrée VIDE.**

Justification, par détecteur :

- **Fixtures §3.1** — elles testent un comportement **neuf** (la tolérance au
  préfixe). Aucune violation préexistante n'est concevable : avant le correctif
  le comportement n'existe pas, après il existe. Zéro exception requise.
- **Scan §3.2** — il compte les expressions du motif **après** que le correctif
  les a unifiées. À l'état livré, l'arbre est conforme par construction : une
  définition, deux interpolations, zéro littéral résiduel en position de
  commande. L'allowlist `AC_HEADING_LITERAL_ALLOWED` atterrit donc **vide**, et
  un test frère (`…_the_allowlist_ships_empty`) refuse qu'elle cesse de l'être
  sans entrée nommée.

**Livraison armée**, pas désarmée — et l'argument est mesuré plutôt que
prudentiel : l'option (b) existe pour un détecteur dont la population de
violations préexistantes est inconnue ou massive. Ici elle est **connue et
nulle**, établie par la lecture exhaustive du fichier (§0.1 : un seul motif,
deux sites, tous deux réécrits par ce correctif). Livrer désarmé un détecteur
dont on a établi que l'arbre est propre reproduirait la panne de mika#2272 —
une condition d'armement que rien ne vient jamais satisfaire, et un zéro qui est
l'absence de mesure plutôt que la présence de prudence.

**Chaque exception future doit porter** : la donnée précise (le fichier et la
forme littérale), un ticket de suivi, et l'assertion auto-nettoyante qui rougit
le jour où le fichier ne contient plus cette forme.

---

## 5. Documentation

**D1 — `docs/solutions/workflow-issues/verify-pipeline-ac-heading-case-insensitive-2026-06-30.md`.**
Étendre le § « La classe récurrente (n=2) » en **n=3**, avec la ligne
manquante :

> **n=3 — préfixe de numérotation** (mika#2516) : `/ce:plan` numérote ses
> en-têtes de section ; le gate matchait le motif nu. Corrigé aux **deux**
> matchers ensemble, par une constante unique — la co-mutation que n=2 avait dû
> faire à la main sans rien poser pour la tenir.

C'est le doc qui porte déjà la règle et son compteur d'occurrences ; ouvrir un
second doc scinderait une famille dont la valeur **est** d'être comptée au même
endroit. Mettre à jour son `created:` n'est pas requis ; ajouter mika#2516 aux
`tags` l'est.

**D2 — le commentaire du §2.2**, dans `scripts/verify-pipeline.sh`. C'est la
documentation qui compte, parce que c'est la seule qu'un futur éditeur du
fichier lira nécessairement. Il nomme les trois axes de permissivité, dit
pourquoi il n'y a pas d'ancrage `$`, et renvoie à la garde du §3.2.

**D3 — pas d'entrée `CLAUDE.md`.** Ce correctif n'ajoute ni variable
d'environnement, ni surface opérateur, ni événement de journal, ni sonde. Il
n'y a rien qu'un opérateur ait à lire ou à surveiller ; le seul signal est le
rouge du check lui-même. Écrire une entrée serait ajouter du bruit à un fichier
index déjà dense.

---

## 6. Surfaces opérateur, sondes, et leurs haltes

**Aucune surface nouvelle.** Aucun compteur, aucun événement, aucune ligne de
journal : le défaut est un check rouge, et son correctif est un check vert. Les
seuls instruments sont ceux qui existent.

**S1 — le défaut est fermé (premier plan groomé après merge).** Le prochain plan
produit par la boucle avec un en-tête AC numéroté doit passer Pipeline Artifacts
sans fix manuel.

> **Halte 1 — le check rougit encore sur un en-tête numéroté.** Lire le message
> **avant** de toucher au motif. `missing … section` ⇒ le `grep` n'a pas la
> tolérance ou n'a pas le `-E`. `empty … section` ⇒ **le `sed` n'a pas bougé**,
> c'est le §0.3 réalisé, et le remède est le second site, pas un élargissement
> du premier. Les deux messages nomment deux remèdes opposés ; c'est pour ça
> qu'on ne les a pas fusionnés (R5).

**S2 — contrôle positif, et il n'est pas décoratif.** Sur les 30 jours suivants,
au moins un plan portant un en-tête AC **numéroté** doit avoir traversé le
check. Zéro plan numéroté **et** zéro échec ne prouve rien : `/ce:plan` peut
avoir cessé de numéroter, ou aucun groom n'avoir tourné, et un check qui n'a
jamais été exercé se lit exactement comme un check qui marche (classe
mika#2205). La lecture :

```bash
grep -l '^## [0-9]\+\.\? \+Acceptance criteria' docs/plans/*.md
```

Non vide ⇒ le correctif a servi. Vide ⇒ **on ne sait rien**, et il faut regarder
si des plans récents portent des en-têtes numérotés du tout avant de conclure
quoi que ce soit.

**S3 — contrôle négatif de la décision.** Aucun plan sans section AC, ou avec
une section vide, ne doit passer.

> **Halte 2 — un plan sans AC passe.** La tolérance a avalé la décision.
> **Reverter d'abord**, diagnostiquer ensuite : un gate de plan qui ne prouve
> plus rien est pire que le fix-1-ligne qu'il remplace, parce qu'il est vert.

> **Halte 3 — la variante non couverte apparaît** (`## 1.1 Acceptance criteria`,
> `## Phase 3 — Acceptance criteria`). C'est un **n=4**, pas un défaut de ce
> correctif : le §2.4 borne délibérément le motif à ce qui est mesuré.
> Le traiter par la même méthode — mesurer la forme réelle, élargir la
> constante à cet endroit unique, ajouter une fixture, incrémenter le compteur
> du doc D1. **Ne pas élargir au jugé vers « tout préfixe ».**

---

## 7. Hors périmètre, délibérément

- **Le générateur `/ce:plan`** (option (b) du ticket). Il vit dans le plugin
  marketplace `compound-engineering`, hors de ce dépôt. **Suivi nommé**, non
  simulé. Et même livré, il ne fermerait pas ce ticket : il ne corrigerait aucun
  des plans déjà écrits, et le check resterait le seul lecteur strict d'un texte
  produit par un tiers — la configuration que le doc D1 décrit comme
  structurellement récurrente.
- **Les commandes de groom** (`.claude/commands/mika-groom-plan-only.md` étape
  5b, qui instruit d'ajouter la section). Elles vivent dans
  `senara-solutions/mika-platform` et sont semées dans le worktree par
  `_seed_worktree_slash_commands` (mika#1415) : **un ticket ouvert sur
  `senara-solutions/mika` ne peut pas les éditer.** Même borne que
  `_PLAN_FIRE_DISPOSITION_RULE` a dû écrire pour elle-même.
- **Les trois lecteurs LLM** (§2.5) — inchangés, et c'est une décision.
- **Les messages d'échec** (R5) — format de fil.
- **Le check ne strippe pas les blocs clôturés**, contrairement à
  `auto_pull::is_groomed` (mika#2120, qui a dû apprendre exactement cette
  leçon : *un jeton cité dans une fence est une mention, pas une instruction*).
  Conséquence réelle et non hypothétique : un plan qui **cite** un en-tête AC
  dans un exemple — ce plan-ci le fait au §0.2 — voit le `grep` matcher la
  citation d'abord, et le `sed` démarrer sa plage **là**. Le check passe alors
  en ayant lu le mauvais bloc. Contourné ici par une indentation de deux
  espaces, ce qui est un contournement d'auteur et non une propriété du gate.
  Même famille que ce ticket, discriminant différent (position dans le document,
  pas forme de l'en-tête), correction différente (stripper les fences avant les
  deux matchers). **Suivi**, à ouvrir avec cet exemple comme fixture.
- **Le commentaire d'en-tête périmé** du script, qui déclare
  « Aligned with mika-platform/scripts/verify-pipeline.sh » alors que ce fichier
  **n'existe plus** (vérifié : `/data/workspace/mika-platform/scripts/verify-pipeline.sh`
  absent). Trouvé en chemin, réel, sans rapport avec ce ticket : **suivi**.
- **Les autres gates du script** (buckets, trailers, labels, auteur automatisé)
  — aucune ligne ne bouge.

---

## 8. Definition of Done

- [ ] `AC_HEADING_RE` défini une seule fois dans `scripts/verify-pipeline.sh`,
      avec le commentaire du §2.2, et interpolé aux deux sites (`grep -qiE`,
      `sed -nE`).
- [ ] Le `$` de `/^[[:space:]]*$/d` échappé lors du passage en quotes doubles
      (§2.3d).
- [ ] La décision (`[[ -z "$AC_CONTENT" ]]`) et les deux messages d'échec :
      **inchangés à l'octet**.
- [ ] Les quatre fixtures T1–T4 ajoutées à `scripts/verify-pipeline-test.sh`.
- [ ] La garde de lecteur unique (§3.2) ajoutée, allowlist livrée vide, terme
      d'anti-vacuité inclus.
- [ ] Les trois contrôles négatifs du §3.3 exécutés et **vus rouges**, puis
      restaurés — le premier en particulier.
- [ ] `bash scripts/verify-pipeline-test.sh` rend 0, avec un compte total
      augmenté de quatre.
- [ ] Le doc D1 étendu en n=3.
- [ ] `bash scripts/verify-pipeline.sh origin/main` vert sur la PR elle-même.

---

## Acceptance criteria

- [ ] **AC1.** Un plan dont l'en-tête AC porte un préfixe de numérotation
      (`## 11. Acceptance criteria`) **passe** le check Pipeline Artifacts sans
      fix manuel — le critère littéral du ticket. Attesté par la fixture T1.
- [ ] **AC2.** Le motif n'exige plus de fix-1-ligne par groom : un plan produit
      par `/ce:plan` avec ses en-têtes numérotés traverse la CI sans édition
      humaine de son en-tête AC.
- [ ] **AC3.** Un plan numéroté dont la section AC est **vide** échoue toujours,
      avec le message `empty '## Acceptance criteria' section` — la décision
      reste stricte (fixture T2).
- [ ] **AC4.** Un plan **sans** section AC échoue toujours, avec le message
      `missing '## Acceptance criteria' section` (fixture T4).
- [ ] **AC5.** Non-régression mika#1639 : un en-tête en casse de titre
      (`## Acceptance Criteria`), numéroté ou non, passe (fixture T3).
- [ ] **AC6.** Non-régression §0.2 : un en-tête suivi d'un suffixe
      (`## Acceptance criteria (revised)`) passe, comme aujourd'hui.
- [ ] **AC7.** Les deux matchers sont dérivés d'une définition unique, et la
      garde du §3.2 rougit si une troisième forme littérale est écrite en
      position de commande — allowlist livrée vide.
- [ ] **AC8.** Le contrôle négatif porteur (corriger le `grep` seul) a été
      exécuté et **vu rouge** sur T1 avant livraison.
- [ ] **AC9.** `bash scripts/verify-pipeline-test.sh` rend 0, avec quatre tests
      de plus qu'avant.
- [ ] **AC10.** Le doc `verify-pipeline-ac-heading-case-insensitive-2026-06-30.md`
      compte désormais **n=3** et nomme mika#2516.
