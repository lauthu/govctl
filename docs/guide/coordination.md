# Repository coordination

`govctl coord` coordinates requests and execution ownership across independent
clones. Canonical repository `main` is the authority: a request or claim becomes
effective only after its signed control event is accepted there. RFCs, ADRs and
Work Items remain the collaboration targets; requests are attached records, not a
fourth governed task type.

Version 1 publishes control events and immutable outcome references. An external
client still authenticates participants, starts agents, applies code or source
changes and invokes lifecycle commands. A completion receipt does not say that a
process ran, code was merged, verification passed or an artifact advanced.

## Configure an external authority

Check support without opening a repository:

```bash
govctl coord capabilities
```

The response advertises protocol version `1`, capability
`repository-main-coordination-v1` and publication mode `direct-fast-forward`.

Keep the context and private signing key outside the repository. The context names
the canonical remote, trusted issuer keys and subject permissions. A subject ID
is a lightweight policy identity; it does not require an Agent registry or a
runtime profile. Actors retain the attribution shape described in
[Authors, Comments, and Reviews](collaboration.md).

An example context is shown below. Replace the remote, activation commit, public
key and its fingerprint with real values. The activation commit must be on main's
first-parent history. `keyId` is the SHA-256 of the issuer's canonical LF Ed25519
SPKI PEM public key, including its final newline.

```json
{
  "protocolVersion": 1,
  "repoId": "owner/repository",
  "remote": "ssh://git@example.com/owner/repository.git",
  "branch": "main",
  "authorityId": "authority:team",
  "currentPolicyRevision": "policy-1",
  "policies": {
    "policy-1": {
      "effectiveAfterCommit": "<main commit SHA>",
      "trustedIssuers": [
        {
          "keyId": "<SHA-256 of public PEM>",
          "publicKeyPem": "<Ed25519 SPKI PEM with LF newlines>"
        }
      ],
      "grants": [
        {
          "subjectId": "human:owner",
          "issuerKeyId": "<same keyId>",
          "actions": [
            "publish",
            "claim",
            "handoff",
            "accept",
            "release",
            "cancel",
            "takeover",
            "complete",
            "delegate"
          ]
        },
        {
          "subjectId": "agent:reviewer",
          "issuerKeyId": "<same keyId>",
          "actions": ["claim", "handoff", "accept", "release", "complete"]
        }
      ]
    }
  }
}
```

The remote may be an absolute filesystem path, HTTPS URL, SSH URL or SCP-style
SSH repository address. A local alias such as `origin` is insufficient because
these commands are independent of the calling workspace. The branch is `main`.
Git credentials stay in the external Git authentication setup, not control
events. The private signing key is Ed25519 PKCS#8 PEM; protect it as described in
the collaboration guide.

An issuer authenticates the actor it asserts. A repository-embedded key, an
asserted role or Git push access alone grants no coordination permission. Grants
bind both the subject and the issuing key. For a delegated action, the actual
actor needs the action permission plus `delegate`, and `onBehalfOf` names the
execution holder. The new holder also needs a `claim` grant under that issuer.

When changing policy, add a new revision with a distinct main activation commit,
retain the earlier revisions and update `currentPolicyRevision`. Replay selects
the policy active at each event's actual publication parent. Historical grants
validate historical facts; they cannot authorize a new operation after a newer
policy becomes effective. Removing historical context makes validation fail
closed. Policy changes do not silently transfer an existing claim.

## Read canonical state

```bash
govctl coord list --context-file /secure/govctl/context.json
```

The JSON snapshot includes `head`, `authority`, available artifact `targets`,
`requests` and artifact `writes`. Use this canonical snapshot for inputs rather
than a potentially stale local checkout. A request exposes its last
`stateEventId`, request `generation`, holder, optional handoff, effective result
and `stale` flag. Write generations remain visible after release.

Each later action supplies the exact `expectedState`, `expectedGeneration` and
`expectedWrites` values it observed. A state or write conflict means the intent
needs review against a fresh snapshot; merely editing the expected values does
not establish that the same action is still appropriate.

## Publish and claim a review request

Copy the complete target of the artifact you want to review from `list`. A publish
input has this form:

```json
{
  "operationId": "publish-rfc-0002-review-1",
  "action": "publish",
  "actor": { "subjectId": "human:owner", "name": "Owner" },
  "requestId": "rfc-0002-review-1",
  "target": {
    "repoId": "owner/repository",
    "kind": "rfc",
    "id": "RFC-0002",
    "version": "0.1.0",
    "contentDigest": "<canonical target digest>",
    "commit": null
  },
  "requestedOperation": "review",
  "writeTargets": [],
  "description": "Review the RFC's behavior and compatibility guarantees."
}
```

Save the input in `publish.json`, then publish it:

```bash
govctl coord apply --context-file /secure/govctl/context.json \
  --sign-key-file /secure/govctl/issuer.pem --stdin < publish.json
```

The receipt identifies the accepted `eventId`, `publicationCommit`, observed
`head` and projected request. Only a successfully published effective receipt
grants authority. Global `--dry-run` validates without publishing and returns
`effective: false` with `publicationCommit: null`.

To claim this review for an agent through the authorized owner, use the state
from the publish receipt or a fresh snapshot:

```json
{
  "operationId": "claim-rfc-0002-review-1",
  "action": "claim",
  "actor": { "subjectId": "human:owner", "name": "Owner" },
  "requestId": "rfc-0002-review-1",
  "expectedState": "<current stateEventId>",
  "expectedGeneration": 0,
  "expectedWrites": {},
  "holder": {
    "subjectId": "agent:reviewer",
    "claimId": "review-attempt-1",
    "sessionId": "review-run-1"
  },
  "onBehalfOf": "agent:reviewer"
}
```

Apply it with the same command and a different input file. The recorded actor
remains the owner; the execution holder is the reviewer. An agent acting directly
uses its own actor and omits `onBehalfOf`. `claimId` and `sessionId` identify an
execution attempt, separate from the stable contributor identity.

One request has at most one current holder. Distinct review requests may inspect
the same revision concurrently. A `draft`, `edit` or `implement` request instead
lists its existing artifact write scopes in `writeTargets`, including its target
artifact. Claiming it also reserves all those scopes exclusively. These claims
coordinate participating clients; they do not block arbitrary source edits or
Git pushes made outside this protocol. A changed source prevents effective
completion against the old input.

## Transfer, release or finish work

Every action below carries the common operation/actor/request fields and current
causal preconditions. Holder actions also carry the accepted `claimId` and
`sessionId`, with explicit delegation when applicable.

| Action     | Additional input                                                              | Result                                                                              |
| ---------- | ----------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `handoff`  | `toSubjectId` and `checkpoint` with `summary`, nullable `commit`, `remaining` | Records an offer. The current holder keeps authority.                               |
| `accept`   | A new `holder` for the offered subject                                        | Atomically replaces request and write claims with higher generations.               |
| `release`  | Current attempt identity                                                      | Clears the offer and ownership; the fixed request returns to open.                  |
| `cancel`   | Nonempty `reason`                                                             | Makes the request terminal and invalidates claims.                                  |
| `takeover` | New eligible `holder` and nonempty `reason`                                   | Requires explicit takeover permission and atomically invalidates the former holder. |
| `complete` | Current attempt identity and `result`                                         | Records an outcome, makes the request terminal and releases ownership.              |

A result has a nonempty `summary`, with optional `verdict`, `commit` and
`references`. Verdict is `approved`, `changes_requested` or `comment`. For example:

```json
{
  "summary": "The proposed migration needs an explicit rollback condition.",
  "verdict": "changes_requested",
  "commit": null,
  "references": []
}
```

Use exact immutable commits for code targets and evidence. A pushed feature-branch
commit can be referenced before it merges into main; an unpublished local commit
cannot. A new grant or effective completion checks that its fixed code input is
still reachable from the canonical remote's advertised branches or tags. Main
commits containing only coordination records do not change that code target.

A handoff offer can be replaced using current causal state, or cleared by
release. There is no automatic timeout or heartbeat takeover. After acceptance,
takeover, release or completion, an old process may still run, but its previous
generation cannot finish the current request.

Stale requests can still be cancelled or released. A late completion may use
`allowObsolete: true` to append ineffective evidence only when its supplied state,
generation, holder, attempt and input match accepted history. It cannot alter
the current holder, state, result or generations. Completed and cancelled
requests cannot be reopened; changed inputs need a new request identity.

## Retries, history and limits

Keep `operationId` stable for an unchanged logical operation. An exact replay
returns the accepted receipt even after completion. Changing its semantic input,
including signer identity or causal preconditions, requires a new operation ID.
A transport failure may hide an already accepted push: the client reconciles
canonical history before reporting success. `COORD_PUBLICATION_UNKNOWN` means no
new grant can be assumed until the accepted operation is observed.

Control records use signed collaboration format version 2 `execution` envelopes
with payload protocol `govctl-coordination-v1`. They remain attached under
`gov/collaboration/<kind>/<artifactId>/`. Their order comes from actual main
commits, not timestamps. Records cannot be edited or deleted without invalidating
the control history. Existing standalone collaboration formats 1 and 2 retain
their original meaning; they do not become execution claims or change approval
targets through migration.

Historical code references retain their signed issuer assertion after a feature
branch is deleted. Replay does not claim to reconstruct past remote refs or erase
accepted ownership because those refs are no longer available. Fresh actions
still validate current reachability where required.

Version 1 requires direct fast-forward publication. Protected-branch rejection
does not grant ownership, create a pull request or bypass protection. Local hooks
are not executed; remote hooks and branch protections remain effective. There is
no automatic source application, lifecycle transition, agent launch or exactly-once
process execution. See [RFC-0012](../rfc/RFC-0012.md) for the full wire contract and
stable `COORD_*` error markers.
