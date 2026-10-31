/**
 * Configuration contract: defaults, normalization and feature flags.
 *
 * Contract: binaryPath (string, default "govctl"), toolTimeoutMs (number,
 * 20_000), maxOutputBytes (number, 65_536), enablePolicySection /
 * enableSessionContext / enableEditAdvisory (booleans, default true).
 */
import assert from 'node:assert/strict'
import test from 'node:test'

import { apply } from '../lib/index.js'
import { applyPlugin, callTool, govctlSpawns, listenerFor } from './helpers/mock-ctx.js'

test('defaults: binary govctl, 20s tool timeout, 64 KiB output cap, gated features on', async () => {
  const { state } = await applyPlugin(apply, { executable: '/fake/govctl' })
  assert.deepEqual(state.resolveExecutableCalls, ['govctl'])
  const tool = state.tools.find((definition) => definition.name === 'govctl_status')
  assert.equal(tool.timeoutMs, 20_000)
  await callTool(state, 'govctl_status', {})
  const request = govctlSpawns(state).at(-1)
  assert.deepEqual(request.argv, ['/fake/govctl', 'status'])
  assert.equal(request.stdio.stdout.maxBytes, 65_536)
  assert.equal(request.stdio.stderr.maxBytes, 65_536)
  assert.ok(listenerFor(state, 'tools/pre-execute') !== undefined)
  assert.ok(listenerFor(state, 'agent/pre-step') !== undefined)
  assert.ok(listenerFor(state, 'tools/post-execute') !== undefined)
  assert.ok(listenerFor(state, 'agent/created') !== undefined)
})

test('custom config is honored: binaryPath, toolTimeoutMs, maxOutputBytes', async () => {
  const { state } = await applyPlugin(apply, {
    executable: '/fake/custom-govctl',
    config: { binaryPath: '/fake/custom-govctl', toolTimeoutMs: 1_234, maxOutputBytes: 128 },
  })
  assert.deepEqual(state.resolveExecutableCalls, ['/fake/custom-govctl'])
  const tool = state.tools.find((definition) => definition.name === 'govctl_status')
  assert.equal(tool.timeoutMs, 1_234)
  await callTool(state, 'govctl_status', {})
  const request = govctlSpawns(state).at(-1)
  assert.equal(request.argv[0], '/fake/custom-govctl')
  assert.equal(request.stdio.stdout.maxBytes, 128)
})

test('invalid config values fall back to defaults', async () => {
  const cases = [
    [undefined, 'govctl'],
    [null, 'govctl'],
    ['not-an-object', 'govctl'],
    [{}, 'govctl'],
    [{ binaryPath: '   ' }, 'govctl'],
    [{ binaryPath: 42 }, 'govctl'],
  ]
  for (const [config, expectedBinary] of cases) {
    const { state } = await applyPlugin(apply, { executable: '/fake/govctl', config })
    assert.deepEqual(state.resolveExecutableCalls, [expectedBinary], `config ${JSON.stringify(config)}`)
    assert.equal(state.tools[0].timeoutMs, 20_000)
  }

  for (const invalid of [0, -5, Number.NaN, Number.POSITIVE_INFINITY, '20000']) {
    const { state } = await applyPlugin(apply, {
      executable: '/fake/govctl',
      config: { toolTimeoutMs: invalid, maxOutputBytes: invalid },
    })
    const tool = state.tools.find((definition) => definition.name === 'govctl_status')
    assert.equal(tool.timeoutMs, 20_000, `toolTimeoutMs=${String(invalid)} must fall back`)
    await callTool(state, 'govctl_status', {})
    assert.equal(govctlSpawns(state).at(-1).stdio.stdout.maxBytes, 65_536, `maxOutputBytes=${String(invalid)} must fall back`)
  }
})

test('binaryPath falls back to PATH-resolved govctl when the configured path fails', async () => {
  const { state } = await applyPlugin(apply, {
    config: { binaryPath: '/opt/missing/govctl' },
    resolveExecutable: async (name) => {
      if (name !== 'govctl') throw new Error(`no ${name}`)
      return '/from/path/govctl'
    },
  })
  await callTool(state, 'govctl_status', {})
  assert.deepEqual(state.resolveExecutableCalls.slice(0, 2), ['/opt/missing/govctl', 'govctl'])
  assert.equal(govctlSpawns(state).at(-1).argv[0], '/from/path/govctl')
  assert.deepEqual(govctlSpawns(state).at(-1).argv, ['/from/path/govctl', 'status'])
})

test('an explicitly configured binaryPath is preferred when it resolves', async () => {
  const { state } = await applyPlugin(apply, {
    config: { binaryPath: '/opt/custom/govctl' },
    resolveExecutable: async (name) => `/resolved/${name}`,
  })
  await callTool(state, 'govctl_status', {})
  assert.deepEqual(state.resolveExecutableCalls, ['/opt/custom/govctl'])
  assert.equal(govctlSpawns(state).at(-1).argv[0], '/resolved//opt/custom/govctl')
})

test('enablePolicySection=false suppresses the policy section only', async () => {
  const { state } = await applyPlugin(apply, { config: { enablePolicySection: false } })
  assert.equal(state.sections.length, 0)
  assert.equal(state.contexts.length, 1)
  assert.ok(listenerFor(state, 'tools/pre-execute') !== undefined)
})

test('enableEditAdvisory=false suppresses the post-execute advisory listener', async () => {
  const { state } = await applyPlugin(apply, { config: { enableEditAdvisory: false } })
  assert.equal(listenerFor(state, 'tools/post-execute'), undefined)
  assert.ok(listenerFor(state, 'agent/pre-step') !== undefined)
  assert.equal(state.sections.length, 1)
})

test('enableSessionContext=false suppresses the pre-step listener and state context', async () => {
  const { state } = await applyPlugin(apply, { config: { enableSessionContext: false } })
  assert.equal(listenerFor(state, 'agent/pre-step'), undefined)
  assert.equal(state.contexts.find((context) => context.name === 'govctl:state'), undefined)
  assert.ok(listenerFor(state, 'tools/post-execute') !== undefined)
})

test('enableSessionContext=false also skips the session-start hook subprocess', async () => {
  // With session context disabled the fetched state is never delivered, so the
  // per-agent hook probe must not be registered at all.
  const { state } = await applyPlugin(apply, { config: { enableSessionContext: false } })
  assert.equal(
    listenerFor(state, 'agent/created'),
    undefined,
    'enableSessionContext=false must not register the session-start hook listener',
  )
})

test('all feature flags disabled leaves only the approval gate and tools', async () => {
  const { state } = await applyPlugin(apply, {
    config: { enablePolicySection: false, enableSessionContext: false, enableEditAdvisory: false },
  })
  assert.deepEqual(
    state.listeners.map((entry) => entry.event),
    ['tools/pre-execute'],
  )
  assert.equal(state.tools.length, 14)
})
