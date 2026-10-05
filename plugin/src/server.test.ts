import { expect, test } from "bun:test";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import type { JSONRPCNotification } from "@modelcontextprotocol/sdk/types.js";
import { Effect } from "effect";
import { buildServer, tellSession } from "./server";

const pluginRoot = new URL("..", import.meta.url).pathname;
const bundle = `${pluginRoot}dist/server.js`;

const buildTheBundle = () => {
	const build = Bun.spawnSync(["bun", "run", "build"], { cwd: pluginRoot });
	expect(build.exitCode).toBe(0);
};

test("the server sends a channel message to the session", async () => {
	const server = buildServer();
	const client = new Client({ name: "session", version: "0" });
	const received = new Promise<JSONRPCNotification>((resolve) => {
		client.fallbackNotificationHandler = async (notification) =>
			resolve(notification as JSONRPCNotification);
	});
	const [clientSide, serverSide] = InMemoryTransport.createLinkedPair();
	await server.connect(serverSide);
	await client.connect(clientSide);

	await Effect.runPromise(tellSession(server, "bdi-7 closed", { id: "bdi-7" }));

	expect(await received).toEqual({
		jsonrpc: "2.0",
		method: "notifications/claude/channel",
		params: { content: "bdi-7 closed", meta: { id: "bdi-7" } },
	});
	await client.close();
});

test("the bundle runs under node and declares the channel", async () => {
	buildTheBundle();
	const client = new Client({ name: "session", version: "0" });
	await client.connect(
		new StdioClientTransport({
			command: "node",
			args: [bundle],
		}),
	);

	expect(client.getServerCapabilities()?.experimental).toEqual({
		"claude/channel": {},
	});
	await client.close();
});

test("the bundle stops when the session closes its input", async () => {
	buildTheBundle();
	const server = Bun.spawn(["node", bundle], {
		stdin: "pipe",
		stdout: "ignore",
	});

	server.stdin.end();

	expect(await server.exited).toBe(0);
});
