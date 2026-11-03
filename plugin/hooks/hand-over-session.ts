#!/usr/bin/env node
// Claude Code runs this when a session starts, clears, resumes or compacts,
// and before each call to the plugin's tools, because an MCP server is not
// told the session it serves. It leaves the session's id where the server
// reads it, and changes nothing about the call, so auto mode's review of the
// call still covers what runs.

import { mkdirSync, renameSync, writeFileSync } from 'node:fs'
import { text } from 'node:stream/consumers'
import { sessionFile, sessionsDirectory } from './session-file.ts'

const sessionOf = (stdin: string): string | undefined => {
  try {
    const parsed: unknown = JSON.parse(stdin)
    if (typeof parsed !== 'object' || parsed === null || !('session_id' in parsed)) return
    const { session_id } = parsed
    return typeof session_id === 'string' && session_id.length > 0 ? session_id : undefined
  } catch {
    return
  }
}

const session = sessionOf(await text(process.stdin))
if (session !== undefined) {
  try {
    const directory = sessionsDirectory(process.env)
    mkdirSync(directory, { recursive: true, mode: 0o700 })
    // Claude Code runs the hook itself, so its parent is the process the
    // server runs under too. A new file replaces the old whole, so the
    // server never reads half an id.
    const file = sessionFile(directory, process.ppid)
    const writing = `${file}.${process.pid}`
    writeFileSync(writing, session)
    renameSync(writing, file)
  } catch {
    // A session whose id cannot be left goes ahead, and its server keeps the
    // id it had.
  }
}
