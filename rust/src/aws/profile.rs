//! The shared AWS files, `~/.aws/config` and `~/.aws/credentials`.
//!
//! Both are INI with the AWS tools' own reading: the configuration file
//! spells every profile but `default` as `[profile name]` and holds
//! `[sso-session name]` and `[services name]` sections beside them, the
//! credentials file uses bare names, a key with nothing after its `=` opens
//! an indented table (`s3 =` followed by `addressing_style = path`), keys
//! fold to lower case, a `#` or `;` line is a comment, and where the two
//! files name one profile the credentials file's values win. A file that is
//! missing or unreadable contributes nothing: the shared files are optional on
//! every machine, and a profile nobody wrote is not an error here - the
//! session reports it as one source among the ones it walked.
//!
//! Nothing is read at construction; the session loads both files once, on the
//! first question that needs them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::credentials::Credentials;
use super::sso::Sso;
use super::sts::{AssumedRole, CredentialSource};
use crate::{Error, Result};

/// One entry of a section: a value, or the table an indented block spells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    /// `key = value`.
    Text(String),
    /// `key =` followed by indented `name = value` lines.
    Table(BTreeMap<String, String>),
}

/// The entries of one section, keys folded to lower case.
pub(crate) type Table = BTreeMap<String, Entry>;

/// What a section header names, in the file it appears in.
enum Header {
    Profile(String),
    SsoSession(String),
    Services(String),
    /// A section the AWS tools keep but nothing here reads, such as
    /// `[plugins]`.
    Other,
}

/// Which file a section header is read as belonging to.
#[derive(Clone, Copy)]
enum Style {
    /// `[profile name]`, `[sso-session name]`, `[services name]`, `[default]`.
    Config,
    /// `[name]`, every section a profile.
    Credentials,
}

/// Both files, read once.
#[derive(Clone, Default)]
pub struct Files {
    /// The configuration file's profiles with the credentials file's values
    /// laid over them.
    profiles: BTreeMap<String, Table>,
    /// The configuration file's profiles alone.
    config_profiles: BTreeMap<String, Table>,
    /// The credentials file's profiles alone, which the chain reads as a
    /// source of their own ahead of the configuration file.
    credential_profiles: BTreeMap<String, Table>,
    sso_sessions: BTreeMap<String, Table>,
    services: BTreeMap<String, Table>,
}

impl Files {
    /// Read both files from their text, each optional.
    pub fn parse(config: Option<&str>, credentials: Option<&str>) -> Self {
        let mut files = Self::default();
        for (header, table) in config
            .map(|text| parse(text, Style::Config))
            .unwrap_or_default()
        {
            let target = match header {
                Header::Profile(name) => {
                    files
                        .config_profiles
                        .entry(name.clone())
                        .or_default()
                        .extend(table.clone());
                    files.profiles.entry(name)
                }
                Header::SsoSession(name) => files.sso_sessions.entry(name),
                Header::Services(name) => files.services.entry(name),
                Header::Other => continue,
            };
            target.or_default().extend(table);
        }
        for (header, table) in credentials
            .map(|text| parse(text, Style::Credentials))
            .unwrap_or_default()
        {
            let Header::Profile(name) = header else {
                continue;
            };
            files
                .credential_profiles
                .entry(name.clone())
                .or_default()
                .extend(table.clone());
            files.profiles.entry(name).or_default().extend(table);
        }
        files
    }

    /// The profile `name`, when either file holds one.
    pub fn profile(&self, name: &str) -> Option<Profile> {
        let values = self.profiles.get(name)?.clone();
        let config_values = self.config_profiles.get(name).cloned().unwrap_or_default();
        let sso_session = text(&values, "sso_session").and_then(|session| {
            self.sso_sessions
                .get(session)
                .map(|table| (session.to_owned(), table.clone()))
        });
        let services = text(&values, "services").and_then(|services| {
            self.services
                .get(services)
                .map(|table| (services.to_owned(), table.clone()))
        });
        Some(Profile {
            name: name.to_owned(),
            credential_values: self
                .credential_profiles
                .get(name)
                .cloned()
                .unwrap_or_default(),
            config_values,
            values,
            sso_session,
            services,
        })
    }

    /// Every profile name either file holds, in name order.
    pub fn names(&self) -> Vec<String> {
        self.profiles.keys().cloned().collect()
    }
}

/// One profile, as the AWS tools see it: the configuration file's section
/// with the credentials file's values over it, plus the `[sso-session]` and
/// `[services]` sections it names.
///
/// ```
/// use yggdryl::aws::Session;
///
/// # fn main() -> yggdryl::Result<()> {
/// let session = Session::new()
///     .with_environment(false)
///     .with_config_text("[profile trading]\nregion = eu-west-3\ns3 =\n  addressing_style = path\n")
///     .with_profile("trading");
/// let profile = session.profile().expect("the profile the text spells");
/// assert_eq!(profile.region(), Some("eu-west-3"));
/// assert_eq!(profile.s3("addressing_style"), Some("path"));
/// assert_eq!(profile.get("output"), None);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Profile {
    name: String,
    values: Table,
    config_values: Table,
    credential_values: Table,
    sso_session: Option<(String, Table)>,
    services: Option<(String, Table)>,
}

impl Profile {
    /// The profile's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// A top-level value, by its key in any case; an empty value is absent.
    pub fn get(&self, key: &str) -> Option<&str> {
        text(&self.values, &key.to_ascii_lowercase())
    }

    /// A boolean value, in the spellings the AWS tools accept.
    pub fn flag(&self, key: &str) -> Option<bool> {
        self.get(key).map(crate::auth::is_true)
    }

    /// A value of the indented table under `table`, such as `s3`.
    pub fn nested(&self, table: &str, key: &str) -> Option<&str> {
        nested(&self.values, &table.to_ascii_lowercase(), key)
    }

    /// A value of the `s3` table, such as `addressing_style`.
    pub fn s3(&self, key: &str) -> Option<&str> {
        self.nested("s3", key)
    }

    /// Every top-level value, in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.values.iter().filter_map(|(key, entry)| match entry {
            Entry::Text(value) if !value.is_empty() => Some((key.as_str(), value.as_str())),
            Entry::Text(_) | Entry::Table(_) => None,
        })
    }

    /// The `region`.
    pub fn region(&self) -> Option<&str> {
        self.get("region")
    }

    /// The `output` format the CLI renders in.
    pub fn output(&self) -> Option<&str> {
        self.get("output")
    }

    /// The `endpoint_url` every service is reached at.
    pub fn endpoint_url(&self) -> Option<&str> {
        self.get("endpoint_url")
    }

    /// The `endpoint_url` of `service` in the `[services]` section the
    /// profile names, when it names one that states it.
    pub fn service_endpoint_url(&self, service: &str) -> Option<&str> {
        let (_, table) = self.services.as_ref()?;
        nested(table, &service_key(service), "endpoint_url")
    }

    /// The `[sso-session]` section the profile names, when it names one that
    /// exists.
    pub fn sso_session_name(&self) -> Option<&str> {
        self.sso_session.as_ref().map(|(name, _)| name.as_str())
    }

    /// The key pair the profile holds: the credentials file's, else the
    /// configuration file's, and never a pair with one file's key and the
    /// other's token.
    pub fn credentials(&self) -> Option<Credentials> {
        self.credential_file_credentials()
            .or_else(|| self.config_file_credentials())
    }

    /// The key pair the credentials file alone holds.
    pub(crate) fn credential_file_credentials(&self) -> Option<Credentials> {
        credentials_of(&self.credential_values)
    }

    /// The key pair the configuration file alone holds.
    pub(crate) fn config_file_credentials(&self) -> Option<Credentials> {
        credentials_of(&self.config_values)
    }

    /// The `credential_process` command line.
    pub fn credential_process(&self) -> Option<&str> {
        self.get("credential_process")
    }

    /// The role the profile assumes, with how it obtains the keys that sign
    /// the exchange.
    ///
    /// # Errors
    ///
    /// A `role_arn` with neither `source_profile`, `credential_source` nor
    /// `web_identity_token_file`, or with both of the first two; a
    /// `credential_source` that names no source; a `duration_seconds` that is
    /// not a number.
    pub fn assumed_role(&self) -> Result<Option<AssumedRole>> {
        let Some(role_arn) = self.get("role_arn") else {
            return Ok(None);
        };
        let mut role = AssumedRole::new(role_arn);
        if let Some(name) = self.get("role_session_name") {
            role = role.with_session_name(name);
        }
        if let Some(external_id) = self.get("external_id") {
            role = role.with_external_id(external_id);
        }
        if let Some(serial) = self.get("mfa_serial") {
            role = role.with_mfa_serial(serial);
        }
        if let Some(seconds) = self.get("duration_seconds") {
            let seconds: u64 = seconds.parse().map_err(|_| {
                self.refusal(format!(
                    "expected a number of seconds in duration_seconds, got {seconds:?}"
                ))
            })?;
            role = role.with_duration(Duration::from_secs(seconds));
        }
        if let Some(token_file) = self.get("web_identity_token_file") {
            role = role.with_web_identity_token_file(token_file);
        }
        if let Some(source) = self.get("source_profile") {
            role = role.with_source_profile(source);
        }
        if let Some(source) = self.get("credential_source") {
            let source: CredentialSource = source
                .parse()
                .map_err(|error: Error| self.refusal(format!("credential_source: {error}")))?;
            role = role.with_credential_source(source);
        }
        match (
            role.source_profile().is_some(),
            role.credential_source().is_some(),
            role.web_identity_token_file().is_some(),
        ) {
            (true, true, _) => Err(self.refusal(
                "names both source_profile and credential_source for its role, and a role has one source",
            )),
            (false, false, false) => Err(self.refusal(
                "names role_arn without source_profile, credential_source or web_identity_token_file",
            )),
            _ => Ok(Some(role)),
        }
    }

    /// The IAM Identity Center sign-in the profile obtains its keys through.
    ///
    /// # Errors
    ///
    /// An `sso_session` no `[sso-session]` section defines, or a sign-in
    /// missing one of its start URL, region, account id or role name.
    pub fn sso(&self) -> Result<Option<Sso>> {
        const LEGACY: [&str; 4] = [
            "sso_start_url",
            "sso_region",
            "sso_account_id",
            "sso_role_name",
        ];
        let (session, start_url, region, scopes) = match self.get("sso_session") {
            Some(name) => {
                let Some((_, table)) = &self.sso_session else {
                    return Err(self.refusal(format!(
                        "names sso_session {name}, which no [sso-session {name}] section defines"
                    )));
                };
                (
                    Some(name),
                    text(table, "sso_start_url"),
                    text(table, "sso_region"),
                    text(table, "sso_registration_scopes"),
                )
            }
            None => {
                if !LEGACY.iter().any(|key| self.get(key).is_some()) {
                    return Ok(None);
                }
                (
                    None,
                    self.get("sso_start_url"),
                    self.get("sso_region"),
                    None,
                )
            }
        };
        let account_id = self.get("sso_account_id");
        let role_name = self.get("sso_role_name");
        let missing: Vec<&str> = [
            ("sso_start_url", start_url),
            ("sso_region", region),
            ("sso_account_id", account_id),
            ("sso_role_name", role_name),
        ]
        .into_iter()
        .filter(|(_, value)| value.is_none())
        .map(|(key, _)| key)
        .collect();
        if !missing.is_empty() {
            return Err(self.refusal(format!(
                "signs in through IAM Identity Center without {}",
                missing.join(", ")
            )));
        }
        let mut sso = Sso::new(
            start_url.unwrap_or_default(),
            region.unwrap_or_default(),
            account_id.unwrap_or_default(),
            role_name.unwrap_or_default(),
        );
        if let Some(session) = session {
            sso = sso.with_session_name(session);
        }
        if let Some(scopes) = scopes {
            sso = sso.with_scopes(scopes.split(',').map(str::trim).filter(|s| !s.is_empty()));
        }
        Ok(Some(sso))
    }

    /// A refusal naming this profile.
    fn refusal(&self, reason: impl std::fmt::Display) -> Error {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("the profile {} {reason}", self.name),
        ))
    }
}

impl std::fmt::Debug for Files {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Files")
            .field("profiles", &redacted_all(&self.profiles))
            .field("sso_sessions", &redacted_all(&self.sso_sessions))
            .field("services", &redacted_all(&self.services))
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Profile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Profile")
            .field("name", &self.name)
            .field("values", &redacted(&self.values))
            .field(
                "sso_session",
                &self.sso_session.as_ref().map(|(name, _)| name),
            )
            .field("services", &self.services.as_ref().map(|(name, _)| name))
            .finish()
    }
}

/// Whether a key names a value the files hold that must never render.
fn is_secret_key(key: &str) -> bool {
    matches!(
        key,
        "aws_secret_access_key" | "aws_session_token" | "aws_security_token"
    ) || key.contains("secret")
        || key.contains("password")
}

/// One table with its secrets redacted, for `Debug`.
fn redacted(table: &Table) -> BTreeMap<&str, String> {
    table
        .iter()
        .map(|(key, entry)| {
            let shown = match entry {
                Entry::Text(_) if is_secret_key(key) => "<redacted>".to_owned(),
                Entry::Text(value) => value.clone(),
                Entry::Table(values) => format!("{values:?}"),
            };
            (key.as_str(), shown)
        })
        .collect()
}

/// Every table with its secrets redacted, for `Debug`.
fn redacted_all(tables: &BTreeMap<String, Table>) -> BTreeMap<&str, BTreeMap<&str, String>> {
    tables
        .iter()
        .map(|(name, table)| (name.as_str(), redacted(table)))
        .collect()
}

/// The key a service's `[services]` entry or `AWS_ENDPOINT_URL_` variable
/// is spelled under: the service id with a hyphen or a space as `_`.
pub(crate) fn service_key(service: &str) -> String {
    service.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

/// One non-empty text value of `table`.
fn text<'a>(table: &'a Table, key: &str) -> Option<&'a str> {
    match table.get(key)? {
        Entry::Text(value) if !value.is_empty() => Some(value.as_str()),
        Entry::Text(_) | Entry::Table(_) => None,
    }
}

/// One non-empty value of the nested table under `name`.
fn nested<'a>(table: &'a Table, name: &str, key: &str) -> Option<&'a str> {
    match table.get(name)? {
        Entry::Table(values) => values
            .get(&key.to_ascii_lowercase())
            .map(String::as_str)
            .filter(|value| !value.is_empty()),
        Entry::Text(_) => None,
    }
}

/// The key pair a section holds, when it holds one.
fn credentials_of(table: &Table) -> Option<Credentials> {
    let access_key_id = text(table, "aws_access_key_id")?;
    let secret_access_key = text(table, "aws_secret_access_key")?;
    let mut credentials = Credentials::new(access_key_id, secret_access_key);
    if let Some(token) =
        text(table, "aws_session_token").or_else(|| text(table, "aws_security_token"))
    {
        credentials = credentials.with_session_token(token);
    }
    if let Some(account_id) = text(table, "aws_account_id") {
        credentials = credentials.with_account_id(account_id);
    }
    Some(credentials)
}

/// Read one file's sections, in document order.
///
/// A header opens a section; a `key = value` (or `key: value`) belongs to
/// it; a key with nothing after its delimiter opens a table of the lines
/// indented deeper than it, read the same way. A line indented deeper than
/// a key that had a value is that value's continuation, which the AWS tools
/// cannot read as a table and this reader passes over; a line indented no
/// deeper than the key before it - or right after a header, at any indent -
/// is the key it is, as the AWS tools' parser reads it. A later value for a
/// key replaces an earlier one.
fn parse(text: &str, style: Style) -> Vec<(Header, Table)> {
    let mut sections: Vec<(Header, Table)> = Vec::new();
    // The key a table is being read under, and the indent of the last key.
    let mut pending: Option<String> = None;
    let mut key_indent: Option<usize> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if key_indent.is_some_and(|key| indent > key) {
            if let (Some(key), Some((_, table))) = (&pending, sections.last_mut()) {
                if let Some((name, value)) = split_pair(trimmed) {
                    if let Some(Entry::Table(values)) = table.get_mut(key) {
                        values.insert(name.to_ascii_lowercase(), value.to_owned());
                    }
                }
            }
            continue;
        }
        pending = None;
        if let Some(inner) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            sections.push((header(inner.trim(), style), Table::new()));
            key_indent = None;
            continue;
        }
        let Some((_, table)) = sections.last_mut() else {
            continue;
        };
        let Some((key, value)) = split_pair(trimmed) else {
            continue;
        };
        key_indent = Some(indent);
        let key = key.to_ascii_lowercase();
        if value.is_empty() {
            table.insert(key.clone(), Entry::Table(BTreeMap::new()));
            pending = Some(key);
        } else {
            table.insert(key, Entry::Text(value.to_owned()));
        }
    }
    sections
}

/// `key = value` or `key: value`, split at the first delimiter, both trimmed.
fn split_pair(line: &str) -> Option<(&str, &str)> {
    let at = line.find(['=', ':'])?;
    let (key, value) = (line[..at].trim(), line[at + 1..].trim());
    (!key.is_empty()).then_some((key, value))
}

/// What a section header names.
fn header(inner: &str, style: Style) -> Header {
    match style {
        Style::Credentials => Header::Profile(inner.to_owned()),
        Style::Config => {
            if inner == "default" {
                return Header::Profile(inner.to_owned());
            }
            let words = split_words(inner);
            let [kind, name] = words.as_slice() else {
                return Header::Other;
            };
            match kind.as_str() {
                "profile" => Header::Profile(name.clone()),
                "sso-session" => Header::SsoSession(name.clone()),
                "services" => Header::Services(name.clone()),
                _ => Header::Other,
            }
        }
    }
}

/// Split `text` into words the way the AWS tools do before running it, which
/// is Python's `shlex` in POSIX mode: whitespace separates, single quotes
/// keep everything, double quotes keep everything but a backslash before a
/// `"` or another backslash, and a backslash outside quotes keeps the
/// character after it.
///
/// The tools apply it to a `[profile "my name"]` header and to a
/// `credential_process` line, so both are read by one splitter.
pub(crate) fn split_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        match quote {
            Some('\'') => {
                if character == '\'' {
                    quote = None;
                } else {
                    word.push(character);
                }
            }
            Some(_) => match character {
                '"' => quote = None,
                '\\' => match characters.next() {
                    Some(escaped @ ('"' | '\\')) => word.push(escaped),
                    Some(other) => {
                        word.push('\\');
                        word.push(other);
                    }
                    None => word.push('\\'),
                },
                other => word.push(other),
            },
            None => match character {
                '\'' | '"' => {
                    quote = Some(character);
                    in_word = true;
                }
                '\\' => {
                    if let Some(escaped) = characters.next() {
                        word.push(escaped);
                        in_word = true;
                    }
                }
                c if c.is_whitespace() => {
                    if in_word {
                        words.push(std::mem::take(&mut word));
                        in_word = false;
                    }
                }
                other => {
                    word.push(other);
                    in_word = true;
                }
            },
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// `path` with a leading `~` replaced by `home`, the way the AWS tools read
/// `AWS_CONFIG_FILE`.
pub(crate) fn expand_user(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            home.join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(path),
    }
}

/// The key pair the original EC2 credential file spells, `AWSAccessKeyId=`
/// and `AWSSecretKey=` one per line, which `AWS_CREDENTIAL_FILE` names.
pub(crate) fn ec2_credential_file(text: &str) -> Option<Credentials> {
    let mut access_key_id = None;
    let mut secret = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "AWSAccessKeyId" => access_key_id = Some(value.trim().to_owned()),
            "AWSSecretKey" => secret = Some(value.trim().to_owned()),
            _ => {}
        }
    }
    Some(Credentials::new(
        access_key_id.filter(|key| !key.is_empty())?,
        secret.filter(|key| !key.is_empty())?,
    ))
}

/// The key pair a boto configuration file's `[Credentials]` section spells.
pub(crate) fn boto_config(text: &str) -> Option<Credentials> {
    parse(text, Style::Credentials)
        .into_iter()
        .find_map(|(header, table)| match header {
            Header::Profile(name) if name.eq_ignore_ascii_case("credentials") => {
                credentials_of(&table)
            }
            _ => None,
        })
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/aws/profile.rs` pins and a caller cannot reach.
    //!
    //! The files themselves are read through a session; what is pinned here is
    //! the reading beneath it, over text a test spells - the header
    //! vocabulary, the indented tables, the precedence between the two files,
    //! the shell-style splitter, and the two legacy formats.
    pub use super::Files;

    use std::path::{Path, PathBuf};

    use crate::aws::Credentials;

    /// Split `text` into words the way a POSIX shell does.
    pub fn split_words(text: &str) -> Vec<String> {
        super::split_words(text)
    }

    /// `path` with a leading `~` replaced by `home`.
    pub fn expand_user(path: &str, home: Option<&Path>) -> PathBuf {
        super::expand_user(path, home)
    }

    /// The key pair the original EC2 credential file spells.
    pub fn ec2_credential_file(text: &str) -> Option<Credentials> {
        super::ec2_credential_file(text)
    }

    /// The key pair a boto configuration file's `[Credentials]` section spells.
    pub fn boto_config(text: &str) -> Option<Credentials> {
        super::boto_config(text)
    }

    /// The key a service's endpoint is spelled under.
    pub fn service_key(service: &str) -> String {
        super::service_key(service)
    }
}
