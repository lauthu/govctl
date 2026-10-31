# DSH platform notes — how a third-party bundle plugs into DeepSeek Harness (installed 0.2.1-rc.5)

Research date: 2026-10-31. Authoritative target: the **installed** DSH app at
`/Applications/DeepSeek Harness.app` (release `0.2.1-rc.5`, host protocol 4, Node 24.21.0,
pnpm 11.28.5 — `app.asar/dsh/desktop-runtime.json`). Everything below was read from the
installed ASAR or observed by running the bundled CLI; where the older checkout
`/Users/liuxinyu/Projects/deepseek-harness` (version `0.1.1-rc.2`) disagrees it is marked
as skew. Installed-package paths are abbreviated: `<PKG>` = `dsh/node_modules/@deepseek-ai/<pkg>/`
inside `/Applications/DeepSeek Harness.app/Contents/Resources/app.asar` (read with
`python3 /tmp/asar.py`). Docs cited as `<PKG>/README.md` are shipped inside the npm
packages, not the source-repo `docs/` tree.

## 0. TL;DR — the shape that works for govctl

1. Ship one npm package, e.g. `govctl-dsh`, that is a **bundle**: `package.json` declares
   `dsh.bundle.patch` (one file or an ordered array), `dsh.manifestVersion: 1`,
   `engines.dsh: ">=0.2.1-rc.5 <0.3.0-0"`, and `peerDependencies` on every
   `@deepseek-ai/dsh-*` package its code imports.
2. The bundle patch **inserts ordinary root-level rows**: one plugin row that in `apply(ctx)`
   calls `ctx.tools.register(...)`, `ctx.systemPrompt.section(...)`,
   `ctx.commands.register(...)`, `ctx.skills.registerProvider(...)` (or `register`), and
   spawns the native `govctl` binary through `ctx.subprocess`. Root-level registrations land
   in the registries' global layer and are inherited by every agent, including the
   `standard` Agent preset used by the Web/Desktop GUI. **No preset duplication is needed.**
3. Optionally ship a second patch file that inserts a `@deepseek-ai/dsh-agent-preset` row if
   govctl wants its own preset entry in the preset picker; do not restate shipped presets.
4. Skills: the most robust contribution is a small code provider (in-box precedent
   `@deepseek-ai/dsh-skill-badge`) that reads `SKILL.md` files from the package via
   `new URL('../assets/…', import.meta.url)`; filesystem roots are an alternative for
   project/user skills (`<projectRoot>/.dsh/skills`, `<projectRoot>/.agents/skills`,
   `customSkillDirs`, `bundledSkillDir`). Claude-style frontmatter loads if the file is found:
   `name`+`description` are required, `whenToUse`/`metadata`/`disable-model-invocation`/
   `user-invocable` are honored, unknown keys such as `allowed-tools`/`argument-hint` are
   silently ignored.
5. Install into the **desktop** profile (the profile the running GUI uses) either from the
   GUI's **Plugins** page or with `dsh plugin --profile desktop add <spec>` while the app is
   fully quit. CLI boot of `--profile desktop` is refused by design; only the Electron app
   loads it. HMR recomposes live on config/plugin changes; Desktop owns package operations.

## 1. Installed environment, package inventory, profiles

- CLI: `/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin/dsh` is a sh
  wrapper that runs
  `ELECTRON_RUN_AS_NODE=1 …/MacOS/DeepSeek Harness --expose-internals app.asar/dsh/node_modules/@deepseek-ai/dsh-desktop-host/lib/cli.js`.
  `dsh --version` → `0.2.1-rc.5`.
- All `@deepseek-ai/dsh-*` packages in the ASAR are `0.2.1-rc.5`; vendored Cordis is
  `@deepseek-ai/cordis@4.0.5`, loader `cordis-plugin-loader@1.0.6`, group `1.0.5`,
  include `1.0.10` (from `dsh/package.json` + per-package `package.json` in ASAR).
- ~256 installed `@deepseek-ai/*` package dirs are present in the ASAR, including
  `dsh-agent-instructions`, `dsh-persona` and `dsh-tool-cordis`. **There is no
  `dsh-hooks-claude-code`, `dsh-hooks-codex`, or `dsh-hook-protocol`** (the old checkout's
  `packages/hooks/` trio is absent; see §4.8).
- Profiles live at `$DSH_HOME/profiles/<name>` (`DSH_HOME` defaults to `~/.dsh`; a scratch
  `DSH_HOME` is honoured — verified, see §10.5). Each profile is a pnpm project containing
  `package.json` (with `dsh.profile.bundles`), `cordis.patch.yml`, `cordis.yml`,
  `pnpm-workspace.yaml` (`nodeLinker: hoisted`, `autoInstallPeers: false`), `node_modules/`,
  and `.plugin-manager/` state/logs once managed.
- **The running GUI uses the `desktop` profile**, not `web` (verified from the live process
  arguments: `…/dsh-desktop-host/lib/index.js <app.asar/dsh> /Users/liuxinyu/.dsh/profiles/desktop
  <runtime/primary-runtime> <…/pnpm.mjs> <runtime/bin>`). `desktop/package.json` bundles:
  `dsh-base`, `dsh-web-app`, `dsh-experimental-agent-team-profile`, `dsh-experimental-auto-review`,
  `dsh-experimental-voice-input-bundle`, `dsh-experimental-inspector-profile`; its
  `cordis.patch.yml` additionally pins `agent-default-model` to
  `deepseek-official/deepseek-flash` and several UI settings.
- `~/.dsh/profiles/web` bundles only `dsh-base` + `dsh-web-app` (207-byte package.json, empty
  deps). `~/.dsh/profiles/node_modules/` contains stale `0.1.1-rc.2` packages from an older
  DSH layout — see §11.

## 2. Bundle / plugin packaging contract (Q1)

### 2.1 `package.json` `dsh` manifest

From `<dsh-package-manifest>/README.md` (type declarations only; each reader parses itself):

```jsonc
{
  "name": "govctl-dsh", "version": "0.1.0", "type": "module",
  "main": "lib/index.js",
  "exports": { ".": "./lib/index.js", "./cordis.patch.yml": "./cordis.patch.yml",
               "./package.json": "./package.json" },
  "engines": { "dsh": ">=0.2.1-rc.5 <0.3.0-0" },
  "dsh": {
    "manifestVersion": 1,
    "bundle": { "patch": ["./cordis.patch.yml", "./presets/govctl.patch.yml"] },
    "client": { "platform": "web", "inject": ["@deepseek-ai/dsh-client-ui-tool"], "external": [] }
  }
}
```

- `dsh.manifestVersion`: format identifier, currently `1`; **declarative only — loaders do not
  enforce it** (`<dsh-package-manifest>/README.md`, "Compatibility is declarative").
- `dsh.bundle.patch`: one path or an **ordered list**, each relative to the package root; the
  launcher concatenates the lists in order into **one bundle layer**
  (`<dsh-package-manifest>/README.md`; `<dsh-app-boot>/README.md#profiles`; shipped example
  `dsh-web-app/package.json` with 5 files).
- `dsh.client`: optional browser half; validated by `dsh-client-modules` (see §7).
- `engines.dsh`: SemVer range, prereleases allowed; this is the **admission** mechanism
  (see §2.5). If present, `peerDependencies` are ignored for compatibility; if absent, every
  `@deepseek-ai/dsh` / `@deepseek-ai/dsh-*` peer range must match the single runtime version
  (`<dsh-app-boot>/README.md#profiles`).
- Display metadata (Plugins page cards) comes from exported `./locale/*.json` `meta`, falling
  back to `package.json.description`, plus `icon`/`./icon` (`dsh-app-boot/lib/index.js`
  `readPluginMeta`, `resolvePluginResource`; `<dsh-client-ui-plugin-manager>/README.md`).
- A bundle package needs **built JS** (`main`/`exports` must point at loadable ESM). There is
  no build step at install time unless the package itself declares install scripts; avoid
  scripts because pnpm 11 blocks dependency build scripts by default (see §6.5).

### 2.2 Profile composition and patch layers

`dsh --help` and `<dsh-app-boot>/README.md#profiles`:

1. CLI option layers (defaults → `$DSH_HOME/.env` → invoking-dir `.env` → inherited env) are
   loaded first.
2. Bundle layers: for each name in `package.json` → `dsh.profile.bundles`, in order, apply
   that bundle's patch list(s) in order. Bundles resolve from the installation's package set
   and the ordered bundle dependency graphs; incompatible/unreadable bundles are **skipped**
   (listed in `skippedBundles`, printed at start) without failing the profile.
3. The profile's own `cordis.patch.yml`.
4. The home-level `$DSH_HOME/cordis.patch.yml` (**outranks** the per-profile patch).
5. CLI `--patch <path>` overlays (repeatable) — applied after the profile layer; "Home and
   invocation patches retain their higher priority than Plugin Manager writes"
   (`<dsh-plugin-manager>/README.md`).
6. Then `dsh-hmr` (when enabled) recomposes on profile/home patch edits and Plugin Manager
   writes; without HMR changes need a restart.

The profile root `cordis.yml` is an empty entry list by design; the tree is composed from
patches ("Edit cordis.patch.yml, not this file" — `~/.dsh/profiles/web/cordis.yml`).

### 2.3 Patch entry format

A patch file is a top-level YAML **array** of loader patch entries (empty/comment-only file
fails boot; use `[]` to disable). Two entry shapes (`<dsh-base>/cordis.patch.yml`,
`<dsh-web-app>/cordis.patch.yml`, `<dsh-app-boot>/README.md`):

```yaml
# 1) target an existing row by id: replace whole config / change disabled / assert name
- id: system-prompt
  name: '@deepseek-ai/dsh-system-prompt'     # optional assertion
  disabled: false
  config: { personaPrefix: 'You are …' }     # REPLACES the row's complete config
# 2) insert new rows (also allowed inside nested groups)
- insert:
    - id: govctl
      name: 'govctl-dsh'                     # package specifier, subpath, abs path, file: URL
    - id: preset-govctl
      name: '@deepseek-ai/dsh-agent-preset'
      config: { id: govctl, plugins: [ … ] }
```

- `config` is **replaced, not merged** — restate every key the row owns (shipped patches say so
  explicitly). A patch naming a non-existent entry prints a stderr warning.
- `!!js` expressions are allowed anywhere in values and are evaluated at mount with
  `new Function("ctx","expr", "with (ctx) { return eval(expr) }")` and eager child-list
  interpolation (`cordis-plugin-loader/lib/index.js`). Available names: the loadering context
  (`ctx`, hence `ctx.get('profileContext')`, services), `dshHomePath(...)` (provided by app boot:
  `app-boot/lib/index.js` → `ctx.provide("dshHomePath", dshHomePath)`), and ordinary globals
  (`process`, `URL`, …). **`import.meta`/`require` are not available**, so a package-relative
  path cannot be expressed there — use a code provider or an env/`dshHomePath` expression.
- Carriers: `name: cordis:group` + `group: true` and `config:` = child entry list (native
  group, used for isolation realms, e.g. `isolate: { compaction: true }`); `cordis:include`
  includes another entry-list file; `cordis:group` / `cordis:include` are the only recognized
  native child carriers.
- Inserted `name`s may be absolute filesystem paths, file URLs, or package specifiers; patch
  loading converts absolute and patch-relative `./`/`../` names to file URLs inside `insert`
  rows and nested groups; existing-entry name assertions and replacement `config` values stay
  literal (`<dsh-app-boot>/README.md#profiles`).

### 2.4 Row/plugin name resolution

- The launcher computes one immutable **runtime resolution** from the installation and the
  ordered bundle dependency graphs and installs it as Node ESM/CJS resolution hooks; rows are
  imported through it (`<dsh-app-boot>/README.md` "Understand the implementation").
  In practice: `@deepseek-ai/dsh-tools` and other installation packages resolve from a
  profile-installed plugin row, and profile-installed packages resolve from the profile's
  node_modules.
- **Linked packages are special**: a profile dependency installed as `link:` to a directory
  outside `$DSH_HOME/profiles/...` gets a "linked root" interception layer. A bare import of a
  runtime package from inside that linked package resolves only if the linked package's own
  `package.json` `peerDependencies` names the imported runtime package
  (`dsh-app-boot/lib/index.js` `routeLinked` + `readPeerNames`: at each ancestor
  `…/node_modules` position it checks `dirname(searchPath)/package.json` peers). Empirically:
  a link-installed probe bundle failed `Cannot find package '@deepseek-ai/dsh-tools' imported
  from /private/tmp/…/lib/tool.js` until `@deepseek-ai/dsh-tools` was added to its
  `peerDependencies`; then the module resolved and its config schema was read. Registry-
  installed copies under the profile resolve through the profile layer instead, but declaring
  peers is still the portable choice.
- `--dump-config` prints the composed tree without importing modules; `--dump-config-schema`
  imports each composed module to read its Config (trust required). Preset rows
  (`@deepseek-ai/dsh-agent-preset`) make `--dump-config-schema` report
  `unrecognized Loader tree carrier; use cordis:group or cordis:include …` — the shipped web
  profile logs four such errors (`/187–/190`) for its own presets, so this is a tooling
  limitation, not a packaging error.

### 2.5 `engines.dsh` admission and exact-version exemptions

- Checked before import: with `engines.dsh`, that range alone decides; otherwise every
  `@deepseek-ai/dsh*` peer must match; empty/invalid ranges are incompatible; prereleases
  participate. A denied row becomes a detached `disabled: true` row (no import); an
  incompatible **bundle** is skipped and listed in `skippedBundles`.
- Plugin Manager checks an install **before pnpm runs** for registry/path specs (registry spec
  resolved through `pnpm view` to read that version's `engines`/peers); git/tarball specs are
  judged after install and rolled back on failure (`<dsh-plugin-manager>/README.md`
  "Version compatibility and exemptions").
- Exemptions: `<profile>/compatibility.json`, mapping exact `name@version` →
  exact DSH runtime versions. CLI:
  `dsh plugin --profile <p> version-exemptions`,
  `dsh plugin --profile <p> allow-version <name@version> --dsh-version <runtime> --accept-risk`,
  `… revoke-version …`. Grants apply on next composition; not inherited by upgrades.
- Verified end-to-end on a scratch profile: incompatible probe bundle refused with code/message
  `incompatible-version` and no files changed; after `allow-version`, re-install succeeded and
  `compatibility.json` contained `{"govctl-dsh-probe-bad@0.1.0":["0.2.1-rc.5"]}` (§10).

### 2.6 Bundle vs profile vs overlay vs dependency

- **Bundle** = a package with `dsh.bundle.patch` selected in `dsh.profile.bundles`; its patches
  are a layer. A "profile bundle" (e.g. `dsh-experimental-agent-team-profile`) is just a bundle
  whose patches compose a surface; there is no separate type.
- **Plain dependency** without `dsh.bundle.patch` is not a plugin and is refused by Plugin
  Manager installation ("Only bundles are managed" — `<dsh-client-ui-plugin-manager>/README.md`);
  it may still be an ordinary dependency of a bundle.
- **Overlay** = the profile patch / home patch / `--patch` layers; the way to modify a bundle's
  rows without editing the package.
- **Plugin** = a Cordis module loaded by a row (`name` field). One bundle may insert many rows
  and may itself export plugin code (`main`/subpaths).

## 3. Scope / visibility model — root rows vs preset rows (Lead question 1)

Registries are built on `@deepseek-ai/dsh-scope`'s `ScopedLayers` ("host+per-scope shape",
stated for tools, skills, systemPrompt, commands, tools restrictions):

- A registration is filed into the layer of **the calling context's scope**
  (`scopeOf(ctx)`). A root/host context has no scope → **global layer**. An agent scope
  (`agent.ctx`) or a preset's standing mount has a scope → that scope's layer.
- A read merges **the global layer plus the viewing scope's ancestor chain**
  (`chainLayers(scope)`), nearest layer winning duplicates. `dsh-scope` README: "Child scopes
  inherit ancestor contributions, with the nearest definition taking precedence, while
  ancestor scopes can observe descendant activity; neither relationship works in reverse."
- `dsh-skill` implementation comment: "host rows and repository plugins land in the global
  layer, while a plugin mounted by an agent preset's standing composition lands in that
  preset's layer. A read merges the global layer with the viewing scope's chain."
- `dsh-tools` `view(scope)`: `const inherited = new Map(this.layers.global.tools.entries())`
  then overlays each `layer.tools` on the chain, then applies per-layer restrictions to the
  inherited set; the scope's OWN layer registrations are always visible and are not filtered
  by its own restrictions. `register` throws for a same-name duplicate **within one layer**.
- `dsh-system-prompt`: sections/variables/tools registered in a scope shadow globals;
  globals remain visible to every agent. `dsh-commands`: "A plain registration is global";
  agent-scoped registrations shadow global commands for that agent.

Consequences for a third-party bundle:

1. **Root-level rows are model-visible to standard-preset agents** (and to subagents, whose
   scope joins the parent's preset revision). A plugin inserted by a bundle patch at root that
   registers a tool, a prompt section, slash command or skill provider appears without touching
   `preset-standard`. This is the minimal reliable pattern.
2. Root registration is **process-global**: visible to every preset, including `minimal`, `ptc`,
   `cordis`, in every session and subagent. Design tool names as unique capabilities (the base
   composition already occupies `bash`, `read`, `write`, `edit`, `glob`, `grep`, `skill`,
   `subagent`, …). A preset-local registration of the same name would shadow it; a root row
   with a duplicate name in the global layer throws on mount.
3. A preset that restricts tools (`ctx.tools.restrict({allow|deny})`) filters inherited
   (global + ancestor) tools but keeps its own; no shipped preset restricts the sample tools
   except through its own composition. A `complete: true` persona (`preset-minimal`) makes its
   own prompt the whole system prompt, so root prompt sections are suppressed **for that
   preset** while root tools/skills remain visible.
4. **Model-visible ⟺ the agent's composed tree**, so a per-preset design must either insert a
   new preset row (users select it; default stays `standard`) or patch `preset-standard` by id.
   Patching replaces the whole `config` — you must restate the entire shipped `plugins` list,
   and the registry README states the known limitation: "A user override replaces the complete
   child list and does not automatically merge future changes to the builtin list." Prefer
   root rows unless you truly need a distinct preset.
5. A bundle **can** ship presets: insert `@deepseek-ai/dsh-agent-preset` rows from any patch
   file (the shipped `dsh-web-app` does exactly this from `presets/*.patch.yml`). Presets are
   YAML/plugin rows only — the registry "neither scans directories nor accepts preset paths";
   there is **no Markdown-defined agent/preset mechanism** in 0.2.1-rc.5 (the `.claude/agents`
   Markdown agents used by Claude Code are not loaded by any installed package).

## 4. Extension points with exact API (Q2, Q3)

### 4.1 Tools — `ctx.tools` (`<dsh-tools>/README.md`, `lib/index.js`)

Two authoring paths:

**(a) `defineTool` (typed DSL + automatic argument validation).**

```js
import { defineTool } from '@deepseek-ai/dsh-tools'
ctx.tools.register(defineTool({
  name: 'govctl_status',
  description: 'Show governed artifacts and lifecycle state.',
  parameters: {                    // per-property DSL map, NOT raw JSON Schema
    artifact: { type: 'string', required: true, description: 'RFC/ADR/WI id' },
    json: { type: 'boolean' },
  },
  output: {
    schema: { type: 'string' },    // DSL value schema
    render: (_args, value) => [{ type: 'text', text: value }],
    // optional presentationMeta(args, value) → JSON for UI
  },
  async execute(args, exec) { return await runGovctl(args, exec) },
}))
```

`defineTool` compiles the DSL with `parameterSchemaSpecToJsonSchema` and installs a validating
`execute` wrapper; definitions may add `timeoutMs` (declarative only), `deferLoading`,
`isConcurrencySafe`, `finalizeContent`, `projectContent`, `presentCall`/`presentResult`.

**(b) Raw `ctx.tools.register(definition)` — answer to Lead question D.** `register()` accepts a
plain definition object; it validates only:
`output` must be `{ schema, render, presentationMeta? }`; `output.schema` must pass
`assertSupportedJsonSchema`; `timeoutMs` positive; name ≠ reserved `run_code`. It does **not**
validate `parameters` at registration and does **not** validate model arguments against
`parameters` — that is `defineTool`'s wrapper. Raw shape accepted:

```js
ctx.tools.register({
  name: 'govctl_status',
  description: '…',
  parameters: {                          // raw JSON Schema, object-rooted
    type: 'object',
    properties: { artifact: { type: 'string', description: '…' } },
    required: ['artifact'],
    // additionalProperties: false optional (boolean only)
  },
  output: { schema: { type: 'string' }, render: (_a, v) => [{ type: 'text', text: v }] },
  async execute(args, exec) { /* args is a deep-frozen lossless-JSON snapshot */ },
})
```

Enforced raw-schema subset (`dsh-tools/lib/index.js` `CONSTRAINT_KEYWORDS`/`SCHEMA_TYPES`):
one scalar `type` from `object|array|string|number|integer|boolean|null`, or `oneOf`
(exact-one), plus `properties`, `required`, `additionalProperties` (boolean), `items`,
`enum`/`const` (scalar, type-matched), and annotation keywords `description|title|default|
examples`. Unsupported keywords are rejected when checked (outputs always; parameters only by
`assertSupportedJsonSchema` if you call it). Argument handling: raw definitions receive
`exec.arguments` snapshotted/deep-frozen (lossless JSON required); no `INVALID_ARGS`
normalization runs — validate yourself (exported `validateJsonSchemaValue(schema, value,
"value")` is available, as are `assertSupportedJsonSchema`/`assertObjectJsonSchema`). Output
values are validated against `output.schema` at materialization.
Tool `exec` object: `name`, `arguments`, `callId`, `rootCallId`, `token`, `agent`, `parent`,
`schema`, `signal`, `deferContext(ctx)`, `concludeTurn()`.

Policy hooks/events: `tools/pre-execute` (waterfall returning `{kind:'allow'|'deny'|'ask'}`),
monotonic `ctx.tools.guard(fn)` (return string = deny), `tools/execute` (around dispatch),
`tools/post-execute` (replace/block content), `tools/result` (observe final). Approvals route
through `ctx.approval.request(...)` (see §5). Per-agent visibility: `ctx.tools.restrict({allow,
deny})` from an agent scope; `get(name, scope)`, `schemas(scope)`; `presentAs('ptc')`.
**Code Mode (PTC)**: `tools` row config `mode: native|ptc`; under `ptc` the model sees only
`run_code` plus a generated SDK and can call tools as `await tools.<name>(args)` inside the
program; model-direct calls to other tools resolve `UNKNOWN_TOOL`. A tool row's definition is
unchanged, but ordering/serialization go through the PTC runtime
(`@deepseek-ai/dsh-ptc-runtime-node` is mounted at root). GUI currently selects native by
default (`DSH_TOOLS_MODE` env seam in the web patch).

### 4.2 Skills — `ctx.skills` + provider (`<dsh-skill>`, `<dsh-skill-filesystem>`, `<dsh-tool-skill>`)

- Registry: `ctx.skills.registerProvider(createFn)` where `createFn(control)` returns
  `{ name, list(), get(candidate) }`; `control.signal` + `control.invalidate()`. Root
  registration → global layer → visible to all agents. Reserved provider name `runtime`.
- Embedded skills: `ctx.skills.register({ name, description, content, invocation?,
  resourceBase?, source?, rank?, locator? })` (validated name/description; defaults
  `invocation: {modelInvocable:true, userInvocable:true}`, `provider: 'runtime'`).
- In-box precedent for a bundle shipping skill content:
  `<dsh-skill-badge>/lib/index.js` — `apply(ctx){ ctx.skills.registerProvider(() => provider) }`
  with `resourceBase = { kind:'directory', path: fileURLToPath(new URL('../assets/',
  import.meta.url)) }` and body read via `new URL(...)`. Its bundle
  (`dsh-experimental-badge-skill-badge-bundle`) is just a patch inserting the provider row.
- Filesystem provider `<dsh-skill-filesystem>/README.md` roots/priority: rank 100
  `<projectRoot>/.dsh/skills`; 200 `<projectRoot>/.agents/skills`; 300 `customSkillDirs`;
  400 `<dshHome>/skills` (skips `.system`); 500 `<agentsHome>/skills`; 600 `bundledSkillDir`.
  `$DSH_BUNDLED_SKILL_DIR` is the default `bundledSkillDir` source. Project root = nearest
  ancestor with `.git`. Config fields: `providerName`, `includeDefaultRoots` (false → only
  custom/bundled roots, omits project/user rows and the env default), `dshHome`, `agentsHome`,
  `customSkillDirs` (strings, `path.resolve`d — no `~` expansion), `watch*`, `bundledSkillDir`.
  `.claude/skills` is **not** a default root; a project can add it via `customSkillDirs` in the
  profile patch (e.g. `customSkillDirs: ["<abs>/<project>/.claude/skills"]`).
- SKILL.md format (directory bundle `<name>/SKILL.md` or flat `<name>.md`, one level deep):
  frontmatter required `name` (kebab-case) + `description`; optional `whenToUse` (string),
  `metadata` (object), `disable-model-invocation`, `user-invocable` (strict boolean grammar:
  bool / `true|false|yes|no|on|off|1|0` case-insensitive). Legacy camelCase invocation keys
  (`disableModelInvocation`, `modelInvocable`, `userInvocable`) are **rejected** — the whole
  skill is dropped with a warning. Unknown keys (`allowed-tools`, `argument-hint`, …) are
  ignored silently; govctl's `.claude/skills/*/SKILL.md` frontmatter (`name`, `description`,
  `allowed-tools`, `argument-hint`) therefore loads as-is if the file is pointed at.
- Model surface: `dsh-tool-skill` publishes a durable `<available_skills>` catalog at the first
  eligible `agent/pre-step` and a `skill` loader tool; a `/skill-name` token in user input
  injects that skill's body (user-invocable only). Tool descriptions: `Skill` tool name is
  `skill`; dashboard lists source/provider/rank — not model-visible.

### 4.3 Commands — `ctx.commands` (`<dsh-commands>/README.md`)

```js
ctx.commands.register({
  name: 'govctl',                     // lowercase [a-z0-9_-], slash command /govctl
  description: 'Run govctl governance commands',
  input: { hint: '<subcommand>' },    // optional; attachments optional
  handler: async ({ agent, rawInput }) => ({ kind: 'success', text: '…' }),
})
```

Global registration = all agents; agent-scoped registrations shadow globals; duplicate names in
one layer throw. Handlers run directly against the agent without creating a model message;
results are `{kind:'success'|'error', text}` (may name `sourceEventSeq`). Registration/disposal
emit `commands/change`; the Web client renders `/` completion (`ui-commands` row).

### 4.4 System prompt — `ctx.systemPrompt` (`<dsh-system-prompt>/README.md`, lib)

```js
ctx.systemPrompt.section({ name: 'govctl:governance', order: 1000,
  text: 'This repository is governed by govctl …' })          // text: string | (context)=>string
ctx.systemPrompt.context({ name: 'govctl:state', order: 1000,
  text: () => 'Current governance summary: …', required: false })  // user-role runtime context
ctx.systemPrompt.variable('govctl_branch', () => 'main')      // {{govctl_branch}}
```

- Sections concatenate by ascending `order`, then name; `interpolate:false` preserves
  `{{…}}`; a `complete:true` section becomes the whole prompt (minimal preset uses persona
  `complete:true`). Registered root-level sections are visible to every agent that does not
  use a complete persona.
- Runtime **contexts** are separate sourced user-role snapshots (not system prompt); they are
  the recommended place for frequently changing session facts (`required` survives suppression).
- `ctx.systemPrompt.getSectionOrder(name)` / `getContextOrder(name)` resolve first-party
  placement; external sections may use any finite order (use ≥1000 to stay after first-party
  guidance; first-party suffix orders are 10000/10100/10200).
- Scoped registrations shadow same-name global registrations for one agent; duplicate names
  within one layer throw; `refreshContext(assembly, context)` refreshes context providers.
- Lead question B confirmed: `section`/`context` accept `{name, order, text}` where `text` may
  be a function receiving the assembly context (shipped plan-mode and user-approval register
  functions this way); use a static section for policy prose and an `agent/pre-step` injection
  for per-turn dynamic state.

### 4.5 Agent presets — `@deepseek-ai/dsh-agent-preset` + `-registry`

Shipped rows (from `dsh-web-app/presets/*.patch.yml`, composed in the web/desktop profiles):

```yaml
- id: agent-preset-registry
  name: '@deepseek-ai/dsh-agent-preset-registry'
  config: { default: standard }
- id: preset-standard
  name: '@deepseek-ai/dsh-agent-preset'
  config:
    id: standard; name: Standard; order: 1
    plugins:                      # ordinary Cordis entry list, mounted per session
      - { id: persona, name: '@deepseek-ai/dsh-persona', config: { prefix: '…' } }
      - { id: tool-bash, name: '@deepseek-ai/dsh-tool-bash' }
      - { id: planning, name: cordis:group, group: true, isolate: { planMode: true },
          config: [ { id: plan-mode, name: '@deepseek-ai/dsh-plan-mode' } ] }
      - …
```

- Presets are declarative rows; the registry mounts each definition eagerly in a registry-owned
  scope and reuses revisions across sessions; sessions record the preset id
  (`agent-preset/selected`), and existing agents keep their composition. `default` is required;
  the user's `selectedDefault` volatile setting overrides it for new sessions.
- A bundle ships a preset by inserting another `@deepseek-ai/dsh-agent-preset` row (own
  `config.id`). A new preset appears in the picker (`order`); users select it — the default is
  unchanged. If you need govctl tools in the *default* preset, prefer root rows (§3) over
  restating `preset-standard`.
- "Markdown-defined agents": none in 0.2.1-rc.5. `dsh-agent-preset` docs: "Definitions are
  ordinary plugin rows; the registry neither scans directories nor accepts preset paths."
  Subagents inherit the parent's preset revision unless explicitly given an `agentPreset` id.
- Preset mounting audits imports, missing services and root-realm leaks; failed definitions
  remain visible in the roster; a row denied by compatibility policy mounts disabled.

### 4.6 MCP client — `@deepseek-ai/dsh-mcp-client`

Rows: one per server; `serverName` namespaces tool names as `mcp__<serverName>__<rawName>`;
`transport: stdio` (`command`, `args`, `env` merged over scrubbed ambient env, `cwd`) or
`streamable-http` (`url`, `headers`). Tools register into `ctx.tools` and are model-visible like
native tools; discovery failures preserve the previous generation; reconnect backoff defaults
500 ms→30 s, 10 attempts. MCP instruction text is capped (`maxInstructionBytes`, 32 KiB). This
is the zero-TS-code alternative if govctl ever exposes an MCP server (`govctl mcp`): a bundle
could simply insert a `dsh-mcp-client` row pointing at the `govctl` binary (config values can
use `!!js process.env` for secrets). No MCP resource/ skill surface beyond instructions/tools.

### 4.7 Subagent / goals / workflow seams

- Subagent (`<dsh-subagent>/README.md`): service `ctx.subagents` (service key in package is
  `subagent`), providers registered by name (`spawn`, `fork` shipped), model-facing tools
  `subagent`/`subagent_fork` from `dsh-tool-subagent` rows; children inherit parent preset or
  accept an explicit `agentPreset`; depth/capacity settings; completion notices. A bundle can
  register its own provider via `ctx.subagent`/service API (same-process trusted values) but
  this is orthogonal to govctl.
- Goals (`<dsh-goal>`): `ctx.goals.get(agent)` / verbs create/edit/pause/resume/complete/block/
  clear; durable session projection; `/goal` + `tool-goal` consumers; no plugin seam needed.
- Workflow (`<dsh-workflow>`): one `ctx.workflowEngine` provider per context (shipped PTC
  engine in base); `ctx.workflowEngine.start({script, meta, args, parent, signal})` for
  programmatic runs; model path is `dsh-tool-workflow` + `run_code`. A govctl plugin could
  orchestrate multi-artifact work through `start()` but must dispose runs.
- Shell/subprocess: see §5.

### 4.8 Hooks & interception points — what replaced `dsh-hooks-claude-code`/`-codex` (Q3)

- **Absent in 0.2.1-rc.5.** Scan of all 476 installed package `lib/*.js` files: zero hits for
  `agent/session-start` or `dsh-hook-protocol`; no `dsh-hooks-*` package directories. The old
  checkout (`0.1.1-rc.2`) had `packages/hooks/{hook-protocol,hooks-claude-code,hooks-codex}`;
  the installed app has no external-hook bridge and no Claude-Code hook-config loader. No
  installed package reads `.claude/settings.json` `hooks` keys.
- What exists instead (native Cordis interception points; the old README called these "the
  canonical extension surface"):
  - `agent/pre-step` (waterfall) — replace/reject the entering message batch; the installed
    `dsh-agent-instructions` uses it to splice a durable sourced `user/message` into
    `decision.messages` after the last claimed message (`dsh-agent-instructions/lib/index.js`
    ~line 1406: `ctx.on("agent/pre-step", async ({agent, messages, step, signal}, next) => {
    const decision = await next(); … return {...decision, messages: entered} })`).
    `createUserMessage({content, source})` comes from `@deepseek-ai/dsh-llm`.
  - `agent.inject({content, source})` — add model-facing context without waking the driver
    (used by plan-mode/approval notices); available on the `Agent` handle/agent object.
  - `agent/turn-stopping` — steer before a turn closes; `agent/request-error` — retry failures;
    `agent/assistant-stream`, `agent/created|disposed|status` observe.
  - `tools/pre-execute` / `tools/execute` / `tools/post-execute` / `tools/result`,
    `tools/change` — the PreToolUse/PostToolUse equivalents (typed decisions, no shell-hook
    serialization).
  - `session/*` events (created, added, event, flush, activity, …) — SessionStart-like
    observation is `session/created` + `agent/created`, not `agent/session-start`.
  The installed `@deepseek-ai/dsh-tool-cordis/lib/types/api-catalog.js` (generated service/event
  catalog, used by Creator-mode introspection) lists these event names; notably there is **no**
  `agent/session-start`, `SessionStart` or `hook/*` event.
- Replacement for govctl's SessionStart hook: a root-level plugin that (a) registers a static
  `ctx.systemPrompt.section` for standing governance prose, and (b) on the first eligible
  `agent/pre-step` (or with `agent.inject` during `agent/created`) injects a durable
  governance-state message (`createUserMessage` with an explicit source kind), then re-injects
  on state changes. This is exactly the installed `dsh-agent-instructions` pattern.
- Claude-Code compatibility that *does* survive: `dsh-agent-instructions` loads `AGENTS.md`/
  `CLAUDE.md` (candidate defaults `['AGENTS.md','CLAUDE.md']`, additive
  `['AGENTS.local.md','CLAUDE.local.md']`, `projectRootMarkers: ['.git']`, required
  `maxBytes`); user-global chain `$DSH_HOME/AGENTS.md` then `<agentsHome>/AGENTS.md`; project
  chain from project root to the session cwd; unchanged digests are not re-injected; literal
  `</system-reminder>` is escaped. `dsh-persona` gives a preset its own persona/complete prompt
  (scope-only; mounting globally collides with the prompt registry's own persona).

## 5. Process spawning and native-binary behaviour (Lead question 3)

Three supported seams; plugin code itself runs **in-process with host authority**.

1. `ctx.subprocess` — `@deepseek-ai/dsh-subprocess` service, provided by
   `@deepseek-ai/dsh-subprocess-local` (base mounts it; row `subprocess`).
   API (`<dsh-subprocess>/README.md`):
   ```js
   const exe = await ctx.subprocess.resolveExecutable('govctl')   // bare name → scrubbed PATH
   const handle = ctx.subprocess.spawn({
     argv: [exe, 'status', '--json'],
     cwd,                                    // pass the Session cwd explicitly
     stdio: { stdin: 'ignore',
              stdout: { maxBytes: 64 * 1024 },   // or {maxBytes, spill}
              stderr: 'inherit' },               // or 'pipe' / collect
     graceMs: 5000, signal, env: { GOVCTL_…: '…' },
   })
   const { exitCode, signal } = await handle.done
   const out = handle.collected.stdout?.readFrom(0)
   // handle.terminate(), handle.waitForExit(); reads are offset-based, non-consuming
   ```
   No shell interpretation (pass `['bash','-c',cmd]` yourself if wanted); child environment
   strips ambient credentials and `DSH_*` names before explicit `env` merges (`undefined`
   tombstone removes an ambient entry); provider disposal terminates and joins managed ranges.
   `resolveExecutable`: absolute paths verified; bare names resolved against **scrubbed PATH**
   with platform executable extensions; relative paths with separators rejected.
   **Not confined** — the subprocess seam is host authority, not the sandbox.
2. `ctx.shell` — `@deepseek-ai/dsh-shell` capability seam (executor provider selected by the
   composition; base+web profile mount `@deepseek-ai/dsh-bash-sandbox` at root, with
   `dsh-sandbox-local` + `dsh-sandbox-policy`). `ctx.shell.resolve({command, workdir,
   timeoutMs, onExpiry, env, stdin})` → `ctx.shell.execute(spec)` → `result()` with
   `{exitCode, signal, timedOut, stdout, stderr}`; background via `onExpiry:'none'`,
   `readOutput()/kill()`. The sandboxed executor resolves the per-call policy through
   `ctx.sandboxPolicy` (session mode override > deployment default; session `header.cwd` is the
   workspace-write root), confines its own `['bash','-c',cmd]` argv via `ctx.sandbox.confine`,
   and fails closed with `SANDBOX_UNAVAILABLE` if no runner exists. It is the seam used by the
   shipped `bash` tool; mirroring that tool's pattern is how a custom tool honors session file
   policy.
3. `ctx.sandbox` directly — `ctx.sandbox.confine(argv, policy)` returns enforcing argv (or
   throws `SandboxUnavailableError`); `ctx.sandboxPolicy.resolve({session, mode})` returns
   `{mode, workspaceRoot, sessionId?}`. A plugin that wants its own `govctl` subprocess confined
   without going through a shell can call `confine` and spawn the returned argv via
   `ctx.subprocess`. `danger-full-access` bypasses the provider entirely.
4. Approval: plugin code is *not* sandboxed and tool calls are not automatically gated for
   file effects (only `bash`-style tools run through confinement). The approval seam is
   `ctx.approval.request({agent, tool, callId?, reason, displayReason?, signal})` (turn-enclosed
   only; outcome vocabulary in `dsh-user-approval`; fail-closed `unavailable`). `ctx.tools.guard`
   can deny monotonically; `tools/pre-execute` can return `ask`. Node's `child_process` is also
   importable directly from plugin code (same process) but loses the env scrub, span
   ownership/termination, spill handling and provider abstraction — not recommended.
5. Session cwd for a tool: the shipped bash tool uses
   `const cwd = exec.agent === void 0 ? void 0 : await ctx.workingDirectory.ensure(exec.agent, exec.signal)`
   (`dsh-tool-bash/lib/index.js` ~line 663; service from
   `@deepseek-ai/dsh-working-directory`, row `working-directory`, projection-backed). Then
   `resolveWorkdir(args.workdir, cwd)`.
6. Native `govctl` discovery options, strongest first: (a) resolve the binary relative to the
   bundle package (`new URL('../bin/<platform>/govctl', import.meta.url)`), shipped as
   per-platform optional dependencies or `assets/`; (b) `ctx.subprocess.resolveExecutable('govctl')`
   for a user-installed binary on PATH; (c) explicit absolute path in plugin config. Caveat:
   the GUI may be launched from Finder with a minimal PATH, so PATH-only discovery is fragile;
   also child env scrubbing means `DSH_*`/credential-shaped vars are not forwarded unless passed
   explicitly. Use `resolveExecutable` and surface a model-readable diagnostic that names the
   searched locations.

## 6. Distribution and install paths (Q4)

### 6.1 CLI (bundled CLI, no PATH `dsh` needed)

- `dsh plugin --profile <name> …` **forwards remaining args verbatim to pnpm in the profile
  directory** (`dsh plugin --profile web --help` prints pnpm-style "manage a profile's plugins
  by forwarding…"; observed pnpm 11.28.5 output). It initializes a base-backed profile on first
  use (`dsh plugin --profile probe add <pkg>` → "initialized profile probe at
  $DSH_HOME/profiles/probe"; bundles become `["@deepseek-ai/dsh-base", "<pkg>"]`).
- Available commands include the pnpm verbs (`add`, `remove`, `why`, …) plus
  `version-exemptions`, `allow-version`, `revoke-version` (plugin-manager README).
- Installed specs follow pnpm: registry name/range/tag/alias, `file:`/`link:` path (made
  absolute against the profile or `~` for `~`), git address/URL, tarball. A local path is
  recorded as `link:` (verified: `"govctl-dsh-probe": "link:/tmp/dsh-probe-bundle"`).
- `dsh --profile <name> --dump-config` / `--dump-default-config` are the safe read-only
  composition previews; `--from-default-profile <template>` recreates a custom profile from a
  shipped template (verified: `dsh --profile probe2 --from-default-profile web --help` created a
  web-template profile without binding a server).

### 6.2 Inside the running GUI (Web/Desktop)

- The Web **Plugins** page is `ui-plugin-manager` (row inserted by `dsh-web-app`), backed by the
  root `plugin-manager` service row (`disabled: !ctx.get('profileContext')` — enabled in a
  profiled boot). It lists bundles (profile deps + installation-supplied/official optional),
  toggles rows (`setPluginEnabled` writes `disabled` into the profile patch), and installs:
  **"Add plugin"** accepts a package name/version, Git address, tarball, or absolute local path;
  it first calls `pluginManager.inspect`, shows a trust box, executes one pnpm run at a time,
  streams output, supports cancel, offers "Allow these scripts and retry" when pnpm blocks
  install scripts, and offers **Enable now** after install. Docs:
  `<dsh-client-ui-plugin-manager>/README.md`.
- Agent-facing equivalent: the `plugin_manager` tool (row `tool-plugin-manager`, service
  `@deepseek-ai/dsh-plugin-manager/tools`) is enabled **only in the `cordis` (Creator) preset**;
  "Every tool action requires `danger-full-access` or approval for that call". Operations:
  `listBundles`, `listPlugins`, `install_bundle`, `remove_bundle`, `set_bundle_enabled`,
  `set_plugin_enabled`, `inspect`, `registries`, version-exemption methods, `waitForInstall`,
  etc. The current session's file policy is `danger-full-access` (runtime context), so the tool
  would run without approval if the Creator preset is selected.
- `plugin_market` tool / Discover tab exist only when the optional
  `@deepseek-ai/dsh-experimental-plugin-market-bundle` is selected. **It is not in either the
  web or desktop profile bundle list** (`~/.dsh/profiles/*/package.json`), so no market is
  enabled by default. Market indexes are JSON format 1, validated by
  `@deepseek-ai/dsh-plugin-market-index` (`parseMarketIndex(value, url)`,
  `marketInstallTarget(name, version)` → `name@version` + registry, or commit-pinned
  `git+https…#&path:`), configured per profile via `pluginMarket.setIndexes`; installing still
  goes through `pluginManager.installBundle`. govctl's `.claude-plugin/marketplace.json` is a
  **Claude Code** marketplace document, not a DSH market index — not reusable as-is.

### 6.3 Desktop profile specifics (Lead question 6 / F)

Verified facts:

- The live GUI boots `desktop-host` with
  `loadProfileDirectory("dsh", "/Users/liuxinyu/.dsh/profiles/desktop", installAnchor)` and
  `runProfile({profile:"desktop", patchFiles: [], args:["--no-open","--host","127.0.0.1",
  "--port","0"], …})` (`dsh-desktop-host/lib/index.js` `main()`). It also passes launcher facts
  `packageManager: { command: process.execPath, args:[electron, "<runtime>/…/pnpm.mjs"],
  env:{ELECTRON_RUN_AS_NODE:"1", DSH_DESKTOP_NODE_EXECUTABLE, PATH:<runtime/bin>:…} }` when the
  Electron shell supplies the pnpm path — i.e. Desktop owns the package-manager invocation and
  the packaged runtimes.
- `dsh --profile desktop --dump-config` (and booting it from the CLI) is refused:
  `error: profile "desktop" is managed exclusively by the Electron application`. This is a
  name/ownership reservation independent of `DSH_HOME` (also refused in a scratch home).
- `dsh plugin --profile desktop …` **is** wired for the desktop profile but requires the
  Desktop app to have initialized it and to be fully quit: in an empty scratch home the command
  fails with `Open DeepSeek Harness Desktop once to initialize its profile, then fully quit it
  before running dsh plugin --profile desktop.` In the real home a read-only call
  (`version-exemptions`) returned `{}` while the app was running. So (i) `dsh plugin --profile
  desktop add <spec>` is expected to work as a durable profile-file change when the app is quit
  (profile `package.json` deps + `dsh.profile.bundles` + `node_modules` are ordinary profile
  files); the app reads them at next launch. No evidence the Desktop shell rewrites the bundle
  list — it owns the *installer* path, not the manifest.
- (ii) The GUI Plugins page can install third-party bundles in the desktop profile: the page's
  Host reads a "managed profile", the base `plugin-manager` row activates when `profileContext`
  exists, and the desktop host supplies the packageManager fact the manager needs. Desktop's
  own limitation in the package docs is narrower: "Desktop package operations remain owned by
  the Desktop shell" — i.e. pnpm runs/launch facts are delegated to the shell, not that install
  is impossible. Recommended for the report: GUI Plugins page is the supported in-app path;
  CLI-with-app-quit is the scripted path; app restart guarantees the new bundle layer.
- (iii) Desktop owns: app lifecycle/update, the profile directory location (`--profile desktop`
  reserved), packaged runtime dirs + bundled pnpm invocation, Office engine resolution, quit/
  update task admission. The profile directory itself (package.json, cordis.patch.yml,
  pnpm-workspace.yaml, compatibility.json, node_modules) is mutable profile state; `sanitizeProfile`
  is only the fatal-recovery path (renames `cordis.patch.yml`, restores the supplied bundle
  list). HMR is enabled for profiled boots (`hmr` row disabled only when `profileContext` is
  absent), so config/bundle-selection changes can recompose live; package *installation* is
  still subject to restart/HMR rules §6.5.

### 6.4 npm-published bundle requirements

- Package must be installable by pnpm from a registry (or git/tarball/path) and declare the
  bundle manifest §2.1; Plugin Manager validates the manifest and the patch files before
  selecting the bundle. Prefer `saveExact` behavior is only for official on-demand installs;
  normal `add` saves what pnpm saves.
- Ship compiled ESM (`main`/`exports`), patch file(s), assets (skills, binaries) in `files`.
  Peer-declare runtime packages you import. No `postinstall`/`prepare` build is needed; if you
  use one, pnpm 11 blocks it until the user approves ("Allow these scripts and retry"; approval
  persists per package name in the profile's `pnpm-workspace.yaml`; tool path
  `install_bundle` takes `approvedBuilds` after explicit user approval). A git dependency's
  prepare script and a tarball's scripts are subject to the same pending-build flow.
- Version admission: publish `engines.dsh` matching the runtime you tested; otherwise every
  `@deepseek-ai/dsh*` peer must match. Use exact-version exemptions only for deliberate
  incompatibility testing.

### 6.5 HMR, restart, uninstall, upgrade

- Web/desktop profiles enable `dsh-hmr` with `root: []` (config/profile watches only; source
  watching off by default — `hmr` row in `dsh --profile web --dump-config`). HMR recomposes on:
  bundle-list changes, profile/home patch edits, Plugin Manager writes, and module file changes
  below the profile directory/linked targets (at recomposition). Plugin Manager operations run
  outside the HMR queue; a new bundle's rows are started by the recomposition.
- Without HMR (headless/acp/sdk templates): install succeeds but activation waits for restart;
  results carry `restartReason: no-hmr | package-moved | …`. With HMR, installing a package
  already installed is refused as `already-installed`; a version change is remove+install
  (previous version's plugins stop, modules evicted first). A replacement that fails to import
  stops the previous version and reports `failed`; later operations retry it.
- Uninstall: remove from `dsh.profile.bundles`, unload runtime contributions, `pnpm remove`,
  publish resolution (in that order); failures preserve completed steps. Upgrading is
  uninstall+install (market has no upgrade verb). Disabling a bundle only deselects the layer
  and keeps the dependency; enabling appends it at the **end** of `dsh.profile.bundles`, which
  can change patch precedence.
- Diagnostics: Plugin Manager writes `.plugin-manager/run.json` + `.plugin-manager/logs/…`;
  install output streams to the GUI; CLI prints one-line failure + log path; app-boot prints
  startup diagnostics and exit code 1 for required-entry failures. Plugin logging is
  `ctx.logger` (fiber-scoped); unhandled rejections/throws are fatal (dispose + exit) per
  app-boot's startup/reload failure policy.

## 7. Client / browser-side contribution (what a bundle can/cannot add to the UI)

- A package can declare `dsh.client: {platform:'web', inject:[…], external:[…],
  immediately?:boolean}` and export a `./client` bundle (built `lib/client.js`); the host half
  (`dsh-client-modules`) scans enabled Loader entries, composes `window.__DSH_BOOT__`, serves
  `/plugins` bundles, and the browser loads them lazily. Requirements: built client artifact
  before launch, imports of other client packages listed under `external` (baseline module table
  otherwise), no config; client factories execute lazily (`<dsh-client-modules>/README.md`).
- Custom tool cards: business packages register a view into the `tool.call.toolview` slot keyed
  by wire tool name; unregistered tools render a generic card
  (`<dsh-client-ui-tool>/README.md`). Bundle/row configuration pages use
  `plugins.bundle.config` / `plugins.row.config` keyed `<pkg>` / `<pkg>#<rowId>`, and pages are
  attached to the Loader row of the bare package name (sub-plugins that must outlive other rows
  ship as their own package) — `<dsh-client-ui-plugin-manager>/README.md`.
- A host-only bundle (no `dsh.client`) is fully functional for tools/skills/commands; it just
  renders with generic tool cards. UI work is a separate build target and is not required for
  govctl's first version.

## 8. Constraints and gotchas

- **Trust boundary**: plugin code runs in-process with the host user's authority, outside the
  workspace sandbox. Installing a bundle is equivalent to running arbitrary host code;
  Plugin Manager docs: "installed Host code executes in-process outside the workspace sandbox".
  The compatibility check "is not a sandbox against malicious package code".
- **Approvals**: only the management actions are approval-gated (`plugin_manager` tool calls
  require `danger-full-access` or approval; the GUI page has its own trust box). A plugin's own
  tool calls are gated only by whatever `tools/pre-execute`/guards/its own approval requests it
  implements. `approval.request` needs an open turn and a composed answerer (`ask` policy).
- **No external shell-hook bridge** in 0.2.1-rc.5; implement interception natively (§4.8).
- **No Markdown-defined agents/presets**; presets are YAML rows; `.claude/agents/*.md` are not
  loaded.
- **`.claude/skills` is not a default skill root** (`.dsh/skills` and `.agents/skills` are).
- **Preset restatement brittleness**: patching a shipped preset by id replaces its full config;
  it will silently diverge when the shipped list changes across DSH versions.
- **Compatibility is enforced only at composition boundaries** DSH owns (startup,
  recomposition, preset mounting); code loaded through other paths isn't re-checked. `manifestVersion`
  is not enforced; `engines.dsh` is.
- **Linked package imports need peerDependencies** (verified); use registry installs or
  `file:`/copied installs for less surprise. pnpm warns for `link:` peer resolution.
- **PATH/env**: child env scrubs credentials and `DSH_*`; Finder-launched GUI may have a minimal
  PATH — resolve the native binary explicitly (package-relative) and surface diagnostics.
- **`--dump-config-schema` executes module imports** and reports known errors on preset rows;
  don't use it as a pass/fail gate.
- **Desktop profile is app-owned**; CLI cannot boot it, and plugin operations require the app to
  be quit. Do not edit `~/.dsh/profiles/desktop` by hand while the app runs.
- **Stale `~/.dsh/profiles/node_modules`** from the 0.1.1 layout (187 packages at
  `0.1.1-rc.2`) sits as an ancestor `node_modules` of every profile. Installation mappings are
  intercepted by runtime resolution, so this normally does not shadow, but it is a real
  version-skew hazard for native/fallback resolution; do not install against it.
- **Session/agent identity not available at `apply()` time**: a root plugin activating at boot
  has no session; per-session behavior must hook agent events (`agent/pre-step`, `agent/inject`,
  `session/created`) and read `exec.agent` inside tools.
- **One provider per service** (subprocess/shell/sandbox/workflowEngine); a bundle must not
  mount a second provider. Bundles should *consume* `ctx.subprocess`, `ctx.shell`, `ctx.tools`,
  `ctx.skills`, `ctx.systemPrompt`, `ctx.commands`, `ctx.workingDirectory`, `ctx.approval`.

## 9. Minimal reliable govctl bundle recipe

```
govctl-dsh/
├─ package.json          # name, type:module, main, exports (., ./cordis.patch.yml, ./package.json)
│                        # engines.dsh ">=0.2.1-rc.5 <0.3.0-0"
│                        # dsh.bundle.patch ["./cordis.patch.yml", "./presets/govctl.patch.yml"]
│                        # peerDependencies: @deepseek-ai/dsh-tools, dsh-skill, dsh-system-prompt,
│                        #   dsh-commands, dsh-subprocess, dsh-llm, dsh-working-directory, cordis 4.x
├─ cordis.patch.yml      # insert root rows:
│                        #   - id: govctl            name: 'govctl-dsh'            (apply registers everything)
│                        #   - id: govctl-skills     name: 'govctl-dsh/skills'     (ctx.skills.registerProvider)
│                        #   - id: preset-govctl     name: '@deepseek-ai/dsh-agent-preset'  (optional)
├─ presets/govctl.patch.yml
├─ lib/index.js          # export const name/inject/apply; raw ctx.tools.register + systemPrompt.section
│                        # + commands.register + agent/pre-step injection; spawn via ctx.subprocess
├─ lib/skills.js         # skill provider: assets dir via new URL('../assets/skills/', import.meta.url)
└─ assets/skills/**/SKILL.md
```

Notes: raw `ctx.tools.register` lets the PoC stay dependency-light beyond peer imports; guard
`config = {}` because a row without `config` passes `undefined` and no Config schema means no
validation (Lead question E: confirmed below). For a tool that runs `govctl`, resolve the
session cwd with `await ctx.workingDirectory.ensure(exec.agent, exec.signal)` and spawn with
`ctx.subprocess.spawn({argv, cwd, stdio:{…}, graceMs, signal})`; for file-policy-aware runs,
confine through `ctx.sandboxPolicy.resolve` + `ctx.sandbox.confine` (or use `ctx.shell`).
Root rows are inherited by every preset (do not duplicate standard's list); a new preset is an
optional discovery convenience only.

**Lead question E answered**: a row's `config:` is passed to `apply(ctx, config)` as the raw
object; declaring a static `Config` (Schemastery `z.object`) is **not required**. Cordis
`resolveConfig(runtime, config)` returns `config` unchanged when the plugin has no `Config`
static (`cordis/lib/index.js` line ~956), and `fiber.config` is what `apply` receives
(`runtime.callback(this.ctx, this.config)` lines ~1068/1071). Consequences: no validation, no
defaults, and a row that omits `config` gets `undefined`/absent — guard for it in the PoC.

## 10. Verified by running

All commands used the bundled CLI (no network/LLM actions; scratch `DSH_HOME` for install
tests; real profiles untouched).

1. `dsh --version` → `0.2.1-rc.5`.
2. `dsh --profile web --dump-config` → 1231 lines; bundle headers `# == @deepseek-ai/dsh-base`,
   `# == @deepseek-ai/dsh-web-app`, `# == @deepseek-ai/dsh-base, patched by @deepseek-ai/dsh-web-app`;
   HMR row enabled via `disabled: !!js '!ctx.get(''profileContext'')'`, `config: {root: []}`;
   root tool rows disabled by the web-app layer; presets `preset-standard|ptc|minimal|cordis`.
   Note `- id: plugin-manager … disabled: !!js '!ctx.get(''profileContext'')'`,
   `- id: sandbox … dsh-sandbox-local`, `- id: sandbox-policy … mode: !!js
   process.env.DSH_PERMISSION_MODE ?? 'workspace-write'`.
3. `dsh --profile desktop --dump-config` → exit 1,
   `error: profile "desktop" is managed exclusively by the Electron application`. Same in a
   scratch `DSH_HOME`.
4. Live process inspection (`ps aux`) identified the running GUI as
   `…/dsh-desktop-host/lib/index.js <app.asar/dsh> /Users/liuxinyu/.dsh/profiles/desktop
   <runtime/primary-runtime> <…/pnpm/bin/pnpm.mjs> <runtime/bin>` (see §1).
5. Scratch installs (`DSH_HOME=/tmp/dsh-probe-home`):
   - `dsh --profile probe --dump-config` → `profile "probe" does not exist; create it with
     'dsh plugin --profile probe add <package>'`.
   - `dsh plugin --profile probe add /tmp/dsh-probe-bundle` → `initialized profile probe at
     …`; deps `+ govctl-dsh-probe 0.1.0 <- ../../../../../tmp/dsh-probe-bundle`; pnpm
     `11.28.5`; profile bundles became `["@deepseek-ai/dsh-base","govctl-dsh-probe"]`.
   - `dsh --profile probe --dump-config` → both rows from two patch files:
     `# == govctl-dsh-probe` containing `- id: govctl-probe-tool name: 'govctl-dsh-probe/tool'`
     and `- id: preset-govctl-probe name: '@deepseek-ai/dsh-agent-preset'` with its plugins list.
   - `dsh plugin --profile probe add /tmp/dsh-probe-bad` (engines.dsh `>=0.3.0`) → refused:
     `installation rejected: Plugin govctl-dsh-probe-bad@0.1.0 is incompatible with dsh
     0.2.1-rc.5: requirements {"engines.dsh":">=0.3.0"}` + exact `allow-version` command;
     `nothing was installed`; package.json unchanged.
   - `dsh plugin --profile probe allow-version govctl-dsh-probe-bad@0.1.0 --dsh-version
     0.2.1-rc.5 --accept-risk` → wrote `compatibility.json`
     `{"govctl-dsh-probe-bad@0.1.0":["0.2.1-rc.5"]}`; the retried add then installed.
   - `dsh --profile probe --dump-config-schema` → imported the probe package; failed first with
     `Cannot find package '@deepseek-ai/dsh-tools' imported from /private/tmp/dsh-probe-bundle/
     lib/tool.js` (link-root rule), succeeded after adding that peer; remaining error
     `[/97] unrecognized Loader tree carrier …` is the same error the shipped web profile emits
     for its preset rows (`/187-/190`).
6. Web-template scratch profile: `dsh --profile probe2 --from-default-profile web --help` created
   `$DSH_HOME/profiles/probe2` with bundles `[dsh-base, dsh-web-app]`; after
   `dsh plugin --profile probe2 add /tmp/dsh-probe-bundle` and `--dump-config`, the third-party
   layer appears last (`# == govctl-dsh-probe` at `/1232`) and `preset-standard` is still present
   (`/630`) — appended bundle layers win over earlier layers and are not disabled by web-app.
7. Read-only SDK/metadata probes: `dsh plugin --profile web version-exemptions` → `{}`;
   `dsh plugin --profile desktop version-exemptions` → `{}` (app running); empty scratch home
   desktop → error telling the user to open Desktop once and quit it.

Scratch artifacts (not in repo): `/tmp/dsh-probe-bundle`, `/tmp/dsh-probe-bad`,
`/tmp/dsh-probe-home`, `/tmp/probe-dump.yml`, `/tmp/probe2-dump.yml`, `/tmp/web-dump.yml`,
`/tmp/dsh-extract/` (selected ASAR package docs/sources).

## 11. Version skew / uncertainty

- **Repo checkout vs installed**: `/Users/liuxinyu/Projects/deepseek-harness` is `0.1.1-rc.2`
  and contains packages/APIs that no longer exist in 0.2.1-rc.5 (notably
  `packages/hooks/{hook-protocol,hooks-claude-code,hooks-codex}` and their
  `agent/session-start` mapping). Treat the checkout's `docs/` as historical context only; this
  note is based on installed package READMEs/sources, which are the runtime's own docs.
- `~/.dsh/profiles/node_modules/` holds `0.1.1-rc.2` copies of `dsh-base`/`dsh-web-app`/
  `dsh-tools`/… from the old shared-workspace layout. Current installs are per-profile pnpm
  projects (`…/profiles/<name>/node_modules`); the stale directory is an ancestor lookup path
  and a potential hazard, not part of the current composition.
- Several installed READMEs link to source-repo `docs/subsystems/*.md`,
  `.agents/notes/*.md`, `docs/cookbook/*` that are **not shipped** in the ASAR; claims above
  therefore cite the shipped package README/source, and a few deep behaviours (exact approval
  outcome vocabulary, exec-agent API surface beyond what tool-bash uses) are marked from source
  reading rather than docs.
- Desktop-plugin-listener installation: "GUI Plugins page works for desktop" is inferred from
  the composed rows + desktop `packageManager` launcher fact; it was not exercised end-to-end
  (we did not install into the real desktop profile). CLI `add` to desktop is inferred from the
  refusal messages and profile-file layout; confirm with a real install if it matters.
- Finder-launched GUI PATH minimality is a general macOS assumption, not verified for this app.
- `agent/session-start` absence was verified by scanning installed package `lib/*.js` for the
  literal string, not by enumerating the event registry type declarations.
- HMR live-apply of a *newly installed third-party bundle* in the running desktop app was not
  exercised (needs a real install); docs state recomposition starts new entries, but the shell's
  ownership note is a caveat.

## 12. Evidence index (installed ASAR paths; `<PKG>` = `dsh/node_modules/@deepseek-ai/<pkg>/`)

- Manifest/profile/patch: `<dsh-package-manifest>/README.md`; `<dsh-app-boot>/README.md`
  (§Profiles, §Previewing, §Inspecting schemas); `<dsh-package-manifest>/lib/*` types;
  `dsh-base/cordis.patch.yml`; `dsh-web-app/cordis.patch.yml`,
  `dsh-web-app/presets/{standard,ptc,minimal,cordis}.patch.yml`; `dsh-web-app/package.json`.
- Plugin manager/install/versioning: `<dsh-plugin-manager>/README.md`; CLI
  `dsh plugin --help`; `~/.dsh/profiles/{web,desktop}/{package.json,cordis.patch.yml}`.
- Scope model: `<dsh-scope>/README.md`; `<dsh-tools>/lib/index.js` (`view`, `register`,
  `restrict`); `<dsh-skill>/lib/index.js` (layering comment); `<dsh-system-prompt>/README.md`
  + `lib/index.js`; `<dsh-commands>/README.md` + `lib/index.js`;
  `<dsh-agent-preset-registry>/README.md` + `lib/index.js`; `<dsh-agent-preset>/README.md`.
- Extension points: `<dsh-tools>/README.md`; `<dsh-skill-filesystem>/README.md` + `lib/index.js`;
  `<dsh-skill-badge>/lib/index.js` (bundle skill precedent); `<dsh-tool-skill>/README.md`;
  `<dsh-mcp-client>/README.md`; `<dsh-subagent>/README.md`; `<dsh-goal>/README.md`;
  `<dsh-workflow>/README.md`; `<dsh-agent-instructions>/README.md` + `lib/index.js`
  (pre-step recipe); `<dsh-persona>/README.md`; `<dsh-tool-cordis>/lib/types/api-catalog.js`
  (event/service catalog).
- Spawning/sandbox: `<dsh-subprocess>/README.md`; `<dsh-subprocess-local>/README.md`
  (§Resolving executables); `<dsh-shell>/README.md`; `<dsh-bash-local>/README.md`;
  `<dsh-bash-sandbox>/README.md`; `<dsh-sandbox>/README.md`; `<dsh-sandbox-policy>/README.md`;
  `<dsh-user-approval>/README.md`; `<dsh-tool-bash>/README.md` + `lib/index.js`
  (`ctx.workingDirectory.ensure` pattern); `<dsh-working-directory>/README.md`.
- HMR/restart: `<dsh-hmr>/README.md`; `<dsh-app-boot>/README.md` startup/reload failures.
- Client/UI: `<dsh-client-modules>/README.md`; `<dsh-client-ui-tool>/README.md`;
  `<dsh-client-ui-plugin-manager>/README.md`; `<dsh-plugin-market>/README.md`;
  `<dsh-plugin-market-index>/README.md`; `<dsh-package-manifest>/README.md` (`DshClientManifest`).
- Desktop ownership: `dsh-desktop-host/lib/index.js` (`main()`, `loadProfileDirectory`,
  `runProfile`, `packageManager`); live `ps aux` arguments.

---

## Appendix: API-correctness review of the PoC bundle (`research/poc/govctl-dsh-plugin`)

Reviewed against installed 0.2.1-rc.5 sources. Verdict: the core contract usage is
correct; fix the items below before calling it production-shaped.

Correct as written (verified against installed sources):
- Plugin shape `export const name / inject / function apply(ctx, config)`; row `config` is
  passed raw (no `Config` static needed — §9), so the PoC's `config = {}` default is right.
- Raw `ctx.tools.register({name, description, parameters, output:{schema, render}, execute})`
  is the accepted raw form; JSON-Schema subset used (`object/properties/required/
  additionalProperties/array/items/enum`) is inside the enforced subset; empty
  `properties: {}` is valid. `output.render` returning `[{type:'text', text}]` matches.
- `ctx.systemPrompt.section({name, order, text: (context) => string})` and
  `ctx.systemPrompt.context({name, order, text: (context) => string})`: `context` is
  `assembleContextFor(agent, signal)` = `{agent, scope: agent, signal?}`, so `context.agent`
  is correct (`dsh-agent/lib/index.js`, `dsh-agent-loop/lib/index.js` assemble call).
- `ctx.skills.register({name, description, content})`: valid — register validates only name
  (kebab-case regex `^[a-z0-9]+(?:-[a-z0-9]+)*$`), non-empty description and invocation;
  `content` is required at load time. govctl's kebab-case skill names fit.
- `ctx.commands.register({name, description, handler({agent, rawInput}) → {kind, text}})`
  matches the shipped signature; global registration is visible to all agents.
- `ctx.workingDirectory.get(agent.session)` is the correct synchronous accessor (the service
  also has `ensure(agent, signal)`); `exec.agent`/`agent.session` exist in `execute`.
- `agent/created` payload really is `{agent, source, signal?}`; `tools/pre-execute`
  `(exec, next)` is a waterfall listener and returning `next()` is fine; `file_path` is the
  correct write/edit argument name.
- `engines.dsh: ">=0.2.1-rc.5"` admits the running runtime; a registry version only needs
  `dsh.bundle.patch` + built JS.

Issues to fix (ordered):
1. `runGovctl` accepts `options.input` but never writes stdin (`execFile` has no `input`
   option), so `govctl agent hook session-start` runs with closed stdin — the per-session
   state is almost certainly empty. Either deliver the hook payload on stdin via
   `ctx.subprocess.spawn({stdin:{data?}})`, or pass the payload through argv/env.
2. `agent/created` is a **serial, awaited** creation event (`ctx.serial(…, "agent/created",
   {agent, source, signal})`), so awaiting a subprocess there delays every new session
   up to the 5 s hook timeout. Prefer the `agent/pre-step` pattern used by shipped
   `dsh-agent-instructions` (await state on the first eligible step, splice a sourced
   `user/message` into `decision.messages` with `createUserMessage` from
   `@deepseek-ai/dsh-llm`), or kick off the spawn without awaiting and let a
   `tools/post-execute`/pre-step gate fill state. The state cache must not gate session
   creation on a missing `govctl` binary.
3. Mutating `govctl_run` has no approval gate. Cheapest correct form: from the same root
   plugin listen on `tools/pre-execute` and return
   `{kind:'ask', reason:'govctl lifecycle mutation …'}` for mutating argv (the registry then
   calls `ctx.approval.request({agent, toolName, callId, reason, signal})` itself and maps
   `allowed-once|rejected|cancelled|unavailable`). Alternatively call
   `ctx.approval.request` inside `execute` and reject on non-allow. Note `ask` fails closed
   (deny) when no approval answerer is composed.
4. The managed-edit advisory can be attached non-blockingly: `tools/post-execute` listeners
   may return `{kind:'accept', additionalContexts:[createUserMessage({content, source})]}`
   (the loop FIFOs them), so no blocking is needed; `tools/pre-execute` alone cannot attach
   content. The PoC's "log only" is honest but incomplete.
5. `execFile` bypasses the subprocess seam: no env scrub, no managed-range termination,
   no path resolution, no spill. Use `ctx.subprocess.resolveExecutable` + `spawn`; pass
   stdout/stderr collect caps; the current 8 MB `maxBuffer` with full stdout in the tool
   result can blow the model budget (apply a tail cap or spill).
6. Native binary discovery: `binaryPath: 'govctl'` relies on PATH; Finder-launched GUI PATH
   can be minimal. Resolve package-relative vendor binary first, then
   `ctx.subprocess.resolveExecutable('govctl')`, and return a model-readable diagnostic with
   the searched locations/version.
7. Skills are registered as embedded runtime skills from a naive frontmatter regex. That is
   acceptable for the PoC, but production should either ship via `skill-filesystem`
   (`customSkillDirs`/`bundledSkillDir`) or use YAML parsing; also note `$ARGUMENTS`
   templating does not exist in 0.2.1 skills (the `skill` tool loads the body verbatim;
   user `/name` tokens inject the body), so ported govctl skill text must not depend on
   argument substitution.
8. `private: true` blocks npm publication — remove it for a published bundle; also decide
   whether `engines.dsh` needs an upper bound (`<0.3.0-0`) to fail fast on the next minor.
9. `ctx.on('tools/pre-execute', async (exec, next) => …)` is registered at root, i.e. for all
   agents/sessions; that is intended here, but document it as a global policy attachment.

Suggested acceptance check for v1 (no LLM cost): boot a scratch web profile containing the
bundle (`dsh --profile <p> --from-default-profile web --help` to create it, then
`dsh plugin --profile <p> add <path>`, then `dsh --profile <p> --dump-config`), and inspect
`--dump-config-schema` only with the caveat in §2.4; a real model turn is required to prove
tool visibility, since registration is only observable through assembly.
