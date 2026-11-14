//! Isolated bare transport; no caller worktree, checkout filters or client hooks.
//! [[RFC-0012:C-PUBLICATION]], [[RFC-0012:C-HISTORY]].
use super::{failure, model, policy::Context};
use crate::{
    collaboration::{Target, source},
    config::Config,
    diagnostic::DiagnosticResult,
};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tempfile::{NamedTempFile, TempDir};

const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

pub(super) struct Output {
    pub success: bool,
    pub timed_out: bool,
    pub stdout: Vec<u8>,
}

pub(super) struct Repository {
    _root: TempDir,
    path: PathBuf,
    hooks: PathBuf,
    index: PathBuf,
    remote: String,
    source_cache: BTreeMap<String, BTreeMap<String, Target>>,
}

fn io_failure(_: std::io::Error) -> crate::diagnostic::Diagnostic {
    failure(
        "COORD_GIT_FAILED",
        "Cannot access the isolated publication repository.",
    )
}

impl Repository {
    pub fn open(context: &Context) -> DiagnosticResult<Self> {
        let root = tempfile::tempdir().map_err(io_failure)?;
        let path = root.path().join("repository.git");
        let hooks = root.path().join("empty-hooks");
        fs::create_dir(&path).map_err(io_failure)?;
        fs::create_dir(&hooks).map_err(io_failure)?;
        let repo = Self {
            index: root.path().join("publication.index"),
            path,
            hooks,
            remote: context.remote.clone(),
            _root: root,
            source_cache: BTreeMap::new(),
        };
        let template = repo.hooks.to_string_lossy().into_owned();
        repo.text(&["init", "--bare", "--template", &template], None)?;
        Ok(repo)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command
            .current_dir(&self.path)
            .arg("--git-dir")
            .arg(&self.path)
            .args([
                "-c",
                "core.bare=true",
                "-c",
                "commit.gpgSign=false",
                "-c",
                "tag.gpgSign=false",
            ])
            .arg("-c")
            .arg(format!("core.hooksPath={}", self.hooks.display()))
            .args([
                "-c",
                "protocol.allow=never",
                "-c",
                "protocol.file.allow=always",
                "-c",
                "protocol.https.allow=always",
                "-c",
                "protocol.ssh.allow=always",
                "-c",
                "fetch.recurseSubmodules=false",
            ])
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_INDEX_FILE", &self.index)
            .env("GIT_AUTHOR_NAME", "govctl coordination")
            .env("GIT_AUTHOR_EMAIL", "govctl@localhost")
            .env("GIT_COMMITTER_NAME", "govctl coordination")
            .env("GIT_COMMITTER_EMAIL", "govctl@localhost");
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
            "GIT_REPLACE_REF_BASE",
            "GIT_SHALLOW_FILE",
        ] {
            command.env_remove(key);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command
    }

    pub fn run(&self, args: &[&str], input: Option<&[u8]>) -> DiagnosticResult<Output> {
        let mut capture = NamedTempFile::new().map_err(io_failure)?;
        let errors = NamedTempFile::new().map_err(io_failure)?;
        let mut command = self.command(args);
        command
            .stdout(Stdio::from(capture.reopen().map_err(io_failure)?))
            .stderr(Stdio::from(errors.reopen().map_err(io_failure)?))
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        let mut child = command.spawn().map_err(io_failure)?;
        if let Some(bytes) = input
            && let Some(mut stdin) = child.stdin.take()
        {
            stdin.write_all(bytes).map_err(io_failure)?;
        }
        let started = Instant::now();
        let (status, timed_out) = loop {
            if let Some(status) = child.try_wait().map_err(io_failure)? {
                break (status, false);
            }
            if started.elapsed() >= TIMEOUT {
                #[cfg(unix)]
                {
                    unsafe extern "C" {
                        fn kill(pid: i32, sig: i32) -> i32;
                    }
                    if let Ok(pid) = i32::try_from(child.id()) {
                        // The child starts in its own process group, never the caller's group.
                        unsafe {
                            kill(-pid, 9);
                        }
                    }
                }
                let _ = child.kill();
                break (child.wait().map_err(io_failure)?, true);
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        capture
            .as_file_mut()
            .seek(SeekFrom::Start(0))
            .map_err(io_failure)?;
        let mut stdout = Vec::new();
        capture
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut stdout)
            .map_err(io_failure)?;
        if stdout.len() as u64 > OUTPUT_LIMIT {
            return Err(failure(
                "COORD_GIT_FAILED",
                "Repository output exceeds the bounded transport limit.",
            ));
        }
        Ok(Output {
            success: status.success(),
            timed_out,
            stdout,
        })
    }

    pub fn text(&self, args: &[&str], input: Option<&[u8]>) -> DiagnosticResult<String> {
        let output = self.run(args, input)?;
        if !output.success {
            return Err(failure(
                "COORD_GIT_FAILED",
                if output.timed_out {
                    "The canonical repository operation timed out."
                } else {
                    "The canonical repository operation failed."
                },
            ));
        }
        String::from_utf8(output.stdout)
            .map(|s| s.trim().to_owned())
            .map_err(|_| failure("COORD_GIT_FAILED", "Git returned invalid UTF-8."))
    }

    pub fn fetch(&self) -> DiagnosticResult<String> {
        self.text(
            &[
                "fetch",
                "--quiet",
                "--prune",
                "--no-recurse-submodules",
                &self.remote,
                "+refs/heads/*:refs/remotes/canonical/*",
                "+refs/tags/*:refs/tags/*",
            ],
            None,
        )?;
        let head = self.text(&["rev-parse", "refs/remotes/canonical/main"], None)?;
        model::commit(&head)?;
        Ok(head)
    }

    pub fn first_parent(&self, head: &str) -> DiagnosticResult<Vec<String>> {
        Ok(self
            .text(&["rev-list", "--first-parent", "--reverse", head], None)?
            .lines()
            .map(str::to_owned)
            .collect())
    }

    pub fn published_commit(&self, commit: &str) -> DiagnosticResult<bool> {
        model::commit(commit)?;
        let object = self.run(&["cat-file", "-t", commit], None)?;
        if !object.success || object.stdout != b"commit\n" {
            return Ok(false);
        }
        Ok(!self
            .text(
                &[
                    "for-each-ref",
                    "--format=%(refname)",
                    &format!("--contains={commit}"),
                    "refs/remotes/canonical/",
                    "refs/tags/",
                ],
                None,
            )?
            .is_empty())
    }

    pub fn blob(&self, revision: &str, path: &str) -> DiagnosticResult<Vec<u8>> {
        let output = self.run(&["show", &format!("{revision}:{path}")], None)?;
        if !output.success {
            return Err(failure(
                "COORD_HISTORY_INVALID",
                "A referenced repository record is unavailable.",
            ));
        }
        Ok(output.stdout)
    }

    pub fn targets(
        &mut self,
        revision: &str,
        repo_id: &str,
    ) -> DiagnosticResult<BTreeMap<String, Target>> {
        let trees = self.text(
            &["ls-tree", revision, "--", "gov/rfc", "gov/adr", "gov/work"],
            None,
        )?;
        if let Some(targets) = self.source_cache.get(&trees) {
            return Ok(targets.clone());
        }
        let snapshot = tempfile::tempdir().map_err(io_failure)?;
        let gov = snapshot.path().join("gov");
        fs::create_dir(&gov).map_err(io_failure)?;
        let output = self.run(
            &[
                "ls-tree", "-r", "-z", revision, "--", "gov/rfc", "gov/adr", "gov/work",
            ],
            None,
        )?;
        if !output.success {
            return Err(failure(
                "COORD_HISTORY_INVALID",
                "Cannot inspect canonical source files.",
            ));
        }
        let mut total = 0usize;
        for entry in output.stdout.split(|b| *b == 0).filter(|e| !e.is_empty()) {
            total += 1;
            if total > 10_000 {
                return Err(failure(
                    "COORD_UNSUPPORTED",
                    "Too many governance source files.",
                ));
            }
            let entry = std::str::from_utf8(entry).map_err(|_| {
                failure(
                    "COORD_HISTORY_INVALID",
                    "Non-UTF-8 source paths are unsupported.",
                )
            })?;
            let (meta, path) = entry
                .split_once('\t')
                .ok_or_else(|| failure("COORD_HISTORY_INVALID", "Invalid source tree entry."))?;
            let mut fields = meta.split_whitespace();
            let mode = fields.next();
            let kind = fields.next();
            let object = fields
                .next()
                .ok_or_else(|| failure("COORD_HISTORY_INVALID", "Missing source object."))?;
            if !matches!(mode, Some("100644" | "100755")) || kind != Some("blob") {
                return Err(failure(
                    "COORD_HISTORY_INVALID",
                    "Governance source paths must be regular files.",
                ));
            }
            let relative = path
                .strip_prefix("gov/")
                .ok_or_else(|| failure("COORD_HISTORY_INVALID", "Source escaped the gov root."))?;
            let destination = source::safe_path(&gov, relative, true)?;
            let blob = self.run(&["cat-file", "blob", object], None)?;
            if !blob.success || blob.stdout.len() > 4 * 1024 * 1024 {
                return Err(failure(
                    "COORD_HISTORY_INVALID",
                    "Governance source blob is unavailable or too large.",
                ));
            }
            fs::write(destination, blob.stdout).map_err(io_failure)?;
        }
        let config = Config {
            gov_root: gov,
            ..Config::default()
        };
        let mut targets = BTreeMap::new();
        for source in source::sources(&config, repo_id)? {
            if targets
                .insert(source.target.id.clone(), source.target)
                .is_some()
            {
                return Err(failure(
                    "COORD_HISTORY_INVALID",
                    "Duplicate canonical artifact identity.",
                ));
            }
        }
        self.source_cache.insert(trees, targets.clone());
        Ok(targets)
    }

    pub fn candidate(&self, parent: &str, path: &str, bytes: &[u8]) -> DiagnosticResult<String> {
        self.text(&["read-tree", parent], None)?;
        let blob = self.text(&["hash-object", "-w", "--stdin"], Some(bytes))?;
        self.text(
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "100644",
                &blob,
                path,
            ],
            None,
        )?;
        let tree = self.text(&["write-tree"], None)?;
        let commit = self.text(
            &["commit-tree", &tree, "-p", parent],
            Some(b"Publish govctl coordination operation\n"),
        )?;
        let changed = self.text(
            &[
                "diff-tree",
                "--no-commit-id",
                "--name-status",
                "-r",
                parent,
                &commit,
            ],
            None,
        )?;
        if changed != format!("A\t{path}") {
            return Err(failure(
                "COORD_HISTORY_INVALID",
                "A control publication must add exactly its one record.",
            ));
        }
        Ok(commit)
    }

    pub fn push(&self, commit: &str) -> DiagnosticResult<Output> {
        self.run(
            &[
                "push",
                "--porcelain",
                &self.remote,
                &format!("{commit}:refs/heads/main"),
            ],
            None,
        )
    }
}

pub(super) fn record_path(target: &Target, event_id: &str) -> String {
    format!(
        "gov/collaboration/{}/{}/{}.json",
        target.kind, target.id, event_id
    )
}

pub(super) fn read_external(path: &Path, limit: u64) -> DiagnosticResult<Vec<u8>> {
    source::read_bounded(path, limit)
}
