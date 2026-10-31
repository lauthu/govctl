/**
 * Bundled skill registration.
 *
 * The skill bodies under `assets/skills/` are normalized copies of the govctl
 * repo's `.claude/skills` and `.claude/agents` (see scripts/sync-assets.mjs).
 * DSH requires only `name` + `description` in frontmatter; Claude-only keys are
 * stripped by the sync script.
 */
import { existsSync, readFileSync, readdirSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const ASSETS_SKILLS = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'assets', 'skills')

/** Parse a normalized SKILL.md into a DSH runtime skill registration. */
export function parseSkill(raw, fallbackName) {
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(raw)
  const frontmatter = match === null ? '' : match[1]
  const content = (match === null ? raw : raw.slice(match[0].length)).trim()
  const field = (key) => {
    const found = new RegExp(`^${key}:\\s*(.+)$`, 'm').exec(frontmatter)
    if (found === null) return undefined
    return found[1].trim().replace(/^"(.*)"$/, '$1').replace(/^'(.*)'$/, '$1')
  }
  const name = field('name') ?? fallbackName
  const description = field('description') ?? `govctl skill: ${fallbackName}`
  return { name, description, content }
}

/** Register every bundled skill; returns the number registered. */
export function registerSkills(ctx, log) {
  let entries
  try {
    entries = readdirSync(ASSETS_SKILLS, { withFileTypes: true })
  } catch (error) {
    log(`bundled skills unavailable: ${error.message ?? error}`)
    return 0
  }
  let registered = 0
  for (const entry of entries) {
    if (!entry.isDirectory()) continue
    const file = path.join(ASSETS_SKILLS, entry.name, 'SKILL.md')
    if (!existsSync(file)) continue
    try {
      const skill = parseSkill(readFileSync(file, 'utf8'), entry.name)
      ctx.skills.register(skill)
      registered += 1
    } catch (error) {
      log(`skipped skill ${entry.name}: ${error.message ?? error}`)
    }
  }
  log(`registered ${registered} bundled skills`)
  return registered
}
