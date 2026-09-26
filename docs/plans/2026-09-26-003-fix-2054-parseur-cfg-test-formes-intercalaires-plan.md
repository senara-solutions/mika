# mika#2054 — Le parseur `#[cfg(test)]` ne voit qu'une forme adjacente : rectification du ticket + fermeture de deux défauts du même fichier

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

### 1.1 La conduite qui en découle ne revendique aucune fermeture (F1)

Ce plan ne revendique **pas** la résolution de #2054, et son corps de PR porte
**`Refs #2054`**, jamais `Closes`. Revendiquer la fermeture d'un ticket dont les
trois cases ont été cochées par une autre PR serait faux au sens où un
consommateur en aval le lit : `Closes` énonce « les AC de ce ticket sont livrées
par cette PR », et elles ne le sont pas — elles l'étaient déjà.

**Ce que ce choix coûte : rien, et pour une raison mesurée.** Laisser #2054
ouvert n'expose pas la boucle à un nouveau cycle de re-drives : le ticket porte
`operator-review` depuis le 2026-09-21, et ce label l'exclut des **trois** phases
du feeder par `is_feeder_excluded` (mika#2020). Le ticket est déjà tenu, et le
tenir est l'état correct tant que sa portée déclarée n'a pas été tranchée.

**Pourquoi ce travail sort tout de même sous ce numéro.** Les trois cases sont
cochées ; le défaut que le **titre** nomme — « le parseur ne voit qu'une forme
adjacente » — ne l'est qu'à moitié. D1 (§2) est littéralement une forme **non
adjacente** que le parseur ne voit pas, et D3a est littéralement l'audit partiel
indiscernable d'un audit complet que le ticket a été ouvert pour fermer. *Une
case à cocher est un instrument de vérification, pas la définition du défaut* —
et c'est ce qui rattache le résidu à cette classe plutôt qu'à un ticket neuf sans
lignée.

**Le geste administratif est nommé au §9 et aucune ligne de ce plan n'en
dépend.** Trancher entre « rectifier la portée déclarée de #2054 » et « le fermer
comme résolu par #2079, puis ouvrir un suivi » appartient à l'orchestrateur ; le
périmètre d'implémentation ci-dessous est identique dans les deux cas, donc aucune
question de spécification n'est laissée en aval.

## 2. Le résidu réellement ouvert — deux défauts retenus, un reporté

#2079 a fermé la branche « forme non modélisée » en la faisant échouer. Elle n'a
pas touché les **deux parseurs qui décident d'y entrer** : la reconnaissance de
l'attribut (`:150`) et le suivi de portée du bloc (`:137-146`). Les trois défauts
ci-dessous vivent là — mais **ils ne sont pas de la même gravité, et la mesure du
périmètre en écarte un** (F2) : D1 est un faux refus sur du Rust ordinaire, D3 un
fail-open, et D2 un faux positif dont la population dans le périmètre est **vide**
et qui est reporté au §9. Les fixtures `tmp-residu-2054/` qui traînent sur le HEAD
de cette branche viennent de la session de grooming morte sur D1, committées par
le sauvetage post-vol (mika#2031).

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

### D2 — les formes `cfg` composées ne sont pas reconnues — faux positif, population vide, **reporté** (F2)

Le regex de détection est ancré sur la forme littérale :
`/^[[:space:]]*#\[cfg\(test\)\]/` (`:150`). Une forme composée n'est pas reconnue
du tout : `#[cfg(all(test, feature = "x"))] mod tests {` tombe dans la branche
générique `:178` et est émise comme ligne de production ; le corps du module de
test est alors audité comme s'il en était, et chaque `debug!` de test y devient
une violation rapportée.

**La classification que la première rédaction de ce plan avait fausse, et c'est
elle qui décide du périmètre.** Ce n'est **pas** un fail-open : la branche par
défaut du parseur **émet** la ligne, donc une forme non reconnue fait auditer
*plus* de code, jamais moins. D2 est donc un **faux positif** — la famille de D1
(une garde qui rougit sur du code légitime est une garde qu'on désarme), et non
celle de D3 (du code de production abandonné en silence). Les deux familles ne
justifient pas la même urgence, et la première rédaction les traitait à égalité.

**Population dans le périmètre : vide, mesurée et non plus argumentée.** La garde
scanne `crates/mika-gateway/src/egress_search/*.rs` (`:48`, `:220`).
`git grep -n 'cfg' -- crates/mika-gateway/src/egress_search/` rend cinq
occurrences d'attribut, **toutes littérales et adjacentes** (`mod.rs:57,364,367` ;
`brave.rs:265,364`), plus quatre lignes de prose ou de variable locale. Zéro
`all(`, zéro `any(`, zéro `not(`, zéro `cfg_attr`.

**La citation qui avait servi de justification ne portait pas sur le périmètre.**
`crates/mika-gateway/src/voice/mod.rs:81` porte bien
`#[cfg(any(test, feature = "test-utils"))]`, et c'est la forme canonique maison
pour le code `test-utils` — mais `voice/` n'est pas dans le périmètre de cette
garde, donc cette occurrence atteste que la forme est **idiomatique dans le
dépôt**, jamais qu'elle est **vivante là où la garde regarde**. Une forme
idiomatique ailleurs est une raison d'attendre une occurrence réelle, pas d'armer
un parseur par anticipation.

**Le piège qui interdirait de toute façon d'élargir le regex naïvement :
`#[cfg(not(test))]` contient le token `test` et signifie l'inverse** — du code
compilé **hors** tests, donc de la production, donc exactement ce que la garde
doit scanner. Un regex « contient `test` » le sauterait : **fail-open**, la classe
du ticket, réintroduite par son propre correctif. Aujourd'hui `not(test)` n'est
pas reconnu, tombe dans `:178` et est **correctement** audité comme production —
donc le comportement actuel est juste sur la forme-piège, et le seul risque
qu'aurait porté un élargissement était de le casser.

**Conduite retenue : D2 est reporté au §9**, avec pour précondition la première
occurrence réelle d'une forme composée dans `egress_search/`. Ce qui reste de lui
ici est un **verrou** : V5 (§6) fige le traitement correct de `not(test)` de sorte
qu'un futur élargissement du regex rougisse au lieu de rouvrir le fail-open.

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
- **~~R3 (D2)~~ — retiré du périmètre (F2).** Reconnaître les formes `cfg`
  composées est reporté au §9 : la population dans le périmètre de la garde est
  vide (§2 D2), et le défaut est un faux positif, pas un fail-open. Ce qui reste
  ici est le **verrou** V5, qui fige le traitement correct de `#[cfg(not(test))]`
  comme production. L'identifiant R3 n'est pas réattribué — le corps de PR et la
  seconde passe architecte réfèrent ces numéros, et réutiliser R3 pour autre chose
  rendrait cette révision illisible.
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

### 4.2 Reconnaissance de l'attribut — **inchangée** (R3 retiré, F2)

Le regex `/^[[:space:]]*#\[cfg\(test\)\]/` (`:150`) ne bouge pas, et l'absence de
changement est ici une décision, pas une omission. Le prédicat reste **une
allowlist d'une seule forme comprise**, jamais une denylist de formes
dangereuses : toute autre forme `cfg` — composée, `not(test)`, `cfg_attr`,
imbriquée — tombe dans la branche générique `:178`, est émise comme production, et
est donc **auditée**. *Construct the incapacity, don't promise the restraint*
(en-tête, `:8-11`) : l'incapacité en vigueur est déjà du bon côté, et son coût est
un faux positif nommé au §2 D2 plutôt qu'un fail-open.

Ce qui est livré à la place d'un élargissement est le verrou V5, dont la fonction
est de **rougir le jour où quelqu'un élargit ce regex sans traiter `not(test)`** —
la seule manière dont ce report pourrait devenir dangereux.

### 4.3 Traversée des lignes intercalaires (R2)

Tant que `pending_cfg_test` est armé, une ligne **vide**, **entièrement
commentaire** (`//`, `///`, `//!`) ou **attribut** (`#[…]` sur sa propre ligne)
est traversée sans consommer l'état. La première ligne significative décide.

Deux bornes à écrire au site, sans quoi la traversée devient elle-même un
fail-open :

- Un attribut traversé qui est lui-même un `cfg` **n'est pas un intercalaire
  neutre** : il rend l'item indécidable et échoue closed par le chemin
  diagnostique de R2. Ce prédicat est **syntaxique** — « l'attribut traversé
  commence-t-il par `#[cfg` ? » — et ne dépend donc d'aucune classification des
  formes composées, ce qui est ce qui le rend livrable sans R3 (F2). Il couvre le
  cas dangereux (`#[cfg(test)]` suivi de `#[cfg(not(test))]`, deux conditions
  contradictoires sur un même item) au prix d'un refus sur un empilement qui
  serait en principe cohérent — un refus nommé, et le bon côté de l'arbitrage :
  deux `cfg` sur un même item ne sont pas quelque chose que ce parseur a à
  arbitrer.
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

La branche fail-closed de #2079 (`:167-175`), son diagnostic, **le regex de
reconnaissance de l'attribut (`:150`) — R3 retiré, F2**, `macro_block()`,
`strip_comment_lines()`, les quatre tableaux de motifs interdits, les codes de
sortie, et les 16 assertions existantes du harness.

Ce plan fait exactement deux choses : il **autorise la traversée des lignes non
significatives** entre un attribut et son item (R2), et il **sanitise le comptage
de portée** en y incluant la ligne d'ouverture (R4).

**Ce qu'il assouplit, dit précisément.** R2 retire des refus — c'est son objet :
V1–V3 passent de `1` à `0`. Mais ce sont des refus **faux**, portés par un
diagnostic qui prescrivait d'envelopper dans un `mod` du code qui l'était déjà
(§2 D1), et aucun d'eux ne porte sur une ligne du substrat réel, qui n'emploie
aucun intercalaire (§5). Aucun motif interdit n'est retiré, aucun refus fondé n'est
levé, et R4 va dans l'autre sens en refusant ce qui passait.

## 5. Fire-Disposition

Ce plan livre des détecteurs : les cas ajoutés au harness (§6), le parseur
durci, et le scan de définition unique de `sanitize` (R5). La section est donc
requise, et la disposition retenue est
**(a) — exception nommée en allowlist, allowlist livrée VIDE**.

**Population de violations existantes : vide, établie par lecture, pas par
exécution.** Le durcissement change le verdict de la garde dans deux directions
opposées, et chacune a été vérifiée contre le substrat réel :

- *Plus tolérant* (R2 seul, R3 étant retiré — F2) — ne peut retirer aucun refus
  actuel, les cinq attributs du périmètre étant tous adjacents et littéraux
  (`mod.rs:57,364,367` ; `brave.rs:265,364`, relevés par
  `git grep -n 'cfg' -- crates/mika-gateway/src/egress_search/`). La traversée des
  intercalaires ne peut donc changer le verdict d'aucune ligne existante : il n'y a
  aucun intercalaire à traverser.
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

**Les fixtures sont écrites avec l'attribut sur sa propre ligne, et ce détail est
porteur.** Le regex `:150` n'est **pas** ancré à droite, donc
`#[cfg(test)] mod m { fn t() {} }` écrit sur une seule ligne matche l'attribut,
consomme la ligne entière et arme `pending` — la ligne *suivante* tombe alors dans
la branche fail-closed et la garde sort `1` **pour la mauvaise raison** (« parser »,
et non la violation attendue). Une fixture ainsi rédigée rougirait sans rien
attester de D3a. Chaque cas ci-dessous place donc l'attribut sur sa ligne et l'item
sur la suivante.

| # | cas | mutation | attendu | atteste |
|---|---|---|---|---|
| V1 | ligne vide intercalaire | `#[cfg(test)]` / ligne vide / `mod m { debug!(…) }` | `0` | R2 — vu rouge (`1`) aujourd'hui |
| V2 | commentaire intercalaire | idem avec `// pourquoi` | `0` | R2 — vu rouge aujourd'hui |
| V3 | attribut intercalaire | idem avec `#[allow(clippy::all)]` | `0` | R2 — vu rouge aujourd'hui |
| ~~V4~~ | ~~`cfg(all(test, …))`~~ | **retiré — reporté §9 (F2)** : population vide dans le périmètre | — | — |
| V5 | **`cfg(not(test))` reste production** | `#[cfg(not(test))]` / `fn f() { debug!(…) }` | `1` | **verrou, vert aujourd'hui** — voir la note ci-dessous |
| ~~V6~~ | ~~forme `cfg`/`test` indécidable~~ | **retiré (F2)** : `#[cfg_attr(test, …)] fn f()` est de la production, et l'auditer comme telle est **correct** — ce cas prescrivait un faux refus | — | — |
| V7 | **module inline clos sur sa ligne** | `#[cfg(test)]` / `mod m { fn t() {} }` / `fn p() { warn!(…) }` | `1` + `warn!` | **D3a — le fail-open** ; vu rouge aujourd'hui (`0` : le reste du fichier est abandonné) |
| V8 | accolade non appariée en chaîne | `#[cfg(test)]` / `mod m {` / `let j = "{";` / `}` / production avec `warn!` | `1` + `warn!` | D3b — vu rouge aujourd'hui (`0`) |
| V9 | bloc inline jamais refermé | module de test tronqué en fin de fichier | `1` + « undeterminable » | R4, fail-closed |
| V10 | attribut sans item | fichier se terminant sur `#[cfg(test)]` | `1` | R2 borne, §4.3 |
| V11 | bonne foi — accolades appariées en chaîne | module de test avec `format!("{}", x)` puis production propre | `0` | anti-sur-correction : sans lui, « sanitise » et « casse le comptage » sont indiscernables |
| V12 | bonne foi — arbre réel | aucune mutation | `0` | non-régression du substrat vivant |
| V13 | `sanitize` définie une seule fois | scan de source | `0`, population ≥ 1 | R5 + son anti-vacuité |

**V5 est vert aujourd'hui, et on l'ajoute pour ça.** `#[cfg(not(test))]` n'est pas
reconnu par `:150`, tombe dans `:178`, est émis comme production, et son `debug!`
est rapporté : le comportement actuel est **déjà correct**. V5 n'atteste donc
aucune correction — c'est un **verrou dont la valeur est de rougir plus tard**, le
jour où quelqu'un élargit le regex de §4.2 en croyant fermer D2 et saute
`not(test)` avec lui. C'est ce qui rend le report de D2 (§9) réversible sans
risque, et c'est la seule pièce de l'ancien R3 qui reste livrée.

**V7 et V11 portent le contrat de ce qui est corrigé.** V7 est le fail-open que ce
travail ferme ; V11 est ce qui distingue « le comptage sanitise » de « le comptage
est cassé » — sans lui, une régression qui ferait échouer *tout* comptage
passerait V7 et V8 en ayant l'air correcte.

Les 16 assertions existantes doivent rester vertes sans modification.

## 7. Definition of Done

- [ ] `tmp-residu-2054/` retiré de la branche (R1).
- [ ] `sanitize()` a une définition unique, interpolée dans les deux programmes awk (R5).
- [ ] Le parseur traverse les intercalaires (R2) et compte les accolades sanitisées, ligne d'ouverture comprise (R4). **Le regex de reconnaissance de l'attribut (`:150`) n'est pas modifié** — R3 retiré, §4.2.
- [ ] V1–V3, V5, V7–V13 ajoutés au harness (V4 et V6 retirés, F2) ; **V1–V3 et V7–V10 vus rouges** contre le parseur actuel avant correction, consigné dans le corps de PR. V5, V11 et V12 sont verts des deux côtés et le corps de PR le dit — les présenter comme « passés » sans cette précision laisserait croire à une correction qu'ils n'attestent pas.
- [ ] Les 16 assertions préexistantes passent sans modification.
- [ ] `make verify-egress-no-log` rend `0` sur l'arbre réel (gate de pré-vol §5).
- [ ] Le corps de PR porte **`Refs #2054`** et **jamais `Closes`** (§1.1), et énonce la rectification du §1 : les trois AC étaient livrées par #2079, ce travail ferme deux défauts de la même classe dans le même fichier et en reporte un troisième.
- [ ] Aucun motif interdit n'est retiré, et aucun refus **fondé** n'est levé : les seuls refus qui disparaissent sont les faux refus de D1 (V1–V3), dont aucun ne porte sur une ligne du substrat réel (§4.5, §5).

## Acceptance criteria

Transcrits du corps de #2054, avec leur état établi par lecture et ce que ce
travail y ajoute.

**AC1–AC3 sont livrées par #2079 et ce travail ne les revendique pas** (F1, §1.1).
Elles sont transcrites ici parce qu'un plan qui les omettrait laisserait croire
qu'elles restent ouvertes ; leur `[x]` date du 2026-08-30, pas de cette PR. Seules
AC4 et AC5 sont les acceptance criteria **de ce travail**, et AC6 est le verrou du
report décidé en §9.

- [x] **AC1 — La branche `exit` échoue au lieu d'abandonner : toute forme non
  modélisée après `#[cfg(test)]` fait sortir la garde non nul avec un message
  qui nomme la ligne et dit quoi ajouter au parseur.** Livré par #2079
  (`1d3b1688`), `verify-egress-no-log.sh:167-175` + `:213-218`. **Ce travail
  élargit sa population** : l'attribut sans item (V10) et le second `cfg`
  intercalaire (§4.3) y entrent, là où ils étaient auparavant rapportés sous un
  diagnostic faux. Les formes `cfg` composées n'y entrent **pas** — V6 retiré,
  F2 : leur population dans le périmètre est vide et leur traitement actuel est
  correct.
- [x] **AC2 — La fenêtre de 8 lignes est remplacée par un rattachement au bloc
  de l'appel, ou la garde échoue quand elle ne peut pas déterminer le bloc.**
  Livré par #2079, `macro_block()` `:259-289`, fail-closed `:302-308`. **Ce
  travail étend la même discipline au second parseur du fichier** : la portée
  `#[cfg(test)]` est désormais elle aussi déterminée sur une syntaxe sanitisée,
  et échoue quand elle ne peut pas l'établir (V9).
- [x] **AC3 — Anti-vacuité : un fixture portant `#[cfg(test)] fn` après du code
  de production fait échouer la garde ; il passe sur la forme corrigée.** Livré
  par #2079, `test-verify-egress-no-log.sh:95-130`. **Ce travail ajoute onze cas**
  (V1–V3, V5, V7–V13 ; V4 et V6 retirés par F2), dont **sept** vus rouges contre le
  parseur actuel — V5, V11 et V12 étant verts des deux côtés par construction.
- [ ] **AC4 (ajouté par ce plan) — Un module `#[cfg(test)]` clos sur sa propre
  ligne n'abandonne pas le reste du fichier.** V7 : une violation de production
  placée après un tel module est rapportée. C'est le fail-open que #2079 a
  laissé ouvert, dans la classe même du ticket.
- [ ] **AC5 (ajouté par ce plan) — La garde ne rougit pas sur du Rust
  idiomatique.** V1–V3 : ligne vide, commentaire ou attribut entre l'attribut
  `cfg` et son item passent. Un diagnostic faux sur du code légitime est ce qui
  fait désarmer une garde.
- [ ] **AC6 (ajouté par ce plan, reformulé rev 2) — `#[cfg(not(test))]` reste de
  la production, et ce fait est désormais verrouillé.** V5 : une violation sous
  cet attribut est rapportée. Ce n'est **pas** le contrôle négatif d'un
  élargissement livré — l'élargissement est reporté (§9, F2) — c'est le verrou qui
  rendra ce report réversible sans risque : il rougira le jour où quelqu'un
  élargira le regex de §4.2 en sautant la forme-piège. Un test vert dont la valeur
  est de rougir plus tard.

## 8. Ce que ce travail n'achète PAS

- **Il ne répare aucune violation en cours.** Le substrat est propre et le reste
  (§5). Ce plan ferme des **incapacités du parseur**, latentes jusqu'au jour où
  quelqu'un écrit une forme Rust ordinaire dans `egress_search/`.
- **Aucun compteur, aucun événement de journal, aucune surface opérateur.** La
  garde est un gate CI : son unique signal est son propre rouge. Le régime attendu
  des cas ajoutés est vert, et **leur vert ne prouve rien que le jour où quelqu'un
  a d'abord vu V1–V3 et V7–V10 rouges** — ce que la DoD exige de consigner. V5,
  V11 et V12 sont verts des deux côtés et n'attestent donc aucune correction ; les
  compter comme preuve serait exactement l'erreur que cette clause existe pour
  interdire.
- **Il ne touche pas à l'espace `cfg`, et ne le couvre donc pas davantage
  qu'aujourd'hui** (F2). La seule forme reconnue reste `#[cfg(test)]` littéral —
  l'unique forme mesurément vivante dans le périmètre (cinq occurrences, §2 D2) —
  et tout le reste continue d'être audité comme production, ce qui est le côté sûr.
  L'élargissement est reporté au §9 avec sa précondition.
- **Il ne dit rien du Layer 2** (métadonnées réseau), qui est du spec de
  substrat et non un invariant de source (en-tête `:30-33`).

## 9. Hors périmètre, délibérément

- **Reconnaître les formes `cfg` composées — D2, ex-R3, retiré en rev 2 (F2).**
  `all(test, …)`, `any(test, …)` et leurs imbrications ne sont pas reconnues et
  continuent d'être auditées comme production. **Précondition du suivi : la
  première occurrence réelle d'une forme composée dans
  `crates/mika-gateway/src/egress_search/`** — le grep du §2 D2 en rend zéro
  aujourd'hui, et armer un parseur sur une population vide est la définition du
  YAGNI. Le défaut est un **faux positif** (auditer du test comme de la
  production), donc sa réalisation coûte un rouge visible et immédiat, jamais un
  audit silencieusement partiel ; le report n'expose donc aucune fenêtre de
  fail-open. V5 le verrouille dans l'autre sens : un élargissement qui sauterait
  `not(test)` rougira. La forme est idiomatique ailleurs dans le dépôt
  (`crates/mika-gateway/src/voice/mod.rs:81`), ce qui rend l'occurrence
  **probable** — mais « probable ailleurs » n'est pas « vivant ici ».
- **TF-suggest : auditer les autres gardes structurelles pour les mêmes patterns
  D1–D3** (F3). Trois défauts d'une même classe — état `pending` consommé par la
  ligne suivante immédiate, comptage de délimiteurs sur du texte non sanitisé,
  profondeur initiale posée à la main sans compter la ligne d'ouverture — ont été
  trouvés dans **un seul** fichier, par lecture. `scripts/check-byte-slices.sh`,
  `scripts/check-loop-select.sh`, `scripts/check-a2a-timeout-literals.sh`,
  `scripts/check-landing-tokens.sh`, `scripts/check-pilot-push-sites.sh` et
  `scripts/check-dispatch-seats-declared.sh` n'ont **pas** été audités pour eux, et
  rien ne permet de supposer qu'un seul fichier les portait. La classe ne devrait
  pas être considérée close sur la seule foi de ce ticket. **Ce n'est pas un
  périmètre à ajouter ici** : chaque garde a sa population et son arbitrage
  fail-open/fail-closed propres — `check-pilot-push-sites.sh` est explicitement
  positionnel-et-lexical par décision (mika#2520), et lui appliquer le remède de
  D3 sans lire cette décision serait une régression. **Suivi**, sans précondition :
  c'est une lecture, pas une réaction à une mesure, et son produit attendu est un
  verdict par garde — « porte le défaut » / « ne le porte pas » / « le porte et
  c'est voulu » — à consigner dans
  `docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md`,
  qui porte déjà la doctrine de la classe.
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
- **Le sort administratif de #2054 — geste d'orchestrateur, hors du chemin
  d'implémentation (F1).** Le corps de PR porte `Refs #2054` et ne ferme rien
  (§1.1), donc rien dans cette PR ne dépend de la décision. Deux voies, entre
  lesquelles ce plan ne tranche pas parce que les deux laissent son périmètre
  d'implémentation inchangé : **(a)** rectifier la portée déclarée de #2054 pour
  qu'elle soit « durcissement post-#2079 du même parseur », en y transcrivant AC4
  et AC5, après quoi un `Closes` deviendrait légitime — c'est la voie qui préserve
  la lignée du titre ; **(b)** fermer #2054 comme résolu par #2079 et ouvrir un
  suivi portant D1, D3 et ce plan tel quel. **Précondition commune, et elle
  importe : ne pas retirer `operator-review` avant d'avoir tranché.** Ce label est
  ce qui tient le ticket hors des trois phases du feeder (§1.1) ; le retirer avant
  que la portée soit claire relancerait exactement le cycle de re-drives qui a
  produit le commentaire 2/2 du 2026-09-21. Ce plan ne peut poser aucun des deux
  gestes : `gh` n'est pas authentifié dans le bac à sable de dispatch.

## Revision history

- **rev 2 (2026-09-26)** — première passe architecte, `Disposition: ITERATE`.
  - **F1 traitée** en retirant la revendication de fermeture plutôt qu'en
    argumentant qu'elle est justifiée : nouveau §1.1, le corps de PR porte
    `Refs #2054` et jamais `Closes`, la DoD le dit, l'en-tête des acceptance
    criteria énonce qu'AC1–AC3 sont livrées par #2079 et **non revendiquées ici**,
    et le §9 nomme les deux voies administratives avec leur précondition commune
    (ne pas retirer `operator-review` avant d'avoir tranché). Ce qui est conservé du
    §1 est la rectification elle-même, que la finding valide ; ce qui est retiré est
    la conclusion « #2054 se ferme », qui la dépassait. Aucune question de
    spécification ne subsiste en aval : les deux voies laissent le périmètre
    d'implémentation identique.
  - **F2 traitée** en exécutant la mesure qu'elle demandait au lieu de la trancher
    par argument. `git grep -n 'cfg' -- crates/mika-gateway/src/egress_search/`
    rend cinq attributs, **tous littéraux et adjacents**, zéro forme composée — la
    finding est confirmée. La lecture du parseur (`:150` non ancré à droite, `:178`
    émettant toute ligne non matchée) a de plus établi que **D2 est un faux positif
    et non un fail-open**, ce que la rev 1 classait à tort à égalité avec D3. Donc :
    R3 retiré, §4.2 devient « inchangée, et c'est une décision », V4 et V6 retirés
    de la table (V6 prescrivait un **faux refus** : `#[cfg_attr(test, …)] fn f()`
    est bien de la production), V5 conservé mais **reclassé en verrou vert** dont la
    valeur est de rougir si quelqu'un élargit le regex plus tard, AC6 reformulée en
    ce sens, §5 et §8 réalignés sur la mesure, et D2 reporté au §9 avec sa
    précondition explicite. Les identifiants V4/V6/R3 ne sont **pas** réattribués,
    la seconde passe et le corps de PR référant ces numéros.
  - **F3 traitée** par un bullet TF-suggest distinct en §9 — distinct du bullet
    « généraliser `sanitize` » qui existait déjà et qui portait sur le **partage de
    code**, non sur l'**audit**. Les six gardes concernées sont nommées, le produit
    attendu est un verdict par garde à consigner dans la doctrine existante
    (`structural-guard-fails-open-parser-fixture-harness.md`), et la raison de ne
    **pas** l'absorber dans ce périmètre est écrite : `check-pilot-push-sites.sh`
    est positionnel-et-lexical **par décision** (mika#2520), donc lui appliquer le
    remède de D3 sans lire cette décision serait une régression.
  - **Correction de précision non demandée, trouvée en vérifiant F2** : les
    fixtures V7 et V8 de la rev 1 plaçaient l'attribut et l'item sur la même ligne.
    Le regex `:150` n'étant pas ancré à droite, une telle fixture consomme la ligne
    entière et fait sortir la garde `1` par la branche « parser » — donc **rouge
    pour la mauvaise raison**, sans rien attester de D3a. Le §6 porte désormais la
    règle de rédaction et les fixtures corrigées. Sans cette correction,
    l'implémenteur aurait vu ses cas rougir puis verdir en croyant avoir attesté le
    fail-open.
