/**
 * Runner internals: binary resolution, result rendering, hook stdin protocol
 * and version compatibility warnings.
 */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'

import { createRunner, renderResult } from '../lib/run.js'
import { apply } from '../lib/index.js'
import {
  applyPlugin,
  capturedConsoleLines,
  createFakeGovctlScript,
  createGovernedProject,
  createMockContext,
  removeTree,
} from './helpers/mock-ctx.js'

test('renderResult: success returns trimmed stdout, empty success falls back to stderr', () => {
  assert.equal(renderResult({ code: 0, stdout: 'ok\n', stderr: '' }), 'ok')
  assert.equal(renderResult({ code: 0, stdout: '', stderr: 'warning' }), 'warning')
  assert.equal(renderResult({ code: 0, stdout: '  ', stderr: 'warn' }), 'warn')
})

test('renderResult: failure names the exit code and includes stdout/stderr', () => {
  const text = renderResult({ code: 2, stdout: 'out', stderr: 'E0001: bad' })
  assert.match(text, /^govctl exited with code 2/m)
  assert.match(text, /out/)
  assert.match(text, /\[stderr\]\nE0001: bad/)
})

test('renderResult bounds output to maxOutputBytes bytes with a truncation marker', () => {
  const text = renderResult({ code: 0, stdout: 'x'.repeat(200), stderr: '' }, 50)
  assert.ok(Buffer.byteLength(text, 'utf8') <= 50, `expected <=50 bytes, got ${Buffer.byteLength(text, 'utf8')}`)
  assert.match(text, /\[truncated \d+ bytes\]/)
  assert.equal(renderResult({ code: 0, stdout: 'x'.repeat(10), stderr: '' }, 50), 'x'.repeat(10))
})

test('createRunner resolves the configured binary through ctx.subprocess', async () => {
  const { ctx, state } = createMockContext({ executable: '/fake/govctl' })
  const runner = createRunner(ctx, { binaryPath: 'govctl', toolTimeoutMs: 1_000, maxOutputBytes: 4_096 })
  const result = await runner.runRaw(['status'], { cwd: '/workspace' })
  assert.deepEqual(result, { code: 0, stdout: '', stderr: '' })
  assert.deepEqual(state.spawns.at(-1).argv, ['/fake/govctl', 'status'])
  assert.equal(state.spawns.at(-1).cwd, '/workspace')
  assert.equal(state.spawns.at(-1).stdio.stdout.maxBytes, 4_096)
})

test('createRunner reports an actionable error when every candidate fails', async () => {
  const { ctx } = createMockContext({
    resolveExecutable: async () => {
      throw new Error('missing')
    },
  })
  const runner = createRunner(ctx, { binaryPath: '/opt/none/govctl', toolTimeoutMs: 1_000, maxOutputBytes: 4_096 })
  const result = await runner.runRaw(['status'], {})
  assert.equal(result.code, 1)
  assert.equal(result.unavailable, true)
  assert.match(result.stderr, /govctl binary not found \(tried: \/opt\/none\/govctl, govctl\)/)
  assert.match(result.stderr, /cargo binstall govctl/)
})

test('createRunner version parses `<major>.<minor>.<patch>` and warns off the tested line', async () => {
  const probe = async (stdout) => {
    const { ctx, state } = createMockContext({ executable: '/fake/govctl', spawnResult: { stdout, exitCode: 0 } })
    const runner = createRunner(ctx, { binaryPath: 'govctl', toolTimeoutMs: 1_000, maxOutputBytes: 4_096 })
    const version = await runner.version()
    await runner.checkCompatibility()
    return { version, state }
  }

  const onLine = await probe('govctl 0.22.3\n')
  assert.deepEqual(onLine.version, { raw: '0.22.3', major: 0, minor: 22, patch: 3 })
  assert.equal(
    onLine.state.spawns.filter((entry) => entry.argv[1] === '--version').length,
    1,
    'version probe must run `govctl --version` once and cache it',
  )

  const offLine = await probe('govctl 0.23.0\n')
  assert.equal(offLine.version.minor, 23)
  assert.match(capturedConsoleLines().join('\n'), /outside the tested 0\.22\.x range/)

  await assert.rejects(() => probe('weird output\n'), /unrecognized govctl version output/)
})

test('hook(): sends {cwd, tool_input} on stdin and reads additionalContext', async () => {
  const projectRoot = createGovernedProject()
  const fake = createFakeGovctlScript({
    stdout: JSON.stringify({ hookSpecificOutput: { hookEventName: 'SessionStart', additionalContext: 'STATE' } }),
  })
  try {
    const { ctx } = createMockContext({ executable: fake.file })
    const runner = createRunner(ctx, { binaryPath: fake.file, toolTimeoutMs: 1_000, maxOutputBytes: 4_096 })
    const context = await runner.hook('session-start', projectRoot, {})
    assert.equal(context, 'STATE')
    const payload = JSON.parse(readFileSync(fake.capturePath, 'utf8'))
    assert.deepEqual(payload, { cwd: projectRoot, tool_input: {} })
  } finally {
    removeTree(projectRoot)
    removeTree(fake.dir)
  }
})

test('hook(): non-zero exit resolves undefined, plain-text output falls back to the raw text', async () => {
  const cwd = createGovernedProject()
  const failing = createFakeGovctlScript({ stdout: 'ignored', exitCode: 3 })
  const plain = createFakeGovctlScript({ stdout: 'raw context text' })
  try {
    for (const [schema, expected] of [
      [failing, undefined],
      [plain, 'raw context text'],
    ]) {
      const { ctx } = createMockContext({ executable: schema.file })
      const runner = createRunner(ctx, {
        binaryPath: schema.file,
        toolTimeoutMs: 1_000,
        maxOutputBytes: 4_096,
      })
      // eslint-disable-next-line no-await-in-loop
      assert.equal(await runner.hook('session-start', cwd, {}), expected)
    }
  } finally {
    removeTree(cwd)
    removeTree(failing.dir)
    removeTree(plain.dir)
  }
})

test('apply() warns (without failing) when the installed govctl is off the tested line', async () => {
  const { state } = await applyPlugin(apply, {
    executable: '/fake/govctl',
    spawnResult: { stdout: 'govctl 0.99.1\n', exitCode: 0 },
  })
  await new Promise((resolve) => setImmediate(resolve))
  const lines = capturedConsoleLines().join('\n')
  assert.match(
    lines,
    /outside the tested 0\.22\.x range/,
    `expected a compatibility warning, got: ${JSON.stringify(capturedConsoleLines())}`,
  )
})

test('hook(): the child environment is scrubbed of credentials and DSH_* facts', async () => {
  const fake = createFakeGovctlScript({ stdout: '{}' })
  const saved = {
    apiKey: process.env.GOVCTL_TEST_API_KEY,
    dshFact: process.env.DSH_TEST_FACT,
  }
  process.env.GOVCTL_TEST_API_KEY = 'super-secret-value'
  process.env.DSH_TEST_FACT = 'deployment-fact'
  try {
    const { ctx } = createMockContext({ executable: fake.file })
    const runner = createRunner(ctx, { binaryPath: fake.file, toolTimeoutMs: 1_000, maxOutputBytes: 4_096 })
    await runner.hook('session-start', process.cwd(), {})
    const childEnv = readFileSync(fake.envCapturePath, 'utf8')
    assert.ok(!childEnv.includes('GOVCTL_TEST_API_KEY'), 'credential-shaped names must be scrubbed')
    assert.ok(!childEnv.includes('super-secret-value'), 'credential values must not reach the child')
    assert.ok(!childEnv.includes('DSH_TEST_FACT'), 'DSH_* deployment facts must be scrubbed')
    // Ordinary ambient facts survive so govctl runs normally.
    assert.match(childEnv, /(^|\n)PATH=/, 'PATH must survive the scrub')
  } finally {
    if (saved.apiKey === undefined) delete process.env.GOVCTL_TEST_API_KEY
    else process.env.GOVCTL_TEST_API_KEY = saved.apiKey
    if (saved.dshFact === undefined) delete process.env.DSH_TEST_FACT
    else process.env.DSH_TEST_FACT = saved.dshFact
    removeTree(fake.dir)
  }
})

test('checkCompatibility at apply() time never fails the plugin for a missing binary', async () => {
  const { state } = await applyPlugin(apply, {
    resolveExecutable: async () => {
      throw new Error('govctl not installed')
    },
  })
  // apply() resolved, tools are registered, and no unhandled rejection escaped.
  assert.equal(state.tools.length, 14)
  await new Promise((resolve) => setImmediate(resolve))
})

test('an aborting signal is forwarded to ctx.subprocess.spawn', async () => {
  const controller = new AbortController()
  const { state } = await applyPlugin(apply, { executable: '/fake/govctl' })
  const tool = state.tools.find((definition) => definition.name === 'govctl_status')
  await tool.execute({}, { agent: { session: { id: 's' } }, signal: controller.signal })
  assert.equal(state.spawns.at(-1).signal, controller.signal)
})

test('cwd resolution falls back to the committed projection when ensure() fails', async () => {
  const projectRoot = process.cwd()
  const throwingEnsure = createMockContext({ cwd: projectRoot, ensureError: new Error('gone') })
  const runner = createRunner(throwingEnsure.ctx, { binaryPath: 'govctl', toolTimeoutMs: 1_000, maxOutputBytes: 512 })
  const resolved = await runner.run({ agent: { session: { id: 's' } }, signal: undefined }, ['status'])
  assert.equal(resolved.code, 0)
  assert.equal(throwingEnsure.state.spawns.at(-1).cwd, projectRoot)
})
