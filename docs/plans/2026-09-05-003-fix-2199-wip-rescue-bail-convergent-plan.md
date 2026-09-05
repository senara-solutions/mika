---
issue: 2199
repo: senara-solutions/mika
type: fix
module: mika-agent/wip_rescue
tags: [wip-rescue, livelock, labels, label-sync, audit-events, loop-substrate, convergence]
problem_type: silent-write-failure
status: groomed
---

# mika#2199 — le bail-to-human n'écrit pas son exclusion, et la boucle repasse

**Issue :** senara-solutions/mika#2199
**Branche :** `fix/2199/wip-rescue-le-d-mon-livelock-si-le-label`
**Palier :** Tier 2 — ralentit la boucle.

---

## 1. Le fait, relu dans le code

`bail_to_human` (`crates/mika-agent/src/wip_rescue.rs:796-856`) avale l'échec de son
unique écriture d'état :

```rust
if let Err(e) = gh(
    &["pr", "edit", &pr_number.to_string(), "--repo", DEFAULT_REPO,
      "--add-label", HUMAN_REVIEW_LABEL],
    token,
).await
{
    warn!(pr_number, error = %e, trace_id, "wip_rescue_error");   // :820
}
// …commentaire PR (:829-844), même traitement
log_audit(db, session_id, "wip_rescue_bail_to_human", …).await;   // :846
ChainOutcome::Bailed(reason)                                      // :855 — inconditionnel
```

La fonction retourne `Bailed` **que le label ait été posé ou non**. Or ce label est le
**seul** mécanisme d'exclusion. Le filtre d'éligibilité (`wip_rescue.rs:403-421`) :

```rust
.filter(|pr| pr.has_label(WIP_RESCUE_LABEL) && !pr.has_label(HUMAN_REVIEW_LABEL))
…
eligible.sort_by(|a, b| b.0.cmp(&a.0));   // âge décroissant
let Some((age_secs, pr)) = eligible.into_iter().next() else { … };
```

Trois propriétés se composent en livelock :

1. l'exclusion vit **uniquement sur GitHub**, dans un label ;
2. l'écriture de ce label peut échouer sans que rien ne change d'état ;
3. le démon traite **une PR par tick** et choisit **la plus vieille** — une PR non parquée
   reste la plus vieille, donc ré-élue au tick suivant, indéfiniment.

Le cran de sûreté qu'on croirait présent ne l'est pas non plus : `bump_rescue_depth_metadata`
(`wip_rescue.rs:186-194`) n'est appelé que sur le chemin `Resumed` (`:578-584`). **Un bail
n'incrémente pas la profondeur.** La garde `rescue-depth-exceeded` (`:471-481`) ne peut donc
jamais s'armer sur une PR qui ne fait que bailer — et si elle s'armait, elle appellerait
`bail_to_human`, c'est-à-dire le chemin défaillant lui-même.

## 2. Pourquoi le label manquait, et pourquoi il va remanquer

`human-review-required` est déclaré dans `wip_rescue.rs:74` et **nulle part dans
`.github/labels.yml`** (225 lignes, 51 entrées ; `grep -c` → 0).

Le workflow de synchronisation (`.github/workflows/labels.yml`) tourne avec :

```yaml
- uses: EndBug/label-sync@…  # v2.3.3
  with:
    config-file: .github/labels.yml
    delete-other-labels: true
```

Mesure du 2026-09-05, `gh label list --limit 200` diffé contre `labels.yml` :

| labels vivants absents de `labels.yml` | labels déclarés absents du dépôt |
|---|---|
| **`human-review-required`** — et lui seul | *(aucun)* |

Le label créé à la main par l'orchestrateur à 16:05 est donc le **seul** point de drift du
dépôt, et `delete-other-labels: true` le supprimera au **prochain push touchant
`.github/labels.yml` sur `main`**. Ce fichier a été modifié quatre fois en dix jours
(`f1965cfc` 09-04, `b6070bba`, `04945721`, `46eeef98`). **Demi-vie du contournement manuel :
le prochain commit sur `labels.yml`, soit quelques jours.**

C'est la troisième occurrence de la même classe sur ce dépôt :

| # | code qui écrit | label non déclaré | correctif |
|---|---|---|---|
| 1 | `dispatch-lib.sh` | `dispatch:ssc` | `438d28a4` — déclaré pour que la synchro cesse de l'élaguer |
| 2 | `auto_pull.rs:1684` | `operator-review`, `blocked` | PR #2128 + #2130 — 48 échecs, `ready` jamais retiré |
| 3 | `wip_rescue.rs:806` | `human-review-required` | **ce ticket** |

Enseignement déjà écrit, jamais appliqué à ce module :
`docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md`.

## 3. La trace, mesurée

**Précaution de mesure : chaque ligne de `/var/log/mika/server.log` est écrite deux fois.**
Vérifié en dédupliquant hors horodatage — les deux copies partagent le `trace_id` et sont
séparées de quelques dizaines de microsecondes. Tous les comptes ci-dessous sont **les comptes
bruts divisés par deux**. Le corps du ticket cite des comptes bruts (« 26 bails ») ; ce n'est pas
une divergence, c'est la même trace lue avec ou sans cette correction.

```
2532  wip_rescue: running auto-resume scan
1079  wip_rescue_resume_attempt
1079  wip_rescue: scan complete      ← dont resumed:0 = 1077, resumed:1 = 2
  17  wip_rescue_bail_to_human       ← dont 14 sur #2197, 3 sur #2187
  17  wip_rescue_error               ← un par bail : l'échec --add-label
   2  wip_rescue_success             ← PR #2180 et #2198, sur toute l'histoire du démon
```

Un `wip_rescue_error` **par** `wip_rescue_bail_to_human` : l'échec d'écriture est
systématique, pas intermittent.

```
{"level":"WARN","message":"wip_rescue_error","pr_number":2197,
 "error":"gh exit code 1: 'human-review-required' not found",
 "target":"mika_agent::wip_rescue"}
```

**La preuve du blocage une-par-tick** — le démon est resté sur #2197 de 09:43 à ~16:10,
puis a repris #2198 **avec succès à 16:11:22**, dans la minute où #2197 est passée `CLOSED` :

| horodatage | fait |
|---|---|
| 09:43 → 16:05 | 28 bails sur #2197, un `wip_rescue_error` chacun |
| ~16:10 | #2197 fermée (le correctif est passé par #2196) |
| 16:11:22 | `wip_rescue_success pr_number=2198` |

Ce n'est pas le contournement manuel qui a débloqué la file : **ni #2187 ni #2197 ne portent
`human-review-required` aujourd'hui**, label créé compris. Le bail n'a jamais réussi à parquer
quoi que ce soit ; la file s'est libérée parce que les PRs sont sorties de l'état `open`.

## 3 bis. Deux faits de terrain qui conditionnent la vérification

Ils ne changent pas le diagnostic — le livelock est établi par la trace du §3 — mais ils
déterminent ce qu'une vérification en production peut et ne peut pas prouver.

**(a) `resolve_repo_dir` n'a aucun repli.** `wip_rescue.rs:785-793` lit `MIKA_WIP_RESCUE_REPO_DIR`
et rien d'autre ; absente, la chaîne rend `Skipped("no_repo_dir")` **avant** toute opération git
(`:512-514`). Or cette variable n'est présente ni dans `~/.mika/.env`, ni dans l'unité systemd,
ni dans l'environnement du processus `mika --agent mika-dev` vivant (lu dans `/proc/<pid>/environ`).
Les bails `rebase-failed` du §3 prouvent qu'elle **était** résolue pendant la fenêtre de l'incident.
Les 1077 scans à `resumed:0` pour 17 bails laissent ~1060 issues qui ne sont ni un succès ni un bail :
`Skipped`, très probablement `no_repo_dir`. **La trace ne peut pas le confirmer** — `wip_rescue_skipped`
est un `debug!` (`:447`) et aucun DEBUG de `mika_agent` n'est écrit. À énoncer comme une inférence,
pas comme une mesure.

**(b) Le démon ne tourne plus depuis le 2026-09-05T16:20:00Z.** Dernier
`wip_rescue: running auto-resume scan` à 16:20 ; le cron est `0 */5 * * * *`
(`server/mod.rs:101`), donc une centaine de scans manquent à 19:55. `MIKA_DEV_WIP_RESCUE`
n'est définie nulle part (le dernier « disabled » date du 2026-08-26), et le processus
`mika-dev` courant a démarré à 18:22 sans émettre un seul scan depuis.

**Conséquence pour ce plan, et elle est bornée :** la vérification du §7 qui dépend du démon —
observer un bail réel se parquer et le scan avancer — **exige que le démon tourne et que
`MIKA_WIP_RESCUE_REPO_DIR` soit résolue**. Si ce n'est pas le cas au moment de la PR, cette
vérification-là est *impossible*, et il faut le dire ainsi plutôt que de la déclarer passée :
les tests du §4.5, eux, ne dépendent d'aucun des deux.

**Ce qui n'est pas fait ici :** diagnostiquer (a) ou (b). Ni l'un ni l'autre n'est le défaut que
#2199 décrit, et aucun des deux n'a encore l'évidence qu'un ticket demande. Ils sont signalés à
l'opérateur avec le rapport de grooming, à investiguer séparément.

## 4. Ce que ce plan livre

### 4.1 — L'écriture d'exclusion cesse d'échouer sur un label absent (AC1)

Dans `bail_to_human`, remplacer l'appel unique par la séquence **tenter → créer → retenter**,
transposition directe du seul précédent du dépôt, `_stamp_pr_origin`
(`skills/bundled/_shared/dispatch-lib.sh:5014-5019`, mika#2026), dont le commentaire de tête
énonce déjà le pattern : *« a failed edit is retried once behind an idempotent `label create` »*.

Nouvelles constantes à côté de `HUMAN_REVIEW_LABEL` (`wip_rescue.rs:74`) :

```rust
const HUMAN_REVIEW_LABEL_COLOR: &str = "D93F0B";
const HUMAN_REVIEW_LABEL_DESC: &str =
    "wip-rescue: le démon a bailé vers un humain (conflit/erreur), à résoudre à la main";
```

Valeurs reprises **à l'identique** du label vivant, qui a été accepté par GitHub : 82
caractères, sous la limite de 100 qui a mordu deux fois (#2130, #2168).

`bail_to_human` retourne désormais si le parquage GitHub a réussi, au lieu de le taire.

### 4.2 — L'exclusion ne dépend plus d'un label (AC1 seconde voie, AC2)

Un marqueur **durable et clé par PR** est écrit dans `audit_events`, **inconditionnellement**,
avant toute tentative GitHub :

```rust
const BAILED_MARKER_TOOL: &str = "wip_rescue_bailed";
fn bailed_marker_key(pr_number: u64) -> String { format!("pr:{DEFAULT_REPO}#{pr_number}") }
```

Le filtre d'éligibilité consulte ce marqueur via
`count_recent_audit_events_for_target(BAILED_MARKER_TOOL, &key, since)`
(`async_db.rs:700`).

**Précédent réutilisé, pas inventé.** `ci_success_handler.rs:194-230` (mika#1869) fait
exactement cela — un marqueur `audit_events` clé par PR, *« audit-durable gate … that survives
a process restart the in-memory map cannot »*. Même convention de clé `pr:{repo}#{n}`, même
absence de migration : le schéma est free-form et `guards.rs:738-742` cite nommément
`wip_rescue (mika#1852)` comme précédent de cette convention.

**Deux écarts assumés d'avec ce précédent, et leurs raisons :**

- **Fail-closed, là où `ci_success_handler` est fail-open.** Si la lecture d'audit échoue, on
  traite la PR comme **déjà bailée** (on l'exclut). Chez `ci_success_handler`, fail-open protège
  un merge légitime ; ici l'arbitrage est inverse et sans coût : **une PR qui a bailé est par
  définition destinée à un humain**. L'exclure à tort ne perd aucun travail — elle reste ouverte,
  étiquetée, commentée. La re-tenter en boucle coûte la file entière. Le choix est asymétrique,
  donc il se tranche.
- **Fenêtre `since` = toute la rétention**, pas 60 s. Le bail est terminal par conception
  (plan #1852 § 4 : *« End chain — NO further auto-attempts »*), pas un dédup de rafale.

**Demi-vie de cette exclusion, nommée :** `compact_old_audit_events(90)` (`server/mod.rs:629`)
purge les lignes après **90 jours**. Une PR draft encore ouverte après 90 jours qui se ferait
re-tenter **une** fois n'est pas un livelock — et son âge est en soi le signal qu'un humain
doit la regarder. Écart accepté, mesuré, borné.

**Portée `agent_id` :** `count_recent_audit_events_for_target` compte dans le scope de l'agent
courant. Écriture et lecture ont lieu dans le même processus, sous `mika-dev`
(`server/mod.rs:1596` n'enregistre le scan que pour cet agent) — le scope est correct, pour la
raison que `ci_success_handler.rs:198-201` documente pour son propre cas.

### 4.3 — Le scan avance dès le tick courant (AC2)

Le marqueur est consulté **pendant** le filtrage, donc la PR parquée disparaît de `eligible` et
`.next()` rend la suivante **au même tick** — pas seulement au tick d'après. Le ticket
n'exigeait que le tick suivant ; le tick courant tombe de la forme du code, ce n'est pas un
élargissement.

Coût : la consultation se fait sur les seuls candidats déjà retenus par âge, en parcours
ordonné avec court-circuit au premier non-bailé. Cas nominal : **une** lecture par tick de
5 min. Borne haute : la taille de la liste, plafonnée à 100 par `--limit 100`
(`wip_rescue.rs:895-896`).

### 4.4 — Le label est déclaré, et une garde l'y maintient (AC4)

- Entrée ajoutée à `.github/labels.yml`, section *Automation*, à côté de `wip-rescue`
  (`:178-180`) — mêmes couleur et description que ci-dessus.
- Test de registre calqué **littéralement** sur `auto_pull.rs:3239-3258` :

```rust
#[test]
fn human_review_label_is_declared_in_labels_yml() {
    let yml = include_str!("../../../.github/labels.yml");
    let declared = |name: &str| yml.contains(&format!("- name: {name}"));

    assert!(declared(WIP_RESCUE_LABEL));                          // contrôle positif
    assert!(!declared("a-label-nobody-has-ever-declared"));       // contrôle négatif
    assert!(declared(HUMAN_REVIEW_LABEL), "…");                   // la garde
}
```

L'assertion porte sur **les constantes**, jamais sur des littéraux recopiés — sans quoi elle
compterait un nom que rien n'écrit et resterait verte à jamais
(`docs/solutions/best-practices/a-count-assertion-on-an-event-name-nothing-emits-is-always-green-2026-08-31.md`).
`wip_rescue.rs` ne manipule que ces deux labels ; la garde est donc **exhaustive pour ce module**.

**Tie-back explicite :** la garde équivalente pour `auto_pull.rs` — dont la garde actuelle
(`auto_pull.rs:3239`) ne couvre que `REFUSAL_LABEL` — reste **mika#2127 AC2**, ouvert et
`operator-gated`. Ce plan ne l'élargit pas et ne l'attend pas. C'est la découpe que
PR #2128 a elle-même posée sur cette surface : *« La garde structurelle qui empêche le retour …
reste dans mika#2127. »*

### 4.5 — Le rejeu (AC3)

Deux niveaux, et il faut dire lequel prouve quoi.

**(a) Test de sélection, permanent et falsifiable.** Extraire la sélection en fonction pure :

```rust
fn select_eligible(drafts: Vec<DraftPr>, now: &DateTime<Utc>, threshold: i64,
                   is_bailed: &dyn Fn(u64) -> bool) -> Option<(i64, DraftPr)>
```

Cas de test : deux drafts `wip-rescue`, aucune ne portant `human-review-required`, la plus
vieille marquée bailée. Attendu : la **seconde** est rendue. **Contrôle négatif obligatoire :**
le même cas avec un prédicat qui rend toujours `false` — c'est-à-dire le comportement de `main`
— rend la première, en boucle. Le test échoue donc sans le correctif, ce qui est la seule
manière qu'il ait de valoir quelque chose.

**(b) Rejeu de bout en bout, avec un `gh` défaillant.** Faisable : `run_gh_subprocess`
(`tools/pr_merge_with_gate.rs:755`) fait `Command::new("gh")`, résolu via `PATH`, et
`scrub_mika_env_vars` ne retire que `MIKA_*` et `GH_TOKEN` (`skills/executor.rs:33-49`) —
**`PATH` est hérité**. Un faux `gh` déposé dans un répertoire temporaire en tête de `PATH`,
sous `#[serial_test::serial]` (crate déjà présent, `Cargo.toml:84`, employé en
`validate.rs:326`, `tier_guard.rs:370`), reproduit `'human-review-required' not found` puis le
succès après `label create`. Scénarios nommés d'après `test_stamp_pr_origin.sh:74`
(`nominal`, `needs-label`).

*Le faux `gh` ne doit communiquer par aucune variable préfixée `MIKA_` — elles sont purgées
avant l'exec.*

Aucun test Rust du dépôt ne manipule `PATH` aujourd'hui : c'est le premier. Si `(b)` s'avère
instable en CI, **`(a)` est le livrable non négociable** et `(b)` dégrade en script bash
exécuté à la main, sa sortie collée dans la PR. Le ticket demande la sortie dans la PR, pas un
harnais permanent.

**Ce qui ne sera pas fait :** rendre le binaire `gh` surchargeable par variable d'environnement.
`run_gh_subprocess` a huit consommateurs ; les toucher pour un test dépasse ce ticket.

## 5. Périmètre

**Fichiers touchés**

| fichier | action |
|---|---|
| `crates/mika-agent/src/wip_rescue.rs` | constantes, `ensure_label`, `bail_to_human`, marqueur, filtre, `select_eligible` + tests |
| `.github/labels.yml` | une entrée (4 lignes) |

**Hors périmètre**

- La garde exhaustive des labels d'`auto_pull.rs` — **mika#2127 AC2**.
- L'ordre appliquer-puis-retirer d'`auto_pull` — déclaré hors périmètre par #2127, inchangé ici.
- Rendre `WIP_RESCUE_CRON` (`server/mod.rs:101`) configurable — non demandé.
- Une abstraction GitHub mockable pour les huit consommateurs de `run_gh_subprocess`.
- L'incrémentation de `wip_rescue.depth` sur bail. Constatée §1 comme absente, mais avec
  l'exclusion durable de §4.2 elle ne rachète rien : le bail devient terminal par le marqueur.
  Le noter dans la PR, ne pas l'implémenter.
- `stale-against-main` (`.github/workflows/wip-staleness-check.yml:93,109`) : même forme
  fragile (`|| true`), mais son label **est** déclaré (`labels.yml:186`) — pas de défaut ici.

## 6. Note de lecture sur AC1 — pourquoi les deux voies

La correspondance AC → livrable est portée par la section `## Acceptance criteria` ci-dessous,
qui est la forme canonique. Cette note ne traite que le point qui demande un arbitrage.

Le ticket propose pour AC1 « soit … soit … ». Ce plan fait **les deux**, pour deux raisons
distinctes et mesurées : la création idempotente seule est défaite par `delete-other-labels`
(§2) ; le marqueur seul laisserait la PR sans étiquette visible sur GitHub, où l'humain qui
doit la reprendre va la chercher. Ni l'un ni l'autre n'est redondant.

## Acceptance criteria

Transcrits **littéralement** du corps de senara-solutions/mika#2199, chacun avec le test qui
le rend falsifiable. Les intitulés `AC1`–`AC4` ne sont pas renommés.

- **AC1** — Le bail-to-human ne peut pas livelocker : soit le démon **crée le label**
  `human-review-required` s'il manque (idempotent, au démarrage ou à la première bail), soit il
  marque la PR bailée par un moyen qui ne dépend pas d'un label pré-existant (état/commentaire)
  et qui l'exclut du prochain scan.
  → **Livré par §4.1 *et* §4.2** (les deux voies, raisons au §6). Testé par le test de sélection
  du §4.5(a) : marqueur présent ⇒ la PR n'est pas ré-élue.

- **AC2** — Une bail dont l'écriture d'exclusion échoue (label absent, gh en erreur) **ne laisse
  pas la PR éligible** : le démon avance à la PR suivante au tick d'après (pas de blocage
  one-per-tick sur une PR qu'il ne peut pas parquer).
  → **Livré par §4.2** (marqueur écrit **avant** et **indépendamment** de toute tentative GitHub)
  **et §4.3**. Testé par le cas « deux drafts, la plus vieille marquée bailée ⇒ la seconde est
  rendue » du §4.5(a). Le fail-closed sur erreur de lecture (§4.2) ferme la dernière voie par
  laquelle une PR non parquable resterait éligible.

- **AC3** — Rejeu : une PR wip-rescue en conflit de rebase, dans un repo SANS le label
  `human-review-required` → sur `main` le démon boucle (re-tente), avec le correctif il parque la
  PR et avance. Sortie collée dans la PR.
  → **Livré par §4.5**, en deux niveaux dont le premier est non négociable : (a) test de sélection
  **avec contrôle négatif** — un prédicat toujours-faux, c'est-à-dire le comportement de `main`,
  doit rendre la première PR en boucle, donc le test **échoue sans le correctif** ; (b) rejeu
  bout-en-bout avec faux `gh` sur `PATH`, dégradable en script manuel. Dans les deux cas la sortie
  est collée dans la PR.

- **AC4** — Les labels que le démon écrit (`human-review-required`, et les autres qu'il pose) sont
  **documentés comme requis** et/ou seedés (ex. `.github/labels.yml`), pour que le repo les ait
  toujours.
  → **Livré par §4.4**. Mesure qui borne « les autres » : `wip_rescue.rs` ne manipule que
  `WIP_RESCUE_LABEL` et `HUMAN_REVIEW_LABEL`, et n'en **pose** qu'un seul ; il n'en retire ni
  n'en crée aucun autre. « Les autres » est l'ensemble vide, donc la garde du §4.4 est
  **exhaustive pour ce module**. Testé par le test de registre, assertions portées sur les
  constantes et non sur des littéraux recopiés.

## Fire-Disposition

Ce plan livre des **détecteurs** (§4.4 test de registre, §4.5(a) test de sélection, §4.5(b) rejeu).
Disposition pré-spécifiée pour chacun, avant qu'il ne se déclenche.

| détecteur | ce qu'il détecte | disposition quand il rougit |
|---|---|---|
| §4.5(a) test de sélection | une PR marquée bailée reste ré-élue | **Halte et remontée.** C'est le défaut même de #2199 ; un rouge ici invalide le correctif, il ne se contourne pas. |
| §4.5(a) **contrôle négatif** | le test resterait vert sans le correctif | **Halte et remontée.** Un contrôle négatif qui ne rougit pas sur `main` prouve que le test ne mesure rien ; le corriger avant tout autre travail. |
| §4.4 test de registre | `human-review-required` (ou `wip-rescue`) absent de `labels.yml` | **Halte et remontée.** C'est exactement la classe que ce ticket ferme ; un rouge signifie que la déclaration a été perdue. |
| §4.4 contrôle négatif (retrait temporaire de `wip-rescue`) | la garde ne discrimine pas | **Halte et remontée**, même raison. |
| §4.5(b) rejeu bout-en-bout | l'écriture d'exclusion échoue encore de bout en bout | **Deux cas, distingués.** Rouge *sur le comportement* → halte et remontée. Rouge *par instabilité du harnais* (`PATH`, ordonnancement des tests) → **dégrader en script manuel**, coller la sortie dans la PR, et le dire comme tel. Ne jamais désarmer §4.5(a) pour faire passer §4.5(b). |
| vérification post-merge du §7 (survie du label) | `delete-other-labels` a supprimé le label | **Halte et remontée.** Cela signifierait que la déclaration n'a pas pris ; le correctif serait redevenu un tapis roulant. |
| vérification en production (§3 bis) | — | **Non armée si le démon ne tourne pas.** Dans ce cas, écrire « non effectuée » et pourquoi. Ne jamais la déclarer passée sur la foi des tests. |

Aucun de ces détecteurs n'est autorisé à être mis en sourdine pour faire passer la PR. Un
détecteur dont on ne veut pas la disposition est un détecteur qu'il ne fallait pas écrire.

## 7. Vérification

- `cargo build`, `cargo clippy --tests`, `cargo test -p mika-agent`, `cargo fmt --check`.
- Le test de §4.5(a) **échoue** sur `main` — à démontrer et coller dans la PR.
- Le test de §4.4 échoue si l'on retire temporairement `wip-rescue` de `labels.yml` — à
  démontrer et coller (contrôle négatif au-delà du nom inventé, comme l'exige #2127 AC3).
- Après merge sur `main`, la synchro des labels tourne (le push touche `.github/labels.yml`) :
  vérifier que `human-review-required` **survit** — c'est l'événement même qui aurait détruit le
  contournement manuel, donc la mesure qui distingue le correctif du tapis roulant. C'est la seule
  vérification post-merge qui ne dépende ni du démon ni de sa configuration.
- Vérification en production du comportement du démon — **conditionnelle, cf. §3 bis(b)** : elle
  suppose un scan qui tourne et `MIKA_WIP_RESCUE_REPO_DIR` résolue. Si l'une des deux manque au
  moment de la PR, l'écrire comme **non effectuée**, avec la raison. Ne pas la présenter comme
  passée sur la foi des tests : un test vert dit que le code fait ce qu'on croit, pas que le démon
  tourne.

## 8. Risques

| risque | traitement |
|---|---|
| Le token n'a pas le droit de créer un label | La séquence dégrade proprement : `label create` échoue, le marqueur d'audit est déjà écrit, la PR est exclue quand même. C'est précisément ce que §4.2 achète. |
| Test `PATH` instable en CI | §4.5 : `(a)` est le livrable dur, `(b)` dégrade en script manuel. |
| Purge d'audit à 90 jours | §4.2 : borne nommée, conséquence bornée à une re-tentative sur une PR de plus de 90 jours. |
| Description > 100 caractères | Valeur reprise du label vivant, 82 caractères. Deux incidents antérieurs (#2130, #2168) sur ce mur exact. |
