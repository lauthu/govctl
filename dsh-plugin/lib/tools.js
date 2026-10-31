/**
 * Model-facing tools for govctl.
 *
 * All tools are thin transports over the govctl CLI: governance semantics
 * (lifecycle gates, signatures, locking, claims) stay in the Rust binary. The
 * raw `ctx.tools.register` form is used so this bundle needs no
 * `@deepseek-ai/dsh-tools` import; each definition owns its argument handling.
 */
import { renderResult } from './run.js'

const ARTIFACT_RESOURCES = ['rfc', 'adr', 'work', 'guard', 'clause', 'conformance']

const NO_PARAMS = { type: 'object', properties: {}, additionalProperties: false }

function textParameters(properties, required = []) {
  return { type: 'object', properties, required, additionalProperties: false }
}

function string(description) {
  return { type: 'string', description }
}

function integer(description) {
  return { type: 'integer', description }
}

function stringArray(description) {
  return { type: 'array', description, items: { type: 'string' } }
}

function boolean(description) {
  return { type: 'boolean', description }
}

/**
 * Register every govctl tool. Mutating tools are approval-gated by the caller's
 * `tools/pre-execute` listener (see lib/index.js).
 */
export function registerTools(ctx, runner, config) {
  const register = (definition) => {
    ctx.tools.register({
      ...definition,
      timeoutMs: config.toolTimeoutMs,
      output: {
        schema: { type: 'string' },
        render: (_args, value) => [{ type: 'text', text: value }],
      },
    })
  }

  // -------------------------------------------------------------------------
  // Read-only tools
  // -------------------------------------------------------------------------

  register({
    name: 'govctl_describe',
    description:
      'Output govctl\u2019s machine-readable schema (JSON): commands, flags, valid field paths and context. ' +
      'Call this when you need the exact current CLI surface before constructing a command.',
    parameters: textParameters({ context: boolean('Include project state and suggested actions.') }),
    async execute(args, exec) {
      const argv = ['describe']
      if (args.context === true) argv.push('--context')
      return renderResult(await runner.run(exec, argv), config.maxOutputBytes)
    },
  })

  register({
    name: 'govctl_status',
    description:
      'Show the govctl project overview: config, artifact counts, active work items and open loops. ' +
      'Call this first in a governed repository.',
    parameters: NO_PARAMS,
    async execute(_args, exec) {
      return renderResult(await runner.run(exec, ['status']), config.maxOutputBytes)
    },
  })

  register({
    name: 'govctl_list',
    description:
      'List governed artifacts of one kind as JSON. Useful filters: a status, phase, or ID/title substring ' +
      '(e.g. filter "active" for work items), a tag, or a result limit.',
    parameters: textParameters(
      {
        resource: { type: 'string', enum: ARTIFACT_RESOURCES, description: 'Artifact kind to list.' },
        filter: string('Status, phase, or ID/title substring filter.'),
        limit: integer('Maximum number of rows.'),
        tag: string('Only artifacts carrying this tag.'),
      },
      ['resource'],
    ),
    async execute(args, exec) {
      const argv = [String(args.resource), 'list']
      if (typeof args.filter === 'string' && args.filter.length > 0) argv.push(args.filter)
      if (typeof args.limit === 'number') argv.push('-n', String(args.limit))
      if (typeof args.tag === 'string' && args.tag.length > 0) argv.push('--tag', args.tag)
      argv.push('-o', 'json')
      return renderResult(await runner.run(exec, argv), config.maxOutputBytes)
    },
  })

  register({
    name: 'govctl_get',
    description:
      'Get one governed artifact\u2019s metadata (JSON), or one field/path of it. Use for compact state inspection; ' +
      'use govctl_show for rendered content.',
    parameters: textParameters(
      {
        resource: { type: 'string', enum: ARTIFACT_RESOURCES, description: 'Artifact kind.' },
        id: string('Artifact identifier, e.g. RFC-0002, ADR-0061, WI-2026-10-18-001, RFC-0001:C-SCOPE.'),
        field: string('Optional field path, e.g. title, verification.required_guards, "alternatives[0].status".'),
      },
      ['resource', 'id'],
    ),
    async execute(args, exec) {
      const argv = [String(args.resource), 'get', String(args.id)]
      if (typeof args.field === 'string' && args.field.length > 0) {
        argv.push(args.field, '-o', 'plain')
      } else {
        argv.push('-o', 'json')
      }
      return renderResult(await runner.run(exec, argv), config.maxOutputBytes)
    },
  })

  register({
    name: 'govctl_show',
    description:
      'Show the rendered content of one governed artifact (RFC, ADR, work item, guard, clause, conformance case). ' +
      'Default output is readable markdown-like plain text; request json for structured data.',
    parameters: textParameters(
      {
        resource: { type: 'string', enum: ARTIFACT_RESOURCES, description: 'Artifact kind.' },
        id: string('Artifact identifier.'),
        format: {
          type: 'string',
          enum: ['plain', 'json', 'yaml', 'toml'],
          description: 'Output format (default plain).',
        },
      },
      ['resource', 'id'],
    ),
    async execute(args, exec) {
      const format = typeof args.format === 'string' ? args.format : 'plain'
      const argv = [String(args.resource), 'show', String(args.id), '-o', format]
      return renderResult(await runner.run(exec, argv), config.maxOutputBytes)
    },
  })

  register({
    name: 'govctl_search',
    description:
      'Search all governed artifacts (title, body, IDs) and return ranked JSON rows. ' +
      'Use it to locate the RFC/ADR/work item that governs a topic before changing behavior.',
    parameters: textParameters(
      {
        query: string('Search terms (user search syntax, not backend query syntax).'),
        types: {
          type: 'array',
          description: 'Restrict to artifact kinds.',
          items: { type: 'string', enum: ARTIFACT_RESOURCES },
        },
        tag: string('Only artifacts carrying this tag.'),
        limit: integer('Maximum number of results.'),
      },
      ['query'],
    ),
    async execute(args, exec) {
      const argv = ['search', String(args.query)]
      if (Array.isArray(args.types)) {
        for (const type of args.types) argv.push('--type', String(type))
      }
      if (typeof args.tag === 'string' && args.tag.length > 0) argv.push('--tag', args.tag)
      if (typeof args.limit === 'number') argv.push('-n', String(args.limit))
      argv.push('-o', 'json')
      return renderResult(await runner.run(exec, argv), config.maxOutputBytes)
    },
  })

  register({
    name: 'govctl_check',
    description:
      'Validate every governed artifact and rendered projection in the repository. Returns the diagnostics; ' +
      'a non-zero result means the repository currently violates governance constraints.',
    parameters: NO_PARAMS,
    async execute(_args, exec) {
      return renderResult(await runner.run(exec, ['check']), config.maxOutputBytes)
    },
  })

  register({
    name: 'govctl_verify',
    description:
      'Run reusable verification guards, or the effective guard set for one work item. ' +
      'Executes project-defined guard commands (e.g. tests), so it is approval-gated.',
    parameters: textParameters({
      work: string('Run the effective guard set for this work item ID.'),
      guards: stringArray('Specific guard IDs to run.'),
    }),
    async execute(args, exec) {
      const argv = ['verify']
      if (typeof args.work === 'string' && args.work.length > 0) {
        argv.push('--work', args.work)
      } else if (Array.isArray(args.guards) && args.guards.length > 0) {
        for (const guard of args.guards) argv.push(String(guard))
      }
      return renderResult(await runner.run(exec, argv), config.maxOutputBytes)
    },
  })

  register({
    name: 'govctl_trace',
    description:
      'Query declared requirement-to-Conformance-Case-to-Guard traceability as JSON. Omit target to map everything.',
    parameters: textParameters({
      target: string('RFC, Clause, Conformance Case, or Verification Guard ID.'),
    }),
    async execute(args, exec) {
      const argv = ['conformance', 'trace']
      if (typeof args.target === 'string' && args.target.length > 0) argv.push(args.target)
      argv.push('-o', 'json')
      return renderResult(await runner.run(exec, argv), config.maxOutputBytes)
    },
  })

  // -------------------------------------------------------------------------
  // Approval-gated tools (see MUTATING_TOOLS in lib/index.js)
  // -------------------------------------------------------------------------

  register({
    name: 'govctl_work_new',
    description:
      'Create a new work item (starts queued, or active with active=true). ' +
      'Requires user approval; add acceptance criteria before moving it to done.',
    parameters: textParameters(
      {
        title: string('Work item title.'),
        active: boolean('Immediately activate the new work item.'),
      },
      ['title'],
    ),
    async execute(args, exec) {
      const argv = ['work', 'new', String(args.title)]
      if (args.active === true) argv.push('--active')
      return renderResult(await runner.run(exec, argv), config.maxOutputBytes)
    },
  })

  register({
    name: 'govctl_work_move',
    description:
      'Move a work item to a new status: queue, active, done, or cancelled. ' +
      'done requires acceptance criteria and effective guards to pass. Requires user approval.',
    parameters: textParameters(
      {
        id: string('Work item ID or file path.'),
        status: { type: 'string', enum: ['queue', 'active', 'done', 'cancelled'], description: 'Target status.' },
      },
      ['id', 'status'],
    ),
    async execute(args, exec) {
      return renderResult(
        await runner.run(exec, ['work', 'move', String(args.id), String(args.status)]),
        config.maxOutputBytes,
      )
    },
  })

  register({
    name: 'govctl_edit',
    description:
      'Edit a governed artifact through its canonical path-first command: set/add/remove/tick a field path. ' +
      'This preserves lifecycle-owned fields and signatures. Requires user approval.',
    parameters: textParameters(
      {
        resource: { type: 'string', enum: ARTIFACT_RESOURCES, description: 'Artifact kind.' },
        id: string('Artifact identifier.'),
        path: string('Canonical field path, e.g. description, refs, "alternatives[0].pros".'),
        action: { type: 'string', enum: ['set', 'add', 'remove', 'tick'], description: 'Edit operation.' },
        value: string('Value for the operation (omit only with action remove on an indexed path, or with stdin).'),
        stdin: boolean('Read the value from stdin.'),
      },
      ['resource', 'id', 'path', 'action'],
    ),
    async execute(args, exec) {
      const action = String(args.action)
      const value = typeof args.value === 'string' ? args.value : undefined
      const stdin = args.stdin === true
      if ((action === 'set' || action === 'add') && value === undefined && !stdin) {
        return `govctl_edit: action "${action}" needs a value or stdin=true.`
      }
      const argv = [String(args.resource), 'edit', String(args.id), String(args.path)]
      if (action === 'set') {
        argv.push('--set')
        if (value !== undefined) argv.push(value)
      } else if (action === 'add') {
        argv.push('--add')
        if (value !== undefined) argv.push(value)
      } else if (action === 'remove') {
        argv.push('--remove')
        if (value !== undefined) argv.push(value)
      } else {
        if (value === undefined) return 'govctl_edit: action "tick" needs a value (done, pending, cancelled, ...).'
        argv.push('--tick', value)
      }
      if (stdin) argv.push('--stdin')
      return renderResult(await runner.run(exec, argv), config.maxOutputBytes)
    },
  })

  register({
    name: 'govctl_render',
    description:
      'Render governed artifacts to markdown projections under docs/. Writes files, so it is approval-gated. ' +
      'Use dryRun=true to preview what would be written.',
    parameters: textParameters({
      target: {
        type: 'string',
        enum: ['rfc', 'adr', 'work', 'changelog', 'all'],
        description: 'Render target (default rfc).',
      },
      dryRun: boolean('Preview without writing.'),
    }),
    async execute(args, exec) {
      const target = typeof args.target === 'string' ? args.target : 'rfc'
      const argv = ['render', target]
      if (args.dryRun === true) argv.push('--dry-run')
      return renderResult(await runner.run(exec, argv), config.maxOutputBytes)
    },
  })

  register({
    name: 'govctl_run',
    description:
      'Run an arbitrary govctl subcommand as an argv array (no shell). Escape hatch for operations the typed tools ' +
      'do not cover: lifecycle verbs (rfc finalize/advance/bump, adr accept, work delete), clause editing/removal, ' +
      'claims, tags, loops, releases. Always requires user approval. Only run mutations the user authorized.',
    parameters: {
      type: 'object',
      properties: {
        argv: {
          type: 'array',
          description: 'Arguments after the govctl binary, e.g. ["rfc","show","RFC-0002"] or ["work","list","active"].',
          items: { type: 'string' },
        },
      },
      required: ['argv'],
      additionalProperties: false,
    },
    async execute(args, exec) {
      const argv = Array.isArray(args.argv) ? args.argv.map(String) : []
      if (argv.length === 0) return 'govctl_run: argv must be a non-empty array of strings.'
      return renderResult(await runner.run(exec, argv), config.maxOutputBytes)
    },
  })
}
