---
title: Une lecture d'API n'est pas une compilation — exigez un fait moteur, pas une assertion
date: 2026-09-29
last_updated: 2026-09-29
category: best-practices
module: mika-agent/evidence/guards
problem_type: best_practice
component: dev-loop
severity: high
applies_when:
  - Une garde demande au modèle d'affirmer qu'il a vérifié quelque chose
  - Un verdict automatique porte une affirmation de compatibilité, de sûreté ou d'équivalence
  - On s'apprête à durcir l'exigence de citation d'une garde qui « n'a pas mordu »
tags:
  - guards
  - fabrication
  - dependabot
  - verification
related:
  - mika#2565
  - mika#2519
  - mika#1646
  - mika#2158
---

# Une lecture d'API n'est pas une compilation

## Le défaut

Le 2026-09-28, deux PR dependabot ont été approuvées par mika-qa avec un check
requis **rouge** derrière.

**mika#2560** (`sha2 0.10.9 → 0.11.0`) — trois `pass`, dont
`REASON: … API-compatible (Sha256/Digest trait stable) … BUILD VERIFICATION:
skipped (pipeline-exempt label)`. L'affirmation était fausse : `finalize()` avait
perdu son `LowerHex` et `hmac` a dû suivre en 0.13.

**mika#2561** (`utoipa 5.5.0 → 6.0.0`) — le verdict **s'inverse** au fil des
tours : `hold[review]` deux fois (« build verification recommended »), puis
`pass` quatre fois. `Check` rouge pendant 6 min 21 s.

La porte de merge a tenu. Le défaut est l'**approbation** : elle porte une
affirmation de compatibilité non vérifiée, et elle s'affiche `APPROVED` pour
l'humain qui merge.

## Ce que la lecture du code a déplacé — et c'est la leçon principale

Le ticket proposait trois remèdes. Deux sur trois auraient été **inertes**, et
la troisième rectification est celle qui porte la leçon.

### R1 — Le mécanisme accusé n'existait pas

Le ticket accusait le label `pipeline-exempt` d'être lu comme une dispense de
build. Recherche exhaustive : quatre occurrences dans le prompt, **toutes dans
Step 2**, aucune à portée du build. Ce qui prescrivait le skip était Step 1.6
item 2, **inconditionnellement** :

> Emit `PLAN-AC VERIFICATION: skipped (…)` and
> **`BUILD VERIFICATION: skipped (Dependabot dependency PR)`**.

Le `(pipeline-exempt label)` des verdicts mesurés est une **paraphrase du
modèle**, et la trace le prouve : la toute première revue de #2561 écrit la
formulation du prompt (`skipped (no Behavioral ACs — Dependabot PR)`), et c'est
seulement aux tours suivants que le label devient la raison invoquée.

> **Leçon transférable.** Quand un verdict cite une raison, cette raison peut
> être une paraphrase. Chercher la formulation **dans le prompt** avant de
> réparer ce que le verdict a nommé. Ici, corriger la lecture du label aurait
> été un correctif sur un mécanisme inexistant — merged, déployé, sans effet.

### R2 — La garde a tourné, a été satisfaite, et n'a rien empêché

mika#2519 avait livré une branche B2 refusant un `pass` sur un saut de majeure
dont le corps n'affirme aucune vérification des sites d'appel. `utoipa 5 → 6`
**est** un saut de majeure : la garde s'appliquait.

Elle a laissé passer, **et à juste titre au regard de son prédicat**. Chaque
`pass` de #2561 porte une ligne `API-SURFACE:` ancrée et non vide — la dernière
fait onze lignes : « 29 `#[derive(ToSchema)]` sites (mika-agent: 27,
mika-gateway: 2) … 14 `#[utoipa::path(...)]` call sites … 0 `#[into_params]`
usages ».

**Ce travail n'est pas fabriqué. Il est réel, sourcé par grep, et faux sur la
conclusion** — la casse d'utoipa 6 n'est pas dans la syntaxe des macros. Le
modèle a fait exactement ce que le prompt demandait, avec soin, et ça n'a pas
suffi.

> **Leçon centrale.** Durcir l'exigence de citation ne ferme rien ici. *Une
> lecture d'API, même honnête et détaillée, n'est pas une compilation.* Une
> garde qui exige une **assertion** obtient une assertion ; si l'assertion peut
> être sincère et insuffisante, la garde ne mesure pas ce qu'elle croit mesurer.

### R3 — Le cas que rien n'attrapait

`version_major("0.10.9")` et `version_major("0.11.0")` rendent tous deux
`Some(0)` → `NoMajorJump` → B2 ne s'applique pas. Vérifiable dans la trace : les
trois `pass` de #2560 ne portent **aucune** ligne `API-SURFACE:`.

Élargir B2 aux `0.x` ne l'aurait pas attrapé non plus. Le défaut y est une
incompatibilité **transverse** — `sha2 0.11` sur `digest 0.11` contre
`hmac 0.12` sur `digest 0.10` — qu'**aucune lecture des call sites de `sha2` ne
montre**. Seul le résolveur de cargo la voit.

## Le remède : un fait moteur, jamais une phrase

**Une PR dont le diff touche `Cargo.toml` ou `Cargo.lock` compile avant tout
verdict. À défaut de build, le verdict ne peut pas être `pass`.**

Deux moitiés, et comme toujours c'est la structurelle qui tient
(`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`) :

| moitié | site | ce qu'elle fait |
|---|---|---|
| **intention** | prompt Step 1.6 + callback | prescrit le build et sa reprise |
| **structure** | branche **B3** | refuse le `pass` quand **le moteur n'a pas vu de build** |

La moitié structurelle **ne lit aucune phrase du corps**. C'est la leçon de R2 :
B2 exige une assertion et le modèle en produit une ; B3 exige un **fait
moteur** — un appel `build_mika` réellement enregistré dans `tool_calls` pour
cette session.

```rust
if verdict_is_pass && !build_observed && is_cargo_dependency_pr(files) {
    return DependabotVerdictOutcome::RefusedUnbuiltCargoBump { … };
}
```

> *Un fait qu'on lit en base ne se rédige pas.* Précédent maison :
> `find_recent_destructive_actions` (mika#1646), qui lit la même table pour la
> même raison.

Le test qui distingue ce travail de mika#2519 est celui qui **satisfait B2** et
est refusé quand même :

```rust
// Le corps porte une `API-SURFACE:` détaillée — donc B2 est satisfaite.
assert!(body_asserts_api_surface(body), "la prémisse est que ce corps SATISFAIT B2");
assert_eq!(
    classify_dependabot_verdict(BOT, false, true, TITLE_2561, body, &f, false),
    DependabotVerdictOutcome::RefusedUnbuiltCargoBump { … }
);
```

Et l'AC6 est le corollaire : un corps qui **revendique** un build
(`BUILD VERIFICATION: Build: pass`, `BUILD-VERIFIED: yes`) est refusé tout autant
— sans quoi B3 serait B2 sous un autre nom.

## Trois pièges rencontrés, et leurs remèdes

### 1. L'abstention qui désarme ses voisines

Premier câblage : un `files` illisible faisait `abstain(); return`. Mesuré —
**les neuf tests de mika#2519 rougissaient**, et le `pass` sur #2453, la PR
témoin que mika#2519 existe pour débloquer, ressortait `abstained` au lieu
d'`allowed`.

B1 et B2 ne lisent ni `files` ni la base : leur jugement reste entier sans eux.
L'abstention doit être **partielle**.

> **Règle.** *Un terme qu'on n'a pas pu évaluer n'est jamais un terme satisfait ;
> ce n'est pas non plus une raison de cesser d'évaluer les autres.*

### 2. Le scan qui accuse sa propre prose

Un scan Layer C vérifiait l'absence de `--detach` dans la section 5c du prompt —
et rougissait, parce que la prose explicative dit « **attached to the branch, not
`--detach`** ». Classe mika#2050 : une **mention** n'est pas une **instruction**.
Remède : scanner les lignes portant `worktree add`, jamais la section entière.

### 3. Le prompt saturé

Les quatre éditions ont poussé `qa-review/system_prompt.md` à 98,9 % de son
plafond, déclenchant le gate mika#852. **Aucune valeur de cap ne convenait** : la
ronde (79 Ko) ne passe pas son propre gate, la suivante touche le plafond dur.

Remède appliqué, et l'arbitrage vaut d'être retenu : **le prompt porte la règle ;
le *pourquoi* vit là où il est consommé** — dans le corps du refus B3 (servi au
modèle au moment exact où il se trompe), dans le plan, et dans les messages
d'assertion des tests. ~2 000 octets ont été coupés avant de relever le cap de
1 536. Le commentaire du `skill.toml` dit désormais que **le prochain ajout devra
sharder** : 327 octets de marge au gate, 512 au plafond dur.

## Quatre obstacles structurels que le remède naïf n'aurait pas vus

1. **Le chemin de build existant est inatteignable.** Step 3e dérive un worktree
   de *dispatch*, qu'une PR dependabot n'a jamais. Y brancher sans plus aurait
   été un skip de plus sous un autre motif — **inerte**.
2. **Le worktree doit être managé.** Créé sous `.claude/worktrees/` (terme T1 du
   faucheur mika#2420, population de la purge `target/` mika#2497) et **attaché à
   la branche**. Ailleurs : un orphelin de 15–50 Go que rien ne fauche.
3. **Le callback est plan-centré.** Sur une PR sans plan il aurait rendu
   `block[pipeline]` — le verdict que **B1 refuse structurellement** sur un auteur
   automatisé. Couplage vicieux : le callback produit le verdict que la garde
   interdit, et la revue meurt sur un refus d'outil.
4. **B3 atteste que le build a eu lieu, pas qu'il était vert.** Population nommée,
   mesurable a posteriori, **suivi** — pas masquée.

## Comment reconnaître cette classe ailleurs

Symptômes :

- une garde « n'a pas mordu » alors que son prédicat était satisfait ;
- le verdict porte une justification **détaillée et sourcée** qui s'avère fausse ;
- le réflexe est de durcir l'exigence de citation.

Questions à poser :

1. **Que mesure réellement la garde ?** Une assertion du modèle, ou un fait que
   le moteur a enregistré lui-même ?
2. **L'assertion peut-elle être sincère et insuffisante ?** Si oui, la durcir ne
   ferme rien.
3. **Existe-t-il un signal que le modèle ne peut pas rédiger ?** Une ligne en
   base, un exit code, un fichier produit. C'est celui-là qu'il faut lire.
4. **Le chemin qui produirait ce signal est-il atteignable ?** Sinon, le
   correctif sera un skip de plus sous un autre motif.

## Ce que ce travail n'achète pas

Il ne rattrape pas #2560 ni #2561 : les verdicts sont postés, #2560 est mergée, et
**rien ne rétro-écrit un verdict passé** — fabriquer une ligne décrivant une
vérification qui n'a pas eu lieu est l'inverse de ce que ce travail défend.

Il ne couvre pas les bumps npm / `actions/*` / pip : `build_mika` ne les compile
pas. Conséquence assumée d'un prédicat sur le **fichier** plutôt que sur
l'intention — un prédicat sur l'écosystème deviné depuis le titre produirait un
refus sans remède.

Il crée un coût réel : un `cargo build --release --features telemetry` sur un
`target/` vide pèse 15 à 50 Go et plusieurs minutes, à 4–8 PR dependabot par
semaine. Borné par les deux faucheurs, mais **c'est un coût que ce travail crée**.
