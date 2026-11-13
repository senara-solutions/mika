# mika#2680 — le protocole de lecture nomme la taxonomie servie, et un détecteur l'empêche de reprendre du retard

> Ticket : senara-solutions/mika#2680 (parent de (g′))
> Branche : `feat/2680/pr-filtre-webhook-phase-4-tours-vide-sur`
> Vol : 7

## 1. Où en est le ticket, et pourquoi ce vol-là

(g′) forme (a) est **livrée** : mika#2917 (CLOSED) a porté ses trois vols, et
mika#2927 puis mika#2948 ont bouché deux trous de son écrivain. La classe vit
dans le moteur sous `PrefilterClass::GreenCheckSuiteHeadAlreadyReviewed`
(`crates/mika-agent/src/server/webhook_prefilter.rs:352`), avec ses trois termes
de relâche. (h) est livrée désarmée par mika#2854. (e) est livrée par #2764.

Il reste à #2680 **AC4 de (g′)** — la vérification, après déploiement, que le
filtre a mordu et que `review` n'a pas baissé. Cette mesure est un geste
opérateur, et elle l'est irréductiblement : `/var/log/mika/server.log` est hors
du bac à sable `bwrap`, et `scripts/measure-webhook-turn-cost` est refusé en
amont par la permission-policy d'une session dispatchée (`no matching policy rule
-- denied by default`). Quatre dispatches de ce ticket l'ont établi (commentaires
des 2026-10-19 00:26Z, 00:52Z, 01:03Z, 04:14Z). **Ce vol ne la tente pas, et n'en
dépend pas** — c'est la propriété qui le rend livrable.

Ce vol s'attaque à ce qui rend cette mesure **faillible du côté du dépôt**, et qui
est à portée d'un pilote : le document que l'opérateur ouvre pour la faire ne
connaît pas la moitié de ce qu'il doit lire.

### Le défaut, vérifié de première main sur l'arbre servi (HEAD `feef58d45`)

1. **Le protocole ignore les classes livrées depuis.**
   `docs/eval/mika-2680/README.md` s'arrête à mika#2855 (§ 10, le vol
   d'instrument). Un `grep -n -E` sur les dix clés de fil livrées depuis — les
   deux classes `comment_triaged_note` et
   `green_check_suite_head_already_reviewed`, et les huit termes `triage_escalated`,
   `triage_unreadable`, `issue_task_active`, `triage_disarmed`,
   `closure_snapshot_absent`, `reviewed_head_absent`,
   `reviewed_head_out_of_contract`, `reviewed_head_switch_disarmed` — rend
   **zéro occurrence** (vérifié sur HEAD `feef58d45`). Le document ne nomme de
   (g′) que l'issue de skill `stop_already_reviewed_sha`, qui est du troisième
   axe (`outcome`, § 10) et non la classe du filtre. Son § 9, *« L'état des quatre classes,
   après les vols 3 à 5 »*, décrit (g) comme **« bloquée par la mesure »** et (h)
   comme **« posée à l'opérateur »** — deux états qui n'existent plus.

2. **La consigne de lecture d'AC4 vit dans un commentaire Rust que l'instrument
   n'applique pas.** Le doc-comment de la classe (g′) dit :
   *« (g′) est consultée AVANT (b) : un événement que les deux sauraient écarter
   s'inscrit désormais ici. La quantité conservée est la **somme** des deux
   classes, jamais l'une d'elles (mika#2205). »* Or `skipped_by_class` est un
   seau **plat**, groupé dynamiquement sur `.class` : l'agrégat rend
   `green_check_suite_head_decided` et `green_check_suite_head_already_reviewed`
   côte à côte, sans dire qu'ils se transfèrent. L'opérateur qui lit (b) seule
   après le déploiement de (g′) voit une baisse qui **n'est pas** une régression,
   et c'est exactement le piège que le commentaire nomme. Le piège est nommé dans
   le code ; il n'est nommé nulle part dans le document d'où la mesure se lit.

3. **Rien ne détecte cette dérive.** Le test existant
   `webhook_prefilter.rs:2481-2500` garde les quatre chaînes de fil de (g′) contre
   un renommage, et son commentaire dit pourquoi : *« ces quatre chaînes sont les
   clés de la mesure d'AC4 […] c'est le seul endroit du vol où un littéral est une
   interface »*. Il garde le **nom**. Il ne garde pas que le protocole le
   **connaisse**. Deux classes et huit termes ont donc atterri sans que rien ne
   rougisse, et c'est le mode de panne de ce document : il prend du retard en
   silence, et la mesure se lit dessus.

**La forme du défaut est connue de ce dépôt.** Deux listes qui doivent bouger
dans le même commit sont gardées par une comparaison bidirectionnelle qui fait
échouer CI : `scripts/check-dispatch-seats-declared.sh` pour
`KNOWN_DISPATCH_SEATS`, `scripts/check-pilot-turn-ceiling-labels.sh` pour
`PILOT_LABEL_TURN_CEILINGS`. Ici la paire est *(la taxonomie servie par le
moteur, la taxonomie déclarée par le protocole de lecture)*, et elle n'a pas sa
garde.

### Ce que ce vol ne fait pas

- **Il n'écarte aucun tour de plus ni de moins.** Aucune variante n'est ajoutée à
  `PrefilterClass` ni à `Release`, aucun bras à `classify` ou à `decide`, aucun
  prédicat n'est touché. Le comportement du pré-filtre est inchangé, octet pour
  octet.
- **Il n'exécute pas la mesure d'AC4** et n'en commite aucun agrégat. Verser un
  JSON collé était le geste du vol 6, et #2833 a été fermée sans merge : il n'est
  pas rejoué.
- **Il ne câble rien en CI.** `.github/workflows/` est hors du scope du jeton
  d'un pilote (hors-périmètre déclaré au vol 3 de ce ticket, voisin de
  mika#2752). Le détecteur est un test `cargo`, donc le job `Check` l'atteint
  déjà, sans toucher un workflow et sans entrée dans
  `scripts/suite-wiring.tsv` — dont la comparaison bidirectionnelle (mika#2761)
  **ferait rougir** une entrée pour une suite que la CI atteint.

## 2. Le dessin

### A. La liste des variantes se dérive de l'énumération, au lieu d'être recopiée

Le recensement existant `mika2680_la_relache_atteint_chacun_de_ses_dix_huit_termes`
(l. 4211-4245) itère sur une **liste littérale** de 18 termes. Son exhaustivité
vient d'ailleurs — du `match` sans bras `_` de `cas_de_relache` (l. 4040) — et ce
découplage a une faiblesse que le doc-comment du voisin nomme déjà pour
lui-même : *« il ne sait rien de l'énumération, donc une variante ajoutée sans cas
le laisse vert »*. Un dix-neuvième terme casse la compilation de `cas_de_relache`,
le rédacteur ajoute son bras — et la liste du recensement peut rester à 18 sans
que rien ne le dise.

Le vol ferme ce découplage par une **chaîne** : pour chacune des deux
énumérations, une fonction de test

```rust
fn suivante(terme: Release) -> Option<Release>
```

dont le `match` **n'a pas de bras `_`** et rend la variante suivante dans l'ordre
de déclaration, `None` sur la dernière. La liste se dérive alors par
`std::iter::successors`, et elle n'est plus une seconde copie : **le match EST la
liste**. Une variante ajoutée ne compile pas tant qu'elle n'a pas son bras, et
une fois le bras écrit elle est dans la liste par construction — l'oubli que la
forme actuelle autorise devient impossible.

Même fonction pour `PrefilterClass`, qui n'en a aucune aujourd'hui : son
exhaustivité n'est tenue qu'en production, par le `match` de `as_str()`
(l. 356-373), et aucun test ne sait énumérer ses quatorze variantes.

Les deux listes dérivées portent chacune une **garde de cardinalité**
(`assert_eq!(…, 14)` / `assert_eq!(…, 18)`) : pas pour tenir l'exhaustivité — la
chaîne la tient — mais pour qu'un bras qui pointerait en arrière et ferait un
cycle, ou en sauterait un, soit rendu visible plutôt que de tronquer la liste en
silence.

### B. Le protocole déclare la taxonomie qu'il doit savoir lire

`docs/eval/mika-2680/README.md` gagne une section `### Taxonomie servie`,
encadrée par deux marqueurs qui en font une **interface** et non de la prose :

```
<!-- taxonomie-servie:debut -->
| clé de fil | axe | ce qu'elle dit |
...
<!-- taxonomie-servie:fin -->
```

Deux tables : les 14 clés de `skipped_by_class`, les 18 de `charged_by_release`.
Une ligne par clé, avec son axe et une phrase. Les marqueurs sont lus par le
détecteur ; le texte autour reste libre.

Pourquoi des marqueurs et non un `grep` sur tout le fichier : le document cite
déjà des termes qui ressemblent à des clés sans en être — `absent_pre_field`,
`outcome_line_absent` sont du **lecteur**, pas du moteur, et le § 10 énumère les
huit valeurs de l'axe `outcome`, qui est un troisième axe. Un détecteur qui
scannerait le texte entier confondrait trois taxonomies et rendrait des faux
rouges sur le sens inverse.

### C. Le détecteur compare les deux, dans les deux sens

Un test dans le `mod tests` de `webhook_prefilter.rs`, qui lit le README à
l'exécution par `Path::new(env!("CARGO_MANIFEST_DIR")).parent().parent()` — le
pattern de `merge_gate_rerun.rs:1106` et de `qa_reviewed_head.rs:551`, retenu
parce que `docs/eval/` n'a pas de copie dans le crate (contrairement à
`docs/openapi/`, qu'`openapi.rs` atteint par `include_str!`) :

1. **Sens « le protocole a pris du retard »** : toute clé servie par
   `as_str()`, sur les deux énumérations, est présente dans la table de son axe.
   C'est le sens qui rougit aujourd'hui, pour dix clés.
2. **Sens « la table a gardé un terme mort »** : toute clé de la table est servie
   par l'énumération de son axe. C'est l'assertion auto-nettoyante : une entrée
   ne peut pas devenir stale sans être vue.
3. **Le fichier absent ou les marqueurs absents sont un échec**, jamais un saut.
   Un détecteur qui saute proprement est un détecteur que rien ne lance
   (mika#2578, mika#2752), et ce document a déjà sa doctrine là-dessus : *« un
   scan inerte en silence se lit [comme un scan qui n'a rien trouvé] »*
   (`README.md:232`, classe mika#2205).
4. **Un contrôle négatif de même graine** : la même comparaison, jouée sur une
   table amputée d'une clé, échoue — et une seconde fois sur une table
   portant une clé que le Rust ne sert pas. Sans eux, un test dont l'extraction
   rendrait l'ensemble vide serait vert des deux côtés.

Le message d'échec nomme **la clé et son axe**, et dit quoi faire : déclarer la
clé dans `### Taxonomie servie`, ou retirer l'entrée morte.

### D. Le § 11 dit comment AC4 de (g′) se lit

Une section nouvelle, qui porte les trois choses qu'un opérateur doit savoir
avant de conclure sur AC4 et qu'aucune page ne dit aujourd'hui :

- **(b) et (g′) se somment.** (g′) est consultée avant (b), donc un événement que
  les deux sauraient écarter s'inscrit désormais sous (g′). La quantité conservée
  est `green_check_suite_head_decided + green_check_suite_head_already_reviewed` ;
  lire (b) seule fait voir une régression là où il y a un transfert de population.
  La doctrine est celle du commentaire Rust (mika#2205) ; le vol la porte à
  l'endroit où la mesure se lit.
- **Les trois termes dont le régime attendu est zéro**, et ce que leur non-zéro
  nomme : `reviewed_head_out_of_contract` (l'écrivain écrit hors contrat — le
  défaut est dans le prompt du skill, pas dans SQLite),
  `reviewed_head_switch_disarmed` (la classe est désarmée — un non-zéro dit un
  oubli d'armement, pas un registre muet), `closure_snapshot_absent` (le site de
  capture de (h) n'a pas tourné).
- **Le terme dont l'effondrement à zéro serait le symptôme** :
  `reviewed_head_absent` est le terme **nominal** — la tête neuve d'un premier
  push — et c'est le contrôle négatif d'AC1 de (g′). Sa présence est saine ; sa
  disparition dirait que le filtre mord une population qu'il ne devrait pas
  voir.

### E. Le § 9 est remis à l'état réel

Les quatre lignes de son tableau sont réécrites sur l'arbre servi : (e)
implémentée (inchangé), (f) couverte par (c) avec son résidu (inchangé), (g)
**livrée en forme (a) par mika#2917, classe `green_check_suite_head_already_reviewed`,
et ce qui reste est AC4 — une mesure, pas un code**, (h) **livrée désarmée par
mika#2854, classe `comment_triaged_note`, `MIKA_WEBHOOK_COMMENT_TRIAGE` à `off`
par défaut**. Les paragraphes qui suivent le tableau et qui parlent de (g)
« bloquée » et de (h) « posée à l'opérateur » sont corrigés au même endroit : une
seule des deux moitiés du document mise à jour serait pire que les deux
périmées.

## 3. Fichiers touchés

| fichier | nature |
|---|---|
| `crates/mika-agent/src/server/webhook_prefilter.rs` | `mod tests` : deux `suivante`, deux listes dérivées, le détecteur et ses deux contrôles négatifs ; migration du recensement existant vers la liste dérivée |
| `docs/eval/mika-2680/README.md` | `### Taxonomie servie` (32 entrées, deux tables entre marqueurs), § 11 (lecture d'AC4), § 9 remis à l'état réel |

Aucun fichier de production n'est modifié. Aucun `skills/`, aucun `scripts/`,
aucun `.github/`.

## 4. Ordre de commit

1. **Rouge** — les deux `suivante`, les deux listes dérivées, le détecteur et ses
   contrôles négatifs. Le détecteur **échoue**, en nommant les dix clés que le
   protocole ne déclare pas. C'est la preuve que le vol répare un défaut réel et
   non un défaut supposé ; elle se rend dans le corps de PR, clés nommées.
2. **Vert** — `### Taxonomie servie`, § 11, § 9. Le détecteur passe au vert par
   la déclaration, jamais par un assouplissement de l'assertion.
3. **Migration** — le recensement existant consomme la liste dérivée ; sa liste
   littérale de 18 disparaît. Suite verte.

Le plancher : si le rouge de l'étape 1 ne nomme pas de clé manquante, le défaut
n'est pas celui que ce plan décrit, et l'étape 2 ne s'écrit pas — le vol s'arrête
et le rend dans son corps de PR.

## 5. Definition of Done

1. `suivante(PrefilterClass) -> Option<PrefilterClass>` et
   `suivante(Release) -> Option<Release>` existent dans le `mod tests`, chacune
   avec un `match` **sans bras `_`**. Une quinzième classe ou un dix-neuvième
   terme **ne compile pas**.
2. Les deux listes sont dérivées par `successors` depuis la première variante, et
   chacune porte sa garde de cardinalité (14, 18).
3. Le détecteur lit `docs/eval/mika-2680/README.md` à l'exécution depuis
   `CARGO_MANIFEST_DIR`, extrait les deux tables entre marqueurs, et assère les
   **deux** sens : aucune clé servie absente de sa table, aucune entrée de table
   non servie.
4. Fichier absent, marqueurs absents, ou table vide ⇒ **échec**, avec un message
   qui distingue les trois.
5. Deux contrôles négatifs de même graine : table amputée d'une clé ⇒ échec ;
   table portant une clé inconnue ⇒ échec. Chacun sur sa propre entrée.
6. Le message d'échec nomme la clé, son axe, et le geste de réparation.
7. `### Taxonomie servie` déclare les 14 clés de classe et les 18 de relâche,
   chacune avec son axe et une phrase. Les marqueurs
   `<!-- taxonomie-servie:debut -->` / `<!-- taxonomie-servie:fin -->` encadrent
   chaque table.
8. Le § 11 porte les trois lectures d'AC4 : la somme (b)+(g′) et le transfert de
   population ; les trois termes à zéro attendu avec ce que leur non-zéro nomme ;
   `reviewed_head_absent` comme terme nominal dont l'effondrement serait le
   symptôme.
9. Le § 9 est à l'état réel pour ses quatre lignes **et** pour les paragraphes
   qui le suivent — (g) livrée par mika#2917 avec AC4 en reste, (h) livrée
   désarmée par mika#2854.
10. Le recensement `mika2680_la_relache_atteint_chacun_de_ses_dix_huit_termes`
    consomme la liste dérivée ; aucune liste littérale de variantes ne subsiste
    dans le module.
11. Aucune variante ajoutée ni retirée à `PrefilterClass` ou à `Release` ; aucun
    bras ajouté à `classify`, `decide` ou `skip_turn`. Le diff de production est
    **vide**.
12. `cargo test -p mika-agent --lib webhook_prefilter` vert ;
    `cargo clippy --all-targets -- -D warnings` propre ; `cargo fmt --check`
    propre ; `make verify-bundled-skills` et les lints du dépôt verts.
13. Le corps de PR porte le rouge de l'étape 1 — les clés que le détecteur a
    nommées — et rappelle que la mesure d'AC4 reste un geste opérateur, hors de
    ce vol.

## 6. Acceptance criteria

#2680 n'a pas de section `## Acceptance criteria` dans son corps : les critères
ci-dessous sont dérivés du § 2 (le dessin) et du § 5 (DoD), et chacun est
testable sur l'arbre.

- **AC1 — le retard du protocole est détecté.** Retirer une clé de
  `### Taxonomie servie` fait échouer la suite, et le message nomme cette clé et
  son axe. Asserté par un contrôle négatif, sur sa propre entrée.
- **AC2 — un terme mort est détecté.** Ajouter à la table une clé que le Rust ne
  sert pas fait échouer la suite. Asserté par un second contrôle négatif, de même
  graine que le premier.
- **AC3 — une variante nouvelle ne passe pas en silence.** Une quinzième classe
  ou un dix-neuvième terme **ne compile pas** tant que son bras de `suivante`
  n'est pas écrit ; une fois le bras écrit, la variante est dans la liste par
  construction, donc elle est soumise à AC1.
- **AC4 — l'absence n'est pas un succès.** README absent, marqueurs absents, ou
  table vide : la suite échoue, avec trois messages distincts.
- **AC5 — les 32 clés sont déclarées.** `### Taxonomie servie` porte les 14 clés
  de `PrefilterClass::as_str()` et les 18 de `Release::as_str()`, et le détecteur
  est **vert** sur l'arbre livré.
- **AC6 — la lecture d'AC4 de (g′) est écrite là où elle se lit.** Le § 11 dit
  que la quantité conservée est la somme de `green_check_suite_head_decided` et
  de `green_check_suite_head_already_reviewed`, et nomme les trois termes à zéro
  attendu plus le terme nominal.
- **AC7 — le comportement du pré-filtre est inchangé.** Le diff ne touche aucun
  fichier de production ; aucune variante, aucun bras de décision. Vérifiable par
  `git diff --stat` : seuls un `mod tests` et un fichier de `docs/`.
- **AC8 — un seul site de vérité pour les listes.** Après le vol, aucune liste
  littérale de variantes de `PrefilterClass` ou de `Release` ne subsiste dans le
  module : le recensement existant consomme la liste dérivée.

## 7. Fire-Disposition

Ce vol livre un **détecteur** : un test dont le chemin de succès est « aucune
clé manquante, aucune entrée morte ». La section est donc requise.

**Option retenue : (a) exception nommée en allowlist — et son ensemble
d'exceptions est VIDE, par construction.**

- **Les violations existantes sont réparées, pas exceptées.** Le détecteur rougit
  aujourd'hui sur dix clés : les deux classes `comment_triaged_note` et
  `green_check_suite_head_already_reviewed`, et les huit termes de relâche
  `triage_escalated`, `triage_unreadable`, `issue_task_active`,
  `triage_disarmed`, `closure_snapshot_absent`, `reviewed_head_absent`,
  `reviewed_head_out_of_contract`, `reviewed_head_switch_disarmed`. L'étape 2 de
  l'ordre de commit les **déclare** dans `### Taxonomie servie`. Il ne reste donc
  aucune violation à excepter, et le détecteur atterrit **armé et vert**. Une
  allowlist vide est ici le bon état : excepter dix clés aurait reporté le défaut
  en le rendant grep-visible, là où le déclarer le ferme.
- **L'assertion auto-nettoyante qu'exige (a) est le second sens du détecteur.**
  Une entrée de table que le Rust ne sert plus fait rougir (AC2). C'est la
  propriété qui empêche une déclaration de devenir stale : le geste qui rend une
  entrée inutile est le geste qui la fait expirer — la propriété que mika#2309
  exige d'une exception pour qu'elle ne dégénère pas en allowlist.
- **Ni (b) ni (c).** (b) — livrer désarmé — serait ici un détecteur qui ne
  détecte rien, alors que le rattrapage tient dans le même commit que lui et que
  le vol entier n'a pas d'autre objet. (c) — halte-et-remontée — n'a pas de
  cadrage à demander : la décision de forme (a) de (g′) est ratifiée (Vincent,
  2026-10-26), et ce vol ne touche à aucun arbitrage opérateur.

## 8. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `webhook_prefilter.rs` — `suivante` × 2 (`match` sans bras `_`, 14 + 18 bras, doc-comments) | 88 |
| `webhook_prefilter.rs` — listes dérivées par `successors` + gardes de cardinalité | 35 |
| `webhook_prefilter.rs` — détecteur : lecture du README, extraction des deux tables, les deux sens, messages | 80 |
| `webhook_prefilter.rs` — les deux contrôles négatifs + les trois cas d'absence | 40 |
| `webhook_prefilter.rs` — migration du recensement existant vers la liste dérivée | 10 |

Total estimé : 253 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 253 estimées contre une borne de
429 et un seuil par PR de 1000 : la plus serrée des deux est la borne par vol, et
le total passe à 59 % d'elle. Le volume est dominé par deux `match` plats — un
bras d'une ligne par variante, sans logique — et par un détecteur qui lit un
fichier et compare deux ensembles de chaînes. Il n'y a ni migration de schéma, ni
chemin asynchrone nouveau, ni fixture à geler : les deux corpus existants ne sont
pas touchés. Les 32 entrées de table sont du `docs/`, hors compte et hors risque
de tours. Le travail de doc (§ 11, § 9) est de la prose sur un document que le
vol lit déjà, sans dépendance à une mesure.

Aucune suite n'est renvoyée : le périmètre est clos par le vol, et la seule chose
qu'il laisse derrière lui est **AC4 de (g′) elle-même**, qui est une mesure
opérateur et non un vol — elle reste portée par #2680, comme depuis le
2026-10-26.
