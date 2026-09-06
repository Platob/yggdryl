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
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            session_token,
            region: region.into(),
            key: Mutex::new(None),
        }
    }

    /// The region every credential scope names.
    ///
    /// Only the tests read it back: the scope the signer builds is where it
    /// otherwise appears.
    #[cfg(test)]
    pub(crate) fn region(&self) -> &str {
        &self.region
    }

    /// The access key id every authorization header carries.
    ///
    /// Only the tests read it back: the header the signer builds is where it
    /// otherwise appears.
    #[cfg(test)]
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
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
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
        for part in [self.region.as_str(), "s3", "aws4_request"] {
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
fn civil_from_days(days: u64) -> (u64, u64, u64) {
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    // The examples of the S3 API reference page "Authenticating Requests: Using the Authorization
    // Header (AWS Signature Version 4)", with its documented credentials, host, region and time.
    const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
    const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    const HOST: &str = "examplebucket.s3.amazonaws.com";
    const SCOPE: &str = "20130524/us-east-1/s3/aws4_request";
    const CREDENTIAL: &str = "AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request";
    const MAY_24_2013: u64 = 1_369_353_600;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn example_signer(token: Option<&str>) -> Signer {
        Signer::new(
            ACCESS_KEY,
            SECRET_KEY,
            token.map(str::to_owned),
            "us-east-1",
        )
    }

    /// Canonical request, string to sign, and emitted headers for one example request.
    fn gold(
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
        payload_hash: &str,
    ) -> (String, String, Vec<(String, String)>) {
        let signer = example_signer(None);
        let (query, headers) = (pairs(query), pairs(headers));
        let canonical = signer.canonical_headers(HOST, "20130524T000000Z", payload_hash, &headers);
        let request = canonical_request(method, path, &query, &canonical, payload_hash);
        let to_sign = string_to_sign("20130524T000000Z", SCOPE, &request);
        let emitted = signer.sign(
            method,
            HOST,
            path,
            &query,
            &headers,
            payload_hash,
            at(MAY_24_2013),
        );
        (request, to_sign, emitted)
    }

    fn expected_headers(
        signed: &str,
        signature: &str,
        payload_hash: &str,
    ) -> Vec<(String, String)> {
        pairs(&[
            ("x-amz-date", "20130524T000000Z"),
            ("x-amz-content-sha256", payload_hash),
            (
                "authorization",
                &format!(
                    "AWS4-HMAC-SHA256 Credential={CREDENTIAL}, SignedHeaders={signed}, Signature={signature}"
                ),
            ),
        ])
    }

    #[test]
    fn get_object_with_a_range_matches_the_aws_example() {
        let (request, to_sign, emitted) = gold(
            "GET",
            "/test.txt",
            &[],
            &[("Range", "bytes=0-9")],
            EMPTY_PAYLOAD_SHA256,
        );
        assert_eq!(
            request,
            "GET\n\
             /test.txt\n\
             \n\
             host:examplebucket.s3.amazonaws.com\n\
             range:bytes=0-9\n\
             x-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n\
             x-amz-date:20130524T000000Z\n\
             \n\
             host;range;x-amz-content-sha256;x-amz-date\n\
             e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            to_sign,
            "AWS4-HMAC-SHA256\n\
             20130524T000000Z\n\
             20130524/us-east-1/s3/aws4_request\n\
             7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972"
        );
        assert_eq!(
            emitted,
            expected_headers(
                "host;range;x-amz-content-sha256;x-amz-date",
                "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41",
                EMPTY_PAYLOAD_SHA256,
            )
        );
    }

    #[test]
    fn put_object_matches_the_aws_example() {
        let payload_hash = sha256_hex(b"Welcome to Amazon S3.");
        assert_eq!(
            payload_hash,
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        let (request, to_sign, emitted) = gold(
            "PUT",
            &encode_key("/test$file.text"),
            &[],
            &[
                ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
                ("Date", "Fri, 24 May 2013 00:00:00 GMT"),
            ],
            &payload_hash,
        );
        assert_eq!(
            request,
            "PUT\n\
             /test%24file.text\n\
             \n\
             date:Fri, 24 May 2013 00:00:00 GMT\n\
             host:examplebucket.s3.amazonaws.com\n\
             x-amz-content-sha256:44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072\n\
             x-amz-date:20130524T000000Z\n\
             x-amz-storage-class:REDUCED_REDUNDANCY\n\
             \n\
             date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class\n\
             44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        assert_eq!(
            to_sign,
            "AWS4-HMAC-SHA256\n\
             20130524T000000Z\n\
             20130524/us-east-1/s3/aws4_request\n\
             9e0e90d9c76de8fa5b200d8c849cd5b8dc7a3be3951ddb7f6a76b4158342019d"
        );
        assert_eq!(
            emitted,
            expected_headers(
                "date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class",
                "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd",
                &payload_hash,
            )
        );
    }

    #[test]
    fn get_bucket_lifecycle_matches_the_aws_example() {
        let (request, to_sign, emitted) =
            gold("GET", "/", &[("lifecycle", "")], &[], EMPTY_PAYLOAD_SHA256);
        assert_eq!(
            request,
            "GET\n\
             /\n\
             lifecycle=\n\
             host:examplebucket.s3.amazonaws.com\n\
             x-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n\
             x-amz-date:20130524T000000Z\n\
             \n\
             host;x-amz-content-sha256;x-amz-date\n\
             e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            to_sign,
            "AWS4-HMAC-SHA256\n\
             20130524T000000Z\n\
             20130524/us-east-1/s3/aws4_request\n\
             9766c798316ff2757b517bc739a67f6213b4ab36dd5da2f94eaebf79c77395ca"
        );
        assert_eq!(
            emitted,
            expected_headers(
                "host;x-amz-content-sha256;x-amz-date",
                "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543",
                EMPTY_PAYLOAD_SHA256,
            )
        );
    }

    #[test]
    fn get_bucket_list_objects_matches_the_aws_example() {
        // Given out of order: the canonical query sorts by name.
        let (request, to_sign, emitted) = gold(
            "GET",
            "/",
            &[("prefix", "J"), ("max-keys", "2")],
            &[],
            EMPTY_PAYLOAD_SHA256,
        );
        assert_eq!(
            request,
            "GET\n\
             /\n\
             max-keys=2&prefix=J\n\
             host:examplebucket.s3.amazonaws.com\n\
             x-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n\
             x-amz-date:20130524T000000Z\n\
             \n\
             host;x-amz-content-sha256;x-amz-date\n\
             e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            to_sign,
            "AWS4-HMAC-SHA256\n\
             20130524T000000Z\n\
             20130524/us-east-1/s3/aws4_request\n\
             df57d21db20da04d7fa30298dd4488ba3a2b47ca3a489c74750e0f1e7df1b9b7"
        );
        assert_eq!(
            emitted,
            expected_headers(
                "host;x-amz-content-sha256;x-amz-date",
                "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7",
                EMPTY_PAYLOAD_SHA256,
            )
        );
    }

    #[test]
    fn the_empty_payload_constant_is_the_sha256_of_nothing() {
        assert_eq!(sha256_hex(b""), EMPTY_PAYLOAD_SHA256);
    }

    #[test]
    fn encode_key_keeps_separators_and_unreserved_bytes_and_escapes_the_rest() {
        assert_eq!(encode_key("a/b c.txt"), "a/b%20c.txt");
        assert_eq!(encode_key("caf\u{e9}/\u{1f600}"), "caf%C3%A9/%F0%9F%98%80");
        assert_eq!(encode_key("a+b=c~d%e"), "a%2Bb%3Dc~d%25e");
        assert_eq!(encode_key("-_.~09AZaz"), "-_.~09AZaz");
        assert_eq!(encode_key("already%20encoded"), "already%2520encoded");
        assert_eq!(encode_key(""), "");
    }

    #[test]
    fn encode_query_component_escapes_the_slash_too() {
        assert_eq!(encode_query_component("a/b c"), "a%2Fb%20c");
        assert_eq!(encode_query_component("x=&y"), "x%3D%26y");
        assert_eq!(encode_query_component("safe-_.~"), "safe-_.~");
    }

    #[test]
    fn canonical_query_sorts_by_name_then_value_and_renders_empty_values() {
        let query = pairs(&[
            ("prefix", "a/b"),
            ("list-type", "2"),
            ("delimiter", ""),
            ("continuation-token", "z"),
            ("continuation-token", "a"),
        ]);
        assert_eq!(
            canonical_query(&query),
            "continuation-token=a&continuation-token=z&delimiter=&list-type=2&prefix=a%2Fb"
        );
        assert_eq!(canonical_query(&[]), "");
    }

    #[test]
    fn amz_date_renders_known_epochs_in_utc() {
        let cases: [(u64, &str); 7] = [
            (0, "19700101T000000Z"),
            (MAY_24_2013, "20130524T000000Z"),
            (946_684_799, "19991231T235959Z"),
            (951_868_799, "20000229T235959Z"),
            (1_709_210_096, "20240229T123456Z"),
            (4_107_542_399, "21000228T235959Z"),
            (4_107_542_400, "21000301T000000Z"),
        ];
        for (seconds, expected) in cases {
            let (date, datetime) = amz_date(at(seconds));
            assert_eq!(datetime, expected);
            assert_eq!(date, &expected[..8]);
        }
        assert_eq!(amz_date(UNIX_EPOCH - Duration::from_secs(5)).0, "19700101");
    }

    #[test]
    fn the_signing_key_is_reused_within_a_day_and_rederived_across_days() {
        let signer = example_signer(None);
        let empty: [(String, String); 0] = [];
        let cached = |signer: &Signer| signer.key.lock().unwrap().clone();
        assert!(cached(&signer).is_none());
        signer.sign(
            "GET",
            HOST,
            "/",
            &empty,
            &empty,
            EMPTY_PAYLOAD_SHA256,
            at(MAY_24_2013),
        );
        let (date, key) = cached(&signer).unwrap();
        assert_eq!(date, "20130524");
        assert_eq!(key, signer.signing_key("20130524"));
        signer.sign(
            "GET",
            HOST,
            "/",
            &empty,
            &empty,
            EMPTY_PAYLOAD_SHA256,
            at(MAY_24_2013 + 86_399),
        );
        assert_eq!(cached(&signer), Some(("20130524".to_owned(), key)));
        signer.sign(
            "GET",
            HOST,
            "/",
            &empty,
            &empty,
            EMPTY_PAYLOAD_SHA256,
            at(MAY_24_2013 + 86_400),
        );
        let (next_date, next_key) = cached(&signer).unwrap();
        assert_eq!(next_date, "20130525");
        assert_ne!(next_key, key);
    }

    #[test]
    fn a_session_token_is_emitted_and_signed() {
        let signer = example_signer(Some("token/with+chars="));
        let empty: [(String, String); 0] = [];
        let emitted = signer.sign(
            "GET",
            HOST,
            "/",
            &empty,
            &empty,
            EMPTY_PAYLOAD_SHA256,
            at(0),
        );
        let names: Vec<&str> = emitted.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "x-amz-date",
                "x-amz-content-sha256",
                "x-amz-security-token",
                "authorization"
            ]
        );
        assert_eq!(emitted[2].1, "token/with+chars=");
        assert!(
            emitted[3].1.contains(
                "SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token,"
            )
        );
        let without = example_signer(None).sign(
            "GET",
            HOST,
            "/",
            &empty,
            &empty,
            EMPTY_PAYLOAD_SHA256,
            at(0),
        );
        assert_eq!(without.len(), 3);
        assert!(
            without[2]
                .1
                .contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date,")
        );
    }

    #[test]
    fn header_values_are_trimmed_collapsed_lowercased_and_merged() {
        let signer = example_signer(None);
        let headers = pairs(&[
            ("X-Amz-Meta-B", "  two   words\t here "),
            ("x-amz-meta-a", "first"),
            ("X-AMZ-META-A", "second"),
            ("Host", "ignored.example"),
            ("x-amz-date", "19990101T000000Z"),
        ]);
        let canonical = signer.canonical_headers("h:9000", "20130524T000000Z", "hash", &headers);
        assert_eq!(
            canonical,
            pairs(&[
                ("host", "h:9000"),
                ("x-amz-content-sha256", "hash"),
                ("x-amz-date", "20130524T000000Z"),
                ("x-amz-meta-a", "first,second"),
                ("x-amz-meta-b", "two words here"),
            ])
        );
    }

    #[test]
    fn accessors_answer_the_bound_credentials() {
        let signer = Signer::new("id", "secret", None, "eu-west-3");
        assert_eq!(signer.access_key_id(), "id");
        assert_eq!(signer.region(), "eu-west-3");
    }
}
