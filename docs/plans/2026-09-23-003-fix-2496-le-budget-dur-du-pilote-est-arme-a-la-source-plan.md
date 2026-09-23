# mika#2496 — Le budget dur du pilote est armé à la source

> **Parent umbrella :** #2491 (Défaut 4). Enfant du cadre substrat — 1 PR atomique.
> **Ne PAS mettre `ready` sans ratification opérateur.** Cette réserve vient de la
> carte de l'umbrella #2491 et **non** d'une rectification de #2496 : ce plan
> réfute le correctif littéral du ticket (§ 2, A1) et ne livre pas son DoD tel
> qu'il est écrit. La divergence doit donc être portée **dans le corps de #2496**
> avant l'armement — c'est l'item 0 du DoD, et c'est bloquant.

## 1. Le défaut, tel que le ticket le pose

« La règle *kill à 120 tours / 40 USD* ne peut pas être appliquée en vol : le
compte de tours SDK n'est nulle part en temps réel (le DB `Turns:` n'existe
qu'à la fin), et le proxy *lignes de transcript* est non fiable (ratio mesuré
1.63 pour #2425, 2.16 pour #2484). #2425 a tronqué à 142 tours (>120) sans
jamais franchir le seuil-lignes calibré. »

Correctif proposé : exposer le compte de tours et le coût cumulé en temps réel,
« pour qu'un budget dur soit applicable côté moteur ».

Les deux emballements de référence : **#2425 (142 tours)**, **#2484 (201 tours,
86 USD)**.

## 2. Ce que la lecture du code déplace dans le ticket

C'est le premier livrable, et il déplace les deux moitiés dans des directions
opposées. Le ticket est **exact sur son constat** (le proxy-lignes est mort) et
**faux sur sa conclusion** (il faut un compteur temps-réel côté moteur).

### 2.1 — R1. Le budget de tours existe déjà, natif et exact. Personne ne l'arme.

`claude-pilot` expose depuis toujours un plafond de tours **SDK-natif** :

| maillon | site | état |
|---|---|---|
| drapeau CLI `--max-turns` | `claude-pilot/src/claude_pilot/cli.py:69` | existe |
| validation (`≥ 1`) | `cli.py:85-87` | existe |
| mapping `out["maxTurns"]` | `cli.py:157-158` | existe |
| champ `GuardrailConfig.maxTurns` | `types.py:22` | existe |
| résolution des défauts | `guardrails.py:179` | existe |
| **`kwargs["max_turns"] = config.maxTurns`** | `agent.py:1194-1195` | **atteint le SDK** |
| sous-type de terminaison `error_max_turns` | `agent.py:54` | existe |
| classification aval | `dispatch-lib.sh:3543` | existe |

Le défaut du plafond est **`maxTurns=200`** (`types.py:77`), et
`skills/bundled/_shared/dispatch-lib.sh` **ne passe le drapeau à aucun de ses
trois sites de lancement** (`3060`, `5912`, `6031`).

**Ça explique les deux emballements exactement, et c'est la mesure qui tranche :**

- **#2484 — 201 tours.** C'est `maxTurns=200` qui **a mordu** (200 tours servis,
  plus le `ResultMessage` terminal que le compteur du pilote rapporte). Le
  mécanisme fonctionne de bout en bout ; il est simplement réglé à 200 et non à
  120, parce que rien en aval ne l'a jamais réglé.
- **#2425 — 142 tours.** Sous 200 : rien n'avait à mordre. La troncature vient
  d'ailleurs (garde-fou applicatif de temps, ou geste opérateur).

`permissions.py:751-752` le dit déjà, en toutes lettres, dans le dépôt d'à
côté : *« `maxTurns=200` — the real bound. SDK-native; ends the run with
`error_max_turns`, a genuine ResultMessage »*. Et `agent.py:165` : *« `maxTurns`
is the ONLY guardrail that bounds a BUSY refusal loop »*.

**Donc la règle *est* applicable en vol, et mieux que ce que le ticket
demande** — non par un compteur lu de l'extérieur, mais par le SDK qui compte
ses propres tours et termine proprement.

### 2.2 — R2. Le budget en dollars est un drapeau qui ne fait RIEN

Même chaîne, et elle s'arrête une ligne avant la fin :

```python
# claude-pilot/src/claude_pilot/agent.py:1191-1202
def _sdk_guardrail_kwargs(config: Any) -> dict[str, Any]:
    """Pass SDK-native guardrails only when > 0."""
    kwargs: dict[str, Any] = {}
    if config.maxTurns > 0:
        kwargs["max_turns"] = config.maxTurns
    # maxBudgetUsd is TS-SDK-specific; the Python SDK accepts it via
    # permission_mode/options extras if exposed. Include defensively.
    if config.maxBudgetUsd > 0:
        # Attribute name varies by SDK version; set if the option exists.
        # Leaving it out is safe — application-level guardrails still apply.
        pass
    return kwargs
```

`--max-budget` est parsé (`cli.py:70`), validé (`cli.py:88-89`), mappé
(`cli.py:159-160`), résolu (`guardrails.py:180`), porté jusqu'à cette fonction —
puis **`pass`**. Le sous-type `error_max_budget_usd` est déclaré
(`agent.py:54`) et classé en aval (`dispatch-lib.sh:3545`) : **un vocabulaire
de terminaison pour un mécanisme que rien ne peut produire.**

Et le commentaire qui l'autorise est faux sur cet axe précis : *« application-level
guardrails still apply »* — les garde-fous applicatifs (`stallThreshold`,
`emptyResponseThreshold`, `idleTimeoutMs`, `toolWaitCeilingMs`,
`modelWaitCeilingMs`) mesurent tous du **temps** ou de la **productivité**.
**Aucun ne mesure des dollars.** Le défaut est `maxBudgetUsd=0.0` (*« 0 =
disabled »*, `types.py:78`), et même armé il ne ferait rien.

**Conséquence directe : les 86 USD de #2484 n'ont franchi aucune borne parce
qu'il n'en existe aucune.** Et armer `--max-budget 40` depuis mika poserait une
garantie fausse — un budget annoncé fermé et ouvert en fait, c'est-à-dire
exactement la classe de défaut que ce ticket existe pour fermer
(cf. mika#2304 : *un champ qui affirme, avec autorité, l'override qui n'a pas eu
lieu*).

### 2.3 — R3. Aucune surface mika ne porte le compte de tours en temps réel

Le correctif littéral du ticket n'est pas seulement inutile : il n'est **pas
réalisable dans ce dépôt**. Les deux seules surfaces vivantes le disent :

**(a) Le transcript JSONL** (`~/.mika/data/pilot-transcripts/<task-id>.jsonl`).
Son écrivain — `claude-pilot/src/claude_pilot/transcript_writer.py:42,65-88` —
écrit **une ligne par `AssistantMessage` *ou* `ResultMessage`**, avec
`tokens_in`, `tokens_out`, `latency_ms`, `model`, `response_body`.
**Aucun champ `turns`, aucun champ `cost_usd`.** Le compte de lignes est donc
un compte de *messages assistants*, pas de tours — le SDK regroupant plusieurs
`AssistantMessage` sous un même `num_turns`. **C'est précisément le ratio que le
ticket a mesuré (1.63, 2.16), et il n'est pas constant :** le proxy n'est pas
mal calibré, il est incalibrable. Le ticket a raison, et sa raison interdit sa
propre piste.

Le `ResultMessage` terminal porte bien `num_turns` et `total_cost_usd` dans
`response_body` (`dataclasses.asdict(message)`) — mais **à la fin**, ce qui est
l'état que le ticket décrit déjà.

**(b) Le log de session `--verbose`.** `log_turn_summary(turn, …)`
(`ui.py:224-232`) porte un index de tour, mais n'est appelé qu'aux deux sites
de `agent.py:1217,1219` — **uniquement sur un tour diagnostiquement muet**.
Sur un run sain il n'émet rien. Le plus grand index vu est une borne
inférieure, absente la plupart du temps.

### 2.4 — R4. Ce que le correctif du ticket coûterait s'il était écrit quand même

Un compteur moteur ne peut tuer que **de l'extérieur** : signal au groupe de
processus, comme le fait le faucheur de silence (mika#2249/#2277). Le SDK, lui,
termine le run avec un `ResultMessage` authentique — donc le JSON de sortie, le
`Turns:`/`Cost:` du callback, `extract_callback_fields`, la chaîne de
récupération post-vol et la classification `_halt_family` fonctionnent tous.
**Une autorité en aval serait plus faible que celle qui existe déjà en amont**,
et elle s'ajouterait à un plafond de 200 que personne n'aurait remarqué.

## 3. Ce qui est livré

Une PR, dans `mika` seul.

### U1 — Armer le plafond de tours aux trois sites de lancement

Un résolveur unique dans `dispatch-lib.sh`, calqué **trait pour trait** sur
`_pilot_log_dir` (`dispatch-lib.sh:246-248`), y compris sa discipline de
co-location :

```sh
# Le plafond de tours du pilote. Réglage de DÉMARRAGE, lu à chaque lancement.
# Vide ou 0 => le drapeau n'est PAS passé et claude-pilot retombe sur son
# propre défaut (maxTurns=200) : c'est le rollback, et il restaure le
# comportement d'avant mika#2496 à l'octet près.
_pilot_max_turns() {
    _PILOT_MAX_TURNS="${PILOT_MAX_TURNS:-120}"
}
```

- **Nom non préfixé**, par précédent tenu dans le même fichier : `PILOT_LOG_DIR`
  l'est, et mika#2249 a écrit pourquoi (`scrub_mika_env_vars` retire tout
  `MIKA_*` de l'enfant de dispatch). *Le préfixe effectivement survivant est une
  vérification V1a/V1b, pas une hypothèse.*
- **Trois paliers maison** : absent → défaut ; `0` ou vide → drapeau omis
  (rollback) ; illisible ou négatif → défaut **avec un WARN nommant la valeur
  entre guillemets**. Un désarmement par coquille sur un frein de coût serait la
  panne silencieuse que tout ceci ferme.
- Le drapeau est ajouté aux **trois** sites (`3060` dispatch principal, `5912`
  pilote de révision, `6031` free-dispatch). Les deux derniers n'approchent
  jamais 120 ; les armer ne coûte rien et ferme la surface **structurellement**,
  plutôt que par une liste que personne ne relit.

### U2 — Le budget armé est DIT à chaque lancement

Doctrine mika#2293, citée mot pour mot parce qu'elle décrit ce ticket : *un
réglage qu'on ne peut pas observer n'est pas un réglage, c'est un espoir.* Une
ligne, sur `stderr` de dispatch-lib, donc dans le sillon forensique
per-dispatch (`$PILOT_LOG_DIR/<task-id>.stderr`) :

```
dispatch-lib: pilot_budget_armed max_turns=120 source=default cost_bound=absent_upstream
```

- `source ∈ {default, env}` — sépare « le réglage est en vigueur » de « une
  variable de service l'écrase », les deux remèdes étant opposés.
- **`cost_bound=absent_upstream` est posé en dur et c'est le point.** Cette
  ligne refuse d'annoncer un budget dollars, parce qu'il n'en existe pas
  (R2). Elle nomme l'absence au lieu de la taire — et le jour où le suivi cpp
  aboutit, c'est cette valeur qui change.
- Ancrage `^dispatch-lib: ` obligatoire à la lecture : le `.stderr` porte aussi
  la prose du pilote, et mika#2050 a mesuré le faux positif (une session
  discutant du signal apparaissait comme une émission).

### U3 — La garde structurelle : aucun site de lancement sans son plafond

`skills/bundled/_shared/test-dispatch-lib.sh` (7 609 lignes, le harnais
existant) reçoit un **scan de source** : tout **lancement** de `claude-pilot`
dans `dispatch-lib.sh` doit porter `--max-turns`. **Allowlist livrée vide.**

Aucun test comportemental ne peut voir cette classe : un quatrième site de
lancement écrit demain sans le drapeau ne rend **aucune** décision fausse — il
tourne simplement sans borne, et toutes les assertions existantes restent
vertes. C'est la forme exacte du défaut que ce ticket ferme, reproduite un cran
plus tard.

**Le motif est figé ici, parce qu'un scan sur « tout appel `claude-pilot` » est
faux dans les deux directions.** La chaîne apparaît **47 fois** dans le fichier,
dont une seule famille est un lancement : commentaires, `--ro-bind-try
"$HOME/.local/bin/claude-pilot"`, `claude-pilot.json` (config du relais), et de
la prose dans les corps `RESULT=`. Un scan naïf rougirait sur les 44 autres ;
resserré sur la ligne, il **manquerait deux des trois sites réels**, qui sont
écrits sur plusieurs lignes avec continuation `\` — `--max-turns` y vivrait
légitimement sur une ligne que le grep ne regarde pas. Le prédicat est donc :

1. **Unité d'analyse = l'invocation logique**, obtenue en joignant les
   continuations `\` (un `sed`/`awk` de recollement), jamais la ligne physique.
   *C'est le terme que la forme des sites 5912/6031 impose.*
2. Les lignes de commentaire (`^[[:space:]]*#`) sont retirées **avant** le
   recollement.
3. **Candidat** = une invocation logique où `claude-pilot` apparaît en **position
   de commande** : précédée d'un début d'invocation (début de ligne, `;`, `|`,
   `&&`, `||`, `(`) ou du nom du wrapper `_run_pilot_sandboxed`, **et** non
   précédée d'un `/` (exclut les chemins) **et** non suivie de `.json` (exclut la
   config du relais).
4. **Assertion** : chaque candidat contient `--max-turns`. Le compte de candidats
   est lui aussi asserté **égal à 3** — sans quoi un prédicat devenu trop étroit
   passerait au vert en ne regardant plus rien (classe mika#2205, que ce plan
   invoque pour S4 et doit s'appliquer à son propre scan).

Le terme 3 est délibérément une **disjonction** (`_run_pilot_sandboxed` *ou*
`claude-pilot` en position de commande) et non le wrapper seul : un site futur
qui appellerait le binaire sans passer par `_run_pilot_sandboxed` échapperait à
un prédicat ancré sur le wrapper — et ce site-là est aussi celui qui perdrait le
confinement réseau (mika#2049), donc il doit rougir ici plutôt que passer.

Plus la co-location `_pilot_max_turns` / `$_PILOT_MAX_TURNS` sur la même ligne,
par le mécanisme déjà en place pour `_pilot_log_dir` (le harnais refuse déjà
toute lecture de `$_PILOT_LOG_DIR` hors d'une ligne appelant son résolveur).

### U4 — Le dépassement de coût est MESURÉ, à défaut d'être empêché

Le frein dollars n'existe pas en amont (R2) et ce dépôt ne peut pas le créer.
Ce qu'il peut faire, c'est **compter la population** — et c'est exactement la
précondition que le ticket de suivi cpp réclamera.

`extract_callback_fields` (`dispatcher.rs:4265-4300`) parse déjà `Cost:` dans
`metadata.claude_pilot.cost_usd`. Au même site, quand la valeur dépasse le seuil
de la règle, une ligne et une row :

- `pilot_cost_overrun` (WARN, champs `repo`, `issue`, `task_id`, `cost_usd`,
  `turns`, `threshold_usd`) ;
- row `audit_events` sous le même nom, **SOLE WRITER**, épinglé par scan de
  source.

`PILOT_COST_ALERT_USD`, défaut **40** (la règle), mêmes trois paliers. C'est
**rétrospectif et assumé comme tel** : ça n'empêche rien, ça date et ça compte.
Sans cette mesure, le ticket cpp s'ouvrirait sur une intuition.

**40 est posé comme la règle, pas comme une mesure — et c'est une asymétrie
assumée, pas un oubli de la discipline exigée pour les tours (V2).** Un seuil
d'*alerte* mal calibré produit du bruit ou du silence, et S4 ne détecte que le
second (parse muet) ; mais contrairement au plafond de tours, il ne **coupe
rien** — un faux positif coûte une ligne WARN, pas un run tronqué, ce qui est
précisément ce qui autorise à poser la règle d'abord et à mesurer ensuite. Le
seuil est donc **révisé sur la première distribution mesurée par U4** (doctrine
du § 5.1, mika#2293 : on règle sur ce qu'on observe, pas sur ce qu'on espère) —
et cette révision est le premier livrable du ticket de suivi cpp, à qui U4 fournit
sa population. Si la distribution montre que 40 porte le trafic nominal, c'est le
seuil qui monte, pas la mesure qu'on désarme.

### Ce qui n'est PAS livré, et pourquoi le drapeau dollars n'est pas passé

`--max-budget 40` **n'est pas ajouté**. Il serait accepté, validé, résolu,
porté jusqu'à `_sdk_guardrail_kwargs` — et ignoré. Le passer ferait paraître le
budget fermé, dans l'argv, dans la ligne U2, et dans la lecture de quiconque
ouvrirait le fichier. **Une borne annoncée et inerte est pire que pas de
borne :** elle éteint la question.

## 4. L'arithmétique, écrite parce qu'elle décide du périmètre

La règle est « 120 tours **/** 40 USD ». Ce qui est livré ferme le premier
terme et **pas** le second, et les deux ne se substituent pas :

`#2484 = 201 tours / 86 USD` → ≈ **0,43 USD par tour**. À ce taux, 120 tours ≈
**51 USD** — au-dessus de 40. **Le plafond de tours ne fait donc pas respecter
le plafond dollars**, il le borne seulement par le haut, et une session au
contexte plus lourd le franchirait plus tôt encore. Le dire est la condition
pour que personne ne lise cette PR comme « la règle est appliquée ».

Ce qu'elle achète, chiffré sur la population mesurée : #2484 aurait été coupé à
120 tours au lieu de 201, soit ≈ **40 % du coût évité** sur ce run ; #2425 à 120
au lieu de 142, soit ≈ 15 %.

## 5. Le risque que ça crée, et ce qui le paye

**Armer à 120 tronque les runs qui dépassent légitimement 120.** #2425 a atteint
142 : sous ce correctif, il aurait été coupé. Trois choses payent ce risque, et
la première est bloquante.

1. **La valeur par défaut est dérivée d'une mesure, pas de la règle seule
   (V2, bloquant).** Le compte de tours terminal est persisté depuis toujours
   dans `metadata.claude_pilot.turns`. La distribution se lit avant d'armer :

   ```sql
   SELECT json_extract(metadata, '$.claude_pilot.turns') AS turns,
          json_extract(metadata, '$.claude_pilot.cost_usd') AS cost
     FROM tasks
    WHERE source = 'self_dev' AND metadata LIKE '%claude_pilot%'
      AND turns IS NOT NULL
    ORDER BY turns DESC LIMIT 50;
   ```

   **Halte :** si le p95 des runs **aboutis** (PR ouverte) dépasse 120, la
   valeur 120 tronque le régime nominal et **le défaut doit être relevé, pas la
   règle appliquée en aveugle** — avec le compte, remonté à l'opérateur. Livrer
   une borne qui coupe la population saine, c'est déplacer le défaut, pas le
   fermer.

2. **La chaîne de récupération, qui est déjà promise et doit être vérifiée
   (V3a/V3b).** `_halt_family` affirme, pour les deux sous-types de budget : *« work
   was produced and the recovery chain carries it »* (`dispatch-lib.sh:3543-3546`).
   La récupération post-vol (mika#1282) committe un worktree sale en `wip()` et
   ouvre une PR brouillon. **Cette promesse est une affirmation du dépôt, pas
   une mesure :** V3a l'exerce déterministiquement sur le commit `wip()`, V3b
   (= sonde S2) sur la PR brouillon. Si elle ne tient pas, **U1 ne doit pas être
   armé** — et la réparation de la chaîne devient le préalable. Exécutants,
   moments et artefacts : tableau du § 11.

   **La promesse et V3a/V3b ne portent QUE sur `error_max_turns`, et le dire lève une
   lecture fausse.** `_halt_family` range les deux sous-types de budget sous la
   même affirmation, mais R2 établit que `error_max_budget_usd` **ne peut être
   produit par aucun chemin** (`_sdk_guardrail_kwargs` s'arrête à `pass`) : c'est
   un vocabulaire de terminaison sans émetteur. V3a/V3b n'exercent donc que le premier,
   et rien dans ce plan ne doit se lire comme « la chaîne est vérifiée sur les
   deux » — la moitié dollars est un contrat non exercé faute de population, et
   elle le restera jusqu'au ticket de suivi cpp (§ 8.1). Même discipline que
   mika#2304, invoqué au § 2.2 : une garantie annoncée plus large que ce qui a
   été mesuré est la classe de défaut que ce ticket ferme.

3. **Le rollback est une variable, sans redéploiement :** `PILOT_MAX_TURNS=0`
   omet le drapeau et rend le comportement d'avant à l'octet près (défaut
   amont 200).

## 6. Alternatives refusées

| # | Alternative | Pourquoi refusée |
|---|---|---|
| A1 | **Le correctif littéral du ticket** : compteur temps-réel côté moteur + kill | Autorité plus faible (kill externe *vs* `ResultMessage` propre qui préserve callback, métadonnées et récupération), et **non réalisable** : aucune surface mika ne porte un compte de tours exact (R3) |
| A2 | Enforcement sur le compte de lignes du transcript | C'est le proxy que le ticket réfute : ratio mesuré 1.63/2.16, non constant **par construction** (plusieurs `AssistantMessage` par `num_turns`). réparer un proxy par le même proxy ne ferme rien |
| A3 | Enforcement sur `sum(tokens_in + tokens_out)` du transcript | Exact pour ce qu'il mesure, mais les lignes v1 ne portent **ni cache-read ni cache-creation** — qui dominent sur un pilote. Un budget dollars bâti là-dessus **sous-estime**, dans la direction dangereuse |
| A4 | Passer `--max-budget 40` « au cas où » | Silencieusement inerte (R2). Poserait une garantie fausse dans l'argv et dans la ligne U2 |
| A5 | Réparer `_sdk_guardrail_kwargs` dans ce PR | Autre dépôt. L'umbrella prescrit **1 PR atomique** sur `mika` |
| A6 | Un plafond par site de lancement | YAGNI : les deux sites de révision n'approchent jamais 120. Un knob unique, trois sites armés |
| A7 | Bumper le schéma transcript en v2 avec `turns`/`cost_usd` | Changement **cross-repo** que `pilot_transcript.rs:44-48` encadre explicitement (accepter les deux versions une release). Et ça ne servirait que l'observabilité, l'enforcement étant déjà résolu en amont |

## 7. Fichiers touchés

| fichier | nature |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` | résolveur `_pilot_max_turns`, `--max-turns` aux 3 sites, ligne `pilot_budget_armed` |
| `skills/bundled/_shared/test-dispatch-lib.sh` | scan de source (allowlist vide), co-location, rollback, trois sites |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | U4 : `pilot_cost_overrun` au site `extract_callback_fields` |
| `crates/mika-agent/src/canonical_tokens.rs` (tests) | SOLE WRITER de `pilot_cost_overrun` |
| `CLAUDE.md` (racine) | section budget du pilote : les deux knobs, la ligne U2, l'absence amont du frein dollars, les sondes |
| `crates/mika-agent/CLAUDE.md` | U4 dans le voisinage de `extract_callback_fields` |

**Aucune migration, aucune colonne, aucun changement de politique de dispatch.**
`maxTurns` amont, `MAX_DENY_RESUMES`, les plafonds d'attente cpp#145, les
faucheurs mika#2249/#2277 : tous inchangés.

## 8. Périmètre non couvert — nommé, avec sa raison

1. **`_sdk_guardrail_kwargs`'s `pass` (le frein dollars).**
   → **Ticket de suivi `senara-solutions/claude-pilot`**, et c'est le seul
   endroit où le second terme de la règle peut être fermé. Précondition : la
   mesure U4. Note à y porter : le commentaire *« application-level guardrails
   still apply »* est faux sur cet axe.
2. **Un compte de tours en temps réel, exact.**
   → **Ticket de suivi cpp.** `TurnBoundaryEvent.just_closed_turn` existe déjà
   en interne (`agent.py:1204-1219`) ; l'émettre à **chaque** frontière de tour,
   ou ajouter `turns` à la ligne transcript, est petit. Précondition : un besoin
   mesuré — l'enforcement n'en a plus besoin.
3. **L'interaction budget × `DEFAULT_MAX_DENY_RESUMES`.** `agent.py:164-165`
   l'écrit : le plafond effectif d'un run est `(1 + resumes) × maxTurns`, donc
   **jusqu'à 3 × 120 = 360 tours** dans le pire cas de reprise après refus. Réel,
   amont, et **ce PR ne le change pas** (il l'améliore : 3 × 120 < 3 × 200).
   Le fermer suppose un compteur cumulatif inter-reprises, côté cpp.
4. **Une borne de durée.** Le faucheur de silence (mika#2249/#2277) borne le
   *mutisme*, pas la durée d'un pilote bavard. Autre population, autre ticket.
5. **`docs/architecture/pilot-transcript-schema.md`**, référencé par
   `pilot_transcript.rs:38-39` et **absent de l'arbre**. Trouvé en chemin, sans
   rapport avec ce défaut. **Ticket de suivi.**

## 9. Surfaces opérateur

```bash
# 1. Sous quel budget ce dispatch a-t-il tourné ? (ancrage obligatoire)
grep -h '^dispatch-lib: pilot_budget_armed' \
     "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | tail

# 2. Contrôle POSITIF — combien de dispatches ont été armés ?
grep -lc '^dispatch-lib: pilot_budget_armed' \
     "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | wc -l

# 3. Le plafond a-t-il mordu ? (famille budget_exhausted, déjà classée)
grep -h 'error_max_turns' "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | tail

# 4. Les dépassements de coût (U4)
grep pilot_cost_overrun "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{repo, issue, turns, cost_usd, threshold_usd}'
```

```sql
-- La population que le suivi cpp devra dimensionner
SELECT count(*), round(avg(CAST(after_value AS REAL)), 2)
  FROM audit_events WHERE tool_name = 'pilot_cost_overrun';
```

| événement | niveau | régime attendu | lecture |
|---|---|---|---|
| `pilot_budget_armed` | stderr | **une par dispatch** | son absence = binaire antérieur (classe mika#2340), jamais « pas de budget » |
| `pilot_budget_invalid` | stderr | **vide** | une coquille dans la variable, nommée entre guillemets |
| `error_max_turns` | stderr | **non vide, faible** | chaque ligne est un emballement coupé — c'est le succès, pas la panne |
| `pilot_cost_overrun` | WARN | **non vide** | dimensionne le suivi cpp ; décroît si le plafond de tours borne en pratique |

**Le contrôle positif (2) n'est pas décoratif.** Zéro `error_max_turns` avec zéro
`pilot_budget_armed` ne dit rien du tout ; zéro avec un compte d'armements non
nul dit que la flotte ne s'emballe pas. C'est la classe mika#2205 : *une garde
qu'on n'a pas déployée se lit exactement comme une flotte saine.*

## 10. Sondes post-déploiement, et leurs quatre haltes

**S1 — l'armement atteint l'argv (premier dispatch).** `pilot_budget_armed` avec
`max_turns=120`, et `--max-turns 120` dans l'argv réel.
**Halte 1 — aucune ligne.** Ne pas retoucher le résolveur : établir d'abord que
le binaire servi porte le correctif. `skills/bundled/` n'est lu par aucun agent
avant `make deploy` → seed (classe mika#2340) ; `cat ~/.mika/skills/.manifest-writer`.

**S2 — le plafond mord (30 jours). C'est l'exécution de V3b** (opérateur ;
artefact : commentaire sur #2496). Au moins une `error_max_turns`, classée
`budget_exhausted|deterministic`, **avec sa PR brouillon de récupération**.
`error_max_budget_usd` n'entre pas dans cette sonde : aucun chemin ne peut le
produire (R2), donc son absence n'est pas un signal.
**Halte 2 — une troncature sans travail récupéré.** La promesse de
`_halt_family` ne tient pas. **Désarmer (`PILOT_MAX_TURNS=0`) avant tout
diagnostic** : un budget qui détruit le travail est pire que l'emballement
qu'il coupe.

**S3 — pas de régression sur le régime nominal (30 jours).** La part de
dispatches terminés `error_max_turns` doit rester **marginale**.
**Halte 3 — elle porte le trafic nominal.** 120 est sous la population saine et
la mesure V2 était fausse ou datée : **relever le défaut, avec la distribution**,
plutôt que laisser un frein couper le travail courant. *Un frein est un frein,
pas un chemin.*

**S4 — la population coût (30 jours).** Le compte `pilot_cost_overrun` **est**
la précondition du suivi cpp.
**Halte 4 — compte nul alors que des runs dépassent 40 USD.** Le parsing de
`Cost:` ne rend rien sur cette population : vérifier `extract_callback_fields`
sur un `status: terminated` **avant** de conclure que la flotte est sous le
seuil. Une sonde muette et une flotte saine se lisent pareil.

## 11. Contrat de vérification (tests)

**Chaque vérification bloquante porte son exécutant, son moment et son
artefact.** Sans les trois, un bloquant différé est un TBD déguisé
(Unresolved-Decision Gate, mika#1244) : personne ne sait qui doit agir, à quel
moment du cycle la PR est retenue, ni où lire que c'est fait. Le tableau est le
contrat ; les paragraphes qui suivent en portent les haltes.

| V | question | exécutant | moment | artefact de rapport | ce qu'un rouge bloque |
|---|---|---|---|---|---|
| **V1a** | le préfixe non préfixé est-il le bon, statiquement ? | **implémenteur** | avant PR | corps de la PR | rien — informe V1b |
| **V1b** | la variable atteint-elle vraiment le dispatch ? | **opérateur** (hôte de dispatch) | **premier dispatch après déploiement** | **commentaire sur #2496** | la revendication **AC4** (rollback), pas U1 |
| **V2** | quelle est la distribution des tours des runs aboutis ? | **opérateur** (base de production) | **avant merge** | **commentaire sur #2496** | la **valeur** par défaut — sans elle, U1 est livré désarmé |
| **V3a** | `error_max_turns` est-il classé, et le commit `wip()` sort-il ? | **implémenteur** | avant PR | test dans `test-dispatch-lib.sh` + corps de la PR | **l'armement de U1** |
| **V3b** | la PR brouillon sort-elle, en vrai ? | **opérateur** | au premier `error_max_turns` réel (= sonde **S2**) | **commentaire sur #2496** | l'armement — halte 2, désarmer |

**V1a / V1b — quel préfixe survit, et la scission est une rectification.** Le
plan v1 posait V1 « bloquant, préalable à U1 », ce qui était incohérent : la
seule mesure concluante est un dispatch réel portant la variable, donc elle exige
U1 déjà écrit. La question se scinde le long de ce qui est mesurable quand.
**V1a (statique, implémenteur, avant PR)** : lire `scrub_mika_env_vars` et les
sites de seed pour établir que `PILOT_*` traverse là où `MIKA_*` est retiré —
précédent contradictoire dans le fichier même, `PILOT_LOG_DIR` (non préfixé,
mika#2249) contre `MIKA_PILOT_SANDBOX` (`:183`). **V1b (vivant, opérateur, au
premier dispatch)** : poser `PILOT_MAX_TURNS=<valeur ≠ 120>` sur
l'environnement du **service** et lire la ligne `pilot_budget_armed` — **U2 est
l'instrument de V1b**, `source=env` étant la preuve directe que la variable a
traversé. **Ce qu'un rouge bloque est la revendication, pas le correctif :** par
mika#2249, une divergence de préfixe ne peut acheter que de l'inertie (le défaut
120 s'applique quand même, et 120 < 200 reste un gain), jamais un faux positif.
**Halte V1b :** si `source=env` n'apparaît jamais, **AC4 est rétracté** du corps
de la PR et du `CLAUDE.md` — un rollback annoncé et inatteignable pendant un
incident est exactement la garantie fausse que mika#2304 nomme, et la retirer
vaut mieux que la laisser debout.

**V2 — la distribution des tours** (§ 5.1). Requête sur
`~/.mika/data/mika.db`, que le sandbox de dispatch ne monte pas : c'est donc un
geste **opérateur sur l'hôte**, pas une mesure que l'implémenteur peut fournir.
**Moment : avant merge.** **Artefact : un commentaire sur #2496** portant le
compte, le p95 des runs aboutis et la valeur retenue. **Halte (déjà écrite au
§ 5.1)** : p95 > 120 ⇒ le défaut est relevé avec la distribution, jamais la règle
appliquée en aveugle. **Si l'opérateur ne fournit pas la mesure, U1 est livré
désarmé** (`PILOT_MAX_TURNS=0` par défaut) et la raison est écrite dans le corps
de la PR — c'est l'item 1 du DoD, et c'est ce qui rend ce bloquant tenable sans
bloquer la PR indéfiniment.

**V3a / V3b — la chaîne de récupération, et la scission suit ce qui est
testable hors réseau.** La promesse ne porte que sur `error_max_turns` (§ 5.2).
**V3a (implémenteur, avant PR, déterministe)** : le harnais fabrique un stdout
`status: terminated / subtype: error_max_turns` et un worktree sale, puis vérifie
que `_halt_family` rend `budget_exhausted|deterministic` **et** que
`_rescue_dirty_worktree` produit son commit `wip()`. Le harnais exerce déjà cette
fonction sur des dépôts git temporaires (`test_auto_rescue_*`), donc la moitié
commit est vérifiable sans réseau et devient un **test**, pas une promesse.
**V3b (opérateur)** : l'ouverture de la PR brouillon passe par `gh` et n'est pas
testable hors réseau — c'est la sonde **S2**, avec sa halte 2 déjà écrite
(désarmer avant tout diagnostic). **Artefact : commentaire sur #2496.**

**V4 — harnais shell** (`test-dispatch-lib.sh`) : les trois sites portent le
drapeau ; la valeur passée est la valeur résolue ; `PILOT_MAX_TURNS=0` **omet**
le drapeau (contrôle négatif du rollback) ; une valeur illisible retombe au
défaut **et** émet le WARN.

Le **contrôle négatif du scan U3 est fabriqué sur les formes d'appel réellement
présentes dans le fichier**, pas sur une forme inventée — un scan vert par
vacuité sur les formes qu'il ne reconnaît pas est indistinguable d'un scan qui
couvre (mika#2205). Cinq fixtures, chacune **vue rouge** ou **vue verte** :

| fixture | forme | attendu |
|---|---|---|
| N1 | mono-ligne, celle du site `3060` (`_pilot_log_dir; _run_pilot_sandboxed claude-pilot --verbose …`), sans le drapeau | **rouge** |
| N2 | multi-lignes avec continuations `\`, celle des sites `5912`/`6031`, sans le drapeau | **rouge** — atteste le recollement du terme 1 |
| N3 | multi-lignes, drapeau présent mais **sur une ligne de continuation** | **vert** — le miroir de N2 ; sans lui, « le scan lit l'invocation » ne se distingue pas de « le scan rougit sur tout multi-ligne » |
| N4 | appel direct `claude-pilot --verbose …` **sans** `_run_pilot_sandboxed`, sans le drapeau | **rouge** — atteste la disjonction du terme 3 |
| N5 | bonne foi : un commentaire mentionnant `claude-pilot`, un `--ro-bind-try "$HOME/.local/bin/claude-pilot"`, un `--relay-config …/claude-pilot.json` | **vert** — atteste les exclusions 2/3, sans quoi le scan serait rouge en permanence et donc désarmé |

Plus l'assertion de cardinalité du terme 4 (**exactement 3 candidats** dans
`dispatch-lib.sh`), qui est ce qui fait rougir un prédicat devenu trop étroit —
la seule forme de panne qu'aucune des cinq fixtures ne peut voir.

**V5 — Rust** : `pilot_cost_overrun` émis au-dessus du seuil et **pas** en
dessous (contrôle négatif) ; absence de `cost_usd` ⇒ aucune ligne (jamais `0`,
mika#2331) ; scan SOLE WRITER à allowlist vide.

**Ce qui n'est PAS testable ici, et c'est écrit plutôt que découvert :** « le
budget tue avant 120 tours » est exécuté par le SDK, dans un autre processus,
avec un vrai fournisseur. Le contrat **côté mika** est *le drapeau atteint
l'argv avec la bonne valeur* — V4 l'atteste déterministiquement. La moitié
comportementale est la sonde S2.

## Fire-Disposition

Ce plan livre deux détecteurs : le **scan de source U3** (tout site de lancement
porte `--max-turns`) et le **scan SOLE WRITER U4** (`pilot_cost_overrun` n'a
qu'un écrivain).

**Option retenue : (a) — exception nommée en allowlist, allowlist livrée VIDE.**

- **U3.** Violations existantes à l'ouverture : **trois** (`dispatch-lib.sh:3060`,
  `5912`, `6031`). Les trois sont **fermées par ce même PR** (U1 les arme), donc
  le compte de violations au merge est **zéro** et l'allowlist est vide. Quand
  le scan tire, **la résolution est d'armer le site, jamais de l'allowlister** —
  doctrine mika#2201 (*on déclare, on n'allowliste pas*), écrite dans le message
  d'échec du scan.
- **U4.** Violations existantes : **zéro** — le nom `pilot_cost_overrun` est créé
  par ce PR et n'a qu'un site d'émission. Allowlist vide dès l'origine.

Une allowlist née vide n'est pas un formalisme : c'est l'absence de case où
déposer le prochain manquement. Les deux scans portent leur **contrôle négatif**
(V4, V5) — sans lui, un scan vacant se lit exactement comme un scan qui couvre.

**Aucun détecteur n'est livré désarmé** (option b) : les deux sont verts au
merge et n'ont rien à mettre en quarantaine. **Aucune halte-et-remontée**
(option c) n'est requise pour les détecteurs — les haltes bloquantes de ce plan
(item 0 du DoD, V1b, V2, V3a/V3b) portent sur la **procédure de divergence**, sur
la **valeur** du budget et sur la chaîne de récupération, jamais sur les scans.

## Definition of Done

- [ ] **Item 0 — la divergence est portée dans le corps de #2496, et ratifiée
      (BLOQUANT, préalable à tout le reste).** Ce plan **réfute le correctif du
      ticket** (« exposer le compte de tours en temps réel » : refusé par A1 et
      R3, non réalisable dans ce dépôt) **et ne livre pas son DoD littéral**
      (« compte lisible en temps réel ; kille avant 120 tours, testé » : le
      premier terme est abandonné, le second est armé à la source et non côté
      moteur). La convention (mika#2169, mika#2158 ; précédent immédiat **#2493**,
      dont l'item 0 est de la même forme) exige trois gestes, dans cet ordre :
  - [ ] **encadré daté rédigé dans le corps de #2496**, portant R1 (le plafond
        natif existe et personne ne l'arme), R2 (le frein dollars est un `pass`),
        R3 (aucune surface mika ne porte le compte de tours), et **remplaçant le
        DoD littéral par AC1–AC8** ;
  - [ ] **commentaire d'avis d'édition** sur #2496, disant que le corps a été
        rectifié, par qui et pourquoi — l'édition silencieuse d'un corps de
        ticket est ce que cette convention existe pour interdire ;
  - [ ] **ratification opérateur** obtenue sur ce commentaire. L'en-tête de ce
        plan porte déjà « ne PAS mettre `ready` sans ratification » — mais cette
        réserve vient de la carte de l'umbrella #2491, **pas** d'une
        rectification de #2496, et ne peut donc pas en tenir lieu.
- [ ] V1a, V2, V3a exécutées et rapportées **avant** l'armement, chacune par son
      exécutant et dans son artefact (tableau du § 11) ; si V2 ou V3a rougit, U1
      est livré **désarmé** (`PILOT_MAX_TURNS=0` par défaut) et la raison est
      écrite dans le corps de la PR. V1b et V3b sont rapportées **après
      déploiement** en commentaire sur #2496 ; un V1b rouge **rétracte AC4** au
      lieu de le laisser debout.
- [ ] `--max-turns` passé aux **trois** sites de lancement, valeur résolue par
      `_pilot_max_turns`, trois paliers maison, rollback par variable.
- [ ] `pilot_budget_armed` émis à chaque lancement, avec `source` et
      `cost_bound=absent_upstream`.
- [ ] Scan de source U3 vert, allowlist vide, motif figé (les quatre termes du
      § U3), **cardinalité assertée à 3**, et les **cinq fixtures N1–N5** du § 11
      exécutées — les trois rouges **vues rouges**, les deux vertes vues vertes.
- [ ] `pilot_cost_overrun` émis + row `audit_events`, SOLE WRITER épinglé,
      contrôle négatif.
- [ ] `CLAUDE.md` racine et `crates/mika-agent/CLAUDE.md` portent les knobs, la
      ligne U2, **l'absence amont du frein dollars** et les quatre sondes.
- [ ] Deux tickets de suivi ouverts sur `senara-solutions/claude-pilot` : le
      `pass` de `_sdk_guardrail_kwargs`, et le compte de tours par frontière.
- [ ] Le corps de la PR dit explicitement que **le second terme de la règle
      (40 USD) n'est pas appliqué par ce PR**, avec l'arithmétique du § 4.
- [ ] `cargo test`, `cargo clippy`, `make verify-bundled-skills` verts.

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de son DoD, du § 3 et du contrat de vérification.

**AC1 — Le budget de tours est armé à la source, aux trois sites.** Un dispatch
lance `claude-pilot` avec `--max-turns <valeur résolue>` aux trois sites de
`dispatch-lib.sh`. Vérifié par assertion sur l'argv (V4), pas par relecture.

**AC2 — Le budget en vigueur est observable sans lire le code.** Chaque
lancement émet `pilot_budget_armed` avec la valeur, sa provenance
(`default`/`env`) et l'état du frein dollars. Son absence est documentée comme
« binaire antérieur », jamais comme « pas de budget ».

**AC3 — Un emballement est coupé, et son travail survit.** Une session
atteignant le plafond se termine en `status: terminated / subtype:
error_max_turns`, est classée `budget_exhausted|deterministic` par
`_halt_family`, et sa chaîne de récupération produit un commit et une PR
brouillon. **Portée : `error_max_turns` seul** — `error_max_budget_usd` est un
sous-type sans émetteur (R2), donc rien ici ne le couvre et rien ne doit se lire
comme s'il le couvrait. **V3a établit le commit avant l'armement (déterministe,
hors réseau) ; V3b = S2 confirme la PR brouillon en production.**

**AC4 — Le rollback est une variable.** `PILOT_MAX_TURNS=0` omet le drapeau et
restaure le comportement antérieur à l'octet près, sans redéploiement. Contrôle
négatif dédié (V4). **Cet AC est conditionné à V1b** : il n'est revendiqué —
dans le corps de la PR et dans le `CLAUDE.md` — qu'une fois `source=env` observé
sur une ligne `pilot_budget_armed` réelle. Sinon il est **rétracté**, le défaut
120 restant appliqué (mika#2249 : la divergence de préfixe n'achète que de
l'inertie).

**AC5 — Le régime nominal n'est pas tronqué.** La valeur par défaut est dérivée
de la distribution mesurée des tours des runs aboutis (V2), pas de la règle
seule. La part de dispatches coupés reste marginale (S3), sous peine de halte 3.

**AC6 — Le dépassement de coût est compté.** Tout dispatch dont `cost_usd`
terminal dépasse `PILOT_COST_ALERT_USD` produit une ligne WARN et une row
`audit_events`, écrivain unique. Cette mesure est la précondition explicite du
ticket cpp.

**AC7 — Rien n'annonce un budget dollars qui n'existe pas.** `--max-budget`
n'est passé à aucun site ; `pilot_budget_armed` porte `cost_bound=absent_upstream` ;
la PR et le `CLAUDE.md` disent que le second terme de la règle n'est pas
appliqué. Aucune surface de ce PR ne peut se lire comme « 40 USD est appliqué ».

**AC8 — Les scans ne peuvent pas devenir vacants.** U3 et U4 portent chacun un
contrôle négatif exécuté et **vu rouge**, et leurs allowlists sont vides. Pour
U3, le contrôle négatif est fabriqué sur les **formes d'appel réellement
présentes** dans `dispatch-lib.sh` — les cinq fixtures N1–N5 du § 11, dont N2
(continuations) et N4 (appel sans le wrapper), plus l'assertion de cardinalité
à 3. Une forme qu'aucune fixture n'exerce est une forme sur laquelle le scan
peut être vert par vacuité (mika#2205), et un motif non figé est le moyen le
plus court d'y arriver.

## Revision history

- **rev 2 (2026-09-23)** — révision sur les cinq findings de la première passe
  architecte. La substance architecturale n'a pas bougé (refus de `--max-budget`,
  Fire-Disposition (a), arithmétique du § 4, AC dérivés) ; la révision porte sur
  la **procédure de divergence** et le **contrat des vérifications**, ce que les
  findings disaient eux-mêmes.
  - **F1 (bloquant) — addressed.** Ajout de l'**item 0 au DoD** : encadré daté
    dans le corps de #2496 (R1/R2/R3, DoD littéral remplacé par AC1–AC8),
    commentaire d'avis d'édition, ratification opérateur — les trois dans cet
    ordre et préalables à tout le reste. L'en-tête du plan dit désormais
    explicitement que sa réserve « ne PAS mettre `ready` » vient de la carte de
    l'umbrella #2491 et **ne tient pas lieu** de rectification de #2496.
    Convention mika#2169/#2158, précédent #2493.
  - **F2 (bloquant) — addressed.** Chaque V porte son **exécutant, son moment et
    son artefact**, dans un tableau au § 11, plus la colonne « ce qu'un rouge
    bloque ». Deux scissions de fond plutôt qu'une simple annotation : **V1 →
    V1a** (statique, implémenteur, avant PR) **/ V1b** (vivant, opérateur, premier
    dispatch, **instrument = la ligne U2 `source=env`**), parce que « V1 bloquant
    préalable à U1 » était incohérent — la seule mesure concluante exige U1 déjà
    écrit ; et **V3 → V3a** (déterministe dans le harnais, hors réseau) **/ V3b**
    (= sonde S2, la PR brouillon passant par `gh`). Un V1b rouge **rétracte AC4**
    au lieu de laisser debout un rollback inatteignable (mika#2304).
    Unresolved-Decision Gate (mika#1244).
  - **F3 (sharpening) — addressed.** Le motif du scan U3 est **figé en quatre
    termes** au § U3 (invocation logique après recollement des continuations `\` ;
    commentaires retirés ; `claude-pilot` en position de commande, hors chemins et
    hors `.json` ; disjonction avec `_run_pilot_sandboxed`), avec la mesure qui le
    justifie (47 occurrences de la chaîne, **3** lancements, 2 des 3 écrits sur
    plusieurs lignes — donc un scan à la ligne en manquerait deux). Le contrôle
    négatif est fabriqué sur les **formes réelles** : cinq fixtures N1–N5 (dont N2
    continuations, N3 miroir vert, N4 appel sans wrapper, N5 bonne foi) plus une
    **assertion de cardinalité à 3**, seule forme de panne qu'aucune fixture ne
    voit. mika#2205 appliqué au scan de ce plan, review-guide § KISS.
  - **F4 (sharpening) — addressed.** Le § 5.2 dit maintenant que la promesse de
    `_halt_family` et V3 **ne portent que sur `error_max_turns`**, R2 établissant
    que `error_max_budget_usd` n'a aucun émetteur. Propagé à **AC3** (portée
    explicite) et à **S2** (l'absence de `error_max_budget_usd` n'est pas un
    signal). Cohérence interne R2 ↔ § 5.2, discipline mika#2304.
  - **F5 (sharpening) — addressed.** Une ligne au § U4 : 40 est **la règle**, posé
    d'office parce qu'un seuil d'alerte **ne coupe rien** (un faux positif coûte
    une ligne WARN, pas un run tronqué) — ce qui est l'asymétrie qui autorise à ne
    pas exiger V2 ici — et **révisé sur la première distribution mesurée par U4**,
    cette révision étant le premier livrable du ticket de suivi cpp. Doctrine du
    § 5.1, mika#2293.
