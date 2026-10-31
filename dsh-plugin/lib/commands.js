/** Human-facing slash commands for read-only governance status. */
import { isGovernedProject } from './paths.js'
import { renderResult } from './run.js'

export function registerCommands(ctx, runner, config) {
  const register = (name, description, argvOf) => {
    ctx.commands.register({
      name,
      description,
      handler: async ({ agent, signal }) => {
        let cwd
        try {
          cwd = ctx.workingDirectory.get(agent.session)
        } catch {
          cwd = undefined
        }
        if (!isGovernedProject(cwd)) {
          return { kind: 'error', text: 'This workspace is not governed by govctl (no gov/config.toml).' }
        }
        try {
          const result = await runner.run({ agent, signal }, argvOf())
          const text = renderResult(result, config.maxOutputBytes)
          return result.code === 0 ? { kind: 'success', text } : { kind: 'error', text }
        } catch (error) {
          return { kind: 'error', text: String(error?.message ?? error) }
        }
      },
    })
  }

  register('gov-status', 'Show the govctl project overview', () => ['status'])
  register('gov-check', 'Validate governed artifacts and rendered projections', () => ['check'])
}
