---
title: "mika#2472 — la baseline mika-arch, sur le modèle qui tourne, et son candidat de repli"
type: chore
issue: 2472
parent: 2457
status: groomed-pending
revision: 2 (après mika-arch première passe ESCALATE F1–F5 et bearing Prime 2026-09-22)
date: 2026-09-22
---

# mika#2472 — la baseline mika-arch, sur le modèle qui tourne, et son candidat de repli

> **Ce que ce plan livre, en une phrase.** Il fait exister ce que `CLAUDE.md` l. 57
> et `Makefile` l. 117 nomment depuis mai sans que le répertoire existe — une
> **baseline `calibrate-mika-arch`** produite par le binaire, sur le modèle que
> l'instrument dit **en service** (`moonshotai/kimi-k3` via OpenRouter, pas le
> `kimi-k2.5` du dépôt) — puis passe **un** candidat de repli à cette porte
> (`openrouter/deepseek/deepseek-v4.1-flash`), et enregistre le verdict. **Aucune
> valeur de production ne bouge.** Le mécanisme de repli (AC7 Step 2 de #2457)
> reste hors de ce ticket, comme sa DoD l'exige.

---

## 1. Ce que l'instrument dit — les faits qui bornent ce plan

Tous relevés le 2026-09-22 sur `/var/log/mika/server.log`, hors corps de requête
(`grep <event> | grep -v 'llm request body'`) ; rien n'est repris de mémoire.

### 1.1 Le modèle en service n'est pas celui du dépôt

`llm_budget_resolved` pour `mika-arch`, dernière émission `2026-09-21T16:00:55Z` :

```
provider=openrouter (agent_config)   model=moonshotai/kimi-k3 (agent_config, clé openrouter_model)
http_timeout_secs=300 (process_env)  agent_total_timeout_secs=660 (process_env)
llm_max_tokens=32768 (agent_config)  reachable_output_tokens=11250  max_attempts=2
```

- `MIKA_ARCH_CONFIG` (`crates/mika-agent/src/well_known_agents.rs:1528`) déclare
  `openrouter_model = "moonshotai/kimi-k2.5"`, `llm_http_timeout_secs = 240`,
  `agent_total_timeout_secs = 900`. **Trois des cinq valeurs en service viennent
  d'ailleurs** : le modèle du `config.toml` sur disque (édité le 2026-09-18 21:01,
  « ESSAI MESURÉ, réversible », repli `config.toml.bak-20260918-2103-kimi-k25`),
  le plafond et l'enveloppe de `~/.mika/.env` (`MIKA_LLM_HTTP_TIMEOUT_SECS=300`,
  `MIKA_AGENT_TOTAL_TIMEOUT_SECS=660`).
- `MIKA_DISABLE_AGENT_PROVISIONING=1` est posé dans le même `.env` : c'est la
  branche « provisionnement gelé » du § R4 du plan #2457, désormais **établie**,
  plus soupçonnée. Le disque fait loi ; la constante du dépôt est décorative pour
  cet agent.

**Conséquence pour ce ticket : la baseline se mesure sur `kimi-k3`.** Une
baseline sur `kimi-k2.5` mesurerait un modèle qui ne tourne pas ; la question
que #2457 pose — « un repli dégrade-t-il ? » — n'a de sens que contre ce qui
rend les verdicts aujourd'hui. Si k3 est un jour réverti (le `.bak` existe pour
ça), la baseline se rétablit sur le modèle réverti : **la baseline suit le modèle
en service, pas la constante.** Le § 5 en fait un test.

### 1.2 La coupure que #2457 décrit est réelle, temporelle, et sur k3

`llm_call_cap_exhausted`, agent `mika-arch`, par jour :

| jour | modèle | plafond | coupures |
|---|---|---|---|
| 09-18 | (champ absent, binaire antérieur) | — | 8 |
| 09-19 | idem | — | 11 |
| 09-19 | `moonshotai/kimi-k3` | 300 | 1 |
| 09-20 | `moonshotai/kimi-k3` | 300 | 9 |
| 09-21 | `moonshotai/kimi-k3` | 300 | 20 |

Chaque ligne k3 porte `elapsed_ms ≈ 300 000`, `cause_is_timeout: true`,
`max_tokens: 32768`. Le modèle générait encore au plafond de temps. C'est la
grandeur que la calibration mesure **par appel** (`latency_ms` par scénario) et
c'est pour cela qu'elle est nécessaire ici — et non suffisante, voir § 1.4.

`turn_usage` mika-arch depuis le 09-18, par modèle et tranche de latence :

| modèle | `<120 s` | `120–240 s` | `>240 s` | dont `error` `>240 s` |
|---|---|---|---|---|
| kimi-k3 | 590 | 58 | 26 | 11 |
| kimi-k2.5 | 338 | 43 | 49 | 15 |

Aucun des deux n'est indemne au-delà de 240 s ; k3 y est moins souvent en
proportion. Ce tableau n'est pas un verdict, c'est le repère que les
`latency_ms` de la calibration devront pouvoir être lus contre.

### 1.3 Le candidat de repli : deux DeepSeek, une décision Prime, une seule classe écartée

**La décision qui borne ce choix, citée à la source** (mika#2296, commentaire de
samidarko, 2026-09-15T16:33:13Z, relu par `gh issue view` le 2026-09-22) :

> **deepseek-reasoner est ÉCARTÉ pour la porte arch.** Pré-vol : content VIDE 3/3
> sur briefs réels (23-30 KB) alors que mika-arch est **déjà à
> llm_max_tokens=32768** → le `reasoning_content` de deepseek-reasoner **dépasse
> 32768** sur un vrai brief de grooming. Prime : **pas de rallonge d'enveloppe**
> pour l'accommoder. **kimi reste le socle arch confirmé.**

La raison de l'écartement est mécanique et nommée : le *reasoning_content* d'un
modèle **raisonnant** déborde l'enveloppe de sortie. Elle mord sur la classe
raisonnante ; elle ne mord pas sur un modèle qui n'émet pas de `reasoning_content`.

Trois modèles ont été essayés ou gardés « pour le retour » sur cet agent :

| candidat | classe | trace | verdict pour ce ticket |
|---|---|---|---|
| `deepseek/deepseek-reasoner` | raisonnant | pré-vol 09-15 : contenu vide 3/3 sur briefs réels (mika#2296, ci-dessus) | **écarté**, décision Prime 2026-09-15, inchangée |
| `zai/glm-5.3` (`zai_model` conservé sur disque) | raisonnant | 2026-09-11 : `MaxTokens` à 32k, contenu vide (mika#2296) ; « pense > 11 min » (#2297) — commentaire du `config.toml` | **exclu** : même classe, même mécanisme ; un repli *au premier cut temporel* ne peut pas être plus lent que le primaire |
| `openrouter/deepseek/deepseek-v4.1-flash` | **non raisonnant** | 8 tours `turn_usage` sur mika-arch (1 le 09-11, 7 le 09-15) : 16 385 / 17 158 / 18 283 / 21 020 / 31 372 / 32 726 / 96 081 / **293 461** ms. Retiré le 09-15 09:09 pour le tour de 293 s sur #2310 ; le `config.toml` d'`arch-probe` écrit la porte de retour : *« DeepSeek redevient candidat après merge #2295 »* — #2295 est mergé (`5a7a50fb`, PR #2327) | **candidat** : hors de la classe écartée *pour la raison même qui a servi à écarter* ; c'est le « DeepSeek » du bearing 2026-09-21 (#2457) ; **même rail** que le primaire (une clé, une entrée d'allowlist proxy, un `llm_provider`) |
| `openrouter/moonshotai/kimi-k2.5` (`.bak` du 09-18) | raisonnant | modèle précédent, 49 tours > 240 s depuis le 09-18 | ce n'est pas un repli, c'est **le retour arrière du primaire** ; il n'entre pas dans la porte |

**Bearing Prime, 2026-09-22 (session canonique, en réponse à la première passe
arch) :** *« Candidat de repli = `openrouter/deepseek/deepseek-v4.1-flash`.
`deepseek-reasoner` reste écarté, pour la raison du 09-15 inchangée. »* Ce n'est
pas une réouverture du 09-15 ; c'est le 09-15 appliqué à un candidat d'une autre
classe. Le README (§ 4.2) porte ce tableau **et** la citation du 09-15.

**Le trade que le candidat porte, à mesurer et non à découvrir (Prime, même
réponse).** Les deux queues de flash (96 s, 293 s sur 8 tours) sont la chose que
« bascule auto à la première coupure » hériterait : une coupure-k3 nette peut
devenir une attente-longue-flash. La suite de calibration est mono-tir par
scénario (`_runs_per_scenario` = 1, DR-8) : elle ne rend pas une distribution par
scénario, mais elle rend **dix latences** sur dix fixtures. Le README rapporte
donc pour flash **min / médiane / max sur les dix scénarios**, à côté des huit
tours de production ci-dessus — jamais une seule moyenne. Un repli qui achète de
la latence de queue doit le dire avant que le mécanisme (AC7 Step 2) ne soit
conçu ; c'est ce que ce chiffre est là pour établir.

### 1.4 Ce que la calibration ne peut pas dire

`crates/mika-agent/src/calibration/roles/mika_arch.rs` fait **un appel** par
scénario (`provider.send_message`, `max_tokens = CALIBRATION_SCENARIO_MAX_TOKENS
= 8192`, pas de boucle d'outils), sur 10 fixtures de 500–3 000 tokens. Un groom
réel est multi-tours (gh_read, KG, plan de 30 Ko) sous 32 768. **Un modèle peut
passer 10/10 et se faire couper à 300 s en production** — c'est le même
avertissement que `docs/eval/calibration/mika-qa-2328/README.md` écrit pour
mika-qa, et il vaut ici. La calibration est la porte mika#1190 (*nécessaire*) ;
la mesure d'une nuit que #2457 demande reste l'autre moitié (*suffisante*), et
elle n'est pas ce ticket.

Le binaire charge `~/.mika/.env` (`load_dotenv`, `calibrate.rs:113`) puis
construit le provider avec `LlmTimeoutBudget::from_env()` : **la calibration
tourne sous le même plafond de 300 s que la production.** Un scénario coupé y
apparaît en `FailureClass::Timeout`, pas en silence. C'est une propriété, pas un
hasard, et le README la nomme.

---

## 2. Pourquoi le pilote bwrap ne peut pas produire les artefacts — et le siège retenu

Le sandbox du pilote ne détient **aucun secret**, par conception :
`_PILOT_SANDBOX_SECRET_ALLOWLIST=()` (`skills/bundled/_shared/dispatch-lib.sh:740`),
`~/.mika` n'est pas monté hors `~/.mika/data` (l. 86). `openrouter.ai` est bien
dans `HOST_ALLOWLIST` du proxy d'egress (`scripts/mika-pilot-egress-proxy:119`),
mais en tunnel CONNECT — sans injection de clé côté hôte, contrairement à
Anthropic et GitHub. `create_provider_from_spec` (`calibration/providers.rs:104`)
rend `None` sans `MIKA_OPENROUTER_API_KEY` → `calibrate` sort en 2 avant tout
appel. **Le confinement et la tâche sont structurellement incompatibles** ; ce
n'est pas un choix de confort.

Trois sièges ont été pesés, et portés au bearing (Prime, 2026-09-22) :

| siège | ce qu'il fait à l'artefact | verdict |
|---|---|---|
| (c) pilote bwrap d'abord, artefacts en pas de fin | test § 4.3 **rouge à l'ouverture** de la PR — viole le release-gate « PR ouverte + CI verte » | écarté |
| (b) orchestrator-CC commite les artefacts sur la branche avant `ready`, pilote bwrap ensuite (le plan v1) | fait de l'orchestrateur un **second auteur** des JSON à côté du binaire ; l'artefact n'est plus une mesure, c'est une mesure-plus-un-commit — *material-as-attested* sous une autre peau | écarté (mika-arch F2 + Prime) |
| **(a) un tenant `/mika` hors-bwrap sur l'hôte**, éphémère, scopé à ce ticket : il lit `~/.mika/.env`, exécute le pipeline entier (plan → work → review → PR), le binaire est le seul auteur des JSON, la PR naît verte | l'artefact reste une mesure pure ; le gate est respecté à l'ouverture | **bearing Prime : (a)** |

**Ce que (a) coûte, dit sans le vendre gratuit.** Hors-bwrap = hors du sandbox de
confinement : on échange l'isolation du pilote contre l'intégrité de l'artefact.
Prime pose deux conditions, et le feu vert n'est pas le sien :

1. Le tenant est **éphémère et scopé** — il naît pour ce ticket, ouvre la PR,
   meurt. Ce n'est pas un siège hors-bwrap durable ni un chemin d'exécution
   permanent non confiné.
2. Faire lire des clés LLM à un tenant hors du sandbox est **une dérogation de
   substrat de sécurité qui appartient à Vincent**. Elle est surfacée à
   l'opérateur ; le ticket reste `blocked` jusqu'au feu vert, et **le label
   `ready` n'est jamais posé** sur ce ticket — il enverrait un pilote bwrap qui
   sortirait en 2 (§ 3.1, contrôle négatif) et ne pourrait que fabriquer.

Le corps de l'issue porte la dérogation dans un encadré daté (convention
mika#2169/#2158) une fois le feu vert donné ; le spawn s'appuie sur
`/mika-spawn` avec le `/mika` de ce ticket comme premier prompt, dans le worktree
de cette branche (précédent d'exécution hors-bwrap sur worktree : mika#2248 /
#2249, avec la même précaution — `blocked`, pas de `ready`, aucune tâche moteur
en vol, vérifié dans `mika.db`).

**Garde-fou absolu pour le tenant, écrit ici et répété dans le README :**
- Le tenant **n'écrit, ne modifie, ne « complète »** aucun fichier `.json` sous
  `docs/eval/calibration/` autrement que par une invocation du binaire
  `calibrate`. Un artefact de calibration n'a qu'un auteur : le binaire. Un
  JSON rédigé à la main est une preuve fabriquée — c'est la faute que
  `mika-orchestrator-1641/README.md` refuse en toutes lettres. AC7 le rend
  vérifiable (un seul commit auteur des JSON, dont le message nomme le binaire).
- Si `calibrate` sort en 2 après « Verifying provider authentication » dans le
  tenant : clé ou réseau — **halte**, pas de contournement, pas de PR.

**Surface manquante, nommée sans ticket (n=1).** Le sandbox ne peut appeler
aucun provider hors Anthropic avec une clé. Ce plan le contourne par l'hôte ;
il ne le corrige pas. Si un second ticket bute sur la même marche, c'est le
signal (n=2) d'une injection de clé OpenRouter côté proxy, sur le modèle de
`mika-pilot-github-auth-addon.py`.

---

## 3. La campagne (exécutée par le tenant, dans `/ce:work`, avant tout README de résultats)

Depuis le worktree de cette branche, binaire construit depuis **cette** branche
(pas `target/release/calibrate` du checkout principal, daté du 09-21 17:30 :
la suite `SCENARIOS` doit être celle que le test du § 4.3 lira) :

```bash
cargo build --release --bin calibrate --features telemetry   # une seule build à la fois (cap disque)
```

### 3.1 Contrôle négatif de la porte, à blanc

```bash
make calibrate-mika-arch MODEL=openrouter/moonshotai/kimi-k3 ; echo "exit=$?"
```

Attendu **avant** ce plan : `GATE NOT ENFORCEABLE … Exiting 2`. C'est le rouge
qui prouve que la cible `make` était inerte — à coller dans le README (§ 4.2).
Ne pas le sauter : sans lui, « la cible marche maintenant » et « la cible a
toujours marché » sont indistinguables.

### 3.2 Baseline du primaire

```bash
target/release/calibrate --role mika-arch --model openrouter/moonshotai/kimi-k3 \
  --establish-baseline \
  --baseline docs/eval/calibration/baselines/mika-arch.json \
  --output   docs/eval/calibration/mika-arch-2472/baseline-kimi-k3/artifact.json
```

- **Attendu :** `BaselineEstablished`, exit 0, 10/10. Le binaire écrit le JSON
  aux **deux** chemins (`establish_target = --baseline`, `calibrate.rs:362` ;
  artefact + `.md` à `--output`).
- **Halte `BaselineRefused` (exit 1)** : un scénario rouge **sur le primaire**.
  Ne pas passer `--force-failing-baseline`. Lire `error_class` dans l'artefact
  de `--output` (il est écrit même en cas de refus) :
  - `timeout` / `empty_response` avec `output_tokens` collé à 8192 et
    `stop_reason = MaxTokens` → le budget de calibration (8192,
    `roles/mod.rs:36`) étouffe le raisonnement de k3 sur **cette** fixture.
    C'est un fait sur le **cadre**, et la doctrine mika#2296 D5 dit quoi en
    faire — *« its need is measured on the spot, not assumed »* : le tenant
    ajoute pour le ou les scénarios concernés une **exception nommée et mesurée**,
    `CALIBRATION_ARCH_<SCENARIO>_MAX_TOKENS` dans `roles/mod.rs`, au niveau que
    l'artefact a mesuré nécessaire (borne : 32 768, la valeur de production de
    `MIKA_ARCH_CONFIG` — jamais au-delà, sinon la calibration mesurerait un
    régime que la production n'a pas), avec le `const _: () = assert!(… >
    CALIBRATION_SCENARIO_MAX_TOKENS)` auto-nettoyant sur le gabarit de
    `CALIBRATION_QA_VERDICT_BODY_MAX_TOKENS`, et relance § 3.2. L'artefact du
    run refusé est **conservé** dans `baseline-kimi-k3/refused-at-8192/` : il est
    la mesure qui justifie l'exception. Pas de `--force-failing-baseline`, pas de
    dormeur : le chemin existe et il est dans le périmètre (cadre de calibration,
    pas production — AC6 intact).
  - `contract_violation` / `fabrication` → le modèle **en service** échoue au
    contrat arch. C'est un résultat, et il monte au bearing (Prime) avant tout
    geste : la baseline ne s'établit pas sur un primaire qui rate sa propre
    porte, et le repli n'a plus de référence.
- **Halte `exit 2` après « Verifying provider authentication »** : clé ou
  réseau, pas le modèle. Régler, relancer.

### 3.3 Candidat de repli, porté à la porte

```bash
target/release/calibrate --role mika-arch --model openrouter/deepseek/deepseek-v4.1-flash \
  --baseline docs/eval/calibration/baselines/mika-arch.json \
  --output   docs/eval/calibration/mika-arch-2472/candidate-deepseek-v4.1-flash/artifact.json
echo "exit=$?"
```

- **exit 0 (`Pass`)** : le candidat franchit la porte mika#1190 sur la suite.
  Il ne devient **pas** repli pour autant — le mécanisme n'existe pas (AC7
  Step 2) et la mesure de nuit de #2457 n'est pas faite. Le README l'écrit.
- **exit 1 (`FailFloor`)** : le candidat rate un ou plusieurs scénarios.
  Commiter l'artefact tel quel — un rouge est un résultat — et l'écrire au
  README : *« deepseek-v4.1-flash ne franchit pas la porte arch sur
  `<scénarios>` ; sans candidat, AC7 Step 2 reste bloqué »*. Le ticket #2472
  ferme quand même (sa DoD est la baseline + le verdict, pas un candidat vert) ;
  #2457 garde sa condition de réveil.

### 3.4 Lecture par scénario, à coller au README

```bash
for d in baseline-kimi-k3 candidate-deepseek-v4.1-flash; do
  echo "== $d"
  jq -r '.providers["mika-arch"].scenarios | to_entries[]
         | "\(.key)\t\(.value.outcome)\t\(.value.latency_ms)\t\(.value.output_tokens)"' \
     docs/eval/calibration/mika-arch-2472/$d/artifact.json
done
```

**Ce n'est pas un seuil, c'est une mesure** (même clause que mika-qa-2328) : un
ratio de latence sur une fixture de 1 000 tokens prédit mal un brief de 30 Ko.
Le chiffre sert à dire *combien* de marge les 300 s laissent, pas à trancher.

### 3.5 Le commit des artefacts — un seul, dont le message nomme l'auteur

```bash
git add docs/eval/calibration/baselines/mika-arch.json docs/eval/calibration/mika-arch-2472/
git commit -m "docs(eval): mika-arch — baseline kimi-k3 et candidat deepseek-v4.1-flash, artefacts écrits par \`calibrate\` (mika#2472)"
```

Séparé des commits de code/README pour que AC7 (`git log -- '**/*.json'` rend un
seul commit) reste lisible.

Coût attendu : 20 appels, ≤ 8192 tokens de sortie chacun ; k3 ≈ 5× k2.5 d'après
le commentaire du `config.toml` — de l'ordre de 1–2 USD, sous le `--_max_cost_usd`
par défaut (5.0). Durée : borne haute 20 × 300 s ≈ 100 min si tout est coupé ;
attendu 5–15 min d'après les latences du § 1.3.

---

## 4. Ce que le tenant livre, une fois les artefacts sur la branche

### 4.1 `Makefile` — la cible arch pointe une baseline par rôle

```make
calibrate-mika-arch: ## Pre-swap calibration gate for mika-arch (MODEL=provider/model required)
	@if [ -z "$(MODEL)" ]; then …; exit 1; fi
	cargo run --bin calibrate --release -- --role mika-arch --model "$(MODEL)" --baseline docs/eval/calibration/baselines/mika-arch.json
```

C'est le chemin que le docstring du binaire donne déjà en exemple
(`calibrate.rs:8`). Le `latest.json` partagé était une impossibilité de
principe : un artefact ne porte qu'un rôle (`providers` est indexé par rôle,
`to_scenario_outcome(&args.role, …)`), et un fichier « dernier » commun à quatre
rôles serait écrasé par le dernier rôle calibré. **Les trois autres cibles ne
bougent pas** — elles n'ont pas de baseline à pointer, et les leur donner est un
ticket par rôle avec sa propre campagne. Le README `baselines/` (§ 4.4) nomme
la dette pour qu'elle ne soit pas redécouverte.

### 4.2 `docs/eval/calibration/mika-arch-2472/README.md`

Sur le gabarit de `mika-qa-2328/README.md`, en français, avec :

1. **Étape 0 — l'instrument** : les deux lignes du § 1.1 (`llm_budget_resolved`
   citée verbatim, `stat` du `config.toml`), le tableau des coupures § 1.2. Ce que
   ça établit : *k3 est le primaire, le plafond est 300 (process_env), le dépôt
   dit autre chose et c'est mesuré, pas corrigé* — corriger la constante est le
   § R4 de #2457 et la surface `mika agents budget` (PR #2461), pas ce ticket.
2. **Ce que la calibration ne peut pas dire** (§ 1.4), et la propriété
   « même plafond que la production ».
3. **Réconciliation du repli** : le tableau § 1.3, tel quel.
4. **Le rouge de la cible `make`** (§ 3.1, sortie collée) puis les deux runs
   (§ 3.2, 3.3) avec **exit code** et la lecture par scénario (§ 3.4), en
   tableaux.
5. **Ce qui ne change pas** : `~/.mika/agents/mika-arch/config.toml`,
   `MIKA_ARCH_CONFIG`, `.env`, aucun swap, aucun mécanisme de repli. Et la
   phrase de sortie : *ce que AC7 Step 2 attend maintenant* (mécanisme +
   mesure de nuit), avec le pointeur vers #2457.
6. **Le garde-fou** du § 2 (aucun JSON n'est écrit à la main ; auteur unique =
   binaire), et la règle « la baseline suit le modèle en service ».

### 4.3 Test structurel — la baseline couvre la suite courante

Dans `crates/mika-agent/src/calibration/roles/mika_arch.rs`, `mod tests`, à
côté de `scenario_count_is_ten` :

```rust
/// mika#2472: a repo↔artifact COHERENCE test, not a fixture test. The fixtures
/// under `tests/eval/calibration_fixtures/` are frozen INPUTS; the file read here
/// is a live OUTPUT of the `calibrate` binary that is re-established whenever the
/// model in service changes. So this test can legitimately go red for a reason
/// outside this crate's code, and that is its purpose. Exactly two reds are
/// legitimate:
///
///   1. the baseline is absent — `make calibrate-mika-arch` is inert again
///      (it pointed at a path that never existed from #1190 to #2472 and exited 2
///      on every call);
///   2. a scenario was added to SCENARIOS without re-establishing the baseline —
///      the gate would compare the new scenario against nothing (the "8 vs 5"
///      drift mika-qa-2328 documents).
///
/// Fix for either: re-run § 3.2 of the mika#2472 plan on the model in service
/// and commit what the binary wrote. Do NOT edit the JSON by hand, do NOT
/// allowlist the missing scenario — its only author is the `calibrate` binary.
#[test]
fn mika2472_the_arch_baseline_covers_the_current_suite() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/eval/calibration/baselines/mika-arch.json");
    let baseline = CalibrationArtifact::load(&path)
        .unwrap_or_else(|e| panic!("mika#2472: arch baseline unreadable at {}: {e}", path.display()));
    let cal = baseline.providers.get("mika-arch").expect("baseline carries the mika-arch role");
    let want: BTreeSet<&str> = SCENARIOS.iter().map(|s| s.id).collect();
    let have: BTreeSet<&str> = cal.scenarios.keys().map(String::as_str).collect();
    assert_eq!(have, want, "baseline scenario set must equal the current SCENARIOS");
    for (id, s) in &cal.scenarios {
        assert_eq!(s.outcome, "pass", "baseline scenario {id} must be a pass (#1701: a failing baseline lowers every future bar)");
    }
    // Bonne foi: the set comparison is live, not vacuous — a synthetic extra id trips it.
    let mut drifted = have.clone();
    drifted.insert("mika2472_synthetic_scenario_that_does_not_exist");
    assert_ne!(drifted, want);
}
```

Ce que ça pine, et ce qui rougit :
- baseline absente → la cible `make` est redevenue inerte (exit 2 silencieux) ;
- scénario ajouté sans re-baseline → comparaison à vide ;
- outcome ≠ `pass` → quelqu'un a passé `--force-failing-baseline` ;
- le contrôle de bonne foi distingue « les ensembles sont égaux » de « la
  comparaison ne compare rien ».

Le test lit un fichier du dépôt sans réseau ni clé, comme `fixture_*` — mais
l'analogie s'arrête au mécanisme de lecture : une fixture est une **entrée
figée**, la baseline est une **sortie vivante** du binaire. C'est un test de
cohérence dépôt↔artefact, et ses deux rouges légitimes sont nommés dans son
doc-comment comme propriété, pas laissés à découvrir. Rouge sur `main` tant que
cette PR n'est pas mergée : voulu, et c'est pourquoi les artefacts et le test
voyagent dans la **même** PR (§ 2).

### 4.4 `docs/eval/calibration/baselines/README.md`

Court : convention `<role>.json`, auteur unique = `calibrate --establish-baseline`,
« la baseline suit le modèle en service », et la ligne de dette : *mika-dev,
mika-qa, mika-orchestrator n'ont pas de fichier ici ; leurs cibles `make`
pointent encore `latest.json` et sortent en 2* — avec le pointeur vers les
campagnes existantes (`mika-dev-1633/`, `mika-qa-1632/`, `mika-qa-2328/`) qui
pourraient être promues, chacune par son ticket — **et vers le ticket de suivi
du § 4.5**, pour que la dette ait un propriétaire et pas seulement un README.

### 4.5 Ticket de suivi — les trois autres cibles `make`

Le tenant file, au moment de la PR, **un** ticket sur `senara-solutions/mika`
(preuve : `Makefile` l. 113/121/125 pointent `baselines/latest.json`, le rouge
§ 3.1 vaut pour chacune) : *« repointer `calibrate-mika-dev|qa|orchestrator` vers
`baselines/<role>.json` — chacune parquée jusqu'à sa campagne de baseline »*,
avec une condition de réveil concrète par rôle (« quand une baseline
`<role>.json` est établie par le binaire sur le modèle en service »). Le ticket
est un dormeur visible ; il ne rouvre pas la question ici. La PR et le README
`baselines/` pointent son numéro.

### 4.6 `CLAUDE.md` racine l. 57 et `crates/mika-agent/CLAUDE.md` l. 1682

Une phrase : *« Baselines live at `docs/eval/calibration/baselines/<role>.json`
(mika-arch since #2472; other roles pending) »*. Puis `scripts/sync-agent-docs.sh`
si le job `docs-sync` le réclame.

---

## 5. Acceptance Criteria

| AC | Critère | Vérification |
|---|---|---|
| AC1 | `docs/eval/calibration/baselines/mika-arch.json` existe sur la branche, produit par `calibrate --establish-baseline` sur `openrouter/moonshotai/kimi-k3`, 10/10 `pass` ; le `.md` et l'artefact jumeau sous `mika-arch-2472/baseline-kimi-k3/` | `jq '.providers["mika-arch"].model' == "openrouter/moonshotai/kimi-k3"` ; test § 4.3 vert |
| AC2 | `mika-arch-2472/candidate-deepseek-v4.1-flash/artifact.{json,md}` issus d'un run **porté** contre AC1 ; l'exit code du run figure au README ; le README cite la décision Prime du 2026-09-15 (mika#2296) et dit pourquoi elle ne s'applique pas à `v4.1-flash` (classe non raisonnante) | README § 4 et § 3 ; `jq '.providers["mika-arch"].model'` du candidat |
| AC3 | `make calibrate-mika-arch MODEL=…` ne sort plus en 2 sur un checkout frais de la branche (la cible pointe `baselines/mika-arch.json`) ; les trois autres cibles sont **inchangées** | `git diff main -- Makefile` ne touche que la cible arch ; § 3.1 rouge-avant collé au README |
| AC4 | README `mika-arch-2472/` conforme au § 4.2 (six sections), tableau de réconciliation § 1.3 inclus | lecture |
| AC5 | `mika2472_the_arch_baseline_covers_the_current_suite` présent, vert, avec son contrôle de bonne foi | `cargo test -p mika-agent calibration::roles::mika_arch` |
| AC6 | Rien en production ne bouge : `config.toml` de mika-arch, `MIKA_ARCH_CONFIG`, `.env` intacts (le cadre de calibration, `roles/mod.rs`, relève d'AC11) ; `test_mika_arch_config_toml_is_valid_toml` et `mika2280_the_three_shipped_geometries_and_their_verdict` verts **sans modification** | `git diff main --stat` ; `cargo test -p mika-agent well_known_agents` |
| AC7 | Aucun `.json` sous `docs/eval/calibration/` n'est écrit ou modifié autrement que par le binaire — un seul commit touche ces fichiers, et son message nomme `calibrate` | `git log --format=%s -- 'docs/eval/calibration/**/*.json'` sur la branche : un seul commit, celui du § 3.5 |
| AC8 | `CLAUDE.md` (racine + crate) disent le chemin vrai (§ 4.6) ; `docs-sync` vert | CI |
| AC9 | Le README rapporte pour le candidat **min / médiane / max** de `latency_ms` sur les dix scénarios, à côté des huit tours de production (queues 96 s / 293 s) — jamais une moyenne seule | lecture, chiffres croisés avec `jq` § 3.4 |
| AC10 | Le ticket de suivi § 4.5 existe, avec une condition de réveil par rôle ; la PR et `baselines/README.md` le référencent | `gh issue view <n>` |
| AC11 | Si une exception de budget a été nécessaire (§ 3.2), elle est nommée, mesurée (artefact `refused-at-8192/` commité), bornée à 32 768, et porte son `const _: () = assert!` auto-nettoyant ; sinon, `roles/mod.rs` est intact | `git diff main -- crates/mika-agent/src/calibration/roles/mod.rs` |

**Hors périmètre, nommé pour ne pas être redécouvert :** le mécanisme de repli
(AC7 Step 2 de #2457) ; la mesure de nuit de #2457 ; la correction de
`MIKA_ARCH_CONFIG` vers k3 (§ R4 de #2457, et PR #2461 pour la surface de
lecture) ; les baselines des trois autres rôles ; l'injection de clé OpenRouter
dans le proxy d'egress (§ 2, n=1).

---

## 6. Contrat de vérification

| Unité | Ce qui est asserté | Ce qui rougit si on se trompe |
|---|---|---|
| § 3.1 | La cible `make` sort en 2 **avant** le changement | « a toujours marché » indistinguable de « marche maintenant » |
| § 3.2 | `BaselineEstablished`, exit 0, JSON aux deux chemins | un `--establish-baseline` qui n'écrit que l'un des deux |
| § 4.3 | Ensemble des scénarios de la baseline == `SCENARIOS`, tous `pass` ; contrôle de bonne foi | baseline absente / dérive de suite / `--force-failing-baseline` |
| AC6 | Tests de valeur existants verts **sans diff** | une valeur de production touchée « en passant » |
| AC7 | Un seul commit auteur des JSON, message nommant le binaire | un artefact « complété » à la main |
| § 2 | Le tenant est hors-bwrap **et** `ready` n'est jamais posé ; `mika.db` ne porte aucune tâche moteur sur #2472 au spawn | un pilote bwrap dispatché en parallèle, exit 2, ou pire : un JSON fabriqué |

---

## Fire-Disposition

Un seul détecteur nouveau : `mika2472_the_arch_baseline_covers_the_current_suite`
(§ 4.3). Sujet : **le fichier `baselines/mika-arch.json` du dépôt**, une donnée
existante au sens de la doctrine.

- **Population pré-existante à HEAD (`2b5456cc`) : zéro.** Le répertoire
  `docs/eval/calibration/baselines/` n'existe pas ; le test serait rouge sur
  `main` aujourd'hui, et c'est le rouge-avant voulu. Il devient vert par la
  phase B, pas par une exception.
- **Disposition : (a) allowlist nommée, livrée VIDE.** Aucune entrée à exempter,
  donc aucun tracker de suivi ni assertion auto-nettoyante à écrire.
- **Conduite quand le test tire plus tard :** on re-établit la baseline sur le
  modèle en service (§ 3.2) ; on n'édite pas le JSON, on n'allowliste pas le
  scénario manquant. Écrit dans le doc-comment du test.

Les détecteurs existants touchés par ce plan : aucun (la cible `make` n'est pas
un test ; `test_mika_arch_config_toml_is_valid_toml` et `mika2280_…` sont
lus, non modifiés). Si le § 3.2 impose une exception de budget, elle arrive avec
son propre `const _: () = assert!` auto-nettoyant — la forme que mika#2296 T5 a
retenue **à la place** d'un ticket de suivi, et pour la même raison (un ticket
« converger X et 8192 » fermerait sans rien changer).

---

## Références

- mika#2457 — plan `docs/plans/2026-09-21-002-fix-2457-verdict-arch-sous-le-plafond-plan.md` (branche PR #2461), § 3 : « (a) baseline k2.5, (b) repli, (c) mécanisme — trois choses, dans cet ordre ». Ce plan livre (a) — sur k3, l'instrument ayant tranché — et (b).
- mika#2296, commentaire samidarko 2026-09-15T16:33:13Z — décision Prime : `deepseek-reasoner` écarté de la porte arch (`reasoning_content` > 32768), kimi socle. Bearing Prime 2026-09-22 (session canonique) : candidat = `deepseek-v4.1-flash`, mesurer la distribution de latence ; siège (a) sous feu vert Vincent.
- mika#1190 — `CLAUDE.md:57`, la porte ; mika#1701 — `calibration/gate.rs`, le contrat d'exit code.
- mika#2296 — `roles/mod.rs:36` (8192) et `well_known_agents.rs:1530` (32768) : les deux budgets et pourquoi ils diffèrent.
- `docs/eval/calibration/mika-qa-2328/README.md` — le gabarit de protocole, et le piège `latest.json`.
- `docs/eval/calibration/mika-orchestrator-1641/README.md` — « un JSON écrit à la main est une preuve fabriquée ».
