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
keys=${5:-}

runtime=$(mktemp -d /tmp/bdi-rt.XXXX)
trap 'rm -rf "$runtime"' EXIT

"$repo/tools/capture/ground.sh" "$ground" >/dev/null
# shellcheck source=/dev/null
source "$ground/environment"
export XDG_RUNTIME_DIR="$runtime"

# `%q` because the path is interpolated into a command line a second shell
# parses: a repository checked out somewhere with a space in it would
# otherwise be split into words and the capture would run something else.
printf -v draw 'stty rows %q cols %q; %q' "$rows" "$cols" "$repo/target/release/bdi"

# Keys are typed a moment in, so the first collection has been drawn before
# anything is pressed: a key that arrives before the first frame is refused.
# Then a settle, then `q` — `bdi` leaves on its own that way, rather than the
# capture waiting out a timeout that is only there as a backstop.
cd "$ground"
{
  sleep 3
  printf '%b' "$keys"
  sleep 2
  printf 'q'
  sleep 1
} | timeout 30 script -q -c "$draw" /dev/null >"$out" || true

# The exit status is not the check, and reaching for it is worse than not
# checking at all: measured here, a healthy capture that the backstop had to
# kill exits **124**, and a run whose binary does not exist exits **0** with
# a shell's error message in the file. `frame.py` would replay that into a
# well-formed picture of nothing. What separates the two is whether `bdi`
# ever took the terminal, so that is what is asked.
if ! grep -q $'\033\[?1049h' "$out"; then
  echo "no frame: bdi never reached the alternate screen. What it wrote:" >&2
  head -c 400 "$out" >&2
  exit 1
fi

# Taking the terminal is not drawing on it. `bdi` can reach the screen and
# then die — a panic, or an exit down a path that returns early — and leave a
# stream that replays into a blank or half-drawn picture over the committed
# one. The change that breaks the draw is exactly the change that sends
# somebody to `regenerate.sh`, so this is the likely case rather than the
# unlucky one.
if grep -qa 'panicked at' "$out"; then
  echo "no frame: bdi panicked. What it said:" >&2
  grep -a -m1 -A3 'panicked at' "$out" >&2
  exit 1
fi

# The key row is asked for because it is the last of the frame's furniture to
# be drawn, so a stream carrying it is one where the draw ran to the end. It
# closes the notice the header describes as well: `status_bar` yields notices
# last, so a notice at these widths takes the key row off the screen rather
# than adding a line, and the picture silently loses a row.
#
# One word rather than the phrase the row reads as, because crossterm moves
# the cursor between them: the row arrives as `[26;27Hq[26;29Hquit`, and
# nothing a person can see on the screen is contiguous in the stream.
if ! grep -qa 'quit' "$out"; then
  echo "no frame: bdi took the terminal and drew no key row. What it wrote:" >&2
  tail -c 400 "$out" >&2
  exit 1
fi

if [ -s "$ground/bd-unanswered" ]; then
  echo "bd was asked what the ground has no answer for:" >&2
  cat "$ground/bd-unanswered" >&2
  exit 1
fi

echo "captured $(wc -c <"$out") bytes into $out"
