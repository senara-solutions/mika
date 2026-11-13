# mika#2935 — le parquage appartient au tour, jamais au flush

- Ticket : senara-solutions/mika#2935 (`bug`, `p2-normal`, `dispatch:loop`)
- Branche : `fix/2935/failed-sends-un-flush-rat-re-parque-la`
- Lu sur `origin/main` @ `967e1f1f`
- Voisins **hors périmètre**, nommés pour qu'on ne les y ramène pas : mika#1751
  (le seuil de 5 minutes, qui reste tel quel), mika#1752 (la course flush / tour
  frais, dormante).

## Contexte

`failed_sends` est le parking des réponses que la passerelle n'a pas pu livrer.
Deux acteurs y écrivent aujourd'hui, et c'est un de trop.

Le premier est le **tour normal** : `GatewayMessageSender::send_with_severity`
(`crates/mika-agent/src/messaging.rs:362-372`), quand ses deux essais échouent,
appelle `self.db.save_failed_send(text, None)`. C'est juste : le tour ne possède
aucune ligne, il en crée une, et elle porte le texte nu.

Le second est le **flush** : `flush_failed_sends`
(`crates/mika-agent/src/server/handlers.rs:2390-2459`) lit jusqu'à 5 lignes, et
sur la branche `FlushAction::Deliver` envoie
`sender.send(&format!("{}{}", prefix, send.text))`. Or ce `send` est *le même*
que celui du tour normal — donc il parque. Le flush, lui, possède déjà la ligne :
il la supprime sur `Delivered` et incrémente son compteur sur `Failed`. Sur
`Failed` les deux gestes se cumulent :

| acteur | ce qu'il fait d'une ligne ratée |
|---|---|
| `flush_failed_sends` | `increment_failed_send_retry(send.id)` — **conserve** l'ancienne |
| `send_with_severity`, en dessous | `save_failed_send(prefix + text)` — **crée** une nouvelle |

Une ligne devient deux, et la nouvelle porte un préfixe de plus. Le processus est
géométrique tant que le canal reste en panne : 5 lignes par flush, chacune
dédoublée. Le seuil de 5 minutes de mika#1751 abandonne la plupart des copies —
ce qui est précisément pourquoi le défaut est resté lisible plutôt que bruyant —
mais celles qui restent sous le seuil sont **livrées**, et le lecteur reçoit
« ⏳ from earlier — ⏳ from earlier — ». Le ticket compte 36 lignes sur 108 avec
au moins deux préfixes, sur trois semaines de `/var/log/mika/server.log`.

### Ce que le défaut n'est pas

Ce n'est pas une course : rien ici ne dépend d'un entrelacement. C'est une
**composition**. Deux fonctions correctes séparément — « parque ce que tu n'as
pas pu livrer » et « retente ce qui est parqué » — composées de telle façon que
la seconde appelle la première. Le remède n'est donc ni un verrou ni une
déduplication, mais une **séparation de la tentative de livraison et de la
décision de parquer**.

## Le remède : une primitive sans effet de bord, et un propriétaire par ligne

La règle à rendre vraie : **le parquage appartient à celui qui ne possède pas de
ligne.** Un tour normal n'en possède aucune, il parque. Le flush en possède une,
il décide de son sort lui-même (`delete` ou `increment`) et n'en crée jamais.

Pour que cette règle soit tenue par le code et non par la mémoire du relecteur,
`GatewayMessageSender` acquiert deux entrées au lieu d'une :

- `try_deliver_with_severity(text, severity)` — **toute** la séquence actuelle
  (résolution du chat, routage de sévérité, court-circuit `NoChannel`, les deux
  POST) **moins** le `save_failed_send`. Aucun effet de bord sur la table.
- `send_with_severity(text, severity)` — appelle la primitive, puis parque si et
  seulement si l'issue est `Failed`. Reste la seule implémentation du trait
  `MessageSender`, donc **tous les sites existants sont inchangés**.

Il y a donc après ce plan **un seul site de parquage dans tout le crate**, et il
est dans la fonction dont le nom contient `send`.

### Pourquoi un trait distinct de `MessageSender`, et non `&dyn MessageSender`

Le flush doit devenir testable — AC1 l'exige — donc sa dépendance doit être
injectable. La tentation est de lui passer un `&dyn MessageSender` et de tester
avec un double. **C'est le piège exact à ne pas prendre** : un double de test
n'écrit pas dans `failed_sends`, le vrai `GatewayMessageSender` si. Le test
serait vert et la production conserverait le défaut, mot pour mot. Le trait
injecté doit donc être celui qui **ne peut pas** parquer :

```rust
/// Ce dont le flush a besoin, et rien de plus : livrer un texte et dire si
/// c'est passé. Volontairement PAS `MessageSender` — ce trait-là parque, et
/// c'est le défaut de mika#2935. Un double de `MessageSender` ne parquerait
/// pas, donc un test écrit contre lui resterait vert au-dessus du bug.
#[async_trait]
pub trait FlushDelivery: Send + Sync {
    async fn try_deliver(&self, text: &str) -> Result<SendOutcome>;
}
```

Une seule implémentation en production, `GatewayMessageSender`, déléguant à
`try_deliver_with_severity(text, P2)` — la sévérité que `sender.send()` résolvait
déjà sur ce chemin, donc le routage ne bouge pas.

### Une propriété que la correction préserve sans effort, et qu'il faut nommer

Après un flush raté, la ligne conservée garde son `created_at` **d'origine**.
C'est ce que mika#1751 veut : le seuil de 5 minutes se mesure depuis l'envoi
initial, pas depuis la dernière tentative. Le défaut actuel le violait aussi —
la copie re-parquée repartait avec un `created_at` neuf, donc une ligne en panne
depuis une heure pouvait se présenter comme fraîche. Ce plan ne corrige pas cela
par une ligne de code : il le corrige en cessant de créer la copie.

## Requirements

### R1 — `messaging.rs` : scinder la tentative et le parquage

`try_deliver_with_severity(&self, text, severity) -> Result<SendOutcome>` reçoit
le corps actuel de `send_with_severity` **privé de** la clause
`save_failed_send`. `send_with_severity` devient :

```rust
async fn send_with_severity(&self, text: &str, severity: NotificationSeverity)
    -> Result<SendOutcome>
{
    let outcome = self.try_deliver_with_severity(text, severity).await?;
    if let SendOutcome::Failed { .. } = &outcome {
        // Site UNIQUE de parquage du crate (mika#2935). Voir le détecteur R4.
        if let Err(db_err) = self.db.save_failed_send(text, None).await {
            warn!(error = %db_err, "failed to save to failed_sends table");
        }
    }
    Ok(outcome)
}
```

**Un détail de journalisation, parce qu'il mentirait sinon.** Le `warn!` actuel
du second échec dit `"retry failed, saving to failed_sends"`. Cette ligne vit
désormais dans la primitive, où le parquage n'a pas lieu. Elle devient
`"both /send attempts failed"` ; la mention du parquage descend d'un cran, dans
`send_with_severity`, sous l'événement nommé `failed_send_parked` portant
`text_len`. Sans cela un flush raté continuerait d'écrire « saving to
failed_sends » dans le log pour un parquage qui n'a pas eu lieu — et le prochain
diagnostic repartirait du même faux témoignage que celui-ci.

Le doc-comment de la struct (`messaging.rs:109-113`, « On failure, retries once
after 2s. If both attempts fail, saves to `failed_sends` ») décrit maintenant
`send`, pas le type : il est déplacé sur la méthode.

### R2 — le trait `FlushDelivery` et son unique implémentation

Dans `messaging.rs`, le trait cité plus haut, plus :

```rust
#[async_trait]
impl FlushDelivery for GatewayMessageSender {
    async fn try_deliver(&self, text: &str) -> Result<SendOutcome> {
        self.try_deliver_with_severity(text, NotificationSeverity::P2).await
    }
}
```

Pas de méthode inhérente du même nom : une méthode inhérente masquerait celle du
trait à la résolution, et deux `try_deliver` dont l'une est invisible est
exactement le genre d'ambiguïté qui remet un défaut de composition en place.

### R3 — `handlers.rs` : le flush devient injectable, et n'appelle plus `send`

`flush_failed_sends(state, agent_state)` garde sa signature — ses deux sites
d'appel (`handlers.rs:516` et `handlers.rs:1404`) ne bougent pas — et se réduit
à construire le sender puis déléguer :

```rust
async fn flush_failed_sends(state: &AppState, agent_state: &AgentState) {
    let sender = GatewayMessageSender::new(/* inchangé */);
    flush_failed_sends_with(&agent_state.db, &sender).await;
}

async fn flush_failed_sends_with<D: FlushDelivery + ?Sized>(
    db: &AsyncDatabase,
    delivery: &D,
) { /* la boucle actuelle, avec `delivery.try_deliver(&text_to_send)` */ }
```

Le `get_pending_failed_sends` et son court-circuit « table vide » restent dans la
fonction interne : la construction du sender remonte donc avant la lecture, ce
qui est gratuit (`new` est un constructeur de struct, aucune E/S).

Les quatre branches d'issue gardent leur comportement **mot pour mot** :
`Delivered` → `delete`, `Failed` → `increment`, `NoChannel` → `delete`, `Err` →
`increment`. Ce plan ne change aucune de ces décisions ; il retire seulement
l'écriture qui se produisait un cran plus bas.

**Et une ligne de log rendue lisible par la sonde.** `info!(id, "flushed failed
send")` ne porte aucun `text_preview`, alors qu'AC3 veut compter les préfixes sur
cet événement. La sonde y renverrait du vide, lisible à tort comme « zéro
violation ». Le site reçoit le même `text_preview` que la branche `Drop` : les 80
premiers caractères du texte **envoyé**, préfixe compris — c'est la grandeur
qu'AC3 mesure.

### R4 — le détecteur : `save_failed_send` n'a qu'un appelant de production

La correction est locale ; la classe ne l'est pas. Un futur site qui parque de
son côté — ou qui rebranche le flush sur `send` — réinstalle le défaut sans
rougir aucune assertion de comportement. Un scan de source dans `messaging.rs` :

- **aiguille** : `.save_failed_send(`
- **périmètre** : `crates/mika-agent/src/**/*.rs`, moitié production seulement
  (`crate::source_scan::production_half` et `is_test_source_path`, mika#2321)
- **propriétaire autorisé** : `src/messaging.rs`, le fichier qui porte le test
- **hors périmètre, et ce n'est pas une exception** : `src/db.rs` (la définition)
  et `src/async_db.rs` (le délégué vers le fil DB). Ces deux-là *sont* la méthode ;
  les compter reviendrait à déclarer le type en violation de lui-même.

Le prédicat est extrait en `fn is_unexpected_parker(path: &Path, src: &str) -> bool`
pour que le contrôle de bonne foi puisse le nourrir d'une source synthétique sans
écrire sur le disque. Le test porteur s'appelle
`mika2935_le_parquage_a_un_seul_site` ; il assène aussi `files.len() > 50` pour
qu'un parcours qui ne trouve plus rien soit rouge au lieu d'être vert, et son
message dit « retirez le second site, ne l'allowlistez pas ».

### R5 — les tests qui tiennent les critères d'acceptation

**Test A — `messaging.rs`, AC1 au niveau de la primitive et AC2 dans le même
test.** Nom : `mika2935_le_parquage_appartient_a_send_jamais_a_la_tentative_nue`.

`mika_common::dead_endpoint::DeadEndpoint::reserve()` pour le gateway — garde
tenue pour toute la durée du test, jamais un `bind(:0)` suivi d'un `drop`
(mika#2569 : ce binaire est le plus parallèle de l'arbre, et un port libéré est
repris par le voisin, qui répond alors à l'appel censé échouer). `TestHarness`
pour la base, `set_customer_config("chat_id", …)` pour que la résolution passe.

1. `FlushDelivery::try_deliver(&sender, "réponse parquée")` → `Failed`, et
   `get_pending_failed_sends(10)` rend **zéro** ligne. C'est AC1 à la racine :
   la primitive n'écrit pas.
2. Puis, même sender : `MessageSender::send(&sender, "un tour normal")` →
   `Failed`, et la table porte **exactement une** ligne, de texte `"un tour
   normal"`. C'est AC2, contrôle négatif dans le même test comme demandé.

La paire est ce qui distingue « le parquage a été déplacé » de « le parquage a
été supprimé » — un seul des deux appels suffirait à l'un, jamais aux deux.

**Test B — `handlers.rs`, AC1 au niveau du flush.** Nom :
`mika2935_un_flush_rate_laisse_exactement_une_ligne`. Un double
`FailingDelivery` qui rend `Failed` et **mémorise** le texte reçu, puis deux
appels consécutifs de `flush_failed_sends_with(&db, &double)` sur une table
pré-remplie d'une ligne de texte nu. Après chaque passe :

- la table porte **une** ligne — jamais deux, jamais trois ;
- son texte est le texte nu d'origine, **sans aucun préfixe** ;
- son `created_at` n'a pas bougé ;
- `retry_count` vaut 1 puis 2 ;
- le texte **reçu par le double** porte **exactement un** préfixe à chaque passe.

La seconde passe est ce qui ferme la propriété que le ticket mesure : « le nombre
de lignes grossit tant que le canal reste en panne ». Une passe unique ne
distingue pas une croissance arrêtée d'une croissance plus lente.

**Test C — le contrôle positif du flush.** Nom :
`mika2935_un_flush_reussi_supprime_la_ligne`. Même harnais, double qui rend
`Delivered` : la table est vide après la passe. Sans lui, un flush devenu
inopérant — qui ne livre rien et ne supprime rien — satisferait le test B à la
lettre.

## Non-Requirements (hors de ce plan, délibérément)

- **La course flush / tour frais** — mika#1752, dormante. Ce plan ne la réveille
  ni ne la referme : elle porte sur l'ordre de deux écrivains, pas sur leur
  nombre.
- **Le seuil de 5 minutes** — mika#1751, inchangé, y compris sa branche
  fail-open sur `created_at` illisible.
- **La journalisation côté passerelle** (texte, `request_id`) — hors périmètre
  déclaré par le ticket.
- **Le `request_id` du parquage.** `save_failed_send(text, None)` passe `None`
  depuis toujours alors que le sender porte `self.request_id`. C'est une perte de
  traçabilité réelle et **séparée** : la corriger changerait ce qui est écrit,
  quand ce plan change seulement qui écrit. Pas de ticket ouvert par ce plan ;
  noté ici pour le prochain lecteur de ce fichier.
- **Un test `tests/eval/`.** La propriété qu'AC1 demande — « la table porte
  exactement une ligne » — s'observe en lisant la table après un appel de
  fonction. Un `MockLlmProvider` et un tour complet n'y ajouteraient qu'un
  scénario de script, pour ~200 lignes.
- **Réduire le `sleep(2s)` entre les deux essais.** Les deux tests de R5 le
  payent deux fois chacun (≈ 4 s de suite). Le rendre configurable est un
  changement de comportement du transport pour un gain de confort de test.

## Verification Contract

- `cargo test -p mika-agent messaging` — vert, les tests de R5 et le détecteur de
  R4 inclus.
- `cargo test -p mika-agent failed_send` — vert : les quatre tests
  `classify_failed_send` de mika#1751 et les nouveaux du flush.
- `cargo test` — suite complète verte (~3463 tests).
- `cargo clippy --all-targets -- -D warnings` — propre.
- `cargo fmt --check` — propre.
- `scripts/verify-pipeline.sh origin/main` — vert.
- Lecture de bonne foi du détecteur R4 : poser une seconde ligne
  `.save_failed_send(` dans un fichier de production réel fait rougir
  `mika2935_le_parquage_a_un_seul_site`, avec un message qui nomme le fichier.

## Definition of Done

- `GatewayMessageSender` porte `try_deliver_with_severity`, sans aucun effet de
  bord sur `failed_sends`, et `send_with_severity` est le seul site du crate à
  appeler `save_failed_send`.
- `flush_failed_sends` passe par `FlushDelivery::try_deliver` ; aucun chemin du
  flush n'atteint plus `MessageSender::send`.
- Les quatre décisions de cycle de vie du flush (`delete` / `increment` ×
  quatre issues) sont inchangées, et le `created_at` d'une ligne conservée l'est
  aussi.
- `flushed failed send` porte un `text_preview`, donc la sonde d'AC3 peut le lire.
- Les trois tests de R5 passent, chacun avec son contrôle, et le détecteur de R4
  passe avec son contrôle de bonne foi.
- Le Verification Contract est vert en entier.
- La sonde post-déploiement est écrite dans ce plan, exécutable telle quelle.
- La PR porte `Closes #2935` et cite ce plan.

## Acceptance criteria

Transcrits du corps du ticket. Les tests qui les tiennent sont R5.

- **AC1.** Un flush raté laisse **exactement une** ligne pour l'envoi d'origine :
  il n'ajoute aucune ligne, et le texte stocké ne porte jamais le préfixe deux
  fois. Un test le démontre sur un sender qui échoue.
- **AC2.** Le contrôle négatif est dans le même test : un envoi raté **hors
  flush** parque toujours une ligne. Le parquage d'un tour normal ne doit pas
  régresser.
- **AC3, la sonde post-déploiement**, écrite dans le plan : après `make deploy`,
  aucune ligne `dropping stale failed_send` ou `flushed failed send` postérieure
  au déploiement ne porte un `text_preview` avec 2 préfixes ou plus. Le plan
  donne la commande, celle qui a produit le tableau du ticket.

## Fire-Disposition

Ce plan livre des détecteurs : un scan de source (R4) et trois tests dont le
chemin de succès est « aucune violation trouvée » (R5). La section est due.

**Option retenue : (a) exception nommée en allowlist, livrée VIDE.**

```rust
/// Fichiers dispensés du détecteur de parquage unique. LIVRÉE VIDE, et c'est
/// le contrat. `(chemin, ticket de suivi)`.
#[cfg(test)]
const PARKING_SITE_ALLOWLIST: &[(&str, &str)] = &[];
```

Elle est vide parce qu'**aucune violation ne subsiste à l'issue de ce plan** :
`handlers.rs` est le seul second parqueur du dépôt et R3 le retire dans le même
commit que le détecteur qui le surveille. Les deux fichiers de la couche DB
(`db.rs`, `async_db.rs`) ne sont pas des entrées d'allowlist mais une **exclusion
de périmètre** — ils portent la définition et son délégué, pas une décision de
parquer. La distinction compte : une exclusion de périmètre décrit ce que le
détecteur mesure, une exception décrit une dette qu'il tolère.

L'assertion auto-nettoyante, telle que mika#2309 l'exige — *l'exception expire
par le geste qui la rend inutile* :

```rust
#[test]
fn mika2935_lallowlist_du_parqueur_unique_est_livree_vide_et_le_reste() {
    assert!(
        PARKING_SITE_ALLOWLIST.is_empty(),
        "Une entrée a été ajoutée ici. Un second site de parquage rouvre \
         exactement mika#2935 : la ligne ratée est conservée par son \
         propriétaire ET recréée par le transport, avec un préfixe de plus à \
         chaque passe. Route le site par `send_with_severity`, ou ouvre un \
         ticket de suivi et nomme-le dans l'entrée."
    );
}
```

Et le contrôle de bonne foi, parce qu'une allowlist vide au-dessus d'un scan qui
ne voit rien est verte pour la mauvaise raison :
`mika2935_le_detecteur_rougit_sur_un_second_parqueur` nourrit
`is_unexpected_parker` d'un chemin de production réel et d'une source synthétique
portant l'aiguille, et exige `true`. Un second cas lui donne la même source sous
un chemin de test, et exige `false`.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/messaging.rs` — scission R1 (signature, doc, nouveau corps de `send_with_severity`, journalisation) | 25 |
| `crates/mika-agent/src/messaging.rs` — trait `FlushDelivery` + impl (R2) | 22 |
| `crates/mika-agent/src/messaging.rs` — prédicat `is_unexpected_parker` (R4) | 22 |
| `crates/mika-agent/src/server/handlers.rs` — coquille + `flush_failed_sends_with` + `text_preview` (R3) | 30 |
| tests `messaging.rs` — test A (AC1 primitive + AC2) | 45 |
| tests `messaging.rs` — détecteur R4 : allowlist, walk, scan, bonne foi | 60 |
| tests `handlers.rs` — tests B et C + le double de livraison | 70 |

Total estimé : 274 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient — avec une marge mince, nommée comme
telle.** 274 passe sous la borne par vol (279) de 5 lignes, et très loin sous le
seuil par PR (1000). La marge est étroite, donc elle a été obtenue par des refus
argumentés et non par un arrondi optimiste :

- **Aucun test `tests/eval/`** (§ Non-Requirements) : les trois voisins du
  répertoire pèsent 196, 290 et 305 lignes et n'attesteraient que leur propre
  script. La propriété d'AC1 est une ligne de table, lue après un appel.
- **Aucune migration, aucune DDL, aucune variable d'environnement.** Le schéma
  de `failed_sends` est intact ; le défaut est dans qui écrit, pas dans ce qui
  est écrit.
- **Aucun site d'appel déplacé.** `MessageSender` ne change pas, donc ses ~16
  sites de construction et tous ses appelants sont hors diff. C'est ce qui rend
  ce vol petit : la correction est une scission interne plus un trait de cinq
  lignes.
- **Le livrable le plus gros (60 lignes) est le détecteur R4**, dont ~15 lignes
  sont des helpers de parcours déjà dupliqués trois fois ailleurs dans le crate
  (`sink_dir.rs`, `liveness.rs`) et recopiés à l'identique.

Un découpage en deux vols aurait séparé la scission de son détecteur, ou le flush
de ses tests : dans les deux cas un vol non mergeable seul.

## Ordre de commit

Chaque étape laisse l'arbre compilable et la suite verte.

1. **R1 + R2** — la primitive et le trait. `send_with_severity` conserve son
   comportement exact, donc toute la suite reste verte sans qu'une seule ligne
   d'appelant bouge. Le défaut est encore là : personne n'utilise la primitive.
2. **Test A** — écrit maintenant, parce qu'il porte sur R1/R2 seuls et doit être
   vert avant que le flush change. C'est lui qui prouve que l'étape 1 n'a pas
   déplacé le parquage par accident.
3. **R3** — le flush passe par `try_deliver`. **C'est le commit qui corrige le
   défaut**, et le seul dont le comportement observable change.
4. **Tests B et C** — le flush est injectable depuis l'étape 3, donc testable.
5. **R4 + sa Fire-Disposition** — en dernier : le détecteur atterrit sur un arbre
   où `handlers.rs` ne parque plus, donc armé et vert, allowlist vide.

L'ordre 3-avant-5 est volontaire. Poser le détecteur avant la correction le
ferait atterrir rouge, ce qui obligerait soit à une entrée d'allowlist à retirer
au commit suivant, soit à un commit intermédiaire cassé.

## Sonde post-déploiement (AC3)

À exécuter **après `make deploy`**, sur l'hôte. Le préfixe compté est
`⏳ from earlier — `.

```sh
# 1. L'instant du déploiement, pour borner la lecture.
date -u +%Y-%m-%dT%H:%M:%SZ

# 2. Distribution des préfixes sur les abandons — la commande du ticket,
#    celle qui a produit son tableau.
grep '"dropping stale failed_send"' /var/log/mika/server.log \
  | jq -r '.text_preview | [scan("⏳ from earlier — ")] | length' | sort -n | uniq -c

# 3. La même, sur les flushs réussis (lisible seulement depuis ce déploiement).
grep '"flushed failed send"' /var/log/mika/server.log \
  | jq -r '.text_preview | [scan("⏳ from earlier — ")] | length' | sort -n | uniq -c
```

**Lecture — et les deux pièges qui font qu'une sonde verte ne prouve rien.**

- **Le journal n'est pas tronqué au déploiement.** Les lignes du tableau du
  ticket (28 à deux préfixes, 8 à trois) y sont encore, et resteront. La sonde
  n'est concluante que sur les lignes **postérieures** à l'instant du point 1 :
  borner avec `jq 'select(.timestamp > "<instant>")'` avant le comptage, ou
  lire sur une rotation neuve. Une distribution lue sur tout le fichier contient
  le passé et se lit à tort comme un échec.
- **Sur le point 3, l'absence de `text_preview` est l'absence du champ, jamais
  l'absence de violation.** `flushed failed send` ne porte ce champ qu'à partir
  de ce déploiement (R3) ; les binaires sont lus au build, donc un `jq` muet là
  signifie *binaire antérieur au correctif* (motif mika#2340) et non *zéro
  préfixe en trop*. Le contrôle : la sortie du point 3 doit comporter au moins
  une ligne `0` dès le premier flush réussi. Si elle est vide alors que le point
  2 produit des lignes, le binaire servi n'est pas celui qui vient d'être
  construit.

**Attendu sur les lignes postérieures au déploiement** : une seule classe, `0`
pour les parquages d'un tour normal et `1` pour les lignes passées par un flush.
**Toute ligne à 2 ou plus est une violation d'AC3** : elle signifie qu'un second
parqueur existe — et le détecteur de R4 dira lequel.

Le défaut est géométrique sous panne de canal, donc il faut une panne pour que la
sonde soit informative. Elle doit être lue après un épisode d'indisponibilité de
la passerelle ; en l'absence d'épisode, le point 2 ne produit aucune ligne neuve
et la sonde est **muette, pas verte**.
