use dcc_core::{CacheError, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Structured output captured from process execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub execution_time_ms: u64,
    pub timed_out: bool,
}

/// Cross-platform process executor supporting Windows & Unix execution semantics,
/// environment isolation, executable discovery, timeout termination, and stream capture (Milestone 12.2).
pub struct ProcessExecutor;

impl ProcessExecutor {
    /// Discover an executable on PATH or as a relative/absolute path.
    /// On Windows, checks standard extensions (.exe, .cmd, .bat) if missing.
    pub fn discover_executable(command: &str) -> Option<PathBuf> {
        let cmd_path = Path::new(command);

        // If it's already an existing file path (absolute or relative)
        if cmd_path.is_file() {
            return Some(cmd_path.to_path_buf());
        }

        #[cfg(windows)]
        {
            if cmd_path.extension().is_none() {
                for ext in &["exe", "cmd", "bat"] {
                    let with_ext = cmd_path.with_extension(ext);
                    if with_ext.is_file() {
                        return Some(with_ext);
                    }
                }
            }
        }

        // Search in system PATH
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let candidate = dir.join(command);
                if candidate.is_file() {
                    return Some(candidate);
                }

                #[cfg(windows)]
                {
                    if candidate.extension().is_none() {
                        for ext in &["exe", "cmd", "bat"] {
                            let with_ext = candidate.with_extension(ext);
                            if with_ext.is_file() {
                                return Some(with_ext);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Execute a command with full cross-platform process isolation and lifecycle handling.
    pub fn execute(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        working_dir: Option<&Path>,
    ) -> Result<ProcessOutput> {
        Self::execute_with_timeout(command, args, env, working_dir, None)
    }

    /// Execute a command with an optional timeout duration.
    /// If the timeout expires, the spawned process is forcefully terminated.
    pub fn execute_with_timeout(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        working_dir: Option<&Path>,
        timeout: Option<Duration>,
    ) -> Result<ProcessOutput> {
        let mut cmd = Command::new(command);
        cmd.args(args);

        // Explicit environment handling
        for (k, v) in env {
            cmd.env(k, v);
        }

        if let Some(wd) = working_dir {
            cmd.current_dir(wd);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let start = Instant::now();

        let mut child: Child = cmd.spawn().map_err(|e| {
            CacheError::ConfigurationError(format!("Failed to spawn process '{}': {}", command, e))
        })?;

        let (timed_out, exit_code) = match timeout {
            Some(limit) => {
                let poll_interval = Duration::from_millis(10);
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            break (false, status.code().unwrap_or(-1));
                        }
                        Ok(None) => {
                            if start.elapsed() >= limit {
                                let _ = child.kill();
                                break (true, -1);
                            }
                            std::thread::sleep(poll_interval);
                        }
                        Err(e) => {
                            let _ = child.kill();
                            return Err(CacheError::ConfigurationError(format!(
                                "Error waiting for process '{}': {}",
                                command, e
                            )));
                        }
                    }
                }
            }
            None => {
                let status = child.wait().map_err(|e| {
                    CacheError::ConfigurationError(format!(
                        "Failed waiting for process '{}': {}",
                        command, e
                    ))
                })?;
                (false, status.code().unwrap_or(-1))
            }
        };

        let elapsed = start.elapsed().as_millis() as u64;

        // Collect stdout & stderr streams
        let mut stdout = Vec::new();
        if let Some(mut out) = child.stdout.take() {
            let _ = std::io::Read::read_to_end(&mut out, &mut stdout);
        }

        let mut stderr = Vec::new();
        if let Some(mut err) = child.stderr.take() {
            let _ = std::io::Read::read_to_end(&mut err, &mut stderr);
        }

        Ok(ProcessOutput {
            exit_code,
            stdout,
            stderr,
            execution_time_ms: elapsed,
            timed_out,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executable_discovery() {
        #[cfg(windows)]
        {
            let res = ProcessExecutor::discover_executable("cmd");
            assert!(res.is_some());
            let path = res.unwrap();
            assert!(path.to_string_lossy().to_lowercase().contains("cmd"));
        }

        #[cfg(not(windows))]
        {
            let res = ProcessExecutor::discover_executable("sh");
            assert!(res.is_some());
        }
    }

    #[test]
    fn test_process_execution_stdout_stderr_and_exit_code() {
        #[cfg(windows)]
        let (cmd, args) = (
            "cmd.exe",
            vec!["/C".to_string(), "echo process output payload".to_string()],
        );

        #[cfg(not(windows))]
        let (cmd, args) = (
            "sh",
            vec!["-c".to_string(), "echo process output payload".to_string()],
        );

        let env = BTreeMap::new();
        let res = ProcessExecutor::execute(cmd, &args, &env, None).unwrap();

        assert_eq!(res.exit_code, 0);
        assert!(!res.timed_out);
        let out_str = String::from_utf8_lossy(&res.stdout);
        assert!(out_str.contains("process output payload"));
    }

    #[test]
    fn test_process_environment_handling() {
        #[cfg(windows)]
        let (cmd, args) = (
            "cmd.exe",
            vec!["/C".to_string(), "echo %DCC_CUSTOM_ENV_VAR%".to_string()],
        );

        #[cfg(not(windows))]
        let (cmd, args) = (
            "sh",
            vec!["-c".to_string(), "echo $DCC_CUSTOM_ENV_VAR".to_string()],
        );

        let mut env = BTreeMap::new();
        env.insert(
            "DCC_CUSTOM_ENV_VAR".to_string(),
            "custom_env_value_123".to_string(),
        );

        let res = ProcessExecutor::execute(cmd, &args, &env, None).unwrap();
        assert_eq!(res.exit_code, 0);
        let out_str = String::from_utf8_lossy(&res.stdout);
        assert!(out_str.contains("custom_env_value_123"));
    }

    #[test]
    fn test_process_timeout_and_termination() {
        #[cfg(windows)]
        let (cmd, args) = (
            "powershell.exe",
            vec![
                "-Command".to_string(),
                "Start-Sleep -Milliseconds 1500".to_string(),
            ],
        );

        #[cfg(not(windows))]
        let (cmd, args) = ("sleep", vec!["1.5".to_string()]);

        let env = BTreeMap::new();
        let timeout = Duration::from_millis(150);

        let res =
            ProcessExecutor::execute_with_timeout(cmd, &args, &env, None, Some(timeout)).unwrap();
        assert!(res.timed_out);
    }
}
