//! `rust/src/auth/report.rs`: what a walk found wanting.

use yggdryl::internals::auth_report::Report;

#[test]
fn nothing_failed_is_no_refusal() {
    let mut report = Report::new("AWS credentials");
    report.absent("environment");
    report.absent("container");
    assert!(
        report
            .conclude::<()>()
            .expect("absence alone is not failure")
            .is_none(),
        "nothing configured is anonymous"
    );
}

#[test]
fn a_failure_is_a_refusal_naming_every_source() {
    let mut report = Report::new("AWS credentials");
    report.absent("environment");
    report.failed("sso", "the sign-in lapsed");
    report.failed("credential process", "exit status 1");
    report.absent("instance metadata");
    let refusal = report.conclude::<()>().expect_err("something failed");
    let message = refusal.to_string();
    assert!(
        message.contains("no AWS credentials could be obtained: sso:"),
        "{message}"
    );
    assert!(message.contains("sso: the sign-in lapsed"), "{message}");
    assert!(
        message.contains("credential process: exit status 1"),
        "{message}"
    );
    assert!(
        message.contains("nothing configured in: environment, instance metadata"),
        "{message}"
    );
    assert!(
        matches!(refusal, yggdryl::Error::Io(ref error) if error.kind() == std::io::ErrorKind::PermissionDenied),
        "{refusal:?}"
    );
}
