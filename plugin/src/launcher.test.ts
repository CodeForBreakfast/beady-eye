import { expect, test } from 'bun:test'
import pluginManifest from '../.claude-plugin/plugin.json'
import mcpConfig from '../.mcp.json'

/**
 * Claude Code writes every `${user_config.KEY}` into the server's environment,
 * as an empty string where the user set nothing. npm skips an empty
 * `npm_config_*` variable, so an unset setting leaves the `min-release-age`
 * an `.npmrc` sets in force, and only a value the user chose replaces it.
 */
test("a machine sets the npm release age the plugin's server starts under, and an unset one changes nothing", () => {
  expect(mcpConfig.mcpServers['beady-eye'].env.npm_config_min_release_age).toBe(
    '${user_config.NPM_MIN_RELEASE_AGE}',
  )
  const setting = pluginManifest.userConfig.NPM_MIN_RELEASE_AGE
  expect(setting.required).toBe(false)
  expect(setting).not.toHaveProperty('default')
})
