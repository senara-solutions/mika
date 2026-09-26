# mika#2054 — Le parseur `#[cfg(test)]` ne voit qu'une forme adjacente : rectification du ticket + fermeture du résidu de la même classe

- **Ticket :** senara-solutions/mika#2054
- **Branche :** `fix/2054/verify-egress-no-log-cfg-test`
- **Type :** fix (substrat de garde)
- **Fichiers :** `scripts/verify-egress-no-log.sh`, `scripts/test-verify-egress-no-log.sh`

---

## 1. Ce que la lecture du code déplace dans le ticket — premier livrable

**Les trois cases à cocher du ticket sont déjà cochées, sur `main`, depuis le
2026-08-30.** Le correctif est la PR #2079, commit `1d3b1688`
(`fix(guards): verify-egress-no-log.sh fails closed on unmodeled #[cfg(test)]
instead of abandoning the file (mika#2054)`), et il est présent dans ce
worktree : `git diff origin/main -- scripts/verify-egress-no-log.sh` est vide.

| AC du ticket | état | preuve par lecture |
|---|---|---|
| La branche `exit` échoue au lieu d'abandonner, en nommant la ligne et en disant quoi ajouter au parseur | **livré** | `verify-egress-no-log.sh:167-175` — quatre `printf` vers `/dev/stderr` nommant `FILENAME:FNR`, le texte de la ligne, les deux formes modélisées, et le remède (`extend production_lines()`), puis `exit 3`. L'appelant capture le code (`:213-218`) et appelle `report_violation "parser"` — donc `violations > 0` et la garde sort `1`. |
| La fenêtre de 8 lignes est remplacée par un rattachement au bloc de l'appel, ou la garde échoue si le bloc est indéterminable | **livré** | `macro_block()` (`:259-289`) équilibre les parenthèses depuis l'ouvreur `info!(`, avec `sanitize()` qui ignore les chaînes et les commentaires `//`. Parenthèses non équilibrées ⇒ `exit 2` ⇒ `report_violation "layer-1" "info! (block undeterminable)"` (`:302-308`). |
| Anti-vacuité : un fixture `#[cfg(test)] fn` après du code de production fait échouer la garde ; il passe sur la forme corrigée | **livré** | `scripts/test-verify-egress-no-log.sh` — 16 assertions sur 9 cas, dont exactement la paire demandée : « violation-after-cfg-test » attend `exit 1` (`:95-108`) et « legit-test-mod » attend `exit 0` (`:119-130`). Câblé en CI (`.github/workflows/ci.yml:379-381`) et au Makefile (`:163-164`, `:238-240`). |

**Ce que ça dit du ticket, et pourquoi c'est le premier livrable.** #2054 est
resté **ouvert après sa propre résolution**. Le feeder l'a donc traité comme du
travail disponible : trois re-drives, budget épuisé, `operator-review` posé
(commentaire 2/2 du 2026-09-21). La raison de « aucun progrès observable » n'est
pas que le dispatch échoue — c'est qu'**il n'y a rien à implémenter**. Le
reconciler a fait exactement son travail (mika#2020) sur une population qu'il ne
pouvait pas qualifier : *un ticket sans travail restant et un ticket bloqué
produisent, vus du feeder, les mêmes bytes.*

La conduite qui en découle est écrite ici plutôt que devinée par le prochain
lecteur : **#2054 se ferme**, et ce qui suit est livré sous son numéro parce
que c'est la **même classe dans le même fichier**, mesurée en le lisant.

## 2. Le résidu réellement ouvert — trois formes que #2079 n'a pas fermées

#2079 a fermé la branche « forme non modélisée » en la faisant échouer. Elle n'a
pas touché les **deux parseurs qui décident d'y entrer** : la reconnaissance de
l'attribut (`:150`) et le suivi de portée du bloc (`:137-146`). Les trois défauts
ci-dessous vivent là, et le premier est celui qui a coûté la session de grooming
précédente — dont les fixtures `tmp-residu-2054/` traînent sur le HEAD de cette
branche, committées par le sauvetage post-vol (mika#2031).

### D1 — une ligne intercalaire fait échouer la garde sur du Rust idiomatique

`pending_cfg_test` est consommé par **la ligne suivante immédiate**, quelle
qu'elle soit (`:155-156`). Toute ligne qui n'est ni `mod NAME;` ni `mod NAME {`
tombe dans la branche fail-closed. Or trois formes parfaitement ordinaires
s'intercalent entre un attribut et son item :

```rust
#[cfg(test)]
                                  // (a) ligne vide
mod tests { }

#[cfg(test)]
// pourquoi ces tests vivent ici    (b) commentaire
mod tests { }

#[cfg(test)]
#[allow(clippy::unwrap_used)]     // (c) second attribut
mod tests { }
```

Les trois font sortir la garde en `1` avec le diagnostic « unmodeled
`#[cfg(test)]` form », en pointant une ligne vide ou un commentaire. **Le
message est faux et le remède qu'il prescrit est inapplicable** : il demande
d'envelopper dans un `mod` du code qui l'est déjà.

**C'est fail-closed, donc sans danger pour la sûreté — et c'est précisément
pour ça que ça compte.** Une garde qui rougit sur du code légitime, avec un
diagnostic qui ne décrit pas la faute, est une garde qu'on désarme. Le dépôt a
déjà eu à écrire cette leçon : les fixtures de bonne foi de mika#2496 (N5)
existent « sans quoi le scan serait en permanence rouge et donc désarmé ».

### D2 — `#[cfg(any(test, …))]` n'est pas reconnu, et `#[cfg(not(test))]` est le piège

Le regex de détection est ancré sur la forme littérale :
`/^[[:space:]]*#\[cfg\(test\)\]/` (`:150`). Une forme composée n'est pas
reconnue du tout : `#[cfg(all(test, feature = "x"))] mod tests {` est alors lu
comme du code de production ordinaire, et le corps du module de test est audité
comme s'il en était — chaque `debug!` de test y devient une violation rapportée.

**La population n'est pas hypothétique.** `crates/mika-gateway/src/voice/mod.rs:81`
porte déjà `#[cfg(any(test, feature = "test-utils"))]`, et c'est la forme
canonique maison pour le code `test-utils` (cf. `MockLlmProvider`,
`Settings::test_defaults()` dans le CLAUDE.md racine). Le jour où le substrat
egress en gagne un, la garde rougit sur du test.

**Le piège, et c'est lui qui interdit d'élargir le regex naïvement :
`#[cfg(not(test))]` contient le token `test` et signifie l'inverse** — du code
compilé **hors** tests, donc de la production, donc exactement ce que la garde
doit scanner. Un regex « contient `test` » le sauterait : **fail-open**, la
classe du ticket, réintroduite par le correctif du ticket.

### D3 — le suivi de portée compte les accolades naïvement : fail-open

Le skip de bloc (`:137-146`) fait `gsub(/\{/, "{")` sur la ligne brute. Il compte
donc les accolades **dans les chaînes et les commentaires**, et il ne compte pas
du tout la ligne d'ouverture (`brace_depth = 1` est posé à la main puis `next`,
`:163-165`). Deux manifestations, dont une est un fail-open :

- **D3a — module inline auto-fermé sur sa ligne d'ouverture.**
  `#[cfg(test)] mod tests { fn t() {} }` : la ligne matche `mod NAME {`, le
  parseur pose `brace_depth = 1` et saute la ligne — sans voir qu'elle contient
  autant de fermantes que d'ouvrantes et que le bloc est **déjà clos**. Le
  parseur se croit alors dans un module de test pour le reste du fichier :
  **tout le code de production qui suit est abandonné en silence.** C'est
  littéralement le défaut du ticket — audit partiel indiscernable d'un audit
  complet — sous une forme que #2079 n'a pas fermée.
- **D3b — accolade non appariée dans une chaîne de test.** Un excédent de `{`
  (p. ex. `let j = "{";`) prolonge le skip au-delà du module et emporte le code
  de production suivant : **fail-open**. Un excédent de `}` le termine trop tôt
  et fait auditer du test comme de la production : faux positif.

**L'ironie est l'argument.** #2079 a livré `sanitize()` (`:262-276`) qui fait
exactement ce travail — ignorer chaînes et commentaires — et l'a appliquée au
parseur d'`info!`, à cent lignes de là, **sans l'appliquer au parseur qui décide
ce qui est audité du tout**. La capacité est déjà dans le fichier ; il manque de
la brancher là où son absence est un fail-open.

**Latent aujourd'hui, vérifié par lecture :** le substrat n'emploie que les deux
formes adjacentes modélisées (`mod.rs:57,364,367` ; `brave.rs:265,364`), chacune
sur sa propre ligne, et les accolades de ses chaînes de test sont appariées
(`tests_e4_no_log.rs:68,214`). La garde passe donc légitimement aujourd'hui. Ce
plan ferme une incapacité, il ne répare pas une violation en cours.

## 3. Requirements

- **R1** — Retirer `tmp-residu-2054/` de la branche. Quatre fichiers, 19 lignes,
  committés par le sauvetage post-vol mika#2031 depuis une session de grooming
  morte. Ce sont des fixtures de travail, pas un livrable ; les laisser les
  enverrait dans la PR.
- **R2 (D1)** — Le parseur traverse les lignes non significatives entre
  `#[cfg(test)]` et son item — ligne vide, commentaire de ligne, attribut
  supplémentaire — sans consommer l'état `pending`. Une forme non modélisée
  **après** traversée continue d'échouer comme aujourd'hui.
- **R3 (D2)** — Le parseur reconnaît les formes `cfg` composées qui **incluent**
  la configuration `test` (`all(test, …)`, `any(test, …)`), et **refuse de
  traiter comme test** les formes qui l'excluent (`not(test)`). Une forme `cfg`
  mentionnant `test` qu'il ne sait pas classer **échoue closed**, par le même
  chemin diagnostique que R2.
- **R4 (D3)** — Le suivi de portée du bloc inline compte les accolades
  **sanitisées** (hors chaînes et commentaires) et compte **la ligne
  d'ouverture**, de sorte qu'un module clos sur sa propre ligne ne place pas le
  parseur en skip. Un bloc dont les accolades ne s'équilibrent pas avant la fin
  du fichier **échoue closed**.
- **R5** — `sanitize()` a **une seule définition** dans le fichier, partagée par
  les deux programmes awk. Deux copies divergeraient, et la divergence serait
  exactement du type que personne ne remarque : le parseur d'`info!` resterait
  correct pendant que celui de la portée régresserait.
- **R6 (anti-vacuité)** — Le harness gagne un cas par défaut, chacun vu rouge
  contre le parseur actuel avant d'être vu vert contre le parseur corrigé, plus
  les contrôles de bonne foi qui empêchent une sur-correction.

## 4. Conception

### 4.1 `sanitize()` partagée (R5)

Extraire le corps de la fonction awk dans une constante shell en lecture seule,
interpolée dans les deux programmes :

```bash
# Corps awk partagé : rend la ligne débarrassée de ses littéraux chaîne et de
# son commentaire `//` terminal, pour que le comptage de délimiteurs porte sur
# la syntaxe et non sur du texte. UNE seule définition : les deux parseurs de ce
# fichier (portée `#[cfg(test)]`, bloc `info!`) en dépendent, et une copie qui
# dérive laisse l'un correct pendant que l'autre régresse (mika#2054).
readonly AWK_SANITIZE_FN='
function sanitize(s,   out, i, c, n, instr, prev) { ... }
'
```

Les deux appels `awk` deviennent `awk -v … "$AWK_SANITIZE_FN"'<programme>'`.
Le corps est repris **verbatim** de `macro_block()` — ce plan ne modifie pas son
comportement, il en élargit le lectorat.

### 4.2 Reconnaissance de l'attribut (R3)

Trois classes, décidées sur le texte de l'attribut `cfg` :

| forme | classe | conduite |
|---|---|---|
| `#[cfg(test)]` | test | armer `pending_cfg_test` |
| `#[cfg(all(test, …))]`, `#[cfg(any(test, …))]` — `test` en tête de liste, sans `not` | test | armer `pending_cfg_test` |
| `#[cfg(not(test))]`, `#[cfg(all(not(test), …))]` | **production** | ne rien armer, la ligne suit le chemin ordinaire |
| toute autre forme `cfg` mentionnant `test` | **indécidable** | échouer closed, diagnostic R2 |

La quatrième ligne est le cœur de la conception : le prédicat est **une
allowlist de formes comprises**, jamais une denylist de formes dangereuses.
`#[cfg(not(test))]` est classé explicitement production parce qu'il est la
forme-piège ; tout le reste de l'espace `cfg` × `test` — imbrications
profondes, `cfg_attr`, macros — tombe dans l'indécidable et fait rougir la
garde avec un message qui nomme la ligne. *Construct the incapacity, don't
promise the restraint* (en-tête, `:8-11`).

### 4.3 Traversée des lignes intercalaires (R2)

Tant que `pending_cfg_test` est armé, une ligne **vide**, **entièrement
commentaire** (`//`, `///`, `//!`) ou **attribut** (`#[…]` sur sa propre ligne)
est traversée sans consommer l'état. La première ligne significative décide.

Deux bornes à écrire au site, sans quoi la traversée devient elle-même un
fail-open :

- Un attribut traversé qui est lui-même un `cfg` est reclassé par §4.2 — donc
  `#[cfg(test)]` suivi de `#[cfg(not(test))]` n'est pas un « intercalaire
  neutre ». La conduite sûre y est l'indécidable : deux `cfg` contradictoires
  sur un même item ne sont pas quelque chose que ce parseur a à arbitrer.
- La **fin de fichier** avec `pending_cfg_test` encore armé est une erreur, pas
  une fin propre : un attribut sans item est du code qui ne compile pas, et le
  taire rendrait un fichier tronqué indiscernable d'un fichier sain.

### 4.4 Portée du bloc inline (R4)

À l'entrée, compter les délimiteurs **sanitisés de la ligne d'ouverture** au lieu
de poser `brace_depth = 1` :

```awk
s = sanitize($0)
t = s; opens  = gsub(/\{/, "", t)
t = s; closes = gsub(/\}/, "", t)
depth = opens - closes
if (depth <= 0) { next }        # module clos sur sa ligne — ne PAS entrer en skip
in_inline_test = 1; brace_depth = depth; next
```

Et dans le bloc de skip, compter sur `sanitize($0)` plutôt que sur `$0`. Un
`brace_depth` encore positif à la fin du fichier est un bloc indéterminable :
échec closed, message distinct de celui de R2 (ce n'est pas une forme inconnue,
c'est un bloc non refermé), sur le modèle du « block undeterminable » que
`macro_block()` emploie déjà pour `info!`.

### 4.5 Ce qui ne bouge pas

La branche fail-closed de #2079 (`:167-175`), son diagnostic, `macro_block()`,
`strip_comment_lines()`, les quatre tableaux de motifs interdits, les codes de
sortie, et les 16 assertions existantes du harness. Ce plan **ajoute des formes
comprises et sanitise deux comptages** ; il n'assouplit aucun refus existant.

## 5. Fire-Disposition

Ce plan livre des détecteurs : les cas ajoutés au harness (§6), le parseur
durci, et le scan de définition unique de `sanitize` (R5). La section est donc
requise, et la disposition retenue est
**(a) — exception nommée en allowlist, allowlist livrée VIDE**.

**Population de violations existantes : vide, établie par lecture, pas par
exécution.** Le durcissement change le verdict de la garde dans deux directions
opposées, et chacune a été vérifiée contre le substrat réel :

- *Plus tolérant* (R2, R3) — ne peut retirer aucun refus actuel, le substrat
  n'employant aucune forme intercalaire ni composée
  (`mod.rs:57,364,367` ; `brave.rs:265,364`, toutes adjacentes et littérales).
- *Plus strict* (R4) — le comptage sanitisé ne peut diverger du comptage brut
  que si une chaîne ou un commentaire porte une accolade non appariée. Les
  chaînes du substrat en portent des appariées uniquement
  (`tests_e4_no_log.rs:68` `"{}"`, `:214` `"{text}"`). Aucun module inline n'est
  clos sur sa ligne d'ouverture.

L'allowlist matérialisée est celle du scan R5 :
`scripts/verify-egress-no-log.sh` doit porter **exactement une** définition de
`sanitize(`, et le tableau d'exemptions du scan est **livré vide**.

**Assertion auto-nettoyante, double sens.** Le scan échoue (i) si le compte de
définitions s'écarte de 1 — donc si quelqu'un duplique la fonction plutôt que
de l'interpoler — et (ii) si une entrée d'exemption ne correspond plus à rien
dans le fichier, de sorte qu'une exemption devenue caduque rougit le jour de sa
péremption et non des mois après. Le scan porte aussi son **anti-vacuité** : il
échoue si le nom `sanitize(` est introuvable, sans quoi un renommage le rendrait
silencieusement inerte — et *un scan devenu aveugle se lit exactement comme un
arbre propre* (classe mika#2205).

**Quand ce scan tire, on interpole ; on n'allowliste pas** (doctrine mika#2201).
Une seconde définition de `sanitize` n'est pas une exception à déclarer, c'est
la divergence que R5 existe pour empêcher.

**Gate de pré-vol, bloquant, et il est explicitement une exécution — donc un
geste d'implémentation, pas de grooming :** avant de proposer la PR,
`make verify-egress-no-log` doit rendre `0` sur l'arbre réel **et** le harness
doit rendre `0`. Si le durcissement fait rougir le substrat, **halte** : la
vérification par lecture ci-dessus est fausse quelque part, et c'est elle qu'il
faut refaire — pas le seuil qu'il faut bouger.

## 6. Verification contract

Tous les cas s'ajoutent à `scripts/test-verify-egress-no-log.sh`, qui copie
l'arbre réel dans un répertoire de fixture et le mute d'une seule façon (motif
déjà en place, `:69-75`). Chaque cas négatif doit être **vu rouge** contre le
parseur actuel avant d'être vu vert contre le parseur corrigé — un cas qui n'a
jamais échoué n'atteste de rien.

| # | cas | mutation | attendu | atteste |
|---|---|---|---|---|
| V1 | ligne vide intercalaire | `#[cfg(test)]`, ligne vide, `mod m { debug!(…) }` | `0` | R2 — vu rouge (`1`) aujourd'hui |
| V2 | commentaire intercalaire | idem avec `// pourquoi` | `0` | R2 — vu rouge aujourd'hui |
| V3 | attribut intercalaire | idem avec `#[allow(clippy::all)]` | `0` | R2 — vu rouge aujourd'hui |
| V4 | `cfg(all(test, …))` | `#[cfg(all(test, feature = "x"))] mod m { debug!(…) }` | `0` | R3 — vu rouge aujourd'hui (le `debug!` de test est rapporté) |
| V5 | **`cfg(not(test))` reste production** | `#[cfg(not(test))] fn f() { debug!(…) }` | `1` | R3, le piège — **le contrôle négatif central** : un `0` ici serait le fail-open que R3 introduirait |
| V6 | forme `cfg`/`test` indécidable | `#[cfg_attr(test, …)] fn f() {}` | `1` + « parser » | R3, fail-closed sur l'inconnu |
| V7 | **module inline clos sur sa ligne** | `#[cfg(test)] mod m { fn t() {} }` puis `fn p() { warn!(…) }` | `1` + `warn!` | **D3a — le fail-open** ; vu rouge aujourd'hui (`0` : tout le reste du fichier est abandonné) |
| V8 | accolade non appariée en chaîne | module de test contenant `let j = "{";` puis production avec `warn!` | `1` + `warn!` | D3b — vu rouge aujourd'hui (`0`) |
| V9 | bloc inline jamais refermé | module de test tronqué en fin de fichier | `1` + « undeterminable » | R4, fail-closed |
| V10 | attribut sans item | fichier se terminant sur `#[cfg(test)]` | `1` | R2 borne, §4.3 |
| V11 | bonne foi — accolades appariées en chaîne | module de test avec `format!("{}", x)` puis production propre | `0` | anti-sur-correction : sans lui, « sanitise » et « casse le comptage » sont indiscernables |
| V12 | bonne foi — arbre réel | aucune mutation | `0` | non-régression du substrat vivant |
| V13 | `sanitize` définie une seule fois | scan de source | `0`, population ≥ 1 | R5 + son anti-vacuité |

**V5, V7 et V11 sont les trois qui portent le contrat.** V7 est le fail-open que
ce travail ferme ; V5 est le fail-open que ce travail pourrait **créer** en
élargissant le regex ; V11 est ce qui distingue « le comptage sanitise » de « le
comptage est cassé » — sans lui, une régression qui ferait échouer *tout*
comptage passerait V7 et V8 en ayant l'air correcte.

Les 16 assertions existantes doivent rester vertes sans modification.

## 7. Definition of Done

- [ ] `tmp-residu-2054/` retiré de la branche (R1).
- [ ] `sanitize()` a une définition unique, interpolée dans les deux programmes awk (R5).
- [ ] Le parseur traverse les intercalaires (R2), classe les `cfg` composés et refuse `not(test)` comme test (R3), compte les accolades sanitisées ligne d'ouverture comprise (R4).
- [ ] V1–V13 ajoutés au harness ; V1–V10 **vus rouges** contre le parseur actuel avant correction, consigné dans le corps de PR.
- [ ] Les 16 assertions préexistantes passent sans modification.
- [ ] `make verify-egress-no-log` rend `0` sur l'arbre réel (gate de pré-vol §5).
- [ ] Le corps de PR porte `Closes #2054` et énonce la rectification du §1 : les trois AC étaient livrés par #2079, ce travail ferme le résidu de la même classe.
- [ ] Aucun refus existant de la garde n'est assoupli ; aucun motif interdit retiré.

## Acceptance criteria

Transcrits du corps de #2054, avec leur état établi par lecture et ce que ce
travail y ajoute.

- [x] **AC1 — La branche `exit` échoue au lieu d'abandonner : toute forme non
  modélisée après `#[cfg(test)]` fait sortir la garde non nul avec un message
  qui nomme la ligne et dit quoi ajouter au parseur.** Livré par #2079
  (`1d3b1688`), `verify-egress-no-log.sh:167-175` + `:213-218`. **Ce travail
  élargit sa population** : les formes `cfg` composées indécidables (V6) et
  l'attribut sans item (V10) y entrent, là où elles étaient auparavant soit
  ignorées, soit rapportées sous un diagnostic faux.
- [x] **AC2 — La fenêtre de 8 lignes est remplacée par un rattachement au bloc
  de l'appel, ou la garde échoue quand elle ne peut pas déterminer le bloc.**
  Livré par #2079, `macro_block()` `:259-289`, fail-closed `:302-308`. **Ce
  travail étend la même discipline au second parseur du fichier** : la portée
  `#[cfg(test)]` est désormais elle aussi déterminée sur une syntaxe sanitisée,
  et échoue quand elle ne peut pas l'établir (V9).
- [x] **AC3 — Anti-vacuité : un fixture portant `#[cfg(test)] fn` après du code
  de production fait échouer la garde ; il passe sur la forme corrigée.** Livré
  par #2079, `test-verify-egress-no-log.sh:95-130`. **Ce travail ajoute treize
  cas** (V1–V13), dont dix vus rouges contre le parseur actuel.
- [ ] **AC4 (ajouté par ce plan) — Un module `#[cfg(test)]` clos sur sa propre
  ligne n'abandonne pas le reste du fichier.** V7 : une violation de production
  placée après un tel module est rapportée. C'est le fail-open que #2079 a
  laissé ouvert, dans la classe même du ticket.
- [ ] **AC5 (ajouté par ce plan) — La garde ne rougit pas sur du Rust
  idiomatique.** V1–V3 : ligne vide, commentaire ou attribut entre l'attribut
  `cfg` et son item passent. Un diagnostic faux sur du code légitime est ce qui
  fait désarmer une garde.
- [ ] **AC6 (ajouté par ce plan) — `#[cfg(not(test))]` reste de la production.**
  V5 : une violation sous cet attribut est rapportée. Contrôle négatif du
  fail-open que l'élargissement de AC5/D2 pourrait créer.

## 8. Ce que ce travail n'achète PAS

- **Il ne répare aucune violation en cours.** Le substrat est propre et le reste
  (§5). Ce plan ferme des **incapacités du parseur**, latentes jusqu'au jour où
  quelqu'un écrit une forme Rust ordinaire dans `egress_search/`.
- **Aucun compteur, aucun événement de journal, aucune surface opérateur.** La
  garde est un gate CI : son unique signal est son propre rouge. Le régime
  attendu de V1–V13 est vert, et **leur vert ne prouve rien que le jour où
  quelqu'un a d'abord vu V1–V10 rouges** — ce que la DoD exige de consigner.
- **Il ne couvre pas l'espace `cfg` en entier.** Il couvre les quatre formes
  mesurément vivantes dans ce dépôt et fait échouer le reste en le nommant.
  C'est le contrat du fichier, pas une limite qu'on subit.
- **Il ne dit rien du Layer 2** (métadonnées réseau), qui est du spec de
  substrat et non un invariant de source (en-tête `:30-33`).

## 9. Hors périmètre, délibérément

- **Généraliser `sanitize` aux autres gardes du dépôt** (`check-byte-slices.sh`,
  `check-loop-select.sh`, `check-pilot-push-sites.sh`). Chacune a sa propre
  population et son propre arbitrage fail-open/fail-closed ; une bibliothèque
  awk partagée entre gardes est un couplage qu'aucune mesure ne demande
  aujourd'hui. **Suivi**, précondition : qu'une seconde garde manifeste le même
  défaut de comptage.
- **Un lint de classe « tout parseur de garde doit échouer closed sur une forme
  inconnue »**, la généralisation naturelle de mika#2039 et de ce ticket. Le
  prédicat serait sémantique et donc faux dans les deux sens.
  `docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md`
  porte déjà la leçon sous forme de doctrine — ce qui est le bon support tant
  qu'aucun prédicat mécanisable n'est identifié.
- **Écrire un vrai parseur Rust** plutôt qu'un modèle de portée en awk. La
  garde est un gate CI de quelques centaines de lignes sans dépendance ; lui
  adjoindre `syn` ou un binaire à compiler échange une incapacité bornée et
  nommée contre une dépendance de build sur le chemin de tous les PR.
- **Le contenu de l'allowlist `ALLOWED_EVENT_NAMES`** : y toucher demande un
  bearing Prime (en-tête `:364-366`), et rien ici ne le demande.
- **La fermeture administrative de #2054** : c'est un geste d'orchestrateur, que
  le `Closes #2054` du corps de PR déclenche. Ce plan l'établit et le motive ;
  il ne peut pas le poser (`gh` n'est pas authentifié dans le bac à sable de
  dispatch).
