import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import {
	CallToolRequestSchema,
	type CallToolResult,
	ListToolsRequestSchema,
	type Tool,
} from "@modelcontextprotocol/sdk/types.js";
import { Data, Effect } from "effect";
import { version } from "../package.json";
import type { Answer, Watches } from "./watches";

const aBead: Tool["inputSchema"] = {
	type: "object",
	properties: {
		id: { type: "string", description: "The bead's id." },
		project: {
			type: "string",
			description: "The name the bdi config gives the bead's project.",
		},
	},
	required: ["id"],
};

const tools: Tool[] = [
	{
		name: "watch",
		description:
			"Watch a bead, so that its changes arrive in this session as messages. Answers with the bead as it stands. Without a project, the watcher is asked which project holds the id.",
		inputSchema: aBead,
	},
	{
		name: "unwatch",
		description:
			"Stop watching a bead. Name its project where this session watches the id in more than one.",
		inputSchema: aBead,
	},
	{
		name: "watching",
		description:
			"List the beads this session watches, each with its last known status, and say whether the watcher is answering.",
		inputSchema: { type: "object", properties: {} },
	},
];

/** A word the watcher can read on one line: something, with no space in it. */
const word = (value: unknown): string | undefined =>
	typeof value === "string" && /^\S+$/.test(value) ? value : undefined;

const answering = ({ text, refused }: Answer): CallToolResult => ({
	content: [{ type: "text", text }],
	isError: refused,
});

const call = (
	watches: Watches,
	name: string,
	args: Record<string, unknown>,
): Effect.Effect<Answer> => {
	if (name === "watching") return watches.watching;
	const id = word(args.id);
	const project = word(args.project);
	if (
		id === undefined ||
		(args.project !== undefined && project === undefined)
	) {
		return Effect.succeed({
			text: "A bead's id and project are each one word with no space in it.",
			refused: true,
		});
	}
	if (name === "watch") return watches.watch(id, project);
	if (name === "unwatch") return watches.unwatch(id, project);
	return Effect.succeed({ text: `There is no tool ${name}.`, refused: true });
};

export const buildServer = (watches: Watches): Server => {
	const server = new Server(
		{ name: "beady-eye", version },
		{ capabilities: { tools: {}, experimental: { "claude/channel": {} } } },
	);
	server.setRequestHandler(ListToolsRequestSchema, () => ({ tools }));
	server.setRequestHandler(CallToolRequestSchema, ({ params }) => {
		const args = params.arguments ?? {};
		return Effect.runPromise(
			watches
				.learn(
					typeof args.session_id === "string" ? args.session_id : undefined,
				)
				.pipe(
					Effect.zipRight(call(watches, params.name, args)),
					Effect.map(answering),
				),
		);
	});
	return server;
};

export class SessionNotTold extends Data.TaggedError("SessionNotTold")<{
	readonly cause: unknown;
}> {}

/**
 * Puts a message into the session as a `<channel source="beady-eye" …>`
 * block, each `meta` entry becoming one of its attributes. Claude Code drops
 * it unless the session was started with this plugin's channel allowed.
 */
export const tellSession = (
	server: Server,
	content: string,
	meta: Record<string, string>,
): Effect.Effect<void, SessionNotTold> =>
	Effect.tryPromise({
		try: () =>
			server.notification({
				method: "notifications/claude/channel",
				params: { content, meta },
			}),
		catch: (cause) => new SessionNotTold({ cause }),
	});
