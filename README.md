# beady-eye

A read-only view of work in flight: a tree of beads, each node annotated with the
live agent working on it.

[beads](https://github.com/steveyegge/beads) knows the work — the tree, the
dependency edges, each bead's status and who claimed it.
[herdr](https://herdr.dev) knows the agents — which pane is alive and what it is
doing. Nothing joins them, so there is no single answer to "what is left, what is
done, and what is being worked on right now" — and drift between the two (a
closed bead whose agent never exited, a claim whose agent died) is invisible.

`bdi` is that join. The work is the spine; agents are an annotation.

```
▾ summit-works · nix-9670s   DMS → noctalia v5      8/21   3 agents  ⚠ 3
  ├── ● .20  wallpaper timer calls dms            ◍ wCM:p9  working
  ├── ● .1   wire the niri theme include          ◍ wCM:p6  idle
  │   ├── ○ .4   restore app theming
  │   │   ├── ○ .8   make the switch permanent
  │   │   │   └── ○ .9   confirm quickshell wedges gone
  │   │   └── ○ .5   retire the DMS remnants
  │   └── ○ .17  apply the two niri settings
  ├── ◐ .16  guard a key in both layers          ⚠ claimed · no pane
  └── … 13 more
```

## What it assumes

**bd, and nothing else.** herdr is optional: without it you still get the tree,
the counts, the claims and an age-based stale-claim warning. With it you also get
liveness, exact drift detection, and a tail of the selected bead's pane.

It knows nothing about any particular way of organising agents — no orchestration
model, no roles, no workflow. Conventions your setup encodes in bead metadata are
named in config and drawn as badges; `bdi` never learns what they mean.

## Status

Design accepted, not yet implemented. See [docs/design.md](docs/design.md).
