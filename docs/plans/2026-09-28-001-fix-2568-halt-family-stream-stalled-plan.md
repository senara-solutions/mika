---
title: "fix(dispatch-lib): _halt_family classe stream_stalled (claude-pilot#222)"
type: fix
date: 2026-09-28
origin: senara-solutions/mika#2568
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# fix(dispatch-lib): _halt_family classe stream_stalled (mika#2568)

## Goal Capsule

- **Objective :** un pilote tué par un arrêt en pleine génération porte, dans son callback, une famille et un indice de retry exacts (`model_stalled`, `transient`) au lieu de `unknown|investigate` ; et `make test-dispatch-lib` redevient vert sur l'hôte de dispatch, dont le checkout claude-pilot sert déjà claude-pilot#222.
- **Means :** une branche de plus dans la table `case` de `_halt_family`, et la ligne correspondante dans la table T1 du harnais. Rien d'autre.
- **Autorité :** le corps de mika#2568 (famille et indice proposés), puis le `types.py` de claude-pilot à `main` (le vocabulaire amont).
- **Stop :** si la famille proposée contredit ce que le commentaire amont affirme de `stream_stalled`, arrêter et le dire plutôt que choisir.

## Product Contract

### Summary

claude-pilot#222 (volet 1 de cpp#219), mergée en `ef9f8d0` et servie depuis 2026-09-28 15:31Z, ajoute `stream_stalled` au `Literal` de `GuardrailAbortReason.guardrail`. `_halt_family` ne le connaît pas.

### Problem Frame

Deux effets, tous deux mesurés dans ce worktree le 2026-09-28 :

1. **Classification fausse.** Un callback portant `stream_stalled` tombe dans le bras `*)` : `unknown|investigate|subtype outside the downstream table`. Sans danger (le bras existe pour ça, et il émet `halt_family.unknown` dans les deux puits), mais faux — l'opérateur lit « inconnu » là où l'amont a nommé un motif précis. Par ricochet, `_rescue_cause_token` (mika#2539) rend `halt_unmapped` pour ce motif au lieu d'une famille.
2. **Garde T6 rouge en local.** `CLAUDE_PILOT_TYPES=<meta>/claude-pilot/src/claude_pilot/types.py bash skills/bundled/_shared/test-dispatch-lib.sh` rend **1154 passed, 1 failed** : `✗ T6 drift: upstream value 'stream_stalled' has a downstream family (got: unknown)`. Le garde fait son travail ; c'est la table aval qui est en retard.

### Requirements

- R1. `_halt_family stream_stalled` rend `model_stalled|transient|<indice>`, jamais `unknown`.
- R2. Le garde T6 passe contre le `types.py` du checkout claude-pilot à jour sur `main` (contient `ef9f8d0`).
- R3. La table T1 du harnais couvre `stream_stalled` de la même façon qu'`idle_timeout` (une ligne `subtype|family|hint`, les deux lignes `Halt class:` / `Retry hint:` assertées).

### Key Decisions

- **Famille `model_stalled`, indice `transient` — ceux du ticket, retenus.** Le commentaire amont (`types.py`, bloc cpp#219) affirme : tour **ouvert** (`message_start` vu, pas de `message_stop`), un bloc de contenu laissé ouvert, seulement des pings `progress=False` sur le fil — le modèle s'est figé en pleine génération, personne d'autre n'est attendu. C'est une cause **hors de la session**, ce que la légende de la table définit comme `transient`. La mesure de cpp#214 citée par le ticket (4 arrêts : dernier contenu reçu, puis pings seuls, sans cause côté session) va dans le même sens. `investigate` est écarté : il désigne « la cause est dans le log », ce que l'amont affirme justement ne pas être le cas.
- **Famille nouvelle plutôt que `session_silent` ou `model_never_resumed`.** `session_silent` est la famille d'`idle_timeout`, dont cpp#219 a **séparé** ce motif précisément pour qu'il cesse de s'y blanchir ; le remettre dans la même famille annulerait la séparation en aval. `model_never_resumed` (`awaiting_model`) désigne un **premier jeton** jamais reçu ; ici des jetons ont été reçus, puis plus rien. Deux faits distincts, deux familles.
- **Aucune autre surface touchée.** `_rescue_cause_token` consulte `_halt_family` et gagne la bonne famille sans une ligne ; la population du scan mika#2539 (S2) est **dérivée** de la table et suit d'elle-même (à 12 ; le commentaire et le seuil anti-vacuité, calés sur le compte de 11, suivent — voir U2).

### Scope Boundaries

- Hors périmètre : la surface opérateur graduée par sévérité (mika#1381, consommateur aval distinct nommé par l'amont) ; toute modification de claude-pilot ; tout changement de timing ou de retry côté moteur — `transient` est un **indice** lu par `self-dev-callback`, pas un mécanisme.

### Success Criteria

- Le harnais complet rend 0 échec avec `CLAUDE_PILOT_TYPES` pointé sur le checkout à jour, le marqueur `DRIFT-GUARD: armed … (9 values)` présent.

## Planning Contract

### Key Technical Decisions

- KTD1. La branche s'insère **juste après `idle_timeout)`** dans la table, avec son commentaire de source `# cpp#219, cpp#222` aligné comme les autres. Voisinage lisible : le motif est une scission d'`idle_timeout`.
- KTD2. La ligne T1 s'insère juste après `idle_timeout|session_silent|investigate`, même raison.
- KTD3. Le commentaire de légende de `_halt_family` (« where upstream does not rule — stall_detected, empty_response, idle_timeout — the hint is `investigate` ») reste vrai tel quel : `stream_stalled` est un cas où l'amont **statue**. Aucune édition.

### Assumptions

- Le texte d'indice est celui du ticket, en anglais comme les onze autres lignes de la table (jeton machine + prose de callback déjà anglophone dans cette table).

## Implementation Units

### U1. Branche `stream_stalled` dans `_halt_family`

- **Fichier :** `skills/bundled/_shared/dispatch-lib.sh` (fonction `_halt_family`).
- **Changement :** `stream_stalled)` → `printf '%s\n' "model_stalled|transient|the model stopped mid-generation (turn open, pings only) with nobody outstanding; a re-run has a fair chance"`, commentaire de source `# cpp#219, cpp#222`.
- **Execution note :** le rouge est déjà établi (1154/1 failed) ; le vérifier vert après le changement, sans modifier le garde.

### U2. Couverture T1 et vérification T6

- **Fichier :** `skills/bundled/_shared/test-dispatch-lib.sh` (heredoc `T1_TABLE`).
- **Changement :** ajouter `stream_stalled|model_stalled|transient`. Dans le même fichier, le bloc anti-vacuité du scan mika#2539 (S2) dit « la table en compte onze aujourd'hui » avec un seuil `-ge 11` égal au compte : passer le commentaire à « douze » et le seuil à `-ge 12`, pour que le compte écrit reste vrai et que le plancher reste serré (constat de la revue du plan).
- **Scénarios :**
  - T1 : `_classify_probe '' 2 stream_stalled 'x' full` produit `Halt class: model_stalled` et `Retry hint: transient`.
  - T2 (contrôle négatif existant) : un sous-type connu reste muet sur stderr — couvre désormais implicitement la classe ; inchangé.
  - T6 : contre le `types.py` du checkout, `stream_stalled` rend une famille non vide et ≠ `unknown`.
  - S2 mika#2539 : la population dérivée passe à 12, l'anti-vacuité `-ge 12` reste verte.

## Verification Contract

```bash
CLAUDE_PILOT_TYPES=<meta>/claude-pilot/src/claude_pilot/types.py \
  bash skills/bundled/_shared/test-dispatch-lib.sh      # attendu : 0 failed, DRIFT-GUARD armed (9 values)
bash skills/bundled/_shared/tests/test_rescue_cause_token.sh   # non-régression mika#2539
bash -n skills/bundled/_shared/dispatch-lib.sh
```

`<meta>` = racine du workspace mika-platform ; le checkout claude-pilot doit contenir `ef9f8d0`.

## Definition of Done

- U1 et U2 livrés dans un seul commit.
- Harnais complet vert contre le `types.py` à jour (sortie citée dans la PR).
- `test_rescue_cause_token.sh` vert.
- PR `Closes #2568`, label `origin:spawn`.

## Acceptance criteria

- [ ] `_halt_family stream_stalled` rend une famille et un indice nommés, pas `unknown`.
- [ ] Le garde T6 passe contre le `types.py` de claude-pilot#222.
- [ ] Le test existant de classification couvre `stream_stalled`, de la même façon que `idle_timeout`.
