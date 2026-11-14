//! Explicit, versioned external authorization — [[RFC-0012:C-POLICY]].
use super::{failure, git::read_external, model};
use crate::{collaboration::sha256, diagnostic::DiagnosticResult};
use ed25519_dalek::{
    VerifyingKey,
    pkcs8::{DecodePublicKey, EncodePublicKey, spki::der::pem::LineEnding},
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Context {
    pub protocol_version: u32,
    pub repo_id: String,
    pub remote: String,
    pub branch: String,
    pub authority_id: String,
    pub current_policy_revision: String,
    pub policies: BTreeMap<String, Policy>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Policy {
    pub effective_after_commit: String,
    pub trusted_issuers: Vec<Issuer>,
    pub grants: Vec<Grant>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Issuer {
    pub key_id: String,
    pub public_key_pem: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Grant {
    pub subject_id: String,
    pub issuer_key_id: String,
    pub actions: Vec<String>,
}

impl Context {
    pub fn load(path: &Path) -> DiagnosticResult<Self> {
        let bytes = read_external(path, 2 * 1024 * 1024).map_err(|_| {
            failure(
                "COORD_INVALID_CONTEXT",
                "Cannot read bounded external coordination context.",
            )
        })?;
        let context: Self = serde_json::from_slice(&bytes).map_err(|_| {
            failure(
                "COORD_INVALID_CONTEXT",
                "Invalid external coordination context.",
            )
        })?;
        if context.protocol_version != 1 || context.branch != "main" {
            return Err(failure(
                "COORD_UNSUPPORTED",
                "Only version 1 direct publication to main is supported.",
            ));
        }
        crate::collaboration::validate_repo_id(&context.repo_id)?;
        model::bounded(&context.authority_id, 300)?;
        model::bounded(&context.current_policy_revision, 300)?;
        if !valid_remote(&context.remote) {
            return Err(failure(
                "COORD_INVALID_CONTEXT",
                "Use an explicit absolute repository path or HTTPS/SSH remote.",
            ));
        }
        if context.policies.is_empty() || context.policies.len() > 100 {
            return Err(failure(
                "COORD_INVALID_CONTEXT",
                "A bounded policy history is required.",
            ));
        }
        let mut anchors = BTreeSet::new();
        for (revision, policy) in &context.policies {
            model::bounded(revision, 300)?;
            model::commit(&policy.effective_after_commit)?;
            if !anchors.insert(&policy.effective_after_commit) {
                return Err(failure(
                    "COORD_POLICY_INVALID",
                    "Policy activation anchors must be distinct.",
                ));
            }
            if policy.trusted_issuers.len() > 100 || policy.grants.len() > 10_000 {
                return Err(failure(
                    "COORD_POLICY_INVALID",
                    "Invalid policy issuer or grant count.",
                ));
            }
            let mut keys = BTreeSet::new();
            for issuer in &policy.trusted_issuers {
                let key = VerifyingKey::from_public_key_pem(&issuer.public_key_pem)
                    .map_err(|_| failure("COORD_POLICY_INVALID", "Invalid issuer public key."))?;
                let canonical = key.to_public_key_pem(LineEnding::LF).map_err(|_| {
                    failure("COORD_POLICY_INVALID", "Cannot encode issuer public key.")
                })?;
                if canonical != issuer.public_key_pem
                    || issuer.key_id != sha256(canonical.as_bytes())
                    || !keys.insert(&issuer.key_id)
                {
                    return Err(failure(
                        "COORD_POLICY_INVALID",
                        "Issuer identities must use unique canonical LF SPKI PEM hashes.",
                    ));
                }
            }
            for grant in &policy.grants {
                model::bounded(&grant.subject_id, 300)?;
                if !keys.contains(&grant.issuer_key_id)
                    || grant.actions.is_empty()
                    || grant.actions.iter().any(|action| {
                        action != "delegate" && !model::ACTIONS.contains(&action.as_str())
                    })
                {
                    return Err(failure(
                        "COORD_POLICY_INVALID",
                        "A grant names an unknown issuer or action.",
                    ));
                }
            }
        }
        if !context
            .policies
            .contains_key(&context.current_policy_revision)
        {
            return Err(failure(
                "COORD_POLICY_INVALID",
                "Current policy revision is missing from context.",
            ));
        }
        Ok(context)
    }

    pub fn ordered<'a>(
        &'a self,
        commits: &[String],
    ) -> DiagnosticResult<Vec<(usize, &'a str, &'a Policy)>> {
        let positions: BTreeMap<_, _> = commits
            .iter()
            .enumerate()
            .map(|(i, sha)| (sha.as_str(), i))
            .collect();
        let mut result = Vec::new();
        for (revision, policy) in &self.policies {
            let position = positions
                .get(policy.effective_after_commit.as_str())
                .ok_or_else(|| {
                    failure(
                        "COORD_POLICY_INVALID",
                        "A policy activation anchor is not on canonical main's first-parent chain.",
                    )
                })?;
            result.push((*position, revision.as_str(), policy));
        }
        result.sort_by_key(|entry| entry.0);
        if result
            .last()
            .is_none_or(|entry| entry.1 != self.current_policy_revision)
        {
            return Err(failure(
                "COORD_POLICY_INVALID",
                "Current policy is not the latest activated policy.",
            ));
        }
        Ok(result)
    }
}

impl Policy {
    pub fn permits(&self, issuer: &str, subject: &str, action: &str) -> bool {
        self.grants.iter().any(|grant| {
            grant.issuer_key_id == issuer
                && grant.subject_id == subject
                && grant.actions.iter().any(|value| value == action)
        })
    }

    pub fn authorize(&self, issuer: &str, subject: &str, action: &str) -> DiagnosticResult<()> {
        if self.permits(issuer, subject, action) {
            Ok(())
        } else {
            Err(failure(
                "COORD_UNAUTHORIZED",
                "The explicit policy does not authorize this subject, issuer and action.",
            ))
        }
    }

    pub fn trust(&self, key_id: &str, pem: &str) -> DiagnosticResult<()> {
        if self
            .trusted_issuers
            .iter()
            .any(|issuer| issuer.key_id == key_id && issuer.public_key_pem == pem)
        {
            Ok(())
        } else {
            Err(failure(
                "COORD_UNAUTHORIZED",
                "The operation issuer is not trusted by the applicable policy.",
            ))
        }
    }
}

fn valid_remote(remote: &str) -> bool {
    if remote.is_empty() || remote.len() > 4096 || remote.chars().any(char::is_control) {
        return false;
    }
    if Path::new(remote).is_absolute() {
        return true;
    }
    if let Some(rest) = remote
        .strip_prefix("https://")
        .or_else(|| remote.strip_prefix("ssh://"))
    {
        return !rest.is_empty()
            && !rest.chars().any(char::is_whitespace)
            && !rest.starts_with('/');
    }
    let Some((host, path)) = remote.split_once(':') else {
        return false;
    };
    host.contains('@')
        && !host.starts_with('-')
        && !host.contains('/')
        && !path.is_empty()
        && !remote.chars().any(char::is_whitespace)
        && !remote.contains("::")
}
