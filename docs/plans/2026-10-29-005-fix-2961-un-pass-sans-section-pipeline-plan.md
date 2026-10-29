# fix(2961) — un `pass` sans section `PIPELINE:` n'est pas posté comme `pass`

> Ticket : senara-solutions/mika#2961 — `bug`, `p1-important`, `dispatch:loop`, `loop-substrate`
> Vol 1/2. Suite nommée en fin de document par `Tracked in:`.

## 1. Contexte

Le prompt `qa-review` exige depuis mika#2172 (#2217) et mika#2264 (#2268) deux
choses de **chaque** verdict : une section `PIPELINE:` citant chaque garde
lancée (Step 2E, ou `PIPELINE: not-applicable (…)` par Step 2D quand le dépôt
n'a aucune garde) et exactement une ligne `NEGATIVE-TEST:` (Step 2.5.4b). La
ligne 45 du prompt plafonne à `hold[review]` tout verdict qui en manque.

Aucun garde moteur ne tient cette règle sur la population `pass`. Le seul garde
de la famille, mika#2687, a une population étroite et déclarée : `block[pipeline]`
seul (`guards.rs:4420-4436`, `classify_pipeline_block_grounding` retourne
`Allowed` dès que `verdict_is_pipeline_block` est faux).

## 2. Mesure refaite — et la mesure du ticket est corrigée à la hausse

Le ticket mesure la population avec `contains("PIPELINE:")`. Le prédicat du
moteur, `pipeline_section_present` (`guards.rs:4339`), est **ancré en début de
ligne** (blancs de tête et course d'emphase markdown tolérés, casse repliée).
Les deux ne mesurent pas la même chose, et l'écart porte précisément sur les
4 verdicts que le ticket compte comme conformes.

Requête (ancrage de ligne, le prédicat réel) :

```sh
gh pr list --repo senara-solutions/<repo> --state all --limit 60 \
  --json number,reviews,state \
  --jq '[.[] | .number as $n | .state as $st | .reviews[]
         | select(.author.login=="mika-platform-qa")
         | select(.body|startswith("VERDICT: pass"))
         | {n:$n, st:$st,
            pipe:([.body|split("\n")|.[]
                   |select(test("^[ \t*_]*[Pp][Ii][Pp][Ee][Ll][Ii][Nn][Ee]:"))]|length)}]
        | {pass:length,
           sans_pipeline:([.[]|select(.pipe==0)]|length),
           merged_sans_pipeline:([.[]|select(.pipe==0 and .st=="MERGED")]|length)}'
```

Relevé du 2026-10-29, 60 dernières PR par dépôt :

| dépôt | `pass` | ligne `PIPELINE:` ancrée | ligne `NEGATIVE-TEST:` ancrée |
|---|---|---|---|
| mika | 79 | 0 | 0 |
| claude-pilot | 96 | 0 | 0 |
| mika-platform | 70 | 0 | 0 |
| control-monitor | 16 | 0 | 0 |
| **total** | **261** | **0** | **0** |

Et sur mika#2691, les 4 verdicts que le ticket compte comme porteurs de la
section :

```
2026-10-14T14:00:51Z  pass          contains=true  lignes_ancrées=0
2026-10-14T15:05:35Z  pass          contains=true  lignes_ancrées=0
2026-10-14T15:20:47Z  pass          contains=true  lignes_ancrées=0
2026-10-14T16:25:27Z  pass          contains=true  lignes_ancrées=0
```

Le mot `PIPELINE:` y est **dans la prose** — cette PR implémentait mika#2687 et
parle de « a `PIPELINE:` section citing a failing guard ». **Le régime réel est
donc 426 sur 426, pas 422 sur 426 : aucun `pass` de la population mesurée ne
porte de section.** Ce fait a deux conséquences qui structurent tout le plan :

1. le prédicat du garde doit rester **ancré**. Un `contains` compterait la prose
   d'une revue comme sa preuve — c'est le fail-open que le ticket condamne ;
2. le contrôle positif que demande AC2 (« une revue de mika#2691 qui porte
   `PIPELINE:` passe ») **n'existe pas dans la population**. § 4.4 dit comment
   AC2 est tenu sans l'inventer.

Ligne de base à 7 jours (depuis 2026-10-22), l'état de départ de la sonde AC5 :

| dépôt | `pass` | sans ligne `PIPELINE:` | dont **mergées** |
|---|---|---|---|
| mika | 69 | 69 | 69 |
| claude-pilot | 31 | 31 | 27 |

## 3. Pourquoi mika#2264 n'a rien vu

`skills/bundled/self-dev-webhook-qa/system_prompt.md:36` dit, côté mika-dev :
« Line absent entirely → … Do NOT call `pr_merge_with_gate` … notify Vincent ».
C'est une **consigne de prompt adressée à un second modèle**, pas un garde
moteur. Le relevé ci-dessus la mesure : sur 7 jours, 69 PR mika portant un
`pass` sans ligne `NEGATIVE-TEST:` sont **mergées**, soit la totalité. Le
contrôle n'a pas arrêté un merge sur 69. C'est la même classe de dérive que
celle du premier modèle, une copie plus loin — exactement l'argument du ticket
contre une troisième passe de prompt.

## 4. Décisions

### 4.1. D1 — Refuser, et non plafonner à `hold[review]`

**Décision : refuser**, dans la chaîne pré-subprocess de `run_gh`.

La chaîne retourne `Result<(), ToolOutput>` : elle peut laisser passer ou
refuser, rien d'autre. Plafonner demanderait de **muter l'argv** — réécrire la
ligne `VERDICT:` du corps et basculer `--approve` en `--comment` — c'est-à-dire
poster, sous l'identité du relecteur, un verdict qu'il n'a pas écrit. Aucun des
quatre gardes de la famille (mika#1646, mika#2237, mika#2687, mika#2455,
mika#2519) ne mute un argv ; tous rendent `Ok(())` ou `Err(ToolOutput::error)`.
Un plafonnement serait le premier, et il falsifierait une signature.

Ce que chaque choix aurait fait sur la population mesurée :

| choix | effet sur les 261 `pass` relevés | effet sur un modèle qui s'entête |
|---|---|---|
| refuser, armé d'emblée | 261 refus au premier appel, soit tout le régime | tour mort, aucune revue postée |
| plafonner | 261 verdicts réécrits par le moteur sous la signature du relecteur | aucun tour mort, mais le `pass` du relecteur devient indiscernable d'un `hold` du moteur |
| **refuser, désarmé par défaut** | **0 — le régime ne change pas** | **aucun : le garde ne mord pas avant l'ouverture** |

L'asymétrie que calcule la doc de `validate_qa_ci_coherence` (un faux positif
tue le tour sans revue) n'est donc **pas** tenue par le plafonnement : elle est
tenue par l'interrupteur de D5, qui laisse à Vincent le choix de l'instant.

### 4.2. D2 — Élargir le garde mika#2687, ne pas en écrire un second

AC1 demande « dans la même chaîne que mika#2687 ». La lecture la plus fidèle, et
la moins coûteuse, est **dans le même garde** : la population de
`classify_pipeline_block_grounding` passe de un à deux membres, et
`validate_pipeline_block_grounding_gated` reçoit un second bit d'interrupteur.

Ce que cela supprime : un second `validate_*` avec son wrapper, son helper
d'audit, son `enum` et son module de test — environ 150 lignes dont le seul
contenu propre aurait été une duplication. Ce que cela préserve : la place dans
la chaîne (juste après mika#2237, avant mika#2455), déjà justifiée par
« dernière de la famille à ne lire ni le réseau ni la base » — le nouveau membre
ne lit ni l'un ni l'autre, donc la justification tient inchangée.

Le risque est de toucher un garde armé en production. Il est borné et mesurable :
l'élargissement est **additif** (une branche de plus dans la fonction pure, en
tête, sur un paramètre neuf), le chemin `block[pipeline]` conserve ses trois
termes dans leur ordre, et les **24 tests `mika2687_*`** existants
(`guards.rs:9364-9759`) sont le contrôle de non-régression.

### 4.3. D3 — Population `pass`, et **présence** de la section seulement

Le terme exigé du `pass` est `pipeline_section_present(body)`, **pas**
`body_cites_failing_guard(body)`. Un `pass` ne cite aucun échec par
construction : Step 2E lui prescrit `PIPELINE: pass (<repo>)` avec `exit 0`, et
Step 2D `PIPELINE: not-applicable (…)` quand le dépôt n'a aucune garde. La
polarité du second terme est donc **inversée entre les deux membres de la
population**, et c'est voulu :

| verdict | section présente | cite un échec | exécution observée |
|---|---|---|---|
| `block[pipeline]` | exigée | **exigée** | exigée (vol 2 de mika#2687) |
| `pass` | **exigée** | interdite d'exiger (§ ci-dessus) | hors vol — voir `Tracked in:` |

Conséquence directe sur le cas mesuré de control-monitor : `PIPELINE:
not-applicable (…)`, que mika#2687 refuse **délibérément** sous
`block[pipeline]` (`mika2687_la_section_not_applicable_de_step_2d_est_refusee_deliberement`),
doit **passer** sous `pass`. Le test du § 4.4 l'épingle dans les deux sens.

Le garde exige la **présence** d'au moins une ligne, pas « exactement une » : la
classe mesurée est l'absence (0 sur 261), jamais la duplication. Resserrer sur
un comptage ajouterait un chemin de faux refus du mauvais côté de l'arithmétique
fail-open, pour une classe dont la population mesurée est vide.

### 4.4. D4 — Comment AC2 est tenu, le contrôle positif n'existant pas

AC2 exige trois corps **mesurés**. Les deux premiers sont relevés tels quels ;
le troisième n'existe pas sous la forme attendue (§ 2), et le plan ne l'invente
pas. Les trois tests, en Layer A, plus deux contrôles positifs :

| test | corps | attendu |
|---|---|---|
| T1 | control-monitor#217, 2026-10-29T14:26:24Z | `RefusedPassSectionAbsent` |
| T2 | mika#2960, 2026-10-29T13:57:06Z | `RefusedPassSectionAbsent` |
| T3 | mika#2691, 2026-10-14T14:00:51Z | `RefusedPassSectionAbsent`, **et** `body.contains("PIPELINE:")` vrai |
| T4 | `STEP_2E_GREEN`, fixture **déjà en place** (`guards.rs:9358`) | `Allowed` |
| T5 | `PIPELINE: not-applicable (…)`, verbatim de Step 2D | `Allowed` |

T3 est le test le plus utile du lot et il remplace le contrôle positif qu'AC2
demandait : il assère **les deux faits à la fois** — le corps contient la chaîne
`PIPELINE:`, et il ne porte aucune ligne de section. Il interdit structurellement
d'écrire un jour ce prédicat en `contains`, ce qui est la seule manière dont la
classe mesurée pourrait se rouvrir en silence.

T4 et T5 sont les contrôles positifs, et leurs corps ne sont pas inventés : T4
réutilise une fixture déjà au dépôt, verbatim du prompt déployé (le même recours
que mika#2687 a pris pour `STEP_2E_BLOCKING`), et T5 est le verbatim de Step 2D,
`system_prompt.md:275` — la ligne exacte que control-monitor#217 devait porter.

### 4.5. D5 — L'interrupteur : patron de mika#2687, polarité inversée

`MIKA_QA_PASS_EVIDENCE_GATE`, **désarmé par défaut**.

Le **contrat de lecture** est celui de `PIPELINE_GROUNDING_GATE_ENV`, à
l'identique : un `OnceLock` résolu une fois par process (poser la variable sur
un process démarré n'a aucun effet — le STOP est « poser **et redémarrer** »),
le vocabulaire `0`/`false`/`off`/`no` et `1`/`true`/`on`/`yes`, une valeur non
reconnue qui **laisse au défaut** avec un WARN nommant la valeur entre
guillemets (mika#2220), et une ligne au démarrage depuis `server/mod.rs`.

La **polarité** est inversée, et c'est la seule divergence : ses trois voisins
sont armés par défaut, celui-ci ne l'est pas. Parce que 261 sur 261, et parce
qu'AC4 dit « l'ouverture revient à Vincent ».

Cette inversion retourne le contrôle positif de mika#2205, et le plan le nomme
plutôt que de le subir : chez mika#2687, le silence au démarrage est le cas
nominal et la ligne signale le désarmement ; ici **la ligne est le cas nominal**
et le silence signifie armé. La ligne de boot dit donc toujours quelque chose,
et l'instrument qui distingue les deux régimes n'est pas elle mais
`audit_events` sous `PIPELINE_GROUNDING_AUDIT_TOOL`, écrit sur `allowed` comme
sur `refused` — déjà en place.

### 4.6. D6 — `NEGATIVE-TEST:` n'est pas dans ce vol

AC1 autorise le choix écrit. **Décision : non, et renvoyé au vol 2.** Trois
raisons :

1. **la borne par vol mord.** Les deux motifs ensemble sont estimés à ~520
   lignes contre une borne de 429 (§ Taille estimée). Le motif `PIPELINE:` seul
   tient ; les deux ensemble ne tiennent pas, et un vol mort au plafond ne livre
   ni l'un ni l'autre ;
2. **le jeton `NEGATIVE-TEST` demande sa propre surface.** `PIPELINE` porte
   déjà sa ligne dans `scripts/canonical-tokens.tsv:149` avec sa tolérance
   `ci+emphasis:line-anchored`, donc le vol 1 n'ajoute **aucun** jeton. Un
   second préfixe ancré en ajouterait un, avec son site de match et son relevé —
   surface propre, à livrer avec son lint vert, pas en appendice ;
3. **sa cécité aval demande sa propre mesure.** § 3 établit que mika#2264 n'a
   arrêté aucun merge sur 69 ; fermer ce côté-là proprement veut dire décider si
   le garde moteur remplace la consigne de prompt ou s'y ajoute, ce qui est une
   décision de conception et pas un second `if`.

Ce que coûte ce report, dit franchement : tant que le vol 2 n'est pas livré, un
`pass` portant sa section mais pas sa ligne `NEGATIVE-TEST:` traverse le garde.
Le régime mesuré étant 0 sur 261 pour **les deux** lignes, la population qui
profite de ce report est vide à l'instant du relevé.

### 4.7. D7 — AC3 : l'abstention de mika#2455 reste fail-open, inchangée

**Décision écrite : oui, elle reste ouverte.** Aucune ligne de ce vol ne touche
`validate_qa_ci_coherence`. Son arithmétique est juste et le ticket ne la
conteste pas : un faux refus rend une revue légitime impossible à poster *et*
fait traverser la porte de merge à une PR réellement cassée.

Ce qui tient alors le merge fermé, sur le cas mesuré de control-monitor —
réponse franche : **rien de mécanique.** Le merge y est un acte humain, et c'est
un humain qui a tenu la porte (samidarko a requalifié l'approbation en
`hold[review]` le 2026-10-29). La doc de `validate_qa_ci_coherence` dit « the
merge itself stays closed by `pr_merge_with_gate`'s own CI gate », et cet
argument ne vaut, comme le ticket l'écrit, que sur un dépôt qui merge par là.

Ce que le vol change n'est donc pas la porte, c'est le **signal** : l'enchaînement
mesuré du 2026-10-29 — `qa_ci_coherence_abstained` à 14:26:21Z, puis
`pr review --approve` nu à 14:26:22Z — n'est plus postable une fois
l'interrupteur ouvert. Sur un dépôt sans garde, Step 2D prescrit
`PIPELINE: not-applicable (…)` ; le garde exige cette ligne (D3), donc une
approbation sans aucune preuve pipeline est refusée **avant** d'exister sur
GitHub. L'abstention CI reste fail-open ; l'approbation nue qu'elle laissait
passer, non.

## 5. Implémentation

### 5.1. `crates/mika-agent/src/evidence/guards.rs`

Sous la section mika#2687, un bandeau mika#2961 qui porte la mesure du § 2 (261
sur 261, et l'écart `contains`/ancrage sur les 4 verdicts de #2691), puis :

- `PASS_EVIDENCE_GATE_ENV: &str = "MIKA_QA_PASS_EVIDENCE_GATE"` ;
- un variant de plus sur `PipelineGroundingOutcome` :
  `RefusedPassSectionAbsent`, dont la doc dit que c'est la seule classe du
  membre `pass` et pourquoi les deux autres variants ne s'y appliquent pas (D3) ;
- `classify_pipeline_block_grounding` prend un paramètre de plus,
  `verdict_is_pass: bool`, et une branche **en tête** :

  ```
  si verdict_is_pass      → section présente ? Allowed : RefusedPassSectionAbsent
  si !verdict_is_pipeline_block → Allowed                      (inchangé)
  …les trois termes de mika#2687, dans leur ordre               (inchangés)
  ```

  Les deux booléens sont mutuellement exclusifs chez l'appelant (un corps a une
  classe de verdict) ; la doc le dit et le `pass` est testé en tête parce que
  son terme est le moins coûteux ;
- `pass_evidence_gate_is_armed(raw: Option<&str>) -> bool` (défaut **false** :
  absente, vide ou non reconnue ⇒ désarmé, avec WARN sur la troisième),
  `pass_evidence_gate_armed()` sur `OnceLock`, et
  `log_pass_evidence_gate_state()` qui dit le désarmement — donc parle par
  défaut (D5).

### 5.2. `crates/mika-agent/src/skills/builtin_handlers.rs`

- `validate_pipeline_block_grounding` passe le second bit :
  `pass_evidence_gate_armed()` ;
- `validate_pipeline_block_grounding_gated` prend `pass_gate_enabled: bool`.
  Son court-circuit de tête devient « les **deux** gates désarmés ⇒ `Ok(())` ».
  La lecture du verdict reconnaît `Verdict::Pass` en plus de
  `Verdict::Block("pipeline")` — avec `parse_verdict`, exactement comme le lit
  `verdict_handler`, et avec la même casse repliée. Un `pass` dont le gate
  propre est désarmé sort en `Ok(())` avant la lecture en base ;
- la lecture du ledger (`observe_pipeline_guard_execution`) reste **réservée au
  membre `block[pipeline]`** : le membre `pass` n'en a pas besoin (D3) et la
  payer serait un appel base sur la population la plus fréquente de la flotte ;
- le nouveau variant reçoit son couple `(reason, reasoning)` —
  `"pass_section_absent"` — et un **remedy propre**. Celui de mika#2687 parle
  d'une garde en échec et serait un contresens ici. Le remedy du `pass` nomme
  les deux sorties correctes, sur le patron de mika#2237 : poster la section que
  Step 2E prescrit avec la sortie de la garde telle qu'elle s'imprime, ou
  `PIPELINE: not-applicable (…)` de Step 2D si le dépôt n'a aucune garde. Et,
  contrainte structurelle reprise de mika#2687 : **aucune ligne de ce texte ne
  commence par le libellé d'un verdict**, `VERDICT_RE` étant ancré sur ce
  préfixe.

Le nom `validate_pipeline_block_grounding` devient inexact dès lors que la
population a deux membres. Le renommage en `validate_pipeline_evidence` part au
vol 2 avec le second motif, pour ne pas mêler un diff de renommage à un diff de
comportement : le vol 1 écrit la divergence en commentaire au-dessus de la
fonction, qui nomme le ticket de suite.

### 5.3. `crates/mika-agent/src/server/mod.rs`

`log_pass_evidence_gate_state()` à la suite de ses trois voisins (`:981`), avec
le commentaire qui dit la polarité inversée : ici la ligne est l'état nominal,
et son absence veut dire que le garde mord.

### 5.4. Docs (hors compte de taille)

- `docs/operator/environment-variables.md` : `MIKA_QA_PASS_EVIDENCE_GATE`, son
  défaut OFF, le geste d'ouverture (`=1` + redémarrage), la sonde AC5 avec sa
  requête, et la sonde d'audit (`audit_events` sous `pipeline_grounding_guard`,
  `GROUP BY after_value`) ;
- `docs/agent-internals/tools.md` : la population à deux membres et
  l'inversion de polarité du § 4.3 ;
- `docs/solutions/best-practices/` : le compound du § 2 — *une mesure en
  `contains` et un prédicat ancré ne comptent pas la même population*, le fait
  qu'elle ait déplacé le régime de 422/426 à 426/426, et pourquoi le test T3
  est le garde de cette doctrine.

## 6. Verification Contract

| # | vérification | commande / lecture |
|---|---|---|
| V1 | Layer A : T1–T5 du § 4.4 | `cargo test -p mika-agent mika2961` |
| V2 | non-régression mika#2687 : les 24 tests existants verts, inchangés | `cargo test -p mika-agent mika2687` |
| V3 | Layer B : le corps mesuré de control-monitor#217 est refusé **avant** le subprocess, gate armé | `cargo test -p mika-agent mika2961_pass_evidence` |
| V4 | Layer B : contrôle positif — un corps portant la section traverse **et** laisse une ligne `allowed` dans `audit_events` (mika#2205) | idem V3 |
| V5 | Layer B : gate désarmé ⇒ le corps mesuré traverse | idem V3 |
| V6 | polarité : défaut désarmé, armé par `1`/`true`/`on`/`yes`, valeur non reconnue ⇒ désarmé + WARN | `cargo test -p mika-agent mika2961` |
| V7 | le remedy n'écrit aucune ligne commençant par un libellé de verdict | assertion dans V3 |
| V8 | suite complète, lints, pipeline | `cargo test`, `cargo clippy`, `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`, `scripts/check-byte-slices.sh`, `scripts/canonical-tokens` lint |

Aucun jeton canonique ajouté : le vol réutilise `PIPELINE_SECTION_LINE_PREFIX`,
déjà déclaré (`canonical-tokens.tsv:149`). V8 vérifie que le relevé reste vert
sans nouvelle ligne.

## 7. Definition of Done

- La population du garde mika#2687 a deux membres ; le chemin `block[pipeline]`
  est inchangé et ses 24 tests sont verts sans édition.
- `MIKA_QA_PASS_EVIDENCE_GATE` existe, est désarmé par défaut, est lu une fois,
  et le démarrage le dit.
- V1–V8 verts.
- Docs du § 5.4 écrites, `docs-sync` vert.
- Périmètre CODEOWNERS : **mesuré, aucun chemin du vol n'y figure.**
  `.github/CODEOWNERS` nomme `perimeter/`, `server/verdict_handler.rs`,
  `tools/pr_merge_with_gate.rs`, `docs/gate/`, `docs/egress/`,
  `well_known_agents.rs` et lui-même ; ce vol touche `evidence/guards.rs`,
  `skills/builtin_handlers.rs` et `server/mod.rs`, aucun des trois. L'anticipation
  que demandait le ticket se résout donc par la mesure : la PR n'est pas bloquée
  par CODEOWNERS. Elle reste à merger par Vincent sur la consigne d'origine du
  ticket (`loop-substrate`), ce qui est une décision d'opérateur et non une
  règle de fichier.

## 8. Acceptance criteria

- **AC1.** Un `pr review --approve` dont le corps porte `VERDICT: pass` sans
  ligne `PIPELINE:` n'est pas posté comme `pass`, dans la même chaîne que
  mika#2687. Il est soit refusé, soit plafonné à `hold[review]`, selon le choix
  écrit du plan. Le plan dit s'il traite `NEGATIVE-TEST:` dans le même garde, ou
  pourquoi non.
  → Choix écrit : **refusé** (§ 4.1), dans le **même garde** que mika#2687
  (§ 4.2). `NEGATIVE-TEST:` : **non**, renvoyé au vol 2, trois raisons au § 4.6.
- **AC2.** Test dans le même appel, sur des corps **mesurés** et non inventés :
  les revues de control-monitor#217 (2026-10-29T14:26:24Z) et de mika#2960
  (2026-10-29T13:57:06Z) sont arrêtées, et une revue de mika#2691 (2026-10-14)
  qui porte `PIPELINE:` passe.
  → T1, T2 et T3 (§ 4.4), plus V3 pour « dans le même appel ». Le troisième
  volet est tenu autrement qu'écrit, et le § 2 dit pourquoi : la revue de
  mika#2691 porte `PIPELINE:` au sens de `contains` et **aucune ligne ancrée**,
  comme les trois autres `pass` de cette PR — le contrôle positif n'existe pas
  dans la population. T3 épingle les deux faits sur ce corps mesuré ; T4 et T5
  fournissent les contrôles positifs sur des corps verbatim du prompt déployé.
- **AC3.** L'abstention de mika#2455 : le plan tranche par écrit si elle reste
  fail-open quand le dépôt de la PR ne merge pas par `pr_merge_with_gate`, avec
  control-monitor comme cas mesuré. Si elle reste ouverte, il dit ce qui tient
  alors le merge fermé.
  → § 4.7 : elle **reste ouverte**, inchangée. Ce qui tient le merge fermé sur
  control-monitor est un humain, dit franchement ; ce que le vol ferme est la
  production de l'approbation nue, pas la porte.
- **AC4.** Un interrupteur suit le patron de `PIPELINE_GROUNDING_GATE_ENV`, avec
  le même contrat : lu une fois, et dit au démarrage s'il est désarmé.
  L'ouverture revient à Vincent.
  → § 4.5 : `MIKA_QA_PASS_EVIDENCE_GATE`, contrat de lecture identique, polarité
  inversée (défaut OFF) et l'inversion du contrôle mika#2205 nommée. V6.
- **AC5.** Sonde après ouverture de l'interrupteur : sur les revues
  `mika-platform-qa` de 7 jours, le compte des `pass` sans ligne `PIPELINE:` est
  écrit ici, avec la requête qui l'a produit. La cible est 0. Le compte des tours
  QA morts sans revue sur la même fenêtre est écrit à côté.
  → **Structurellement post-merge** : l'ouverture revient à Vincent (AC4), donc
  la sonde ne peut pas être relevée dans ce vol. Le plan livre la requête (§ 2)
  et la **ligne de base avant ouverture** (mika 69/69, claude-pilot 31/31). Le
  relevé après ouverture, et le compte des tours morts sans revue
  (`server.log`, tours de l'agent mika-qa sans `pr review` abouti sur la
  fenêtre), sont portés par le vol 2.

## 9. Fire-Disposition

Le livrable est un détecteur : un garde dont le chemin de succès est « aucune
violation trouvée ». Option retenue : **(b) livrer désarmé.**

- **Le désarmement.** `MIKA_QA_PASS_EVIDENCE_GATE` est absent par défaut, donc
  `pass_evidence_gate_is_armed(None) == false`, donc le membre `pass` de la
  population sort en `Ok(())` avant toute lecture. C'est l'équivalent exact du
  `#[ignore]` que la doctrine nomme, au niveau du produit et non du test : les
  261 verdicts du relevé traverseraient inchangés sur un binaire livré.
- **Pourquoi pas (a), l'allowlist.** Les violations existantes ne sont pas des
  lignes de code mais des **corps de revue déjà postés sur GitHub** : il n'y a
  aucune donnée au dépôt à exempter nommément, et une entrée grep-visible
  n'aurait rien à désigner. Trois des corps mesurés entrent au contraire au
  dépôt comme **fixtures de test** (T1–T3), ce qui est l'inverse d'une
  exemption : ils y sont pour que le garde refuse, et le jour où le régime
  change ils restent la mémoire de ce qu'il refusait.
- **Les tests ne sont pas désarmés.** Ils passent le bit de gate en **paramètre**
  (`validate_pipeline_block_grounding_gated`, patron déjà en place), donc les
  deux branches sont exercées dès ce vol : V3/V4 sur gate armé, V5 sur gate
  désarmé. Le détecteur est éteint en production et vert en CI.
- **Le suivi pour l'armer.** L'ouverture est l'acte de Vincent (AC4) et la sonde
  AC5 la mesure. Les deux sont portés par le ticket nommé en `Tracked in:`
  ci-dessous, qui porte aussi le second motif et le renommage.

## 10. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/evidence/guards.rs` (bandeau, const, variant, branche, lecteur de gate + log) | 140 |
| tests Layer A (`mod mika2961` dans `guards.rs` — 3 fixtures mesurées, T1–T5, polarité) | 144 |
| `crates/mika-agent/src/skills/builtin_handlers.rs` (second bit, `Verdict::Pass`, reason, remedy du `pass`) | 70 |
| tests Layer B (`mod mika2961_pass_evidence` — V3, V4, V5, V7) | 48 |
| `crates/mika-agent/src/server/mod.rs` (log au démarrage) | 6 |

Total estimé : 408 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient sous la borne, et ce qui en a été retiré pour l'y
mettre.** L'estimation des deux motifs ensemble, avec un garde neuf plutôt
qu'élargi, était de ~670 lignes. Deux décisions l'ont ramenée à 408 :

- **D2 (élargir mika#2687 plutôt qu'écrire un second garde)** supprime un
  `validate_*` complet, son wrapper, son helper d'audit et son `enum` propre —
  ~150 lignes dont le seul contenu aurait été une duplication de plomberie. Elle
  supprime aussi tout nouveau jeton canonique : `PIPELINE_SECTION_LINE_PREFIX`
  est réutilisé tel quel, prédicat ancré compris, donc zéro ligne dans
  `canonical-tokens.tsv` et zéro surface de lint.
- **D4 (`NEGATIVE-TEST:` au vol 2)** retire ~110 lignes : un second préfixe, sa
  ligne de jeton canonique avec son site de match, son variant, son remedy et sa
  batterie.

Le renommage `validate_pipeline_block_grounding` → `validate_pipeline_evidence`
(~10 lignes et un diff de renommage à travers deux modules de test) part au vol
2 avec le second motif, pour que ce vol-ci ne mêle pas un diff de nom à un diff
de comportement.

Marge : 21 lignes sous la borne, soit 5 %. Deux repères qui la rendent
soutenable : le vol ne crée **aucun** symbole de plomberie (ni `validate_*`, ni
`enum`, ni helper d'audit, ni jeton), et le poste le plus lourd — les tests — est
borné par énumération, cinq cas nommés au § 4.4 sur trois fixtures de 8 à 12
lignes, sur le gabarit mesuré des fixtures `MEASURED_2685_*` de mika#2687
(`guards.rs:9316-9338`).

Ordre de commit du vol : (1) `guards.rs` — const, variant, branche, lecteur de
gate ; (2) tests Layer A ; (3) `builtin_handlers.rs` — second bit, `Verdict::Pass`,
remedy ; (4) tests Layer B ; (5) `server/mod.rs` ; (6) docs. Le plancher du vol
est l'étape 4 : en deçà, le garde est câblé sans contrôle de câblage et AC2
n'est pas tenu.

Tracked in: ticket de suivi à ouvrir après le merge de ce vol — vol 2/2 de
mika#2961 : le motif `NEGATIVE-TEST:` dans le même garde (avec son jeton
canonique et son lint), le renommage `validate_pipeline_evidence`, la sonde AC5
après ouverture de l'interrupteur par Vincent, et le compte des tours QA morts
sans revue sur la même fenêtre.
