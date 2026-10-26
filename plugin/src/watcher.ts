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
 * The plugin asks `watch <project> <id>` and keys a watch on the two. The
 * watcher splits on Rust's whitespace, which counts U+0085 where `\s` does not. */
const aWord = Schema.String.check(Schema.isPattern(/^[^\s\u0085]+$/))

/** The name the bdi config gives a project. */
export const ProjectName = aWord.pipe(Schema.brand('ProjectName'))
export type ProjectName = typeof ProjectName.Type

/** A bead's id in its tracker. */
export const BeadId = aWord.pipe(Schema.brand('BeadId'))
export type BeadId = typeof BeadId.Type

/** The id Claude Code gives the session, which herdr reports as the pane's
 * `agent_session`. */
export const SessionId = Schema.String.check(Schema.isUUID()).pipe(Schema.brand('SessionId'))
export type SessionId = typeof SessionId.Type

/** A status as bd accepts one: lowercase, letter first, with digits, hyphens
 * and underscores after. It is the `statusNameRegexp` in beads' types. */
export const StatusWord = Schema.String.check(Schema.isPattern(/^[a-z][a-z0-9_-]*$/)).pipe(
  Schema.brand('StatusWord'),
)
export type StatusWord = typeof StatusWord.Type

/** How many comments a bead has. */
export const CommentCount = Schema.Int.check(Schema.isGreaterThanOrEqualTo(0))

/** The fields of bd's row this plugin reads. */
const Row = Schema.Struct({
  id: BeadId,
  title: Schema.String,
  status: StatusWord,
  comment_count: Schema.optional(CommentCount),
  close_reason: Schema.optional(Schema.String),
  issue_type: Schema.optional(Schema.String),
  /** bd keeps whatever JSON it was given, so its shape is read where it is
   * used rather than taken for a protocol the plugin does not know. */
  metadata: Schema.optional(Schema.Unknown),
  dependencies: Schema.optional(
    Schema.Array(Schema.Struct({ depends_on_id: Schema.String, type: Schema.String })),
  ),
})

/** A bead as its tracker has it, in the fields this plugin reads. */
export type Row = typeof Row.Type

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

/** The line that closes an answer for a project. */
export type Freshness = Extract<Said, { line: 'freshness' }>

type RefusedLine = Extract<Said, { line: 'refused' }>

/** A line of an answer before its freshness line. */
export type AnswerLine = Exclude<Said, Freshness | RefusedLine>

/** What the watcher sends about one watch at a time: the lines of one answer
 * for the bead's project, closed by its freshness line, or a refused line on
 * its own. */
export type Batch =
  | { readonly is: 'answer'; readonly lines: readonly AnswerLine[]; readonly freshness: Freshness }
  | { readonly is: 'refused'; readonly refused: RefusedLine }

/** What a watched bead's connection hears: a batch, or that the watcher is
 * down. */
export type Heard = Result.Result<Batch, Down>

export const Bead = Schema.Struct({ project: ProjectName, id: BeadId })
export type Bead = typeof Bead.Type

/** The bdi config at `config` is there but cannot be read: the file will not
 * open, is not TOML, or is not shaped as bdi reads it. */
export class ConfigUnreadable extends Data.TaggedError('ConfigUnreadable')<{
  readonly config: string
  readonly reason: string
}> {}

const WatcherSection = Schema.Struct({
  watcher: Schema.optionalKey(Schema.Struct({ socket: Schema.optionalKey(Schema.NonEmptyString) })),
})

const ProjectsSection = Schema.Struct({
  projects: Schema.optionalKey(Schema.Array(Schema.Struct({ name: Schema.String }))),
})

const JoinSection = Schema.Struct({
  join: Schema.optionalKey(
    Schema.Struct({ agent_session_key: Schema.optionalKey(Schema.NonEmptyString) }),
  ),
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

/** The metadata key on a bead that names the agent session working it, read
 * again each time the plugin learns its session. */
export class AgentSessionKey extends Context.Service<
  AgentSessionKey,
  { readonly read: Effect.Effect<string, ConfigUnreadable> }
>()('beady-eye/AgentSessionKey') {
  /** The `[join]` `agent_session_key` the bdi config at `config` names, or
   * bdi's own default. */
  static readonly namedIn = (config: string) =>
    Layer.effect(
      AgentSessionKey,
      Effect.gen(function* () {
        const fs = yield* FileSystem.FileSystem
        return {
          read: readIn(config, JoinSection).pipe(
            Effect.map(({ join }) => join?.agent_session_key ?? 'agent_session'),
            Effect.provideService(FileSystem.FileSystem, fs),
          ),
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
 * connection is down. Each line `sending` offers later goes on the same
 * connection. */
const linesFrom = (at: string, asking: readonly string[], sending: Queue.Dequeue<string>) =>
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
      yield* Queue.take(sending).pipe(
        Effect.flatMap((line) => Effect.sync(() => connection.write(`${line}\n`))),
        Effect.forever,
        Effect.forkScoped,
      )
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

/** The line to send in place of one the watcher refused, where there is one.
 * The refusal then goes no further. */
type AskInstead = (refused: RefusedLine) => string | undefined

/**
 * Connect to the watcher, send it `asking`, and read every line it sends of a
 * kind the plugin reads, until the connection is down.
 */
const talkTo = (asking: readonly string[], instead: AskInstead = () => undefined) =>
  Stream.unwrap(
    Effect.gen(function* () {
      const timing = yield* WatcherTiming
      const at = yield* (yield* WatcherLocation).find.pipe(Effect.mapError(down))
      if (Option.isNone(at)) return yield* down('nowhere')
      const notBelieved = yield* whyNotBelieved(at.value)
      if (Option.isSome(notBelieved)) return yield* down(notBelieved.value)
      const sending = yield* Queue.unbounded<string>()
      return linesFrom(at.value, asking, sending).pipe(
        Stream.timeoutOrElse({
          duration: timing.wedgedAfter,
          orElse: () => Stream.fail(down('wedged')),
        }),
        Stream.mapEffect(saidIn),
        Stream.filterMap(Filter.fromPredicateOption(identity)),
        Stream.filterEffect((said) => {
          const again = said.line === 'refused' ? instead(said) : undefined
          return again === undefined
            ? Effect.succeed(true)
            : Effect.as(Queue.offer(sending, again), false)
        }),
      )
    }),
  )

/** Each answer, closed by its freshness line, and each refused line on its
 * own. */
const answersIn = <E, R>(lines: Stream.Stream<Said, E, R>) =>
  lines.pipe(
    Stream.mapAccum(
      (): readonly AnswerLine[] => [],
      (answer, said): [readonly AnswerLine[], readonly Batch[]] => {
        if (said.line === 'refused') return [answer, [{ is: 'refused', refused: said }]]
        if (said.line === 'freshness') {
          return [[], [{ is: 'answer', lines: answer, freshness: said }]]
        }
        return [[...answer, said], []]
      },
    ),
  )

const watchLine = (bead: Bead) => `watch ${bead.project} ${bead.id}`

/**
 * What a connection to the watcher that watches one bead hears, connecting
 * again with a growing pause whenever it is down. The protocol has no line to
 * stop a watch, so each bead has a connection of its own, and it closes when
 * the stream stops being read.
 *
 * The watch names the latest of `sessions`, and a new one closes the
 * connection and opens another naming it. A watcher older than agent sessions
 * refuses a line naming one as malformed, so the connection then asks again
 * without it. The next connection names the session again, because the
 * watcher it reaches may be a newer one.
 */
export const watchBead = (
  bead: Bead,
  sessions: Stream.Stream<SessionId | undefined> = Stream.succeed(undefined),
): Stream.Stream<Heard, never, WatcherLocation | FileSystem.FileSystem> =>
  Stream.switchMap(sessions, (session) => watchBeadAs(bead, session))

const watchBeadAs = (bead: Bead, session: SessionId | undefined) => {
  const plain = watchLine(bead)
  const asking = session === undefined ? plain : `${plain} ${session}`
  const olderWatcher: AskInstead = ({ asked, reason }) =>
    asking !== plain && asked === asking && reason === 'malformed' ? plain : undefined
  return heardOver(asking, olderWatcher)
}

/** What a connection asking `asking` hears, connecting again with a growing
 * pause whenever it is down. */
const heardOver = (asking: string, instead?: AskInstead) =>
  Stream.unwrap(
    Effect.gen(function* () {
      const timing = yield* WatcherTiming
      const failures = yield* Ref.make(0)
      const connection = answersIn(talkTo([asking], instead)).pipe(
        Stream.tap((batch) => (batch.is === 'answer' ? Ref.set(failures, 0) : Effect.void)),
        Stream.mapError(({ why }) => why),
        Stream.result,
      )
      const pause = Ref.updateAndGet(failures, Num.increment).pipe(
        Effect.flatMap((failed) => Effect.sleep(pauseAfter(failed, timing))),
      )
      return connection.pipe(Stream.concat(Stream.fromEffectDrain(pause)), Stream.forever)
    }),
  )

/** What a connection to the watcher that watches every project it reads
 * hears, starting from each project's beads that are not closed, connecting
 * again with a growing pause whenever it is down. */
export const watchEveryProject: Stream.Stream<
  Heard,
  never,
  WatcherLocation | FileSystem.FileSystem
> = heardOver('watch')

/** What one project answered when asked about a bead: it holds the bead, the
 * bead has gone from it, the watcher has not read it yet, its tracker is
 * unreachable, the watcher refused to look in it, or it had not answered when
 * the time to answer ran out. */
export type ProjectAnswer = Data.TaggedEnum<{
  Holds: Record<never, never>
  Gone: Record<never, never>
  NotReadYet: Record<never, never>
  Unreachable: { readonly reason: string }
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
        return settle(
          state.heard.get(project) ??
            (said.tracker === 'ok'
              ? ProjectAnswer.NotReadYet()
              : ProjectAnswer.Unreachable({ reason: said.tracker.unreachable.reason })),
        )
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
