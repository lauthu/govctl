/**
 * Slash commands: /gov-status and /gov-check are read-only governance status
 * helpers and must fail cleanly outside a governed workspace.
 */
import assert from 'node:assert/strict'
import path from 'node:path'
import test from 'node:test'

import { apply } from '../lib/index.js'
import {
  applyPlugin,
  createGovernedProject,
  createUngovernedDir,
  govctlSpawns,
  removeTree,
} from './helpers/mock-ctx.js'

function commandByName(state, name) {
  return state.commands.find((command) => command.name === name)
}

test('registers /gov-status and /gov-check', async () => {
  const { state } = await applyPlugin(apply)
  const names = state.commands.map((command) => command.name).sort()
  assert.deepEqual(names, ['gov-check', 'gov-status'])
  assert.ok(state.commands.every((command) => typeof command.handler === 'function'))
})

test('/gov-status runs `govctl status` in the session cwd and returns success text', async () => {
  const projectRoot = createGovernedProject()
  const { state } = await applyPlugin(apply, {
    cwd: projectRoot,
    executable: '/fake/govctl',
    spawnResult: { stdout: 'Project: demo\n', exitCode: 0 },
  })
  try {
    const command = commandByName(state, 'gov-status')
    const result = await command.handler({ agent: { session: { id: 's1' } } })
    assert.equal(result.kind, 'success')
    assert.match(result.text, /Project: demo/)
    assert.deepEqual(govctlSpawns(state).at(-1).argv, ['/fake/govctl', 'status'])
    assert.equal(govctlSpawns(state).at(-1).cwd, projectRoot)
  } finally {
    removeTree(projectRoot)
  }
})

test('/gov-check runs `govctl check` and maps non-zero exits to error results', async () => {
  const projectRoot = createGovernedProject()
  const { state } = await applyPlugin(apply, {
    cwd: projectRoot,
    executable: '/fake/govctl',
    spawnResult: { stdout: '', stderr: 'E0001: broken', exitCode: 1 },
  })
  try {
    const command = commandByName(state, 'gov-check')
    const result = await command.handler({ agent: { session: { id: 's1' } } })
    assert.equal(result.kind, 'error')
    assert.match(result.text, /govctl exited with code 1/)
    assert.match(result.text, /E0001: broken/)
    assert.deepEqual(govctlSpawns(state).at(-1).argv, ['/fake/govctl', 'check'])
  } finally {
    removeTree(projectRoot)
  }
})

test('commands reject an ungoverned workspace without spawning govctl', async () => {
  // Must be outside this repository: the repo itself is a governed project.
  const ungoverned = createUngovernedDir()
  const { state } = await applyPlugin(apply, { cwd: ungoverned, executable: '/fake/govctl' })
  try {
    const before = govctlSpawns(state).length
    for (const name of ['gov-status', 'gov-check']) {
      // eslint-disable-next-line no-await-in-loop
      const result = await commandByName(state, name).handler({ agent: { session: { id: 's1' } } })
      assert.equal(result.kind, 'error')
      assert.match(result.text, /not governed/)
    }
    assert.equal(govctlSpawns(state).length, before, 'ungoverned commands must not spawn govctl')
  } finally {
    removeTree(ungoverned)
  }
})

test('commands accept a governed subdirectory cwd (ancestor discovery)', async () => {
  const projectRoot = createGovernedProject()
  const subdir = path.join(projectRoot, 'docs')
  const { mkdirSync } = await import('node:fs')
  mkdirSync(subdir, { recursive: true })
  const { state } = await applyPlugin(apply, {
    cwd: subdir,
    executable: '/fake/govctl',
    spawnResult: { stdout: 'ok', exitCode: 0 },
  })
  try {
    const result = await commandByName(state, 'gov-status').handler({ agent: { session: { id: 's1' } } })
    assert.equal(result.kind, 'success')
    assert.deepEqual(govctlSpawns(state).at(-1).argv, ['/fake/govctl', 'status'])
  } finally {
    removeTree(projectRoot)
  }
})

test('commands forward the invocation signal to the govctl subprocess', async () => {
  const projectRoot = createGovernedProject()
  const controller = new AbortController()
  const { state } = await applyPlugin(apply, {
    cwd: projectRoot,
    executable: '/fake/govctl',
    spawnResult: { stdout: 'ok', exitCode: 0 },
  })
  try {
    const result = await commandByName(state, 'gov-check').handler({
      agent: { session: { id: 's1' } },
      signal: controller.signal,
    })
    assert.equal(result.kind, 'success')
    assert.equal(govctlSpawns(state).at(-1).signal, controller.signal)
  } finally {
    removeTree(projectRoot)
  }
})

test('commands convert runner failures into error results instead of throwing', async () => {
  const projectRoot = createGovernedProject()
  const { state } = await applyPlugin(apply, {
    cwd: projectRoot,
    executable: '/fake/govctl',
    spawnResult: { doneError: new Error('spawn ENOENT') },
  })
  try {
    const result = await commandByName(state, 'gov-status').handler({ agent: { session: { id: 's1' } } })
    assert.equal(result.kind, 'error')
    assert.match(result.text, /spawn ENOENT/)
  } finally {
    removeTree(projectRoot)
  }
})

test('commands surface an unavailable govctl binary as an error result', async () => {
  const projectRoot = createGovernedProject()
  const { state } = await applyPlugin(apply, {
    cwd: projectRoot,
    resolveExecutable: async () => {
      throw new Error('not installed')
    },
  })
  try {
    const result = await commandByName(state, 'gov-check').handler({ agent: { session: { id: 's1' } } })
    assert.equal(result.kind, 'error')
    assert.match(result.text, /govctl binary not found/)
  } finally {
    removeTree(projectRoot)
  }
})
