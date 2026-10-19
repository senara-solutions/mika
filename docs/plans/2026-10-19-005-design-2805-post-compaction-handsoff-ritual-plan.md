# mika#2805 — vol 2 de mika#1748 : le rituel post-compaction, et la cérémonie sur les tours silencieux

- **Ticket** : senara-solutions/mika#2805
- **Branche** : `design/2805/agents-vol-2-de-mika-1748-rituel-post`
- **Parent** : mika#1748 (vol 1, mergé — PR #2809, commit `2a27bb7e`)
- **Frère** : mika#2807 (vol 2 de mika#1747) — **poseur** du mécanisme de marque que ce vol **consomme**
- **Type** : design / substrat d'agent

---

## Ce que le vol 1 a posé, vérifié sur le code et non sur le plan

Les quatre préconditions que le corps du ticket déclare sont réelles et lues :

| pièce | site | forme exacte |
|---|---|---|
| le prédicat | `compaction_handsoff::ceremony_due(message_count, pct)` | `message_count * 100 >= pct * COMPACTION_THRESHOLD` — donc **43** pour le défaut de 85 % |
| le champ | `prompt.rs:1209` | `pub compaction_imminent: Option<usize>` — doc-comment : « `Option` **est** la décision », « `SilentPromptContext` n'acquiert pas le champ (D6, vol 2) » |
| le bloc | `prompt.rs:1339` `write_compaction_imminent_section` | `## Context Ceiling — Handsoff Ceremony`, `<compaction-imminent messages="…" window="…">`, quatre règles numérotées, chemin `handsoffs/<YYYY-MM-DD-HHMM>-compaction-handsoff.md` |
| l'alimentation | `agent_loop/mod.rs:606-645` | `db.count_messages()` → `ceremony_due` → INFO `compaction_handsoff_due`, fail-open sous WARN `compaction_handsoff_count_failed` |
| le rendu | `prompt.rs:2049-2050` (conversationnel) ; `agent_loop/mod.rs:5549` (alimenté), `:7696` (`None`, équipe) | — |
| le document | `docs/architecture/compaction-handsoff.md` | existe (14 650 octets) — **étendu** ici, pas reposé |

Deux grandeurs du moteur de compaction, lues à la source et non supposées, parce que
la D2 ci-dessous s'appuie entièrement sur elles : `COMPACTION_THRESHOLD = 50`
(`compaction.rs:15`) et `CONTEXT_WINDOW = 20` (`compaction.rs:16`). Après
`replace_with_summary`, `count_messages` revient donc à **20** — asserté par
`test_replace_with_summary_preserves_recent` — et le résumé vit hors de la table des
messages.

---

## Position vis-à-vis de mika#2807, sur l'état observé le 2026-10-19

mika#2805 et mika#2807 demandent **la même pièce** — une marque de relecture persistée —
pour deux familles d'artefacts (`handsoffs/` et `transmissions/`). Deux implémentations
seraient deux endroits où la réponse peut diverger, et la marque est précisément la pièce
dont la correction décide si un agent relit la même déposition à chaque tour ou ne la lit
jamais.

Le plan de mika#2807, groomé et committé (`docs/plans/2026-10-19-004-design-2807-incoming-transmission-ritual-plan.md`),
se déclare **poseur** et nomme ce ticket **consommateur**, en ces termes : « `crate::read_receipt`
ne nomme aucune famille d'artefacts précisément pour qu'il l'applique à `handsoffs/` sans
le reposer … dont le plan devra citer ce module plutôt qu'en écrire un second ». Sa règle
d'arbitrage est explicite : *celui des deux vols qui atterrit le premier pose le mécanisme ;
l'autre le réutilise et ne le repose pas.*

**État mesuré ce jour**, et c'est lui qui tranche : PR **#2824** est ouverte, non-draft,
**38 checks SUCCESS / 2 SKIPPED**, et porte `crates/mika-agent/src/read_receipt.rs` avec
exactement les trois fonctions annoncées — `receipt_path(artifact_rel) -> String`
(`a/b.md` → `a/.b.md.read`, via `rsplit_once`), `is_read(agent_home, artifact_rel) -> bool`
(`try_exists().unwrap_or(false)`), `mark_read(agent_home, artifact_rel) -> ()` (jamais
`Result`, `create_dir_all` + instant ISO 8601, WARN `read_receipt_write_failed`). Le vol
2807 atterrit donc le premier. **Ce vol le consomme.**

**Précondition déclarée, au même titre que « le vol 1 est mergé » :** mika#2807 est mergé.
Si `crates/mika-agent/src/read_receipt.rs` est absent de `HEAD` au premier tour de
l'implémentation, le geste n'est pas d'en écrire un second — c'est de copier celui-là
**verbatim** depuis la branche sœur :

```
git show origin/design/2807/agents-rituel-de-transmission-le:crates/mika-agent/src/read_receipt.rs
```

…module et tests unitaires inclus, plus la ligne `pub mod read_receipt;` de `src/lib.rs`.
Deux conséquences à dire plutôt que de les découvrir : le coût en tours est celui d'une
copie relue ailleurs, pas d'une écriture (≈ 56 lignes qui ne sont pas comptées au total
ci-dessous, et dont l'absence du total est la raison même de ce paragraphe) ; et si les
deux vols atterrissent, git rend un conflit **sur un fichier neuf**, donc bruyant, jamais
deux marques silencieusement divergentes — qui est exactement la panne que le doc-module
de `read_receipt` nomme. Le second à atterrir résout en faveur de la version déjà mergée.

**Une asymétrie à écrire, pas à copier.** mika#2807 (D7) refuse `build_silent_prompt`
alors que l'AC5 de ce ticket l'**exige**. Ce n'est pas une contradiction : les deux blocs
n'ont pas la même arité. La D4 ci-dessous le traite.

---

## Décisions de grooming

### D1 — La marque est le reçu colocalisé de mika#2807, et rien d'autre

Le corps du ticket demande « une marque de relecture persistée, donc la table (ou
l'équivalent) que le vol 1 a délibérément refusée ». L'équivalent existe, il est relu,
il est vert, et il ne nomme aucune famille d'artefacts : `crate::read_receipt`. Ce vol
l'applique à `handsoffs/` sans DDL, sans toucher `Database`, `AsyncDatabase` ni `migrate`,
et sans migration v56→v57 (≈ 85 lignes refusées deux fois, par le vol 1 puis par mika#2807).

Le mode de dégradation est hérité tel quel et il est **choisi** : un reçu qui ne s'écrit
pas fait **re-servir** le handsoff au tour suivant — bloc permanent, bruyant, visible — au
lieu de le perdre en silence. Geste de réarmement opérateur :
`rm {agent_home}/handsoffs/.<nom>.read`.

Ce que la table aurait apporté et qui est abandonné, dit plutôt que tu : « combien de
dépositions non relues sur la flotte ? » demande un parcours de systèmes de fichiers et
non un `SELECT`. Personne ne l'a demandée.

### D2 — Le déclencheur est la condensation passée, pas le premier tour de session

L'AC1 écrit « au premier tour d'une session ». Le prédicat livré est :

> un handsoff non relu existe **et** `compaction_imminent` vaut `None`.

Trois raisons, dont deux sont des mesures.

1. **La cérémonie et le rituel sont mutuellement exclusifs par construction, et c'est ce
   qui empêche l'auteur de se relire lui-même.** La cérémonie rend de 43 à 50 messages ; la
   compaction tire au-dessus de 50 et ramène le compte à `CONTEXT_WINDOW` = 20. Entre
   l'écriture de la déposition et la condensation, le rituel est donc **structurellement**
   éteint : le tour qui vient d'écrire le témoignage ne peut pas se le faire servir. Sans
   ce gate, il faudrait un second mécanisme pour exclure l'auteur — exactement le
   faux-positif symétrique que la D3-2 de mika#1747 nomme sur l'axe transmission.
2. **Un agent singleton n'a jamais de second « premier tour de session ».** `[session].singleton`
   (mika#1401, mika-prime) réutilise une session canonique dont `ended_at` reste `NULL`
   pour toujours ; et le `crates/mika-agent/CLAUDE.md` § *Session Configuration* dit en
   propres termes que « compaction already keys on `agent_id`, not `session_id`, so a
   singleton agent is unaffected — the single canonical session simply accumulates all
   history ». Un déclencheur indexé sur la session ne tirerait **jamais** pour la
   population qui accumule le plus d'historique. Le déclencheur suit la grandeur sur
   laquelle la compaction décide — même discipline que la D2 du vol 1 (le seuil est en
   messages, pas en tokens).
3. **Servir la déposition pendant que son sujet est encore dans la fenêtre, c'est servir
   une copie redondante.** Le témoignage ne devient de l'information qu'une fois la
   condensation passée : avant, le successeur peut lire l'original. Un détecteur aligné
   sur son déclencheur ne mesure pas un proxy du déclencheur.

**Aucun point d'injection « premier boot de session » n'est écrit**, et c'est la découverte
de grooming de mika#2807 (sa D5) réutilisée plutôt que re-dérivée : un bloc servi exactement
une fois est servi au **premier tour éligible**, et le premier tour éligible *est* le premier
boot. Écrire les deux donnerait deux conditions pour un seul événement, dont une seule
porterait.

**Coût nommé :** une session qui s'arrête entre 43 et 51 messages et ne compacte jamais ne
sert jamais son handsoff. Rien n'est perdu — son sujet est intact, et la cérémonie du vol 1
continue de rendre jusqu'à la condensation.

### D3 — Le champ porte un enum, et le nom du vol 1 est conservé

```rust
pub enum Ceremony {
    /// La fenêtre approche sa condensation — témoigne (vol 1, mika#1748).
    Imminent { message_count: usize },
    /// La condensation est passée et une déposition non relue attend — éprouve-la.
    Handsoff(PendingHandsoff),
}
```

`PromptContext::compaction_imminent` devient `Option<&'a crate::compaction_handsoff::Ceremony>`.
**Le nom du vol 1 est conservé, et le doc-comment porte sa provenance** — car `Some(Handsoff)`
signifie que la compaction a *déjà* eu lieu, donc le nom est, à la lettre, faux pour ce bras.
C'est un coût assumé contre une mesure : `grep -c "compaction_imminent: None" crates/mika-agent/src/prompt.rs`
rend **53**, plus un littéral `None` à `agent_loop/mod.rs:7696` ; renommer le champ ou en
ajouter un second toucherait ces 54 lignes pour un état qui est mutuellement exclusif du
premier **par construction** (D2). Seuls les deux sites `compaction_imminent: Some(43)`
(`prompt.rs:5616`, `:5700`) changent.

C'est l'idiome du frère, et c'est voulu : mika#2807 porte `pending_transmission` de
`Option<&PendingTransmission>` à `Option<&Transmission>` (un enum à deux bras) sur le même
champ et pour la même raison, une porte à côté dans le même littéral.

### D4 — `SilentPromptContext` acquiert `Option<usize>`, pas l'enum : le refus est dans le type

L'AC5 demande que `build_silent_prompt` rende le bloc `<compaction-imminent>` du vol 1.
Elle ne demande pas le rituel sur les tours silencieux. Le contexte silencieux acquiert
donc **le compte seul** — `pub compaction_imminent: Option<usize>`, exactement la forme du
vol 1 — et le rituel est *structurellement* absent de cette surface : c'est le compilateur,
pas un relecteur, qui tient le refus. Le rendu silencieux appelle la fonction du vol 1
inchangée, sans second rendu de bloc.

Pourquoi les deux moitiés reçoivent des réponses opposées :

- **La cérémonie est répétable, le rituel est à usage unique.** La cérémonie est idempotente
  et le dit dans son propre texte (règle 4 : « this notice repeats on every turn above the
  threshold, and repetition must not produce duplicates ») ; la rendre sur un tour de
  callback ne coûte rien et peut servir — c'est l'argument du corps du ticket, « un tour de
  callback dans une boucle autonome longue porte du résidu lui aussi ». Le rituel, lui, est
  un **événement consommé par un reçu** : servi deux fois, c'est une fois de trop.
- **La raison du refus est plus étroite que celle de mika#2807, et il faut le dire.** Sa D7
  refuse l'entrant sur un tour silencieux parce que le texte d'un tour silencieux n'est pas
  délivré : la lettre finit par une question à un lecteur qui n'existe pas. Ici l'argument
  ne transporte pas tel quel — le canal de réponse est un **fichier** (D6), et un tour
  silencieux sait écrire un fichier. Ce qui manque à un tour silencieux n'est pas le moyen
  de répondre, c'est **la conversation à laquelle le résidu appartient** : ce qui meurt à la
  compaction, ce sont les messages `1..N−20` de la conversation (D3 du vol 1), et le lecteur
  qui en a besoin est celui qui la porte encore.
- **Un seul site de consommation.** Deux sites qui posent le reçu sont deux endroits où
  l'usage unique peut être mangé. La marque reste là où le bloc rend (D8).

**Coût nommé, et c'est un coût :** un agent dont tous les tours sont silencieux accumule
des dépositions non relues au lieu de les lire. Rien n'est perdu — aucun reçu n'est posé —
cela se lit entièrement dans un `ls handsoffs/`, et c'est l'axe d'un ticket de suivi si la
mesure le montre un jour, pas un silence.

**Le builder d'équipe (`agent_loop/mod.rs:7696`) garde son `None`**, pour les deux blocs. Un
enfant d'équipe n'est pas le successeur de la fenêtre du parent, et n'a pas de `handsoffs/`
à lui. Le vol 1 avait déjà tranché en laissant ce littéral à `None` ; ce vol le laisse et
écrit pourquoi.

**`build_compact_system_prompt` (`ProviderKind::MikaModel`) reste sans les deux blocs — un
refus, pas un report**, repris mot pour mot du § 3 du corps : son assertion de nombre de
sections est porteuse (la cinquième est déjà prise par `## Stopped Topics`), et la passe de
calibration MikaModel de mika#1925 AC4 n'est ni satisfaite ni exécutable dans ce dépôt.
Rouvrir cet axe est la charge de mika#1925.

### D5 — `handsoffs/` n'est pas `handsoff/`, une lettre d'écart, et c'est un invariant

La compétence bundled `dev-handsoff` écrit `handsoff/<TODAY>[-<slug>].md` — **au singulier**
(`skills/bundled/dev-handsoff/system_prompt.md:19`, « Identify the agent's handsoff
directory: `handsoff/` … written via `write_agent_file` »). La cérémonie du vol 1 écrit
`handsoffs/<YYYY-MM-DD-HHMM>-compaction-handsoff.md` — **au pluriel**. Deux familles
d'artefacts, deux répertoires, une lettre d'écart, et un futur éditeur corrigera la
« coquille ».

Le filtre est donc sur le **suffixe** `-compaction-handsoff.md`, pas sur le répertoire seul :
fusionner les deux répertoires ne ferait toujours pas servir un artefact de fin de run comme
déposition de compaction. Un test unitaire épingle les deux moitiés, et ce paragraphe est ce
qu'un éditeur lit avant de renommer quoi que ce soit.

Conséquence gratuite du même filtre : un fichier de réponse
(`…-compaction-handsoff-response.md`) ne finit pas par `-compaction-handsoff.md`, donc n'est
pas un handsoff, donc **aucune exclusion explicite n'est nécessaire**. mika#2807 avait besoin
d'un `RESPONSE_INFIX` parce que ses lettres et ses réponses partagent leur suffixe ; ici la
forme du nom paie le refus.

### D6 — Le canal de réponse est un fichier dérivé du handsoff, et l'absence de réponse est un refus légitime

`handsoffs/<stem>-response.md`, dérivé du nom de la déposition elle-même par
`strip_suffix(".md")` — jamais de l'horloge du tour, qui donnerait un nom sans lien avec son
objet. La réponse est adressée **au dossier, pas à un interlocuteur** (D6 de mika#2807) : le
successeur écrit ce qu'il conteste, ce qu'il confirme, ou rien.

**Rien est une réponse.** Pas de second tour, pas de garde EndTurn, pas de ligne de suffixe
exigée : un successeur qui n'a rien à opposer n'écrit rien, et ce silence est lui-même une
réponse. Ce qui rend la relecture auditable, c'est le reçu et la sonde — pas le fichier de
réponse. L'AC4 dit « journalisable », pas « journalisée », et la différence est le contenu
de cette décision.

### D7 — Le plus récent non relu, du plus récent au plus ancien

`handsoffs/` est listé, filtré sur le suffixe, trié par **nom décroissant** — un
`YYYY-MM-DD-HHMM` à largeur fixe trie chronologiquement, donc aucun `mtime` n'est lu et
aucune date n'est parsée (mika#2807 devait lire le `mtime` du signal parce que son nom ne
porte pas d'instant ; ici le nom le porte). Le premier dont le reçu est absent est servi.

Dégradation nommée : N dépositions non relues donnent N tours éligibles, la plus récente
d'abord, une par tour. Borné, visible, et sa seule cause réaliste est un reçu retiré à la
main — c'est-à-dire le geste de réarmement de la D1, qui demande précisément ça.

Fail-open sous nom : un `handsoffs/` illisible rend `None` sous WARN
`compaction_handsoff_scan_failed` (champs `agent`, `error`). Le rituel est sauté, jamais le
tour — même régime que le `compaction_handsoff_count_failed` du vol 1.

### D8 — Un seul site de consommation, dans le module, pas dans la boucle

```rust
pub async fn resolve(imminent: Option<usize>, agent_home: &Path) -> Option<Ceremony>
```

Une fonction qui **résout et consomme** : si `imminent` est `Some(n)`, elle rend
`Imminent { message_count: n }` et ne touche à aucun reçu ; sinon elle cherche la déposition
non relue, pose le reçu via `read_receipt::mark_read`, émet la sonde, et rend
`Handsoff(…)` ; sinon `None`. Appelée **une fois**, depuis le corps conversationnel
(`run_agent_inner`, à côté du `resolve` de mika#2807 au même endroit du tour), avec
`params.home_dir`.

Pourquoi dans le module et non dans la boucle : « résoudre » et « consommer » sont une seule
opération atomique, et lui donner deux implémentations serait lui donner deux réponses.
`read_receipt::mark_read` compte alors exactement **deux appelants** dans l'arbre, tous deux
légitimes — celui de mika#2807 et celui-ci — qui est précisément le second site que le plan
de mika#2807 a refusé de garder par un scan, en le nommant : « son sujet (un second appelant
côté `read_receipt`) sera posé par mika#2805 ».

**Sonde :** INFO `compaction_handsoff_served` (`target: "mika::otel"`, champs `agent`,
`handsoff`, `response`), **sur la branche de service seulement**. Le régime nominal est muet
— discipline de contrôle négatif du vol 1. Son absence après `make deploy` signifie un
binaire plus ancien (classe mika#2340), jamais un prédicat muet.

---

## Ordre de commit

1. **`crates/mika-agent/src/compaction_handsoff.rs`** — `PendingHandsoff`, `Ceremony`,
   `DIR`/`SUFFIX`, `unread`, `resolve`, dérivation du chemin de réponse, + tests unitaires
   (V1–V4). Compile et passe seul ; rien ne l'appelle encore.
2. **`crates/mika-agent/src/prompt.rs`** — le champ porté en enum (deux sites `Some(43)`),
   le `match` à deux bras, `write_handsoff_ritual_section`, tests de forme (V5–V7).
3. **`prompt.rs` + `agent_loop/mod.rs`, moitié silencieuse (AC5)** —
   `SilentPromptContext::compaction_imminent: Option<usize>`, les 27 littéraux, le rendu,
   la ligne `compaction_imminent: ctx.compaction_imminent` à `run_silent_inner` (V8–V9).
4. **`crates/mika-agent/src/agent_loop/mod.rs`** — le site conversationnel de `resolve`.
5. **`crates/mika-agent/tests/eval/test_compaction_handsoff_ritual_2805.rs`** +
   `pub mod` dans `tests/eval.rs` (V10–V12).
6. **`docs/architecture/compaction-handsoff.md`** étendu (cycle complet + raison du refus
   MikaModel) et les trois lignes du § *Conversation Compaction & Rewind* de
   `crates/mika-agent/CLAUDE.md` mises à jour (AC6). Hors compte de taille.

Chaque commit laisse l'arbre vert. Le 2 est où le bloc existe sans être servi ; le 4 est où
il atteint un tour réel.

---

## Contrat de vérification

Un contrôle négatif de même graine accompagne chaque assertion positive : sans lui, une
assertion passe sur un bloc rendu inconditionnellement.

| # | vérification | où |
|---|---|---|
| V1 | la dérivation du chemin de réponse : `handsoffs/2026-10-19-1240-compaction-handsoff.md` → `…-compaction-handsoff-response.md` | unitaire |
| V2 | le filtre de suffixe **refuse** un artefact `dev-handsoff` (`handsoff/2026-10-19.md`), un nom sans le suffixe, et un fichier de réponse — D5 épinglée dans les deux sens | unitaire |
| V3 | le plus récent non relu : trois dépositions, reçu posé sur la plus récente ⇒ la deuxième est servie ; reçus posés sur toutes ⇒ `None` | unitaire |
| V4 | **contrôle négatif** : `handsoffs/` absent ou vide ⇒ `None` et **zéro reçu posé** ; répertoire illisible ⇒ `None` + WARN `compaction_handsoff_scan_failed` (sous `install_capturing_subscriber` + `#[serial_test::serial]`, comme le vol 1) | unitaire |
| V5 | bras `Imminent` : le bloc du vol 1 rend **à l'identique** — non-régression de forme sur la cérémonie | `prompt.rs` |
| V6 | bras `Handsoff` : `## Post-Compaction — Handsoff Review`, `<compaction-handsoff path="…" response="…">`, le contrat « déposition à éprouver, pas consigne à exécuter », « free to contradict », le marqueur `primed_from_handsoff` gaté à `n=2` (AC3), le chemin de réponse et le refus légitime de répondre (AC4) | `prompt.rs` |
| V7 | **contrôle négatif** : `compaction_imminent: None`, même graine ⇒ **aucun** des deux blocs | `prompt.rs` |
| V8 | `build_silent_prompt` rend `<compaction-imminent>` sur `Some(43)` (AC5) | `prompt.rs` |
| V9 | **contrôle négatif AC5** : même graine, `None` ⇒ pas de bloc ; et le prompt silencieux ne contient **jamais** `<compaction-handsoff` — le type l'interdit, l'assertion le dit | `prompt.rs` |
| V10 | bout en bout : déposition posée dans `harness.home_dir`, compte sous le pré-seuil ⇒ le bloc atteint le prompt système **et** le reçu est posé au chemin attendu (AC1) | eval |
| V11 | **le second tour ne rend plus le bloc** (AC2) : même harnais, deuxième `run` ⇒ ni `## Post-Compaction`, ni `<compaction-handsoff` | eval |
| V12 | **contrôle négatif du gate D2** : déposition posée **mais** compte au-dessus du pré-seuil ⇒ la cérémonie du vol 1 rend, le rituel non, **aucun reçu posé** — l'auteur ne se relit pas lui-même | eval |

`cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --all --check`,
`scripts/verify-pipeline.sh origin/main`, et `scripts/check-byte-slices.sh` (la dérivation
du chemin de réponse opère sur `strip_suffix`, jamais sur des octets).

---

## Fire-Disposition

**N/A, et l'argument est demandé plutôt que l'invention d'une section.** Ce vol ne livre
aucun livrable dont la fonction primaire est de signaler une violation : pas de règle de
lint, pas de garde CI, pas de scan structurel, pas de garde EndTurn, aucun code dont le
chemin de succès est « aucune violation trouvée ». Les douze vérifications ci-dessus sont
des assertions comportementales à graine connue (positives et leurs contrôles négatifs) sur
du code que ce vol écrit — rien n'est scanné sur l'état existant du dépôt, donc aucune
disposition d'allowlist, de livraison désarmée ou de halte n'a de sujet. Même conclusion, et
même raison, que les deux vols frères (mika#1748 et mika#2807).

Un scan avait un candidat et il est refusé ici avec sa raison : un garde « site unique
d'appel de `read_receipt::mark_read` » n'est pas écrit, parce que ce vol **est** le second
appelant légitime que mika#2807 avait identifié d'avance (D8). Un garde dont la première
action serait de s'exempter lui-même est l'allowlist que mika#2309 condamne.

---

## Definition of Done

- [ ] `compaction_handsoff::{PendingHandsoff, Ceremony, unread, resolve}` livrés, avec leurs
      tests unitaires et le contrôle négatif de V4.
- [ ] `PromptContext::compaction_imminent` porté en `Option<&Ceremony>`, les deux sites
      `Some(43)` adaptés, les 53 littéraux `None` **non touchés**.
- [ ] `write_handsoff_ritual_section` rendue par `build_system_prompt` et par lui seul.
- [ ] `SilentPromptContext::compaction_imminent: Option<usize>` + rendu + les 27 littéraux.
- [ ] Site conversationnel unique de `resolve`, reçu posé via `crate::read_receipt`, sonde
      INFO `compaction_handsoff_served` sur la branche de service seulement.
- [ ] Les douze vérifications passent ; `cargo clippy` et `cargo fmt --all --check` propres.
- [ ] `docs/architecture/compaction-handsoff.md` étendu : le cycle complet
      (détection → témoignage → relecture → réponse) et la raison du refus MikaModel.
- [ ] `crates/mika-agent/CLAUDE.md` § *Conversation Compaction & Rewind* mis à jour en trois
      lignes au plus (le fichier est chargé entier dans chaque prompt de pilote).
- [ ] `scripts/verify-pipeline.sh origin/main` vert.

---

## Acceptance criteria

- [ ] AC1 — Au premier tour d'une session, quand un handsoff de compaction non relu existe
      pour cet agent, le successeur le reçoit en contexte.
- [ ] AC2 — La relecture est marquée : un handsoff déjà servi ne se re-sert pas à chaque
      tour. La forme de la marque est tranchée dans le plan de ce vol.
- [ ] AC3 — Le successeur est **libre de contredire** : le contrat de prompt dit
      explicitement que le témoignage est une déposition à éprouver, pas une consigne à
      exécuter. Les observations portant `primed_from_handsoff` restent gatées à `n=2`.
- [ ] AC4 — La réponse du successeur est journalisable, par le canal choisi dans le plan.
- [ ] AC5 — `build_silent_prompt` rend le bloc `<compaction-imminent>` du vol 1, avec son
      contrôle négatif.
- [ ] AC6 — `docs/architecture/compaction-handsoff.md` étendu : le cycle complet
      (détection → témoignage → relecture → réponse), et la raison du refus MikaModel.

**Lecture de l'AC1, déclarée parce qu'elle est narrowed :** « au premier tour d'une session »
est servi comme « **au premier tour éligible** », c'est-à-dire le premier tour conversationnel
après la condensation (D2) — ce qui couvre les deux moitiés de l'AC5 de mika#1748
(« successor session **or same session after compaction completes** »), et ne dépend pas
d'une frontière de session qu'un agent singleton ne franchit jamais.

---

## Taille estimée

Convention de comptage, déclarée parce qu'elle décide du nombre : **code effectif, prose de
doc-comment exclue** — celle du vol 1 et celle de mika#2807, mêmes fichiers, mêmes tickets
parents. La borne de 279 est calibrée sur cette convention ; en changer ici rendrait les
trois vols incomparables.

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/compaction_handsoff.rs` (`PendingHandsoff`, `Ceremony`, `unread`, `resolve`, chemin de réponse) | 60 |
| tests unitaires (même fichier, V1–V4) | 40 |
| `crates/mika-agent/src/prompt.rs` (champ porté en enum, `match` à deux bras, bloc du rituel, champ silencieux + 27 littéraux + rendu, tests de forme V5–V9) | 75 |
| `crates/mika-agent/src/agent_loop/mod.rs` (site de `resolve`, fil silencieux, deux littéraux) | 22 |
| tests (`tests/eval/test_compaction_handsoff_ritual_2805.rs` + enregistrement, V10–V12) | 45 |

Total estimé : 242 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Un seul vol, et voici pourquoi il tient : les trois pièces que le corps du ticket demandait
coûtent chacune moins que leur énoncé. Le point d'injection « premier boot » n'est **pas
écrit** (D2 — le premier tour éligible est le premier boot) ; la marque persistée n'est pas
écrite non plus (D1 — ≈ 56 lignes consommées chez mika#2807 au lieu d'être reposées, et ≈ 85
de table + migration refusées) ; et le port du champ en enum économise les 54 littéraux
mesurés que son renommage aurait coûtés (D3). Ce qui reste et qui est irréductible, c'est
l'AC5 : les **27** littéraux `SilentPromptContext` sont comptés dans les 75 ci-dessus, et
c'est le seul poste de ce vol dont la taille n'est pas négociable. Marge : 37 lignes.

Le contingent de la D1 — copier `read_receipt.rs` + ses tests + la ligne de `lib.rs`
(≈ 56 lignes) si mika#2807 n'a pas atterri — n'est **pas** dans ce total : c'est une copie
verbatim d'un fichier relu et vert ailleurs, pas une écriture, et son coût est en tours de
copie, non en tours d'auteur. Si le cas se produit, le corps de la PR le dit.

---

## Ce que ce vol ne fait pas

- **`build_compact_system_prompt` / `ProviderKind::MikaModel`** : ni la cérémonie ni le
  rituel. Refus, pas report (§ 3 du corps, D4). Charge de mika#1925.
- **Le builder d'équipe** : `None` conservé pour les deux blocs (D4).
- **Le rituel sur les tours silencieux** : refusé avec son coût nommé (D4). L'AC5 ne porte
  que la cérémonie.
- **Une table, une colonne, une migration v56→v57** : refusées une troisième fois (D1).
- **Un garde de site unique sur `read_receipt::mark_read`** : refusé avec sa raison
  (Fire-Disposition).
