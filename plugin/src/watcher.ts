// The plugin's side of `bdi watch`. `docs/design.md`'s *Watching* is the
// protocol, and *A quiet watcher, and one that has gone* says when a watcher
// is down.

import { createConnection } from 'node:net'
import { dirname, join, resolve } from 'node:path'
import {
  Cause,
  Config,
  Data,
  Duration,
  Effect,
  FileSystem,
  Option,
  Queue,
  Ref,
  Schema,
  Stream,
} from 'effect'
import { parse } from 'smol-toml'

/** The version of the watcher's lines this plugin reads. */
export const PROTOCOL = 1

export interface Timing {
  /** How long a connection may carry no line before it has wedged. The
   * watcher sends an alive line every 20 seconds. */
  readonly wedgedAfter: Duration.Duration
  readonly firstPause: Duration.Duration
  readonly longestPause: Duration.Duration
  /** How long a question asked once may wait for every project's answer. */
  readonly answeredWithin: Duration.Duration
  /** How long the watcher or a tracker may be down before a session is told.
   * The watcher restarts on each upgrade and each edit to its config. */
  readonly quietFor: Duration.Duration
}

export const WATCHER_TIMING: Timing = {
  wedgedAfter: Duration.minutes(1),
  firstPause: Duration.seconds(1),
  longestPause: Duration.seconds(30),
  answeredWithin: Duration.seconds(15),
  quietFor: Duration.minutes(1),
}

/**
 * Why a bead's connection is not watching: there is no path to look for a
 * watcher at, nothing at the path would take the connection, what is at the
 * path could not be believed, the watcher closed it, it carried no line for
 * too long, or it spoke a protocol this plugin does not know.
 */
export type Down = 'nowhere' | 'refused' | 'untrusted' | 'closed' | 'wedged' | 'protocol'

export class WatcherDown extends Data.TaggedError('WatcherDown')<{
  readonly why: Down
}> {}

/** A word the watcher can read on one line: something, with no space in it.
 * The plugin asks `watch <project> <id>` and keys a watch on the two. */
const aWord = Schema.String.check(Schema.isPattern(/^\S+$/))

/** The name the bdi config gives a project. */
export const ProjectName = aWord.pipe(Schema.brand('ProjectName'))
export type ProjectName = typeof ProjectName.Type

/** A bead's id in its tracker. */
export const BeadId = aWord.pipe(Schema.brand('BeadId'))
export type BeadId = typeof BeadId.Type

/** The fields of bd's row this plugin reads. */
const Row = Schema.Struct({
  id: BeadId,
  title: Schema.String,
  status: Schema.String,
  comment_count: Schema.optional(Schema.Finite),
  close_reason: Schema.optional(Schema.String),
})

/** One of bd's event records. Only a comment's record carries a comment. */
const EventRecord = Schema.Struct({
  op: Schema.String,
  issue_id: BeadId,
  comment: Schema.optional(Schema.Struct({ author: Schema.String, text: Schema.String })),
}).check(Schema.makeFilter(({ op, comment }) => (op === 'comment') === (comment !== undefined)))

/** Each kind of line about a watch that the plugin reads, with the fields it
 * reads. The watcher's alive lines are not among them: a connection that
 * stops carrying any line has wedged, whatever the line. */
const SaidLine = Schema.Union([
  Schema.Struct({
    line: Schema.Literal('bead'),
    project: ProjectName,
    ready: Schema.Boolean,
    row: Row,
  }),
  Schema.Struct({ line: Schema.Literal('gone'), project: ProjectName, id: BeadId }),
  Schema.Struct({
    line: Schema.Literal('freshness'),
    project: ProjectName,
    protocol: Schema.Literal(PROTOCOL),
    tracker: Schema.Union([
      Schema.Literal('ok'),
      Schema.Struct({ unreachable: Schema.Struct({ reason: Schema.String }) }),
    ]),
  }),
  Schema.Struct({ line: Schema.Literal('event'), project: ProjectName, event: EventRecord }),
  Schema.Struct({ line: Schema.Literal('refused'), asked: Schema.String, reason: Schema.String }),
])

/** One line the watcher sent about a watch. */
export type Said = typeof SaidLine.Type

/** What a watched bead's connection hears: the lines of one answer for the
 * bead's project, ending with its freshness line, or a refused line on its
 * own; or that the watcher is down. */
export type Heard = { readonly batch: readonly Said[] } | { readonly down: Down }

export const Bead = Schema.Struct({ project: ProjectName, id: BeadId })
export type Bead = typeof Bead.Type

const WatcherSection = Schema.Struct({
  watcher: Schema.Struct({ socket: Schema.String }),
})

const ProjectsSection = Schema.Struct({
  projects: Schema.Array(Schema.Struct({ name: Schema.String })),
})

/** What the bdi config at `config` says, as `section` reads it. */
const readIn = <A>(config: string, section: Schema.Codec<A, unknown>) =>
  Effect.gen(function* () {
    const fs = yield* FileSystem.FileSystem
    const text = yield* fs.readFileString(config)
    const read = yield* Effect.try(() => parse(text))
    return yield* Schema.decodeEffect(section)(read)
  }).pipe(Effect.option)

const socketNamedIn = (config: string) =>
  readIn(config, WatcherSection).pipe(Effect.map(Option.map(({ watcher }) => watcher.socket)))

/** The name of each of the bdi config's `[[projects]]` the watcher can be
 * asked about. */
export const projectsNamedIn = (
  config: string,
): Effect.Effect<readonly ProjectName[], never, FileSystem.FileSystem> =>
  readIn(config, ProjectsSection).pipe(
    Effect.map(
      Option.match({
        onNone: () => [],
        onSome: ({ projects }) =>
          projects.flatMap(({ name }) => (Schema.is(ProjectName)(name) ? [name] : [])),
      }),
    ),
  )

const runtimeDirectory = Config.option(Config.NonEmptyString('XDG_RUNTIME_DIR')).pipe(
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
 * Why the socket at `at` cannot be believed, if it cannot: there is no file
 * there, or it is not one only this user could have put there, a socket of
 * the user's own under a way down nobody else may take a name in, judged both
 * as spelled and as resolved. `bdi` makes the same checks before it believes
 * a watcher.
 */
const whyNotBelieved = (
  at: string,
): Effect.Effect<Option.Option<Down>, never, FileSystem.FileSystem> =>
  Effect.gen(function* () {
    const fs = yield* FileSystem.FileSystem
    const untrusted = Option.some<Down>('untrusted')
    if (yield* Effect.isSuccess(fs.readLink(at))) return untrusted
    const socket = yield* fs.stat(at)
    const user = process.geteuid?.()
    if (user === undefined) return untrusted
    const under = dirname(resolve(at))
    const ways = [...directoriesOn(under), ...directoriesOn(yield* fs.realPath(under))]
    for (const directory of ways) {
      const { mode, uid } = yield* fs.stat(directory)
      if (Option.isNone(uid) || othersMayTakeANameIn(mode, uid.value, user)) {
        return untrusted
      }
    }
    const believed = socket.type === 'Socket' && Option.contains(socket.uid, user)
    return believed ? Option.none() : untrusted
  }).pipe(
    Effect.catchReason(
      'PlatformError',
      'NotFound',
      () => Effect.succeedSome<Down>('refused'),
      () => Effect.succeedSome<Down>('untrusted'),
    ),
  )

export const pauseAfter = (failures: number, timing: Timing): Duration.Duration =>
  Duration.min(Duration.times(timing.firstPause, 2 ** (failures - 1)), timing.longestPause)

const down = (why: Down) => new WatcherDown({ why })

const AnyLine = Schema.fromJsonString(
  Schema.StructWithRest(Schema.Struct({ line: Schema.String }), [
    Schema.Record(Schema.String, Schema.Unknown),
  ]),
)

const KINDS_READ = new Set<string>(SaidLine.members.map(({ fields }) => fields.line.literal))

/** What `text` says, where it is a kind of line the plugin reads. A newer
 * watcher may send a kind this plugin does not know, which is passed over,
 * but a kind it does know without the fields that kind carries is a watcher
 * speaking another protocol. */
const saidIn = (text: string) =>
  Schema.decodeEffect(AnyLine)(text).pipe(
    Effect.flatMap((line) =>
      KINDS_READ.has(line.line)
        ? Effect.asSome(Schema.decodeUnknownEffect(SaidLine)(line))
        : Effect.succeedNone,
    ),
    Effect.mapError(() => down('protocol')),
  )

/** Every line the watcher sends after it is sent `asking`, until the
 * connection is down. */
const linesFrom = (at: string, asking: readonly string[]) =>
  Stream.callback<string, WatcherDown>((lines) =>
    Effect.gen(function* () {
      const connection = createConnection(at)
      let connected = false
      let pending = ''
      connection.setEncoding('utf8')
      connection.on('connect', () => {
        connected = true
        connection.write(asking.map((line) => `${line}\n`).join(''))
      })
      connection.on('data', (chunk: string) => {
        const arrived = (pending + chunk).split('\n')
        pending = arrived.pop() ?? ''
        Queue.offerAllUnsafe(lines, arrived)
      })
      connection.on('error', () =>
        Queue.failCauseUnsafe(lines, Cause.fail(down(connected ? 'closed' : 'refused'))),
      )
      connection.on('close', () => Queue.failCauseUnsafe(lines, Cause.fail(down('closed'))))
      yield* Effect.addFinalizer(() => Effect.sync(() => connection.destroy()))
    }),
  )

/**
 * Connect to the watcher, send it `asking`, and read every line it sends of a
 * kind the plugin reads, until the connection is down.
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
      const notBelieved = yield* whyNotBelieved(at.value)
      if (Option.isSome(notBelieved)) return yield* down(notBelieved.value)
      return linesFrom(at.value, asking).pipe(
        Stream.timeoutOrElse({
          duration: timing.wedgedAfter,
          orElse: () => Stream.fail(down('wedged')),
        }),
        Stream.mapEffect(saidIn),
        Stream.filter(Option.isSome),
        Stream.map(({ value }) => value),
      )
    }),
  )

/** The lines of each answer, ending with its freshness line, and each
 * refused line on its own. */
const answersIn = <E, R>(lines: Stream.Stream<Said, E, R>) =>
  lines.pipe(
    Stream.mapAccum(
      (): readonly Said[] => [],
      (batch, said): [readonly Said[], readonly (readonly Said[])[]] => {
        if (said.line === 'refused') return [batch, [[said]]]
        if (said.line === 'freshness') return [[], [[...batch, said]]]
        return [[...batch, said], []]
      },
    ),
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
        Stream.catch(({ why }) => Stream.make<Heard[]>({ down: why })),
      )
      const pause = Ref.updateAndGet(failures, (failed) => failed + 1).pipe(
        Effect.flatMap((failed) => Effect.sleep(pauseAfter(failed, timing))),
      )
      return connection.pipe(Stream.concat(Stream.fromEffectDrain(pause)), Stream.forever)
    }),
  )

export interface Asked {
  /** Each project's answer, up to its freshness line, and each refusal. */
  readonly lines: readonly Said[]
  /** The projects that had not answered when the time to answer ran out. */
  readonly unanswered: readonly ProjectName[]
}

/**
 * Ask the watcher once about the bead `id` in each of `projects`, reading as
 * far as each project's freshness line or refusal, and hang up. A watcher
 * keeps a project waiting until its first read, so the asking stops at
 * `timing.answeredWithin` whatever has arrived.
 */
export const askAbout = <R>(
  id: BeadId,
  projects: readonly ProjectName[],
  findWatcher: Effect.Effect<Option.Option<string>, never, R>,
  timing: Timing = WATCHER_TIMING,
): Effect.Effect<Asked, WatcherDown, R | FileSystem.FileSystem> => {
  const asking = new Map(projects.map((project) => [watchLine({ project, id }), project]))
  const unasked: Asked = { lines: [], unanswered: [...asking.values()] }
  if (asking.size === 0) return Effect.succeed(unasked)
  const take = (asked: Asked, said: Said): Asked => {
    const project = said.line === 'refused' ? asking.get(said.asked) : said.project
    if (said.line !== 'refused' && !asked.unanswered.includes(said.project)) {
      return asked
    }
    const answered = said.line === 'refused' || said.line === 'freshness'
    return {
      lines: [...asked.lines, said],
      unanswered: answered
        ? asked.unanswered.filter((waiting) => waiting !== project)
        : asked.unanswered,
    }
  }
  return talkTo(findWatcher, [...asking.keys()], timing).pipe(
    Stream.scan(() => unasked, take),
    Stream.takeUntil((asked) => asked.unanswered.length === 0),
    Stream.interruptWhen(Effect.sleep(timing.answeredWithin)),
    Stream.runLast,
    Effect.map(Option.getOrElse(() => unasked)),
  )
}
