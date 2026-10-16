# fix(loop-substrate) — vol 1 : un kill qui n'a rien produit se compte par son nom, et le callback cesse de nommer une cause réfutée

- **Ticket :** senara-solutions/mika#2029 (`bug`, `p1-important`, `ready`, `dispatch:loop`, jalon « Substrat de boucle »)
- **Branche :** `fix/2029/loop-substrate-the-pilot-stall-that`
- **Date :** 2026-10-15
- **Ticket frère :** senara-solutions/claude-pilot#177 — **CLOSED/COMPLETED le 2026-09-11**
- **Lignée :** mika#1772 (le rapport du kill) → mika#2035 (les AC) → mika#2121 (le contrat total `PR:`/`NO_PR:`) → mika#2149 (`_halt_family`) → cpp#177 (la cause, côté pilote, réparée) → **ce ticket** (ce qui reste côté `mika`)
- **Remplace :** `2026-09-11-001-fix-2029-pilot-idle-timeout-kill-substrate-owner-plan.md`, retiré par `a136ea26` pour un re-groom depuis `main`. Les trois livrables de septembre sont re-mesurés ci-dessous ; **deux** survivent à la mesure dans ce vol, le troisième change de justification et part en vol 2.

---

## Contexte

### Ce que le re-groom du 2026-10-15 a mesuré

Le plan de septembre a été retiré parce que son substrat avait bougé. Les six
mesures ci-dessous ont été refaites aujourd'hui, contre l'état de `main` et
contre la machine vivante.

**M1 — La cause est côté pilote, et elle est réparée.** cpp#177 est
`CLOSED`/`COMPLETED` au 2026-09-11T06:56:44Z, titre : *« `content_block_stop`
referme la fenêtre d'attente-modèle que le tool result du même bloc vient
d'ouvrir »*. C'est la réponse à l'exigence Prime du commentaire 18/18
(« localiser la cause avant implémentation ») : **le stall vivait dans
claude-pilot, pas dans le crate `mika`**. Ce ticket ne porte donc plus une
cause — il porte ce que `mika` doit encore savoir dire d'un kill.

**M2 — La classe s'est raréfiée, et ce n'est pas une extinction démontrée.**
Kills par garde-fou sur les 578 sessions des 30 derniers jours
(`/var/log/claude-pilot/*.stderr`, motif désanitisé des codes ANSI) :

| sous-type | occurrences / 30 j |
|---|---|
| `error_max_turns` | 30 |
| `stall_detected` | 6 |
| `idle_timeout` | 5 |
| `prompt_cache_dead` | 1 |

Les six fichiers portant `idle_timeout` sont tous datés du **20 au 27
septembre**. En octobre : **0 sur 99 sessions**. Mais le taux de fin septembre
est de 6/489 ≈ 1,2 %, et à ce taux l'espérance sur 99 sessions vaut 1,2 —
observer zéro a une probabilité d'environ 0,30 sous l'hypothèse d'un taux
**inchangé**. **L'extinction n'est pas établie ; seule la crise de septembre
l'est.** Le plan ne s'appuie donc nulle part sur « la classe a disparu ».

**M3 — Le trou du contrat total est toujours là.** mika#2121 a posé le contrat :
*« every delivered callback carries exactly one `PR:` or `NO_PR: <reason>`
line »* (`dispatch-lib.sh:2317`). Le chemin `_terminated_empty`
(`dispatch-lib.sh:4722`) appelle `_classify_terminated_session` et **saute
`_post_flight_recovery`**, où vivent les deux sites qui posent la ligne
(`:2516`, `:2519`). `_classify_terminated_session` n'en pose aucune : sa sortie
(`:5257-5268`) porte `Halt:`, `Halt class:`, `Retry hint:` et `Outcome:` — et
rien d'autre. Le reaper (`task_engine/engine.rs:4337-4348`) ne trouve donc pas
de ligne à parser et écrit le motif générique.

**M4 — Le diagnostic réfuté est toujours livré à mika-dev.** Deux occurrences
vivantes de la chaîne `Anthropic 401`, et une seule troisième occurrence de
`401` qui est légitime :

| ligne | contenu | statut |
|---|---|---|
| `:1082` | `every in-flight pilot went 401 on api.github.com` | **légitime** (GitHub, mika#2056) — ne pas toucher |
| `:2056` | `→ Anthropic 401 / SDK stall → guardrail idle_timeout 300s → pilot dies at Turns:1` | réfuté |
| `:5264` | `…the note above _run_pilot_sandboxed on the Anthropic 401 / SDK-stall chain that ends in exactly this shape.` | réfuté, **et livré dans le callback** |

Le `:5264` est le plus coûteux des deux : il part dans le callback que mika-dev
reçoit. Depuis cpp#177, ce texte n'est pas seulement non étayé — il est
**réfuté**, et il envoie l'opérateur vers une lignée morte (mika#1901) au lieu
de la cause réelle.

**M5 — Le relais d'egress survit toujours à son code, et c'est n=3.** Mesuré à
l'instant :

```
/tmp/mika-pilot-egress.sock          Oct 13 11:00   (posé par le bind() du relais)
~/.local/bin/mika-pilot-egress-proxy Oct 15 07:46   (posé par make install)
```

Le relais vivant est antérieur de deux jours au code déployé. `make deploy`
n'arrête aucun démon d'egress : `restart` (Makefile:23-27) ne connaît que
`mika-spirit` et `mika-gateway` via OpenRC. Et la sonde de vivacité
`_ensure_pilot_egress_proxy` (`:1319-1321`) retourne `0  # already alive` dès
que quelqu'un écoute, **sans jamais demander quelle version écoute**. Même
forme que le 2026-08-29 et le 2026-09-11 → **n=3**.

**M6 — L'architecture d'egress a changé, et la moitié per-session est saine.**
Le shim côté bac à sable est désormais lancé *par la session* (`--sandbox-tcp`,
avec `trap 'kill $_shim_pid' EXIT`) et meurt avec elle. Seul le **relais
host-side** (`--host-unix`, lancé en `nohup` à `:1344`, commenté « a long-lived
daemon that outlives the dispatch launching it ») est concerné par M5. Le plan
de septembre parlait des « démons » au pluriel ; la mesure n'en laisse qu'un.

### Ce que ce vol porte, et pourquoi il s'arrête là

Les trois livrables de septembre se répartissent sur deux sujets qui ne
partagent ni organe ni mode de défaillance :

- **Ce que le callback dit d'un kill** (livrables 1 et 2) — un seul fichier,
  `dispatch-lib.sh`, deux fonctions voisines sur le même chemin d'exécution.
- **Le substrat d'egress** (livrable 3) — `Makefile` + garde de dispatch, un
  cycle de vie de démon, aucun rapport avec le texte d'un callback.

Le découpage suit cette couture. Le détail du compte est en § *Taille estimée* ;
le motif est que l'estimation n'est pas fiable à ±10 % : les trois vols du
2026-10-14 cités par mika#2693 ont sous-estimé d'un facteur 1,3 à 2,2
(832→1 432, 880→1 160, 443→994). Un total estimé à ~230 lignes contre une borne
à 243 ne laisse aucune marge contre ce facteur. **Vol 1 = les livrables 1 et 2.
Vol 2 = le livrable 3.**

---

## Décisions de grooming

**D1 — Le motif nommé n'est plus justifié par le taux d'une classe, mais par la
totalité du contrat.** Septembre justifiait le livrable 1 par « 8/8 des kills
depuis le 09-04 ». M2 interdit de réemployer cet argument : la classe
`idle_timeout` n'est plus la population dominante, et son taux d'octobre n'est
pas mesurable. La justification qui survit est **structurelle** : mika#2121 a
déclaré le contrat *total*, et `_terminated_empty` en est le dernier trou connu.
Elle ne dépend d'aucun taux, et elle couvre les 30 `error_max_turns`, les 6
`stall_detected` et le `prompt_cache_dead` de M2 au même titre que les
`idle_timeout` — **tout** kill qui n'a rien laissé derrière lui.

**D2 — Le motif porte le sous-type, pas la classe.** `NO_PR:
pilot_terminated_<subtype>`, pas `pilot_terminated` nu. Le reaper écrit
`callback_no_pr_<reason>` (`engine.rs:4348`), donc le sous-type remonte jusque
dans `tasks.result` et un `grep` compte par cause exacte sans ouvrir un log.
`_halt_family` existe déjà et classe les sous-types en familles — mais c'est la
**famille** qui agrège, et agréger ici perdrait précisément ce que mika#2121
reproche au motif générique.

**D3 — Le sous-type est normalisé avant d'être posé, parce que le lecteur est un
regex.** `dispatcher.rs:4935` lit `(?m)^NO_PR:\s+([a-z_]+)`. Un sous-type
portant une majuscule ou un tiret serait **tronqué silencieusement** à son
premier caractère non conforme — `callback_no_pr_pilot_terminated_error` pour un
`error-max-turns`, un motif qui ressemble à une mesure et n'en est pas une. La
normalisation (minuscules, tout caractère hors `[a-z_]` → `_`) rend la troncature
impossible par construction. Sous-type vide → `pilot_terminated_unknown`, jamais
une ligne `NO_PR:` à queue vide.

**D4 — La source du sous-type est `_resolve_halt_subtype`, jamais `$SUBTYPE`.**
C'est l'invariant que mika#2539 a établi et pinné : un résultat `terminated` ne
porte pas toujours de sous-type JSON, et le fallback scrape la ligne
`[guardrail]` du stderr. Un troisième site lisant `$SUBTYPE` brut divergerait de
la bannière sur exactement cette population — le défaut de mika#2539 reproduit
par son propre correctif. `_classify_terminated_session` appelle déjà le
résolveur (`:5208`) et publie `_HALT_SUBTYPE_RESOLVED` ; le site de pose le lit.

**D5 — La ligne est posée par `_set_pr_status_line`, pas par une concaténation.**
Le helper strippe toute ligne `PR:`/`NO_PR:` antérieure avant d'appendre
(`:2358`), ce qui rend le « exactement une » du contrat vrai *par construction*
plutôt que par coïncidence d'ordonnancement. Le chemin `_terminated_empty`
n'écrit rien d'autre après, donc la propriété tient aussi en aval.

**D6 — Le site de pose est le chemin `_terminated_empty`, pas
`_classify_terminated_session`.** La fonction a deux modes et le mode `banner`
sert la population *qui a produit du travail* — celle-là passe par
`_post_flight_recovery`, qui pose déjà sa propre ligne. Poser depuis la fonction
ferait écrire deux fois sur le chemin `banner`, et `_set_pr_status_line` ne
garderait que la dernière : le travail réel serait masqué par un
`pilot_terminated_*`. Le site correct est la branche `if [ "$_terminated_empty"
= "1" ]` de `dispatch_claude_pilot` (`:4722-4724`), où la condition *« HEAD n'a
pas bougé ET l'arbre est propre »* est déjà établie.

**D7 — Le texte réfuté est remplacé, pas supprimé.** Le `:5264` répond à une
vraie question de l'opérateur (« d'où vient ce kill ? »). Le retirer sans rien
mettre laisse un callback muet là où il était faux. Il nomme désormais cpp#177 —
la cause réellement identifiée et réparée — et la ligne `Halt class:` que la
sortie porte déjà deux lignes plus haut.

**D8 — Le détecteur du livrable 2 bite sur `Anthropic 401`, pas sur `401`.** M4
établit que le prédicat large attraperait `:1082`, qui parle d'un 401 GitHub
légitime. Un détecteur qui exige une exception dès son atterrissage pour une
donnée qui n'est pas une violation est un détecteur mal spécifié. Le prédicat
étroit discrimine parfaitement sur l'état mesuré : 2 violations, 0 faux positif.

**D9 — Ce vol ne ferme pas le ticket.** Le livrable 3 reste ouvert sur mika#2029
après ce merge. La PR portera `Refs #2029`, **jamais** `Closes #2029`.

---

## Conception

### Livrable 1 — `NO_PR: pilot_terminated_<subtype>` sur le chemin sans production

**Site :** `dispatch-lib.sh`, branche `_terminated_empty` de
`dispatch_claude_pilot` (actuellement `:4722-4724`).

État actuel :

```sh
if [ "$_terminated_empty" = "1" ]; then
    PILOT_SESSION_TERMINATED=1
    RESULT=$(_classify_terminated_session)
else
```

Après : la branche pose aussi la ligne canonique. Le sous-type vient du
résolveur (D4) — déjà appelé par `_classify_terminated_session`, donc
`_HALT_SUBTYPE_RESOLVED` est publié au moment de la lecture ; la mémoïsation
garantit une seule résolution et une seule réponse (D4).

Une fonction de normalisation (D3), voisine de `_halt_family` pour que les deux
vocabulaires du sous-type se lisent au même endroit :

```sh
# Le lecteur de cette ligne est `RE_NO_PR` dans dispatcher.rs : `(?m)^NO_PR:\s+([a-z_]+)`.
# Un caractère hors [a-z_] ne fait pas échouer le parse — il le TRONQUE, et le
# motif tronqué ressemble à une mesure. La normalisation rend ça impossible.
_no_pr_reason_from_halt_subtype() {
    local s="${1:-}"
    [ -n "$s" ] || { printf '%s' "pilot_terminated_unknown"; return 0; }
    s=$(printf '%s' "$s" | tr '[:upper:]' '[:lower:]' | tr -c 'a-z_' '_')
    printf '%s' "pilot_terminated_${s}"
}
```

`tr -c 'a-z_' '_'` remplace aussi le `\n` final — d'où le `printf '%s'` sans
newline en amont et la capture par `$(...)` qui strippe de toute façon.

### Livrable 2 — le callback cesse de nommer une cause réfutée

**Site A — `:2056`, le commentaire de `_run_pilot_sandboxed`.** La flèche
`→ Anthropic 401 / SDK stall → guardrail idle_timeout 300s → pilot dies at
Turns:1` est une *chaîne causale* affirmée. Le commentaire garde son sujet réel
— pourquoi `CLAUDE_CODE_API_BASE_URL` est posé à côté de `ANTHROPIC_BASE_URL` —
et perd la conséquence réfutée. La phrase qui reste dit ce qui est vrai et
vérifié en canary : sans cette variable, le trafic CC-interne tombe dans les
tunnels CONNECT avec la clé placeholder du bac à sable.

**Site B — `:5264`, la dernière phrase du mode `full`.** Remplacée : elle nomme
cpp#177 (cause identifiée et réparée au 2026-09-11) et renvoie à la ligne
`Halt class:` que la même sortie porte déjà. Le reste du paragraphe — *HEAD n'a
pas bougé, l'arbre est propre, il n'y a pas de plan ni de verdict à chercher* —
est mesuré et ne bouge pas.

**Promotion en `docs/solutions/`** (hors compte de lignes) : un document de la
famille `dev-loop` qui porte ce que le commentaire perd et ce que le re-groom a
mesuré — la lignée 401 réfutée et par quoi, la distribution M2, le mécanisme
cpp#177, et la raison pour laquelle le motif nommé ne se justifie plus par un
taux.

---

## Étapes d'implémentation

1. `_no_pr_reason_from_halt_subtype` + son commentaire, posée à côté de
   `_halt_family` dans `dispatch-lib.sh`.
2. Pose de la ligne dans la branche `_terminated_empty`, via
   `_set_pr_status_line` (D5), sous-type lu dans `_HALT_SUBTYPE_RESOLVED` (D4).
3. Réécriture du site A (`:2056`).
4. Réécriture du site B (`:5264`).
5. Les quatre sondes V1–V4 dans `test-dispatch-lib.sh`, dans la section qui
   couvre déjà `_classify_terminated_session` (autour de `:4686-5211`).
6. `docs/solutions/dev-loop/<...>.md`.
7. `make test-dispatch-lib`, `make lint`, `cargo test`,
   `scripts/verify-pipeline.sh origin/main`.

Ordre de commit : (1–2) le motif nommé ; (3–4) le diagnostic ; (5) les sondes ;
(6) le document. Plancher mergeable : les commits 1, 2 et 5 — le motif nommé
avec ses sondes est utile seul.

---

## Contrat de vérification

| sonde | ce qu'elle établit | comment elle échoue si le livrable manque |
|---|---|---|
| **V1a** | Un kill sans production pose `NO_PR: pilot_terminated_<subtype>` pour chacun des quatre sous-types mesurés en M2 (`idle_timeout`, `error_max_turns`, `stall_detected`, `prompt_cache_dead`) | aucune ligne `NO_PR:` dans la sortie |
| **V1b** | Le motif posé est lisible par le regex du consommateur : la sortie, passée au `(?m)^NO_PR:\s+([a-z_]+)` de `dispatcher.rs`, capture le motif **entier** | le motif est tronqué |
| **V1c** | Un sous-type non conforme (`Error-Max-Turns`) est normalisé, pas tronqué | capture `pilot_terminated_error` |
| **V1d** | Sous-type absent **et** ligne `[guardrail]` absente ⇒ `pilot_terminated_unknown`, jamais une queue vide | ligne `NO_PR: pilot_terminated_` |
| **V1e** | **Exactement une** ligne `NO_PR:` et **zéro** ligne `PR:` dans la sortie délivrée | deux lignes, ou aucune |
| **V1f** | *Contre-vacuité* : le mode `banner` (session ayant produit) **ne** pose **pas** de `pilot_terminated_*` — D6 | la sonde passe aussi sur le chemin avec travail, donc elle ne discrimine rien |
| **V2a** | La chaîne `Anthropic 401` n'apparaît **nulle part** dans `dispatch-lib.sh` | 2 occurrences |
| **V2b** | *Contre-vacuité du prédicat* : `:1082` (`401 on api.github.com`) est **toujours** présent — le détecteur V2a n'a pas été satisfait par une suppression trop large | la ligne GitHub a disparu |
| **V2c** | La sortie du mode `full` nomme `claude-pilot#177` | le callback ne nomme aucune cause |
| **V3** | Le sous-type consommé vient de `_resolve_halt_subtype` et non d'une lecture de `$SUBTYPE` — scan de source sur la branche `_terminated_empty`, même forme que le scan mika#2539 déjà présent à `:9682` | un second résolveur peut diverger de la bannière |
| **V4** | *Hermétisme* : les sondes V1 n'écrivent ni dans `/var/log` ni dans le dépôt, et tournent sans réseau | une sonde qui touche la machine n'est pas rejouable en CI |

V1f et V2b sont les deux contre-vacuités : sans elles, V1 passerait sur un
livrable qui pose la ligne partout, et V2a passerait sur un `sed` qui aurait
emporté la ligne GitHub légitime.

---

## Risques nommés

**R1 — Le motif allonge `tasks.result` d'un motif à rallonge.**
`callback_no_pr_pilot_terminated_error_max_turns` fait 46 caractères. C'est le
prix de D2 : agréger en famille rendrait le motif plus court et lui ferait
perdre ce pour quoi il existe. Aucune contrainte de longueur n'est déclarée
côté `engine.rs:4348` (`format!("callback_no_pr_{r}")`, colonne TEXT).

**R2 — Le reaper pourrait déjà compter ces kills autrement.** `engine.rs:4364`
mentionne un chemin `callback_no_pr_*` vers `tasks.result` (mika#2121 KTD5). Si
un motif existe déjà pour cette population, le livrable 1 le *remplacerait*
plutôt que de combler un trou. M3 mesure l'absence de ligne `NO_PR:` en sortie
de `_classify_terminated_session`, donc le reaper reçoit bien le générique —
mais l'implémentation doit relire `engine.rs:4337-4364` avant d'écrire, et
signaler si la lecture contredit M3.

**R3 — Le texte du site B est lu par une sonde existante.** Des tests portent
déjà sur la sortie du mode `full` (`:4711`, `:5198`). Une réécriture peut en
rougir un qui assertait l'ancienne phrase. C'est le comportement correct d'une
sonde — l'implémentation met à jour l'assertion, elle ne contourne pas.

---

## Hors périmètre

- **Le livrable 3 (substrat d'egress) — vol 2.** `make deploy` n'arrête aucun
  démon d'egress (couche 1) et la sonde de vivacité ne vérifie pas la version du
  relais qui écoute (couche 2, motif `stale` / `stale_deferred`). Défaut vivant
  et mesuré ce jour (M5, n=3). Un piège déjà identifié pour ce vol : `make
  install` fait `cp` + `mv` à chaque déploiement même sans changement de
  contenu, donc une comparaison de mtime produit un **faux positif** après tout
  deploy — raison pour laquelle la couche 2 devra atterrir *sans refuser* le
  dispatch. **Tracked in:** senara-solutions/mika#2029 (ce ticket reste ouvert
  après le vol 1 — D9).
- **La cause du stall elle-même** — cpp#177, fermé (M1). Rien à porter ici.
- **Le push de branche vide** (AC3 de #2035) — déjà coupé par mika#1772 R6.
- **Le décisionnel sur les `Retry hint:`** — mika#2149 déclare explicitement que
  rien ne les lit pour décider d'une reprise ; ce vol ne change pas ça.

---

## Résidus dormants, avec leur condition de réveil

- **L'origine du TTFT bimodal.** Septembre a mesuré, sur 371 appels, soit ≤ 29 s
  soit 300–389 s, et zéro valeur entre. cpp#177 a rendu la fenêtre d'attente
  robuste à ce délai, mais n'en a pas expliqué l'origine. **Réveil :** une
  remontée de `idle_timeout` au-dessus de 2 occurrences par semaine, ou un kill
  `stream_stalled` sur une session dont le TTFT dépasse 300 s.
- **La classe « mort-née »** (session initialisée, zéro événement de flux) :
  n=3, toutes le 2026-09-03, toutes le même ticket. Aucune récidive mesurée
  depuis. **Réveil :** une occurrence de `(0 content stream events this
  session)` — motif ancré sur la parenthèse, le motif non ancré compte les
  sessions saines dont le total finit par un zéro.
- **L'extinction apparente d'`idle_timeout`** (M2). **Réveil :** l'inverse des
  deux ci-dessus — si 300 sessions supplémentaires passent sans occurrence, le
  taux devient significativement inférieur à celui de septembre et la classe
  peut être déclarée close sur mesure plutôt que sur silence.

---

## Definition of Done

- `_no_pr_reason_from_halt_subtype` existe, est appelée depuis la branche
  `_terminated_empty`, et la ligne est posée via `_set_pr_status_line`.
- La chaîne `Anthropic 401` n'existe plus dans `dispatch-lib.sh` ; la ligne
  `:1082` sur le 401 GitHub est intacte.
- La sortie du mode `full` nomme `claude-pilot#177`.
- Les sondes V1a–V1f, V2a–V2c, V3 et V4 passent, et chacune a été vue **rougir**
  sur l'état `main` avant le correctif.
- `make test-dispatch-lib`, `make lint`, `cargo test` et
  `scripts/verify-pipeline.sh origin/main` passent.
- Le document `docs/solutions/dev-loop/` est écrit.
- La PR porte `Refs #2029` et **non** `Closes #2029` (D9), et nomme le vol 2.

## Acceptance criteria

Le corps de mika#2029 ne porte pas de section `## Acceptance criteria` ; il
renvoie aux AC de #2035 « comme référence », sous prémisse corrigée. Les
critères ci-dessous les transcrivent pour le périmètre de ce vol.

1. **Un kill sans production est comptable par son nom.** Un callback issu du
   chemin `_terminated_empty` porte exactement une ligne
   `NO_PR: pilot_terminated_<subtype>`, et le parent écrit
   `callback_no_pr_pilot_terminated_<subtype>` dans `tasks.result`. *(AC5 de
   #2035, sous la justification structurelle D1.)*
2. **Le motif est exact, jamais tronqué.** Pour tout sous-type, y compris non
   conforme à `[a-z_]`, le motif capturé par `RE_NO_PR` est le motif entier.
3. **Le callback ne nomme plus de cause réfutée.** Ni `dispatch-lib.sh` ni le
   texte livré à mika-dev ne porte la chaîne `Anthropic 401`. *(AC1 de #2035.)*
4. **Ce qui est retiré est promu, pas perdu.** La lignée réfutée, la mesure M2 et
   le mécanisme cpp#177 vivent dans `docs/solutions/dev-loop/`. *(AC4 de #2035.)*
5. **« Échoue vite et le dit », sous prémisse corrigée.** « Échoue vite » = la
   borne 300 s de claude-pilot, inchangée et hors périmètre `mika` ; « le dit » =
   le motif nommé du critère 1.
6. **Aucune régression du contrat total.** Sur les chemins qui posaient déjà une
   ligne `PR:`/`NO_PR:`, la sortie en porte toujours exactement une, et la
   population *ayant produit du travail* ne reçoit pas de `pilot_terminated_*`.

---

## Fire-Disposition

Ce vol livre **un** détecteur au sens de mika#2306 : **V2a**, le scan qui refuse
la chaîne `Anthropic 401` dans `dispatch-lib.sh`. Son chemin de succès est
« aucune violation trouvée », donc la section est requise.

Les autres sondes n'en sont pas : V1a–V1f et V2c interrogent le comportement
d'un chemin d'exécution (quelle ligne une fonction produit), V4 est une sonde
d'hermétisme. V3 est un scan de source mais c'est une **garde de garde** —
l'invariant mika#2539, déjà pinné par un scan existant à `:9682`, étendu au
nouveau site.

**Option retenue : (a) exception nommée en allowlist — avec une allowlist
VIDE.**

C'est le cas nominal de (a), et il est atteignable ici parce que le même commit
qui arme le détecteur supprime les deux seules violations (sites A et B). La
mesure M4 l'établit : le prédicat `Anthropic 401` compte exactement 2
occurrences, et elles sont toutes deux dans le périmètre de ce vol. Aucune
exception n'est donc écrite — non par indulgence, mais parce qu'il n'en reste
aucune à couvrir.

L'assertion auto-nettoyante que (a) exige prend ici la forme de **V2b** : elle
affirme que `401 on api.github.com` (`:1082`) est **toujours présent**. Elle
rougit dans les deux directions dangereuses — si quelqu'un élargit le prédicat
du détecteur jusqu'à mordre la ligne GitHub, et si quelqu'un satisfait V2a par
une suppression trop large. C'est la propriété qu'une exception nommée aurait
dû porter si une exception avait été nécessaire : elle pin la **frontière** du
détecteur, et pas seulement son verdict.

Le détecteur atterrit donc **armé et vert**, sans dette et sans suivi à ouvrir.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `dispatch-lib.sh` — `_no_pr_reason_from_halt_subtype` + son commentaire (D3 : pourquoi la normalisation, quel lecteur) | 22 |
| `dispatch-lib.sh` — pose de la ligne dans la branche `_terminated_empty` + commentaire (D4 source, D5 helper, D6 site) | 16 |
| `dispatch-lib.sh` — réécriture du site A (`:2056`) et du site B (`:5264`) | 18 |
| `test-dispatch-lib.sh` — V1a–V1e (quatre sous-types, lisibilité regex, normalisation, vide, unicité) | 52 |
| `test-dispatch-lib.sh` — V1f (contre-vacuité : le mode `banner` ne pose rien) | 18 |
| `test-dispatch-lib.sh` — V2a–V2c (scan, contre-vacuité `:1082`, cpp#177 nommé) | 26 |
| `test-dispatch-lib.sh` — V3 (scan de source, source du sous-type) + V4 (hermétisme) | 22 |

Total estimé : 174 lignes

Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Pourquoi un seul vol tient ici.** 174 contre une borne de 243 : la marge est
de 28 %, ce qui absorbe le facteur de sous-estimation bas de la distribution
mika#2693 (1,3 × 174 = 226, sous la borne) sans absorber le facteur haut — d'où
le découpage déjà pris en amont. Les trois livrables de septembre réunis étaient
estimés à ~230, soit 95 % de la borne : sans marge contre un facteur mesuré
entre 1,3 et 2,2. Le livrable 3 est donc sorti (§ *Hors périmètre*), ce qui
retire aussi le `Makefile` et la garde de cycle de vie du relais du périmètre de
cette PR — deux surfaces dont la fin de pipeline (deploy, redémarrage du relais)
coûte des tours et non des lignes.
