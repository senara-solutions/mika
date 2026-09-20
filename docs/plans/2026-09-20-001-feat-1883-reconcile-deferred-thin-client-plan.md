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
- **`--enable-skill` (moitié additive) : refusé, avec raison écrite.** Elle
  laisserait tout appelant authentifié de `/a2a/{agent}` forcer une skill de
  l'agent en `always_on` — élargir une surface depuis le réseau. Ce refus n'est
  pas à relitiger ici.

  **Où ce refus est écrit, et où il ne l'est pas** (rectification rev 2, F1).
  Le corps du ticket **mika#2363 ne le porte pas** : ce ticket s'intitule
  « Réduire la taille d'entrée d'un tour mika-arch (brief) sous le plafond HTTP
  — lever de fond #2362 D2 » et traite du volume du brief architecte. Ce qu'il a
  **livré** est `--only-skill`, et c'est le substrat écrit par sa PR qui porte le
  refus de la moitié additive, à trois sites vérifiables ce jour
  (HEAD `10ad8f8a`, 2026-09-20) :

  | site | ce qu'il pose |
  |---|---|
  | `crates/mika-a2a/src/params.rs:19-42` (doc-comment de `ONLY_SKILLS_KEY`) | « restricts the turn's skill registry … **by subtraction only** … The field can therefore never widen a turn's surface, **which is what makes it safe on an endpoint any authenticated caller can reach** » |
  | `skills/bundled/_shared/dispatch-lib.sh:4688-4691` | « It is strictly subtractive — it evicts the sister passes, **it cannot activate anything** » |
  | `CLAUDE.md` § mika#2304, « Hors périmètre, délibérément » | « `--enable-skill` / `--disable-skill`, la moitié **additive** du canal mika#1727, refusée par mika#2363 avec sa raison écrite (elle laisserait tout appelant authentifié forcer une skill en `always_on`) » |

  Le troisième site est celui qui **attribue** le refus au numéro mika#2363 ;
  les deux premiers sont ceux qui en portent la **raison**, dans le code, sans
  dépendre d'une lecture de GitHub. Ce plan s'ancre désormais sur les deux
  premiers, et ne cite le numéro que comme provenance du changement — la
  distinction que F1 a correctement relevée : une citation n'est portante que si
  elle résout, et un corps de ticket n'est pas son substrat.

- **`--disable-skill` : toujours inerte, et non tranché.** L'argument de refus
  ci-dessus ne s'y applique pas : retirer une skill est strictement subtractif —
  exactement la propriété que le doc-comment d'`ONLY_SKILLS_KEY` nomme comme
  *ce qui rend un champ sûr sur cet endpoint*. C'est le seul morceau du point 1
  encore ouvert. Le § 3 l'arbitre.

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

1. **Usage mesuré : zéro appelant.** La mesure est donnée avec sa méthode, pour
   être re-runnable (rev 2, F4) — un « personne ne l'utilise » est une
   affirmation de mesure et doit porter sa commande, sa date et son commit-ish :

   ```bash
   # HEAD 10ad8f8a, 2026-09-20
   grep -rn "\-\-disable-skill\|\-\-enable-skill" \
     --include="*.sh" --include="*.rs" --include="*.toml" --include="*.json" \
     skills scripts .claude crates
   ```

   Les `*.md` sont exclus **à dessein** : la documentation est le sujet du § 6,
   pas la mesure d'usage. Résultat, `--disable-skill` — **5 occurrences, zéro
   appelant** :

   | site | nature |
   |---|---|
   | `crates/mika-cli/src/cli.rs:276,284,285,291,300` | la **définition** du flag et ses doc-comments |
   | `crates/mika-cli/src/commands/ask.rs:359,390` | le commentaire de report et la validation de conflit |
   | `crates/mika-agent/src/skills/mod.rs:3731` | une **comparaison** en commentaire (« evicted the same way `--disable-skill` evicts ») |

   Aucune invocation `mika ask --disable-skill` dans `skills/`, `scripts/` ou
   `.claude/`. Symétriquement pour `--enable-skill` : les occurrences de
   `test-dispatch-lib.sh:597,6033` et `only_skills_arch_pass_2363.rs:112` sont
   des **assertions qu'il a disparu**, et `dispatch-lib.sh:4680` est le
   commentaire qui acte son retrait.

   **Une exception, et elle est un livrable** (trouvée par cette mesure) :
   `crates/mika-cli/src/commands/skills_variants.rs:514` **imprime à l'opérateur**
   la ligne `mika ask --enable-skill skill-review "…"` comme geste de
   régénération. Ce n'est pas un appelant exécuté, c'est pire — c'est du code
   vivant qui **conseille** un drapeau inerte, donc la même classe de mensonge
   que `docs/skills.md` mais dans un chemin que personne ne relit. Il rejoint le
   § 6 (geste C, point 5) et AC7.
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

**Le côté producteur est tenu par un scan, pas par convention** (rev 2, F5).
Tester le helper atteste que `None + Some(n) = Some(n)` est juste ; ça
n'atteste pas que les **deux sites** y passent — or le risque que §4.1 nomme
*est* la divergence des deux sites, pas la fausseté du helper. Un second site
qui ré-écrirait la fusion à la main rendrait un total faux **avec tous les
tests du helper au vert**, c'est-à-dire la panne silencieuse que ce dépôt
ferme par un scan partout où il l'a rencontrée. La discipline de lecteur
unique était appliquée au consommateur
(`mika1883_both_client_surfaces_read_the_one_reader`) et seulement
asserted-by-convention au producteur : la symétrie est rétablie par
`mika1883_run_usage_accumulates_only_via_the_one_helper` — scan de source
refusant, dans `agent_loop/`, toute addition sur les champs de `run_usage`
hors de l'appel au helper (modèle `mika2131_exclusion_skips_never_return_to_an_uncollected_debug`
et `mika2220_no_local_reparse_of_the_llm_bodies_env_var`). C'est le versant
écrivain du même principe que mika#2158, que ce plan citait déjà pour le
versant lecteur, et la forme que review-guide.md § DRY prescrit ici.

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

Le précédent est celui de `request_bytes`. **Sa source exacte, rectifiée** (rev
2, F2) : la formulation n'est pas dans le corps du ticket mika#2331 — qui porte
sur le retry d'`_arch_ask` sur hang transport — mais dans `CLAUDE.md` § *Signal
O — RT-005 per-turn token accounting*, paragraphe « Brief size, added by
mika#2331 (AC1) », qui pose, en anglais et vérifié ce jour (HEAD `10ad8f8a`) :

> They are `Option` and `null` is never `0` (no request is empty, so a zero
> would be a readable lie).

Ce plan citait une **traduction** de cette phrase en l'attribuant « mot pour
mot » au ticket ; les deux moitiés de l'attribution étaient fausses (ni le
ticket, ni verbatim), même si le principe est juste et le numéro de provenance
correct. La citation est désormais au texte, avec sa vraie adresse.

Le raisonnement s'y transpose **par analogie, pas par autorité** : là-bas
aucune requête n'est vide, ici aucun tour ne consomme zéro token d'entrée. Un
`0` affiché serait indistinguable d'un tour réel et ferait croire à une mesure
là où il n'y en a pas. Si un relecteur récuse l'analogie, c'est ce raisonnement
qu'il faut discuter — la décision est celle de ce plan, adossée à un précédent,
et non déléguée à lui.

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
5. **`crates/mika-cli/src/commands/skills_variants.rs:514`** — `run_regen`
   **imprime** à l'opérateur `mika ask --enable-skill skill-review "…"` comme
   geste de régénération d'une variante. Site trouvé par la mesure du § 3
   (rev 2, F4), et le plus coûteux des quatre : les trois autres sont de la
   documentation qu'on peut ne pas lire, celui-ci est une consigne qu'on
   **suit**, imprimée par l'outil lui-même au moment où l'opérateur en a
   besoin — elle produit un tour qui n'active pas `skill-review` et dont rien
   ne dit qu'il ne l'a pas activée. Remplacer par `--only-skill skill-review`,
   qui atteint spirit et porte la même intention.

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
  ne fait pas disparaître le total. **Teste le helper**, pas les appelants.
- `mika1883_run_usage_accumulates_only_via_the_one_helper` — scan de source
  (rev 2, F5) : dans `crates/mika-agent/src/agent_loop/`, aucune addition sur
  les champs de `run_usage` hors de l'appel à `LlmUsage::accumulate`. C'est le
  pendant écrivain de `mika1883_both_client_surfaces_read_the_one_reader`, et le
  seul des deux qui puisse voir un **second site** de fusion : le test
  précédent resterait vert pendant qu'un total faux serait servi.
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

## Fire-Disposition

Ajoutée en rev 2 (F3). Le plan livre des détecteurs — neuf tests dont deux
scans de source, plus un avertissement runtime — et la question n'est pas
« détectent-ils le défaut ? » (c'est le rôle du contrôle négatif du § 7.2) mais
**« que se passe-t-il quand ils tirent sur la population déjà en place ? »**.

**Branche (a) — allowlist nommée, zéro entrée.** Et c'est démontrable plutôt
qu'espéré, parce que chaque détecteur porte sur une surface que ce plan
introduit :

| détecteur | population pré-existante | pourquoi zéro |
|---|---|---|
| les 7 tests comportementaux (`…sums_every_call…`, `…continuation_is_counted`, `…attests_nothing`, `…non_verbose_render…`, `…warn_on_stderr_only`, `…cache_fields_accumulate…`) | aucune | tests CI sur du code neuf ; aucun `Task.metadata["mika.run_usage"]` n'existe, la clé naît ici |
| `mika1883_both_client_surfaces_read_the_one_reader` (scan lecteur) | aucune | `RUN_USAGE_KEY` n'existe pas encore : zéro décodage à allowlister |
| `mika1883_run_usage_accumulates_only_via_the_one_helper` (scan écrivain) | aucune | le champ `run_usage` naît ici ; les deux seuls sites d'addition sont créés par cette PR et routent par construction vers le helper |
| `cli_skill_flag_inert` (avertissement runtime) | **une, corrigée dans la même PR** | voir ci-dessous |

Les deux scans sont donc livrés **sans clause d'exception** : une allowlist
vide n'est pas un oubli mais la conséquence de l'ordre des choses, et la
première entrée qu'un futur éditeur voudrait y ajouter sera précisément la
divergence que le scan existe pour refuser. **Résolution prescrite quand un
scan tire : retirer le second site, jamais l'allowlister** — la règle que
`ACTOR_READING_PREDICATES_ALLOWED` (mika#2323) pose pour sa propre liste livrée
vide.

**Le seul détecteur à population non vide est `cli_skill_flag_inert`**, et elle
vaut d'être nommée plutôt que rangée sous « zéro » : il tire sur tout appelant
portant `--enable-skill` / `--disable-skill`. La mesure du § 3 en dénombre
**zéro exécuté** et **une consigne imprimée** —
`skills_variants.rs:514`, qui conseille `--enable-skill skill-review` à
l'opérateur. Cette entrée unique est **éteinte par le geste C point 5 dans la
même PR**, pas allowlistée : un détecteur qui tire sur une consigne que le même
changement corrige n'a pas de population résiduelle. Le régime attendu au
déploiement est donc zéro ligne, et le § 5 dit déjà ce qu'une ligne signifierait
— un appelant à migrer vers `--only-skill`, c'est-à-dire la mesure qui
rouvrirait un jour la décision du § 3.

**Ce que la Fire-Disposition ne couvre pas, et c'est voulu :** le champ
`tokens.*` du rendu `--verbose` n'est pas un détecteur — il ne refuse rien et
ne fait échouer aucun tour. Sa population d'absence (spirit antérieur, tour
sans usage lisible) est traitée en § 4.4 comme une **lecture**, pas comme un
tir, et la halte 2 du § 7.3 en donne la conduite.

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
  unique — et un scan de source refuse un second site d'addition.
- `AgentOutput.run_usage` traverse `A2aTurn` jusqu'à `stamp_run_usage`.
- `params.rs` porte une clé et **un** lecteur ; les deux surfaces clientes
  l'utilisent.
- `mika ask --verbose` et `mika ask --remote --verbose` affichent `tokens.*`, et
  n'affichent rien quand le serveur n'a rien attesté.
- Le non-verbose est byte-identique sur les deux surfaces et les deux formats.
- `--enable-skill` / `--disable-skill` émettent `cli_skill_flag_inert` sur stderr
  en nommant `--only-skill`.
- Le commentaire de report de `ask.rs` a disparu.
- `docs/skills.md`, `cli-skill-always-on-transient-override.md`, `CLAUDE.md`
  **et `skills_variants.rs`** disent l'état réel du canal — le dernier cessant
  de prescrire un drapeau inerte.
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
  `CLAUDE.md` : `--model` livré (mika#2304) ; `--enable-skill` refusé avec sa
  raison, **ancrée sur le doc-comment d'`ONLY_SKILLS_KEY` (`params.rs`) et sur
  `dispatch-lib.sh:4688-4691`** — les deux sites qui portent la raison — le
  numéro mika#2363 n'étant cité que comme provenance du changement ;
  `--disable-skill` non livré avec sa raison (zéro appelant mesuré, troisième
  sémantique de sélection). Toute citation de ce bloc résout sur un site
  nommé.
- **AC6 — Le no-op est dit.** Une invocation portant `--enable-skill` ou
  `--disable-skill` émet un avertissement nommant les skills, l'inertie, et
  `--only-skill` comme le canal qui atteint le serveur. L'avertissement est sur
  **stderr** ; `--format json` reste parsable sans changement.
- **AC7 — Plus aucune surface n'affirme ni ne conseille le faux.**
  `docs/skills.md` et
  `docs/solutions/architecture-patterns/cli-skill-always-on-transient-override.md`
  ne décrivent plus `--enable-skill` / `--disable-skill` comme atteignant la
  surface d'exécution, **et `crates/mika-cli/src/commands/skills_variants.rs`
  n'imprime plus `--enable-skill` comme geste à suivre** — il nomme
  `--only-skill`. La vérification est la commande du § 3 rejouée : elle ne doit
  plus rendre de site *prescriptif*, seulement la définition du flag, sa
  validation de conflit et les assertions de retrait.
- **AC9 — Un seul écrivain, deux sites d'addition.** Un scan de source refuse,
  dans `agent_loop/`, toute addition sur les champs de `run_usage` hors de
  l'appel à `LlmUsage::accumulate`. Il est livré **sans entrée d'exception**, et
  sa résolution quand il tire est de retirer le second site, jamais de
  l'allowlister.

---

## Revision history

- **rev 2 (2026-09-20)** — répond aux cinq findings de la première passe
  architecte. Aucun finding n'a été écarté, et deux ont déplacé le contenu au-delà
  de la correction demandée.

  - **F1 (bloquant) — citation mika#2363 non résolvante.** Fondé. Vérifié ce
    jour : le corps du ticket porte sur la taille du brief architecte, pas sur
    le refus de la moitié additive. Le refus **existe** mais dans le substrat
    écrit par sa PR. § 1.2 est ré-ancré sur trois sites nommés et vérifiables à
    HEAD `10ad8f8a` — `params.rs:19-42` (« by subtraction only … never widen a
    turn's surface … safe on an endpoint any authenticated caller can reach »),
    `dispatch-lib.sh:4688-4691`, et `CLAUDE.md` § mika#2304 « Hors périmètre »,
    ce dernier étant celui qui **attribue** le refus au numéro. Le plan ne cite
    plus le numéro que comme provenance. AC5 porte désormais cette exigence
    d'ancrage. Pas d'ESCALATE : le refus n'était pas introuvable, il était mal
    adressé.
  - **F2 (bloquant) — faux verbatim mika#2331.** Fondé sur les deux moitiés de
    l'attribution. La phrase existe, en **anglais**, dans `CLAUDE.md` § Signal O
    (« Brief size, added by mika#2331 (AC1) ») et non dans le corps du ticket ;
    le plan en citait une traduction française comme un « mot pour mot ». § 4.4
    cite maintenant le texte anglais en bloc-quote avec sa vraie adresse, et
    présente la transposition comme une **analogie assumée par ce plan**, pas
    comme une autorité empruntée.
  - **F3 (bloquant) — `## Fire-Disposition` absente.** Ajoutée. Branche (a),
    allowlist nommée à **zéro entrée**, démontrée détecteur par détecteur plutôt
    qu'affirmée : chaque surface visée (`RUN_USAGE_KEY`, le champ `run_usage`,
    les deux sites d'addition) naît dans cette PR. Un détecteur a une population
    non vide — `cli_skill_flag_inert`, une entrée, `skills_variants.rs:514` — et
    elle est **éteinte dans la même PR** plutôt qu'allowlistée ; la section le
    dit au lieu de la ranger sous « zéro ». Résolution prescrite pour les deux
    scans : retirer le second site, jamais l'allowlister (modèle mika#2323).
  - **F4 (affûtage) — mesure `--disable-skill` sans poignée.** Fondé, et le plus
    productif des cinq. § 3 porte la commande exacte, sa date, son commit-ish,
    l'exclusion volontaire des `*.md` et le tableau des 5 occurrences par nature
    — **zéro appelant**. Rejouer la mesure a fait apparaître un site que le plan
    manquait : `skills_variants.rs:514` **imprime à l'opérateur** un geste
    `--enable-skill` inerte. Ajouté au § 6 (geste C, point 5), à AC7, au § 9 et
    à la Fire-Disposition. Le finding demandait une méthode ; il a rendu un
    livrable.
  - **F5 (affûtage) — invariant producteur non tenu.** Fondé : le test du helper
    atteste la fusion, pas le routage des deux sites vers lui. Option (a)
    retenue, celle que le finding jugeait la plus conforme aux précédents du
    plan — `mika1883_run_usage_accumulates_only_via_the_one_helper` (§ 7.2), le
    pendant écrivain du scan lecteur déjà prévu, plus le paragraphe de § 4.1 qui
    dit pourquoi le test comportemental ne peut pas voir cette classe. AC9
    ajoutée.

  Aucune AC n'a été affaiblie. AC5 et AC7 sont **resserrées** (exigence
  d'ancrage résolvant ; surface prescriptive ajoutée), AC9 est nouvelle.
- **AC8 — Le report ne survit pas à sa résolution.** Le bloc « Deferred
  follow-ups » de `crates/mika-cli/src/commands/ask.rs` ne décrit plus les points
  1 et 2 comme ouverts.
