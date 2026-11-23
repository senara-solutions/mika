# fix(3126) — une coupure au plafond ne donne plus droit à la concession de relance rapide

**Ticket :** senara-solutions/mika#3126
**Labels :** `p2-normal`, `dispatch:loop`
**Sources :** commentaire 53 (D1) du parking #2983

---

## Le défaut, tel qu'il se mesure

Le 16/11, trois tours (`dd21ea95`, `c66e3814`, `fef8ca95`) ont été coupés en
`llm_call_cap_exhausted` — un corps de réponse qui cesse d'arriver à ≈ 100 % du
plafond par appel, ce que mika#2280 a nommé *guillotine* : le modèle générait
encore. La chaîne a relancé avec **290 à 345 s restants** sous un plafond de
**300 s**, la relance a elle aussi consommé son plafond entier, et l'enveloppe a
été abandonnée à **660 s** pour **4,82 $** sans qu'un seul octet utilisable soit
revenu. Le tour A2A est mort avec zéro marge : la boucle n'a plus eu de quoi
conclure.

### La cause, en une ligne

`crates/mika-common/src/llm/retry_gate.rs:77-90` dérive deux seuils, et une
coupure au plafond tombe dans le **plus permissif des deux**.

| classe du dernier échec | seuil de marge exigé | justification d'origine |
|---|---|---|
| transport (mika#1744) | `0,50 × plafond` | « DNS, connexions refusées et poignées TLS se résolvent en secondes, pas en un plafond entier » |
| tout le reste | `0,75 × + 0,25 × = 1,0 × plafond` | un appel typique plus une réserve |

Une guillotine est un `LlmError::Transport` — `openai.rs:367` le dit
explicitement, et mika#2280 a **délibérément** laissé la variante, la classe et
la retryabilité intactes (D2/AC6), parce que mika#2015 avait mesuré que les
rendre terminales tuait un cycle de pilote entier sur chaque hoquet réseau en
milieu de corps. Donc `LlmError::is_transport()` rend `true`, le seuil appliqué
est `0,50 × 300 = 150 s`, et `290 ≥ 150` : la relance part.

**C'est la prémisse de la concession qui est fausse, pas sa valeur.** mika#1744
l'a accordée à une classe de pannes *qui se résolvent en secondes*. Une tentative
coupée à son plafond vient de démontrer l'exact contraire : elle a consommé
l'intégralité du plafond. Le seuil `0,50 ×` est calibré sur une panne rapide et
appliqué à la seule panne dont on sait qu'elle sera lente.

### Ce que le défaut coûte, précisément

La mesure que la concession annule n'est pas « la relance échoue » — c'est « la
relance part avec moins d'un appel entier de marge ». À 290 s restants sous un
plafond de 300 s, la seconde tentative **ne peut pas finir** : elle sera coupée
par l'enveloppe avant son propre plafond. On dépense 290 s et la moitié des
4,82 $ pour un appel dont l'arithmétique dit, à l'instant où on le lance, qu'il
ne rendra rien. Et comme l'erreur ne revient à la boucle qu'à 660 s, il ne reste
rien de l'enveloppe pour que le filet de verdict d'erreur (mika#2289,
`server::deadline_verdict`) poste quoi que ce soit : le tour A2A est perdu en
silence au lieu d'être perdu en disant pourquoi.

### La mesure de non-régression, qui décide du périmètre

Retirer la concession n'est un resserrement **sur aucune géométrie de la
flotte** :

| géométrie | plafond | seuil `1,0 ×` | marge après une coupure | verdict avant | verdict après |
|---|---|---|---|---|---|
| 120/300 (défaut flotte) | 120 | 120 | `300 − 120 = 180` | relance | **relance** (inchangé) |
| 240/900 (mika-arch) | 240 | 240 | `900 − 240 = 660` | relance | **relance** (inchangé) |
| 300/660 (l'incident) | 300 | 300 | `290` | relance | **refus** |

Les deux géométries en service ne bougent pas. Seule celle de l'incident, où
l'enveloppe ne porte pas deux appels entiers, change — ce qui est exactement la
famille `E ≈ k·P` que mika#2362 a décrite sous le nom de
`effective_max_attempts`.

---

## Requirements

**R1.** Après une tentative coupée à son plafond, la décision de relance exige
la marge d'**un appel entier** (le seuil non-transport, `1,0 × plafond`), jamais
la concession transport (`0,50 ×`).

**R2.** La classe est décidée **en un seul site**. Les trois rails lisent
aujourd'hui `is_some_and(LlmError::is_transport)` chacun chez eux ; ajouter
`&& !cap_exhausted` à trois endroits, c'est recopier un prédicat — exactement la
dérive que `retry_gate` existe pour fermer (mika#2362 : trois copies d'un seuil,
dont une jamais portée sur le rail ollama pendant deux tickets, mika#2331 §3.3).

**R3.** La ligne `outcome` et la garde continuent de lire **le même** prédicat.
Si seule la garde apprenait la nouvelle classe, `llm_call_attempt` annoncerait
`retrying` pour une tentative que la garde refuse — la divergence fondatrice de
mika#2362, réouverte.

**R4.** Le refus **nomme sa cause**. Le WARN de la garde porte aujourd'hui
`last_was_transport = true` ; sur une guillotine refusée il doit être lisible
que c'est la coupure au plafond, et non la classe d'erreur, qui a décidé.

**R5.** Aucun changement de variante d'erreur, de `error_class`, de
retryabilité, de `max_attempts` ni de `worst_case_failure_secs`. mika#2280 a
posé l'invariant (le drapeau voyage **à côté** de l'erreur, jamais dedans) et
mika#2342 dimensionne son chien de garde sur `max_attempts` — un resserrement
du compte coupe des chaînes saines.

**R6.** Le rail Anthropic ne change pas de comportement. Il ne porte pas de
`LlmTimeoutBudget` et sert à `reqwest` le littéral `ANTHROPIC_HTTP_TIMEOUT_SECS`
(mika#2189, hors périmètre) ; `is_cap_exhaustion` ne doit pas lui être posée —
`budget.rs:434-437` le dit — donc il ne peut jamais produire la nouvelle classe.

---

## Design

### D1 — `retry_gate::transport_concession_applies` : le site unique de l'appartenance

Une fonction pure, à côté des deux autres prédicats du module :

```rust
pub fn transport_concession_applies(
    error: Option<&dyn RetryClass>,
    cap_exhausted: bool,
) -> bool {
    !cap_exhausted && error.is_some_and(RetryClass::is_transport)
}
```

Elle remplace, aux neuf sites, le `is_some_and(LlmError::is_transport)` écrit
sur place. Son doc-comment porte la doctrine : la concession mika#1744 est
accordée à une classe de pannes *identifiée comme rapide*, et une tentative
coupée à son plafond a réfuté cette appartenance pour la suivante ; ce n'est
donc pas un troisième seuil, mais un **retrait d'exception** dont le repli est
le seuil par défaut qui existe déjà.

**Pourquoi une fonction plutôt qu'un troisième seuil.** Un
`cap_exhaustion_secs = plafond + réserve` (`1,25 ×`) refuserait aussi les deux
tours de l'incident partis à 320 et 345 s. Il est écarté de ce vol pour deux
raisons : il refuse une seconde tentative qui, à `345 ≥ 300`, **peut** finir
dans l'enveloppe, et il introduit une quatrième valeur dérivée à faire vivre.
Le résidu est nommé et tracké au § *Périmètre*.

**Pourquoi pas une énumération de classe.** Trois variantes
(`Default`/`Transport`/`CapExhaustion`) obligeraient `deadline_verdict`,
`for_class`, `RetryThresholds` et les neuf sites à changer de forme pour porter
une information que deux seuils suffisent à sélectionner. Le `bool` que les
signatures prennent déjà reste juste ; ce qui manquait, c'est qu'il soit
**calculé en un seul endroit**.

### D2 — `next_attempt_verdict` gagne le drapeau

```rust
pub fn next_attempt_verdict(
    attempt: u32,
    max_attempts: u32,
    error: Option<&dyn RetryClass>,
    cap_exhausted: bool,          // ← nouveau
    remaining: Option<Duration>,
    thresholds: &RetryThresholds,
) -> RetryVerdict
```

Le corps route par D1 au lieu de `error.is_some_and(RetryClass::is_transport)`.
C'est R3 : la ligne `outcome` est le seul consommateur de cette fonction, et
sans le drapeau elle annoncerait `retrying` pour ce que la garde refuse.
`RetryVerdict` ne change pas de forme — le refus porte déjà `remaining_ms` et
`threshold_secs`, et les rails tiennent la classe pour le WARN.

### D3 — `deadline_verdict` : même signature, paramètre renommé

`last_was_transport: bool` devient `transport_concession: bool`, avec une ligne
de doc disant qu'il doit venir de [`transport_concession_applies`] et jamais
d'un `is_transport` lu seul. Le nom actuel est devenu un faux ami : sur une
guillotine, `is_transport` est vrai et la concession ne s'applique pas.

### D4 — les deux rails de forme OpenAI portent le drapeau qu'ils mesurent déjà

C'est là que vit le défaut réel. `openai.rs:593-597` et `ollama.rs:722-725`
reçoivent `attempt_cap_exhausted` de `send_once`, l'émettent sur
`llm_call_attempt` (`Some(attempt_cap_exhausted)`) — **et le laissent tomber**.
`last_error` ne garde que l'erreur, donc la garde du tour suivant ne peut pas
voir la mesure. Trois gestes par rail :

1. `let mut last_cap_exhausted = false;` à côté de `let mut last_error = None;` ;
2. dans le bras `Err(e)` qui `continue` (`openai.rs:656-659`), assigner
   `last_cap_exhausted = attempt_cap_exhausted;` juste après `last_error` ;
3. les trois sites lisent D1 :
   - la garde (`openai.rs:512-514`, `ollama.rs:666-668`),
   - la ligne `outcome` (`openai.rs:608`, `ollama.rs:735`) — qui passe
     `attempt_cap_exhausted`, le drapeau de **la tentative courante**, parce que
     c'est de son échec qu'on demande s'il permet un suivant,
   - le message post-boucle (`openai.rs:673-675`, `ollama.rs:792-794`).

Le WARN de la garde (R4) remplace `last_was_transport` par deux champs :
`transport_concession` (la décision) et `last_cap_exhausted` (la mesure qui l'a
produite). Un refus où le second est vrai et le premier faux est, littéralement,
l'incident de ce ticket lu dans le log.

### D5 — le rail Anthropic passe `false`, et le dit

`claude.rs:639-645` et `claude.rs:722` passent `cap_exhausted = false` avec un
commentaire renvoyant à `budget.rs:434-437` : ce rail n'applique pas le plafond
qu'il déclare, donc la question n'a pas de référent chez lui et `false` est la
réponse honnête, pas un défaut d'instrumentation. `claude.rs:847-850` fait de
même. Comportement strictement inchangé (R6) — ce que les tests
`mika2362_anthropic_*` existants attestent en restant verts.

### Ce qui n'est pas touché

- **Le scan structurel de mika#2362**
  (`mika2362_no_rail_recomputes_the_retry_threshold_inline`) : son prédicat
  cherche `typical_call_duration_secs()` **et** `retry_buffer_secs()` sur une
  même ligne. D1 n'écrit aucune somme et ne touche aucun des deux accesseurs :
  le scan ne tire pas, et n'a rien à apprendre. Voir aussi § *Fire-Disposition*.
- **`budget.rs`** : aucun nouvel accesseur, aucune nouvelle fraction. Le seuil
  appliqué après une guillotine est le `default_secs()` qui existe déjà.
- **`attempt_outcome`** : un refus par la deadline reste `exhausted`
  (mika#2362), et `deadline_abort` garde son invariant `elapsed_ms = 0`.
- **Le format de fil** : aucun déplacement de population sous `GROUP BY outcome`.
  Une guillotine refusée disait déjà `exhausted` quand le budget était épuisé ;
  ici elle le dit quand la deadline refuse, ce que mika#2362 a déjà établi
  comme la bonne réponse pour cette branche.

---

## Périmètre — ce vol et la suite

**Ce vol :** le retrait de la concession, ses trois rails, ses tests unitaires,
et une preuve de bout en bout sur le rail où l'incident a eu lieu.

**Renvoyé, avec son arithmétique :** le seuil `plafond + réserve` (`1,25 ×`),
qui refuserait aussi les deux tours partis à 320 et 345 s restants. Il demande
un quatrième seuil dérivé dans `budget.rs`, son accesseur, sa place dans
`RetryThresholds`, et l'arbitrage « refuser une seconde tentative qui aurait pu
finir » — contre « la laisser finir pile sur l'enveloppe, sans rien laisser à la
boucle ». C'est une décision de géométrie, pas un correctif de prédicat, et
mika#2362 a posé la règle : la valeur se décide avec la mesure en main, par
l'opérateur.

**Renvoyé aussi :** la couverture d'intégration du rail ollama (son portage est
le même code que celui d'openai, ligne pour ligne, et la décision qu'il appelle
est couverte à l'unité), et le garde structurel qui refuserait un dixième site
lisant `is_transport` directement — voir § *Fire-Disposition* pour pourquoi il
ne tient pas dans ce vol.

Tracked in: un ticket de suivi `fix(llm): un seuil propre à la guillotine, et le
garde structurel de l'appartenance transport`, à ouvrir par l'implémentation et
nommé dans le corps de PR.

---

## Verification Contract

**V1 — la décision, à l'unité, sur la géométrie de l'incident.**
`cargo test -p mika-common --lib retry_gate` :
`mika3126_une_coupure_au_plafond_perd_la_concession_de_relance_rapide` construit
`LlmTimeoutBudget::new(300, 660)`, une erreur `Transport`, et une marge de
290 s. Avec `cap_exhausted = true` → `DeadlineInsufficient { threshold_secs: 300 }`
et `outcome_for` → `"exhausted"`. **Contrôle positif dans le même test**, à la
*même marge* : avec `cap_exhausted = false` (une panne de transport rapide) →
`Retry`. Sans lui, un prédicat qui refuserait inconditionnellement passerait.

**V2 — la flotte ne bouge pas.**
`mika3126_la_geometrie_de_la_flotte_relance_toujours_apres_une_coupure` :
`LlmTimeoutBudget::default()` (120/300), marge 180 s, `cap_exhausted = true` →
`Retry` (`180 ≥ 120`). Idem sur 240/900 à 660 s de marge. C'est la propriété qui
distingue « on a retiré une exception mal fondée » de « on a resserré la chaîne
de toute la flotte ».

**V3 — la table de vérité de l'appartenance.**
`mika3126_l_appartenance_a_la_concession_se_decide_en_un_site` sur les quatre
paires : `(transport, cut)` → `false`, `(transport, pas cut)` → `true`,
`(non-transport, cut)` → `false`, `(None, pas cut)` → `false`. La coupure
**domine** la classe d'erreur, et une erreur non classifiée n'hérite jamais de la
concession (l'invariant que `retry_gate` porte déjà).

**V4 — le rail porte le drapeau, de bout en bout.**
`cargo test -p mika-common --test llm_retry` :
`mika3126_openai_une_coupure_au_plafond_ne_relance_pas_sous_un_appel_entier`,
sur le serveur TCP réel du fichier, deux bras et une seule différence entre eux.
- Bras guillotine : `Reply::HeadersThenSilence`, budget `unvalidated(4, 12)`
  (plafond 4 s → seuils 2 / 4 ; `max_attempts = 3`, donc le budget permet une
  suite et c'est bien la deadline qui refuse), deadline à `now + 8 s`. La
  tentative 0 est coupée à ≈ 4 s, la marge vaut ≈ 4 s, le seuil appliqué est 4 →
  `api.hits() == 1`, et la ligne capturée porte `cap_exhausted = "true"` avec
  `outcome = "exhausted"`.
- Bras contrôle : `Reply::TruncatedBody` (coupure immédiate, `elapsed ≈ 0`, donc
  pas une guillotine) puis `Reply::Ok`, deadline à `now + 4 s` — **la même marge
  de ≈ 4 s**. Le seuil appliqué est 2 → `api.hits() == 2` et l'appel réussit.

Deux marges égales, deux verdicts opposés, et la seule variable est la coupure
au plafond. C'est le test que ni `MockLlmProvider` ni un test unitaire ne peuvent
rendre : le défaut réel est que le rail *mesure* le drapeau et ne le *lit* pas.

**V5 — rien d'autre n'a bougé.** `cargo test -p mika-common` en entier, en
particulier :
- `mika2280_openai_a_cut_at_the_plafond_is_attributed_and_still_retried` reste
  vert — il appelle `send_message` **sans** deadline, donc la garde ne s'applique
  pas et la chaîne retente comme avant (AC6 de mika#2280, intact) ;
- `mika2362_*` (les neuf, sur les trois rails) ;
- `mika2362_worst_case_is_unchanged_by_the_effective_count` (le chien de garde
  mika#2342 ne bouge pas) ;
- `mika2362_no_rail_recomputes_the_retry_threshold_inline` et son contrôle
  `mika2362_the_threshold_detector_detects`.

**V6 — les portes du dépôt.** `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check`, `make verify-bundled-skills`,
`bash scripts/verify-pipeline.sh origin/main`.

**V7 — lecture opérateur (post-déploiement, pas une porte de CI).**
```
grep llm_call_attempt $MIKA_SPIRIT_LOG_FILE \
  | jq -r 'select(.cap_exhausted == "true") | [.attempt, .outcome, .deadline_remaining_ms] | @tsv'
```
Régime attendu : sur une géométrie dont l'enveloppe ne porte pas deux appels
entiers, une guillotine en tentative 0 est suivie de `exhausted` et d'aucune
tentative 1. **L'absence de ligne ne prouve rien** — elle dit qu'aucune coupure
au plafond n'a eu lieu sur la fenêtre, ce qui est le régime nominal.

---

## Definition of Done

1. `transport_concession_applies` existe dans `retry_gate.rs`, documentée, et
   est le seul site où l'appartenance à la concession mika#1744 est décidée.
2. `next_attempt_verdict` porte `cap_exhausted` et route par elle ;
   `deadline_verdict` a son paramètre renommé et documenté.
3. `openai.rs` et `ollama.rs` retiennent `last_cap_exhausted` et alimentent
   leurs trois sites ; leur WARN de garde porte `transport_concession` et
   `last_cap_exhausted`.
4. `claude.rs` passe `false` à ses trois sites, avec le commentaire qui dit
   pourquoi c'est la réponse honnête et non une lacune.
5. V1 à V4 passent, V5 ne régresse pas, V6 est vert.
6. `docs/operator/environment-variables.md` § `llm_call_cap_exhausted` gagne la
   règle de lecture : une coupure au plafond n'achète plus de relance rapide, et
   ce que cela change par géométrie (le tableau du § *La mesure de
   non-régression*).
7. Le corps de PR nomme le ticket de suivi par une ligne `Tracked in:` (le seuil
   `1,25 ×`, la couverture ollama, le garde structurel).

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés des Requirements et du Verification Contract.

**AC1.** Après une tentative coupée à son plafond, la relance exige une marge
d'au moins un appel entier (`1,0 × plafond`). Pinné par V1 (refus à 290 s sous
un plafond de 300 s).

**AC2.** La concession transport `0,50 ×` reste intacte pour les pannes qu'elle
a été écrite pour couvrir : à la même marge de 290 s, une panne de transport non
coupée au plafond relance toujours. Pinné par V1 (contrôle positif) et V4 (bras
contrôle).

**AC3.** Aucune géométrie en service ne change de comportement : 120/300 et
240/900 relancent après une coupure exactement comme avant. Pinné par V2.

**AC4.** L'appartenance à la concession est décidée en **un** site, et la
coupure au plafond y domine la classe d'erreur. Pinné par V3.

**AC5.** La ligne `outcome` et la garde rendent le même verdict sur une
guillotine refusée : `exhausted`, jamais `retrying`. Pinné par V1 et V4.

**AC6.** Le refus nomme sa cause : le WARN de la garde porte à la fois la
décision (`transport_concession`) et la mesure qui l'a produite
(`last_cap_exhausted`).

**AC7.** Aucune variante d'erreur, `error_class`, retryabilité, `max_attempts`
ni `worst_case_failure_secs` ne change. Pinné par V5
(`mika2280_openai_a_cut_at_the_plafond_is_attributed_and_still_retried`,
`mika2362_worst_case_is_unchanged_by_the_effective_count`).

**AC8.** Le rail Anthropic ne produit jamais la nouvelle classe et son
comportement est inchangé. Pinné par V5 (`mika2362_anthropic_*`).

---

## Fire-Disposition

**Gate N/A : ce vol ne livre aucun détecteur.**

Les six tests livrés (V1 à V4) sont comportementaux : leur chemin de succès est
« la fonction a rendu la valeur attendue » sur des entrées construites, pas
« aucune violation trouvée » sur un balayage de l'arbre. Aucune règle de lint,
aucune garde CI, aucun scan structurel, aucun validateur de schéma, aucune garde
EndTurn n'est ajouté. Le seul scan de la zone,
`mika2362_no_rail_recomputes_the_retry_threshold_inline`, n'est **pas touché**
et n'a rien à apprendre : son prédicat exige les deux accesseurs de la somme
non-transport sur une même ligne, et D1 n'écrit aucune somme.

**Le détecteur qu'on ne livre pas, et pourquoi.** La dérive que ce ticket crée
est réelle et nommable : un dixième site qui écrirait
`is_some_and(LlmError::is_transport)` au lieu d'appeler
`transport_concession_applies` — exactement la classe mika#2362 (trois copies
d'un prédicat, dont une jamais portée sur un rail pendant deux tickets). Un scan
qui la refuserait **n'aurait pas d'allowlist vide** : `error.rs` définit
`is_transport` et l'assertionne dans ses propres tests, et `claude.rs` porte un
`is_transport_class` local au rail, tous deux légitimes. Un détecteur avec des
exceptions réelles demande, par mika#1574, une entrée grep-visible par violation
avec son ticket et son assertion auto-nettoyante — ce qui le fait sortir de la
borne par vol ci-dessous. Il est donc explicitement renvoyé au ticket de suivi
du § *Périmètre*, avec sa disposition déjà décidée : **option (a), exception
nommée en allowlist**, deux entrées, chacune avec l'assertion qui rougit si le
site disparaît.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-common/src/llm/retry_gate.rs` — `transport_concession_applies` + doc | 35 |
| `crates/mika-common/src/llm/retry_gate.rs` — signatures D2/D3 + corps | 18 |
| `crates/mika-common/src/llm/retry_gate.rs` — tests V1/V2/V3 | 94 |
| `crates/mika-common/src/llm/retry_gate.rs` — édition des tests existants (9 appels) | 9 |
| `crates/mika-common/src/llm/openai.rs` — portage du drapeau, 3 sites, WARN | 22 |
| `crates/mika-common/src/llm/ollama.rs` — idem | 22 |
| `crates/mika-common/src/claude.rs` — 3 sites à `false` + commentaire | 14 |
| `crates/mika-common/tests/llm_retry.rs` — test d'intégration V4, deux bras | 45 |

Total estimé : 259 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** La décision est une fonction pure
de six lignes ; les trois rails ne gagnent qu'un drapeau qu'ils **mesurent
déjà** et laissaient tomber (`attempt_cap_exhausted` existe depuis mika#2280,
il est même émis sur `llm_call_attempt`) ; `budget.rs` ne change pas, aucun
seuil n'est ajouté, aucune énumération de classe n'est introduite. Le poids est
dans les tests — 148 des 259 lignes — et c'est délibéré : trois sondes
unitaires, chacune portant ses deux contrôles dans le même test selon la
discipline du module, plus une sonde d'intégration à deux bras sur le rail de
l'incident. Les trois extensions qui feraient sortir de la borne (le seuil
`1,25 ×`, la couverture d'intégration ollama, le garde structurel de
l'appartenance) sont nommées au § *Périmètre* et renvoyées par une ligne
`Tracked in:`.

---

## Ordre de commit

1. **`refactor(llm): l'appartenance à la concession transport se décide en un
   site`** — `transport_concession_applies` dans `retry_gate.rs`, la signature
   de `next_attempt_verdict`, le renommage dans `deadline_verdict`, et les neuf
   sites convertis à l'identique (`cap_exhausted = false` partout). Aucun
   changement de comportement : l'arbre est vert avant et après, et les tests
   existants l'attestent. C'est le plancher — si la suite est abandonnée, ce
   commit reste correct et mergeable.
2. **`fix(llm): une coupure au plafond perd la concession de relance rapide`** —
   les deux rails de forme OpenAI retiennent `last_cap_exhausted` et
   l'alimentent ; le WARN gagne ses deux champs ; `claude.rs` gagne son
   commentaire. C'est ici, et seulement ici, que le comportement change.
3. **`test(llm): la relance après une guillotine exige un appel entier de
   marge`** — V1, V2, V3 dans `retry_gate.rs`, V4 dans `tests/llm_retry.rs`.
   Écrit après 2 pour que chaque sonde soit **vue rouge sur l'arbre d'avant**
   (`git stash` du commit 2 n'est pas une option dans un worktree partagé — la
   vérification se fait en inversant l'assertion une fois, localement, avant de
   la figer) : une sonde qui n'a jamais rougi n'a jamais montré qu'elle voit.
4. **`docs(operator): une coupure au plafond n'achète plus de relance rapide`** —
   `docs/operator/environment-variables.md`, la règle de lecture et le tableau
   par géométrie.
