//! A filesystem instance bound to one opaque path.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::{Result, Scheme, Url};

use super::FileSystem;

/// One resource identity: filesystem equality domain plus exact raw path.
///
/// Its hash intentionally uses only the path. Filesystems expose equality but
/// Arrow does not expose a corresponding hash; equal locations therefore hash
/// alike, while unequal filesystem domains may harmlessly collide.
#[derive(Clone)]
pub struct BoundLocationIdentity {
    filesystem: Arc<dyn FileSystem>,
    path: Arc<str>,
}

impl PartialEq for BoundLocationIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.filesystem.equals(other.filesystem.as_ref())
    }
}

impl Eq for BoundLocationIdentity {}

impl Hash for BoundLocationIdentity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}

impl fmt::Debug for BoundLocationIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundLocationIdentity")
            .field("filesystem", &self.filesystem.type_name())
            .field("path", &"<opaque>")
            .finish()
    }
}

/// A filesystem and its exact path, with separate caller and diagnostic URIs.
#[derive(Clone)]
pub struct BoundLocation {
    filesystem: Arc<dyn FileSystem>,
    path: Arc<str>,
    uri: Option<Arc<str>>,
    masked_uri: Option<Arc<str>>,
    diagnostic_url: Url,
}

impl BoundLocation {
    /// Bind an injected filesystem path without parsing or normalizing it.
    ///
    /// `uri`, when supplied, is retained exactly. It is never used to derive
    /// the backend path.
    pub fn new(
        filesystem: Arc<dyn FileSystem>,
        path: impl Into<String>,
        uri: Option<String>,
    ) -> Result<Self> {
        let path = path.into();
        let uri = uri.map(Arc::<str>::from);
        let masked_uri = uri.as_deref().map(mask_uri).map(Arc::<str>::from);
        let diagnostic_url = diagnostic_url(filesystem.as_ref(), &path)?;
        Ok(Self {
            filesystem,
            path: Arc::from(path),
            uri,
            masked_uri,
            diagnostic_url,
        })
    }

    /// Borrow the exact filesystem instance/equality domain.
    pub fn filesystem(&self) -> &Arc<dyn FileSystem> {
        &self.filesystem
    }

    /// Borrow the exact opaque path passed to the filesystem.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Borrow the exact optional URI spelling supplied by the caller.
    pub fn uri(&self) -> Option<&str> {
        self.uri.as_deref()
    }

    /// Borrow the credential-free URI used for diagnostics.
    pub fn masked_uri(&self) -> Option<&str> {
        self.masked_uri.as_deref()
    }

    /// Borrow the credential-free URL used by generic media/path behavior.
    pub const fn diagnostic_url(&self) -> &Url {
        &self.diagnostic_url
    }

    /// Return the opaque, hashable identity of this location.
    pub fn identity(&self) -> BoundLocationIdentity {
        BoundLocationIdentity {
            filesystem: Arc::clone(&self.filesystem),
            path: Arc::clone(&self.path),
        }
    }

    /// Return whether another value addresses the same exact location.
    pub fn same_location(&self, other: &Self) -> bool {
        self.path == other.path && self.filesystem.equals(other.filesystem.as_ref())
    }

    /// Fallible equality for host-runtime filesystem adapters.
    pub fn try_same_location(&self, other: &Self) -> Result<bool> {
        if self.path != other.path {
            return Ok(false);
        }
        self.filesystem.try_equals(other.filesystem.as_ref())
    }

    /// Bind a raw child name without URL decoding or path normalization.
    pub fn child(&self, name: &str) -> Result<Self> {
        // The empty name and `.` both name this location: a caller asking
        // for its own handle on a folder gets one, not a `./` the store
        // would keep as a segment of its own.
        if name.is_empty() || name == "." {
            return Ok(self.clone());
        }
        let path = if self.path.is_empty() {
            name.to_owned()
        } else if is_filesystem_root(self.path()) {
            format!("{}{name}", self.path)
        } else {
            format!("{}/{name}", self.path)
        };
        let uri = self.uri.as_deref().map(|uri| uri_join(uri, name));
        Self::new(Arc::clone(&self.filesystem), path, uri)
    }

    /// Bind an exact path returned by this filesystem.
    pub fn listed(&self, path: impl Into<String>) -> Result<Self> {
        let path = path.into();
        let uri = relative_to(self.path(), &path)
            .and_then(|relative| self.uri.as_deref().map(|uri| uri_join(uri, relative)));
        Self::new(Arc::clone(&self.filesystem), path, uri)
    }

    /// Return the raw parent, preserving repeated separators and escapes.
    pub fn parent(&self) -> Option<Result<Self>> {
        let path = self.path.strip_suffix('/').unwrap_or(self.path());
        let (mut parent, _) = path.rsplit_once('/')?;
        let root;
        if parent.is_empty() {
            if path.starts_with('/') {
                parent = "/";
            } else {
                return None;
            }
        } else if parent.len() == 2
            && parent.as_bytes()[1] == b':'
            && path.as_bytes().get(2) == Some(&b'/')
        {
            root = format!("{parent}/");
            parent = &root;
        }
        let uri = self.uri.as_deref().and_then(uri_parent);
        Some(Self::new(
            Arc::clone(&self.filesystem),
            parent.to_owned(),
            uri,
        ))
    }
}

impl fmt::Debug for BoundLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = mask_uri(self.path());
        formatter
            .debug_struct("BoundLocation")
            .field("filesystem", &self.filesystem.type_name())
            .field("path", &path)
            .field("uri", &self.masked_uri)
            .finish()
    }
}

impl fmt::Display for BoundLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.masked_uri() {
            Some(uri) => formatter.write_str(uri),
            None => formatter.write_str(&mask_uri(self.path())),
        }
    }
}

fn relative_to<'path>(base: &str, path: &'path str) -> Option<&'path str> {
    if base == path {
        return Some("");
    }
    if base.is_empty() {
        return Some(path);
    }
    let suffix = path.strip_prefix(base)?;
    if base.ends_with('/') {
        Some(suffix.strip_prefix('/').unwrap_or(suffix))
    } else {
        suffix.strip_prefix('/')
    }
}

fn uri_join(uri: &str, relative: &str) -> String {
    let (head, suffix) = split_suffix(uri);
    let separator = if uri_head_is_root(head) { "" } else { "/" };
    format!("{head}{separator}{relative}{suffix}")
}

fn is_filesystem_root(path: &str) -> bool {
    path == "/"
        || path.len() == 3
            && path.as_bytes()[1] == b':'
            && path.as_bytes()[2] == b'/'
            && path.as_bytes()[0].is_ascii_alphabetic()
}

fn uri_head_is_root(head: &str) -> bool {
    let Some(authority) = head.find("://").map(|marker| marker + 3) else {
        return false;
    };
    let Some(path_start) = head[authority..].find('/').map(|offset| authority + offset) else {
        return false;
    };
    let path = &head[path_start..];
    path == "/"
        || path.len() == 4
            && path.as_bytes()[0] == b'/'
            && path.as_bytes()[2] == b':'
            && path.as_bytes()[3] == b'/'
            && path.as_bytes()[1].is_ascii_alphabetic()
}

fn uri_parent(uri: &str) -> Option<String> {
    let (head, suffix) = split_suffix(uri);
    let scheme_end = head.find("://").map_or(0, |index| index + 3);
    let trimmed = head.strip_suffix('/').unwrap_or(head);
    let split = trimmed.rfind('/')?;
    if split < scheme_end {
        return None;
    }
    if split == scheme_end {
        return Some(format!("{}{suffix}", &trimmed[..=split]));
    }
    Some(format!("{}{suffix}", &trimmed[..split]))
}

fn split_suffix(uri: &str) -> (&str, &str) {
    let split = uri.find(['?', '#']).unwrap_or(uri.len());
    uri.split_at(split)
}

/// Mask URI user information and credential-like query values.
pub fn mask_uri(uri: &str) -> String {
    let mut masked = uri.to_owned();
    let mut search = 0;
    while let Some(scheme) = masked[search..].find("://") {
        let authority = search + scheme + 3;
        let end = masked[authority..]
            .find(|character: char| {
                matches!(character, '/' | '?' | '#') || character.is_whitespace()
            })
            .map_or(masked.len(), |offset| authority + offset);
        if let Some(at) = masked[authority..end].rfind('@') {
            let at = authority + at;
            masked.replace_range(authority..at, "***");
        }
        search = (authority + 3).min(masked.len());
    }

    let mut cursor = 0;
    while let Some(relative) = masked[cursor..].find(['=', ':']) {
        let delimiter = cursor + relative;
        let bytes = masked.as_bytes();
        let mut key_end = delimiter;
        while key_end > 0 && bytes[key_end - 1].is_ascii_whitespace() {
            key_end -= 1;
        }
        let mut key_start = key_end;
        while key_start > 0 && !credential_field_boundary(bytes[key_start - 1]) {
            key_start -= 1;
        }
        if sensitive_credential_key(&masked[key_start..key_end]) {
            let mut value_start = delimiter + 1;
            while masked
                .as_bytes()
                .get(value_start)
                .is_some_and(u8::is_ascii_whitespace)
            {
                value_start += 1;
            }
            let quote = masked
                .as_bytes()
                .get(value_start)
                .copied()
                .filter(|byte| matches!(byte, b'\'' | b'"'));
            if quote.is_some() {
                value_start += 1;
            }
            let value_end = masked.as_bytes()[value_start..]
                .iter()
                .position(|byte| {
                    quote.map_or_else(|| credential_value_boundary(*byte), |quote| *byte == quote)
                })
                .map_or(masked.len(), |offset| value_start + offset);
            masked.replace_range(value_start..value_end, "***");
            cursor = value_start + 3;
        } else {
            cursor = delimiter + 1;
        }
    }
    masked
}

fn credential_field_boundary(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'?' | b'&' | b'#' | b';' | b',' | b'\'' | b'"' | b'(' | b'[' | b'{'
        )
}

fn credential_value_boundary(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'&' | b'#' | b';' | b',' | b'\'' | b'"' | b')' | b']' | b'}'
        )
}

fn sensitive_credential_key(key: &str) -> bool {
    let normalized = decode_query_key(key).to_ascii_lowercase().replace('-', "_");
    matches!(
        normalized.as_str(),
        "access_key"
            | "access_key_id"
            | "aws_access_key_id"
            | "secret_key"
            | "secret_access_key"
            | "aws_secret_access_key"
            | "session_token"
            | "aws_session_token"
            | "security_token"
            | "x_amz_security_token"
            | "credential"
            | "x_amz_credential"
            | "signature"
            | "x_amz_signature"
            | "token"
            | "password"
    ) || normalized.contains("secret")
        || normalized.ends_with("_token")
        || normalized.ends_with("_credential")
        || normalized.ends_with("_signature")
}

fn decode_query_key(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if let (Some(high), Some(low)) = (
                bytes.get(index + 1).and_then(|byte| hex(*byte)),
                bytes.get(index + 2).and_then(|byte| hex(*byte)),
            ) {
                decoded.push((high << 4 | low) as char);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index] as char);
        index += 1;
    }
    decoded
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn diagnostic_url(filesystem: &dyn FileSystem, path: &str) -> Result<Url> {
    let scheme = Scheme::from_str(filesystem.type_name()).or_else(|_| Scheme::from_str("fs"))?;
    let safe_path = mask_uri(path);
    let encoded = safe_path
        .split('/')
        .map(encode_component)
        .collect::<Vec<_>>()
        .join("/");
    Url::from_str(&format!("{}://bound/{encoded}", scheme.as_str()))
}

fn encode_component(component: &str) -> String {
    let mut encoded = String::with_capacity(component.len());
    for byte in component.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'=' | b'+') {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}
