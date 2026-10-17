# fix(agent) — le masque de branche s'applique au texte qu'un gestionnaire a enveloppé (mika#2768)

> Ticket : senara-solutions/mika#2768 — `bug`, `p1-important`, `loop-substrate`.
> Voisins : mika#2755 (vol 1, le masque — fermé par la PR mika#2756), mika#2757
> (vol 2, la contradiction `required_tools` / outils retirés — ouvert).

## 1. Constat

Le masque livré par le vol 1 de mika#2755 ne couvre pas la population réelle.
`mask_cited_branch` (`crates/mika-common/src/github_event_format.rs:223`)
commence par :

```rust
if !text.starts_with(GITHUB_EVENT_TEXT_PREFIX) {
    return Cow::Borrowed(text);
}
```

Or le texte qui atteint `match_skills` n'est presque jamais le texte brut du
gateway. Le 2026-10-17 à 08:59:18Z, le tour mika-dev ouvert par
`ci_success_handler` pour la PR mika#2765 a reçu un **pré-digest** produit par
`format_merge_ready_pre_digest`
(`crates/mika-agent/src/server/ci_success_handler.rs:975`) :

```
<ci_success_handler>
[GitHub] Check suite success on senara-solutions/mika#2765 (branch: fix/2762/rollback-reaper-groom)
CI checks all green + VERDICT: pass from @mika-qa — every gate cleared for head <sha>.
…
</ci_success_handler>
```

La première ligne est `<ci_success_handler>`, donc le `starts_with` refuse, donc
rien n'est masqué, donc le `groom` du slug `rollback-reaper-groom` satisfait les
deux ancres `\b` de mika#1878 et matche le mot-clé de dev-groom :

```
WARN agent responded without calling required tools — re-prompting
     missing_names=["run_claude_pilot_groom"]
```

Hors de portée de mika#2757 également : sur ce tour les outils de dispatch
étaient **présents**, il n'y avait donc aucune contradiction à désarmer.

**Le coût mesuré n'est pas le re-prompt.** Sous la contrainte, mika-dev a groomé
mika#2766, un ticket sans label `ready` et absent du bassin : `create_task "Groom
mika#2766"` puis `run_claude_pilot_groom mika#2766`, 48 tours, 3,40 $. Le tour de
reprise a ensuite tenté de **dispatcher l'implémentation** du même ticket non
promu ; seul l'occupation de la place `implement` l'a arrêté (lanceur différé
expiré à 09:22:11Z). Le consentement positif du label `ready` (mika#841) a été
contourné par une contrainte d'outil que personne n'avait demandée. À 09:13:21Z
(n=4) l'événement de fermeture de la PR mika#2765 a reforcé l'exigence : tâche
fantôme « Groom mika#2768 », dispatch refusé par
`webhook_dispatch_target_mismatch`, tâche laissée `in_progress` sans process
jusqu'au balayage de démarrage.

## 2. Cause racine — deux prémisses fausses, pas une

Le défaut n'est pas « un `starts_with` trop strict ». Les **deux** prémisses du
masque sont contredites par la chaîne d'enrichissement de
`server/handlers.rs:1743-1960`, qui compose le texte du tour de deux manières :

```rust
VerdictAction::Handled { pre_digest }           => { req.text = pre_digest; }
VerdictAction::Passthrough { enrichment: Some(e) } => { req.text = format!("{e}{}", req.text); }
```

— la première **remplace** le texte par un pré-digest enveloppé, la seconde
**préfixe** une prose d'enrichissement au texte brut. D'où quatre chemins par
lesquels un slug cité atteint l'évaluation des mots-clés :

| # | forme | couvert par le vol 1 ? |
|---|---|---|
| 1 | texte brut du gateway, `[GitHub] …` en ligne 1 | **oui** |
| 2 | pré-digest enveloppé, gabarit `[GitHub] ` en **ligne 2** | non — incident fondateur de mika#2768 |
| 3 | enrichissement en prose citant le slug sur une ligne qui ne commence pas par `[GitHub] ` et ne finit pas par `)` | non |
| 4 | ligne `[GitHub] ` brute **reléguée** hors de la ligne 1 par un préfixage | non — forme probable de n=4 |

Prémisse fausse n° 1 : *« le texte de l'événement commence à l'octet 0 »*.
Prémisse fausse n° 2 : *« tout ce qui est cité tient sur la première ligne, et la
citation finit la ligne »*. Le chemin 3 ne viole que la seconde ; le recensement
des producteurs le montre.

### 2.1 Recensement des producteurs qui citent un slug

Lu site par site (`grep -n "branch: " crates/mika-agent/src/server/*.rs`) :

| producteur | forme | chemin |
|---|---|---|
| `ci_success_handler::format_merge_ready_pre_digest:975` | `<ci_success_handler>` + ligne `[GitHub] ` | 2 |
| `ci_success_handler::format_error_pre_digest:999` | idem | 2 |
| `ci_success_handler::format_decision_core_hold_pre_digest:1016` | idem | 2 |
| `ci_failure_handler::format_dispatch_pre_digest:1065` | `<ci_failure_handler>` + ligne `[GitHub] ` | 2 |
| `ci_failure_handler::format_escalation_pre_digest:1126` | idem | 2 |
| `ci_failure_handler`, enrichissement en vol (`:409`) | `[ci_failure_handler] CI {} on {}#{} (branch: {}). A fix is already in-flight…` | 3 |
| `ci_failure_handler::format_surface_enrichment:784` | `[surface_for_adoption] CI {} on {}#{} (branch: {}).` | 3 |

`merge_ready_handler`, `ready_label_handler`, `verdict_handler` et
`milestone_context_handler` enveloppent du texte mais **ne citent aucun slug** :
ils sont hors de la population affectée, et aucun test ne doit prétendre le
contraire.

## 3. Décision de conception

**Retenu : un engagement par ligne, et un masquage par occurrence.**

1. **Engagement** — le texte est du texte de substrat citant un événement si
   *au moins une* de ses lignes commence par `GITHUB_EVENT_TEXT_PREFIX`,
   **ou** si sa première ligne est une balise ouvrante de gestionnaire,
   reconnue **sur sa forme** : `<` + identifiant `[a-z0-9_]+` finissant par
   `_handler` + `>`. Jamais sur une liste de noms (doctrine
   `landing-tokens-lint`, mika#1804 : la régression réaliste n'est pas de
   réécrire `<ci_success_handler>`, c'est d'ajouter un huitième gestionnaire).
2. **Masquage** — une fois engagé, **chaque** occurrence de `BRANCH_MARKER`,
   sur **chaque** ligne, est coupée du marqueur jusqu'au **prochain `)` de la
   même ligne**. Une occurrence sans `)` sur sa ligne est laissée intacte et
   n'empêche pas les autres d'être masquées (le refus 3 du vol 1 devient
   per-occurrence au lieu d'être global).
3. `Cow::Borrowed` est rendu dès qu'aucune occurrence n'a été masquée —
   la discipline d'emprunt du vol 1 est conservée.

Esquisse (le détail d'implémentation est libre, les propriétés ne le sont pas) :

```rust
pub fn mask_cited_branch(text: &str) -> Cow<'_, str> {
    if !cites_a_github_event(text) {
        return Cow::Borrowed(text);
    }
    let mut out = String::new();
    let (mut last, mut cursor, mut masked_any) = (0usize, 0usize, false);
    while let Some(rel) = text[cursor..].find(BRANCH_MARKER) {
        let at = cursor + rel;
        let after = at + BRANCH_MARKER.len();
        let line_end = text[after..].find('\n').map_or(text.len(), |n| after + n);
        match text[after..line_end].find(')') {
            Some(rel_close) => {
                let close = after + rel_close;
                out.push_str(&text[last..after]);
                (masked_any, last, cursor) = (true, close, close);
            }
            // Forme inattendue : on ne coupe pas à l'aveugle, on passe à la suivante.
            None => cursor = after,
        }
    }
    if !masked_any {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[last..]);
    Cow::Owned(out)
}
```

**Frontières de caractère (mika#764, `scripts/check-byte-slices.sh`)** : tous les
offsets sont prouvés sur frontière — `find` en rend une, `BRANCH_MARKER.len()`
ajouté à une position où le marqueur a matché en est une, et `')'` comme `'\n'`
sont ASCII. Le cas multi-octets reste épinglé par son test.

### 3.1 Options écartées, et pourquoi

- **Option A — masquer toute ligne commençant par `[GitHub] `, où qu'elle
  soit.** Ferme les chemins 2 et 4, laisse le chemin 3 grand ouvert : les deux
  enrichissements de `ci_failure_handler` citent le slug dans une prose qui ne
  porte pas le gabarit. Le ticket demande la fermeture de la **classe** (« la
  classe reste ouverte ») ; A ferme deux cas sur trois.
- **Option C — masquer avant l'enrichissement.** C'est la seconde branche
  proposée par le ticket. Elle supposerait un point unique en amont de la chaîne
  de `handlers.rs`, alors que le masque doit s'appliquer au texte **final** :
  un enrichissement ajouté *après* le point de masquage réintroduirait le défaut
  en silence, et rien ne le dirait. Le masque reste là où mika#2755 l'a mis — en
  tête de `match_skills`, le dernier point avant l'évaluation — précisément pour
  que tout futur producteur en hérite sans avoir à s'en souvenir.
- **Masquage inconditionnel (sans prédicat d'engagement).** Supprimerait le
  refus 1 du vol 1, qui est une propriété **déclarée** et épinglée
  (`mask_cited_branch_refuses_a_human_message`) : un message humain est le seul
  canal par lequel un opérateur exprime une intention en texte libre, et toute
  la population de l'incident est du texte produit par le substrat. Retirer ce
  refus est une décision séparée, avec son propre rayon d'action.

## 4. Delta de contrat — ce qui change pour un lecteur du vol 1

Trois propriétés documentées de `mask_cited_branch` changent. Elles doivent être
réécrites dans le doc-comment, pas seulement dans le code.

1. **Refus 1 devient per-ligne.** Un texte dont une ligne — pas forcément la
   première — commence par `[GitHub] ` est désormais engagé. Conséquence
   assumée : un humain qui *recopie* une ligne d'événement cite lui aussi une
   donnée, et son slug est masqué. C'est le bon sens de l'erreur.
2. **Refus 3 devient per-occurrence.** Une occurrence sans `)` sur sa ligne est
   sautée au lieu de faire refuser le texte entier.
3. **« `find`, pas `rfind` » disparaît, et avec elle la propriété « un titre qui
   imite le marqueur est mangé ».** Le test
   `mask_cited_branch_eats_a_title_that_imitates_the_marker` épingle aujourd'hui
   `… — lire (branch: )` (titre tronqué). Avec le masquage par occurrence il
   rendra `… — lire (branch: ) comme une intention (branch: )` : **tous** les
   slugs partent *et* le titre survit. La raison fail-safe de manger le titre
   était qu'on ne savait couper qu'une fois ; elle disparaît quand chaque
   occurrence est traitée. Le test est **réécrit et renommé**
   (`…_masks_every_run_and_keeps_the_title_between_them`), avec le pourquoi en
   commentaire — sans quoi un relecteur lira une régression du vol 1.

Les six autres tests du module restent verts sans modification (vérifié cas par
cas : chacun porte une occurrence unique, en fin de première ligne, suivie ou non
d'une ligne d'URL).

## 5. Livrables et ordre de commit

1. **`crates/mika-common/src/github_event_format.rs`** — `cites_a_github_event`
   (privé) et `is_handler_envelope_open_tag` (public, c'est une forme de la
   grammaire déclarée par ce module), nouveau corps de `mask_cited_branch`,
   doc-comment réécrit sur le delta du § 4.
2. **Tests du même module** — les sept existants (dont un réécrit, § 4.3), plus :
   - le message de mika#2768 **mot pour mot**, pré-digest complet de
     `format_merge_ready_pre_digest`, assertion `!masked.contains("groom")` ;
   - la forme d'enrichissement du chemin 3 (`[ci_failure_handler] … (branch: x).
     A fix is already in-flight…`), qui prouve que la coupe s'arrête au `)` et ne
     mange pas la phrase ;
   - `is_handler_envelope_open_tag` : positifs (`<ci_success_handler>`,
     `<verdict_handler>`, `<ready_label_handler>`) et contrôles négatifs
     (`<html>`, `</ci_success_handler>`, `<not-a-handler>`, `<_handler>`,
     `[GitHub] …`) ;
   - une enveloppe **sans** ligne `[GitHub] ` mais citant un slug en prose —
     c'est le seul cas où la clause « enveloppe » décide seule, et sans lui elle
     serait non testée ;
   - deux occurrences sur une même ligne, dont une sans `)` : la première est
     masquée, la seconde intacte ;
   - `Cow::Borrowed` sur un texte engagé mais sans occurrence masquable.
3. **`crates/mika-agent/src/server/ci_success_handler.rs`** — test du
   **producteur réel** traversant le masque : construire le pré-digest par
   `format_merge_ready_pre_digest`, le passer à `mask_cited_branch`, exiger
   l'absence du slug. Un littéral seul ne voit pas le jour où le gabarit du
   pré-digest change ; c'est la leçon du test cross-crate de mika#2755
   (`mika-gateway/src/github.rs:2978`).
4. **`crates/mika-agent/src/server/ci_failure_handler.rs`** — extraire
   l'enrichissement en vol de `:409` dans une fonction nommée
   (`format_inflight_enrichment`), puis un test qui fait traverser le masque aux
   **deux** producteurs du chemin 3 (`format_inflight_enrichment`,
   `format_surface_enrichment`). L'extraction est la condition de la testabilité :
   un `format!` au milieu d'un bras de `match` n'est atteignable que par un tour
   complet.
5. **`crates/mika-agent/src/skills/matcher.rs`** — test de bout en bout réutilisant
   le décor `mika2755_self_dev_scene()` : avec le pré-digest de mika#2768,
   dev-groom n'est **pas** promu `MatchReason::Keyword` ; contrôle négatif
   conservé — une vraie demande humaine « groom mika#N » matche toujours.
   Doc-comment de `match_skills` mis à jour (il décrit le contrat du vol 1).
6. **`crates/mika-gateway/src/github.rs`** — le message de la moitié
   auto-nettoyante (`:3104`) dit « relire `mask_cited_branch` (première ligne,
   parenthèse fermante) » : les deux termes deviennent faux. Reformuler ; aucune
   assertion ne change.

**Plancher** : `cargo test -p mika-common -p mika-agent -p mika-gateway`,
`cargo clippy`, `cargo fmt`, `scripts/check-byte-slices.sh`, puis
`scripts/verify-pipeline.sh origin/main`.

**Déploiement** : ce changement est en Rust, pas sous `skills/bundled/` — il
atteint les agents par le binaire, donc par `make deploy` au sens ordinaire ;
la chaîne rebuild → seed → read de mika#2340 ne s'applique pas ici.

## 6. Hors périmètre de ce vol, nommé plutôt que tu — et pourquoi

Le troisième point du ticket — *« à considérer au groom : un dispatch de groom
forcé par `required_tools` ne doit jamais viser un ticket sans `ready` »* — **ne
part pas dans ce vol**, pour trois raisons mesurées :

1. **Un gate `ready` aveugle refuserait un flux réel.** `/mika-groom-ticket`
   groome légitimement des tickets non promus, sur dispatch opérateur en texte
   libre ; c'est sa raison d'être (mika#1173).
2. **Le bon prédicat n'est pas « le ticket a-t-il `ready` » mais « ce dispatch
   a-t-il été *forcé* par `required_tools` plutôt que demandé ».** Le moteur ne
   porte aujourd'hui aucune provenance de ce genre : la construire est un
   livrable à part entière, pas une clause.
3. **La garde existante ne peut pas la porter.**
   `webhook_dispatch_target_mismatch` (`skills/executor.rs:2583`, mika#2649) est
   une garde de **lignée** — elle demande « une lignée relie-t-elle cette tâche à
   la cible de l'événement ? » et ne sait rien du consentement du ticket visé.
   C'est elle qui a refusé le dispatch de n=4, par accident de lignée, pas par
   consentement.

Et mika#2757 n'en est pas le domicile : son axe est l'outil **retiré** par le
tour, le nôtre est l'outil présent sur un ticket non promu.

Le vol ouvre donc un ticket de suivi dédié — titre :
`fix(agent): un dispatch forcé par required_tools ne vise jamais un ticket sans
ready`, corps reprenant le § 1 ci-dessus et les trois points ci-dessus — et le
corps de la PR porte `Tracked in: senara-solutions/mika#<numéro ouvert>`
(exigé par `pr-body-validation.yml` dès qu'un report est formulé).

**Ce vol réduit malgré tout la population de ce défaut à presque rien** : sans
slug cité dans le texte d'intention, `required_tools` ne s'arme plus sur un
événement de branche, donc le dispatch non consenti n'a plus de producteur connu.
Le ticket de suivi garde la classe ouverte ; il ne garde pas un incident en cours.

## 7. Definition of Done

- `mask_cited_branch` masque tout slug cité dans un pré-digest enveloppé et dans
  un enrichissement préfixé, quelle que soit la ligne qui le porte.
- La reconnaissance d'enveloppe est faite **sur la forme**, sans liste de noms.
- Le delta de contrat du § 4 est écrit dans le doc-comment de la fonction, et le
  test dont la propriété change est réécrit avec sa justification.
- `matcher.rs` et le message de garde de `mika-gateway/src/github.rs` ne décrivent
  plus l'ancien contrat.
- Suite complète verte, clippy et fmt propres, `check-byte-slices.sh` vert,
  `verify-pipeline.sh origin/main` vert.
- Le ticket de suivi du § 6 est ouvert et nommé par `Tracked in:` dans le corps
  de la PR.

## 8. Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de sa section « Ce qu'il faut » et du § 5.

1. **AC1 — le message fondateur, mot pour mot.** Un test passe le pré-digest de
   mika#2768 (`<ci_success_handler>\n[GitHub] Check suite success on
   senara-solutions/mika#2765 (branch: fix/2762/rollback-reaper-groom)\n…`) à
   `mask_cited_branch` et exige que la sortie ne contienne pas `groom`, le
   marqueur ` (branch: )` restant en place.
2. **AC2 — le producteur réel, pas un littéral.** Un test construit ce même
   pré-digest par `format_merge_ready_pre_digest` et fait la même exigence, de
   sorte qu'un changement de gabarit du pré-digest rougisse.
3. **AC3 — le chemin 3 est couvert.** `format_inflight_enrichment` et
   `format_surface_enrichment` traversent le masque sans laisser de slug, et la
   phrase qui suit le `)` survit intacte.
4. **AC4 — la forme, pas les noms.** `is_handler_envelope_open_tag` accepte
   `<ci_success_handler>`, `<ci_failure_handler>`, `<verdict_handler>`,
   `<ready_label_handler>` et refuse `<html>`, `</ci_success_handler>`,
   `<not-a-handler>`, `<_handler>`. Aucun nom de gestionnaire n'apparaît dans le
   code de production du prédicat.
5. **AC5 — bout en bout.** `match_skills` appliqué au pré-digest de mika#2768 ne
   rend pas dev-groom avec `MatchReason::Keyword` ; appliqué à « groom mika#2750 »
   (texte humain), il le rend toujours.
6. **AC6 — les refus survivants.** Un message humain citant une branche reste
   emprunté inchangé ; un texte engagé sans occurrence masquable est rendu
   `Cow::Borrowed` ; un slug multi-octets est retiré sans coupe en milieu de
   caractère.
7. **AC7 — aucune régression du vol 1.** Les six tests inchangés de
   `github_event_format.rs` et le test cross-crate
   `mika2755_le_masque_retire_le_slug_de_chaque_gabarit_qui_cite_une_branche`
   restent verts sans modification d'assertion.

## 9. Fire-Disposition

Le plan livre des détecteurs : les tests des livrables 2 à 5 signalent une
violation (« un slug cité atteint l'évaluation des mots-clés »).

**Option retenue : (a) exception nommée en allowlist — avec une liste
d'exceptions vide, et c'est une mesure, pas une facilité.**

- **Population des violations existantes : zéro.** Les sept producteurs recensés
  au § 2.1 sont tous couverts par le même changement de masque ; aucun d'eux ne
  demande d'exception, et le recensement au § 2.1 est la mesure qui l'établit.
  Les détecteurs atterrissent donc **armés**, sans entrée d'allowlist, sans
  `#[ignore]`.
- **Assertion auto-nettoyante.** Elle existe déjà et n'est pas réinventée :
  la seconde moitié de
  `mika2755_le_masque_retire_le_slug_de_chaque_gabarit_qui_cite_une_branche`
  (`mika-gateway/src/github.rs:3092`) rougit dès qu'un **huitième gabarit** du
  gateway se met à citer une branche, et nomme alors le gabarit. Son message est
  mis à jour par le livrable 6 pour pointer le nouveau contrat.
- **Pourquoi aucun scan de source côté `mika-agent`.** Un recensement des sites
  `" (branch: "` dans `server/*.rs` serait un détecteur sans classe à détecter :
  après ce vol le masque est général **par forme** — chaque ligne, chaque
  occurrence — donc un huitième producteur de pré-digest est couvert le jour où
  il est écrit, sans que personne ait à s'en souvenir. Ajouter ce scan créerait
  une population d'exceptions à entretenir pour un risque déjà fermé
  structurellement.

## 10. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-common/src/github_event_format.rs` (masque, prédicats, doc) | 95 |
| `crates/mika-common/src/github_event_format.rs` (tests) | 140 |
| `crates/mika-agent/src/skills/matcher.rs` (doc + test bout en bout) | 45 |
| `crates/mika-agent/src/server/ci_success_handler.rs` (test producteur réel) | 45 |
| `crates/mika-agent/src/server/ci_failure_handler.rs` (extraction + test) | 55 |
| `crates/mika-gateway/src/github.rs` (message de garde) | 10 |

Total estimé : 390 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 390 < 429, et 390 < 1000. Le volume
est dominé par les tests (240 lignes sur 390) dans quatre fichiers qui portent
déjà leur module `#[cfg(test)]` et leurs décors (`mika2755_self_dev_scene`,
`check_suite_text`, `mika2680_blank_event`) : aucun harnais à construire. Le code
de production tient dans **une** fonction réécrite, deux prédicats et une
extraction de trois lignes — pas de migration, pas de schéma, pas de surface
opérateur nouvelle. Le découpage en vols coûterait ici plus qu'il ne borne : le
masque et ses tests ne sont pas séparément mergeables, puisque le vol 1 est déjà
en service et qu'un masque élargi sans ses tests est exactement ce que mika#2768
reproche au vol 1.

## 11. Risques

- **Le refus 1 s'assouplit.** Un message humain recopiant une ligne
  `[GitHub] …` voit son slug masqué. Évalué au § 4.1 : c'est une citation, donc
  le bon sens de l'erreur. Aucun flux connu n'en dépend.
- **Le test 7 change de verdict.** Traité explicitement au § 4.3 et au livrable 2 ;
  le risque réel est qu'on le lise comme une régression, d'où la réécriture
  commentée plutôt qu'une suppression.
- **Une enveloppe inconnue de forme exotique** (`<CiSuccessHandler>`, balise
  majuscule) ne serait pas reconnue par `is_handler_envelope_open_tag`. Sans
  conséquence pratique : tous les pré-digests recensés portent aussi une ligne
  `[GitHub] `, et c'est la clause par ligne qui décide pour eux. La clause
  enveloppe est une défense en profondeur, pas le mécanisme principal — le test
  « enveloppe sans ligne `[GitHub] ` » du livrable 2 est ce qui l'empêche d'être
  du code mort non testé.
