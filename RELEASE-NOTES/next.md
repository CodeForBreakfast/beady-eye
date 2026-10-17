bdi 0.31.0

Minor release, **0.30.0 → 0.31.0**.

## Highlights

**`bdi gates` closes a bead gate once the bead it awaits closes.** The bead can be in the gate's own tracker or in another project the config names. For another project, set that project's `prefix` in the config, or `bdi gates` reports the gate as unsettleable on every look. Where a producer covers the bead's tracker, the gate closes moments after the bead does, because a producer's change report now wakes `bdi gates` at once rather than waiting for the next poll.

## Maintenance

Dependency bumps and internal improvements.
