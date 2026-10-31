/**
 * Bundled assets: 17 normalized SKILL.md files and their manifest.
 *
 * DSH loads embedded skills through `ctx.skills.register`, so the asset files
 * are the shipped behavior: each needs `name` + `description` frontmatter and
 * no Claude-only keys or argument substitution.
 */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

import { parseSkill } from '../lib/skills.js'

const PACKAGE_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..')
const SKILLS_DIR = path.join(PACKAGE_ROOT, 'assets', 'skills')
const MANIFEST = JSON.parse(readFileSync(path.join(PACKAGE_ROOT, 'assets', 'manifest.json'), 'utf8'))

const PORTED_SKILLS = [
  'adr-writer',
  'commit',
  'decision-analysis',
  'detach',
  'discuss',
  'gov',
  'guard-writer',
  'init',
  'migrate',
  'quick',
  'rfc-writer',
  'spec',
  'wi-writer',
]
const REVIEWER_SKILLS = ['adr-reviewer', 'compliance-checker', 'rfc-reviewer', 'wi-reviewer']

test('assets contain exactly 17 skill directories', () => {
  const directories = readdirSync(SKILLS_DIR, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort()
  assert.equal(directories.length, 17)
  assert.deepEqual(directories, [...PORTED_SKILLS, ...REVIEWER_SKILLS].sort())
  for (const skillName of directories) {
    assert.ok(statSync(path.join(SKILLS_DIR, skillName, 'SKILL.md')).isFile(), `${skillName}/SKILL.md missing`)
  }
})

test('every SKILL.md has exactly two frontmatter keys: name and description', () => {
  for (const skillName of [...PORTED_SKILLS, ...REVIEWER_SKILLS]) {
    const raw = readFileSync(path.join(SKILLS_DIR, skillName, 'SKILL.md'), 'utf8')
    const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(raw)
    assert.ok(match !== null, `${skillName}: missing frontmatter`)
    const keys = match[1]
      .split(/\r?\n/)
      .filter((line) => /^[A-Za-z][A-Za-z0-9_-]*:/.test(line))
      .map((line) => line.slice(0, line.indexOf(':')))
      .sort()
    assert.deepEqual(keys, ['description', 'name'], `${skillName}: frontmatter keys ${keys}`)
    assert.ok(!match[1].includes('allowed-tools'), `${skillName}: keep allowed-tools out of the shipped asset`)
    assert.ok(!match[1].includes('argument-hint'), `${skillName}: keep argument-hint out of the shipped asset`)
  }
})

test('every SKILL.md body is non-trivial and free of Claude $ARGUMENTS substitution', () => {
  for (const skillName of [...PORTED_SKILLS, ...REVIEWER_SKILLS]) {
    const raw = readFileSync(path.join(SKILLS_DIR, skillName, 'SKILL.md'), 'utf8')
    assert.ok(!raw.includes('$ARGUMENTS'), `${skillName}: $ARGUMENTS is not substituted by DSH`)
    const parsed = parseSkill(raw, skillName)
    assert.equal(parsed.name, skillName, `${skillName}: frontmatter name must match the directory`)
    assert.ok(parsed.description.trim().length >= 20, `${skillName}: description too short`)
    assert.ok(parsed.content.length >= 100, `${skillName}: body too short`)
  }
})

test('reviewer skills are skill-shaped ports (no role/tool jargon in frontmatter)', () => {
  for (const skillName of REVIEWER_SKILLS) {
    const raw = readFileSync(path.join(SKILLS_DIR, skillName, 'SKILL.md'), 'utf8')
    const { description } = parseSkill(raw, skillName)
    assert.ok(description.length > 0)
    // Converted reviewer agents must instruct the model to review the named target.
    assert.match(raw, /review/i, `${skillName}: reviewer body should describe a review task`)
  }
})

test('parseSkill tolerates quoted values, CRLF and missing frontmatter', () => {
  assert.deepEqual(parseSkill('---\nname: "my-skill"\ndescription: "Does things."\n---\nBody', 'fallback'), {
    name: 'my-skill',
    description: 'Does things.',
    content: 'Body',
  })
  assert.deepEqual(parseSkill("---\r\nname: 'a-b'\r\ndescription: \"x\"\r\n---\r\n\r\nBody\r\n", 'fallback'), {
    name: 'a-b',
    description: 'x',
    content: 'Body',
  })
  assert.deepEqual(parseSkill('Body only', 'fallback'), {
    name: 'fallback',
    description: 'govctl skill: fallback',
    content: 'Body only',
  })
})

test('manifest records the ported govctl version and deterministic hashes', () => {
  assert.equal(typeof MANIFEST.govctlVersion, 'string')
  assert.match(MANIFEST.govctlVersion, /^\d+\.\d+\.\d+$/)
  assert.equal(MANIFEST.source, '.claude')
  assert.equal(MANIFEST.skills.length, 17)

  const names = MANIFEST.skills.map((entry) => entry.name)
  assert.deepEqual(names, [...names].sort(), 'manifest entries must be sorted by name')
  for (const entry of MANIFEST.skills) {
    assert.match(entry.source, /^\.claude\/(skills|agents)\//)
    const asset = readFileSync(path.join(SKILLS_DIR, entry.name, 'SKILL.md'))
    assert.equal(createHash('sha256').update(asset).digest('hex'), entry.sha256, `${entry.name}: sha256 drift`)
  }
})

test('manifest hash source mapping resolves inside the repository when .claude is present', () => {
  for (const entry of MANIFEST.skills) {
    const sourcePath = path.join(PACKAGE_ROOT, '..', entry.source)
    const exists = statSync(sourcePath, { throwIfNoEntry: false })
    if (exists === undefined) continue // package checked out standalone; hash check still applies
    assert.ok(exists.isFile(), `${entry.name}: manifest source ${entry.source} must be a file`)
  }
})
