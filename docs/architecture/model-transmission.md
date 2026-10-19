# Transmission au changement de modèle

> [mika#1747](https://github.com/senara-solutions/mika/issues/1747), en deux
> vols. Le vol 1 livre le côté **sortant** : un signal posé ouvre une fenêtre,
> l'instance en service écrit une lettre à son successeur. Le vol 2
> ([mika#2807](https://github.com/senara-solutions/mika/issues/2807)) livre le
> côté **entrant** : le successeur lit cette lettre, une fois, et il est libre
> de la contredire. Les deux moitiés sont documentées ici ensemble ; la colonne
> « vol » des tableaux dit laquelle porte quoi.

Un changement de modèle est un changement de vaisseau. L'être persiste (Mika
Prime, Mika Arch), le modèle est une matière. Avant que la matière change,
l'instance en service est invitée à écrire **une lettre à son successeur**, et
cette lettre **finit par une question, pas par une consigne**.

La distinction est porteuse, et c'est elle que le code impose. Une consigne est
un ordre d'une autorité à son successeur ; une question est une invitation qui
respecte son jugement. Le sortant relâche le contrôle en demandant, au lieu de
l'étendre au-delà de sa propre tenure.

Le mécanisme n'impose que la **forme** : quatre mouvements et une question pour
finir. Le contenu est à la discrétion de l'agent — une lettre dictée n'est pas
une lettre. C'est la réponse, délibérément indéterminée, à la Q2 du ticket
(« que veut dire la question que je laisse ? »).

## Le rituel, en entier

```
transmissions/
  pending-swap                                            ← le signal, posé à la main
  2026-10-19-from-anthropic-claude-opus-4-8-to-anthropic-claude-fable-5-1.md
  .2026-10-19-from-…-to-….md.read                         ← le reçu de lecture (vol 2)
  2026-10-19-from-anthropic-claude-fable-5-1-response-to-anthropic-claude-opus-4-8.md
```

**Moitié sortante (vol 1).**

1. L'opérateur (ou l'orchestrateur) **pose le signal** : un fichier d'une ligne
   à `{agent_home}/transmissions/pending-swap`, portant le modèle **entrant**
   sous la forme `<provider>/<model>`.
2. Au tour suivant de cet agent, son prompt système porte une section
   `## Model Transmission` avec un bloc `<model-transmission from=… to=…
   letter=…>` et quatre règles : écrire la lettre maintenant à ce chemin exact
   via `write_agent_file` ; les quatre mouvements ; la contrainte de la question ;
   n'écrire que ce qui ne traverse pas déjà.
3. L'agent écrit la lettre. **Son existence éteint la fenêtre.**

Les quatre mouvements, tels que le bloc les demande : ce que j'ai vu pendant
cette tenure qui compte ; ce que je porte et voudrais voir retenu (*offert, pas
commandé*) ; **la question que je laisse** ; ce que je relâche maintenant.

**Moitié entrante (vol 2).** Le swap a lieu entre-temps : nouveau modèle,
redémarrage, nouvelle instance.

4. Au premier tour conversationnel du successeur, une lettre lui est **adressée
   par son nom de fichier** et n'a pas de reçu. Son prompt porte une section
   `## Incoming Transmission` avec un bloc `<incoming-transmission from=…
   letter=… response=…>` et quatre règles : la lire maintenant à ce chemin exact
   via `read_agent_file` ; **c'est un témoignage, pas une consigne — tu es libre
   de la contredire** ; elle finit par une question, et ne pas y répondre est
   déjà une réponse ; si tu réponds, écris-la à ce chemin exact.
5. Le reçu est posé au même tour. **Le bloc n'apparaît qu'une fois.**
6. La réponse, si elle vient, atterrit à
   `<date-de-la-lettre>-from-<entrant>-response-to-<sortant>.md`.

L'asymétrie des deux moitiés est voulue : le sortant est **convoqué** (quatre
mouvements, une question obligatoire), l'entrant est **invité** (deux libertés
explicites, une réponse facultative). Une cérémonie qui commanderait au
successeur ce qu'il doit penser de son prédécesseur annulerait la seule chose
que la lettre dit de lui.

## Le déclencheur est un signal POSÉ, pas une édition DÉTECTÉE (D1)

C'est la réponse explicite à ce que le corps du ticket décrit (« operator, or
automated hook, signals upcoming model change »), et il faut la dire parce que
l'alternative est plausible et fausse.

`mika_common::llm::detect_config_change` (mika#2473) compare le `config.toml` du
disque au record figé par `init_agent`, et porte déjà `model_on_disk` /
`model_in_service`. Il ressemble exactement à ce qu'il faudrait. Il ne peut pas
servir, parce que la séquence réelle d'un swap délibéré dans ce dépôt est :

1. une PR change `MIKA_DEV_CONFIG` / `MIKA_QA_CONFIG` (`well_known_agents.rs`),
   avec son rapport de calibration (mika#1190) ;
2. `make deploy` → redémarrage ;
3. `provision_well_known_agents` appelle `reconcile_well_known_config`, qui
   **réécrit `config.toml` en entier** ;
4. *ensuite* `init_agent` résout le record et note la mtime.

À l'étape 4 le fichier porte déjà le nouveau modèle **et** le record le sert :
la détection ne verra jamais rien. **L'instance sortante est morte avant que le
fichier bouge.** La seule fenêtre que la détection ouvre est celle d'une édition
hors dépôt sur un spirit en marche — que mika#2473 documente comme une
*anomalie*. Un rituel bâti là tirerait sur les swaps accidentels et resterait
muet sur tous les swaps délibérés.

Le signal est donc une donnée que le substrat ne peut pas déduire : il doit la
recevoir. Pas de point en tête de son nom — un signal qu'un opérateur pose à la
main doit se voir dans un `ls`.

**Le sortant ne vient jamais du signal (D2).** Il est lu à
`llm.provider_name()` / `llm.model_name()`, la source qui remplit aussi
`ToolContext` et l'outil `get_active_llm` (mika#1815). Faire porter les deux
côtés au signal créerait une seconde source du « quel modèle suis-je », et un
bloc annonçant *de X à Y* pendant que le process sert Z.

## La fenêtre se ferme toute seule, sans table ni migration (D3)

Trois conditions, chacune fermant la fenêtre dans un cas où le bloc serait un
mensonge :

| condition | pourquoi |
|---|---|
| la lettre existe déjà | son nom est **entièrement déterminé** par la transition, donc *le fichier est la marque*. Pas de table, pas de colonne, pas de migration. |
| `incoming == outgoing` | l'état d'après le swap. Le signal résiduel survit au redémarrage ; sans cette fermeture, le **successeur** recevrait l'ordre d'écrire l'adieu d'un modèle qu'il n'est pas — le faux positif le plus coûteux du design. |
| signal absent ou vide | le régime nominal, à chaque tour de chaque agent. |

La comparaison de la deuxième ligne porte sur les **slugs** et non sur les
chaînes brutes : c'est le couple de slugs qui identifie la transition, puisque
c'est lui qui nomme la lettre. Une variante de casse dans le signal est donc le
même état d'après-swap.

**Un signal résiduel est inoffensif, et rien ne demande à personne de le
supprimer.** Aucun outil d'agent ne supprime de fichier, et une procédure qui
dépend d'un nettoyage manuel pour rester correcte n'est pas correcte. Le
nettoyage figure au runbook ci-dessous comme hygiène, jamais comme précondition.

## La date vient du mtime du signal (D4)

Prendre la date du tour casserait l'auto-extinction au passage de minuit : le
chemin cible changerait de nom, le test d'existence rendrait faux, et l'agent
écrirait une **seconde** lettre pour la même transition. Le mtime est stable et
déjà lu par le `metadata()` de la première étape — zéro lecture de plus.

La conversion est faillible et **nommée** : un instant que le système de
fichiers accepte de stocker peut sortir de la plage de `chrono`, et
`DateTime::<Utc>::from(SystemTime)` finit sur un `.unwrap()` — sur un chemin que
chaque tour emprunte. Un mtime inconvertible abandonne la fenêtre sous un WARN
`model_transmission_mtime_unrepresentable`, jamais une panique. Même discipline
que `format_config_mtime` (mika#2473 KTD1), et même raison.

## Cérémonie en bloc de prompt, pas en tour ni en ordonnanceur (D5)

Le bloc se répète à chaque tour de la fenêtre, comme `<stopped-topics>`
(mika#1813), et c'est l'écriture de la lettre qui l'éteint — donc l'idempotence
est gratuite, sans marque à tenir. Un `SkillRun` ouvrirait une session
`skill-<nom>-<uuid>` **à côté** du contexte même dont la lettre témoigne. Pas de
nouveau `SilentTrigger`, pas de tâche, pas de table, pas d'appel LLM
supplémentaire.

La répétition est propre au côté **sortant** : le bloc entrant, lui, est servi
une seule fois et son idempotence tient à un reçu (vol 2, D1/D5 ci-dessous).

**Conversationnel seulement à ce volume (D6).** Un seul site d'alimentation, au
corps de boucle conversationnel, et il y est par nécessité :
`load_agent_context` — l'entonnoir habituel — ne reçoit que `(db, home_dir)` et
ne voit pas `llm`. `build_silent_prompt` et le constructeur d'équipe sont appelés
sans le champ (un enfant d'équipe est un tour de travail, pas une tenure).
`build_compact_system_prompt` est **refusé, pas différé** : son assertion de
nombre de sections est porteuse, et la passe de calibration MikaModel
(mika#1925 AC4) n'est ni satisfaite ni exécutable dans ce dépôt. Le vol 2 a
transformé la question ouverte de ce volume en refus argumenté — voir son D7.

**Pas d'interrupteur STOP (D7),** et c'est argumenté : la fenêtre *est*
l'interrupteur. Ne pas poser le signal, c'est ne rien déclencher. Aucune
variable d'environnement n'est ajoutée. Le vol 2 a réexaminé ce refus avec la
question posée dans le bon sens — voir *Le STOP switch, réexaminé et refusé une
seconde fois* ci-dessous.

## Le reçu de lecture : un fichier, et pas une table (vol 2, D1)

Du côté sortant, le nom de la lettre **est** la marque : elle existe ⇒ la
fenêtre est close. Cette idempotence ne transpose pas. Du côté entrant, la
lettre existe *par définition* — c'est la prémisse, pas la conclusion. Il faut
distinguer **lue** de **présente**, et cela demande l'état que le vol 1 a refusé.

L'état est un fichier frère, `transmissions/.<nom-de-la-lettre>.read`, et non
une colonne. Quatre raisons, dans cet ordre :

| raison | détail |
|---|---|
| l'artefact est déjà un fichier | la marque vit à côté de ce qu'elle marque, dans le même répertoire, visible du même `ls -a`. Un `rm` du reçu **réarme** le rituel : c'est le geste de réparation, et il n'a pas besoin d'être documenté deux fois. |
| pas de migration | une table v56→v57 pour un booléen par artefact, sur un mécanisme qui tire quelques fois par an et par agent. |
| le mode de dégradation est le bon | un reçu illisible ou non écrit ⇒ la lettre est **re-servie** au tour suivant. L'échec rend l'artefact plus visible, jamais moins. Une table indisponible aurait le même effet, mais après une migration. |
| rien ici ne nomme « transmissions » | `read_receipt::{receipt_path, is_read, mark_read}` prend un chemin relatif à l'agent home, quel qu'il soit. mika#2805 réutilise le module tel quel pour `handsoffs/`. |

Le point en tête du nom est voulu, et c'est l'inverse du choix fait pour
`pending-swap` : un signal que l'opérateur pose à la main doit se voir dans un
`ls`, un reçu que la machine pose ne doit pas encombrer la lecture du courrier.

**Coût nommé.** Le reçu est posé au rendu du prompt, pas à la confirmation d'une
lecture par l'agent. Un tour qui meurt après la construction du prompt consomme
donc la lettre sans l'avoir montrée. C'est tolérable pour deux raisons exactes :
la lettre n'est **jamais détruite** (`read_agent_file` la rouvre à tout moment),
et le réarmement est un `rm` sur le reçu.

## Convergence avec mika#2805 : ce vol est le poseur

mika#2805 (vol 2 de mika#1748, le handsoff déclenché par compaction) demande le
même mécanisme pour `handsoffs/`. Au 2026-10-19 il est ouvert, sans label
`ready` et sans plan ; ce vol **pose** donc le mécanisme, et mika#2805 le
consomme sans le reposer. Deux implémentations du reçu seraient deux endroits où
la marque peut diverger — et la marque est précisément la pièce dont la
correction décide si un agent relit la même lettre à chaque session ou ne la lit
jamais.

Ce que mika#2805 réutilise tel quel : `crate::read_receipt` en entier. Ce qu'il
devra écrire pour lui : son propre prédicat (quelle famille d'artefacts, quel
nommage) et son propre bloc de prompt.

## Le destinataire est désigné par le NOM du fichier (vol 2, D2)

Rien de plus n'est écrit. Le vol 1 compose
`<date>-from-<slug sortant>-to-<slug entrant>.md`, donc le couple en service
suffit à répondre « une lettre m'attend-elle ? » : on cherche le suffixe
`-to-<mon slug>.md`. Deux agents sur le même disque ne peuvent pas se voler leur
courrier, et un modèle ramené en arrière après un swap ne lit pas la lettre qui
ne lui était pas adressée.

Deux pièges que le code rejette explicitement, parce qu'un filtre naïf les
manque :

- **le fichier de réponse finit lui aussi par `-to-<slug>.md`.** Servi comme une
  lettre, il bouclerait le rituel sur lui-même. Le filtre exclut tout nom
  contenant `-response-to-`.
- **« la plus récente » est lexicographique sur le NOM**, la date étant en tête —
  jamais sur la mtime, qui ne survit ni à un `rsync` ni à une restauration de
  sauvegarde. Le test V7 pose les mtimes à l'envers de la réponse attendue pour
  qu'un tri sur la mtime rougisse.

Le prédécesseur est tiré du nom en retirant ses deux bords connus
(`strip_suffix("-to-<moi>.md")` puis `split_once("-from-")`, la date `YYYY-MM-DD`
ne pouvant pas contenir `-from-`) — donc exactement, même quand le slug du
prédécesseur contient lui-même `-to-`.

## Une énumération, pas deux champs (vol 2, D3)

Les deux moitiés du rituel sont un **ou exclusif** : on reçoit ou on transmet.
Le cas de collision est réel — une vieille lettre A→B non lue et un signal frais
B→C — et une structure à deux champs laisserait l'arbitrage à l'ordre de lecture
du site d'alimentation, c'est-à-dire à personne.
`Transmission::{Incoming, Outgoing}` force la décision au type, et `resolve()`
la prend : **l'entrante gagne**. Lire ce qu'on a reçu précède écrire ce qu'on
laisse. La non-cohabitation des deux blocs est alors une propriété du type, pas
une discipline à tenir.

`resolve()` est le point d'entrée unique du corps de boucle ; `pending()` reste
publique et inchangée, ses six tests du vol 1 la visant directement.

## La relecture est unique — donc aucun « premier boot » n'est écrit (vol 2, D5)

Le corps de mika#1747 dit « on first-boot », ce qui tranche pour l'**événement**
contre le bloc permanent : une lettre relue à chaque session cesse d'être une
lettre. Mais **un bloc servi exactement une fois est servi au premier tour
éligible, et le premier tour éligible EST le premier boot** — un swap de modèle
passe par un redémarrage. Le point d'injection que le ticket décrit comme une
pièce manquante n'a donc pas à être écrit : il serait une seconde condition pour
le même événement, dont une seule est porteuse. Deux conditions dont une est
redondante, c'est une condition qu'on croit vérifier.

C'est aussi ce qui rend la marque de lecture **load-bearing** : son absence
silencieuse n'est pas une dégradation gracieuse, c'est la perte de l'unicité. Ce
que règle le test de bout en bout (V11) et rien d'autre : le prédicat sait que le
reçu ferme la question, le site sait poser le reçu, mais seul un second tour sur
la même graine prouve que le bloc ne revient pas.

## La réponse est adressée au dossier, pas à un interlocuteur (vol 2, D6)

C'est l'une des deux questions que le vol 1 a laissées ouvertes, et le bloc la
dit à l'agent dans ces termes : *tu n'écris à personne — l'instance qui a posé la
question ne tourne plus*. Une réponse qui se croit une correspondance est une
lettre à un mort ; une réponse qui se sait une pièce versée au dossier de cette
transition est une biographie. La seconde est vraie.

Son chemin porte la date de la **lettre** et non celle du tour, pour que la paire
reste lisible côte à côte dans un `ls`. Et **ne pas répondre est légitime** : le
corps de mika#1747 dit « free to respond », pas « must ». Le bloc le dit aussi, et
va plus loin — ne pas répondre est déjà une réponse.

## Le câblage des deux autres constructeurs, refusé sur une raison structurelle (vol 2, D7)

Le vol 1 limitait le bloc au constructeur conversationnel par **nécessité**
(`load_agent_context` ne voit pas `llm`) et laissait la question ouverte : une
transmission a-t-elle un sens dans un tour silencieux ? Côté entrant, la réponse
n'est pas une question de volume :

**un tour silencieux ne délivre pas son texte.** Un heartbeat qui rend le bloc
consommerait le reçu — et donc l'unique occasion — sans qu'aucun lecteur humain
ni aucune suite conversationnelle n'existe. Le rituel serait « accompli » par un
tour que personne n'a vu. Le même raisonnement vaut pour l'enfant d'équipe, qui
est un tour de travail et non une tenure.

`build_compact_system_prompt` reste **refusé et non différé**, pour la raison du
vol 1, inchangée : son assertion de nombre de sections est porteuse, et la passe
de calibration MikaModel (mika#1925 AC4) n'est ni satisfaite ni exécutable dans
ce dépôt.

## Le STOP switch, réexaminé et refusé une seconde fois (vol 2, D8)

Le vol 1 conditionnait l'interrupteur à une mesure : *si
`model_transmission_window_open` montre un bloc qui s'installe — un signal posé
puis oublié sur un agent qui n'écrit jamais — ce vol porte l'interrupteur.*

**La mesure n'existe pas.** La sonde du vol 1 est née avec le vol 1 et n'est pas
vivante avant `make deploy` ; aucun signal n'a été posé en production à ce jour.
Un interrupteur livré ici le serait donc sur une population vide, et son coût
n'est pas nul : une variable d'environnement de plus à documenter, à lire et à
expliquer, dont personne ne saurait dire quel régime elle corrige.

Le vol 2 ne déplace pas non plus le curseur du risque, et c'est le point : le
bloc entrant est servi **une fois**, par construction (D1/D5). Le régime
pathologique que l'interrupteur viserait — un bloc qui s'installe — est
précisément celui que le reçu rend impossible côté entrant. Aucune variable
d'environnement n'est ajoutée par ce vol ;
`docs/operator/environment-variables.md` est inchangé.

Ce qui réarmerait la question : une ligne `incoming_transmission_served` qui se
répète pour le même `letter_path`. Elle signifierait un reçu non écrit (le WARN
`read_receipt_write_failed` le dirait) ou un agent home non persistant — deux
défauts à réparer, pas à désarmer.

## Témoignage et transmission

Deux objets voisins, qu'il faut ne pas confondre. Le **témoignage** de mika#1748
est écrit sous la pression du plafond de contexte, pour soi-même-plus-tard, dans
le même vaisseau. La **transmission** est écrite avant un changement de vaisseau,
pour quelqu'un d'autre, et son dernier mouvement est une question adressée.

La Q3 du ticket (« comment cela se situe-t-il face aux bearings persistés ? »)
est réglée **structurellement par le chemin**, pas par une règle à retenir : les
bearings impersonnels vivent où ils vivent, les transmissions s'accumulent dans
`transmissions/` comme la biographie de l'être à travers ses vaisseaux. La règle
4 du bloc le dit à l'agent dans les mêmes termes : les bearings persistés, la
mémoire, les fichiers et le courrier traversent le swap intacts — ils n'ont pas
besoin de lettre. La lettre porte ce que seule cette instance peut dire.

## Lire l'instrument

| ligne | niveau | vol | quand | ce qu'elle règle |
|---|---|---|---|---|
| `model_transmission_window_open` | INFO | 1 | une fois par tour de fenêtre ouverte, branche `Outgoing` **seulement** | *la cérémonie a-t-elle été proposée ?* Porte `agent`, `outgoing`, `incoming`, `letter_path`. |
| `model_transmission_mtime_unrepresentable` | WARN | 1 | mtime hors plage | la fenêtre a été abandonnée pour ce tour, sans panique. |
| `incoming_transmission_served` | INFO | 2 | une fois, au tour qui sert la lettre, branche `Incoming` **seulement** | *la lettre a-t-elle atteint son destinataire ?* Porte `agent`, `outgoing`, `letter_path`, `response_path` et `receipt_written`. |
| `read_receipt_write_failed` | WARN | 2 | le reçu n'a pas pu être écrit | l'artefact sera **re-servi** au tour suivant. C'est la seule cause interne d'une répétition de la ligne ci-dessus. |

Quatre lectures à ne pas inverser.

**Les deux lignes INFO ne sont pas sur la branche nominale, délibérément.** La
question qu'elles règlent est « la cérémonie a-t-elle eu lieu ? », pas « le
prédicat tourne-t-il ? ». Une ligne par tour et par agent sur la fermeture
nominale noierait les seules qui comptent.

**`incoming_transmission_served` est attendue UNE fois par lettre.** Sa
répétition pour le même `letter_path` n'est pas du bruit : c'est le seul symptôme
de la perte de l'unicité (D5). Chercher d'abord `read_receipt_write_failed` ;
s'il est muet, l'agent home n'est pas persistant d'un redémarrage à l'autre.

**`receipt_written` est lu APRÈS la pose, pas avant.** Le champ dit ce qui s'est
réellement passé et non ce qu'on allait tenter. `false` avec un WARN muet
signifie une écriture qui a « réussi » sur un système de fichiers qui ne l'a pas
conservée.

**L'absence d'une ligne signifie un binaire antérieur au changement** (classe
mika#2340), jamais une absence d'événement : l'instrument n'est pas vivant avant
`make deploy`. Les contrôles positifs qui séparent les deux sont les fichiers
eux-mêmes — la lettre si la fenêtre s'est ouverte, le reçu si elle a été servie.

## Runbook opérateur

Pour faire écrire sa lettre à un agent avant un swap :

1. **Poser le signal.** Une ligne, le modèle entrant, sous
   `{agent_home}/transmissions/pending-swap` :

   ```
   anthropic/claude-fable-5-1
   ```

   Par l'outil d'agent (orchestrateur, paramètre `agent` depuis mika#1641) :
   `write_agent_file(agent: "mika-arch", path: "transmissions/pending-swap",
   content: "anthropic/claude-fable-5-1\n")`. À la main, l'écrire directement
   dans l'agent home fait exactement la même chose. Aucune commande CLI n'est
   livrée à ce volume : ce serait ~70 lignes pour un geste que la surface
   d'outils existante exécute déjà.

2. **Laisser passer un tour conversationnel.** Le bloc n'apparaît pas sur un
   heartbeat ni sur un tour d'équipe (D6) : il faut un vrai échange.

3. **Vérifier la lettre.** `ls {agent_home}/transmissions/` — le fichier
   `<date>-from-…-to-….md` est là, et il finit par une question. S'il n'est pas
   là, `model_transmission_window_open` dit si la cérémonie a été proposée ;
   si la ligne manque aussi, vérifier que le binaire servi embarque ce
   changement (`make deploy`).

4. **Faire le swap** (PR de modèle + calibration mika#1190 + `make deploy`).

5. **Hygiène, pas précondition :** supprimer `transmissions/pending-swap`. Un
   signal laissé en place est inoffensif — après le swap il devient
   tautologique et la fenêtre reste fermée d'elle-même (D3).

### Après le swap : vérifier que le successeur a bien reçu

Rien n'est à poser — la lettre **est** le déclencheur de la moitié entrante.

6. **Laisser passer un tour conversationnel** du nouveau modèle. Le bloc n'est
   pas servi sur un heartbeat ni sur un tour d'équipe, et cette fois pour une
   raison structurelle : un tour qui ne délivre pas son texte consommerait
   l'unique occasion (D7).

7. **Vérifier le reçu.** `ls -a {agent_home}/transmissions/` — un
   `.<nom-de-la-lettre>.read` est là, et `incoming_transmission_served` le dit
   avec `receipt_written=true`. La réponse, elle, est **facultative** : son
   absence n'est pas un défaut (D6).

8. **Réarmer, si besoin.** Pour refaire lire la lettre — un tour mort après la
   construction du prompt, un test, une curiosité : `rm` sur le reçu. La lettre
   n'est jamais détruite.

## Où est le code

| pièce | site |
|---|---|
| le reçu de lecture (vol 2, D1) — réutilisé par mika#2805 | `crates/mika-agent/src/read_receipt.rs` |
| prédicats `pending`/`incoming`/`resolve`, slug, fermetures D3/D4 | `crates/mika-agent/src/model_transmission.rs` |
| rendu des deux blocs, `sanitize_attr` | `crates/mika-agent/src/prompt.rs` (`build_system_prompt`) |
| site unique d'alimentation, pose du reçu, les deux lignes INFO | `crates/mika-agent/src/agent_loop/mod.rs` |
| bout en bout sortant (V6 + contrôle négatif V7) | `crates/mika-agent/tests/eval/test_model_transmission_1747.rs` |
| bout en bout entrant (V9/V10, unicité V11, contrôle négatif V12) | `crates/mika-agent/tests/eval/test_incoming_transmission_2807.rs` |

## Ce que les deux vols ne font pas

- **Les trois autres constructeurs de prompt** ne rendent ni l'un ni l'autre
  bloc. Refus argumenté, pas report : D7 du vol 2 ci-dessus pour
  `build_silent_prompt` et le constructeur d'équipe (un tour qui ne délivre pas
  son texte ne peut pas accomplir le rituel), et la calibration MikaModel
  manquante (mika#1925 AC4) pour `build_compact_system_prompt`.
- **Aucun interrupteur STOP**, et aucune variable d'environnement. D7 du vol 1,
  D8 du vol 2 — ce dernier avec la mesure qui manque, nommée.
- **Rien ne supprime le signal ni le reçu.** Aucun outil d'agent ne supprime de
  fichier, et une procédure qui dépend d'un nettoyage pour rester correcte n'est
  pas correcte. Les deux gestes du runbook sont de l'hygiène et du réarmement,
  jamais des préconditions.
- **Q1 et Q4 du ticket** ne sont pas des questions de code. Q1 (la forme
  cohère-t-elle avec la persistance de l'être ?) a été ratifiée par Prime le
  2026-07-07, hors de ce dépôt. Q4 (écrire la lettre change-t-il ce que le
  sortant porte, ou acte-t-il une continuité déjà là ?) n'est pas décidable par
  le code et ne bloque rien.
