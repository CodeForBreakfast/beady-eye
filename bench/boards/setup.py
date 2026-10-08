#!/usr/bin/env python3
"""Build the invented trackers the boards benchmark reads.

One local `dolt sql-server` holds a database for each of sixteen projects.
Each project gets a directory with a server-mode `.beads/`, filled from
generated JSONL by `bd import`. The shape follows a long-lived tracker: most
beads closed, epics nesting their work, about two in three beads under a
parent, blockers between siblings, and long descriptions. Every name in it is
invented.

    bench/boards/setup.py --root ~/bdi-bench

The build takes about an hour, nearly all of it bd importing the large
tracker. `run.py` takes the same `--root`. Setup refuses a root that already
exists, so remove the directory to build again.
"""

import argparse
import json
import os
import random
import shutil
import socket
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent

# name, prefix, beads. The first is the large tracker the writer works on.
PROJECTS = [
    ("kadath", "kad", 7000),
    ("arkham", "ark", 3000),
    ("dunwich", "dun", 4000),
    ("ferry", "fry", 2500),
    ("innsmouth", "inn", 1700),
]

# Projects nobody writes to and no board is on, each generated from a seed of
# its own. The watcher still polls each one, and every answer it gives goes to
# every board.
QUIET = [
    ("celephais", "cel", 900), ("ulthar", "ult", 700), ("sarnath", "sar", 600),
    ("leng", "len", 500), ("yuggoth", "yug", 400), ("hatheg", "hat", 350),
    ("ilarnek", "ila", 300), ("thalarion", "tha", 250), ("zar", "zar", 200),
    ("mnar", "mna", 150), ("lomar", "lom", 100),
]

# Open beads in arkham blocked by open beads in kadath, so a board on arkham
# reads kadath too.
ACROSS = 12

WORDS = (
    "the hull survey harbour lantern ledger bearing parser socket guard tide "
    "chart beacon ferry crossing lock keeper index cache writer reader probe "
    "window frame column badge row tree root epic child blocker pane agent "
    "status ready closed open claim lease journal record answer freshness "
    "watch board line batch draw fold elide anomaly config path project "
    "measure timing budget queue thread channel signal retry backoff"
).split()

PEOPLE = ["Mira Vance", "Tobias Reed", "Ines Calloway", "Oren Pike"]

EPOCH = datetime(2026, 3, 1, tzinfo=timezone.utc)


def prose(rng, mean):
    """About `mean` characters of words, in paragraphs."""
    length = max(20, int(rng.lognormvariate(0, 0.8) * mean / 1.37))
    out, n = [], 0
    while n < length:
        sentence = " ".join(rng.choice(WORDS) for _ in range(rng.randint(6, 18)))
        out.append(sentence.capitalize() + ".")
        n += len(sentence) + 2
        if rng.random() < 0.2:
            out.append("\n\n")
    return " ".join(out).strip()


def stamp(at):
    return at.strftime("%Y-%m-%dT%H:%M:%SZ")


def base36(n, width=4):
    digits = "0123456789abcdefghijklmnopqrstuvwxyz"
    s = ""
    while n:
        n, r = divmod(n, 36)
        s = digits[r] + s
    return s.rjust(width, "0")


def generate(name, prefix, count, rng):
    """Every bead of one project, oldest first, as bd exports them."""
    beads, epics = [], []
    children = {}
    used = set()

    def fresh_root_id():
        while True:
            candidate = f"{prefix}-{base36(rng.randrange(36 ** 4))}"
            if candidate not in used:
                used.add(candidate)
                return candidate

    def bead(i, parent):
        at = EPOCH + timedelta(minutes=i * 200000 / count + rng.random() * 30)
        if parent is None:
            ident = fresh_root_id()
        else:
            children[parent] = children.get(parent, 0) + 1
            ident = f"{parent}.{children[parent]}"
            used.add(ident)
        return ident, at

    # The board's root: an epic kept open, which a board is started on.
    board_root, at = bead(0, None)
    beads.append({"id": board_root, "issue_type": "epic", "at": at, "parent": None, "keep_open": True})
    epics.append(board_root)
    board_epics = [board_root]

    for i in range(1, count):
        roll = rng.random()
        if roll < 0.28:
            parent = None
        elif roll < 0.36:
            parent = rng.choice(board_epics[-12:])
        else:
            parent = rng.choice(epics[-30:])
        ident, at = bead(i, parent)
        is_epic = rng.random() < (0.15 if parent is None else 0.06)
        kind = "epic" if is_epic else rng.choice(["task"] * 5 + ["bug"] * 2 + ["feature"])
        beads.append({"id": ident, "issue_type": kind, "at": at, "parent": parent})
        if is_epic:
            epics.append(ident)
            if parent in board_epics:
                board_epics.append(ident)

    rows = []
    recent = int(count * 0.85)
    # A bead waits only on the sibling just before it. A blocker anywhere else
    # can close a loop through the nesting back to the bead it blocks, which
    # the real trackers this copies never have. A tree draws a bead once for
    # each path to it, and blockers on further siblings multiply the paths far
    # beyond the two or three draws per bead that real trees show.
    siblings = {}
    for i, b in enumerate(beads):
        earlier = siblings.setdefault(b["parent"], [])[-1:]
        siblings[b["parent"]].append(b["id"])
        if b.get("keep_open"):
            status = "open"
        elif i < recent:
            status = "closed" if rng.random() < 0.97 else "open"
        else:
            status = rng.choices(
                ["closed", "open", "in_progress", "blocked"], [55, 33, 7, 5]
            )[0]
        at = b["at"]
        updated = at + timedelta(hours=rng.random() * 72)
        row = {
            "_type": "issue",
            "id": b["id"],
            "title": prose(rng, 50).rstrip(".")[:90],
            "description": prose(rng, 2200),
            "status": status,
            "priority": rng.choice([1, 2, 2, 2, 3]),
            "issue_type": b["issue_type"],
            "owner": "mira@example.com",
            "created_at": stamp(at),
            "created_by": rng.choice(PEOPLE),
            "updated_at": stamp(updated),
        }
        if rng.random() < 0.3:
            row["notes"] = prose(rng, 1500)
        if status == "closed":
            row["closed_at"] = stamp(updated)
            row["close_reason"] = prose(rng, 120)
        if rng.random() < 0.4:
            row["metadata"] = {"working_topic": f"{name}/{rng.choice(WORDS)}-{i}"}
        deps = []
        if b["parent"]:
            deps.append({"issue_id": b["id"], "depends_on_id": b["parent"], "type": "parent-child",
                         "created_at": stamp(at), "created_by": row["created_by"], "metadata": "{}"})
        if i > 10 and rng.random() < 0.45:
            blockers = set()
            for _ in range(rng.choice([1, 1, 2])):
                pick = rng.randrange(max(0, i - 400), i)
                if earlier and pick % 2 == 0:
                    blockers.add(earlier[0])
            for on in sorted(blockers):
                deps.append({"issue_id": b["id"], "depends_on_id": on, "type": "blocks",
                             "created_at": stamp(at), "created_by": row["created_by"], "metadata": "{}"})
        if deps:
            row["dependencies"] = deps
        rows.append(row)
    return rows, board_root


def seat(rows, rng, prefix, panes, roots):
    """Put a live seat on in-progress beads under each board root, so a
    board draws a tree with an agent in it, and hand back the panes."""
    under = {}
    for row in rows:
        for root in roots:
            if row["id"].startswith(root + ".") and row["status"] != "closed":
                under.setdefault(root, []).append(row)
    seated = []
    for root in roots:
        for row in rng.sample(under.get(root, []), min(3, len(under.get(root, [])))):
            pane = f"w{prefix[0].upper()}:p{len(panes) + 1}"
            row["status"] = "in_progress"
            row.setdefault("metadata", {})["agent_pane"] = pane
            panes.append(pane)
            seated.append((row["id"], pane))
    return seated


def other_live_roots(rows, board_root, count=8):
    """Epics with no parent and open work beneath them, besides the board's
    root, where the large tracker's other seats go: a busy tracker has live
    work in several trees."""
    open_ids = [r["id"] for r in rows if r["status"] != "closed"]
    return [r["id"] for r in rows
            if r["issue_type"] == "epic" and r["id"] != board_root
            and not any(d["type"] == "parent-child" for d in r.get("dependencies", []))
            and any(i.startswith(r["id"] + ".") for i in open_ids)][:count]


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def env():
    """The environment bd runs in here: nothing of the caller's tracker."""
    e = {k: v for k, v in os.environ.items() if not k.startswith(("BEADS_", "BD_"))}
    e["BD_NON_INTERACTIVE"] = "1"
    e["BEADS_ACTOR"] = "Mira Vance"
    return e


def run(args, cwd=None):
    subprocess.run(args, cwd=cwd, env=env(), check=True, stdout=subprocess.DEVNULL)


def start_server(root, port):
    data = root / "dolt"
    data.mkdir(exist_ok=True)
    log = open(root / "dolt.log", "a")
    proc = subprocess.Popen(
        ["dolt", "sql-server", "--host", "127.0.0.1", "--port", str(port), "--data-dir", str(data)],
        cwd=data, stdout=log, stderr=subprocess.STDOUT, env=env(),
    )
    for _ in range(100):
        try:
            socket.create_connection(("127.0.0.1", port), timeout=0.2).close()
            return proc
        except OSError:
            time.sleep(0.2)
    proc.kill()
    sys.exit("dolt sql-server did not start; see dolt.log")


def build(root, port, projects, generated):
    """A server-mode tracker for each of `projects`, holding its generated rows."""
    server = start_server(root, port)
    try:
        for name, prefix, _ in projects:
            project = root / "projects" / name
            project.mkdir(parents=True)
            run(["bd", "init", "--server", "--external", "--server-host", "127.0.0.1",
                 "--server-port", str(port), "--server-user", "root", "--database", name,
                 "--prefix", prefix, "--skip-hooks", "--skip-agents", "--non-interactive", "-q"],
                cwd=project)
            # bd reads the port from .beads/dolt-server.port, which init does
            # not always fill with the port it was given, and warns on every
            # call while metadata.json names it as well.
            (project / ".beads" / "dolt-server.port").write_text(str(port))
            metadata = project / ".beads" / "metadata.json"
            fields = json.loads(metadata.read_text())
            fields.pop("dolt_server_port", None)
            metadata.write_text(json.dumps(fields, indent=2))
            (root / f"{name}.jsonl").write_text("".join(json.dumps(r) + "\n" for r in generated[name]))
        # Each import commits as it goes, and slows as its table grows, so the
        # projects are imported together and committed once at the end.
        started = time.time()
        imports = [subprocess.Popen(["bd", "--dolt-auto-commit", "off", "import", str(root / f"{name}.jsonl")],
                                    cwd=root / "projects" / name, env=env(), stdout=subprocess.DEVNULL)
                   for name, _, _ in projects]
        statuses = [i.wait() for i in imports]
        if any(statuses):
            sys.exit("an import failed")
        for name, _, _ in projects:
            run(["bd", "dolt", "commit", "-m", "imported"], cwd=root / "projects" / name)
            print(f"{name}: {len(generated[name])} beads")
        print(f"imported in {time.time() - started:.0f}s")
    finally:
        server.terminate()
        server.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, required=True,
                        help="a directory of your own; setup creates it")
    parser.add_argument("--scale", type=float, default=1.0, help="multiply every project's size")
    opts = parser.parse_args()
    root = opts.root.resolve()
    if root.exists():
        sys.exit(f"{root} already exists; remove it to build again")
    root.mkdir(parents=True, mode=0o700)
    port = free_port()
    (root / "port").write_text(str(port))

    rng = random.Random(1979)
    panes, roots, generated = [], {}, {}
    for name, prefix, count in PROJECTS:
        rows, board_root = generate(name, prefix, int(count * opts.scale), rng)
        generated[name] = rows
        roots[name] = board_root
    kadath_open = [r["id"] for r in generated["kadath"] if r["status"] != "closed"]
    arkham_under = [r for r in generated["arkham"]
                    if r["id"].startswith(roots["arkham"] + ".") and r["status"] != "closed"]
    for row in rng.sample(arkham_under, min(ACROSS, len(arkham_under))):
        row.setdefault("dependencies", []).append({
            "issue_id": row["id"], "depends_on_id": rng.choice(kadath_open), "type": "blocks",
            "created_at": row["created_at"], "created_by": row["created_by"], "metadata": "{}"})
    seats = {}
    for name, prefix, _ in PROJECTS:
        seats[name] = seat(generated[name], rng, prefix, panes, [roots[name]])
    seats["kadath"] += seat(generated["kadath"], random.Random("kadath seats"), "kad", panes,
                            other_live_roots(generated["kadath"], roots["kadath"]))
    for name, prefix, count in QUIET:
        generated[name], _ = generate(name, prefix, int(count * opts.scale), random.Random(name))

    build(root, port, PROJECTS + QUIET, generated)

    agents = []
    for name, _, _ in PROJECTS:
        for bead, pane in seats[name]:
            agents.append({
                "agent": "claude", "agent_status": "working", "cwd": str(root / "projects" / name),
                "foreground_cwd": str(root / "projects" / name), "focused": False, "pane_id": pane,
                "revision": 1, "state_change_seq": 1, "tab_id": pane.split(":")[0] + ":t1",
                "terminal_id": "term_" + pane.replace(":", ""), "terminal_title": f"✳ Working {bead}",
                "terminal_title_stripped": f"Working {bead}", "workspace_id": pane.split(":")[0],
            })
    herdr = root / "herdr"
    herdr.mkdir()
    (herdr / "sessions.json").write_text(json.dumps({"sessions": [{
        "default": True, "name": "bench", "running": True,
        "session_dir": str(herdr), "socket_path": str(herdr / "herdr.sock")}]}))
    (herdr / "agents.json").write_text(json.dumps({"id": "cli:agent:list", "result": {"agents": agents}}))
    shutil.copy(HERE.parent.parent / "tests" / "fixtures" / "herdr_agent_read_ansi.txt", herdr / "read.txt")

    config = []
    for name, prefix, _ in PROJECTS + QUIET:
        config += ["[[projects]]", f'name = "{name}"', f'path = "{root / "projects" / name}"',
                   'environment_command = "env"', f'prefix = "{prefix}"', ""]
    config += ["[join]", 'pane_key = "agent_pane"', ""]
    (root / "config.toml").write_text("\n".join(config))
    (root / "roots.json").write_text(json.dumps(roots))
    print(f"built under {root}")


if __name__ == "__main__":
    main()
