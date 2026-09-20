# mika#2423 — Un AC comportemental non exécutable dans le budget n'est plus un `hold[review]`

> Ticket : `senara-solutions/mika#2423` — p2, tier-2.
> Preuve fondatrice : QA du 2026-09-20 09:11Z sur mika#2418 (fix #2413).

---

## Goal Capsule

Un AC **comportemental** dont la vérification demande une compilation ne peut pas
être exécuté par mika-qa : le budget de son outil `run_shell` est de **30 s**
(mika#2276), et une compilation du workspace `mika-agent` prend plusieurs minutes.
Aujourd'hui la QA tente quand même, échoue par timeout, et rend `hold[review]`
« test non exécuté » — **alors que le test existe, compile, et passe en CI**. Chaque
tentative laisse en plus un `cargo` orphelin (`ppid=1`) qui tient le lock `target/`
et garantit l'échec des tentatives suivantes.

Ce travail ferme la classe par **deux moitiés structurelles et une moitié de
prompt** :

1. **Le moteur refuse la commande au lieu de la laisser expirer** — une commande de
   build soumise à un budget qui ne peut pas la contenir est refusée *avant le
   spawn*, avec une erreur qui nomme la classification à employer et dit
   explicitement qu'il s'agit d'un **refus de politique, pas d'un échec d'outil**.
   Aucun `cargo` n'est lancé, donc aucun orphelin, donc aucune spirale de contention.
2. **Le moteur ne fuit plus de petits-enfants** — filet de défense en profondeur pour
   toute autre commande longue : le handler exec devient chef de groupe de processus
   et le groupe entier est tué à l'expiration.
3. **Le prompt cesse de se contredire** — la règle d'intégrité « toute étape sautée
   par un échec d'outil plafonne le verdict à `hold[review]` » et la règle
   « tous les AC `✅` ou `⏭️` → la vérification passe » se contredisaient exactement
   sur cette population. La contradiction est tranchée par écrit.

**Ce que ce travail ne fait PAS :** il ne donne pas à mika-qa la lecture des logs
CI. Cette piste — la lettre du ticket — est examinée, chiffrée et **refusée** au § *Le
refus mesuré*, sur la base d'un fait vérifié dans le code : **la porte de merge exige
déjà la CI verte**, sans limite de 30 s, un cran en aval du verdict.

---

## Product Contract

### Ce que le ticket a mesuré, et ce que la lecture du code déplace

Le corps du ticket est juste sur le **symptôme** et sur l'**effet de bord**. Trois
mesures déplacent le **diagnostic**, et ce déplacement est le premier livrable.

**M1 — Le prompt interdit déjà cette compilation, et depuis dix jours.**
`skills/bundled/qa-review/system_prompt.md:293`, posé par mika#2276 (`e17ba3a7`,
2026-09-10) :

> **Never compile inside the review turn (mika#2276).** `cargo build/test/clippy`,
> `npm run build` and their kin are **not** available here, whatever an AC seems to
> ask. […] If a Behavioral AC needs a build, mark it `[⏭️] not verifiable within the
> review budget — requires a build`.

La QA du 2026-09-20 a fait **5+ tentatives** de `cargo test --release` contre cette
interdiction. Le budget de 30 s que le ticket constate *est* la moitié moteur de
mika#2276, donc le binaire est postérieur au correctif — et le prompt voyage avec le
binaire (`build.rs`). La règle était servie et n'a pas mordu : classe
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`. **Une règle de
prompt de plus ne ferme pas cette classe** ; c'est ce qui impose U1.

**M2 — Le `hold[review]` mesuré était CORRECT au regard du prompt tel qu'écrit.**
Deux règles se contredisent sur exactement cette population :

| site | texte | conséquence ici |
|---|---|---|
| `system_prompt.md:40` (Data Integrity, « *These rules override everything else* ») | « If any step was skipped **due to a tool failure**, the maximum verdict is `hold[review]` » | le timeout *est* un échec d'outil → plafond `hold[review]` |
| `system_prompt.md:428` (2.5.7) | « All ACs `✅` or `⏭️`: AC verification passes » | le `[⏭️]` de la ligne 293 → la vérification passe |

mika#2276 a livré l'échappatoire `[⏭️]` et **laissé la contradiction en place**. Le
modèle a appliqué la règle qui se déclare prioritaire. Le défaut n'est donc pas
« la QA est trop sévère » mais « le prompt lui demande deux choses opposées, dont
l'une se dit prioritaire ». C'est U3.

**M3 — L'orphelin a un site unique et il est lisible.** `skills/executor.rs:828`
(`execute_exec`) construit sa `Command` avec `.kill_on_drop(true)` (ligne 854) et
**sans** `.process_group(0)`. À l'expiration, `execute_skill_tool:645`
(`tokio::time::timeout`) laisse tomber le futur ; `kill_on_drop` envoie SIGKILL au
**seul enfant direct**, `handlers/run.sh`. Or ce handler finit sur `eval "$COMMAND"
2>&1` : `cargo` est son enfant, dans le groupe de processus de mika-spirit, et
survit — reparenté à init, `ppid=1`. C'est exactement le pid 145584 du ticket.
Le motif correct existe déjà **dans le même fichier** : `spawn_long_running_exec`
pose `.process_group(0)` (ligne 3497, mika#855).

### Le refus mesuré : la QA ne lira pas les logs CI

La « Règle proposée » du ticket demande que la QA lise le log du job `Check` pour y
chercher `test <chemin>::<nom> ... ok`. Trois faits vérifiés font refuser cette
piste, et ils sont donnés dans l'ordre de leur poids.

**(a) La CI verte est DÉJÀ une précondition structurelle du merge, en aval, sans
limite de 30 s.** `server/verdict_handler.rs:566` classe les checks sur la branche
`VERDICT: pass` ; `classification == CheckClassification::HasFailures` rend
`Passthrough` avec la liste des jobs rouges, et **ne merge pas**. Un `pass` sur une
PR dont la CI est rouge ne passe pas la porte ; un `pass` sur une CI *pending* passe
par l'auto-merge, qui n'aboutit qu'au vert (`ci_success_handler`, mika#571).
Autrement dit : **« ce test passe-t-il ? » est déjà tranché, par une porte qui
compile sans budget.** Faire lire la CI à la QA duplique cette porte — et une
duplication de porte est une porte qui peut diverger de l'autre (leçon
`grooming_marker`, mika#2158).

**(b) Deux barrages, un de prompt et un de moteur, protègent une propriété réelle.**
Le prompt (`:45`, règle d'intégrité) : « *Do NOT fetch or reason about GitHub CI
status through any tool* […] *Your scope is diff review and pipeline artifacts
only* ». Le moteur : `builtin_handlers.rs:1945`,
`QA_REVIEW_GH_ALLOWED = [("pr","review"),("pr","diff"),("pr","list"),("issue","view")]`
— `gh run view` est refusé **avant le spawn**. Ouvrir la voie demanderait donc
d'élargir *les deux*, dont une allowlist d'identité forge. Ce que cette frontière
protège est nommé : une QA qui lit « CI verte » finit par approuver **sur la CI**
plutôt que sur la revue de diff, et le seul relecteur du diff disparaît. La maison
sait ouvrir une exception ici (`:158`, la GitHub Advisory Database, « *explicitly
permitted here (and only here)* ») — mais cette exception-là achète un signal
**distinct de la CI** ; celle-ci achèterait un signal **identique** à une porte
existante.

**(c) Le coût d'exécution est incompatible avec le budget qu'on prétend contourner.**
`gh run view --log` sur une suite `cargo test` du workspace rend plusieurs dizaines
de Mo. Il faudrait cibler le job, filtrer, et rester sous 30 s — c'est-à-dire
reconstruire, dans le budget qui a causé le problème, un extracteur fragile dont
l'échec retomberait sur la ligne 40 et **rendrait le `hold[review]` qu'on ferme**.

**Ce que le refus coûte, nommé.** Le test négatif nº 2 du ticket — « test ABSENT du
CI (ignoré / feature-gated non couvert) → reste un vrai trou » — n'est plus couvert
par une preuve d'exécution mais par une **preuve structurelle** : le test est-il
présent dans le diff, et n'est-il pas exclu de la CI (`#[ignore]`, `#[cfg(feature`)
? Ces deux prédicats se lisent sur le diff déjà injecté, sans appel d'outil. Ce qui
n'est **pas** couvert : un test présent, non ignoré, et qui échoue — population
prise par la porte (a), en aval, et jamais mergée.

### Périmètre

**Dans le périmètre :** `crates/mika-agent/src/skills/executor.rs` (U1, U2),
`skills/bundled/qa-review/system_prompt.md` (U3).

**Hors périmètre, délibérément :**
- La lecture des logs CI par la QA (§ *Le refus mesuré*) — et donc `gh run view`,
  `QA_REVIEW_GH_ALLOWED`, `GH_API_ALLOW_MATRIX`.
- Le chemin `spawn_long_running_exec` (mika#855) : il pose déjà `process_group(0)`
  et `kill_on_drop(false)` **par contrat** — un dispatch doit survivre à son tour.
  U2 ne touche que le chemin synchrone.
- Le budget de 30 s lui-même : mika#2276 l'a posé comme garde-fou, pas comme budget,
  et le relever réintroduirait la mort par deadline qu'il a fermée (PR #2275 :
  469 s de 506 s d'enveloppe mangés par deux `cargo test`).
- L'enveloppe de mika-qa (`llm_budget_resolved`) : non mesurée ici, autre axe.

---

## Planning Contract

### Décision 1 — U1 refuse sur la **conjonction** commande × budget, jamais sur le nom du skill

Le discriminant naturel serait « le tour est un tour qa-review ». Il est écarté :
`validate_review_depth_present` le fait via `!ctx.required_tool_arg_suffixes.is_empty()`
et son propre commentaire mika#2237 nomme la fragilité — la garde « *disparaît en
silence le jour où qa-review réorganise son manifeste* ».

Le prédicat retenu est **auto-descriptif et universellement vrai** :

> une commande de la famille build, soumise à un budget d'outil inférieur au plancher
> de build, **ne peut pas aboutir** — quel que soit l'agent qui la soumet.

Conséquences voulues : mika-dev soumettant `cargo build` via `run_shell` sous 30 s est
refusé aussi, et c'est correct (elle échouait déjà, en orphelinant). Les outils
`long_running` (`build_mika`, `dev-pilot`) rendent la main **avant** l'application du
timeout (`execute_skill_tool:562`) et ne traversent donc jamais cette garde — la voie
légitime pour compiler reste ouverte, intacte.

### Décision 2 — Le refus dit qu'il est un refus, et c'est ce qui casse la ligne 40

C'est le point central du travail. Un **timeout** se lit comme un échec d'outil et
déclenche le plafond `hold[review]` de la ligne 40. Un **refus de politique** n'est
pas un échec : le corps de l'erreur le dit en toutes lettres, nomme la classification
`[⏭️]` à employer, et interdit la réécriture de la commande pour contourner le scan
(même geste que le refus `shell-exec` de mika#1196 : « *Report the refusal; do not
rewrite the command to evade the scan* »).

C'est la moitié structurelle qui manquait à mika#2276 : sa règle disait au modèle de
ne pas compiler, et le modèle apprenait le contraire par l'expérience (l'outil
acceptait la commande et mettait 30 s à mourir). Désormais l'outil répond
immédiatement et la réponse **est** la règle.

### Décision 3 — U2 est un filet, pas le correctif

U1 ferme la famille mesurée (build sous budget court). U2 ferme la **classe** : toute
commande exec qui expire laisse aujourd'hui ses petits-enfants. U2 seul ne suffirait
pas — il tuerait proprement 5 tentatives à 30 s, soit 150 s d'enveloppe brûlés pour
rien, et le `hold[review]` tomberait quand même. U1 seul laisserait la fuite ouverte
pour tout le reste. Les deux, dans cet ordre de poids.

**Le groupe est strictement plus étroit que l'existant, pas plus large.**
`process_group(0)` fait de l'enfant son propre chef de groupe ; le groupe ne contient
alors que lui et sa descendance. Aujourd'hui l'enfant est dans le groupe de
mika-spirit — c'est-à-dire qu'aucun kill de groupe n'est *possible* sans toucher le
démon. Le kill de groupe n'est armé **que sur la branche d'expiration** ; une commande
qui finit normalement est inchangée, bit pour bit.

### Décision 4 — U3 tranche la contradiction, il ne l'atténue pas

La ligne 40 gagne une exclusion explicite : *un outil refusé par une politique
déclarée n'est pas un échec d'outil*. Sans cette phrase, U1 rend une `ToolOutput`
`is_error` que le modèle relit comme un échec, et la ligne 40 referme la porte que U1
vient d'ouvrir. **Les trois unités sont donc conjonctives, aucune n'est autonome.**

La classification `[⏭️]` gagne symétriquement sa **condition** : elle n'est légitime
que si le test existe dans le diff et n'est pas exclu de la CI. Sinon `block[ac]` — le
trou est réel, et c'est le test négatif nº 2 du ticket.

### Décision 5 — Ce qui NE change pas

Aucune valeur de réglage ne bouge : `timeout_secs = 30` de qa-review reste 30, le
défaut de manifeste reste 30, aucune variable d'environnement n'est créée. Aucune
porte n'est élargie : `QA_REVIEW_GH_ALLOWED`, `GH_API_ALLOW_MATRIX`,
`GH_ALLOWED_SUBCOMMANDS` sont inchangés. `parse_verdict` et `verdict_handler` sont
inchangés — le vocabulaire `[⏭️]` est de prompt et ne traverse aucun format de fil.

---

## Implementation Units

### U1 — Une commande de build sous un budget qui ne la contient pas est refusée avant le spawn

**Fichier :** `crates/mika-agent/src/skills/executor.rs`.

**Site :** dans `execute_skill_tool`, **après** la validation des champs requis
(`validate_required_fields`, ligne 550) et **avant** la branche `long_running`
(ligne 555). Cet ordre est porteur : placer la garde après la branche `long_running`
serait inoffensif aujourd'hui mais mettrait un futur handler de build long-running
sous un prédicat qui n'est pas pour lui.

**Prédicat** — fonction pure, deux termes conjonctifs :

```rust
/// Refuse une commande dont le budget ne peut pas contenir l'exécution.
/// `None` = pas de refus.
fn refuse_uncontainable_build(
    tool_name: &str,
    input: &serde_json::Value,
    timeout_secs: u64,
) -> Option<ToolOutput>;
```

1. `timeout_secs < BUILD_FLOOR_SECS` (constante nommée, 120 s — au-dessus de tout
   budget de skill court, très en dessous de toute compilation réelle du workspace).
2. Le champ `command` de l'entrée appartient à la famille build, reconnue par un scan
   **borné par identifiant**, sur le modèle exact du scan L3 de `run.sh` : la
   frontière est « tout caractère qui ne peut pas faire partie d'un identifiant de
   commande », `.` et `-` exclus de la frontière. Famille v1, close et énumérée :
   `cargo build|test|clippy|check|bench`, `npm run build|test`, `npx tsc`,
   `make build|test`, `go build|test`.

**Ce que le prédicat ne prétend pas être.** Le scan est lexical, donc contournable
(découpage de token, assemblage par variable, `sh -c`) — exactement la posture que
`run.sh` écrit déjà pour ses deux scans : *defense-in-depth, NOT a sole gate*. Le
dernier recours reste U2 : une commande qui passe sous le scan et expire est tuée en
groupe. C'est pourquoi les deux unités coexistent.

**Corps du refus** — `ToolOutput::error` avec un JSON structuré, même forme que les
refus mika#1646 / mika#2237 :

```json
{
  "error": "build_command_exceeds_tool_budget",
  "policy": "refusal",
  "tool_budget_secs": 30,
  "detail": "This is a POLICY REFUSAL, not a tool failure. The command was never
             spawned. Do NOT retry, do NOT rewrite the command to evade the scan,
             and do NOT treat this as a failed verification step.",
  "remedy": "Mark the acceptance criterion `[⏭️] not verifiable within the review
             budget — requires a build` and state so in the verdict. CI runs this
             build without a time limit and the merge gate reads its result."
}
```

Les trois négations de `detail` sont chacune un comportement mesuré ou prédit : la
réécriture (mika#1196), le retry (les 5+ tentatives du ticket), et la relecture en
échec d'étape (la ligne 40).

**Journal :** `build_command_refused_over_budget` (WARN — champs `tool`, `agent_id`
si disponible, `timeout_secs`, `matched_family`, **jamais la commande complète**).
**Régime attendu : proche de zéro après stabilisation.** Un flot soutenu signifie que
U3 n'a pas pris et que le modèle relance à chaque tour — et c'est **là** qu'il faut
chercher, pas dans le seuil.

### U2 — Le handler exec devient chef de groupe, et le groupe meurt avec le timeout

**Fichier :** `crates/mika-agent/src/skills/executor.rs`, `execute_exec` (ligne 828).

1. Ajouter `.process_group(0)` à la construction de la `Command` (ligne 846-854),
   à côté de `.kill_on_drop(true)`. Citer mika#855 / `spawn_long_running_exec:3497`
   comme le motif préexistant.
2. Introduire une garde RAII locale au module :

```rust
/// Tue le groupe de processus de l'enfant quand le futur est abandonné.
/// `kill_on_drop` ne signale que le pid direct ; `run.sh` finissant sur
/// `eval "$COMMAND"`, ses petits-enfants survivent et sont reparentés à init.
struct ProcessGroupKillGuard { pgid: i32, disarmed: bool }
```

   Armée juste après le `spawn` réussi, **désarmée** après `wait_with_output`. À
   `drop` en état armé : `libc::killpg(pgid, SIGKILL)` (`libc` est déjà une
   dépendance — `task_engine/process_liveness.rs:57`). Le désarmement sur le chemin
   nominal est ce qui garantit que le kill ne concerne que l'expiration.

**Frontière `unsafe` :** un seul appel `libc`, encapsulé dans la garde, avec le
contrat écrit sur le bloc — `pgid` vient de `child.id()` d'un spawn réussi avec
`process_group(0)`, donc strictement positif et distinct du groupe de mika-spirit.

**Risque nommé et vérifié :** un handler qui laisse volontairement un démon derrière
lui (`tmux new-session -d`) pourrait être atteint. En pratique le serveur tmux se
détache lui-même (double fork + `setsid`) et quitte le groupe ; et le kill ne fire
que sur expiration, où le handler a déjà échoué. À vérifier par un test dédié
(`V5`), pas par raisonnement.

### U3 — Le prompt cesse de se contredire, et la classification gagne sa condition

**Fichier :** `skills/bundled/qa-review/system_prompt.md`.

**U3a — ligne 40 (Data Integrity), exclusion explicite.** Après « If any step was
skipped due to a tool failure, the maximum verdict is `hold[review]` », ajouter :

> A tool refused by a **declared policy** — a structured refusal returned before the
> subprocess spawns, carrying `"policy": "refusal"` — is **not** a tool failure and
> does **not** cap the verdict. It is a designed guardrail (mika#2276, mika#2423).
> Classify the affected AC per 2.5.3 and continue.

**U3b — ligne 293, la condition de `[⏭️]`.** Le marqueur n'est légitime que si le
diff **contient** le test qui vérifierait l'AC **et** que ce test n'est pas exclu de
la CI. Deux prédicats lus sur le diff déjà injecté (Step 3), sans appel d'outil :

| état du diff | classification | verdict |
|---|---|---|
| test présent, non exclu | `[⏭️] not verifiable within the review budget — requires a build` | ne plafonne rien (2.5.7) |
| test présent mais `#[ignore]` ou derrière un `#[cfg(feature = …)]` non couvert | `[❌]` | `block[ac]` |
| aucun test pour cet AC dans le diff | `[❌]` | `block[ac]` |

Avec, écrit sur place, la raison qui rend la première ligne sûre : *CI runs this build
with no time limit, and `verdict_handler` refuses to merge a `pass` whose checks are
failing (`server/verdict_handler.rs`, `CheckClassification::HasFailures`). Deferring
execution here does not defer the gate.*

**U3c — ligne 45, la frontière CI est maintenue et motivée.** Ajouter une phrase :
ne pas lire la CI n'est pas une limitation d'outillage mais le maintien du seul
relecteur du diff ; la question « ce test passe-t-il » appartient à la porte de merge.
Sans cette phrase, un futur éditeur lisant U3b conclut naturellement qu'il manque une
lecture de CI et rouvre la porte.

**U3d — budget de prompt.** `max_prompt_size = 73728` ; taille actuelle ~68 871 o ;
la barrière à 95 % **panique** (`tests/bundled_skills_load.rs:141,166`) à 70 041 o.
U3 doit tenir **sous ~1 170 octets nets**. C'est une contrainte dure, pas une
préférence : les ajouts sont rédigés serré et U3d est une AC à part entière. Si le
budget est dépassé, relever `max_prompt_size` (plafond dur 81 920) **dans le même
commit**, avec le commentaire daté qu'exige la convention du fichier.

---

## Verification Contract

Chaque unité a son contrôle positif **et** son contrôle négatif. Un contrôle positif
seul ne distingue pas « la garde décide » de « la garde bloque tout ».

| # | Test | Unité | Ce qu'il établit |
|---|---|---|---|
| V1 | `mika2423_a_build_command_under_a_short_budget_is_refused_before_spawn` | U1 | `cargo test --release` à `timeout_secs = 30` rend le refus structuré ; l'appel revient en **moins d'une seconde** (donc rien n'a été lancé) |
| V2 | `mika2423_the_same_command_under_a_build_budget_is_not_refused` | U1 | **Contrôle négatif** : même commande à `timeout_secs = 600` → pas de refus. Sans lui, « refuse les builds sous budget court » est indistinguable de « refuse les builds » |
| V3 | `mika2423_an_ordinary_command_is_never_refused` | U1 | **Contrôle négatif** : `git status`, `grep -r foo`, `cat Cargo.toml` à 30 s → pas de refus. Plus `cargo.log` / `make-believe` / `libcargo-dev`, qui prouvent la borne par identifiant |
| V4 | `mika2423_a_timed_out_handler_leaves_no_orphan_grandchild` | U2 | **Le test central du corollaire.** Handler `sh -c 'sleep 300 & echo $!; wait'`, budget 2 s. Après expiration, le pid du petit-enfant est mort. **Vérifié rouge avant U2** : sans `process_group(0)` + killpg il survit — c'est exactement le pid 145584 du ticket |
| V5 | `mika2423_a_normally_completing_handler_is_not_group_killed` | U2 | **Contrôle négatif** : un handler qui détache un démon (`setsid`) et rend la main normalement laisse ce démon vivant. Établit que le kill est sur la branche d'expiration seule |
| V6 | `mika2423_the_refusal_names_the_skip_classification_and_denies_being_a_failure` | U1+U3 | Le corps du refus porte `"policy": "refusal"` et la chaîne `[⏭️] not verifiable within the review budget` — le **joint** entre la moitié moteur et la moitié prompt. Sans lui, U1 et U3 peuvent diverger en silence |
| V7 | `mika2423_the_qa_prompt_resolves_the_integrity_contradiction` | U3 | Scan du prompt : la ligne 40 porte l'exclusion `policy refusal`, la 293 porte la condition sur le diff, la 45 porte sa motivation. Un test de forme, parce que la régression ici est une **suppression** que rien d'autre ne rend rouge |
| V8 | `bundled_skills_approaching_max_prompt_size_warns` (existant) | U3d | Le prompt reste sous la barrière à 95 %. Déjà en place et **paniquant** ; il suffit qu'il reste vert |
| V9 | `mika2423_the_gh_scope_for_qa_review_is_unchanged` | périmètre | `QA_REVIEW_GH_ALLOWED` porte exactement ses quatre entrées. **Pin d'une décision** : ce ticket a examiné et refusé la lecture CI ; un futur éditeur qui l'ajoute doit le faire sciemment |

**Vérification manuelle exigée avant livraison :** V4 doit être **observé rouge**
sans U2 puis vert avec. Un test de fuite de processus qui n'a jamais été vu rouge ne
prouve pas qu'il détecte la fuite — il peut ne mesurer que le pid direct, qui mourait
déjà.

**Ce que la vérification n'achète pas.** Aucun test déterministe n'établit qu'un LLM
suivra U3 — c'est la limite que M1 vient précisément de mesurer sur mika#2276. C'est
pourquoi la charge porte sur U1 (le refus est un fait, pas une consigne) et pourquoi
la sonde post-déploiement ci-dessous est la seule mesure comportementale disponible.

**Sonde post-déploiement, et ses trois haltes.** Sur les 7 PR suivantes portant un AC
comportemental à test lourd :

- `grep build_command_refused_over_budget "$MIKA_SPIRIT_LOG_FILE"` — **une occurrence
  par tour QA concerné, au plus**. Plusieurs sur le même tour = U3a n'a pas pris, le
  modèle relit le refus comme un échec et relance.
- `ps -eo pid,ppid,etimes,comm | awk '$2==1 && $4 ~ /cargo|rustc/'` — **doit rester
  vide** après un tour QA.
- **Halte 1** — un `hold[review]` « non exécuté » réapparaît alors que le refus est
  journalisé : U3a n'est pas dans le prompt **servi**. Vérifier le déploiement avant
  de toucher au texte — `cat ~/.mika/skills/.manifest-writer` (mika#2340 ; **absent
  sur cette machine au 2026-09-20**, ce qui signale déjà une chaîne de seed
  antérieure au correctif et doit être établi en premier).
- **Halte 2** — un `hold[review]` « non exécuté » réapparaît **sans** refus
  journalisé : la commande a échappé au scan lexical. Élargir la famille v1, ne pas
  toucher au plancher.
- **Halte 3** — une PR est mergée alors que son test ajouté est rouge : **arrêt
  immédiat**, le raisonnement (a) du refus mesuré est faux et tout le plan en dépend.
  Établir pourquoi `verdict_handler` n'a pas classé `HasFailures` avant quoi que ce
  soit d'autre.

---

## Definition of Done

- U1, U2, U3 livrés ensemble. Aucune n'est autonome : U1 sans U3a rend une erreur que
  la ligne 40 relit en échec ; U3 sans U1 est une règle de prompt de plus sur une
  classe que M1 mesure comme résistante au prompt ; U2 sans U1 tue proprement cinq
  tentatives inutiles.
- V1–V9 verts ; V4 **observé rouge** sans U2, consigné dans le corps de PR.
- `cargo clippy --all-targets -- -D warnings` et `cargo fmt --check` verts.
- `make verify-bundled-skills` vert (U3 touche un bundle).
- `QA_REVIEW_GH_ALLOWED`, `GH_API_ALLOW_MATRIX`, `GH_ALLOWED_SUBCOMMANDS`,
  `timeout_secs` de qa-review : **inchangés**, et V9 le pin.
- Le corps de PR nomme les trois faits refusés par le plan (la lecture CI, le
  relèvement du budget, le discriminant par nom de skill) avec leur raison.

---

## Acceptance criteria

Dérivés des trois tests négatifs du ticket. Les deux premiers sont **reformulés** :
le ticket les énonce en termes de lecture du log CI, que ce plan refuse au § *Le refus
mesuré* ; la reformulation préserve l'issue observable — le verdict rendu — et change
le moyen.

**AC1 — Un AC comportemental à test lourd ne produit plus `hold[review]-pour-non-exécuté`.**
Sur une PR dont le diff porte le test de l'AC, non ignoré et non feature-gated, le
verdict QA n'est pas `hold[review]` motivé par la non-exécution : l'AC est marqué
`[⏭️] not verifiable within the review budget — requires a build` et 2.5.7 laisse
passer. Le verdict peut rester `block`/`hold` pour **tout autre motif réel**.
*(Ticket, test négatif nº 1 — « test vert dans le log CI `Check` » devient « test
présent dans le diff », la verdeur étant garantie en aval par `verdict_handler`.)*
Vérifié par V6, V7 ; confirmé par la sonde à 7 PR.

**AC2 — Un test absent ou exclu de la CI reste un vrai trou.**
Si aucun test de l'AC n'est présent dans le diff, ou si le test ajouté porte
`#[ignore]` ou un `#[cfg(feature = …)]` non couvert par la CI, la classification
`[⏭️]` est **refusée** et l'AC est `[❌]` → `VERDICT: block[ac]`.
*(Ticket, test négatif nº 2, littéral.)* Vérifié par V7.

**AC3 — Aucune tentative `cargo` de la QA ne subsiste en `ppid=1` après la fin du tour.**
Deux mécanismes conjoints : la commande de build n'est plus lancée du tout (U1), et
toute autre commande exec qui expire voit son groupe de processus tué (U2).
*(Ticket, test négatif nº 3, littéral.)* Vérifié par V4 (contrôle positif observé
rouge d'abord) et V5 (contrôle négatif) ; confirmé par la sonde `ps`.

**AC4 — Le refus est un refus, pas un échec.**
Le refus de U1 revient en moins d'une seconde, porte `"policy": "refusal"`, nomme la
classification `[⏭️]`, et la ligne 40 du prompt l'exclut explicitement du plafond
`hold[review]`. Vérifié par V1, V6, V7.

**AC5 — La frontière CI de mika-qa est inchangée et sa raison est écrite.**
`QA_REVIEW_GH_ALLOWED` porte exactement ses quatre entrées ; la ligne 45 du prompt
porte désormais la raison de l'interdiction (la porte de merge possède déjà la
question). Vérifié par V9, V7.

**AC6 — Aucune régression sur les commandes ordinaires ni sur la voie de build légitime.**
Une commande non-build sous budget court n'est jamais refusée ; la même commande de
build sous un budget de build ne l'est pas non plus ; les outils `long_running`
(`build_mika`) ne traversent pas la garde. Vérifié par V2, V3.

**AC7 — Le prompt reste sous sa barrière de taille.**
`qa-review/system_prompt.md` ne franchit pas le seuil à 95 % de `max_prompt_size`,
ou `max_prompt_size` est relevé dans le même commit avec son commentaire daté.
Vérifié par V8.

---

## Sources

Tout ce qui est affirmé ici a été lu dans l'arbre à `HEAD = ba08a9bc`.

- `skills/bundled/qa-review/system_prompt.md:40` — plafond `hold[review]` sur échec
  d'outil, déclaré prioritaire sur le reste du document.
- `skills/bundled/qa-review/system_prompt.md:45` — interdiction absolue de lire la CI.
- `skills/bundled/qa-review/system_prompt.md:158` — précédent d'exception explicite
  (GitHub Advisory Database, « here and only here »).
- `skills/bundled/qa-review/system_prompt.md:293` — mika#2276, interdiction de
  compiler + échappatoire `[⏭️]`.
- `skills/bundled/qa-review/system_prompt.md:428` — 2.5.7, « all ACs ✅ or ⏭️ → passes ».
- `skills/bundled/qa-review/skill.toml:14,30` — `timeout_secs = 30`,
  `max_prompt_size = 73728` ; taille mesurée du prompt : 68 871 o.
- `crates/mika-agent/src/skills/executor.rs:645` — `tokio::time::timeout` qui
  abandonne le futur.
- `crates/mika-agent/src/skills/executor.rs:828-871` — `execute_exec`,
  `.kill_on_drop(true)` sans `.process_group(0)`.
- `crates/mika-agent/src/skills/executor.rs:3496-3497` — le motif correct préexistant
  (mika#855).
- `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh` — `eval "$COMMAND"`
  en dernière ligne ; les deux scans lexicaux bornés par identifiant servant de
  modèle au prédicat de U1.
- `crates/mika-agent/src/skills/builtin_handlers.rs:1945` — `QA_REVIEW_GH_ALLOWED`,
  quatre entrées, `gh run` absent.
- `crates/mika-agent/src/server/verdict_handler.rs:541,566-590` — **le fait porteur
  du refus** : la branche `pass` classe les checks et refuse de merger une PR rouge.
- `crates/mika-agent/src/task_engine/process_liveness.rs:57` — `libc` déjà dépendance.
- `git log e17ba3a7` — mika#2276, 2026-09-10, dix jours avant l'incident.
- Ticket mika#2423, corps et commentaire du 2026-09-20T19:30:46Z.
