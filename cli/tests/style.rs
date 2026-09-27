//! `cli/src/style.rs`: output that ends quietly when its reader has gone.

use std::process::{Command, Stdio};

#[test]
fn a_reader_that_closes_the_pipe_ends_the_command_quietly() {
    let root = std::env::temp_dir().join(format!("ygg-cli-style-{}", std::process::id()));
    let mut child = Command::new(env!("CARGO_BIN_EXE_yggdryl"))
        .args(["fix", "--root"])
        .arg(&root)
        .args(["fields", "list"])
        .env("NO_COLOR", "1")
        .env_remove("GITHUB_ACTIONS")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run CLI");
    // Closing the read end before the command writes is `yggdryl ... | head -0`:
    // every write then meets a broken pipe.
    drop(child.stdout.take());
    let output = child.wait_with_output().expect("the command ends");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}
