# Authors, comments, and reviews

RFCs, ADRs, and Work Items can carry explicit creator attribution and attached
collaboration history. Comments and reviews belong to the artifact; they are not
additional task types. Lattice can schedule agents and manage credentials while
govctl keeps portable records in the repository.

## Attribute a creation or edit

An actor file identifies the person or agent making a change:

```json
{
  "subjectId": "agent:dsh",
  "kind": "agent",
  "agentId": "dsh",
  "name": "DSH",
  "role": "implementer"
}
```

Pass it explicitly when creating an artifact or editing authored content:

```bash
govctl rfc new "Release design" --actor-file actor.json --repo-id owner/repo
govctl adr new "Storage decision" --actor-file actor.json --repo-id owner/repo
govctl work new "Implement storage" --actor-file actor.json --repo-id owner/repo
govctl rfc edit RFC-0001 title --set "Updated release design" \
  --actor-file actor.json --repo-id owner/repo
```

The immutable `created_by` metadata declares the creator. An attached authorship
record describes contributors to the exact resulting source revision. Edits
preserve previous contributors as declarations and add the explicit editor.
Responsibility assignments and status ticks do not establish authorship.
Existing documents without attribution stay unknown; govctl does not infer an
author from owners, Git configuration, or the last reviewer.

## Append a comment or review

Write a comment payload to `comment.json`:

```json
{ "body": "Please clarify the rollout limit." }
```

Then append it with a stable key for safe retries:

```bash
govctl collab add RFC-0001 --repo-id owner/repo --type comment \
  --actor-file actor.json --payload-file comment.json \
  --idempotency-key rollout-question-1
```

The receipt includes the event ID. To reply, add `"replyTo":"ev_..."` to a
comment payload. For a review, use `--type review` and a payload such as:

```json
{
  "scope": "artifact",
  "verdict": "changes_requested",
  "reason": "Define a measurable rollout limit."
}
```

Verdicts are `approved`, `changes_requested`, and `comment`. A resolution is a
new `--type resolution` record with `reviewId` and `reason`. Replies and
resolutions must reference an existing comment or review on the same artifact.
An effective resolution requires a trusted original reviewer or an actor with
an owner, arbiter, or admin role.

## Declared identity and trusted conclusions

Native additions are unsigned declarations by default. Their author and body
are visible, but they do not count as trusted approval. For a signed addition,
pass `--sign-key-file` with a local Ed25519 PKCS#8 PEM private key. On Unix, the
file must be accessible only by its owner. Keep private keys outside the
repository. Only the public key and signature enter the event.

Reading a signed event verifies its cryptographic integrity. Trust is separate:
provide an explicitly trusted Ed25519 SPKI PEM public key when querying:

```bash
govctl collab list RFC-0001 --repo-id owner/repo --trust-key-file issuer.pub.pem
govctl collab list --all --repo-id owner/repo --trust-key-file issuer.pub.pem
govctl collab show RFC-0001 --repo-id owner/repo -o plain
```

An embedded public key never makes itself trusted. An approval counts only when
it is trusted, current, independent of every declared author, and complete
authorship for that revision has been confirmed. A signed `authorship` payload
contains `authors` and is confirmed when issued with an owner/admin role or an
automatic attribution claim from a trusted issuer. A newer unsigned contributor
declaration prevents incomplete confirmation from silently authorizing review.
Lattice additionally checks current participant membership and permissions.

Changing source makes previous approvals stale. Unresolved trusted change
requests continue blocking until explicitly resolved. Code reviews bind to a
Git commit and require a clean working tree to count as an approval. Comments
and reviews never advance an artifact lifecycle automatically.

## Storage and integration

Events live in `gov/collaboration/<kind>/<artifactId>/<eventId>.json`, where kind
is `rfc`, `adr`, or `work`. Each append atomically publishes one immutable file.
Repeated requests return the original receipt; reusing an idempotency key for
different content fails. `govctl check` validates stored structure, references,
and signature integrity without trusting unknown issuers.

`govctl collab target ID --repo-id owner/repo` returns a SHA-256 target over an
ordered manifest of raw TOML source bytes. RFC clause sources participate;
collaboration records do not. `collab add --target-file target.json` can bind a
review to a previously captured target. A changed target fails unless
`--allow-stale` explicitly preserves a historical conclusion.

`collab capabilities -o json` advertises protocol version 1. Clients may append
an existing signed envelope through `collab append ID --repo-id owner/repo
--stdin`. The first append retains the supplied bytes, and replay retains the
original stored bytes. Existing Lattice version 1 signatures and event IDs
remain intact; native version 2 also permits `signature: null`. No migration or
implicit re-signing of historical files is needed.

After installing this version, run `govctl migrate` in each existing repository
to refresh the additive schema assets, including the collaboration JSON schema
and optional creator fields. This refresh does not rewrite legacy event files.
