import { expect, test } from 'bun:test'

const hook = new URL('./hand-over-session.ts', import.meta.url).pathname

const runHook = async (stdin: string) => {
  const proc = Bun.spawn(['node', hook], { stdin: 'pipe', stdout: 'pipe' })
  proc.stdin.write(stdin)
  await proc.stdin.end()
  expect(await proc.exited).toBe(0)
  return JSON.parse(await new Response(proc.stdout).text())
}

test("the call carries the session's id and working directory", async () => {
  const output = await runHook(
    JSON.stringify({
      session_id: 'a1b2c3',
      cwd: '/srv/summit-works',
      tool_input: { id: 'smt-4kd3p' },
    }),
  )

  expect(output).toEqual({
    hookSpecificOutput: {
      hookEventName: 'PreToolUse',
      updatedInput: {
        id: 'smt-4kd3p',
        session_id: 'a1b2c3',
        cwd: '/srv/summit-works',
      },
    },
  })
})

test('a session with no working directory still hands over its id', async () => {
  const output = await runHook(JSON.stringify({ session_id: 'a1b2c3', cwd: '', tool_input: {} }))

  expect(output.hookSpecificOutput.updatedInput).toEqual({
    session_id: 'a1b2c3',
  })
})

test('a call with no session goes ahead unchanged', async () => {
  for (const stdin of [JSON.stringify({ cwd: '/srv' }), 'not json']) {
    expect(await runHook(stdin)).toEqual({
      hookSpecificOutput: { hookEventName: 'PreToolUse' },
    })
  }
})
