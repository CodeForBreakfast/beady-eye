import { afterEach, expect, test } from "bun:test";
import { linkSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import { Effect } from "effect";
import { buildServer } from "./server";
import {
	aPrivateDirectory,
	aWatcherAt,
	cleanUp,
	cleanUpAfterEach,
	said,
	someWatches,
} from "./test-watcher";
import type { Timing } from "./watcher";

afterEach(cleanUpAfterEach);

const timing: Timing = {
	wedgedAfter: 5_000,
	firstPause: 10,
	longestPause: 40,
	answeredWithin: 100,
};
const session = "6f1c2d3e-4a5b-4c6d-8e7f-901234567890";

const beadLine = (project: string) => ({
	line: "bead",
	project,
	ready: false,
	blocked_by: ["smt-4kd3p.13"],
	bd: { ready: false, blocked_by: ["smt-4kd3p.13"] },
	row: {
		id: "smt-4kd3p.20",
		title: "the daily wallpaper timer calls dms",
		status: "blocked",
	},
});
const gone = (project: string) => ({
	line: "gone",
	project,
	id: "smt-4kd3p.20",
});
const freshness = (project: string) => ({
	line: "freshness",
	project,
	as_of: "2026-08-30T10:22:14Z",
	tracker: "ok",
	events: "off",
	protocol: 1,
	reach: { path: `/home/mira/${project}`, environment_command: [] },
});

const watched = "smt-4kd3p.20 in summit-works";
const asWatched = `"the daily wallpaper timer calls dms", blocked, not ready`;

interface Place {
	readonly directory: string;
	readonly at: string | undefined;
}

const aPlace = (): Place => {
	const directory = aPrivateDirectory();
	return { directory, at: join(directory, "watcher.sock") };
};

/** A server of its own for a session, and a way to call its tools as that
 * session would, with the hook's session id added. */
const aServer = async (
	{ directory, at }: Place,
	projects: readonly string[] = ["summit-works", "harbour"],
) => {
	const { watches, close } = await someWatches(directory, at, projects, timing);
	const client = new Client({ name: "session", version: "0" });
	const [clientSide, serverSide] = InMemoryTransport.createLinkedPair();
	await buildServer(watches).connect(serverSide);
	await client.connect(clientSide);
	const exit = async () => {
		await close();
		await client.close();
	};
	cleanUp.push(exit);
	const use = async (name: string, args: Record<string, string> = {}) => {
		const result = await client.callTool({
			name,
			arguments: { ...args, session_id: session },
		});
		const [content] = result.content as { text: string }[];
		return { text: content?.text, refused: result.isError };
	};
	return Object.assign(use, { exit });
};

const kept = ({ directory }: Place) =>
	JSON.parse(
		readFileSync(
			join(directory, "beady-eye", "watches", `${session}.json`),
			"utf8",
		),
	);

test("watch answers with the bead as it stands, and keeps the watch", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const use = await aServer(place);

	const answer = use("watch", { id: "smt-4kd3p.20", project: "summit-works" });
	const { connection, asked } = await watcher.next();
	connection.write(said(beadLine("summit-works"), freshness("summit-works")));

	expect(asked).toBe("watch summit-works smt-4kd3p.20\n");
	expect(await answer).toEqual({
		text: `Watching ${watched}: ${asWatched}.`,
		refused: false,
	});
	expect(kept(place)).toEqual([
		{ project: "summit-works", id: "smt-4kd3p.20" },
	]);
	expect(await use("watching")).toEqual({
		text: `The watcher is answering.\n- ${watched}: ${asWatched}`,
		refused: false,
	});
});

test("watch finds a bead named without its project by asking the watcher", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const use = await aServer(place);

	const answer = use("watch", { id: "smt-4kd3p.20" });
	const asking = await watcher.next();
	asking.connection.write(
		said(
			beadLine("summit-works"),
			freshness("summit-works"),
			gone("harbour"),
			freshness("harbour"),
		),
	);
	const watching = await watcher.next();
	watching.connection.write(
		said(beadLine("summit-works"), freshness("summit-works")),
	);

	expect(asking.asked).toBe(
		"watch summit-works smt-4kd3p.20\nwatch harbour smt-4kd3p.20\n",
	);
	expect(watching.asked).toBe("watch summit-works smt-4kd3p.20\n");
	expect(await answer).toEqual({
		text: `Watching ${watched}: ${asWatched}.`,
		refused: false,
	});
});

test("watch refuses a bead named without its project that no project holds", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const use = await aServer(place);

	const answer = use("watch", { id: "smt-4kd3p.20" });
	(await watcher.next()).connection.write(
		said(
			gone("summit-works"),
			freshness("summit-works"),
			gone("harbour"),
			freshness("harbour"),
		),
	);

	expect(await answer).toEqual({
		text: "No project the watcher reads holds smt-4kd3p.20. Name its project.",
		refused: true,
	});
});

test("watch refuses a bead named without its project that several projects hold", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const use = await aServer(place);

	const answer = use("watch", { id: "smt-4kd3p.20" });
	(await watcher.next()).connection.write(
		said(
			beadLine("summit-works"),
			freshness("summit-works"),
			beadLine("harbour"),
			freshness("harbour"),
		),
	);

	expect(await answer).toEqual({
		text: "smt-4kd3p.20 is held by more than one project: summit-works, harbour. Name its project.",
		refused: true,
	});
	expect(await use("watching")).toEqual({
		text: "This session watches no beads.",
		refused: false,
	});
});

test("watch refuses a project the watcher does not read", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const use = await aServer(place);

	const answer = use("watch", { id: "smt-4kd3p.20", project: "dunmore" });
	const { connection } = await watcher.next();
	const hungUp = new Promise((resolve) => connection.on("close", resolve));
	connection.write(
		said({
			line: "refused",
			asked: "watch dunmore smt-4kd3p.20",
			reason: "unknown-project",
		}),
	);

	expect(await answer).toEqual({
		text: "The watcher does not read a project dunmore.",
		refused: true,
	});
	await hungUp;
	expect(kept(place)).toEqual([]);
});

test("watch refuses a bead its project does not hold, naming the project", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const use = await aServer(place);

	const answer = use("watch", { id: "smt-4kd3p.20", project: "harbour" });
	(await watcher.next()).connection.write(
		said(gone("harbour"), freshness("harbour")),
	);

	expect(await answer).toEqual({
		text: "harbour holds no bead smt-4kd3p.20.",
		refused: true,
	});
});

test("watching while the watcher is down is accepted, and the answer says so", async () => {
	const place = { ...aPlace(), at: undefined };
	const use = await aServer(place);

	const down =
		"The watcher is down: the bdi config names no watcher socket, and there is no runtime directory to look for one in.";
	expect(
		await use("watch", { id: "smt-4kd3p.20", project: "summit-works" }),
	).toEqual({
		text: `Watching ${watched} from when the watcher answers. ${down}`,
		refused: false,
	});
	expect(kept(place)).toEqual([
		{ project: "summit-works", id: "smt-4kd3p.20" },
	]);
	expect((await use("watching")).text).toBe(
		`${down} Each bead's status is the last one known.\n- ${watched}: not heard of yet`,
	);
});

test("a session's watches are kept by replacing its file, never by writing into it", async () => {
	const place = { ...aPlace(), at: undefined };
	const use = await aServer(place);
	await use("watch", { id: "smt-4kd3p.20", project: "summit-works" });
	const file = join(place.directory, "beady-eye", "watches", `${session}.json`);
	const before = join(place.directory, "before.json");
	linkSync(file, before);

	await use("watch", { id: "hbr-2", project: "harbour" });

	expect(JSON.parse(readFileSync(before, "utf8"))).toEqual([
		{ project: "summit-works", id: "smt-4kd3p.20" },
	]);
	expect(kept(place)).toEqual([
		{ project: "summit-works", id: "smt-4kd3p.20" },
		{ project: "harbour", id: "hbr-2" },
	]);
});

test("watch refuses to find a bead's project where the watcher has not read a project", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const use = await aServer(place);

	const answer = use("watch", { id: "smt-4kd3p.20" });
	(await watcher.next()).connection.write(
		said(beadLine("summit-works"), freshness("summit-works"), {
			...freshness("harbour"),
			as_of: null,
			tracker: { unreachable: "timeout" },
		}),
	);

	expect(await answer).toEqual({
		text: "The watcher has not read harbour yet, so it cannot say which project holds smt-4kd3p.20. Name its project.",
		refused: true,
	});
});

test("watch refuses to find a bead's project where a project does not answer in time", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const use = await aServer(place);

	const answer = use("watch", { id: "smt-4kd3p.20" });
	(await watcher.next()).connection.write(
		said(beadLine("summit-works"), freshness("summit-works")),
	);

	expect(await answer).toEqual({
		text: "The watcher has not read harbour yet, so it cannot say which project holds smt-4kd3p.20. Name its project.",
		refused: true,
	});
});

test("a watch is kept only once it is accepted", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const use = await aServer(place);

	const waiting = use("watch", { id: "smt-4kd3p.20", project: "summit-works" });
	const first = await watcher.next();
	const answered = use("watch", { id: "smt-4kd3p.20", project: "harbour" });
	(await watcher.next()).connection.write(
		said(beadLine("harbour"), freshness("harbour")),
	);
	await answered;

	expect(kept(place)).toEqual([{ project: "harbour", id: "smt-4kd3p.20" }]);
	first.connection.write(
		said(beadLine("summit-works"), freshness("summit-works")),
	);
	await waiting;
	expect(kept(place)).toEqual([
		{ project: "summit-works", id: "smt-4kd3p.20" },
		{ project: "harbour", id: "smt-4kd3p.20" },
	]);
});

test("watch refuses to find a bead's project while the watcher is down", async () => {
	const use = await aServer({ ...aPlace(), at: undefined });

	expect(await use("watch", { id: "smt-4kd3p.20" })).toEqual({
		text: "The watcher is down, so the project holding smt-4kd3p.20 cannot be found: the bdi config names no watcher socket, and there is no runtime directory to look for one in. Name its project to watch it from when the watcher answers.",
		refused: true,
	});
});

test("watching a watched bead again keeps the watch, though the bead has gone", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const use = await aServer(place);
	const answer = use("watch", { id: "smt-4kd3p.20", project: "summit-works" });
	const { connection } = await watcher.next();
	connection.write(said(beadLine("summit-works"), freshness("summit-works")));
	await answer;

	connection.write(said(gone("summit-works"), freshness("summit-works")));
	while (!(await use("watching")).text?.includes("gone")) await Bun.sleep(5);

	expect(
		await use("watch", { id: "smt-4kd3p.20", project: "summit-works" }),
	).toEqual({
		text: `Watching ${watched}: gone from its tracker.`,
		refused: false,
	});
	expect(kept(place)).toEqual([
		{ project: "summit-works", id: "smt-4kd3p.20" },
	]);
});

test("a watch unwatched before the watcher answers says so", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const use = await aServer(place);
	const answer = use("watch", { id: "smt-4kd3p.20", project: "summit-works" });
	await watcher.next();

	await use("unwatch", { id: "smt-4kd3p.20" });

	expect(await answer).toEqual({
		text: `${watched} was unwatched before the watcher answered.`,
		refused: true,
	});
	expect(kept(place)).toEqual([]);
});

test("unwatch closes the bead's connection and forgets it", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const use = await aServer(place);
	const answer = use("watch", { id: "smt-4kd3p.20", project: "summit-works" });
	const { connection } = await watcher.next();
	connection.write(said(beadLine("summit-works"), freshness("summit-works")));
	await answer;
	const closed = new Promise((resolve) => connection.on("close", resolve));

	expect(await use("unwatch", { id: "smt-4kd3p.20" })).toEqual({
		text: `Stopped watching ${watched}.`,
		refused: false,
	});
	await closed;
	expect(kept(place)).toEqual([]);
	expect((await use("watching")).text).toBe("This session watches no beads.");
});

test("unwatch refuses a bead the session does not watch, or watches in several projects", async () => {
	const use = await aServer({ ...aPlace(), at: undefined });

	expect(await use("unwatch", { id: "smt-4kd3p.20" })).toEqual({
		text: "This session does not watch smt-4kd3p.20.",
		refused: true,
	});
	await use("watch", { id: "smt-4kd3p.20", project: "summit-works" });
	await use("watch", { id: "smt-4kd3p.20", project: "harbour" });
	expect(await use("unwatch", { id: "smt-4kd3p.20" })).toEqual({
		text: "This session watches smt-4kd3p.20 in more than one project: summit-works, harbour. Name its project.",
		refused: true,
	});
	expect(
		await use("unwatch", { id: "smt-4kd3p.20", project: "harbour" }),
	).toEqual({
		text: "Stopped watching smt-4kd3p.20 in harbour.",
		refused: false,
	});
});

test("a bead's id or project with a space in it is refused", async () => {
	const use = await aServer({ ...aPlace(), at: undefined });

	for (const args of <Record<string, string>[]>[
		{ id: "smt-4kd3p.20\nwatch harbour" },
		{ id: "smt-4kd3p.20", project: "summit works" },
		{},
	]) {
		expect(await use("watch", args)).toEqual({
			text: "A bead's id and project are each one word with no space in it.",
			refused: true,
		});
	}
});

test("a server started again watches what the session watched once it learns the session's id", async () => {
	const place = aPlace();
	const watcher = await aWatcherAt(place.at as string);
	const before = await aServer(place);
	const answer = before("watch", {
		id: "smt-4kd3p.20",
		project: "summit-works",
	});
	(await watcher.next()).connection.write(
		said(beadLine("summit-works"), freshness("summit-works")),
	);
	await answer;
	await before.exit();

	const after = await aServer(place);
	const listed = after("watching");
	const { connection, asked } = await watcher.next();

	expect(asked).toBe("watch summit-works smt-4kd3p.20\n");
	expect((await listed).text).toBe(
		`The watcher is answering.\n- ${watched}: not heard of yet`,
	);
	connection.write(said(beadLine("summit-works"), freshness("summit-works")));
	expect(
		await after("watch", { id: "smt-4kd3p.20", project: "summit-works" }),
	).toEqual({ text: `Watching ${watched}: ${asWatched}.`, refused: false });
});

test("calls that arrive together all see the session's watches restored", async () => {
	const place = { ...aPlace(), at: undefined };
	mkdirSync(join(place.directory, "beady-eye", "watches"), { recursive: true });
	writeFileSync(
		join(place.directory, "beady-eye", "watches", `${session}.json`),
		JSON.stringify([{ project: "summit-works", id: "smt-4kd3p.20" }]),
	);
	const use = await aServer(place);

	const [listed, unwatched] = await Promise.all([
		use("watching"),
		use("unwatch", { id: "smt-4kd3p.20" }),
	]);

	expect(listed.text).toContain(`- ${watched}: not heard of yet`);
	expect(unwatched).toEqual({
		text: `Stopped watching ${watched}.`,
		refused: false,
	});
});

test("a server keeps the first session id it learns", async () => {
	const place = { ...aPlace(), at: undefined };
	const { watches } = await someWatches(place.directory, place.at, [], timing);

	await Effect.runPromise(
		Effect.all([
			watches.learn("not a session id"),
			watches.learn(session),
			watches.learn("0e1d2c3b-4a59-4876-9543-210fedcba987"),
			watches.watch("smt-4kd3p.20", "summit-works"),
		]),
	);

	expect(kept(place)).toEqual([
		{ project: "summit-works", id: "smt-4kd3p.20" },
	]);
});
