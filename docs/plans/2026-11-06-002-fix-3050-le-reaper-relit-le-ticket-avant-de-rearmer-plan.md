# fix(3050) — le reaper relit le ticket à la forge avant de ré-armer une parente `ready-label`

Ticket : senara-solutions/mika#3050 (`bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`)
Base : `4fb4a4dde`
Branche : `fix/3050/task-engine-le-reaper-des-t-ches-bloqu`

## 0. Le défaut, et le fil exact qui l'a retenu

Le reaper des parentes bloquées (mika#2045) ré-arme un lanceur différé après la
grâce de 45 min **sans relire le ticket**. Sur 6 parentes `ready-label` ré-armées
depuis le 2026-10-20, deux l'ont été sans consentement :

| parente | ticket | `ready` au ré-armement | suite |
|---|---|---|---|
| `74130014` | #3002 | **retiré** à 14:08:43Z | le tour mika-dev ferme #3002, qu'aucune PR ne fermait |
| `0529ed9b` | #3044 | **retiré** à 14:10:04Z (1 s avant la création de la parente) | deux tours mika-dev sans dispatch, puis `failed` |

Le fil qui rend la forme **structurelle** et non accidentelle : depuis mika#3022,
`ready` est retiré à chaque merge d'une PR de la boucle
(`merged_pr_ready_label_cleaned`). Une parente `ready-label` en attente au moment
d'un merge `Refs` est donc dans cette forme par construction.

**Pourquoi mika#2995 ne couvre pas ce cas, et c'est le point de conception du
vol.** mika#2995 garde déjà la *promotion* d'un lanceur différé, mais avec le
prédicat de **parc de la timeline** : « le dernier événement sur `ready` est un
`unlabeled` par un acteur non-machine ». Le retrait de mika#3022 est fait par une
identité **machine**, donc le prédicat de parc rend « non parqué » →
`ConsentHeld` → le tour part. C'est exactement le cas `0529ed9b`/#3044. Ce vol a
donc besoin d'un prédicat d'**état** (ouvert + porte `ready`), pas d'un prédicat
de timeline. Les deux gardes sont complémentaires et doivent porter des raisons
terminales **distinctes**, sans quoi les deux populations ne sont plus
soustractibles (mika#2205) — et la cible d'AC4 est précisément un compte.

mika#3047 empêche désormais la phase 2 de *créer* une telle parente. Le
ré-armement d'une parente **déjà en file** n'est pas couvert. C'est ce vol.

## 1. Le site, sur `4fb4a4dde`

| élément | lieu | lecture actuelle |
|---|---|---|
| reaper des parentes bloquées | `task_engine/engine.rs`, `reap_orphaned_pending_issue_tasks` (l. 1956), appel l. 2162 | ré-arme sans relire |
| balayage `stale_blocked_dispatch` | `task_engine/engine.rs`, appel l. 2630 | **troisième appelant, absent du tableau du ticket** |
| ré-armement après un tour vide | `task_engine/dispatcher.rs`, `rearm_consumed_deferred_wrapper` (l. 3955) | même absence |
| le ré-armement lui-même | `skills/executor.rs`, `rearm_deferred_callback` (l. 5862) | reconstruit le lanceur depuis la config de la parente |
| module canonique du label | `ready_label.rs` — `READY_LABEL`, `READY_LABEL_TASK_PREFIX` (l. 150), `parse_ready_label_task_label` | site de définition unique |
| modèle de relecture `gh` | `auto_pull.rs`, `gh_reread_issue_at_dispatch` + `enum DispatchReread` (l. 5651) | mika#3047, **hors vol à modifier**, à imiter |
| seam de test `gh` | `gh_process.rs` — double `#[cfg(test)] run_gh` (l. 281), `set_canned_stdout_for`, `recorded_calls()`, `FIXTURE_ISSUE_FLOOR = 990_000` (l. 206) | |

Le ticket nomme deux chemins ; il y en a **trois**. Le troisième
(`stale_blocked_dispatch`) est un surensemble gratuit de la décision D1
ci-dessous, pas une extension de périmètre : c'est une **conséquence** du site
unique, et le plan le nomme plutôt que de la laisser se découvrir en revue.

## 2. AC1 — la relecture, et où elle vit

**D1. La garde vit à l'intérieur de `rearm_deferred_callback`, en position 3 de
son ordre interne.** Pas aux deux (trois) sites appelants. Doctrine du lecteur
unique (mika#2158, mika#2205, mika#3049) : trois copies d'un prédicat de
consentement, c'est trois endroits où un futur éditeur peut en oublier un, avec
toutes les assertions vertes. L'ordre interne devient :

1. `has_non_deferred_active_callback_child` — fail-closed → `NotNow` *(inchangé)*
2. `find_live_deferred_wrapper_child` — mika#2413 → `AlreadyRepresented` *(inchangé)*
3. **la relecture du consentement** *(nouveau)*
4. `get_stuck_rearm_count` ≥ `MAX_STUCK_REARMS` → `Unrepairable`
5. `count_pending_deferred_callbacks` ≥ cap → `NotNow`
6. insertion du `NewTask`

La position est décidée, pas libre. **Après 1 et 2**, parce que ce sont deux
gardes locales gratuites et qu'il est inutile de payer un appel réseau pour une
parente déjà représentée. **Avant 4**, parce qu'un consentement retiré doit
sortir la parente en terminal **immédiatement** — la faire d'abord épuiser son
quota de ré-armements lui coûterait les tours mika-dev que ce ticket existe pour
supprimer.

**D2. Le prédicat vit dans `ready_label.rs`.**

```rust
pub(crate) enum RearmConsent { Held, ReadyAbsent, Closed }

pub(crate) async fn reread_issue_consent_via_gh(
    token: &str, owner_repo: &str, number: u64,
) -> Result<RearmConsent, String>
```

Deux raisons de ne **pas** réutiliser `fetch_issue_body_and_labels_via_gh`
(`server/ready_label_handler.rs` l. 1794), que le ticket cite comme existant :
il ne rend **pas** `state`, donc la moitié « ouvert » d'AC1 n'est pas couverte ;
et il vit dans un module de handler HTTP, pas dans le module qui définit
`READY_LABEL` une seule fois dans la crate. `ready_label.rs` porte déjà le
prédicat de parc de mika#2995 ; le prédicat d'état est son frère exact.

**D3. Fail-closed champ par champ, sur le modèle de mika#3047.**
`gh issue view <n> --repo <owner/repo> --json state,labels`.
`state` doit matcher `OPEN`/`CLOSED` insensiblement à la casse, sinon `Err`.
`labels` est **requis** : absent ⇒ `Err`, **jamais** `ReadyAbsent`. Un `labels`
manquant qui atterrirait dans le registre sous `ready_absent_at_rearm` serait
« le nom d'une course gagnée » et empoisonnerait la sonde d'AC4 (mika#2131,
mika#2331 — une absence n'est pas un zéro).

**D4. Le discriminant est `parse_ready_label_task_label(&parent.label)`.** Il rend
`None` pour toute parente qui n'est pas de la classe `ready-label` ; dans ce cas
la garde est un **no-op total**, et notamment **aucun appel `gh` n'est émis**.
C'est la clause d'AC3, et elle est attestée par l'absence d'appel dans
`recorded_calls()`, pas seulement par l'issue rendue.

**D5. Une nouvelle variante : `RearmOutcome::ConsentWithdrawn { reason: &'static str }`.**
Pas une réutilisation d'`Unrepairable` : les trois appelants expirent déjà la
parente sous `Unrepairable` avec la raison `stuck_pending_no_deferred_wrapper`,
et fusionner les deux populations les rend non-soustractibles (mika#2205) alors
qu'AC4 est un compte. Le `match` des trois appelants est exhaustif et **sans bras
`_ =>`** (mika#2290, mika#2413) : la nouvelle variante ne compile pas sans que
chacun des trois la nomme.

**D6. Le terminal et l'audit sont écrits une seule fois, au site de la garde.**
`update_task_failed(parent_task_id, reason)` puis
`log_audit_event(&format!("system-{agent_id}"), REARM_CONSENT_RECHECK_TOOL_NAME,
parent_task_id, Some("pending"), Some("failed"), …)`. `failed` est dans
`TERMINAL_PARENT_STATUSES` et exclu de `idx_tasks_manual_active_ref_url`, donc la
parente **libère le prochain `ready`** — c'est la troisième clause d'AC1, et elle
est obtenue par un statut qui existe déjà, sans nouveau statut ni migration.

Trois constantes, dans `ready_label.rs` à côté de leurs sœurs de mika#2995 :

```rust
pub(crate) const READY_ABSENT_AT_REARM_REASON: &str = "ready_absent_at_rearm";
pub(crate) const ISSUE_CLOSED_AT_REARM_REASON: &str = "issue_closed_at_rearm";
pub(crate) const REARM_CONSENT_RECHECK_TOOL_NAME: &str = "ready_label_rearm_recheck";
```

Les deux premières sont les noms **littéraux** qu'AC1 exige. Elles sont distinctes
de `READY_CONSENT_WITHDRAWN_REASON` (mika#2995) : deux gardes, deux populations,
deux greps.

Conséquence sur les bras appelants, chacun ~6-8 lignes :

- `engine.rs` l. 2162 (reaper) et l. 2630 (stale-blocked) : `ConsentWithdrawn` →
  `cancel_deferred_wrappers_of_parent` puis un `info!` nommant la raison. Le
  terminal est déjà posé par la garde ; l'appelant ne le repose pas.
- `dispatcher.rs` l. 4037 : `ConsentWithdrawn` → `record_wrapper_noop(&task.id,
  reason)` — seul l'appelant tient le wrapper consommé — puis un `debug!`.

**D7. Le jeton.** `rearm_deferred_callback` ne tient pas `Settings`. Elle gagne un
paramètre `github_token: Option<&str>`, résolu **une fois par passe** par chaque
appelant via `self.settings.resolve_github_token(self.github_app.as_deref()).await`
(`mika-common/src/config.rs` l. 1942). Une fois par passe et non par candidate :
la résolution d'un jeton d'App fait un appel réseau et le reaper boucle sur N
candidates. `None` ⇒ fail-closed `NotNow`, cause `rearm_reread_no_token`.

## 3. AC2 — fail-closed, et ce que « échoue » recouvre

Trois causes d'échec, **une seule issue** : `RearmOutcome::NotNow`, la parente
reste `pending`, aucun lanceur, aucun terminal. Le tick suivant réessaiera.

| cause | nommée dans le log | issue |
|---|---|---|
| `Err` de `reread_issue_consent_via_gh` (sortie illisible, `state` hors contrat, `labels` absent, `gh` en erreur) | `rearm_reread_failed` + le détail | `NotNow` |
| jeton absent | `rearm_reread_no_token` | `NotNow` |

L'asymétrie est intentionnelle et chiffrée : un faux négatif coûte **un tick de
10 minutes** ; un faux positif coûte **~600 k tokens Opus** et, dans le cas
`74130014`/#3002, la fermeture d'un ticket gardé ouvert pour une sonde.

**Le faux positif de l'autre sens, et pourquoi il est hors population.** On
pourrait craindre la fenêtre retrait→repose de la phase 2 d'auto_pull. Elle ne
croise pas : le reaper n'agit qu'à ≥ 45 min (`stuck_pending_reaper_grace_secs`),
et le filtre 4a d'auto_pull exclut un ticket que le moteur tient déjà. Et même en
cas de croisement, l'effet est une parente terminale qui **libère** le créneau de
`idx_tasks_manual_active_ref_url` — elle ne verrouille rien.

## 4. AC3 — les autres classes

`parse_ready_label_task_label` rend `None` sur tout label qui ne commence pas par
`READY_LABEL_TASK_PREFIX`, ou dont la queue n'est pas un `owner/repo#N` valide.
Les parentes `implement` et groom, et les lanceurs différés de ces classes,
traversent la garde sans la payer : pas d'appel `gh`, pas de jeton requis, issue
inchangée. Le test T6 l'atteste par `recorded_calls()` vide.

## 5. Les tests, et les deux coûts cachés qu'ils ont révélés

**D8. Le plancher de fixture.** `FIXTURE_ISSUE_FLOOR = 990_000` et le double
`#[cfg(test)] run_gh` **assertent** `n >= FIXTURE_ISSUE_FLOOR` sur tout
`GhSubject::Issue(n)`. Ajouter une lecture `gh` sur le chemin du reaper fait donc
**paniquer les 21 tests existants** bâtis sur `seed_pending_issue_parent`
(l. 7903), dont les numéros sont 2013/2026/2158/2159/2184/9999 — tous sous le
plancher.

**Le plancher n'est pas desserré.** mika#2713 le dit en toutes lettres : il n'y a
pas d'allowlist, et en ajouter une rouvrirait la fuite que ce plancher a fermée
(précédent : plan mika#3047 § 10). Le geste est la renumérotation, et elle se
fait **à l'intérieur de la fixture partagée** — `issue + FIXTURE_ISSUE_FLOOR` dans
`seed_pending_issue_parent`, sous `#[cfg(test)]` — donc **zéro site d'appel
touché**. La fixture poste en même temps un payload `gh` canné OPEN + `ready`,
pour que les 21 tests restent verts sans modification. Restent ~4 littéraux
d'assertion à ajuster : l. 7987 (`"mika#2013"` dans l'`action_config`), l. 8484 /
8488, l. 8990 / 9003, l. 9307 (l'URL `…/issues/2158`). (l. 10380 / 10387 est un
test pur de `rebuild_deferred_action_config`, hors chemin `gh`.)

Les fixtures `ready-label:` **écrites à la main** sont à vérifier une par une :
`engine.rs` l. 8484 / 8990, `dispatcher.rs` l. 8922 (`#2108`) et l. 9746
(`#1614`). Deux issues possibles pour chacune — soit elle n'atteint pas la garde
(elle sort avant, faute de wrapper différé vivant), soit elle l'atteint et doit
être renumérotée dans la bande 990 000+ avec son payload. La DoD 5 le nomme comme
une vérification, pas comme une hypothèse.

**D9. Le jeton sous test.** `resolve_github_token` rend `None` sous test (aucun
PAT configuré), donc la garde ferait fail-closed `NotNow` et rougirait les 21
tests pour une raison qui n'a rien à voir avec leur sujet. Remède : poser un
jeton de test par défaut dans `test_dispatcher_with` (l. 6081-6115) **avant** le
hook `tweak`, pour qu'un test puisse le vider et exercer précisément le cas
« jeton absent » d'AC2. Rampe de sortie nommée, si cette pose rougit des tests
voisins : déplacer la pose dans un helper spécifique au reaper et migrer les 21
sites d'appel mécaniquement.

**Le seam de test est celui qui existe déjà.** Le double `#[cfg(test)] run_gh`
avec le routage par motif d'argv de mika#3047 —
`set_canned_stdout_for(GhSubject::Issue(n), "--json", payload)`. Pas de trait
d'injection, pas de fonction `_with` dupliquée : c'est la plus grosse économie de
lignes face au seam par trait de mika#2995, et c'est ce qui fait tenir ce vol
sous la borne.

Sept tests, six dans `executor.rs` (sur `rearm_fixture()`, l. 12224, qui crée
déjà `label: "ready-label: x/y#2045"`) et un bout-en-bout dans `engine.rs` :

| # | AC | cas | attendu |
|---|---|---|---|
| T1 | AC2 | contrôle positif : OPEN + `ready` (la forme des 4 premiers cas du tableau) | `Rearmed`, un lanceur, comme aujourd'hui |
| T2 | AC1, AC2 | `ready` retiré | `ConsentWithdrawn { ready_absent_at_rearm }`, parente `failed`, ligne d'audit, **zéro** lanceur |
| T3 | AC1, AC2 | ticket `CLOSED` | `ConsentWithdrawn { issue_closed_at_rearm }`, idem |
| T4 | AC2 | relecture en erreur (payload illisible) | `NotNow`, parente **reste** `pending`, zéro lanceur |
| T5 | AC2 | jeton absent | `NotNow`, cause `rearm_reread_no_token` |
| T6 | AC3 | parente d'une autre classe | `Rearmed` **et** `recorded_calls()` sans aucun appel `gh` |
| T7 | AC1 | bout-en-bout à travers `reap_orphaned_pending_issue_tasks` | parente terminale, aucun wrapper créé, audit posé |

T1 est le contrôle positif qu'AC2 exige nommément : sans lui, « la garde refuse
tout » et « la garde refuse ce qu'il faut » rendent les mêmes six tests verts.

## 6. Découpage en vols

**Un seul vol, et voici pourquoi il tient.** Le total estimé est de 387 lignes
hors `docs/`, contre une borne par vol de 429. Trois leviers de compaction, tous
acquis et non spéculatifs :

- la garde est écrite **une fois** (D1), donc les trois bras appelants coûtent
  ~6-8 lignes chacun au lieu d'un prédicat complet chacun ;
- le seam de test **existe déjà** (D8), là où mika#2995 avait dû livrer un trait
  d'injection et un doublon `_with` ;
- `rearm_fixture()` et `seed_pending_issue_parent()` existent et portent déjà un
  label `ready-label:`, donc les sept tests sont des corps, pas des échafaudages ;
  et le décalage du numéro **dans** la fixture économise 21 migrations de sites.

Aucun reste n'est renvoyé à un vol suivant. Le ticket est sous gel jusqu'au 01/12 :
ce vol n'ouvre aucun ticket de suivi, et un reste éventuel irait en commentaire
sur mika#3050 ou dans la PR.

## 7. AC4 — la sonde

Fenêtre : les **14 jours** qui suivent le `make deploy` de ce vol.

Mesure : le nombre de ré-armements de parentes `ready-label` dont le ticket ne
portait pas `ready` à l'instant du ré-armement, lu dans la **timeline du ticket**
(la même méthode que le tableau du § 0, pas le compteur interne — un compteur
interne mesurerait la garde, pas le défaut).

Cible : **0**.

Dénominateur, et c'est lui qui rend la mesure lisible : le nombre de retraits de
`ready` survenus **pendant qu'une parente attendait**. Un compte nul avec un
dénominateur nul s'écrit **« aucun croisement sur la fenêtre »** — c'est l'absence
d'occasion, pas une garde attestée (classe mika#2205). Un compte nul avec un
dénominateur non nul est la garde attestée.

Greps de lecture : `ready_absent_at_rearm`, `issue_closed_at_rearm`,
`rearm_reread_failed`, `rearm_reread_no_token` — plus
`stuck_pending_task_rearmed`, `stale_blocked_dispatch_rearmed` et
`deferred_dispatch_rearmed` pour le numérateur du tableau du § 0.

**L'absence de ces lignes ne signifie pas « aucun refus » mais « binaire
antérieur au vol »** (motif mika#2340) : l'instrument n'est pas vivant avant
`make deploy`.

## 8. Definition of Done

1. `ready_label.rs` porte `RearmConsent`, `reread_issue_consent_via_gh` et les
   trois constantes de D6. Aucun autre module ne construit la commande `gh issue
   view` du consentement de ré-armement.
2. `RearmOutcome` porte `ConsentWithdrawn { reason }`. Les trois `match`
   appelants la nomment explicitement, sans bras `_ =>`.
3. `rearm_deferred_callback` consulte la garde en position 3, et elle seule écrit
   le terminal et la ligne d'audit.
4. Les trois chemins — reaper (l. 2162), stale-blocked (l. 2630),
   `noop_completion` (l. 4037) — héritent de la garde sans la dupliquer.
5. Le plancher `FIXTURE_ISSUE_FLOOR` n'est **pas** desserré ; aucune allowlist
   n'est ajoutée. Les fixtures `ready-label:` écrites à la main (`engine.rs`
   l. 8484 / 8990, `dispatcher.rs` l. 8922 / 9746) sont vérifiées une par une :
   soit elles n'atteignent pas la garde, soit elles sont renumérotées dans la
   bande 990 000+ avec leur payload canné.
6. Les sept tests du § 5 passent, T1 et T6 compris.
7. `cargo test`, `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`
   et `bash scripts/verify-pipeline.sh origin/main` sont verts.
8. La PR porte `Code review: delegated to mika-qa (mika#2694)` et ne contient ni
   invocation de `ce-simplify-code` ni de `ce-code-review`.

## 9. Acceptance criteria

- **AC1.** Avant de ré-armer une parente `ready-label`, sur les deux chemins du tableau, le moteur relit le ticket à la forge : ouvert, et portant `ready`.
  - Si l'un des deux manque, aucun lanceur n'est créé, et aucun tour mika-dev n'a lieu.
  - La parente quitte la file dans un état terminal qui libère le prochain `ready`, comme `failed` aujourd'hui.
  - Une ligne d'audit nomme la raison : `ready_absent_at_rearm` ou `issue_closed_at_rearm`.
- **AC2.** Fail-closed : si la relecture échoue, pas de ré-armement à ce tick (`RearmOutcome::NotNow`), avec la cause nommée dans le log.
  - Un test par cas, dans le même appel : `ready` retiré, ticket fermé, relecture en erreur.
  - Le contrôle positif (`ready` encore posé, la forme des 4 premiers cas du tableau) ré-arme comme aujourd'hui.
- **AC3.** Les parentes d'une autre classe que `ready-label` gardent leur comportement actuel. Un test le garde.
- **AC4.** Sonde : sur les 14 jours qui suivent le deploy, on compte les ré-armements de parentes `ready-label` dont le ticket ne portait pas `ready` à l'instant du ré-armement (timeline du ticket), puis on l'écrit ici. La cible est 0. Un compte nul sans aucun retrait de `ready` pendant qu'une parente attendait s'écrit « aucun croisement sur la fenêtre ».

## 10. Fire-Disposition

**Gate N/A — ce vol ne livre aucun détecteur.** La section est écrite pour dire
*pourquoi*, pas pour inventer une disposition (mika#2306 : « ne l'invente pas »).

Les sept tests du § 5 n'en sont pas : leur chemin de succès est « le comportement
attendu est arrivé », pas « aucune violation trouvée » — précédent établi par le
plan mika#3047 § 10. Aucune règle de lint, aucun scan de source, aucune garde CI,
aucun validateur de schéma n'est livré.

**Un détecteur existant est néanmoins traversé, et son traitement est nommé plutôt
que laissé à l'implémentation** : le plancher `FIXTURE_ISSUE_FLOOR` (mika#2713)
rougit sur les 21 fixtures existantes dès qu'une lecture `gh` entre sur le chemin
du reaper. Le remède est la **renumérotation** (DoD 5), jamais une exception en
allowlist — mika#2713 dit en toutes lettres qu'il n'y en a pas, et en ajouter une
rouvrirait la fuite que ce plancher a fermée.

## 11. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/ready_label.rs` — `RearmConsent`, `reread_issue_consent_via_gh`, 3 constantes | 70 |
| `crates/mika-agent/src/skills/executor.rs` (prod) — variante `ConsentWithdrawn`, paramètre jeton, garde en position 3, terminal + audit | 87 |
| `crates/mika-agent/src/task_engine/engine.rs` (prod) — jeton résolu une fois par passe sur les deux balayages + 2 bras de `match` | 35 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` (prod) — résolution du jeton + 1 bras de `match` | 20 |
| tests `skills/executor.rs` — T1 à T6 + payload canné dans `rearm_fixture` | 120 |
| tests/fixtures `task_engine/engine.rs` — décalage au-dessus du plancher dans `seed_pending_issue_parent`, payload canné, T7 bout-en-bout | 55 |

Total estimé : 387 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

## 12. Ordre de commit

1. **`ready_label.rs`** — `RearmConsent`, `reread_issue_consent_via_gh`, les trois
   constantes, et ses tests unitaires de parsing (OPEN+`ready`, OPEN sans `ready`,
   CLOSED, `state` hors contrat, `labels` absent). Compile et passe seul, sans
   appelant.
2. **`gh_process.rs` / fixtures** — le décalage au-dessus de `FIXTURE_ISSUE_FLOOR`
   dans `seed_pending_issue_parent`, le payload canné OPEN+`ready`, le jeton de
   test dans `test_dispatcher_with`, les ~4 littéraux d'assertion. **Suite verte à
   ce commit, avant toute garde** — c'est ce qui sépare « les 21 tests rougissent
   à cause du décalage » de « ils rougissent à cause de la garde ».
3. **`skills/executor.rs`** — la variante `ConsentWithdrawn`, le paramètre jeton,
   la garde en position 3, le terminal et l'audit. Les trois `match` appelants
   cassent la compilation : c'est l'effet voulu de D5.
4. **`engine.rs` + `dispatcher.rs`** — les trois bras de `match` et la résolution
   du jeton une fois par passe. Compilation rétablie.
5. **Les sept tests** — T1 à T6 dans `executor.rs`, T7 dans `engine.rs`.
6. **Vérification de clôture** — `cargo test`, `cargo clippy`, `cargo fmt --check`,
   `make verify-bundled-skills`, `bash scripts/verify-pipeline.sh origin/main`.
