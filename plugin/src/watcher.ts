// The plugin's side of `bdi watch`. `docs/design.md`'s *Watching* is the
// protocol, and *A quiet watcher, and one that has gone* says when a watcher
// is down.

import { lstatSync, readFileSync, realpathSync, statSync } from "node:fs";
import { createConnection } from "node:net";
import { dirname, join, resolve } from "node:path";
import { parse } from "smol-toml";

/** The version of the watcher's lines this plugin reads. */
export const PROTOCOL = 1;

export interface Timing {
	/** How long a connection may carry no line before it has wedged. The
	 * watcher sends an alive line every 20 seconds. */
	readonly wedgedAfter: number;
	readonly firstPause: number;
	readonly longestPause: number;
}

export const WATCHER_TIMING: Timing = {
	wedgedAfter: 60_000,
	firstPause: 1_000,
	longestPause: 30_000,
};

/**
 * Why a bead's connection is not watching: there is no path to look for a
 * watcher at, nothing at the path would take the connection or could be
 * believed, the watcher closed it, it carried no line for too long, or it
 * spoke a protocol this plugin does not know.
 */
export type Down = "nowhere" | "refused" | "closed" | "wedged" | "protocol";

/** One line the watcher sent about a watch. */
export interface Said {
	readonly line: string;
	readonly [field: string]: unknown;
}

export interface Hearing {
	/** The lines of one answer for the bead's project, ending with its
	 * freshness line, or a refused line on its own. */
	heard(batch: readonly Said[]): void;
	down(why: Down): void;
}

export interface Bead {
	readonly project: string;
	readonly id: string;
}

/**
 * Where `bdi watch` takes its socket: `[watcher]`'s `socket` in the bdi
 * config, or under the runtime directory where the config names none.
 */
export const whereTheWatcherIs = (
	config: string,
	runtimeDirectory: string | undefined,
): string | undefined =>
	socketNamedIn(config) ??
	(runtimeDirectory
		? join(runtimeDirectory, "beady-eye", "watcher.sock")
		: undefined);

const socketNamedIn = (config: string): string | undefined => {
	let read: Record<string, unknown>;
	try {
		read = parse(readFileSync(config, "utf8"));
	} catch {
		return undefined;
	}
	const watcher = read.watcher;
	if (typeof watcher !== "object" || watcher === null) return undefined;
	const socket = (watcher as Record<string, unknown>).socket;
	return typeof socket === "string" ? socket : undefined;
};

const A_GROUP_MAY_TAKE_NAMES = 0o030;
const ANYBODY_MAY_TAKE_NAMES = 0o003;
const NAMES_STAY_THEIR_OWNERS = 0o1000;
const THE_SYSTEM = 0;

/**
 * Whether somebody other than `user` could put their own file at a name in a
 * directory with this mode and owner. An owner may take any name in their own
 * directory, and anybody who may write and search there may too, unless the
 * sticky bit keeps each name its owner's.
 */
export const othersMayTakeANameIn = (
	mode: number,
	owner: number,
	user: number,
): boolean => {
	if (owner !== user && owner !== THE_SYSTEM) return true;
	const anybodyElse =
		(mode & A_GROUP_MAY_TAKE_NAMES) === A_GROUP_MAY_TAKE_NAMES ||
		(mode & ANYBODY_MAY_TAKE_NAMES) === ANYBODY_MAY_TAKE_NAMES;
	return anybodyElse && (mode & NAMES_STAY_THEIR_OWNERS) === 0;
};

/** The directory and every one above it but the root, nearest first. */
const directoriesOn = (way: string): string[] => {
	const directories = [way];
	let above = dirname(way);
	while (dirname(above) !== above) {
		directories.push(above);
		above = dirname(above);
	}
	return directories;
};

/**
 * Whether the socket at `at` is one only this user could have put there: a
 * socket of the user's own, under a way down nobody else may take a name in,
 * judged both as spelled and as resolved. `bdi` makes the same checks before
 * it believes a watcher.
 */
export const onlyThisUserHolds = (at: string): boolean => {
	const user = process.geteuid?.();
	if (user === undefined) return false;
	try {
		const under = dirname(resolve(at));
		const ways = [
			...directoriesOn(under),
			...directoriesOn(realpathSync(under)),
		];
		for (const directory of ways) {
			const { mode, uid } = statSync(directory);
			if (othersMayTakeANameIn(mode, uid, user)) return false;
		}
		const socket = lstatSync(at);
		return socket.isSocket() && socket.uid === user;
	} catch {
		return false;
	}
};

export const pauseAfter = (failures: number, timing: Timing): number =>
	Math.min(timing.firstPause * 2 ** (failures - 1), timing.longestPause);

const isSaid = (value: unknown): value is Said =>
	typeof value === "object" &&
	value !== null &&
	typeof (value as { line?: unknown }).line === "string";

/**
 * Hold a connection to the watcher that watches one bead, connecting again
 * with a growing pause whenever it is down. The protocol has no line to stop
 * a watch, so each bead has a connection of its own and stopping closes it.
 */
export const watchBead = (
	bead: Bead,
	findWatcher: () => string | undefined,
	hearing: Hearing,
	timing: Timing = WATCHER_TIMING,
): { stop(): void } => {
	let stopped = false;
	let failures = 0;
	let closeConnection = () => {};
	let again: ReturnType<typeof setTimeout> | undefined;

	const down = (why: Down) => {
		if (stopped) return;
		failures += 1;
		hearing.down(why);
		again = setTimeout(connect, pauseAfter(failures, timing));
	};

	const connect = () => {
		const at = findWatcher();
		if (at === undefined) return down("nowhere");
		if (!onlyThisUserHolds(at)) return down("refused");

		const connection = createConnection(at);
		let connected = false;
		let over = false;
		let pending = "";
		let batch: Said[] = [];

		const end = (why: Down) => {
			if (over) return;
			over = true;
			clearTimeout(wedged);
			connection.destroy();
			down(why);
		};
		const wedged = setTimeout(() => end("wedged"), timing.wedgedAfter);
		closeConnection = () => connection.destroy();

		const take = (line: string) => {
			wedged.refresh();
			let said: unknown;
			try {
				said = JSON.parse(line);
			} catch {
				return end("protocol");
			}
			if (!isSaid(said)) return end("protocol");
			if (said.line === "alive") return;
			if (said.line === "refused") return hearing.heard([said]);
			batch.push(said);
			if (said.line !== "freshness") return;
			if (said.protocol !== PROTOCOL) return end("protocol");
			failures = 0;
			hearing.heard(batch);
			batch = [];
		};

		connection.setEncoding("utf8");
		connection.on("connect", () => {
			connected = true;
			connection.write(`watch ${bead.project} ${bead.id}\n`);
		});
		connection.on("data", (chunk: string) => {
			const lines = (pending + chunk).split("\n");
			pending = lines.pop() ?? "";
			for (const line of lines) {
				if (over) return;
				take(line);
			}
		});
		connection.on("error", () => end(connected ? "closed" : "refused"));
		connection.on("close", () => end("closed"));
	};

	connect();
	return {
		stop: () => {
			stopped = true;
			clearTimeout(again);
			closeConnection();
		},
	};
};
