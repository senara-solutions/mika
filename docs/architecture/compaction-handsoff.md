# Handsoff de compaction — témoignage automatique au plafond de contexte

> Ticket : [mika#1748](https://github.com/senara-solutions/mika/issues/1748) ·
> Parent de conception : mp#187 (rituel de transmission au changement de modèle) ·
> Vol 1/2 — la suite (rituel du successeur après la compaction, boucle
> silencieuse) est suivie à `senara-solutions/mika#2805`.

## Ce que ce document répond

Quand une fenêtre de conversation approche sa condensation, l'agent écrit un
**témoignage** de ce qui ne traversera pas. Ce document dit pourquoi le seuil se
mesure en messages, pourquoi la forme du danger n'est pas la même que dans
l'observation fondatrice, et comment lire l'instrument.

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
| `build_silent_prompt` (`SilentPromptContext`) | non | un tour de callback dans une boucle autonome longue porte du résidu lui aussi ; c'est reconnu et c'est **mika#2805**, pas un oubli |
| `build_compact_system_prompt` (`ProviderKind::MikaModel`) | non — **refusé** | son assertion de nombre de sections est load-bearing (la cinquième est déjà prise par `## Stopped Topics`), et la passe de calibration MikaModel (mika#1925 AC4) n'est ni satisfaite ni exécutable dans ce dépôt. Un bloc de plus serait une section de plus sans obéissance mesurée |

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

- **AC5 — rituel du successeur.** La session qui suit la compaction ne lit pas
  encore le témoignage à son premier démarrage, et ne peut donc pas y répondre ou
  le contredire. Suivi à `senara-solutions/mika#2805`.
- **La boucle silencieuse.** `build_silent_prompt` ne rend pas le bloc (voir la
  table de la section 5). Même suivi.
- **Shape B du ticket** (surveillance côté opérateur). Le § *Wrapper-doctrine
  consideration* du corps tranche lui-même : c'est « a temporary bridge, not the
  routine mechanism ». Aucune ligne n'en est écrite.

## 10. Garanties sous test

| # | Ce qui est tenu | Où |
|---|---|---|
| V1 | `ceremony_due` : sous / au / au-dessus du seuil, franchissement exact à 43 | tests unitaires `compaction_handsoff.rs` |
| V2 | `threshold_pct` : défaut, valeur configurée, valeur hors borne → défaut **et** WARN nommant la valeur ; régime nominal muet | idem |
| V3 | Le bloc atteint le prompt conversationnel au franchissement, avec ses quatre règles et le compte exact | `tests/eval/test_compaction_handsoff_1748.rs` |
| V4 | **Contrôle négatif** — même semis à un message près : le bloc est absent | idem |

V4 est ce qui sépare « le bloc est rendu au franchissement » de « le bloc est
rendu inconditionnellement ». Sans le contrôle négatif, V3 passe sur un bloc
toujours présent.
