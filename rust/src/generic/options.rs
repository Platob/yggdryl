//! The settings a record read or write takes, shared across encodings.
//!
//! Reading rows out of an Arrow IPC stream and reading them out of a Parquet
//! file need the same handful of answers - what schema, what to call an
//! inferred root, how strict a cast may be, how many rows per batch, how hard
//! to compress, and what makes two rows the same row - and each encoding then
//! adds its own. [`IORecordOptions`] is that shared surface, each encoding
//! stores those settings as its own flat fields, and [`RecordOptions`] is the
//! enum naming every encoding's options.
//!
//! An encoding is never guessed: [`RecordOptions::for_media_type`] derives it
//! from the handle's media type, which is what [`crate::io::IOBase`]'s record
//! methods use when a caller does not supply options of their own.
//!
//! ```
//! use yggdryl::generic::{IORecordOptions, RecordOptions};
//! use yggdryl::{DataType, Url};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
//!     .required_field("row");
//!
//! // Arrow IPC is the encoding every build implements, so this example reads
//! // the same in a default build as in one with the `parquet` feature on.
//! let options = RecordOptions::for_media_type(&Url::from_str("file:///t.arrows")?.media_type())?
//!     .with_schema(schema.clone())
//!     .with_batch_size(1024);
//!
//! assert_eq!(options.schema(), Some(&schema));
//! assert_eq!(options.batch_size(), Some(1024));
//! # Ok(())
//! # }
//! ```

use smol_str::SmolStr;

use crate::Level;
use crate::ipc::IpcOptions;
use crate::{Error, Field, MediaType, MimeType, Result};

/// The read and write settings shared by every record encoding.
///
/// Each encoding stores these as its own fields - there is no shared settings
/// struct to thread through - and the builders here are what every caller uses,
/// so the encodings cannot drift apart in what a shared setting means.
pub trait IORecordOptions: Sized {
    /// Borrow the declared canonical schema, if any.
    fn schema(&self) -> Option<&Field>;

    /// Declare the canonical schema.
    fn set_schema(&mut self, schema: Field);

    /// Borrow the root Field name used when a schema is inferred.
    fn root_name(&self) -> &str;

    /// Set the root Field name used when a schema is inferred.
    fn set_root_name(&mut self, root_name: SmolStr);

    /// Return whether a cast may null a value it cannot convert.
    fn safe(&self) -> bool;

    /// Set whether a cast may null a value it cannot convert.
    fn set_safe(&mut self, safe: bool);

    /// Return the row-per-batch bound, if any.
    fn batch_size(&self) -> Option<usize>;

    /// Set the row-per-batch bound.
    fn set_batch_size(&mut self, batch_size: Option<usize>);

    /// Return the compression level applied to a declared content coding.
    fn level(&self) -> Level;

    /// Set the compression level.
    fn set_level(&mut self, level: Level);

    /// Borrow the column names whose values form a write's match key.
    ///
    /// An empty list is an overwrite. A non-empty one names the columns a
    /// [`write`](crate::io::IOBase::write_arrow_batch_reader) matches incoming
    /// rows against: a row whose key is already stored updates it, and a row
    /// whose key is not appends.
    fn merge_by_names(&self) -> &[String];

    /// Set the column names whose values form a write's match key.
    fn set_merge_by_names(&mut self, merge_by_names: Vec<String>);

    /// Borrow the column names a read or write is narrowed to.
    ///
    /// An empty list selects everything. A non-empty one names the columns, in
    /// the order they are wanted: a read yields exactly those columns of the
    /// stored rows, and a write keeps exactly those columns of the incoming
    /// rows. Names match ASCII case-insensitively, the way every cast selects,
    /// and a name the rows do not have is an error rather than a null column,
    /// because a selection is a claim about what is there.
    fn select_by_names(&self) -> &[String];

    /// Set the column names a read or write is narrowed to.
    fn set_select_by_names(&mut self, select_by_names: Vec<String>);

    /// Borrow the partition equalities a read is pruned and filtered by.
    ///
    /// An empty list keeps every row. A non-empty one names
    /// `(column, value)` pairs, values spelled as
    /// [`partition_text`](crate::io::partition::partition_text) spells them:
    /// a folder read skips every leaf whose path names a different value -
    /// nothing under it is listed or decoded - and rows whose data carries
    /// the column are filtered to the named values, so path-partitioned and
    /// data-partitioned layouts answer the same question the same way.
    fn filter_partitions(&self) -> &[(String, String)];

    /// Set the partition equalities a read is pruned and filtered by.
    fn set_filter_partitions(&mut self, filter_partitions: Vec<(String, String)>);

    /// The predicate these options' partition equalities spell about a *path*.
    ///
    /// One expression, built from the pairs, asked of the holder rather than
    /// of the rows: `&holder.partition['year'] = '2024'`. This is what prunes
    /// a listing before anything is opened, and it is the same predicate type
    /// the rows are filtered with - the pairs are sugar over one
    /// representation, not a second filter.
    fn partition_filter(&self) -> crate::Expression {
        crate::Expression::all_holder_partitions_equal(
            self.filter_partitions()
                .iter()
                .map(|(column, value)| (column, value)),
        )
    }

    /// The predicate these options' partition equalities spell about *rows*.
    ///
    /// The same pairs, read through the schema's own datatypes, which is what
    /// makes `("price", "20")` an integer comparison on an `int32` column. A
    /// pair naming a column the schema does not declare is left out: the path
    /// answered for it already.
    fn partition_predicate(&self, schema: &crate::Field) -> crate::Expression {
        crate::Expression::all_partitions_equal(
            schema,
            self.filter_partitions()
                .iter()
                .map(|(column, value)| (column, value)),
        )
    }

    /// Borrow the declared schema, or say that one is required.
    ///
    /// # Errors
    ///
    /// Returns an error naming the builder that declares a schema.
    fn require_schema(&self) -> Result<&Field> {
        self.schema().ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static(
                "expected a declared schema to write records; call with_schema first",
            ),
        })
    }

    /// Return these options with a declared canonical schema.
    #[must_use]
    fn with_schema(mut self, schema: Field) -> Self {
        self.set_schema(schema);
        self
    }

    /// Return these options with a different inferred-root Field name.
    #[must_use]
    fn with_root_name(mut self, root_name: impl Into<SmolStr>) -> Self {
        self.set_root_name(root_name.into());
        self
    }

    /// Return these options with a different cast strictness.
    #[must_use]
    fn with_safe(mut self, safe: bool) -> Self {
        self.set_safe(safe);
        self
    }

    /// Return these options with a row-per-batch bound.
    #[must_use]
    fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.set_batch_size(Some(batch_size));
        self
    }

    /// Return these options with a different compression level.
    #[must_use]
    fn with_level(mut self, level: Level) -> Self {
        self.set_level(level);
        self
    }

    /// Return these options with a match key for a write.
    #[must_use]
    fn with_merge_by_names<I, S>(mut self, merge_by_names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.set_merge_by_names(merge_by_names.into_iter().map(Into::into).collect());
        self
    }

    /// Return these options narrowed to the named columns, for reads and writes.
    #[must_use]
    fn with_select_by_names<I, S>(mut self, select_by_names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.set_select_by_names(select_by_names.into_iter().map(Into::into).collect());
        self
    }

    /// Return these options pruned and filtered to the named partitions.
    #[must_use]
    fn with_filter_partitions<I, C, V>(mut self, filter_partitions: I) -> Self
    where
        I: IntoIterator<Item = (C, V)>,
        C: Into<String>,
        V: Into<String>,
    {
        self.set_filter_partitions(
            filter_partitions
                .into_iter()
                .map(|(column, value)| (column.into(), value.into()))
                .collect(),
        );
        self
    }

    /// Cast one batch the way these options say, completed by what is stored.
    ///
    /// This is the one definition of option-driven casting, in three layers
    /// applied in order: the declared [`schema`](Self::schema) says what the
    /// rows are meant to be, [`select_by_names`](Self::select_by_names)
    /// narrows and orders the columns, and `existing` - a holder's stored
    /// shape - is what the batch is finally cast onto, always safely, so a
    /// value that will not convert into a stored column becomes null rather
    /// than quietly redefining that column for every reader of the resource.
    /// Each absent layer costs nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when a cast cannot be planned or a selected name is
    /// not a column of the rows.
    fn cast_arrow_batch(
        &self,
        batch: arrow_array::RecordBatch,
        existing: Option<&Field>,
    ) -> Result<arrow_array::RecordBatch> {
        let safe = self.safe();
        let batch = match self.schema() {
            Some(declared) => crate::field::cast::cast_record_batch(declared, batch, safe)?,
            None => batch,
        };
        let root =
            crate::arrow::record_schema_from_arrow(self.root_name(), batch.schema().as_ref())?;
        let batch =
            match crate::arrow::selected_root(&root, self.select_by_names(), self.root_name())? {
                Some(target) => crate::field::cast::cast_record_batch(&target, batch, safe)?,
                None => batch,
            };
        match existing {
            Some(stored) => Ok(crate::field::cast::cast_record_batch(stored, batch, true)?),
            None => Ok(batch),
        }
    }

    /// Cast a whole reader as [`cast_arrow_batch`](Self::cast_arrow_batch)
    /// casts one batch, streaming - nothing is collected, each batch is cast
    /// as it is pulled, and a layer whose target already matches costs
    /// nothing at all.
    ///
    /// # Errors
    ///
    /// Returns an error when a cast cannot be planned or a selected name is
    /// not a column of the reader.
    fn cast_arrow_reader(
        &self,
        reader: crate::arrow::BatchReader,
        existing: Option<&Field>,
    ) -> Result<crate::arrow::BatchReader> {
        let safe = self.safe();
        let reader = match self.schema() {
            Some(declared) => crate::arrow::cast_reader(reader, declared, safe)?,
            None => reader,
        };
        let root =
            crate::arrow::record_schema_from_arrow(self.root_name(), reader.schema().as_ref())?;
        let reader =
            match crate::arrow::selected_root(&root, self.select_by_names(), self.root_name())? {
                Some(target) => crate::arrow::cast_reader(reader, &target, safe)?,
                None => reader,
            };
        match existing {
            Some(stored) => Ok(crate::arrow::cast_reader(reader, stored, true)?),
            None => Ok(reader),
        }
    }
}

/// Implement [`IORecordOptions`] over one struct's own fields.
///
/// Every encoding stores the same five settings under the same names, so the
/// accessors are mechanical; what differs is the fields an encoding adds.
#[macro_export]
macro_rules! record_options_fields {
    () => {
        fn schema(&self) -> Option<&$crate::Field> {
            self.schema.as_ref()
        }

        fn set_schema(&mut self, schema: $crate::Field) {
            self.schema = Some(schema);
        }

        fn root_name(&self) -> &str {
            self.root_name.as_str()
        }

        fn set_root_name(&mut self, root_name: smol_str::SmolStr) {
            self.root_name = root_name;
        }

        fn safe(&self) -> bool {
            self.safe
        }

        fn set_safe(&mut self, safe: bool) {
            self.safe = safe;
        }

        fn batch_size(&self) -> Option<usize> {
            self.batch_size
        }

        fn set_batch_size(&mut self, batch_size: Option<usize>) {
            self.batch_size = batch_size;
        }

        fn level(&self) -> $crate::Level {
            self.level
        }

        fn set_level(&mut self, level: $crate::Level) {
            self.level = level;
        }

        fn merge_by_names(&self) -> &[String] {
            &self.merge_by_names
        }

        fn set_merge_by_names(&mut self, merge_by_names: Vec<String>) {
            self.merge_by_names = merge_by_names;
        }

        fn select_by_names(&self) -> &[String] {
            &self.select_by_names
        }

        fn set_select_by_names(&mut self, select_by_names: Vec<String>) {
            self.select_by_names = select_by_names;
        }

        fn filter_partitions(&self) -> &[(String, String)] {
            &self.filter_partitions
        }

        fn set_filter_partitions(&mut self, filter_partitions: Vec<(String, String)>) {
            self.filter_partitions = filter_partitions;
        }
    };
}

/// One value naming every record encoding's options.
///
/// The variant *is* the encoding: a record call takes `RecordOptions` and
/// needs no separate format argument.
#[derive(Clone, Debug)]
pub enum RecordOptions {
    /// Arrow IPC stream options.
    Ipc(IpcOptions),
    /// Apache Parquet file options.
    #[cfg(feature = "parquet")]
    Parquet(crate::parquet::ParquetOptions),
    /// Apache Avro container options.
    Avro(crate::avro::AvroOptions),
    /// Text-line options: records split by a terminator, grouped and
    /// projected by [`TextLineOptions`](crate::text::TextLineOptions).
    ///
    /// Boxed because the extractor - two compiled expressions and a schema -
    /// dwarfs the other variants, and options are cloned per read.
    Text(Box<crate::text::TextOptions>),
}

impl RecordOptions {
    /// Derive the options for the encoding a media type names.
    ///
    /// Content codings are ignored here: they are the handle's business, not
    /// the record encoding's.
    ///
    /// # Errors
    ///
    /// Returns an error when no encoding in this build covers `media_type`,
    /// naming what was found.
    pub fn for_media_type(media_type: &MediaType) -> Result<Self> {
        Self::for_mime_type(media_type.base())
    }

    /// Derive the options for the encoding a MIME type names.
    ///
    /// # Errors
    ///
    /// Returns an error when no encoding in this build covers `base`.
    pub fn for_mime_type(base: &MimeType) -> Result<Self> {
        if base == &MimeType::ARROW_STREAM || base == &MimeType::ARROW_FILE {
            return Ok(Self::Ipc(IpcOptions::new()));
        }
        #[cfg(feature = "parquet")]
        if base == &MimeType::PARQUET {
            return Ok(Self::Parquet(crate::parquet::ParquetOptions::new()));
        }
        if base == &MimeType::AVRO {
            return Ok(Self::Avro(crate::avro::AvroOptions::new()));
        }
        // Plain text reads and writes as lines: the projection is the
        // encoding, so a `.log` answers the record surface out of the box.
        if base == &MimeType::PLAIN_TEXT {
            return Ok(Self::Text(Box::new(crate::text::TextOptions::new())));
        }
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                if cfg!(feature = "parquet") {
                    "a record encoding this build implements (application/vnd.apache.arrow.stream, application/vnd.apache.parquet, application/avro, text/plain)"
                } else {
                    "a record encoding this build implements (application/vnd.apache.arrow.stream, application/avro, text/plain; the `parquet` feature is not enabled)"
                },
                base,
            ),
        })
    }

    /// Return the MIME type of the encoding these options describe.
    pub const fn mime_type(&self) -> MimeType {
        match self {
            Self::Ipc(_) => MimeType::ARROW_STREAM,
            #[cfg(feature = "parquet")]
            Self::Parquet(_) => MimeType::PARQUET,
            Self::Avro(_) => MimeType::AVRO,
            Self::Text(_) => MimeType::PLAIN_TEXT,
        }
    }
}

impl IORecordOptions for RecordOptions {
    fn schema(&self) -> Option<&Field> {
        match self {
            Self::Ipc(options) => options.schema(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.schema(),
            Self::Avro(options) => options.schema(),
            Self::Text(options) => options.schema(),
        }
    }

    fn set_schema(&mut self, schema: Field) {
        match self {
            Self::Ipc(options) => options.set_schema(schema),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_schema(schema),
            Self::Avro(options) => options.set_schema(schema),
            Self::Text(options) => options.set_schema(schema),
        }
    }

    fn root_name(&self) -> &str {
        match self {
            Self::Ipc(options) => options.root_name(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.root_name(),
            Self::Avro(options) => options.root_name(),
            Self::Text(options) => options.root_name(),
        }
    }

    fn set_root_name(&mut self, root_name: SmolStr) {
        match self {
            Self::Ipc(options) => options.set_root_name(root_name),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_root_name(root_name),
            Self::Avro(options) => options.set_root_name(root_name),
            Self::Text(options) => options.set_root_name(root_name),
        }
    }

    fn safe(&self) -> bool {
        match self {
            Self::Ipc(options) => options.safe(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.safe(),
            Self::Avro(options) => options.safe(),
            Self::Text(options) => options.safe(),
        }
    }

    fn set_safe(&mut self, safe: bool) {
        match self {
            Self::Ipc(options) => options.set_safe(safe),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_safe(safe),
            Self::Avro(options) => options.set_safe(safe),
            Self::Text(options) => options.set_safe(safe),
        }
    }

    fn batch_size(&self) -> Option<usize> {
        match self {
            Self::Ipc(options) => options.batch_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.batch_size(),
            Self::Avro(options) => options.batch_size(),
            Self::Text(options) => options.batch_size(),
        }
    }

    fn set_batch_size(&mut self, batch_size: Option<usize>) {
        match self {
            Self::Ipc(options) => options.set_batch_size(batch_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_batch_size(batch_size),
            Self::Avro(options) => options.set_batch_size(batch_size),
            Self::Text(options) => options.set_batch_size(batch_size),
        }
    }

    fn level(&self) -> Level {
        match self {
            Self::Ipc(options) => options.level(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.level(),
            Self::Avro(options) => options.level(),
            Self::Text(options) => options.level(),
        }
    }

    fn set_level(&mut self, level: Level) {
        match self {
            Self::Ipc(options) => options.set_level(level),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_level(level),
            Self::Avro(options) => options.set_level(level),
            Self::Text(options) => options.set_level(level),
        }
    }

    fn merge_by_names(&self) -> &[String] {
        match self {
            Self::Ipc(options) => options.merge_by_names(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.merge_by_names(),
            Self::Avro(options) => options.merge_by_names(),
            Self::Text(options) => options.merge_by_names(),
        }
    }

    fn set_merge_by_names(&mut self, merge_by_names: Vec<String>) {
        match self {
            Self::Ipc(options) => options.set_merge_by_names(merge_by_names),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_merge_by_names(merge_by_names),
            Self::Avro(options) => options.set_merge_by_names(merge_by_names),
            Self::Text(options) => options.set_merge_by_names(merge_by_names),
        }
    }

    fn select_by_names(&self) -> &[String] {
        match self {
            Self::Ipc(options) => options.select_by_names(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.select_by_names(),
            Self::Avro(options) => options.select_by_names(),
            Self::Text(options) => options.select_by_names(),
        }
    }

    fn set_select_by_names(&mut self, select_by_names: Vec<String>) {
        match self {
            Self::Ipc(options) => options.set_select_by_names(select_by_names),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_select_by_names(select_by_names),
            Self::Avro(options) => options.set_select_by_names(select_by_names),
            Self::Text(options) => options.set_select_by_names(select_by_names),
        }
    }

    fn filter_partitions(&self) -> &[(String, String)] {
        match self {
            Self::Ipc(options) => options.filter_partitions(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.filter_partitions(),
            Self::Avro(options) => options.filter_partitions(),
            Self::Text(options) => options.filter_partitions(),
        }
    }

    fn set_filter_partitions(&mut self, filter_partitions: Vec<(String, String)>) {
        match self {
            Self::Ipc(options) => options.set_filter_partitions(filter_partitions),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_filter_partitions(filter_partitions),
            Self::Avro(options) => options.set_filter_partitions(filter_partitions),
            Self::Text(options) => options.set_filter_partitions(filter_partitions),
        }
    }
}

impl From<IpcOptions> for RecordOptions {
    fn from(value: IpcOptions) -> Self {
        Self::Ipc(value)
    }
}

#[cfg(feature = "parquet")]
impl From<crate::parquet::ParquetOptions> for RecordOptions {
    fn from(value: crate::parquet::ParquetOptions) -> Self {
        Self::Parquet(value)
    }
}

impl From<crate::avro::AvroOptions> for RecordOptions {
    fn from(value: crate::avro::AvroOptions) -> Self {
        Self::Avro(value)
    }
}

impl From<crate::text::TextOptions> for RecordOptions {
    fn from(value: crate::text::TextOptions) -> Self {
        Self::Text(Box::new(value))
    }
}

impl From<crate::text::TextLineOptions> for RecordOptions {
    fn from(value: crate::text::TextLineOptions) -> Self {
        Self::Text(Box::new(crate::text::TextOptions::with_lines(value)))
    }
}
