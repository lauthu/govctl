use std::path::{Path, PathBuf};

pub fn snapshot_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

pub fn current_test_snapshot_name(prefix: &str, function_name: &str) -> String {
    let test_name = function_name.rsplit("::").next().unwrap_or(function_name);
    let snapshot_case = test_name.strip_prefix("test_").unwrap_or(test_name);
    format!("{prefix}__{snapshot_case}")
}

pub fn named_snapshot_name(prefix: &str, name: &str) -> String {
    format!("{prefix}__{name}")
}

pub fn normalized_display_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
            return format!("//{path}").replace('\\', "/");
        }
        if let Some(path) = path.strip_prefix(r"\\?\")
            && path.as_bytes().get(1) == Some(&b':')
        {
            return path.replace('\\', "/");
        }
        path.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path.into_owned()
    }
}

pub fn normalized_toml_path(
    record: &str,
    field: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let record = toml::from_str::<toml::Value>(record)?;
    let path = record
        .get(field)
        .and_then(toml::Value::as_str)
        .ok_or("record path missing")?;
    Ok(normalized_display_path(Path::new(path)))
}

/// Normalize output for stable snapshots:
/// - Replace temp directory paths with `<TEMPDIR>`
/// - Replace today's date with `<DATE>`
/// - Replace work item IDs (WI-YYYY-MM-DD-NNN) with WI-<DATE>-NNN
/// - Replace ADR IDs with date component normalized
pub fn normalize_output(output: &str, dir: &Path, date: &str) -> Result<String, regex::Error> {
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let canonical_str = canonical.display().to_string();
    let dir_str = dir.display().to_string();
    let mut normalized = output.replace(&canonical_str, "<TEMPDIR>");
    normalized = normalized.replace(&dir_str, "<TEMPDIR>");
    #[cfg(windows)]
    {
        normalized = normalized.replace(&canonical_str.replace('\\', "/"), "<TEMPDIR>");
        normalized = normalized.replace(&dir_str.replace('\\', "/"), "<TEMPDIR>");
    }
    normalized = normalized.replace(date, "<DATE>");

    // Replace work item IDs
    let wi_pattern = regex::Regex::new(r"WI-\d{4}-\d{2}-\d{2}-(\d{3})")?;
    normalized = wi_pattern
        .replace_all(&normalized, "WI-<DATE>-$1")
        .to_string();

    // Replace ADR filenames with dates
    let adr_file_pattern = regex::Regex::new(r"ADR-(\d{4})-")?;
    normalized = adr_file_pattern
        .replace_all(&normalized, "ADR-XXXX-")
        .to_string();

    // Replace signature hashes (date-dependent due to embedded dates in specs)
    let sig_pattern = regex::Regex::new(r"sha256:[0-9a-f]{64}")?;
    normalized = sig_pattern
        .replace_all(&normalized, "sha256:<HASH>")
        .to_string();

    // Replace govctl version in JSON contexts to avoid snapshot churn on version bumps.
    // Only replace inside double quotes to avoid corrupting semver strings in CHANGELOG fixtures.
    let version = env!("CARGO_PKG_VERSION");
    normalized = normalized.replace(&format!("\"{version}\""), "\"<VERSION>\"");

    Ok(normalized)
}
