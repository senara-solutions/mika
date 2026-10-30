# mika#2961 — vol 2/2 : la ligne `NEGATIVE-TEST:` dans le garde du `pass`

- **Ticket :** senara-solutions/mika#2961
- **Branche :** `fix/2961/qa-gate-mika-qa-poste-des-pass-sans`
- **Vol 1/2 :** PR #2967, mergée le 2026-10-30 à 06:41:19Z, commit `3335ecf5` —
  la section `PIPELINE:`. Son plan :
  `docs/plans/2026-10-29-005-fix-2961-un-pass-sans-section-pipeline-plan.md`.
- **Type :** fix

## 0. En tête, avant toute implémentation : la contrainte de périmètre et ce qu'elle reporte

La contrainte du 2026-10-30 interdit à ce vol les fonctions que trois PR
ouvertes modifient : `validate_qa_ci_coherence`,
`validate_qa_ci_coherence_with_reader` et `run_gh` (mika#2966),
`make_pr_dedup_key` (mika#2973).

**Ce vol ne touche aucune des quatre.** Mais une pièce de sa charge utile, telle
que le vol 1 l'a nommée dans son `Tracked in:`, en touche une — et le ticket
demande que ce soit dit ici plutôt que découvert à l'implémentation.

**Le renommage `validate_pipeline_block_grounding` → `validate_pipeline_evidence`
est reporté.** Sa raison est mécanique et vérifiée : la fonction n'a **qu'un seul
site d'appel**, `builtin_handlers.rs:5403`, et ce site est **dans le corps de
`run_gh`** (`run_gh` ouvre à `builtin_handlers.rs:5236`). Renommer la fonction
édite donc une ligne à l'intérieur de `run_gh`, que la contrainte place hors
périmètre. Le report n'est pas un arbitrage de budget : aucune formulation du
renommage ne l'évite, puisque le renommage *est* l'édition de ce nom à son site
d'appel.

Conséquence assumée : le nom `validate_pipeline_block_grounding` reste inexact
pendant encore un vol, pour une population qui passe de deux membres (vol 1) à
deux membres et trois classes de refus sur le membre `pass` (ce vol). Le
commentaire de divergence que le vol 1 a écrit au-dessus de la fonction
(`builtin_handlers.rs:4096-4100`) est **mis à jour** pour nommer ce plan comme la
raison du report, et le renommage repart dans la ligne `Tracked in:` du § 10.
C'est la seule édition hors-zone-vol-1 de ce plan, et elle est en commentaire.

Deux précisions sur la frontière, pour qu'un relecteur n'ait pas à les
reconstruire :

- le scan de source de `builtin_handlers.rs:14295` **lit** le texte de `run_gh`
  pour établir que `validate_pipeline_block_grounding` n'a pas d'autre appelant
  (garde de la classe mika#2158). Ce vol l'étend au nouveau préfixe. Lire le
  corps d'une fonction n'est pas le modifier : le fichier de test est hors des
  quatre fonctions nommées, et aucune ligne de `run_gh` ne bouge ;
- la zone de ce vol est exactement celle du vol 1 :
  `crates/mika-agent/src/evidence/guards.rs`, plus
  `validate_pipeline_block_grounding`, `validate_pipeline_block_grounding_gated`
  et les deux modules de test `mika2687_pipeline_grounding_tests` /
  `mika2961_pass_evidence` dans `builtin_handlers.rs`. Plus une ligne dans
  `scripts/canonical-tokens.tsv`.

## 1. Contexte

Le prompt de qa-review exige **exactement une** ligne `NEGATIVE-TEST:` par corps
de verdict, en ligne de premier niveau, quel que soit le verdict émis
(`skills/bundled/qa-review/system_prompt.md`, l. 406-415), avec quatre formes
verbatim :

```
NEGATIVE-TEST: satisfied — <invariant named> — <test file:symbol asserting it>
NEGATIVE-TEST: mechanical — <invariant named> — no behavior change in perimeter
NEGATIVE-TEST: missing — <invariant named> — no negative assertion in diff
NEGATIVE-TEST: n/a — PR out of perimeter
```

L'exigence date de mika#2264 (#2268, 2026-09-09). Le prompt plafonne à
`hold[review]` tout verdict qui en manque (l. 45). Aucun garde moteur ne la
tient : le vol 1 a fermé le motif `PIPELINE:` du membre `pass` et a écrit
explicitement, en commentaire à `guards.rs:4669`, que la ligne
`NEGATIVE-TEST:` reste ouverte.

Ce vol la ferme, dans le **même** garde, avec le prédicat ancré, la place dans la
chaîne et l'interrupteur que le vol 1 a mutualisés.

## 2. Mesure refaite pour ce motif — 2026-10-30

Revues `mika-platform-qa` dont le corps commence par `VERDICT: pass`, 60
dernières PR par dépôt (tous états), prédicat **ancré** appliqué ligne à ligne :

| dépôt | `pass` | ligne `PIPELINE:` ancrée | ligne `NEGATIVE-TEST:` ancrée | `contains("NEGATIVE-TEST:")` |
|---|---|---|---|---|
| mika | 78 | 0 | 0 | 1 |
| claude-pilot | 96 | 0 | 0 | 0 |
| mika-platform | 70 | 0 | 0 | 0 |
| control-monitor | 16 | 0 | 0 | 0 |
| **total** | **260** | **0** | **0** | **1** |

Requête (un dépôt par appel) :

```
gh pr list --repo senara-solutions/<dépôt> --state all --limit 60 --json reviews \
  --jq '[.[] | .reviews[]
         | select(.author.login=="mika-platform-qa")
         | select(.body|startswith("VERDICT: pass"))]
        | {pass:length,
           nt_ancree:([.[]|select(.body|split("\n")|any(
             test("^[ \t*_]*[Nn][Ee][Gg][Aa][Tt][Ii][Vv][Ee]-[Tt][Ee][Ss][Tt]:")))]|length),
           nt_contains:([.[]|select(.body|contains("NEGATIVE-TEST:"))]|length)}'
```

**Le régime est 260 sur 260.** Et l'écart `contains`/ancrage, que le vol 1 avait
établi comme doctrine sur `PIPELINE:`, se rejoue ici avec un cas plus net encore.
L'unique corps sur 260 qui `contains("NEGATIVE-TEST:")` est **la revue de la PR
#2967 elle-même** — le vol 1 — et l'occurrence est dans une ligne de case à
cocher d'AC :

```
- [✅] satisfied: AC1 — … `NEGATIVE-TEST:` explicitly deferred to vol 2 (§ 4.6).
```

C'est-à-dire : une mesure en `contains` compterait comme **preuve de test
négatif** une revue qui dit que le test négatif est reporté. Le fail-open n'est
pas théorique, il a un numéro de PR.

**Ligne de base à 7 jours** (PR mergées, `merged:>=2026-10-23`), l'état de départ
de la sonde AC5 pour ce motif :

| dépôt | PR mergées | revues `pass` | `pass` sans ligne `NEGATIVE-TEST:` ancrée | PR mergées sans aucune revue `mika-platform-qa` |
|---|---|---|---|---|
| mika | 46 | 68 | 68 | 0 |
| claude-pilot | 11 | 18 | 18 | 0 |
| **total** | **57** | **86** | **86** | **0** |

La dernière colonne est le compte que l'AC5 demande « à côté » : les tours QA
morts sans revue. Avant ouverture de l'interrupteur il vaut **0**, et c'est
précisément le nombre qui doit rester 0 après — c'est la contrepartie du risque
de refus que le § 4 du vol 1 a calculé.

**Note sur l'état de l'interrupteur du vol 1.** La colonne `PIPELINE:` ancrée
vaut encore 0 sur 260 après le merge de 06:41:19Z, ce qui est l'état **nominal** :
`MIKA_QA_PASS_EVIDENCE_GATE` est désarmé par défaut et son ouverture revient à
Vincent (AC4). La mesure ne distingue pas « non déployé » de « déployé et
désarmé », et ce plan ne le prétend pas ; elle établit seulement que le régime
n'a pas changé, donc que la population de ce vol est la même que celle du vol 1.

## 3. Pourquoi mika#2264 n'a rien vu — et pourquoi la calibration non plus

Deux instruments existent déjà sur cette ligne, et aucun n'est un garde moteur.

1. **Le contrôle de cohérence aval**
   (`skills/bundled/self-dev-webhook-qa/system_prompt.md`, l. 32-36) a trois
   branches : `satisfied|mechanical|n/a` ⇒ merge ; `missing` ⇒ contradiction,
   retenir ; **ligne absente ⇒ retenir et notifier Vincent**. La troisième
   branche est exactement notre population. Elle est **du texte de prompt adressé
   à un second modèle** : sur les 57 PR mergées de la fenêtre de 7 jours, dont 86
   revues `pass` sans la ligne, elle a retenu **0 merge**. C'est la même signature
   que le § 3 du vol 1 relevait sur `PIPELINE:`.
2. **La scène de calibration** `negative_test_invariant_gate`
   (`crates/mika-agent/src/calibration/roles/mika_qa.rs:72`, exécution l. 999)
   mesure bien la règle — mais elle mesure le **modèle**, derrière
   `make calibrate-mika-qa` et une clé de fournisseur réelle, pas le trafic. Et
   son assertion est sur la **fuite hors périmètre** (`negative-test: missing` sur
   une PR hors périmètre, l. 1170), pas sur l'absence de la ligne. Un modèle qui
   n'écrit jamais la ligne passe cette scène : le contrôle négatif cherche un
   `missing` de trop, pas un rien.

Une troisième copie de la consigne serait la troisième copie d'une règle qui a
déjà dérivé deux fois. **Ni le prompt aval ni la scène de calibration ne sont
modifiés par ce vol** — ils restent, et ce qui change est qu'un garde moteur leur
retire leur population.

## 4. Décisions

### 4.1. D1 — Refuser, dans le même garde, sur le même membre

**Décision : refus**, pas plafonnement — le vol 1 a tranché cette question pour
le membre `pass` (son § 4.1) et ce vol n'en rouvre pas l'arithmétique. Le refus
se produit au même point : la chaîne pré-subprocess de `run_gh`, par
`classify_pipeline_block_grounding`, **avant** que l'approbation existe sur
GitHub.

Et sur le **même membre** : `verdict_is_pass`. Le terme nouveau est conjoint au
terme existant du membre `pass` :

```
si verdict_is_pass → section PIPELINE: présente ?
                       ├─ non  → RefusedPassSectionAbsent          (vol 1)
                       └─ oui  → ligne NEGATIVE-TEST: présente ?
                                   ├─ non → RefusedPassNegativeTestAbsent  (ce vol)
                                   └─ oui → Allowed
```

### 4.2. D2 — L'ordre des deux termes est choisi pour ne coûter aucune édition de non-régression

L'ordre n'est pas cosmétique. `PIPELINE:` d'abord, `NEGATIVE-TEST:` ensuite,
parce que les trois corps **mesurés** que le vol 1 a mis au dépôt
(`MEASURED_CM217`, `MEASURED_2960`, `MEASURED_2691`) ne portent **ni l'une ni
l'autre** — leurs doc-commentaires le disent chacun, mesuré sur le corps entier.
Avec cet ordre, ils continuent de sortir en `RefusedPassSectionAbsent`, et les
trois tests du vol 1 qui l'assèrent (`mika2961_les_deux_corps_mesures_sont_arretes`,
`mika2961_le_corps_de_2691_contient_le_mot_et_na_pas_la_section`,
`mika2961_hors_population_le_meme_corps_traverse`) **restent verts sans une ligne
d'édition**.

L'ordre inverse les ferait tous basculer sur le variant neuf, ce qui aurait coûté
trois éditions de tests mesurés pour un résultat identique en production — et une
édition de fixture mesurée est précisément ce qu'on ne veut pas banaliser.

### 4.3. D3 — Les deux contrôles positifs du vol 1 **doivent** changer, et c'est le signe que la conjonction mord

Deux tests du vol 1 ne peuvent pas rester tels quels, et ce n'est pas un dégât
collatéral : c'est l'assertion que le second terme existe.

- `mika2961_un_pass_qui_porte_la_section_passe` (`guards.rs:10148`) utilise
  `super::mika2687::STEP_2E_GREEN` — qui porte sa section et **pas** de ligne
  `NEGATIVE-TEST:`. Sous la conjonction, le verdict attendu devient
  `RefusedPassNegativeTestAbsent` ;
- `mika2961_le_not_applicable_de_step_2d_passe_sous_pass` (`guards.rs:10166`)
  utilise `STEP_2D_NOT_APPLICABLE`, même cas pour sa première assertion. **Sa
  seconde assertion** — le même corps refusé en `RefusedNoFailingGuardCited` sous
  `block[pipeline]`, qui porte la doctrine de polarité inversée — est
  **inchangée**.

**Les deux fixtures ne sont pas mutées.** `STEP_2E_GREEN` est `pub(super)` et
partagée avec le module mika2687 (`guards.rs:9532-9537`, promue par le vol 1
justement pour être partagée) : la muter casserait les tests du membre `block`.
`STEP_2D_NOT_APPLICABLE` est privée, mais elle est **verbatim de
`system_prompt.md:275`**, et cette propriété verbatim est tout son intérêt.

Les deux tests sont donc **amendés et renommés** pour dire ce qu'ils assèrent
désormais (`…_porte_la_section_mais_pas_la_ligne_est_refuse`), et les contrôles
positifs sont **de nouvelles fixtures composites** qui portent les deux lignes,
chacune verbatim de sa source de prompt.

### 4.4. D4 — **Présence** de la ligne seulement : ni validation de forme, ni bras de contradiction

Le prompt déclare quatre dispositions et une incohérence explicite : « emitting
`VERDICT: pass` alongside `NEGATIVE-TEST: missing` is a contradiction the
downstream merge handler rejects » (l. 413-415). Deux gestes de plus étaient donc
possibles, et **les deux sont écartés de ce vol** :

- **valider la disposition** (la ligne doit ouvrir sur `satisfied`, `mechanical`,
  `missing` ou `n/a`) ;
- **refuser la contradiction** `pass` + `missing`.

Raison principale, et c'est la doctrine D3 du vol 1 : **on ferme la classe
mesurée.** La classe mesurée est 260 sur 260 *sans aucune ligne*. La population de
`pass` + `missing` est **vide** au relevé, et celle d'une ligne mal formée l'est
aussi — par construction, puisqu'aucune ligne n'existe. Un garde écrit contre une
population vide est un garde dont on ne sait pas s'il mord bien, et dont le
premier vrai contact sera post-armement.

Raison seconde : ces deux gestes ne deviennent pertinents **qu'après** ouverture
de l'interrupteur, quand le modèle commencera à écrire la ligne. Ils appartiennent
à l'après-sonde, pas à ce vol.

**Ce que ce choix coûte, dit franchement.** La présence seule crée une incitation
perverse : la forme la moins chère à produire pour satisfaire le garde est
`NEGATIVE-TEST: missing — … — no negative assertion in diff`, qui est
exactement la seule forme que le prompt déclare incohérente avec `pass`. Le garde
accepterait alors un corps que son propre prompt interdit. Ce risque est **nommé
ici, écrit dans la doc du variant**, et porté par le ticket de la ligne
`Tracked in:` (§ 10) — qui est aussi celui de la sonde : la sonde AC5 post-armement
est exactement l'instrument qui dira si l'incitation s'est matérialisée, et dans
quelle proportion. Fermer le bras de contradiction avant d'avoir ce chiffre serait
dimensionner un garde sur une hypothèse.

### 4.5. D5 — Le membre `block[pipeline]` n'est **pas** étendu

Le terme nouveau va sur le membre `pass` seul. Le membre `block[pipeline]` est
armé **par défaut** en production (`MIKA_PIPELINE_GROUNDING_GATE`, patron
mika#2687) : lui ajouter un terme changerait le comportement au merge **sans
interrupteur pour le refermer**, ce qui est l'inverse exact de ce que le ticket
demande à l'AC4.

Coût nommé : un `block[pipeline]` qui ne porte pas sa ligne `NEGATIVE-TEST:`
traverse encore après ce vol. L'asymétrie est tolérable et elle est dans le bon
sens : un `block` qui manque sa ligne retient un merge qui aurait dû être retenu
de toute façon ; un `pass` qui manque la sienne en ouvre un.

### 4.6. D6 — Le prédicat est **factorisé**, pas dupliqué

`pipeline_section_present` (`guards.rs:4208` à l'origine) marche le préfixe
caractère à caractère depuis `line.trim_start()`, après avoir sauté une suite
d'emphase markdown (`*`/`_`), casse repliée, **sans ancre de fin**. Ce prédicat
est le cœur de la doctrine du § 2 et il ne doit exister qu'une fois.

Ce vol en extrait `line_prefix_present(body: &str, prefix: &str) -> bool`,
re-exprime `pipeline_section_present` à travers lui (corps devenant une seule
ligne), et écrit `negative_test_line_present` de même. Trois conséquences qui
justifient le geste :

- le trait d'union de `NEGATIVE-TEST:` ne demande **aucune** règle : il est un
  caractère du préfixe comme un autre, et `eq_ignore_ascii_case` le traite
  correctement ;
- un futur passage du prédicat en `contains` casserait les deux motifs d'un coup
  — donc les tests des deux motifs le détectent, au lieu d'un seul ;
- aucune ancre de fin, donc
  `NEGATIVE-TEST: satisfied — <invariant> — <file:symbol>` est reconnue par la
  seule présence du préfixe, ce qui est exactement la sémantique voulue par D4.

### 4.7. D7 — Un seul nom d'événement, discriminé par sa `reason`

mika#2131 interdit de publier deux populations sous un nom d'événement. Ce n'est
pas le cas ici : la population est **identique** (le membre `pass`, sous un seul
interrupteur), et ce qui diffère est la **classe de refus**. Le nom
`pass_evidence_refused` du vol 1 est donc réutilisé, et la nouvelle classe reçoit
sa propre `reason` : `"pass_negative_test_absent"`, à côté de
`"pass_section_absent"`.

Le précédent est dans le même fichier : les **trois** classes de refus du membre
`block[pipeline]` (mika#2687) partagent `pipeline_block_refused` et se
soustraient par la colonne `reason`. Le contrôle positif mika#2205 reste entier :
une ligne est écrite dans `audit_events` sous
`PIPELINE_GROUNDING_AUDIT_TOOL = "pipeline_grounding_guard"` sur **chaque** sortie,
`Allowed` comprise, donc `GROUP BY reason` donne la décomposition et une table
vide veut toujours dire « le lien n'est pas atteint ».

### 4.8. D8 — AC3 : l'abstention de mika#2455 reste fail-open, et ce vol ne peut pas en décider autrement

Le vol 1 a tranché par écrit (son § 4.7) : elle **reste ouverte**. Ce vol
confirme, et ajoute une raison structurelle qui n'existait pas le 2026-10-29 :
`validate_qa_ci_coherence` et `validate_qa_ci_coherence_with_reader` sont
explicitement **hors périmètre** par la contrainte du 2026-10-30 (mika#2966). Même
si ce plan voulait refermer l'abstention, il ne pourrait pas — et c'est la bonne
lecture : la question appartient à mika#2966, pas à un vol de garde de preuve.

Ce qui tient le merge fermé sur le cas mesuré de control-monitor, réponse
inchangée et franche : **rien de mécanique.** Le merge y est manuel et c'est un
humain qui a tenu la porte (samidarko a requalifié l'approbation en `hold[review]`
le 2026-10-29). Ce que ce vol ajoute n'est pas une porte, c'est un second motif
sur le **signal** : la revue de control-monitor#217 ne portait ni section ni
ligne, donc elle était déjà arrêtée par le vol 1 ; ce vol ferme le cas voisin
qu'un modèle corrigé par le remedy du vol 1 produirait — une section `PIPELINE:
not-applicable (…)` posée correctement, et toujours aucun test négatif nommé.

### 4.9. D9 — Le jeton canonique : une ligne, sur le patron de la ligne 149

`scripts/canonical-tokens.tsv` déclare aujourd'hui
`PIPELINE` à la ligne 149, quatre colonnes TAB :

```
PIPELINE	A	crates/mika-agent/src/evidence/guards.rs::PIPELINE_SECTION_LINE_PREFIX	ci+emphasis:line-anchored
```

`NEGATIVE` et `NEGATIVE-TEST` n'y apparaissent **0 fois** aujourd'hui. Trois faits
mécaniques vérifiés, qui bornent exactement ce que ce vol doit écrire :

1. le relevé (`scripts/canonical-tokens-survey.sh:386`) capture
   `/[A-Z][A-Z_]{4,}([ -][A-Z][A-Z_]+)*/` — l'alternative `[ -]` fait de
   `NEGATIVE-TEST` **un seul** jeton, pas deux. Une ligne suffit donc ;
2. le périmètre de scan (`scripts/canonical-tokens-survey.sh:455-470`) est
   `crates/*/src/**/*.rs`, `scripts/*.sh` (maxdepth 1, hors `test-*`/`test_*`) et
   `skills/bundled/**/*.sh` : **le markdown n'est pas scanné.** Les occurrences
   existantes dans `qa-review/system_prompt.md` et
   `self-dev-webhook-qa/system_prompt.md` ne créent donc **aucune** obligation
   aujourd'hui, et c'est la déclaration du const Rust de ce vol qui en crée une ;
3. les deux lecteurs sont `scripts/check-canonical-tokens.sh` et
   `canonical_tokens::tests::mika2201_every_match_site_is_declared` — la ligne
   doit être au même commit que le const, sinon le lint rougit.

Ligne à ajouter, tolérance identique à celle de `PIPELINE` puisque le prédicat
est le même (D6) :

```
NEGATIVE-TEST	A	crates/mika-agent/src/evidence/guards.rs::NEGATIVE_TEST_LINE_PREFIX	ci+emphasis:line-anchored
```

### 4.10. D10 — Comment AC2 est tenu, le contrôle positif n'existant toujours pas

AC2 demande les trois corps mesurés arrêtés, et « une revue de mika#2691 qui
porte `PIPELINE:` passe ». Le vol 1 a établi que ce troisième volet **n'existe pas
dans la population** pour `PIPELINE:` (le mot est dans la prose de `REASON:`).
Pour `NEGATIVE-TEST:` c'est pire et mieux documenté : 0 sur 260 lignes ancrées, et
l'unique `contains` est la revue du vol 1 parlant du report (§ 2).

AC2 est donc tenu de la même manière que le vol 1 l'a tenu, en ajoutant un cas
que la mesure de ce jour offre :

- **T1-T3 (hérités, non édités)** — les trois corps mesurés restent arrêtés, en
  `RefusedPassSectionAbsent`, par l'ordre de D2 ;
- **T6** — le corps de la revue de **#2967** (le seul `contains` sur 260) entre au
  dépôt comme fixture mesurée : il `contains("NEGATIVE-TEST:")` et
  `negative_test_line_present` est **faux** sur lui. Les deux assertions sur le
  **même** corps, comme T3 du vol 1 : séparées, chacune survivrait à un passage du
  prédicat en `contains` ;
- **T7-T8** — les contrôles positifs sur des fixtures composites, dont chaque
  ligne est verbatim d'une source de prompt citée en doc (`system_prompt.md:275`
  pour la section, `:409`/`:412` pour les deux formes de la ligne) ;
- **« dans le même appel »** — V3 à V5, Layer B, qui traversent
  `validate_pipeline_block_grounding_gated`.

## 5. Implémentation

### 5.1. `crates/mika-agent/src/evidence/guards.rs`

Sous le bandeau mika#2961 du vol 1, un bandeau de vol 2 portant la mesure du § 2
(260 sur 260, et le cas `contains` de #2967 nommé), puis :

- `NEGATIVE_TEST_LINE_PREFIX: &str = "NEGATIVE-TEST:"`, dont la doc nomme sa
  source de prompt (`qa-review/system_prompt.md:406-415`), les quatre formes, le
  fait que le prédicat est sans ancre de fin, et sa ligne de
  `scripts/canonical-tokens.tsv` ;
- `line_prefix_present(body: &str, prefix: &str) -> bool` — le corps actuel de
  `pipeline_section_present`, paramétré ; `pipeline_section_present` et
  `negative_test_line_present` deviennent des une-lignes qui l'appellent.
  `pipeline_section_present` **garde sa signature et sa visibilité** : aucun de
  ses appelants ne change ;
- un variant de plus sur `PipelineGroundingOutcome` :
  `RefusedPassNegativeTestAbsent`, dont la doc dit (a) qu'il est la **seconde**
  classe du membre `pass`, (b) l'ordre de D2 et la non-régression qu'il préserve,
  (c) **l'incitation perverse de D4 nommée en clair**, avec le ticket de suivi ;
- la branche `pass` de `classify_pipeline_block_grounding` devient la conjonction
  ordonnée de D1. Les trois termes du membre `block[pipeline]` sont
  **inchangés** ; la signature de la fonction est **inchangée** (le corps suffit,
  le second motif ne demande aucun paramètre neuf).

**Aucun** nouveau lecteur d'environnement, **aucun** `OnceLock`, **aucune** ligne
de démarrage : l'interrupteur du vol 1 est réutilisé tel quel (AC4 tenu par
héritage, § 8).

### 5.2. Tests Layer A — `mod mika2961` dans `guards.rs`

Deux fixtures neuves, plus la promotion d'aucune (les trois `MEASURED_*` restent
privées : D2 leur épargne toute édition) :

- `MEASURED_2967_PROSE` — le corps de la revue `pass` de #2967, réduit à ses
  lignes utiles, dont la ligne de case à cocher d'AC qui contient le jeton. Doc :
  le compte mesuré 1 sur 260, et pourquoi c'est le meilleur contrôle de la
  doctrine du prédicat ancré qu'on ait eu ;
- `PASS_CONFORME` et `PASS_CONFORME_NOT_APPLICABLE` — deux composites portant
  section **et** ligne. Le premier sur le patron de Step 2E, le second sur Step 2D
  (dépôt sans garde, le cas control-monitor), avec pour chacun la forme de ligne
  verbatim citée en doc.

Les cas :

| # | cas | attendu |
|---|---|---|
| T4′ | `STEP_2E_GREEN` sous `pass` — section, pas de ligne | `RefusedPassNegativeTestAbsent` *(test du vol 1 amendé + renommé)* |
| T5′ | `STEP_2D_NOT_APPLICABLE` sous `pass` — idem ; et **inchangé** sous `block[pipeline]` | `RefusedPassNegativeTestAbsent` / `RefusedNoFailingGuardCited` *(amendé + renommé)* |
| T6 | `MEASURED_2967_PROSE` : `contains` vrai **et** `negative_test_line_present` faux, sur le même corps, puis classé | `RefusedPassNegativeTestAbsent` |
| T7 | `PASS_CONFORME` sous `pass` | `Allowed` |
| T8 | `PASS_CONFORME_NOT_APPLICABLE` sous `pass` | `Allowed` |
| T9 | indépendance des deux termes : table des 4 combinaisons (section ×  ligne) sur des corps dérivés des composites | `RefusedPassSectionAbsent` / `RefusedPassNegativeTestAbsent` / `Allowed`, et l'ordre de D2 épinglé |
| T10 | hors population (`NOT_A_PASS`) : les composites **et** `MEASURED_2967_PROSE` traversent | `Allowed` |

T9 est le test qui porte D2 : il assère que le corps sans **ni** l'une **ni**
l'autre sort en `RefusedPassSectionAbsent` et non sur le variant neuf. Sans lui,
inverser l'ordre des deux termes passerait inaperçu et les trois fixtures
mesurées changeraient de classe en silence.

### 5.3. `crates/mika-agent/src/skills/builtin_handlers.rs`

Dans `validate_pipeline_block_grounding_gated` — **pas** dans `run_gh` :

- le `match` des raisons passe de 5 à 6 bras ; le nouveau reçoit
  `("pass_negative_test_absent", "pass with no NEGATIVE-TEST: line anywhere in the body")` ;
- le choix du nom d'événement est **inchangé** : `pass_evidence_refused` couvre
  les deux classes du membre, discriminées par `reason` (D7) ;
- un **remedy propre**. Celui du vol 1 parle de la section et serait un
  contresens. Celui-ci nomme la ligne, son caractère obligatoire quel que soit le
  verdict, et les formes utilisables sous `pass` — donc `satisfied`, `mechanical`
  et `n/a`, **et pas `missing`**, dont le remedy dit en une phrase pourquoi elle
  est incohérente avec `pass` (c'est l'endroit où D4 paie son incitation perverse :
  le garde accepte `missing`, le remedy ne la propose pas). `"error":
  "pass_negative_test_absent"`, `"doctrine": "mika#2961"`, sur le patron
  mika#2237 ;
- **deux contraintes structurelles sur ce texte**, héritées et load-bearing :
  1. **aucune ligne du remedy ne commence par un libellé de verdict** —
     `VERDICT_RE` est ancré sur ce préfixe, et un remedy qui en porterait un
     serait relu comme un verdict par le consommateur suivant ;
  2. **aucune ligne du remedy ne porte `NEGATIVE-TEST:` en préfixe de ligne
     ancré** — sinon le remedy satisfait le garde qu'il explique, le jour où le
     modèle le recopie dans son corps suivant. Les formes sont citées **en ligne,
     dans une phrase**, jamais en tête de ligne. Une assertion le vérifie (V7).

Et la mise à jour du commentaire de divergence (`:4096-4100`) : le renommage est
reporté, avec sa raison du § 0 et le ticket de la ligne `Tracked in:`.

### 5.4. Tests Layer B — `mod mika2961_pass_evidence` dans `builtin_handlers.rs`

- un corps portant la section et pas la ligne est refusé **avant** le subprocess,
  interrupteur armé, avec sa ligne `audit_events` sous la nouvelle `reason` ;
- `PASS_CONFORME` traverse **et** laisse une ligne `allowed` (contrôle positif
  mika#2205) ;
- interrupteur **désarmé** ⇒ le même corps traverse ;
- **extension du scan de source** (`:14295`) au nouveau préfixe : il assère
  aujourd'hui que `validate_pipeline_block_grounding` est le seul appelant, ce qui
  ferme la classe mika#2158 (une suite qui resterait verte si un wrapper
  non-gaté figeait son argument à une constante). Le scan gagne l'assertion que le
  nouveau préfixe est bien consommé par le chemin gaté et non en dur ailleurs.

### 5.5. `scripts/canonical-tokens.tsv`

La ligne de D9, une seule, au bon rang alphabétique.

### 5.6. Docs (hors compte de taille)

- `docs/operator/environment-variables.md` : l'entrée
  `MIKA_QA_PASS_EVIDENCE_GATE` (l. 5166-5174) gagne la **seconde** classe de refus
  qu'elle arme désormais, et la requête de sonde du § 2 pour le second motif ;
- `docs/agent-internals/tools.md` : les deux classes du membre `pass`, l'ordre de
  D2 et pourquoi il est le bon ;
- `docs/solutions/best-practices/` : le compound du § 2 — l'unique corps sur 260
  que `contains` compte est **la revue qui annonce le report du test négatif**.
  C'est la meilleure illustration de la doctrine que la mesure ait produite, et T6
  en est le garde.

## 6. Verification Contract

| # | vérification | commande / lecture |
|---|---|---|
| V1 | Layer A : T4′-T10 du § 5.2 | `cargo test -p mika-agent mika2961` |
| V2 | **non-régression du vol 1** : T1, T2, T3 et le hors-population verts **sans édition** (c'est D2 qui est mesuré ici) | `cargo test -p mika-agent mika2961` + `git diff` ne montre aucune édition de leurs corps |
| V3 | non-régression mika#2687 : les 24 tests du membre `block` verts, inchangés | `cargo test -p mika-agent mika2687` |
| V4 | Layer B : section présente, ligne absente ⇒ refus **avant** le subprocess, interrupteur armé | `cargo test -p mika-agent mika2961_pass_evidence` |
| V5 | Layer B : contrôle positif — composite conforme traverse **et** laisse une ligne `allowed` (mika#2205) | idem V4 |
| V6 | Layer B : interrupteur désarmé ⇒ le même corps traverse | idem V4 |
| V7 | le remedy n'écrit aucune ligne commençant par un libellé de verdict, **ni** aucune ligne ancrée `NEGATIVE-TEST:` | assertion dans V4 |
| V8 | jeton canonique : la ligne de D9 est déclarée et le relevé est vert | `scripts/check-canonical-tokens.sh` + `cargo test -p mika-agent mika2201_every_match_site_is_declared` |
| V9 | scan de source : classe mika#2158 fermée sur le nouveau préfixe | `cargo test -p mika-agent mika2961_pass_evidence` |
| V10 | suite complète, lints, pipeline | `cargo test`, `cargo clippy`, `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`, `scripts/check-byte-slices.sh` |

V8 est nouveau par rapport au vol 1, qui n'ajoutait aucun jeton. C'est la surface
que D6 du vol 1 citait comme deuxième raison du report.

## 7. Definition of Done

- Le membre `pass` du garde a **deux** classes de refus ; le membre
  `block[pipeline]` est inchangé et ses 24 tests sont verts sans édition.
- Les trois fixtures mesurées du vol 1 et leurs trois tests sont verts **sans
  édition** ; les deux contrôles positifs du vol 1 sont amendés et renommés, et
  leurs fixtures ne sont pas mutées.
- `NEGATIVE_TEST_LINE_PREFIX` est déclaré dans `scripts/canonical-tokens.tsv` et
  le relevé est vert.
- Le prédicat ancré n'existe **qu'une fois** (`line_prefix_present`), et les deux
  motifs le traversent.
- **Aucune ligne de `run_gh`, `validate_qa_ci_coherence`,
  `validate_qa_ci_coherence_with_reader` ou `make_pr_dedup_key` n'est modifiée** —
  vérifiable par `git diff` : la seule édition de `builtin_handlers.rs` hors
  `validate_pipeline_block_grounding_gated` et hors modules de test est le
  commentaire de divergence du § 5.3.
- V1-V10 verts.
- Docs du § 5.6 écrites, `docs-sync` vert.
- **Périmètre CODEOWNERS : mesuré, aucun chemin du vol n'y figure.**
  `.github/CODEOWNERS` nomme `perimeter/`, `server/verdict_handler.rs`,
  `tools/pr_merge_with_gate.rs`, `docs/gate/`, `docs/egress/`,
  `well_known_agents.rs` et lui-même. Ce vol touche `evidence/guards.rs`,
  `skills/builtin_handlers.rs` et `scripts/canonical-tokens.tsv` — aucun des
  sept. L'anticipation demandée par le ticket se résout donc par la mesure, comme
  au vol 1 : la PR n'est **pas** bloquée par CODEOWNERS. Elle reste à merger par
  Vincent sur la consigne d'origine du ticket (`loop-substrate`), qui est une
  décision d'opérateur et non une règle de fichier.

## 8. Acceptance criteria

- **AC1.** Un `pr review --approve` dont le corps porte `VERDICT: pass` sans
  ligne `PIPELINE:` n'est pas posté comme `pass`, dans la même chaîne que
  mika#2687. Il est soit refusé, soit plafonné à `hold[review]`, selon le choix
  écrit du plan. Le plan dit s'il traite `NEGATIVE-TEST:` dans le même garde, ou
  pourquoi non.
  → Tenu par le vol 1 pour `PIPELINE:` (**refusé**, même chaîne). Pour ce vol :
  `NEGATIVE-TEST:` est traité **dans le même garde**, sur le **même membre**, en
  conjonction du terme existant (§ 4.1), et **refusé** de la même manière. Le
  prédicat est factorisé plutôt que dupliqué (§ 4.6). Ce qui est *écarté* est dit
  en clair : validation de forme et bras de contradiction `pass` + `missing`
  (§ 4.4), le membre `block[pipeline]` (§ 4.5).
- **AC2.** Test dans le même appel, sur des corps **mesurés** et non inventés :
  les revues de control-monitor#217 (2026-10-29T14:26:24Z) et de mika#2960
  (2026-10-29T13:57:06Z) sont arrêtées, et une revue de mika#2691 (2026-10-14) qui
  porte `PIPELINE:` passe.
  → § 4.10. Les deux premiers corps restent arrêtés **sans édition de test**, par
  l'ordre de D2 (V2). Le troisième volet est tenu autrement qu'écrit, et le § 2
  dit pourquoi : le contrôle positif n'existe pas dans la population — 0 ligne
  ancrée sur 260, et l'unique `contains` est la revue du vol 1 annonçant ce
  report. T6 épingle les deux faits sur ce corps mesuré ; T7 et T8 fournissent les
  contrôles positifs sur des composites verbatim du prompt. V4 tient « dans le
  même appel ».
- **AC3.** L'abstention de mika#2455 : le plan tranche par écrit si elle reste
  fail-open quand le dépôt de la PR ne merge pas par `pr_merge_with_gate`, avec
  control-monitor comme cas mesuré. Si elle reste ouverte, il dit ce qui tient
  alors le merge fermé.
  → § 4.8 : elle **reste ouverte**, inchangée, et ce vol ne peut structurellement
  pas en décider autrement — `validate_qa_ci_coherence` est hors périmètre par la
  contrainte du 2026-10-30. Ce qui tient le merge fermé sur control-monitor est un
  humain, dit franchement. Ce que ce vol ferme est le cas voisin : une section
  posée correctement et toujours aucun test négatif nommé.
- **AC4.** Un interrupteur suit le patron de `PIPELINE_GROUNDING_GATE_ENV`, avec
  le même contrat : lu une fois, et dit au démarrage s'il est désarmé.
  L'ouverture revient à Vincent.
  → Tenu **par héritage, sans ligne neuve** : `MIKA_QA_PASS_EVIDENCE_GATE`
  (`guards.rs:4683`), son lecteur `pass_evidence_gate_is_armed`, son `OnceLock`
  `pass_evidence_gate_armed()` et sa ligne de démarrage
  `log_pass_evidence_gate_state()` existent depuis le vol 1 et couvrent les deux
  classes du membre. Le ticket demande qu'un interrupteur suive ce patron, pas
  qu'un second soit créé : un second interrupteur fractionnerait la population en
  deux sous-populations armables séparément, ce qui n'a pas de sens pour deux
  exigences que le même prompt déclare obligatoires au même paragraphe. V6
  exerce la branche désarmée sur le motif neuf.
- **AC5.** Sonde après ouverture de l'interrupteur : sur les revues
  `mika-platform-qa` de 7 jours, le compte des `pass` sans ligne `PIPELINE:` est
  écrit ici, avec la requête qui l'a produit. La cible est 0. Le compte des tours
  QA morts sans revue sur la même fenêtre est écrit à côté.
  → **Structurellement post-merge, et ce vol ne peut pas le relever non plus** :
  l'ouverture est l'acte de Vincent (AC4), et la mesure du § 2 établit que le
  régime n'a pas changé depuis le merge du vol 1. Ce que ce vol livre, et qui est
  tout ce qu'un plan peut livrer avant l'ouverture :
  1. **la requête**, complète, exécutable, pour les deux motifs (§ 2) ;
  2. **la ligne de base avant ouverture, mesurée ce jour** sur la fenêtre de 7
     jours exacte de l'AC : **86 `pass` sur 57 PR mergées, 86 sans ligne
     `NEGATIVE-TEST:` ancrée** (mika 68/68, claude-pilot 18/18) ;
  3. **le compte des tours QA morts sans revue sur la même fenêtre : 0** — aucune
     PR mergée des deux dépôts sans revue `mika-platform-qa`, avec sa requête. Ce
     0 est la contrepartie du risque de refus, et c'est le nombre qui doit rester
     0 après ouverture : s'il monte, le refus tue des tours au lieu de corriger
     des corps.
  Le relevé **après** ouverture est porté par la ligne `Tracked in:` du § 10, avec
  le bras de contradiction dont D4 dit qu'il dépend de ce chiffre.

## 9. Fire-Disposition

Ce vol livre des détecteurs : le terme de garde (chemin de succès « aucune
violation trouvée »), le scan de source étendu, et la ligne de lint de jeton
canonique. Option retenue : **(b) livrer désarmé**, exactement comme le vol 1 et
par le même mécanisme.

- **Le désarmement, et il est hérité.** `MIKA_QA_PASS_EVIDENCE_GATE` est absent
  par défaut, donc `pass_evidence_gate_is_armed(None) == false`, donc le membre
  `pass` sort en `Ok(())` **avant** la lecture du corps. Le terme neuf vit à
  l'intérieur de ce membre : il est désarmé par construction, sans qu'une seule
  ligne de ce vol le désarme. C'est l'équivalent du `#[ignore]` que la doctrine
  nomme, au niveau du produit. Les 260 verdicts du relevé traverseraient
  inchangés sur un binaire livré.
- **Pourquoi pas (a), l'allowlist.** Les violations existantes ne sont pas des
  lignes de code mais des **corps de revue déjà postés sur GitHub** : rien au
  dépôt à exempter nommément, et une entrée grep-visible n'aurait rien à
  désigner. Les corps mesurés entrent au contraire au dépôt comme **fixtures**
  (T1-T3 hérités, T6 neuf), ce qui est l'inverse d'une exemption : ils y sont pour
  que le garde refuse.
- **Le détecteur de jeton canonique, lui, atterrit armé — et c'est voulu.** La
  ligne de `canonical-tokens.tsv` est ajoutée **au même commit** que le const, donc
  `scripts/check-canonical-tokens.sh` et
  `mika2201_every_match_site_is_declared` sont verts à l'atterrissage sans aucune
  exception. Un lint de déclaration n'a pas de population existante à exempter :
  il n'y a qu'un site, et il est déclaré.
- **Les tests ne sont pas désarmés.** Ils passent le bit en **paramètre** à
  `validate_pipeline_block_grounding_gated` (patron en place), donc les deux
  branches sont exercées : V4/V5 armé, V6 désarmé. Le détecteur est éteint en
  production et vert en CI.
- **Le suivi pour l'armer.** L'ouverture est l'acte de Vincent (AC4), la sonde
  AC5 la mesure, et le bras de contradiction de D4 attend son chiffre. Les trois
  sont portés par le ticket de la ligne `Tracked in:` du § 10.

## 10. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/evidence/guards.rs` (bandeau de mesure, `NEGATIVE_TEST_LINE_PREFIX`, `line_prefix_present` + les deux prédicats re-exprimés, variant, conjonction ordonnée) | 95 |
| tests Layer A (`mod mika2961` — 3 fixtures neuves, T4′/T5′ amendés, T6-T10) | 118 |
| `crates/mika-agent/src/skills/builtin_handlers.rs` (6<sup>e</sup> bras, `reason`, remedy du motif, commentaire de divergence) | 45 |
| tests Layer B (`mod mika2961_pass_evidence` — V4, V5, V6, V7, extension du scan V9) | 60 |
| `scripts/canonical-tokens.tsv` (une ligne) | 1 |

Total estimé : 319 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient sous la borne, et pourquoi sa marge est délibérément plus
large que celle du vol 1.** Le vol 1 a estimé 408 lignes et en a livré **1 201**
(`guards.rs` 546, `builtin_handlers.rs` 645, `server/mod.rs` 10) — un facteur 2,9.
Il a survécu, mais sa marge déclarée était de 5 % et ce n'est pas elle qui l'a
sauvé : c'est que le travail s'est terminé avant la fin de pipeline. Reprendre une
marge de 5 % sur ce vol serait répéter une estimation qu'une mesure vient de
démentir. Ce vol se donne **110 lignes de marge, soit 26 %**, et ce qui rend ce
chiffre soutenable est que le vol 1 a payé une fois pour toutes les postes qui
l'ont fait déborder :

- **la plomberie entière est héritée, et le plan le vérifie poste par poste :**
  aucun `validate_*` neuf, aucun wrapper, aucun `enum` neuf, aucun helper
  d'audit, **aucune variable d'environnement, aucun `OnceLock`, aucune ligne de
  démarrage, aucun nom d'événement** (D7). `server/mod.rs` n'est pas touché. Le
  vol 1 a créé tout cela ; ce vol ajoute un const, un variant, un `if` et un bras
  de `match` ;
- **D2 (l'ordre des termes) supprime trois éditions de fixtures mesurées et les
  tests qui les portent** — ~40 lignes qui n'existent pas, et un risque de
  régression qui n'existe pas non plus ;
- **D4 (ni forme ni contradiction) retire ~75 lignes** : un troisième variant,
  son bras de `match`, son remedy propre, la table des quatre dispositions et sa
  batterie ;
- **D6 (le prédicat factorisé) est le seul poste qui *ajoute* du net hors
  tests** — ~12 lignes pour le helper, contre ~20 qu'une duplication du prédicat
  aurait coûtées, et il retire la possibilité que les deux motifs divergent ;
- **le renommage, ~10 lignes plus un diff à travers deux modules de test, est
  hors de ce vol** par la contrainte de périmètre (§ 0), pas par le budget.

Le poste le plus lourd reste les tests, et il est borné **par énumération** : sept
cas nommés au § 5.2, sur trois fixtures de 8 à 14 lignes, au gabarit mesuré des
fixtures `MEASURED_*` du vol 1 (`guards.rs:10026-10095`).

**Ordre de commit du vol :** (1) `guards.rs` — `line_prefix_present`, les deux
prédicats, le const, le variant ; (2) la conjonction dans le classifieur ;
(3) `scripts/canonical-tokens.tsv` + V8 vert ; (4) tests Layer A, T4′/T5′ amendés
d'abord (la suite est rouge entre (2) et (4) : c'est voulu, et c'est pourquoi
l'étape 4 suit immédiatement) ; (5) `builtin_handlers.rs` — bras, remedy,
commentaire de divergence ; (6) tests Layer B et scan ; (7) docs.

**Le plancher du vol est l'étape 6.** En deçà, le garde est câblé sans contrôle de
câblage : AC2 n'est pas tenu « dans le même appel », et la classe mika#2158 n'est
pas fermée sur le nouveau préfixe — un wrapper non gaté figeant le second terme
passerait une suite verte. Si la pression de tours se fait sentir avant l'étape 7,
c'est la **documentation** qui est reportée au suivi, jamais l'étape 6.

Tracked in: ticket de suivi à ouvrir après le merge de ce vol — (a) le relevé de
la sonde AC5 **après** ouverture de `MIKA_QA_PASS_EVIDENCE_GATE` par Vincent,
pour les deux motifs, avec le compte des tours QA morts sans revue à côté
(ligne de base de ce plan : 86/86 et 0) ; (b) le bras de contradiction
`pass` + `NEGATIVE-TEST: missing` et la validation des quatre dispositions, dont
le § 4.4 dit qu'ils se dimensionnent sur le chiffre de (a) ; (c) le renommage
`validate_pipeline_block_grounding` → `validate_pipeline_evidence`, reporté par la
contrainte de périmètre du § 0 et à reprendre une fois mika#2966 mergée.
