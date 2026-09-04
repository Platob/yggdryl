//! Record-oriented media operations derived from the one byte-storage trait.
//!
//! [`IOBase`] remains the sole storage abstraction. [`IOMedia`] is its
//! object-safe media surface: defaults reach positional storage through the
//! hidden escape hatches, while encoding wrappers override only the operations
//! their representation implements specially.

use crate::IOBase;
#[cfg(feature = "arrow")]
use crate::Result;
#[cfg(feature = "arrow")]
use crate::media::RecordOptions;

/// Record-oriented operations every [`IOBase`] handle exposes.
///
/// The trait deliberately has no storage primitives of its own. Implementors
/// return their [`IOBase`] view through hidden methods so the shared defaults
/// keep one storage abstraction and remain callable through trait objects.
pub trait IOMedia: Send {
    /// Borrow this media value as the one positional storage abstraction.
    #[doc(hidden)]
    fn as_io_base(&self) -> &dyn IOBase;

    /// Mutably borrow this media value as the one positional storage abstraction.
    #[doc(hidden)]
    fn as_io_base_mut(&mut self) -> &mut dyn IOBase;

    /// Return the number of logical rows in the whole media value.
    ///
    /// The answer ignores transient selection, partition-filter, and read-limit
    /// settings held by a stateful media wrapper. Implementations whose format
    /// records row counts in metadata override this default so no row arrays
    /// are decoded. An explicitly opened media caches that metadata until
    /// [`IOBase::close`]; a closed handle computes a fresh answer on each call.
    /// Text extraction is the unavoidable exception: record boundaries can
    /// depend on multiline expressions, so counting streams the extractor
    /// without materializing Arrow batches.
    ///
    /// # Errors
    ///
    /// Returns a metadata, listing, decoding, or row-count overflow failure.
    #[cfg(feature = "arrow")]
    fn row_size(&self) -> Result<u64> {
        let options = dimension_options(self)?;
        let handle = self.as_io_base();
        if handle.is_container() {
            #[cfg(feature = "iceberg")]
            if let Some(table) = crate::media::iceberg::located(handle)? {
                return table.row_size();
            }
            let encoding = options.mime_type();
            let mut rows = 0_u64;
            for child in handle.children_where(&[], false)? {
                let child = child?;
                if child.media_type().base() != &encoding {
                    continue;
                }
                rows = add_rows(rows, crate::iobase::leaf_row_size(&child, &options)?)?;
            }
            return Ok(rows);
        }
        crate::iobase::leaf_row_size(handle, &options)
    }

    /// Return the number of columns in the media's canonical Struct field.
    ///
    /// Like [`Self::row_size`], this describes the whole logical media value
    /// rather than a transient selection. Schema-bearing encodings answer from
    /// their header or footer and never decode rows. An explicitly declared
    /// field remains authoritative, including for an empty resource.
    ///
    /// # Errors
    ///
    /// Returns a metadata, schema, or listing failure.
    #[cfg(feature = "arrow")]
    fn column_size(&self) -> Result<usize> {
        use crate::media::IORecordOptions;

        let options = dimension_options(self)?;
        if let Some(field) = options.field() {
            return Ok(field.fields().len());
        }
        let handle = self.as_io_base();
        #[cfg(feature = "iceberg")]
        if handle.is_container() {
            if let Some(table) = crate::media::iceberg::located(handle)? {
                return table.column_size();
            }
        }
        // Preserve the container route: its canonical field may include Hive
        // partition columns restored from paths across multiple leaves.
        if handle.is_container() {
            return Ok(self.read_arrow_field(&options)?.fields().len());
        }
        if handle.is_empty() {
            return Ok(0);
        }
        Ok(crate::iobase::leaf_field(handle, &options)?.fields().len())
    }

    /// Return the record options this resource's encoding names.
    ///
    /// This is what a caller supplies when they have no options of their own,
    /// so the encoding is never guessed: it is whatever the handle already says
    /// it holds. A container has no bytes and therefore no media type of its
    /// own, so it answers with the encoding of the leaves beneath it - a
    /// partitioned tree is one table in one encoding - and a container that is
    /// an Iceberg table answers with the encoding its data files are written
    /// in, which its metadata knows before a single file exists.
    ///
    /// # Errors
    ///
    /// Returns an error when no record encoding in this build covers the
    /// handle's media type, or the media type of anything below it.
    #[cfg(feature = "arrow")]
    fn record_options(&self) -> Result<RecordOptions> {
        let handle = self.as_io_base();
        if handle.is_container() {
            #[cfg(feature = "iceberg")]
            if let Some(table) = crate::media::iceberg::located(handle)? {
                return table.record_options();
            }
            // The listing is lazy, so a lake costs the walk to its first
            // structured leaf and stops there. Text answers last: plain text
            // maps to the line projection, so a stray README or marker file
            // must not re-type a lake whose data files are a structured
            // encoding.
            let mut lines = None;
            for child in handle.children_where(&[], false)? {
                if let Ok(options) = RecordOptions::for_media_type(child?.media_type()) {
                    if matches!(options, RecordOptions::Text(_)) {
                        lines.get_or_insert(options);
                        continue;
                    }
                    return Ok(options);
                }
            }
            if let Some(options) = lines {
                return Ok(options);
            }
        }
        RecordOptions::for_media_type(handle.media_type())
    }

    /// Read this Parquet leaf's footer statistics without decoding rows.
    ///
    /// The handle's media type selects the encoding first. This refuses an
    /// IPC, Avro, text, or container handle with a typed record error instead
    /// of trying to parse unrelated bytes as a Parquet footer.
    ///
    /// # Errors
    ///
    /// Returns an encoding, footer, or positional-read failure.
    #[cfg(feature = "parquet")]
    fn read_parquet_statistics(&self) -> Result<crate::media::parquet::FileStatistics> {
        let handle = parquet_leaf(self)?;
        Ok(crate::media::parquet::read_statistics(handle)?)
    }

    /// Recompute one Parquet geospatial column's statistics from stored WKB.
    ///
    /// Unlike [`Self::read_parquet_statistics`], this is a projected column
    /// scan: it decodes only the named top-level binary column and folds its
    /// geometries without materializing them.
    ///
    /// # Errors
    ///
    /// Returns an encoding or read failure, an unknown/non-binary column, or
    /// malformed WKB.
    #[cfg(feature = "parquet")]
    fn read_parquet_geospatial_statistics(
        &self,
        column: &str,
    ) -> Result<crate::media::parquet::GeospatialStatistics> {
        let handle = parquet_leaf(self)?;
        Ok(crate::media::parquet::read_geospatial_statistics(
            handle, column,
        )?)
    }

    /// Read the canonical non-null Struct root Field of this resource.
    ///
    /// A declared schema is returned as it stands; otherwise this is the shape
    /// [`Self::read_arrow_reader`] reports, so the schema a caller reads
    /// and the batches a caller gets can never disagree.
    ///
    /// # Errors
    ///
    /// Returns a read, decoding, or schema-projection failure.
    #[cfg(feature = "arrow")]
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<crate::Field> {
        use crate::media::IORecordOptions;

        if let Some(field) = options.field() {
            return Ok(field.clone());
        }
        let schema = self.read_arrow_reader(options)?.schema();
        Ok(crate::arrow::field_from_arrow_schema(
            options.name(),
            schema.as_ref(),
        )?)
    }

    /// Read this resource's rows as one [`BatchReader`](crate::arrow::BatchReader).
    ///
    /// This is the one read path, so an encoding is decoded in exactly one
    /// place, and the result streams: one batch at a time, never a materialized
    /// vector.
    ///
    /// **A declared schema selects and casts during the read.** The columns it
    /// names that the resource stores become the encoding's own projection - a
    /// Parquet projection mask, an Arrow IPC projection - so the rest are
    /// skipped rather than read and discarded, and what comes back is then cast
    /// to the declared shape as each batch arrives. Ordering, conversion, and a
    /// column the resource does not hold are the cast's business, because a
    /// projection can only drop columns, never reorder or invent them. Say
    /// plainly what each encoding's projection saves: Parquet skips locating and
    /// decoding a column chunk, while an Arrow IPC record batch is one
    /// contiguous message, so its projection saves the decode and the
    /// allocation but not the bytes. With no declared schema the stored shape is
    /// preserved exactly.
    ///
    /// **A folder reads as the table beneath it.** When this handle addresses a
    /// container, every leaf holding this encoding is read in turn, the columns
    /// its `column=value` directories spell out are restored, and each batch is
    /// cast to one root - so a caller never has to know whether they addressed
    /// one file or a partitioned tree. A container holding a *table format*
    /// reads through that format instead: an Iceberg table's current snapshot
    /// says which data files are live and which of them a filtered read can
    /// skip, so the folder is never listed and a file an overwrite replaced is
    /// never read back.
    ///
    /// Per the laziness contract, a resource that does not exist yet holds no
    /// batches rather than failing.
    ///
    /// The shaping order is fixed: declared schema, then selection, then
    /// completion cast, then partition filter, then
    /// [`max_row_size`](crate::media::IORecordOptions::max_row_size) and
    /// [`max_byte_size`](crate::media::IORecordOptions::max_byte_size)
    /// last - so a limit counts result rows, and a limit of ten with a filter
    /// means the first ten matching rows. A satisfied limit stops pulling, so
    /// the rest of the resource is never decoded.
    ///
    /// # Errors
    ///
    /// Returns a listing, read, decoding, or cast failure.
    #[cfg(feature = "arrow")]
    fn read_arrow_reader(&self, options: &RecordOptions) -> Result<crate::arrow::BatchReader> {
        use crate::media::IORecordOptions;

        let handle = self.as_io_base();
        let reader = if handle.is_container() {
            #[cfg(feature = "iceberg")]
            if let Some(table) = crate::media::iceberg::located(handle)? {
                let filtered =
                    crate::media::partition::filtered_reader(table.read(options)?, options)?;
                return options
                    .limit_arrow_reader(crate::iobase::select_reader(filtered, options)?);
            }
            crate::media::partition::folder_reader(handle, options)?
        } else {
            crate::iobase::leaf_reader(handle, options)?
        };
        let reader = crate::media::partition::filtered_reader(reader, options)?;
        options.limit_arrow_reader(crate::iobase::select_reader(reader, options)?)
    }

    /// Write a batch stream using one explicit [`IOMode`](crate::IOMode).
    ///
    /// This is the configurable counterpart to the three intent-specific
    /// primitives. The canonical argument order is input, mode, options; the
    /// Python and JavaScript bindings keep that order and infer/cast their
    /// input before redirecting here.
    ///
    /// # Errors
    ///
    /// Returns the selected primitive's validation, cast, read, or publication
    /// failure. In particular, merge requires non-empty match keys while the
    /// other modes refuse them.
    #[cfg(feature = "arrow")]
    fn write_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        mode: crate::IOMode,
        options: &RecordOptions,
    ) -> Result<()> {
        // The generic entry point owns mode validation so an implementor's
        // specialized primitive cannot consume a one-shot reader before the
        // authoritative intent and its key settings are known to agree.
        options.require_write_mode(mode)?;
        match mode {
            crate::IOMode::Overwrite => self.overwrite_arrow_reader(batches, options),
            crate::IOMode::Append => self.append_arrow_reader(batches, options),
            crate::IOMode::Merge => self.merge_arrow_reader(batches, options),
            crate::IOMode::ReadOnly | crate::IOMode::Random => Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$.mode"),
                reason: smol_str::SmolStr::new_static(
                    "write mode readonly or random is not supported for write_arrow_reader",
                ),
            }),
        }
    }

    /// Replace this resource's rows with every batch `batches` yields.
    ///
    /// This is the required publication hook each handle implements. The
    /// workspace implementations use [`super::overwrite_arrow_reader_default`] for
    /// byte and folder handles; table formats override it so one call is one
    /// native commit. A declared field is applied to the incoming rows exactly
    /// once, followed by selection and completion onto the stored field.
    ///
    /// A folder routes each row to the leaf its partition values name, creating
    /// the `column=value` directory when the layout has one and no leaf holds
    /// that value yet. A folder holding a table format commits instead: an
    /// Iceberg table writes one snapshot, whose merge reads only the data files
    /// whose statistics say they can hold an incoming key and carries the rest
    /// forward untouched.
    ///
    /// A limited overwrite truncates data the caller offered:
    /// [`max_row_size`](crate::media::IORecordOptions::max_row_size) and
    /// [`max_byte_size`](crate::media::IORecordOptions::max_byte_size) bound
    /// the incoming reader exactly as they bound a read, and what they cut off
    /// is never pulled from it. A match key is refused: use
    /// [`merge_arrow_reader`](Self::merge_arrow_reader) for that intent.
    ///
    /// [`commit_row_size`](crate::media::IORecordOptions::commit_row_size)
    /// changes publication, not shaping: the incoming reader is cast and
    /// limited once, then sliced into exact row cadences. The first cadence
    /// overwrites and every later one appends. Successful prefixes remain
    /// visible if a later cadence fails; zero is rejected before the source is
    /// pulled. With no cadence, this method publishes once at the end.
    ///
    /// # Errors
    ///
    /// Returns a listing, read, schema, cast, encoding, or write failure.
    #[cfg(feature = "arrow")]
    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()>;

    /// Publish one complete leaf value that generic write shaping already
    /// prepared.
    ///
    /// This hidden hook exists for a resumable runtime write: its target field
    /// is fixed before the first await and must not be re-read or re-planned at
    /// every cadence. Stateful media override the hook only to reconcile their
    /// open metadata cache; the default reaches the one encoding writer
    /// directly. Callers must never pass unshaped incoming rows here.
    ///
    /// # Errors
    ///
    /// Returns an encoding or publication failure.
    #[cfg(feature = "arrow")]
    #[doc(hidden)]
    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        crate::iobase::leaf_writer(self.as_io_base_mut(), batches, options)
    }

    /// Replace this resource with one Arrow record batch.
    ///
    /// This optimized held-batch adapter performs no row conversion or copy:
    /// it widens the batch into a one-item reader and redirects to
    /// [`overwrite_arrow_reader`](Self::overwrite_arrow_reader).
    ///
    /// # Errors
    ///
    /// Returns the same field, cast, encoding, and write failures as the
    /// reader primitive.
    #[cfg(feature = "arrow")]
    fn overwrite_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<()> {
        let schema = batch.schema();
        self.overwrite_arrow_reader(crate::arrow::batch_reader(schema, [batch]), options)
    }

    /// Write one held Arrow batch using one explicit intent.
    ///
    /// The selected same-shape adapter remains authoritative: it widens the
    /// batch into a one-item reader without copying, while an implementation
    /// may preserve a more specialized record-batch override.
    ///
    /// # Errors
    ///
    /// Returns the selected held-batch adapter's validation, cast, encoding,
    /// or publication failure.
    #[cfg(feature = "arrow")]
    fn write_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        mode: crate::IOMode,
        options: &RecordOptions,
    ) -> Result<()> {
        options.require_write_mode(mode)?;
        match mode {
            crate::IOMode::Overwrite => self.overwrite_arrow_batch(batch, options),
            crate::IOMode::Append => self.append_arrow_batch(batch, options),
            crate::IOMode::Merge => self.merge_arrow_batch(batch, options),
            crate::IOMode::ReadOnly | crate::IOMode::Random => Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$.mode"),
                reason: smol_str::SmolStr::new_static(
                    "write mode readonly or random is not supported for write_arrow_batch",
                ),
            }),
        }
    }

    /// Add every batch `batches` yields after the rows this resource holds.
    ///
    /// The encodings here are whole-value containers - an Arrow IPC stream and a
    /// Parquet file each carry one schema and one footer - so appending means
    /// reading what is there, adding to it, and rewriting. The current rows are
    /// read as the declared schema when there is one and as the stored schema
    /// otherwise, a resource holding nothing is skipped rather than decoded, and
    /// the incoming batches are cast to that same shape, so a caller may append
    /// data whose schema merely *fits*. Both sides stream: the stored batches
    /// are chained ahead of the incoming ones and encoded as they arrive, so
    /// neither is collected.
    ///
    /// A folder appends into each partition the incoming rows name, leaving
    /// every other partition untouched. A folder holding a table format appends
    /// the way that format does: an Iceberg table writes new data files and
    /// commits a snapshot that keeps every manifest the last one had, so nothing
    /// already stored is read, rewritten, or even listed.
    ///
    /// A limited write truncates data the caller offered: an append is a
    /// write, so
    /// [`max_row_size`](crate::media::IORecordOptions::max_row_size) and
    /// [`max_byte_size`](crate::media::IORecordOptions::max_byte_size) bound
    /// the incoming reader here exactly as they do on
    /// [`overwrite_arrow_reader`](Self::overwrite_arrow_reader), and a
    /// limit combined with a non-empty match key is refused the same way.
    /// `commit_row_size` retains append intent for every bounded publication;
    /// successful prefixes remain visible after a later failure.
    ///
    /// # Errors
    ///
    /// Returns a listing, read, cast, encoding, or write failure. With no
    /// commit cadence the resource stays unchanged until the replacement is
    /// complete; with a positive cadence, completed prefixes stay published.
    #[cfg(feature = "arrow")]
    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        crate::iobase::append_arrow_reader_default(self.as_io_base_mut(), batches, options)
    }

    /// Append one Arrow record batch to this resource.
    ///
    /// The batch is widened into a one-item reader without copying and routed
    /// through [`append_arrow_reader`](Self::append_arrow_reader).
    ///
    /// # Errors
    ///
    /// Returns the same intent, cast, encoding, and write failures as the
    /// reader primitive.
    #[cfg(feature = "arrow")]
    fn append_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<()> {
        let schema = batch.schema();
        self.append_arrow_reader(crate::arrow::batch_reader(schema, [batch]), options)
    }

    /// Merge every incoming row into this resource by the declared match key.
    ///
    /// The match key is required. Stored rows are indexed because a reader
    /// cannot be rewound; the incoming side remains streaming and is folded in
    /// one batch at a time. The resulting stream is published through the
    /// implementor's [`overwrite_arrow_reader`](Self::overwrite_arrow_reader)
    /// after the declared field has been popped from a cloned options value, so
    /// the already-cast rows are never cast to that field twice.
    ///
    /// # Errors
    ///
    /// Returns a read, cast, merge, encoding, or write failure. An empty match
    /// key is refused: use overwrite or append when rows have no identity.
    /// `commit_row_size` retains merge intent for every bounded publication;
    /// successful prefixes remain visible after a later failure.
    #[cfg(feature = "arrow")]
    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        crate::iobase::merge_arrow_reader_default(self.as_io_base_mut(), batches, options)
    }

    /// Merge one Arrow record batch into this resource by explicit keys.
    ///
    /// The batch is widened into a one-item reader without copying and routed
    /// through [`merge_arrow_reader`](Self::merge_arrow_reader).
    ///
    /// # Errors
    ///
    /// Returns the same key, cast, merge, encoding, and write failures as the
    /// reader primitive.
    #[cfg(feature = "arrow")]
    fn merge_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<()> {
        let schema = batch.schema();
        self.merge_arrow_reader(crate::arrow::batch_reader(schema, [batch]), options)
    }

    /// Replace this resource from native row values.
    ///
    /// A native row is one ordered [`Scalar`](crate::Scalar) sequence under
    /// `options.field`; no parallel record or record-schema type exists.
    /// Rust structs participate with `TryInto<Scalar>`. Implementing the
    /// ordinary infallible `From<Row> for Scalar` is sufficient because the
    /// standard library supplies its `TryInto` implementation.
    ///
    /// Rows are converted lazily and held only for the current
    /// [`batch_row_size`](crate::media::IORecordOptions::batch_row_size), additionally
    /// bounded by `commit_row_size` when it is smaller so conversion never
    /// reads past the next publication. The exact
    /// declared schema reaches the reader primitive unchanged. Its one shaping
    /// seam applies selection, limits, and stored-shape completion, and its
    /// exact-schema fast path returns these arrays without rebuilding them.
    ///
    /// ```
    /// use yggdryl::media::IORecordOptions;
    /// use yggdryl::{IOMedia, holder::Buffer};
    /// use yggdryl::{DataType, MimeType, Scalar};
    ///
    /// struct Quote {
    ///     id: i32,
    ///     symbol: &'static str,
    /// }
    ///
    /// impl From<Quote> for Scalar {
    ///     fn from(row: Quote) -> Self {
    ///         Scalar::from_sequence([Scalar::from(row.id), Scalar::from(row.symbol)])
    ///     }
    /// }
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let field = DataType::from_fields([
    ///     DataType::Int32.required_field("id"),
    ///     DataType::Utf8.required_field("symbol"),
    /// ])?
    /// .required_field("quote");
    /// let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    /// let options = handle.record_options()?.with_field(field);
    ///
    /// handle.overwrite_records(
    ///     [Quote { id: 1, symbol: "AAPL" }, Quote { id: 2, symbol: "MSFT" }],
    ///     &options,
    /// )?;
    /// assert_eq!(handle.read_arrow_reader(&options)?.count(), 1);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error before pulling `records` when `options.field` is
    /// absent or not a non-null Struct root. A pulled row can fail its
    /// `TryInto<Scalar>` conversion, field validation, Arrow materialization,
    /// or the delegated overwrite.
    #[cfg(feature = "arrow")]
    fn overwrite_records<I, R>(&mut self, records: I, options: &RecordOptions) -> Result<()>
    where
        Self: Sized,
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<crate::Scalar>,
        R::Error: Into<crate::Error>,
    {
        use crate::media::IORecordOptions;

        options.require_write_mode(crate::IOMode::Overwrite)?;
        options.require_commit_row_size()?;
        let field = options.require_field()?.clone();
        let batches = crate::arrow::rows::reader(
            &field,
            records,
            options.write_batch_row_size(),
            options.commit_row_size(),
            options.max_row_size(),
        )?;
        self.overwrite_arrow_reader(batches, options)
    }

    /// Append native row values to this resource.
    ///
    /// This is the row-by-row adapter over
    /// [`append_arrow_reader`](Self::append_arrow_reader). It requires
    /// `options.field`, lazily converts each struct through `TryInto<Scalar>`,
    /// and holds at most the current row batch. An empty iterator is a no-op.
    ///
    /// # Errors
    ///
    /// Returns the same field, row-conversion, intent, cast, encoding, and
    /// write failures as [`overwrite_records`](Self::overwrite_records) and
    /// [`append_arrow_reader`](Self::append_arrow_reader).
    #[cfg(feature = "arrow")]
    fn append_records<I, R>(&mut self, records: I, options: &RecordOptions) -> Result<()>
    where
        Self: Sized,
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<crate::Scalar>,
        R::Error: Into<crate::Error>,
    {
        use crate::media::IORecordOptions;

        options.require_write_mode(crate::IOMode::Append)?;
        options.require_commit_row_size()?;
        let field = options.require_field()?.clone();
        let batches = crate::arrow::rows::reader(
            &field,
            records,
            options.write_batch_row_size(),
            options.commit_row_size(),
            options.max_row_size(),
        )?;
        self.append_arrow_reader(batches, options)
    }

    /// Merge native row values into this resource by explicit keys.
    ///
    /// This is the row-by-row adapter over
    /// [`merge_arrow_reader`](Self::merge_arrow_reader). `merge_by_names` must
    /// contain at least one field name; an empty iterator is a no-op once that
    /// intent has been validated.
    ///
    /// # Errors
    ///
    /// Returns the same field, row-conversion, key, cast, encoding, and write
    /// failures as [`overwrite_records`](Self::overwrite_records) and
    /// [`merge_arrow_reader`](Self::merge_arrow_reader).
    #[cfg(feature = "arrow")]
    fn merge_records<I, R>(&mut self, records: I, options: &RecordOptions) -> Result<()>
    where
        Self: Sized,
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<crate::Scalar>,
        R::Error: Into<crate::Error>,
    {
        use crate::media::IORecordOptions;

        options.require_write_mode(crate::IOMode::Merge)?;
        options.require_commit_row_size()?;
        let field = options.require_field()?.clone();
        let batches = crate::arrow::rows::reader(
            &field,
            records,
            options.write_batch_row_size(),
            options.commit_row_size(),
            options.max_row_size(),
        )?;
        self.merge_arrow_reader(batches, options)
    }

    /// Write native row values using one explicit intent.
    ///
    /// The selected same-shape adapter owns the one streamed row-conversion
    /// implementation and then redirects its reader to the matching primitive.
    /// Mode is validated before `records` is turned into or pulled as an
    /// iterator.
    ///
    /// # Errors
    ///
    /// Returns a missing field, row conversion, validation, cast, or selected
    /// publication failure.
    #[cfg(feature = "arrow")]
    fn write_records<I, R>(
        &mut self,
        records: I,
        mode: crate::IOMode,
        options: &RecordOptions,
    ) -> Result<()>
    where
        Self: Sized,
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<crate::Scalar>,
        R::Error: Into<crate::Error>,
    {
        options.require_write_mode(mode)?;
        match mode {
            crate::IOMode::Overwrite => self.overwrite_records(records, options),
            crate::IOMode::Append => self.append_records(records, options),
            crate::IOMode::Merge => self.merge_records(records, options),
            crate::IOMode::ReadOnly | crate::IOMode::Random => Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$.mode"),
                reason: smol_str::SmolStr::new_static(
                    "write mode readonly or random is not supported for write_records",
                ),
            }),
        }
    }
}

/// Remove settings that narrow a read before computing whole-media dimensions.
#[cfg(feature = "arrow")]
fn dimension_options<M: IOMedia + ?Sized>(media: &M) -> Result<RecordOptions> {
    use crate::media::IORecordOptions;

    let mut options = media.record_options()?;
    options.set_select_by_names(Vec::new());
    options.set_filter_partitions(Vec::new());
    options.set_max_row_size(None);
    options.set_max_byte_size(None);
    Ok(options)
}

/// Add one metadata row count without allowing an aggregate to wrap.
#[cfg(feature = "arrow")]
fn add_rows(total: u64, rows: u64) -> Result<u64> {
    total
        .checked_add(rows)
        .ok_or_else(|| crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$"),
            reason: smol_str::SmolStr::new_static("logical row count exceeds u64::MAX"),
        })
}

/// Resolve one media value as a Parquet leaf before a footer or column read.
#[cfg(feature = "parquet")]
fn parquet_leaf<M: IOMedia + ?Sized>(media: &M) -> Result<&dyn IOBase> {
    let options = media.record_options()?;
    if !matches!(options, RecordOptions::Parquet(_)) {
        return Err(crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$.encoding"),
            reason: smol_str::format_smolstr!(
                "expected Parquet media, got {}",
                options.mime_type()
            ),
        });
    }
    let handle = media.as_io_base();
    if handle.is_container() {
        return Err(crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$"),
            reason: smol_str::SmolStr::new_static(
                "expected one Parquet leaf for file statistics, got a container",
            ),
        });
    }
    Ok(handle)
}

/// Implement the default media contract for an [`IOBase`] value.
///
/// Use this inside an `impl IOMedia for Type` block when record operations
/// should run on the value itself rather than being forwarded to an inner
/// handle.
#[cfg(feature = "arrow")]
#[macro_export]
macro_rules! impl_default_iomedia {
    () => {
        fn as_io_base(&self) -> &dyn $crate::IOBase {
            self
        }

        fn as_io_base_mut(&mut self) -> &mut dyn $crate::IOBase {
            self
        }

        fn overwrite_arrow_reader(
            &mut self,
            batches: $crate::arrow::BatchReader,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()> {
            $crate::iobase::overwrite_arrow_reader_default(self, batches, options)
        }
    };
}

/// Schema-only form of [`impl_default_iomedia!`].
#[cfg(not(feature = "arrow"))]
#[macro_export]
macro_rules! impl_default_iomedia {
    () => {
        fn as_io_base(&self) -> &dyn $crate::IOBase {
            self
        }

        fn as_io_base_mut(&mut self) -> &mut dyn $crate::IOBase {
            self
        }
    };
}

/// Feature-selected media forwarding bodies used by [`delegate_iomedia!`].
#[cfg(feature = "arrow")]
#[doc(hidden)]
#[macro_export]
macro_rules! __delegate_iomedia_arrow {
    ($handle:ident) => {
        fn row_size(&self) -> $crate::Result<u64> {
            $crate::IOMedia::row_size(&self.$handle)
        }

        fn column_size(&self) -> $crate::Result<usize> {
            $crate::IOMedia::column_size(&self.$handle)
        }

        fn record_options(&self) -> $crate::Result<$crate::media::RecordOptions> {
            $crate::IOMedia::record_options(&self.$handle)
        }

        fn read_arrow_field(
            &self,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<$crate::Field> {
            $crate::IOMedia::read_arrow_field(&self.$handle, options)
        }

        fn read_arrow_reader(
            &self,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<$crate::arrow::BatchReader> {
            $crate::IOMedia::read_arrow_reader(&self.$handle, options)
        }

        fn overwrite_arrow_reader(
            &mut self,
            batches: $crate::arrow::BatchReader,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()> {
            $crate::IOMedia::overwrite_arrow_reader(&mut self.$handle, batches, options)
        }

        fn overwrite_prepared_arrow_reader(
            &mut self,
            batches: $crate::arrow::BatchReader,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()> {
            $crate::IOMedia::overwrite_prepared_arrow_reader(&mut self.$handle, batches, options)
        }

        fn overwrite_arrow_batch(
            &mut self,
            batch: arrow_array::RecordBatch,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()> {
            $crate::IOMedia::overwrite_arrow_batch(&mut self.$handle, batch, options)
        }

        fn append_arrow_reader(
            &mut self,
            batches: $crate::arrow::BatchReader,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()> {
            $crate::IOMedia::append_arrow_reader(&mut self.$handle, batches, options)
        }

        fn append_arrow_batch(
            &mut self,
            batch: arrow_array::RecordBatch,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()> {
            $crate::IOMedia::append_arrow_batch(&mut self.$handle, batch, options)
        }

        fn merge_arrow_reader(
            &mut self,
            batches: $crate::arrow::BatchReader,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()> {
            $crate::IOMedia::merge_arrow_reader(&mut self.$handle, batches, options)
        }

        fn merge_arrow_batch(
            &mut self,
            batch: arrow_array::RecordBatch,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()> {
            $crate::IOMedia::merge_arrow_batch(&mut self.$handle, batch, options)
        }

        fn overwrite_records<I, R>(
            &mut self,
            records: I,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()>
        where
            Self: Sized,
            I: IntoIterator<Item = R>,
            I::IntoIter: Send + 'static,
            R: TryInto<$crate::Scalar>,
            R::Error: Into<$crate::Error>,
        {
            $crate::IOMedia::overwrite_records(&mut self.$handle, records, options)
        }

        fn append_records<I, R>(
            &mut self,
            records: I,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()>
        where
            Self: Sized,
            I: IntoIterator<Item = R>,
            I::IntoIter: Send + 'static,
            R: TryInto<$crate::Scalar>,
            R::Error: Into<$crate::Error>,
        {
            $crate::IOMedia::append_records(&mut self.$handle, records, options)
        }

        fn merge_records<I, R>(
            &mut self,
            records: I,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()>
        where
            Self: Sized,
            I: IntoIterator<Item = R>,
            I::IntoIter: Send + 'static,
            R: TryInto<$crate::Scalar>,
            R::Error: Into<$crate::Error>,
        {
            $crate::IOMedia::merge_records(&mut self.$handle, records, options)
        }
    };
}

/// Schema-only media forwarding bodies.
#[cfg(not(feature = "arrow"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __delegate_iomedia_arrow {
    ($handle:ident) => {};
}

/// Parquet-selected media forwarding bodies used by [`delegate_iomedia!`].
#[cfg(feature = "parquet")]
#[doc(hidden)]
#[macro_export]
macro_rules! __delegate_iomedia_parquet {
    ($handle:ident) => {
        fn read_parquet_statistics(
            &self,
        ) -> $crate::Result<$crate::media::parquet::FileStatistics> {
            $crate::IOMedia::read_parquet_statistics(&self.$handle)
        }

        fn read_parquet_geospatial_statistics(
            &self,
            column: &str,
        ) -> $crate::Result<$crate::media::parquet::GeospatialStatistics> {
            $crate::IOMedia::read_parquet_geospatial_statistics(&self.$handle, column)
        }
    };
}

/// Parquet-free media forwarding bodies.
#[cfg(not(feature = "parquet"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __delegate_iomedia_parquet {
    ($handle:ident) => {};
}

/// Implement [`IOMedia`] by forwarding its whole contract to an inner handle.
///
/// Use this independently from `delegate_iobase!`: storage delegation and
/// media delegation are intentionally separate choices.
#[macro_export]
macro_rules! delegate_iomedia {
    ($handle:ident) => {
        fn as_io_base(&self) -> &dyn $crate::IOBase {
            $crate::IOMedia::as_io_base(&self.$handle)
        }

        fn as_io_base_mut(&mut self) -> &mut dyn $crate::IOBase {
            $crate::IOMedia::as_io_base_mut(&mut self.$handle)
        }

        $crate::__delegate_iomedia_arrow!($handle);
        $crate::__delegate_iomedia_parquet!($handle);
    };
}
