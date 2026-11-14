//! Explicit creator declarations and revision-bound contributions.

mod common;

use common::{TestResult, first_work_id, init_project_with_date};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const REPO: &str = "example/portable-project";

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_govctl"))
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .expect("run govctl")
}

fn success(dir: &Path, args: &[&str]) -> String {
    let output = run(dir, args);
    assert!(
        output.status.success(),
        "{args:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("UTF-8 command output")
}

fn json_output(dir: &Path, args: &[&str]) -> Value {
    serde_json::from_str(&success(dir, args)).expect("JSON command output")
}

fn actor(subject: &str, name: &str) -> Value {
    json!({"subjectId": format!("agent:{subject}"), "agentId":subject,
        "participantId":format!("participant:{subject}"), "name":name,
        "kind":"agent", "role":"author", "roles":["author"]})
}

fn actor_file(dir: &Path, filename: &str, value: &Value) -> String {
    let path = dir.join(filename);
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    path.to_string_lossy().into_owned()
}

fn history(dir: &Path, id: &str) -> Value {
    json_output(dir, &["collab", "list", id, "--repo-id", REPO])
}

#[test]
fn creators_round_trip_for_every_kind_and_remain_distinct_from_contributors() -> TestResult {
    let (temp, date) = init_project_with_date()?;
    let dir = temp.path();
    let first = actor("first", "Original Agent");
    let second = actor("second", "Editing Agent");
    let first_file = actor_file(dir, "first.json", &first);
    let second_file = actor_file(dir, "second.json", &second);
    let work_id = first_work_id(&date);

    for (kind, id) in [("rfc", "RFC-0001"), ("adr", "ADR-0001"), ("work", &work_id)] {
        success(
            dir,
            &[
                kind,
                "new",
                "Attributed artifact",
                "--actor-file",
                &first_file,
                "--repo-id",
                REPO,
            ],
        );
        let creator = json_output(dir, &[kind, "get", id, "created_by", "-o", "json"]);
        assert_eq!(creator, first);
        let shown = json_output(dir, &[kind, "show", id, "-o", "json"]);
        let meta = if kind == "rfc" {
            &shown
        } else {
            &shown["govctl"]
        };
        assert_eq!(meta["created_by"], first);
        let rendered = success(dir, &[kind, "show", id, "-o", "plain"]);
        assert!(rendered.contains("Created by (declared):"));
        assert!(rendered.contains("Original Agent"));
        assert!(rendered.contains("agent:first"));

        let before = history(dir, id);
        assert_eq!(before["authorship"]["confirmed"], false);
        assert_eq!(before["authorship"]["authors"], json!([first]));
        let events = before["history"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"]["target"], before["target"]);
        assert_eq!(events[0]["event"]["target"]["repoId"], REPO);
        assert_eq!(events[0]["event"]["payload"]["created"], true);
        assert_eq!(events[0]["event"]["signature"], Value::Null);

        success(
            dir,
            &[
                kind,
                "edit",
                id,
                "title",
                "--set",
                "Contributed artifact",
                "--actor-file",
                &second_file,
                "--repo-id",
                REPO,
            ],
        );
        assert_eq!(
            json_output(dir, &[kind, "get", id, "created_by", "-o", "json"]),
            first
        );
        let after = history(dir, id);
        assert_eq!(after["history"].as_array().unwrap().len(), 2);
        assert_ne!(
            before["target"]["contentDigest"],
            after["target"]["contentDigest"]
        );
        assert_eq!(after["authorship"]["authors"], json!([first, second]));
        assert_eq!(after["authorship"]["confirmed"], false);
        assert_eq!(after["reviewState"]["authorsConfirmed"], false);
        assert_eq!(after["reviewState"]["approvedCount"], 0);

        let immutable = run(
            dir,
            &[kind, "edit", id, "created_by", "--set", "Replacement"],
        );
        assert!(!immutable.status.success());
        assert_eq!(
            json_output(dir, &[kind, "get", id, "created_by", "-o", "json"]),
            first
        );
    }
    Ok(())
}

#[test]
fn unknown_creators_are_not_inferred_from_owners_and_noops_do_not_author() -> TestResult {
    let (temp, date) = init_project_with_date()?;
    let dir = temp.path();
    let editor = actor_file(dir, "editor.json", &actor("editor", "Editor"));
    let work_id = first_work_id(&date);
    for (kind, id) in [("rfc", "RFC-0001"), ("adr", "ADR-0001"), ("work", &work_id)] {
        success(dir, &[kind, "new", "Historical artifact"]);
        assert_eq!(
            json_output(dir, &[kind, "get", id, "created_by", "-o", "json"]),
            Value::Null
        );
        let rendered = success(dir, &[kind, "show", id, "-o", "plain"]);
        assert!(!rendered.contains("Created by"));
        assert_eq!(history(dir, id)["authorship"]["authors"], json!([]));
        // In particular, the first RFC set may update `updated` even though the title is unchanged.
        success(
            dir,
            &[
                kind,
                "edit",
                id,
                "title",
                "--set",
                "Historical artifact",
                "--actor-file",
                &editor,
                "--repo-id",
                REPO,
            ],
        );
        assert_eq!(history(dir, id)["history"], json!([]));
    }

    success(
        dir,
        &[
            "rfc",
            "edit",
            "RFC-0001",
            "owners",
            "--add",
            "@assigned-agent",
            "--actor-file",
            &editor,
            "--repo-id",
            REPO,
        ],
    );
    assert_eq!(history(dir, "RFC-0001")["authorship"]["authors"], json!([]));
    success(
        dir,
        &[
            "work",
            "edit",
            &work_id,
            "acceptance_criteria",
            "--add",
            "add: Deliver the feature",
        ],
    );
    success(
        dir,
        &[
            "work",
            "edit",
            &work_id,
            "acceptance_criteria[0]",
            "--tick",
            "done",
            "--actor-file",
            &editor,
            "--repo-id",
            REPO,
        ],
    );
    assert_eq!(history(dir, &work_id)["history"], json!([]));

    success(
        dir,
        &[
            "rfc",
            "edit",
            "RFC-0001",
            "title",
            "--set",
            "Real contribution",
            "--actor-file",
            &editor,
            "--repo-id",
            REPO,
        ],
    );
    assert_eq!(
        history(dir, "RFC-0001")["authorship"]["authors"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        json_output(dir, &["rfc", "get", "RFC-0001", "created_by", "-o", "json"]),
        Value::Null
    );
    Ok(())
}

#[test]
fn preview_and_invalid_context_cannot_write_artifacts_or_contributions() -> TestResult {
    let (temp, _) = init_project_with_date()?;
    let dir = temp.path();
    let valid_file = actor_file(dir, "actor.json", &actor("writer", "Writer"));
    for kind in ["rfc", "adr", "work"] {
        success(
            dir,
            &[
                kind,
                "new",
                "Preview",
                "--actor-file",
                &valid_file,
                "--repo-id",
                REPO,
                "--dry-run",
            ],
        );
    }
    assert!(!dir.join("gov/rfc/RFC-0001").exists());
    assert_eq!(fs::read_dir(dir.join("gov/adr"))?.count(), 0);
    assert_eq!(fs::read_dir(dir.join("gov/work"))?.count(), 0);
    assert!(!dir.join("gov/collaboration").exists());

    let invalid_file = dir.join("invalid.json");
    let invalid_path = invalid_file.to_str().unwrap();
    for bytes in [
        b"{invalid json".to_vec(),
        serde_json::to_vec(&json!({"subjectId":" ","name":"Unknown"}))?,
        serde_json::to_vec(&json!({"subjectId":"agent:writer","name":"Writer","kind":"model"}))?,
        serde_json::to_vec(&json!({"subjectId":"agent:writer","name":"Writer\nInjected"}))?,
        vec![b' '; 65_537],
    ] {
        fs::write(&invalid_file, bytes)?;
        assert!(
            !run(
                dir,
                &[
                    "rfc",
                    "new",
                    "Invalid",
                    "--actor-file",
                    invalid_path,
                    "--repo-id",
                    REPO
                ]
            )
            .status
            .success()
        );
        assert!(!dir.join("gov/rfc/RFC-0001").exists());
    }
    assert!(
        !run(
            dir,
            &[
                "rfc",
                "new",
                "Invalid repo",
                "--actor-file",
                &valid_file,
                "--repo-id",
                ""
            ]
        )
        .status
        .success()
    );
    assert!(!dir.join("gov/rfc/RFC-0001").exists());
    assert!(
        !run(dir, &["rfc", "new", "Missing actor", "--repo-id", REPO])
            .status
            .success()
    );
    assert!(!dir.join("gov/collaboration").exists());

    success(dir, &["rfc", "new", "Unchanged"]);
    let source = dir.join("gov/rfc/RFC-0001/rfc.toml");
    let original = fs::read(&source)?;
    assert!(
        !run(
            dir,
            &[
                "rfc",
                "edit",
                "RFC-0001",
                "title",
                "--set",
                "Changed",
                "--actor-file",
                invalid_path
            ]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(&source)?, original);
    success(
        dir,
        &[
            "rfc",
            "edit",
            "RFC-0001",
            "title",
            "--set",
            "Preview",
            "--actor-file",
            &valid_file,
            "--repo-id",
            REPO,
            "--dry-run",
        ],
    );
    assert_eq!(fs::read(&source)?, original);
    assert!(!dir.join("gov/collaboration").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn actor_symlinks_are_rejected_before_creation() -> TestResult {
    let (temp, _) = init_project_with_date()?;
    let dir = temp.path();
    let actor = actor_file(dir, "actor.json", &actor("writer", "Writer"));
    let link = dir.join("link.json");
    std::os::unix::fs::symlink(actor, &link)?;
    assert!(
        !run(
            dir,
            &[
                "rfc",
                "new",
                "Unsafe context",
                "--actor-file",
                link.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    assert!(!dir.join("gov/rfc/RFC-0001").exists());
    Ok(())
}

#[test]
fn human_clause_contributions_bind_the_parent_rfc_without_fabricating_its_creator() -> TestResult {
    let (temp, _) = init_project_with_date()?;
    let dir = temp.path();
    let human = json!({"subjectId":"human:editor", "name":"Human editor", "kind":"human"});
    let context = actor_file(dir, "human.json", &human);
    success(dir, &["rfc", "new", "Existing RFC"]);
    success(dir, &["clause", "new", "RFC-0001:C-TEST", "Test clause"]);
    for (field, operation, value) in [
        ("text", "--set", "A human-authored clause."),
        ("anchors", "--add", "src/example.rs"),
    ] {
        success(
            dir,
            &[
                "clause",
                "edit",
                "RFC-0001:C-TEST",
                field,
                operation,
                value,
                "--actor-file",
                &context,
                "--repo-id",
                REPO,
            ],
        );
    }
    let view = history(dir, "RFC-0001");
    assert_eq!(view["history"].as_array().unwrap().len(), 2);
    assert_eq!(view["authorship"]["authors"], json!([human]));
    assert_eq!(view["authorship"]["confirmed"], false);
    assert_eq!(
        json_output(dir, &["rfc", "get", "RFC-0001", "created_by", "-o", "json"]),
        Value::Null
    );
    Ok(())
}

#[test]
fn explicit_human_creators_round_trip_for_all_artifact_kinds() -> TestResult {
    let (temp, date) = init_project_with_date()?;
    let dir = temp.path();
    let human = json!({"subjectId":"human:creator", "name":"Human creator", "kind":"human"});
    let context = actor_file(dir, "human.json", &human);
    let work_id = first_work_id(&date);
    for (kind, id) in [("rfc", "RFC-0001"), ("adr", "ADR-0001"), ("work", &work_id)] {
        success(
            dir,
            &[
                kind,
                "new",
                "Human authored",
                "--actor-file",
                &context,
                "--repo-id",
                REPO,
            ],
        );
        assert_eq!(
            json_output(dir, &[kind, "get", id, "created_by", "-o", "json"]),
            human
        );
        let view = history(dir, id);
        assert_eq!(view["authorship"]["authors"], json!([human]));
        assert_eq!(view["authorship"]["confirmed"], false);
    }
    Ok(())
}
