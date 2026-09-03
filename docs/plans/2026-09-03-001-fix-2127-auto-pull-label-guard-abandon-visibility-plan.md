# Plan : rendre impossible le retour du label fantôme, et rouvrir la porte d'abandon qu'il a scellée (mika#2127)

**Ticket :** mika issue#2127 — `fix(auto_pull,labels): le label 'operator-review' n'existe pas — l'abandon échoue 48 fois, 'ready' n'est jamais retiré, le bassin reste bloqué`
**Labels :** `bug`, `p1-important`
**Type :** issue (bug — casseur de boucle)
**Jalon :** Substrat de boucle (#33)
**Palier de priorité :** Tier 1 — *casse la boucle*. Un ticket disjoncté garde `ready` indéfiniment et occupe une place du bassin sans qu'aucune ligne au-dessus de `debug!` ne le dise.

---

## Problème

La cause immédiate nommée par le ticket est **déjà corrigée en amont** : PR mika#2128 a déclaré `operator-review` et `blocked` dans `.github/labels.yml`, PR mika#2130 a raccourci leurs descriptions sous 100 caractères parce que la synchro de #2128 avait avorté. Les deux ont mergé le 2026-09-01. Les quatre labels sont déclarés (`:102`, `:106`, `:110`, `:114`) et présents sur le dépôt.

Ce qui reste est ce que le ticket appelait déjà « le vrai livrable », plus une conséquence que personne n'avait mesurée.

## Mesures — ce qui a été lu et interrogé, pas déduit

Toutes les lignes ci-dessous ont été produites le 2026-09-03 contre `origin/main` à `7b4ec10a` et contre la base réelle `~/.mika/data/mika.db`.

### M1 — les labels existent, des deux côtés

```
$ grep -n '^- name: \(ready\|operator-gated\|operator-review\|blocked\)$' .github/labels.yml
102:- name: ready
106:- name: operator-gated
110:- name: operator-review
114:- name: blocked

$ gh label list --repo senara-solutions/mika --json name -q '.[].name' | grep -E '^(operator-review|blocked|operator-gated|ready)$'
ready
operator-gated
blocked
operator-review
```

AC1 est satisfait en amont. Aucun changement de `labels.yml` n'est attendu de ce ticket.

### M2 — la garde existe, et elle ne couvre qu'un littéral sur quatre

`test_refusal_label_is_declared_in_labels_yml` (`crates/mika-agent/src/auto_pull.rs:2785`) fait déjà le bon geste — `include_str!("../../../.github/labels.yml")`, contrôle positif sur `ready`, contrôle négatif sur un nom inventé — mais son assertion de fond porte sur **`REFUSAL_LABEL` seul** :

```rust
assert!(declared(REFUSAL_LABEL), "the promotion gate applies `{REFUSAL_LABEL}` …");
```

`REFUSAL_LABEL` vaut `"operator-gated"` (`:206`). Les deux littéraux qui ont produit l'incident — `"operator-review"` (`:1684`) et `"blocked"` (`:1012`) — ne sont couverts par **aucune** garde. La classe de défaut est rouverte pour eux à la première suppression dans `labels.yml`.

### M3 — le doc-comment du module affirme le contraire du monde

`auto_pull.rs:180-205` :

> **Not `operator-review`, and that is a measured constraint** … `operator-review` does not exist: it is absent from `gh label list` and undeclared in `.github/labels.yml` — verified 2026-09-01 … `blocked`, the module's other exclusion label, is equally absent.

Vrai le matin du 2026-09-01, faux depuis 11:22 le même jour. La prose porte une date, ce qui la rend honnête sur son moment — mais elle est lue comme un état présent, et elle justifie une décision (`REFUSAL_LABEL = operator-gated`) par une prémisse qui n'est plus.

### M4 — le doc-comment de `abandon_stuck_ready` décrit l'ordre inverse de son code

`:1660-1666` :

> Order matters: `ready` comes off first because that is what actually stops the loop; everything after it is visibility layered on top of an arrest already secured.

Le code fait l'inverse, et son commentaire interne (`:1677-1684`) l'explique correctement : le label va **d'abord**, et son échec **avorte** l'abandon. Deux commentaires de la même fonction se contredisent ; celui du haut est celui qu'un lecteur voit en premier.

### M5 — le défaut s'est auto-scellé : la porte d'abandon est fermée en amont

C'est la mesure qui change la forme du ticket.

```
$ sqlite3 ~/.mika/data/mika.db \
  "SELECT issue_number, failure_count, redrive_count, redrive_abandoned_at FROM auto_pull_stats WHERE issue_number IN (2117,1651,1403);"
1651|3|3|
1403|3|3|
2117|3|3|
```

`redrive_abandoned_at` est **NULL** pour les trois : aucun abandon n'a jamais été enregistré.

Le chemin qui menait à l'abandon, dans `classify_stuck_ready` :

```rust
if facts.circuit_broken {                                  // :953
    return StuckReadyVerdict::Skip { reason: "circuit_breaker" };
}

if redrive_budget > 0 && facts.redrive_count >= redrive_budget {   // :960
    return StuckReadyVerdict::Abandon(AbandonReason::RedriveBudgetExhausted { … });
}
```

Le disjoncteur est testé **avant** le budget, et `Abandon` n'est atteignable que par la seconde branche. Le seuil est `CIRCUIT_BREAKER_THRESHOLD = 3` (`:58`), et le filtre 5 de la phase 2 (`:2348-2350`) pose `circuit_broken = count >= 3`.

Or le compteur qui a atteint 3 a été incrémenté **par l'échec d'application du label lui-même** — branche d'erreur de `abandon_stuck_ready`, `:1685-1692` :

```rust
if let Err(e) = gh_apply_label(github_token, issue_number, "operator-review").await {
    warn!(…);
    if let Err(e2) = db.increment_auto_pull_failure(DEFAULT_REPO, issue_number).await { … }
    return;
}
```

**Trois échecs d'abandon ont produit exactement le compteur qui rend l'abandon inatteignable.** Le raisonnement du commentaire — « the next tick simply retries the abandonment » — est correct pour les deux premiers ticks et faux pour tous les suivants : au troisième, le ticket bascule dans `Skip`, et il n'y revient jamais.

### M6 — la trace confirme l'auto-scellement, par son silence

```
lignes de log réelles « abandon could not apply operator-review » : 18
  6 le 2026-08-30 · 6 le 2026-08-31 · 6 le 2026-09-01
dernière : 2026-09-01T05:00:02Z, issue 2117
lignes auto_pull mentionnant #2117 après 2026-09-01T11:00 : 0
```

Le warn ne s'est pas arrêté parce que le défaut a été réparé — les labels ont été créés à 11:22, six heures **après** la dernière occurrence. Il s'est arrêté parce que le ticket est passé sous le disjoncteur, où le verdict est un `debug!` (`:2378`) que la configuration de log ne montre pas. Le ticket n'est plus abandonné, plus signalé, et garde `ready`.

État courant de `#2117` : OPEN, `ready`, pas de `operator-review`. Il occupe une place du bassin `pullable` et rien ne le dira plus jamais.

### M7 — `#1651` et `#1403` sont sortis par la main, pas par la boucle

```
#1651 CLOSED labels=enhancement,p2-normal,agent-core,blocked
#1403 CLOSED labels=enhancement,p2-normal,agent-core,gateway,blocked
```

Fermés en portant `blocked` — un geste opérateur, pas un abandon de la boucle (leur `redrive_abandoned_at` est NULL). Ils sont hors du rattrapage ; ils comptent comme confirmation du diagnostic, pas comme travail restant.

### M8 — le précédent de scan-de-source existe déjà dans ce module

`test_promotion_gate_never_resolves_conflicts` (`:2828`) lit son propre fichier via `include_str!("auto_pull.rs")`, le coupe à la première occurrence de `#[cfg(test)]` pour n'analyser que la moitié production, et **vérifie que la coupe a réussi** avant d'asserter :

```rust
assert!(production.contains("fn classify_promotion"),
        "production slice must actually contain the gate ({} bytes)", production.len());
```

La garde d'AC2 n'a donc pas à inventer sa technique : elle a un modèle dans le même fichier, contrôle de coupe compris.

## Décision

**Quatre livrables, dans cet ordre.**

1. **La garde d'AC2 devient auto-exhaustive plutôt qu'énumérative.** Un test qui liste les labels à la main ne détecte pas le littéral qu'on oubliera d'y ajouter — c'est la même classe de défaut, déplacée d'un cran. La garde extrait les noms **du source de production lui-même**, par deux voies qui se complètent :
   - les littéraux passés à `gh_apply_label(…, "x")` et `gh_remove_label(…, "x")` ;
   - les valeurs des constantes de la forme `const *_LABEL: &str = "x";`, qui captent `REFUSAL_LABEL` là où le scan de littéraux ne voit qu'un identifiant.

   L'ensemble extrait doit être inclus dans l'ensemble déclaré par `labels.yml`. La coupe production/test reprend le geste de M8, **contrôle de coupe compris**.

2. **AC3 exige deux contrôles négatifs, pas un.** Celui qui existe (un nom inventé n'est pas déclaré) prouve que le prédicat `declared()` discrimine. Il ne prouve pas que la **liste extraite** est non vide : une extraction qui rend zéro label passerait tous les asserts. Le plan ajoute donc un contrôle de non-vacuité — l'extraction doit rendre au minimum `operator-review`, `blocked` et `ready` — puis la démonstration exigée par l'AC : retirer `ready` de `labels.yml`, montrer l'échec, coller la sortie dans la PR.

3. **AC4 réutilise le geste que le module possède déjà.** `abandon_stuck_ready` reçoit `db`, `trace_id` et `session_id` (`:1667-1674`) : tout ce qu'il faut pour `log_audit_event`. Le voisin exact existe à `:1508` (`auto_pull_refusal_marker_unavailable`). L'événement d'abandon s'appelle `auto_pull_abandon_marker_unavailable`, par symétrie.

4. **AC5 n'est pas un rattrapage manuel — c'est la porte à rouvrir.** Poser `operator-review` à la main sur `#2117` réglerait un ticket et laisserait la classe intacte : à la prochaine disjonction, un autre ticket se scellera de la même façon, silencieusement. Le correctif de classe est que **`circuit_broken` ne doit pas court-circuiter un abandon déjà dû**. Un ticket dont le budget de re-drive est épuisé doit être abandonné *même* — et surtout — quand le disjoncteur a sauté, puisque c'est précisément l'échec d'abandon qui l'a fait sauter.

   Deux gestes, pas un. Le **découplage** du compteur (étape 11) empêche tout scellement futur — la branche qui scelle cesse d'être alimentée par le geste qu'elle bloque, et l'abandon redevient ré-essayable à chaque tick. Le **verdict** (étape 12) rouvre la porte pour les tickets déjà scellés, dont `#2117`. L'un sans l'autre laisserait soit l'état résiduel, soit la classe.

   → **Périmètre tranché** — voir §Arbitrage de périmètre. La question a été routée après l'`ESCALATE` de seconde passe et rendue : garder ici, le test étant la relation causale et non la forme du diff.

**Ce que le plan ne fait pas :** il ne réordonne pas `abandon_stuck_ready` (appliquer avant retirer). Ce choix est délibéré, documenté, et il **redevient** convergent dès lors que le label existe et que la porte est rouverte. Le hors-périmètre du ticket est respecté à la lettre.

## Phases

### Phase 1 — Constater AC1, sans le refaire (AC1)

1. Vérifier que `.github/labels.yml` déclare les quatre labels et que `gh label list` les rend. Consigner les deux sorties dans le corps de la PR. **Aucune édition de `labels.yml` n'est attendue** ; s'il en faut une, c'est une régression de #2128/#2130 à signaler avant de continuer, pas une étape à exécuter en silence.

### Phase 2 — La garde exhaustive (AC2, AC3)

2. Dans le module de test de `auto_pull.rs`, écrire `test_every_label_the_module_uses_is_declared`. Découper le source sur `#[cfg(test)]` et **asserter que la coupe a réussi** (la tranche production contient `fn abandon_stuck_ready`), sur le modèle de `:2833-2846`.
3. Extraire l'ensemble des labels employés : littéraux passés à `gh_apply_label` / `gh_remove_label`, plus valeurs des `const *_LABEL: &str = "…";`. Ajouter les littéraux du corps de `is_feeder_excluded` — ils sont comparés (`l.name == "blocked"`), pas passés en argument, donc le scan d'appels seul les manquerait.
4. **Contrôle de non-vacuité** : l'ensemble extrait doit contenir au moins `operator-review`, `blocked` et `ready`. Sans lui, une extraction cassée rendrait un test vert.
5. Asserter que chaque nom extrait est déclaré dans `labels.yml`, avec un message d'échec qui nomme le label manquant et le rappel de la conséquence (`ready` jamais retiré, bassin bloqué).
6. Conserver le contrôle négatif existant (nom inventé) — les deux prouvent des choses différentes.
7. **Démonstration AC3** : retirer temporairement `- name: ready` de `labels.yml`, lancer le test, capturer l'échec, restaurer. Coller la sortie dans la PR. Un test de cohérence qui passe quoi qu'on enlève ne vérifie rien.

### Phase 3 — L'abandon impossible devient interrogeable (AC4)

8. Dans la branche d'erreur de `abandon_stuck_ready` (`:1685`), après l'incrément du compteur, écrire un `log_audit_event` nommé `auto_pull_abandon_marker_unavailable`, portant le numéro d'issue, le `reason.slug()`, le compteur d'échec résultant, et une raison en clair disant ce qui ne s'est pas produit : `ready` reste posé, le ticket reste dans le bassin.
9. Le passer au niveau `error!` plutôt que `warn!`, ou justifier le maintien en `warn!` dans le plan. Dix-huit lignes ont crié pendant trois jours sans que quiconque agisse : le niveau n'est pas ce qui a manqué, mais l'événement d'audit ne remplace pas un niveau juste.
10. Test : un abandon dont l'application de label échoue écrit la ligne d'audit. Contrôle négatif : un abandon qui réussit n'en écrit pas.

### Phase 4 — Découpler, rouvrir, rendre visible, rattraper (AC5)

*Périmètre tranché par Mika Prime le 2026-09-03 — voir §Arbitrage de périmètre.*

11. **Découpler le compteur du disjoncteur de l'échec d'application du label — chirurgicalement.** Dans la branche d'erreur de `abandon_stuck_ready` (`:1685-1692`), cesser d'appeler `increment_auto_pull_failure`. La branche qui scelle ne doit pas être alimentée par le geste qu'elle bloque.

    **`increment_auto_pull_failure` a cinq appelants dans ce module** — `:1687` (abandon), `:2075`, `:2224`, `:2502`, `:2509` (chemins de promotion). **Seul `:1687` change.** Sur les quatre autres, le disjoncteur compte ce qu'il doit compter : des échecs de promotion, où le ticket est bien la source du problème. Les désarmer transformerait un correctif en régression silencieuse du disjoncteur. Le `git diff` doit montrer un seul site touché.

    Effet : l'abandon redevient ré-essayable à chaque tick, indéfiniment. La convergence que le commentaire de `:1682-1684` promettait devient réelle au lieu d'être théorique — et la porte ne se scelle plus jamais.

12. **Le budget épuisé l'emporte sur le disjoncteur.** Dans `classify_stuck_ready`, un ticket dont le budget de re-drive est épuisé doit être abandonné même quand le disjoncteur a sauté. L'étape 11 empêche le scellement à l'avenir ; celle-ci rouvre la porte pour les tickets **déjà** scellés, dont `#2117`. Ne pas toucher aux autres branches — `ReEntry` (`:947-951`), `SkipAndResetBudget`, `in_flight` — qui protègent des états différents.

    **L'ordre appliquer-puis-retirer de `abandon_stuck_ready` reste inchangé**, et ce n'est pas de la déférence au hors-périmètre : inverser l'ordre produirait l'état estampille-sans-label, que `classify_stuck_ready:947-951` lit comme le geste de ré-entrée de l'opérateur — `ReEntry`, reset du budget, ticket re-promu. On échangerait un ticket collé avec `ready`, visible, contre une boucle de re-promotion tous les N re-drives qui ressemble à du travail. Voir §Arbitrage de périmètre.

13. **Le verdict `Skip` cesse de vivre en `debug!`.** `:2378` et `:2381` journalisent en `debug!` un verdict qui décide du sort d'un ticket, à un niveau que la configuration de production ne montre pas. C'est ce qui a rendu `#2117` invisible pendant deux jours après que ses `warn!` se sont tus. Un `Skip { reason: "circuit_breaker" }` doit écrire un `audit_events` interrogeable — pas seulement passer le niveau de log, qui déplacerait le bruit sans créer l'action.

14. **Test de régression reproduisant M5** : `circuit_broken = true`, `redrive_count >= budget`, `abandoned = false` → verdict `Abandon`, pas `Skip`. C'est le test qui aurait vu le défaut. Plus un test qui vérifie qu'un échec d'application de label **n'incrémente plus** le compteur, et un contrôle négatif : un échec de promotion l'incrémente toujours.

15. **Rattrapage vérifié par reproduce→observe-gone, pas par geste nommé.** Le critère n'est pas « l'opérateur a posé le label à la main » mais « l'état ne se reproduit plus ». Reproduire : un ticket satisfaisant `failure_count >= 3 ∧ redrive_abandoned_at IS NULL ∧ ready posé` est-il abandonné au tick suivant ? Observer disparu : après déploiement, `#2117` porte `operator-review`, ne porte plus `ready`, et son `redrive_abandoned_at` est non-NULL. **Mesure avant/après du compte `pullable`**, les deux consignés.

16. Consigner `#1651` et `#1403` comme CLOSED-par-la-main portant `blocked` (M7) — hors rattrapage, et pourquoi.

### Phase 5 — La prose cesse de mentir (AC6)

17. Réécrire `:180-205` : garder l'incident comme **histoire datée** (« au 2026-09-01, `operator-review` n'existait pas ; 18 lignes en production ; déclaré depuis par #2128/#2130 »), retirer l'affirmation au présent, et redire pourquoi `REFUSAL_LABEL` reste `operator-gated` — parce que sa description déclarée *est* l'état qu'un refus crée, ce qui reste vrai indépendamment de l'existence de l'autre label.
18. Corriger le doc-comment de `abandon_stuck_ready` (`:1660-1666`, M4) pour qu'il décrive l'ordre réel : le marqueur d'abord, son échec avorte. Ne **pas** changer l'ordre lui-même.

## Definition of Done

- [ ] `test_every_label_the_module_uses_is_declared` existe, extrait les labels du source de production, et échoue en nommant le label manquant.
- [ ] La coupe production/test est vérifiée par une assertion, pas supposée.
- [ ] Le contrôle de non-vacuité empêche une extraction cassée de rendre un test vert.
- [ ] Le contrôle négatif « nom inventé » est conservé en plus de la démonstration AC3.
- [ ] La sortie d'échec obtenue en retirant `ready` de `labels.yml` est collée dans la PR.
- [ ] `abandon_stuck_ready` écrit `auto_pull_abandon_marker_unavailable` dans `audit_events` quand l'application du label échoue, et n'en écrit pas quand elle réussit.
- [ ] Le niveau de log de cette branche est tranché explicitement (`error!` ou `warn!` justifié).
- [ ] `auto_pull.rs:180-205` ne contient plus d'affirmation au présent que `operator-review` ou `blocked` n'existent pas.
- [ ] Le doc-comment de `abandon_stuck_ready` décrit l'ordre que son code exécute.
- [ ] `labels.yml` est **inchangé** par cette PR (hors la restauration de la démonstration AC3).
- [ ] L'ordre appliquer-puis-retirer de `abandon_stuck_ready` est inchangé.
- [ ] Chaque détecteur de test (D1-D5) halte la CI en tirant, avec le message que le tableau §Fire-Disposition exige.
- [ ] D6 remonte sans halter, et sa remontée est une ligne `audit_events` interrogeable, pas seulement un log.
- [ ] Le commentaire au-dessus de D1 nomme les formes syntaxiques que le scan reconnaît, pour que R1 soit lisible depuis le code.
- [ ] `abandon_stuck_ready` n'incrémente plus le compteur du disjoncteur sur échec d'application de label, et le `git diff` ne touche **qu'un seul** des cinq `increment_auto_pull_failure` (`:1687`).
- [ ] `classify_stuck_ready` rend `Abandon` quand le budget est épuisé même si le disjoncteur a sauté, avec le test de régression de M5.
- [ ] Les branches `ReEntry`, `SkipAndResetBudget` et `in_flight` de `classify_stuck_ready` sont inchangées.
- [ ] Le verdict `Skip` écrit un `audit_events`, et ne vit plus seulement en `debug!` (`:2378`).
- [ ] AC5 est vérifié par reproduce→observe-gone, pas par un geste manuel : `#2117` porte `operator-review`, ne porte plus `ready`, `redrive_abandoned_at` non-NULL, comptes `pullable` avant/après consignés.

## Acceptance criteria

Transcrits du corps réconcilié de mika#2127.

- [ ] **AC1** — Non-régression : `operator-review` et `blocked` déclarés **et** présents sur le dépôt, les deux sorties consignées dans la PR. Aucun changement de `labels.yml` attendu.
- [ ] **AC2** — Garde structurelle exhaustive : un test échoue si un quelconque littéral de label employé par `auto_pull.rs` n'est pas déclaré dans `labels.yml`. Couvre au minimum `operator-review`, `blocked`, `ready`, `operator-gated`.
- [ ] **AC3** — Contrôle négatif obligatoire : le test échoue si l'on retire `ready` de `labels.yml`. Démontré et consigné dans la PR.
- [ ] **AC4** — L'échec d'application du label d'abandon remonte au-delà du journal : un `audit_events` dédié, interrogeable.
- [ ] **AC5** — Rattrapage de l'état résiduel, mesuré : le chemin d'abandon ne peut pas atteindre `#2117` (M5). Livrer le découplage (étape 11) **et** le verdict (étape 12), puis vérifier par reproduce→observe-gone — pas par geste manuel nommé. Mesurer `pullable` avant/après.
- [ ] **AC6** — Le doc-comment de `REFUSAL_LABEL` cesse d'affirmer au présent une absence qui n'est plus.

## Rattachement aux critères d'acceptation

| AC | Traité par | Preuve exigée |
|---|---|---|
| AC1 | Phase 1 (étape 1) | Sorties `grep labels.yml` + `gh label list` dans la PR |
| AC2 | Phase 2 (étapes 2-6) | Test rouge sur label non déclaré, nommant le label |
| AC3 | Phase 2 (étapes 4, 7) | Sortie d'échec après retrait de `ready`, collée dans la PR ; plus le contrôle de non-vacuité |
| AC4 | Phase 3 (étapes 8-10) | Ligne `audit_events` en cas d'échec, absente en cas de succès |
| AC5 | Phase 4 (étapes 11-16) | D5 vert ; D8 vert (un seul site d'incrément touché) ; `#2117` désétiqueté avec `redrive_abandoned_at` non-NULL ; comptes `pullable` avant/après |
| AC6 | Phase 5 (étapes 15-16) | `:180-205` et `:1660-1666` relus contre le code |

## Fire-Disposition

Ce plan introduit huit détecteurs. Chacun doit dire, **avant d'exister**, ce qui se passe quand il tire — sinon un détecteur qui trouve quelque chose devient une ligne que personne ne traite, ce qui est exactement le défaut de ce ticket (dix-huit `warn!` en trois jours, aucune action). Citation : mika#1574, porte Fire-Disposition.

**Disposition retenue pour les six détecteurs de test (D1-D5, D8) : (c) halte-et-remontée.** Ils vivent dans `cargo test`, donc dans la CI, donc dans la porte de merge. Un tir arrête la PR. Aucun n'a de mode « avertir et continuer » — un détecteur de cohérence qu'on peut ignorer ne vérifie rien, ce que l'AC3 dit déjà de son côté.

| # | Détecteur | Condition de tir | Disposition | Message exigé |
|---|---|---|---|---|
| D1 | `test_every_label_the_module_uses_is_declared` | un label extrait du source de production n'est pas déclaré dans `labels.yml` | **halte** — échec de test, CI rouge | nomme **le label**, le **fichier et la ligne** d'où il a été extrait, et la conséquence (`ready` jamais retiré, ticket collé dans le bassin) |
| D2 | contrôle de non-vacuité | l'extraction rend moins que `operator-review`, `blocked`, `ready` | **halte** | dit que l'extraction est cassée, pas que les labels manquent — la confusion entre les deux est ce qui rendrait D1 vert pour la mauvaise raison |
| D3 | contrôle négatif (nom inventé) | un nom jamais déclaré est vu comme déclaré | **halte** | dit que le prédicat `declared()` ne discrimine plus |
| D4 | contrôle de coupe production/test | la tranche production ne contient pas `fn abandon_stuck_ready` | **halte** | donne la taille de la tranche obtenue, sur le modèle de `:2841-2845` |
| D5 | test de régression disjoncteur→abandon | `circuit_broken && redrive_count >= budget && !abandoned` ne rend pas `Abandon` | **halte** | nomme le verdict obtenu et rappelle M5 |
| D6 | `auto_pull_abandon_marker_unavailable` (production, pas test) | l'application du label d'abandon échoue en production | **remontée sans halte** — la boucle continue | ligne `audit_events` interrogeable + `warn!` conservé (§Arbitrage, Q3) |
| D7 | audit event du verdict `Skip` (production, pas test) | `classify_stuck_ready` écarte un ticket sous disjoncteur | **remontée sans halte** | ligne `audit_events` nommant l'issue et la raison — remplace le `debug!` de `:2378` que la config de production ne montre pas |
| D8 | non-régression du disjoncteur | le `git diff` touche un `increment_auto_pull_failure` autre que `:1687` | **halte** | nomme le site touché et rappelle que quatre appelants comptent des échecs de promotion légitimes |

**D6 et D7 sont délibérément les seuls qui ne haltent pas.** Ce sont des détecteurs d'exécution, pas de merge : arrêter `auto_pull` parce qu'un abandon a échoué punirait tous les autres tickets pour un seul. Ils remontent, ils ne bloquent pas. Mais « remonte » a désormais un sens vérifiable — une ligne d'audit qu'on peut interroger — au lieu d'un `warn!` que personne ne lit. C'est la totalité d'AC4, et D7 est ce qui aurait rendu `#2117` visible.

### Faux positifs — ce qui est traité, et ce qui reste ouvert

Le seul détecteur qui peut tirer à tort est **D1**, par sa technique de scan.

- **Faux positif** (tire alors que tout va bien) : un littéral extrait d'un contexte qui n'est pas un vrai usage de label — une chaîne dans un message d'erreur, un nom dans un doc-test. **Traitement :** l'extraction est bornée aux trois formes nommées (arguments de `gh_apply_label` / `gh_remove_label`, valeurs de `const *_LABEL: &str`, comparaisons `l.name == "…"` dans `is_feeder_excluded`) et **jamais** au fichier entier. Un tir de D1 est donc, par construction, un usage réel.
- **Faux négatif** (ne tire pas alors qu'un label manque) : un reformatage `rustfmt` casse un appel sur plusieurs lignes et le motif ne reconnaît plus l'argument. **Traitement partiel :** D2 attrape le cas pour les trois labels connus. Il n'attrape pas un quatrième label ajouté plus tard puis rendu invisible par un reformatage. **C'est le risque résiduel R1, accepté et nommé dans le code** — pas fermé. Un commentaire au-dessus de D1 doit dire quelle forme syntaxique la garde reconnaît, pour que quiconque écrit un appel sous une autre forme sache qu'il sort de sa portée.

Ce choix d'accepter R1 plutôt que de le fermer est délibéré : la fermer exigerait de passer par l'AST (`syn`) plutôt que par le texte, ce qui ajoute une dépendance de test et une complexité sans commune mesure avec la classe de défaut visée — un label appliqué et jamais déclaré. Si un faux négatif se produit réellement, il devient l'incident qui justifie l'AST ; l'anticiper aujourd'hui serait construire la garde de la garde avant d'avoir mesuré qu'elle manque.

## Arbitrage de périmètre — tranché, pas laissé ouvert (2026-09-03)

La première passe architecte a rendu `ITERATE` sur une finding (Fire-Disposition manquante, appliquée). La seconde a rendu `ESCALATE` sur une seule question : **le correctif de la porte scellée appartient-il à ce ticket ?** L'architecte a refusé de trancher un périmètre depuis son siège — à juste titre — et l'a nommé décision opérateur. La question a été routée à Mika Prime.

**Tranchage : garder dans #2127.** Le test qui départage n'est pas « est-ce de la logique de sélection du bassin » — vrai, mais c'est la forme du diff, pas la cause. Le test est la **relation causale** : la porte a été scellée par le défaut même que ce ticket traite. Ce n'est pas du travail adjacent, c'est sa conséquence directe, mesurée. La discipline « adjacent → suiveur » sépare le voisin, pas la conséquence ; ici la distance causale est nulle. Un suiveur qui hériterait de cet état naîtrait dormant sans condition de réveil autre que « la prochaine fois, en silence » — or l'invisibilité *est* le coût.

**Le coût du tranchage, nommé :** le ticket s'élargit au-delà de garde-structurelle + visibilité, vers un correctif de sélection de bassin. C'est réel. Le prix de l'inverse — AC5 réduit au geste manuel, D5 hors tableau, classe intacte — est plus lourd. Le périmètre suit la causalité, pas la taxonomie du diff.

### Une prescription reçue puis corrigée — par un fait du code

Le tranchage initial demandait aussi de **retirer `ready` avant de tenter `operator-review`**, pour que l'abandon ne dépende pas de l'application réussie du label. L'intention est juste ; le mécanisme rouvrait un défaut documenté à l'endroit exact (`:1677-1684`, confirmé dans `classify_stuck_ready:947-951`) :

> Removing `ready` first and then failing here would leave the ticket with neither label but with an abandonment stamp: Phase 1 would re-promote it, Phase 2 would read the stamp-without-label as **the operator's re-entry gesture**, and the budget would reset — a fresh loop every N re-drives.

`facts.abandoned` sans `operator-review` rend `StuckReadyVerdict::ReEntry`, qui appelle `reset_auto_pull_redrive` et remet le ticket dans les survivants. Inverser l'ordre échangerait un ticket collé avec `ready` — visible — contre une boucle de re-promotion tous les N re-drives, budget remis à zéro à chaque tour. **Le second est plus difficile à voir que le premier, parce qu'il ressemble à du travail.**

Le fait a été remonté plutôt qu'exécuté, et la prescription a été retirée : **le découplage du compteur (étape 11) atteint la même convergence sans produire l'état estampille-sans-label.** L'abandon cesse d'être définitivement bloqué par l'échec du label, ce qui était l'intention, sans inverser un ordre dont le code explique la raison. C'est une correction, pas un compromis.

L'ordre appliquer-puis-retirer reste donc inchangé — décision tranchée sur mesure, non par déférence au hors-périmètre.

### Les deux autres questions

**Q2 — scan de source contre tableau de constantes : tranché par l'orchestrateur, scan seul.** Ni l'architecte ni le bearing-keeper ne l'ont relevée ; c'est une décision d'implémentation, réversible, et j'ai une position argumentée. Le module a déjà le précédent (`:2828`), et l'AC2 dit « un littéral … qui n'est pas déclaré », pas « un membre du tableau ». Un tableau de constantes rendrait l'exhaustivité déclarative — la même classe de défaut déplacée d'un cran. R1 est le prix, assumé et nommé (§Fire-Disposition).

**Q3 — `warn!` conservé.** Dix-huit `warn!` en trois jours sans action ne disent pas « niveau trop bas », ils disent « personne ne lit ce flux ». Monter en `error!` déplacerait le bruit sans créer l'action. La réponse est l'audit event, traçable et requêtable, qui survit à la configuration de log. Le vrai trou n'était pas là : il était dans le `debug!` du verdict `Skip` (étape 13), où un ticket disparaît sans qu'aucun niveau visible ne le dise.

## Hors périmètre

Repris du ticket, sans extension :
- L'ordre des opérations dans `abandon_stuck_ready` (appliquer avant retirer) — délibéré, documenté, redevenu convergent.
- Le choix de `operator-gated` comme label de refus de la porte de promotion (mika#2123).
- mika#2121, mika#2125.

## Risques

- **R1 — Le scan de source est fragile au reformatage.** *(disposition et traitement : §Fire-Disposition → Faux positifs.)* Un `rustfmt` qui casse un appel sur plusieurs lignes peut faire échapper un littéral au regex, rendant la garde silencieusement partielle. Le contrôle de non-vacuité (étape 4) le détecte pour les trois labels connus, pas pour un quatrième ajouté plus tard. Surface résiduelle assumée et à nommer dans le code.
- **R4 — Le découplage peut déborder.** `increment_auto_pull_failure` a cinq appelants ; désarmer les quatre autres transformerait le correctif en régression silencieuse du disjoncteur sur les chemins de promotion. D8 le garde, et la DoD exige que le `git diff` montre un seul site.
- **R2 — Phase 4 touche la logique de sélection du bassin.** Un verdict `Abandon` élargi désétiquette des tickets ; si le prédicat est trop large, il retire `ready` à des tickets vivants. Le test de l'étape 12 borne le cas exact (budget épuisé **et** non déjà abandonné) ; les autres branches restent intactes.
- **R3 — AC5 se vérifie après déploiement, pas dans la PR.** Le compte `pullable` avant/après exige le binaire corrigé en production. La PR porte AC1-AC4 et AC6 ; AC5 se coche sur le ticket une fois la mesure produite. Fermer le ticket avant rejouerait l'erreur qu'il documente.

## Références

- `crates/mika-agent/src/auto_pull.rs:58` — `CIRCUIT_BREAKER_THRESHOLD = 3`
- `:180-205` — doc-comment de `REFUSAL_LABEL`, devenu faux (M3)
- `:953` / `:960` — disjoncteur testé avant le budget ; `Abandon` inatteignable (M5)
- `:1012` — `is_feeder_excluded`, littéraux `"blocked"` / `"operator-review"` comparés
- `:1660-1666` — doc-comment décrivant l'ordre inverse du code (M4)
- `:1684` / `:1685-1692` — application du label, et l'incrément qui scelle la porte
- `:1697` — retrait de `ready`, en aval
- `:2348-2350` — filtre 5 phase 2, `count >= CIRCUIT_BREAKER_THRESHOLD`
- `:2785` — garde existante, partielle (M2)
- `:2828-2846` — précédent de scan-de-source avec contrôle de coupe (M8)
- `:1508` — `log_audit_event` voisin, modèle d'AC4
- `.github/labels.yml:102,106,110,114` (M1)
- PR mika#2128, PR mika#2130 — mergées le 2026-09-01
- `auto_pull_stats` pour `#2117`, `#1651`, `#1403` (M5, M7)
