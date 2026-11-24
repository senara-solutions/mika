# mika#3127 — une citation courte du brief se dit courte, et un libellé en gras suivi d'une annotation reste un libellé

> Ticket : senara-solutions/mika#3127. Sources de mesure : commentaires 62, 6 (cas 1) et 18 du
> parking mika#2983. Les trois sont des mesures en lecture seule sur `messages`, `llm_calls` et
> `server.log` — aucune n'est une hypothèse.

## Le défaut, tel qu'il a été mesuré

Deux défauts indépendants de `crates/mika-agent/src/agent_loop/review_anchor.rs`, qui atterrissent
tous les deux au même endroit : la garde d'ancrage (mika#2037) refuse une attestation **en nommant
la mauvaise cause**, et la relance corrective envoie donc le mauvais correctif. Les deux coûtent
des vols entiers : le cas du commentaire 18 a fini en `Verdict: ESCALATE` terminal, 50 tours,
5,28 $, et une main humaine pour retirer puis reposer `ready`.

### Défaut A — « plus court que 40 » est rendu comme « absent du brief »

`find_brief_quote_range` (l. 712-751) ne rend `QuoteSearch::TooShort` que si la **ligne entière**
fait moins de `min_quote_chars` caractères normalisés (l. 722). Au-delà, la fenêtre glissante de 40
caractères ne trouve rien et la fonction rend `QuoteSearch::NoQuote`, que `verify_review_anchors`
traduit en `AnchorMissReason::QuoteNotInBrief` (l. 880-883).

Conséquence : une ancre dont la citation **est** dans le brief, mot pour mot, mais sur moins de 40
caractères, est annoncée au modèle comme *« ne cite pas le brief verbatim — la paraphrase ne
satisfait pas le contrat »* (`describe`, l. 86-89), et le registre de lecture de mika#3003 part
chercher de quel outil elle vient, puisque sa porte (l. 361) s'ouvre précisément sur
`QuoteNotInBrief`. Dans le meilleur des cas le modèle reçoit la phrase conditionnelle *« une
citation absente du brief vient d'habitude d'un document que vous avez lu avec un outil »* —
l'exact contraire de ce qui s'est passé.

Deux mesures, deux formes du même défaut :

- **Commentaire 62** (épreuve provoquée mika#2996, 2026-11-22, trace `4c245c23`, step 1). L'ancre
  A3, « Une ligne ajoutée, aucun code » — 29 caractères — est refusée en `QuoteNotInBrief` alors
  que la phrase figure mot pour mot dans le brief. Le commentaire classe lui-même le cas :
  « la garde peut exiger une longueur ou une forme de ligne qui n'a pas été lue. Si le cas est
  confirmé, c'est un excès de refus, pas un défaut de morsure. » Il est confirmé : c'est la ligne
  722 qui n'a pas été lue.
- **Commentaire 6, cas 1** (mika#2958 vol 2e, 2026-10-31, trace `a0e82156`, session `62162fa2`).
  L'ancre A3, « Total estimé : 358 lignes » — 25 caractères — est **aussi** dans le brief. La garde
  l'a d'abord refusée en `QuoteNotInBrief`, puis en `QuoteTooShort` après la relance. Le même
  défaut rend donc **deux noms différents selon que le modèle a ou non laissé sa propre prose sur
  la ligne**, et aucun des deux ne dit « votre citation est juste, elle est seulement trop
  courte ». L'architecte n'a pas allongé sa citation.

Le refus lui-même est juste, et le commentaire 6 le dit en toutes lettres : **« la garde a eu
raison »**. 40 caractères est le contrat. Ce vol ne touche donc à aucun seuil et n'accepte rien de
nouveau : il corrige le **nom** de la cause, et donc le correctif demandé.

### Défaut B — un libellé en gras dont le deux-points est hors du gras n'est plus un libellé

`verify_review_anchors` cherche le préfixe sur `line.trim_start().trim_start_matches('*')`
(l. 806) : seuls les `*` **de tête** sont retirés. mika#2786 avait ouvert cette tolérance pour
`**A1:** "…"`, où le deux-points est *dans* le gras — après retrait des `*` de tête il reste
`A1:** "…"` et `strip_prefix("A1:")` réussit. Elle ne couvre pas les deux formes mesurées :

| forme écrite par l'architecte | après l. 806 | `strip_prefix("A1:")` |
|---|---|---|
| `**A1:** "…"` (mika#2786) | `A1:** "…"` | ✅ |
| `**A1** : "…"` | `A1** : "…"` | ❌ |
| `**A1** (déjà acceptée, conservée) : "…"` | `A1** (déjà acceptée, conservée) : "…"` | ❌ |

**Commentaire 18** (mika#3022, 2026-11-03, trace `f5eae27d`, session `e59ab2dc`). Premier passage :
trois ancres `A1:`/`A2:`/`A3:`, A3 refusée en `QuoteNotInBrief` (elle citait le corps du ticket, pas
le plan — faute de l'architecte, pas du moteur). **Relance corrective** : le nouvel A3 est bien dans
le plan (l. 25-26 du plan commité), mais les trois ancres sont écrites
`**A1** (déjà acceptée, conservée) : "…"`. La garde rend `anchors_found: 0`, donc `NoAnchorLine`,
donc `guard.review_anchor_withheld` avec `emitted: escalate`. Une citation **juste** compte comme
absente, et le groom meurt en ESCALATE terminal.

Le commentaire 18 pose la question ouverte en deux branches : *« faut-il que la garde lise aussi
`**A1**`, ou que le texte de la relance cite la forme exacte attendue ? »* Le titre du ticket
tranche la première : la garde doit lire la forme. L'ajout de samidarko au même commentaire dit
pourquoi c'est devenu un ticket : deux grooms sur les cinq du 03/11 ont demandé une main humaine
pour cette famille de cause.

## La décision de conception

Trois gestes, tous dans `review_anchor.rs`, et **aucun n'assouplit un seuil**.

### D1 — une sonde de présence, qui ne peut rien accepter

La boucle de fenêtre glissante de `find_brief_quote_range` est extraite dans un
`scan_windows(chars, brief, width, claimed) -> QuoteSearch` sans contrôle de longueur et **sans
récursion** (c'est la propriété qui compte : faire rappeler `find_brief_quote_range` avec une
largeur plus petite reboucle à l'infini, puisque l'échec de la sonde relancerait la sonde).
`find_brief_quote_range` l'appelle alors **deux fois** :

1. à `min_quote_chars`, avec les régions déjà réclamées — c'est le comportement actuel, inchangé ;
2. si et seulement si (1) rend `NoQuote`, à `QUOTE_PRESENCE_PROBE_CHARS`, avec une liste de régions
   **vide**. Cette seconde passe ne décide d'aucune validité : elle répond à la seule question
   « cette ligne cite-t-elle le brief, ne serait-ce que sur un fragment ? ». Une région déjà
   réclamée par une ancre antérieure prouve la présence autant qu'une région libre, donc la sonde
   les ignore — et le bras `OnlyClaimedRegions` est évalué **avant** elle, donc la géométrie garde
   la priorité sur la longueur.

La sonde court aussi sur le bras court (l. 722) : une ligne plus courte que `min_quote_chars` mais
au moins aussi longue que la sonde est cherchée dans le brief. C'est exactement le cas du
commentaire 6 — `"Total estimé : 358 lignes"` — et c'est ce qui fait que les deux formes du défaut A
rendent désormais **le même nom**.

**La propriété de sûreté, qui est la raison pour laquelle ce vol est petit : la sonde ne peut
faire accepter aucune ancre.** Elle ne rend jamais `Found`, elle ne pousse rien dans `claimed`,
elle n'incrémente pas `anchors_valid`. Sur toute entrée, `Satisfied` reste `Satisfied` et `Missing`
reste `Missing` ; seul le `reason` change. Le coût d'un faux positif de la sonde est donc borné à
*un message moins juste qu'il pourrait l'être*, jamais à une attestation forgée — et c'est ce qui
autorise une largeur de sonde bien plus basse que celle d'une décision.

`QUOTE_PRESENCE_PROBE_CHARS = 16`, constante nommée et documentée. Le plafond vient des mesures :
les deux citations à retrouver font 25 et 29 caractères normalisés, donc la sonde doit passer sous
25. Le plancher vient du bruit : 16 caractères consécutifs d'un brief précis ne sont pas une
coïncidence de prose française courante, et une ligne plus courte que 16 n'est pas sondable du tout.
Elle n'est **pas** `ATTRIBUTION_MIN_QUOTE_CHARS` (40, l. 255) : ce plancher-là garde une phrase à
l'**indicatif** (« cette ligne vient de cet outil »), donc il doit valoir une preuve ; la sonde ne
garde qu'un choix de message, donc elle n'en doit pas autant. Les deux doc-comments se citent
mutuellement pour qu'un futur éditeur ne les aligne pas par symétrie.

### D2 — une sixième cause, parce que deux populations sous un nom ne se soustraient pas

`AnchorMissReason` gagne `QuoteSpanTooShort` : *la ligne cite bien le brief, mais sur moins de
`min_quote_chars` caractères consécutifs*. Elle ne remplace pas `QuoteTooShort`, qui garde sa
population propre : une ligne qui ne porte **rien** de vérifiable (plus courte que la sonde, ou
courte **et** absente du brief). Après ce vol, l'axe de la citation se lit en trois noms disjoints,
chacun avec son correctif :

| cause | ce qui est vrai | ce que le modèle doit faire |
|---|---|---|
| `QuoteSpanTooShort` | la citation est dans le brief, sur < 40 car. | **allonger** la citation, ne pas la remplacer |
| `QuoteTooShort` | la ligne ne porte rien de vérifiable | écrire une vraie citation |
| `QuoteNotInBrief` | assez longue, mais absente du brief | citer **ce** brief, pas un autre document |

Fusionner les deux premières aurait été moins de code. C'est refusé pour la raison de mika#2205 :
`guard.review_anchor_withheld` porte `miss_reason`, et « la citation était juste mais courte » et
« la ligne ne disait rien » sous un seul nom ne sont plus soustractibles dans les journaux — or
c'est exactement le compte qui dira si ce vol a servi. Le `describe()` de `QuoteSpanTooShort` dit
*« la citation est présente dans le brief, allongez-la ; ne la remplacez pas »*, et c'est la phrase
que le commentaire 6 n'a jamais obtenue.

Trois conséquences, nommées parce qu'elles sont les seules :

- **La porte du registre de lecture (l. 361) n'est pas touchée** et c'est elle qui ferme la moitié
  la plus trompeuse du défaut A : elle ne s'ouvre que sur `QuoteNotInBrief`, donc une ancre reclassée
  en `QuoteSpanTooShort` ne reçoit plus ni l'attribution à un outil, ni la phrase conditionnelle
  « cette citation vient probablement d'un document lu avec un outil ». Zéro ligne à y écrire ; son
  doc-comment (l. 339-343) nomme déjà `QuoteTooShort` dans le même rôle et gagne la nouvelle cause.
- **La priorité d'agrégation** (l. 899-907) insère la nouvelle cause entre `not_in_brief` et
  `too_short`, l'ordre relatif des quatre causes existantes restant intact : sur toute entrée qui ne
  produit aucun `QuoteSpanTooShort`, `verify_review_anchors` rend le même `reason` qu'avant.
- **`verdicts_et_dispositions.rs:188-194`** itère sur les cinq variantes pour prouver que la ligne
  de finding ne porte jamais de disposition en clair ; la sixième y est ajoutée, sinon la couverture
  de ce test rétrécit en silence au moment où l'enum grandit.

### D3 — le libellé se trouve par consommation tolérante, sans regex

Un `match_anchor_prefix(line, prefix) -> Option<&str>` privé au module remplace
`trim_start_matches('*') + strip_prefix`. Il consomme les caractères du préfixe déclaré un par un et,
**entre deux d'entre eux**, saute :

- le bruit de balisage et d'espacement — `*`, `` ` ``, `_`, espaces et tabulations ;
- **au plus un** groupe équilibré `(…)` ou `[…]`, d'au plus `LABEL_ANNOTATION_MAX_CHARS` (64)
  caractères, et **jamais** avant qu'au moins un caractère du préfixe ait été apparié.

Pas de regex : le module porte le précédent anti-regex explicite de mika#864 (l. 30-33), et une
garde qui cesse de tirer en silence est la classe de défaut qu'il ferme, pas un coût acceptable.

Les quatre bornes sont ce qui empêche la tolérance de devenir une recherche de motif dans la prose :

- **Ancré au début de la ligne.** Chaque caractère du préfixe doit s'apparier ; seul le bruit se
  saute. `**Attention** : 1 : …` échoue sur `t` après `A`. Une ligne de prose n'est pas candidate.
- **Pas de groupe en position 0.** `(A1) : …` reste `NoAnchorLine`, comme aujourd'hui. Autoriser le
  saut avant le premier caractère apparié laisserait le matcheur chasser `A`, `1` et `:` à travers
  une phrase entière.
- **Un seul groupe, borné à 64 caractères.** Une parenthèse plus longue est de la prose, pas une
  annotation de libellé.
- **Le groupe est consommé avec le libellé, donc il ne compte pas dans la citation.** C'est
  l'invariant de mika#2786 (l. 788-802) étendu, pas contourné : le préfixe identifie la ligne et
  n'entre jamais dans la citation, « sinon un préfixe plus long achèterait une citation plus
  courte ». Une annotation suit la même règle. Le corollaire est une propriété de sûreté : une
  citation de 40 caractères glissée **dans** la parenthèse est consommée comme libellé, donc l'ancre
  retombe sur un reste vide et est refusée. La tolérance n'ouvre aucun contournement.

L'ordre d'essai des préfixes est celui de la déclaration (`A1:` … `A10:`), et il reste correct :
contre `**A10** : x`, le préfixe `A1:` apparie `A` puis `1`, attend `:` et voit `0`, qui n'est pas du
bruit — échec, puis `A10:` apparie. Le `label` rendu dans `AnchorOutcome` reste le préfixe **déclaré**,
donc la prose de la relance et les champs `accepted_labels`/`refused_labels` ne changent pas de forme.

## Hors périmètre, nommé

- **Aucun seuil ne bouge.** `review_anchor_min_count = 3`, `review_anchor_min_quote_chars = 40`,
  `review_anchor_min_brief_chars = 2000` restent identiques dans les trois `skill.toml`
  (`mika-arch-groom-ticket`, `mika-arch-second-review`, `mika-arch-groom-milestone`). Un AC le pin.
- **Le texte des prompts de skill n'est pas touché.** La seconde branche de la question du
  commentaire 18 — faire citer par la relance la forme exacte attendue — n'est pas tranchée ici :
  le titre du ticket demande que la garde lise la forme, et un texte de `skills/bundled/` n'atteint
  un agent qu'au travers d'un `make deploy` (mika#2340). Si la mesure d'après-vol montre encore des
  formes non lues, c'est un ticket de suivi, pas un ajout silencieux à celui-ci.
- **Le `describe()` de `NoAnchorLine` n'énumère pas les formes acceptées.** Après D3 cette cause
  devient rare ; lui faire réciter une grammaire serait un contrat de forme écrit à l'endroit où le
  moteur dit un constat.
- **Aucune garde structurelle de lecteur unique.** `match_anchor_prefix` et `scan_windows` sont
  privés au module : un second lecteur vivrait dans le même fichier, sous les yeux du relecteur. La
  réponse est structurelle (la visibilité), pas un scan de source.
- **`normalize_for_anchor_match` est intacte.** Les `*` ne sont pas retirés plus largement en amont :
  la normalisation est symétrique brief/ancre (mika#2338) et D3 n'a besoin de tolérance que pour
  *trouver* la ligne.
- **La famille « l'ancre cite le corps du ticket au lieu du plan »** (commentaire 18, premier
  passage ; commentaire 6, cas 2) n'est pas de ce vol : la garde a raison, et mika#3003 a déjà livré
  l'attribution qui le dit. Ce vol s'assure seulement qu'une citation **courte mais juste** ne soit
  plus rangée dans cette famille.

## Ordre de commit

Un seul vol, une seule PR, trois commits. Il tient sous la borne (§ *Taille estimée*) parce que les
trois gestes vivent dans un seul fichier de production, qu'aucun n'ajoute d'état ni de migration, et
qu'aucun ne traverse une frontière de crate.

1. **`fix(review-anchor): une citation courte du brief se dit courte (mika#3127)`** — D1 + D2 :
   `scan_windows` extrait, `QUOTE_PRESENCE_PROBE_CHARS`, la sonde sur les deux bras,
   `QuoteSpanTooShort` et ses cinq sites (`QuoteSearch`, `AnchorMissReason`, `describe`, compteur,
   priorité d'agrégation), la sixième variante dans `verdicts_et_dispositions.rs`. Plancher : les
   tests de D1/D2 passent, et le suite existante de `review_anchor` reste verte sans modification.
2. **`fix(review-anchor): un libellé en gras suivi d'une annotation reste un libellé (mika#3127)`** —
   D3 : `match_anchor_prefix`, `LABEL_ANNOTATION_MAX_CHARS`, le site d'appel l. 803-813. Plancher :
   la table de formes passe, y compris ses deux contrôles négatifs.
3. **`docs(agent-loop): les trois causes de l'axe citation et les formes de libellé lues (mika#3127)`** —
   la section *review anchor* de `docs/agent-internals/agent-loop.md` : le tableau des trois causes,
   la sonde et sa propriété de sûreté, les formes de libellé lues, et la sonde de lecture
   d'après-déploiement. Hors budget de lignes.

Fin de pipeline : `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt`,
`scripts/check-byte-slices.sh`, `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`.

**`scripts/check-byte-slices.sh` est un plancher, pas une formalité** : `scan_windows` et
`match_anchor_prefix` manipulent des positions dans des briefs français, et le fichier porte deux
commentaires qui disent que le découpage par octets a déjà paniqué ici (l. 709-711, l. 734-736).
`match_anchor_prefix` rend un `&str` découpé à un offset d'octet obtenu de `char_indices`, donc sur
une frontière de caractère par construction — à écrire ainsi, jamais par arithmétique sur un index.

## Ce que la mesure d'après-vol doit tenir

Aucune assertion hors ligne ne peut dire si l'architecte **obéit** à un meilleur message — c'est le
même aveu que mika#2794 (l. 53-57) et il tient ici mot pour mot. Le protocole, à inscrire dans la
doc du commit 3 :

- `guard.review_anchor_withheld`, champ `miss_reason` : l'apparition de `QuoteSpanTooShort` est le
  **positif** de D1/D2. Sa valeur isolée ne dit rien ; ce qui se lit c'est la bascule — des
  `QuoteNotInBrief` qui deviennent des `QuoteSpanTooShort` sur la même population d'ancres courtes.
- `guard.review_anchor_armed` (mika#3003) fournit le dénominateur. Le taux à surveiller est
  retenues / passages armés, 14 jours avant / 14 jours après, et la fenêtre « avant » se reconstruit
  hors ligne depuis `llm_calls.response_text` (colonne v31) comme mika#3003 l'a établi.
- `anchors_found: 0` sur un tour d'architecte est le symptôme de D3. Après déploiement il doit
  tomber ; s'il ne tombe pas, la forme écrite n'est pas celle qui a été mesurée, et c'est une
  nouvelle mesure — pas un élargissement du matcheur à l'aveugle.
- **L'absence de ces lignes ne vaut rien avant `make deploy`** (mika#2340) : l'instrument est dans le
  binaire.

## Definition of Done

- [ ] `find_brief_quote_range` distingue trois issues sur l'axe de la citation, et la fenêtre
      décisionnelle reste à `min_quote_chars`.
- [ ] `verify_review_anchors` reconnaît les trois formes de libellé mesurées, et refuse toujours une
      ligne de prose.
- [ ] Aucun seuil de `skill.toml` modifié ; aucune signature publique modifiée.
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`,
      `scripts/check-byte-slices.sh`, `make verify-bundled-skills` et
      `scripts/verify-pipeline.sh origin/main` passent.
- [ ] Les doc-comments de `ATTRIBUTION_MIN_QUOTE_CHARS` et de `QUOTE_PRESENCE_PROBE_CHARS` se citent
      mutuellement et disent pourquoi les deux nombres diffèrent.
- [ ] `docs/agent-internals/agent-loop.md` porte les trois causes, les formes de libellé lues et le
      protocole de mesure d'après-déploiement.

## Acceptance criteria

Le corps de mika#3127 ne porte pas de section `## Acceptance criteria` ; les critères ci-dessous sont
dérivés des deux défauts mesurés et de la décision de conception.

- [ ] **AC1 (commentaire 62).** Sur un brief contenant « Une ligne ajoutée, aucun code » et une ligne
      d'ancre de plus de 40 caractères dont la seule intersection verbatim avec le brief est cette
      phrase de 29 caractères, `verify_review_anchors` rend
      `Missing { reason: QuoteSpanTooShort, .. }` et l'entrée `per_anchor` correspondante porte
      `refused: Some(QuoteSpanTooShort)` — jamais `QuoteNotInBrief`.
- [ ] **AC2 (commentaire 6, cas 1).** Sur la ligne `A3: "Total estimé : 358 lignes"`, dont le contenu
      fait moins de 40 caractères et figure dans le brief, la cause rendue est `QuoteSpanTooShort`.
      Sur `A3: ok` — plus courte que la sonde — elle reste `QuoteTooShort`.
- [ ] **AC3 (contrôle négatif de la sonde, le plus important).** Une ancre de plus de 40 caractères
      qui paraphrase le brief sans en partager 16 caractères consécutifs rend toujours
      `QuoteNotInBrief` ; et le croupion de mika#2037 reste refusé. La sonde n'a pas avalé la classe
      qu'elle borde.
- [ ] **AC4 (la sonde n'accepte rien).** Pour chaque cas de la table de tests, `anchors_valid` est
      identique à ce que rend le code d'avant le vol, et aucune entrée refusée avant ne devient
      acceptée. Une ancre dont la seule intersection avec le brief fait 20 caractères laisse
      `anchors_valid` à 0.
- [ ] **AC5 (commentaire 18, formes de libellé).** `verify_review_anchors` rend `anchors_found: 3`
      sur une réponse dont les trois ancres sont écrites `**A1** (déjà acceptée, conservée) : "…"`,
      et sur la variante `**A1** : "…"`. Une table de formes couvre aussi `A1:` et `**A1:**`
      (non-régression de mika#2786) et `A10:`.
- [ ] **AC6 (contrôles négatifs du libellé).** `(A1) : x` et `**Attention** : 1 : x` ne sont pas
      reconnues comme ancres ; une annotation de plus de `LABEL_ANNOTATION_MAX_CHARS` caractères non
      plus.
- [ ] **AC7 (l'annotation n'achète pas de longueur).** Une ligne
      `**A1** (<40+ caractères verbatim du brief>) :` suivie d'un reste vide ou court est **refusée** :
      la parenthèse est consommée avec le libellé et ne compte pas dans la citation.
- [ ] **AC8 (la porte du registre reste fermée).** Sur une ancre refusée en `QuoteSpanTooShort`,
      `ToolReadLedger::attribute` rend `None`, et `format_anchor_diagnosis` n'émet ni phrase
      d'attribution à un outil, ni la phrase conditionnelle « absente du brief ». Il émet la phrase
      « allongez-la » de `describe()`.
- [ ] **AC9 (non-régression de bout en bout).** Une réponse à trois ancres valides et dispersées
      reste `Satisfied` ; `mika2338_the_finding_line_never_carries_a_declared_line_in_clear` couvre
      les six variantes ; les trois `skill.toml` portent les mêmes seuils qu'avant le vol.

## Fire-Disposition

Le vol livre des détecteurs : les tests unitaires de `review_anchor.rs`, dont le chemin de succès
est « aucune violation trouvée ». Option **(a) — exception nommée en allowlist**, avec la même
particularité que mika#3124 :

**L'allowlist est vide par construction, et ne peut pas ne pas l'être.** Les détecteurs de ce vol
s'évaluent sur des fixtures littérales écrites dans le fichier de test — des couples
(ligne d'ancre, brief) tirés des trois commentaires de mesure. Il n'y a aucune population statique
de violations à recenser : pas de fichier à lister, pas de ligne de code existante à exempter, pas
de site de production à migrer, et aucun scan de source n'est livré (§ *Hors périmètre, nommé*).
Un détecteur dont l'entrée est une constante du test ne peut rougir que sur une régression du code
qu'il garde. Il n'y a donc **aucune entrée à créer**, et l'assertion auto-nettoyante de la forme (a)
porterait sur l'ensemble vide.

Deux conséquences existantes sont néanmoins nommées, parce que ce sont elles qui « tirent » au
moment du vol et qu'aucune n'est une exemption :

1. **`verdicts_et_dispositions.rs:188-194` doit être complété, pas exempté.** Sa table énumère les
   cinq variantes de `AnchorMissReason` ; elle en compte six après D2. Le test compile sans le
   changement (c'est une table, pas un `match` exhaustif), donc rien ne le signalerait — c'est
   précisément pourquoi le geste est inscrit dans le commit 1 et tenu par AC9.
2. **Aucun test existant de `review_anchor.rs` n'a à changer de valeur attendue.** C'est une
   prédiction vérifiable, pas un vœu : D1 ne change le `reason` que sur des entrées où la fenêtre de
   40 échouait *et* où un fragment de 16 caractères est présent, et D3 n'ajoute que des formes
   auparavant non reconnues. Si un test existant redevient rouge sur une valeur attendue, la
   prédiction est fausse : l'implémentation **s'arrête et remonte à l'opérateur** (option (c)) au
   lieu de réécrire l'attente — un test d'ancrage qui change de valeur attendue est la trace d'un
   seuil qui a bougé, et ce vol a déclaré n'en bouger aucun.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `review_anchor.rs` — `match_anchor_prefix` + `LABEL_ANNOTATION_MAX_CHARS` + doc (D3) | 62 |
| `review_anchor.rs` — `scan_windows` extrait + sonde + `QUOTE_PRESENCE_PROBE_CHARS` + doc (D1) | 46 |
| `review_anchor.rs` — `QuoteSpanTooShort` sur ses cinq sites + doc (D2) | 24 |
| tests inline `review_anchor.rs` (1 table de formes + 8 tests + fixtures) | 120 |
| `agent_loop/tests/verdicts_et_dispositions.rs` (sixième variante dans la table) | 1 |

Total estimé : 253 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Un seul vol, et voici pourquoi il tient : les trois gestes vivent dans **un seul fichier de
production**, aucun ne crée de table, de migration, d'option de configuration ni de frontière de
crate, et aucun ne touche `skills/bundled/` — donc pas de `make deploy` dans la boucle de
vérification. Le seul fichier hors de `review_anchor.rs` reçoit **une** ligne. La marge au-dessus de
la borne est de 26 lignes ; le poste qui peut déraper est celui des tests, et il est borné par
construction en rendant la couverture des formes de libellé **table-dirigée** (une fonction de test,
une table de six formes) plutôt qu'un test par forme.
