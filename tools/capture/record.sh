#!/usr/bin/env bash
# The README's animation, recorded from the `bdi` in this tree.
#
#   nix develop -c cargo build --release
#   nix shell nixpkgs#vhs nixpkgs#gifsicle -c tools/capture/record.sh
#
# Run it after a change to what `bdi` draws, because nothing else in this
# repository checks the animation against the binary.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
out="$repo/docs/bdi-at-work.gif"

# The runtime directory is short and fresh. `bdi` binds $XDG_RUNTIME_DIR/beady-eye/changes.sock, so a
# socket left from an earlier run draws a notice over the recording, and a
# path near `sockaddr_un`'s ~108 bytes fails the bind and draws another.
ground=$(mktemp -d /tmp/bdi-ground.XXXX)
runtime=$(mktemp -d /tmp/bdi-rt.XXXX)
trap 'rm -rf "$ground" "$runtime"' EXIT

"$repo/tools/capture/ground.sh" "$ground"
# shellcheck source=/dev/null
. "$ground/isolate"

# Once bdi first shows what an agent is saying, arkham's work moves on under
# it: a new agent takes a bead, and the search index's agent finishes a bead
# and takes the next.
#
# Each step waits for herdr to take it, so the tracker bdi reads next agrees
# with the agents herdr lists. The steps start soon after bdi first shows a
# pane's tail.
arkham="$ground/arkham"
step() { # commands
  printf '%s\n' "$1" >"$ground/herdr/next" && mv "$ground/herdr/next" "$ground/herdr/due"
  for _ in $(seq 600); do [ -e "$ground/herdr/taken" ] && break; sleep 0.1; done
  rm -f "$ground/herdr/taken"
}
(
  for _ in $(seq 1200); do [ -e "$ground/herdr/read" ] && break; sleep 0.1; done
  sleep 0.5
  step ". '$ground/isolate' && cd '$arkham' && bd update ark-1.2 --status in_progress --set-metadata agent_pane=wG:p4 >/dev/null &&
    cp '$ground/herdr/keys.json' '$ground/herdr/default.json'"
  sleep 1.5
  step ". '$ground/isolate' && cd '$arkham' && bd update ark-2.1 --unset-metadata agent_pane >/dev/null &&
    bd close ark-2.1 >/dev/null &&
    bd update ark-2.2 --status in_progress --set-metadata agent_pane=wG:p3 >/dev/null"
) </dev/null >"$ground/arkham.log" 2>&1 &
disown

# HOME stays the caller's here, and the tape points it at the ground inside the
# recorded shell. Under the ground's HOME, vhs draws the terminal too narrow.
export DEMO_GROUND="$ground" XDG_RUNTIME_DIR="$runtime"
export PATH="$ground/bin:$repo/target/release:$PATH"

cd "$repo"
vhs tools/capture/demo.tape
gifsicle -O3 --colors 32 --batch "$out"
echo "wrote docs/bdi-at-work.gif, $(wc -c <"$out") bytes"
