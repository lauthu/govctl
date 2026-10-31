# Making govctl a DeepSeek Harness (DSH) plugin — research and design report

Status: research only (no governed artifacts created, no runtime code changed outside `research/`).
Date: 2026-10-31.
Repo: `/Users/liuxinyu/Projects/govctl` (govctl 0.22.0).
Target: installed DeepSeek Harness **0.2.1-rc.5** (`/Applications/DeepSeek Harness.app`).

This report answers one question: **how can govctl be made usable as a DSH plugin, and what
would it take?** It is backed by three evidence files and a validated proof-of-concept:

| Input | Deliverable |
| --- | --- |
| DSH plugin platform (installed ASAR, 0.2.1-rc.5) | [`research/dsh-platform-notes.md`](dsh-platform-notes.md) (922 lines) |
| govctl CLI / asset / agent-integration inventory | [`research/govctl-capabilities.md`](govctl-capabilities.md) (504 lines) |
| govctl governance constraints and prior art | [`research/governance-constraints.md`](governance-constraints.md) (567 lines) |
| Working proof-of-concept bundle | [`research/poc/govctl-dsh-plugin/`](poc/govctl-dsh-plugin/) (installed + booted in a scratch DSH home) |

---

## 0. Executive summary

**Yes, govctl can be a DSH plugin — but "plugin" has two different meanings, and the cheap one is
the right first move.**

1. **Recommended: an external DSH bundle (`govctl-dsh`) that wraps the installed govctl binary.**
   It is an ordinary npm package declaring `dsh.bundle.patch`; its patch inserts root-level plugin
   rows that register model tools, a governance prompt section, the govctl skills, slash commands
   and a session-start governance summary. DSH root registrations are inherited by every agent
   preset, so no shipped preset has to be patched. This adds **no govctl CLI surface**, so it does
   not require an RFC-0002 amendment: it needs an **ADR + Work Items + docs + release bookkeeping**.
   A PoC of exactly this shape was installed into a scratch DSH profile, composed into the tree
   with `dsh --dump-config`, and booted; the plugin loaded and registered skills (see §7).
2. **Later, only if first-class support is wanted: a native runtime (`govctl agent install dsh`).**
   This changes normative behavior — RFC-0002 is `normative`/`impl`/v3.8.0 (sealed) and its
   `C-AGENT-INTEGRATION` clause enumerates `<codex|claude|all>` — so it requires an RFC-0002
   amendment (bump 3.8.0 → 3.9.0, `spec` → `impl`), a new ADR, Work Items and a minor release. It
   is also blocked today by the third-party `agent-plugin-installer 0.6.1` crate, whose
   `AgentRuntime` enum only knows Codex and Claude.
3. **Free quick win (docs-only):** DSH's filesystem skill provider scans `<projectRoot>/.dsh/skills`
   by default, and govctl already has `govctl init-skills --dir .dsh` (Claude-format bundles).
   A short guide can make the existing skills appear in DSH today, with light content normalization
   and no new package. This is not a full plugin (no tools, no hook/context parity), but it is
   zero-risk and immediately useful.

**What does *not* work:** porting the existing Claude/Codex hook manifests as-is. DSH 0.2.1-rc.5
**removed** the Claude-Code hook bridge (`dsh-hooks-claude-code`/`dsh-hook-protocol` are absent,
and there is no `agent/session-start` event). Hook behavior must be reimplemented as native Cordis
interceptors (`agent/created`, `agent/pre-step`, `tools/pre-execute`, `tools/post-execute`).

**Biggest design consequences**

- Skills are format-compatible: DSH accepts `SKILL.md` with `name` + `description` and silently
  ignores Claude-only keys (`allowed-tools`, `argument-hint`). `$ARGUMENTS` is not substituted and
  must be rewritten.
- The CLI is wrapper-friendly for reads (`describe` JSON schema v1; `list/get/show/search/
  conformance trace/tag list/loop list -o json`) and exit-code-based for writes; `status`, `check`
  and `verify` have no machine-readable output.
- Plugin code runs **in-process, outside the workspace sandbox**. A governance plugin is
  therefore a trust decision for users; mutations need an explicit approval design.
- The live GUI runs the app-owned **`desktop`** profile. Installation is possible from the GUI
  Plugins page or `dsh plugin --profile desktop add …` with the app fully quit, but the desktop
  path was not exercised end-to-end here.
- DSH plugin packages must track a fast-moving API (0.1.1-rc.2 → 0.2.1-rc.5 removed an entire hook
  subsystem); version pinning and a compatibility matrix are mandatory.

---

## 1. Target platform: what a DSH plugin is (installed 0.2.1-rc.5)

### 1.1 Environment

- App release `0.2.1-rc.5`, host protocol 4, packaged Node 24.21.0 and pnpm 11.28.5
  (`app.asar/dsh/desktop-runtime.json`).
- Bundled CLI: `/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin/dsh`
  (a wrapper around `dsh-desktop-host/lib/cli.js`); no `dsh` on the user's `PATH`.
- Profiles: `$DSH_HOME/profiles/<name>` (`DSH_HOME` defaults to `~/.dsh`). The **running GUI uses
  `desktop`** (verified from live process arguments); `web` exists but is not the live profile.
- `dsh --profile desktop --dump-config` is refused: *profile "desktop" is managed exclusively by
  the Electron application*.
- The `~/.dsh/profiles/node_modules` tree is stale `0.1.1-rc.2` material from an older layout — a
  version-skew hazard, not part of current compositions.

### 1.2 Plugin = Cordis plugin; distribution = bundle + profile

- A plugin is a TypeScript/JavaScript module exporting `name`, optional `inject`, and
  `apply(ctx, config)` (object/class forms also supported). It is loaded by Cordis (vendored
  `@deepseek-ai/cordis@4.0.5`), with Fiber lifecycle, automatic disposal of `ctx` registrations and
  HMR support.
- A **bundle** is an npm package whose `package.json` declares
  `"dsh": { "manifestVersion": 1, "bundle": { "patch": "./cordis.patch.yml" } }` (or an ordered
  array of patch files). The patch is a normal Cordis entry list (`insert:` rows, `id`-targeted
  overrides, `disabled`, `config`, `!!js` expressions).
- A **profile** is a pnpm project at `$DSH_HOME/profiles/<name>` listing `dsh.profile.bundles`.
  Layer order: bundle patches in list order → profile `cordis.patch.yml` → `$DSH_HOME/
  cordis.patch.yml` → `--patch` overlays. A later layer wins per row, and a patch **replaces a
  row's entire `config`** (no deep merge).
- `engines.dsh` is enforced at install/recomposition; incompatible versions are refused with an
  `allow-version … --accept-risk` escape hatch that writes `compatibility.json`.
- Host package operations are approval-gated (`plugin_manager` tool needs `danger-full-access` or
  approval); plugin code itself runs unsandboxed in-process.

### 1.3 Distribution paths available today

| Path | Status / notes |
| --- | --- |
| `dsh plugin --profile <name> add <pkg\|path\|git\|tarball>` | Works (verified in scratch home). For `desktop`: requires the app initialized the profile and is fully quit. Clipboard of pnpm build-script approvals applies to `prepare` scripts. |
| Web/GUI **Plugins** page | `plugin-manager` row + `dsh-client-ui-plugin-manager` ship in desktop/web profiles; installs update profile files and trigger recomposition/restart. Inferred from composition + launcher facts; not exercised end-to-end here. |
| `plugin_manager` agent tool | Present but **disabled outside Creator mode**; requires approval per call. |
| Plugin market (Discover) | `dsh-plugin-market` + market-index format exist, but the market bundle is **not enabled** in `desktop` or `web`; no index source configured. Optional future channel, not required for v1. |
| Source/local development | `link:`/`file:`/`npm pack` tarball all work; published registry installs are the production route. |

### 1.4 Extension points a govctl plugin would use

| Need | DSH 0.2.1-rc.5 mechanism | Key facts |
| --- | --- | --- |
| Model tools | `ctx.tools.register(definition)` or `defineTool` from `@deepseek-ai/dsh-tools` | Raw registration takes raw JSON-Schema `parameters`; only `output.schema` is validated at register; argument validation is `defineTool`'s job. `execute(args, exec)` gets `exec.agent`, `exec.signal`, `exec.arguments`. |
| Prompt policy | `ctx.systemPrompt.section({name, order, text})` | `text` may be a function; registered at root it is visible to every agent except a `complete:true` persona preset (minimal). First-party guidance orders 1000–5000, persona suffix 10200. |
| Dynamic session context | `ctx.systemPrompt.context({name, order, text})` + agent/session events | Contexts become sourced user-role snapshots; use `agent/created` / `agent/pre-step` to prepare text. |
| Skills | `ctx.skills.register({name, description, content})`; or `registerProvider`; or filesystem roots | Default roots: `<projectRoot>/.dsh/skills` (100), `.agents/skills` (200), custom dirs (300), `$DSH_HOME/skills` (400), `<agentsHome>/skills` (500), `bundledSkillDir` (600). `.claude/skills` is **not** a root. |
| Slash commands | `ctx.commands.register({name, description, input, handler})` | Handler runs against the agent without a model message; returns `{kind:'success'|'error', text}`. |
| Subprocess (govctl binary) | `ctx.subprocess` (`resolveExecutable`, `spawn`) or `ctx.shell` (sandbox-aware) | `ctx.subprocess` is unconfined host authority with env scrubbing; `ctx.shell`/`ctx.sandboxPolicy` is the way to honor session file policy. Node `child_process` works but loses env/termination/spill handling. |
| Approvals | `ctx.approval.request({agent, tool, reason, signal})`; `ctx.tools.guard`; `tools/pre-execute` `ask` | Tool calls are not auto-gated for file effects; the plugin owns its approval policy. |
| Agent presets | `@deepseek-ai/dsh-agent-preset` rows | Presets are YAML rows only — **no Markdown-defined agents**. Root rows are inherited; patching a shipped preset replaces its whole plugin list (brittle). |
| MCP | `dsh-mcp-client` row | Possible alternative integration path, but requires govctl to grow an MCP server (new CLI/API surface → RFC change). |

**Root-level vs preset visibility (important):** registries are scoped layers merged global +
ancestor chain, nearest wins. A bundle patch that inserts ordinary rows at root makes its tools,
prompt sections, skills and commands visible to the `standard` preset (the GUI default) and to
subagents. Consequences: unique tool names are required (the global layer already has `bash`,
`read`, `write`, `edit`, `glob`, `grep`, `skill`, `subagent`, …), and a root tool is visible in
`minimal`/`ptc` presets too, while a `complete` persona suppresses root prompt sections there.

### 1.5 What is gone or different vs the stale source checkout

The local checkout `/Users/liuxinyu/Projects/deepseek-harness` is **0.1.1-rc.2** and must not be
trusted as current. Notably it still contains `packages/hooks/{hook-protocol,hooks-claude-code,
hooks-codex}` and the `agent/session-start` event; the installed 0.2.1-rc.5 has neither. This is
the single largest porting trap for govctl's existing hook design.

---

## 2. What govctl brings to the integration

### 2.1 Governed artifact model (what the plugin must not bypass)

Seven artifact types: RFC, Clause, ADR, Work Item, Guard, Conformance Case, Release. RFCs are
normative; phases are `spec → impl → test → stable`; work items carry acceptance criteria and
guards; guards are executable completion gates. Non-negotiables already encoded in the CLI:
work-item closure guard conditions, `since` assignment on `spec→impl`, ADR alternatives before
acceptance, gov-root write locking, trunk-scope for `release`/`migrate`, and cross-workspace
artifact claims. **The plugin must call `govctl`, not reimplement these rules.**

### 2.2 CLI surface: what a tool wrapper can rely on

| Group | Commands | Machine-readable? |
| --- | --- | --- |
| Discovery | `describe` (JSON schema v1, parser-owned) | **Yes** — best entry point; no flags |
| Reads | `list`, `get`, `show`, `search`, `conformance trace`, `tag list`, `loop list` | **Yes** with explicit `-o json` (`get` also yaml/toml/plain; `show` needs explicit `-o json` because its default is always table) |
| Status | `status` | No — human table |
| Validation | `check`, `verify` | No — stderr prose + exit code; parse `E*/W*` codes if needed |
| Governed writes | `new`, `edit`, lifecycle verbs (`accept/reject/finalize/advance/bump/deprecate/supersede/delete`), clause ops, tag/claim writes | No structured success output; exit code + re-read via `get -o json` |
| Execution | `loop` family, `guard` family | `loop list` only |
| Agent integration | `agent doctor/install/update`, hidden `agent hook`, `init-skills` | `agent` is human prose; `init-skills` writes files |
| Maintenance | `self-update`, `completions`, `tui`, `migrate`, `release` | n/a |

- Exit codes: `0` success, `1` error diagnostic, `2` clap usage error.
- Diagnostics: stderr `error[E0101]: …` with stable families (E01xx RFC … E14xx agent, W…, I…);
  stdout carries only data — a deliberate agent-friendly channel split.
- Global flags: `--dry-run`, `-C/--config`; `--stdin` for rich text; `-n/--limit` only on list/search.
- **Gap:** no `--json`/`--quiet` error envelope and no machine-readable `status`/`check`. Fixing
  that in govctl would be new behavior (RFC change); the plugin must instead synthesize results
  from exit codes, stderr codes and follow-up `-o json` reads.

### 2.3 Shipped agent integration assets (the raw material to reuse)

- `.claude/` is the asset SSOT: 13 skills (`init`, `discuss`, `spec`, `gov`, `quick`, `commit`,
  `migrate`, `detach`, `rfc-writer`, `adr-writer`, `wi-writer`, `guard-writer`,
  `decision-analysis`), 4 reviewer/audit agents (RFC/ADR/WI reviewer, compliance checker),
  2 hook manifests (`claude.json`, `codex.json`).
- Packaging: `.claude-plugin/marketplace.json` (Claude marketplace) and `.codex-plugin/plugin.json`.
- `govctl agent install/update <codex|claude|all>` materializes the embedded assets into a
  user-scoped support path and delegates native install to third-party
  `agent-plugin-installer 0.6.1` (closed `AgentRuntime {Codex, Claude}` — the blocker for Path A).
- `build.rs` embeds skills/agents/hooks into the binary; `Cargo.toml` `include` ships them in the
  crate. `govctl init-skills [--format claude|codex] [--dir PATH]` projects skills+agents into a
  project (`.claude`/`.codex` defaults; `--dir .dsh` is the DSH-relevant option).
- Hook contract (the reusable protocol): stdin `{cwd, tool_input}`; stdout
  `{"hookSpecificOutput":{"hookEventName":…,"additionalContext":…}}`; SessionStart emits a bounded
  status summary (≤8 items/≤4000 chars, silent outside governed projects); PreToolUse on
  `Edit|Write`/apply_patch emits a **fixed advisory** for lifecycle-managed `gov/{rfc,adr,work,
  guard,conformance}/**` and `gov/releases.toml`, and never blocks. 5 s timeout in both manifests.
  RFC-0002 requires this to stay advisory, never an enforcement boundary.
- Runtime distribution: GitHub Releases assets ~8.2–10.9 MB per platform, cargo-binstall metadata;
  no local Rust toolchain in this environment, so no local build/startup measurement.

### 2.4 Reuse verdict

Reusable nearly as-is: skill bodies (normalize frontmatter/`$ARGUMENTS`), the SessionStart context
producer (call `govctl agent hook session-start` and parse `additionalContext`), the PreToolUse path
detection logic (reimplement over `tools/post-execute`), the read-only JSON CLI contract,
`describe` for discovery. Not reusable: hook manifests, reviewer-agent Markdown loading, Claude
marketplace packaging.

---

## 3. Options considered

| | Option | What it is | Governance cost | Verdict |
| --- | --- | --- | --- | --- |
| **A** | Native runtime in govctl (`govctl agent install dsh`, `--format dsh`) | govctl materializes DSH assets and installs/updates the plugin itself | RFC-0002 amendment (3.9.0) + ADR + WIs + minor release 0.23.0; blocker: `agent-plugin-installer` closed enum / upstream change | Defer; only if first-class support is a product goal |
| **B** | External DSH bundle `govctl-dsh` | npm bundle wrapping the govctl binary + shipping skills/prompt/commands/context | ADR + WIs + docs + release bookkeeping; **no RFC-0002 amendment** if scope test holds | **Recommended** |
| **C** | Skills/docs-only compatibility | `govctl init-skills --dir .dsh` and a short DSH guide; DSH scans `.dsh/skills` | Docs-only WI | **Do first** (free win), not a plugin |
| **D** | MCP server bridge | govctl grows an MCP server; DSH `dsh-mcp-client` consumes it | New CLI/API surface → RFC change; conflicts with govctl's stated "no MCP" design | Rejected |

Path B's scope test (from the governance analysis): the package must add **no govctl command
grammar**, change **no existing command behavior**, leave `govctl agent` semantics untouched, and
leave `init-skills` overwrite/destination semantics untouched. Under that boundary the RFCs do not
bind the package; an ADR is still required because govctl is endorsing a second install channel
(ADR-0061's "one façade" intent) and because asset SSOT/version alignment is a design decision.

---

## 4. Recommended architecture (Option B)

### 4.1 Package shape

```
govctl-dsh/                          # npm package, MIT, published by govctl-org
├─ package.json                      # dsh.bundle.patch, engines.dsh, peerDependencies, files
├─ cordis.patch.yml                  # root rows: govctl core plugin + skills provider (+ optional preset)
├─ presets/govctl.patch.yml          # OPTIONAL: a dedicated "govctl"/"governed" agent preset
├─ lib/index.js                      # plugin entry: tools, prompt, context, commands
├─ lib/tools.js                      # tool definitions wrapping the govctl CLI
├─ lib/skills.js                     # skill provider over assets/skills
├─ lib/run.js                        # binary resolution + ctx.subprocess invocation
└─ assets/skills/**/SKILL.md         # normalized copies of .claude/skills (SSOT: govctl repo)
```

`package.json` essentials:

```json
{
  "name": "govctl-dsh",
  "type": "module",
  "main": "./lib/index.js",
  "files": ["lib", "assets", "cordis.patch.yml", "presets"],
  "engines": { "node": ">=24", "dsh": ">=0.2.1-rc.5 <0.3.0-0" },
  "dsh": { "manifestVersion": 1, "bundle": { "patch": ["./cordis.patch.yml", "./presets/govctl.patch.yml"] } },
  "peerDependencies": {
    "@deepseek-ai/dsh-tools": ">=0.2.1-rc.5 <0.3.0-0",
    "@deepseek-ai/dsh-skill": ">=0.2.1-rc.5 <0.3.0-0",
    "@deepseek-ai/dsh-system-prompt": ">=0.2.1-rc.5 <0.3.0-0",
    "@deepseek-ai/dsh-commands": ">=0.2.1-rc.5 <0.3.0-0",
    "@deepseek-ai/dsh-subprocess": ">=0.2.1-rc.5 <0.3.0-0",
    "@deepseek-ai/dsh-llm": ">=0.2.1-rc.5 <0.3.0-0"
  }
}
```

Zero-dependency variant: the PoC proves a bundle can be plain ESM with **no** `@deepseek-ai/*`
imports (raw `ctx.tools.register`, no message factories). Start there; add peer deps only for the
modules that need typed factories (`dsh-llm` for `createUserMessage`, `dsh-tools` for `defineTool`).

### 4.2 Bundle patch (root rows — the minimal reliable pattern)

```yaml
# cordis.patch.yml
- insert:
    - id: govctl
      name: govctl-dsh
      config:
        binaryPath: govctl
        enablePolicySection: true
        enableSessionContext: true
        enableEditAdvisory: true
        mutationRequiresApproval: true
    - id: govctl-skills
      name: govctl-dsh/skills
```

Do **not** patch `preset-standard` by id: a patch replaces the whole `config.plugins` list, must
restate the shipped preset, and silently drifts across DSH versions. Root rows are inherited by
every preset; a dedicated preset row is an optional discovery convenience only (`presets/
govctl.patch.yml` inserting a `@deepseek-ai/dsh-agent-preset` row whose `config.plugins` is the
govctl composition — only if a distinguishable "governed" preset is worth the duplication).

### 4.3 Tool catalog

Read-only tools (no approval; deterministic, low risk):

| Tool | Backing CLI | Output treatment |
| --- | --- | --- |
| `govctl_describe` | `govctl describe` | JSON schema v1 → compact command/resource catalog for the model (also usable as a self-documenting reference) |
| `govctl_status` | `govctl status` | human text (summarize; no JSON contract) |
| `govctl_list` | `govctl list <resource> -o json` | JSON |
| `govctl_get` | `govctl get <resource> <id> [-f field] -o json/plain` | JSON/scalar |
| `govctl_show` | `govctl show <resource> <id> -o json` | JSON |
| `govctl_search` | `govctl search <query> -o json` | JSON |
| `govctl_check` | `govctl check` | exit code + stderr diagnostics (bounded) |
| `govctl_verify` | `govctl verify <WI>` | exit code + diagnostics |
| `govctl_trace` | `govctl conformance trace -o json` | JSON |

Mutating tools (approval-gated) — keep the set small to limit schema/token cost:

| Tool | Backing CLI | Policy |
| --- | --- | --- |
| `govctl_work` | `work new/edit/...`, accept/reject/delete/supersede | `ctx.approval.request` before execution; return CLI diagnostics |
| `govctl_rfc` | `rfc new/edit/finalize/advance/bump/deprecate` | approval; destructive verbs additionally require an explicit `confirm` argument |
| `govctl_adr` | `adr new/edit/accept/...` | approval |
| `govctl_clause` | `clause show/get/edit/add/remove/...` | approval for mutations |
| `govctl_run` | arbitrary argv escape hatch | approval always; intended for operations not yet typed |

Implementation notes: resolve cwd with `ctx.workingDirectory.ensure(exec.agent, exec.signal)`; spawn
through `ctx.subprocess.spawn({argv, cwd, stdio:{stdout:{maxBytes}, stderr:'pipe'}, graceMs,
signal})`; never use a shell; cap output (the CLI is chatty on stderr); surface the structured
diagnostic codes. For mutations that touch the workspace, consider confining through
`ctx.sandboxPolicy.resolve` + `ctx.sandbox.confine` (or run the whole tool through `ctx.shell`) so
the plugin does not bypass the session's file policy.

### 4.4 Skills

Ship the 13 `SKILL.md` files as package assets and register them with a small code provider (in-box
precedent: `@deepseek-ai/dsh-skill-badge` uses `ctx.skills.registerProvider` with
`resourceBase = {kind:'directory', path: fileURLToPath(new URL('../assets/', import.meta.url))}`);
`ctx.skills.register({name, description, content})` is the simpler embedded variant.

Porting rules:
- keep `name` (kebab-case) and `description`; drop `allowed-tools`/`argument-hint` (silently ignored
  by DSH, but dropping keeps the assets clean and unambiguous);
- replace `$ARGUMENTS` with wording (`the task the user named`), since DSH does not substitute it;
- replace Claude-specific tool references: `TodoWrite` → `todo`, `AskUserQuestion` →
  `ask_user_question`, `Read/Write/Edit/Glob/Grep/Bash` map to DSH's `read`/`write`/`edit`/
  `glob`/`grep`/`bash`; keep `govctl` invocations as-is (agents can run `bash`);
- do not use legacy camelCase invocation keys (DSH drops the whole skill);
- user-invocable skills become `/skill-name` slash commands automatically.

Alternative/free path: `govctl init-skills --dir .dsh` writes the same bundles into the project and
DSH's filesystem provider picks up `.dsh/skills` with no plugin at all. Use it as the docs quick win
(Option C) and as a fallback for users who prefer project-local assets.

### 4.5 Reviewer agents (4)

0.2.1-rc.5 has **no Markdown-defined agent mechanism** (`.claude/agents/*.md` is not loaded; presets
are YAML rows). Do not fake it. Options, preferred first:

1. **Reviewer skills**: port each agent prompt to a skill (`rfc-review`, `adr-review`, `wi-review`,
   `compliance-check`) whose instructions say to inspect the target artifacts and report findings;
   orchestrate with the built-in `subagent` tool when a separate context is wanted.
2. **Subagent prompts**: a `govctl_review` tool that spawns the DSH subagent provider with the
   reviewer prompt and the artifact IDs as input (more faithful isolation, more moving parts).
3. **A dedicated preset** per reviewer role (only if the workflow needs a genuinely different tool
   set).

Start with (1): lowest risk, no new DSH mechanism, and the review content is what matters. Record
the fidelity gap in the ADR.

### 4.6 Session-start context and edit advisory (hook parity)

DSH has no `agent/session-start`; reimplement the two govctl hooks natively.

**SessionStart** — two fidelity levels:
- **v1 (zero deps, what the PoC does):** on `agent/created`, kick off a background
  `govctl agent hook session-start` call (stdin `{"cwd": <cwd>, "tool_input": {}}`), cache
  `hookSpecificOutput.additionalContext` per session, and render it through
  `ctx.systemPrompt.context({name:'govctl:state', order:130, text:(c)=>cache})`. Creation must not
  await the subprocess (`agent/created` is serial and awaited); the first `agent/pre-step` waits
  for the pending refresh with a short bound instead.
- **v2 (full parity, peer dep on `@deepseek-ai/dsh-llm`):** on `agent/pre-step`, splice a
  `createUserMessage({content, source:{kind:'plugin', …}})` into `decision.messages` after the last
  claimed message (exactly the `dsh-agent-instructions` pattern), and refresh on significant state
  changes. This keeps durable, source-attributed history; v1's prompt context is lighter but less
  explicit.

**PreToolUse advisory** — DSH's interception point is `tools/pre-execute` (allow/deny/ask/cancel)
plus `tools/post-execute` (accept/replace/block + `additionalContexts`). The govctl hook is
advisory-only, so: allow the call, and in `tools/post-execute` attach the advisory context as a
`createUserMessage` when the edited `file_path` (or apply-patch `command`) is a lifecycle-managed
`gov/` path — never deny. That requires the `@deepseek-ai/dsh-llm` peer dependency; v1 can instead
rely on the standing policy section ("use govctl … edit; run check after direct edits"). Do **not**
turn this into enforcement: RFC-0002 explicitly says hook advice is a recoverability aid, not an
enforcement boundary. A monotonic `ctx.tools.guard` should be reserved for truly hard invariants
(if any are ever identified) and recorded in the ADR.

**Stop** — govctl has no Stop hook; nothing to port (and RFC-0002 forbids project-wide validation
after every edit).

### 4.7 Prompt policy section

One static `ctx.systemPrompt.section` (order 700, after plan/team policy 500/600, before per-tool
guidance 1000+), contributed only when the session cwd contains `gov/config.toml`:
RFC supremacy, phase discipline, no silent deviation, hard stops (no direct `gov/` edits, ask before
lifecycle operations, substantive work needs an active Work Item), and "call `govctl_status` first,
load the `gov`/`quick`/`spec` skill by situation". Keep it ≤ ~1–2 KB; it is paid on every request.
The `minimal` preset uses a `complete:true` persona and will suppress it (while root tools remain
visible) — acceptable, and worth a note in docs.

### 4.8 Binary provisioning and version alignment

- Resolve `govctl` in this order: plugin config `binaryPath` (absolute) → `ctx.subprocess.
  resolveExecutable('govctl')` (scrubbed PATH; beware Finder-launched GUI minimal PATH) → optional
  package-relative binary. On failure, return an actionable error naming the release page /
  `cargo binstall govctl`; do not vendor the ~8–11 MB binary in the default package (optional
  per-platform `optionalDependencies` later if UX demands).
- At session start or first tool call, run `govctl --version` and warn (not fail) when it is
  outside the plugin's tested range; keep a compatibility table. Assets are version-coupled: a
  plugin release must declare which govctl versions it targets.

### 4.9 Configuration surface

`binaryPath` (string, default `govctl`), `enablePolicySection`, `enableSessionContext`,
`enableEditAdvisory`, `mutationRequiresApproval` (all booleans, default true), `toolTimeoutMs`,
`maxOutputBytes`, `skills` (include/exclude list), optional `governedProjectMarkers`
(default `["gov/config.toml"]`). No Schemastery schema is required for the MVP (DSH passes the raw
`config` object); add a `Config` schema later for validation and defaults.

### 4.10 Security and approval model

- Plugin code runs with host authority, unsandboxed; installing `govctl-dsh` is therefore a trust
  decision equivalent to installing govctl itself. Document it.
- Read-only tools need no approval. Mutating tools must call `ctx.approval.request` and fail closed
  when no approval channel is available; never auto-approve lifecycle/destructive verbs.
- Governance semantics stay in Rust: the plugin is a transport, not a second implementation.

---

## 5. Distribution, asset SSOT, versioning

**Channel:** npm (`govctl-dsh`, or an org scope if one exists) + `files` with compiled ESM, patches
and assets; `npm pack` tarball is the offline fallback. Publishing from CI of the govctl repo keeps
the assets and the binary release in lockstep.

**Install UX (document exactly):**
- Desktop GUI: quit the app → `dsh plugin --profile desktop add govctl-dsh@<version>` → relaunch
  (or use the GUI Plugins page, pending end-to-end verification). CLI boot of `desktop` is refused,
  so do not document `dsh --profile desktop`.
- Web/headless/dev profiles: `dsh plugin --profile web add <path-or-package>` then
  `dsh --profile web --dump-config` to verify the layer.
- Uninstall/update: `dsh plugin --profile desktop remove govctl-dsh`; DSH reports
  restart-required cases; HMR recomposes config changes, but package installation is subject to
  restart rules.

**Asset SSOT (ADR decision):** `.claude/` remains the single source of truth. Add a generator
(npm script) that copies+normalizes `.claude/skills` (and reviewer prompts → skills) into
`assets/`, stamps the govctl version, and a CI check that fails on drift. This mirrors the
ADR-0033 lesson (a separate repository was rejected for asset drift) and resolves ADR-0061's
version-alignment requirement for bundled assets. Do not read DSH-side files from govctl's
materialized Claude support path (an RFC implementation detail).

**Versioning:** independent npm version for the plugin, but:
- `engines.dsh` pins the tested DSH minor (`>=0.2.1-rc.5 <0.3.0-0` for now);
- a `govctl` compatibility range is documented and checked at runtime;
- DSH releases that change any used API require a plugin release and a re-test; keep a small
  compatibility matrix in the README (tools smoke test can run with a stub binary that prints
  version/JSON fixtures — no LLM turn needed).

**Optional later:** publish an entry in a DSH plugin-market index (format 1) so Discover can
install it. This is a distribution convenience, not required for v1; the market bundle is disabled
in shipped profiles today.

---

## 6. Governance path

### 6.1 Path B (recommended): external package

Scope test: adds no govctl CLI grammar, changes no existing command behavior, leaves `govctl agent`
and `init-skills` semantics untouched. Then:

1. **ADR** (proposed → accepted): distribution channel; why no RFC amendment; resolution of
   ADR-0061's one-façade tension (docs present `govctl agent` as the Claude/Codex facade, the DSH
   bundle as a separate DSH-native channel); asset SSOT + version alignment; hook-equivalence
   policy (advisory, non-blocking, no repeated validation); reviewer-agent fidelity gap; binary
   provisioning policy.
2. **Work Items** (active → done): package scaffold + tools + skills + prompt/context + commands;
   approval-gated mutations; asset generator + CI drift check; tests (manifest/patch composition,
   tool schema/args, asset version equality, diagnostic mapping); docs.
3. **Docs + CHANGELOG**: README "Agent workflow", `docs/guide/` page for DSH, AGENTS.md §5/§6 if
   the skill table gains a porting note, install/uninstall/upgrade instructions.
4. **Release**: next minor (0.23.0) must also carry the two done-but-unreleased
   `WI-2026-10-18-*` items per `C-RELEASE-DEF`; publish npm package + release notes.
5. No RFC-0002 amendment while the scope test holds. If the package later adds a govctl command or
   changes behavior → Path A.

### 6.2 Path C (free quick win, docs-only)

`govctl init-skills --dir .dsh` + a short guide: DSH's filesystem skill provider scans
`<projectRoot>/.dsh/skills` at rank 100 with no plugin. Caveats: requires a loaded governed project;
skills are project-local (not user-global); content still needs the `$ARGUMENTS`/tool-name
normalization for best results. Treat as a tip, not a supported distribution claim, unless an ADR
is written.

### 6.3 Path A (defer): native runtime

Only if DSH must be a first-class `govctl agent` target:

1. Check `govctl claim list` (RFC-0010 claim before bump).
2. Amend RFC-0002 `C-AGENT-INTEGRATION` (runtime syntax, `all` semantics — keep `all` =
   Codex+Claude and add explicit `dsh` — shared behavior, DSH projection, runtime-specific hook
   manifests) and `C-GLOBAL-COMMANDS` items as needed; edit content, then
   `govctl rfc bump RFC-0002 minor` (→ `spec`), review, `govctl rfc advance RFC-0002 impl` (seal).
3. New ADR; Work Items; implementation; tests (`tests/test_agent_plugin.rs`, hook/path tests);
   `just pre-commit`; release 0.23.0; docs truth pass.
4. Blocker: `agent-plugin-installer 0.6.1` has a closed `AgentRuntime {Codex, Claude}` and
   hard-coded native mappings; `src/cmd/agent.rs` also assumes two bundled manifests. Resolve via
   upstream support, govctl-side orchestration, or a new internal module — a decision the ADR must
   record.

### 6.4 Repo state and authorization

- govctl 0.22.0 released; two done-but-unreleased WIs; 272/272 WIs done; no active/queued WIs; 10
  RFCs all normative (RFC-0002 `impl` v3.8.0 sealed); 0 proposed ADRs/draft RFCs. Research and
  spikes are allowed now; implementation needs authorization.
- User approval needed before: ADR acceptance, new Work Items, release cut/tag, any `.claude/`
  asset changes (they ship to every bundled integrator), and any RFC lifecycle verb.
- Skipping governance risks: an unversioned RFC-0002 amendment (detectable, blocks later
  progression), a compliance-audit failure under Law 1, and release/CHANGELOG drift.

---

## 7. Proof of concept — what was validated

`research/poc/govctl-dsh-plugin/` is a zero-dependency bundle (`package.json` +
`cordis.patch.yml` + `index.js` + 2 ported skills) implementing: 5 tools, a policy prompt section,
session-state `systemPrompt.context` fed by `govctl agent hook session-start`, 2 embedded skills,
2 slash commands, and managed-edit detection.

Validated on 2026-10-31 with a scratch `DSH_HOME=/tmp/dsh-poc-home` (real profiles untouched):

1. `dsh --profile web --dump-config` auto-initialized the web template.
2. `dsh plugin --profile web add <poc-path>` → pnpm link; profile gained
   `"govctl-dsh-plugin": "link:…"` and appended `govctl-dsh-plugin` to `dsh.profile.bundles`.
3. `dsh --profile web --dump-config` printed the third-party layer `# == govctl-dsh-plugin` and
   the inserted root row (`id: govctl`).
4. `dsh web --no-open --port 59631` booted; log showed
   `[govctl-dsh-plugin] loaded (binary: govctl)`, `registered skill "gov"`, `registered skill
   "quick"`, then the served URL. The process was stopped immediately; no model turn was run.

This confirms the packaging contract, root-row visibility at composition level, plugin load,
skill registration and config passing. Not yet validated end-to-end: a live model tool call, the
desktop-profile install, and the `additionalContexts` advisory (needs the `dsh-llm` peer dep).
See the PoC README for the exact commands, API mapping and known gaps.

The DSH platform researcher reviewed the PoC against installed sources; the review's top fixes were
applied: stdin is now written for `agent hook` calls, session creation is no longer blocked by the
subprocess, `govctl_run` returns a `tools/pre-execute` `ask` decision (approval-gated), managed-edit
detection moved from pre-execute to post-execute logging, and `engines.dsh` gained an upper bound.
Remaining production deltas (`ctx.subprocess` instead of `execFile`, `createUserMessage`-based
context/advisory, the real desktop install) are listed in the PoC README.

---

## 8. Delivery roadmap

| Phase | Content | Governance | Acceptance |
| --- | --- | --- | --- |
| **P0** | Docs-only: DSH quickstart using `govctl init-skills --dir .dsh`; note `$ARGUMENTS`/tool-name normalization | Docs WI (or folded into P1) | A user can see govctl skills in a DSH session in a governed repo; no new package |
| **P1** | `govctl-dsh` bundle scaffold: manifest/patch, read-only tools over `describe`/`list`/`get`/`show`/`search`/`check`/`verify`, policy section, 13 embedded/ported skills, slash commands, binary resolution + version warning | ADR accepted; WIs | `dsh --profile <test> --dump-config` shows the layer; boot loads plugin; tools appear in a session; skills visible; `govctl check` still passes in this repo |
| **P2** | Session context (v2 `agent/pre-step` + `dsh-llm`), edit advisory via `tools/post-execute`, approval-gated mutating tools, reviewer skills (`rfc-review` etc.) | WIs | Context injected once per session with source attribution; advisory appears after a direct `gov/` edit and never blocks; mutations require approval |
| **P3** | Distribution: npm publish from CI, asset generator + drift check, docs (desktop/web/headless/dev install, uninstall/update), compatibility matrix, tests | WIs + release 0.23.0 | `dsh plugin --profile desktop add govctl-dsh@x` (app quit) works; CI fails on asset/version drift |
| **P4** (optional) | Native runtime `govctl agent install dsh` (+ market index entry) | RFC-0002 3.9.0 + ADR + WIs + release | `govctl agent install dsh` passes doctor/install/update; version equality enforced |

---

## 9. Risks and open questions

**Risks**

1. **DSH API churn.** The installed version is a release candidate; the previous line removed a
   whole hook subsystem. Mitigate with `engines.dsh`, peer ranges, a smoke-test matrix and fast
   plugin patch releases.
2. **Desktop install UX unverified end-to-end.** The profile is app-owned; document the quit-first
   CLI path and validate the GUI path before claiming support.
3. **Root-global visibility side effects.** Tools appear in `minimal`/`ptc` presets too, and a
   `complete` persona hides the policy prose while tools remain — document, or optionally restrict
   per preset later.
4. **Machine-readable gaps.** `status/check/verify` are human-only; the plugin must synthesize.
   Adding `--json` diagnostics to govctl is new behavior (RFC change) — defer it.
5. **Binary provisioning.** Finder-launched GUI PATH may be minimal; version mismatch between
   plugin assets and installed binary must warn, not silently mislead.
6. **Asset drift.** Skills copied from `.claude/` can diverge; needs generator + CI hash check and
   a version stamp (ADR-0033 precedent).
7. **Reviewer-agent parity.** DSH has no Markdown agents; skill-based reviewers are a fidelity
   compromise that should be recorded.
8. **Security/approvals.** Unsandboxed in-process plugin + artifact mutations: approval design is
   part of the product, not an afterthought.
9. **Governance drift.** Two governance artifact sets: skills shipped by the plugin and assets
   embedded in govctl. The ADR must state which is authoritative and how they are reconciled.
10. **`desktop` profile ownership.** The Desktop shell owns package operations; a plugin upgrade
    flow that assumes CLI semantics may not match desktop behavior.

**Open questions for the user (from the governance analysis)**

1. Ownership/scope: official govctl deliverable vs third-party package?
2. Phase 1: Path B only, or also authorize Path A later?
3. Publish under what npm name/scope?
4. Is a 0.23.0 minor release (also carrying the two unreleased WIs) acceptable, and who approves
   lifecycle verbs?
5. May `all` stay Codex+Claude (recommended) if Path A ever happens?
6. Which reviewer-agent parity level is required: skills, subagent prompts, or presets?
7. Should the plugin warn or refuse when the installed govctl version is outside its tested range?
8. Is a DSH plugin-market index entry planned, or npm + docs enough?
9. Is the `additionalContexts` advisory (v2, peer dependency) required for v1, or is the standing
   policy section enough?
10. Should the plugin support `desktop`, `web`, and `headless` profiles equally, or desktop only?

---

## 10. Evidence index

- DSH platform evidence: [`research/dsh-platform-notes.md`](dsh-platform-notes.md) — §1 environment
  and profiles; §2 packaging/manifest/patch; §3 scope model; §4 extension point APIs; §5 subprocess
  and sandbox; §6 distribution/desktop; §8 constraints; §10 verified commands; §11 version skew;
  §12 installed-ASAR evidence paths.
- govctl capability evidence: [`research/govctl-capabilities.md`](govctl-capabilities.md) — §2 CLI
  surface and `-o json` verdict (`C-OUTPUT-FORMAT`); §3 shipped agent integration and hook
  contract; §4 skill/agent inventory; §5 distribution/runtime; §6 extension seams for a `dsh`
  runtime (including the `agent-plugin-installer 0.6.1` blocker).
- Governance evidence: [`research/governance-constraints.md`](governance-constraints.md) — §2 cited
  RFC-0002 clauses; §3 ADR-0061 boundary; §4 prior art; §5 Paths A/B/C and authorization checklist;
  §6 skip-governance risks.
- PoC: [`research/poc/govctl-dsh-plugin/README.md`](poc/govctl-dsh-plugin/README.md) — validated
  install/dump/boot transcript, API mapping, gaps.
- Installed runtime references: `/Applications/DeepSeek Harness.app/Contents/Resources/
  runtime/cli/bin/dsh`; `app.asar/dsh/node_modules/@deepseek-ai/dsh-{package-manifest,app-boot,
  plugin-manager,plugin-market,scope,tools,skill,skill-filesystem,system-prompt,commands,
  agent-preset,agent-preset-registry,subprocess,shell,agent-instructions,agent,llm,
  working-directory}/README.md`; `dsh-base/cordis.patch.yml`; `dsh-web-app/presets/standard.patch.yml`.
- Stale but useful source checkout: `/Users/liuxinyu/Projects/deepseek-harness` (0.1.1-rc.2) —
  version-skewed; not authority for installed behavior.
