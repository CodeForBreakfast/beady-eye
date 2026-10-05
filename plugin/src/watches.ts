// The beads a session watches, kept on disk under its session id so a server
// started again for the same session watches them again. `docs/design.md`'s
// *Watching a bead* is the design.

import { homedir } from "node:os";
import { join } from "node:path";
import { FileSystem } from "@effect/platform";
import {
	Config,
	Deferred,
	Effect,
	Fiber,
	Option,
	Schema,
	type Scope,
	Stream,
} from "effect";
import {
	askAbout,
	type Bead,
	type Down,
	type Heard,
	type Said,
	type Timing,
	WATCHER_TIMING,
	watchBead,
} from "./watcher";

export interface Settings<R> {
	readonly findWatcher: Effect.Effect<Option.Option<string>, never, R>;
	/** The projects a bead named without one is looked for in. */
	readonly projects: Effect.Effect<readonly string[], never, R>;
	readonly timing?: Timing;
}

/** What a tool says back, and whether it refused what it was asked. */
export interface Answer {
	readonly text: string;
	readonly refused: boolean;
}

/** What the watcher last said of a watched bead. */
type Standing =
	| { readonly is: "unheard" }
	| {
			readonly is: "bead";
			readonly title: string;
			readonly status: string;
			readonly ready: boolean;
	  }
	| { readonly is: "gone" }
	| { readonly is: "refused"; readonly reason: string };

/** What is known of a watched bead from its connection. */
interface Known {
	standing: Standing;
	down: Down | undefined;
}

interface Watch {
	readonly bead: Bead;
	readonly known: Known;
	/** Whether the session has been told it watches the bead. Only a watch
	 * it has not been told of can still be refused. */
	accepted: boolean;
	/** Done once the watcher has answered, been found down, or the watch has
	 * stopped. */
	readonly firstHeard: Deferred.Deferred<void>;
	readonly connection: Fiber.RuntimeFiber<void>;
}

const SessionId = Schema.UUID;

const watchesDirectory = Config.nonEmptyString("XDG_STATE_HOME").pipe(
	Config.withDefault(join(homedir(), ".local", "state")),
	Config.map((state) => join(state, "beady-eye", "watches")),
);

const FILENAME_SAFE = /[^a-zA-Z0-9._-]/g;

const KeptWatches = Schema.parseJson(
	Schema.Array(Schema.Struct({ project: Schema.String, id: Schema.String })),
);

const row = (said: Said): Record<string, unknown> =>
	typeof said.row === "object" && said.row !== null
		? (said.row as Record<string, unknown>)
		: {};

const isAbout = (said: Said, bead: Bead) =>
	said.project === bead.project &&
	(said.line === "gone" ? said.id : row(said).id) === bead.id;

/** What a batch says of the bead, where it says anything. */
const standingIn = (
	batch: readonly Said[],
	bead: Bead,
): Standing | undefined => {
	const [first] = batch;
	if (first?.line === "refused") {
		return { is: "refused", reason: String(first.reason) };
	}
	const said = batch.filter((line) => isAbout(line, bead)).at(-1);
	if (said?.line === "gone") return { is: "gone" };
	if (said?.line !== "bead") return undefined;
	return {
		is: "bead",
		title: String(row(said).title),
		status: String(row(said).status),
		ready: said.ready === true,
	};
};

const named = ({ project, id }: Bead) => `${id} in ${project}`;

const whyDown: Record<Down, string> = {
	nowhere:
		"the bdi config names no watcher socket, and there is no runtime directory to look for one in",
	refused:
		"nothing at its socket took the connection, or the socket is not the user's own",
	closed: "it closed the connection",
	wedged: "it stopped sending lines",
	protocol:
		"it speaks a protocol this plugin does not know, so the plugin and the watcher need releases that speak the same one",
};

const told = (text: string): Answer => ({ text, refused: false });
const refusal = (text: string): Answer => ({ text, refused: true });

/** Why the watcher will not watch `bead`, where it will not. */
const refusalOf = ({ bead, known }: Watch): Answer | undefined => {
	const { standing } = known;
	if (standing.is === "gone") {
		return refusal(`${bead.project} holds no bead ${bead.id}.`);
	}
	if (standing.is !== "refused") return undefined;
	if (standing.reason === "unknown-project") {
		return refusal(`The watcher does not read a project ${bead.project}.`);
	}
	return refusal(
		`The watcher refused to watch ${named(bead)}: ${standing.reason}.`,
	);
};

const describe = ({ known: { standing } }: Watch): string => {
	switch (standing.is) {
		case "bead":
			return `"${standing.title}", ${standing.status}, ${standing.ready ? "ready" : "not ready"}`;
		case "gone":
			return "gone from its tracker";
		case "refused":
			return `refused by the watcher: ${standing.reason}`;
		case "unheard":
			return "not heard of yet";
	}
};

export interface Watches {
	/** Take the session's id, and watch again what it watched, where this is
	 * the first id the server has been given. A server serves one session, so
	 * a later id changes nothing. */
	learn(session: string | undefined): Effect.Effect<void>;
	watch(id: string, project: string | undefined): Effect.Effect<Answer>;
	unwatch(id: string, project: string | undefined): Effect.Effect<Answer>;
	readonly watching: Effect.Effect<Answer>;
}

/** A session's watches, each held until `unwatch` or until the scope closes,
 * which keeps what is on disk. */
export const makeWatches = <R>(
	settings: Settings<R>,
): Effect.Effect<Watches, never, R | FileSystem.FileSystem | Scope.Scope> =>
	Effect.gen(function* () {
		const timing = settings.timing ?? WATCHER_TIMING;
		const context = yield* Effect.context<R | FileSystem.FileSystem>();
		const scope = yield* Effect.scope;
		const fs = yield* FileSystem.FileSystem;
		const directory = yield* Effect.orDie(watchesDirectory);
		const session = yield* Deferred.make<string>();
		/** The session's file, once its watches are back. */
		const file = yield* Deferred.make<string>();
		const keeping = yield* Effect.makeSemaphore(1);
		const watches = new Map<string, Watch>();

		const keyOf = ({ project, id }: Bead) => `${project} ${id}`;

		const start = (bead: Bead, accepted: boolean) =>
			Effect.gen(function* () {
				const known: Known = { standing: { is: "unheard" }, down: undefined };
				const firstHeard = yield* Deferred.make<void>();
				const hear = (heard: Heard) =>
					Effect.sync(() => {
						if ("down" in heard) {
							known.down = heard.down;
							return;
						}
						known.down = undefined;
						known.standing = standingIn(heard.batch, bead) ?? known.standing;
					}).pipe(Effect.zipRight(Deferred.succeed(firstHeard, undefined)));
				const connection = yield* watchBead(
					bead,
					settings.findWatcher,
					timing,
				).pipe(
					Stream.runForEach(hear),
					Effect.provide(context),
					Effect.forkIn(scope),
				);
				const watch: Watch = { bead, known, accepted, firstHeard, connection };
				watches.set(keyOf(bead), watch);
				return watch;
			});

		const stop = (watch: Watch) =>
			Effect.sync(() => watches.delete(keyOf(watch.bead))).pipe(
				Effect.zipRight(Fiber.interrupt(watch.connection)),
				Effect.zipRight(Deferred.succeed(watch.firstHeard, undefined)),
			);

		const persist = keeping.withPermits(1)(
			Deferred.poll(file).pipe(
				Effect.flatMap(
					Option.match({
						onNone: () => Effect.void,
						onSome: (kept) =>
							Effect.flatMap(kept, (path) => {
								const beads = [...watches.values()]
									.filter((watch) => watch.accepted)
									.map(({ bead: { project, id } }) => ({ project, id }));
								return fs
									.makeDirectory(directory, { recursive: true })
									.pipe(
										Effect.zipRight(
											fs.writeFileString(path, JSON.stringify(beads)),
										),
									);
							}),
					}),
				),
				Effect.orDie,
			),
		);

		const restore = (id: string) =>
			Effect.gen(function* () {
				const path = join(directory, `${id.replace(FILENAME_SAFE, "_")}.json`);
				const kept = yield* fs.readFileString(path).pipe(
					Effect.flatMap(Schema.decodeUnknown(KeptWatches)),
					Effect.orElseSucceed(() => []),
				);
				for (const bead of kept) {
					if (!watches.has(keyOf(bead))) yield* start(bead, true);
				}
				yield* Deferred.succeed(file, path);
				const unkept = [...watches.values()].some(
					(watch) =>
						watch.accepted &&
						!kept.some((bead) => keyOf(bead) === keyOf(watch.bead)),
				);
				if (unkept) yield* persist;
			});

		/** The project of the one bead the watcher has with `id`. */
		const projectHolding = (id: string) =>
			Effect.gen(function* () {
				const answer = yield* Effect.flatMap(settings.projects, (projects) =>
					askAbout(id, projects, settings.findWatcher, timing),
				).pipe(Effect.provide(context));
				if (typeof answer === "string") {
					return refusal(
						`The watcher is down, so the project holding ${id} cannot be found: ${whyDown[answer]}. Name its project to watch it from when the watcher answers.`,
					);
				}
				const projectsOf = (lines: readonly Said[]) => [
					...new Set(lines.map((said) => String(said.project))),
				];
				const about = answer.lines.filter((said) =>
					isAbout(said, { project: String(said.project), id }),
				);
				const holders = projectsOf(
					about.filter((said) => said.line === "bead"),
				);
				const answered = projectsOf(about);
				const unread = [
					...projectsOf(
						answer.lines.filter(
							(said) =>
								said.line === "freshness" &&
								!answered.includes(String(said.project)),
						),
					),
					...answer.unanswered,
				];
				if (holders.length <= 1 && unread.length > 0) {
					const them = unread.join(", ");
					return refusal(
						`The watcher has not read ${them} yet, so it cannot say which project holds ${id}. Name its project.`,
					);
				}
				if (holders.length === 1 && holders[0] !== undefined) {
					return holders[0];
				}
				if (holders.length === 0) {
					return refusal(
						`No project the watcher reads holds ${id}. Name its project.`,
					);
				}
				return refusal(
					`${id} is held by more than one project: ${holders.join(", ")}. Name its project.`,
				);
			});

		return {
			learn: (given) =>
				Effect.gen(function* () {
					const id = Schema.decodeUnknownOption(SessionId)(given);
					if (
						Option.isSome(id) &&
						(yield* Deferred.succeed(session, id.value))
					) {
						yield* restore(id.value);
					}
					if (yield* Deferred.isDone(session)) yield* Deferred.await(file);
				}),

			watch: (id, project) =>
				Effect.gen(function* () {
					const found = project ?? (yield* projectHolding(id));
					if (typeof found !== "string") return found;
					const bead = { project: found, id };
					const watch = watches.get(keyOf(bead)) ?? (yield* start(bead, false));
					yield* Deferred.await(watch.firstHeard);
					const refused = watch.accepted ? undefined : refusalOf(watch);
					if (refused !== undefined) {
						if (watches.get(keyOf(bead)) === watch) yield* stop(watch);
						yield* persist;
						return refused;
					}
					if (watches.get(keyOf(bead)) !== watch) {
						return refusal(
							`${named(bead)} was unwatched before the watcher answered.`,
						);
					}
					watch.accepted = true;
					yield* persist;
					const { down, standing } = watch.known;
					if (down === undefined) {
						return told(`Watching ${named(bead)}: ${describe(watch)}.`);
					}
					const isDown = `The watcher is down: ${whyDown[down]}.`;
					if (standing.is === "unheard") {
						return told(
							`Watching ${named(bead)} from when the watcher answers. ${isDown}`,
						);
					}
					return told(
						`Watching ${named(bead)}: ${describe(watch)}, as last known. ${isDown}`,
					);
				}),

			unwatch: (id, project) =>
				Effect.gen(function* () {
					const matching = [...watches.values()].filter(
						({ bead }) =>
							bead.id === id &&
							(project === undefined || bead.project === project),
					);
					const [only] = matching;
					if (only === undefined) {
						return refusal(
							`This session does not watch ${project === undefined ? id : named({ project, id })}.`,
						);
					}
					if (matching.length > 1) {
						return refusal(
							`This session watches ${id} in more than one project: ${matching.map(({ bead }) => bead.project).join(", ")}. Name its project.`,
						);
					}
					yield* stop(only);
					yield* persist;
					return told(`Stopped watching ${named(only.bead)}.`);
				}),

			watching: Effect.sync(() => {
				const all = [...watches.values()];
				if (all.length === 0) return told("This session watches no beads.");
				const down = all.find(({ known }) => known.down !== undefined)?.known
					.down;
				return told(
					[
						down === undefined
							? "The watcher is answering."
							: `The watcher is down: ${whyDown[down]}. Each bead's status is the last one known.`,
						...all.map((watch) => `- ${named(watch.bead)}: ${describe(watch)}`),
					].join("\n"),
				);
			}),
		};
	});
