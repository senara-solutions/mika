# Plan — mika#2149 : les motifs de halte de claude-pilot reçoivent une classe en aval, et le vocabulaire cesse de dériver en silence

- **Ticket** : senara-solutions/mika#2149
- **Type** : fix (substrat, p2 — ralentit la boucle)
- **Branche** : `fix/2149/dispatch-lib-le-vocabulaire-des-motifs`
- **Date** : 2026-09-21
- **Lignée** : mika#1772 (les deux populations de `terminated`), cpp#54
  (`api_error_status`), cpp#119 (`rate_limited`), cpp#145 (`awaiting_tool` /
  `awaiting_model`), cpp#168 (`watchdog_error`), cpp#185 (`prompt_cache_dead`),
  cpp#187 (`transport_message_too_large`)

---

## Contexte

### Le fait, re-mesuré le 2026-09-21

Le ticket a été déposé le 2026-09-03 à 01:25Z, six heures avant le merge de
cpp#145. Ses trois constats tiennent ; leurs nombres ont bougé. Chaque écart est
daté ici parce qu'il change la taille du remède, pas sa nature.

**P1 — le vocabulaire compte huit valeurs, pas six.** Depuis le ticket,
`GuardrailAbortReason.guardrail` (`claude-pilot/src/claude_pilot/types.py:287-295`)
a reçu `watchdog_error` (cpp#168) et `prompt_cache_dead` (cpp#185 D1). Le
commentaire de `dispatch-lib.sh` — qui en nomme trois — est périmé de **cinq**
valeurs, et il l'est devenu deux fois de plus *pendant que le ticket dormait*.
C'est la démonstration du défaut réel : rien, dans ce dépôt, n'observe la
dérive. Un commentaire qu'aucun test ne lit se périme au rythme exact des
ajouts amont.

**P2 — les lignes citées ont migré.** `:2128 → :3016` (commentaire),
`:2134 → :3022` (lecture opaque), `:2199 → :3099` (routage sur `.status`),
`:2511 → :3412` (rendu prose) ; dans `test-dispatch-lib.sh`, `:3835 → :4110`
(commentaire du bloc mika#1772) et `:3945` (le probe `idle_timeout`). Mesuré sur
`main@2b5456cc`.

**P3 — la condition de réveil est remplie, mais le cas concret n'est pas
arrivé.** cpp#145 est mergé (PR cpp#147, 2026-09-03 07:22Z) et déployé — l'outil
`claude-pilot` est installé en éditable depuis `claude-pilot/src`, vérifié via
`_editable_impl_claude_pilot.pth`. Pourtant, sur les **365 sessions** dont le
stderr est persisté dans `/var/log/claude-pilot/` depuis le 03/09, on compte
**zéro** `awaiting_model` et **zéro** `awaiting_tool`. Ce qui s'observe :
`idle_timeout` (≈15, deux formes : « nothing observed since the session
started » et « Silent for 300s … nobody outstanding »), `stall_detected` (1),
`prompt_cache_dead` (1), `rate_limited` (1 sur tout l'historique). Le choix de
politique que le ticket voulait trancher « sur de vrais cas » n'a toujours pas
de cas. Ce plan le tranche quand même — à la hauteur de ce que l'amont a écrit,
pas plus — et dit ce qui reste à mesurer.

**P4 — `api_error_status` : le contrat cpp#54 n'est toujours câblé nulle
part.** Zéro occurrence dans `mika/skills`, `mika/crates`, `mika/docs`
(re-grep 2026-09-21). L'émetteur, lui, est complet : `agent.py:825` le pose sur
le `ResultMessage` terminal, `agent.py:515` sur la voie d'abandon garde-fou
(cpp#119), et `types.py:241` nomme explicitement « mika-dev dispatch-lib »
comme consommateur.

### Ce que le retry fait aujourd'hui, et pourquoi la classe est un préalable

Tout callback portant `PIPELINE FAILURE:` est rejoué jusqu'à deux fois par
mika-dev (`skills/bundled/self-dev-callback/system_prompt.md:110-118`), **sans
lire le motif**. `awaiting_model`, `idle_timeout`, `error_max_budget_usd` et un
subtype inconnu reçoivent le même traitement : deux relances aveugles. Le
subtype n'existe en aval que comme fragment de prose (`Halt: ${SUBTYPE}`,
`dispatch-lib.sh:3412`).

Le ticket exclut le comportement de retry de son périmètre, et ce plan le
respecte : **rien ne change dans qui relance quoi**. Ce qui change, c'est que
le callback dit désormais, en deux lignes lisibles par une machine, *de quelle
famille* est la halte et *ce qu'une relance peut en attendre*. C'est le
préalable à toute politique future — on ne peut pas gater sur une classe qui
n'existe pas — et c'est ce qui épargne à l'opérateur la réouverture du journal
que le tier 2 désigne.

### Le défaut structurel : une énumération que rien ne vérifie

Un commentaire n'a pas de test. Une table `case` en a un — et surtout, elle a
une branche par défaut. Le remède du point 1 n'est donc pas « corriger le
commentaire » (il se périmera à nouveau au prochain cpp#) mais **remplacer
l'énumération en prose par une table exécutée**, dont la branche `*)` journalise
tout subtype qu'elle ne connaît pas. La prochaine valeur ajoutée en amont
laissera une trace `halt_family.unknown subtype=<x>` dès sa première
occurrence, au lieu de rejoindre silencieusement la prose.

---

## Requirements

- **R-1 (point 1)** — `dispatch-lib.sh:3016` et `test-dispatch-lib.sh:4110`
  cessent d'énumérer en prose et renvoient à la table `_halt_family` comme
  source de vérité aval, elle-même pointant vers `GuardrailAbortReason.guardrail`
  comme source de vérité amont.
- **R-2 (point 2, décision)** — dispatch-lib classe chaque subtype d'une session
  `terminated` en une **famille** et un **indice de relance**, émis dans le
  callback en deux lignes structurées (`Halt class:` / `Retry hint:`), dans les
  deux modes de `_classify_terminated_session` (full et banner). Aucun
  changement du comportement de retry.
- **R-3 (point 3, décision)** — `api_error_status` est branché : lu à côté de
  `.subtype`, rendu dans la ligne `Halt:` quand présent. Le commentaire cpp#54
  devient vrai ; aucune modification côté claude-pilot.
- **R-4 (dérive)** — un subtype hors table est classé `unknown`/`investigate`
  **et** journalisé sur stderr (`dispatch-lib: halt_family.unknown
  subtype=<x>`) — visible dans le `.stderr` persisté et dans le tail du callback.
- **R-5 (dérive, garde)** — un test lit le `Literal` de
  `GuardrailAbortReason.guardrail` dans `types.py` quand le dépôt claude-pilot
  est accessible, et exige que chaque valeur ait une famille ≠ `unknown`.
  S'il n'est pas accessible, le test le dit sur une ligne visible et ne
  prétend pas avoir couvert.
- **R-6 (chemin de repli)** — la voie de secours qui gratte `[guardrail] <x>:`
  dans le stderr (quand le JSON n'a pas de subtype) alimente la même table :
  une halte n'est jamais classée `unknown` parce qu'elle est arrivée par le
  mauvais canal.

## Hors périmètre (repris du ticket, précisé)

- Le comportement de retry de dispatch-lib et de `self-dev-callback` : ni gate,
  ni changement du plafond de deux relances, ni lecture des nouvelles lignes
  par le prompt de mika-dev. Condition de réveil pour ce suivant : **n≥3
  haltes d'une même famille** dont l'indice de relance s'est vérifié juste ou
  faux — voir § Contrat de vérification.
- Toute modification dans `claude-pilot/`.
- Les subtypes de `status: error` (`pipeline_incomplete`,
  `blocked_on_operator_input`, `error_during_execution:after_deny`,
  `stream_ended_without_result`, `fatal`) : ils ne passent pas par
  `_classify_terminated_session` et ont déjà leur rendu propre.

---

## Approche / Conception

### C-1 — `_halt_family` : la table, et sa branche par défaut (R-1, R-2, R-4)

Nouvelle fonction dans `dispatch-lib.sh`, à côté de
`_classify_terminated_session`. Entrée : un subtype. Sortie sur stdout :
`<family>|<hint>|<meaning>`. La table est l'énumération ; le commentaire de
tête ne liste rien, il pointe vers `types.py` et vers cette table.

| subtype | family | hint | meaning (une ligne) | source amont |
|---|---|---|---|---|
| `rate_limited` | `quota_throttled` | `transient` | l'API a refusé (429) et le SDK a épuisé son backoff ; la session n'a rien à se reprocher | cpp#119, cpp#133 |
| `awaiting_model` | `model_never_resumed` | `transient` | le modèle n'a jamais rendu le premier jeton du tour suivant ; la session attendait, elle ne tournait pas en rond | cpp#145 |
| `awaiting_tool` | `tool_never_returned` | `investigate` | un outil n'a jamais rendu son résultat ; relancer sans lire lequel le rejoue | cpp#145 |
| `idle_timeout` | `session_silent` | `investigate` | silence réel, personne en attente ; la cause est dans le journal, pas dans la relance | cpp#54, précisé cpp#145 |
| `stall_detected` | `model_unproductive` | `investigate` | N tours sans appel d'outil ; l'état de départ conduit le modèle nulle part | cpp#54 |
| `empty_response` | `model_unproductive` | `investigate` | N réponses vides consécutives | cpp#54 |
| `watchdog_error` | `pilot_bug` | `investigate` | le watchdog lui-même a levé ; c'est un défaut de claude-pilot, pas de la session | cpp#168 |
| `prompt_cache_dead` | `substrate` | `investigate` | le cache de prompt n'est plus lu ; vérifier le relais (mika#2313/#2316) avant toute relance | cpp#185 D1 |
| `error_max_turns` | `budget_exhausted` | `deterministic` | limite SDK atteinte ; du travail a été produit, la chaîne de récupération le porte | agent.py:54 |
| `error_max_budget_usd` | `budget_exhausted` | `deterministic` | idem, en dollars | agent.py:54 |
| `transport_message_too_large` | `transport` | `investigate` | un message NDJSON a dépassé `max_buffer_size` | cpp#187 |
| `*` | `unknown` | `investigate` | subtype hors table — voir R-4 | — |

**Les trois indices, définis pour ne pas être confondus avec une décision :**

- `transient` — la cause est *hors* de la session (quota, modèle muet en
  amont). Une relance a des chances raisonnables de ne pas la revoir.
- `deterministic` — la cause est *dans* ce que la session a fait. Une relance
  depuis le même état la reproduit ; ce qui compte est ce qu'elle a laissé.
- `investigate` — une relance ne renseigne rien tant que la cause n'a pas été
  lue. Ni promesse ni interdiction : un pointeur vers le journal.

Ces indices sont écrits **à la hauteur de ce que les commentaires amont
affirment** (cités colonne de droite) et de rien d'autre. Là où l'amont ne
tranche pas — `stall_detected`, `empty_response`, `idle_timeout` — l'indice est
`investigate`, pas une conjecture de ce plan. La ligne `Retry hint:` est une
**annotation** que mika-dev ne lit pas encore ; elle deviendra une entrée de
politique quand le § Contrat de vérification aura des mesures.

Pourquoi un `case` et non une `declare -A` : le fichier n'en utilise aucune
aujourd'hui (grep : 0), et un `case` se lit comme la table qu'il est —
une ligne par valeur, la branche `*)` en dernier, rien à initialiser avant
l'appel.

### C-2 — Deux lignes dans le callback (R-2)

`_classify_terminated_session` appelle `_halt_family "$SUBTYPE"` et émet, après
la ligne `Halt:`, dans les deux modes :

```
Halt: awaiting_model (HTTP 529) — model-wait ceiling 900s exceeded …
Halt class: model_never_resumed — le modèle n'a jamais rendu le premier jeton du tour suivant ; …
Retry hint: transient — la cause est hors de la session ; une relance a des chances raisonnables de ne pas la revoir
```

Les deux préfixes `Halt class:` et `Retry hint:` sont **stables** (même
contrat que `Outcome:` et `RECOVERY_PENDING:` — une ligne, un préfixe, un
`grep -m1` suffit). Le mode `banner` les porte aussi : une session qui a laissé
du travail mérite autant de savoir pourquoi elle est morte.

### C-3 — `api_error_status` branché (R-3)

À `dispatch-lib.sh:3022-3023`, une lecture de plus :

```bash
API_ERROR_STATUS=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.api_error_status // empty' 2>/dev/null)
```

Dans `_classify_terminated_session`, la ligne `Halt:` devient
`Halt: ${SUBTYPE}${API_ERROR_STATUS:+ (HTTP ${API_ERROR_STATUS})}${TERMINATION_REASON:+ — …}`.
Rien de plus : la famille reste tirée du subtype. `api_error_status` est un
**qualificatif**, pas un second axe — cpp#119 garantit qu'il n'est posé que sur
`rate_limited` côté garde-fou, et cpp#54 qu'il est absent (`exclude_none`)
quand la session s'est terminée sans erreur API. Le commentaire de
`types.py:241` (« letting downstream (mika-dev dispatch-lib) classify ») est
vrai à la fin de cette phase ; il n'y a rien à retirer.

### C-4 — Le chemin de repli alimente la même table (R-6)

Quand `SUBTYPE` est vide, la voie existante gratte `[guardrail] <x>: …` dans le
stderr. Elle extrait désormais `<x>` (`sed -n 's/.*\[guardrail\] \([a-z_]*\):.*/\1/p'`,
après le strip ANSI déjà en place — les lignes réelles portent `[38;5;208m` et
`[1m` autour du nom) et le passe à `_halt_family`. La ligne `Halt:` garde son
libellé actuel (`Halt: [guardrail] …`, texte brut du stderr) ; seules les deux
lignes de classe s'ajoutent. Sans ligne `[guardrail]`, la classe est `unknown`
et le message actuel « cause not recorded » reste tel quel.

### C-5 — La garde de dérive (R-5)

Dans `test-dispatch-lib.sh`, un bloc qui cherche `types.py` à
`${CLAUDE_PILOT_TYPES:-$(git rev-parse --show-toplevel)/../claude-pilot/src/claude_pilot/types.py}`
(le worktree mika vit sous `<meta>/mika` ou `<meta>/.claude/worktrees/<slug>/mika` ;
les deux ont `claude-pilot/` à un ou trois niveaux au-dessus — le test essaie
les deux, puis la variable). S'il le trouve : extrait le bloc
`guardrail: Literal[ … ]` par `sed`, et pour chaque valeur entre guillemets,
`assert_not_contains "drift: $v" "unknown" "$(_halt_family "$v" | cut -d'|' -f1)"` (helper existant, `test-dispatch-lib.sh:73`). S'il ne le
trouve pas : `echo "SKIP: drift guard — claude-pilot types.py not reachable
(set CLAUDE_PILOT_TYPES)"` et **aucune assertion**, pour que le vert ne mente
pas.

Le test tourne sur le poste de dispatch (où claude-pilot est toujours présent —
c'est lui qu'il dispatche). En CI, il saute et le dit.

---

## Fire-Disposition

### D-1 — `halt_family.unknown` (C-1, Phase 1)

**Tire quand** : un `status: terminated` porte un subtype absent de la table.
**Fait** : `echo "dispatch-lib: halt_family.unknown subtype=<x>" >&2` — atterrit
dans `$STDERR_FILE` (donc dans le tail de 10 Ko du callback) et dans le
`.stderr` persisté. Classe `unknown`, indice `investigate`. **Ne change rien**
au routage ni au retry. **Contrôle négatif** : un subtype connu n'émet pas la
ligne (T2).

### D-2 — Garde de dérive (C-5, Phase 3)

**Tire quand** : `types.py` porte une valeur que la table ne connaît pas.
**Fait** : rouge dans `test-dispatch-lib.sh`, en nommant la valeur. **Ne tire
pas** en CI sans claude-pilot : ligne `SKIP:` visible.

### D-3 — `(HTTP n)` sur la ligne `Halt:` (C-3, Phase 1)

**Tire quand** : `.api_error_status` est présent dans le JSON. **Fait** : le
qualificatif s'insère. **Contrôle négatif** : absent → aucun `(HTTP` dans la
sortie (T3).

### Ce que cette section ne couvre pas

Aucun mécanisme de ce plan ne modifie une décision. Les trois dispositions
ci-dessus **écrivent** ; aucune ne **branche**.

---

## Phases d'implémentation

### Phase 1 — La table et les deux lignes (C-1, C-2, C-3, R-1..R-4)

1. Ajouter `_halt_family` avant `_classify_terminated_session`.
2. Lire `API_ERROR_STATUS` à `:3022`.
3. Réécrire le commentaire `:3013-3021` : plus d'énumération ; renvoi à
   `types.py` (`GuardrailAbortReason.guardrail`) et à `_halt_family`.
4. Dans `_classify_terminated_session` : qualificatif HTTP, appel à
   `_halt_family`, deux lignes dans les deux modes.

### Phase 2 — Le chemin de repli (C-4, R-6)

5. Extraire le nom après `[guardrail] ` sur la ligne grattée, classer, émettre.

### Phase 3 — Tests (T1..T6, D-2)

6. Étendre `_classify_probe` d'un sixième argument `api_error_status`
   (`test-dispatch-lib.sh:3920`), exporté comme `API_ERROR_STATUS`.
7. Réécrire le commentaire `:4108-4116` (renvoi, plus d'énumération).
8. Ajouter les sondes T1..T6 dans le bloc mika#1772 existant.

### Phase 4 — Documentation

9. Une entrée dans `docs/solutions/best-practices/` : *une énumération en
   commentaire se périme au rythme de l'amont ; une table avec branche par
   défaut se signale à la première dérive*. Courte ; le fait mesuré (P1 : deux
   valeurs ajoutées pendant que le ticket dormait) en est la preuve.

Ordre imposé : Phase 1 avant 2 (la voie de repli appelle la table) ; Phase 3
avant tout push (les sondes sont le vert). Une seule PR.

---

## Contrat de vérification

**Sondes (`bash skills/bundled/_shared/test-dispatch-lib.sh`, bloc mika#1772) :**

- **T1** — table-driven : pour chacun des 11 subtypes nommés, `_classify_probe
  '' 2 <subtype> 'x'` contient `Halt class: <family>` et `Retry hint: <hint>`.
- **T2** — `_classify_probe '' 2 foo_bar 'x'` contient `Halt class: unknown`
  et `Retry hint: investigate` ; **stderr** contient `halt_family.unknown
  subtype=foo_bar`. Contrôle négatif : `idle_timeout` → stderr ne contient
  **pas** `halt_family.unknown`.
- **T3** — `rate_limited` + `api_error_status=429` → `Halt: rate_limited (HTTP
  429)`. Contrôle négatif : sans statut → sortie sans `(HTTP`.
- **T4** — repli : `SUBTYPE=''`, stderr `[38;5;208m[guardrail][0m
  [1mawaiting_model[0m: …` → `Halt class: model_never_resumed` (le strip ANSI
  précède l'extraction). Contrôle négatif : stderr sans `[guardrail]` →
  `Halt class: unknown` et « cause not recorded » conservé.
- **T5** — mode `banner` porte les deux lignes (`awaiting_tool` →
  `tool_never_returned`).
- **T6** — garde de dérive (C-5) : rouge si une valeur du `Literal` est
  `unknown` ; `SKIP:` visible sinon. **Rouge-avant** : exécuter T6 avec la
  table amputée de `prompt_cache_dead` et voir le rouge nommer
  `prompt_cache_dead` avant de restaurer.
- **Rouge-avant global** : T1 et T2 doivent être rouges sur `main@2b5456cc`
  (pas de ligne `Halt class:`) et verts après Phase 1 — terme par terme.

**Sondes post-déploiement, et leurs haltes :**

- Sur la prochaine session `terminated` réelle (idle_timeout est la plus
  fréquente, ≈1/24 sessions) : le callback dans mika-dev porte `Halt class:`
  et `Retry hint:`. Sans les deux lignes → le binaire/skill déployé n'est pas
  celui de la PR (`feedback_mika_skills_update_noop_verify_prompt_by_diff`) :
  `diff` le fichier résolu en prod vs le dépôt avant de conclure.
- **Condition de réveil du suivant (politique de retry)** : **n≥3 haltes
  d'une même famille** dont on peut dire, journal en main, si la relance
  aveugle a servi ou reproduit. À ce moment le ticket de gate se dépose avec
  ces trois cas comme évidence, et `self-dev-callback` lit `Retry hint:`.
  Avant, pas de gate.

---

## Definition of Done

- [ ] `_halt_family` en place, 11 valeurs + défaut, commentaire de tête sans
  énumération (renvoi `types.py` + table).
- [ ] `API_ERROR_STATUS` lu ; `(HTTP n)` rendu quand présent.
- [ ] `Halt class:` / `Retry hint:` dans les deux modes, y compris via repli.
- [ ] `halt_family.unknown` sur stderr pour tout subtype hors table.
- [ ] Commentaires `:3013-3021` et test `:4108-4116` sans énumération périmable.
- [ ] T1..T6 verts ; rouge-avant constaté sur T1, T2, T6.
- [ ] Aucune ligne modifiée dans `self-dev-callback/system_prompt.md` ni dans
  `claude-pilot/` (diff de PR à vérifier).
- [ ] Entrée `docs/solutions/best-practices/` commise.

## Acceptance criteria

- **AC1 (point 1)** — Sur la branche, `grep -n 'stall_detected / empty_response / idle_timeout' skills/bundled/_shared/dispatch-lib.sh skills/bundled/_shared/test-dispatch-lib.sh` rend zéro ligne ; les deux emplacements renvoient à `GuardrailAbortReason.guardrail` et à `_halt_family`.
- **AC2 (point 2)** — Pour chacune des 8 valeurs de `GuardrailAbortReason.guardrail`, des 2 valeurs de `SDK_TERMINATION_SUBTYPES` et de `transport_message_too_large`, `_classify_terminated_session` émet `Halt class: <family ≠ unknown>` et `Retry hint: <transient|deterministic|investigate>` conformes à la table C-1, en mode full et banner (T1, T5).
- **AC3 (point 2, dérive)** — Un subtype absent de la table produit `Halt class: unknown`, `Retry hint: investigate` et la ligne stderr `dispatch-lib: halt_family.unknown subtype=<x>` ; un subtype connu ne produit pas cette ligne (T2, contrôle négatif).
- **AC4 (point 3)** — `api_error_status` est lu depuis le JSON et rendu `(HTTP <n>)` sur la ligne `Halt:` quand présent, absent sinon (T3, deux contrôles). Zéro modification dans `claude-pilot/`.
- **AC5 (repli)** — Une halte connue seulement par la ligne `[guardrail]` du stderr (ANSI compris) reçoit la même classe que si le JSON l'avait portée (T4).
- **AC6 (garde)** — `test-dispatch-lib.sh` lit le `Literal` de `types.py` quand il est accessible et échoue en nommant toute valeur non classée ; sinon imprime `SKIP:` et n'affirme rien (T6, rouge-avant constaté).
- **AC7 (périmètre)** — Le diff de la PR ne touche ni `self-dev-callback/system_prompt.md` ni aucun fichier hors `skills/bundled/_shared/{dispatch-lib,test-dispatch-lib}.sh` et `docs/`.

## Revision history

- 2026-09-21 — v1 (/ce:plan, orchestrateur). Prémisses re-mesurées P1–P4.
