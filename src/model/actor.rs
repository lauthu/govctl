//! Explicit attribution identities per [[RFC-0011:C-ACTOR]].

use crate::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticResult};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;

const MAX_ACTOR_BYTES: u64 = 64 * 1024;

/// A declared identity; roles and names are claims, never authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Actor {
    pub subject_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ActorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorKind {
    Human,
    Agent,
}

impl Actor {
    pub fn validate(&self) -> DiagnosticResult<()> {
        for (label, value) in [("subjectId", &self.subject_id), ("name", &self.name)] {
            validate_text(label, value)?;
        }
        for (label, value) in [
            ("agentId", &self.agent_id),
            ("participantId", &self.participant_id),
            ("role", &self.role),
        ] {
            if let Some(value) = value {
                validate_text(label, value)?;
            }
        }
        if self.roles.len() > 32 {
            return Err(invalid_actor("Actor roles must contain at most 32 entries"));
        }
        for role in &self.roles {
            validate_text("roles entry", role)?;
        }
        Ok(())
    }
}

fn validate_text(label: &str, value: &str) -> DiagnosticResult<()> {
    if value.trim().is_empty() || value.chars().count() > 300 || value.chars().any(char::is_control)
    {
        return Err(invalid_actor(format!(
            "Actor {label} must contain 1-300 characters without control characters"
        )));
    }
    Ok(())
}

fn invalid_actor(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(DiagnosticCode::E0820InvalidFieldValue, message, "actor")
}

/// Read bounded explicit context before any artifact mutation.
pub fn read_actor_file(path: &Path) -> DiagnosticResult<Actor> {
    let io_error = |error: std::io::Error| {
        Diagnostic::new(
            DiagnosticCode::E0901IoError,
            format!("Cannot read actor file: {error}"),
            path.display().to_string(),
        )
    };
    let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > MAX_ACTOR_BYTES {
        return Err(invalid_actor(
            "Actor file must be a regular file of at most 64 KiB",
        ));
    }
    let file = std::fs::File::open(path).map_err(io_error)?;
    let opened = file.metadata().map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.ino() != opened.ino() || metadata.dev() != opened.dev() {
            return Err(invalid_actor("Actor file changed while opening it"));
        }
    }
    if !opened.is_file() || opened.len() > MAX_ACTOR_BYTES {
        return Err(invalid_actor(
            "Actor file must be a regular file of at most 64 KiB",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_ACTOR_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > MAX_ACTOR_BYTES {
        return Err(invalid_actor("Actor file exceeds 64 KiB"));
    }
    let actor: Actor = serde_json::from_slice(&bytes)
        .map_err(|_| invalid_actor("Actor file must contain a valid JSON actor object"))?;
    actor.validate()?;
    Ok(actor)
}
