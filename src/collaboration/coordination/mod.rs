//! Repository-backed request publication — [[RFC-0012]].
mod git;
mod history;
mod model;
mod policy;
mod state;

use crate::{
    cli::CoordCommand,
    collaboration::{Target, records, sha256},
    diagnostic::{Diagnostic, DiagnosticCode, DiagnosticResult},
    write::WriteOp,
};
use ed25519_dalek::{
    SigningKey,
    pkcs8::{DecodePrivateKey, EncodePublicKey, spki::der::pem::LineEnding},
};
use git::Repository;
use model::{Accepted, Operation, PROTOCOL};
use policy::Context;
use serde_json::{Value, json};
use state::State;
use std::{collections::BTreeMap, io::Read, path::Path};

pub(super) fn failure(code: &str, message: &str) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::E0820InvalidFieldValue,
        format!("{code}: {message}"),
        "coordination",
    )
}

pub(super) fn invalid_input(message: &str) -> Diagnostic {
    failure("COORD_INVALID_INPUT", message)
}

fn print(value: &Value) -> DiagnosticResult<()> {
    println!(
        "{}",
        serde_json::to_string(value)
            .map_err(|_| invalid_input("Cannot encode coordination response."))?
    );
    Ok(())
}

pub(crate) fn execute(command: CoordCommand, op: WriteOp) -> DiagnosticResult<()> {
    execute_inner(command, op).map_err(|error| {
        if error.message.contains("COORD_") {
            error
        } else {
            invalid_input("A source, actor, signing key or operation field failed validation.")
        }
    })
}

fn execute_inner(command: CoordCommand, op: WriteOp) -> DiagnosticResult<()> {
    match command {
        CoordCommand::Capabilities => print(&json!({
            "protocolVersion":1,"capabilities":["repository-main-coordination-v1"],
            "actions":model::ACTIONS,"publicationModes":["direct-fast-forward"],"sourceMutation":false,
        })),
        CoordCommand::List { context_file } => {
            let context = Context::load(&context_file)?;
            let mut repo = Repository::open(&context)?;
            let head = repo.fetch()?;
            let mut state = history::replay(&mut repo, &context, &head)?;
            mark_unavailable(&repo, &mut state)?;
            let sources = repo.targets(&head, &context.repo_id)?;
            print(&snapshot(&context, &head, &state, &sources))
        }
        CoordCommand::Apply {
            context_file,
            sign_key_file,
            ..
        } => {
            let context = Context::load(&context_file)?;
            let mut bytes = Vec::new();
            std::io::stdin()
                .take(records::EVENT_LIMIT + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| invalid_input("Cannot read the coordination operation."))?;
            if bytes.len() as u64 > records::EVENT_LIMIT {
                return Err(invalid_input("Coordination input exceeds 2 MiB."));
            }
            let raw: Value = serde_json::from_slice(&bytes)
                .map_err(|_| invalid_input("Coordination input must be one JSON object."))?;
            let operation = Operation::parse(&raw)?;
            let (issuer, pem) = signer(&sign_key_file)?;
            let mut repo = Repository::open(&context)?;
            print(&publish(
                &mut repo,
                &context,
                &raw,
                &operation,
                &issuer,
                &pem,
                &sign_key_file,
                op.is_preview(),
            )?)
        }
    }
}

fn signer(path: &Path) -> DiagnosticResult<(String, String)> {
    let bytes = git::read_external(path, 16384)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(path)
            .map_err(|_| invalid_input("Cannot inspect signing key."))?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err(invalid_input(
                "Private signing key must not be accessible to other users.",
            ));
        }
    }
    let pem = std::str::from_utf8(&bytes)
        .map_err(|_| invalid_input("Private signing key is not UTF-8 PEM."))?;
    let key = SigningKey::from_pkcs8_pem(pem)
        .map_err(|_| invalid_input("Private signing key must be Ed25519 PKCS#8 PEM."))?;
    let public = key
        .verifying_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(|_| invalid_input("Cannot encode issuer public key."))?;
    Ok((sha256(public.as_bytes()), public))
}

fn snapshot(
    context: &Context,
    head: &str,
    state: &State,
    sources: &BTreeMap<String, Target>,
) -> Value {
    json!({"protocolVersion":1,"repoId":context.repo_id,"head":head,
        "authority":{"id":context.authority_id,"revision":context.current_policy_revision},
        "artifacts":sources.values().map(|target| json!({"target":target})).collect::<Vec<_>>(),
        "requests":state.requests.values().collect::<Vec<_>>(),
        "writes":state.writes.values().collect::<Vec<_>>()})
}

fn mark_unavailable(repo: &Repository, state: &mut State) -> DiagnosticResult<()> {
    for request in state.requests.values_mut() {
        if let Some(commit) = &request.target.commit {
            request.stale |= !repo.published_commit(commit)?;
        }
    }
    Ok(())
}

fn receipt(
    operation_id: &str,
    accepted: &Accepted,
    state: &State,
    head: &str,
    replayed: bool,
    preview: bool,
) -> Value {
    json!({"protocolVersion":1,"operationId":operation_id,"requestId":accepted.request_id,
        "eventId":accepted.event_id,"publicationCommit":if preview {None} else {Some(&accepted.publication_commit)},
        "head":head,"replayed":replayed,"effective":accepted.effective && !preview,
        "request":state.requests.get(&accepted.request_id)})
}

fn require_published(repo: &Repository, commit: &str) -> DiagnosticResult<()> {
    if repo.published_commit(commit)? {
        Ok(())
    } else {
        Err(failure(
            "COORD_SOURCE_CHANGED",
            "A referenced commit is not available from canonical advertised branches or tags.",
        ))
    }
}

fn check_commits(
    repo: &Repository,
    operation: &Operation,
    target: &Target,
) -> DiagnosticResult<()> {
    if (matches!(
        operation.action.as_str(),
        "publish" | "claim" | "accept" | "takeover"
    ) || (operation.action == "complete" && !operation.allow_obsolete))
        && let Some(commit) = &target.commit
    {
        require_published(repo, commit)?;
    }
    if let Some(commit) = operation
        .checkpoint
        .as_ref()
        .and_then(|c| c.commit.as_deref())
    {
        require_published(repo, commit)?;
    }
    if let Some(commit) = operation.result.as_ref().and_then(|r| r.commit.as_deref()) {
        require_published(repo, commit)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn publish(
    repo: &mut Repository,
    context: &Context,
    raw: &Value,
    operation: &Operation,
    issuer: &str,
    pem: &str,
    sign_key: &Path,
    preview: bool,
) -> DiagnosticResult<Value> {
    let semantic = history::digest(raw, issuer);
    let mut head = repo.fetch()?;
    for _ in 0..4 {
        let state = history::replay(repo, context, &head)?;
        // [[RFC-0012:C-REQUEST]]: exact retry survives terminal status and revocation.
        if let Some(accepted) = state.operations.get(&operation.operation_id) {
            if accepted.semantic_digest != semantic {
                return Err(failure(
                    "COORD_IDEMPOTENCY_CONFLICT",
                    "The operation identity already names different semantic input or issuer.",
                ));
            }
            return Ok(receipt(
                &operation.operation_id,
                accepted,
                &state,
                &head,
                true,
                preview,
            ));
        }
        let policy = context
            .policies
            .get(&context.current_policy_revision)
            .ok_or_else(|| failure("COORD_POLICY_INVALID", "Current policy is missing."))?;
        policy.trust(issuer, pem)?;
        let target = state.target(operation)?;
        if target.repo_id != context.repo_id {
            return Err(invalid_input(
                "Target repository identity does not match the external context.",
            ));
        }
        check_commits(repo, operation, &target)?;
        let sources = repo.targets(&head, &context.repo_id)?;
        let id = history::event_id(operation, &target);
        let mut staged = state.clone();
        let effective = staged.apply(operation, policy, issuer, &sources, &id)?;
        staged.mark_stale(&sources);
        let event = records::create(
            &target,
            "execution",
            &operation.actor,
            json!({
                "protocol":PROTOCOL,"operation":raw,"semanticDigest":semantic,"parentCommit":head,
                "authority":{"id":context.authority_id,"revision":context.current_policy_revision},
            }),
            &operation.operation_id,
            Some(sign_key),
        )?;
        // Rotation between the two key reads must fail before remote publication.
        if event["signature"]["keyId"] != issuer || event["signature"]["publicKey"] != pem {
            return Err(failure(
                "COORD_UNAUTHORIZED",
                "The signing key changed while preparing publication; retry with consistent context.",
            ));
        }
        let bytes = serde_json::to_vec_pretty(&event)
            .map_err(|_| invalid_input("Cannot encode publication."))?;
        let accepted = Accepted {
            request_id: operation.request_id.clone(),
            semantic_digest: semantic.clone(),
            event_id: id.clone(),
            publication_commit: String::new(),
            effective,
        };
        if preview {
            return Ok(receipt(
                &operation.operation_id,
                &accepted,
                &staged,
                &head,
                false,
                true,
            ));
        }
        let candidate = repo.candidate(&head, &git::record_path(&target, &id), &bytes)?;
        let pushed = repo.push(&candidate);
        if pushed.as_ref().is_ok_and(|output| output.success) {
            let accepted = Accepted {
                publication_commit: candidate.clone(),
                ..accepted
            };
            return Ok(receipt(
                &operation.operation_id,
                &accepted,
                &staged,
                &candidate,
                false,
                false,
            ));
        }
        // No authority is granted on a failed/ambiguous transport result.
        let refreshed = repo.fetch().map_err(|_| {
            failure(
                "COORD_PUBLICATION_UNKNOWN",
                "Publication could not be reconciled; retry the same operation identity.",
            )
        })?;
        let observed = history::replay(repo, context, &refreshed)?;
        if let Some(accepted) = observed.operations.get(&operation.operation_id) {
            if accepted.semantic_digest != semantic {
                return Err(failure(
                    "COORD_IDEMPOTENCY_CONFLICT",
                    "A concurrent publication used this operation identity for different input.",
                ));
            }
            return Ok(receipt(
                &operation.operation_id,
                accepted,
                &observed,
                &refreshed,
                true,
                false,
            ));
        }
        if refreshed == head {
            let explicitly_rejected = pushed.as_ref().is_ok_and(|output| {
                !output.timed_out
                    && String::from_utf8_lossy(&output.stdout).contains("[remote rejected]")
            });
            if !explicitly_rejected {
                return Err(failure(
                    "COORD_PUBLICATION_UNKNOWN",
                    "No accepted publication could be confirmed; retry the same operation identity.",
                ));
            }
            return Err(failure(
                "COORD_PUBLISH_REJECTED",
                "Canonical main rejected publication; no ownership was granted. PR or merge-queue publication is not supported.",
            ));
        }
        head = refreshed;
        // Re-enter full semantic validation against the new parent. No rebase.
    }
    Err(failure(
        "COORD_STATE_CONFLICT",
        "Canonical main kept advancing; retry after refreshing its state.",
    ))
}
