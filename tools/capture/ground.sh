#!/usr/bin/env bash
# The invented ground the README's frame is drawn from: a `bd` and a `herdr`
# that answer for the arkham project and nothing else.
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

arkham="$ground/arkham"
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
mkdir -p "$arkham" "$answers" "$agents" "$ground/.config/beady-eye"

cat >"$ground/.config/beady-eye/config.toml" <<TOML
[[projects]]
name = "arkham"
path = "$arkham"
TOML

# The tracker. Twelve beads over two roots: a payments epic whose refund
# strand is being worked, and a search bug. Three are closed, three name a
# pane, and arkham-5 names one that no session reports — the drift the caption
# points at.
python3 - "$answers" "$arkham" <<'PY'
import json, sys

answers, arkham = sys.argv[1], sys.argv[2]

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
    bead("arkham-1", "Payments move to the new gateway", "open", "epic"),
    bead("arkham-2", "Pin the gateway client version", "open", "task", "arkham-1"),
    bead("arkham-3", "The refund path calls the gateway", "in_progress", "task",
         "arkham-1", pane="wG:p2"),
    bead("arkham-4", "Backfill the refund ledger", "open", "task", "arkham-3"),
    bead("arkham-5", "Retire the old refund worker", "in_progress", "task",
         "arkham-3", pane="wG:p9"),
    bead("arkham-6", "Cut the live keys over", "open", "task", "arkham-1",
         blocked_by="arkham-2"),
    bead("arkham-7", "Webhook retries are not idempotent", "in_progress", "bug",
         "arkham-1", pane="wG:p4"),
    bead("arkham-8", "Reconcile the settlement report", "closed", "task", "arkham-1"),
    bead("arkham-9", "Drop the gateway shim", "closed", "task", "arkham-1"),
    bead("arkham-10", "Search returns stale results after an edit", "open", "bug"),
    bead("arkham-11", "Invalidate the index on write", "in_progress", "task",
         "arkham-10", pane="wG:p6"),
    bead("arkham-12", "Measure the reindex cost", "closed", "task", "arkham-10"),
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
    json.dump({"database_path": f"{arkham}/.beads/dolt",
               "path": f"{arkham}/.beads", "schema_version": 1}, f)
PY

# The session. Six panes in the project's directory: the three the beads name,
# and three more working there that no bead claims. arkham-5's pane is not
# among them, so the claim it carries has nothing behind it.
cat >"$ground/herdr-sessions.json" <<JSON
{"sessions":[{"default":true,"name":"default","running":true,
  "session_dir":"$ground/herdr","socket_path":"$ground/herdr/herdr.sock"}]}
JSON

cat >"$agents/default.json" <<JSON
{"id":"cli:agent:list","result":{"type":"agent_list","agents":[
  {"pane_id":"wG:p2","cwd":"$arkham","agent_status":"working","agent":"claude"},
  {"pane_id":"wG:p4","cwd":"$arkham","agent_status":"working","agent":"claude"},
  {"pane_id":"wG:p6","cwd":"$arkham","agent_status":"working","agent":"claude"},
  {"pane_id":"wG:p1","cwd":"$arkham","agent_status":"idle","agent":"claude"},
  {"pane_id":"wG:p7","cwd":"$arkham","agent_status":"idle","agent":"claude"},
  {"pane_id":"wG:pB","cwd":"$arkham","agent_status":"working","agent":"claude"}
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
