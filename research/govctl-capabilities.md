# govctl product / CLI / agent-integration inventory

Task: `task-2` (owner: govctl-capability-analyst) — factual base for packaging govctl as a DeepSeek Harness (DSH) plugin.
Repo: `/Users/liuxinyu/Projects/govctl` @ `main` `4223933a0b803b6e99e6c98d7e5f2a26242ae3a4` (clean tree), crate `govctl` **0.22.0**, Rust edition 2024, `rust-version = 1.96`.
Inspection mode: read-only source/doc/artifact inspection + public release metadata. **No repo files modified except this deliverable.**

Markers used below:

| Marker | Meaning |
| --- | --- |
| **[V]** | Verified in this repo (source, tests, snapshots, governed artifacts) or in upstream source/release metadata cited inline |
| **[I]** | Inferred from verified facts (design consequence, not directly observed) |
| **[U]** | Unverified/unknown; cannot be established from this environment |

## Environment limitation (important)

- **There is no Rust toolchain on this machine**: `cargo`, `rustc`, `rustup` are not on `PATH`, `~/.cargo` does not exist, no `target/` directory. `govctl` is **not installed** (`which govctl` → not found). [V]
- Consequence: no local build, no `cargo run -- --help`, no binary size/startup measurement, no `govctl status/check` run. All CLI claims below are reconstructed from clap definitions (`src/cli/`, `src/command_router/`), implementation (`src/cmd/`), insta help snapshots (`tests/snapshots/test_help__*.snap`), and normative RFC clauses. A DSH plugin must **not assume a locally compiled govctl**; users get the binary from crates.io/cargo-binstall/GitHub Releases (see §6). [V]

---

## 0. Bottom line for the plugin decision

1. **govctl is a single self-contained Rust binary that already treats the CLI as the agent integration surface.** Its own RFC defines a versioned agent-introspection command (`govctl describe`, schema v1), JSON/plain output contracts for reads, path-first mutation for governed writes, and an internal bridge for non-Rust agent runtimes to call (`govctl agent hook`, hidden). [V] — RFC-0002 clauses `C-DESCRIBE-COMMAND`, `C-OUTPUT-FORMAT`, `C-AGENT-INTEGRATION`; `src/cli/commands.rs:270-276`, `src/cmd/describe/mod.rs:14-57`.
2. **Read paths are stable enough to wrap today; write/lifecycle paths are stable enough to invoke but only report human-readable prose.** `list/get/show/search/tag list/loop list/conformance trace/describe` emit machine-readable stdout; `status/check/verify/render/init/*edit/lifecycle/delete/agent` emit human prose on **stderr** and communicate via exit codes + `E*/W*` diagnostic codes. There is **no `--json` diagnostic envelope** (no structured errors). [V] — `src/ui/messages/*` use `eprintln`, `src/cmd/output.rs` uses `println`, `src/ui/diagnostics.rs:6-33`.
3. **The shipped agent integration is a Claude Code plugin + a Codex projection, both materialized from assets embedded in the Rust binary and installed through the external crate `agent-plugin-installer 0.6.1`.** The plugin is *not* a tool server: it is 13 skills, 4 reviewer agents, 2 hook manifests, and a marketplace catalog; all "tools" are the govctl CLI itself. [V] — `.claude/`, `src/cmd/agent.rs`, `build.rs`.
4. **Hook contract is Claude/Codex-specific**: JSON on stdin `{cwd, tool_input}`; JSON on stdout `{"hookSpecificOutput":{"hookEventName":…,"additionalContext":…}}`; non-blocking only; 5 s native timeout; internal 4,000-char / 8-item caps; Codex manifest adds `additionalContextLimit: 1200`. [V] — `src/cmd/agent_hooks.rs`, `.claude/hooks/{claude,codex}.json`.
5. **`govctl agent install dsh` is not feasible today without upstream changes.** The runtime set is a closed enum in `agent-plugin-installer 0.6.1` (`AgentRuntime::{Codex,Claude}`, no `Dsh`), and DSH has no "marketplace add" native CLI; DSH plugins are npm/Cordis bundles installed per profile (`dsh plugin --profile <name> add …`) plus filesystem-discovered skills. [V] for crate/RFC; DSH-side from a version-skewed checkout (see §7). A separate DSH bundle (TS/Cordis plugin wrapping the binary, optionally shipping skills) is the structurally lower-risk path; skills-only can already be materialized today via `govctl init-skills --dir .dsh`. [I]
6. **Do not reimplement govctl logic in TypeScript.** Validation, lifecycle gates, concurrency locking, ID reservation, claims, migration, and rendering live in Rust and are normative RFC behavior; a wrapper that shells out inherits it for free and stays version-compatible. Reimplementation would duplicate 10+ RFC-defined invariant families and drift on every 0.x release. [I, strongly supported by RFC-0002/RFC-0004/RFC-0010]

---

## 1. Product & artifact model

### 1.1 What govctl is

`govctl` is a "governance-as-code" CLI for AI-assisted development. The repo's `gov/` directory is the authoritative, TOML-encoded source of truth; rendered markdown under `docs/` is a generated projection. The CLI is the only sanctioned mutation surface; agents are expected to call it rather than edit `gov/` directly (the shipped hook only *advises* this). [V] — `README.md:27-97`, `AGENTS.md` §§1-5, RFC-0000 summary.

### 1.2 Artifact types (7)

Source: `gov/RFC-0000/clauses/C-SUMMARY.toml`, `gov/schema/*.json`, `src/model/*.rs`, `docs/guide/getting-started.md:42-120`.

| Artifact | Storage | ID form | Lifecycle / state |
| --- | --- | --- | --- |
| **RFC** | `gov/rfc/RFC-NNNN/rfc.toml` + `clauses/*.toml` | `RFC-NNNN` | status `draft → normative → deprecated`; phase `spec → impl → test → stable` |
| **Clause** | inside RFC dir, first-class CLI resource | `RFC-NNNN:C-NAME` | kind `normative|informative`; status `active → deprecated|superseded` (`since` version lifecycle-owned) |
| **ADR** | `gov/adr/ADR-NNNN-*.toml` | `ADR-NNNN` | status `proposed → accepted|rejected`, `superseded`; alternatives have `considered|accepted|rejected` |
| **Work Item** | `gov/work/*.toml` | `WI-YYYY-MM-DD-NNN` (strategies configurable) | status `queue → active → done|cancelled`; acceptance criteria `pending|done|cancelled` with changelog category |
| **Verification Guard** | `gov/guard/*.toml` | `GUARD-*` | no lifecycle; shell command + timeout + optional regex pattern |
| **Conformance Case** | `gov/conformance/*.toml` | `CONF-*` | applicability `stale|provisional|candidate|current` (derived from RFC version bindings) |
| **Release** | `gov/releases.toml` | semver string | append/correct only; `release <version>` / `release undo <version>` |

All artifacts are validated against JSON Schemas shipped in `gov/schema/` (schema version **5**; project schema versions 3–5 supported, `CURRENT_SCHEMA_VERSION = 5`). [V] — `src/cmd/migrate/mod.rs:18`, RFC-0002:C-COMPATIBILITY-BOUNDARY.

### 1.3 Gates a plugin must honor (do not bypass)

- **Work Item → done** requires ≥1 acceptance criterion, all criteria `done|cancelled`, and every effective guard (work-item `verification.required_guards` ∪ project `[verification].default_guards` when enabled) to pass or be explicitly waived with a reason. [V] — RFC-0001:C-GATE-CONDITIONS; `src/cmd/verify.rs:67-117`.
- **RFC spec → impl** requires normative status and every clause to have a resolved `since` version. Version-changing bumps require normative status, eligible source phase, and a stored sealed signature; bumps reset phase to `spec` for released content. [V] — RFC-0001:C-GATE-CONDITIONS, RFC-0002:C-LIFECYCLE-VERBS.
- **ADR accept** requires ≥1 accepted alternative (and with ≥2 alternatives, ≥1 rejected); `--force` bypasses for historical backfills. [V] — `src/cli/resources/adr.rs:82-105`.
- **Concurrent writes**: mutating commands take an exclusive gov-root lock (`.govctl/gov.lock`, `[concurrency] lock_timeout_secs` default 30, waits bounded then fails); read-only commands are lock-free. [V] — RFC-0004, `src/main.rs:126-156`, `src/command_router/plan.rs:311-346`.
- **Multi-workspace** (git worktrees / jj workspaces): command scopes are workspace / branch-content / trunk; only `release`, `release undo`, `migrate` are trunk-scoped and refuse outside the primary workspace (E0823). Version-semantics RFC ops acquire exclusive claims (E0824 when another workspace holds one); content edits only warn (W0116). [V] — RFC-0010:C-COMMAND-SCOPE, C-ARTIFACT-CLAIM; `src/main.rs:178-212`, `src/command_router/plan.rs:316-382`.
- **Direct file edits** to `gov/{rfc,adr,work,guard,conformance}/**` and `gov/releases.toml` are *not blocked* by govctl; the bundled PreToolUse hook only injects advisory context. [V] — RFC-0002:C-AGENT-INTEGRATION; `src/cmd/agent_hooks.rs:18-21,162-170`.

### 1.4 Filesystem layout, local state, projections

```
gov/                 # authoritative TOML
├── config.toml      # project name/owner, paths(agent_dir, docs_output), schema.version,
│                    # source_scan{enabled,include,pattern}, work_item.id_strategy,
│                    # verification{enabled,default_guards}, concurrency.lock_timeout_secs,
│                    # tags.allowed[], workspace{primary,claim_ttl_days}
├── rfc/ adr/ work/ guard/ conformance/ schema/ templates/
├── releases.toml    # created by first successful `govctl release`
.govctl/             # disposable local state (gitignored): gov.lock, search index (SQLite),
│                    # loops/<LOOP-ID>/state.toml, round files
docs/                # projections: docs/rfc published; docs/adr + docs/work gitignored locally
CHANGELOG.md         # rendered from releases + completed work items
```
[V] — `.gitignore`, `docs/.gitignore`, `src/config/mod.rs:16-298`, `gov/config.toml` (this repo: schema 5, tags allow-list, `default_guards=["GUARD-GOVCTL-CHECK"]`).

- `govctl init` creates dirs/config/schemas and local-state ignore entries; it **does not** install skills/agents (hint only). [V] — `src/cmd/new/mod.rs:15-100`.
- `govctl render` writes projections (`rfc` default, `adr`, `work`, `changelog`, `all`; `--dry-run`, `-f/--force`); renders from loaded sources, reports load/render diagnostics. [V] — `src/command_router/execute/render.rs`, `src/cli/commands.rs:129-145`.
- `govctl search` keeps a disposable SQLite FTS index under `.govctl/`; `--reindex` rebuilds; freshness is required before returning results. [V] — RFC-0002:C-SEARCH-COMMAND, `src/cmd/search.rs`, Cargo deps `rusqlite bundled`.
- Source reference scanning (`[[RFC-0001]]` annotations in source) is opt-in via `[source_scan] enabled`; default include `src/**/*.rs`, `crates/**/*.rs`, `**/*.md`; `.govignore` layered over `.gitignore`. [V] — `src/config/mod.rs:237-284`, RFC-0009.
- Multi-workspace coordination state lives in a clone-shared registry (derived from VCS; `status` prints its location), never in `gov/`. [V] — RFC-0010, `src/cmd/status.rs:176-199,296-322`.

---

## 2. CLI surface inventory

### 2.1 Global shape

`src/cli/mod.rs:21-36` — `govctl [-C/--config <PATH>] [--dry-run] <COMMAND>`; `-h/--help`, `-V/--version`.

Top-level commands (visible), in clap declaration order (`src/cli/commands.rs:82-305`): **23 visible + 1 hidden**:
`init`, `init-skills`, `agent`, `check`, `status`, `render`, `migrate`, `verify`, `search`, `loop`, `claim`, `rfc`, `clause`, `adr`, `work`, `guard`, `conformance`, `release`, `describe`, `completions`, `self-update`, `tui` (default feature), `tag`; hidden: `agent hook <session-start|pre-tool-use>` (`#[command(hide = true)]`, intentionally excluded from help/completions — `src/command_router/execute/builtin.rs:129-168`).

Root help is **not** captured in snapshots; subcommand help snapshots exist for `rfc/work/adr/clause/guard/loop/check` and `rfc|adr|work|clause edit`, `rfc bump`, `* get`. [V] — `tests/snapshots/test_help__*.snap`.

### 2.2 Read-only / inspect commands

| Command | Syntax (flags) | stdout payload | Notes |
| --- | --- | --- | --- |
| `status` | `govctl status` | human table (RFC/Clause/ADR/WI/Guard/Conformance/Release counts, active work, foreign-workspace presence, Local State paths) | **No output-format flag by design** (RFC-0002:C-OUTPUT-FORMAT exempts it) [V] |
| `list` | `govctl {rfc\|clause\|adr\|work\|guard\|conformance} list [FILTER] [-n N] [-o table\|json\|yaml] [--tag a,b]` | table/json/yaml | sorted by ID; non-TTY default = **json** (`-o` optional!) [V] |
| `get` | `… get <ID> [FIELD] [-o table\|json\|yaml\|toml\|plain]` | complete resource (table/json/yaml/toml) or field value (plain/json/yaml) | no field: TTY=table, non-TTY=json; field selected: default=plain [V] |
| `show` | `… show <ID> [-o table\|json\|yaml\|toml\|plain] [--history]` | rendered markdown (table), or complete structured artifact | **default is `table` even non-TTY**; help states structured output is always complete and cannot be combined with `--history` [V] |
| `search` | `govctl search <QUERY>... [--type KIND]... [--tag TAG]... [-n N] [-o table\|json\|plain] [--reindex]` | table/json/plain (default table; default limit 20) | JSON rows: `kind,id,title,path,snippet[,score,status,scenario_path]` [V] |
| `conformance trace` | `govctl conformance trace [TARGET] [-o table\|json]` | requirement→case→guard mapping | scoped exception: only table/json; non-TTY default = **json** [V] |
| `tag list` | `govctl tag list [-o table\|json\|plain]` | table/json/plain | default table [V] |
| `loop list` | `govctl loop list [FILTER] [-n N] [-o table\|json\|plain]` | table/json/plain (default table) | filters: state, `open`, `resumable`, loop ID, WI ID [V] |
| `loop show` / `loop resume` | `govctl loop show\|resume <LOOP-ID>` | human-readable multi-line text on **stdout** | no JSON for a single loop; use `loop list -o json` for structured view [V] |
| `describe` | `govctl describe [--context]` | **JSON only**, `schema_version: 1`, `tool_version`, `purpose`, parser-owned command tree, `workflow.phase_order`; with `--context`: counts + non-terminal records + read-only `suggested_actions` | project-independent without `--context`; on context load failure prints diagnostics to stderr, exits non-zero, **no partial JSON** [V] |
| `claim list` | `govctl claim list` | one line per claim across workspaces (stdout; no `-o` flag) | coordination read; degraded/no-VCS cases print info prose via stderr [V] |
| `check` | `govctl check [-W/--deny-warnings]` | human summary + diagnostics (stderr) | exit 0 valid / 1 errors; warnings exit 0 unless `-W` [V] |
| `render --dry-run` | `govctl render [target] --dry-run` | previews (stderr) | no writes [V] |

`get` field catalog (per resource, from help text): RFC `title,version,status,phase,owners,refs,changelog`; Clause `title,kind,text,status,anchors,superseded_by,since`; ADR `title,date,status,superseded_by,context,decision,consequences,refs,alternatives`; Work Item `title,description,status,completed_at,refs,depends_on,notes,acceptance_criteria,verification.required_guards,verification.waivers`; Guard `title,refs,description,command,timeout_secs,pattern`; Conformance Case (`list/get/show`). [V]

### 2.3 Mutating / governed commands

Universal CRUD (RFC-0002:C-CRUD-VERBS), all validated before write; failed edits leave artifacts unchanged; `--dry-run` performs the same validation/target selection and reports planned writes without touching state. [V]

| Family | Commands |
| --- | --- |
| Bootstrap | `init [-f]`; `init-skills [-f] [--format claude\|codex] [--dir PATH]` |
| Create | `{rfc,adr,work,guard,conformance} new …`; `clause new RFC-NNNN:C-NAME TITLE [-s SECTION] [-k normative\|informative]`; `rfc new TITLE [--id RFC-NNNN]`; `work new TITLE [--active]`; `conformance new TITLE --path P --selector S --requirement CLAUSE@VERSION… [--guard G]… [--id CONF-…]` |
| Edit (canonical path-first, the **only** field mutation surface) | `<resource> edit <ID> <PATH>` with exactly one of `--set [VALUE]`, `--add [VALUE]`, `--remove [PATTERN]`, `--tick <done\|pending\|cancelled\|accepted\|considered\|rejected>`; `--stdin` for set/add values; `--regex`, `--all` for removes |
| Delete | `{work,clause,guard,conformance} delete <ID> [-f/--force]` (RFC/ADR use lifecycle; deletion gates: work must be `queue` + unreferenced; clauses/guards/conformance have referrer checks) |
| Run orchestration | `loop start\|replan\|add\|remove\|run`; `loop run` advances local rounds only (does not change WI state) |
| Conformance trace/CRUD | `conformance edit/delete` |
| Governance config | `tag new\|delete` (controlled vocabulary in `gov/config.toml`) |
| Rendering | `render` / `<resource> render <ID> [--dry-run]` (single-item render) |

**Lifecycle verbs** (`src/cli/resources/*.rs`, RFC-0002:C-LIFECYCLE-VERBS):

| Resource | Verbs |
| --- | --- |
| RFC | `bump <ID> --patch\|--minor\|--major [-m SUMMARY] [-c CHANGE]…` (also `-c` alone = changelog-only), `finalize <ID> normative`, `advance <ID> <spec\|impl\|test\|stable>`, `deprecate <ID> [-f]`, `supersede <ID> --by <ID> [-f]` |
| Clause | `deprecate <ID> [-f]`, `supersede <ID> --by <ID> [-f]`, `delete` (draft/unreferenced only) |
| ADR | `accept <ID> [-f]`, `reject <ID>`, `supersede <ID> --by <ID> [-f]` |
| Work Item | `move <FILE_OR_ID> <queue\|active\|done\|cancelled>` (`done` runs the guard gate) |
| Release | `release <VERSION> [--date YYYY-MM-DD]` (collects all unreleased `done` WIs; **trunk scope**), `release undo <EXPECTED_VERSION>` (only newest local cut; **trunk scope**) |
| Tag | `tag new`/`tag delete` |

### 2.4 Execution / verification commands

- `verify [GUARD-ID…]` or `verify --work <WI-ID>` — runs guards via `/bin/bash -c <command>` from the **project root** in a non-login shell inheriting the caller env (ADR-0064), default timeout 300 s per guard (`timeout_secs` overridable), output captured to temp files, process group killed on timeout, optional case-insensitive regex `pattern`; `--work` selects effective guard set after waivers; `--work` conflicts with explicit IDs. Exit non-zero if any guard fails, no guards selected, or unknown guard. [V] — `src/verification/{mod,runner/*}.rs`, `src/cmd/verify.rs`.
- `check` — schema/phase/cross-ref/tag/clause-structure validation + optional source-reference scan + schema-outdated and local-state-ignore diagnostics. Human output only. [V] — `src/cmd/check.rs`.
- `loop` — project-local execution state under `.govctl/loops/<LOOP-ID>/state.toml`; lifecycle `pending, active, paused, completed, failed`; rounds; dependency closure; `loop run` opens/closes rounds and requires summary evidence when closing (per RFC-0006). [V] — RFC-0006:C-LOOP-COMMAND-SURFACE, `src/cmd/loop_cmd/*`, `src/loop_state/*`.

### 2.5 Agent-integration / maintenance commands

- `agent doctor <codex|claude|all>` — read-only readiness probe; project-independent; non-zero if any selected runtime lacks required native `plugin` subcommands. [V]
- `agent install|update <codex|claude|all>` — materialize embedded plugin assets at user-scope path, delegate native install/update to `agent-plugin-installer`, then (Codex only) project reviewer roles. `--dry-run` = preflight + plan only, no persistent state change. [V]
- `agent hook <session-start|pre-tool-use>` — hidden stdin/stdout JSON adapter (see §3.3). [V]
- `self-update [--check]` — GitHub Releases download/replace; **`--check` exits 1 when a newer version is available**, 0 when up to date; optional `GITHUB_TOKEN`. [V] — `src/cmd/self_update.rs:64-104`, RFC-0002:C-SELF-UPDATE.
- `completions <bash|zsh|fish|powershell>` — completion script to stdout. [V]
- `tui` — read-only cockpit (default cargo feature `tui`; ratatui/crossterm/markdown-to-ansi). Mutations remain CLI-owned. [V] — RFC-0007, Cargo.toml features.
- `migrate` — transactional schema upgrade 3→5, syncs `gov/schema/`, ensures local-state ignore entries; **trunk scope**; `--dry-run` plans only. [V] — RFC-0002:C-GLOBAL-COMMANDS, `src/cmd/migrate/`.

### 2.6 Machine-readable affordances — explicit verdict

Per RFC-0002:C-OUTPUT-FORMAT, "commands that output resource data MUST support `--output/-o`" with `table|json|yaml|toml|plain` applicability: `get` complete = table/json/yaml/toml; `get` field = plain (default)/json/yaml; `list` = table(default)/json/yaml; TTY default table, non-TTY default JSON. **Implementation nuance to account for** [V]:

| Output | Flag? | stdout format | Default (non-TTY) | Stable for wrapper? |
| --- | --- | --- | --- | --- |
| `list` (all resources) | `-o table\|json\|yaml` | JSON array of row structs (`RfcSummary{id,version,status,phase,title,amended?}`, `ClauseSummary{id,rfc_id,kind,status,title}`, `AdrSummary{id,status,date,title}`, `WorkItemSummary{id,status,title}`, `GuardSummary{id,title,command}`) | **json** (auto) | **Yes** — pass `-o json` explicitly |
| `get` complete | `-o table\|json\|yaml\|toml` | resource-shaped JSON/YAML/TOML | json (auto) | **Yes** |
| `get` field | `-o plain\|json\|yaml` | scalar/list values | plain | **Yes** (`plain` ideal for scripts) |
| `show` | `-o table\|json\|yaml\|toml\|plain` | complete structured artifact or rendered md | **table always** | Yes if `-o json` passed explicitly |
| `search` | `-o table\|json\|plain` | JSON rows (kind/id/title/path/snippet/score?/status?) | **table always** | Yes if `-o json` passed |
| `conformance trace` | `-o table\|json` | graph JSON | **json** (auto) | Yes if `-o json` |
| `tag list` / `loop list` | `-o table\|json\|plain` | JSON rows | table | Yes if `-o json` |
| `describe` | none (always JSON) | JSON schema v1 | JSON | **Yes — best discovery contract** |
| `status` | none | human table stdout | table | No (human-only) |
| `check`, `verify` | none | human prose + diagnostics on **stderr** | — | Exit code + parse `E*/W*` codes only |
| `render`, `init`, `init-skills`, `new`, `edit`, all lifecycle/delete/tag-write/claim-write/agent/migrate | none | human prose on **stderr** (previews `[DRY RUN]`/`Would write:` also stderr) | — | No structured success output; rely on exit code + re-read via `get/list -o json` |
| `completions` | n/a | shell script stdout | — | Yes, n/a |

**Channel discipline**: data payloads go to **stdout** (`println!`), all informational/success/preview/diagnostic text goes to **stderr** (`eprintln!`). This is deliberate and agent-friendly; a wrapper can parse stdout without stripping prose. [V] — `src/cmd/output.rs:122-190`, `src/ui/messages/common.rs`, `src/ui/messages/artifact.rs`, `src/ui/diagnostics.rs`.

**Exit codes** [V]: `0` success (warnings do not fail; exception: `check -W`), `1` any error diagnostic/operation failure (also `self-update --check` when update available), `2` clap argument/usage errors (evidence: `tests/test_help.rs:97-106` asserts `exit: 2`). No documented distinct codes for lock timeout/claim conflict — they surface as `1` + diagnostics. `status` says exit codes are 0/1 in `docs/guide/validation.md:19-23`.

**Diagnostics** [V]:
- Format (stderr, one line): `error[E0101]: message (file)`; `warning[W…]`; `info[I…]`. Colors auto-disable when not a TTY; tests set `NO_COLOR=1`. `src/ui/diagnostics.rs:6-33`.
- Code families (stable registry; retired codes retained): RFC `E01xx`, Clause `E02xx`, ADR `E03xx`, Work Item `E04xx`, Config `E05xx`, Signature `E06xx`, Release `E07xx`, CLI/command `E08xx` (incl. `E0823` trunk-scope, `E0824` claim), general `E09xx`, Guard `E10xx`, Tag `E11xx`, Loop `E12xx`, Conformance `E13xx`, Agent `E1401`; warnings `W01xx` (incl. `W0110` schema outdated, `W0114` trunk enforcement inactive, `W0116` claim held), info `I04xx`. `src/diagnostic/code/mod.rs:14-206`.
- **Gap**: there is no `--quiet`, no `--json` error envelope, no `--format` alias, no machine-readable `status`/`check` exit summary. A wrapper should key on exit code and code strings.

Other behavior a wrapper must know [V]:
- `--dry-run` is **global** and applies to every mutating command; previews go to stderr; validation is identical; no writes/lock side effects. (`ADR-0006`, `ADR-0019`; `src/write/mod.rs:31-77`.)
- `-n/--limit` exists only on `list`, `search`, `loop list`.
- `-f/--force` exists on `init`, `init-skills`, delete/deprecate/supersede, `adr accept`; not on `work move`.
- `--stdin` is the escape hatch for rich text (backticks, `$()`); quote paths with brackets.
- `-C/--config` overrides config discovery (global).
- Writes are atomic (temp file + fsync + rename), symlink-aware (≤40 hops), with multi-file transaction rollback; previews never lock. `src/write/mod.rs:84-303`.
- Command scope: `release`/`release undo`/`migrate` must run in the primary workspace or fail E0823; when primary can't be determined, they proceed with `W0114`. `src/main.rs:178-212`.

---

## 3. Agent integration currently shipped

### 3.1 Repository plugin layout (SSOT = `.claude/`)

```
.claude/                                  # plugin content (SSOT for skills/agents/hooks)
├── skills/<13 dirs>/SKILL.md             # no auxiliary references/assets/scripts dirs exist
├── agents/{rfc,adr,wi}-reviewer.md, compliance-checker.md   # 4 Markdown agents
├── hooks/claude.json  hooks/codex.json   # runtime-specific hook manifests
├── .claude-plugin/plugin.json            # Claude plugin manifest
├── .codex-plugin/plugin.json             # Codex plugin manifest
└── .gitignore                            # settings.local.json (not embedded)
.claude-plugin/marketplace.json           # repo-root marketplace catalog (Claude Code layout)
```
[V] — `find .claude`, `build.rs:85-123`.

Manifests (quoted, 0.22.0) [V]:

| File | Content |
| --- | --- |
| `.claude-plugin/marketplace.json` | `{"$schema":"https://anthropic.com/claude-code/marketplace.schema.json","name":"govctl","plugins":[{"name":"govctl","version":"0.22.0","source":"./.claude",…}]}` |
| `.claude/.claude-plugin/plugin.json` | `{"name":"govctl","version":"0.22.0","description":"Governed workflow skills, reviewer agents, and lifecycle hooks for govctl","hooks":"./hooks/claude.json"}` |
| `.claude/.codex-plugin/plugin.json` | `{"name":"govctl","version":"0.22.0","description":"Governed workflow skills for govctl","skills":"./skills/","hooks":"./hooks/codex.json"}` |

The Claude manifest intentionally has no `skills`/`agents` fields: Claude Code auto-discovers `skills/` and `agents/` from the plugin root. [I, matches Anthropic plugin convention; govctl tests only assert hooks path]

### 3.2 `govctl agent` implementation and install flow

Normative contract: RFC-0002:C-AGENT-INTEGRATION (normative, RFC-0002 v3.8.0 phase `impl`). Key MUSTs [V]:
- `doctor`/`install`/`update` take required selector `<codex|claude|all>`; `hook` takes no runtime, consumes native event JSON on stdin, emits JSON response on stdout, MUST stay hidden.
- All four work **without an initialized govctl project**.
- `doctor` read-only; `install/update` use assets **bundled with the running binary**, materialized at a persistent user-scoped support path before registering it as a marketplace source; materialized version MUST equal running version.
- `install` MUST preserve existing Codex reviewer-role files; `update` MUST replace them.
- With `all`, validation+preflight complete for both before first native mutation; non-zero exit unless every selected runtime succeeds; failure output identifies completed/failed/skipped.
- Successful install/update MUST tell user a new session is required.
- Claude gets skills+Markdown agents+hooks via native plugin; Codex gets skills+hooks via native plugin **plus standalone TOML reviewer roles** (name/description/developer_instructions, read-only sandbox).
- Hook advice is advisory; MUST NOT be treated as enforcement.

Implementation (`src/cmd/agent.rs`) [V]:
1. `resolve_locations()`: marketplace root = `directories::ProjectDirs::from("org","govctl","govctl").data_local_dir()/agent-plugin`; Codex home = `$CODEX_HOME` or `~/.codex`.
2. `validate_bundled_versions()`: parse every embedded `*plugin.json`, require `version == CARGO_PKG_VERSION`, and require **exactly 2** manifests (`checked != 2` → E1401, `src/cmd/agent.rs:235-262`).
3. `materialize_plugin()`: write `<root>/.claude-plugin/marketplace.json` with `name="govctl-bundled"`, plugin `version`, `source="./versions/<version>/.claude"`; write all `PLUGIN_ASSETS` under `<root>/versions/<version>/.claude/…` (`src/cmd/agent.rs:186-233`).
4. Preflight per selected runtime (doctor-mapped probes).
5. Native ops via `agent-plugin-installer 0.6.1`: `install_many(..., FailurePolicy::StopOnFailure)` or `update_from_source_many(...)`; Claude request adds `marketplace_scope="user"` + `plugin_scope="user"`; plugin selector `govctl@govctl-bundled`.
6. Codex-only `project_codex_roles()` → `<codex_home>/agents/<name>.toml` (install skips existing, update overwrites).
7. Emit per-runtime success/failure, then hint "Start a new agent session to load the refreshed integration."

Exact native commands (verified in crate source 0.6.1 + asserted by `tests/test_agent_plugin.rs:205-356`) [V]:

| Runtime | install | update (source-aware) | `govctl agent doctor` probes (Update = broadest probe) |
| --- | --- | --- | --- |
| Codex | `codex plugin marketplace add <root>` → `codex plugin add govctl@govctl-bundled` | `codex plugin marketplace add <root>` → `codex plugin add govctl@govctl-bundled` (no `marketplace upgrade`) | `codex plugin marketplace add --help`, `codex plugin add --help` (Install probe set only, `src/cmd/agent.rs:136-153`) |
| Claude | `claude plugin marketplace add --scope user <root>` → `claude plugin install --scope user govctl@govctl-bundled` | `claude plugin marketplace add --scope user <root>` → `claude plugin update --scope user govctl@govctl-bundled` | `claude plugin marketplace add --help`, `claude plugin install --help`, `claude plugin marketplace update --help`, `claude plugin update --help` (Install+Update probe sets) |

Note: the crate's own `doctor()` runs a broader fixed probe list (including `plugin --help` / `marketplace upgrade` / `uninstall`), but govctl deliberately uses `check_operation(runtime, Update)` as the "broadest readiness probe" instead; `tests/test_agent_plugin.rs` asserts doctor only probes `--help` forms and that install/update run all-runtime preflight before any mutation. [V]

Crate facts: `DEFAULT_COMMAND_TIMEOUT = 60 s`; child stdin closed; stdout/stderr captured (not forwarded); Codex supports git-ref/sparse marketplace sources, Claude supports scopes+sparse; commands are hard-coded per runtime (`AgentRuntime::install_commands()/update_commands()/source_update_commands()`, docs.rs source for 0.6.1). govctl does not expose `uninstall` (`_ => Err("Unsupported agent integration operation")`). [V]

### 3.3 `agent_hooks.rs` — exact hook contract (the most reusable protocol)

Source: `src/cmd/agent_hooks.rs`; behavior locked by `tests/test_agent_plugin.rs:358-559`. [V]

**Invocation**: `govctl agent hook <session-start|pre-tool-use>`; native manifests call it via bare `govctl` from `PATH` (not a plugin-relative path) with `timeout: 5` seconds.

**stdin payload (Rust struct, extra fields ignored; `cwd` required)** — quoted:

```rust
#[derive(Deserialize)]
struct HookInput {
    cwd: PathBuf,                 // relative values are joined to the process CWD and normalized
    #[serde(default)]
    tool_input: Value,            // object; Claude Edit/Write uses file_path; Codex apply_patch uses command
}
```

**request context**: global `--dry-run`/`-C` not used; `hook` is project-independent at config load, then discovers the governed project upward from `input.cwd` (`Config::discover_from` / `Config::governed_root_from`).

**SessionStart behavior** [V]:
- No governed project → **exit 0, no stdout** (silent).
- Governed project with load failure → bounded recovery context: `"govctl governance state was detected but could not be loaded: <diag>. Repair the reported state or run `govctl check` for full diagnostics."`
- Valid project: bounded context lines — `govctl project: <name> (<root>)`; optional `Work Item load warnings:` (up to 8 + `- ... N more`); `Active work:` (WI id/title + `[refs: …]`, up to 8); `Open loops:` (`<id>: <state> round <n>, next <action> [work: …]`, up to 8); `Next: govctl loop resume <ID>` / `Next: govctl work show <WI-ID>`; ≤8 items per section (`HOOK_ITEM_LIMIT = 8`), hard truncation to **4,000 chars** with `\n... context truncated` (`HOOK_CONTEXT_CHAR_LIMIT = 4_000`, `bound_context`). No project-wide validation.
- Successful event output is JSON on stdout (one line + newline):

```json
{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"<bounded context>"}}
```

**PreToolUse behavior** [V]:
- Only when a governed project root exists and the tool touches a lifecycle-managed path. Managed = any path under `<project_root>/gov/{rfc,adr,work,guard,conformance}/**` or `gov/releases.toml`; **`gov/config.toml` and source files are not managed**.
- Target extraction: `tool_input.file_path` (Claude `Edit|Write`) OR any line of `tool_input.command` beginning with `*** Add File: `, `*** Update File: `, `*** Delete File: `, `*** Move to: ` (Codex apply_patch), trimmed.
- Emits fixed advisory as `additionalContext` (exact text):
  > This edit targets a lifecycle-managed govctl artifact. Prefer the resource-specific `govctl ... edit` or lifecycle command when it can express the change. Direct editing is permitted when the current CLI has no suitable operation; preserve lifecycle-owned fields and run `govctl check` after the edit.
- **Never blocks**: no `permissionDecision`/`decision` field is emitted; tool call proceeds (tests assert absence).
- Unmanaged/non-governed target → exit 0, empty stdout.

**Errors**: unreadable/invalid stdin JSON or serialization failure → `DiagnosticCode::E1401AgentIntegrationFailed` printed to stderr and process exit **1** (not a hook-protocol error object). [V] — `read_hook_input`, `hook_error`.

**Hook manifests (quoted)** [V]:

`.claude/hooks/claude.json`:
```json
{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"govctl agent hook session-start","timeout":5}]}],
          "PreToolUse":[{"matcher":"Edit|Write","hooks":[{"type":"command","command":"govctl agent hook pre-tool-use","timeout":5}]}]}}
```
`.claude/hooks/codex.json`: same events/commands, plus `"additionalContextLimit": 1200` on each command (Codex-specific field; Claude manifest must not have it). Tests assert **no `Stop` hook** exists in either manifest.

**Contract caveats for DSH**: the stdin/output envelope is Claude Code's `hookSpecificOutput` shape (with Codex only adding a context limit); `timeout` is enforced by the *native client*, not by govctl; "matcher" `Edit|Write` is Claude-tool-name-based. DSH 0.2.1-rc.5 does not expose this hook protocol (Claude/Codex hook bundles appear absent from the installed packages), so this contract is **not reusable as-is** — a DSH plugin must implement context injection in-process instead. [I for DSH; V for contract]

### 3.4 `build.rs` embedding of plugin + skill assets

`build.rs` runs at compile time and generates Rust source in `OUT_DIR` [V]:
- `generate_skill_assets()` (`build.rs:53-83`): for `DISTRIBUTED_SKILL_DIRS` = `discuss, gov, quick, spec, rfc-writer, adr-writer, wi-writer, guard-writer, commit, migrate, decision-analysis, detach` (12; **`init` deliberately omitted** — plugin/global-only), recursively embed every file from `.claude/skills/<dir>/**` as `SKILL_ASSETS: &[(&str, &str)]` → used by `init-skills` (`src/cmd/new/skills.rs:8-10`).
- `generate_plugin_assets()` (`build.rs:85-123`): recursively embed `.claude/{.claude-plugin,.codex-plugin,skills,agents,hooks}/**` as `PLUGIN_ASSETS: &[(&str, &str)]` (paths are plugin-root-relative, so `.claude/.claude-plugin/plugin.json` → `.claude-plugin/plugin.json`); embeds `.claude-plugin/marketplace.json` as `PLUGIN_MARKETPLACE_TEMPLATE` → used by `govctl agent` (`src/cmd/agent.rs:18`).
- `generate_codex_agent_templates()` (`build_support/agent_templates.rs:30-68`): converts each `.claude/agents/*.md` (only `name`/`description` parsed from YAML frontmatter) into a Codex role TOML `{name, description, sandbox_mode="read-only", developer_instructions=<body>}`, embedded as `AGENT_TEMPLATES_CODEX`.
- Also generates edit-rule tables and runtime field specs from `gov/schema/edit-ops.json` (SSOT) — not agent-related but part of build.

`cargo:rerun-if-changed` covers `.claude` assets, `gov/schema/edit-ops.*`, `build_support/*`. Cargo package `include` lists `.claude-plugin/*`, `.claude/.{claude,codex}-plugin/*`, `.claude/agents/*`, `.claude/hooks/*`, `.claude/skills/**/*`, `gov/schema/*.json`, `gov/templates/*` so `cargo install`/crates.io builds have the assets — a DSH-facing runtime would need its manifest roots added to both `build.rs` and `Cargo.toml` include. [V]

### 3.5 `init-skills` (project-local projection, separate from agent install)

`govctl init-skills [-f] [--format claude|codex] [--dir PATH]` [V] — `src/cli/commands.rs:92-105`, `src/cmd/new/skills.rs:36-134`, RFC-0002:C-GLOBAL-COMMANDS item 9:
- Destination resolution: `--dir` > `paths.agent_dir` from config > format default (`.claude` / `.codex`); relative `--dir` resolves against the project root (`gov_root.parent()`).
- Writes the 12 distributed skill bundles recursively (`skills/<name>/SKILL.md`) + 4 agents in the format's shape (`claude`: `agents/*.md`; `codex`: `agents/*.toml`); skips existing files unless `-f`; reports created/updated/skipped counts; `--dry-run` previews.
- **DSH bridge available today**: in a governed project, `govctl init-skills --dir .dsh` writes `.dsh/skills/<name>/SKILL.md` (plus an ignored-by-DSH `.dsh/agents/*.md`). This matches DSH's project skill root `<projectRoot>/.dsh/skills` (DSH platform notes, §7). [I — depends on DSH tolerating extra directories / unknown frontmatter]

---

## 4. Skill & agent inventory

### 4.1 Skills (13 total; 12 distributed locally, all 13 in plugin)

All are single `SKILL.md` files (no `references/`, `assets/`, `scripts/` subdirs exist). Frontmatter = `name`, `description`, plus Claude-specific `allowed-tools` (12 of 13) and `argument-hint` (12 of 13); `decision-analysis` has neither. [V]

| Skill | Role class | Description (frontmatter) | `allowed-tools` | `argument-hint` |
| --- | --- | --- | --- | --- |
| `init` | workflow (onboarding, plugin-only; **not** distributed by `init-skills`) | "Set up govctl in the current project with explicit installation and overwrite authorization" | Read, Write, Edit, Bash, Glob, Grep, TodoWrite | `[optional setup scope]` |
| `discuss` | workflow (design→draft RFC/ADR) | "Explore a governance design, resolve ambiguity, and draft RFC or ADR artifacts without implementation" | same 7 | `<topic-or-question>`; body uses `$ARGUMENTS` |
| `spec` | workflow (artifact maintenance, no code) | "Maintain RFC and ADR artifacts without implementation work" | same 7 | `<artifact-maintenance-task>`; uses `$ARGUMENTS` |
| `gov` | workflow (main execution loop) | "Execute governed implementation with work-item traceability, RFC/ADR authority, risk-scoped verification, and closure" | same 7 | `<what-to-do>`; uses `$ARGUMENTS` |
| `quick` | workflow (trivial non-behavioral) | "Execute a trivial non-behavioral change without unnecessary governance ceremony" | same 7 | `<what-to-do>`; uses `$ARGUMENTS` |
| `commit` | workflow (VCS commit) | "Record a focused VCS commit with govctl validation and Work Item traceability" | same 7 | `[optional commit message hint]` |
| `migrate` | workflow (brownfield adoption) | "Adopt govctl in an existing project by discovering and confirming historical decisions, specifications, and active work before backfilling a governed baseline" | same 7 | `[optional scope hint, e.g. "focus on database decisions"]` |
| `detach` | workflow (removal) | "Remove project-local govctl integration through an ownership-aware dry run, archival plan, explicit confirmation, and recoverable edits" | same 7 | `[optional scope hint]` |
| `rfc-writer` | writer helper | "Write precise, implementation-independent RFCs with testable normative clauses and correct lifecycle handoff" | same 7 | `[optional RFC topic]` |
| `adr-writer` | writer helper | "Write decision records that explain context, evaluated alternatives, chosen direction, and consequences without creating product obligations" | same 7 | `[optional ADR topic]` |
| `wi-writer` | writer helper | "Write durable work items with scoped descriptions, testable categorized acceptance criteria, governing references, and risk-matched guards" | same 7 | `[optional work-item topic]` |
| `guard-writer` | writer helper | "Define reusable, non-interactive verification guards with stable commands and risk-matched scope" | same 7 | `[optional guard topic]` |
| `decision-analysis` | reference-only | "Stress-test consequential design options with premortem and backcast reasoning, then return a risk-calibrated recommendation to the owning governance workflow" | — | — |

Workflow-critical: `gov`, `quick`, `discuss`, `spec`, `commit`, `init`, `migrate`, `detach` (the 8 "workflow skills" in `AGENTS.md` §6). Helper: the 4 writers. Reference: `decision-analysis`. [V]

**Portability notes for DSH** [V for content, I for DSH mapping]:
- `allowed-tools` and `argument-hint` are Claude Code frontmatter; generic skill loaders minimally need `name` + `description` (DSH's summary model additionally exposes `whenToUse` and invocation controls — how SKILL.md frontmatter maps onto that must be confirmed against installed 0.2.1-rc.5; `task-1`). Unknown fields are likely tolerated but unverified; strip or remap the two Claude fields for DSH.
- `$ARGUMENTS` substitution appears in `discuss`, `gov`, `quick`, `spec`; must be replaced with explicit parameter text for DSH.
- Skills are deeply cross-referential ("hand off to `quick`/`spec`", "run `rfc-reviewer`/`adr-reviewer`/`compliance-checker`"). Skills→skill handoff maps to DSH skills; skills→**agent** delegation assumes Claude subagents, so DSH needs either agent presets/subagent providers for the 4 reviewers or in-skill review instructions. Agents are not plain SKILL.md.
- Skills encode workflow policy but never duplicate CLI syntax beyond examples; they explicitly say "use `govctl <resource> --help` for current syntax". This makes them portable across govctl versions.
- Skill bodies mention `jj` and `git`; `commit` supports both. Environment-specific but not Claude-specific.

### 4.2 Agents (4; Markdown with YAML frontmatter, `name` + `description` only)

| Agent | Class | Description | Notes |
| --- | --- | --- | --- |
| `rfc-reviewer` | review | "Review RFC drafts for contract clarity, normative correctness, completeness, and artifact-authority boundaries" | read-only reviewer; body instructs "Review only. Do not edit artifacts, create Work Items, run lifecycle verbs, or use VCS." |
| `adr-reviewer` | review | "Review ADR drafts for decision evidence, credible alternatives, honest consequences, projection ownership, and authority boundaries" | read-only |
| `wi-reviewer` | review | "Review Work Items for durable scope, testable categorized outcomes, governing authority, dependencies, notes, and risk-matched verification" | read-only |
| `compliance-checker` | audit | "Audit implementation against normative RFC obligations and report separate design drift from accepted ADR rationale" | read-only |

Claude representation = `.claude/agents/<name>.md` (frontmatter only name/description; no `tools:`/`model:` fields). Codex representation = generated TOML `{name, description, sandbox_mode="read-only", developer_instructions=<markdown body>}`. No equivalent DSH agent-preset artifact exists today; DSH-side mapping is a design task (DSH has agent-team/subagent/preset seams — see platform research). [V]

---

## 5. Runtime / build / distribution facts

### 5.1 Local environment

- No Rust toolchain, no `target/`, no installed `govctl` (§Environment). Build/startup/size measurements impossible locally. [V]
- `AGENTS.md` prescribes dev invocation `cargo run --quiet -- …`; that only works on a machine with Rust 1.96+. [V]
- Repo HEAD is `main` @ `4223933` (chore commit), crate version 0.22.0; latest governed release `0.22.0` (2026-10-09, WIs `WI-2026-10-09-001/002`); 59 releases recorded in `gov/releases.toml`. [V]

### 5.2 How users obtain the binary

- `cargo install govctl` (crates.io, README) or `cargo binstall govctl` (prebuilt). [V] — `README.md:105-125`.
- `[package.metadata.binstall]` in `Cargo.toml:108-131`: `pkg-url = "{repo}/releases/download/v{version}/govctl-v{version}-{target}.tar.gz"`, `bin-dir = "govctl-v{version}-{target}/{bin}{binary-ext}"`, `pkg-fmt = tgz`; Windows overrides map MSVC targets to GNU/gnullvm zips. Governed by ADR-0060. [V]
- `govctl self-update` downloads from `govctl-org/govctl` GitHub Releases and replaces the running exe; `--check` returns 1 when an update exists (useful for wrappers/CI). [V]
- `cargo-llvm-cov`/coverage and `just pre-commit` are dev-only. [V]

### 5.3 Release artifacts & targets (GitHub Releases API, `v0.22.0`, published 2026-10-09) [V]

Canonical build targets (Zig cross-build in CI, musl for Linux; smoke-tested on native OS/arch):

| Target | Archive | Size (as published) |
| --- | --- | --- |
| `x86_64-unknown-linux-musl` (canonical) — also published under 0.x alias `x86_64-unknown-linux-gnu` (byte-identical exe, repackaged) | tar.gz | 8,827,695 B / alias 8,827,617 B |
| `aarch64-unknown-linux-musl` (canonical) — also alias `aarch64-unknown-linux-gnu` | tar.gz | 8,197,387 B / alias 8,197,539 B |
| `x86_64-apple-darwin` | tar.gz | 9,327,495 B |
| `aarch64-apple-darwin` | tar.gz | 8,868,167 B |
| `x86_64-pc-windows-gnu` (canonical) — also alias `x86_64-pc-windows-msvc` | zip | 10,878,578 B / alias 10,878,586 B |
| `aarch64-pc-windows-gnullvm` (canonical) — also alias `aarch64-pc-windows-msvc` | zip | 7,563,294 B / alias 7,563,270 B |

(0.x releases publish four duplicate "compat alias" archives by repackaging canonical exes — ADR-0060/RFC-0002:C-PRE-1-RELEASE-TARGET-COMPATIBILITY.) **Uncompressed binary size and startup time were not measurable here** [U]; archives are ~8–11 MB compressed, so bundling the binary inside a DSH plugin is heavy and version-coupled — prefer resolving `govctl` from PATH plus an explicit install/upgrade path. [I]

### 5.4 CI (`.github/workflows/ci.yml`, `release.yml`) [V]

- CI: fmt, clippy `-D warnings`, `cargo test --all-features`, llvm-cov+Codecov, `cargo build --release`, then `./target/release/govctl check`; separate macOS/Windows release builds; `jj` installed for workspace tests.
- Release: tag `v*` → verify version/CHANGELOG/releases.toml consistency + tag reachable from main → Zig cross-build matrix → package tar.gz/zip → smoke (`--version`, `check`, `self-update --check`) on native runners → GitHub release. No crates.io publish step visible in these workflows (crates.io release is manual/other). [V]
- `rust-version = 1.96`, edition 2024; default feature `tui` pulls ratatui/crossterm/markdown-to-ansi/ansi-to-tui (TUI can be excluded with `--no-default-features`). [V]

### 5.5 What is embedded in the binary (packaging relevance)

Binary embeds: 12 skill bundles, 4 Claude agents, 4 generated Codex agent TOMLs, 2 plugin manifests + 2 hook manifests, the marketplace template, 9 JSON schemas (7 artifact schemas + loop-state + loop-round), and edit-rule tables. So `govctl` alone can install the full agent integration offline (no network, no adjacent asset files). Any DSH packaging that also embeds assets would duplicate what the binary already carries; using `govctl agent` output or `init-skills` is preferable. (`gov/templates/*.toml` are repo authoring aids for this project, not embedded release assets; `init` only creates an empty `gov/templates/` dir.) [I from build.rs + `src/schema.rs` + `agent.rs`]

---

## 6. Extension seams for a new agent runtime (`dsh`)

### 6.1 Where runtimes are selected/validated in govctl

- CLI: `Commands::Agent` fields are typed `agent_plugin_installer::AgentSelector` (clap `ValueEnum` from the crate feature `clap`); allowed values are fixed by the crate: `codex`, `claude`, `all`. [V] — `src/cli/commands.rs:9-35`, `Cargo.toml:46`.
- Planning: `BuiltinOp::Agent { operation, selector }` carries `agent_plugin_installer::{AgentPluginOperation, AgentSelector}`; `AgentPluginOperation` includes `Uninstall`, but govctl rejects it. [V] — `src/command_router/plan.rs:40-43`, `src/cmd/agent.rs:30-42`.
- Execution: `src/cmd/agent.rs` matches `AgentRuntime::Claude | AgentRuntime::Codex` for probe sets, install request scoping, and Codex role projection; `selects_codex()` gates role projection; version validation assumes exactly 2 embedded plugin manifests (`checked != 2` error). [V]
- Hooks: out-of-process adapter contract hard-coded around Claude/Codex event payloads + Claude hook output envelope; runtime-specific manifests selected per client (`hooks/claude.json`, `hooks/codex.json`). [V]
- Project-local: `SkillFormat` = `claude | codex` only (`src/cli/common/targets.rs`); `Config.paths.agent_dir` default `.claude`. [V]

### 6.2 The external boundary: `agent-plugin-installer 0.6.1`

Fetched source/docs (docs.rs + upstream `main`; 0.6.1 source confirmed for the parts govctl uses) [V]:

```rust
pub enum AgentRuntime { Codex, Claude }        // closed; two variants; methods id()/cli()/supported()/readiness_commands()/install_commands()/…
pub enum AgentSelector { Codex, Claude, All }  // #[non_exhaustive]; str parse only codex|claude|all
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
```

- All native command sequences are **hard-coded per runtime inside the crate** (install/update/update_from_source/uninstall/doctor), plus capability flags `supports_marketplace_git_ref()` (Codex) and `supports_scopes()` (Claude). Callers cannot register a runtime.
- Batch helpers (`install_many`, `update_many`, `update_from_source_many`, `uninstall_many`): all-runtime validation → all-runtime preflight → mutations; `FailurePolicy::{StopOnFailure,Continue}`; reports carry per-runtime status and only commands that actually spawned.
- **Therefore: a `dsh` runtime cannot be added in govctl alone.** Options: (a) upstream PR to `agent-plugin-installer` adding `AgentRuntime::Dsh` + command mapping + selector value + `FromStr`/ValueEnum release, then bump govctl's pin and extend govctl; or (b) govctl replaces/wraps the crate for DSH-specific orchestration; or (c) don't touch this surface at all (separate DSH package).

### 6.3 Hypothetical `govctl agent install dsh` — what it would require

**Upstream crate (new version):**
1. `AgentRuntime::Dsh` (binary `dsh`, id `dsh`) + `supported()` addition. `AgentRuntime` is **not** `#[non_exhaustive]`, so adding a variant is semver-breaking for exhaustive matchers; `AgentSelector` is `#[non_exhaustive]` so adding `Dsh` there is additive (but `FromStr` currently hard-codes the accepted strings and error text).
2. Native DSH command mapping. Unlike Codex/Claude, DSH has **no `marketplace`/`plugin install` analogue**: DSH "plugin" is pnpm inside a profile (`dsh plugin --profile <name> add <dir-or-package>`), plus Cordis bundle manifest + patch file. A Dsh runtime adapter would need at least: profile selection, `dsh plugin --profile <name> add <package>`, probably an install/update/uninstall mapping, and scopes/git-ref capability flags. This is a substantially bigger abstraction change than adding a variant.
3. New readiness probes for whatever DSH CLI is stable (`dsh --version`, `dsh plugin … --help`), plus tests.

**govctl (after crate bump):**
1. `src/cli/commands.rs` / `help.rs::AGENT` text (`codex|claude|all` documented normatively in RFC-0002 too) + `init` hint text (`/plugin install govctl@govctl` is Claude-specific).
2. `src/cmd/agent.rs`: add runtime branches for probe matrix, request scoping, role projection (DSH has no standalone reviewer-role file format like Codex TOML — reviewers would need DSH agent presets/subagents or be dropped), version-manifest count (`checked != 2` → N), and `selects_codex`-style gates.
3. New embedded DSH package assets: a `.claude/.dsh-plugin/…`-style directory holding a DSH `package.json` (`dsh.bundle`), `cordis.patch.yml`, and a JS/TS entry; add the directory to `build.rs::generate_plugin_assets` roots, `Cargo.toml` `include`, and `validate_bundled_versions`. Alternatively materialize only skills into `.dsh/skills`.
4. Hook strategy: govctl's `agent hook` adapter speaks Claude Code's protocol; DSH has no matching out-of-process hook contract in 0.2.1-rc.5 (Claude/Codex hook compat bundles absent). A DSH runtime would need either a new in-process TS plugin that injects context (not `govctl agent hook`), or a new DSH-specific subcommand + event schema (new RFC clause).
5. Project-local projection: `SkillFormat::Dsh` for `init-skills` writing `.dsh/skills/**` (+ maybe `.dsh/agents`), and docs. (Until then, `init-skills --dir .dsh` already writes to the DSH project skill root.)
6. **Governance**: RFC-0002:C-AGENT-INTEGRATION is normative and enumerates the runtime selector and client projections; adding a third runtime changes normative CLI behavior → RFC amendment (and likely a new ADR) before implementation under the repo's own laws. `AGENTS.md` §2: "Execution MUST NOT begin on new features until RFC is normative." [V for rule; I for classification]

Effort/risk: high (external crate release chain + RFC amendment + new packaging model). Feasibility today: **no**.

### 6.4 Separate DSH bundle (recommended zero-govctl-change path)

What the DSH side needs (from DSH docs; see caveat below):
- **Package shape**: npm package with `package.json` declaring `dsh.bundle.patch` → `cordis.patch.yml` → plugin rows referencing the package entry; JS/TS entry exports `{name, apply(ctx)}` (Cordis plugin); install via `dsh plugin --profile <name> add <path-or-package>`; profile is at `$DSH_HOME/profiles/<name>`; loading order bundles → profile patch → home patch → `--patch`. [V for docs checkout; installed-version details owned by `task-1`]
- **Tools**: register via `ctx.tools.register`/`defineTool` (schema/policy hooks); suggested thin tools wrapping the stable govctl surface: `govctl_describe`, `govctl_status_context` (`describe --context`), `govctl_search`, `govctl_read` (`list/get/show -o json`), `govctl_check` (exit code + stderr codes), `govctl_verify`, `govctl_edit`/`govctl_lifecycle` (mutations; consider approval policy), `govctl_loop`, `govctl_claim`. [I — mapping suggestion only]
- **Skills**: ship the 13 `SKILL.md` files (plus strip/remap `allowed-tools`/`argument-hint`/`$ARGUMENTS`) either as packaged DSH skill assets or by writing to `<projectRoot>/.dsh/skills` at install time (or instruct users to run `govctl init-skills --dir .dsh`). DSH local skill roots: `.dsh/skills` rank 100, `.agents/skills` rank 200, custom dirs, `$DSH_HOME/skills`, bundled dir; identity = kebab-case name, bundle `<name>/SKILL.md` or flat `<name>.md`, no nested `**/SKILL.md`. [V for docs; template applies to installed version]
- **Commands/prompt**: optional DSH commands (`/gov`, `/quick`, …) and a system-prompt section summarizing governance rules; agents (4 reviewers) need DSH agent presets/subagents or folded into skills. [I]
- **Binary provisioning**: do **not** vendor the ~8–11 MB binary in the npm package by default; resolve `govctl` from PATH, then a configurable path; on absence, return an actionable error pointing to `cargo binstall govctl`/release URL. Optionally offer a postinstall/asset-download path. [I]
- **Hook/context substitution**: implement session-start context in-process (call `govctl describe --context` / `govctl status` at session start and inject bounded text; in-process edits guidance can be a tool policy note) rather than porting the Claude hook protocol. [I]

Why this is the lower-risk route: it needs **no** change to govctl's normative behavior, no upstream crate release, no Rust toolchain, and stays independent of govctl's release cadence; it can reuse govctl's JSON read contracts and skills verbatim. It does duplicate the skill assets and a thin CLI knowledge layer; mitigate by pinning to govctl version and generating the tool list/skills from the repo (or from `govctl describe`).

### 6.5 What is unknown / out of scope here

- Exact installed DSH 0.2.1-rc.5 plugin API surface, tool registration signatures, agent preset format, whether unknown skill frontmatter is tolerated — owned by `task-1` (`research/dsh-platform-notes.md`). The DSH facts above come from the stale local checkout `/Users/liuxinyu/Projects/deepseek-harness` (docs claim 0.1.1-rc.2 in places) and must be re-validated against installed 0.2.1-rc.5.
- Whether DSH has any Claude-Code-hook compatibility layer in 0.2.1-rc.5 (task-1 reports Claude/Codex hook bundles appear absent; treated here as "not reusable").
- govctl startup latency and uncompressed binary size (no toolchain/binary locally).
- Whether crates.io 0.22.0 is published and exactly which binstall/`cargo install` path users will have; README asserts both, not re-verified via crates.io API.
- Exact schema of `get -o json` for each artifact (it mirrors the wire TOML layout; verified shape only structurally, not field-by-field for all resources).
- Any govctl-side active work already targeting a DSH integration (would require loading `gov/work`; not done here — governance analyst covers process/prior art).

---

## 7. Evidence index (primary files read)

| Topic | Evidence |
| --- | --- |
| Product overview / install | `README.md`, `docs/INTRODUCTION.md`, `docs/guide/getting-started.md` |
| Artifact model/lifecycles | `gov/RFC-0000/clauses/C-SUMMARY.toml`, `gov/RFC-0001/clauses/C-{GATE-CONDITIONS,RFC-STATUS,RFC-PHASE,WORK-STATUS}.toml`, `src/model/*.rs`, `gov/schema/*.json` |
| CLI grammar | `src/cli/commands.rs`, `src/cli/resources/*.rs`, `src/cli/common/*.rs`, `src/cli/loop_cmd.rs`, `src/cli/help.rs` |
| CLI behavior | `src/command_router/{plan.rs,execute/*.rs}`, `src/cmd/**`, `src/main.rs`, `src/write/mod.rs` |
| Output contracts | `gov/RFC-0002/clauses/C-OUTPUT-FORMAT.toml`, `C-DESCRIBE-COMMAND.toml`, `C-SEARCH-COMMAND.toml`; `src/cmd/output.rs`, `src/cmd/list/output.rs`, `src/cmd/describe/*`, `src/ui/*` |
| Exit codes/diagnostics | `src/main.rs:45-90`, `src/ui/diagnostics.rs`, `src/diagnostic/code/mod.rs`, `tests/test_help.rs:97-106`, `docs/guide/validation.md` |
| Agent integration contract | `gov/RFC-0002/clauses/C-AGENT-INTEGRATION.toml`; `gov/adr/ADR-0061-*.toml`; superseded `ADR-0033`, `ADR-0015`, `ADR-0058` |
| Agent implementation | `src/cmd/agent.rs`, `src/cmd/agent_hooks.rs`, `build.rs`, `build_support/agent_templates.rs`, `src/cmd/new/{mod,skills}.rs` |
| Plugin assets | `.claude/**`, `.claude-plugin/marketplace.json`, `Cargo.toml` `include`/`binstall` |
| Hook tests | `tests/test_agent_plugin.rs` (incl. exact manifests/commands assertions), `tests/test_agent_dir.rs` |
| Help snapshots | `tests/snapshots/test_help__*.snap` |
| Runtime/distribution | `.github/workflows/{ci,release}.yml`, `Cargo.toml`, `gov/adr/ADR-{0041,0060}-*.toml`, GitHub Releases API `https://api.github.com/repos/govctl-org/govctl/releases/latest` |
| Crate boundary | `agent-plugin-installer 0.6.1` docs.rs (`https://docs.rs/agent-plugin-installer/0.6.1/agent_plugin_installer/`) + source (`https://docs.rs/agent-plugin-installer/0.6.1/src/agent_plugin_installer/lib.rs.html`) + upstream `main` (`https://github.com/lucifer1004/agent-plugin-installer`) |
| Multi-workspace/locking | `gov/RFC-0010/clauses/C-{COMMAND-SCOPE,ARTIFACT-CLAIM}.toml`, `src/registry.rs`, `src/lock.rs` |
| DSH platform (preliminary) | `/Users/liuxinyu/Projects/deepseek-harness/docs/subsystems/skills.md`, `docs/user/develop/basic/publish.md` — version-skewed; installed-version facts owned by `task-1` |

## 8. Gaps / questions for the Lead

1. **Diagnostics are not machine-readable** (stderr prose + exit codes only). Decide whether the DSH plugin accepts that or adds a wrapper-side parser/`govctl check`-based result synthesis. (A govctl `--json` diagnostics flag would be a new feature → RFC change.)
2. **DSH install model mismatch** must be settled with `task-1`: is the intended deliverable an npm/Cordis bundle, a skills-only asset drop, or both? That determines whether govctl needs any change at all.
3. **Binary provisioning policy**: PATH vs vendored vs auto-download vs `cargo binstall`; no local binary exists here, so the plugin must handle "not installed" gracefully.
4. **Skill/agent port fidelity**: `$ARGUMENTS`, `allowed-tools`, `argument-hint`, and skills→reviewer-agent references need a documented transformation; decide whether to upstream a neutral skill format to govctl (RFC change) or transform in the DSH package.
5. **Governance path** for anything that changes normative CLI behavior (`agent install dsh`, new hook protocol): RFC-0002 amendment/ADR first — `task-3` owns the authoritative answer; this inventory only supplies the technical seam facts.
