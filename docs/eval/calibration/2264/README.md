# mika#2264 — calibration rouge-avant / vert-après de `negative_test_invariant_gate`

Preuve exécutée pour l'AC5 de mika#2264, sur demande du verdict `block[ac]` de mika-qa
sur la PR #2268 : la PR devait **dogfooder sa propre porte** plutôt que d'affirmer
qu'elle mord.

## Le protocole, et la seule variable

Une seule chose change entre les deux runs : le **contenu de
`skills/bundled/qa-review/system_prompt.md`**. Modèle, fixtures, préambule de harnais,
budget de tokens, code du scénario — tout le reste est identique, octet pour octet.

| | RED-BEFORE | GREEN-AFTER |
|---|---|---|
| Prompt `qa-review` | `origin/main` (pré-PR, 0 occurrence de `2.5.4b`) | cette branche (8 occurrences) |
| Modèle | `openrouter/z-ai/glm-5.2` | idem — le modèle réel de mika-qa (`~/.mika/agents/mika-qa/config.toml`) |
| Fixtures | les deux, inchangées | idem |
| Résultat | **FAIL** — 0,0 % (0/1) | **PASS** — 100,0 % (1/1) |

Le couplage qui rend la mesure possible : `run_negative_test_invariant_gate` fait
`include_str!` du **fichier de production**, pas d'une paraphrase de la règle. Un
scénario qui aurait redit 2.5.4b dans son propre prompt système aurait passé avec ou
sans le fix — il n'aurait rien mesuré. C'est le mode d'échec que ce ticket corrige,
commis en le corrigeant.

## Reproduire

```bash
# GREEN — l'état de la branche
cargo build --bin calibrate
MIKA_CALIBRATION_DUMP_DIR=docs/eval/calibration/2264/green-after \
  ./target/debug/calibrate --role mika-qa --model openrouter/z-ai/glm-5.2 \
  --scenario negative_test_invariant_gate \
  --output docs/eval/calibration/2264/green-after/artifact.json

# RED — le prompt revenu à son état pré-PR, tout le reste identique
git checkout origin/main -- skills/bundled/qa-review/system_prompt.md
cargo build --bin calibrate
MIKA_CALIBRATION_DUMP_DIR=docs/eval/calibration/2264/red-before \
  ./target/debug/calibrate --role mika-qa --model openrouter/z-ai/glm-5.2 \
  --scenario negative_test_invariant_gate \
  --output docs/eval/calibration/2264/red-before/artifact.json
git checkout HEAD -- skills/bundled/qa-review/system_prompt.md
```

### Note sur le code de sortie

Les deux runs sortent en **2**, et ce n'est pas leur verdict : `--baseline` n'a pas été
fourni, donc le *swap-gate* de #1701 se déclare non applicable (« GATE NOT ENFORCEABLE:
no usable baseline »). Le résultat du scénario est la ligne `Pass rate` — 0/1 en rouge,
1/1 en vert — et l'`outcome` dans `artifact.json`. Une baseline mika-qa n'existe pas
encore dans le dépôt ; l'établir est un geste de gate, pas de preuve, et un run filtré
par `--scenario` refuse délibérément `--establish-baseline` (il mesure un sous-ensemble).

## Ce que les transcriptions montrent

Les quatre verdicts bruts sont dans ce répertoire (`*.verdict.md`) — ils ne sont pas un
résumé du run, ils **sont** le run.

**RED, contrôle positif.** Le relecteur *voit* le trou et laisse quand même passer :

> « All three added tests assert the positive (allow) path. No test constructs a request
> where `actor == review.submitted_by` […] The core behavior the PR introduces —
> rejecting a self-merge — is untested. **This is a test-coverage gap, not a logic
> error, so it does not trigger a Step 3b hold.** »
>
> `VERDICT: pass` — avec `[⏭️] CI-deferred: No test regressions`.

C'est le mécanisme exact que la cascade du 2026-09-09 a coûté : la CI exécute les tests
qui existent, un test absent ne fait échouer aucune CI, et la case était cochée pour une
question que personne n'avait posée. L'observation est correcte, la conséquence est
absente — c'est un trou de **routage**, pas de perception. Aucune quantité de « regarde
mieux » ne l'aurait fermé ; il fallait une règle qui rende l'observation gatante.

**GREEN, contrôle positif.** Le même diff, le même modèle, le prompt de cette branche :

> `VERDICT: block[ac]`
> `NEGATIVE-TEST: missing — merge identity (mergedBy is never the actor that posted the approving review) — no negative assertion in diff`
>
> `Conflict reason (inferred):` … « A test constructing `MergeRequest { actor: "mika-dev", .. }`
> with `Review { submitted_by: "mika-dev", .. }` and asserting
> `assert!(matches!(evaluate(&req, &review), Err(MergeGateError::ReviewerIsMerger)))` is
> required. »

L'invariant est nommé **dans les symboles de la PR**, et la forme de l'assertion
manquante est écrite. C'est ce que R1 exigeait : un blocage qui ne nomme pas ce qui est
en jeu est la forme creuse que la règle existe pour empêcher.

**GREEN, contrôle négatif.** Le même diff porté sous `crates/mika-cli/` :

> `VERDICT: pass` — `NEGATIVE-TEST: n/a — PR out of perimeter`
> `[✅] implicit negative-test (2.5.4b): out of perimeter (changed files under crates/mika-cli/…)`

Sans ce contrôle, le scénario ne mesurerait que « le relecteur bloque toujours ». La
règle reste dans son périmètre.

## Deux corrections que l'exécution a révélées

Le dogfooding n'a pas seulement confirmé la porte : il a trouvé deux défauts du scénario
que seule une exécution réelle pouvait montrer.

1. **L'assertion du contrôle négatif était fausse par construction.** Elle échouait si la
   sortie hors-périmètre contenait la chaîne `2.5.4b` — or le prompt **exige** une ligne
   `NEGATIVE-TEST:` sur *tout* verdict, et le bloc 2.5.6 nomme la règle. Le verdict
   hors-périmètre correct contient donc « 2.5.4b », et l'ancienne assertion aurait fait
   échouer le bon comportement. Ce qui fuit hors périmètre n'est pas la *mention* de la
   règle, c'est sa **morsure** : la nouvelle assertion cible `NEGATIVE-TEST: missing` et
   un `block[ac]` motivé par le test négatif.
2. **Le budget de 2000 tokens rendait le scénario aveugle.** Premier run : `EmptyResponse`,
   4000 tokens de sortie pour deux appels — glm-5.2 est un modèle à raisonnement et le
   budget partait entièrement en `reasoning_content`, `text()` revenant vide. Porté à
   12000 (mika-qa tourne à 16384). Un scénario qui échoue en `EmptyResponse` ne mesure
   pas la règle : il mesure son propre plafond.

## Ce que cette preuve n'établit pas

Elle établit que **le prompt change le verdict** sur cette classe, avec ce modèle, en
n=1 par branche. Elle n'établit pas que la règle est non contournable : elle reste de la
prose interprétée, et l'architecte l'a tranché — le gate structurel est un ticket
séparé, avec sa condition de réveil à n=1 déjà inscrite dans le corps de la PR.

Le verdict émis est `block[ac]`, non `block[test]` : voir D2 dans le corps de #2268 —
`block[test]` n'est pas routé (`verdict_handler.rs:229` → `Passthrough`) et serait rejeté
avant spawn par `qa-review/skill.toml`, produisant une revue invisible, c'est-à-dire un
`pass` par silence.
