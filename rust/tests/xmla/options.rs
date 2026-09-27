//! `rust/src/xmla/options.rs`: the settings an XMLA rowset document is read
//! and written with - the shared record settings, the envelope, the method
//! and the content - and the one `RecordOptions` variant they are.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, RecordBatchReader, StringArray};

use yggdryl::arrow::BatchReader;
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::soap::ENVELOPE_NAMESPACE;
use yggdryl::xmla::{Content, Method, NAMESPACE, ROWSET_NAMESPACE, XmlaOptions};
use yggdryl::{
    Codec, DataType, Field, Filter, IOBase, IOMedia, IOMode, Level, MediaType, MimeType, Plan,
    Scalar, Selector, StructType, Timezone, Url,
};

/// The rowset the fixtures read and write: `id` and a nullable `symbol`.
fn schema() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

/// Two rows, the second without a symbol.
fn batch() -> RecordBatch {
    RecordBatch::try_new(
        schema().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )
    .unwrap()
}

/// One reader over the two-row batch.
fn reader() -> BatchReader {
    let batch = batch();
    yggdryl::arrow::batch_reader(batch.schema(), [batch])
}

/// An empty in-memory handle declaring the XMLA media type.
fn handle() -> Buffer {
    Buffer::new().with_media_type(MediaType::new(MimeType::XMLA))
}

/// An in-memory handle holding `document`, declaring the XMLA media type.
fn holding(document: &str) -> Buffer {
    let mut handle = handle();
    handle.write_all_bytes(document.as_bytes()).unwrap();
    handle
}

/// The document `options` write the two-row batch as.
fn written(options: XmlaOptions) -> String {
    let mut handle = handle();
    handle
        .overwrite_arrow_batch(batch(), &RecordOptions::from(options))
        .unwrap();
    String::from_utf8(handle.as_slice().to_vec()).unwrap()
}

/// The `id` and `symbol` columns of every batch a reader yields.
fn rows(reader: BatchReader) -> (Vec<i64>, Vec<Option<String>>) {
    let mut ids = Vec::new();
    let mut symbols = Vec::new();
    for batch in reader {
        let batch = batch.unwrap();
        let id = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        ids.extend(id.values().iter().copied());
        let symbol = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        symbols.extend(
            (0..symbol.len())
                .map(|index| (!symbol.is_null(index)).then(|| symbol.value(index).to_owned())),
        );
    }
    (ids, symbols)
}

/// The standard library's hash of one value.
fn std_hash(value: &XmlaOptions) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// What the two-row batch reads back as.
fn expected() -> (Vec<i64>, Vec<Option<String>>) {
    (vec![1, 2], vec![Some("AAPL".to_owned()), None])
}

// Refusals.

#[test]
fn a_write_with_no_declared_field_names_the_builders_that_declare_one() {
    let error = XmlaOptions::new().require_field().unwrap_err();
    assert!(matches!(error, yggdryl::Error::InvalidRecord { .. }));
    let message = error.to_string();
    assert!(message.contains("at $:"), "{message}");
    assert!(message.contains("with_field"), "{message}");
    assert!(message.contains("with_dtype"), "{message}");
}

#[test]
fn a_match_key_naming_a_column_twice_is_refused_naming_the_repeat() {
    let error = XmlaOptions::new().with_merge_by("id, ID").unwrap_err();
    let message = error.to_string();
    assert!(message.contains("$.merge_by"), "{message}");
    assert!(
        message.contains("expected each match key column once"),
        "{message}"
    );
    assert!(message.contains("\"ID\" twice"), "{message}");

    let planned = XmlaOptions::new()
        .with_plan("upsert by (id, id)")
        .unwrap_err()
        .to_string();
    assert!(planned.contains("\"id\" twice"), "{planned}");
}

#[test]
fn a_refused_match_key_scalar_leaves_the_options_unchanged() {
    // `set_*` is a validated in-place update: a failure leaves self as it was.
    let mut options = XmlaOptions::new().with_merge_by("id").unwrap();
    let before = options.clone();
    let error = options
        .set_merge_by_scalar(&Scalar::from("symbol, symbol"))
        .unwrap_err();
    assert!(error.to_string().contains("\"symbol\" twice"), "{error}");
    assert_eq!(options, before);
}

#[test]
fn a_refused_plan_leaves_the_options_unchanged() {
    let mut options = XmlaOptions::new()
        .with_filter("id > 1")
        .unwrap()
        .with_merge_by("id")
        .unwrap();
    let before = options.clone();
    let error = options
        .set_plan("upsert by (id, id) select id".parse::<Plan>().unwrap())
        .unwrap_err();
    assert!(error.to_string().contains("\"id\" twice"), "{error}");
    assert_eq!(options, before);
}

#[test]
fn a_filter_or_selector_that_does_not_parse_is_refused() {
    // Each refusal names the byte it stopped at and what it found there.
    for (error, position, found) in [
        (
            XmlaOptions::new().with_filter("id >").unwrap_err(),
            4,
            "the end of the expression",
        ),
        (
            XmlaOptions::new().with_select("id,,").unwrap_err(),
            3,
            "\",\"",
        ),
        (
            XmlaOptions::new()
                .with_plan("select from where")
                .unwrap_err(),
            7,
            "the reserved word \"from\"",
        ),
    ] {
        assert!(
            matches!(error, yggdryl::Error::Parse { position: at, .. } if at == position),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(
            message.contains(&format!("at byte {position}")),
            "{message}"
        );
        assert!(message.contains(&format!("got {found}")), "{message}");
    }
    let mut options = XmlaOptions::new();
    let error = options
        .set_filter_scalar(&Scalar::from("id >"))
        .unwrap_err();
    assert!(
        error.to_string().contains("got the end of the expression"),
        "{error}"
    );
    assert_eq!(options, XmlaOptions::new());
}

#[test]
fn a_bounded_merge_is_refused_naming_both_settings() {
    let options = XmlaOptions::new()
        .with_merge_by("id")
        .unwrap()
        .with_max_row_size(3)
        .with_max_byte_size(1024);
    let error = options.require_write_limits().unwrap_err();
    let message = error.to_string();
    assert!(message.contains("max_row_size = 3"), "{message}");
    assert!(message.contains("max_byte_size = 1024"), "{message}");
    assert!(message.contains("merge_by `id`"), "{message}");
    let Err(limited) = options.limit_arrow_reader(reader()) else {
        panic!("a bounded merge limits nothing");
    };
    assert_eq!(limited.to_string(), message);
}

#[test]
fn the_xmla_variant_refuses_the_settings_of_another_encoding() {
    let mut options = RecordOptions::from(XmlaOptions::new());
    let before = options.clone();

    assert_eq!(options.timezone(), None);
    let timezone = options.set_timezone(Some(Timezone::UTC)).unwrap_err();
    let message = timezone.to_string();
    assert!(message.contains("$.timezone"), "{message}");
    assert!(
        message.contains("expected text options to set an autotyping timezone"),
        "{message}"
    );
    assert!(
        message.contains("got application/xmla+xml options"),
        "{message}"
    );

    assert_eq!(options.avro_block_codec(), None);
    assert_eq!(options.avro_sync_marker(), None);
    for (error, path) in [
        (
            options.set_avro_block_codec("null").unwrap_err(),
            "$.block_codec",
        ),
        (
            options.set_avro_sync_marker(None).unwrap_err(),
            "$.sync_marker",
        ),
    ] {
        let message = error.to_string();
        assert!(message.contains(path), "{message}");
        assert!(message.contains("expected Avro options"), "{message}");
        assert!(
            message.contains("got application/xmla+xml options"),
            "{message}"
        );
    }

    #[cfg(feature = "parquet")]
    {
        assert_eq!(options.parquet_compression_name(), None);
        assert_eq!(options.parquet_max_row_group_size(), None);
        assert_eq!(options.parquet_key_value_metadata(), None);
        let message = options
            .set_parquet_max_row_group_size(10)
            .unwrap_err()
            .to_string();
        assert!(message.contains("expected Parquet options"), "{message}");
        assert!(
            message.contains("got application/xmla+xml options"),
            "{message}"
        );
    }

    assert_eq!(options, before);
}

#[test]
fn a_write_mode_is_checked_against_the_match_key() {
    let plain = RecordOptions::from(XmlaOptions::new());
    plain.require_write_mode(IOMode::Overwrite).unwrap();
    plain.require_write_mode(IOMode::Append).unwrap();
    let message = plain
        .require_write_mode(IOMode::Merge)
        .unwrap_err()
        .to_string();
    assert!(message.contains("$.merge_by"), "{message}");
    assert!(
        message.contains("write mode merge requires at least one merge_by column"),
        "{message}"
    );

    let keyed = RecordOptions::from(XmlaOptions::new().with_merge_by("id").unwrap());
    keyed.require_write_mode(IOMode::Merge).unwrap();
    for mode in [IOMode::Overwrite, IOMode::Append] {
        let message = keyed.require_write_mode(mode).unwrap_err().to_string();
        assert!(message.contains("does not accept merge_by"), "{message}");
    }
}

#[test]
fn a_zero_commit_cadence_is_refused_before_a_write() {
    let zero = RecordOptions::from(XmlaOptions::new().with_commit_row_size(0));
    let message = zero.require_commit_row_size().unwrap_err().to_string();
    assert!(message.contains("$.commit_row_size"), "{message}");
    assert!(message.contains("non-zero row count, got 0"), "{message}");

    let every_three = RecordOptions::from(XmlaOptions::new().with_commit_row_size(3));
    assert_eq!(every_three.require_commit_row_size().unwrap(), Some(3));
    let once = RecordOptions::from(XmlaOptions::new());
    assert_eq!(once.require_commit_row_size().unwrap(), None);
}

#[test]
fn an_encoding_no_variant_covers_names_the_xmla_media_type_among_those_that_are() {
    let message = RecordOptions::for_mime_type(&MimeType::XML)
        .unwrap_err()
        .to_string();
    assert!(message.contains("application/xmla+xml"), "{message}");
    assert!(message.contains("application/xml"), "{message}");
}

// Defaults.

#[test]
fn new_and_default_are_one_value() {
    assert_eq!(XmlaOptions::new(), XmlaOptions::default());
    // The module path and the re-export name one type.
    let options: yggdryl::xmla::options::XmlaOptions = XmlaOptions::default();
    assert_eq!(options, XmlaOptions::new());
}

#[test]
fn the_defaults_write_an_execute_response_holding_the_schema_and_the_rows() {
    let options = XmlaOptions::new();
    assert!(options.envelope);
    assert_eq!(options.method, Method::Execute);
    assert_eq!(options.content, Content::SchemaData);
    assert_eq!(options.content, Content::DEFAULT);
}

#[test]
fn every_shared_setting_starts_unset() {
    let options = XmlaOptions::new();
    assert_eq!(options.name, yggdryl::media::DEFAULT_ROOT_NAME);
    assert_eq!(options.name(), "row");
    assert_eq!(options.field, None);
    assert_eq!(options.field(), None);
    assert_eq!(options.declared(), None);
    assert_eq!(options.filter, Filter::always_true());
    assert!(options.filter().is_always_true());
    assert_eq!(options.select, Selector::all());
    assert!(options.select().is_all());
    assert_eq!(options.merge_by, Selector::all());
    assert!(options.merge_by().is_empty());
    assert!(!options.safe);
    assert!(!options.safe());
    assert_eq!(options.batch_byte_size, None);
    assert_eq!(options.batch_byte_size(), None);
    assert_eq!(options.batch_row_size, None);
    assert_eq!(options.batch_row_size(), None);
    assert_eq!(options.write_batch_row_size(), None);
    assert_eq!(options.max_row_size, None);
    assert_eq!(options.max_row_size(), None);
    assert_eq!(options.max_byte_size, None);
    assert_eq!(options.max_byte_size(), None);
    assert!(!options.write_limit_is_zero());
    assert_eq!(options.commit_row_size, None);
    assert_eq!(options.commit_row_size(), None);
    assert_eq!(options.level, Level::DEFAULT);
    assert_eq!(options.level(), Level::DEFAULT);
    assert!(options.plan().is_empty());
    assert_eq!(options.apply_columns(), None);
    assert!(options.partition_pairs().is_empty());
}

// The XMLA settings.

#[test]
fn without_envelope_writes_the_bare_rowset_and_touches_nothing_else() {
    let options = XmlaOptions::new().without_envelope();
    assert!(!options.envelope);
    assert_eq!(options.method, Method::Execute);
    assert_eq!(options.content, Content::SchemaData);
    assert_eq!(
        XmlaOptions {
            envelope: true,
            ..options.clone()
        },
        XmlaOptions::new()
    );
    // Taking the envelope off twice is taking it off once.
    assert_eq!(options.clone().without_envelope(), options);
}

#[test]
fn with_method_and_with_content_replace_only_their_own_setting() {
    for method in Method::ALL {
        let options = XmlaOptions::new().with_method(method);
        assert_eq!(options.method, method);
        assert!(options.envelope);
        assert_eq!(options.content, Content::SchemaData);
    }
    for content in Content::ALL.iter().copied() {
        let options = XmlaOptions::new().with_content(content);
        assert_eq!(options.content, content);
        assert!(options.envelope);
        assert_eq!(options.method, Method::Execute);
    }
    // The last one stated wins.
    let options = XmlaOptions::new()
        .with_method(Method::Discover)
        .with_content(Content::Data)
        .with_method(Method::Execute)
        .with_content(Content::Schema);
    assert_eq!(options.method, Method::Execute);
    assert_eq!(options.content, Content::Schema);
}

#[test]
fn the_xmla_settings_are_part_of_the_value_and_of_its_hash() {
    let variants = [
        XmlaOptions::new(),
        XmlaOptions::new().without_envelope(),
        XmlaOptions::new().with_method(Method::Discover),
        XmlaOptions::new().with_content(Content::Data),
        XmlaOptions::new().with_content(Content::Schema),
        XmlaOptions::new().with_content(Content::None),
    ];
    for (index, left) in variants.iter().enumerate() {
        for (other, right) in variants.iter().enumerate() {
            assert_eq!(left == right, index == other, "{left:?} against {right:?}");
        }
    }
    let hashed: HashSet<u64> = variants.iter().map(std_hash).collect();
    assert_eq!(hashed.len(), variants.len());
    assert_eq!(
        std_hash(&XmlaOptions::new()),
        std_hash(&XmlaOptions::default())
    );
    let hashes: HashSet<u64> = variants
        .iter()
        .map(|options| RecordOptions::from(options.clone()).stable_hash())
        .collect();
    assert_eq!(hashes.len(), variants.len());
    // The hash is deterministic, and the encoding is part of it: the same
    // shared settings under IPC are another value.
    assert_eq!(
        RecordOptions::from(XmlaOptions::new()).stable_hash(),
        RecordOptions::from(XmlaOptions::new()).stable_hash()
    );
    assert_ne!(
        RecordOptions::from(XmlaOptions::new()).stable_hash(),
        RecordOptions::from(yggdryl::ipc::IpcOptions::new()).stable_hash()
    );
}

// The shared settings, each through its setter and its getter.

#[test]
fn declaring_a_field_names_the_root_and_stores_it_non_null() {
    let mut options = XmlaOptions::new();
    options.set_field(schema().with_name("trade"));
    assert_eq!(options.name(), "trade");
    assert_eq!(options.name, "trade");
    assert_eq!(options.field().unwrap().name(), "trade");
    assert_eq!(options.field().unwrap().dtype(), schema().dtype());

    // The declared field's nullability is not part of the declaration.
    let nullable = XmlaOptions::new().with_field(schema().with_nullable(true));
    assert!(!nullable.field().unwrap().is_nullable());
    assert_eq!(nullable, XmlaOptions::new().with_field(schema()));

    // A datatype declares the same root under the name the options carry.
    assert_eq!(
        XmlaOptions::new().with_dtype(schema().dtype().clone()),
        XmlaOptions::new().with_field(schema())
    );
    assert_eq!(
        XmlaOptions::new()
            .with_name("trade")
            .with_dtype(schema().dtype().clone())
            .field(),
        Some(schema().with_name("trade"))
    );
    assert_eq!(
        XmlaOptions::new()
            .with_field(schema())
            .require_field()
            .unwrap(),
        schema()
    );
}

#[test]
fn renaming_the_root_renames_the_declared_field() {
    let mut options = XmlaOptions::new().with_field(schema());
    options.set_name("trades".into());
    assert_eq!(options.name(), "trades");
    assert_eq!(options.field().unwrap().name(), "trades");

    // With nothing declared the name alone moves.
    let bare = XmlaOptions::new().with_name("Kurs ü");
    assert_eq!(bare.name(), "Kurs ü");
    assert_eq!(bare.field(), None);
    assert_eq!(bare.with_name("").name(), "");
}

#[test]
fn taking_the_field_clears_the_declaration_and_keeps_the_name() {
    let mut options = XmlaOptions::new().with_field(schema().with_name("trade"));
    assert_eq!(options.take_field(), Some(schema().with_name("trade")));
    assert_eq!(options.field(), None);
    assert_eq!(options.name(), "trade");
    assert_eq!(options.take_field(), None);

    let mut cleared = XmlaOptions::new().with_field(schema());
    cleared.set_declared(None);
    assert_eq!(cleared.declared(), None);
    assert_eq!(cleared.name(), "row");
}

#[test]
fn the_filter_the_selector_and_the_match_key_round_trip() {
    let mut options = XmlaOptions::new();
    let filter: Filter = "id > 1".parse().unwrap();
    options.set_filter(filter.clone());
    assert_eq!(options.filter(), &filter);
    assert_eq!(options.filter, filter);

    let select: Selector = "id, symbol".parse().unwrap();
    options.set_select(select.clone());
    assert_eq!(options.select(), &select);
    assert_eq!(options.select, select);

    let merge_by: Selector = "id".parse().unwrap();
    options.set_merge_by(merge_by.clone());
    assert_eq!(options.merge_by(), &merge_by);
    assert_eq!(options.merge_by, merge_by);

    // The builders take text, and a list of names is a list of columns.
    let built = XmlaOptions::new()
        .with_filter("where id > 1")
        .unwrap()
        .with_select("select id, symbol")
        .unwrap()
        .with_merge_by(["id"])
        .unwrap();
    assert_eq!(built.filter(), &filter);
    assert_eq!(built.select(), &select);
    assert_eq!(built.merge_by(), &merge_by);
}

#[test]
fn the_scalar_setters_read_what_the_builders_read() {
    let mut options = XmlaOptions::new();
    options.set_filter_scalar(&Scalar::from("id > 1")).unwrap();
    options
        .set_select_scalar(&Scalar::from("id, symbol"))
        .unwrap();
    options.set_merge_by_scalar(&Scalar::from("id")).unwrap();
    let built = XmlaOptions::new()
        .with_filter("id > 1")
        .unwrap()
        .with_select("id, symbol")
        .unwrap()
        .with_merge_by("id")
        .unwrap();
    assert_eq!(options, built);
    assert_eq!(
        XmlaOptions::new()
            .with_filter_scalar(&Scalar::from("id > 1"))
            .unwrap()
            .with_select_scalar(&Scalar::from("id, symbol"))
            .unwrap()
            .with_merge_by_scalar(&Scalar::from("id"))
            .unwrap(),
        built
    );
}

#[test]
fn every_bound_and_the_level_round_trip() {
    let mut options = XmlaOptions::new();
    options.set_safe(true);
    assert!(options.safe());
    options.set_batch_byte_size(Some(4096));
    assert_eq!(options.batch_byte_size(), Some(4096));
    options.set_batch_row_size(Some(64));
    assert_eq!(options.batch_row_size(), Some(64));
    options.set_max_row_size(Some(10));
    assert_eq!(options.max_row_size(), Some(10));
    options.set_max_byte_size(Some(1 << 20));
    assert_eq!(options.max_byte_size(), Some(1 << 20));
    options.set_commit_row_size(Some(16));
    assert_eq!(options.commit_row_size(), Some(16));
    options.set_level(Level::BEST);
    assert_eq!(options.level(), Level::BEST);

    // The fields hold what the setters stated.
    assert!(options.safe);
    assert_eq!(options.batch_byte_size, Some(4096));
    assert_eq!(options.batch_row_size, Some(64));
    assert_eq!(options.max_row_size, Some(10));
    assert_eq!(options.max_byte_size, Some(1 << 20));
    assert_eq!(options.commit_row_size, Some(16));
    assert_eq!(options.level, Level::BEST);

    // The builders are the same setters.
    assert_eq!(
        XmlaOptions::new()
            .with_safe(true)
            .with_batch_byte_size(4096)
            .with_batch_row_size(64)
            .with_max_row_size(10)
            .with_max_byte_size(1 << 20)
            .with_commit_row_size(16)
            .with_level(Level::BEST),
        options
    );

    // `None` clears each bound again.
    options.set_batch_byte_size(None);
    options.set_batch_row_size(None);
    options.set_max_row_size(None);
    options.set_max_byte_size(None);
    options.set_commit_row_size(None);
    options.set_safe(false);
    options.set_level(Level::DEFAULT);
    assert_eq!(options, XmlaOptions::new());
}

#[test]
fn a_native_row_write_never_runs_past_the_next_commit() {
    let commit = XmlaOptions::new().with_commit_row_size(5);
    assert_eq!(commit.write_batch_row_size(), Some(5));
    assert_eq!(
        commit.clone().with_batch_row_size(3).write_batch_row_size(),
        Some(3)
    );
    assert_eq!(
        commit.with_batch_row_size(50).write_batch_row_size(),
        Some(5)
    );
    assert_eq!(
        XmlaOptions::new()
            .with_batch_row_size(7)
            .write_batch_row_size(),
        Some(7)
    );
}

#[test]
fn a_zero_bound_admits_no_row() {
    assert!(
        XmlaOptions::new()
            .with_max_row_size(0)
            .write_limit_is_zero()
    );
    assert!(
        XmlaOptions::new()
            .with_max_byte_size(0)
            .write_limit_is_zero()
    );
    assert!(
        !XmlaOptions::new()
            .with_max_row_size(1)
            .write_limit_is_zero()
    );
}

#[test]
fn the_row_bound_cuts_a_reader_at_the_exact_count() {
    let limited = XmlaOptions::new()
        .with_max_row_size(1)
        .limit_arrow_reader(reader())
        .unwrap();
    assert_eq!(rows(limited), (vec![1], vec![Some("AAPL".to_owned())]));
    let unbounded = XmlaOptions::new().limit_arrow_reader(reader()).unwrap();
    assert_eq!(rows(unbounded), expected());
}

#[test]
fn the_properties_are_the_sections_of_one_plan() {
    let options = XmlaOptions::new()
        .with_plan("create trade (id int64 not null) upsert by (id) select id where id > 1")
        .unwrap();
    assert_eq!(options.name(), "trade");
    assert_eq!(
        options.field().unwrap().dtype(),
        &DataType::from(StructType::from_fields([DataType::Int64.required_field("id")]).unwrap())
    );
    assert_eq!(options.merge_by().to_string(), "id");
    assert_eq!(options.select().to_string(), "id");
    assert_eq!(options.filter().to_string(), "id > 1");
    assert_eq!(
        options.plan().to_string(),
        "create trade (id int64 not null) upsert by (id) select id where id > 1"
    );
    // The composed plan reads back as the same options.
    assert_eq!(
        XmlaOptions::new().with_plan(options.plan()).unwrap(),
        options
    );
    assert_eq!(
        XmlaOptions::new()
            .with_plan_scalar(&Scalar::from(options.plan().to_string()))
            .unwrap(),
        options
    );
    // The XMLA settings are not sections of the plan and a plan leaves them.
    let enveloped = XmlaOptions::new()
        .without_envelope()
        .with_method(Method::Discover)
        .with_content(Content::Data)
        .with_plan("select id")
        .unwrap();
    assert!(!enveloped.envelope);
    assert_eq!(enveloped.method, Method::Discover);
    assert_eq!(enveloped.content, Content::Data);
}

#[test]
fn a_section_the_plan_does_not_spell_is_cleared() {
    let mut options = XmlaOptions::new()
        .with_field(schema())
        .with_filter("id > 1")
        .unwrap()
        .with_merge_by("id")
        .unwrap();
    options.set_plan_scalar(&Scalar::from("select id")).unwrap();
    assert_eq!(options.field(), None);
    assert!(options.filter().is_always_true());
    assert!(options.merge_by().is_empty());
    assert_eq!(options.select().to_string(), "id");
}

#[test]
fn the_plan_reads_which_columns_and_partitions_it_narrows_to() {
    let options = XmlaOptions::new()
        .with_select("id")
        .unwrap()
        .with_filter("venue = 'XNAS' and symbol is null")
        .unwrap();
    assert_eq!(
        options.apply_columns(),
        Some(vec![
            "venue".to_owned(),
            "symbol".to_owned(),
            "id".to_owned()
        ])
    );
    assert_eq!(
        options.partition_pairs(),
        vec![
            ("venue".to_owned(), "XNAS".to_owned()),
            (
                "symbol".to_owned(),
                yggdryl::media::NULL_PARTITION.to_owned()
            ),
        ]
    );
}

// The one `RecordOptions` variant.

#[test]
fn the_xmla_media_type_names_the_xmla_variant() {
    let media_type = MediaType::from_str("application/xmla+xml").unwrap();
    assert_eq!(media_type.base(), &MimeType::XMLA);
    let options = RecordOptions::for_media_type(&media_type).unwrap();
    assert_eq!(options, RecordOptions::Xmla(XmlaOptions::new()));
    assert_eq!(options.mime_type(), MimeType::XMLA);
    assert_eq!(
        RecordOptions::for_mime_type(&MimeType::XMLA).unwrap(),
        options
    );
    // The name is matched without regard to case.
    let shouted = MediaType::from_str("APPLICATION/XMLA+XML").unwrap();
    assert_eq!(
        RecordOptions::for_media_type(&shouted).unwrap(),
        RecordOptions::Xmla(XmlaOptions::new())
    );
}

#[test]
fn a_content_coding_does_not_change_the_record_encoding() {
    let url = Url::from_str("file:///a.xmla.gz").unwrap();
    let media_type = url.media_type();
    assert_eq!(media_type.base(), &MimeType::XMLA);
    assert_eq!(media_type.encoding(), Some(&MimeType::GZIP));
    assert_eq!(
        RecordOptions::for_media_type(&media_type).unwrap(),
        RecordOptions::Xmla(XmlaOptions::new())
    );
}

#[test]
fn the_options_convert_into_the_xmla_variant_unchanged() {
    let options = XmlaOptions::new()
        .without_envelope()
        .with_method(Method::Discover)
        .with_content(Content::Data)
        .with_field(schema())
        .with_max_row_size(9);
    let record = RecordOptions::from(options.clone());
    assert_eq!(record.mime_type(), MimeType::XMLA);
    assert_eq!(record.field(), Some(schema()));
    assert_eq!(record.name(), "row");
    assert_eq!(record.max_row_size(), Some(9));
    let RecordOptions::Xmla(inner) = record else {
        panic!("XMLA options convert into the XMLA variant");
    };
    assert_eq!(inner, options);
}

#[test]
fn the_variant_forwards_every_shared_setting_to_the_xmla_options() {
    let mut record = RecordOptions::from(XmlaOptions::new());
    record.set_name("trade".into());
    record.set_declared(Some(schema()));
    record.set_filter("id > 1".parse().unwrap());
    record.set_select("id".parse().unwrap());
    record.set_merge_by("id".parse().unwrap());
    record.set_safe(true);
    record.set_batch_byte_size(Some(8));
    record.set_batch_row_size(Some(4));
    record.set_max_row_size(Some(3));
    record.set_max_byte_size(Some(2));
    record.set_commit_row_size(Some(1));
    record.set_level(Level::FAST);
    let RecordOptions::Xmla(inner) = record else {
        unreachable!("the setters keep the variant");
    };
    assert_eq!(
        inner,
        XmlaOptions::new()
            .with_name("trade")
            .with_field(schema())
            .with_filter("id > 1")
            .unwrap()
            .with_select("id")
            .unwrap()
            .with_merge_by("id")
            .unwrap()
            .with_safe(true)
            .with_batch_byte_size(8)
            .with_batch_row_size(4)
            .with_max_row_size(3)
            .with_max_byte_size(2)
            .with_commit_row_size(1)
            .with_level(Level::FAST)
    );
}

#[test]
fn an_xmla_handle_answers_the_default_xmla_options() {
    assert_eq!(
        handle().record_options().unwrap(),
        RecordOptions::Xmla(XmlaOptions::new())
    );
    let named =
        Buffer::new().with_media_type(Url::from_str("file:///cube.XMLA").unwrap().media_type());
    assert_eq!(
        named.record_options().unwrap(),
        RecordOptions::Xmla(XmlaOptions::new())
    );
}

#[test]
fn the_xmla_suffix_is_the_xmla_media_type() {
    assert_eq!(
        Url::from_str("file:///a.xmla").unwrap().media_type(),
        MediaType::new(MimeType::XMLA)
    );
    assert_eq!(MimeType::from_extension("xmla").unwrap(), MimeType::XMLA);
    assert_eq!(MimeType::from_extension(".XMLA").unwrap(), MimeType::XMLA);
    assert_eq!(
        MimeType::from_str("application/xmla+xml").unwrap(),
        MimeType::XMLA
    );
    assert_eq!(
        MimeType::from_content_type("application/xmla+xml; charset=utf-8").unwrap(),
        MimeType::XMLA
    );
}

#[test]
fn the_xmla_mime_type_is_a_textual_tabular_record_encoding() {
    let mime = MimeType::XMLA;
    assert_eq!(mime.as_str(), "application/xmla+xml");
    assert_eq!(mime.to_string(), "application/xmla+xml");
    assert_eq!(mime.top_level(), "application");
    assert_eq!(mime.subtype(), "xmla+xml");
    assert_eq!(mime.structured_suffix(), Some("xml"));
    assert_eq!(mime.extension(), Some("xmla"));
    // An `+xml` suffix reads as an XML document; a rowset is read as records.
    assert_eq!(mime.format(), None);
    assert_ne!(MimeType::XML.format(), None);
    assert!(mime.is_tabular());
    assert!(mime.is_textual());
    assert!(mime.is_structured());
    assert!(mime.is_application());
    assert!(mime.is_known());
    assert!(!mime.is_encoding());
    assert!(!mime.is_archive());
    assert_eq!(mime.content_coding(), None);
    assert_ne!(mime, MimeType::XML);
}

// What the XMLA settings steer.

#[test]
fn an_enveloped_write_is_the_response_of_the_method() {
    let execute = written(XmlaOptions::new());
    assert!(execute.contains(ENVELOPE_NAMESPACE), "{execute}");
    assert!(
        execute.contains(&format!("<ExecuteResponse xmlns=\"{NAMESPACE}\"><return>")),
        "{execute}"
    );
    assert!(execute.contains("</return></ExecuteResponse>"), "{execute}");
    assert!(!execute.contains("DiscoverResponse"), "{execute}");
    assert!(
        execute.contains(&format!("<root xmlns=\"{ROWSET_NAMESPACE}\"")),
        "{execute}"
    );

    let discover = written(XmlaOptions::new().with_method(Method::Discover));
    assert!(
        discover.contains(&format!("<DiscoverResponse xmlns=\"{NAMESPACE}\"><return>")),
        "{discover}"
    );
    assert!(
        discover.contains("</return></DiscoverResponse>"),
        "{discover}"
    );
    assert!(!discover.contains("ExecuteResponse"), "{discover}");
}

#[test]
fn a_write_without_the_envelope_is_the_bare_rowset_root() {
    for method in Method::ALL {
        let document = written(XmlaOptions::new().without_envelope().with_method(method));
        assert!(
            document.starts_with(&format!("<root xmlns=\"{ROWSET_NAMESPACE}\"")),
            "{document}"
        );
        assert!(document.ends_with("</root>"), "{document}");
        assert!(!document.contains(ENVELOPE_NAMESPACE), "{document}");
        assert!(!document.contains("Response"), "{document}");
    }
}

#[test]
fn the_content_says_which_of_the_schema_and_the_rows_are_written() {
    for envelope in [true, false] {
        for (content, schema, data) in [
            (Content::SchemaData, true, true),
            (Content::Schema, true, false),
            (Content::Data, false, true),
            (Content::None, false, false),
        ] {
            let mut options = XmlaOptions::new().with_content(content);
            options.envelope = envelope;
            let document = written(options);
            assert_eq!(
                document.contains("<xsd:schema"),
                schema,
                "{content}: {document}"
            );
            assert_eq!(
                document.matches("<row>").count(),
                if data { 2 } else { 0 },
                "{content}: {document}"
            );
            assert!(
                document.contains(&format!("<root xmlns=\"{ROWSET_NAMESPACE}\"")),
                "{content}: {document}"
            );
        }
    }
}

#[test]
fn a_read_accepts_a_bare_rowset_whatever_the_envelope_setting() {
    let document = format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\">\
         <row><id>1</id><symbol>AAPL</symbol></row>\
         <row><id>2</id></row>\
         </root>"
    );
    for options in [
        XmlaOptions::new(),
        XmlaOptions::new().without_envelope(),
        XmlaOptions::new().with_method(Method::Discover),
    ] {
        let options = RecordOptions::from(options.with_field(schema()));
        let read = holding(&document).read_arrow_reader(&options).unwrap();
        assert_eq!(rows(read), expected());
    }
}

#[test]
fn a_read_accepts_an_envelope_of_either_method_whatever_the_settings() {
    for (method, prefix) in [(Method::Discover, "SOAP-ENV"), (Method::Execute, "soap")] {
        let response = method.response_name();
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
             <{prefix}:Envelope xmlns:{prefix}=\"{ENVELOPE_NAMESPACE}\">\
             <{prefix}:Body><{response} xmlns=\"{NAMESPACE}\"><return>\
             <root xmlns=\"{ROWSET_NAMESPACE}\">\
             <row><id>1</id><symbol>AAPL</symbol></row>\
             <row><id>2</id></row>\
             </root></return></{response}></{prefix}:Body></{prefix}:Envelope>"
        );
        for options in [
            XmlaOptions::new().without_envelope(),
            XmlaOptions::new().with_method(Method::Execute),
            XmlaOptions::new()
                .with_method(Method::Discover)
                .with_content(Content::Schema),
        ] {
            let options = RecordOptions::from(options.with_field(schema()));
            let read = holding(&document).read_arrow_reader(&options).unwrap();
            assert_eq!(rows(read), expected(), "{method} under {options:?}");
        }
    }
}

#[test]
fn every_document_shape_the_options_write_reads_back() {
    for envelope in [true, false] {
        for method in Method::ALL {
            for content in [Content::SchemaData, Content::Data] {
                let mut options = XmlaOptions::new().with_method(method).with_content(content);
                options.envelope = envelope;
                let mut handle = handle();
                handle
                    .overwrite_arrow_reader(reader(), &RecordOptions::from(options))
                    .unwrap();
                // A document without its schema is typed by the declared
                // field, whichever settings the read carries.
                let read = RecordOptions::from(XmlaOptions::new().with_field(schema()));
                assert_eq!(
                    rows(handle.read_arrow_reader(&read).unwrap()),
                    expected(),
                    "envelope {envelope}, {method}, {content}"
                );
            }
        }
    }
}

#[test]
fn the_root_name_names_a_schema_read_from_the_document() {
    let mut handle = handle();
    handle
        .overwrite_arrow_batch(batch(), &RecordOptions::from(XmlaOptions::new()))
        .unwrap();
    let field = handle
        .read_arrow_field(&RecordOptions::from(XmlaOptions::new().with_name("trades")))
        .unwrap();
    assert_eq!(field.name(), "trades");
    assert_eq!(
        field.fields().iter().map(Field::name).collect::<Vec<_>>(),
        ["id", "symbol"]
    );
    // A declared field answers itself, without reading the document.
    let declared = handle
        .read_arrow_field(&RecordOptions::from(
            XmlaOptions::new().with_field(schema().with_name("declared")),
        ))
        .unwrap();
    assert_eq!(declared, schema().with_name("declared"));
}

// What the shared settings steer through an XMLA handle.

/// A rowset schema another writer could have saved, under the `xs` prefix
/// rather than this crate's `xsd`: a required `id` of `xs:long` and a
/// nullable `symbol` of `xs:string`.
const STATED_SCHEMA: &str = concat!(
    "<xs:schema targetNamespace=\"urn:schemas-microsoft-com:xml-analysis:rowset\" ",
    "xmlns:sql=\"urn:schemas-microsoft-com:xml-sql\" elementFormDefault=\"qualified\">",
    "<xs:complexType name=\"row\"><xs:sequence>",
    "<xs:element sql:field=\"id\" name=\"id\" type=\"xs:long\"/>",
    "<xs:element sql:field=\"symbol\" name=\"symbol\" type=\"xs:string\" minOccurs=\"0\"/>",
    "</xs:sequence></xs:complexType>",
    "</xs:schema>",
);

/// Three rows: `AAPL`, a null symbol, `MSFT`.
const THREE_ROWS: &str = concat!(
    "<row><id>1</id><symbol>AAPL</symbol></row>",
    "<row><id>2</id></row>",
    "<row><id>3</id><symbol>MSFT</symbol></row>",
);

/// A bare rowset `root` stating [`STATED_SCHEMA`] and holding `rows`.
fn stated(rows: &str) -> String {
    format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\" \
         xmlns:xs=\"http://www.w3.org/2001/XMLSchema\">{STATED_SCHEMA}{rows}</root>"
    )
}

/// Every batch a reader yields.
fn batches(reader: BatchReader) -> Vec<RecordBatch> {
    reader.map(|batch| batch.unwrap()).collect()
}

/// The column names of a reader's schema.
fn names(reader: &BatchReader) -> Vec<String> {
    reader
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect()
}

/// The `id` column of every batch a reader yields, the first column.
fn ids(reader: BatchReader) -> Vec<i64> {
    batches(reader)
        .iter()
        .flat_map(|batch| {
            batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect()
}

/// The first column of every batch a reader yields, read as text.
fn texts(reader: BatchReader) -> Vec<Option<String>> {
    batches(reader)
        .iter()
        .flat_map(|batch| {
            let column = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .clone();
            (0..column.len())
                .map(|index| (!column.is_null(index)).then(|| column.value(index).to_owned()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// `rows` of `(id, symbol)` as one batch under [`schema`].
fn batch_of(rows: &[(i64, Option<&str>)]) -> RecordBatch {
    RecordBatch::try_new(
        schema().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(
                rows.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|(_, symbol)| *symbol).collect::<Vec<_>>(),
            )),
        ],
    )
    .unwrap()
}

/// The rowset with `symbol` declared as a nullable `int64`.
fn numeric_symbol() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Int64.nullable_field("symbol"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

/// The `symbol` column of every batch a reader yields, read as `int64`.
fn numbers(reader: BatchReader) -> Vec<Option<i64>> {
    batches(reader)
        .iter()
        .flat_map(|batch| {
            let column = batch
                .column(1)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .clone();
            (0..column.len())
                .map(|index| (!column.is_null(index)).then(|| column.value(index)))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn the_filter_the_selector_and_the_bounds_shape_an_xmla_read() {
    let read = |options: XmlaOptions| {
        holding(&stated(THREE_ROWS))
            .read_arrow_reader(&RecordOptions::from(options))
            .unwrap()
    };

    // The `where` keeps the rows it answers true for.
    let kept = read(XmlaOptions::new().with_filter("id > 1").unwrap());
    assert_eq!(names(&kept), ["id", "symbol"]);
    assert_eq!(ids(kept), [2, 3]);

    // The `select` publishes the columns it names, in its order, and a
    // `where` may read a column the `select` does not publish.
    let published = read(
        XmlaOptions::new()
            .with_select("symbol")
            .unwrap()
            .with_filter("id > 1")
            .unwrap(),
    );
    assert_eq!(names(&published), ["symbol"]);
    assert_eq!(texts(published), [None, Some("MSFT".to_owned())]);

    // The row bound counts result rows: the first one the `where` kept.
    let bounded = read(
        XmlaOptions::new()
            .with_filter("id > 1")
            .unwrap()
            .with_max_row_size(1),
    );
    assert_eq!(ids(bounded), [2]);

    // A zero bound reads the schema and no batch.
    let none = read(XmlaOptions::new().with_max_row_size(0));
    assert_eq!(names(&none), ["id", "symbol"]);
    assert!(batches(none).is_empty());

    // A non-zero byte bound smaller than one row still yields one row.
    assert_eq!(ids(read(XmlaOptions::new().with_max_byte_size(1))), [1]);
    assert!(batches(read(XmlaOptions::new().with_max_byte_size(0))).is_empty());
}

#[test]
fn the_filter_the_selector_and_the_bounds_shape_an_xmla_write() {
    let three = || batch_of(&[(1, Some("AAPL")), (2, None), (3, Some("MSFT"))]);
    let write = |options: XmlaOptions| {
        let mut handle = handle();
        handle
            .overwrite_arrow_batch(three(), &RecordOptions::from(options))
            .unwrap();
        handle
    };
    let default = RecordOptions::from(XmlaOptions::new());

    let kept = write(XmlaOptions::new().with_filter("id > 1").unwrap());
    assert_eq!(ids(kept.read_arrow_reader(&default).unwrap()), [2, 3]);

    // The `select` decides which columns the written schema declares.
    let published = write(XmlaOptions::new().with_select("id").unwrap());
    let document = String::from_utf8(published.as_slice().to_vec()).unwrap();
    assert!(document.contains("name=\"id\""), "{document}");
    assert!(!document.contains("symbol"), "{document}");
    let field = published.read_arrow_field(&default).unwrap();
    assert_eq!(
        field.fields().iter().map(Field::name).collect::<Vec<_>>(),
        ["id"]
    );

    let bounded = write(XmlaOptions::new().with_max_row_size(2));
    let document = String::from_utf8(bounded.as_slice().to_vec()).unwrap();
    assert_eq!(document.matches("<row>").count(), 2, "{document}");
    assert_eq!(ids(bounded.read_arrow_reader(&default).unwrap()), [1, 2]);

    // A zero bound admits no row, and the schema is still written.
    let none = write(XmlaOptions::new().with_max_row_size(0));
    let document = String::from_utf8(none.as_slice().to_vec()).unwrap();
    assert_eq!(document.matches("<row>").count(), 0, "{document}");
    assert!(document.contains("<xsd:schema"), "{document}");
}

#[test]
fn a_cell_the_declared_field_cannot_cast_is_refused_unless_safe() {
    // The document states `symbol` as text; the declared field reads it as
    // a number, which `AAPL` is not and `42` is.
    let document = stated(
        "<row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id><symbol>42</symbol></row>",
    );
    let strict = RecordOptions::from(XmlaOptions::new().with_field(numeric_symbol()));
    // The document is parsed whole, so the refusal comes before a batch.
    let refused = holding(&document).read_arrow_reader(&strict).map(|_| ());
    assert!(
        refused.is_err(),
        "an unsafe cast of `AAPL` to int64 is refused"
    );
    // The refusal names the row and the column it could not read.
    let message = refused.unwrap_err().to_string();
    assert!(message.contains("$[0]"), "{message}");
    assert!(message.contains("symbol"), "{message}");
    assert!(message.contains("expected int64"), "{message}");

    // `safe` nulls the value it cannot convert, as a cast under it does in
    // every other encoding.
    let safe = RecordOptions::from(
        XmlaOptions::new()
            .with_field(numeric_symbol())
            .with_safe(true),
    );
    let read = holding(&document).read_arrow_reader(&safe).unwrap();
    assert_eq!(numbers(read), [None, Some(42)]);
}

#[test]
fn a_write_the_declared_field_cannot_cast_is_refused_unless_safe() {
    let rows = || batch_of(&[(1, Some("AAPL")), (2, Some("42"))]);

    let mut refused = handle();
    let strict = RecordOptions::from(XmlaOptions::new().with_field(numeric_symbol()));
    let message = refused
        .overwrite_arrow_batch(rows(), &strict)
        .unwrap_err()
        .to_string();
    assert!(message.contains("symbol"), "{message}");
    assert!(message.contains("'AAPL'"), "{message}");
    assert!(message.contains("Int64"), "{message}");
    // A refused write leaves the handle as it was.
    assert!(refused.as_slice().is_empty());

    let mut written = handle();
    let safe = RecordOptions::from(
        XmlaOptions::new()
            .with_field(numeric_symbol())
            .with_safe(true),
    );
    written.overwrite_arrow_batch(rows(), &safe).unwrap();
    let document = String::from_utf8(written.as_slice().to_vec()).unwrap();
    assert!(!document.contains("AAPL"), "{document}");
    assert!(document.contains("<symbol>42</symbol>"), "{document}");
    assert_eq!(
        numbers(written.read_arrow_reader(&strict).unwrap()),
        [None, Some(42)]
    );
}

#[test]
fn the_level_reaches_the_content_coding_and_nothing_else() {
    // With no content coding the level changes no byte of the document.
    let plain = written(XmlaOptions::new());
    for level in [Level::NONE, Level::FAST, Level::BEST, Level::new(3)] {
        assert_eq!(written(XmlaOptions::new().with_level(level)), plain);
    }

    // With one, the document is compressed at exactly the level stated.
    let coded =
        || Buffer::new().with_media_type(Url::from_str("file:///a.xmla.gz").unwrap().media_type());
    let mut encodings = Vec::new();
    for level in [Level::NONE, Level::FAST, Level::DEFAULT, Level::BEST] {
        let mut handle = coded();
        let options = RecordOptions::from(XmlaOptions::new().with_level(level));
        handle.overwrite_arrow_batch(batch(), &options).unwrap();
        let encoded = handle.as_slice().to_vec();
        assert_eq!(&encoded[..2], &[0x1F, 0x8B]);
        assert_eq!(
            encoded,
            Codec::Gzip
                .dump_with_level(plain.as_bytes(), level)
                .unwrap(),
            "{level:?}"
        );
        assert_eq!(
            rows(handle.read_arrow_reader(&options).unwrap()),
            expected()
        );
        encodings.push(encoded);
    }
    // Storing and compressing hardest are different bytes.
    assert_ne!(encodings[0], encodings[3]);
}

#[test]
fn a_zero_commit_cadence_leaves_the_stored_document_as_it_was() {
    let mut stored = holding(&stated(THREE_ROWS));
    let before = stored.as_slice().to_vec();
    let zero = RecordOptions::from(XmlaOptions::new().with_commit_row_size(0));
    let message = stored
        .overwrite_arrow_batch(batch(), &zero)
        .unwrap_err()
        .to_string();
    assert!(message.contains("$.commit_row_size"), "{message}");
    assert_eq!(stored.as_slice(), before.as_slice());
}

#[test]
fn a_data_content_with_a_slicer_writes_the_rows_without_the_schema() {
    // A rowset has no default slicer, so both slicer contents are `Data`.
    for envelope in [true, false] {
        let data = {
            let mut options = XmlaOptions::new().with_content(Content::Data);
            options.envelope = envelope;
            written(options)
        };
        for content in [
            Content::DataOmitDefaultSlicer,
            Content::DataIncludeDefaultSlicer,
        ] {
            assert!(content.has_data() && !content.has_schema());
            let mut options = XmlaOptions::new().with_content(content);
            options.envelope = envelope;
            let document = written(options);
            assert!(!document.contains("<xsd:schema"), "{content}: {document}");
            assert_eq!(
                document.matches("<row>").count(),
                2,
                "{content}: {document}"
            );
            assert_eq!(document, data, "{content}");
            let read = RecordOptions::from(XmlaOptions::new().with_field(schema()));
            assert_eq!(
                rows(holding(&document).read_arrow_reader(&read).unwrap()),
                expected()
            );
        }
    }
}

#[test]
fn a_document_without_rows_reads_as_zero_rows_under_its_schema() {
    for envelope in [true, false] {
        let mut options = XmlaOptions::new().with_content(Content::Schema);
        options.envelope = envelope;
        let document = written(options);
        let stored = holding(&document);
        let default = RecordOptions::from(XmlaOptions::new());
        let field = stored.read_arrow_field(&default).unwrap();
        assert_eq!(field.dtype(), schema().dtype());
        let reader = stored.read_arrow_reader(&default).unwrap();
        assert_eq!(names(&reader), ["id", "symbol"]);
        assert_eq!(rows(reader), (vec![], vec![]));
        assert_eq!(stored.row_size().unwrap(), 0);
    }
}

// The value.

/// The default options with exactly one setting changed, one per setting.
fn one_setting_each() -> Vec<(&'static str, XmlaOptions)> {
    let default = XmlaOptions::new;
    vec![
        (
            "name",
            XmlaOptions {
                name: "trade".into(),
                ..default()
            },
        ),
        (
            "field",
            XmlaOptions {
                field: Some(schema()),
                ..default()
            },
        ),
        (
            "filter",
            XmlaOptions {
                filter: "id > 1".parse().unwrap(),
                ..default()
            },
        ),
        (
            "select",
            XmlaOptions {
                select: "id".parse().unwrap(),
                ..default()
            },
        ),
        (
            "merge_by",
            XmlaOptions {
                merge_by: "id".parse().unwrap(),
                ..default()
            },
        ),
        (
            "safe",
            XmlaOptions {
                safe: true,
                ..default()
            },
        ),
        (
            "batch_byte_size",
            XmlaOptions {
                batch_byte_size: Some(1),
                ..default()
            },
        ),
        (
            "batch_row_size",
            XmlaOptions {
                batch_row_size: Some(1),
                ..default()
            },
        ),
        (
            "max_row_size",
            XmlaOptions {
                max_row_size: Some(1),
                ..default()
            },
        ),
        (
            "max_byte_size",
            XmlaOptions {
                max_byte_size: Some(1),
                ..default()
            },
        ),
        (
            "commit_row_size",
            XmlaOptions {
                commit_row_size: Some(1),
                ..default()
            },
        ),
        (
            "level",
            XmlaOptions {
                level: Level::BEST,
                ..default()
            },
        ),
        (
            "envelope",
            XmlaOptions {
                envelope: false,
                ..default()
            },
        ),
        (
            "method",
            XmlaOptions {
                method: Method::Discover,
                ..default()
            },
        ),
        (
            "content",
            XmlaOptions {
                content: Content::Data,
                ..default()
            },
        ),
    ]
}

#[test]
fn every_setting_is_part_of_the_value_its_hashes_and_its_order() {
    let mut values = vec![("default", XmlaOptions::new())];
    values.extend(one_setting_each());
    assert_eq!(values.len(), 16, "the default and one per public setting");
    let hashes: HashSet<u64> = values.iter().map(|(_, value)| std_hash(value)).collect();
    assert_eq!(hashes.len(), values.len());
    let stable: HashSet<u64> = values
        .iter()
        .map(|(_, value)| RecordOptions::from(value.clone()).stable_hash())
        .collect();
    assert_eq!(stable.len(), values.len());
    for (index, (left_name, left)) in values.iter().enumerate() {
        for (other, (right_name, right)) in values.iter().enumerate() {
            let same = index == other;
            assert_eq!(left == right, same, "{left_name} against {right_name}");
            assert_eq!(
                left.cmp(right) == Ordering::Equal,
                same,
                "{left_name} against {right_name}"
            );
        }
    }
    // A clone is the same value with the same hashes.
    for (name, value) in &values {
        let clone = value.clone();
        assert_eq!(&clone, value, "{name}");
        assert_eq!(std_hash(&clone), std_hash(value), "{name}");
    }
}

#[test]
fn the_order_is_total_and_agrees_with_equality() {
    let mut values = vec![XmlaOptions::new(), XmlaOptions::new()];
    values.extend(one_setting_each().into_iter().map(|(_, value)| value));
    for left in &values {
        for right in &values {
            let order = left.cmp(right);
            assert_eq!(left.partial_cmp(right), Some(order));
            assert_eq!(right.cmp(left), order.reverse());
            assert_eq!(order == Ordering::Equal, left == right);
            for third in &values {
                if left <= right && right <= third {
                    assert!(left <= third, "{left:?} <= {right:?} <= {third:?}");
                }
            }
        }
    }
    // The settings compare in the order the struct declares them, so the
    // root name decides before the envelope does.
    assert!(XmlaOptions::new().without_envelope() < XmlaOptions::new());
    assert!(XmlaOptions::new().with_method(Method::Discover) < XmlaOptions::new());
    assert!(XmlaOptions::new().with_content(Content::None) < XmlaOptions::new());
    assert!(
        XmlaOptions::new().with_content(Content::DataIncludeDefaultSlicer) > XmlaOptions::new()
    );
    assert!(XmlaOptions::new().with_name("s").without_envelope() > XmlaOptions::new());
    let mut sorted = values.clone();
    sorted.sort();
    sorted.reverse();
    sorted.sort();
    assert!(sorted.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[test]
fn the_debug_form_names_the_type_and_the_xmla_settings() {
    let debug = format!("{:?}", XmlaOptions::new());
    assert!(debug.starts_with("XmlaOptions {"), "{debug}");
    assert!(debug.contains("name: \"row\""), "{debug}");
    assert!(debug.contains("envelope: true"), "{debug}");
    assert!(debug.contains("method: Execute"), "{debug}");
    assert!(debug.contains("content: SchemaData"), "{debug}");
    let bare = format!(
        "{:?}",
        XmlaOptions::new()
            .without_envelope()
            .with_method(Method::Discover)
            .with_content(Content::None)
    );
    assert!(bare.contains("envelope: false"), "{bare}");
    assert!(bare.contains("method: Discover"), "{bare}");
    assert!(bare.contains("content: None"), "{bare}");
}

/// The three XMLA builders chained where only a `const fn` may be called.
const fn bare_discover_data(options: XmlaOptions) -> XmlaOptions {
    options
        .without_envelope()
        .with_method(Method::Discover)
        .with_content(Content::Data)
}

#[test]
fn the_xmla_builders_are_const() {
    let options = bare_discover_data(XmlaOptions::new());
    assert!(!options.envelope);
    assert_eq!(options.method, Method::Discover);
    assert_eq!(options.content, Content::Data);
}

// The plan's row bound.

#[test]
fn a_plan_limit_is_the_row_bound_and_the_row_bound_is_the_plans_limit() {
    let limited = XmlaOptions::new().with_plan("select id limit 5").unwrap();
    assert_eq!(limited.max_row_size(), Some(5));
    assert_eq!(limited.plan().row_limit(), Some(5));
    assert_eq!(limited.plan().to_string(), "select id limit 5");
    assert_eq!(
        XmlaOptions::new().with_plan(limited.plan()).unwrap(),
        limited
    );

    let bounded = XmlaOptions::new().with_max_row_size(3);
    assert_eq!(bounded.plan().row_limit(), Some(3));
    assert_eq!(bounded.plan().to_string(), "select * limit 3");

    // A plan's limit replaces a bound stated beside it; a plan that spells
    // none leaves that bound, as a write target's `max_row_size` property
    // stands beside the plan it runs. The byte bound is never a section.
    let beside = XmlaOptions::new()
        .with_max_row_size(7)
        .with_max_byte_size(9);
    assert_eq!(
        beside
            .clone()
            .with_plan("select id limit 2")
            .unwrap()
            .max_row_size(),
        Some(2)
    );
    let unspelled = beside.with_plan("select id").unwrap();
    assert_eq!(unspelled.max_row_size(), Some(7));
    assert_eq!(unspelled.max_byte_size(), Some(9));

    let zero = XmlaOptions::new().with_plan("limit 0").unwrap();
    assert_eq!(zero.max_row_size(), Some(0));
    assert!(zero.write_limit_is_zero());
}

#[test]
fn a_plan_section_the_options_cannot_hold_is_never_silently_dropped() {
    // `set_plan` names what it does not read - the plan's targets and its
    // source - and an `offset` or an `order by` is neither: taking such a
    // plan is either refused naming the section, or the options spell it
    // back. Dropping it answers other rows than the plan asked for.
    let mut dropped = Vec::new();
    for (text, section) in [
        ("select id limit 2 offset 1", "offset"),
        ("select id order by id desc", "order by"),
    ] {
        match XmlaOptions::new().with_plan(text) {
            Ok(options) => {
                let spelled = options.plan().to_string();
                if spelled != text {
                    dropped.push(format!("{section}: `{text}` spelled back as `{spelled}`"));
                }
            }
            Err(error) => assert!(error.to_string().contains(section), "{error}"),
        }
    }
    assert!(dropped.is_empty(), "silently dropped: {dropped:?}");
}

#[test]
fn a_plan_reading_an_xmla_file_keeps_its_offset() {
    // `Plan::execute` pushes its read sections into the file's XMLA options;
    // the offset must survive that, or the plan answers other rows.
    let mut folder = yggdryl::local::LocalFolder::temporary()
        .unwrap()
        .path()
        .unwrap();
    folder.push(format!(
        "yggdryl-xmla-options-offset-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("trades.xmla");
    std::fs::write(&path, stated(THREE_ROWS)).unwrap();
    let url = Url::from_path(&path).unwrap();
    let plan: Plan = format!("select id from '{url}' limit 1 offset 1")
        .parse()
        .unwrap();
    let read = plan.execute();
    let _ = std::fs::remove_dir_all(&folder);
    assert_eq!(ids(read.unwrap()), [2]);
}

#[test]
fn a_where_clause_reads_the_decoded_text_of_the_cells() {
    // Escaped and non-ASCII text is compared as the characters it spells; a
    // cell marked `xsi:nil` and an absent one are null; an emptied one is
    // the empty text.
    let document = format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\" \
         xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" \
         xmlns:i=\"http://www.w3.org/2001/XMLSchema-instance\">{STATED_SCHEMA}\
         <row><id>1</id><symbol>Z&#xFC;rich &amp; Co</symbol></row>\
         <row><id>2</id><symbol i:nil=\"true\"/></row>\
         <row><id>3</id></row>\
         <row><id>4</id><symbol></symbol></row>\
         <row><id>5</id><symbol>\u{1F4C8} &lt;up&gt;</symbol></row>\
         </root>"
    );
    let read = |filter: &str| {
        ids(holding(&document)
            .read_arrow_reader(&RecordOptions::from(
                XmlaOptions::new().with_filter(filter).unwrap(),
            ))
            .unwrap())
    };
    assert_eq!(read("symbol = 'Zürich & Co'"), [1]);
    assert_eq!(read("symbol is null"), [2, 3]);
    assert_eq!(read("symbol = ''"), [4]);
    assert_eq!(read("symbol = '\u{1F4C8} <up>'"), [5]);
    assert_eq!(read("symbol is not null"), [1, 4, 5]);
}
