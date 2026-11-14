# Architecture

govctl stores governance in the repository and provides the CLI that validates,
changes and presents it. People and agent clients use the same contracts. Agent
runtime selection, credentials, authentication and scheduling belong to external
clients; govctl does not require a particular orchestration product.

The diagram separates local governance from repository-authoritative coordination.
Blue nodes are governance and collaboration components; green is clone-local
coordination; amber is the shared publication boundary on canonical `main`.

[![govctl architecture: local governance above repository-authoritative coordination](diagrams/architecture.svg)](diagrams/architecture.svg)

[Open the full-size diagram](diagrams/architecture.svg) ·
[Mermaid source](diagrams/architecture.mmd)

## Implemented architecture

The CLI reads authoritative files under `gov/`. A mutation passes the relevant
schema, reference, lifecycle and local write-safety checks before changing those
files. RFCs define obligations, ADRs explain decisions, and Work Items track
execution. RFC Clauses remain separately addressable resources within an RFC.
These three business entities are the targets of collaboration; they are not the
only artifact types in govctl.

Verification Guards and Conformance Cases support validation. Guards define
executable checks used by applicable completion gates. Conformance Cases connect
scenarios to versioned requirements and reusable Guards; a mapping does not itself
execute a test. `check` validates governance structure and consistency, while the
relevant verification commands run executable checks.

Authors, contributions, comments and review conclusions are attached to RFCs, ADRs
and Work Items. Native collaboration records are append-only and bind their
targets to an exact source revision. Their projections distinguish current,
stale, resolved and unverified conclusions. A review approval does not advance an
artifact's lifecycle by itself. Actor metadata records a contributor; a declared
name or model footprint is not authentication or proof of reviewer independence.
See [Authors, Comments, and Reviews](collaboration.md) for the current CLI and trust
contract.

Readers derive status, JSON responses, rendered Markdown and the read-only TUI
from this source. Generated RFC, ADR and Work Item pages are projections, not an
additional authority. The hand-authored guide explains the contracts; the RFCs
define them.

### Local coordination has a clone boundary

The current coordination mechanisms solve local problems:

| Mechanism                          | Scope and purpose                                                                                                                                                                 |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Governance write lock              | Serializes supported mutations of the same local governance root.                                                                                                                 |
| RFC claims and ID reservations     | Coordinate workspaces sharing one Git or jj repository store. RFC claims cover the version-sensitive operations defined by RFC-0010; they are not universal artifact write locks. |
| Work Item presence                 | Reports activity in other workspaces of that clone; it is advisory.                                                                                                               |
| Workspace loops and round evidence | Record local execution and verification progress for Work Items; they do not run a distributed agent scheduler.                                                                   |

Independent clones do not share these registries. A local claim is not a remote
execution grant, and an assignment or execution receipt in collaboration history
is not an exclusive distributed claim. Existing standalone commands remain useful
without a shared publication service. See [Parallel Workspaces](parallel-workspaces.md),
[RFC-0004](../rfc/RFC-0004.md), [RFC-0006](../rfc/RFC-0006.md) and
[RFC-0010](../rfc/RFC-0010.md).

## Repository-authoritative collaboration

[RFC-0012](../rfc/RFC-0012.md) defines coordination protocol v1. The `coord`
commands publish signed control events and derive request and ownership state
from canonical `main`. See [Repository Coordination](coordination.md) for setup,
JSON examples and first-version limits.

The configured canonical repository's `main` becomes the shared authority for
requests and execution ownership. Each clone can reconstruct accepted state from
the same history and explicit policy context. A local event or an unmerged pull
request grants no execution rights. The publication point is the conditional
fast-forward of canonical `main`, validated against the exact parent being
advanced. The first version supports direct fast-forward publication. Rejection
by a protected branch grants no ownership; a pull-request or merge-queue adapter
would need to validate the eventual main update and is not supplied by v1.

### Attached requests, two kinds of exclusivity

A request describes an operation on one existing RFC, ADR or Work Item at an exact
input revision. Request, claim, handoff and result records remain attached to that
entity. Their projected states do not create a fourth governed task artifact or
replace the entity's lifecycle. A cancelled review request, for example, does
not cancel its RFC.

Each request has at most one current execution holder. A modifying request also
needs exclusive write ownership of every existing artifact it will change.
Different review requests may examine the same immutable revision concurrently;
they own their individual requests, not the reviewed artifact's write scope.
Version 1 operates on existing artifacts. It does not create or apply source
artifacts as part of coordination publication.

A request's operation and input are fixed. Changed requirements or a newer input
need a new request identity. Completed and cancelled requests are terminal;
retrying a completed operation returns its recorded outcome instead of starting
another execution. This permits many collaboration patterns without prescribing
one RFC-to-ADR-to-Work-Item workflow. A client can request a review, an amendment,
drafting based on an agreed RFC, or implementation against a Work Item. Version 1
records the request and resulting evidence references; the external execution
flow applies source changes and invokes lifecycle commands under their existing
governance rules.

### Publication and retries

An external client proposes a request or claim against the observed authoritative
parent. The publication validator checks authorization, current input, existing
ownership and the operation's identity. Execution rights take effect only when
that proposal is accepted on `main`. The agent can then execute locally and
propose a result against its current claim generations.

Two clones claiming the same request from the same parent cannot both win. After
the first publication, the other must fetch the new authority and revalidate its
intent. Textual rebasing alone cannot make an obsolete claim valid. The same rule
applies when separate modifying requests compete for one artifact's write scope.

Every operation carries a stable identity. If the server accepted a publication
but its response was lost, retrying the same semantic operation returns the
accepted receipt. Reusing its identity with different content is an error. A
retry may update its publication-parent witness, but cannot silently substitute a
new actor, input, holder or result. This makes effects idempotent; it does not
guarantee that an external process physically runs only once.

### Handoff, takeover and late results

Request ownership and artifact write ownership each have increasing generations.
An explicit handoff first records an offer and checkpoint. Its current holder
keeps authority until the named successor accepts. Acceptance or an authorized
takeover atomically replaces the request and write-scope claims. The previous
holder can no longer publish an effective result for those scopes.
Completion, cancellation and release invalidate the
affected claims. A process may continue running after losing a claim, but its
late output cannot change an artifact or finish the current request.

The first version deliberately uses explicit transfer. Elapsed time, a missing
heartbeat or an unreachable machine does not automatically revoke or transfer
authority. The existing clone-local claim expiry rules remain separate. The
external policy identifies who may authorize takeover and why.

### Results bind to work, not incidental coordination commits

The protocol distinguishes these identities:

| Binding                                     | Meaning                                                                              |
| ------------------------------------------- | ------------------------------------------------------------------------------------ |
| Request and operation identities            | Which fixed task is being performed, and which logical update is being retried.      |
| Authoritative publication parent            | The `main` state used to validate a proposed coordination update.                    |
| Input artifact revision                     | The exact RFC, ADR or Work Item content used by the operation.                       |
| Claim generations and holder                | The current request and write-scope authority that permits an effective result.      |
| Immutable code commit                       | The code revision reviewed, when a request includes code review.                     |
| Outcome or resulting revision               | The durable conclusion or output of the operation.                                   |
| Authorization authority and policy revision | The external decision context used to permit the operation, without its credentials. |

A code-review target is an explicit immutable commit, separate from the current
coordination `main` HEAD. Publishing another claim or comment on `main` therefore
does not stale a review of unchanged document and code targets. A changed target
does stale it; a later client cannot reinterpret the old conclusion as approval
of the new target. Obsolete results may remain as explicitly ineffective history.
Stale input does not prevent cancellation or release of its request. An immutable
code reference can identify a pushed feature-branch commit before it is merged.
A completion receipt records an outcome; it does not establish that code was
merged, verification passed or a legacy standalone review was approved.

### Authorization and compatibility

External authorities authenticate participants, bind signers to subjects and
decide permissions. govctl validates protocol structure, revision bindings and
causal state using that explicit context. A key stored in the repository, a role
label or permission to push Git commits is insufficient by itself to authorize an
agent's action or make its approval independent. Scheduling and credentials stay
outside the repository's collaboration records. The actual actor remains distinct
from the execution holder: a delegated action needs explicit policy permission
and an `onBehalfOf` binding, and preserves both identities.

External policy revisions are anchored to main commits. Replay uses the policy
active at each control event's actual publication parent, while a new operation
uses the policy active at current main. Retaining a previous policy for history
does not revive its permissions. Invalid or rewritten control history fails
closed rather than silently discarding an ownership transition.

Publication mode advertises `repository-main-coordination-v1`. It does not silently fall
back to clone-local coordination. Existing collaboration format versions 1 and 2
retain their bytes, identities and standalone interpretation under
[RFC-0011](../rfc/RFC-0011.md). Old assignments and receipts are not promoted to
ownership, and old reviews are not rebound to the new code-target semantics.

Hosting and authentication adapters remain external integrations. The first
version has no automatic agent runner, source merger, lifecycle transition,
lease expiry or protected-branch bypass. A valid claim or successful publication
does not replace lifecycle and verification gates.
