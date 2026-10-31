/**
 * Native hook parity: session-start state delivery and the advisory-only
 * direct-edit notice.
 *
 * DSH 0.2.1-rc.5 has no Claude-Code hook bridge, so the plugin reimplements the
 * two govctl hook adapters on `agent/created` + `agent/pre-step` (session state)
 * and `tools/post-execute` (edit advisory).
 */
import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'

import { apply } from '../lib/index.js'
import { EDIT_ADVISORY } from '../lib/context.js'
import { findGovernedRoot, isGovernedProject, managedEditTarget } from '../lib/paths.js'
import {
  applyPlugin,
  capturedConsoleLines,
  createFakeGovctlScript,
  createGovernedProject,
  createUngovernedDir,
  listenerFor,
  removeTree,
  waitFor,
} from './helpers/mock-ctx.js'

/** A fake govctl that answers `agent hook session-start` with additionalContext. */
function fakeSessionStartBinary(contextText = 'Active work: WI-1') {
  return createFakeGovctlScript({
    stdout: JSON.stringify({ hookSpecificOutput: { hookEventName: 'SessionStart', additionalContext: contextText } }),
  })
}

/** Apply with a governed cwd and the fake binary wired to both resolve and exec paths. */
async function setupSession(options = {}) {
  const projectRoot = options.projectRoot ?? createGovernedProject()
  const fake = options.fake ?? fakeSessionStartBinary()
  const config = { binaryPath: fake.file }
  const mock = await applyPlugin(apply, {
    cwd: projectRoot,
    executable: fake.file,
    config,
  })
  return { ...mock, projectRoot, fake }
}

test('path predicates: governed detection walks ancestors like govctl', () => {
  const root = createGovernedProject()
  const ungoverned = createUngovernedDir()
  try {
    // The project root and every descendant (including sessions started in a
    // subdirectory, which is what govctl's own Config::discover_from supports).
    assert.equal(isGovernedProject(root), true)
    assert.equal(isGovernedProject(path.join(root, 'gov', 'rfc')), true)
    assert.equal(isGovernedProject(path.join(root, 'docs', 'deep', 'nested')), true)
    assert.equal(findGovernedRoot(path.join(root, 'docs', 'nested')), path.resolve(root))
    assert.equal(findGovernedRoot(root), path.resolve(root))
    // Outside a governed tree: undefined / false.
    assert.equal(isGovernedProject(ungoverned), false)
    assert.equal(isGovernedProject(path.join(ungoverned, 'nested')), false)
    assert.equal(findGovernedRoot(path.join(ungoverned, 'nested')), undefined)
    assert.equal(isGovernedProject(undefined), false)
    assert.equal(isGovernedProject(''), false)
    assert.equal(findGovernedRoot(undefined), undefined)
  } finally {
    removeTree(root)
    removeTree(ungoverned)
  }
})

test('managedEditTarget compares against the discovered governed root', () => {
  const root = '/project'
  const managed = [
    'gov/rfc/RFC-0002/rfc.toml',
    'gov/adr/ADR-0001.toml',
    'gov/work/WI-1.toml',
    'gov/guard/GUARD-1.toml',
    'gov/conformance/CONF-1.toml',
    'gov/releases.toml',
  ]
  for (const file of managed) {
    assert.equal(managedEditTarget(root, { name: 'edit', arguments: { file_path: file } }, root), file, file)
    assert.equal(managedEditTarget(root, { name: 'write', arguments: { file_path: file } }, root), file, file)
  }
  const unmanaged = ['gov/config.toml', 'src/main.rs', 'gov/releases.toml/extra', 'docs/guide/x.md']
  for (const file of unmanaged) {
    assert.equal(managedEditTarget(root, { name: 'edit', arguments: { file_path: file } }, root), undefined, file)
  }
  // Only edit/write carry the advisory contract.
  assert.equal(
    managedEditTarget(root, { name: 'read', arguments: { file_path: 'gov/rfc/RFC-0002/rfc.toml' } }, root),
    undefined,
  )
  // A session cwd in a subdirectory: a relative path that climbs back to the
  // project root is manageable ...
  assert.equal(
    managedEditTarget(
      path.join('/project', 'docs'),
      { name: 'edit', arguments: { file_path: path.join('..', 'gov', 'adr', 'ADR-0001.toml') } },
      '/project',
    ),
    path.join('gov', 'adr', 'ADR-0001.toml'),
  )
  // ... and so is an absolute path inside the project.
  assert.equal(
    managedEditTarget(
      path.join('/project', 'docs'),
      { name: 'edit', arguments: { file_path: '/project/gov/rfc/RFC-0002/rfc.toml' } },
      '/project',
    ),
    path.join('gov', 'rfc', 'RFC-0002', 'rfc.toml'),
  )
  // Without the discovered root, the same subdirectory-absolute case is not
  // project-relative (callers must pass the governed root).
  assert.equal(
    managedEditTarget(
      path.join('/project', 'docs'),
      { name: 'edit', arguments: { file_path: '/project/gov/rfc/RFC-0002/rfc.toml' } },
    ),
    undefined,
  )
  // Absolute paths outside the governed root are not managed.
  assert.equal(
    managedEditTarget('/project', { name: 'edit', arguments: { file_path: '/elsewhere/gov/rfc/x.toml' } }, '/project'),
    undefined,
  )
})

test('agent/created runs the session-start hook once for a governed session', async () => {
  const { state, projectRoot, fake } = await setupSession()
  try {
    const created = listenerFor(state, 'agent/created')
    // The plugin caches state per session object (stable in DSH; same reference in tests).
    const agent = { session: { id: 's1' } }
    created({ agent, source: 'startup' })
    await waitFor(() => existsSync(fake.capturePath))
    await waitFor(() => state.contexts[0].text({ agent }).length > 0)
    const payload = JSON.parse(readFileSync(fake.capturePath, 'utf8'))
    assert.deepEqual(payload, { cwd: projectRoot, tool_input: {} })
    // The cached state is delivered through the runtime prompt context without the peer.
    const context = state.contexts.find((entry) => entry.name === 'govctl:state')
    const text = context.text({ agent })
    // The fallback path contributes the raw hook context (the "govctl governance
    // state" banner is only used for the durable user-message delivery path).
    assert.match(text, /Active work: WI-1/)
    assert.equal(context.text({ agent: { session: { id: 'other' } } }), '')
  } finally {
    removeTree(projectRoot)
    removeTree(fake.dir)
  }
})

test('agent/created runs the session-start hook for a session cwd in a subdirectory', async () => {
  const projectRoot = createGovernedProject()
  const subdir = path.join(projectRoot, 'docs', 'nested')
  // mkdir the subdir so path semantics are realistic
  const { mkdirSync } = await import('node:fs')
  mkdirSync(subdir, { recursive: true })
  const fake = fakeSessionStartBinary()
  const { state } = await applyPlugin(apply, {
    cwd: subdir,
    executable: fake.file,
    config: { binaryPath: fake.file },
  })
  try {
    const agent = { session: { id: 's1' } }
    listenerFor(state, 'agent/created')({ agent })
    await waitFor(() => state.contexts[0].text({ agent }).length > 0)
    const payload = JSON.parse(readFileSync(fake.capturePath, 'utf8'))
    assert.equal(payload.cwd, subdir, 'hook receives the session cwd (govctl discovers ancestors itself)')
    assert.match(state.contexts[0].text({ agent }), /Active work: WI-1/)
  } finally {
    removeTree(projectRoot)
    removeTree(fake.dir)
  }
})

test('agent/created does not run the hook in an ungoverned workspace', async () => {
  // Must be a temp dir outside this repository: the repo itself is governed.
  const ungoverned = createUngovernedDir()
  const fake = fakeSessionStartBinary()
  const { state } = await applyPlugin(apply, {
    cwd: ungoverned,
    executable: fake.file,
    config: { binaryPath: fake.file },
  })
  try {
    listenerFor(state, 'agent/created')({ agent: { session: { id: 's1' } } })
    await new Promise((resolve) => setTimeout(resolve, 150))
    assert.equal(existsSync(fake.capturePath), false, 'hook must not run outside a governed project')
  } finally {
    removeTree(ungoverned)
    removeTree(fake.dir)
  }
})

test('session-start failures leave no state and never throw', async () => {
  const projectRoot = createGovernedProject()
  const failing = createFakeGovctlScript({ exitCode: 1 })
  const { state } = await applyPlugin(apply, {
    cwd: projectRoot,
    executable: failing.file,
    config: { binaryPath: failing.file },
  })
  try {
    const agent = { session: { id: 's1' } }
    // The failure is contained by the agent/created listener (serial dispatch would
    // otherwise reject agent registration).
    assert.doesNotThrow(() => listenerFor(state, 'agent/created')({ agent }))
    await new Promise((resolve) => setTimeout(resolve, 250))
    assert.equal(state.contexts[0].text({ agent }), '')
  } finally {
    removeTree(projectRoot)
    removeTree(failing.dir)
  }
})

test('agent/pre-step without @deepseek-ai/dsh-llm returns the next() decision unchanged', async () => {
  const fake = fakeSessionStartBinary()
  const { state } = await applyPlugin(apply, {
    cwd: createGovernedProject(),
    executable: fake.file,
    config: { binaryPath: fake.file },
  })
  try {
    const preStep = listenerFor(state, 'agent/pre-step')
    const claimed = { id: 'message-1', role: 'user', content: [{ type: 'text', text: 'hello' }] }
    const decision = { kind: 'enter', messages: [claimed] }
    const result = await preStep({ agent: { session: { id: 's1' } }, messages: [claimed], turn: 1, step: 1 }, () =>
      Promise.resolve(decision),
    )
    assert.equal(result, decision)
    assert.deepEqual(result.messages, [claimed])
  } finally {
    removeTree(fake.dir)
  }
})

test('agent/pre-step preserves a reject decision', async () => {
  const { state } = await applyPlugin(apply)
  const preStep = listenerFor(state, 'agent/pre-step')
  const reject = { kind: 'reject', reason: 'blocked' }
  const result = await preStep({ agent: { session: { id: 's1' } }, messages: [], turn: 1, step: 1 }, () =>
    Promise.resolve(reject),
  )
  assert.equal(result, reject)
})

test('tools/post-execute attaches no context without the peer but logs the direct edit', async () => {
  const projectRoot = createGovernedProject()
  const { state } = await applyPlugin(apply, { cwd: projectRoot })
  const before = capturedConsoleLines().length
  try {
    const post = listenerFor(state, 'tools/post-execute')
    const exec = {
      name: 'edit',
      arguments: { file_path: path.join(projectRoot, 'gov', 'rfc', 'RFC-0002', 'rfc.toml') },
      agent: { session: { id: 's1' } },
    }
    const decision = { kind: 'accept' }
    const result = await post(exec, { content: [] }, () => Promise.resolve(decision))
    assert.equal(result, decision)
    assert.equal(result.additionalContexts, undefined)
    assert.ok(
      capturedConsoleLines()
        .slice(before)
        .some((line) => line.includes('managed artifact edited directly')),
      `expected advisory fallback log, got: ${JSON.stringify(capturedConsoleLines().slice(before))}`,
    )
  } finally {
    removeTree(projectRoot)
  }
})

test('tools/post-execute advisory works for a session cwd in a subdirectory', async () => {
  const projectRoot = createGovernedProject()
  const subdir = path.join(projectRoot, 'docs')
  const { mkdirSync } = await import('node:fs')
  mkdirSync(subdir, { recursive: true })
  const { state } = await applyPlugin(apply, { cwd: subdir })
  const before = capturedConsoleLines().length
  try {
    const post = listenerFor(state, 'tools/post-execute')
    const result = await post(
      {
        name: 'write',
        arguments: { file_path: path.join(projectRoot, 'gov', 'work', 'WI-1.toml') },
        agent: { session: { id: 's1' } },
      },
      { content: [] },
      () => Promise.resolve({ kind: 'accept' }),
    )
    assert.equal(result.kind, 'accept')
    assert.ok(
      capturedConsoleLines()
        .slice(before)
        .some((line) =>
          line.includes('managed artifact edited directly: gov/work/WI-1.toml'),
        ),
      `expected ancestor-discovered advisory log, got: ${JSON.stringify(capturedConsoleLines().slice(before))}`,
    )
  } finally {
    removeTree(projectRoot)
  }
})

test('tools/post-execute ignores unmanaged edits and preserves block decisions', async () => {
  const projectRoot = createGovernedProject()
  const { state } = await applyPlugin(apply, { cwd: projectRoot })
  try {
    const post = listenerFor(state, 'tools/post-execute')
    const accept = { kind: 'accept' }
    const unmanaged = await post(
      { name: 'edit', arguments: { file_path: path.join(projectRoot, 'src', 'main.rs') }, agent: { session: { id: 's1' } } },
      { content: [] },
      () => Promise.resolve(accept),
    )
    assert.equal(unmanaged, accept)

    const block = { kind: 'block', feedback: [{ type: 'text', text: 'no' }] }
    const blocked = await post(
      { name: 'edit', arguments: { file_path: path.join(projectRoot, 'gov', 'adr', 'ADR-0001.toml') }, agent: { session: { id: 's1' } } },
      { content: [] },
      () => Promise.resolve(block),
    )
    assert.equal(blocked, block)
  } finally {
    removeTree(projectRoot)
  }
})

test('EDIT_ADVISORY is advisory wording per RFC-0002 (no deny/enforcement language)', () => {
  assert.match(EDIT_ADVISORY, /Prefer the resource-specific/)
  assert.match(EDIT_ADVISORY, /Direct editing is permitted/)
  assert.ok(!/must not|forbidden|deny/i.test(EDIT_ADVISORY))
})
