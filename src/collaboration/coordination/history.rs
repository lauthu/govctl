//! Replay actual publication order, never wall-clock order — [[RFC-0012:C-HISTORY]].
use super::{
    failure,
    git::{Repository, record_path},
    model::{Accepted, Operation, PROTOCOL},
    policy::Context,
    state::State,
};
use crate::{
    collaboration::{records, sha256, stable_json},
    diagnostic::DiagnosticResult,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn digest(operation: &Value, issuer: &str) -> String {
    sha256(stable_json(&json!({"operation":operation,"issuerKeyId":issuer})).as_bytes())
}

pub(super) fn event_id(operation: &Operation, target: &crate::collaboration::Target) -> String {
    format!(
        "ev_{}",
        sha256(
            stable_json(&json!({
                "repoId":target.repo_id,"kind":target.kind,"id":target.id,"type":"execution",
                "subjectId":operation.actor.subject_id,"key":operation.operation_id,
            }))
            .as_bytes()
        )
    )
}

fn invalid() -> crate::diagnostic::Diagnostic {
    failure(
        "COORD_HISTORY_INVALID",
        "Canonical coordination history has an invalid publication or causal transition.",
    )
}

pub(super) fn replay(
    repo: &mut Repository,
    context: &Context,
    head: &str,
) -> DiagnosticResult<State> {
    let commits = repo.first_parent(head)?;
    let positions: BTreeMap<_, _> = commits
        .iter()
        .enumerate()
        .map(|(index, sha)| (sha.as_str(), index))
        .collect();
    let policies = context.ordered(&commits)?;
    let changes = repo.text(
        &[
            "log",
            "--first-parent",
            "--full-history",
            "--reverse",
            "--format=%H",
            head,
            "--",
            "gov/collaboration",
        ],
        None,
    )?;
    let mut state = State::default();
    let mut accepted_paths = BTreeSet::new();
    let mut count = 0;
    for publication in changes.lines().filter(|line| !line.is_empty()) {
        let parents = repo.text(&["rev-list", "--parents", "-n", "1", publication], None)?;
        let parents: Vec<_> = parents.split_whitespace().skip(1).collect();
        let args = if let Some(parent) = parents.first() {
            vec![
                "diff-tree",
                "--no-commit-id",
                "--no-renames",
                "--name-status",
                "-z",
                "-r",
                parent,
                publication,
            ]
        } else {
            vec![
                "diff-tree",
                "--root",
                "--no-commit-id",
                "--no-renames",
                "--name-status",
                "-z",
                "-r",
                publication,
            ]
        };
        let diff = repo.run(&args, None)?;
        if !diff.success {
            return Err(invalid());
        }
        let fields: Vec<_> = diff
            .stdout
            .split(|b| *b == 0)
            .filter(|field| !field.is_empty())
            .collect();
        if fields.len() % 2 != 0 {
            return Err(invalid());
        }
        for pair in fields.as_chunks::<2>().0 {
            let status = std::str::from_utf8(pair[0]).map_err(|_| invalid())?;
            let path = std::str::from_utf8(pair[1]).map_err(|_| invalid())?;
            if accepted_paths.contains(path) {
                return Err(invalid());
            }
            if !path.starts_with("gov/collaboration/") || status == "D" {
                continue;
            }
            let bytes = repo.blob(publication, path)?;
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            if value["payload"]["protocol"] != PROTOCOL {
                if value["payload"]["protocol"]
                    .as_str()
                    .is_some_and(|p| p.starts_with("govctl-coordination-"))
                {
                    return Err(failure(
                        "COORD_UNSUPPORTED",
                        "Canonical history requires an unsupported coordination protocol.",
                    ));
                }
                continue;
            }
            count += 1;
            if count > 10_000 {
                return Err(failure(
                    "COORD_UNSUPPORTED",
                    "Coordination history exceeds the v1 replay limit.",
                ));
            }
            if status != "A" || fields.len() != 2 || parents.len() != 1 {
                return Err(invalid());
            }
            let parent = parents[0];
            let (event, key) = records::validate(&bytes).map_err(|_| invalid())?;
            if key.is_none() || event["formatVersion"] != 2 || event["type"] != "execution" {
                return Err(invalid());
            }
            let payload = &event["payload"];
            if payload.as_object().is_none_or(|p| p.len() != 5)
                || payload["parentCommit"] != parent
                || payload["authority"]["id"] != context.authority_id
            {
                return Err(invalid());
            }
            let position = positions.get(parent).ok_or_else(invalid)?;
            let (_, revision, policy) = policies
                .iter()
                .rev()
                .find(|p| p.0 <= *position)
                .ok_or_else(|| {
                    failure(
                        "COORD_POLICY_INVALID",
                        "Historical operation has no applicable external policy.",
                    )
                })?;
            if payload["authority"]["revision"] != *revision {
                return Err(invalid());
            }
            let issuer = event["signature"]["keyId"].as_str().ok_or_else(invalid)?;
            policy
                .trust(
                    issuer,
                    event["signature"]["publicKey"]
                        .as_str()
                        .ok_or_else(invalid)?,
                )
                .map_err(|_| invalid())?;
            let raw_operation = &payload["operation"];
            let operation = Operation::parse(raw_operation).map_err(|_| invalid())?;
            let semantic = digest(raw_operation, issuer);
            if payload["semanticDigest"] != semantic
                || event["actor"]
                    != serde_json::to_value(&operation.actor).map_err(|_| invalid())?
                || state.operations.contains_key(&operation.operation_id)
            {
                return Err(invalid());
            }
            let target = state.target(&operation).map_err(|_| invalid())?;
            let id = event_id(&operation, &target);
            if event["target"] != serde_json::to_value(&target).map_err(|_| invalid())?
                || event["eventId"] != id
                || path != record_path(&target, &id)
                || target.repo_id != context.repo_id
            {
                return Err(invalid());
            }
            let sources = repo.targets(parent, &context.repo_id)?;
            // Historical ref availability is the trusted issuer's signed assertion;
            // deleting a feature branch later must not erase an accepted claim.
            let effective = state
                .apply(&operation, policy, issuer, &sources, &id)
                .map_err(|_| invalid())?;
            state.operations.insert(
                operation.operation_id,
                Accepted {
                    request_id: operation.request_id,
                    semantic_digest: semantic,
                    event_id: id,
                    publication_commit: publication.to_owned(),
                    effective,
                },
            );
            accepted_paths.insert(path.to_owned());
        }
    }
    let sources = repo.targets(head, &context.repo_id)?;
    state.mark_stale(&sources);
    Ok(state)
}
