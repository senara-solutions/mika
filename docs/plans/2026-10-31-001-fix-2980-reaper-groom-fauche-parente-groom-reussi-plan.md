# mika#2980 — le reaper de groom fauche la parente d'un groom réussi

**Type :** fix
**Branche :** `fix/2980/task-engine-le-reaper-de-groom-fauche-la`
**Ticket :** senara-solutions/mika#2980
**Lié :** mika#2287 (porte de provenance), mika#2762 (même reaper, autre trou), mika#2181 (même angle mort sur `stuck_pending`), mika#2750 (le reaper), mika#2976 (le `REBASE_CONFLICT`), mika#2968 (le vol perdu)

---

## 1. Le défaut, en une phrase et en un calcul

`find_orphaned_parent_tasks(…, "groom")` (`crates/mika-agent/src/db/tasks.rs` l. 2067) **ne voit pas** un enfant de groom `completed` dont le callback n'est pas encore livré. Il ne le voit ni comme ancre de grâce (l. 2087 : `AND child.status = 'delivered'`) ni comme frère actif (l. 2095 : `sibling.status IN ('pending', 'in_progress')`). Quand la parente porte par ailleurs un enfant de groom livré et ancien, elle est donc fauchée **pendant** la fenêtre `completed → delivered` du groom qui a réussi.

Le calcul du cas mesuré, à 15:21:05Z le 2026-10-30, sur la parente `be9eca02-a0ee-4753-9eb0-b20f48a3c37a` :

| terme | valeur |
|---|---|
| dernier enfant de groom `delivered` non différé | `207589e1`, `updated_at = 14:38:05Z` (résultat `STATUS=REBASE_CONFLICT`) |
| `MAX(child.updated_at)` | `14:38:05Z` |
| seuil `strftime('now', '-600 seconds')` | `15:11:05Z` |
| `HAVING 14:38:05 < 15:11:05` | **vrai** ⇒ candidat |
| frères `pending`/`in_progress` | **aucun** — `6b7695ce` est `completed`, les lanceurs `e7d2cf32`/`49e5b961` sont `delivered` |

La parente passe `failed` à 15:21:05Z. Le `callback_delivered` de `6b7695ce` arrive 40 s plus tard, à une parente terminale, et `validate_dispatch_readiness` répond *« Task is in 'failed' state and cannot be dispatched. Only 'pending' and 'in_progress' tasks can be dispatched. »* (`crates/mika-agent/src/skills/executor.rs` l. 3617-3632). Le plan `docs/plans/2026-10-30-004-fix-2968-manager-l-valuateur-recommande-merge-plan.md` était poussé (commit `4ae5ec9f6`), le verdict avait convergé — et le vol est perdu sans alarme.

La fenêtre mesurée, entre `completed` (15:20:31Z) et `callback_delivered` (15:21:45Z), vaut **74 s**. C'est elle que ce vol ferme.

### Pourquoi mika#2762 ne couvre pas ce trou

mika#2762 a ancré la grâce sur la dernière **livraison** et exclu les lanceurs `:deferred` (l. 2077 et l. 2088). Les deux correctifs sont en place et corrects. Ils ne mordent pas ici : la dernière livraison *est* celle de `207589e1`, et c'est précisément elle qui est ancienne. Le trou n'est pas dans le choix de l'ancre, il est dans le fait qu'**aucun** des deux prédicats ne connaît l'état `completed`.

### Pourquoi le cas est atteignable, et donc récurrent

Il faut **un groom en deux tentatives sur la même parente**, la première livrée et la seconde réussie plus de 600 s plus tard. C'est exactement ce que produit un `REBASE_CONFLICT` (mika#2976) suivi d'un regroom. Rien dans cette configuration n'est exceptionnel : la population visée par la sonde d'AC4 est « les grooms qui comptent deux enfants de groom sur la même parente ».

---

## 2. AC1 — la décision : c'est la **garde de frère actif** qui est élargie, bornée

### Le changement

Dans `find_orphaned_parent_tasks`, le `NOT EXISTS` gagne un second bras :

```sql
AND NOT EXISTS (
  SELECT 1 FROM tasks sibling
  WHERE sibling.parent_task_id = parent.id
    AND sibling.id != child.id
    AND (
      sibling.status IN ('pending', 'in_progress')
      OR (sibling.status = 'completed'
          AND sibling.completed_at IS NOT NULL
          AND sibling.completed_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2))
    )
)
```

`?2` est **déjà** lié : c'est le `grace_modifier` (`format!("-{grace_seconds} seconds")`, l. 2073) que le `HAVING` consomme. Aucun paramètre nouveau, aucune signature touchée.

### Pourquoi la garde, et pas l'ancre

L'alternative que le ticket laisse ouverte — lire les `completed` dans `MAX(child.updated_at)` — est refusée, pour trois raisons distinctes :

1. **Les deux prédicats ne répondent pas à la même question.** L'ancre répond *« à quand remonte le dernier mouvement contre lequel la grâce est mesurée ? »* ; la garde répond *« quelqu'un représente-t-il encore cette parente ? »*. Un enfant `completed` non livré est un enfant **en vol** : son callback est en route, et c'est `deliver_callback` qui, en le livrant, fera tourner `try_dispatch_pilot_after_groom_success`. C'est une propriété de **représentation**, pas d'ancienneté. La ranger dans la garde, c'est la ranger là où elle est vraie.

2. **Élargir l'ancre donnerait deux causes à une seule valeur.** mika#2762 a délibérément fait de `last_callback_at` l'instant de la dernière **livraison**, et son doc-comment le dit (« `callback_task_id` is NOT the anchor »). Si l'ancre lisait aussi les `completed`, le champ remonté dans `task_engine_reaper.evaluated` signifierait « dernière livraison » ou « dernière complétion » selon la ligne, et l'opérateur qui le lit ne pourrait plus savoir laquelle. C'est la classe mika#2205 : deux populations sous un même nom ne sont pas soustractables. L'instrument doit rester honnête.

3. **Élargir l'ancre décalerait la faux sur la population saine.** Une ancre qui lit les `completed` repousse le seuil de 600 s pour **toute** parente portant un enfant de groom `completed`, y compris un cadavre jamais livré — elle allonge le délai pour tout le monde. La garde, elle, n'agit que dans la fenêtre de liveness et rend la main **au même instant** que le comportement actuel dès que la fenêtre est passée. Le correctif est strictement local au trou.

Le précédent maison est exact et nommé : `has_live_deferred_wrapper_child` / `find_live_deferred_wrapper_child` (`crates/mika-agent/src/db/tasks.rs` l. 2695-2800, mika#2181) ont réglé le **même** angle mort pour le faucheur `stuck_pending`, avec **la même forme** :

```sql
AND (
  status = 'pending'
  OR (status = 'completed'
      AND completed_at IS NOT NULL
      AND completed_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?4))
)
```

Le reaper de groom n'en avait pas hérité. Ce vol le lui fait hériter, sans inventer de forme nouvelle.

### Pourquoi la clause est **bornée**, et sur `completed_at`

Une clause non bornée (`sibling.status = 'completed'` tout court) rendrait la parente immortelle. Le doc-comment de mika#2181 porte l'argument et il s'applique mot pour mot ici : le chemin d'erreur du tour silencieux laisse un enfant `completed` **pour toujours**, et un cadavre qui ne sera jamais livré deviendrait alors un bouclier permanent — on échangerait un vol perdu par orphelinage contre une parente wedgée à vie, c'est-à-dire un défaut pire, et muet.

`completed_at IS NOT NULL` fait partie de la borne, pas de la décoration : une ligne sans `completed_at` se lit comme **non** vivante. `update_task_completed` (l. 775-788) écrit `status='completed'` et `completed_at` dans le même `UPDATE`, donc sur le chemin nominal la colonne est toujours renseignée ; un `NULL` y signale une ligne écrite autrement, dont on ne sait pas dater la complétion — et une date inconnue ne peut pas prouver la fraîcheur.

### Pourquoi la fenêtre est `grace_seconds`, et pas une nouvelle constante

Décision : **réutiliser `grace_seconds`** (donc 600 s au site d'appel, `REAPER_GRACE_SECONDS`, `crates/mika-agent/src/task_engine/engine.rs` l. 110). Trois raisons, dans cet ordre :

1. **Zéro cadran nouveau, donc zéro révocation silencieuse.** Une constante dédiée voudrait son `MIKA_*`, donc son clamp, donc le piège que `PROMOTED_WRAPPER_LIVENESS_MAX_SECS` existe pour couvrir : `strftime(…, 'now', '-N seconds')` rend **NULL** sur un modificateur hors plage, et `x > NULL` est NULL — une surcharge absurde ferait donc revenir le défaut **sans aucun signal**, la garde se contentant de ne plus jamais retenir personne. En réutilisant le paramètre déjà lié, ce chemin n'existe pas pour ce vol.
2. **L'échelle est juste.** 600 s = 2 × `AGENT_TOTAL_TIMEOUT` (300 s) — c'est la justification que `STUCK_PENDING_ACTIVITY_WINDOW_DEFAULT_SECS` porte déjà — et 8 × la fenêtre mesurée (74 s). La marge est large sans être une éternité.
3. **La réutilisation délibérée a son précédent.** `DISPATCH_PARENT_SETTLE_GRACE_DEFAULT_SECS = REAPER_GRACE_SECONDS` : la maison accepte qu'un nombre serve deux fois quand c'est écrit.

Contrepartie, nommée parce que c'est le coût réel : **ce nombre fait désormais deux métiers** — il borne la grâce *et* la liveness d'un frère `completed`. Le doc-comment le dit explicitement, et ajoute la conséquence : recalibrer `REAPER_GRACE_SECONDS` déplace les deux. Si un jour les deux doivent diverger, c'est à ce moment-là qu'une constante dédiée se justifie, avec son clamp.

### Pourquoi le SQL est correct — la vérification qui n'est pas évidente

Le `NOT EXISTS` porte `sibling.id != child.id`, et on pourrait craindre qu'il exclue justement l'enfant qu'on veut compter. Il ne peut pas : la clause `WHERE` principale exige `child.status = 'delivered'`, donc **un enfant `completed` n'entre jamais dans la population `child`**. Il n'est donc jamais le `child` d'une ligne jointe, et `sibling.id != child.id` ne le filtre jamais.

Déroulé sur le cas mesuré : la seule ligne jointe est `child = 207589e1` ; le `NOT EXISTS` trouve `sibling = 6b7695ce` (`completed`, `completed_at = 15:20:31Z > 15:11:05Z`) ⇒ la ligne est écartée ; le `GROUP BY` n'a plus de ligne pour `be9eca02` ⇒ **aucun candidat**. La parente reste `in_progress`, le callback arrive sur une parente dispatchable, l'auto-fire part.

Et le cas symétrique ne régresse pas : une parente dont le **seul** enfant de groom est `completed`-non-livré ne rend déjà aucune ligne aujourd'hui (il n'y a aucun `child` `delivered` à joindre). Le changement ne lui fait rien.

---

## 3. AC3 — le couple est changé **symétriquement**

`find_completable_parent_tasks_on_pr_url` (l. 2491-2536) porte le contrat de couple dans le doc-comment de `find_orphaned_parent_tasks` (l. 2058-2066) : *« Any filter change here … MUST be applied symmetrically there »*. La garde de frère actif y est **mot pour mot la même**, et mika#2762 a établi le précédent en changeant les deux et en livrant un test de symétrie (`mika2762_la_voie_de_succes_sancre_pareil`).

Décision : **le même second bras y est ajouté**, dans les mêmes termes, avec le même paramètre de grâce.

Coût nommé, parce qu'il n'est pas nul : côté succès, une ligne écartée ne fait pas échouer une parente — elle la laisse `in_progress` jusqu'à 600 s de plus au lieu de la passer `completed`, donc elle **retient le slot de dispatch** d'autant. C'est strictement moins grave que la faux côté échec, et surtout : la question que la garde pose (*la parente est-elle encore représentée ?*) a exactement la même réponse des deux côtés. Laisser le couple divergent sur la forme de la garde reviendrait à soutenir que la représentation compte pour décider d'un échec mais pas pour décider d'une complétion — ce qui est indéfendable, et ce que le contrat de couple interdit.

---

## 4. AC5 — le vol repart sans geste humain, et c'est AC1 qui le fait

### L'argument

La fenêtre d'orphelinage d'un groom convergé est **exactement** `completed → deliver_callback`. L'auto-fire `try_dispatch_pilot_after_groom_success` (`crates/mika-agent/src/task_engine/dispatcher.rs` l. 1537, corps l. 4709+) tourne **dans** `deliver_callback`, avant `run_silent_agent` et avant `mark_task_delivered` (l. 1642). Donc :

- tant que la livraison arrive **sous 600 s** après le `completed`, AC1 protège la parente sur toute la fenêtre, l'auto-fire trouve une parente `in_progress`, et `validate_dispatch_readiness` (étape 5e) ne refuse rien ;
- une fois l'auto-fire parti, il a créé un enfant callback `pending` (étape 5f) : un frère actif au sens du prédicat **d'origine**. La protection passe le relais sans discontinuité, sans rien de nouveau.

Sur la population mesurée — 74 s, soit 8 × de marge — AC5 est donc satisfait **par le correctif d'AC1**, ce que le ticket autorise explicitement (« Le correctif d'AC1 peut suffire, s'il empêche l'orphelinage »).

### Le résidu, et pourquoi il est renvoyé plutôt que construit

Le résidu est la livraison qui arrive **au-delà** de 600 s après le `completed`. Deux faits décident de ne pas le traiter dans ce vol :

1. **Sa population est inconnue, et AC4 est précisément l'instrument qui la mesurera.** Rien dans le cas observé ne l'atteint. Construire le filet maintenant, c'est bâtir sur un proxy quand une mesure directe est déjà commandée par le ticket — le motif que la maison refuse (mika#2184 : le proxy filtre, la mesure directe tranche).
2. **Sa forme est d'une autre classe de risque.** Le filet devrait **réhabiliter** une parente terminale : écrire `in_progress` sur une ligne `failed`, avant l'étape 5e de l'auto-fire. Le précédent le plus proche, `try_promote_parent_on_retry_success` (#958), promeut `failed → completed` sur présentation d'un `pr_url` — il **ferme** une parente, il ne la rouvre pas. Rouvrir est un geste nouveau, qui veut son propre cadrage et ses propres gardes (que se passe-t-il si deux callbacks réhabilitent la même parente ? si le reaper la refauche entre-temps ?). À ~200 lignes, il ferait aussi sortir ce vol de sa borne (§ 7).

**Ce que ce vol ne livre donc pas, dit en clair :** le test « parente orpheline + plan convergé ⇒ un dev-pilot part ». Il n'est pas livré parce que le code qui le rendrait vert n'est pas livré. Le vol livre l'autre moitié de l'AC — l'orphelinage n'a plus lieu — et la mesure qui dira si la première moitié a une population.

Condition d'ouverture du suivi : la sonde d'AC4 rend un non-zéro **dont la cause est une livraison hors fenêtre de 600 s**. Si elle rend un non-zéro d'une autre cause, c'est un autre ticket.

Tracked in: senara-solutions/mika#2980 (ticket de suivi à ouvrir sur lecture de la sonde AC4 — réhabilitation d'une parente terminale sur groom convergé, ~200 lignes, hors borne de ce vol)

### Le second visage, relu

Le ticket mesure 47 min de rail sec et montre que seul un repose de `ready` à la main a relancé le vol. Les deux maillons de ce second visage sont corrects pris séparément :

- l'auto-skip `already_groomed` de `3596d847` est juste — le plan convergeait déjà ;
- la porte de provenance `has_completed_groom_for_issue` (l. 4340-4416) est juste — elle accepte `child.status IN ('completed','delivered')`, et c'est elle qui a laissé partir `84a482fb` à 15:53:14 (la correction du 2026-10-30 16:30Z du ticket le constate).

Ce qui manquait n'était ni l'un ni l'autre : c'était un **tour de mika-dev avec une parente dispatchable**. AC1 le restitue à la source, en empêchant la parente de devenir terminale. Le chemin `mika ask` → `already_groomed` → `parent_resolved_from_dispatch_refusal` (`try_resolve_parent_on_dispatch_refusal`, l. 4556+) n'est pas touché par ce vol : il écrit `completed`, pas `failed`, et il n'est pas la cause de la perte.

---

## 5. Le second fait du ticket : les 14 min sont **attendues**

Entre la fin du pilote (15:06:40Z, ligne `[done] Success | 34 turns | $4.11 | 678s`) et le passage à `completed` (15:20:31Z), 14 min s'écoulent. Elles ne sont pas un second défaut : c'est le travail d'après-pilote de `dispatch-lib.sh`, et il est entièrement postérieur à cette ligne de journal.

La preuve est dans la séquence de `_iterate_groom_loop` : `PLAN_COMMITTED` est émis à l'étage du pilote, `PLAN_GROOMED` seulement **après** que `_iterate_groom_loop` a réussi (`skills/bundled/_shared/dispatch-lib.sh` l. 7850-7852). Or entre les deux il y a `_push_branch`, puis `_arch_ask_with_retry` (l. 9072) pour la première passe architecte, puis la seconde passe, puis `_write_canonical_callout` (l. 10165), puis le `POST /tasks/{id}/complete`. Deux appels `mika ask --agent mika-arch` sur un plan entier coûtent plusieurs minutes chacun ; 14 min pour push + deux passes + callout est l'ordre de grandeur attendu.

Nuance de lecture, puisque le ticket la pose : la table du ticket lit 15:06:40Z comme l'instant du verdict `PLAN_GROOMED`. C'est l'instant de fin du pilote de **contenu**. Le verdict, lui, ne peut par construction pas être antérieur aux deux passes architecte — il est émis dans les secondes qui précèdent le `completed` de 15:20:31Z.

**Conséquence pour le correctif : aucune.** Le trou du prédicat existe quel que soit ce délai — il suffit que la livraison tombe après le seuil de grâce ancré sur une livraison antérieure. Réduire le délai masquerait le défaut sans le fermer. Rien n'est donc fait sur cet axe dans ce vol.

---

## 6. Ordre de commit

1. **`crates/mika-agent/src/db/tasks.rs`** — les deux prédicats et leurs doc-comments :
   - `find_orphaned_parent_tasks` : second bras de la garde ; doc-comment étendu (le trou mika#2980, pourquoi la garde et pas l'ancre, pourquoi borné, et le fait que `grace_seconds` fait deux métiers) ;
   - `find_completable_parent_tasks_on_pr_url` : même bras, doc-comment nommant la symétrie et son coût (slot tenu jusqu'à 600 s de plus) ;
   - le contrat de couple (l. 2058-2066) est laissé intact — il vient d'être honoré, pas amendé.
2. **`crates/mika-agent/src/db/tests/mod.rs`** — helper `seed_completed_groom_callback(db, parent_id, label, completed_age_secs) -> String`, jumeau de `seed_groom_callback` de `reapers.rs` mais **sans** `mark_task_delivered`, qui rétro-date `completed_at` (et non `updated_at`). Un second helper, ou un paramètre, permet de poser `completed_at = NULL` pour le cas 4.
3. **`crates/mika-agent/src/db/tests/reapers.rs`** — les cinq cas du § 8, à la suite du bloc mika#2762 (l. 790-960), nommés `mika2980_*` en français selon l'usage du fichier.
4. Vérification : `cargo test -p mika-agent db::tests::reapers`, puis `cargo test`, `cargo clippy`, `cargo fmt`, `scripts/verify-pipeline.sh origin/main`.

Plancher du vol : les étapes 1 à 3 ensemble. L'étape 1 seule livrerait un prédicat sans contrôle négatif, donc sans preuve que le comportement voulu par mika#2750 tient encore.

---

## 7. Vérification

| ce qui doit être vrai | comment on le sait |
|---|---|
| la parente du cas mesuré n'est plus fauchée | cas 1 du § 8 (fixture de la table du § 1) |
| le reaper fauche toujours ce qu'il doit faucher | cas 2 — contrôle négatif, même fixture sans le second enfant |
| la protection est bornée | cas 3 — enfant `completed` hors fenêtre ⇒ fauchée |
| une complétion non datée ne protège pas | cas 4 — `completed_at IS NULL` ⇒ fauchée |
| le couple est symétrique | cas 5 — côté succès, même fixture, parente non complétable |
| rien d'autre n'a bougé | les cinq tests mika#2762 (l. 790-960) restent verts sans modification |

---

## 8. Les tests (AC2)

Tous dans `crates/mika-agent/src/db/tests/reapers.rs`, dans le même appel que le correctif.

1. `mika2980_un_enfant_de_groom_complete_protege_sa_parente` — **AC2 positif.** Parente manual / `in_progress` / `self_dev` sans `pr_url` ; enfant de groom `delivered` à T−40 min ; second enfant de groom `completed` à T−30 s, `completed_at` renseigné ; aucun frère `pending`/`in_progress`. `find_orphaned_parent_tasks("mika", 600, "groom")` ⇒ **vide**.
2. `mika2980_sans_le_second_enfant_la_parente_est_bien_fauchee` — **AC2, contrôle négatif.** Même fixture, second enfant retiré. ⇒ **un candidat**, et c'est la parente attendue. C'est ce cas qui sépare « la garde a été élargie » de « le reaper a été désarmé ».
3. `mika2980_une_completion_hors_fenetre_ne_protege_plus` — **la borne.** Second enfant `completed` avec `completed_at` à T−40 min (hors des 600 s). ⇒ **un candidat**. Sans ce cas, une clause non bornée passerait les cas 1 et 2 et rendrait la parente immortelle.
4. `mika2980_une_completion_sans_date_ne_protege_pas` — second enfant `completed` avec `completed_at IS NULL`. ⇒ **un candidat**. Épingle que l'absence de date se lit comme « non vivant », et non comme « vivant jusqu'à preuve du contraire ».
5. `mika2980_la_voie_de_succes_garde_pareil` — **AC3.** Fixture `create_completable_parent_setup` + `pr_url` posé, enfant `delivered` ancien, second enfant de groom `completed` récent. `find_completable_parent_tasks_on_pr_url("mika", 600)` ⇒ **vide**. Jumeau de `mika2762_la_voie_de_succes_sancre_pareil`.

Chaque cas pose sa fixture avec les helpers de `crates/mika-agent/src/db/tests/mod.rs` (`create_orphaned_parent_setup`, `create_completable_parent_setup`, `backdate_task_updated_at`) et le nouveau helper du § 6.2 — aucun `DELETE` ni `UPDATE` ad hoc sur une ligne existante, pour que la fixture se lise.

---

## 9. AC4 — la sonde d'après-déploiement

La sonde se lit sur la base de l'agent après `make deploy`, sur les **10 grooms suivants qui comptent deux enfants de groom sur la même parente**. Elle compte les `groom_parent_orphaned` dont le dernier enfant a fini `PLAN_GROOMED`. **Cible : 0.**

```sql
-- population : les parentes à ≥ 2 enfants de groom, les 10 plus récentes
WITH parentes AS (
  SELECT parent.id AS parent_id, MAX(child.created_at) AS dernier_enfant
  FROM tasks parent
  JOIN tasks child ON parent.id = child.parent_task_id
  WHERE COALESCE(child.dispatch_class, 'implement') = 'groom'
    AND child.trigger_type = 'callback'
    AND child.label NOT LIKE '%:deferred'
    AND parent.created_at > '<instant du deploy>'
  GROUP BY parent.id
  HAVING COUNT(*) >= 2
  ORDER BY dernier_enfant DESC
  LIMIT 10
)
SELECT
  (SELECT COUNT(*) FROM parentes) AS population,
  COUNT(*) AS orphelinages_sur_groom_converge
FROM parentes p
JOIN audit_events ae
  ON ae.event_type = 'groom_parent_orphaned'
 AND ae.entity_id  = p.parent_id
WHERE EXISTS (
  SELECT 1 FROM tasks c
  WHERE c.parent_task_id = p.parent_id
    AND COALESCE(c.dispatch_class, 'implement') = 'groom'
    AND c.result LIKE '%PLAN_GROOMED%'
    AND c.created_at = p.dernier_enfant
);
```

Deux règles de lecture, parce que sans elles la sonde peut mentir :

- **`population` est rendue à côté du compte, et elle n'est pas facultative.** Une sonde qui rend `0` sur une population de 0 ne dit rien ; absent n'est pas zéro (mika#2331). Tant que `population < 10`, la lecture est « pas encore concluante », et on attend.
- **Un non-zéro ne clôt pas l'affaire tout seul : il faut lire *pourquoi*.** Le résidu d'AC5 (livraison > 600 s après le `completed`) et un trou encore inconnu produisent la même ligne. Le départage se fait sur le `last_callback_at` du log `task_engine_reaper.evaluated` de la parente et sur l'écart `completed_at → callback_delivered` du dernier enfant. C'est cet écart qui décide si le ticket de suivi du § 4 s'ouvre.

Le résultat est écrit **sur le ticket mika#2980** — c'est ce que l'AC demande.

---

## Definition of Done

- [ ] `find_orphaned_parent_tasks` ne rend pas candidate une parente dont un frère de groom est `completed` avec un `completed_at` dans la fenêtre de grâce.
- [ ] La clause est bornée : `completed_at IS NOT NULL` **et** `completed_at > now − grace_seconds`, le paramètre `?2` déjà lié, aucune constante et aucune variable d'environnement nouvelle.
- [ ] `find_completable_parent_tasks_on_pr_url` porte le même bras, dans les mêmes termes.
- [ ] Les doc-comments des deux fonctions portent : le trou mika#2980, la raison du choix de la garde contre l'ancre, la raison du bornage, et le fait que `grace_seconds` sert désormais deux métiers.
- [ ] Les cinq tests du § 8 sont verts, dont le contrôle négatif (cas 2) et les deux contrôles de borne (cas 3 et 4).
- [ ] Les cinq tests mika#2762 existants sont verts, sans modification.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` passent.
- [ ] Le plan dit si les 14 min du ticket sont attendues ou un second défaut (§ 5 : attendues, avec la citation de `dispatch-lib.sh` l. 7850-7852).
- [ ] La requête de la sonde AC4 est dans le plan, avec ses deux règles de lecture (§ 9).
- [ ] Le résidu d'AC5 est nommé, avec sa condition d'ouverture et sa ligne `Tracked in:` (§ 4).

## Acceptance criteria

- **AC1.** Un enfant de groom `completed`, dont le callback n'est pas encore livré, protège sa parente du reaper : la garde de frère actif le compte, ou l'ancre de grâce le lit. Le plan tranche lequel, et dit pourquoi.
- **AC2.** Test dans le même appel, avec contrôle négatif. Parente avec un enfant de groom `delivered` à T−40 min et un second enfant `completed` à T−30 s, aucun frère actif, reaper à T : la parente reste `in_progress`. Même fixture, sans le second enfant : la parente est fauchée (le comportement voulu par mika#2750 tient).
- **AC3.** Le couple `find_completable_parent_tasks_on_pr_url` (doc-comment l. 2058-2066, « Any filter change here … MUST be applied symmetrically there ») est traité : changé symétriquement, ou le plan dit pourquoi ce changement-ci ne s'y applique pas.
- **AC4.** Sonde après déploiement : sur les 10 grooms suivants qui comptent deux enfants de groom sur la même parente, le compte de `groom_parent_orphaned` dont le dernier enfant a fini `PLAN_GROOMED` est écrit ici. La cible est 0.
- **AC5.** Après un orphelinage de parente de groom dont le dernier enfant a fini `PLAN_GROOMED`, le vol repart sans geste humain. Le correctif d'AC1 peut suffire, s'il empêche l'orphelinage. Sinon, le plan dit qui enchaîne dev-pilot sur un groom convergé dont la parente est terminale, par exemple un `already_groomed` qui dispatche l'implémentation quand la preuve existe. Test dans le même appel : plan convergé + parente orpheline ⇒ un dev-pilot part ; plan sans preuve ⇒ aucun dev-pilot.

## Fire-Disposition

**Option (a) — exception nommée en allowlist, et l'allowlist est livrée VIDE.**

Les détecteurs livrés par ce vol sont les cinq tests du § 8. Ils atterrissent **armés** et verts, sans aucune exception, et l'allowlist est vide pour une raison structurelle, pas par chance : chacun **construit sa propre fixture** en base de test (parente + enfants posés par les helpers du § 6.2). Aucun ne balaie une population existante, ni en base de production, ni dans l'arbre des sources. Il n'existe donc pas de « violation préexistante » à excepter — la seule donnée qu'un de ces tests puisse voir est celle qu'il vient d'écrire lui-même.

Les deux tests qui pourraient surprendre un futur éditeur en rougissant sont le cas 2 (contrôle négatif : la parente **doit** être fauchée) et le cas 3 (la borne : une complétion ancienne **doit** cesser de protéger). Leur rougeur ne signale jamais une violation à excepter : elle signale que la garde a été élargie trop loin — désarmée dans le cas 2, débornée dans le cas 3. La réponse est de réparer le prédicat, jamais d'ajouter une entrée d'allowlist.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db/tasks.rs` (deux prédicats + doc-comments) | 110 |
| `crates/mika-agent/src/db/tests/mod.rs` (helper de fixture `completed`) | 30 |
| tests (`crates/mika-agent/src/db/tests/reapers.rs`, 5 cas) | 250 |

Total estimé : 390 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 390 < 429 (borne par vol) et 390 < 1000 (seuil par PR) : les deux bornes passent, et la plus serrée — 429 — laisse 39 lignes de marge. Le volume est dominé par les tests (250 sur 390) ; côté production il s'agit de **deux ajouts de la même forme** dans deux requêtes SQL existantes, sans migration (`completed_at` existe depuis la v1), sans signature touchée (le paramètre `?2` est déjà lié), sans nouvelle constante et sans nouvelle variable d'environnement. Le ~200 lignes du filet de réhabilitation d'AC5 ferait sortir de la borne : il est renvoyé au suivi du § 4, gaté sur la lecture de la sonde AC4.
