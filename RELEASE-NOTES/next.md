bdi 0.46.0

Minor release, **0.45.0 → 0.46.0**.

## Highlights

**You can make small changes to a bead from bdi.** Open a bead's window and press a key: `m` comments, `x` closes with a reason, `o` reopens, `R` responds to a bead that needs a human, `X` dismisses one, `z` defers until a date, `u` releases the claim of an agent that has gone, and `p` sets the priority. Words are typed on a line at the foot, Enter makes the change and Esc leaves it unmade, and a close is the only change that needs no words. bdi says at the foot what it did, or shows the message bd refused it with.

**`bdi web` can make the same changes, once you turn them on.** Start it with `--writes` and each bead's page offers forms to comment, close, reopen, respond, dismiss, defer and set the priority. Release stays in the terminal. Without `--writes`, `bdi web` stays read-only, and no page shows a form.

**A writable `bdi web` writes as the one name you give it.** Set `actor` under `[web]` in your config, and bd records every change from the page under that name. `bdi web --writes` refuses to start without one. It reads the name when it starts, so an edit waits for a restart.

**A writable `bdi web` refuses a form posted from another site.** The refusal says which address the form came from. If you put a proxy in front of `bdi web`, it has to pass on the Host the browser asked for, or every change is refused.

**`bdi bd` passes more changes through.** It now takes `close`, `reopen`, `human dismiss`, `unclaim --if-assignee` and `update --priority` as well as the comment, response and deferral it took before.

**A claim whose pane holds another agent now says so.** The bead reads "claimed · its pane holds another agent session", where before it looked like a seat that had never registered. `bdi --beads` lists the same reason beside each bead.

## Upgrading

**Nothing changes until you pass `--writes`.** The keys in the terminal are new, and `bdi web` stays read-only unless you start it with `--writes` and set `[web] actor`.

## Maintenance

A demo of `bdi web` over invented beads that you run with `nix run github:CodeForBreakfast/beady-eye#web-demo`, a lighter git in the container image, and dependency bumps.
