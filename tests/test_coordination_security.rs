//! Adversarial trust and history boundaries for [[RFC-0012:C-POLICY]].
mod common;
mod coordination_support;

use coordination_support::*;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn revoke_actor(f: &Fixture, subject: &str) -> Value {
    let mut context = f.context_value();
    let mut next = context["policies"]["v1"].clone();
    next["effectiveAfterCommit"] = json!(f.head());
    next["grants"]
        .as_array_mut()
        .unwrap()
        .retain(|grant| grant["subjectId"] != subject);
    context["policies"]["v2"] = next;
    context["currentPolicyRevision"] = json!("v2");
    f.write_context(&context);
    context
}

fn record_file(f: &Fixture, event_id: &str) -> PathBuf {
    f.seed
        .path()
        .join("gov/collaboration/rfc/RFC-0001")
        .join(format!("{event_id}.json"))
}

#[test]
fn revocation_preserves_accepted_claims_and_replay_without_new_authority() {
    let f = Fixture::new();
    let request = f.publish("revoked-review", "RFC-0001", "review", &[]);
    let claim = Fixture::claim_operation("accepted-before-revocation", "agent:a", &request);
    let accepted = f.apply(&f.a, &claim);
    let claimed = accepted["request"].clone();
    let activation = f.head();
    revoke_actor(&f, "agent:a");

    let snapshot = f.list(&f.b);
    assert_eq!(snapshot["authority"]["revision"], "v2");
    assert_eq!(snapshot["requests"][0]["holder"], claimed["holder"]);
    assert_eq!(snapshot["requests"][0]["generation"], 1);

    let replay = f.apply(&f.b, &claim);
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["eventId"], accepted["eventId"]);
    assert_eq!(f.head(), activation);

    let mut completion = Fixture::held_operation("complete", "revoked-result", &claimed);
    completion["result"] = json!({"summary":"A revoked actor cannot finish this request"});
    rejected(f.apply_output(&f.a, &completion), "COORD_UNAUTHORIZED");
    assert_eq!(f.head(), activation);

    let mut cancel = Fixture::operation("cancel", "authorized-cleanup", "owner", &claimed);
    cancel["reason"] = json!("The holder's permission was revoked");
    let cancelled = f.apply(&f.b, &cancel);
    assert_eq!(cancelled["request"]["status"], "cancelled");
    assert!(cancelled["request"]["holder"].is_null());
    assert_eq!(f.list(&f.a)["requests"][0]["status"], "cancelled");
}

#[test]
fn an_old_policy_cannot_authorize_a_new_event_after_its_activation_boundary() {
    let f = Fixture::new();
    let request = f.publish("old-policy-injection", "RFC-0001", "review", &[]);
    let old_context = f.context_value();
    let current_context = revoke_actor(&f, "agent:a");
    let claim = Fixture::claim_operation("revoked-claim", "agent:a", &request);
    rejected(f.apply_output(&f.a, &claim), "COORD_UNAUTHORIZED");

    // Simulate a stale or malicious publisher with an old external context. Its
    // signature is real, but current-policy readers must reject the new event.
    f.write_context(&old_context);
    f.apply(&f.a, &claim);
    f.write_context(&current_context);
    rejected(
        f.command(&f.b, "list").output().unwrap(),
        "COORD_HISTORY_INVALID",
    );
    let head = f.head();
    let mut another = claim.clone();
    another["operationId"] = json!("another-old-policy-claim");
    rejected(f.apply_output(&f.b, &another), "COORD_HISTORY_INVALID");
    assert_eq!(f.head(), head);
}

#[test]
fn rewriting_or_deleting_an_accepted_record_fails_closed() {
    for delete in [false, true] {
        let f = Fixture::new();
        let request = f.publish("immutable-history", "RFC-0001", "review", &[]);
        f.sync_seed();
        let path = record_file(&f, request["stateEventId"].as_str().unwrap());
        if delete {
            fs::remove_file(path).unwrap();
        } else {
            let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            record["payload"]["operation"]["description"] = json!("Rewritten control intent");
            fs::write(path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        }
        f.push_seed(if delete {
            "Delete accepted history"
        } else {
            "Rewrite accepted history"
        });
        rejected(
            f.command(&f.b, "list").output().unwrap(),
            "COORD_HISTORY_INVALID",
        );
        let head = f.head();
        rejected(
            f.apply_output(
                &f.a,
                &Fixture::claim_operation("claim-on-invalid-history", "agent:a", &request),
            ),
            "COORD_HISTORY_INVALID",
        );
        assert_eq!(f.head(), head);
    }
}

#[test]
fn a_signed_control_record_cannot_enter_main_through_a_merge_commit() {
    let f = Fixture::new();
    let request = f.publish("merged-control", "RFC-0001", "review", &[]);
    let base = f.head();
    f.apply(
        &f.a,
        &Fixture::claim_operation("side-branch-claim", "agent:a", &request),
    );
    let control_commit = f.head();
    f.sync_seed();
    git(
        f.seed.path(),
        &["branch", "control-proposal", &control_commit],
    );

    // All ref rewrites stay inside this fresh test fixture. The eventual main
    // publication is a merge containing a valid signed record from a side branch.
    git(
        &f.server,
        &["update-ref", "refs/heads/main", &base, &control_commit],
    );
    git(f.seed.path(), &["reset", "--hard", &base]);
    git(
        f.seed.path(),
        &[
            "-c",
            "commit.gpgSign=false",
            "commit",
            "--allow-empty",
            "-qm",
            "Concurrent main change",
        ],
    );
    git(
        f.seed.path(),
        &[
            "-c",
            "commit.gpgSign=false",
            "merge",
            "--no-ff",
            "-m",
            "Import control proposal",
            "control-proposal",
        ],
    );
    git(f.seed.path(), &["push", "-q", "origin", "main"]);
    rejected(
        f.command(&f.b, "list").output().unwrap(),
        "COORD_HISTORY_INVALID",
    );
}

#[test]
fn obsolete_evidence_must_match_a_real_accepted_attempt() {
    let f = Fixture::new();
    let request = f.publish("historical-attempt", "RFC-0001", "review", &[]);
    let claimed = f.apply(
        &f.a,
        &Fixture::claim_operation("real-attempt", "agent:a", &request),
    )["request"]
        .clone();
    let released = f.apply(
        &f.a,
        &Fixture::held_operation("release", "release-real-attempt", &claimed),
    )["request"]
        .clone();
    let head = f.head();
    let mut historical = Fixture::held_operation("complete", "historical-result", &claimed);
    historical["allowObsolete"] = json!(true);
    historical["result"] = json!({"summary":"Late evidence from a released attempt"});

    for (field, value) in [
        ("claimId", json!("invented-claim")),
        ("sessionId", json!("invented-session")),
        ("expectedGeneration", json!(999)),
        ("expectedState", json!(format!("ev_{}", "0".repeat(64)))),
    ] {
        let mut forged = historical.clone();
        forged[field] = value;
        rejected(f.apply_output(&f.b, &forged), "COORD_STATE_CONFLICT");
        assert_eq!(f.head(), head);
    }

    let evidence = f.apply(&f.a, &historical);
    assert_eq!(evidence["effective"], false);
    assert_eq!(
        evidence["request"]["stateEventId"],
        released["stateEventId"]
    );
    assert_eq!(evidence["request"]["generation"], released["generation"]);
    assert_eq!(evidence["request"]["status"], "open");
    assert!(evidence["request"]["holder"].is_null());
    assert!(evidence["request"]["result"].is_null());
}

#[test]
fn role_claims_do_not_authorize_an_ungranted_actor() {
    let f = Fixture::new();
    let request = f.publish("role-forgery", "RFC-0001", "review", &[]);
    let head = f.head();
    let mut forged = Fixture::claim_operation("forged-admin-role", "ungranted", &request);
    forged["actor"]["role"] = json!("admin");
    forged["actor"]["roles"] = json!(["owner", "admin", "arbiter"]);
    rejected(f.apply_output(&f.a, &forged), "COORD_UNAUTHORIZED");
    assert_eq!(f.head(), head);
    assert_eq!(f.list(&f.b)["requests"][0]["status"], "open");
}

#[test]
fn operation_identity_cannot_be_reused_by_a_different_actor() {
    let f = Fixture::new();
    let request = f.publish("operation-identity", "RFC-0001", "review", &[]);
    let original = Fixture::claim_operation("shared-operation-id", "agent:a", &request);
    let receipt = f.apply(&f.a, &original);
    let head = f.head();
    let replacement = Fixture::claim_operation("shared-operation-id", "agent:b", &request);
    rejected(
        f.apply_output(&f.b, &replacement),
        "COORD_IDEMPOTENCY_CONFLICT",
    );
    assert_eq!(f.head(), head);
    let replay = f.apply(&f.b, &original);
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["eventId"], receipt["eventId"]);
    assert_eq!(replay["request"]["holder"]["subjectId"], "agent:a");
}

#[test]
fn nested_target_extensions_and_invalid_actor_text_are_rejected_before_publication() {
    let f = Fixture::new();
    let original = f.publish_operation("invalid-wire", "RFC-0001", "review", &[]);
    let head = f.head();
    let mut extended = original.clone();
    extended["target"]["futureAuthorization"] = json!({"admin":true});
    rejected(f.apply_output(&f.a, &extended), "COORD_INVALID_INPUT");
    let mut invalid_actor = original;
    invalid_actor["actor"]["name"] = json!("Name\nwith control text");
    rejected(f.apply_output(&f.a, &invalid_actor), "COORD_INVALID_INPUT");
    assert_eq!(f.head(), head);
    assert!(f.list(&f.b)["requests"].as_array().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn rotating_a_signing_key_mid_apply_cannot_poison_canonical_history() {
    use ed25519_dalek::{
        SigningKey,
        pkcs8::{EncodePrivateKey, spki::der::pem::LineEnding},
    };
    use std::os::unix::fs::PermissionsExt;

    fn quoted(path: &std::path::Path) -> String {
        format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"))
    }

    let f = Fixture::new();
    let operation = f.publish_operation("key-rotation-race", "RFC-0001", "review", &[]);
    let head = f.head();
    let path = std::env::var_os("PATH").unwrap();
    let real_git = std::env::split_paths(&path)
        .map(|entry| entry.join("git"))
        .find(|entry| entry.is_file())
        .unwrap();
    let replacement = f.root.path().join("rotated-key.pem");
    fs::write(
        &replacement,
        SigningKey::from_bytes(&[77; 32])
            .to_pkcs8_pem(LineEnding::LF)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600)).unwrap();
    let tools = f.root.path().join("test-tools");
    fs::create_dir(&tools).unwrap();
    let marker = f.root.path().join("rotation-observed");
    let wrapper = tools.join("git");
    fs::write(&wrapper, format!(
        "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ \"$arg\" = fetch ] && [ ! -e {} ]; then\n    cp {} {}\n    touch {}\n  fi\ndone\nexec {} \"$@\"\n",
        quoted(&marker), quoted(&replacement), quoted(&f.key), quoted(&marker), quoted(&real_git),
    )).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = f.apply_command(&f.a);
    let mut entries = vec![tools];
    entries.extend(std::env::split_paths(&path));
    command.env("PATH", std::env::join_paths(entries).unwrap());
    let output = Fixture::run_apply(command, &operation);
    assert!(
        marker.exists(),
        "The key must rotate after signer inspection and before event creation"
    );

    if output.status.success() {
        // Holding one immutable key snapshot is also valid: publication remains
        // signed by the originally authorized issuer, and fresh replay succeeds.
        let receipt = success(output);
        f.sync_seed();
        let record: Value = serde_json::from_slice(
            &fs::read(record_file(&f, receipt["eventId"].as_str().unwrap())).unwrap(),
        )
        .unwrap();
        assert_eq!(record["signature"]["keyId"], f.key_id);
        assert_eq!(f.list(&f.b)["requests"][0]["status"], "open");
    } else {
        assert!(String::from_utf8_lossy(&output.stderr).contains("COORD_"));
        assert_eq!(f.head(), head);
        assert!(f.list(&f.b)["requests"].as_array().unwrap().is_empty());
    }
}
