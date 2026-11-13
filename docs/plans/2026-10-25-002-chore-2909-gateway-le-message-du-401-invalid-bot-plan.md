---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: mika#2909
branch: chore/2909/gateway-le-message-du-401-invalid-bot
---

# chore(gateway): le message du 401 « invalid bot_token » cesse d'être un format de fil

## Problem Frame

Le gateway porte, sur deux symboles de `POST /admin/customers`, une interdiction
de reformulation : le message du 401 et la phrase du mismatch de `bot_username`
sont documentés et testés comme des **formats de fil**, parce que
`classify_gateway_error` (mika-cloud, `api/src/services/provisioning.rs`) les
lisait comme prose. Ces deux lectures ont disparu, et la console qui les faisait
n'est plus déployée nulle part.

**Ce qui a changé côté consommateur — mesuré, pas supposé.**

1. **mika-cloud#272** (`589c9df3`, ferme mika-cloud#271) a retiré de
   `classify_gateway_error` le barreau 3, celui qui lisait
   `json.get("error").…is_some_and(|e| e.contains("invalid bot_token"))`. Le
   doc-comment de la fonction dit désormais : *« The ladder has two rungs, both
   closed fields; the `error` prose is never read (mika-cloud#230 AC3,
   mika-cloud#271) »*. Un test côté console l'épingle par la négative,
   `ac1_401_text_without_upstream_status_stays_generic`.
2. **mika-cloud#275** (`69d074a8`) a réécrit mika-cloud#230 AC1 et ajouté un
   barreau 3 d'une autre nature : `code == "bot_username_mismatch"` →
   `ProvisionError::GatewayRegistration`, c'est-à-dire la branche **transitoire**
   qui garde Retry. Et le doc-comment de la fonction, à cet endroit précis :
   *« the rung reads that code, never the prose »*.
3. **La condition de réveil du ticket est remplie.** `image.tag` de
   `helm/mika-console/values-aws-dev.yaml` vaut `main-69d074a8` sur mika-cloud
   `main` (`2dac4de`), la release, le pod et `/version` désignent ce commit, et
   `git merge-base --is-ancestor 589c9df3 69d074a8` est vrai. Vérifié par
   l'opérateur le 2026-10-25 et reporté dans le corps du ticket.
4. **`provisioning.rs` n'a pas bougé depuis `69d074a8`.** Le seul commit entre
   `69d074a8` et `main` est `2dac4dec`, qui ne touche que
   `helm/mika-console/values-aws-dev.yaml`.

**Le défaut, précisément.** Six textes du gateway décrivent une consommation qui
n'existe plus. Trois disent que mika-cloud lit la sous-chaîne `invalid bot_token`
(doc-comment de la constante, doc-comment du test AC2, message d'assertion). Un
garde structurel interdit un second site de production pour un littéral que plus
personne ne lit. Un cinquième texte dit que mika-cloud classe le mismatch *« to
send the user back to fix the username and not the token »*, alors que le barreau
livré le classe en **transitoire**, précisément pour garder Retry et ne **pas**
renvoyer l'utilisateur chercher un token qui marche. Un sixième, dans
`crates/mika-gateway/CLAUDE.md`, dit que la console *« still reads the mismatch
sentence until mika-cloud#230 AC1 switches onto `code` »* — ce basculement a eu
lieu.

Un commentaire qui décrit une contrainte morte coûte deux fois : il fait reculer
une reformulation légitime, et il entraîne à ne pas croire les commentaires qui,
eux, décrivent une contrainte vivante. Le symbole `bot_username_mismatch` **est**
encore un format de fil, et c'est exactement ce qu'il faut que le prochain
lecteur puisse distinguer de ce qui ne l'est plus.

## Requirements

- **R1.** Aucun commentaire, doc-comment ou message d'assertion du gateway ne
  présente plus la sous-chaîne `invalid bot_token` comme lue par mika-cloud.
- **R2.** La **valeur** de `INVALID_BOT_TOKEN_MESSAGE` ne change pas. Le ticket
  retire l'interdiction, il ne demande aucune reformulation.
- **R3.** `upstream_status: 401` sur la branche 401 reste épinglé. Les tests de
  `mod mika2191_upstream_status` qui portent sur le **statut** —
  `mika2191_ac1_le_401_porte_le_statut_damont`,
  `mika2191_ac3_les_branches_non_401_ne_portent_pas_401`,
  `mika2191_la_cle_error_est_toujours_presente` — ne sont pas touchés.
- **R4.** Les deux textes du mismatch (doc-comment de
  `BOT_USERNAME_MISMATCH_CODE` et la ligne `/admin/customers` de
  `crates/mika-gateway/CLAUDE.md`) décrivent la consommation réelle : le
  **symbole** reste un format de fil, la **phrase** ne l'est plus, et le barreau
  qui le lit classe en transitoire.
- **R5.** `BOT_USERNAME_MISMATCH_CODE` garde sa valeur, et
  `mika2908_ac1/ac2/ac3` restent verts **sans que leurs assertions changent**.
- **R6.** Le corps de PR cite la ligne de `classify_gateway_error` sur mika-cloud
  `main` qui prouve qu'aucun barreau ne lit plus `error`.

## Non-objectifs

- **N1. Ne pas reformuler les messages.** `R2` le dit pour le 401 ; la même
  retenue s'applique à la phrase du mismatch, que `mika2908_ac3` épingle en
  entier et que `R5` garde intacte. Le ticket lève une interdiction ; profiter de
  la levée pour réécrire dans la même PR rendrait illisible la question « cette
  PR a-t-elle changé le fil ? » — la réponse doit être un non vérifiable au
  `git diff`.
- **N2. Ne pas remplacer le garde retiré par un autre garde.** Voir D4.
- **N3. Ne rien changer côté mika-cloud.** Les deux PR consommatrices sont
  mergées et déployées ; ce vol ne fait que rattraper la documentation de
  l'émetteur.
- **N4. Ne pas toucher `handle_create_customer` ni aucun corps d'erreur.** Le
  périmètre production de ce vol est strictement **documentaire** : zéro octet
  change sur le fil.

## Approche

### D1 — Le message du 401 : la constante reste, son doc-comment change de nature

`INVALID_BOT_TOKEN_MESSAGE` ne disparaît pas. Elle nomme le message que la
branche `Unauthorized` choisit, par opposition au `format!("bot token validation
failed: {other}")` que prennent toutes les autres — c'est une distinction de
**branche**, qui reste vraie et qui mérite un nom.

Ce qui change, c'est ce que le doc-comment déclare. Aujourd'hui il dit *« This
literal is a wire format, not prose »* et *« A change here is a cross-repo break
to be dated, never a wording improvement »*. Il dira ce qui est vrai : c'était un
format de fil jusqu'à mika-cloud#272 (`589c9df3`), qui a retiré le barreau
textuel ; `classify_gateway_error` ne lit plus que des champs fermés
(`upstream_status`, puis `code`) ; reformuler ce message ne casse plus rien
hors de ce dépôt.

La date est nommée, et la raison avec, parce que c'est ce qui permet au prochain
lecteur de vérifier plutôt que de croire — exactement ce que l'ancien
doc-comment faisait bien, sur une information qui est devenue fausse.

### D2 — `mika2191_ac2` : réduit à ce qu'il protège encore, pas supprimé

Le ticket offre les deux (« retirés, **ou réduits à ce qu'ils protègent
encore** »). Ce test protège encore quelque chose : que la branche
`Unauthorized` prenne **son** message et non le message générique. Supprimé, plus
rien ne tient ce routage.

La réduction inverse exactement la doctrine inscrite dans son doc-comment.
Aujourd'hui il assère le littéral **en entier** et explique pourquoi asserter
contre la constante ne prouverait rien : *« whoever reworded it would move both
sides at once »* — raisonnement juste tant que ce sont les **octets** qui sont
l'objet. Ils ne le sont plus. Ce qui reste à tenir, c'est le **choix de branche**,
et pour ce choix-là la constante est le bon côté de l'égalité : une reformulation
doit passer, un re-routage doit rougir.

Le test devient donc :

```rust
/// AC2 — la branche 401 prend son propre message, pas le générique.
///
/// **Asserté contre la constante, et c'est le renversement de mika#2909.**
/// Jusqu'à mika-cloud#272 (`589c9df3`) ce test écrivait le littéral en entier,
/// parce que mika-cloud classait sur la sous-chaîne `invalid bot_token` et
/// qu'asserter contre la constante aurait laissé passer une reformulation qui
/// cassait la console. Ce barreau est retiré, la console ne lit plus que des
/// champs fermés, et l'objet du test n'est plus les octets mais le **routage** :
/// `Unauthorized` prend la constante, tout le reste prend le message générique.
/// Une reformulation doit maintenant passer ; un re-routage doit rougir.
#[test]
fn mika2191_ac2_la_branche_401_prend_son_propre_message() {
    let body = token_validation_error_body(&TelegramApiError::Unauthorized);
    assert_eq!(
        body["error"].as_str(),
        Some(INVALID_BOT_TOKEN_MESSAGE),
        "the 401 branch must take its own message: {body}"
    );

    // Le contrôle négatif, sans lequel la première moitié est une tautologie :
    // c'est la **variante**, jamais le statut, qui sélectionne ce message.
    let other_401 = token_validation_error_body(&TelegramApiError::Other {
        status: 401,
        body: "unauthorized".to_string(),
    });
    assert_ne!(
        other_401["error"].as_str(),
        Some(INVALID_BOT_TOKEN_MESSAGE),
        "only the Unauthorized variant selects this message, never a bare 401: {other_401}"
    );
}
```

Le renommage est volontaire : `..._est_un_format_de_fil` serait un nom faux sur
un test vert, c'est-à-dire la forme exacte du défaut que ce ticket répare. Il est
hors de la réserve de `R3` — `mika2191_ac2` ne porte pas sur le statut.

**La moitié négative n'est pas décorative.** `token_validation_error_body` rend
littéralement `INVALID_BOT_TOKEN_MESSAGE.to_string()` sur la branche
`Unauthorized` ; sans second volet, le test serait une égalité d'une constante
avec elle-même à travers une fonction pure. `Other { status: 401 }` est le cas
qui distingue « cette variante » de « ce statut », et c'est le seul contenu que
la première moitié n'a pas.

### D3 — `mika2191_le_litteral_na_quun_seul_site_de_production` : retiré

Ce garde scanne les sources de production et exige exactement un site pour
`"invalid bot_token`. Son doc-comment donne sa propre condition de péremption :
*« Shipping the structured field while leaving the string as loosely held as
before would repair the visible half only, **during the window where the other
half still decides** (mika-cloud has not removed its textual rung) »*. La fenêtre
est fermée, et la phrase entre parenthèses est fausse depuis `589c9df3`.

Après retrait, rien n'empêche un second site de production du littéral. C'est une
question de DRY sur de la prose ordinaire, plus une question de correction
inter-dépôts : un second site ne casse plus aucun consommateur. Garder un garde
structurel pour cette raison-là serait exactement l'allowlist que mika#2309
condamne — un prédicat sur la **valeur** plutôt que sur le site qui la borne, qui
survit à sa justification et finit par refuser des changements sains.

`mika_common::source_guard::ProductionScanner` reste utilisée dans ce fichier par
`mika2025_copy_has_one_producer` (`routes.rs:4628`), en chemin complet : aucun
`use` ne devient inutilisé, aucune dépendance ne devient orpheline.

### D4 — Gate mika#2306 : aucun détecteur livré, la section `## Fire-Disposition` est N/A

Le vol **retire** un détecteur (D3) et **réduit** un test existant (D2). Il n'en
livre aucun de neuf : aucune règle de lint, aucun scan structurel, aucun garde
CI, aucun validateur de schéma n'apparaît dans le diff. `mika2191_ac2` réduit
n'est pas un détecteur livré — c'est un test existant dont l'objet se resserre,
et son chemin de succès est « la branche route vers la constante », pas « aucune
violation trouvée » sur un corpus.

La tentation d'en livrer un existe et est refusée ici : un scan qui interdirait
la réapparition de la phrase « lu par mika-cloud » dans les commentaires serait
un prédicat sur de la prose, exactement la famille que mika#2309 écarte. La
garantie que ce ticket produit est un **état**, vérifié une fois par V4
ci-dessous, pas une barrière permanente.

Conformément à la règle mika#2306 (« si le plan ne livre AUCUN détecteur, la
section n'est pas requise : ne l'invente pas »), ce plan ne porte pas de section
`## Fire-Disposition`.

### D5 — Le mismatch : le symbole reste un fil, sa raison change

C'est la moitié la plus facile à rater, parce que le texte actuel dit à la fois
une chose vraie et une chose fausse dans la même phrase.

**Vrai, et qui le reste :** `BOT_USERNAME_MISMATCH_CODE` est un format de fil.
Le barreau 3 de `classify_gateway_error` lit `code == "bot_username_mismatch"`
par égalité exacte ; changer la valeur casse la console. La casse minuscule reste
celle du consommateur — le barreau `code == "invalid_bot_token"` est toujours là
sur `main`. Ce paragraphe du doc-comment ne bouge pas.

**Faux :** *« mika-cloud#230 AC1 classifies this cause on this symbol, **to send
the user back to fix the username and not the token** »*. mika-cloud#275 a
réécrit AC1 à l'exact opposé. Le barreau rend `GatewayRegistration`, la branche
transitoire qui **garde Retry**, parce que le token est bon — Telegram a répondu
200 à `getMe` — et que `retry_provisioning` relit l'username depuis le token à
chaque relance. Appeler cela une faute d'identifiants retirerait ce Retry et
enverrait l'utilisateur chercher chez BotFather un token qui fonctionne.

**Faux aussi, et moins visible :** le renvoi `/// [INVALID_BOT_TOKEN_MESSAGE]
says of its own` (`routes.rs:1152`). Il pointe vers une doctrine que D1 supprime.
Laissé tel quel, il transformerait un commentaire corrigé en pointeur vers rien.
Il est remplacé par la relation désormais vraie, qui est une asymétrie : depuis
mika#2909, le `code` du mismatch est le **seul** format de fil que le corps
d'erreur de cet endpoint porte encore.

### D6 — `mika2908_ac3` : la prose se corrige, les assertions ne bougent pas

`R5` demande que `mika2908_ac1/ac2/ac3` restent verts sans modification, et `R4`
demande que la phrase du mismatch ne soit plus présentée comme lue par
mika-cloud. Deux textes de `mika2908_ac3` la présentent ainsi : son doc-comment
(*« The message stays a wire format until mika-cloud#230 AC1 switches its
classifier onto `code` »*) et son message d'échec (*« mika-cloud's textual
classifier still reads this message »*).

La lecture retenue, et c'est une décision à rendre explicite : **« sans
modification » porte sur ce que le test assère**, pas sur sa prose. Les trois
`assert` de `mika2908_ac1`, `ac2` et `ac3` — valeurs, clés, absences — sont
identiques octet pour octet après ce vol ; seuls le doc-comment et la chaîne
d'échec de `ac3` changent, et uniquement parce que `R4` l'exige. Un message
d'échec ne s'imprime que sur un rouge, donc le changer ne peut rien faire passer
de vert à rouge ni l'inverse.

Les deux autres mentions de mika-cloud#230 AC1 dans ce module (`routes.rs:4466`
et `4484`, *« classifies on this exact string / symbol »*) restent **vraies** :
AC1 réécrit classe bien sur ce symbole exact. Elles ne sont pas touchées. C'est
la différence entre « quel champ est lu » (inchangé) et « vers quel écran il
mène » (inversé), et seule la seconde était écrite ici.

### D7 — `crates/mika-gateway/CLAUDE.md` : la dernière phrase, pas la ligne

La ligne `/admin/customers` (ligne 22) est longue et presque entièrement exacte :
la description d'`upstream_status`, de sa règle de présence, de la disjonction
des deux vocabulaires et du 400 constant reste valable mot pour mot. Seule la
phrase finale est fausse :

> The `error` message of the 401 branch **and of the mismatch** are both **wire
> formats**: mika-cloud's `classify_gateway_error` still matches the substring
> `invalid bot_token` (mika-cloud#205) and still reads the mismatch sentence
> until mika-cloud#230 AC1 switches onto `code`, so rewording either is a
> cross-repo break to be dated.

Elle est remplacée par la situation réelle : la console classe sur des champs
fermés uniquement (`upstream_status` depuis mika-cloud#272, puis `code` depuis
mika-cloud#275) ; les deux messages `error` sont de la prose ordinaire et
reformulables ; la **valeur** `bot_username_mismatch` reste, elle, un format de
fil. Le remplacement est de taille comparable, ce qui garde
`crates/mika-common/tests/claude_md_size.rs` hors de cause.

### D8 — Le renvoi croisé de `mika2908_ac1`, que le renommage de D2 rendrait pendant

`mika2908_ac1` ouvre son doc-comment par *« Same doctrine as
`mika2191_ac2_le_message_du_401_est_un_format_de_fil` »* (`routes.rs:4463`). Le
renommage de D2 ferait de ce nom un identifiant qui n'existe plus — dans un
doc-comment, donc sans rougir nulle part. C'est la forme même du défaut que ce
ticket répare, et il serait malvenu de l'introduire en le réparant.

La cible est mise à jour vers `mika2191_ac2_la_branche_401_prend_son_propre_message`,
et la phrase qui suit est ajustée d'un mot : la doctrine que `mika2908_ac1`
revendique — asserter le littéral en entier plutôt que la constante — n'est plus
celle que `mika2191_ac2` applique, elle est celle qu'il **appliquait**. Pour
`mika2908_ac1` elle reste en vigueur, parce que son symbole est, lui, toujours lu
par mika-cloud ; c'est précisément le contraste que le renvoi doit maintenant
porter.

## Fichiers

| fichier | nature |
|---|---|
| `crates/mika-gateway/src/routes.rs` (≈1107-1122) | doc-comment de `INVALID_BOT_TOKEN_MESSAGE` réécrit (D1) ; valeur inchangée |
| `crates/mika-gateway/src/routes.rs` (≈1147-1164) | doc-comment de `BOT_USERNAME_MISMATCH_CODE` : §1 réécrit, renvoi ligne 1152 corrigé (D5) ; valeur inchangée |
| `crates/mika-gateway/src/routes.rs` (≈4316-4332) | `mika2191_ac2` renommé et réduit, + contrôle négatif (D2) |
| `crates/mika-gateway/src/routes.rs` (≈4414-4446) | `mika2191_le_litteral_na_quun_seul_site_de_production` retiré (D3) |
| `crates/mika-gateway/src/routes.rs` (≈4462-4468) | renvoi croisé de `mika2908_ac1` recalé sur le nouveau nom (D8) ; assertions inchangées |
| `crates/mika-gateway/src/routes.rs` (≈4534-4551) | doc-comment et message d'échec de `mika2908_ac3` (D6) ; assertions inchangées |
| `crates/mika-gateway/CLAUDE.md` (ligne 22) | phrase finale de `/admin/customers` réécrite (D7) |

Aucun autre fichier. Recensement fait sur l'arbre : `invalid bot_token`
n'apparaît que dans `routes.rs` et `crates/mika-gateway/CLAUDE.md` (hors
`docs/plans/`), et ni `docs/openapi/gateway.yaml` ni aucun autre document ne
mentionne `invalid bot_token` ou `bot_username_mismatch`. Rien sous `docs/` n'est
touché : le job `docs-sync` est hors de cause.

## Vérification

- **V1** — `cargo test -p mika-gateway` passe.
  `mika2191_ac2_la_branche_401_prend_son_propre_message` est vert, et ses deux
  moitiés : contre la constante, et le contrôle négatif sur `Other { status:
  401 }`.
- **V2** — `mika2191_ac1_le_401_porte_le_statut_damont`,
  `mika2191_ac3_les_branches_non_401_ne_portent_pas_401` et
  `mika2191_la_cle_error_est_toujours_presente` sont verts et **absents du
  diff** : `git diff origin/main -- crates/mika-gateway/src/routes.rs` ne montre
  aucune ligne de ces trois fonctions (R3).
- **V3** — `mika2908_ac1_le_mismatch_porte_son_code`,
  `mika2908_ac2_aucun_refus_de_token_ne_porte_ce_code` et
  `mika2908_ac3_le_message_du_mismatch_est_inchange` sont verts. Dans le diff,
  aucune ligne commençant par `assert` n'apparaît pour ce module (R5, D6).
- **V4 — l'état que le ticket produit, vérifié une fois.**
  `grep -rn "invalid bot_token" crates/mika-gateway/` ne rend plus **qu'une
  seule** ligne : la définition de la constante, ligne ≈1122. Zéro occurrence
  dans un commentaire, un doc-comment ou un message d'assertion (AC1). Et
  `grep -rn "mika-cloud" crates/mika-gateway/src/routes.rs` ne rend plus aucune
  ligne décrivant une lecture de prose.
- **V4bis** — aucun nom pendant :
  `grep -rn "mika2191_ac2_le_message_du_401_est_un_format_de_fil" crates/` ne
  rend plus rien, et `grep -rn "mika2191_le_litteral_na_quun_seul_site" crates/`
  non plus (D3, D8).
- **V5** — `cargo clippy -p mika-gateway --all-targets -- -D warnings` propre
  (notamment : aucun import devenu inutilisé par le retrait de D3).
- **V6** — `cargo fmt --check` propre.
- **V7** — `cargo test -p mika-common claude_md_size` vert (D7).
- **V8** — `scripts/verify-pipeline.sh origin/main` vert.

## Preuve pour AC3 — comment citer mika-cloud sans brûler des tours

AC3 demande que le corps de PR cite la ligne de `classify_gateway_error` sur
mika-cloud `main` prouvant qu'aucun barreau ne lit plus `error`. mika-cloud n'est
**pas** dans ce bac à sable, et plusieurs formes d'accès sont refusées par la
permission-policy. Ce qui marche, mesuré pendant ce grooming :

```bash
# Le patch qui retire le barreau textuel (mika-cloud#272) :
gh api repos/senara-solutions/mika-cloud/commits/589c9df3 --jq '.files[0].patch'

# Le patch qui ajoute le barreau du mismatch (mika-cloud#275) :
gh api repos/senara-solutions/mika-cloud/commits/69d074a8ee7155a0e7e3b4847bd17250d516b6ef \
  --jq '.files[] | select(.filename=="api/src/services/provisioning.rs") | .patch'

# La preuve que `main` n'a pas bougé depuis :
gh api repos/senara-solutions/mika-cloud/compare/69d074a8ee7155a0e7e3b4847bd17250d516b6ef...main \
  --jq '{commits:[.commits[]|.sha[0:8]], files:[.files[].filename]}'
```

**Formes refusées, à ne pas réessayer :** `gh search code` (policy),
`gh api -H "Accept: application/vnd.github.raw" …` (policy), et un pipe de
`--jq '.content'` vers `base64 -d` (veto de chaîne). Le chemin ci-dessus est le
seul établi.

**Ancrage mesuré, à re-vérifier avant d'écrire le corps de PR.** Sur mika-cloud
`main` = `2dac4dec` (`provisioning.rs` inchangé depuis `69d074a8`),
`api/src/services/provisioning.rs` :

- **ligne 579** — `fn classify_gateway_error(err: &gateway::GatewayError) -> ProvisionError {`
- **lignes 568-578** — le doc-comment : *« the rung reads that code, never the prose »*
- **ligne 560** — barreau 2 documenté : `code == "invalid_bot_token"`
- **lignes 599-602** — barreau 3 : `if json.get("code")… == Some("bot_username_mismatch")`
  → `GatewayRegistration`
- **lignes 595-605** — le corps après les barreaux : **aucun `json.get("error")`**

Ces numéros sont dérivés des en-têtes de hunk des deux patches, pas d'une lecture
directe du fichier : si l'implémenteur peut relire le fichier entier, qu'il
confirme ; sinon le patch de `589c9df3` est à lui seul la citation demandée, car
il **montre** la suppression du `json.get("error")`.

## Definition of Done

Les six textes sont à jour, le renvoi croisé de D8 est recalé, le garde de D3 est
retiré, `mika2191_ac2` est renommé et réduit avec son contrôle négatif, V1 à V8
sont verts en local, aucune
assertion de `mika2191_ac1/ac3`, `mika2191_la_cle_error_est_toujours_presente` ni
de `mika2908_ac1/ac2/ac3` n'a changé, aucune valeur de constante n'a changé, et
le tout est commité sur la branche sans push.

## Acceptance criteria

- [ ] **AC1** — aucun commentaire ni test du gateway ne présente plus
  `invalid bot_token` comme lu par mika-cloud.
- [ ] **AC2** — `upstream_status: 401` sur la branche 401 reste épinglé. Les
  tests `mika2191_upstream_status` qui portent sur le statut ne changent pas.
- [ ] **AC3** — la PR cite la ligne de `classify_gateway_error` sur mika-cloud
  `main` qui prouve qu'aucun barreau ne lit plus `error`.
- [ ] **AC4** — ces deux textes décrivent la consommation réelle. Le symbole
  `bot_username_mismatch` reste un format de fil, parce que mika-cloud le lit,
  mais ils ne disent plus que la console renvoie corriger le username. La phrase
  du mismatch n'est plus présentée comme lue par mika-cloud. La valeur du code ne
  change pas, et `mika2908_ac1/ac2/ac3` restent verts sans modification.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-gateway/src/routes.rs` (doc-comments de production : D1, D5) | 32 |
| `crates/mika-gateway/src/routes.rs` (tests : `mika2191_ac2` réduit D2, retrait du garde D3, prose de `mika2908_ac3` D6, renvoi croisé D8) | 60 |

Total estimé : 92 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Un seul vol, et il tient de très loin. Le diff net est **négatif** : D3 retire un
test de 33 lignes et D2 en remplace 17 par environ 20 ; le reste est de la prose
de doc-comment, et `crates/mika-gateway/CLAUDE.md` ne compte pas. La production
ne change **aucun octet exécutable** — N4 borne le vol à du documentaire plus
deux gestes de test, donc ni fixture, ni harnais, ni migration, ni accès réseau à
l'exécution. Le seul coût hors dépôt est la citation d'AC3, et le chemin `gh api`
qui la produit est déjà établi ci-dessus avec ses trois commandes exactes : zéro
tour d'exploration à refaire. Aucune découpe n'est nécessaire, et il n'y a pas de
suite à tracker — les deux PR consommatrices de mika-cloud sont mergées et
déployées, ce vol est le dernier maillon de la chaîne.

## Assumptions

- **A1.** `provisioning.rs` reste inchangé sur mika-cloud `main` entre ce
  grooming et l'ouverture de la PR. Si un commit le touche, la citation d'AC3
  doit être re-mesurée — V-AC3 ci-dessus donne la commande de comparaison, et
  c'est pour cela qu'elle est dans le plan plutôt que dans un numéro de ligne nu.
- **A2.** La console déployée sur dev ne régresse pas vers un tag antérieur à
  `589c9df3` pendant le vol. Un rollback rendrait l'épingle à nouveau utile ;
  dans ce cas l'implémentation s'arrête et remonte, plutôt que de livrer une
  documentation vraie sur `main` et fausse sur le déployé.
- **A3.** `mika2191_ac2` n'est cité par son nom nulle part en dehors de
  `routes.rs` — ni script CI, ni `scripts/suite-wiring.tsv`, ni document.
  Recensé sur l'arbre ; la seule occurrence hors de sa propre définition est le
  renvoi croisé traité par D8.
