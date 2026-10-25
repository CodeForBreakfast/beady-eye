#!/usr/bin/env bash
# The invented ground the README's animation is recorded in: beads trackers
# for the arkham and dunwich projects, and a herdr that answers for them and
# nothing else. Once bdi first shows what an agent is saying, arkham's work
# moves on under it: a new agent takes a bead, and the search index's agent
# finishes a bead and takes the next.
#
# Everything here is invented, and has to be — CONTRIBUTING.md's *Only
# invented examples go in this repository*.
#
# Run by record.sh, which hands it an empty directory.
set -euo pipefail

ground=${1:?an empty directory to lay the ground in}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
arkham="$ground/arkham"
dunwich="$ground/dunwich"

if [ -n "$(ls -A "$ground" 2>/dev/null)" ]; then
  echo "$ground is not empty: refusing to lay a ground in it" >&2
  exit 1
fi

mkdir -p "$arkham" "$dunwich" "$ground/bin" "$ground/herdr" \
  "$ground/.config/beady-eye"
cp "$here"/bin/* "$ground/bin/"

# Every `bd` here must reach the trackers laid here and nothing else, both
# below and in the steps herdr takes later. A maintainer's shell points
# BEADS_DIR at a real tracker, and bd prefers it to the directory it runs in,
# so the ground's beads would land there.
cat >"$ground/isolate" <<'SH'
for var in $(env | grep -E '^(BEADS|BD)_' | cut -d= -f1); do
  unset "$var"
done
export BEADS_ACTOR=wilbur
SH
# shellcheck source=/dev/null
. "$ground/isolate"

cat >"$ground/.config/beady-eye/config.toml" <<TOML
[tui]
refresh_seconds = 1

# Only arkham is polled. bdi reads one project a look, so a change it saw in
# one project and not yet in the other would be drawn half made.
[[projects]]
name = "arkham"
path = "$arkham"
poll = true

[[projects]]
name = "dunwich"
path = "$dunwich"
poll = false
TOML

tracker() { # prefix
  git init -q
  bd metrics off >/dev/null 2>&1 || true
  bd init --prefix "$1" --stealth >/dev/null
}
new() { bd create --silent "$@" >/dev/null; }

cd "$arkham"
tracker ark
new --id ark-1 --type epic --title "Payments move to the new gateway"
new --parent ark-1 --type task --status in_progress \
  --title "The refund path calls the gateway" --metadata '{"agent_pane":"wG:p2"}'
new --parent ark-1 --type task --title "Cut the live keys over"
new --parent ark-1 --type task --title "Pin the gateway client version"
bd close ark-1.3 >/dev/null
new --id ark-2 --type bug --title "Search returns stale results after an edit"
new --parent ark-2 --type task --status in_progress \
  --title "Invalidate the index on write" --metadata '{"agent_pane":"wG:p3"}'
new --parent ark-2 --type task --title "Measure the reindex cost"

cd "$dunwich"
tracker dun
new --id dun-1 --type epic --title "dunwich v1: release notes from the commit log"
new --parent dun-1 --type task --status in_progress \
  --title "Group the commits by type" --metadata '{"agent_pane":"wG:p5"}'

cat >"$ground/herdr/sessions.json" <<JSON
{"sessions":[{"default":true,"name":"default","running":true,
  "session_dir":"$ground/herdr","socket_path":"$ground/herdr/herdr.sock"}]}
JSON

agents() { # pane=cwd=label ...
  local sep="" pane cwd label
  printf '{"id":"cli:agent:list","result":{"type":"agent_list","agents":['
  for a in "$@"; do
    IFS== read -r pane cwd label <<<"$a"
    printf '%s{"pane_id":"%s","cwd":"%s","agent_status":"working","agent":"claude","state_labels":{"working":"%s"}}' \
      "$sep" "$pane" "$cwd" "$label"
    sep=,
  done
  printf ']}}\n'
}
refunds="wG:p2=$arkham=refunds" search="wG:p3=$arkham=search index"
keys="wG:p4=$arkham=live keys" notes="wG:p5=$dunwich=release notes"
agents "$refunds" "$search" "$notes" >"$ground/herdr/default.json"
agents "$refunds" "$search" "$notes" "$keys" >"$ground/herdr/keys.json"

cat >"$ground/herdr/visible.wG:p2" <<'PANE'
> the gateway rejects a refund over the original charge, so the
  partial path needs its own call. Writing the failing test first.

  running 3 tests ... 2 passed, 1 failed
PANE
cat >"$ground/herdr/visible.wG:p4" <<'PANE'
> Taking ark-1.2. The live keys sit in the gateway's vault, so
  the cutover reads them at start rather than from the config.

  Reading the gateway's docs on key rotation.
PANE

# Each step waits for herdr to take it, so the tracker bdi reads next agrees
# with the agents herdr lists. The steps start soon after bdi first shows a
# pane's tail.
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
