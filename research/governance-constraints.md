# Governance constraints for making govctl installable/usable as a DeepSeek Harness (DSH) plugin

Status: research (read-only). Date: 2026-10-31. Author: governance-analyst.
Repo: `/Users/liuxinyu/Projects/govctl` (all paths below are relative to it unless absolute).
Question: what does govctl's own law require, and what does prior art say, before an eventual
implementation can add a DSH integration — and what authorization/artifacts would it need?

---

## TL;DR

1. **There is no "free" way to add `dsh` to the govctl CLI.** RFC-0002 is `normative`, phase
   `impl`, version **3.8.0**, sealed; its agent clause normatively enumerates
   `<codex|claude|all>` and the exact shared behavior. Any content change is an *unversioned
   amendment* that MUST be released by a version bump before further phase progression — so an
   in-repo DSH runtime needs: RFC-0002 amendment (edit content → `rfc bump minor` → phase `spec` →
   `advance impl` seal) + an ADR + Work Items + a minor release. An ADR alone does **not**
   authorize new CLI behavior.
2. **A DSH-side package is the only route that avoids RFC-0002 normative work** — provided it
   adds *no govctl CLI surface* and does not change existing command behavior. A DSH plugin is
   an npm plugin bundle installed with `dsh plugin --profile <name> add <pkg>`; DSH skills are
   `SKILL.md` bundles, the same convention as `.claude/skills/*`. Such a package still needs an
   ADR (design choice), Work Items, docs and release bookkeeping if govctl ships/endorses it, and
   it must answer asset SSOT/version-alignment and the "one installation facade" intent of
   ADR-0061.
3. The repo is **mid-version**: 0.22.0 released (tag v0.22.0), two done-but-unreleased WIs from
   2026-10-18, no active/queued WIs, no proposed ADRs, no draft RFCs. Research/spike is
   ungoverned; implementation is not.
4. **Hard blocker for the native path:** the `agent-plugin-installer` crate (0.6.1, third-party)
   only orchestrates Codex and Claude; RFC-0002 delegates native install/update to it and requires
   bundled assets materialized at a user-scoped support path with version equality.

---

## 0. Method and limits

- `cargo` is not installed in this environment (`cargo: command not found`), and
  `target/debug/govctl` does not exist, so governed CLI views (`govctl status`, `rfc show`, ...)
  could not be run. All state below was reconstructed by parsing the authoritative TOML under
  `gov/` (AGENTS.md §1: `gov/` is SSOT; `docs/` projections are generated). This is sufficient for
  a constraints report but the eventual implementer must re-run `govctl status` before acting.
- No `gov/` artifact, `src/`, or `.claude/` file was modified; no lifecycle verb was invoked.
  This document is the only write.

---

## 1. Current repo state (what an implementation would inherit)

| Fact | Value | Evidence |
| --- | --- | --- |
| govctl version | `0.22.0` | `Cargo.toml:3`; newest `gov/releases.toml` entry 0.22.0 / 2026-10-09 (59 releases) |
| Git state | branch `main`, clean at research start, tag `v0.22.0` exists; only `research/` is untracked | `git status`, `git tag --list 'v0.22*'` |
| Unreleased work | `WI-2026-10-18-001`, `WI-2026-10-18-002` done but not in any release | `CHANGELOG.md` `[Unreleased]`; `gov/releases.toml` |
| Work Items | 272 total, **all `done`**; none queue/active | parsed `gov/work/*.toml` |
| RFCs | 10, all `normative`, none `draft`/`deprecated` | parsed `gov/rfc/*/rfc.toml` |
| RFC phases | `impl`: RFC-0000, RFC-0002, RFC-0007, RFC-0008, RFC-0010; `stable`: RFC-0001, 0003, 0004, 0006, 0009 | same |
| RFC-0002 | **version 3.8.0, status normative, phase impl**, signature `fb1f84ec…`, current entry dated 2026-10-18, 49 changelog versions | `gov/rfc/RFC-0002/rfc.toml` |
| ADRs | 66 files: 49 `accepted`, 17 `superseded`, **0 `proposed`** | parsed `gov/adr/*.toml` |
| Project schema | version 5; verification default guard `GUARD-GOVCTL-CHECK`; source scan `src/**/*.rs` | `gov/config.toml` |
| Local state | no `.govctl/` directory, no open loops in this checkout | `ls -a` |

**Is the repo "mid-version"?** Yes, in the release sense: 0.22.0 is tagged and two done WIs are not
yet released. Consequences that matter here:

- New normative work is **not frozen**, but the RFC-0002 window is closed: because phase is `impl`
  and a sealed signature exists, `spec`-phase in-place authoring ([[RFC-0000:C-PHASE-LIFECYCLE]])
  is not available; content edits create a detectable unversioned amendment and block further
  progression until bumped.
- A release cut MUST include every done and unreleased Work Item
  ([[RFC-0000:C-RELEASE-DEF]]). So a DSH feature cannot be released "alone": its release also
  carries `WI-2026-10-18-*` (e.g. next minor 0.23.0), or a 0.22.1 patch release goes first.
- The RFC version and the govctl release version are independent: the RFC-0002 bump (3.8.0 →
  3.9.0) and the govctl release bump (0.22.x → 0.23.0) are two separate lifecycle actions.

---

## 2. Binding normative requirements (quote level)

### 2.1 RFC-0002 `C-AGENT-INTEGRATION` — the clause a DSH runtime would change

File: `gov/rfc/RFC-0002/clauses/C-AGENT-INTEGRATION.toml` (kind `normative`, status `active`,
`since = 3.2.0`). Exact obligations:

- "`govctl agent` manages the user-scoped govctl agent integration independently of project
  governance state."
- Syntax is enumerated, and exhaustive-looking:
  `govctl agent doctor|install|update <codex|claude|all>` and
  `govctl agent hook <session-start|pre-tool-use>`.
- "The runtime selector is required for `doctor`, `install`, and `update`; **`all` selects Codex
  and Claude**. `hook` is an internal adapter entry point used by bundled runtime manifests. …
  It **MUST remain hidden from user-facing command discovery**."
- Shared behavior:
  - "These commands **MUST work without an initialized govctl project**."
  - "`doctor` **MUST be read-only**. It MUST verify that every selected runtime exposes the native
    plugin operations required by `install` and `update`, and it **MUST exit non-zero** when any
    selected runtime is unavailable or incompatible."
  - "`install` and `update` **MUST use plugin assets bundled with the running govctl version**. They
    **MUST materialize those assets at a persistent user-scoped support path** before registering
    that path as a native marketplace source."
  - "The materialized plugin version **MUST equal** the running govctl version."
  - "`install` **MUST preserve** an existing Codex reviewer-role file rather than replacing it."
  - "`update` **MUST replace** the govctl reviewer-role files for Codex with the bundled projection."
  - "When `all` is selected, request validation and native-operation preflight **MUST complete for
    both runtimes before the first native mutation**. The command MUST exit non-zero unless every
    selected runtime succeeds, and failure output MUST identify completed, failed, and skipped
    runtime work without claiming cross-runtime atomicity."
  - "With global `--dry-run`, `install` and `update` MUST perform target resolution and readiness
    checks, report planned persistent writes and native operations, and leave persistent state
    unchanged."
  - "Successful `install` and `update` MUST tell the user that a new agent session is required…"
- Client projections:
  - "Claude MUST receive the bundled skills, Claude Markdown reviewer agents, and hooks through its
    native plugin mechanism."
  - "Codex MUST receive the bundled common skills and hooks through its native plugin mechanism."
  - "Codex reviewer agents MUST be installed separately as standalone TOML files… Every generated
    role MUST contain `name`, `description`, and `developer_instructions`, and review-only roles MUST
    select a read-only sandbox mode."
  - "The bundled plugin MUST select a **runtime-specific hook manifest** for each client. Shared
    governance behavior MUST be adapted to the client's supported event fields and output protocol
    rather than relying on one shared hook document."
- Bundled hook behavior (a DSH-native hook adapter would be judged against these semantics):
  - `SessionStart`: same upward project discovery as other commands; silent when unmanaged; concise
    non-blocking recovery when governance state is invalid; bounded active-WI/loop context for a
    valid project, **without** project-wide validation.
  - `PreToolUse`: non-blocking guidance toward canonical commands before direct edits of
    lifecycle-managed artifacts; MUST allow the call, acknowledge direct edit as recovery for
    unsupported operations, recommend `govctl check`.
  - "Bundled hooks **MUST NOT run project-wide validation** on `Stop` or after every file edit. Hook
    advice is a recoverability aid and **MUST NOT be treated as an enforcement boundary**."
- Final carve-out: "The existing `govctl init-skills` command remains the project-local and
  custom-directory projection path. Agent plugin management **MUST NOT change its overwrite or
  destination semantics**."

### 2.2 RFC-0002 `C-GLOBAL-COMMANDS` — the command-surface clause

File: `gov/rfc/RFC-0002/clauses/C-GLOBAL-COMMANDS.toml` (250 lines). Relevant items:

- Item 9 `init-skills`: syntax `govctl init-skills [--force] [--format <claude|codex>] [--dir PATH]`;
  `--format` selects `claude` (`skills/*/SKILL.md` bundles + Markdown agents) or `codex` (same skill
  bundles + TOML agent roles); destination resolution `--dir` > `agent_dir` > format default
  (`.claude` / `.codex`); "Skips files that already exist unless `--force`".
- Item 14 `govctl agent`: "Manages the user-scoped govctl agent integration. … Behavior, including
  the internal adapter exception to runtime selection, is defined by
  [[RFC-0002:C-AGENT-INTEGRATION]]."
- "Future Additions: New global commands **MAY be added via RFC amendment**. They MUST meet at least
  one criterion: 1) operate on multiple resource types; 2) perform project-level initialization or
  cleanup; 3) provide meta-information about the CLI itself; 4) manage local execution state that
  coordinates governed resources without becoming a governed resource itself." — see also
  ADR-0018/ADR-0035, which both treat this clause as requiring an RFC amendment for global-command
  changes.

### 2.3 RFC-0002 `C-COMPATIBILITY-BOUNDARY` — canonical-only CLI

File: `gov/rfc/RFC-0002/clauses/C-COMPATIBILITY-BOUNDARY.toml` (kind `normative`, `since 1.0.0`):
"The CLI **MUST accept only** command names, subcommand names, field paths, and mutation flags
defined by the current normative RFC clauses." and "The CLI **MUST reject compatibility aliases**
for command names and subcommand names."

Reading: an added runtime selector (`dsh`) / `--format dsh` value is normative syntax enumerated by
C-AGENT-INTEGRATION and C-GLOBAL-COMMANDS; shipping it without amending those clauses makes the
normative RFC false for the running binary (= Law-1 bug) and fails the compatibility boundary's
spirit. Conservative and defensible position: **RFC amendment required** for any new CLI value that
appears in those clauses.

### 2.4 Other RFC-0002 clauses that would be touched or must be respected

- `C-DESCRIBE-COMMAND`: `govctl describe` is parser-owned; adding a nested command/help change flows
  into the catalog automatically. Schema v1 requires sorted command nodes and no new required
  fields; *additive optional* fields may keep `schema_version` 1, removing/renaming/requiring needs a
  major govctl bump. A new runtime selector alone should not require a describe schema change, but
  the implementer must not edit the catalog by hand.
- `C-LIFECYCLE-VERBS` #2/#3/#4: `rfc advance`/`rfc bump` semantics — see §2.6.
- `C-PRE-1-RELEASE-TARGET-COMPATIBILITY` / `C-SELF-UPDATE` / ADR-0060: release asset naming and
  self-update are untouched by a DSH integration; do not accidentally change them.
- `C-COMPATIBILITY-BOUNDARY` also fixes project schema version 5 and the storage layout; nothing
  here changes.

### 2.5 What the RFC does **not** cover (the external-package scope test)

- C-AGENT-INTEGRATION governs **`govctl agent` and bundled assets**: "materialize those assets at a
  persistent user-scoped support path before registering that path as a native marketplace source",
  "bundled plugin", "bundled hook behavior". An integration shipped outside the governed binary is
  not literally bound by those MUSTs.
- Nothing in the RFCs forbids a third-party/companion distribution channel. ADR-0033 already
  authorized a *packaging* decision without RFC content change (Claude plugin, before the
  `govctl agent` CLI existed); ADR-0060 authorized a build/distribution mechanism decision.
- However, C-AGENT-INTEGRATION's user-facing promise ("`govctl agent` manages the user-scoped govctl
  agent integration") and ADR-0061's "one `govctl agent` command group as the user-facing facade"
  become inaccurate if govctl docs present a second official install path without a recorded
  ADR decision. Governance-wise the gap is an ADR, not an RFC amendment.

### 2.6 Lifecycle machinery an implementation must use (exact rules)

- `AGENTS.md` §2 **Law 1 (RFC Supremacy)**: "RFCs are constitutional law. Code that conflicts with a
  normative RFC is a bug." / "No silent deviation: fix the code or propose an RFC amendment." /
  "Normative RFC content MAY continue changing without another bump **while its current version
  remains in `spec`**" / "Content changed after entry to `impl` is an **unversioned amendment** and
  **MUST be released by a version bump** before further phase progression."
- `AGENTS.md` §2 **Law 2 (Phase Discipline)**: `spec → impl → test → stable`; "Skipping is
  forbidden"; only `impl` permits code writing per normative RFC; `stable` = bug fixes only.
- `AGENTS.md` §2 **Law 3 (No Silent Deviation)**: "If behavior is unspecified or ambiguous, escalate.
  Do not invent."
- `AGENTS.md` §4 Decision Tree: "New behavior or design choice → Draft RFC first"; "Execution MUST
  NOT begin on new features until RFC is normative."
- RFC-0000 `C-PHASE-LIFECYCLE`: "A version-changing bump MUST be rejected while the current version
  remains in `spec`"; "A later version-changing bump MAY start another phase lifecycle only when the
  current version is in `impl`, `test`, or `stable`"; "A version bump that releases a detected content
  amendment MUST start the new version at `spec`"; "An unversioned amendment MUST be released by a
  version bump before further phase progression"; "A normative RFC in `impl`, `test`, or `stable`
  MUST have a stored amendment signature."
- RFC-0002 `C-LIFECYCLE-VERBS` #4 (`rfc bump`): "The version-changing form MUST require normative RFC
  status"; "MUST enforce the source-phase and stored-signature eligibility"; "Version bumping MUST
  follow semantic versioning"; "A version bump MUST update the changelog automatically"; "A version
  bump MUST be rejected when the RFC has a stored amendment signature and no amendment content has
  changed since that signature"; "A bump that releases detected RFC or clause content MUST set phase
  to `spec`"; "A bump MUST assign its new version to pending clauses."
- RFC-0000 `C-CLAUSE-DEF`: "A new clause in a normative RFC whose phase is `impl`, `test`, or
  `stable` MUST be pending"; "A version bump of a normative RFC MUST assign the new RFC version to
  every pending clause"; "Advancing a normative RFC from `spec` to `impl` MUST reject every pending
  clause." (Order therefore: content edit(s) → bump → advance.)
- RFC-0010 `C-ARTIFACT-CLAIM`: "Operations that change an RFC's version semantics — version bump,
  finalization, phase advancement, deprecation, and supersession — **MUST acquire an exclusive
  artifact claim** before mutating, on every RFC whose version-semantics fields the operation
  mutates. … When another workspace holds a live claim … the command MUST fail without mutating."
  Content edits/clause authoring are only claim-warned, not blocked. Relevant because DSH Agent
  Teams can run parallel sessions on the same clone.
- RFC-0000 `C-REFERENCE-HIERARCHY`: "RFC `refs` entries, `[[...]]` link targets, and known
  artifact-ID mentions in governed RFC clause text **MUST NOT identify an ADR or a Work Item**";
  likewise ADR content may not name Work Items. Consequence: the RFC-0002 amendment text must not
  cite ADR-0061/ADR-0067; the ADR cites clauses instead (normal practice: ADR-0061 refs
  `RFC-0002:C-AGENT-INTEGRATION`).
- RFC-0000 `C-WORK-DEF`: "A Work Item MUST contain at least one acceptance criterion before it
  transitions to done"; acceptance criteria are categorized (categories drive CHANGELOG output).
- RFC-0000 `C-RELEASE-DEF`: release entries newest-first; `refs` only done WIs; a WI appears in at
  most one release; "A release cut MUST include every Work Item whose status is `done` and which is
  not referenced by an existing release."
- `gov` skill Hard Stops (`.claude/skills/gov/SKILL.md:26`): "Ask the user before any lifecycle or
  destructive artifact operation (accept, reject, finalize, advance, bump, deprecate, supersede,
  delete) unless the request already authorizes it"; "Substantive implementation needs a matching
  active Work Item."
- Repo gates (would fail if artifacts/code/docs drift): `.pre-commit-config.yaml` runs
  `cargo fmt`, `cargo check`, `cargo clippy -D warnings`, `cargo test`, `render rfc`,
  `render changelog`, `govctl check`; `.github/workflows/ci.yml` runs `govctl check` on the release
  binary; `.github/workflows/release.yml` verify job enforces tag == `Cargo.toml` version ==
  `CHANGELOG.md` section == `gov/releases.toml` entry, and tag reachable from `origin/main`.

---

## 3. ADR-0061 and the recorded decision boundary

### 3.1 ADR-0061 (live decision, `accepted`)

File: `gov/adr/ADR-0061-use-one-agent-integration-command-with-client-specific-projections.toml`
(status `accepted`, 2026-07-31, refs `RFC-0002:C-AGENT-INTEGRATION`, ADR-0033, ADR-0035).

- Context: govctl had "two disconnected installation paths" (Claude native plugin vs Codex
  `init-skills --format codex`); "Codex native plugins carry skills and hooks but do not define
  standalone custom-agent roles"; manual native CLI instructions "duplicate marketplace, preflight,
  partial-failure, and update handling that the **agent-plugin-installer** crate already centralizes."
- Decision: "Use one `govctl agent` command group as the user-facing facade for doctor, install, and
  update operations. **Materialize the plugin assets bundled with the running binary** at a stable
  user-scoped support path and **delegate native marketplace/plugin orchestration to
  agent-plugin-installer**." Per-client projections; retain `init-skills` for project-local copies;
  per-runtime hook manifests routed to a shared govctl adapter; hooks advisory only, no Stop
  validation.
- Consequences: positive "users get one discoverable workflow, **plugin content stays aligned with
  the running govctl version**…"; negative "binary gains a small orchestration dependency plus
  persistent support files"; neutral "Claude and Codex remain different internally".
- **Did it anticipate more runtimes?** No. The context/decision/consequences only ever say "Codex"
  and "Claude" / "both clients"; there is no future-runtime extension clause. Extending to DSH is a
  new design decision that must be recorded (new ADR), and it must address ADR-0061's "one facade"
  intent if the DSH path is a separate installer.
- **`agent-plugin-installer` boundary:** declared at `Cargo.toml:46`
  (`agent-plugin-installer = { version = "0.6.1", features = ["clap"] }`); the crate describes itself
  as "Native CLI orchestration helpers for managing **Codex and Claude** agent plugins"
  (crates.io metadata; repo `lucifer1004/agent-plugin-installer` — not govctl-org). A native DSH
  runtime therefore needs either upstream runtime support or govctl-owned orchestration, and an ADR
  must record that choice. Also concrete code coupling in `src/cmd/agent.rs`:
  `validate_bundled_versions()` hard-codes "Expected two bundled plugin manifests"; the
  materialized support path is `ProjectDirs::from("org","govctl","govctl")/agent-plugin/versions/<ver>/.claude`;
  `build.rs` embeds `.claude-plugin`, `.claude/.claude-plugin`, `.claude/.codex-plugin`,
  `.claude/skills`, `.claude/agents`, `.claude/hooks`, and `Cargo.toml`'s `include` list must ship
  any new manifest/template.

### 3.2 Adjacent/prior ADRs that constrain the choice

| ADR | Status | Constraint it contributes |
| --- | --- | --- |
| **ADR-0033** (distribute agent integration as Claude Code plugin) | superseded by ADR-0061 | Precedent that a *packaging/distribution* choice can be an ADR-only decision (no RFC content change) when no CLI behavior changes. Its rejected alternatives are reusable evidence: **separate repo** rejected because "Skills and agents drift from main codebase"; thin plugin without hooks rejected ("hooks are the high-leverage differentiator"); MCP rejected (two interfaces to maintain, per ADR-0015). |
| **ADR-0035** (decouple skills/agents from init) | accepted | Precedent for adding a global command (`init-skills`) via *RFC amendment*: its context quotes C-GLOBAL-COMMANDS: "new global commands … added via RFC amendment". Also fixed `init-skills` overwrite/destination semantics, which C-AGENT-INTEGRATION now protects. |
| **ADR-0018** (strict resource-first syntax) | accepted | Precedent: "any such addition [convenience/global command] would require an RFC amendment to RFC-0002:C-GLOBAL-COMMANDS." |
| **ADR-0015** (self-describing CLI) → **ADR-0058** | superseded / accepted | CLI-only discovery, no MCP; parser-owned introspection. `describe` schema v1 is a compatibility surface; additive changes are cheap, removals are major. Nothing forces a describe schema bump for a new selector value. |
| **ADR-0057** (policy-oriented agent guidance) | accepted | Information-ownership model: RFCs own obligations; project instructions own authority boundaries; skills own strategy; CLI/status/describe own current syntax; guides own explanation. **Skills are SSOT** and must not be duplicated with divergent copies — directly relevant to how a DSH package sources skills. |
| **ADR-0028** / **ADR-0024** (skills cross-platform; writers-as-skills/reviewers-as-agents) | accepted / superseded by ADR-0057 | The portable unit is a `SKILL.md` bundle; reviewer roles are agents (cognitive isolation). DSH must map (or explicitly drop) reviewer-agent behavior in its own agent model. ADR-0057 keeps the split. |
| **ADR-0060** (Zig release targets) | accepted | Release asset/target policy is unrelated to a DSH package, but release-version stamping is automated: `Justfile` `_bump-plugin` stamps `.claude-plugin/marketplace.json`, `.claude/.claude-plugin/plugin.json`, `.claude/.codex-plugin/plugin.json`; a new DSH manifest would have to be added there and in `build.rs`/`Cargo.toml`. |
| **ADR-0063** (relax ADR alternatives thresholds) | accepted | `adr accept` requires ≥1 accepted alternative; a rejected alternative is required only when ≥2 alternatives exist. New ADR must still be honest about options (no fabricated straw men). |

---

## 4. Prior art: how Claude/Codex integration was actually introduced

Chronology reconstructed from `gov/work/*.toml`, `gov/releases.toml`, `CHANGELOG.md`, and git history.

| When | Artifact / release | What it established |
| --- | --- | --- |
| 2026-02-11 | ADR-0024; WIs 2026-02-11-002/003/004/005 → **0.4.1** | `.claude/` is SSOT for assets; writers as skills, reviewers as agents; sync command pushes to downstream |
| 2026-02-22 | ADR-0028; WI-2026-02-22-003 → **0.5.1** | Commands → skills format for cross-platform portability (Claude/Cursor/Codex) |
| 2026-03-04 | **ADR-0033**; `WI-2026-03-04-001` "Package govctl as Claude Code plugin with hooks" → **0.5.4** | ADR-only authorization for plugin packaging; marketplace.json; hooks added. **No RFC content change** (pre-`govctl agent` CLI) |
| 2026-03-17 | **ADR-0035**; `WI-2026-03-17-010` → **0.7.0** | `init-skills` replaces `sync`; C-GLOBAL-COMMANDS amended (new global command = RFC amendment) |
| 2026-04-02/04/08 | WIs 2026-04-02-001 (0.7.5), 2026-04-04-002 (0.7.7), 2026-04-08-002/003/004 (0.8.1/0.8.2) | Cross-format agent output (`--format`, `--dir`), plugin manifest metadata bundling, Codex global-install docs, skills/agents authority-boundary cleanup |
| 2026-07-26 | `WI-2026-07-26-014` "Make describe a reliable agent entry point" → **0.15.0** | Agent discovery via parser-owned `describe` (ADR-0058) |
| 2026-07-31 | **ADR-0061**; `WI-2026-07-31-001` → **0.18.0** | `govctl agent doctor/install/update`; four RFC-0002 amendment versions created in one day: feature commit `a36253a` bumped **3.1.0 → 3.4.0**, and review-fix commit `43a87a5` bumped **3.4.0 → 3.5.0** (so review findings were sealed as a new normative version before release); bundled assets embedded via `build.rs`; per-client hook manifests + adapter; docs + tests in same change |
| 2026-09-08 | **RFC-0010** + **8 WIs** → **0.20.0** | New-RFC path precedent: spec commit (`9d8653b`: RFC + 8 clause files + ADR) then impl commit (`5577748`: code + all WIs + changelog) same day; follow-up audits WIs; minor release |
| 2026-10-18 | `WI-2026-10-18-001/002` (unreleased) | RFC-0002 3.8.0 amendment/bump + code; **released later** — demonstrates the current mid-version state |

**Reusable pattern (what an eventual DSH change should imitate):**

1. Authorize first: new RFC or RFC amendment for behavior; ADR for the design choice (RFCs never
   cite ADRs; ADRs cite RFC clauses).
2. One Work Item per durable outcome (existing example: one WI with 16 acceptance criteria covered
   the entire `govctl agent` feature), categorized acceptance criteria → CHANGELOG categories.
3. Implement + focused tests + render + docs in the same change; run `just pre-commit`
   (`fmt`, `clippy`, `test`, `render rfc`, `render changelog`, `govctl check`).
4. Close WIs through `govctl work move <WI> done` (runs guards; `GUARD-CARGO-TEST` is the usual
   WI-level guard; `GUARD-GOVCTL-CHECK` is the project default).
5. Release: `just bump X.Y.Z` (Cargo + plugin manifests + `govctl release` + changelog render),
   then tag `vX.Y.Z`; CI verifies version/CHANGELOG/releases consistency and main reachability.
6. Update the rendered projection (`docs/rfc/RFC-0002.md`) only through `govctl render`; treat it as
   generated, never hand-edit.

---

## 5. Concrete governance paths

Assumption: the user has not yet authorized any lifecycle operation; only research was requested.
Every path below assumes explicit user approval before the first lifecycle verb (per `gov` skill
Hard Stops).

### Path A — native DSH runtime inside govctl (first-class `govctl agent install dsh`)

Use when: govctl should own DSH install/update/doctor and keep the "one facade, version-aligned
assets" promise. Cost: RFC-0002 amendment + ADR + 2–4 WIs + minor release.

1. **Respect the RFC-0002 claim** (RFC-0010 C-ARTIFACT-CLAIM): the `rfc bump`/`advance` command
   acquires the exclusive claim itself and fails if another workspace holds a live claim; verify
   with `govctl claim list` before starting so parallel DSH/Agent-Teams workspaces do not collide.
2. **Amend RFC-0002** (normative, `impl`, 3.8.0 → 3.9.0):
   - Content edit first: `C-AGENT-INTEGRATION` (syntax selectors, `all` semantics — recommend
     keeping `all` = Codex+Claude for backward compatibility and adding explicit `dsh`, shared
     behavior "both runtimes" wording, client projections, hook behavior) and
     `C-GLOBAL-COMMANDS` (item 14 syntax; item 9 if `init-skills --format dsh` is added). Optionally
     a focused new clause (e.g. `C-DSH-PROJECTION`) created while `impl` (pending `since`).
   - `govctl rfc bump RFC-0002 minor -m "Add DeepSeek Harness runtime"` → sets phase `spec`
     (bump is rejected if no content changed, so edit first — C-LIFECYCLE-VERBS #4).
   - Review with `rfc-reviewer`; refine while still `spec`; `govctl rfc advance RFC-0002 impl` to
     seal (`spec→impl` rejects pending clauses; bump already assigned their `since`).
3. **New ADR** (proposed → reviewed by `adr-reviewer` → accepted only with user approval) recording:
   projection model for DSH; how skills/reviewer roles/hooks map; asset version alignment;
   `all` backward-compatibility decision; how the `agent-plugin-installer` gap is closed (upstream
   runtime support vs govctl-side orchestration); alternatives (external package, vendor assets).
   It must not cite Work Items; it may cite `RFC-0002:C-AGENT-INTEGRATION` (cf. ADR-0061).
4. **Work Items** (active → done), e.g.:
   - "Add DeepSeek Harness runtime to govctl agent" (assets, doctor/install/update, dry-run,
     version stamping, partial-failure reporting, hidden hook adapter, tests);
   - "Project and embed DSH plugin assets" (`build.rs` asset roots, `Cargo.toml` include, manifest
     count check in `src/cmd/agent.rs`, `Justfile _bump-plugin`, completions);
   - "Document DSH agent integration" (README, `docs/guide/validation.md`, AGENTS.md §5/§6,
     `init`/`detach` skills if behavior changes; chore category so release notes stay clean).
5. **Implement + tests** (`tests/test_agent_plugin.rs`, hook/path tests, describe/completions
   tests); no direct `gov/` edits; `just pre-commit`; `govctl check`.
6. **Close WIs**, **release minor 0.23.0** (includes the currently-unreleased 2026-10-18 WIs), tag
   `v0.23.0`; publish notes via CI release workflow.
7. Docs truth pass: README "Agent workflow"/"Agent Integration", `docs/guide/validation.md`,
   AGENTS.md CLI reference and skills tables, `docs/rfc/RFC-0002.md` (regenerated), CHANGELOG.

Design constraints to settle in the ADR (all consequences of current clauses/code):
`all` semantics back-compat; version equality requirement; `install` preserve vs `update` replace
per projection artifact; non-blocking/advisory hook semantics; hidden adapter in completions;
`init-skills` overwrite/destination semantics unchanged; two-manifest assumption in
`validate_bundled_versions`; third-party crate boundary.

### Path B — external DSH plugin package (recommended minimal path for "make it usable as a DSH plugin")

Use when: the deliverable is a DSH-native package (npm plugin bundle installed with
`dsh plugin --profile <name> add <pkg>`) that wraps the installed `govctl` binary and reuses the
bundled skill/agent content, *without* adding govctl CLI surface. A DSH plugin is an npm package
providing a plugin bundle; DSH skills are `SKILL.md` directories — the same format as
`.claude/skills/*`, so asset reuse is plausible (compatibility and load path are DSH-side open
questions, not govctl law).

Governance requirement, in order:
1. **ADR** (proposed → accepted): decide the distribution channel and record why it does not need an
   RFC amendment. Scope test to state explicitly: adds no govctl CLI grammar; changes no existing
   command behavior; does not alter `govctl agent` semantics. It must address:
   - ADR-0061 "one facade" tension (second official install path);
   - asset SSOT/drift (ADR-0033's rejected separate-repo rationale) — how skills/hooks stay in sync
     with `.claude/` SSOT and how the package version aligns with the govctl binary version
     (ADR-0061 rationale: "plugin content stays aligned with the running govctl version");
   - hook-equivalence policy if the package injects context/advises (mirror the advisory,
     non-blocking, no-repeated-validation semantics rather than inventing enforcement);
   - **not** changing `init-skills` overwrite/destination semantics (C-AGENT-INTEGRATION final line).
2. **Work Items** (active → done) for the package, its tests (asset/version consistency checks), and
   docs; category `added`/`chore` per CHANGELOG convention.
3. **Docs** (see §7) + **CHANGELOG**; if published from this repo, extend release tooling (npm
   publish step, version stamping in `Justfile`, release workflow) — that is itself a WIs/CI change.
4. **No RFC-0002 amendment** is needed as long as the scope test holds; if the package later grows a
   `govctl` command (e.g. `govctl agent install dsh`) or changes existing command behavior, it moves
   to Path A.

Asset-sourcing options for this path (pick in the ADR; each has a governance consequence):
- (a) **Vendor at package build/publish time from `.claude/` SSOT** with an automated version stamp —
  needs a sync check in CI to avoid drift (ADR-0033's drift concern), and version parity with the
  govctl release; cleanest ownership story.
- (b) **Wrap the installed binary only** (skills shipped by the package itself or loaded from a
  user-configured path): duplication risk; ADR must state how stale copies are avoided.
- (c) **Read `govctl`'s materialized support path** (`…/agent-plugin/versions/<version>/.claude`
  from `resolve_locations()`): works only after a `govctl agent install claude|codex`, relies on an
  implementation detail not specified by the RFC; not recommended.
- (d) **Project-local `govctl init-skills --format claude --dir …`**: existing RFC-defined command,
  but it requires a loaded governed project (`Config`), resolves relative `--dir` against the project
  root, and is defined as the project-local/custom-directory path — usable for per-project DSH
  setups, not for a global user-scoped plugin.

### Path C — documentation-only compatibility note (lowest cost, only if DSH loads the existing skills)

If DSH can consume `SKILL.md` bundles directly, a guide/README quickstart showing how to point DSH at
the existing projected skills (e.g. `govctl init-skills --format claude --dir …` inside a governed
project) is a docs-only WI. No RFC change; an ADR is only needed if this becomes a supported
distribution claim rather than a tip. Verify DSH's skill discovery first; do not claim support that
was not tested (ADR-0057 policy ownership; no silent invention).

### Authorization checklist (what the user must approve before implementation)

- [ ] Scope decision: Path A (native runtime) vs Path B (external package) vs C (docs tip).
- [ ] Creation/finalization/amendment of a normative RFC (Path A) or ADR acceptance (A/B).
- [ ] RFC-0002 version bump (`rfc bump` 3.8.0 → 3.9.0) and `rfc advance … impl` (Path A).
- [ ] New Work Items, their guards, and their closure (active → done).
- [ ] Release cut / version bump (`just bump 0.23.0`, `govctl release`), tag `v0.23.0`.
- [ ] Any `.claude/` asset changes (publication affects every bundled integrator and the embedded
      binary assets).

### What can be done now without triggering governance

- Read-only research and spikes (this document; DSH-side prototypes in `/tmp` or a separate repo).
- Drafting proposed RFC/ADR *text* for review (discuss skill: "Do not write code or create Work
  Items"; lifecycle verbs remain user-authorized).
- Do **not** edit the sealed `gov/rfc/RFC-0002` content in place without the bump path: that creates
  an unversioned amendment that blocks later progression, and may require restoring the sealed
  baseline from VCS.

---

## 6. Risks of skipping governance

1. **Law-1 nonconformance:** a `dsh` selector/`--format dsh` not defined in a normative clause makes
   the CLI conflict with RFC-0002; the project's own rule is "code that conflicts with a normative
   RFC is a bug." A later `compliance-checker` audit would surface it (precedent: RFC-0010 evidence
   fixes; release 0.20.2 fixed guidance drift).
2. **Phase/lifecycle corruption:** editing the sealed RFC-0002 in place without a bump produces a
   detectable unversioned amendment; `rfc bump`/`advance` will then behave as "releasing an
   amendment" rather than "adding a feature", or be rejected (missing/unequal signatures), leaving
   the repo unable to progress without VCS restoration.
3. **Broken release pipeline:** next release REQUIRES all done WIs referenced (C-RELEASE-DEF); the
   release workflow verifies `Cargo.toml` == tag == `CHANGELOG.md` section == `gov/releases.toml`
   entry. An ungoverned feature has no WIs/CHANGELOG category, so it cannot be released by
   `just bump` without retrofitting artifacts (spec-after-the-fact, phase-order violation).
4. **CI/pre-commit failures:** `govctl check`, render freshness (`render rfc`/`render changelog`),
   cargo fmt/clippy/test all gate the change; `govctl check` validates cross-references
   (dangling `refs`, ADR/WI hierarchy, pending clauses).
5. **Concrete breakage in the native path:** adding a third plugin manifest to the embedded assets
   without updating `validate_bundled_versions()` ("Expected two bundled plugin manifests") makes
   `govctl agent install/update` fail; missing `build.rs` rerun/include entries silently ship stale
   or absent assets.
6. **Documentation becomes false:** README/AGENTS/guide currently state the integration surface is
   `govctl agent` for Claude/Codex only; ungoverned additions create docs that no artifact or test
   owns (ADR-0057 ownership model), and generated projections can drift.
7. **Security/support expectations:** the user-scoped integration writes persistent files and native
   client config; without the RFC's preserve/replace/preflight/partial-failure/dry-run obligations,
   an installer can silently overwrite user config (ADR-0061 consequences) or claim atomicity it
   does not have.
8. **Ecosystem split:** a second unofficial install path recreates exactly the "two disconnected
   installation paths" problem ADR-0061 was accepted to remove; without an ADR, future maintainers
   cannot tell whether it is supported.
9. **Pre-1.0 compatibility:** govctl publishes pre-1.0 target aliases and canonical-only CLI; any
   undocumented alias/flag added for DSH convenience conflicts with C-COMPATIBILITY-BOUNDARY.

---

## 7. Documentation and asset surfaces that would need updating

Hand-authored docs (must be changed by a Work Item; not generated):

| Path | What it says today | Needed for a DSH integration |
| --- | --- | --- |
| `README.md` (Agent workflow ~L139–162; Install/Quick Start) | `govctl agent doctor/install <claude\|codex>`; plugin session refresh | DSH install/usable section, support matrix, hook/skills behavior, version alignment |
| `docs/guide/validation.md` §"Agent Integration" (L345–~395) | `govctl agent doctor/install/update` for Claude/Codex; `init-skills` formats claude/codex; hook behavior | DSH instructions or explicit "unsupported/native package" note |
| `docs/guide/getting-started.md` | no runtime-specific content today | add first-run DSH steps if the package is a supported path |
| `AGENTS.md` §5 CLI Reference (L~175 agent commands) and §6 agent/skills tables | `<codex\|claude\|all>`; skills/agents ownership | DSH command/selector, new assets, hook semantics |
| `.claude/skills/init/SKILL.md` (L17–18, 43) and maybe `.claude/skills/detach/SKILL.md` | runtime integration is `agent doctor\|install\|update`; detach lists plugin paths | DSH-aware install/removal guidance if it is part of the agent layer |
| `.claude-plugin/marketplace.json` version + `.claude/.claude-plugin/plugin.json`, `.claude/.codex-plugin/plugin.json` | version 0.22.0 stamped by `Justfile _bump-plugin` | add/stamp a DSH manifest if DSH uses one (Path A); keep version parity (Path B if in-repo) |
| `CHANGELOG.md` | `[Unreleased]` + released sections | new `Added`/`Changed` entries from WI categories at release |
| `gov/releases.toml` | newest 0.22.0 | new release entry containing the DSH WIs |

Generated projections (never hand-edit; regenerate with `govctl render`): `docs/rfc/RFC-0002.md`
(any RFC-0002 amendment), `docs/adr/ADR-006x.md` (new ADR), `docs/work/*.md` (new WIs),
`CHANGELOG.md` (via `render changelog`). `docs/SUMMARY.md` is generated by `scripts/build-book.sh`;
a new guide page must be added to that script.

Code/release surfaces (Path A; also Path B if shipped from this repo): `src/cmd/agent.rs`,
`src/cmd/agent_hooks.rs` (if hooks), `src/cli/commands.rs`, `src/cli/help.rs`,
`src/command_router/*`, `build.rs` + `build_support/agent_templates.rs`, `Cargo.toml` (`include`,
possibly dependency), `Justfile` (`_bump-plugin`), `.github/workflows/release.yml` (only if a new
published artifact type is added), `tests/test_agent_plugin.rs` and related snapshot/path tests.

---

## 8. Open questions for the user

1. **Scope/ownership:** is the DSH integration an official govctl deliverable (in-repo or
   govctl-org-published) or a third-party package? That decides whether govctl governance artifacts
   are required at all, and whether Path B needs an ADR here.
2. **Native vs external:** should DSH be a first-class runtime in `govctl agent` (Path A: RFC-0002
   amendment + release) or an external DSH package wrapping the binary (Path B: ADR-only, no RFC
   amendment)? If both, which is phase 1?
3. **DSH capabilities** (determines which RFC obligations are even applicable): does DSH provide
   a native plugin/marketplace install API, session-start/pre-tool-use hook events, and
   agent/role definitions? Does it load arbitrary `SKILL.md` bundles from a user directory?
   (A DSH plugin is installed as an npm plugin bundle via `dsh plugin --profile <name> add <pkg>`;
   DSH ships `SKILL.md` skill bundles — compatibility needs verification, not assumption.)
4. **`all` semantics:** may `all` gain DSH, or must it stay Codex+Claude for backward compatibility
   (recommended) with DSH selected explicitly? This is a normative clause change either way.
5. **Installer boundary:** should govctl extend/fork/bypass `agent-plugin-installer` (third-party,
   Codex/Claude-only), or should DSH be orchestrated natively by govctl? Who accepts that
   dependency/ownership cost?
6. **Asset SSOT and version alignment:** for Path B, where does the package's skill/hook content come
   from, and what keeps it in sync with `.claude/` and the installed govctl version? (ADR-0033
   rejected a separate repo precisely for drift; ADR-0061 requires version alignment for bundled
   assets.)
7. **Hooks expected?** Skills-only integration is much cheaper; context injection/edit-guidance
   hooks require DSH event equivalents and a recorded advisory-not-enforcement policy decision.
8. **Release approval:** is a govctl minor release (0.23.0, also carrying the two unreleased
   2026-10-18 WIs) acceptable, and who authorizes bump/advance/accept/release/tag lifecycle verbs?
9. **Compatibility promise:** confirm the DSH integration must preserve canonical-only CLI
   (C-COMPATIBILITY-BOUNDARY) and no alias/convenience flags.

---

## Appendix: key file references

- `AGENTS.md` (laws §2; decision tree §4; CLI reference §5; skills/agents §6)
- `gov/rfc/RFC-0002/rfc.toml`; `gov/rfc/RFC-0002/clauses/C-AGENT-INTEGRATION.toml`,
  `C-GLOBAL-COMMANDS.toml`, `C-LIFECYCLE-VERBS.toml`, `C-COMPATIBILITY-BOUNDARY.toml`,
  `C-DESCRIBE-COMMAND.toml`, `C-SELF-UPDATE.toml`
- `gov/rfc/RFC-0000/clauses/C-PHASE-LIFECYCLE.toml`, `C-STATUS-LIFECYCLE.toml`,
  `C-CLAUSE-DEF.toml`, `C-REFERENCE-HIERARCHY.toml`, `C-WORK-DEF.toml`, `C-RELEASE-DEF.toml`
- `gov/rfc/RFC-0010/clauses/C-ARTIFACT-CLAIM.toml`
- `gov/adr/ADR-0061-*.toml`, `ADR-0033-*.toml`, `ADR-0035-*.toml`, `ADR-0018-*.toml`,
  `ADR-0015-*.toml`, `ADR-0058-*.toml`, `ADR-0057-*.toml`, `ADR-0028-*.toml`, `ADR-0024-*.toml`,
  `ADR-0060-*.toml`, `ADR-0063-*.toml`
- `gov/work/2026-07-31-harden-agent-plugin-installation-across-codex-and-claude.toml`,
  `2026-03-04-package-govctl-as-claude-code-plugin-with-hooks.toml`,
  `2026-04-04-clarify-governance-artifact-boundaries-and-bundle-plugin-manifest-metadata.toml`,
  `2026-07-26-make-describe-a-reliable-agent-entry-point.toml`,
  `2026-10-18-*.toml`
- `src/cmd/agent.rs`, `src/cmd/agent_hooks.rs`, `build.rs`, `build_support/agent_templates.rs`,
  `Cargo.toml`, `Justfile`, `.pre-commit-config.yaml`, `.github/workflows/{ci,release}.yml`
- `docs/guide/validation.md`, `README.md`, `docs/rfc/RFC-0002.md`
