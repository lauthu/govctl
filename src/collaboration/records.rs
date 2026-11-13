//! Envelope validation and append-only publication ([[RFC-0011:C-STORAGE]]).
use super::{Target, hex, invalid, now, sha256, source, stable_json, validate_id};
use crate::{
    config::Config,
    diagnostic::{DiagnosticResult, Diagnostics},
    model::Actor,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{
    Signature, Signer, SigningKey, VerifyingKey,
    pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePublicKey, spki::der::pem::LineEnding},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(super) const EVENT_LIMIT: u64 = 2 * 1024 * 1024;
const MAX_RECORDS: usize = 100_000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct History {
    pub event_id: String,
    pub event: Option<Value>,
    pub verification: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub provenance: &'static str,
}
fn malformed() -> crate::diagnostic::Diagnostic {
    invalid(
        "COLLAB_INVALID_EVENT",
        "The collaboration envelope is malformed or inconsistent.",
    )
}
fn text<'a>(value: &'a Value, field: &str, max: usize) -> DiagnosticResult<&'a str> {
    let result = value[field].as_str().ok_or_else(malformed)?;
    if result.is_empty() || result.len() > max || result.contains('\0') {
        return Err(malformed());
    }
    Ok(result)
}
fn actor(value: &Value) -> DiagnosticResult<Actor> {
    let actor: Actor = serde_json::from_value(value.clone()).map_err(|_| malformed())?;
    actor.validate().map_err(|_| malformed())?;
    Ok(actor)
}
fn event_id(id: &str) -> bool {
    id.strip_prefix("ev_").is_some_and(|value| hex(value, 64))
}
pub(super) fn target(value: &Value) -> DiagnosticResult<Target> {
    let fields = ["repoId", "kind", "id", "version", "contentDigest", "commit"];
    if value.as_object().is_none_or(|object| {
        object.len() != fields.len() || fields.iter().any(|key| !object.contains_key(*key))
    }) {
        return Err(malformed());
    }
    let target: Target = serde_json::from_value(value.clone()).map_err(|_| malformed())?;
    validate_id(&target.id)?;
    super::validate_repo_id(&target.repo_id)?;
    if super::kind_of(&target.id)? != target.kind
        || !hex(&target.content_digest, 64)
        || (target.kind != "rfc" && target.version.is_some())
        || target
            .version
            .as_ref()
            .is_some_and(|v| v.is_empty() || v.len() > 100 || v.chars().any(char::is_control))
        || target
            .commit
            .as_ref()
            .is_some_and(|c| !hex(c, 40) && !hex(c, 64))
    {
        return Err(malformed());
    }
    Ok(target)
}
fn payload(event: &Value) -> DiagnosticResult<()> {
    let value = &event["payload"];
    if !value.is_object() {
        return Err(malformed());
    }
    match event["type"].as_str().ok_or_else(malformed)? {
        "review" => {
            if !matches!(value["scope"].as_str(), Some("artifact" | "code"))
                || !matches!(
                    value["verdict"].as_str(),
                    Some("approved" | "changes_requested" | "comment")
                )
            {
                return Err(malformed());
            }
            text(value, "reason", 65536)?;
            if value["scope"] == "code" && event["target"]["commit"].is_null() {
                return Err(malformed());
            }
            if value["scope"] == "artifact" && !event["target"]["commit"].is_null() {
                return Err(malformed());
            }
        }
        "comment" => {
            if event["formatVersion"] != 2 {
                return Err(malformed());
            }
            text(value, "body", 65536)?;
            if let Some(reply) = value.get("replyTo")
                && !reply.is_null()
                && !reply.as_str().is_some_and(event_id)
            {
                return Err(malformed());
            }
        }
        "authorship" => {
            let authors = value["authors"].as_array().ok_or_else(malformed)?;
            if authors.is_empty() || authors.len() > 1000 {
                return Err(malformed());
            }
            for author in authors {
                actor(author)?;
            }
            if value.get("automatic").is_some_and(|v| !v.is_boolean()) {
                return Err(malformed());
            }
        }
        "resolution" => {
            if !event_id(text(value, "reviewId", 67)?) {
                return Err(malformed());
            }
            text(value, "reason", 65536)?;
        }
        "execution" | "legacy-import" | "review-assignment" | "review-run" => {}
        _ => return Err(malformed()),
    }
    Ok(())
}

/// Integrity is checked even when the key is not in the caller's trust store.
pub(super) fn validate(bytes: &[u8]) -> DiagnosticResult<(Value, Option<VerifyingKey>)> {
    if bytes.len() as u64 > EVENT_LIMIT {
        return Err(invalid(
            "COLLAB_INVALID_EVENT",
            "The event exceeds the 2 MiB limit.",
        ));
    }
    let event: Value = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    let object = event.as_object().ok_or_else(malformed)?;
    let fields = [
        "formatVersion",
        "eventId",
        "type",
        "createdAt",
        "actor",
        "target",
        "requestDigest",
        "payload",
        "signature",
    ];
    if object.len() != fields.len()
        || fields.iter().any(|key| !object.contains_key(*key))
        || !matches!(event["formatVersion"].as_u64(), Some(1 | 2))
        || !event_id(text(&event, "eventId", 67)?)
        || !event["createdAt"]
            .as_u64()
            .is_some_and(|n| n <= 9_007_199_254_740_991)
        || !hex(text(&event, "requestDigest", 64)?, 64)
    {
        return Err(malformed());
    }
    let actor = actor(&event["actor"])?;
    target(&event["target"])?;
    payload(&event)?;
    let digest = sha256(stable_json(&json!({"type":event["type"],"subjectId":actor.subject_id,"target":event["target"],"payload":event["payload"]})).as_bytes());
    if event["requestDigest"] != digest {
        return Err(invalid(
            "COLLAB_INVALID_EVENT",
            "The event request digest does not match its content.",
        ));
    }
    if event["signature"].is_null() {
        if event["formatVersion"] != 2 {
            return Err(malformed());
        }
        return Ok((event, None));
    }
    let sig = &event["signature"];
    if sig.as_object().is_none_or(|o| o.len() != 4) || sig["algorithm"] != "Ed25519" {
        return Err(malformed());
    }
    let pem = text(sig, "publicKey", 16384)?;
    if text(sig, "keyId", 64)? != sha256(pem.as_bytes()) {
        return Err(invalid(
            "COLLAB_INVALID_EVENT",
            "The signature key identity is inconsistent.",
        ));
    }
    let key = VerifyingKey::from_public_key_pem(pem)
        .map_err(|_| invalid("COLLAB_INVALID_EVENT", "The event public key is invalid."))?;
    let signature = STANDARD
        .decode(text(sig, "value", 256)?)
        .ok()
        .and_then(|v| Signature::from_slice(&v).ok())
        .ok_or_else(malformed)?;
    let mut body = event.clone();
    body.as_object_mut()
        .ok_or_else(malformed)?
        .remove("signature");
    key.verify_strict(stable_json(&body).as_bytes(), &signature)
        .map_err(|_| invalid("COLLAB_INVALID_EVENT", "The event signature is invalid."))?;
    Ok((event, Some(key)))
}
pub(super) fn trusted_keys(paths: &[PathBuf]) -> DiagnosticResult<Vec<VerifyingKey>> {
    if paths.len() > 100 {
        return Err(invalid("COLLAB_INVALID_EVENT", "Too many trusted keys."));
    }
    paths
        .iter()
        .map(|path| {
            let bytes = source::read_bounded(path, 16384)?;
            let pem = std::str::from_utf8(&bytes).map_err(|_| {
                invalid(
                    "COLLAB_UNTRUSTED_KEY",
                    "Trusted public key must be Ed25519 SPKI PEM.",
                )
            })?;
            VerifyingKey::from_public_key_pem(pem).map_err(|_| {
                invalid(
                    "COLLAB_UNTRUSTED_KEY",
                    "Trusted public key must be Ed25519 SPKI PEM.",
                )
            })
        })
        .collect()
}
fn directory(target: &Target) -> String {
    format!("collaboration/{}/{}", target.kind, target.id)
}
fn record_path(
    config: &Config,
    target: &Target,
    id: &str,
    create: bool,
) -> DiagnosticResult<PathBuf> {
    if !event_id(id) {
        return Err(malformed());
    }
    source::safe_path(
        &config.gov_root,
        &format!("{}/{}.json", directory(target), id),
        create,
    )
}
fn identity(event: &Value, current: &Target) -> DiagnosticResult<()> {
    let value = target(&event["target"])?;
    if value.id != current.id || value.kind != current.kind || value.repo_id != current.repo_id {
        return Err(invalid(
            "COLLAB_INVALID_TARGET",
            "Record identity does not match the requested repository artifact.",
        ));
    }
    Ok(())
}
pub(super) fn history(
    config: &Config,
    target: &Target,
    keys: &[VerifyingKey],
) -> DiagnosticResult<Vec<History>> {
    let path = source::safe_path(&config.gov_root, &directory(target), false)?;
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(_) => {
            return Err(invalid(
                "COLLAB_UNSAFE_PATH",
                "The collaboration directory cannot be read.",
            ));
        }
    };
    let mut history = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| {
            invalid(
                "COLLAB_UNSAFE_PATH",
                "A collaboration directory entry is unavailable.",
            )
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Interrupted atomic publications are never interpreted as records.
        if name.starts_with(".append-") {
            continue;
        }
        let id = name.strip_suffix(".json").unwrap_or(&name).to_owned();
        let result = (|| {
            let path = record_path(config, target, &id, false)?;
            let (event, key) = validate(&source::read_bounded(&path, EVENT_LIMIT)?)?;
            identity(&event, target)?;
            if event["eventId"] != id {
                return Err(malformed());
            }
            let verified = key.is_some_and(|key| keys.contains(&key));
            let declared = event["signature"].is_null();
            Ok(History {
                event_id: id.clone(),
                event: Some(event),
                verification: if verified { "verified" } else { "unverified" },
                error: None,
                provenance: if verified {
                    "verified"
                } else if declared {
                    "declared"
                } else {
                    "unknown-key"
                },
            })
        })();
        history.push(match result {
            Ok(record) => record,
            Err(error) => History {
                event_id: id,
                event: None,
                verification: "unverified",
                error: Some(error.message),
                provenance: "unknown-key",
            },
        });
        if history.len() > MAX_RECORDS {
            return Err(invalid(
                "COLLAB_RECORD_LIMIT",
                "Too many collaboration records.",
            ));
        }
    }
    history.sort_by(|a, b| {
        a.event
            .as_ref()
            .and_then(|e| e["createdAt"].as_u64())
            .cmp(&b.event.as_ref().and_then(|e| e["createdAt"].as_u64()))
            .then(a.event_id.cmp(&b.event_id))
    });
    Ok(history)
}
fn equivalent(a: &Value, b: &Value) -> bool {
    [
        "formatVersion",
        "eventId",
        "type",
        "actor",
        "target",
        "requestDigest",
        "payload",
    ]
    .iter()
    .all(|key| a[*key] == b[*key])
        && a["signature"]["keyId"] == b["signature"]["keyId"]
}
fn references(event: &Value, history: &[History]) -> DiagnosticResult<()> {
    let id = if event["type"] == "resolution" {
        event["payload"]["reviewId"].as_str()
    } else if event["type"] == "comment" {
        event["payload"]["replyTo"].as_str()
    } else {
        None
    };
    if let Some(id) = id
        && !history.iter().any(|r| {
            r.event.as_ref().is_some_and(|v| {
                v["eventId"] == id
                    && v["target"]["repoId"] == event["target"]["repoId"]
                    && v["target"]["kind"] == event["target"]["kind"]
                    && v["target"]["id"] == event["target"]["id"]
                    && matches!(v["type"].as_str(), Some("review" | "comment"))
            })
        })
    {
        return Err(invalid(
            "COLLAB_INVALID_EVENT",
            "A reply or resolution must reference an existing review or comment on this artifact.",
        ));
    }
    Ok(())
}
pub(super) fn append(
    config: &Config,
    current: &Target,
    bytes: &[u8],
    allow_stale: bool,
    preview: bool,
) -> DiagnosticResult<Value> {
    let (event, _) = validate(bytes)?;
    identity(&event, current)?;
    let id = text(&event, "eventId", 67)?;
    let path = record_path(config, current, id, false)?;
    if path
        .try_exists()
        .map_err(|_| invalid("COLLAB_UNSAFE_PATH", "Cannot inspect the existing record."))?
    {
        let (prior, _) = validate(&source::read_bounded(&path, EVENT_LIMIT)?)?;
        if !equivalent(&prior, &event) {
            return Err(invalid(
                "COLLAB_IDEMPOTENCY_CONFLICT",
                "The idempotency key already identifies different content or a different issuer.",
            ));
        }
        return Ok(json!({"event":prior,"replayed":true}));
    }
    let event_target = target(&event["target"])?;
    let validate_current = || -> DiagnosticResult<()> {
        let mut expected = super::current_target(config, &current.id, &current.repo_id)?;
        if event_target.commit.is_some() {
            expected.commit = super::git(config, &["rev-parse", "HEAD"]);
        }
        if !allow_stale && event_target != expected {
            return Err(invalid(
                "COLLAB_SOURCE_CHANGED",
                "The event target does not match the current source revision.",
            ));
        }
        Ok(())
    };
    validate_current()?;
    references(&event, &history(config, current, &[])?)?;
    if preview {
        return Ok(json!({"event":event,"replayed":false,"preview":true}));
    }
    let path = record_path(config, current, id, true)?;
    let parent = path.parent().ok_or_else(|| {
        invalid(
            "COLLAB_UNSAFE_PATH",
            "Record parent directory is unavailable.",
        )
    })?;
    let temp = parent.join(format!(".append-{}-{}-{}", std::process::id(), now(), id));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| {
                invalid(
                    "COLLAB_WRITE_FAILED",
                    "Cannot stage a collaboration record.",
                )
            })?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| {
                invalid(
                    "COLLAB_WRITE_FAILED",
                    "Cannot flush the collaboration record.",
                )
            })?;
        validate_current()?;
        // A hard link publishes a complete file without ever replacing an existing record.
        match fs::hard_link(&temp, &path) {
            Ok(()) => {
                if let Ok(dir) = fs::File::open(parent) {
                    let _ = dir.sync_all();
                }
                Ok(json!({"event":event,"replayed":false}))
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let (prior, _) = validate(&source::read_bounded(&path, EVENT_LIMIT)?)?;
                if equivalent(&prior, &event) {
                    Ok(json!({"event":prior,"replayed":true}))
                } else {
                    Err(invalid(
                        "COLLAB_IDEMPOTENCY_CONFLICT",
                        "A concurrent append already used this event identity.",
                    ))
                }
            }
            Err(_) => Err(invalid(
                "COLLAB_WRITE_FAILED",
                "Cannot publish the collaboration record.",
            )),
        }
    })();
    let _ = fs::remove_file(temp);
    result
}
pub(super) fn create(
    target: &Target,
    kind: &str,
    actor: &Actor,
    payload: Value,
    key: &str,
    sign_key: Option<&Path>,
) -> DiagnosticResult<Value> {
    if key.trim().is_empty() || key.len() > 1024 || key.chars().any(char::is_control) {
        return Err(invalid(
            "COLLAB_INVALID_EVENT",
            "The idempotency key must be bounded nonempty text.",
        ));
    }
    let id=sha256(stable_json(&json!({"repoId":target.repo_id,"kind":target.kind,"id":target.id,"type":kind,"subjectId":actor.subject_id,"key":key})).as_bytes());
    let digest = sha256(
        stable_json(
            &json!({"type":kind,"subjectId":actor.subject_id,"target":target,"payload":payload}),
        )
        .as_bytes(),
    );
    let mut event = json!({"formatVersion":2,"eventId":format!("ev_{id}"),"type":kind,"createdAt":now(),"actor":actor,"target":target,"requestDigest":digest,"payload":payload});
    let signature = if let Some(path) = sign_key {
        let bytes = source::read_bounded(path, 16384)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if fs::metadata(path)
                .map_err(|_| invalid("COLLAB_INVALID_KEY", "Cannot inspect private signing key."))?
                .permissions()
                .mode()
                & 0o077
                != 0
            {
                return Err(invalid(
                    "COLLAB_INVALID_KEY",
                    "Private signing key must not be accessible by group or other users.",
                ));
            }
        }
        let pem = std::str::from_utf8(&bytes).map_err(|_| {
            invalid(
                "COLLAB_INVALID_KEY",
                "Private signing key must be Ed25519 PKCS#8 PEM.",
            )
        })?;
        let key = SigningKey::from_pkcs8_pem(pem).map_err(|_| {
            invalid(
                "COLLAB_INVALID_KEY",
                "Private signing key must be Ed25519 PKCS#8 PEM.",
            )
        })?;
        let public = key
            .verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .map_err(|_| {
                invalid(
                    "COLLAB_INVALID_KEY",
                    "Cannot encode the signing public key.",
                )
            })?;
        json!({"algorithm":"Ed25519","keyId":sha256(public.as_bytes()),"publicKey":public,"value":STANDARD.encode(key.sign(stable_json(&event).as_bytes()).to_bytes())})
    } else {
        Value::Null
    };
    event["signature"] = signature;
    validate(&serde_json::to_vec(&event).map_err(|_| malformed())?)?;
    Ok(event)
}

pub(super) fn check(config: &Config) -> Diagnostics {
    let mut diagnostics = Vec::new();
    // Projects without attached records retain their existing storage policy,
    // including explicitly configured/symlinked governance roots. Do not make
    // the collaboration boundary a new restriction on unrelated artifacts.
    if fs::symlink_metadata(config.gov_root.join("collaboration"))
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return diagnostics;
    }
    let root = match source::safe_path(&config.gov_root, "collaboration", false) {
        Ok(path) => path,
        Err(error) => return vec![error],
    };
    if !root.exists() {
        return diagnostics;
    }
    let scan = (|| -> DiagnosticResult<()> {
        let sources = source::sources(config, "validation")?;
        for kind in fs::read_dir(root).map_err(|_| malformed())? {
            let kind = kind.map_err(|_| malformed())?;
            let name = kind.file_name().to_string_lossy().into_owned();
            if !matches!(name.as_str(), "rfc" | "adr" | "work") {
                diagnostics.push(malformed());
                continue;
            }
            let kind_path =
                source::safe_path(&config.gov_root, &format!("collaboration/{name}"), false)?;
            for artifact in fs::read_dir(kind_path).map_err(|_| malformed())? {
                let artifact = artifact.map_err(|_| malformed())?;
                let id = artifact.file_name().to_string_lossy().into_owned();
                validate_id(&id)?;
                let path = source::safe_path(
                    &config.gov_root,
                    &format!("collaboration/{name}/{id}"),
                    false,
                )?;
                if !sources
                    .iter()
                    .any(|source| source.target.id == id && source.target.kind == name)
                {
                    diagnostics.push(invalid(
                        "COLLAB_NOT_FOUND",
                        "Collaboration refers to a missing artifact.",
                    ));
                }
                let mut loaded = Vec::new();
                for entry in fs::read_dir(path).map_err(|_| malformed())? {
                    let entry = entry.map_err(|_| malformed())?;
                    let filename = entry.file_name().to_string_lossy().into_owned();
                    if filename.starts_with(".append-") {
                        continue;
                    }
                    let record = (|| {
                        let bytes = source::read_bounded(&entry.path(), EVENT_LIMIT)?;
                        let (event, _) = validate(&bytes)?;
                        if filename != format!("{}.json", text(&event, "eventId", 67)?)
                            || event["target"]["id"] != id
                            || event["target"]["kind"] != name
                        {
                            return Err(malformed());
                        }
                        Ok(History {
                            event_id: text(&event, "eventId", 67)?.into(),
                            event: Some(event),
                            verification: "unverified",
                            error: None,
                            provenance: "unknown-key",
                        })
                    })();
                    match record {
                        Ok(record) => loaded.push(record),
                        Err(error) => diagnostics.push(error),
                    };
                    if loaded.len() > MAX_RECORDS {
                        return Err(invalid(
                            "COLLAB_RECORD_LIMIT",
                            "Too many collaboration records.",
                        ));
                    }
                }
                for record in &loaded {
                    if let Some(event) = &record.event
                        && let Err(error) = references(event, &loaded)
                    {
                        diagnostics.push(error);
                    }
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = scan {
        diagnostics.push(error);
    }
    diagnostics
}
