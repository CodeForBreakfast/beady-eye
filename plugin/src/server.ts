import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { Data, Effect } from "effect";
import { version } from "../package.json";

export const buildServer = (): Server =>
	new Server(
		{ name: "beady-eye", version },
		{ capabilities: { experimental: { "claude/channel": {} } } },
	);

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
