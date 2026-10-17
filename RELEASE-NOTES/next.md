bdi 0.31.0

Minor release, **0.30.0 → 0.31.0**.

## Highlights

**`bdi gates` closes a bead gate once the bead it awaits closes.** The bead can be in the gate's own tracker or in another project the config names. For another project, set that project's `prefix` in the config, or `bdi gates` reports the gate as unsettleable on every look. Where a producer covers the bead's tracker, the gate closes moments after the bead does, because a producer's change report now wakes `bdi gates` at once rather than waiting for the next poll.

**A GitHub delivery to `bdi gates` reads only the trackers that changed.** It settles the pull request against the gates `bdi gates` already holds, so the burst of check deliveries after a merge starts no `bd` once a producer covers every tracker. A gate written into a tracker the clock reads is settled at the next look rather than by a delivery. You need do nothing.

## Maintenance

Dependency bumps and internal improvements.
