//! What actually goes on the wire, and how the store's answers are read.
//!
//! The accounting suite proves how *many* requests an operation costs; this
//! one proves each is the request S3 documents - signed the way SigV4 says,
//! shaped the way the REST API says - and that the answers, including the
//! unhappy ones, come back as the typed results the crate's contracts name.

use super::{BUCKET, file, folder, location, options, path, payload, store};
use crate::holder::s3::{Credentials, File, S3Options};
use crate::{Error, IOBase, IOKind};

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
    assert_eq!(payload_hash, crate::holder::s3::sign::sha256_hex(b"PAR1"));
}

#[test]
fn a_refused_signature_is_the_stores_own_verdict() {
    let store = store();
    store.require_access_key(Some("SOMEONEELSE"));
    let handle = file(&store, "lake/part.parquet");

    let error = handle.read_all_bytes().expect_err("a refusal");
    match &error {
        Error::Remote {
            service,
            operation,
            status,
            code,
            path,
            ..
        } => {
            assert_eq!(*service, "s3");
            assert_eq!(*operation, "GetObject");
            assert_eq!(*status, 403);
            assert_eq!(code.as_str(), "InvalidAccessKeyId");
            assert_eq!(path.as_str(), "s3://trades/lake/part.parquet");
        }
        other => panic!("expected a remote refusal, got {other:?}"),
    }
    // A refusal is neither an absence nor a conflict, so a caller repairing
    // one of those is never sent down the wrong branch.
    assert!(!error.is_absent());
    assert!(!error.is_conflict());
}

#[test]
fn absence_is_emptiness_on_every_read_and_a_success_on_every_delete() {
    let store = store();
    let mut handle = file(&store, "lake/absent.parquet");

    assert!(handle.read_all_bytes().expect("an empty read").is_empty());
    assert_eq!(handle.size(), 0);
    assert!(handle.is_empty());
    assert_eq!(handle.kind(), IOKind::Unknown);
    let mut window = [0_u8; 8];
    assert_eq!(handle.pread(0, &mut window).expect("an empty read"), 0);
    assert_eq!(handle.pread(4096, &mut window).expect("an empty read"), 0);
    assert!(
        handle
            .pstream_bytes(0, 1024)
            .expect("a stream")
            .next()
            .is_none()
    );
    handle.remove(true).expect("a removal of nothing");

    // A digest of nothing is the algorithm's empty-input value, not an error.
    assert_eq!(
        handle
            .read_digest(crate::DigestAlgorithm::Xxh3)
            .expect("a digest")
            .as_u64(),
        Some(crate::xxhash::xxh3(b"")),
    );
}

#[test]
fn a_range_past_the_end_reads_nothing_and_a_straddling_one_is_short() {
    let store = store();
    store.put(BUCKET, "lake/part.parquet", &payload(100));
    let handle = file(&store, "lake/part.parquet");

    let mut window = [0_u8; 32];
    assert_eq!(handle.pread(90, &mut window).expect("a short read"), 10);
    assert_eq!(handle.pread(100, &mut window).expect("an empty read"), 0);
    assert_eq!(handle.pread(1000, &mut window).expect("an empty read"), 0);
    // A ranged read clamps to what is there, exactly as the trait says.
    assert_eq!(handle.read_range_bytes(95, 50).expect("a range").len(), 5);
}

#[test]
fn a_listing_asks_for_url_encoded_keys_and_a_delimiter_for_one_level() {
    let store = store();
    store.put(BUCKET, "lake/year=2026/part.parquet", b"PAR1");
    store.put(BUCKET, "lake/readme.txt", b"notes");
    let lake = folder(&store, "lake/");

    store.clear_requests();
    let entries: Vec<_> = lake
        .ls(false, false)
        .collect::<crate::Result<Vec<_>>>()
        .expect("a listing");
    let recorded = store.requests();
    let query: std::collections::HashMap<String, String> =
        recorded[0].query.iter().cloned().collect();
    assert_eq!(query.get("list-type").map(String::as_str), Some("2"));
    assert_eq!(query.get("prefix").map(String::as_str), Some("lake/"));
    assert_eq!(query.get("delimiter").map(String::as_str), Some("/"));
    assert_eq!(query.get("encoding-type").map(String::as_str), Some("url"));

    // One level: the sub-prefix rolls up into a container, the key is a leaf.
    assert_eq!(entries.len(), 2);
    let names: Vec<String> = entries
        .iter()
        .filter_map(|entry| entry.url().map(ToString::to_string))
        .collect();
    assert!(
        names.contains(&"s3://trades/lake/readme.txt".to_owned()),
        "{names:?}"
    );
    assert!(
        names.contains(&"s3://trades/lake/year=2026/".to_owned()),
        "{names:?}"
    );

    // A recursive listing drops the delimiter, which is what makes it flat.
    store.clear_requests();
    let _: Vec<_> = lake
        .ls(true, false)
        .collect::<crate::Result<Vec<_>>>()
        .expect("a listing");
    let recorded = store.requests();
    let query: std::collections::HashMap<String, String> =
        recorded[0].query.iter().cloned().collect();
    assert_eq!(query.get("delimiter"), None);
}

#[test]
fn a_listing_pages_through_a_prefix_wider_than_one_page() {
    let store = store();
    for part in 0..25 {
        store.put(BUCKET, &format!("lake/part-{part:03}.parquet"), b"PAR1");
    }
    let lake = super::Folder::new(
        std::sync::Arc::new(
            super::Client::new(&location("lake/"), options(&store).with_list_page_size(10))
                .expect("a client"),
        ),
        location("lake/"),
    )
    .expect("a prefix handle");

    store.clear_requests();
    let entries: Vec<_> = lake
        .ls(false, false)
        .collect::<crate::Result<Vec<_>>>()
        .expect("a listing");
    assert_eq!(entries.len(), 25);
    assert_eq!(store.request_count(), 3, "ten, ten, and five");

    // The pages are consumed lazily: three entries cost the first page only.
    store.clear_requests();
    let first: Vec<_> = lake.ls(false, false).take(3).collect();
    assert_eq!(first.len(), 3);
    assert_eq!(
        store.request_count(),
        1,
        "a caller taking three pays for one page"
    );
}

#[test]
fn keys_that_a_url_cannot_spell_survive_the_round_trip() {
    let store = store();
    // Spaces, a plus, an equals, a percent, and a unicode name: every one of
    // them means something different to a URL than to a key.
    let awkward = [
        "lake/a b/c.parquet",
        "lake/a+b/c.parquet",
        "lake/year=2026/part.parquet",
        "lake/100%/done.parquet",
        "lake/données/prix.parquet",
    ];
    for key in awkward {
        store.put(BUCKET, key, key.as_bytes());
    }

    let lake = folder(&store, "lake/");
    let found: Vec<String> = lake
        .ls(true, false)
        .collect::<crate::Result<Vec<_>>>()
        .expect("a listing")
        .iter()
        .filter(|entry| !entry.is_container())
        .map(|entry| {
            // Reading each listed entry addresses the same object it names.
            let bytes = entry.read_all_bytes().expect("the object");
            String::from_utf8(bytes).expect("the key it stores")
        })
        .collect();
    for key in awkward {
        assert!(
            found.contains(&key.to_owned()),
            "{key} missing from {found:?}"
        );
    }
}

#[test]
fn a_throttled_request_is_retried_and_a_refusal_is_not() {
    let store = store();
    store.put(BUCKET, "lake/part.parquet", b"PAR1");
    let handle = file(&store, "lake/part.parquet");

    // Two throttles, then the real answer: the default budget is three tries.
    store.fail_next(503, "SlowDown", 2);
    store.clear_requests();
    assert_eq!(handle.read_all_bytes().expect("the object"), b"PAR1");
    assert_eq!(store.request_count(), 3, "two retries and the answer");

    // A refusal is the store's verdict, not a hiccup, so it is not retried.
    store.fail_next(403, "AccessDenied", 1);
    store.clear_requests();
    handle.read_all_bytes().expect_err("a refusal");
    assert_eq!(
        store.request_count(),
        1,
        "a refusal stands on the first answer"
    );
}

#[test]
fn a_bucket_in_another_region_is_signed_again_for_the_region_it_is_in() {
    let store = store();
    store.set_bucket_region(BUCKET, Some("eu-west-3"));
    store.put(BUCKET, "lake/part.parquet", b"PAR1");
    let handle = file(&store, "lake/part.parquet");

    store.clear_requests();
    assert_eq!(handle.read_all_bytes().expect("the object"), b"PAR1");
    assert_eq!(
        store.request_count(),
        2,
        "the redirect names the region, and the retry is signed for it"
    );
    let recorded = store.requests();
    let authorization = recorded[1]
        .headers
        .iter()
        .find(|(name, _)| name == "authorization")
        .map(|(_, value)| value.clone())
        .expect("an authorization header");
    assert!(authorization.contains("/eu-west-3/s3/"), "{authorization}");

    // The region sticks, so the next read costs one request again.
    store.clear_requests();
    handle.read_all_bytes().expect("the object");
    assert_eq!(store.request_count(), 1, "the correction is learned once");
}

#[test]
fn an_anonymous_client_signs_nothing() {
    let store = store();
    store.allow_anonymous(true);
    store.put(BUCKET, "lake/part.parquet", b"PAR1");
    let handle = super::file_with(
        "lake/part.parquet",
        S3Options::default()
            .with_environment(false)
            .with_endpoint(store.endpoint())
            .with_path_style(true)
            .with_anonymous(true),
    );

    assert_eq!(handle.read_all_bytes().expect("a public object"), b"PAR1");
    let recorded = store.requests();
    assert!(
        !recorded
            .last()
            .expect("the read")
            .headers
            .iter()
            .any(|(name, _)| name == "authorization"),
        "an anonymous request carries no authorization"
    );
}

#[test]
fn credentials_written_into_a_location_are_used_and_then_never_rendered() {
    let store = store();
    store.require_access_key(Some("AKIAINURL"));
    let url = crate::Url::from_str(&format!("s3://AKIAINURL:s3cr3t@{BUCKET}/lake/part.parquet"))
        .expect("a location carrying credentials");
    let client = std::sync::Arc::new(
        super::Client::new(
            &url,
            S3Options::default()
                .with_environment(false)
                .with_endpoint(store.endpoint())
                .with_region("us-east-1")
                .with_path_style(true),
        )
        .expect("a client"),
    );
    let mut handle = File::new(client, url).expect("a handle");
    handle.write_all_bytes(b"PAR1").expect("a signed write");

    // The keys signed the request, and the handle's own location has none.
    let rendered = handle.url().to_string();
    assert_eq!(rendered, "s3://trades/lake/part.parquet");
    assert!(!rendered.contains("s3cr3t"));
    assert!(!format!("{handle:?}").contains("s3cr3t"));
}

#[test]
fn a_bulk_delete_carries_the_digest_s3_requires() {
    let store = store();
    for part in 0..3 {
        store.put(BUCKET, &format!("lake/part-{part}.parquet"), b"PAR1");
    }
    let mut lake = folder(&store, "lake/");

    store.clear_requests();
    lake.clear().expect("an emptied prefix");
    let recorded = store.requests();
    let delete = recorded
        .iter()
        .find(|request| request.method == "POST")
        .expect("the bulk delete");
    assert!(
        delete.query.iter().any(|(name, _)| name == "delete"),
        "the delete sub-resource names the operation"
    );
    assert!(
        delete.headers.iter().any(|(name, _)| name == "content-md5"),
        "S3 refuses a bulk delete without Content-MD5"
    );
}

#[test]
fn a_prefix_answers_emptiness_from_one_key_and_refuses_to_lose_children() {
    let store = store();
    store.put(BUCKET, "lake/part.parquet", b"PAR1");
    let mut lake = folder(&store, "lake/");

    store.clear_requests();
    assert!(lake.exists());
    assert_eq!(store.request_count(), 1, "one entry settles it");

    // A populated prefix is refused rather than silently recursed.
    let error = lake.remove(false).expect_err("a refusal");
    assert!(error.to_string().contains("still has children"), "{error}");
    assert!(store.get(BUCKET, "lake/part.parquet").is_some());

    // With `recursive` it goes, and the keys with it.
    lake.remove(true).expect("a recursive removal");
    assert!(store.keys(BUCKET).is_empty());
}

#[test]
fn the_media_type_comes_from_the_key_without_asking_the_store() {
    let store = store();
    let parquet = file(&store, "lake/part.parquet");
    let arrow = file(&store, "lake/part.arrows");
    let nameless = file(&store, "lake/part");

    assert_eq!(parquet.media_type().base(), &crate::MimeType::PARQUET);
    assert!(parquet.is_tabular());
    assert_eq!(arrow.media_type().base(), &crate::MimeType::ARROW_STREAM);
    assert_eq!(nameless.media_type().base(), &crate::MimeType::FILE);
    assert_eq!(store.request_count(), 0, "a name is free evidence");

    // A declared type overrides the name, still without a request.
    let mut declared = file(&store, "lake/part.parquet");
    declared.set_media_type(crate::MediaType::from(crate::MimeType::CSV));
    assert_eq!(declared.media_type().base(), &crate::MimeType::CSV);
    assert_eq!(store.request_count(), 0);
}

#[test]
fn a_glob_descends_its_fixed_prefix_rather_than_the_whole_bucket() {
    let store = store();
    for year in ["2024", "2025", "2026"] {
        for part in 0..2 {
            store.put(
                BUCKET,
                &format!("lake/year={year}/part-{part}.parquet"),
                b"PAR1",
            );
        }
    }
    store.put(BUCKET, "other/ignored.parquet", b"PAR1");
    let lake = folder(&store, "lake/");

    store.clear_requests();
    let matched: Vec<String> = lake
        .glob("year=2025/*.parquet", false)
        .expect("a pattern")
        .collect::<crate::Result<Vec<_>>>()
        .expect("the matches")
        .iter()
        .filter_map(|entry| entry.url().map(ToString::to_string))
        .collect();
    assert_eq!(matched.len(), 2, "{matched:?}");
    assert!(matched.iter().all(|url| url.contains("year=2025")));
    // The fixed prefix was descended, so nothing under `other/` was listed.
    let listed_prefixes: Vec<String> = store
        .requests()
        .iter()
        .filter_map(|request| {
            request
                .query
                .iter()
                .find(|(name, _)| name == "prefix")
                .map(|(_, value)| value.clone())
        })
        .collect();
    assert!(
        listed_prefixes
            .iter()
            .all(|prefix| prefix.starts_with("lake/")),
        "{listed_prefixes:?}"
    );
}

#[test]
fn a_location_naming_no_bucket_is_refused_before_anything_is_built() {
    let error = crate::holder::s3::file("file:///tmp/part.parquet").expect_err("a refusal");
    assert!(error.to_string().contains("naming a bucket"), "{error}");
    // The credential and endpoint knobs are equally unusable without one.
    crate::holder::s3::folder("https://example.com/x").expect_err("a refusal");
}

#[test]
fn every_s3_url_spelling_reaches_the_same_object() {
    // `s3`, `s3a`, and `s3n` name one protocol - the Hadoop spellings differ
    // only in the connector that once read them - so all three have to reach
    // this backend and address the same key, not just parse.
    let store = store();
    for scheme in ["s3", "s3a", "s3n"] {
        let url = format!("{scheme}://{BUCKET}/lake/{scheme}.bin");
        let mut handle =
            crate::holder::s3::file_with(&url, options(&store)).expect("an object handle");
        handle.write_all_bytes(b"AAPL").expect("a write");

        assert_eq!(handle.bucket(), BUCKET);
        assert_eq!(handle.key(), format!("lake/{scheme}.bin"));
        // The spelling the caller used is the one the handle reports back, so
        // a location survives a round trip through a listing or a log.
        assert_eq!(handle.url().to_string(), url);
        assert_eq!(
            store.get(BUCKET, &format!("lake/{scheme}.bin")),
            Some(b"AAPL".to_vec())
        );

        // A prefix and a location resolve over the same store just as well.
        let prefix = format!("{scheme}://{BUCKET}/lake/");
        let listed = crate::holder::s3::folder_with(&prefix, options(&store))
            .expect("a prefix handle")
            .ls(false, false)
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.url().map(ToString::to_string))
            .collect::<Vec<_>>();
        assert!(listed.contains(&url), "{listed:?}");
        assert_eq!(
            crate::holder::s3::located_with(&url, options(&store))
                .expect("a location")
                .kind(),
            IOKind::File
        );
    }

    // A refusal names the location the handle reports, spelling included, so
    // an error text is something a caller can paste back as a location.
    store.require_access_key(Some("SOMEONEELSE"));
    for scheme in ["s3", "s3a", "s3n"] {
        let url = format!("{scheme}://{BUCKET}/lake/{scheme}.bin");
        let refused = crate::holder::s3::file_with(&url, options(&store))
            .expect("an object handle")
            .read_all_bytes()
            .expect_err("a refusal");
        match &refused {
            Error::Remote { path, status, .. } => {
                assert_eq!(*status, 403);
                assert_eq!(path.as_str(), url);
            }
            other => panic!("expected a remote refusal, got {other:?}"),
        }
    }
}

#[test]
fn options_clamp_what_s3_will_not_accept_rather_than_refusing_it() {
    let bounded = S3Options::default()
        .with_part_size(1)
        .with_list_page_size(50_000)
        .with_max_attempts(0);
    assert_eq!(bounded.part_size(), 5 * 1024 * 1024, "S3's own part floor");
    assert_eq!(bounded.list_page_size(), 1000, "S3's own page ceiling");
    assert_eq!(bounded.max_attempts(), 1, "one attempt is still an attempt");

    // A bare host becomes an https endpoint, and a trailing slash is dropped.
    assert_eq!(
        S3Options::default()
            .with_endpoint("s3.example.io/")
            .endpoint(),
        Some("https://s3.example.io")
    );
    // Asking for anonymous access drops any credentials that were set.
    let anonymous = S3Options::default()
        .with_credentials(Credentials::new("a", "b"))
        .with_anonymous(true);
    assert!(anonymous.credentials().is_none());
    assert!(anonymous.anonymous());
}

#[test]
fn a_path_that_shares_a_textual_prefix_is_not_mistaken_for_a_container() {
    let store = store();
    store.put(BUCKET, "lake/partial.parquet", b"PAR1");
    // `lake/part` is neither an object nor a prefix, even though a key starts
    // with those very characters.
    let ambiguous = path(&store, "lake/part");
    assert_eq!(ambiguous.kind(), IOKind::Unknown);

    // Adding the object settles it as a leaf...
    store.put(BUCKET, "lake/part", b"AAPL");
    assert_eq!(path(&store, "lake/part").kind(), IOKind::File);
    // ...and a key beneath it makes the same name a container as well, where
    // the exact object wins because it sorts first.
    store.put(BUCKET, "lake/part/under.parquet", b"PAR1");
    assert_eq!(path(&store, "lake/part").kind(), IOKind::File);
}

#[test]
fn many_reads_share_one_connection_rather_than_reconnecting() {
    let store = store();
    store.put(BUCKET, "lake/part.bin", &payload(4096));
    let handle = file(&store, "lake/part.bin");

    // Ranged reads, whole reads, and streams in turn: every one of them has
    // to leave its connection reusable.
    for _ in 0..10 {
        assert_eq!(handle.read_range_bytes(0, 64).expect("a range").len(), 64);
        assert_eq!(handle.read_all_bytes().expect("the object").len(), 4096);
        assert_eq!(handle.pstream_bytes(0, 1024).expect("a stream").count(), 4);
    }

    assert_eq!(store.request_count(), 30);
    // The number that matters: on a real store each extra connection is a TCP
    // and TLS handshake, which would dwarf the transfer a footer read makes.
    assert_eq!(
        store.connection_count(),
        1,
        "thirty requests went down one connection"
    );
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
        Some(crate::holder::s3::sign::sha256_hex(b"AAPL,187.23").as_str()),
    );

    // Asking for the other policy sends the literal S3 accepts instead, which
    // is what an HTTPS endpoint selects on its own: hashing a large value
    // costs more than the rest of the request, and TLS already covers it.
    store.clear_requests();
    let mut unsigned = super::file_with(
        "lake/unsigned.bin",
        options(&store).with_payload_signing(false),
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
    let over_tls = S3Options::default().with_endpoint("https://s3.example.io");
    assert!(!over_tls.signs_payload("https"));
    assert!(S3Options::default().signs_payload("http"));
    assert!(over_tls.with_payload_signing(true).signs_payload("https"));
}

#[test]
fn a_refusal_is_never_read_as_an_empty_prefix_or_an_absent_object() {
    let store = store();
    store.put(BUCKET, "lake/part.bin", b"AAPL,187.23");

    // A listing nobody was allowed to see says nothing about what is there,
    // so a non-recursive removal must refuse rather than report success.
    let mut lake = folder(&store, "lake/");
    store.fail_next(403, "AccessDenied", 1);
    let refused = lake.remove(false).expect_err("a refusal");
    assert!(!refused.is_absent(), "{refused}");
    assert_eq!(
        store.keys(BUCKET),
        vec!["lake/part.bin".to_owned()],
        "and nothing was deleted"
    );

    // The same for a location: absence reads as emptiness, a refusal does not.
    let handle = path(&store, "lake/part.bin");
    store.fail_next(403, "AccessDenied", 1);
    let refused = handle.read_all_bytes().expect_err("a refusal");
    assert!(
        matches!(refused, Error::Remote { status: 403, .. }),
        "{refused}"
    );

    let missing = path(&store, "lake/absent.bin");
    assert_eq!(
        missing.read_all_bytes().expect("absence reads empty"),
        Vec::<u8>::new()
    );
}

#[test]
fn an_empty_value_is_one_put_however_low_the_multipart_threshold() {
    let store = store();
    let options = options(&store).with_multipart_threshold(0);
    let url = location("lake/empty.bin");
    let client = std::sync::Arc::new(
        crate::holder::s3::client::Client::new(&url, options).expect("a client"),
    );
    let mut handle = File::new(client, url).expect("an object handle");

    store.clear_requests();
    handle.write_all_bytes(b"").expect("a write");
    assert_eq!(
        store.request_count(),
        1,
        "a multipart upload of no parts is not something S3 completes"
    );
    assert_eq!(store.requests()[0].method, "PUT");
    assert_eq!(store.open_uploads(), 0, "and nothing was left open");
    assert_eq!(store.get(BUCKET, "lake/empty.bin"), Some(Vec::new()));
}

/// Whether a recorded request is the STS exchange rather than a bucket one.
fn is_exchange(request: &super::server::Recorded) -> bool {
    request
        .query
        .iter()
        .any(|(name, value)| name == "Action" && value == "AssumeRole")
}

#[test]
fn a_named_role_is_traded_for_a_session_once_and_signs_everything_after() {
    let store = store();
    store.put(BUCKET, "lake/part.bin", &payload(64));
    let role = crate::holder::s3::AssumedRole::new("arn:aws:iam::123456789012:role/lake-reader")
        .with_session_name("power-desk")
        .with_external_id("desk-42")
        // STS is a host of its own on AWS; the fixture shares this one.
        .with_endpoint(store.endpoint());
    let handle = super::file_with("lake/part.bin", options(&store).with_assumed_role(role));

    store.clear_requests();
    assert_eq!(handle.read_all_bytes().expect("a read"), payload(64));
    assert_eq!(handle.read_range_bytes(0, 8).expect("a read"), payload(8));

    let recorded = store.requests();
    let exchanges: Vec<_> = recorded
        .iter()
        .filter(|request| is_exchange(request))
        .collect();
    assert_eq!(
        exchanges.len(),
        1,
        "one exchange, however many requests follow it: {:?}",
        recorded.iter().map(|r| &r.path).collect::<Vec<_>>()
    );

    // The exchange says what was asked for, and is signed for STS rather than
    // for S3 - a different service in the scope is a different signing key.
    let exchange = exchanges[0];
    let asked = |name: &str| {
        exchange
            .query
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(asked("Version"), Some("2011-06-15"));
    assert_eq!(
        asked("RoleArn"),
        Some("arn:aws:iam::123456789012:role/lake-reader")
    );
    assert_eq!(asked("RoleSessionName"), Some("power-desk"));
    assert_eq!(asked("ExternalId"), Some("desk-42"));
    assert_eq!(asked("DurationSeconds"), Some("3600"));
    let authorization = |request: &super::server::Recorded| {
        request
            .headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };
    assert!(
        authorization(exchange).contains("/sts/aws4_request"),
        "{}",
        authorization(exchange)
    );

    // And every bucket request after it is signed as the role's session.
    let bucket_requests: Vec<_> = recorded
        .iter()
        .filter(|request| !is_exchange(request))
        .collect();
    assert_eq!(bucket_requests.len(), 2);
    for request in bucket_requests {
        let signed = authorization(request);
        assert!(signed.contains("Credential=ASIAlake-reader/"), "{signed}");
        assert!(signed.contains("/s3/aws4_request"), "{signed}");
    }
}

#[test]
fn a_lapsed_session_is_traded_again_rather_than_signed_with() {
    let store = store();
    store.put(BUCKET, "lake/part.bin", &payload(64));
    // Every session comes back already expired, which is the refresh path
    // without waiting an hour for it.
    store.expire_roles(true);
    let role = crate::holder::s3::AssumedRole::new("arn:aws:iam::123456789012:role/lake-reader")
        .with_endpoint(store.endpoint());
    let handle = super::file_with("lake/part.bin", options(&store).with_assumed_role(role));

    store.clear_requests();
    assert_eq!(handle.read_all_bytes().expect("a read"), payload(64));
    assert_eq!(handle.read_all_bytes().expect("a read"), payload(64));
    let exchanges = store
        .requests()
        .iter()
        .filter(|request| is_exchange(request))
        .count();
    assert_eq!(
        exchanges, 2,
        "a session past its expiry is not signed with, it is replaced"
    );
}

#[test]
fn a_bucket_lifecycle_this_client_may_not_perform_costs_no_request() {
    let store = store();
    let refusing = || {
        options(&store)
            .with_bucket_creation(false)
            .with_bucket_deletion(false)
    };
    let url = crate::Url::from_str("s3://ledger/").expect("a location");
    let client = std::sync::Arc::new(
        crate::holder::s3::client::Client::new(&url, refusing()).expect("a client"),
    );
    let mut ledger = crate::holder::s3::Folder::new(client, url).expect("a prefix handle");

    store.clear_requests();
    let refused = ledger.create().expect_err("a refusal");
    assert!(
        refused.to_string().contains("create the bucket"),
        "{refused}"
    );
    let refused = ledger.remove(false).expect_err("a refusal");
    assert!(
        refused.to_string().contains("delete the bucket"),
        "{refused}"
    );
    assert_eq!(
        store.request_count(),
        0,
        "the client's own rule is a refusal, not a round trip"
    );
    assert!(!store.buckets().contains(&"ledger".to_owned()));
}

#[test]
fn default_metadata_rides_every_write_without_displacing_a_content_type() {
    let store = store();
    let mut handle = super::file_with(
        "lake/part.parquet",
        options(&store).with_default_metadata([
            ("desk", "power"),
            ("Cache-Control", "max-age=31536000"),
            ("content-type", "application/x-nonsense"),
        ]),
    );

    store.clear_requests();
    handle.write_all_bytes(b"PAR1").expect("a write");
    let headers = &store.requests()[0].headers;
    let header = |name: &str| {
        headers
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(header("x-amz-meta-desk"), Some("power"));
    assert_eq!(header("cache-control"), Some("max-age=31536000"));
    assert_eq!(
        header("content-type"),
        Some("application/vnd.apache.parquet"),
        "a name the write sets for itself is not displaced by a default"
    );
}

#[test]
fn a_transfer_cut_part_way_through_resumes_from_where_it_stopped() {
    let store = store();
    let payload = payload(64 * 1024);
    store.put(BUCKET, "lake/part.bin", &payload);
    let handle = file(&store, "lake/part.bin");

    // The next body stops after 8 KiB with its declared length unchanged,
    // which is what a severed connection looks like from the inside.
    store.cut_next_body(8 * 1024, 1);
    store.clear_requests();
    let streamed: Vec<u8> = handle
        .pstream_bytes(0, 4096)
        .expect("a stream")
        .flat_map(|chunk| chunk.expect("bytes"))
        .collect();
    assert_eq!(
        streamed, payload,
        "the caller sees one uninterrupted stream"
    );

    let ranges: Vec<String> = store
        .requests()
        .iter()
        .filter_map(|request| {
            request
                .headers
                .iter()
                .find(|(name, _)| name == "range")
                .map(|(_, value)| value.clone())
        })
        .collect();
    assert_eq!(
        ranges,
        vec!["bytes=8192-".to_owned()],
        "the resumed request asks for the rest, not for the whole object again"
    );
    assert_eq!(store.request_count(), 2, "the open, and the one resume");
}

#[test]
fn a_transfer_that_cannot_deliver_a_byte_stops_rather_than_looping() {
    let store = store();
    store.put(BUCKET, "lake/part.bin", &payload(64 * 1024));
    let handle = super::file_with("lake/part.bin", options(&store).with_max_attempts(3));

    // Every body is cut before a single byte arrives, so nothing ever
    // progresses and the consecutive-failure budget is what ends it.
    store.cut_next_body(0, 100);
    store.clear_requests();
    let outcome: crate::Result<Vec<u8>> = handle
        .pstream_bytes(0, 4096)
        .expect("a stream")
        .collect::<crate::Result<Vec<_>>>()
        .map(|chunks| chunks.concat());
    assert!(outcome.is_err(), "a stream that never moves is a failure");
    assert!(
        store.request_count() <= 3,
        "bounded by the attempt limit, not by the object: {}",
        store.request_count()
    );
}
