# mika#2539 — Le commit de sauvetage nomme sa cause, et il la lit là où elle est déjà classée

> **Ticket :** `senara-solutions/mika#2539` — « Le post-flight recovery de
> `dispatch-lib` (mika#1282) committe le travail sauvé avec un message
> générique : `wip(mika#<N>): impl staged by post-flight recovery (mika#1282)`.
> Ce message ne dit **pas pourquoi** le pilote a rendu un worktree sale sans
> committer. »
>
> **Mesure (n=2, 2026-09-25/26) :** #2532 cœur (wip `b83d9214`) — cause réelle
> deny classifieur (tour 62, terminal bash-grep) + stall SDK `idle_timeout` ;
> #2536 (wip `10366892`) — cause réelle plafond 150 tours
> (`error_max_turns after 151 turns`). **Même message générique.**
>
> **Branche :** `fix/2539/rescue-commit-message-cause-token`
> **Classe :** dette p2, loop-substrate. Ratifié Prime + Vincent 2026-09-26.

---

## 0. Ce que la lecture du code déplace dans le ticket

Trois rectifications, et chacune change le remède. Elles sont le premier
livrable de ce grooming.

### R1 — La cause n'est pas « lisible dans le log pilote » : elle est DÉJÀ PARSÉE

Le ticket écrit : *« La cause est déjà lisible dans le log pilote au moment où
`dispatch-lib` compose le message de sauvetage : `error_max_turns`,
`idle_timeout`, `error_during_execution:after_deny`, `[done] Success` »*, ce qui
invite à écrire un scraper de log dans le compositeur.

C'est vrai, et c'est en dessous de la réalité. Au moment exact où
`_rescue_dirty_worktree` tourne, les deux signaux sont **déjà extraits, déjà
classés, en portée** :

| signal | où il est posé | ce qu'il porte |
|---|---|---|
| `SUBTYPE` | `dispatch-lib.sh:3285`, `jq '.subtype'` sur `PILOT_OUTPUT` | `error_max_turns`, `idle_timeout`, `stall_detected`, … |
| `TERMINATION_REASON` | `:3286` | le détail |
| `POLICY_DENY` | `:4575`, extrait par `_policy_deny_excerpt` | l'événement de deny en entier |
| `POLICY_DENY_LETHALITY` | `:4577`, par `_policy_deny_lethality` | `terminal` \| `non-terminal` \| `undeclared` |

**L'ordre le garantit.** `_rescue_dirty_worktree` est appelé à `:4664`, donc
**après** le bloc policy-deny (`:4566`–`:4657`), à l'intérieur du même
`if [ -n "$PRE_RUN_HEAD" ] && [ -n "$REPO" ]`. Et le sauvetage lui-même ne
procède que si `PRE_RUN_HEAD = POST_RUN_HEAD` (`:4081`), condition strictement
plus étroite que celle du bloc deny (`:4556`) — donc **chaque fois que le
sauvetage tire, le bloc deny a tourné**.

Écrire un scraper serait un **troisième lecteur des mêmes octets**. Ce plan n'en
écrit aucun.

### R2 — Le vocabulaire de cause EXISTE, et il a son garde de dérive

`_halt_family` (`dispatch-lib.sh:3699`) est la table aval du vocabulaire de
halte de claude-pilot, dont l'amont est `GuardrailAbortReason.guardrail`
(`types.py`) plus `SDK_TERMINATION_SUBTYPES` (`agent.py`). Elle rend
`<famille>|<indice>|<sens>` sur neuf familles plus un bras `*)`.

Elle est **gardée** : le drift guard T6 de `test-dispatch-lib.sh:4336` lit le
`Literal` amont et refuse toute valeur que la table classerait `unknown`.

Les quatre jetons que le ticket propose (`pre-turn-cap`, `post-deny`,
`post-stall`, `dirty-no-commit`) sont une **seconde classification du même
signal**. Livrée comme une table à part, elle :

1. serait **hors du garde T6** — un sous-type ajouté en amont ferait rougir
   `_halt_family` et **passerait en silence** dans le message de sauvetage ;
2. reproduirait mot pour mot la panne mesurée de mika#2158 — `auto_pull.rs`
   portait une regex commentée *« Mirrors GROOMED_VERDICT_RE »* qui **n'a suivi
   aucun des deux élargissements suivants**, et promotion et routage ont répondu
   différemment à la même question pendant des mois.

**Le plan dérive donc le jeton de `_halt_family`, sans nouvelle table.**

### R3 — Quatre jetons ne couvrent pas la population, et le trou MENT

`_halt_family` distingue neuf familles. Le jeu de quatre jetons n'a de place que
pour trois d'entre elles ; les six autres — `quota_throttled`,
`model_never_resumed`, `tool_never_returned`, `pilot_bug`, `substrate`,
`transport` — tomberaient dans `dirty-no-commit`, glosé par le ticket
« cause de coupure **non identifiée** ».

Or leur cause **est** identifiée et nommée. Écrire « non identifiée » d'une
session dont le halt est classé serait le défaut que ce ticket ferme, déplacé
d'un cran — la forme exacte que mika#2304 nomme : *un champ qui affirme, avec
autorité, ce qui n'a pas eu lieu.*

Le ticket écrit « distinguant **au moins** » : la licence d'aller plus large est
dans sa lettre. Ce plan livre la partition complète.

### R4 — Le cas mesuré #2532 porte DEUX causes, et la précédence est déjà tranchée dans ce fichier

#2532 = deny terminal **et** `idle_timeout`. Le deny est la **cause** (le pilote
a été empêché), le stall la **conséquence** (il est ensuite resté muet).
Rapporter `session_silent` enverrait l'opérateur sur le mauvais organe.

`_post_flight_recovery` a déjà tranché cette précédence exacte, et par écrit :
le bloc Class C (`:4599`) place le deny **avant** le message de drift, avec le
commentaire *« THE ORDER OF THE CONJUNCTS IS LOAD-BEARING »* et deux tests qui
mesurent sa position (Test 13, Test 14 de `test-dispatch-lib.sh`).

**Le deny prime sur la famille de halte.** Ce n'est pas une précédence inventée
ici : c'est celle que le fichier applique déjà, reprise telle quelle.

---

## 1. Requirements

- **R-1** Le sujet du commit de sauvetage porte un jeton de cause.
- **R-2** Le jeton distingue au minimum les quatre cas du ticket, et ne ment sur
  aucun autre.
- **R-3** Le vocabulaire a **un seul site de définition**, et la partie
  sous-type dérive de `_halt_family` — aucune seconde table.
- **R-4** Aucun nouveau lecteur du log ou de stderr.
- **R-5** Le comportement de recovery est **inchangé** : mêmes conditions de
  déclenchement, mêmes exclusions de pathspec, même `--no-verify`, même
  `POST_RUN_HEAD`, même `RESCUED_DIRTY_WORKTREE`, même `RESULT`.
- **R-6** Le préfixe `wip(` et la queue existante du sujet sont préservés
  **mot pour mot** (contrats aval, § 4.2).
- **R-7** Le jeton et la ligne `Halt class:` du callback disent **le même mot**
  pour la même session.

---

## 2. Conception — le vocabulaire, dérivé et non redéclaré

### 2.1 `_rescue_cause_token` — seul site de définition

Une fonction, quatre branches, dans cet ordre. Elle n'énumère **que** les deux
cas qui ne sont pas des familles de halte ; tout le reste passe par
`_halt_family`.

| # | condition | jeton | pourquoi ce rang |
|---|---|---|---|
| 1 | `POLICY_DENY` non vide **et** `POLICY_DENY_LETHALITY = terminal` | `policy_deny` | R4 — le deny est la cause, le halt qui suit est sa conséquence |
| 2 | sous-type résolu non vide, famille ≠ `unknown` | `<famille>` | dérivé de `_halt_family`, donc couvert par T6 |
| 3 | sous-type résolu non vide, famille `unknown` | `halt_unmapped` | *« la session a halté pour une raison hors de notre table »* |
| 4 | aucun sous-type résolu | `no_halt_signal` | *« la session n'a pas halté du tout »* |

**La lethality est requise au rang 1, et c'est la sémantique déjà établie.** Le
bloc Class C exige `terminal` (`:4599`) ; un deny non-terminal est *annexé en
note* (`_annex_policy_deny_note`, `:4656`), jamais traité comme la cause. Sans
cette condition, une session ayant **survécu** à un deny puis heurté le plafond
serait étiquetée `policy_deny` — faux, et faux sur le cas le plus fréquent
(1093 denies non-terminaux mesurés par mika#2493 contre 62 terminaux).

**Les rangs 3 et 4 sont distincts, et c'est R3 rendu exécutable.** Le
discriminant est celui que `_classify_terminated_session:3795` emploie déjà :
`famille = unknown` **et** sous-type non vide. Les fusionner rendrait
indistinguables « halte inconnue » et « pas de halte » — la confusion même que
le ticket ferme.

### 2.2 Couverture des jetons du ticket

| jeton du ticket | jeton livré | rapport |
|---|---|---|
| `rescue pre-turn-cap` | `rescue budget_exhausted` | **renommé** — voir ci-dessous |
| `rescue post-deny` | `rescue policy_deny` | renommé pour la cohérence de casse |
| `rescue post-stall` | `rescue session_silent` \| `rescue model_unproductive` | **affiné en deux** — `idle_timeout` (silence réel) vs `stall_detected`/`empty_response` (modèle improductif) sont deux diagnostics, pas un |
| `rescue dirty-no-commit` | `rescue no_halt_signal` | **resserré** — ne couvre plus que « pas de halte », les haltes non mappées partant sur `halt_unmapped` |
| — | six familles restantes + `halt_unmapped` | R3 |

**Pourquoi `budget_exhausted` et non `pre-turn-cap`.** Trois raisons, par poids
croissant. (i) La famille couvre `error_max_turns` **et**
`error_max_budget_usd` : un jeton qui dit « turn cap » deviendrait faux le jour
où le plafond dollar devient émissible — et le CLAUDE.md racine documente
précisément ce jour comme non arrivé (`_sdk_guardrail_kwargs` finit sur `pass`,
*« un vocabulaire de terminaison que rien ne peut émettre »*). (ii) C'est le mot
que le callback écrit déjà sur sa ligne `Halt class:` : commit et callback
disent alors le même mot pour la même session (R-7). (iii) Un jeton à soi est
une seconde table à tenir, ce que R-3 refuse.

Casse **snake_case partout**, alignée sur `_halt_family`. Mélanger
`policy_deny` et `budget_exhausted` avec des kebab-case couperait une population
en deux à la lecture — ce que `scripts/canonical-tokens.tsv` (mika#2201) existe
pour empêcher.

---

## 3. Conception — un seul résolveur de sous-type

### 3.1 La divergence que la version naïve créerait

`_rescue_cause_token` pourrait lire `SUBTYPE` directement. Ce serait faux dans
un cas précis, et le cas est prévu par le code : `_classify_terminated_session`
a un **repli** quand `SUBTYPE` est vide — il scrape la ligne `[guardrail]` de
stderr (`:3753`–`:3777`) et en dérive le sous-type par `sed` (`:3774`).

Donc sur une session `terminated`, sans `subtype` JSON, avec une ligne
`[guardrail]` en stderr, et un arbre sale :

- le bandeau du callback dirait `Halt class: session_silent` ;
- le commit dirait `rescue no_halt_signal`.

**Deux affirmations sur la même session, contradictoires, dont une gravée dans
l'historique git pour toujours.** C'est le défaut du ticket, reproduit par son
correctif.

### 3.2 `_resolve_halt_subtype` — l'extraction

```
_resolve_halt_subtype() {
    # Mémoïsé : le sauvetage et le bandeau posent la même question sur la même
    # session, et la seconde réponse doit être la première.
    [ -n "${_HALT_SUBTYPE_RESOLVED+x}" ] && { printf '%s' "$_HALT_SUBTYPE_RESOLVED"; return 0; }
    ...  # 1) $SUBTYPE  2) sinon, scrape [guardrail] — la précédence existante
    printf '%s' "$_HALT_SUBTYPE_RESOLVED"
}
```

`_classify_terminated_session` est réécrit pour en tirer son `halt_subtype`,
dans ses **deux** branches. Il **garde** sa variable `guardrail` (elle porte la
ligne entière, dont il a besoin pour construire `cause`) : ce qui est extrait
est la résolution du *sous-type*, pas la présentation.

**Coût nommé.** La mémoïsation est une variable de process ; `dispatch-lib` est
sourcé une fois par dispatch et traite une session, donc la portée est juste.
Un futur appelant qui traiterait deux sessions dans un même process lirait la
première — d'où le nom explicite et le commentaire au site. La variable est
réinitialisée au même endroit que `SUBTYPE` (`:3285`), pour que la propriété
soit tenue par la construction et non par la mémoire de quelqu'un.

**Pourquoi la mémoïsation plutôt que deux appels.** Sans elle le scrape tourne
deux fois par session terminée-avec-travail (un `sed | grep` sur un fichier
stderr) — coût négligeable, mais surtout : `_rescue_dirty_worktree` tourne
**avant** le bandeau, donc c'est le sauvetage qui remplirait le cache et le
bandeau qui le lirait. L'ordre rend l'accord structurel plutôt que
coïncident.

---

## 4. Conception — la forme du message

### 4.1 Le sujet

```
wip(mika#2539): rescue budget_exhausted — impl staged by post-flight recovery (mika#1282)
wip(mika#2539): rescue policy_deny — plan staged by post-flight recovery (mika#2031)
```

Le jeton est **inséré entre le préfixe et la queue existante**. Trois propriétés
tombent de ce placement :

1. **Visible dans `git log --oneline`** avant toute troncature — c'est le point
   du ticket.
2. **La queue est intacte**, donc les contrats aval du § 4.2 ne bougent pas.
3. **Les deux assertions existantes de `test_dev_groom_dirty_rescue.sh`
   (lignes 145 et 197) sont des `assert_contains` sur cette queue : elles
   restent VERTES sans modification et deviennent le contrôle de non-régression
   du placement.** Une queue déplacée ou reformulée les ferait rougir.

Le corps du message (`Content written by pilot session … / Auto-rescued … /
Scaffold paths excluded …`) est inchangé.

### 4.2 Ce que le sujet doit préserver, mesuré

| consommateur | ce qu'il lit | impact |
|---|---|---|
| `test_rescue_commit_no_verify.sh:140` | `grep -c 'git -C "$WORKTREE_DIR" commit -m "wip('`, **exactement 3** | aucun — le jeton vit dans `_rescue_what`, interpolé **après** le littéral |
| `test_rescue_commit_no_verify.sh` awk (`:129`) | bloc `commit -m "wip(` → `2>&`, doit contenir `--no-verify` | aucun |
| `self-dev-webhook-qa/system_prompt.md:252` | regex ancrée `^wip\(` | aucun — préfixe intact ; le reste y est un « e.g. » |
| `test_dev_groom_dirty_rescue.sh:145,197` | `assert_contains` sur la queue | aucun — § 4.1 point 3 |
| `test_rescue_signal_open_pr.sh:143,337` | fabriquent leur propre sujet | aucun |
| `_compose_rescue_pr_body` (`:8617`) | ne lit pas le sujet | aucun |

Le commentaire de `:4086`–`4088` — *« The `commit -m "wip(` literal on both
sites below is load-bearing … keep the interpolation after it, not around it »* —
est respecté à la lettre : **aucun des deux sites `git commit` n'est touché.**
Seule la valeur de `_rescue_what` change, calculée une fois à `:4089`–`4094` et
consommée par les deux sites (chemin direct `:4181`, retry cargo-fmt `:4225`),
qui portent donc automatiquement le même jeton.

---

## 5. Contrat de vérification

### 5.1 Comportemental — `skills/bundled/_shared/tests/test_rescue_cause_token.sh`

Le harnais de `test_dev_groom_dirty_rescue.sh` (source `dispatch-lib`, dépôt git
jetable, appel direct de `_rescue_dirty_worktree`) est réemployé, avec
`SUBTYPE` / `POLICY_DENY` / `POLICY_DENY_LETHALITY` posés en amont.

| # | entrée | sujet attendu |
|---|---|---|
| V1 | `SUBTYPE=error_max_turns` | `rescue budget_exhausted` — **rejoue #2536** |
| V2 | deny `terminal` + `SUBTYPE=idle_timeout` | `rescue policy_deny` — **rejoue #2532**, et atteste la précédence R4 |
| V3 | `SUBTYPE=idle_timeout`, pas de deny | `rescue session_silent` |
| V4 | `SUBTYPE=stall_detected` | `rescue model_unproductive` |
| V5 | tout vide (`status: success`, arbre sale) | `rescue no_halt_signal` |
| V6 | `SUBTYPE=une_valeur_inconnue` | `rescue halt_unmapped` |
| V7 | deny `non-terminal` + `SUBTYPE=error_max_turns` | `rescue budget_exhausted` — **contrôle négatif de la lethality** |
| V8 | deny `undeclared` + `SUBTYPE=error_max_turns` | `rescue budget_exhausted` — idem |
| V9 | `SUBTYPE=""`, ligne `[guardrail] idle_timeout:` en stderr | `rescue session_silent` — **le repli du § 3.1** |
| V10 | V1 en `dev-groom` | jeton présent **et** queue `plan staged … (mika#2031)` |
| V11 | chemin retry cargo-fmt | **même** jeton qu'au chemin direct |
| V12 | V1 | `_rescue_cause_token` et `Halt class:` du bandeau disent le même mot (R-7) |

**Contrôles de non-régression (R-5), sur chaque vecteur :** `POST_RUN_HEAD`
avancé, `RESCUED_DIRTY_WORKTREE` posé pour `dev-pilot` et absent pour
`dev-groom`, `RESULT` porte la note `_compose_rescue_note` inchangée, arbre
propre ⇒ aucun commit.

**Négatifs à VOIR ROUGES avant d'être remis verts** — sans quoi « le test passe »
et « le test ne regarde rien » sont indistinguables (classe mika#2205) :

- **N1** — jeton retiré du sujet ⇒ V1 rouge.
- **N2** — rang 1 déplacé après le rang 2 ⇒ V2 rouge (la précédence est
  mesurée, pas supposée).
- **N3** — condition de lethality retirée ⇒ V7 rouge.
- **N4** — rangs 3 et 4 fusionnés ⇒ V5 **ou** V6 rouge.
- **N5** — `_resolve_halt_subtype` remplacé par une lecture nue de `SUBTYPE`
  ⇒ V9 rouge (la divergence du § 3.1, rendue visible).

### 5.2 Statique — `test-dispatch-lib.sh`

- **S1** — `_rescue_cause_token` appelle `_halt_family` : le corps de la
  fonction contient `_halt_family`. Sans quoi le jeton porte une table à soi.
- **S2** — le corps de `_rescue_cause_token` ne contient **aucun** nom de
  sous-type amont (`error_max_turns`, `idle_timeout`, `stall_detected`,
  `empty_response`, `rate_limited`, `awaiting_model`, `awaiting_tool`,
  `watchdog_error`, `prompt_cache_dead`, `error_max_budget_usd`,
  `transport_message_too_large`). C'est **R-3 rendu exécutable** : la seule
  forme que prendrait une seconde table.
- **S3** — le scan S2 est **anti-vacuité** : il échoue si
  `_rescue_cause_token` est introuvable ou si son corps est vide. Un scan qui
  regarde une fonction disparue se lit exactement comme un arbre propre.
- **S4** — `_resolve_halt_subtype` a exactement deux appelants de production
  (`_classify_terminated_session`, `_rescue_cause_token`). Un troisième site qui
  résoudrait le sous-type à la main rouvrirait le § 3.1.
- **S5** — `bash -n` sur `dispatch-lib.sh`.
- **S6** — le compte de `commit -m "wip(` reste **3** (le garde existant de
  `test_rescue_commit_no_verify.sh` est ré-exécuté, non dupliqué).

Le drift guard T6 existant n'est **ni touché ni dupliqué** : il couvre déjà le
jeton par construction, puisque le jeton *est* la sortie de `_halt_family`.

### 5.3 Pré-vol — à exécuter et à reporter dans le corps de la PR

1. `bash skills/bundled/_shared/tests/test_rescue_cause_token.sh`
2. `bash skills/bundled/_shared/tests/test_dev_groom_dirty_rescue.sh` — **doit
   passer sans modification** (§ 4.1 point 3)
3. `bash skills/bundled/_shared/tests/test_rescue_commit_no_verify.sh`
4. `bash skills/bundled/_shared/tests/test_rescue_signal_open_pr.sh`,
   `test_rescue_fmt_clean.sh`, `test_rescue_closes_guard.sh`,
   `test_rescue_pipeline_verified.sh`
5. `bash skills/bundled/_shared/test-dispatch-lib.sh` — dont T6 (qui **SKIP**
   dans le bac à sable de dispatch, `types.py` étant hors d'atteinte : le
   reporter comme SKIP et non comme PASS)
6. `make verify-bundled-skills`
7. `bash scripts/canonical-tokens-survey.sh --check`

---

## 6. Fire-Disposition

Ce plan livre des détecteurs : les scans **S1–S4** de `test-dispatch-lib.sh` et
la suite comportementale V1–V12.

**Option retenue : (a) exception nommée en allowlist, ALLOWLIST LIVRÉE VIDE.**

Motif : les scans portent sur deux fonctions que ce plan **crée**. Il ne peut
donc exister aucune violation préexistante, et l'allowlist naît vide plutôt que
de naître avec une dette.

Détail d'implémentation :

```bash
# skills/bundled/_shared/test-dispatch-lib.sh — scan S2
#
# Une entrée = un site qui énumère un sous-type de halte hors de _halt_family,
# avec sa raison et son ticket de suivi. LIVRÉE VIDE.
#
# Quand ce scan tire, la résolution est de router le site vers _halt_family —
# JAMAIS d'ajouter une ligne ici (doctrine mika#2201). Un site qu'on ne veut pas
# router est un site à supprimer.
#
# Comparée DANS LES DEUX SENS : une entrée qui ne matche plus rien fait rougir
# le build. C'est l'assertion auto-nettoyante — le jour de la réparation, pas
# des mois après.
RESCUE_CAUSE_SUBTYPE_ALLOWED=()
```

Le double sens est asserté explicitement : pour chaque entrée, le scan vérifie
que le site nommé existe **et** énumère encore ; une entrée périmée échoue.

**Pré-vol bloquant, à reporter dans le corps de la PR :** S1–S4 doivent être
**verts avec allowlist vide** avant merge. S'ils tirent au pré-vol, la conduite
est (c) **halte-et-remontée** : un scan qui tire sur du code neuf signifie que
le § 2.1 a été implémenté avec une table à soi — corriger l'implémentation, ne
pas allowlister.

V1–V12 ne peuvent structurellement pas tirer sur de l'existant : ils mesurent un
comportement que ce plan introduit. Leur disposition est (a) sans population.

---

## 7. Surfaces opérateur et sondes

**Ce travail n'ajoute aucun compteur, aucun événement de journal, aucune ligne
`audit_events`, et c'est délibéré.** Le défaut est un message de commit muet ; le
correctif est ce message. Une seconde surface qui dirait la même chose ailleurs
serait une seconde vérité à tenir. L'instrument est `git log`.

```bash
# Distribution des causes de sauvetage sur les 200 derniers commits de main
git log --format=%s -200 origin/main \
  | grep -oP '^wip\([^)]*\): rescue \K[a-z_]+' | sort | uniq -c | sort -rn

# Recouper un sauvetage avec le callback de son dispatch
git log --format='%h %s' --grep='^wip(.*): rescue ' -20
```

| jeton | régime attendu | lecture |
|---|---|---|
| `budget_exhausted` | **non vide** | la voie normale R-class (décision Prime/Vincent : le plafond 150 reste, l'iterate-from-wip est nominal) |
| `policy_deny` | non vide, faible | chaque ligne est un trou d'allow-list qui a coûté une session |
| `session_silent` / `model_unproductive` | faible | stall SDK ; voisin de claude-pilot#168 |
| `no_halt_signal` | **faible** | le pilote a fini nominalement et laissé de la saleté — c'est un défaut de queue de session, pas une coupure |
| `halt_unmapped` | **vide** | tout occurrence est un sous-type amont hors de `_halt_family` |

**Sonde S1 — le jeton atterrit (premier sauvetage après déploiement).** Le sujet
porte `rescue <cause>`, et la cause concorde avec la ligne `Halt class:` du
callback du même dispatch.
*Halte 1 — le sujet est générique.* Ne pas retoucher le compositeur :
`skills/bundled/_shared/` est une projection du **binaire**, pas du checkout —
`cat ~/.mika/skills/.manifest-writer` et établir le déploiement avant toute
conclusion (classe mika#2340).

**Sonde S2 — attribution (30 jours).** La distribution ci-dessus doit être
**non dégénérée** : si un seul jeton porte la totalité du trafic, le résolveur
ne discrimine pas.
*Halte 2 — tout est `no_halt_signal`.* `SUBTYPE` n'arrive pas jusqu'au
sauvetage. Lire d'abord si les callbacks portent une ligne `Halt class:` ; si
oui, c'est le § 3.2 qui n'a pas pris, **pas** le vocabulaire à élargir.

**Sonde S3 — contrôle négatif de l'accord (30 jours).** Aucun couple
(commit `rescue X`, callback `Halt class: Y`) avec X ≠ Y, hors le cas
`policy_deny` où la divergence est le **contrat** du rang 1 (R4).
*Halte 3 — une divergence hors ce cas.* Le § 3.2 a été contourné : un troisième
résolveur existe. Le scan S4 est censé le refuser — s'il est vert malgré la
divergence, c'est **le scan** qu'il faut réparer, pas le vocabulaire.

**Sonde S4 — `halt_unmapped` non vide.** C'est un **résultat**, pas une panne :
un sous-type amont a été ajouté. Le remède est une ligne dans `_halt_family`
(ce qui corrige du même geste le callback, le bandeau et le commit) — jamais une
branche dans `_rescue_cause_token`. Le drift guard T6 le dit déjà de son côté
quand `types.py` est atteignable ; `halt_unmapped` est le même fait vu depuis
l'historique git, donc lisible là où T6 ne tourne pas.

**Ce que ce travail n'achète pas.** Il ne dit rien des sauvetages **déjà**
committés : aucune ligne ne réécrit l'historique, et les wip de #2532 et #2536
gardent leur message générique — une cause inventée après coup serait pire
qu'une cause absente, elle aurait l'air d'une mesure. La sonde est le
**prochain** sauvetage.

---

## Definition of Done

- [ ] `_resolve_halt_subtype` livrée (mémoïsée, réinitialisée au site de
      `SUBTYPE`), et `_classify_terminated_session` réécrit pour l'appeler dans
      ses deux branches.
- [ ] `_rescue_cause_token` livrée : quatre rangs, déléguant à `_halt_family`,
      n'énumérant aucun sous-type amont.
- [ ] `_rescue_dirty_worktree` interpole le jeton dans `_rescue_what` — **aucun
      des deux sites `git commit` n'est modifié**.
- [ ] `skills/bundled/_shared/tests/test_rescue_cause_token.sh` : V1–V12 verts,
      N1–N5 **vus rouges** avant d'être remis verts.
- [ ] Scans S1–S6 verts dans `test-dispatch-lib.sh`, allowlist S2 **vide**.
- [ ] `test_dev_groom_dirty_rescue.sh` vert **sans modification** (contrôle de
      non-régression du § 4.1).
- [ ] Les quatre autres suites `test_rescue_*.sh` vertes.
- [ ] `make verify-bundled-skills` vert.
- [ ] Documentation : commentaire de site sur les deux nouvelles fonctions
      (précédence R4, divergence du § 3.1, allowlist vide) ; mise à jour du
      commentaire de `_rescue_dirty_worktree:4086` qui décrit la composition du
      sujet.
- [ ] Pré-vol du § 5.3 exécuté, **T6 reporté comme SKIP et non comme PASS**, et
      le résultat reporté dans le corps de la PR.
- [ ] Corps de PR : section Fire-Disposition citée (allowlist vide, conduite si
      elle tire).

---

## Acceptance criteria

Le corps de senara-solutions/mika#2539 porte `## Attendu` et non
`## Acceptance criteria` ; les critères ci-dessous en sont dérivés, testables.

- **AC1** — Le sujet du commit de sauvetage porte le lien ticket (`mika#<N>`,
  déjà présent) **et** un jeton de cause de la forme `rescue <cause>`.
- **AC2** — Le jeton distingue au minimum les quatre cas nommés par le ticket :
  plafond de tours atteint, deny classifieur, stall SDK, arbre sale sans
  coupure. Une session dont la cause est connue n'est jamais étiquetée
  « cause non identifiée ».
- **AC3** — Les deux sessions mesurées (#2532 deny, #2536 plafond) produiraient
  des jetons **différents**, rejouées sur leurs signaux réels.
- **AC4** — Le comportement de recovery est inchangé : conditions de
  déclenchement, exclusions, `--no-verify`, `POST_RUN_HEAD`,
  `RESCUED_DIRTY_WORKTREE`, `RESULT`.
- **AC5** — Le vocabulaire a un seul site de définition et aucune seconde
  énumération des sous-types de halte.

**Couverture :**

| AC | Où c'est livré | Où c'est attesté |
|---|---|---|
| AC1 | § 4.1 — jeton inséré entre préfixe et queue | V1–V11 ; N1 vu rouge |
| AC2 | § 2.1 — quatre rangs ; § 2.2 — table de couverture ; rangs 3/4 distincts (R3) | V1, V3, V4, V5, V6 ; N4 vu rouge |
| AC3 | § 2.1 rang 1 et sa précédence (R4) | **V1 (#2536) et V2 (#2532)** ; N2 vu rouge |
| AC4 | § 4.2 — aucun site `git commit` touché ; seule `_rescue_what` change | contrôles de non-régression du § 5.1 ; `test_dev_groom_dirty_rescue.sh` vert sans modification ; S6 (compte à 3) |
| AC5 | § 2.1 — délégation à `_halt_family` ; § 3.2 — résolveur unique | S1, S2, S4 ; S3 anti-vacuité ; N5 vu rouge |

---

## 8. Hors périmètre, délibérément

- **Le site mika#1383** (`:4720`, `wip(…): trailing content after pilot
  end_turn`). Population **disjointe** : ce site traite les sessions dont HEAD a
  **avancé** — le pilote a committé puis laissé de la saleté. Le ticket nomme
  « le compositeur du message de recovery (mika#1282) ». La question « pourquoi
  le pilote n'a-t-il pas committé ? » n'a pas de sens pour une session qui a
  committé. **Suivi possible**, précondition : une mesure montrant qu'on lit
  aussi ces sujets pour en chercher la cause.
- **Le corps de la PR de sauvetage** (`_rescue_class_fact`, `:8617`). Le ticket
  borne à « seulement le message (donc l'historique) » ; le corps de PR est une
  surface éphémère, l'historique git ne l'est pas. Adjacent et cohérent à
  faire ensuite ; pas ici.
- **`_compose_rescue_note`** (la note dans `RESULT`). Le callback porte déjà la
  cause sur sa ligne `Halt class:` — l'y répéter créerait une seconde vérité à
  tenir, et R-5 interdit de toucher `RESULT`.
- **La cause des coupures elle-même** — le plafond 150 (décidé conservé par
  Prime + Vincent le 2026-09-26), les trous d'allow-list du classifieur, les
  stalls SDK (claude-pilot#168). Ce travail rend la cause **lisible**, il ne la
  fait pas disparaître.
- **`error_during_execution` dans `_halt_family`.** Le ticket le cite comme
  signal, et il est absent de la table (c'est un sous-type SDK, que le drift
  guard T6 ne lit pas — T6 ne lit que le `Literal` de `types.py`). Il sortirait
  donc en `halt_unmapped` — **sauf** que le rang 1 l'attrape déjà en
  `policy_deny` sur la population mesurée (#2532), le deny étant sa cause
  réelle. L'ajouter à `_halt_family` change le **callback** de toutes les
  sessions concernées, pas seulement le commit : blast radius distinct, ticket
  distinct. **Suivi nommé**, précondition : que la sonde S4 montre
  `halt_unmapped` non vide sur ce sous-type.
- **Réécrire l'historique des sauvetages passés.** Voir § 7.
