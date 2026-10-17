---
title: "feat(webhooks): pré-filtre phase 4 — le lecteur du registre, vol 2 : une commande rend la mesure 3, au lieu de trois tuyaux dont un lit une variable qui n'existe pas (mika#2680)"
type: feat
status: active
date: 2026-10-16
issue: senara-solutions/mika#2680
---

# Phase 4, vol 2 — `measure-webhook-turn-cost` : le lecteur

## Pourquoi

Le vol 1 (#2705, mergé en `185362cc`) a livré **le champ** : depuis le
2026-10-16 13:01Z, chaque tour webhook facturé émet
`webhook_turn_charged{trigger, text_touched, candidate, prefilter_enabled,
request_id}`, et chaque tour écarté émet déjà `webhook_prefilter_skipped{class,
target, request_id}`. Le registre est donc servi.

Il reste sans **lecteur**. Ce que la doc compound du module livre aujourd'hui
est la sonde AC5 : trois tuyaux séparés — une requête SQL sur `llm_calls`, un
`grep | jq | sort | uniq -c` sur les tours payés, un troisième sur les écarts —
dont l'opérateur doit faire la jointure de tête. C'est exactement la forme que
`scripts/measure-empty-turns` a été écrit pour remplacer (son propre en-tête :
*« l'opérateur reçoit un tuyau et doit inventer la règle sur le moment »*).

Et cette fois le tuyau est pire que vague : il est **inerte**. Il lit
`"$MIKA_LOG_FILE"`, variable qui n'est déclarée **nulle part** — zéro occurrence
dans `crates/`, absente de `.env.example`, absente de
`docs/operator/environment-variables.md`. Le nom réel est
`MIKA_SPIRIT_LOG_FILE`. Non définie, `grep motif ""` lit l'entrée standard : la
sonde ne rend rien, et ne dit pas pourquoi. C'est la classe mika#2205 dans
l'instrument même qui doit fermer le critère d'acceptation du ticket.

Ce vol livre la commande, et **n'implémente aucune classe de filtre** — pour la
raison que le ticket écrit lui-même :

> Avant toute implémentation, refaire cette mesure sur une fenêtre comparable
> **après** le déploiement de #2679, pour retirer ce que (b) couvre déjà.

## Ce que le groom a établi

**1. La moitié « payé » du registre n'existe que dans les journaux.** Choix
délibéré du vol 1, consigné dans la doc compound : `emit_turn_charged` n'écrit
**aucune** ligne d'audit, parce que `audit_events` est réservé aux décisions du
moteur et que `webhook_prefilter_skipped` y garde son monopole. Conséquence pour
le lecteur : son corpus est `$MIKA_SPIRIT_LOG_FILE`, et **pas la base**. C'est
un gain, pas une contrainte — la sonde SQL de la doc compound lit `llm_calls`,
table gatée par `MIKA_STORE_LLM_CALLS`, tandis que `turn_usage` est dégaté et
porte les mêmes `input_tokens`. Le lecteur mesure donc sur une machine où le
stockage des appels est coupé, ce que la sonde actuelle ne peut pas.

**2. Aucun pilote dispatché ne peut produire la mesure 3.** Re-vérifié dans ce
worktree, aujourd'hui : `~/.mika/data/` ne contient que `pilot-transcripts/`
(pas de `mika.db`), `~/.mika/logs` est absent, `MIKA_SPIRIT_LOG_FILE` n'est pas
définie, et `/var/log/mika*` n'existe pas. Le constat 3 du vol 1 tient donc
inchangé : **la mesure est un geste d'orchestrateur sur l'hôte ; le dépôt ne
peut livrer que l'instrument.** Un plan qui demande des chiffres à un implement
le fait mourir ou fabriquer.

C'est le point où ce plan s'écarte du § *Vol 2* du corps du ticket, qui demande
au même vol de refaire la mesure **puis** d'implémenter (e), (f) et (g). Les
deux moitiés sont hors d'atteinte d'un seul vol, pour deux raisons disjointes :
la mesure n'est pas prenable depuis un worktree (ce constat), et trois classes à
250–400 lignes chacune — chiffrage du ticket — passent la borne par vol d'un
facteur 2 à 3 avant même la marge. Le vol livre l'instrument ; le vol 3 mesure
puis implémente, une classe par vol.

**3. Ce que le registre tranche seul, et ce qu'il ne tranche pas.** Établi champ
par champ, pour que le vol 3 n'ait pas à le redécouvrir :

- **(g) est décidable en entier.** `trigger = check_suite_pr_branch` et
  `agent_id = mika-qa` sont deux champs de la **même** ligne. Le coût de (g)
  sort du lecteur sans aucune inspection.
- **(e) et (f) sont bornées par le haut, pas décidées.** La taxonomie du vol 1
  rend `pr_closed` pour une PR mergée **comme** pour une PR fermée (le gateway
  émet la même forme et ne dit `Merged:` qu'en ligne suivante), et « sans tâche
  de boucle active » n'est sur aucun champ. Le lecteur rend donc le coût du
  *déclencheur*, qui majore la classe — et il rend la **liste des
  `request_id`**, ce qui fait de l'inspection un relevé borné et nommé au lieu
  d'une pêche.
- **La classe (b) s'écarte par construction, et c'est à attester.** Depuis le
  déploiement de `e14ace7f`, un tour couvert par (b) part en
  `webhook_prefilter_skipped` et n'apparaît donc **jamais** côté payé : « retirer
  ce que (b) couvre déjà » est satisfait par la forme du corpus, pas par une
  soustraction. Mais une absence doit être **attestée**, pas supposée — d'où la
  lecture des deux moitiés dans un seul rapport, la moitié écarts donnant le
  compte par classe qui prouve que (b) a bien tiré.

**4. Le coût d'un tour est une somme, pas une ligne.** `emit_turn_usage` a
quatre sites d'appel et écrit une ligne **par étape**, plus la ligne de
continuation (`step = u32::MAX`). Les mesures 1 et 2 du ticket sommaient
`llm_calls` par session ; le lecteur somme `input_tokens` par `trace_id`. Un
lecteur qui prendrait une ligne par tour sous-compterait d'un facteur égal au
nombre d'étapes — et le sous-comptage serait le plus fort précisément sur les
tours les plus chers, ceux que le ticket poursuit.

**5. L'instrument ne demande aucun déploiement.** Ce vol ne touche **aucune
ligne de Rust**. Le champ est servi depuis 13:01Z ; dès le merge, le lecteur
s'applique à un corpus déjà écrit. La chaîne `rebuild → seed → read` (mika#2340)
est franchie par le vol 1, pas par celui-ci.

## Décision

1. **Un corpus, un passage en flux, trois événements.**
   `scripts/measure-webhook-turn-cost` lit `$MIKA_SPIRIT_LOG_FILE` (surchargeable
   par `--log-file`) et n'ouvre **jamais** la base. `grep -F` narrow sur les trois
   noms d'événement, `jq -R` ligne à ligne (une ligne non-JSON ne doit pas
   emporter la mesure), projection en TSV, agrégation séparée. Le journal se
   mesure en gigaoctets : rien n'est slurpé avant la projection.

2. **La jointure est `trace_id`, l'unité est le tour, la valeur est une somme.**
   `webhook_turn_charged.request_id == turn_usage.trace_id` (les deux noms, une
   seule valeur : `req.request_id`). Pour chaque tour payé : `input_tokens`
   sommé sur **toutes** les lignes `turn_usage` du `trace_id`, lignes de
   continuation comprises, plus le compte d'appels. `cache_read_tokens` est
   sommé à part — un modèle de coût viendra, et l'écraser maintenant rendrait la
   fenêtre non comparable à celle qui le voudra.

3. **Deux sorties, comme `measure-empty-turns`.** `--out` : un objet JSON par
   tour payé, chronologique, portant `request_id`, `agent_id`, `trigger`,
   `text_touched`, `candidate` (`null` quand le champ était **absent**),
   `prefilter_enabled`, `input_tokens`, `cache_read_tokens`, `llm_calls`.
   `--summary` : un agrégat portant, par `agent_id × trigger`, le nombre de
   tours et la somme des tokens ; la moitié écarts par `class` ; les totaux ;
   `field_epoch` ; les drapeaux de fenêtre ; le seau non attribué.

4. **Les bornes sont refusées plutôt que mal répondues.** `--since` / `--until`
   prennent `YYYY-MM-DD` (journée entière, UTC) ou un instant
   `YYYY-MM-DDTHH:MM:SSZ`, normalisés par la forme exacte de
   `scripts/pr-origin-report.sh::_norm_instant`. Tout le reste est refusé
   (`--since hier` compris) : une fenêtre qui n'est pas celle qu'on a demandée
   est un mensonge d'instrument. La comparaison se fait sur le champ `timestamp`
   des lignes, ISO-8601 UTC à la milliseconde, donc en ordre lexicographique.

5. **Fail-safe toujours vers l'indéterminé ; deux codes de sortie.** Une ligne
   illisible, un champ absent, un `trace_id` manquant : rien de tout cela ne
   devient un tour attribué. `exit 0` = le corpus a été lu (zéro tour payé est
   un **résultat**) ; `exit 2` = rien n'a été mesuré — pas de fichier, illisible,
   `jq` absent, aucune ligne `webhook_turn_charged`, ou un `--agent` qui n'a rien
   apparié. Jamais 0 dans ces cas : un scan inerte en silence se lit exactement
   comme un scan qui n'a rien trouvé à dire (mika#2205). `unparseable_lines` est
   compté **avant** le filtre `--agent` et **ne conjugue pas** la décision
   « rien mesuré » — c'est le piège qui rendait `exit 2` inatteignable chez son
   aîné, et sa propre suite l'a trouvé.

6. **L'époque du champ se lit sur le corpus, jamais sur une date codée.**
   `field_epoch` = l'horodatage du plus ancien `webhook_turn_charged` lu. Quand
   `--since` le précède, l'agrégat porte `window_precedes_field_epoch: true` et
   le seau des `turn_usage` sans ligne payée est rendu sous le nom
   `unattributed_undetermined` au lieu d'être attribué : dans cette zone, « pas
   de ligne payée » ne distingue pas un tour Telegram d'un binaire antérieur au
   champ (contrôle positif du vol 1, AC7). Au-delà de l'époque, le seau garde son
   nom `unattributed` et se lit comme « hors registre par construction ». Aucune
   date du 2026-10-16 n'est écrite dans le script.

7. **La prose va dans `docs/eval/mika-2680/README.md`, pas dans l'en-tête du
   script.** Écart assumé au style de `measure-empty-turns`, dont l'en-tête fait
   130 lignes : la borne de ce vol est en **lignes de code**, et 130 lignes de
   prose en consommeraient 30 % pour un texte que le README porte mieux — c'est
   là que l'opérateur lit le protocole, pas dans un `--help`. L'en-tête du script
   est borné à 60 lignes et ne garde que ce qui défend le code adjacent : le
   prédicat ordonné, la forme de fil, les deux codes de sortie.

8. **La batterie est armée en CI.** `make test-measure-webhook-turn-cost`, et le
   job `Check` l'appelle — contrairement à `test-measure-empty-turns`, resté
   cible locale. L'argument est celui de son propre en-tête : *« un geste
   reproductible qui n'a jamais été reproduit est un geste que personne n'a
   vérifié »*, et son premier lancement a trouvé un script qui mourait sur sa
   toute première invocation. Fixtures synthétiques et gelées, pas échantillonnées
   du corpus vivant : une suite qui lit le journal de production change de verdict
   quand le journal grossit, et ne tourne pas sur une machine qui n'a jamais
   dispatché.

9. **Aucune classe de pré-filtre, aucun champ nouveau, aucun Rust.** Ni (e), ni
   (f), ni (g), ni (h). Ajouter un champ pour décider (e) ou (f) sur le fil
   serait un vol de la forme du vol 1 — il rouvrirait la chaîne
   `rebuild → deploy → attendre une fenêtre`, donc repousserait la mesure
   d'autant. Le vol 3 tranchera, mesure en main, si une inspection bornée par
   `request_id` suffit ou si un champ est dû.

## Découpage en vols

- **Vol 1 — le champ.** Livré, mergé en `185362cc`, servi depuis 13:01Z.
- **Vol 2 — le lecteur (CE plan).** Le script, sa batterie armée, le protocole
  de mesure, et la correction de la variable inerte de la sonde AC5. Mergeable
  seul, utile dès le merge (constat 5 : aucun déploiement requis).
- **Vol 3 — la mesure 3, puis une classe par vol.** Le geste d'orchestrateur sur
  l'hôte sur la fenêtre ≥ 4 h 30 du § *Vol 2* du ticket, l'artefact déposé sous
  `docs/eval/mika-2680/`, et **ensuite** les filtres que cette mesure confirme —
  (g) d'abord, décidable en entier par le registre (constat 3). C'est le vol 3
  qui ferme le critère d'acceptation de la mesure.

Ce vol porte `Refs #2680`, **pas** `Closes` : le critère d'acceptation reste
ouvert jusqu'à la mesure 3.

Tracked in: senara-solutions/mika#2680 — vol 3 (« la mesure 3, puis une classe par vol »)

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/measure-webhook-turn-cost` (en-tête borné à 60 lignes, décision 7) | 200 |
| `scripts/test-measure-webhook-turn-cost.sh` (contrôle positif + six négatifs) | 150 |
| `crates/mika-agent/tests/fixtures/mika2680/register_corpus.jsonl` | 24 |
| `Makefile` — cible `test-measure-webhook-turn-cost` | 3 |
| `.github/workflows/ci.yml` — étape dans le job `Check` | 4 |

Total estimé : 381 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 431 lignes estimées — (200 − 35 − 50) × 7,5 / 2

**Pourquoi un seul vol tient (règle 4).** 381 < 431, et la marge est tenue par
trois bornes posées dans le plan plutôt que par un espoir : aucun fichier Rust
n'est touché (donc ni `cargo` à faire converger, ni test d'intégration à
câbler), l'en-tête du script est borné à 60 lignes par la décision 7, et la
batterie est bornée à sept assertions par la Fire-Disposition ci-dessous. Les
deux gros volumes de prose — le protocole de mesure et la lecture des deux
moitiés — vont sous `docs/`, hors compte.

Le relevé du 2026-10-14 (commentaire 2 du ticket) note que le vol 1 a sous-compté
d'un facteur 3,8 : 345 estimées pour 1 314 réelles. Ce plan en tient compte en
ne laissant la taille dépendre d'aucune découverte à l'implémentation — les trois
noms d'événement, les formes de fil, la jointure et l'unité sont toutes établies
ci-dessus, fixture de production en main.

## Fichiers

- `scripts/measure-webhook-turn-cost` — **nouveau.** Le lecteur.
- `scripts/test-measure-webhook-turn-cost.sh` — **nouveau.** La batterie
  anti-vacuité.
- `crates/mika-agent/tests/fixtures/mika2680/register_corpus.jsonl` —
  **nouveau.** Corpus synthétique gelé : les trois événements, un tour à trois
  étapes, un tour sans `candidate`, un tour écarté, une ligne `turn_usage`
  orpheline, une ligne non-JSON, un tour hors fenêtre.
- `docs/eval/mika-2680/README.md` — **nouveau.** Le protocole de la mesure 3 :
  bornes de la fenêtre, commande exacte, lecture des deux moitiés, ce que le
  registre tranche et ce qu'il majore (constat 3), et la **halte** qui refuse de
  conclure quand `field_epoch` est postérieur à `--since` ou quand
  `prefilter_enabled` est faux sur la fenêtre. Emplacement de l'artefact du vol 3.
- `docs/solutions/best-practices/un-pre-filtre-de-webhook-se-place-apres-les-handlers-2026-10-07.md`
  — § *La sonde des tours payés* : `MIKA_LOG_FILE` → `MIKA_SPIRIT_LOG_FILE`
  (constat 4), et les trois tuyaux renvoient au lecteur, qui devient la forme
  canonique de la sonde AC5.
- `Makefile`, `.github/workflows/ci.yml` — la cible et son armement.

## Fire-Disposition

**Option (a) — exception nommée en allowlist, et l'allowlist est vide à la
livraison.**

Le détecteur livré est `scripts/test-measure-webhook-turn-cost.sh` : son chemin
de succès est « aucune divergence trouvée ». Il porte **exclusivement** sur du
code créé par ce vol (le lecteur) et sur des fixtures synthétiques créées par ce
vol — il n'existe donc, par construction, aucune violation préexistante à
excepter, et l'ensemble d'exceptions est **vide**.

Vide, pas absent : c'est l'emplacement où la prochaine infraction se déposera
nommée plutôt que tolérée (mika#2323). Trois conséquences d'implémentation :

- **Il atterrit ARMÉ, pas désarmé.** Aucun `#[ignore]`, aucune cible locale :
  l'étape est dans le job `Check` (décision 8). Un détecteur que CI n'appelle pas
  est l'option (b) sans l'avoir déclarée.
- **L'asymétrie est obligatoire, et les négatifs sont séparés.** Une batterie
  purement négative est satisfaite par un lecteur qui ne compte jamais rien ;
  une batterie purement positive, par un lecteur qui compte tout. D'où **un
  contrôle positif** (le tour à trois étapes : la somme vaut bien la somme) et
  **six négatifs, un par confusion, chacun sur sa propre entrée** — une
  conjonction de termes fail-safe ne se prouve pas en les invalidant tous
  ensemble (leçon mika#2277) :
  1. un `turn_usage` orphelin ne reçoit **pas** de `trigger` (seau non attribué) ;
  2. `candidate` absent sort `null`, jamais `"none"` ni `"0"` (motif mika#2331) ;
  3. un tour écarté ne produit **aucune** ligne côté payé ;
  4. corpus sans aucune ligne `webhook_turn_charged` ⇒ `exit 2` ;
  5. une borne mal formée est refusée, pas interprétée ;
  6. une ligne non-JSON est comptée `unparseable` et **n'autorise pas** à elle
     seule un `exit 0`.
  Sept assertions, pas plus : la borne de la batterie est posée ici (§ *Taille
  estimée*).
- **Aucune entrée dans une liste d'exclusion du lecteur.** Le seul terme de repli
  est `undetermined`, et il n'a pas de liste : il attrape ce qui ne se lit pas,
  et une classe qui s'y retrouverait est une **déclaration** à faire dans la
  taxonomie du vol 1, jamais une exemption.

## Definition of Done

- `scripts/measure-webhook-turn-cost` rend, en une commande et sans ouvrir la
  base, le coût par `agent_id × trigger` des tours webhook payés et le compte
  par `class` des tours écartés, sur une fenêtre bornée.
- Le coût d'un tour est la **somme** de `input_tokens` sur toutes les lignes
  `turn_usage` de son `trace_id` ; la mutation « une ligne par tour » est vue
  rouge par le contrôle positif et consignée dans le corps de la PR.
- `exit 2` sur tout corpus où rien n'a été mesuré ; `exit 0` quand le corpus a
  été lu, zéro tour payé compris.
- Bornes refusées plutôt que mal répondues ; `field_epoch` dérivé du corpus,
  aucune date codée.
- `make test-measure-webhook-turn-cost` vert, et l'étape présente dans le job
  `Check` de `ci.yml`.
- `docs/eval/mika-2680/README.md` porte le protocole de la mesure 3 et sa halte.
- La sonde AC5 de la doc compound nomme `MIKA_SPIRIT_LOG_FILE` et renvoie au
  lecteur.
- Aucune ligne de Rust modifiée ; `cargo test`, `cargo clippy`, les lints du
  dépôt et `scripts/verify-pipeline.sh origin/main` verts.
- PR : `Refs #2680`, vol 3 nommé par la ligne `Tracked in:`,
  `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

- [ ] **AC1** — pour un tour payé dont le `trace_id` porte trois lignes
  `turn_usage`, le lecteur rend **un** tour et `input_tokens` égal à la
  **somme** des trois. Un lecteur qui prendrait la première, la dernière ou la
  ligne de continuation seule est rouge.
- [ ] **AC2** — `candidate` sort `null` quand le champ était **absent** de la
  ligne, et jamais la chaîne `"none"`, `"0"` ni `"-"` (motif mika#2331). Les
  deux formes émises par `emit_turn_charged` — avec et sans le champ — sont dans
  la fixture.
- [ ] **AC3** — une ligne `turn_usage` sans `webhook_turn_charged` de même
  `trace_id` tombe dans le seau non attribué et ne reçoit **aucun** `trigger`.
  Quand `--since` précède `field_epoch`, ce seau est rendu sous le nom
  `unattributed_undetermined` (contrôle positif du vol 1, AC7).
- [ ] **AC4** — les deux moitiés sortent dans le même agrégat : coût par
  `agent_id × trigger` côté payé, compte par `class` côté écarté. C'est cette
  seconde moitié qui **atteste** que la classe (b) a tiré, au lieu de supposer
  son absence.
- [ ] **AC5** — `(g)` sort du lecteur sans inspection : le couple
  `trigger = check_suite_pr_branch` × `agent_id = mika-qa` est une entrée de
  l'agrégat, avec son coût.
- [ ] **AC6** — le JSONL par tour porte `request_id`, de sorte que l'inspection
  de `(e)` et `(f)` — que le registre majore sans décider (constat 3) — soit un
  relevé borné et nommé.
- [ ] **AC7** — `exit 2` quand rien n'a été mesuré : fichier absent, illisible,
  `jq` absent, aucune ligne `webhook_turn_charged`, `--agent` sans appariement.
  Une **unique** ligne non-JSON dans le corpus ne suffit **pas** à rendre
  `exit 0`.
- [ ] **AC8** — `--since hier` et `--since 10/16/2026` sont refusés avec un
  message qui nomme les deux formes acceptées ; `--since 2026-10-16` couvre la
  journée UTC entière.
- [ ] **AC9** — la sonde AC5 de la doc compound nomme `MIKA_SPIRIT_LOG_FILE` :
  plus aucune occurrence de `MIKA_LOG_FILE` dans ce fichier.
- [ ] **AC10** *(vol 3, suivi — transcrit du ticket)* — refaire la mesure sur
  une fenêtre comparable ≥ 4 h 30 **avec activité PR** après `e14ace7f`, pour
  retirer ce que la classe (b) couvre déjà, puis implémenter les filtres que
  cette mesure confirme. Geste d'orchestrateur sur l'hôte, pas un pilote
  dispatché (constat 2). **Hors périmètre du vol 2.**

## Hors périmètre

- **Implémenter une classe de pré-filtre** — (e), (f), (g), (h). Le ticket
  chiffre ; ce vol livre le lecteur du chiffrage (décision 9).
- **La mesure 3 elle-même** : geste d'orchestrateur, constat 2. Le vol dépose le
  protocole et l'emplacement de l'artefact, pas l'artefact.
- **Ajouter un champ au registre** pour décider `(e)` ou `(f)` sur le fil : vol
  de la forme du vol 1, qui rouvrirait la chaîne `rebuild → deploy → fenêtre`.
- **Un modèle de coût en monnaie.** Le lecteur somme des tokens, `cache_read`
  compté à part ; convertir demanderait une table de prix par modèle qui n'a pas
  de site de vérité dans ce dépôt.
- **Armer `test-measure-empty-turns` en CI.** Le même argument vaut pour lui,
  mais c'est son ticket, pas celui-ci.
- **Rerouter les constantes de préfixe locales des quatre handlers** vers
  `github_event_format` : refactor déjà écarté par le vol 1, sans rapport avec la
  mesure.
- **Le tour de revue QA**, légitime (dit par le ticket), et **la renotification
  opérateur à chaque retraitement DECISION-CORE**, nommée dans #2679.
- **Les tours non-webhook** (Telegram, silencieux, heartbeat) : hors registre par
  construction, et leur silence ne se lit pas comme un manque.
