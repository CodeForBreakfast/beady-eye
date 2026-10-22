// What the tests stand in place of a running `bdi watch` with, and the
// watches they point at it.

import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { createServer, type Server, type Socket } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { NodeFileSystem } from '@effect/platform-node'
import {
  ConfigProvider,
  Effect,
  Exit,
  type FileSystem,
  identity,
  Layer,
  Schema,
  Scope,
} from 'effect'
import {
  BeadId,
  ProjectName,
  StatusWord,
  type Timing,
  WatchedProjects,
  WatcherLocation,
  WatcherTiming,
} from './watcher'
import { makeWatches } from './watches'

export const aProject = Schema.decodeSync(ProjectName)
export const anId = Schema.decodeSync(BeadId)
export const aStatus = Schema.decodeSync(StatusWord)

/** What each test leaves behind, for `cleanUpAfterEach` to undo. */
export const cleanUp: (() => unknown)[] = []

export const cleanUpAfterEach = async () => {
  for (const clean of cleanUp.splice(0)) await clean()
}

export const aPrivateDirectory = () => {
  const directory = mkdtempSync(join(tmpdir(), 'beady-eye-watcher-'))
  cleanUp.push(() => rmSync(directory, { recursive: true, force: true }))
  return directory
}

export const said = (...lines: unknown[]) =>
  lines.map((line) => `${JSON.stringify(line)}\n`).join('')

export interface Connection {
  readonly connection: Socket
  readonly asked: string
}

/** A watcher of the test's own, handing over each connection with the lines
 * it asked. */
export const aWatcherAt = async (at: string) => {
  const arrived: Connection[] = []
  const waiting: ((connection: Connection) => void)[] = []
  const held: Socket[] = []
  const server: Server = createServer((connection) => {
    held.push(connection)
    connection.setEncoding('utf8')
    // A session can hang up as a line is written. The write fails and closes
    // the connection, as it does in `bdi watch`, and that is no failure.
    connection.on('error', () => {})
    connection.once('data', (asked) => {
      const accepted = { connection, asked: String(asked) }
      const waiter = waiting.shift()
      if (waiter) waiter(accepted)
      else arrived.push(accepted)
    })
  })
  await new Promise<void>((listening) => server.listen(at, listening))
  cleanUp.push(() => server.close())
  return {
    server,
    /** Stop listening, and close every connection the watcher holds. */
    stop: () => {
      server.close()
      for (const connection of held) connection.destroy()
    },
    next: () =>
      new Promise<Connection>((resolve) => {
        const accepted = arrived.shift()
        if (accepted) resolve(accepted)
        else waiting.push(resolve)
      }),
  }
}

/** A session's watches, kept under `state` through `fileSystem`, reading a
 * bdi config that says `text`, until the test is over. There is no runtime
 * directory, so a config naming no socket leaves the watcher nowhere. */
export const watchesReading = async (
  state: string,
  text: string,
  timing?: Timing,
  fileSystem: Layer.Layer<FileSystem.FileSystem> = NodeFileSystem.layer,
) => {
  const config = join(aPrivateDirectory(), 'config.toml')
  writeFileSync(config, text)
  const scope = Scope.makeUnsafe()
  const watches = await Effect.runPromise(
    makeWatches.pipe(
      Scope.provide(scope),
      Effect.provide(
        Layer.mergeAll(WatcherLocation.namedIn(config), WatchedProjects.namedIn(config)).pipe(
          Layer.provideMerge(fileSystem),
        ),
      ),
      timing === undefined ? identity : Effect.provideService(WatcherTiming, timing),
      Effect.provideService(
        ConfigProvider.ConfigProvider,
        ConfigProvider.fromEnv({ env: { XDG_STATE_HOME: state } }),
      ),
    ),
  )
  const close = () => Effect.runPromise(Scope.close(scope, Exit.void))
  cleanUp.push(close)
  return { watches, close, config }
}

/** A session's watches, kept under `state` through `fileSystem`, looking for
 * the watcher at `at` and for beads in `projects`, until the test is over. */
export const someWatches = (
  state: string,
  at: string | undefined,
  projects: readonly string[],
  timing?: Timing,
  fileSystem?: Layer.Layer<FileSystem.FileSystem>,
) =>
  watchesReading(
    state,
    [
      ...(at === undefined ? [] : [`[watcher]\nsocket = ${JSON.stringify(at)}\n`]),
      ...projects.map((project) => `[[projects]]\nname = ${JSON.stringify(project)}\n`),
    ].join('\n'),
    timing,
    fileSystem,
  )
