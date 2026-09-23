# mika#2493 — Un deny non-terminal survécu est une note, jamais un « halted »

**Ticket :** mika issue#2493 (sous-issue de mika issue#2491, umbrella loop-substrate — Défaut 2)
**Type :** fix (substrat de boucle — `skills/bundled/_shared/dispatch-lib.sh`)
**Date :** 2026-09-23

---

## Problème

Le champ `result` d'un dispatch préfixe « PIPELINE FAILURE: … halted by policy
deny » alors que le refus n'a pas arrêté la session et qu'elle a réussi. Le
libellé a fait conclure « échec » à tort deux fois — opérateur **et**
orchestrateur — dans l'incident de la nuit du 2026-09-22.

### M0 — Les deux preuves du ticket, re-mesurées

Les deux sessions nommées par le ticket existent encore sous
`/var/log/claude-pilot/`. Mesure refaite plutôt que reprise :

```bash
grep -c 'policy:deny' /var/log/claude-pilot/98b60020-*.stderr          # 2
grep -o -E '\((non-)?terminal\)' /var/log/claude-pilot/98b60020-*.stderr | sort | uniq -c
#       2 (non-terminal)
grep -o -E '\((non-)?terminal\)' /var/log/claude-pilot/a0886164-*.stderr | sort | uniq -c
#       5 (non-terminal)
```

**Aucun des sept refus de ces deux sessions n'est terminal.** Les deux ont
abouti (`status: success`, et `Outcome: PLAN_GROOMED` pour la première). Le
défaut est confirmé sur sa population fondatrice.

### M1 — Le défaut n'est pas dans le libellé, il est dans l'ORDRE de la chaîne

Deux sites lisent `POLICY_DENY` (`dispatch-lib.sh`, seuls sites — `grep -n
'POLICY_DENY'` rend 4138/4141/4149/4155 et 4331/4341/4345/4352) :

| Site | Ligne | Garde d'entrée | Branches |
|---|---|---|---|
| A | ~4149 | HEAD inchangé | deny → Note dev-groom saine → échec générique |
| B | ~4345 | **aucune** (tout dev-groom) | deny → trois échecs « pas de plan » → note advisory saine |

Le site B teste `POLICY_DENY` **en tête de chaîne, sans condition**. Un
dev-groom qui a produit son plan, l'a commité et a réussi traverse donc la
branche C dès qu'un refus quelconque traîne dans son stderr. C'est la cause
directe de M0.

Et l'intention correcte est écrite dans le commentaire du site, trois lignes
au-dessus du défaut (`dispatch-lib.sh:4327`) :

> *« Disambiguate by reading the persistent stderr for `[policy:deny]` **before
> declaring drift**. »*

Le refus devait **désambiguïser un échec déjà établi**. Il a été implanté comme
**diagnostic prioritaire**. C'est l'écart entre les deux qui produit le
mensonge : il n'y a rien à désambiguïser quand la session a livré.

### M2 — Le marqueur de létalité n'est pas là où le code le cherche

`grep -m1 '\[policy:deny\]'` capture **une ligne**. Or le `<detail>` d'un refus
est multi-ligne dès que la commande l'est, et le suffixe `(terminal)` /
`(non-terminal)` (cpp#151) **suit** le `[rule-id]` en fin de `<detail>`.
Mesuré sur la session de grooming de ce ticket même :

```
[TS] [policy:deny] Bash: cd /var/log/claude-pilot && ls -t *.log … | while read
[TS]   sz=$(stat -c%s "$f"); mt=$(stat -c '%y' "$f" | cut -c1-16)
[TS]   ec=$(head -c 4000 "$f" | grep -oE '…')  [bash-grep] (non-terminal)
```

Le `grep -m1` rend la **première** ligne : ni `[bash-grep]`, ni
`(non-terminal)`. Population, sur les stderr postérieurs au déploiement de
cpp#151 :

```bash
find /var/log/claude-pilot -name '*.stderr' -newermt '2026-09-05' \
  | xargs grep -h -m1 '\[policy:deny\]' \
  | grep -c -E '\((non-)?terminal\)[[:space:]]*$'     # 206  — marqueur présent
# total de fichiers portant un refus sur la même fenêtre : 268
```

**62 des 268 (23 %) perdent leur marqueur à la capture** — et la preuve
`98b60020` du ticket est dans ces 62 (`grep -m1 … | grep -c '(terminal)'` → 0).
Deux conséquences : un prédicat de létalité qui lirait la ligne capturée serait
**faux sur l'une des deux preuves du ticket** ; et l'instruction que le message
donne déjà à l'opérateur — *« Read the halt event's bracketed [rule-id]
FIRST »* — est **inexécutable** sur cette même population. C'est le point 3 de
la doctrine mika#2312 (`docs/solutions/security-issues/le-classifier-ne-decide-jamais-sur-le-contenu-dun-fichier-2026-09-18.md`,
*« ne jamais tronquer la commande en la rapportant »*), violé structurellement.

### M3 — Taille et forme de la population

```bash
ls /var/log/claude-pilot/*.stderr | wc -l                              # 2441
grep -l '\[policy:deny\]' /var/log/claude-pilot/*.stderr | wc -l       # 1258  (52 %)
```

Le marqueur de létalité n'existe **pas avant le 2026-09-04** (date du plus
ancien stderr en portant un ; déploiement de cpp#151) — d'où une population
historique qui ne le déclare pas et pour laquelle aucune lecture n'est
possible. Sur la fenêtre postérieure :

```bash
find … -newermt '2026-09-05' | xargs grep -h -o -E '\((non-)?terminal\)' | sort | uniq -c
#    1093 (non-terminal)
#      92 (terminal)
find … -newermt '2026-09-05' | xargs grep -l -E '[^-]\(terminal\)' | wc -l   # 78 / 268
```

**92 % des refus sont non-terminaux.** 190 des 268 sessions portant un refus
n'en portent que des non-terminaux.

### M4 — Un refus terminal est bien le dernier événement de sa session

```bash
grep -n -o -E '\((non-)?terminal\)' /var/log/claude-pilot/da4aa7ae-*.stderr | tail -5
#   160:(terminal)
wc -l /var/log/claude-pilot/da4aa7ae-*.stderr      # 166
```

Unique refus, terminal, en ligne 160 sur 166. La sémantique tient : un refus
terminal finit la session. C'est ce qui autorise le prédicat de U2 à poser la
question sous la forme « **un** refus terminal existe-t-il dans ce fichier ? »
plutôt que « le premier / le dernier est-il terminal ? ».

---

## Ce que la mesure rectifie du ticket

Trois points, chacun changeant un choix d'implémentation.

**R1 — `_denial_is_terminal` n'est pas accessible depuis mika.** Le ticket
prescrit de « réserver "halted" aux denys terminaux (`_denial_is_terminal=true`) ».
Ce champ est un attribut interne de claude-pilot : il n'apparaît ni dans le
stdout JSON que `dispatch-lib` parse (`status`, `session_id`, `turns`,
`subtype`, `termination_reason`, `api_error_status`, `cost_usd`, `duration_ms`
— `dispatch-lib.sh:3071-3095`), ni dans le `.log`, ni dans le `.stderr`
autrement que rendu en texte :

```bash
grep -c 'denial_is_terminal' /var/log/claude-pilot/a0886164-*.log       # 0
```

**La seule source de vérité disponible côté mika est le marqueur textuel**
`(terminal)` / `(non-terminal)`. Le plan lit donc ce marqueur, et doit traiter
explicitement le cas où il est absent (M3) — cas que le ticket, supposant un
booléen toujours présent, n'a pas.

**R2 — Le correctif principal n'est pas un libellé, c'est une garde.** Le
ticket demande de changer le texte. Mais sur ses deux propres preuves, U1 (une
garde de deux lignes) suffit à faire disparaître le message : la session avait
livré son plan, il n'y avait aucun échec à expliquer. Le libellé (U2) reste
nécessaire pour la population résiduelle — un échec réel accompagné d'un refus
non-terminal — mais il vient en second.

**R3 — Deux tests existants épinglent l'ordre actuel comme une décision.**
`test-dispatch-lib.sh:3036` et `:3154` asserttent que la branche `POLICY_DENY`
**précède** les messages de dérive, la note de re-dispatch et le message
« zéro commit ». Un correctif qui déplacerait la branche les ferait rougir. Le
plan préserve ces deux assertions **sans les modifier** (voir U1) : leur
décision — *le refus l'emporte sur la dérive quand les deux s'appliquent* —
reste vraie ; ce que le fix change, c'est **quand les deux s'appliquent**.

---

## Requirements

- **REQ1** — Une session qui a produit son livrable ne peut plus être libellée
  par un refus, quelle que soit la létalité de celui-ci.
- **REQ2** — Le verbe « halted » est réservé aux refus dont le marqueur dit
  `(terminal)`.
- **REQ3** — Un refus non-terminal observé est rapporté comme **note
  factuelle** annexée, jamais comme cause et jamais en remplacement du
  diagnostic de la branche qui s'applique.
- **REQ4** — Un refus dont la létalité n'est pas déclarée (antérieur à
  cpp#151) n'est affirmé ni terminal ni non-terminal ; l'indétermination est
  **nommée**.
- **REQ5** — Le refus rapporté à l'opérateur porte son `[rule-id]` et son
  marqueur de létalité quand le pilote les a émis, afin que l'affirmation de
  REQ2/REQ3 soit vérifiable sur la preuve jointe.
- **REQ6** — Aucun texte ajouté n'introduit un jeton que des prédicats en aval
  lisent comme une classification terminale.
- **REQ7** — Les deux assertions d'ordre existantes passent **sans
  modification**.
- **REQ8** — Le comportement de `dev-pilot` est strictement inchangé au site A
  pour tout ce qui n'est pas le verbe.

---

## Décisions

### D1 — La garde est `[ -z "$VALID_PLAN" ]`, et l'ordre des conjoints est porteur

Les deux sites deviennent :

```bash
if [ -n "$POLICY_DENY" ] && [ -z "$VALID_PLAN" ]; then
```

`[ -n "$POLICY_DENY" ]` **reste le conjoint de tête**. Les deux assertions
existantes cherchent la sous-chaîne littérale `if [ -n "$POLICY_DENY" ]`
(`test-dispatch-lib.sh:3036`, `:3154`) : elles continuent de matcher et de
mesurer le même ordre relatif. Écrire la garde en tête aurait fait rougir deux
tests que rien ne demande de toucher, pour un résultat identique — et aurait
donné l'apparence d'un fix qui corrige ses propres tests.

`VALID_PLAN` est résolu en haut de `_post_flight_recovery`
(`dispatch-lib.sh:4104-4124`) et n'est peuplé que sous `[ "$SKILL" =
"dev-groom" ]`. Pour `dev-pilot` il est donc **structurellement vide**, la
garde est toujours vraie, et le site A se comporte exactement comme avant
(REQ8). La même forme de garde vaut aux deux sites : un seul concept, pas deux.

**Alternative écartée — garder sur `STATUS = success`.** Le ticket la propose
(« ne jamais préfixer … quand `status: success` »). Elle est plus large que le
défaut et fausse dans les deux sens : un `dev-pilot` peut rendre
`status: success` en n'ayant rien commité, auquel cas l'échec de pipeline est
réel et doit rester dit ; et un refus terminal produit un `status` qui n'est
pas `success`, si bien que la garde ne changerait rien là où elle serait utile.
Le discriminant juste est *« un échec a-t-il été établi par ailleurs ? »*, que
`VALID_PLAN` mesure directement au site B et que la garde HEAD-inchangé mesure
déjà au site A.

### D2 — Le prédicat de létalité porte sur le FICHIER, jamais sur la ligne capturée

`_policy_deny_lethality <stderr_path>` rend exactement un mot :
`terminal` | `non-terminal` | `undeclared`.

Trois raisons mesurées de ne pas lire la ligne du `grep -m1` :

1. **M2** — le marqueur est déporté hors de cette ligne dans 23 % des cas,
   dont la preuve `98b60020`. Un prédicat sur la ligne serait faux sur une des
   deux preuves du ticket.
2. **M0** — une session porte plusieurs refus (5 pour `a0886164`). La question
   utile est « un terminal existe-t-il ? », pas « le premier l'était-il ? ».
3. **M4** — un refus terminal termine la session, donc « au moins un terminal »
   et « le dernier est terminal » coïncident, et la première formulation est la
   seule robuste à la troncature.

La discrimination est un `grep` littéral : `(non-terminal)` **ne contient pas**
la sous-chaîne `(terminal)` — la parenthèse ouvrante exigée par cette dernière
est occupée par le `-`. Vérifié :

```bash
printf '%s\n' 'x (non-terminal)' 'y (terminal)' | grep -c -F '(terminal)'    # 1
```

Le contrôle négatif correspondant est un test livré (T6) : c'est l'erreur qui
transformerait d'un coup les 1093 non-terminaux mesurés en terminaux.

### D3 — `undeclared` n'affirme rien, et c'est la seule lecture sûre

Le défaut réparé est une affirmation fausse dans un sens. Replier l'absence de
marqueur sur `non-terminal` produirait l'affirmation fausse dans l'autre sens ;
la replier sur `terminal` reconduirait le défaut. Un signal illisible est donc
**nommé** plutôt que replié sur une valeur lisible — même geste que
`unknown_provider` (mika#2328) et `pilot_stall_signal_unavailable` (mika#2277).

Le coût est nommé : sur la population antérieure au 2026-09-04, le message ne
tranche pas. Il dit pourquoi, et il dit que c'est le build du pilote qui ne
déclarait pas la létalité — pas le dispatch qui n'a pas su lire.

### D4 — `terminal` remplace, `non-terminal` annexe

C'est le cœur conceptuel, et l'asymétrie est raisonnée.

Un refus **terminal** *est* la cause : il remplace le diagnostic de la branche,
comme aujourd'hui, avec le libellé actuel augmenté de la mention `(terminal)`.

Un refus **non-terminal** n'est pas la cause : la session a continué après lui.
Dans une branche d'échec, remplacer « pas de plan trouvé, causes probables
(a) dérive (b) bug de découverte » par « halted by policy deny » **déplace le
mensonge du ticket au lieu de le fermer** — on substitue au vrai diagnostic un
faux, sous couvert de précision. Le refus reste donc rapporté, en **annexe**,
parce qu'il peut avoir gêné le pilote sans le tuer ; mais la branche qui
s'applique garde la parole.

Corollaire assumé : l'annexe s'écrit aussi sur une session **réussie**, ce que
le ticket demande explicitement (*« un deny non-terminal survécu est une
note … , pas un "Halt" »*). Elle est courte, factuelle, et non alarmante.

### D5 — L'annexe ne peut porter aucun jeton de reclassement

`dispatch-lib.sh:3427` et `:4438` lisent le contenu de `RESULT` :

```bash
grep -qE '(PIPELINE FAILURE:|STRUCTURAL VIOLATION:|HANDLER CRASH|^STATUS=CANCELLED|^Outcome: PIPELINE_INCOMPLETE)'
```

Une note d'information qui introduirait l'un de ces jetons reclasserait la
session en échec — exactement le défaut réparé, reconstruit par le correctif.
La contrainte est donc **dure** et tenue par un test (T10), pas par la
relecture.

**Effet de bord, nommé parce qu'il est réel et voulu :** en retirant
`PIPELINE FAILURE:` d'un `result` qui n'aurait pas dû le porter, U1 rend la
main aux gates en aval. Pour une session qui a livré (les deux preuves), rien
ne tire. Pour une session qui n'a rien produit, le gate `empty_completion`
(mika#1996) reprend la main et pose son propre diagnostic — lequel est le
**bon** : « ce cycle n'a rien produit », plutôt que « halted par un refus qui ne
l'a pas arrêtée ».

### D6 — La capture est dé-tronquée, bornée dans les deux régimes

`grep -m1` devient une extraction qui part de la première ligne portant le
marqueur et s'arrête à la **première** de ces conditions :

- une ligne portant `(terminal)` ou `(non-terminal)` (incluse) ;
- une ligne appartenant visiblement à un autre événement de journal ;
- un plafond de lignes.

Les trois bornes sont nécessaires ensemble. Sans le plafond, un stderr
antérieur à cpp#151 — aucun marqueur nulle part — capturerait jusqu'à la fin du
fichier. Sans l'arrêt sur autre événement, un refus mono-ligne suivi de lignes
`[debug]` en emporterait le bruit jusqu'au plafond. Sans l'arrêt sur marqueur,
on ne gagnerait rien sur le cas visé.

Le comportement fail-open est préservé à l'identique : fichier absent ou
illisible → capture vide → la chaîne existante s'applique comme aujourd'hui.

---

## Scope Boundaries

**Dans le périmètre** — `skills/bundled/_shared/dispatch-lib.sh` (les deux
sites `POLICY_DENY` et une fonction nouvelle) et
`skills/bundled/_shared/test-dispatch-lib.sh`.

**Hors périmètre, délibérément :**

- **La policy elle-même.** Aucune règle n'est élargie, aucun refus n'est évité.
  Ce travail répare ce que le dispatch **dit** d'un refus, pas ce que la policy
  **décide**. Les 1258 stderr portant un refus restent ce qu'ils sont.
- **Le format émis par claude-pilot.** Le marqueur, sa position après le
  `[rule-id]`, et son absence avant cpp#151 sont des faits amont. Ce plan les
  lit ; il ne demande rien à `claude-pilot`.
- **Les autres classifications de `RESULT`** — `_classify_terminated_session`,
  le gate `empty_completion`, la rescue de worktree sale, le gate structurel de
  complétion. Aucune n'est touchée ; D5 nomme la seule interaction.
- **Le rattrapage rétroactif.** Aucun `result` déjà délivré n'est réécrit. Les
  deux callbacks de M0 gardent leur texte : la sonde est la prochaine
  occurrence.

---

## Implementation Units

### U1 — Le refus ne préempte plus une session qui a livré

Deux lignes, symétriques, dans `_post_flight_recovery` :

| Site | Ligne actuelle | Ligne cible |
|---|---|---|
| A (~4149) | `if [ -n "$POLICY_DENY" ]; then` | `if [ -n "$POLICY_DENY" ] && [ -z "$VALID_PLAN" ]; then` |
| B (~4345) | `if [ -n "$POLICY_DENY" ]; then` | `if [ -n "$POLICY_DENY" ] && [ -z "$VALID_PLAN" ]; then` |

Un commentaire à chaque site nomme mika#2493, la mesure M0, et **la raison de
l'ordre des conjoints** (D1) — sans quoi un futur éditeur « normalisera » en
mettant la garde en tête et fera rougir deux tests sans comprendre pourquoi.

Ferme REQ1. Ferme à lui seul les deux preuves du ticket.

### U2 — Le verbe suit la létalité

1. Nouvelle fonction `_policy_deny_lethality <stderr_path>` (D2), placée près
   des autres accesseurs de journal pilote, rendant un mot sur stdout. Lecture
   ANSI-strippée comme les sites existants. Fichier absent/illisible →
   `undeclared`.
2. Aux deux sites, résolution de la létalité juste après la capture.
3. La branche C n'est prise que pour `terminal`, et son libellé gagne la
   mention `(terminal)`. Les chaînes `halted by claude-pilot policy deny` et
   `halted by policy deny — not generic exit` sont **conservées** : les
   assertions `test-dispatch-lib.sh:3012` et `:3122` cherchent une sous-chaîne
   dans un extrait de source large, elles continuent de passer (REQ7).
4. Pour `non-terminal` et `undeclared`, une note est **annexée** à `RESULT`
   après la chaîne (D4), sous contrainte D5.

Ferme REQ2, REQ3, REQ4, REQ6.

### U3 — Le refus rapporté porte son rule-id et son marqueur

Remplacement de `grep -m1 '\[policy:deny\]'` par l'extraction bornée de D6, aux
deux sites. Ferme REQ5.

U3 est la condition de **vérifiabilité** de U2 : sans lui, le message affirme
« non-terminal » en joignant une preuve où le marqueur n'apparaît pas (23 % des
cas), c'est-à-dire un troisième « croire sur parole » dans un ticket dont le
sujet est un libellé qu'on a cru sur parole.

---

## Verification Contract

Tous les tests vivent dans `skills/bundled/_shared/test-dispatch-lib.sh`,
exécutés par `make test-dispatch-lib`, déjà en CI. Les fixtures sont des
fichiers stderr synthétiques écrits dans un répertoire temporaire, sur le
modèle des fixtures policy-deny existantes (`:3065-3077`), avec codes ANSI
pour exercer le strip.

**Prédicat de létalité (U2)**

- **T1** — refus non-terminal mono-ligne → `non-terminal`.
- **T2** — refus terminal → `terminal`.
- **T3** — refus sans marqueur (forme pré-cpp#151) → `undeclared`.
- **T4** — refus dont le `<detail>` est multi-ligne, marqueur en 3ᵉ ligne
  (**la forme exacte de la preuve `98b60020` et de M2**) → `non-terminal`.
  C'est le test qui sépare « lit le fichier » de « lit la ligne » : il rougit
  sur toute implémentation qui lirait la ligne capturée.
- **T5** — quatre non-terminaux suivis d'un terminal → `terminal`.
- **T6** — **contrôle négatif** : un fichier ne portant que `(non-terminal)`
  ne rend jamais `terminal`. C'est l'erreur de discrimination qui
  reclasserait 1093 refus mesurés.
- **T7** — fichier absent, puis fichier illisible → `undeclared` dans les deux
  cas (fail-open préservé).

**Garde de préemption (U1, structurel)**

- **T8** — tout site dont la condition de branche lit `POLICY_DENY` porte la
  garde `[ -z "$VALID_PLAN" ]`. Formulé sur la **population** des sites, pas
  sur deux lignes nommées, pour qu'un troisième site futur soit vu.

**Comportement bout-en-bout — les deux sens du DoD**

- **T9** — dev-groom, `VALID_PLAN` non vide, refus non-terminal : le `result`
  ne contient ni `PIPELINE FAILURE:` ni `halted`. *(DoD point 1.)*
- **T10** — refus terminal : le `result` contient `halted` **et**
  `(terminal)`. *(DoD point 2, sens inverse.)*
- **T11** — la note annexée ne contient aucun de
  `PIPELINE FAILURE:`, `STRUCTURAL VIOLATION:`, `HANDLER CRASH`,
  `STATUS=CANCELLED`, `Outcome: PIPELINE_INCOMPLETE` (D5). Assertion portée sur
  le **texte produit**, pas sur la constante.

**Non-régression (REQ7)**

- **T12** — les deux assertions d'ordre existantes (`:3036`, `:3154`) passent
  **sans avoir été modifiées**. Vérifié en exécutant la suite complète : leur
  présence inchangée dans le diff est elle-même la preuve.

**Dé-troncature (U3)**

- **T13** — sur la fixture de T4, la capture rapportée contient le `[rule-id]`
  **et** le marqueur.
- **T14** — sur une fixture sans marqueur suivie de lignes `[debug]`, la
  capture s'arrête et n'emporte pas le bruit (bornes de D6).

**Commande de vérification :** `make test-dispatch-lib` — zéro échec.
**Non-régression large :** `make verify-bundled-skills`.

---

## Definition of Done

- U1, U2, U3 implémentés dans `dispatch-lib.sh`.
- T1–T14 écrits et verts ; `make test-dispatch-lib` sans échec.
- Les deux assertions d'ordre existantes intactes dans le diff.
- Aucune règle de policy modifiée, aucun autre classificateur de `RESULT`
  touché.
- Les commentaires des deux sites nomment mika#2493 et la raison de l'ordre
  des conjoints.

---

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de son DoD et des Requirements.

- **AC1** — Une session `status: success` portant un refus non-terminal ne
  contient plus `PIPELINE FAILURE` ni `halted by policy deny` dans son
  `result`. Couvert par T9, sur la forme mesurée en M0.
- **AC2** — Un refus terminal produit toujours un libellé portant `halted` et
  la mention `(terminal)`. Couvert par T10.
- **AC3** — Les deux sens sont testés — AC1 et AC2 sont deux tests distincts,
  et T5/T6 tiennent la frontière entre eux.
- **AC4** — Un refus non-terminal reste **visible** dans le `result` : il est
  requalifié, jamais supprimé. Couvert par T9 (absence de `halted`) conjuguée à
  la présence de la note.
- **AC5** — Un refus dont la létalité n'est pas déclarée n'est affirmé ni
  terminal ni non-terminal. Couvert par T3.
- **AC6** — `make test-dispatch-lib` et `make verify-bundled-skills` passent.

---

## Fire-Disposition

Ce plan livre des détecteurs : T1–T14, dont T6 (contrôle négatif), T8 (scan
structurel sur la population des sites lisant `POLICY_DENY`) et T11 (contrainte
de jetons).

**Option retenue : (a) — exception nommée en allowlist, allowlist livrée
VIDE.**

Aucune violation préexistante ne subsiste au moment du merge, et ce n'est pas
une chance : les trois unités de correctif précèdent les détecteurs **dans le
même commit**, donc la population que T8 mesure est conforme (2 sites sur 2)
avant que T8 n'existe. T6 et T11 portent sur du code neuf. Il n'y a donc rien à
excepter.

**Détail d'implémentation de l'allowlist.** T8 porte une constante
d'exception explicite, déclarée vide :

```sh
# mika#2493 — allowlist des sites lisant POLICY_DENY sans la garde VALID_PLAN.
# LIVRÉE VIDE. Quand T8 tire, la résolution est d'ajouter la garde au site
# fautif, jamais d'ajouter une entrée ici.
POLICY_DENY_UNGUARDED_ALLOWED=""
```

**Assertion auto-nettoyante.** Un test compagnon assertte que
`POLICY_DENY_UNGUARDED_ALLOWED` est vide. Il rougit donc le jour où quelqu'un y
inscrit une exception plutôt que de réparer le site — ce qui rend le
contournement visible à l'instant où il est tenté, et non des mois plus tard.
C'est la forme employée par les allowlists vides de mika#2201 et mika#2323
(*« on déclare, on n'allowliste pas »*).

**Pourquoi ni (b) ni (c).** (b) livrer désarmé n'a pas d'objet : un détecteur
désarmé se lit exactement comme un détecteur vert (classe mika#2205), et il n'y
a ici aucune population à mesurer avant d'armer. (c) halte-et-remontée n'a pas
d'objet non plus : aucune décision d'opérateur n'est requise, la population est
conforme.

---

## Sondes post-déploiement, et leurs haltes

Ce travail ne crée **aucun événement de journal ni compteur** : il corrige un
texte destiné à un lecteur humain et au parseur de `self-dev-callback`. Les
sondes sont donc des lectures, et leur silence ne prouve rien tant qu'aucun
dispatch n'a rencontré de refus (mika#2205).

**S1 — Le libellé ne ment plus (7 jours).** Sur les dispatches ayant abouti,
aucun `result` ne porte `halted` alors que la session a livré.

```bash
find /var/log/claude-pilot -name '*.stderr' -newermt '<date-de-déploiement>' \
  | xargs grep -l '\[policy:deny\]' | wc -l
```

donne la population rencontrée. **Halte 1 — cette population est nulle** :
aucune conclusion ne peut être tirée, la sonde n'a rien observé. **Établir le
déploiement avant de conclure quoi que ce soit** (classe mika#2340) — le
substrat `dispatch-lib.sh` n'atteint un agent que par `make deploy`, et une
correction présente dans l'arbre est invisible tant qu'elle n'est pas seedée.

**S2 — Le verbe reste disponible pour les vrais halts.** Un refus terminal doit
continuer de produire `halted (terminal)`. **Halte 2 — plus aucun `halted` nulle
part sur une fenêtre portant des refus terminaux** (`grep -l -E '[^-]\(terminal\)'`
non vide) : le prédicat rend `non-terminal` ou `undeclared` là où il devrait
rendre `terminal`. **Ne pas élargir la garde U1 par réflexe** — c'est D2 qu'il
faut relire, et T6 qui aurait dû rougir.

**S3 — La note est vérifiable.** Sur un `result` portant la note non-terminale,
le `Halt event:` joint doit porter le marqueur. **Halte 3 — il ne le porte pas
alors que le stderr l'a émis** : U3 n'a pas pris, et l'affirmation de U2 est
redevenue invérifiable. Réparer l'extraction, **ne pas retirer l'affirmation**.

**S4 — Contrôle négatif d'aval.** Aucune session ayant réellement échoué ne
devient silencieuse. **Halte 4 — un échec réel arrive sans aucun diagnostic** :
D5 a été mal appliqué et la note a évincé un jeton que le gate
`empty_completion` attendait. Lire `cycle_output.empty.banner_skipped` en
premier.

---

## Suivi (hors périmètre, nommé)

- **Le champ structuré de létalité.** R1 établit que mika ne peut lire la
  létalité que dans du texte rendu. Demander à `claude-pilot` d'exposer
  `denial_is_terminal` — et le compte des refus — dans son stdout JSON
  supprimerait `undeclared` et rendrait U3 inutile pour le prédicat. **Ticket
  amont (cpp)**, précondition : que S1 montre une population `undeclared`
  résiduelle non négligeable après extinction des logs antérieurs au
  2026-09-04.
- **La troncature du `<detail>` elle-même.** U3 dé-tronque jusqu'au marqueur ;
  il ne restitue pas une commande dont le corps dépasserait le plafond de D6.
  Le point 3 de la doctrine mika#2312 reste donc partiellement ouvert pour les
  commandes très longues. **Suivi**, précondition : une occurrence mesurée où
  le plafond ampute l'information décisive.
- **Le frottement de policy lui-même.** 52 % des sessions portent un refus et
  92 % de ces refus sont non-terminaux (M3). Ce plan les rend **lisibles** ; il
  ne les réduit pas. Savoir si ce taux est le régime nominal du bac à sable ou
  le signe d'une allow-list trop étroite est une question distincte, dont la
  mesure existe désormais. **Suivi.**

---

## Ce que ce travail n'achète PAS

- **Il ne supprime aucun refus** et n'élargit aucune règle.
- **Il ne réécrit aucun `result` déjà délivré** : les deux callbacks de M0
  gardent leur texte. Fabriquer rétroactivement un diagnostic qu'on n'a pas
  émis serait l'inverse de ce que ce ticket défend.
- **Il ne tranche pas la létalité sur la population antérieure au
  2026-09-04** : il dit qu'il ne peut pas, ce qui est la seule chose vraie
  disponible (D3).
- **Il n'ajoute aucun compteur, aucun événement.** Le défaut est un texte lu
  par un humain ; le seul instrument est la lecture de ce texte, et une lecture
  que personne ne fait reste un silence.

---

## Références

- **Ticket :** mika issue#2493 ; parent mika issue#2491 (umbrella
  loop-substrate, Défaut 2).
- **Amont :** cpp#151 (marqueur de létalité, déployé le 2026-09-04 d'après
  M3), cpp#128 (refus survivable).
- **Code :** `skills/bundled/_shared/dispatch-lib.sh` — sites `POLICY_DENY`
  (~4138-4164, ~4331-4361), lecteurs de `RESULT` (~3427, ~4438), résolution de
  `VALID_PLAN` (~4104-4124), champs du stdout pilote (~3071-3095).
- **Tests :** `skills/bundled/_shared/test-dispatch-lib.sh` — Test 13
  (~2982-3045), son pendant dev-pilot (~3110-3165), fixtures policy-deny
  (~3065-3077).
- **Doctrine :**
  `docs/solutions/security-issues/le-classifier-ne-decide-jamais-sur-le-contenu-dun-fichier-2026-09-18.md`
  (mika#2312 — forme de la ligne, lecture du `[rule-id]`, non-troncature) ;
  `docs/solutions/workflow-issues/2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md`
  (origine de la désambiguïsation Class C).
- **Motifs maison :** mika#2328 et mika#2277 (un signal illisible est nommé,
  jamais replié) ; mika#2205 (un détecteur silencieusement inerte se lit comme
  un détecteur sain) ; mika#2201 et mika#2323 (allowlist livrée vide — *on
  déclare, on n'allowliste pas*) ; mika#2340 (établir le déploiement avant de
  conclure sur un texte) ; mika#1996 (gate `empty_completion`).

---

## Revision history

- **2026-09-23** — rédaction initiale. Mesures M0–M4 prises sur
  `/var/log/claude-pilot/` (2441 stderr, fenêtre 2026-05-15 → 2026-09-23).
  Trois rectifications du ticket posées en R1–R3.
