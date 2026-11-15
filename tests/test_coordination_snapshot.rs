//! Canonical input package acceptance for [[RFC-0012:C-SNAPSHOT]].
mod common;
mod coordination_support;
use coordination_support::*;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn snapshot(f: &Fixture, dir: &Path, request: &str) -> std::process::Output {
    f.command(dir, "snapshot")
        .args(["--request-id", request])
        .output()
        .unwrap()
}

fn native(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_govctl"))
        .args(args)
        .current_dir(dir)
        .env("GOVCTL_DEFAULT_OWNER", "@test")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

#[test]
fn snapshot_capability_is_additive_and_does_not_need_a_repository() {
    let root = tempfile::tempdir().unwrap();
    let capabilities = success(native(root.path(), &["coord", "capabilities"]));
    assert_eq!(capabilities["protocolVersion"], 1);
    let values = capabilities["capabilities"].as_array().unwrap();
    assert!(values.contains(&json!("repository-main-coordination-v1")));
    assert!(values.contains(&json!("request-input-snapshot-v1")));
    assert_eq!(capabilities["sourceMutation"], false);
}

fn clause(f: &Fixture, text: &str) {
    let path = f.seed.path().join("gov/rfc/RFC-0001/rfc.toml");
    let rfc = fs::read_to_string(&path).unwrap();
    fs::write(
        path,
        rfc.replace(
            "title = \"Specification\"",
            "title = \"Specification\"\nclauses = [\"clauses/C-HISTORY.toml\"]",
        ),
    )
    .unwrap();
    fs::write(
        f.seed.path().join("gov/rfc/RFC-0001/clauses/C-HISTORY.toml"),
        format!(
            "[govctl]\nid = \"C-HISTORY\"\ntitle = \"Historical requirement\"\nkind = \"normative\"\nstatus = \"deprecated\"\n[content]\ntext = {}\n",
            serde_json::to_string(text).unwrap()
        ),
    )
    .unwrap();
}

fn author_record(f: &Fixture, id: &str, signed: bool, target: Option<Value>) -> Value {
    let actor_path = f.root.path().join("author.json");
    let payload_path = f.root.path().join("authorship.json");
    fs::write(
        &actor_path,
        serde_json::to_vec(&actor("agent:writer")).unwrap(),
    )
    .unwrap();
    fs::write(
        &payload_path,
        serde_json::to_vec(&json!({"authors":[actor("agent:writer")],"automatic":true})).unwrap(),
    )
    .unwrap();
    let mut args = vec![
        "collab",
        "add",
        "RFC-0001",
        "--repo-id",
        REPO,
        "--type",
        "authorship",
        "--actor-file",
        actor_path.to_str().unwrap(),
        "--payload-file",
        payload_path.to_str().unwrap(),
        "--idempotency-key",
        id,
    ];
    if signed {
        args.extend(["--sign-key-file", f.key.to_str().unwrap()]);
    }
    let target_path = f.root.path().join("target.json");
    if let Some(target) = target {
        fs::write(&target_path, serde_json::to_vec(&target).unwrap()).unwrap();
        args.extend([
            "--target-file",
            target_path.to_str().unwrap(),
            "--allow-stale",
        ]);
    }
    success(native(f.seed.path(), &args))
}

#[test]
fn canonical_package_contains_archival_clauses_and_ignores_dirty_checkout() {
    let f = Fixture::new();
    clause(&f, "The historical requirement MUST remain visible.");
    f.push_seed("Add referenced historical clause");
    let request = f.publish("review", "RFC-0001", "review", &[]);
    let claimed = f.apply(&f.a, &Fixture::claim_operation("run", "agent:a", &request));
    let head = f.head();
    let local_head = git(&f.a, &["rev-parse", "HEAD"]);
    fs::write(
        f.a.join("gov/rfc/RFC-0001/rfc.toml"),
        "dirty invalid local TOML",
    )
    .unwrap();
    git(&f.a, &["add", "gov/rfc/RFC-0001/rfc.toml"]);
    let before_index = fs::read(f.a.join(".git/index")).unwrap();
    let package = success(snapshot(&f, &f.a, "review"));
    assert_eq!(package["head"], head);
    assert_eq!(package["request"], claimed["request"]);
    assert_eq!(package["target"], request["target"]);
    assert!(
        package["body"]
            .as_str()
            .unwrap()
            .contains("MUST remain visible")
    );
    assert_eq!(
        package["content"]["clauseSources"][0]["status"],
        "deprecated"
    );
    assert_eq!(
        package["content"]["clauseSources"][0]["clause_id"],
        "C-HISTORY"
    );
    assert_eq!(f.head(), head);
    assert_eq!(git(&f.a, &["rev-parse", "HEAD"]), local_head);
    assert_eq!(fs::read(f.a.join(".git/index")).unwrap(), before_index);
    assert_eq!(
        fs::read_to_string(f.a.join("gov/rfc/RFC-0001/rfc.toml")).unwrap(),
        "dirty invalid local TOML"
    );
    assert_eq!(success(snapshot(&f, f.root.path(), "review")), package);
}

#[test]
fn native_adr_and_work_content_are_preserved() {
    let f = Fixture::new();
    let result = native(f.seed.path(), &["adr", "new", "Canonical decision"]);
    assert!(result.status.success());
    common::write_guarded_work_item(f.seed.path(), "WI-2026-01-01-001", "GUARD-TEST", None)
        .unwrap();
    f.push_seed("Add ADR and work source");
    for (kind, id) in [("adr", "ADR-0001"), ("work", "WI-2026-01-01-001")] {
        f.publish(id, id, "review", &[]);
        let package = success(snapshot(&f, f.root.path(), id));
        assert_eq!(
            package["content"],
            success(native(f.seed.path(), &[kind, "show", id, "-o", "json"]))
        );
        assert!(package["body"].as_str().unwrap().contains(id));
        assert!(package["content"].get("clauseSources").is_none());
    }
}

#[test]
fn rfc_package_follows_section_order_and_excludes_unbound_orphan_clauses() {
    let f = Fixture::new();
    let dir = f.seed.path().join("gov/rfc/RFC-0001");
    let path = dir.join("rfc.toml");
    let rfc = fs::read_to_string(&path).unwrap().replace(
        "title = \"Specification\"",
        "title = \"Specification\"\nclauses = [\"clauses/C-Z.toml\", \"clauses/C-A.toml\"]",
    );
    fs::write(path, rfc).unwrap();
    for name in ["C-Z", "C-A", "C-ORPHAN"] {
        fs::write(
            dir.join(format!("clauses/{name}.toml")),
            format!("[govctl]\nid = \"{name}\"\ntitle = \"{name}\"\nkind = \"normative\"\n[content]\ntext = \"{name} bound requirement\""),
        ).unwrap();
    }
    f.push_seed("Add clauses in reference order and an orphan");
    f.publish("review", "RFC-0001", "review", &[]);
    let package = success(snapshot(&f, &f.a, "review"));
    let clauses = package["content"]["clauseSources"].as_array().unwrap();
    assert_eq!(clauses.len(), 2);
    assert_eq!(clauses[0]["clause_id"], "C-Z");
    assert_eq!(clauses[1]["clause_id"], "C-A");
    assert!(!package["body"].as_str().unwrap().contains("C-ORPHAN"));
    f.sync_seed();
    fs::write(dir.join("clauses/C-ORPHAN.toml"), "[govctl]\nid = \"C-ORPHAN\"\ntitle = \"Unbound edit\"\nkind = \"normative\"\n[content]\ntext = \"Changed unbound text\"").unwrap();
    f.push_seed("Edit only an unbound orphan");
    let after = success(snapshot(&f, &f.a, "review"));
    assert_eq!(package["target"], after["target"]);
    assert_eq!(package["body"], after["body"]);
    assert_eq!(package["content"], after["content"]);
}

#[test]
fn unknown_request_and_changed_or_removed_source_fail_explicitly() {
    let f = Fixture::new();
    rejected(snapshot(&f, &f.a, "missing"), "COORD_NOT_FOUND");
    f.publish("review", "RFC-0001", "review", &[]);
    f.sync_seed();
    clause(&f, "New input");
    f.push_seed("Change fixed source");
    rejected(snapshot(&f, &f.a, "review"), "COORD_SOURCE_CHANGED");
    fs::remove_dir_all(f.seed.path().join("gov/rfc/RFC-0001")).unwrap();
    f.push_seed("Remove fixed source");
    rejected(snapshot(&f, &f.a, "review"), "COORD_SOURCE_CHANGED");
}

#[test]
fn identical_clause_basenames_render_the_exact_referenced_paths() {
    let f = Fixture::new();
    let dir = f.seed.path().join("gov/rfc/RFC-0001");
    let path = dir.join("rfc.toml");
    let rfc = fs::read_to_string(&path).unwrap().replace(
        "title = \"Specification\"",
        "title = \"Specification\"\nclauses = [\"clauses/first/C-SAME.toml\", \"clauses/second/C-SAME.toml\"]",
    );
    fs::write(path, rfc).unwrap();
    for (directory, id) in [("first", "C-FIRST"), ("second", "C-SECOND")] {
        let clauses = dir.join("clauses").join(directory);
        fs::create_dir_all(&clauses).unwrap();
        fs::write(clauses.join("C-SAME.toml"), format!("[govctl]\nid = \"{id}\"\ntitle = \"{id}\"\nkind = \"normative\"\n[content]\ntext = \"Exact {directory} clause body\"")).unwrap();
    }
    f.push_seed("Add distinct referenced clauses with shared basename");
    f.publish("review", "RFC-0001", "review", &[]);
    let package = success(snapshot(&f, &f.a, "review"));
    let body = package["body"].as_str().unwrap();
    assert_eq!(body.matches("Exact first clause body").count(), 1);
    assert_eq!(body.matches("Exact second clause body").count(), 1);
    assert_eq!(
        package["content"]["clauseSources"][0]["clause_id"],
        "C-FIRST"
    );
    assert_eq!(
        package["content"]["clauseSources"][1]["clause_id"],
        "C-SECOND"
    );
    let native_body = native(
        f.seed.path(),
        &["rfc", "show", "RFC-0001", "-o", "plain", "--history"],
    );
    assert!(native_body.status.success());
    let native_body = String::from_utf8(native_body.stdout).unwrap();
    assert_eq!(native_body.matches("Exact first clause body").count(), 1);
    assert_eq!(native_body.matches("Exact second clause body").count(), 1);
}

#[test]
fn another_fixed_write_input_becoming_stale_rejects_package() {
    let f = Fixture::new();
    f.publish("edit", "RFC-0001", "edit", &["RFC-0001", "RFC-0002"]);
    assert!(snapshot(&f, &f.a, "edit").status.success());
    f.sync_seed();
    let path = f.seed.path().join("gov/rfc/RFC-0002/rfc.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        path,
        text.replace("Second artifact", "Changed second artifact"),
    )
    .unwrap();
    f.push_seed("Change another captured write input");
    rejected(snapshot(&f, &f.a, "edit"), "COORD_SOURCE_CHANGED");
}

#[test]
fn missing_referenced_clause_is_a_changed_source_error() {
    let f = Fixture::new();
    clause(&f, "Fixed referenced input");
    f.push_seed("Add referenced clause");
    f.publish("review", "RFC-0001", "review", &[]);
    f.sync_seed();
    fs::remove_file(
        f.seed
            .path()
            .join("gov/rfc/RFC-0001/clauses/C-HISTORY.toml"),
    )
    .unwrap();
    f.push_seed("Remove the fixed referenced clause");
    let output = snapshot(&f, &f.a, "review");
    assert!(output.stdout.is_empty());
    rejected(output, "COORD_SOURCE_CHANGED");
}

#[test]
fn terminal_request_can_be_read_without_creating_an_approval() {
    let f = Fixture::new();
    let open = f.publish("review", "RFC-0001", "review", &[]);
    let mut cancel = Fixture::operation("cancel", "cancel", "owner", &open);
    cancel["reason"] = json!("No longer needed");
    f.apply(&f.a, &cancel);
    let package = success(snapshot(&f, &f.a, "review"));
    assert_eq!(package["request"]["status"], "cancelled");
    assert_eq!(package["authorEvidence"]["authorship"], json!([]));
    assert!(package.get("reviewState").is_none());
}

#[test]
fn code_reference_is_distinct_from_main_and_must_remain_available() {
    let f = Fixture::new();
    git(f.seed.path(), &["checkout", "-qb", "candidate"]);
    fs::write(f.seed.path().join("source.txt"), "Candidate code\n").unwrap();
    git(f.seed.path(), &["add", "source.txt"]);
    git(
        f.seed.path(),
        &[
            "-c",
            "commit.gpgSign=false",
            "commit",
            "-qm",
            "Candidate code",
        ],
    );
    git(f.seed.path(), &["push", "-q", "origin", "candidate"]);
    let code_commit = git(f.seed.path(), &["rev-parse", "HEAD"]);
    git(f.seed.path(), &["checkout", "-q", "main"]);
    let mut publish = f.publish_operation("code-review", "RFC-0001", "review", &[]);
    publish["target"]["commit"] = json!(code_commit);
    f.apply(&f.a, &publish);
    let package = success(snapshot(&f, &f.a, "code-review"));
    assert_eq!(package["target"]["commit"], code_commit);
    assert_ne!(package["head"], code_commit);
    assert!(package.get("files").is_none());
    git(
        f.seed.path(),
        &["push", "-q", "origin", "--delete", "candidate"],
    );
    rejected(snapshot(&f, &f.a, "code-review"), "COORD_SOURCE_CHANGED");
    assert_eq!(f.list(&f.a)["requests"][0]["requestId"], "code-review");
}

#[test]
fn creator_declaration_and_signed_evidence_use_current_policy_trust() {
    let f = Fixture::new();
    let path = f.seed.path().join("gov/rfc/RFC-0001/rfc.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(path, text.replace("created = \"2026-01-01\"", "created = \"2026-01-01\"\ncreated_by = { subjectId = \"agent:writer\", name = \"Writer\" }" )).unwrap();
    author_record(&f, "signed", true, None);
    author_record(&f, "unsigned", false, None);
    f.push_seed("Add creator declaration and native authorship evidence");
    f.publish("review", "RFC-0001", "review", &[]);
    let package = success(snapshot(&f, &f.a, "review"));
    let authors = package["authorEvidence"]["authorship"].as_array().unwrap();
    assert_eq!(
        package["authorEvidence"]["createdBy"]["subjectId"],
        "agent:writer"
    );
    assert_eq!(authors.len(), 2);
    assert!(authors.iter().any(|e| e["verification"] == "verified"));
    assert!(authors.iter().any(|e| e["provenance"] == "declared"));
    let mut context = f.context_value();
    context["policies"]["v2"] =
        json!({"effectiveAfterCommit":f.head(),"trustedIssuers":[],"grants":[]});
    context["currentPolicyRevision"] = json!("v2");
    f.write_context(&context);
    let package = success(snapshot(&f, &f.a, "review"));
    let authors = package["authorEvidence"]["authorship"].as_array().unwrap();
    assert!(authors.iter().all(|e| e["verification"] == "unverified"));
    assert!(authors.iter().any(|e| e["provenance"] == "unknown-key"));
    assert_eq!(package["request"]["status"], "open");
}

#[test]
fn stale_author_evidence_is_omitted_but_malformed_record_diagnostics_remain() {
    let f = Fixture::new();
    let open = f.publish("review", "RFC-0001", "review", &[]);
    f.sync_seed();
    let mut target = open["target"].clone();
    target["contentDigest"] = json!("a".repeat(64));
    author_record(&f, "old-source", true, Some(target));
    let records = f.seed.path().join("gov/collaboration/rfc/RFC-0001");
    fs::write(
        records.join(format!("ev_{}.json", "b".repeat(64))),
        b"not valid JSON",
    )
    .unwrap();
    f.push_seed("Add unrelated revision evidence and malformed diagnostic");
    let package = success(snapshot(&f, &f.a, "review"));
    let evidence = package["authorEvidence"]["authorship"].as_array().unwrap();
    assert_eq!(evidence.len(), 1);
    assert!(evidence[0]["event"].is_null());
    assert!(evidence[0]["error"].is_string());
    assert_eq!(evidence[0]["verification"], "unverified");
}

#[cfg(unix)]
#[test]
fn unsafe_collaboration_directory_is_not_treated_as_missing_evidence() {
    let f = Fixture::new();
    // Ordinary unsafe attached files must not disappear during evidence capture.
    f.publish("review", "RFC-0001", "review", &[]);
    f.sync_seed();
    let records = f.seed.path().join("gov/collaboration/rfc/RFC-0001");
    std::os::unix::fs::symlink("/etc/passwd", records.join("unsafe.json")).unwrap();
    f.push_seed("Add unsafe attached source path");
    rejected(snapshot(&f, &f.a, "review"), "COORD_HISTORY_INVALID");
}

#[test]
fn oversized_package_is_rejected_without_partial_stdout() {
    let f = Fixture::new();
    // Each source is below the native 4 MiB bound, but rendering and structured
    // content together exceed the 16 MiB package bound.
    let dir = f.seed.path().join("gov/rfc/RFC-0001");
    let path = dir.join("rfc.toml");
    let rfc = fs::read_to_string(&path).unwrap().replace("title = \"Specification\"", "title = \"Specification\"\nclauses = [\"clauses/C-A.toml\", \"clauses/C-B.toml\", \"clauses/C-C.toml\"]");
    fs::write(path, rfc).unwrap();
    for name in ["C-A", "C-B", "C-C"] {
        fs::write(dir.join(format!("clauses/{name}.toml")), format!("[govctl]\nid = \"{name}\"\ntitle = \"Large\"\nkind = \"normative\"\n[content]\ntext = \"{}\"", "x".repeat(3 * 1024 * 1024))).unwrap();
    }
    f.push_seed("Add large bounded source files");
    f.publish("review", "RFC-0001", "review", &[]);
    let output = snapshot(&f, &f.a, "review");
    assert!(output.stdout.is_empty());
    rejected(output, "COORD_UNSUPPORTED");
}
