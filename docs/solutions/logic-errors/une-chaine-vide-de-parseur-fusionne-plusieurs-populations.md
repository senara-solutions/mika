---
module: skills/bundled/_shared/dispatch-lib
date: 2026-10-02
problem_type: logic_error
component: tooling
severity: medium
category: logic-errors
ticket: mika#2641
symptoms:
  - "Une seconde passe architecte qui écrit `Disposition: ITERATE` est relancée avec le prompt correctif, puis escaladée sous `Groom-halt-cause: architect_unreadable`"
  - "Le motif d'échec dit « absence de signal, pas une objection » sur une réponse qui EST une objection"
root_cause: logic_error
resolution_type: code_fix
tags:
  - groom
  - parse-verdict
  - second-pass
  - unparsed
  - disposition
  - fail-safe
related:
  - docs/solutions/best-practices/fuzzy-disposition-parsing-two-tier-2026-05-27.md
  - docs/solutions/best-practices/an-exempted-disposition-is-the-attack-surface-2026-08-30.md
  - docs/solutions/workflow-issues/verdict-writer-and-gate-must-share-one-vocabulary-2026-08-27.md
---

# Une chaîne vide de parseur fusionne plusieurs populations : les énumérer avant de lui donner un sens

## Problem

mika#2641 a donné un sens nouveau à la chaîne vide que rend `_parse_verdict`
(`skills/bundled/_shared/dispatch-lib.sh:7206`) en seconde passe de groom :
« l'architecte n'a émis aucune disposition lisible » → une relance, puis un
ESCALATE à motif `architect_unreadable`. Or cette chaîne vide n'a jamais
désigné une seule population, et l'une de celles qu'elle contient est
exactement l'inverse d'une absence de signal.

## Symptoms

Une seconde passe dont la réponse se termine par `Disposition: ITERATE` (ou
`**Verdict:** ITERATE`) était relancée avec le prompt correctif, puis, si la
relance ne rendait toujours pas GROOMED/ESCALATE, escaladée avec la prose
« This is an absence of signal, NOT an objection to the plan ». L'opérateur
était envoyé chercher un modèle tronqué là où l'architecte avait objecté.

## What Didn't Work

Le plan (§ « Note de lecture sur AC1 ») a posé le prédicat
« `_parse_verdict` rend une chaîne vide » comme équivalent d'« aucune
disposition reconnue », en notant seulement que le parseur accepte `Verdict:`
et `Disposition:`. C'était vrai des **orthographes**, faux des **valeurs** :
le tier 1b du parseur n'a délibérément **pas** de bras ITERATE
(`dispatch-lib.sh:7287`, « the spec forbids a third-pass »), donc une ITERATE
explicite retombe sur le fuzzy puis sur le vide, comme un préambule tronqué.
Les tests du pilote couvraient le verbatim tronqué, l'ESCALATE explicite, le
transport et le `.content` vide, mais aucun cas « disposition lisible que le
parseur ne mappe pas ». C'est la revue de code (correctness) qui l'a trouvé,
pas la suite.

## Solution

Un terme dédié, lu **ancré en début de ligne** (emphase markdown tolérée), qui
sépare l'objection explicite de l'illisible avant que la relance ne parte :

```bash
_second_pass_explicit_iterate() {
    grep -qE '^[[:space:]>*_`-]*(Verdict|Disposition)[*_`]*:[[:space:]*_`]*ITERATE([^[:alnum:]_]|$)'
}
```

(`dispatch-lib.sh:7978`). Aux deux sites de seconde passe, une réponse qui le
satisfait **sort de la boucle sans relance** (`dispatch-lib.sh:8203`) et est
routée vers le bras ESCALATE à cause `verdict` (`dispatch-lib.sh:8214`),
c'est-à-dire le `RESULT` d'avant mika#2641 à l'octet près, avec le motif
d'échec « architect refused on second pass ». Le terme ne modifie pas
`_parse_verdict` (R-i du plan : ses tiers ne bougent pas).

L'ancrage est porteur : un préambule tronqué qui **cite** la disposition de
première passe en prose (« Ma première passe rendait Disposition: ITERATE ;
je relis… ») doit garder sa relance. Les deux sens sont épinglés dans
`test-dispatch-lib.sh` (V10, deux sites, plus le contrôle d'ancrage).

## Why This Works

« Le parseur ne rend rien » est une propriété du **parseur**, pas de la
**réponse**. Au moins trois populations y tombent en seconde passe :

| population | ce que c'est | traitement juste |
|---|---|---|
| réponse tronquée / préambule seul | absence de signal | relance, puis `architect_unreadable` (mika#2641) |
| disposition lisible non mappée (ITERATE) | objection | terminal `verdict`, sans relance |
| disposition retirée par le moteur (tier 0, `Disposition-Withheld`) | attestation manquante | population vide aujourd'hui, parce que le tier 0b tire d'abord ; prémisse épinglée par une assertion de manifeste |

Attacher un sens à « vide » sans les énumérer revient à étiqueter les trois du
nom de la plus fréquente. La correction ne cherche pas à élargir le parseur :
elle nomme la population qu'on ne veut pas relancer, avant la relance.

## Prevention

- **Avant de donner une sémantique au `None`/vide d'un parseur, énumérer chaque
  chemin qui le produit** : bras délibérément absents, tiers qui rendent vide
  exprès (retrait moteur), et seulement ensuite le « rien du tout ». Les bras
  absents sont les plus faciles à rater, parce qu'ils sont documentés comme une
  décision, pas comme une sortie.
- **Un test par population**, pas seulement par forme du défaut fondateur. Le
  cas manquant ici était « disposition lisible que le parseur ignore ».
- **Ancrer le terme qui sépare**, et épingler l'ancrage par un contrôle négatif
  en prose : sinon le terme vole leur relance aux réponses tronquées qui citent
  le mot.

## Ce qui reste ouvert (même relance, autre frontière)

La revue adversariale de la même branche a trouvé une seconde frontière
traversée par la relance, **non corrigée ici parce qu'elle renverse une
décision du plan (D7/V4)** : le prompt correctif (~350 octets) passe sous
`review_anchor_min_brief_chars = 2000`
(`skills/bundled/mika-arch-second-review/skill.toml:37`), seuil sous lequel la
garde d'ancrage mika#2037 ne s'arme pas
(`crates/mika-agent/src/agent_loop/mod.rs:3902`). Un `Verdict: GROOMED` nu au
tour de relance n'est donc pas attesté. Le plan n'a pesé que le sens inverse
(une garde armée qui transforme la relance en ESCALATE). Même famille de
leçon : une exception de chemin (« la relance est courte ») traverse une
frontière que le reste du système tient pour universelle (« toute approbation
de seconde passe est attestée »). Remède routé à l'arbitrage du plan.
