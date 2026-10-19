# mika#2814 — vérification par injection (AC1, AC2, AC3)

Doctrine `feedback_verify_pipeline_passes_without_the_fix` ; précédent immédiat
`todos/1959-injection-verification.md`, dont ce ticket ferme le résidu D5.

**Ce que ce document établit :** les tests de mika#2814 rougissent quand on
retire le correctif qu'ils prétendent tenir, et le mode de panne le plus
probable de ce vol — *une fonction pure testée, un câblage oublié* — a été
mesuré avant d'être corrigé. Un détecteur dont on n'a pas vu le rouge n'est pas
un détecteur.

Les quatre cas du plan § *Vérification par injection* ont été appliqués un par
un, mesurés, puis **restaurés et re-vérifiés au vert** (§ Retour au vert).

---

## Cas 1 — le filtre écrit, mais **non câblé**

**Ce qu'il mesure :** que le test e2e ne tient pas la fonction pure, mais la
publication réelle. `retain_publiables` existait, était complet et correct ;
l'appel manquait dans `connect_all` (étape 2 du plan). C'est l'état exact d'un
contributeur qui livre la logique et oublie le fil — et le seul état où la
suite unitaire est **entièrement verte** alors que rien n'est tenu.

**Mesure :**

```
---- eval::test_mcp_testimony_non_publication_2814::mika2814_un_outil_mcp_testimony_natteint_jamais_le_modele stdout ----
panicked at crates/mika-agent/tests/eval/test_mcp_testimony_non_publication_2814.rs:141:
les deux outils d'un serveur testimony ne doivent pas être publiés, or le manager
en porte : ["mcp__journal__list_environments", "mcp__journal__create_local_env_file"]

test result: FAILED. 1 passed; 1 failed
```

Le rouge **nomme les deux outils trouvés**, ce qui est la réparation : il dit
quel serveur publie et quels noms atteignent le modèle.

**Le contrôle positif AC2 reste vert sous ce cas**, et c'est la propriété qui
compte : il mesure qu'un serveur `operational` publie, ce qui est vrai avec ou
sans le filtre. Les deux tests ne sont donc pas redondants — ils ne rougissent
jamais ensemble.

### Cas 1 bis — le rouge de l'assertion AC1 elle-même

Le cas 1 s'arrête à l'assertion de structure (ligne 141), **avant** celle qui
porte la mesure d'AC1 sur les octets soumis au modèle (ligne 175). Vu le
principe qui ouvre ce document, l'assertion porteuse ne pouvait pas rester une
assertion dont on n'a jamais vu le rouge. L'assertion de structure a donc été
neutralisée un instant, dans le même état non câblé :

```
panicked at crates/mika-agent/tests/eval/test_mcp_testimony_non_publication_2814.rs:175:
AC1 ÉCHOUE : mcp__journal__list_environments est dans ce qui a été soumis au
modèle — il peut donc encore être proposé, ce que la doctrine mika#1798 interdit
au même titre que l'accès. Requêtes : LlmRequest { model: "mock-model", system: …
```

La requête complète est rendue dans le message : c'est la surface d'AC1, et la
seule qui dise ce que le modèle a *vu* plutôt que ce que le manager *porte*.

---

## Cas 2 — le filtre dégénéré (`return Vec::new()`)

**Ce qu'il mesure :** qu'aucune assertion d'absence ne soit tenue par un filtre
qui dépublie tout. Trois des quatre assertions du ticket mesurent une absence,
et l'absence est aussi ce que rend un instrument qui ne mesure rien.

**Inversion** — `mcp/mod.rs`, `retain_publiables` : corps remplacé par un
`return Vec::new()` inconditionnel.

**Mesure, unitaire :**

```
test mcp::tests::mika2814_seuls_les_outils_testimony_sont_depublies ... FAILED
test mcp::tests::mika2814_une_definition_hors_map_est_publiee ... FAILED
test mcp::tests::mika2814_controle_une_table_sans_testimony_ressort_intacte ... FAILED
test mcp::tests::mika2814_le_routage_et_les_grades_ne_sont_pas_amputes ... ok

assertion `left == right` failed: aucun grade testimony ⇒ aucune dépublication,
et l'ordre préservé (c'est l'ordre dans lequel le modèle reçoit les outils)

test result: FAILED. 1 passed; 3 failed
```

**Mesure, e2e :**

```
test eval::…::mika2814_controle_positif_un_serveur_operational_est_publie ... FAILED
test eval::…::mika2814_un_outil_mcp_testimony_natteint_jamais_le_modele ... ok

AC2 ÉCHOUE : un serveur operational doit publier ses deux outils, or le manager porte []

test result: FAILED. 1 passed; 1 failed
```

**Les deux verts de ce cas sont la raison d'être des quatre tests.** AC1 reste
vert — un filtre qui dépublie tout dépublie *aussi* le testimony ;
`mika2814_le_routage_et_les_grades_ne_sont_pas_amputes` reste vert, parce que sa
première assertion attend justement une liste vide. Un ticket livré avec ces
deux tests seuls serait passé en revue, passé en CI, et n'aurait rien tenu :
MCP cassé entièrement se lit comme MCP protégé. Ce sont le contrôle positif AC2
et le contrôle de bonne foi unitaire qui l'interdisent, et aucun des deux
n'était demandé par le ticket.

---

## Cas 3 — sans les compteurs : **rien ne rougit, et c'est attendu**

**Inversion** — retirer `discovered` / `published` / `unpublished` de la ligne
`"MCP manager initialized"` et les lignes `mcp_tool_testimony_unpublished`.

**Mesure — l'absence de lecteur, établie par recherche plutôt que par exécution :**

```
$ grep -rn "MCP manager initialized" crates/ scripts/ skills/ docs/ dashboard/src
crates/mika-agent/src/mcp/mod.rs:239:                "MCP manager initialized"
docs/plans/2026-10-19-006-feat-2814-…-plan.md:129, 215, 370, 391
```

Aucun test, aucun script, aucun tableau de bord ne lit cette ligne : la retirer
ne peut donc rougir nulle part, et une assertion sur un `tracing` global serait
soit fragile, soit silencieusement vide dans le runtime multi-thread des tests —
c'est-à-dire verte pour la mauvaise raison (même arbitrage que mika#2281, § *Ce
que ce test mesure*).

**Conséquence assumée, pas dissimulée :** l'observabilité de ce ticket n'est pas
tenue par la suite. Elle est tenue par une sonde opérateur, à exécuter **après**
`make deploy` (avant, l'instrument n'existe pas dans le binaire servi —
classe mika#2340) :

```
grep '"event":"mcp_tool_testimony_unpublished"' ~/.mika/logs/mika-spirit.log
grep '"message":"MCP manager initialized"'      ~/.mika/logs/mika-spirit.log
```

La seconde doit porter `discovered`, `published` et `unpublished` — ce dernier
**même à zéro** : absent ne vaut pas zéro (mika#2331), et l'absence de la ligne
signifie un binaire antérieur à ce ticket, jamais « rien n'a été banni ».

---

## Cas 4 — l'état d'`origin/main`, rendu mesurable

**Ce qu'il mesure :** que les deux nouveaux fichiers de test rougissent sur le
tronc, et que les tests de mika#1959 y restent verts — donc que ce ticket
*ajoute* une porte sans déplacer la précédente.

L'injection littérale (`git checkout origin/main`) ferait disparaître les tests
avec le correctif, et un test absent ne rougit pas : la mesure serait vide. La
forme équivalente et mesurable est un **passe-plat** — `retain_publiables` câblé,
rendant ses définitions inchangées, c'est-à-dire le comportement d'`origin/main`
avec les tests présents.

**Mesure :**

```
test mcp::tests::mika2814_le_routage_et_les_grades_ne_sont_pas_amputes ... FAILED
test mcp::tests::mika2814_une_definition_hors_map_est_publiee ... FAILED
test mcp::tests::mika2814_seuls_les_outils_testimony_sont_depublies ... FAILED
test mcp::tests::mika2814_controle_une_table_sans_testimony_ressort_intacte ... ok
test mcp::tests::mika1959_chaque_outil_porte_le_grade_de_son_serveur ... ok
test mcp::tests::mika1959_un_serveur_sans_champ_ou_absent_vaut_operational ... ok
test result: FAILED. 3 passed; 3 failed

test eval::…::mika2814_un_outil_mcp_testimony_natteint_jamais_le_modele ... FAILED
test eval::…::mika2814_controle_positif_un_serveur_operational_est_publie ... ok
test eval::test_mcp_secret_boundary_2281::mika2281_mcp_channel_never_carries_the_secret_value ... ok
test eval::test_mcp_secret_boundary_2281::mika2281_positive_control_the_file_channel_does_carry_it ... ok
test result: FAILED. 3 passed; 1 failed

$ cargo test -p mika-agent --lib mika1959
test result: ok. 10 passed; 0 failed
```

**Les dix verts de mika#1959 sous cette injection sont l'AC5 de ce ticket.** La
garde d'appariement
`mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4` compte les
sites de publication ; la décision D1 filtre **dans `connect_all`**, donc aucun
des deux sites ne bouge, les deux ensemencements restent, et le `== 2` reste
exact. Le ticket demandait de la mettre à jour dans le même commit plutôt que de
la contourner : son **code** n'avait rien à changer, et seul son commentaire a
gagné une phrase disant que l'inventaire qu'elle compte est désormais
pré-filtré. C'est cette injection qui l'établit, pas une relecture.

---

## Retour au vert

Les trois inversions exécutables ont été restaurées à l'identique.

```
$ cargo test -p mika-agent --lib "mcp::tests::mika"
test mcp::tests::mika1959_chaque_outil_porte_le_grade_de_son_serveur ... ok
test mcp::tests::mika1959_un_serveur_sans_champ_ou_absent_vaut_operational ... ok
test mcp::tests::mika2814_seuls_les_outils_testimony_sont_depublies ... ok
test mcp::tests::mika2814_une_definition_hors_map_est_publiee ... ok
test mcp::tests::mika2814_le_routage_et_les_grades_ne_sont_pas_amputes ... ok
test mcp::tests::mika2814_controle_une_table_sans_testimony_ressort_intacte ... ok
test result: ok. 6 passed; 0 failed

$ cargo test -p mika-agent --test eval -- mika2814 mika2281
test result: ok. 4 passed; 0 failed
```

*(Les numéros de ligne cités dans les mesures ci-dessus sont ceux de l'état
**injecté**, qui porte de deux à quatre lignes de plus ou de moins selon le cas,
et ils ont encore bougé au `cargo fmt` final. Les ancres durables sont les
messages, pas les lignes : l'assertion de structure du cas 1 est celle qui dit
« or le manager en porte », celle d'AC1 celle qui ouvre par « AC1 ÉCHOUE », et le
contrôle de bonne foi unitaire celle qui dit « aucun grade testimony ⇒ aucune
dépublication ». Au commit de livraison elles sont respectivement à
`test_mcp_testimony_non_publication_2814.rs:139`, `:173` et `mcp/mod.rs:968`.)*

Le détail de la suite complète (workspace, clippy, lints dépôt,
`scripts/verify-pipeline.sh`) est dans le corps de la PR.

## Refs

- Parent : mika#1798 (bake de l'invariant non-transit)
- Précédent immédiat : mika#1959 (décision D5, le résidu que ce ticket ferme)
- Doctrine : `crates/mika-agent/docs/non-transit-data-grade.md` § *Layer 2 —
  Non-publication* (sous-section *MCP side — publication filter*) et
  § *Vigilance surface*
- Vol 2 (hors de ce commit) : le helper unique publication + ensemencement, et
  l'avertissement sur une clé `mcp.json` inconnue — `senara-solutions/mika#2814`
