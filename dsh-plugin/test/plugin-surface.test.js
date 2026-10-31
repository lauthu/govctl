/**
 * Surface contract of the govctl-dsh bundle: tools, approval gate, skills,
 * commands, prompt contributions and event listeners.
 *
 * These assertions encode the task contract (14 tools / 17 skills / 2 commands
 * / prompt section order 700 / prompt context order 130 / 4 listeners).
 */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

import { apply, MUTATING_TOOLS, name as pluginName, inject } from '../lib/index.js'
import {
  applyPlugin,
  capturedConsoleLines,
  createGovernedProject,
  createUngovernedDir,
  execContext,
  listenerFor,
  removeTree,
} from './helpers/mock-ctx.js'

const PACKAGE_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..')

const READ_TOOL_NAMES = [
  'govctl_check',
  'govctl_describe',
  'govctl_get',
  'govctl_list',
  'govctl_search',
  'govctl_show',
  'govctl_status',
  'govctl_trace',
]

const MUTATING_TOOL_NAMES = [
  'govctl_edit',
  'govctl_render',
  'govctl_run',
  'govctl_verify',
  'govctl_work_move',
  'govctl_work_new',
]

const ALL_TOOL_NAMES = [...READ_TOOL_NAMES, ...MUTATING_TOOL_NAMES].sort()

test('exports plugin identity and required service injections', () => {
  assert.equal(pluginName, 'govctl-dsh')
  for (const service of ['tools', 'systemPrompt', 'skills', 'commands', 'subprocess', 'workingDirectory']) {
    assert.ok(inject.includes(service), `inject must include ${service}`)
  }
})

test('registers exactly 14 tools: 8 ungated reads + 6 approval-gated mutations/executions', async () => {
  const { state } = await applyPlugin(apply)
  const names = state.tools.map((definition) => definition.name).sort()
  // 14 tool definitions: the 5 typed mutating tools, the gated verify tool, and
  // 8 ungated read-only tools.
  assert.equal(state.tools.length, 14, `expected 14 tools, got: ${names.join(', ')}`)
  assert.deepEqual(READ_TOOL_NAMES, [
    'govctl_check',
    'govctl_describe',
    'govctl_get',
    'govctl_list',
    'govctl_search',
    'govctl_show',
    'govctl_status',
    'govctl_trace',
  ])
  assert.equal(ALL_TOOL_NAMES.length, 14)
  // Read-only tools (verify is approval-gated despite being read-shaped).
  for (const name of [...READ_TOOL_NAMES, ...MUTATING_TOOL_NAMES]) {
    assert.ok(names.includes(name), `missing tool ${name}`)
  }
  assert.deepEqual(
    names.filter((toolName) => !READ_TOOL_NAMES.includes(toolName) && !MUTATING_TOOL_NAMES.includes(toolName)),
    [],
  )
})

test('every tool definition has a schema-valid output, description and timeout', async () => {
  const { state } = await applyPlugin(apply)
  for (const definition of state.tools) {
    assert.equal(typeof definition.name, 'string')
    assert.ok(
      typeof definition.description === 'string' && definition.description.trim().length > 0,
      `tool ${definition.name} needs a non-empty description`,
    )
    assert.equal(typeof definition.execute, 'function', `tool ${definition.name} needs execute()`)
    assert.deepEqual(definition.output.schema, { type: 'string' })
    assert.equal(typeof definition.output.render, 'function')
    assert.equal(definition.timeoutMs, 20_000)
    assert.equal(typeof definition.parameters, 'object')
    assert.equal(definition.parameters.type, 'object')
  }
})

test('MUTATING_TOOLS exports the six approval-gated tool names', () => {
  assert.ok(MUTATING_TOOLS instanceof Set)
  assert.deepEqual([...MUTATING_TOOLS].sort(), MUTATING_TOOL_NAMES)
})

test('tools/pre-execute returns {kind:ask} for mutating tools and next() for read tools', async () => {
  const { state } = await applyPlugin(apply)
  const gate = listenerFor(state, 'tools/pre-execute')
  assert.equal(typeof gate, 'function', 'tools/pre-execute listener must be registered')

  for (const mutatingName of MUTATING_TOOL_NAMES) {
    const decision = await gate({ name: mutatingName, arguments: {} }, () => Promise.resolve({ kind: 'allow' }))
    assert.equal(decision.kind, 'ask', `${mutatingName} must be approval-gated`)
    assert.equal(typeof decision.reason, 'string')
    assert.ok(decision.reason.includes(mutatingName), 'ask reason should name the tool')
  }

  // Read tools delegate to the next listener (default allow) unchanged.
  for (const readName of READ_TOOL_NAMES) {
    const decision = await gate({ name: readName, arguments: {} }, () => Promise.resolve({ kind: 'allow' }))
    assert.equal(decision.kind, 'allow', `${readName} must not be gated`)
  }

  // A downstream deny survives for read tools.
  const denied = await gate({ name: 'govctl_status', arguments: {} }, () => Promise.resolve({ kind: 'deny', reason: 'nope' }))
  assert.deepEqual(denied, { kind: 'deny', reason: 'nope' })
})

test('every mutating tool is registered and gated, and no read tool is gated', async () => {
  const { state } = await applyPlugin(apply)
  const gate = listenerFor(state, 'tools/pre-execute')
  for (const definition of state.tools) {
    const decision = await gate({ name: definition.name, arguments: {} }, () => Promise.resolve({ kind: 'allow' }))
    const expected = MUTATING_TOOLS.has(definition.name) ? 'ask' : 'allow'
    assert.equal(decision.kind, expected, `${definition.name} gate mismatch`)
  }
})

test('registers 17 skills with non-empty descriptions and normalized content', async () => {
  const { state } = await applyPlugin(apply)
  assert.equal(state.skills.length, 17)

  const names = state.skills.map((skill) => skill.name).sort()
  for (const skillName of names) {
    assert.match(skillName, /^[a-z0-9]+(?:-[a-z0-9]+)*$/, `invalid kebab-case skill name: ${skillName}`)
  }
  for (const skill of state.skills) {
    assert.ok(typeof skill.description === 'string' && skill.description.trim().length > 0, `empty description: ${skill.name}`)
    assert.ok(typeof skill.content === 'string' && skill.content.trim().length > 0, `empty content: ${skill.name}`)
    assert.ok(!skill.content.includes('$ARGUMENTS'), `${skill.name} must not depend on Claude $ARGUMENTS substitution`)
    assert.ok(!/^allowed-tools:/m.test(skill.content), `${skill.name} must not carry Claude allowed-tools frontmatter`)
  }

  // The registered set must equal the files on disk (bundle assets are the source).
  const manifest = JSON.parse(readFileSync(path.join(PACKAGE_ROOT, 'assets', 'manifest.json'), 'utf8'))
  assert.deepEqual(
    names,
    manifest.skills.map((entry) => entry.name).sort(),
    'registered skills must match assets/manifest.json',
  )
})

test('registers exactly 2 slash commands', async () => {
  const { state } = await applyPlugin(apply)
  assert.equal(state.commands.length, 2)
  const names = state.commands.map((command) => command.name).sort()
  assert.deepEqual(names, ['gov-check', 'gov-status'])
  for (const command of state.commands) {
    assert.ok(command.description.trim().length > 0)
    assert.equal(typeof command.handler, 'function')
  }
})

test('registers one policy prompt section at order 700, governed-project scoped', async () => {
  const projectRoot = createGovernedProject()
  const ungovernedDir = createUngovernedDir()
  const { state } = await applyPlugin(apply, { cwd: projectRoot })
  try {
    assert.equal(state.sections.length, 1)
    const section = state.sections[0]
    assert.equal(section.name, 'policy:govctl')
    assert.equal(section.order, 700)
    assert.equal(typeof section.text, 'function')

    const governedText = section.text({ agent: { session: { id: 's' } } })
    assert.match(governedText, /Governance mode \(govctl\)/)
    assert.match(governedText, /RFC supremacy/)

    // No agent → no contribution; a cwd outside any governed tree → no contribution.
    assert.equal(section.text({}), '')
    assert.equal(section.text(undefined), '')
    const ungoverned = await applyPlugin(apply, { cwd: ungovernedDir })
    assert.equal(ungoverned.state.sections[0].text({ agent: { session: { id: 's2' } } }), '')
  } finally {
    removeTree(projectRoot)
    removeTree(ungovernedDir)
  }
})

test('policy section is contributed for a session cwd below the governed root', async () => {
  const projectRoot = createGovernedProject()
  const subdir = path.join(projectRoot, 'docs', 'nested')
  const { mkdirSync } = await import('node:fs')
  mkdirSync(subdir, { recursive: true })
  const { state } = await applyPlugin(apply, { cwd: subdir })
  try {
    const agent = { session: { id: 's' } }
    assert.match(state.sections[0].text({ agent }), /Governance mode \(govctl\)/)
  } finally {
    removeTree(projectRoot)
  }
})

test('registers one runtime prompt context at order 130 (peer-less fallback)', async () => {
  const projectRoot = createGovernedProject()
  const { state } = await applyPlugin(apply, { cwd: projectRoot })
  try {
    assert.equal(state.contexts.length, 1)
    const context = state.contexts[0]
    assert.equal(context.name, 'govctl:state')
    assert.equal(context.order, 130)
    assert.equal(typeof context.text, 'function')
    // Nothing cached before the session-start hook completes.
    assert.equal(context.text({ agent: { session: { id: 'unknown-session' } } }), '')
  } finally {
    removeTree(projectRoot)
  }
})

test('registers the four expected event listeners by default', async () => {
  const { state } = await applyPlugin(apply)
  const events = state.listeners.map((entry) => entry.event).sort()
  assert.deepEqual(events, ['agent/created', 'agent/pre-step', 'tools/post-execute', 'tools/pre-execute'])
})

test('emits the load banner and skill count on stdout, on a single channel', async () => {
  const { state } = await applyPlugin(apply)
  const lines = capturedConsoleLines()
  assert.ok(
    lines.some((line) => line === '[govctl-dsh] loaded (binary: govctl)'),
    `missing stdout load banner in: ${JSON.stringify(lines)}`,
  )
  assert.ok(
    lines.some((line) => line.includes('registered 17 bundled skills')),
    `missing stdout skill count in: ${JSON.stringify(lines)}`,
  )
  // The banner must not also travel through ctx.logger (one channel only).
  assert.deepEqual(
    state.logLines.info.filter((line) => line.includes('[govctl-dsh]')),
    [],
    `banner must not be duplicated through ctx.logger: ${JSON.stringify(state.logLines.info)}`,
  )
})

test('tools are usable with the approval exec context shape', async () => {
  const exec = execContext()
  assert.equal(typeof exec.agent.session.id, 'string')
})
