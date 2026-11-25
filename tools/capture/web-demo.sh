#!/usr/bin/env bash
# `bdi web` over the invented ground ground.sh lays, with arkham's beads
# moving under the open page on a fixed cycle, for recording the page as it
# updates.
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: web-demo [--listen ADDR]

Serves bdi web over invented beads on ADDR, 127.0.0.1:8080 unless given,
and moves arkham's beads under the open page the same way on every run.
Laying the beads takes about half a minute before the page answers.

One cycle runs 24 seconds and repeats until Ctrl-C:

   0s  the beads are put back as they started
   6s  the refund path closes, so cutting the live keys over goes ready
  12s  an agent takes cutting the live keys over
  18s  invalidating the index on write closes

Each step is printed as it happens. The address it prints holds every tree
open, so no tree folds away while its beads move.
USAGE
}

listen=127.0.0.1:8080
while [ $# -gt 0 ]; do
  case "$1" in
    --listen) listen=${2:?--listen takes an address}; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ground=$(mktemp -d /tmp/bdi-ground.XXXX)
web=
# bdi web's own bd reads write under the ground, so they are stopped and
# waited for before it is removed.
stop() {
  if [ -n "$web" ]; then
    kill -- -"$web" 2>/dev/null || true
    wait "$web" 2>/dev/null || true
  fi
  rm -rf "$ground"
}
trap stop EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

echo "Laying the invented ground..."
"$here/ground.sh" "$ground"
# shellcheck source=/dev/null
. "$ground/isolate"
cd "$ground/arkham"
bd dep add ark-1.2 ark-1.1 >/dev/null

# Job control puts bdi web in a process group of its own, which stop() ends
# whole.
set -m
"$ground/web" --listen "$listen" &
web=$!
set +m
echo "Open http://$listen/?open=b%3Aarkham%3Aark-1&open=b%3Aarkham%3Aark-2&open=b%3Adunwich%3Adun-1"

# The bd actor of the agent that takes the live keys. A claim makes it the
# bead's assignee.
agent=live-keys

at() { # seconds into the cycle, then what happens then
  local due=$((start + $1)) now
  shift
  now=$(date +%s)
  [ "$due" -le "$now" ] || sleep $((due - now))
  [ $# -eq 0 ] || printf '%3ss  %s\n' "$(($(date +%s) - start))" "$*"
}

while kill -0 "$web" 2>/dev/null; do
  start=$(date +%s)
  at 0 "the beads are put back as they started"
  bd update ark-1.1 ark-2.1 --status in_progress >/dev/null
  BEADS_ACTOR=$agent bd update ark-1.2 --status open --assignee "" \
    --unset-metadata agent_pane >/dev/null
  at 6 "the refund path closes, so cutting the live keys over goes ready"
  bd close ark-1.1 >/dev/null
  at 12 "an agent takes cutting the live keys over"
  BEADS_ACTOR=$agent bd update ark-1.2 --claim --set-metadata agent_pane=wG:p4 >/dev/null
  at 18 "invalidating the index on write closes"
  bd close ark-2.1 >/dev/null
  at 24
done
wait "$web"
