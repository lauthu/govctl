//! Wire and projected control state — [[RFC-0012:C-PROTOCOL]].
use super::{failure, invalid_input};
use crate::{collaboration::Target, diagnostic::DiagnosticResult, model::Actor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) const PROTOCOL: &str = "govctl-coordination-v1";
pub(super) const ACTIONS: [&str; 8] = [
    "publish", "claim", "handoff", "accept", "release", "cancel", "takeover", "complete",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Holder {
    pub subject_id: String,
    pub claim_id: String,
    pub session_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Checkpoint {
    pub summary: String,
    pub commit: Option<String>,
    pub remaining: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Outcome {
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    pub commit: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
}

/// Keep raw JSON alongside this parsed view: every supplied semantic field is signed.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Operation {
    pub operation_id: String,
    pub action: String,
    pub actor: Actor,
    pub request_id: String,
    pub target: Option<Target>,
    pub requested_operation: Option<String>,
    pub write_targets: Option<Vec<String>>,
    pub description: Option<String>,
    pub expected_state: Option<String>,
    pub expected_generation: Option<u64>,
    pub expected_writes: Option<BTreeMap<String, u64>>,
    pub holder: Option<Holder>,
    pub claim_id: Option<String>,
    pub session_id: Option<String>,
    pub to_subject_id: Option<String>,
    pub checkpoint: Option<Checkpoint>,
    pub reason: Option<String>,
    pub result: Option<Outcome>,
    #[serde(default)]
    pub allow_obsolete: bool,
    pub on_behalf_of: Option<String>,
}

pub(super) fn bounded(value: &str, max: usize) -> DiagnosticResult<()> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        Err(invalid_input(
            "An identity must be bounded, nonempty printable text.",
        ))
    } else {
        Ok(())
    }
}

fn prose(value: &str) -> DiagnosticResult<()> {
    if value.trim().is_empty() || value.len() > 65_536 || value.contains('\0') {
        Err(invalid_input(
            "A description, reason or result must be bounded nonempty text.",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn commit(value: &str) -> DiagnosticResult<()> {
    if crate::collaboration::hex(value, 40) || crate::collaboration::hex(value, 64) {
        Ok(())
    } else {
        Err(invalid_input(
            "An immutable 40- or 64-digit commit ID is required.",
        ))
    }
}

impl Operation {
    pub fn parse(value: &Value) -> DiagnosticResult<Self> {
        let operation: Self = serde_json::from_value(value.clone()).map_err(|_| {
            invalid_input("The operation does not match the coordination wire contract.")
        })?;
        bounded(&operation.operation_id, 300)?;
        bounded(&operation.request_id, 300)?;
        operation.actor.validate()?;
        if !ACTIONS.contains(&operation.action.as_str()) {
            return Err(failure(
                "COORD_UNSUPPORTED",
                "Unsupported coordination action.",
            ));
        }
        let mut allowed = vec!["operationId", "action", "actor", "requestId"];
        let required: &[&str] = match operation.action.as_str() {
            "publish" => &[
                "target",
                "requestedOperation",
                "writeTargets",
                "description",
            ],
            "claim" | "accept" => &["holder"],
            "handoff" => &["claimId", "sessionId", "toSubjectId", "checkpoint"],
            "release" => &["claimId", "sessionId"],
            "cancel" => &["reason"],
            "takeover" => &["holder", "reason"],
            "complete" => &["claimId", "sessionId", "result"],
            _ => &[],
        };
        for field in required {
            if value.get(*field).is_none_or(Value::is_null) {
                return Err(invalid_input(
                    "The operation is missing a required action field.",
                ));
            }
        }
        allowed.extend(required);
        if operation.action != "publish" {
            for field in ["expectedState", "expectedGeneration", "expectedWrites"] {
                if value.get(field).is_none_or(Value::is_null) {
                    return Err(invalid_input(
                        "Causal state and generation preconditions are required.",
                    ));
                }
                allowed.push(field);
            }
            if operation.action != "cancel" {
                allowed.push("onBehalfOf");
            }
        }
        if operation.action == "complete" {
            allowed.push("allowObsolete");
        }
        if value
            .as_object()
            .is_none_or(|object| object.keys().any(|key| !allowed.contains(&key.as_str())))
        {
            return Err(invalid_input(
                "The operation contains fields not supported by this action.",
            ));
        }
        if operation.target.is_some() {
            crate::collaboration::records::target(&value["target"])?;
        }
        if let Some(kind) = &operation.requested_operation {
            if !["review", "draft", "edit", "implement"].contains(&kind.as_str()) {
                return Err(failure(
                    "COORD_UNSUPPORTED",
                    "Unsupported requested operation.",
                ));
            }
            let writes = operation.write_targets.as_deref().unwrap_or_default();
            if writes.len() > 100 || (kind == "review") != writes.is_empty() {
                return Err(invalid_input(
                    "Review is read-only; other requests need a bounded write scope.",
                ));
            }
            let unique: std::collections::BTreeSet<_> = writes.iter().collect();
            if unique.len() != writes.len()
                || (kind != "review"
                    && operation
                        .target
                        .as_ref()
                        .is_none_or(|t| !writes.contains(&t.id)))
            {
                return Err(invalid_input(
                    "Write scopes must be unique and include the attached artifact.",
                ));
            }
            for id in writes {
                crate::collaboration::kind_of(id)?;
            }
        }
        for value in [&operation.description, &operation.reason]
            .into_iter()
            .flatten()
        {
            prose(value)?;
        }
        for value in [
            &operation.expected_state,
            &operation.claim_id,
            &operation.session_id,
            &operation.to_subject_id,
            &operation.on_behalf_of,
        ]
        .into_iter()
        .flatten()
        {
            bounded(value, 300)?;
        }
        if let Some(holder) = &operation.holder {
            bounded(&holder.subject_id, 300)?;
            bounded(&holder.claim_id, 300)?;
            bounded(&holder.session_id, 300)?;
        }
        if let Some(checkpoint) = &operation.checkpoint {
            prose(&checkpoint.summary)?;
            prose(&checkpoint.remaining)?;
            if let Some(sha) = &checkpoint.commit {
                commit(sha)?;
            }
        }
        if let Some(result) = &operation.result {
            prose(&result.summary)?;
            if result.verdict.as_ref().is_some_and(|v| {
                !["approved", "changes_requested", "comment"].contains(&v.as_str())
            }) {
                return Err(invalid_input("Invalid result verdict."));
            }
            if let Some(sha) = &result.commit {
                commit(sha)?;
            }
            if result.references.len() > 100 {
                return Err(invalid_input("Too many result references."));
            }
            for reference in &result.references {
                bounded(reference, 4096)?;
            }
        }
        if let Some(writes) = &operation.expected_writes {
            if writes.len() > 100 {
                return Err(invalid_input("Too many write preconditions."));
            }
            for id in writes.keys() {
                crate::collaboration::kind_of(id)?;
            }
        }
        Ok(operation)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Handoff {
    pub to_subject_id: String,
    pub checkpoint: Checkpoint,
}

#[derive(Clone)]
pub(super) struct Witness {
    pub generation: u64,
    pub holder: Option<Holder>,
    pub writes: BTreeMap<String, u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Request {
    pub request_id: String,
    pub target: Target,
    pub requested_operation: String,
    pub description: String,
    pub write_targets: Vec<String>,
    pub status: String,
    pub state_event_id: String,
    pub generation: u64,
    pub holder: Option<Holder>,
    pub handoff: Option<Handoff>,
    pub result: Option<Outcome>,
    pub stale: bool,
    #[serde(skip)]
    pub write_inputs: BTreeMap<String, Target>,
    #[serde(skip)]
    pub witnesses: BTreeMap<String, Witness>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WriteClaim {
    pub artifact_id: String,
    pub generation: u64,
    pub request_id: Option<String>,
    pub holder: Option<Holder>,
}

#[derive(Clone)]
pub(super) struct Accepted {
    pub request_id: String,
    pub semantic_digest: String,
    pub event_id: String,
    pub publication_commit: String,
    pub effective: bool,
}
