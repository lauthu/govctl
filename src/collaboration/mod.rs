//! Portable, append-only collaboration attached to RFC/ADR/work items.
//! Implements [[RFC-0011:C-SCOPE]], [[RFC-0011:C-CLI]].
mod projection;
mod records;
mod source;

use crate::cli::{CollabCommand, OutputFormat};
use crate::config::Config;
use crate::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticResult, Diagnostics};
use crate::model::Actor;
use crate::write::WriteOp;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Target {
    pub repo_id: String,
    pub kind: String,
    pub id: String,
    pub version: Option<String>,
    pub content_digest: String,
    pub commit: Option<String>,
}

pub(super) fn invalid(code: &str, message: &str) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::E0820InvalidFieldValue,
        format!("{code}: {message}"),
        "collaboration",
    )
}
pub(super) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(super) fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn validate_id(id: &str) -> DiagnosticResult<()> {
    if id.is_empty()
        || id.len() > 160
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(invalid(
            "COLLAB_INVALID_TARGET",
            "Invalid artifact identity.",
        ));
    }
    Ok(())
}
pub(crate) fn validate_repo_id(repo: &str) -> DiagnosticResult<()> {
    if repo.trim().is_empty() || repo.len() > 300 || repo.chars().any(char::is_control) {
        return Err(invalid(
            "COLLAB_INVALID_TARGET",
            "Provide a bounded repository identity.",
        ));
    }
    Ok(())
}
pub(super) fn kind_of(id: &str) -> DiagnosticResult<&'static str> {
    validate_id(id)?;
    if id.starts_with("RFC-") {
        Ok("rfc")
    } else if id.starts_with("ADR-") {
        Ok("adr")
    } else if id.starts_with("WI-") {
        Ok("work")
    } else {
        Err(invalid(
            "COLLAB_INVALID_TARGET",
            "An RFC, ADR or work-item ID is required.",
        ))
    }
}
pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// JavaScript stableJson compatibility: keys sort by UTF-16, strings retain CRLF.
pub(super) fn stable_json(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        Value::String(key.clone()),
                        stable_json(&object[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(array) => format!(
            "[{}]",
            array.iter().map(stable_json).collect::<Vec<_>>().join(",")
        ),
        Value::Number(number) => {
            let mut buffer = ryu_js::Buffer::new();
            number
                .as_f64()
                .map(|number| buffer.format(number).to_owned())
                .unwrap_or_else(|| number.to_string())
        }
        _ => value.to_string(),
    }
}
pub(crate) fn current_target(config: &Config, id: &str, repo_id: &str) -> DiagnosticResult<Target> {
    validate_id(id)?;
    source::sources(config, repo_id)?
        .into_iter()
        .find(|source| source.target.id == id)
        .map(|source| source.target)
        .ok_or_else(|| invalid("COLLAB_NOT_FOUND", "The artifact does not exist."))
}
pub(crate) fn source_digest(config: &Config, id: &str) -> DiagnosticResult<String> {
    Ok(current_target(config, id, "local")?.content_digest)
}
pub(super) fn git(config: &Config, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(config.project_root())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}
fn inferred_repo(config: &Config) -> String {
    if let Some(remote) = git(config, &["remote", "get-url", "origin"]) {
        for marker in ["github.com/", "github.com:"] {
            if let Some((_, repo)) = remote.split_once(marker) {
                return repo
                    .trim_end_matches('/')
                    .trim_end_matches(".git")
                    .to_owned();
            }
        }
    }
    format!(
        "local:{}",
        config
            .project_root()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    )
}

/// Called under the existing source writer lock; never reacquire that lock.
pub(crate) fn append_authorship(
    config: &Config,
    id: &str,
    actor: &Actor,
    created: bool,
    repo_id: Option<&str>,
    op: WriteOp,
) -> DiagnosticResult<()> {
    if op.is_preview() {
        return Ok(());
    }
    actor.validate()?;
    let repo = repo_id
        .map(str::to_owned)
        .unwrap_or_else(|| inferred_repo(config));
    let target = current_target(config, id, &repo)?;
    let mut authors = std::collections::BTreeMap::<String, Value>::new();
    // Carry prior contributors as declarations, never inherit old approval trust.
    for entry in records::history(config, &target, &[])? {
        if let Some(event) = entry.event
            && event["type"] == "authorship"
        {
            for author in event["payload"]["authors"].as_array().into_iter().flatten() {
                if let Some(subject) = author["subjectId"].as_str() {
                    authors.insert(subject.to_owned(), author.clone());
                }
            }
        }
    }
    authors.insert(actor.subject_id.clone(), json!(actor));
    let payload = json!({"authors":authors.into_values().collect::<Vec<_>>(), "automatic":true, "created":created});
    let key = format!(
        "native-author:{}:{}:{}",
        now(),
        std::process::id(),
        target.content_digest
    );
    let event = records::create(&target, "authorship", actor, payload, &key, None)?;
    let bytes = serde_json::to_vec_pretty(&event)
        .map_err(|_| invalid("COLLAB_INVALID_EVENT", "Cannot encode the event."))?;
    records::append(config, &target, &bytes, false, false)?;
    Ok(())
}

fn print(value: &Value, output: OutputFormat) -> DiagnosticResult<()> {
    match output {
        OutputFormat::Json => println!("{value}"),
        _ => println!(
            "{}",
            serde_json::to_string_pretty(value)
                .map_err(|_| invalid("COLLAB_INVALID_EVENT", "Cannot encode the result."))?
        ),
    }
    Ok(())
}
pub(crate) fn execute(
    config: &Config,
    command: CollabCommand,
    op: WriteOp,
) -> DiagnosticResult<()> {
    match command {
        CollabCommand::Capabilities { output } => print(
            &json!({"protocolVersion":1,"features":["authorship","comments","reviews","legacy-v1","batch-read","signed-append"]}),
            output,
        ),
        CollabCommand::Target(args) => print(
            &json!(current_target(config, &args.id, &args.repo_id)?),
            args.output,
        ),
        CollabCommand::List(args) | CollabCommand::Show(args) => {
            let keys = records::trusted_keys(&args.trust_key_file)?;
            let sources = source::sources(config, &args.repo_id)?;
            let sources: Vec<_> = if let Some(id) = args.id {
                validate_id(&id)?;
                vec![
                    sources
                        .into_iter()
                        .find(|s| s.target.id == id)
                        .ok_or_else(|| {
                            invalid("COLLAB_NOT_FOUND", "The artifact does not exist.")
                        })?,
                ]
            } else {
                sources
            };
            let mut views = Vec::new();
            let head = git(config, &["rev-parse", "HEAD"]);
            // [[RFC-0011:C-TARGET]]: attached records cannot invalidate the
            // revision they review. Keep every other tracked/untracked change.
            let clean = git(
                config,
                &[
                    "status",
                    "--porcelain=v1",
                    "--untracked-files=all",
                    "--",
                    ".",
                    ":(exclude)gov/collaboration/**",
                ],
            )
            .is_some_and(|s| s.is_empty());
            for source in sources {
                let history = records::history(config, &source.target, &keys)?;
                views.push(projection::view(source, history, head.as_deref(), clean));
            }
            if args.output != OutputFormat::Json {
                for view in &views {
                    projection::render(view);
                }
                Ok(())
            } else {
                print(
                    &if args.all {
                        json!({"artifacts":views})
                    } else {
                        views.remove(0)
                    },
                    args.output,
                )
            }
        }
        CollabCommand::Append(args) => {
            let target = current_target(config, &args.target.id, &args.target.repo_id)?;
            let mut bytes = Vec::new();
            std::io::stdin()
                .take(records::EVENT_LIMIT + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| invalid("COLLAB_INVALID_EVENT", "Cannot read the event envelope."))?;
            let receipt =
                records::append(config, &target, &bytes, args.allow_stale, op.is_preview())?;
            print(&receipt, args.target.output)
        }
        CollabCommand::Add(args) => {
            let current = current_target(config, &args.target.id, &args.target.repo_id)?;
            let target = match args.target_file {
                Some(path) => serde_json::from_slice(&source::read_bounded(&path, 16384)?)
                    .map_err(|_| invalid("COLLAB_INVALID_TARGET", "Target file is invalid."))?,
                None => current.clone(),
            };
            let actor = crate::model::read_actor_file(&args.actor_file)?;
            let payload = serde_json::from_slice(&source::read_bounded(
                &args.payload_file,
                records::EVENT_LIMIT,
            )?)
            .map_err(|_| {
                invalid(
                    "COLLAB_INVALID_EVENT",
                    "Payload file must be a JSON object.",
                )
            })?;
            let event = records::create(
                &target,
                &args.record_type,
                &actor,
                payload,
                &args.idempotency_key,
                args.sign_key_file.as_deref(),
            )?;
            let receipt = records::append(
                config,
                &current,
                &serde_json::to_vec_pretty(&event)
                    .map_err(|_| invalid("COLLAB_INVALID_EVENT", "Cannot encode the event."))?,
                args.allow_stale,
                op.is_preview(),
            )?;
            print(&receipt, args.target.output)
        }
    }
}

pub(crate) fn check(config: &Config) -> Diagnostics {
    records::check(config)
}

#[cfg(test)]
mod tests {
    use super::stable_json;
    use serde_json::json;

    #[test]
    fn canonical_json_matches_ecmascript_numbers_strings_and_utf16_keys() {
        let value = json!({"numbers":[1.0, -0.0, 1e-7, 1e-6, 1e20, 1e21], "text":"first\r\nsecond", "\u{e000}":1,"\u{10000}":2});
        assert_eq!(
            stable_json(&value),
            "{\"numbers\":[1,0,1e-7,0.000001,100000000000000000000,1e+21],\"text\":\"first\\r\\nsecond\",\"𐀀\":2,\"\u{e000}\":1}"
        );
    }
}
