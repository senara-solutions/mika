# mika#2329 — le STOP global n'a pas d'interrupteur à chaud, et le geste que l'opérateur croit poser ne peut pas en être un

- **Ticket :** senara-solutions/mika#2329
- **Priorité :** p1 (substrat — l'opérateur croit avoir coupé la boucle et ne l'a pas fait)
- **Branche :** `feat/2329/stop-global-hot-swappable-mika-dev-auto`
- **Lignage :** #2315 / PR #2325 (le park par ticket — premier sens du « STOP ne tient pas »), #2313 (le STOP P0 pendant lequel le défaut a été vécu), #2271 (le cancel-par-knob qui a déjà dû être réparé une fois), #1742 (la garde anti-zombie que #2271 a dû exempter), #1363 (l'origine du knob), #2205 (un scan silencieusement inactif se lit comme un scan oisif)

---

## Contexte

`MIKA_DEV_AUTO_PULL=0` est lu une seule fois, au démarrage, dans `init_agent`. La
branche knob-off annule la row récurrente `auto_pull_groomed`, la branche knob-on la
crée. Rien ne relit la variable ensuite. Couper le feeder en pleine incidence exige
donc un redémarrage de mika-spirit — précisément ce qu'on veut le moins faire pendant
un P0 (dispatch en vol, worktrees ouverts, session pilote active).

Le ticket demande qu'un STOP global prenne effet au tick suivant, sans redémarrage, et
soit réversible de la même manière. Il propose trois pistes non tranchées.

**Ce plan en tranche une quatrième, parce que la lecture du code déplace le
diagnostic sur deux points.** La piste que le ticket cite en premier — relire l'env à
chaque exécution — ne produirait pas un STOP à chaud pour le vecteur que le ticket
décrit lui-même (E2). Et le mécanisme actuel, annuler la row, a déjà coûté un ticket de
réparation dont la machinerie est encore là (E4). Les deux faits pointent dans la même
direction : un objet distinct, lu à l'exécution, qui ne touche aucune row.

---

## Ce qui est établi, et comment le vérifier

### E1 — La lecture unique est à `server/mod.rs:1621`, pas `:1581`

Le ticket cite `crates/mika-agent/src/server/mod.rs:1581`. La lecture réelle est
`mod.rs:1621` :

```rust
if std::env::var("MIKA_DEV_AUTO_PULL").map(|v| v == "0").unwrap_or(false) {
    info!(agent = %name, "auto_pull disabled via MIKA_DEV_AUTO_PULL=0");
    if let Err(e) = db.cancel_recurring_task_by_label("auto_pull_groomed").await { … }
} else {
    task_engine::ensure_recurring_task(&db, "auto_pull_groomed", AUTO_PULL_CRON, …).await;
}
```

Dérive de ligne ordinaire, sans conséquence — notée pour que le relecteur ne cherche
pas au mauvais endroit. `grep -rn MIKA_DEV_AUTO_PULL --include=*.rs crates/` rend
exactement deux sites de lecture, tous deux dans ce bloc.

### E2 — Relire l'env à chaque tick ne rendrait PAS le knob hot-swappable

C'est la rectification centrale, et elle invalide la première piste du ticket telle
qu'écrite.

`crates/mika-agent/src/bin/mika-spirit.rs:28` appelle
`mika_common::dotenv::load_dotenv(&home_dir)` **une fois**, au démarrage.
`load_dotenv` délègue à `dotenvy`, qui lit le fichier et pose les variables dans
l'environnement du process. Rien ne surveille le fichier ensuite : aucun watcher,
aucun rechargement, aucun second appel dans tout le crate agent (`grep -rn load_dotenv
--include=*.rs crates/mika-agent/` ne rend aucun site).

Or l'environnement d'un process Linux vivant n'est pas mutable de l'extérieur. Donc
**éditer `~/.mika/.env` ne change rien à ce que `std::env::var` renverra**, même appelé
à chaque tick : la valeur est figée depuis le boot.

Conséquence directe : le geste que le ticket décrit — « l'opérateur qui pose
`MIKA_DEV_AUTO_PULL=0` dans le `.env` pendant un STOP P0 » — **resterait sans effet
après le correctif que le ticket propose**. Le défaut ne serait pas fermé, il serait
déplacé d'un cran et rendu plus difficile à voir, puisque le code aurait alors l'air de
relire.

Une variante existe et est exploitable : relire le **fichier** (pas l'env) à chaque
tick, via `parse_dotenv` (E8). Elle est écartée en D1, pour la raison d'E3.

### E3 — La maison fige délibérément les variables d'env, et ce n'est pas un accident

Quatre variables au moins sont documentées « not hot-swappable », lues une fois par
process et mises en cache : `MIKA_AGENT_TIER`, `MIKA_DEPLOYMENT`,
`MIKA_TELEGRAM_HTML_RENDER`, `MIKA_LOG_LLM_BODIES`. Le `TaskDispatcher` porte la
justification en commentaire, sur le champ `tier` :

> Silent turns read this instead of `AgentTier::from_env()` so a mid-runtime env change
> cannot flip the tier of a running dispatcher.

Rendre **une** variable `MIKA_*` relue à chaud en ferait une exception invisible : deux
variables d'apparence strictement identique, l'une relue, l'autre pas, et rien dans le
nom, la forme ou le lieu de déclaration ne dirait laquelle. Un opérateur qui apprend sur
`MIKA_DEV_AUTO_PULL` que « ça se relit » transportera cette croyance sur
`MIKA_DEV_WIP_RESCUE`, qui a la forme jumelle (E9) et ne se relit pas.

D'où : l'interrupteur à chaud doit être un **objet distinct**, dont la nature hot se lit
sur l'objet, pas une variable d'env promue au cas particulier.

### E4 — Le mécanisme actuel (annuler la row) a déjà coûté un ticket de réparation

mika#2271, mesuré le 2026-09-09 :
`docs/plans/2026-09-09-004-fix-2271-feeder-reregister-after-knob-plan.md`.

> `MIKA_DEV_AUTO_PULL=0` **annule** (cancelled) la task récurrente feeder. Au retour
> (knob retiré + restart), une garde lignée mika#1742 — pensée pour ne pas ressusciter
> une task volontairement tuée — refuse la ré-inscription, laissant le feeder mort.
> Mesuré 2026-09-09 : restart 19:20 sans knob, aucun tick auto_feeder, ready manuel
> obligatoire.

La réparation est encore là, et elle n'est pas petite : `RECURRING_CONFIG_CANCEL_REVERTED_PATH`
(`db.rs:66`), `Database::revert_config_cancel_recurring_task` (writer), et une exemption
dédiée dans `create_recurring_task_if_absent` (`db.rs:6207`), qui coexiste maintenant
avec l'exemption mika#2337 et la garde `RECURRING_ZOMBIE_GRACE_HOURS`.

**C'est l'argument de conception décisif, et il n'est pas dans le ticket.** Tout STOP qui
passe par l'annulation de la row paie cette machinerie et son risque : la réversibilité
n'y est pas une propriété, c'est une exemption qu'il faut écrire, tenir, et ne pas
casser en ajoutant la suivante. Un STOP qui court-circuite à l'exécution laisse la row
vivante et n'entre en contact avec aucune des trois.

### E5 — Le point de court-circuit le moins cher est en tête de `dispatch_auto_pull_groomed`

`dispatcher.rs:1184`, dans l'ordre : `resolve_periodic_scan_token` (PAT puis repli App,
donc potentiellement un échange de token App sur le réseau), puis
`resolve_periodic_scan_label_token` (seconde résolution, identité App — mika#2228), puis
`auto_pull_groomed_ticket`, dont les deux premières instructions sont
`gh_list_open_issues` et `gh_list_open_pr_closing_issues`.

Court-circuiter en tête de `dispatch_auto_pull_groomed` économise les deux résolutions
et les deux fetchs. Placer la garde plus bas (dans `auto_pull_groomed_ticket`) ferait
payer au STOP deux résolutions de token par tick pour rien.

Précédent de placement raisonné, même raisonnement littéral : la porte 2c de mika#2279,
placée avant `gh issue view` parce que « le prédicat lit l'URL de l'issue, jamais son
corps ».

### E6 — Le `home_dir` du dispatcher est le home PER-AGENT, pas le global

`server/mod.rs:558` : `home_dir: agent_home.to_path_buf()`. Le `TaskDispatcher` ne porte
aucun champ pointant le home global.

Or le précédent maison des fichiers d'état est global :
`$HOME/.mika/state/pilot-gitconfig` (`dispatch-lib.sh:355`) et
`${MIKA_HOME:-$HOME/.mika}/state/pr-origin-epoch` (`dispatch-lib.sh:5683`,
`scripts/pr-origin-report.sh:94`).

Le câblage est trivial et c'est mesuré : `init_agent` reçoit `global_home: &std::path::Path`
en paramètre (`mod.rs:440`) et construit le `TaskDispatcher` **dans la même fonction**
(`mod.rs:551`). Il y a exactement **une** construction en production (la ligne 628 voisine
est celle d'`AgentState`, pas d'un second dispatcher).

Deux pièges évités en le disant : dériver le global par `../..` depuis l'agent home
(fragile, et faux dès que la disposition change), ou appeler `resolve_home_dir()` au tick
(qui relit `MIKA_HOME`/`HOME`, réintroduisant exactement la dépendance à l'environnement
que ce ticket retire — précédent du piège : mika#1968).

### E7 — « Au tick suivant » vaut ≤ 10 minutes, et c'est conforme

`AUTO_PULL_CRON = "0 */10 * * * *"` (`server/mod.rs:96`). Le STOP prend donc effet en
10 minutes au pire.

C'est exactement l'exigence écrite du ticket (« prendre effet **au tick suivant** »). Un
STOP instantané reste le geste existant — arrêter le service — et ce ticket sert
précisément le cas où l'on ne veut pas le poser. Dit ici pour qu'aucun relecteur ne lise
la latence comme un défaut du correctif.

### E8 — `parse_dotenv` existe déjà, et c'est ce qui rend la garde D4 gratuite à écrire

`crates/mika-common/src/dotenv.rs:107` :
`pub fn parse_dotenv(home_dir: &Path) -> HashMap<String, String>`. Elle lit le fichier
**sans le charger dans l'environnement**, est silencieuse sur fichier absent, WARN sur
parse invalide, et ne panique pas. `mika-spirit.rs:39` s'en sert déjà au boot pour son
propre diagnostic.

Aucun parseur à écrire pour D4.

### E9 — Deux scans jumeaux ont le même défaut, et ce ticket ne les couvre pas

`MIKA_DEV_WIP_RESCUE` a la forme strictement identique, vingt lignes plus bas
(`mod.rs:1644` : même `map(|v| v == "0")`, même `cancel_recurring_task_by_label`, même
`ensure_recurring_task`). `MIKA_QA_REVIEW_RECONCILE` a la même forme boot-time.

Établi ici pour que D7 puisse trancher la portée en connaissance de cause, et pour que
le mécanisme soit écrit généralisable sans être généralisé.

### E10 — Ce que ce plan ne prouve PAS

Aucune trace de production n'a été relue pour ce ticket : le défaut est structurel et se
lit dans le code (E1, E2, E4), mais l'incident du 2026-09-13 (#2313) n'est pas rejoué
ici. Ce plan n'établit donc pas **combien de fois** le piège a été rencontré, seulement
qu'il est ouvert et qu'il ne peut pas se refermer par la piste 1. Ça ne bloque pas : les
deux corrections ne dépendent d'aucun volume.

---

## Décisions

### D1 — Un fichier sentinelle, pas une variable d'env promue

`{global_home}/state/auto-pull-stop`. Son **existence** vaut STOP ; son contenu n'est
jamais lu (un fichier vide est un STOP valide — lire le contenu créerait une seconde
question, « que vaut un contenu invalide ? », dont la réponse fail-open ou fail-closed
serait un piège de plus).

Trois raisons, dans l'ordre de force :

1. **L'env ne peut pas être hot pour le vecteur décrit** (E2). La piste 1 du ticket ne
   ferme pas le défaut.
2. **Ne pas créer d'exception invisible dans la famille `MIKA_*`** (E3). La nature hot
   doit se lire sur l'objet.
3. **Robustesse du geste en P0.** `touch` et `rm` fonctionnent depuis n'importe quel
   shell, sans binaire `mika`, sans DB joignable, sans redémarrage, et sans que
   l'opérateur ait à se souvenir d'une syntaxe. C'est le geste le plus court disponible
   pendant un incident.

Écartées : le toggle `identity.toml` (piste 2 du ticket) — les sections code-owned sont
réécrites par `reconcile_well_known_config` (mika#2330), donc un STOP posé là peut être
effacé au redémarrage suivant, ce qui est le pire mode de panne possible pour un
interrupteur d'arrêt ; et la commande CLI écrivant en DB (piste 3) — voir D8.

### D2 — Court-circuit à l'exécution : la row n'est jamais touchée

Conséquence directe d'E4. Le STOP n'annule pas `auto_pull_groomed`, ne la marque pas, ne
la reprogramme pas. La row continue de tourner ; c'est son **dispatch** qui rend la main
immédiatement.

Propriété obtenue, et c'est celle que le ticket demande : la réversibilité n'est pas une
machinerie, c'est l'absence de machinerie. Aucun contact avec la garde anti-zombie
mika#1742, ni avec l'exemption config-cancel mika#2271, ni avec
`RECURRING_ZOMBIE_GRACE_HOURS`. Rien à ressusciter, donc rien qui puisse refuser de
ressusciter.

### D3 — La branche boot-time ne bouge pas

`MIKA_DEV_AUTO_PULL=0` reste ce qu'il est : un réglage de **démarrage** qui annule la
row. Deux mécanismes, deux portées — exactement ce que le ticket énonce (« ce sont deux
mécanismes avec deux rayons d'action »).

Ni fusion ni suppression : supprimer la branche casserait les déploiements qui posent le
knob aujourd'hui, et la rendre hot est impossible (E2). Ce qui change, c'est qu'un
opérateur qui se trompe de mécanisme s'en aperçoit (D4) au lieu de croire avoir coupé.

### D4 — Le piège vécu se ferme par un WARN, pas par un mécanisme

**C'est l'ajout hors-ticket, et c'est lui qui ferme réellement l'incident.**

À chaque tick, `parse_dotenv` lit le `.env` **sur disque** (global puis per-agent, dans
l'ordre où `Settings::load_for_agent` les compose). Si le fichier porte
`MIKA_DEV_AUTO_PULL=0` alors qu'un tick est en train de tourner, l'opérateur a posé un
STOP qui ne coupe rien : WARN nommé, disant la valeur lue, le fichier, et le geste qui
marche.

**Le prédicat est juste par construction, et c'est ce qui le rend sûr.** Si le process
avait démarré avec le knob, la row serait annulée et *aucun tick ne tournerait* — le code
de la garde ne serait jamais atteint. La garde ne peut donc émettre que dans la
population « fichier édité après le boot », qui est exactement celle du ticket. Aucun
faux positif n'est atteignable par cette voie.

Sans ce WARN, le défaut reste ouvert dans sa forme la plus dangereuse : **un STOP
silencieusement inopérant se lit exactement comme un STOP qui marche.** L'opérateur en
P0 pose le knob, ne voit rien dans le log (il n'y a rien à voir), et conclut que la
boucle est arrêtée. C'est le symétrique inverse de mika#2205, où un scan silencieusement
inactif se lisait comme un scan oisif.

### D5 — INFO à chaque tick court-circuité, audit à chaque transition seulement

**INFO par tick**, comme le ticket l'exige explicitement (« un `info!` nommé à chaque
tick court-circuité, pour que le STOP soit *visible* dans le log et pas seulement
effectif »). Un STOP prolongé écrit 144 lignes/jour, et c'est voulu : pour un
interrupteur, la vivacité **est** l'information — un opérateur qui grep veut savoir que
le STOP est armé *maintenant*, pas qu'il l'a été un jour. Précédent assumé et
identiquement raisonné : Signal P (mika#2156), où une ligne par minute pendant qu'un
dispatch tourne est « attendu, pas une fuite ».

**`audit_events` par transition**, pas par tick : une ligne à l'armement, une à la levée
(`tool_name = "auto_pull_stop"`, `after_value` ∈ `{armed, lifted}`). Doctrine mika#2131 :
l'information durable est « le STOP a été armé à telle heure », pas « il l'était encore à
14 h 32 » — et 144 lignes/jour en audit déplaceraient dans la table le churn que la
doctrine borne.

La transition se détecte par comparaison avec l'état du tick précédent, gardé en mémoire
sur le dispatcher (`AtomicBool`, accessible derrière `&self` puisque le dispatcher vit
dans un `Arc`). **L'état est perdu au redémarrage, à dessein** : un process neuf
re-photographie l'état qu'il trouve et écrit une transition si le STOP est armé au
premier tick — même raisonnement que le jeu de déduplication mika#2131.

### D6 — Fail-open, nommé, et pourquoi il est acceptable *ici* seulement

`Path::exists()` renvoie `false` sur toute erreur d'accès (permissions, I/O). C'est un
fail-open : un fichier illisible fait tourner la boucle.

L'asymétrie penche pourtant du mauvais côté à première vue — un faux « pas de STOP » fait
tourner le feeder pendant un incident, ce qui est le défaut qu'on ferme. Deux choses le
rendent acceptable, et la seconde est load-bearing :

1. Le fail-closed n'est pas implémentable proprement : `exists()` ne distingue pas
   « absent » de « illisible ». Passer par `symlink_metadata()` pour trancher ferait du
   cas « répertoire `state/` inexistant » — le cas **nominal** sur une installation qui
   n'a jamais posé de STOP — une erreur, donc un STOP permanent. Le remède serait pire.
2. **L'opérateur constate l'effet au log en ≤ 10 minutes** (D5, E7). La boucle de
   rétroaction est courte et l'erreur est auto-détectable : poser le fichier et ne pas
   voir la ligne INFO est un signal immédiat et sans ambiguïté.

Le point 2 est ce qui autorise le point 1 : **le fail-open n'est acceptable que parce que
la visibilité D5 existe.** Si un jour le lecteur devient faillible d'une manière que
l'opérateur ne peut pas constater (DB, réseau), cet arbitrage est à refaire, pas à
transporter.

### D7 — Portée : `auto_pull` seul, mécanisme généralisable, non généralisé

`wip_rescue` et `qa_review_reconcile` ont le même défaut (E9). Ce ticket porte sur
`auto_pull` et ce plan s'y tient.

Le lecteur est néanmoins écrit paramétré par le nom du scan
(`is_stopped(global_home, "auto-pull")` → `state/auto-pull-stop`), pour que l'extension
soit une ligne. Elle n'est pas faite ici parce que **chaque scan a une population et un
coût d'arrêt différents** : arrêter la revue QA n'est pas la même décision qu'arrêter le
feeder, et livrer trois interrupteurs dont deux n'ont jamais été demandés créerait trois
gestes à documenter et à tester pour un besoin mesuré sur un seul. Ticket de suivi en fin
de plan.

### D8 — Pas de commande CLI, et pourquoi (piste 3 du ticket)

`mika dev auto-pull {pause,resume}` n'est pas livré. Trois raisons :

1. **Le geste doit survivre au contexte où on en a besoin.** Pendant un P0, le CLI peut
   lui-même être en difficulté (dépendances `.env`, A2A vers le démon depuis mika#1727) ;
   `touch` ne dépend de rien.
2. Il n'existe aucune sous-commande `mika dev` : l'ajouter est du travail clap +
   accès DB ou API, pour un geste que `touch` rend déjà.
3. Elle reste possible plus tard **par-dessus** ce mécanisme (elle écrirait le fichier),
   sans rien invalider. L'inverse n'est pas vrai : livrer d'abord la DB aurait figé le
   geste sur le composant le plus fragile.

### D9 — Un seul lecteur, et une garde structurelle

Le prédicat vit dans **un** module (`auto_pull_stop.rs`). Précédent explicite :
`grooming_marker` (mika#2158), né du constat qu'une copie de prédicat avait dérivé
pendant des mois en répondant différemment à la même question, sans que rien ne casse.

Garde structurelle (T7) : un scan de source refuse une seconde occurrence du littéral du
chemin ailleurs sous `crates/mika-agent/src/`. Un test comportemental ne peut pas voir
cette classe — une copie ne rendrait aucune décision fausse le jour où elle est écrite.

---

## Volets d'implémentation

### V1 — `crates/mika-agent/src/auto_pull_stop.rs` (nouveau)

Le lecteur unique (D9). Sans I/O coûteux, sans `async`.

- `const STOP_DIR: &str = "state";`
- `const AUTO_PULL_STOP_FILE: &str = "auto-pull-stop";`
- `pub fn stop_file_path(global_home: &Path, scan: &str) -> PathBuf` — `{global_home}/state/{scan}-stop`.
- `pub fn is_stopped(global_home: &Path, scan: &str) -> bool` — `stop_file_path(..).exists()`. Fail-open documenté (D6).
- `pub fn stale_env_knob(global_home: &Path, agent_home: &Path) -> Option<StaleKnob>` — la
  garde D4 : `parse_dotenv` sur les deux homes, renvoie `Some` avec le chemin du fichier
  et la valeur lue quand `MIKA_DEV_AUTO_PULL` y vaut `0`. Per-agent prioritaire sur le
  global, dans l'ordre de `Settings::load_for_agent`.
- Doc de module portant : pourquoi un fichier et pas l'env (E2/E3), pourquoi le contenu
  n'est pas lu (D1), pourquoi fail-open (D6).

### V2 — `crates/mika-agent/src/task_engine/dispatcher.rs`

- Champ `pub global_home_dir: PathBuf` sur `TaskDispatcher`, avec doc renvoyant à E6
  (« per-agent `home_dir` ne convient pas ; le précédent des fichiers d'état est global »).
- Champ `auto_pull_stop_armed: AtomicBool` pour la détection de transition (D5).
- En **tête** de `dispatch_auto_pull_groomed`, avant `resolve_periodic_scan_token` (E5) :
  court-circuit `is_stopped` → `info!` + transition éventuelle en audit → `Ok(())`.
- Garde D4 exécutée sur le chemin **non** court-circuité (quand le tick tourne
  réellement) — c'est la population du prédicat (D4).

### V3 — `crates/mika-agent/src/server/mod.rs`

Une ligne : `global_home_dir: global_home.to_path_buf(),` dans la construction du
`TaskDispatcher` (ligne ~551). `global_home` est déjà le paramètre d'`init_agent` (E6).
Les constructions de test (`mod.rs:1845` et voisines) prennent le même `/tmp/mika-test`
que `home_dir`.

### V4 — Tests

`crates/mika-agent/src/auto_pull_stop.rs` (inline, `#[cfg(test)]`) pour T1/T2/T4/T5/T7 ;
`crates/mika-agent/tests/eval/` pour T3 si le harnais permet d'observer la row sans
réseau, sinon assertion DB directe.

### V5 — Documentation

- `CLAUDE.md` — section « Optional (STOP global à chaud — mika#2329) » : le geste exact
  (`touch` / `rm`), la latence ≤ 10 min (E7), la distinction explicite des deux
  mécanismes (D3), les greps opérateur, et l'avertissement que `MIKA_DEV_AUTO_PULL` dans
  le `.env` **ne coupe rien à chaud** (E2) — c'est la phrase qui ferme le piège dans la
  doc comme D4 le ferme dans le log.
- `docs/runtime-structure.md` — le fichier sous `~/.mika/state/`.

---

## Verification contract

### T1 — Le fichier présent court-circuite, et court-circuite tôt

`is_stopped` rend `true` sur fichier présent (vide comme non vide, D1). Test
d'intégration : `dispatch_auto_pull_groomed` rend `Ok(())` **sans** résoudre de token et
sans appel `gh` — vérifié par l'absence de token configuré dans le harnais (un chemin non
court-circuité échouerait à résoudre et émettrait `auto_pull_no_token`).

### T2 — Le fichier absent laisse le chemin nominal strictement inchangé

`is_stopped` rend `false`, aucune ligne INFO/WARN nouvelle, le dispatch se déroule comme
avant. Couvre le régime nominal, qui est l'écrasante majorité des ticks.

### T3 — La réversibilité ne touche aucune row (l'AC central)

Armer puis lever : la row `auto_pull_groomed` est `recurring_active` **avant, pendant et
après**, son `status` et son `metadata` inchangés. Assertion explicite qu'aucun marqueur
`config_cancel_reverted` n'a été écrit. C'est le test qui vaut preuve de D2 et qui
distingue ce correctif du mécanisme d'E4.

### T4 — La garde D4 émet quand le `.env` disque contredit le process

`.env` sur disque portant `MIKA_DEV_AUTO_PULL=0`, environnement du process ne le portant
pas → `stale_env_knob` rend `Some`, WARN émis. Les deux homes testés (global, per-agent).

### T5 — La garde D4 se tait dans tous les autres cas

`.env` absent, `.env` sans la clé, `.env` avec `MIKA_DEV_AUTO_PULL=1` → `None`, aucun
WARN. Un avertissement qui crie en régime nominal est un avertissement qu'on filtre.

### T6 — L'audit écrit une transition, pas un tick

Trois ticks consécutifs STOP armé → **une** ligne `audit_events`, trois lignes INFO.
Levée puis ré-armement → deux lignes d'audit de plus. Pin de D5.

### T7 — Garde structurelle : un seul lecteur

Scan de source refusant le littéral `auto-pull-stop` hors de `auto_pull_stop.rs`
(module + tests exclus). Pin de D9.

### T8 — `cargo build`, `clippy -D warnings`, suite verte

---

## Fire-Disposition

- **T1 rouge (le court-circuit ne prend pas)** → le placement est faux : vérifier qu'il
  est bien avant `resolve_periodic_scan_token` et non dans `auto_pull_groomed_ticket`.
- **T3 rouge (une row a bougé)** → **halte**. Le correctif est retombé dans la classe
  d'E4 et hérite de la dette #1742/#2271. Ne pas corriger en ajoutant une exemption :
  retirer l'écriture.
- **T4 rouge alors que le fichier disque porte bien `0`** → `parse_dotenv` ne lit pas le
  home attendu. Vérifier global vs per-agent (E6) avant de toucher au prédicat.
- **T5 rouge (WARN en régime nominal)** → le prédicat lit l'environnement du process au
  lieu du fichier ; il ré-introduirait E2 à l'envers.
- **T7 rouge** → une copie du chemin existe. La supprimer, ne pas élargir l'exclusion du
  scan.

---

## Definition of Done

1. Un fichier sentinelle armé coupe les trois phases d'`auto_pull` au tick suivant, sans
   redémarrage de mika-spirit.
2. Le retirer rétablit le feeder au tick suivant, sans redémarrage et sans qu'aucune row
   récurrente ait été modifiée.
3. Chaque tick court-circuité écrit une ligne INFO nommée ; chaque transition écrit une
   ligne `audit_events`.
4. Un `MIKA_DEV_AUTO_PULL=0` posé dans un `.env` après le démarrage produit un WARN
   nommé disant que ce geste ne coupe rien et lequel coupe.
5. La branche boot-time `MIKA_DEV_AUTO_PULL` est inchangée.
6. `cargo build` + `cargo clippy -D warnings` + suite de tests verts.
7. `CLAUDE.md` et `docs/runtime-structure.md` documentent le geste, la latence et la
   distinction des deux mécanismes.

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` formelle ; les critères
ci-dessous sont dérivés de son § « Exigence » et de sa demande de visibilité.

- **AC1** — Un STOP global de `auto_pull` prend effet **au tick suivant** (≤ 10 min,
  E7) sans redémarrage de mika-spirit. Vérifié par T1.
- **AC2** — Le STOP est **réversible de la même manière** : le geste inverse rétablit le
  feeder au tick suivant, sans redémarrage. Vérifié par T3.
- **AC3** — La réversibilité ne dépend d'aucune résurrection de row : aucune row
  récurrente n'est annulée, marquée ou recréée par l'armement ou la levée. Vérifié par
  T3 (assertion explicite sur `status` et `metadata`).
- **AC4** — Un `info!` **nommé** est émis à chaque tick court-circuité, de sorte que le
  STOP soit visible dans le log et pas seulement effectif. Vérifié par T1 et T6.
- **AC5** — Une ligne `audit_events` est écrite à chaque **transition** (armement,
  levée), et non à chaque tick. Vérifié par T6.
- **AC6** — Un `MIKA_DEV_AUTO_PULL=0` posé sur disque après le démarrage — le geste que
  l'opérateur de #2313 a posé — produit un WARN nommé plutôt que le silence. Vérifié par
  T4 ; l'absence de faux positif par T5.
- **AC7** — Le chemin nominal (STOP non armé) est strictement inchangé : aucune ligne
  nouvelle, aucun appel supplémentaire. Vérifié par T2.
- **AC8** — `cargo build`, `cargo clippy -D warnings` et la suite de tests passent.
  Vérifié par T8.

---

## Surfaces opérateur et sonde post-déploiement

**Le geste :**

```bash
touch ~/.mika/state/auto-pull-stop     # STOP  — effectif au tick suivant (≤ 10 min)
rm    ~/.mika/state/auto-pull-stop     # REPRISE — idem
```

Le répertoire `state/` existe déjà sur toute installation ayant dispatché (mika#2026,
`pilot-gitconfig`) ; sur une installation neuve, `mkdir -p` d'abord.

**Journal** (`$MIKA_SPIRIT_LOG_FILE`) :

- `auto_pull_stop_armed` (INFO, une ligne par tick court-circuité). **C'est la
  confirmation que le STOP mord** : poser le fichier et ne pas voir cette ligne dans les
  10 minutes signifie que le chemin n'est pas celui que le process lit — vérifier
  `MIKA_HOME`.
- `auto_pull_stop_stale_env_knob` (WARN). **Régime attendu : zéro ligne.** Toute
  occurrence est un opérateur qui croit avoir coupé la boucle et ne l'a pas fait. La
  ligne nomme le fichier et la valeur lue.

**SQL :**

```sql
SELECT created_at, after_value FROM audit_events
 WHERE tool_name = 'auto_pull_stop' ORDER BY created_at DESC;
```

Une ligne par transition — c'est l'historique des STOP, donc la réponse directe à
« la boucle a-t-elle été arrêtée pendant cet incident, et de quand à quand ? ».

**Sonde post-déploiement, avec sa halte.** Sur la première occasion réelle : armer,
vérifier la ligne INFO au tick suivant, vérifier qu'aucun ticket n'est promu `ready`
pendant la fenêtre (`tool_name = 'auto_feeder'` silencieux), lever, vérifier la reprise.
Si le feeder **ne reprend pas** après la levée : **halte, et ne pas redémarrer pour
réparer** — un redémarrage effacerait la preuve. C'est la signature d'E4 (une row a été
touchée quelque part) et elle se lit dans `tasks` sur la row `auto_pull_groomed`, pas
dans le log.

---

## Hors périmètre (suivi à ouvrir)

- **Les deux scans jumeaux** (E9, D7) : `MIKA_DEV_WIP_RESCUE` et
  `MIKA_QA_REVIEW_RECONCILE` ont le même défaut boot-time. Le lecteur est paramétré pour
  les accueillir en une ligne chacun ; la décision d'arrêter la revue QA n'est pas celle
  d'arrêter le feeder, et mérite d'être prise pour elle-même. **Ticket de suivi.**
- **La commande `mika dev auto-pull {pause,resume}`** (D8) : possible par-dessus ce
  mécanisme, non requise par l'exigence du ticket.
- **Un STOP à effet immédiat** (< 10 min) : hors exigence (E7), et le geste existe déjà
  (arrêt du service).
- **Le rechargement à chaud des variables `MIKA_*` en général** : délibérément non
  ouvert (E3). Ce plan ajoute un objet hot, il ne rend hot aucune variable existante.
- **La cause des incidents qui motivent un STOP** (#2313 et suivants) : ce travail rend
  l'arrêt possible, il ne rend rien plus sain.
