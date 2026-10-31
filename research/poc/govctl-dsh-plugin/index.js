/**
 * govctl — DeepSeek Harness bundle (research proof of concept).
 *
 * This is a dependency-free Cordis plugin. It is loaded by a bundle patch row
 * inserted in the root (global) layer, so every agent scope inherits what it
 * registers:
 *
 *   - govctl_* model tools that shell out to the `govctl` binary
 *   - a governance policy section in the system prompt
 *   - a per-session governance-state context rendered from `govctl agent hook session-start`
 *   - the shipped govctl skills, registered as embedded runtime skills
 *   - human slash commands (/gov-status, /gov-check)
 *
 * Evidence for the APIs used here is the installed DeepSeek Harness 0.2.1-rc.5
 * package set inside app.asar (dsh-tools, dsh-system-prompt, dsh-skill,
 * dsh-commands, dsh-agent, dsh-working-directory). See research/dsh-platform-notes.md.
 *
 * Deliberately zero runtime dependencies: it imports only Node builtins, so the
 * package needs no npm install beyond the bundle link itself and cannot drift
 * with @deepseek-ai/* package versions. A production plugin that wants richer
 * tool definitions can import `defineTool` from '@deepseek-ai/dsh-tools' and
 * declare a peer dependency.
 */
import { execFile } from 'node:child_process'
import { existsSync, readFileSync, readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

export const name = 'govctl'
export const inject = ['tools', 'systemPrompt', 'skills', 'commands', 'workingDirectory']

const HERE = path.dirname(fileURLToPath(import.meta.url))
const SKILLS_DIR = path.join(HERE, 'skills')

/** Lifecycle-managed paths that govctl reserves for its own commands. */
const MANAGED_DIRS = new Set(['rfc', 'adr', 'work', 'guard', 'conformance'])

const POLICY_SECTION = `## Governance mode (govctl)

This repository is governed by govctl. Its live governance state is summarized
under "govctl state" in the runtime context. Treat governed artifacts as the
source of truth, not chat history.

### Laws

1. RFC supremacy. A normative RFC outranks code, prompts, and convenience. If
   code and a normative RFC disagree, the code is the bug. Never implement
   against a draft RFC.
2. Phase discipline. \`spec → impl → test → stable\`. Do not skip phases and do
   not write implementation while an RFC is still in spec.
3. No silent deviation. If behavior is unspecified or ambiguous, stop and ask.
   Do not invent behavior.

### Hard stops

- Do not edit \`gov/\` artifacts directly. Use \`govctl rfc|adr|work|guard|clause\`
  edit commands (or \`govctl_run\`) so lifecycle-owned fields and signatures stay
  valid. Direct edits are permitted only when the current CLI has no suitable
  operation; then run \`govctl_check\` afterwards.
- Ask the user before any lifecycle or destructive operation (accept, reject,
  finalize, advance, bump, deprecate, supersede, delete).
- Substantive implementation work needs a matching active Work Item.

### Before substantive work

Call \`govctl_status\` first. Read the matching Work Item with \`govctl_show\`
(\`kind: "work"\`) and the governing RFCs/ADRs it references. Load the \`gov\`
skill for the full governed-implementation workflow, \`quick\` for a trivial
non-behavioral change, or \`spec\` when no code is needed.`

const GUARD_ADVISORY =
  'This edit targets a lifecycle-managed govctl artifact. Prefer the ' +
  'resource-specific `govctl ... edit` or lifecycle command when it can ' +
  'express the change. Direct editing is permitted when the current CLI has no ' +
  'suitable operation; preserve lifecycle-owned fields and run ' +
  '`govctl check` after the edit.'

/**
 * Run the govctl binary without a shell; resolves rather than throws.
 *
 * NOTE: production should use `ctx.subprocess.resolveExecutable` +
 * `ctx.subprocess.spawn` (env scrubbing, output caps, managed termination,
 * disposal) instead of node:child_process; see research/dsh-platform-notes.md §5.
 * Kept dependency-free here so the bundle needs no peer imports to prove the
 * packaging model.
 */
function runGovctl(binary, argv, options = {}) {
  const { cwd, input, timeoutMs = 20_000, signal } = options
  return new Promise((resolve) => {
    const child = execFile(
      binary,
      argv,
      { cwd, timeout: timeoutMs, signal, maxBuffer: 8 * 1024 * 1024, env: process.env },
      (error, stdout, stderr) => {
        const code = error && typeof error.code === 'number' ? error.code : error ? 1 : 0
        resolve({ code, stdout: String(stdout ?? ''), stderr: String(stderr ?? '') })
      },
    )
    if (input !== undefined && child.stdin !== null) {
      child.stdin.end(input)
    }
  })
}

function renderResult(result) {
  const stdout = result.stdout.trimEnd()
  const stderr = result.stderr.trimEnd()
  if (result.code === 0 && stderr.length === 0) return stdout
  const parts = []
  if (stdout.length > 0) parts.push(stdout)
  if (stderr.length > 0) parts.push(`[stderr]\n${stderr}`)
  if (parts.length === 0) parts.push(`[exit ${result.code}]`)
  return parts.join('\n\n')
}

/** Resolve the session working directory (falls back to the process cwd). */
function sessionCwd(ctx, agent) {
  try {
    if (agent !== undefined) return ctx.workingDirectory.get(agent.session)
  } catch {
    // fall through
  }
  return process.cwd()
}

function isGovernedProject(cwd) {
  return existsSync(path.join(cwd, 'gov', 'config.toml'))
}

/** Extract `name` and `description` from a SKILL.md frontmatter block. */
function parseSkillBundle(raw, fallbackName) {
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(raw)
  const frontmatter = match ? match[1] : ''
  const content = match ? raw.slice(match[0].length).trim() : raw.trim()
  const field = (key) => {
    const line = new RegExp(`^${key}:\\s*(.+)$`, 'm').exec(frontmatter)
    if (!line) return undefined
    return line[1].trim().replace(/^"(.*)"$/, '$1').replace(/^'(.*)'$/, '$1')
  }
  const skillName = field('name') ?? fallbackName
  const description = field('description') ?? `govctl skill: ${fallbackName}`
  return { name: skillName, description, content }
}

function loadEmbeddedSkills(ctx, log) {
  let entries
  try {
    entries = readdirSync(SKILLS_DIR, { withFileTypes: true })
  } catch (error) {
    log(`skills directory unavailable: ${error.message}`)
    return
  }
  for (const entry of entries) {
    if (!entry.isDirectory()) continue
    const file = path.join(SKILLS_DIR, entry.name, 'SKILL.md')
    if (!existsSync(file)) continue
    try {
      const skill = parseSkillBundle(readFileSync(file, 'utf8'), entry.name)
      ctx.skills.register(skill)
      log(`registered skill "${skill.name}"`)
    } catch (error) {
      log(`skipped skill ${entry.name}: ${error.message}`)
    }
  }
}

/** Collect a govctl hook advisory (additionalContext) for one payload. */
async function collectHookContext(binary, event, cwd) {
  const result = await runGovctl(binary, ['agent', 'hook', event], {
    cwd,
    input: JSON.stringify({ cwd, tool_input: {} }),
    timeoutMs: 5_000,
  })
  if (result.code !== 0) return undefined
  try {
    const parsed = JSON.parse(result.stdout)
    const context = parsed?.hookSpecificOutput?.additionalContext
    return typeof context === 'string' && context.length > 0 ? context : undefined
  } catch {
    const text = result.stdout.trim()
    return text.length > 0 ? text : undefined
  }
}

export function apply(ctx, config = {}) {
  const binary = config.binaryPath ?? 'govctl'
  const log = (message) => console.log(`[govctl-dsh-plugin] ${message}`)

  // Per-session governance state, rendered by the system prompt context below.
  const stateBySession = new WeakMap()

  ctx.effect(() => {
    log(`loaded (binary: ${binary})`)
    return () => log('disposed')
  })

  // -------------------------------------------------------------------------
  // Model tools
  // -------------------------------------------------------------------------
  const textOutput = {
    schema: { type: 'string' },
    render: (_args, value) => [{ type: 'text', text: value }],
  }

  const tool = (definition) => ctx.tools.register({ ...definition, output: textOutput })

  tool({
    name: 'govctl_status',
    description:
      'Show the govctl project overview: config, artifact counts, active work items and open loops.',
    parameters: { type: 'object', properties: {}, additionalProperties: false },
    async execute(_args, exec) {
      const result = await runGovctl(binary, ['status'], {
        cwd: sessionCwd(ctx, exec.agent),
        signal: exec.signal,
      })
      return renderResult(result)
    },
  })

  tool({
    name: 'govctl_check',
    description:
      'Validate every governed artifact and rendered projection in the repository, returning diagnostics.',
    parameters: { type: 'object', properties: {}, additionalProperties: false },
    async execute(_args, exec) {
      const result = await runGovctl(binary, ['check'], {
        cwd: sessionCwd(ctx, exec.agent),
        signal: exec.signal,
      })
      return renderResult(result)
    },
  })

  tool({
    name: 'govctl_search',
    description: 'Search all governed artifacts for a query string.',
    parameters: {
      type: 'object',
      properties: {
        query: { type: 'string', description: 'Text to search for.' },
      },
      required: ['query'],
      additionalProperties: false,
    },
    async execute(args, exec) {
      const result = await runGovctl(binary, ['search', String(args.query ?? '')], {
        cwd: sessionCwd(ctx, exec.agent),
        signal: exec.signal,
      })
      return renderResult(result)
    },
  })

  tool({
    name: 'govctl_show',
    description:
      'Show one governed artifact. kind is rfc, adr, work, guard, conformance, or clause; id is e.g. RFC-0002, ADR-0061, WI-ID, or RFC-0001:C-CLAUSE.',
    parameters: {
      type: 'object',
      properties: {
        kind: {
          type: 'string',
          enum: ['rfc', 'adr', 'work', 'guard', 'conformance', 'clause'],
          description: 'Artifact kind.',
        },
        id: { type: 'string', description: 'Artifact identifier.' },
      },
      required: ['kind', 'id'],
      additionalProperties: false,
    },
    async execute(args, exec) {
      const result = await runGovctl(binary, [String(args.kind), 'show', String(args.id)], {
        cwd: sessionCwd(ctx, exec.agent),
        signal: exec.signal,
      })
      return renderResult(result)
    },
  })

  tool({
    name: 'govctl_run',
    description:
      'Run an arbitrary govctl subcommand (argv array, no shell). Use for mutating or lifecycle commands that govctl_status/check/show do not cover. Only run mutations the user has authorized.',
    parameters: {
      type: 'object',
      properties: {
        argv: {
          type: 'array',
          items: { type: 'string' },
          description: 'Arguments after the govctl binary, e.g. ["work","list","active"].',
        },
      },
      required: ['argv'],
      additionalProperties: false,
    },
    async execute(args, exec) {
      const argv = Array.isArray(args.argv) ? args.argv.map(String) : []
      if (argv.length === 0) return 'govctl_run requires a non-empty argv array'
      const result = await runGovctl(binary, argv, {
        cwd: sessionCwd(ctx, exec.agent),
        signal: exec.signal,
      })
      return renderResult(result)
    },
  })

  // -------------------------------------------------------------------------
  // System prompt: governance policy + per-session state
  // -------------------------------------------------------------------------
  if (config.enablePolicySection !== false) {
    ctx.systemPrompt.section({
      name: 'policy:govctl',
      // 700: after plan/team policy (500/600), before per-tool guidance.
      order: 700,
      text: (context) => {
        if (context?.agent === undefined) return ''
        return isGovernedProject(sessionCwd(ctx, context.agent)) ? POLICY_SECTION : ''
      },
    })
  }

  if (config.enableSessionState !== false) {
    const refreshBySession = new WeakMap()

    // `agent/created` is serial and awaited before creation resolves, so the
    // subprocess must not be awaited here (it would delay every new session).
    const scheduleSessionState = (agent) => {
      const pending = refreshBySession.get(agent.session)
      if (pending !== undefined) return pending
      const cwd = sessionCwd(ctx, agent)
      if (!isGovernedProject(cwd)) return Promise.resolve()
      const task = collectHookContext(binary, 'session-start', cwd)
        .then((text) => {
          if (text !== undefined) stateBySession.set(agent.session, text)
        })
        .catch((error) => log(`session state unavailable: ${error.message}`))
      refreshBySession.set(agent.session, task)
      return task
    }

    ctx.on('agent/created', ({ agent }) => {
      void scheduleSessionState(agent)
    })

    // Give the first step a bounded chance to see the state before it is read.
    // A production plugin instead splices a sourced `createUserMessage` into the
    // pre-step decision (the `dsh-agent-instructions` pattern), which requires a
    // peer dependency on `@deepseek-ai/dsh-llm`.
    ctx.on('agent/pre-step', async (payload, next) => {
      const pending = refreshBySession.get(payload.agent.session)
      if (pending !== undefined) {
        await Promise.race([pending, new Promise((resolve) => setTimeout(resolve, 1_500))])
      }
      return next()
    })

    ctx.systemPrompt.context({
      name: 'govctl:state',
      // 130: after working directory (100) and sandbox/approval (110/115).
      order: 130,
      text: (context) => {
        if (context?.agent === undefined) return ''
        return stateBySession.get(context.agent.session) ?? ''
      },
    })
  }

  // -------------------------------------------------------------------------
  // Skills and slash commands
  // -------------------------------------------------------------------------
  loadEmbeddedSkills(ctx, log)

  const command = (name, description, argvOf) =>
    ctx.commands.register({
      name,
      description,
      handler: async ({ agent, rawInput }) => {
        const cwd = sessionCwd(ctx, agent)
        if (!isGovernedProject(cwd)) {
          return { kind: 'error', text: 'This workspace is not governed by govctl.' }
        }
        const result = await runGovctl(binary, argvOf(rawInput.trim()), { cwd })
        return result.code === 0
          ? { kind: 'success', text: renderResult(result) }
          : { kind: 'error', text: renderResult(result) }
      },
    })

  command('gov-status', 'Show the govctl project overview', () => ['status'])
  command('gov-check', 'Validate governed artifacts', () => ['check'])

  // -------------------------------------------------------------------------
  // Approval gate and managed-edit advisory
  // -------------------------------------------------------------------------
  // govctl_run is an escape hatch that can reach mutating and lifecycle verbs.
  // `tools/pre-execute` `ask` routes to the registry's approval flow and fails
  // closed when no approval channel is available.
  ctx.on('tools/pre-execute', async (exec, next) => {
    if (exec.name === 'govctl_run') {
      return {
        kind: 'ask',
        reason:
          'govctl_run can mutate governed artifacts and lifecycle state; explicit user approval is required.',
      }
    }
    return next()
  })

  if (config.enableGuardAdvisory !== false) {
    // Advisory only — never deny: RFC-0002 requires hook-style advice to stay a
    // recoverability aid, not an enforcement boundary. Attaching the advisory as
    // model-visible context needs `tools/post-execute` `additionalContexts`
    // plus a `createUserMessage` factory from '@deepseek-ai/dsh-llm'; this
    // zero-dependency PoC only records the detection.
    ctx.on('tools/post-execute', async (exec, _result, next) => {
      const target = managedEditTarget(ctx, exec)
      if (target !== undefined) log(`managed artifact edited directly: ${target} — ${GUARD_ADVISORY}`)
      return next()
    })
  }
}

/** True when a write/edit call targets a lifecycle-managed govctl path. */
function managedEditTarget(ctx, exec) {
  if (exec.name !== 'edit' && exec.name !== 'write') return undefined
  const filePath = exec.arguments?.file_path
  if (typeof filePath !== 'string') return undefined
  const cwd = sessionCwd(ctx, exec.agent)
  const absolute = path.isAbsolute(filePath) ? filePath : path.join(cwd, filePath)
  const relative = path.relative(cwd, absolute)
  if (relative.startsWith('..') || path.isAbsolute(relative)) return undefined
  const [first, second, third] = relative.split(path.sep)
  if (first !== 'gov') return undefined
  if (second === 'releases.toml') return third === undefined ? relative : undefined
  return MANAGED_DIRS.has(second) ? relative : undefined
}
