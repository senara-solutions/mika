# mika#2508 — `PILOT_MAX_TURNS` n'atteint jamais le pilote : le canal, pas l'allowlist

> **Parent umbrella :** mika#2491 (Défaut 4-bis, suivi de mika#2496 / PR #2501).
> Enfant du cadre substrat — 1 PR atomique.

---

## 0. La rectification centrale — le remède proposé contredit une décision écrite du dépôt

> **⚠️ DIVERGENCE PLAN↔TICKET — RATIFIÉE.** Opérateur via orchestrateur,
> **2026-09-24 08:10Z**, portée au corps du ticket (Item 0) et à son commentaire
> du 08:04:48Z. Le remède littéral du § *Fix proposé* point 1 — « ajouter
> `PILOT_MAX_TURNS` à l'allowlist `is_sandbox_env_allowed` » — est **remplacé**
> par un **relais explicite** post-`sandboxed_pilot_env` (motif mika#2354 AC9(b),
> siblings `inject_rescue_verify_env` / `inject_arch_ask_retry_env`), avec
> `relayed_env_pairs_preserving_empty` préservant le palier de rollback
> `${VAR+set}`. **Raison de la ratification :** surface de sûreté plus petite —
> le relais achemine la variable précise sans agrandir la surface générale du
> bac à sable — et c'est le motif établi. **Les critères d'acceptation sont
> inchangés** (`pilot_budget_armed max_turns=150 source=env` + `--max-turns 150`
> sur la cmdline au 1er dispatch post-déploiement) : seul le moyen change.
>
> C'était le **F1 bloquant** de l'ESCALATE de première passe (groom `a200a497`,
> 07:54:50Z). F2 (absence de #2508 dans la carte-enfants de mika#2491) est
> corrigé indépendamment. Le § ci-dessous est l'argument qui a fondé la
> divergence ; il est conservé parce que la ratification porte sur sa conclusion,
> pas à sa place.

Le ticket a **raison sur le diagnostic** et **se trompe sur le remède**, et il faut
le dire avant tout le reste parce que le remède proposé casserait une garde
existante.

Le § *Fix proposé* point 1 demande d'ajouter `PILOT_MAX_TURNS` à
`SANDBOX_ENV_CORE_ALLOWLIST`. Or `crates/mika-agent/src/skills/executor.rs` porte
déjà, depuis mika#2354 et mika#2278, **la réponse à cette exacte question**, écrite
et tenue par un test :

```rust
/// mika#2354 AC9(b): the positive allowlist stays the guard and the explicit
/// injection stays the named exception. Adding either name to
/// [`SANDBOX_ENV_CORE_ALLOWLIST`] — or covering it with a new entry in
/// [`SANDBOX_ENV_ALLOWED_PREFIXES`] — would make the settings arrive by
/// inheritance […]
#[test]
fn mika2354_rescue_verify_env_never_joins_the_sandbox_allowlist() { … }
```

Deux familles de réglages opérateur lus par `dispatch-lib.sh`
(`MIKA_RESCUE_VERIFY_*`, `MIKA_ARCH_ASK_RETRY*`) traversent déjà `env_clear()`, et
**aucune** ne le fait par l'allowlist : chacune est **relayée explicitement** après
`sandboxed_pilot_env`, par `inject_rescue_verify_env` / `inject_arch_ask_retry_env`.
L'allowlist est la garde de confinement du pilote — la surface qui empêche
`AWS_*`, `NODE_AUTH_TOKEN` et tout jeton opérateur non listé d'entrer par
héritage. Y déposer un réglage de configuration mélange deux populations qui n'ont
ni la même durée de vie ni le même critère d'admission, et fait du confinement la
poubelle des réglages.

**Le plan livre donc l'esprit du DoD par le mécanisme établi : relais explicite.**
La divergence ne réduit aucun critère d'acceptation, elle en change le moyen — et
elle est **ratifiée** (encadré ci-dessus), donc le corps de PR la *rappelle* au
lieu de la soumettre.

Et le doc-comment de `RESCUE_VERIFY_ENV` avait déjà écrit, mot pour mot, la phrase
qui condamne le nommage nu de `PILOT_MAX_TURNS` :

> `sandboxed_pilot_env` rebuilds the child env from a **positive** allowlist, so no
> name crosses by inheritance — prefixed or not. **A same-named-but-unprefixed
> variable would propagate exactly as little and cost a vocabulary divergence for
> nothing.**

`PILOT_MAX_TURNS` a été nommé nu sur un diagnostic que le fichier voisin réfutait
déjà. C'est la mesure de ce que coûte un commentaire faux : il a produit un nom.

---

## 1. Ce que la mesure établit, et ce qu'elle ne dit pas

**Mesure du 2026-09-24** (1er dispatch après restart 07:13:10Z, `PILOT_MAX_TURNS=150`
dans `~/.mika/.env`) :

| maillon | état |
|---|---|
| `~/.mika/.env` → mika-spirit (PID 2274572) | **porte** `PILOT_MAX_TURNS=150` |
| mika-spirit → child de dispatch `run.sh` (PID 2277335) | **ABSENT** |
| `_pilot_max_turns` | branche « non défini » → `_PILOT_MAX_TURNS=""`, `source=default` |
| argv du pilote (PID 2278055) | **aucun** `--max-turns` → défaut amont `maxTurns=200` |

La cause est unique et localisée : `spawn_long_running_exec` appelle
`sandboxed_pilot_env`, qui fait `cmd.env_clear()` puis recopie **seulement** les
clés admises par `is_sandbox_env_allowed` (baseline exact-match + cinq préfixes).
`PILOT_MAX_TURNS` n'y est ni admise ni relayée : elle est effacée, quel que soit
son nom.

**Ce que la mesure ne met PAS en cause.** La moitié shell est correcte et
intégralement testée : `_pilot_max_turns` distingue ses trois paliers, les trois
sites de lancement portent le drapeau, la co-location est tenue par un scan, et
`${_PILOT_MAX_TURNS:+--max-turns …}` s'étend juste. `skills/bundled/_shared/test-dispatch-lib.sh`
en atteste sur ~10 assertions (V4 de mika#2496). **Aucune ligne de shell n'est à
corriger** — et c'est précisément ce qui rend la panne durable : tout ce qui était
testé était vert.

**Pourquoi personne ne l'a vue.** mika#2496 a énoncé sa propre vérification :

> **V1a is statically green** — the scrub is a `MIKA_*` denylist plus `GH_TOKEN`, so
> an unprefixed name traverses […] **V1b is the operator gesture** […] **Do not claim
> AC4 […] until `source=env` has been observed.**

V1a a regardé `scrub_mika_env_vars` — qui n'est **pas** sur ce chemin — et a conclu
« vert » sur le mauvais mécanisme. V1b, la seule qui pouvait trancher, n'avait jamais
été exécutée. La mesure du 2026-09-24 **est** l'exécution de V1b, et elle est rouge.
La consigne « ne pas revendiquer AC4 » a donc tenu exactement ce qu'elle promettait :
elle a empêché une fausse garantie d'être écrite, pas la panne d'exister.

---

## 2. L'angle mort de classe (point 3 du ticket) — relevé

Toute variable posée sur l'environnement du service, lue par `dispatch-lib.sh`, et
ni allowlistée ni relayée, est **inerte**. Relevé manuel sur `dispatch-lib.sh`
(le scan de R4 produira le relevé faisant foi) :

| variable | lecture | traverse ? | conséquence de l'inertie |
|---|---|---|---|
| `PILOT_MAX_TURNS` | `_pilot_max_turns` | **non** | le défaut mesuré — aucun plafond tours |
| `PILOT_LOG_DIR` | `${PILOT_LOG_DIR:-/var/log/claude-pilot}` | **non** | divergence **garantie** avec `MIKA_PILOT_LOG_DIR` |
| `MIKA_PILOT_SANDBOX` | `${MIKA_PILOT_SANDBOX:-1}` | non | kill-switch du confinement décoratif (inertie **fail-safe**) |
| `MIKA_PILOT_EGRESS_LOG_DIR` | `${…:-/var/log/mika}` | non | journal du relais d'egress non déplaçable |
| `MIKA_PLATFORM_DIR` | `${…:-$HOME/workspace/mika-platform}` | non | racine plateforme non déplaçable |
| `MIKA_HOME` | `${MIKA_HOME:-$HOME/.mika}/state/pr-origin-epoch` | non | l'epoch mika#2026 s'écrit sous le mauvais home |
| `MIKA_PR_ORIGIN_LABEL_COLOR` | `--color "$…"` | non | couleur de label non réglable |
| `CLAUDE_PILOT_MIN_TOOL_CALLS` | `export …:-3` | non | seuil figé à 3 |

`MIKA_SPIRIT_LOG_FILE` apparaît **seulement échappé** (`\$MIKA_SPIRIT_LOG_FILE`)
dans une chaîne de prose destinée à un corps de PR : ce n'est pas une lecture et
elle est **hors population**. C'est le faux positif que mika#2050 a mesuré sur le
Signal S et que mika#2201 a dû nommer — le prédicat de R4 doit l'exclure, et le
contrôle négatif N4 l'atteste.

### 2.1 — Le commentaire faux n'est pas à trois sites, il est à sept

Le ticket en nomme deux. La première passe de ce plan en a trouvé un troisième et
a écrit « il y en a **trois** ». Le relevé exhaustif (`grep -rn scrub_mika_env_vars`
sur `*.rs` / `*.md` / `*.sh`, hors `target/` et hors `docs/plans/`) en rend
**sept sur le chemin pilote**, et ils se scindent en deux classes qui n'appellent
pas le même geste — distinction portante, parce que confondre les deux ferait
réécrire des phrases justes et manquer des phrases fausses.

**Classe A — mécanisme faux ET conclusion fausse.** Ces sites affirment ou
impliquent qu'un nom **nu** traverse le child de dispatch. C'est la classe qui
**produit la panne** : c'est elle qui a fait nommer `PILOT_MAX_TURNS` nu, et elle
ferait nommer nu le prochain réglage.

| # | site | ce qu'il affirme de faux |
|---|---|---|
| A1 | `crates/mika-common/src/config.rs:1065` | « the unprefixed form exists because `scrub_mika_env_vars` strips every `MIKA_*` » |
| A2 | `crates/mika-common/src/config.rs:1374` (`DEFAULT_PILOT_LOG_DIR`) | les deux noms divergent « because `scrub_mika_env_vars` strips every `MIKA_*` » |
| A3 | `crates/mika-agent/CLAUDE.md:2506` | « Prefixed, unlike its `PILOT_MAX_TURNS` sibling, because … `scrub_mika_env_vars` does not run » |
| A4 | `crates/mika-agent/CLAUDE.md:2527` | « different environment variables … because `scrub_mika_env_vars` strips every `MIKA_*` » |
| A5 | `CLAUDE.md:277` (Signal S, *Halte 1*) | « deliberately distinct (`scrub_mika_env_vars` strips every `MIKA_*`) » |
| A6 | `CLAUDE.md:502` (`MIKA_PILOT_LOG_DIR`) | « deliberately *different variable names* because `scrub_mika_env_vars` strips » |
| A7 | `CLAUDE.md:517` (`PILOT_MAX_TURNS`) | « **Unprefixed on purpose** … so a prefixed name would be removed on the way » |

A7 est **le site le plus lu et le plus faux** : il ne se trompe pas seulement de
mécanisme, il invoque A6/A2 comme **précédent** — « `PILOT_LOG_DIR` is too, and
mika#2249 wrote why ». Une erreur qui se cite elle-même à travers deux fichiers.

**Classe B — mécanisme mal nommé, conclusion juste.** Ces sites disent qu'un
`MIKA_*` n'atteint pas le child. C'est **vrai sous les deux mécanismes** (le scrub
le retirerait, l'`env_clear` le retire), donc aucune décision qui en dépend n'est
fausse et aucune panne n'en découle.

| # | site | statut |
|---|---|---|
| B1 | `skills/bundled/_shared/dispatch-lib.sh:460` (`$HOME/.mika` écrit en toutes lettres) | conclusion **juste** — `MIKA_HOME` n'atteint pas le child |
| B2 | `crates/mika-agent/src/pilot_egress_stamp.rs:56` | conclusion **juste** — idem, et sa table reste exacte |
| B3 | `docs/operator/pilot-egress-relay.md:272` | conclusion **juste** — `MIKA_INTERNAL_TOKEN` est insensible au chemin env |

**Périmètre retenu : la classe A est réparée, la classe B reçoit une phrase.**
Réécrire les trois sites B serait toucher trois raisonnements corrects pour un mot,
sur des fichiers (dont un marqueur de sûreté et un runbook opérateur) dont la
relecture coûte plus que l'imprécision. Ils reçoivent, en une ligne chacun, la
précision « `env_clear()` + allowlist positive, et non un scrub sélectif » **sans
que leur conclusion ne bouge** — ce qui est le strict nécessaire pour qu'un
lecteur qui suit la piste depuis A7 n'y retrouve pas le mécanisme réfuté.

### 2.2 — `PILOT_LOG_DIR` est de la même classe exacte, et il est le précédent invoqué à tort

Le root `CLAUDE.md` justifie le nommage nu de `PILOT_MAX_TURNS` par
« `PILOT_LOG_DIR` is too, and mika#2249 wrote why ». Le précédent est lui-même
cassé : mika#2249 a écrit que la divergence entre `MIKA_PILOT_LOG_DIR` (moteur) et
`PILOT_LOG_DIR` (shell) « ne peut acheter que de l'inertie, jamais un faux
positif » — vrai, mais ce qu'il n'a pas vu est que la divergence est **garantie**,
pas accidentelle : le shell est **toujours** sur `/var/log/claude-pilot`, et un
opérateur qui déplace les deux obtient un moteur qui lit où le shell n'écrit pas,
donc un reaper mika#2249/#2277 aveugle sur `pilot_log`. Le ticket l'a prédit au
point 3 ; la mesure le confirme.

---

## 3. Périmètre — ce qui est réparé, ce qui est mesuré, ce qui est suivi

**Réparé** (une cause, un mécanisme, deux variables de la même famille pilote) :
`PILOT_MAX_TURNS` et `PILOT_LOG_DIR`. La seconde entre parce que la décision est
**déjà prise dans le dépôt** — le root `CLAUDE.md` prescrit noir sur blanc que les
deux noms doivent correspondre et nomme la divergence comme une **halte** (Signal S,
*Halte 1*) — seule son exécution manquait. Corriger le commentaire de
`PILOT_MAX_TURNS` en laissant son précédent cassé pointerait le lecteur vers une
seconde panne.

**Mesuré et non réparé** : les six `MIKA_*` du tableau. Les rendre traversantes est
**six décisions distinctes**, pas une correction de bug — et l'une d'elles,
`MIKA_PILOT_SANDBOX`, donnerait à l'environnement du service un levier pour
**désarmer le confinement bwrap**, ce qui est un arbitrage de sûreté qui appartient
à un ticket qui le pèse, jamais à un effet de bord. Elles reçoivent une exception
nommée (§ Fire-Disposition) et un suivi.

**Hors périmètre** : voir § 10.

---

## 4. Requirements

- **R1 — le canal.** `PILOT_MAX_TURNS` et `PILOT_LOG_DIR` atteignent le child de
  dispatch par **relais explicite après `sandboxed_pilot_env`**, jamais par
  l'allowlist. Les deux listes d'allowlist sont **inchangées**.
- **R2 — les trois paliers du shell survivent au relais.** `_pilot_max_turns`
  distingue *non défini* / *défini-vide-ou-zéro* (le **rollback**) / *entier*. Un
  relais qui aplatit « défini mais vide » en « absent » détruit un palier que le
  shell a délibérément construit avec `${VAR+set}`. La valeur vide doit donc être
  **relayée comme telle**, et non omise.
- **R3 — les sept sites de classe A sont corrigés.** Le ticket en nomme deux, la
  première passe en a trouvé un troisième ; le relevé exhaustif en rend **sept**
  (§ 2.1), dont le plus lu — A7 — invoque deux des six autres comme précédent.
  Les trois sites de classe B reçoivent la précision de mécanisme sans que leur
  conclusion, qui est juste, ne bouge.
- **R4 — la classe est rendue détectable.** Un scan structurel refuse qu'une
  variable opérateur lue par `dispatch-lib.sh` soit ni allowlistée, ni relayée, ni
  exceptée nommément.
- **R5 — le scan ne peut pas être inerte.** Anti-vacuité : fichier introuvable,
  vide, ou extraction rendant une population sous un plancher ⇒ **rouge**. Un scan
  visant un fichier mort se lit exactement comme un scan propre (mika#2103, #2205).
- **R6 — rien de ce qui traverse aujourd'hui ne cesse de traverser.** Le confinement
  n'est ni élargi ni rétréci : `is_sandbox_env_allowed` est intouchée, et
  `mika2354_rescue_verify_env_never_joins_the_sandbox_allowlist` reste vert sans
  modification.

---

## 5. Implémentation

### 5.1 — `crates/mika-agent/src/skills/executor.rs` : le canal (R1, R2)

Une constante et un injecteur, calqués sur `RESCUE_VERIFY_ENV` /
`inject_rescue_verify_env`, dont le doc-comment porte la correction du diagnostic :

```rust
/// Les deux réglages opérateur du canal pilote que `dispatch-lib.sh` lit
/// (mika#2508) : le plafond de tours (mika#2496) et le puits de journal
/// (mika#2249).
///
/// **Non préfixés, et ce n'est PAS ce qui les fait traverser.** Les deux ont été
/// nommés nus sur un diagnostic faux — « `scrub_mika_env_vars` retire tout
/// `MIKA_*` du child, donc un nom nu survit ». Le child de dispatch n'est pas
/// scrubbé : `sandboxed_pilot_env` fait `env_clear()` puis recopie une allowlist
/// **positive**, donc **aucun** nom ne traverse par héritage, préfixé ou non.
/// Mesuré le 2026-09-24 : `PILOT_MAX_TURNS=150` posé sur le service, absent du
/// child, pilote lancé sans `--max-turns`.
///
/// Les noms restent nus parce qu'ils sont un **format de fil** pour l'opérateur
/// (`PILOT_MAX_TURNS=150` est déjà posé dans `~/.mika/.env`), jamais parce que la
/// forme nue achèterait quoi que ce soit. Les renommer est une dette de
/// vocabulaire, pas un correctif — voir § *Hors périmètre* de mika#2508.
///
/// Même contrat de placement que [`RESCUE_VERIFY_ENV`] : relayées APRÈS
/// [`sandboxed_pilot_env`], et **jamais** ajoutées à l'allowlist — mika#2354
/// AC9(b), tenu par `mika2354_rescue_verify_env_never_joins_the_sandbox_allowlist`
/// et étendu à ces deux noms par `mika2508_…`.
const PILOT_DISPATCH_ENV: &[&str] = &["PILOT_MAX_TURNS", "PILOT_LOG_DIR"];
```

Le relais ne peut **pas** réutiliser `relayed_env_pairs` : celui-ci omet la valeur
vide, et son doc-comment dit pourquoi (« An absence must therefore stay an
absence »). Cette règle est juste pour ses deux familles — `MIKA_ARCH_ASK_RETRY_DELAY_SECS=""`
y est la forme d'une demi-ligne `.env` mal écrite. Elle est **fausse ici** :
`PILOT_MAX_TURNS=""` est documenté comme **le rollback**, à égalité avec `"0"`.
D'où un second helper pur, testable de la même façon :

```rust
/// Comme [`relayed_env_pairs`], mais **préserve la valeur vide**.
///
/// La différence est portante et elle est du côté du lecteur, pas de l'écrivain :
/// `_pilot_max_turns` distingue trois paliers avec `${PILOT_MAX_TURNS+set}`, et
/// son palier « défini mais vide » est le ROLLBACK explicite (le drapeau n'est pas
/// passé, claude-pilot retombe sur son `maxTurns=200`). Omettre le vide le
/// replierait sur le palier « non défini », c'est-à-dire sur le défaut de flotte.
///
/// Aujourd'hui les deux coïncident (le défaut de flotte est vide), donc le piège
/// est **programmé et non hypothétique** : le résolveur prescrit lui-même
/// `local _default=120` une fois la V2 de mika#2496 rapportée, et ce jour-là un
/// rollback par `""` deviendrait silencieusement un plafond à 120.
fn relayed_env_pairs_preserving_empty<F>(keys: &[&'static str], read: F)
    -> Vec<(&'static str, String)>
where F: Fn(&str) -> Option<String>
{
    keys.iter().filter_map(|k| read(k).map(|v| (*k, v))).collect()
}

fn inject_pilot_dispatch_env(cmd: &mut tokio::process::Command) {
    for (key, value) in
        relayed_env_pairs_preserving_empty(PILOT_DISPATCH_ENV, |k| std::env::var(k).ok())
    {
        cmd.env(key, value);
    }
}
```

Appel dans `spawn_long_running_exec`, **après** `sandboxed_pilot_env` et à la suite
de ses trois siblings, avec le commentaire de placement de la maison.

**Best-effort et silencieux**, comme ses siblings : un dispatch qui ne porte pas le
réglage retombe sur les défauts du shell (désarmé, `/var/log/claude-pilot`), jamais
un dispatch bloqué.

### 5.2 — Les sept sites de classe A, plus une phrase sur les trois de classe B (R3)

**Une seule formulation, reprise aux sept sites**, pour qu'un futur `grep` sur
l'une d'elles les rende tous : *le child de dispatch n'est pas scrubbé — il est
`env_clear()` + allowlist positive, donc aucun nom ne traverse par héritage,
préfixé ou non ; ces deux-là traversent par relais explicite (mika#2508).*

1. **A1 — `config.rs:1065`** (doc de `pilot_cost_alert_usd`). Ce qui reste vrai et
   doit rester écrit : `MIKA_PILOT_COST_ALERT_USD` est préfixée parce qu'elle est
   lue par **mika-spirit**, qui n'est ni scrubbé ni `env_clear()`é. **L'asymétrie
   de préfixe est correcte ; seule sa justification était fausse** — c'est la
   nuance à ne pas perdre en réécrivant.
2. **A2 — `config.rs:1374`** (`DEFAULT_PILOT_LOG_DIR`). Son raisonnement « une
   divergence ne peut acheter que de l'inertie, jamais un faux positif » **reste
   vrai et reste écrit** ; ce qui change est la cause de la divergence, et le fait
   que depuis ce correctif elle devient **réglée** au lieu de garantie (§ 2.2).
3. **A3 — `crates/mika-agent/CLAUDE.md:2506`** (§ cost overrun) — une phrase.
4. **A4 — `crates/mika-agent/CLAUDE.md:2527`** (chemin dérivé du log pilote) — même
   phrase ; son argument fail-safe est intact.
5. **A5 — `CLAUDE.md:277`** (Signal S, *Halte 1*). La halte elle-même est
   **inchangée et le reste** : lire une divergence avant de conclure sur l'egress
   est juste. Seule la parenthèse causale est corrigée.
6. **A6 — `CLAUDE.md:502`** (`MIKA_PILOT_LOG_DIR`) — la moitié moteur du couple.
7. **A7 — `CLAUDE.md:517`, § *pilot turn budget*** — le site le plus lu, et le seul
   qui porte **le mauvais mécanisme ET le précédent cassé** (« by a precedent held
   in the same file: `PILOT_LOG_DIR` is too »). À réécrire en entier : les deux
   noms sont nus **par format de fil** (ils sont déjà posés dans les `.env` et les
   commandes opérateur publiées), ils traversent **par relais explicite**, et le
   précédent invoqué était lui-même faux. Y porter aussi la correction de V1a
   (elle a mesuré `scrub_mika_env_vars`, qui n'est pas sur ce chemin) et le fait
   que **V1b est désormais exécutable et rouge avant ce correctif**.

**Classe B (B1/B2/B3, § 2.1) — une ligne chacun, conclusion inchangée.** Préciser
`env_clear()` + allowlist là où ils écrivent « scrub », sans toucher au
raisonnement : celui de B1 (ne pas « harmoniser » sur `${MIKA_HOME:-…}`) et la
table de B2 restent exacts mot pour mot.

Ajouter enfin au § *pilot turn budget* la ligne d'inertie que l'opérateur doit
pouvoir lire : un `pilot_budget_armed … source=default` alors que la variable
**est** posée sur le service ne signifie plus « défaut de flotte » mais « binaire
antérieur à mika#2508 » (classe mika#2340) — établir le déploiement avant toute
conclusion sur le réglage.

### 5.3 — Le scan de classe (R4, R5)

Test Rust dans `executor.rs`, à côté des constantes qu'il protège — c'est là qu'un
futur contributeur ajoutant un relais le verra. Il lit `dispatch-lib.sh` par
`CARGO_MANIFEST_DIR/../../skills/bundled/_shared/dispatch-lib.sh`.

**Prédicat frozen, six termes.** Un scan approximatif sur ce fichier est faux dans
les deux sens (mika#2496 U3 l'a mesuré sur un prédicat voisin) :

1. **Unité d'analyse : la ligne, commentaires retirés d'abord** (`^\s*#`).
2. **Les `$` échappés sont retirés avant extraction** (`\$VAR` dans une chaîne de
   prose destinée à un corps de PR ou à un message opérateur). C'est le faux positif
   `MIKA_SPIRIT_LOG_FILE` du § 2, et la classe mika#2050 / mika#2201.
3. **Lecture** = `\$\{?([A-Z][A-Z0-9_]*)\b`. Majuscule initiale obligatoire : la
   convention du fichier réserve `_PILOT_*` / `_ARCH_*` aux variables internes, et
   celles-ci sont de toute façon écrites (terme 4).
4. **Écriture** = `VAR=`, `VAR+=`, `export VAR=`, `local VAR=`, `declare VAR=`,
   `read … VAR`, `for VAR in`, en début de ligne ou après `;` / `&&` / `||` / `{`.
   Une variable écrite quelque part dans le fichier sort de la population : elle
   n'attend rien de l'extérieur.
4bis. **Sauf si l'écriture se lit elle-même.** `export VAR="${VAR:-défaut}"` est
   une écriture **et** une lecture d'un réglage externe — c'est l'idiome canonique
   d'un knob opérateur avec défaut, pas une variable interne. Une écriture dont la
   partie droite référence la variable écrite **ne l'évince pas** de la population.

   **Ce terme n'est pas défensif, il est correctif.** Sans lui, le scan livré par
   ce plan serait **rouge au premier `cargo test`** — et de la pire façon, celle
   qui accuse le plan de mentir : `dispatch-lib.sh:8395` porte
   `export CLAUDE_PILOT_MIN_TOOL_CALLS="${CLAUDE_PILOT_MIN_TOOL_CALLS:-3}"`, le
   terme 4 nu l'évincerait de la population, et l'assertion auto-nettoyante du
   § Fire-Disposition rougirait alors sur l'entrée `CLAUDE_PILOT_MIN_TOOL_CALLS`
   de `DISPATCH_ENV_KNOWN_INERT` en disant « cette exception ne correspond à rien ».
   Le tableau du § 2 et l'allowlist seraient **contradictoires avec le prédicat du
   même plan**. Vérifié sur l'arbre : c'est la seule occurrence de cette forme dans
   le fichier aujourd'hui, mais c'est la forme que prendra le prochain knob.
5. **Builtins du shell retirés** par liste nommée (`PATH`, `HOME`, `PWD`, `SECONDS`,
   `TMPDIR`, `USER`, `HOSTNAME`, `SHELL`, `TERM`, `LANG`, `GIT_DIR`, `SSH_AUTH_SOCK`…).
   La liste est **explicite et commentée**, jamais devinée : plusieurs de ces noms
   sont par ailleurs dans `SANDBOX_ENV_CORE_ALLOWLIST` et y passeraient le test sans
   rien attester.
6. **Population = lues − (écrites non self-référentielles) − builtins.** Chaque
   membre doit être admis par `is_sandbox_env_allowed`, couvert par un relais, ou
   porter une **exception nommée** (§ Fire-Disposition). Les relais existants :
   `PILOT_DISPATCH_ENV`, `RESCUE_VERIFY_ENV` et `ARCH_ASK_RETRY_ENV` sont des
   `&[&str]` ; `DISPATCH_WORKTREE_ENV` et `PILOT_TRANSCRIPT_ENV` sont des **`&str`
   scalaires** (vérifié : `executor.rs:164` et `:236`), et `GH_TOKEN` est un
   littéral injecté en clair — les trois formes doivent être agrégées
   explicitement, un `.iter().chain(…)` naïf sur les cinq ne compile pas.

**Anti-vacuité (R5), trois assertions avant toute autre :** le fichier existe, il
fait plus de 100 Ko, et la population extraite compte au moins 8 membres. Sans elles,
un scan dont le chemin pourrit ou dont le prédicat se resserre trop passe en
regardant zéro ligne.

**Cinq contrôles négatifs**, sur des fixtures shell **inline** (jamais le vrai
fichier, qui changera), construits sur les formes réellement présentes :

| fixture | attendu | ce qu'elle atteste |
|---|---|---|
| N1 `foo="${OPERATOR_KNOB:-x}"` | **dans** la population | le cas nominal |
| N2 `OPERATOR_KNOB=3; echo "$OPERATOR_KNOB"` | **hors** population | terme 4 — l'écriture évince |
| N3 `# echo "$OPERATOR_KNOB"` | **hors** population | terme 1 — le commentaire |
| N4 `printf 'grep x \$OPERATOR_KNOB'` | **hors** population | terme 2 — la prose échappée |
| N5 `echo "$_INTERNAL"` | **hors** population | terme 3 — la convention interne |
| N6 `export KNOB="${KNOB:-3}"` | **dans** la population | terme 4bis — l'écriture self-référentielle n'évince pas |

N1, N3, N4 et **N6** sont à **voir rouges** en retirant leur terme respectif avant
d'être déclarés verts : un contrôle négatif jamais vu rouge n'atteste rien. N6 est
le plus important des six — il est construit sur la forme **réellement présente**
à `dispatch-lib.sh:8395`, et c'est le seul dont l'absence casserait le plan
contre lui-même plutôt que de laisser passer un cas.

### 5.4 — Le test du DoD sur `is_sandbox_env_allowed`

Le DoD demande trois assertions. **Deux existent déjà** et il serait faux de les
redupliquer : `sandbox_env_denies_secret_vars` (contrôle négatif d'une variable non
allowlistée) et `sandbox_env_denies_mika_prefixed_vars_even_if_core_listed` (un
`MIKA_*` refusé même si listé). Le plan les **nomme** dans le corps de PR comme déjà
couvertes.

La troisième — « `PILOT_MAX_TURNS` est admise » — est le remède refusé. Elle est
remplacée par son inverse exact, qui atteste la même propriété finale (la variable
atteint le child) par le canal juste :

```rust
/// mika#2508 : les deux réglages du canal pilote atteignent `dispatch-lib.sh` par
/// injection explicite, et JAMAIS par héritage. Extension de la population de
/// `mika2354_rescue_verify_env_never_joins_the_sandbox_allowlist`.
#[test]
fn mika2508_the_pilot_dispatch_env_never_joins_the_sandbox_allowlist() { … }

/// mika#2508 : le relais préserve le ROLLBACK. `PILOT_MAX_TURNS=""` doit arriver
/// sur le child comme une variable DÉFINIE et vide — `_pilot_max_turns` le lit
/// avec `${VAR+set}` et en fait le rollback, pas le défaut de flotte.
#[test]
fn mika2508_an_empty_pilot_knob_is_relayed_not_dropped() { … }
```

Plus le pendant absent/présent (`read → None` ⇒ rien relayé) et une assertion que
`relayed_env_pairs` **n'a pas changé de comportement** pour ses deux familles
d'origine.

---

## 6. Fire-Disposition

Ce plan livre un détecteur (§ 5.3 : le scan de classe R4) dont la population
existante est **non nulle** — les six `MIKA_*` du § 2 le feraient rougir au premier
`cargo test`.

**Option retenue : (a) exception nommée en allowlist.**

Pourquoi pas les deux autres. **(b) livrer désarmé** (`#[ignore]`) produirait un
détecteur muet dont le silence se lit comme une flotte saine — exactement la classe
mika#2205, et sur un scan dont toute la valeur est de rougir au prochain ajout.
**(c) halte-et-remontée** bloquerait la PR sur une décision de sûreté
(`MIKA_PILOT_SANDBOX` doit-il être désarmable depuis l'environnement du service ?)
qui n'a pas besoin d'être tranchée pour fermer le défaut mesuré, et qui mérite son
propre cadrage.

**Forme de l'allowlist :**

```rust
/// Variables lues par `dispatch-lib.sh` qui ne traversent PAS le child de dispatch
/// et dont l'inertie est connue, datée et suivie (mika#2508 § 2).
///
/// **Ce n'est pas une liste d'exemptions permanentes.** Chaque entrée est une
/// inertie mesurée, dont la résolution est une décision de canal que mika#2508 n'a
/// pas prise : voir le suivi porté par l'umbrella mika#2491. Quand une entrée est
/// tranchée, on la RELAIE et on retire sa ligne — on n'élargit pas la liste.
const DISPATCH_ENV_KNOWN_INERT: &[(&str, &str)] = &[
    ("MIKA_PILOT_SANDBOX",          "mika#2491 — kill-switch du confinement bwrap ; \
                                     le relayer donnerait à l'env du service un levier \
                                     de désarmement : décision de sûreté, pas de canal"),
    ("MIKA_PILOT_EGRESS_LOG_DIR",   "mika#2491 — puits du journal du relais d'egress"),
    ("MIKA_PLATFORM_DIR",           "mika#2491 — racine plateforme"),
    ("MIKA_HOME",                   "mika#2491 — l'epoch mika#2026 s'écrit sous $HOME/.mika"),
    ("MIKA_PR_ORIGIN_LABEL_COLOR",  "mika#2491 — couleur de label"),
    ("CLAUDE_PILOT_MIN_TOOL_CALLS", "mika#2491 — seuil figé à 3"),
];
```

**Assertion auto-nettoyante**, dans le même test : toute entrée qui **n'est plus**
dans la population — variable retirée de `dispatch-lib.sh`, ou devenue relayée /
allowlistée — fait **rougir**, avec le message « retirer cette ligne de
`DISPATCH_ENV_KNOWN_INERT` ». C'est ce qui empêche l'allowlist de survivre à sa
raison d'être, et ce qui rend le jour de la réparation visible au lieu de
silencieux.

**Chaque entrée nomme la donnée précise** (la variable), **référence un suivi**
(mika#2491, l'umbrella qui porte le cadre substrat — le ticket enfant dédié est
nommé dans le corps de PR), et la liste est **grep-visible** en un seul site.

---

## 7. Verification Contract

**Déterministe (CI, `cargo test -p mika-agent` + `make test-dispatch-lib`) :**

- **V1** — `mika2508_the_pilot_dispatch_env_never_joins_the_sandbox_allowlist` :
  les deux noms sont refusés par `is_sandbox_env_allowed`, absents de
  `SANDBOX_ENV_CORE_ALLOWLIST`, non couverts par `SANDBOX_ENV_ALLOWED_PREFIXES`.
- **V2** — le relais rend exactement les paires présentes ; l'absence ne relaie
  rien ; **la valeur vide EST relayée** (le rollback survit) ; `relayed_env_pairs`
  est inchangée pour ses deux familles d'origine.
- **V3** — le scan de classe est vert sur l'arbre, avec ses trois assertions
  d'anti-vacuité et ses **six** contrôles négatifs (N1/N3/N4/**N6** **vus rouges**
  avant d'être déclarés verts). **V3 est la vérification qui doit être exécutée en
  premier** : si elle rougit sur `CLAUDE_PILOT_MIN_TOOL_CALLS`, le terme 4bis n'est
  pas implémenté et c'est le prédicat qu'il faut corriger — **jamais** l'entrée de
  `DISPATCH_ENV_KNOWN_INERT`, dont le retrait ferait taire le scan en le laissant
  faux.
- **V4** — l'assertion auto-nettoyante rougit sur une entrée `DISPATCH_ENV_KNOWN_INERT`
  factice pointant une variable que `dispatch-lib.sh` ne lit pas.
- **V5** — non-régression : `mika2354_rescue_verify_env_never_joins_the_sandbox_allowlist`,
  `sandbox_env_denies_secret_vars`, `sandbox_env_denies_mika_prefixed_vars_even_if_core_listed`
  et les ~10 assertions shell de mika#2496 passent **sans modification**.

**Ce qui n'est PAS testable déterministe, écrit plutôt que découvert.** « La
variable survit à `env_clear` dans un vrai spawn » exige de lancer un sous-processus
et de muter l'environnement du process de test. Le contrat côté Rust est *le relais
pose la paire sur la commande*, et V2 l'atteste sur une fonction pure. La moitié
comportementale est la sonde S1 ci-dessous — c'est la même frontière que mika#2496 a
dû écrire pour son propre drapeau, et c'est précisément là que la panne s'est logée :
**la sonde est donc obligatoire, pas décorative.**

**Sondes post-déploiement, et leurs haltes.**

```bash
# S1 — le réglage atteint l'argv (1er dispatch après déploiement).
#      Ancre obligatoire : le .stderr porte aussi la prose du pilote (mika#2050).
grep -h '^dispatch-lib: pilot_budget_armed' \
     "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | tail

# S2 — contrôle POSITIF : combien de dispatches ont seulement résolu un budget ?
grep -l '^dispatch-lib: pilot_budget_armed' \
     "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | wc -l

# S3 — l'argv réelle du pilote, sur un dispatch en vol
ps -eo pid,args | grep -F 'claude-pilot --verbose' | grep -F -- '--max-turns'
```

- **S1 attendu :** `max_turns=150 source=env cost_bound=absent_upstream`.
- **Halte 1 — `source=default` alors que la variable est posée sur le service.**
  Ne pas toucher au relais : établir d'abord que le binaire servi porte le
  correctif (classe mika#2340). Le relais est du code Rust, donc un `make deploy`,
  pas un seed de skill.
- **Halte 2 — S1 vide et S2 rend 0.** La commande regarde au mauvais endroit, ou
  aucun dispatch n'a tourné. **Zéro ne vaut « sain » que si S2 est non nul** : un
  frein qu'on n'a pas déployé se lit exactement comme un frein qui ne mord jamais
  (mika#2205). Vérifier `PILOT_LOG_DIR` / `MIKA_PILOT_LOG_DIR` — et noter que
  **depuis ce correctif ces deux-là peuvent enfin diverger de façon réglée**, alors
  qu'avant le shell était toujours sur le défaut.
- **Halte 3 — `source=env` mais S3 ne montre aucun `--max-turns`.** Le résolveur a
  la valeur et le site de lancement ne la passe pas : c'est la moitié **shell**, que
  ce plan ne touche pas et que mika#2496 V4 tient. Relire le scan de co-location
  avant toute conclusion.
- **Halte 4 — le plafond mord et détruit du travail** (`error_max_turns` sans PR de
  récupération). **Désarmer d'abord** (`PILOT_MAX_TURNS=0` sur le service —
  rollback en un geste, sans redéploiement), diagnostiquer ensuite. Un frein qui
  détruit est pire que l'emballement qu'il coupe. *C'est la halte 2 de mika#2496,
  qui devient pour la première fois atteignable.*

**Ce que ce travail n'achète pas.** Aucun compteur, aucun événement nouveau : la
ligne `pilot_budget_armed` existe déjà et disait la vérité — `source=default` était
un rapport **exact** d'un canal rompu. Ce plan ne rend pas l'instrument plus
bavard ; il rend le réglage réel. Et il ne tranche la valeur de personne : le défaut
de flotte reste vide (désarmé), la V2 de mika#2496 reste due, et **le kill de rule-1
reste la seule borne tant que ce correctif n'est pas déployé**.

---

## 8. Definition of Done

- `PILOT_MAX_TURNS` et `PILOT_LOG_DIR` sont relayées explicitement au child de
  dispatch ; les deux allowlists sandbox sont **inchangées**.
- Le relais préserve les trois paliers du lecteur shell, valeur vide comprise.
- Les **sept** sites de classe A sont corrigés (§ 2.1), les trois sites de classe
  B reçoivent la précision de mécanisme sans changement de conclusion, et le
  § *pilot turn budget* porte en plus la correction de V1a et la nouvelle lecture
  d'inertie (`source=default` ⇒ suspecter le binaire, classe mika#2340).
- Le scan de classe est livré, vert, avec anti-vacuité, cinq contrôles négatifs et
  son allowlist d'exceptions nommées auto-nettoyante.
- `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` et
  `make test-dispatch-lib` passent.
- Le corps de PR **rappelle la divergence ratifiée** (relais au lieu d'allowlist,
  opérateur 2026-09-24 08:10Z, raison : surface de sûreté plus petite), nomme les
  deux assertions du DoD **déjà couvertes** plutôt que de les redupliquer, et pose
  la sonde S1 comme la vérification opérateur restante.

---

## Acceptance criteria

1. **AC1** — Au 1er dispatch après déploiement, avec `PILOT_MAX_TURNS=150` sur
   l'environnement du service : la ligne `pilot_budget_armed` porte
   `max_turns=150 source=env` (jamais `source=default`, jamais `max_turns=none`),
   **et** la cmdline du pilote porte `--max-turns 150`.
2. **AC2** — Un test atteste que `PILOT_MAX_TURNS` traverse `env_clear()`. *Moyen
   divergent, **ratifié** le 2026-09-24 08:10Z (§ 0)* : elle traverse par **relais
   explicite**, non par admission dans l'allowlist — mika#2354 AC9(b) interdit la
   seconde voie par test. L'assertion livrée est l'inverse exacte de celle
   demandée, et atteste la même propriété finale.
3. **AC3** — Contrôle négatif : une variable non allowlistée et non relayée est
   refusée. *Déjà couvert par `sandbox_env_denies_secret_vars`* — nommé, non
   redupliqué.
4. **AC4** — Un `MIKA_*` reste refusé même s'il était listé. *Déjà couvert par
   `sandbox_env_denies_mika_prefixed_vars_even_if_core_listed`* — nommé, non
   redupliqué.
5. **AC5** — Les commentaires de `crates/mika-common/src/config.rs` et
   `crates/mika-agent/CLAUDE.md` sont corrigés : le mécanisme est `env_clear()` +
   allowlist positive, et un nom nu ne suffit pas. *Élargi par la mesure* : ces
   deux fichiers portent **deux** sites chacun (A1/A2 et A3/A4, § 2.1), pas un.
6. **AC6** — *(au-delà du DoD du ticket)* Le root `CLAUDE.md` est corrigé à ses
   **trois** sites (A5 Signal S *Halte 1*, A6 `MIKA_PILOT_LOG_DIR`, A7 § *pilot
   turn budget*). A7 est le plus lu et le seul qui porte **à la fois** le mauvais
   mécanisme et le précédent cassé, qu'il emprunte à A6/A2 — l'erreur se citait
   elle-même à travers deux fichiers, ce qui est la raison pour laquelle la
   corriger à un seul site ne l'aurait pas éteinte.
7. **AC7** — *(point 3 du ticket, exécuté)* L'angle mort de classe est relevé,
   `PILOT_LOG_DIR` est réparé avec lui, et les six inerties restantes portent une
   exception nommée avec suivi plutôt qu'un silence.
8. **AC8** — Aucune régression : les gardes de confinement existantes et les
   assertions shell de mika#2496 passent sans modification.

---

## 10. Hors périmètre, délibérément

- **Les six `MIKA_*` inertes** du § 2 : six décisions de canal distinctes, dont
  `MIKA_PILOT_SANDBOX` qui est un arbitrage de **sûreté** (désarmer le confinement
  depuis l'environnement du service). Exceptées nommément, suivi sur mika#2491.
- **Le renommage de `PILOT_MAX_TURNS` / `PILOT_LOG_DIR` en forme préfixée.** La
  justification de leur forme nue est morte, mais les noms sont un **format de fil**
  déjà posé dans `~/.mika/.env` et dans plusieurs commandes opérateur publiées ;
  renommer pendant qu'on répare le canal ferait changer deux choses à la fois sur un
  frein de coût. Dette de vocabulaire nommée, suivi.
- **Le dédoublement `MIKA_PILOT_LOG_DIR` (moteur) / `PILOT_LOG_DIR` (shell).** Sa
  raison d'être — « un nom préfixé serait retiré en chemin » — est fausse pour la
  même raison que le reste. Unifier est possible **après** ce correctif, jamais
  pendant : la divergence des deux noms est aujourd'hui la seule chose qui rend la
  halte 2 de mika#2249 lisible.
- **La valeur du plafond.** La V2 de mika#2496 (distribution mesurée des tours des
  runs aboutis) reste due et reste un geste opérateur sur l'hôte : ce plan rend le
  réglage **atteignable**, il ne choisit pas sa valeur.
- **Le frein en dollars.** Toujours absent en amont (`_sdk_guardrail_kwargs` finit
  sur `pass`) — suivi `senara-solutions/claude-pilot`, inchangé.
- **`scrub_mika_env_vars`** et le chemin non-pilote (`executor.rs:1181`), qui reste
  un scrub négatif et n'est pas concerné par cette classe. Les ~30 mentions de
  `scrub_mika_env_vars` sous `docs/solutions/`, `todos/` et les plans antérieurs
  sont **hors population** : elles décrivent le chemin exec-handler, où le scrub
  est bien le mécanisme, ou elles sont des archives datées qu'on ne réécrit pas.
- **Le raisonnement des trois sites de classe B** (§ 2.1). Ils reçoivent la
  précision de mécanisme et **rien d'autre** : leur conclusion est vraie sous les
  deux mécanismes, et réécrire un raisonnement juste — dont un marqueur de sûreté
  (`pilot_egress_stamp.rs`) et un runbook opérateur — pour un seul mot coûte plus
  de relecture que l'imprécision ne coûte de confusion.
