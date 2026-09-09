//! Exchange objects with a real S3 implementation, both directions.
//!
//! Self-consistency proves nothing about a wire protocol: the fake store the
//! unit tests run against was written from the same reading of the S3 API as
//! the client, so the two can agree and both be wrong. These tests run against
//! a real S3-compatible server - MinIO in CI, or any endpoint a developer
//! points them at - and cross-check with `boto3`, the reference client:
//!
//! 1. this half writes objects, prefixes, awkward keys, and a multipart
//!    upload, and reads back what `boto3` wrote;
//! 2. `scripts/check_s3_interop.py` drives it, and asserts from the Python
//!    side that every object this half wrote is exactly what it meant.
//!
//! Nothing runs without an endpoint. `YGGDRYL_S3_ENDPOINT` is what turns the
//! suite on; the reading half prints `SKIPPED` when the objects `boto3` was
//! meant to write are not there, and the driver fails on that word, so a
//! skipped half can never read as a pass.

use yggdryl::holder::object::{Credentials, ObjectOptions};
use yggdryl::{IOBase, IOKind};

/// The bucket both sides exchange through.
const BUCKET: &str = "yggdryl-interop";
/// The prefix this half writes under.
const FROM_RUST: &str = "from-rust";
/// The prefix `boto3` writes under.
const FROM_BOTO: &str = "from-boto3";
/// The KMS key the encryption cross-check names, which nothing has to exist.
const KMS_KEY_ID: &str = "arn:aws:kms:eu-west-1:123456789012:key/abcd-ef01";
/// The encryption context it binds the ciphertext to.
const KMS_CONTEXT: &str = r#"{"desk":"power","book":"eu-gas"}"#;

/// The endpoint the suite runs against, or `None` to skip.
fn endpoint() -> Option<String> {
    std::env::var("YGGDRYL_S3_ENDPOINT")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Options addressing that endpoint with the credentials the driver set.
fn options() -> ObjectOptions {
    let access = std::env::var("AWS_ACCESS_KEY_ID").unwrap_or_else(|_| "minioadmin".to_owned());
    let secret = std::env::var("AWS_SECRET_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".to_owned());
    ObjectOptions::default()
        .with_environment(false)
        .with_endpoint(endpoint().expect("an endpoint"))
        .with_region(std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_owned()))
        // MinIO and most gateways serve buckets from the path; a bucket
        // subdomain would need DNS the fixture has no way to provide.
        .with_path_style(true)
        .with_credentials(Credentials::new(access, secret))
}

/// The object `key` names in the exchange bucket.
///
/// Reached by raw name rather than by location, because that is what these
/// keys are: `a b/spaced.txt` is an ordinary key and not a URL, and the point
/// of the exercise is that both sides address the same object by it.
fn object(key: &str) -> yggdryl::holder::object::File {
    yggdryl::holder::object::file_at_with(BUCKET, key, options()).expect("an object handle")
}

/// The prefix `key` names in the exchange bucket.
fn prefix(key: &str) -> yggdryl::holder::object::Folder {
    yggdryl::holder::object::folder_at_with(BUCKET, key, options()).expect("a prefix handle")
}

/// The keys this half writes, with the bytes each holds.
///
/// The awkward ones are the point: a space, a plus, an equals, a percent, and
/// a non-ASCII name each mean something different to a URL, to a signature,
/// and to a key, and a client that confuses any two of them writes to the
/// wrong place. `boto3` reads these back by their raw names.
fn fixtures() -> Vec<(String, Vec<u8>)> {
    [
        "plain.txt",
        "nested/deep/part.parquet",
        "year=2026/month=01/part-0.parquet",
        "a b/spaced.txt",
        "a+b/plus.txt",
        "100%/percent.txt",
        "données/prix.txt",
        "tilde~and.dots..txt",
    ]
    .iter()
    .map(|key| {
        let key = format!("{FROM_RUST}/{key}");
        let bytes = format!("yggdryl wrote {key}").into_bytes();
        (key, bytes)
    })
    .collect()
}

#[test]
fn objects_written_here_are_readable_here_and_by_boto3() {
    let Some(_) = endpoint() else {
        println!("s3-interop: SKIPPED (set YGGDRYL_S3_ENDPOINT to run)");
        return;
    };

    // The bucket is the driver's to create, but creating an existing one is
    // absorbed, so this works whichever ran first.
    prefix("").create().expect("the exchange bucket");
    prefix(FROM_RUST).remove(true).expect("a clean prefix");

    for (key, bytes) in fixtures() {
        let mut handle = object(&key);
        handle.write_all_bytes(&bytes).expect("a written object");
        // Read it straight back: the key a write addressed is the key a read
        // addresses, escapes and all.
        assert_eq!(handle.read_all_bytes().expect("the object"), bytes, "{key}");
        assert_eq!(handle.size(), bytes.len() as u64, "{key}");
        assert_eq!(handle.kind(), IOKind::File, "{key}");

        // A ranged read against a real store, which is the path a record
        // reader takes: the range is the transfer and it bounds itself.
        let tail = handle
            .read_range_bytes(bytes.len() as u64 - 4, 16)
            .expect("a range");
        assert_eq!(tail, &bytes[bytes.len() - 4..], "{key}");
        assert!(
            handle
                .read_range_bytes(bytes.len() as u64 + 10, 8)
                .expect("a range past the end")
                .is_empty(),
            "{key}"
        );
    }

    // A listing of what was just written, against a real implementation's
    // ordering, continuation tokens, and URL encoding.
    let listed: Vec<String> = prefix(FROM_RUST)
        .ls(true, false)
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("a listing")
        .iter()
        .filter(|entry| !entry.is_container())
        .filter_map(|entry| entry.url().map(ToString::to_string))
        .collect();
    assert_eq!(listed.len(), fixtures().len(), "{listed:#?}");

    // A multipart upload, which is a different code path on every store.
    let large = vec![b'y'; 12 * 1024 * 1024];
    let mut big = yggdryl::holder::object::file_at_with(
        BUCKET,
        &format!("{FROM_RUST}/multipart.bin"),
        options().with_multipart_threshold(5 * 1024 * 1024),
    )
    .expect("an object handle");
    big.write_all_bytes(&large).expect("a multipart upload");
    assert_eq!(big.size(), large.len() as u64);
    assert_eq!(big.read_all_bytes().expect("the object"), large);
}

#[test]
fn objects_boto3_wrote_are_readable_here() {
    let Some(_) = endpoint() else {
        println!("s3-interop: SKIPPED (set YGGDRYL_S3_ENDPOINT to run)");
        return;
    };
    let written = prefix(FROM_BOTO);
    let entries: Vec<_> = written
        .ls(true, false)
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("a listing");
    let leaves: Vec<_> = entries
        .iter()
        .filter(|entry| !entry.is_container())
        .collect();
    if leaves.is_empty() {
        println!(
            "s3-interop: SKIPPED the external objects; nothing under {FROM_BOTO}/. Run \
             `python scripts/check_s3_interop.py`"
        );
        return;
    }

    for leaf in leaves {
        let url = leaf.url().expect("a located entry").to_string();
        let bytes = leaf.read_all_bytes().expect("the object");
        // The driver writes each object's own key as its content, so reading
        // one proves the key round-tripped through the listing, the URL, and
        // the signature without changing.
        let key = String::from_utf8(bytes).expect("the key it stores");
        assert_eq!(
            url,
            object(&key).url().to_string(),
            "the listed location does not name {key}"
        );
        // Addressing the same key by name reaches the same bytes.
        assert_eq!(
            object(&key).read_all_bytes().expect("the object"),
            key.as_bytes(),
        );
    }

    // A prefix `boto3` wrote is a container here, and one level of it lists
    // the way the store orders it.
    let level = written.ls(false, false).count();
    assert!(level > 0);
}

#[test]
fn a_removal_here_is_a_removal_there() {
    let Some(_) = endpoint() else {
        println!("s3-interop: SKIPPED (set YGGDRYL_S3_ENDPOINT to run)");
        return;
    };
    let mut scratch = object(&format!("{FROM_RUST}/scratch/removed.txt"));
    scratch.write_all_bytes(b"transient").expect("a write");
    assert!(scratch.exists());

    scratch.remove(false).expect("a removal");
    assert!(!scratch.exists());
    // Removing what is not there is a success against a real store too.
    scratch.remove(false).expect("a second removal");

    // And a recursive prefix removal takes the keys with it.
    let mut tree = prefix(&format!("{FROM_RUST}/scratch"));
    for part in 0..3 {
        object(&format!("{FROM_RUST}/scratch/part-{part}.bin"))
            .write_all_bytes(b"x")
            .expect("a write");
    }
    assert!(tree.exists());
    tree.remove(true).expect("a recursive removal");
    assert!(!tree.exists());
}

/// The exchange key for the encryption cross-check: 32 fixed bytes.
///
/// A fixed key is the point - both sides derive the same header values from
/// it, so a disagreement is about the encoding rather than about the key.
fn exchange_key() -> Vec<u8> {
    (0..32_u8)
        .map(|index| index.wrapping_mul(7).wrapping_add(3))
        .collect()
}

/// Put the headers this client sends beside the ones `botocore` computes.
///
/// This one needs no store, and it is the half `boto3` can check without one:
/// `SSE-C` is refused over plain HTTP by most implementations, so what is
/// exchanged here is the *headers* - their names and the base64 spellings of a
/// key, of its MD5, and of a KMS encryption context. `botocore` derives all of
/// them from the same inputs, and the driver compares them line by line.
#[test]
fn the_encryption_headers_are_what_botocore_computes() {
    let customer =
        yggdryl::holder::object::Encryption::customer(&exchange_key()).expect("a 32-byte key");
    for (name, value) in customer.write_headers() {
        println!("SSE-C {name} {value}");
    }

    let kms = yggdryl::holder::object::Encryption::Kms(
        yggdryl::holder::object::KmsKey::new(KMS_KEY_ID)
            .with_context(KMS_CONTEXT)
            .with_bucket_key(true),
    );
    for (name, value) in kms.write_headers() {
        println!("SSE-KMS {name} {value}");
    }
}
