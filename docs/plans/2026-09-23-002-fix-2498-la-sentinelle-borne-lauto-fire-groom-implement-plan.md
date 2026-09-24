# mika#2498 — La sentinelle borne l'auto-fire groom→implement

> **Ticket :** senara-solutions/mika#2498 — enfant de l'umbrella #2491 (Défaut 6).
> **Type :** fix, substrat de boucle. 1 PR atomique.
> **Aucune variable d'environnement n'est créée.** Aucun fichier sentinelle n'est ajouté.

---

## 1. Le défaut, mesuré

Le 2026-09-23, l'opérateur pose `~/.mika/state/auto-pull-stop` à **05:38** locales.
Le tick `auto_pull` se court-circuite comme prévu — plus aucune promotion, plus aucun
dispatch par la voie du feeder. À **06:11:39Z**, un implement (`ff694ec0`) part quand
même : le groom de #2492 (callback `3667d844`) a convergé, et le moteur a enchaîné
directement sur son implémentation. 103 tours, 17,86 USD, `resume_agent` parent
`107f8849`. L'opérateur n'a pu que l'annuler **en réaction**.

Le STOP n'a donc pas arrêté ce que l'opérateur croyait arrêter. C'est la forme de
panne que la doc de `auto_pull_stop.rs` nomme elle-même pour une autre raison :
*« un STOP silencieusement inopérant se lit exactement comme un STOP qui marche »*.

---

## 2. Ce que la lecture du code déplace dans le ticket

Deux rectifications, et chacune change le remède.

### R1 — La route mesurée est l'auto-fire moteur, pas un tour LLM

Le ticket parle du « callback `resume_agent` ». Il y a en réalité **deux** routes
possibles après un groom convergé, et elles ne se ferment pas au même endroit :

| # | Route | Site | Couverte par ce ticket ? |
|---|---|---|---|
| 1 | **Auto-fire moteur** — `try_dispatch_pilot_after_groom_success` (`task_engine/dispatcher.rs`, mika#1614) : spawn direct du subprocess depuis `handle_task_complete`, sans webhook, sans tour LLM | `dispatcher.rs:3829` | **oui** |
| 2 | **Prompt-level** — `self-dev-callback` étape 25b : `run_gh("issue edit <n> --add-label ready")` → webhook `labeled` → `ready_label_handler` | `skills/bundled/self-dev-callback/system_prompt.md:25` | **non** (§ 8) |

La route 1 est celle que l'évidence nomme (spawn direct, pas de round-trip `gh`).
La route 2 est, dans la population nominale, **inerte** : le ticket porte déjà
`ready` (c'est ainsi que le groom a été dispatché), `dispatch-lib.sh` ne le retire
jamais — vérifié, aucun `--remove-label ready` dans le fichier — et GitHub n'émet
`labeled` que sur une **transition** (mika#2323). Un `--add-label` redondant
n'émet rien. Elle cesse d'être inerte pour un groom lancé par
`mika ask "groom mika issue#N"` (mika#2484), où le ticket peut ne pas porter
`ready` : c'est la population nommée non couverte du § 8.

**Je n'ai pas pu attribuer l'occurrence mesurée à l'une des deux routes** : la base
de production n'est pas joignable depuis le bac à sable de dispatch
(`~/.mika/data/mika.db` absent). L'attribution est donc portée par la **sonde S2**
du § 10, dont c'est précisément l'objet, et non affirmée ici.

### R2 — La sentinelle n'est pas « l'interrupteur d'auto_pull », c'est le frein de dispatch de la boucle

`auto_pull_stop` est livré **paramétré par nom de scan** (mika#2329), et étendu une
fois (`worktree-reap`, mika#2420) sur le critère écrit : *une décision distincte
mérite un fichier distinct*. La question est donc : « ne plus démarrer d'implement »
est-elle une décision **distincte** de « ne plus alimenter la file » ?

Non. Les deux ont la **même sortie** — un dispatch dev-pilot neuf — atteinte par deux
routes. Phase 0/1 promeut `ready`, Phase 2 dispatche in-process (mika#2470), et
l'auto-fire dispatche. Un opérateur qui arrête l'une et pas l'autre n'a rien arrêté :
c'est exactement l'incident. La sentinelle est donc élargie **en sens documenté**, pas
dupliquée.

---

## 3. Ce qui est livré

### U1 — La garde

`try_dispatch_pilot_after_groom_success` prend un paramètre `global_home: &Path` et,
**après avoir établi qu'un dispatch aurait lieu** et **avant toute écriture**, rend la
main quand `auto_pull_stop::is_stopped(global_home, auto_pull_stop::AUTO_PULL_SCAN)`.

Le chemin du fichier n'est **jamais recomposé** : l'appel passe par `is_stopped`, et
la garde structurelle `mika2329_le_chemin_du_fichier_sentinelle_a_un_seul_lecteur`
couvre le nouveau lecteur gratuitement. **Contrainte de rédaction** : cette garde est
littérale et scanne aussi les commentaires — le doc-comment doit désigner la sentinelle
par sa constante (`auto_pull_stop::AUTO_PULL_SCAN`), jamais par son nom de fichier.
Précédent exact et copiable : `worktree_reaper.rs:78`.

### U2 — Le site d'appel

`handle_task_complete` (`dispatcher.rs:1063`) passe `&self.global_home_dir`. Le champ
existe déjà sur `TaskDispatcher` (`dispatcher.rs:482`) et est déjà lu par le
court-circuit d'`auto_pull` (`dispatcher.rs:1660`) — aucune plomberie nouvelle.

### U3 — Les surfaces opérateur

- **Journal** : `groom_pilot_autofire_stopped` (INFO), champs `parent_task_id`,
  `callback_task_id`, `repo`, `issue`, `stop_file` (rendu par `stop_file_path`, jamais
  un littéral).
- **`audit_events`** : **même `tool_name`** que le chemin nominal
  (`task_engine_groom_pilot_dispatcher`), `before_value = "groom_delivered"`,
  `after_value = "stopped_by_sentinel"`.

**Pourquoi le même `tool_name` et pas un second nom.** La doctrine maison (mika#2156,
#2184, #2205, #2368) impose deux noms quand deux populations doivent rester
soustractibles **et que chaque nom porte sa propre cause**. Ici les deux issues
appartiennent au **même dispatcheur**, sur la **même population** (les callbacks de
groom qui ont convergé) : `after_value` porte déjà l'issue (`implement_dispatched`), et
l'y ajouter garde le nom vrai. Un `GROUP BY after_value` rend alors les deux comptes
d'une requête, soustractibles. Précédent exact : `ready_label_outcome` (mika#2323),
un `tool_name`, la porte dans `after_value`.

**Coût nommé et daté :** un `SELECT count(*) WHERE tool_name =
'task_engine_groom_pilot_dispatcher'` nu change de sens au déploiement.
`WHERE after_value = 'implement_dispatched'` reste **exact de part et d'autre**, et
c'est la requête à écrire.

### U4 — Le vocabulaire d'`after_value` devient un format de fil

Trois constantes à site unique (`GROOM_PILOT_DISPATCHER_TOOL`,
`GROOM_PILOT_DISPATCHED_VALUE`, `GROOM_PILOT_STOPPED_VALUE`), épinglées par un test.
Deux orthographes d'une même issue couperaient une population en deux sans le dire —
la leçon que mika#2323 a dû engraver (`mika2323_gate_names_are_a_wire_format`).

### U5 — Documentation

- `crates/mika-agent/src/auto_pull_stop.rs` § *Portée* : la sentinelle
  `AUTO_PULL_SCAN` est le **frein de dispatch neuf de la boucle autonome**, pas
  l'interrupteur privé d'un scan. Le critère pour un futur consommateur est écrit :
  *démarre-t-il du travail pilote neuf ?* Si oui, il lit cette sentinelle ; si non,
  il lui faut son propre nom de scan (le critère de mika#2420).
- `crates/mika-agent/CLAUDE.md` § *auto-fire post-grooming* et § *Hot STOP*.
- `CLAUDE.md` racine § *Optional (STOP global à chaud — mika#2329)* : la portée, les
  deux surfaces, les sondes et leurs haltes.

---

## 4. Le placement, et pourquoi exactement là

La fonction est une suite de cinq étapes. Le placement est **après l'étape 3** (URL
d'issue parsée) et **avant l'étape 4** (jeton GitHub).

**Pourquoi pas en tête (avant l'étape 1).** Les étapes 1–3 sont le **prédicat** qui
décide qu'un dispatch aura lieu : classe `groom`, marqueur `Outcome: PLAN_GROOMED`,
parent avec URL d'issue parsable. En tête, la garde tournerait sur **chaque** livraison
de callback, et sa ligne signifierait « un callback est arrivé pendant un STOP » — un
fait sans conduite associée, émis des dizaines de fois par jour. Après l'étape 3, la
ligne signifie **« le STOP a refusé un dispatch »**, qui est la seule lecture
actionnable. C'est aussi ce que le court-circuit d'`auto_pull` fait déjà, mutatis
mutandis : il est en tête **de son dispatch**, parce que là-bas la fonction *entière*
est le dispatch.

**Pourquoi avant l'étape 4 (le jeton).** Les deux peuvent tenir en même temps. Le STOP
est une **décision de l'opérateur**, l'absence de jeton est un **fait d'environnement** :
l'opérateur doit voir la ligne du geste qu'il a posé, y compris sur un hôte sans jeton.

**Pourquoi impérativement avant l'étape 5c.** 5c écrit en base : il bascule le
`dispatch_class` du parent `groom` → `implement` *avant* le contrôle de readiness, pour
que la garde de slot par classe (#1001) porte sur le bon slot. Une garde placée après
laisserait un parent étiqueté `implement` sans aucun implement en vol, et le prochain
dispatch légitime serait compté sur le mauvais slot. **Le test T5 est celui qui attrape
cette erreur de placement**, et il n'existe que pour ça.

---

## 5. Ce que le refus ne fait PAS — et pourquoi il est convergent

Le refus **ne touche rien** : il n'annule pas le groom, ne touche pas au plan (déjà
committé et poussé par `_push_branch`), ne retire pas `ready`, ne marque rien `failed`,
ne crée aucune row.

**État laissé.** Le parent reste `in_progress`, `dispatch_class = "groom"`. Il sera
fauché `failed` par `reap_orphaned_parent_tasks` (#871) à +10 min : parent
`in_progress` / `self_dev` / `manual`, enfant callback `delivered` plus vieux que
`REAPER_GRACE_SECONDS`, pas de `pr_url` sur le parent (un callback de groom n'en porte
jamais). **Ce n'est pas introduit ici** : c'est exactement ce que produisent déjà les
cinq chemins de saut existants de cette fonction (pas de jeton, outil absent du
registre, handler non long-running, flip impossible, readiness refusée). La garde ajoute
une sixième raison de refus dans un sillon déjà creusé — c'est ce qui garde le
changement petit.

**Récupération après la levée.** Le ticket porte toujours ses trois callouts de
grooming et (nominalement) `ready` ; le parent est terminal donc plus `in_flight` ;
aucun label de rétention n'est posé. Le réconciliateur stuck-ready (Phase 2) le
re-drive, et depuis mika#2470 il dispatche **in-process** : `groomed_state` rend
`Groomed` (callouts + preuve en base, mika#2484) et route vers `dev-pilot`. Pendant le
STOP, aucun tick ne tourne, donc **aucun point du budget de re-drive n'est consommé**
(mika#2020). *Le refus est convergent, pas terminal* — et c'est la propriété qui rend
le refus acceptable plutôt que destructeur.

**Effet de bord utile, et il répond à la moitié « groom-only » du ticket.** Sous STOP :
le tick `auto_pull` se court-circuite (aucun groom neuf), les grooms en vol convergent,
committent leur plan, et **ne chaînent pas**. C'est littéralement la « pause
grooms-seuls » que le ticket décrit — obtenue sans introduire d'état nouveau.

---

## 6. Alternatives refusées

1. **Un second fichier sentinelle (`groom-implement-stop`).** Refusé : il scinde une
   décision d'opérateur en deux gestes. Un opérateur qui pose le fichier de l'incident
   — celui de sa mémoire musculaire — obtiendrait exactement le comportement
   d'aujourd'hui en croyant avoir arrêté la boucle. C'est la panne que ce ticket ferme,
   réintroduite sous un autre nom.

2. **Un état « groom-only » (option B du ticket), comme mode explicite.** Refusé
   *comme mode* : c'est un état produit nouveau (les grooms tournent, les implements
   non) sans demande mesurée au-delà de cet incident, et l'intention mesurée était une
   pause pleine — *« annulable seulement en réaction »* dit que l'opérateur voulait que
   ça ne parte pas. Le § 5 montre que la moitié utile du mode est livrée sans lui.

3. **Une garde dans `validate_dispatch_readiness`** (le point de confluence de toutes
   les routes). Refusé : quatre appelants de production — `ready_label_handler`, la
   frontière d'outil, cet auto-fire, `verdict_handler` — donc elle refuserait aussi un
   `/mika` d'opérateur et une réparation CI de PR ouverte. **Et le scopage est
   impossible** : `tasks.dispatcher_source` (mika#1948) résout `NULL → mika_dev`, et
   les dispatches d'opérateur n'écrivent pas la colonne — le scopage enfermerait
   l'opérateur dehors, soit l'inverse d'un frein que l'opérateur pose.

4. **Renommer la sentinelle** pour que son nom dise sa portée élargie. Refusé : le
   fichier peut être armé **en ce moment** sur l'hôte, et la mémoire musculaire d'un
   opérateur est ce qu'un frein de P0 ne doit pas casser. Ce qui change est la portée
   **documentée** (U5), pas le geste.

5. **Une variable d'environnement.** Refusé par mika#2329, qui porte l'argument complet :
   lue une fois au démarrage, non mutable de l'extérieur sur un process vivant, et en
   rendre **une** relue à chaud créerait une exception invisible entre deux variables
   que rien ne distingue.

6. **Annuler le groom en vol.** Hors périmètre et faux : le plan est du travail déjà
   payé, committé et poussé.

7. **Une clause de prompt dans `self-dev-callback`.** Refusé par
   `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` — c'est du
   substrat de boucle.

---

## 7. Fichiers touchés

| Fichier | Nature |
|---|---|
| `crates/mika-agent/src/task_engine/dispatcher.rs` | garde + paramètre + site d'appel + 3 constantes + tests T1–T5 ; **5 tests existants** de l'auto-fire prennent le nouveau paramètre |
| `crates/mika-agent/src/canonical_tokens.rs` | scan SOLE WRITER (T6) + test de format de fil (T7) |
| `crates/mika-agent/src/auto_pull_stop.rs` | doc de module § *Portée* (U5) |
| `crates/mika-agent/CLAUDE.md` | § auto-fire post-grooming, § Hot STOP |
| `CLAUDE.md` (racine) | § mika#2329 : portée, surfaces, sondes, haltes |

Aucune migration, aucune colonne, aucun `.env.example`.

---

## 8. Périmètre non couvert — nommé, avec sa raison

1. **Route 2, `run_gh --add-label ready`** depuis le tour silencieux du callback
   (§ R1). Inerte sur la population nominale (label déjà présent ⇒ pas de transition
   ⇒ pas d'événement, mika#2323) ; réelle pour un groom lancé par `mika ask "groom …"`.
   **Ticket de suivi, précondition = la sonde S2 la montre.**

2. **Un `ready` posé à la main pendant un STOP** → `ready_label_handler` dispatche.
   Délibérément non couvert, et **le discriminant qui permettrait de le scoper
   étroitement est structurellement interdit** : depuis mika#2323 l'acteur (`Labeled
   by: @<login>`) est lisible mais **aucun prédicat de refus n'a le droit de le lire**,
   tenu par `mika2323_no_gate_predicate_reads_the_actor` à allowlist livrée vide. Une
   garde là-bas est donc forcément la décision large — savoir si un fichier STOP prime
   sur le label que l'opérateur vient de poser est une décision produit, pas substrat.
   **Ticket de suivi.**

3. **`verdict_handler`** (`block[ac]` / `block[ci]`) : réparer une PR ouverte est une
   **continuation**, pas du travail neuf. Décision distincte, inchangée.

4. **`wip_rescue`, `qa_review_reconcile`** : inchangés, toujours sans interrupteur —
   règle YAGNI de mika#2329, rien de détruit à les laisser tourner.

---

## 9. Surfaces opérateur

```bash
# La garde a-t-elle mordu ? (régime attendu : non vide pendant un STOP, vide sinon)
grep groom_pilot_autofire_stopped "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{parent_task_id, callback_task_id, repo, issue, stop_file}'

# Le STOP est-il armé, et depuis quand ? (surfaces mika#2329, inchangées)
grep auto_pull_stop_armed "$MIKA_SPIRIT_LOG_FILE" | tail -1
```

```sql
-- Les deux issues du même dispatcheur, soustractibles en une requête
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'task_engine_groom_pilot_dispatcher' GROUP BY 1;

-- L'historique des STOP (mika#2329, une ligne par transition)
SELECT created_at, after_value FROM audit_events
 WHERE tool_name = 'auto_pull_stop' ORDER BY created_at DESC;
```

| événement | niveau | régime attendu | lecture |
|---|---|---|---|
| `groom_pilot_autofire_stopped` | INFO | **non vide pendant un STOP, vide hors STOP** | chaque ligne est un implement que l'opérateur n'a pas eu à annuler en réaction |
| `after_value = 'stopped_by_sentinel'` | audit | idem | le même fait, comptable et daté |
| `after_value = 'implement_dispatched'` | audit | **zéro pendant un STOP** | toute occurrence dans une fenêtre de STOP est une fuite — sonde S2 |

---

## 10. Sondes post-déploiement, et leurs quatre haltes

**S1 — la garde mord.** Armer la sentinelle pendant qu'un groom est en vol, le laisser
converger. Attendu : la ligne INFO apparaît, aucune row callback
`long_running:run_claude_pilot` n'est créée sous ce parent, le `dispatch_class` du
parent est toujours `groom`, et le compte `implement_dispatched` n'a pas bougé.

**S2 — attribution des fuites (48 h, et c'est elle qui décide des suivis du § 8).**
Sur la fenêtre d'un STOP, croiser les dispatches réellement partis :

```bash
grep ready_label_outcome "$MIKA_SPIRIT_LOG_FILE" | jq -c 'select(.gate == "dispatched")'
```

Zéro ligne dans la fenêtre ⇒ les routes 2 et « label à la main » n'ont rien produit,
et les suivis du § 8 restent fermés. Des lignes ⇒ le suivi s'ouvre **avec un compte**,
et le `repo`/`num` dit laquelle des deux.

**Halte 1 — la ligne apparaît ET un pilote a tourné quand même.** Une **autre** route a
dispatché. **Ne pas élargir cette garde par réflexe** : lire `ready_label_outcome`
(S2) et établir laquelle des trois routes du § 8 a servi ; les trois remèdes diffèrent.

**Halte 2 — aucune ligne, et aucun implement.** On ne peut **rien** conclure : vérifier
d'abord qu'un groom a réellement convergé dans la fenêtre
(`grep 'Outcome: PLAN_GROOMED'`, ou un `callback` `delivered` portant le marqueur).
*Une garde que personne n'a exercée se lit exactement comme une garde qui marche*
(classe mika#2205). Vérifier ensuite que le binaire servi porte le correctif
(classe mika#2340) **avant** toute conclusion sur le prédicat.

**Halte 3 — la ligne apparaît alors qu'aucun STOP n'est posé.** Le lecteur regarde
ailleurs que là où l'opérateur écrit : comparer le champ `stop_file` de la ligne avec
le chemin réellement posé, et vérifier `MIKA_HOME` / `global_home_dir` **avant** de
toucher au prédicat. Le fail-open de `Path::exists` ne peut produire que l'inverse
(un STOP non vu), jamais un faux positif — donc une ligne de trop est une erreur de
**chemin**, pas de prédicat.

**Halte 4 — après la levée, le ticket n'est jamais dispatché.** La prémisse de
convergence du § 5 est fausse. Lire le registre d'exclusion **avant** d'ajouter le
moindre mécanisme de ré-armement ici :

```sql
SELECT after_value, created_at FROM audit_events
 WHERE tool_name = 'auto_pull_exclusion' AND target_key = 'issue:<n>'
 ORDER BY created_at DESC LIMIT 5;
```

Il nomme le vrai filtre (`not_groomed`, `below_threshold`, `operator_review_or_blocked`,
`abandoned_operator_held`, …), et aucun de ces remèdes ne vit dans cette fonction.

---

## 11. Contrat de vérification (tests)

Tous au site de production (`try_dispatch_pilot_after_groom_success` appelée
directement), sur le harnais existant `create_groom_callback_pair` +
`dispatch_class_of` (`dispatcher.rs:6873`), avec un `tempfile::tempdir()` comme
`global_home`. Armer = écrire le fichier via `auto_pull_stop::stop_file_path` (jamais
un littéral — la garde mika#2329 scanne aussi les tests).

| # | Test | Ce qu'il atteste |
|---|---|---|
| **T1** | sentinelle armée, callback `PLAN_GROOMED` → aucune row callback créée sous le parent, row d'audit `stopped_by_sentinel` présente, `implement_dispatched` absente | l'AC principale |
| **T2** | **contrôle négatif** : fixture identique, sentinelle **absente** → aucune row `stopped_by_sentinel`, et le chemin de saut préexistant est atteint (registre vide ⇒ `run_claude_pilot` introuvable) | sans lui, T1 passerait sur une fonction qui rend la main inconditionnellement |
| **T3** | `worktree-reap` armé, `auto-pull` absent → l'auto-fire n'est pas arrêté | les scans ne se coupent pas l'un l'autre — miroir de `mika2329_le_chemin_est_parametre_par_le_scan` |
| **T4** | sentinelle armée, callback `PLAN_ITERATE` → **aucune** row `stopped_by_sentinel` | la ligne signifie « un dispatch a été refusé », pas « un callback est arrivé pendant un STOP » — épingle le placement du § 4 |
| **T5** | sentinelle armée → `dispatch_class` du parent vaut toujours `groom` après l'appel | **attrape une garde placée après l'étape 5c** ; c'est sa seule raison d'être |
| **T6** | scan de source : un seul écrivain de production du nom de journal `groom_pilot_autofire_stopped`, **allowlist livrée vide** | aucun test comportemental ne peut voir un second écrivain : il ne rendrait aucune décision fausse, il rendrait seulement les deux populations non soustractibles |
| **T7** | les trois constantes valent exactement `task_engine_groom_pilot_dispatcher`, `implement_dispatched`, `stopped_by_sentinel` | format de fil — deux orthographes couperaient une population en silence (mika#2323) |

**Non-régression à constater, pas à écrire :**
`mika2329_le_chemin_du_fichier_sentinelle_a_un_seul_lecteur` doit rester vert — le
nouveau lecteur passe par `is_stopped` et n'écrit aucun littéral, y compris dans son
doc-comment.

**Anti-vacuité de T6**, sur le modèle de `mika2242_the_two_audit_names_have_a_single_writer` :
le propriétaire attendu (`dispatcher.rs`) doit être **trouvé** par le scan, sinon la
garde est décorative et se lit comme une garde propre.

---

## Fire-Disposition

Ce plan livre des détecteurs : sept tests, dont un **scan de source** (T6) et un test
de **format de fil** (T7).

**Option retenue : (a) — exception nommée en allowlist, allowlist livrée VIDE.**

Détail d'implémentation :

- T6 porte une constante `GROOM_PILOT_STOPPED_SOLE_WRITER_EXCEPTIONS: &[&str] = &[]`,
  avec le doc-comment qui dit la conduite : **quand ce scan tire, on retire le second
  écrivain, on ne l'excepte pas** (doctrine mika#2201 : *« on déclare, on
  n'allowliste pas »*). Une exception rendrait le `GROUP BY` opérateur du § 9
  silencieusement faux — strictement pire que le silence qu'il remplace.
- **Rien à excepter à la livraison, et c'est vérifiable** : le nom de journal
  `groom_pilot_autofire_stopped` et la valeur `stopped_by_sentinel` sont **neufs**, donc
  aucune violation préexistante ne peut exister. L'allowlist est vide parce qu'elle
  n'a rien à contenir, pas par optimisme.
- **Assertion auto-nettoyante** : un test assert que l'allowlist est vide
  (`mika2498_the_sole_writer_allowlist_is_empty`, forme de
  `mika2242_the_sole_writer_allowlist_is_empty`). Le jour où quelqu'un y dépose une
  entrée, c'est ce test qui rougit, et non un `GROUP BY` qui ment des mois plus tard.
- **Aucun détecteur n'est livré désarmé.** T1–T5 sont comportementaux sur fixtures
  synthétiques et auto-contenues (`test_db()` + `tempdir()`), sans réseau, sans base de
  production : rien ne justifie un `#[ignore]`, et un détecteur désarmé se lirait comme
  un détecteur vert (classe mika#2205).

---

## Definition of Done

- [ ] Sous `auto_pull_stop::AUTO_PULL_SCAN` armé, un groom convergé (`Outcome:
      PLAN_GROOMED`) ne déclenche **aucun** dispatch implement par l'auto-fire moteur.
- [ ] Le refus est **observable** : une ligne INFO nommée et une row `audit_events`
      portant `after_value = 'stopped_by_sentinel'`.
- [ ] Le refus n'écrit **rien d'autre** : le `dispatch_class` du parent est inchangé,
      aucune row callback n'est créée, aucun subprocess n'est lancé.
- [ ] Sentinelle absente : comportement **byte-identique** à aujourd'hui.
- [ ] Les sept tests du § 11 passent ; `mika2329_le_chemin_du_fichier_sentinelle_a_un_seul_lecteur`
      reste vert.
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt` propres.
- [ ] Portée élargie documentée aux trois endroits du § U5, avec le critère pour un
      futur consommateur.
- [ ] Le périmètre non couvert du § 8 est écrit dans le corps de PR, avec la sonde S2
      comme précondition de ses suivis.

---

## Acceptance criteria

*(le corps du ticket ne porte pas de section `## Acceptance criteria` — les critères
ci-dessous sont dérivés du DoD du ticket et du § 11.)*

- **AC1 — la garde mord.** Sentinelle armée, callback de groom portant `Outcome:
  PLAN_GROOMED`, parent avec URL d'issue parsable : aucune tâche callback
  `long_running:run_claude_pilot` n'est créée sous ce parent, et aucun subprocess n'est
  lancé. *(T1)*
- **AC2 — la garde ne mord que là.** Sentinelle absente, fixture identique : la
  fonction poursuit au-delà du contrôle et atteint son chemin de saut préexistant ;
  aucune row `stopped_by_sentinel` n'est écrite. *(T2)*
- **AC3 — les scans restent indépendants.** La sentinelle `worktree-reap` armée seule
  n'arrête pas l'auto-fire. *(T3)*
- **AC4 — l'événement signifie « un dispatch a été refusé ».** Un callback de groom
  **non convergé** (`PLAN_ITERATE`) sous sentinelle armée n'écrit aucune row
  `stopped_by_sentinel`. *(T4)*
- **AC5 — aucune mutation d'état sur le refus.** Après un refus, le `dispatch_class`
  du parent vaut toujours `groom`. *(T5)*
- **AC6 — le refus est attribuable.** Une row `audit_events` avec
  `tool_name = 'task_engine_groom_pilot_dispatcher'` et
  `after_value = 'stopped_by_sentinel'`, plus une ligne INFO `groom_pilot_autofire_stopped`
  nommant le parent, le callback, le ticket et le fichier sentinelle consulté. *(T1)*
- **AC7 — écrivain unique et format de fil.** Un seul site de production écrit le nom
  de journal ; les trois valeurs d'`after_value`/`tool_name` sont épinglées ;
  l'allowlist du scan est vide et un test l'assert. *(T6, T7)*
- **AC8 — pas de littéral de chemin.** Aucune occurrence du nom de fichier sentinelle
  hors `auto_pull_stop.rs`, commentaires et tests compris —
  `mika2329_le_chemin_du_fichier_sentinelle_a_un_seul_lecteur` reste vert.
- **AC9 — le non-couvert est écrit.** Les quatre routes du § 8 sont nommées dans le
  corps de PR avec leur raison, et la sonde S2 est posée comme précondition de leurs
  tickets de suivi.
