bdi 0.30.0

Minor release, **0.29.0 → 0.30.0**.

## Highlights

**A live claim is no longer an orphan on a machine its seat does not run on.** A claim whose lease is live, or that sits beneath a bead holding one, counts as worked, so every machine reading the tracker agrees about it. The pane is still drawn only on the machine where it runs. A claim stays live only while its holder renews it with `bd heartbeat`. `--beads` now lists each bead's `assignee` and `lease_expires_at`.

**`bdi gates` closes a bead gate once the bead it awaits closes.** The bead can be in the gate's own tracker or in another project the config names. For another project, set that project's `prefix` in the config, or `bdi gates` reports the gate as unsettleable on every look. Where a producer covers the bead's tracker, the gate closes moments after the bead does, because a producer's change report now wakes `bdi gates` at once rather than waiting for the next poll.

## Maintenance

Dependency bumps and internal improvements.
