beady-eye plugin 0.3.0

Minor release, **0.2.4 → 0.3.0**.

## Highlights

**`bdi` can show which agents are watching a bead.** Each watch now tells the watcher which Claude Code session it comes from. From bdi 0.37.0, a watched bead's row ends in `◉`, and its bead window names each agent watching it. Against an older bdi, watches work as before, and no bead is marked.

**Watches survive `/clear`.** When `/clear` or `/resume` starts a new session in the same terminal, the plugin moves its watches to the new session, and changes keep arriving. The move happens at the session's next `watch`, `unwatch` or `watching` call.

**A session hears when the watcher stops watching a bead.** If the watcher drops a watch it had accepted, for instance because the bead's project left the bdi config, the session is told once, in a block marked `watch="refused"` that gives the reason.

**A bead gone from its tracker reads differently.** It now arrives marked `gone="true"`, with no status, so it cannot be mistaken for a bead whose own status is called `gone`.

**Clearer refusals.** A watch refused because a project is unreachable names that project and the reason. A bdi config that cannot be read, or that sets an empty watcher socket, is reported as a broken config naming the file, not as a watcher that is down.

**Fixes.** A session that missed the message that the watcher was down is no longer told it is back. A watcher sending a status that is not a bead status now counts as down. A project name or bead id holding a character the watcher reads as a space is refused when you watch it.

**Updating.** Pin the marketplace to `plugin-v0.3.0` as the README shows, and update the plugin. bdi and the plugin can be updated in either order.

## Maintenance

Dependency bumps and tests.
