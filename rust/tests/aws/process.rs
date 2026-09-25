//! `rust/src/aws/process.rs`: the `credential_process` a session or a profile
//! names, run the way a shell would split its line.
//!
//! The processes are `sh` running `printf` over a document handed to it as an
//! argument, so every one is a real program on a real pipe, and the tests
//! that spawn one are Unix-only. A session that states only its process and
//! consults no environment walks nothing else, so most of these need no
//! socket; the one that proves the chain walks on past a program that cannot
//! be started ends at the identity fake's instance metadata service.

use std::time::SystemTime;
#[cfg(unix)]
use std::time::{Duration, UNIX_EPOCH};

use yggdryl::aws::{Credentials, Session};

use crate::identity::Identity;
#[cfg(unix)]
use crate::mod_::scratch;
use crate::mod_::sealed;

/// An expiry no test outlives, and the instant it names.
#[cfg(unix)]
const FAR: &str = "2099-01-01T00:00:00Z";
#[cfg(unix)]
const FAR_SECONDS: u64 = 4_070_908_800;

/// The keys the fake's instance role answers.
const INSTANCE_KEY: &str = "ASIAINSTANCEROLE";

/// A program no machine has.
const MISSING: &str = "yggdryl-no-such-credential-helper";

/// A session that consults nothing but what it states: no environment, no
/// shared file, no metadata service.
fn stated() -> Session {
    Session::new().with_environment(false)
}

/// A session that reads `pairs` as its whole environment and its shared files
/// from a directory of its own, with the instance metadata service off, so
/// the chain walks every source it has without a socket.
#[cfg(unix)]
fn local(name: &str, pairs: &[(&str, &str)]) -> Session {
    Session::new()
        .with_variables(pairs.iter().copied())
        .with_directory(scratch(name))
        .with_metadata_disabled(true)
}

/// The document a process prints for `access_key`: version 1, with a
/// secret, a session token, an expiry and an account.
#[cfg(unix)]
fn document(access_key: &str) -> serde_json::Value {
    serde_json::json!({
        "Version": 1,
        "AccessKeyId": access_key,
        "SecretAccessKey": "process-secret",
        "SessionToken": "process-token",
        "Expiration": FAR,
        "AccountId": "123456789012",
    })
}

/// A `credential_process` line that prints `document` and exits 0: `sh` runs
/// `printf` over the document handed to it as `$0`, so the document crosses
/// the shell-style split as one single-quoted word.
#[cfg(unix)]
fn printing(document: &serde_json::Value) -> String {
    format!("sh -c 'printf %s \"$0\"' '{document}'")
}

/// The set the session signs with now, which the test expects to exist.
fn found(session: &Session) -> Credentials {
    session
        .credentials(SystemTime::now())
        .expect("a walk that answers")
        .expect("a credential set rather than unsigned requests")
}

/// The refusal the session answers now, rendered.
fn refused(session: &Session) -> String {
    session
        .credentials(SystemTime::now())
        .expect_err("a walk that refuses")
        .to_string()
}

// --- what a process answers -------------------------------------------------------

#[cfg(unix)]
#[test]
fn a_stated_process_s_document_is_the_set_with_its_token_its_expiry_and_its_account() {
    let session = stated().with_credential_process(printing(&document("AKIAPROCESS")));

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), "AKIAPROCESS");
    assert_eq!(keys.session_token(), Some("process-token"));
    assert_eq!(
        keys.expires_at(),
        Some(UNIX_EPOCH + Duration::from_secs(FAR_SECONDS))
    );
    assert_eq!(keys.account_id(), Some("123456789012"));
    assert_eq!(session.credential_source(), Some("credential process"));
}

#[cfg(unix)]
#[test]
fn a_profile_s_credential_process_line_runs_the_same_way() {
    let line = printing(&document("AKIAPROFILE"));
    let session = local("process-profile", &[])
        .with_config_text(format!("[profile vault]\ncredential_process = {line}\n"))
        .with_profile("vault");

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), "AKIAPROFILE");
    assert_eq!(keys.session_token(), Some("process-token"));
    assert_eq!(keys.account_id(), Some("123456789012"));
    assert_eq!(session.credential_source(), Some("credential process"));
}

#[cfg(unix)]
#[test]
fn a_document_without_a_version_is_read_and_one_stating_another_is_refused() {
    let mut unversioned = document("AKIAUNVERSIONED");
    unversioned
        .as_object_mut()
        .expect("the document is an object")
        .remove("Version");
    let session = stated().with_credential_process(printing(&unversioned));
    assert_eq!(found(&session).access_key_id(), "AKIAUNVERSIONED");

    let mut second = document("AKIASECOND");
    second["Version"] = serde_json::json!(2);
    let message = refused(&stated().with_credential_process(printing(&second)));
    assert!(message.contains("credential process"), "{message}");
    assert!(message.contains("Version 1"), "{message}");
    assert!(message.contains("got 2"), "{message}");
}

#[cfg(unix)]
#[test]
fn output_that_is_no_credential_document_is_refused_by_what_it_lacks() {
    let message = refused(&stated().with_credential_process("sh -c 'echo the-keys-are-elsewhere'"));
    assert!(
        message.contains("expected a JSON credential document from credential_process"),
        "{message}"
    );

    let keyless = serde_json::json!({"Version": 1, "AccessKeyId": "AKIAHALF"});
    let message = refused(&stated().with_credential_process(printing(&keyless)));
    assert!(
        message.contains("AccessKeyId and SecretAccessKey"),
        "a key without its secret is no set: {message}"
    );
}

// --- how a process fails ------------------------------------------------------------

#[cfg(unix)]
#[test]
fn a_process_that_exits_non_zero_is_refused_quoting_its_standard_error() {
    let session =
        stated().with_credential_process("sh -c 'echo \"the vault is sealed\" >&2; exit 3'");

    let message = refused(&session);
    assert!(message.contains("credential process"), "{message}");
    assert!(message.contains("the vault is sealed"), "{message}");
    assert!(message.contains("exit status: 3"), "{message}");
}

#[cfg(unix)]
#[test]
fn the_exit_status_decides_even_when_the_document_was_printed() {
    let json = document("AKIAFAILED");
    let session = stated().with_credential_process(format!(
        "sh -c 'printf %s \"$0\"; echo vault-said-no >&2; exit 1' '{json}'"
    ));

    let message = refused(&session);
    assert!(message.contains("vault-said-no"), "{message}");
    assert!(message.contains("exit status: 1"), "{message}");
    assert!(
        !message.contains("AKIAFAILED"),
        "the document a failed process printed is not read: {message}"
    );
}

#[cfg(unix)]
#[test]
fn the_standard_error_a_refusal_quotes_is_bounded() {
    let noise = "Q".repeat(4096);
    let session =
        stated().with_credential_process(format!("sh -c 'printf %s {noise} >&2; exit 1'"));

    let message = refused(&session);
    assert_eq!(
        message.matches('Q').count(),
        512,
        "at most 512 bytes of standard error are quoted"
    );
}

#[test]
fn an_unknown_program_is_a_recorded_failure_and_the_chain_walks_on() {
    let identity = Identity::start();
    let session = sealed(&identity, "process-unknown")
        .with_credential_process(format!("{MISSING} --profile trading"));

    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
    assert_eq!(session.credential_source(), Some("instance metadata"));
    assert_eq!(
        identity.request_count(),
        3,
        "the instance's token, listing and role, after the process failed to start"
    );

    let identity = Identity::start();
    identity.set_imds_role(None);
    let session = sealed(&identity, "process-unknown-refused").with_credential_process(MISSING);
    let message = refused(&session);
    assert!(message.contains("credential process"), "{message}");
    assert!(
        message.contains(&format!("could not run credential_process {MISSING}")),
        "the program is named: {message}"
    );
}

#[test]
fn a_blank_command_is_no_process_at_all() {
    let session = stated().with_credential_process("   ");

    assert_eq!(
        session
            .credentials(SystemTime::now())
            .expect("nothing configured is not a refusal"),
        None
    );
    assert_eq!(session.credential_source(), None);
}

// --- how a line is split and run ----------------------------------------------------

#[cfg(unix)]
#[test]
fn quoted_and_escaped_arguments_cross_the_shell_style_split_whole() {
    let json = document("AKIASPLIT");
    // The script refuses unless each argument arrived exactly as quoted:
    // double quotes keeping a doubled space, a backslash keeping a space,
    // an escaped quote inside double quotes, single quotes keeping the rest.
    let command = format!(
        r#"sh -c 'test "$0" = "a b  c" && test "$1" = "x y" && test "$2" = "say \"hi\"" && printf %s "$3"' "a b  c" x\ y "say \"hi\"" '{json}'"#
    );
    let session = stated().with_credential_process(command);

    assert_eq!(found(&session).access_key_id(), "AKIASPLIT");
}

#[cfg(unix)]
#[test]
fn the_process_reads_nothing_on_its_standard_input() {
    let json = document("AKIASTDIN");
    // `cat` returns at once only because its input is empty, not inherited.
    let session = stated().with_credential_process(format!(
        "sh -c 'cat > /dev/null; printf %s \"$0\"' '{json}'"
    ));

    assert_eq!(found(&session).access_key_id(), "AKIASTDIN");
}

// --- where a process sits in the chain ----------------------------------------------

#[cfg(unix)]
#[test]
fn a_stated_process_answers_ahead_of_the_environment_keys_and_a_failing_one_is_passed_over() {
    let pairs = [
        ("AWS_ACCESS_KEY_ID", "AKIAENVIRONMENT"),
        ("AWS_SECRET_ACCESS_KEY", "environment-secret"),
    ];

    let answering =
        local("process-ahead", &pairs).with_credential_process(printing(&document("AKIAPROCESS")));
    assert_eq!(found(&answering).access_key_id(), "AKIAPROCESS");
    assert_eq!(answering.credential_source(), Some("credential process"));

    let failing = local("process-passed-over", &pairs).with_credential_process("sh -c 'exit 1'");
    assert_eq!(found(&failing).access_key_id(), "AKIAENVIRONMENT");
    assert_eq!(failing.credential_source(), Some("environment"));
}

#[cfg(unix)]
#[test]
fn a_profile_s_process_answers_behind_the_credentials_file_s_keys_and_ahead_of_the_config_file_s() {
    let marker = scratch("process-marker").join("ran");
    let json = document("AKIAPROFILE");
    // The process leaves a file behind, so whether it ran is a fact on disk.
    let line = format!(
        "sh -c 'touch \"$1\"; printf %s \"$0\"' '{json}' '{}'",
        marker.display()
    );
    let config = format!("[default]\ncredential_process = {line}\n");

    let behind = local("process-behind-file", &[])
        .with_config_text(config.as_str())
        .with_credentials_text(
            "[default]\naws_access_key_id = AKIAFILE\naws_secret_access_key = file-secret\n",
        );
    assert_eq!(found(&behind).access_key_id(), "AKIAFILE");
    assert_eq!(behind.credential_source(), Some("shared credentials file"));
    assert!(
        !marker.exists(),
        "the credentials file answered, so the process never ran"
    );

    let ahead = local("process-ahead-of-config", &[]).with_config_text(format!(
        "{config}aws_access_key_id = AKIACONFIG\naws_secret_access_key = config-secret\n"
    ));
    assert_eq!(found(&ahead).access_key_id(), "AKIAPROFILE");
    assert_eq!(ahead.credential_source(), Some("credential process"));
    assert!(
        marker.exists(),
        "the process ran ahead of the config file's keys"
    );
}
