//! `rust/src/xmla/options.rs`: the settings an XMLA rowset document is read
//! and written with - the shared record settings, the envelope, the method
//! and the content - and the one `RecordOptions` variant they are.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};

use yggdryl::arrow::BatchReader;
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::xml::soap::ENVELOPE_NAMESPACE;
use yggdryl::xmla::{Content, Method, NAMESPACE, ROWSET_NAMESPACE, XmlaOptions};
use yggdryl::{
    DataType, Field, Filter, IOBase, IOMedia, IOMode, Level, MediaType, MimeType, Plan, Scalar,
    Selector, StructType, Timezone, Url,
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
