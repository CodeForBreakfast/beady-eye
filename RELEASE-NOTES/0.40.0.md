bdi 0.40.0

Minor release, **0.39.0 → 0.40.0**.

## Highlights

**`r` in a view hides the beads that are still waiting.** The view then lists only the beads you could act on now: those whose blockers have all closed, in any project bdi reads, and those already being worked. Its heading gains `ready`, and `r` again shows the whole view. `v` keeps this on as it moves through your views, and after the last one it shows the ready list instead of the tree.

## Upgrading

**`r` in a view no longer opens the ready list.** To get there from a view, press `r` and then `v` until you pass the last view.
