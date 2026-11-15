//! Fixed candidate review packages — [[RFC-0012:C-CANDIDATE-SNAPSHOT]].
mod common;
mod coordination_support;
use coordination_support::*;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn snapshot(f: &Fixture, dir: &Path, request: &str) -> std::process::Output {
    f.command(dir, "candidate-snapshot")
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

fn clause(f: &Fixture, text: &str) {
    fs::create_dir_all(f.seed.path().join("gov/rfc/RFC-0001/clauses")).unwrap();
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

fn candidate(f: &Fixture, branch: &str, change: impl FnOnce(&Fixture)) -> String {
    f.sync_seed();
    git(f.seed.path(), &["checkout", "-qb", branch]);
    change(f);
    git(f.seed.path(), &["add", "."]);
    git(
        f.seed.path(),
        &[
            "-c",
            "commit.gpgSign=false",
            "commit",
            "--allow-empty",
            "-qm",
            "Candidate input",
        ],
    );
    git(f.seed.path(), &["push", "-q", "origin", branch]);
    let commit = git(f.seed.path(), &["rev-parse", "HEAD"]);
    git(f.seed.path(), &["checkout", "-q", "main"]);
    commit
}

fn publish(f: &Fixture, request: &str, artifact: &str, commit: &str) -> Value {
    let mut operation = f.publish_operation(request, artifact, "review", &[]);
    operation["target"]["commit"] = json!(commit);
    f.apply(&f.a, &operation)["request"].clone()
}

#[test]
fn additive_capability_is_discoverable_outside_a_repository() {
    let root = tempfile::tempdir().unwrap();
    let caps = success(native(root.path(), &["coord", "capabilities"]));
    assert_eq!(caps["protocolVersion"], 1);
    for name in [
        "repository-main-coordination-v1",
        "request-input-snapshot-v1",
        "candidate-input-snapshot-v1",
    ] {
        assert!(
            caps["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!(name))
        );
    }
    assert_eq!(caps["sourceMutation"], false);
}

#[test]
fn baseline_and_candidate_are_distinct_and_ignore_the_caller_checkout() {
    let f = Fixture::new();
    clause(&f, "The baseline MUST remain available for comparison.");
    f.push_seed("Create baseline clause");
    let commit = candidate(&f, "candidate", |f| {
        let path = f
            .seed
            .path()
            .join("gov/rfc/RFC-0001/clauses/C-HISTORY.toml");
        let text = fs::read_to_string(&path).unwrap();
        fs::write(
            path,
            text.replace(
                "The baseline MUST remain available for comparison.",
                "The candidate MUST have its own exact digest.",
            ),
        )
        .unwrap();
    });
    let request = publish(&f, "review", "RFC-0001", &commit);
    let claimed = f.apply(&f.a, &Fixture::claim_operation("run", "agent:a", &request));
    let head = f.head();
    let local_head = git(&f.a, &["rev-parse", "HEAD"]);
    fs::write(
        f.a.join("gov/rfc/RFC-0001/rfc.toml"),
        "dirty invalid source",
    )
    .unwrap();
    git(&f.a, &["add", "gov/rfc/RFC-0001/rfc.toml"]);
    let index = fs::read(f.a.join(".git/index")).unwrap();
    let package = success(snapshot(&f, &f.a, "review"));
    assert_eq!(package["head"], head);
    assert_eq!(package["request"], claimed["request"]);
    assert_eq!(package["candidate"]["commit"], commit);
    assert_ne!(package["head"], commit);
    assert_eq!(package["baseline"]["target"]["commit"], Value::Null);
    assert_eq!(package["candidate"]["target"]["commit"], Value::Null);
    assert_eq!(
        package["baseline"]["target"]["contentDigest"],
        request["target"]["contentDigest"]
    );
    assert_ne!(
        package["baseline"]["target"]["contentDigest"],
        package["candidate"]["target"]["contentDigest"]
    );
    assert!(
        package["baseline"]["body"]
            .as_str()
            .unwrap()
            .contains("baseline MUST")
    );
    assert!(
        package["candidate"]["body"]
            .as_str()
            .unwrap()
            .contains("candidate MUST")
    );
    assert_eq!(
        package["candidate"]["content"]["clauseSources"][0]["status"],
        "deprecated"
    );
    assert_eq!(
        package["candidate"]["content"]["clauseSources"][0]["clause_id"],
        "C-HISTORY"
    );
    let old = success(
        f.command(&f.a, "snapshot")
            .args(["--request-id", "review"])
            .output()
            .unwrap(),
    );
    assert_eq!(old["body"], package["baseline"]["body"]);
    assert_eq!(old["content"], package["baseline"]["content"]);
    assert_eq!(old["target"], request["target"]);
    assert_eq!(f.head(), head);
    assert_eq!(git(&f.a, &["rev-parse", "HEAD"]), local_head);
    assert_eq!(fs::read(f.a.join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read_to_string(f.a.join("gov/rfc/RFC-0001/rfc.toml")).unwrap(),
        "dirty invalid source"
    );
    assert_eq!(success(snapshot(&f, f.root.path(), "review")), package);
    // Coordination-only main changes keep the fixed sources current, including
    // terminal requests; reading must not publish a new record or approval.
    let mut complete = Fixture::held_operation("complete", "finish", &claimed["request"]);
    complete["result"] = json!({"summary":"Candidate reviewed","references":[],"commit":commit});
    f.apply(&f.a, &complete);
    let terminal_head = f.head();
    let terminal = success(snapshot(&f, f.root.path(), "review"));
    assert_eq!(terminal["request"]["status"], "completed");
    assert_eq!(terminal["baseline"], package["baseline"]);
    assert_eq!(terminal["candidate"], package["candidate"]);
    assert_eq!(f.head(), terminal_head);
    assert!(terminal.get("reviewState").is_none());
}

#[test]
fn adr_and_work_candidates_use_their_native_source_revision() {
    let f = Fixture::new();
    assert!(
        native(f.seed.path(), &["adr", "new", "Canonical decision"])
            .status
            .success()
    );
    common::write_guarded_work_item(f.seed.path(), "WI-2026-01-01-001", "GUARD-TEST", None)
        .unwrap();
    f.push_seed("Add ADR and WI baselines");
    let mut contents = Vec::new();
    let commit = candidate(&f, "candidate", |f| {
        for (kind, id) in [("adr", "ADR-0001"), ("work", "WI-2026-01-01-001")] {
            let directory = f.seed.path().join(format!("gov/{kind}"));
            let path = fs::read_dir(directory)
                .unwrap()
                .map(|p| p.unwrap().path())
                .find(|p| fs::read_to_string(p).unwrap().contains(id))
                .unwrap();
            let mut source: toml::Value =
                toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            source["govctl"]["title"] = toml::Value::String(format!("Candidate {kind} title"));
            fs::write(path, toml::to_string(&source).unwrap()).unwrap();
            contents.push(success(native(
                f.seed.path(),
                &[kind, "show", id, "-o", "json"],
            )));
        }
    });
    for (index, (kind, id)) in [("adr", "ADR-0001"), ("work", "WI-2026-01-01-001")]
        .into_iter()
        .enumerate()
    {
        publish(&f, id, id, &commit);
        let package = success(snapshot(&f, f.root.path(), id));
        assert_eq!(package["candidate"]["target"]["kind"], kind);
        assert_eq!(package["candidate"]["target"]["id"], id);
        assert_eq!(package["candidate"]["content"], contents[index]);
        assert!(
            package["candidate"]["body"]
                .as_str()
                .unwrap()
                .contains(&format!("Candidate {kind} title"))
        );
        assert_ne!(
            package["baseline"]["content"],
            package["candidate"]["content"]
        );
        assert!(
            package["candidate"]["content"]
                .get("clauseSources")
                .is_none()
        );
    }
}

#[test]
fn candidate_authorship_matches_null_commit_and_current_policy() {
    let f = Fixture::new();
    let path = f.seed.path().join("gov/rfc/RFC-0001/rfc.toml");
    let source = fs::read_to_string(&path).unwrap();
    fs::write(path, source.replace("created = \"2026-01-01\"", "created = \"2026-01-01\"\ncreated_by = { subjectId = \"agent:writer\", name = \"Writer\" }")).unwrap();
    let baseline_record = author_record(&f, "baseline-author", true, None);
    f.push_seed("Record baseline author");
    let commit = candidate(&f, "candidate", |f| {
        clause(
            f,
            "The revised document MUST have independent author evidence.",
        );
        let record = author_record(f, "candidate-author", true, None);
        author_record(f, "declared-candidate-author", false, None);
        author_record(
            f,
            "stale-author",
            true,
            Some(baseline_record["event"]["target"].clone()),
        );
        let mut code_target = record["event"]["target"].clone();
        code_target["commit"] = json!(f.initial_head);
        author_record(f, "code-scoped-author", true, Some(code_target));
        let directory = f.seed.path().join("gov/collaboration/rfc/RFC-0001");
        fs::write(
            directory.join(format!("ev_{}.json", "b".repeat(64))),
            b"not JSON",
        )
        .unwrap();
    });
    publish(&f, "review", "RFC-0001", &commit);
    let package = success(snapshot(&f, &f.a, "review"));
    assert_eq!(
        package["candidate"]["authorEvidence"]["createdBy"]["subjectId"],
        "agent:writer"
    );
    let authors = package["candidate"]["authorEvidence"]["authorship"]
        .as_array()
        .unwrap();
    assert_eq!(authors.len(), 3);
    assert_eq!(
        authors
            .iter()
            .filter(|e| e["verification"] == "verified")
            .count(),
        1
    );
    assert!(authors.iter().any(|e| e["provenance"] == "declared"));
    assert!(
        authors
            .iter()
            .any(|e| e["event"].is_null() && e["error"].is_string())
    );
    for entry in authors.iter().filter(|e| !e["event"].is_null()) {
        assert_eq!(entry["event"]["target"], package["candidate"]["target"]);
    }
    let mut context = f.context_value();
    context["policies"]["v2"] =
        json!({"effectiveAfterCommit":f.head(),"trustedIssuers":[],"grants":[]});
    context["currentPolicyRevision"] = json!("v2");
    f.write_context(&context);
    let rotated = success(snapshot(&f, &f.a, "review"));
    let authors = rotated["candidate"]["authorEvidence"]["authorship"]
        .as_array()
        .unwrap();
    assert!(authors.iter().all(|e| e["verification"] == "unverified"));
    assert!(authors.iter().any(|e| e["provenance"] == "unknown-key"));
    assert_eq!(rotated["authority"]["revision"], "v2");
}

#[test]
fn unsupported_requests_are_distinct_from_unknown_requests() {
    let f = Fixture::new();
    rejected(snapshot(&f, &f.a, "missing"), "COORD_NOT_FOUND");
    f.publish("ordinary", "RFC-0001", "review", &[]);
    rejected(snapshot(&f, &f.a, "ordinary"), "COORD_UNSUPPORTED");
    let commit = candidate(&f, "candidate", |_| {});
    let mut operation = f.publish_operation("edit", "RFC-0001", "edit", &["RFC-0001"]);
    operation["target"]["commit"] = json!(commit);
    f.apply(&f.a, &operation);
    rejected(snapshot(&f, &f.a, "edit"), "COORD_UNSUPPORTED");
}

#[test]
fn unavailable_candidate_and_changed_baseline_fail_without_stdout() {
    let f = Fixture::new();
    let commit = candidate(&f, "candidate", |f| clause(f, "Candidate requirement"));
    publish(&f, "review", "RFC-0001", &commit);
    assert!(snapshot(&f, &f.a, "review").status.success());
    git(
        f.seed.path(),
        &["push", "-q", "origin", "--delete", "candidate"],
    );
    let unavailable = snapshot(&f, &f.a, "review");
    assert!(unavailable.stdout.is_empty());
    rejected(unavailable, "COORD_SOURCE_CHANGED");
    git(f.seed.path(), &["push", "-q", "origin", "candidate"]);
    f.sync_seed();
    clause(&f, "Changed main document");
    f.push_seed("Change canonical input");
    let changed = snapshot(&f, &f.a, "review");
    assert!(changed.stdout.is_empty());
    rejected(changed, "COORD_SOURCE_CHANGED");
}

#[test]
fn missing_malformed_or_ambiguous_candidate_sources_fail_closed() {
    let f = Fixture::new();
    assert!(
        native(f.seed.path(), &["adr", "new", "Decision"])
            .status
            .success()
    );
    f.push_seed("Add decision");
    for case in [
        "missing",
        "malformed",
        "missing-clause",
        "invalid-schema",
        "duplicate",
    ] {
        let commit = candidate(&f, case, |f| {
            let directory = f.seed.path().join("gov/rfc/RFC-0001");
            match case {
                "missing" => fs::remove_dir_all(directory).unwrap(),
                "malformed" => fs::write(directory.join("rfc.toml"), "not TOML").unwrap(),
                "missing-clause" => {
                    clause(f, "Referenced clause");
                    fs::remove_file(directory.join("clauses/C-HISTORY.toml")).unwrap();
                }
                "invalid-schema" => {
                    clause(f, "Referenced clause");
                    fs::write(
                        directory.join("clauses/C-HISTORY.toml"),
                        "[govctl]\nid = \"C-HISTORY\"",
                    )
                    .unwrap();
                }
                "duplicate" => {
                    let adr = fs::read_dir(f.seed.path().join("gov/adr"))
                        .unwrap()
                        .next()
                        .unwrap()
                        .unwrap()
                        .path();
                    fs::copy(adr, f.seed.path().join("gov/adr/duplicate.toml")).unwrap();
                }
                _ => unreachable!(),
            }
        });
        publish(&f, case, "RFC-0001", &commit);
        let result = snapshot(&f, &f.a, case);
        assert!(result.stdout.is_empty());
        rejected(result, "COORD_SOURCE_CHANGED");
    }
}

#[cfg(unix)]
#[test]
fn unsafe_candidate_source_and_attached_paths_are_rejected() {
    let f = Fixture::new();
    for case in ["source-link", "record-link"] {
        let commit = candidate(&f, case, |f| {
            let directory = if case == "source-link" {
                f.seed.path().join("gov/rfc/RFC-0001")
            } else {
                let directory = f.seed.path().join("gov/collaboration/rfc/RFC-0001");
                fs::create_dir_all(&directory).unwrap();
                directory
            };
            std::os::unix::fs::symlink("/etc/passwd", directory.join("unsafe.json")).unwrap();
        });
        publish(&f, case, "RFC-0001", &commit);
        let result = snapshot(&f, &f.a, case);
        assert!(result.stdout.is_empty());
        rejected(result, "COORD_HISTORY_INVALID");
    }
}

#[test]
fn aggregate_limit_counts_both_complete_packages() {
    let f = Fixture::new();
    let directory = f.seed.path().join("gov/rfc/RFC-0001");
    let path = directory.join("rfc.toml");
    let text = fs::read_to_string(&path).unwrap().replace(
        "title = \"Specification\"",
        "title = \"Specification\"\nclauses = [\"clauses/C-A.toml\", \"clauses/C-B.toml\"]",
    );
    fs::write(path, text).unwrap();
    for id in ["C-A", "C-B"] {
        fs::write(directory.join(format!("clauses/{id}.toml")), format!("[govctl]\nid = \"{id}\"\ntitle = \"Large\"\nkind = \"normative\"\n[content]\ntext = \"{}\"", "x".repeat(2300 * 1024))).unwrap();
    }
    f.push_seed("Add bounded sources");
    let commit = candidate(&f, "candidate", |_| {});
    publish(&f, "review", "RFC-0001", &commit);
    // Either one-source body/content package is below 16 MiB; their union is not.
    assert!(
        f.command(&f.a, "snapshot")
            .args(["--request-id", "review"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let output = snapshot(&f, &f.a, "review");
    assert!(output.stdout.is_empty());
    rejected(output, "COORD_UNSUPPORTED");
}
