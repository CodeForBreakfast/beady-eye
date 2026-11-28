bdi 0.48.0

Minor release, **0.47.0 → 0.48.0**.

## Highlights

**bdi can flag a bead that nobody is watching.** Add an `[[anomalies.unwatched]]` entry to your config, choosing beads with the same `projects`, `when` and `unless` a view's selection takes, and each such bead that is not closed and that no agent watches reads "nobody watches it". An agent holding the bead's claim counts as watching it. The flag needs `bdi watch` running, because only the watcher knows who watches a bead.

**A bead's page in `bdi web` shows the bead even where the view leaves it out.** When the page's list or root hides the bead, the page opens the bead's panel under a line saying "This view hides this bead", with a Show all link to the bead among every tree. A bead bdi has not read still says "This bead is not in any tree".

**A tracker bdi cannot read now names the `bd` command that failed.** The screen, the `bdi gates` log and `bdi --beads` add the command after the failure's sentence, quoted so you can paste it after `bd` and run it again. Where bdi cannot tell what went wrong, it adds what `bd` said, unless those words may hold a password. `bdi watch` also writes a line to its error output when a tracker stops answering or answers again.

**Fewer trackers read as unavailable when many share one server.** bdi now runs at most sixteen `bd` at a time, so reading many projects asks their server for far fewer connections at once.

**`bdi watch` keeps taking connections on a host that starts it with a low open-files limit.** It raises its own limit as far as the system allows, and says so on its error output if the system refuses.

## Upgrading

**Restart `bdi watch` after upgrading if you use the new flag.** A watcher started before the upgrade cannot say which beads nobody watches, so until it restarts bdi flags none of them.

## Maintenance

Dependency bumps.
