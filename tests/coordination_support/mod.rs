#![allow(dead_code)]
use ed25519_dalek::{
    SigningKey,
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

pub const REPO: &str = "test/coordination";

pub fn actor(subject: &str) -> Value {
    json!({"subjectId":subject,"name":subject,"kind":"agent"})
}

pub fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

pub fn rejected(output: Output, marker: &str) {
    assert!(
        !output.status.success(),
        "Unexpected success: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(marker),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn git_output(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap()
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = git_output(dir, args);
    assert!(
        output.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

pub struct Fixture {
    pub root: tempfile::TempDir,
    pub seed: tempfile::TempDir,
    pub server: PathBuf,
    pub a: PathBuf,
    pub b: PathBuf,
    pub context: PathBuf,
    pub key: PathBuf,
    pub key_id: String,
    pub public_pem: String,
    pub initial_head: String,
}

impl Fixture {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let seed = crate::common::init_project().unwrap();
        crate::common::write_minimal_rfc(seed.path(), "RFC-0001", "First artifact").unwrap();
        crate::common::write_minimal_rfc(seed.path(), "RFC-0002", "Second artifact").unwrap();
        fs::write(seed.path().join("source.txt"), "canonical source\n").unwrap();
        git(seed.path(), &["init", "-q", "--initial-branch=main"]);
        git(seed.path(), &["add", "."]);
        git(
            seed.path(),
            &[
                "-c",
                "commit.gpgSign=false",
                "commit",
                "-qm",
                "Initial canonical sources",
            ],
        );
        let initial_head = git(seed.path(), &["rev-parse", "HEAD"]);
        let server = root.path().join("server.git");
        git(
            root.path(),
            &[
                "init",
                "--bare",
                "--initial-branch=main",
                server.to_str().unwrap(),
            ],
        );
        git(
            seed.path(),
            &["remote", "add", "origin", server.to_str().unwrap()],
        );
        git(seed.path(), &["push", "-q", "origin", "main"]);
        let a = root.path().join("clone-a");
        let b = root.path().join("clone-b");
        for clone in [&a, &b] {
            git(
                root.path(),
                &[
                    "clone",
                    "-q",
                    server.to_str().unwrap(),
                    clone.to_str().unwrap(),
                ],
            );
        }
        let key = root.path().join("issuer.pem");
        let signing = SigningKey::from_bytes(&[19; 32]);
        fs::write(
            &key,
            signing.to_pkcs8_pem(LineEnding::LF).unwrap().as_bytes(),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let public_pem = signing
            .verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        let key_id = format!("{:x}", Sha256::digest(public_pem.as_bytes()));
        let context = root.path().join("context.json");
        let grants: Vec<_> = ["owner", "agent:a", "agent:b", "agent:c"]
            .into_iter()
            .map(|subject| {
                let actions = if subject == "owner" {
                    vec![
                        "publish", "claim", "handoff", "accept", "release", "cancel", "takeover",
                        "complete", "delegate",
                    ]
                } else {
                    vec!["claim", "handoff", "accept", "release", "complete"]
                };
                json!({"subjectId":subject,"issuerKeyId":key_id,"actions":actions})
            })
            .collect();
        fs::write(
            &context,
            serde_json::to_vec_pretty(&json!({
                "protocolVersion":1,"repoId":REPO,"remote":server,"branch":"main",
                "authorityId":"test-authority","currentPolicyRevision":"v1",
                "policies":{"v1":{"effectiveAfterCommit":initial_head,
                    "trustedIssuers":[{"keyId":key_id,"publicKeyPem":public_pem}],"grants":grants}}
            }))
            .unwrap(),
        )
        .unwrap();
        Self {
            root,
            seed,
            server,
            a,
            b,
            context,
            key,
            key_id,
            public_pem,
            initial_head,
        }
    }

    pub fn command(&self, dir: &Path, action: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_govctl"));
        command
            .args(["coord", action, "--context-file"])
            .arg(&self.context)
            .current_dir(dir)
            .env("NO_COLOR", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null");
        command
    }

    pub fn list(&self, dir: &Path) -> Value {
        success(self.command(dir, "list").output().unwrap())
    }

    pub fn apply_command(&self, dir: &Path) -> Command {
        let mut command = self.command(dir, "apply");
        command.arg("--sign-key-file").arg(&self.key).arg("--stdin");
        command
    }

    pub fn run_apply(mut command: Command, operation: &Value) -> Output {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(operation).unwrap())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    pub fn apply_output(&self, dir: &Path, operation: &Value) -> Output {
        Self::run_apply(self.apply_command(dir), operation)
    }

    pub fn apply(&self, dir: &Path, operation: &Value) -> Value {
        success(self.apply_output(dir, operation))
    }

    pub fn publish_operation(
        &self,
        request_id: &str,
        artifact_id: &str,
        kind: &str,
        write_targets: &[&str],
    ) -> Value {
        let snapshot = self.list(&self.a);
        let target = snapshot["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|artifact| artifact["target"]["id"] == artifact_id)
            .unwrap()["target"]
            .clone();
        json!({"operationId":format!("publish-{request_id}"),"action":"publish","actor":actor("owner"),
            "requestId":request_id,"target":target,"requestedOperation":kind,
            "writeTargets":write_targets,"description":format!("Execute {request_id}")})
    }

    pub fn publish(
        &self,
        request_id: &str,
        artifact_id: &str,
        kind: &str,
        write_targets: &[&str],
    ) -> Value {
        self.apply(
            &self.a,
            &self.publish_operation(request_id, artifact_id, kind, write_targets),
        )["request"]
            .clone()
    }

    pub fn operation(action: &str, operation_id: &str, subject: &str, request: &Value) -> Value {
        json!({"operationId":operation_id,"action":action,"actor":actor(subject),
            "requestId":request["requestId"],"expectedState":request["stateEventId"],
            "expectedGeneration":request["generation"],"expectedWrites":{}})
    }

    pub fn claim_operation(operation_id: &str, subject: &str, request: &Value) -> Value {
        let mut operation = Self::operation("claim", operation_id, subject, request);
        operation["holder"] = json!({"subjectId":subject,"claimId":format!("claim-{operation_id}"),"sessionId":format!("run-{operation_id}")});
        operation
    }

    pub fn held_operation(action: &str, operation_id: &str, request: &Value) -> Value {
        let subject = request["holder"]["subjectId"].as_str().unwrap();
        let mut operation = Self::operation(action, operation_id, subject, request);
        operation["claimId"] = request["holder"]["claimId"].clone();
        operation["sessionId"] = request["holder"]["sessionId"].clone();
        operation
    }

    pub fn head(&self) -> String {
        git(&self.server, &["rev-parse", "main"])
    }

    pub fn context_value(&self) -> Value {
        serde_json::from_slice(&fs::read(&self.context).unwrap()).unwrap()
    }

    pub fn write_context(&self, value: &Value) {
        fs::write(&self.context, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    pub fn sync_seed(&self) {
        git(self.seed.path(), &["fetch", "-q", "origin"]);
        git(self.seed.path(), &["reset", "--hard", "origin/main"]);
    }

    pub fn push_seed(&self, message: &str) {
        git(self.seed.path(), &["add", "."]);
        git(
            self.seed.path(),
            &["-c", "commit.gpgSign=false", "commit", "-qm", message],
        );
        git(self.seed.path(), &["push", "-q", "origin", "main"]);
    }
}
