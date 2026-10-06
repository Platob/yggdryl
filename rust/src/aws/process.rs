//! The `credential_process` a profile names.
//!
//! A profile may say `credential_process = /usr/local/bin/vault-keys`, and
//! the AWS tools run that line the way a shell would split it and read one
//! JSON document off its standard output:
//! `Version` 1, `AccessKeyId`, `SecretAccessKey`, `SessionToken` and
//! `Expiration`. The process is what every external secret store integrates
//! through, so the document is read by the same reader every other credential
//! document is. A process runs under a bound its caller states - a helper
//! that hangs is killed at it rather than hanging every request behind it -
//! or under none, which is botocore's own rule. It is handed this process's
//! standard input only where that is a terminal, so a helper that prompts
//! (`aws-vault --prompt=terminal`) can ask the person at it, and one that
//! reads anything else reads nothing rather than waiting on a pipe.

use std::io::IsTerminal;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::credentials::{self, Credentials};
use crate::{Error, Result};

/// The most of a process's standard error a refusal quotes.
const MAX_STDERR: usize = 512;
/// The bound a process runs under where its caller states no other.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// How often the process is looked at while it runs.
const POLL: Duration = Duration::from_millis(20);

/// Run `command` under [`DEFAULT_TIMEOUT`] and read the credential set it
/// prints.
///
/// # Errors
///
/// What [`run_with`] refuses.
pub(crate) fn run(command: &str) -> Result<Credentials> {
    run_with(command, Some(DEFAULT_TIMEOUT))
}

/// Run `command` and read the credential set it prints, killing it once it
/// has run for `timeout`; `None` waits for it however long it takes.
///
/// # Errors
///
/// An empty command line, a program that cannot be started, one that exits
/// non-zero (its standard error quoted) or does not exit within `timeout`
/// (the bound and the program named), or output that is not a credential
/// document.
pub(crate) fn run_with(command: &str, timeout: Option<Duration>) -> Result<Credentials> {
    let words = super::profile::split_command(command, cfg!(windows));
    let Some((program, arguments)) = words.split_first() else {
        return Err(refusal("credential_process names no program"));
    };
    let stdin = if std::io::stdin().is_terminal() {
        Stdio::inherit()
    } else {
        Stdio::null()
    };
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            refusal(format!(
                "could not run credential_process {program}: {error}"
            ))
        })?;
    // The pipes are drained on threads so a helper that writes more than a
    // pipe holds cannot block on us while we wait on it.
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => match timeout {
                Some(bound) if started.elapsed() >= bound => {
                    let _ = child.kill();
                    let _ = child.wait();
                    // The drain threads are left to end with the pipes: a
                    // grandchild the helper started may hold them open.
                    return Err(refusal(format!(
                        "credential_process {program} did not exit within {bound:?}, so it was killed"
                    )));
                }
                _ => std::thread::sleep(POLL),
            },
            Err(error) => {
                return Err(refusal(format!(
                    "could not wait for credential_process {program}: {error}"
                )));
            }
        }
    };
    let stdout = stdout
        .and_then(|thread| thread.join().ok())
        .unwrap_or_default();
    let stderr = stderr
        .and_then(|thread| thread.join().ok())
        .unwrap_or_default();
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        let stderr = stderr.trim();
        let end = stderr
            .char_indices()
            .map(|(index, _)| index)
            .find(|index| *index >= MAX_STDERR)
            .unwrap_or(stderr.len());
        return Err(refusal(format!(
            "credential_process {program} exited with {status}: {}",
            &stderr[..end]
        )));
    }
    credentials::parse_document(&stdout, "credential_process")
}

/// Read one of the child's pipes to its end, on a thread of its own.
fn drain(mut pipe: impl std::io::Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    })
}

fn refusal(message: impl Into<String>) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        message.into(),
    ))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/aws/process.rs` pins and a caller cannot reach: a
    //! process under a bound of the test's choosing, which no session states
    //! below the default.

    use std::time::Duration;

    use crate::aws::Credentials;

    /// Run `command` under `timeout` (`None` unbounded) and read the set it
    /// prints.
    ///
    /// # Errors
    ///
    /// What `run_with` refuses.
    pub fn run_with(command: &str, timeout: Option<Duration>) -> crate::Result<Credentials> {
        super::run_with(command, timeout)
    }
}
