/**
 * Optional peer dependency: @deepseek-ai/dsh-llm.
 *
 * The plugin must work when the peer is absent (prompt-context fallback) and
 * upgrade to durable sourced user messages when it is present. Both branches
 * are exercised here: the fallback against the repo package (the peer is not
 * installed in this workspace) and the upgraded branch against a copied
 * package tree with a stub peer on the module-resolution path.
 */
import assert from 'node:assert/strict'
import { cpSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { apply } from '../lib/index.js'
import {
  applyPlugin,
  createFakeGovctlScript,
  createGovernedProject,
  createMockContext,
  flushAsync,
  listenerFor,
  removeTree,
  waitFor,
} from './helpers/mock-ctx.js'

const PACKAGE_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..')

function writeStubPeer(root) {
  const peerRoot = path.join(root, 'node_modules', '@deepseek-ai', 'dsh-llm')
  mkdirSync(peerRoot, { recursive: true })
  writeFileSync(
    path.join(peerRoot, 'package.json'),
    JSON.stringify({ name: '@deepseek-ai/dsh-llm', version: '0.2.1-rc.5', type: 'module', main: './index.js' }),
    'utf8',
  )
  writeFileSync(
    path.join(peerRoot, 'index.js'),
    [
      'export function createUserMessage(input) {',
      "  return Object.freeze({ id: 'stub-user-message', role: 'user', content: input.content, source: input.source })",
      '}',
      '',
    ].join('\n'),
    'utf8',
  )
}

/** Copy the package runtime into a scratch tree so a stub peer can be resolved. */
function copyPackageRuntime() {
  const root = mkdtempSync(path.join(os.tmpdir(), 'govctl-dsh-peer-'))
  cpSync(path.join(PACKAGE_ROOT, 'lib'), path.join(root, 'lib'), { recursive: true })
  cpSync(path.join(PACKAGE_ROOT, 'assets'), path.join(root, 'assets'), { recursive: true })
  return root
}

test('peer-absent: apply() succeeds and the prompt context carries session state', async () => {
  const projectRoot = createGovernedProject()
  const fake = createFakeGovctlScript({
    stdout: JSON.stringify({ hookSpecificOutput: { additionalContext: 'Active work: WI-9' } }),
  })
  const { state } = await applyPlugin(apply, {
    cwd: projectRoot,
    executable: fake.file,
    config: { binaryPath: fake.file },
  })
  try {
    assert.equal(state.tools.length, 14)
    assert.equal(state.contexts.length, 1)
    const agent = { session: { id: 's1' } }
    listenerFor(state, 'agent/created')({ agent })
    await waitFor(() => state.contexts[0].text({ agent }).length > 0)
    const text = state.contexts[0].text({ agent })
    assert.match(text, /Active work: WI-9/)

    // No peer factory means no message injection: pre-step decision is untouched.
    const preStep = listenerFor(state, 'agent/pre-step')
    const claimed = { id: 'm1', role: 'user' }
    const decision = { kind: 'enter', messages: [claimed] }
    const result = await preStep({ agent, messages: [claimed], turn: 1, step: 1 }, () =>
      Promise.resolve(decision),
    )
    assert.equal(result, decision)
  } finally {
    removeTree(projectRoot)
    removeTree(fake.dir)
  }
})

test('peer-present: agent/pre-step splices a sourced user message after the claimed batch', async () => {
  const copyRoot = copyPackageRuntime()
  const projectRoot = createGovernedProject()
  const fake = createFakeGovctlScript({
    stdout: JSON.stringify({ hookSpecificOutput: { additionalContext: 'Active work: WI-9' } }),
  })
  try {
    writeStubPeer(copyRoot)
    const copied = await import(pathToFileURL(path.join(copyRoot, 'lib', 'index.js')).href)
    const { ctx, state } = createMockContext({ cwd: projectRoot, executable: fake.file })
    copied.apply(ctx, { binaryPath: fake.file })
    await flushAsync()

    // Session-start state is fetched on creation.
    const agent = { session: { id: 's1' } }
    listenerFor(state, 'agent/created')({ agent })
    await waitFor(() => state.contexts[0].text({ agent }).length > 0)

    const preStep = listenerFor(state, 'agent/pre-step')
    const claimed = { id: 'm1', role: 'user', content: [{ type: 'text', text: 'do the thing' }] }
    const step1 = await preStep({ agent, messages: [claimed], turn: 1, step: 1 }, () =>
      Promise.resolve({ kind: 'enter', messages: [claimed] }),
    )
    assert.equal(step1.kind, 'enter')
    assert.equal(step1.messages.length, 2, 'one injected user message expected')
    assert.equal(step1.messages[0], claimed)
    const injected = step1.messages[1]
    assert.equal(injected.role, 'user')
    assert.deepEqual(injected.source, { kind: 'plugin', plugin: 'govctl-dsh' })
    assert.match(injected.content[0].text, /govctl governance state \(session start\)/)
    assert.match(injected.content[0].text, /Active work: WI-9/)

    // The fallback prompt context must yield to the durable message after injection.
    assert.equal(state.contexts[0].text({ agent }), '')

    // ... and injection happens at most once per session.
    const later = { id: 'm2', role: 'user' }
    const step2 = await preStep({ agent, messages: [later], turn: 2, step: 1 }, () =>
      Promise.resolve({ kind: 'enter', messages: [later] }),
    )
    assert.deepEqual(step2.messages, [later])

    // A downstream reject decision is preserved untouched.
    const reject = { kind: 'reject', reason: 'blocked' }
    const rejected = await preStep({ agent: { session: { id: 's2' } }, messages: [claimed], turn: 1, step: 1 }, () =>
      Promise.resolve(reject),
    )
    assert.equal(rejected, reject)
  } finally {
    removeTree(copyRoot)
    removeTree(projectRoot)
    removeTree(fake.dir)
  }
})

test('peer-present: advisory resolves the governed root for a subdirectory session cwd', async () => {
  const copyRoot = copyPackageRuntime()
  const projectRoot = createGovernedProject()
  try {
    const subdir = path.join(projectRoot, 'docs')
    const { mkdirSync } = await import('node:fs')
    mkdirSync(subdir, { recursive: true })
    writeStubPeer(copyRoot)
    const copied = await import(pathToFileURL(path.join(copyRoot, 'lib', 'index.js')).href)
    const { ctx, state } = createMockContext({ cwd: subdir })
    copied.apply(ctx, {})
    await flushAsync()

    const post = listenerFor(state, 'tools/post-execute')
    const result = await post(
      {
        name: 'write',
        arguments: { file_path: path.join(projectRoot, 'gov', 'adr', 'ADR-0001.toml') },
        agent: { session: { id: 's1' } },
      },
      { content: [] },
      () => Promise.resolve({ kind: 'accept' }),
    )
    assert.equal(result.kind, 'accept')
    assert.equal(result.additionalContexts.length, 1)
    assert.match(result.additionalContexts[0].content[0].text, /Target: gov\/adr\/ADR-0001\.toml$/)

    // Files outside the governed root stay untouched...
    const outside = await post(
      {
        name: 'write',
        arguments: { file_path: path.join(projectRoot, 'docs', 'guide.md') },
        agent: { session: { id: 's1' } },
      },
      { content: [] },
      () => Promise.resolve({ kind: 'accept' }),
    )
    assert.equal(outside.additionalContexts, undefined)
  } finally {
    removeTree(copyRoot)
    removeTree(projectRoot)
  }
})

test('peer-present: tools/post-execute attaches the edit advisory as sourced context', async () => {
  const copyRoot = copyPackageRuntime()
  const projectRoot = createGovernedProject()
  try {
    writeStubPeer(copyRoot)
    const copied = await import(pathToFileURL(path.join(copyRoot, 'lib', 'index.js')).href)
    const { ctx, state } = createMockContext({ cwd: projectRoot })
    copied.apply(ctx, {})
    await flushAsync()

    const post = listenerFor(state, 'tools/post-execute')
    const exec = {
      name: 'write',
      arguments: { file_path: path.join(projectRoot, 'gov', 'work', 'WI-1.toml') },
      agent: { session: { id: 's1' } },
    }
    const accept = { kind: 'accept' }
    const result = await post(exec, { content: [] }, () => Promise.resolve(accept))
    assert.equal(result.kind, 'accept')
    assert.equal(result.additionalContexts.length, 1)
    const advisory = result.additionalContexts[0]
    assert.equal(advisory.role, 'user')
    assert.deepEqual(advisory.source, { kind: 'plugin', plugin: 'govctl-dsh' })
    assert.match(advisory.content[0].text, /lifecycle-managed govctl artifact/)
    // The advisory must name the project-relative gov/ path exactly once.
    assert.match(advisory.content[0].text, /Target: gov\/work\/WI-1\.toml$/)

    // One advisory per target per session.
    const again = await post(exec, { content: [] }, () => Promise.resolve(accept))
    assert.equal(again, accept)
    assert.equal(again.additionalContexts, undefined)
  } finally {
    removeTree(copyRoot)
    removeTree(projectRoot)
  }
})
