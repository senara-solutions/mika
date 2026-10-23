# mika#2879 — rattacher la moitié accomplie de 5e à la classe `fabrication-detected` (vol 3/4)

**Ticket :** senara-solutions/mika#2879
**Branche :** `feat/2879/suivi-mika-2872-vol-2-rattacher-la-seule`
**Vol :** 3 sur 4 — vol 1 = mika#2872/#2878 (`d51da321`, PR #2878), vol 2 = la scission (`9341c7c0`, PR #2883)
**Plan du vol 2 :** `docs/plans/2026-10-23-001-feat-2879-scinder-les-deux-natures-de-lassertion-de-frequence-plan.md`

---

## Contexte — ce que le ticket demande encore

Le corps du ticket décrit trois gestes. **Le premier est livré et mergé.** La
scission du détecteur (`9341c7c0`, PR #2883) a atterri sur `main` le 2026-10-23
à 08:01Z : `FREQUENCY_ASSERTION_ACCOMPLISHED_STATE` (4 alternatives) et
`FREQUENCY_ASSERTION_FUTURE_COMMITMENT` (8) sont séparées, l'union est
reconstruite en **un seul site** via `LazyLock<String>` avec des groupes nommés
imbriqués `(?P<assert>(?P<accompli>…)|(?P<engagement>…))`, et
`FrequencyAssertionNature::{AccomplishedState, FutureCommitment}` se lit de
`caps.name("accompli").is_some()` — la branche que le moteur a prise, jamais une
re-classification du fragment capturé. Les deux `warn!` de 5e portent déjà
`assertion_nature`.

Ce qui reste du ticket, c'est **le geste 2 et le geste 3**, et le plan du vol 2
les a déjà nommés dans sa ligne `Tracked in:` :

> Vol 3 : le rattachement de la moitié accomplie à la classe
> `fabrication-detected` (variante `PendingFabricationP0Site`, bras de
> `still_violated`, dépôt dans la branche 5e derrière `fabrication_p0_notify`
> **et** `pending_fabrication_p0.is_none()`, et ses tests moteur) — il ferme
> AC1, AC4 et AC5. Vol 4 : le fixture de collision 5d + `assert_grounded` qui
> atteste le verrou de classe — il ferme AC3.

**Ce plan est le vol 3.** Il ne replanifie rien de la scission.

---

## Le constat

`detect_unactioned_frequency_promise` sait depuis le vol 2 *laquelle* des deux
natures il vient d'apparier. Mais cette nature n'a aujourd'hui **qu'un lecteur
d'observabilité** : les deux `warn!` la journalisent. Aucune décision du moteur
ne la lit.

Conséquence mesurable sur l'arbre courant : la branche résiduelle de 5e
(`agent_loop/mod.rs:3041-3072`, `event = "guard.unactioned_frequency_promise_uncorrected"`)
journalise et **ne dépose rien**. Un tour qui a épuisé son budget de re-prompt
en affirmant « c'est corrigé : plus aucun message de veille technique » part
chez le tenant avec un état du système que rien n'établit — et l'opérateur n'est
pas réveillé, alors que le site 5d voisin, huit lignes plus haut, dépose une P0
pour exactement la même forme de faute.

Le critère d'appartenance de mika#2872 D1 — *« la phrase fautive affirme-t-elle
un état du système qu'aucun résultat d'outil de ce tour ne porte ? »* — tranche
les deux moitiés en sens contraires :

| famille | exemple | verdict D1 |
|---|---|---|
| état accompli | « c'est corrigé », « i've stopped » | **dans la classe** — le terme (C) du détecteur prouve qu'aucun `set_config` sur `PROACTIVE_DAILY_BUDGET_KEY`/`PROACTIVE_PAUSE_UNTIL_KEY` n'a tourné, donc l'état affirmé n'existe pas, et seul un humain peut l'établir |
| engagement sur l'avenir | « je vais réduire la fréquence » | **hors classe** — une promesse non tenue n'est pas un état fabriqué ; aucun geste d'opérateur ne la rendrait vraie |

Le doc-comment de `PendingFabricationP0Site` (`guards.rs:1006`) porte déjà la
dette, mot pour mot : « `unactioned_frequency_promise` n'y est qu'à moitié (sa
branche « état accompli » seulement) et **attend son sous-prédicat** ».

---

## Périmètre de ce vol

**Ce vol livre le troisième site d'écriture de la classe `fabrication-detected`,
et les deux attestations de classe qui le séparent de sa moitié hors classe.**

Il ne livre **pas** le fixture de collision d'AC3. Ce n'est pas un choix de
confort : vol 3 (≈ 263 lignes estimées) + vol 4 (≈ 230) dépasse franchement la
borne par vol de 279 lignes, et le découpage est donc imposé par mika#2693. Le
vol 4 est par ailleurs **indépendant** des trois premiers — il n'ajoute aucun
site, il observe la collision entre deux sites déjà livrés — ce qui en fait la
coupe la moins coûteuse.

---

## Décisions de conception

### D1 — une troisième variante de `PendingFabricationP0Site`, nommée sur la moitié

```rust
/// 5e sur sa seule branche « état accompli » (mika#2879, vol 3). La
/// moitié « engagement sur l'avenir » reste HORS de la classe : une
/// promesse non tenue n'affirme aucun état du système, et aucun geste
/// d'opérateur ne la rendrait vraie (critère mika#2872 D1).
UnactionedFrequencyPromiseAccomplished,
```

Le nom porte la moitié. Une variante nommée `UnactionedFrequencyPromise` tout
court inviterait le prochain lecteur à y rattacher le détecteur entier — c'est
exactement le verdict en bloc que mika#2878 a refusé dans les deux sens.

### D2 — le bras de `still_violated` rejoue le détecteur **et** relit la nature

```rust
Self::UnactionedFrequencyPromiseAccomplished => {
    detect_unactioned_frequency_promise(text, summaries).is_some_and(|promise| {
        promise.nature == FrequencyAssertionNature::AccomplishedState
    })
}
```

La signature existante suffit : `(text, summaries, deployment)` — le détecteur
consomme `text` et `summaries`, et le terme (C) (`frequency_actor_called`) lit
les `summaries` du tour d'acceptation, pas ceux du tour de dépôt. C'est le
point : le créneau est **revérifié** à l'acceptation, et un tour qui aurait
entre-temps appelé `set_config` ne réveille personne.

La nature est relue, pas transportée. Transporter `promise.nature` dans
`PendingFabricationP0` ferait d'un champ du créneau la source de vérité sur un
texte qui a changé — la forme même que mika#2158 condamne (un prédicat recopié
divergent en silence). Ici les deux lectures appellent **la même fonction**, à
deux instants, sur deux textes : c'est la forme de `Self::FalseLocalHosting`.

### D3 — le dépôt dans la branche résiduelle de 5e, derrière trois termes

Dans `agent_loop/mod.rs`, à la fin de la branche
`guard.unactioned_frequency_promise_uncorrected` (après le `warn!`, qui reste
inchangé) :

```rust
if fabrication_p0_notify
    && pending_fabrication_p0.is_none()
    && promise.nature == FrequencyAssertionNature::AccomplishedState
{
    pending_fabrication_p0 = Some(PendingFabricationP0 {
        message: format!(
            "Fabrication detected (uncorrected) on message frequency: the \
             frequency-promise guard fired and the re-prompt budget is spent, \
             yet the response still asserts «{assertion} … {subject}» as an \
             accomplished state, and no set_config on \
             {PROACTIVE_DAILY_BUDGET_KEY} or {PROACTIVE_PAUSE_UNTIL_KEY} ran \
             this turn. The answer went out, and the tenant was told the \
             frequency had already changed. Investigate trace_id {trace_id} \
             in /var/log/mika/server.log.",
            assertion = promise.assertion,
            subject = promise.subject,
            trace_id = tool_ctx.trace_id,
        ),
        site: PendingFabricationP0Site::UnactionedFrequencyPromiseAccomplished,
    });
}
```

Trois termes, trois rôles distincts, et c'est délibéré :

- `fabrication_p0_notify` — la porte mika#1574 option (b) (voir
  § Fire-Disposition).
- `pending_fabrication_p0.is_none()` — le verrou de **classe** : une seule P0
  par tour pour toute la classe, le premier écrivain de la chaîne gagne.
- `promise.nature == AccomplishedState` — l'appartenance. Sans lui, un
  engagement sur l'avenir **occuperait le créneau**, et le 5d du même tour
  trouverait la place prise : la P0 de 5d serait perdue, puis celle de 5e
  supprimée à l'acceptation par D2. Le terme n'est pas redondant avec D2 ; il
  protège le créneau, là où D2 protège l'émission.

L'ordre du message suit celui de 5d : assertion puis sujet, dans l'ordre où la
garde les a appariés, avec l'ellipse qui dit que les deux fragments ne sont pas
contigus. Le texte nomme les deux clés du terme (C) parce que le geste que
l'opérateur doit poser en sortie du réveil est de vérifier l'état réel du budget
proactif.

### D4 — deux attestations de classe, un seul échafaudage

Un nouveau fichier `crates/mika-agent/tests/eval/test_fabrication_detected_p0_2879.rs`,
sur la forme canonique de `test_fabrication_detected_p0_2872.rs` (`SeverityRecordingSender`,
`Emission`, `SEVERITE_ABSENTE` importés de `test_dispatch_stall_p0_1381`,
`EvalHarness::builder().responses(…).message_sender(…).fabrication_p0_notify(…)`).

1. **Le test positif** — récidive d'état accompli, porte armée : exactement une
   émission portant le marqueur, son étage `!= SEVERITE_ABSENTE` (le contrôle
   mika#1381 : `send_with_severity` a un corps par défaut qui délègue à
   `self.send(text)`, donc un site qui régresse sur `send()` perd son étage **en
   silence**), son étage `== Some(NotificationSeverity::P0)`, et son texte qui
   nomme l'assertion, le sujet et `harness.trace_id`. Précédé du contrôle
   d'anti-vacuité `trace.llm_call_count > 1` — sans lui, « la branche résiduelle
   n'a jamais été atteinte » se lit comme « le site ne tire pas ».

2. **Les deux contrôles négatifs**, derrière un seul helper
   `async fn aucune_p0_de_classe(reponses, porte_armee) -> Result<()>` qui monte
   le harnais, joue le fixture, exige `trace.llm_call_count > 1` puis
   `fabrications.is_empty()` :
   - récidive d'**engagement**, porte **armée** ⇒ rien. C'est l'attestation que
     la moitié hors classe le reste.
   - récidive d'**état accompli**, porte **au défaut** (le `false` livré — le
     helper omet simplement `.fabrication_p0_notify(…)`) ⇒ rien. C'est AC5 au
     comportement.

Les deux cas ne diffèrent chacun que par **un seul axe** par rapport au test
positif, et le helper rend cette différence littérale : c'est l'idiome de paire
de mika#1926, et c'est aussi ce qui fait tenir les trois attestations sous la
borne.

### D5 — ce que ce vol ne touche pas

| surface | pourquoi |
|---|---|
| les deux listes d'alternatives, l'union `LazyLock`, `FrequencyAssertionNature` | livrés par le vol 2 ; l'ensemble des réponses qui font tirer le détecteur est **inchangé** (AC2) |
| les deux `warn!` de 5e, et la branche de tir (`agent_loop/mod.rs:2964-3039`) | le tir et son prompt de correction sont intacts ; seule la branche **résiduelle** gagne un dépôt |
| `crates/mika-common/src/config.rs` (lignes 352, 573, 1268, 2807, 2947) | AC5 : ni la clé `fabrication_p0_notify`, ni son défaut `false`, ni `SETTABLE_CONFIG_KEYS`. L'armement relève de mika#2871 |
| le site d'acceptation (`agent_loop/mod.rs:4518-4545`) | il consomme déjà `pending.site.still_violated(…)` ; la nouvelle variante s'y branche sans le modifier |
| `PendingFabricationP0` (les deux champs) | le créneau ne gagne aucun champ (D2) |
| les 10 tests 5e de `guards.rs` et les 4 de `test_unactioned_frequency_promise_2358.rs` | plancher AC2 : ils doivent rester verts **non modifiés** |

---

## Ordre de commit

1. **`guards.rs`** — la variante D1, son bras D2, et l'amendement du
   doc-comment de `PendingFabricationP0Site` (la phrase « attend son
   sous-prédicat » devient fausse le jour où ce vol atterrit ; la laisser est
   la dette que mika#2340 décrit). `cargo test -p mika-agent evidence::guards`
   doit être vert ici : rien ne dépose encore dans la variante, mais le `match`
   de `still_violated` est exhaustif et le compilateur l'exige.
2. **`agent_loop/mod.rs`** — le dépôt D3. `cargo test -p mika-agent` vert : les
   10 tests 5e et le scan `mika2879_les_deux_sites_5e_emettent_la_nature`
   (2 sites attendus) ne bougent pas.
3. **`tests/eval/test_fabrication_detected_p0_2879.rs` + sa déclaration dans
   `tests/eval.rs`** — les trois attestations D4. `cargo test -p mika-agent
   --test eval` vert.
4. **Rouge-avant** — les six recettes ci-dessous, jouées une par une sur
   l'arbre vert, chacune révoquée par `git checkout` avant la suivante, et
   l'**observé** (non l'attendu) écrit dans l'en-tête de doctrine du fichier de
   test.
5. `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
   `scripts/verify-pipeline.sh origin/main`.

---

## Rouge-avant, terme par terme (AC4)

Chaque recette est une injection jouée sur l'arbre, la suite lancée, puis
l'injection révoquée par `git checkout`. Ce qui va dans l'en-tête de doctrine du
fichier de test est **ce qui a été observé**, y compris les recettes restées
vertes avec leur raison — c'est précisément le geste que R6 de mika#2872 a
rendu non négociable.

| # | injection | attendu | ce qui le rougit |
|---|---|---|---|
| R1 | le dépôt D3 entier retiré | **ROUGE** | le test positif : zéro émission portant le marqueur |
| R2 | `promise.nature == AccomplishedState` retiré du **bras de `still_violated`** (D2) | **ROUGE** | le contrôle d'engagement : une P0 d'engagement atteint le transport |
| R3 | `fabrication_p0_notify &&` retiré du dépôt | **ROUGE** | le contrôle à porte fermée : une P0 sort sur la configuration livrée |
| R4 | `NotificationSeverity::P0` → `::P2` au site d'acceptation | **ROUGE** | le test positif, sur l'étage observé. Rougit aussi 2867 et 2872 — le site d'acceptation est partagé par les trois, et c'est la propriété qu'on veut |
| R5 | `send_with_severity(…)` → `send(…)` au site d'acceptation | **ROUGE** | le test positif, sur `*etage != SEVERITE_ABSENTE` (mika#1381 : le corps par défaut délègue, donc la perte d'étage est silencieuse sans cette assertion) |
| R6 | `promise.nature == AccomplishedState` retiré du **dépôt** (D3), le bras de D2 intact | **VERT — et la raison est la livraison** | rien dans ce vol. Le créneau est revérifié à l'acceptation par un bras qui porte le même terme, donc l'émission reste supprimée ; le seul observable est le **verrou de classe** — un tour qui occupe le créneau hors classe et fait perdre sa P0 à 5d — et aucun fixture de ce vol ne produit deux violations de classe dans le même tour. C'est le fixture de collision d'AC3, **vol 4** |
| R7 | `pending_fabrication_p0.is_none()` retiré du dépôt | **VERT** | même raison que R6, et c'est le R6 de mika#2872 reconduit : avec un seul écrivain par tour dans les fixtures, le verrou ne se distingue pas de son absence. **Vol 4** |

Deux verts sur sept, et les deux portent le même manque — un fixture où deux
sites de la classe écrivent dans le même tour. C'est exactement le livrable du
vol 4, et le ticket l'a nommé AC3.

---

## Fire-Disposition

**Ce vol livre un détecteur** : le dépôt D3 est une garde EndTurn dont le chemin
de succès est « aucune violation trouvée ». La section est donc requise.

**Option retenue : (b) livrer désarmé** — la forme déjà livrée par mika#2867 et
mika#2872 pour les deux premiers sites de la classe, et celle que le ticket
prescrit en AC5.

- **Le désarmement :** le dépôt est gardé par `fabrication_p0_notify`, dont le
  défaut livré est `false` (`crates/mika-common/src/config.rs:1268`, défaut de
  test `config.rs:2807`). Sur la configuration que tout tenant fait tourner
  aujourd'hui, le site journalise (le `warn!` existant, inchangé) et **n'émet
  rien**. Aucune ligne de ce vol ne touche cette valeur.
- **L'attestation du désarmement :** le second contrôle négatif de D4 — même
  fixture que le test positif, `.fabrication_p0_notify(…)` simplement omis,
  zéro émission attendue. C'est ce qui fait que « livrer désarmé » n'est pas
  qu'une intention documentée (R3 ci-dessus la rougit).
- **Le suivi tracké pour l'armer :** `senara-solutions/mika#2871` — l'armement
  de la porte pour toute la classe, explicitement hors périmètre par AC5.
- **Allowlist :** sans objet. Le détecteur est comportemental, pas structurel ;
  il ne scanne aucune source et n'a donc aucune violation existante à excepter.
  Zéro entrée d'exception livrée, et rien à faire expirer.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/evidence/guards.rs` (variante D1, bras `still_violated` D2, amendement du doc-comment de classe) | 34 |
| `crates/mika-agent/src/agent_loop/mod.rs` (dépôt D3 dans la branche résiduelle de 5e) | 40 |
| tests (`crates/mika-agent/tests/eval/test_fabrication_detected_p0_2879.rs` : en-tête de doctrine, fixtures accompli/engagement, test positif, helper + deux contrôles négatifs) | 188 |
| `crates/mika-agent/tests/eval.rs` (déclaration du module) | 1 |

Total estimé : 263 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et pourquoi le suivant est un vol et non une phase.**
263 < 279 : un seul vol pour ce périmètre. Le périmètre complet du résidu du
ticket (ce vol + le fixture de collision d'AC3, estimé ≈ 230 lignes : en-tête,
fixture d'un tour qui viole 5d **et** `assert_grounded`, l'assertion
d'unicité de la P0, et le rouge-avant du verrou de classe qui doit enfin
rougir) pèse ≈ 493 lignes, soit 1,77 × la borne. Le vol 4 est de surcroît
**indépendant** des trois premiers — il n'ajoute aucun site d'écriture, il
observe la collision de deux sites déjà sur `main` — donc il est mergeable
seul, avec son propre plancher.

Le découpage est en **vols** et non en phases d'un même vol pour cette raison
précise : le vol 3 part en PR et se merge sans rien attendre du vol 4, et le
vol 4 pourra se merger sans rien attendre d'un vol 5.

**Tracked in:** senara-solutions/mika#2879 — vol 4 : le fixture de collision
5d + `assert_grounded` qui atteste le verrou de classe (AC3), avec le
rouge-avant R6/R7 ci-dessus qui doit y rougir. Le ticket reste ouvert comme
porteur de ce vol.

**Le risque sur l'estimation, nommé.** Trois tests moteur de cette exacte
famille ont coûté **369 lignes réelles** (`test_fabrication_detected_p0_2872.rs`)
pour 150 estimées — facteur 2,5 — et le plan de mika#2878 a livré 567 lignes
réelles pour 266 estimées. L'estimation ci-dessus est en **lignes estimées**, la
même unité que la borne (`docs/solutions/dev-loop/une-estimation-de-taille-se-compare-a-un-seuil-dans-sa-propre-unite.md`),
donc la comparaison est licite ; mais le volume réel attendu est de l'ordre de
400 lignes. Le facteur de sécurité tient sur deux appuis concrets : le
mutualisateur `aucune_p0_de_classe` (un seul échafaudage pour deux contrôles,
là où 2872 écrivait trois tests indépendants), et un en-tête de doctrine
**court** qui cite les deux fichiers frères (`…_2867.rs`, `…_2872.rs`) plutôt
que de re-déposer la doctrine de la classe. Si l'en-tête dérape, le signal est
que le contenu appartenait à un fichier frère.

---

## Definition of Done

- [ ] `PendingFabricationP0Site::UnactionedFrequencyPromiseAccomplished` existe,
      son nom porte la moitié, et son doc-comment nomme le critère mika#2872 D1
      ainsi que la raison pour laquelle l'autre moitié reste dehors.
- [ ] Le bras de `still_violated` rejoue `detect_unactioned_frequency_promise`
      et relit la nature ; aucun champ nouveau sur `PendingFabricationP0`.
- [ ] Le doc-comment de `PendingFabricationP0Site` ne dit plus que 5e « attend
      son sous-prédicat ».
- [ ] Le dépôt existe dans la branche résiduelle de 5e, derrière les trois
      termes de D3, et le message nomme l'assertion, le sujet, les deux clés du
      terme (C) et `tool_ctx.trace_id`.
- [ ] Les trois attestations de D4 existent et passent ; le test positif porte
      son contrôle d'anti-vacuité (`trace.llm_call_count > 1`) et son assertion
      `*etage != SEVERITE_ABSENTE`.
- [ ] Les sept recettes de rouge-avant sont jouées, et l'**observé** de chacune
      — y compris les deux verts avec leur raison et leur renvoi au vol 4 — est
      écrit dans l'en-tête de doctrine du fichier de test.
- [ ] `git diff --stat` ne montre aucune ligne sous `crates/mika-common/src/config.rs`.
- [ ] Les 10 tests 5e de `guards.rs`, les 4 de
      `test_unactioned_frequency_promise_2358.rs` et le scan
      `mika2879_les_deux_sites_5e_emettent_la_nature` sont verts **sans
      modification**.
- [ ] `cargo test`, `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` verts.
- [ ] Corps de PR : `Tracked in: senara-solutions/mika#2879` pour AC3 (vol 4),
      et `Code review: delegated to mika-qa (mika#2694)`.

---

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2879 :

- **AC1** : une réponse d'état accompli que le terme (C) contredit lève la
  récidive de classe `fabrication-detected`. Une réponse d'engagement sur
  l'avenir ne la lève pas. Les deux sont testées.
- **AC2** : l'ensemble des réponses qui font tirer le détecteur est inchangé
  (test de non-régression sur les fixtures existants).
- **AC3** : le fixture de collision 5d + `assert_grounded` émet une seule P0 par
  tour. Retirer le verrou de classe le fait rougir.
- **AC4** : rouge-avant, terme par terme, sur le commit vert.
- **AC5** : la porte reste désarmée par défaut. Son armement relève de
  mika#2871.

### Répartition par vol

| AC | vol 2 (`9341c7c0`, mergé) | **vol 3 (ce plan)** | vol 4 (`Tracked in:`) |
|---|---|---|---|
| AC1 | la nature est calculée et journalisée | **fermé** — les deux moitiés testées au niveau de la classe (D4.1 et D4.2) | — |
| AC2 | fermé — la scission ne change pas l'ensemble apparié | **reconduit** — plancher vert non modifié (§ D5) | reconduit |
| AC3 | — | — | **fermé** — fixture de collision, et R6/R7 y rougissent |
| AC4 | fermé sur son périmètre | **fermé** sur ce périmètre — sept recettes, deux vertes avec raison écrite | fermé sur son périmètre |
| AC5 | sans objet | **fermé** — aucune ligne sous `config.rs`, porte au défaut `false`, contrôle à porte fermée (D4.2) | reconduit |

### Critères dérivés de ce vol

- **AC-a** : une récidive d'état accompli, porte armée, produit **exactement
  une** émission portant le marqueur de classe, en `NotificationSeverity::P0`,
  dont le texte nomme l'assertion, le sujet et le `trace_id` du tour.
- **AC-b** : une récidive d'engagement sur l'avenir, porte armée, produit
  **zéro** émission — et le contrôle d'anti-vacuité prouve que la branche
  résiduelle a bien été atteinte.
- **AC-c** : la même récidive d'état accompli, porte au défaut livré, produit
  **zéro** émission.
- **AC-d** : retirer le terme de nature du bras de `still_violated` fait rougir
  AC-b ; retirer `fabrication_p0_notify` du dépôt fait rougir AC-c ; retirer le
  dépôt fait rougir AC-a.
- **AC-e** : la nature n'est pas stockée dans `PendingFabricationP0` ; les deux
  lectures appellent `detect_unactioned_frequency_promise`.

---

## Risques

| risque | mitigation |
|---|---|
| **Le terme de nature au dépôt est livré non attesté** (R6 vert). | Nommé explicitement dans l'en-tête de doctrine du fichier de test avec sa raison et son renvoi au vol 4, exactement comme mika#2872 a nommé son R6. Le terme reste écrit au dépôt plutôt que retiré, parce que son absence coûte une P0 de 5d perdue dans un tour de collision — un défaut réel, et non une redondance. |
| **Le volume réel dépasse l'estimation** (facteur 2,5 mesuré sur la famille). | Le mutualisateur de D4 et l'en-tête court sont les deux appuis. Le vol a trois livrables ordonnés dont les deux premiers sont petits et compilent seuls : si le plafond de tours devient serré, l'arbre est vert et mergeable après le commit 3, et le rouge-avant est le seul reste. |
| **Re-classifier le fragment capturé au lieu de lire la branche prise** — la faute que mika#2879 D1 propriété 2 a nommée pour le vol 2. | D2 n'a aucune regex : il appelle le détecteur et lit `promise.nature`. Toute apparition d'une seconde liste d'alternatives dans ce vol est le symptôme (classe mika#2158). |
| **Une seconde population sous un seul nom d'événement** (classe mika#2205) — si le dépôt était inconditionnel sur la nature, le marqueur du message confondrait les deux moitiés chez l'opérateur. | Le marqueur `on message frequency` ne sort que pour la moitié accomplie, et le texte dit « as an accomplished state ». La moitié d'engagement n'a aucun chemin vers le transport. |
| **Le site d'acceptation est partagé par trois sites** : une injection sur lui (R4, R5) rougit aussi 2867 et 2872. | C'est la propriété voulue, pas un effet de bord : les trois sites doivent perdre leur étage ensemble, sinon l'un d'eux a cessé de passer par `send_with_severity`. Écrit comme tel dans l'en-tête. |
| **Un troisième écrivain rend la collision ordinaire** — le verrou de classe passe de théorique à load-bearing dès que ce vol atterrit. | C'est précisément ce qui rend le vol 4 actionnable, et c'est la raison pour laquelle mika#2878 l'avait renvoyé à un tour où un troisième site existe. Le vol 4 est nommé par la ligne `Tracked in:`. |
