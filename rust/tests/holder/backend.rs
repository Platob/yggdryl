//! `rust/src/holder/backend.rs`: the byte backends a crate claims, the
//! register `Holder::from_url` reads them from, and `Holder::Registered`,
//! the handle one answers held as it is.
//!
//! The refusals come first and claim nothing - a refused claim leaves the
//! register as it was - so they hold whatever else the process claimed.
//! Then the core's own claim, the object stores' ten schemes under the `s3`
//! feature. Then one test-only backend no crate claims, `ygshelf`: a
//! location `ygshelf://<rack>/<name>` held as a local file named after it,
//! every verb its own and the capability verbs stated, claimed once by
//! [`installed`] under two schemes no other test names. A claim is
//! process-wide and never withdrawn, and no other suite of the `holder`
//! target lists backends, so a listing below asserts what it finds of its
//! own claims and not the listing's length.

use std::any::Any;
use std::io::Read;
use std::sync::OnceLock;

use yggdryl::holder::{
    Holder, RegisteredHandle, StorageBackend, backend_for, backends, claim_backend,
};
use yggdryl::local::{LocalFile, LocalFolder};
use yggdryl::{Codec, Error, IOBase, IOFile, IOMedia, MimeType, Result, Scheme, Uri, Url};

/// The crate these tests claim as.
const BY: &str = "yggdryl-tests";

/// A scheme as the register keys it.
fn scheme(spelling: &str) -> Scheme {
    Scheme::from_str(spelling).expect("a URI scheme")
}

/// A backend named `name` over `schemes`, leaked: a claim keeps what it is
/// handed for the life of the process.
fn backend(name: &'static str, schemes: &[&str]) -> &'static Shelves {
    let schemes: Vec<Scheme> = schemes.iter().map(|spelling| scheme(spelling)).collect();
    Box::leak(Box::new(Shelves {
        name,
        schemes: Box::leak(schemes.into_boxed_slice()),
    }))
}

/// No property stated beside a location.
const NONE: [(&str, &str); 0] = [];

/// A backend over shelves: each location a local file named after it.
#[derive(Debug)]
struct Shelves {
    name: &'static str,
    schemes: &'static [Scheme],
}

impl StorageBackend for Shelves {
    fn name(&self) -> &'static str {
        self.name
    }

    fn schemes(&self) -> &'static [Scheme] {
        self.schemes
    }

    /// The two names a shelf reads for itself.
    fn is_property(&self, name: &str) -> bool {
        matches!(name, "rack_region" | "shelf_depth")
    }

    fn holder(&self, url: &Url, properties: &[(String, String)]) -> Result<Holder> {
        Shelf::new(url.clone(), properties.to_vec())
            .map(|shelf| Holder::Registered(Box::new(shelf)))
    }
}

/// One shelf location: the local file its name names, under the location it
/// was held by, and what the backend was handed for it.
#[derive(Debug)]
struct Shelf {
    url: Url,
    file: LocalFile,
    /// The properties the backend was handed, in the order handed.
    properties: Vec<(String, String)>,
    /// A length a caller stated, answered before the file's own.
    known: Option<u64>,
    /// The uploads taken through the shelf's own door.
    uploads: usize,
}

impl Shelf {
    fn new(url: Url, properties: Vec<(String, String)>) -> Result<Self> {
        let name = url.file_name().unwrap_or("rack").to_owned();
        let path = LocalFolder::temporary()?
            .path()?
            .join(format!("yggdryl-shelf-{}-{name}", std::process::id()));
        Ok(Self {
            url,
            file: LocalFile::new(path)?,
            properties,
            known: None,
            uploads: 0,
        })
    }
}

impl IOMedia for Shelf {
    yggdryl::impl_default_iomedia!();
}

impl IOBase for Shelf {
    yggdryl::delegate_iobase!(file: pread, read_all_bytes, write_all_bytes, create_bytes, pwrite,
        capacity, reserve, truncate, media_type, set_media_type, flush, clear, remove);

    fn uri(&self) -> Option<&Uri> {
        Some(self.url.as_ref())
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn size(&self) -> u64 {
        self.known.unwrap_or_else(|| self.file.size())
    }

    fn set_known_size(&mut self, size: u64) {
        self.known = Some(size);
    }

    /// The source read up to `length`, counted as the shelf's own upload.
    fn upload_from(&mut self, source: &mut dyn Read, length: u64) -> Result<()> {
        let mut bytes = Vec::new();
        source.take(length).read_to_end(&mut bytes)?;
        self.uploads += 1;
        self.file.write_all_bytes(&bytes)
    }

    /// A shelf publishes whole values alone, so a failed write left nothing.
    fn discard(&self) -> Result<bool> {
        Ok(true)
    }

    /// A shelf location names a file.
    fn as_leaf(&self) -> Result<Option<Holder>> {
        Ok(Some(Holder::from(LocalFile::new(self.file.path())?)))
    }

    /// The rack a shelf stands in is the folder its file is in.
    fn as_container(&self) -> Result<Option<Holder>> {
        Ok(Some(Holder::folder(LocalFolder::temporary()?.path()?)?))
    }
}

impl RegisteredHandle for Shelf {
    fn implementation_name(&self) -> &'static str {
        "Shelf"
    }

    fn exists(&self) -> bool {
        self.file.file_exists()
    }

    /// The shelf again, under what the backend was handed for it.
    fn reopen(&self) -> Result<Holder> {
        Shelf::new(self.url.clone(), self.properties.clone())
            .map(|shelf| Holder::Registered(Box::new(shelf)))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The shelf backend, claimed once for the whole process under `ygshelf`
/// and `ygshelves`.
fn installed() -> &'static Shelves {
    static INSTALLED: OnceLock<&'static Shelves> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let shelves = backend("yggdryl-tests-shelf", &["ygshelf", "ygshelves"]);
        claim_backend(shelves, BY).expect("two schemes no other test names");
        shelves
    })
}

/// A shelf location, removed before and after the test that names it.
struct Named(std::path::PathBuf);

impl Named {
    fn new(name: &str) -> Self {
        let path = LocalFolder::temporary()
            .unwrap()
            .path()
            .unwrap()
            .join(format!("yggdryl-shelf-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for Named {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// What a claim can be refused for, none of which claims anything.
mod refusals {
    use super::*;

    #[test]
    fn a_scheme_a_core_arm_holds_is_never_a_backends_to_claim() {
        let lowered = "an identifier spelling it is lowered before any backend is asked";
        for (spelling, arm) in [
            ("file", "the local and ZIP arm holds it"),
            ("http", "the HTTP arm holds it"),
            ("https", "the HTTP arm holds it"),
            ("mem", "an in-memory buffer's identity spells it"),
            ("urn", lowered),
            ("arn", lowered),
        ] {
            let refused = claim_backend(backend("yggdryl-tests-core-arm", &[spelling]), BY)
                .expect_err("a core arm's scheme");
            assert_eq!(
                refused.to_string(),
                format!(
                    "invalid record value at $.url: `{spelling}` is not a storage backend's to \
                     claim: {arm}"
                ),
            );
            assert!(backend_for(&scheme(spelling)).is_none(), "{spelling}");
        }

        // All or none: the fresh scheme named before the core's is left
        // unclaimed by the refusal.
        let refused = claim_backend(backend("yggdryl-tests-core-arm", &["ygarm", "file"]), BY)
            .expect_err("a core arm's scheme");
        assert!(matches!(refused, Error::InvalidRecord { .. }), "{refused}");
        assert!(backend_for(&scheme("ygarm")).is_none());
    }

    #[test]
    fn a_claim_in_the_cores_own_name_is_refused() {
        let refused = claim_backend(backend("yggdryl-tests-core", &["ygcore"]), "yggdryl")
            .expect_err("the core's own name");
        assert_eq!(
            refused.to_string(),
            "invalid record value at $.url: a storage backend is claimed by the crate that holds \
             it, never as `yggdryl`"
        );
        assert!(backend_for(&scheme("ygcore")).is_none());
    }

    #[test]
    fn a_backend_naming_no_scheme_or_one_scheme_twice_is_refused() {
        let refused =
            claim_backend(backend("yggdryl-tests-empty", &[]), BY).expect_err("no scheme");
        assert_eq!(
            refused.to_string(),
            "invalid record value at $.url: a storage backend names at least one scheme, \
             `yggdryl-tests-empty` names none"
        );

        let refused = claim_backend(backend("yggdryl-tests-twice", &["ygtwice", "ygtwice"]), BY)
            .expect_err("one scheme twice");
        assert_eq!(
            refused.to_string(),
            "invalid record value at $.url: `yggdryl-tests-twice` names the scheme `ygtwice` twice"
        );
        assert!(backend_for(&scheme("ygtwice")).is_none());
    }

    #[test]
    fn a_scheme_no_claim_answers_is_refused_naming_the_crate_to_install() {
        let url = Url::from_str("ftp://example.com/lake/trades.csv").unwrap();
        let refused = Holder::from_url(&url, NONE).expect_err("no backend holds `ftp`");
        assert!(refused.is_unsupported(), "{refused}");
        assert_eq!(
            refused.to_string(),
            "filesystem \"ftp\" does not support holding a location of this scheme; install the \
             crate that claims it and call its `install()`"
        );
        assert!(backend_for(&scheme("ftp")).is_none());
        assert!(backend_for(&Scheme::FILE).is_none(), "the local arm's own");
    }

    /// Without the feature that claims them, an object store's location is
    /// a scheme no claim answers, refused as every other is.
    #[cfg(not(feature = "s3"))]
    #[test]
    fn an_object_store_location_without_its_claim_names_the_crate_to_install() {
        assert!(backend_for(&Scheme::S3).is_none());
        let url = Url::from_str("s3://trades/lake/part.parquet").unwrap();
        let refused = Holder::from_url(&url, NONE).expect_err("no backend holds `s3`");
        assert_eq!(
            refused.to_string(),
            "filesystem \"s3\" does not support holding a location of this scheme; install the \
             crate that claims it and call its `install()`"
        );
    }
}

/// The object stores' backend, which the core claims itself under the `s3`
/// feature until `yggdryl-s3` does.
#[cfg(feature = "s3")]
mod core_claim {
    use super::*;

    const OBJECT_STORES: [&str; 10] = [
        "s3", "s3a", "s3n", "gs", "gcs", "az", "abfs", "abfss", "wasb", "wasbs",
    ];

    #[test]
    fn the_object_stores_ten_schemes_are_the_cores_own_claim() {
        let s3 = backend_for(&Scheme::S3).expect("the core claims `s3`");
        assert_eq!(s3.name(), "yggdryl-s3");
        assert_eq!(s3.schemes().len(), 10);
        for spelling in OBJECT_STORES {
            let claimed = backend_for(&scheme(spelling)).expect("an object-store scheme");
            assert!(std::ptr::addr_eq(claimed, s3), "{spelling}");
        }
        // The store's own names, and nothing an object takes as a query.
        assert!(s3.is_property("region"));
        assert!(s3.is_property("endpoint_override"));
        assert!(!s3.is_property("versionId"));

        // Once in the listing, however many schemes it holds.
        let listed = backends()
            .into_iter()
            .filter(|held| std::ptr::addr_eq(*held, s3))
            .count();
        assert_eq!(listed, 1);

        // A table bucket's location is a locator's object, not a backend's
        // bytes.
        assert!(backend_for(&Scheme::S3TABLES).is_none());
    }

    #[test]
    fn a_second_claim_of_an_object_store_scheme_names_the_core() {
        let refused = claim_backend(backend("yggdryl-tests-rival", &["s3"]), BY)
            .expect_err("`s3` is claimed");
        assert!(matches!(refused, Error::Conflict { .. }), "{refused}");
        assert_eq!(
            refused.to_string(),
            "expected to create a storage backend at \"s3\", got an existing yggdryl"
        );

        // All or none: a fresh scheme beside a claimed one stays unclaimed,
        // and the claimed one stays the core's.
        let refused = claim_backend(backend("yggdryl-tests-rival", &["ygrival", "gs"]), BY)
            .expect_err("`gs` is claimed");
        assert_eq!(
            refused.to_string(),
            "expected to create a storage backend at \"gs\", got an existing yggdryl"
        );
        assert!(backend_for(&scheme("ygrival")).is_none());
        assert_eq!(
            backend_for(&Scheme::GS).map(|claimed| claimed.name()),
            Some("yggdryl-s3")
        );
    }
}

/// A backend a crate claims, and every door it is reached through.
mod claimed {
    use super::*;

    #[test]
    fn a_claimed_backend_answers_each_of_its_schemes_and_is_listed_once() {
        let shelves = installed();
        for spelling in ["ygshelf", "ygshelves"] {
            let claimed = backend_for(&scheme(spelling)).expect("a claimed scheme");
            assert!(std::ptr::addr_eq(claimed, shelves), "{spelling}");
            assert_eq!(claimed.name(), "yggdryl-tests-shelf");
        }
        let listed = backends()
            .into_iter()
            .filter(|held| std::ptr::addr_eq(*held, shelves))
            .count();
        assert_eq!(listed, 1);

        // A second claim names the first claimant; a claim reaching a
        // claimed scheme after a fresh one claims neither.
        let refused = claim_backend(shelves, "yggdryl-tests-again").expect_err("claimed once");
        assert_eq!(
            refused.to_string(),
            "expected to create a storage backend at \"ygshelf\", got an existing yggdryl-tests"
        );
        let refused = claim_backend(
            backend("yggdryl-tests-clash", &["ygclash", "ygshelves"]),
            BY,
        )
        .expect_err("`ygshelves` is claimed");
        assert!(matches!(refused, Error::Conflict { .. }), "{refused}");
        assert!(backend_for(&scheme("ygclash")).is_none());
    }

    #[test]
    fn a_location_is_held_by_the_backend_its_scheme_names_under_its_query_and_the_callers() {
        installed();
        let named = Named::new("trades-held.csv");
        let url = Url::from_str("ygshelf://rack/trades-held.csv?shelf_depth=3").unwrap();
        let mut held =
            Holder::from_url(&url, [("rack_region", "eu-west-1"), ("shelf_depth", "5")]).unwrap();
        assert!(matches!(held, Holder::Registered(_)), "{held:?}");

        // The query is handed first, so the caller's statement of a name
        // comes after it; the location the handle reports names the
        // resource, not how it is reached.
        let shelf = held.downcast_ref::<Shelf>().expect("the shelf");
        assert_eq!(
            shelf.properties,
            [
                ("shelf_depth".to_owned(), "3".to_owned()),
                ("rack_region".to_owned(), "eu-west-1".to_owned()),
                ("shelf_depth".to_owned(), "5".to_owned()),
            ]
        );
        assert_eq!(
            held.url().map(ToString::to_string).as_deref(),
            Some("ygshelf://rack/trades-held.csv")
        );
        assert_eq!(held.media_type().base(), &MimeType::CSV);

        // The handle as the type it is, mutably too; any other variant is no
        // registered handle.
        held.downcast_mut::<Shelf>()
            .expect("the shelf")
            .properties
            .clear();
        assert!(held.downcast_ref::<Shelf>().unwrap().properties.is_empty());
        assert!(held.downcast_ref::<LocalFile>().is_none());
        assert!(
            Holder::from(LocalFile::new(named.0.clone()).unwrap())
                .downcast_ref::<LocalFile>()
                .is_none()
        );

        // A parameter naming no property the backend reads is refused by
        // name before the backend is asked, since a resource takes no query.
        let url = Url::from_str("ygshelves://rack/trades-held.csv?version=3").unwrap();
        let refused = Holder::from_url(&url, NONE).expect_err("an unread parameter");
        assert_eq!(
            refused.to_string(),
            "invalid storage location expression at byte 0: the query parameter \"version\" \
             names no property the `ygshelves` backend reads"
        );
    }

    #[test]
    fn a_held_location_is_described_and_held_again_through_its_own_reopen() {
        installed();
        let _named = Named::new("trades-described.bin");
        let url = Url::from_str("ygshelf://rack/trades-described.bin").unwrap();
        let mut held =
            Holder::from_url(&url, [("media_type", "text/csv"), ("codec", "gzip")]).unwrap();

        // `media_type` and `codec` are read over a backend's handle as over
        // every other: the coding presents the decoded value.
        assert!(matches!(held, Holder::Coded(_)), "{held:?}");
        held.write_all_bytes(b"AAPL,187.23\n").unwrap();
        assert_eq!(held.read_all_bytes().unwrap(), b"AAPL,187.23\n");

        // A second handle is the plain one beneath the coding, as its own
        // `reopen` answers, under the media type it declared.
        let again = Holder::from_handle(&held).unwrap();
        assert!(again.downcast_ref::<Shelf>().is_some(), "{again:?}");
        assert_eq!(again.media_type().base(), &MimeType::CSV);
        assert_eq!(
            Codec::Gzip.load(&again.read_all_bytes().unwrap()).unwrap(),
            b"AAPL,187.23\n"
        );

        // A location spelling a container takes no coding.
        let url = Url::from_str("ygshelf://rack/").unwrap();
        let held = Holder::from_url(&url, [("codec", "gzip")]).unwrap();
        assert!(matches!(held, Holder::Registered(_)), "{held:?}");
    }

    #[test]
    fn a_registered_handle_answers_every_verb_for_itself_through_the_holder() {
        installed();
        let _named = Named::new("trades-verbs.bin");
        let url = Url::from_str("ygshelves://rack/trades-verbs.bin").unwrap();
        let mut held = Holder::from_url(&url, NONE).unwrap();
        assert!(!held.exists(), "nothing written yet");

        // The upload is the shelf's own door, not the whole-value default.
        let mut source: &[u8] = b"AAPL,187.23\nMSFT,411.10\n";
        held.upload_from(&mut source, 12).unwrap();
        assert!(held.exists());
        assert_eq!(held.read_all_bytes().unwrap(), b"AAPL,187.23\n");
        assert_eq!(held.downcast_ref::<Shelf>().unwrap().uploads, 1);

        // A stated length is the shelf's to keep.
        held.set_known_size(4096);
        assert_eq!(held.size(), 4096);

        // Its stage, its leaf and its container are its own answers, where
        // the defaults would say there is nothing staged and no role to
        // re-describe.
        assert!(held.discard().unwrap());
        let leaf = held.as_leaf().unwrap().expect("the shelf's file");
        assert!(matches!(leaf, Holder::LocalFile(_)), "{leaf:?}");
        assert_eq!(leaf.read_all_bytes().unwrap(), b"AAPL,187.23\n");
        let rack = held.as_container().unwrap().expect("the shelf's rack");
        assert!(matches!(rack, Holder::LocalFolder(_)), "{rack:?}");

        // Held again, the shelf is there for the second handle too.
        let again = Holder::from_handle(&held).unwrap();
        assert!(again.exists());
        assert_eq!(
            again.url().map(ToString::to_string).as_deref(),
            Some("ygshelves://rack/trades-verbs.bin")
        );
    }
}
