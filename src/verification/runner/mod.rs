use crate::config::Config;
use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::model::GuardEntry;
use regex::RegexBuilder;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

mod capture;
mod process_group;

use capture::GuardOutputCapture;
use process_group::{
    configure_guard_process_group, guard_process_group, terminate_guard_process,
    terminate_guard_process_group,
};

pub const DEFAULT_GUARD_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone)]
pub struct GuardRunResult {
    pub id: String,
    pub passed: bool,
    pub timed_out: bool,
    pub primary_shell_running_at_timeout: Option<bool>,
    pub output: String,
}

/// Prefer an installed non-WSL Bash so the WSL launcher cannot shadow it.
fn windows_bash_program(path: Option<&OsStr>) -> OsString {
    let Some(path) = path else {
        return OsString::from("bash");
    };
    for directory in std::env::split_paths(path) {
        let candidate = directory.join("bash.exe");
        if candidate.is_file() && !is_wsl_bash(&candidate) {
            return candidate.into_os_string();
        }
    }
    OsString::from("bash")
}

fn is_wsl_bash(path: &Path) -> bool {
    let path = path.to_string_lossy().to_lowercase().replace('/', "\\");
    path.contains(r"\windows\system32\bash.exe")
        || path.contains(r"\windows\syswow64\bash.exe")
        || path.contains(r"\windowsapps\bash.exe")
}

fn guard_shell(command_text: &str, windows: bool) -> Command {
    let program = if windows {
        windows_bash_program(std::env::var_os("PATH").as_deref())
    } else {
        OsString::from("/bin/bash")
    };
    let mut command = Command::new(program);
    command.args(["-c", command_text]);
    command
}

pub fn run_guard(config: &Config, guard: &GuardEntry) -> Result<GuardRunResult, Diagnostic> {
    let project_root = config
        .gov_root
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| config.gov_root.clone());
    let timeout = if guard.spec.check.timeout_secs == 0 {
        DEFAULT_GUARD_TIMEOUT_SECS
    } else {
        guard.spec.check.timeout_secs
    };

    let mut stdout_capture = GuardOutputCapture::new(guard, "stdout")?;
    let mut stderr_capture = GuardOutputCapture::new(guard, "stderr")?;

    // Non-login shell: inherit the caller's environment unchanged instead of
    // re-sourcing profile files [[RFC-0000:C-GUARD-DEF]], [[ADR-0064]].
    let mut command = guard_shell(&guard.spec.check.command, cfg!(windows));
    command
        .current_dir(project_root)
        .stdout(stdout_capture.stdio(guard, "stdout")?)
        .stderr(stderr_capture.stdio(guard, "stderr")?);
    configure_guard_process_group(&mut command);

    let mut child = command.spawn().map_err(|err| {
        Diagnostic::new(
            DiagnosticCode::E1004GuardCheckFailed,
            format!(
                "Failed to start verification guard '{}': {}",
                guard.meta().id,
                err
            ),
            guard.path.display().to_string(),
        )
    })?;
    let child_id = child.id();
    let process_group = guard_process_group(child_id);

    let deadline = Duration::from_secs(timeout);
    let started = Instant::now();
    let mut timed_out = false;
    let mut primary_shell_running_at_timeout = None;
    let status: ExitStatus;

    loop {
        match child.try_wait() {
            Ok(Some(exit_status)) => {
                status = exit_status;
                terminate_guard_process_group(process_group);
                break;
            }
            Ok(None) if started.elapsed() < deadline => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Ok(None) => {
                timed_out = true;
                primary_shell_running_at_timeout = Some(true);
                terminate_guard_process(&mut child, process_group);
                status = child.wait().map_err(|err| {
                    Diagnostic::new(
                        DiagnosticCode::E1004GuardCheckFailed,
                        format!(
                            "Failed while waiting on timed-out verification guard '{}': {}",
                            guard.meta().id,
                            err
                        ),
                        guard.path.display().to_string(),
                    )
                })?;
                break;
            }
            Err(err) => {
                return Err(Diagnostic::new(
                    DiagnosticCode::E1004GuardCheckFailed,
                    format!(
                        "Failed while waiting on verification guard '{}': {}",
                        guard.meta().id,
                        err
                    ),
                    guard.path.display().to_string(),
                ));
            }
        }
    }

    let stdout = stdout_capture.read(guard, "stdout")?;
    let stderr = stderr_capture.read(guard, "stderr")?;
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );

    let pattern_matched = match &guard.spec.check.pattern {
        Some(pattern) => RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .map_err(|err| {
                Diagnostic::new(
                    DiagnosticCode::E1001GuardSchemaInvalid,
                    format!(
                        "Invalid regex pattern for guard '{}': {}",
                        guard.meta().id,
                        err
                    ),
                    guard.path.display().to_string(),
                )
            })?
            .is_match(&combined_output),
        None => true,
    };

    Ok(GuardRunResult {
        id: guard.meta().id.clone(),
        passed: !timed_out && status.success() && pattern_matched,
        timed_out,
        primary_shell_running_at_timeout,
        output: combined_output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_shell_uses_platform_bash_without_login_arguments() {
        assert_eq!(guard_shell("printf ok", false).get_program(), "/bin/bash");
        for windows in [false, true] {
            let command = guard_shell("printf ok", windows);
            let arguments = command
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            assert_eq!(arguments, ["-c", "printf ok"]);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_bash_program_prefers_non_wsl_bash() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let wsl = temp.path().join("Windows/System32");
        let git = temp.path().join("Git/bin");
        std::fs::create_dir_all(&wsl)?;
        std::fs::create_dir_all(&git)?;
        std::fs::write(wsl.join("bash.exe"), [])?;
        let git_bash = git.join("bash.exe");
        std::fs::write(&git_bash, [])?;
        let path = std::env::join_paths([wsl, git])?;

        assert_eq!(windows_bash_program(Some(&path)), git_bash);
        Ok(())
    }
}
