//! `.netrc`: the credentials a user keeps per host, read as curl and
//! Python's `requests` read them for a request that names none.
//!
//! The file is the one `NETRC` names, else `.netrc` in the home directory,
//! else `_netrc` there. A `machine` entry naming the request's host wins,
//! else the `default` entry; its `login` and `password` become a `Basic`
//! credential. `account` is read and ignored, a `macdef` body is skipped to
//! the blank line ending it, `#` opens a comment to the end of its line, and
//! a token may be double-quoted with `\` escaping the next character.
//!
//! The file is parsed once per version of it - its path, length and
//! modification time - so a lookup costs one `stat`, and an edit is read at
//! the next request.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use super::Authorization;

/// One entry: the host it names (`None` for `default`) and its credential.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    machine: Option<String>,
    login: Option<String>,
    password: Option<String>,
}

/// The entries of one file, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Netrc {
    entries: Vec<Entry>,
}

impl Netrc {
    /// Read the text of a `.netrc` file. Nothing in it is refused: an entry
    /// the grammar cannot place is skipped, as the reference readers skip it.
    pub(crate) fn parse(text: &str) -> Self {
        let mut tokens = Tokens::new(text);
        let mut entries = Vec::new();
        let mut current: Option<Entry> = None;
        while let Some(token) = tokens.next() {
            match token.as_str() {
                "machine" | "default" => {
                    entries.extend(current.take());
                    let machine = if token == "machine" {
                        match tokens.next() {
                            Some(name) => Some(name.to_ascii_lowercase()),
                            None => break,
                        }
                    } else {
                        None
                    };
                    current = Some(Entry {
                        machine,
                        login: None,
                        password: None,
                    });
                }
                "login" | "user" => {
                    let value = tokens.next();
                    if let Some(entry) = current.as_mut() {
                        entry.login = value;
                    }
                }
                "password" | "passwd" => {
                    let value = tokens.next();
                    if let Some(entry) = current.as_mut() {
                        entry.password = value;
                    }
                }
                "account" => {
                    tokens.next();
                }
                "macdef" => {
                    tokens.next();
                    tokens.skip_macro();
                }
                _ => {}
            }
        }
        entries.extend(current);
        Self { entries }
    }

    /// The credential for `host`: its `machine` entry, else `default`; `None`
    /// when neither names a login or a password.
    pub(crate) fn authorization(&self, host: &str) -> Option<Authorization> {
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.machine.as_deref() == Some(host.as_str()))
            .or_else(|| self.entries.iter().find(|entry| entry.machine.is_none()))?;
        if entry.login.is_none() && entry.password.is_none() {
            return None;
        }
        Some(Authorization::basic(
            entry.login.clone().unwrap_or_default(),
            entry.password.clone().unwrap_or_default(),
        ))
    }
}

/// The whitespace-separated tokens of a `.netrc`, comments dropped.
struct Tokens<'a> {
    rest: &'a str,
}

impl<'a> Tokens<'a> {
    fn new(text: &'a str) -> Self {
        Self { rest: text }
    }

    fn next(&mut self) -> Option<String> {
        loop {
            self.rest = self.rest.trim_start();
            if self.rest.starts_with('#') {
                let end = self.rest.find('\n').unwrap_or(self.rest.len());
                self.rest = &self.rest[end..];
                continue;
            }
            break;
        }
        if self.rest.is_empty() {
            return None;
        }
        let mut token = String::new();
        let mut characters = self.rest.char_indices();
        if self.rest.starts_with('"') {
            characters.next();
            let mut end = self.rest.len();
            while let Some((index, character)) = characters.next() {
                match character {
                    '\\' => {
                        if let Some((_, escaped)) = characters.next() {
                            token.push(escaped);
                        }
                    }
                    '"' => {
                        end = index + 1;
                        break;
                    }
                    other => token.push(other),
                }
            }
            self.rest = &self.rest[end..];
            return Some(token);
        }
        let end = self
            .rest
            .find(char::is_whitespace)
            .unwrap_or(self.rest.len());
        token.push_str(&self.rest[..end]);
        self.rest = &self.rest[end..];
        Some(token)
    }

    /// Skip a `macdef` body: every line up to the first blank one.
    fn skip_macro(&mut self) {
        let Some(line_end) = self.rest.find('\n') else {
            self.rest = "";
            return;
        };
        self.rest = &self.rest[line_end + 1..];
        loop {
            let end = self.rest.find('\n').unwrap_or(self.rest.len());
            let line = &self.rest[..end];
            self.rest = self.rest.get(end + 1..).unwrap_or("");
            if line.trim().is_empty() || self.rest.is_empty() {
                return;
            }
        }
    }
}

/// The version of a file a parse was taken from.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Version {
    path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
}

/// The last file parsed, and its version.
static PARSED: Mutex<Option<(Version, Netrc)>> = Mutex::new(None);

/// The `.netrc` credential for `host`, read from the file the environment
/// names now; `variable` reads one environment variable.
pub(crate) fn environment_authorization(
    host: &str,
    variable: impl Fn(&str) -> Option<String>,
) -> Option<Authorization> {
    let (path, metadata) = locate(variable)?;
    let version = Version {
        path,
        length: metadata.len(),
        modified: metadata.modified().ok(),
    };
    let mut parsed = PARSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((held, netrc)) = parsed.as_ref() {
        if *held == version {
            return netrc.authorization(host);
        }
    }
    let text = std::fs::read(&version.path).ok()?;
    let netrc = Netrc::parse(&String::from_utf8_lossy(&text));
    let answer = netrc.authorization(host);
    *parsed = Some((version, netrc));
    answer
}

/// The `.netrc` file the environment names, and what the file system says
/// of it: `NETRC`, else `.netrc` then `_netrc` in the home directory.
fn locate(variable: impl Fn(&str) -> Option<String>) -> Option<(PathBuf, std::fs::Metadata)> {
    let found = |path: PathBuf| {
        std::fs::metadata(&path)
            .ok()
            .filter(std::fs::Metadata::is_file)
            .map(|metadata| (path, metadata))
    };
    if let Some(path) = variable("NETRC") {
        return found(PathBuf::from(path));
    }
    let home = crate::local::LocalFolder::home().ok()?.path().ok()?;
    [".netrc", "_netrc"]
        .into_iter()
        .find_map(|name| found(home.join(name)))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/http/netrc.rs` pins and a caller cannot reach.
    //!
    //! The grammar and the lookup are pinned over text in hand, and the file
    //! lookup over a path named by a stand-in `NETRC`, so no test reads the
    //! home directory of the machine it runs on.
    use std::collections::BTreeMap;

    use crate::http::Authorization;

    /// The credential the `.netrc` text `text` holds for `host`.
    pub fn authorization(text: &str, host: &str) -> Option<Authorization> {
        super::Netrc::parse(text).authorization(host)
    }

    /// The credential for `host` from the file `variables` locate.
    pub fn environment_authorization(
        host: &str,
        variables: &[(&str, &str)],
    ) -> Option<Authorization> {
        let variables: BTreeMap<&str, &str> = variables.iter().copied().collect();
        super::environment_authorization(host, |name| {
            variables.get(name).map(|value| (*value).to_owned())
        })
    }
}
