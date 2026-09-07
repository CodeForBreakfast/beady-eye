#!/usr/bin/env bash
# One `bdi` frame at a chosen size, captured raw.
#
# The runtime directory is this capture's own and is made fresh. `bdi` binds
# $XDG_RUNTIME_DIR/beady-eye/changes.sock, so a directory holding a socket
# from any earlier run — this one's or another seat's — draws a notice about
# another bdi having the inbound channel. A notice is not a line added to the
# frame: `status_bar` yields notices last, so at these widths one silently
# takes the key row off the screen instead.
#
# It is short for the same reason it is fresh. A path near `sockaddr_un`'s
# ~108 bytes fails the bind and draws a different notice.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
rows=${1:?rows}
cols=${2:?columns}
out=${3:?where to write the raw stream}
ground=${4:-/tmp/bdi-frame-ground}

runtime=$(mktemp -d /tmp/bdi-rt.XXXX)
trap 'rm -rf "$runtime"' EXIT

"$repo/tools/capture/ground.sh" "$ground" >/dev/null
# shellcheck source=/dev/null
source "$ground/environment"
export XDG_RUNTIME_DIR="$runtime"

# Keys are typed a moment in, so the first collection has been drawn before
# anything is pressed: a key that arrives before the first frame is refused.
# Stdin is held open afterwards rather than closed, because `script` exits
# when its own stdin ends and would take the frame with it.
keys=${5:-}

cd "$ground"
{
  sleep 3
  printf '%b' "$keys"
  sleep 3
} | timeout 15 script -q \
  -c "stty rows $rows cols $cols; $repo/target/release/bdi" /dev/null >"$out" || true

if [ -s "$ground/bd-unanswered" ]; then
  echo "bd was asked what the ground has no answer for:" >&2
  cat "$ground/bd-unanswered" >&2
  exit 1
fi

echo "captured $(wc -c <"$out") bytes into $out"
