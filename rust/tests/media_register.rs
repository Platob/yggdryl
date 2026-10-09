//! The media registers of `rust/src/media/codec.rs`, `rust/src/media/format.rs`,
//! `rust/src/holder/locator.rs` and `rust/src/warehouse/catalog.rs`, over
//! five test-only implementations no crate claims: a record medium
//! `testmedium` under `application/x-yggdryl-test` that stores Arrow IPC, a
//! second medium `latemedium` claimed in the middle of one test, a table
//! format, a locator for `ygtest:` and a catalog factory. A claim is
//! process-wide and never withdrawn, so this target owns its process: every
//! medium test opens with [`installed`], and no other harness claims a medium, so
//! every other target's listings are the core's own.
//!
//! What is pinned: the claim and what it registers, every refusal a claim
//! can meet, every intake door reaching the medium through the register -
//! the options a MIME type names, the typed settings door, the hash, the
//! order, the wrapper `Media::open_as` builds and the rows a handle of the
//! type round-trips - and the install rule: a type read before its claim is
//! refused naming the crate to install, and answers after it.

use std::sync::{Arc, LazyLock, OnceLock};

use arrow_array::{Int64Array, RecordBatch};
use smol_str::SmolStr;
use yggdryl::arrow::BatchReader;
use yggdryl::avro::AvroOptions;
use yggdryl::holder::{Buffer, Holder, Locator, claim_locator, locators};
use yggdryl::ipc::{self, IpcOptions};
use yggdryl::media::{
    EXTERNAL_RANK, IORecordOptions, LocatedTable, Media, MediaCodec, MediaWrapper, MediumSettings,
    RecordOptions, TableFormat, codec, codec_for, codecs, format, format_named, formats,
};
use yggdryl::warehouse::{CatalogFactory, claim_factory, factories};
use yggdryl::{
    Arn, Catalog, DataType, Error, Field, Filter, IOBase, IOMedia, Level, MediaTable, MediaType,
    MemoryCatalog, MimeType, ObjectValue, Properties, Result, Scheme, Selector, StructType, Table,
    Uri, Url,
};

/// The crate these tests claim as.
const BY: &str = "yggdryl-tests";

/// A custom MIME type.
fn mime(text: &str) -> MimeType {
    MimeType::from_str(text).expect("a custom MIME type")
}

static TEST_TYPES: LazyLock<[MimeType; 1]> = LazyLock::new(|| [mime("application/x-yggdryl-test")]);
static LATE_TYPES: LazyLock<[MimeType; 1]> = LazyLock::new(|| [mime("application/x-yggdryl-late")]);
/// A fresh type first and the test medium's second: a claim stopped by the
/// second leaves the first unclaimed.
static CLASH_TYPES: LazyLock<[MimeType; 2]> = LazyLock::new(|| {
    [
        mime("application/x-yggdryl-clash"),
        mime("application/x-yggdryl-test"),
    ]
});

fn test_types() -> &'static [MimeType] {
    &*TEST_TYPES
}

fn late_types() -> &'static [MimeType] {
    &*LATE_TYPES
}

fn clash_types() -> &'static [MimeType] {
    &*CLASH_TYPES
}

fn no_types() -> &'static [MimeType] {
    &[]
}

fn test_mime() -> MimeType {
    test_types()[0].clone()
}

fn late_mime() -> MimeType {
    late_types()[0].clone()
}

/// The options struct of a test medium: the fourteen shared sections and
/// nothing of its own, held whole behind the register like every medium's.
macro_rules! test_options {
    ($options:ident, $codec:expr) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        struct $options {
            name: SmolStr,
            field: Option<Field>,
            filter: Filter,
            select: Selector,
            merge_by: Selector,
            safe: bool,
            batch_byte_size: Option<u64>,
            batch_row_size: Option<usize>,
            max_row_size: Option<u64>,
            row_offset: Option<u64>,
            max_byte_size: Option<u64>,
            commit_batch_num: Option<usize>,
            num_threads: Option<usize>,
            cache_ttl: yggdryl::media::CacheTtl,
            level: Level,
        }

        impl $options {
            fn new() -> Self {
                Self {
                    name: SmolStr::new_static(yggdryl::media::DEFAULT_ROOT_NAME),
                    field: None,
                    filter: Filter::always_true(),
                    select: Selector::all(),
                    merge_by: Selector::all(),
                    safe: true,
                    batch_byte_size: None,
                    batch_row_size: None,
                    max_row_size: None,
                    row_offset: None,
                    max_byte_size: None,
                    commit_batch_num: None,
                    num_threads: None,
                    cache_ttl: yggdryl::media::CacheTtl::REALTIME,
                    level: Level::DEFAULT,
                }
            }
        }

        impl IORecordOptions for $options {
            yggdryl::record_options_fields!();
        }

        impl MediumSettings for $options {
            fn medium() -> &'static dyn MediaCodec {
                &$codec
            }
        }
    };
}

test_options!(TestOptions, TEST_CODEC);
test_options!(LateOptions, LATE_CODEC);

fn test_defaults() -> RecordOptions {
    RecordOptions::registered(TestOptions::new())
}

fn late_defaults() -> RecordOptions {
    RecordOptions::registered(LateOptions::new())
}

/// A medium stated by data: the leaf doors of every one store Arrow IPC.
#[derive(Debug)]
struct TestCodec {
    name: &'static str,
    title: &'static str,
    rank: u8,
    types: fn() -> &'static [MimeType],
    defaults: fn() -> RecordOptions,
}

/// The medium that is claimed first and stays.
static TEST_CODEC: TestCodec = TestCodec {
    name: "testmedium",
    title: "Test",
    rank: EXTERNAL_RANK,
    types: test_types,
    defaults: test_defaults,
};

/// The medium a test claims in the middle of its own body.
static LATE_CODEC: TestCodec = TestCodec {
    name: "latemedium",
    title: "Late",
    rank: EXTERNAL_RANK + 1,
    types: late_types,
    defaults: late_defaults,
};

/// A medium naming a type the test medium holds already.
static CLASH_CODEC: TestCodec = TestCodec {
    name: "clash",
    title: "Clash",
    rank: EXTERNAL_RANK + 2,
    types: clash_types,
    defaults: test_defaults,
};

/// A medium ranking inside the core's positions.
static LOW_RANK_CODEC: TestCodec = TestCodec {
    name: "lowrank",
    title: "Low",
    rank: EXTERNAL_RANK - 1,
    types: late_types,
    defaults: late_defaults,
};

/// A medium naming no MIME type.
static NO_TYPE_CODEC: TestCodec = TestCodec {
    name: "notype",
    title: "None",
    rank: EXTERNAL_RANK + 3,
    types: no_types,
    defaults: late_defaults,
};

impl MediaCodec for TestCodec {
    fn name(&self) -> &'static str {
        self.name
    }

    fn title(&self) -> &'static str {
        self.title
    }

    fn rank(&self) -> u8 {
        self.rank
    }

    fn mime_types(&self) -> &'static [MimeType] {
        (self.types)()
    }

    fn default_options(&self, _base: &MimeType) -> RecordOptions {
        (self.defaults)()
    }

    fn read_batch_reader(
        &self,
        handle: &dyn IOBase,
        declared: Option<&Field>,
        _options: &RecordOptions,
    ) -> Result<BatchReader> {
        Ok(ipc::read_batch_reader(
            handle,
            declared,
            &IpcOptions::new(),
        )?)
    }

    fn row_size(&self, handle: &dyn IOBase, _options: &RecordOptions) -> Result<u64> {
        let mut rows = 0_u64;
        for batch in ipc::read_batch_reader(handle, None, &IpcOptions::new())? {
            rows += batch.map_err(Error::Arrow)?.num_rows() as u64;
        }
        Ok(rows)
    }

    fn read_field(&self, handle: &dyn IOBase, options: &RecordOptions) -> Result<Field> {
        let mut stored = IpcOptions::new();
        stored.set_name(SmolStr::new(options.name()));
        stored.set_declared(options.field());
        Ok(ipc::read_field(handle, &stored)?)
    }

    fn overwrite_arrow_reader(
        &self,
        handle: &mut dyn IOBase,
        batches: BatchReader,
        _options: &RecordOptions,
    ) -> Result<()> {
        Ok(ipc::overwrite_arrow_reader(
            handle,
            batches,
            &IpcOptions::new(),
        )?)
    }

    fn open(&self, handle: Holder) -> Media {
        Media::Registered(Box::new(TestMedium {
            handle,
            field: None,
            late: self.name == LATE_CODEC.name,
        }))
    }
}

/// The wrapper a registered medium opens: a byte handle that answers records
/// under its medium's options and the field it was given.
#[derive(Debug)]
struct TestMedium {
    handle: Holder,
    field: Option<Field>,
    late: bool,
}

impl IOBase for TestMedium {
    yggdryl::delegate_iobase!(handle);
}

impl IOMedia for TestMedium {
    yggdryl::impl_default_iomedia!();

    fn record_options(&self) -> Result<RecordOptions> {
        let mut options = self.medium().default_options(&test_mime());
        if let Some(field) = &self.field {
            options.set_field(field.clone());
        }
        Ok(options)
    }
}

impl MediaWrapper for TestMedium {
    fn medium(&self) -> &'static dyn MediaCodec {
        if self.late { &LATE_CODEC } else { &TEST_CODEC }
    }

    fn handle(&self) -> &Holder {
        &self.handle
    }

    fn into_handle(self: Box<Self>) -> Holder {
        self.handle
    }

    fn with_field(self: Box<Self>, field: Field) -> Box<dyn MediaWrapper> {
        Box::new(TestMedium {
            field: Some(field),
            ..*self
        })
    }
}

/// A table format that locates nothing.
#[derive(Debug)]
struct TestFormat;

static TEST_FORMAT: TestFormat = TestFormat;

impl TableFormat for TestFormat {
    fn name(&self) -> &'static str {
        "testformat"
    }

    fn locate(&self, _handle: &dyn IOBase) -> Result<Option<Box<dyn LocatedTable>>> {
        Ok(None)
    }

    fn table(&self, path: Vec<SmolStr>, root: Holder, _inherited: &Properties) -> Result<Table> {
        Ok(Table::Media(Box::new(MediaTable::bound(path, root)?)))
    }
}

/// A locator for `ygtest:` whose every location is the same seven bytes.
#[derive(Debug)]
struct TestLocator;

static TEST_LOCATOR: TestLocator = TestLocator;

impl Locator for TestLocator {
    fn scheme(&self) -> Scheme {
        Scheme::from_str("ygtest").expect("a custom scheme")
    }

    fn names(&self, location: &Uri) -> bool {
        location.scheme() == &self.scheme()
    }

    fn holder(&self, _location: &Uri, _properties: &Properties) -> Result<Holder> {
        Ok(Holder::buffer(Buffer::from_bytes(b"located".to_vec())))
    }
}

/// A catalog factory claimed under a `type` word and a scheme, building an
/// empty memory catalog.
#[derive(Debug)]
struct TestFactory {
    word: Option<&'static str>,
    scheme: Option<&'static str>,
}

static TEST_FACTORY: TestFactory = TestFactory {
    word: Some("testcatalog"),
    scheme: Some("ygcat"),
};

impl CatalogFactory for TestFactory {
    fn type_word(&self) -> Option<&'static str> {
        self.word
    }

    fn scheme(&self) -> Option<Scheme> {
        self.scheme
            .map(|scheme| Scheme::from_str(scheme).expect("a custom scheme"))
    }

    fn catalog(
        &self,
        name: SmolStr,
        _url: &Url,
        _arn: Option<&Arn>,
        _properties: &Properties,
    ) -> Result<Catalog> {
        Ok(Catalog::from(MemoryCatalog::new(name)))
    }
}

/// Claim the test medium once, before anything reads the register.
fn installed() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        codec::claim(&TEST_CODEC, BY).expect("the external ranks hold free positions");
    });
}

/// The struct root the rows land under.
fn schema() -> Field {
    StructType::from_fields([DataType::Int64.required_field("id")])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
}

fn reader() -> BatchReader {
    let arrow = schema().into_arrow_schema().unwrap();
    let batch = RecordBatch::try_new(
        arrow.clone(),
        vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
    )
    .unwrap();
    yggdryl::arrow::batch_reader(arrow, [batch])
}

#[test]
fn a_claim_registers_the_medium_under_every_type_it_names() {
    installed();
    let held = codec_for(&test_mime()).unwrap();
    assert_eq!(held.name(), "testmedium");
    assert_eq!(held.title(), "Test");
    assert_eq!(held.rank(), EXTERNAL_RANK);
    assert!(std::ptr::addr_eq(held, &TEST_CODEC as &dyn MediaCodec));
    // The core's seven and this process's one, in rank order - and the late
    // medium, where its test ran first.
    let names: Vec<&str> = codecs()
        .iter()
        .map(|medium| medium.name())
        .filter(|name| *name != "latemedium")
        .collect();
    let mut expected = vec!["ipc"];
    if cfg!(feature = "parquet") {
        expected.push("parquet");
    }
    expected.extend(["avro", "text", "xmla", "csv", "excel", "testmedium"]);
    assert_eq!(names, expected);
}

#[test]
fn a_second_claim_is_refused_naming_the_first_claimant() {
    installed();
    for by in [BY, "another"] {
        let refused = codec::claim(&TEST_CODEC, by).unwrap_err();
        let text = refused.to_string();
        assert!(refused.is_conflict(), "{by}: {text}");
        assert!(text.contains(BY), "{by}: {text}");
        assert!(text.contains("application/x-yggdryl-test"), "{by}: {text}");
    }
    // One type of two held already refuses the whole claim.
    let refused = codec::claim(&CLASH_CODEC, "another").unwrap_err();
    assert!(refused.is_conflict(), "{refused}");
    assert!(refused.to_string().contains(BY), "{refused}");
}

#[test]
fn a_claim_is_all_or_none() {
    installed();
    // `clash` names a fresh type and the test medium's: stopped by the
    // second, it leaves the first unclaimed.
    codec::claim(&CLASH_CODEC, "another").unwrap_err();
    let fresh = mime("application/x-yggdryl-clash");
    let refused = codec_for(&fresh).unwrap_err().to_string();
    assert!(
        refused.contains("a record encoding this build implements"),
        "{refused}"
    );
    assert!(RecordOptions::for_mime_type(&fresh).is_err());
}

#[test]
fn a_claim_in_the_cores_name_a_low_rank_and_a_medium_without_a_type_are_refused() {
    installed();
    let as_core = codec::claim(&LATE_CODEC, "yggdryl")
        .unwrap_err()
        .to_string();
    assert!(
        as_core.contains("invalid record value at $.encoding"),
        "{as_core}"
    );
    assert!(as_core.contains("never as `yggdryl`"), "{as_core}");

    let low = codec::claim(&LOW_RANK_CODEC, "another")
        .unwrap_err()
        .to_string();
    assert!(
        low.contains("a medium outside the core ranks at or above 32, got 31 for `lowrank`"),
        "{low}"
    );

    let none = codec::claim(&NO_TYPE_CODEC, "another")
        .unwrap_err()
        .to_string();
    assert!(
        none.contains("a medium names at least one MIME type, `notype` names none"),
        "{none}"
    );

    // Nothing of what was refused is registered.
    let names: Vec<&str> = codecs().iter().map(|medium| medium.name()).collect();
    for refused in ["lowrank", "notype", "clash"] {
        assert!(!names.contains(&refused), "{refused} in {names:?}");
    }
    // The rank-31 medium names the late type too: it did not take it.
    if let Ok(held) = codec_for(&late_mime()) {
        assert_eq!(held.name(), "latemedium");
    }
}

#[test]
fn a_type_is_refused_naming_the_crate_to_install_and_answers_after_its_claim() {
    installed();
    let message = RecordOptions::for_mime_type(&late_mime())
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("a record encoding this build implements"),
        "{message}"
    );
    assert!(
        message.contains("install the crate that claims it"),
        "{message}"
    );
    assert!(
        message.contains("got application/x-yggdryl-late"),
        "{message}"
    );
    // The list names what is claimed, the test medium's type among it.
    assert!(message.contains("application/x-yggdryl-test"), "{message}");
    assert!(Media::open_as(Holder::buffer(Buffer::new()), &late_mime()).is_err());

    codec::claim(&LATE_CODEC, BY).expect("a free type and a free rank");

    let options = RecordOptions::for_mime_type(&late_mime()).unwrap();
    assert!(matches!(options, RecordOptions::Registered(_)));
    assert_eq!(options.codec().name(), "latemedium");
    assert_eq!(options.mime_type(), late_mime());
    assert!(options.settings::<LateOptions>().is_some());
    assert!(options.settings::<TestOptions>().is_none());
    let media = Media::open_as(Holder::buffer(Buffer::new()), &late_mime()).unwrap();
    assert_eq!(media.medium().name(), "latemedium");
}

#[test]
fn a_claimed_type_names_registered_options_reached_by_type() {
    installed();
    let options = RecordOptions::for_mime_type(&test_mime()).unwrap();
    assert!(matches!(options, RecordOptions::Registered(_)));
    assert_eq!(options.codec().name(), "testmedium");
    assert_eq!(options.mime_type(), test_mime());
    assert_eq!(
        options,
        RecordOptions::for_media_type(&MediaType::from(test_mime())).unwrap(),
        "the type and its media type name one value"
    );

    // The typed door answers the medium's own struct and no other.
    assert_eq!(options.settings::<TestOptions>(), Some(&TestOptions::new()));
    assert!(options.settings::<IpcOptions>().is_none());
    assert!(options.settings::<AvroOptions>().is_none());
    #[cfg(feature = "parquet")]
    assert!(
        options
            .settings::<yggdryl::parquet::ParquetOptions>()
            .is_none()
    );
    assert!(options.require_settings::<TestOptions>().is_ok());
    assert!(options.require_settings::<IpcOptions>().is_err());
    let mut options = options;
    options
        .require_settings_mut::<TestOptions>("$.x", "a thing")
        .unwrap()
        .safe = false;
    assert!(!options.safe());
    assert_ne!(options, RecordOptions::for_mime_type(&test_mime()).unwrap());
}

#[test]
fn another_medium_refuses_the_test_mediums_settings_naming_both() {
    installed();
    let mut ipc = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).unwrap();
    assert!(ipc.settings::<TestOptions>().is_none());
    assert!(ipc.settings_mut::<TestOptions>().is_none());
    let error = ipc
        .require_settings_mut::<TestOptions>("$.x", "a thing")
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid record value at $.x: expected Test options to set a thing, got \
         application/vnd.apache.arrow.stream options"
    );
    let error = ipc.require_settings::<TestOptions>().unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid record value at $.encoding: expected Test options, got \
         application/vnd.apache.arrow.stream options"
    );

    // And the other way round.
    let mut test = RecordOptions::for_mime_type(&test_mime()).unwrap();
    let error = test
        .require_settings_mut::<IpcOptions>("$.x", "a thing")
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid record value at $.x: expected Arrow IPC options to set a thing, got \
         application/x-yggdryl-test options"
    );
}

#[test]
fn registered_options_order_by_rank_then_by_value() {
    installed();
    let test = RecordOptions::for_mime_type(&test_mime()).unwrap();
    let excel = RecordOptions::for_mime_type(&MimeType::XLSX).unwrap();
    let ipc = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).unwrap();
    // The core's seven hold the positions below the external ones.
    assert!(ipc < excel && excel < test);
    // A medium of a higher rank sorts after, whatever its options hold.
    let late = late_defaults();
    assert!(test < late);
    let widest = test.clone().with_max_row_size(u64::MAX);
    assert!(widest < late);
    // Of one medium, the options decide.
    assert!(test < widest);
    assert_eq!(test.cmp(&test.clone()), std::cmp::Ordering::Equal);
}

#[cfg(feature = "internals")]
#[test]
fn the_registered_hash_is_the_media_tag_then_the_whole_struct() {
    use yggdryl::internals::hashing_stable::stable_hash_of;

    installed();
    let settings = TestOptions::new();
    let options = RecordOptions::registered(settings.clone());
    assert_eq!(
        options.stable_hash(),
        stable_hash_of(&("testmedium", &settings))
    );
    // A shared section feeds it, as it does every medium's.
    let changed = options.clone().with_max_row_size(5);
    assert_ne!(changed.stable_hash(), options.stable_hash());
    // The tag is the medium's: the same struct under the other medium's name
    // hashes apart.
    assert_ne!(
        stable_hash_of(&("latemedium", &settings)),
        options.stable_hash()
    );
}

#[test]
fn a_handle_of_the_claimed_type_round_trips_its_rows_through_the_medium() {
    installed();
    let mut handle = Buffer::new().with_media_type(MediaType::from(test_mime()));
    let options = handle.record_options().unwrap().with_field(schema());
    assert_eq!(options.codec().name(), "testmedium");

    handle.overwrite_arrow_reader(reader(), &options).unwrap();
    // The medium stores Arrow IPC: a stream opens with the continuation
    // marker.
    assert_eq!(handle.read_range_bytes(0, 4).unwrap(), [0xFF; 4]);

    let rows: usize = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 3);
    assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    assert_eq!(handle.row_size().unwrap(), 3);
}

#[test]
fn open_as_builds_the_wrapper_the_medium_opens() {
    installed();
    let media = Media::open_as(Holder::buffer(Buffer::new()), &test_mime()).unwrap();
    assert!(matches!(media, Media::Registered(_)));
    assert_eq!(media.medium().name(), "testmedium");
    assert!(matches!(media.handle(), Holder::Buffer(_)));

    let mut media = media.with_field(schema());
    let options = media.record_options().unwrap();
    assert_eq!(options.codec().name(), "testmedium");
    assert_eq!(options.field(), Some(schema()));
    media.overwrite_arrow_reader(reader(), &options).unwrap();
    assert_eq!(media.read_range_bytes(0, 4).unwrap(), [0xFF; 4]);
    assert_eq!(media.row_size().unwrap(), 3);
    assert_eq!(media.read_arrow_field(&options).unwrap(), schema());
    assert!(matches!(media.into_handle(), Holder::Buffer(_)));

    // The type names its medium whatever the handle's own name says.
    let named =
        Holder::buffer(Buffer::new().with_media_type(MediaType::from(MimeType::ARROW_STREAM)));
    let media = Media::open_as(named, &test_mime()).unwrap();
    assert_eq!(media.medium().name(), "testmedium");
    // And an unclaimed type is refused naming the crate to install.
    let error = Media::open_as(Holder::buffer(Buffer::new()), &MimeType::ORC)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("a record encoding this build implements"),
        "{error}"
    );
    assert!(
        error.contains("install the crate that claims it"),
        "{error}"
    );
}

#[test]
fn a_table_format_is_claimed_once_and_listed() {
    assert!(format_named("testformat").is_none());
    format::claim(&TEST_FORMAT, BY).expect("a free name");
    assert!(std::ptr::addr_eq(
        format_named("testformat").unwrap(),
        &TEST_FORMAT as &dyn TableFormat
    ));
    assert!(formats().iter().any(|held| held.name() == "testformat"));

    for by in [BY, "another"] {
        let refused = format::claim(&TEST_FORMAT, by).unwrap_err();
        assert!(refused.is_conflict(), "{by}: {refused}");
        assert!(refused.to_string().contains(BY), "{by}: {refused}");
    }
    let as_core = format::claim(&TEST_FORMAT, "yggdryl")
        .unwrap_err()
        .to_string();
    assert!(as_core.contains("never as `yggdryl`"), "{as_core}");
    // A format that locates nothing leaves a plain handle a plain handle.
    let mut handle = Buffer::new().with_media_type(MediaType::from(MimeType::ARROW_STREAM));
    let options = handle.record_options().unwrap().with_field(schema());
    handle.overwrite_arrow_reader(reader(), &options).unwrap();
    assert_eq!(handle.row_size().unwrap(), 3);
}

#[test]
fn a_locator_is_claimed_once_and_reached_by_its_scheme() {
    let location = Url::from_str("ygtest://lake/trades").unwrap();
    let none = std::iter::empty::<(&str, &str)>();
    let refused = Holder::from_url(&location, none).unwrap_err().to_string();
    assert!(
        refused.contains("install the crate that claims it and call its `install()`"),
        "{refused}"
    );

    claim_locator(&TEST_LOCATOR, BY).expect("a free scheme");
    assert!(
        locators()
            .iter()
            .any(|held| held.scheme().as_str() == "ygtest")
    );
    let held = Holder::from_url(&location, std::iter::empty::<(&str, &str)>()).unwrap();
    assert_eq!(held.read_all_bytes().unwrap(), b"located");

    for by in [BY, "another"] {
        let refused = claim_locator(&TEST_LOCATOR, by).unwrap_err();
        assert!(refused.is_conflict(), "{by}: {refused}");
        assert!(refused.to_string().contains(BY), "{by}: {refused}");
    }
    let as_core = claim_locator(&TEST_LOCATOR, "yggdryl")
        .unwrap_err()
        .to_string();
    assert!(as_core.contains("never as `yggdryl`"), "{as_core}");
}

#[test]
fn a_catalog_factory_is_claimed_once_and_reached_by_its_word() {
    let location = Url::from_str("file:///lake").unwrap();
    let stated = Properties::new()
        .with_property("type", "testcatalog")
        .with_property("name", "probe");
    let refused = Catalog::from_url(&location, &stated)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("$.with.type"), "{refused}");
    assert!(refused.contains("\"testcatalog\""), "{refused}");

    claim_factory(&TEST_FACTORY, BY).expect("a free word and a free scheme");
    assert!(
        factories()
            .iter()
            .any(|held| held.type_word() == Some("testcatalog"))
    );
    let catalog = Catalog::from_url(&location, &stated).unwrap();
    assert!(matches!(catalog, Catalog::Memory(_)));
    assert_eq!(ObjectValue::name(&catalog), "probe");
    // The word is listed among the ones the refusal of another offers.
    let other = Catalog::from_url(&location, &stated.clone().with_property("type", "glue"))
        .unwrap_err()
        .to_string();
    assert!(other.contains("`testcatalog`"), "{other}");

    for by in [BY, "another"] {
        let refused = claim_factory(&TEST_FACTORY, by).unwrap_err();
        assert!(refused.is_conflict(), "{by}: {refused}");
        assert!(refused.to_string().contains(BY), "{by}: {refused}");
    }
    let as_core = claim_factory(&TEST_FACTORY, "yggdryl")
        .unwrap_err()
        .to_string();
    assert!(as_core.contains("never as `yggdryl`"), "{as_core}");

    // A word the core answers itself, and a factory naming neither a word
    // nor a scheme, are refused.
    static CORE_WORD: TestFactory = TestFactory {
        word: Some("memory"),
        scheme: None,
    };
    static NEITHER: TestFactory = TestFactory {
        word: None,
        scheme: None,
    };
    let core_word = claim_factory(&CORE_WORD, "another")
        .unwrap_err()
        .to_string();
    assert!(core_word.contains("the core answers it"), "{core_word}");
    let neither = claim_factory(&NEITHER, "another").unwrap_err().to_string();
    assert!(neither.contains("got neither"), "{neither}");
}
