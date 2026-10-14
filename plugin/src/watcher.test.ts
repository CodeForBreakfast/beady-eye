import { afterEach, expect, test } from 'bun:test'
import { chmodSync, lstatSync, mkdirSync, symlinkSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { NodeFileSystem } from '@effect/platform-node'
import {
  ConfigProvider,
  Duration,
  Effect,
  Fiber,
  type FileSystem,
  Option,
  Queue,
  Result,
  Stream,
} from 'effect'
import {
  anId,
  aPrivateDirectory,
  aProject,
  aWatcherAt,
  cleanUp,
  cleanUpAfterEach,
  said,
} from './test-watcher'
import {
  askAbout,
  ConfigUnreadable,
  type Heard,
  othersMayTakeANameIn,
  ProjectAnswer,
  pauseAfter,
  projectsNamedIn,
  type Timing,
  WatcherLocation,
  WatcherTiming,
  watchBead,
  whereTheWatcherIs,
} from './watcher'

const timing: Timing = {
  wedgedAfter: Duration.millis(200),
  firstPause: Duration.millis(10),
  longestPause: Duration.millis(40),
  answeredWithin: Duration.millis(100),
  quietFor: Duration.millis(300),
}

afterEach(cleanUpAfterEach)
const bead = { project: aProject('summit-works'), id: anId('smt-4kd3p.20') }
const asked = 'watch summit-works smt-4kd3p.20\n'

const beadLine = {
  line: 'bead',
  project: 'summit-works',
  ready: false,
  blocked_by: ['smt-4kd3p.13'],
  bd: { ready: false, blocked_by: ['smt-4kd3p.13'] },
  row: { id: 'smt-4kd3p.20', title: 'the daily wallpaper timer calls dms', status: 'blocked' },
}
/** The bead line as the plugin reads it. */
const beadHeard = {
  line: 'bead',
  project: aProject('summit-works'),
  ready: false,
  row: {
    id: anId('smt-4kd3p.20'),
    title: 'the daily wallpaper timer calls dms',
    status: 'blocked',
  },
} as const
const freshness = {
  line: 'freshness',
  project: 'summit-works',
  as_of: '2026-08-30T10:22:14Z',
  tracker: 'ok',
  events: 'off',
  protocol: 1,
  reach: { path: '/home/mira/summit-works', environment_command: [] },
}
/** The freshness line as the plugin reads it. */
const freshnessHeard = {
  line: 'freshness',
  project: aProject('summit-works'),
  protocol: 1,
  tracker: 'ok',
} as const
/** The bead line's answer, closed by its freshness line, as a watch hears it. */
const answerHeard = Result.succeed({
  is: 'answer',
  lines: [beadHeard],
  freshness: freshnessHeard,
} as const)

const run = <A, E>(effect: Effect.Effect<A, E, FileSystem.FileSystem>) =>
  Effect.runPromise(effect.pipe(Effect.provide(NodeFileSystem.layer)))

/** Look for the watcher at `path`, on the test's timings. */
const at =
  (path: string | undefined, timed: Timing = timing) =>
  <A, E, R>(effect: Effect.Effect<A, E, R>) =>
    effect.pipe(
      Effect.provideService(WatcherLocation, { find: Effect.succeed(Option.fromNullishOr(path)) }),
      Effect.provideService(WatcherTiming, timed),
    )

/** A watcher killed where it stood, which leaves its socket at the path with
 * nothing listening. One that closes its server removes the socket. */
const aWatcherThatDied = async (at: string) => {
  const watcher = Bun.spawn(
    [
      process.execPath,
      '-e',
      `require("node:net").createServer().listen(${JSON.stringify(at)}, () => console.log("listening"))`,
    ],
    { stdout: 'pipe' },
  )
  await watcher.stdout.getReader().read()
  watcher.kill('SIGKILL')
  await watcher.exited
}

/** A session watching the bead at `path`, and what it is told, in order. */
const watching = (path: string | undefined, watchTiming = timing) => {
  const told = Effect.runSync(Queue.unbounded<Heard>())
  const watch = Effect.runFork(
    watchBead(bead).pipe(
      Stream.runForEach((heard) => Queue.offer(told, heard)),
      at(path, watchTiming),
      Effect.provide(NodeFileSystem.layer),
    ),
  )
  const stop = () => Effect.runPromise(Fiber.interrupt(watch))
  cleanUp.push(stop)
  return {
    stop,
    next: () => Effect.runPromise(Queue.take(told)),
    heard: () => Effect.runSync(Queue.clear(told)),
  }
}

/** What a session that stops after hearing `count` things is told. */
const firstHeard = (path: string, count: number) =>
  run(watchBead(bead).pipe(Stream.take(count), Stream.runCollect, at(path)))

const pause = (millis: number) => new Promise((resolve) => setTimeout(resolve, millis))

const whereWith = (config: string, runtimeDirectory: string | undefined) =>
  run(
    whereTheWatcherIs(config).pipe(
      Effect.provideService(
        ConfigProvider.ConfigProvider,
        ConfigProvider.fromEnv({
          env: runtimeDirectory === undefined ? {} : { XDG_RUNTIME_DIR: runtimeDirectory },
        }),
      ),
      Effect.map(Option.getOrUndefined),
      Effect.catchTag('ConfigUnreadable', Effect.succeed),
    ),
  )

test("the config's watcher socket is where the watcher is", async () => {
  const config = join(aPrivateDirectory(), 'config.toml')
  writeFileSync(config, '[watcher]\nsocket = "/var/folders/T/watcher.sock"\n')

  expect(await whereWith(config, '/run/user/1000')).toBe('/var/folders/T/watcher.sock')
})

test('a config naming no socket leaves the watcher under the runtime directory', async () => {
  const directory = aPrivateDirectory()
  const config = join(directory, 'config.toml')
  writeFileSync(config, '[tui]\nrefresh_seconds = 30\n')
  const emptyWatcher = join(directory, 'empty-watcher.toml')
  writeFileSync(emptyWatcher, '[watcher]\n')

  for (const named of [config, emptyWatcher, join(directory, 'missing.toml')]) {
    expect(await whereWith(named, '/run/user/1000')).toBe('/run/user/1000/beady-eye/watcher.sock')
  }
})

test('a config that is not TOML is unreadable, and says where it went wrong', async () => {
  const config = join(aPrivateDirectory(), 'config.toml')
  writeFileSync(config, '[watcher]\nsocket = \n')

  expect(await whereWith(config, '/run/user/1000')).toEqual(
    new ConfigUnreadable({
      config,
      reason: 'Invalid TOML document: invalid value, at line 2, column 10',
    }),
  )
  expect(await run(projectsNamedIn(config).pipe(Effect.flip))).toEqual(
    new ConfigUnreadable({
      config,
      reason: 'Invalid TOML document: invalid value, at line 2, column 10',
    }),
  )
})

test('a config whose watcher section is the wrong shape is unreadable, and does not fall back to the runtime directory', async () => {
  const config = join(aPrivateDirectory(), 'config.toml')
  writeFileSync(config, '[watcher]\nsocket = 3\n')

  expect(await whereWith(config, '/run/user/1000')).toEqual(
    new ConfigUnreadable({ config, reason: 'Expected string at ["watcher"]["socket"]' }),
  )
})

test('a config whose projects are the wrong shape is unreadable', async () => {
  const config = join(aPrivateDirectory(), 'config.toml')
  writeFileSync(config, '[[projects]]\npath = "/srv/harbour"\n')

  expect(await run(projectsNamedIn(config).pipe(Effect.flip))).toEqual(
    new ConfigUnreadable({ config, reason: 'Missing key at ["projects"][0]["name"]' }),
  )
})

test('a config with no projects, or no config at all, names no project', async () => {
  const directory = aPrivateDirectory()
  const config = join(directory, 'config.toml')
  writeFileSync(config, '[watcher]\nsocket = "/var/folders/T/watcher.sock"\n')

  for (const named of [config, join(directory, 'missing.toml')]) {
    expect<readonly string[]>(await run(projectsNamedIn(named))).toEqual([])
  }
})

test('with no socket named and no runtime directory there is no watcher to reach', async () => {
  const config = join(aPrivateDirectory(), 'missing.toml')

  expect(await whereWith(config, undefined)).toBeUndefined()
  expect(await whereWith(config, '')).toBeUndefined()
})

test('a directory is one others may take a name in by its owner as well as its mode', () => {
  const me = 1000
  expect(othersMayTakeANameIn(0o700, me, me)).toBe(false)
  expect(othersMayTakeANameIn(0o755, 0, me)).toBe(false)
  expect(othersMayTakeANameIn(0o1777, 0, me)).toBe(false)
  expect(othersMayTakeANameIn(0o755, 1001, me)).toBe(true)
  expect(othersMayTakeANameIn(0o770, me, me)).toBe(true)
  expect(othersMayTakeANameIn(0o707, me, me)).toBe(true)
  expect(othersMayTakeANameIn(0o750, me, me)).toBe(false)
  expect(othersMayTakeANameIn(0o705, me, me)).toBe(false)
})

test("a bead's lines arrive as one batch at its project's freshness line", async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)
  const session = watching(path)

  const { connection, asked: line } = await watcher.next()
  connection.write(said({ line: 'alive' }, beadLine, freshness))

  expect(line).toBe(asked)
  expect(await session.next()).toEqual(answerHeard)
})

test('a line of a kind the plugin does not read is passed over', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)
  const session = watching(path)

  ;(await watcher.next()).connection.write(
    said(beadLine, { line: 'greeting', from: 'a newer watcher' }, freshness),
  )

  expect(await session.next()).toEqual(answerHeard)
})

test('a refusal of the watch arrives on its own', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)
  const session = watching(path)

  const refused = {
    line: 'refused',
    asked: asked.trim(),
    reason: 'unknown-project',
  } as const
  ;(await watcher.next()).connection.write(said(refused))

  expect(await session.next()).toEqual(Result.succeed({ is: 'refused', refused }))
})

test('a refused connection is a watcher that is down', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  await aWatcherThatDied(path)
  expect(lstatSync(path).isSocket()).toBe(true)
  const session = watching(path)

  expect(await session.next()).toEqual(Result.fail('refused'))
})

test('a watcher that is not there yet is tried again until it answers', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const session = watching(path)

  expect(await session.next()).toEqual(Result.fail('refused'))
  const watcher = await aWatcherAt(path)
  const { connection, asked: line } = await watcher.next()
  connection.write(said(beadLine, freshness))

  expect(line).toBe(asked)
  expect(await session.next()).toEqual(answerHeard)
})

test('a connection the watcher closes is down, and the bead is watched again', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)
  const session = watching(path)

  ;(await watcher.next()).connection.end()

  expect(await session.next()).toEqual(Result.fail('closed'))
  expect((await watcher.next()).asked).toBe(asked)
})

test('a connection with no line for a minute has wedged and is closed', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)
  const session = watching(path)

  const { connection } = await watcher.next()
  const closed = new Promise((resolve) => connection.on('close', resolve))

  expect(await session.next()).toEqual(Result.fail('wedged'))
  await closed
  expect((await watcher.next()).asked).toBe(asked)
})

test('alive lines keep a quiet connection up', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)
  const session = watching(path)

  const { connection } = await watcher.next()
  const alive = setInterval(
    () => connection.write(said({ line: 'alive' })),
    Duration.toMillis(timing.wedgedAfter) / 4,
  )
  await pause(Duration.toMillis(timing.wedgedAfter) * 3)
  clearInterval(alive)

  expect(session.heard()).toEqual([])
})

test('a protocol the plugin does not know is a watcher that is down', async () => {
  const { protocol: _, ...noProtocol } = freshness
  for (const answer of [
    said(beadLine, { ...freshness, protocol: 2 }),
    said(beadLine, noProtocol),
    'unknown watch summit-works smt-4kd3p.20\n',
    said('a string'),
  ]) {
    const path = join(aPrivateDirectory(), 'watcher.sock')
    const watcher = await aWatcherAt(path)
    const session = watching(path)

    ;(await watcher.next()).connection.write(answer)

    expect(await session.next()).toEqual(Result.fail('protocol'))
    await session.stop()
  }
})

test('a line naming a project or bead id that is not one word is a watcher speaking another protocol', async () => {
  for (const answer of [
    said({ ...beadLine, project: 'summit works' }, freshness),
    said({ ...beadLine, row: { ...beadLine.row, id: 'smt-4kd3p.20\nwatch harbour' } }, freshness),
    said({ line: 'gone', project: 'summit-works', id: 'smt 4kd3p.20' }, freshness),
    said(
      {
        line: 'event',
        project: 'summit-works',
        event: { op: 'status_changed', issue_id: 'smt-4kd3p.20 smt-4kd3p.13' },
      },
      freshness,
    ),
    said(beadLine, { ...freshness, project: 'summit\nworks' }),
  ]) {
    const path = join(aPrivateDirectory(), 'watcher.sock')
    const watcher = await aWatcherAt(path)
    const session = watching(path)

    ;(await watcher.next()).connection.write(answer)

    expect(await session.next()).toEqual(Result.fail('protocol'))
    await session.stop()
  }
})

test("a socket that is not the user's own is not believed", async () => {
  const open = join(aPrivateDirectory(), 'open')
  mkdirSync(open)
  chmodSync(open, 0o777)
  const path = join(open, 'watcher.sock')
  const watcher = await aWatcherAt(path)
  let connections = 0
  watcher.server.on('connection', () => {
    connections += 1
  })
  const session = watching(path)

  expect(await session.next()).toEqual(Result.fail('untrusted'))
  expect(connections).toBe(0)
})

test('something other than a socket at the path is not believed', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  writeFileSync(path, '')
  const session = watching(path)

  expect(await session.next()).toEqual(Result.fail('untrusted'))
})

test("a link to the user's own socket is not believed", async () => {
  const directory = aPrivateDirectory()
  const socket = join(directory, 'watcher.sock')
  const watcher = await aWatcherAt(socket)
  let connections = 0
  watcher.server.on('connection', () => {
    connections += 1
  })
  const path = join(directory, 'link.sock')
  symlinkSync(socket, path)
  const session = watching(path)

  expect(await session.next()).toEqual(Result.fail('untrusted'))
  expect(connections).toBe(0)
})

test('a link to nothing is not believed', async () => {
  const directory = aPrivateDirectory()
  const path = join(directory, 'link.sock')
  symlinkSync(join(directory, 'watcher.sock'), path)
  const session = watching(path)

  expect(await session.next()).toEqual(Result.fail('untrusted'))
})

test('with nowhere to find a watcher the watcher is down', async () => {
  const session = watching(undefined)

  expect(await session.next()).toEqual(Result.fail('nowhere'))
})

test('the pause before connecting again grows to a limit', () => {
  expect(
    [1, 2, 3, 4, 9].map((failures) => Duration.toMillis(pauseAfter(failures, timing))),
  ).toEqual([10, 20, 40, 40, 40])
})

test('a watcher that answered is tried again after the shortest pause', async () => {
  const slowly: Timing = {
    ...timing,
    firstPause: Duration.millis(20),
    longestPause: Duration.seconds(5),
  }
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const session = watching(path, slowly)
  for (let refused = 0; refused < 4; refused += 1) {
    expect(await session.next()).toEqual(Result.fail('refused'))
  }

  const watcher = await aWatcherAt(path)
  ;(await watcher.next()).connection.end(said(beadLine, freshness))
  expect(await session.next()).toEqual(answerHeard)
  expect(await session.next()).toEqual(Result.fail('closed'))
  const downAt = Date.now()
  await watcher.next()

  expect(Date.now() - downAt).toBeLessThan(200)
})

test('a watch that stops closes its connection and is not tried again', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)
  const session = watching(path)

  const { connection } = await watcher.next()
  const closed = new Promise((resolve) => connection.on('close', resolve))
  await session.stop()
  await closed
  await pause(Duration.toMillis(timing.longestPause) * 3)

  expect(session.heard()).toEqual([])
})

test('a watch stopped as it hears its watcher is down is not tried again', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)
  let connections = 0
  watcher.server.on('connection', () => {
    connections += 1
  })

  const told = firstHeard(path, 1)
  ;(await watcher.next()).connection.end()
  expect(await told).toEqual([Result.fail('closed')])
  await pause(Duration.toMillis(timing.longestPause) * 3)

  expect(connections).toBe(1)
})

test('a watch stopped as it hears a batch closes its connection', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)

  const told = firstHeard(path, 1)
  const { connection } = await watcher.next()
  const closed = new Promise((resolve) => connection.on('close', resolve))
  connection.write(said(beadLine, freshness, beadLine, freshness))

  expect(await told).toEqual([answerHeard])
  await closed
})

test("the config's projects are named by their names", async () => {
  const directory = aPrivateDirectory()
  const config = join(directory, 'config.toml')
  writeFileSync(
    config,
    '[[projects]]\nname = "summit-works"\npath = "/home/mira/summit-works"\n\n[[projects]]\nname = "harbour"\npath = "/srv/harbour"\n',
  )

  expect<readonly string[]>(await run(projectsNamedIn(config))).toEqual(['summit-works', 'harbour'])
  expect<readonly string[]>(await run(projectsNamedIn(join(directory, 'missing.toml')))).toEqual([])
})

test('a config project whose name is not one word is passed over', async () => {
  const config = join(aPrivateDirectory(), 'config.toml')
  writeFileSync(
    config,
    '[[projects]]\nname = "summit works"\npath = "/home/mira/summit-works"\n\n[[projects]]\nname = "harbour"\npath = "/srv/harbour"\n',
  )

  expect<readonly string[]>(await run(projectsNamedIn(config))).toEqual(['harbour'])
})

/** Each project's answer, keyed as `askAbout` keys it. */
const answers = (each: Record<string, ProjectAnswer>) =>
  new Map(Object.entries(each).map(([project, answer]) => [aProject(project), answer]))

const asking = (path: string | undefined, projects: readonly string[]) =>
  run(
    askAbout(
      anId('smt-4kd3p.20'),
      projects.map((project) => aProject(project)),
    ).pipe(
      Effect.catchTag('WatcherDown', ({ why }) => Effect.succeed(why)),
      at(path),
    ),
  )

test('asking about a bead asks each project and hangs up once each has answered', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)

  const answer = asking(path, ['summit-works', 'harbour', 'dunmore'])
  const { connection, asked: lines } = await watcher.next()
  const hungUp = new Promise((resolve) => connection.on('end', resolve))
  const gone = { line: 'gone', project: 'harbour', id: 'smt-4kd3p.20' } as const
  const harbourFreshness = { ...freshness, project: 'harbour' }
  const refused = {
    line: 'refused',
    asked: 'watch dunmore smt-4kd3p.20',
    reason: 'unknown-project',
  } as const
  connection.write(said({ line: 'alive' }, beadLine, freshness, gone, harbourFreshness, refused))

  expect(lines).toBe(
    'watch summit-works smt-4kd3p.20\nwatch harbour smt-4kd3p.20\nwatch dunmore smt-4kd3p.20\n',
  )
  expect(await answer).toEqual(
    answers({
      'summit-works': ProjectAnswer.Holds(),
      harbour: ProjectAnswer.Gone(),
      dunmore: ProjectAnswer.Refused({ reason: 'unknown-project' }),
    }),
  )
  await hungUp
})

test('asking a watcher that is down answers why', async () => {
  expect(await asking(undefined, ['summit-works'])).toBe('nowhere')

  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)
  const answer = asking(path, ['summit-works'])
  ;(await watcher.next()).connection.end(said(beadLine))
  expect(await answer).toBe('closed')
})

test('asking a watcher whose refusal does not say what it refused answers that it speaks another protocol', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)

  const answer = asking(path, ['summit-works'])
  ;(await watcher.next()).connection.write(said({ line: 'refused', reason: 'unknown-project' }))

  expect(await answer).toBe('protocol')
})

test('asking about a bead in no project asks nothing', async () => {
  expect(await asking(undefined, [])).toEqual(new Map())
})

test("asking about a bead takes each project's answer and nothing it says after", async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)

  const answer = asking(path, ['summit-works', 'harbour'])
  const gone = { line: 'gone', project: 'summit-works', id: 'smt-4kd3p.20' } as const
  const harbourFreshness = { ...freshness, project: 'harbour' }
  ;(await watcher.next()).connection.write(
    said(beadLine, freshness, gone, freshness, harbourFreshness),
  )

  expect(await answer).toEqual(
    answers({ 'summit-works': ProjectAnswer.Holds(), harbour: ProjectAnswer.NotReadYet() }),
  )
})

test('asking about a bead stops waiting in time, and names the projects that did not answer', async () => {
  const path = join(aPrivateDirectory(), 'watcher.sock')
  const watcher = await aWatcherAt(path)

  const answer = asking(path, ['summit-works', 'harbour'])
  const { connection } = await watcher.next()
  const hungUp = new Promise((resolve) => connection.on('close', resolve))
  connection.write(said(beadLine, freshness))
  const alive = setInterval(
    () => connection.write(said({ line: 'alive' })),
    Duration.toMillis(timing.answeredWithin) / 4,
  )
  cleanUp.push(() => clearInterval(alive))

  expect(await answer).toEqual(
    answers({ 'summit-works': ProjectAnswer.Holds(), harbour: ProjectAnswer.Unanswered() }),
  )
  await hungUp
})
