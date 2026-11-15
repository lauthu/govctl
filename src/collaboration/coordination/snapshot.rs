//! Canonical document packages for external runners — [[RFC-0012:C-SNAPSHOT]].
use super::{failure, git::Repository, history, mark_unavailable, model, policy::Context};
use crate::{
    collaboration::{Target, records, source},
    config::Config,
    diagnostic::{Diagnostic, DiagnosticResult},
    render::{
        RenderProjection, render_adr_with_projection, render_rfc_with_projection,
        render_work_item_with_projection,
    },
};
use ed25519_dalek::{VerifyingKey, pkcs8::DecodePublicKey};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const RESPONSE_LIMIT: usize = 16 * 1024 * 1024;

pub(super) fn request(context: &Context, request_id: &str) -> DiagnosticResult<Value> {
    model::bounded(request_id, 300)?;
    let mut repo = Repository::open(context)?;
    let head = repo.fetch()?;
    let mut state = history::replay(&mut repo, context, &head).map_err(source_error)?;
    mark_unavailable(&repo, &mut state)?;
    let request = state.requests.get(request_id).ok_or_else(|| {
        failure(
            "COORD_NOT_FOUND",
            "The request has not been published on canonical main.",
        )
    })?;
    if request.stale {
        return Err(failure(
            "COORD_SOURCE_CHANGED",
            "The request's fixed governance or code input is no longer current or available.",
        ));
    }
    let target = &request.target;
    let record_path = format!("gov/collaboration/{}/{}", target.kind, target.id);
    let (_snapshot, config) = repo.source_snapshot(&head, Some(&record_path))?;
    let source = source::sources(&config, &context.repo_id)
        .map_err(source_error)?
        .into_iter()
        .find(|source| source.target.id == target.id)
        .ok_or_else(|| failure("COORD_SOURCE_CHANGED", "The fixed source is unavailable."))?;
    // Retain the distinct code reference while checking native source identity.
    let mut captured_target = source.target.clone();
    captured_target.commit = target.commit.clone();
    if &captured_target != target {
        return Err(failure(
            "COORD_SOURCE_CHANGED",
            "Captured source does not match the request's fixed input.",
        ));
    }
    let (body, content) = document(&config, target)?;
    let evidence = author_evidence(&config, target, context)?;
    let response = json!({
        "protocolVersion":1,"repoId":context.repo_id,"head":head,
        "authority":{"id":context.authority_id,"revision":context.current_policy_revision},
        "request":request,"target":target,"body":body,"content":content,
        "authorEvidence":{"createdBy":source.created_by,"authorship":evidence},
    });
    bounded_response(response)
}

/// Keep candidate document identity separate from both main and its Git commit.
/// [[RFC-0012:C-CANDIDATE-SNAPSHOT]] does not change stored request semantics.
pub(super) fn candidate(context: &Context, request_id: &str) -> DiagnosticResult<Value> {
    model::bounded(request_id, 300)?;
    let mut repo = Repository::open(context)?;
    let head = repo.fetch()?;
    let mut state = history::replay(&mut repo, context, &head).map_err(source_error)?;
    mark_unavailable(&repo, &mut state)?;
    let request = state.requests.get(request_id).ok_or_else(|| {
        failure(
            "COORD_NOT_FOUND",
            "The request has not been published on canonical main.",
        )
    })?;
    let commit = match request.target.commit.as_deref() {
        Some(commit)
            if request.requested_operation == "review" && request.write_targets.is_empty() =>
        {
            commit
        }
        _ => {
            return Err(failure(
                "COORD_UNSUPPORTED",
                "Candidate snapshots require a read-only review with a fixed code commit.",
            ));
        }
    };
    if request.stale {
        return Err(failure(
            "COORD_SOURCE_CHANGED",
            "The request's fixed governance or code input is no longer current or available.",
        ));
    }
    let (_baseline_snapshot, baseline_config) = repo.source_snapshot(&head, None)?;
    let baseline = document_source(&baseline_config, context, &request.target)?;
    let mut fixed_baseline = request.target.clone();
    fixed_baseline.commit = None;
    if baseline.target != fixed_baseline {
        return Err(failure(
            "COORD_SOURCE_CHANGED",
            "Captured baseline does not match the request's fixed input.",
        ));
    }
    let (baseline_body, baseline_content) =
        document(&baseline_config, &baseline.target).map_err(candidate_source_error)?;
    let record_path = format!(
        "gov/collaboration/{}/{}",
        baseline.target.kind, baseline.target.id
    );
    let (_candidate_snapshot, candidate_config) =
        repo.source_snapshot(commit, Some(&record_path))?;
    let candidate = document_source(&candidate_config, context, &baseline.target)?;
    let (body, content) =
        document(&candidate_config, &candidate.target).map_err(candidate_source_error)?;
    // Authorship is attached before the candidate commit is created: its native
    // document target has a null commit, not the request's baseline or code SHA.
    let evidence = author_evidence(&candidate_config, &candidate.target, context)?;
    bounded_response(json!({
        "protocolVersion":1,"repoId":context.repo_id,"head":head,
        "authority":{"id":context.authority_id,"revision":context.current_policy_revision},
        "request":request,
        "baseline":{"target":baseline.target,"body":baseline_body,"content":baseline_content},
        "candidate":{"commit":commit,"target":candidate.target,"body":body,"content":content,
            "authorEvidence":{"createdBy":candidate.created_by,"authorship":evidence}},
    }))
}

fn document_source(
    config: &Config,
    context: &Context,
    target: &Target,
) -> DiagnosticResult<source::Source> {
    let sources = source::sources(config, &context.repo_id).map_err(candidate_source_error)?;
    let mut identities = BTreeSet::new();
    if sources
        .iter()
        .any(|source| !identities.insert(&source.target.id))
    {
        return Err(failure(
            "COORD_SOURCE_CHANGED",
            "Duplicate candidate artifact identity.",
        ));
    }
    sources
        .into_iter()
        .find(|source| {
            source.target.repo_id == target.repo_id
                && source.target.kind == target.kind
                && source.target.id == target.id
        })
        .ok_or_else(|| failure("COORD_SOURCE_CHANGED", "The fixed source is unavailable."))
}

fn candidate_source_error(error: Diagnostic) -> Diagnostic {
    if error.message.contains("COORD_") {
        error
    } else {
        failure(
            "COORD_SOURCE_CHANGED",
            "A candidate governance source is missing or malformed.",
        )
    }
}

fn author_evidence(
    config: &Config,
    target: &Target,
    context: &Context,
) -> DiagnosticResult<Vec<records::History>> {
    let policy = &context.policies[&context.current_policy_revision];
    let keys: Vec<VerifyingKey> = policy
        .trusted_issuers
        .iter()
        .map(|issuer| {
            VerifyingKey::from_public_key_pem(&issuer.public_key_pem)
                .map_err(|_| failure("COORD_POLICY_INVALID", "Invalid issuer public key."))
        })
        .collect::<DiagnosticResult<_>>()?;
    Ok(records::history(config, target, &keys)?
        .into_iter()
        .filter(|entry| match &entry.event {
            // Preserve native diagnostics instead of silently hiding bad evidence.
            None => true,
            Some(event) => {
                event["type"] == "authorship"
                    && serde_json::from_value::<Target>(event["target"].clone())
                        .is_ok_and(|value| value == *target)
            }
        })
        .collect())
}

fn bounded_response(response: Value) -> DiagnosticResult<Value> {
    if serde_json::to_vec(&response)
        .map_err(|_| super::invalid_input("Cannot encode request input snapshot."))?
        .len()
        > RESPONSE_LIMIT
    {
        return Err(failure(
            "COORD_UNSUPPORTED",
            "The complete request input snapshot exceeds 16 MiB.",
        ));
    }
    Ok(response)
}

fn source_error(error: Diagnostic) -> Diagnostic {
    if [
        "COLLAB_NOT_FOUND",
        "COLLAB_INVALID_SOURCE",
        "COLLAB_SOURCE_CHANGED",
    ]
    .iter()
    .any(|marker| error.message.contains(marker))
    {
        failure(
            "COORD_SOURCE_CHANGED",
            "A fixed governance source or referenced clause is missing or invalid.",
        )
    } else {
        error
    }
}

fn document(config: &Config, target: &Target) -> DiagnosticResult<(String, Value)> {
    let encode_error = |_| super::invalid_input("Cannot encode native artifact content.");
    match target.kind.as_str() {
        "rfc" => {
            let path = crate::load::find_rfc_toml(config, &target.id).ok_or_else(|| {
                failure(
                    "COORD_SOURCE_CHANGED",
                    "The fixed RFC source is unavailable.",
                )
            })?;
            let mut rfc = crate::load::load_rfc(config, &path).map_err(Diagnostic::from)?;
            let directory = path
                .parent()
                .ok_or_else(|| super::invalid_input("The RFC source directory is unavailable."))?;
            // Native loading also discovers orphan clauses for validation. The
            // fixed RFC-0011 digest binds only section references, in whose order
            // the snapshot exposes content. Do not add unbound orphan text.
            rfc.clauses = rfc
                .rfc
                .sections
                .iter()
                .flat_map(|section| &section.clauses)
                .map(|reference| {
                    rfc.clauses
                        .iter()
                        .find(|entry| entry.path == directory.join(reference))
                        .cloned()
                        .ok_or_else(|| {
                            super::invalid_input("A referenced RFC clause is unavailable.")
                        })
                })
                .collect::<DiagnosticResult<_>>()?;
            let mut content = serde_json::to_value(&rfc.rfc).map_err(encode_error)?;
            content["clauseSources"] = serde_json::to_value(
                rfc.clauses
                    .iter()
                    .map(|entry| &entry.spec)
                    .collect::<Vec<_>>(),
            )
            .map_err(encode_error)?;
            Ok((
                render_rfc_with_projection(&rfc, RenderProjection::Archive, None)?,
                content,
            ))
        }
        "adr" => {
            let adr = crate::artifact_catalog::load_adr_by_id(config, &target.id)?;
            Ok((
                render_adr_with_projection(&adr, RenderProjection::Archive)?,
                serde_json::to_value(&adr.spec).map_err(encode_error)?,
            ))
        }
        "work" => {
            let work = crate::artifact_catalog::load_work_item_by_id(config, &target.id)?;
            Ok((
                render_work_item_with_projection(&work, RenderProjection::Archive)?,
                serde_json::to_value(&work.spec).map_err(encode_error)?,
            ))
        }
        _ => Err(super::invalid_input("Unknown artifact kind.")),
    }
}
