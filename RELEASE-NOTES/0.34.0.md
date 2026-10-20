bdi 0.34.0

Minor release, **0.33.0 → 0.34.0**.

## Highlights

**A bead's tree in `bdi --beads` lists the beads in delivery and those already delivered.** A bead held by an open pull request's gate is listed under `in_delivery`, and a ready bead whose pull request has merged is listed under `delivered`, to close rather than start. Each names the `pull_requests` it goes through.

**A ready bead whose pull request has merged is no longer in `ready`.** It wants closing rather than starting, so it moves to `delivered`, with or without a `[delivery]` section in your config. If a script of yours reads `.ready[]` and should still see such a bead, have it read `.delivered[]` too.

**`[delivery]` in the config names how your setup marks delivery.** `outside_key` is the metadata key that marks a gate as another effort's pull request, and `outcome_label` is the label that marks a molecule's outcome step. Neither has a default, and without the section every pull request's gate delivers a bead and no step is an outcome step.

## Maintenance

Dependency bumps and internal improvements.
