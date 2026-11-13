# fix(3047) vol 1 — la phase 2 relit le ticket à la forge avant de dispatcher, et le handler décide sur cette lecture

Ticket : senara-solutions/mika#3047
Mesure sur : `c8ef105f6`

---

## 0. Le défaut, et le fil exact qui l'a retenu

**La phase 2 décide sur une copie, et agit sur la forge.** Un tick lit la liste
des tickets ouverts **une fois** (l. 4775), puis les phases 0 à 2 la partagent.
Entre cette lecture et le dispatch en processus de la phase 2 (l. 6092) il peut
s'écouler plusieurs secondes, pendant lesquelles le merge d'une PR retire
`ready` (mika#3022). Le ticket ne porte plus le consentement ; la liste du tick,
elle, le porte encore.

Le 2026-11-05 au merge de #3045, la distance a été de **163 ms** :

| heure | événement |
|---|---|
| 14:10:05.646Z | `merged_pr_ready_label_cleaned` — `ready` retiré |
| 14:10:05.808786Z | le callout est réécrit en « Plan antérieur » (mika#2839) |
| 14:10:05.808970Z | `auto_pull_promotion_allowed` (`phase2_stuck_rescue`), puis `dev-pilot_dispatch_prepared` |
| 14:10:06Z | `ready_label_dispatch_readiness_failed` : `dispatch_no_grooming_marker` |

**Ce qui a arrêté le dispatch n'est pas une garde de consentement.** C'est
`dispatch_no_grooming_marker`, et il n'a mordu que parce que la réécriture du
callout a précédé la promotion de **184 µs**, dans la même passe. Rien n'ordonne
ces deux écritures. Dans l'ordre inverse, le même tick dispatche un pilote sur un
ticket dont `ready` venait d'être retiré — c'est-à-dire sans consentement.

Le résidu de ce quasi-dispatch a coûté deux tours : la parente `0529ed9b` est
restée `pending`, a été ré-armée à 14:55:47Z par la grâce de mika#2045, puis est
passée `failed` à 14:58:57Z sans aucun dispatch.

**Population.** Sur les 11 `merged_pr_ready_label_cleaned` écrits depuis le
deploy de mika#3022, un seul est suivi d'un `stuck_ready_direct_dispatch` sur le
même ticket dans la minute. Le cas exige qu'un tick tombe dans les secondes du
merge et que le ticket garde un plan sur `main`. C'est rare, et c'est une course :
sa fréquence est celle d'une coïncidence d'horloges, pas celle d'un bug
déterministe. Au merge de #3046 (19:45:59Z) il n'y a eu aucun dispatch.

### Gel des tickets (jusqu'au 01/12)

Ce vol n'ouvre aucun ticket de suivi. Le vol 2 (§ 6) est dispatché sur
mika#3047 lui-même, qui reste OPEN — le précédent est mika#3044, dont le vol 1
(PR #3045) ne portait pas de `Closes` et dont le vol 2 (PR #3046) a été dispatché
sur le même ticket.

---

## 1. Le site, sur `c8ef105f6`

| élément | lieu | ce qu'il donne à ce vol |
|---|---|---|
| la liste du tick | `auto_pull.rs` l. 3031 `gh_list_open_issues`, appelée l. 4775 | la copie dont tout le reste dérive ; `--json number,body,labels,updatedAt` |
| le filtre `ready` de la phase 2 | l. 5666, dans `phase2_reconcile_stuck_ready` (l. 5649) | le premier lecteur de cette copie — **il reste en place**, c'est le filtre de périmètre, pas la garde de consentement |
| le cap et son incrément | l. 5999 `attempted >= MAX_STUCK_RESCUE_PER_TICK` (= 5), l. 6091 `attempted += 1` | la borne de coût sous laquelle la relecture doit vivre |
| la porte de promotion | l. 6072 `promotion_gate_allows` | la dernière décision avant le dispatch aujourd'hui |
| **le point d'insertion** | entre l. 6091 et l. 6092 | là où la « Taille mesurée » du ticket place la relecture |
| le dispatch en processus | l. 6092 `dispatch_rescued_ticket_in_process` (l. 5607) | son fetcher injecté (l. 5627) rend `body` + `labels` de la copie du tick |
| la prémisse invalidée | l. 1446-1452, `classify_promotion` | « Le ticket porte déjà `ready` » — ce que la liste du tick ne prouve plus |
| le churn | l. 6162 `gh_remove_label(label_auth, n, "ready")` | ce que le refus ne doit **pas** atteindre (§ 3.2) |
| la couture `gh` | `gh_process.rs` l. 262 `run_gh`, plancher `FIXTURE_ISSUE_FLOOR = 990_000` | le seul chemin autorisé pour la relecture (scan d'égalité l. 522) |
| le patron de lecture d'un ticket | `auto_pull.rs` l. 3531 `gh_issue_body` | la forme exacte à mirer : `run_gh(token, GhSubject::Issue(n), &["issue","view",…])` |

**`classify_promotion` n'est pas touchée par ce vol.** Elle est pure et son
verdict est juste *pour la mesure qu'on lui donne* : une branche supprimée par le
merge de sa PR promeut en phase 2 parce que c'est ainsi que part le vol suivant
d'un ticket à plusieurs vols (mika#2837). Ce qui manque n'est pas une décision
différente, c'est **un fait frais à l'instant du dispatch**. Le commentaire
l. 1446 est amendé pour dire que la prémisse « le ticket porte déjà `ready` » est
désormais vérifiée au dispatch, et par qui — sans quoi la prochaine lecture de ce
`match` croira que la liste du tick la prouve.

---

## 2. AC1 — la relecture, et pourquoi elle est placée là

### 2.1 Ce qu'elle lit

Une fonction, un ticket, un appel :

```rust
async fn gh_reread_issue_at_dispatch(github_token: &str, n: u64) -> Result<DispatchReread>
```

`run_gh(github_token, GhSubject::Issue(n), &["issue","view", …, "--json",
"state,labels,body,updatedAt"])` — mirant `gh_issue_body` (l. 3531) jusqu'au
traitement du `status` non nul. Pas de `-q` : les quatre champs sont parsés en
Rust, parce que le dispatch a besoin des trois derniers et que la décision a
besoin du premier.

`updatedAt` est demandé bien que personne ne le lise ici : l'[`Issue`] rendue est
une `Issue` entière, pas une `Issue` à un champ vide. Un `updated_at: ""` serait
un champ qui mente à son prochain lecteur (mika#2331), pour un coût nul.

```rust
enum DispatchReread {
    /// Ouvert, et portant `ready` : la lecture fraîche, celle qu'on dispatche.
    Ready(Issue),
    /// Ouvert, mais `ready` a disparu depuis la liste du tick.
    ReadyAbsent,
    /// Fermé depuis la liste du tick.
    Closed,
}
```

**`state` se lit fail-closed.** `gh` rend `"OPEN"` ou `"CLOSED"` ; la comparaison
est insensible à la casse, et **toute autre valeur** — un champ absent, un JSON
illisible, un `state` inconnu — rend `Err`, jamais `Ready`. Une valeur qu'on ne
sait pas lire n'est pas « ouvert » : c'est le cas 3 d'AC2, et traiter l'absence
comme un zéro est précisément ce que mika#2331 refuse.

### 2.2 Pourquoi entre `attempted += 1` et le dispatch, et pas avant

Le ticket place la relecture « entre `promotion_gate_allows` et
`dispatch_rescued_ticket_in_process` ». Dans cet intervalle il reste un choix :
avant ou après `attempted += 1` (l. 6091).

**Après.** La relecture est le dernier contrôle **du dispatch**, donc elle est
bornée par le même budget que lui : au plus `MAX_STUCK_RESCUE_PER_TICK` = 5
lectures par tick, quoi que fasse la forge. Placée avant l'incrément, une forge
qui répond mal ferait une lecture par candidat et par tick, sans borne autre que
la taille du vivier — le coût caché que le cap existe pour refuser (le
commentaire l. 5989-5995 le dit de la chaîne de readiness du handler, et la
relecture est de la même classe).

**Le scan structurel de mika#2470 (l. 8192) reste donc vert sans être desserré** :
il exige `cap_check < attempt_increment < dispatch`, et l'ordre est inchangé. Un
refus de relecture n'incrémente ni `rescued`, ni le budget de re-drive, ni le
compteur d'échecs — il `continue`.

---

## 3. AC2 — fail-closed, trois noms, et ce que le refus ne touche pas

### 3.1 Trois filtres, parce que trois remèdes

```rust
const FILTER_READY_ABSENT_AT_DISPATCH: &str = "ready_absent_at_dispatch";
const FILTER_ISSUE_CLOSED_AT_DISPATCH: &str = "issue_closed_at_dispatch";
const FILTER_DISPATCH_REREAD_FAILED: &str  = "dispatch_reread_failed";
```

Les deux premiers sont les chaînes qu'AC1 nomme, à l'octet. Le troisième est le
« filtre nommé » qu'AC2 exige pour la relecture en erreur.

**Trois noms et non un.** C'est la doctrine que ce module a déjà payée quatre
fois — `below_threshold` / `no_ready_label_event` (mika#2131),
`in_flight_self_dev` / `live_pilot_orphaned_parent` (mika#2279),
`operator_review_or_blocked` / `abandoned_operator_held` (mika#2361),
`egress_relay_down` / `pilot_launcher_braked` (mika#2634) : deux états qui se
ressemblent et dont les remèdes diffèrent ne partagent pas un nom. Ici :

- `ready_absent_at_dispatch` est **le succès de ce vol** — la course a eu lieu et
  la garde a mordu. Son compte est le numérateur de la sonde AC4.
- `issue_closed_at_dispatch` est un ticket fermé entre deux lectures : rien à
  faire, et surtout rien à écrire dessus (mika#2713).
- `dispatch_reread_failed` est une panne de forge. Un compte qui ne décroît pas
  nomme une panne à traiter à sa source, pas un ticket à filtrer plus fort.

Fondues sous un seul nom, les trois populations ne seraient pas soustractables —
l'erreur que mika#2205 a mesurée et que les deux scans de wire-format de ce
module existent pour rendre impossible.

### 3.2 Le refus `continue` **avant** le churn, et c'est la moitié qui compte

Le churn (l. 6162-6183) est un `remove` puis un `add` de `ready` par
l'applicateur canonique. Sur un refus de relecture, il ne doit pas courir, et pas
seulement par économie :

- sur `ready_absent_at_dispatch`, re-poser `ready` serait **se rendre le
  consentement qu'on vient de rendre** — exactement ce que la phase 0 refuse
  depuis mika#3036 sous `ConsentReturnedByMerge` (l. 1443). Le défaut serait
  alors pire qu'aujourd'hui : non plus un dispatch sans consentement, mais un
  consentement fabriqué par la boucle ;
- sur `issue_closed_at_dispatch`, toute écriture part vers un ticket fermé. C'est
  la forme de la fuite mika#2713 (29 commentaires sur le #2636 fermé).

Le `continue` est donc placé avant `dispatch_rescued_ticket_in_process` et, par
construction, avant tout ce qui suit : pas de parente, pas de churn, pas de
`record_auto_pull`, pas d'incrément de budget. C'est la lettre d'AC1 — « aucun
dispatch préparé, aucune parente créée ».

### 3.3 La ligne d'audit est celle du ledger, et pourquoi les tests ne la lisent pas

`ExclusionLedger::record` est le canal canonique : son `flush` écrit une ligne
`audit_events` par refus (`tool_name = auto_pull_exclusion`, `after_value =
<filtre>`, `reasoning = phase2_stuck_ready`). C'est cette ligne qui « nomme la
raison » au sens d'AC1, et aucune décision d'exclusion ne repart vers un `debug!`
que le filtre de log de ce serveur ne collecte pas (mika#2131, garde l. 9654).

**Les tests assertent sur `ledger.entries`, pas sur `audit_events`.** Le flush
passe par `exclusion_audit_is_due`, dont la table `EXCLUSION_AUDIT_SEEN` est un
`static` **global au processus** keyé par `(phase, issue, filtre)` avec une
fenêtre de 24 h : une assertion sur la base serait sensible à l'ordre
d'exécution de la suite. Le ledger en mémoire est le même fait, sans l'état
global — et c'est ce que font déjà les quatre tests de ledger de ce module
(l. 11448, 11487, 11696, 11768).

---

## 4. AC3 — le fetcher rend la lecture fraîche

L'[`Issue`] passée à `dispatch_rescued_ticket_in_process` devient celle
qu'`AC1` vient de lire. **La signature ne change pas** : la fonction prend
toujours un `&Issue`, et les quatre tests qui l'appellent directement (l. 7961,
8019, 8052, 8087) compilent sans un octet de changement.

Deux conséquences à écrire, parce qu'aucune n'est visible depuis le site :

1. **Le doc-comment D2 (l. 5603-5606) devient faux et doit être amendé.** Il
   affirme aujourd'hui que le handler décide « on the **same snapshot** the
   Phase 2 filters just used to call the ticket eligible ». C'était la propriété
   recherchée en mika#2470 (zéro `gh issue view` par secours) et c'est la cause
   de mika#3047. Le nouveau texte dit : la lecture est fraîche, elle coûte un
   appel, et c'est ce coût qui achète le consentement.
2. **L'aiguille du scan de mika#2470 (l. 8206) se resserre.** Elle cherche
   `"dispatch_rescued_ticket_in_process(db, ctx, issue"` et l'argument ne
   s'appelle plus `issue`. Elle devient
   `"dispatch_rescued_ticket_in_process(db, ctx, "`. Le resserrement est un gain
   et non une concession : le scan garde un **ordre** (le dispatch précède le
   churn), jamais le nom d'une variable locale, et une aiguille qui épingle ce
   nom est stale au premier renommage sans que la propriété ait bougé.

**Le discriminant comportemental.** Un test oppose les deux copies dans un seul
appel de la phase 2 : la liste du tick porte `UNGROOMED_BODY`, la relecture
cannée porte `GROOMED_BODY`, et la preuve `PLAN_GROOMED` est en base. L'observable
est la classe préparée — `dev-pilot_dispatch_prepared` si le handler a reçu la
lecture fraîche, `dev-groom_dispatch_prepared` s'il a reçu la copie du tick. Le
patron est écrit : `mika2470_the_handler_decides_groom_vs_implement` (l. 8002)
oppose déjà ces deux corps et lit cette ligne. Il n'y a pas de verdict pour
lequel les deux chemins rendraient les mêmes octets.

La seconde phrase d'AC3 — *« un test garde qu'aucun chemin de dispatch de la
phase 2 n'utilise plus la copie du tick »* — est le scan structurel, et il va au
vol 2 (§ 6).

---

## 5. Les tests, et la couture à desserrer d'abord

### 5.1 Pourquoi le double `gh` doit gagner un cannage par motif d'`argv`

Pour qu'un test atteigne le dispatch, un seul appel de
`phase2_reconcile_stuck_ready` doit voir **deux charges différentes sur le même
sujet** `GhSubject::Issue(n)` :

- `gh api repos/…/issues/<n>/timeline` (l. 5909, via
  `ready_label::GhTimelineFetcher`) doit rendre un `labeled(ready)` plus vieux
  que le seuil, sans quoi le ticket sort sous `no_ready_label_event` ;
- `gh issue view <n> --json state,labels,body,updatedAt` doit rendre l'objet
  d'état de la relecture.

Or `CANNED_STDOUT` est keyé **par sujet seul** (`gh_process.rs` l. 306-311) : une
seule charge par issue. `CANNED_FAILURES`, lui, est keyé par sujet **et par motif
d'`argv`** depuis mika#2839 — pour la raison exacte qui bloque ici, écrite dans
son doc-comment l. 231-237 : *« un test qui veut mesurer ce que fait un appelant
quand une écriture est refusée porte souvent deux appels sur la même issue […]
dont un seul doit échouer »*.

Ce vol lève l'asymétrie. La valeur de `CANNED_STDOUT` devient une liste
`Vec<(Option<String>, Vec<u8>)>` ; `set_canned_stdout` garde sa signature et sa
sémantique de remplacement (motif `None`), et `set_canned_stdout_for(subject,
argv_needle, stdout)` s'ajoute. L'ordre de résolution devient : échec canné →
premier motif qui apparie l'`argv` → charge sans motif → `b"[]"`. Aucun appelant
existant ne change.

C'est la couture, donc **le seul** `Command::new("gh")` de production hors
allowlist reste interdit (scan d'égalité l. 522) : la relecture passe par
`run_gh`, comme `gh_issue_body`.

### 5.2 Les quatre cas d'AC2, dans un appel de la phase 2 chacun

Une fixture commune, `tick_pret_a_dispatcher(n)`, pose : la base en mémoire, la
liste du tick (`GROOMED_BODY`, `["ready"]`), la timeline cannée avec un
`labeled(ready)` ancien, la preuve `PLAN_GROOMED` en base, et le
`LabelWriteToken` de test. Les quatre tests n'en diffèrent que par la charge de
la relecture :

| | relecture cannée | attendu |
|---|---|---|
| T1 **contrôle positif** | `state: OPEN`, labels `["ready"]` | dispatche comme aujourd'hui : `dev-pilot_dispatch_prepared`, une parente, aucun des trois filtres au ledger |
| T2 | `state: OPEN`, labels `["p1-important"]` | `ready_absent_at_dispatch` au ledger, **zéro** parente, aucun appel `--remove-label` au journal du double |
| T3 | `state: CLOSED`, labels `["ready"]` | `issue_closed_at_dispatch` au ledger, zéro parente, aucune écriture au journal |
| T4 | `set_canned_failure(Issue(n), "issue view", …)` | `dispatch_reread_failed` au ledger, zéro parente, aucune écriture |

T1 n'est pas décoratif : sans lui, les trois autres seraient satisfaits par une
garde qui refuse tout le monde — le mode de défaillance que les contrôles
négatifs de mika#2049 et mika#2634 ont dû ajouter après coup (l. 11469, 11720).

`parents_for(db, n)` (l. 7922) est l'observable de « aucune parente créée », et
`gh_calls_for(n)` (l. 7921) celui de « rien n'a été écrit sur la forge » — le
journal du double est global au processus, donc le filtre par numéro est ce qui
rend ces tests sûrs en parallèle (D4 de mika#2713).

### 5.3 Numérotation des fixtures, et ce qu'elle protège

Les nouveaux tests prennent **990 105 à 990 109**, au-dessus du plancher
`FIXTURE_ISSUE_FLOOR` et distincts de 990 101-990 104 déjà pris. Le plancher est
asserté à la pose (`assert_fixture_issue`) et à la couture : il n'y a pas
d'allowlist.

Les quatre tests qui appellent `dispatch_rescued_ticket_in_process` directement
gardent `n = 2470`, **sous** le plancher, et c'est correct : ils n'atteignent
jamais `run_gh`, parce que le fetcher injecté court-circuite toute lecture et que
la relecture vit dans la phase 2, pas dans la fonction de dispatch.

### 5.4 Aucun test existant ne peut rougir — et pourquoi, pas seulement que

Les sept appels existants de `phase2_reconcile_stuck_ready` (l. 9769, 9848,
11332, 11433, 11474, 11678, 11754, 11800, 11854) sortent **tous avant** le point
d'insertion :

- 2360 (`abandoned_operator_held`), 2276 (`live_pilot`), 2634 (`launcher_braked`)
  et 990 101 (`egress_relay_down`) sortent sur un filtre antérieur ;
- les contrôles négatifs et 990 102-990 104 atteignent la mesure d'âge, où le
  double rend `b"[]"` par défaut : aucun `labeled(ready)`, donc
  `no_ready_label_event`, donc aucun candidat à dispatcher.

Les fixtures sous le plancher (2360, 2276, 2634) ne peuvent donc pas faire
paniquer la couture sur la nouvelle lecture. **Cette propriété est vérifiée par
une exécution de la suite, pas par ce paragraphe** — c'est l'objet du point 4 de
la DoD.

---

## 6. Découpage en vols, et ce que ce vol ne fait pas

**Total estimé 409 lignes contre une borne par vol de 429** (§ 11). Ce qui part
au vol 2 est le **scan structurel d'AC3** et son contrôle de bonne foi, estimés à
95 lignes — 504 au total, soit 75 de trop pour un seul vol.

Le découpage n'est pas qu'arithmétique, et le précédent est d'hier. Le § 4 du
plan du vol 1 de mika#3044 refusait d'atterrir un scan de lecteur unique sur un
prédicat **qui n'avait pas encore d'appelant** : rouge par construction, et
convergé par itérations au lieu d'être mesuré. Ici le scan épinglerait une forme
que ce vol est en train d'écrire — quel argument le dispatch reçoit, et dans
quel ordre la relecture le précède. Il atterrit mieux sur du code atterri, avec
son allowlist **mesurée** vide plutôt que supposée.

Ce n'est pas l'option (b) de mika#1574 : rien du détecteur n'atterrit désarmé
dans ce vol, et la propriété qu'il gardera est, au vol 1, tenue par le
discriminant comportemental du § 4 — un test qui rougit si le handler reçoit la
copie du tick.

Tracked in: senara-solutions/mika#3047

Hors vol, et ce sont les frontières du ticket :

- **Les phases 0 et 1.** La phase 0 ne dispatche pas, et la phase 1 promeut un
  ticket qu'elle vient elle-même de rendre `ready` : la fenêtre de mika#3047 n'y
  existe pas sous cette forme.
- **La décision de mika#3036** et le `match` par phase de `classify_promotion`
  (l. 1438-1454) : seul son commentaire est amendé, aucun verdict ne bouge.
- **La parente `pending` laissée par un refus de readiness**, puis ré-armée par
  le reaper. Avec AC1 ce cas ne crée plus de parente ; les autres refus de
  readiness suivent le chemin du pre-digest (mika#1571), qui n'est pas visé.
- **La tâche `ready-label` déjà en file qui survit au retrait du label** —
  parking mika#2983, commentaire 2 (issuecomment-6560377724) : autre site, autre
  forme.
- **La sonde AC4** (§ 7) : elle se relève après le deploy, en commentaire sur
  mika#3047.
- Aucun `MIKA_*`, aucune migration, aucun index, aucun changement de signature
  publique. Schéma v56 inchangé.

---

## 7. AC4 — la sonde, son dénominateur et ses deux pièges

Sur les 14 jours qui suivent le `make deploy`, trois termes, lus sur
`audit_events` :

1. **Le numérateur** — les `stuck_ready_direct_dispatch` précédés d'un
   `merged_pr_ready_label_cleaned` sur le même ticket dans la minute. Cible **0**.
2. **Le dénominateur** — le nombre de `merged_pr_ready_label_cleaned` sur la
   fenêtre (11 sur la fenêtre de mesure du ticket). Sans lui, un numérateur nul
   ne distingue pas « la garde a tenu » de « aucun merge n'est tombé pendant un
   tick ».
3. **La preuve que la garde a mordu** — le compte des trois filtres du § 3.1 dans
   `auto_pull_exclusion`. C'est le seul terme qui distingue un mécanisme
   *efficace* d'un mécanisme *jamais atteint*, et c'est la doctrine mika#2331 :
   absent n'est pas zéro.

**Deux pièges de lecture, à écrire avec le relevé.**

- **Les deux `target_key` n'ont pas la même forme.**
  `merged_pr_ready_label_cleaned` écrit `issue:senara-solutions/mika#3044`
  (`merged_pr_ready_cleanup_handler.rs` l. 261) et
  `stuck_ready_direct_dispatch` écrit `issue:3044` (`auto_pull.rs` l. 6133). La
  jointure se fait sur le **numéro extrait**, jamais sur l'égalité des deux
  champs — qui rendrait zéro croisement sur toutes les fenêtres, y compris celle
  du 2026-11-05 où le croisement a eu lieu.
- **Une absence de ligne peut être un binaire d'avant le deploy** (mika#2340). La
  fenêtre part de l'instant du `make deploy`, et le relevé le nomme.

Un compte de croisements nul s'écrit **« aucun croisement sur la fenêtre »**, et
jamais « course corrigée ».

---

## 8. Definition of Done

1. `cargo build`, `cargo clippy --all-targets -p mika-agent` (zéro avertissement
   neuf), `cargo fmt --all --check`, `cargo test --workspace` verts.
2. `scripts/verify-pipeline.sh origin/main` vert, et les lints du dépôt que CI
   lance.
3. T1 à T4 et le discriminant d'AC3 verts chacun seul
   (`cargo test -p mika-agent mika3047`).
4. **Les sept appels existants de `phase2_reconcile_stuck_ready` et les quatre
   appels directs de `dispatch_rescued_ticket_in_process` ne changent pas de
   verdict**, et c'est une exécution qui le dit, pas le § 5.4. Si l'un rougit par
   le plancher de fixture, le geste est de renuméroter cette fixture — il n'y a
   pas d'allowlist (mika#2713).
5. `mika2470_direct_dispatch_precedes_the_label_churn` (l. 8192) reste vert avec
   son aiguille resserrée, et la trace de son resserrement est dans le corps de
   la PR : un scan dont l'aiguille bouge dans le même diff que ce qu'il garde est
   la chose qu'un relecteur doit voir nommée.
6. `mika2131_filter_names_are_a_wire_format` (l. 10240) porte les trois nouvelles
   égalités, et `ready_absent_at_dispatch` / `issue_closed_at_dispatch` sont les
   octets d'AC1.
7. Aucun appel `gh` ne part vers la forge depuis la suite : le journal du double
   est l'assertion (`gh_calls_for`), et aucun `Command::new("gh")` n'est ajouté.
8. Les fichiers de `src/` modifiés sont exactement deux : `auto_pull.rs` et
   `gh_process.rs`.
9. La PR porte `Code review: delegated to mika-qa (mika#2694)`, référence
   mika#3047 **sans** `Closes` — le vol 2 et la sonde AC4 restent dus — et dit
   que le vol 2 est dispatché sur ce ticket.

---

## 9. Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#3047. La colonne de droite
dit ce que ce vol en sert.

- **AC1.** Juste avant le dispatch en processus, la phase 2 relit l'état du
  ticket à la forge : ouvert, et portant `ready`. Si l'un des deux manque, aucun
  dispatch n'est préparé, aucune parente n'est créée, et une ligne d'audit nomme
  la raison (`ready_absent_at_dispatch`, `issue_closed_at_dispatch`).

  → **Ce vol, en entier** (§ 2 et § 3).

- **AC2.** Fail-closed : si la relecture échoue, la phase 2 ne dispatche pas, et
  le ledger du tick l'inscrit sous un filtre nommé. Un test par cas, dans le même
  appel : `ready` retiré, ticket fermé, relecture en erreur. Le contrôle positif
  (le ticket porte toujours `ready`) dispatche comme aujourd'hui.

  → **Ce vol, en entier** : T1 à T4 au § 5.2, chacun un appel complet de
  `phase2_reconcile_stuck_ready`.

- **AC3.** Le fetcher passé au handler rend la lecture fraîche d'AC1, et non la
  liste du tick. Un test garde qu'aucun chemin de dispatch de la phase 2
  n'utilise plus la copie du tick.

  → **Première phrase : ce vol** (§ 4), avec le discriminant comportemental qui
  la prouve et l'amendement du doc-comment D2. **Seconde phrase : vol 2** — le
  scan structurel, pour la raison du § 6.

- **AC4.** Sonde : sur les 14 jours qui suivent le deploy, on compte les
  `stuck_ready_direct_dispatch` précédés d'un retrait de `ready` dans la minute
  sur le même ticket, puis on l'écrit ici. La cible est 0. Un compte de
  croisements nul s'écrit « aucun croisement sur la fenêtre », et non « course
  corrigée ».

  → **Hors vol.** Protocole, dénominateur et deux pièges de lecture au § 7 ; elle
  se relève en commentaire sur mika#3047 après le deploy.

---

## 10. Fire-Disposition

Ce vol touche **deux détecteurs**, et n'en crée aucun nouveau :

- `mika2131_filter_names_are_a_wire_format` (l. 10240) gagne trois égalités ;
- `mika2470_direct_dispatch_precedes_the_label_churn` (l. 8192) voit son aiguille
  de dispatch resserrée (§ 4, point 2).

Les quatre tests d'AC2 et le discriminant d'AC3 n'en sont pas : leur chemin de
succès est « le comportement attendu est arrivé », pas « aucune violation
trouvée ».

**Option retenue : (a) exception nommée en allowlist — et il n'y a aucune
exception à nommer.**

- **Zéro violation existante à l'atterrissage, et c'est vérifiable avant d'écrire
  le code** : les trois constantes naissent avec les valeurs que les trois
  égalités assertent, donc le wire-format atterrit vert par construction. Le scan
  de mika#2470 atterrit vert parce que l'ordre `cap_check < attempt_increment <
  dispatch` est inchangé (§ 2.2) et que son aiguille apparie l'appel après le
  renommage de l'argument.
- **Aucun mécanisme d'exception n'est livré** : ni liste d'ignorés, ni attribut de
  contournement, ni variable de désarmement, dans aucun des deux détecteurs.
- **L'assertion auto-nettoyante du resserrement est le resserrement lui-même.**
  L'aiguille cesse de citer le nom d'une variable locale pour ne citer que
  l'appel : elle ne peut plus devenir stale sur un renommage sans que la
  propriété gardée — le dispatch précède le churn — ait réellement bougé. Une
  aiguille qui épingle `issue` serait une exception à nommer dès le prochain
  `cargo fmt` du site.
- **Le plancher de fixture n'est pas desserré.** Si un test rougit sur
  `FIXTURE_ISSUE_FLOOR`, le geste est de renuméroter la fixture dans la bande
  990 000+ (DoD 4). mika#2713 le dit en toutes lettres : il n'y a pas
  d'allowlist, et en ajouter une rouvrirait la fuite que ce plancher a fermée.

Aucune option (b) : rien n'atterrit `#[ignore]`. Le scan structurel d'AC3 renvoyé
au vol 2 n'est pas un détecteur désarmé — rien de lui n'atterrit, et la propriété
qu'il gardera est tenue dans ce vol par le discriminant comportemental du § 4
(§ 6). Aucune option (c) : rien ne remonte à l'opérateur, le périmètre est tranché
par la « Taille mesurée » et le « Hors vol » du ticket.

---

## 11. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/gh_process.rs` — cannage de `stdout` par motif d'`argv` : la table, l'ordre de résolution, `set_canned_stdout_for`, la doc | 45 |
| `crates/mika-agent/src/auto_pull.rs` — `DispatchReread` et `gh_reread_issue_at_dispatch` (lecture, parse, `state` fail-closed, doc) | 75 |
| `crates/mika-agent/src/auto_pull.rs` — les trois `FILTER_*` et leur doc | 30 |
| `crates/mika-agent/src/auto_pull.rs` — le filtre au site de dispatch, la lecture fraîche passée au handler, le doc-comment D2 amendé, le commentaire de `classify_promotion` | 52 |
| tests (`auto_pull.rs`) — la fixture `tick_pret_a_dispatcher` et la timeline cannée | 40 |
| tests (`auto_pull.rs`) — les quatre cas d'AC2 | 112 |
| tests (`auto_pull.rs`) — AC3 : le discriminant « corps frais, pas corps du tick » | 35 |
| tests (`auto_pull.rs`) — wire-format étendu et aiguille du scan mika#2470 | 20 |

Total estimé : 409 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**409 est sous la borne par vol de 20 lignes, et c'est le découpage du § 6 qui
l'y met** : le périmètre entier (AC1, AC2, AC3 en entier) pèse 504 lignes, dont
95 pour le scan structurel d'AC3 renvoyé au vol 2 avec son `Tracked in:`. Le
seuil par PR (1000) n'est pas approché.

Trois raisons pour lesquelles ce total est peu susceptible de dériver :

- **Les postes de production sont bornés par un patron existant, ligne pour
  ligne.** `gh_issue_body` (l. 3531) donne la forme entière de la lecture d'un
  ticket par la couture, et `CANNED_FAILURES` (`gh_process.rs` l. 231-244, 289-304)
  donne celle du cannage par motif d'`argv` — la nouvelle table est sa symétrique.
- **Les tests réutilisent des fixtures écrites.** `mem_db`, `make_issue`,
  `GROOMED_BODY` / `UNGROOMED_BODY`, `test_ctx_fixture`, `parents_for`,
  `gh_calls_for`, `audit_after_values`,
  `crate::db::tests::completed_groom_pair` : la fixture du § 5.2 les assemble,
  elle n'en crée aucune.
- **L'estimation compte la doc AVEC le code** (≈ 45 % du total), le ratio mesuré
  sur le vol 1 de mika#3044 (408 estimées pour ~1 123 réelles, facteur 2,75, dû
  pour l'essentiel aux doc-comments que ce dépôt exige). Un total « nu » de
  ~210 lignes serait cette erreur répétée.

---

## 12. Ordre de commit

Trois commits, dans cet ordre, chacun vert seul.

1. **`test(3047): le double gh canne un stdout par motif d'argv, comme il canne déjà un échec`**
   — `gh_process.rs` seul. Vert seul par construction : aucun appelant ne change,
   et `set_canned_stdout` garde sa sémantique. Sans ce commit, aucun test des deux
   suivants ne peut atteindre le dispatch.
2. **`fix(3047): la phase 2 relit le ticket à la forge avant de dispatcher, et refuse fail-closed`**
   — `auto_pull.rs` : `DispatchReread`, `gh_reread_issue_at_dispatch`, les trois
   `FILTER_*`, le filtre au site, le doc-comment D2 amendé, l'aiguille du scan
   mika#2470, le wire-format étendu, et T1 à T4. **C'est le plancher du vol** :
   un mika-qa qui ne lirait qu'un commit doit lire celui-là.
3. **`test(3047): le handler décide sur la lecture fraîche, pas sur la copie du tick`**
   — `auto_pull.rs` : le discriminant d'AC3. Séparé du précédent parce qu'il
   mesure une propriété distincte — non pas « le refus refuse » mais « le dispatch
   dispatche sur le bon corps » — et parce que c'est lui qui, au vol 2, dira au
   scan structurel quelle forme épingler.
