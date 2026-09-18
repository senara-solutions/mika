# fix(permission-policy) : le classifier ne décide jamais sur le contenu d'un fichier — la dichotomie du ticket est mal posée (mika#2312)

- **Ticket :** mika issue#2312
- **Type :** fix (décision de sûreté déclarée + garde structurelle + message de diagnostic)
- **Date :** 2026-09-18
- **Relie :** mika#2306 (nommé par le ticket, non absorbé), mika#1410, mika#1686, mika#1708, mika#1817

---

## Ce que le ticket affirme, et ce que la mesure établit

Le ticket part d'une comparaison entre deux événements du log claude-pilot `3587fe25`
(2026-09-14, groom #2295) :

| Heure | Commande | Verdict |
|---|---|---|
| 21:06 | `cat skills/bundled/<skill>/skill.toml` | `[policy:allow]` |
| 21:11:57 | `for f in skills/bundled/mika-arch-groom-ticket/system_prompt.md skills/bundled/mika-arch-second-review/system_prompt.md; do …` | `[policy:deny]` |

et en tire : « C'est donc le CONTENU des system_prompt qui est protégé, pas le
chemin/pattern. »

**La comparaison n'est pas contrôlée.** Les deux commandes diffèrent sur deux axes,
pas un : le fichier lu (`skill.toml` vs `system_prompt.md`) **et la forme de la
commande** (un spawn simple vs une boucle `for … do … done`). L'inférence attribue
au premier axe un effet que le second explique entièrement. Trois preuves
indépendantes le montrent, et elles convergent.

### Preuve 1 — le control flow est refusé au niveau source brut, par conception

`docs/solutions/security-issues/1817-mika-side-plugin-per-binary-safety-functions.md`,
§ *What the plugin does NOT check*, écrit le 2026-07-22 :

> The engine's `per_spawn.decompose()` already refuses these at the raw-source
> level, so per-binary functions never see them :
> - Command substitution, heredocs, process substitution, arithmetic expansion.
> - **Control flow (`if`, `for`, `while`, `case`, `select`, `until`, functions).**
> - Dynamic execution builtins (`eval`, `source`, `.`, `exec`).

La commande déniée commence par `for f in … ; do`. Elle est refusée **avant** que
la moindre question soit posée sur un fichier, un chemin ou un contenu.

### Preuve 2 — aucune fonction du registre ne peut lire un fichier

`tools/mika_permission_policy/mika_permission_policy/_binaries.py:15-25`, docstring
*What we do NOT check here* :

> - **Path containment for filesystem targets** — this is a Write/Edit tool
>   concern, not a Bash one, and tier1 doesn't check it either.

et `_binaries.py:46-47` :

```python
def is_safe_cat(argv: list[str], cwd: str) -> bool:
    return True
```

Toutes les fonctions du registre ont la signature `(argv, cwd) -> bool`. Le module
n'importe que `__future__` : il n'a aucun moyen d'ouvrir un fichier. Le contenu
n'est pas « autorisé » ni « refusé » — **il n'est pas une dimension de décision**.

### Preuve 3 — mesure directe, 2026-09-18, depuis un pilote sandboxé

Le présent groom, dispatché en sandbox sous le même classifier, a exécuté :

```
head -5 skills/bundled/mika-arch-groom-ticket/system_prompt.md
```

— exactement le fichier que le ticket déclare protégé — et l'a **lu**. Le même
pilote a lu `_binaries.py` (outil `Read`), `dispatch-lib.sh` (`sed -n`) et
plusieurs `docs/solutions/**.md`. Aucun deny.

*Le groom qui devait être impossible vient de faire la chose qu'on disait
impossible.* C'est l'expérience contrôlée qui manquait au ticket : même fichier,
forme simple, allow.

### La conclusion, et elle tient dans les deux modes du classifier

Le classifier décide sur **le binaire** et sur **la forme de la commande**. Jamais
sur le fichier. En mode `per_spawn` (plugin mika#1817) par les preuves 1 et 2 ; en
mode classique tier1/tier2 par la même propriété — `SAFE_SHELL_COMMANDS` est une
liste de binaires, et la rustine `^(for\s.*do\s+.*\s)?jq\s` relevée dans
`docs/solutions/workflow-issues/2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md`
prouve que les boucles `for` y sont traitées au cas par cas, donc qu'une boucle
dont le corps n'est couvert par aucune rustine tombe en default-deny.

**La conclusion est donc indépendante de l'état de la bascule Phase 1/2/3**, ce qui
compte : personne n'a besoin de savoir quel mode est armé pour s'y fier.

---

## La décision demandée, et pourquoi ses deux branches sont refusées

Le ticket demande de trancher : *voulu* (→ documenter une contrainte de routage) ou
*over-block* (→ carve-out lecture seule). **Les deux branches supposent l'existence
d'une règle de contenu. Il n'y en a pas.** Répondre par l'une ou l'autre, c'est
répondre à côté — et chacune coûte quelque chose de réel.

**Branche « oui, c'est voulu » → contrainte de routage.** Elle consacrerait une
protection fantôme et exclurait de la boucle autonome toute une classe de tickets
(« le prompt X a grossi/dérivé ») qui sont, mesure à l'appui, parfaitement
groomables en sandbox. Le coût est direct et permanent : du travail routé à
l'orchestrateur sans raison, pour toujours, sur la foi d'une inférence.

**Branche « non, over-block » → carve-out lecture seule.** Pire que sans effet. Un
carve-out *de chemin* dans un registre qui décide *par binaire* introduirait la
première dimension « chemin » dans un classifier qui n'en a aucune — précisément
celle que le docstring de `_binaries.py` exclut explicitement. On créerait la
protection qu'on croyait lever, et on ouvrirait une surface de décision nouvelle
pour réparer un défaut inexistant.

**La réponse posée ici :** il n'existe aucune protection du contenu des
`system_prompt.md`, ni voulue ni accidentelle. Le deny de #2295 est un deny de
**forme** — la classe C documentée depuis le 2026-06-14 — et son remède connu est
de réémettre la commande en spawns simples.

---

## Ce qui a réellement coûté 2 h à #2295, et ce que ce plan ferme

Deux défauts distincts se sont enchaînés :

- **D1 — le pilote émet une forme refusée** et la session halte (`interrupt=True`).
  Le remède est que le deny soit *récupérable* : que le pilote apprenne « refusé :
  control flow » et réémette, au lieu de mourir. C'est **mika#1410**, dans
  claude-pilot, hors du périmètre dispatchable. **Hors périmètre ici, et nommé.**

- **D2 — le lecteur du deny infère la mauvaise cause.** Le message de classe C de
  `dispatch-lib.sh` nomme la commande déniée mais pas la *raison*, et propose deux
  remèdes — (a) élargir la policy, (b) réécrire le contexte de dispatch — dont
  aucun n'est le bon pour un deny de forme. C'est ce message qu'a lu l'opérateur,
  et c'est de sa lecture qu'est né ce ticket.

Ce plan ferme D2 et déclare la décision. **D1 reste à #1410.** Un plan qui
prétendrait fermer les deux absorberait un ticket amont dont la surface est dans un
autre dépôt.

---

## Conception

### U1 — Déclarer la décision (le cœur)

Un document de solution :
`docs/solutions/security-issues/2026-09-18-le-classifier-ne-decide-jamais-sur-le-contenu-dun-fichier.md`

Frontmatter YAML de la maison (`module`, `tags`, `problem_type: architecture-pattern`,
`category: security-issues`, `applies_when`, `resolution_type: pattern`).

Contenu :

1. **Le fait**, avec ses trois ancrages code/doc/mesure.
2. **La règle de lecture d'un `[policy:deny]`** : le marqueur nomme une commande,
   jamais un fichier. Avant toute autre hypothèse, chercher dans la commande une
   construction de forme — boucle, `if`, substitution `$(…)`, heredoc, `eval`,
   `bash -c`. Si elle y est, c'est elle. **Le remède premier est de réémettre en
   commandes simples**, pas d'élargir la policy et pas de router hors sandbox.
3. **Les deux branches refusées**, avec le coût de chacune (§ ci-dessus) — pour que
   la prochaine occurrence ne les re-propose pas.
4. **Le seul étage où la question se reposerait** : le troisième, le classifier
   LLM atteint par `canUseTool` (`.claude/claude-pilot.json` → `mika --agent
   mika-dev ask`). Un jugement sémantique pourrait, lui, porter sur le fichier
   nommé. Son signal distinctif : **le refus ne porte pas le marqueur
   `[policy:deny]`**, qui appartient à l'étage déterministe. Le deny de #2295 le
   portait. Le skill `permission-policy` étant retiré depuis mika#1193, aucun
   prompt dédié ne donne aujourd'hui cette consigne à mika-dev.
5. **La contre-mesure reproductible**, en une ligne, pour que quiconque puisse
   refaire l'expérience au lieu de refaire l'inférence.

### U2 — Rendre la déclaration durable (garde structurelle)

Aujourd'hui l'invariant « le classifier ne décide jamais sur le contenu » est vrai
**par construction** mais n'est écrit nulle part et rien ne l'empêche de cesser de
l'être. Un carve-out de bonne foi — exactement celui que ce ticket envisage —
l'enfreindrait sans qu'aucun test ne rougisse.

Nouveau fichier `tools/mika_permission_policy/tests/test_no_filesystem_access.py`,
deux gardes complémentaires :

**(a) Garde structurelle, par AST.** Parcourir `_binaries.py` et `__init__.py` et
refuser tout `import` d'un module d'accès au monde extérieur, ainsi que tout appel
au builtin `open`. Liste **nommée et justifiée dans le test**, pas une interdiction
totale d'import : `os`, `pathlib`, `io`, `glob`, `shutil`, `subprocess`, `socket`,
`urllib`, `requests` — plus `open`. `re` et `shlex` restent disponibles, une future
fonction de sûreté pouvant légitimement en avoir besoin. `collections.abc`, déjà
importé par `__init__.py` pour `Callable`, reste permis.

**(b) Pin comportemental.** `is_safe_cat`, `is_safe_head`, `is_safe_grep`,
`is_safe_sed`, `is_safe_tail` rendent le **même** verdict pour un jeu de chemins
incluant `skills/bundled/mika-arch-groom-ticket/system_prompt.md`,
`skills/bundled/<x>/skill.toml`, un chemin inexistant et un chemin hors worktree.
Le test affirme l'**indifférence au chemin**, pas un verdict particulier : c'est la
propriété qui est en jeu, et elle survit à un durcissement futur de `sed`.

**Pourquoi (a) en plus de (b).** Un carve-out ajouté plus tard passerait tous les
tests comportementaux existants et n'échouerait que sur le chemin exact
carve-outé — que personne n'aurait pensé à tester. (a) refuse la *capacité*, pas
une instance. C'est la forme de garde que la maison écrit déjà
(`mika2131_exclusion_skips_never_return_to_an_uncollected_debug`,
`mika2205_periodic_scans_do_not_read_the_pat_field_directly`) et pour la même
raison : la régression ne rendrait aucune décision fausse, elle lèverait
l'invariant en silence.

Cible existante : `make test-permission-policy-plugin`.

### U3 — Réparer le message qui a produit la mauvaise inférence

`skills/bundled/_shared/dispatch-lib.sh` porte **deux** blocs de classe C, et les
deux doivent recevoir l'ajout — sous peine d'un message juste une fois sur deux :

| Ligne | En-tête | Chemin |
|---|---|---|
| ~3600 | `PIPELINE FAILURE: claude-pilot session halted by policy deny — not generic exit.` | dev-pilot, HEAD-unchanged |
| ~3785 | `PIPELINE FAILURE: dev-groom session halted by claude-pilot policy deny — not LLM drift.` | dev-groom, drift |

**Les en-têtes diffèrent légitimement** (chacun nie le diagnostic que son chemin
aurait produit) : ne pas les uniformiser. C'est le paragraphe de remèdes, identique
sur les deux sites, qui reçoit la troisième voie — insérée **avant** « Likely a
tier1 or tier2 allow-list gap… », car la forme doit être écartée avant qu'on parte
chercher un trou d'allow-list :

> The deny is about **the command**, never about the file it reads. First look in
> the command above for a *shape* construct — `for`/`while` loop, `if`,
> `$(…)` substitution, heredoc, `eval`, `bash -c`: the classifier refuses these at
> the raw-source level, before any argument is examined. If that is what happened,
> the fix is **(c) re-issue the same read as simple commands** (`cat`, `head`,
> `sed -n`, one file at a time) — neither a policy widening nor a route out of the
> sandbox.

Rédigé en anglais : c'est la langue du bloc hôte, et un paragraphe français inséré
au milieu d'un message anglais se lirait comme une greffe.

Assertion correspondante dans `skills/bundled/_shared/test-dispatch-lib.sh`, à côté
des `assert_contains` existants sur ce bloc.

**Coût assumé :** `dispatch-lib.sh` est DECISION-CORE et gate en entier
(`perimeter/rules.rs`). Une PR humaine est requise. C'est le régime correct pour un
ticket de sûreté, et le changement est du texte de diagnostic — aucun prédicat, aucun
flux.

---

## Unités d'implémentation

| # | Fichier | Nature | Dépend de |
|---|---|---|---|
| U1 | `docs/solutions/security-issues/2026-09-18-le-classifier-ne-decide-jamais-sur-le-contenu-dun-fichier.md` | création | — |
| U2a | `tools/mika_permission_policy/tests/test_no_filesystem_access.py` | création (garde AST) | — |
| U2b | idem, second bloc | création (pin comportemental) | U2a |
| U3a | `skills/bundled/_shared/dispatch-lib.sh` (2 sites) | édition texte | U1 |
| U3b | `skills/bundled/_shared/test-dispatch-lib.sh` | assertion | U3a |

Ordre : U1 → U2 → U3. U1 d'abord parce que U3 cite sa doctrine ; U2 est
indépendante et peut être vérifiée seule.

---

## Fire-Disposition

**Rien ne change de comportement à l'exécution.** Aucun prédicat, aucun seuil,
aucune variable d'environnement, aucun flux de dispatch n'est touché. Le classifier
rend exactement les mêmes verdicts après ce travail qu'avant — c'est le propre d'un
ticket dont l'objet est de *déclarer* ce que le substrat fait déjà.

Ce qui change : une chaîne de diagnostic lue par un opérateur et par le pilote quand
un deny a tué la session ; un test qui rougit si l'invariant est enfreint ; un
document de doctrine.

Aucun déploiement n'est requis pour U1 et U2. U3 prend effet au prochain
`seed_support_dirs` (redémarrage du démon, qui réécrit le `dispatch-lib.sh` installé).

---

## Contrat de vérification

| Quoi | Comment | Attendu |
|---|---|---|
| Garde AST | `make test-permission-policy-plugin` | vert sur la base actuelle |
| Garde AST mord | ajouter `import os` à `_binaries.py`, relancer | **rouge**, en nommant le module |
| Pin comportemental | idem cible | vert, chemins indifférents |
| Message classe C | `make test-dispatch-lib` | vert, assertion sur la 3ᵉ voie |
| Symétrie des 2 sites | `grep -c 're-issue the same read as simple commands' skills/bundled/_shared/dispatch-lib.sh` | `2` |
| Mesure du fait | depuis un pilote sandboxé : `head -5 skills/bundled/mika-arch-groom-ticket/system_prompt.md` | contenu lu, aucun deny |

La cinquième ligne existe parce que le bloc de classe C est dupliqué dans
`dispatch-lib.sh` : réparer un site et pas l'autre donnerait un message juste une
fois sur deux, ce qui est plus trompeur qu'un message uniformément incomplet.

---

## Definition of Done

- Le document U1 est écrit, avec les trois ancrages (code, doc, mesure) cités par
  chemin et par ligne.
- La garde U2 est verte sur la base actuelle et rouge sur une violation introduite
  à la main.
- Les deux sites de U3 portent le même paragraphe, et le test de dispatch-lib
  l'asserte.
- Aucune règle de chemin ni de contenu n'a été ajoutée au classifier.
- Le plan nomme mika#1410 comme propriétaire de D1 et ne prétend pas le fermer.

## Acceptance criteria

*(le corps de mika#2312 ne porte pas de section `## Acceptance criteria` ; ceux-ci
sont dérivés de sa question — « décision à prendre, pas un contournement » — et du
contrat de vérification ci-dessus.)*

- **AC1** — La décision demandée est déclarée par écrit dans `docs/solutions/`,
  avec ses trois preuves indépendantes et la raison pour laquelle **aucune** des
  deux branches proposées par le ticket ne s'applique.
- **AC2** — Aucun carve-out — de chemin, de motif ou de contenu — n'est ajouté au
  classifier. Le diff sur `tools/mika_permission_policy/mika_permission_policy/`
  est vide.
- **AC3** — Une garde refuse structurellement qu'une fonction du registre accède
  au système de fichiers, et cette garde est démontrée mordante (rouge sur
  violation introduite).
- **AC4** — Le message de classe C de `dispatch-lib.sh` nomme la forme de la
  commande comme cause possible et la réémission en commandes simples comme
  remède, **sur ses deux sites**.
- **AC5** — Aucune contrainte de routage « ces tickets ne sont pas groomables en
  sandbox » n'est documentée. La doctrine écrite dit l'inverse et cite la mesure
  du 2026-09-18 qui l'établit.

---

## Risques, et ce qui les borne

**R1 — La garde U2 ne couvre que la moitié « plugin » du classifier.** Les étages
tier1/tier2 vivent dans `claude-pilot`, hors du périmètre dispatchable de la
boucle. L'invariant y est vrai aujourd'hui (listes de binaires, regex sur chaînes
de commande — aucune lecture de fichier), mais rien ne l'y garde. *Borne :* la
conclusion du ticket ne dépend pas de la garde, seulement des preuves ; la garde
protège l'avenir du seul côté où nous pouvons écrire. Un jumeau côté claude-pilot
est un **ticket de suivi**, à ouvrir, pas à absorber.

**R2 — Le troisième étage (classifier LLM) n'est pas couvert par une garde
déterministe**, et ne peut pas l'être : c'est un jugement. *Borne :* son signal
distinctif est documenté en U1 — l'absence du marqueur `[policy:deny]` — de sorte
qu'une occurrence future soit attribuée au bon étage au lieu de relancer cette
enquête.

**R3 — `dispatch-lib.sh` est DECISION-CORE, la PR gate en entier.** *Borne :*
assumé ; le changement est du texte de diagnostic, sans prédicat ni flux, ce qui
rend la revue humaine courte.

**R4 — Le mode réellement armé en production n'a pas été vérifié** (les variables
`MIKA_PERMISSION_POLICY_MODE` / `_MODULE` ne sont lisibles ni depuis le sandbox ni
depuis le dépôt). *Borne :* c'est précisément pourquoi la conclusion est
argumentée dans **les deux** modes. Si elle ne tenait que dans l'un, elle serait
suspendue à une vérification que ce plan ne peut pas faire.

---

## Sondes post-déploiement, et leurs haltes

**Sonde 1 — la classe C est-elle correctement attribuée ?** Sur les 30 jours
suivants, pour chaque `PIPELINE FAILURE: … halted by policy deny` surfacé, relever
la commande citée et classer : forme (boucle, substitution, heredoc, `eval`) vs
binaire absent du registre. *Attendu :* la forme domine, conformément à #1686
(« chaque nouvel idiome opérateur engendre un n=1 »).

**Halte :** si un deny tombe sur un **spawn simple lisant un `system_prompt.md`**,
tout ce plan est réfuté — **ne pas ajouter de carve-out par réflexe**. Ce serait
soit un troisième étage devenu actif (vérifier l'absence du marqueur
`[policy:deny]`), soit un changement du classifier amont. Établir lequel d'abord.

**Sonde 2 — la déclaration est-elle lue ?** Sur les tickets futurs portant sur le
contenu d'un `system_prompt.md`, vérifier qu'ils sont dispatchés en sandbox et non
routés à l'orchestrateur. *Halte :* un routage manuel malgré la doctrine signifie
que le document est écrit au mauvais endroit — le remède est de le rapprocher du
canal que l'opérateur lit, pas de le réécrire plus fort.

---

## Hors périmètre, délibérément

- **mika#1410 — deny récupérable.** Le vrai remède à D1 : que le pilote survive à
  un deny et réémette. Dans `claude-pilot`, hors de la boucle dispatchable.
- **Élargir tier1/tier2 pour accepter les boucles `for`.** Ce serait rouvrir la
  surface compound que mika#1686 et mika#1708 ont fermée **par conception** : le
  refus du control flow au niveau source brut *est* le fix de la classe des
  n=13+ idiomes. Une décision de sûreté amont, avec son propre ticket.
- **La bascule Phase 2 / Phase 3 du mode `per_spawn`** (retrait des chemins Bash
  de `tier1.py`), suivi de mika#1817.
- **mika#2306**, nommé par le ticket comme relié. Non lu ici (pas de jeton `gh` en
  sandbox), non absorbé. S'il porte sur la même inférence, ce document est ce
  qu'il faut lui opposer ; s'il porte sur autre chose, il garde sa surface.
- **Le prompt de dev-groom.** Une ligne « n'émets pas de boucle `for` » est de
  l'enforcement par prompt sur substrat de boucle, que
  `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` réfute
  empiriquement (9 récurrences contre 0). Le canal qui tient est le message de
  deny, que U3 répare.

---

## Revision history

- 2026-09-18 — rédaction initiale (dev-groom, mika#2312). Le diagnostic du ticket
  est rectifié sur trois preuves, dont une mesure faite par le pilote de ce groom
  lui-même.
