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
pub(crate) struct Profile {
    /// The key pair the credentials file holds, with its session token.
    pub(crate) credentials: Option<Credentials>,
    /// The `region` the configuration file names.
    pub(crate) region: Option<String>,
    /// The `endpoint_url` the configuration file names.
    pub(crate) endpoint_url: Option<String>,
}

/// Read `profile` - or `AWS_PROFILE`, or `default` - from both files.
///
/// A file that is missing or unreadable contributes nothing: the shared files
/// are optional on every machine, and a profile nobody wrote is not an error.
pub(crate) fn load(profile: Option<&str>) -> Profile {
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
pub(crate) fn credentials(profile: Option<&str>) -> Option<Credentials> {
    load(profile).credentials
}

/// `~/.aws/{name}`, through the one home-directory rule the crate has.
fn home_file(name: &str) -> Option<PathBuf> {
    let home = crate::local::LocalFolder::home().ok()?.path().ok()?;
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/object/aws/profile.rs` pins and a caller cannot reach.
    //!
    //! `load` reads two files off the machine running the test, which is not
    //! a fixture anybody can build; what is worth pinning is the INI reading
    //! underneath it, so that is what forwards here.
    use crate::object::Credentials;

    /// The top-level `key = value` pairs of one section, or `None` when absent.
    pub fn section(text: &str, name: &str, prefixed: bool) -> Option<Vec<(String, String)>> {
        super::section(text, name, prefixed)
    }

    /// One value of a section.
    pub fn value(section: &[(String, String)], key: &str) -> Option<String> {
        super::value(section, key)
    }

    /// The key pair a section holds, when it holds one.
    pub fn credentials_of(section: &[(String, String)]) -> Option<Credentials> {
        super::credentials_of(section)
    }
}
