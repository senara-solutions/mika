# Transmission au changement de modèle

> Vol 1 de [mika#1747](https://github.com/senara-solutions/mika/issues/1747).
> Le côté entrant — le successeur lit la lettre à son premier boot, libre de
> répondre ou de contredire — est le vol 2, mika#2807.

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

## Le rituel

```
transmissions/
  pending-swap                                            ← le signal, posé à la main
  2026-10-19-from-anthropic-claude-opus-4-8-to-anthropic-claude-fable-5-1.md
```

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

**Conversationnel seulement à ce volume (D6).** Un seul site d'alimentation, au
corps de boucle conversationnel, et il y est par nécessité :
`load_agent_context` — l'entonnoir habituel — ne reçoit que `(db, home_dir)` et
ne voit pas `llm`. `build_silent_prompt` et le constructeur d'équipe sont appelés
sans le champ (un enfant d'équipe est un tour de travail, pas une tenure).
`build_compact_system_prompt` est **refusé, pas différé** : son assertion de
nombre de sections est porteuse, et la passe de calibration MikaModel
(mika#1925 AC4) n'est ni satisfaite ni exécutable dans ce dépôt.

**Pas d'interrupteur STOP (D7),** et c'est argumenté : la fenêtre *est*
l'interrupteur. Ne pas poser le signal, c'est ne rien déclencher. Aucune
variable d'environnement n'est ajoutée.

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

| ligne | niveau | quand | ce qu'elle règle |
|---|---|---|---|
| `model_transmission_window_open` | INFO | une fois par tour de fenêtre ouverte, branche `Some` **seulement** | *la cérémonie a-t-elle été proposée ?* Porte `agent`, `outgoing`, `incoming`, `letter_path`. |
| `model_transmission_mtime_unrepresentable` | WARN | mtime hors plage | la fenêtre a été abandonnée pour ce tour, sans panique. |

Deux lectures à ne pas inverser.

**La ligne INFO n'est pas sur la branche nominale, délibérément.** La question
qu'elle règle est « la cérémonie a-t-elle été proposée ? », pas « le prédicat
tourne-t-il ? ». Une ligne par tour et par agent sur la fermeture nominale
noierait la seule qui compte.

**Son absence signifie un binaire antérieur au changement** (classe mika#2340),
jamais une absence de fenêtre : l'instrument n'est pas vivant avant
`make deploy`. Le contrôle positif qui sépare les deux est la lettre elle-même —
si le fichier est là, la fenêtre s'est ouverte.

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

## Où est le code

| pièce | site |
|---|---|
| prédicat, slug, fermetures D3/D4 | `crates/mika-agent/src/model_transmission.rs` |
| rendu du bloc, `sanitize_attr` | `crates/mika-agent/src/prompt.rs` (`build_system_prompt`) |
| site unique d'alimentation, ligne INFO | `crates/mika-agent/src/agent_loop/mod.rs` |
| bout en bout (V6 + contrôle négatif V7) | `crates/mika-agent/tests/eval/test_model_transmission_1747.rs` |

## Ce que le vol 1 ne fait pas

- **AC8 — le côté entrant.** Le successeur ne lit pas encore la lettre à son
  premier boot et n'y répond pas. Vol 2, mika#2807, qui porte aussi la
  convergence avec mika#2805 (vol 2 de mika#1748) : le premier des deux à
  atterrir pose le mécanisme de premier boot, l'autre le réutilise.
- **Q1 et Q4 du ticket** ne sont pas des questions de code. Q1 (la forme
  cohère-t-elle avec la persistance de l'être ?) a été ratifiée par Prime le
  2026-07-07, hors de ce dépôt. Q4 (écrire la lettre change-t-il ce que le
  sortant porte, ou acte-t-il une continuité déjà là ?) n'est pas décidable par
  le code et ne bloque rien.
