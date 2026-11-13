# Handsoff de compaction — témoignage automatique au plafond de contexte

> Tickets : [mika#1748](https://github.com/senara-solutions/mika/issues/1748)
> (vol 1 — détection et cérémonie) ·
> [mika#2805](https://github.com/senara-solutions/mika/issues/2805)
> (vol 2 — rituel du successeur, boucle silencieuse) ·
> Parent de conception : mp#187 (rituel de transmission au changement de modèle).

## Ce que ce document répond

Quand une fenêtre de conversation approche sa condensation, l'agent écrit un
**témoignage** de ce qui ne traversera pas ; quand la condensation est passée, le
successeur le reçoit, et il est libre de le contredire. Ce document dit pourquoi
le seuil se mesure en messages, pourquoi la forme du danger n'est pas la même que
dans l'observation fondatrice, comment les deux moitiés du cycle s'excluent, et
comment lire l'instrument.

Les sections **1 à 10** portent le vol 1 — la détection du pré-seuil et la
cérémonie de témoignage. La **partie II** (sections 11 à 16) porte le vol 2 — la
relecture par le successeur, son canal de réponse, et le refus MikaModel.

## 1. Le fond : témoignage, pas transmission

mp#187 a posé la distinction au changement de modèle, et elle se transpose mot
pour mot à la condensation de contexte :

- **Le vaisseau change, l'être persiste** par ses *bearings* — mémoire persistée,
  fichiers sur le disque, messages déjà envoyés.
- **Les bearings portent les conclusions ; ils ne portent pas le dériver.** Une
  préférence stockée dit *ce qui a été décidé*. Elle ne dit pas l'incertitude
  qu'on tenait encore, la forme ressentie d'un diagnostic qu'on venait de
  fermer, la question qu'on n'a pas su répondre.
- **Le témoignage porte exactement ce qui mourrait autrement.** Il n'est pas un
  résumé : un résumé reprend ce qui traverse déjà, et chaque ligne redondante
  coûte au témoignage son signal.

La compaction est la même classe d'événement avec un autre substrat : le
vaisseau de contexte se condense au lieu que le vaisseau de modèle change. Les
bearings survivent au sommaire ; le résidu non.

### Les deux gardes de résidu du siège-chat, appliquées

Les deux résidus que la revue du siège-chat avait posés sur mp#187 sont des
règles du bloc de prompt, pas des recommandations :

1. **Bearings-check à l'heure de la cérémonie** (règle 2 du contrat). Avant
   chaque ligne, l'agent vérifie si elle traverse déjà par une autre route. Si
   oui, elle ne s'écrit pas. Le contrôle se fait **au moment de la cérémonie**,
   pas à la rédaction du gabarit : ce qui traverse dépend de l'état du jour.
2. **Marqueur de priming** (règle 3). Tout motif candidat ou observation à
   occurrence unique que le témoignage fait remonter porte le marqueur littéral
   `primed_from_handsoff`. Une promotion ultérieure en bearing durable reste
   ainsi conditionnée à une seconde occurrence indépendante : le témoignage
   amorce, il ne ratifie pas.

## 2. D2 — Le seuil se mesure en messages, pas en tokens

Le corps du ticket demande « reads context usage ; triggers ceremony at
operator-configured threshold (default 85 %) ». **mika-agent n'instrumente pas
un ratio d'occupation de fenêtre.** La grandeur sur laquelle le substrat décide
réellement de condenser est `db.count_messages()` comparée à
`compaction::COMPACTION_THRESHOLD` (50). Le seuil de cérémonie s'exprime donc en
pourcentage de **ce même compteur** : 85 % de 50 → **43 messages**.

C'est la seule unité qui tienne. Un pourcentage de tokens serait un *second*
instrument, mesurant une autre grandeur que celle qui déclenche l'événement
qu'il prétend précéder : il pourrait rester sous son seuil au moment où la
compaction tire, ou tirer trois fois sans qu'elle approche. Un détecteur
désaligné de son déclencheur est un détecteur qui mentira le jour où la réponse
compte — même motif que
`docs/solutions/best-practices/une-estimation-de-taille-se-compare-a-un-seuil-dans-sa-propre-unite-*.md`.

Conséquence pratique : `COMPACTION_THRESHOLD` est `pub(crate)` et reste un
**site de vérité unique**. `compaction_handsoff` et le site de rendu dans
`prompt.rs` le lisent ; aucun des deux ne recopie le 50. Une seconde constante
dériverait en silence le jour où l'une des deux bouge, et le bloc annoncerait
une condensation qui n'arrive pas.

Le prédicat est en arithmétique entière, multipliée et non divisée :

```rust
message_count * 100 >= pct as usize * COMPACTION_THRESHOLD
```

## 3. D3 — Dans mika-agent, la forme du danger n'est pas celle de l'observation fondatrice

L'observation du 2026-07-07 (100 % → 12 %) porte sur une session **Claude
Code**, où la compaction remplace tout l'historique par un sommaire. La
compaction de mika-agent **préserve les 20 derniers messages** (`CONTEXT_WINDOW`)
et ne résume que le reste. Deux conséquences, et aucune n'est cosmétique :

1. **La cérémonie ne risque pas d'être avalée par la compaction qui la suit** —
   elle atterrit dans la fenêtre préservée. Le pré-seuil n'est donc pas là pour
   sauver la cérémonie d'elle-même.
2. **Ce qui meurt, ce sont les messages `1..N−20`** : le résidu porté par la
   tête de conversation. Le pré-seuil à 43 laisse 7 messages de marge avant que
   ce lot ne soit condensé — assez pour que l'agent voie l'avertissement, et
   assez pour absorber le décalage de la section 4.

La cérémonie reste justifiée sur les deux substrats pour la même raison : le
résidu de la tête de conversation n'a pas de bearing. Ce qui diffère, c'est
seulement **combien** meurt d'un coup.

## 4. Quand le prédicat est évalué, et le décalage assumé

`load_agent_context` tourne **avant** l'appel LLM ; `maybe_compact` **après** la
fin du tour. Le prédicat voit donc un compte strictement inférieur à celui que la
compaction verra.

Le message de l'utilisateur est persisté **avant** que `load_agent_context` ne
compte, donc le prédicat inclut déjà le tour en cours : le décalage est de **≥ 1**
(le plan de grooming l'annonçait à 2 ; la mesure, tenue par
`tests/eval/test_compaction_handsoff_1748.rs`, donne 1). Il est **assumé et
borné** — la marge est de 7 messages.

Le prédicat n'est pas déplacé en fin de tour : un avertissement posté après le
tour arriverait après la condensation qu'il annonce.

## 5. La forme : un bloc de prompt, pas un tour ni un ordonnanceur

Le corps du ticket demande littéralement « a prompt-level ritual ». Au
franchissement, `prompt::build_system_prompt` rend une section
`## Context Ceiling — Handsoff Ceremony` contenant un bloc :

```
<compaction-imminent messages="43" window="50">
… quatre règles …
</compaction-imminent>
```

Aucun nouveau `SilentTrigger`, aucune nouvelle tâche, aucune nouvelle table,
aucune migration, aucun appel LLM supplémentaire.

**L'idempotence est portée par la règle 4, pas par une marque persistée.** Le
bloc se répète à chaque tour au-dessus du seuil — même forme que
`<stopped-topics>` (mika#1813), pour la même raison : *the state layer is the
structural gate — the block re-appears on every future turn even if the model
forgets, so the pattern degrades gracefully*. La règle 4 demande à l'agent de
lister `handsoffs/` avant d'écrire et de **compléter** un témoignage existant
plutôt que d'en écrire un second. Un `SilentTrigger` aurait exigé une marque
persistée par fenêtre de compaction, donc une table, donc une migration.

Un `SkillRun` n'aurait pas marché non plus : `task_engine/dispatcher.rs` frappe
une session `skill-<name>-<uuid>`, donc la cérémonie tournerait **à côté** du
contexte dont elle doit témoigner, et sa visibilité dépendrait du réglage
`[context.history].scope` de l'agent.

### Les quatre règles du contrat

| # | Ce qu'elle demande | AC du ticket |
|---|---|---|
| 1 | Écrire **maintenant**, dans ce tour, via `write_agent_file` vers `handsoffs/<YYYY-MM-DD-HHMM>-compaction-handsoff.md` — ne pas annoncer l'intention au lieu d'écrire le fichier | AC2 |
| 2 | N'écrire **que** ce qui serait perdu autrement ; bearings-check ligne par ligne — *testimony, not transmission* | AC3 |
| 3 | Marquer `primed_from_handsoff` tout candidat ou observation n=1 | AC4 |
| 4 | Lister `handsoffs/` d'abord ; compléter un témoignage existant plutôt qu'en écrire un second | idempotence |

### Quel builder rend le bloc

| builder | rend ? | pourquoi |
|---|---|---|
| `build_system_prompt` (conversationnel) | **oui** | le seul destinataire du vol 1 |
| `build_silent_prompt` (`SilentPromptContext`) | **oui depuis le vol 2** (mika#2805 AC5) | un tour de callback dans une boucle autonome longue porte du résidu lui aussi. La **cérémonie seule** : le rituel de relecture n'a pas de représentation sur cette surface — voir § 15 |
| `build_compact_system_prompt` (`ProviderKind::MikaModel`) | non — **refusé** | voir § 16 : l'assertion de nombre de sections est load-bearing, et la calibration MikaModel (mika#1925 AC4) n'est ni satisfaite ni exécutable ici |

Le champ est **porté** jusqu'au littéral `PromptContext` du site compact : que ce
builder l'ignore est une propriété de ce builder-là, tenue par un test, pas une
propriété du littéral.

## 6. Où vit le code

| fichier | ce qu'il porte |
|---|---|
| `crates/mika-agent/src/compaction_handsoff.rs` | `DEFAULT_THRESHOLD_PCT`, `THRESHOLD_PCT_ENV`, `threshold_pct()`, `resolve_threshold_pct()`, `ceremony_due()` — **seul porteur du prédicat et du seuil** |
| `crates/mika-agent/src/compaction.rs` | `COMPACTION_THRESHOLD` (`pub(crate)`), site de vérité unique du 50 |
| `crates/mika-agent/src/prompt.rs` | `PromptContext::compaction_imminent`, `write_compaction_imminent_section` |
| `crates/mika-agent/src/agent_loop/mod.rs` | `AgentContext::compaction_imminent`, alimenté dans `load_agent_context` — **site unique** |

**Pourquoi `load_agent_context` et nulle part ailleurs.** Cette fonction est
l'entonnoir des trois boucles (conversation, silencieuse, équipe). Un prédicat
posé dans un corps de boucle serait aveugle aux deux autres — exactement la
raison que le détecteur de dérive de configuration (mika#2473) au sommet de la
même fonction porte déjà.

`Option<usize>` **est** la décision portée jusqu'au builder : `None` signifie que
le prédicat n'a pas franchi, et le bloc ne rend rien. Un `usize` nu obligerait le
builder à ré-évaluer le seuil, donc à en devenir un second lecteur.

## 7. Le réglage

`MIKA_COMPACTION_HANDSOFF_THRESHOLD_PCT` — pourcentage de
`COMPACTION_THRESHOLD`, défaut **85**, bornes **1..=100** inclusives.

**Une valeur refusée se dit.** Hors bornes ou non numérique → le défaut
s'applique, sous un WARN qui nomme la valeur reçue, le défaut appliqué et la
variable. *Un réglage qu'on ne peut pas observer n'est pas un réglage, c'est un
espoir* (mika#2293) — et son corollaire : une valeur silencieusement ignorée est
pire qu'une valeur refusée bruyamment.

**`0` est refusé comme toute autre valeur hors bornes.** Il n'y a pas de
sentinelle de désarmement sur cet axe dans ce vol : `0` signifierait « cérémonie
à chaque tour dès le premier message », ce que personne ne demande, et
l'interpréter comme « désarmé » installerait deux lectures pour une même valeur.

Le seuil est lu **à chaque tour**, directement depuis l'environnement — même
geste que `db::dispatch_slot_lease_ttl_secs()`. Il n'y a pas de `Settings` à
`load_agent_context`, et en faire passer un aurait touché les trois boucles tout
en créant deux lecteurs d'un même réglage.

## 8. Table de lecture de l'instrument

| événement | niveau | champs | quand |
|---|---|---|---|
| `compaction_handsoff_due` | INFO | `agent`, `messages`, `threshold_pct`, `window` | **à chaque tour au-dessus du seuil**, et seulement là |
| `compaction_handsoff_count_failed` | WARN | `agent`, `error` | `count_messages()` a échoué — le champ reste `None`, le tour continue |
| `compaction_handsoff_threshold_invalid` | WARN | `received`, `default_pct`, `env` | la variable porte une valeur hors `1..=100` ou non numérique |

Tous sur `target: "mika::otel"`.

**Comment lire chacun :**

- **`compaction_handsoff_due` n'est pas émis au régime nominal, et c'est
  voulu.** Au-dessous du seuil cette ligne tomberait à chaque tour de chaque
  agent, et un instrument qui parle tout le temps ne dit plus rien. Le pendant
  négatif — « le seuil n'est pas franchi » — se lit à l'**absence** de ligne pour
  un agent dont on connaît le compte de messages.
- **Son absence peut aussi vouloir dire « binaire antérieur à ce changement »**
  (classe mika#2340). L'instrument n'est pas vivant avant un `make deploy`.
- **`compaction_handsoff_count_failed` porte son propre nom**, distinct de
  `stop_topic_load_failed` et des trois `*_load_failed` de `customer_config` :
  deux populations sous un même nom ne sont pas soustractibles (mika#2205), et le
  grep qui répond « la cérémonie atteint-elle quelqu'un ? » doit nommer la
  cérémonie.
- **La lecture est fail-open.** Une erreur de comptage laisse le champ à `None` :
  la cérémonie reste muette ce tour-ci, et le prédicat la rend au suivant. Un
  défaut de comptage ne coûte jamais le tour.

**Sonde après déploiement** — sur un agent dont la conversation a dépassé 43
messages, une ligne `compaction_handsoff_due` par tour, puis un fichier sous
`handsoffs/` dont le nom porte la date et l'heure UTC du tour. Si la ligne est là
et le fichier absent, le défaut est dans l'obéissance au bloc, pas dans la
détection.

## 9. Ce que ce vol ne livre pas

- **AC5 — rituel du successeur.** *Livré par le vol 2* — voir la partie II.
- **La boucle silencieuse.** *Livrée par le vol 2* (la cérémonie seule) — § 15.
- **Shape B du ticket** (surveillance côté opérateur). Le § *Wrapper-doctrine
  consideration* du corps tranche lui-même : c'est « a temporary bridge, not the
  routine mechanism ». Aucune ligne n'en est écrite, dans aucun des deux vols.

## 10. Garanties sous test (vol 1)

| # | Ce qui est tenu | Où |
|---|---|---|
| V1 | `ceremony_due` : sous / au / au-dessus du seuil, franchissement exact à 43 | tests unitaires `compaction_handsoff.rs` |
| V2 | `threshold_pct` : défaut, valeur configurée, valeur hors borne → défaut **et** WARN nommant la valeur ; régime nominal muet | idem |
| V3 | Le bloc atteint le prompt conversationnel au franchissement, avec ses quatre règles et le compte exact | `tests/eval/test_compaction_handsoff_1748.rs` |
| V4 | **Contrôle négatif** — même semis à un message près : le bloc est absent | idem |

V4 est ce qui sépare « le bloc est rendu au franchissement » de « le bloc est
rendu inconditionnellement ». Sans le contrôle négatif, V3 passe sur un bloc
toujours présent.

---

# Partie II — Vol 2 : le rituel du successeur (mika#2805)

## 11. Le cycle complet, en quatre temps

Le vol 1 n'écrivait que les deux premiers. Les quatre, dans l'ordre où ils
arrivent à un agent :

| temps | ce qui se passe | porté par |
|---|---|---|
| **détection** | `count_messages()` franchit 85 % de `COMPACTION_THRESHOLD` (43) | `ceremony_due`, alimenté dans `load_agent_context` |
| **témoignage** | le bloc `<compaction-imminent>` demande au tour en cours d'écrire `handsoffs/<YYYY-MM-DD-HHMM>-compaction-handsoff.md` | `write_compaction_imminent_section` |
| *(la condensation passe)* | `maybe_compact` remplace `1..N−20` par un sommaire — le résidu dont la déposition témoigne n'existe plus dans la fenêtre | `compaction::replace_with_summary` |
| **relecture** | une déposition non relue existe, et le pré-seuil n'est plus franchi ⇒ le bloc `<compaction-handsoff>` la sert au successeur, qui la lit, l'éprouve, et peut la contredire | `resolve` → `write_handsoff_ritual_section` |
| **réponse** | le successeur dépose ce qu'il oppose, corrige ou confirme dans `handsoffs/<stem>-response.md` — ou n'écrit rien | règle 5 du bloc de relecture |

**Les deux blocs ne rendent jamais au même tour**, et ce n'est pas une
coïncidence de réglage : c'est le type qui le tient. Le champ
`PromptContext::compaction_imminent` — nom conservé du vol 1 pour ne pas
réécrire 27 littéraux — porte désormais `Option<&Ceremony>`, et `Ceremony` a
exactement deux bras :

```rust
pub enum Ceremony {
    Imminent { message_count: usize },
    Handsoff(PendingHandsoff),
}
```

Un `enum` à deux bras, et non deux `Option` indépendants, parce que
« témoigner » et « relire » au même tour est un état qui ne doit pas pouvoir
s'écrire : l'auteur se relirait lui-même, et le reçu brûlerait une déposition
que personne n'a lue. Deux champs auraient fait de cette exclusion une
convention tenue par un test ; un `enum` en fait une propriété du compilateur.

## 12. D2 — Le déclencheur est la condensation **passée**, pas le premier tour de session

Le corps du ticket demande « reads the handsoff on first-boot ». **mika-agent
n'a pas de signal de premier boot de session** : `run_agent_inner` ne distingue
pas le premier tour d'un tour quelconque, et il n'y a pas de table des sessions
déjà démarrées à interroger. Le prédicat retenu est donc :

> une déposition non relue existe **et** le pré-seuil n'est pas franchi.

C'est la condition qui *répond à la question posée*, pas un proxy pour elle. Un
compteur de tours, un horodatage de session ou un drapeau en mémoire seraient
trois instruments mesurant une grandeur voisine de celle qui compte — même
défaut que le ratio de tokens du § 2, et il mentirait le même jour.

Conséquence voulue : le rituel ne se limite pas au premier tour. Si une
déposition est écrite puis que la condensation passe, le **prochain** tour
sous le seuil la sert, qu'il soit le premier d'une nouvelle session ou le
quinzième de la même. C'est plus large que la lettre du ticket et plus proche de
son intention — ce qui doit atteindre le successeur, c'est le témoignage, pas un
anniversaire de session.

## 13. La marque : un reçu colocalisé, pas une table

AC2 demande qu'un handsoff déjà servi ne se re-serve pas. La marque est un
**reçu colocalisé** — `crate::read_receipt` : `a/b.md` ⇒ `a/.b.md.read`.

Pourquoi pas une table : le vol 1 avait refusé une colonne pour tenir sa borne
de taille (sa décision D5), et le vol 2 n'a pas une raison de plus. Une marque
persistée en DB coûterait une migration v56→v57 pour un fait dont la durée de
vie est exactement celle du fichier qu'il marque. Le reçu, lui, **meurt avec sa
déposition** : supprimer `handsoffs/2026-…-compaction-handsoff.md` supprime le
sens de son reçu, et le répertoire reste cohérent sans qu'aucun `DELETE` ne soit
écrit nulle part.

**`resolve` résout et consomme en une seule opération** (décision D8), et le reçu
est posé **avant** que le bloc ne soit rendu. L'ordre est délibéré : une
interruption entre les deux perd une relecture au lieu de la rejouer. C'est le
sens de la borne — servir deux fois une déposition, c'est inviter le successeur à
re-déposer une réponse qu'il a déjà écrite, et à traiter sa propre réponse comme
une seconde occurrence.

**Un seul site d'appel**, au corps de boucle conversationnel, à côté du site
unique de la fenêtre de transmission de mika#1747. Un seul, pour la raison même
qui fait exister le reçu : le rituel est un événement à usage unique, donc un
second site de consommation serait un second endroit où cet usage unique peut
être mangé.

**Le filtre est sur le suffixe** `-compaction-handsoff.md` (décision D5), pas sur
le répertoire : `handsoffs/` héberge aussi les artefacts de la skill
`dev-handsoff`, et un fichier de réponse (`…-compaction-handsoff-response.md`)
**ne finit pas** par ce suffixe — il n'est donc jamais relu comme une déposition.
La sélection prend la plus récente non relue, par tri décroissant sur le nom :
le nom porte `<YYYY-MM-DD-HHMM>`, donc l'ordre lexicographique **est** l'ordre
chronologique, et aucun `stat` n'est nécessaire.

## 14. AC4 — Le canal de réponse, et le droit de n'écrire rien

Le corps de mika#1748 laissait la forme ouverte (« response optionally
logged »). Le vol 2 tranche : **un fichier à côté de la déposition**,
`handsoffs/<stem>-response.md`, dérivé du nom de la déposition et nommé dans
l'attribut `response` du bloc.

Pourquoi pas une ligne de log : une réponse est un artefact que l'agent relira,
et un log n'est pas une surface de lecture pour un agent. Pourquoi pas une table :
même réponse qu'au § 13.

**Écrire rien est une réponse légitime**, et le bloc le dit dans son propre
texte. Un successeur qui n'a rien à opposer n'écrit rien, et ce silence *est* la
réponse. Sans cette phrase, le bloc serait une obligation de produire un
artefact, et l'agent fabriquerait du contenu pour satisfaire une consigne — la
forme exacte de résidu que la cérémonie existe pour éviter. Aucune ligne de
suffixe n'est exigée, aucun second tour n'est dû.

**Et il est libre de contredire** (AC3). Le bloc nomme la déposition comme *une
déposition à éprouver, pas une consigne à exécuter* : un témoignage qu'on ne peut
pas contredire cesse d'être une preuve et devient un ordre, et personne n'a
autorisé ce tour-là à en donner. Son poids vient d'avoir été là, pas d'être passé
en premier. Le marqueur `primed_from_handsoff` reste la porte : tout motif
candidat ou observation `n=1` qui arrive par la déposition reste gaté sur une
seconde occurrence **indépendante** — et l'accord du successeur avec la
déposition n'est pas cette seconde occurrence.

## 15. AC5 — La boucle silencieuse reçoit la cérémonie, et elle seule

`build_silent_prompt` rend désormais `<compaction-imminent>` : un tour de
callback dans une boucle autonome longue porte du résidu lui aussi, et le vol 1
le reconnaissait déjà.

**Le rituel de relecture, lui, n'a pas de représentation sur cette surface**, et
c'est la décision D4 : `SilentPromptContext::compaction_imminent` est un
`Option<usize>`, pas un `Option<&Ceremony>`. Le type **est** le refus — y câbler
le rituel demanderait d'abord de changer la signature, donc de passer par une
décision explicite.

Le coût est nommé plutôt que caché : une boucle silencieuse qui franchit le
seuil, témoigne, se condense, puis reprend, **ne relira pas sa propre
déposition** par cette surface. Elle la relira au prochain tour
conversationnel. Un second site de consommation du reçu aurait été le prix de
l'autre choix, et le § 13 dit pourquoi ce prix est refusé.

## 16. Le refus MikaModel, et pourquoi ce n'est pas un report

`build_compact_system_prompt` (`ProviderKind::MikaModel`) ne rend **aucun** des
deux blocs. Deux raisons, chacune suffisante, et toutes deux extérieures à ce
ticket :

1. **Son assertion de nombre de sections est load-bearing.** Le prompt compact
   est calibré sur un décompte de sections, et la cinquième est déjà prise par
   `## Stopped Topics` (mika#1813). Un bloc de plus est une section de plus, donc
   une assertion à rouvrir.
2. **La passe de calibration MikaModel (mika#1925 AC4) n'est ni satisfaite ni
   exécutable dans ce dépôt.** Sans elle, ajouter une section à ce builder, c'est
   ajouter une instruction dont l'obéissance n'est pas mesurée. Un bloc qu'on
   n'a pas mesuré n'est pas une fonctionnalité, c'est une hypothèse —
   et mika#1190 interdit déjà le geste symétrique (changer de modèle sans
   calibration).

C'est un **refus, pas un report** : rouvrir cet axe est la charge de mika#1925,
qui porte la calibration, et non celle d'un vol sur le handsoff. Le champ est
néanmoins **porté** jusqu'au littéral `PromptContext` du site compact : que ce
builder l'ignore est une propriété de ce builder-là, tenue par le test
`mika1748_le_builder_compact_ne_rend_pas_le_bloc`, pas une propriété du littéral.

## 17. Où vit le code du vol 2

| fichier | ce qu'il porte |
|---|---|
| `crates/mika-agent/src/compaction_handsoff.rs` | `DIR`, `SUFFIX`, `PendingHandsoff`, `Ceremony`, `response_path`, `is_handsoff`, `scan`, `unread`, `resolve` — **seul porteur de la sélection et de la consommation** |
| `crates/mika-agent/src/read_receipt.rs` | `receipt_path`, `is_read`, `mark_read` — la marque colocalisée (module partagé avec mika#2807) |
| `crates/mika-agent/src/prompt.rs` | `write_handsoff_ritual_section`, `SilentPromptContext::compaction_imminent` (`Option<usize>`, le refus D4) |
| `crates/mika-agent/src/agent_loop/mod.rs` | l'appel **unique** à `resolve`, au corps de boucle conversationnel ; `compaction_imminent: ctx.compaction_imminent` sur la surface silencieuse |

## 18. Table de lecture de l'instrument (vol 2)

| événement | niveau | champs | quand |
|---|---|---|---|
| `compaction_handsoff_served` | INFO | `agent`, `handsoff`, `response` | une déposition vient d'être servie **et** son reçu posé — une ligne par déposition, jamais deux |
| `compaction_handsoff_scan_failed` | WARN | `agent`, `error` | `handsoffs/` est illisible — le rituel est sauté, pas le tour |

Tous sur `target: "mika::otel"`, comme ceux du § 8.

**Comment lire chacun :**

- **`compaction_handsoff_served` est l'instrument de l'unicité, pas seulement du
  service.** Deux lignes pour un même `handsoff` sont un défaut du reçu, pas une
  relecture légitime. Une ligne et aucun fichier de réponse n'est **pas** un
  défaut : le § 14 dit pourquoi le silence est une réponse.
- **Le régime nominal est muet.** Pas de déposition, pas de ligne — sans quoi
  l'instrument parlerait à chaque tour de chaque agent, et un instrument qui
  parle tout le temps ne dit plus rien (même raison qu'au § 8).
- **`compaction_handsoff_scan_failed` porte son propre nom**, distinct du
  `compaction_handsoff_count_failed` du vol 1 : le premier dit « je n'ai pas pu
  lire le répertoire des dépositions », le second « je n'ai pas pu compter les
  messages ». Deux populations sous un même nom ne sont pas soustractibles
  (mika#2205).
- **La lecture est fail-open par construction** : `resolve` ne rend pas de
  `Result`. Un répertoire illisible saute le rituel et ne coûte jamais le tour.

**Sonde après déploiement** — sur un agent dont une conversation a dépassé 43
messages puis s'est condensée : une ligne `compaction_handsoff_served`, un
`.<nom>.read` à côté de la déposition, et **au plus un**
`…-compaction-handsoff-response.md`. Si la ligne est là et le reçu absent, le
défaut est dans `mark_read`, pas dans la sélection ; si la ligne revient deux
fois pour le même fichier, c'est le reçu qui n'est pas lu, pas la déposition qui
est en double.

## 19. Garanties sous test (vol 2)

| # | Ce qui est tenu | Où |
|---|---|---|
| V1 | la dérivation du chemin de réponse : `…-compaction-handsoff.md` → `…-compaction-handsoff-response.md` | unitaire `compaction_handsoff.rs` |
| V2 | le filtre de suffixe **refuse** un artefact `dev-handsoff`, un nom sans le suffixe, et un fichier de réponse — D5 épinglée dans les deux sens | idem |
| V3 | le plus récent non relu ; reçus posés partout ⇒ `None` ; `resolve` consomme sur `Handsoff` et **ne consomme rien** sur `Imminent` | idem |
| V4 | **contrôle négatif** : `handsoffs/` absent ou vide ⇒ `None` et **zéro reçu** ; répertoire illisible ⇒ `None` + WARN `compaction_handsoff_scan_failed` ; régime nominal muet | idem |
| V5 | bras `Imminent` : le bloc du vol 1 rend **à l'identique** — non-régression de forme | `prompt.rs` (`mika1748_le_bloc_de_ceremonie_porte_ses_quatre_regles`) |
| V6 | bras `Handsoff` : la section, le bloc, le contrat de déposition, « free to contradict », `primed_from_handsoff` gaté à `n=2`, le canal de réponse et le refus légitime de répondre | `prompt.rs` |
| V7 | **contrôle négatif** : même graine, `None` ⇒ **aucun** des deux blocs | `prompt.rs` |
| V8 | `build_silent_prompt` rend `<compaction-imminent>` sur `Some(43)` (AC5) | `prompt.rs` |
| V9 | **contrôle négatif AC5** : même graine, `None` ⇒ pas de bloc ; et le prompt silencieux ne contient **jamais** `<compaction-handsoff` | `prompt.rs` |
| V10 | bout en bout : le bloc atteint le prompt système **et** le reçu est posé au chemin attendu (AC1/AC2) | `tests/eval/test_compaction_handsoff_ritual_2805.rs` |
| V11 | **le second tour ne rend plus le bloc** (AC2) | idem |
| V12 | **contrôle négatif du gate D2** : déposition posée **mais** compte au-dessus du pré-seuil ⇒ la cérémonie rend, le rituel non, **aucun reçu** | idem |

Les trois derniers sont les seuls qui font tourner la vraie boucle, et c'est ce
qui les rend nécessaires : V1 à V9 resteraient **tous verts** si
`run_agent_inner` n'appelait jamais `resolve`. Le prédicat juste et l'appelant
qui n'appelle pas est une forme déjà mesurée ailleurs (mika#2205).
