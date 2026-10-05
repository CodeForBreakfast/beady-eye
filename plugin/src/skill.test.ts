import { expect, test } from 'bun:test'

const skill = await Bun.file(new URL('../skills/using-beady-eye/SKILL.md', import.meta.url)).text()

const frontmatter = (() => {
  const block = skill.match(/^---\n([\s\S]*?)\n---/)?.[1]
  if (block === undefined) throw new Error('using-beady-eye has no frontmatter')
  return Bun.YAML.parse(block) as { readonly name?: string; readonly description?: string }
})()

test('the skill declares the name Claude Code finds it by', () => {
  expect(frontmatter.name).toBe('using-beady-eye')
})

test('the skill says when to load it', () => {
  expect(frontmatter.description?.length ?? 0).toBeGreaterThan(0)
})

test('the skill shows a change as it arrives and says when to unwatch', () => {
  expect(skill).toContain('<channel source="beady-eye"')
  expect(skill).toMatch(/unwatch/)
})
