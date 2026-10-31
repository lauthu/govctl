/**
 * govctl-dsh — a DeepSeek Harness bundle that exposes govctl governance to DSH
 * agents.
 *
 * The bundle patch inserts this plugin as a root-level row. Root registrations
 * land in the registries' global layer, which every agent scope inherits, so no
 * shipped agent preset is patched or duplicated.
 *
 * Registered surface:
 *  - tools: read-only govctl_* wrappers + approval-gated mutating wrappers
 *  - system prompt: standing governance policy, only in governed repositories
 *  - session context: bounded `govctl agent hook session-start` summary
 *  - skills: the bundled govctl skill set (13 workflows + 4 reviewers)
 *  - commands: /gov-status, /gov-check
 *
 * No runtime imports from @deepseek-ai/* packages are required; the optional
 * @deepseek-ai/dsh-llm peer upgrades session/advisory context to sourced user
 * messages and is loaded lazily.
 */
import { registerCommands } from './commands.js'
import { registerGovernanceContext } from './context.js'
import { isGovernedProject } from './paths.js'
import { POLICY_SECTION } from './prompt.js'
import { createRunner } from './run.js'
import { registerSkills } from './skills.js'
import { registerTools } from './tools.js'

export const name = 'govctl-dsh'
export const inject = ['tools', 'systemPrompt', 'skills', 'commands', 'subprocess', 'workingDirectory']

/**
 * Tools that can mutate governed state or execute project code. The plugin
 * gates these through `tools/pre-execute` -> `ask`, which routes to DSH's
 * approval flow and fails closed when no approval channel is available.
 */
export const MUTATING_TOOLS = new Set([
  'govctl_work_new',
  'govctl_work_move',
  'govctl_edit',
  'govctl_render',
  'govctl_verify',
  'govctl_run',
])

const DEFAULT_CONFIG = {
  binaryPath: 'govctl',
  toolTimeoutMs: 20_000,
  maxOutputBytes: 65_536,
  enablePolicySection: true,
  enableSessionContext: true,
  enableEditAdvisory: true,
}

function positiveNumber(value, fallback) {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : fallback
}

function normalizeConfig(raw) {
  const input = raw !== null && typeof raw === 'object' ? raw : {}
  return {
    binaryPath:
      typeof input.binaryPath === 'string' && input.binaryPath.trim().length > 0
        ? input.binaryPath.trim()
        : DEFAULT_CONFIG.binaryPath,
    toolTimeoutMs: positiveNumber(input.toolTimeoutMs, DEFAULT_CONFIG.toolTimeoutMs),
    maxOutputBytes: positiveNumber(input.maxOutputBytes, DEFAULT_CONFIG.maxOutputBytes),
    enablePolicySection: input.enablePolicySection !== false,
    enableSessionContext: input.enableSessionContext !== false,
    enableEditAdvisory: input.enableEditAdvisory !== false,
  }
}

export function apply(ctx, rawConfig) {
  const config = normalizeConfig(rawConfig)
  const log = (message) => {
    // A plain stdout line is the load/observability signal for headless boots
    // and integration checks. Deliberately one channel, so a deployment that
    // also routes ctx.logger to stdout cannot double-print the banner.
    console.log(`[govctl-dsh] ${message}`)
  }

  ctx.effect(() => {
    log(`loaded (binary: ${config.binaryPath})`)
    return () => log('disposed')
  })

  const runner = createRunner(ctx, config)

  // Approval gate for every mutating/executing tool.
  ctx.on('tools/pre-execute', async (exec, next) => {
    if (MUTATING_TOOLS.has(exec.name)) {
      return {
        kind: 'ask',
        reason:
          `${exec.name} can modify governed artifacts or execute project commands; ` +
          'run it only when the user has authorized this operation.',
      }
    }
    return next()
  })

  registerTools(ctx, runner, config)

  if (config.enablePolicySection) {
    ctx.systemPrompt.section({
      name: 'policy:govctl',
      // 700: after plan/team policy (500/600), before first-party tool guidance.
      order: 700,
      text: (context) => {
        const agent = context?.agent
        if (agent === undefined) return ''
        let cwd
        try {
          cwd = ctx.workingDirectory.get(agent.session)
        } catch {
          return ''
        }
        return isGovernedProject(cwd) ? POLICY_SECTION : ''
      },
    })
  }

  if (config.enableSessionContext || config.enableEditAdvisory) {
    registerGovernanceContext(ctx, runner, config, log)
  }

  registerSkills(ctx, log)
  registerCommands(ctx, runner, config)

  // Warn (never fail) when the installed binary is outside the tested line.
  void runner.checkCompatibility()
}
