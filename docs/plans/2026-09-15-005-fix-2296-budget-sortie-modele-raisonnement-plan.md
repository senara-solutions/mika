# mika#2296 — `llm_max_tokens = 8192` étouffe les modèles à raisonnement : le verdict n'est jamais atteint

> Ticket : senara-solutions/mika#2296 (p1, substrat)
> Type : fix
> Plan : `docs/plans/2026-09-15-005-fix-2296-budget-sortie-modele-raisonnement-plan.md`

## Le symptôme

La passe arch `8b623724` a tenu **123 s** (budget temps largement respecté) et
s'est terminée sur `stop_reason = MaxTokens`, `output_tokens = 8192` — exactement
`llm_max_tokens` — avec un **`content` vide**. Le raisonnement de glm-5.3 a
consommé la totalité du budget de sortie avant d'émettre la ligne
`Disposition:` ; `dispatch-lib` n'a rien trouvé à parser et a conclu
`PIPELINE FAILURE` (`326780db`). Même `MaxTokens` sur mika-dev au callback
14:08Z.

Le commentaire opérateur du 2026-09-15 mesure la même chose sur un second modèle
et isole la cause par contraste :

| Brief | `content` | Latence | Parsable |
|---|---|---|---|
| court (« Réponds: Disposition: READY ») | `"Disposition: READY"` | 6,7 s | ✅ |
| réel (plan 23–30 KB), 3 passes | **vide** | 26 / 39 / 26 s | ❌ |

Ce n'est donc ni le provider, ni le budget temps, ni le modèle : c'est la taille
du raisonnement que le brief déclenche, rapportée à un budget de sortie fixe.

## Ce que le code établit

Tout ce qui suit a été vérifié dans l'arbre à `e85a0b46`.

### C1 — le budget est bien celui du tour d'agent

`agent_loop/mod.rs:4598` et `:5169` construisent la `LlmRequest` avec
`max_tokens: llm.max_tokens()`, valeur posée à la construction du provider depuis
`Settings.llm_max_tokens` (`config.rs:1836` et `:1868` →
`create_provider_with_budget`). `openai.rs:658` transmet `req.max_tokens` tel
quel. La chaîne config.toml → tour d'agent est directe et sans clamp.

### C2 — le fait que le raisonnement compte dans le budget de sortie est documenté côté fournisseurs

Sur GLM-5.3, les tokens de raisonnement sont facturés et comptés **en sortie**,
et le raisonnement ne peut pas être désactivé (`thinking.type: "disabled"` est
refusé par un HTTP 400). Sur DeepSeek, la documentation actuelle donne
`max_tokens ∈ [1, 384K]` avec un défaut de 64K en mode thinking, et la règle de
comptage a changé depuis l'ère R1 : raisonnement et réponse **partagent** le même
plafond. Le symptôme du ticket est donc le comportement nominal de ces modèles
sous un plafond de 8192, pas une anomalie.

### C3 — la constante à modifier, et le fait qu'elle se propage

`well_known_agents.rs:1394-1417` porte `MIKA_ARCH_CONFIG` :
`llm_provider = "openrouter"`, `openrouter_model = "moonshotai/kimi-k2.5"`,
`llm_max_tokens = 8192`, plus les budgets 240/900 posés par mika#2189.
`reconcile_well_known_config` (`:688-737`, appelée en `:805`) réécrit le
`config.toml` d'un agent déjà provisionné à chaque passe de provisioning. Le
changement d'une constante prend donc effet au redémarrage — c'est le mécanisme
mika#1633 qui a rendu les swaps de modèle applicables sans re-provision.

### C4 — le temps est déjà le frein réel, et il borne le runaway

La mesure du ticket donne le débit : 8192 tokens en 123 s, soit **≈ 66 tok/s**.
Avec le plafond HTTP de mika-arch à 240 s (mika#2189), un appel ne peut
matériellement pas produire plus de **≈ 16 000 tokens**. Deux conséquences, et
c'est ce qui fixe la valeur à choisir (§ D1) :

1. le budget temps borne déjà le runaway — le ticket le dit et le code le
   confirme, donc relever le budget token n'ouvre pas une dépense non bornée ;
2. un plafond de 16384 serait **exactement** à la limite de l'atteignable et
   redeviendrait contraignant à la moindre accélération du modèle ou au moindre
   élargissement du plafond HTTP.

### C5 — la classe de panne est déjà nommée dans le code, mais seulement en calibration

`calibration/failure.rs:23-27` définit `FailureClass::ReasoningBudgetExhausted`,
avec en commentaire la remédiation exacte : *« Remediation is raising
`max_tokens` »* (mika#1665). `classify_failure` (`:120-124`) la distingue d'un
`EmptyResponse` par la conjonction `finish_reason_is_length && output_tokens > 0`
— précisément la signature du ticket. **Ce diagnostic n'existe que dans le module
`calibration/`.** En production, `agent_loop/mod.rs:1246` traite `MaxTokens`
comme un `EndTurn` : le tour sort avec un texte vide, sans un mot.

### C6 — ce que l'opérateur voit à la place, et pourquoi la cause a coûté trois essais

Côté aval, `dispatch-lib.sh` (~5224) garde le contenu **avant** de parser :

```sh
[ -n "$content1" ] && [ -n "$session_id" ] || {
    _groom_warn "first-pass response missing .content or .metadata.session_id"
    return 1
}
```

Un `content` vide sort donc immédiatement, sur un message qui parle d'un champ
JSON manquant. Deux effets : la reprise corrective mika#1823 (~5177-5210) n'est
**pas** atteinte — donc rien n'est gaspillé, bonne nouvelle — mais l'opérateur
lit un défaut de transport là où il y a un défaut de budget. Et il ne peut pas
faire mieux depuis là : `ask.rs:432-434` note explicitement que l'usage par run
n'est **pas** porté par le `Task` A2A. Le diagnostic ne peut vivre que côté
serveur.

### C7 — une donnée fausse dit le contraire du fix, au moment exact où on en a besoin

`llm/mod.rs:441` et `:443` donnent `DeepSeek => 8_192` et `Kimi => 8_192` dans
`max_output_tokens()`. Pour DeepSeek, c'est la valeur de l'ère R1 ; la valeur
actuelle est à trois ordres de grandeur. `provider.rs:117-121` s'en sert pour
avertir l'opérateur que son `llm_max_tokens` « dépasse la limite du provider » —
donc un opérateur qui applique ce ticket puis bascule sur DeepSeek reçoit un
avertissement qui contredit le fix, sur la base d'une donnée périmée.

**Portée exacte de cette table, pour ne pas lui prêter plus de pouvoir qu'elle
n'en a :** elle ne sert qu'à cet avertissement et à
`calibration/providers.rs:64`, où elle n'est pas contraignante non plus puisque
chaque scénario pose son propre `max_tokens` (§ C8). Elle ne clampe aucun appel
de production.

### C8 — le gate de calibration est plafonné à 2000 tokens de sortie

`calibration/roles/mika_dev.rs:121` (et ses cinq sœurs, et les suites arch/qa)
posent `max_tokens: 2000` par scénario, avec un commentaire qui dit déjà le
problème : *« reasoning-mode models can burn a 1000-token budget entirely on
internal reasoning before emitting visible content »* — mika#1665 avait répondu
en passant de 1000 à 2000. Or `docs/solutions/architecture-patterns/well-known-agent-config-toml-override.md`
et la règle mika#1190 imposent une calibration passante pour tout swap de modèle
d'un agent well-known. Le commentaire opérateur annonce #2296 comme le
« prérequis commun » de deux adoptions de modèles de raisonnement : si le gate
obligatoire de ces adoptions tourne lui-même à 2000 tokens de sortie, le ticket
promet un déblocage qu'il ne livre pas.

## Décisions

### D1 — 32768 pour mika-arch, et ce n'est pas une cible mais un plafond rendu non contraignant

Le but n'est pas que l'architecte émette 32 768 tokens : C4 montre qu'il ne le
peut pas en 240 s. Le but est que **le budget token cesse d'être le frein**, pour
que le seul frein restant soit le frein temporel — déjà borné, déjà mesuré, déjà
réglé par mika#2189. 16384 est refusé pour la raison donnée en C4 : il coïncide
avec l'atteignable et redeviendrait contraignant sans prévenir. 32768 laisse un
facteur 2 de marge et reste sous le seuil d'avertissement de
`validate.rs:73` (`> 32768`, donc 32768 pile ne déclenche rien) et loin de la
borne dure de `validation.rs:110` (131 072).

Sécurité côté fournisseur : `moonshotai/kimi-k2.5` sur OpenRouter annonce
262 144 tokens de complétion. 32768 est sûr par deux ordres de grandeur. (La
variante datée `-0127` plafonne à 65 535 — toujours au-dessus, et ce n'est pas le
slug configuré.)

### D2 — mika-arch seulement ; mika-dev est explicitement écarté, et il y a une raison mesurée

Le ticket dit « mika-arch d'abord, mika-dev avec sa bascule ». Ce n'est pas une
formule de prudence, c'est un piège documenté :
`well-known-agent-config-toml-override.md:56` écrit noir sur blanc que
`MIKA_DEV_CONFIG` déclare `openrouter` + `z-ai/glm-5.2` et que **« sa source a
dérivé de son runtime »** — les plans mika#2179 et mika#2189 mesurent mika-dev
sur `z-ai/glm-5.3`. Toucher `MIKA_DEV_CONFIG` déclenche `reconcile_well_known_config`
(C3), qui réécrit **tout le fichier** : on corrigerait le budget token en
rétrogradant le modèle de 5.3 vers 5.2, sans calibration et sans l'avoir demandé.

Réconcilier cette dérive est un travail à part entière (elle exige une
calibration mika-dev sur le modèle réellement en service). Ce plan ne l'ouvre
pas. Il l'écrit dans le fichier, à l'endroit où le prochain lecteur le lira
(M1b).

mika-qa est déjà à 16384 (`:185`) et relève de #2328.

### D3 — le diagnostic doit exister en production, pas seulement en calibration

C5 + C6 : le code sait nommer cette panne, et ne le fait qu'à un endroit où
aucune panne de production ne passe. Trois essais et quatre tickets ont été
dépensés à redescendre une chaîne de couches dont celle-ci était la dernière ; la
contre-mesure structurelle est qu'un `MaxTokens` à texte vide **se nomme
lui-même** dans le journal, avec le nom de la remédiation. C'est un `warn!`, pas
un changement de comportement : la valeur de D1 peut être dépassée un jour, et ce
jour-là le diagnostic doit être une ligne de `grep`, pas trois essais.

Corollaire assumé : le message de `dispatch-lib` (C6) est corrigé pour ne plus
accuser un champ JSON manquant, et pour nommer le `grep` côté serveur. Aucun
changement de protocole — `mika ask` ne porte pas l'usage (C6) et ce plan ne le
lui ajoute pas.

### D4 — corriger la donnée fausse de la table, et l'épingler à sa source

C7 : `DeepSeek => 8_192` est faux et ment exactement au moment où ce ticket rend
la question vivante. Correction à `65_536` — la valeur par défaut documentée en
mode thinking, choisie plutôt que le maximum de 384K parce que cette table se
décrit elle-même comme *« conservative »* et sert à avertir, pas à autoriser.

`Kimi => 8_192` est **laissé tel quel**. Le provider natif `Kimi` pointe
`api.moonshot.cn` (`llm/mod.rs:415`) et n'est pas la route utilisée par
mika-arch, qui passe par `OpenRouter` (déjà à 128 000). Corriger une valeur que
je n'ai pas mesurée sur sa propre route serait remplacer une donnée périmée par
une donnée inventée.

### D5 — le budget des scénarios de calibration doit laisser place au raisonnement

C8 : sans ce point, la promesse du commentaire opérateur est vide. Le budget
par scénario passe de `2000` à une constante nommée unique,
`CALIBRATION_SCENARIO_MAX_TOKENS = 8192`, partagée par les **quatre** suites de
rôles — `mika_dev`, `mika_arch`, `mika_qa` et `mika_orchestrator`. La quatrième
compte : elle porte cinq scénarios à `max_tokens: 2000` et c'est la suite du
rôle que mika#1641 est en train de transférer, donc celle qu'un swap de modèle
sollicitera ensuite.

Pourquoi 8192 et pas 32768 : les fixtures de calibration sont des briefs courts,
et le pré-vol du commentaire montre qu'un brief court se conclut en 6,7 s. Le
plafond n'a pas à absorber un raisonnement de plan de 30 KB, seulement à ne plus
couper au milieu d'un raisonnement de fixture. 8192 est un facteur 4 sur la
valeur actuelle et reste le plafond historique d'un tour d'agent, donc une valeur
dont on sait qu'elle ne surprend aucun fournisseur.

Une constante nommée plutôt que quatorze littéraux : le commentaire de mika#1665
à `mika_dev.rs:118-121` dit « parity with the other scenarios (2000) » — une
parité tenue à la main, entre littéraux, qui est précisément ce qui se défait.

### D6 — pas de calibration exigée pour ce changement, et la règle citée le dit

La règle mika#1190 et le doc de pattern portent sur le **provider/model** :
*« never change `config_toml`'s provider/model without a passing calibration
run »*. Ce plan ne change ni l'un ni l'autre. Exiger ici une passe de calibration
reviendrait à la faire tourner sous le budget de 2000 tokens que D5 est en train
de corriger — l'ordre serait à l'envers. La vérification qui vaut pour ce
changement est celle de V1 : une vraie passe arch sur un vrai plan.

## Modifications

| # | Fichier | Changement |
|---|---|---|
| **M1a** | `crates/mika-agent/src/well_known_agents.rs:1399` | `llm_max_tokens = 8192` → `32768`, avec le bloc de commentaire dérivant la valeur de la mesure (66 tok/s × 240 s ≈ 16 000 atteignables ⇒ 32768 non contraignant), dans la forme du bloc mika#2189 qui le suit. |
| **M1b** | `crates/mika-agent/src/well_known_agents.rs` (près de `MIKA_DEV_CONFIG`) | Commentaire nommant la dérive source/runtime de mika-dev (D2) et disant que relever son budget token exige d'abord de réconcilier son modèle. Aucun changement de valeur. |
| **M2** | `crates/mika-agent/src/agent_loop/mod.rs` (branche `LlmStopReason::MaxTokens`, ~`:1246`) | `warn!(event = "llm_reasoning_budget_exhausted", …)` quand `stop_reason == MaxTokens` **et** que le texte extrait est vide. Champs : `agent_id`, `provider`, `model`, `output_tokens`, `max_tokens`, `step`, `trace_id`, `session_id`, plus la remédiation en clair. Aucun changement de flux : le tour se termine comme avant. |
| **M3** | `skills/bundled/_shared/dispatch-lib.sh` (~5224) | Distinguer « `content` vide » de « `session_id` manquant » et, pour le premier, nommer la cause probable et le `grep` serveur (`llm_reasoning_budget_exhausted`). Message uniquement — la garde et son `return 1` sont inchangés. |
| **M4** | `crates/mika-common/src/llm/mod.rs:441` + `crates/mika-cli/src/tui/commands/handlers.rs:2033` | `DeepSeek => 8_192` → `65_536`, commentaire citant la source et la date ; le test qui épingle 8_192 suit la valeur. |
| **M5** | `crates/mika-agent/src/calibration/roles/{mod,mika_dev,mika_arch,mika_qa,mika_orchestrator}.rs` | `CALIBRATION_SCENARIO_MAX_TOKENS: u32 = 8192` dans `roles/mod.rs` ; les `max_tokens:` littéraux des scénarios des quatre suites la lisent (30 occurrences au total dans `roles/`, à trier entre scénarios de production et fixtures de test — seules les premières sont concernées). |

### Tests

| # | Test | Ce qu'il tient |
|---|---|---|
| T1 | `well_known_agents::tests::test_mika_arch_config_toml_is_valid_toml` (étendu) | `llm_max_tokens == 32768` — la valeur est un fait du dépôt, pas une intention de plan. |
| T2 | `well_known_agents::tests` (nouveau) | Aucune constante `config_toml` d'agent well-known ne déclare un `llm_max_tokens` sous 8192 sans commentaire adjacent. Attrape la régression par copier-coller d'un futur agent. |
| T3 | `agent_loop::tests` (nouveau) | Sur `MockLlmProvider` renvoyant `MaxTokens` + contenu vide + `output_tokens > 0`, le prédicat de M2 est vrai ; sur `MaxTokens` + texte non vide, il est faux. Le prédicat est extrait en fonction pure pour être testable sans capturer le journal. |
| T4 | `llm::tests` (étendu) | `DeepSeek.max_output_tokens() == 65_536`, avec la source en commentaire. |
| T5 | `calibration::roles::tests` (nouveau) | Tous les scénarios des quatre suites lisent `CALIBRATION_SCENARIO_MAX_TOKENS` — scan de source refusant un littéral `max_tokens: <n>` dans `roles/`. C'est un scan et pas une assertion de valeur parce que la régression ne rendrait aucun scénario faux : elle re-désynchroniserait la parité que D5 vient de rendre structurelle, et toutes les assertions de comportement resteraient vertes. |
| T6 | `skills/bundled/_shared/test-dispatch-lib.sh` (étendu) | Un `content` vide et un `session_id` manquant produisent deux messages distincts. |

## Contrat de vérification

- **V1 — une vraie passe.** Après déploiement, une passe `mika-arch-groom-ticket`
  sur un plan réel de 20–30 KB doit produire une `Disposition:` parsable. C'est
  la seule vérification qui teste ce que le ticket répare ; T1–T6 tiennent la
  forme, V1 tient le fond.
- **V2 — l'appel ne se transforme pas en 400.** Premier appel arch après
  redéploiement : pas d'erreur `max_tokens` côté OpenRouter. C4 dit que c'est
  très improbable (facteur 8 sous la limite annoncée), mais la table d'OpenRouter
  est connue pour annoncer parfois plus que le backend routé n'accepte, et un 400
  est immédiat et visible.
- **V3 — la réconciliation a bien eu lieu.**
  `grep config_reconcile.updated $MIKA_SPIRIT_LOG_FILE` doit montrer `mika-arch`
  au redémarrage suivant le déploiement. Son absence signifie que le provisioning
  est gelé (`MIKA_DISABLE_AGENT_PROVISIONING`) et que le changement de constante
  est **inerte** : le `config.toml` doit alors être édité à la main. Ce point est
  une branche réelle, pas une précaution — je n'ai pas pu lire `~/.mika` depuis
  ce worktree (refus de la politique d'accès), donc l'état du provisioning sur
  gentux est une inconnue que seule cette ligne lève.
- **V4 — le diagnostic est muet en régime nominal.**
  `grep llm_reasoning_budget_exhausted $MIKA_SPIRIT_LOG_FILE` : zéro ligne
  attendue après V1. Toute occurrence ultérieure sur mika-arch signifie que
  32768 est à son tour dépassé — et alors **la réponse n'est pas de relever
  encore** : deux plafonds franchis d'affilée diraient que le modèle de la panne
  est faux (la même règle d'arrêt que mika#2189 s'est donnée pour son propre
  plafond). La piste serait la taille du brief (#2295), pas le budget.

## Acceptance criteria

- **AC1** — `MIKA_ARCH_CONFIG` déclare `llm_max_tokens = 32768`, avec la
  dérivation de la valeur écrite dans le fichier ; T1 l'épingle.
- **AC2** — Une passe arch de premier tour sur un plan réel (20–30 KB) produit
  un `content` non vide contenant une ligne `Disposition:` parsable par
  `_parse_disposition` (V1).
- **AC3** — Un tour d'agent terminé sur `MaxTokens` avec un texte vide émet
  `llm_reasoning_budget_exhausted` dans `$MIKA_SPIRIT_LOG_FILE`, portant
  `output_tokens`, `max_tokens`, `model` et la remédiation ; le flux du tour est
  inchangé. T3 tient le prédicat.
- **AC4** — Le message d'échec de `dispatch-lib` sur un `content` vide nomme la
  cause probable et le `grep` d'AC3, et ne se confond plus avec l'absence de
  `session_id`. T6 tient la distinction.
- **AC5** — `ProviderKind::DeepSeek.max_output_tokens()` ne contredit plus un
  `llm_max_tokens` de 32768 ; la nouvelle valeur porte sa source en commentaire.
  T4 l'épingle.
- **AC6** — Les scénarios de calibration des quatre suites de rôles lisent une
  constante unique valant 8192, de sorte qu'un modèle à raisonnement puisse
  franchir le gate mika#1190. T5 refuse le retour d'un littéral.
- **AC7** — `MIKA_DEV_CONFIG` et `MIKA_QA_CONFIG` sont **inchangés**, et la
  raison (dérive source/runtime mesurée pour mika-dev, #2328 pour mika-qa) est
  écrite dans le fichier au point où un futur contributeur voudra les modifier.

## Definition of Done

- `cargo build`, `cargo test`, `cargo clippy` et `cargo fmt --check` passent.
- T1–T6 écrits et verts.
- `make verify-bundled-skills` passe (M3 touche `_shared/`).
- `scripts/verify-pipeline.sh` passe (section AC présente).
- AC1 et AC3–AC7 vérifiables dans l'arbre ; AC2 vérifié après déploiement (V1).
- La PR porte : la mesure de débit qui dérive 32768 (C4/D1), le refus explicite
  de toucher mika-dev avec sa raison (D2), et la branche V3 pour l'opérateur.

## Risques et hors périmètre

**Risque 1 — le provisioning peut être gelé.** Traité en V3, avec sa branche
d'action. C'est la seule inconnue que ce plan n'a pas pu lever depuis le
worktree, et elle est nommée plutôt que supposée résolue.

**Risque 2 — un appel plus long sur la moitié qui échoue.** Relever un budget de
sortie ne ralentit aucun appel qui réussissait déjà : on paie les tokens émis,
pas le plafond. Ce qui devient plus cher, c'est un appel qui part en raisonnement
long — et C4 montre qu'il est coupé par le plafond HTTP de 240 s bien avant
32768. Le coût maximal par appel est inchangé : c'est le temps, pas le token.

**Risque 3 — un modèle rapide peut vraiment atteindre 32768.** Le pré-vol
DeepSeek donne ~210–315 tok/s (8192 en 26–39 s) : à ce débit, 240 s permettent
50 000–75 000 tokens, donc pour ce modèle le plafond token redeviendrait le
frein. C'est acceptable — 32768 tokens de sortie DeepSeek restent bon marché —
mais c'est la raison pour laquelle AC3 existe : le jour où ce plafond mord, il
faut que la ligne de journal le dise au lieu de se lire comme un modèle
défaillant.

**Hors périmètre, délibérément :**

- **mika-dev** — sa dérive source/runtime doit être réconciliée avec une
  calibration sur le modèle réellement en service (D2). Ce plan écrit le piège
  dans le fichier et s'arrête là.
- **mika-qa / le retour de glm-5.3** — #2328.
- **L'adoption de deepseek-reasoner à la porte arch** — ce plan lève le blocage
  commun (budget token, et gate de calibration praticable) ; le swap lui-même
  reste soumis à mika#1190.
- **La taille du brief** — #2295, déjà mergé (`5a7a50fb`), couche précédente de
  la même chaîne.
- **Le port de `llm_max_tokens` en réglage par agent via env** — il l'est déjà
  par `config.toml`, qui est la granularité dont ce problème a besoin.
