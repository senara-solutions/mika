# Plan — mika#2309 : le pré-vol (d) devient une garde CI, et la mesure rectifie sa règle

> **Ticket :** `mika issue#2309` — « Transformer le pré-vol (d) en test CI ».
> **Dépendances :** #2295 (livré, `0a946b95`/`5a7a50fb`), #2298 (livré, `289ea6fe`), relie #2297.
> **Type :** garde permanente (CI). Substrat borné, pas de changement de comportement runtime.

---

## Objet

Le pré-vol (d) — `~/scripts/prevol-d-timeouts.sh`, livré par samidarko le 14/09 en lecture seule —
est un grep manuel qui a attrapé la « couche 4 » (client a2a 300 s < total 600 s) sur main
`891004c9`. La table ronde du 11/09 (Prime) a tranché le **changement de classe** : ce qu'un
opérateur attrape à la main une fois doit devenir une garde qui l'attrape toujours.

Ce plan porte cette logique en CI. Il ne la porte **pas à la lettre** : quatre mesures faites sur
le code en vigueur montrent que le prédicat tel qu'il est écrit dans le ticket rougirait le jour de
sa naissance sur du code sain, et resterait vert sur la régression qu'il existe pour attraper. La
section *Ce que la mesure déplace* établit ces mesures ; la section *Conception* en dérive la règle
qui tient.

---

## Ce que la mesure déplace

Toutes les mesures ci-dessous sont faites sur le worktree de ce ticket, à `e1342dfa`.

### M1 — Le prédicat du ticket rougit sur du code parfaitement sain

Le ticket demande : *« tout littéral de durée ≥60s vivant sur le chemin a2a doit être un DÉFAUT
dérivé d'une env (sinon échec) »*. La population réelle de ce prédicat, hors `#[cfg(test)]` et hors
`tests/`, contient des dizaines de durées qui **ne sont pas des budgets de timeout** :

| Site | Valeur | Nature |
|---|---|---|
| `server/check_suite_dedup.rs:57` `ENTRY_TTL` | 600 | TTL de cache |
| `server/check_suite_dedup.rs:50` `DEDUP_WINDOW` | 60 | fenêtre de déduplication |
| `common/github_app.rs:29` `JWT_LIFETIME` | 540 | durée de vie d'un JWT (contrainte GitHub) |
| `common/github_app.rs:22` `IAT_BACKDATE` | 60 | correction d'horloge |
| `auto_pull.rs:1316` `EXCLUSION_AUDIT_REFRESH` | 86 400 | horizon de déduplication d'audit |
| `tools/pr_merge_with_gate.rs:838` `UPDATE_ATTEMPT_TTL` | 21 600 | TTL |
| `milestone_manager/spawn.rs:803` `AUTH_ALARM_REEMIT_INTERVAL` | 3 600 | cadence de ré-émission |
| `gateway/orchestrator_inbox.rs:49` `RETENTION_TICK_INTERVAL` | 3 600 | cadence de tick |
| `server/ci_success_handler.rs:306,621,672` | 60 | plafond d'un appel `gh` |

« Durée ≥ 60 s » n'est pas « budget de la cascade client/total/http ». Exiger qu'un TTL de cache ou
la durée de vie d'un JWT soit single-sourcée sur une variable d'environnement n'a pas de sens ; une
garde qui le réclame se fait désarmer, ou voit son allowlist devenir un fourre-tout où la
régression qu'on cherche passe inaperçue. **C'est le mode de panne que le ticket existe pour
empêcher, atteint par le remède.**

### M2 — Le grep du pré-vol est contournable par réécriture arithmétique, et le contournement est déjà écrit

Un motif `from_secs\([0-9]+\)` rate toute valeur composée. Sept sites **vivants** l'écrivent déjà :

```
auto_pull.rs:1316                 Duration::from_secs(24 * 60 * 60)
milestone_manager/spawn.rs:798    Duration::from_secs(30 * 60)
milestone_manager/spawn.rs:803    Duration::from_secs(60 * 60)
common/github_app.rs:19           Duration::from_secs(5 * 60)
tools/pr_merge_with_gate.rs:838   Duration::from_secs(6 * 3600)
gateway/orchestrator_inbox.rs:49  Duration::from_secs(60 * 60)
```

Une garde qu'on contourne en écrivant `10 * 60` au lieu de `600` n'est pas une garde. Elle est
**pire que pas de garde** : elle donne une couverture qu'elle n'a pas, et c'est précisément sous ce
genre de couverture que la couche-6 réapparaîtrait. Le portage doit **évaluer** l'expression, pas
la matcher.

### M3 — Le périmètre annoncé ne retranche rien

Le pré-vol nomme sept crates : `mika-a2a`, `mika-common`, `mika-cli`, `mika-agent`,
`mika-gateway`, `mika-llm`, `mika-core`. **`mika-llm` et `mika-core` n'existent pas** (les membres
réels sont `mika-a2a`, `mika-agent`, `mika-cli`, `mika-common`, `mika-gateway`, `mika-os`). Les cinq
restants sont tout le workspace sauf `mika-os`. « Le chemin aller/retour a2a » n'est donc pas un
périmètre : c'est le code entier, moins un crate sans rapport. Un périmètre qui ne retranche rien
ne discrimine rien — ce qui est la cause directe de M1.

### M4 — L'ordre demandé est plus faible que l'invariant que le code tient déjà

Le ticket demande `client ≥ total ≥ http`. Le code refuse `total == http` :

- `common/llm/budget.rs:104` — *« per-call timeout must be **strictly less than** … »*
- `common/llm/budget.rs:229` — `if self.http_timeout_secs >= self.agent_total_timeout_secs { … }`
- `common/llm/budget.rs:456` — `LlmTimeoutBudget::new(300, 300)` **doit** être une erreur.

L'invariant réel est **`client ≥ total > http`**. Coder le `≥` du ticket ferait passer un couple
que `server::budget_guard::assert_llm_budgets_valid` refuse au démarrage : un test vert sur une
configuration qui empêche mika-spirit de démarrer. La garde doit porter l'invariant du code, pas sa
paraphrase.

### M5 — La moitié « ordre » est déjà tenue à l'exécution, deux fois — mais jamais transitivement

- `mika-a2a/src/client.rs:51` `resolve_timeout_secs` pose le plancher `client ≥ total` (`.max()`).
- `mika-agent/src/server/budget_guard.rs` (mika#2293) refuse le démarrage si `http ≥ total`.

Chacun ne connaît **que sa paire**. Personne n'affirme la chaîne complète, et personne ne l'affirme
sans démarrer un serveur. C'est là, et seulement là, que #2309 ajoute quelque chose à l'ordre : la
**transitivité**, affirmée en un lieu, vérifiable en CI.

### M6 — Un script d'opérateur et un test CI n'ont pas la même nature

Le pré-vol vérifie l'assertion sur « les budgets effectifs (`~/.mika/.env` + défauts du code) ». En
CI il n'y a pas de `~/.mika/.env`. Porter le script tel quel, c'est porter une lecture de la machine
de l'opérateur : en CI elle ne lit rien, et le script retombe sur les défauts — qu'il **recopierait**
en shell. Un test qui recopie les valeurs qu'il vérifie ne teste que sa propre copie. La moitié
« ordre » doit donc être un test **Rust** qui appelle les résolveurs réels, pas un shell qui les
réimplémente. (Précédent maison explicite : mika#2293 épingle sa reconstruction de cascade contre
`Settings::load_for_agent` pour exactement cette raison.)

---

## Requirements

**R1 — Empêcher structurellement une couche-6.** Un budget en dur, non single-sourcé, qui borne un
appel sur le chemin a2a, doit faire échouer la CI.

**R2 — Ne pas rougir sur du code sain.** La garde est verte sur `main` au moment du merge, sans
allowlist fourre-tout : toute entrée d'allowlist porte une justification nominale.

**R3 — Ne pas être contournable par réécriture.** `from_secs(10 * 60)` et `from_secs(600)` sont
traités identiquement.

**R4 — Porter l'invariant réel.** `client ≥ total > http`, la chaîne complète, affirmée en un lieu,
en lisant les résolveurs de production et non une copie.

**R5 — La garde doit avoir été vue rouge.** Contrainte maison (mika#2103, inscrite dans `ci.yml` :
*« A guard nobody has watched go red is a decoration »*) : chaque lint a un test négatif qui épingle
son refus.

---

## Conception

Deux gardes, de natures différentes parce que les deux moitiés du pré-vol sont de natures
différentes (M6).

### Garde A — `scripts/check-a2a-timeout-literals.sh` (source-scan, shell)

Le prédicat porte sur **le site qui borne**, pas sur la valeur. C'est la correction que M1 impose :
ce qui est suspect n'est pas « 600 », c'est « un appel du chemin a2a dont le budget vient d'un
littéral au lieu d'un résolveur ».

**Périmètre (explicite, et c'est ce qui manquait — M3).** Un fichier `PERIMETER` déclaré en tête du
script :

- `crates/mika-a2a/src/**` — le client et son transport ;
- `crates/mika-cli/src/remote_ask.rs` et `crates/mika-agent/src/tools/a2a_call.rs` — les deux
  constructeurs de `A2aClient` ;
- `crates/mika-common/src/llm/**` — le rail qui porte la cascade (`budget.rs`, `mod.rs`,
  `budget_provenance.rs`, les providers).

`mika-os` est hors périmètre ; `mika-llm` et `mika-core` sont retirés (ils n'existent pas). Le
périmètre est **fermé et nommé**, pas « les sept crates ».

**Règle A1 — aucun littéral au site de bornage.** Dans le périmètre, l'argument de
`.timeout(`, `.connect_timeout(`, `with_timeout(`, `tokio::time::timeout(` ne peut pas être une
**durée littérale**. Le prédicat porte sur ce que reçoit `from_secs`, pas sur la présence de
`from_secs` : `from_secs(<littéral>)` et `from_secs(<expression purement arithmétique>)` sont
refusés ; `from_secs(<identifiant>)`, `from_secs(<appel de fonction>)` et un argument qui n'est pas
un `Duration::from_*` du tout sont admis.

Cette précision n'est pas cosmétique : la formulation large (« pas de `Duration::from_*` inline »)
rougit à la naissance sur deux sites parfaitement single-sourcés —
`openai.rs:185` et `ollama.rs:233` écrivent tous deux
`.timeout(Duration::from_secs(budget.http_timeout_secs()))`, où le budget vient du résolveur. Elle
reproduirait donc, à l'intérieur de sa propre règle, le mode de panne que M1 démonte. AC1 portait
déjà sur le littéral ; c'est la prose qui était plus large que son critère. Voir § *Fire-Disposition*,
surface 1.

Mesuré au 18/09 (`e1342dfa`) sur le périmètre déclaré : zéro littéral au site de bornage —
`client.rs:97` reçoit la variable `timeout`, `openai.rs:185` et `ollama.rs:233` reçoivent
`budget.http_timeout_secs()`. **Vert à la naissance, rouge sur régression.**

Cette règle est immunisée contre M2 : le contournement arithmétique porte sur la valeur, la règle
porte sur la forme du site.

**Règle A2 — une const de durée ≥ 60 s doit être le défaut d'une env.** Dans le périmètre, toute
`const … : Duration = Duration::from_secs(<expr>)` dont l'expression **évaluée** vaut ≥ 60 doit :

- soit être nommée `DEFAULT_*` **et** cohabiter dans son module avec une déclaration d'env
  (`*_ENV: &str = "MIKA_…"` ou un `env::var("MIKA_…")`) — c'est exactement le cas de
  `client.rs:24` `DEFAULT_TIMEOUT = 600` avec `TIMEOUT_ENV = "MIKA_A2A_TIMEOUT_SECS"` ;
- soit figurer dans l'allowlist déclarée avec sa justification.

L'évaluation de `<expr>` se fait après validation que l'expression ne contient que des chiffres,
`_`, `*` et des espaces (sinon : échec explicite, jamais silencieux). Cela ferme M2.

**Allowlist, et elle est comparée dans les deux sens.** Sur le modèle de
`check-dispatch-seats-declared.sh` : une entrée allowlistée qui ne correspond plus à aucun site
**fait échouer** le script au même titre qu'un site non allowlisté. Une allowlist qu'on ne nettoie
pas devient le fourre-tout de M1 ; la comparer dans les deux sens est ce qui l'en empêche.
Population attendue à la livraison : **zéro** entrée. Comptée le 18/09 (`e1342dfa`), le périmètre
ne contient que deux `const … : Duration` : `client.rs:24` `DEFAULT_TIMEOUT` (600 s — conforme,
`DEFAULT_*` + `TIMEOUT_ENV`) et `client.rs:61` `RECOVERY_TIMEOUT` (30 s — sous le seuil, donc hors
population ; mentionné pour que le lecteur sache qu'il a été examiné). A2 naît donc sur une
population d'une seule entrée conforme : sa valeur est **prospective**, elle borne ce qu'on ajoutera,
et c'est A1 qui porte le travail sur le code en vigueur.

**Seuil.** 60 s, repris du ticket, porté par une constante nommée en tête de script. Coût nommé :
un littéral de 59 s posé au site de bornage passerait A2 — mais il ne passe pas A1, qui ne regarde
pas la valeur. Le seuil ne protège rien tout seul ; il borne le volume de A2.

### Garde B — test Rust de transitivité

**Lieu contraint par le graphe de dépendances, et ce n'est pas le lieu proposé d'abord.**
`mika-common` ne dépend pas de `mika-a2a` (vérifié : `mika-a2a/Cargo.toml` n'a aucune dépendance
maison, et c'est `mika-agent` qui tire les deux). Un test posé dans `crates/mika-common/src/llm/budget.rs`
ne peut donc pas voir `client`, et la chaîne s'y réduirait à `total > http` — la moitié que
`budget_guard` tient déjà (M5). Le test vit dans **`mika-agent`** (module de tests ou
`tests/`), seul crate qui voit les trois valeurs. La visibilité de `resolve_timeout_secs` reste à
trancher à l'implémentation — elle est privée aujourd'hui : soit le test passe par
`resolve_send_timeout()` avec env posée, soit le cœur pur est exposé — **en préférant la voie qui
n'élargit pas l'API publique**.

Le test affirme, en appelant les résolveurs de production :

```
client  = mika_a2a::client::resolve_send_timeout()        (ou son cœur pur)
total   = planning::policy::agent_total_timeout_secs(...)  / DEFAULT_AGENT_TOTAL_TIMEOUT_SECS
http    = mika_common::llm::http_timeout_secs()            / DEFAULT_HTTP_TIMEOUT_SECS

assert client >= total          // plancher de mika#2297
assert total  >  http           // strict — M4
```

Vérifié sur les défauts (600 / 300 / 120) **et** sur au moins un couple non-défaut posé par env,
pour que la chaîne soit affirmée sur la cascade et pas seulement sur les constantes. Les tests qui
manipulent l'env process-global doivent suivre la discipline maison (sérialisation ou cœur pur
paramétré — `resolve_timeout_secs` a justement été séparé pour ça, cf. son commentaire
`client.rs:48`).

**Ce que ce test n'est pas :** une réimplémentation. S'il recopie `600`, `300`, `120`, il teste sa
copie (M6). Il lit les constantes exportées et appelle les fonctions réelles.

**Ce que ce test ne peut pas attraper, et il faut le dire ici plutôt que le découvrir après.** Sur la
voie env, `client >= total` est vrai **par construction** : `resolve_timeout_secs` applique
`.max(total)` (`client.rs:53`). Le premier `assert` est donc un épinglage de ce plancher — il rougit
si quelqu'un retire le `.max()`, ce qui est une régression réelle — mais il ne peut pas découvrir une
divergence, parce que la seule cascade que le client sache lire est celle de l'env du process. La
cascade **per-agent** lui est invisible, et c'est là qu'une divergence existe aujourd'hui : voir
§ *Fire-Disposition*, surface 3.

### Garde C — le test négatif (R5)

`scripts/test-check-a2a-timeout-literals.sh`, sur le gabarit de `test-check-byte-slices.sh` :
fabrique dans un répertoire temporaire un cas violant A1 et un cas violant A2 (dont un écrit
`from_secs(10 * 60)`, pour épingler M2), et **exige que le script échoue** sur chacun ; puis un cas
conforme, et exige qu'il passe. Un quatrième cas épingle la comparaison bidirectionnelle de
l'allowlist.

### Job CI

Dans `.github/workflows/ci.yml`, sur le gabarit exact de `byte-slice-lint` /
`image-tag-immutability-lint` (deux étapes : le lint, puis l'épinglage du comportement négatif) :

```yaml
  a2a-timeout-literal-lint:
    name: A2A Timeout Literal Lint
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1  # v6
      - name: Reject hardcoded timeout budgets on the a2a path (mika#2309)
        run: bash scripts/check-a2a-timeout-literals.sh
      - name: Pin the guard's negative behaviour
        run: bash scripts/test-check-a2a-timeout-literals.sh
```

La garde B tourne dans `cargo test` (job `check`), sans job dédié.

---

## Ce qui est délibérément refusé

1. **Porter le prédicat du ticket à la lettre** (« tout littéral ≥ 60 s »). Refusé par M1 : il
   rougit sur des TTL, des lifetimes de JWT et des cadences de tick qui n'ont aucun rapport avec la
   cascade. Le prédicat retenu porte sur le site de bornage, pas sur la valeur.
2. **Porter les sept crates annoncés.** Refusé par M3 : deux n'existent pas et les cinq autres sont
   le workspace entier. Le périmètre est déclaré et fermé.
3. **Coder `total ≥ http`.** Refusé par M4 : l'invariant du code est strict, et le `≥` laisserait
   passer un couple qui empêche le démarrage.
4. **Un script shell qui lit `~/.mika/.env` en CI.** Refusé par M6 : il ne lirait rien et
   recopierait les défauts.
5. **Ajouter un `MIKA_*` pour désarmer la garde.** Une garde CI se désarme en retirant son job,
   visiblement, dans un diff relu — pas par une variable d'environnement qu'un runner peut porter
   en silence.
6. **Allowlister `openai.rs:185` et `ollama.rs:233`.** Refusé : ces deux sites sont conformes, c'est
   le prédicat qui était trop large. On répare la règle, on ne gèle pas deux sites corrects dans le
   fichier qui sert à prouver que la garde discrimine (§ Fire-Disposition, surface 1).
7. **Corriger ici la divergence client 600 / enveloppe arch 900.** Refusé : c'est un changement de
   comportement runtime, que ce ticket s'interdit. Il est mesuré, épinglé par un contrôle positif
   auto-nettoyant et remonté dans son propre ticket (§ Fire-Disposition, surface 3).
8. **Étendre la garde à `from_millis` / `from_secs_f64` / `from_mins`.** Mesuré : zéro occurrence
   dans le workspace. Les ajouter serait de la couverture spéculative ; le motif du script les
   nommera en commentaire pour que l'extension soit une ligne le jour où l'une apparaît.

---

## Fire-Disposition

*(Exigée par la Fire-Disposition Gate — mika#1574, `docs/solutions/best-practices/fire-disposition-doctrine.md` ;
première passe mika-arch, F1 bloquant. Les gardes A, B et C sont toutes de classe détecteur : leur
chemin de succès est « aucune violation ».)*

La question de la porte est : **que fait l'implémentation quand un détecteur tire sur des données
préexistantes ?** Il y a quatre surfaces de tir et elles n'appellent pas la même disposition — dont
deux qui tirent réellement, mesurées le 18/09 à `e1342dfa`.

### Surface 1 — Règle A1 sur le code en vigueur : **tir certain** sur la formulation large

Mesure, pas hypothèse. Sur le périmètre déclaré, deux sites écrivent un `Duration::from_*` inline en
argument de `.timeout(` :

| Site | Écriture | Budget |
|---|---|---|
| `crates/mika-common/src/llm/openai.rs:185` | `.timeout(Duration::from_secs(budget.http_timeout_secs()))` | résolveur |
| `crates/mika-common/src/llm/ollama.rs:233` | `.timeout(Duration::from_secs(budget.http_timeout_secs()))` | résolveur |

Les deux sont **exemplaires** : le budget vient de `LlmTimeoutBudget`, c'est-à-dire du single-sourcing
que ce ticket existe pour protéger. Le prédicat « pas de `Duration::from_*` inline au site de
bornage » les refuserait tous les deux le jour de sa naissance.

**Disposition : réparation du prédicat dans le périmètre — ni allowlist, ni `#[ignore]`.**

Le choix mérite sa justification, la disposition par défaut de la doctrine étant (a) l'exception
nommée. Elle ne convient pas : une allowlist existe pour **isoler une violation réelle** que le
correctif ne traite pas. Ici il n'y a aucune violation — le prédicat est simplement plus large que
ce qu'il veut dire, et l'allowlist gèlerait deux sites corrects dans le fichier même qui sert à
prouver que la garde discrimine. Ce serait le fourre-tout de M1, atteint par le remède, une seconde
fois. Et c'est **une réparation, pas un arbitrage** : AC1 écrivait déjà « `Duration::from_secs(<littéral>)` »,
donc la prose de la Conception contredisait son propre critère. La règle porte sur l'argument de
`from_secs` (§ Conception, Règle A1). Après réparation : population de violations = **0**.

### Surface 2 — Règle A2 sur le code en vigueur : **population vide, comptée**

Deux `const … : Duration` dans le périmètre entier : `DEFAULT_TIMEOUT` (600 s, conforme) et
`RECOVERY_TIMEOUT` (30 s, sous le seuil). Zéro violation, donc **aucune disposition n'est due** ; ce
qui est dû, c'est de dire que la population a été comptée et à quelle date — pour que le prochain
lecteur sache que le zéro est mesuré et non supposé. Allowlist livrée **vide** (AC6), et c'est la
comparaison bidirectionnelle (AC4) qui la maintient vide.

### Surface 3 — Garde B sur la cascade per-agent : **divergence vivante, hors périmètre de ce ticket**

Mesurée : `crates/mika-agent/src/well_known_agents.rs:1478` pose `agent_total_timeout_secs = 900`
pour mika-arch (mika#2189), tandis que `resolve_timeout_secs` (`client.rs:51`) ne lit que
`MIKA_AGENT_TOTAL_TIMEOUT_SECS` dans l'env du process — jamais le `config.toml` per-agent. Sur un
appel vers mika-arch, le client résout donc **600 s** face à une enveloppe de **900 s** : le plancher
`client ≥ total` de mika#2297 **n'est pas tenu**, et le client abandonne une génération que le moteur
a encore le droit de finir — exactement le sinistre du 11/09 qui a motivé le passage de 300 à 600.

C'est une violation préexistante de l'invariant que la Garde B affirme, et il faut être précis sur ce
qu'elle est : elle n'est **pas** dans le code que ce plan touche, elle est dans la portée de lecture
du résolveur. La corriger, c'est apprendre au client a2a à lire la cascade per-agent — un changement
de **comportement runtime**, que ce ticket exclut en tête (« substrat borné, pas de changement de
comportement runtime »), et dont la forme même est la question : le client ne connaît pas l'agent
visé au moment où il résout son budget.

**Disposition : (c) halte-et-remontée, bornée — avec un contrôle positif auto-nettoyant.**

L'option (c) est ici la bonne au sens strict de la doctrine (« la résolution de la violation
préexistante *est* la question de périmètre »), mais elle n'autorise pas à surfacer et passer à
autre chose. Concrètement :

- La Garde B affirme la chaîne sur la cascade que le client sait réellement lire (défauts + env),
  comme le dit déjà AC5. Elle ne prétend pas couvrir le per-agent.
- Un **contrôle positif** épingle la divergence au lieu de la taire : un test lit les deux valeurs
  réelles — `mika_a2a::client::DEFAULT_TIMEOUT` et le `agent_total_timeout_secs` de `MIKA_ARCH_CONFIG` —
  et asserte que le défaut client est **strictement inférieur** à l'enveloppe d'arch. Il documente
  l'écart comme connu et mesuré ; **il rougit le jour où l'écart disparaît** (alignement des valeurs,
  ou client apprenant à lire le per-agent), ce qui est l'assertion auto-nettoyante de la doctrine :
  elle force à retirer l'exception au lieu de la laisser survivre à sa cause. Il vit dans `mika-agent`,
  seul crate voyant les deux constantes.
- **Suivi :** l'implémenteur ouvre un ticket nommant la divergence (portée de lecture du plancher
  mika#2297 face aux cascades per-agent de mika#2189) et le cite dans le commentaire du contrôle
  positif. Le ticket est la remontée ; le test est ce qui l'empêche de se périmer en silence.
- **Ce qui est interdit à l'implémenteur :** aligner `DEFAULT_TIMEOUT` sur 900 de sa propre autorité,
  ou faire lire le `config.toml` per-agent au client. Les deux changent un budget de production dans
  un ticket qui s'interdit d'en bouger aucun.

### Surface 4 — Garde C : hors doctrine, et il vaut mieux le dire que l'omettre

Le test négatif ne tire pas sur des données préexistantes : sa population est **fabriquée** dans un
répertoire temporaire, close et connue à l'écriture. Une disposition de tir n'a pas d'objet pour lui.
Il est listé ici pour que son absence des trois surfaces précédentes se lise comme une décision et
non comme un oubli.

### Déclencheur de halte-et-remontée (transverse, borné)

Les surfaces 1 et 2 sont mesurées au 18/09 à `e1342dfa` ; l'implémentation arrive après, et le
périmètre couvre `crates/mika-common/src/llm/**`, qui bouge.

> Si, au moment d'implémenter, la re-mesure fait apparaître **une seule** violation de A1 ou de A2 qui
> ne figure pas dans les surfaces 1 et 2 ci-dessus, l'implémenteur **s'arrête et remonte** avec le
> chemin, la ligne et l'écriture fautive. Il n'ajoute pas d'entrée d'allowlist de sa propre autorité
> et n'assouplit pas la règle pour faire passer le cas.

La raison est celle de la doctrine : une allowlist posée sans que personne ait regardé la violation
est le fourre-tout de M1, et c'est sous ce genre de couverture que la couche-6 réapparaîtrait.

---

## Contrat de vérification

| # | Vérification | Commande | Attendu |
|---|---|---|---|
| V1 | La garde A est verte sur `main` | `bash scripts/check-a2a-timeout-literals.sh` | exit 0, zéro violation |
| V2 | La garde A refuse un littéral au site de bornage | `bash scripts/test-check-a2a-timeout-literals.sh` | exit 0 (le test négatif passe) |
| V3 | Le contournement arithmétique est attrapé | cas `from_secs(10 * 60)` dans V2 | le script échoue sur ce cas |
| V4 | L'allowlist est comparée dans les deux sens | cas « entrée orpheline » dans V2 | le script échoue |
| V5 | La transitivité tient sur les défauts | `cargo test -p mika-common budget` | vert, `client ≥ total > http` |
| V6 | La transitivité tient sur une cascade non-défaut | idem, cas env-posé | vert |
| V7 | Pas de régression | `make lint && make test` | vert |
| V8 | Le job CI tourne sur PR | inspection du run de la PR | `A2A Timeout Literal Lint` présent et vert |
| V9 | A1 n'accuse pas les deux sites conformes | `bash scripts/check-a2a-timeout-literals.sh` | `openai.rs:185` et `ollama.rs:233` ne sont pas signalés (§ Fire-Disposition, surface 1) |
| V10 | La divergence per-agent est épinglée | `cargo test -p mika-agent` (contrôle positif) | vert, et son commentaire cite le ticket de suivi |

**Sonde de réalité (obligatoire avant merge).** Introduire localement, sans commit, un
`.timeout(Duration::from_secs(900))` dans `crates/mika-a2a/src/client.rs`, vérifier que V1 devient
rouge, puis le retirer. Une garde dont on n'a pas vu le rouge sur le **vrai** fichier — pas
seulement sur une fixture — n'a pas démontré qu'elle couvre son périmètre.

---

## Definition of Done

- `scripts/check-a2a-timeout-literals.sh` livré, exécutable, avec un en-tête qui énonce le
  périmètre, les deux règles, le seuil, et pourquoi le prédicat porte sur le site et non sur la
  valeur (M1/M2 en deux phrases, pour que le prochain lecteur ne « resserre » pas la garde vers le
  prédicat naïf).
- `scripts/test-check-a2a-timeout-literals.sh` livré, couvrant A1, A2, le contournement
  arithmétique et l'allowlist bidirectionnelle.
- Garde B livrée avec les deux cas (défauts + cascade), dans `mika-agent` — seul crate voyant les
  trois valeurs.
- Contrôle positif auto-nettoyant de la surface 3 livré, citant le ticket de suivi ouvert pour la
  divergence client / enveloppe per-agent.
- Job `a2a-timeout-literal-lint` ajouté à `ci.yml` sur le gabarit à deux étapes.
- V1–V8 verts, sonde de réalité effectuée et mentionnée dans le corps de PR.
- `crates/mika-a2a/CLAUDE.md` : une ligne renvoyant à la garde depuis la section
  *Client transport policy*.
- `CLAUDE.md` racine : `a2a-timeout-literal-lint` ajouté à l'énumération des jobs CI de la section
  *CI/CD*, aux côtés de `byte-slice-lint` et `loop-select-lint`.

---

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les critères ci-dessous sont dérivés
des *Requirements* et du *Contrat de vérification*.

- **AC1** — Un `Duration::from_secs(<littéral>)` passé en argument à `.timeout(`,
  `.connect_timeout(`, `with_timeout(` ou `tokio::time::timeout(` dans le périmètre déclaré fait
  échouer la CI, quelle que soit la valeur.
- **AC2** — Une `const … : Duration` ≥ 60 s dans le périmètre qui n'est ni le défaut d'une env
  déclarée ni allowlistée avec justification fait échouer la CI.
- **AC3** — `from_secs(10 * 60)` et `from_secs(600)` sont traités identiquement ; une expression
  non purement arithmétique fait échouer le script explicitement plutôt que d'être ignorée.
- **AC4** — Une entrée d'allowlist qui ne correspond plus à aucun site fait échouer la CI.
- **AC5** — Un test Rust affirme `client ≥ total > http` en appelant les résolveurs de production,
  sur les défauts et sur au moins une cascade posée par env ; il ne recopie aucune des trois
  valeurs.
- **AC6** — La garde est verte sur `main` à la livraison, avec **zéro** entrée d'allowlist.
- **AC7** — Le job CI comporte les deux étapes (lint + test négatif), conformément à mika#2103.
- **AC8** — Le périmètre est déclaré explicitement dans le script et ne nomme aucun crate
  inexistant.
- **AC9** — La garde A ne signale ni `openai.rs:185` ni `ollama.rs:233` : le prédicat porte sur
  l'argument de `from_secs`, pas sur sa présence (§ Fire-Disposition, surface 1).
- **AC10** — Un contrôle positif épingle la divergence mesurée entre le défaut client (600 s) et
  l'enveloppe per-agent de mika-arch (900 s) en lisant les deux valeurs réelles ; il rougit le jour
  où l'écart disparaît, et son commentaire cite le ticket de suivi
  (§ Fire-Disposition, surface 3).

---

## Risques et hors-périmètre

**Risque 1 — la garde se fait resserrer vers le prédicat naïf.** Un futur lecteur qui relit le
ticket sans relire *Ce que la mesure déplace* peut « corriger » la garde vers « tout littéral
≥ 60 s », et la faire rougir
sur les TTL. Atténuation : l'en-tête du script porte le raisonnement, et la table M1 vit dans ce
plan, référencé depuis l'en-tête.

**Risque 2 — faux négatif sur un site de bornage non énuméré.** Un futur helper qui borne un appel
sans passer par les quatre motifs reconnus échappe à A1. C'est le trou structurel d'un source-scan
et il est assumé : A2 le rattrape dès que le budget est une const ≥ 60 s. Le jour où un cinquième
motif apparaît, c'est une ligne dans le script.

**Risque 3 — coût de maintenance du périmètre.** Un nouveau call site `A2aClient` hors des deux
connus n'est pas couvert tant qu'il n'est pas ajouté au périmètre. Atténuation possible et
**délibérément non faite ici** : une règle qui exigerait que tout fichier construisant un
`A2aClient` soit dans le périmètre. Elle mérite son propre ticket ; l'ajouter ici doublerait la
surface d'un ticket substrat borné.

**Hors périmètre, délibérément :**

- La *cause* des budgets mal ordonnés côté configuration — `budget_guard` (mika#2293) la tient
  déjà au démarrage, et l'observabilité de provenance (`llm_budget_resolved`) la rend lisible.
- Le littéral `120s` en dur du rail Anthropic (`common/llm/anthropic.rs` / `claude.rs:382`), connu
  et déjà nommé hors périmètre par mika#2189 puis mika#2342. Il est **hors du chemin a2a** et son
  correctif change une classe d'erreur ; il garde son ticket.
- `pool_idle_timeout(90)` du gateway (`gateway/src/main.rs:106`) : un idle de pool n'est pas un
  budget de requête, et le gateway n'est pas dans le périmètre déclaré.
- La *correction* de la divergence entre le plancher client (mika#2297, lu sur l'env du process) et
  les cascades per-agent (mika#2189, lues dans le `config.toml` de l'agent). Mesurée, épinglée,
  remontée dans son propre ticket — § Fire-Disposition, surface 3. Elle change un comportement
  runtime ; ce ticket n'en change aucun.
- Toute modification de valeur. Ce travail ne bouge aucun budget : 600 / 300 / 120 — ni le 240 / 900
  de mika-arch — restent ce qu'ils sont. Il rend leur ordre non-régressable.

---

## Revision history

- **rev 2 (2026-09-18)** — première passe architecte, `Disposition: ITERATE`, F1 bloquant.
  - **F1 (§ Fire-Disposition manquante, mika#1574) : adressée.** Section ajoutée, couvrant les trois
    gardes sur quatre surfaces de tir. Le finding suggérait l'option (a) avec allowlist vide comme
    disposition naturelle ; la mesure faite pour écrire la section montre que ce n'est le bon choix
    sur **aucune** des surfaces, et la section dit pourquoi à chaque fois. Surface 1 (Règle A1) :
    tir certain mesuré sur `openai.rs:185` et `ollama.rs:233`, deux sites conformes — disposition
    *réparation du prédicat*, parce qu'une allowlist y gèlerait deux sites corrects et reproduirait
    le mode de panne M1 à l'intérieur du remède. Surface 2 (Règle A2) : population comptée, vide,
    allowlist livrée vide comme prévu. Surface 3 (Garde B) : divergence vivante mesurée entre le
    défaut client (600 s) et l'enveloppe per-agent de mika-arch (900 s, `well_known_agents.rs:1478`) —
    disposition *(c) halte-et-remontée* bornée par un contrôle positif auto-nettoyant, parce que la
    corriger serait un changement de comportement runtime que ce ticket s'interdit. Surface 4
    (Garde C) : hors doctrine, population fabriquée — dit plutôt qu'omis. Plus un déclencheur de
    halte-et-remontée transverse, les mesures ayant une date et l'implémentation arrivant après.
  - **Trois corrections dérivées de la mesure faite pour F1**, sans lesquelles la disposition
    n'aurait pas de sens : (i) la prose de la Règle A1 était plus large que son propre AC1 et
    rougissait sur deux sites sains — la prose est alignée sur l'AC, aucun critère affaibli ;
    (ii) la Garde B était proposée dans `mika-common`, qui ne dépend pas de `mika-a2a` et ne peut donc
    pas compiler la chaîne complète — elle est déplacée dans `mika-agent` ; (iii) la Garde B est verte
    par construction sur la voie env (le `.max()` de `client.rs:53`), ce qui est maintenant écrit au
    lieu d'être découvert à l'implémentation.
  - **AC9 et AC10 ajoutés**, V9 et V10 au contrat de vérification, deux entrées aux refus délibérés
    et une au hors-périmètre. Aucun AC existant n'a été affaibli ni retiré.
