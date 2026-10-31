/**
 * govctl binary discovery and invocation.
 *
 * Design notes (see research/dsh-platform-notes.md §5 in the research branch):
 * - Tool commands spawn through `ctx.subprocess`: explicit argv, bounded output,
 *   env scrubbing, managed termination. No shell interpretation.
 * - The two hook adapters need stdin. `ctx.subprocess` has no documented stdin
 *   channel in DSH 0.2.1-rc.5, so those calls use node:child_process with a
 *   single write-and-close. Hooks are read-only by contract.
 * - The plugin never rebuilds govctl behavior; it only transports CLI calls.
 */
import { execFile } from 'node:child_process'

/** The govctl release line this plugin is developed and tested against. */
const TESTED = { major: 0, minor: 22 }

const NOT_INSTALLED_HINT =
  'Install govctl from https://github.com/govctl-org/govctl/releases ' +
  '(or `cargo binstall govctl`), make sure it is on PATH, or set the ' +
  'plugin config `binaryPath` to an absolute path.'

function truncate(text, maxBytes) {
  if (Buffer.byteLength(text, 'utf8') <= maxBytes) return text
  const total = Buffer.byteLength(text, 'utf8')
  // Binary-search the largest character prefix that fits the byte budget.
  let low = 0
  let high = text.length
  while (low < high) {
    const mid = Math.ceil((low + high) / 2)
    if (Buffer.byteLength(text.slice(0, mid), 'utf8') <= maxBytes) low = mid
    else high = mid - 1
  }
  let keep = low
  const suffix = (dropped) => `\n… [truncated ${dropped} bytes]`
  while (keep > 0 && Buffer.byteLength(text.slice(0, keep) + suffix(total), 'utf8') > maxBytes) keep -= 1
  if (keep <= 0) return suffix(total)
  return text.slice(0, keep) + suffix(total)
}

function collectedText(handle, stream) {
  const reader = handle?.collected?.[stream]
  if (reader === undefined || typeof reader.readFrom !== 'function') return ''
  const chunk = reader.readFrom(0)
  if (typeof chunk === 'string') return chunk
  return typeof chunk?.text === 'string' ? chunk.text : ''
}

/**
 * Ambient credentials and harness facts never reach a child by default.
 * Mirrors the harness subprocess policy word set (KEY/PASSWORD/SECRET/TOKEN)
 * plus the DSH_ namespace; generic "AUTH" is deliberately not matched.
 */
const CREDENTIAL_SHAPED = /(KEY|PASSWORD|SECRET|TOKEN)/i

function scrubbedChildEnv() {
  const env = { ...process.env }
  for (const key of Object.keys(env)) {
    if (key.startsWith('DSH_') || CREDENTIAL_SHAPED.test(key)) delete env[key]
  }
  return env
}

function execFileAsync(file, argv, options) {
  return new Promise((resolve) => {
    const child = execFile(
      file,
      argv,
      {
        cwd: options.cwd,
        env: scrubbedChildEnv(),
        timeout: options.timeoutMs,
        maxBuffer: options.maxOutputBytes * 2,
        signal: options.signal,
      },
      (error, stdout, stderr) => {
        const code = error && typeof error.code === 'number' ? error.code : error ? 1 : 0
        resolve({
          code,
          stdout: String(stdout ?? ''),
          stderr: String(stderr ?? ''),
          ...(error?.killed === true ? { timedOut: true } : {}),
        })
      },
    )
    if (options.stdin !== undefined && child.stdin !== null) child.stdin.end(options.stdin)
  })
}

/**
 * Create the per-plugin runner. One runner per plugin instance; it caches the
 * resolved binary and the reported govctl version.
 */
export function createRunner(ctx, config) {
  const binaryCandidates = [config.binaryPath]
  if (config.binaryPath !== 'govctl') binaryCandidates.push('govctl')

  let binaryPromise
  let versionPromise

  const log = (message) => {
    // Single stdout channel, matching the load banner: ctx.logger output was not
    // observably reaching a headless boot, so compatibility/diagnostic warnings
    // would otherwise be invisible.
    console.log(`[govctl-dsh] ${message}`)
  }

  async function resolveBinary() {
    if (binaryPromise !== undefined) return binaryPromise
    binaryPromise = (async () => {
      for (const candidate of binaryCandidates) {
        try {
          if (ctx.subprocess === undefined) throw new Error('no subprocess service')
          return await ctx.subprocess.resolveExecutable(candidate)
        } catch {
          // try the next candidate
        }
      }
      throw new Error(`govctl binary not found (tried: ${binaryCandidates.join(', ')}). ${NOT_INSTALLED_HINT}`)
    })().catch((error) => {
      binaryPromise = undefined
      throw error
    })
    return binaryPromise
  }

  async function resolveCwd(agent, signal) {
    if (agent !== undefined) {
      try {
        return await ctx.workingDirectory.ensure(agent, signal)
      } catch {
        // fall through to the committed projection
      }
      try {
        return ctx.workingDirectory.get(agent.session)
      } catch {
        // fall through
      }
    }
    return process.cwd()
  }

  /** Run govctl with an explicit argv; resolves with exit/output facts. */
  async function runRaw(argv, options = {}) {
    const timeoutMs = options.timeoutMs ?? config.toolTimeoutMs
    const maxOutputBytes = options.maxOutputBytes ?? config.maxOutputBytes
    const cwd = options.cwd ?? process.cwd()
    let executable
    try {
      executable = await resolveBinary()
    } catch (error) {
      return { code: 1, stdout: '', stderr: String(error.message ?? error), unavailable: true }
    }

    // stdin is only needed by `agent hook <event>`; node child_process owns that
    // write-and-close (see the module comment).
    if (options.stdin !== undefined || ctx.subprocess === undefined) {
      return execFileAsync(executable, argv, {
        cwd,
        timeoutMs,
        maxOutputBytes,
        signal: options.signal,
        stdin: options.stdin,
      })
    }

    try {
      const handle = ctx.subprocess.spawn({
        argv: [executable, ...argv],
        cwd,
        stdio: {
          stdin: 'ignore',
          stdout: { maxBytes: maxOutputBytes },
          stderr: { maxBytes: maxOutputBytes },
        },
        graceMs: 2_000,
        signal: options.signal,
      })
      const outcome = await handle.done
      const stdout = collectedText(handle, 'stdout')
      const stderr = collectedText(handle, 'stderr')
      return { code: outcome?.exitCode ?? 1, stdout, stderr }
    } catch (error) {
      return { code: 1, stdout: '', stderr: String(error?.message ?? error) }
    }
  }

  /** Run govctl for one tool call in the session working directory. */
  async function run(exec, argv) {
    const cwd = await resolveCwd(exec?.agent, exec?.signal)
    return runRaw(argv, { cwd, signal: exec?.signal })
  }

  /** Run a govctl binary probe (e.g. `--version`) outside a session. */
  async function probe(argv, cwd) {
    return runRaw(argv, { cwd, timeoutMs: 5_000, maxOutputBytes: 8_192 })
  }

  /**
   * Run one bundled hook adapter and return its additional context.
   * stdin payload: { cwd, tool_input } (RFC-0002:C-AGENT-INTEGRATION contract).
   */
  async function hook(event, cwd, toolInput = {}) {
    const result = await runRaw(['agent', 'hook', event], {
      cwd,
      timeoutMs: 5_000,
      stdin: JSON.stringify({ cwd, tool_input: toolInput }),
    })
    if (result.code !== 0) {
      const detail = result.stderr.trim().split('\n').slice(0, 3).join(' | ')
      log(`agent hook ${event} exited with code ${result.code}${detail.length > 0 ? `: ${detail}` : ''}`)
      return undefined
    }
    try {
      const parsed = JSON.parse(result.stdout)
      const context = parsed?.hookSpecificOutput?.additionalContext
      if (typeof context === 'string' && context.length > 0) return context
      return undefined
    } catch {
      const text = result.stdout.trim()
      return text.length > 0 ? text : undefined
    }
  }

  async function version() {
    if (versionPromise !== undefined) return versionPromise
    versionPromise = probe(['--version'], process.cwd()).then((result) => {
      if (result.code !== 0) throw new Error(result.stderr || 'govctl --version failed')
      const match = /(\d+)\.(\d+)\.(\d+)(?:[-+][0-9A-Za-z.-]+)?/.exec(result.stdout)
      if (match === null) throw new Error(`unrecognized govctl version output: ${result.stdout.trim()}`)
      return { raw: match[0], major: Number(match[1]), minor: Number(match[2]), patch: Number(match[3]) }
    })
    return versionPromise
  }

  /** Warn (do not fail) when the installed binary is outside the tested line. */
  async function checkCompatibility() {
    try {
      const found = await version()
      if (found.major !== TESTED.major || found.minor !== TESTED.minor) {
        log(
          `govctl ${found.raw} is outside the tested ${TESTED.major}.${TESTED.minor}.x range; ` +
            'governance behavior may differ. Update govctl-dsh or pin the binary.',
        )
      }
      return found
    } catch {
      return undefined
    }
  }

  return { config, run, runRaw, probe, hook, version, checkCompatibility }
}

/** Format a govctl result for model consumption. */
export function renderResult(result, maxOutputBytes = 65_536) {
  const stdout = truncate(result.stdout.trimEnd(), maxOutputBytes)
  const stderr = truncate(result.stderr.trimEnd(), maxOutputBytes)
  if (result.code === 0) {
    return stdout.length > 0 ? stdout : stderr
  }
  const parts = [`govctl exited with code ${result.code}`]
  if (stdout.length > 0) parts.push(stdout)
  if (stderr.length > 0) parts.push(`[stderr]\n${stderr}`)
  return parts.join('\n\n')
}
