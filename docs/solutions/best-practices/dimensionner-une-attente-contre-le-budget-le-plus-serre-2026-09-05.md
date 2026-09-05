---
module: mika-agent/server
tags: [timeout, backpressure, a2a, claude-pilot, budget, code-review, mika-2163]
problem_type: logic-error
category: best-practices
---

# Une attente serveur se dimensionne contre le budget de l'appelant **le plus serré**

## Le problème

mika#2163 remplace un refus sec (`-32603 "Agent is busy"`) par une attente bornée
sur `/a2a/{agent}`. Un tel changement n'est un progrès que tant que l'appelant est
encore là pour lire la réponse : au-delà, il convertit un refus lisible en une
coupure de transport illisible — c'est-à-dire exactement le mal qu'il prétend
guérir.

Le plan l'avait vu, et avait écrit la contrainte noir sur blanc :

> `mika ask` appelle avec `A2aClient::DEFAULT_TIMEOUT = 300 s`. Ce budget couvre
> l'attente ET le tour d'agent. […] **Le refus doit donc arriver franchement avant
> le plafond client.** Défaut retenu : **120 s**, ce qui laisse 180 s au tour.

Le raisonnement est juste. La prémisse était fausse — pas parce que le chiffre de
300 s était faux, mais parce qu'il n'était pas le seul.

```
.claude/claude-pilot.json   (identique dans les cinq dépôts du plan de travail)
{ "command": "mika", "args": ["--agent","mika-dev","ask"], "timeout": 120000 }
```

| Appelant | Budget | Couvre |
|---|---|---|
| `A2aClient::DEFAULT_TIMEOUT` | 300 s | attente + tour |
| relais `canUseTool` de claude-pilot | **120 s** | attente + tour |

Le second n'est pas un appelant secondaire : c'est **le chemin sur lequel le
ticket a été fiché**. Une attente de 120 s dépense donc la totalité de son budget.
Le relais tue `mika ask` à l'instant exact où l'attente expire — le `AGENT_BUSY`
tout juste construit, avec sa raison et son `retry_after_ms`, n'est jamais lu.
Avant le correctif, le pilote recevait un `-32603` immédiat qu'il pouvait
réessayer ; après, il aurait reçu un silence.

## Pourquoi c'est facile à rater

Parce que chercher « le budget de l'appelant » rend une réponse **plausible et
unique** — la constante nommée `DEFAULT_TIMEOUT`, dans le crate client, avec son
commentaire de justification. Rien dans cette constante ne signale qu'un appelant
réel la surcharge par trois fois moins. Le fichier qui porte le vrai plafond n'est
pas dans le crate ; c'est un JSON de configuration de déploiement, dans cinq
dépôts, sans lien lexical avec le mot « timeout » côté serveur.

Le biais est structurel : **on dimensionne contre le budget qu'on a trouvé, et le
plus facile à trouver est le plus générique — donc le plus généreux.**

## La règle

Avant de fixer une attente, un recul, un délai de garde ou tout autre laps pendant
lequel un appelant est censé patienter : **énumérer les appelants réels, relever
leur plafond, et dimensionner contre le plus petit.** La constante par défaut du
client est un plancher de recherche, pas la réponse.

Concrètement, pour ce dépôt, la question à poser est : *qui appelle ce chemin, et
avec quel `timeout` ?* Le relais du pilote (`.claude/claude-pilot.json`), les
appels d'agent à agent, le gateway et le client A2A par défaut n'ont pas le même,
et le fichier de déploiement gagne contre la constante du crate.

Et la fraction compte autant que l'ordre de grandeur : le budget couvre
**l'attente ET le travail**. Une attente qui consomme la moitié du plafond le plus
serré ne laisse pas de quoi faire le travail pour lequel on a attendu. Retenu ici :
30 s sur 120, soit un quart, avec 90 s pour le tour.

## Le test qui l'empêche de revenir

Un défaut de temporisation ne se protège pas par un test d'égalité — celui-ci se
met à jour mécaniquement avec la constante qu'il devait garder. Il se protège par
l'**inégalité qui porte la raison** :

```rust
// La borne qui rend ce nombre correct : l'attente doit rester une fraction du
// budget de l'appelant le plus serré — le relais canUseTool de claude-pilot,
// 120 s — ou l'appelant est tué avant d'avoir pu lire le refus.
const TIGHTEST_CALLER_BUDGET_MS: u64 = 120_000;
assert!(settings.effective_a2a_queue_wait_timeout_ms() * 2 < TIGHTEST_CALLER_BUDGET_MS);
```

Il laisse ajuster le défaut librement dans la zone sûre, et n'arrête que la
personne qui le pousse au-delà — laquelle doit alors lire pourquoi.

## Où ça vit

- `crates/mika-common/src/config.rs` — `DEFAULT_A2A_QUEUE_WAIT_TIMEOUT_MS`, sa
  table de budgets et l'assertion de borne dans `config::tests::a2a_queue_defaults`.
- `docs/plans/2026-09-05-001-fix-2163-a2a-attente-bornee-au-lieu-du-refus-sec-plan.md`
  §2 — le raisonnement d'origine, la mesure qui l'a corrigé, et §9 pour les deux
  autres coûts que la même revue a nommés.
