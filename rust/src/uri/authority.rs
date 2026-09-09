//! URI authority and S3 host interpretation.

use super::*;

/// A validated URI authority.
///
/// The empty value is a concrete authority component used when a URI has no
/// authority; it is never represented as an optional or nullable value.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Authority(pub(super) SmolStr);

impl Authority {
    /// Parse and validate an authority component without surrounding `//`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the normalized authority without allocating.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Return whether this concrete authority is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Return the user name before the first user-information colon.
    pub fn user(&self) -> Option<&str> {
        let user_information = self.as_str().rsplit_once('@')?.0;
        Some(
            user_information
                .split_once(':')
                .map_or(user_information, |(user, _)| user),
        )
    }

    /// Return the password after the first user-information colon.
    ///
    /// Later colons belong to the password, so `user:pass:word@host` returns
    /// `pass:word` without allocating.
    pub fn password(&self) -> Option<&str> {
        self.as_str()
            .rsplit_once('@')?
            .0
            .split_once(':')
            .map(|(_, password)| password)
    }

    /// Return the host and optional port without user information.
    pub fn host_port(&self) -> &str {
        self.as_str()
            .rsplit_once('@')
            .map_or(self.as_str(), |(_, host_port)| host_port)
    }

    /// Return the host without user information, brackets, or a port.
    pub fn host(&self) -> &str {
        let host_port = self.host_port();
        if let Some(bracketed) = host_port.strip_prefix('[') {
            return bracketed
                .split_once(']')
                .map_or(bracketed, |(host, _)| host);
        }
        host_port
            .rsplit_once(':')
            .map_or(host_port, |(host, _)| host)
    }

    /// Return the explicit numeric port, when one was written.
    pub fn port(&self) -> Option<u16> {
        let host_port = self.host_port();
        let port = if let Some(bracketed) = host_port.strip_prefix('[') {
            let close = bracketed.find(']')?;
            bracketed[close + 1..].strip_prefix(':')?
        } else {
            host_port.rsplit_once(':')?.1
        };
        port.parse().ok()
    }

    /// Return a deterministic cross-language hash of the authority.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }
}

impl FromStr for Authority {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        validate_authority(value)?;
        Ok(Self(normalize_percent_hex(value)))
    }
}

pub(super) fn validate_authority(value: &str) -> Result<()> {
    validate_component(value, "authority", 0, is_authority_byte)?;
    if value.bytes().filter(|byte| *byte == b'@').count() > 1 {
        return Err(parse_error(
            "authority",
            value.find('@').unwrap_or(0),
            "authority may contain at most one user-information delimiter",
        ));
    }

    let (user_information, host_port) = value
        .rsplit_once('@')
        .map_or((None, value), |(user_information, host_port)| {
            (Some(user_information), host_port)
        });
    if let Some(position) = user_information.and_then(|part| part.find(['[', ']'])) {
        return Err(parse_error(
            "authority",
            position,
            "brackets are permitted only around an IP literal host",
        ));
    }
    if !value.is_empty() && host_port.is_empty() {
        return Err(parse_error(
            "authority",
            value.len(),
            "non-empty authority must contain a host",
        ));
    }
    if let Some(bracketed) = host_port.strip_prefix('[') {
        let Some(close) = bracketed.find(']') else {
            return Err(parse_error(
                "authority",
                value.len(),
                "bracketed host is missing its closing bracket",
            ));
        };
        if close == 0 {
            return Err(parse_error(
                "authority",
                value.len() - host_port.len() + 1,
                "bracketed host must not be empty",
            ));
        }
        let literal = &bracketed[..close];
        validate_ip_literal(literal, value.len() - host_port.len() + 1)?;
        let suffix = &bracketed[close + 1..];
        if !suffix.is_empty() {
            let port = suffix.strip_prefix(':').ok_or_else(|| {
                parse_error(
                    "authority",
                    value.len() - suffix.len(),
                    "characters after a bracketed host must form a numeric port",
                )
            })?;
            validate_port(port, value.len() - port.len())?;
        }
    } else {
        if let Some(position) = host_port.find(['[', ']']) {
            return Err(parse_error(
                "authority",
                value.len() - host_port.len() + position,
                "brackets are permitted only around an IP literal host",
            ));
        }
        let colon_count = host_port.bytes().filter(|byte| *byte == b':').count();
        if colon_count > 1 {
            return Err(parse_error(
                "authority",
                value.len() - host_port.len(),
                "IPv6 hosts must be enclosed in brackets",
            ));
        }
        if let Some((host, port)) = host_port.rsplit_once(':') {
            if host.is_empty() {
                return Err(parse_error(
                    "authority",
                    value.len() - host_port.len(),
                    "authority host must not be empty",
                ));
            }
            validate_port(port, value.len() - port.len())?;
        }
    }

    Ok(())
}

fn validate_port(port: &str, position: usize) -> Result<()> {
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(parse_error(
            "authority",
            position,
            "authority port must contain one or more decimal digits",
        ));
    }
    if port.parse::<u16>().is_err() {
        return Err(parse_error(
            "authority",
            position,
            "authority port must be between 0 and 65535",
        ));
    }
    Ok(())
}

fn validate_ip_literal(value: &str, base: usize) -> Result<()> {
    if let Some(future) = value.strip_prefix('v').or_else(|| value.strip_prefix('V')) {
        let Some((version, address)) = future.split_once('.') else {
            return Err(parse_error(
                "authority",
                base,
                "IPvFuture literal requires a hexadecimal version and address",
            ));
        };
        if version.is_empty()
            || !version.bytes().all(|byte| byte.is_ascii_hexdigit())
            || address.is_empty()
            || !address
                .bytes()
                .all(|byte| is_unreserved(byte) || is_sub_delimiter(byte) || byte == b':')
        {
            return Err(parse_error(
                "authority",
                base,
                "IPvFuture literal contains an invalid version or address",
            ));
        }
        return Ok(());
    }

    let (address, zone) = value
        .split_once("%25")
        .map_or((value, None), |(address, zone)| (address, Some(zone)));
    if address.parse::<Ipv6Addr>().is_err() {
        return Err(parse_error(
            "authority",
            base,
            "bracketed host must contain a valid IPv6 or IPvFuture literal",
        ));
    }
    if let Some(zone) = zone {
        if zone.is_empty() {
            return Err(parse_error(
                "authority",
                base + address.len() + 3,
                "IPv6 zone identifier must not be empty",
            ));
        }
        validate_component(zone, "authority", base + address.len() + 3, is_unreserved)?;
    }
    Ok(())
}

impl AsRef<str> for Authority {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Authority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for Authority {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Authority {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(D::Error::custom)
    }
}

impl Uri {
    /// The object-store location this URI names, when its scheme names a store.
    ///
    /// One walk of the authority and the path serves all three stores, because
    /// the question is the same in each: which part is the endpoint, which is
    /// the container, and what is left is the key. Only *recognizing a
    /// hostname* differs, and that is what [`StoreHost`] answers per store.
    pub(super) fn store_location(&self) -> Option<StoreLocation<'_>> {
        if !self.scheme.is_object_store() {
            return None;
        }

        // The cursor walks the path so whatever the endpoint and the container
        // do not consume is the key, spelled exactly as the path spells it.
        let mut cursor = 0;
        let next_segment = |cursor: &mut usize| {
            let (next, segment) = self.path.next_segment(*cursor)?;
            *cursor = next;
            Some(segment)
        };
        let key_of = |cursor: usize| {
            self.path
                .as_str()
                .get(cursor..)
                .unwrap_or_default()
                .trim_start_matches('/')
        };

        let authority =
            (self.has_authority && !self.authority.is_empty()).then_some(&self.authority);
        // The Hadoop Azure spellings put the container in the user position -
        // `abfss://data@account.dfs.core.windows.net/lake` - which no other
        // store does and no container name can be mistaken for.
        let attached = authority
            .filter(|_| self.scheme.is_az())
            .and_then(Authority::user)
            .filter(|container| !container.is_empty());
        // A port is something no container name carries, so it names the
        // endpoint before any suffix rule runs.
        if let Some(authority) = authority.filter(|authority| authority.port().is_some()) {
            let host = authority.host();
            // An Azure endpoint that is not one of the published hosts names no
            // account, so the emulators put it first in the path -
            // `az://127.0.0.1:10000/devstoreaccount1/lake/blob`. A container
            // attached to the authority already said which is which.
            let (account, bucket) = match (self.scheme.is_az(), attached) {
                (true, None) => {
                    let account = next_segment(&mut cursor);
                    (account, next_segment(&mut cursor))
                }
                (_, attached) => (None, attached.or_else(|| next_segment(&mut cursor))),
            };
            return Some(StoreLocation {
                hostname: Some(host),
                endpoint: Some(authority.host_port()),
                bucket,
                account,
                region: None,
                virtual_addressing: false,
                key: key_of(cursor),
            });
        }

        let first = match authority {
            Some(authority) => authority.host(),
            None => next_segment(&mut cursor)?,
        };

        if let Some(host) = StoreHost::parse(&self.scheme, first) {
            let bucket = attached
                .or(host.bucket)
                .or_else(|| next_segment(&mut cursor));
            return Some(StoreLocation {
                hostname: Some(first),
                endpoint: Some(host.endpoint),
                bucket,
                account: host.account,
                region: host.region,
                virtual_addressing: host.bucket.is_some(),
                key: key_of(cursor),
            });
        }
        if is_store_hostname(first) {
            let bucket = attached.or_else(|| next_segment(&mut cursor));
            return Some(StoreLocation {
                hostname: Some(first),
                endpoint: Some(first),
                bucket,
                account: None,
                region: None,
                virtual_addressing: false,
                key: key_of(cursor),
            });
        }
        Some(StoreLocation {
            hostname: None,
            endpoint: None,
            bucket: Some(first),
            account: None,
            region: None,
            virtual_addressing: false,
            key: key_of(cursor),
        })
    }
}

/// What an object-store location names, whichever store it addresses.
#[derive(Clone, Copy, Debug)]
pub(super) struct StoreLocation<'a> {
    pub(super) hostname: Option<&'a str>,
    pub(super) endpoint: Option<&'a str>,
    /// The bucket on Amazon S3 and Google Cloud Storage, the container on Azure.
    pub(super) bucket: Option<&'a str>,
    /// The Azure storage account, when the hostname or the path names one.
    pub(super) account: Option<&'a str>,
    pub(super) region: Option<&'a str>,
    pub(super) virtual_addressing: bool,
    pub(super) key: &'a str,
}

/// A hostname a store publishes, split into what it says.
#[derive(Clone, Copy, Debug)]
struct StoreHost<'a> {
    /// The endpoint, which is the hostname without a virtual container.
    endpoint: &'a str,
    /// The container the hostname carries, when it is virtual-hosted.
    bucket: Option<&'a str>,
    /// The region the hostname states, where a store puts one there.
    region: Option<&'a str>,
    /// The Azure storage account the hostname names.
    account: Option<&'a str>,
}

impl<'a> StoreHost<'a> {
    /// Read `hostname` as the store `scheme` names publishes it.
    fn parse(scheme: &Scheme, hostname: &'a str) -> Option<Self> {
        if scheme.is_s3() {
            let aws = parse_aws_s3_hostname(hostname)?;
            return Some(Self {
                endpoint: aws
                    .bucket
                    .map_or(hostname, |bucket| &hostname[bucket.len() + 1..]),
                bucket: aws.bucket,
                region: aws.region,
                account: None,
            });
        }
        if scheme.is_gs() {
            return parse_google_hostname(hostname);
        }
        parse_azure_hostname(hostname)
    }
}

/// Every Azure Storage host suffix, longest first so a nested one still wins.
const AZURE_SUFFIXES: [&str; 4] = [
    ".core.windows.net",
    ".core.chinacloudapi.cn",
    ".core.usgovcloudapi.net",
    ".core.cloudapi.de",
];

/// The blob and Data Lake services an Azure host can name.
const AZURE_SERVICES: [&str; 2] = ["blob", "dfs"];

/// Read `hostname` as a Google Cloud Storage endpoint.
///
/// `storage.googleapis.com` is the endpoint, `<bucket>.storage.googleapis.com`
/// the virtual-hosted form of the XML API, and
/// `storage.<region>.rep.googleapis.com` a regional endpoint that states its
/// own region.
fn parse_google_hostname(hostname: &str) -> Option<StoreHost<'_>> {
    let body = strip_suffix_ascii_case(hostname, ".googleapis.com")?;
    if is_google_endpoint(body) {
        return Some(StoreHost {
            endpoint: hostname,
            bucket: None,
            region: google_endpoint_region(body),
            account: None,
        });
    }
    // Whatever precedes a recognized endpoint is the bucket, and a bucket name
    // may itself hold dots, so the split is found from the right.
    let mut start = 0;
    loop {
        let dot = body[start..].find('.')?;
        start += dot + 1;
        if is_google_endpoint(&body[start..]) {
            return Some(StoreHost {
                endpoint: &hostname[start..],
                bucket: Some(&body[..start - 1]),
                region: google_endpoint_region(&body[start..]),
                account: None,
            });
        }
    }
}

/// The region a `storage.<region>.rep` endpoint states, when it states one.
fn google_endpoint_region(value: &str) -> Option<&str> {
    strip_prefix_ascii_case(value, "storage.")
        .and_then(|rest| strip_suffix_ascii_case(rest, ".rep"))
        .filter(|region| !region.is_empty() && !region.contains('.'))
}

/// Return whether `value` is a Google Cloud Storage endpoint's own label.
fn is_google_endpoint(value: &str) -> bool {
    value.eq_ignore_ascii_case("storage")
        || value.eq_ignore_ascii_case("www")
        || value.eq_ignore_ascii_case("storage.mtls")
        || google_endpoint_region(value).is_some()
}

/// Read `hostname` as an Azure Storage endpoint, naming the account it carries.
///
/// `account.blob.core.windows.net` is the blob service and
/// `account.dfs.core.windows.net` the Data Lake one; the container is never in
/// the hostname, so nothing here answers a bucket.
fn parse_azure_hostname(hostname: &str) -> Option<StoreHost<'_>> {
    let body = AZURE_SUFFIXES
        .iter()
        .find_map(|suffix| strip_suffix_ascii_case(hostname, suffix))?;
    let (account, service) = body.split_once('.')?;
    if account.is_empty()
        || !AZURE_SERVICES
            .iter()
            .any(|known| service.eq_ignore_ascii_case(known))
    {
        return None;
    }
    Some(StoreHost {
        endpoint: hostname,
        bucket: None,
        region: None,
        account: Some(account),
    })
}

#[derive(Clone, Copy, Debug)]
struct AwsS3Hostname<'a> {
    bucket: Option<&'a str>,
    region: Option<&'a str>,
}

/// Return whether a first component spells an endpoint rather than a container.
///
/// A name ending in `.com`, `.io`, or `.net` is a hostname, and so is
/// `localhost` or an IP literal: no container is named either, and a local
/// store speaking one of the three protocols is what those spellings mean. A
/// port settles it earlier still, in [`Uri::store_location`].
fn is_store_hostname(value: &str) -> bool {
    if value.eq_ignore_ascii_case("localhost") || value.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    [".com", ".io", ".net"].iter().any(|suffix| {
        value
            .get(value.len().saturating_sub(suffix.len())..)
            .is_some_and(|ending| ending.eq_ignore_ascii_case(suffix))
    })
}

fn strip_suffix_ascii_case<'a>(value: &'a str, suffix: &str) -> Option<&'a str> {
    let head = value.get(..value.len().checked_sub(suffix.len())?)?;
    value[head.len()..]
        .eq_ignore_ascii_case(suffix)
        .then_some(head)
}

fn strip_prefix_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let tail = value.get(prefix.len()..)?;
    value[..prefix.len()]
        .eq_ignore_ascii_case(prefix)
        .then_some(tail)
}

fn parse_aws_s3_hostname(hostname: &str) -> Option<AwsS3Hostname<'_>> {
    let body = strip_suffix_ascii_case(hostname, ".amazonaws.com.cn")
        .or_else(|| strip_suffix_ascii_case(hostname, ".amazonaws.com"))?;
    let mut start = 0;
    loop {
        let endpoint = &body[start..];
        if let Some(region) = parse_aws_s3_endpoint(endpoint) {
            return Some(AwsS3Hostname {
                bucket: (start != 0).then(|| &body[..start - 1]),
                region,
            });
        }
        let dot = body[start..].find('.')?;
        start += dot + 1;
    }
}

fn parse_aws_s3_endpoint(value: &str) -> Option<Option<&str>> {
    if value.eq_ignore_ascii_case("s3")
        || value.eq_ignore_ascii_case("s3-accelerate")
        || value.eq_ignore_ascii_case("s3-accelerate.dualstack")
    {
        return Some(None);
    }

    for prefix in ["s3.dualstack.", "s3-fips.dualstack.", "s3.", "s3-fips."] {
        if let Some(region) = strip_prefix_ascii_case(value, prefix) {
            return (!region.is_empty() && !region.contains('.')).then_some(Some(region));
        }
    }
    strip_prefix_ascii_case(value, "s3-")
        .and_then(|region| (!region.is_empty() && !region.contains('.')).then_some(Some(region)))
}
