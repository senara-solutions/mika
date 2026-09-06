---
module: mika-common
tags: [logging, tracing-subscriber, telemetry, double-write, observability, openrc, rt005, structural-guard]
problem_type: measurement-channel-corruption
issues: [2195, 2209, 2179]
date: 2026-09-06
---

# Deux sinks visant un seul fichier : le producteur ne doit pas dépendre de son
# lanceur pour ne pas se dédoubler

## Le symptôme, mesuré

Chaque événement du log spirit était écrit **deux fois** dans
`/var/log/mika/server.log`. Uniformément, depuis toujours, sans un seul message
d'erreur.

La propriété n'était pas propre à un type d'événement — elle était structurelle à
la composition des layers, ce qui la rendait invisible par la voie habituelle
(« ce compteur-là est bizarre ») :

| Événement | Occurrences réelles | Lues dans le log |
|---|---|---|
| `mika-spirit listening` | 1 par restart (le serveur bind son port une fois) | 2 |
| `domain_rebuild_complete` | 1 par boot | 4 |
| `turn_usage` | 1 par appel LLM | 2 |

Les dégâts sont ceux d'un thermomètre faux de ×2, donc des dégâts en aval :

- **RT-005** : le contrôle intra-design −7,43 devient −3,71 une fois dédoublonné.
  Le double-logging expliquait le **facteur 2**, ni l'existence ni le signe de
  l'effet.
- **mika#2179** : les « 38 échecs de livraison » d'une nuit étaient **19**
  événements réels.
- **mika#2209** : un fantôme de « double-init » du serveur, entièrement fabriqué
  par le canal de mesure — le serveur ne s'initialisait qu'une fois.
- Covariables dégénérées : `turns:2.0` pour un tour unique, `handshakes:0.0` sur
  tout un batch.

## La cause racine

`mika_common::logging::init`, branche `(Some(path), LogFormat::Json)`, composait
**deux** layers `fmt` JSON :

```rust
tracing_subscriber::registry()
    .with(otel_layer).with(filter)
    .with(fmt::layer().json().flatten_event(true))                            // (A) → STDOUT
    .with(fmt::layer().json().flatten_event(true).with_writer(non_blocking))  // (B) → FICHIER
    .init();
```

Pris isolément, ce code est correct : (A) va sur stdout, (B) va dans le fichier,
deux destinations distinctes. **Il ne devient faux qu'une fois lancé**, et
l'unité OpenRC de production fait exactement cela :

```
supervise-daemon mika-spirit --stdout /var/log/mika/server.log \
                             --stderr /var/log/mika/server.log
```

Le lanceur redirige stdout dans le fichier même que (B) écrit déjà. Les deux
destinations n'en sont plus qu'une, et chaque ligne est écrite deux fois.

## La leçon, généralisable

**Un producteur ne doit pas dépendre de la configuration de son lanceur pour ne
pas se dédoubler.** La correction évidente — retirer la redirection `--stdout`
côté OpenRC/mika-cloud — a été rejetée au grooming pour cette raison précise :
elle aurait laissé le défaut latent, prêt à se rouvrir sous n'importe quel autre
lanceur (systemd, docker, un futur init-script) qui redirige stdout vers le
fichier de log. Le correctif appartient au point qui compose les sinks, parce que
c'est le seul endroit qui connaît *tous* les sinks.

Corollaire pratique : **le nombre de sinks d'un processus n'est pas une propriété
du code seul.** Deux `with_writer` distincts peuvent être une seule destination.
La question à se poser en revue n'est pas « ces deux layers écrivent-ils au même
endroit ? » mais « existe-t-il une configuration de lancement plausible où ils
écrivent au même endroit ? ».

Second corollaire, sur la classe de défaut : un canal de mesure faux d'un
**facteur uniforme** est le plus difficile à repérer, parce qu'il préserve tous
les rapports, tous les classements et tous les signes. Il ne se trahit que sur un
événement dont on connaît indépendamment le compte réel — ici « le serveur bind
son port une fois par restart ». **Garder un événement singleton connu dans le
canal est ce qui rend un canal de comptage auditable.**

## Le correctif (Option A, tranchée mika#2195)

Une décision unique, lue par les **deux** bras JSON de `init`, plutôt qu'un bras
modifié à la main :

```rust
fn json_stdout_layer_enabled(log_file_configured: bool) -> bool {
    !log_file_configured
}
```

```rust
.with(json_stdout_layer_enabled(true).then(|| fmt::layer().json().flatten_event(true)))
```

`Option<L>` implémente `Layer`, ce qui permet de piloter au *runtime* une
composition par ailleurs type-level — c'est ce qui évite de dupliquer la décision
dans un prédicat parallèle qui dériverait (la classe de défaut nommée par
mika#2158 sur les regex de grooming).

Périmètre volontairement étroit : seul le layer **JSON** de stdout est retiré, et
seulement quand un fichier est configuré. `LogFormat::Pretty` garde sa console —
c'est le format de développement local, où la sortie console *est* l'objectif et
où aucune redirection de lanceur n'est en jeu.

## Le prix, énoncé

Avec `MIKA_SPIRIT_LOG_FILE` défini, `docker logs` et `supervise-daemon --stdout`
ne portent plus de lignes applicatives : il faut lire le fichier. C'est le coût
assumé de l'Option A. Les conteneurs Docker du dépôt ne définissent pas de
fichier de log (vérifié : `Dockerfile.agent`, `Dockerfile.gateway`,
`docker-compose.yml`) et tombent donc dans la branche « stdout seul », inchangée.

## Les tests, et pourquoi ils ont cette forme

Un test unitaire ne peut pas reproduire la redirection OpenRC. Il peut reproduire
sa **conséquence** : brancher les deux layers sur un `MakeWriter` partagé —
littéralement « stdout et le fichier sont la même destination » — émettre un
événement, et compter les lignes.

- `mika2195_one_event_is_one_line_when_a_log_file_is_configured` (AC1/AC2/AC3) —
  compte = 1, vérifié sur les trois noms d'événements de l'incident. Contrôle de
  non-vacuité effectué : en remettant le prédicat à son comportement d'avant, il
  échoue avec `left: 2`, exactement le doublon mesuré.
- `mika2195_stdout_still_logs_when_no_log_file_is_configured` (AC4) — le fix
  retire un doublon, jamais la sortie ; sans fichier configuré stdout reste le
  seul sink et doit porter l'événement.
- `mika2195_json_file_arm_gates_stdout_and_keeps_one_file_writer` — garde
  structurelle sur le source du bras. Les deux tests runtime exercent le
  *prédicat* ; ils ne verraient pas une réintroduction d'un layer stdout non
  gardé **à côté** de lui. La garde, oui.

## Sonde post-déploiement (halt-and-surface)

```bash
grep -c 'mika-spirit listening' "$MIKA_SPIRIT_LOG_FILE"   # sur une fenêtre d'un restart → 1
```

**2 signifie que le fix n'a pas pris.** Ne pas redéployer à l'aveugle : c'est le
seul événement dont le compte réel est connu indépendamment, donc le seul qui
puisse arbitrer.

## Conséquence pour les données antérieures

Tout comptage fait sur des lignes écrites **avant** ce déploiement est doublé.
La correction est une déduplication, rien d'autre : le défaut n'a jamais touché
les *valeurs* portées par un événement, seulement le nombre de fois qu'il
apparaît. Conformément à la note de mika#2195, **aucune réécriture rétroactive**
des valeurs RT-005 : −7,4 reste la valeur étiquetée du batch, accompagnée de son
diagnostic, sans estimation concurrente.
