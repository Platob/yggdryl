//! `rust/src/aws/sigv4.rs`: the request signing no caller can name.
//!
//! The signature is what every AWS request stands or falls on, so it is
//! pinned to what the reference implementations produce - the canonical
//! request, the string to sign and the headers that come out: AWS's published
//! vectors for four S3 requests, and botocore's for a service outside the S3
//! family, whose canonical URI follows the other rule. What a caller observes
//! of a signed request is pinned in `rust/tests/s3/client.rs` and
//! `rust/tests/aws/request.rs`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

// The object key encoder is the S3 client's, and built with it.
#[cfg(feature = "s3")]
use yggdryl::internals::aws_sigv4::encode_key;
use yggdryl::internals::aws_sigv4::{
    EMPTY_PAYLOAD_SHA256, Signer, amz_date, canonical_query, canonical_request,
    encode_query_component, is_s3_family, sha256_hex, string_to_sign,
};

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

fn expected_headers(signed: &str, signature: &str, payload_hash: &str) -> Vec<(String, String)> {
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

#[cfg(feature = "s3")]
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

#[cfg(feature = "s3")]
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
        let (date, datetime): (String, String) = amz_date(at(seconds));
        assert_eq!(datetime, expected);
        assert_eq!(date, &expected[..8]);
    }
    assert_eq!(amz_date(UNIX_EPOCH - Duration::from_secs(5)).0, "19700101");
}

#[test]
fn the_signing_key_is_reused_within_a_day_and_rederived_across_days() {
    let signer = example_signer(None);
    let empty: [(String, String); 0] = [];
    let cached = |signer: &Signer| signer.cached_key();
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
    let (date, key): (String, [u8; 32]) = cached(&signer).unwrap();
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
    let (next_date, next_key): (String, [u8; 32]) = cached(&signer).unwrap();
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
        emitted[3]
            .1
            .contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token,")
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

// --- services outside the S3 family -------------------------------------------
//
// Vectors computed by botocore 1.43.93 (pure Python, `botocore.compat.HAS_CRT` false):
// `SigV4Auth` for the signing name `s3tables` in `us-east-1`, and `S3SigV4Auth` for the one
// S3 contrast, at 2015-08-30T12:36:00Z. This signer always sends and signs
// `x-amz-content-sha256`, which `SigV4Auth` signs when the request states it, so each
// request stated it before `add_auth`:
//
//     botocore.auth.get_current_datetime = lambda *a, **k: datetime.datetime(2015, 8, 30, 12, 36, 0)
//     signer = SigV4Auth(Credentials("AKIDEXAMPLE", SECRET, token), "s3tables", "us-east-1")
//     request = AWSRequest(method=method, url=url, data=data,
//                          headers={"X-Amz-Content-SHA256": sha256(data or b"").hexdigest(), ...})
//     signer.add_auth(request)
//     canonical = signer.canonical_request(request)
//     to_sign = signer.string_to_sign(request, canonical)
//     request.headers["Authorization"]

const BOTOCORE_ACCESS_KEY: &str = "AKIDEXAMPLE";
const BOTOCORE_SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
const BOTOCORE_TOKEN: &str = "AQoDYXdzEPT//////////wEXAMPLE";
const AUG_30_2015_123600: u64 = 1_440_938_160;
const CATALOG_HOST: &str = "s3tables.us-east-1.amazonaws.com";
/// A table bucket's ARN as a catalog's URL carries it: one path segment, encoded once.
const CATALOG_PREFIX: &str =
    "/iceberg/v1/arn%3Aaws%3As3tables%3Aus-east-1%3A123456789012%3Abucket%2Flake";
/// The same prefix as the canonical request carries it: encoded once more.
const CANONICAL_PREFIX: &str =
    "/iceberg/v1/arn%253Aaws%253As3tables%253Aus-east-1%253A123456789012%253Abucket%252Flake";

fn catalog_signer(token: Option<&str>) -> Signer {
    Signer::for_service(
        "s3tables",
        BOTOCORE_ACCESS_KEY,
        BOTOCORE_SECRET_KEY,
        token.map(str::to_owned),
        "us-east-1",
    )
}

/// One request botocore signed: what it is, and the hash of its canonical
/// request and the `authorization` botocore computed for it.
struct Vector<'a> {
    case: &'a str,
    signer: Signer,
    method: &'a str,
    host: &'a str,
    path: &'a str,
    query: &'a [(&'a str, &'a str)],
    headers: &'a [(&'a str, &'a str)],
    payload_hash: &'a str,
    request_hash: &'a str,
    /// The scope's service, the signed header names and the signature.
    authorization: (&'a str, &'a str, &'a str),
}

impl Vector<'_> {
    /// The hash of the canonical request the signer signs, and the
    /// `authorization` it emits.
    fn signed(&self) -> (String, String) {
        let (query, headers) = (pairs(self.query), pairs(self.headers));
        let canonical = self.signer.canonical_headers(
            self.host,
            "20150830T123600Z",
            self.payload_hash,
            &headers,
        );
        let request = canonical_request(
            self.method,
            &self.signer.canonical_uri(self.path),
            &query,
            &canonical,
            self.payload_hash,
        );
        let emitted = self.signer.sign(
            self.method,
            self.host,
            self.path,
            &query,
            &headers,
            self.payload_hash,
            at(AUG_30_2015_123600),
        );
        let authorization = emitted
            .into_iter()
            .find(|(name, _)| name == "authorization")
            .expect("an authorization header")
            .1;
        (sha256_hex(request.as_bytes()), authorization)
    }
}

#[test]
fn every_request_signs_as_botocore_signs_it() {
    let tables = format!("{CATALOG_PREFIX}/namespaces/a%1Fb/tables");
    assert_eq!(
        catalog_signer(None).canonical_uri(&tables),
        format!("{CANONICAL_PREFIX}/namespaces/a%251Fb/tables"),
        "an escape on the wire is escaped in the canonical request"
    );
    let namespaces = format!("{CATALOG_PREFIX}/namespaces");
    let body_hash = sha256_hex(br#"{"namespace":["a","b"],"properties":{}}"#);
    assert_eq!(
        body_hash,
        "9f5081f189782fbeec0701d3739a93a5412196c1c7397173834755d1f6cec17b"
    );
    let s3 = Signer::new(BOTOCORE_ACCESS_KEY, BOTOCORE_SECRET_KEY, None, "us-east-1");
    assert_eq!(
        s3.canonical_uri("/a%20b/c%2Fd.txt"),
        "/a%20b/c%2Fd.txt",
        "S3 signs its path as sent"
    );
    let unsigned = "host;x-amz-content-sha256;x-amz-date";
    for vector in [
        Vector {
            case: "a catalog GET, its path encoded once more",
            signer: catalog_signer(None),
            method: "GET",
            host: CATALOG_HOST,
            path: &tables,
            query: &[],
            headers: &[],
            payload_hash: EMPTY_PAYLOAD_SHA256,
            request_hash: "3358ffabb356e35767b50512f5d7585cb068cb3978434656cb3776f7e69fbc4f",
            authorization: (
                "s3tables",
                unsigned,
                "5700d6288c5d028115121dd9d24ac41de0d4e1b0823aa016b246413886647297",
            ),
        },
        Vector {
            case: "a catalog GET, its query sorted: `?pageToken=abc%2Fdef&pageSize=100`",
            signer: catalog_signer(None),
            method: "GET",
            host: CATALOG_HOST,
            path: &tables,
            query: &[("pageToken", "abc/def"), ("pageSize", "100")],
            headers: &[],
            payload_hash: EMPTY_PAYLOAD_SHA256,
            request_hash: "39d24edce1adfbfbc100c4968e60b9f2b3e390c7eb1ed36671e9113c6761e9d0",
            authorization: (
                "s3tables",
                unsigned,
                "75257237e7c4a403ed13c15fd282851dbaebf83a5bc3dd65120ba0a09b1e6f77",
            ),
        },
        Vector {
            case: "a catalog POST, its body and its content type",
            signer: catalog_signer(None),
            method: "POST",
            host: CATALOG_HOST,
            path: &namespaces,
            query: &[],
            headers: &[("Content-Type", "application/json")],
            payload_hash: &body_hash,
            request_hash: "59682b817fa6424ae2e249b36ae5aed922d01dba98a50eb63190f697793053df",
            authorization: (
                "s3tables",
                "content-type;host;x-amz-content-sha256;x-amz-date",
                "3c7b5e27e613a07ffec7024db5b53f12f67b1888fd3b328cccd140383e527a97",
            ),
        },
        Vector {
            case: "a catalog GET under a temporary set, its token",
            signer: catalog_signer(Some(BOTOCORE_TOKEN)),
            method: "GET",
            host: CATALOG_HOST,
            path: &tables,
            query: &[],
            headers: &[],
            payload_hash: EMPTY_PAYLOAD_SHA256,
            request_hash: "5503cd61a2497ec1035a3ae650cd2ced2e3907560c11c8f9775839ae9a1d1282",
            authorization: (
                "s3tables",
                "host;x-amz-content-sha256;x-amz-date;x-amz-security-token",
                "1825547c00e07fbcd20b6ef3ce314cffd0c734e624bb81c2363a95c6e5d4825e",
            ),
        },
        Vector {
            case: "a host naming its port: `http://127.0.0.1:4566/iceberg/v1/config?warehouse=...`",
            signer: catalog_signer(None),
            method: "GET",
            host: "127.0.0.1:4566",
            path: "/iceberg/v1/config",
            query: &[(
                "warehouse",
                "arn:aws:s3tables:us-east-1:123456789012:bucket/lake",
            )],
            headers: &[],
            payload_hash: EMPTY_PAYLOAD_SHA256,
            request_hash: "6b547decf99f873889b8d0e5a88f9dac1b30ae4bb4e24f78ef28083ba4a1e604",
            authorization: (
                "s3tables",
                unsigned,
                "ca3619a9a590402b1bea52af33c2404a3cde4ccea2a598a2615907f5d571826c",
            ),
        },
        Vector {
            case: "`S3SigV4Auth`, its path as sent: `GET https://examplebucket.s3.amazonaws.com/a%20b/c%2Fd.txt`",
            signer: s3,
            method: "GET",
            host: HOST,
            path: "/a%20b/c%2Fd.txt",
            query: &[],
            headers: &[],
            payload_hash: EMPTY_PAYLOAD_SHA256,
            request_hash: "0dd4646f198bd194881630eba5cb7d8333475d6053a0bee03bea4705721b6230",
            authorization: (
                "s3",
                unsigned,
                "193597080c2ec36d58ae5821fead6e1847327c2bccba64db0264b6d3c39d22ad",
            ),
        },
    ] {
        let (request_hash, authorization) = vector.signed();
        assert_eq!(request_hash, vector.request_hash, "{}", vector.case);
        let (service, signed_headers, signature) = vector.authorization;
        assert_eq!(
            authorization,
            format!(
                "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/{service}/aws4_request, \
                 SignedHeaders={signed_headers}, Signature={signature}"
            ),
            "{}",
            vector.case
        );
    }
}

#[test]
fn the_canonical_uri_of_every_other_service_is_botocores_normalized_path() {
    // `SigV4Auth._normalize_url_path(path)` of botocore 1.43.93 for each input, beside what
    // `S3SigV4Auth` answers: the path as sent.
    let generic = catalog_signer(None);
    let s3 = example_signer(None);
    for (path, canonical) in [
        ("", "/"),
        ("/", "/"),
        ("//", "/"),
        ("//a//b", "/a/b"),
        ("/a/./b", "/a/b"),
        ("/a/../b", "/b"),
        ("/../a", "/a"),
        ("/a/..", "/"),
        ("/a/b/..", "/a"),
        ("/.", "/"),
        ("/a/b/", "/a/b/"),
        ("/a//b/", "/a/b/"),
        ("/~user", "/~user"),
        ("/%7Euser", "/%257Euser"),
        ("/a+b", "/a%2Bb"),
        ("/a=b", "/a%3Db"),
        ("/a:b", "/a%3Ab"),
        ("/a%3Ab", "/a%253Ab"),
        ("/a%2Fb", "/a%252Fb"),
        ("/a%1Fb", "/a%251Fb"),
        ("/a b", "/a%20b"),
        ("/a%20b", "/a%2520b"),
        ("/\u{e9}", "/%C3%A9"),
        ("/a;b", "/a%3Bb"),
        ("/a@b", "/a%40b"),
        ("/a!b*c'(d)", "/a%21b%2Ac%27%28d%29"),
        ("/-_.", "/-_."),
    ] {
        assert_eq!(generic.canonical_uri(path), canonical, "{path:?}");
        assert_eq!(s3.canonical_uri(path), path, "S3 signs {path:?} as sent");
    }
}

#[test]
fn the_s3_family_is_four_signing_names_and_no_name_that_only_begins_with_s3() {
    for service in ["s3", "s3express", "s3-object-lambda", "s3-outposts"] {
        assert!(is_s3_family(service), "{service}");
    }
    for service in [
        "s3tables",
        "s3vectors",
        "s3files",
        "sts",
        "execute-api",
        "S3",
        "",
    ] {
        assert!(!is_s3_family(service), "{service:?}");
    }
}
