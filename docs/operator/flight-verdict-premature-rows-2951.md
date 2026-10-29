# Retirer les trois verdicts prématurés du 2026-10-28 — procédure opérateur (mika#2951 AC3/D4)

> **Geste unique, borné et terminal.** Il porte sur **trois lignes nommées** de
> `audit_events`, inscrites le 2026-10-28 par le défaut que mika#2951 corrige.
> Ce n'est pas un rattrapage général : il n'y a pas d'autre population à nettoyer,
> et le correctif ferme l'avenir de lui-même.

## Pourquoi c'est un geste opérateur, et pas du code

Le correctif de mika#2951 (ajournement d'une parente dont le pilote
d'implémentation vole encore) ferme **l'avenir** : plus aucun verdict ne peut
être rendu avant la fin du vol, donc `flight_verdict_ledger::write_and_swallow`
ne gèlera plus de ligne prématurée. **Aucune ligne de `flight_verdict_ledger.rs`
n'a été écrite par ce vol** — l'idempotence du registre reste ce qu'elle est.

Les trois lignes déjà inscrites, elles, ne sont pas réécrites par le programme.
Trois raisons, dont la troisième est une contrainte mesurée :

1. **Un journal d'audit n'est pas une table mutable par le programme.** Le
   registre a un écrivain unique gardé (`MANAGER_FLIGHT_VERDICT_TOOL`, garde
   AC2 de mika#2922) ; lui adjoindre un correcteur en fait deux.
2. **Un rattrapage automatique demanderait à `write_and_swallow` de savoir
   *quand* un verdict antérieur fut prématuré.** Il ne peut pas le savoir sans
   rejouer la topologie des enfants à l'instant de l'écriture — c'est-à-dire
   sans réimplémenter l'AC1 à l'envers, dans le seul site qui doit rester bête.
3. **La suppression ne provoquera PAS un re-jugement de ces trois vols**, et
   c'est dit ici plutôt que laissé à croire. `milestone_manager/cadence.rs`
   borne la lecture par `checkpoint.last_observed_at`. Depuis le correctif cette
   borne porte sur la fin du **vol** — la parente passe la fenêtre si sa propre
   terminalité *ou* celle d'un de ses enfants est postérieure au point de
   reprise — et c'est ce qui fait qu'une parente ajournée revient au cycle
   suivant au lieu d'être oubliée. Mais pour ces trois vols-ci, tout est passé
   le 2026-10-28 : la parente, le pilote de groom et le pilote
   d'implémentation. Le point de reprise étant désormais au-delà des trois,
   rien ne les ramène. **Trois lignes fausses deviennent trois absences** — et
   une absence dit « non jugé », ce qui est vrai : le manager n'a jamais jugé
   correctement ces vols. Elle ne dit pas « pas de vol », ce que la ligne
   actuelle dit faussement.

**Reculer `last_observed_at` pour forcer le re-jugement n'est pas proposé** : le
cycle rejouerait toute la fenêtre et son rapport, pour trois lignes.

## Les trois lignes, telles que mesurées le 2026-10-28

| parente (`tasks.id`, préfixe) | ticket | vrai résultat du vol | ligne du registre |
|---|---|---|---|
| `9bed44f8…` | mika#2944 | PR #2947 ouverte, 197 tours, 13:20:48Z | 12:31:59Z — mort `PLAN_GROOMED` |
| `c5848eaa…` | mika#2946 | PR #2949 ouverte, 148 tours, 15:09:42Z | 14:26:59Z — mort `PLAN_GROOMED` |
| `9f9be2a1…` | mika#2948 | PR #2950 ouverte, 198 tours, 16:37:20Z | 15:31:58Z — mort `PLAN_GROOMED` |

Les trois PR ont été mergées. Les `tasks.id` sont des UUID : les préfixes
ci-dessus sont ce que le rapport affiche, **pas** la valeur de `target_key`.
L'étape 1 résout les clés entières ; ne les devinez pas.

## Précondition — le correctif doit être déployé

```bash
mika --version                       # le binaire servi
cat ~/.mika/skills/.manifest-writer  # sha attesté du dernier seed
```

Si le binaire en service précède le correctif, **ne faites rien** : le défaut
inscrirait de nouvelles lignes prématurées au cycle suivant, et le geste serait
à refaire. La sonde qui l'atteste est la ligne INFO
`flight_deferred_pending_implement` (cible `mika::milestone_manager`), émise à
chaque cycle qui ajourne au moins une parente :

```bash
journalctl -u mika-spirit --since today | grep flight_deferred_pending_implement
```

**Son absence signifie un binaire antérieur au correctif** (motif mika#2340),
jamais « rien n'a été ajourné ».

## Étape 1 — le `SELECT` de contrôle

Montre les lignes du registre pour ces trois parentes, et **seulement** elles.

```sql
-- sqlite3 ~/.mika/data/mika.db
SELECT target_key, created_at, after_value, reasoning
  FROM audit_events
 WHERE tool_name = 'manager_flight_verdict'
   AND (target_key LIKE '9bed44f8%'
     OR target_key LIKE 'c5848eaa%'
     OR target_key LIKE '9f9be2a1%')
 ORDER BY created_at;
```

**Trois lignes doivent sortir, pas deux, pas quatre.** Chacune doit porter un
`after_value` dont l'`issue` est `plan_groomed` (ou `not_a_flight` si le cycle a
tourné sous mika#2949 avant la lecture) et un `created_at` du 2026-10-28
antérieur à l'heure de fin du pilote d'implémentation du tableau ci-dessus.

Si le compte diffère, **arrêtez-vous ici** et reportez ce que la requête rend :
un compte inattendu veut dire que la population n'est pas celle que ce runbook
décrit, et la suite serait un geste à l'aveugle. Relevez les `target_key`
entiers que cette requête rend — l'étape 2 les demande littéralement.

## Étape 2 — le `DELETE` borné

Trois `target_key` **littéraux**, substitués depuis la sortie de l'étape 1.
Aucun `LIKE`, aucune plage de dates, aucun `tool_name` seul : la borne est la
liste des trois clés, et c'est ce qui rend le geste relisible.

```sql
-- sqlite3 ~/.mika/data/mika.db
-- Remplacer les trois UUID par ceux que l'étape 1 a rendus.
DELETE FROM audit_events
 WHERE tool_name = 'manager_flight_verdict'
   AND target_key IN (
         '<uuid-complet-9bed44f8…>',
         '<uuid-complet-c5848eaa…>',
         '<uuid-complet-9f9be2a1…>'
       );
```

`changes()` doit rendre **3**. Le `tool_name` reste dans la clause : les lignes
de MPC (`mpc_flight_judgment`) portent le **même** `target_key` et ne sont pas
visées — ce sont les jugements humains, et ce sont eux qui disent la vérité sur
ces trois vols.

## Étape 3 — le `SELECT` de re-contrôle

La même requête que l'étape 1. Elle doit rendre **zéro ligne**.

```sql
SELECT COUNT(*) AS restantes
  FROM audit_events
 WHERE tool_name = 'manager_flight_verdict'
   AND (target_key LIKE '9bed44f8%'
     OR target_key LIKE 'c5848eaa%'
     OR target_key LIKE '9f9be2a1%');
```

Et le côté MPC doit être intact — c'est le contrôle négatif du geste :

```sql
SELECT COUNT(*) AS jugements_mpc_intacts
  FROM audit_events
 WHERE tool_name = 'mpc_flight_judgment'
   AND (target_key LIKE '9bed44f8%'
     OR target_key LIKE 'c5848eaa%'
     OR target_key LIKE '9f9be2a1%');
```

## Étape 4 — ce que la concordance montre après, et ce qu'elle ne montre pas

```bash
mika flight-verdicts concordance
```

**Le compte de concordance de mika#2698 ne remonte pas de trois d'autant.** Il
cesse d'être **contredit**, ce qui n'est pas la même chose et ne doit pas être
compté comme tel : là où trois lignes affirmaient « mort — cause non classée
(`PLAN_GROOMED`) · reprise à la main » contre trois PR mergées, il y a
désormais trois **absences**. Une absence dit « non jugé » ; elle ne compte ni
pour ni contre la concordance.

Le compte remonte au **prochain** groom suivi d'un pilote d'implémentation —
c'est la sonde AC4 de mika#2951 :

> Au prochain groom suivi d'une relance, `concordance` montre la parente avec
> l'issue du pilote d'implémentation (PR ouverte et son numéro), et non
> `PLAN_GROOMED` ni `not_a_flight`.

## Ce que ce geste ne laisse pas derrière lui

Rien. La population est close (trois lignes nommées), le correctif ferme
l'avenir, et aucun suivi n'est ouvert : le geste est complet et terminal.
