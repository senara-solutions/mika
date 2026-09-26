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

Une précision qui compte pour la suite, et que le second commentaire opérateur
apporte en corrigeant le premier : ce pré-vol tournait sous un plafond de
**32768**, non de 8192. La ligne du bas ne dit donc pas « 8192 est trop petit »,
elle dit « ce modèle-là dépasse même 32768 » — d'où sa mise à l'écart (§ hors
périmètre) et non une rallonge. Le cas qui fonde ce ticket reste celui de
mika-arch sur kimi à 8192.

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

### C9 — le runtime de mika-arch est **déjà** à 32768, et la source ne l'est pas

Le commentaire opérateur du 2026-09-15 16:33Z l'écrit en passant, pour justifier
autre chose : *« mika-arch est **déjà à llm_max_tokens=32768** »*. La source, elle,
porte `8192` (`well_known_agents.rs:1399`, vérifié à `e85a0b46`). Il y a donc une
dérive source/runtime **sur l'agent que ce plan modifie** — la classe de piège que
D2 documente pour mika-dev, et que ce plan avait cru cantonnée à mika-dev.

Cette session ne peut pas la lever : le `~/.mika` visible ici est un HOME de
sandbox créé le jour même et ne contenant que `data/`, donc l'état de gentux est
hors d'atteinte. L'attestation de l'opérateur est la seule autorité disponible, et
elle suffit à trois conclusions.

**1. M1a change de nature : c'est un alignement de la source sur le runtime, pas
une correction de production.** La valeur qui répare la panne est déjà en vigueur,
posée à la main. Ce que M1a ajoute, c'est sa **durabilité**.

**2. Et c'est un renforcement, pas un affaiblissement.**
`reconcile_well_known_config` réécrit le `config.toml` d'un agent provisionné à
chaque passe de provisioning (C3). Une valeur manuelle de 32768 face à une
constante de 8192 est donc une valeur **non protégée** : au prochain redémarrage
avec provisioning actif, la source gagne, mika-arch retombe à 8192 et **la panne
du ticket revient**. M1a n'est pas cosmétique — c'est ce qui empêche la
régression.

**3. Le corollaire dit où en est le système, et il rend V3 décisive.** Si la
réconciliation avait tourné depuis l'édition manuelle, le runtime serait
retombé à 8192. Qu'il soit encore à 32768 signifie donc *soit* que le
provisioning est gelé (`MIKA_DISABLE_AGENT_PROVISIONING`), *soit* qu'aucun
redémarrage n'a eu lieu depuis. Les deux branches de V3 s'en trouvent précisées,
et elles ne demandent pas le même geste (V3, V0).

**Ce que cela impose avant la pose.** Modifier la constante déclenche la
réécriture de **tout** le fichier, pas du seul champ touché. Si le runtime de
mika-arch porte d'autres écarts manuels — le modèle au premier chef, comme pour
mika-dev — les aligner sur le budget token les écraserait au passage. D'où V0 :
recenser les écarts avant de laisser la réconciliation faire son travail. C'est
exactement le raisonnement de D2, appliqué cette fois à l'agent qu'on touche.

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

**M4 n'est pas la « rallonge d'enveloppe » que Prime a refusée**, et il faut
l'écrire parce que la confusion est facile : les deux chiffres sont des budgets de
sortie, et l'un monte à 65 536 dans la PR même où Prime dit non à une rallonge.
Ce que Prime a refusé (commentaire du 2026-09-15 16:33Z) est de relever
`llm_max_tokens` **au-delà de 32768** pour accommoder le raisonnement de
deepseek-reasoner. `max_output_tokens()` n'est pas `llm_max_tokens` : C7 établit
qu'elle n'alloue rien, ne clampe aucun appel de production, et ne sert qu'à
avertir l'opérateur quand son réglage dépasse ce que le provider accepte. La
corriger rend un avertissement exact ; elle n'accorde de budget à personne. Aucun
`llm_max_tokens` de ce plan ne dépasse 32768.

### D5 — le budget des scénarios de calibration doit laisser place au raisonnement

C8 : sans ce point, la promesse du commentaire opérateur est vide. Cette promesse
a rétréci de moitié entre les deux commentaires et le volet qui reste suffit :
le commentaire du 14:39Z annonçait #2296 comme « prérequis commun » de **deux**
adoptions, celui du 16:33Z en ferme une (deepseek-reasoner à la porte arch,
écarté par Prime). Reste le retour de glm-5.3 côté QA (#2328) — et le fait
général qu'un gate de calibration tournant à 2000 tokens de sortie ne peut valider
**aucun** modèle à raisonnement, ce qui est vrai indépendamment du modèle candidat
du jour. D5 ne dépend donc pas du volet clos.

Le budget par scénario passe de `2000` à une constante nommée unique,
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

Une constante nommée plutôt que trente littéraux : le commentaire de mika#1665
à `mika_dev.rs:117-120` dit « parity with the other scenarios (2000) » — une
parité tenue à la main, entre littéraux, qui est précisément ce qui se défait. Le
comptage fait pour la § Fire-Disposition montre qu'elle **s'est déjà défaite** :
trois scénarios du fichier qui invoque cette parité sont restés à 1000.

Un scénario échappe à l'uniformisation, et un seul : celui de `mika_qa.rs:1017`,
à 12000, dont le besoin est déjà mesuré et documenté sur place. Le descendre à
8192 serait la régression que ce plan prétend réparer ailleurs. Il devient une
seconde constante nommée, assortie d'une assertion qui exige sa suppression le
jour où elle devient superflue (§ Fire-Disposition).

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
| **M1a** | `crates/mika-agent/src/well_known_agents.rs:1399` | `llm_max_tokens = 8192` → `32768`, avec le bloc de commentaire dérivant la valeur de la mesure (66 tok/s × 240 s ≈ 16 000 atteignables ⇒ 32768 non contraignant), dans la forme du bloc mika#2189 qui le suit. Le commentaire dit aussi **pourquoi cette ligne protège une valeur déjà en service** : le runtime est à 32768 depuis une édition manuelle (C9), et sans cette constante la prochaine réconciliation le rétrograde à 8192. **Aucun autre champ de la constante n'est touché** (AC9) ; la réécriture porte sur tout le fichier, pas sur le seul champ modifié. |
| **M1b** | `crates/mika-agent/src/well_known_agents.rs` (près de `MIKA_DEV_CONFIG`) | Commentaire nommant la dérive source/runtime de mika-dev (D2) et disant que relever son budget token exige d'abord de réconcilier son modèle. Aucun changement de valeur. |
| **M2** | `crates/mika-agent/src/agent_loop/mod.rs` (branche `LlmStopReason::MaxTokens`, ~`:1246`) | `warn!(event = "llm_reasoning_budget_exhausted", …)` quand `stop_reason == MaxTokens` **et** que le texte extrait est vide. Champs : `agent_id`, `provider`, `model`, `output_tokens`, `max_tokens`, `step`, `trace_id`, `session_id`, plus la remédiation en clair. Aucun changement de flux : le tour se termine comme avant. |
| **M3** | `skills/bundled/_shared/dispatch-lib.sh` (~5224) | Distinguer « `content` vide » de « `session_id` manquant » et, pour le premier, nommer la cause probable et le `grep` serveur (`llm_reasoning_budget_exhausted`). Message uniquement — la garde et son `return 1` sont inchangés. |
| **M4** | `crates/mika-common/src/llm/mod.rs:441` + `crates/mika-cli/src/tui/commands/handlers.rs:2033` **et `:1947-1960`** | `DeepSeek => 8_192` → `65_536`, commentaire citant la source et la date ; le test qui épingle 8_192 suit la valeur. **Second test, dépendant mais pas épinglant :** `handlers.rs:1947` écrit `llm_max_tokens = 16384` et attend le message `exceeds deepseek's limit` — il a choisi 16384 *parce que* la limite valait 8192. À 65 536 la valeur ne dépasse plus rien, l'avertissement n'est plus émis et le test échoue. Sa fixture passe à `131072` (borne dure de `validation.rs:110`, donc la plus grande valeur qui reste légale). |
| **M5** | `crates/mika-agent/src/calibration/roles/{mod,mika_dev,mika_arch,mika_qa,mika_orchestrator}.rs` | `CALIBRATION_SCENARIO_MAX_TOKENS: u32 = 8192` dans `roles/mod.rs` ; **29 des 30** littéraux `max_tokens:` des quatre suites la lisent. Le trentième (`mika_qa.rs:1017`, valant 12000) garde sa valeur via une seconde constante nommée, `CALIBRATION_QA_VERDICT_BODY_MAX_TOKENS: u32 = 12_000` — l'uniformiser serait une régression, voir § Fire-Disposition. Les 30 occurrences sont toutes des scénarios de production : il n'y a aucune fixture de test à trier dans `roles/`, et les valeurs ne sont pas toutes à 2000 (24 × 2000, 3 × 1000, 1 × 1500, 1 × 3000, 1 × 12000). |

### Tests

| # | Test | Ce qu'il tient |
|---|---|---|
| T1 | `well_known_agents::tests::test_mika_arch_config_toml_is_valid_toml` (étendu) | `llm_max_tokens == 32768` — la valeur est un fait du dépôt, pas une intention de plan. |
| T2 | `well_known_agents::tests` (nouveau) | Aucune constante `config_toml` d'agent well-known ne déclare un `llm_max_tokens` sous 8192. Attrape la régression par copier-coller d'un futur agent. Prédicat strict (`< 8192`), et **sans clause d'échappement par commentaire** — voir § Fire-Disposition, qui la retire au profit d'un halt-and-surface. |
| T3 | `agent_loop::tests` (nouveau) | Sur `MockLlmProvider` renvoyant `MaxTokens` + contenu vide + `output_tokens > 0`, le prédicat de M2 est vrai ; sur `MaxTokens` + texte non vide, il est faux. Le prédicat est extrait en fonction pure pour être testable sans capturer le journal. |
| T4 | `llm::tests` (étendu) | `DeepSeek.max_output_tokens() == 65_536`, avec la source en commentaire. |
| T4b | `tui::commands::handlers::tests` (fixture corrigée) | Le test de l'avertissement de bascule de provider continue de vérifier qu'un `llm_max_tokens` **au-dessus** de la limite DeepSeek déclenche `exceeds deepseek's limit` — avec une fixture qui dépasse la nouvelle limite. Il tient le comportement, pas la valeur ; c'est pour cela qu'il est distinct de T4 et qu'il doit bouger avec M4 plutôt qu'être découvert rouge à la compilation. |
| T5 | `calibration::roles::tests` (nouveau) | Les scénarios des quatre suites lisent `CALIBRATION_SCENARIO_MAX_TOKENS`, à l'exception nommée près — scan de source refusant un littéral `max_tokens: <n>` dans `roles/`, les deux constantes nommées étant seules admises (§ Fire-Disposition). C'est un scan et pas une assertion de valeur parce que la régression ne rendrait aucun scénario faux : elle re-désynchroniserait la parité que D5 vient de rendre structurelle, et toutes les assertions de comportement resteraient vertes. Porte aussi l'assertion auto-nettoyante de l'exception. |
| T6 | `skills/bundled/_shared/test-dispatch-lib.sh` (étendu) | Un `content` vide et un `session_id` manquant produisent deux messages distincts. |

## Fire-Disposition

Deux livrables de ce plan sont de classe détecteur au sens de mika#1574 : **T2**
(scan des constantes `config_toml` well-known) et **T5** (scan des littéraux
`max_tokens:` dans `calibration/roles/`). Les deux sont des `#[test]` ordinaires,
donc **bloquants en CI par le job `cargo test` existant** — aucun nouveau job à
créer, et aucune possibilité de les lander verts mais inertes.

La disposition ci-dessous est écrite sur une population **comptée dans l'arbre à
`e85a0b46`**, pas supposée : une disposition qui annonce des exceptions sans avoir
compté ce qu'elle exempte est le défaut même que ce gate existe pour attraper. La
mesure a d'ailleurs contredit le plan sur deux points, reportés en M5.

### T2 — aucune constante well-known ne déclare `llm_max_tokens` sous 8192

**Population.** Trois constantes `config_toml` existent
(`well_known_agents.rs:167`, `:180`, `:1394`) : `MIKA_DEV_CONFIG` = 8192,
`MIKA_QA_CONFIG` = 16384, `MIKA_ARCH_CONFIG` = 8192 → **32768** par M1a. Les
agents dont la spec porte `config_toml: None` (`:85`, et tout futur agent sans
config) ne déclarent rien : ils héritent du défaut global et sont **hors
population**. T2 scanne des déclarations, jamais des absences — sans quoi il
firerait sur des agents qui n'ont pris aucune décision de budget.

**Violations existantes : zéro.** Après M1a, la plus basse déclaration est celle
de mika-dev à 8192, qui est **à** la borne et non en dessous ; le prédicat est
strict.

**Disposition : (a) allowlist nommée, avec une liste vide.** Le gate est bloquant
dès le land, sans exemption ni période de grâce, parce qu'il n'y a rien à
exempter. **Aucune exemption n'est écrite pour mika-dev ni mika-qa** : les deux
passent le scan, et exempter d'un scan ce qui le passe déjà crée une dispense
morte que plus rien ne nettoie — précisément la dette que le sous-point (3) de
l'option (a) cherche à éviter. Le finding F1 demandait une « exception nommée »
côté agents ; la mesure dit qu'il n'y a pas de population à exempter, et écrire
l'exemption quand même l'aurait rendue permanente sans jamais avoir été vraie.

**Si le scan fire malgré tout : (c) halt-and-surface.** Si le poseur découvre en
écrivant T2 une déclaration sous 8192 — constante ajoutée entre cette rédaction et
le land, ou déclaration hors des trois sites recensés — il **s'arrête et remonte à
l'opérateur** au lieu d'ajouter une exemption ou de relever la valeur lui-même.
Raison : le budget de sortie d'un agent well-known est solidaire de son modèle, et
relever celui d'un agent est exactement ce que D2 refuse de faire pour mika-dev
sans réconcilier d'abord sa dérive source/runtime. La résolution de la violation
est ici la décision de périmètre elle-même, ce qui est le critère d'emploi de
l'option (c).

**Corollaire, et c'est un durcissement de T2 :** la formulation initiale tolérait
une déclaration sous 8192 « avec un commentaire adjacent ». Cette clause est
retirée. N'importe quel commentaire désarmait le détecteur — c'est la disposition
la plus faible qu'un fire puisse recevoir, et elle rendait le scan décoratif au
premier cas réel. Le halt-and-surface ci-dessus la remplace.

### T5 — aucun littéral `max_tokens:` dans `calibration/roles/`

**Population comptée : 30 littéraux, tous dans du code de production** (les trois
modules `#[cfg(test)]` de `roles/` commencent en `mika_orchestrator.rs:658`,
`mika_arch.rs:1055` et `mika_qa.rs:1150`, après la dernière occurrence de chaque
fichier — il n'y a donc aucune fixture de test à trier, contrairement à ce que M5
supposait). Et ils ne valent **pas** tous 2000 :

| Valeur | Occurrences | Où |
|---|---|---|
| 2000 | 24 | les quatre suites |
| 1000 | 3 | `mika_dev.rs:308`, `:364`, `:444` |
| 1500 | 1 | `mika_arch.rs:285` |
| 3000 | 1 | `mika_arch.rs:217` |
| 12000 | 1 | `mika_qa.rs:1017` |

Cette dispersion est une preuve de D5 plus forte que celle qu'il invoquait : le
commentaire de mika#1665 à `mika_dev.rs:117-120` revendique *« parity with the
other scenarios (2000) »* alors que **trois scénarios du même fichier sont restés
à 1000**. La parité tenue à la main était déjà fausse dans le fichier qui s'en
réclame — et personne ne l'a vu, parce qu'aucune assertion ne la portait.

**Migrent sans exception — 29 littéraux.** Les valeurs 1000, 1500, 2000 et 3000
lisent `CALIBRATION_SCENARIO_MAX_TOKENS`. Toutes **montent** vers 8192, donc
aucune ne peut introduire la panne que ce ticket répare.

**Une exception nommée, et une seule : `mika_qa.rs:1017` (12000).** Son
commentaire mesure déjà exactement la classe de panne du ticket — *« a first run
at 2000 spent the whole budget on `reasoning_content`, returning empty text »* —
pour le seul scénario qui demande un corps de verdict complet. L'uniformiser
serait une **régression de 12000 à 8192 sur le seul scénario ayant mesuré son
propre besoin**. Elle est portée par une seconde constante nommée dans
`roles/mod.rs`, `CALIBRATION_QA_VERDICT_BODY_MAX_TOKENS: u32 = 12_000`, avec :

1. **la donnée qui la déclenche**, nommée dans le commentaire de la constante : le
   scénario de corps de verdict de la suite `mika_qa`, et la mesure qui l'a fixée
   à 12000 ;
2. **le suivi — et pourquoi ce n'est pas un numéro de ticket.** L'option (a)
   réclame un tracker parce qu'une exception y est normalement une dette
   temporaire. Ici ce n'en est pas une : le besoin de ce scénario est mesuré et
   durable, pas un reliquat de migration. Ouvrir un ticket « faire converger 12000
   et 8192 » créerait un suivi sans travail à faire, qui se fermerait sans rien
   changer — un faux signal de plus dans le backlog. L'écart à l'option (a) est
   assumé ici, et compensé par (3), qui est vérifiable et ne repose sur la
   vigilance de personne ;
3. **l'assertion auto-nettoyante**, portée par T5 : le test échoue si
   `CALIBRATION_QA_VERDICT_BODY_MAX_TOKENS <= CALIBRATION_SCENARIO_MAX_TOKENS`. Le
   jour où le budget partagé rattrape l'exception, celle-ci n'a plus d'objet et le
   test **exige sa suppression** au lieu de la laisser survivre en doublon muet.

Le scan refuse donc les littéraux et n'admet que ces deux constantes nommées, ce
qui est la propriété voulue : la valeur redevient modifiable en un point, et toute
divergence future doit se déclarer pour exister.

## Contrat de vérification

- **V0 — recenser la dérive avant de laisser la réconciliation écrire (C9).**
  Avant la pose, sur gentux :
  `diff <(sed -n '/MIKA_ARCH_CONFIG/,/^"#;/p' crates/mika-agent/src/well_known_agents.rs) ~/.mika/agents/mika-arch/config.toml`
  — ou à défaut une lecture du fichier. Attendu : le **seul** écart est
  `llm_max_tokens` (8192 en source, 32768 en runtime). Tout autre écart —
  `openrouter_model` au premier chef — signifie qu'un réglage manuel vit dans ce
  fichier et que la réconciliation déclenchée par M1a l'écrasera. Dans ce cas :
  **s'arrêter et remonter à l'opérateur**, comme D2 le fait pour mika-dev, au lieu
  de trancher dans la PR. Cette vérification ne peut pas être faite depuis le
  worktree de grooming (C9) ; elle appartient à la pose.
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
  est **inerte** — sans être sans effet pour autant : C9 montre que le runtime
  porte déjà 32768 à la main, donc dans cette branche la panne reste réparée en
  pratique et la constante devient une protection différée, qui s'appliquera le
  jour où le provisioning sera réactivé. Dans la branche inverse (réconciliation
  active), c'est M1a qui **empêche** la rétrogradation à 8192. Aucune des deux
  branches ne demande de geste d'urgence, mais elles ne disent pas la même chose
  et l'opérateur doit savoir laquelle il est. Ce point est une inconnue réelle,
  pas une précaution : le `~/.mika` visible depuis ce worktree est un HOME de
  sandbox (C9), donc l'état du provisioning sur gentux n'a pas pu être lu ici.
- **V4 — le diagnostic est muet en régime nominal.**
  `grep llm_reasoning_budget_exhausted $MIKA_SPIRIT_LOG_FILE` : zéro ligne
  attendue après V1. Toute occurrence ultérieure sur mika-arch signifie que
  32768 est à son tour dépassé — et alors **la réponse n'est pas de relever
  encore** : deux plafonds franchis d'affilée diraient que le modèle de la panne
  est faux (la même règle d'arrêt que mika#2189 s'est donnée pour son propre
  plafond). La piste serait la taille du brief (#2295), pas le budget.

  Cette règle d'arrêt n'est pas hypothétique : elle a **déjà été appliquée une
  fois**, le jour même. deepseek-reasoner dépasse 32768 sur un vrai brief, et la
  réponse de Prime (commentaire du 16:33Z) a été d'écarter le modèle, pas
  d'élargir l'enveloppe. C'est le précédent auquel se référer si la ligne
  apparaît un jour sur kimi.

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
  T4 l'épingle. L'avertissement de bascule de provider (`provider.rs:117`) reste
  fonctionnel — il se déclenche toujours au-dessus de la nouvelle limite, ce que
  T4b tient.
- **AC6** — Les scénarios de calibration des quatre suites de rôles lisent une
  constante partagée valant 8192, de sorte qu'un modèle à raisonnement puisse
  franchir le gate mika#1190 ; la seule exception est le scénario de corps de
  verdict de `mika_qa`, qui conserve 12000 via sa propre constante nommée et dont
  aucun budget ne baisse. T5 refuse le retour d'un littéral et échoue si
  l'exception devient superflue (§ Fire-Disposition).
- **AC7** — `MIKA_DEV_CONFIG` et `MIKA_QA_CONFIG` sont **inchangés**, et la
  raison (dérive source/runtime mesurée pour mika-dev, #2328 pour mika-qa) est
  écrite dans le fichier au point où un futur contributeur voudra les modifier.
- **AC8** — Aucune constante `config_toml` d'agent well-known ne déclare un
  `llm_max_tokens` sous 8192, sans clause d'exemption ; T2 l'épingle, et la
  découverte d'une violation existante arrête la pose au lieu d'être absorbée
  (§ Fire-Disposition).
- **AC9** — Dans `MIKA_ARCH_CONFIG`, la PR ne modifie **que** `llm_max_tokens`
  (plus du commentaire) : `openrouter_model`, `llm_provider`,
  `llm_http_timeout_secs` et `agent_total_timeout_secs` sont identiques à
  `e85a0b46`. Vérifiable au diff. La constante est réécrite en entier dans le
  `config.toml` de l'agent à la réconciliation (C3), donc un champ modifié par
  inadvertance ici est un réglage runtime écrasé là-bas — et le recensement de V0
  ne protège que ce qu'il a pu lire.

## Definition of Done

- `cargo build`, `cargo test`, `cargo clippy` et `cargo fmt --check` passent.
- T1–T6 (T4b inclus) écrits et verts.
- `make verify-bundled-skills` passe (M3 touche `_shared/`).
- `scripts/verify-pipeline.sh` passe (section AC présente).
- AC1, AC3–AC9 vérifiables dans l'arbre ; AC2 vérifié après déploiement (V1).
- **V0 exécuté avant la pose** : le seul écart entre `MIKA_ARCH_CONFIG` et le
  `config.toml` runtime de mika-arch est `llm_max_tokens`. Tout autre écart est un
  halt, pas un arbitrage de PR (C9).
- Les deux détecteurs (T2, T5) sont verts **sans exemption ajoutée en cours de
  pose** : une violation existante découverte par T2 est un halt, pas une
  exemption (§ Fire-Disposition).
- La PR porte : la mesure de débit qui dérive 32768 (C4/D1), le refus explicite
  de toucher mika-dev avec sa raison (D2), la branche V3 pour l'opérateur, et le
  fait que M1a protège une valeur runtime déjà posée à la main plutôt qu'elle
  n'en change une (C9).

## Risques et hors périmètre

**Risque 1 — l'état runtime de mika-arch est inconnu d'ici, sur deux axes.**
(a) Le provisioning peut être gelé : traité en V3, avec ses deux branches et ce
qu'elles impliquent. (b) Le `config.toml` de l'agent peut porter d'autres
réglages manuels que le `llm_max_tokens` attesté, et la réconciliation
déclenchée par M1a réécrit le fichier entier : traité en V0, dont l'issue est un
halt et non un arbitrage de PR. Les deux axes viennent de la même cause — le
`~/.mika` visible depuis ce worktree est un HOME de sandbox (C9) — et sont
nommés plutôt que supposés résolus. C'est la seule inconnue de ce plan.

**Risque 2 — un appel plus long sur la moitié qui échoue.** Relever un budget de
sortie ne ralentit aucun appel qui réussissait déjà : on paie les tokens émis,
pas le plafond. Ce qui devient plus cher, c'est un appel qui part en raisonnement
long — et C4 montre qu'il est coupé par le plafond HTTP de 240 s bien avant
32768. Le coût maximal par appel est inchangé : c'est le temps, pas le token.

**Risque 3 — un modèle rapide peut vraiment atteindre 32768, et le pré-vol
DeepSeek le montre sans qu'on puisse en chiffrer le débit.** Une rev antérieure
de ce plan dérivait ~210–315 tok/s de « 8192 tokens en 26–39 s ». Ce calcul est
retiré : le commentaire du 16:33Z établit que ce pré-vol tournait sous un plafond
de **32768**, pas de 8192, et les `output_tokens` réels des trois passes ne sont
pas au dossier. Diviser une latence par un budget qu'on suppose saturé donne un
chiffre, pas une mesure.

Ce qui subsiste est l'essentiel et ne dépend d'aucun de ces chiffres : ce modèle
a épuisé **32768** tokens de sortie en 26–39 s, là où kimi mettait 123 s pour en
épuiser 8192. Un plafond token peut donc être atteint bien avant le plafond
temporel de 240 s, et c'est précisément pourquoi AC3 existe : le jour où ce
plafond mord, la ligne de journal doit le nommer au lieu de se lire comme un
modèle défaillant.

**Hors périmètre, délibérément :**

- **mika-dev** — sa dérive source/runtime doit être réconciliée avec une
  calibration sur le modèle réellement en service (D2). Ce plan écrit le piège
  dans le fichier et s'arrête là.
- **mika-qa / le retour de glm-5.3** — #2328.
- **L'adoption de deepseek-reasoner à la porte arch — question CLOSE, pas
  différée.** Décision Prime du 2026-09-15 (commentaire 16:33Z) : le modèle est
  **écarté**, son `reasoning_content` dépassant 32768 sur un vrai brief, et
  aucune rallonge d'enveloppe n'est accordée pour l'accommoder. **kimi reste le
  socle arch.** Une rev antérieure de ce plan écrivait que « le swap reste soumis
  à mika#1190 » — c'est faux depuis cette décision : il n'y a plus de swap en
  attente d'un gate, il y a un candidat écarté. Ce plan ne débloque donc **pas**
  ce volet et ne doit pas être lu comme le faisant.
- **La taille du brief** — #2295, déjà mergé (`5a7a50fb`), couche précédente de
  la même chaîne.
- **Le port de `llm_max_tokens` en réglage par agent via env** — il l'est déjà
  par `config.toml`, qui est la granularité dont ce problème a besoin.

## Revision history

- **rev 3 (2026-09-15)** : re-groom sur le contexte enrichi du ticket — les deux
  commentaires opérateur, dont le second (16:33Z) porte une décision Prime que le
  plan ne pouvait pas connaître. Quatre corrections, dont une qui change la
  nature du livrable principal :
  - **C9 (nouveau) — le runtime de mika-arch est déjà à 32768, la source non.**
    L'opérateur l'écrit en passant ; la source porte 8192 (vérifié). M1a n'est
    donc pas une correction de production mais un **alignement source→runtime**,
    et c'est un renforcement : sans lui, la prochaine réconciliation (C3)
    rétrograde mika-arch à 8192 et **réintroduit la panne**. Corollaire utile :
    que le runtime soit encore à 32768 dit que le provisioning est gelé ou
    qu'aucun redémarrage n'a eu lieu — ce qui précise les deux branches de V3.
    Conséquences portées en M1a, V0 (nouveau), V3, AC9 (nouveau), Risque 1, DoD.
  - **M4 dissocié du refus de Prime.** « Pas de rallonge d'enveloppe » porte sur
    `llm_max_tokens` au-delà de 32768, pas sur `max_output_tokens()`, qui
    n'alloue rien (C7). La confusion est facile — deux budgets de sortie, dont
    l'un monte à 65 536 dans la PR même — donc elle est écartée explicitement en
    D4 plutôt que laissée au lecteur.
  - **Le volet DeepSeek est clos, pas différé.** Le hors-périmètre écrivait que
    « le swap reste soumis à mika#1190 » ; Prime a **écarté** le modèle. Corrigé,
    et D5 requalifié : il tenait sur « deux adoptions », il n'en reste qu'une
    (#2328) — plus la raison générale, qui ne dépend d'aucun candidat.
  - **Un chiffre retiré faute de mesure.** Le débit DeepSeek de ~210–315 tok/s
    supposait 8192 tokens en 26–39 s ; le pré-vol tournait à 32768 et ses
    `output_tokens` ne sont pas au dossier. Risque 3 garde le fait (ce modèle
    épuise 32768 en 26–39 s, là où kimi met 123 s pour 8192) et abandonne le
    ratio. Le § symptôme porte la même précision, puisque son tableau ne dit pas
    « 8192 est trop petit » mais « ce modèle dépasse même 32768 ».
- **rev 2 (2026-09-15)** : F1 traité par l'ajout d'une section
  `## Fire-Disposition` couvrant T2 et T5 (review-guide.md § Fire-Disposition
  Gate, mika#1574). Les deux détecteurs sont des `#[test]`, donc bloquants par le
  `cargo test` de CI sans nouveau job. La section a été écrite **après avoir
  compté la population de chaque scan**, et le comptage a contredit le plan sur
  quatre points, tous reportés dans le corps :
  - **T2, zéro violation existante.** Les trois seules constantes `config_toml`
    valent 8192 / 16384 / 32768 : aucune n'est sous 8192. F1 demandait une
    exemption nommée pour mika-dev et mika-qa ; elle n'est **pas** écrite, parce
    qu'exempter d'un scan ce qui le passe déjà crée une dispense morte que rien ne
    nettoie. Le halt-and-surface demandé par F1 est en revanche écrit, et couvre le
    cas où la mesure serait démentie à la pose.
  - **T2 durci.** La clause « sous 8192 *sans commentaire adjacent* » est retirée :
    n'importe quel commentaire désarmait le détecteur, ce qui est la disposition la
    plus faible possible sur un fire. Remplacée par le halt-and-surface. AC8 ajouté
    pour porter le détecteur, qu'aucun AC ne couvrait.
  - **T5, la population n'est pas uniforme.** 30 littéraux, pas tous à 2000 :
    24 × 2000, 3 × 1000, 1 × 1500, 1 × 3000, 1 × 12000. Le cas à 12000
    (`mika_qa.rs:1017`) reçoit l'**exception nommée** demandée par F1, avec
    l'assertion auto-nettoyante de l'option (a) — l'uniformiser aurait été une
    régression sur le seul scénario ayant déjà mesuré son besoin, c'est-à-dire la
    panne même que ce plan répare. M5, D5 et AC6 ajustés en conséquence.
  - **T5, pas de fixtures à trier.** M5 supposait un tri entre scénarios de
    production et fixtures de test ; les trois modules `#[cfg(test)]` de `roles/`
    commencent après la dernière occurrence de leur fichier, donc les 30 sont des
    scénarios de production. La supposition est retirée.
  - Écart assumé à l'option (a), déclaré dans la section : l'exception à 12000 ne
    cite **pas** de ticket de suivi. Ce n'est pas une dette temporaire, et un
    tracker « faire converger 12000 et 8192 » se fermerait sans rien changer. Le
    nettoyage est porté par l'assertion, qui ne dépend de la vigilance de personne.
    C'est l'arbitrage de l'architecte en seconde passe.
