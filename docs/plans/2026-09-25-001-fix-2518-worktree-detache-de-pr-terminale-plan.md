# mika#2518 — Un worktree détaché dont la PR est terminale est fauchable

**Ticket :** `senara-solutions/mika#2518` (enfant de mika#2420, milestone mika#2491)
**Cible :** `crates/mika-agent/src/worktree_reaper.rs` (bras faucheur, terme T2)
**Type :** fix substrat — une clé de résolution ajoutée, aucun terme de sûreté touché
**Voisin :** mika#2497 (purge `target/` des worktrees vifs) — population adjacente, élargie en effet de bord **nommé** au § 4.3

---

## 0. Ce que la lecture du code déplace dans le ticket

Quatre rectifications, vérifiées sur l'arbre à `c5c4d70f`. Aucune ne change la
population à traiter ; **deux changent le mécanisme du remède**, une change ce
que le plan peut promettre, une ouvre un suivi.

### R1 — La cause écrite dans le ticket est réfutable, et le remède ne doit pas en dépendre

Le ticket pose : « À la fermeture d'une PR la branche est supprimée → le HEAD du
worktree devient **détaché** ». La première implication est fausse en git, dans
les deux lectures possibles de « la branche » :

- **Supprimer `origin/<branche>`** (ce que GitHub fait à la fermeture, et ce que
  `git fetch --prune` propage) ne touche pas `refs/heads/<branche>` et ne
  détache **rien**. Le worktree reste sur sa branche locale, simplement sans
  remote-tracking — état que `collect_work_state` traite déjà explicitement
  (`worktree_reaper.rs:1137-1144`, « ref absente ⇒ branche distante supprimée
  après fermeture de la PR ⇒ `Clean` »).
- **Supprimer `refs/heads/<branche>`** par `git branch -D` est **refusé par git**
  tant que la branche est checked out dans un worktree (`error: Cannot delete
  branch 'x' checked out at '<path>'`). C'est d'ailleurs pourquoi le
  `git branch -D` de `remove_worktree` (`:1252`) est best-effort : il ne peut
  aboutir **qu'après** le retrait du worktree.

Donc le détachement **requiert un geste explicite** (`git checkout --detach`,
`git switch --detach`, `git worktree add --detach`, un `git update-ref -d` qui
contourne la garde de `branch -D`, un rebase interrompu, ou une chirurgie sur
`packed-refs`). Aucun de ces gestes n'existe dans `dispatch-lib.sh` : ses deux
seules créations de worktree attachent (`worktree add -b "$BRANCH"` puis le
repli `worktree add "$WORKTREE_DIR" "$BRANCH"`, `:2921-2930`) et son seul
`checkout "$BRANCH"` (`:2891`) attache aussi.

**Le producteur du détachement est donc non identifié**, et il n'est pas
identifiable depuis le bac à sable de dispatch (les trois worktrees mesurés ont
été purgés à la main le 2026-09-24, et `/data/workspace/mika-platform/mika`
n'est pas monté ici). **Conséquence de conception, et c'est la principale :** le
prédicat doit être **agnostique du producteur**. Il ne suppose ni un geste, ni un
ordre, ni une provenance ; il lit l'état du worktree tel qu'il est. Candidats
pour l'enquête et son préalable : § 9, suivi F1.

### R2 — Ni le chemin ni un mapping de métadonnées ne sont la bonne clé. Le SHA du HEAD l'est, et il est exact là où les deux autres sont heuristiques.

AC1 énumère deux mécanismes de résolution : « le **chemin du worktree** ou ses
**métadonnées** (mapping worktree→PR/issue) ». Les deux sont refusés, chacun sur
une mesure :

| mécanisme | refus | mesure |
|---|---|---|
| dérivation depuis le chemin | **heuristique et lossy** | le répertoire est produit par `scripts/derive-worktree-path` avec `/`→`-` et translittération : `feat/2425/agent-exposer-le-réglage-context-history` → `feat-2425-agent-exposer-le-r-glage-context-history`. L'inverse n'est pas une fonction — deux branches dont les slugs collident donnent le même répertoire. Et re-dériver un chemin de worktree est très exactement la duplication que mika-platform#58 a fermée, règle déjà écrite dans le module : *« le registre est la vérité terrain, jamais une dérivation de chemin »* (`:583-585`) |
| stamp de métadonnées worktree→PR | **n'atteint pas la population mesurée** | il faudrait que `dispatch-lib.sh` estampille le numéro de PR, qu'il ne connaît qu'à `_post_flight_recovery`. Un stamp ne couvre que les dispatches **futurs** ; les trois worktrees du ticket sont déjà sur disque sans stamp. C'est la propriété qui a mis #1694 en échec et que mika#2420 a dû écrire : *« un hook ne rattrape jamais ce qu'il a raté »* |

**La clé que le ticket n'a pas vue existe, elle est déjà dans les deux flux, et
elle est exacte.** `git worktree list --porcelain` émet une ligne `HEAD <sha>`
pour **toute** entrée, détachée comprise — mesuré sur cet arbre :

```
worktree /data/.../feat-2518-.../mika
HEAD c5c4d70f0cebdbfe3e821e65951d53473e8d99d4
branch refs/heads/feat/2518/faucheur-un-worktree-de-pr-merg-e-dont
```

et `gh pr list --json headRefOid` rend le commit de tête de chaque PR — champ
**déjà demandé en production** sur exactement cet appel
(`server/ci_success_handler.rs:617`, `"number,headRefOid"`). La résolution est
donc `HEAD du worktree == headRefOid de la PR` : une égalité de SHA, sans
parseur, sans heuristique, sans nouveau canal.

**Et c'est aussi l'argument de sûreté, qui est plus fort que celui du chemin.**
Apparier exactement le `headRefOid` d'une PR signifie *ce worktree est à l'état
livré, et pas un commit de plus*. Un worktree qui porte du travail non fusionné a
un HEAD différent et **ne peut pas apparier** — il sort de la population de
lui-même, avant même T7. Le pire scénario — un worktree détaché sur la tête de
`main`, qui après un rebase-merge **est** le `headRefOid` d'une PR mergée — se
résout correctement : un tel worktree ne contient qu'un checkout de `main`, sans
travail ; s'il en portait, il serait `dirty` (T7) ou son HEAD serait ailleurs.

### R3 — La disposition ne doit PAS recevoir un second `tool_name`, et AC4 se lit comme une intention, pas comme un nom

AC4 demande « un motif distinct (p. ex. `reaped_detached_merged`) séparable de
`worktree_reaped` nominal ». Le « p. ex. » est pris au mot : créer un second
`tool_name` casserait une propriété que le module documente et qu'un test épingle
(`mika2420_le_tool_name_daudit_a_un_seul_writer`, `:3541`) —
`SELECT … WHERE tool_name = 'worktree_reaped'` est **la liste exacte des
worktrees que la boucle a retirés**, c'est-à-dire le garde-fou 3 de mika#2420. Un
second nom la tronquerait **en silence**, et toute requête opérateur publiée dans
le `CLAUDE.md` racine sous-compterait sans rien casser.

La maison a les deux motifs et les distingue :

- **deux noms** (`phantom_aged_out` / `phantom_sweep_spared`, mika#2156 ;
  `qa_deadline_verdict` / `qa_callback_verdict`, mika#2368) quand **chaque nom
  porte sa propre cause** et que les populations ne doivent jamais être sommées ;
- **un nom, le discriminant dans le champ** (`ready_label_outcome`, mika#2323 ;
  `task_engine_groom_pilot_dispatcher`, mika#2498) quand les deux issues
  appartiennent au même dispatcheur et à la même population.

Ici les deux retraits sont faits par **le même bras**, sous la **même
conjonction** de sept termes, avec la **même létalité** : seule la clé de
résolution diffère. C'est le second motif. Donc : **un `tool_name`, un champ
`resolution`** sur la ligne de journal et en tête de `reasoning`. L'intention
d'AC4 — « séparable » — est servie exactement (`jq 'select(.resolution ==
"detached_sha")'` et `reasoning LIKE 'resolution=detached_sha%'`), sans payer la
rupture.

### R4 — Aucun nouveau réglage, et refuser un knab dédié est une décision, pas un oubli

Le réflexe serait de donner au chemin détaché sa propre disposition ou son propre
kill-switch, comme mika#2497 s'en est donné un. **Refusé, et le critère de
mika#2497 est ce qui le refuse :** il a scindé les leviers parce que *« les deux
létalités diffèrent d'un ordre de grandeur »* (faucheur = travail potentiel,
purge = dérivé pur). Ici la létalité est **identique au bit près** — même
`remove_worktree`, même conjonction, même `bytes_reclaimed`. Deux interrupteurs
pour une seule décision obligeraient l'opérateur à en poser deux pour arrêter les
suppressions, ce qui est la faute symétrique de celle que mika#2498 a dû corriger
(« un opérateur qui arrête l'une et pas l'autre n'a rien arrêté »).

Le chemin détaché hérite donc, sans une ligne : `MIKA_WORKTREE_REAP=0`,
`MIKA_WORKTREE_REAP_DISPOSITION=observe`, `MIKA_WORKTREE_REAP_GRACE_SECS`,
`MIKA_WORKTREE_REAP_MAX_PER_TICK`, et la sentinelle STOP partagée
`~/.mika/state/worktree-reap-stop`. **Zéro variable d'environnement nouvelle,
zéro migration, zéro colonne.**

---

## 1. Requirements

### R-1 — La résolution change la CLÉ, jamais le prédicat

Les sept termes T1–T7 de mika#2420 sont inchangés, dans leur ordre, leur
formulation et leur direction de fail-safe. Ce qui change est **un seul point** :
la façon d'obtenir l'ensemble des PR associées à une entrée du registre.

| avant | après |
|---|---|
| T2 : branche attachée, sinon refus `detached_head` | T2 : **résoudre une clé** — branche attachée (inchangé), ou SHA du HEAD détaché |
| T3–T7 | **identiques**, sur l'ensemble de PR ainsi résolu |

C'est cette propriété qui rend AC3 gratuite : aucun terme fail-safe n'est
réécrit, donc aucun ne peut régresser.

### R-2 — Toute information manquante SORT de la population, sans exception

Le fail-safe du module (`:732-733`, *« un worktree dont un seul terme est
illisible est conservé ; il n'existe aucune exception »*) s'étend à la nouvelle
clé :

| situation | disposition | motif |
|---|---|---|
| détaché, pas de ligne `HEAD` dans le porcelain | conserver | `detached_head` |
| détaché, `HEAD` = SHA nul (`000…0`) ou non hexadécimal ou de longueur non canonique | conserver | `detached_head` |
| détaché, SHA lisible, **aucune** PR à ce SHA | conserver | `detached_head_pr_unknown` |
| détaché, SHA lisible, au moins une PR **ouverte** à ce SHA | conserver | `pr_open` |
| `headRefOid` absent de la réponse `gh` | conserver (et **le chemin attaché reste intact** — voir R-4) | `detached_head_pr_unknown` |

Le SHA nul n'est pas théorique : mesuré sur cet arbre, `git worktree list
--porcelain` rend `HEAD 0000000000000000000000000000000000000000` + `detached`
pour le checkout principal.

### R-3 — Deux motifs de refus nouveaux, et la SCISSION est datée

`detached_head` **existe** et son sens se resserre : il ne veut plus dire « pas
de branche attachée » mais « pas de branche attachée **et** pas de SHA
exploitable ». La population qui s'en détache reçoit son propre nom,
`detached_head_pr_unknown` (« SHA lisible, aucune PR à ce SHA »).

Les deux doivent être comptables séparément : le premier est une **anomalie
git** (remède : inspecter le worktree), le second est **nominal** pour un
worktree hors boucle — et c'est surtout **la sonde qui dit que la clé SHA ne
mord pas** (§ 8, S1).

**Coût nommé et daté** (motif mika#2361) : une requête `GROUP BY after_value`
qui enjambe le déploiement compare deux vocabulaires. Les lignes antérieures
gardent `detached_head` et **ne sont pas réécrites** — les réécrire rendrait faux
ce qu'elles ont dit quand elles ont été écrites. Un opérateur qui compare de part
et d'autre doit **sommer les deux noms**. À dater dans le `CLAUDE.md` racine.

### R-4 — Le champ `headRefOid` est additif et ne peut pas rendre le faucheur inerte

`PrSnapshot` gagne `head_ref_oid`. Le module pose en doctrine que `state` et
`headRefName` **n'ont pas** de `#[serde(default)]` (`:596-600`) : leur absence
ferait *entrer* un worktree dans la population sur une information manquante.

**Ici l'asymétrie est inverse et la décision l'est donc aussi.** Un
`headRefOid` absent ferait échouer le parsing, donc `list_prs` rendrait `Err`,
donc **le dépôt entier serait sauté** (`worktree_reap_failed stage=pr_list`,
`:1372`) — y compris le chemin attaché qui fonctionne aujourd'hui. Un champ
additif ne doit pas pouvoir éteindre la fonction qu'il enrichit. Donc
`#[serde(default)]`, avec **la chaîne vide traitée comme non résolvable** : la
dégradation est bornée au nouveau chemin.

### R-5 — La disposition détachée ne supprime PAS de branche locale

`remove_worktree` appelle `git branch -D <branch>` inconditionnellement
(`:1252`). Sur le chemin détaché, la branche nommée par la PR n'a **jamais été
checked out par ce worktree** : la supprimer serait un effet de bord sans mandat
(et, si elle a disparu, un `-D` qui échoue sans information). Le chemin détaché
saute cet appel et rapporte `branch_deleted = false`.

### R-6 — Le nom de branche rapporté vient de la PR, jamais d'une dérivation

Sur le chemin détaché, `ReapCandidate.branch` et `ReapRefusal.branch` sont
remplis avec le `headRefName` **de la PR appariée** — une donnée déclarée par
GitHub, pas une inversion de slug. Deux conséquences : T7 dispose d'un
`origin/<branche>` pour son second sous-processus, et la surface opérateur de
mika#2497 (`:2913`, qui résout son `pr_number` depuis `refusal.branch`) cesse
d'être aveugle sur cette population.

### R-7 — Aucun repli heuristique derrière la clé exacte

Si le SHA n'apparie aucune PR, **on conserve**. On ne retombe pas sur une
dérivation de chemin. Raison : un repli heuristique se déclencherait exactement
quand la clé exacte dit *« ce worktree n'est pas à un état livré »*, c'est-à-dire
quand conserver est la bonne réponse.

### R-8 — Périmètre : le chemin attaché est inchangé, y compris sa clé

Un worktree attaché continue d'être résolu **par sa branche**. Le résoudre aussi
par SHA serait un risque gratuit sur le chemin nominal. Épinglé par un test
d'anti-vacuité (§ 5, V3c), sans lequel « la clé SHA marche » serait
indistinguable de « tout est résolu par SHA ».

---

## 2. Conception — la résolution

### 2.1 Le registre porte le SHA

`WorktreeEntry` gagne `head: Option<String>` ; `parse_worktree_registry` capture
la ligne `HEAD <sha>` en plus de `worktree` / `branch` / `prunable`. Les entrées
`prunable` restent écartées (leur répertoire n'existe déjà plus).

Normalisation à la lecture, dans **un seul site** :

```rust
/// Le SHA d'un HEAD détaché, s'il est exploitable comme clé.
///
/// Refuse : la chaîne vide, le SHA nul (`000…0` — mesuré en production sur le
/// checkout principal), tout ce qui n'est pas 40 caractères hexadécimaux. Un
/// SHA non exploitable n'est jamais « pas de PR » : c'est `detached_head`.
fn usable_head_sha(raw: &str) -> Option<String>
```

40 caractères exactement : `git worktree list --porcelain` rend le SHA complet.
Accepter un préfixe ouvrirait un appariement partiel, c'est-à-dire une
heuristique — exactement ce que R-2 et R-7 refusent.

### 2.2 `PrIndex` — les deux clés dans un objet qui ne peut pas diverger

```rust
pub struct PrIndex {
    by_branch:   HashMap<String, Vec<PrSnapshot>>,
    by_head_sha: HashMap<String, Vec<PrSnapshot>>,   // clé en minuscules
}
impl PrIndex {
    pub fn build(prs: Vec<PrSnapshot>) -> Self;      // site de construction UNIQUE
    pub fn by_branch(&self, branch: &str) -> Option<&[PrSnapshot]>;
    pub fn by_head_sha(&self, sha: &str) -> Option<&[PrSnapshot]>;
}
```

Un struct plutôt qu'un sixième paramètre à `screen_worktrees` : les deux index
sont construits de la même liste au même instant et ne peuvent pas se
désynchroniser. `index_prs_by_branch` est absorbée (elle n'a aucun appelant hors
du module — vérifié). Les deux côtés de l'appariement sont **mis en
minuscules** ; git et GitHub rendent tous deux du minuscule, la normalisation est
défensive et coûte un `to_ascii_lowercase` par PR.

### 2.3 Le discriminant de résolution — format de fil

```rust
/// Par quelle clé ce worktree a été rattaché à ses PR.
///
/// **Format de fil** : atterrit dans `audit_events.reasoning` (en tête) et sur
/// le champ `resolution` de la ligne INFO. Deux orthographes couperaient une
/// population en deux sans le dire.
pub const RESOLUTION_BRANCH: &str = "branch";
pub const RESOLUTION_DETACHED_SHA: &str = "detached_sha";
pub const ALL_RESOLUTIONS: &[&str] = &[RESOLUTION_BRANCH, RESOLUTION_DETACHED_SHA];
```

`"detached_sha"` et non `"detached_head"` **délibérément** : ce dernier est déjà
un motif de refus (`REASON_DETACHED_HEAD`). Deux vocabulaires distincts qui
partageraient une chaîne se liraient mal, même s'ils vivent dans des champs
différents — et le nom retenu dit la clé réellement employée.

### 2.4 T2 devient une résolution, et T3–T7 ne bougent pas

```
// T2 — résoudre la clé de rattachement.
let resolved = match entry.branch.as_deref() {
    Some(branch) => match prs.by_branch(branch) {
        Some(p) if !p.is_empty() => Resolved { prs: p, branch, kind: RESOLUTION_BRANCH },
        _ => { refuse(REASON_PR_UNKNOWN); continue }          // T3 attaché, inchangé
    },
    None => {
        let Some(sha) = entry.head.as_deref().and_then(usable_head_sha) else {
            refuse(REASON_DETACHED_HEAD); continue            // R-2 : SHA inexploitable
        };
        match prs.by_head_sha(&sha) {
            Some(p) if !p.is_empty() =>
                Resolved { prs: p, branch: p[0].head_ref_name, kind: RESOLUTION_DETACHED_SHA },
            _ => { refuse(REASON_DETACHED_HEAD_PR_UNKNOWN); continue }
        }
    }
};
// T4 (aucune PR ouverte) … T7 : identiques, sur `resolved.prs`.
```

Le refus `pr_unknown` du chemin attaché garde son nom et sa place : c'est la même
question (« aucune PR connue pour cette clé ») posée d'une clé différente, et
les deux populations doivent rester comptables séparément (le premier est
fréquent et nominal — « groomé, pas encore implémenté » ; le second est la sonde
d'attribution de ce ticket).

### 2.5 T4 reste formulé en négatif, et c'est ce qui couvre le cas rouvert

Deux PR peuvent partager un `headRefOid` (une PR fermée puis rouverte en une
nouvelle depuis le même commit). Le prédicat existant — *« aucune PR n'est
ouverte »*, `:717-725` — s'applique tel quel à l'ensemble résolu par SHA, sans
une ligne de plus. C'est la conséquence directe de R-1 : **la clé change, le
prédicat ne change pas.**

### 2.6 T7, et pourquoi le chemin détaché est plus sûr que l'attaché

`collect_work_state(path, branch)` est appelé avec le `headRefName` de la PR
(R-6). Sa règle existante « `origin/<branche>` absente ⇒ `Clean` » (`:1116-1128`)
couvre exactement cette population : la PR est terminale, donc la branche **a
été poussée**, donc une ref absente signifie que le distant l'a supprimée après
fermeture.

Et le chemin détaché obtient en prime une garantie que l'attaché n'a pas : **son
HEAD est, par construction de l'appariement, exactement le commit que GitHub a
enregistré comme tête de la PR** — il ne peut donc rien y avoir de non poussé.
La moitié `dirty` continue de protéger le travail **non committé**. À écrire au
site : c'est la raison pour laquelle ce chemin n'est pas une exception au
fail-safe, mais un cas où il est plus serré.

---

## 3. Conception — la disposition

`ReapCandidate` gagne deux champs :

```rust
pub struct ReapCandidate {
    pub path: String,
    pub branch: String,          // R-6 : de la PR sur le chemin détaché
    pub pr_number: u64,
    pub pr_state: String,
    pub pr_url: String,
    pub resolution: &'static str,   // RESOLUTION_BRANCH | RESOLUTION_DETACHED_SHA
    pub head_sha: Option<String>,   // Some(sha) sur le chemin détaché — la clé de jointure
}
```

`remove_worktree` :

- `git worktree remove --force <path>` — **inchangé** ;
- retrait du parent vide — **inchangé** ;
- `git worktree prune` — **inchangé** ;
- `git branch -D <branch>` — **sauté** quand `resolution == RESOLUTION_DETACHED_SHA`
  (R-5), `branch_deleted = false`.

Journal (`info!`, un seul site, `outcome_for(disposition)` inchangé) : les
champs existants **plus** `resolution` et `head_sha`.

Audit (`record_reaped`) : `tool_name` inchangé (R3), `after_value` inchangé
(les octets), et `reasoning` préfixé —

```
resolution=detached_sha head_sha=<40 hex> pr=2489 state=MERGED url=… branch=… bytes_reclaimed=… truncated=… disposition=…
```

`resolution=` **en tête** pour qu'un `reasoning LIKE 'resolution=detached_sha%'`
soit ancré et exact plutôt qu'une sous-chaîne flottante.

---

## 4. Effets de bord, tous nommés

### 4.1 Le sens de `detached_head` se resserre

Couvert par R-3. Daté dans le `CLAUDE.md` racine, halte H3 (§ 8).

### 4.2 Un worktree détaché de PR **terminale** entre dans la population du faucheur

C'est le ticket. Il traverse ensuite les sept termes complets.

### 4.3 Un worktree détaché de PR **ouverte** entre dans la population de mika#2497

`screen_target_purges` sélectionne **exactement** les refus `pr_open` du tick
(`:2206-2213`). Aujourd'hui un worktree détaché est refusé `detached_head`, donc
son `target/` n'est purgé **ni** par le faucheur **ni** par le bras de purge : il
vit indéfiniment. Après ce correctif il est refusé `pr_open` et devient
purgeable.

**C'est un élargissement voulu**, il ferme un trou de la même famille que celui
du ticket, et il est **gratuit en sûreté** : les cinq termes P1–P5 de mika#2497
s'appliquent inchangés, verrou de build compris. Il est épinglé par le contrôle
négatif N1 (§ 5), qui assère le motif `pr_open` — c'est-à-dire la donnée même qui
alimente ce bras. Et grâce à R-6, la ligne de purge porte désormais son
`pr_number` au lieu d'un trou.

### 4.4 Ce que ce travail n'achète PAS

- **Il n'explique pas le détachement** (R1) et ne l'empêche pas. Suivi F1.
- **Il ne rattrape pas les trois worktrees mesurés** : ils ont été purgés à la
  main le 2026-09-24. La sonde est la **prochaine** occurrence (§ 8, S1).
- **Il n'ajoute aucun compteur de « combien de worktrees sont détachés »** hors
  des motifs de refus existants : le faucheur ne journalise que ce qu'il refuse
  ou retire, et un scan qui journalise tout le monde ne distingue plus personne
  (doctrine mika#2131, déjà écrite au site `:1269-1271`).

---

## 5. Contrat de vérification

### V1 — Pré-vol, geste OPÉRATEUR, et sa halte

**Non exécutable dans le bac à sable de dispatch** (`gh` n'y est pas
authentifié — mesuré : `gh pr list --json …` rend *« To get started with GitHub
CLI, please run: gh auth login »* quelle que soit la requête). Le plan livre donc
le geste et sa conduite, il ne livre pas la mesure.

```bash
gh pr list --repo senara-solutions/mika --state all \
  --json number,state,headRefName,headRefOid,closedAt --limit 100 \
  | jq '.[] | select(.number == 2489 or .number == 2514 or .number == 2509)
        | {number, state, headRefName, headRefOid, closedAt}'
```

**Attendu :** `headRefOid` non vide et de 40 caractères hexadécimaux sur les
trois, dont les branches ont été supprimées.
**Halte V1 :** si `headRefOid` est vide ou absent sur cette population, **la
conception est inerte** — ne pas armer, ne pas élargir vers une dérivation de
chemin (R-7), et ouvrir le suivi F2 (route par stamp). L'inertie est **sûre** :
sans appariement, chaque worktree détaché est refusé
`detached_head_pr_unknown`, c'est-à-dire conservé comme aujourd'hui.

*Note : la disponibilité du champ sur `gh pr list --json` n'est pas en question —
elle est déjà exercée en production à `server/ci_success_handler.rs:617`. Ce que
V1 établit est sa **survie à la suppression de la branche**.*

### V2 — La seconde moitié n'est pas vérifiable avant déploiement, et c'est écrit

« Le HEAD du worktree détaché égale le `headRefOid` de sa PR » suppose que rien
n'a déplacé ce HEAD après le dernier push. Les trois worktrees témoins n'existent
plus ; **aucune mesure ne peut l'établir ici**. C'est la sonde S1 qui la mesure,
et son échec est **inerte, jamais dangereux** (aucun appariement ⇒ aucune
suppression).

### V3 — Tests comportementaux, chacun **vu rouge par mutation**

Discipline de la maison (mika#2420 : *« les sept termes ont été mutés un à un et
chacun a été observé rouge »*). Un contrôle négatif qui ne rougit pas sur le
retrait de son terme n'atteste rien.

**V3a — contrôle positif.** Worktree détaché, `HEAD` = `headRefOid` d'une PR
`MERGED` close depuis plus que la grâce, `Clean`, aucun processus dedans ⇒
**candidat**, `resolution == RESOLUTION_DETACHED_SHA`, `head_sha == Some(sha)`,
`branch == headRefName` de la PR.

**V3b — sept contrôles négatifs, un terme neutralisé à la fois.**

| # | fixture | attendu | ce que ça atteste |
|---|---|---|---|
| N1 | détaché, SHA d'une PR **OPEN** | refus `pr_open` | AC2 littéral + la porte d'entrée de mika#2497 (§ 4.3) |
| N2 | détaché, SHA lisible, **aucune** PR à ce SHA | refus `detached_head_pr_unknown` | la clé n'invente rien |
| N3 | détaché, `HEAD` absent / `000…0` / non hexadécimal / tronqué | refus `detached_head` | R-2, et le SHA nul **mesuré** en production |
| N4 | détaché, PR mergée, worktree **dirty** | refus `dirty` | T7 inchangé (AC3) |
| N5 | détaché, PR mergée **close depuis moins que la grâce** | refus `too_young` | T5 inchangé |
| N6 | détaché, PR mergée, **processus vivant** dedans | refus `live_process` | T6 inchangé |
| N7 | détaché, SHA sans PR, **branche homonyme existant par ailleurs** dans `by_branch` | refus `detached_head_pr_unknown` | AC2 mot pour mot : *« branche existante ailleurs »* ne suffit pas à faucher, et la clé SHA ne retombe pas sur la clé branche |

**V3c — anti-vacuité (R-8).** Un worktree **attaché** dont la branche n'a pas de
PR mais dont le HEAD apparie une PR mergée ⇒ refus `pr_unknown`, **pas** un
candidat. Sans ce test, « la clé SHA marche » serait indistinguable de « tout est
résolu par SHA ».

**V3d — non-régression du chemin attaché.** Le contrôle positif existant de
mika#2420 reste vert **sans modification de sa fixture**, et son candidat porte
`resolution == RESOLUTION_BRANCH` avec `head_sha == None`.

**V3e — pas de suppression de branche sur le chemin détaché (R-5).** Test unitaire
sur le prédicat de décision (`should_delete_local_branch(resolution)`), pas sur
`remove_worktree` (qui touche le disque).

**V3f — la surface d'audit.** `record_reaped` sur un candidat détaché : `tool_name
== REAPED_TOOL`, `reasoning` **commence par** `resolution=detached_sha`, contient
`head_sha=`, et `after_value` reste les octets. Miroir en `observe` :
`WOULD_DISPOSE_TOOL`, aucune ligne `worktree_reaped` (non-régression mika#2469).

### V4 — Tests de parsing

- `parse_worktree_registry` sur la **forme réelle mesurée** : entrée attachée
  (`HEAD` + `branch`), entrée détachée (`HEAD` + `detached`), entrée `prunable`
  (écartée), entrée `bare`.
- `usable_head_sha` à ses bornes : 40 hex ⇒ `Some` ; 39 / 41 / non hex / vide /
  `000…0` ⇒ `None`.
- `PrSnapshot` : un payload `gh` **sans** `headRefOid` parse quand même
  (`head_ref_oid == ""`) et n'apparie rien — **c'est R-4, et c'est le test qui
  garantit que le chemin attaché survit à l'absence du champ.**
- `PrIndex::build` : appariement insensible à la casse, une PR par SHA, deux PR
  partageant un SHA (§ 2.5).

### V5 — Tests de format de fil

- `mika2420_les_motifs_sont_un_format_de_fil` **étendu** avec les deux nouveaux
  motifs, en conservant son message d'échec (*« renommer un motif est une rupture
  de format de fil : la dater dans CLAUDE.md, jamais mettre ce test à jour en
  silence »*).
- `mika2518_les_resolutions_sont_un_format_de_fil` — **nouveau**, même forme :
  liste figée, unicité, et l'assertion que les deux valeurs diffèrent.

### V6 — `cargo test -p mika-agent` vert, `cargo clippy` sans avertissement,
`cargo fmt` appliqué.

---

## 6. Fire-Disposition

Ce plan livre **trois détecteurs** (code dont le chemin de succès est « aucune
violation trouvée »). Les tests comportementaux du § 5 V3/V4 n'en sont pas : leur
chemin de succès est « le verdict vaut X ».

**Option retenue : (a) exception nommée en allowlist — avec allowlist LIVRÉE
VIDE**, l'inventaire de violations existantes étant **nul et vérifié**, pas
supposé.

| détecteur | état à la livraison | inventaire | disposition quand il tire |
|---|---|---|---|
| `mika2420_les_motifs_sont_un_format_de_fil` (**étendu**) | **vert par construction** — il assère la liste que ce plan écrit | 0 | **dater le renommage dans le `CLAUDE.md` racine**, jamais mettre le test à jour en silence (son propre message d'échec le prescrit déjà) |
| `mika2518_les_resolutions_sont_un_format_de_fil` (**nouveau**) | **vert par construction** | 0 | idem |
| `mika2420_le_tool_name_daudit_a_un_seul_writer` (**inchangé**) | **vert**, allowlist vide et le reste | 0 — vérifié : `grep -rn "worktree_reaped\|worktree_reap_would_dispose" crates/ --exclude-dir=target` ne rend que `worktree_reaper.rs` et les `CLAUDE.md` | **retirer le second site**, jamais ajouter une entrée (doctrine mika#2201 : *« on déclare, on n'allowliste pas »*). Un second `tool_name` pour le chemin détaché **est** la violation que R3 refuse : ce scan est la garde structurelle de cette décision |

**Aucun détecteur n'est livré désarmé** (option b) : les trois sont verts au
moment de la livraison, donc aucun `#[ignore]` n'aurait de population à
attendre — et un détecteur désarmé sans violation à couvrir est une garde dont on
ne saura jamais si elle marche (classe mika#2205).

**Aucune halte-et-remontée** (option c) n'est nécessaire : aucune violation
préexistante n'a été trouvée.

**Refus explicite d'un quatrième détecteur.** Un scan de source « aucun site ne
dérive une PR depuis un chemin de worktree » serait séduisant (il garderait R-7 et
R2). Il est **refusé** : la classe a **zéro membre** aujourd'hui et aucune
population attendue, et livrer une garde sans population est le smell que la
maison nomme (« une garde qui n'a pas de population à mesurer »). Le refus est
écrit dans le doc-comment de la résolution, à l'endroit où un futur éditeur le
lira.

---

## 7. Documentation

### 7.1 `crates/mika-agent/CLAUDE.md` — § *Terminal-Worktree Reaper (mika#2420)*

Ajouter une sous-section **« Un HEAD détaché n'est pas un worktree sans PR
(mika#2518) »** portant :

- la rectification R1 (la cause écrite dans le ticket est réfutable ; le prédicat
  est agnostique du producteur ; l'enquête est le suivi F1) ;
- la rectification R2 (les deux mécanismes proposés par AC1 sont refusés sur
  mesure ; la clé est le `HEAD` ↔ `headRefOid`, exacte, et **l'argument de sûreté
  qui en découle** : apparier exactement le `headRefOid` signifie que le worktree
  est à l'état livré et pas un commit de plus) ;
- la rectification R3 (un seul `tool_name`, discriminant `resolution` ; pourquoi
  le motif `ready_label_outcome` et pas le motif `phantom_aged_out`) ;
- la rectification R4 (aucun réglage nouveau, et pourquoi un knob dédié est
  refusé) ;
- **la scission de vocabulaire datée** : `detached_head` se resserre,
  `detached_head_pr_unknown` naît, les lignes antérieures ne sont pas réécrites,
  une requête qui enjambe le déploiement doit sommer les deux ;
- l'effet de bord § 4.3 sur la population de mika#2497 ;
- la garantie T7 renforcée du chemin détaché (§ 2.6).

### 7.2 `CLAUDE.md` racine — § *Optional (terminal-worktree reaper — mika#2420)*

- la table des motifs gagne `detached_head_pr_unknown`, et la ligne
  `detached_head` est reformulée (« pas de branche attachée **et** pas de SHA
  exploitable — rare, anomalie git ») ;
- la ligne INFO `worktree_reaped` / `worktree_reap_would_dispose` gagne
  `resolution` et `head_sha` dans la liste de ses champs ;
- les sondes et haltes du § 8 ci-dessous ;
- la note de vocabulaire datée (R-3).

### 7.3 `docs/solutions/`

Une entrée sous `architecture-patterns/` : **« une clé de résolution exacte vaut
mieux qu'une dérivation, et l'appariement par SHA porte son propre argument de
sûreté »** — la leçon transportable étant que `HEAD == headRefOid` ne dit pas
seulement *quelle* PR, mais *que le worktree est à l'état livré*, ce qu'aucune
dérivation de chemin ne peut établir.

---

## 8. Surfaces opérateur et sondes

### SQL

```sql
-- Ce que la boucle a retiré, par clé de résolution (AC4)
SELECT CASE WHEN reasoning LIKE 'resolution=detached_sha%' THEN 'detached_sha'
            ELSE 'branch' END AS resolution,
       count(*), sum(CAST(after_value AS INTEGER))
  FROM audit_events WHERE tool_name = 'worktree_reaped' GROUP BY 1;

-- La distribution des refus — ATTENTION au vocabulaire scindé (R-3)
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'worktree_reap_skipped' GROUP BY 1 ORDER BY 2 DESC;

-- Ce qui SERAIT retiré, en observe
SELECT target_key, created_at, reasoning FROM audit_events
 WHERE tool_name = 'worktree_reap_would_dispose' ORDER BY created_at DESC;
```

### Journal (`$MIKA_SPIRIT_LOG_FILE`)

```bash
# 1. Les retraits par la clé SHA — c'est l'attribution de ce ticket
grep worktree_reaped "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.resolution == "detached_sha")
           | {worktree_path, head_sha, pr_number, pr_state, bytes_reclaimed}'

# 2. CONTRÔLE POSITIF — le bras tourne-t-il seulement ?
grep worktree_reap_tick "$MIKA_SPIRIT_LOG_FILE" | tail

# 3. La clé ne mord pas : SHA lisible, aucune PR appariée
grep worktree_reap_skipped "$MIKA_SPIRIT_LOG_FILE" | tail   # ou la requête SQL ci-dessus
```

| événement / motif | régime attendu | lecture |
|---|---|---|
| `worktree_reaped` avec `resolution=detached_sha` | **non vide après la première PR mergée dont la branche est supprimée** | chaque ligne est du disque rendu sans geste humain — l'attribution de ce ticket |
| `worktree_reaped` avec `resolution=branch` | **inchangé** | non-régression du chemin nominal |
| refus `detached_head_pr_unknown` | **faible** | worktrees hors boucle, ou HEAD déplacé après le dernier push |
| refus `detached_head` | **proche de zéro** | anomalie git : HEAD illisible ou nul |
| `worktree_reap_failed` | **vide** | inchangé |

### Sondes, et leurs haltes

**S0 — commencer en `observe`** (prescription de mika#2497, reprise telle
quelle). Poser `MIKA_WORKTREE_REAP_DISPOSITION=observe` et lire les lignes
`worktree_reap_would_dispose` portant `resolution=detached_sha`. **Le cap par
tick (`MIKA_WORKTREE_REAP_MAX_PER_TICK`, défaut 3) vaut aussi en observation** :
laisser tourner `ceil(N / 3)` ticks — jusqu'à ce qu'un tick ne nomme plus de
worktree que `SELECT DISTINCT target_key` n'ait déjà rendu — **puis** armer.
Armer après un seul tick retirerait les N−3 autres sans les avoir jamais vus en
dry-run.

**S1 — attribution (30 jours).** Au moins un `worktree_reaped` portant
`resolution=detached_sha`, sur un worktree de PR mergée à branche supprimée.
**C'est la sonde qui exécute V2.**
**Halte S1 —** `detached_head_pr_unknown` non vide **et** zéro
`resolution=detached_sha` : la clé SHA n'apparie rien. **Ne pas élargir vers une
dérivation de chemin** (R-7) ; établir d'abord si `headRefOid` survit à la
suppression de branche (V1) — si oui, le HEAD des worktrees a bougé après le
dernier push, et le remède est la route par stamp, suivi F2.

**S2 — contrôle négatif de l'anomalie (30 jours).** `detached_head` doit rester
proche de zéro. **Halte S2 —** un compte soutenu signifie des worktrees dont le
HEAD est illisible ou nul : c'est une anomalie git à établir **avant** de toucher
à `usable_head_sha` (l'élargir ferait entrer dans la population des worktrees
dont on ne sait pas où ils sont).

**S3 — non-régression (7 jours).** Le compte `resolution=branch` et la
distribution de `pr_open` / `pr_unknown` / `dirty` ne bougent pas.
**Halte S3 —** une baisse de `resolution=branch` signifie que la résolution par
branche a été détournée vers le SHA : c'est R-8 rompu, et V3c aurait dû rougir.

**S4 — symptôme (30 jours).** Plus aucun worktree de PR mergée ne survit sur
disque, et `/data` cesse d'accumuler cette population.
**Halte S4 —** le disque remplit encore alors qu'aucun worktree de PR terminale
ne subsiste : la cause est ailleurs — c'est la HALTE 2 de mika#2420 (le `target/`
des PR **ouvertes**), dont le remède est mika#2497 puis son propre suivi.

**HALTE 1 — un worktree portant du travail réel a été retiré.**
`touch ~/.mika/state/worktree-reap-stop` **immédiatement**, puis diagnostiquer :
la ligne d'audit porte `resolution` et `head_sha`, donc la décision est rejouable.
Un faux positif est irréversible et ne se règle pas en bougeant un seuil —
établir lequel des sept termes a lu vrai alors qu'il était faux. En `observe`, la
ligne symétrique est `worktree_reap_would_dispose` et la halte s'applique
identiquement, **à condition que S0 ait tourné assez longtemps** pour que toute
la population ait été nommée.

**HALTE 2 — aucune ligne du tout, ni retrait ni refus détaché.** On ne peut
**rien** conclure. Vérifier le contrôle positif (`worktree_reap_tick`), puis que
le binaire servi porte le correctif (classe mika#2340) — *une ligne absente ne
prouve rien tant qu'on n'a pas établi que le binaire qui tourne sait l'écrire*.

**HALTE 3 — une requête `GROUP BY after_value` enjambe le déploiement.** Les
lignes antérieures portent `detached_head` pour une population que le correctif
scinde en deux. **Sommer `detached_head` + `detached_head_pr_unknown`** de part et
d'autre, ou borner la requête au déploiement.

---

## 9. Hors périmètre, délibérément

- **F1 — le producteur du détachement.** R1 établit qu'il requiert un geste
  explicite et qu'aucun n'existe dans `dispatch-lib.sh`. Candidats à instruire :
  `scripts/mika-platform-worktree-cleanup` (dépôt `mika-platform`, hors de ce
  workspace), un `git update-ref -d` qui contourne la garde de `branch -D`, un
  rebase interrompu, une réécriture de `packed-refs`. **Préalable à l'ouverture :
  la sonde S1 non vide** — c'est-à-dire la preuve que la classe recommence — et
  la capture d'un worktree détaché **avant** sa fauche
  (`git -C <wt> reflog show HEAD | head`, lisible seulement depuis l'hôte).
  **Suivi à ouvrir.**
- **F2 — la route par stamp** (AC1, second mécanisme) : faire estampiller le
  numéro de PR par `_post_flight_recovery` sur le worktree. Refusée ici parce
  qu'elle ne couvre que les dispatches futurs (§ R2). **Préalable :** la halte S1,
  c'est-à-dire la preuve que la clé SHA n'apparie pas. **Suivi à ouvrir.**
- **Le `target/` des PR ouvertes** — mika#2497, population disjointe, élargie ici
  en effet de bord (§ 4.3) mais son mécanisme n'est pas touché.
- **La rétro-fauche des trois worktrees du ticket** — purgés à la main le
  2026-09-24. Rien ici ne les recrée, et inventer une ligne d'audit datée d'un
  événement qu'on n'a pas observé serait l'inverse de tout ce que ce travail
  défend.
- **Un knob, une disposition ou un kill-switch dédiés au chemin détaché** —
  refusés avec leur raison (R4).
- **La résolution du chemin attaché par SHA** — refusée (R-8), épinglée par V3c.
- **Toute modification de `screen_target_purges`, `apply_lock_probes`,
  `should_stop_repo_loop`, `probe_main_checkout`** — non touchés.
- **Le dédoublonnage des refus, la fenêtre de 24 h, la sentinelle STOP, le
  budget par tick** — inchangés.

---

## 10. Definition of Done

1. `WorktreeEntry.head` capturé par `parse_worktree_registry`, et `usable_head_sha`
   comme site unique de normalisation (§ 2.1).
2. `PrSnapshot.head_ref_oid` (`#[serde(default)]`, R-4) et `PrIndex` avec son
   site de construction unique (§ 2.2) ; `headRefOid` ajouté au `--json` de
   `list_prs`.
3. `RESOLUTION_BRANCH` / `RESOLUTION_DETACHED_SHA` / `ALL_RESOLUTIONS` (§ 2.3).
4. `REASON_DETACHED_HEAD_PR_UNKNOWN` ajouté à `ALL_REFUSAL_REASONS` (§ R-3).
5. T2 devient une résolution ; T3–T7 **inchangés** (§ 2.4) ; T7 reçoit le
   `headRefName` de la PR (§ 2.6, R-6).
6. `ReapCandidate` gagne `resolution` et `head_sha` ; `ReapRefusal.branch` est
   rempli depuis la PR sur le chemin détaché (R-6).
7. `remove_worktree` saute `git branch -D` sur le chemin détaché (R-5).
8. La ligne INFO gagne `resolution` et `head_sha` ; `record_reaped` préfixe
   `reasoning` par `resolution=` ; `tool_name` **inchangé** (R3).
9. V3a + les sept contrôles négatifs V3b, **chacun vu rouge par mutation de son
   terme**, plus V3c (anti-vacuité), V3d (non-régression attachée), V3e, V3f.
10. V4 (parsing, dont le payload `gh` sans `headRefOid`) et V5 (les deux tests de
    format de fil).
11. `cargo test -p mika-agent` vert, `cargo clippy` sans avertissement,
    `cargo fmt` appliqué.
12. Documentation § 7 : `crates/mika-agent/CLAUDE.md`, `CLAUDE.md` racine
    (motifs, champs, sondes, haltes, **note de vocabulaire datée**), une entrée
    `docs/solutions/`.
13. Le corps de PR porte : la rectification R1 (la cause du ticket est
    réfutable), le refus argumenté des deux mécanismes d'AC1 (R2), le refus d'un
    second `tool_name` (R3), la halte V1 avec son geste opérateur, et la
    prescription S0 (armer après un passage en `observe` complet).
14. **Aucune** variable d'environnement, **aucune** migration, **aucune**
    colonne, **aucune** valeur de réglage déplacée.

---

## Acceptance criteria

- **AC1** — Un worktree en HEAD détaché **dont la PR est MERGED ou CLOSED** est
  fauchable. La résolution passe par le **chemin du worktree** ou ses
  **métadonnées** (mapping worktree→PR/issue) quand la branche a disparu, pas par
  le nom de branche absent.

  *Servie par une troisième clé que l'AC n'énumère pas et qui satisfait sa
  clause négative (« pas par le nom de branche absent ») : l'égalité
  `HEAD du worktree == headRefOid de la PR`. Les deux mécanismes énumérés sont
  refusés sur mesure (§ R2) — la dérivation de chemin parce qu'elle est
  heuristique et lossy et qu'elle rouvre mika-platform#58, le stamp de
  métadonnées parce qu'il n'atteint pas la population déjà sur disque, qui est
  celle du ticket. Le SHA est exact là où les deux sont heuristiques, et porte en
  plus son propre argument de sûreté (§ R2, dernier paragraphe).*

- **AC2** — **Test négatif obligatoire** : un worktree en HEAD détaché **SANS**
  PR terminale (branche existante ailleurs, ou PR ouverte) reste **CONSERVÉ**. Le
  HEAD détaché seul ne suffit pas à faucher.

  *Servie par sept contrôles négatifs (§ 5, V3b), dont N1 (« PR ouverte » ⇒
  `pr_open`) et N7 (« branche existante ailleurs » ⇒
  `detached_head_pr_unknown`) couvrent les deux cas nommés mot pour mot. Chacun
  est vu rouge par mutation de son terme.*

- **AC3** — La disposition reste fail-safe vers *conserver* sur toute information
  illisible (PR non résolue, mapping absent) — un faux positif détruit du
  `target/` reconstructible, jamais du travail non poussé (T7 inchangé).

  *Servie structurellement : la résolution change la **clé**, jamais le prédicat
  (§ R-1). T7 n'est pas réécrit, et le chemin détaché en obtient une version plus
  serrée (§ 2.6). Les quatre nouvelles formes d'illisibilité sortent toutes de la
  population (§ R-2), y compris le `headRefOid` absent, dont la dégradation est
  bornée au nouveau chemin (§ R-4).*

- **AC4** — Observabilité : le nouveau chemin émet un motif distinct (p. ex.
  `reaped_detached_merged`) séparable de `worktree_reaped` nominal.

  *Servie par un champ `resolution` (`branch` | `detached_sha`) sur la ligne INFO
  et en tête de `audit_events.reasoning`, sous le `tool_name` **inchangé**. Le
  « p. ex. » est rectifié (§ R3) : un second `tool_name` tronquerait en silence
  `SELECT … WHERE tool_name = 'worktree_reaped'`, qui est le garde-fou 3 de
  mika#2420 et une requête publiée. Le motif retenu est celui de
  `ready_label_outcome` (mika#2323), pas celui de `phantom_aged_out` (mika#2156),
  et le § R3 dit lequel s'applique quand.*
