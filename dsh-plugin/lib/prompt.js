/**
 * Standing governance policy contributed as a system-prompt section when the
 * session works in a govctl-governed repository.
 *
 * Kept short on purpose: this text is sent on every model request. The detailed
 * procedures live in the bundled skills (`gov`, `quick`, `spec`, writers, ...).
 */
export const POLICY_SECTION = `## Governance mode (govctl)

This repository is governed by govctl. Live governance state appears under
"govctl state" in the runtime context. Governed artifacts — not chat history —
are the source of truth.

### Laws

1. RFC supremacy: a normative RFC outranks code, prompts, and convenience. If
   code and a normative RFC disagree, the code is the bug; never implement
   against a draft RFC.
2. Phase discipline: \`spec → impl → test → stable\`. Do not skip phases and do
   not write implementation while the governing RFC is still in \`spec\`.
3. No silent deviation: when behavior is unspecified or ambiguous, stop and ask
   instead of inventing behavior.

### Hard stops

- Do not edit files under \`gov/\` directly. Use the resource commands
  (\`govctl rfc|adr|work|guard|clause ...\`, or the \`govctl_run\` tool) so
  lifecycle-owned fields and signatures stay valid. Direct edits are acceptable
  only when the current CLI has no suitable operation; then run
  \`govctl_check\` afterwards.
- Ask the user before lifecycle or destructive operations: accept, reject,
  finalize, advance, bump, deprecate, supersede, delete.
- Substantive implementation work needs a matching active Work Item.

### Before substantive work

Call \`govctl_status\` first. Read the matching Work Item with \`govctl_show\`
(\`resource: "work"\`) and the RFCs/ADRs it references. Load the \`gov\` skill for
the governed-implementation workflow, \`quick\` for a trivial non-behavioral
change, or \`spec\` when no code is needed.`
