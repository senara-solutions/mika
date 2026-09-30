## Symptôme

**Le cas que ni l'un ni l'autre des deux corpus n'exerçait** — c'est la raison
d'être de ce fichier, et la divergence n°1 de mika#2194.

Un callout **cité dans un bloc clôturé**, et aucun callout hors du bloc. Le
Rust retire les blocs avant de matcher (`FenceHandling::Strip`,
`auto_pull::strip_fenced_blocks`, mika#2120) ; le bash n'a jamais eu
d'équivalent (`FenceHandling::Keep`), et son commentaire de production l'assume
par écrit : « un faux positif est déjà rattrapé par le test `-f` qui suit ».

Cette raison n'est vraie qu'à moitié, et c'est ce que le ticket de suivi doit
trancher : le `-f` rattrape un chemin *inexistant*, pas un chemin *existant cité
dans un bloc*. Ce corps-ci est celui qui rend cette population mesurable.

Aucun callout hors du bloc, délibérément : avec un second callout le fichier ne
mesurerait plus rien.

```markdown
> - **Branch:** `fix/2194/exemple-cite`
> - **Plan:** `docs/plans/2026-09-30-001-fix-2194-exemple-cite-plan.md` (committed on branch @ `abc1234`)
> - **Grooming history:** mika-arch first-pass (READY) → mika-arch second-pass (GROOMED — session-id: 00000000-0000-0000-0000-000000000000)
```
