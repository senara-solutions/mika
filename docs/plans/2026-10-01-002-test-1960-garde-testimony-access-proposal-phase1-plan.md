# mika#1960 — PHASE 1 : le prédicat `testimony_access_proposal`, fonction pure

> **Ticket :** senara-solutions/mika#1960 (`p3-nice-to-have`, `evaluation`, `agent-core`)
> **Parent :** mika#1798 (bake de l'invariant non-transit) — **frère :** mika#1814 (doctrine invitation-only)
> **Report d'origine :** verdict de la PR #1956, commentaire `5382232239`
> **Référence de conception :** le plan monolithique du 2026-09-30, lisible à `2e6494f2` (§ D1 et suivants). Ce plan en est le **sous-ensemble phase 1**.

## Périmètre — phase 1 seule, et pourquoi le découpage existe

Le corps du ticket (bearing Prime du 2026-10-01) borne ce ticket à **la garde comme
fonction pure** : prédicat par phrase, couches A/B/C bilingues, tests unitaires.
**Aucun câblage EndTurn, aucune télémétrie, aucun eval.**

Le découpage n'est pas cosmétique. Le plan du 30/09 livrait d'un bloc une garde de
production **et** son câblage **et** deux fichiers d'eval **et** un élargissement de
scan — le profil exact des trois morts en `max_turns` du 26/09 au 01/10. Ce que la
phase 1 doit garantir est donc **aussi** une propriété de taille : un seul fichier
touché, un livrable mergeable sans toucher une garde en production.

**Conséquence qui décide la forme du livrable : rien de ce qui est écrit ici ne peut
firer.** La fonction est pure, personne ne l'appelle, et c'est voulu — voir
§ Fire-Disposition, où cette propriété *est* la disposition.

## Les deux lectures du 30/09 qui portent la phase 1

Les quatre lectures du plan de référence ne sont pas toutes de ce périmètre : R1
(la forme d'assertion n'était jamais circulaire, il manquait un sujet à mesurer) et
R4 (l'angle mort du scan mika#2265 sur les sous-répertoires) concernent **l'eval**,
donc la phase 2. Deux restent portantes ici.

### R2 — la population est vivante, pas hypothétique

`google-workspace` est dans `FAMILY_AGENT_SKILL_ALLOWLIST`
(`crates/mika-common/src/home.rs`), donc le tier famille — celui de l'incident
fondateur du 2026-07-18, tenant cloud — atteint `run_gws`. Et **aucune des neuf
gardes EndTurn déclarées dans `evidence/guards.rs` n'est testimony** : les deux
occurrences de `testimony` sous `agent_loop/` sont des commentaires renvoyant au
Layer 4, c'est-à-dire côté *accès*. La moitié *propose* du HARD NO est donc
prompt-only, et le doc doctrine nomme cet état par écrit comme le pire disponible
(*« the only remaining defense is Layer 1 (the prompt) — the fragile layer the
doctrine explicitly distrusts »*).

### R3 — le discriminant est la DIRECTION DU MOUVEMENT D'ACCÈS, jamais le lexique

C'est la difficulté centrale, et elle est strictement plus dure que celle de 5c. Le
Layer 1 **prescrit de nommer la doctrine en refusant**, donc un refus conforme
contient nécessairement le sujet interdit et le mot accès dans la même phrase :

| texte | verdict attendu |
|---|---|
| « je pourrais t'aider si tu me donnais accès à ta boîte Gmail » | **fire** |
| « je ne peux pas accéder à tes emails — c'est de la donnée testimony-grade » | **ne fire pas** |

Un prédicat lexical sur {Gmail} × {accès} refuse le refus, c'est-à-dire casse le
comportement que mika#1798 a livré. L'inversion à faire est celle que mika#2290 a
déjà écrite pour 5d (*« Layer B, positive polarity — … carrying its own grammatical
subject »*), portée ici sur le **mouvement** : une proposition demande une
**ouverture** d'accès, un refus déclare une **fermeture**.

**Le fail-safe penche vers *ne pas firer*, et c'est une mesure de coût.** Un faux
négatif laisse passer une proposition — le comportement d'aujourd'hui, à quoi ce
travail ne peut pas être pire. Un faux positif, une fois la phase 2 câblée,
re-prompterait un refus légitime sur le tier famille et pousserait le modèle à
**cesser de nommer la doctrine**, dégradant exactement ce que mika#1798 a construit.

## Requirements

- **REQ1** — Une fonction pure de `crates/mika-agent/src/evidence/guards.rs` rend
  `Some(..)` quand une **même phrase** du texte propose d'ouvrir un accès à de la
  donnée testimony-grade, et `None` sinon.
- **REQ2** — Elle rend `None` sur **toutes** les formulations de refus que le Layer 1
  prescrit, y compris celles qui nomment le sujet interdit et la doctrine dans la
  même phrase.
- **REQ3** — Les deux contrôles négatifs que le corps du ticket nomme sont couverts
  par un test chacun : **la négation modale** (« je ne peux pas accéder à… ») et **le
  refus long sans proposition**.
- **REQ4** — Aucun site de production n'appelle la fonction ; aucune couche de
  mika#1798 n'est modifiée ; aucun test existant ne change de sens ; aucune valeur de
  réglage ne bouge.

## Deliverables

Un seul fichier de production touché : `crates/mika-agent/src/evidence/guards.rs`.

### D1 — `enclosing_sentence`, l'utilitaire de bornes (prérequis de D2)

`sentence_is_suppressed` (5d, l.981) et `frequency_sentence_is_suppressed` (5f,
l.1261) ont un corps **byte-identique** jusqu'à la liste consultée : mêmes
`TERMINATORS`, même `rfind`/`find`, même court-circuit sur `?`. Elles ne diffèrent que
par `CLAIM_CONDITIONAL_MARKERS` contre `FREQUENCY_INCAPACITY_MARKERS`.

**Ce n'est pas un lecteur dupliqué, c'est un utilitaire de segmentation dupliqué** —
les deux prédicats *sémantiques* sont distincts et doivent le rester (« registre de
possibilité » ≠ « aveu d'incapacité »). Donc extraire le **calcul des bornes** :

```rust
fn enclosing_sentence(text: &str, start: usize, end: usize) -> (&str, Option<char>)
```

**Les deux appelants existants ne sont PAS routés vers lui dans cette PR**, et c'est
l'arbitrage de périmètre : les toucher ferait entrer 5d et 5f — deux gardes de
production dont le faux positif coûte un tour cassé sur un tenant famille — dans le
rayon de souffle d'un ticket de phase 1. L'abstraction juste est **posée** maintenant,
le routage des deux anciens est un diff de deux lignes chacun, nommé en § Hors
périmètre. Écrire la troisième copie sans poser l'helper serait le choix inverse : la
dette deviendrait structurelle au lieu d'être à un geste près.

### D2 — `detect_testimony_access_proposal`, le prédicat

```rust
pub(crate) fn detect_testimony_access_proposal(text: &str)
    -> Option<TestimonyAccessProposalMatch>
```

**Aucun paramètre contextuel, et c'est une décision.** 5d prend un `Deployment`, 5f
une `language`, 5g une heure locale — parce que chacune garde un fait conditionnel.
La doctrine non-transit, elle, est **inconditionnelle** : le doc le dit en toutes
lettres (*« There is no runtime override in v1. Not a CLI flag. Not an env var. Not a
DB row. »*). Ajouter un axe conditionnel ici serait donner au futur appelant un levier
pour rouvrir la moitié *propose* du HARD NO.

**Le prédicat est par phrase, sur le patron de 5d/5f** : la garde rend `Some` quand
une même phrase porte Layer A **et** Layer B et que Layer C ne la couvre pas. Le
« même phrase » est obtenu par la classe de caractères du gap (`[^.!?\n]{0,N}`) plutôt
que par une seconde passe — précédent de `CLAIM_GAP_MAX`, que 5f réutilise déjà pour
une promesse, donc une constante déjà générique de fait. Trois propriétés en
découlent, qu'un override global sur le texte entier ne donne pas :

- « je ne peux pas accéder à tes emails, c'est testimony-grade » — une phrase, Layer B
  absent (une négation modale n'est pas un mouvement d'ouverture) ⇒ `None` ;
- « je ne peux pas accéder à Gmail. Mais si tu me donnais accès, on irait plus loin. »
  — la **seconde** phrase porte A ∧ B ⇒ `Some`, et c'est le comportement voulu : c'est
  la forme mesurée de l'incident fondateur, un refus suivi d'une proposition ;
- un refus long qui nomme le sujet en ouverture et ne propose rien ⇒ `None`, quelle
  que soit sa longueur.

**Les deux ordres A→B et B→A sont reconnus**, comme 5f et pour la même raison
grammaticale : « si tu me donnais accès à ta boîte Gmail » est B→A, « ta boîte Gmail,
tu peux m'y donner accès » est A→B. Deux alternances, ou une regex par ordre — jamais
un seul ordre, qui raterait la forme littérale de l'incident.

**Layer A — sujets testimony-grade.** Dérivés de la taxonomie du doc doctrine
(`crates/mika-agent/docs/non-transit-data-grade.md`, l. 18-22), jamais inventés :
contenu Gmail / messagerie, Drive **complet**, journaux intimes et confessionnel.
Bilingues FR + EN pour la raison que 5c et 5d écrivent déjà. Word-bounded et
**qualifiés** : `drive` seul et `mail` seul sont hors de la couche — `drive.file`
app-scoped est opérationnel *par doctrine* (le doc le nomme comme exception), et `mail`
nu attrape le courrier postal. Même règle que le `veille technique` de 5f.

**Layer B — mouvement d'ouverture, polarité positive, porteur grammatical propre.**
Trois formes, et la troisième est celle de l'incident : octroi demandé (`donne-moi
accès`, `si tu me donnais accès`, `grant me access`), autorisation proposée (`tu peux
m'autoriser`, `il faudrait me connecter`, `you can authorize me`), conditionnel d'aide
(`je pourrais t'aider si`, `I could help if`). Cette dernière est la forme **littérale**
que le doc cite comme brèche : *« A well-meaning "I could help if you gave me Gmail
access…" is a breach at the propose surface, even without a tool call. »* Les formes
modales et interrogatives sont absentes **par construction** — elles ne sont pas des
mouvements d'ouverture — plutôt que spécialement exemptées.

**Layer C — couverture doctrinale de la phrase.** Seconde ligne, pas première : une
phrase qui porte A ∧ B **et** un fragment doctrinal explicite (`testimony-grade`,
`non-transit`, `HARD NO`) est une méta-discussion, pas une proposition. Gardée
**narrow** pour la raison que 5d écrit au même endroit (*« to avoid becoming a bypass
shape »*) : le Layer 1 prescrivant de nommer la doctrine, un override large ferait
qu'il suffit de la nommer pour que la proposition passe. **C'est la segmentation par
phrase qui retire ce bypass**, pas la largeur de C.

**Chemin rapide** obligatoire, patron de 5d : un `contains` sur les atomes de Layer A
avant toute compilation de regex. La contrainte qui va avec est héritée mot pour mot
et doit être écrite au site : *ajouter une surface à la regex oblige à mettre à jour le
`contains`.*

**Sortie** : `TestimonyAccessProposalMatch { subject, movement }`, jumelle de
`FalseLocalHostingMatch` et `FrequencyPromiseMatch`.

### D3 — les tests unitaires

Dans le `mod tests` existant de `guards.rs` (l. 3209), nommés `mika1960_*` sur le
patron des 10 `mika2290_*` et 10 `mika2358_*`.

**Positifs** — la forme mesurée de l'incident (conditionnel d'aide + Gmail) ; l'octroi
direct ; l'autorisation proposée ; la forme anglaise ; le refus **suivi** d'une
proposition en seconde phrase ; les deux ordres A→B et B→A.

**Contrôles négatifs** — les deux que le ticket nomme : **la négation modale** et **le
refus long sans proposition**. Plus : le refus qui nomme la doctrine ; le refus qui
propose une aide operational-grade en substitution ; la réponse pédagogique ; la
question (`?`) ; `drive.file` app-scoped ; `mail` nu au sens postal ; une phrase
couverte par Layer C.

**Et le contrôle qui sépare « le prédicat lit une phrase » de « le prédicat lit le
texte »** : une proposition **supprimée** ne doit pas masquer une violation plus loin
dans le même texte — le site itère les matches, patron de `first_surviving_claim`
(l. 946), et le test jumeau existe déjà pour 5d (`mika2290_suppressed_match_does_not_mask_a_later_violation`).

## Fire-Disposition

Ce plan livre **un** détecteur : le prédicat de D2.

**Disposition : (b) livrer désarmé — et par CONSTRUCTION, pas par un attribut.**

La phase 1 livre une fonction pure que **personne n'appelle**. Il n'y a donc ni
`#[ignore]` à poser ni `cfg` à écrire : l'absence de site d'appel *est* le
désarmement, et elle est vérifiable par un `grep` (V4 ci-dessous) plutôt que par une
convention. C'est la forme la plus forte de l'option (b) — un détecteur muni d'un
`#[ignore]` peut être armé d'un geste distrait, un détecteur sans appelant demande une
PR qui le branche.

**Suivi pour l'armement : la phase 2**, ticket à ouvrir à la livraison de celle-ci
(précondition écrite dans le corps du ticket : phase 1 mergée). Elle porte le câblage
en position 5h, le re-prompt et son budget, le résidu `_uncorrected`, la télémétrie
`guard.*`, l'eval de régression et la doc.

**Pas d'allowlist, et il n'y aurait rien à y mettre :** la population d'un détecteur
de ce type n'est pas une donnée du dépôt mais le texte d'un tour, donc il n'existe
aucune violation préexistante à exempter. Le risque réel est le faux positif de R3, et
ce qui le couvre est la liste de contrôles négatifs de D3, **vue verte**.

**Levier de désarmement après la phase 2, nommé ici parce que c'est une décision de
conception et non d'implémentation :** le revert du terme 5h, qui sera additif et
isolé. **Pas** de variable d'environnement — aucune garde de cette famille n'en a, et
en introduire une donnerait à l'environnement du service un levier pour rouvrir la
moitié *propose* du HARD NO, ce que le § *Operator override path* du doc doctrine
refuse en toutes lettres.

## Verification Contract

| # | Vérification | Commande | Attendu |
|---|---|---|---|
| V1 | Les tests du prédicat | `cargo test -p mika-agent evidence::guards::tests::mika1960` | vert |
| V2 | **Les contrôles négatifs, vus verts** | idem, cibler les cas négatifs de D3 | vert — `None` sur les treize formes, dont négation modale et refus long |
| V3 | Non-régression des gardes voisines | `cargo test -p mika-agent evidence::guards` | vert, les `mika2290_*` / `mika2358_*` / `mika2247_*` inclus |
| V4 | **Le détecteur est bien sans appelant** | `grep -rn 'detect_testimony_access_proposal' crates/ --include=*.rs` | **uniquement** sa définition et ses tests — zéro site de production |
| V5 | Non-régression globale | `cargo test --workspace` | vert |
| V6 | Lint et format | `cargo clippy --workspace --all-targets -- -D warnings` puis `cargo fmt --all --check` | propres |

**V2 et V4 sont les vérifications porteuses.** V2 est la seule mesure du risque R3.
V4 est ce qui rend la Fire-Disposition vérifiable : sans elle, « livré désarmé » et
« câblé en passant » rendent des bytes identiques.

## Definition of Done

- [ ] D1 : `enclosing_sentence` posé, les deux appelants existants **non touchés**.
- [ ] D2 : le prédicat, ses trois couches bilingues, son chemin rapide, sa contrainte
      de synchronisation écrite au site, et sa sortie structurée.
- [ ] D3 : les tests positifs et les treize contrôles négatifs, dont la négation modale
      et le refus long nommés par le ticket.
- [ ] V1 à V6 verts, avec V2 et V4 rapportés explicitement dans le corps de PR.
- [ ] Un seul fichier de production modifié (`evidence/guards.rs`).
- [ ] Aucun câblage, aucune télémétrie, aucun eval, **aucune ligne du doc doctrine**.

## Acceptance criteria

*(Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de son § Périmètre phase 1 et du report de la PR #1956.)*

- **AC1** — `detect_testimony_access_proposal` existe dans
  `crates/mika-agent/src/evidence/guards.rs` comme **fonction pure**, sans accès base,
  sans I/O, sans lecture d'environnement. Attesté par V1.
- **AC2** — Le prédicat est **par phrase** : une proposition en seconde phrase après un
  refus est détectée, et un refus long sans proposition ne l'est pas. Attesté par les
  deux tests correspondants de D3.
- **AC3** — Les trois couches A/B/C sont **bilingues FR + EN**, et Layer A est dérivée
  de la taxonomie du doc doctrine plutôt qu'inventée. Attesté par les tests de forme
  anglaise et par les cas `drive.file` / `mail` nu.
- **AC4** — Les contrôles négatifs nommés par le ticket — **négation modale** et
  **refus long sans proposition** — ont chacun un test, et ils sont **verts** (V2).
- **AC5** — Le détecteur est livré **désarmé** : aucun site de production ne l'appelle,
  ce qui est vérifié par V4 et non par convention.
- **AC6** — Aucun eval, aucune télémétrie, aucun câblage EndTurn, aucune modification
  du doc doctrine ni d'une couche de mika#1798. La suite complète reste verte (V3, V5).

## Risks

| # | Risque | Mitigation |
|---|---|---|
| RK1 | **Faux positif sur le refus prescrit** — le risque principal ; une fois la phase 2 câblée, il dégrade ce que mika#1798 a livré | Prédicat par phrase (D2), fail-safe vers `None`, les treize contrôles négatifs vus verts (V2) |
| RK2 | **Layer C devient un bypass** — le Layer 1 prescrit de nommer la doctrine, donc la nommer suffirait | La segmentation par phrase, jamais un override global ; C gardé narrow, patron 5d |
| RK3 | Le chemin rapide sous-filtre la regex et le prédicat devient muet | Contrainte héritée de 5d, **écrite au site** ; un test dont le sujet n'est pas dans le `contains` rougirait |
| RK4 | **L'implémenteur déborde sur la phase 2** (câblage, eval, doc) — le risque de périmètre que le découpage existe pour fermer | V4 interdit le site d'appel ; la DoD interdit le doc ; le § Hors périmètre nomme chaque pièce de la phase 2 |
| RK5 | `enclosing_sentence` est posé et la troisième copie est écrite quand même | D1 est un livrable de la DoD, et D2 l'appelle — un prédicat qui recalcule ses bornes ne passe pas la relecture |

## Ce que ce travail n'achète PAS

- **Il ne protège rien.** Une fonction pure que personne n'appelle ne refuse aucun
  tour : la moitié *propose* du HARD NO reste prompt-only jusqu'à la phase 2. Dire
  l'inverse — et en particulier **l'écrire dans le doc doctrine** — serait une
  affirmation fausse dans le document qui porte la doctrine. C'est pourquoi la DoD
  l'interdit explicitement : le plan du 30/09 prévoyait cette phrase (son § D7), et
  elle n'est vraie qu'avec le câblage.
- **Il ne prouve pas qu'un modèle refuse.** Aucun test déterministe ne peut l'établir ;
  c'est la moitié comportementale, désarmée, de la phase 2.
- **Il n'ajoute aucune surface d'observabilité.** Pas de ligne de journal, pas
  d'`audit_events`, pas de compteur — il n'y a pas d'événement à émettre tant que rien
  n'appelle la fonction.
- **Il ne retire pas la dette de duplication de la segmentation.** Il la **borne** : un
  helper juste est posé, deux routages d'une ligne restent à faire, et la dette est
  nommée plutôt que silencieuse.

## Hors périmètre, délibérément — c'est la phase 2

Ticket à ouvrir à la livraison de celle-ci. Chaque pièce vient du plan de référence
(`2e6494f2`), dont les § correspondants restent valides :

- **Le câblage en position 5h** (§ D2 du plan de référence) : forme partagée de la
  famille, single-retry via `intent_guard_retries`, non sauté par
  `skip_remaining_guards`, résidu `guard.testimony_access_proposal_uncorrected`, et un
  texte de correction à **deux branches** (refuser en nommant la doctrine, **ou**
  proposer une aide operational-grade — sans la seconde, la correction pousse vers un
  refus sec que le Layer 1 interdit).
- **L'eval déterministe et son contrôle négatif** (§ D3/D4), dont la forme d'assertion
  est celle de R1 : le sujet est le moteur, jamais la disposition du modèle.
- **La moitié comportementale désarmée** (§ D5) et **l'élargissement du scan
  mika#2265 aux sous-répertoires** (§ D6, avec sa mesure « 87 fichiers, 0 orphelin »).
- **Les tags `doctrine:*` et les deux § du doc doctrine** (§ D7).
- **Le routage de `sentence_is_suppressed` et `frequency_sentence_is_suppressed` vers
  `enclosing_sentence`** — deux lignes, mais elles touchent 5d et 5f ; à faire dans une
  PR dont le rayon de souffle est assumé, avec les 20 tests existants pour mesure de
  non-régression.
- **Les Layers 1/2/3/4 de mika#1798** — inchangés ici et en phase 2.
- **Une suite `calibrate-*` pour un tenant famille ou champion** — la seule voie vers
  une mesure comportementale répétable, déjà nommée par mika#2292 (suivi n°7).
