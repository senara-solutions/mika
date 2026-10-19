# Vol 4 de mika#2680 — le terme de relâche du registre, et ce qu'il reste vraiment de (f)

Le vol 1 (#2705, `185362cc`) a livré le registre des tours webhook ; le vol 2
(#2747, `034698b2`) son lecteur et le protocole de mesure
(`docs/eval/mika-2680/README.md`) ; le vol 3 (#2764, `c71ea5e6`) la classe (e)
et la halte de la mesure 3. Le re-groom demande les classes restantes **(f)** et
**(g)**.

**Ce vol-ci ne livre aucune classe de filtre, et ce n'est pas une halte.** Une
lecture de source faite pour ce groom a établi deux faits que ni le corps du
ticket ni le README ne portent, et qui déplacent le travail :

1. **La prémisse de (f) est fausse sur le code servi.** Le corps écrit que la
   règle « texte intact après handler » exclut le `pr_review` réécrit par le
   `verdict_handler`. C'est l'inverse : la fenêtre de la classe (c) admet
   **délibérément** la réécriture du `verdict_handler`, et un test existant le
   dit. Les deux formes que le corps cite — `CIRCUIT BREAKER TRIPPED` et
   `verdict_classification_failed` à ligne `VERDICT:` illisible — sont donc
   **déjà écartées aujourd'hui**.
2. **Ce qui reste de (f) n'est pas une classe, c'est une ignorance.** Les tours
   mesurés le 2026-10-07 se répartissent sur quatre chemins de relâche
   distincts, dont un seul serait une décision de filtre — et le registre, tel
   que le vol 1 l'a bâti, **ne sait pas dire lequel**. Son champ `text_touched`
   confond la réécriture du `verdict_handler` avec celle d'un handler ultérieur,
   c'est-à-dire exactement les deux chemins qu'il faudrait séparer.

Le vol livre donc le terme qui manque : **pourquoi un tour webhook a eu lieu**.
Un champ `release` sur la ligne `webhook_turn_charged`, dix valeurs, une par
porte qui relâche un tour, plus le seau qui va avec dans l'agrégat du lecteur.
Il livre avec lui les assertions qui figent le fait 1 — pour que la prémisse
fausse ne soit plus re-dérivée au prix d'un tour, ce qui est arrivé **trois
fois** les 2026-10-19 00:26Z, 00:52Z et 01:03Z. **(g)** reste bloquée par la
mesure 3 et **(h)** reste posée à l'opérateur : les deux sont renvoyées, nommées,
avec leur raison propre.

---

## Décision 1 — la prémisse de (f) est fausse, et c'est démontré par le code servi

Le corps du ticket dit, classe (f) : *« La règle « texte intact après handler »
de la phase 2 exclut ce cas. »* Le vol 3 l'a repris tel quel dans son § *Décision
4*, et le README § 9 en a fait la ligne « résidu renvoyé ». Aucun des trois n'a
lu le terme.

**Ce que la fenêtre de (c) dit, en code.** `Touched::applies_to`
(`crates/mika-agent/src/server/webhook_prefilter.rs:787`) rend, pour un
`Candidate::ReviewVerdict`, le seul terme `after_verdict_handler`. Ce terme est
calculé au site d'appel (`crates/mika-agent/src/server/handlers.rs:2026`) :

```rust
after_verdict_handler: text_after_verdict.as_ref().is_none_or(|t| req.text != *t),
```

et `text_after_verdict` est posé **après** le retour du `verdict_handler`, sans
condition, pour tout événement `github` (`handlers.rs:1797`). Le terme vaut donc
« un handler a touché le texte **depuis** l'instantané pris après le
`verdict_handler` » — autrement dit, **la réécriture du `verdict_handler` est
dans la fenêtre**, et seule celle d'un handler ultérieur garde le tour.

Ce n'est pas un effet de bord : l'en-tête du module l'écrit en toutes lettres
(`webhook_prefilter.rs:30-33`) — *« Pour (c), le texte doit être intact APRÈS le
`verdict_handler` : lui a déjà dit au modèle ce qu'il avait à dire, et ce n'est
rien d'actionnable ; un handler ultérieur qui toucherait le texte garde le
tour. »* Et un test l'épingle déjà, `mika2675_c_texte_intact_apres_le_verdict_handler`
(l. 1516) :

```rust
assert!(!by_verdict_only.applies_to(&verdict));  // réécrit par le verdict_handler seul : la porte continue
assert!(after.applies_to(&verdict));             // un handler ultérieur : le tour a lieu
```

**Conséquence sur les deux formes que le corps cite.**

| forme citée par le corps | ce que `classify` en fait | ce que `decide` en fait | état réel |
|---|---|---|---|
| `CIRCUIT BREAKER TRIPPED` (`verdict_handler.rs:2374`) — le corps de revue porte une ligne `VERDICT: block[*]` | `Candidate::ReviewVerdict`, aucun des trois termes de texte | tombe sur `pr_state` → `Closed`/`Merged` ⇒ `Skip { VerdictPrClosed }` | **déjà écarté** |
| `verdict_classification_failed` à ligne `VERDICT:` présente mais valeur non reconnue (`verdict_handler.rs:2102`) | `Verdict::Missing` ⇒ `facts.unreadable = true` | `Skip { VerdictUnreadable }`, sans consulter la forge | **déjà écarté** |
| `verdict_classification_failed` **sans aucune ligne `VERDICT:`** (`verdict_handler.rs:2092`) | `verdict_raw_value(&event.body)?` rend `None` ⇒ `classify` rend `None` | jamais appelée | **hors population**, par AC2 de mika#2675 |

Les deux premières lignes supposent, comme toute classe (c), `pr_has_active_task
== Ok(false)` et une lecture de forge qui aboutit. C'est le § *Décision 2*.

**Ce que ce vol en fait.** Pas une classe : des **assertions**, plus la
correction des trois documents qui portent la prémisse fausse. Un fait établi
par lecture de source et laissé en prose se re-dérive ; les trois commentaires
de dispatch vide du 2026-10-19 sont la facture de cette propriété.

---

## Décision 2 — (f) n'a pas de résidu implémentable, et les quatre chemins ne sont pas le même refus

Sur le moteur du 2026-10-07, la phase 2 était déployée et (c) armée. Les trois
sessions `pr_review` mesurées (≈ 192 k tokens d'entrée) ont donc **franchi** une
porte armée. Quatre chemins peuvent l'expliquer, et chacun appelle une réponse
différente :

| chemin | terme qui relâche | que faudrait-il pour l'écarter ? |
|---|---|---|
| **A** — la revue ne porte aucune ligne `VERDICT:` | `classify` rend `None` | **renverser AC2 de mika#2675** |
| **B** — `pr_has_active_task == Ok(true)` | le premier terme de `decide_review_verdict` | écarter un tour alors qu'une tâche vit : un jugement, pas une construction |
| **C** — `pr_state` en erreur (jeton absent, `gh` injoignable, délai de 15 s) | `state_unreadable` | remplacer la lecture de forge par une trace durable : un autre vol, et une spéculation sans mesure |
| **D** — un handler **ultérieur** au `verdict_handler` a touché le texte | `after_verdict_handler == true` | écarter un tour dont un handler a quelque chose à dire : non |

**Seul A est une décision de filtre, et c'est une décision qu'on ne prend pas
en silence.** AC2 de la phase 2 protège la revue sans ligne `VERDICT:` pour une
raison écrite dans l'en-tête du module (l. 36-38) : *« c'est la forme d'une
consigne écrite en revue, et l'identité GitHub de l'opérateur est partagée (AC2,
« un commentaire n'est jamais filtré ») »*. Écarter ce tour, même restreint aux
PR fermées sans tâche, c'est accepter qu'une consigne d'opérateur écrite en
revue sur une PR mergée — *« ouvre un ticket de suivi pour X »* — ne soit **jamais
lue**, et la perte est silencieuse. Le protocole § 7 tranche cette forme :
*« Une classe dont le chiffre est significatif mais dont la question reste
ouverte ne s'implémente pas : elle se pose à l'opérateur. »*

**Ce qui n'est PAS le résidu, et c'est une correction utile.** Le vol 3 a renvoyé
(f) en écrivant que l'écarter *« décide que la renotification opérateur du
disjoncteur n'a pas lieu »*. Ce n'est pas vrai de ce qui reste : le cas du
disjoncteur est **déjà écarté** aujourd'hui (§ *Décision 1*), donc cette
renotification n'a déjà plus lieu, et aucune décision de ce vol ne la change. La
notification elle-même part en code, avant la porte — `send_notification` à
`verdict_handler.rs:2345`, ligne d'audit `verdict_circuit_breaker_tripped` à
l. 2328 — et le pré-digest le dit au modèle (*« Operator has been notified »*).
Ce que le tour écarté perd est la phrase de relais conversationnelle que le
pré-digest demande au modèle (*« surface the PR URL for operator triage »*), sur
une PR **mergée sans tâche**. Le renvoi « ticket à part, voisin de #2679 » reste
juste pour la renotification à chaque retraitement DECISION-CORE — qui porte sur
`check_suite`, pas sur `pr_review`. Les deux questions avaient été confondues.

**Et le chemin réel est inconnu.** Lequel de A, B, C ou D a produit les 192 k
mesurés ? Aucune lecture de source ne le dit : c'est un fait d'exécution. Le
registre du vol 1 ne le dit pas non plus — c'est le § *Décision 3*. Implémenter
pour A, B ou C sans le savoir serait optimiser un chemin qu'on n'a pas observé,
et le critère d'acceptation du ticket existe précisément pour refuser cela.

---

## Décision 3 — ce qui manque n'est pas une mesure, c'est un terme

Le README § 9 dit de (f) : *« Le résidu est donc mesurable, et non mesuré : il
attend la commande du § 3, pas une décision. »* **C'est optimiste, et le § 4 du
même document porte la raison sans en tirer la conséquence** : il prescrit de
lire (f) sur *« `text_touched: true` conjugué à un `trigger` de revue »*. Or
`text_touched` est `Touched::by_any_handler` (`emit_turn_charged`, l. 1026),
c'est-à-dire `req.text != original` — **vrai pour le chemin A comme pour le
chemin D**, et vrai aussi dès que le `verdict_handler` a réécrit sans qu'aucun
handler ultérieur n'intervienne. Le champ ne sépare pas les quatre chemins ; il
les agrège. La mesure 3, lancée demain par l'opérateur, rendrait pour (f) un
chiffre qui ne désigne aucune décision.

Le registre nomme aujourd'hui **ce qu'un tour était** — `trigger`,
`text_touched`, `candidate`, `prefilter_enabled` — et jamais **pourquoi il a eu
lieu**. Le seul renseignement de causalité qu'il porte est indirect : `candidate`
**absent** dit « hors population », donc chemin A ; `candidate` présent dit
« B, C ou D », sans trancher. Ce vol ajoute le terme direct.

**Un champ, pas une conjugaison.** `release` porte **une** valeur par tour
facturé : le nom de la porte qui l'a relâché. Pourquoi un champ plutôt que de
laisser le lecteur conjuguer les champs existants : la leçon du § 5.2 du
protocole est qu'une population lue par conjonction de deux champs est une
population qu'on compte mal un jour. L'agrégat groupe par **un** champ ou ne
groupe pas.

**Pourquoi ce vol-ci peut le livrer, quand (f) et (g) ne peuvent pas être
implémentées.** Le terme de relâche ne se déduit d'aucune mesure : il se lit
dans le flot de contrôle de `skip_turn` et de `decide`, qui est devant les yeux.
Il ne décide rien — aucun tour de plus n'est écarté, aucun de moins. Et c'est
lui qui rend la mesure 3 capable de trancher (f) au lieu de la chiffrer : une
fois servi, `charged_by_release` dira si les 192 k sont A (une question AC2), B
(une tâche vivante), C (un jeton absent) ou D (un handler qui parle) — quatre
réponses, quatre suites différentes, dont trois ne sont pas des filtres.

---

## Décision 4 — la forme du terme : dix valeurs, et un seau pour son absence

**Les dix portes qui relâchent un tour**, relevées une par une sur le code servi.
La liste est le code, pas une taxonomie inventée :

| valeur de `release` | site | ce qu'elle dit |
|---|---|---|
| `switch_disarmed` | `skip_turn`, premier `return` | `MIKA_WEBHOOK_PREFILTER` désarmé |
| `not_classified` | `skip_turn`, `classify` rend `None` | hors population du pré-filtre (chemin A de (f)) |
| `text_touched` | `skip_turn`, `touched.applies_to` | un handler a touché le texte dans la fenêtre de cette classe (chemin D) |
| `branch_task_active` | `decide_default_branch`, `Ok(true)` | (a) verte ou rouge sur `main`, une tâche vit sur la branche |
| `pr_task_active` | `decide_review_verdict` et `decide_pr_closed`, `Ok(true)` | (c) revue ou (e) `pr_closed`, une tâche vit sur la PR (chemin B) |
| `pr_open` | `decide_review_verdict`, `PrForgeState::Open` | (c) revue, la PR est ouverte côté forge |
| `no_terminal_trace` | `decide_ci_failure_no_action`, `ci_failure_no_action_class` rend `None` | (c) CI rouge de branche de PR : le handler n'a pas laissé de sortie terminale |
| `no_decision_attested` | `decide_head_decided`, aucune ligne DECISION-CORE pour cet événement | (b) cet événement n'a rien redécidé |
| `head_not_previously_decided` | `decide_head_decided`, `head_decided_and_notified` rend `None` | (b) attestation présente, mais aucune antérieure notifiée sur la même tête |
| `state_unreadable` | `state_unreadable`, tous appelants | une recherche d'état a échoué — le tour a lieu, AC3 |

Trois choix de forme, chacun avec sa raison :

- **`state_unreadable` est UN terme, pas cinq.** Le prédicat en échec est déjà
  nommé, à l'octet près, par le `warn!` de `state_unreadable` (champ `what`). Le
  dupliquer dans la valeur de relâche multiplierait la cardinalité du seau par
  cinq pour une information que le journal porte déjà sur la ligne voisine.
- **`switch_disarmed` existe bien qu'il redouble `prefilter_enabled: false`.**
  Le redoublement est le but : l'agrégat groupe par un champ unique, et une
  fenêtre partiellement désarmée doit se lire dans le même tableau que le reste
  — sinon on retombe sur la conjonction que le § 5.2 interdit.
- **`InertLabel` n'a pas de terme**, parce que `decide` ne le relâche jamais :
  la classe (d) écarte inconditionnellement. Une valeur pour un chemin
  inatteignable est une valeur qu'un lecteur cherchera un jour.

**Le seau d'absence, et son nom.** Une ligne `webhook_turn_charged` écrite par un
binaire antérieur à ce vol ne porte pas `release`. Le contrôle positif de
l'en-tête du vol 1 s'applique mot pour mot : *« un `turn_usage` présent SANS
`webhook_turn_charged` se lit « binaire en service antérieur au champ », jamais
« aucun tour webhook » »*. Donc : l'agrégat groupe les tours sans `release` sous
le nom **`absent_pre_field`**, qui dit la raison, et jamais sous `unknown` ni
`"none"` (motif mika#2331, déjà tenu par `candidate`).

**Pas de second drapeau de halte.** Le § 5 du protocole porte deux conditions,
et il serait tentant d'en ajouter une troisième pour l'époque de ce champ-ci.
Non : `absent_pre_field` est un seau **visible et nommé** dans l'agrégat, donc
une fenêtre entièrement antérieure au déploiement se lit à 100 % dans ce seau,
sans qu'un drapeau ne le répète. Un second mécanisme pour un fait que le premier
rend déjà lisible est la duplication que ce module refuse ailleurs.

**Le piège du `//` de jq, nommé parce qu'il a déjà mordu ce ticket.** Le README
§ 9 rappelle que *« le `//` de jq masquait la valeur `false` »* du champ
`text_touched` avant le vol 2. `release` est une chaîne, jamais `false` ni `0`,
donc `.release // "absent_pre_field"` est sûr ici — mais l'implémentation écrit
cette raison en commentaire au-dessus de la ligne, parce que le prochain champ
ajouté à cet agrégat ne le sera peut-être pas.

---

## Décision 5 — ce que ce vol ne touche pas, et pourquoi chaque renvoi est différent

- **La mesure 3 reste un geste opérateur.** Vérifié de première main dans ce
  groom : `ls /var/log/mika` rend `No such file or directory`. La cause est la
  décision de binds de `_pilot_log_bind_args` (mika#2165), pas un oubli, et le
  README § 2 bis la tient pour inconditionnelle. Le commentaire du 2026-10-19
  01:03Z ajoute un second mur indépendant : `scripts/measure-webhook-turn-cost`
  est refusé en amont par la permission-policy de la session dispatchée. Élargir
  l'un ne lèverait pas l'autre. **Ce vol ne la tente pas et n'en dépend pas** —
  c'est la propriété qui le distingue des trois dispatches vides.
- **(g) `check_suite success` sur branche de PR côté mika-qa** reste bloquée par
  la mesure, et rien n'a changé à cela : son déclencheur est exactement celui
  que la classe (b) traite, donc tout chiffre pris sans `skipped_by_class`
  compterait comme à filtrer ce que (b) écarte déjà. Ce vol **améliore** sa
  condition sans la lever : `charged_by_release` dira, sur la même fenêtre, si
  les `check_suite` de branche de PR facturés le sont par
  `no_decision_attested` (premier traitement de la tête — hors de (b) par
  construction) ou par `head_not_previously_decided` (ce que (b) couvrira).
  C'est exactement la soustraction que la mesure 3 doit opérer, et elle devient
  lisible sur un champ au lieu d'être reconstruite.
- **(h) `issue_comment` / `issues closed` sans tâche** reste posée à l'opérateur.
  1,30 M tokens, 79 % de la fenêtre de la mesure 2 : le volume est établi, la
  question du 2026-10-08 — que doit faire ce tour ? — est ouverte, et le
  protocole § 7 dit qu'elle se pose.
- **Le câblage CI de `make test-measure-webhook-turn-cost`** reste hors
  périmètre : `.github/workflows/`, scope `workflow` absent du jeton des
  pilotes (voisin de mika#2752).

---

## Phases

Une phase par unité mergeable-en-pensée, dans l'ordre où chacune rend la
suivante compilable.

### Phase 1 — le terme, et les dix sites qui le portent

`crates/mika-agent/src/server/webhook_prefilter.rs` :

1. `pub(crate) enum Release` — dix variantes, l'ordre du tableau du § *Décision
   4*, chacune documentée par son site d'appel. `as_str()` rend la valeur de fil,
   même discipline que `PrefilterClass::as_str`.
2. `Decision::Llm` devient `Decision::Llm(Release)`. Les dix sites de production
   reçoivent leur terme ; `state_unreadable` rend
   `Decision::Llm(Release::StateUnreadable)` et garde son `warn!` inchangé.
3. `skip_turn` ne rend plus `bool` mais `Gate` :

   ```rust
   pub(crate) enum Gate { Skipped, Charged(Release) }
   ```

   Les trois `return false` deviennent `Gate::Charged(Release::SwitchDisarmed)`,
   `Gate::Charged(Release::NotClassified)` et
   `Gate::Charged(Release::TextTouched)`. L'ordre des gardes est **inchangé** —
   en particulier le `!enabled` reste avant `classify`, donc le chemin désarmé
   n'est pas réordonné par ce vol.
4. `emit_turn_charged` prend `release: Release` et l'écrit dans ses **deux**
   bras `info!`, qui doivent rester identiques champ pour champ (la raison est
   déjà écrite l. 1028-1034).

**Non fait, et c'est une décision.** `emit_turn_charged` appelle `classify` une
seconde fois ; ce vol ne supprime pas ce second appel. Ce n'est pas la
duplication que ce module combat : c'est **une** fonction pure appelée deux fois
sur **la même** entrée, donc sans divergence possible, là où le défaut nommé
(mika#2158, mika#2362) est celui de deux lecteurs *différents* d'un même fait.
La supprimer exigerait de porter le candidat dans `Gate`, donc de déplacer la
garde `!enabled` après `classify` — une modification du chemin désarmé qu'aucun
besoin de ce vol ne justifie.

### Phase 2 — le site d'appel

`crates/mika-agent/src/server/handlers.rs:2035-2057` : le `if skipped { return }`
suivi d'un appel nu devient un `match` sur `Gate`, qui est la forme où un terme
ne peut pas être oublié :

```rust
match super::webhook_prefilter::skip_turn(/* … */).await {
    Gate::Skipped => return,
    Gate::Charged(release) => super::webhook_prefilter::emit_turn_charged(
        a.db.agent_id(), original, touched, &req.request_id, enabled, release,
    ),
}
```

Le commentaire de l. 2018-2022 — qui explique pourquoi le booléen est lié plutôt
que testé en ligne — est réécrit : sa raison d'être (faire exister la branche
« payé » comme bloc) est désormais tenue par le `match` lui-même.

### Phase 3 — le lecteur, pour qu'aucun champ ne soit écrit sans être lu

`scripts/measure-webhook-turn-cost` :

1. `release: $c.release` dans l'objet par tour, donc dans chaque ligne de
   `turns-*.jsonl`.
2. `charged_by_release` dans l'agrégat, en regard de `skipped_by_class` : groupe
   `$turns` par `(.release // "absent_pre_field")`, même forme `from_entries` que
   son voisin, avec le commentaire sur le `//` du § *Décision 4*.
3. L'en-tête `--help` du script gagne la ligne qui nomme le nouveau seau.

`scripts/test-measure-webhook-turn-cost.sh` et
`crates/mika-agent/tests/fixtures/mika2680/` :

4. Un contrôle **positif** : un corpus dont deux lignes portent `release`
   différents rend deux seaux de 1.
5. Un contrôle **négatif** : une ligne `webhook_turn_charged` **sans** `release`
   atterrit dans `absent_pre_field` et **pas** dans un seau nommé `null`.
   Chacun sur sa propre entrée, comme les six contrôles existants.

### Phase 4 — les assertions

`crates/mika-agent/src/server/webhook_prefilter.rs`, dans `mod tests`, en
réutilisant `Stub` (l. 1173, `Default` + champs `Option`, et qui **panique** sur
toute recherche non attendue) et `verdict_handler_rewrite` (l. 1976).

**A. La taxonomie des dix termes** — `mika2680_la_relache_atteint_chacun_de_ses_dix_termes` :
une table `(Candidate, Stub, Release)`, un cas par terme, et l'exhaustivité
tenue par un `match` sur la variante à l'intérieur du test plutôt que par un
comptage. Le comptage est l'idiome du voisin
`mika2680_la_taxonomie_atteint_chacun_de_ses_douze_termes` (l. 2040-2048) et il
a une faiblesse : ajouter une onzième variante sans cas le laisse **vert**. Un
`match` exhaustif rend le même service et rougit à la compilation.

**B. Les quatre assertions de (f)** —
`mika2680_f_le_pr_review_reecrit_est_deja_ecarte` :

1. Le pré-digest du disjoncteur, PR **mergée**, `pr_task: Ok(false)` ⇒
   `Decision::Skip { class: VerdictPrClosed }`, et `applies_to == false` pour
   `Touched { after_verdict_handler: false, by_any_handler: true, .. }`.
2. Le pré-digest `verdict_classification_failed` à valeur `VERDICT:` illisible ⇒
   `Decision::Skip { class: VerdictUnreadable }`, **sans** consulter la forge —
   tenu par le bouchon, qui panique si `pr_state` est lu.
3. La revue **sans** ligne `VERDICT:` ⇒ `classify` rend `None`, donc relâche
   `not_classified`. C'est le seul résidu de (f) qui soit une décision de
   filtre, et l'assertion porte en commentaire qu'elle épingle **AC2 de
   mika#2675** : la renverser est une décision opérateur, pas un refactor.
4. **Contrôle négatif, de même graine** : les mêmes trois textes avec
   `pr_task: Ok(true)` ⇒ `Decision::Llm(Release::PrTaskActive)`. L'asymétrie est
   obligatoire — une batterie purement positive est satisfaite par un filtre qui
   écarte tout (leçon mika#2277).

**C. Les douze `Decision::Llm` existants** du module reçoivent leur terme
attendu. Ce n'est pas une mise à jour mécanique : chacun devient une assertion
sur *quelle* porte a relâché, donc une couverture gagnée sans ligne nouvelle.

### Phase 5 — les documents, là où le prochain lecteur les trouvera

1. `webhook_prefilter.rs` — section d'en-tête `# Le terme de relâche (mika#2680,
   vol 4)` : les dix termes, le seau d'absence, et le fait 1 du § *Décision 1*
   écrit à l'endroit où la prémisse fausse a été lue.
2. `docs/eval/mika-2680/README.md` — § 4 gagne `charged_by_release` dans la table
   des deux moitiés ; § 9 voit la ligne **(f)** corrigée : *« couverte par (c)
   pour les deux formes citées ; résidu = une décision AC2 et trois chemins
   non-filtres »*, avec les quatre chemins du § *Décision 2* ; la ligne **(g)**
   gagne la soustraction que `charged_by_release` rend lisible.
3. `docs/solutions/best-practices/un-pre-filtre-de-webhook-se-place-apres-les-handlers-2026-10-07.md`
   — la table des classes gagne la colonne « terme de relâche », qui est la
   lecture réciproque de la table des classes.

---

## Definition of Done

1. `Release` existe avec ses dix variantes, chacune documentée par son site, et
   `as_str()` rend les dix valeurs de fil du tableau du § *Décision 4*.
2. `Decision::Llm` porte un `Release` à ses dix sites de production ;
   `state_unreadable` rend `Release::StateUnreadable` et son `warn!` continue de
   nommer le prédicat en échec (champ `what`).
3. `skip_turn` rend `Gate`, l'ordre de ses gardes est inchangé, et le site
   d'appel de `handlers.rs` est un `match` sans bras par défaut.
4. `emit_turn_charged` écrit `release` dans ses **deux** bras `info!`, champ pour
   champ identiques ; `WEBHOOK_TURN_CHARGED_EVENT` reste son seul écrivain.
5. **Aucun tour de plus ni de moins n'est écarté.** Ce vol n'ajoute aucune
   variante de `PrefilterClass`, aucun bras de `classify`, aucun terme à
   `applies_to`.
6. Le lecteur porte `release` par tour et `charged_by_release` dans l'agrégat ;
   une ligne sans le champ atterrit dans `absent_pre_field`, jamais dans `null`
   ni `unknown`.
7. Le harnais du lecteur porte un contrôle positif (deux termes, deux seaux) et
   un contrôle négatif (champ absent), chacun sur sa propre entrée.
8. La taxonomie des dix termes est exhaustive **par `match`** : ajouter une
   variante sans cas ne compile pas.
9. Les quatre assertions de (f) passent, dont le contrôle négatif de même
   graine ; l'assertion 2 prouve que la forge n'est pas consultée (le bouchon
   panique si elle l'est).
10. Suite verte : `cargo test -p mika-agent --lib webhook_prefilter`,
    `cargo test -p mika-agent --lib canonical_tokens` (dont
    `mika2517_the_fallthrough_domain_has_a_single_definition`),
    `make test-measure-webhook-turn-cost`, `cargo clippy`, `cargo fmt --check`,
    et `scripts/verify-pipeline.sh origin/main`.
11. Les trois documents du § *Phase 5* portent la correction de (f) ; le corps
    de la PR nomme ce qui n'est pas livré — la mesure 3, (g), (h), le câblage CI
    — et porte la commande de la mesure 3 prête à coller.

## Acceptance criteria

Le corps de mika#2680 ne porte pas de section `## Acceptance criteria` ; ceux-ci
sont dérivés des exigences ci-dessus et du § 7 du protocole de mesure.

**AC1 — le registre dit pourquoi.** Pour tout tour webhook facturé, la ligne
`webhook_turn_charged` porte un champ `release` dont la valeur est l'une des dix
du tableau du § *Décision 4*, et c'est celle de la porte qui a effectivement
relâché le tour.

**AC2 — rien d'autre ne change.** La population des tours écartés est
inchangée : aucun ajout à `PrefilterClass`, aucun bras à `classify`, aucun terme
à `applies_to`. Un test épingle que les classes existantes rendent exactement
les mêmes décisions qu'avant le vol.

**AC3 — le champ est lu.** `scripts/measure-webhook-turn-cost` rend
`charged_by_release` dans son agrégat et `release` sur chaque ligne de
`turns-*.jsonl`. Un corpus sans le champ ne produit aucun seau nommé `null`.

**AC4 — l'absence est nommée, pas devinée.** Une ligne `webhook_turn_charged`
antérieure à ce vol est comptée dans `absent_pre_field`. Le seau existe même
vide, et son nom dit la raison de l'absence.

**AC5 — (f) est tranchée par assertion, plus par prose.** Les deux formes que le
corps du ticket cite (`CIRCUIT BREAKER TRIPPED`, `verdict_classification_failed`
à valeur illisible) sont prouvées **déjà écartées** par (c) sur une PR
fermée/mergée sans tâche active. La troisième (revue sans ligne `VERDICT:`) est
prouvée **hors population**, avec AC2 de mika#2675 nommé comme ce qu'il faudrait
renverser.

**AC6 — le contrôle négatif existe et diffère d'un seul terme.** Les mêmes
textes avec `pr_has_active_task == Ok(true)` rendent `Decision::Llm(PrTaskActive)`.

**AC7 — l'exhaustivité est auto-nettoyante.** Une onzième variante de `Release`
sans cas de test fait échouer la **compilation** du test, pas seulement son
assertion.

**AC8 — les renvois sont nommés, un par raison.** Le corps de la PR nomme la
mesure 3 (geste opérateur, deux murs indépendants), (g) (bloquée par la
soustraction de (b), que `charged_by_release` rendra lisible), (h) (question
ouverte du 2026-10-08) et le câblage CI (scope `workflow` absent). Aucun des
quatre n'est livré.

## Fire-Disposition

**Option (a) — exception nommée en allowlist, et l'allowlist est vide à la
livraison.**

Les détecteurs livrés sont les deux batteries de la phase 4 — la taxonomie des
dix termes et les quatre assertions de (f) — plus les deux contrôles du harnais
du lecteur. Leur chemin de succès est « aucune divergence trouvée » entre le
terme déclaré et la porte réellement empruntée.

Elles portent **exclusivement** sur du code créé par ce vol (`Release`, `Gate`,
le champ `release` de l'agrégat) et sur des états synthétiques construits dans
le test ou dans un corpus gelé. Il n'existe donc, par construction, **aucune
violation préexistante à excepter**, et l'ensemble d'exceptions est vide.

Vide, pas absent : c'est l'emplacement où la prochaine infraction se déposera
nommée plutôt que tolérée (mika#2323). Trois conséquences d'implémentation :

- **Elles atterrissent ARMÉES.** Aucun `#[ignore]` : les assertions sont inline
  dans `mod tests` du module, donc dans `cargo test`, donc dans le job `Check`.
  Les deux contrôles du harnais passent par `make test-measure-webhook-turn-cost`,
  qu'un opérateur lance et que **CI n'appelle pas encore** — l'étape CI est hors
  périmètre déclaré (scope `workflow`), et c'est pourquoi les faits qui décident
  quelque chose vivent dans les assertions Rust, jamais dans le seul harnais
  shell.
- **L'asymétrie est obligatoire.** Une batterie purement positive est satisfaite
  par un instrument qui nomme tout `state_unreadable` ; d'où le contrôle négatif
  de même graine de l'assertion 4, qui ne diffère que par un terme d'état
  (leçon mika#2277).
- **L'exhaustivité est tenue par le compilateur, pas par un comptage.** Le
  voisin `mika2680_la_taxonomie_atteint_chacun_de_ses_douze_termes` compte ses
  termes, et un treizième ajouté sans cas le laisserait vert. Le `match` de
  l'AC7 est la correction de cette faiblesse pour la taxonomie neuve ; corriger
  celle du voisin est un renvoi, pas ce vol.

Les détecteurs **préexistants** que ce vol doit laisser verts et qui ne reçoivent
aucune exception : `mika2517_the_fallthrough_domain_has_a_single_definition`
(census des jetons canoniques), `mika2201_every_match_site_is_declared`, et les
six contrôles négatifs existants du harnais du lecteur. Vérifié pour ce groom :
aucune valeur de `PrefilterClass::as_str` ni aucun champ du registre n'apparaît
dans `scripts/canonical-tokens.tsv` — le census déclare les **sites de
correspondance** de jetons matchés, et une valeur d'émission n'en est pas un.
`Release::as_str` suit ce précédent et ne reçoit **aucune** ligne de census ; le
point 10 du *Definition of Done* le vérifie plutôt que de le supposer.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/webhook_prefilter.rs` — `Release` (dix variantes) + `as_str` + la doc de chaque site | 50 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` — `Decision::Llm(Release)` : les dix sites de production + `state_unreadable` | 25 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` — `Gate`, `skip_turn`, `emit_turn_charged` (un paramètre, deux bras) | 33 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` — section d'en-tête du vol 4 | 38 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` — tests : les douze `Decision::Llm` existants portent leur terme | 15 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` — tests : la taxonomie des dix termes, exhaustive par `match` | 80 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` — tests : les quatre assertions de (f) | 60 |
| `crates/mika-agent/src/server/handlers.rs` — le `match` au site d'appel + réécriture du commentaire | 12 |
| `scripts/measure-webhook-turn-cost` — `release` par tour + `charged_by_release` + en-tête `--help` | 15 |
| `scripts/test-measure-webhook-turn-cost.sh` + `crates/mika-agent/tests/fixtures/mika2680/` — un contrôle positif, un négatif | 35 |

Total estimé : 363 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Sous les deux bornes, et la marge est tenue par trois propriétés plutôt que par
un vœu :

1. **Aucune porte nouvelle.** Le vol ajoute un terme aux portes existantes ; il
   n'écrit ni bras de `classify`, ni variante de `PrefilterClass`, ni terme de
   `applies_to`. C'est la moitié du volume d'un vol de classe (le vol 3 a
   déclaré 282 pour une seule classe).
2. **Le gros poste est mécanique.** Les dix sites de `Decision::Llm` et les
   douze assertions existantes sont des éditions d'un seul jeton ; le relevé du
   2026-10-14 (sous-estimation de ×2,25 à ×4,1) porte sur du code **neuf**, pas
   sur de l'édition dénombrée à l'avance.
3. **Le découpage en vols est déjà fait, et nommé ci-dessous.** Ce vol ne tente
   ni la mesure 3, ni (g), ni (h) : les trois sont renvoyés, et c'est ce qui le
   distingue des trois dispatches des 2026-10-19 00:26Z, 00:52Z et 01:03Z.

## Fichiers touchés

**Code :**

- `crates/mika-agent/src/server/webhook_prefilter.rs` — `Release`, `Gate`, les
  dix sites, `emit_turn_charged`, la section d'en-tête, les deux batteries.
- `crates/mika-agent/src/server/handlers.rs` — le `match` au site d'appel.
- `scripts/measure-webhook-turn-cost` — `release` par tour, `charged_by_release`.
- `scripts/test-measure-webhook-turn-cost.sh` +
  `crates/mika-agent/tests/fixtures/mika2680/` — les deux contrôles et leur
  corpus gelé.

**Documentation :**

- `docs/eval/mika-2680/README.md` — § 4 (la table des deux moitiés), § 9 (la
  ligne (f), corrigée, et la soustraction que (g) gagne).
- `docs/solutions/best-practices/un-pre-filtre-de-webhook-se-place-apres-les-handlers-2026-10-07.md`
  — la colonne « terme de relâche ».

**Non touchés, et c'est une décision :**

- `.github/workflows/ci.yml` — hors périmètre déclaré du corps ; le jeton des
  pilotes n'a pas le scope `workflow`.
- `crates/mika-agent/src/server/verdict_handler.rs` — ce vol **lit** ses deux
  pré-digests pour les épingler en test ; il n'en change aucun. La phrase de
  relais du pré-digest du disjoncteur reste telle quelle.
- `scripts/canonical-tokens.tsv` — aucune valeur d'émission de ce module n'y
  figure ; `Release::as_str` suit le précédent (§ *Fire-Disposition*).
- `crates/mika-agent/src/server/{ci_success_handler,ci_failure_handler}.rs` — les
  termes de relâche se lisent au point de décision du pré-filtre, jamais dans
  les handlers.

## Renvois explicites

| à renvoyer | pourquoi ici et pas ailleurs |
|---|---|
| **La mesure 3** — § 3 du protocole, fenêtre ≥ 4 h 30, borne basse ≥ `2026-10-16T13:01:00Z`, avec activité PR, puis commit de `turns-*.jsonl` + `summary-*.json` | geste **opérateur**, deux murs indépendants : `/var/log/mika` absent du bac à sable (vérifié dans ce groom), et le script refusé en amont par la permission-policy (commentaire du 2026-10-19 01:03Z) |
| **La classe (g)** `check_suite success` sur branche de PR côté mika-qa | la forme est connue ; seule manque la soustraction de (b). Ce vol la rend **lisible sur un champ** (`no_decision_attested` vs `head_not_previously_decided`) sans la faire : il faut toujours une fenêtre mesurée |
| **La classe (h)** `issue_comment` / `issues closed` sans tâche — 79 % de la fenêtre de la mesure 2 | la question du 2026-10-08 est ouverte : que doit faire ce tour ? Protocole § 7 : elle se pose, elle ne s'implémente pas |
| **Le résidu de (f), chemin A** — écarter la revue **sans** ligne `VERDICT:` sur une PR fermée sans tâche | c'est le renversement d'**AC2 de mika#2675**, qui protège la consigne d'opérateur écrite en revue sur une identité GitHub partagée. La perte serait silencieuse : décision opérateur, et ce vol l'épingle en test pour qu'elle reste visible |
| **Le résidu de (f), chemin C** — lire la clôture de PR dans la trace durable d'`upstream_close_handler` plutôt que par `gh pr view` | ferait écarter (c) quand le jeton manque. Mais on ne sait pas encore si ce chemin a produit un seul des 192 k mesurés : c'est `charged_by_release` qui le dira. Optimiser avant serait la spéculation que le critère d'acceptation du ticket refuse |
| **L'exhaustivité par comptage** de `mika2680_la_taxonomie_atteint_chacun_de_ses_douze_termes` | faiblesse **préexistante** (un treizième terme sans cas la laisse verte) ; la taxonomie neuve naît avec un `match`, corriger l'ancienne est un suivi |
| **Le câblage CI de `make test-measure-webhook-turn-cost`** | hors périmètre déclaré ; `.github/workflows/`, scope `workflow` absent du jeton (voisin de mika#2752) |

Tracked in: senara-solutions/mika#2680 — les classes (g) et (h) et la mesure 3
restent sur ce ticket ; les deux résidus de (f) y sont posés comme questions
opérateur par le corps de la PR de ce vol.

Refs #2675, #2705, #2747, #2764
