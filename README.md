# beady-eye

A read-only view of work in flight: a fleet effort's bead graph drawn as a tree,
each node annotated with the live agent working on it.

[beads](https://github.com/steveyegge/beads) knows the work — the anchor epic,
its children, the dependency edges, each bead's status and who claimed it.
[herdr](https://herdr.dev) knows the workers — which pane is alive and what it
is doing. Nothing joins them, so there is no single answer to "what is left,
what is done, and what is being worked on right now" — and drift between the two
(a closed bead whose seat never exited, a claim whose worker died) is invisible.

`bdi` is that join. The work is the spine; agents are an annotation.

```
▾ summit-works · nix-9670s   DMS → noctalia v5      8/21   3 seats  ⚠ 3
  ├── ● .20  wallpaper timer calls dms            ◍ wCM:p9  working
  ├── ● .1   wire the niri theme include          ◍ wCM:p6  asleep
  │   ├── ○ .4   restore app theming
  │   │   ├── ○ .8   make the switch permanent
  │   │   │   └── ○ .9   confirm quickshell wedges gone
  │   │   └── ○ .5   retire the DMS remnants
  │   └── ○ .17  apply the two niri settings
  ├── ◐ .16  guard a key in both layers          ⚠ claimed · no pane
  └── … 13 more
```

herdr is optional. Without it you still get the tree, the counts, the claims and
the waiting-on-you set; with it you also get liveness, drift detection, and a
tail of the selected bead's pane.

## Status

Design accepted, not yet implemented. See [docs/design.md](docs/design.md).
