/**
 * Tool → govctl argv contract.
 *
 * Every registered tool is a thin transport over the govctl CLI. These tests
 * drive each tool's registered `execute(args, exec)` against a mock
 * `ctx.subprocess` (resolveExecutable → fake executable, spawn → fake handle)
 * and assert the exact argv passed to the binary.
 *
 * The CLI grammar under test (govctl 0.22.x):
 *   <resource> list|get|show   (resource ∈ rfc,adr,work,guard,clause,conformance)
 *   search/check/status/verify/describe/render
 *   conformance trace
 *   <resource> edit <id> <path> --set|--add|--remove|--tick [value] [--stdin]
 */
import assert from 'node:assert/strict'
import test from 'node:test'

import { apply } from '../lib/index.js'
import {
  applyPlugin,
  callTool,
  execContext,
  govctlSpawns,
  lastGovctlArgv,
} from './helpers/mock-ctx.js'

/** Apply the plugin with a successful fake govctl and return captured state. */
async function setup(spawnResult = {}) {
  const { state } = await applyPlugin(apply, {
    executable: '/fake/govctl',
    spawnResult: { stdout: 'ok', stderr: '', exitCode: 0, ...spawnResult },
  })
  return state
}

async function argvOf(state, toolName, args) {
  const result = await callTool(state, toolName, args)
  const calls = govctlSpawns(state)
  assert.ok(calls.length > 0, `${toolName} did not spawn govctl`)
  const request = calls[calls.length - 1]
  assert.equal(request.argv[0], '/fake/govctl')
  assert.equal(result, 'ok')
  return request.argv.slice(1)
}

test('the runner resolves the configured binary name through ctx.subprocess', async () => {
  const state = await setup()
  assert.ok(state.resolveExecutableCalls.includes('govctl'), `resolved candidates: ${state.resolveExecutableCalls}`)
})

test('spawn requests carry the resolved cwd, tool signal and bounded stdio', async () => {
  const state = await setup()
  const exec = execContext({ signal: undefined })
  await callTool(state, 'govctl_status', {}, exec)
  const request = govctlSpawns(state).at(-1)
  assert.deepEqual(request.argv, ['/fake/govctl', 'status'])
  assert.ok(typeof request.cwd === 'string' && request.cwd.length > 0)
  assert.equal(request.stdio.stdin, 'ignore')
  assert.equal(request.stdio.stdout.maxBytes, 65_536)
  assert.equal(request.stdio.stderr.maxBytes, 65_536)
  assert.equal(request.graceMs, 2_000)
  assert.ok('signal' in request)
})

test('govctl_status / govctl_check / govctl_describe argv', async () => {
  const state = await setup()
  assert.deepEqual(await argvOf(state, 'govctl_status', {}), ['status'])
  assert.deepEqual(await argvOf(state, 'govctl_check', {}), ['check'])
  assert.deepEqual(await argvOf(state, 'govctl_describe', {}), ['describe'])
  assert.deepEqual(await argvOf(state, 'govctl_describe', { context: true }), ['describe', '--context'])
})

test('govctl_list argv: resource list [filter] [-n limit] [--tag tag] -o json', async () => {
  const state = await setup()
  assert.deepEqual(await argvOf(state, 'govctl_list', { resource: 'work' }), ['work', 'list', '-o', 'json'])
  assert.deepEqual(
    await argvOf(state, 'govctl_list', { resource: 'work', filter: 'active', limit: 5, tag: 'cli' }),
    ['work', 'list', 'active', '-n', '5', '--tag', 'cli', '-o', 'json'],
  )
  assert.deepEqual(
    await argvOf(state, 'govctl_list', { resource: 'clause', filter: 'RFC-0002:C-SCOPE' }),
    ['clause', 'list', 'RFC-0002:C-SCOPE', '-o', 'json'],
  )
})

test('govctl_get argv: no field -> -o json; field -> positional path + -o plain', async () => {
  const state = await setup()
  assert.deepEqual(
    await argvOf(state, 'govctl_get', { resource: 'work', id: 'WI-2026-10-18-001' }),
    ['work', 'get', 'WI-2026-10-18-001', '-o', 'json'],
  )
  assert.deepEqual(
    await argvOf(state, 'govctl_get', { resource: 'work', id: 'WI-2026-10-18-001', field: 'acceptance_criteria[0].status' }),
    ['work', 'get', 'WI-2026-10-18-001', 'acceptance_criteria[0].status', '-o', 'plain'],
  )
})

test('govctl_show argv: default -o plain, explicit format preserved', async () => {
  const state = await setup()
  assert.deepEqual(
    await argvOf(state, 'govctl_show', { resource: 'rfc', id: 'RFC-0002' }),
    ['rfc', 'show', 'RFC-0002', '-o', 'plain'],
  )
  assert.deepEqual(
    await argvOf(state, 'govctl_show', { resource: 'rfc', id: 'RFC-0002', format: 'json' }),
    ['rfc', 'show', 'RFC-0002', '-o', 'json'],
  )
})

test('govctl_search argv: query, repeated --type, --tag, -n, -o json', async () => {
  const state = await setup()
  assert.deepEqual(await argvOf(state, 'govctl_search', { query: 'caching' }), ['search', 'caching', '-o', 'json'])
  assert.deepEqual(
    await argvOf(state, 'govctl_search', {
      query: 'work item',
      types: ['rfc', 'work'],
      tag: 'cli',
      limit: 7,
    }),
    ['search', 'work item', '--type', 'rfc', '--type', 'work', '--tag', 'cli', '-n', '7', '-o', 'json'],
  )
})

test('govctl_verify argv: --work vs positional guard IDs', async () => {
  const state = await setup()
  assert.deepEqual(await argvOf(state, 'govctl_verify', { work: 'WI-2026-10-18-001' }), [
    'verify',
    '--work',
    'WI-2026-10-18-001',
  ])
  assert.deepEqual(await argvOf(state, 'govctl_verify', { guards: ['GUARD-CLIPPY', 'GUARD-TESTS'] }), [
    'verify',
    'GUARD-CLIPPY',
    'GUARD-TESTS',
  ])
  // --work wins when both are supplied (CLI flags them as conflicting).
  assert.deepEqual(await argvOf(state, 'govctl_verify', { work: 'WI-1', guards: ['GUARD-X'] }), ['verify', '--work', 'WI-1'])
})

test('govctl_trace argv: conformance trace [target] -o json', async () => {
  const state = await setup()
  assert.deepEqual(await argvOf(state, 'govctl_trace', {}), ['conformance', 'trace', '-o', 'json'])
  assert.deepEqual(await argvOf(state, 'govctl_trace', { target: 'RFC-0002' }), [
    'conformance',
    'trace',
    'RFC-0002',
    '-o',
    'json',
  ])
})

test('govctl_edit argv: set/add/remove/tick and --stdin', async () => {
  const state = await setup()
  assert.deepEqual(
    await argvOf(state, 'govctl_edit', { resource: 'work', id: 'WI-1', path: 'description', action: 'set', value: 'Scope' }),
    ['work', 'edit', 'WI-1', 'description', '--set', 'Scope'],
  )
  assert.deepEqual(
    await argvOf(state, 'govctl_edit', { resource: 'work', id: 'WI-1', path: 'depends_on', action: 'add', value: 'WI-2' }),
    ['work', 'edit', 'WI-1', 'depends_on', '--add', 'WI-2'],
  )
  assert.deepEqual(
    await argvOf(state, 'govctl_edit', { resource: 'work', id: 'WI-1', path: 'acceptance_criteria[0]', action: 'tick', value: 'done' }),
    ['work', 'edit', 'WI-1', 'acceptance_criteria[0]', '--tick', 'done'],
  )
  assert.deepEqual(
    await argvOf(state, 'govctl_edit', { resource: 'work', id: 'WI-1', path: 'acceptance_criteria[0]', action: 'remove' }),
    ['work', 'edit', 'WI-1', 'acceptance_criteria[0]', '--remove'],
  )
  assert.deepEqual(
    await argvOf(state, 'govctl_edit', {
      resource: 'work',
      id: 'WI-1',
      path: 'description',
      action: 'set',
      value: 'from stdin',
      stdin: true,
    }),
    ['work', 'edit', 'WI-1', 'description', '--set', 'from stdin', '--stdin'],
  )
})

test('govctl_edit rejects missing values without spawning govctl', async () => {
  const state = await setup()
  const before = govctlSpawns(state).length
  const setError = await callTool(state, 'govctl_edit', {
    resource: 'work',
    id: 'WI-1',
    path: 'description',
    action: 'set',
  })
  assert.match(String(setError), /action "set" needs a value or stdin=true/)
  const tickError = await callTool(state, 'govctl_edit', {
    resource: 'work',
    id: 'WI-1',
    path: 'acceptance_criteria[0]',
    action: 'tick',
  })
  assert.match(String(tickError), /action "tick" needs a value/)
  assert.equal(govctlSpawns(state).length, before, 'invalid edits must not spawn govctl')
})

test('govctl_render argv: target defaults to rfc, --dry-run only when true', async () => {
  const state = await setup()
  assert.deepEqual(await argvOf(state, 'govctl_render', {}), ['render', 'rfc'])
  assert.deepEqual(await argvOf(state, 'govctl_render', { target: 'adr', dryRun: true }), ['render', 'adr', '--dry-run'])
  assert.deepEqual(await argvOf(state, 'govctl_render', { target: 'all' }), ['render', 'all'])
})

test('govctl_work_new argv: title and optional --active', async () => {
  const state = await setup()
  assert.deepEqual(await argvOf(state, 'govctl_work_new', { title: 'Implement feature' }), [
    'work',
    'new',
    'Implement feature',
  ])
  assert.deepEqual(await argvOf(state, 'govctl_work_new', { title: 'Implement feature', active: true }), [
    'work',
    'new',
    'Implement feature',
    '--active',
  ])
})

test('govctl_work_move argv: id and target status', async () => {
  const state = await setup()
  assert.deepEqual(await argvOf(state, 'govctl_work_move', { id: 'WI-1', status: 'done' }), ['work', 'move', 'WI-1', 'done'])
})

test('govctl_run passes an explicit argv array and rejects empty input', async () => {
  const state = await setup()
  assert.deepEqual(await argvOf(state, 'govctl_run', { argv: ['rfc', 'show', 'RFC-0002'] }), [
    'rfc',
    'show',
    'RFC-0002',
  ])
  assert.deepEqual(await argvOf(state, 'govctl_run', { argv: ['work', 'list', 'active', '-o', 'json'] }), [
    'work',
    'list',
    'active',
    '-o',
    'json',
  ])
  const before = govctlSpawns(state).length
  const error = await callTool(state, 'govctl_run', { argv: [] })
  assert.match(String(error), /argv must be a non-empty array/)
  assert.equal(govctlSpawns(state).length, before, 'empty argv must not spawn govctl')
})

test('non-zero exit is rendered with the exit code and stderr', async () => {
  const { state } = await applyPlugin(apply, {
    executable: '/fake/govctl',
    spawnResult: { stdout: 'partial', stderr: 'E0001: broken', exitCode: 3 },
  })
  const text = await callTool(state, 'govctl_status', {})
  assert.match(text, /govctl exited with code 3/)
  assert.match(text, /partial/)
  assert.match(text, /\[stderr\]\nE0001: broken/)
})

test('spawn failures are surfaced as model-readable errors, not crashes', async () => {
  const { state } = await applyPlugin(apply, {
    executable: '/fake/govctl',
    spawnResult: { doneError: new Error('spawn ENOENT') },
  })
  const text = await callTool(state, 'govctl_list', { resource: 'rfc' })
  assert.match(text, /govctl exited with code 1/)
  assert.match(text, /spawn ENOENT/)
})

test('a missing govctl binary returns the actionable install hint', async () => {
  const { state } = await applyPlugin(apply, {
    resolveExecutable: async () => {
      throw new Error('not found')
    },
    spawnResult: { stdout: '', stderr: '', exitCode: 0 },
  })
  const text = await callTool(state, 'govctl_status', {})
  assert.match(text, /govctl binary not found/)
  assert.match(text, /cargo binstall govctl/)
  assert.match(text, /binaryPath/)
})
