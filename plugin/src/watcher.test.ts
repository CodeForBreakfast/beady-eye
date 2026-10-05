import { afterEach, expect, test } from "bun:test";
import {
	chmodSync,
	lstatSync,
	mkdirSync,
	mkdtempSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { createServer, type Server, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
	type Down,
	othersMayTakeANameIn,
	pauseAfter,
	type Said,
	type Timing,
	watchBead,
	whereTheWatcherIs,
} from "./watcher";

const timing: Timing = { wedgedAfter: 200, firstPause: 10, longestPause: 40 };
const bead = { project: "summit-works", id: "smt-4kd3p.20" };
const asked = "watch summit-works smt-4kd3p.20\n";

const beadLine = {
	line: "bead",
	project: "summit-works",
	ready: false,
	blocked_by: ["smt-4kd3p.13"],
	bd: { ready: false, blocked_by: ["smt-4kd3p.13"] },
	row: { id: "smt-4kd3p.20", status: "blocked" },
};
const freshness = {
	line: "freshness",
	project: "summit-works",
	as_of: "2026-08-30T10:22:14Z",
	tracker: "ok",
	events: "off",
	protocol: 1,
	reach: { path: "/home/mira/summit-works", environment_command: [] },
};

const said = (...lines: unknown[]) =>
	lines.map((line) => `${JSON.stringify(line)}\n`).join("");

const cleanUp: (() => void)[] = [];
afterEach(() => {
	for (const clean of cleanUp.splice(0)) clean();
});

const aPrivateDirectory = () => {
	const directory = mkdtempSync(join(tmpdir(), "beady-eye-watcher-"));
	cleanUp.push(() => rmSync(directory, { recursive: true, force: true }));
	return directory;
};

interface Connection {
	readonly connection: Socket;
	readonly asked: string;
}

/** A watcher of the test's own, handing over each connection with the line
 * it asked. */
const aWatcherAt = async (at: string) => {
	const arrived: Connection[] = [];
	const waiting: ((connection: Connection) => void)[] = [];
	const server: Server = createServer((connection) => {
		connection.setEncoding("utf8");
		connection.once("data", (asked) => {
			const accepted = { connection, asked: String(asked) };
			const waiter = waiting.shift();
			if (waiter) waiter(accepted);
			else arrived.push(accepted);
		});
	});
	await new Promise<void>((listening) => server.listen(at, listening));
	cleanUp.push(() => server.close());
	return {
		server,
		next: () =>
			new Promise<Connection>((resolve) => {
				const accepted = arrived.shift();
				if (accepted) resolve(accepted);
				else waiting.push(resolve);
			}),
	};
};

/** A watcher killed where it stood, which leaves its socket at the path with
 * nothing listening. One that closes its server removes the socket. */
const aWatcherThatDied = async (at: string) => {
	const watcher = Bun.spawn(
		[
			process.execPath,
			"-e",
			`require("node:net").createServer().listen(${JSON.stringify(at)}, () => console.log("listening"))`,
		],
		{ stdout: "pipe" },
	);
	await watcher.stdout.getReader().read();
	watcher.kill("SIGKILL");
	await watcher.exited;
};

type Heard = { batch: readonly Said[] } | { down: Down };

/** What a session would be told, in order. */
const aSession = () => {
	const heard: Heard[] = [];
	const waiting: ((heard: Heard) => void)[] = [];
	const hear = (what: Heard) => {
		const waiter = waiting.shift();
		if (waiter) waiter(what);
		else heard.push(what);
	};
	return {
		heard,
		hearing: {
			heard: (batch: readonly Said[]) => hear({ batch }),
			down: (why: Down) => hear({ down: why }),
		},
		next: () =>
			new Promise<Heard>((resolve) => {
				const first = heard.shift();
				if (first) resolve(first);
				else waiting.push(resolve);
			}),
	};
};

const watching = (
	at: string | undefined,
	session: ReturnType<typeof aSession>,
) => {
	const watch = watchBead(bead, () => at, session.hearing, timing);
	cleanUp.push(watch.stop);
	return watch;
};

const pause = (millis: number) =>
	new Promise((resolve) => setTimeout(resolve, millis));

test("the config's watcher socket is where the watcher is", () => {
	const config = join(aPrivateDirectory(), "config.toml");
	writeFileSync(config, '[watcher]\nsocket = "/var/folders/T/watcher.sock"\n');

	expect(whereTheWatcherIs(config, "/run/user/1000")).toBe(
		"/var/folders/T/watcher.sock",
	);
});

test("a config naming no socket leaves the watcher under the runtime directory", () => {
	const directory = aPrivateDirectory();
	const config = join(directory, "config.toml");
	writeFileSync(config, "[tui]\nrefresh_seconds = 30\n");

	for (const named of [config, join(directory, "missing.toml")]) {
		expect(whereTheWatcherIs(named, "/run/user/1000")).toBe(
			"/run/user/1000/beady-eye/watcher.sock",
		);
	}
});

test("with no socket named and no runtime directory there is no watcher to reach", () => {
	const config = join(aPrivateDirectory(), "missing.toml");

	expect(whereTheWatcherIs(config, undefined)).toBeUndefined();
	expect(whereTheWatcherIs(config, "")).toBeUndefined();
});

test("a directory is one others may take a name in by its owner as well as its mode", () => {
	const me = 1000;
	expect(othersMayTakeANameIn(0o700, me, me)).toBe(false);
	expect(othersMayTakeANameIn(0o755, 0, me)).toBe(false);
	expect(othersMayTakeANameIn(0o1777, 0, me)).toBe(false);
	expect(othersMayTakeANameIn(0o755, 1001, me)).toBe(true);
	expect(othersMayTakeANameIn(0o770, me, me)).toBe(true);
	expect(othersMayTakeANameIn(0o707, me, me)).toBe(true);
	expect(othersMayTakeANameIn(0o750, me, me)).toBe(false);
	expect(othersMayTakeANameIn(0o705, me, me)).toBe(false);
});

test("a bead's lines arrive as one batch at its project's freshness line", async () => {
	const at = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(at);
	const session = aSession();
	watching(at, session);

	const { connection, asked: line } = await watcher.next();
	connection.write(said({ line: "alive" }, beadLine, freshness));

	expect(line).toBe(asked);
	expect(await session.next()).toEqual({ batch: [beadLine, freshness] });
});

test("a refusal of the watch arrives on its own", async () => {
	const at = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(at);
	const session = aSession();
	watching(at, session);

	const refused = {
		line: "refused",
		asked: asked.trim(),
		reason: "unknown-project",
	};
	(await watcher.next()).connection.write(said(refused));

	expect(await session.next()).toEqual({ batch: [refused] });
});

test("a refused connection is a watcher that is down", async () => {
	const at = join(aPrivateDirectory(), "watcher.sock");
	await aWatcherThatDied(at);
	expect(lstatSync(at).isSocket()).toBe(true);
	const session = aSession();
	watching(at, session);

	expect(await session.next()).toEqual({ down: "refused" });
});

test("a watcher that is not there yet is tried again until it answers", async () => {
	const at = join(aPrivateDirectory(), "watcher.sock");
	const session = aSession();
	watching(at, session);

	expect(await session.next()).toEqual({ down: "refused" });
	const watcher = await aWatcherAt(at);
	const { connection, asked: line } = await watcher.next();
	connection.write(said(beadLine, freshness));

	expect(line).toBe(asked);
	expect(await session.next()).toEqual({ batch: [beadLine, freshness] });
});

test("a connection the watcher closes is down, and the bead is watched again", async () => {
	const at = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(at);
	const session = aSession();
	watching(at, session);

	(await watcher.next()).connection.end();

	expect(await session.next()).toEqual({ down: "closed" });
	expect((await watcher.next()).asked).toBe(asked);
});

test("a connection with no line for a minute has wedged and is closed", async () => {
	const at = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(at);
	const session = aSession();
	watching(at, session);

	const { connection } = await watcher.next();
	const closed = new Promise((resolve) => connection.on("close", resolve));

	expect(await session.next()).toEqual({ down: "wedged" });
	await closed;
	expect((await watcher.next()).asked).toBe(asked);
});

test("alive lines keep a quiet connection up", async () => {
	const at = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(at);
	const session = aSession();
	watching(at, session);

	const { connection } = await watcher.next();
	const alive = setInterval(
		() => connection.write(said({ line: "alive" })),
		timing.wedgedAfter / 4,
	);
	await pause(timing.wedgedAfter * 3);
	clearInterval(alive);

	expect(session.heard).toEqual([]);
});

test("a protocol the plugin does not know is a watcher that is down", async () => {
	const { protocol: _, ...noProtocol } = freshness;
	for (const answer of [
		said(beadLine, { ...freshness, protocol: 2 }),
		said(beadLine, noProtocol),
		"unknown watch summit-works smt-4kd3p.20\n",
		said("a string"),
	]) {
		const at = join(aPrivateDirectory(), "watcher.sock");
		const watcher = await aWatcherAt(at);
		const session = aSession();
		const watch = watching(at, session);

		(await watcher.next()).connection.write(answer);

		expect(await session.next()).toEqual({ down: "protocol" });
		watch.stop();
	}
});

test("a socket that is not the user's own is not believed", async () => {
	const open = join(aPrivateDirectory(), "open");
	mkdirSync(open);
	chmodSync(open, 0o777);
	const at = join(open, "watcher.sock");
	const watcher = await aWatcherAt(at);
	let connections = 0;
	watcher.server.on("connection", () => {
		connections += 1;
	});
	const session = aSession();
	watching(at, session);

	expect(await session.next()).toEqual({ down: "refused" });
	expect(connections).toBe(0);
});

test("something other than a socket at the path is not believed", async () => {
	const at = join(aPrivateDirectory(), "watcher.sock");
	writeFileSync(at, "");
	const session = aSession();
	watching(at, session);

	expect(await session.next()).toEqual({ down: "refused" });
});

test("with nowhere to find a watcher the watcher is down", async () => {
	const session = aSession();
	watching(undefined, session);

	expect(await session.next()).toEqual({ down: "nowhere" });
});

test("the pause before connecting again grows to a limit", () => {
	expect(
		[1, 2, 3, 4, 9].map((failures) => pauseAfter(failures, timing)),
	).toEqual([10, 20, 40, 40, 40]);
});

test("a watcher that answered is tried again after the shortest pause", async () => {
	const slowly: Timing = { ...timing, firstPause: 20, longestPause: 5_000 };
	const at = join(aPrivateDirectory(), "watcher.sock");
	const session = aSession();
	const watch = watchBead(bead, () => at, session.hearing, slowly);
	cleanUp.push(watch.stop);
	for (let refused = 0; refused < 4; refused += 1) {
		expect(await session.next()).toEqual({ down: "refused" });
	}

	const watcher = await aWatcherAt(at);
	(await watcher.next()).connection.end(said(beadLine, freshness));
	expect(await session.next()).toEqual({ batch: [beadLine, freshness] });
	expect(await session.next()).toEqual({ down: "closed" });
	const downAt = Date.now();
	await watcher.next();

	expect(Date.now() - downAt).toBeLessThan(200);
});

test("a watch that stops closes its connection and is not tried again", async () => {
	const at = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(at);
	const session = aSession();
	const watch = watching(at, session);

	const { connection } = await watcher.next();
	const closed = new Promise((resolve) => connection.on("close", resolve));
	watch.stop();
	await closed;
	await pause(timing.longestPause * 3);

	expect(session.heard).toEqual([]);
});

test("a watch stopped as it hears its watcher is down is not tried again", async () => {
	const at = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(at);
	let connections = 0;
	watcher.server.on("connection", () => {
		connections += 1;
	});
	const told: Down[] = [];
	const watch = watchBead(
		bead,
		() => at,
		{
			heard: () => {},
			down: (why) => {
				told.push(why);
				watch.stop();
			},
		},
		timing,
	);
	cleanUp.push(watch.stop);

	(await watcher.next()).connection.end();
	await pause(timing.longestPause * 3);

	expect(told).toEqual(["closed"]);
	expect(connections).toBe(1);
});

test("a watch stopped as it hears a batch is told nothing more", async () => {
	const at = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(at);
	const heard: (readonly Said[] | Down)[] = [];
	const watch = watchBead(
		bead,
		() => at,
		{
			heard: (batch) => {
				heard.push(batch);
				watch.stop();
			},
			down: (why) => heard.push(why),
		},
		timing,
	);
	cleanUp.push(watch.stop);

	(await watcher.next()).connection.write(
		said(beadLine, freshness, beadLine, freshness),
	);
	await pause(timing.longestPause * 3);

	expect(heard).toEqual([[beadLine, freshness]]);
});
