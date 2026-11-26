---
name: using-beady-eye
description: Use to subscribe to a bead's changes instead of sleeping and re-reading it, when a session waits on a bead it cannot move itself, such as a question it asked, another agent's work or a pull request merging, and when a `<channel source="beady-eye">` block arrives. Covers what to watch, how to act on a change, what to do when the watcher is down, and unwatching a bead once the session is done waiting on it.
---

# Using beady-eye

The server's instructions say what the tools do and what each block means.
This is how to use them well.

## What to watch

Watch a bead when the session's next step waits on a change to it that someone
else makes: an answer to a question the session put on a bead, a bead its work
depends on closing, a comment or close on a bead it handed on, or a pull
request merging. The work the session holds is already watched, so watch it
yourself only to keep watching it once you let it go.

**Watch a pull request through its gh:pr gate, not through the bead it holds
back.** Where the waiting bead depends on a molecule with the gate as one of its
steps, the waiting bead turns ready only when the molecule's last step closes,
so watching it misses the merge. A pull request closed without merging leaves
its gate open and comments on the bead the gate blocks, so watch that bead as
well.

Do not poll a watched bead with `bd`, and do not schedule a check on it.

Name the project when you know it. A bare id makes the watcher search every
project it reads, and it refuses where more than one holds the id.

## Acting on a change

A change arrives like this:

```
<channel source="beady-eye" project="summit-works" id="smt-4kd3p.20" status="closed" ready="false">
smt-4kd3p.20 in summit-works, "the daily wallpaper timer calls dms", has changed.
- Its status went from blocked to closed, with the reason "Answered: keep the timer".
- "Mira Vance" commented: "Keep the timer and drop the dms call."
</channel>
```

The attributes say where the bead stands, and the lines say why. One message
covers everything one write did, so an answer that comments and closes arrives
once. Act on the change, then decide whether the session still waits on the
bead.

A change of status the session made itself comes back as a message too, so do
not act on it twice.

## When the watcher is down

Do not start polling between `watcher="down"` and `watcher="answering"`. If the
work cannot wait, read the bead once with `bd`. Comments made in the gap arrive
as a count, so read them with `bd` once the watcher is back.

After `watches="unreadable"`, watch again each bead the session still waits on.
After `watches="unkept"`, the next `watch` or `unwatch` that is kept keeps the
beads it names. After `watch="refused"`, stop waiting on that bead, or watch it
again once the watcher can.

## Unwatching

Once the session no longer waits on a bead, unwatch it. Before a session ends,
run `watching` and unwatch every bead it is done with. A bead watched because
the session holds it needs no unwatching, because letting the work go ends the
watch.
