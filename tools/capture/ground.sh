#!/usr/bin/env bash
# The invented ground the README's frame is drawn from: a `bd` and a `herdr`
# that answer for the atlas project and nothing else.
#
# Everything here is invented, and has to be — CLAUDE.local.md's *Only
# invented examples go in the repo*. The frame this produces ships inside the
# published crate, so a real pane id or cwd that reached it could not be
# withdrawn afterwards.
#
# It answers through `tests/shims/`, which `bdi` finds by a PATH lookup, so
# no tracker is opened and no herdr session is reached. The same scripts the
# suite runs.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ground=${1:?a directory to lay the ground in}

atlas="$ground/atlas"
answers="$ground/bd-answers"
agents="$ground/herdr-agents"

# Laid fresh, not added to. A previous run leaves its `bd-unanswered` behind,
# and that file is what `capture.sh` reads to decide the capture was served —
# so a ground that is only ever `mkdir -p`ed makes one run's unanswered call
# fail every run after it. An answer file that outlives the fixture it was
# written for is the quieter half: the shim serves it, the frame draws, and
# the picture is of a tracker this script no longer describes.
#
# Only a directory this script wrote is cleared, and the marker is the last
# file it writes. The argument is a path from whoever ran it, so mistyping it
# would otherwise hand `rm -rf` a directory that was never a ground.
if [ -e "$ground" ] && [ ! -e "$ground/environment" ]; then
  echo "$ground exists and is not a capture ground: refusing to clear it" >&2
  exit 1
fi
rm -rf "$ground"
mkdir -p "$atlas" "$answers" "$agents" "$ground/.config/beady-eye"

cat >"$ground/.config/beady-eye/config.toml" <<TOML
[[projects]]
name = "atlas"
path = "$atlas"
TOML

# The tracker. Twelve beads over two roots: a payments epic whose refund
# strand is being worked, and a search bug. Three are closed, three name a
# pane, and atlas-5 names one that no session reports — the drift the caption
# points at.
python3 - "$answers" "$atlas" <<'PY'
import json, sys

answers, atlas = sys.argv[1], sys.argv[2]

def bead(id, title, status, kind, parent=None, blocked_by=None, pane=None):
    row = {
        "id": id,
        "title": title,
        "status": status,
        "priority": 2,
        "issue_type": kind,
        "dependencies": [],
    }
    if parent:
        row["parent"] = parent
        row["dependencies"].append({"depends_on_id": parent, "type": "parent-child"})
    if blocked_by:
        row["dependencies"].append({"depends_on_id": blocked_by, "type": "blocks"})
    if pane:
        row["metadata"] = {"agent_pane": pane}
    return row

rows = [
    bead("atlas-1", "Payments move to the new gateway", "open", "epic"),
    bead("atlas-2", "Pin the gateway client version", "open", "task", "atlas-1"),
    bead("atlas-3", "The refund path calls the gateway", "in_progress", "task",
         "atlas-1", pane="wG:p2"),
    bead("atlas-4", "Backfill the refund ledger", "open", "task", "atlas-3"),
    bead("atlas-5", "Retire the old refund worker", "in_progress", "task",
         "atlas-3", pane="wG:p9"),
    bead("atlas-6", "Cut the live keys over", "open", "task", "atlas-1",
         blocked_by="atlas-2"),
    bead("atlas-7", "Webhook retries are not idempotent", "in_progress", "bug",
         "atlas-1", pane="wG:p4"),
    bead("atlas-8", "Reconcile the settlement report", "closed", "task", "atlas-1"),
    bead("atlas-9", "Drop the gateway shim", "closed", "task", "atlas-1"),
    bead("atlas-10", "Search returns stale results after an edit", "open", "bug"),
    bead("atlas-11", "Invalidate the index on write", "in_progress", "task",
         "atlas-10", pane="wG:p6"),
    bead("atlas-12", "Measure the reindex cost", "closed", "task", "atlas-10"),
]

UNFINISHED = "open,in_progress,blocked,deferred"

def write(call, value):
    with open(f"{answers}/{call}", "w") as f:
        json.dump(value, f)

write("list --all --limit 0 --json", rows)
write(f"list --status {UNFINISHED} --limit 0 --json",
      [r for r in rows if r["status"] != "closed"])
for row in rows:
    write(f"show {row['id']} --json", [row])
for call in ("query ephemeral=true --limit 0 --json",
             "query ephemeral=true --all --limit 0 --json",
             "ready --limit 0 --json",
             "blocked --json"):
    write(call, [])
with open(f"{answers}/where --json", "w") as f:
    json.dump({"database_path": f"{atlas}/.beads/dolt",
               "path": f"{atlas}/.beads", "schema_version": 1}, f)
PY

# The session. Six panes in the project's directory: the three the beads name,
# and three more working there that no bead claims. atlas-5's pane is not
# among them, so the claim it carries has nothing behind it.
cat >"$ground/herdr-sessions.json" <<JSON
{"sessions":[{"default":true,"name":"default","running":true,
  "session_dir":"$ground/herdr","socket_path":"$ground/herdr/herdr.sock"}]}
JSON

cat >"$agents/default.json" <<JSON
{"id":"cli:agent:list","result":{"type":"agent_list","agents":[
  {"pane_id":"wG:p2","cwd":"$atlas","agent_status":"working","agent":"claude"},
  {"pane_id":"wG:p4","cwd":"$atlas","agent_status":"working","agent":"claude"},
  {"pane_id":"wG:p6","cwd":"$atlas","agent_status":"working","agent":"claude"},
  {"pane_id":"wG:p1","cwd":"$atlas","agent_status":"idle","agent":"claude"},
  {"pane_id":"wG:p7","cwd":"$atlas","agent_status":"idle","agent":"claude"},
  {"pane_id":"wG:pB","cwd":"$atlas","agent_status":"working","agent":"claude"}
]}}
JSON

cat >"$ground/herdr-visible" <<'PANE'
> the gateway rejects a refund over the original charge, so the
  partial path needs its own call. Writing the failing test first.

  running 3 tests ... 2 passed, 1 failed
PANE

cat >"$ground/environment" <<ENV
export PATH="$repo/tests/shims:\$PATH"
export HOME="$ground"
export BDI_SHIM_BD_ANSWERS="$answers"
export BDI_SHIM_BD_UNANSWERED="$ground/bd-unanswered"
export BDI_SHIM_HERDR_SESSIONS="$ground/herdr-sessions.json"
export BDI_SHIM_HERDR_AGENTS="$agents"
export BDI_SHIM_HERDR_VISIBLE="$ground/herdr-visible"
ENV

echo "ground laid in $ground"
