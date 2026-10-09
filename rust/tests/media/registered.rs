//! `rust/src/media/registered.rs`: an encoding another crate implements,
//! registered under its MIME type, reached by every record door the core's
//! own encodings are - the options by MIME type, the composed wrapper, the
//! schema, the row count, the rows and a write - and refused by name where
//! nothing is registered.

use std::str::FromStr;
use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, RecordBatchIterator};
use yggdryl::arrow::BatchReader;
use yggdryl::holder::{Buffer, Holder};
use yggdryl::media::doors::{overwrite_arrow_reader_default_with_field, own_options};
use yggdryl::media::{
    IORecordOptions, Media, RecordOptions, RegisteredEncoding, RegisteredMedia, RegisteredOptions,
    register, registered, registered_mime_types,
};
use yggdryl::{
    DataType, Field, IOBase, IOMedia, MediaType, MimeType, Result, Selector, StructType,
};

/// The MIME type the toy encoding below registers: a count per line.
fn counts_mime() -> MimeType {
    MimeType::from_str("text/x-yggdryl-counts").expect("a MIME type")
}

/// A MIME type nothing in this process ever registers.
fn never_mime() -> MimeType {
    MimeType::from_str("text/x-yggdryl-never").expect("a MIME type")
}

/// The one column every counts document holds.
fn counts_field() -> Field {
    DataType::from(
        StructType::from_fields([DataType::Int64.required_field("n")]).expect("a valid root"),
    )
    .required_field("row")
}

/// The lines of a counts document, each one number.
fn lines(handle: &dyn IOBase) -> Result<Vec<String>> {
    let bytes = handle.read_all_bytes()?;
    let text = String::from_utf8(bytes).expect("counts are ASCII");
    Ok(text.lines().map(str::to_owned).collect())
}

/// One `int64` per line: an encoding small enough to stand in for one
/// another crate implements.
#[derive(Debug)]
struct Counts;

impl RegisteredEncoding for Counts {
    fn mime_type(&self) -> MimeType {
        counts_mime()
    }

    fn options(&self) -> RegisteredOptions {
        RegisteredOptions::new(counts_mime()).with_property("separator", "newline")
    }

    fn read_field(&self, _handle: &dyn IOBase, _options: &RegisteredOptions) -> Result<Field> {
        Ok(counts_field())
    }

    fn row_size(&self, handle: &dyn IOBase, _options: &RegisteredOptions) -> Result<u64> {
        Ok(lines(handle)?.len() as u64)
    }

    fn read_batch_reader(
        &self,
        handle: &dyn IOBase,
        _declared: Option<&Field>,
        _options: &RegisteredOptions,
    ) -> yggdryl::arrow::Result<BatchReader> {
        let values = lines(handle)?
            .iter()
            .map(|line| line.parse::<i64>())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(yggdryl::arrow::Error::external)?;
        let schema = counts_field().into_arrow_schema()?;
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(Int64Array::from(values))],
        )
        .map_err(yggdryl::arrow::from_reader_error)?;
        Ok(Box::new(RecordBatchIterator::new(
            std::iter::once(Ok(batch)),
            schema,
        )))
    }

    fn overwrite_arrow_reader(
        &self,
        handle: &mut dyn IOBase,
        batches: BatchReader,
        _options: &RegisteredOptions,
    ) -> Result<()> {
        let mut text = String::new();
        for batch in batches {
            let batch = batch.map_err(yggdryl::arrow::from_reader_error)?;
            let column = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("an int64 column");
            for value in column.values() {
                text.push_str(&value.to_string());
                text.push('\n');
            }
        }
        handle.write_all_bytes(text.as_bytes())
    }

    fn stated_field(
        &self,
        handle: &dyn IOBase,
        _options: &RegisteredOptions,
    ) -> Result<Option<Field>> {
        Ok((handle.size() > 0).then(counts_field))
    }

    fn open(&self, handle: Holder) -> Box<dyn RegisteredMedia> {
        Box::new(CountsMedia {
            handle,
            options: self.options(),
        })
    }
}

/// The wrapper the toy encoding retains over a handle.
#[derive(Debug)]
struct CountsMedia {
    handle: Holder,
    options: RegisteredOptions,
}

impl IOMedia for CountsMedia {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    fn record_options(&self) -> Result<RecordOptions> {
        Ok(RecordOptions::Registered(self.options.clone()))
    }

    fn overwrite_serie(
        &mut self,
        value: yggdryl::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<yggdryl::IOResult> {
        let options = own_options(self, options)?;
        let batches = yggdryl::StreamChunkedSerie::from_serie(value)?.into_arrow_reader();
        overwrite_arrow_reader_default_with_field(self, batches, &options).map(|(_, result)| result)
    }
}

impl IOBase for CountsMedia {
    yggdryl::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, read_tail_bytes,
        read_digest, read_range_digest, write_all_bytes, create_bytes, append_bytes,
        applied_codec, pstream_bytes, pwrite, size, capacity, reserve, truncate, uri, url,
        bound_location, mtime, media_type, set_media_type, flush, open, opened, close, parent,
        child_by_path, ls, kind, clear, remove, is_container, is_thread_bound, is_io);

    fn is_tabular(&self) -> bool {
        true
    }

    fn is_atomic(&self) -> bool {
        false
    }
}

impl RegisteredMedia for CountsMedia {
    fn encoding(&self) -> MimeType {
        counts_mime()
    }

    fn handle(&self) -> &Holder {
        &self.handle
    }

    fn into_handle(self: Box<Self>) -> Holder {
        self.handle
    }

    fn with_field(mut self: Box<Self>, field: Field) -> Box<dyn RegisteredMedia> {
        self.options.set_field(field);
        self
    }
}

/// An empty buffer declaring the toy encoding's media type.
fn counts_buffer() -> Buffer {
    Buffer::new().with_media_type(MediaType::new(counts_mime()))
}

/// Three counts as one batch.
fn counts_batch() -> RecordBatch {
    RecordBatch::try_new(
        counts_field().into_arrow_schema().expect("an Arrow schema"),
        vec![Arc::new(Int64Array::from(vec![7, 8, 9]))],
    )
    .expect("a batch")
}

#[test]
fn nothing_registered_is_an_encoding_this_build_does_not_implement() {
    let never = never_mime();
    assert!(registered(&never).is_none());
    assert!(!registered_mime_types().contains(&never));

    let refused = RecordOptions::for_mime_type(&never)
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("a record encoding this build implements"),
        "{refused}"
    );
    assert!(refused.contains("text/x-yggdryl-never"), "{refused}");

    let refused = Media::open_as(Holder::buffer(counts_buffer()), &never)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("text/x-yggdryl-never"), "{refused}");

    let refused = RegisteredOptions::new(never).encoding().unwrap_err();
    assert!(
        matches!(&refused, yggdryl::Error::InvalidRecord { path, .. } if path == "$.encoding"),
        "{refused}"
    );
    assert!(refused.to_string().contains("text/x-yggdryl-never"));
}

#[test]
fn a_registered_encoding_answers_its_options_by_mime_type() {
    register(Arc::new(Counts));
    assert!(registered(&counts_mime()).is_some());
    assert!(registered_mime_types().contains(&counts_mime()));

    let options = RecordOptions::for_mime_type(&counts_mime()).expect("registered options");
    let RecordOptions::Registered(inner) = &options else {
        panic!("expected the registered variant, got {options:?}");
    };
    assert_eq!(inner.mime_type(), &counts_mime());
    assert_eq!(inner.properties().get("separator"), Some("newline"));
    assert_eq!(options.mime_type(), counts_mime());
    assert_eq!(
        RecordOptions::for_media_type(&MediaType::new(counts_mime())).expect("the same"),
        options
    );
    // The encoding the options name is the registered one.
    assert_eq!(
        inner.encoding().expect("registered").mime_type(),
        counts_mime()
    );
    // The refusal of an unknown MIME type now lists it among what this build
    // implements.
    let refused = RecordOptions::for_mime_type(&never_mime())
        .unwrap_err()
        .to_string();
    assert!(refused.contains("text/x-yggdryl-counts"), "{refused}");
}

#[test]
fn the_registered_options_carry_every_shared_setting_and_their_own() {
    let field = counts_field();
    let mut options = RegisteredOptions::new(counts_mime())
        .with_field(field.clone())
        .with_filter("n > 1")
        .expect("a filter")
        .with_select("n")
        .expect("a selection")
        .with_safe(true)
        .with_batch_row_size(4)
        .with_max_row_size(3)
        .with_row_offset(1)
        .with_max_byte_size(2)
        .with_commit_batch_num(1)
        .with_num_threads(2)
        .with_property("separator", "comma");
    assert_eq!(options.field(), Some(field.clone()));
    assert_eq!(options.name(), "row");
    assert_eq!(options.filter().to_string(), "n > 1");
    assert_eq!(
        options.select(),
        &Selector::from_str("n").expect("a selector")
    );
    assert!(options.safe());
    assert_eq!(options.batch_row_size(), Some(4));
    assert_eq!(options.max_row_size(), Some(3));
    assert_eq!(options.row_offset(), Some(1));
    assert_eq!(options.max_byte_size(), Some(2));
    assert_eq!(options.commit_batch_num(), Some(1));
    assert_eq!(options.num_threads(), Some(2));
    assert_eq!(options.properties().get("separator"), Some("comma"));
    options.properties_mut().set("separator", "tab");
    assert_eq!(options.properties().get("separator"), Some("tab"));

    // Plain data: equal to its clone, ordered and hashed with its properties.
    let same = options.clone();
    assert_eq!(options, same);
    let other = options.clone().with_property("separator", "comma");
    assert_ne!(options, other);
    assert_ne!(
        RecordOptions::from(options.clone()).stable_hash(),
        RecordOptions::from(other).stable_hash()
    );
    assert!(RegisteredOptions::new(counts_mime()) < RegisteredOptions::new(never_mime()));
    let record = RecordOptions::from(options.clone());
    assert_eq!(record.mime_type(), counts_mime());
    assert_eq!(record.field(), Some(field));
    assert_eq!(record.max_row_size(), Some(3));
    // The variant's own doors answer for every other encoding's setting.
    assert_eq!(record.timezone(), None);
    assert_eq!(record.avro_block_codec(), None);
}

#[cfg(feature = "parquet")]
#[test]
fn the_registered_options_refuse_another_encodings_settings() {
    let options = RecordOptions::from(RegisteredOptions::new(counts_mime()));
    assert_eq!(options.parquet_compression_name(), None);
    assert_eq!(options.parquet_max_row_group_size(), None);
    assert_eq!(options.parquet_key_value_metadata(), None);
    let message = options
        .clone()
        .set_parquet_max_row_group_size(10)
        .unwrap_err()
        .to_string();
    assert!(message.contains("expected Parquet options"), "{message}");
    assert!(
        message.contains("got text/x-yggdryl-counts options"),
        "{message}"
    );
}

#[test]
fn a_handle_named_by_the_mime_type_composes_the_wrapper_and_round_trips_rows() {
    register(Arc::new(Counts));
    let mut media = Media::open(Holder::buffer(counts_buffer())).expect("a media");
    assert!(matches!(&media, Media::Registered(inner) if inner.encoding() == counts_mime()));
    let options = media.record_options().expect("the options");
    assert_eq!(options.mime_type(), counts_mime());

    let batch = counts_batch();
    media
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .expect("written");
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        counts_field()
    );
    assert_eq!(media.row_size().expect("the rows"), 3);
    let rows: usize = media
        .read_arrow_reader(&options)
        .expect("a reader")
        .map(|batch| batch.expect("a batch").num_rows())
        .sum();
    assert_eq!(rows, 3);
    assert_eq!(
        media.into_handle().read_all_bytes().expect("the bytes"),
        b"7\n8\n9\n"
    );

    // A declared field travels on the wrapper's options.
    let declared =
        Media::Registered(Counts.open(Holder::buffer(counts_buffer()))).with_field(counts_field());
    assert_eq!(
        declared.record_options().expect("options").field(),
        Some(counts_field())
    );
}

#[test]
fn a_bare_handle_of_the_mime_type_reads_and_writes_through_the_generic_doors() {
    register(Arc::new(Counts));
    let mut handle = counts_buffer();
    let options = handle.record_options().expect("the options");
    assert!(matches!(options, RecordOptions::Registered(_)));
    let batch = counts_batch();
    let written = handle
        .overwrite_arrow_batch(batch, &options)
        .expect("written");
    assert_eq!(written.written_rows, 3);
    assert_eq!(handle.read_all_bytes().expect("the bytes"), b"7\n8\n9\n");
    assert_eq!(handle.row_size().expect("the rows"), 3);
    assert_eq!(handle.column_size().expect("the columns"), 1);
    assert_eq!(
        handle.read_arrow_field(&options).expect("the field"),
        counts_field()
    );
    // An append completes onto the stored shape the encoding states.
    let appended = handle
        .append_arrow_batch(counts_batch(), &options)
        .expect("appended");
    assert_eq!(appended.written_rows, 3);
    assert_eq!(handle.row_size().expect("the rows"), 6);
    let rows: usize = handle
        .read_arrow_reader(&options)
        .expect("a reader")
        .map(|batch| batch.expect("a batch").num_rows())
        .sum();
    assert_eq!(rows, 6);
}

#[test]
fn registering_again_replaces_the_encoding_held_under_the_mime_type() {
    register(Arc::new(Counts));
    register(Arc::new(Counts));
    let held = registered_mime_types()
        .into_iter()
        .filter(|mime| *mime == counts_mime())
        .count();
    assert_eq!(held, 1);
}
