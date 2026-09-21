//! `rust/src/object/aws/profile.rs`: the shared-file reading no caller can name.
//!
//! `~/.aws/config` spells every profile but `default` as `[profile name]` while
//! `~/.aws/credentials` uses the bare name, and an indented `s3 =` block
//! belongs to a nested table rather than to the profile. Those are the three
//! facts a machine's own files would not reliably exercise, so the reading is
//! pinned over text a test spells itself.

use yggdryl::internals::object_aws_profile::{credentials_of, section, value};

const CONFIG: &str = "\
# the shared configuration file
[default]
region = us-east-1
s3 =
  payload_signing_enabled = false
endpoint_url = https://s3.example.io

[profile trading]
region=eu-west-3
aws_access_key_id = AKIATRADING
aws_secret_access_key = secret
";

const CREDENTIALS: &str = "\
[default]
aws_access_key_id = AKIADEFAULT
aws_secret_access_key = default-secret
aws_session_token = default-token
; a comment
[trading]
aws_access_key_id = AKIATRADING
aws_secret_access_key = trading-secret
";

#[test]
fn the_configuration_file_prefixes_named_profiles_and_skips_nested_tables() {
    let default = section(CONFIG, "default", true).unwrap();
    assert_eq!(value(&default, "region").as_deref(), Some("us-east-1"));
    assert_eq!(
        value(&default, "endpoint_url").as_deref(),
        Some("https://s3.example.io")
    );
    assert_eq!(value(&default, "payload_signing_enabled"), None);

    let trading = section(CONFIG, "trading", true).unwrap();
    assert_eq!(value(&trading, "region").as_deref(), Some("eu-west-3"));
    let keys = credentials_of(&trading).unwrap();
    assert_eq!(keys.access_key_id(), "AKIATRADING");
    assert!(section(CONFIG, "missing", true).is_none());
}

#[test]
fn the_credentials_file_uses_bare_names_and_carries_session_tokens() {
    let default = section(CREDENTIALS, "default", false).unwrap();
    let keys = credentials_of(&default).unwrap();
    assert_eq!(keys.access_key_id(), "AKIADEFAULT");
    assert_eq!(keys.session_token(), Some("default-token"));

    let trading = section(CREDENTIALS, "trading", false).unwrap();
    assert_eq!(credentials_of(&trading).unwrap().session_token(), None);
    // The bare spelling never matches a prefixed header, and vice versa.
    assert!(section(CREDENTIALS, "trading", true).is_some());
    assert!(section(CONFIG, "trading", false).is_none());
}
