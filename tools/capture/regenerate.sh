#!/usr/bin/env bash
# The README's picture, redrawn from the `bdi` in this tree.
#
#   nix develop -c cargo build --release
#   tools/capture/regenerate.sh
#
# Run it after a change to what `bdi` draws. The picture is the first thing a
# stranger sees, and nothing else in this repository checks it against the
# binary: the README carried a key row `bdi` had stopped drawing, and every
# gate stayed green through it.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

# 88 columns is the README's width: wide enough that the agent bloc and the
# progress counts are not cut, narrow enough to read at GitHub's column.
# 26 rows is the frame the forest fills without leaving dead screen under it.
ROWS=26
COLS=88

# Down twice, so the selection is on a bead an agent is working and the band
# below carries that pane's text. A frame with nothing selected draws the band
# empty and says so, which is a truthful picture of less.
KEYS='\033[B\033[B'

raw=$(mktemp /tmp/bdi-frame.XXXX)
trap 'rm -f "$raw"' EXIT

"$repo/tools/capture/capture.sh" "$ROWS" "$COLS" "$raw" /tmp/bdi-ground "$KEYS"
python3 "$repo/tools/capture/frame.py" "$raw" \
  --rows "$ROWS" --cols "$COLS" \
  --svg "$repo/docs/bdi-frame.svg" \
  --title "bdi drawing the atlas project: twelve beads over two roots, three with a live agent beside them, and one claim with no pane behind it"

echo "wrote docs/bdi-frame.svg"
