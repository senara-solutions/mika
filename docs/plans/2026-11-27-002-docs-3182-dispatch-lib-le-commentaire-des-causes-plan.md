# mika#3182 — le commentaire des causes de halte du groom dit l'inverse du code d'à côté

## Le fait, mesuré sur la branche

`skills/bundled/_shared/dispatch-lib.sh`, lignes 11316 à 11323, telles qu'elles
sont sur `origin/main` (`b60bd55d`) :

```
# AUCUNE CONSTANTE RUST NE LEUR RÉPOND, et c'est mesuré plutôt que supposé : le
# parseur `groom_halt_cause` (`task_engine/dispatcher.rs`) lit une cause
# QUELCONQUE — son test `some_future_cause` le pin — et les quatre scans
# `mika2641_*` de `canonical_tokens.rs` sont ancrés nommément sur
# `GROOM_HALT_CAUSE_LINE_PREFIX` et `GROOM_HALT_CAUSE_ARCHITECT_UNREADABLE`, donc
# ils ne demandent rien aux valeurs voisines. Les deux causes voyagent dans la
# même colonne `tasks.result` que le `Outcome: PLAN_GROOMED` d'un groom sain : le
# recensement a son dénominateur sans ligne de sonde neuve (mika#3050).
```

Les deux affirmations en majuscules étaient vraies le 26/11, quand le vol 2 de
mika#3118 (PR mika#3175) les a écrites. mika#3181 (PR #3181, mergée en
`2727e477`) les a périmées le lendemain, et son corps le signale sous
« Résidu connu, nommé plutôt que corrigé en silence » — sans le corriger, son
ticket interdisant de toucher `skills/bundled/_shared/`.

Ce qui est vrai aujourd'hui, vérifié fichier par fichier sur la branche :

| affirmation du bloc | état réel | preuve |
|---|---|---|
| « aucune constante Rust ne leur répond » | **faux** — deux constantes | `crates/mika-agent/src/task_engine/dispatcher.rs:6568` et `:6577` |
| « rien ne lit ces causes côté moteur » (implicite) | **faux** — un lecteur, par ÉGALITÉ sur chacune | `dispatcher.rs:6762` `try_report_groom_plan_halt`, test d'égalité aux l. 6776-6780, ligne `audit_events` sous `GROOM_PLAN_HALT_TOOL` = `groom_plan_halt_declared` (`:6608`) |
| « les scans `mika2641_*` ne demandent rien aux valeurs voisines » | **faux** — deux assertions nommées, à l'octet | `canonical_tokens.rs:6746-6763`, dans `mika2641_the_halt_cause_wire_format_is_synchronised_shell_to_rust` |

Les deux assertions neuves comparent par `src.lines().any(|l| l == expect_…)` :
la ligne shell doit être **exactement**
`GROOM_HALT_CAUSE_PLAN_HALT_DECLARED="plan_halt_declared"` (resp.
`…_UNREADABLE="plan_halt_unreadable"`), affectation en colonne 0, guillemets
compris. L'égalité est donc à l'octet, pas au « contient ».

## Pourquoi c'est un défaut et pas une imprécision

Ce bloc est la consigne qu'un pilote lit **avant** de toucher
`GROOM_HALT_CAUSE_PLAN_HALT_DECLARED` ou `…_UNREADABLE`. Tel qu'il est écrit, il
autorise explicitement à changer une valeur côté shell sans toucher au Rust —
« ils ne demandent rien aux valeurs voisines ». Le pilote qui le croit casse la
CI à la ligne suivante, et le message d'erreur qu'il recevra dira le contraire de
ce que le commentaire lui avait promis.

Le coût est asymétrique dans le bon sens : ici la divergence rougit. Mais le bloc
existe précisément pour épargner ce tour-là, et un commentaire qui contredit le
code de dix lignes plus bas est une consigne qu'on désapprend à lire.

## Une inexactitude du corps du ticket, nommée plutôt que recopiée

Le corps de mika#3182 écrit que mika#3181 a ajouté les deux constantes « dans le
module des littéraux canoniques de mika-agent ». Ce n'est pas le découpage réel,
et AC1 demande de nommer « le module qui les porte » :

* les **constantes** vivent dans `crates/mika-agent/src/task_engine/dispatcher.rs`
  (`pub(crate) const`, l. 6568 et 6577), à côté de leurs sœurs
  `GROOM_HALT_CAUSE_LINE_PREFIX` et `GROOM_HALT_CAUSE_ARCHITECT_UNREADABLE` ;
* `crates/mika-agent/src/canonical_tokens.rs` porte les **scans** `mika2641_*`,
  qui les importent depuis `dispatcher.rs` (`use crate::task_engine::dispatcher::{…}`,
  l. 6693-6696).

La rédaction cible nomme donc `task_engine/dispatcher.rs` comme porteur des
constantes **et** du lecteur, et `canonical_tokens.rs` comme porteur des
assertions. C'est ce que le pilote doit pouvoir `grep` sans se tromper de fichier.

## Ce que livre ce vol

### 1. La réécriture du bloc (lignes 11316-11323)

Rédaction cible — les quatre faits qu'AC1 exige, dans l'ordre où un pilote en a
besoin (ce qui existe → qui le lit → ce qui rougit s'il dévie) :

```
# DEUX CONSTANTES RUST LEUR RÉPONDENT DEPUIS mika#3181, et l'égalité est exigée
# À L'OCTET. `task_engine/dispatcher.rs` porte les jumelles
# `GROOM_HALT_CAUSE_PLAN_HALT_DECLARED` et
# `GROOM_HALT_CAUSE_PLAN_HALT_UNREADABLE`, et son lecteur
# `try_report_groom_plan_halt` les compare par ÉGALITÉ — jamais par préfixe
# `plan_halt_` — pour écrire une ligne `audit_events` sous le `tool_name`
# `groom_plan_halt_declared`, la cause en `after_value` (mika#3176). Le parseur
# `groom_halt_cause`, lui, lit toujours une cause QUELCONQUE : son test
# `some_future_cause` le pin.
#
# DONC CHANGER UNE VALEUR ICI SANS TOUCHER AU RUST FAIT ROUGIR LA CI : deux des
# assertions de
# `mika2641_the_halt_cause_wire_format_is_synchronised_shell_to_rust`
# (`canonical_tokens.rs`) cherchent les DEUX lignes d'affectation ci-dessous
# OCTET POUR OCTET — affectation en colonne 0, guillemets compris — et c'est
# leur message qui dit quoi réaligner. Les deux causes voyagent dans la même
# colonne `tasks.result` que le `Outcome: PLAN_GROOMED` d'un groom sain : le
# recensement a son dénominateur sans ligne de sonde neuve (mika#3050).
```

Ce qui est **conservé intact**, parce qu'AC1 l'exige nommément et que rien ne l'a
périmé :

* le paragraphe `# DEUX VALEURS ET JAMAIS UNE (mika#2205)` (l. 11311-11314) — pas
  un octet ;
* l'en-tête mika#3118 et son récit mesuré du 19/11 (l. 11301-11309) ;
* la phrase finale sur `tasks.result` / `Outcome: PLAN_GROOMED` / mika#3050, qui
  passe telle quelle dans le second paragraphe ci-dessus ;
* les deux commentaires individuels des l. 11325-11326 et 11329-11333, qui
  décrivent chaque cause séparément et restent exacts.

### 2. Les deux lignes d'affectation : zéro octet changé

`GROOM_HALT_CAUSE_PLAN_HALT_DECLARED="plan_halt_declared"` (l. 11327) et
`GROOM_HALT_CAUSE_PLAN_HALT_UNREADABLE="plan_halt_unreadable"` (l. 11334) ne sont
pas touchées. C'est AC2, et c'est aussi la condition pour que les deux assertions
de `canonical_tokens.rs` restent vertes : elles cherchent la ligne entière.

### 3. Le document compound

`docs/solutions/best-practices/` — une page nommant la classe : *un commentaire
qui déclare l'absence d'un lecteur se périme le jour où le lecteur naît, et rien
ne le dit*. Les deux faits à y porter :

* le producteur de la péremption (mika#3181) **savait** et l'a écrit dans son
  corps de PR plutôt que dans le fichier, parce que son périmètre le lui
  interdisait. Le résidu a donc vécu dans un corps de PR, qu'aucun pilote ne lit
  avant d'éditer un fichier ;
* la forme qui ne se périme pas : ne pas écrire « rien ne lit X », écrire « voici
  ce qui lit X aujourd'hui, et voici le scan qui rougit ». Une affirmation
  d'absence n'a pas de gardien ; une affirmation de présence nomme le sien.

## Ce que ce vol ne corrige PAS

* **Le parseur `groom_halt_cause`, `_iterate_groom_loop`, `_escalate_groom`, et
  les constantes elles-mêmes** — hors périmètre par le corps du ticket. Aucune
  ligne exécutable ne change (AC2).
* **`skills/bundled/_shared/test-dispatch-lib.sh`** — gelé par mika#3179
  (`scripts/check-dispatch-lib-aggregate-frozen.sh`, plafond statique). AC3. Ses
  assertions mika#3118 (l. 6082-6096, 6190, 6208) portent sur le `case` de la
  porte et sur les lignes `cause_line=`, pas sur ce bloc de commentaire : rien à
  y réaligner.
* **`docs/plans/2026-11-26-001-fix-3118-…-vol2-plan.md:297`**, qui porte la même
  affirmation périmée (« aucune constante Rust »). C'est un **plan archivé**, pas
  une consigne vivante au bord du code : le réécrire falsifierait le dossier de
  mika#3118. Il est nommé dans le corps de PR, non corrigé — conformément à la
  clause du ticket sur les autres commentaires périmés du même ordre.
* **Aucun détecteur n'est livré.** Ce vol est de la prose en commentaire shell :
  aucun test, aucune assertion, aucune garde CI, aucun scan. La gate
  Fire-Disposition (mika#2306) est donc **N/A**, et la section n'est pas inventée.

## Ordre de commit

Un seul commit de travail suffit ; le plan est déjà sur la branche.

1. `docs(3182): le bloc des causes de halte nomme ses deux lecteurs Rust` —
   `skills/bundled/_shared/dispatch-lib.sh` (commentaire seul) + le document
   compound.

## Tests

| quoi | commande | ce que ça prouve |
|---|---|---|
| le diff ne touche rien d'exécutable | `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` — inspection : toute ligne ajoutée ou retirée commence par `#` | AC2, première moitié |
| les deux affectations sont intactes | `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh \| grep -c 'GROOM_HALT_CAUSE_PLAN_HALT'` → compte les seules occurrences en commentaire ; les lignes `=` n'apparaissent pas au diff | AC2, seconde moitié |
| la suite shell | `make test-dispatch-lib` | AC4 — la porte et son `case` sont inchangés |
| la synchronisation shell ↔ Rust | `cargo test -p mika-agent --lib mika2641` | AC4 — les deux assertions à l'octet passent, donc les littéraux shell n'ont pas bougé |
| le pipeline | `bash scripts/verify-pipeline.sh origin/main` | AC4 |

**Un faux signal attendu, et sa date de péremption.** Sur le commit du plan seul,
`verify-pipeline.sh` rend
`REJECT: docs-only PR: plan/solution present but no source changes` (mesuré sur
cette branche avant le commit du plan). Ce n'est pas une exemption à demander :
`SOURCE_BUCKET` est « tout sauf `docs/`, `.github/`, `.claude/worktrees/` »
(`scripts/verify-pipeline.sh:198-202`), donc `skills/bundled/_shared/dispatch-lib.sh`
y entre dès le commit de travail, commentaire ou pas, et la gate se résout d'elle-même.
Le pilote ne lance donc `verify-pipeline.sh` qu'**après** le commit 1. Ni label
`documentation`, ni `pipeline-exempt`, ni trailer `Pipeline-Exempt:` n'ont lieu
d'être ici.
| aucun autre fichier | `git diff --name-only origin/main` → exactement : le plan, le doc compound, `dispatch-lib.sh` | AC3 |

Aucun test neuf. Les deux assertions de mika#3176 **sont déjà** le détecteur de
ce défaut-ci sur sa moitié mécanique (la divergence d'octets) ; ce vol corrige la
moitié que seul un humain lit, et qu'aucun scan ne peut atteindre.

## Risques

* **Le seul risque réel est de toucher une ligne exécutable en éditant 14 lignes
  de commentaire au milieu de 15 650.** Mitigation : l'édition est faite par
  remplacement de chaîne exacte sur les seules l. 11316-11323, et le diff est
  relu ligne à ligne avant le commit (première ligne du tableau de tests).
* **Plafond de taille du fichier.** `dispatch-lib.sh` fait 916 272 octets pour un
  `LARGE_FILE_LIMIT` de 1 048 576 (`scripts/check-secrets.sh`, allowlist vide).
  Le vol ajoute ~600 octets : marge restante ~131 KB. Noté parce que le fichier
  est à 87 % du plafond, pas parce que ce vol l'approche.
* **Péremption future.** La rédaction cible nomme un lecteur *existant* plutôt
  qu'une absence, donc elle se périme si ce lecteur est supprimé — un événement
  qui, lui, fait rougir `cargo test -p mika-agent --lib mika2641`. C'est le point
  du document compound.

## Definition of Done

* [ ] Les l. 11316-11323 de `dispatch-lib.sh` sont réécrites selon la rédaction
      cible ci-dessus : deux constantes nommées, `task_engine/dispatcher.rs` comme
      porteur, `try_report_groom_plan_halt` comme lecteur, le `tool_name`
      `groom_plan_halt_declared`, et l'égalité à l'octet exigée par les deux
      assertions de `mika2641_the_halt_cause_wire_format_is_synchronised_shell_to_rust`.
* [ ] Le paragraphe « DEUX VALEURS ET JAMAIS UNE » (mika#2205) et la phrase
      `tasks.result` / mika#3050 sont présents dans le bloc final.
* [ ] `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` ne montre
      aucune ligne ajoutée ou retirée qui ne commence par `#`.
* [ ] Les deux lignes `GROOM_HALT_CAUSE_PLAN_HALT_*=` sont identiques à l'octet à
      celles d'`origin/main`.
* [ ] `git diff --name-only origin/main` ne liste que le plan, le document
      compound et `dispatch-lib.sh`.
* [ ] `make test-dispatch-lib` vert.
* [ ] `cargo test -p mika-agent --lib mika2641` vert.
* [ ] `bash scripts/verify-pipeline.sh origin/main` passe.
* [ ] Le document compound existe sous `docs/solutions/best-practices/` avec son
      frontmatter YAML (`module`, `tags`, `problem_type`).
* [ ] Le corps de PR nomme le résidu laissé en place
      (`docs/plans/2026-11-26-001-…-vol2-plan.md:297`) sans le corriger, et porte
      la ligne `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

Transcrits du corps de mika#3182.

* **AC1.** Le bloc dit ce qui est vrai sur `origin/main` : il nomme les deux
  constantes Rust, le module qui les porte, et le lecteur de
  `task_engine/dispatcher.rs`. Il dit aussi que les deux assertions des scans
  `mika2641_*` exigent l'égalité à l'octet. Il garde le paragraphe « DEUX VALEURS
  ET JAMAIS UNE » (mika#2205) et la mention de `tasks.result` (mika#3050).
* **AC2.** Seules des lignes de commentaire changent.
  `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` ne touche
  aucune ligne exécutable. Les deux affectations `GROOM_HALT_CAUSE_*=` sont
  identiques à l'octet.
* **AC3.** Aucun autre fichier n'est modifié, en dehors du plan et du document
  compound. En particulier, `skills/bundled/_shared/test-dispatch-lib.sh` ne
  change pas : il est gelé par mika#3179.
* **AC4.** `make test-dispatch-lib` est vert. Le filtre
  `cargo test -p mika-agent --lib mika2641` est vert, et
  `bash scripts/verify-pipeline.sh origin/main` passe.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` (commentaire seul, 8 lignes remplacées par ~16) | 16 |

Total estimé : 16 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Un seul vol, et il tient de très loin : 16 lignes contre une borne de 429, et zéro
ligne exécutable. Le document compound ne compte pas (il vit sous `docs/`), le
plan non plus. Le coût en tours est dominé par la fin de pipeline — trois suites à
lancer, dont `cargo test -p mika-agent --lib` qui compile le crate — et non par la
rédaction.
