# ADR proposal — ship the DeepSeek Harness integration as an external DSH plugin bundle

Status: **ingested as governed artifact `ADR-0067` (proposed)** on 2026-10-31.
The authoritative record now lives in
`gov/adr/ADR-0067-ship-the-deepseek-harness-integration-as-an-external-dsh-plugin-bundle.toml`
and is read with `govctl adr show ADR-0067`. This markdown file remains the
working draft/design note; it is **not** authority once the two diverge. Acceptance
is a separate lifecycle decision that requires explicit maintainer approval.

Date: 2026-10-31
Scope owner: govctl maintainers
Related: `RFC-0002:C-AGENT-INTEGRATION`, `RFC-0002:C-COMPATIBILITY-BOUNDARY`,
`RFC-0002:C-GLOBAL-COMMANDS`, `ADR-0061`, `ADR-0033`.

## Context

DeepSeek Harness (DSH) is a Cordis-based agent harness whose GUI and CLI are
distributed as an Electron app; its extensibility model is an npm **bundle** whose
`package.json` declares `dsh.bundle.patch`, a patch file inserting Cordis plugin
rows into a per-profile composition. Govctl already ships a Claude Code plugin
(skills, reviewer agents, hooks) and a Codex projection, installed by
`govctl agent install/update <codex|claude|all>`.

Research (2026-10-31, `research/` on the research branch) established:

- DSH 0.2.1-rc.5 supports third-party bundles installed with
  `dsh plugin --profile <name> add <package>` (or the GUI Plugins page), with
  `engines.dsh` compatibility enforcement.
- Root-level plugin rows are inherited by every agent preset, so tools, prompt
  sections, skills and commands are visible without patching shipped presets.
- DSH accepts `SKILL.md` bundles with `name` + `description`; Claude-only
  frontmatter keys are ignored, and `$ARGUMENTS` is not substituted.
- DSH removed the Claude-Code hook bridge in the 0.2 line: `agent/session-start`
  and `dsh-hooks-*` do not exist. Bundle hooks must be reimplemented as native
  Cordis interception points (`agent/created`, `agent/pre-step`,
  `tools/pre-execute`, `tools/post-execute`).
- The GUI runs the app-owned `desktop` profile; plugin operations are performed
  through the GUI or `dsh plugin --profile desktop …` with the app fully quit.

`RFC-0002:C-AGENT-INTEGRATION` currently enumerates `<codex|claude|all>` and
defines the bundled-asset install/update contract. `ADR-0061` records the
"one `govctl agent` command group as the user-facing facade" intent and requires
bundled assets to stay version-aligned with the running govctl. `ADR-0033`
rejected a separate distribution repository over asset drift.

## Decision

Ship the DSH integration as an **external DSH bundle in this repository**
(`dsh-plugin/`, npm package `govctl-dsh`):

1. It wraps an installed `govctl` binary; governance semantics stay in the Rust
   binary (no reimplementation of lifecycle, locking, claims, or signatures).
2. It reuses the `.claude/` skill and reviewer-agent content through a
   deterministic generator (`dsh-plugin/scripts/sync-assets.mjs`) with a
   `--check` mode and a CI drift gate; `.claude/` remains the single source of
   truth and every generated asset records the govctl version it came from.
3. It adds **no govctl CLI surface and changes no existing command behavior**;
   `govctl agent` and `init-skills` semantics are untouched.
4. The plugin is distributed independently of govctl releases, declares
   `engines.dsh`, and documents a supported govctl version range, so a DSH
   release does not force a govctl release.

## Scope test (why no RFC amendment is required)

- No new command names, subcommands, flags or field paths are added to govctl.
- No existing command behavior changes.
- `govctl agent` runtime selectors and native installer behavior are unchanged.
- `init-skills` overwrite/destination semantics are unchanged.
- Bundled-asset version equality obligations in `C-AGENT-INTEGRATION` apply to
  `govctl agent`'s own materialization; this package does not run that flow.

If any future change violates one of these points (for example adding
`govctl agent install dsh` or a `--format dsh`), this ADR is superseded and
`RFC-0002` must be amended first.

## Alternatives considered

1. **Native DSH runtime in govctl** (`govctl agent install dsh`). Most cohesive,
   but requires an RFC-0002 amendment (3.8.0 → 3.9.0, `spec` → `impl`), an ADR, a
   minor release, and either upstream support in `agent-plugin-installer` (which
   is closed over Codex/Claude) or a new in-repo orchestration layer. Deferred;
   revisit if DSH support must be first-class.
2. **Documentation-only compatibility note** (`govctl init-skills --dir .dsh`
   plus a guide). Zero-risk quick win, but provides no tools, no session context,
   no slash commands. Adopted as an interim documentation path, not a
   replacement for this ADR.
3. **Separate repository.** Forcefully rejected by `ADR-0033`'s drift rationale;
   keeping the package in this repository preserves one source of truth and one
   release review surface.
4. **MCP server bridge.** Would add a new govctl API surface (RFC change) and
   conflicts with govctl's stated "artifacts are the control plane, not an MCP
   server" design position.
5. **Vendoring the govctl binary in the npm package.** Rejected: release assets
   are ~8–11 MB per platform; version drift and supply-chain surface outweigh
   convenience. The plugin resolves `govctl` from `PATH` or an explicit
   `binaryPath` and returns an actionable error when absent.

## Consequences

Positive:

- DSH users get govctl tooling with no change to govctl's normative CLI surface.
- Asset ownership stays in `.claude/`, with a generator + CI drift check instead
  of hand-maintained copies.
- The integration can track DSH's release cadence independently.

Negative / risks:

- Two delivery channels (`govctl agent` for Claude/Codex, `govctl-dsh` for DSH)
  must be documented so `ADR-0061`'s facade promise is not misread.
- Reviewers become skills on DSH (no Markdown-defined agents in DSH 0.2.x);
  fidelity is lower than Claude Code's agent projection and must be stated.
- Session/edit hook behavior is advisory and depends on an optional
  `@deepseek-ai/dsh-llm` peer for sourced message context; without it the plugin
  falls back to a runtime prompt context.
- The plugin lives in a Rust repo but is a Node package; CI needs a JS job plus
  the existing Rust gates.
- No machine-readable `status`/`check` output exists in govctl today, so the
  wrapper synthesizes results from exit codes and diagnostics text.

## Validation plan

- Asset sync determinism: `npm run check-assets` fails on drift.
- Unit tests with a mock Cordis context: tool registrations, approval gate,
  skill count, event listeners, argv construction.
- End-to-end smoke on a frozen DSH version: scratch `DSH_HOME`,
  `dsh plugin add <path>`, `--dump-config` layer check, web boot load check.
- Manual compatibility matrix: DSH app version × supported govctl range.

## Open questions

1. Ownership: is `govctl-dsh` an official govctl-org package or a companion? This
   ADR assumes the repository hosts the package; npm ownership still needs a
   decision.
2. Publish cadence and npm scope/package name ownership.
3. Whether reviewer parity requires subagent-based orchestration instead of
   skills.
4. Whether a DSH plugin-market index entry is desired.
5. Whether to eventually pursue Alternative 1 (native runtime) after DSH's API
   stabilizes.
