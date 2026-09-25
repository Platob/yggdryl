//! The `credential_process` a profile names.
//!
//! A profile may say `credential_process = /usr/local/bin/vault-keys`, and
//! the AWS tools run that line the way a shell would split it and read one
//! JSON document off its standard output:
//! `Version` 1, `AccessKeyId`, `SecretAccessKey`, `SessionToken` and
//! `Expiration`. The process is what every external secret store integrates
//! through, so the document is read by the same reader every other credential
//! document is - and a helper that hangs is killed after a minute rather than
//! hanging every request behind it. It is given no standard input, so a
//! helper that prompts fails at once rather than waiting on a terminal
//! nobody is at.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::credentials::{self, Credentials};
use crate::{Error, Result};

/// The most of a process's standard error a refusal quotes.
const MAX_STDERR: usize = 512;
/// How long a process is given before it is killed and reported.
const TIMEOUT: Duration = Duration::from_secs(60);
/// How often the process is looked at while it runs.
const POLL: Duration = Duration::from_millis(20);

/// Run `command` and read the credential set it prints.
///
/// # Errors
///
/// An empty command line, a program that cannot be started, one that exits
/// non-zero (its standard error quoted) or does not exit within a minute, or
/// output that is not a credential document.
pub(crate) fn run(command: &str) -> Result<Credentials> {
    let words = super::profile::split_words(command);
    let Some((program, arguments)) = words.split_first() else {
        return Err(refusal("credential_process names no program"));
    };
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
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
            Ok(None) if started.elapsed() < TIMEOUT => std::thread::sleep(POLL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(refusal(format!(
                    "credential_process {program} did not exit within {} seconds",
                    TIMEOUT.as_secs()
                )));
            }
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
