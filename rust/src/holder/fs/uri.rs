//! One URI boundary for local and S3-backed filesystem locations.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::{Authority, Error, Result, Uri};

use super::mask_uri;

/// S3 addressing selected for one resolved filesystem.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum S3AddressingStyle {
    /// Let the Arrow backend choose its normal addressing form.
    #[default]
    Automatic,
    /// Put the bucket in the request path.
    Path,
    /// Put the bucket in the request host.
    Virtual,
}

/// Filesystem configuration extracted from an S3 URI.
#[derive(Clone, Eq, PartialEq)]
pub struct S3FileSystemOptions {
    access_key: Option<Arc<str>>,
    secret_key: Option<Arc<str>>,
    session_token: Option<Arc<str>>,
    endpoint_override: Option<Arc<str>>,
    transport: Arc<str>,
    region: Option<Arc<str>>,
    anonymous: bool,
    addressing_style: S3AddressingStyle,
}

impl S3FileSystemOptions {
    /// Borrow the access key supplied by explicit configuration or URI user information.
    pub fn access_key(&self) -> Option<&str> {
        self.access_key.as_deref()
    }

    /// Borrow the secret key supplied by explicit configuration or URI user information.
    pub fn secret_key(&self) -> Option<&str> {
        self.secret_key.as_deref()
    }

    /// Borrow the optional session token.
    pub fn session_token(&self) -> Option<&str> {
        self.session_token.as_deref()
    }

    /// Borrow the endpoint host, with its explicit port when one was present.
    pub fn endpoint_override(&self) -> Option<&str> {
        self.endpoint_override.as_deref()
    }

    /// Borrow the configured endpoint transport, `http` or `https`.
    pub fn transport(&self) -> &str {
        &self.transport
    }

    /// Borrow the explicit or AWS-host-derived region.
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// Return whether requests are anonymous.
    pub const fn anonymous(&self) -> bool {
        self.anonymous
    }

    /// Return the requested path/virtual addressing form.
    pub const fn addressing_style(&self) -> S3AddressingStyle {
        self.addressing_style
    }
}

impl fmt::Debug for S3FileSystemOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("S3FileSystemOptions")
            .field("access_key", &self.access_key.as_ref().map(|_| "***"))
            .field("secret_key", &self.secret_key.as_ref().map(|_| "***"))
            .field("session_token", &self.session_token.as_ref().map(|_| "***"))
            .field("endpoint_override", &self.endpoint_override)
            .field("transport", &self.transport)
            .field("region", &self.region)
            .field("anonymous", &self.anonymous)
            .field("addressing_style", &self.addressing_style)
            .finish()
    }
}

/// Which Arrow filesystem a URI selects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedFileSystem {
    /// The Arrow local filesystem.
    Local,
    /// The Arrow S3 filesystem with complete configuration.
    S3(S3FileSystemOptions),
}

/// One URI resolved once into filesystem configuration and an opaque path.
#[derive(Clone, Eq, PartialEq)]
pub struct ResolvedFileSystemUri {
    filesystem: ResolvedFileSystem,
    path: Arc<str>,
    bucket: Option<Arc<str>>,
    key: Option<Arc<str>>,
    uri: Arc<str>,
    masked_uri: Arc<str>,
}

impl ResolvedFileSystemUri {
    /// Resolve `file`, `s3`, `s3a`, or `s3n` exactly once.
    ///
    /// S3 percent escapes remain literal characters in [`Self::path`] and
    /// [`Self::key`]. Option values are URI-decoded because they configure the
    /// filesystem rather than name an object.
    pub fn from_uri(
        uri: impl Into<String>,
        options: Option<&BTreeMap<String, String>>,
    ) -> Result<Self> {
        let uri = uri.into();
        // Validate syntax with the one core URI grammar. Raw slices below are
        // deliberate: rebuilding the parsed URI would normalize escape text.
        let parsed = Uri::from_str(&uri)?;
        let scheme = parsed.scheme().as_str().to_owned();
        match scheme.as_str() {
            "file" => resolve_file(uri, parsed),
            "s3" | "s3a" | "s3n" => resolve_s3(uri, parsed, options),
            _ => Err(Error::Unsupported {
                operation: "filesystem URI scheme",
                filesystem: scheme.into(),
            }),
        }
    }

    /// Borrow the resolved backend configuration.
    pub const fn filesystem(&self) -> &ResolvedFileSystem {
        &self.filesystem
    }

    /// Borrow the exact opaque path to pass to the selected filesystem.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Borrow the S3 bucket, when applicable.
    pub fn bucket(&self) -> Option<&str> {
        self.bucket.as_deref()
    }

    /// Borrow the exact escaped S3 object key, excluding the bucket.
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    /// Borrow the caller's exact URI spelling. This may contain credentials.
    pub fn uri(&self) -> &str {
        &self.uri
    }

    /// Borrow the credential-free diagnostic URI.
    pub fn masked_uri(&self) -> &str {
        &self.masked_uri
    }
}

impl fmt::Debug for ResolvedFileSystemUri {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedFileSystemUri")
            .field("filesystem", &self.filesystem)
            .field("path", &self.path)
            .field("bucket", &self.bucket)
            .field("key", &self.key)
            .field("uri", &self.masked_uri)
            .finish()
    }
}

fn resolve_file(uri: String, parsed: Uri) -> Result<ResolvedFileSystemUri> {
    let path = parsed.into_path()?;
    let path = path_to_string(path)?;
    Ok(ResolvedFileSystemUri {
        filesystem: ResolvedFileSystem::Local,
        path: Arc::from(path),
        bucket: None,
        key: None,
        masked_uri: Arc::from(mask_uri(&uri)),
        uri: Arc::from(uri),
    })
}

fn path_to_string(path: PathBuf) -> Result<String> {
    path.into_os_string().into_string().map_err(|_| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file URI resolved to a path that is not valid UTF-8",
        ))
    })
}

fn resolve_s3(
    uri: String,
    parsed: Uri,
    explicit: Option<&BTreeMap<String, String>>,
) -> Result<ResolvedFileSystemUri> {
    let raw = raw_hierarchical(&uri)?;
    if parsed.authority().is_empty() {
        return Err(invalid_uri("S3 URI requires an authority"));
    }

    let mut values = parse_query(parsed.query())?;
    if let Some(explicit) = explicit {
        for (key, value) in explicit {
            values.insert(key.to_ascii_lowercase(), value.clone());
        }
    }
    validate_option_names(&values)?;

    let mut access_key = parsed
        .user()
        .map(|access| percent_decode(access, "S3 access key"))
        .transpose()?;
    let mut secret_key = parsed
        .password()
        .map(|secret| percent_decode(secret, "S3 secret key"))
        .transpose()?;
    replace_option(&mut access_key, &values, "access_key");
    replace_option(&mut secret_key, &values, "secret_key");
    let session_token = values.get("session_token").cloned();

    let mut endpoint_override = values.get("endpoint_override").cloned();
    let region = values
        .get("region")
        .cloned()
        .or_else(|| parsed.region().map(str::to_owned));
    let mut addressing_style = parse_addressing(&values)?;
    let uri_endpoint = parsed.s3_endpoint();
    if let Some(endpoint) = uri_endpoint {
        endpoint_override.get_or_insert_with(|| endpoint.to_owned());
    }
    if let Some(endpoint) = endpoint_override.as_mut() {
        let authority = Authority::from_str(endpoint)
            .map_err(|_| invalid_option("endpoint_override", "expected host or host:port"))?;
        if authority.user().is_some() {
            return Err(invalid_option(
                "endpoint_override",
                "user information is not allowed",
            ));
        }
        *endpoint = authority.host_port().to_owned();
    }
    let (bucket, key) = if parsed.is_s3_virtual() {
        if addressing_style == S3AddressingStyle::Automatic {
            addressing_style = S3AddressingStyle::Virtual;
        }
        (
            parsed
                .bucket()
                .ok_or_else(|| invalid_uri("S3 URI requires a bucket"))?
                .to_owned(),
            raw.path.to_owned(),
        )
    } else if uri_endpoint.is_some() {
        split_bucket(raw.path)?
    } else {
        (
            parsed
                .bucket()
                .ok_or_else(|| invalid_uri("S3 URI requires a bucket"))?
                .to_owned(),
            raw.path.to_owned(),
        )
    };
    if bucket.is_empty() {
        return Err(invalid_uri("S3 URI requires a bucket"));
    }

    let path = if key.is_empty() {
        bucket.clone()
    } else {
        format!("{bucket}/{key}")
    };
    let transport = values
        .get("scheme")
        .map(String::as_str)
        .unwrap_or("https")
        .to_ascii_lowercase();
    if !matches!(transport.as_str(), "http" | "https") {
        return Err(invalid_option("scheme", "expected http or https"));
    }
    let anonymous = values
        .get("anonymous")
        .map(|value| parse_bool("anonymous", value))
        .transpose()?
        .unwrap_or(false);

    let options = S3FileSystemOptions {
        access_key: access_key.map(Arc::from),
        secret_key: secret_key.map(Arc::from),
        session_token: session_token.map(Arc::from),
        endpoint_override: endpoint_override.map(Arc::from),
        transport: Arc::from(transport),
        region: region.map(Arc::from),
        anonymous,
        addressing_style,
    };
    Ok(ResolvedFileSystemUri {
        filesystem: ResolvedFileSystem::S3(options),
        path: Arc::from(path),
        bucket: Some(Arc::from(bucket)),
        key: Some(Arc::from(key)),
        masked_uri: Arc::from(mask_uri(&uri)),
        uri: Arc::from(uri),
    })
}

fn replace_option(target: &mut Option<String>, values: &BTreeMap<String, String>, key: &str) {
    if let Some(value) = values.get(key) {
        *target = Some(value.clone());
    }
}

const OPTION_NAMES: [&str; 10] = [
    "access_key",
    "secret_key",
    "session_token",
    "endpoint_override",
    "scheme",
    "region",
    "anonymous",
    "addressing_style",
    "force_virtual_addressing",
    "force_path_style",
];

fn validate_option_names(values: &BTreeMap<String, String>) -> Result<()> {
    if let Some(key) = values
        .keys()
        .find(|key| !OPTION_NAMES.contains(&key.as_str()))
    {
        return Err(invalid_option(key, "unknown S3 filesystem option"));
    }
    Ok(())
}

fn parse_addressing(values: &BTreeMap<String, String>) -> Result<S3AddressingStyle> {
    let mut style = match values.get("addressing_style").map(String::as_str) {
        None | Some("auto" | "automatic") => S3AddressingStyle::Automatic,
        Some("path") => S3AddressingStyle::Path,
        Some("virtual") => S3AddressingStyle::Virtual,
        Some(_) => {
            return Err(invalid_option(
                "addressing_style",
                "expected automatic, path, or virtual",
            ));
        }
    };
    if let Some(value) = values.get("force_virtual_addressing") {
        if parse_bool("force_virtual_addressing", value)? {
            if style == S3AddressingStyle::Path {
                return Err(invalid_option(
                    "force_virtual_addressing",
                    "conflicts with path addressing",
                ));
            }
            style = S3AddressingStyle::Virtual;
        }
    }
    if let Some(value) = values.get("force_path_style") {
        if parse_bool("force_path_style", value)? {
            if style == S3AddressingStyle::Virtual {
                return Err(invalid_option(
                    "force_path_style",
                    "conflicts with virtual addressing",
                ));
            }
            style = S3AddressingStyle::Path;
        }
    }
    Ok(style)
}

fn parse_bool(key: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(invalid_option(key, "expected a boolean")),
    }
}

fn parse_query(query: Option<&str>) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    let Some(query) = query else {
        return Ok(values);
    };
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(key, "S3 option name")?.to_ascii_lowercase();
        if values.contains_key(&key) {
            return Err(invalid_option(&key, "duplicate S3 filesystem option"));
        }
        values.insert(key, percent_decode(value, "S3 option value")?);
    }
    Ok(values)
}

fn percent_decode(value: &str, target: &'static str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        let high = bytes.get(index + 1).and_then(|byte| hex(*byte));
        let low = bytes.get(index + 2).and_then(|byte| hex(*byte));
        let (Some(high), Some(low)) = (high, low) else {
            return Err(Error::Parse {
                target,
                position: index,
                reason: "percent escape must contain two hexadecimal digits".into(),
            });
        };
        decoded.push((high << 4) | low);
        index += 3;
    }
    String::from_utf8(decoded).map_err(|_| Error::Parse {
        target,
        position: 0,
        reason: "percent escapes must decode to UTF-8".into(),
    })
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

struct RawUri<'a> {
    path: &'a str,
}

fn raw_hierarchical(uri: &str) -> Result<RawUri<'_>> {
    let marker = uri
        .find("://")
        .ok_or_else(|| invalid_uri("filesystem URI requires an authority marker"))?;
    let start = marker + 3;
    let fragment = uri[start..]
        .find('#')
        .map_or(uri.len(), |offset| start + offset);
    let query_start = uri[start..fragment].find('?').map(|offset| start + offset);
    let hierarchy_end = query_start.unwrap_or(fragment);
    let slash = uri[start..hierarchy_end]
        .find('/')
        .map_or(hierarchy_end, |offset| start + offset);
    // Remove exactly the URI path delimiter. Any further slash is part of the
    // object path and must remain visible to the filesystem.
    let path = if slash < hierarchy_end {
        &uri[slash + 1..hierarchy_end]
    } else {
        ""
    };
    Ok(RawUri { path })
}

fn split_bucket(path: &str) -> Result<(String, String)> {
    let (bucket, key) = path
        .split_once('/')
        .map_or((path, ""), |(bucket, key)| (bucket, key));
    if bucket.is_empty() {
        return Err(invalid_uri(
            "S3 endpoint URI requires a bucket path segment",
        ));
    }
    Ok((bucket.to_owned(), key.to_owned()))
}

fn invalid_uri(reason: impl Into<smol_str::SmolStr>) -> Error {
    Error::Parse {
        target: "filesystem URI",
        position: 0,
        reason: reason.into(),
    }
}

fn invalid_option(key: &str, reason: &str) -> Error {
    Error::Parse {
        target: "S3 filesystem options",
        position: 0,
        reason: format!("{key}: {reason}").into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s3(uri: &str) -> ResolvedFileSystemUri {
        ResolvedFileSystemUri::from_uri(uri, None).unwrap()
    }

    #[test]
    fn required_s3_shapes_separate_configuration_bucket_and_literal_key() {
        for uri in [
            "s3://bucket/v=a%2Fb",
            "s3a://bucket/v=a%2Fb",
            "s3n://bucket/v=a%2Fb",
        ] {
            let resolved = s3(uri);
            assert_eq!(resolved.bucket(), Some("bucket"));
            assert_eq!(resolved.key(), Some("v=a%2Fb"));
            assert_eq!(resolved.path(), "bucket/v=a%2Fb");
        }
        assert_eq!(s3("s3://bucket/v=a%2fb").path(), "bucket/v=a%2fb");

        let credentialed = s3("s3://key:sec:ret@bucket/key");
        let ResolvedFileSystem::S3(options) = credentialed.filesystem() else {
            panic!("expected S3")
        };
        assert_eq!(options.access_key(), Some("key"));
        assert_eq!(options.secret_key(), Some("sec:ret"));
        assert_eq!(credentialed.path(), "bucket/key");

        let endpoint = s3("s3://key:secret@minio:9000/bucket/key");
        let ResolvedFileSystem::S3(options) = endpoint.filesystem() else {
            panic!("expected S3")
        };
        assert_eq!(options.endpoint_override(), Some("minio:9000"));
        assert_eq!(endpoint.bucket(), Some("bucket"));
        assert_eq!(endpoint.key(), Some("key"));

        let configured =
            s3("s3://bucket/key?endpoint_override=minio%3A9000&scheme=http&region=eu-west-1");
        let ResolvedFileSystem::S3(options) = configured.filesystem() else {
            panic!("expected S3")
        };
        assert_eq!(options.endpoint_override(), Some("minio:9000"));
        assert_eq!(options.transport(), "http");
        assert_eq!(options.region(), Some("eu-west-1"));

        let virtual_host = s3("s3://bucket.s3.eu-west-1.amazonaws.com/key");
        let ResolvedFileSystem::S3(options) = virtual_host.filesystem() else {
            panic!("expected S3")
        };
        assert_eq!(virtual_host.bucket(), Some("bucket"));
        assert_eq!(virtual_host.key(), Some("key"));
        assert_eq!(
            options.endpoint_override(),
            Some("s3.eu-west-1.amazonaws.com")
        );
        assert_eq!(options.region(), Some("eu-west-1"));
        assert_eq!(options.addressing_style(), S3AddressingStyle::Virtual);
    }

    #[test]
    fn raw_object_path_characters_and_secrets_never_change_or_leak() {
        let resolved =
            s3("s3://access:never-show-this@bucket/v=a%2Fb//x%25+y?session_token=hidden");
        assert_eq!(resolved.path(), "bucket/v=a%2Fb//x%25+y");
        assert_eq!(resolved.key(), Some("v=a%2Fb//x%25+y"));
        assert_eq!(
            resolved.uri(),
            "s3://access:never-show-this@bucket/v=a%2Fb//x%25+y?session_token=hidden"
        );
        let debug = format!("{resolved:?}");
        assert!(!debug.contains("never-show-this"));
        assert!(!debug.contains("hidden"));
        assert!(!resolved.masked_uri().contains("never-show-this"));
        assert!(!resolved.masked_uri().contains("hidden"));
    }
}
