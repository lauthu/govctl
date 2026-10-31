/**
 * Shared test harness for govctl-dsh.
 *
 * Builds a minimal mock of the Cordis context surface used by the plugin
 * (`tools`, `systemPrompt`, `skills`, `commands`, `on`, `effect`,
 * `workingDirectory`, `subprocess`, `logger`) and helpers for driving the
 * registered tools / listeners. Zero dependencies; the mock follows the
 * installed DSH 0.2.1-rc.5 contracts (raw tool definitions, waterfall
 * listeners, `ctx.subprocess` spawn shape, collect readers).
 */
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

// The plugin emits its load banner with a direct console.log (a headless boot
// signal, also captured by scripts/validate-with-dsh.sh). Keep that line out of
// the test reporter output but make it assertable.
const capturedConsole = []
const nativeConsoleLog = console.log
console.log = (...args) => {
  const line = args.map(String).join(' ')
  if (line.startsWith('[govctl-dsh]')) {
    capturedConsole.push(line)
    return
  }
  nativeConsoleLog(...args)
}

/** Lines the plugin wrote straight to stdout (e.g. the load banner). */
export function capturedConsoleLines() {
  return [...capturedConsole]
}

/** Resolve after pending microtasks/timers so async plugin side effects settle. */
export async function flushAsync(rounds = 3) {
  for (let index = 0; index < rounds; index += 1) {
    await new Promise((resolve) => setImmediate(resolve))
  }
}

/** Collect reader shape from @deepseek-ai/dsh-subprocess-local (`readFrom` → {text}). */
export function fakeStreamReader(text) {
  return {
    readFrom(offset) {
      const from = typeof offset === 'number' && offset > 0 ? offset : 0
      return { text: text.slice(from), nextOffset: text.length, lossy: false }
    },
  }
}

/** Fake `ctx.subprocess.spawn` handle: `done` + collected stdout/stderr. */
export function createFakeHandle(spec = {}) {
  const stdout = spec.stdout ?? ''
  const stderr = spec.stderr ?? ''
  return {
    done:
      spec.doneError !== undefined
        ? Promise.reject(spec.doneError)
        : Promise.resolve({ exitCode: spec.exitCode ?? 0, signal: spec.signal ?? null }),
    collected: {
      stdout: fakeStreamReader(stdout),
      stderr: fakeStreamReader(stderr),
    },
    terminate() {},
    waitForExit() {
      return Promise.resolve({})
    },
  }
}

/**
 * Create a mock context plus captured registrations.
 *
 * @param {object} [options]
 * @param {string} [options.cwd] committed working directory returned by `get`/`ensure`
 * @param {object} [options.spawnResult] stdout/stderr/exitCode for spawned govctl calls
 * @param {(name: string) => Promise<string>} [options.resolveExecutable] executable lookup override
 * @param {object} [options.logger] logger override
 */
export function createMockContext(options = {}) {
  const cwd = options.cwd ?? path.join(os.tmpdir(), 'govctl-dsh-mock-cwd')
  const state = {
    tools: [],
    sections: [],
    contexts: [],
    variables: [],
    skills: [],
    commands: [],
    listeners: [],
    effects: [],
    spawns: [],
    resolveExecutableCalls: [],
    logLines: { info: [], warn: [] },
  }

  const logger = options.logger ?? {
    info(message) {
      state.logLines.info.push(String(message))
    },
    warn(message) {
      state.logLines.warn.push(String(message))
    },
    debug() {},
    error() {},
  }

  const ctx = {
    logger,
    tools: {
      register(definition) {
        state.tools.push(definition)
        return () => {}
      },
    },
    systemPrompt: {
      section(section) {
        state.sections.push(section)
        return () => {}
      },
      context(context) {
        state.contexts.push(context)
        return () => {}
      },
      variable(name, provider) {
        state.variables.push({ name, provider })
        return () => {}
      },
    },
    skills: {
      register(skill) {
        state.skills.push(skill)
        return () => {}
      },
    },
    commands: {
      register(command) {
        state.commands.push(command)
        return () => {}
      },
    },
    on(event, listener) {
      state.listeners.push({ event, listener })
      return () => {}
    },
    effect(callback) {
      state.effects.push(callback)
      const disposer = callback()
      return () => {
        if (typeof disposer === 'function') disposer()
      }
    },
    workingDirectory: {
      get(session) {
        return options.cwdFor?.(session) ?? cwd
      },
      async ensure(agent) {
        if (options.ensureError !== undefined) throw options.ensureError
        return options.cwdFor?.(agent?.session) ?? cwd
      },
    },
    subprocess: {
      resolveExecutable(name) {
        state.resolveExecutableCalls.push(name)
        const resolver = options.resolveExecutable
        if (resolver !== undefined) return Promise.resolve(resolver(name))
        return Promise.resolve(options.executable ?? '/fake/govctl')
      },
      spawn(request) {
        state.spawns.push(request)
        return createFakeHandle(options.spawnResult)
      },
    },
  }

  return { ctx, state }
}

/** Run `apply(ctx, config)` against a fresh mock context and settle async work. */
export async function applyPlugin(apply, options = {}) {
  const mock = createMockContext(options)
  apply(mock.ctx, options.config)
  await flushAsync()
  return mock
}

/** The single registered listener for an event (assumes at most one). */
export function listenerFor(state, event) {
  const matches = state.listeners.filter((entry) => entry.event === event)
  if (matches.length === 0) return undefined
  return matches[matches.length - 1].listener
}

/** All listeners registered for an event. */
export function listenersFor(state, event) {
  return state.listeners.filter((entry) => entry.event === event).map((entry) => entry.listener)
}

/** Spawn requests that are real govctl invocations (the `--version` probe excluded). */
export function govctlSpawns(state) {
  return state.spawns.filter((request) => request.argv[1] !== '--version')
}

/** argv (without the executable) of the most recent real govctl spawn. */
export function lastGovctlArgv(state) {
  const spawns = govctlSpawns(state)
  const last = spawns[spawns.length - 1]
  return last === undefined ? undefined : last.argv.slice(1)
}

/** Minimal tool execution context accepted by the registered `execute(args, exec)`. */
export function execContext(overrides = {}) {
  return { agent: { session: { id: 'session-1' } }, signal: undefined, ...overrides }
}

/** Find a registered tool definition by name and execute it. */
export async function callTool(state, name, args, exec = execContext()) {
  const tool = state.tools.find((definition) => definition.name === name)
  if (tool === undefined) throw new Error(`tool not registered: ${name}`)
  return tool.execute(args, exec)
}

/** Create a temporary directory OUTSIDE any governed tree (repo root is governed). */
export function createUngovernedDir() {
  return mkdtempSync(path.join(os.tmpdir(), 'govctl-dsh-ungoverned-'))
}

/** Create a temporary governed project (gov/config.toml) for path predicates. */
export function createGovernedProject({ configContents = 'name = "test-project"\n' } = {}) {
  const root = mkdtempSync(path.join(os.tmpdir(), 'govctl-dsh-project-'))
  mkdirSync(path.join(root, 'gov'), { recursive: true })
  writeFileSync(path.join(root, 'gov', 'config.toml'), configContents, 'utf8')
  return root
}

/**
 * Write an executable shell script that stands in for the govctl binary on the
 * `node:child_process` path (used by the hook adapters, which need stdin).
 * The script records the stdin payload to `capturePath` and prints `stdout`.
 */
export function createFakeGovctlScript({ capturePath, envCapturePath, stdout = '', exitCode = 0 } = {}) {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'govctl-dsh-bin-'))
  const file = path.join(dir, 'govctl')
  const capture = capturePath ?? path.join(dir, 'stdin.json')
  const envCapture = envCapturePath ?? path.join(dir, 'env.txt')
  const script = ['#!/bin/sh']
  if (capture !== undefined) script.push(`cat > '${capture.replaceAll("'", "'\\''")}'`)
  if (envCapture !== undefined) script.push(`env > '${envCapture.replaceAll("'", "'\\''")}'`)
  if (stdout.length > 0) script.push(`printf '%s' '${stdout.replaceAll("'", "'\\''")}'`)
  script.push(`exit ${exitCode}`)
  writeFileSync(file, `${script.join('\n')}\n`, 'utf8')
  chmodSync(file, 0o755)
  return { file, dir, capturePath: capture, envCapturePath: envCapture }
}

/** Remove a temp directory tree, ignoring absence. */
export function removeTree(directory) {
  rmSync(directory, { recursive: true, force: true })
}

/** Poll until `predicate()` is truthy (handles child-process completion). */
export async function waitFor(predicate, { timeoutMs = 10_000, intervalMs = 10 } = {}) {
  const deadline = Date.now() + timeoutMs
  for (;;) {
    // eslint-disable-next-line no-await-in-loop
    if (await predicate()) return true
    if (Date.now() >= deadline) throw new Error('waitFor: condition not met before timeout')
    // eslint-disable-next-line no-await-in-loop
    await new Promise((resolve) => setTimeout(resolve, intervalMs))
  }
}
