// What the tests stand in place of a running `bdi watch` with, and the
// watches they point at it.

import { mkdtempSync, rmSync } from "node:fs";
import { createServer, type Server, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { NodeFileSystem } from "@effect/platform-node";
import { ConfigProvider, Effect, Exit, Option, Scope } from "effect";
import type { Timing } from "./watcher";
import { makeWatches } from "./watches";

/** What each test leaves behind, for `cleanUpAfterEach` to undo. */
export const cleanUp: (() => unknown)[] = [];

export const cleanUpAfterEach = async () => {
	for (const clean of cleanUp.splice(0)) await clean();
};

export const aPrivateDirectory = () => {
	const directory = mkdtempSync(join(tmpdir(), "beady-eye-watcher-"));
	cleanUp.push(() => rmSync(directory, { recursive: true, force: true }));
	return directory;
};

export const said = (...lines: unknown[]) =>
	lines.map((line) => `${JSON.stringify(line)}\n`).join("");

export interface Connection {
	readonly connection: Socket;
	readonly asked: string;
}

/** A watcher of the test's own, handing over each connection with the lines
 * it asked. */
export const aWatcherAt = async (at: string) => {
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

/** A session's watches, kept under `state`, looking for the watcher at `at`
 * and for beads in `projects`, until the test is over. */
export const someWatches = async (
	state: string,
	at: string | undefined,
	projects: readonly string[],
	timing?: Timing,
) => {
	const scope = Effect.runSync(Scope.make());
	const watches = await Effect.runPromise(
		makeWatches({
			findWatcher: Effect.succeed(Option.fromNullable(at)),
			projects: Effect.succeed(projects),
			...(timing === undefined ? {} : { timing }),
		}).pipe(
			Scope.extend(scope),
			Effect.provide(NodeFileSystem.layer),
			Effect.withConfigProvider(
				ConfigProvider.fromMap(new Map([["XDG_STATE_HOME", state]])),
			),
		),
	);
	const close = () => Effect.runPromise(Scope.close(scope, Exit.void));
	cleanUp.push(close);
	return { watches, close };
};
