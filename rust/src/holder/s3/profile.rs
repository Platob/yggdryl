//! The shared AWS files, `~/.aws/credentials` and `~/.aws/config`.
//!
//! Read once per resolution, on the first request, never at construction. The
//! format is INI with one wrinkle: the configuration file spells every profile
//! but `default` as `[profile name]`. Indented sub-keys (`s3 =` blocks) belong
//! to a nested table this reader has no use for and skips.

use std::path::PathBuf;

use super::credentials::{Credentials, variable};

/// What one profile says about the store.
#[derive(Debug, Default)]
pub(super) struct Profile {
    /// The key pair the credentials file holds, with its session token.
    pub(super) credentials: Option<Credentials>,
    /// The `region` the configuration file names.
    pub(super) region: Option<String>,
    /// The `endpoint_url` the configuration file names.
    pub(super) endpoint_url: Option<String>,
}

/// Read `profile` - or `AWS_PROFILE`, or `default` - from both files.
///
/// A file that is missing or unreadable contributes nothing: the shared files
/// are optional on every machine, and a profile nobody wrote is not an error.
pub(super) fn load(profile: Option<&str>) -> Profile {
    let name = profile
        .map(str::to_owned)
        .or_else(|| variable("AWS_PROFILE"))
        .unwrap_or_else(|| "default".to_owned());
    let mut resolved = Profile::default();

    let credentials = variable("AWS_SHARED_CREDENTIALS_FILE")
        .map(PathBuf::from)
        .or_else(|| home_file("credentials"));
    if let Some(text) = credentials.and_then(|path| std::fs::read_to_string(path).ok()) {
        if let Some(section) = section(&text, &name, false) {
            resolved.credentials = credentials_of(&section);
        }
    }

    let config = variable("AWS_CONFIG_FILE")
        .map(PathBuf::from)
        .or_else(|| home_file("config"));
    if let Some(text) = config.and_then(|path| std::fs::read_to_string(path).ok()) {
        if let Some(section) = section(&text, &name, true) {
            resolved.region = value(&section, "region");
            resolved.endpoint_url = value(&section, "endpoint_url");
            // The configuration file may hold keys too, after the
            // credentials file has had its say.
            if resolved.credentials.is_none() {
                resolved.credentials = credentials_of(&section);
            }
        }
    }
    resolved
}

/// Only the key pair of `profile`.
pub(super) fn credentials(profile: Option<&str>) -> Option<Credentials> {
    load(profile).credentials
}

/// `~/.aws/{name}`, through the one home-directory rule the crate has.
fn home_file(name: &str) -> Option<PathBuf> {
    let home = crate::holder::local::Folder::home().ok()?.path().ok()?;
    Some(home.join(".aws").join(name))
}

/// The top-level `key = value` pairs of one section, or `None` when absent.
///
/// `[profile name]` is how the configuration file spells a named profile;
/// `[default]` and the credentials file use the bare name.
fn section(text: &str, name: &str, prefixed: bool) -> Option<Vec<(String, String)>> {
    let mut pairs = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if let Some(header) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            if pairs.is_some() {
                break;
            }
            let header = header.trim();
            let matches = header == name
                || prefixed
                    && header
                        .strip_prefix("profile")
                        .is_some_and(|rest| rest.trim() == name);
            if matches {
                pairs = Some(Vec::new());
            }
            continue;
        }
        let Some(pairs) = pairs.as_mut() else {
            continue;
        };
        // An indented line is a nested table's key, not the profile's.
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            pairs.push((key.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    pairs
}

/// One value of a section.
fn value(section: &[(String, String)], key: &str) -> Option<String> {
    section
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
        .filter(|value| !value.is_empty())
}

/// The key pair a section holds, when it holds one.
fn credentials_of(section: &[(String, String)]) -> Option<Credentials> {
    let access_key_id = value(section, "aws_access_key_id")?;
    let secret_access_key = value(section, "aws_secret_access_key")?;
    let mut credentials = Credentials::new(access_key_id, secret_access_key);
    if let Some(token) = value(section, "aws_session_token") {
        credentials = credentials.with_session_token(token);
    }
    Some(credentials)
}

#[cfg(test)]
mod tests {
    use super::{credentials_of, section, value};

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
}
