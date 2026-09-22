# mika#2267 — vérification par injection (T2, T5, T6)

Doctrine `feedback_verify_pipeline_passes_without_the_fix` ; précédent
`todos/mika-manager-cadence-wiring-injection-verification.md`.

**Ce que ce document établit :** les trois tests porteurs du ticket rougissent
quand on retire le correctif qu'ils prétendent tenir. Un test qui reste vert
sans son correctif n'atteste rien, et c'est précisément la forme d'échec que
mika#2267 ferme une couche plus bas — une surface qui se tait exactement comme
une surface saine.

Les trois inversions ont été appliquées une par une, mesurées, puis **restaurées
et re-vérifiées au vert** (§ Retour au vert, en fin de document).

---

## T2 — `mika2267_reports_absent_dir_is_distinguishable_from_empty_dir`

**Ce qu'il tient :** un puits **absent** et un puits **vide** sont deux états
distincts. Sans cette distinction, le lecteur reproduirait à sa propre surface
le silence qu'il existe pour lever : « la cadence n'a jamais écrit ici » et
« la cadence a écrit ici et le puits a été vidé » rendraient les mêmes octets.

**Inversion** — `sink_dir.rs`, `list_sink_reports_in`, branche `Err` de
`read_dir` : `SinkListing::DirAbsent { … }` → `SinkListing::Empty { … }` (les
deux branches fondues, ce qui est l'implémentation naïve).

**Mesure :**

```
---- milestone_manager::sink_dir::tests::mika2267_reports_absent_dir_is_distinguishable_from_empty_dir stdout ----
panicked at crates/mika-agent/src/milestone_manager/sink_dir.rs:387:9:
Empty { dir: "/tmp/.tmpUgb0Rd/il-nexiste-pas", source: Default }

test result: FAILED. 0 passed; 1 failed
```

La ligne rendue par l'assertion **nomme l'état fautif** (`Empty` sur un chemin
qui n'existe pas), donc le rouge est attribuable sans lire le diff.

---

## T5 — `mika2267_http_failure_fallback_emits_its_own_route`

**Ce qu'il tient :** le repli après échec HTTP émet sa propre ligne
`manager_cycle_delivered`, sous `route = "offline_sink_fallback"`. C'était le
seul des trois chemins d'écriture à n'émettre **rien** : un rapport écrit sans
qu'aucune ligne ne dise qu'il l'avait été, ni où.

**Inversion** — `cadence.rs`, branche `Err(e)` de `Route::Http` : le
`tracing::info!` ajouté par C4 est retiré (état d'avant le correctif, à
l'identique).

**Mesure :**

```
---- mika2267_http_failure_fallback_emits_its_own_route stdout ----
panicked at crates/mika-agent/tests/manager_delivery_observability_2267.rs:237:5:
assertion `left == right` failed: exactement une ligne de livraison — avant le correctif, zéro : []
  left: 0
 right: 1

test result: FAILED. 0 passed; 1 failed
```

Le `[]` du message **est** la mesure du défaut fondateur : zéro ligne sur ce
chemin. C'est le seul des trois où le rouge affiche littéralement l'état
d'avant.

**Contrôle négatif associé, non injecté mais porteur :**
`mika2267_no_url_configured_is_the_nominal_sink_not_a_fallback`. Un correctif
qui aurait réutilisé `offline_sink` pour le repli aurait rendu T5 vert tout en
effaçant la distinction que le ticket demande de faire. Les deux tests
s'assertent mutuellement.

---

## T6 — `mika2267_delivery_resolved_carries_the_resolved_path_and_never_the_token`

**Ce qu'il tient :** `manager_delivery_resolved` porte le chemin résolu et sa
provenance, et **jamais** de matériel de credential — `delivery_token_present`
est un booléen, doctrine `api_key_present` du `CLAUDE.md` racine.

**Inversion** — `spawn.rs`, `emit_delivery_resolved` : ajout d'un champ
`delivery_token = cfg.delivery_token.as_deref().unwrap_or("")`.

**Mesure :**

```
---- mika2267_delivery_resolved_carries_the_resolved_path_and_never_the_token stdout ----
panicked at crates/mika-agent/tests/manager_delivery_observability_2267.rs:331:9:
la valeur du token a fuité dans le champ `delivery_token`

test result: FAILED. 0 passed; 1 failed
```

L'assertion **nomme le champ fautif**, ce qui est la propriété qui compte : le
balayage porte sur *tous* les champs, pas seulement celui du token, parce
qu'une fuite arrive par le champ qu'on n'a pas pensé à regarder. Le test
refuse aussi un **préfixe** de huit caractères — huit suffisent à identifier un
secret, et « on ne journalise qu'un préfixe » est la forme sous laquelle cette
fuite revient.

---

## Ce qui n'est PAS injection-vérifié, et pourquoi

**T3** (`mika2267_sink_dir_resolution_has_a_single_reader`), **T4**
(`mika2267_delivery_route_names_are_a_wire_format`) et **T7**
(`mika2267_every_manager_env_const_is_declared_in_env_example`) sont des scans
de source et des épinglages de constantes. Leur inversion est triviale par
construction — ajouter un second site, changer une lettre d'une constante — et
n'apprend rien : ils ne tiennent pas un comportement mais une **forme**.

Ce qu'ils tiennent est autre chose, et c'est la raison de leur existence :
leur régression ne rendrait aucune décision fausse, elle la rendrait
**invisible**. Un second résolveur du chemin du puits ne diverge pas le jour
où il est écrit ; il diverge plus tard, en silence, et toutes les assertions
comportementales restent vertes pendant que l'opérateur reperd ses rapports.
Aucun test de comportement ne peut attraper cette classe.

T7 a d'ailleurs attrapé un vrai défaut **pendant** l'implémentation : son
premier jet tronquait chaque fichier à son premier `#[cfg(test)]`, et
`spawn.rs` porte un helper `#[cfg(test)]` **au-dessus** de son bloc de
constantes — le scan ne voyait qu'une déclaration sur dix et serait passé au
vert en n'ayant rien vérifié. Corrigé (scan du fichier entier, seuls les
*fichiers* de test sont écartés) et gardé par l'assertion
`declared.len() >= 10`.

---

## Retour au vert

Les trois inversions ont été restaurées à l'identique. État final :

```
cargo test -p mika-agent --test manager_delivery_observability_2267
running 5 tests
test mika2267_delivery_route_names_are_a_wire_format ... ok
test mika2267_delivery_resolved_names_the_sink_when_no_url_is_posted ... ok
test mika2267_delivery_resolved_carries_the_resolved_path_and_never_the_token ... ok
test mika2267_http_failure_fallback_emits_its_own_route ... ok
test mika2267_no_url_configured_is_the_nominal_sink_not_a_fallback ... ok
test result: ok. 5 passed; 0 failed
```

Le détail de la suite complète (lib + CLI) est dans le corps de la PR.
