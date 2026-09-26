# Corps d'issue figés — reconnaissance du verdict de grooming (mika#2158)

Six corps de ticket, un fichier par ticket, qui sont le **jeu de mesure** du prédicat
`mika_agent::grooming_marker::grooming_verdict`. Ils existent parce que le prédicat a été
corrigé contre eux, pas contre des exemples inventés après coup.

## Ne pas rafraîchir

**Ce sont des corps historiques figés. Ne les mettez pas à jour depuis GitHub.**

Un rafraîchissement effacerait précisément les formes que le correctif doit reconnaître : les
trois tickets qui échouaient (#2127, #2108, #1772) seront un jour re-groomés, réécrits ou
fermés, et la forme qui a cassé le prédicat disparaîtra avec eux. Le jour où elle disparaît,
le test devient vert pour la mauvaise raison.

## Provenance — ce qui est mesuré, ce qui est reconstruit

Cette distinction est le point entier de mika#2034 : dire ce qui a été mesuré, pas produire une
attestation à côté de ce qu'elle atteste. Ligne par ligne :

| ligne | provenance |
|---|---|
| `> - **Grooming history:**` | **mesurée.** L'extrait décisif est celui du plan mika#2158 §1.1, relevé sur GitHub le 2026-09-03. Les segments que le relevé élide (`…`) sont rendus par un remplissage générique, jamais par un token de verdict : aucun `GROOMED` ni `ESCALATE` n'a été ajouté ou retiré. |
| `> - **Branch:**` | **reconstruite** depuis le nom de branche réel du dépôt (`git branch -r`). |
| `> - **Plan:**` | **reconstruite** depuis le nom de fichier de plan réel, relevé sur la branche du ticket (`git ls-tree`). Le SHA de commit est un remplissage. |
| prose descriptive | **omise.** Elle ne participe à aucun des trois prédicats et son absence ne change aucun verdict. |

La capture littérale intégrale n'a pas pu être faite : la session qui a implémenté mika#2158
tournait dans un bac à sable sans accès GitHub (`gh` non authentifié, pas de jeton, sortie
réseau refusée). Ce que le prédicat lit — la ligne de callout `Grooming history`, et la
présence des callouts `Branch`/`Plan` — est fidèle ; le reste du corps ne l'est pas.

**Si vous avez l'accès et souhaitez compléter la capture**, remplacez le corps entier :

```sh
for n in 2127 2140 2108 1772 2151 2117; do
    gh issue view "$n" --repo senara-solutions/mika --json body --jq .body \
        > "crates/mika-agent/tests/fixtures/grooming_bodies/$n.md"
done
```

…puis relancez `cargo test -p mika-agent grooming`. Le tableau attendu ci-dessous ne doit pas
bouger. **S'il bouge, ne corrigez pas le tableau : le ticket a été réécrit depuis, et c'est la
capture qu'il faut abandonner, pas l'attente.** Une fois la capture littérale faite, remplacez
la présente section par la date de capture et gelez définitivement.

## Le tableau attendu

Ce que le prédicat rendait avant mika#2158, et ce qu'il doit rendre après. Trois vrais avant,
six après.

| fixture | avant | après | ce que la fixture protège |
|---|---|---|---|
| `2127.md` | false | **true** | AC3 — un `GROOMED` final rendu après un `ESCALATE` de seconde passe et son arbitrage |
| `2140.md` | true | true | non-régression — la forme canonique `second-pass (GROOMED)` |
| `2108.md` | false | **true** | AC1 — une première passe `READY` sans seconde passe (chemin prescrit par `mika-groom-ticket.md` phase 3 étape 10) |
| `1772.md` | false | **true** | AC2 — la variante française `seconde passe (GROOMED, …)` |
| `2151.md` | true | true | non-régression — forme canonique, producteur nommé |
| `2117.md` | true | true | non-régression — forme canonique, producteur nommé |

Les tests qui portent ce tableau vivent dans `crates/mika-agent/src/grooming_marker.rs` :
`fixture_table` (le verdict seul), `ac7_both_rust_predicates_agree_on_the_frozen_bodies` (le
croisement `auto_pull::is_groomed` ↔ `executor::check_grooming_markers`) et, côté
`auto_pull.rs`, `test_is_groomed_six_frozen_bodies` (conditions `Branch`/`Plan` comprises).
Le troisième porteur — la garde Bash de `dispatch-lib.sh` — est croisé contractuellement par
`test_groom_gate_refusal_implies_rust_says_groomed` dans
`skills/bundled/_shared/test-dispatch-lib.sh`.

## Références

- `crates/mika-agent/src/grooming_marker.rs` — le prédicat, seule lecture du marqueur de verdict
- `docs/plans/2026-09-03-001-fix-2158-un-seul-predicat-detat-de-grooming-plan.md` §1.1 — le relevé
- `docs/solutions/dev-loop/two-predicates-for-one-concept-livelock-2026-09-03.md` — la classe
