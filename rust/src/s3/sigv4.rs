//! AWS Signature Version 4 for Amazon S3 requests (header-based authorization, single chunk).
//!
//! Pure: `std`, `sha2` and `hmac` only. The client builds the wire request, hands its parts to
//! [`Signer::sign`], and adds the headers it gets back. S3 differs from the generic `SigV4`
//! rules in two places this module honors: the canonical URI is the path exactly as sent (encoded
//! once, never re-encoded), and `x-amz-content-sha256` is always signed.

use std::fmt::Write as _;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

/// The `x-amz-content-sha256` value that skips payload hashing (HTTPS only).
/// What `x-amz-content-sha256` carries when the body is not hashed.
///
/// S3 accepts it in place of a real digest; the transport is then what
/// guarantees the body arrived intact.
pub(crate) const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

/// SHA-256 of the empty payload, lowercase hex.
pub(crate) const EMPTY_PAYLOAD_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Header names the signer emits itself; same-named entries in the caller's list are dropped so
/// the signature always covers the values actually sent.
const OWNED: [&str; 4] = [
    "host",
    "x-amz-date",
    "x-amz-content-sha256",
    "x-amz-security-token",
];

/// Lowercase hex SHA-256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Percent-encode one raw object key for the request path: every segment is encoded with the
/// `SigV4` unreserved set (`A-Z a-z 0-9 - _ . ~` kept, everything else `%XX` uppercase hex), `/`
/// separators kept. Never double-encodes (input is raw text, not already-encoded).
pub(crate) fn encode_key(key: &str) -> String {
    encode(key, true)
}

/// Percent-encode one query name or value with the same unreserved set (`/` IS encoded here).
pub(crate) fn encode_query_component(text: &str) -> String {
    encode(text, false)
}

/// Canonical query string: pairs sorted by encoded name then encoded value, `name=value` joined by
/// `&`; an empty value renders as `name=`. This is also the exact query string put on the wire.
pub(crate) fn canonical_query(query: &[(String, String)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(name, value)| (encode_query_component(name), encode_query_component(value)))
        .collect();
    pairs.sort();
    let pairs: Vec<String> = pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    pairs.join("&")
}

/// `YYYYMMDD` and `YYYYMMDDTHHMMSSZ` for a `SystemTime` (UTC, no chrono; civil-from-days). A time
/// before the epoch reads as the epoch.
pub(crate) fn amz_date(now: SystemTime) -> (String, String) {
    let seconds = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let (year, month, day) = civil_from_days(seconds / 86_400);
    let date = format!("{year:04}{month:02}{day:02}");
    let (hour, minute, second) = (seconds / 3600 % 24, seconds / 60 % 60, seconds % 60);
    let datetime = format!("{date}T{hour:02}{minute:02}{second:02}Z");
    (date, datetime)
}

/// One credential set bound to a region, with the per-day signing key cached.
pub(crate) struct Signer {
    /// The `Credential=` prefix of every authorization header.
    access_key_id: String,
    /// Only ever used as the root of the signing-key chain; never rendered.
    secret_access_key: String,
    /// Temporary-credential token, sent and signed as `x-amz-security-token` when present.
    session_token: Option<String>,
    /// The region in every credential scope.
    region: String,
    /// The service in every credential scope: `s3`, or `sts` for the one
    /// request that trades a role for a credential set.
    service: &'static str,
    /// (date `YYYYMMDD`, derived signing key) - recomputed when the day changes.
    key: Mutex<Option<(String, [u8; 32])>>,
}

impl Signer {
    /// Bind credentials to `region`; no key is derived until the first [`Signer::sign`].
    pub(crate) fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        session_token: Option<String>,
        region: impl Into<String>,
    ) -> Self {
        Self::for_service(
            "s3",
            access_key_id,
            secret_access_key,
            session_token,
            region,
        )
    }

    /// The same, for a service other than S3.
    ///
    /// The service is part of the credential scope and of the signing key, so
    /// a request to STS signed as `s3` is refused - which is why this exists
    /// rather than the scope being spelled once.
    pub(crate) fn for_service(
        service: &'static str,
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        session_token: Option<String>,
        region: impl Into<String>,
    ) -> Self {
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            session_token,
            region: region.into(),
            service,
            key: Mutex::new(None),
        }
    }

    /// The access key id every authorization header carries.
    ///
    /// The header the signer builds is where it otherwise appears; this reads
    /// it back for the client's own door, which holds a signer rather than a
    /// request to read the header off.
    #[cfg(feature = "internals")]
    pub(crate) fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    /// Produce the headers to add to the request, in this order:
    /// `x-amz-date`, `x-amz-content-sha256`, `x-amz-security-token` (when a token exists),
    /// `authorization`.
    ///
    /// * `method`: e.g. "GET".
    /// * `host`: the `Host` header value exactly as sent (with `:port` when non-default).
    /// * `path`: absolute request path as sent on the wire, already encoded by [`encode_key`]
    ///   (S3 canonical URI = the path as sent; do not re-encode). "/" for the root.
    /// * `query`: raw (unencoded) name/value pairs; the canonical query string is built with
    ///   [`canonical_query`].
    /// * `headers`: additional headers to sign, `(name, value)` with any case; the signer
    ///   lowercases names, trims values and collapses internal whitespace runs to one space,
    ///   sorts by name, and joins duplicate names with `,`. `host`, `x-amz-date`,
    ///   `x-amz-content-sha256` and (when present) `x-amz-security-token` are always signed even
    ///   when absent from `headers`, and the signer's values win over same-named entries.
    /// * `payload_hash`: `sha256_hex(body)` or [`EMPTY_PAYLOAD_SHA256`]. S3 also
    ///   accepts the literal `UNSIGNED-PAYLOAD` over TLS; this crate always signs
    ///   the real hash, so a store can verify what it received.
    /// * `now`: the signing time (injectable for the gold-vector tests).
    ///
    /// Algorithm: AWS4-HMAC-SHA256, scope `{date}/{region}/s3/aws4_request`, canonical request =
    /// method \n canonical URI \n canonical query \n canonical headers (each `name:value\n`) \n
    /// signed headers (`;`-joined) \n payload hash.
    // The argument list is the wire request's parts; a struct would only rename them.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sign(
        &self,
        method: &str,
        host: &str,
        path: &str,
        query: &[(String, String)],
        headers: &[(String, String)],
        payload_hash: &str,
        now: SystemTime,
    ) -> Vec<(String, String)> {
        let (date, datetime) = amz_date(now);
        let canonical_headers = self.canonical_headers(host, &datetime, payload_hash, headers);
        let request = canonical_request(method, path, query, &canonical_headers, payload_hash);
        let scope = format!("{date}/{}/{}/aws4_request", self.region, self.service);
        let signature = hex(&hmac_sha256(
            &self.signing_key(&date),
            string_to_sign(&datetime, &scope, &request).as_bytes(),
        ));
        let signed = signed_headers(&canonical_headers);
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
            self.access_key_id
        );
        let mut out = vec![
            ("x-amz-date".to_owned(), datetime),
            ("x-amz-content-sha256".to_owned(), payload_hash.to_owned()),
        ];
        if let Some(token) = &self.session_token {
            out.push(("x-amz-security-token".to_owned(), token.clone()));
        }
        out.push(("authorization".to_owned(), authorization));
        out
    }

    /// The canonical header list: the caller's headers plus the signer's own, names lowercased,
    /// values trimmed and whitespace-collapsed, sorted by name, duplicates joined with `,`.
    fn canonical_headers(
        &self,
        host: &str,
        datetime: &str,
        payload_hash: &str,
        headers: &[(String, String)],
    ) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = headers
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), collapse_whitespace(value)))
            .filter(|(name, _)| !OWNED.contains(&name.as_str()))
            .collect();
        entries.push(("host".to_owned(), host.to_owned()));
        entries.push(("x-amz-date".to_owned(), datetime.to_owned()));
        entries.push(("x-amz-content-sha256".to_owned(), payload_hash.to_owned()));
        if let Some(token) = &self.session_token {
            entries.push(("x-amz-security-token".to_owned(), token.clone()));
        }
        // Stable: duplicate names keep the caller's order when their values are joined.
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        let mut merged: Vec<(String, String)> = Vec::with_capacity(entries.len());
        for (name, value) in entries {
            match merged.last_mut() {
                Some((last, joined)) if *last == name => {
                    joined.push(',');
                    joined.push_str(&value);
                }
                _ => merged.push((name, value)),
            }
        }
        merged
    }

    /// The signing key for `date`, derived once per day: HMAC chained over `AWS4{secret}`,
    /// the date, the region, `s3` and `aws4_request`.
    fn signing_key(&self, date: &str) -> [u8; 32] {
        let mut cache = self.key.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((cached, key)) = cache.as_ref() {
            if cached == date {
                return *key;
            }
        }
        let secret = format!("AWS4{}", self.secret_access_key);
        let mut key = hmac_sha256(secret.as_bytes(), date.as_bytes());
        for part in [self.region.as_str(), self.service, "aws4_request"] {
            key = hmac_sha256(&key, part.as_bytes());
        }
        *cache = Some((date.to_owned(), key));
        key
    }
}

/// The canonical request over an already canonical header list.
fn canonical_request(
    method: &str,
    path: &str,
    query: &[(String, String)],
    canonical_headers: &[(String, String)],
    payload_hash: &str,
) -> String {
    let mut request = format!("{method}\n{path}\n{}\n", canonical_query(query));
    for (name, value) in canonical_headers {
        request.push_str(name);
        request.push(':');
        request.push_str(value);
        request.push('\n');
    }
    request.push('\n');
    request.push_str(&signed_headers(canonical_headers));
    request.push('\n');
    request.push_str(payload_hash);
    request
}

/// The `;`-joined names of a canonical header list.
fn signed_headers(canonical_headers: &[(String, String)]) -> String {
    let names: Vec<&str> = canonical_headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    names.join(";")
}

/// The string the signing key signs.
fn string_to_sign(datetime: &str, scope: &str, canonical_request: &str) -> String {
    format!(
        "AWS4-HMAC-SHA256\n{datetime}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    )
}

/// Trim and collapse every ASCII whitespace run to one space.
fn collapse_whitespace(value: &str) -> String {
    let words: Vec<&str> = value.split_ascii_whitespace().collect();
    words.join(" ")
}

/// Percent-encode with the `SigV4` unreserved set; `slash_kept` leaves `/` as a separator.
fn encode(text: &str, slash_kept: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        let kept = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (slash_kept && byte == b'/');
        if kept {
            out.push(char::from(byte));
        } else {
            // The Result of writing into a String is always Ok.
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Lowercase hex of `bytes`.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// HMAC-SHA256 tag of `data` under `key`.
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// Proleptic Gregorian `(year, month, day)` of a day count since 1970-01-01.
pub(crate) fn civil_from_days(days: u64) -> (u64, u64, u64) {
    // Howard Hinnant's algorithm, shifted so eras start on March 1st, 0000.
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/s3/sigv4.rs` pins and a caller cannot reach.
    //!
    //! The signature is what every S3 request stands or falls on, so it is
    //! pinned against AWS's own published example vectors - which means
    //! reaching the canonical request and the string to sign, not only the
    //! headers that come out. Each item here forwards to the real one, so
    //! nothing in this module is a visibility the crate would otherwise have.
    use std::sync::PoisonError;
    use std::time::SystemTime;

    /// SHA-256 of the empty payload, lowercase hex.
    pub const EMPTY_PAYLOAD_SHA256: &str = super::EMPTY_PAYLOAD_SHA256;

    /// What `x-amz-content-sha256` carries when the body is not hashed.
    pub const UNSIGNED_PAYLOAD: &str = super::UNSIGNED_PAYLOAD;

    /// Lowercase hex SHA-256 of `bytes`.
    pub fn sha256_hex(bytes: &[u8]) -> String {
        super::sha256_hex(bytes)
    }

    /// Percent-encode one raw object key for the request path.
    pub fn encode_key(key: &str) -> String {
        super::encode_key(key)
    }

    /// Percent-encode one query name or value, the separator included.
    pub fn encode_query_component(text: &str) -> String {
        super::encode_query_component(text)
    }

    /// The canonical query string, which is also what goes on the wire.
    pub fn canonical_query(query: &[(String, String)]) -> String {
        super::canonical_query(query)
    }

    /// The `(date, datetime)` pair every scope and `x-amz-date` is built from.
    pub fn amz_date(now: SystemTime) -> (String, String) {
        super::amz_date(now)
    }

    /// The canonical request over an already canonical header list.
    pub fn canonical_request(
        method: &str,
        path: &str,
        query: &[(String, String)],
        canonical_headers: &[(String, String)],
        payload_hash: &str,
    ) -> String {
        super::canonical_request(method, path, query, canonical_headers, payload_hash)
    }

    /// The string the signing key signs.
    pub fn string_to_sign(datetime: &str, scope: &str, canonical_request: &str) -> String {
        super::string_to_sign(datetime, scope, canonical_request)
    }

    /// One credential set bound to a region, forwarding to the real signer.
    pub struct Signer(super::Signer);

    impl Signer {
        /// Bind credentials to `region`; no key is derived until the first
        /// [`Signer::sign`].
        pub fn new(
            access_key_id: impl Into<String>,
            secret_access_key: impl Into<String>,
            session_token: Option<String>,
            region: impl Into<String>,
        ) -> Self {
            Self(super::Signer::new(
                access_key_id,
                secret_access_key,
                session_token,
                region,
            ))
        }

        /// The headers to add to the request, in the order they are emitted.
        // The argument list is the wire request's parts, exactly as the signer
        // takes them; a struct would only rename them.
        #[allow(clippy::too_many_arguments)]
        pub fn sign(
            &self,
            method: &str,
            host: &str,
            path: &str,
            query: &[(String, String)],
            headers: &[(String, String)],
            payload_hash: &str,
            now: SystemTime,
        ) -> Vec<(String, String)> {
            self.0
                .sign(method, host, path, query, headers, payload_hash, now)
        }

        /// The canonical header list one request signs over.
        pub fn canonical_headers(
            &self,
            host: &str,
            datetime: &str,
            payload_hash: &str,
            headers: &[(String, String)],
        ) -> Vec<(String, String)> {
            self.0
                .canonical_headers(host, datetime, payload_hash, headers)
        }

        /// The signing key derived for `date`.
        pub fn signing_key(&self, date: &str) -> [u8; 32] {
            self.0.signing_key(date)
        }

        /// The `(date, key)` pair the signer is holding, if it derived one.
        pub fn cached_key(&self) -> Option<(String, [u8; 32])> {
            self.0
                .key
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        /// The access key id every authorization header carries.
        pub fn access_key_id(&self) -> &str {
            &self.0.access_key_id
        }

        /// The region every credential scope names.
        pub fn region(&self) -> &str {
            &self.0.region
        }
    }
}
