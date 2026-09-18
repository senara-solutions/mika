# fix(permission-policy) : le classifier ne décide jamais sur le contenu d'un fichier — et la cause du deny de #2295 n'est pas dans le ticket (mika#2312)

- **Ticket :** mika issue#2312
- **Type :** fix (décision de sûreté déclarée + garde structurelle + message de diagnostic)
- **Date :** 2026-09-18
- **Relie :** mika#2306 (nommé par le ticket, non absorbé), mika#1410, mika#1686, mika#1708, mika#1817

---

## Ce que le ticket affirme, et ce que la mesure établit

Le ticket compare deux événements du log claude-pilot `3587fe25` (2026-09-14, groom
#2295) :

| Heure | Commande | Verdict |
|---|---|---|
| 21:06 | `cat skills/bundled/<skill>/skill.toml` | `[policy:allow]` |
| 21:11:57 | `for f in skills/bundled/mika-arch-groom-ticket/system_prompt.md skills/bundled/mika-arch-second-review/system_prompt.md; do …` | `[policy:deny]` |

et conclut : « C'est donc le CONTENU des system_prompt qui est protégé, pas le
chemin/pattern. »

**La comparaison n'est pas contrôlée.** Les deux commandes diffèrent sur deux axes —
le fichier lu *et* la forme de la commande — et l'inférence attribue au premier un
effet que le second pourrait expliquer. Le présent groom, dispatché en sandbox sous
le même classifier et dans le même rôle dev-groom que #2295, a fait l'expérience
contrôlée qui manquait.

### Les quatre mesures, faites le 2026-09-18 depuis un pilote sandboxé

| # | Commande émise | Verdict |
|---|---|---|
| M1 | `head -5 skills/bundled/mika-arch-groom-ticket/system_prompt.md` | **allow** — contenu lu |
| M2 | `for f in skills/bundled/dev-pilot/skill.toml; do head -2 "$f"; done` | **allow** |
| M3 | `for f in skills/bundled/mika-arch-groom-ticket/system_prompt.md skills/bundled/mika-arch-second-review/system_prompt.md; do echo "=== $f ==="; wc -l "$f"; done` | **allow** |
| M4 | une chaîne `ls … \| head -3` + `git log …` + `env \| grep …` avec `&&` et `\|\|` | **veto** — `policy allow (bash-grep) vetoed — command chains a tier3-dangerous or command-substitution tail onto the allowed prefix` |

**M3 est la mesure décisive** : c'est la commande du ticket — mêmes fichiers, même
boucle `for … do … done` — et elle passe. M1 réfute la thèse du contenu ; M3 réfute
aussi la thèse de la forme `for`, qui était la conclusion de la première rédaction
de ce plan. M4 est la seule mesure *positive* : ce qui est refusé, ce jour-là, est
la **chaîne composée**, et le classifier le dit dans son propre message.

Ces mesures valent pour le 14 septembre : `git log --since=2026-09-01 --
tools/mika_permission_policy/` est **vide**, donc l'étage déterministe n'a pas
changé entre les deux dates.

### L'ancrage structurel — le registre ne peut pas lire un fichier

`tools/mika_permission_policy/mika_permission_policy/_binaries.py` n'importe que
`from __future__ import annotations` (ligne 28) ; `__init__.py` n'ajoute que
`collections.abc.Callable`. Aucun `os`, `pathlib`, `io`, ni `open`. Toutes les
fonctions ont la signature `(argv, cwd) -> bool` et `is_safe_cat` (ligne 46) rend
`True` inconditionnellement. Son docstring l'énonce :

> **What we do NOT check here** — Path containment for filesystem targets: this is
> a Write/Edit tool concern, not a Bash one, and tier1 doesn't check it either.

Le contenu n'est pas « autorisé » ni « refusé » : **il n'est pas une dimension de
décision**. C'est vrai par construction, pas par réglage — et c'est ce qui rend la
conclusion indépendante de l'état de la bascule Phase 1/2/3, que ce plan n'a pas
besoin de connaître.

---

## Ce qui reste indéterminé, et pourquoi le ticket ne peut pas le trancher

La cause exacte du deny de #2295 **n'est pas établissable depuis les éléments du
ticket**, et il faut le dire plutôt que de lui substituer une hypothèse.

Le marqueur réel a la forme `[policy:deny] <Tool>: <commande> [<rule-id>]` — le
commentaire de `dispatch-lib.sh:3779-3780` la documente, et la regex d'extraction du
doc du 2026-06-14 (`\[policy:deny\] [A-Za-z]+: [^[]+\[[a-z-]+\]`) la confirme. **Le
`<rule-id>` final est ce qui attribue la cause**, et le ticket ne le cite pas : il
coupe la commande à `do …` et s'arrête avant le crochet.

Trois causes restent compatibles avec ce qu'on sait, et le `rule-id` les
départagerait en un coup d'œil :

1. **Le corps tronqué de la boucle** — le `…` du ticket masque ce que faisait le
   `do`. S'il chaînait (`|`, `&&`) ou substituait (`$(…)`), c'est M4 qui s'applique.
2. **Le veto de chaîne (`chain-safety`)**, dont `_split_compound_command` a un
   historique documenté de faux positifs sur des greps à alternation
   (`2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md:69`).
3. **Le troisième étage**, le classifier LLM atteint par `canUseTool`
   (`.claude/claude-pilot.json` → `mika --agent mika-dev ask`, timeout 120 s). Étant
   un jugement, il peut diverger entre deux dates sans qu'aucun code ait changé —
   ce que M1–M3 ne peuvent pas exclure. Son signal distinctif est l'**absence de
   `rule-id`** : les règles déterministes en portent un, un jugement n'en a pas.

**Aucune des trois n'est une règle de contenu.** La conclusion du ticket ne dépend
donc pas de savoir laquelle a mordu — et c'est pourquoi ce plan tranche la décision
demandée sans attendre le log.

---

## La décision demandée, et pourquoi ses deux branches sont refusées

Le ticket demande de trancher : *voulu* (→ documenter une contrainte de routage) ou
*over-block* (→ carve-out lecture seule). **Les deux branches supposent l'existence
d'une règle de contenu. Il n'y en a pas.** Répondre par l'une ou l'autre coûte
quelque chose de réel.

**Branche « oui, c'est voulu » → contrainte de routage.** Elle consacrerait une
protection fantôme et exclurait de la boucle autonome toute une classe de tickets
(« le prompt X a grossi/dérivé ») que M1 et M3 montrent parfaitement groomables en
sandbox. Coût direct et permanent : du travail routé à l'orchestrateur pour
toujours, sur la foi d'une inférence que quatre commandes réfutent.

**Branche « non, over-block » → carve-out lecture seule.** Pire que sans effet. Un
carve-out *de chemin* dans un registre qui décide *par binaire* introduirait la
première dimension « chemin » d'un classifier qui n'en a aucune — précisément celle
que le docstring de `_binaries.py` exclut. On créerait la protection qu'on croyait
lever, pour réparer un défaut inexistant.

**La réponse posée ici :** il n'existe aucune protection du contenu des
`system_prompt.md`, ni voulue ni accidentelle. Ces tickets sont groomables en
sandbox. Le deny de #2295 vise une commande, pas un fichier, et son `rule-id` dit
laquelle — information que le message transporte déjà et que personne n'a lue.

---

## Ce qui a réellement coûté 2 h à #2295

Deux défauts distincts se sont enchaînés :

- **D1 — le pilote émet une forme refusée et la session halte** (`interrupt=True`).
  Le remède est que le deny soit *récupérable* : que le pilote apprenne « refusé :
  <règle> » et réémette au lieu de mourir. C'est **mika#1410**, dans claude-pilot,
  hors du périmètre dispatchable. **Hors périmètre ici, et nommé.**

- **D2 — le lecteur du deny infère la mauvaise cause.** Le message de classe C
  affiche la ligne complète (rule-id compris) mais n'apprend pas à la lire : son
  paragraphe de remèdes envoie directement vers « élargir la policy » ou « réécrire
  le contexte de dispatch », sans jamais dire que le deny nomme une **commande et
  une règle**, jamais un fichier. C'est ce message qu'a lu l'opérateur, et c'est de
  sa lecture qu'est né ce ticket.

Ce plan ferme D2 et déclare la décision. **D1 reste à #1410.**

---

## Conception

### U1 — Déclarer la décision (le cœur)

Un document de solution :
`docs/solutions/security-issues/2026-09-18-le-classifier-ne-decide-jamais-sur-le-contenu-dun-fichier.md`

Frontmatter YAML de la maison (`module`, `tags`, `problem_type: architecture-pattern`,
`category: security-issues`, `applies_when`, `resolution_type: pattern`).

Contenu :

1. **Le fait**, avec ses deux ancrages : structurel (le registre n'a aucun moyen
   d'ouvrir un fichier — imports, signatures, docstring, cités par ligne) et
   expérimental (le tableau M1–M4, reproductible en quatre commandes).
2. **La règle de lecture d'un `[policy:deny]`**, qui est la doctrine utile :
   - le marqueur nomme **une commande et une règle**, jamais un fichier ;
   - **lire d'abord le `[<rule-id>]` en fin de ligne** — c'est lui qui attribue la
     cause, et il est déjà dans le message ;
   - **ne jamais tronquer la commande** en la rapportant : le `…` du ticket a
     emporté l'information qui aurait clos l'enquête ;
   - un refus **sans** `rule-id` désigne le troisième étage (jugement LLM), pas
     l'étage déterministe — et ne se répare pas par un carve-out.
3. **Les deux branches refusées**, avec le coût de chacune, pour que la prochaine
   occurrence ne les re-propose pas.
4. **Le piège de méthode**, nommé comme tel : la première rédaction de ce plan avait
   conclu « c'est la boucle `for` » sur la foi d'un document décrivant `decompose()`,
   et M3 l'a réfutée. *Une doctrine tirée d'une doc plutôt que d'une mesure reproduit
   exactement l'erreur qu'elle prétend corriger.* C'est la raison d'être du point 2.

### U2 — Rendre la déclaration durable (garde structurelle)

L'invariant « le classifier ne décide jamais sur le contenu » est vrai **par
construction** mais n'est écrit nulle part, et rien ne l'empêche de cesser de l'être.
Un carve-out de bonne foi — exactement celui que ce ticket envisage — l'enfreindrait
sans qu'aucun test ne rougisse.

Nouveau fichier `tools/mika_permission_policy/tests/test_no_filesystem_access.py`
(ramassé automatiquement : la cible fait `uv run pytest -q` sur le répertoire), deux
gardes complémentaires :

**(a) Garde structurelle, par AST.** Parcourir `_binaries.py` et `__init__.py`,
refuser tout `import` d'un module d'accès au monde extérieur ainsi que tout appel au
builtin `open`. Liste **nommée et justifiée dans le test**, pas une interdiction
totale d'import : `os`, `pathlib`, `io`, `glob`, `shutil`, `subprocess`, `socket`,
`urllib`, `requests`, plus `open`. `re` et `shlex` restent disponibles — une future
fonction de sûreté peut légitimement en avoir besoin. `collections.abc`, déjà importé
par `__init__.py`, reste permis.

**(b) Pin comportemental.** `is_safe_cat`, `is_safe_head`, `is_safe_grep`,
`is_safe_sed`, `is_safe_tail` rendent le **même** verdict pour un jeu de chemins
incluant `skills/bundled/mika-arch-groom-ticket/system_prompt.md`, un `skill.toml`
voisin, un chemin inexistant et un chemin hors worktree. Le test affirme
l'**indifférence au chemin**, pas un verdict particulier : c'est la propriété qui est
en jeu, et elle survit à un durcissement futur de `sed`.

**Pourquoi (a) en plus de (b).** Un carve-out ajouté plus tard passerait tous les
tests comportementaux existants et n'échouerait que sur le chemin exact carve-outé —
que personne n'aurait pensé à tester. (a) refuse la *capacité*, pas une instance.
C'est la forme de garde que la maison écrit déjà
(`mika2131_exclusion_skips_never_return_to_an_uncollected_debug`,
`mika2205_periodic_scans_do_not_read_the_pat_field_directly`) et pour la même raison :
la régression ne rendrait aucune décision fausse, elle lèverait l'invariant en
silence.

Cible existante : `make test-permission-policy-plugin` (Makefile:80).

### U3 — Réparer le message qui a produit la mauvaise inférence

`skills/bundled/_shared/dispatch-lib.sh` porte **deux** blocs de classe C, et les
deux doivent recevoir l'ajout — sous peine d'un message juste une fois sur deux :

| Ligne | En-tête | Chemin |
|---|---|---|
| 3600 | `PIPELINE FAILURE: claude-pilot session halted by policy deny — not generic exit.` | dev-pilot, HEAD-unchanged |
| 3789 | `PIPELINE FAILURE: dev-groom session halted by claude-pilot policy deny — not LLM drift.` | dev-groom, drift |

**Deux choses à ne pas uniformiser.** Les en-têtes diffèrent légitimement (chacun nie
le diagnostic que *son* chemin aurait produit). Les paragraphes de remèdes diffèrent
aussi — « the legitimate command shape » / « re-dispatching » à 3604 contre « the
legitimate **research** command shape » / « re-grooming this ticket » à 3793 : ce ne
sont pas deux copies d'un même bloc, et l'implémenteur qui chercherait un texte
identique à remplacer ne le trouvera pas. C'est **la même phrase ajoutée aux deux
endroits**, insérée **avant** « Likely a tier1 or tier2 allow-list gap… » car il faut
lire la règle avant de partir chercher un trou d'allow-list :

> Read the `[rule-id]` in brackets at the end of the halt event first: the deny names
> **a command and a rule**, never a file it reads. A deny with no `[rule-id]` did not
> come from the deterministic classifier at all — it came from the `canUseTool`
> judgment stage, and no allow-list change will affect it. Widening the policy (a) or
> rewriting the dispatch context (b) are only meaningful once the rule is known.

Rédigé en anglais : c'est la langue du bloc hôte, et un paragraphe français inséré au
milieu se lirait comme une greffe.

**Deux assertions** dans `skills/bundled/_shared/test-dispatch-lib.sh`, une par site,
à côté des `assert_contains` existants (lignes 2987 et 3080). Une seule assertion ne
mordrait que sur un bloc — exactement l'asymétrie que ce U3 existe pour empêcher.

**Coût assumé :** `dispatch-lib.sh` est DECISION-CORE et gate en entier
(`perimeter/rules.rs:41` — « whole file gates »). Une PR humaine est requise. C'est le
régime correct pour un ticket de sûreté, et le changement est du texte de diagnostic —
aucun prédicat, aucun flux.

---

## Unités d'implémentation

| # | Fichier | Nature | Dépend de |
|---|---|---|---|
| U1 | `docs/solutions/security-issues/2026-09-18-le-classifier-ne-decide-jamais-sur-le-contenu-dun-fichier.md` | création | — |
| U2a | `tools/mika_permission_policy/tests/test_no_filesystem_access.py` | création (garde AST) | — |
| U2b | idem, second bloc | création (pin comportemental) | U2a |
| U3a | `skills/bundled/_shared/dispatch-lib.sh` (2 sites : 3604, 3793) | édition texte | U1 |
| U3b | `skills/bundled/_shared/test-dispatch-lib.sh` (2 assertions) | assertions | U3a |

Ordre : U1 → U2 → U3. U1 d'abord parce que U3 en cite la doctrine ; U2 est
indépendante et vérifiable seule.

---

## Fire-Disposition

**Rien ne change de comportement à l'exécution.** Aucun prédicat, aucun seuil, aucune
variable d'environnement, aucun flux de dispatch n'est touché. Le classifier rend
exactement les mêmes verdicts après ce travail qu'avant — c'est le propre d'un ticket
dont l'objet est de *déclarer* ce que le substrat fait déjà.

Ce qui change : une chaîne de diagnostic lue par un opérateur quand un deny a tué la
session ; un test qui rougit si l'invariant est enfreint ; un document de doctrine.

Aucun déploiement n'est requis pour U1 et U2. U3 prend effet au prochain
`seed_support_dirs` (redémarrage du démon, qui réécrit le `dispatch-lib.sh` installé).

---

## Contrat de vérification

| Quoi | Comment | Attendu |
|---|---|---|
| Garde AST | `make test-permission-policy-plugin` | vert sur la base actuelle |
| Garde AST mord | ajouter `import os` à `_binaries.py`, relancer | **rouge**, en nommant le module |
| Pin comportemental | idem cible | vert, chemins indifférents |
| Message classe C | `make test-dispatch-lib` | vert, 2 assertions sur la 3ᵉ voie |
| Symétrie des 2 sites | `grep -c 'Read the .rule-id. in brackets' skills/bundled/_shared/dispatch-lib.sh` | `2` |
| Reproduction de M1 | depuis un pilote sandboxé : `head -5 skills/bundled/mika-arch-groom-ticket/system_prompt.md` | contenu lu, aucun deny |
| Reproduction de M3 | la boucle `for` exacte du ticket | passe, aucun deny |

La cinquième ligne existe parce que le bloc de classe C est dupliqué : réparer un site
et pas l'autre donnerait un message juste une fois sur deux, plus trompeur qu'un
message uniformément incomplet.

---

## Definition of Done

- Le document U1 est écrit, avec ses ancrages cités par chemin et par ligne, et le
  tableau M1–M4 reproductible.
- La garde U2 est verte sur la base actuelle et rouge sur une violation introduite à
  la main.
- Les deux sites de U3 portent la même phrase, et deux assertions l'attestent.
- Aucune règle de chemin ni de contenu n'a été ajoutée au classifier.
- Le plan nomme mika#1410 comme propriétaire de D1 et ne prétend pas le fermer.
- La cause exacte du deny de #2295 est déclarée **indéterminée**, avec la procédure
  pour la trancher, plutôt que remplacée par une hypothèse.

## Acceptance criteria

*(le corps de mika#2312 ne porte pas de section `## Acceptance criteria` ; ceux-ci
sont dérivés de sa question — « décision à prendre, pas un contournement » — et du
contrat de vérification ci-dessus.)*

- **AC1** — La décision demandée est déclarée par écrit dans `docs/solutions/`, avec
  ses preuves et la raison pour laquelle **aucune** des deux branches proposées par
  le ticket ne s'applique.
- **AC2** — Aucun carve-out — de chemin, de motif ou de contenu — n'est ajouté au
  classifier. Le diff sur `tools/mika_permission_policy/mika_permission_policy/` est
  vide.
- **AC3** — Une garde refuse structurellement qu'une fonction du registre accède au
  système de fichiers, et cette garde est démontrée mordante.
- **AC4** — Le message de classe C nomme le `[rule-id]` comme premier élément à lire
  et pose que le deny vise une commande, jamais un fichier — **sur ses deux sites**.
- **AC5** — Aucune contrainte de routage « ces tickets ne sont pas groomables en
  sandbox » n'est documentée. La doctrine écrite dit l'inverse et cite les mesures du
  2026-09-18 qui l'établissent.

---

## Risques, et ce qui les borne

**R1 — La garde U2 ne couvre que la moitié « plugin » du classifier.** Les étages
tier1/tier2 vivent dans `claude-pilot`, hors du périmètre dispatchable. L'invariant y
est vrai aujourd'hui (listes de binaires, regex sur chaînes de commande — aucune
lecture de fichier), mais rien ne l'y garde. *Borne :* la conclusion ne dépend pas de
la garde, seulement des mesures ; la garde protège l'avenir du seul côté où nous
pouvons écrire. Un jumeau côté claude-pilot est un **ticket de suivi**.

**R2 — Le troisième étage (jugement LLM) n'est pas couvert par une garde
déterministe**, et ne peut pas l'être. *Borne :* son signal distinctif — l'absence de
`rule-id` — est documenté en U1 et porté par le message en U3, de sorte qu'une
occurrence future soit attribuée au bon étage au lieu de relancer cette enquête.
**C'est aussi la seule hypothèse que M1–M3 ne peuvent pas exclure pour #2295** : un
jugement peut avoir refusé le 14 ce qu'il autorise le 18.

**R3 — `dispatch-lib.sh` est DECISION-CORE, la PR gate en entier.** *Borne :* assumé ;
le changement est du texte de diagnostic, sans prédicat ni flux, ce qui rend la revue
humaine courte.

**R4 — Les mesures M1–M4 portent sur le mode réellement armé le 2026-09-18, sans que
ce mode ait été nommé** (les variables `MIKA_PERMISSION_POLICY_MODE` / `_MODULE` ne
sont pas lisibles depuis le sandbox). *Borne :* c'est précisément pourquoi la
conclusion s'appuie sur l'ancrage structurel — le registre ne *peut pas* lire un
fichier — qui vaut dans tous les modes, et non sur les seules mesures. L'absence de
commit sur le plugin depuis le 01/09 étend leur validité au 14/09.

---

## Sondes post-déploiement, et leurs haltes

**Sonde 0 — trancher #2295 pour de bon.** Récupérer la ligne complète du deny :
`grep -m1 'policy:deny' /var/log/claude-pilot/3587fe25.stderr` (sur l'hôte, hors
sandbox), et lire le `[rule-id]` final. *Attendu :* une règle de chaîne ou de
substitution, ou aucune règle du tout (→ troisième étage). **Halte :** si le
`rule-id` nomme une règle de chemin ou de fichier, ce plan est réfuté et la branche
« over-block » du ticket redevient ouverte — mais elle se traiterait alors dans
claude-pilot, pas ici.

**Sonde 1 — la classe C est-elle correctement attribuée ?** Sur les 30 jours suivants,
pour chaque `PIPELINE FAILURE: … halted by policy deny`, relever le `rule-id` et
classer. *Attendu :* chaîne/substitution dominante, conformément à #1686 (« chaque
nouvel idiome opérateur engendre un n=1 »).

**Halte :** si un deny tombe sur un **spawn simple lisant un `system_prompt.md`**,
tout ce plan est réfuté — **ne pas ajouter de carve-out par réflexe**. Vérifier
d'abord la présence du `rule-id` : sans lui, c'est le troisième étage, et un
carve-out déterministe n'y changerait rien.

**Sonde 2 — la déclaration est-elle lue ?** Sur les tickets futurs portant sur le
contenu d'un `system_prompt.md`, vérifier qu'ils sont dispatchés en sandbox et non
routés à l'orchestrateur. *Halte :* un routage manuel malgré la doctrine signifie que
le document est écrit au mauvais endroit — le remède est de le rapprocher du canal que
l'opérateur lit, pas de le réécrire plus fort.

---

## Hors périmètre, délibérément

- **mika#1410 — deny récupérable.** Le vrai remède à D1 : que le pilote survive à un
  deny et réémette. Dans `claude-pilot`, hors de la boucle dispatchable.
- **Élargir tier1/tier2 pour accepter les chaînes composées.** Ce serait rouvrir la
  surface compound que mika#1686 et mika#1708 ont fermée **par conception**. Une
  décision de sûreté amont, avec son propre ticket. Noter que
  `_split_compound_command` a un historique de faux positifs documenté
  (2026-06-14, §69) : s'il faut y revenir, c'est par là, pas par un carve-out de
  chemin.
- **La bascule Phase 2 / Phase 3 du mode `per_spawn`**, suivi de mika#1817.
- **mika#2306**, nommé par le ticket comme relié. Non lu ici (pas de jeton `gh` en
  sandbox), non absorbé. S'il porte sur la même inférence, ce document est ce qu'il
  faut lui opposer ; s'il porte sur autre chose, il garde sa surface.
- **Le prompt de dev-groom.** Une ligne « n'émets pas de commandes chaînées » serait de
  l'enforcement par prompt sur substrat de boucle, que
  `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` réfute
  empiriquement (9 récurrences contre 0). Le canal qui tient est le message de deny,
  que U3 répare.

---

## Revision history

- 2026-09-18 — rédaction initiale (dev-groom, mika#2312). Concluait « c'est la forme
  `for` qui est refusée », sur la foi de
  `docs/solutions/security-issues/1817-…md:74` (control flow refusé au niveau source
  brut).
- 2026-09-18 — **révision après mesure.** M3 réfute cette conclusion : la boucle
  exacte du ticket passe. Le socle de preuves est remplacé par quatre mesures
  directes ; U1 et U3 enseignent désormais la lecture du `[rule-id]` au lieu d'une
  recherche de forme ; la cause de #2295 est déclarée indéterminée avec sa procédure
  de résolution (Sonde 0). La conclusion principale — aucune règle de contenu — est
  inchangée et repose désormais sur un ancrage structurel plutôt que documentaire.
