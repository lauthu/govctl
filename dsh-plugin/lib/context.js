/**
 * Session-start governance context and lifecycle-managed edit advisory.
 *
 * DSH 0.2.1-rc.5 has no Claude-Code hook bridge and no `agent/session-start`
 * event, so the two bundled govctl hook adapters are reimplemented natively:
 *
 *  - SessionStart: at `agent/created`, run `govctl agent hook session-start`
 *    (read-only) and inject its bounded summary. Two delivery paths:
 *      1. durable sourced message via `createUserMessage` (optional peer
 *         dependency on @deepseek-ai/dsh-llm) spliced at `agent/pre-step`, the
 *         same pattern dsh-agent-instructions uses;
 *      2. fallback: a runtime prompt context, used when the peer is absent.
 *  - PreToolUse: after a successful `edit`/`write` on a lifecycle-managed gov/
 *    artifact, attach the fixed advisory through `tools/post-execute`
 *    `additionalContexts`. Advisory only, never a deny — RFC-0002 requires this
 *    guidance to stay a recoverability aid, not an enforcement boundary.
 */
import { findGovernedRoot, isGovernedProject, managedEditTarget } from './paths.js'

const PLUGIN_SOURCE = { kind: 'plugin', plugin: 'govctl-dsh' }
const REFRESH_WAIT_MS = 1_500

export const EDIT_ADVISORY =
  'This edit targeted a lifecycle-managed govctl artifact. Prefer the ' +
  'resource-specific `govctl ... edit` or lifecycle command when it can express ' +
  'the change. Direct editing is permitted when the current CLI has no suitable ' +
  'operation; preserve lifecycle-owned fields and run `govctl_check` after the ' +
  'edit.'

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function promptContextText(state) {
  return `govctl governance state (session start):\n\n${state}`
}

export function registerGovernanceContext(ctx, runner, config, log) {
  const stateBySession = new WeakMap()
  const refreshBySession = new WeakMap()
  const injectedSessions = new WeakSet()
  const advisoryBySession = new WeakMap()

  let factoryPromise
  /** Lazily load the optional UserMessage factory (peer dependency). */
  function messageFactory() {
    if (factoryPromise === undefined) {
      factoryPromise = import('@deepseek-ai/dsh-llm')
        .then((mod) => (typeof mod.createUserMessage === 'function' ? mod.createUserMessage : undefined))
        .catch(() => undefined)
    }
    return factoryPromise
  }

  function sessionCwd(agent) {
    try {
      return ctx.workingDirectory.get(agent.session)
    } catch {
      return undefined
    }
  }

  function scheduleSessionState(agent) {
    const pending = refreshBySession.get(agent.session)
    if (pending !== undefined) return pending
    const cwd = sessionCwd(agent)
    if (!isGovernedProject(cwd)) return Promise.resolve()
    const task = runner
      .hook('session-start', cwd)
      .then((state) => {
        if (state !== undefined) stateBySession.set(agent.session, state)
      })
      .catch((error) => log(`session-start hook failed: ${error.message ?? error}`))
    refreshBySession.set(agent.session, task)
    return task
  }

  if (config.enableSessionContext !== false) {
    // Creation is serial and awaited by DSH, so do not await the hook here.
    ctx.on('agent/created', ({ agent }) => {
      void scheduleSessionState(agent)
    })

    ctx.on('agent/pre-step', async (payload, next) => {
      const decision = await next()
      try {
        if (decision?.kind === 'reject') return decision
        const agent = payload.agent
        const session = agent.session
        if (injectedSessions.has(session)) return decision
        const pending = refreshBySession.get(session)
        if (pending !== undefined) await Promise.race([pending, delay(REFRESH_WAIT_MS)])
        const state = stateBySession.get(session)
        if (state === undefined) return decision
        const createUserMessage = await messageFactory()
        if (createUserMessage === undefined) return decision
        const claimed = payload.messages ?? []
        const lastClaimed = decision.messages.findLastIndex((message) => claimed.includes(message))
        if (lastClaimed < 0) return decision
        const message = createUserMessage({
          content: [{ type: 'text', text: promptContextText(state) }],
          source: PLUGIN_SOURCE,
        })
        injectedSessions.add(session)
        return {
          ...decision,
          messages: decision.messages.toSpliced(lastClaimed + 1, 0, message),
        }
      } catch (error) {
        log(`session context injection skipped: ${error.message ?? error}`)
        return decision
      }
    })

    // Fallback delivery when @deepseek-ai/dsh-llm is not installed.
    ctx.systemPrompt.context({
      name: 'govctl:state',
      order: 130,
      text: (context) => {
        const agent = context?.agent
        if (agent === undefined) return ''
        if (injectedSessions.has(agent.session)) return ''
        return stateBySession.get(agent.session) ?? ''
      },
    })
  }

  if (config.enableEditAdvisory !== false) {
    ctx.on('tools/post-execute', async (exec, _result, next) => {
      const decision = await next()
      try {
        if (decision?.kind === 'block') return decision
        const agent = exec.agent
        if (agent === undefined) return decision
        const cwd = sessionCwd(agent)
        const governedRoot = findGovernedRoot(cwd)
        if (governedRoot === undefined) return decision
        const target = managedEditTarget(cwd, exec, governedRoot)
        if (target === undefined) return decision
        let sent = advisoryBySession.get(agent.session)
        if (sent === undefined) {
          sent = new Set()
          advisoryBySession.set(agent.session, sent)
        }
        if (sent.has(target)) return decision
        const createUserMessage = await messageFactory()
        if (createUserMessage === undefined) {
          log(`managed artifact edited directly: ${target} (advisory context unavailable without @deepseek-ai/dsh-llm)`)
          return decision
        }
        sent.add(target)
        const message = createUserMessage({
          content: [{ type: 'text', text: `${EDIT_ADVISORY}\n\nTarget: ${target}` }],
          source: PLUGIN_SOURCE,
        })
        return {
          ...decision,
          additionalContexts: [...(decision.additionalContexts ?? []), message],
        }
      } catch (error) {
        log(`edit advisory skipped: ${error.message ?? error}`)
        return decision
      }
    })
  }
}
