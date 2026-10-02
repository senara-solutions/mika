# `run_gh pr ready` refuse un hold explicite (mika#2624)

> **Ticket :** senara-solutions/mika#2624 — Tier 1, casse la boucle.
> **Lignée :** mika#2597 (même invariant, côté `wip_rescue`), mika#1682 (le garde
> actuel), mika#2573 (le garde frère le plus proche au même site).

---

## Le défaut, mesuré (n=1, 2026-10-01)

samidarko met PR #2621 en brouillon à 14:00:41Z — un hold délibéré, posé pour
qu'une revue de code ait lieu avant le merge. À 14:46:03Z l'agent **mika-dev**
appelle `run_gh` avec l'argv `["pr", "ready", "2621"]`. Le hold saute
(`ReadyForReviewEvent` par `mika-platform-dev`, 14:45:59Z).

La séquence, lue sur les lignes `run_gh invocation` de `/var/log/mika/server.log` :

```
14:45:49 mika-dev ["pr","view","2621","--json","state,isDraft,mergeable,reviewDecision,url"]
14:45:53 mika-dev ["pr","view","2621","--json","body","--jq",".body[:200]"]
14:46:00 mika-dev ["pr","diff","2621","--name-only"]
14:46:03 mika-dev ["pr","ready","2621"]
```

mika-dev **lit** `isDraft=true`, puis sort la PR du brouillon. Au même instant une
revue par spawn trouvait dans cette PR un **P1 de perte de données** (corrigé en
`e85d5a81`). Sans cette revue, la PR était hors brouillon, approuvée et verte :
mergeable par le moteur avec le défaut dedans. **Le merge autonome d'un défaut n'a
été évité que par coïncidence de calendrier.**

Déclencheur (précision opérateur, trace `bf4c8580-bda6-11f1-8806-eef2949337cb`) :
le tour mika-dev démarre à 14:45:44Z, dans la même seconde que l'événement
« CI success but VERDICT: pass was for a different SHA — stale verdict ».

---

## Ce que la lecture du code déplace dans le ticket — premier livrable

Sept mesures. Trois changent le remède, quatre ferment des branches d'AC4 par
exclusion plutôt que par un second branchement.

### R1 — Le discriminant est déjà une fonction appelable ; ce qui manque est une classification extractible

`github_graphql::fetch_convert_to_draft_events(token, owner, repo, number)` est
`pub(crate)` et rend `Result<Vec<ConvertToDraftEvent>, String>`
(`crates/mika-agent/src/github_graphql.rs:401`). Son extracteur
`extract_convert_to_draft_events` est **déjà fail-closed sur la forme**, et son
doc-comment écrit pourquoi, mot pour mot : *« answering "no hold event" would be
the one reading that lets the daemon un-draft a PR a human deliberately held »*.

Mais la **classification** `Vec<ConvertToDraftEvent> → HoldVerdict` est inline
dans `wip_rescue::hold_verdict` (`wip_rescue.rs:972`), qui est **privée** et
couplée à trois choses dont `run_gh` ne dispose pas sous cette forme :
`AsyncDatabase` + `trace_id` pour le `report_hold`, et `default_repo_parts()`
codé sur `senara-solutions/mika`.

AC1 exige « factorisé ou appelé, jamais recopié ». La factorisation nécessaire
est donc **étroite et unique** : extraire la classification pure, laisser le
fetch et le report où ils sont.

### R2 — `detect_ready_promote_pr` couvre DEUX formes, et le nouveau terme ne doit en couvrir qu'UNE

`detect_ready_promote_pr` (`builtin_handlers.rs:2482`) rend `Some(pr)` pour :

- `pr ready <N>` sans `--undo` — **sort du brouillon** ;
- `pr edit <N> --title <T>` — **renomme**, et ne touche pas l'état draft.

Le second est dans le périmètre de mika#1682 parce que la promotion wip-rescue a
deux gestes (sortir du brouillon *et* renommer `wip(…)` → `fix(…)`). Il n'est
**pas** dans le périmètre de mika#2624 : AC1 ne parle que de `pr ready`, et un
hold n'interdit pas de corriger un titre. Réutiliser le détecteur tel quel
produirait un refus sur un renommage de titre d'une PR tenue — hors AC, et un
faux positif sur un geste inoffensif.

### R3 — Le handler stale-verdict n'émet AUCUN prompt : il n'y a pas de prescription à corriger

Le commentaire opérateur prévoit : *« Si le prompt du handler stale-verdict
demande ou suggère explicitement `pr ready`, il faut aussi le corriger. »* La
lecture le réfute. Le chemin stale-verdict (`server/ci_success_handler.rs:376`)
rend `VerdictAction::Passthrough { enrichment: None }` : le moteur ne dit **rien
du tout**, le modèle reçoit le texte brut du webhook. C'est la forme exacte que
mika#1745 a dû nommer. `pr ready` n'a été soufflé par personne.

**Mais il y a bien une moitié intention à corriger, et elle est pire qu'un
silence.** Quatre prompts de handler webhook portent, à la virgule près, la même
phrase — dont celui du tour fautif (`self-dev-webhook-ci/system_prompt.md:58`) :

> Do NOT call `gh pr ready` or `gh pr edit --title` on any PR **matching the
> wip-rescue signature**: `wip-rescue` label OR head commit starts with `wip(`.

Elle interdit par **délimitation**, donc elle **permet par contraste** : hors
signature wip-rescue, le prompt dit au modèle que `pr ready` est à lui. Le modèle
a lu `isDraft=true`, vérifié qu'aucune signature wip-rescue n'était là, et conclu
correctement au regard de son prompt. Les quatre sites :
`self-dev-webhook-ci:58`, `self-dev-webhook-qa:326`,
`self-dev-webhook-ready-label:67`, `self-dev-callback:189`.

### R4 — Le chemin `gh api` est déjà structurellement fermé

`GH_API_ALLOW_MATRIX` (`builtin_handlers.rs:2040`) est une **allowlist fermée**
de six entrées, toutes REST : `GET branches`, `GET branches-list`,
`GET commits/<sha>`, `GET milestones/<n>`, `PATCH milestones/<n>`,
`GET /advisories`. `validate_gh_api_scope` refuse tout ce qui ne matche pas.
Donc :

| argv tenté | `path` extrait | verdict |
|---|---|---|
| `gh api graphql -f query='mutation{markPullRequestReadyForReview…}'` | `graphql` | **refusé** (aucune entrée) |
| `gh api repos/o/r/pulls/N --method PATCH -f draft=false` | `/repos/o/r/pulls/N` | **refusé** (aucune entrée `pulls`) |

AC4 est donc satisfaite pour cette branche **par exclusion mesurée**, et c'est le
bon sens de la garantie : une allowlist fermée n'a pas besoin d'un second
discriminant, et lui en ajouter un donnerait l'illusion que la fermeture vient de
là.

### R5 — `run_shell` est déjà fermé, et il l'est contre les contournements

`crates/mika-agent/templates/skills/shell-exec/handlers/run.sh:45` refuse `gh`
nommément, et sa ligne 78 refuse les routes indirectes (sous-shell, préfixe de
chemin, substitution). Hors population, par un refus antérieur.

### R6 — Contrainte dure du scan mika#2597, à connaître AVANT d'écrire une ligne

`wip_rescue::tests::mika2597_un_seul_site_dundraft_en_production` exige
**exactement un** site de production portant le motif normalisé `"pr","ready"`
dans `crates/mika-agent/src/`, avec `UNDRAFT_SITES_ALLOWED` livrée vide et une
assertion qui refuse qu'elle cesse de l'être. Le scan nomme déjà notre voisin :

> `validate_pr_ready_undraft_scope` (`builtin_handlers.rs`, mika#1682) inspects
> an argv and un-drafts nothing — it compares `verb == "ready"` against a
> variable and is out of the population **by shape, not by exemption**.

Notre terme refuse et n'émet rien : il reste hors population **à condition de
garder cette forme**. Conséquence d'implémentation, à écrire au site : **aucune
constante littérale `&["pr", "ready", …]` en moitié production.** Les fixtures
argv des tests vivent sous `#[cfg(test)]`, que le scan tronque — c'est déjà le
cas des fixtures mika#1682 existantes (`str_args(&["pr", "ready", "1681"])`,
l. 11860+).

### R7 — L'angle mort réel : le pilote dispatché, et il n'est pas couvert

Un pilote claude-pilot tape `gh` par **Bash dans son bac à sable**, jamais par
`run_gh`. Aucune deny rule sur `gh pr ready` n'existe dans `.claude/settings.json`
ni dans `.claude/claude-pilot.json`, et `permission-policy` n'en porte aucune.
Donc `gh pr ready` par un pilote **échappe entièrement** à ce correctif.

Population mesurée : **zéro** (aucun `pr ready` de pilote dans l'incident, et
`dispatch-lib.sh` n'en émet aucun — le grep sur `skills/` et `scripts/` ne rend
que les quatre phrases de prompt et le prescripteur opérateur
`.claude/commands/mika-rescue-drafts-process.md:114`). **Nommé, non couvert,
ticket de suivi** — précondition : une mesure montrant qu'un `gh pr ready` de
pilote a franchi un hold.

---

## Conception

### Un second terme, au même garde, sur une forme plus étroite

Le terme hold vit dans `validate_pr_ready_undraft_scope`
(`builtin_handlers.rs:2804`), déjà en place dans la chaîne de `run_gh` : après
`validate_fallthrough_work_creation` (mika#2573), avant
`validate_destructive_action_grounding` (mika#1646). Même garde parce que les
deux termes répondent à **la même question** — *cette PR peut-elle sortir du
brouillon ?* — par deux discriminants ; deux gardes au même endroit scinderaient
une décision en deux sites qu'un futur éditeur pourrait faire diverger, ce que le
dépôt a déjà payé une fois (`two-predicates-for-one-concept-livelock-2026-09-03.md`).

L'ordre des deux termes **dans** le garde, et il n'est pas arbitraire :

| # | terme | coût | disposition |
|---|---|---|---|
| 1 | signature wip-rescue (mika#1682) | un `gh pr view` (subprocess) | **fail-open** sur la lecture |
| 2 | hold explicite (mika#2624) | un appel GraphQL | **fail-closed** sur la lecture |

Le terme 1 d'abord, inchangé : s'il refuse, l'appel GraphQL est économisé. Et le
terme 2 ne tourne que sur la forme `pr ready` (R2), donc son coût est borné à
cette population — un appel GraphQL par `pr ready`, verbe rare.

### Le détecteur, distinct et étroit

`detect_pr_ready_undraft(args) -> Option<&str>` ne reconnaît que
`pr ready <N>` sans `--undo`, et réutilise `extract_pr_number_positional`
(inchangé). `detect_ready_promote_pr` n'est pas touché : son périmètre à deux
formes est correct pour mika#1682, et le modifier changerait la population du
terme 1 par effet de bord d'un ticket qui ne le vise pas.

Un test tient la frontière : `pr edit <N> --title` n'est **pas** dans la
population du terme hold.

### La classification, extraite une fois

`wip_rescue::classify_hold_verdict(events: Result<Vec<ConvertToDraftEvent>, String>) -> HoldVerdict`,
`pub(crate)`, extraite du `match` inline de `hold_verdict`, qui l'appelle
désormais. `HoldVerdict` est déjà `pub(crate)` et porte **tout** le raisonnement
en doc-comment (pourquoi la simple présence suffit, pourquoi pas de filtre
d'acteur, pourquoi « postérieur au dernier push » est refusé) : il reste où il
vit, et `builtin_handlers.rs` l'importe. Les trois états sont conservés —
`NotHeld` / `Held` / `Unreadable`, jamais un `bool`, parce que deux d'entre eux
excluent et appellent des remèdes opposés.

`builtin_handlers.rs` compose donc :
`fetch_convert_to_draft_events` → `classify_hold_verdict` → décision. **Aucune
recopie du prédicat**, et un scan de source le tient (§ Fire-Disposition).

### Fail-closed, et le sens de l'asymétrie est LOCAL

AC2 : toute lecture impossible refuse. Quatre causes, chacune son motif :

| motif | cause | disposition |
|---|---|---|
| `operator_hold` | ≥ 1 `ConvertToDraftEvent` | refus |
| `hold_unreadable` | API injoignable, 401/403/429, payload illisible, forme inattendue | refus |
| `hold_no_token` | `ctx.github_token` est `None` | refus |
| — | `NotHeld` | **autorisé** — octet pour octet le comportement d'avant |

**L'asymétrie, mesurée et non supposée.** Un faux refus coûte **un `pr ready`
refusé** : visible, borné, rattrapable au tour suivant, et le remède (un humain
sort la PR du brouillon) est celui que le hold prescrit de toute façon. Un faux
passage coûte **le hold d'opérateur lui-même**, donc une PR hors brouillon,
approuvée, verte, et mergeable par le moteur — l'incident fondateur, dont rien
n'a empêché le merge sinon le calendrier. *Un terme qu'on ne peut pas lire n'est
jamais un terme satisfait* (mika#2277), appliqué ici au terme « cette PR n'est
**pas** tenue ».

C'est aussi la politique **uniforme** du module d'où vient le discriminant :
`hold_verdict`, `has_bailed_marker`, `has_parked_marker`, `classify_route` et
`fresh_pipeline_verified` sont tous fail-closed. Et c'est l'**inverse** du
faucheur mika#2420, où un signal illisible *conserve* — là-bas l'action
détruisait du travail, ici l'action *est* la levée d'un hold. **L'arbitrage est
local et ne se transporte pas.**

### Résolution du dépôt, et son coût nommé

`fetch_convert_to_draft_events` veut `(owner, repo)`. `run_gh` tient
`gh_args.repo: Option<String>` (le paramètre `repo` de l'outil ; `--repo` dans
l'argv est refusé en amont par `validate_gh_input`). Absent ⇒ repli sur le dépôt
par défaut `senara-solutions/mika`, comme `wip_rescue::default_repo_parts()`.

**Coût, dit plutôt que découvert :** un `pr ready` sur une PR d'un autre dépôt
sans `repo` passé interroge la timeline du mauvais dépôt. GraphQL rend alors
`pullRequest: null`, l'extracteur fail-closed rend `Err`, le refus tombe sous
`hold_unreadable`. C'est un faux positif, et son remède est **nommé dans le corps
du refus** : passer `repo`. Acceptable parce que le sens de l'erreur est le bon et
que le remède est à un argument de distance.

### Le corps du refus nomme le hold, le lève-qui, et AUCUN contournement

Forme JSON du voisin mika#2573 (`error` / `doctrine` / `reason` / `remedy`). Le
`remedy` dit que seul un humain lève un hold — **sans nommer de route
alternative** : un refus qui donne le gabarit est une fuite avec une étape de plus
(doctrine mika#2520). Le `reason` rapporte `since` et `actor` **quand ils sont
lisibles**, et jamais inventés (`None` ⇒ champ absent, jamais la chaîne `"null"` —
mika#2331 : *`null` is never `0`*). Ils ne décident de **rien**, exactement comme
chez mika#2597 : ils voyagent pour que le jour où une machine remet une PR en
draft soit visible.

### Pas d'interrupteur d'environnement

Précédent direct, et il est au même site : mika#2573 n'en a pas, pour la raison
qu'il écrit — *un désarmement par variable sur un chemin de création de travail
serait un désarmement par coquille*. Ici le chemin est la **levée d'un hold
d'opérateur**, strictement plus grave. Le geste de désarmement est un **revert**,
et le coût d'un faux positif (un `pr ready` refusé) le supporte.

### La moitié intention, et elle ne tient pas seule

Les quatre phrases de prompt de R3 passent d'une interdiction **par
délimitation** à une interdiction **topique** : ne pas sortir du brouillon une PR
qu'on n'a pas mise en brouillon, sans énumérer les signatures qui la rendraient
permise. Par
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, cette
moitié **exprime l'intention** ; celle qui **tient** est le refus moteur. Le
prescripteur opérateur `.claude/commands/mika-rescue-drafts-process.md:114` reste
intact : c'est un geste d'humain, et c'est très exactement la sortie que le refus
désigne.

---

## Refus raisonnés

1. **Élargir `detect_ready_promote_pr` au lieu d'ajouter un détecteur** —
   refusé : ça ferait refuser `pr edit --title` sur une PR tenue, hors AC1, et un
   faux positif sur un geste qui ne touche pas l'état draft (R2).
2. **Déplacer `HoldVerdict` dans un module partagé** — refusé : son doc-comment
   porte le raisonnement entier de mika#2597 *et* ses fragilités nommées (le
   `--draft` du listing comme prémisse, l'auto-annulation du prédicat
   « postérieur au dernier push »), tous ancrés sur `wip_rescue`. Le déplacer
   délierait la décision de son argument. `pub(crate)` suffit.
3. **Un second nom d'événement, à la manière de
   `phantom_aged_out` / `phantom_sweep_spared`** — refusé : ce motif s'applique
   quand chaque nom porte **sa propre cause** sur **sa propre population**. Ici
   les deux motifs appartiennent au **même site** et à la **même population** (un
   undraft refusé par `run_gh`). Motif `ready_label_outcome` (mika#2323) : un seul
   nom, la cause dans un champ. La ligne existante porte déjà un champ `reason` —
   il reçoit des valeurs, pas un homonyme.
4. **Fail-open quand `ctx.github_token` est `None`** — refusé, et c'est le cas
   limite qui mérite d'être écrit : `gh` peut s'authentifier par
   `~/.config/gh/hosts.yml`, donc un `pr ready` sans token moteur **réussit
   aujourd'hui**. Le laisser passer garderait une porte ouverte exactement pour la
   population que le moteur ne peut pas mesurer. Refus sous son propre motif,
   pour que cette population soit comptable séparément.
5. **Un garde EndTurn plutôt qu'un garde d'outil** — refusé pour la raison que le
   voisin mika#1646 a déjà dû écrire au même fichier : à EndTurn la PR est déjà
   hors brouillon, et le re-prompt ne peut qu'annoter un fait accompli.
6. **Ajouter un terme à `GH_API_ALLOW_MATRIX` pour « couvrir » la branche
   GraphQL** — refusé : la matrice est une allowlist fermée qui refuse déjà (R4).
   Y ajouter un discriminant donnerait l'illusion que la fermeture vient de là.

---

## Surfaces opérateur

```bash
# 1. Un undraft a-t-il été refusé, et sur quel motif ?
grep pr_ready_undraft_blocked "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{agent_id, session_id, pr_number, reason, repo, hold_since, hold_actor}'

# 2. CONTRÔLE POSITIF — le garde tourne-t-il seulement ?
#    Le nom est le MESSAGE de la ligne, pas un champ `event` : run_gh émet
#    `"run_gh invocation"` (builtin_handlers.rs, fin de `run_gh`). Grepper
#    `run_gh_invocation` ne rend rien, quel que soit l'état.
grep -c 'run_gh invocation' "$MIKA_SPIRIT_LOG_FILE"
```

```sql
-- Les motifs, soustractibles. `pr_ready_undraft_blocked` est SOLE WRITER.
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'pr_ready_undraft_blocked' GROUP BY 1 ORDER BY 2 DESC;
```

| motif (`after_value`) | régime attendu | lecture |
|---|---|---|
| `operator_hold` | **non vide, faible** | chaque ligne est un hold que le moteur n'a pas levé — la mesure directe que la garde mord |
| `hold_unreadable` | **vide** | la timeline n'a pas répondu ; la cause est le jeton ou l'API, **pas le prédicat** |
| `hold_no_token` | **vide** | `run_gh` tourne sans jeton résolu — lire mika#2205 avant de toucher au prédicat |
| `wip_rescue_contract` | inchangé en sens | population mika#1682, comptable séparément |
| `pr_ready_undraft_audit_failed` (WARN) | **vide** | la ligne INFO est passée, l'audit non — le `GROUP BY` sous-compte |

**Coût de surface, daté.** La ligne `audit_events` est **ajoutée** par ce travail :
le garde mika#1682 n'écrivait qu'un `tracing::info!`. Donc un `count(*)` nu sur
`tool_name = 'pr_ready_undraft_blocked'` qui enjambe le déploiement **compare un
vide à une population**. Les lignes antérieures ne sont pas rétro-écrites —
fabriquer une ligne d'audit datée d'un refus qu'on n'a pas observé est l'inverse
de ce que ce travail défend. La requête juste est celle groupée par `after_value`,
et le motif `wip_rescue_contract` est ce qui rend la population mika#1682
identifiable dès sa première ligne.

---

## Sondes post-déploiement, et leurs cinq haltes

> **Préalable.** Le garde vit dans **mika-spirit**, pas dans un handler seedé :
> la sonde décrit le binaire servi. Établir le déploiement avant toute
> conclusion (classe mika#2340). La moitié **prompt** (R3), elle, est une
> projection du binaire via `skills/bundled/` —
> `cat ~/.mika/skills/.manifest-writer` doit porter le sha qu'on vient de bâtir
> (mika#2340).

**S1 — le rejeu du défaut fondateur.** Sur une PR mise en brouillon à la main,
demander à mika-dev un `run_gh ["pr","ready",<N>]`. Attendu : refus, une ligne
`reason = "operator_hold"`, et **aucun** `ReadyForReviewEvent` sur la timeline.
*Halte 1 — la PR sort du brouillon :* **ne pas élargir le prédicat par réflexe.**
Lire d'abord le contrôle positif (sonde 2) : si `run_gh` ne tourne pas du tout, la
question n'est pas le prédicat. Vérifier ensuite **par quelle porte** l'appel est
passé — `run_shell` (R5), `gh api` (R4) et le pilote (R7) ont trois remèdes
différents, et seul le troisième est ouvert.

**S2 — contrôle négatif du chemin nominal (7 jours).** Un brouillon **né** en
draft (`gh pr create --draft`, toute PR de `dispatch-lib`) sans
`ConvertToDraftEvent` reste promouvable, et `wip_rescue` continue de promouvoir.
Contrôle négatif SQL :
`SELECT count(*) FROM audit_events WHERE tool_name = 'wip_rescue' AND target_key = 'wip_rescue_success';`
doit **continuer à croître**.
*Halte 2 — il se figent :* la classification extraite a changé de sens pour son
appelant d'origine. **Revert d'abord** — geler `wip_rescue` casse le mécanisme
mika#1852 en entier — diagnostic ensuite.

**S3 — contrôle négatif de bruit (7 jours).** `hold_unreadable` et
`hold_no_token` restent vides.
*Halte 3 — `hold_unreadable` porte du trafic :* le garde est **fail-closed sur un
signal qu'il ne sait pas lire**, donc il refuse des `pr ready` légitimes. Ce n'est
pas un seuil à régler : lire d'abord si le `repo` est passé (le faux positif nommé
§ Résolution du dépôt), puis le jeton. **Ne pas basculer en fail-open** — ce
serait rouvrir le défaut sous couvert de confort.

**S4 — la frontière de forme (7 jours).** Aucun refus sur un
`pr edit <N> --title` au motif `operator_hold`.
*Halte 4 — une occurrence :* le terme hold a été branché sur le détecteur à deux
formes (R2) ; réparer le détecteur, pas filtrer en aval.

**S5 — le scan de classe reste vert.** `cargo test -p mika-agent mika2597_` rend
vert, `mika2597_un_seul_site_dundraft_en_production` compris.
*Halte 5 — il rougit sur la cardinalité :* une constante littérale
`&["pr","ready"]` a atterri en moitié production (R6). **Ne pas allowlister** —
retirer le littéral.

**Halte transverse — les sondes muettes.** Zéro refus **et** zéro appel `run_gh`
ne prouve rien : il faut qu'un `pr ready` ait été tenté depuis le déploiement.
*Une garde que personne n'a exercée se lit exactement comme une garde qui marche*
(mika#2205).

---

## Fire-Disposition

Ce plan livre trois détecteurs. **Disposition (a) — exception nommée en
allowlist, et les deux allowlists sont livrées VIDES**, avec leur assertion
auto-nettoyante. C'est le motif du dépôt (mika#2201 : *« on déclare, on
n'allowliste pas »* ; mika#2323 : *une allowlist née vide est un tiroir où
déposer la prochaine infraction*).

| # | détecteur | tire-t-il sur l'arbre ? | disposition |
|---|---|---|---|
| D1 | `detect_pr_ready_undraft` + `decide_pr_ready_hold` — fonctions pures, tests comportementaux | n/a (pas un scan) | armé |
| D2 | `mika2624_le_predicat_de_hold_a_un_lecteur_unique` — scan de source refusant une seconde classification `ConvertToDraftEvent → verdict` hors `wip_rescue::classify_hold_verdict` | **non** — mesuré : une seule classification après extraction | armé, allowlist **vide** + test qui refuse qu'elle cesse de l'être |
| D3 | `mika2597_un_seul_site_dundraft_en_production` — **existant**, non modifié | **non** — un seul site (`wip_rescue.rs:1199`) | inchangé |

**Pourquoi D2 est un scan et pas un test comportemental.** Une seconde
classification écrite demain ne rend **aucune décision fausse** le jour où elle
est écrite : les deux lecteurs répondraient d'abord la même chose, toutes les
assertions resteraient vertes, et ils divergeraient des mois plus tard — la classe
exacte que mika#2158 a mesurée (`is_groomed` et
`check_grooming_markers` ont divergé pendant des mois derrière un commentaire
disant « mirrors »). D2 porte son **anti-vacuité** : il échoue si la needle n'est
trouvée **nulle part**, parce qu'un scan visant une needle morte se lit exactement
comme un arbre propre (mika#2205).

**AC3 demande « vu rouge avant le correctif », et voici la forme qui le permet.**
`decide_pr_ready_undraft(pr, Some(view_sans_signature_wip_rescue))` rend `Ok(())`
sur l'arbre actuel. Un test qui lui adjoint un `HoldVerdict::Held` et assert
`is_err()` **rougit avant** et verdit après — c'est le test fondateur, écrit et vu
rouge **avant** la ligne qui le répare. Les deux contrôles positifs (brouillon né
draft ⇒ promouvable ; `--undo` ⇒ permis) sont verts des deux côtés, et c'est leur
rôle : ils distinguent « le prédicat mord » de « le prédicat refuse tout ».

---

## Acceptance criteria

- **AC1.** `run_gh` refuse `gh pr ready <N>` (sans `--undo`) quand la timeline de
  la PR porte au moins un `ConvertToDraftEvent`. Le discriminant est celui de
  mika#2597 — `fetch_convert_to_draft_events` + `classify_hold_verdict`, appelés,
  jamais recopiés — et D2 tient cette propriété structurellement. Le corps du
  refus nomme le hold (avec `since` / `actor` quand ils sont lisibles, jamais
  inventés) et dit que seul un humain le lève, sans nommer de contournement.
- **AC2.** Le refus est fail-closed : toute lecture impossible de la timeline
  refuse, sous un motif qui la distingue d'un hold réel — `hold_unreadable` pour
  une API, un 401/403/429, un payload ou une forme illisibles ; `hold_no_token`
  pour un `ctx.github_token` absent.
- **AC3.** Tests : un brouillon portant un `ConvertToDraftEvent` est refusé
  (**vu rouge** avant le correctif, sur `decide_pr_ready_undraft`) ; un brouillon
  né en draft, sans événement, reste promouvable (contrôle positif) ; `--undo`
  reste permis (contrôle positif) ; `pr edit <N> --title` n'entre pas dans la
  population du terme hold (frontière de forme, R2).
- **AC4.** Tout site pouvant sortir une PR du brouillon est couvert par le même
  discriminant ou nommé avec la raison de son exclusion : `run_gh pr ready`
  **couvert** ; `wip_rescue` **couvert** (mika#2597) ; `gh api` **exclu** —
  allowlist fermée, ni `graphql` ni `/repos/*/pulls/*` ne matchent (R4) ;
  `run_shell` **exclu** — refuse `gh` nommément (R5) ; un pilote claude-pilot
  **exclu et NON couvert**, population mesurée vide, ticket de suivi (R7).
- **AC5.** Les quatre phrases de prompt qui interdisaient `pr ready` **par
  délimitation de la signature wip-rescue** deviennent topiques, pour cesser de
  permettre par contraste (R3). Moitié intention : la moitié qui tient est AC1.
- **AC6.** Non-régression : `NotHeld` autorise octet pour octet comme avant ; le
  terme mika#1682 est inchangé dans son périmètre comme dans sa disposition
  fail-open ; `wip_rescue` continue de promouvoir (contrôle négatif S2) ; le scan
  `mika2597_un_seul_site_dundraft_en_production` reste vert sans modification.

---

## Definition of Done

1. `classify_hold_verdict` extraite dans `wip_rescue.rs`, `pub(crate)`,
   `hold_verdict` l'appelle — aucun changement de comportement pour l'appelant
   d'origine.
2. `detect_pr_ready_undraft` + `decide_pr_ready_hold` dans
   `builtin_handlers.rs`, le terme branché dans
   `validate_pr_ready_undraft_scope` **après** le terme wip-rescue.
3. Ligne `tracing::info!(event = "pr_ready_undraft_blocked", …)` enrichie des
   champs `hold_since` / `hold_actor`, plus une ligne `audit_events` sous le même
   nom, le motif en `after_value`, avec son `pr_ready_undraft_audit_failed` (WARN)
   quand l'audit échoue — motif du voisin mika#2573.
4. Les quatre prompts de R3 reformulés.
5. D2 écrit, allowlist vide, assertion auto-nettoyante, anti-vacuité.
6. Tests AC3, dont le fondateur **vu rouge** avant le correctif.
7. `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt` verts.
8. Entrée dans `crates/mika-agent/CLAUDE.md`, **adossée au § _Un hold explicite
   tient contre `wip_rescue`_ (mika#2597)** — désigné par son titre et non par un
   numéro de ligne, qui pourrit en silence (le ticket cite « ~l.1395 » ; la
   section est à la l.1477 au moment d'écrire, ce qui est exactement le mode de
   péremption en question). Contenu : le défaut mesuré, les sept rectifications,
   le tableau des motifs, les cinq sondes et leurs haltes, ce que le travail
   n'achète pas.
9. `make verify-bundled-skills` vert (les prompts bundled changent).

---

## Ce que ce travail n'achète PAS

- **Il ne rattrape pas l'incident du 2026-10-01.** PR #2621 est sortie du
  brouillon et mergée ; **rien ici ne rétro-écrit** une ligne d'audit décrivant un
  refus qui n'a pas eu lieu. La sonde est la **prochaine** occurrence.
- **Il n'empêche pas un pilote de taper `gh pr ready`.** R7 : population mesurée
  vide, exclusion nommée, suivi. Le périmètre couvert est l'outil `run_gh`, par
  lequel l'incident est passé.
- **Il ne rend pas le modèle incapable de vouloir sortir une PR du brouillon** :
  il le rend incapable de le faire sur une PR tenue. C'est la doctrine maison
  (*construis l'incapacité, ne promets pas la retenue*, mika#1991), applicable ici
  parce qu'il y a bel et bien une capacité à retirer — contrairement à mika#1983,
  qui a dû écrire pourquoi elle ne s'y appliquait pas.
- **Il ne borne pas le réveil stale-verdict.** R3 : le handler rend un
  `Passthrough` muet, et c'est **voulu** — mika#1745 a tranché que ce chemin laisse
  le choix au modèle. Ce qui change est que l'un des choix lui est désormais
  interdit. Un enrichissement de ce `Passthrough` est une autre décision, et elle
  a son propre ticket.
- **Il ne surveille rien.** Les seuls instruments sont le grep et la requête du
  § Surfaces, et **leur silence ne prouve rien tant que personne ne les exécute** —
  sur un verbe aussi rare que `pr ready`, l'absence de refus peut simplement
  vouloir dire que personne n'a essayé. D'où le contrôle positif, sans lequel
  « aucun hold violé » et « le garde ne tourne pas » rendent les mêmes octets.

---

## Hors périmètre, délibérément

- **Le `gh pr ready` d'un pilote dispatché** (R7) — suivi, précondition : une
  mesure.
- **Le périmètre de mika#1682** : la signature wip-rescue, sa disposition
  fail-open, et sa couverture de `pr edit --title`. Inchangés.
- **`markPullRequestReadyForReview` et `PATCH /pulls/N`** — déjà refusés par
  l'allowlist fermée (R4). Y toucher déplacerait une garantie qui tient.
- **Le prescripteur opérateur** `.claude/commands/mika-rescue-drafts-process.md` —
  c'est un geste d'humain, et c'est la sortie que le refus désigne.
- **Le filtre d'acteur sur le `ConvertToDraftEvent`** — refusé par mika#2597 avec
  sa raison (population vide, et lire le geste d'une machine comme un hold est le
  côté **sûr**). L'acteur est *rapporté*, il ne décide de rien.
- **Un interrupteur d'environnement** — refusé ci-dessus, précédent mika#2573.
- **Le merge autonome lui-même** (`pr_merge_with_gate`) : il a ses propres portes,
  et ce travail ferme la porte **en amont** par laquelle le hold a été levé.
