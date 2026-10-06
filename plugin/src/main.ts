#!/usr/bin/env node
import { homedir } from 'node:os'
import { join } from 'node:path'
import { NodeFileSystem, NodeRuntime } from '@effect/platform-node'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { Config, Effect, Option } from 'effect'
import { buildServer } from './server'
import { projectsNamedIn, whereTheWatcherIs } from './watcher'
import { makeWatches } from './watches'

const config = join(homedir(), '.config', 'beady-eye', 'config.toml')

/** Claude Code starts the server with its session's id, so a server started
 * again under a sleeping session watches its beads before any tool is
 * called. */
const bootSession = Config.option(Config.String('CLAUDE_CODE_SESSION_ID')).pipe(
  Effect.orElseSucceed(() => Option.none<string>()),
  Effect.map(Option.getOrUndefined),
)

/** Settles when the session closes the server's input, which is how a
 * session lets its server go. */
const sessionCloses = (input: NodeJS.ReadableStream) =>
  Effect.callback<void>((resume) => {
    const closed = () => resume(Effect.void)
    input.once('end', closed)
    input.once('close', closed)
    return Effect.sync(() => {
      input.removeListener('end', closed)
      input.removeListener('close', closed)
    })
  })

NodeRuntime.runMain(
  Effect.gen(function* () {
    const watches = yield* makeWatches({
      findWatcher: whereTheWatcherIs(config),
      projects: projectsNamedIn(config),
    })
    yield* watches.learn(yield* bootSession)
    yield* Effect.promise(() => buildServer(watches).connect(new StdioServerTransport()))
    yield* sessionCloses(process.stdin)
  }).pipe(Effect.scoped, Effect.provide(NodeFileSystem.layer)),
)
