//! `rust/src/aws/mod.rs`: the module's own door, and the fixtures every
//! suite here shares.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::aws::Session;

use crate::identity::Identity;

/// A directory of this test's own under the platform's temporary one, empty
/// when handed over, for the `~/.aws` a session is pointed at.
pub fn scratch(name: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "yggdryl-aws-{name}-{}-{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a scratch directory");
    path
}

/// A session that consults nothing but the fake at `identity`: no process
/// environment, an empty `~/.aws` of its own, every endpoint - STS, the
/// OIDC service, the portal, the metadata service - pointed at the fake.
pub fn sealed(identity: &Identity, name: &str) -> Session {
    Session::new()
        .with_variables::<&str, &str>([])
        .with_directory(scratch(name))
        .with_endpoint_url(identity.endpoint())
        .with_metadata_endpoint(identity.endpoint())
        .with_region("eu-west-3")
}

#[test]
fn a_session_states_nothing_and_resolves_lazily() {
    let session = Session::new();
    assert!(session.reads_environment());
    assert!(!session.anonymous());
    assert!(session.assumed_role().is_none());
    assert!(session.sso().is_none());
    assert_eq!(session.credential_source(), None);

    // A knob set is a new session; the one it came from is untouched.
    let named = session.with_profile("trading").with_region("eu-west-3");
    assert_eq!(named.profile_name(), "trading");
    assert_eq!(named.region().as_deref(), Some("eu-west-3"));
    assert_ne!(session.profile_name(), "trading");
}

#[test]
fn the_types_the_module_publishes_are_reachable_by_name() {
    use yggdryl::aws::{
        AssumedRole, CredentialSource, Credentials, DeviceAuthorization, Sso, SsoLogin,
    };

    let keys = Credentials::new("AKIA", "secret");
    assert_eq!(keys.access_key_id(), "AKIA");
    let role = AssumedRole::new("arn:aws:iam::123456789012:role/reader")
        .with_credential_source(CredentialSource::Environment);
    assert_eq!(
        role.credential_source(),
        Some(CredentialSource::Environment)
    );
    let sso = Sso::new(
        "https://x.awsapps.com/start",
        "eu-west-1",
        "123456789012",
        "Reader",
    );
    assert_eq!(sso.region(), "eu-west-1");
    assert!(matches!(SsoLogin::default(), SsoLogin::Never));
    fn takes(_: &DeviceAuthorization) {}
    let _ = takes;
}
