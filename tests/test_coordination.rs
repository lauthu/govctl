//! Real remote/independent-clone acceptance for [[RFC-0012]].
mod common;
mod coordination_support;
use coordination_support::*;
use serde_json::json;
use std::{fs, process::Command, sync::Barrier};

#[test]
fn capabilities_work_without_any_governance_project() {
    let dir = tempfile::tempdir().unwrap();
    let result = success(
        Command::new(env!("CARGO_BIN_EXE_govctl"))
            .args(["coord", "capabilities"])
            .current_dir(dir.path())
            .output()
            .unwrap(),
    );
    assert_eq!(result["capabilities"][0], "repository-main-coordination-v1");
    assert_eq!(result["sourceMutation"], false);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn cross_clone_handoff_fences_old_attempts_and_preserves_idempotency() {
    let f = Fixture::new();
    fs::write(f.a.join("source.txt"), "user staged work\n").unwrap();
    git(&f.a, &["add", "source.txt"]);
    fs::write(f.a.join("untracked.txt"), "user untracked work\n").unwrap();
    let original_status = git(&f.a, &["status", "--porcelain"]);
    let original_head = git(&f.a, &["rev-parse", "HEAD"]);
    let published = f.publish("review-1", "RFC-0001", "review", &[]);
    let claimed = f.apply(
        &f.a,
        &Fixture::claim_operation("claim-a", "agent:a", &published),
    )["request"]
        .clone();
    assert_eq!(claimed["generation"], 1);
    assert_eq!(
        f.list(&f.b)["requests"][0]["holder"]["subjectId"],
        "agent:a"
    );

    let mut offer = Fixture::held_operation("handoff", "offer-b", &claimed);
    offer["toSubjectId"] = json!("agent:b");
    offer["checkpoint"] = json!({"summary":"Analyzed first section", "commit":null,"remaining":"Review remaining clauses"});
    let offered = f.apply(&f.a, &offer)["request"].clone();
    assert_eq!(offered["holder"], claimed["holder"]);
    assert_eq!(offered["generation"], 1);

    let mut accept = Fixture::operation("accept", "accept-b", "agent:b", &offered);
    accept["holder"] = json!({"subjectId":"agent:b","claimId":"claim-b","sessionId":"run-b"});
    let accepted = f.apply(&f.b, &accept)["request"].clone();
    assert_eq!(accepted["generation"], 2);
    assert_eq!(accepted["holder"]["subjectId"], "agent:b");

    let mut late = Fixture::held_operation("complete", "late-a", &claimed);
    late["result"] = json!({"summary":"Late result from old process","verdict":"approved"});
    rejected(f.apply_output(&f.a, &late), "COORD_STATE_CONFLICT");
    late["allowObsolete"] = json!(true);
    let historical = f.apply(&f.a, &late);
    assert_eq!(historical["effective"], false);
    assert_eq!(
        historical["request"]["stateEventId"],
        accepted["stateEventId"]
    );
    assert_eq!(historical["request"]["holder"], accepted["holder"]);

    let mut complete = Fixture::held_operation("complete", "complete-b", &accepted);
    complete["result"] =
        json!({"summary":"Reviewed all clauses","verdict":"approved","references":["RFC-0001"]});
    let done = f.apply(&f.b, &complete);
    assert_eq!(done["request"]["status"], "completed");
    assert_eq!(done["request"]["holder"], serde_json::Value::Null);
    assert_eq!(f.apply(&f.a, &complete)["replayed"], true);
    complete["result"]["summary"] = json!("Different result reusing the operation ID");
    rejected(
        f.apply_output(&f.a, &complete),
        "COORD_IDEMPOTENCY_CONFLICT",
    );
    rejected(
        f.apply_output(
            &f.a,
            &Fixture::claim_operation("claim-terminal", "agent:a", &done["request"]),
        ),
        "COORD_TERMINAL",
    );
    assert_eq!(git(&f.a, &["status", "--porcelain"]), original_status);
    assert_eq!(git(&f.a, &["rev-parse", "HEAD"]), original_head);
    assert_eq!(
        git(&f.server, &["show", "main:source.txt"]),
        "canonical source"
    );
    assert_eq!(f.list(&f.a), f.list(&f.b));
}

#[test]
fn competing_clones_cannot_both_claim_the_same_request() {
    let f = Fixture::new();
    let request = f.publish("race", "RFC-0001", "review", &[]);
    let a = Fixture::claim_operation("race-a", "agent:a", &request);
    let b = Fixture::claim_operation("race-b", "agent:b", &request);
    let barrier = Barrier::new(2);
    let (left, right) = std::thread::scope(|scope| {
        let left = scope.spawn(|| {
            barrier.wait();
            f.apply_output(&f.a, &a)
        });
        let right = scope.spawn(|| {
            barrier.wait();
            f.apply_output(&f.b, &b)
        });
        (left.join().unwrap(), right.join().unwrap())
    });
    assert_ne!(left.status.success(), right.status.success());
    let loser = if left.status.success() { right } else { left };
    rejected(loser, "COORD_STATE_CONFLICT");
    let snapshot = f.list(&f.b);
    assert_eq!(snapshot["requests"][0]["generation"], 1);
    assert_eq!(snapshot["requests"][0]["status"], "claimed");
}

#[test]
fn independent_reviews_parallelize_but_overlapping_write_scopes_do_not() {
    let f = Fixture::new();
    let first = f.publish("review-a", "RFC-0001", "review", &[]);
    let second = f.publish("review-b", "RFC-0001", "review", &[]);
    f.apply(
        &f.a,
        &Fixture::claim_operation("review-claim-a", "agent:a", &first),
    );
    f.apply(
        &f.b,
        &Fixture::claim_operation("review-claim-b", "agent:b", &second),
    );
    assert!(f.list(&f.a)["writes"].as_array().unwrap().is_empty());

    let edit_a = f.publish("edit-a", "RFC-0001", "edit", &["RFC-0001", "RFC-0002"]);
    let edit_b = f.publish("edit-b", "RFC-0002", "edit", &["RFC-0002"]);
    let mut claim_a = Fixture::claim_operation("edit-claim-a", "agent:a", &edit_a);
    claim_a["expectedWrites"] = json!({"RFC-0001":0,"RFC-0002":0});
    let held = f.apply(&f.a, &claim_a)["request"].clone();
    let mut claim_b = Fixture::claim_operation("edit-claim-b", "agent:b", &edit_b);
    claim_b["expectedWrites"] = json!({"RFC-0002":1});
    rejected(f.apply_output(&f.b, &claim_b), "COORD_WRITE_CONFLICT");
    let mut release = Fixture::held_operation("release", "release-edit-a", &held);
    release["expectedWrites"] = json!({"RFC-0001":1,"RFC-0002":1});
    f.apply(&f.a, &release);
    claim_b["expectedWrites"] = json!({"RFC-0002":2});
    assert_eq!(f.apply(&f.b, &claim_b)["request"]["status"], "claimed");
    let snapshot = f.list(&f.a);
    let write = snapshot["writes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["artifactId"] == "RFC-0002")
        .unwrap();
    assert_eq!(write["generation"], 3);
}

#[test]
fn source_changes_allow_release_and_cancel_without_rebinding_the_request() {
    let f = Fixture::new();
    let request = f.publish("stale", "RFC-0001", "review", &[]);
    let claimed = f.apply(
        &f.a,
        &Fixture::claim_operation("stale-claim", "agent:a", &request),
    )["request"]
        .clone();
    f.sync_seed();
    let source = f.seed.path().join("gov/rfc/RFC-0001/rfc.toml");
    let text = fs::read_to_string(&source).unwrap();
    fs::write(source, text.replace("First artifact", "Changed artifact")).unwrap();
    f.push_seed("Change the reviewed source");
    assert_eq!(f.list(&f.b)["requests"][0]["stale"], true);
    let mut result = Fixture::held_operation("complete", "stale-result", &claimed);
    result["result"] = json!({"summary":"Cannot approve the new input"});
    rejected(f.apply_output(&f.a, &result), "COORD_SOURCE_CHANGED");
    let released = f.apply(
        &f.a,
        &Fixture::held_operation("release", "stale-release", &claimed),
    )["request"]
        .clone();
    assert_eq!(released["stale"], true);
    rejected(
        f.apply_output(
            &f.b,
            &Fixture::claim_operation("stale-reclaim", "agent:b", &released),
        ),
        "COORD_SOURCE_CHANGED",
    );
    let mut cancel = Fixture::operation("cancel", "cancel-stale", "owner", &released);
    cancel["reason"] = json!("Input replaced; publish another request");
    assert_eq!(f.apply(&f.b, &cancel)["request"]["status"], "cancelled");
}

#[test]
#[cfg(unix)]
fn remote_rejection_grants_no_ownership_and_client_hooks_do_not_run() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let request = f.publish("protected", "RFC-0001", "review", &[]);
    let hook = f.server.join("hooks/pre-receive");
    fs::write(
        &hook,
        "#!/bin/sh\necho 'main requires an approved PR' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let before = f.head();
    rejected(
        f.apply_output(
            &f.a,
            &Fixture::claim_operation("protected-claim", "agent:a", &request),
        ),
        "COORD_PUBLISH_REJECTED",
    );
    assert_eq!(f.head(), before);
    assert_eq!(f.list(&f.b)["requests"][0]["status"], "open");

    fs::remove_file(hook).unwrap();
    let hooks = f.root.path().join("caller-hooks");
    fs::create_dir(&hooks).unwrap();
    let client_hook = hooks.join("pre-push");
    fs::write(&client_hook, "#!/bin/sh\nexit 83\n").unwrap();
    fs::set_permissions(client_hook, fs::Permissions::from_mode(0o755)).unwrap();
    let global = f.root.path().join("caller.gitconfig");
    fs::write(
        &global,
        format!("[core]\n hooksPath = {}\n", hooks.display()),
    )
    .unwrap();
    let mut command = f.apply_command(&f.a);
    command.env("GIT_CONFIG_GLOBAL", &global);
    let receipt = success(Fixture::run_apply(
        command,
        &Fixture::claim_operation("without-client-hook", "agent:a", &request),
    ));
    assert_eq!(receipt["request"]["status"], "claimed");
}

#[test]
fn published_feature_checkpoints_survive_ref_deletion_as_history() {
    let f = Fixture::new();
    git(f.seed.path(), &["checkout", "-qb", "feature/checkpoint"]);
    fs::write(
        f.seed.path().join("checkpoint.txt"),
        "unfinished branch work\n",
    )
    .unwrap();
    git(f.seed.path(), &["add", "checkpoint.txt"]);
    git(
        f.seed.path(),
        &["commit", "-qm", "Checkpoint on feature branch"],
    );
    let checkpoint = git(f.seed.path(), &["rev-parse", "HEAD"]);
    let mut publish = f.publish_operation("code-review", "RFC-0001", "review", &[]);
    publish["target"]["commit"] = json!(checkpoint);
    rejected(f.apply_output(&f.a, &publish), "COORD_SOURCE_CHANGED");
    git(
        f.seed.path(),
        &["push", "-q", "origin", "feature/checkpoint"],
    );
    let request = f.apply(&f.a, &publish)["request"].clone();
    let held = f.apply(
        &f.a,
        &Fixture::claim_operation("code-review-claim", "agent:a", &request),
    )["request"]
        .clone();
    let mut handoff = Fixture::held_operation("handoff", "code-handoff", &held);
    handoff["toSubjectId"] = json!("agent:b");
    handoff["checkpoint"] = json!({"summary":"Persisted partial work","commit":checkpoint,"remaining":"Finish second section"});
    let offered = f.apply(&f.b, &handoff)["request"].clone();
    git(
        f.seed.path(),
        &["push", "-q", "origin", "--delete", "feature/checkpoint"],
    );
    let snapshot = f.list(&f.b);
    assert_eq!(snapshot["requests"][0]["stale"], true);
    assert_eq!(snapshot["requests"][0]["holder"], held["holder"]);
    let mut accept = Fixture::operation("accept", "code-accept", "agent:b", &offered);
    accept["holder"] =
        json!({"subjectId":"agent:b","claimId":"new-code-claim","sessionId":"new-code-run"});
    rejected(f.apply_output(&f.b, &accept), "COORD_SOURCE_CHANGED");
    assert_eq!(
        f.apply(
            &f.a,
            &Fixture::held_operation("release", "code-release", &offered)
        )["request"]["status"],
        "open"
    );
}

#[test]
fn dry_run_is_ineffective_and_delegation_preserves_the_actual_actor() {
    let f = Fixture::new();
    let operation = f.publish_operation("dry-run", "RFC-0001", "review", &[]);
    let before = f.head();
    let mut command = f.apply_command(&f.a);
    command.arg("--dry-run");
    let preview = success(Fixture::run_apply(command, &operation));
    assert_eq!(preview["effective"], false);
    assert_eq!(preview["publicationCommit"], serde_json::Value::Null);
    assert_eq!(f.head(), before);
    assert!(f.list(&f.b)["requests"].as_array().unwrap().is_empty());
    let request = f.apply(&f.a, &operation)["request"].clone();
    let mut claim = Fixture::claim_operation("delegated-claim", "agent:a", &request);
    claim["actor"] = actor("owner");
    rejected(f.apply_output(&f.a, &claim), "COORD_UNAUTHORIZED");
    claim["onBehalfOf"] = json!("agent:a");
    let receipt = f.apply(&f.a, &claim);
    assert_eq!(receipt["request"]["holder"]["subjectId"], "agent:a");
    let path = format!(
        "main:gov/collaboration/rfc/RFC-0001/{}.json",
        receipt["eventId"].as_str().unwrap()
    );
    let event: serde_json::Value = serde_json::from_str(&git(&f.server, &["show", &path])).unwrap();
    assert_eq!(event["actor"]["subjectId"], "owner");
    assert_eq!(event["payload"]["operation"]["onBehalfOf"], "agent:a");
}

#[test]
#[cfg(unix)]
fn accepted_push_with_unreadable_receipt_is_reconciled_by_operation_identity() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let request = f.publish("lost-response", "RFC-0001", "review", &[]);
    let original_path = std::env::var_os("PATH").unwrap();
    let real_git = std::env::split_paths(&original_path)
        .map(|dir| dir.join("git"))
        .find(|path| path.is_file())
        .unwrap();
    let wrappers = f.root.path().join("transport-wrapper");
    fs::create_dir(&wrappers).unwrap();
    let wrapper = wrappers.join("git");
    fs::write(&wrapper, "#!/bin/sh\ncoord_test_push=0\nfor arg do\n  if [ \"$arg\" = push ]; then coord_test_push=1; fi\ndone\n\"$COORD_TEST_GIT_REAL\" \"$@\"\ncoord_test_status=$?\nif [ \"$coord_test_status\" = 0 ] && [ \"$coord_test_push\" = 1 ]; then\n  dd if=/dev/zero bs=1048576 count=17 2>/dev/null\nfi\nexit \"$coord_test_status\"\n").unwrap();
    fs::set_permissions(wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let paths = std::iter::once(wrappers).chain(std::env::split_paths(&original_path));
    let mut command = f.apply_command(&f.a);
    command
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("COORD_TEST_GIT_REAL", real_git);
    let operation = Fixture::claim_operation("accepted-before-output-failed", "agent:a", &request);
    let receipt = success(Fixture::run_apply(command, &operation));
    assert_eq!(receipt["replayed"], true);
    assert_eq!(receipt["effective"], true);
    assert_eq!(receipt["request"]["generation"], 1);
    assert_eq!(f.apply(&f.b, &operation)["replayed"], true);
    assert_eq!(f.list(&f.b)["requests"][0]["generation"], 1);
}
