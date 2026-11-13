---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: mika#2885
branch: fix/2885/skills-un-contexte-optionnel-sans-entr-e
---

# fix(skills): un contexte optionnel sans entrée dans le message se journalise en debug, pas en WARN

## Problem Frame

`crates/mika-agent/src/skills/context.rs:178-185` journalise **tout** échec de
résolution d'un contexte de skill au même niveau :

```rust
Err(err) => {
    tracing::warn!(
        key = key.as_str(),
        context_type = context_type.as_str(),
        error = %err,
        required = required,
        "context resolution failed"
    );
```

Le type de l'erreur est `anyhow::Error` — un type **opaque** qui ne porte aucune
classe. Le site ne peut donc rien distinguer, et il journalise la même ligne pour
deux événements de natures opposées.

Mesuré le 2026-10-23 sur `/var/log/mika/server.log` (depuis le 2026-10-07) :
**565 occurrences**, et les 565 sont le même cas — `required: false`,
`key: pr_diff`, `context_type: gh_pr_diff`,
`error: "no GitHub PR URL found in message"`, presque toujours sur mika-qa. C'est
le WARN le plus fréquent des deux derniers jours.

Or ce cas est un **fonctionnement normal**. Il naît d'une seule ligne,
`context.rs:258-259` :

```rust
let (owner, repo, number) =
    extract_pr_url(message).context("no GitHub PR URL found in message")?;
```

Un skill déclare `[context.pr_diff] type = "gh_pr_diff"`, `required = false` ;
le message du tour ne contient pas d'URL de PR ; la sentinelle
`(Context unavailable: gh_pr_diff resolution failed)` est insérée à sa place
(`context.rs:191-202`), le skill n'est pas exclu, et le tour se déroule comme
prévu. Le WARN annonce un incident qui n'existe pas, 565 fois, et noie ceux qui
comptent.

Le défaut n'est pas le niveau de la ligne : c'est que **l'information de classe
n'existe nulle part**. Le résolveur rend une erreur sans classe, le site de log
en reçoit une sans classe, et le seul moyen de les trier aujourd'hui serait de
comparer le *texte* du message d'erreur — ce que le ticket interdit
explicitement, et avec raison : un littéral de prose reformulé un jour casserait
le tri en silence.

C'est du substrat de niveau 2 : rien ne casse, mais l'instrument ment par
excès. Un opérateur qui lit les WARN apprend à ignorer cette ligne, et c'est
l'apprentissage qui coûte — il se généralise aux lignes voisines.

## Requirements

- **R1** — Le résolveur doit rendre une erreur **typée**, dont une variante dit
  « le message ne contient pas l'entrée que je cherche » et l'autre « ma
  résolution a échoué ». La distinction doit être lisible par le site de log sans
  comparaison sur le texte du message.
- **R2** — Sur un contexte **optionnel**, une absence d'entrée se journalise en
  `debug!`. Aucun WARN n'est émis.
- **R3** — Tout le reste garde le `warn!` : un vrai échec de résolution sur un
  contexte optionnel, et **n'importe quel** échec sur un contexte requis.
- **R4** — Le comportement est inchangé partout ailleurs : la sentinelle est
  toujours insérée sur un contexte optionnel, les skills d'un contexte requis
  sont toujours exclus, les **noms de champs** du log restent `key`,
  `context_type`, `error`, `required`, et la valeur rendue par `error` reste
  identique octet pour octet.
- **R5** — Le défaut de classification doit être **bruyant** : une erreur dont la
  classe n'a pas été déclarée doit atterrir dans la branche `warn!`, jamais dans
  la branche silencieuse. Un futur résolveur qui oublie de classer ne doit pas
  pouvoir rendre un contexte muet par omission.
- **R6** — Une couverture de régression doit rougir contre le code actuel sur R2,
  et rester verte avant comme après sur R3.

## Key Technical Decisions

### KTD1 — Un `enum` à deux variantes, `MissingInput` et `Failed`, avec `#[from] anyhow::Error`

Forme retenue, dans `skills/context.rs` :

```rust
/// Pourquoi la résolution d'un contexte de skill n'a pas rendu de bloc.
///
/// Les deux variantes séparent deux événements de natures opposées que le type
/// `anyhow::Error` confondait (mika#2885) : une entrée absente du message est un
/// fonctionnement normal sur un contexte optionnel, un échec de résolution est
/// un incident. Le site de log de `resolve_contexts` est le seul lecteur de
/// cette distinction, et il la lit par [`Self::is_missing_input`], jamais par le
/// texte du message.
#[derive(Debug, thiserror::Error)]
pub enum ContextResolutionError {
    /// Le message du tour ne contient pas ce que le résolveur cherche.
    ///
    /// Aucun appel réseau n'a eu lieu : le résolveur s'est arrêté à l'entrée.
    #[error("{0}")]
    MissingInput(String),

    /// La résolution a réellement échoué — appel `gh` ou réseau en échec,
    /// réponse illisible, type de contexte inconnu.
    ///
    /// C'est la variante que `?` produit sur n'importe quelle `anyhow::Error`,
    /// donc **le défaut d'un résolveur qui ne classe rien** (R5) : un oubli de
    /// classification reste bruyant.
    #[error("{0}")]
    Failed(#[from] anyhow::Error),
}

impl ContextResolutionError {
    /// `true` quand le message du tour ne portait pas l'entrée du résolveur.
    pub fn is_missing_input(&self) -> bool {
        matches!(self, Self::MissingInput(_))
    }
}
```

Trois propriétés, chacune porteuse :

1. **`#[error("{0}")]` sur les deux variantes conserve R4.** Le `Display` de
   `MissingInput(String)` rend la chaîne nue, et celui de `Failed` délègue au
   `Display` de l'`anyhow` — qui rend son contexte le plus externe, exactement ce
   que `error = %err` rendait avant. La valeur du champ `error` ne bouge pas
   d'un octet, donc aucune mesure existante sur ces lignes ne se décale.
2. **`#[from] anyhow::Error` fait de `Failed` le défaut, et c'est R5.** Dans
   `resolve_gh_pr_diff`, `let diff = fetch_pr_diff(...).await?;` continue de
   compiler sans annotation et produit `Failed`. Un futur résolveur qui écrit du
   code `anyhow` ordinaire obtient la branche bruyante gratuitement ; rendre une
   ligne silencieuse demande d'**écrire** `MissingInput`, c'est-à-dire de
   l'assumer. Le sens du défaut est le livrable, pas un effet de bord.
3. **La classe est portée par le type, pas par une convention.** `MissingInput`
   n'est construit qu'à un seul endroit — l'`ok_or_else` de l'extraction d'URL —
   et le compilateur refuse qu'on la produise par accident.

**Pourquoi pas un `bool` ni un champ sur `ContextBlock`.** Un troisième argument
booléen rendu par le résolveur (`Result<ContextBlock, (anyhow::Error, bool)>`)
donnerait la même information et perdrait les deux propriétés ci-dessus : aucun
défaut (il faut toujours choisir une valeur), et aucun nom (`true` au site
d'appel ne dit pas ce qu'il classe).

**Pourquoi la variante est nommée `MissingInput` et non `NotApplicable`.** Elle
décrit ce que le résolveur a **constaté** — l'entrée n'est pas dans le message —
et non le verdict qu'on en tire. Le verdict (« silencieux ou pas ») dépend de
`required`, qui vit au site de log et pas dans l'erreur.

### KTD2 — Le site de log porte deux branches explicites, sous `!required && is_missing_input()`

```rust
Err(err) => {
    // mika#2885 — une absence d'entrée sur un contexte OPTIONNEL est un
    // fonctionnement normal : la sentinelle est insérée juste en dessous et le
    // tour se déroule. Elle part en `debug!`, sous son propre littéral.
    // Tout le reste — vrai échec, ou contexte requis — garde le `warn!` avec
    // le même message et les mêmes champs : les greps opérateur portent sur
    // ce littéral.
    if !*required && err.is_missing_input() {
        tracing::debug!(
            key = key.as_str(),
            context_type = context_type.as_str(),
            error = %err,
            required = required,
            "optional context has no input in message"
        );
    } else {
        tracing::warn!(
            key = key.as_str(),
            context_type = context_type.as_str(),
            error = %err,
            required = required,
            "context resolution failed"
        );
    }
    // … suite inchangée : exclusion si requis, sentinelle sinon …
}
```

**Pas de prédicat extrait.** Le test est `!*required && err.is_missing_input()` à
**un seul** site de décision, et son membre non trivial — `is_missing_input` — est
déjà une méthode nommée, documentée et testable. Une fonction libre
supplémentaire n'ajouterait qu'une indirection : la doctrine du dépôt refuse un
second site *décisionnel* (mika#2201), elle ne demande pas d'abstraire une
conjonction lue une fois.

**Pas de niveau dynamique.** `tracing` fige le niveau d'un callsite à la
compilation ; `if`/`else` sur deux macros est la forme idiomatique, et c'est
aussi ce qui donne deux callsites distincts — donc deux littéraux grep-ables.

### KTD3 — La branche `debug!` porte un littéral **distinct**, et c'est délibéré

Le ticket demande que « les champs du log » ne changent pas. Les **noms** des
quatre champs sont donc conservés à l'identique, ainsi que la valeur de `error`
(KTD1). Le **message**, lui, change sur la seule branche `debug!` :
`"optional context has no input in message"` au lieu de
`"context resolution failed"`.

Trois raisons, et le coût est nommé :

- **Les deux populations deviennent soustractibles.** C'est la doctrine mika#2205
  du dépôt : deux événements sous un même nom ne peuvent pas se soustraire. Avec
  deux littéraux, `grep "context resolution failed"` rend **exactement** les
  vrais échecs, et `grep "optional context has no input"` rend exactement le
  fonctionnement normal. Avec un littéral unique, répondre à « combien de vrais
  échecs ? » exigerait de filtrer aussi sur le niveau, c'est-à-dire de savoir que
  la distinction existe.
- **La ligne cesse de mentir en dev.** Un développeur sous `RUST_LOG=debug`
  lirait aujourd'hui « context resolution **failed** » pour un tour parfaitement
  sain : le bruit du ticket serait déplacé d'un niveau au lieu d'être supprimé.
- **Le littéral conservé est celui qui porte les greps.** `"context resolution
  failed"` reste le message de la branche `warn!` — celle que les mesures
  opérateur lisent — octet pour octet.

**Coût accepté :** une recherche naïve de « context resolution failed » dans les
logs de dev ne ramènera plus les absences d'entrée. C'est précisément l'effet
voulu ; et la ligne de remplacement est nommée ici et dans le commentaire du
site, donc elle est trouvable.

**Lecture post-déploiement, pour que personne ne se trompe de signal :** après
`make deploy`, `grep -c 'context resolution failed' /var/log/mika/server.log`
doit tomber à ~0 sur `key=pr_diff`. Ce zéro signifie « plus aucun faux WARN ».
Son **absence de baisse** signifierait soit un binaire antérieur au correctif
(motif mika#2340), soit de vrais échecs — et le champ `error` de chaque ligne
restante dit lequel.

### KTD4 — AC2 passe par la variante `Failed`, atteinte **sans réseau**, par le type de contexte inconnu

AC2 demande qu'un contexte optionnel dont la résolution échoue réellement
produise toujours le WARN. Le livrable sous test est la **décision du site de
log**, qui ne voit qu'une chose : `ContextResolutionError::Failed`. Un échec
réseau de `fetch_pr_diff` et un type de contexte inconnu arrivent au site sous
**la même variante, par le même bras du `match`** — le test couvre donc la
décision en entier.

Le type inconnu est atteignable sans réseau et sans horloge : il suffit qu'un
`SkillEntry` de test déclare `type = "zorglub-2885"`, et le bras `_ =>` de
`context.rs:161-164` rend `Failed`. Un échec réseau, lui, n'est pas atteignable
en test : l'URL de l'API GitHub est construite en dur dans `fetch_pr_diff`.

**Pourquoi ne pas ajouter une couture d'injection d'URL de base.** Elle serait
atteignable (`DeadEndpoint::reserve()` existe, mika#2569) mais elle coûterait un
`static` surchargeable en `#[cfg(test)]` dans un fichier de production, pour
attester un transport que **ce ticket ne touche pas**. L'angle mort est donc
assumé et écrit : le chemin réseau de `fetch_pr_diff` n'est pas exercé par ce
plan ; ce qui est exercé, et qui est le livrable, c'est que le site journalise en
WARN tout ce qui n'est pas une absence d'entrée déclarée.

Le bras du type inconnu émet **aussi** son propre `warn!("unknown context
type")`, antérieur et inchangé. Le test AC2 filtre donc sur le littéral
`"context resolution failed"`, jamais sur « il existe un WARN ».

### KTD5 — Les tests vivent dans le `mod tests` inline de `context.rs`, avec leur propre capture de niveau

`resolve_contexts` est une fonction libre `async` qui ne prend ni base de données
ni client LLM : un `#[tokio::test]` l'appelle directement, et les trois cas sous
test ne font **aucun** appel réseau (absence d'URL → arrêt à l'extraction ; type
inconnu → arrêt au `match`). Pas d'`EvalHarness`, pas de tour complet.

La capture de log doit lire le **niveau** de l'événement, ce qu'aucun des
harnais existants ne fait : `Mika2680Layer` (`server/mod.rs:6447`) et les cinq
duplications de `tests/eval/` ne retiennent que message et champs. Et les harnais
de `tests/eval/` ne sont de toute façon pas visibles depuis `src/`. Le `mod
tests` de `context.rs` porte donc une `Layer` locale qui retient
`(level, message, fields)` — la septième variante du motif, dont la mutualisation
mérite son propre ticket (déjà nommée dans
`tests/eval/test_context_scope_observability_2305.rs`) et n'est pas celui-ci.

**Doctrine de callsite, obligatoire ici (mika#2646 / mika#2680).** L'`Interest`
d'un callsite est figé par le premier thread qui l'atteint. Les deux tests
capturants portent donc `#[serial_test::serial]`, font un **tour de chauffe** —
un appel à `resolve_contexts` dont le résultat est ignoré — **avant**
d'installer l'abonné, et passent par
`crate::test_utils::test_helpers::install_capturing_subscriber`, seul site
d'installation permis.

Les `SkillEntry` de test sont construits en désérialisant un `skill.toml`
minimal (`toml::from_str::<SkillManifest>`), parce que `SkillInfo` porte
`#[serde(default)]` sur tout sauf `name` et `description` : le fixture tient en
une chaîne TOML de six lignes au lieu des quatorze champs qu'un littéral de
structure exigerait (cf. `matcher.rs:230-259`).

## Implementation Units

Ordre imposé par AC4 : **U2 d'abord**, mesure du rouge-avant, puis U1.

### U1. `ContextResolutionError` et le site de log

`crates/mika-agent/src/skills/context.rs` :

1. Déclarer `ContextResolutionError` et `is_missing_input` tels qu'en KTD1,
   juste au-dessus de `resolve_contexts`.
2. `resolve_gh_pr_diff` : signature `-> Result<ContextBlock,
   ContextResolutionError>`, et l'extraction d'URL devient
   ```rust
   let (owner, repo, number) = extract_pr_url(message).ok_or_else(|| {
       ContextResolutionError::MissingInput("no GitHub PR URL found in message".to_string())
   })?;
   ```
   La chaîne est reprise telle quelle depuis le `.context(...)` qu'elle remplace
   (R4). Le `let diff = fetch_pr_diff(...).await?;` est inchangé — `#[from]` le
   convertit en `Failed`.
3. Le bras `_ =>` du dispatch rend
   `Err(ContextResolutionError::Failed(anyhow::anyhow!("unknown context type: {}", context_type)))` ;
   son `warn!("unknown context type")` est inchangé.
4. Le site de log prend la forme de KTD2. **Rien d'autre dans le bloc `Err` ne
   bouge** : l'`if *required { exclude_indices.extend(...) } else { … sentinelle … }`
   reste à l'identique, sentinelle comprise.

L'import `use anyhow::{Context as _, Result};` reste nécessaire —
`fetch_pr_diff` continue d'utiliser `.context("failed to read response body")` et
de rendre `anyhow::Result`.

**Sites explicitement NON touchés :** `truncate_diff`, `parse_diff_files`,
`fetch_pr_diff`, `apply_context_replacements`, `ContextBlock`, `ContextStatus`,
`extract_pr_url` et le `warn!` de conflit de types (`context.rs:124-130`).

### U2. Les quatre tests, dans le `mod tests` de `context.rs`

Harnais (KTD5) : une `Mika2885Layer` retenant `(level, message, fields)`, un
`Mika2885Visitor`, une `mika2885_capture()` posée par
`install_capturing_subscriber`, et un `mika2885_entry(key, context_type,
required) -> SkillEntry` construit par `toml::from_str`.

1. `mika2885_une_absence_dentree_sur_contexte_optionnel_ne_produit_aucun_warn`
   — **AC1**, `#[tokio::test] #[serial_test::serial]`. Entrée :
   `("pr_diff", "gh_pr_diff", required = false)`, message
   `"review the plan please"` (aucune URL). Trois assertions :
   - aucun événement capturé ne porte `"context resolution failed"` ;
   - un événement de niveau `DEBUG` porte
     `"optional context has no input in message"` et les quatre champs
     `key = "pr_diff"`, `context_type = "gh_pr_diff"`,
     `error = "no GitHub PR URL found in message"`, `required = "false"` ;
   - `resolved["pr_diff"].content` commence par `"(Context unavailable:"` et
     `exclude_indices` est vide (la sentinelle est toujours insérée, le skill
     n'est pas exclu).
2. `mika2885_un_vrai_echec_sur_contexte_optionnel_produit_toujours_le_warn`
   — **AC2**, `#[tokio::test] #[serial_test::serial]`. Entrée :
   `("pr_diff", "zorglub-2885", required = false)`. Assertions : un événement de
   niveau `WARN` porte `"context resolution failed"` (filtre sur le littéral,
   KTD4) ; la sentinelle est insérée ; `exclude_indices` est vide.
3. `mika2885_un_contexte_requis_qui_echoue_produit_le_warn_et_exclut_ses_skills`
   — **AC3**, `#[tokio::test] #[serial_test::serial]`. **Deux** cas dans la même
   capture, parce que AC3 dit « quelle qu'en soit la raison » et qu'un seul des
   deux ne le dirait pas :
   - `("pr_diff", "gh_pr_diff", required = true)` sur un message sans URL —
     c'est-à-dire une `MissingInput` **requise**, le cas qu'une lecture trop
     rapide du correctif rendrait muet ;
   - `("pr_diff", "zorglub-2885", required = true)` — un `Failed` requis.
     Pour chacun : un `WARN` portant `"context resolution failed"` avec
     `required = "true"`, et `exclude_indices == vec![0]`. Aucune sentinelle
     n'est insérée (`resolved` est vide).
4. `mika2885_une_erreur_anyhow_non_classee_nest_pas_une_absence_dentree`
   — **R5**, test pur, sans capture :
   `assert!(!ContextResolutionError::from(anyhow::anyhow!("boom")).is_missing_input())`
   et `assert!(ContextResolutionError::MissingInput("x".into()).is_missing_input())`.
   Il épingle le **sens du défaut** de KTD1 : si quelqu'un inversait
   `is_missing_input` ou donnait `MissingInput` à `#[from]`, les tests 1 à 3
   resteraient verts pour le résolveur actuel et tout futur résolveur
   deviendrait muet par omission.

## Verification Contract

| # | commande | attendu |
|---|---|---|
| V1 | `cargo test -p mika-agent --lib skills::context` | 4 nouveaux tests verts, 19 existants verts |
| V2 | `cargo test -p mika-agent` | suite entière verte |
| V3 | `cargo clippy --all-targets -- -D warnings` | aucun avertissement |
| V4 | `cargo fmt --check` | propre |
| V5 | `scripts/verify-pipeline.sh origin/main` | vert |
| V6 | rouge-avant (AC4) : U2 commité seul, `cargo test -p mika-agent --lib skills::context` | `…ne_produit_aucun_warn` **rouge** (le code actuel émet le WARN) ; les trois autres **verts** |

V6 est une mesure, pas une intention : elle est exécutée entre U2 et U1, et sa
sortie est reportée dans le corps de la PR.

**Gate Fire-Disposition (mika#2306) : N/A, et voici le raisonnement.** Aucun
livrable de ce plan n'a pour fonction primaire de signaler une violation dans un
corpus. Les quatre tests sont des assertions **positives** sur un site unique —
« le site a émis en `debug` avec ces champs », « le site a émis en `warn` et a
exclu le skill », « le défaut de conversion est `Failed` » — et leur chemin de
succès est « le site a fait ce qu'on attend », non « aucune violation trouvée ».
C'est la même classe que U2 du plan mika#2876, explicitement rangée hors
détecteur par ce plan. Aucun scan de source, aucune règle de lint, aucune garde
CI, aucune allowlist n'est livrée — la propriété structurelle du ticket (« la
distinction vient d'un type, pas d'une comparaison de texte ») est tenue par le
**compilateur** via l'`enum` de KTD1, ce qui est plus fort qu'un scan et n'a
donc pas besoin d'être doublé par un.

## Definition of Done

- `ContextResolutionError` existe, est rendu par `resolve_gh_pr_diff`, et son
  `Display` rend les mêmes octets que l'`anyhow::Error` qu'il remplace.
- Le site de log de `resolve_contexts` porte deux branches, sous
  `!*required && err.is_missing_input()`.
- La sentinelle, l'exclusion des skills sur contexte requis et les quatre noms de
  champs sont inchangés ; seul le littéral de message de la branche `debug!` est
  nouveau, et KTD3 en nomme le coût.
- Les quatre tests de U2 sont verts, et V6 atteste le rouge-avant d'AC1.
- V1 à V5 verts.
- Le corps de la PR porte la sortie de V6 et la ligne de lecture
  post-déploiement de KTD3.

## Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#2885 :

- **AC1** : un contexte optionnel `gh_pr_diff` sur un message sans URL de PR ne
  produit aucun WARN. La sentinelle est toujours insérée.
- **AC2** : un contexte optionnel dont la résolution échoue réellement produit
  toujours le WARN.
- **AC3** : un contexte requis qui échoue, quelle qu'en soit la raison, produit
  toujours le WARN et exclut ses skills.
- **AC4** : rouge-avant, AC1 rougit contre le code actuel ; AC2 et AC3 restent
  verts avant comme après.

Correspondance : AC1 → U2 test 1 ; AC2 → U2 test 2 ; AC3 → U2 test 3 (deux cas) ;
AC4 → V6.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/skills/context.rs` — `ContextResolutionError` + `is_missing_input` + doc | 40 |
| `crates/mika-agent/src/skills/context.rs` — signature de `resolve_gh_pr_diff`, `ok_or_else`, bras du type inconnu | 10 |
| `crates/mika-agent/src/skills/context.rs` — site de log, deux branches + commentaire | 26 |
| tests U2 — harnais de capture avec niveau + fixture `SkillEntry` | 80 |
| tests U2 — les quatre tests | 90 |

Total estimé : 246 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 246 estimées contre une borne de 279
— 33 lignes de marge, ~12 %. Trois raisons rendent cette estimation peu
susceptible de déraper comme celles de mika#2683 (832 estimées / 1 432 réelles) :

1. **Un seul fichier, et aucune signature publique traversante.** Tout vit dans
   `crates/mika-agent/src/skills/context.rs`. `resolve_gh_pr_diff` est une
   fonction **privée** au module : son changement de type de retour n'atteint
   aucun appelant hors du fichier (vérifié — `grep resolve_gh_pr_diff crates/`
   ne rend que ce fichier). `resolve_contexts` garde sa signature exacte, donc
   ses deux appelants (`agent_loop/mod.rs:5874` et `:8028`) ne sont pas touchés.
2. **Le correctif de production fait 76 lignes dont ~40 de documentation du
   type.** Pas de nouveau module, pas de nouvelle dépendance (`thiserror` est
   déjà au `Cargo.toml`), pas de migration, pas de couture d'injection (KTD4 la
   refuse explicitement — c'était la seule voie par laquelle ce ticket pouvait
   enfler).
3. **Les tests n'ont besoin ni de base de données, ni de LLM, ni de réseau, ni
   de `TempDir`.** `resolve_contexts` est une fonction libre ; les trois cas
   s'arrêtent avant tout appel HTTP. Le poste le plus gros — les 80 lignes de
   harnais — est du code dont la forme est **connue d'avance** : il est copié de
   `Mika2680Layer` (`server/mod.rs:6447-6496`) avec un champ `level` en plus, et
   le fixture `SkillEntry` est une chaîne TOML de six lignes plus neuf champs
   littéraux recopiés de `matcher.rs:239-259`.

## Scope Boundaries

**Dans le périmètre :** le type d'erreur du résolveur de contextes, le niveau et
le littéral de la ligne de log d'un échec de résolution, et leur couverture.

**Hors périmètre, délibérément :**

- **Le transport de `fetch_pr_diff`.** Aucune couture d'injection d'URL de base
  n'est ajoutée (KTD4), donc le chemin réseau réel n'est pas exercé. L'angle mort
  est nommé ici et dans le doc-comment de `ContextResolutionError::Failed`.
- **Le `warn!` de conflit de types de contexte** (`context.rs:124-130`). Il
  signale une incohérence de manifestes entre deux skills — un vrai défaut de
  configuration, qui doit rester bruyant. Zéro occurrence mesurée dans les logs.
- **Le second résolveur.** `KNOWN_CONTEXT_TYPES` n'en porte qu'un
  (`gh_pr_diff`) ; rien n'est ajouté. Ce que ce plan laisse derrière lui, pour le
  prochain, c'est le **sens du défaut** (R5 / KTD1) : un résolveur neuf est
  bruyant tant qu'il n'a pas déclaré son silence.
- **Le niveau de log de production.** Aucun changement à la configuration
  `tracing` : la ligne `debug!` n'est simplement pas émise sous le filtre INFO de
  production, et c'est tout le bénéfice du ticket.

## Sources & Research

- Ticket : senara-solutions/mika#2885 (corps mesuré le 2026-10-23 sur
  `/var/log/mika/server.log`, 565 occurrences depuis le 2026-10-07).
- Site du défaut : `crates/mika-agent/src/skills/context.rs:178-185` (le `warn!`)
  et `:258-259` (le `.context(...)` qui produit l'absence d'entrée).
- Appelants de `resolve_contexts` : `crates/mika-agent/src/agent_loop/mod.rs:5874`
  et `:8028` — tous deux inchangés.
- Motif de capture de log réutilisé : `crates/mika-agent/src/server/mod.rs:6447-6496`
  (`Mika2680Layer`), et la doctrine de callsite de
  `crates/mika-agent/src/test_utils.rs:21-71`
  (`install_capturing_subscriber`, mika#2646).
- Fixture `SkillEntry` : `crates/mika-agent/src/skills/matcher.rs:230-259`.
- Doctrine de populations soustractibles invoquée en KTD3 : mika#2205.
- Doctrine du second site décisionnel invoquée en KTD2 : mika#2201.
- Classification « assertion positive ≠ détecteur » invoquée au Verification
  Contract : `docs/plans/2026-10-23-001-fix-2876-journaliser-la-chaine-complete-echec-init-agent-plan.md`
  § Fire-Disposition.
