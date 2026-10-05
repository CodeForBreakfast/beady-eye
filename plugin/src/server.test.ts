import { afterEach, beforeAll, expect, test } from 'bun:test'
import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import type { JSONRPCNotification } from '@modelcontextprotocol/sdk/types.js'
import { Effect } from 'effect'
import { buildServer, tellSession } from './server'
import {
  aPrivateDirectory,
  aWatcherAt,
  cleanUp,
  cleanUpAfterEach,
  said,
  someWatches,
} from './test-watcher'

const pluginRoot = new URL('..', import.meta.url).pathname
const bundle = `${pluginRoot}dist/server.js`
const session = '6f1c2d3e-4a5b-4c6d-8e7f-901234567890'

afterEach(cleanUpAfterEach)

beforeAll(() => {
  const build = Bun.spawnSync(['bun', 'run', 'build'], { cwd: pluginRoot })
  expect(build.exitCode).toBe(0)
})

/** The environment Claude Code starts the bundle with, with a home of the
 * test's own and whatever else `environment` adds. */
const startedIn = (home: string, environment: Record<string, string> = {}) => ({
  PATH: process.env['PATH'] ?? '',
  HOME: home,
  XDG_STATE_HOME: join(home, 'state'),
  XDG_RUNTIME_DIR: join(home, 'run'),
  ...environment,
})

/** The bundle, started as Claude Code starts it, handing each notification
 * it sends to `notified`. */
const theBundle = async (
  home: string,
  environment: Record<string, string> = {},
  notified: (notification: JSONRPCNotification) => void = () => {},
) => {
  const client = new Client({ name: 'session', version: '0' })
  client.fallbackNotificationHandler = async (notification) =>
    notified(notification as JSONRPCNotification)
  await client.connect(
    new StdioClientTransport({
      command: 'node',
      args: [bundle],
      env: startedIn(home, environment),
    }),
  )
  cleanUp.push(() => client.close())
  return client
}

test('the server sends a channel message to the session', async () => {
  const { watches } = await someWatches(aPrivateDirectory(), undefined, [])
  const server = buildServer(watches)
  const client = new Client({ name: 'session', version: '0' })
  const received = new Promise<JSONRPCNotification>((resolve) => {
    client.fallbackNotificationHandler = async (notification) =>
      resolve(notification as JSONRPCNotification)
  })
  const [clientSide, serverSide] = InMemoryTransport.createLinkedPair()
  await server.connect(serverSide)
  await client.connect(clientSide)

  await Effect.runPromise(tellSession(server, 'bdi-7 closed', { id: 'bdi-7' }))

  expect(await received).toEqual({
    jsonrpc: '2.0',
    method: 'notifications/claude/channel',
    params: { content: 'bdi-7 closed', meta: { id: 'bdi-7' } },
  })
  await client.close()
})

test('the bundle runs under node and declares the channel', async () => {
  const client = await theBundle(aPrivateDirectory())

  expect(client.getServerCapabilities()?.experimental).toEqual({
    'claude/channel': {},
  })
})

test('the bundle tells a session what its tools are for, how a change reads and when to unwatch', async () => {
  const client = await theBundle(aPrivateDirectory())
  const instructions = client.getInstructions() ?? ''

  for (const tool of ['`watch`', '`unwatch`', '`watching`']) {
    expect(instructions).toContain(tool)
  }
  expect(instructions).toContain('<channel source="beady-eye"')
  expect(instructions).toMatch(/lasts until `unwatch`/)
  expect(instructions).toMatch(/restart/)
})

test('a bundle started again under the session watches its beads before any tool is called', async () => {
  const home = aPrivateDirectory()
  mkdirSync(join(home, 'run', 'beady-eye'), { recursive: true })
  const watcher = await aWatcherAt(join(home, 'run', 'beady-eye', 'watcher.sock'))
  const first = await theBundle(home)
  const watching = first.callTool({
    name: 'watch',
    arguments: {
      id: 'smt-4kd3p.20',
      project: 'summit-works',
      session_id: session,
    },
  })
  ;(await watcher.next()).connection.end()
  await watching
  await first.close()

  await theBundle(home, { CLAUDE_CODE_SESSION_ID: session })

  expect((await watcher.next()).asked).toBe('watch summit-works smt-4kd3p.20\n')
})

test('a bundle started again under the session tells it of a bead that changed while it was away', async () => {
  const home = aPrivateDirectory()
  mkdirSync(join(home, 'run', 'beady-eye'), { recursive: true })
  const watcher = await aWatcherAt(join(home, 'run', 'beady-eye', 'watcher.sock'))
  const bead = (status: string) => ({
    line: 'bead',
    project: 'summit-works',
    ready: false,
    row: { id: 'smt-4kd3p.20', title: 'guard the gate', status },
  })
  const freshness = {
    line: 'freshness',
    project: 'summit-works',
    as_of: '2026-08-30T10:22:14Z',
    tracker: 'ok',
    events: 'off',
    protocol: 1,
  }
  const first = await theBundle(home)
  const watching = first.callTool({
    name: 'watch',
    arguments: {
      id: 'smt-4kd3p.20',
      project: 'summit-works',
      session_id: session,
    },
  })
  ;(await watcher.next()).connection.write(said(bead('blocked'), freshness))
  await watching
  await first.close()

  const told = new Promise<JSONRPCNotification>((resolve) =>
    theBundle(home, { CLAUDE_CODE_SESSION_ID: session }, resolve),
  )
  ;(await watcher.next()).connection.write(said(bead('closed'), freshness))

  expect((await told).params).toEqual({
    content:
      'smt-4kd3p.20 in summit-works, "guard the gate", has changed.\n- Its status went from blocked to closed.',
    meta: {
      project: 'summit-works',
      id: 'smt-4kd3p.20',
      status: 'closed',
      ready: 'false',
    },
  })
})

/** Start the bundle, close its input as a session does when it goes, and
 * say how the bundle exited. */
const exitOnClosedInput = async (home: string) => {
  const started = Bun.spawn(['node', bundle], {
    stdin: 'pipe',
    stdout: 'ignore',
    env: startedIn(home, { CLAUDE_CODE_SESSION_ID: session }),
  })
  cleanUp.push(() => started.kill('SIGKILL'))

  await started.stdin.end()

  return started.exited
}

test('the bundle stops when the session closes its input', async () => {
  expect(await exitOnClosedInput(aPrivateDirectory())).toBe(0)
})

test('a bundle watching a bead stops when its session closes its input', async () => {
  const home = aPrivateDirectory()
  mkdirSync(join(home, 'state', 'beady-eye', 'watches'), { recursive: true })
  writeFileSync(
    join(home, 'state', 'beady-eye', 'watches', `${session}.json`),
    JSON.stringify([{ project: 'summit-works', id: 'smt-4kd3p.20' }]),
  )

  expect(await exitOnClosedInput(home)).toBe(0)
})

test('a bundle reads the projects to look in from the bdi config', async () => {
  const home = aPrivateDirectory()
  mkdirSync(join(home, '.config', 'beady-eye'), { recursive: true })
  writeFileSync(
    join(home, '.config', 'beady-eye', 'config.toml'),
    '[watcher]\nsocket = "' +
      join(home, 'watcher.sock') +
      '"\n\n[[projects]]\nname = "harbour"\npath = "/srv/harbour"\n',
  )
  const watcher = await aWatcherAt(join(home, 'watcher.sock'))
  const client = await theBundle(home)

  const watching = client.callTool({
    name: 'watch',
    arguments: { id: 'smt-4kd3p.20', session_id: session },
  })
  const { connection, asked } = await watcher.next()
  connection.end()
  await watching

  expect(asked).toBe('watch harbour smt-4kd3p.20\n')
})
