// The plugin's side of `bdi watch`. `docs/design.md`'s *Watching* is the
// protocol, and *A quiet watcher, and one that has gone* says when a watcher
// is down.

import { createConnection } from 'node:net'
import { dirname, join, resolve } from 'node:path'
import {
  Array as Arr,
  Cause,
  Config,
  Context,
  Data,
  Duration,
  Effect,
  FileSystem,
  Filter,
  Layer,
  Number as Num,
  Option,
  Queue,
  Ref,
  type Result,
  Schema,
  Stream,
} from 'effect'
import { identity } from 'effect/Function'
import { parse, TomlError } from 'smol-toml'

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

/** The watcher's timings, which a test shortens. */
export const WatcherTiming = Context.Reference<Timing>('beady-eye/WatcherTiming', {
  defaultValue: () => WATCHER_TIMING,
})

/**
 * Why a bead's connection is not watching: the bdi config that says where the
 * watcher is cannot be read, there is no path to look for a watcher at,
 * nothing at the path would take the connection, what is at the path could
 * not be believed, the watcher closed it, it carried no line for too long, or
 * it spoke a protocol this plugin does not know.
 */
export type Down =
  | ConfigUnreadable
  | 'nowhere'
  | 'refused'
  | 'untrusted'
  | 'closed'
  | 'wedged'
  | 'protocol'

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
]).pipe(Schema.toTaggedUnion('line'))

/** One line the watcher sent about a watch. */
export type Said = typeof SaidLine.Type

/** What a watched bead's connection hears: the lines of one answer for the
 * bead's project, ending with its freshness line, or a refused line on its
 * own; or that the watcher is down. */
export type Heard = Result.Result<readonly Said[], Down>

export const Bead = Schema.Struct({ project: ProjectName, id: BeadId })
export type Bead = typeof Bead.Type

/** The bdi config at `config` is there but cannot be read: the file will not
 * open, is not TOML, or is not shaped as bdi reads it. */
export class ConfigUnreadable extends Data.TaggedError('ConfigUnreadable')<{
  readonly config: string
  readonly reason: string
}> {}

const WatcherSection = Schema.Struct({
  watcher: Schema.optionalKey(Schema.Struct({ socket: Schema.optionalKey(Schema.String) })),
})

const ProjectsSection = Schema.Struct({
  projects: Schema.optionalKey(Schema.Array(Schema.Struct({ name: Schema.String }))),
})

/** Why `parse` refused the text, on one line. */
const whyNotToml = (error: unknown) =>
  error instanceof TomlError
    ? `${error.message.split('\n')[0]}, at line ${error.line}, column ${error.column}`
    : String(error)

/** What the bdi config at `config` says, as `section` reads it. A missing
 * config file names nothing. */
const readIn = <A>(config: string, section: Schema.Codec<A, unknown>) => {
  const unreadable = (reason: string) => new ConfigUnreadable({ config, reason })
  return Effect.gen(function* () {
    const fs = yield* FileSystem.FileSystem
    const text = yield* fs.readFileString(config).pipe(
      Effect.catchReason(
        'PlatformError',
        'NotFound',
        () => Effect.succeed(''),
        (_, error) => Effect.fail(unreadable(error.message)),
      ),
    )
    const read = yield* Effect.try({
      try: () => parse(text),
      catch: (error) => unreadable(whyNotToml(error)),
    })
    return yield* Schema.decodeEffect(section)(read).pipe(
      Effect.mapError(({ message }) => unreadable(message.replace(/\s*\n\s*/g, ' '))),
    )
  })
}

const socketNamedIn = (config: string) =>
  readIn(config, WatcherSection).pipe(
    Effect.map(({ watcher }) => Option.fromNullishOr(watcher?.socket)),
  )

/** The name of each of the bdi config's `[[projects]]` the watcher can be
 * asked about. */
export const projectsNamedIn = (
  config: string,
): Effect.Effect<readonly ProjectName[], ConfigUnreadable, FileSystem.FileSystem> =>
  readIn(config, ProjectsSection).pipe(
    Effect.map(({ projects = [] }) =>
      Arr.filter(
        projects.map(({ name }) => name),
        Schema.is(ProjectName),
      ),
    ),
  )

const runtimeDirectory = Config.NonEmptyString('XDG_RUNTIME_DIR').pipe(Config.option)

/**
 * Where `bdi watch` takes its socket: `[watcher]`'s `socket` in the bdi
 * config, or under the runtime directory where the config names none.
 */
export const whereTheWatcherIs = (
  config: string,
): Effect.Effect<Option.Option<string>, ConfigUnreadable, FileSystem.FileSystem> =>
  Effect.gen(function* () {
    const named = yield* socketNamedIn(config)
    if (Option.isSome(named)) return named
    return Option.map(yield* Effect.orDie(runtimeDirectory), (directory) =>
      join(directory, 'beady-eye', 'watcher.sock'),
    )
  })

/** The projects a bead named without its project is looked for in, read
 * again for each bead. */
export class WatchedProjects extends Context.Service<
  WatchedProjects,
  { readonly list: Effect.Effect<readonly ProjectName[], ConfigUnreadable> }
>()('beady-eye/WatchedProjects') {
  /** The projects the bdi config at `config` names. */
  static readonly namedIn = (config: string) =>
    Layer.effect(
      WatchedProjects,
      Effect.gen(function* () {
        const fs = yield* FileSystem.FileSystem
        return {
          list: projectsNamedIn(config).pipe(Effect.provideService(FileSystem.FileSystem, fs)),
        }
      }),
    )
}

/** Where to look for the watcher's socket, asked again for each connection. */
export class WatcherLocation extends Context.Service<
  WatcherLocation,
  { readonly find: Effect.Effect<Option.Option<string>, ConfigUnreadable> }
>()('beady-eye/WatcherLocation') {
  /** Where the bdi config at `config` says the watcher is. */
  static readonly namedIn = (config: string) =>
    Layer.effect(
      WatcherLocation,
      Effect.gen(function* () {
        const fs = yield* FileSystem.FileSystem
        return {
          find: whereTheWatcherIs(config).pipe(Effect.provideService(FileSystem.FileSystem, fs)),
        }
      }),
    )
}

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

const KINDS_READ = new Set<string>(SaidLine.discriminants)

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
const talkTo = (asking: readonly string[]) =>
  Stream.unwrap(
    Effect.gen(function* () {
      const timing = yield* WatcherTiming
      const at = yield* (yield* WatcherLocation).find.pipe(Effect.mapError(down))
      if (Option.isNone(at)) return yield* down('nowhere')
      const notBelieved = yield* whyNotBelieved(at.value)
      if (Option.isSome(notBelieved)) return yield* down(notBelieved.value)
      return linesFrom(at.value, asking).pipe(
        Stream.timeoutOrElse({
          duration: timing.wedgedAfter,
          orElse: () => Stream.fail(down('wedged')),
        }),
        Stream.mapEffect(saidIn),
        Stream.filterMap(Filter.fromPredicateOption(identity)),
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
export const watchBead = (
  bead: Bead,
): Stream.Stream<Heard, never, WatcherLocation | FileSystem.FileSystem> =>
  Stream.unwrap(
    Effect.gen(function* () {
      const timing = yield* WatcherTiming
      const failures = yield* Ref.make(0)
      const connection = answersIn(talkTo([watchLine(bead)])).pipe(
        Stream.tap((batch) =>
          batch.at(-1)?.line === 'freshness' ? Ref.set(failures, 0) : Effect.void,
        ),
        Stream.mapError(({ why }) => why),
        Stream.result,
      )
      const pause = Ref.updateAndGet(failures, Num.increment).pipe(
        Effect.flatMap((failed) => Effect.sleep(pauseAfter(failed, timing))),
      )
      return connection.pipe(Stream.concat(Stream.fromEffectDrain(pause)), Stream.forever)
    }),
  )

/** What one project answered when asked about a bead: it holds the bead, the
 * bead has gone from it, the watcher has not read it yet, the watcher refused
 * to look in it, or it had not answered when the time to answer ran out. */
export type ProjectAnswer = Data.TaggedEnum<{
  Holds: Record<never, never>
  Gone: Record<never, never>
  NotReadYet: Record<never, never>
  Refused: { readonly reason: string }
  Unanswered: Record<never, never>
}>
export const ProjectAnswer = Data.taggedEnum<ProjectAnswer>()

/** Each project's answer so far, and what each project still answering has
 * said of the bead before its freshness line. */
interface Asking {
  readonly answers: ReadonlyMap<ProjectName, ProjectAnswer>
  readonly heard: ReadonlyMap<ProjectName, ProjectAnswer>
}

/**
 * Ask the watcher once about the bead `id` in each of `projects`, reading as
 * far as each project's freshness line or refusal, and hang up. A watcher
 * keeps a project waiting until its first read, so the asking stops at
 * the timing's `answeredWithin` whatever has arrived.
 */
export const askAbout = (
  id: BeadId,
  projects: readonly ProjectName[],
): Effect.Effect<
  ReadonlyMap<ProjectName, ProjectAnswer>,
  WatcherDown,
  WatcherLocation | FileSystem.FileSystem
> => {
  const asking = new Map(projects.map((project) => [watchLine({ project, id }), project]))
  const unasked: Asking = {
    answers: new Map(projects.map((project) => [project, ProjectAnswer.Unanswered()])),
    heard: new Map(),
  }
  if (asking.size === 0) return Effect.succeed(unasked.answers)
  const take = (state: Asking, said: Said): Asking => {
    const project = said.line === 'refused' ? asking.get(said.asked) : said.project
    if (project === undefined || state.answers.get(project)?._tag !== 'Unanswered') return state
    const settle = (answer: ProjectAnswer) => ({
      ...state,
      answers: new Map(state.answers).set(project, answer),
    })
    const hear = (heard: ProjectAnswer) => ({
      ...state,
      heard: new Map(state.heard).set(project, heard),
    })
    switch (said.line) {
      case 'refused':
        return settle(ProjectAnswer.Refused({ reason: said.reason }))
      case 'freshness':
        return settle(state.heard.get(project) ?? ProjectAnswer.NotReadYet())
      case 'bead':
        return hear(ProjectAnswer.Holds())
      case 'gone':
        return hear(ProjectAnswer.Gone())
      case 'event':
        return state
    }
  }
  return talkTo([...asking.keys()]).pipe(
    Stream.scan(() => unasked, take),
    Stream.takeUntil(({ answers }) =>
      [...answers.values()].every((answer) => answer._tag !== 'Unanswered'),
    ),
    Stream.interruptWhen(
      Effect.flatMap(WatcherTiming, ({ answeredWithin }) => Effect.sleep(answeredWithin)),
    ),
    Stream.runLast,
    Effect.map(Option.match({ onNone: () => unasked.answers, onSome: ({ answers }) => answers })),
  )
}
