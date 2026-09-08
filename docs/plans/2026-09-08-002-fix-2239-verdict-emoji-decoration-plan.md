---
issue: 2239
type: fix
title: "parse_verdict tolère la décoration de fin de valeur (`VERDICT: pass ✅`) — le 3e verrou du merge autonome"
branch: fix/2239/verdict-parse-verdict-classe-verdict
---

# Plan — #2239 : `VERDICT: pass ✅` classe en `Missing` et bloque le merge autonome

## Problème (mesuré 2026-09-08, re-vérifié sur le code à `29d0cfdb`)

`parse_verdict` (`crates/mika-agent/src/server/verdict.rs:162`) rend `Verdict::Missing`
sur `VERDICT: pass ✅`, la forme que `mika-platform-qa` a émise dans **les quatre** reviews
qu'il a postées sur la PR #2236 — les trois `APPROVED` (08:25:08Z, 08:47:25Z, 09:29:39Z) et
la `COMMENTED` initiale (08:03:23Z), toutes avec `VERDICT: pass ✅` en ligne 1
(vérifié via `gh api repos/senara-solutions/mika/pulls/2236/reviews`). Le merge autonome n'a jamais
basculé ; #2236 a été mergée à la main à 12:00:36.

Chaîne exacte, relue dans le fichier :

1. `VERDICT_RE` (`verdict.rs:54-55`) = `(?mi)^\s*[*_]*\s*VERDICT:\s*(.+)$` — `.+` glouton
   capture jusqu'en fin de ligne → `caps[1].trim()` = `"pass ✅"`.
2. Le pipeline de normalisation (`verdict.rs:163-180`) fait trois choses et **seulement**
   trois : peler l'emphase de tête (`trim_start_matches(['*','_'])`), tronquer au premier
   `**`/`__` de fermeture (mika#1821), repeler l'emphase par `strip_md_emphasis`
   (`verdict.rs:117`, `trim_matches(['*','_'])`). Aucune des trois ne touche à ` ✅`.
3. `value.eq_ignore_ascii_case("pass")` avec `value == "pass ✅"` → `false`.
   `BLOCK_RE` (`:60`) et `HOLD_RE` (`:63`) sont ancrés `^…$` → aucun match.
   `normalize_alias` (`:124`) rend `"pass ✅"` (l'emoji n'est ni `_`, ni `-`, ni whitespace)
   → `alias_to_verdict` (`:151`) → `None`.
4. Chute en `Verdict::Missing { truncated: false }` → `handle_missing_verdict`
   (`verdict_handler.rs:1678`) → `verdict_classification_failed` → jamais `handle_pass_verdict`.

Le flux merge est event-driven sur le verdict parsé, **pas** sur `review.state` GitHub
(`verdict_handler.rs:131`, commentaire `#889` : « authoritative regardless of GH review.state »).
Donc trois `APPROVED` + CI CLEAN ne rattrapent rien : la valeur du verdict est le seul vote.

### Constat annexe, mesuré : le message du WARN a induit en erreur

`handle_missing_verdict` logge littéralement `"verdict_classification_failed: no parseable
VERDICT: line in review body"` (`verdict_handler.rs:1697`). Ici la ligne `VERDICT:` **était**
présente et parsable ; c'est sa **valeur** qui a été rejetée. `Verdict::Missing` ne distingue
pas les deux cas, et le message affirme le premier. C'est exactement ce qui a fait diagnostiquer
à mika-dev « reviews sans ligne VERDICT parsable » — un diagnostic faux produit par une
observable fausse, pas par un mauvais raisonnement.

## Décisions de grooming (tranchées — passe 1 architecte, findings F1-F4)

### D-A — Parseur seul, pas l'émetteur (frontière tolérance/contrainte)

Le ticket laissait ouvert « tolérer côté parseur » vs « contraindre l'émetteur ». **Parseur seul.**

1. **Le prompt émetteur dit déjà « nu ».** `skills/bundled/qa-review/system_prompt.md` ne montre
   l'emoji sur **aucun** de ses exemples de ligne `VERDICT:` (`:558`, `:573`, `:601`, `:613`,
   `:631`, `:652`, `:679`, `:691`, `:710` — tous `VERDICT: pass` / `block[ac]` nus). L'émetteur a
   décoré **contre** son prompt. Ajouter « et pas d'emoji » ajoute une phrase à un prompt déjà
   tenu en échec sur ce point précis : c'est de l'enforcement par prompt au niveau substrat.
2. **Le parseur porte déjà ce contrat.** mika#1821 (troncature `**`) et mika#1828 (peel d'emphase
   + table d'alias) ont établi que « les reviewers décorent leur verdict » est une classe traitée
   côté moteur. L'emoji est une décoration de plus dans une famille déjà nommée.
3. **Blast radius.** Le fix parseur est additif et n'est atteint que lorsque la classification
   actuelle a **déjà** échoué (D-B) : aucune forme aujourd'hui reconnue ne change de sens.

### D-B — Repli additif après la passe primaire, pas normalisation en amont (F2)

**Repli additif.** La décoration est retirée **seulement** quand `classify_value(value)` a déjà
rendu `None`. L'alternative — dénuder d'abord, classifier ensuite — n'a qu'un seul chemin de
lecture et serait plus élégante, mais elle place les acquis mika#1821 et mika#1828 **en aval** du
nouveau helper : toute erreur dans `strip_trailing_decoration` deviendrait une régression sur des
formes aujourd'hui vertes.

Le repli additif rend la règle d'arrêt **vérifiable et non négociable** : aucun test
`parse_verdict_*` existant (`verdict.rs:283-556`) ne doit être modifié (AC4). Cette propriété est
la raison du choix ; le coût accepté est un second chemin de code, borné à trois lignes.

### D-C — Exemption de `]` seul, pas de la classe « fermeture de bracket » (F1)

**`]` seul.** L'ensemble d'exemption n'est pas un pari de style : il est **dérivé de la grammaire**.
Les seules formes canoniques à bracket sont `block[…]` (`BLOCK_RE`, `verdict.rs:60`) et `hold[…]`
(`HOLD_RE`, `verdict.rs:63`). `]` est exempté parce qu'il les termine ; le retirer casserait
l'ancrage `^…$` de ces deux regex.

Élargir à `]})>` serait **contre-productif, pas seulement inutile** : exempter un caractère rend
le retrait *plus faible*, jamais plus fort. `pass 🎉)` s'arrêterait sur le `)` exempté et
resterait `Missing`. On ajouterait donc des exemptions mortes — aucune grammaire derrière — qui
bloquent le retrait légitime dans les cas qu'elles couvrent.

**Liaison inscrite dans le code :** le commentaire canonique de `strip_trailing_decoration` nomme
`BLOCK_RE`/`HOLD_RE` comme source de l'exemption, et exige que toute nouvelle forme de verdict à
bracket étende la regex **et** l'exemption dans le même changement.

### D-D — Décoration de tête : attendre la mesure, avec condition de réveil concrète (F3)

**Attendre la mesure.** `VERDICT: ✅ pass` n'a été observée sur aucune review réelle. La couvrir
maintenant élargirait la frontière sur une hypothèse, contre la discipline « preuve dure avant de
filer ». Aucun ticket n'est ouvert pour elle — un dormeur sans preuve serait du bruit, pas un
dormeur.

**Condition de réveil, datable et vérifiable :** un WARN
`verdict_approved_but_unclassified` (D2c) portant un champ `verdict_value` dont la valeur commence
par autre chose qu'un alphanumérique ASCII. C'est-à-dire : la décoration de tête devient un
ticket le jour où le moteur la mesure lui-même, pas le jour où on l'imagine.

**Cette condition n'existe que si D2 est livré** — d'où D-E.

### D-E — D2 (observable honnête) est dans le périmètre, avec AC dédié (F4)

**Dans le périmètre.** Deux raisons, la seconde étant structurelle :

1. L'observable fausse a produit un **diagnostic faux mesuré** dans la même fenêtre d'incident :
   interrogé sur #2236, mika-dev a répondu « reviews sans ligne VERDICT parsable » — ce que le WARN
   affirme littéralement (`verdict_handler.rs:1697`) alors que la ligne était présente.
2. **D2 est le détecteur de la condition de réveil de D-D.** Sortir D2 dans un ticket séparé
   laisserait la décoration de tête aussi invisible que la décoration de queue l'a été jusqu'à
   aujourd'hui — c'est-à-dire qu'on reproduirait exactement le défaut qu'on répare. Les deux
   livrables ne sont séparables qu'en apparence.

Coût : ~15 lignes, **aucun changement de comportement** (`Verdict::Missing` continue de router en
safe-default `hold[review]`). Porté par AC5.

### Écart assumé avec la piste du ticket

Le ticket propose « retenir le **premier token alphanumérique + `[...]` optionnel**
(`^([a-z]+(?:\[[^\]]*\])?)`) ». Ce plan retient plutôt le **retrait du suffixe non-alphanumérique**.
Raison mesurable : la forme tête-de-token tronque `request changes ✅` en `request`, valeur qui
ne figure pas dans la table d'alias `alias_to_verdict` (`verdict.rs:151-158`, entrées
`request changes` / `request change` / `changes requested`) — elle régresserait mika#1828 AC2 sur
toute valeur d'alias multi-mots décorée. Le retrait de suffixe rend `request changes`, qui
matche. Même intention (« ignorer une décoration de fin »), forme qui ne casse pas l'acquis.

## Livrables

### D1 — Repli « décoration » dans `parse_verdict` (`verdict.rs`)

**D1a.** Extraire la cascade de classification actuelle (canonique `pass` → `BLOCK_RE` →
`HOLD_RE` → alias) dans un helper `fn classify_value(value: &str) -> Option<Verdict>`.
Déplacement pur, y compris le `info!(event = "verdict_alias_normalized", …)` existant.

**D1b.** Ajouter le retrait de décoration de queue :

```rust
/// Retire un suffixe décoratif d'une valeur de verdict (mika#2239).
///
/// Les reviewers décorent : `pass ✅`, `block[ac] ❌`, `hold[review] ⏸️`. La décoration
/// est toujours une queue de caractères non-alphanumériques.
///
/// L'ensemble d'exemption est DÉRIVÉ DE LA GRAMMAIRE, pas choisi : `]` est le seul
/// caractère non-alphanumérique porteur de signal, parce qu'il termine les deux seules
/// formes canoniques à bracket — `block[…]` (`BLOCK_RE`) et `hold[…]` (`HOLD_RE`), toutes
/// deux ancrées `^…$`. Toute nouvelle forme de verdict à bracket DOIT étendre la regex ET
/// cette exemption dans le même changement, sinon le retrait la mutile en silence.
/// N'exemptez PAS `)`, `}`, `>` : aucune grammaire derrière, et exempter un caractère
/// AFFAIBLIT le retrait (`pass 🎉)` s'arrêterait sur le `)` et resterait `Missing`).
///
/// Volontairement conservateur — la queue s'arrête au premier alphanumérique ASCII, donc
/// un vrai commentaire de fin (`pass — but see findings`) n'est PAS avalé et continue de
/// classer `Missing`. C'est la borne de mika#1821, inchangée.
///
/// La décoration de TÊTE (`VERDICT: ✅ pass`) est hors périmètre (mika#2239 D-D) : jamais
/// mesurée. Sa condition de réveil est un `verdict_approved_but_unclassified` dont le
/// champ `verdict_value` ne commence pas par un alphanumérique ASCII.
fn strip_trailing_decoration(value: &str) -> &str {
    value.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != ']')
}
```

**D1c.** Câbler le repli dans `parse_verdict`, **après** la passe primaire :

```rust
let value = strip_md_emphasis(truncated_at_close.trim()).trim();

// Passe primaire — pipeline mika#1821/#1828 inchangé.
if let Some(v) = classify_value(value) {
    return v;
}

// mika#2239 : repli décoration. Atteint uniquement quand la passe primaire a
// déjà échoué → aucune forme actuellement reconnue ne change de sens.
let undecorated = strip_trailing_decoration(value).trim_end();
if undecorated != value {
    if let Some(v) = classify_value(undecorated) {
        info!(
            event = "verdict_decoration_stripped",
            raw_value = value,
            undecorated = undecorated,
            mapped_to = ?v,
            "verdict: suffixe décoratif retiré avant classification (mika#2239)"
        );
        return v;
    }
}

return Verdict::Missing { truncated: body.contains("[truncated]") };
```

L'événement `verdict_decoration_stripped` est la contrepartie observable : chaque fois que le
repli sauve un verdict, on sait quel émetteur décore et comment. Sans lui, le fix rend la
dérive émetteur invisible au lieu de bruyante.

### D2 — Rendre l'observable honnête (`verdict.rs` + `verdict_handler.rs`)

**D2a.** Exposer dans `verdict.rs` :

```rust
/// La valeur brute de la ligne `VERDICT:` si une telle ligne existe (mika#2239).
/// `None` ⇔ aucune ligne `VERDICT:` dans le corps. Sert à distinguer, côté handler,
/// « pas de ligne » de « ligne présente, valeur non reconnue ».
pub(crate) fn verdict_raw_value(body: &str) -> Option<String>
```

**D2b.** Dans `handle_missing_verdict` (`verdict_handler.rs:1678`), remplacer le WARN unique
par une branche selon `verdict_raw_value(&event.body)` :

- `None` → message actuel conservé (`no parseable VERDICT: line in review body`).
- `Some(v)` → `verdict_classification_failed: VERDICT: line present but value unrecognized`,
  avec le champ structuré `verdict_value = %v`. C'est le champ qui aurait rendu le diagnostic
  de ce ticket immédiat au lieu de manuel.

**D2c.** Quand `event.state == "approved"`, émettre en plus le WARN nommé demandé par le
ticket, `verdict_approved_but_unclassified` (champs : `pr_number`, `repo`, `reviewer`,
`review_url`, `verdict_value`). Une review APPROVED qui ne classe pas est le signal fort :
GitHub dit oui, le moteur ne comprend pas — et jusqu'ici cela tombait dans le même WARN
générique que « le reviewer n'a rien écrit ».

**Aucun changement de comportement dans D2** : `Verdict::Missing` continue de router en
safe-default hold[review]. D2 ne touche que ce qui est dit, pas ce qui est fait.

### D3 — Tests (rouges avant, verts après)

Dans `verdict.rs` `mod tests` :

| Test | Entrée | Attendu |
|---|---|---|
| `parse_verdict_emoji_suffix_pass` | `VERDICT: pass ✅` | `Verdict::Pass` |
| `parse_verdict_emoji_suffix_block` | `VERDICT: block[ac] ❌` | `Block("ac")` |
| `parse_verdict_emoji_suffix_hold` | `VERDICT: hold[review] ⏸️` | `Hold("review")` |
| `parse_verdict_emoji_suffix_alias` | `VERDICT: approved ✅` | `Verdict::Pass` |
| `parse_verdict_bold_plus_emoji` | `**VERDICT: pass ✅**` | `Verdict::Pass` (cumul #1828 + #2239) |
| `parse_verdict_field_shape_pr2236` | corps de review réel #2236 (ligne 1 = `VERDICT: pass ✅`, puis `DEPTH:`, `REASON:`) | `Verdict::Pass` |

Bornes — **doivent rester `Missing`** (ce sont elles qui prouvent que la frontière tient) :

| Test | Entrée | Attendu |
|---|---|---|
| `parse_verdict_trailing_comment_still_missing` | `VERDICT: pass — but see findings below` | `Missing` |
| `parse_verdict_unknown_token_with_emoji_still_missing` | `VERDICT: frobnicate ✅` | `Missing` |

Unitaires `strip_trailing_decoration` : `"pass ✅"→"pass"`, `"block[ac] ❌"→"block[ac]"`,
`"hold[review] ⏸️"→"hold[review]"`, `"pass"→"pass"` (no-op), `"block[ac]"→"block[ac]"`
(no-op — la garde `]`), `""→""`.

`verdict_raw_value` : `Some("pass ✅")` sur le corps emoji ; `None` sur un corps sans ligne
`VERDICT:`. C'est le cœur testable de D2 ; la branche WARN elle-même est un wrapper mince
au-dessus de cette fonction et n'est pas testée par capture de tracing.

**Non-régression :** les tests `parse_verdict_*` existants (`verdict.rs:283-556`, dont
`parse_verdict_regression_frobnicate_still_missing:522` et `parse_verdict_pr1821_combined_shape:505`)
doivent tous rester verts sans modification. Si l'un doit être édité, le repli D1 est trop
large — c'est le signal d'arrêt.

## Vérification

```bash
cargo test -p mika-agent --lib server::verdict
cargo test -p mika-agent --lib server::verdict_handler
cargo clippy --all-targets -- -D warnings
```

Rouge-avant exigé : lancer les six tests D3 **avant** D1 et constater l'échec de chacun des
six positifs (les deux bornes doivent, elles, être vertes dès avant — elles décrivent le
comportement actuel qu'on préserve).

## Acceptance criteria

- [ ] **AC1** — `parse_verdict("VERDICT: pass ✅")` rend `Verdict::Pass`. Le corps de review réel
      de senara-solutions/mika#2236 (ligne 1 `VERDICT: pass ✅`, puis `DEPTH:`, `REASON:`) rend
      `Verdict::Pass`.
- [ ] **AC2** — `VERDICT: block[ac] ❌` rend `Block("ac")` ; `VERDICT: hold[review] ⏸️` rend
      `Hold("review")` ; `VERDICT: approved ✅` rend `Verdict::Pass` (chemin alias décoré) ;
      `**VERDICT: pass ✅**` rend `Verdict::Pass` (cumul mika#1828 + mika#2239).
- [ ] **AC3** — La frontière tient : `VERDICT: pass — but see findings below` et
      `VERDICT: frobnicate ✅` rendent tous deux `Verdict::Missing`. Un commentaire de fin n'est
      pas une décoration ; un jeton inconnu décoré reste inconnu.
- [ ] **AC4** — Règle d'arrêt (D-B) : **aucun** test `parse_verdict_*` existant
      (`verdict.rs:283-556`) n'est modifié, supprimé, ni marqué `#[ignore]`. Si l'un doit l'être,
      le repli est trop large et le plan est faux — ce n'est pas un test à ajuster.
- [ ] **AC5** — Observable honnête : `handle_missing_verdict` distingue « aucune ligne `VERDICT:` »
      (message actuel conservé) de « ligne présente, valeur non reconnue » (message distinct +
      champ structuré `verdict_value`). Quand `event.state == "approved"`, le WARN nommé
      `verdict_approved_but_unclassified` est émis en plus. Aucun changement de routage.
- [ ] **AC6** — `verdict_decoration_stripped` est émis (champs `raw_value`, `undecorated`,
      `mapped_to`) à chaque fois — et seulement quand — le repli sauve un verdict que la passe
      primaire avait rejeté.

## Rattachement aux critères d'acceptation

| AC | Porté par | Vérifié par |
|---|---|---|
| AC1 | D1a + D1b + D1c | `parse_verdict_emoji_suffix_pass`, `parse_verdict_field_shape_pr2236` |
| AC2 | D1b (exemption `]`, D-C) + D1c | `parse_verdict_emoji_suffix_block`, `…_hold`, `…_alias`, `parse_verdict_bold_plus_emoji` |
| AC3 | D1b (arrêt au premier alphanumérique) | `parse_verdict_trailing_comment_still_missing`, `parse_verdict_unknown_token_with_emoji_still_missing` |
| AC4 | D-B (repli **après** la passe primaire) | suite `parse_verdict_*` existante, verte sans édition ; `git diff` sur `verdict.rs:283-556` vide en zone test |
| AC5 | D2a + D2b + D2c | `verdict_raw_value` : `Some("pass ✅")` sur corps décoré, `None` sans ligne `VERDICT:` — la branche WARN est un wrapper mince au-dessus (voir Fire-Disposition, détecteur 3) |
| AC6 | D1c (`info!`) | inspection du chemin de repli ; non prouvable par capture de tracing en test unitaire |

## Fire-Disposition

Ce plan livre trois artefacts de classe détecteur, dont deux tirent à l'exécution sur des données
**préexistantes** (les reviews futures). La porte mika#1574 exige de dire ce qui se passe quand ils
tirent sur de l'existant, et non sur ce que la PR ajoute. Les trois n'ont pas le même rapport à
l'existant ; disposition séparée pour chacun.

**Détecteur 1 — la suite de tests D3 : disposition (c) halte-et-remontée, sans exception nommée.**
Chaque cas construit sa propre chaîne littérale ; aucun ne lit un corpus du dépôt. Il n'existe donc
**aucune donnée préexistante** sur laquelle ces tests puissent tirer — l'allowlist qu'une
disposition (a) demanderait est vide **par construction**, pas par indulgence. Un échec signale une
régression du repli ou de la frontière. Échec CI, pas d'`#[ignore]`, pas d'exception.

**Détecteur 2 — `verdict_decoration_stripped` (exécution) : observation, pas alarme.**
Son corpus est réel : toute review décorée future. Son tir n'est **pas** un échec — c'est le
chemin de sauvetage prévu, et sa fréquence est la mesure de la dérive émetteur qu'on a choisi de
ne pas contraindre (D-A). Il ne fait échouer aucune CI et ne réveille personne. **Ce qui le fait
devenir un ticket** : le même `reviewer` sur **trois PR consécutives** — la dérive n'est alors plus
un accident de modèle mais une forme stable, et la calibration émetteur redevient discutable sur
mesure au lieu de sur intuition.

**Détecteur 3 — `verdict_approved_but_unclassified` (exécution) : halte-et-remontée opérateur.**
Corpus réel lui aussi. Son tir signifie exactement une chose : GitHub dit `APPROVED`, le moteur ne
comprend toujours pas — donc une **classe de décoration que ce plan ne couvre pas**. C'est le seul
des trois qui porte une conséquence hors CI : c'est la condition de réveil de D-D (décoration de
tête), et plus largement de toute forme suivante. Il est WARN, pas ERROR, parce que le routage
reste sûr (`hold[review]`) — mais il est nommé précisément pour être greppable par le moniteur, ce
que le WARN générique actuel ne permettait pas. **Il ne peut pas tirer sur l'existant au sens de
mika#1574** : il n'existe aucun journal rétroactif à re-classifier, le champ n'est produit qu'à
partir des reviews reçues après le déploiement. Aucun désarmement rétroactif n'est donc dû, et
c'est une propriété du flux, pas une omission.

## Hors périmètre

- **Décoration de tête** (`VERDICT: ✅ pass`). Hors périmètre par décision D-D, avec condition de
  réveil concrète : un WARN `verdict_approved_but_unclassified` dont le champ `verdict_value`
  commence par autre chose qu'un alphanumérique ASCII. Aucun ticket ouvert — sans mesure, un
  dormeur serait du bruit.
- **Calibration de l'émetteur** `mika-platform-qa` / prompt `qa-review` — hors périmètre par
  décision D-A, avec condition de réveil concrète : `verdict_decoration_stripped` sur le même
  `reviewer` pour trois PR consécutives (Fire-Disposition, détecteur 2).
- **#2237** (action GitHub `--comment` vs `--approve`) et **#2238** (BEHIND, pas d'update-branch).
  Verrous distincts du même jalon « fermeture autonome » ; aucun fichier partagé avec ce plan.
- **Le mécanisme de merge lui-même** (`handle_pass_verdict`) : ce plan lui redonne son
  déclencheur, il ne le modifie pas.

## Risques

| Risque | Portée | Atténuation |
|---|---|---|
| Le repli avale un vrai signal de fin de ligne | Un verdict mal classé = mauvais routage de PR | La queue s'arrête au premier alphanumérique ; deux tests-bornes le figent |
| `]` exempté rend un `block[…]` malformé tolérable | Faible — `BLOCK_RE` reste ancré `^…$` | Le repli passe par `classify_value`, mêmes regex qu'aujourd'hui |
| Régression sur les formes #1821/#1828 | Élevée si le repli était placé avant la passe primaire | Le repli est **après** ; les tests existants sont la garde |
