import { afterEach, expect, test } from "bun:test";
import {
	chmodSync,
	lstatSync,
	mkdirSync,
	mkdtempSync,
	rmSync,
	symlinkSync,
	writeFileSync,
} from "node:fs";
import { createServer, type Server, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { FileSystem } from "@effect/platform";
import { NodeFileSystem } from "@effect/platform-node";
import {
	Chunk,
	ConfigProvider,
	Effect,
	Fiber,
	Option,
	Queue,
	Stream,
} from "effect";
import {
	type Heard,
	othersMayTakeANameIn,
	pauseAfter,
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

const cleanUp: (() => unknown)[] = [];
afterEach(async () => {
	for (const clean of cleanUp.splice(0)) await clean();
});

const aPrivateDirectory = () => {
	const directory = mkdtempSync(join(tmpdir(), "beady-eye-watcher-"));
	cleanUp.push(() => rmSync(directory, { recursive: true, force: true }));
	return directory;
};

const run = <A>(effect: Effect.Effect<A, never, FileSystem.FileSystem>) =>
	Effect.runPromise(effect.pipe(Effect.provide(NodeFileSystem.layer)));

const at = (path: string | undefined) =>
	Effect.succeed(Option.fromNullable(path));

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

/** A session watching the bead at `path`, and what it is told, in order. */
const watching = (path: string | undefined, watchTiming = timing) => {
	const told = Effect.runSync(Queue.unbounded<Heard>());
	const watch = Effect.runFork(
		watchBead(bead, at(path), watchTiming).pipe(
			Stream.runForEach((heard) => Queue.offer(told, heard)),
			Effect.provide(NodeFileSystem.layer),
		),
	);
	const stop = () => Effect.runPromise(Fiber.interrupt(watch));
	cleanUp.push(stop);
	return {
		stop,
		next: () => Effect.runPromise(Queue.take(told)),
		heard: () => Chunk.toArray(Effect.runSync(Queue.takeAll(told))),
	};
};

/** What a session that stops after hearing `count` things is told. */
const firstHeard = (path: string, count: number) =>
	run(
		watchBead(bead, at(path), timing).pipe(
			Stream.take(count),
			Stream.runCollect,
			Effect.map(Chunk.toArray),
		),
	);

const pause = (millis: number) =>
	new Promise((resolve) => setTimeout(resolve, millis));

const whereWith = (config: string, runtimeDirectory: string | undefined) =>
	run(
		whereTheWatcherIs(config).pipe(
			Effect.withConfigProvider(
				ConfigProvider.fromMap(
					new Map(
						runtimeDirectory === undefined
							? []
							: [["XDG_RUNTIME_DIR", runtimeDirectory]],
					),
				),
			),
			Effect.map(Option.getOrUndefined),
		),
	);

test("the config's watcher socket is where the watcher is", async () => {
	const config = join(aPrivateDirectory(), "config.toml");
	writeFileSync(config, '[watcher]\nsocket = "/var/folders/T/watcher.sock"\n');

	expect(await whereWith(config, "/run/user/1000")).toBe(
		"/var/folders/T/watcher.sock",
	);
});

test("a config naming no socket leaves the watcher under the runtime directory", async () => {
	const directory = aPrivateDirectory();
	const config = join(directory, "config.toml");
	writeFileSync(config, "[tui]\nrefresh_seconds = 30\n");

	for (const named of [config, join(directory, "missing.toml")]) {
		expect(await whereWith(named, "/run/user/1000")).toBe(
			"/run/user/1000/beady-eye/watcher.sock",
		);
	}
});

test("with no socket named and no runtime directory there is no watcher to reach", async () => {
	const config = join(aPrivateDirectory(), "missing.toml");

	expect(await whereWith(config, undefined)).toBeUndefined();
	expect(await whereWith(config, "")).toBeUndefined();
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
	const path = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(path);
	const session = watching(path);

	const { connection, asked: line } = await watcher.next();
	connection.write(said({ line: "alive" }, beadLine, freshness));

	expect(line).toBe(asked);
	expect(await session.next()).toEqual({ batch: [beadLine, freshness] });
});

test("a refusal of the watch arrives on its own", async () => {
	const path = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(path);
	const session = watching(path);

	const refused = {
		line: "refused",
		asked: asked.trim(),
		reason: "unknown-project",
	};
	(await watcher.next()).connection.write(said(refused));

	expect(await session.next()).toEqual({ batch: [refused] });
});

test("a refused connection is a watcher that is down", async () => {
	const path = join(aPrivateDirectory(), "watcher.sock");
	await aWatcherThatDied(path);
	expect(lstatSync(path).isSocket()).toBe(true);
	const session = watching(path);

	expect(await session.next()).toEqual({ down: "refused" });
});

test("a watcher that is not there yet is tried again until it answers", async () => {
	const path = join(aPrivateDirectory(), "watcher.sock");
	const session = watching(path);

	expect(await session.next()).toEqual({ down: "refused" });
	const watcher = await aWatcherAt(path);
	const { connection, asked: line } = await watcher.next();
	connection.write(said(beadLine, freshness));

	expect(line).toBe(asked);
	expect(await session.next()).toEqual({ batch: [beadLine, freshness] });
});

test("a connection the watcher closes is down, and the bead is watched again", async () => {
	const path = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(path);
	const session = watching(path);

	(await watcher.next()).connection.end();

	expect(await session.next()).toEqual({ down: "closed" });
	expect((await watcher.next()).asked).toBe(asked);
});

test("a connection with no line for a minute has wedged and is closed", async () => {
	const path = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(path);
	const session = watching(path);

	const { connection } = await watcher.next();
	const closed = new Promise((resolve) => connection.on("close", resolve));

	expect(await session.next()).toEqual({ down: "wedged" });
	await closed;
	expect((await watcher.next()).asked).toBe(asked);
});

test("alive lines keep a quiet connection up", async () => {
	const path = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(path);
	const session = watching(path);

	const { connection } = await watcher.next();
	const alive = setInterval(
		() => connection.write(said({ line: "alive" })),
		timing.wedgedAfter / 4,
	);
	await pause(timing.wedgedAfter * 3);
	clearInterval(alive);

	expect(session.heard()).toEqual([]);
});

test("a protocol the plugin does not know is a watcher that is down", async () => {
	const { protocol: _, ...noProtocol } = freshness;
	for (const answer of [
		said(beadLine, { ...freshness, protocol: 2 }),
		said(beadLine, noProtocol),
		"unknown watch summit-works smt-4kd3p.20\n",
		said("a string"),
	]) {
		const path = join(aPrivateDirectory(), "watcher.sock");
		const watcher = await aWatcherAt(path);
		const session = watching(path);

		(await watcher.next()).connection.write(answer);

		expect(await session.next()).toEqual({ down: "protocol" });
		await session.stop();
	}
});

test("a socket that is not the user's own is not believed", async () => {
	const open = join(aPrivateDirectory(), "open");
	mkdirSync(open);
	chmodSync(open, 0o777);
	const path = join(open, "watcher.sock");
	const watcher = await aWatcherAt(path);
	let connections = 0;
	watcher.server.on("connection", () => {
		connections += 1;
	});
	const session = watching(path);

	expect(await session.next()).toEqual({ down: "refused" });
	expect(connections).toBe(0);
});

test("something other than a socket at the path is not believed", async () => {
	const path = join(aPrivateDirectory(), "watcher.sock");
	writeFileSync(path, "");
	const session = watching(path);

	expect(await session.next()).toEqual({ down: "refused" });
});

test("a link to the user's own socket is not believed", async () => {
	const directory = aPrivateDirectory();
	const socket = join(directory, "watcher.sock");
	const watcher = await aWatcherAt(socket);
	let connections = 0;
	watcher.server.on("connection", () => {
		connections += 1;
	});
	const path = join(directory, "link.sock");
	symlinkSync(socket, path);
	const session = watching(path);

	expect(await session.next()).toEqual({ down: "refused" });
	expect(connections).toBe(0);
});

test("with nowhere to find a watcher the watcher is down", async () => {
	const session = watching(undefined);

	expect(await session.next()).toEqual({ down: "nowhere" });
});

test("the pause before connecting again grows to a limit", () => {
	expect(
		[1, 2, 3, 4, 9].map((failures) => pauseAfter(failures, timing)),
	).toEqual([10, 20, 40, 40, 40]);
});

test("a watcher that answered is tried again after the shortest pause", async () => {
	const slowly: Timing = { ...timing, firstPause: 20, longestPause: 5_000 };
	const path = join(aPrivateDirectory(), "watcher.sock");
	const session = watching(path, slowly);
	for (let refused = 0; refused < 4; refused += 1) {
		expect(await session.next()).toEqual({ down: "refused" });
	}

	const watcher = await aWatcherAt(path);
	(await watcher.next()).connection.end(said(beadLine, freshness));
	expect(await session.next()).toEqual({ batch: [beadLine, freshness] });
	expect(await session.next()).toEqual({ down: "closed" });
	const downAt = Date.now();
	await watcher.next();

	expect(Date.now() - downAt).toBeLessThan(200);
});

test("a watch that stops closes its connection and is not tried again", async () => {
	const path = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(path);
	const session = watching(path);

	const { connection } = await watcher.next();
	const closed = new Promise((resolve) => connection.on("close", resolve));
	await session.stop();
	await closed;
	await pause(timing.longestPause * 3);

	expect(session.heard()).toEqual([]);
});

test("a watch stopped as it hears its watcher is down is not tried again", async () => {
	const path = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(path);
	let connections = 0;
	watcher.server.on("connection", () => {
		connections += 1;
	});

	const told = firstHeard(path, 1);
	(await watcher.next()).connection.end();
	expect(await told).toEqual([{ down: "closed" }]);
	await pause(timing.longestPause * 3);

	expect(connections).toBe(1);
});

test("a watch stopped as it hears a batch closes its connection", async () => {
	const path = join(aPrivateDirectory(), "watcher.sock");
	const watcher = await aWatcherAt(path);

	const told = firstHeard(path, 1);
	const { connection } = await watcher.next();
	const closed = new Promise((resolve) => connection.on("close", resolve));
	connection.write(said(beadLine, freshness, beadLine, freshness));

	expect(await told).toEqual([{ batch: [beadLine, freshness] }]);
	await closed;
});
