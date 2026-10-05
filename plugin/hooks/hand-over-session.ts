#!/usr/bin/env node
// Claude Code runs this before each call to the plugin's tools, because an MCP
// server is told neither the session it serves nor where that session runs.
// The model never sees what it adds.

import { text } from 'node:stream/consumers'

interface HookInput {
  readonly session_id?: unknown
  readonly cwd?: unknown
  readonly tool_input?: unknown
}

const nonEmpty = (value: unknown): value is string => typeof value === 'string' && value.length > 0

const handOver = (input: HookInput) => {
  if (!nonEmpty(input.session_id)) return {}
  const toolInput =
    typeof input.tool_input === 'object' && input.tool_input !== null ? input.tool_input : {}
  return {
    updatedInput: {
      ...toolInput,
      session_id: input.session_id,
      ...(nonEmpty(input.cwd) ? { cwd: input.cwd } : {}),
    },
  }
}

let input: HookInput = {}
try {
  input = JSON.parse(await text(process.stdin))
} catch {
  // A call the hook cannot read goes ahead unchanged rather than failing.
}

process.stdout.write(
  JSON.stringify({
    hookSpecificOutput: { hookEventName: 'PreToolUse', ...handOver(input) },
  }),
)
