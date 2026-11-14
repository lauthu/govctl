//! Artifact creation helpers for the `new` command.

mod adr;
mod clause;
mod rfc;
mod work;

use crate::NewTarget;
use crate::config::Config;
use crate::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticResult, Diagnostics};
use crate::schema::{ArtifactSchema, validate_toml_value, with_schema_header};
use crate::write::{WriteOp, write_file};
use serde::Serialize;
use std::path::Path;

pub(super) fn write_new_artifact_toml<T: Serialize>(
    config: &Config,
    path: &Path,
    value: &T,
    schema: ArtifactSchema,
    schema_error: DiagnosticCode,
    label: &str,
    op: WriteOp,
) -> DiagnosticResult<()> {
    let display_path = config.display_path(path);
    let body = toml::to_string_pretty(value).map_err(|err| {
        Diagnostic::new(
            schema_error,
            format!("Failed to serialize {label} TOML: {err}"),
            display_path.display().to_string(),
        )
    })?;
    let raw = toml::from_str(&body).map_err(|err| {
        Diagnostic::new(
            schema_error,
            format!("Failed to normalize {label} TOML: {err}"),
            display_path.display().to_string(),
        )
    })?;
    validate_toml_value(schema, config, path, &raw)?;
    let content = with_schema_header(schema, &body);
    write_file(path, &content, op, Some(&display_path))
}

/// Create a new artifact.
pub fn create(config: &Config, target: &NewTarget, op: WriteOp) -> DiagnosticResult<Diagnostics> {
    let (actor_file, repo_id) = match target {
        NewTarget::Rfc {
            actor_file,
            repo_id,
            ..
        }
        | NewTarget::Adr {
            actor_file,
            repo_id,
            ..
        }
        | NewTarget::Work {
            actor_file,
            repo_id,
            ..
        } => (actor_file.as_deref(), repo_id.as_deref()),
        NewTarget::Clause { .. } => (None, None),
    };
    if let Some(repo_id) = repo_id {
        crate::collaboration::validate_repo_id(repo_id)?;
    }
    let actor = actor_file.map(crate::model::read_actor_file).transpose()?;
    match target {
        NewTarget::Rfc {
            title, id, repo_id, ..
        } => rfc::create(
            config,
            title,
            id.as_deref(),
            actor.as_ref(),
            repo_id.as_deref(),
            op,
        ),
        NewTarget::Clause {
            clause_id,
            title,
            section,
            kind,
        } => clause::create(config, clause_id, title, section, *kind, op),
        NewTarget::Adr { title, repo_id, .. } => {
            adr::create(config, title, actor.as_ref(), repo_id.as_deref(), op)
        }
        NewTarget::Work {
            title,
            active,
            repo_id,
            ..
        } => work::create(
            config,
            title,
            *active,
            actor.as_ref(),
            repo_id.as_deref(),
            op,
        ),
    }
}
