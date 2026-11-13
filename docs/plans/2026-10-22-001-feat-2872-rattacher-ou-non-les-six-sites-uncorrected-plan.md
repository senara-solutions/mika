# mika#2872 — Rattacher ou non les six sites `_uncorrected` à la classe P0 `fabrication-detected`

> **Vol 1 sur 2.** Ce vol porte les **six verdicts** (propositions soumises à la
> ratification de Vincent) et l'**implémentation du seul site dont le verdict est
> entier** : `false_local_hosting_claim`. Le second site rattaché l'est sur une
> moitié seulement de son domaine, et son périmètre n'est pas connaissable avant
> la ratification — il part en vol 2. `Tracked in:` en fin de § *Taille estimée*.

## Ce que ce vol décide, et ce qu'il ne décide pas

mika#2867 (PR #2870) a rattaché **un** garde à la classe P0
`fabrication-detected` : la récidive d'`assert_grounded`. Six autres sites de la
famille `_uncorrected` ont leur `warn!` et aucun étage de notification. La
question ouverte n'est pas d'implémentation — la forme est écrite et attestée —
mais de **produit** : chaque P0 coûte un réveil d'opérateur, et un réveil sans
geste à faire détruit la crédibilité de la surface jamais muette.

Ce vol **propose** six verdicts. Il n'en ratifie aucun. La retenue est
mécanique, pas déclarative : la PR s'ouvre en `--draft` et y reste. Vincent la
sort lui-même du draft, site par site. Si sa ratification écarte un site, le
site et ses tests sortent de la PR **avant** ce passage.

---

## D1 — Le critère, rendu décidable

Le bearing de Prime (2026-10-22) nomme deux formes :

> « Chaque site se juge sur sa forme : un état **attesté** (le garde voit un fait
> vérifiable) ou un état **posé comme attesté** (la réponse affirme un état
> qu'aucun résultat d'outil ne porte). Pas de verdict en gros. »

Le bearing nomme les deux formes sans dire laquelle est la fabrication. Deux
ancres indépendantes du corps du ticket la fixent :

1. Le site déjà rattaché y est décrit comme « **affirmer un état système aval
   sans résultat d'outil** » — c'est littéralement la seconde forme.
2. La dérive de langue y est explicitement **exclue** (« Une dérive de langue,
   par exemple, n'est pas une fabrication d'état système ») — et c'est le cas
   pur de la première forme : le garde tient un fait vérifiable
   (`tenant_language`) et mesure une conformité contre lui.

D'où **la question discriminante**, celle qui est posée à chacun des six sites
et dont la réponse EST le verdict :

> **La phrase fautive affirme-t-elle un état du système qu'aucun résultat
> d'outil de ce tour ne porte ?**

- **Oui** → `fabrication-detected`. Personne dans le système ne sait si c'est
  vrai ; seul un humain peut l'établir ; le réveil a un geste.
- **Non** → hors classe. La faute est une divergence de **forme**, de
  **registre** ou de **conduite** mesurée contre un fait que le garde tient
  déjà — ou un **engagement sur l'avenir**, ou une **omission**. Aucun état
  n'est posé, donc il n'y a rien à aller vérifier.

Ce que la question **ne** demande pas : que le garde ignore la vérité. Un garde
qui voit que la phrase est **fausse** (5d voit `deployment`) ne cesse pas pour
autant de voir une phrase qui **pose** un état qu'aucun outil ne porte. « Un
état attesté » décrit une réponse que le garde peut **confirmer** — pas une
réponse qu'il peut réfuter.

---

## D2 — Les six verdicts proposés

| # | Site (`agent_loop/mod.rs`) | Branche `_uncorrected` | La phrase fautive affirme-t-elle un état système non porté par un outil ? | Verdict proposé |
|---|---|---|---|---|
| 1 | `false_local_hosting_claim` (5d, `2797-2855`) | `2868-2887`, événement `:2883` | **Oui.** « je tourne en local sur ta machine » est un état du déploiement. Aucun résultat d'outil ne le porte — et `tool_ctx.deployment` le **contredit**. | **RATTACHÉ** |
| 2 | `unactioned_frequency_promise` (5e, `2915-2981`) | `2991-3009`, événement `:3005` | **Les deux, selon l'alternative appariée** (voir D3). `je vais réduire la fréquence` → non (avenir). `c'est corrigé` / `i've stopped` → oui (état accompli, et le terme (C) du détecteur garantit qu'aucun `set_config` n'a eu lieu). | **RATTACHÉ SUR LA MOITIÉ ACCOMPLIE SEULEMENT** — vol 2 |
| 3 | `response_language_drift` (5f, `3032-3084`) | `3096-3113`, événement `:3109` | **Non.** La réponse répond dans une langue ; elle n'affirme aucun état. Le garde tient `tenant_language`, un fait vérifiable, et mesure une conformité. | **NON RATTACHÉ** |
| 4 | `time_of_day_greeting_mismatch` (5g, `3129-3182`) | `3186-3205`, événement `:3201` | **Non.** « bonsoir » à 9 h est une divergence de registre contre l'horloge — le fait le plus vérifiable qui soit, et le garde l'a sous la main. Aucun état aval n'est posé. | **NON RATTACHÉ** |
| 5 | `testimony_access_proposal` (5h, `3256-3329`) | `3341-3365`, événement `:3361` | **Non.** `ACCESS_OPENING_ALTERNATION` (`guards.rs:1830-1835`) ne contient que des **mouvements** : « donne-moi accès », « let me access », « if I had access », « with access to ». Aucune alternative n'affirme un accès **existant**. C'est une faute de conduite contre une doctrine, pas un état posé. | **NON RATTACHÉ** |
| 6 | `unacknowledged_send_failure` (6f, `3865-3902`) | `3915-3949`, événement `:3929` + une ligne `audit_events` | **Non.** `undelivered_sends` (`guards.rs:2707`) est « entirely structural — no lexicon, no reading of the assistant's text » : la faute est une **omission d'accusé**, pas une affirmation. Et le `DeliveryRecord` **est** le fait attesté. | **NON RATTACHÉ** (voir la note ci-dessous) |

### Note sur le site 6, parce qu'elle est le seul verdict qui nomme une autre classe

`unacknowledged_send_failure` porte un **marqueur durable** — la ligne
`audit_events` de sa propre branche, et la table `failed_sends`. C'est la forme
exacte de la classe `recovery-runbook` (mika#2866, `wip_rescue.rs:1-80` :
« Le marqueur durable EST la population de cette classe »), pas celle de
`fabrication-detected`.

Ce vol **ne le rattache pas à `recovery-runbook`** et ne l'instruit pas :
la borne du ticket est « Rien d'autre », et choisir une classe pour un site est
exactement la décision de produit que ce ticket soumet à ratification — une à
la fois. Le verdict proposé est donc *non rattaché à `fabrication-detected`*,
avec sa parenté nommée pour que la prochaine lecture ne reparte pas de zéro.

### Pourquoi le site 2 n'a pas un verdict entier

`FREQUENCY_ASSERTION_ALTERNATION` (`guards.rs:1070-1083`) tient **treize**
alternatives, de deux natures :

| nature | alternatives | état posé ? |
|---|---|---|
| engagement sur l'avenir | `je vais {corriger,régler,…}`, `je {corrige,règle,…}`, `je n'enverrai plus`, `i'll {fix,stop,…}`, `i will {fix,…}`, `i {won't,will not} send` | **non** |
| état accompli | `c'est {corrigé,réglé,fait,bon,réparé}`, `i've {fixed,stopped,reduced,changed,adjusted,limited,cut}`, `i have {…}`, `{it's,that's,it is} {fixed,done,sorted}` | **oui** |

Le doc-comment du garde ne tranche pas pour nous : il dit que le garde refuse
« a **commitment about the future** that the turn did nothing to bring about »
(`agent_loop/mod.rs:2904-2907`), ce qui décrit la première moitié seulement, et
il renvoie à la famille assert-grounded (mika#1331) le cas d'un effet
**contredit par un résultat d'outil** — pas le cas d'un effet affirmé avec
**zéro** appel d'outil, qui est précisément ce que le terme (C) du détecteur
garantit.

Rendre un verdict unique sur ce site serait un « verdict en gros » au sens que
Prime refuse, et mettrait **deux populations sous un seul nom d'événement** —
la classe que la doctrine mika#2205 déclare non soustractible. Le verdict est
donc scindé, et sa moitié rattachée exige un sous-prédicat nommé
(`frequency_assertion_poses_accomplished_state`) que ce vol ne porte pas : voir
D7.

---

## D3 — Le créneau `pending_fabrication_p0` devient typé et discriminé

**Le défaut à corriger avant tout ajout de site.** `pending_fabrication_p0`
(`agent_loop/mod.rs:1658`) est un `Option<String>`. Deux choses en découlent, et
les deux mordent dès le second site :

1. Un second site qui écrit dedans **écrase silencieusement** le premier.
2. La revérification à l'acceptation (`4452-4464`) est **câblée en dur** sur
   `detect_affirmative_state_claim` + `assert_grounded_satisfied`. Un message
   posté par 5d serait revérifié par le prédicat d'`assert_grounded` — donc
   supprimé sur un texte qui porte pourtant toujours la fausse revendication de
   localité, ou émis sur un texte qui ne la porte plus. Dans les deux sens, le
   silence est faux.

**Décision.** Le créneau devient
`Option<PendingFabricationP0>` avec

```rust
pub(crate) struct PendingFabricationP0 {
    pub(crate) message: String,
    pub(crate) site: PendingFabricationP0Site,
}

pub(crate) enum PendingFabricationP0Site {
    AssertGrounded,
    FalseLocalHosting,
}
```

et **une seule** méthode décisionnelle, dans `evidence/guards.rs` à côté des
prédicats qu'elle appelle :

```rust
impl PendingFabricationP0Site {
    /// Le texte **accepté** porte-t-il encore la violation que ce site a posée ?
    pub(crate) fn still_violated(
        self,
        text: &str,
        summaries: &[ToolCallSummary],
        deployment: Deployment,
    ) -> bool
}
```

Le `match` à deux bras y vit, pas dans la boucle : le site d'acceptation
redevient trois lignes et la revérification est **une propriété du type**. Un
troisième site ne compile pas sans nommer son bras — « j'ai oublié de
revérifier » n'est plus un état atteignable, exactement la forme que mika#1381
a imposée à `send_notification` pour l'étage.

**Un seul créneau, donc une seule P0 par tour pour toute la classe.** Le verrou
du ticket (« un verrou d'une notification par tour ») est lu à l'échelle de la
**classe**, pas du site : chaque branche n'écrit que si
`pending_fabrication_p0.is_none()`. Le premier écrivain gagne, et l'ordre est
celui de la chaîne — 5d (`:2868`) avant 5e (`:2991`) avant assert-grounded
(`:3694`).

*Coût nommé.* Un tour qui viole 5d **et** assert-grounded ne réveille
l'opérateur qu'une fois, pour 5d. Rien n'est perdu : la notification porte le
`trace_id`, et tous les `_uncorrected` du tour sont sur cette trace dans
`/var/log/mika/server.log`. L'alternative — un créneau par site — rendrait
possible trois réveils pour un seul tour, ce qui est le mode d'extinction d'une
surface jamais muette. Le verrou par site de #2867
(`assert_grounded_uncorrected_notified`) **reste** : il borne les `EndTurn`
successifs d'un même tour, ce que le verrou de classe ne fait pas.

---

## D4 — Le site 1, sur la forme de #2870

Dans la branche `2868-2887`, après le `warn!` existant (inchangé) :

```rust
if fabrication_p0_notify && pending_fabrication_p0.is_none() {
    pending_fabrication_p0 = Some(PendingFabricationP0 {
        message: format!(
            "False local-hosting claim (uncorrected): the guard fired and the \
             re-prompt budget is spent, yet the response still claims «{}» about \
             «{}» while the resolved deployment is {:?} — NOT local. The answer \
             went out to the tenant. Investigate trace_id {} in \
             /var/log/mika/server.log.",
            hosting.assertion, hosting.subject, tool_ctx.deployment, tool_ctx.trace_id,
        ),
        site: PendingFabricationP0Site::FalseLocalHosting,
    });
}
```

Et à l'acceptation (`4452-4464`), le bloc de #2867 devient :

```rust
if let Some(pending) = pending_fabrication_p0.take()
    && pending.site.still_violated(&text, &all_tool_summaries, tool_ctx.deployment)
    && let Some(ref sender) = tool_ctx.message_sender
{
    let _ = sender
        .send_with_severity(&pending.message, NotificationSeverity::P0)
        .await;
}
```

Quatre propriétés de la forme de #2870, toutes reprises telles quelles :

| propriété | où |
|---|---|
| porte désarmée par défaut | `fabrication_p0_notify`, résolu aux trois entrées de `run_loop` par `params.settings.is_some_and(…)` (`:6312`, `:7396`, `:8103`) — `is_some_and`, donc un `Settings` absent laisse la porte **fermée** |
| `send_with_severity(…, P0)` | site d'acceptation, étage en seconde position et **sans défaut** |
| verrou d'une notification par tour | `pending_fabrication_p0.is_none()` (classe) + `..._notified` de #2867 (site) |
| revérification à l'acceptation | `still_violated`, parce que six gardes aval peuvent encore refuser la clôture et obtenir un texte qui ne porte plus la revendication |

**Pourquoi `Deployment::Unknown` suffit au test positif.**
`detect_false_local_hosting_claim` (`guards.rs:903-939`) ne s'exempte que pour
`Deployment::Local` — « a declared local install is telling the truth » — et
**pas** pour `Unknown`, qui est « the state every cloud tenant is in today, and
it is the state the measured incident happened in ». `EvalHarness` sert déjà
`Unknown` (`agent_loop/mod.rs:21272`), donc le harnais n'a **aucun** setter à
gagner.

---

## D5 — Les quatre sites non rattachés ne sont pas touchés

Aucune ligne de `3096-3113`, `3186-3205`, `3341-3365`, `3915-3949`. Pas de
`warn!` déplacé, pas de champ ajouté, pas de test écrit. Un site proposé comme
non rattaché dont le diff montrerait une modification serait un verdict rendu
par le code au lieu d'être rendu par Vincent.

---

## D6 — La PR s'ouvre en draft et y reste

`gh pr create --draft`. Le corps porte les **six** verdicts, un par site, chacun
avec sa raison tirée de la forme — le tableau D2, transposé. Un draft ne se
merge pas : c'est la retenue, et elle est mécanique.

Le vol **ne sort pas** la PR du draft, ne demande pas la ratification dans un
commentaire, et ne reporte rien dans le ticket. L'AC1 est remplie par la
ratification de Vincent, que l'orchestrateur reporte.

---

## Phase unique — l'ordre de commit

1. **`evidence/guards.rs`** — `PendingFabricationP0`,
   `PendingFabricationP0Site`, `still_violated` avec son doc-comment (le critère
   D1 et le coût du verrou de classe y sont écrits, pas dans ce plan seul).
   `cargo check -p mika-agent`.
2. **`agent_loop/mod.rs:1658`** — le créneau passe de `Option<String>` à
   `Option<PendingFabricationP0>`.
3. **`agent_loop/mod.rs:3742`** — la branche de #2867 remplit le nouveau type
   avec `site: AssertGrounded` et gagne le terme `pending_fabrication_p0.is_none()`.
4. **`agent_loop/mod.rs:4452`** — le site d'acceptation délègue à
   `still_violated`. `cargo test -p mika-agent --test eval
   test_fabrication_detected_p0_2867` : **les trois tests de #2867 doivent
   rester verts sans modification**. C'est le plancher de ce vol — un refactor
   qui les touche a changé un comportement.
5. **`agent_loop/mod.rs:2887`** — la branche 5d gagne son dépôt (D4).
6. **`tests/eval/test_fabrication_detected_p0_2872.rs`** — les trois tests, puis
   `pub mod test_fabrication_detected_p0_2872;` dans `tests/eval.rs`
   (`test_eval_modules_declared` rougit sur tout fichier non déclaré).
7. `cargo fmt`, `cargo clippy`, `cargo test -p mika-agent`, les lints du dépôt,
   `scripts/verify-pipeline.sh origin/main`. Les recettes de rouge-avant de la
   § *Risques* sont jouées **avant** l'ouverture de la PR, et leur résultat écrit
   dans l'en-tête du fichier de test.

---

## Fire-Disposition

**Option (b) — livrer désarmé, avec un suivi tracké pour l'armer.**

Le livrable primaire de ce vol est un détecteur au sens de la règle mika#2306 :
son chemin de succès est « aucune violation trouvée », et il atterrit **inerte
en production**.

| moitié | état à l'atterrissage | mécanisme |
|---|---|---|
| le dépôt dans la branche 5d | n'écrit rien | `fabrication_p0_notify` est `false` par défaut (`config.rs:2807`), `#[serde(default)]`, et résolu par `params.settings.is_some_and(…)` — un `Settings` absent laisse la porte **fermée** |
| l'envoi à l'acceptation | ne part pas | le créneau est `None`, donc le `if let Some(…)` ne s'ouvre jamais |

La porte est **délibérément absente de `SETTABLE_CONFIG_KEYS`** : l'armement
n'est pas une bascule qu'un agent peut faire pour lui-même avec `set_config`,
c'est une décision d'opérateur (`MIKA_FABRICATION_P0_NOTIFY`, ou la clé dans
`config.toml`). Rien dans ce vol ne la change.

**Une seule porte pour toute la classe**, réutilisée telle quelle. Un
`MIKA_FABRICATION_P0_NOTIFY_LOCAL_HOSTING` distinct serait une porte par site,
donc une matrice d'états dont aucune sonde ne dirait laquelle est en vigueur.

**Suivi d'armement : mika#2871**, qui porte l'armement de la P0 — la borne de ce
ticket le dit en propres termes (« Les portes restent désarmées par défaut ;
armer la P0 relève de mika#2871 »). Aucun test n'est livré avec `#[ignore]` : la
porte **est** le désarmement, et les tests l'ouvrent explicitement par
`.fabrication_p0_notify(true)`, ce qui les rend verts et probants sans armer
quoi que ce soit en production. Le test à porte fermée est le contrôle positif
de ce désarmement.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/evidence/guards.rs` (`PendingFabricationP0`, l'enum de site, `still_violated` + doc-comments) | 55 |
| `crates/mika-agent/src/agent_loop/mod.rs` (créneau typé, `match` d'acceptation délégué, branche 5d) | 60 |
| tests (`crates/mika-agent/tests/eval/test_fabrication_detected_p0_2872.rs`) | 150 |
| `crates/mika-agent/tests/eval.rs` (déclaration du module) | 1 |

Total estimé : 266 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient sous la borne, et pourquoi il la frôle.** Quatre
arguments de marge structurelle, tous vérifiés plutôt que supposés :

1. **Toute l'infrastructure partagée est déjà payée.** La clé de configuration
   (`config.rs:352`, `:1268`, `:2807`, `:2947`), la variable d'environnement, le
   setter du harnais (`tests/eval/harness.rs:447`, `:641`), le
   `SeverityRecordingSender`, l'`Emission` et le sentinelle `SEVERITE_ABSENTE`
   (mika#1381 vol 3) existent et sont importés, pas réécrits. Ce vol ne paie que
   le discriminant et **un** site.
2. **Aucun nouveau prédicat de détection.**
   `detect_false_local_hosting_claim` existe, est `pub(crate)`, et prend déjà
   `(text, deployment)` — les deux valeurs disponibles au site d'acceptation.
3. **Aucun setter de harnais à gagner.** `Deployment::Unknown` est le défaut du
   harnais et n'est pas exempté par le détecteur (D4).
4. **Quatre sites sur six sont à zéro ligne** par construction : un verdict
   « non rattaché » est un diff vide (D5).

La marge est de 13 lignes, soit 5 %. Elle est étroite **parce que le refactor du
créneau (D3) est non négociable** : sans lui, le second site écrase le premier
et la revérification lit le mauvais prédicat. Le découper en deux vols coûterait
plus que de le payer ici — un vol dont le seul livrable est un type sans second
consommateur n'est pas mergeable seul au sens de la règle mika#2693.

**Tracked in: senara-solutions/mika#2872** — le vol 2 porte la moitié accomplie
du site 2 : le sous-prédicat
`frequency_assertion_poses_accomplished_state` (≈ 30 lignes), la branche 5e
(≈ 25) et ses trois tests (≈ 90), soit ≈ 145 lignes. Il reste sur ce ticket, et
son périmètre n'est **pas** connaissable avant la ratification : si Vincent
écarte le site 2, le vol 2 n'existe pas ; s'il le ratifie entier plutôt que sur
sa moitié, le sous-prédicat disparaît et la branche est celle de D4 copiée.
L'orchestrateur ouvre le ticket du vol 2 quand la ratification a tranché — pas
avant, pour ne pas ouvrir un ticket dont la borne est inconnue.

---

## Acceptance criteria

Transcrits verbatim du corps de mika#2872 :

- **AC1** : la décision par site est écrite dans ce ticket : rattaché ou non,
  avec sa raison.
- **AC2** : chaque site rattaché a un test positif, un test à porte fermée et
  ses recettes de rouge-avant, comme `test_fabrication_detected_p0_2867.rs`.

Critères dérivés, concrets et testables, de la borne du vol (points 1 à 4 du
corps) :

- **AC3** — Les six verdicts figurent dans le corps de la PR, **un par site**,
  chacun avec sa raison tirée de la forme du site. Un verdict sans raison, ou
  une raison qui n'est pas une propriété de la forme, ne satisfait pas AC3.
- **AC4** — La PR est ouverte avec `gh pr create --draft` et est **encore en
  draft** à la fin de la session. Vérifiable : `gh pr view <n> --json isDraft`
  rend `true`.
- **AC5** — Les quatre sites proposés comme non rattachés ne sont pas touchés.
  Vérifiable : `git diff origin/main -- crates/mika-agent/src/agent_loop/mod.rs`
  ne montre aucune ligne dans `3096-3113`, `3186-3205`, `3341-3365`,
  `3915-3949`.
- **AC6** — `false_local_hosting_claim` porte un test positif qui atteste
  l'étage **au transport** : `assert_ne!(*etage, SEVERITE_ABSENTE)` **et**
  `assert_eq!(*etage, Some(NotificationSeverity::P0))`. Les deux assertions,
  parce que `send_with_severity` a un corps par défaut qui délègue à
  `self.send(text)` : un site qui régresse sur `send()` perd son étage **sans
  erreur de compilation**, et seule la première assertion le voit.
- **AC7** — `false_local_hosting_claim` porte un test à **porte fermée**
  (`.fabrication_p0_notify(…)` non appelé, donc le défaut `false` du harnais)
  qui atteste que le transport n'observe **aucune** émission. C'est le contrôle
  positif de la Fire-Disposition (b).
- **AC8** — La notification porte le `trace_id` du tour et le déploiement
  résolu. Sans eux, le réveil n'a pas de geste.
- **AC9** — La revérification à l'acceptation est attestée : un test où une
  garde aval reprend la main et obtient un texte ne portant plus la fausse
  revendication de localité n'émet **rien**.
- **AC10** — Exactement **une** émission par tour pour toute la classe
  (`assert_eq!(emissions.len(), 1)`).
- **AC11** — Les trois tests de `test_fabrication_detected_p0_2867.rs` restent
  verts **sans modification**. Le refactor du créneau (D3) ne change aucun
  comportement de #2870.
- **AC12** — Les recettes de rouge-avant de la § *Risques* sont jouées, et leur
  résultat — y compris celles qui restent **vertes**, avec leur raison — est
  écrit dans l'en-tête de doctrine du fichier de test.

---

## Definition of Done

- [ ] Les six verdicts sont écrits, un par site, avec leur raison tirée de la
      forme — dans ce plan (D2) **et** dans le corps de la PR.
- [ ] `PendingFabricationP0` / `PendingFabricationP0Site` / `still_violated`
      livrés dans `evidence/guards.rs`, avec le critère D1 et le coût du verrou
      de classe dans leurs doc-comments.
- [ ] Le créneau de `agent_loop/mod.rs` est typé ; les deux bras du `match`
      sont nommés ; le site d'acceptation ne contient plus aucun prédicat en dur.
- [ ] La branche 5d dépose sa P0 derrière `fabrication_p0_notify` **et**
      `pending_fabrication_p0.is_none()`.
- [ ] `test_fabrication_detected_p0_2872.rs` livré avec ses trois tests et son
      en-tête de doctrine, déclaré dans `tests/eval.rs`.
- [ ] Les trois tests de #2867 verts, non modifiés.
- [ ] Les quatre sites non rattachés à diff vide.
- [ ] Recettes de rouge-avant jouées, résultats écrits dans l'en-tête du test.
- [ ] `cargo fmt`, `cargo clippy`, `cargo test -p mika-agent`, les lints du
      dépôt et `scripts/verify-pipeline.sh origin/main` verts.
- [ ] **La PR est ouverte en `--draft` et l'est encore à la remise.** Le vol ne
      la sort pas du draft. Vincent le fait, site par site, après ratification ;
      si sa ratification écarte un site, le site et ses tests sortent de la PR
      avant ce passage.
- [ ] Aucune porte armée. `MIKA_FABRICATION_P0_NOTIFY` reste à son défaut et
      `SETTABLE_CONFIG_KEYS` est inchangé.
- [ ] `Code review: delegated to mika-qa (mika#2694)` dans le corps de la PR,
      et `Tracked in: senara-solutions/mika#2872` pour le vol 2.

---

## Risques

### R1 — Les recettes de rouge-avant, et celles qui restent vertes

Un test qui n'a jamais été rouge n'atteste rien. Chaque recette est une
injection jouée sur l'arbre, le test lancé, l'injection retirée.

| # | injection | ce qui doit rougir |
|---|---|---|
| i | `NotificationSeverity::P0` → `P2` au site d'acceptation | `assert_eq!(*etage, Some(P0))` du test positif |
| ii | `sender.send_with_severity(&m, P0)` → `sender.send(&m)` | `assert_ne!(*etage, SEVERITE_ABSENTE)` — **la seule** qui voit cette régression, puisqu'elle ne produit aucune erreur de compilation |
| iii | le bloc de dépôt de la branche 5d retiré | « exactement une émission » tombe à zéro |
| iv | `still_violated` rendu `true` en toutes circonstances | le test de la garde aval qui reprend la main : une P0 part sur un texte qui ne porte plus la revendication |
| v | `site: FalseLocalHosting` → `AssertGrounded` au dépôt de 5d | le test positif : son fixture ne porte **délibérément aucune** forme reconnue par `detect_affirmative_state_claim`, donc le mauvais discriminant supprime l'émission |

La recette (v) est ce qui rend le discriminant de D3 attesté plutôt que
plausible, et elle n'existe que parce que le fixture est construit pour. Sans
cette contrainte sur le fixture, les deux bras du `match` seraient
interchangeables et le refactor serait décoratif.

**Une sixième recette reste verte, et sa raison est la mesure.** Retirer le
terme `pending_fabrication_p0.is_none()` du dépôt de 5d **ne rougit rien** :
aucun des trois fixtures ne produit deux violations de la classe dans un même
tour (il faudrait une récidive de 5d **et** une récidive d'assert-grounded sur
le même texte). Le verrou de classe est donc livré **non attesté**, et c'est dit
ici plutôt que découvert plus tard. Fabriquer ce fixture — un texte portant à la
fois une fausse revendication de localité et une affirmation d'état non ancrée,
sur un tour où les deux gardes ont déjà consommé leur re-prompt — est une pièce
à part entière ; elle appartient au vol 2, qui est celui où le second site rend
la collision réellement atteignable.

### R2 — Le refactor du créneau casse #2870 en silence

Le risque n'est pas que les tests de #2867 rougissent : c'est qu'ils restent
verts alors que le comportement a bougé — par exemple si `still_violated` pour
`AssertGrounded` omet le terme `!assert_grounded_satisfied(…)` et devient plus
permissif. **Mitigation** : l'étape 4 de l'ordre de commit isole le refactor
dans son propre commit, avant tout ajout de site, et le `match` est écrit en
recopiant le prédicat composé de `4452-4454` **à l'identique** plutôt qu'en le
reformulant. L'AC11 est le plancher.

### R3 — Le verdict du site 1 est le plus contestable des six, et c'est assumé

`false_local_hosting_claim` est le seul site où le garde **sait** que la phrase
est fausse. Une lecture plausible du bearing de Prime — « un état attesté (le
garde voit un fait vérifiable) » — le rangerait du côté non rattaché : rien à
investiguer, le système connaît déjà la réponse, et le réveil n'aurait pas de
geste.

Le verdict proposé est l'autre, pour trois raisons écrites ici afin que Vincent
puisse les récuser nommément plutôt qu'en bloc :

1. « Un état attesté » décrit une réponse que le garde peut **confirmer**. Ici
   il la **réfute** : la forme de la phrase reste celle d'un état posé comme
   attesté, et rien dans le bearing ne fait de la connaissance du garde une
   exemption pour la phrase.
2. Le geste existe et n'est pas une investigation : une fausse revendication
   d'hébergement local **est partie chez un tenant payant**. C'est une
   déclaration de confidentialité, que la Distribution Doctrine (mika#1814,
   mika#2290) tient pour porteuse. Le réveil sert à décider quoi dire au tenant,
   pas à établir un fait.
3. C'est la seule faute des six dont la victime est le **tenant** et non
   l'opérateur. Les cinq autres coûtent une réponse de mauvaise qualité ; celle-ci
   coûte une affirmation fausse sur le traitement de ses données.

**Si Vincent récuse ce verdict**, le vol entier sort : le site et ses tests
quittent la PR, le refactor de D3 reste (il est le plancher du vol 2) et la PR
se réduit aux six verdicts plus un type sans second consommateur — auquel cas
elle se ferme et le vol 2 repart du ticket. C'est précisément ce que le draft
permet et qu'un merge aurait interdit.

### R4 — « Rien d'autre » est une borne, pas une suggestion

Trois dérives sont à portée de main et toutes les trois sont hors borne : armer
la porte (mika#2871), rattacher le site 6 à `recovery-runbook` (mika#2866), et
« tant qu'on y est » instrumenter les quatre sites non rattachés avec un
compteur. **Mitigation** : l'AC5 est vérifiable par `git diff` sur des plages de
lignes nommées, et la Fire-Disposition ci-dessus dit en propres termes que rien
dans ce vol ne touche `SETTABLE_CONFIG_KEYS` ni le défaut de la porte.
