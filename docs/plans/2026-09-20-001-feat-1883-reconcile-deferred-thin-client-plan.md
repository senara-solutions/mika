# mika#1883 — Réconcilier les reports du thin-client : la mesure devient juste, le no-op devient dit

> **Plan:** docs/plans/2026-09-20-001-feat-1883-reconcile-deferred-thin-client-plan.md
> **Issue:** senara-solutions/mika#1883
> **Type:** feat
> **Parent:** mika#1727 (refactor thin-client) — origine : section « Deferred » de mika#1881

---

## 1. Ce que la lecture du code déplace dans le ticket

Le ticket liste trois reports. **Deux sont clos, un troisième l'est aux trois
quarts**, et ce qui reste n'est pas ce que le ticket annonce. La rectification est
le premier livrable, parce qu'un plan qui implémenterait la lettre du ticket
ré-implémenterait du code déjà en place.

### 1.1 Point 3 — bookkeeping de session : clos (mika#2070)

Déjà acté par le commentaire opérateur du 30/08. `CALLER_SESSION_ID_KEY`
(`mika.caller_session_id`) traverse `message/send`, et le commentaire de report
dans `ask.rs` a été retiré dans la même PR. **Rien à faire.**

### 1.2 Point 1 — canal de config : trois quarts clos, un quart tranché

Le ticket demande « un canal de config à travers `message/send` ». **Ce canal
existe.** `crates/mika-a2a/src/params.rs` porte aujourd'hui six clés `mika.*`,
dont quatre sont exactement ce canal :

| clé | sens | ticket |
|---|---|---|
| `mika.caller_session_id` | session de l'appelant | mika#2070 |
| `mika.only_skills` | restriction de skills, **subtractive** | mika#2363 |
| `mika.model_override` → `mika.effective_model` | modèle + attestation | mika#2304 |
| `mika.session_isolated` → `mika.session_isolated_applied` | fenêtre + attestation | mika#1951 |

Par flag du ticket :

- **`--model` : livré** (mika#2304). Et son statut dans le ticket était faux
  d'une manière qui compte : ce n'était pas une capacité *manquante* mais un
  **faux vert**. Le flag atteignait le `Settings` local, que l'enveloppe
  `--verbose` relisait ensuite — donc le CLI affichait le modèle demandé avec
  autorité pendant que le tour tournait chez spirit sous celui du `config.toml`.
  C'est la classe de défaut à laquelle appartient aussi le point 2 ci-dessous,
  et c'est pour ça que ce plan la nomme plutôt que de la contourner.
- **`--enable-skill` (moitié additive) : refusé, avec raison écrite** (mika#2363,
  reportée dans `CLAUDE.md`). Elle laisserait tout appelant authentifié de
  `/a2a/{agent}` forcer une skill de l'agent en `always_on` — élargir une surface
  depuis le réseau. Ce refus n'est pas à relitiger ici.
- **`--disable-skill` : toujours inerte, et non tranché.** L'argument de refus de
  mika#2363 ne s'y applique pas : retirer une skill est strictement subtractif.
  C'est le seul morceau du point 1 encore ouvert. Le § 3 l'arbitre.

### 1.3 Point 2 — `tokens.*` : ouvert, et le branchement évident est un piège

`crates/mika-cli/src/commands/ask.rs` pose `tokens: None` en dur, avec son
commentaire de report. Le réflexe est de brancher `AgentOutput.usage`, qui existe
déjà et que le serveur tient en main à l'endroit exact où il écrit les deux
attestations existantes.

**Ce branchement produirait un chiffre faux.** Dans
`agent_loop/mod.rs::run_loop`, `last_usage` est **écrasé à chaque itération** :

```rust
let mut last_usage = None;                       // ~1081
// ... dans la boucle, à chaque tour :
if mode.is_conversation() {
    last_usage = Some(response.usage.clone());   // ~1468  ← écrase
}
```

et les trois sites de retour rendent ce `last_usage`. Le pont max-steps confirme
la sémantique par sa propre forme : `usage: cont.usage.or(usage)` — **un seul**
appel, jamais une somme.

Donc `AgentOutput.usage` est l'usage du **dernier appel LLM**, jamais du tour. Un
tour qui consomme ses 20 pas d'outil fait 21 appels ; le brancher tel quel
afficherait le vingt-et-unième sous l'étiquette « usage par run » — un nombre
plausible, présenté avec autorité, qui sous-compte massivement. **C'est
exactement le défaut de mika#2304, transposé d'un champ.** Le ticket demande
« l'usage **par run** » : livrer `last_usage` serait répondre à côté tout en ayant
l'air d'avoir répondu.

Deuxième piège, sur le même champ : `input_tokens` n'a pas la même définition
selon le rail. `CLAUDE.md` § Signal O le pose déjà — Anthropic rapporte l'entrée
fraîche, les rails OpenAI-compatibles rapportent `prompt_tokens` qui **inclut**
`cache_read_tokens`. Sommer est correct ; *normaliser* côté serveur créerait une
seconde vérité divergente de `turn_usage`.

---

## 2. Périmètre

Trois gestes, dans cet ordre de valeur :

- **A.** Rendre l'usage par tour **mesurable et juste** : agrégat dans la boucle,
  attesté sur le `Task`, lu par les deux surfaces clientes.
- **B.** Rendre **dit** le no-op de `--enable-skill` / `--disable-skill`, et
  corriger la documentation qui affirme aujourd'hui qu'ils fonctionnent.
- **C.** Écrire la fermeture : `CLAUDE.md`, `docs/skills.md`, et le commentaire de
  report de `ask.rs` — qui ne doit pas survivre à sa propre résolution (la règle
  que mika#2070 a déjà appliquée sur le point 3).

**Hors périmètre, délibérément :**

- La moitié **additive** du canal (`--enable-skill` → `apply_transient_always_on`
  serveur). Refusée par mika#2363 avec sa raison ; la rouvrir demande son propre
  ticket et sa propre mesure.
- L'usage des **délégations** et des **runs d'équipe**. Un tour qui appelle
  `delegate_task` dépense sous des sessions `delegate-*` / `team-*` qui lui sont
  propres. `ask.rs` documente déjà cette limite pour `mika.caller_session_id` ;
  ce plan hérite du même périmètre et l'écrit sur le nouveau champ plutôt que de
  laisser croire à un total de campagne.
- `message/stream` et `returnImmediately`. Ils ne produisent pas
  d'`AgentOutput` synchrone à lire ; ils tombent dans la population « non
  attesté », exactement comme pour `mika.effective_model`.
- La normalisation cache/prompt par famille de provider. RAW, comme
  `turn_usage`, et dit.

---

## 3. Arbitrage `--disable-skill` : dire, plutôt que livrer

Trois mesures décident, et elles pointent toutes dans le même sens.

1. **Usage mesuré : zéro.** Recherche sur `skills/bundled/`, `scripts/`,
   `.claude/`, `crates/` : aucun appelant n'utilise `--disable-skill`. La seule
   occurrence de `--enable-skill` dans le substrat est **une assertion qu'il a
   disparu** (`test-dispatch-lib.sh`, mika#2363 l'a retiré d'`_arch_ask`).
2. **Le fil dirait trois choses au lieu de deux.** `cli.rs` pose déjà par écrit,
   sur `--only-skill`, que « three selection semantics on one turn is a
   composition nobody wants to debug ». Ajouter `mika.disabled_skills` à côté de
   `mika.only_skills` livrerait cette troisième sémantique pour un besoin dont
   l'usage mesuré est nul.
3. **Le besoin subtractif est déjà servi.** `--only-skill` atteint spirit, est
   strictement subtractif, et couvre le cas qui a motivé le canal.

**Décision : ne pas livrer le canal ; rendre l'inertie visible et corriger la
doc.** Ce n'est pas un renoncement — c'est le remède que le ticket lui-même
autorise pour son point 2 (« rebrancher ou documenter comme intentionnel »),
appliqué au quart de point 1 que personne n'utilise.

**Avertir, ne pas refuser.** L'asymétrie est celle que `ONLY_SKILLS_KEY` porte
déjà dans son doc-comment, et elle est mesurée : une restriction de skill
silencieusement perdue rend le tour **plus large**, ce qui est visible et ne
falsifie aucune mesure ; c'est un modèle ou une isolation perdus qui rendent la
mesure fausse tout en produisant une réponse plausible. `--disable-skill` inerte
laisse une skill active — direction inoffensive. `--model` est fail-closed pour
la raison inverse, et cette différence est le cœur du design de mika#2304.

L'avertissement va sur **stderr**, jamais stdout : `_arch_ask` et tout appelant
`--format json` parsent stdout, et une ligne de plus y casserait un contrat de
fil pour un message de confort.

---

## 4. Conception — A : l'usage par tour

### 4.1 Agrégat dans la boucle

Ajouter, à côté de `last_usage` et sans le remplacer, un accumulateur sommant
**chaque** `response.usage` du tour.

```rust
let mut last_usage = None;
let mut run_usage: Option<LlmUsage> = None;   // somme, mika#1883
```

Sommé au même site que `last_usage` (même garde `mode.is_conversation()`, même
position après `let response = llm_result?;` — un appel en erreur ne rend pas
d'usage et contribue donc zéro, ce qui est correct et n'est pas un cas spécial).

`last_usage` **reste** : `AgentOutput.usage` est lu ailleurs et ce plan ne touche
pas à sa sémantique. Le nouveau champ vit à côté ; le renommer ou le réutiliser
mélangerait deux mesures sous un nom.

L'addition est portée par un helper unique — `LlmUsage::accumulate` (ou
`add_assign`) — plutôt qu'écrite deux fois en ligne. Il y a **deux** sites
d'addition (la boucle et le pont max-steps) et ils doivent traiter les `Option`
de cache de la même façon : `None + Some(n) = Some(n)`, et non `None`. Deux
écritures manuelles est précisément la forme dont ce dépôt a déjà dû extraire un
lecteur unique (mika#2158, `grooming_marker`).

### 4.2 La continuation est comptée

`attempt_continuation_turn` fait un appel LLM de plus, qui émet son propre
`turn_usage` via `save_continuation_llm_call`. Le pont max-steps doit donc
**ajouter** `cont.usage` à l'agrégat, là où il fait aujourd'hui
`cont.usage.or(usage)` pour `last_usage`. Les deux expressions coexistent et
disent deux choses différentes — c'est voulu, et le commentaire doit le dire,
sinon un futur relecteur « corrigera » l'une vers l'autre.

### 4.3 `AgentOutput.run_usage` et l'attestation

- `AgentOutput` gagne `pub run_usage: Option<LlmUsage>`.
- `A2aTurn` gagne le champ, forwarded depuis `output.run_usage` — même
  trajectoire et même raison que `effective_model` : la valeur n'existe que dans
  la main de ce processus, le `Task` reconstruit depuis la base ne la porte pas.
  Le doc-comment d'`A2aTurn` dit déjà pourquoi c'est un `struct` et non un
  tuple (mika#2270 : le second champ est facile à perdre en silence) — le
  troisième hérite de la protection.
- `params.rs` gagne **une** clé de réponse, `RUN_USAGE_KEY = "mika.run_usage"`,
  et **un** lecteur, `attested_run_usage(task) -> Option<RunUsage>`, posé à côté
  d'`attested_model` et d'`attested_session_isolation`, pour la raison que leur
  doc-comment énonce : les deux surfaces clientes doivent lire le même champ de
  la même façon.
- `server/a2a.rs` gagne `stamp_run_usage`, appelé au même point d'intervention
  que ses deux sœurs — `task` y est déjà `mut`, `a2a_build_task` est en amont,
  rien en aval ne peut écraser.

**Forme sur le fil** : un objet `{input, output, cache_read, cache_write}`, les
deux derniers absents quand le provider n'en rapporte pas. Un objet plutôt que
quatre clés plates parce que le client les rend groupés (`tokens.*`) et qu'une
famille de quatre clés `mika.*` pour une seule mesure encombrerait un espace de
noms partagé par cinq fonctions sans rapport.

### 4.4 Absence, jamais zéro

`stamp_run_usage` n'écrit **rien** quand l'agrégat est `None`. Le client rend
alors une absence, jamais un zéro.

C'est le précédent `request_bytes` de mika#2331, mot pour mot : *« `null` n'est
jamais `0` (aucune requête n'est vide, donc un zéro serait un mensonge
lisible) »*. Aucun tour ne consomme zéro token d'entrée. Un `0` affiché serait
indistinguable d'un tour réel et ferait croire à une mesure là où il n'y en a
pas.

Contrairement à `mika.effective_model` et `mika.session_isolated_applied`, ce
champ n'est **pas** écrit inconditionnellement. La raison de leur
inconditionnalité est qu'une absence y serait ambiguë entre « serveur antérieur »
et « rien n'a été demandé » ; ici il n'y a rien à demander — le champ est une
mesure, pas la réponse à un drapeau — et sa seule absence légitime est « le tour
n'a produit aucun appel dont lire l'usage ». Ambiguïté résiduelle assumée et
nommée : un spirit antérieur au champ et un tour sans usage se lisent pareil.
Les deux populations appellent la même conduite côté client — ne rien afficher —
donc les séparer n'achèterait rien.

### 4.5 Les deux surfaces clientes

- `commands/ask.rs` : `tokens: None` devient la lecture de l'attestation, sous la
  même garde `verbose` que ses voisins. Le rendu texte (`tokens.input:` …) et le
  rendu JSON (`metadata.tokens.*`) existent déjà et sont testés ; seule la source
  change.
- `remote_ask.rs::render` : même lecteur, mêmes lignes. **Ne pas l'ajouter ici
  serait recréer la divergence** que le doc-comment d'`attested_model` a été
  écrit pour prévenir.

Le non-verbose reste **byte-identique** — la contrainte que chaque ticket de
cette famille a tenue et que ses tests pinnent déjà.

---

## 5. Conception — B : le no-op rendu dit

Dans `commands/ask.rs`, après la validation de conflit existante (qui reste : un
`--enable-skill X --disable-skill X` est contradictoire quel que soit l'effet), un
`tracing::warn!` sur stderr quand l'un des deux vecteurs est non vide :

- nomme les skills concernées ;
- dit que le tour tourne chez mika-spirit et que ces drapeaux n'y arrivent pas ;
- nomme **le geste qui marche** : `--only-skill` pour restreindre.

Un refus qui ne nomme pas sa levée est un refus qu'on contourne au jugé — la
règle que la porte 2c de mika#2279 a déjà dû poser.

Événement `cli_skill_flag_inert`, pour qu'il soit comptable et non seulement
lisible.

**Régime attendu : zéro ligne** (usage mesuré nul). Une ligne est un appelant
qu'il faut migrer vers `--only-skill` — et c'est la mesure qui déciderait un jour
de livrer le canal, si elle cessait d'être vide.

---

## 6. Conception — C : la fermeture écrite

1. **`ask.rs`** — le bloc de commentaire « Deferred follow-ups » perd ses deux
   derniers reports et devient un constat daté. Un commentaire de report ne
   survit pas à sa résolution (mika#2070 AC4).
2. **`docs/skills.md:1099-1107`** — décrit `--enable-skill` / `--disable-skill`
   comme fonctionnels. **C'est faux depuis mika#1727.** Une documentation qui
   affirme un comportement disparu est plus coûteuse que pas de documentation :
   elle envoie l'opérateur vérifier autre chose. Corriger, nommer `--only-skill`,
   garder la description de la mécanique serveur (`apply_transient_disable`)
   comme ce qu'elle est — un chemin interne, plus une surface CLI.
3. **`CLAUDE.md`** — une entrée dans le voisinage de mika#2304, qui pose : les
   six clés du canal et leur statut ; la sémantique exacte de `mika.run_usage`
   (somme des appels **de ce tour**, hors délégations, RAW) ; les deux surfaces
   de lecture ; et le grep opérateur.
4. **`docs/solutions/architecture-patterns/cli-skill-always-on-transient-override.md`**
   — même mensonge, même correction.

---

## 7. Contrat de vérification

### 7.1 L'invariant central, et il est testable

> **La somme des `turn_usage` d'un tour égale son `mika.run_usage`.**

C'est la seule assertion qui distingue « l'agrégat est juste » de « l'agrégat
compile ». `turn_usage` est émis à chaque appel, continuation comprise ; le test
de bout en bout d'un tour à N appels vérifie l'égalité des quatre champs.

### 7.2 Tests

- `mika1883_run_usage_sums_every_call_of_the_turn` — **contrôle négatif porteur** :
  un tour à ≥ 3 appels via `EvalHarness` + `MockLlmProvider`, avec des usages
  **distincts** par appel, doit rendre la somme ; l'assertion doit **échouer** si
  on rend `last_usage`. Un test dont les appels auraient le même usage passerait
  sur les deux implémentations et n'attesterait rien.
- `mika1883_the_continuation_call_is_counted` — le pont max-steps ajoute
  `cont.usage` au lieu de le substituer.
- `mika1883_an_unmeasured_turn_attests_nothing` — agrégat `None` ⇒ clé absente ⇒
  client silencieux. Jamais `0`, jamais `null`.
- `mika1883_both_client_surfaces_read_the_one_reader` — scan de source refusant un
  second décodage de `RUN_USAGE_KEY` hors `params.rs` (modèle
  `mika2220_no_local_reparse_of_the_llm_bodies_env_var`).
- `mika1883_a_non_verbose_render_is_byte_identical` — les deux surfaces, les deux
  formats.
- `mika1883_the_inert_skill_flags_warn_on_stderr_only` — stdout intact sous
  `--format json`.
- `mika1883_cache_fields_accumulate_across_none_and_some` — `None + Some(n)`
  donne `Some(n)` ; un provider qui ne rapporte le cache que sur certains appels
  ne fait pas disparaître le total.
- Le doc-comment de `RUN_USAGE_KEY` porte la sémantique (tour, pas campagne ;
  RAW, pas normalisé), comme ses cinq sœurs.

### 7.3 Sonde post-déploiement, avec ses haltes

```bash
mika ask --agent mika-arch --verbose --session-id "probe-1883-$$" "compte jusqu'à trois"
grep turn_usage "$MIKA_SPIRIT_LOG_FILE" | jq 'select(.session_id == "probe-1883-…")'
```

La somme des `input_tokens` des lignes doit égaler le `tokens.input` affiché.

- **Halte 1 — l'affiché est inférieur à la somme, et proche d'une seule ligne :**
  c'est `last_usage` qui est servi. **Ne pas ajuster l'affichage** — l'agrégat
  n'est pas branché.
- **Halte 2 — rien n'est affiché alors que `turn_usage` porte des lignes :** le
  spirit qui tourne est antérieur au champ (classe mika#2340). Établir le
  déploiement avant de toucher au code ; c'est précisément la population que
  l'attestation rend visible.
- **Halte 3 — `input` déconcerte sur un rail OpenAI-compatible :** il inclut
  `cache_read` par construction (Signal O). Ce n'est pas un bug ; lire le
  `model:` de la même sortie avant de conclure.

---

## 8. Ce que ce travail n'achète pas

- Il ne mesure pas les délégations ni les runs d'équipe (§ 2).
- Il ne livre pas la moitié additive du canal, et ne rouvre pas son refus.
- Il ne livre pas `--disable-skill` côté serveur — il rend son inertie **dite et
  comptable**, ce qui est la condition pour qu'une mesure future puisse décider.
- Il ne normalise pas l'asymétrie cache/prompt entre familles de providers. Elle
  est portée sur le fil telle que les providers la rapportent, et dite.

---

## 9. Definition of Done

- L'agrégat est sommé dans la boucle, continuation comprise, derrière un helper
  unique.
- `AgentOutput.run_usage` traverse `A2aTurn` jusqu'à `stamp_run_usage`.
- `params.rs` porte une clé et **un** lecteur ; les deux surfaces clientes
  l'utilisent.
- `mika ask --verbose` et `mika ask --remote --verbose` affichent `tokens.*`, et
  n'affichent rien quand le serveur n'a rien attesté.
- Le non-verbose est byte-identique sur les deux surfaces et les deux formats.
- `--enable-skill` / `--disable-skill` émettent `cli_skill_flag_inert` sur stderr
  en nommant `--only-skill`.
- Le commentaire de report de `ask.rs` a disparu.
- `docs/skills.md`, `cli-skill-always-on-transient-override.md` et `CLAUDE.md`
  disent l'état réel du canal.
- `cargo test`, `cargo clippy`, `cargo fmt` propres.

---

## Acceptance criteria

Dérivés du corps du ticket et du commentaire opérateur du 30/08 (le ticket ne
porte pas de section `## Acceptance criteria`).

- **AC1 — Point 2 rebranché, et juste.** `mika ask --verbose` affiche l'usage
  **par tour**. Pour un tour à N appels LLM, la valeur affichée est la **somme**
  des N `turn_usage` de ce tour, continuation comprise — pas le dernier appel. Un
  test à usages distincts par appel échoue si l'implémentation rend
  `last_usage`.
- **AC2 — L'absence est une absence.** Quand le serveur n'atteste aucun usage
  (spirit antérieur, tour sans appel lisible, chemin hors `message/send`
  synchrone), le client n'affiche aucune ligne `tokens.*` et le JSON ne porte pas
  la clé. Jamais `0`, jamais `null`.
- **AC3 — Un seul lecteur, deux surfaces.** `mika ask --verbose` et
  `mika ask --remote --verbose` lisent l'attestation par la même fonction de
  `mika-a2a`. Un scan de source refuse un second décodage de la clé.
- **AC4 — Aucune régression de sortie.** Sans `--verbose`, la sortie des deux
  surfaces est byte-identique à celle d'avant, en texte comme en JSON.
- **AC5 — Point 1 tranché par écrit.** L'état des trois flags est écrit dans
  `CLAUDE.md` : `--model` livré (mika#2304), `--enable-skill` refusé avec sa
  raison (mika#2363), `--disable-skill` non livré avec sa raison (usage mesuré
  nul, troisième sémantique de sélection).
- **AC6 — Le no-op est dit.** Une invocation portant `--enable-skill` ou
  `--disable-skill` émet un avertissement nommant les skills, l'inertie, et
  `--only-skill` comme le canal qui atteint le serveur. L'avertissement est sur
  **stderr** ; `--format json` reste parsable sans changement.
- **AC7 — La documentation cesse d'affirmer le faux.** `docs/skills.md` et
  `docs/solutions/architecture-patterns/cli-skill-always-on-transient-override.md`
  ne décrivent plus `--enable-skill` / `--disable-skill` comme atteignant la
  surface d'exécution.
- **AC8 — Le report ne survit pas à sa résolution.** Le bloc « Deferred
  follow-ups » de `crates/mika-cli/src/commands/ask.rs` ne décrit plus les points
  1 et 2 comme ouverts.
