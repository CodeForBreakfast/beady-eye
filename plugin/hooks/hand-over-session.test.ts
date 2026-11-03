import { afterEach, expect, setDefaultTimeout, test } from 'bun:test'
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { userInfo } from 'node:os'
import { join } from 'node:path'
import { aPrivateDirectory, cleanUpAfterEach } from '../src/test-watcher'
import hooks from './hooks.json'

afterEach(cleanUpAfterEach)
// A busy machine can take seconds to start node, which the hook runs under.
setDefaultTimeout(60_000)

const hook = new URL('./hand-over-session.ts', import.meta.url).pathname

/** The hook, run as Claude Code runs it, as a child of this process. */
const runHook = async (
  runtime: string,
  stdin: string,
  env: Record<string, string> = { XDG_RUNTIME_DIR: runtime },
) => {
  const proc = Bun.spawn(['node', hook], {
    stdin: 'pipe',
    stdout: 'pipe',
    env: { PATH: process.env['PATH'] ?? '', ...env },
  })
  proc.stdin.write(stdin)
  await proc.stdin.end()
  expect(await proc.exited).toBe(0)
  return new Response(proc.stdout).text()
}

const sessions = (runtime: string) => join(runtime, 'beady-eye', 'sessions')

test('the hook leaves the session id in a file named for the process that ran it', async () => {
  const runtime = aPrivateDirectory()

  for (const event of ['SessionStart', 'PreToolUse']) {
    await runHook(
      runtime,
      JSON.stringify({ hook_event_name: event, session_id: `${event}-id`, tool_input: {} }),
    )

    expect(readFileSync(join(sessions(runtime), String(process.pid)), 'utf8')).toBe(`${event}-id`)
  }
  expect(readdirSync(sessions(runtime))).toEqual([String(process.pid)])
})

test("with no runtime directory, the hook leaves the id in the temporary directory under the user's own name", async () => {
  const temporary = aPrivateDirectory()

  await runHook(temporary, JSON.stringify({ session_id: 'a1b2c3' }), { TMPDIR: temporary })

  expect(
    readFileSync(
      join(temporary, `beady-eye-${userInfo().uid}`, 'sessions', String(process.pid)),
      'utf8',
    ),
  ).toBe('a1b2c3')
})

test("the hook changes nothing about the call it runs before, so auto mode's review covers it", async () => {
  const output = await runHook(
    aPrivateDirectory(),
    JSON.stringify({
      hook_event_name: 'PreToolUse',
      session_id: 'a1b2c3',
      cwd: '/srv/summit-works',
      tool_input: { id: 'smt-4kd3p' },
    }),
  )

  expect(output).toBe('')
})

test('a call with no session leaves nothing and goes ahead', async () => {
  const runtime = aPrivateDirectory()

  for (const stdin of [JSON.stringify({ cwd: '/srv' }), 'not json', 'null', '42']) {
    expect(await runHook(runtime, stdin)).toBe('')
  }
  expect(existsSync(sessions(runtime))).toBe(false)
})

test("the hook runs when a session starts, clears, resumes or compacts, and before each of the plugin's tools", () => {
  const command = {
    type: 'command',
    command: 'node',
    args: ['${CLAUDE_PLUGIN_ROOT}/hooks/hand-over-session.ts'],
  }

  expect(hooks.hooks).toEqual({
    SessionStart: [{ hooks: [command] }],
    PreToolUse: [{ matcher: 'mcp__plugin_beady-eye_beady-eye__.*', hooks: [command] }],
  })
})
