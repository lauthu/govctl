# govctl-dsh

A DeepSeek Harness (DSH) bundle that brings
[govctl](https://github.com/govctl-org/govctl) governance to DSH agents: read
tools over the `govctl` CLI, approval-gated mutations, a standing governance
policy section, session-start state, lifecycle-managed edit advisories, 17
bundled skills, and two slash commands.

govctl stays the source of truth. The plugin is a transport: it calls the
`govctl` binary and never reimplements RFC lifecycle rules, phase gates,
signatures, locking, or claims in JavaScript.

| | |
| --- | --- |
| Package | `govctl-dsh` (`0.1.0`), ESM, MIT |
| DSH bundle | `dsh.bundle.patch` → `cordis.patch.yml` (root rows, inherited by every agent preset) |
| Runtime dependencies | none required; one **optional** peer (`@deepseek-ai/dsh-llm`) upgrades context delivery |
| Verified on | DSH `0.2.1-rc.5` (scratch web-profile install + boot); govctl `0.22.x` — see [Verification status](#verification-status) |
| Status | experimental; DSH is a release candidate and its APIs may still change. This release is published from the `lauthu/govctl` fork; upstream ownership and npm scope are a separate decision. |

## What the plugin registers

- **14 `govctl_*` tools** — 8 read-only tools (no approval) plus 6
  approval-gated mutation/execution tools. See [Tools](#tools).
- **A governance policy section** — a short system-prompt section (order 700),
  contributed only when the session works in a governed repository (one with
  `gov/config.toml`). It states RFC supremacy, phase discipline, no silent
  deviation, and the govctl hard stops.
- **Session-start state** — at agent creation the plugin runs
  `govctl agent hook session-start` and injects the bounded summary, once per
  session. With the optional `@deepseek-ai/dsh-llm` peer installed it is a
  sourced user message; otherwise it falls back to a runtime prompt context.
- **Edit advisories** — after a successful `edit`/`write` that directly targets
  a lifecycle-managed `gov/` artifact, the plugin attaches a fixed advisory
  through `tools/post-execute`. It never blocks or rewrites the edit.
- **17 skills** — the govctl workflows and writers plus four reviewer/audit
  skills. See [Skills](#skills).
- **2 slash commands** — `/gov-status` and `/gov-check`.

## Requirements

- **DSH `>=0.2.1-rc.5 <0.3.0-0`.** The bundled APIs were verified against
  `0.2.1-rc.5`. `engines.dsh` gates install/recomposition; incompatible
  versions are refused by DSH unless you explicitly accept the risk.
- **Node `>=20`** (DSH `0.2.1-rc.5` ships Node 24).
- **The `govctl` binary on `PATH`, or `config.binaryPath` pointing at it.**
  Install it with `cargo binstall govctl` or from the
  [GitHub Releases](https://github.com/govctl-org/govctl/releases) page. The
  plugin does not bundle the binary.
- A govctl-governed workspace (`gov/config.toml`) to get the policy section,
  session state, and the governance workflows; most tools are only meaningful
  there.

The `dsh` CLI is not necessarily on `PATH`. When using the packaged desktop app,
call the bundled CLI explicitly, for example on macOS:
`/Applications/DeepSeek\ Harness.app/Contents/Resources/runtime/cli/bin/dsh`.

## Install

> Pick the profile you actually run. The running desktop GUI uses the
> `desktop` profile; a web or headless session uses its own profile name.
> The `desktop` profile is owned by the Electron app — never boot it from the
> CLI (`dsh --profile desktop` is refused by design).

### Desktop GUI (the profile the app runs)

1. **Quit DeepSeek Harness completely.** The desktop profile is app-owned and
   `dsh plugin --profile desktop …` requires the app to have initialized the
   profile and to be fully quit.
2. Install the package:

   ```sh
   dsh plugin --profile desktop add govctl-dsh
   ```

   Alternatively use the in-app **Plugins** page and its *Add plugin* action
   (DSH's in-app package-manager path).
3. Relaunch the app so the new bundle layer is composed, then confirm the
   governance tools are available in a session.

Profiles are pnpm projects under `$DSH_HOME/profiles/<name>` (`DSH_HOME`
defaults to `~/.dsh`); the install is a durable profile-file change that the app
reads on the next launch. This desktop path follows DSH's profile-ownership
model but was **not** exercised in the verification run (see
[Verification status](#verification-status)).

### Web, headless, or a scratch profile

```sh
dsh plugin --profile web add govctl-dsh
dsh --profile web --dump-config | grep -A8 govctl   # preview the composed layer
```

`dsh plugin` forwards to pnpm in the profile directory and initializes a
base-backed profile on first use (substitute any profile name for `web`).

### Development install from a checkout

```sh
dsh plugin --profile test add /absolute/path/to/govctl/dsh-plugin
```

A local path is installed as a `link:` dependency, so edits under
`dsh-plugin/lib/` and `dsh-plugin/assets/` are picked up on restart (config-only
changes can recompose live through DSH's HMR; package installation follows
DSH's restart rules).

**Development installs and the optional peer.** pnpm may warn that a `link:`
(or `file:`) install does not resolve a declared peer the way a registry install
does. DSH's linked-root resolution can still find `@deepseek-ai/dsh-llm` at
runtime — but the plugin does not depend on that: it imports the peer lazily and
falls back to runtime-prompt-context delivery whenever resolution is unavailable,
so session state always arrives (only the sourced-message form depends on the
peer). To exercise the sourced-message path deterministically from a checkout,
pack the package and install the tarball:

```sh
npm pack
dsh plugin --profile <name> add ./govctl-dsh-0.1.0.tgz
```

### Verify the install

```sh
govctl --version                         # binary is reachable
dsh --profile <name> --dump-config       # shows the "# == govctl-dsh" layer
```

In a governed repository, start a session and try `/gov-status` and
`/gov-check`, or ask the agent to call `govctl_status`. A missing or
out-of-range binary surfaces as an actionable tool error (install govctl,
fix `PATH`, or set `binaryPath`); it does not crash the plugin.

### Upgrade and uninstall

```sh
dsh plugin --profile <name> add govctl-dsh@<version>   # upgrade
dsh plugin --profile <name> remove govctl-dsh          # uninstall
```

Restart the app (or the DSH process) after changing installed bundles.

## Configuration

The bundle's `cordis.patch.yml` inserts the `govctl` row with these defaults.
A profile patch that targets the row **replaces its whole `config`** (Cordis
never deep-merges), so restate every key you care about.

| Key | Type | Default | Effect |
| --- | --- | --- | --- |
| `binaryPath` | string | `"govctl"` | Executable to run. An absolute path is the reliable choice for GUI launches with a minimal `PATH`. |
| `toolTimeoutMs` | number | `20000` | Per-tool-call timeout in milliseconds. |
| `maxOutputBytes` | number | `65536` | Output cap per stream. In-process formatting adds an explicit truncation marker; subprocess-collected caps are silent (the model still sees the exit status). |
| `enablePolicySection` | boolean | `true` | Contribute the standing governance policy section in governed repositories. |
| `enableSessionContext` | boolean | `true` | Inject the `govctl agent hook session-start` state once per session. |
| `enableEditAdvisory` | boolean | `true` | Attach the advisory after direct edits of lifecycle-managed `gov/` artifacts. |

Example override in the profile's `cordis.patch.yml` (full config restated):

```yaml
- id: govctl
  config:
    binaryPath: /usr/local/bin/govctl
    toolTimeoutMs: 20000
    maxOutputBytes: 65536
    enablePolicySection: true
    enableSessionContext: true
    enableEditAdvisory: true
```

Mutating/executing tools are always approval-gated; there is no config switch
to bypass the approval flow. The plugin warns (does not fail) at load when
`govctl --version` is outside the tested `0.22.x` line.

## Tools

All tools spawn `govctl` with an explicit argv array (no shell), run in the
session's working directory, cap output at `maxOutputBytes`, and surface the
exit code plus bounded stdout/stderr to the model.

### Read-only (no approval)

| Tool | Backing command | Arguments |
| --- | --- | --- |
| `govctl_describe` | `govctl describe` | `context?` (boolean, add `--context`) |
| `govctl_status` | `govctl status` | none |
| `govctl_list` | `govctl <resource> list -o json` | `resource` (`rfc`/`adr`/`work`/`guard`/`clause`/`conformance`), `filter?`, `limit?`, `tag?` |
| `govctl_get` | `govctl <resource> get <id> [-o json\|plain]` | `resource`, `id`, `field?` (a canonical field path) |
| `govctl_show` | `govctl <resource> show <id> -o <format>` | `resource`, `id`, `format?` (`plain`/`json`/`yaml`/`toml`; default `plain`) |
| `govctl_search` | `govctl search <query> -o json` | `query`, `types?`, `tag?`, `limit?` |
| `govctl_check` | `govctl check` | none |
| `govctl_trace` | `govctl conformance trace -o json` | `target?` (RFC, Clause, Conformance Case, or Guard ID) |

### Approval-gated (mutating or executing)

Every call is routed through DSH's approval flow (`tools/pre-execute` → `ask`)
and **fails closed** when no approval channel is available. Only perform
mutations the user has authorized.

| Tool | Backing command | Arguments |
| --- | --- | --- |
| `govctl_work_new` | `govctl work new <title> [--active]` | `title`, `active?` |
| `govctl_work_move` | `govctl work move <id> <status>` | `id`, `status` (`queue`/`active`/`done`/`cancelled`) |
| `govctl_edit` | `govctl <resource> edit <id> <path> --set\|--add\|--remove\|--tick` | `resource`, `id`, `path` (canonical field path), `action`, `value?`, `stdin?` |
| `govctl_render` | `govctl render <target> [--dry-run]` | `target?` (`rfc`/`adr`/`work`/`changelog`/`all`; default `rfc`), `dryRun?` |
| `govctl_verify` | `govctl verify [--work <id>] [<guard>…]` | `work?` or `guards?`. Runs project-defined guard commands (e.g. tests), hence approval-gated. |
| `govctl_run` | `govctl <argv…>` | `argv` (non-empty array of strings). Escape hatch for anything not typed above: lifecycle verbs (`rfc finalize`/`advance`/`bump`, `adr accept`, `work delete`), clause editing, claims, tags, loops, releases. Always approval-gated. |

The govctl CLI is resource-first (`govctl rfc list`, `govctl work show`, …) with
global `status`, `check`, `verify`, `search`, `describe`, `render`, and
`conformance trace` commands; `govctl_describe` returns the current
machine-readable command/flag/field-path catalog, so prefer it over guessing
syntax.

### Slash commands

| Command | Runs |
| --- | --- |
| `/gov-status` | `govctl status` |
| `/gov-check` | `govctl check` |

Both error out cleanly when the current workspace is not governed.

## Skills

The bundle ships 17 skills under `assets/skills/`. They are the normalized
govctl workflows and writers (from the govctl repo's `.claude/skills/`) plus
four reviewer/audit skills converted from `.claude/agents/`.

| Skill | Purpose |
| --- | --- |
| `init` | Set up govctl in the current project with explicit installation and overwrite authorization |
| `discuss` | Explore a governance design, resolve ambiguity, and draft RFC or ADR artifacts without implementation |
| `spec` | Maintain RFC and ADR artifacts without implementation work |
| `gov` | Execute governed implementation with work-item traceability, RFC/ADR authority, risk-scoped verification, and closure |
| `quick` | Execute a trivial non-behavioral change without unnecessary governance ceremony |
| `commit` | Record a focused VCS commit with govctl validation and Work Item traceability |
| `migrate` | Adopt govctl in an existing project by confirming historical decisions and active work before backfilling a governed baseline |
| `detach` | Remove project-local govctl integration through an ownership-aware dry run, archival plan, and recoverable edits |
| `rfc-writer` | Write precise, implementation-independent RFCs with testable normative clauses |
| `adr-writer` | Write decision records with context, evaluated alternatives, direction, and consequences |
| `wi-writer` | Write durable work items with scoped descriptions, testable acceptance criteria, and guards |
| `guard-writer` | Define reusable, non-interactive verification guards with stable commands and risk-matched scope |
| `decision-analysis` | Stress-test consequential design options with premortem/backcast reasoning |
| `rfc-reviewer` | Review RFC drafts for contract clarity, normative correctness, completeness, and authority boundaries |
| `adr-reviewer` | Review ADR drafts for decision evidence, credible alternatives, honest consequences, and authority boundaries |
| `wi-reviewer` | Review Work Items for durable scope, testable outcomes, governing authority, and risk-matched verification |
| `compliance-checker` | Audit implementation against normative RFC obligations and report design drift separately from accepted ADR rationale |

DSH exposes skills to the agent (ask for one by name, e.g. "use the `gov`
skill"), and user-invocable skills are reachable as `/skill-name` slash
commands.

### Reviewer/audit skills — fidelity note

DSH `0.2.1-rc.5` has **no Markdown-defined agent mechanism**, so the four
reviewers are not agents. They are skills: the reviewer prompt is bundled with
one prepended instruction telling the model to apply the review to the target
the user named (or the current change set when none is named), and the bodies
remain review-only. This is a deliberate fidelity compromise. When an isolated
review context is wanted, run the skill through DSH's own `subagent` tool
instead of pretending the Markdown agents are loaded.

The skill sources are the govctl repo's `.claude/` assets (the SSOT). The
plugin does not read `.claude/` at runtime and ships only these normalized
copies.

## Verification status

What has actually been exercised against the installed harness:

- DSH `0.2.1-rc.5`.
- Package installed into **scratch `DSH_HOME` profiles** (real profiles
  untouched) from both a `link:` checkout and the published npm registry
  artifact (`dsh plugin --profile headless add govctl-dsh@0.1.0`).
- `dsh --profile <name> --dump-config` shows the `# == govctl-dsh` bundle layer.
- Web boot logs `[govctl-dsh] loaded (binary: govctl)` and
  `registered 17 bundled skills`.
- One **real headless live-model session** against govctl `0.22.0`: the model
  called `govctl_status` and `govctl_check` (both real invocations) and quoted
  the injected session-start governance state in its answer.
- A published-artifact install into a **copy** of the desktop profile updated
  the profile manifest as expected (dependencies + `dsh.profile.bundles`).
- **Live desktop profile**, installed from the npm registry and activated by
  quitting and relaunching the app: the policy section, the 17-skill catalog
  and the `govctl_*` tool schemas load; real tool calls (`govctl_status`,
  `govctl_check`, `govctl_search`) ran against govctl `0.22.0`; and a fresh GUI
  session received the sourced session-start context
  (`{"kind":"govctl-dsh","form":"instructions"}` in its session log).

Not verified: the GUI **post-edit advisory rendering** path (the advisory is
attached through `tools/post-execute` but needs the optional
`@deepseek-ai/dsh-llm` peer, which resolved to the harness fallback package in
this deployment), and the approval-request UI for mutating tools (the current
session runs with approval prompts disabled, so those calls fail closed).
Asset integrity and CLI stubs are covered by `npm run check-assets`,
`npm test`, and `scripts/validate-with-dsh.sh`; use those before claiming more
than the list above.

## Compatibility

| Component | Tested | Declared |
| --- | --- | --- |
| DSH | `0.2.1-rc.5` (scratch web-profile install + boot) | `engines.dsh`: `>=0.2.1-rc.5 <0.3.0-0` |
| govctl | `0.22.x` (assets generated from `0.22.0`) | warns at load when outside `0.22.x`; no hard failure |
| Node | `>=20` | `engines.node` |

DSH is a release candidate: a DSH release that changes any API this plugin uses
requires a plugin patch release and a re-test. The bundled assets are
regenerated from `.claude/` and stamped with the govctl version by
`scripts/sync-assets.mjs`; the package's `assets/manifest.json` records the
govctl version and a SHA-256 per skill so drift is detectable.

Root-row registration means the tools are visible in every agent preset,
including `minimal`/`ptc`. A `complete:true` persona preset suppresses the
policy section prose while leaving the tools visible — by design, not a bug.

## Security

- **Installing this package is a trust decision equivalent to installing
  govctl.** Plugin code runs in-process with host authority, unsandboxed.
- Read-only tools are thin `govctl` calls with fixed argument shapes; no shell
  is used anywhere.
- Mutating and executing tools require DSH approval on every call and fail
  closed without an approval channel. `govctl_run` accepts arbitrary argv and is
  always approval-gated.
- The plugin does not bundle or download a binary. Supply the `govctl` binary
  yourself from a trusted source and keep it up to date.
- Governance semantics remain in the Rust binary; the plugin passes results
  through and never approves a lifecycle operation on its own.
- Tool calls spawn through DSH's subprocess service (env-scrubbed, managed
  termination). The single read-only `SessionStart` hook probe uses
  `node:child_process` with its own credential/`DSH_*`-scrubbed environment only
  because DSH `0.2.1-rc.5` exposes no stdin channel on that service; the edit
  advisory is native in-process code. The plugin does not apply the session's
  file sandbox policy to spawned processes.

## Development

Requirements: this repository checkout, Node `>=20`, and no npm dependencies.
Run the commands below from the `dsh-plugin/` package directory.

```sh
npm run sync-assets     # regenerate assets from .claude/ (deterministic)
npm run check-assets    # fail (exit 1) if assets drift from .claude/
npm test                # node --test (default discovery)
npm run pack:check      # npm pack --dry-run
bash scripts/validate-with-dsh.sh   # end-to-end bundle validation in a scratch DSH_HOME
```

`scripts/validate-with-dsh.sh` is the DSH verification harness; it needs a DSH
installation and uses a scratch `DSH_HOME` so real profiles are untouched.

### Asset pipeline (`.claude/` is the SSOT)

`.claude/skills/*/SKILL.md` and `.claude/agents/*.md` are the only sources.
`scripts/sync-assets.mjs` copies and normalizes them into
`assets/skills/<name>/SKILL.md` and writes `assets/manifest.json`. Never hand-edit
files under `assets/skills/`; edit the `.claude/` source and re-run the sync.

Normalization rules:

- frontmatter keeps only `name` and `description` (double-quoted); Claude-only
  `allowed-tools` and `argument-hint` are dropped (DSH ignores them);
- `$ARGUMENTS` is replaced with the natural phrase "the task the user named"
  (DSH does not substitute Claude-style argument placeholders);
- Claude tool names appear only in the dropped frontmatter; bodies are otherwise
  copied verbatim;
- reviewer agents become skills: the agent body is kept verbatim after one
  prepended instruction line (see the fidelity note above);
- output ordering, bytes, and the manifest are deterministic (no timestamps,
  sorted by name); re-running the sync is a byte-identical no-op.

`assets/manifest.json` shape:

```json
{
  "govctlVersion": "<from Cargo.toml [package].version>",
  "source": ".claude",
  "skills": [{ "name": "gov", "source": ".claude/skills/gov/SKILL.md", "sha256": "…" }]
}
```

`check-assets` recomputes everything in memory, compares it with the checked-in
tree (including the manifest and unexpected files), reports each drift line,
and exits non-zero without writing. Run it in CI next to `npm test`.

## Limitations

- **Edit guidance is advisory, never enforcement.** DSH interception cannot
  hard-block a lifecycle-managed edit without breaking govctl's recoverability
  model; the plugin attaches advice and never denies or rewrites the edit.
- **No DSH-native Markdown agents.** The four govctl reviewers are skills; there
  is no automatic delegation or separate context unless you use `subagent`.
- **Session state is context, not history.** It is delivered once per session as
  a sourced message when the optional peer is available, otherwise as prompt
  context, and is not a durable governance record.
- **No bundled binary.** `govctl` must be installed and reachable; GUI launches
  with a minimal `PATH` may need an absolute `binaryPath`.
- **Human-oriented CLI output.** `status`, `check`, and `verify` have no JSON
  contract, so the plugin returns text plus the exit code; it does not synthesize
  a structured error envelope.
- **Root-scope visibility.** Tools appear in all presets (including `minimal`);
  a `complete:true` persona hides the policy prose while keeping the tools.
- **Desktop install path is app-owned.** Scripted installs require the app to be
  fully quit; otherwise use the GUI Plugins page.
- Not every DSH release candidate is covered: only `0.2.1-rc.5` is tested, and
  the govctl range is the `0.22.x` line.

## License

MIT — see [LICENSE](LICENSE). The bundled skills and reviewer prompts are
normalized copies of MIT-licensed govctl project assets (`.claude/`).
