---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: mika#2908
branch: feat/2908/gateway-le-400-bot-username-mismatch
---

# feat(gateway): le 400 `bot_username` mismatch porte un code machine fermé

## Problem Frame

`POST /admin/customers` refuse un `bot_username` qui ne correspond pas à ce que
`getMe` a renvoyé. Le corps de ce refus ne contient **que de la prose** :

```rust
// crates/mika-gateway/src/routes.rs:1152-1156
fn bot_username_mismatch_body(provided: &str, actual: &str) -> serde_json::Value {
    serde_json::json!({
        "error": format!("bot_username mismatch: provided '{provided}' but Telegram returned '{actual}'")
    })
}
```

mika#2191 (PR #2410, `bb18681d`) a donné au refus du **token** un champ fermé,
`upstream_status`. Il ne l'a volontairement pas fait pour cette branche-ci : le
mismatch vit sur le bras `Ok` de `getMe` (`routes.rs:1209-1221`), Telegram a
répondu 200 et n'a rien refusé, donc il n'y a aucun statut d'amont à propager.
Le doc comment de la fonction le dit déjà, et
`mika2191_ac3_les_branches_non_401_ne_portent_pas_401` (`routes.rs:4334-4338`)
l'épingle.

**La conséquence est côté console.** mika-cloud#230 AC1 demande que le mismatch
remonte comme une cause distincte, pour que l'écran envoie l'utilisateur
corriger l'*username* et non le *token*. Le classifier ne peut le faire
aujourd'hui qu'en lisant la prose, ce que #230 AC3 interdit explicitement
(« la classification repose sur le statut d'amont ou un code machine, jamais sur
le texte du message »). mika-cloud#230 a livré AC2, AC3 et AC4 et laisse **AC1
en dormeur**, avec un test qui épingle le refus actuel
(`ac1_deployed_mismatch_body_carries_nothing_closed_and_stays_generic`). Ces
utilisateurs lisent encore « Your telegram bot token is still saved — you can
retry without re-entering it », et leur Retry renvoie les mêmes entrées : il ne
peut pas aboutir.

## Requirements

- **R1.** Le corps rendu par `bot_username_mismatch_body` contient une clé
  `code`, de valeur fixe et indépendante de `provided` / `actual`. (AC1)
- **R2.** La valeur est exactement `BOT_USERNAME_MISMATCH`, et un test l'asserte
  **en toutes lettres**, pas contre la constante qui la porte. (AC1)
- **R3.** Les corps de `token_validation_error_body` sont inchangés sur
  **toutes** leurs variantes, et aucun ne porte de clé `code`. (AC2)
- **R4.** Le test `mika2191_ac1_le_401_porte_le_statut_damont` et les trois
  autres tests du module `mika2191_upstream_status` restent verts **sans
  modification**. (AC2)
- **R5.** Le corps du mismatch ne porte **pas** `upstream_status`, et le
  contrôle négatif existant qui l'affirme reste vert. (AC2, par cohérence)
- **R6.** Le statut HTTP du mismatch reste `400` et son champ `error` garde son
  libellé au caractère près. (AC3)
- **R7.** La ligne `POST /admin/customers` de la table *Endpoints* de
  `crates/mika-gateway/CLAUDE.md` nomme le nouveau champ et la règle qui décide
  de sa présence.

## Non-objectifs

- **Donner un `code` aux refus du token.** AC2 l'interdit en toutes lettres, et
  c'est la bonne lecture : `upstream_status` répond déjà à la question « l'amont
  a-t-il refusé, et avec quel statut ». Deux vocabulaires pour une même question
  est la table de correspondance inter-dépôt que mika#2191 KTD1 a refusé de
  créer.
- **Ouvrir un vocabulaire de codes.** Ce vol livre **un** symbole, pour la seule
  cause que `upstream_status` ne peut structurellement pas porter. Une deuxième
  cause sans refus d'amont (il n'en existe pas aujourd'hui sur ce chemin) ouvrira
  son propre ticket. Pas d'`enum`, pas de module `error_codes`, pas de registre :
  YAGNI à un seul membre.
- **Changer le `error` du mismatch.** Il reste un format de fil tant que le
  classifier de mika-cloud n'a pas basculé sur le `code` — et ce basculement est
  mika-cloud#230, pas ce vol. Le reformuler ici casserait les deux chemins le
  même jour.
- **Côté console.** La lecture du `code` par `classify_gateway_error` et l'écran
  qui renvoie corriger l'username vivent dans `mika-cloud`, hors de ce workspace.
  *Tracked in: senara-solutions/mika-cloud#230 (AC1).*
- **Tester le handler de bout en bout.** Le module de tests existant explique
  pourquoi (`routes.rs:4265-4271`) : `AppState.pool` est un `PgPool` non
  optionnel et `api_url` code en dur `https://api.telegram.org`. C'est
  l'extraction des corps en fonctions qui rend le contrat assertable, et ce vol
  s'y tient.

## Approche

### D1 — Pourquoi un `code` ici alors que mika#2191 a tranché pour `upstream_status`

Cet arbitrage antérieur est enregistré, pas rouvert. mika#2191 KTD1 écrit :
« un `code: "invalid_bot_token"` exigerait un nouveau symbole à chaque nature de
refus et une table de correspondance à tenir dans deux dépôts ; l'entier n'en
demande aucune ». Cela reste vrai **pour les refus d'amont** — et le mismatch
n'en est pas un. Il n'y a aucun statut à propager, puisque Telegram a répondu
200 et a accepté le token.

Le choix n'est donc pas « `code` contre `upstream_status` » : c'est le seul champ
fermé disponible pour une classe que `upstream_status` ne peut pas exprimer sans
mentir. Poser `upstream_status: 200` serait « factuellement vrai et
catégoriquement faux », selon la formule du plan de mika#2191 — c'est exactement
ce qu'il a refusé de faire. Les deux champs restent disjoints : AC2 l'exige, R3
le teste.

### D2 — La valeur, et qui l'arbitre

mika#2191 n'arbitrait rien : le consommateur avait déjà fixé `upstream_status`,
entier. Ici, **personne ne l'a fixée**. mika-cloud#230 AC1 parle de « cause
distincte » et AC3 de « code machine », sans nommer de symbole ; le corps de
mika#2908 propose `"code": "BOT_USERNAME_MISMATCH"`. Ce dépôt tranche donc, et
prend la valeur du ticket telle quelle — inventer un synonyme
(`bot_username_mismatch`, `USERNAME_MISMATCH`) laisserait les deux dépôts
attendre l'autre, pour un gain nul.

`SCREAMING_SNAKE` plutôt que `snake_case` : c'est la forme du ticket, et elle
distingue à l'œil un symbole de protocole d'une clé JSON ordinaire.

### D3 — Une constante, et un test qui ne l'utilise pas

```rust
/// Code machine du refus « l'username ne correspond pas au bot ».
///
/// **Format de fil.** mika-cloud#230 AC1 classifie cette cause sur ce symbole,
/// pour renvoyer l'utilisateur corriger l'username et non le token. En changer
/// la valeur est une rupture inter-dépôt à dater, jamais un renommage — à la
/// lettre de ce que dit [`INVALID_BOT_TOKEN_MESSAGE`] de la sienne.
const BOT_USERNAME_MISMATCH_CODE: &str = "BOT_USERNAME_MISMATCH";
```

La constante existe pour **porter ce doc comment au bon endroit**, pas pour
factoriser un site unique. Et le test d'AC1 asserte le littéral
`"BOT_USERNAME_MISMATCH"` écrit en toutes lettres, **pas** la constante : la
doctrine est déjà écrite dans ce fichier
(`mika2191_ac2_le_message_du_401_est_un_format_de_fil`, `routes.rs:4292-4299`) —
asserter contre la constante ne prouverait rien, puisque qui la renommerait
déplacerait les deux côtés d'un coup et passerait le test en cassant le
consommateur.

### D4 — Ce que touche la production : deux lignes utiles

```rust
fn bot_username_mismatch_body(provided: &str, actual: &str) -> serde_json::Value {
    serde_json::json!({
        "error": format!("bot_username mismatch: provided '{provided}' but Telegram returned '{actual}'"),
        "code": BOT_USERNAME_MISMATCH_CODE
    })
}
```

Le site d'appel (`routes.rs:1213-1220`) n'est **pas touché** : c'est lui qui
porte `StatusCode::BAD_REQUEST`, et le laisser littéralement intact est ce qui
réalise la moitié « statut » d'AC3. La moitié « `error` » est réalisée par le
`format!`, inchangé, et testée en toutes lettres (V3).

### D5 — Pas de scan structurel, et pourquoi

mika#2191 avait livré `mika2191_le_litteral_na_quun_seul_site_de_production`
parce que son littéral risquait d'être réécrit dans un `format!` ailleurs dans
le crate. Ce risque n'existe pas ici : le symbole est produit par une seule
fonction, déjà extraite, dont la valeur de retour est directement assertable.
Livrer un `ProductionScanner` de plus serait un détecteur à population vide
par construction — du coût sans signal.

### Gate mika#2306 — aucun détecteur livré, la section `## Fire-Disposition` est N/A

Les trois tests de V1-V3 sont des **assertions de contrat sur la valeur de
retour de deux fonctions** : ils lisent un `serde_json::Value` produit par un
appel, pas l'arbre source. Aucun `ProductionScanner`, aucune règle de lint,
aucune garde CI, aucun validateur de schéma, aucune garde EndTurn n'est livré
(D5 dit pourquoi le scan a été écarté). Le test V2 énumère six variantes d'un
`enum` fermé — c'est une table de cas, pas une population à excepter.

Les trois options de mika#1574 seraient vides de sens ici : il n'existe aucune
violation existante à mettre en allowlist (`token_validation_error_body`
construit son corps avec des clés nommées, il ne peut pas porter `code` par
accident), rien à désarmer, et rien à remonter. La section n'est donc pas écrite
— le gate est N/A, pas oublié.

## Fichiers

| fichier | changement |
|---|---|
| `crates/mika-gateway/src/routes.rs` | la constante + son doc comment, la clé `code` dans `bot_username_mismatch_body`, le doc comment de la fonction, le module de tests `mika2908_code_du_mismatch` |
| `crates/mika-gateway/CLAUDE.md` | la ligne `POST /admin/customers` de la table *Endpoints* (R7) |
| `docs/plans/2026-10-25-001-feat-2908-gateway-code-machine-ferme-sur-le-mismatch-plan.md` | ce plan |

Aucun autre fichier. `docs/openapi/gateway.yaml` ne décrit pas
`POST /admin/customers` (vérifié : zéro occurrence), il n'y a donc rien à y
synchroniser.

## Vérification

Un nouveau module `mod mika2908_code_du_mismatch`, voisin de
`mika2191_upstream_status`, qui ne modifie aucun test existant.

- **V1 (AC1) — `mika2908_ac1_le_mismatch_porte_son_code`.** Le corps porte
  `code == "BOT_USERNAME_MISMATCH"`, littéral en toutes lettres, et la valeur ne
  dépend pas des arguments : deux appels avec des couples `(provided, actual)`
  différents rendent le même `code`. **Rougit sur `origin/main`** — la clé est
  absente aujourd'hui.
- **V2 (AC2) — `mika2908_ac2_aucun_refus_de_token_ne_porte_ce_code`.** Sur les
  six variantes déjà énumérées par
  `mika2191_la_cle_error_est_toujours_presente` (`Unauthorized`, `BotBlocked`,
  `RateLimited`, `BadRequest`, `Other{502}`, `Other{200}`),
  `token_validation_error_body(...).get("code").is_none()`. **Garde de
  non-régression : verte avant comme après**, et c'est dit plutôt que maquillé
  en rouge-avant. Elle rougirait le jour où quelqu'un déciderait que les deux
  vocabulaires doivent se recouvrir, ce que le non-objectif 1 refuse.
- **V3 (AC3) — `mika2908_ac3_le_message_du_mismatch_est_inchange`.** Le champ
  `error` vaut exactement
  `bot_username mismatch: provided 'wanted_bot' but Telegram returned 'actual_bot'`,
  asserté en toutes lettres (le classifier textuel de mika-cloud le lit encore
  jusqu'à ce que #230 AC1 bascule). Le corps ne porte toujours aucun
  `upstream_status`. Même nature que V2 : garde de non-régression, verte avant.
- **V4.** `mika2191_ac3_les_branches_non_401_ne_portent_pas_401` et
  `mika2191_la_cle_error_est_toujours_presente` restent verts **sans
  modification** — ils appellent tous deux `bot_username_mismatch_body` et
  n'assertent rien sur l'absence de clés autres qu'`upstream_status`, donc
  l'ajout ne les touche pas. Vérifié par lecture avant d'écrire (R4).
- **V5.** Le statut HTTP d'AC3 n'est pas testable sans Postgres ni injection
  d'`api_url` (non-objectif 5). Il est tenu **structurellement** :
  `git diff origin/main -- crates/mika-gateway/src/routes.rs` ne contient aucune
  ligne portant `StatusCode::` — le site d'appel est hors du diff. C'est une
  vérification de diff, déclarée comme telle, pas un test déguisé.
- **V6.** `cargo test -p mika-gateway`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --check`.
- **V7.** `scripts/verify-pipeline.sh origin/main` passe.

## Sonde post-déploiement, et sa halte

Le changement ne réveille mika-cloud#230 AC1 qu'une fois **mergé et déployé sur
le gateway de dev** — le ticket le dit, et c'est la classe de défaut mika#2340 :
un binaire déployé qui précède le correctif est indiscernable d'un correctif
absent.

```bash
curl -s -X POST "$GATEWAY/admin/customers" \
  -H "Authorization: Bearer $MIKA_INTERNAL_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"customer_id":"<uuid>","bot_token":"<token valide>","bot_username":"un_username_qui_nest_pas_le_bon"}' \
  | jq '{error, code, upstream_status}'
```

- `code: "BOT_USERNAME_MISMATCH"` + `error` inchangé + `upstream_status: null` →
  régime attendu, AC1 de mika-cloud#230 peut être réveillé.
- `code: null` → **halte : établir la version déployée avant de toucher au
  code.** `GET /version` du gateway, et comparer son `git_hash` au commit de
  fusion de ce vol. Un `code` absent sur un binaire antérieur n'est pas un
  défaut de ce changement.
- `error` modifié → le classifier textuel de mika-cloud#205 est en train de
  casser : c'est une rupture inter-dépôt, pas une amélioration de libellé.

## Definition of Done

La clé `code` est produite par `bot_username_mismatch_body`, les trois tests de
V1-V3 existent et passent, aucun test de `mika2191_upstream_status` n'a été
modifié, V5 à V7 sont verts en local, la ligne de `crates/mika-gateway/CLAUDE.md`
est à jour, et le tout est commité sur la branche sans push.

## Acceptance criteria

- [ ] **AC1** — le corps de `bot_username_mismatch_body` contient un champ
  `code` de valeur fixe. Un test sur ce corps vérifie le champ et sa valeur
  exacte.
- [ ] **AC2** — les corps de refus du token (`token_validation_error_body`) ne
  changent pas. Le test existant d'`upstream_status` reste vert, et ces corps ne
  contiennent pas ce `code`.
- [ ] **AC3** — le statut HTTP et le champ `error` du mismatch ne changent pas.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-gateway/src/routes.rs` (production : constante, doc comments, clé `code`) | 18 |
| `crates/mika-gateway/src/routes.rs` (tests : module `mika2908_code_du_mismatch`, 3 tests) | 75 |

Total estimé : 93 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Un seul vol, et il tient largement : le changement de production est l'ajout
d'une clé à un `json!` de quatre lignes plus la constante qui porte sa
documentation. Le volume est dans les tests et leurs doc comments, qui ne
demandent ni fixture, ni harnais, ni accès réseau — les corps sont des fonctions
pures déjà extraites par mika#2191, ce qui est précisément ce qui rend ce vol
petit. Aucune découpe nécessaire : la suite de la chaîne est côté console, et
elle est nommée ci-dessus.

*Tracked in: senara-solutions/mika-cloud#230 (AC1 — la lecture du code par le
classifier et l'écran qui renvoie corriger l'username).*

## Assumptions

- **A1.** mika-cloud#230 AC1 acceptera le symbole `BOT_USERNAME_MISMATCH` tel
  quel. Rien ne le fixe à date (D2), et le corps de mika#2908 le propose. Si la
  console préfère une autre forme, le correctif est d'une ligne dans la
  constante, plus la même ligne dans le test de V1.
- **A2.** Aucun consommateur actuel ne rejette un corps JSON portant une clé
  supplémentaire. Le seul lecteur connu est `classify_gateway_error`
  (mika-cloud), qui lit `error` et `upstream_status` par nom.
