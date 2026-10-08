#!/usr/bin/env python3
"""Run one watcher and five boards against the trackers `setup.py` built,
with the large tracker written to constantly, and measure what each costs.

    bench/boards/run.py --root ~/bdi-bench --bdi target/release/bdi --label before

Each write is followed by a producer's line to the watcher naming the
project, as a bd wrapper would send. After a warm-up the run samples every
process once a second, then stops everything it started and prints one row
per process: its CPU, its children's CPU (the watcher's are bd), and its
resident set. bd's calls are counted from a shim that logs each one. The
samples and the summary are kept under `<root>/runs/<label>`.

Every board runs in a pty of its own, in a project's directory, on that
project's board root, as `bdi <root>` would in a terminal. herdr is a shim
answering from canned files, so nothing here reaches a real session.
"""

import argparse
import json
import os
import pty
import random
import select
import shutil
import signal
import socket
import statistics
import struct
import subprocess
import sys
import termios
import fcntl
import threading
import time
from pathlib import Path

from setup import start_server

TICK = os.sysconf("SC_CLK_TCK")

# How long a process this run started may take to go after SIGTERM.
STOP_PATIENCE = 15

# project the board reads, as in setup.py's PROJECTS
BOARDS = ["kadath", "arkham", "arkham", "dunwich", "ferry"]

HERDR = """#!/usr/bin/env bash
case "$*" in
  "session list --json") cat "$BENCH_HERDR/sessions.json" ;;
  *"agent list"*) cat "$BENCH_HERDR/agents.json" ;;
  *"agent read"*) cat "$BENCH_HERDR/read.txt" ;;
esac
"""

# Logs each call's start, wall, user and system seconds, directory and
# arguments, and passes everything else through.
BD = """#!/usr/bin/env bash
start=$EPOCHREALTIME
TIMEFORMAT="%R %U %S"
exec 3>&2
{{ time {real} "$@" 2>&3 ; }} 2>"$BENCH_BD_LOG.$$"
status=$?
printf '%s %s %s %s\\n' "$start" "$(<"$BENCH_BD_LOG.$$")" "${{PWD##*/}}" "$*" >> "$BENCH_BD_LOG"
rm -f "$BENCH_BD_LOG.$$"
exit $status
"""


def clean_env():
    drop = ("BEADS_", "BD_", "HERDR_", "BDI_")
    e = {k: v for k, v in os.environ.items() if not k.startswith(drop)}
    for k in ("DISPLAY", "WAYLAND_DISPLAY"):
        e.pop(k, None)
    e["BEADS_ACTOR"] = "Mira Vance"
    return e


def listening(port):
    try:
        socket.create_connection(("127.0.0.1", port), timeout=0.2).close()
        return True
    except OSError:
        return False


def exited(pid):
    """Whether a child has exited, leaving it unreaped."""
    return os.waitid(os.P_PID, pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None


def cpu_and_rss(pid):
    """(self ticks, children ticks, rss kB), or nothing once it has gone."""
    try:
        stat = Path(f"/proc/{pid}/stat").read_text()
        status = Path(f"/proc/{pid}/status").read_text()
    except OSError:
        return None
    fields = stat.rsplit(")", 1)[1].split()
    utime, stime, cutime, cstime = (int(x) for x in fields[11:15])
    # A process that has exited and not been reaped has no VmRSS.
    rss = next((int(line.split()[1]) for line in status.splitlines() if line.startswith("VmRSS")), None)
    if rss is None:
        return None
    return utime + stime, cutime + cstime, rss


class Bench:
    def __init__(self, opts):
        self.opts = opts
        self.root = opts.root.resolve()
        self.out = self.root / "runs" / opts.label
        if self.out.exists():
            sys.exit(f"{self.out} exists; choose another --label")
        self.out.mkdir(parents=True)
        self.roots = json.loads((self.root / "roots.json").read_text())
        self.port = int((self.root / "port").read_text())
        self.started = []
        # Holding each Popen stops subprocess reaping its child behind stop().
        self.popens = []
        self.writers = []
        self.failures = []
        self.stopping = threading.Event()
        self.real_bd = shutil.which("bd")

    def env(self):
        e = clean_env()
        e["XDG_RUNTIME_DIR"] = str(self.runtime)
        e["PATH"] = f"{self.bin}:{e['PATH']}"
        e["BENCH_HERDR"] = str(self.root / "herdr")
        e["BENCH_BD_LOG"] = str(self.out / "bd.log")
        e["TERM"] = "xterm-256color"
        return e

    def prepare(self):
        self.runtime = self.root / "rt"
        shutil.rmtree(self.runtime, ignore_errors=True)
        self.runtime.mkdir(mode=0o700)
        self.bin = self.root / "bin"
        self.bin.mkdir(exist_ok=True)
        (self.bin / "herdr").write_text(HERDR)
        (self.bin / "bd").write_text(BD.format(real=self.real_bd))
        for shim in ("herdr", "bd"):
            (self.bin / shim).chmod(0o755)
        config = (self.root / "config.toml").read_text()
        if self.opts.events_journal:
            config = config.replace('environment_command = "env"',
                                    'environment_command = "env"\nevents_journal = true')
        self.config = self.out / "config.toml"
        self.config.write_text(config)

    def server(self):
        if not listening(self.port):
            dolt = start_server(self.root, self.port)
            self.popens.append(dolt)
            self.started.append(("dolt", dolt.pid))

    def watcher(self):
        log = open(self.out / "watcher.log", "w")
        proc = subprocess.Popen([self.opts.bdi, "watch", "--config", str(self.config)],
                                cwd=self.root, stdout=log, stderr=subprocess.STDOUT, env=self.env(),
                                start_new_session=True)
        self.popens.append(proc)
        self.started.append(("watcher", proc.pid))
        self.watcher_socket = self.runtime / "beady-eye" / "watcher.sock"
        while not self.watcher_socket.exists():
            if exited(proc.pid):
                sys.exit(f"the watcher exited; see {self.out / 'watcher.log'}")
            time.sleep(0.1)

    def boards(self):
        self.masters = []
        for n, project in enumerate(BOARDS[: self.opts.boards], 1):
            pid, master = pty.fork()
            if pid == 0:
                os.chdir(self.root / "projects" / project)
                argv = [self.opts.bdi, "--config", str(self.config), self.roots[project]]
                os.execve(self.opts.bdi, argv, self.env())
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 60, 220, 0, 0))
            self.started.append((f"board{n}:{project}", pid))
            self.masters.append(master)
        threading.Thread(target=self.drain, daemon=True).start()

    def drain(self):
        """Read every board's terminal for as long as it writes, since a
        board blocked in a write stops drawing."""
        open_ = list(self.masters)
        while open_:
            ready, _, _ = select.select(open_, [], [], 1)
            for fd in ready:
                try:
                    if not os.read(fd, 65536):
                        open_.remove(fd)
                except OSError:
                    open_.remove(fd)

    def ping(self, project):
        try:
            with socket.socket(socket.AF_UNIX) as s:
                s.settimeout(5)
                s.connect(str(self.watcher_socket))
                s.sendall(f"{project}\n".encode())
                s.recv(256)
        except OSError:
            pass

    def writer(self, project, every):
        """Write to `project` every `every` seconds and tell the watcher."""
        rng = random.Random(project)
        cwd = self.root / "projects" / project
        rows = [json.loads(line) for line in (self.root / f"{project}.jsonl").read_text().splitlines()]
        live = [r["id"] for r in rows if r["status"] in ("open", "blocked")]
        created = []
        n = 0
        while not self.stopping.wait(every):
            n += 1
            bead = rng.choice(live)
            if n % 6 == 0:
                args = ["create", "--silent", "--title", f"bench write {n}", "--parent",
                        self.roots[project], "-d", "made by the benchmark's writer"]
            elif n % 6 == 3 and created:
                args = ["close", created.pop(0)]
            elif n % 2:
                args = ["note", bead, f"bench note {n}: " + "the tide turns " * 20]
            else:
                args = ["update", bead, "--priority", str(rng.choice([1, 2, 3]))]
            try:
                done = subprocess.run([self.real_bd, *args], cwd=cwd, env=clean_env(),
                                      capture_output=True, text=True, timeout=60)
            except subprocess.TimeoutExpired:
                self.failures.append(f"bd {' '.join(args[:2])} in {project} took over a minute")
                return
            if done.returncode:
                self.failures.append(f"bd {' '.join(args[:2])} in {project}: {done.stderr.strip()}")
                return
            if args[0] == "create":
                created.append(done.stdout.strip())
            self.ping(project)

    def sample(self):
        with open(self.out / "samples.jsonl", "w") as f:
            while not self.stopping.is_set():
                now = time.time()
                row = {"t": now, "p": {}}
                for name, pid in self.started:
                    got = cpu_and_rss(pid)
                    if got:
                        row["p"][name] = got
                f.write(json.dumps(row) + "\n")
                f.flush()
                self.stopping.wait(1 - (time.time() - now) % 1)

    def stop(self):
        """Stop everything this run started, killing whatever has not gone
        within its patience: a board busy collecting can take a long time to
        answer SIGTERM. The watcher and each board lead a session of their
        own, so the bd and herdr calls they have in flight go with them."""
        self.stopping.set()
        for writer in self.writers:
            writer.join()

        def signal_(name, pid, sig):
            try:
                (os.kill if name == "dolt" else os.killpg)(pid, sig)
            except ProcessLookupError:
                pass

        for name, pid in reversed(self.started):
            signal_(name, pid, signal.SIGTERM)
        giving_up = time.time() + STOP_PATIENCE
        for name, pid in self.started:
            killed = False
            while not exited(pid):
                if time.time() > giving_up and not killed:
                    print(f"{name} did not stop on SIGTERM and was killed", file=sys.stderr)
                    signal_(name, pid, signal.SIGKILL)
                    killed = True
                time.sleep(0.1)
            # Its pid stays reserved until it is reaped, so the group is
            # still its own.
            if name != "dolt":
                signal_(name, pid, signal.SIGKILL)
            os.waitpid(pid, 0)

    def go(self):
        self.prepare()
        try:
            self.server()
            self.watcher()
            if self.opts.boards:
                self.boards()
            (self.out / "pids.json").write_text(json.dumps(self.started))
            threading.Thread(target=self.sample, daemon=True).start()
            writing = [("kadath", self.opts.write_every), ("arkham", self.opts.second_write_every)]
            for project, every in writing:
                if every:
                    writer = threading.Thread(target=self.writer, args=(project, every))
                    writer.start()
                    self.writers.append(writer)
            self.window = (time.time() + self.opts.warmup,
                           time.time() + self.opts.warmup + self.opts.measure)
            time.sleep(self.opts.warmup + self.opts.measure)
            self.failures += [f"{name} exited during the run" for name, pid in self.started if exited(pid)]
        finally:
            self.stop()
        if self.failures:
            sys.exit("the run is not a measurement:\n" + "\n".join(self.failures))
        report(self.out, self.window)


def report(out, window):
    start, end = window
    samples = [json.loads(line) for line in (out / "samples.jsonl").read_text().splitlines()]
    inside = [s for s in samples if start <= s["t"] <= end]
    names = list(inside[0]["p"]) if inside else []
    rows = {}
    for name in names:
        series = [(s["t"], *s["p"][name]) for s in inside if name in s["p"]]
        if len(series) < 2:
            print(f"{name}: too few samples to measure", file=sys.stderr)
            continue
        (t0, c0, k0, _), (t1, c1, k1, _) = series[0], series[-1]
        span = t1 - t0
        per_second = [
            100 * ((b[1] + b[2]) - (a[1] + a[2])) / TICK / (b[0] - a[0])
            for a, b in zip(series, series[1:])
        ]
        rss = [r for *_, r in series]
        rows[name] = {
            "cpu_pct": round(100 * (c1 - c0) / TICK / span, 1),
            "children_cpu_pct": round(100 * (k1 - k0) / TICK / span, 1),
            "peak_cpu_pct_1s": round(max(per_second), 0),
            "rss_mean_mb": round(statistics.mean(rss) / 1024),
            "rss_peak_mb": round(max(rss) / 1024),
        }
    calls = {}
    log = out / "bd.log"
    for line in log.read_text().splitlines() if log.exists() else []:
        at, wall, user, system, project, *args = line.split()
        try:
            at, wall, user, system = float(at), float(wall), float(user), float(system)
        except ValueError:
            # bash's `time` now and then prints a garbled field
            continue
        if not start <= at <= end:
            continue
        words = [a for a in args if not a.startswith(("-", "/"))]
        verb = "probe" if words[0] == "sql" else " ".join(words[:2] if words[0] == "query" else words[:1])
        key = f"{project} {verb}"
        c = calls.setdefault(key, {"calls": 0, "wall_s": 0.0, "cpu_s": 0.0})
        c["calls"] += 1
        c["wall_s"] += wall
        c["cpu_s"] += user + system
    summary = {"seconds": end - start, "processes": rows, "bd": calls}
    (out / "summary.json").write_text(json.dumps(summary, indent=2))
    print(f"{'process':<18} {'cpu%':>6} {'child%':>7} {'peak%':>6} {'rss MB':>7} {'peak MB':>8}")
    for name, r in rows.items():
        print(f"{name:<18} {r['cpu_pct']:>6} {r['children_cpu_pct']:>7} {r['peak_cpu_pct_1s']:>6}"
              f" {r['rss_mean_mb']:>7} {r['rss_peak_mb']:>8}")
    print(f"\nbd calls in {end - start:.0f}s")
    for key, c in sorted(calls.items()):
        print(f"{key:<40} {c['calls']:>4} calls {c['wall_s'] / c['calls']:>6.2f}s wall"
              f" {c['cpu_s'] / c['calls']:>6.2f}s cpu each")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, required=True, help="the directory setup.py built")
    parser.add_argument("--bdi", required=True, help="the bdi binary to measure")
    parser.add_argument("--label", required=True)
    parser.add_argument("--boards", type=int, default=len(BOARDS))
    parser.add_argument("--warmup", type=int, default=90)
    parser.add_argument("--measure", type=int, default=300)
    parser.add_argument("--write-every", type=float, default=12)
    parser.add_argument("--second-write-every", type=float, default=20)
    parser.add_argument("--events-journal", action="store_true",
                        help="claim events_journal for every project (the trackers must keep one)")
    opts = parser.parse_args()
    if opts.measure < 10:
        parser.error("--measure needs at least 10 seconds")
    opts.bdi = str(Path(opts.bdi).resolve())
    Bench(opts).go()


if __name__ == "__main__":
    main()
