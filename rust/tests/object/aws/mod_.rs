//! `rust/src/object/aws/mod.rs`: what is Amazon's own - the region a bucket is
//! signed for, and the digest a bulk delete carries.

mod protocol {

    use yggdryl::IOBase;

    use crate::mod_::{BUCKET, file, folder, store};

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
}

#[cfg(all(feature = "object", feature = "internals"))]
#[path = "credentials.rs"]
mod credentials;
#[cfg(all(feature = "object", feature = "internals"))]
#[path = "profile.rs"]
mod profile;
#[cfg(feature = "object")]
#[path = "sts.rs"]
mod sts;
#[cfg(all(feature = "object", feature = "internals"))]
#[path = "xml.rs"]
mod xml;
