# govctl DSH bundle — proof of concept

A minimal, dependency-free DeepSeek Harness (DSH) bundle that exposes govctl to
DSH agents. This directory is the executable answer to "what does it actually
take to make govctl a DSH plugin?"; it validates the packaging model end to end
on the installed DSH **0.2.1-rc.5**.

## What it proves

1. A third-party npm package can declare `dsh.bundle.patch`, ship a
   `cordis.patch.yml`, and be installed into a DSH profile with
   `dsh plugin --profile <name> add <path>`.
2. Its patch inserts an ordinary root row; the composed profile shows the bundle
   layer in `dsh --profile <name> --dump-config`.
3. At boot the Cordis plugin's `apply(ctx, config)` runs; it registers
   `ctx.tools`, `ctx.systemPrompt`, `ctx.skills`, `ctx.commands`, and an
   `agent/created` listener without any `@deepseek-ai/*` runtime import
   (zero dependencies, no build step).

## Files

| File | Purpose |
| --- | --- |
| `package.json` | npm package + `dsh.bundle.patch` manifest + `engines.dsh` |
| `cordis.patch.yml` | bundle layer: inserts `id: govctl` row with config |
| `index.js` | Cordis plugin: tools, prompt section/context, skills, commands |
| `skills/gov/SKILL.md` | ported from govctl `.claude/skills/gov` (`$ARGUMENTS` → `<task>`) |
| `skills/quick/SKILL.md` | ported from govctl `.claude/skills/quick` |

## Validated on 2026-10-31 (scratch DSH_HOME, real profiles untouched)

```sh
export DSH_HOME=/tmp/dsh-poc-home
export PATH="/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin:$PATH"
mkdir -p "$DSH_HOME"
dsh --profile web --dump-config            # auto-initializes the web profile template
dsh plugin --profile web add "$PWD"        # pnpm link + bundle reconciliation
dsh --profile web --dump-config | grep -A6 govctl
dsh web --no-open --port 59631             # boot smoke test; stopped after check
```

Observed:

- `dsh plugin ... add` rewrote the profile manifest:
  `"govctl-dsh-plugin": "link:/…/research/poc/govctl-dsh-plugin"` and appended
  `govctl-dsh-plugin` to `dsh.profile.bundles`.
- `--dump-config` printed a `# == govctl-dsh-plugin` layer with the row
  `- id: govctl / name: govctl-dsh-plugin / config: …`.
- Boot log: `[govctl-dsh-plugin] loaded (binary: govctl)`,
  `registered skill "gov"`, `registered skill "quick"`, then the web URL.
  (No model turn was run, so tools were not invoked.)

## API mapping used

| Capability | DSH 0.2.1-rc.5 API | Notes |
| --- | --- | --- |
| Plugin entry | `export const name`, `export const inject`, `export function apply(ctx, config)` | Cordis plugin; row `config` is passed as `apply`'s second argument |
| Model tools | `ctx.tools.register({ name, description, parameters, output, execute })` | Raw JSON-Schema `parameters`; `output.schema` + `output.render`; `execute(args, { agent, signal })` |
| System prompt | `ctx.systemPrompt.section({ name, order, text })` | `text` may be a `(context) => string` function; order 700 sits after plan/team policy |
| Session context | `ctx.on('agent/created', …)` + `ctx.systemPrompt.context({ name, order, text })` | caches `govctl agent hook session-start` output per session; `agent/session-start` does not exist in 0.2.1 |
| Skills | `ctx.skills.register({ name, description, content })` | embedded runtime skills; unknown frontmatter keys (`allowed-tools`, `argument-hint`) are ignored |
| Slash commands | `ctx.commands.register({ name, description, handler })` | handler returns `{ kind: 'success' | 'error', text }` |
| Workspace cwd | `ctx.workingDirectory.get(agent.session)` | sync, projection-backed |
| Binary call | `node:child_process.execFile` (no shell) | production should use `ctx.subprocess` (lifecycle/env scrub) or `ctx.shell` (sandbox policy) |

## Known gaps (why this is a PoC, not a release)

- **Only 2 of 13 skills** are ported. Porting is mechanical (`$ARGUMENTS` → wording,
  drop `allowed-tools`/`argument-hint`) but reviewer-agent references need a
  DSH-side answer.
- **`govctl_run` is approval-gated** through `tools/pre-execute` → `ask` (the
  registry routes that to `approval.request` and fails closed without an approval
  channel), but there is no argument-level policy (e.g. allow `work list`,
  always ask for `rfc bump`).
- **PreToolUse advisory is detected but not attached.** govctl's Claude hook never
  blocks; it attaches an advisory context message. This PoC logs the detection in
  `tools/post-execute`; attaching the message requires an `additionalContexts`
  `createUserMessage` from `@deepseek-ai/dsh-llm` (peer dependency).
- **No binary discovery UX.** `binaryPath` defaults to `govctl` on `PATH`; there
  is no version check, no `cargo binstall` hint, and no vendored binary. Local
  tool execution was not exercised (govctl is not installed in this environment).
- **`execFile` instead of `ctx.subprocess`.** Production should spawn through
  `ctx.subprocess.resolveExecutable`/`spawn` for env scrubbing, output caps and
  managed termination (or `ctx.shell` to honor session sandbox policy).
- **Session state is a prompt context, not a sourced message.** The v2 design
  (`agent/pre-step` + `createUserMessage`) gives durable source-attributed history.
- **No reviewer agents.** govctl's four `.claude/agents/*.md` reviewers have no
  direct DSH drop-in; options are presets, skills that instruct a reviewer
  workflow, or subagent prompts.
- **Distribution not decided.** npm publish + a DSH plugin-market index entry is
  the likely channel; the live GUI runs the app-owned `desktop` profile, so the
  install path for desktop users needs to be confirmed separately.
- **`engines.dsh` is a single range** (`>=0.2.1-rc.5 <0.3.0-0`); every DSH API
  used here must be re-verified per DSH release, and the package needs its own
  release cadence.

## Review

The DSH platform researcher reviewed this PoC against the installed 0.2.1-rc.5 sources. Applied
fixes: stdin plumbing for `govctl agent hook`, non-blocking session state refresh, the
`tools/pre-execute` approval gate for `govctl_run`, post-execute advisory detection, and the
`engines.dsh` upper bound. Deliberately deferred: `ctx.subprocess`, `createUserMessage`-based
sourced context/advisory, full skill set and a real desktop-profile install.

## Cleanup

```sh
rm -rf /tmp/dsh-poc-home /tmp/poc-boot.log /tmp/poc-dump-*.txt
```
