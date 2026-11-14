//! Causal requests and dual ownership — [[RFC-0012:C-EXCLUSION]].
use super::{failure, model::*, policy::Policy};
use crate::{collaboration::Target, diagnostic::DiagnosticResult};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default)]
pub(super) struct State {
    pub requests: BTreeMap<String, Request>,
    pub writes: BTreeMap<String, WriteClaim>,
    pub operations: BTreeMap<String, Accepted>,
    used_claims: BTreeSet<String>,
}

fn conflict() -> crate::diagnostic::Diagnostic {
    failure(
        "COORD_STATE_CONFLICT",
        "The request state, holder or generation has changed.",
    )
}
fn increment(value: u64) -> DiagnosticResult<u64> {
    value
        .checked_add(1)
        .ok_or_else(|| failure("COORD_STATE_CONFLICT", "Claim generation exhausted."))
}

pub(super) fn matches_source(target: &Target, sources: &BTreeMap<String, Target>) -> bool {
    sources.get(&target.id).is_some_and(|current| {
        let mut expected = current.clone();
        expected.commit = target.commit.clone();
        &expected == target
    })
}

pub(super) fn current_input(request: &Request, sources: &BTreeMap<String, Target>) -> bool {
    matches_source(&request.target, sources)
        && request
            .write_inputs
            .values()
            .all(|target| matches_source(target, sources))
}

impl State {
    pub fn target(&self, operation: &Operation) -> DiagnosticResult<Target> {
        if operation.action == "publish" {
            operation.target.clone().ok_or_else(conflict)
        } else {
            self.requests
                .get(&operation.request_id)
                .map(|r| r.target.clone())
                .ok_or_else(|| {
                    failure(
                        "COORD_NOT_FOUND",
                        "The request has not been published on canonical main.",
                    )
                })
        }
    }

    fn expected(&self, operation: &Operation, request: &Request) -> DiagnosticResult<()> {
        if operation.expected_state.as_deref() != Some(&request.state_event_id)
            || operation.expected_generation != Some(request.generation)
        {
            return Err(conflict());
        }
        let scopes: BTreeMap<_, _> = if request.holder.is_some() || operation.action == "claim" {
            request
                .write_targets
                .iter()
                .map(|id| {
                    (
                        id.clone(),
                        self.writes.get(id).map_or(0, |claim| claim.generation),
                    )
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        if operation.expected_writes.as_ref() != Some(&scopes) {
            return Err(failure(
                "COORD_WRITE_CONFLICT",
                "The operation must name all current write generations.",
            ));
        }
        if request.holder.is_some()
            && request.write_targets.iter().any(|id| {
                self.writes.get(id).is_none_or(|claim| {
                    claim.request_id.as_ref() != Some(&request.request_id)
                        || claim.holder != request.holder
                })
            })
        {
            return Err(failure(
                "COORD_WRITE_CONFLICT",
                "The request no longer holds its complete write scope.",
            ));
        }
        Ok(())
    }

    fn acting_for(
        operation: &Operation,
        holder: &Holder,
        policy: &Policy,
        issuer: &str,
    ) -> DiagnosticResult<()> {
        if operation.actor.subject_id != holder.subject_id {
            policy.authorize(issuer, &operation.actor.subject_id, "delegate")?;
            if operation.on_behalf_of.as_ref() != Some(&holder.subject_id) {
                return Err(failure(
                    "COORD_UNAUTHORIZED",
                    "Delegation must explicitly identify the actual execution holder.",
                ));
            }
        } else if operation
            .on_behalf_of
            .as_ref()
            .is_some_and(|subject| subject != &holder.subject_id)
        {
            return Err(failure(
                "COORD_UNAUTHORIZED",
                "Delegation names a different holder.",
            ));
        }
        Ok(())
    }

    fn holding(
        operation: &Operation,
        holder: Option<&Holder>,
        policy: &Policy,
        issuer: &str,
    ) -> DiagnosticResult<()> {
        let holder = holder.ok_or_else(conflict)?;
        Self::acting_for(operation, holder, policy, issuer)?;
        if operation.claim_id.as_ref() != Some(&holder.claim_id)
            || operation.session_id.as_ref() != Some(&holder.session_id)
        {
            return Err(conflict());
        }
        Ok(())
    }

    fn acquire(
        &mut self,
        request: &mut Request,
        operation: &Operation,
        policy: &Policy,
        issuer: &str,
    ) -> DiagnosticResult<()> {
        let holder = operation.holder.as_ref().ok_or_else(conflict)?;
        policy.authorize(issuer, &holder.subject_id, "claim")?;
        Self::acting_for(operation, holder, policy, issuer)?;
        if self.used_claims.contains(&holder.claim_id) {
            return Err(failure(
                "COORD_STATE_CONFLICT",
                "A claim ID cannot be reused for another execution attempt.",
            ));
        }
        for id in &request.write_targets {
            if self.writes.get(id).is_some_and(|write| {
                write
                    .request_id
                    .as_ref()
                    .is_some_and(|owner| owner != &request.request_id)
            }) {
                return Err(failure(
                    "COORD_WRITE_CONFLICT",
                    "Another request holds an overlapping artifact write scope.",
                ));
            }
        }
        request.generation = increment(request.generation)?;
        for id in &request.write_targets {
            let generation = increment(self.writes.get(id).map_or(0, |claim| claim.generation))?;
            self.writes.insert(
                id.clone(),
                WriteClaim {
                    artifact_id: id.clone(),
                    generation,
                    request_id: Some(request.request_id.clone()),
                    holder: Some(holder.clone()),
                },
            );
        }
        self.used_claims.insert(holder.claim_id.clone());
        request.holder = Some(holder.clone());
        request.handoff = None;
        request.status = "claimed".to_owned();
        Ok(())
    }

    fn release(&mut self, request: &mut Request) -> DiagnosticResult<()> {
        request.generation = increment(request.generation)?;
        if request.holder.is_some() {
            for id in &request.write_targets {
                let write = self.writes.get_mut(id).ok_or_else(conflict)?;
                write.generation = increment(write.generation)?;
                write.request_id = None;
                write.holder = None;
            }
        }
        request.holder = None;
        request.handoff = None;
        Ok(())
    }

    fn witness(&self, request: &mut Request) {
        let writes = if request.holder.is_some() {
            request
                .write_targets
                .iter()
                .filter_map(|id| {
                    self.writes
                        .get(id)
                        .map(|write| (id.clone(), write.generation))
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        request.witnesses.insert(
            request.state_event_id.clone(),
            Witness {
                generation: request.generation,
                holder: request.holder.clone(),
                writes,
            },
        );
    }

    /// A staged state is discarded if publication loses the remote compare-and-swap.
    pub fn apply(
        &mut self,
        operation: &Operation,
        policy: &Policy,
        issuer: &str,
        sources: &BTreeMap<String, Target>,
        event_id: &str,
    ) -> DiagnosticResult<bool> {
        policy.authorize(issuer, &operation.actor.subject_id, &operation.action)?;
        if operation.action == "publish" {
            if self.requests.contains_key(&operation.request_id) {
                return Err(failure(
                    "COORD_STATE_CONFLICT",
                    "Request identity is already published and immutable.",
                ));
            }
            let target = operation.target.clone().ok_or_else(conflict)?;
            if !matches_source(&target, sources) {
                return Err(failure(
                    "COORD_SOURCE_CHANGED",
                    "The request input is not the current canonical artifact revision.",
                ));
            }
            let write_targets = operation.write_targets.clone().ok_or_else(conflict)?;
            let mut write_inputs = BTreeMap::new();
            for id in &write_targets {
                let source = sources.get(id).ok_or_else(|| {
                    failure(
                        "COORD_NOT_FOUND",
                        "A write-scope artifact has not been published on main.",
                    )
                })?;
                write_inputs.insert(id.clone(), source.clone());
            }
            let mut request = Request {
                request_id: operation.request_id.clone(),
                target,
                requested_operation: operation.requested_operation.clone().ok_or_else(conflict)?,
                description: operation.description.clone().ok_or_else(conflict)?,
                write_targets,
                write_inputs,
                status: "open".to_owned(),
                state_event_id: event_id.to_owned(),
                generation: 0,
                holder: None,
                handoff: None,
                result: None,
                stale: false,
                witnesses: BTreeMap::new(),
            };
            self.witness(&mut request);
            self.requests.insert(operation.request_id.clone(), request);
            return Ok(true);
        }
        let mut request = self
            .requests
            .get(&operation.request_id)
            .cloned()
            .ok_or_else(|| {
                failure(
                    "COORD_NOT_FOUND",
                    "The request has not been published on main.",
                )
            })?;
        if operation.action == "complete" && operation.allow_obsolete {
            // [[RFC-0012:C-PROTOCOL]]: a real historical attempt, never invented old IDs.
            let witness = operation
                .expected_state
                .as_ref()
                .and_then(|id| request.witnesses.get(id))
                .ok_or_else(conflict)?;
            if operation.expected_generation != Some(witness.generation)
                || operation.expected_writes.as_ref() != Some(&witness.writes)
            {
                return Err(conflict());
            }
            Self::holding(operation, witness.holder.as_ref(), policy, issuer)?;
            return Ok(false);
        }
        if matches!(request.status.as_str(), "completed" | "cancelled") {
            return Err(failure(
                "COORD_TERMINAL",
                "A completed or cancelled request cannot be reopened or completed again.",
            ));
        }
        self.expected(operation, &request)?;
        if matches!(
            operation.action.as_str(),
            "claim" | "accept" | "takeover" | "complete"
        ) && !current_input(&request, sources)
        {
            return Err(failure(
                "COORD_SOURCE_CHANGED",
                "The immutable request input or write-scope source has changed.",
            ));
        }
        match operation.action.as_str() {
            "claim" => {
                if request.status != "open" {
                    return Err(conflict());
                }
                self.acquire(&mut request, operation, policy, issuer)?;
            }
            "handoff" => {
                Self::holding(operation, request.holder.as_ref(), policy, issuer)?;
                let subject = operation.to_subject_id.clone().ok_or_else(conflict)?;
                policy.authorize(issuer, &subject, "claim")?;
                request.handoff = Some(Handoff {
                    to_subject_id: subject,
                    checkpoint: operation.checkpoint.clone().ok_or_else(conflict)?,
                });
                request.status = "handoff".to_owned();
            }
            "accept" => {
                let offer = request.handoff.as_ref().ok_or_else(conflict)?;
                if request.status != "handoff"
                    || operation
                        .holder
                        .as_ref()
                        .is_none_or(|h| h.subject_id != offer.to_subject_id)
                {
                    return Err(conflict());
                }
                self.acquire(&mut request, operation, policy, issuer)?;
            }
            "release" => {
                Self::holding(operation, request.holder.as_ref(), policy, issuer)?;
                self.release(&mut request)?;
                request.status = "open".to_owned();
            }
            "cancel" => {
                self.release(&mut request)?;
                request.status = "cancelled".to_owned();
            }
            "takeover" => {
                if request.holder.is_none() {
                    return Err(conflict());
                }
                self.acquire(&mut request, operation, policy, issuer)?;
            }
            "complete" => {
                Self::holding(operation, request.holder.as_ref(), policy, issuer)?;
                self.release(&mut request)?;
                request.status = "completed".to_owned();
                request.result = operation.result.clone();
            }
            _ => return Err(conflict()),
        }
        request.state_event_id = event_id.to_owned();
        self.witness(&mut request);
        self.requests.insert(operation.request_id.clone(), request);
        Ok(true)
    }

    pub fn mark_stale(&mut self, sources: &BTreeMap<String, Target>) {
        for request in self.requests.values_mut() {
            request.stale = !current_input(request, sources);
        }
    }
}
