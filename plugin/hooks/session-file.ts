// Where the plugin's hooks leave the session's id for its server. The hooks
// run under node with no dependencies, so this imports node's own modules
// alone.

import { tmpdir } from 'node:os'
import { join } from 'node:path'

/** The directory holding one file for each Claude Code process, named for
 * its process id and holding the id of the session it runs now. */
export const sessionsDirectory = (env: NodeJS.ProcessEnv): string =>
  join(env['XDG_RUNTIME_DIR'] || tmpdir(), 'beady-eye', 'sessions')

export const sessionFile = (directory: string, claudeCode: number): string =>
  join(directory, String(claudeCode))
