#!/usr/bin/env node
import { NodeRuntime } from '@effect/platform-node'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { Effect } from 'effect'
import { buildServer } from './server'

/** Settles when the session closes the server's input, which is how a
 * session lets its server go. */
const sessionCloses = (input: NodeJS.ReadableStream) =>
  Effect.async<void>((resume) => {
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
    yield* Effect.promise(() => buildServer().connect(new StdioServerTransport()))
    yield* sessionCloses(process.stdin)
  }),
  { disablePrettyLogger: true },
)
