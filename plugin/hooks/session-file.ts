// Where the plugin's hooks leave the session's id for its server. The hooks
// run under node with no dependencies, so this imports node's own modules
// alone.

import { tmpdir, userInfo } from 'node:os'
import { join } from 'node:path'

/** The directory holding one file for each Claude Code process, named for
 * its process id and holding the id of the session it runs now. The
 * temporary directory can be shared between users, so there the directory is
 * named for the user. */
export const sessionsDirectory = (env: NodeJS.ProcessEnv): string =>
  env['XDG_RUNTIME_DIR']
    ? join(env['XDG_RUNTIME_DIR'], 'beady-eye', 'sessions')
    : join(tmpdir(), `beady-eye-${userInfo().uid}`, 'sessions')

export const sessionFile = (directory: string, claudeCode: number): string =>
  join(directory, String(claudeCode))
