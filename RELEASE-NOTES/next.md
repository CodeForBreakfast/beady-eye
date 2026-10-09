bdi 0.26.0

Minor release, **0.25.0 → 0.26.0**. Nothing needs changing to upgrade. To crop Claude Code's input box out of the tail, read the second highlight.

## Highlights

**The tail grows into the rows the tree leaves free.** It used to take six lines whatever the screen. It now takes whatever the tree does not need, up to half the screen, and still gives rows up before the tree does.

**The tail can cut an agent's input box off the bottom of a pane.** A `[tail.crop]` table maps the agent name herdr reports to a crop `bdi` ships. The one shipped is `claude-code`, which cuts above Claude Code's input box, so the tail shows what the agent last said rather than the box, its meter and its mode line. The [configuration reference](https://github.com/CodeForBreakfast/beady-eye/blob/main/docs/configuration.md) has the table.

**`t` hides and shows the tail.** Hiding it gives its rows to the tree, and `bdi` reads no pane while it is hidden. The tail is shown at every start.

**A gate with no open blocker reads as ready.** Before, `bdi` read every gate as not ready.

**Boards cost less.** A board skips a watcher push that moved nothing it shows, and holds less in memory, while drawing exactly what it drew before. The watcher uses about two fifths less memory.

## Maintenance

Dependency bumps, a benchmark of one watcher and five boards, and a crate description and `--help` line that say what `bdi` does.
