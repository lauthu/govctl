//! End-to-end attached collaboration acceptance for RFC-0011.
mod common;
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{
    Signer, SigningKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey, spki::der::pem::LineEnding},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

const ID: &str = "RFC-0001";
const REPO: &str = "test/project";
fn command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_govctl"));
    command
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("GOVCTL_DEFAULT_OWNER", "@test");
    command
}
fn run(dir: &Path, args: &[&str]) -> Output {
    command(dir, args).output().unwrap()
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn project() -> tempfile::TempDir {
    let dir = common::init_project().unwrap();
    common::write_minimal_rfc(dir.path(), ID, "Portable test").unwrap();
    dir
}
fn target(dir: &Path) -> Value {
    success(run(dir, &["collab", "target", ID, "--repo-id", REPO]))
}
fn keyfiles(dir: &Path) -> (PathBuf, PathBuf, SigningKey) {
    let key = SigningKey::from_bytes(&[7; 32]);
    let private = dir.join("private.pem");
    let public = dir.join("public.pem");
    fs::write(
        &private,
        key.to_pkcs8_pem(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&private, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fs::write(
        &public,
        key.verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap(),
    )
    .unwrap();
    (private, public, key)
}
fn actor(subject: &str, role: &str) -> Value {
    json!({"subjectId":subject,"name":subject,"role":role,"kind":"agent"})
}
fn add(dir: &Path, kind: &str, actor: Value, payload: Value, key: &str, signed: bool) -> Value {
    fs::write(dir.join("actor.json"), serde_json::to_vec(&actor).unwrap()).unwrap();
    fs::write(
        dir.join("payload.json"),
        serde_json::to_vec(&payload).unwrap(),
    )
    .unwrap();
    let mut args = vec![
        "collab",
        "add",
        ID,
        "--repo-id",
        REPO,
        "--type",
        kind,
        "--actor-file",
        "actor.json",
        "--payload-file",
        "payload.json",
        "--idempotency-key",
        key,
    ];
    if signed {
        args.extend(["--sign-key-file", "private.pem"]);
    }
    success(run(dir, &args))
}
fn list(dir: &Path, trusted: bool) -> Value {
    let mut args = vec!["collab", "list", ID, "--repo-id", REPO];
    if trusted {
        args.extend(["--trust-key-file", "public.pem"]);
    }
    success(run(dir, &args))
}
fn append(dir: &Path, bytes: &[u8], stale: bool) -> Output {
    let mut args = vec!["collab", "append", ID, "--repo-id", REPO, "--stdin"];
    if stale {
        args.push("--allow-stale");
    }
    let mut child = command(dir, &args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    child.wait_with_output().unwrap()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn canonical(value: &Value) -> String {
    serde_json::to_string(value).unwrap()
}
fn legacy(target: Value, signer: &SigningKey, key: &str) -> Value {
    let who = actor("agent:reviewer", "reviewer");
    let payload = json!({"scope":"artifact","verdict":"changes_requested","reason":"First line\r\nSecond line 🧪"});
    let mut body = json!({"formatVersion":1,"eventId":format!("ev_{}",hash(key.as_bytes())),"createdAt":1700000000000u64,"type":"review","actor":who,"target":target,
        "requestDigest":hash(canonical(&json!({"type":"review","subjectId":who["subjectId"],"target":target,"payload":payload})).as_bytes()),"payload":payload});
    let public = signer
        .verifying_key()
        .to_public_key_pem(LineEnding::LF)
        .unwrap();
    let signature = STANDARD.encode(signer.sign(canonical(&body).as_bytes()).to_bytes());
    body["signature"] = json!({"algorithm":"Ed25519","keyId":hash(public.as_bytes()),"publicKey":public,"value":signature});
    body
}

#[test]
fn unsigned_comments_replies_batch_and_idempotence_preserve_source() {
    let dir = project();
    let dir = dir.path();
    let initial = target(dir);
    let first = add(
        dir,
        "comment",
        actor("agent:dsh", "reviewer"),
        json!({"body":"Define the rollout limit."}),
        "comment-1",
        false,
    );
    let retry = add(
        dir,
        "comment",
        actor("agent:dsh", "reviewer"),
        json!({"body":"Define the rollout limit."}),
        "comment-1",
        false,
    );
    assert_eq!(first["event"], retry["event"]);
    assert_eq!(retry["replayed"], true);
    add(
        dir,
        "comment",
        actor("agent:writer", "implementer"),
        json!({"body":"Added it.","replyTo":first["event"]["eventId"]}),
        "reply-1",
        false,
    );
    assert_eq!(initial, target(dir));
    let view = list(dir, false);
    assert_eq!(view["reviews"].as_array().unwrap().len(), 2);
    assert_eq!(view["reviews"][0]["provenance"], "declared");
    assert_eq!(view["reviewState"]["approvedCount"], 0);
    let batch = success(run(dir, &["collab", "list", "--all", "--repo-id", REPO]));
    assert_eq!(batch["artifacts"].as_array().unwrap().len(), 1);
    let human = run(
        dir,
        &["collab", "show", ID, "--repo-id", REPO, "-o", "plain"],
    );
    assert!(String::from_utf8_lossy(&human.stdout).contains("Define the rollout limit."));
    fs::write(dir.join("payload.json"), r#"{"body":"different"}"#).unwrap();
    fs::write(
        dir.join("actor.json"),
        serde_json::to_vec(&actor("agent:dsh", "reviewer")).unwrap(),
    )
    .unwrap();
    let conflict = run(
        dir,
        &[
            "collab",
            "add",
            ID,
            "--repo-id",
            REPO,
            "--type",
            "comment",
            "--actor-file",
            "actor.json",
            "--payload-file",
            "payload.json",
            "--idempotency-key",
            "comment-1",
        ],
    );
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("COLLAB_IDEMPOTENCY_CONFLICT"));
}

#[test]
fn legacy_signed_bytes_unknown_trust_tamper_and_stale_replay() {
    let dir = project();
    let dir = dir.path();
    let (_, _, key) = keyfiles(dir);
    let event = legacy(target(dir), &key, "legacy-1");
    let bytes = format!("{}\n", serde_json::to_string_pretty(&event).unwrap()).into_bytes();
    assert_eq!(success(append(dir, &bytes, false))["replayed"], false);
    let stored = dir.join(format!(
        "gov/collaboration/rfc/{ID}/{}.json",
        event["eventId"].as_str().unwrap()
    ));
    assert_eq!(fs::read(&stored).unwrap(), bytes);
    assert_eq!(list(dir, false)["reviewState"]["blockingCount"], 0);
    assert_eq!(list(dir, true)["reviewState"]["blockingCount"], 1);
    let rfc = dir.join(format!("gov/rfc/{ID}/rfc.toml"));
    let source = fs::read_to_string(&rfc).unwrap();
    fs::write(&rfc, source.replace("Portable test", "Changed source")).unwrap();
    assert_eq!(success(append(dir, &bytes, false))["replayed"], true);
    assert_eq!(fs::read(stored).unwrap(), bytes);
    let stale = list(dir, true);
    assert_eq!(stale["reviews"][0]["validity"]["state"], "stale");
    assert_eq!(stale["reviewState"]["blockingCount"], 1);
    let mut second = legacy(event["target"].clone(), &key, "legacy-2");
    let second_bytes = serde_json::to_vec(&second).unwrap();
    let rejected = append(dir, &second_bytes, false);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("COLLAB_SOURCE_CHANGED"));
    success(append(dir, &second_bytes, true));
    second["payload"]["reason"] = json!("Tampered");
    assert!(
        !append(dir, &serde_json::to_vec(&second).unwrap(), true)
            .status
            .success()
    );
    assert!(run(dir, &["check"]).status.success());
}

#[test]
fn approvals_require_complete_authors_independence_and_current_revision() {
    let dir = project();
    let dir = dir.path();
    keyfiles(dir);
    let author = actor("agent:writer", "reviewer");
    add(
        dir,
        "authorship",
        actor("human:owner", "owner"),
        json!({"authors":[author]}),
        "authors",
        true,
    );
    add(
        dir,
        "review",
        author.clone(),
        json!({"scope":"artifact","verdict":"approved","reason":"Self review"}),
        "self",
        true,
    );
    let independent = actor("agent:independent", "reviewer");
    add(
        dir,
        "review",
        independent.clone(),
        json!({"scope":"artifact","verdict":"approved","reason":"Independent review"}),
        "independent",
        true,
    );
    let view = list(dir, true);
    assert_eq!(view["authorship"]["confirmed"], true);
    assert_eq!(view["reviewState"]["approvedCount"], 1);
    add(
        dir,
        "authorship",
        independent.clone(),
        json!({"authors":[independent]}),
        "declared-coauthor",
        false,
    );
    let view = list(dir, true);
    assert_eq!(view["authorship"]["confirmed"], false);
    assert_eq!(view["reviewState"]["approvedCount"], 0);
    let rfc = dir.join(format!("gov/rfc/{ID}/rfc.toml"));
    let source = fs::read_to_string(&rfc).unwrap();
    fs::write(rfc, source.replace("Portable test", "Revision two")).unwrap();
    let view = list(dir, true);
    assert!(
        view["reviews"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["validity"]["state"] == "stale")
    );
}

#[test]
fn code_approvals_survive_attached_records_but_not_source_changes() {
    let dir = project();
    let dir = dir.path();
    common::git_init_isolated(dir).unwrap();
    keyfiles(dir);
    // CLI input/key files are local test context, never repository content.
    fs::write(
        dir.join(".git/info/exclude"),
        "private.pem\npublic.pem\nactor.json\npayload.json\ntarget.json\n",
    )
    .unwrap();
    fs::create_dir(dir.join("src")).unwrap();
    fs::write(dir.join("src/code.txt"), "reviewed code\n").unwrap();
    let git = |args: &[&str]| {
        let result = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap().trim().to_owned()
    };
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "-qm",
        "Reviewed revision",
    ]);
    assert!(git(&["status", "--porcelain"]).is_empty());
    let mut code_target = target(dir);
    code_target["commit"] = json!(git(&["rev-parse", "HEAD"]));
    fs::write(
        dir.join("target.json"),
        serde_json::to_vec(&code_target).unwrap(),
    )
    .unwrap();
    let add_code = |kind: &str, who: Value, payload: Value, key: &str| {
        fs::write(dir.join("actor.json"), serde_json::to_vec(&who).unwrap()).unwrap();
        fs::write(
            dir.join("payload.json"),
            serde_json::to_vec(&payload).unwrap(),
        )
        .unwrap();
        success(run(
            dir,
            &[
                "collab",
                "add",
                ID,
                "--repo-id",
                REPO,
                "--type",
                kind,
                "--actor-file",
                "actor.json",
                "--payload-file",
                "payload.json",
                "--target-file",
                "target.json",
                "--sign-key-file",
                "private.pem",
                "--idempotency-key",
                key,
            ],
        ))
    };
    add_code(
        "authorship",
        actor("human:owner", "owner"),
        json!({"authors":[actor("agent:writer", "implementer")]}),
        "code-authors",
    );
    add_code(
        "review",
        actor("agent:reviewer", "reviewer"),
        json!({"scope":"code","verdict":"approved","reason":"Inspected this commit"}),
        "code-review",
    );
    assert_eq!(list(dir, true)["reviewState"]["approvedCount"], 1);
    add(
        dir,
        "comment",
        actor("agent:reviewer", "reviewer"),
        json!({"body":"An attached comment does not change the reviewed code"}),
        "code-comment",
        true,
    );
    assert!(!git(&["status", "--porcelain"]).is_empty());
    assert_eq!(list(dir, true)["reviewState"]["approvedCount"], 1);
    fs::write(dir.join("src/code.txt"), "unreviewed modification\n").unwrap();
    assert_eq!(list(dir, true)["reviewState"]["approvedCount"], 0);
    fs::write(dir.join("src/code.txt"), "reviewed code\n").unwrap();
    fs::write(dir.join("src/new.txt"), "untracked code\n").unwrap();
    assert_eq!(list(dir, true)["reviewState"]["approvedCount"], 0);
    fs::remove_file(dir.join("src/new.txt")).unwrap();
    assert_eq!(list(dir, true)["reviewState"]["approvedCount"], 1);
    // Governance outside collaboration remains part of the clean-worktree gate.
    fs::write(dir.join("gov/unreviewed.toml"), "unreviewed = true\n").unwrap();
    assert_eq!(list(dir, true)["reviewState"]["approvedCount"], 0);
}

#[test]
fn resolutions_require_a_trusted_authorized_actor_and_existing_reference() {
    let dir = project();
    let dir = dir.path();
    keyfiles(dir);
    let reviewer = actor("agent:reviewer", "reviewer");
    let review = add(
        dir,
        "review",
        reviewer.clone(),
        json!({"scope":"artifact","verdict":"changes_requested","reason":"Fix it"}),
        "review",
        true,
    );
    let payload = json!({"reviewId":review["event"]["eventId"],"reason":"Done"});
    add(
        dir,
        "resolution",
        actor("agent:other", "implementer"),
        payload.clone(),
        "unauthorized",
        true,
    );
    assert_eq!(list(dir, true)["reviewState"]["blockingCount"], 1);
    add(dir, "resolution", reviewer, payload, "resolved", true);
    assert_eq!(
        list(dir, true)["reviews"][0]["validity"]["state"],
        "resolved"
    );
    assert_eq!(list(dir, true)["reviewState"]["blockingCount"], 0);
    fs::write(
        dir.join("payload.json"),
        format!(
            r#"{{"body":"bad reply","replyTo":"ev_{}"}}"#,
            "0".repeat(64)
        ),
    )
    .unwrap();
    let bad = run(
        dir,
        &[
            "collab",
            "add",
            ID,
            "--repo-id",
            REPO,
            "--type",
            "comment",
            "--actor-file",
            "actor.json",
            "--payload-file",
            "payload.json",
            "--idempotency-key",
            "missing",
        ],
    );
    assert!(!bad.status.success());
}

#[test]
fn preview_and_concurrent_publication_do_not_overwrite_records() {
    let dir = project();
    let dir = dir.path();
    let (_, _, key) = keyfiles(dir);
    let bytes = serde_json::to_vec(&legacy(target(dir), &key, "race")).unwrap();
    let mut children = Vec::new();
    for _ in 0..4 {
        let mut child = command(dir, &["collab", "append", ID, "--repo-id", REPO, "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        children.push(child);
    }
    let receipts: Vec<_> = children
        .into_iter()
        .map(|child| success(child.wait_with_output().unwrap()))
        .collect();
    assert_eq!(
        receipts.iter().filter(|r| r["replayed"] == false).count(),
        1
    );
    assert_eq!(list(dir, false)["history"].as_array().unwrap().len(), 1);
    fs::write(
        dir.join("actor.json"),
        serde_json::to_vec(&actor("agent:dsh", "reviewer")).unwrap(),
    )
    .unwrap();
    fs::write(dir.join("payload.json"), r#"{"body":"preview only"}"#).unwrap();
    let preview = success(run(
        dir,
        &[
            "--dry-run",
            "collab",
            "add",
            ID,
            "--repo-id",
            REPO,
            "--type",
            "comment",
            "--actor-file",
            "actor.json",
            "--payload-file",
            "payload.json",
            "--idempotency-key",
            "preview",
        ],
    ));
    assert_eq!(preview["preview"], true);
    assert_eq!(list(dir, false)["history"].as_array().unwrap().len(), 1);
}

#[test]
fn malformed_stored_records_fail_check_without_becoming_authors_or_reviews() {
    let dir = project();
    let dir = dir.path();
    let path = dir.join(format!("gov/collaboration/rfc/{ID}"));
    fs::create_dir_all(&path).unwrap();
    fs::write(
        path.join(format!("ev_{}.json", "0".repeat(64))),
        b"not JSON",
    )
    .unwrap();
    let view = list(dir, false);
    assert_eq!(view["history"][0]["event"], Value::Null);
    assert_eq!(view["reviews"], json!([]));
    let checked = run(dir, &["check"]);
    assert!(!checked.status.success());
    assert!(String::from_utf8_lossy(&checked.stderr).contains("COLLAB_INVALID_EVENT"));
    assert!(
        !run(dir, &["collab", "target", "../RFC-0001", "--repo-id", REPO])
            .status
            .success()
    );
    let oversized = vec![b' '; 2 * 1024 * 1024 + 1];
    assert!(!append(dir, &oversized, false).status.success());
}

#[cfg(unix)]
#[test]
fn symlink_sources_and_record_directories_are_rejected() {
    use std::os::unix::fs::symlink;
    let dir = project();
    let dir = dir.path();
    let (_, _, key) = keyfiles(dir);
    let bytes = serde_json::to_vec(&legacy(target(dir), &key, "unsafe")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), dir.join("gov/collaboration")).unwrap();
    assert!(!append(dir, &bytes, false).status.success());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    fs::remove_file(dir.join("gov/collaboration")).unwrap();
    let source = dir.join(format!("gov/rfc/{ID}/rfc.toml"));
    let bytes = fs::read(&source).unwrap();
    fs::write(outside.path().join("rfc.toml"), bytes).unwrap();
    fs::remove_file(&source).unwrap();
    symlink(outside.path().join("rfc.toml"), source).unwrap();
    assert!(
        !run(dir, &["collab", "target", ID, "--repo-id", REPO])
            .status
            .success()
    );
}

#[test]
fn source_changes_while_stdin_is_pending_cannot_publish_a_current_review() {
    let dir = project();
    let dir = dir.path();
    let (_, _, key) = keyfiles(dir);
    let bytes = serde_json::to_vec(&legacy(target(dir), &key, "stdin-race")).unwrap();
    let mut child = command(dir, &["collab", "append", ID, "--repo-id", REPO, "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Wait for the writer lock, acquired before target capture and stdin reads.
    // Keeping stdin open makes this a deterministic source-change window.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.join(".govctl/gov.lock"))
        .unwrap();
    loop {
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Ok(()) => {
                fs2::FileExt::unlock(&lock).unwrap();
            }
            Err(error) => panic!("cannot inspect writer lock: {error}"),
        }
        assert!(
            std::time::Instant::now() < deadline,
            "append did not acquire its writer lock"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::thread::sleep(std::time::Duration::from_millis(150));
    let rfc = dir.join(format!("gov/rfc/{ID}/rfc.toml"));
    let source = fs::read_to_string(&rfc).unwrap();
    fs::write(
        &rfc,
        source.replace("Portable test", "Changed while reading stdin"),
    )
    .unwrap();
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("COLLAB_SOURCE_CHANGED"));
    assert!(!dir.join("gov/collaboration").exists());
}

#[test]
fn malformed_creator_metadata_is_rejected_before_projection() {
    let dir = project();
    let dir = dir.path();
    let rfc = dir.join(format!("gov/rfc/{ID}/rfc.toml"));
    let source = fs::read_to_string(&rfc).unwrap();
    fs::write(
        &rfc,
        source.replace(
            "[govctl]",
            "[govctl]\ncreated_by = { agentId = \"reviewer\" }",
        ),
    )
    .unwrap();
    let result = run(dir, &["collab", "list", ID, "--repo-id", REPO]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("COLLAB_INVALID_SOURCE"));
}
