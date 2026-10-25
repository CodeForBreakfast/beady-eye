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

# HOME stays the caller's here, and the tape points it at the ground inside the
# recorded shell. Under the ground's HOME, vhs draws the terminal too narrow.
export DEMO_GROUND="$ground" XDG_RUNTIME_DIR="$runtime"
export PATH="$ground/bin:$repo/target/release:$PATH"

cd "$repo"
vhs tools/capture/demo.tape
gifsicle -O3 --colors 32 --batch "$out"
echo "wrote docs/bdi-at-work.gif, $(wc -c <"$out") bytes"
