# `pr_unknown` cesse de confondre deux populations, et l'index cesse de s'arrêter à une borne mobile (mika#2482)

> Ticket : senara-solutions/mika#2482 — `enhancement`, `p3-nice-to-have`,
> `agent-core`, `infrastructure`, `dispatch:loop`. Zéro commentaire.
> Fichier visé : `crates/mika-agent/src/worktree_reaper.rs` (6952 lignes).

---

## 1. Ce que la lecture du code rectifie du ticket — premier livrable

Le ticket est un dormeur soigné, et ses deux constats sont réels. Cinq mesures
faites le 2026-09-29 déplacent néanmoins ses pistes, et chacune change le remède.

### R1 — la borne de pagination n'est pas un dimensionnement, c'est une fuite

| date | `min(number)` de `gh pr list --state all --limit 300` |
|---|---|
| 2026-09-22 (ticket) | **#1917** |
| 2026-09-29 (mesuré) | **#2067** |

**150 PR en 7 jours ⇒ la borne recule d'environ 21 numéros par jour.** Deux des
trois pistes du ticket ne ferment donc rien : doubler `LIST_LIMIT` à 600 achète
~28 jours puis le défaut revient ; paginer coûte O(total PR) à chaque tick,
indéfiniment croissant, pour un gain qui ne concerne qu'une poignée de worktrees
non résolus. **Le défaut est structurel, pas dimensionnel.**

La première piste, elle, est **vérifiée par mesure** :

```
$ gh pr list --repo senara-solutions/mika --state all \
    --head feat/1888/research-mika-dev-confidence-high-low \
    --json number,state,headRefName,headRefOid,closedAt,url
[{"number":2011,"state":"MERGED",...},{"number":1900,"state":"CLOSED",...}]
```

`#1900` est **hors** de la fenêtre (min = 2067) et la requête la rend. Mieux :
**mêmes champs, même schéma, donc `Vec<PrSnapshot>` se désérialise sans une ligne
de structure nouvelle.** Une branche sans PR rend `[]`, pas une erreur.

### R2 — le constat 1 a deux moitiés, et une seule est une décision de conception

La condition de réveil que le DoD nomme est *« `/data` refranchit régulièrement
80 % »*. Ce qui coûte ce disque n'est pas le worktree, c'est son `target/` — 15 à
50 Go par pilote. Or **mika#2497 purge déjà exactement cela**, avec cinq termes
fail-safe et un verrou de build, et son filtre de population est littéralement :

```rust
if refusal.reason != REASON_PR_OPEN { continue; }   // l. 2535 et l. 3117
```

L'asymétrie écrite qui autorise ce bras — *« le faucheur supprime du travail
potentiel, ce bras supprime du dérivé pur »* — est **indifférente à la raison
pour laquelle le worktree est conservé**. Un `target/` de `pr_unknown` est
exactement aussi reconstructible qu'un `target/` de `pr_open`. La moitié disque
du constat 1 ne demande donc **aucune règle N-jours** : elle demande d'élargir
une population de deux noms à trois.

La moitié qui reste — *« ce worktree est vieux et n'a jamais produit de PR »* —
est de l'**observabilité**, et personne ne l'a.

### R3 — la fauche d'un `pr_unknown` est refusée, et pas seulement par prudence

Le ticket l'écrit (« pas fauche auto : l'issue est vivante »). Le code va plus
loin : un `pr_unknown` **sort à T3**, donc **T7 (`dirty` / `unpushed_commits`)
n'est jamais évalué sur lui**. On ne sait pas s'il porte du travail non poussé.
Le faucher détruirait du travail sous un prédicat qui n'a pas regardé. Donc :
jamais de fauche, jamais de « parking », seulement (a) le dérivé et (b) le
signalement. Cette ligne du ticket n'est pas une précaution, c'est une
contrainte.

### R4 — le « N jours » doit dater le WORKTREE, pas l'issue

Le ticket propose « issue toujours ouverte ». Refusé, deux motifs :

1. **Coût sans emploi** — lire l'état de l'issue coûte un `gh` par worktree et
   par tick pour une information dont **aucune décision ne dépend** (rien n'est
   fauché, cf. R3).
2. **Le filtre va dans le mauvais sens** — un worktree dont l'issue est *fermée*
   sans PR est **encore plus** un candidat au signalement. Filtrer sur « ouverte »
   rétrécirait la population visée.

La date lisible sans réseau, et qui répond à la question réellement posée
(« rien ne progresse ici »), est **la date du dernier commit de la branche**
(`git log -1 --format=%cI`, via le `run_git` déjà présent). Un worktree repris
par `_set_up_worktree` est rebasé, donc sa date remonte — ce qui est correct : la
boucle l'a repris.

### R5 — le signalement ne doit PAS être un nouveau motif de refus

Tentant : ajouter `REASON_PR_UNKNOWN_STALE`. **Refusé** — cela scinderait la
population `pr_unknown` en deux noms et casserait, en silence, les `GROUP BY`
publiés dans le `CLAUDE.md` racine. C'est la scission datée dont mika#2361 a dû
écrire le coût (`operator_review_or_blocked` → `abandoned_operator_held`) et que
mika#2518 a refusée sur son propre axe. Un **événement distinct à côté** du
refus : `pr_unknown` garde son sens intact, et les deux populations restent
soustractibles.

### R6 — la composition qui fait tenir l'ensemble

Un `pr_unknown` **récent** est nominal (« groomé, PR imminente ») : sa PR entrera
dans l'index de masse dès qu'elle existera, et la sonder serait un appel `gh`
pour rien, 144 fois par jour. Un `pr_unknown` **vieux** est soit un abandon
(constat 1), soit un faux `pr_unknown` par angle mort (constat 2).

> **Une seule sonde tranche les deux constats, et son coût est borné par la
> petitesse de la population vieille.**

C'est ce qui fait de ce plan un mécanisme et non deux.

---

## 2. Requirements

### B1 — la purge `target/` couvre `pr_unknown` (ferme la condition de réveil)

- **B1.1** — Une constante unique `PURGE_ELIGIBLE_REASONS: &[&str] = &[REASON_PR_OPEN, REASON_PR_UNKNOWN]`
  et un prédicat unique `is_purge_eligible_reason(&str) -> bool`, lus par les
  **deux** sites (`purge_stale_target_dirs` l. 3117 pour le calcul des `states`,
  `screen_target_purges` l. 2535 pour la décision).
- **B1.2** — Les deux sites **doivent** bouger ensemble. Élargir la décision sans
  le calcul d'états fait tomber les nouveaux worktrees sur
  `unwrap_or(TargetState::Present { idle_secs: None })`, c'est-à-dire **conserve
  tout** : un bras qui se lit comme élargi et ne purge rien (classe mika#2205).
  Un scan de source refuse un troisième site lisant `REASON_PR_OPEN` dans un
  filtre de population de purge.
- **B1.3** — Aucun des cinq termes P1–P5 ne bouge. Le seul danger de ce bras est
  la **concurrence** avec un `cargo build`, et P5 (verrou, mika#2511) la couvre
  identiquement sur la population élargie.
- **B1.4** — `pr_number` vaut naturellement `None` sur un `pr_unknown`
  (`by_branch` rend `None`) : sémantique exacte, **zéro ligne à changer**, et
  `null` n'est jamais `0` (mika#2331).

### B2 — la sonde ciblée `--head` ferme l'angle mort de pagination

- **B2.1** — Seconde passe, **après** `screen_worktrees`, sur les refus
  `pr_unknown` **stale uniquement** (R6). `gh pr list --repo <r> --state all
  --head <branche> --json number,state,headRefName,headRefOid,closedAt,url`,
  désérialisé en `Vec<PrSnapshot>` — **aucune structure nouvelle** (R1).
- **B2.2** — PR trouvée ⇒ le worktree est re-screené sous la **même conjonction
  de sept termes**, sans exception ni assouplissement. La provenance de la PR ne
  change pas sa vérité.
- **B2.3** — Troisième valeur de résolution, `RESOLUTION_BRANCH_PROBE =
  "branch_probe"`, ajoutée à `ALL_RESOLUTIONS`. **Format de fil**, épinglé comme
  ses deux aînées. `reasoning LIKE 'resolution=branch_probe%'` compte exactement
  les worktrees que l'angle mort aurait conservés à tort — la sonde d'attribution
  du constat 2, sur le modèle de `detached_head_pr_unknown` (mika#2518).
- **B2.4** — Sonde en échec (timeout, `gh` non nul, parse KO) ⇒ **conserve**,
  sous un motif nommé, et **ne signale pas** : on ne sait rien. Fail-safe maison,
  direction *conserver*, comme tout ce module.
- **B2.5** — Cap par tick (`MIKA_WORKTREE_STALE_PROBE_MAX_PER_TICK`, défaut `3`)
  **plus** la déduplication 24 h partagée avec B3 : au plus **un appel `gh` par
  worktree stale et par jour**. Population mesurée le 22/09 : 6.

### B3 — le signal `worktree_stale_no_pr`

- **B3.1** — `audit_events.tool_name = "worktree_stale_no_pr"`, **SOLE WRITER**,
  dédupliqué 24 h par `REFUSAL_DEDUP_SECS` (déjà présent). Ligne INFO du même
  nom. **Aucun motif de refus n'est ajouté, renommé ni retiré** (R5).
- **B3.2** — Champs : `worktree_path`, `branch`, `issue` (2ᵉ segment de la
  branche ; `None` si non conforme — **jamais inventé**), `branch_idle_days`,
  `probe` ∈ `{no_pr, unreadable}`.
- **B3.3** — Datation par `git log -1 --format=%cI HEAD` dans le worktree (R4).
  Date illisible ⇒ **ne signale pas**, sous motif nommé.
- **B3.4** — `MIKA_WORKTREE_STALE_DAYS`, défaut **7**, trois paliers maison
  (absent/vide → défaut ; illisible, `0` ou négatif → défaut + `warn!` nommant la
  valeur **entre guillemets**). Le `0` **ne désarme pas** : sur un scan qui
  déclenche une suppression de dérivé, une coquille ne doit pas être un
  désarmement silencieux.
- **B3.5** — **Rien n'est supprimé par B3.** C'est de l'observabilité.

### Justification du seuil `7` jours

| borne | argument |
|---|---|
| plancher | la vie nominale d'un `pr_unknown` est de l'ordre de l'heure à la journée, et un `ready` abandonné est borné à trois re-drives (mika#2020) — 7 j laisse un ordre de grandeur |
| plafond | la population mesurée le 22/09 va de ~3 semaines (`bug/2260`, `test/2266`) à ~84 jours (`incident/1696`, issue du 30/06) — elle est intégralement attrapée |
| asymétrie | un faux « stale » coûte une ligne de journal et **un** appel `gh`/jour ; un faux « pas stale » laisse le worktree invisible un jour de plus. Le seuil peut donc être généreux |

---

## 3. Fire-Disposition

Ce plan livre des détecteurs : deux scans de source et un jeu de tests dont le
chemin de succès est « aucune violation ». Disposition retenue :
**(a) exception nommée en allowlist — allowlists livrées VIDES.**

| détecteur | allowlist | contrôle |
|---|---|---|
| `mika2482_le_tool_name_stale_a_un_seul_writer` (SOLE WRITER de `worktree_stale_no_pr`) | `STALE_WRITER_ALLOWED: &[&str] = &[]` | test sœur épinglant qu'elle reste vide, plus une assertion d'anti-vacuité (le nom doit être écrit **quelque part**, sinon le scan vise un nom mort et se lit propre — classe mika#2205) |
| `mika2482_le_predicat_deligibilite_de_purge_a_un_site_unique` (aucun second site ne filtre sur `REASON_PR_OPEN` dans une population de purge) | `PURGE_REASON_FILTER_ALLOWED: &[&str] = &[]` | idem, plus un **contrôle négatif** (fixture portant un second filtre → le scan doit rougir) |

Les deux scans sont **vus verts sur l'arbre actuel avant merge**. Quand l'un
tire, la résolution est de **router le site vers le prédicat unique**, jamais
d'ajouter une entrée (doctrine mika#2201, reprise du scan SOLE WRITER déjà
présent dans ce fichier l. 6108 : *« livrée vide, et elle le reste »*).

Aucun détecteur n'est livré désarmé : aucun des deux n'a de population préexistante.

---

## 4. Acceptance criteria

Dérivés des Requirements et du DoD du ticket (le corps de l'issue ne porte pas de
section `## Acceptance criteria`).

- **AC1** — Le `target/` d'un worktree refusé `pr_unknown`, inactif au-delà de la
  fenêtre et sans verrou de build, est purgé exactement comme celui d'un
  `pr_open` : mêmes cinq termes, même disposition, même cap, même surface
  opérateur, `pr_number` à `null`.
- **AC2** — Un worktree dont la PR est **mergée et hors de la fenêtre de 300**
  est résolu par la sonde ciblée, re-screené sous les sept termes, et fauché s'il
  les passe. Sa ligne porte `resolution=branch_probe`.
- **AC3** — Un worktree stale **réellement sans PR** produit exactement une ligne
  `worktree_stale_no_pr` par 24 h, **n'est pas fauché**, et son motif de refus
  reste `pr_unknown` inchangé.
- **AC4** — Un worktree `pr_unknown` **récent** (branche mue dans les `N` jours)
  ne déclenche **aucun** appel `gh` supplémentaire et **aucune** ligne de signal.
- **AC5** — Toute lecture illisible (sonde en échec, date de commit illisible,
  `gh` en timeout) **conserve** et **ne signale pas**, chacune sous un motif
  distinct et nommé.
- **AC6** — `ALL_REFUSAL_REASONS` est **inchangé** : aucun motif ajouté, renommé
  ni retiré ; les `GROUP BY` publiés dans le `CLAUDE.md` racine restent exacts de
  part et d'autre du déploiement.
- **AC7** — `ALL_RESOLUTIONS` gagne `branch_probe` et rien d'autre ; les deux
  valeurs existantes sont intactes.
- **AC8** — Les trois nouvelles clés d'environnement suivent les trois paliers
  maison, `0` ne désarmant aucune d'elles.

---

## 5. Verification Contract

### Tests comportementaux (`#[cfg(test)]`, dans le module)

| # | ce qui est établi | forme |
|---|---|---|
| V1 | AC1 — un refus `pr_unknown` entre dans la population de purge et en sort purgé | `screen_target_purges` sur un `ReapRefusal{reason: pr_unknown}` + `target/` vieux ⇒ candidat |
| V2 | **Contrôle négatif de V1** — un refus `dirty` ou `too_young` n'entre **pas** | sans lui, « la population est élargie » est indistinguable de « la population est devenue tout le monde » |
| V3 | AC1 — `pr_number` vaut `None` sur cette population | index sans la branche ⇒ `None`, jamais `0` |
| V4 | AC2 — une PR mergée rendue par la sonde produit un candidat `resolution=branch_probe` | ré-injection dans `screen_worktrees` avec un index enrichi |
| V5 | **Contrôle négatif de V4** — la même PR **ouverte** rend `pr_open`, et **dirty** rend `dirty` : les sept termes s'appliquent sans assouplissement | c'est la garantie de sûreté de B2.2, et elle ne se démontre pas autrement |
| V6 | AC3 — une seconde passe dans les 24 h n'écrit pas de seconde ligne ; au-delà, si | dédup, motif `record_refusal` |
| V7 | AC4 — un `pr_unknown` récent ne figure ni dans la population de sonde ni dans le signal | **anti-vacuité** : sans lui, « la sonde est ciblée » et « la sonde est morte » se lisent pareil |
| V8 | AC5 — les trois illisibles conservent et ne signalent pas | trois cas, trois motifs distincts |
| V9 | AC8 — trois paliers × trois clés, `0` ne désarme pas | motif `parse_positive_i64` existant |
| V10 | AC6/AC7 — les deux formats de fil sont figés | extension des tests `…_sont_un_format_de_fil` existants |

### Scans structurels

Les deux de la § 3. Un test comportemental ne peut pas voir leur classe : un
second site de filtre ne rend **aucune décision fausse** le jour où il est écrit
— il rend simplement une population plus étroite que ce que la constante dit, en
silence, toutes les autres assertions restant vertes.

### Contrôle négatif du fixture

Le scan `purge_reason_filter` doit être **vu rouge** sur une fixture portant un
second filtre, et **vu vert** sur l'arbre. Un scan qu'on n'a jamais vu rougir ne
prouve rien.

### Ce qui n'est PAS testable ici, écrit plutôt que découvert

**La mesure de la population réelle est structurellement hors de portée du
pilote.** Mesuré dans ce bac à sable :

```
$ ls /data/workspace/mika-platform/.claude/worktrees
feat-2482-worktree-reaper-pr-unknown-confond          # une seule entrée

$ git worktree list --porcelain
worktree /data/workspace/mika-platform/mika
HEAD 0000000000000000000000000000000000000000
detached
worktree .../feat-2482-worktree-reaper-pr-unknown-confond/mika
```

bwrap ne monte que le worktree courant, et `~/.mika/data/mika.db` n'est pas
montée. Les 13 `pr_unknown` du 22/09 sont invisibles d'ici, et aucune requête
`audit_events` n'est exécutable. **Toutes les sondes de la § 7 sont
post-déploiement, sur l'hôte.** (Note incidente : le checkout principal apparaît
avec un `HEAD` nul — exactement le cas que le doc-comment de
`REASON_DETACHED_HEAD` dit avoir mesuré en production.)

---

## 6. Surfaces opérateur

```bash
# 1. B1 — des `target/` de worktrees sans PR sont-ils purgés ?
grep target_purged "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.pr_number == null) | {worktree_path, branch, idle_secs, bytes_reclaimed}'

# 2. B3 — quels worktrees sont vieux et sans PR ? (CONTRÔLE POSITIF de la sonde)
grep worktree_stale_no_pr "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{worktree_path, branch, issue, branch_idle_days, probe}'
```

```sql
-- 3. B2 — combien de worktrees l'angle mort aurait-il conservés à tort ?
SELECT count(*) FROM audit_events
 WHERE tool_name = 'worktree_reaped' AND reasoning LIKE 'resolution=branch_probe%';

-- 4. B3 — la file vieille, datée et comptable
SELECT target_key, created_at FROM audit_events
 WHERE tool_name = 'worktree_stale_no_pr' ORDER BY created_at DESC;

-- 5. AC6 — la distribution des motifs, INCHANGÉE de part et d'autre du déploiement
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'worktree_reap_skipped' GROUP BY 1 ORDER BY 2 DESC;
```

| surface | régime attendu | lecture |
|---|---|---|
| `target_purged` avec `pr_number: null` | **non vide** après déploiement | chaque ligne est du disque rendu que ni le faucheur ni le bras ne rendaient |
| `worktree_stale_no_pr` | **non vide, faible et stable** (~6 mesurés le 22/09) | la file groomée-jamais-implémentée, enfin nommée. Croissance soutenue ⇒ le grooming produit plus que l'implémentation ne consomme |
| `resolution=branch_probe` | **rare** | chaque ligne est un faux `pr_unknown` fermé. Zéro ⇒ voir Halte 2 |
| `worktree_stale_probe_unreadable` | **vide** | toute occurrence est une sonde qui n'a pas su regarder |
| `worktree_reap_skipped` / `pr_unknown` | **inchangé en sens** | AC6 : la scission a été refusée pour que cette requête reste exacte |

---

## 7. Sondes post-déploiement, et leurs quatre haltes

> **Préalable.** Ces sondes lisent l'hôte. Établir d'abord que le binaire servi
> porte le correctif — une ligne absente ne prouve rien tant qu'on n'a pas établi
> que le binaire qui tourne sait l'écrire (classe mika#2340).

**S1 — B1 mord (48 h).** La requête 1 est non vide, et `/data` cesse de
retrouver 80 % au rythme observé.
*Halte 1 —* si des purges surviennent sur des worktrees **fraîchement groomés**,
lire `idle_secs` **avant** de rallonger la fenêtre : un worktree qui vient d'être
groomé n'a pas de `target/` du tout (P2 le refuse), donc une purge là signifie
qu'un `target/` est réellement resté inactif au-delà de la fenêtre — ce qui est
le contrat, pas un défaut.

**S2 — B2 mord (30 jours).** La requête 3 rend au moins une ligne.
*Halte 2 —* **zéro ligne ne prouve rien** tant que le contrôle positif n'est pas
établi : il faut qu'une sonde ait réellement tourné, et c'est la requête 2 qui le
dit (une ligne `worktree_stale_no_pr` prouve qu'une sonde a tourné et rendu « pas
de PR »). Zéro des deux ⇒ la population stale est vide, ce qui est un **résultat**
et non une panne. *Une garde que personne n'a exercée se lit exactement comme une
garde qui marche* (mika#2205).

**S3 — contrôle négatif de bruit (7 jours).** Aucun `worktree_stale_no_pr` sur un
worktree dont la branche a bougé dans les `N` jours.
*Halte 3 —* une occurrence signifie que la datation lit autre chose que ce qu'on
croit (un `fetch` qui touche un mtime, une branche rebasée non détectée) :
**réparer la datation, pas relever le seuil.**

**S4 — AC6, la non-régression des comptes.** La requête 5 doit garder le même
vocabulaire de part et d'autre du déploiement.
*Halte 4 —* l'apparition d'une valeur nouvelle signifie qu'un motif a été ajouté
contre R5 : **désarmer d'abord** (`MIKA_TARGET_PURGE=0` puis revert), les
requêtes publiées du `CLAUDE.md` racine étant cassées en silence pendant ce temps.

**Halte transverse — la file stale croît sans borne.** Si la requête 4 grossit de
semaine en semaine, le remède n'est **pas** dans ce module : c'est que le
grooming produit plus de worktrees que l'implémentation n'en consomme. Ouvrir le
suivi **avec ce compte**, jamais avec une intuition.

---

## 8. Definition of Done

- Les trois briques livrées, `cargo test -p mika-agent` vert, `cargo clippy` propre.
- Les deux scans structurels **vus verts sur l'arbre** et **vus rouges sur leur
  fixture**, allowlists vides et pinnées vides.
- Les trois clés d'environnement documentées dans le `CLAUDE.md` racine, dans la
  section du faucheur, avec leurs surfaces opérateur, leurs régimes attendus et
  leurs haltes — **et la limite de la § 5 écrite** (la population n'est pas
  mesurable depuis un pilote).
- Le corps de PR nomme les deux constats du ticket, ce qui est fermé, et ce qui
  ne l'est pas (§ 9).

---

## 9. Ce que ce travail n'achète PAS

- **Il ne fauche aucun `pr_unknown`.** Le ticket l'interdit, et l'asymétrie aussi
  (R3 : T7 n'est jamais évalué sur cette population). Ce qui est retiré est le
  `target/` — du dérivé pur — jamais le worktree, jamais une branche, jamais un
  commit.
- **Il ne ferme aucune issue et ne réveille aucun ticket.** Le signal dit « ce
  worktree est vieux » ; décider quoi en faire est un geste d'opérateur.
- **Il ne borne pas la production.** Si N pilotes compilent simultanément, aucun
  n'est stale et rien n'est purgé pendant la montée — limite héritée de
  mika#2497, inchangée, et c'est sa Halte 3.
- **Il ne rend pas la pagination exacte.** L'index de masse reste borné à 300 ;
  ce qui est ajouté est un rattrapage **ciblé** sur les non-résolus vieux. Un
  faux `pr_unknown` **récent** — PR hors fenêtre **et** branche mue dans les 7
  jours — reste invisible. Combinaison improbable (une PR hors fenêtre a des
  semaines) et **nommée plutôt que masquée**.
- **Aucune mesure n'est faite par ce plan sur la population réelle** (§ 5). Les
  chiffres cités viennent du ticket (22/09) et de `gh` (29/09).

---

## 10. Hors périmètre, délibérément

| écarté | raison |
|---|---|
| augmenter ou paginer `LIST_LIMIT` | mesuré : la borne recule de ~21/jour, doubler achète 28 jours (R1) |
| lire l'état de l'issue | coût réseau pour une information dont aucune décision ne dépend, et filtre dans le mauvais sens (R4) |
| un nouveau motif de refus pour le stale | casse les `GROUP BY` publiés, scission datée à la mika#2361 (R5) |
| fauche d'un `pr_unknown`, même gated-opérateur | exclu par le ticket **et** par l'absence d'évaluation de T7 (R3) |
| `CARGO_TARGET_DIR` partagé | déjà refusé par mika#2497 sur trois motifs mesurés ; à rouvrir avec une mesure |
| un hook `pull_request.closed` | tranché par mika#2420 : scan seul, quatre raisons |
| retirer `#2482` de `docs/dormeurs.md` | geste d'orchestrateur, signalé par le corps de PR |
