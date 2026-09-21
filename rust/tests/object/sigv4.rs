//! `rust/src/object/sigv4.rs`: the request signing no caller can name.
//!
//! The signature is what every S3 request stands or falls on, and the only way
//! to know it is right is to reproduce AWS's own published example vectors -
//! the canonical request, the string to sign, and the headers that come out -
//! for the four requests their reference page documents. Everything a caller
//! can observe of a signed request is pinned in `rust/tests/object/client.rs`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yggdryl::internals::object_sigv4::{
    EMPTY_PAYLOAD_SHA256, Signer, amz_date, canonical_query, canonical_request, encode_key,
    encode_query_component, sha256_hex, string_to_sign,
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

mod protocol {
    use crate::mod_::{BUCKET, file, file_with, options, store};
    use yggdryl::IOBase;
    use yggdryl::internals::object_options::signs_payload;
    use yggdryl::internals::object_sigv4::sha256_hex;
    use yggdryl::object::{AwsOptions, ObjectOptions};

    #[test]
    fn every_request_carries_a_signature_over_the_headers_it_names() {
        let store = store();
        store.require_access_key(Some("AKIAIOSFODNN7EXAMPLE"));
        let mut handle = file(&store, "lake/part.parquet");
        handle.write_all_bytes(b"PAR1").expect("a signed write");

        let recorded = store.requests();
        let put = recorded.last().expect("the write");
        let authorization = put
            .headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .map(|(_, value)| value.clone())
            .expect("an authorization header");
        assert!(
            authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"),
            "{authorization}"
        );
        assert!(
            authorization.contains("/us-east-1/s3/aws4_request"),
            "{authorization}"
        );
        assert!(authorization.contains("SignedHeaders="), "{authorization}");
        // The payload is signed by its real hash, so the store can verify it.
        let payload_hash = put
            .headers
            .iter()
            .find(|(name, _)| name == "x-amz-content-sha256")
            .map(|(_, value)| value.clone())
            .expect("a payload hash");
        assert_eq!(payload_hash, sha256_hex(b"PAR1"));
    }

    #[test]
    fn a_write_signs_its_payload_over_http_and_leaves_it_unsigned_over_tls() {
        let store = store();
        // The fixture endpoint is plain HTTP, where nothing but the hash would
        // establish that the body arrived as it was sent.
        let mut handle = file(&store, "lake/part.bin");
        handle.write_all_bytes(b"AAPL,187.23").expect("a write");
        let recorded = store.requests();
        let put = recorded.last().expect("the write");
        assert_eq!(
            put.headers
                .iter()
                .find(|(name, _)| name == "x-amz-content-sha256")
                .map(|(_, value)| value.as_str()),
            Some(sha256_hex(b"AAPL,187.23").as_str()),
        );

        // Asking for the other policy sends the literal S3 accepts instead, which
        // is what an HTTPS endpoint selects on its own: hashing a large value
        // costs more than the rest of the request, and TLS already covers it.
        store.clear_requests();
        let mut unsigned = file_with(
            "lake/unsigned.bin",
            options(&store).with_aws(AwsOptions::default().with_payload_signing(false)),
        );
        unsigned.write_all_bytes(b"AAPL,187.23").expect("a write");
        let recorded = store.requests();
        let put = recorded.last().expect("the write");
        assert_eq!(
            put.headers
                .iter()
                .find(|(name, _)| name == "x-amz-content-sha256")
                .map(|(_, value)| value.as_str()),
            Some("UNSIGNED-PAYLOAD"),
        );
        // Either way the store received the bytes it was sent.
        assert_eq!(
            store.get(BUCKET, "lake/unsigned.bin").expect("the object"),
            b"AAPL,187.23"
        );

        // The policy an unset value picks follows the endpoint's scheme.
        let over_tls = ObjectOptions::default().with_endpoint("https://s3.example.io");
        assert!(!signs_payload(&over_tls, "https"));
        assert!(signs_payload(&ObjectOptions::default(), "http"));
        assert!(signs_payload(
            &over_tls.with_aws(AwsOptions::default().with_payload_signing(true)),
            "https"
        ));
    }
}
