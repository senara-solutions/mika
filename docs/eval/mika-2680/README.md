# Mesure 3 — ce que coûtent les tours webhook, après le vol 1 (mika#2680)

Ce répertoire porte le **protocole** de la troisième mesure du § *Critère
d'acceptation de la mesure* de mika#2680, et accueillera son **artefact**. Le
vol 2 livre l'instrument (`scripts/measure-webhook-turn-cost`) et ce protocole ;
**le vol 3 exécute la mesure, écrit l'artefact, puis implémente ce qu'elle
confirme.** Aucune classe de pré-filtre — ni (e), ni (f), ni (g), ni (h) — n'est
implémentée par le vol 2 : chiffrer d'abord, décider ensuite.

## 1. Pourquoi une troisième mesure

Les deux premières (commentaires du 2026-10-08 et du 2026-10-14) ont chiffré par
`llm_calls` × premier message `user` de chaque session, en rattachant à la main
chaque session à son événement déclencheur. Deux limites, et c'est le vol 1 qui
les lève :

- **Le rattachement était humain.** Rien dans la donnée ne disait « ce tour-ci
  vient d'un `check_suite` sur une branche de PR » ; il fallait lire le premier
  message et décider. Depuis le vol 1, `webhook_turn_charged` porte `trigger`,
  `text_touched`, `candidate` et `prefilter_enabled` **à l'instant du tour**.
- **La classe (b) n'avait jamais été exercée.** La mesure 2 le dit en toutes
  lettres : aucune `check_suite` de branche de PR depuis le déploiement de
  `e14ace7f`. Un chiffrage de (e)+(f)+(g) pris là aurait compté comme à filtrer
  ce que (b) écarte déjà.

## 2. La fenêtre

**Au moins 4 h 30 de données servies**, pour être comparable à la fenêtre de
référence (2026-10-07 16:00Z–20:30Z).

Le registre est servi depuis le **2026-10-16 13:01Z** (merge de #2705 en
`185362cc`). La borne basse de toute mesure est donc `2026-10-16T13:01:00Z` —
**jamais avant**, et l'instrument le dit lui-même : quand `--since` précède
`field_epoch`, le seau des `turn_usage` sans ligne payée change de nom (§5).

Une fenêtre qui ne contient **aucune activité PR** ne vaut rien pour ce
chiffrage : (g) porte sur les `check_suite` de branche de PR, (f) sur les
`pr_review`. C'est précisément ce qui a invalidé la mesure 2.

## 3. La commande

```bash
scripts/measure-webhook-turn-cost \
    --log-file "$MIKA_SPIRIT_LOG_FILE" \
    --since 2026-10-16T13:01:00Z \
    --until <fin de fenêtre, ISO-8601 UTC> \
    --out    docs/eval/mika-2680/turns-<fenêtre>.jsonl \
    --summary docs/eval/mika-2680/summary-<fenêtre>.json
```

Le corpus est le **journal**, jamais la base : la moitié « payé » du registre
n'écrit aucune ligne d'audit (choix délibéré du vol 1), et `turn_usage` est
dégaté là où `llm_calls` dépend de `MIKA_STORE_LLM_CALLS`. Sans `--agent`, les
deux agents sont dans le même tableau — ce qu'il faut, puisqu'un `check_suite`
vert coûte un tour **aux deux**.

Les deux fichiers produits sont l'artefact du vol 3 ; ils se committent ici.

## 4. Lire les deux moitiés

L'agrégat porte deux populations qui ne se soustraient pas l'une de l'autre :

| moitié | champ | ce qu'elle dit |
|---|---|---|
| payée | `by_agent_trigger` | les tours qui ont **eu lieu**, par agent × déclencheur, avec leur coût |
| écartée | `skipped_by_class` | les tours que le pré-filtre a **évités**, par classe |

**Ce que le registre décide.** La moitié payée chiffre chaque classe candidate
par son `trigger` : (e) est `pr_closed`, (g) est `check_suite_pr_branch` côté
`mika-qa`, (h) est `issue_comment` et `issues_closed`. (f) se lit sur
`text_touched: true` conjugué à un `trigger` de revue. Ces chiffres-là sont des
mesures.

**Ce que le registre ne fait que majorer.** Un tour payé n'est pas un tour
*inutile*. `by_agent_trigger` donne le **plafond** de ce qu'une classe pourrait
économiser, jamais l'économie elle-même : décider qu'un `issue_comment` sans
tâche de boucle ne mérite pas un tour complet est un jugement sur ce que le tour
fait, et la question est posée telle quelle dans le commentaire du 2026-10-08.
L'instrument ne la tranche pas.

**Ce qu'`unattributed*` n'est pas.** Les `turn_usage` sans ligne payée de même
`trace_id` ne sont **pas** des tours webhook non comptés : ce sont, pour
l'essentiel, les tours Telegram et CLI, qui ne passent pas par le registre. Leur
seau existe pour que leur coût soit **visible et hors tableau**, pas pour être
ajouté.

## 5. La halte — deux raisons de refuser de conclure

Un chiffre pris dans l'une de ces deux conditions ressemble à un résultat et n'en
est pas. Les deux sont **dans l'agrégat**, pas dans une note :

1. **`window_precedes_field_epoch: true`.** La borne basse précède l'époque du
   champ (ou il n'y a pas de borne basse : la fenêtre est alors ouverte vers le
   passé). Dans cette zone, « pas de ligne payée » ne distingue pas un tour
   Telegram d'un tour servi par un binaire antérieur au registre. Le seau est
   alors rendu sous le nom `unattributed_undetermined` : **le seau change de nom,
   pas de place** — un lecteur qui cherche `unattributed` ne doit pas trouver un
   chiffre qui ne veut pas dire ce qu'il croit. *Remède : remonter `--since` à
   `field_epoch` ou après.*
2. **`prefilter_enabled: false` sur des tours de la fenêtre.** Le pré-filtre
   était désarmé : ces tours-là auraient peut-être été écartés par (a)–(d), et
   les compter comme candidats à (e)/(f)/(g) attribuerait à une classe nouvelle
   ce qu'une classe existante couvrait déjà. *Remède : restreindre la fenêtre à
   la période armée, ou chiffrer les deux populations séparément.*

`field_epoch` est levé sur le **corpus entier**, avant `--since`, `--until` et
`--agent` : c'est une propriété du binaire déployé, pas de la fenêtre qu'on lui
demande. Levé après les filtres, il vaudrait toujours au moins `--since` et le
drapeau ne saurait dire que « oui ».

## 6. Les deux codes de sortie

| code | sens |
|---|---|
| `0` | le corpus a été **lu**. Zéro tour payé est alors un **résultat**. |
| `2` | **rien n'a été mesuré** : pas de fichier, illisible, `jq` absent, borne mal formée, aucune ligne `webhook_turn_charged`, `--agent` sans appariement. |

La distinction est la leçon mika#2205 : un scan inerte en silence se lit
exactement comme un scan qui n'a rien trouvé. `unparseable_lines` est compté et
rapporté, mais **ne conjugue pas** la décision « rien mesuré » — une seule ligne
tronquée, quasi certaine sur un journal de 19 Go, rendrait sinon `exit 2`
inatteignable pour toujours.

## 7. Ce que le vol 3 fait de l'artefact

1. Chiffrer (e), (f), (g) et (h) sur la fenêtre, **après** retrait de ce que
   `skipped_by_class` montre déjà écarté par (a)–(d).
2. Pour chaque classe, répondre à la question du § *Ce que le registre ne fait
   que majorer* — que fait le tour, et ce travail survit-il autrement ?
3. N'implémenter que les classes dont la réponse est « rien, ou rien qui
   nécessite un tour LLM », une classe par phase dans `webhook_prefilter.rs`.

Une classe dont le chiffre est significatif mais dont la question reste ouverte
ne s'implémente pas : elle se pose à l'opérateur.

## 8. L'instrument lui-même

- `scripts/measure-webhook-turn-cost` — le lecteur. `--help` imprime son en-tête.
- `scripts/test-measure-webhook-turn-cost.sh` — son harnais : un contrôle positif
  (l'unité est le tour, la valeur est une **somme** sur toutes les lignes
  `turn_usage` de la trace, continuation `step = u32::MAX` comprise) et six
  contrôles négatifs, chacun sur sa propre entrée. `make
  test-measure-webhook-turn-cost`. **L'étape CI qui appelle cette cible reste à
  poser** : le jeton de dispatch n'a pas le scope `workflow`, donc la branche du
  vol 2 ne peut pas porter `.github/workflows/ci.yml` — l'étape et son
  emplacement exact sont dans le corps de la PR du vol 2.
- `crates/mika-agent/tests/fixtures/mika2680/` — trois corpus gelés et
  synthétiques. Ils ne sont pas échantillonnés sur le journal vivant : une suite
  qui le lirait changerait de verdict à mesure qu'il grossit.
