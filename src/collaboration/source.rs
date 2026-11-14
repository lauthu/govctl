//! Exact source manifests, compatible with protocol v1 ([[RFC-0011:C-TARGET]]).
use super::{Target, invalid, sha256};
use crate::config::Config;
use crate::diagnostic::DiagnosticResult;
use serde::Serialize;
use serde_json::Value;
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const FILE_LIMIT: u64 = 4 * 1024 * 1024;
#[derive(Clone)]
pub(super) struct Source {
    pub target: Target,
    pub created_by: Option<Value>,
}
#[derive(Serialize)]
struct ManifestEntry {
    path: String,
    sha256: String,
}

pub(super) fn safe_path(
    root: &Path,
    relative: &str,
    make_parents: bool,
) -> DiagnosticResult<PathBuf> {
    if relative.is_empty()
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || relative.contains(['\\', '\0'])
        || Path::new(relative).is_absolute()
        || !Path::new(relative)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(invalid(
            "COLLAB_UNSAFE_PATH",
            "Repository paths must be bounded relative components.",
        ));
    }
    let root_meta = fs::symlink_metadata(root)
        .map_err(|_| invalid("COLLAB_UNSAFE_PATH", "The governed root is unavailable."))?;
    if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
        return Err(invalid(
            "COLLAB_UNSAFE_PATH",
            "The governed root must be a real directory.",
        ));
    }
    let canonical = fs::canonicalize(root)
        .map_err(|_| invalid("COLLAB_UNSAFE_PATH", "The governed root is unavailable."))?;
    let parts: Vec<_> = Path::new(relative).components().collect();
    let mut cursor = canonical;
    for (index, part) in parts.iter().enumerate() {
        cursor.push(part.as_os_str());
        match fs::symlink_metadata(&cursor) {
            Ok(meta)
                if meta.file_type().is_symlink() || (index + 1 < parts.len() && !meta.is_dir()) =>
            {
                return Err(invalid(
                    "COLLAB_UNSAFE_PATH",
                    "Symbolic links and non-directory parent paths are forbidden.",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if make_parents && index + 1 < parts.len() {
                    fs::create_dir(&cursor).map_err(|_| {
                        invalid(
                            "COLLAB_UNSAFE_PATH",
                            "Could not create a collaboration directory.",
                        )
                    })?;
                }
            }
            Err(_) => {
                return Err(invalid(
                    "COLLAB_UNSAFE_PATH",
                    "A repository path cannot be inspected.",
                ));
            }
        }
    }
    Ok(cursor)
}

pub(super) fn read_bounded(path: &Path, limit: u64) -> DiagnosticResult<Vec<u8>> {
    let before = fs::symlink_metadata(path).map_err(|_| {
        invalid(
            "COLLAB_NOT_FOUND",
            "A required source or input file is unavailable.",
        )
    })?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() > limit {
        return Err(invalid(
            "COLLAB_UNSAFE_PATH",
            "Input must be a bounded regular file, not a symbolic link.",
        ));
    }
    let file = fs::File::open(path).map_err(|_| {
        invalid(
            "COLLAB_NOT_FOUND",
            "A required source or input file cannot be opened.",
        )
    })?;
    let opened = file
        .metadata()
        .map_err(|_| invalid("COLLAB_UNSAFE_PATH", "Input metadata is unavailable."))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.ino() != opened.ino() || before.dev() != opened.dev() {
            return Err(invalid(
                "COLLAB_UNSAFE_PATH",
                "Input changed while opening it.",
            ));
        }
    }
    if !opened.is_file() || opened.len() > limit {
        return Err(invalid(
            "COLLAB_UNSAFE_PATH",
            "Input is not a bounded regular file.",
        ));
    }
    let mut bytes = Vec::new();
    (&file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("COLLAB_INVALID_EVENT", "Input could not be read."))?;
    let after = file
        .metadata()
        .map_err(|_| invalid("COLLAB_UNSAFE_PATH", "Input metadata is unavailable."))?;
    if opened.len() != after.len() || opened.modified().ok() != after.modified().ok() {
        return Err(invalid(
            "COLLAB_SOURCE_CHANGED",
            "Input changed while being read.",
        ));
    }
    if bytes.len() as u64 > limit {
        return Err(invalid(
            "COLLAB_INVALID_EVENT",
            "Input exceeds its byte limit.",
        ));
    }
    Ok(bytes)
}
fn children(config: &Config, relative: &str) -> DiagnosticResult<Vec<fs::DirEntry>> {
    let path = safe_path(&config.gov_root, relative, false)?;
    match fs::read_dir(path) {
        Ok(entries) => entries
            .map(|entry| {
                entry.map_err(|_| {
                    invalid(
                        "COLLAB_UNSAFE_PATH",
                        "A source directory entry is unavailable.",
                    )
                })
            })
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(_) => Err(invalid(
            "COLLAB_UNSAFE_PATH",
            "A source directory cannot be read.",
        )),
    }
}
fn parse(bytes: &[u8]) -> DiagnosticResult<toml::Value> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        invalid(
            "COLLAB_INVALID_SOURCE",
            "Governance sources must be UTF-8 TOML.",
        )
    })?;
    toml::from_str(text).map_err(|_| {
        invalid(
            "COLLAB_INVALID_SOURCE",
            "Governance sources must be valid TOML.",
        )
    })
}
fn source(config: &Config, kind: &str, relative: &str, repo: &str) -> DiagnosticResult<Source> {
    let bytes = read_bounded(&safe_path(&config.gov_root, relative, false)?, FILE_LIMIT)?;
    let parsed = parse(&bytes)?;
    let meta = parsed
        .get("govctl")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| invalid("COLLAB_INVALID_SOURCE", "Source has no govctl metadata."))?;
    let id = meta
        .get("id")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| invalid("COLLAB_INVALID_SOURCE", "Source has no artifact ID."))?;
    super::validate_id(id)?;
    if super::kind_of(id)? != kind {
        return Err(invalid(
            "COLLAB_INVALID_SOURCE",
            "Artifact ID does not match its directory kind.",
        ));
    }
    let mut manifest = vec![ManifestEntry {
        path: format!("gov/{relative}"),
        sha256: sha256(&bytes),
    }];
    if kind == "rfc" {
        if Path::new(relative)
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            != Some(id)
        {
            return Err(invalid(
                "COLLAB_INVALID_SOURCE",
                "RFC identity does not match its source directory.",
            ));
        }
        let sections = parsed
            .get("sections")
            .and_then(toml::Value::as_array)
            .ok_or_else(|| invalid("COLLAB_INVALID_SOURCE", "RFC sections are missing."))?;
        let dir = Path::new(relative)
            .parent()
            .ok_or_else(|| invalid("COLLAB_INVALID_SOURCE", "RFC directory is missing."))?;
        for section in sections {
            let clauses = match section.get("clauses") {
                Some(value) => value.as_array().ok_or_else(|| {
                    invalid("COLLAB_INVALID_SOURCE", "RFC clauses must be an array.")
                })?,
                None => continue,
            };
            for clause in clauses {
                let name = clause.as_str().ok_or_else(|| {
                    invalid("COLLAB_INVALID_SOURCE", "RFC clause path must be a string.")
                })?;
                // Validate before joining so an absolute clause cannot discard the source directory.
                if Path::new(name).is_absolute()
                    || name.contains(['\\', '\0'])
                    || !Path::new(name)
                        .components()
                        .all(|c| matches!(c, Component::Normal(_)))
                {
                    return Err(invalid(
                        "COLLAB_UNSAFE_PATH",
                        "RFC clause paths must remain inside the artifact directory.",
                    ));
                }
                let relative = dir.join(name).to_string_lossy().replace('\\', "/");
                let clause_bytes =
                    read_bounded(&safe_path(&config.gov_root, &relative, false)?, FILE_LIMIT)?;
                parse(&clause_bytes)?;
                let entry = ManifestEntry {
                    path: format!("gov/{relative}"),
                    sha256: sha256(&clause_bytes),
                };
                if !manifest.iter().any(|prior| prior.path == entry.path) {
                    manifest.push(entry);
                }
                if manifest.len() > 1000 {
                    return Err(invalid(
                        "COLLAB_SOURCE_LIMIT",
                        "RFC contains too many source files.",
                    ));
                }
            }
        }
    }
    manifest.sort_by(|a, b| a.path.encode_utf16().cmp(b.path.encode_utf16()));
    let bytes = serde_json::to_vec(&manifest).map_err(|_| {
        invalid(
            "COLLAB_INVALID_SOURCE",
            "Cannot encode the source manifest.",
        )
    })?;
    let created_by = meta
        .get("created_by")
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| invalid("COLLAB_INVALID_SOURCE", "Invalid creator metadata."))?;
    if let Some(value) = &created_by {
        let creator: crate::model::Actor = serde_json::from_value(value.clone()).map_err(|_| {
            invalid(
                "COLLAB_INVALID_SOURCE",
                "Creator metadata must be a valid actor.",
            )
        })?;
        creator.validate().map_err(|_| {
            invalid(
                "COLLAB_INVALID_SOURCE",
                "Creator metadata must be a valid actor.",
            )
        })?;
    }
    Ok(Source {
        target: Target {
            repo_id: repo.into(),
            kind: kind.into(),
            id: id.into(),
            version: if kind == "rfc" {
                meta.get("version")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
            } else {
                None
            },
            content_digest: sha256(&bytes),
            commit: None,
        },
        created_by,
    })
}

fn capture_sources(config: &Config, repo: &str) -> DiagnosticResult<Vec<Source>> {
    super::validate_repo_id(repo)?;
    let mut found = Vec::new();
    for kind in ["rfc", "adr", "work"] {
        for entry in children(config, kind)? {
            let meta = entry
                .file_type()
                .map_err(|_| invalid("COLLAB_UNSAFE_PATH", "Source metadata is unavailable."))?;
            if meta.is_symlink() {
                return Err(invalid(
                    "COLLAB_UNSAFE_PATH",
                    "Governance source directories cannot contain symbolic links.",
                ));
            }
            let name = entry.file_name().into_string().map_err(|_| {
                invalid(
                    "COLLAB_INVALID_SOURCE",
                    "Governance filenames must be UTF-8.",
                )
            })?;
            let relative = if kind == "rfc" && meta.is_dir() {
                format!("{kind}/{name}/rfc.toml")
            } else if kind != "rfc" && meta.is_file() && name.ends_with(".toml") {
                format!("{kind}/{name}")
            } else {
                continue;
            };
            let item = source(config, kind, &relative, repo)?;
            if found
                .iter()
                .any(|prior: &Source| prior.target.id == item.target.id)
            {
                return Err(invalid("COLLAB_INVALID_SOURCE", "Duplicate artifact ID."));
            }
            found.push(item);
            if found.len() > 50_000 {
                return Err(invalid(
                    "COLLAB_SOURCE_LIMIT",
                    "Project contains too many artifacts.",
                ));
            }
        }
    }
    found.sort_by(|a, b| a.target.id.cmp(&b.target.id));
    Ok(found)
}

pub(super) fn sources(config: &Config, repo: &str) -> DiagnosticResult<Vec<Source>> {
    // Govctl writers share a lock. A second complete capture also detects
    // ordinary editor/Git changes that do not participate in that lock.
    let first = capture_sources(config, repo)?;
    let second = capture_sources(config, repo)?;
    if first.len() != second.len() || first.iter().zip(&second).any(|(a, b)| a.target != b.target) {
        return Err(invalid(
            "COLLAB_SOURCE_CHANGED",
            "Governance sources changed during snapshot capture.",
        ));
    }
    Ok(second)
}
