bdi 0.30.0

Minor release, **0.29.0 → 0.30.0**.

## Highlights

**A live claim is no longer an orphan on a machine its seat does not run on.** A claim whose lease is live, or that sits beneath a bead holding one, counts as worked, so every machine reading the tracker agrees about it. The pane is still drawn only on the machine where it runs. A claim stays live only while its holder renews it with `bd heartbeat`. `--beads` now lists each bead's `assignee` and `lease_expires_at`.

## Maintenance

Dependency bumps and internal improvements.
