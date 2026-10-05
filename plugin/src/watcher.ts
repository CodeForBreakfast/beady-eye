// The plugin's side of `bdi watch`. `docs/design.md`'s *Watching* is the
// protocol, and *A quiet watcher, and one that has gone* says when a watcher
// is down.

import { createConnection } from 'node:net'
import { dirname, join, resolve } from 'node:path'
import { FileSystem } from '@effect/platform'
import { Chunk, Config, Data, Effect, Option, Ref, Schema, Stream } from 'effect'
import { parse } from 'smol-toml'

/** The version of the watcher's lines this plugin reads. */
export const PROTOCOL = 1

export interface Timing {
  /** How long a connection may carry no line before it has wedged. The
   * watcher sends an alive line every 20 seconds. */
  readonly wedgedAfter: number
  readonly firstPause: number
  readonly longestPause: number
}

export const WATCHER_TIMING: Timing = {
  wedgedAfter: 60_000,
  firstPause: 1_000,
  longestPause: 30_000,
}

/**
 * Why a bead's connection is not watching: there is no path to look for a
 * watcher at, nothing at the path would take the connection or could be
 * believed, the watcher closed it, it carried no line for too long, or it
 * spoke a protocol this plugin does not know.
 */
export type Down = 'nowhere' | 'refused' | 'closed' | 'wedged' | 'protocol'

class WatcherDown extends Data.TaggedError('WatcherDown')<{
  readonly why: Down
}> {}

/** One line the watcher sent about a watch. */
export interface Said {
  readonly line: string
  readonly [field: string]: unknown
}

/** What a watched bead's connection hears: the lines of one answer for the
 * bead's project, ending with its freshness line, or a refused line on its
 * own; or that the watcher is down. */
export type Heard = { readonly batch: readonly Said[] } | { readonly down: Down }

export interface Bead {
  readonly project: string
  readonly id: string
}

const WatcherSection = Schema.Struct({
  watcher: Schema.Struct({ socket: Schema.String }),
})

const socketNamedIn = (config: string) =>
  Effect.gen(function* () {
    const fs = yield* FileSystem.FileSystem
    const text = yield* fs.readFileString(config)
    const read = yield* Effect.try(() => parse(text))
    return (yield* Schema.decodeUnknown(WatcherSection)(read)).watcher.socket
  }).pipe(Effect.option)

const runtimeDirectory = Config.option(Config.nonEmptyString('XDG_RUNTIME_DIR')).pipe(
  Effect.orElseSucceed(() => Option.none<string>()),
)

/**
 * Where `bdi watch` takes its socket: `[watcher]`'s `socket` in the bdi
 * config, or under the runtime directory where the config names none.
 */
export const whereTheWatcherIs = (
  config: string,
): Effect.Effect<Option.Option<string>, never, FileSystem.FileSystem> =>
  Effect.gen(function* () {
    const named = yield* socketNamedIn(config)
    if (Option.isSome(named)) return named
    return Option.map(yield* runtimeDirectory, (directory) =>
      join(directory, 'beady-eye', 'watcher.sock'),
    )
  })

const A_GROUP_MAY_TAKE_NAMES = 0o030
const ANYBODY_MAY_TAKE_NAMES = 0o003
const NAMES_STAY_THEIR_OWNERS = 0o1000
const THE_SYSTEM = 0

/**
 * Whether somebody other than `user` could put their own file at a name in a
 * directory with this mode and owner. An owner may take any name in their own
 * directory, and anybody who may write and search there may too, unless the
 * sticky bit keeps each name its owner's.
 */
export const othersMayTakeANameIn = (mode: number, owner: number, user: number): boolean => {
  if (owner !== user && owner !== THE_SYSTEM) return true
  const anybodyElse =
    (mode & A_GROUP_MAY_TAKE_NAMES) === A_GROUP_MAY_TAKE_NAMES ||
    (mode & ANYBODY_MAY_TAKE_NAMES) === ANYBODY_MAY_TAKE_NAMES
  return anybodyElse && (mode & NAMES_STAY_THEIR_OWNERS) === 0
}

/** The directory and every one above it but the root, nearest first. */
const directoriesOn = (way: string): string[] => {
  const directories = [way]
  let above = dirname(way)
  while (dirname(above) !== above) {
    directories.push(above)
    above = dirname(above)
  }
  return directories
}

/**
 * Whether the socket at `at` is one only this user could have put there: a
 * socket of the user's own, under a way down nobody else may take a name in,
 * judged both as spelled and as resolved. `bdi` makes the same checks before
 * it believes a watcher.
 */
const onlyThisUserHolds = (at: string) =>
  Effect.gen(function* () {
    const user = process.geteuid?.()
    if (user === undefined) return false
    const fs = yield* FileSystem.FileSystem
    const under = dirname(resolve(at))
    const ways = [...directoriesOn(under), ...directoriesOn(yield* fs.realPath(under))]
    for (const directory of ways) {
      const { mode, uid } = yield* fs.stat(directory)
      if (Option.isNone(uid) || othersMayTakeANameIn(mode, uid.value, user)) {
        return false
      }
    }
    if (yield* Effect.isSuccess(fs.readLink(at))) return false
    const socket = yield* fs.stat(at)
    return socket.type === 'Socket' && Option.contains(socket.uid, user)
  }).pipe(Effect.orElseSucceed(() => false))

export const pauseAfter = (failures: number, timing: Timing): number =>
  Math.min(timing.firstPause * 2 ** (failures - 1), timing.longestPause)

const down = (why: Down) => new WatcherDown({ why })

const SaidLine = Schema.parseJson(
  Schema.Struct(
    { line: Schema.String },
    Schema.Record({ key: Schema.String, value: Schema.Unknown }),
  ),
)

const saidIn = (line: string) =>
  Schema.decodeUnknown(SaidLine)(line).pipe(
    Effect.filterOrFail((said) => said.line !== 'freshness' || said['protocol'] === PROTOCOL),
    Effect.mapError(() => down('protocol')),
  )

/** Every line the watcher sends after it is sent `asking`, until the
 * connection is down. */
const linesFrom = (at: string, asking: readonly string[]) =>
  Stream.async<string, WatcherDown>((emit) => {
    const connection = createConnection(at)
    let connected = false
    let pending = ''
    connection.setEncoding('utf8')
    connection.on('connect', () => {
      connected = true
      connection.write(asking.map((line) => `${line}\n`).join(''))
    })
    connection.on('data', (chunk: string) => {
      const lines = (pending + chunk).split('\n')
      pending = lines.pop() ?? ''
      if (lines.length > 0) emit.chunk(Chunk.unsafeFromArray(lines))
    })
    connection.on('error', () => emit.fail(down(connected ? 'closed' : 'refused')))
    connection.on('close', () => emit.fail(down('closed')))
    return Effect.sync(() => connection.destroy())
  }, 'unbounded')

/**
 * Connect to the watcher, send it `asking`, and read every line it sends but
 * an alive line, until the connection is down.
 */
const talkTo = <R>(
  findWatcher: Effect.Effect<Option.Option<string>, never, R>,
  asking: readonly string[],
  timing: Timing,
) =>
  Stream.unwrap(
    Effect.gen(function* () {
      const at = yield* findWatcher
      if (Option.isNone(at)) return yield* down('nowhere')
      if (!(yield* onlyThisUserHolds(at.value))) return yield* down('refused')
      return linesFrom(at.value, asking).pipe(
        Stream.timeoutFail(() => down('wedged'), timing.wedgedAfter),
        Stream.mapEffect(saidIn),
        Stream.filter((said) => said.line !== 'alive'),
      )
    }),
  )

/** The lines of each answer, ending with its freshness line, and each
 * refused line on its own. */
const answersIn = <E, R>(lines: Stream.Stream<Said, E, R>) =>
  lines.pipe(
    Stream.mapAccum(
      [] as readonly Said[],
      (batch, said): [readonly Said[], readonly (readonly Said[])[]] => {
        if (said.line === 'refused') return [batch, [[said]]]
        if (said.line === 'freshness') return [[], [[...batch, said]]]
        return [[...batch, said], []]
      },
    ),
    Stream.flattenIterables,
  )

const watchLine = (bead: Bead) => `watch ${bead.project} ${bead.id}`

/**
 * What a connection to the watcher that watches one bead hears, connecting
 * again with a growing pause whenever it is down. The protocol has no line to
 * stop a watch, so each bead has a connection of its own, and it closes when
 * the stream stops being read.
 */
export const watchBead = <R>(
  bead: Bead,
  findWatcher: Effect.Effect<Option.Option<string>, never, R>,
  timing: Timing = WATCHER_TIMING,
): Stream.Stream<Heard, never, R | FileSystem.FileSystem> =>
  Stream.unwrap(
    Effect.map(Ref.make(0), (failures) => {
      const connection = answersIn(talkTo(findWatcher, [watchLine(bead)], timing)).pipe(
        Stream.tap((batch) =>
          batch.at(-1)?.line === 'freshness' ? Ref.set(failures, 0) : Effect.void,
        ),
        Stream.map((batch): Heard => ({ batch })),
        Stream.catchAll(({ why }) => Stream.make<Heard[]>({ down: why })),
      )
      const pause = Ref.updateAndGet(failures, (failed) => failed + 1).pipe(
        Effect.flatMap((failed) => Effect.sleep(pauseAfter(failed, timing))),
      )
      return connection.pipe(Stream.concat(Stream.execute(pause)), Stream.forever)
    }),
  )
