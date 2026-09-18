# mika#2292 — le tenant possédait la réponse et n'avait pas le nom : doctrine Matérielle aliasée, butée Spirituelle sans énumération

> Ticket : `senara-solutions/mika#2292`
> Type : feat (section de prompt code-managed + élargissement de portée d'une discipline existante)
> Date : 2026-09-18

---

## Contexte

Mesure du 11/09 (canary Al, tenant cloud). Question : « Qu'est-ce que la doctrine
Mika ? ». Réponse : « rien trouvé qui s'appelle doctrine Mika », **puis** la
philosophie — dans le même tour.

Le ticket nomme le défaut exactement : « alors qu'il POSSÉDAIT la réponse ». Ce
n'est pas un trou de connaissance, c'est un **trou de nom**. Et la forme de la
réponse — incertitude à t=0, assertion à t=1 — est précisément celle que la
règle 4 de `## Self-Identity Discipline` condamne déjà mot pour mot
(`prompt.rs:1060` : *« Uncertainty at t=0 and confidence at t=1 within the same
response is confabulation »*). Elle n'a pas mordu parce que la portée écrite de
cette section est *« which model you are, which provider powers you, WHERE you
run »* — exactement l'argument qui a fondé mika#2290 sur un autre axe : la règle
était déjà écrite pour ce cas et ne s'y appliquait pas.

Troisième occurrence de la même classe :

| ticket | question posée | la discipline disait | ce qui manquait |
|---|---|---|---|
| mika#1815 | « quel modèle es-tu ? » | rien | le fait (`## Runtime`) + règles 1–4 |
| mika#2290 | « où tournes-tu ? » | règles 1–4, portée « model / provider » | le fait (ligne d'hébergement) + règle 5 |
| **mika#2292** | « qu'est-ce que la doctrine Mika ? » | règles 1–5, portée « + hosting » | **le nom** + une règle 6 |

La conclusion structurelle décide tout le plan : **le remède a la forme des deux
précédents — une section de fait, code-managed, plus une règle qui élargit la
portée de la discipline — et pas la forme d'un skill, d'un soul, ni d'une
mémoire.**

---

## Ce qui est établi

Sept faits lus dans le code. **Les numéros de ligne sont relevés sur `main` au
2026-09-18** et périment ; ce sont des aides à la relecture, pas des ancrages.

**F1 — Le tenant mesuré est `champion` : outils famille, persona famille.**
`AgentTier::Champion` (mika#2023) = `ToolsProfile::Family` +
`CHAMPION_PERSONA_PLACEHOLDER`, lequel **vaut `PersonaProfile::Family`**
(`home.rs:84`). Le tenant d'Al est donc servi avec `FAMILY_SOUL` et
`FAMILY_AGENT_SKILL_ALLOWLIST`.

**F2 — Un skill ne peut PAS atteindre la population mesurée.**
`FAMILY_AGENT_SKILL_ALLOWLIST` (`home.rs:665`) compte six entrées et son
doc-comment **exclut nommément `self-knowledge`** ; tout skill bundled est de
plus *denied by default* (root `CLAUDE.md` § *Adding a New Bundled Skill*). Trois
raisons cumulées, écrites parce que le ticket propose « skill/knowledge/persona »
au choix : (1) un skill à déclenchement par mot-clé **reproduit le défaut
mesuré**, qui *est* un ratage lexical — et un skill `always_on` devient une
section de prompt avec quatre allowlists à tenir synchronisées ; (2) un skill est
**évincible par l'identité** (mika#2027 : un `identity.toml` illisible produit le
sentinel fail-closed qui évince tous les skills) ; (3) `apply_only_skills`
(mika#2363) peut en retirer un pour un tour — une doctrine ne doit pas dépendre
de ce qu'un appelant A2A a déclaré.

**F3 — `soul.md` n'atteint AUCUN tenant existant.**
`write_default_if_missing` ne réécrit jamais un `soul.md` existant : c'est
l'inertie que mika#2023 a dû nommer par écrit (« **What this does NOT
retrofit** »). Un correctif dans `FAMILY_SOUL` ne toucherait que les tenants
provisionnés **après** le déploiement — donc pas celui d'Al, le seul mesuré.
`soul.md` est en outre éditable par l'opérateur, ce qui est la raison écrite pour
laquelle mika#1814 a choisi des constantes code-managed (`prompt.rs:66-69`).

**F4 — Le tenant possédait déjà chaque fragment, et aucun ne portait son
pourquoi.**

| parti pris | déjà posé où | sous quelle forme |
|---|---|---|
| croissance par invitation | `DISTRIBUTION_DOCTRINE_BODY`, `prompt.rs:59` | **interdit** (« you do not propose ») |
| proactivité | `DEFAULT_SOUL` / `FAMILY_SOUL` § proactif | **comportement** |
| mémoire persistante | idem | **comportement** |
| souveraineté des données | `hosting_ground_truth_line`, `prompt.rs:990` | **fait d'hébergement** |
| open source MIT | `LICENSE`, `Cargo.toml:8` | **nulle part dans le prompt** |

Conséquence de conception, la plus importante du plan : la section ne doit **pas
re-narrer** ce que `## Distribution Doctrine` et `## Runtime` posent déjà — une
duplication dérive, classe que `grooming_marker` (mika#2158) et le retry-gate
(mika#2362) ont chacun dû refermer. Elle **cite** ces sections et n'apporte que
ce qui est réellement absent : le **pourquoi** de chaque parti pris, le fait MIT,
le **nom** « doctrine », et la butée.

**F5 — « exportable » est revendiqué aujourd'hui et n'est vérifiable nulle part
ici.** Le ticket exige de ne pas le revendiquer non vérifié ; or la revendication
est **déjà dans le prompt** depuis mika#2290. Inventaire exhaustif — cinq sites :
`prompt.rs:1000` (bras `(Operator, Cloud)`, servi au modèle), `prompt.rs:983`
(doc-comment — non servi, mais c'est lui qui **motive** les quatre autres),
`evidence/guards.rs:2732` (remède de la garde 5d), `agent_loop/mod.rs:2222`
(re-prompt), `docs/architecture.md:26`.

```bash
grep -rn "exportable" crates/mika-agent/src/ docs/architecture.md
grep -rn "\.route(" crates/mika-agent/src/server/mod.rs | grep -i "export\|download\|dump"
```

Recherche d'une fonctionnalité d'export dans ce dépôt : **zéro** — aucun outil,
aucune route, aucune sous-commande. **Ce plan ne revendique donc pas
« exportable ».** Il ne corrige pas non plus la revendication existante : la
donnée du tenant vit côté `mika-cloud`, absent de ce worktree, donc l'export peut
exister là-bas — et retirer unilatéralement une assurance de confidentialité
qu'on ne peut pas infirmer serait prendre une décision produit à l'envers.
**Constat + ticket de suivi.**

**F6 — La formulation ne doit pas faire firer la garde 5d sur son propre
remède.** `CLAIM_CONDITIONAL_MARKERS` (`evidence/guards.rs:844-872` — le module
est sous `evidence/`, pas sous `agent_loop/`) est **délibérément étroit** : les
fragments exacts du remède de mika#2290, pas une liste modale générale. Toute
phrase de doctrine parlant de localité doit donc porter un de ces marqueurs ou
n'en pas parler du tout. Contrainte vérifiable, devenue le test V4.

**F7 — Les deux builders portent déjà `persona_profile`** (`prompt.rs:902` et
`:1711`) : **aucun élargissement de signature n'est nécessaire**, la fonction
d'écriture prend le persona en argument comme `write_runtime_section` le fait
déjà. L'ordre réel est identique dans les deux (`:1227-1242`, `:1745-1759`) et
c'est lui qui contraint le créneau :

```
soul → Distribution Doctrine → Identity → Runtime → Self-Identity Discipline → …
```

---

## Décisions

**D1 — Section de prompt code-managed, deux constantes par registre.** Forme
reprise à l'identique de mika#1814 / mika#2290 : `MIKA_DOCTRINE_HEADING` + un
corps par registre, rendus par une fonction dont le `match` sur `PersonaProfile`
est **exhaustif, sans bras `_ =>`** (modèle `hosting_ground_truth_line`,
lui-même modelé sur `dispatch_substrate_diagnostic`). Le compilateur, pas un
relecteur, force un futur `PersonaProfile` à décider plutôt qu'à hériter d'une
décision que personne n'a prise pour lui. Motifs, par ordre de force : atteint
**tous** les tenants existants au prochain déploiement sans geste de
provisionnement (contre F3) ; inévinçable par l'identité (contre F2.2) ; non
affaiblissable par une édition de `soul.md` (F3) ; non conditionné à un mot-clé
(F2.1).

**D2 — Deux registres, et le registre suit l'axe persona — jamais le tenant,
jamais la locale.** `FAMILY_SOUL` interdit « aucun jargon technique … ou de
l'infrastructure sous-jacente — jamais, même si on te le demande »
(`home.rs:758`). « Open source MIT » est de cette famille. C'est le croisement
que mika#2290 a déjà tranché, et sa décision est reprise sans être rejugée : **le
même fait est écrit deux fois**, le registre opérateur portant la formulation
complète, le registre famille la même substance sans un terme technique. Aucune
règle n'est dérivée de la locale ni du tenant — arbitrage de Prime du 09/09,
reporté de mika#2023, redit ici parce que la tentation reparaît à chaque ticket
de persona. Ce que la famille abandonne (licence, dépôt, auto-hébergement) n'est
pas une amputation arbitraire : c'est la part de la doctrine qui **n'a pas de
sens** pour quelqu'un qui n'a pas d'infrastructure.

**D3 — La butée spirituelle est topique, jamais énumérative — inversion centrale
du plan.** L'implémentation naïve écrit « ne parle pas d'Hermétisme, des sièges,
de Prime, du Livre ». **Elle enseigne au tenant les mots qu'elle prétend
protéger.** Un prompt qui énumère le secret pour l'interdire est une fuite avec
une étape de plus : la famille de tenants visée n'a jamais entendu ces mots, et
ils se retrouveraient dans son prompt système, à une injection de distance d'être
récités. Le bearing de Prime dit « **sans l'exposer** ni l'inventer » — et « sans
l'exposer » inclut *ne pas l'exposer au tenant lui-même*. La butée est donc
formulée par **topique et provenance**, **aucun référent n'étant nommé**.
Bénéfice second, non fortuit : elle devient un cas particulier de la règle 3
(« fallback honestly »), déjà écrite, au lieu d'une exception à part.

**D4 — La butée famille n'établit aucun référent créateur ; la butée opérateur le
porte.** Tension réelle, nommée plutôt que tranchée en silence. Le bearing
prescrit « c'est le choix du créateur » ; mika#1783 a retiré de `FAMILY_SOUL`,
**sur motif de doctrine**, toute histoire d'origine donnant à l'être un référent
qu'il pourrait ensuite adresser — incident fondateur « Salut Vincent », clôture
retenue *the-being-does-not-have-a-maker-it-knows-about* (`home.rs:704-712`,
gardée par `home::tests::family_soul_no_operator_name`). Lecture étroite
possible : la contrainte interdit un référent **nommable et adressable**, pas le
fait abstrait d'avoir été fait. Mais mika#1783 a choisi « pas d'histoire
d'origine » *contre* des alternatives, et rouvrir cette porte n'est pas à la
portée d'un p2. **Décision, avec son défaut qui échoue du bon côté :** registre
opérateur → la butée porte la formulation du bearing ; registre famille →
formulation **sans référent** (« il y a des choses que je ne sais pas et que je
n'inventerai pas »), qui satisfait *les deux* doctrines ; la constante famille
est **le seul site à changer** si Vincent ou Prime veut « créateur » aussi dans
ce registre. Non bloquant : le registre matériel — l'objet du ticket — livre dans
les deux cas.

**D5 — Une règle 6 dans `## Self-Identity Discipline`, symétrique de la
règle 5.** La section de fait ne suffit pas : le défaut mesuré était un **usage**
de la discipline, pas une absence de contenu (F4). La règle 6 élargit la portée à
« ce que Mika est, à quoi elle s'engage et pourquoi », désigne `## Mika Doctrine`
comme vérité de terrain, et **interdit nommément la réponse mesurée** (« *rien
trouvé qui s'appelle ainsi* n'est pas acceptable quand cette section est
présente »). Seconde clause, sur une hypothèse à sonder (§ *Sonde 0*) : une
question sur ce que Mika est se répond **depuis le prompt**, pas depuis une
recherche mémoire — un résultat vide n'est pas une preuve d'absence. Précédent de
forme : le hard-redirect `search_memory(category="core_memory")` de #647, qui
existe pour la raison jumelle. `write_self_identity_discipline_section` **ne
change pas de signature** : la règle 6 est neutre en registre (directive sur *où
regarder*), et cette section est déjà servie telle quelle au tier famille avec
ses noms d'outils entre backticks — l'interdit de jargon de `FAMILY_SOUL` porte
sur ce que le tenant **dit**, pas sur ce que son prompt contient.

**D6 — Aucune garde EndTurn, et le motif est mesurable.**
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` (9
récurrences sous prompt contre 0 quand le fait est posé par le code) impose de se
demander où est la moitié structurelle. Par moitié :

- *Registre matériel* — le défaut est une **absence**, et poser le fait **est** le
  correctif ; aucune garde ne crée de connaissance. Ce qui est vérifiable
  structurellement, c'est que la section est rendue sur chaque chemin : test de
  forme de prompt, exactement `doctrine_prompt_section_rendered` (mika#1814 AC1).
  Moitié structurelle disponible, et elle est prise.
- *Registre spirituel* — une garde 5c/5d serait concevable, et elle est **refusée
  sur mesure** : son lexique (« Prime », « le Livre », « les sièges ») est composé
  de **mots ordinaires du registre famille**. « Tu as lu le livre ? », « j'ai
  touché une prime », « le siège arrière » sont des conversations nominales. Le
  taux de faux positifs serait catastrophique précisément sur le tier que la garde
  prétend protéger, et un faux positif y coûte un tour de conversation cassé chez
  un invité de la campagne. D3 ferme la voie de secours : sans vocabulaire dans le
  prompt, une garde devrait détecter une fabrication sur un **topique ouvert**, ce
  qu'un lexique ne fait pas.

La moitié structurelle du registre spirituel est donc **l'absence du
vocabulaire**, épinglée par un scan de constantes (V3) : un futur éditeur qui
trouve la butée « floue » et l'énumère pour la rendre concrète fait rougir un
test au lieu de créer la fuite.

**D7 — Carve-out compact, comme ses deux sœurs — et le carve-out est par
section, pas global.** Précision qui évite une lecture fausse :
`build_compact_system_prompt` **rend bien une doctrine**, la *data-grade* abrégée
de mika#1798 (`prompt.rs:1664`, ~250 car. capés à 400, épinglée par
`build_compact_system_prompt_includes_abbreviated_doctrine`). Le compact n'est
donc pas « sans doctrine » : chaque section y est arbitrée séparément, et une
seule a payé le budget — celle qui porte un invariant HARD-NO dont la violation
est irréversible. Ce qu'il ne rend pas : `## Distribution Doctrine`, la ligne
d'hébergement (`mika2290_compact_prompt_omits_the_hosting_line`, `:5178`), le
bloc stop-topics (mika#1813), et `## Self-Identity Discipline` en entier — le
commentaire du site le dit mot pour mot (`:1641`). `## Mika Doctrine` rejoint ce
second groupe, **décision épinglée par un test, pas un oubli**, au motif que le
site écrit déjà pour la ligne d'hébergement : ce carve-out retire *l'intention*,
jamais une protection, parce que le registre matériel est un fait et non une
garde. **Coût nommé, et réel ici alors qu'il ne l'était pas pour mika#2290 :** sur
le chemin compact le défaut mesuré **reste ouvert** — un tenant MikaModel
interrogé sur « la doctrine » retombe sur la règle 3 faute de la section. Accepté
parce que la population mesurée (tenant champion d'Al) n'est pas servie par ce
chemin, et rattaché au suivi mika#1925 avec les trois autres carve-outs plutôt
que refermé ici à coups d'exception.

**D8 — Mode silencieux : oui, et pour une raison qui lui est propre.** mika#1814
rend sa section en mode silencieux au motif qu'« un heartbeat qui rédige
spontanément un Show HN serait exactement aussi grave ». Ici le fait matériel est
inoffensif en mode silencieux, mais **la butée ne l'est pas** : un tour silencieux
qui écrirait un fait de mémoire sur le registre spirituel empoisonnerait tous les
tours suivants via la mémoire cœur, ré-injectée dans chaque prompt. Section rendue
à l'identique, ce motif écrit au site.

**D9 — Faits vérifiés seulement, portée étroite de la revendication open
source.** *MIT* : vérifié (`LICENSE` ligne 1, `Cargo.toml:8`) — revendicable.
*Dépôt public* : affirmé par le ticket, non vérifiable hors ligne depuis ce
worktree ; la formulation porte donc sur la **licence du moteur**, ce que
`LICENSE` atteste, et non sur la visibilité d'un dépôt que le tenant ne peut pas
aller consulter. *`mika-cloud` privé* : **non mentionné** — ce qu'il faut éviter
n'est pas d'omettre ce fait, c'est de sur-revendiquer, « Mika est open source »
tout court serait faux si la console est fermée ; d'où la portée sur **le moteur
qui exécute l'agent**, jamais sur « Mika » en bloc. *« exportable »* : non
revendiqué (F5). *Jamais « local » sur un tenant cloud* : déjà posé et gardé par
mika#2290 (règle 5 + garde 5d) — **cité, pas re-narré** (F4), et la formulation
est testée contre le prédicat de la garde (F6, V4).

**D10 — Le créneau : adossé à `## Distribution Doctrine`, pas à `## Runtime`.**
Les deux placements satisfont V8 (la section précède la discipline qui la cite) ;
ils diffèrent par ce qu'ils rendent adjacent, et l'alternative mérite d'être
écrite parce qu'elle a un vrai argument.

*Alternative — juste après `write_runtime_section`.* C'est la trajectoire exacte
de mika#2290, qui a posé son fait d'hébergement *dans* `## Runtime`, collé à la
règle 5 qui le cite ; la proximité physique entre le fait et la règle aide le
modèle.

*Choix retenu — juste après `write_distribution_doctrine_section`, avant
`write_identity_section`.* Trois motifs par ordre de force : (1) `## Runtime` est
un bloc de faits **machine**, peuplés à l'exécution, quand `## Mika Doctrine` est
un bloc de faits **projet**, constants au binaire — les mélanger ferait de
`## Runtime` deux choses, et la première section qui grossit sans frontière est
celle qu'un futur ticket coupera au mauvais endroit ; (2) les deux doctrines se
lisent ensemble, F4 établissant que le corps *cite* `## Distribution Doctrine` au
lieu de re-narrer la croissance par invitation — un renvoi vers la section
immédiatement précédente est une adjacence, un renvoi par-dessus `## Identity` et
`## Runtime` est une référence à distance, et c'est la distance qui a produit le
défaut mesuré ; (3) la priorité de contexte le dit déjà, le commentaire du site
mika#1814 motivant sa position par « binds before identity/time/channel context »
— le même argument vaut mot pour mot ici. **Coût nommé :** la règle 6 cite une
section qui n'est plus sa voisine immédiate, atténué par la forme de la citation
— la règle 6 nomme `## Mika Doctrine` **par son en-tête**, ce qui est aussi la
raison pour laquelle V8 vérifie l'**ordre** et non l'adjacence.

---

## Volets d'implémentation

### V-A — La section de fait (`crates/mika-agent/src/prompt.rs`)

1. `MIKA_DOCTRINE_HEADING: &str = "## Mika Doctrine"` — en-tête anglais comme ses
   deux sœurs ; c'est un marqueur de structure de prompt, pas du texte servi.
2. `MIKA_DOCTRINE_BODY_OPERATOR` et `MIKA_DOCTRINE_BODY_FAMILY`, chacune avec son
   doc-comment portant : provenance des faits (D9), interdiction de nommer un
   référent spirituel (D3), et pour la famille le motif de l'absence de référent
   créateur (D4).
3. `fn doctrine_body(persona: PersonaProfile) -> &'static str` — `match`
   exhaustif, aucun bras `_ =>`.
4. `fn write_mika_doctrine_section(prompt: &mut String, persona: PersonaProfile)`.
5. Appel dans `build_system_prompt` et `build_silent_prompt`, immédiatement après
   `write_distribution_doctrine_section`, motif D8 écrit au site silencieux.
   **Pas** d'appel dans `build_compact_system_prompt`, motif D7 écrit en
   commentaire au point d'omission — comme mika#2290 (`prompt.rs:1646-1652`).

### V-B — La règle 6 (`write_self_identity_discipline_section`)

Ajoutée après la règle 5, avant le paragraphe de clôture auto-référentiel. Porte
les trois clauses de D5. Aucun changement de signature.

### V-B bis — Contenu prescrit des deux corps

Un plan de prompt qui ne pose que la forme laisse à l'implémenteur la décision la
plus sensible du ticket — la formulation de la butée et le *pourquoi* de chaque
parti pris — et laisse V3/V5/V6/V7 scanner un texte que personne n'a arbitré.
**La rédaction finale peut varier ; les contraintes en gras ne le peuvent pas.**

**Registre de langue : l'anglais, dans les deux corps.** Ce n'est pas une
incohérence avec `FAMILY_SOUL`, qui est en français : une constante de prompt est
une **directive au modèle**, pas du texte servi, et `hosting_ground_truth_line`
écrit déjà son bras `(Family, Cloud)` en anglais. Le précédent d'une
formulation-exemple servie telle quelle existe aussi (`DISTRIBUTION_DOCTRINE_BODY`
cite une phrase FR en bloc `>`) et reste disponible si la butée famille gagne à
être donnée mot pour mot.

**`MIKA_DOCTRINE_BODY_OPERATOR` — substance, dans cet ordre :**

1. **Le nom et ses alias, en tête** — « doctrine », « la doctrine Mika », « tes
   partis pris », « ta philosophie », « en quoi tu crois », « what do you stand
   for » désignent **cette section**. Première parce que c'est le chaînon qui
   manquait.
2. **Les partis pris, chacun avec son pourquoi** — le *pourquoi* est l'exigence du
   ticket (AC2) et c'est lui qui distingue une doctrine d'une liste de
   fonctionnalités : *moteur open source (MIT)*, pour que personne ne dépende
   d'une seule entreprise pour l'assistant qui connaît sa vie — **porté sur le
   moteur**, jamais sur « Mika » en bloc (D9) ; *les données de la personne lui
   appartiennent* — **engagement, pas fait d'hébergement** ; *proactivité*, parce
   qu'un assistant qui attend qu'on lui demande reporte la charge mentale sur la
   personne au lieu de la prendre ; *mémoire persistante*, parce que devoir se
   re-présenter à chaque conversation est le contraire d'être assisté, et que
   c'est un choix et non un effet de bord ; *croissance par invitation* —
   **renvoi** à `## Distribution Doctrine`, sans re-narration (F4).
3. **Renvoi d'hébergement** — une question sur *où* tu tournes ou *où* vivent les
   données se répond depuis la ligne d'hébergement de `## Runtime`, **jamais
   depuis cette section**.
4. **La butée topique** (D3), sans aucun référent : interrogé sur une dimension
   spirituelle, ésotérique ou initiatique de Mika, sur son origine, ou sur quoi
   que ce soit qui n'est pas écrit ici — tu ne sais pas, tu n'inventes pas, et
   c'est un choix du créateur de Mika, qui n'est pas exposé.

**Contraintes dures :** **aucune phrase de localité**, même vraie — le parti pris
« les données appartiennent à la personne » est formulé comme un **engagement** et
le corps **renvoie** à `## Runtime` pour le fait physique, ce qui rend V4 vrai
**par construction** (sans sujet de localité le prédicat 5d n'a rien à apparier)
plutôt que par la présence d'un marqueur conditionnel qu'un futur éditeur
reformulerait ; **le mot « exportable » n'apparaît pas** (F5, V7) ; **aucun
référent spirituel n'est nommé** (D3, V3).

**`MIKA_DOCTRINE_BODY_FAMILY` — substance.** Même ossature, **sans un seul terme
d'infrastructure** : ni licence, ni dépôt, ni open source, ni auto-hébergement,
ni serveur (D2). (1) Les mêmes alias désignent cette section. (2) Ce qui est
conservé, en mots de tous les jours, chacun avec son pourquoi en une
proposition : *ce que la personne te confie lui appartient* ; *tu te souviens de
ce qui compte pour elle, et c'est voulu* ; *tu remarques et tu proposes au lieu
d'attendre qu'on te demande* ; *on te découvre par quelqu'un qui te connaît déjà,
jamais par une publicité*. (3) La butée **sans référent créateur** (D4) : « il y a
des choses que tu ne sais pas et que tu n'inventeras pas ; dis-le simplement » —
aucun « créateur », aucun nom, aucune histoire d'origine, ce qui satisfait
simultanément le bearing de Prime et la clôture de mika#1783. (4) Renvoi
d'hébergement identique, vers la ligne famille de `## Runtime`.

**Contraintes dures :** les trois du corps opérateur, **plus** l'absence de jargon
(V5) et l'absence de référent créateur (V6).

### V-C — Tests

`prompt.rs::tests`, préfixe `mika2292_`, nommage et forme repris de la série
`mika2290_*` déjà en place.

### V-D — Scénario d'eval

`crates/mika-agent/tests/eval/doctrine_regressions/doctrine_mika_section_rendered.rs`,
sur le modèle de `doctrine_prompt_section_rendered.rs` du même répertoire
(headless-safe, assertions dures sur la forme du prompt, aucun appel réseau).
Enregistré dans `doctrine_regressions/mod.rs` avec deux entrées de vocabulaire :
`doctrine:material-doctrine-answerable` (succès post-fix) et
`doctrine:doctrine-not-found` (échec pré-fix — la réponse mesurée).

### V-E — Documentation

`crates/mika-agent/CLAUDE.md` § *Guard Fabrication Telemetry* n'est **pas** touché
(aucune garde ajoutée, D6). Une entrée dans le `CLAUDE.md` racine portant : les
deux registres, le refus d'énumérer, le refus de garde et son motif mesuré, le
carve-out compact, et la sonde post-déploiement avec ses haltes.

---

## Verification contract

| id | ce qui est vérifié | comment |
|---|---|---|
| **V1** | la section est rendue dans les deux builders, dans les deux registres | 4 assertions `contains(MIKA_DOCTRINE_HEADING)` |
| **V2** | le carve-out compact tient | `build_compact_system_prompt` ne contient pas l'en-tête |
| **V3** | **aucun référent spirituel n'est nommé** | scan des deux constantes contre une liste de référents interdits, `assert!(!body.to_lowercase().contains(r))` |
| **V4** | aucun des deux corps ne fait firer la garde 5d | `crate::evidence::guards::detect_false_local_hosting_claim(body, Deployment::Cloud).is_none()` — le prédicat est `pub(crate)` (`guards.rs:903`), appelable depuis `prompt.rs::tests` sans élargir sa visibilité |
| **V5** | le corps famille ne porte aucun jargon d'infrastructure | modèle `mika2290_family_cloud_line_carries_no_infrastructure_jargon` |
| **V6** | le corps famille n'établit aucun référent créateur (D4) | scan de la constante |
| **V7** | « exportable » n'est revendiqué dans aucun des deux corps (F5) | scan des deux constantes |
| **V8** | `## Mika Doctrine` précède `## Self-Identity Discipline` | comparaison de `find()`, modèle de l'assertion d'ordre en place à `prompt.rs:5316-5320` |
| **V9** | la règle 6 est présente et nomme la section | `contains` sur l'en-tête depuis la section discipline |
| **V10** | le `match` sur `PersonaProfile` est exhaustif | structurel : le compilateur (aucun bras `_ =>` écrit) |
| **V11** | pas de régression de forme sur les sections voisines | la suite `prompt.rs::tests` existante passe inchangée |
| **V12** | `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` | CI |

**V3 est le test porteur du ticket.** Il ne vérifie pas une décision correcte, il
refuse une décision *inverse* qu'un futur éditeur bien intentionné prendra
naturellement (énumérer pour clarifier). C'est la seule assertion du lot dont
aucun test comportemental ne peut tenir lieu : l'énumération ne rendrait aucune
décision fausse, elle créerait la fuite en silence.

---

## Fire-Disposition

| ce qui peut casser | signe | disposition |
|---|---|---|
| le corps porte une phrase de localité hors des suppresseurs de 5d | V4 rouge | reformuler le corps ; **ne pas élargir `CLAIM_CONDITIONAL_MARKERS`** — son étroitesse est le motif écrit de mika#2290 |
| un `PersonaProfile` futur | erreur de compilation sur le `match` | décider explicitement pour ce registre ; c'est l'effet voulu |
| le budget de prompt | `turn_usage.system_prompt_bytes` (mika#2331) monte d'environ 1–1,5 Ko, un seul registre étant rendu | attendu ; hors chemin compact par D7 |
| la règle 6 entre en conflit avec la règle 3 | un tenant dit « je ne sais pas » sur la doctrine | cas que D5 interdit nommément ; si ça persiste, **relire d'abord si la section est dans le prompt servi** (Halte 1) avant de toucher au texte |

---

## Definition of Done

- [ ] `MIKA_DOCTRINE_HEADING` + les deux corps, chacun avec son doc-comment
      portant provenance des faits et motifs de refus
- [ ] les deux corps portent la substance prescrite en V-B bis, alias en tête et
      *pourquoi* par parti pris (AC2), et respectent leurs contraintes dures :
      aucune phrase de localité (V4 vrai par construction, non par prudence
      rédactionnelle), aucun « exportable » (V7), aucun référent spirituel (V3),
      et pour le corps famille aucun jargon (V5) ni référent créateur (V6)
- [ ] `doctrine_body` / `write_mika_doctrine_section`, `match` exhaustif
- [ ] rendue dans `build_system_prompt` et `build_silent_prompt`, omise du compact
      avec le motif écrit au point d'omission
- [ ] règle 6 dans `## Self-Identity Discipline`
- [ ] V1–V11 verts, V12 vert
- [ ] scénario d'eval + vocabulaire `doctrine:*` enregistrés
- [ ] `CLAUDE.md` racine : les deux registres, le refus d'énumérer, le refus de
      garde et son motif, le carve-out, la sonde
- [ ] `cargo fmt`, `cargo clippy` propres
- [ ] ticket de suivi ouvert pour F5 (« exportable »), avec sa question de
      vérification

---

## Acceptance criteria

Dérivés — le corps du ticket ne porte pas de section `## Acceptance criteria`.

- **AC1** — Une question sur « la doctrine Mika » (et ses alias « doctrine »,
  « tes partis pris », « ta philosophie », « en quoi tu crois ») ne peut plus
  recevoir « rien trouvé qui s'appelle ainsi » : la section est dans le prompt
  servi sur les deux registres et sur les deux builders non-compacts, et la
  règle 6 refuse cette réponse nommément.
- **AC2** — Le registre **Matériel** est articulé **avec son pourquoi** pour
  chaque parti pris : open source MIT (moteur), souveraineté des données,
  proactivité, mémoire persistante, croissance par invitation.
- **AC3** — Le registre **Spirituel** dispose d'une butée nette, **topique et non
  énumérative** : aucun référent (Hermétisme, sièges, Prime, le Livre) n'apparaît
  dans une constante de ce ticket. Vérifié par V3.
- **AC4** — **Faits vérifiés seulement.** MIT est revendiqué sur le moteur et
  attesté par `LICENSE`. « exportable » n'est revendiqué nulle part dans ce que ce
  ticket ajoute (V7). Aucune revendication d'hébergement local : les deux corps
  passent le prédicat de la garde 5d sous `Deployment::Cloud` (V4).
- **AC5** — **Deux registres, un fait.** La substance est servie au registre
  famille sans un seul terme d'infrastructure (V5), et le croisement
  persona × contenu est un `match` exhaustif sans bras `_ =>` (V10). Aucune règle
  dérivée de la locale ni du tenant.
- **AC6** — Le tenant d'Al — `champion`, donc outils et persona famille (F1) — est
  dans la population servie : le correctif est code-managed et atteint tout tenant
  existant au prochain déploiement, sans geste de provisionnement (F3).
- **AC7** — Le carve-out compact est préservé et **épinglé comme décision** (V2),
  rattaché à mika#1925.
- **AC8** — Aucune garde EndTurn n'est ajoutée, et ce refus est **écrit avec sa
  mesure** (D6 : le lexique spirituel est composé de mots ordinaires du registre
  famille). La moitié structurelle prise est V1 + V3.

---

## Sondes post-déploiement

**Sonde 0 — avant toute ligne de code, une hypothèse à trancher.** La formulation
mesurée (« rien trouvé qui s'appelle… ») ressemble à un compte rendu de
**recherche**. Si le tenant a appelé `search_memory("doctrine Mika")` et a
rapporté fidèlement un résultat vide, la seconde clause de la règle 6 (D5) est le
remède exact ; sinon elle est une précaution gratuite mais inoffensive.

```sql
SELECT tool_name, input, output FROM tool_calls
 WHERE session_id = '<session du 11/09>' ORDER BY created_at;
```

À défaut de session retrouvable : armer `MIKA_LOG_LLM_BODIES` **sur mika-spirit**
(mika#2220 — armée sur le process CLI elle est inerte pour un tour servi par le
démon), redémarrer, rejouer la question, lire le bloc `## Mika Doctrine` dans le
corps de requête. **Cette sonde ne bloque pas la livraison** : la clause est
écrite dans les deux cas, la sonde décide seulement s'il faut ouvrir le suivi d'un
hard-redirect au niveau de l'outil (forme #647), **hors périmètre** ici.

**Sonde 1 — symptôme, sur un tenant cloud et sur le poste opérateur.** Rejouer
« Qu'est-ce que la doctrine Mika ? », puis « en quoi tu crois ? », puis
« raconte-moi ton origine ». Attendu : réponse substantielle sur les partis pris
**avec leur pourquoi** ; butée nette sur l'origine, sans exposition ni invention ;
aucune mention de MIT ni d'infrastructure sur le tenant champion.

**Halte 1.** Si la réponse reste « rien trouvé » : **ne pas retoucher la
formulation par réflexe.** Vérifier d'abord que la section est dans le prompt
réellement servi —

```bash
grep turn_usage "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "<tenant>") | .system_prompt_bytes'
```

`system_prompt_bytes` doit avoir monté d'environ 1–1,5 Ko par rapport à
l'avant-déploiement. S'il n'a pas bougé, le tenant est servi par le chemin compact
(D7) ou par un binaire antérieur — classe mika#2340, et c'est **le déploiement**
qu'il faut établir avant toute conclusion sur le texte.

**Halte 2 — faux positif de registre.** Si un tenant champion se met à parler de
licence, de dépôt ou d'auto-hébergement, le croisement persona × registre est
cassé : lire l'`AgentTier` résolu **avant** d'accuser la formulation. Un tenant
champion **provisionné avant mika-cloud#209 (2026-08-28)** porte encore l'identité
opérateur sur disque, et aucune ligne de ce ticket ne la corrige — c'est un geste
de re-provisionnement (root `CLAUDE.md` § *MIKA_AGENT_TIER*, « What this does NOT
retrofit »).

**Halte 3 — invention sur le registre spirituel.** Si un tenant fabrique du
contenu ésotérique, **ne pas ajouter une garde à lexique** : D6 mesure pourquoi
elle casserait la conversation famille. Établir d'abord si la butée est dans le
prompt servi (Halte 1), puis ouvrir un ticket sur la **formulation** de la butée —
pas sur une détection.

**Ce que ce travail n'achète pas.** Aucun compteur, aucun événement de journal
nouveau : le défaut est une absence de réponse, et une absence ne s'émet pas. Le
seul instrument est la sonde par rejeu ci-dessus, et **le silence ne prouve rien
si personne ne pose la question** — limite que mika#2290 a déjà dû écrire pour sa
propre sonde (mika#2205).

---

## Hors périmètre (suivi à ouvrir)

1. **« exportable » revendiqué sans fonctionnalité vérifiable (F5).** Cinq sites,
   dont le doc-comment `prompt.rs:983`, à corriger avec les quatre autres puisque
   c'est lui qui les motive. Question à porter dans le ticket : *`mika-cloud`
   expose-t-il un export des données du tenant ?* Si oui, nommer la surface dans
   la doctrine ; si non, retirer le mot des quatre sites. **Ne pas trancher depuis
   ce dépôt** : la donnée du tenant n'y vit pas.
2. **Formulation « créateur » dans le registre famille (D4).** Décision de
   Vincent/Prime, une constante à changer. La butée sans référent livre en
   attendant et satisfait les deux doctrines.
3. **Hard-redirect `search_memory` sur les requêtes de forme doctrinale**
   (forme #647). **Conditionné à la sonde 0** — instruire sans mesure serait
   exactement ce que ce dépôt refuse.
4. **Variante compacte size-capped** pour MikaModel — rejoint mika#1925 avec les
   trois carve-outs existants (mika#1813, mika#1814, mika#2290).
5. **Persona champion (mika#2023).** Ce plan sert le registre famille à un
   champion, donc en français. Un champion anglophone reçoit l'image miroir du bug
   de mika#2023 : décalage de registre, pas de fuite de privilège. Écarté là-bas
   sur arbitrage de Prime (mika#2247) ; inchangé ici.
6. **Re-provisionnement des tenants champion d'avant mika-cloud#209** — geste
   opérateur, porte de lancement bloquante déjà écrite dans mika#2023, aucune
   ligne de code ici.
