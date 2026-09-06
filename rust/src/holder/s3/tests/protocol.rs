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
