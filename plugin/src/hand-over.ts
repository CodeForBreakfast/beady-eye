// The session's id as the plugin's hooks leave it, in a file named for the
// Claude Code process that runs both them and this server. `docs/design.md`'s
// *The server learns its session from the plugin's hooks* is the design.

import { basename } from 'node:path'
import { Effect, FileSystem, Option, Schema, String as Str, Stream } from 'effect'
import { ChildProcess, ChildProcessSpawner } from 'effect/process'
import { sessionFile, sessionsDirectory } from '../hooks/session-file'
import { SessionId } from './watcher'

export interface HandOver {
  /** The id left for the nearest process the server runs under that has
   * one. */
  readonly current: Effect.Effect<SessionId | undefined>
  /** The id each time the hooks leave one for a process the server runs
   * under. */
  readonly changes: Stream.Stream<SessionId | undefined>
}

/** The processes this one runs under, nearest first. Claude Code starts the
 * server with `npx`, so Claude Code is one of them without being the
 * parent. */
const ancestors = Effect.gen(function* () {
  const spawner = yield* ChildProcessSpawner.ChildProcessSpawner
  const table = yield* spawner.lines(ChildProcess.make('ps', ['-A', '-o', 'pid=', '-o', 'ppid=']))
  const parents = new Map(
    table.map((row) => {
      const [pid, ppid] = Str.trim(row).split(/\s+/).map(Number)
      return [pid, ppid] as const
    }),
  )
  const chain: number[] = []
  for (
    let pid = parents.get(process.pid);
    pid !== undefined && pid > 1 && !chain.includes(pid);
    pid = parents.get(pid)
  ) {
    chain.push(pid)
  }
  return chain
}).pipe(Effect.orElseSucceed((): number[] => []))

const decodeSession = Schema.decodeUnknownOption(SessionId)

export const makeHandOver: Effect.Effect<
  HandOver,
  never,
  FileSystem.FileSystem | ChildProcessSpawner.ChildProcessSpawner
> = Effect.gen(function* () {
  const fs = yield* FileSystem.FileSystem
  const directory = sessionsDirectory(process.env)
  const files = (yield* ancestors).map((pid) => sessionFile(directory, pid))
  const ours = new Set(files.map((file) => basename(file)))

  const current = Effect.firstSuccessOf([
    ...files.map((file) => fs.readFileString(file)),
    Effect.succeed(''),
  ]).pipe(
    Effect.map((id) => Option.getOrUndefined(decodeSession(Str.trim(id)))),
    Effect.orElseSucceed(() => undefined),
  )

  const changes = Stream.unwrap(
    fs
      .makeDirectory(directory, { recursive: true, mode: 0o700 })
      .pipe(Effect.as(fs.watch(directory))),
  ).pipe(
    Stream.filter(({ path }) => ours.has(basename(path))),
    Stream.mapEffect(() => current),
    Stream.ignore,
  )

  return { current, changes }
})
