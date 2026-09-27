---
title: Une exemption écrite deux fois et tenue zéro fois — comptez les étages avant d'en ajouter un
date: 2026-09-26
last_updated: 2026-09-26
category: best-practices
module: mika-agent/skills/builtin_handlers
problem_type: best_practice
component: dev-loop
severity: high
applies_when:
  - Un ticket demande de « livrer » une exemption, une garde ou une règle
  - Le symptôme mesuré est POSTÉRIEUR au merge d'une règle qui devrait le couvrir
  - Un verdict, un refus ou un motif rapporté est cité dans un corps de ticket
  - Un prompt de skill prescrit une règle et rien dans le moteur ne la tient
  - Une AC pose un build vert comme le filet d'une exemption
---

# Une exemption écrite deux fois et tenue zéro fois

## Le fait

mika#2519 rapportait que mika-qa rendait `VERDICT: block[pipeline]` sur des PR
Dependabot Cargo-only build-vertes, et demandait de **livrer** une exemption
nommée pour `author == dependabot[bot]` (AC1) plus la déclaration de son label
(AC4).

Quatre mesures prises sur l'arbre ont déplacé ce remède, et chacune a changé ce
qu'il fallait écrire.

**R1 — l'exemption existait déjà, à deux étages, et les deux précédaient la
mesure.**

| étage | site | mergé |
|---|---|---|
| garde exécutable | `scripts/verify-pipeline.sh` mécanisme 4, `AUTOMATED_PR_AUTHORS` sur `.pull_request.user.login` | **2026-09-21** (mika#2419) |
| prompt QA | `qa-review/system_prompt.md` Step 1.6, *« Do NOT run Steps 2/2.5/3e for a Dependabot PR »* | **2026-08-26** (mika#1729) |

Le symptôme est daté du **2026-09-24** — trois jours après le second étage, un
mois après le premier.

**R2 — AC4 était déjà satisfait.** `pipeline-exempt` est déclaré à
`.github/labels.yml:118`. Zéro ligne à écrire.

**R3 — le motif cité n'était le texte d'aucune garde.**

```bash
grep -rn "no plan document" --include='*.md' --include='*.sh' --include='*.rs' .
# → zéro ligne (hors le plan, qui le cite)
```

La sortie réelle de `verify-pipeline.sh` est
`[pipeline-exempt: none] REJECT: code-only PR: source changes present but no plan/solution doc`.
Le motif rapporté — *« Dependabot dependency bump — no plan document or
`Pipeline-Exempt` trailer present. Build verified successfully. »* — est une
**prose fabriquée**. Or Step 2E exige qu'un `block[pipeline]` cite la sortie de
sa garde verbatim.

**R4 — le ticket se trompait sur sa propre PR témoin, et cette rectification
inverse la valeur d'une AC.** Le ticket écrivait *« #2454 (jsonwebtoken 9→11,
build-vérifié PASS) »* et *« sur #2453 comme #2454 le build a réussi et aucun
site d'appel n'était cassé »*. L'historique du dépôt dit l'inverse, deux commits
plus tard :

> **mika#2525** (`d4514180`, 2026-09-25) — *« Le bump 9.3.1 -> 11.1.0 faisait
> PANIQUER `generate_jwt` : jsonwebtoken 11 a retiré ring pour une architecture
> à provider enfichable et son `default` ne porte que `use_pem`, donc aucun
> backend crypto n'était actif. **Build vert, test rouge.** »*

Trois sites de production appellent `EncodingKey::from_rsa_pem`. Donc le `block`
sur #2454 était, **par accident, le bon verdict** — et l'AC2 du ticket, qui pose
le build comme le filet de l'exemption, décrivait un filet qui ne tient pas pour
cette classe.

## La leçon

**Comptez les étages d'une règle avant d'en ajouter un, et datez-les contre le
symptôme.** Un symptôme postérieur au merge d'une règle qui devrait le couvrir ne
dit pas « la règle manque » : il dit « la règle n'est pas tenue ». Les deux
diagnostics ont des remèdes opposés — le premier fait écrire une troisième copie,
le second fait écrire la **moitié structurelle**. Et une règle posée à un
troisième endroit dérive des deux autres : c'est la classe que mika#2172 a fermée
sur ce prompt précis.

**Un motif cité dans un corps de ticket est une donnée à vérifier, jamais un
fait.** Le `grep` de R3 coûte dix secondes et il a déplacé le diagnostic entier :
un motif qu'aucune garde n'émet n'est pas un refus de garde, c'est une prose de
modèle, donc la classe
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` — *le
prompt exprime l'intention ; il ne tient pas seul au substrat de la boucle.*

**Un build vert n'est pas un filet ; c'est un plancher.** L'erreur d'AC2 n'est
pas d'avoir exigé le build, c'est de l'avoir posé comme la garantie. Un défaut
derrière une cascade `cfg` — `panic!` à l'exécution, feature non activée — est
invisible au build par construction. Quand une AC nomme un signal comme le filet
d'une exemption, demandez **quelle classe de défaut ce signal ne voit pas**.

**Et une PR témoin citée comme « saine » se vérifie dans l'historique.** Deux
commits plus loin, le dépôt portait la réparation du bump que le ticket donnait
pour correct. Sans cette lecture, un rail dependabot autonome aurait mergé un
bump qui panique à la génération du JWT GitHub App — c'est-à-dire à
l'authentification de toute la boucle.

## Ce que le correctif a livré, et ce qu'il a refusé

Aucune exemption nouvelle (`verify-pipeline.sh`, `.github/labels.yml` et `ci.yml`
sont inchangés). Ce qui est livré est la moitié qui **tient** :

- **B1** — une garde pré-subprocess refuse un `block[pipeline]` posté sur une PR
  d'auteur automatisé : Step 1.6 rend ce verdict structurellement inatteignable,
  et le refus nomme les deux issues correctes.
- **B2** — la garde de substance que R4 rend nécessaire : un `pass` sur un **saut
  de majeure** dont le corps ne porte pas de ligne `API-SURFACE:` est refusé.

Pré-subprocess et non EndTurn, pour la raison que mika#2237 a déjà écrite : *le
défaut est l'appel, pas une phrase* — quand un bras EndTurn tournerait, la revue
serait sur GitHub.

Et la règle `0.x` de semver n'est **pas** appliquée : l'élargir ferait entrer
`0.22 → 0.23`, donc quatre des cinq PR témoins, dans la population de B2 —
c'est-à-dire refuser le rail que le ticket existe pour ouvrir. Un saut de mineure
sur `0.x` reste couvert par le scan de changelog (`block[dependency]`). Si une
mesure montre une rupture `0.x` passée au travers, **c'est un ticket avec son
compte**, pas un élargissement au jugé.

## Sites

- `crates/mika-agent/src/evidence/guards.rs` — section mika#2519 : les deux
  classifieurs purs, `AUTOMATED_PR_AUTHORS` (propriétaire unique côté Rust, en
  parité bidirectionnelle avec le shell), `API_SURFACE_LINE_PREFIX`.
- `crates/mika-agent/src/skills/builtin_handlers.rs` —
  `validate_dependabot_verdict_coherence`, en queue de la chaîne `run_gh`.
- `skills/bundled/qa-review/system_prompt.md` Step 1.6 — `author.login` (et non
  `author`, un **objet**) plus la clause de saut de majeure.
- `scripts/canonical-tokens.tsv` — la ligne du jeton `API-SURFACE:`.

## Voisins

- `docs/solutions/best-practices/une-memoire-apprise-dun-echec-survit-au-fix-de-cet-echec-2026-09-19.md`
  — même signature : le skill prescrit, la mémoire gagne, argv contredit.
- `docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md`
  — la vigilance qu'AC4 exprimait, déjà honorée ici.
- `docs/solutions/architecture-patterns/guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md`
  — la doctrine qui gouverne la lecture de `API-SURFACE:` (permissive) contre sa
  décision (stricte).
