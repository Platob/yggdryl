//! Encryption at rest, and what each kind puts on the wire.
//!
//! The three kinds differ in who holds the key, which is the same thing as
//! differing in what a request has to say - and the last of them, `SSE-C`, is
//! the one worth testing hardest: a key the store never keeps has to be
//! presented again on *every* read, and a handle that forgets it on one path
//! reads as a refusal rather than as bytes.

use super::{BUCKET, file, file_with, options, payload, store};
use crate::holder::s3::{CustomerKey, Encryption, KmsKey, S3Options};
use crate::{Error, IOBase};

/// The 32 bytes `AES256` takes, and nothing anyone would use twice.
fn customer_key() -> Vec<u8> {
    (0..32_u8)
        .map(|index| index.wrapping_mul(7).wrapping_add(3))
        .collect()
}

/// The header a request carried, if it carried it.
fn header(store: &super::server::FakeS3, index: usize, name: &str) -> Option<String> {
    store.requests()[index]
        .headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.clone())
}

#[test]
fn keys_the_store_manages_are_one_header_on_the_write_and_none_on_the_read() {
    let store = store();
    let encrypted = || options(&store).with_encryption(Encryption::managed());

    let mut handle = file_with("lake/part.parquet", encrypted());
    store.clear_requests();
    handle.write_all_bytes(&payload(1024)).expect("a write");
    assert_eq!(
        header(&store, 0, "x-amz-server-side-encryption").as_deref(),
        Some("AES256")
    );
    assert_eq!(
        store
            .stored_encryption(BUCKET, "lake/part.parquet")
            .as_deref(),
        Some("AES256")
    );

    // The store knows which of its own keys it used, so a read says nothing -
    // and a handle that never heard of the encryption reads it all the same.
    let plain = file(&store, "lake/part.parquet");
    assert_eq!(plain.read_all_bytes().expect("a read"), payload(1024));
}

#[test]
fn a_kms_key_carries_its_id_its_context_and_its_bucket_key() {
    let store = store();
    let key = KmsKey::new("arn:aws:kms:eu-west-1:1234:key/abcd")
        .with_context(r#"{"desk":"power"}"#)
        .with_bucket_key(true);
    let mut handle = file_with(
        "lake/part.parquet",
        options(&store).with_encryption(Encryption::Kms(key)),
    );

    store.clear_requests();
    handle.write_all_bytes(&payload(1024)).expect("a write");
    assert_eq!(
        header(&store, 0, "x-amz-server-side-encryption").as_deref(),
        Some("aws:kms")
    );
    assert_eq!(
        header(&store, 0, "x-amz-server-side-encryption-aws-kms-key-id").as_deref(),
        Some("arn:aws:kms:eu-west-1:1234:key/abcd")
    );
    // The context goes over as base64 of the JSON, not as the JSON.
    assert_eq!(
        header(&store, 0, "x-amz-server-side-encryption-context").as_deref(),
        Some("eyJkZXNrIjoicG93ZXIifQ==")
    );
    assert_eq!(
        header(&store, 0, "x-amz-server-side-encryption-bucket-key-enabled").as_deref(),
        Some("true")
    );
    assert_eq!(
        store
            .stored_encryption(BUCKET, "lake/part.parquet")
            .as_deref(),
        Some("aws:kms arn:aws:kms:eu-west-1:1234:key/abcd")
    );

    // Dual-layer is the same request under another name.
    let dual = file_with(
        "lake/dual.parquet",
        options(&store).with_encryption(Encryption::Kms(KmsKey::default().with_dual_layer(true))),
    );
    let mut dual = dual;
    store.clear_requests();
    dual.write_all_bytes(b"PAR1").expect("a write");
    assert_eq!(
        header(&store, 0, "x-amz-server-side-encryption").as_deref(),
        Some("aws:kms:dsse")
    );
    assert_eq!(
        header(&store, 0, "x-amz-server-side-encryption-aws-kms-key-id"),
        None,
        "an unnamed key is the bucket's own default"
    );
}

#[test]
fn a_customer_key_is_presented_again_on_every_read() {
    let store = store();
    let key = customer_key();
    let encrypted =
        || options(&store).with_encryption(Encryption::customer(&key).expect("a 32-byte key"));
    let expected = CustomerKey::new(&key).expect("a 32-byte key");

    let mut handle = file_with("lake/part.bin", encrypted());
    handle.write_all_bytes(&payload(4096)).expect("a write");
    assert_eq!(
        store.stored_encryption(BUCKET, "lake/part.bin"),
        Some(format!("customer {}", expected.checksum())),
        "the store keeps the checksum and forgets the key"
    );

    // Every shape of read carries it: whole, ranged, streamed, and the HEAD
    // behind a size.
    let reader = file_with("lake/part.bin", encrypted());
    assert_eq!(
        reader.read_all_bytes().expect("a whole read"),
        payload(4096)
    );
    assert_eq!(
        reader.read_range_bytes(1000, 8).expect("a ranged read"),
        payload(4096)[1000..1008]
    );
    assert_eq!(reader.size(), 4096);
    let streamed: Vec<u8> = reader
        .pstream_bytes(0, 512)
        .expect("a stream")
        .flat_map(|chunk| chunk.expect("bytes"))
        .collect();
    assert_eq!(streamed, payload(4096));
    let digest = reader
        .read_range_digest(0, 16, crate::DigestAlgorithm::Xxh3)
        .expect("a digest");
    assert_eq!(digest, crate::DigestAlgorithm::Xxh3.digest(&payload(16)));

    // Every request that touched the bytes presented the key, and every one
    // that did not touch them did not.
    for recorded in store.requests() {
        let presented = recorded
            .headers
            .iter()
            .any(|(name, _)| name == "x-amz-server-side-encryption-customer-key");
        let touches_bytes = matches!(recorded.method.as_str(), "GET" | "HEAD" | "PUT")
            && !recorded.path.contains("list-type");
        assert_eq!(
            presented, touches_bytes,
            "{} {} carried the key: {presented}",
            recorded.method, recorded.path
        );
    }

    // And a handle without the key is refused rather than answered.
    let blind = file(&store, "lake/part.bin");
    let refused = blind.read_all_bytes().expect_err("a refusal");
    assert!(
        matches!(refused, Error::Remote { status: 400, .. }),
        "{refused}"
    );
}

#[test]
fn a_customer_key_reaches_every_part_of_a_multipart_upload() {
    let store = store();
    let key = customer_key();
    let encrypted = || {
        options(&store)
            .with_encryption(Encryption::customer(&key).expect("a 32-byte key"))
            .with_multipart_threshold(512 * 1024)
            .with_part_size(5 * 1024 * 1024)
    };

    let bytes = payload(600 * 1024);
    let mut handle = file_with("lake/big.bin", encrypted());
    store.clear_requests();
    handle.write_all_bytes(&bytes).expect("a large write");

    let carried: Vec<(String, bool)> = store
        .requests()
        .iter()
        .map(|recorded| {
            (
                recorded.method.clone(),
                recorded
                    .headers
                    .iter()
                    .any(|(name, _)| name == "x-amz-server-side-encryption-customer-key"),
            )
        })
        .collect();
    assert_eq!(
        carried,
        vec![
            // Initiation says how the object is stored, a part presents the
            // key it is encrypted under, and completion touches no bytes.
            ("POST".to_owned(), true),
            ("PUT".to_owned(), true),
            ("POST".to_owned(), false),
        ]
    );
    assert_eq!(
        store.stored_encryption(BUCKET, "lake/big.bin"),
        Some(format!(
            "customer {}",
            CustomerKey::new(&key).expect("a key").checksum()
        ))
    );
    assert_eq!(
        file_with("lake/big.bin", encrypted())
            .read_all_bytes()
            .expect("a read"),
        bytes
    );
}

#[test]
fn an_unusable_customer_key_is_refused_before_anything_is_sent() {
    let short = Encryption::customer(b"too short").expect_err("a refusal");
    assert!(short.to_string().contains("32 bytes"), "{short}");

    let unreadable = CustomerKey::from_base64("not base64 at all!!").expect_err("a refusal");
    assert!(unreadable.to_string().contains("base64"), "{unreadable}");

    // The MD5 the AWS tools carry alongside a key is checked against it, so a
    // pair copied out of step is heard here rather than as a 400 from AWS.
    let key = CustomerKey::new(&customer_key()).expect("a key");
    let encoded = key.encoded().to_owned();
    let mismatched = CustomerKey::from_base64_checked(&encoded, "AAAAAAAAAAAAAAAAAAAAAA==")
        .expect_err("a refusal");
    assert!(mismatched.to_string().contains("MD5"), "{mismatched}");
    assert!(CustomerKey::from_base64_checked(&encoded, key.checksum()).is_ok());
}

#[test]
fn a_customer_key_is_never_rendered() {
    let key = CustomerKey::new(&customer_key()).expect("a key");
    let encoded = key.encoded().to_owned();
    let options = S3Options::default().with_encryption(Encryption::Customer(key));

    let rendered = format!("{options:?}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
    assert!(!rendered.contains(&encoded), "the key itself: {rendered}");
}
