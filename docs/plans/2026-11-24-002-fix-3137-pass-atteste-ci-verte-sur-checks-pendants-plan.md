# fix(3137) — un `pass` dont le corps atteste la CI verte est refusé quand des vérifications requises tournent encore

- **Ticket :** senara-solutions/mika#3137
- **Type :** fix
- **Composant :** `crates/mika-agent` (`evidence::guards`, `skills::builtin_handlers`), `skills/bundled/qa-review`
- **Voisin nommé par le ticket :** #3135 (une seule étape rejouée, tête citée absente) — **hors périmètre**, voir § *Périmètre*

---

## Le défaut, tel qu'il se mesure

Quatre occurrences, deux jours, trois PR.

| instant | PR / tête | ce que le corps affirme | état réel des vérifications requises |
|---|---|---|---|
| 2026-11-21 15:46:18Z | #3133 / `176bab64` | « check_suite success » | `Check`, `Dashboard`, `Security` en cours ; **`Security` rouge à 15:54Z** |
| 2026-11-21 16:39:26Z | #3133 / `33a4f0df` | « CI green » | conclues à 17:02:26Z (vertes) |
| 2026-11-22 18:59:56Z | #3139 | `pass` 84 s après l'ouverture | conclues à 19:15:45Z (vertes) |
| 2026-11-22 20:44:36Z | #3143 | `pass` 61 s après l'ouverture | conclues à 21:00:02Z (vertes) |

### Le site exact

Le gate CI↔verdict de mika#2455 existe déjà et tourne **avant** le sous-processus
qui poste la revue : `skills::builtin_handlers::validate_qa_ci_coherence_with_reader`
(`builtin_handlers.rs:4710`). Il lit `gh pr checks --required` **côté moteur**
(le modèle ne voit toujours pas la CI, mika#2423), classe par
`evidence::guards::classify_ci_coherence` → `pr_merge_with_gate::classify_checks`,
et refuse un `pass` sur bucket `fail`/`cancel`.

Le trou est une **branche nommée** de ce gate, `CiCoherenceOutcome::AllowedPending`
(`builtin_handlers.rs:4848-4858`, R6/AC3 de mika#2455) :

```rust
// R6/AC3: a pending check refuses nothing. The review can legitimately
// start before the CI has concluded — `pull_request.opened` routes to
// mika-qa with no CI term at all — so refusing here would refuse the
// nominal case.
CiCoherenceOutcome::AllowedPending => { audit(…, "allowed_pending", …); return Ok(()); }
```

Cette branche a raison sur ce qu'elle protège et aveugle sur ce qu'elle laisse
passer. Elle protège la revue **précoce** : `pull_request.opened` route vers
mika-qa, et les occurrences 3 et 4 le montrent au chronomètre (61 s et 84 s après
l'ouverture — `qa_review_submission::REVIEW_TRIGGERING_ACTIONS` contient
`opened`). Mais elle décide sur les **checks seuls**, jamais sur ce que le corps
**affirme** d'eux. Un corps qui écrit « CI green » pendant que trois vérifications
requises tournent passe donc par la porte prévue pour le corps qui n'affirme rien.

### Pourquoi le corps affirme ça

Le modèle n'a aucun accès CI : `qa_pr_view` retire les champs à la source
(`handlers/qa_pr_view.sh`, `SAFE_FIELDS`), `system_prompt.md:46` interdit
`gh pr checks` / `check-runs` / `statusCheckRollup`. Donc « check_suite success »
n'est pas une lecture : c'est la **paraphrase de son déclencheur**. Le handler
`qa-review-webhook-success` part sur `check_suite.completed(success)` — et un seul
push produit jusqu'à huit `check_suite.completed` (mika#1869, cité par
`merge_gate_rerun.rs:79`). Le premier suite vert déclenche la revue ; la QA relit
le mot « success » de l'événement comme s'il valait pour l'ensemble des
vérifications requises. Occurrence 1 : la suite qui avait conclu n'était pas celle
qui a fini rouge huit minutes plus tard.

L'occurrence 3 et la 4 sont l'autre moitié : à 61 s de l'ouverture, **aucun**
check_suite n'avait conclu, et le corps a quand même écrit un terme de CI verte.
Il n'y avait rien à paraphraser.

### Ce que le défaut coûte, et ce qu'il ne coûte pas

Il ne fait **pas** merger du code rouge : un `pass` route vers
`pr_merge_with_gate`, qui relit `gh pr checks --required` sans limite de temps et
refuse sur tout bucket `fail`/`cancel` (mika#485/#490). C'est l'arbitrage déjà
écrit au point 1 de mika#2455 et il tient ici.

Ce qu'il coûte est un **signal faux** : un `pass` / APPROVED qui *atteste*
nommément la CI et qui trompe l'humain qui lit la PR. Sur l'occurrence 1, le
signal a survécu huit minutes à la vérité, puis a continué à la contredire.

---

## Requirements

- **R1** — Sur `AllowedPending` (au moins une vérification requise `pending`,
  aucune rouge), un corps de verdict `pass` **dont une ligne atteste une CI
  conclue au vert** est refusé avant le sous-processus.
- **R2** — Le corps du refus est auto-suffisant : il nomme les vérifications
  requises encore `pending` et **cite la ligne** qui atteste.
- **R3** — Un corps `pass` qui **n'affirme rien** sur l'état de la CI reste
  laissé passer sur `AllowedPending`. R6/AC3 de mika#2455 est préservé pour sa
  population d'origine : la revue précoce est nominale et ne devient pas
  impossible.
- **R4** — Le remède nomme **deux** sorties atteignables, et n'en prescrit
  aucune : (a) garder `pass` et remplacer l'attestation par la ligne canonique
  qui dit la CI en cours ; (b) réécrire en `hold[review]`. Le gate n'a aucun
  moyen de savoir si la CI finira verte — l'occurrence 1 a fini rouge, les trois
  autres vertes — donc il ne choisit pas.
- **R5** — La ligne canonique que le remède prescrit ne déclenche **jamais** le
  détecteur : sans quoi la réécriture est refusée à son tour et le tour meurt
  sans revue (le mode de panne du point 2 de mika#2455).
- **R6** — Chaque décision est **dite** : nouvel outcome d'audit
  `refused_pending_attested`, en **ajout de queue** au vocabulaire existant
  (`allowed_green`, `allowed_pending`, `refused`, `abstained`). Aucun renommage :
  les `GROUP BY` publiés sur `audit_events` restent exacts (doctrine mika#2131,
  patron de B3/mika#2565).
- **R7** — Le détecteur décide sur la **forme** de l'attestation, pas sur la
  liste des deux phrases mesurées. « check_suite success » et « CI green » sont
  deux instances ; la régression réaliste est une troisième formulation, pas une
  recopie des deux premières (doctrine mika#1804).
- **R8** — L'asymétrie est **calculée**, pas ressentie, et elle penche du même
  côté que mika#2455 : un faux négatif laisse le signal trompeur — le régime
  d'aujourd'hui, et le merge reste fermé ; un faux positif rend une revue
  légitime impossible à poster. Donc toute ligne **ambiguë** est écartée du
  détecteur.
- **R9** — Le désarmement reste le seul existant, `MIKA_QA_CI_COHERENCE_GATE`.
  Aucune nouvelle variable : ce terme est une branche du gate mika#2455, pas un
  second gate.
- **R10** — Le prompt de `qa-review` dit la règle d'abstention que le modèle
  **peut** appliquer sans signal : n'affirme rien sur l'état de la CI. Elle
  complète l'interdiction de lecture déjà en place (ligne 46) ; elle ne **ferme**
  pas le défaut — la garde le ferme (point 4 de mika#2455 : une règle de prompt
  qu'un modèle ne peut pas appliquer faute de signal ne tient rien ; celle-ci est
  une abstention, donc applicable).

---

## Design

### D1 — Le discriminant est l'attestation du corps, jamais le temps

L'alternative évidente — refuser tout `pass` tant que les vérifications requises
n'ont pas conclu — est chiffrée et **écartée** au § *Décision de conception
écartée* ci-dessous. Elle renverse R6/AC3 de mika#2455 pour toute la flotte.

Le ticket accorde explicitement les deux sorties par un « ou » : « un `pass` qui
atteste la CI n'est rendu qu'une fois toutes les vérifications requises terminées
**ou il dit explicitement que la CI est en cours** ». La seconde clause nomme la
population : le `pass` **qui atteste**. Le `pass` muet est hors population, et il
reste permis.

### D2 — `AllowedPending` porte ses noms, par symétrie avec `Refused`

```rust
pub(crate) enum CiCoherenceOutcome {
    Refused { failing: Vec<String> },
    AllowedGreen,
    AllowedPending { pending: Vec<String> },   // ← nouveau champ
}
```

L'extraction suit `classify_checks` et ne **redéfinit rien** : filtre
`bucket == "pending"`, le prédicat exact de `classify_checks`
(`pr_merge_with_gate.rs:917`), exactement comme `Refused` filtre `fail`/`cancel`.
Une seconde définition de « requis » ou de « pendant » est la classe que
`grooming_marker` (mika#2158) a dû fermer après des mois de divergence
silencieuse. D6/R10 de mika#2455 tient inchangé.

Les cinq sites de `match` existants (quatre dans les tests inline de `guards.rs`,
un dans `builtin_handlers.rs`) prennent `AllowedPending { .. }`.

### D3 — Le détecteur : une grammaire en deux listes et une exclusion

`evidence::guards::body_attests_ci_concluded(body: &str) -> Option<String>`,
voisine de `body_asserts_api_surface`, pure, rend la **ligne** qui atteste
(trimée, tronquée sur une frontière de **caractère** — `chars().take(n)`, jamais
un offset d'octet : `scripts/check-byte-slices.sh` le refuse, mika#764/mika#2103).

Une ligne atteste quand, casse repliée, elle porte **à la fois** un sujet-CI et un
terme de conclusion verte, **et** qu'aucune exclusion ne la désarme.

- **`CI_SUBJECT_WORDS`** — mots entiers : `ci`, `checks`, `check_suite`,
  `check_suites`. Le mot est une suite maximale de caractères alphanumériques
  Unicode ou `_` : c'est ce qui fait que `check_suite` est **un** mot (une
  tokenisation sur non-alphanumérique le couperait en deux) et que `ci` ne se
  trouve pas dans `précis`, `décision`, `spécifique`.
- **`CI_SUBJECT_PHRASES`** — sous-chaînes : `check suite`, `required checks`,
  `vérifications requises`, `status check`.
- **`CI_GREEN_WORDS`** — mots entiers : `green`, `success`, `successful`,
  `passed`, `vert`, `verte`, `vertes`, `concluded`, `conclu`, `conclue`,
  `conclues`.

Trois absences volontaires de `CI_GREEN_WORDS`, chacune parce qu'elle produit un
faux positif sur un corps de verdict **nominal** (gabarits de
`skills/bundled/qa-review/system_prompt.md`, lignes 700-800) :

- **`pass`** (sans `-ed`) — `VERDICT: pass`, `PIPELINE: pass (mika)`,
  `Build: pass`, `DEP-REVIEW`. Le participe `passed` reste, lui : « all required
  checks passed » est bien une attestation.
- **`✅`** — la ligne `- [✅] satisfied: CI job added` porterait `ci` + `✅` et
  n'atteste rien. La règle est sur des **mots**, pas sur un pictogramme de
  balisage.
- **`pipeline`** n'est pas un sujet-CI — `verify-pipeline.sh`, `PIPELINE:`,
  `block[pipeline]` sont du vocabulaire local, pas la CI de la forge.

### D4 — Les deux exclusions, et de quel côté elles échouent

**(i) Négateurs et termes de pendance** — si la ligne porte l'un de ces mots
entiers (`pending`, `queued`, `running`, `still`, `encore`, `not`, `no`, `ne`,
`ni`, `pas`, `unknown`, `unconfirmed`, `in_progress`) ou l'une de ces phrases
(`en cours`, `in progress`, `not yet`, `pas encore`, `n'a pas`, `n'est pas`,
`cannot`, `could not`, `do not`), elle est **écartée**.

C'est délibérément sur-inclusif. « CI green, no findings » porte `no` et sera
écartée : faux négatif. R8 dit pourquoi c'est le bon côté — un faux négatif rend
le régime d'aujourd'hui, un faux positif tue un tour de revue. Les deux phrases
mesurées ne portent aucun négateur, donc le défaut reste attrapé.

**(ii) Préfixes et jetons de vocabulaire** — `CI_ATTESTATION_LINE_EXEMPT`, trois
entrées grep-visibles, cherchées en sous-chaîne sur la ligne repliée :
`ci-deferred` (`- [⏭️] CI-deferred: …`, vocabulaire canonique de la section
`PLAN-AC VERIFICATION`, présent trois fois dans le prompt), `block[ci]` (un jeton
de verdict, pas une affirmation), `ci-status:` (la ligne canonique que le remède
prescrit — D5).

`ci-status:` est redondant avec (i), qui l'écarterait déjà par `pending` / `en
cours`. La redondance est **voulue** : R5 est la condition pour que le remède ne
soit pas une impasse, et un remède qui ne tiendrait que par une liste de
négateurs tiendrait par accident.

### D5 — Le remède, en deux sorties

Warn `qa_ci_coherence_refused_pending_attested`, audit
`refused_pending_attested` avec les noms pendants en `reasoning`, puis
`ToolOutput::error` sur la forme de `qa_ci_coherence_violation` :

```
error:          "qa_ci_coherence_pending_attested"
doctrine:       "mika#3137"
target:         pr_review:<repo>#<n>
pending_checks: [Check, Dashboard, Security]
attested_line:  "<la ligne citée>"
remedy:         ce corps rend `pass` et affirme une CI conclue au vert, alors que
                N vérification(s) requise(s) tournent encore sur cette tête :
                <liste>. Deux sorties : (a) garder `pass` et remplacer cette
                ligne par `CI-Status: pending at review time — <liste>`, qui dit
                l'état sans l'attester ; (b) réécrire le verdict en
                `hold[review]`. Rejouer l'appel avec `--comment`. Ne garder ni
                l'attestation ni le silence sur la correction.
```

**Aucune ligne de ce corps ne commence par `VERDICT:`** — `VERDICT_RE` est ancrée
sur ce préfixe de ligne et une ligne parasite serait lue comme un verdict. Les
jetons sont nommés en clair **dans une phrase**, ce qui est sûr : ni
`validate_tool_arg_suffixes` ni `parse_verdict` ne capture un jeton nu hors
préfixe de ligne. Contrainte déjà écrite pour mika#2455 ; elle ne se généralise
pas, elle tient parce que les deux lecteurs ont été lus.

Le refus **ne prescrit pas** `block[ci]` : ce jeton dispatche un pilote CI-fix
borné à trois tentatives (`verdict_handler::handle_block_ci`), et une CI qui n'a
pas encore conclu n'a rien à réparer. Même arbitrage que le point 3 de mika#2455.

### D6 — Le prompt

Deux ajouts dans `skills/bundled/qa-review/system_prompt.md`, au § Data-Integrity
où vit déjà l'interdiction de lecture (ligne 46) :

1. la règle d'abstention : ton corps de verdict n'affirme **rien** sur l'état de
   la CI — ni « CI green », ni « check_suite success », ni « vérifications
   requises vertes ». Tu ne la lis pas, donc tu ne l'attestes pas. Le mot
   « success » d'un événement `check_suite` qui t'a déclenché atteste **une**
   suite, jamais l'ensemble des vérifications requises (jusqu'à huit par push,
   mika#1869).
2. la ligne canonique, quand l'état doit être dit :
   `CI-Status: pending at review time — <checks>`.

Le prompt change pour la seule raison qui tient : il est le canal par lequel le
modèle évite le refus **avant** de le payer. La fermeture reste la garde.

---

## Périmètre — ce vol et ce qu'il laisse

**Dans ce vol** : la branche `AllowedPending` du gate mika#2455, son détecteur,
son refus, son audit, et la règle d'abstention du prompt.

**Hors de ce vol, nommé :**

- **#3135** (une seule étape d'un job rouge rejouée, flake conclu sur un rouge
  reproductible, tête citée absente de la PR). Autre population — un `hold`, pas
  un `pass` — et autre mécanisme. Le ticket le nomme en « voir aussi », pas en
  dépendance.
- **Le déclenchement de la revue sur le premier `check_suite` vert.** Le vrai
  rendez-vous serait « toutes les suites requises ont conclu », et le poser
  retarderait la revue de toute la boucle autonome de ~15 min par PR (mesure du
  ticket : 15 min 49 s sur #3139, 15 min 26 s sur #3143). La revue précoce est
  **nominale** (R6/AC3 de mika#2455, et `opened` est un déclencheur déclaré de
  `REVIEW_TRIGGERING_ACTIONS`) : ce vol la laisse précoce et lui retire le droit
  de mentir. Un changement de rendez-vous est un ticket avec son compte.
- **Le suivi des clôtures de bloc de code (```) dans le détecteur.** Un corps de
  revue peut citer une sortie dans un bloc, et une citation n'est pas une
  attestation. Écarté pour ce vol sur un fait, pas sur un budget : la QA n'a
  **aucun** accès CI, donc aucune sortie CI à citer — les blocs qu'elle cite sont
  `verify-pipeline.sh`, `cargo`, le diff. Si une mesure montre un faux positif par
  citation, c'est un ticket avec son compte, pas un élargissement au jugé. Coût
  si repris : ~35 lignes.

### Décision de conception écartée, et son coût

**Refuser tout `pass` tant que les vérifications requises n'ont pas conclu** (la
lecture par la première clause seule du ticket). Écarté, et chiffré :

- Il renverse R6/AC3 de mika#2455 pour **toute** la flotte, pas pour la
  population du ticket.
- La QA reviewe à l'ouverture de la PR, 61-84 s après, et les vérifications
  concluent ~15 min plus tard : **tout** `pass` de revue précoce deviendrait
  impossible. Le verdict atteignable serait `hold[review]`, qui notifie
  l'opérateur, sur chaque PR de la boucle.
- Il change la cadence de la boucle autonome (le merge n'entre plus sur le
  verdict mais sur une seconde revue après CI), ce qui est un arbitrage d'une
  portée très supérieure au ticket.
- Coût estimé : ~30 lignes de code, et une renégociation de mika#2455.

Il n'est **pas pratiqué** ici. La seconde clause du ticket autorise la voie
retenue, donc ce n'est pas une réserve d'opérateur au sens de mika#3137 → si
l'opérateur veut malgré tout cette sémantique, elle s'obtient par un ticket qui
amende mika#2455 R6, pas par un élargissement de ce détecteur.

---

## Verification Contract

| ce qui est vérifié | comment | où |
|---|---|---|
| les deux phrases mesurées sont attrapées | `body_attests_ci_concluded("… check_suite success …")` et `("CI green")` rendent `Some` | test inline `guards.rs` |
| un corps muet sur la CI n'est pas attrapé | gabarit `pass` nominal du prompt → `None` | test inline |
| la ligne canonique du remède n'est pas attrapée (R5) | `CI-Status: pending at review time — Check, Security` → `None` | test inline |
| `CI-deferred` / `block[ci]` ne sont pas attrapés | les trois lignes du prompt → `None` | test inline |
| négateurs : `ne … pas`, `not yet`, `pending` écartent | 3 cas → `None` | test inline |
| `ci` ne se trouve pas dans un mot | « décision précise sur un point spécifique : succès » → `None` | test inline |
| `check_suite` est **un** mot | tokenisation, pas de coupe sur `_` | test inline |
| la troncature de la ligne citée ne coupe pas un caractère | ligne UTF-8 multi-octets > plafond | test inline |
| le corpus du dépôt ne déclenche rien (Fire-Disposition) | prompt `qa-review` lu depuis `BUNDLED_SKILL_MANIFESTS`, ligne par ligne → zéro détection hors allowlist | test inline |
| `AllowedPending { pending }` nomme les bons checks | `classify_ci_coherence` sur une liste mixte | test inline |
| **câblage** : `pass` + attestation + 1 requis pendant → refus, audit `refused_pending_attested`, revue **non** postée | `validate_qa_ci_coherence_with_reader` + lecteur injecté, `gate_outcomes` / `gate_reasonings` | `tests/eval/test_qa_ci_coherence_2455.rs` |
| **complémentaire** : `pass` muet + 1 requis pendant → laissé passer, audit `allowed_pending` | idem | idem |
| aucune régression sur les trois autres outcomes | suite mika#2455 existante inchangée | `cargo test -p mika-agent` |
| structure des bundles | `make verify-bundled-skills` | CI |
| pipeline | `scripts/verify-pipeline.sh origin/main`, `cargo fmt --check`, `cargo clippy -D warnings` | local + CI |

---

## Definition of Done

1. `body_attests_ci_concluded` livrée, pure, testée, et `AllowedPending` porte
   ses noms pendants.
2. La branche `AllowedPending` de `validate_qa_ci_coherence_with_reader` décide
   sur l'attestation, refuse avec les deux sorties nommées, et écrit
   `refused_pending_attested`.
3. Le vocabulaire d'audit est **étendu** et non renommé ; les quatre outcomes
   existants gardent leur orthographe.
4. Deux tests de câblage eval (refus attesté, muet laissé passer) verts.
5. Le prompt `qa-review` porte la règle d'abstention et la ligne canonique.
6. Section `## Fire-Disposition` du présent plan appliquée.
7. `cargo test -p mika-agent`, `cargo fmt --check`, `cargo clippy -D warnings`,
   `make verify-bundled-skills` et `scripts/verify-pipeline.sh origin/main`
   verts.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de son « Attendu » et du Verification Contract.

- **AC1** — Sur une tête dont au moins une vérification requise est `pending` et
  aucune rouge, un appel `run_gh pr review` dont le corps rend `VERDICT: pass`
  **et** dont une ligne atteste une CI conclue au vert est refusé avant le
  sous-processus : aucune revue n'atteint GitHub.
- **AC2** — Le corps du refus nomme chaque vérification requise encore `pending`
  et cite la ligne qui atteste.
- **AC3** — Le refus nomme **deux** sorties atteignables — remplacer
  l'attestation par `CI-Status: pending at review time — <checks>` en gardant
  `pass`, ou réécrire en `hold[review]` — et n'en prescrit aucune. Il ne nomme pas
  `block[ci]`.
- **AC4** — Un corps `pass` qui n'affirme rien sur l'état de la CI, sur la même
  tête et le même état de checks, est **laissé passer** et audité
  `allowed_pending`.
- **AC5** — La ligne canonique `CI-Status: pending at review time — <checks>` ne
  déclenche pas le détecteur : la réécriture prescrite par AC3 est postable.
- **AC6** — Les trois lignes de vocabulaire du prompt `qa-review` (`[⏭️]
  CI-deferred: …`, `block[ci]`, `PIPELINE: pass (mika)`) ne déclenchent pas le
  détecteur, et aucune ligne du prompt `qa-review` lu depuis
  `BUNDLED_SKILL_MANIFESTS` ne le déclenche hors allowlist.
- **AC7** — Les deux formulations mesurées du ticket — « check_suite success » et
  « CI green » — déclenchent le détecteur, et une ligne portant un négateur ou un
  terme de pendance ne le déclenche pas.
- **AC8** — L'outcome d'audit `refused_pending_attested` est un **ajout** :
  `allowed_green`, `allowed_pending`, `refused` et `abstained` gardent leur
  orthographe exacte, épinglée par le test de format de fil existant.
- **AC9** — `MIKA_QA_CI_COHERENCE_GATE=0` désarme ce terme avec le reste du gate ;
  aucune nouvelle variable d'environnement n'est introduite.
- **AC10** — Le prompt `qa-review` porte la règle d'abstention CI et la ligne
  canonique, et `make verify-bundled-skills` reste vert.

---

## Fire-Disposition

Ce plan livre un détecteur — `body_attests_ci_concluded` et la branche de refus
qui le lit : son chemin de succès est « aucune attestation trouvée ».

**Option retenue : (a) exception nommée en allowlist, détecteur armé, allowlist
vide par construction.**

La population du détecteur est faite des corps de verdict **produits à
l'exécution** : il n'y a pas de corpus de violations dormantes dans le dépôt à
exempter. Mais il y en a un qui compte, et c'est celui qui a produit les corps
incriminés : les **gabarits de verdict** de
`skills/bundled/qa-review/system_prompt.md`. Un gabarit qui déclencherait le
détecteur ferait recopier au modèle un corps que la garde refuse — la violation
dormante exacte, et elle est grep-visible.

Implémentation :

```rust
/// Lignes du prompt `qa-review` que le détecteur attrape et qui ne sont PAS
/// des attestations. Vide : la lecture du prompt n'en a trouvé aucune.
/// Toute entrée nomme la ligne exacte et porte son ticket de suivi.
const CI_ATTESTATION_PROMPT_ALLOWLIST: &[(&str, &str)] = &[];  // (ligne, ticket)
```

Avec, dans le même test, **les deux directions** :

1. **Armement** — le `system_prompt` de `qa-review`, lu depuis
   `BUNDLED_SKILL_MANIFESTS` (compilé par `build.rs`, donc sans accès au système
   de fichiers), est passé ligne par ligne au détecteur. Toute détection hors
   allowlist **rougit** et le message nomme la ligne.
2. **Auto-nettoyage** — chaque entrée de l'allowlist doit **encore** déclencher
   le détecteur. Une exception devenue stale — ligne reformulée, détecteur
   resserré — rougit au lieu de dormir. C'est le geste qui fait expirer
   l'exception, et c'est ce qui empêche cette liste de devenir l'allowlist que
   mika#2309 condamne.

Si l'implémentation trouve une détection sur le prompt, la **ligne du prompt est
corrigée dans ce vol** (c'est de la rédaction de prompt, pas du code en
production) ; l'allowlist n'est pas la sortie de confort. Une entrée n'est
justifiée que si la ligne doit rester telle quelle pour une raison nommée.

Le détecteur atterrit **armé** : son faux positif a une sortie — réécrire le
corps, ce que D5 rend atteignable — et son faux négatif rend le régime
d'aujourd'hui, qui est le défaut du ticket. Livrer désarmé (option b) laisserait
la cinquième occurrence passer avec la garde dans l'arbre, ce que mika#2205 nomme :
une garde inerte se lit exactement comme une garde saine.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/evidence/guards.rs` — grammaire (3+2 listes), `body_attests_ci_concluded`, `AllowedPending { pending }`, constante d'audit, doc | 101 |
| `crates/mika-agent/src/evidence/guards.rs` — tests inline (grammaire, négateurs, tokenisation, troncature UTF-8, corpus du prompt + allowlist auto-nettoyante) | 65 |
| `crates/mika-agent/src/skills/builtin_handlers.rs` — branche de décision, refus, audit, doc | 50 |
| tests (`crates/mika-agent/tests/eval/test_qa_ci_coherence_2455.rs`) — 2 cas de câblage | 40 |
| `skills/bundled/qa-review/system_prompt.md` — règle d'abstention + ligne canonique | 10 |

Total estimé : 266 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 266 contre une borne de 279 : la
marge est de 13 lignes, donc la raison doit être nommée plutôt que supposée. Elle
tient sur trois faits de structure, pas sur de l'optimisme :

1. **Rien n'est construit.** Le gate (`validate_qa_ci_coherence_with_reader`), son
   lecteur injectable, son vocabulaire d'audit, son harnais de test eval avec
   lecteur simulé (`harness_with`, `gate_outcomes`, `gate_reasonings`) existent
   tous. Ce vol ajoute **une branche** à un `match` de trois bras et **une
   fonction pure** à côté de sa jumelle `body_asserts_api_surface`.
2. **Aucune migration, aucun schéma, aucune variable d'environnement.**
   `audit_events.tool_name` est du TEXT libre et l'outcome est un ajout de queue.
3. **Les deux coupes sont déjà prises** et nommées au § *Périmètre* : pas de suivi
   des clôtures de bloc de code (~35 lignes), pas de changement du rendez-vous de
   déclenchement. Sans elles, le vol serait à ~300 et au-dessus de la borne.

Si l'implémentation dépasse la borne — typiquement parce que la doc du module
s'allonge au style du fichier — la coupe à prendre en premier est la liste
`CI_SUBJECT_PHRASES` réduite aux deux termes mesurés, et la suite est nommée par
un `Tracked in:` ouvert à ce moment-là. Elle n'est **pas** prise d'avance :
R7 demande une règle de forme, et deux phrases ne font pas une forme.

---

## Ordre de commit

1. `fix(3137): le détecteur d'attestation CI, et AllowedPending porte ses noms`
   — `evidence/guards.rs` + tests inline. **Plancher mergeable** : la fonction est
   pure et testée, l'enum porte ses noms, les cinq sites de `match` compilent,
   rien n'a changé de comportement.
2. `fix(3137): un pass qui atteste la CI verte est refusé sur un requis pendant`
   — `skills/builtin_handlers.rs` + les deux cas de câblage eval. C'est le commit
   qui ferme le défaut.
3. `fix(3137): qa-review n'atteste pas un état de CI qu'elle ne lit pas`
   — `skills/bundled/qa-review/system_prompt.md`.
4. `docs(solutions): un pass ne peut pas attester ce que la garde ne lui donne pas`
   — `docs/solutions/` (hors compte de lignes), la classe en une page : un gate
   qui décide sur l'état du monde et jamais sur ce que le texte en affirme laisse
   passer l'affirmation fausse.
