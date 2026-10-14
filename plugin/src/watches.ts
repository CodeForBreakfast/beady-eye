// The beads a session watches, kept on disk under its session id so a server
// started again for the same session watches them again. `docs/design.md`'s
// *Watching a bead* is the design.

import { homedir } from 'node:os'
import { join } from 'node:path'
import {
  Array as Arr,
  Clock,
  Config,
  Deferred,
  Effect,
  Fiber,
  FiberSet,
  FileSystem,
  Match,
  Number as Num,
  Option,
  Predicate,
  Queue,
  Ref,
  Result,
  Schema,
  type Scope,
  Semaphore,
  Stream,
} from 'effect'
import {
  askAbout,
  Bead,
  type BeadId,
  type ConfigUnreadable,
  type Down,
  type Heard,
  type ProjectAnswer,
  type ProjectName,
  type Said,
  WatchedProjects,
  type WatcherLocation,
  WatcherTiming,
  watchBead,
} from './watcher'

/** What a tool says back, and whether it refused what it was asked. */
export interface Answer {
  readonly text: string
  readonly refused: boolean
}

/** The kind of channel block a message arrives in, each carrying the
 * attributes the server's instructions publish for it. */
export type Block =
  | {
      readonly is: 'bead'
      readonly project: ProjectName
      readonly id: BeadId
      readonly status: string
      readonly ready: boolean
    }
  | { readonly is: 'watcher'; readonly watcher: 'down' | 'answering' }
  | {
      readonly is: 'tracker'
      readonly project: ProjectName
      readonly tracker: 'unreachable' | 'ok'
    }
  | { readonly is: 'watches'; readonly watches: 'unreadable' | 'unkept' }

/** A message for the session, and the channel block it arrives in. */
export interface News {
  readonly content: string
  readonly block: Block
  /** Keep what the message told, once the session has been sent it. */
  readonly told: Effect.Effect<void>
}

const Told = Schema.Union([
  Schema.Struct({
    is: Schema.Literal('bead'),
    title: Schema.String,
    status: Schema.String,
    ready: Schema.Boolean,
    comments: Schema.optional(Schema.Finite),
    closeReason: Schema.optional(Schema.String),
  }),
  Schema.Struct({ is: Schema.Literal('gone') }),
])

/** What the session was last told of a bead, which is what the watcher last
 * said of it but for a refusal. */
type Told = typeof Told.Type

/** What the watcher last said of a watched bead. */
type Standing =
  | { readonly is: 'unheard' }
  | Told
  | { readonly is: 'refused'; readonly reason: string }

/** Whether the watcher is watching a bead. `toldDown` says the session has
 * been told the watcher cannot watch it, and not yet that it is back: while
 * it answers again, the bead waits for the others the session was told of. */
type Connection =
  | { readonly is: 'connecting' }
  | { readonly is: 'answering'; readonly toldDown: boolean }
  | {
      readonly is: 'down'
      readonly why: Down
      /** Which outage the connection is down in. */
      readonly outage: number
      readonly toldDown: boolean
    }

/** What is known of a watched bead from its connection. */
interface Known {
  standing: Standing
  /** What the plugin last heard of the bead, kept through a refusal, which
   * the next batch is compared with. */
  heard: Told | undefined
  /** What the session was last told of the bead, which is what is kept on
   * disk, so a change a server never told is told by the next. */
  told: Told | undefined
  connection: Connection
}

/** Why the watcher cannot watch the bead, where it cannot. */
const downBecause = (connection: Connection) =>
  connection.is === 'down' ? connection.why : undefined

const toldDownOf = (connection: Connection) => connection.is !== 'connecting' && connection.toldDown

/** Whether the session has been told it watches a bead. Only a watch it has
 * not been told of can still be refused. An accepting watch's `answer` is done
 * once the session's file keeps it, with why not where it does not. */
type Acceptance =
  | { readonly is: 'unaccepted' }
  | { readonly is: 'accepting'; readonly answer: Deferred.Deferred<Answer | undefined> }
  | { readonly is: 'accepted' }

interface Watch {
  readonly bead: Bead
  readonly known: Known
  acceptance: Acceptance
  /** Done once the watcher has answered, been found down, or the watch has
   * stopped. */
  readonly firstHeard: Deferred.Deferred<void>
  readonly connection: Fiber.Fiber<void>
}

const isAccepted = ({ acceptance }: Watch) => acceptance.is === 'accepted'

export const SessionId = Schema.String.check(Schema.isUUID()).pipe(Schema.brand('SessionId'))
export type SessionId = typeof SessionId.Type

const watchesDirectory = Config.NonEmptyString('XDG_STATE_HOME').pipe(
  Config.withDefault(join(homedir(), '.local', 'state')),
  Config.map((state) => join(state, 'beady-eye', 'watches')),
)

const KeptWatches = Schema.fromJsonString(
  Schema.Array(Schema.Struct({ ...Bead.fields, told: Schema.optional(Told) })),
)

/** The bead a bead or gone line names. */
const beadNamedIn = (said: Said): Bead | undefined => {
  if (said.line === 'bead') return { project: said.project, id: said.row.id }
  if (said.line === 'gone') return { project: said.project, id: said.id }
  return undefined
}

const isAbout = (said: Said, bead: Bead) => {
  const named = beadNamedIn(said)
  return named?.project === bead.project && named.id === bead.id
}

/** What a batch says of the bead, where it says anything. */
const standingIn = (batch: readonly Said[], bead: Bead): Standing | undefined => {
  const [first] = batch
  if (first?.line === 'refused') return { is: 'refused', reason: first.reason }
  const said = Arr.findLast(batch, (line) => isAbout(line, bead))
  if (Option.isNone(said)) return undefined
  if (said.value.line === 'gone') return { is: 'gone' }
  if (said.value.line !== 'bead') return undefined
  const { row, ready } = said.value
  const { title, status, comment_count, close_reason } = row
  return {
    is: 'bead',
    title,
    status,
    ready,
    comments: comment_count,
    closeReason: close_reason,
  }
}

const same = (one: Told | undefined, other: Told | undefined) =>
  JSON.stringify(one) === JSON.stringify(other)

const toldOf = (standing: Standing): Told | undefined =>
  standing.is === 'bead' || standing.is === 'gone' ? standing : undefined

/** Each comment on `bead` whose event line is in the batch. */
const commentsIn = (batch: readonly Said[], bead: Bead) =>
  batch.flatMap((said) =>
    said.line === 'event' &&
    said.project === bead.project &&
    said.event.issue_id === bead.id &&
    said.event.comment !== undefined
      ? [said.event.comment]
      : [],
  )

/** Text a tracker's writer chose, on one line in quotes, so it cannot pass
 * for a line of the message. */
const quoted = (text: string) => JSON.stringify(text)

const plural = (count: number, one: string, many: string) => (count === 1 ? one : many)

/** What the session is told of the change from `before` to `now`, line by
 * line: a status, readiness, comments, and the bead going or coming back. */
const changesFrom = (before: Told, now: Told, batch: readonly Said[], bead: Bead): string[] => {
  if (before.is === 'gone' || now.is === 'gone') {
    if (before.is === now.is) return []
    return now.is === 'gone'
      ? ['It has gone from its tracker.']
      : [`It is back in its tracker, ${now.status}.`]
  }
  const changes: string[] = []
  if (now.status !== before.status) {
    const reason =
      now.status === 'closed' && now.closeReason !== undefined
        ? `, with the reason ${quoted(now.closeReason)}`
        : ''
    changes.push(`Its status went from ${before.status} to ${now.status}${reason}.`)
  }
  if (now.ready !== before.ready) {
    changes.push(now.ready ? 'It became ready.' : 'It is no longer ready.')
  }
  const comments = commentsIn(batch, bead)
  for (const { author, text } of comments) {
    changes.push(`${quoted(author)} commented: ${quoted(text)}`)
  }
  const uncarried =
    now.comments === undefined || before.comments === undefined
      ? 0
      : now.comments - before.comments - comments.length
  if (uncarried > 0) {
    const more = comments.length > 0 ? 'more ' : ''
    changes.push(
      `${uncarried} ${more}${plural(uncarried, 'comment', 'comments')} arrived. Read ${plural(uncarried, 'it', 'them')} with bd.`,
    )
  }
  return changes
}

/** The message for the session where `bead` went from `before` to `now`
 * with something a waiting session acts on. */
const newsOf = (
  bead: Bead,
  before: Told,
  now: Told,
  batch: readonly Said[],
): Omit<News, 'told'> | undefined => {
  const changes = changesFrom(before, now, batch, bead)
  if (changes.length === 0) return undefined
  const title = now.is === 'bead' ? now.title : before.is === 'bead' ? before.title : ''
  return {
    content: [
      `${named(bead)}, ${quoted(title)}, has changed.`,
      ...changes.map((change) => `- ${change}`),
    ].join('\n'),
    block: {
      is: 'bead',
      project: bead.project,
      id: bead.id,
      status: now.is === 'bead' ? now.status : 'gone',
      ready: now.is === 'bead' && now.ready,
    },
  }
}

const named = ({ project, id }: Bead) => `${id} in ${project}`

const whyDown = (why: Down) =>
  typeof why === 'string'
    ? whyTheWatcherIsDown[why]
    : `the bdi config at ${why.config} cannot be read: ${why.reason}`

const whyTheWatcherIsDown: Record<Exclude<Down, ConfigUnreadable>, string> = {
  nowhere:
    'the bdi config names no watcher socket, and there is no runtime directory to look for one in',
  refused: 'nothing at its socket took the connection',
  untrusted:
    "the file at its socket is not believed, because it is a link, is not a socket, is not the user's own, or is under a directory somebody else may write to",
  closed: 'it closed the connection',
  wedged: 'it stopped sending lines',
  protocol:
    'it speaks a protocol this plugin does not know, so the plugin and the watcher need releases that speak the same one',
}

const listed = (beads: readonly Bead[]) => beads.map((bead) => `- ${named(bead)}`)

const watcherDown = (why: Down, beads: readonly Bead[]): Omit<News, 'told'> => ({
  content: [
    `The watcher is down: ${whyDown(why)}.`,
    'These beads are not watched until it is back:',
    ...listed(beads),
    'Their watches are kept, and this session will be told when the watcher is back.',
  ].join('\n'),
  block: { is: 'watcher', watcher: 'down' },
})

const watcherBack = (changed: readonly Omit<News, 'told'>[]): Omit<News, 'told'> => ({
  content: [
    'The watcher is answering again.',
    ...changed.map(({ content }) => content),
    'A comment made while it was down arrives as a count, without its text.',
  ].join('\n'),
  block: { is: 'watcher', watcher: 'answering' },
})

const trackerUnreachable = (
  project: ProjectName,
  why: string,
  beads: readonly Bead[],
): Omit<News, 'told'> => ({
  content: [
    `The watcher cannot reach the tracker of ${project}: ${why}.`,
    'These beads keep their last known status until it can:',
    ...listed(beads),
    'This session will be told when it can.',
  ].join('\n'),
  block: { is: 'tracker', project, tracker: 'unreachable' },
})

const trackerBack = (project: ProjectName): Omit<News, 'told'> => ({
  content: `The watcher reaches the tracker of ${project} again.`,
  block: { is: 'tracker', project, tracker: 'ok' },
})

const keptUnreadable = (
  why: string,
  aside: string | { readonly unmoved: string },
): Omit<News, 'told'> => ({
  content: [
    `The watches this session kept could not be read: ${why}.`,
    typeof aside === 'string'
      ? `The file was moved to ${aside}, and this session watches none of the beads it named. Watch again each bead the session still waits on.`
      : `It could not be moved aside: ${aside.unmoved}. This session watches none of the beads it named, and its next watch or unwatch writes over the file.`,
  ].join('\n'),
  block: { is: 'watches', watches: 'unreadable' },
})

const watchesUnkept = (why: string, beads: readonly Bead[]): Omit<News, 'told'> => ({
  content: [
    `The watches this session made before its id was known could not be kept: ${why}.`,
    'These beads are watched until the plugin stops, and the next watch or unwatch that is kept keeps them:',
    ...listed(beads),
  ].join('\n'),
  block: { is: 'watches', watches: 'unkept' },
})

const told = (text: string): Answer => ({ text, refused: false })
const refusal = (text: string): Answer => ({ text, refused: true })

/** Why the watcher will not watch `bead`, where it will not. */
const refusalOf = ({ bead, known: { standing } }: Watch): Answer | undefined =>
  Match.value(standing).pipe(
    Match.discriminatorsExhaustive('is')({
      bead: () => undefined,
      gone: () => refusal(`${bead.project} holds no bead ${bead.id}.`),
      refused: ({ reason }) =>
        reason === 'unknown-project'
          ? refusal(`The watcher does not read a project ${bead.project}.`)
          : refusal(`The watcher refused to watch ${named(bead)}: ${reason}.`),
      unheard: () => undefined,
    }),
  )

const describe = ({ known: { standing } }: Watch): string =>
  Match.value(standing).pipe(
    Match.discriminatorsExhaustive('is')({
      bead: ({ title, status, ready }) => `"${title}", ${status}, ${ready ? 'ready' : 'not ready'}`,
      gone: () => 'gone from its tracker',
      refused: ({ reason }) => `refused by the watcher: ${reason}`,
      unheard: () => 'not heard of yet',
    }),
  )

export interface Watches {
  /** Take the session's id, and watch again what it watched, where this is
   * the first id the server has been given. A server serves one session, so
   * a later id changes nothing. */
  learn(session: SessionId | undefined): Effect.Effect<void>
  watch(id: BeadId, project: ProjectName | undefined): Effect.Effect<Answer>
  unwatch(id: BeadId, project: ProjectName | undefined): Effect.Effect<Answer>
  readonly watching: Effect.Effect<Answer>
  /** Each message for the session, until the watches are let go. */
  readonly news: Stream.Stream<News>
}

/** A session's watches, each held until `unwatch` or until the scope closes,
 * which keeps what is on disk. A bead named without a project is looked for
 * in each of the watched projects. */
export const makeWatches: Effect.Effect<
  Watches,
  never,
  WatcherLocation | WatchedProjects | FileSystem.FileSystem | Scope.Scope
> = Effect.gen(function* () {
  const timing = yield* WatcherTiming
  const projects = yield* WatchedProjects
  const scope = yield* Effect.scope
  const fs = yield* FileSystem.FileSystem
  const directory = yield* Effect.orDie(watchesDirectory)
  const session = yield* Deferred.make<SessionId>()
  /** The session's file, once its watches are back. */
  const file = yield* Deferred.make<string>()
  const keeping = yield* Semaphore.make(1)
  const watches = new Map<string, Watch>()
  const news = yield* Effect.acquireRelease(Queue.unbounded<News>(), Queue.shutdown)
  const withTheWatcher = yield* FiberSet.makeRuntime<WatcherLocation | FileSystem.FileSystem>()

  const keyOf = ({ project, id }: Bead) => `${project} ${id}`

  const keepTold = (known: Known, told: Told | undefined) =>
    Effect.suspend(() => {
      known.told = told
      return Effect.ignoreCause(persist)
    })

  /** Take what a batch says of `bead`, and the message for the session
   * where it watches the bead and something it acts on has changed since it
   * was last told. A journal that answers again late sends a comment's
   * event line in a batch after its bead line. */
  const takeIn = (
    batch: readonly Said[],
    bead: Bead,
    known: Known,
  ): Effect.Effect<News | undefined> =>
    Effect.gen(function* () {
      const before = known.heard
      known.standing = standingIn(batch, bead) ?? known.standing
      const after = toldOf(known.standing) ?? before
      known.heard = after
      const keep = keepTold(known, after)
      const watch = watches.get(keyOf(bead))
      if (watch === undefined || !isAccepted(watch)) {
        known.told = after
        return
      }
      const said =
        before === undefined || after === undefined ? undefined : newsOf(bead, before, after, batch)
      if (said !== undefined) return { ...said, told: keep }
      if (same(known.told, before) && !same(before, after)) yield* keep
      return undefined
    })

  const tell = (said: Omit<News, 'told'>) => Queue.offer(news, { ...said, told: Effect.void })

  /** Run `check` once the watcher or a tracker has had a minute to come
   * back. */
  const inAMinute = (check: Effect.Effect<void>) =>
    check.pipe(Effect.delay(timing.quietFor), Effect.forkIn(scope))

  const outages = yield* Ref.make(0)

  /** Tell the session of every watched bead the watcher cannot watch and
   * the session has not been told of, in one message. */
  const tellDown = Effect.suspend(() => {
    const untold = [...watches.values()].flatMap((watch) => {
      const { bead, known } = watch
      const { connection } = known
      return isAccepted(watch) && connection.is === 'down' && !connection.toldDown
        ? [{ bead, known, connection }]
        : []
    })
    const why =
      untold.find(({ connection }) => connection.why === 'protocol')?.connection.why ??
      untold[0]?.connection.why
    if (why === undefined) return Effect.void
    for (const { known, connection } of untold) known.connection = { ...connection, toldDown: true }
    return tell(
      watcherDown(
        why,
        untold.map(({ bead }) => bead),
      ),
    )
  })

  /** Once every bead the session was told is down is back, tell the session
   * so in one message, with what changed in each since it was last told. */
  const tellBackOnceAllAre = Effect.suspend(() => {
    const all = [...watches.values()]
    const back = all.filter(({ known }) => toldDownOf(known.connection))
    if (back.length === 0 || back.some(({ known }) => known.connection.is === 'down')) {
      return Effect.void
    }
    const changed = back.flatMap(({ bead, known }) => {
      known.connection = { is: 'answering', toldDown: false }
      const { told, heard } = known
      const said =
        told === undefined || heard === undefined ? undefined : newsOf(bead, told, heard, [])
      return said === undefined ? [] : [said]
    })
    const heard = back.map(({ known }) => [known, known.heard] as const)
    return Queue.offer(news, {
      ...watcherBack(changed),
      told: Effect.forEach(heard, ([known, now]) => keepTold(known, now), { discard: true }),
    })
  })

  /** Each project whose tracker the watcher cannot reach, and whether the
   * session has been told. */
  const unreachable = new Map<string, { told: boolean }>()

  const trackerIn = (batch: readonly Said[], project: ProjectName) =>
    Effect.gen(function* () {
      const freshness = batch.at(-1)
      if (freshness?.line !== 'freshness') return
      const why = freshness.tracker === 'ok' ? undefined : freshness.tracker.unreachable.reason
      const known = unreachable.get(project)
      if (why === undefined) {
        unreachable.delete(project)
        if (known?.told) yield* tell(trackerBack(project))
        return
      }
      if (known !== undefined) return
      const outage = { told: false }
      unreachable.set(project, outage)
      yield* inAMinute(
        Effect.suspend(() => {
          const beads = [...watches.values()]
            .filter((watch) => isAccepted(watch) && watch.bead.project === project)
            .map(({ bead }) => bead)
          if (unreachable.get(project) !== outage || beads.length === 0) return Effect.void
          outage.told = true
          return tell(trackerUnreachable(project, why, beads))
        }),
      )
    })

  /** Watch `bead`. It is not an Effect, so a caller that finds the bead
   * unwatched starts it before any other call can look. */
  const start = (bead: Bead, acceptance: Acceptance, told?: Told) => {
    const known: Known = {
      standing: told ?? { is: 'unheard' },
      heard: told,
      told,
      connection: { is: 'connecting' },
    }
    const firstHeard = Deferred.makeUnsafe<void>()
    const stillDownIn = (outage: number) => {
      const { connection } = known
      const held = watches.get(keyOf(bead))?.known === known
      return held && connection.is === 'down' && connection.outage === outage
    }
    const goDown = (why: Down) =>
      Effect.gen(function* () {
        const was = known.connection
        if (was.is === 'down') {
          known.connection = { ...was, why }
        } else {
          const outage = yield* Ref.updateAndGet(outages, Num.increment)
          known.connection = { is: 'down', why, outage, toldDown: toldDownOf(was) }
          yield* inAMinute(Effect.suspend(() => (stillDownIn(outage) ? tellDown : Effect.void)))
        }
        if (why === 'protocol') yield* tellDown
      })
    const hearBatch = (batch: readonly Said[]) =>
      Effect.gen(function* () {
        known.connection = { is: 'answering', toldDown: toldDownOf(known.connection) }
        yield* trackerIn(batch, bead.project)
        const said = yield* takeIn(batch, bead, known)
        if (!toldDownOf(known.connection)) {
          if (said !== undefined) yield* Queue.offer(news, said)
          return
        }
        yield* tellBackOnceAllAre
      })
    const hear = (heard: Heard) =>
      Result.match(heard, { onSuccess: hearBatch, onFailure: goDown }).pipe(
        Effect.andThen(Deferred.succeed(firstHeard, undefined)),
      )
    const connection = withTheWatcher(watchBead(bead).pipe(Stream.runForEach(hear)))
    const watch: Watch = {
      bead,
      known,
      acceptance,
      firstHeard,
      connection,
    }
    watches.set(keyOf(bead), watch)
    return watch
  }

  const forget = (watch: Watch) => {
    if (watches.get(keyOf(watch.bead)) === watch) watches.delete(keyOf(watch.bead))
  }

  const stop = (watch: Watch) =>
    Effect.sync(() => {
      forget(watch)
      const { project } = watch.bead
      if (![...watches.values()].some(({ bead }) => bead.project === project)) {
        unreachable.delete(project)
      }
    }).pipe(
      Effect.andThen(Fiber.interrupt(watch.connection)),
      Effect.andThen(Deferred.succeed(watch.firstHeard, undefined)),
      Effect.andThen(tellBackOnceAllAre),
    )

  const acceptedWatches = () => [...watches.values()].filter(isAccepted)

  /** Make the session's file keep `kept`. A caller holds a `keeping`
   * permit. */
  const write = (kept: readonly Watch[]) =>
    Deferred.poll(file).pipe(
      Effect.flatMap(
        Option.match({
          onNone: () => Effect.void,
          onSome: (known) =>
            Effect.flatMap(known, (path) => {
              const beads = kept.map(({ bead: { project, id }, known }) => ({
                project,
                id,
                told: known.told,
              }))
              const written = `${path}.new`
              return fs.makeDirectory(directory, { recursive: true }).pipe(
                Effect.andThen(Schema.encodeEffect(KeptWatches)(beads)),
                Effect.flatMap((encoded) => fs.writeFileString(written, encoded)),
                Effect.andThen(fs.rename(written, path)),
              )
            }),
        }),
      ),
    )

  const persist = keeping.withPermit(Effect.suspend(() => write(acceptedWatches())))

  /** Accept `watch` once the session's file keeps it, or say why it is not
   * watched. Calls watching the same bead together share one answer, so
   * nothing between reading `acceptance` and installing the answer yields. */
  const accept = (watch: Watch) =>
    Effect.gen(function* () {
      if (watches.get(keyOf(watch.bead)) !== watch) {
        return refusal(`${named(watch.bead)} was unwatched before the watcher answered.`)
      }
      const { acceptance } = watch
      if (acceptance.is === 'accepted') return undefined
      if (acceptance.is === 'accepting') return yield* Deferred.await(acceptance.answer)
      const answer = Deferred.makeUnsafe<Answer | undefined>()
      watch.acceptance = { is: 'accepting', answer }
      const refused = yield* keeping.withPermit(
        Effect.gen(function* () {
          if (watches.get(keyOf(watch.bead)) !== watch) {
            return refusal(`${named(watch.bead)} was unwatched before the watcher answered.`)
          }
          const kept = yield* Effect.result(
            write([...watches.values()].filter((kept) => isAccepted(kept) || kept === watch)),
          )
          if (Result.isSuccess(kept)) {
            watch.acceptance = { is: 'accepted' }
            return undefined
          }
          forget(watch)
          return refusal(
            `${named(watch.bead)} is not watched, because the watch could not be kept: ${kept.failure.message}.`,
          )
        }),
      )
      if (refused !== undefined) yield* stop(watch)
      yield* Deferred.succeed(answer, refused)
      return refused
    })

  /** Move a kept file that could not be read out of the way of the next
   * write, and tell the session its watches are gone. */
  const setAside = (path: string, why: string) =>
    Effect.gen(function* () {
      const aside = `${path}.unreadable-${yield* Clock.currentTimeMillis}`
      const moved = yield* fs.rename(path, aside).pipe(
        Effect.match({
          onSuccess: () => aside,
          onFailure: (unmoved) => ({ unmoved: unmoved.message }),
        }),
      )
      yield* tell(keptUnreadable(why, moved))
    })

  const restore = (id: SessionId) =>
    Effect.gen(function* () {
      const path = join(directory, `${id}.json`)
      const kept = yield* fs.readFileString(path).pipe(
        Effect.flatMap(Schema.decodeEffect(KeptWatches)),
        Effect.catchReason('PlatformError', 'NotFound', () => Effect.succeed([])),
        Effect.catch((unread) => Effect.as(setAside(path, unread.message), [])),
      )
      for (const { told: lastTold, ...bead } of kept) {
        if (!watches.has(keyOf(bead))) start(bead, { is: 'accepted' }, lastTold)
      }
      yield* Deferred.succeed(file, path)
      const unkept = Arr.differenceWith((one: Bead, other: Bead) => keyOf(one) === keyOf(other))(
        acceptedWatches().map(({ bead }) => bead),
        kept,
      )
      if (unkept.length === 0) return
      yield* persist.pipe(
        Effect.catch((unkeptBecause) => tell(watchesUnkept(unkeptBecause.message, unkept))),
      )
    })

  /** The project of the one bead the watcher has with `id`. */
  const projectHolding = (id: BeadId) =>
    Effect.gen(function* () {
      const asked = yield* Fiber.join(
        withTheWatcher(Effect.flatMap(projects.list, (projects) => askAbout(id, projects))),
      ).pipe(Effect.result)
      if (Result.isFailure(asked)) {
        const { failure } = asked
        const why = failure._tag === 'WatcherDown' ? failure.why : failure
        return refusal(
          `The watcher is down, so the project holding ${id} cannot be found: ${whyDown(why)}. Name its project to watch it from when the watcher answers.`,
        )
      }
      const answers = [...asked.success]
      const answered = (...tags: ProjectAnswer['_tag'][]) =>
        answers.filter(([, answer]) => tags.includes(answer._tag)).map(([project]) => project)
      const holders = answered('Holds')
      const unread = answered('NotReadYet', 'Unanswered')
      const refused = answers.flatMap(([project, answer]) =>
        answer._tag === 'Refused' ? [`${project} (${answer.reason})`] : [],
      )
      if (holders.length <= 1 && refused.length > 0) {
        return refusal(
          `The watcher refused to look in ${refused.join(', ')}, so it cannot say which project holds ${id}. Name its project.`,
        )
      }
      if (holders.length <= 1 && unread.length > 0) {
        const them = unread.join(', ')
        return refusal(
          `The watcher has not read ${them} yet, so it cannot say which project holds ${id}. Name its project.`,
        )
      }
      if (holders.length === 1 && holders[0] !== undefined) {
        return holders[0]
      }
      if (holders.length === 0) {
        return refusal(`No project the watcher reads holds ${id}. Name its project.`)
      }
      return refusal(
        `${id} is held by more than one project: ${holders.join(', ')}. Name its project.`,
      )
    })

  return {
    news: Stream.fromQueue(news),

    learn: (given) =>
      Effect.gen(function* () {
        if (given !== undefined && (yield* Deferred.succeed(session, given))) {
          return yield* restore(given)
        }
        if (yield* Deferred.isDone(session)) {
          yield* Deferred.await(file)
          // Completing `file` resumes this call at once, inside the call
          // that restored the watches. Yielding lets that call, which
          // arrived first, run until it next waits.
          yield* Effect.yieldNow
        }
      }),

    watch: (id, project) =>
      Effect.gen(function* () {
        const found = project ?? (yield* projectHolding(id))
        if (typeof found !== 'string') return found
        const bead = { project: found, id }
        const watch = watches.get(keyOf(bead)) ?? start(bead, { is: 'unaccepted' })
        yield* Deferred.await(watch.firstHeard)
        const refused = watch.acceptance.is === 'unaccepted' ? refusalOf(watch) : undefined
        if (refused !== undefined) {
          if (watches.get(keyOf(bead)) === watch) yield* stop(watch)
          yield* Effect.ignore(persist)
          return refused
        }
        const unkept = yield* accept(watch)
        if (unkept !== undefined) return unkept
        const { connection, standing } = watch.known
        const down = downBecause(connection)
        if (down === undefined) {
          return told(`Watching ${named(bead)}: ${describe(watch)}.`)
        }
        const isDown = `The watcher is down: ${whyDown(down)}.`
        if (standing.is === 'unheard') {
          return told(`Watching ${named(bead)} from when the watcher answers. ${isDown}`)
        }
        return told(`Watching ${named(bead)}: ${describe(watch)}, as last known. ${isDown}`)
      }),

    unwatch: (id, project) =>
      Effect.gen(function* () {
        const matching = [...watches.values()].filter(
          ({ bead }) => bead.id === id && (project === undefined || bead.project === project),
        )
        const [only] = matching
        if (only === undefined) {
          return refusal(
            `This session does not watch ${project === undefined ? id : named({ project, id })}.`,
          )
        }
        if (matching.length > 1) {
          return refusal(
            `This session watches ${id} in more than one project: ${matching.map(({ bead }) => bead.project).join(', ')}. Name its project.`,
          )
        }
        const unkept = yield* keeping.withPermit(
          Effect.gen(function* () {
            const kept = yield* Effect.result(
              write(acceptedWatches().filter((watch) => watch !== only)),
            )
            if (Result.isSuccess(kept)) return forget(only)
            return refusal(
              `${named(only.bead)} is still watched, because stopping the watch could not be kept: ${kept.failure.message}.`,
            )
          }),
        )
        if (unkept !== undefined) return unkept
        yield* stop(only)
        return told(`Stopped watching ${named(only.bead)}.`)
      }),

    watching: Effect.sync(() => {
      const all = [...watches.values()]
      if (all.length === 0) return told('This session watches no beads.')
      const down = all
        .map(({ known }) => downBecause(known.connection))
        .find(Predicate.isNotUndefined)
      const answered = all.some(({ known }) => known.connection.is === 'answering')
      return told(
        [
          down !== undefined
            ? `The watcher is down: ${whyDown(down)}. Each bead's status is the last one known.`
            : answered
              ? 'The watcher is answering.'
              : "The watcher has not answered yet. Each bead's status is the last one known.",
          ...all.map((watch) => `- ${named(watch.bead)}: ${describe(watch)}`),
        ].join('\n'),
      )
    }),
  }
})
