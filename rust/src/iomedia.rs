//! Record-oriented media operations derived from the one byte-storage trait.
//!
//! [`IOBase`] remains the sole storage abstraction. [`IOMedia`] is its
//! object-safe media surface: defaults reach positional storage through the
//! hidden escape hatches, while encoding wrappers override only the operations
//! their representation implements specially.

use crate::IOBase;
use crate::Result;
use crate::media::RecordOptions;

/// What a handle opens as under `options`: a located table format, or the
/// reader over a folder of leaves or over one leaf.
enum Opened {
    Table(Box<dyn crate::media::LocatedTable>),
    Reader(crate::arrow::BatchReader),
}

/// What a handle already known to be a container opens as: the table format
/// located in it, or the reader over its leaves.
fn open_container(handle: &dyn IOBase, options: &RecordOptions) -> Result<Opened> {
    if let Some(table) = crate::media::format::locate(handle)? {
        return Ok(Opened::Table(table));
    }
    Ok(Opened::Reader(crate::media::partition::folder_reader(
        handle, options,
    )?))
}

/// The root field an opened resource reports under `options`: a table
/// answers from its metadata, a reader with the schema its clauses publish.
fn opened_field(opened: Opened, options: &RecordOptions) -> Result<crate::Field> {
    use crate::media::IORecordOptions;

    let schema = match opened {
        // A table format states its schema in its metadata: the table
        // answers it as the table it is, and no scan is planned to learn it.
        Opened::Table(table) => return table.read_arrow_field(options),
        Opened::Reader(reader) => options
            .limit_arrow_reader(options.apply_arrow_expressions(reader)?)?
            .schema(),
    };
    Ok(crate::arrow::field_from_arrow_schema(
        options.name(),
        schema.as_ref(),
    )?)
}

/// A container's root field under `options`: the declared one, or the one
/// the table located in it or its leaves report.
///
/// The container half of [`IOMedia::read_arrow_field`], shared with the media
/// wrappers whose own schema read reads one leaf's bytes.
pub(crate) fn container_field(
    handle: &dyn IOBase,
    options: &RecordOptions,
) -> Result<crate::Field> {
    use crate::media::IORecordOptions;

    if let Some(field) = options.field() {
        return Ok(field.clone());
    }
    opened_field(open_container(handle, options)?, options)
}

/// `options`, or the handle's own where none were given: the one place an
/// absent record option set is resolved.
pub(crate) fn own_options<'o, M: IOMedia + ?Sized>(
    media: &M,
    options: Option<&'o RecordOptions>,
) -> Result<std::borrow::Cow<'o, RecordOptions>> {
    match options {
        Some(options) => Ok(std::borrow::Cow::Borrowed(options)),
        None => Ok(std::borrow::Cow::Owned(media.record_options()?)),
    }
}

/// A stream of batches as the serie it is: read as transport, so a write of
/// it through the serie doors hands the batches on untouched - over an
/// identity plan the reader itself.
pub(crate) fn arrow_serie(batches: crate::arrow::BatchReader) -> Result<crate::Serie> {
    Ok(crate::Serie::from(landed(batches)?))
}

/// A read's batches as the stream of record columns they land as, one
/// identity plan compiled from their schema.
pub(crate) fn landed(reader: crate::arrow::BatchReader) -> Result<crate::StreamChunkedSerie> {
    Ok(crate::StreamChunkedSerie::from_media_reader(
        crate::media::DEFAULT_ROOT_NAME,
        reader,
    )?)
}

/// Land a native record read under the root the media options publish.
pub(crate) fn landed_options(
    reader: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<crate::StreamChunkedSerie> {
    use crate::media::IORecordOptions;
    let declared = options.field();
    let name = declared
        .as_ref()
        .map_or_else(|| options.name(), crate::Field::name);
    Ok(crate::StreamChunkedSerie::from_media_reader(name, reader)?)
}

/// Read one structured document under its optional declared field.
pub(crate) fn read_document(
    handle: &(impl IOBase + ?Sized),
    options: Option<&RecordOptions>,
) -> Result<crate::Serie> {
    use crate::media::IORecordOptions;
    let field = options.and_then(IORecordOptions::field);
    crate::media::structured::read_arrow(handle, field.as_ref())
}

/// A document is replaced whole before its options or source are read.
pub(crate) fn require_document_mode(mode: crate::IOMode) -> Result<()> {
    if mode == crate::IOMode::Overwrite {
        return Ok(());
    }
    Err(crate::Error::InvalidRecord {
        path: smol_str::SmolStr::new_static("$.mode"),
        reason: smol_str::format_smolstr!(
            "a structured text document is one frame around its rows, so it is written \
             whole; expected overwrite, got {mode}"
        ),
    })
}

/// The structured-document publication behind the overwrite primitive.
pub(crate) fn overwrite_document(
    handle: &mut (impl IOBase + ?Sized),
    value: crate::Serie,
    options: Option<&RecordOptions>,
) -> Result<crate::IOResult> {
    use crate::media::IORecordOptions;
    let mut reader = crate::StreamChunkedSerie::from_serie(value)?;
    if let Some(field) = options.and_then(IORecordOptions::field) {
        let safe = options.is_none_or(IORecordOptions::safe);
        reader = reader.cast(&field, crate::ArrowCastOptions::new().with_safe(safe))?;
    }
    let rows =
        crate::media::structured::write_arrow(handle, reader, crate::text::Formatting::default())?;
    Ok(crate::IOResult::new(rows, rows))
}

pub(crate) fn require_serie_write_mode(mode: crate::IOMode) -> Result<()> {
    match mode {
        crate::IOMode::Overwrite | crate::IOMode::Append | crate::IOMode::Merge => Ok(()),
        crate::IOMode::ReadOnly | crate::IOMode::Random => Err(crate::Error::InvalidRecord {
            path: "$.mode".into(),
            reason: smol_str::format_smolstr!("write mode {mode} is not supported for a write"),
        }),
    }
}

/// The generic write under `mode`, its options resolved: the serie door of
/// that intent.
fn write_serie_as<M: IOMedia + ?Sized>(
    media: &mut M,
    value: crate::Serie,
    mode: crate::IOMode,
    options: &RecordOptions,
) -> Result<crate::IOResult> {
    // The mode is checked before the source is touched, so a specialized
    // intent cannot consume a one-shot stream before the intent and its key
    // settings are known to agree.
    let options = media.write_options(mode, options)?;
    require_serie_write_mode(mode)?;
    match mode {
        crate::IOMode::Overwrite => media.overwrite_serie(value, Some(&options)),
        crate::IOMode::Append => media.append_serie(value, Some(&options)),
        crate::IOMode::Merge => media.merge_serie(value, Some(&options)),
        crate::IOMode::ReadOnly | crate::IOMode::Random => {
            unreachable!("write mode checked before dispatch")
        }
    }
}

/// Record-oriented operations every [`IOBase`] handle exposes.
///
/// The trait deliberately has no storage primitives of its own. Implementors
/// return their [`IOBase`] view through hidden methods so the shared defaults
/// keep one storage abstraction and remain callable through trait objects.
///
/// Every write door - the overwrite, the append and the merge of each shape,
/// and the generic `write_*` beside them - answers an
/// [`IOResult`](crate::IOResult): the rows it read off its source, the rows
/// that reached the destination, and the rows between the two that the
/// options' `where` or a bound kept out.
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
    fn row_size(&self) -> Result<u64> {
        let options = dimension_options(self)?;
        let handle = self.as_io_base();
        if handle.is_container() {
            return container_row_size(handle, &options);
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
    fn column_size(&self) -> Result<usize> {
        use crate::media::IORecordOptions;

        let options = dimension_options(self)?;
        if let Some(field) = options.field() {
            return Ok(field.fields().len());
        }
        let handle = self.as_io_base();
        // Asked once and reused: on a store an unresolved location answers
        // this with a listing, and the two routes below want the same answer.
        let container = handle.is_container();
        if container && let Some(table) = crate::media::format::locate(handle)? {
            return table.column_size();
        }
        // Preserve the container route: its canonical field may include Hive
        // partition columns restored from paths across multiple leaves.
        if container {
            return Ok(self.read_arrow_field(&options)?.fields().len());
        }
        if handle.is_empty() && !matches!(options, RecordOptions::Text(_)) {
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
    fn record_options(&self) -> Result<RecordOptions> {
        let handle = self.as_io_base();
        if handle.is_container() {
            if let Some(table) = crate::media::format::locate(handle)? {
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

    /// Return the match key this resource states for its own rows: what a
    /// merge whose options name no
    /// [`merge_by`](crate::media::IORecordOptions::merge_by) matches on -
    /// the key left out, null, or `true` through
    /// [`set_merge_by_scalar`](crate::media::IORecordOptions::set_merge_by_scalar).
    ///
    /// Empty - the default - where the resource states none, as a leaf, a
    /// folder and a buffer do, so a merge naming no key is refused there
    /// naming `$.merge_by`. An Iceberg table answers its identity partition
    /// columns, then the columns its schema's `identifier-field-ids` name,
    /// read off its metadata with no data file opened.
    ///
    /// # Errors
    ///
    /// Returns the read of the metadata the key is stated in, or a stated
    /// key naming no column.
    fn merge_by(&self) -> Result<crate::Selector> {
        Ok(crate::Selector::all())
    }

    /// Return the options a write under `mode` runs under here: `options` as
    /// they stand, refused as
    /// [`RecordOptions::require_write_mode`] refuses them, except a merge
    /// whose options name no key, which runs keyed by this resource's own
    /// [`merge_by`](Self::merge_by) where it states one; a merge into a
    /// structured text document is refused naming the mode before any key
    /// is asked for, since a document is written whole.
    ///
    /// Every generic write door resolves its options through this once,
    /// before a one-shot source is pulled, and the bindings call it from
    /// their preflight. The answer borrows `options` wherever it does not
    /// re-key them, and never `self`.
    ///
    /// # Errors
    ///
    /// Returns the refusal of the mode or of the key, naming `$.merge_by`,
    /// or the error [`merge_by`](Self::merge_by) raises.
    #[doc(hidden)]
    fn write_options<'o>(
        &self,
        mode: crate::IOMode,
        options: &'o RecordOptions,
    ) -> Result<std::borrow::Cow<'o, RecordOptions>> {
        use crate::media::IORecordOptions;

        if mode == crate::IOMode::Merge {
            if crate::text::Format::from_media_type(self.as_io_base().media_type()).is_ok() {
                require_document_mode(mode)?;
            }
            if options.merge_by().is_empty() {
                let own = IOMedia::merge_by(self)?;
                if !own.is_empty() {
                    let mut keyed = options.clone();
                    keyed.set_merge_by(own);
                    return Ok(std::borrow::Cow::Owned(keyed));
                }
            }
        }
        options.require_write_mode(mode)?;
        Ok(std::borrow::Cow::Borrowed(options))
    }

    /// The medium's own state, which a caller who knows the medium
    /// downcasts: what no verb answers. A Parquet wrapper answers its
    /// `ParquetFooter`, the footer its `open` read, so a record read of it
    /// reads no byte of the file's end again; a wrapper forwards its inner
    /// handle's, and every other handle answers `None`.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// Read the canonical non-null Struct root Field of this resource.
    ///
    /// A declared schema is returned as it stands; otherwise this is the shape
    /// [`Self::read_arrow_reader`] reports, so the schema a caller reads
    /// and the batches a caller gets can never disagree. A leaf answers from
    /// its encoding's header or footer - an Arrow IPC schema message, an Avro
    /// header, a Parquet footer - typed by the plan its options state, the
    /// filter and the selection in the phases a read runs them in, so a
    /// `where` that does not bind still fails here and one over a column the
    /// `select` builds is typed after it; no reader is built and no row is
    /// read. A container answers as the table located in it or its leaves.
    ///
    /// # Errors
    ///
    /// Returns a read, decoding, or schema-projection failure.
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<crate::Field> {
        use crate::media::IORecordOptions;

        if let Some(field) = options.field() {
            return Ok(field.clone());
        }
        let handle = self.as_io_base();
        if handle.is_container() {
            return opened_field(open_container(handle, options)?, options);
        }
        options
            .plan()
            .field_from(&crate::iobase::leaf_field(handle, options)?.with_name(options.name()))
    }

    /// Read this resource's rows as a [`StreamChunkedSerie`](crate::StreamChunkedSerie):
    /// one record [`Serie`](crate::Serie) per batch. This is the one read
    /// every medium implements; [`read_arrow_reader`](Self::read_arrow_reader)
    /// is its transport face, so an encoding is decoded in exactly one place,
    /// and the result streams: one batch at a time, never a materialized
    /// vector.
    ///
    /// A record encoding - Arrow IPC, Parquet, Avro, plain text, delimited
    /// text - answers its batches, each landed as it is pulled. A structured
    /// text document - JSON, JSON Lines, YAML, TOML, XML - answers the one
    /// record column its rows parse into, because a document has no frame to
    /// read a prefix of, and reads only the declared field off the options.
    /// `options` absent is the handle's own: a record encoding answers its
    /// stored schema, a document names the root its own contents prove, and
    /// a container - a folder, a path ending in `/`, a glob, a table - reads
    /// as the table its leaves hold, under the encoding
    /// [`record_options`](Self::record_options) finds beneath it.
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
    /// rows rather than failing.
    ///
    /// The shaping order is fixed: declared schema, then partition filter,
    /// then the applied expressions, then
    /// [`max_row_size`](crate::media::IORecordOptions::max_row_size) and
    /// [`max_byte_size`](crate::media::IORecordOptions::max_byte_size)
    /// last - so a limit counts result rows, and a limit of ten with a filter
    /// means the first ten matching rows. A satisfied limit stops pulling, so
    /// the rest of the resource is never decoded.
    ///
    /// ```
    /// use yggdryl::{IOMedia, IOBase, Serie, Url, holder::Buffer};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut handle = Buffer::new()
    ///     .with_media_type(Url::from_str("file:///trades.jsonl")?.media_type());
    /// handle.write_all_bytes(b"{\"symbol\": \"AAPL\", \"size\": 100}\n")?;
    ///
    /// let columns = handle
    ///     .read_serie(None)?
    ///     .into_chunked_stream(None, None)?
    ///     .into_chunks()
    ///     .collect::<Result<Vec<Serie>, _>>()?;
    /// assert_eq!(columns.len(), 1);
    /// assert_eq!(columns[0].len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a listing, read, decoding, parse, inference, or cast failure,
    /// or an error naming the media type when it is neither a record encoding
    /// this build implements nor a structured text format.
    fn read_serie(&self, options: Option<&RecordOptions>) -> Result<crate::Serie> {
        read_serie_default(self, options)
    }

    /// Read this resource's rows as one [`BatchReader`](crate::arrow::BatchReader):
    /// [`read_serie`](Self::read_serie)'s transport face, each batch
    /// reconciled to the root and never landed - over an identity plan the
    /// encoding's own reader, handed back untouched.
    ///
    /// # Errors
    ///
    /// [`read_serie`](Self::read_serie)'s.
    fn read_arrow_reader(&self, options: &RecordOptions) -> Result<crate::arrow::BatchReader> {
        Ok(
            crate::StreamChunkedSerie::from_serie(self.read_serie(Some(options))?)?
                .into_arrow_reader(),
        )
    }

    /// Write a [`Serie`](crate::Serie) of any kind - a held column, a
    /// chunked one, a stream of rows or of record columns - as this
    /// resource's rows, under one explicit [`IOMode`](crate::IOMode): the
    /// generic write, which hands the rows to
    /// [`overwrite_serie`](Self::overwrite_serie),
    /// [`append_serie`](Self::append_serie) or
    /// [`merge_serie`](Self::merge_serie).
    ///
    /// The rows cross as the batches they already are
    /// ([`StreamChunkedSerie::from_serie`](crate::StreamChunkedSerie::from_serie)):
    /// a record column one batch, a chunk one each, a stream itself,
    /// nothing copied or re-landed, so every mode and every option applies.
    /// `options` absent is the handle's own: the encoding its media type
    /// names, a container's the table beneath it. A structured text document
    /// is one frame around every row it holds, so it is replaced whole, only
    /// [`IOMode::Overwrite`](crate::IOMode::Overwrite) applies, and of the
    /// options only the declared `field` - which the rows are cast onto -
    /// reaches it: a document has no frame to select, filter or bound within.
    ///
    /// What a write holds between publications is bounded by the process
    /// spill bound ([`SpillOptions::from_env`](crate::SpillOptions::from_env)):
    /// a cadence's held batches past it are spilled to disk, so a stream of
    /// any length is written under that bound plus one encoded file.
    ///
    /// ```
    /// use yggdryl::{DataType, IOMedia, IOMode, Scalar, Serie, Url, holder::Buffer};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut handle = Buffer::new()
    ///     .with_media_type(Url::from_str("file:///trades.arrows")?.media_type());
    /// let sizes = Serie::from_scalars(
    ///     DataType::Int64.required_field("size"),
    ///     [Scalar::from(100_i64), Scalar::from(250_i64)],
    /// )?;
    /// handle.write_serie(sizes.clone(), IOMode::Overwrite, None)?;
    /// let appended = handle.append_serie(sizes, None)?;
    /// assert_eq!((appended.read_rows, appended.written_rows, appended.skipped_rows), (2, 2, 0));
    /// let rows: usize = handle
    ///     .read_serie(None)?
    ///     .into_chunked_stream(None, None)?
    ///     .into_chunks()
    ///     .map(|batch| Ok::<_, yggdryl::Error>(batch?.len()))
    ///     .sum::<Result<_, _>>()?;
    /// assert_eq!(rows, 4);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a schema, value, encoding, or write failure, a run or a
    /// record holding an absent row, which no table states, an error naming
    /// the media type when it names no format, or an error naming the mode
    /// when a structured text document is asked for anything but an
    /// overwrite; merge requires non-empty match keys while the other modes
    /// refuse them.
    fn write_serie(
        &mut self,
        value: crate::Serie,
        mode: crate::IOMode,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        if crate::text::Format::from_media_type(self.as_io_base().media_type()).is_ok() {
            require_document_mode(mode)?;
            return self.overwrite_serie(value, options);
        }
        let options = own_options(self, options)?;
        write_serie_as(self, value, mode, &options)
    }

    /// Replace this resource's rows with `value`'s.
    ///
    /// This is the required publication hook each handle implements. The
    /// workspace implementations use [`super::overwrite_serie_default`] for
    /// byte and folder handles; table formats override it so one call is one
    /// native commit. `options` absent is the handle's own. A declared field
    /// is cast onto the incoming rows exactly once, followed by selection and
    /// completion onto the stored field; a `TRANSFORM:`, `PARTITION:` or
    /// `DIGEST:` declaration it carries fills no column.
    ///
    /// A folder routes each row to the leaf its partition values name, creating
    /// the `column=value` directory when the layout has one and no leaf holds
    /// that value yet. A folder holding a table format commits instead: an
    /// Iceberg table writes a snapshot per cadence, the first replacing the
    /// addressed partitions and the rest appending.
    ///
    /// A limited overwrite truncates data the caller offered:
    /// [`max_row_size`](crate::media::IORecordOptions::max_row_size) and
    /// [`max_byte_size`](crate::media::IORecordOptions::max_byte_size) bound
    /// the incoming rows exactly as they bound a read, and what they cut off
    /// is never pulled from them. A match key is refused: use
    /// [`merge_serie`](Self::merge_serie) for that intent.
    ///
    /// [`commit_batch_num`](crate::media::IORecordOptions::commit_batch_num)
    /// changes publication, not shaping: the incoming rows are cast and
    /// limited once, then cut into cadences of that many batches, no batch
    /// ever split. The first cadence overwrites and every later one appends.
    /// Successful prefixes remain visible if a later cadence fails; zero is
    /// rejected before the source is pulled. With no cadence, a leaf, a
    /// folder and an Iceberg table publish once at the end - the table
    /// holding every partition's rows under the process spill bound until
    /// the source ends, so an overwrite of any length is one atomic
    /// replacement, and `commit_batch_num` paces a stream whose rows would
    /// outgrow the spill folder.
    ///
    /// The answer counts the whole write, every cadence included: the rows
    /// pulled from `value`, the rows the destination took, and the rows a
    /// `where` kept out or a bound cut off the batch it fell in. It says
    /// nothing of what was replaced.
    ///
    /// # Errors
    ///
    /// Returns a listing, read, schema, cast, encoding, or write failure.
    fn overwrite_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult>;

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
    #[doc(hidden)]
    fn overwrite_prepared_serie(
        &mut self,
        value: crate::StreamChunkedSerie,
        options: &RecordOptions,
    ) -> Result<()> {
        crate::iobase::leaf_writer(self.as_io_base_mut(), value.into_arrow_reader(), options)
    }

    /// Add `value`'s rows after the rows this resource holds.
    ///
    /// The encodings here are whole-value containers - an Arrow IPC stream and a
    /// Parquet file each carry one schema and one footer - so appending means
    /// reading what is there, adding to it, and rewriting. The current rows are
    /// read as the declared schema when there is one and as the stored schema
    /// otherwise, a resource holding nothing is skipped rather than decoded, and
    /// the incoming rows are cast to that same shape, so a caller may append
    /// data whose schema merely *fits*. Both sides stream: the stored batches
    /// are chained ahead of the incoming ones and encoded as they arrive, so
    /// neither is collected. `options` absent is the handle's own.
    ///
    /// A folder appends into each partition the incoming rows name, leaving
    /// every other partition untouched. A folder holding a table format appends
    /// the way that format does: an Iceberg table writes new data files and
    /// commits a snapshot per cadence that keeps every manifest the last one
    /// had, so nothing already stored is read, rewritten, or even listed.
    ///
    /// A limited write truncates data the caller offered: an append is a
    /// write, so
    /// [`max_row_size`](crate::media::IORecordOptions::max_row_size) and
    /// [`max_byte_size`](crate::media::IORecordOptions::max_byte_size) bound
    /// the incoming rows here exactly as they do on
    /// [`overwrite_serie`](Self::overwrite_serie), and a limit combined with
    /// a non-empty match key is refused the same way. `commit_batch_num`
    /// retains append intent for every bounded publication; successful
    /// prefixes remain visible after a later failure. With no cadence a leaf,
    /// a folder and an Iceberg table publish once at the end, the table
    /// holding every partition's rows under the process spill bound until
    /// then.
    ///
    /// # Errors
    ///
    /// Returns a listing, read, cast, encoding, or write failure. A leaf or a
    /// folder with no commit cadence stays unchanged until the rewrite is
    /// complete; an Iceberg table with none, and any destination with a
    /// positive cadence, keeps the commits completed before the failure
    /// published.
    fn append_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        crate::iobase::append_serie_default(self.as_io_base_mut(), value, options)
    }

    /// Merge `value`'s rows into this resource by the declared match key.
    ///
    /// The match key is required. Stored rows are indexed because a stream
    /// cannot be rewound; the incoming side remains streaming and is folded in
    /// one batch at a time. The resulting stream is published through the
    /// implementor's [`overwrite_serie`](Self::overwrite_serie) after the
    /// declared field has been popped from a cloned options value, so the
    /// already-cast rows are never cast to that field twice. `options` absent
    /// is the handle's own.
    ///
    /// A folder holding an Iceberg table merges the way the table does: the
    /// partition columns lead the match key, so a key naming nothing beyond
    /// them replaces the partitions the rows fall in, and otherwise only the
    /// data files whose statistics say they can hold an incoming key are
    /// read, the rest carried forward untouched. With no cadence it commits
    /// once when the source ends, and under one every commit merges by the
    /// key: a keyed merge upserts per commit, and a merge keyed by the
    /// partition alone replaces a partition on the first commit of the write
    /// that reaches it and appends to it on every later one, so no commit
    /// drops the rows an earlier one wrote.
    ///
    /// # Errors
    ///
    /// Returns a read, cast, merge, encoding, or write failure. An empty match
    /// key is refused: use overwrite or append when rows have no identity.
    /// `commit_batch_num`, and an Iceberg table's own cadence where none is
    /// stated, retain merge intent for every bounded publication; successful
    /// prefixes remain visible after a later failure.
    fn merge_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        crate::iobase::merge_serie_default(self.as_io_base_mut(), value, options)
    }

    /// Write a batch stream using one explicit [`IOMode`](crate::IOMode): the
    /// stream read as a [`StreamChunkedSerie`](crate::StreamChunkedSerie) -
    /// transport, nothing landed - and handed to the serie door of that
    /// intent.
    ///
    /// The canonical argument order is input, mode, options; the Python and
    /// JavaScript bindings keep that order and infer/cast their input before
    /// redirecting here.
    ///
    /// # Errors
    ///
    /// Returns the selected intent's validation, cast, read, or publication
    /// failure. In particular, merge requires non-empty match keys while the
    /// other modes refuse them.
    fn write_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        mode: crate::IOMode,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.write_serie(arrow_serie(batches)?, mode, Some(options))
    }

    /// Replace this resource's rows with every batch `batches` yields:
    /// [`overwrite_serie`](Self::overwrite_serie) over the stream, read as
    /// transport.
    ///
    /// # Errors
    ///
    /// [`overwrite_serie`](Self::overwrite_serie)'s.
    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.overwrite_serie(arrow_serie(batches)?, Some(options))
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
    fn overwrite_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        let schema = batch.schema();
        self.overwrite_arrow_reader(crate::arrow::batch_reader(schema, [batch]), options)
    }

    /// Write one held Arrow batch using one explicit intent.
    ///
    /// The selected same-shape adapter remains authoritative: it widens the
    /// batch into a one-item reader without copying.
    ///
    /// # Errors
    ///
    /// Returns the selected held-batch adapter's validation, cast, encoding,
    /// or publication failure.
    fn write_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        mode: crate::IOMode,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        let schema = batch.schema();
        self.write_arrow_reader(crate::arrow::batch_reader(schema, [batch]), mode, options)
    }

    /// Add every batch `batches` yields after the rows this resource holds:
    /// [`append_serie`](Self::append_serie) over the stream, read as
    /// transport.
    ///
    /// # Errors
    ///
    /// [`append_serie`](Self::append_serie)'s.
    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.append_serie(arrow_serie(batches)?, Some(options))
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
    fn append_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        let schema = batch.schema();
        self.append_arrow_reader(crate::arrow::batch_reader(schema, [batch]), options)
    }

    /// Merge every batch `batches` yields into this resource by the declared
    /// match key: [`merge_serie`](Self::merge_serie) over the stream, read as
    /// transport.
    ///
    /// # Errors
    ///
    /// [`merge_serie`](Self::merge_serie)'s.
    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.merge_serie(arrow_serie(batches)?, Some(options))
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
    fn merge_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
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
    /// [`batch_row_size`](crate::media::IORecordOptions::batch_row_size); a
    /// commit cadence counts those batches and never cuts one, so conversion
    /// never reads past the next publication. The exact
    /// declared schema reaches the reader primitive unchanged. Its one shaping
    /// seam applies selection, limits, and stored-shape completion, and its
    /// exact-schema fast path returns these arrays without rebuilding them.
    ///
    /// ```
    /// use yggdryl::media::IORecordOptions;
    /// use yggdryl::{IOMedia, StructType, holder::Buffer};
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
    /// let field = DataType::from(StructType::from_fields([
    ///     DataType::Int32.required_field("id"),
    ///     DataType::utf8().required_field("symbol"),
    /// ])?)
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
    fn overwrite_records<I, R>(
        &mut self,
        records: I,
        options: &RecordOptions,
    ) -> Result<crate::IOResult>
    where
        Self: Sized,
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<crate::Scalar>,
        R::Error: Into<crate::Error>,
    {
        use crate::media::IORecordOptions;

        let options = self.write_options(crate::IOMode::Overwrite, options)?;
        options.require_commit_batch_num()?;
        options.require_num_threads()?;
        let field = options.require_field()?.clone();
        let batches = crate::arrow::rows::reader(
            &field,
            records,
            options.batch_row_size(),
            options.batch_byte_size(),
            options.max_row_size(),
        )?;
        self.overwrite_arrow_reader(batches, &options)
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
    fn append_records<I, R>(
        &mut self,
        records: I,
        options: &RecordOptions,
    ) -> Result<crate::IOResult>
    where
        Self: Sized,
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<crate::Scalar>,
        R::Error: Into<crate::Error>,
    {
        use crate::media::IORecordOptions;

        let options = self.write_options(crate::IOMode::Append, options)?;
        options.require_commit_batch_num()?;
        options.require_num_threads()?;
        let field = options.require_field()?.clone();
        let batches = crate::arrow::rows::reader(
            &field,
            records,
            options.batch_row_size(),
            options.batch_byte_size(),
            options.max_row_size(),
        )?;
        self.append_arrow_reader(batches, &options)
    }

    /// Merge native row values into this resource by explicit keys.
    ///
    /// This is the row-by-row adapter over
    /// [`merge_arrow_reader`](Self::merge_arrow_reader). `merge_by` must
    /// name at least one key, or this resource must state its own
    /// ([`merge_by`](Self::merge_by)); an empty iterator is a no-op once
    /// that intent has been validated.
    ///
    /// # Errors
    ///
    /// Returns the same field, row-conversion, key, cast, encoding, and write
    /// failures as [`overwrite_records`](Self::overwrite_records) and
    /// [`merge_arrow_reader`](Self::merge_arrow_reader).
    fn merge_records<I, R>(
        &mut self,
        records: I,
        options: &RecordOptions,
    ) -> Result<crate::IOResult>
    where
        Self: Sized,
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<crate::Scalar>,
        R::Error: Into<crate::Error>,
    {
        use crate::media::IORecordOptions;

        let options = self.write_options(crate::IOMode::Merge, options)?;
        options.require_commit_batch_num()?;
        options.require_num_threads()?;
        let field = options.require_field()?.clone();
        let batches = crate::arrow::rows::reader(
            &field,
            records,
            options.batch_row_size(),
            options.batch_byte_size(),
            options.max_row_size(),
        )?;
        self.merge_arrow_reader(batches, &options)
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
    fn write_records<I, R>(
        &mut self,
        records: I,
        mode: crate::IOMode,
        options: &RecordOptions,
    ) -> Result<crate::IOResult>
    where
        Self: Sized,
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<crate::Scalar>,
        R::Error: Into<crate::Error>,
    {
        let options = self.write_options(mode, options)?;
        match mode {
            crate::IOMode::Overwrite => self.overwrite_records(records, &options),
            crate::IOMode::Append => self.append_records(records, &options),
            crate::IOMode::Merge => self.merge_records(records, &options),
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
pub(crate) fn dimension_options<M: IOMedia + ?Sized>(media: &M) -> Result<RecordOptions> {
    media.record_options().map(dimensions)
}

/// `options` with the five clauses that narrow a read taken off - the
/// filter, the selection, the row bound, the row offset and the byte bound -
/// the one owner of that list, which a dimension and an edited media serie
/// read under.
pub(crate) fn dimensions(mut options: RecordOptions) -> RecordOptions {
    use crate::media::IORecordOptions;

    options.set_filter(crate::Filter::always_true());
    options.set_select(crate::Selector::all());
    options.set_max_row_size(None);
    options.set_row_offset(None);
    options.set_max_byte_size(None);
    options
}

/// Count a container's rows: a located table format answers from its
/// metadata, and anything else sums the leaves holding the encoding
/// `options` names, each counted as the leaf it is.
///
/// The one container count, shared by the [`IOMedia::row_size`] default and
/// the media wrappers whose own count reads one leaf's bytes.
pub(crate) fn container_row_size(handle: &dyn IOBase, options: &RecordOptions) -> Result<u64> {
    if let Some(table) = crate::media::format::locate(handle)? {
        return table.row_size();
    }
    let mut rows = 0_u64;
    for child in crate::media::partition::record_parts(handle, options.mime_type())? {
        rows = add_rows(rows, crate::iobase::leaf_row_size(&child?, options)?)?;
    }
    Ok(rows)
}

/// Add one metadata row count without allowing an aggregate to wrap.
fn add_rows(total: u64, rows: u64) -> Result<u64> {
    total
        .checked_add(rows)
        .ok_or_else(|| crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$"),
            reason: smol_str::SmolStr::new_static("logical row count exceeds u64::MAX"),
        })
}

/// Implement the default media contract for an [`IOBase`] value.
///
/// Use this inside an `impl IOMedia for Type` block when record operations
/// should run on the value itself rather than being forwarded to an inner
/// handle.
#[macro_export]
macro_rules! impl_default_iomedia {
    () => {
        fn as_io_base(&self) -> &dyn $crate::IOBase {
            self
        }

        fn as_io_base_mut(&mut self) -> &mut dyn $crate::IOBase {
            self
        }

        fn overwrite_serie(
            &mut self,
            value: $crate::Serie,
            options: Option<&$crate::media::RecordOptions>,
        ) -> $crate::Result<$crate::IOResult> {
            $crate::overwrite_serie_default(self, value, options)
        }
    };
}

/// The media forwarding bodies used by [`delegate_iomedia!`].
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

        fn merge_by(&self) -> $crate::Result<$crate::Selector> {
            $crate::IOMedia::merge_by(&self.$handle)
        }

        fn read_arrow_field(
            &self,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<$crate::Field> {
            $crate::IOMedia::read_arrow_field(&self.$handle, options)
        }

        // Forwarded, because a handle can answer its rows other than through
        // its bytes - an HTTP request walks the pages of a paginated document.
        fn read_serie(
            &self,
            options: Option<&$crate::media::RecordOptions>,
        ) -> $crate::Result<$crate::Serie> {
            $crate::IOMedia::read_serie(&self.$handle, options)
        }

        fn overwrite_serie(
            &mut self,
            value: $crate::Serie,
            options: Option<&$crate::media::RecordOptions>,
        ) -> $crate::Result<$crate::IOResult> {
            $crate::IOMedia::overwrite_serie(&mut self.$handle, value, options)
        }

        fn overwrite_prepared_serie(
            &mut self,
            value: $crate::StreamChunkedSerie,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()> {
            $crate::IOMedia::overwrite_prepared_serie(&mut self.$handle, value, options)
        }

        fn append_serie(
            &mut self,
            value: $crate::Serie,
            options: Option<&$crate::media::RecordOptions>,
        ) -> $crate::Result<$crate::IOResult> {
            $crate::IOMedia::append_serie(&mut self.$handle, value, options)
        }

        fn merge_serie(
            &mut self,
            value: $crate::Serie,
            options: Option<&$crate::media::RecordOptions>,
        ) -> $crate::Result<$crate::IOResult> {
            $crate::IOMedia::merge_serie(&mut self.$handle, value, options)
        }

        fn overwrite_records<I, R>(
            &mut self,
            records: I,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<$crate::IOResult>
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
        ) -> $crate::Result<$crate::IOResult>
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
        ) -> $crate::Result<$crate::IOResult>
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

/// Every [`IOMedia`] verb forwarded to a handle resolved on the first call
/// that needs one, as `__delegate_resolved_iobase!` forwards the byte verbs;
/// the two `as_io_base` doors answer `self`.
#[doc(hidden)]
#[macro_export]
macro_rules! __delegate_resolved_iomedia {
    ($get:ident, $get_mut:ident) => {
        fn as_io_base(&self) -> &dyn $crate::IOBase {
            self
        }

        fn as_io_base_mut(&mut self) -> &mut dyn $crate::IOBase {
            self
        }

        fn row_size(&self) -> $crate::Result<u64> {
            $crate::IOMedia::row_size(self.$get()?)
        }

        fn column_size(&self) -> $crate::Result<usize> {
            $crate::IOMedia::column_size(self.$get()?)
        }

        fn record_options(&self) -> $crate::Result<$crate::media::RecordOptions> {
            $crate::IOMedia::record_options(self.$get()?)
        }

        fn merge_by(&self) -> $crate::Result<$crate::Selector> {
            $crate::IOMedia::merge_by(self.$get()?)
        }

        // A handle that cannot resolve holds no medium's state; the verb
        // that reads it next reports why.
        fn as_any(&self) -> Option<&dyn ::std::any::Any> {
            self.$get()
                .ok()
                .and_then(|held| $crate::IOMedia::as_any(held))
        }

        fn read_arrow_field(
            &self,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<$crate::Field> {
            $crate::IOMedia::read_arrow_field(self.$get()?, options)
        }

        fn read_serie(
            &self,
            options: Option<&$crate::media::RecordOptions>,
        ) -> $crate::Result<$crate::Serie> {
            $crate::IOMedia::read_serie(self.$get()?, options)
        }

        fn overwrite_serie(
            &mut self,
            value: $crate::Serie,
            options: Option<&$crate::media::RecordOptions>,
        ) -> $crate::Result<$crate::IOResult> {
            $crate::IOMedia::overwrite_serie(self.$get_mut()?, value, options)
        }

        fn overwrite_prepared_serie(
            &mut self,
            value: $crate::StreamChunkedSerie,
            options: &$crate::media::RecordOptions,
        ) -> $crate::Result<()> {
            $crate::IOMedia::overwrite_prepared_serie(self.$get_mut()?, value, options)
        }

        fn append_serie(
            &mut self,
            value: $crate::Serie,
            options: Option<&$crate::media::RecordOptions>,
        ) -> $crate::Result<$crate::IOResult> {
            $crate::IOMedia::append_serie(self.$get_mut()?, value, options)
        }

        fn merge_serie(
            &mut self,
            value: $crate::Serie,
            options: Option<&$crate::media::RecordOptions>,
        ) -> $crate::Result<$crate::IOResult> {
            $crate::IOMedia::merge_serie(self.$get_mut()?, value, options)
        }
    };
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

        fn as_any(&self) -> Option<&dyn ::std::any::Any> {
            $crate::IOMedia::as_any(&self.$handle)
        }
    };
}

/// The common native read behind the generic serie door.
pub(crate) fn read_serie_default<M: IOMedia + ?Sized>(
    media: &M,
    options: Option<&RecordOptions>,
) -> Result<crate::Serie> {
    let handle = media.as_io_base();
    // A retained plain-text reader owns its encoding even when its bytes
    // resemble a structured document (for example a bracketed log header).
    if !matches!(options, Some(RecordOptions::Text(_)))
        && crate::text::Format::from_media_type(handle.media_type()).is_ok()
    {
        return read_document(handle, options);
    }
    read_record_serie(media, options)
}

/// Read a retained record encoding, whose owner already chose its format.
pub(crate) fn read_record_serie<M: IOMedia + ?Sized>(
    media: &M,
    options: Option<&RecordOptions>,
) -> Result<crate::Serie> {
    use crate::media::IORecordOptions;

    let handle = media.as_io_base();
    let options = own_options(media, options)?;
    let container = handle.is_container();
    if !container {
        let rows = options
            .codec()
            .read_stream(handle, options.declared(), &options)?;
        if let Some(rows) = rows {
            let rows = options.apply_stream(rows)?;
            if options.batch_row_size().is_some() || options.batch_byte_size().is_some() {
                return rows
                    .into_chunked_stream(options.batch_row_size(), options.batch_byte_size())
                    .map(crate::Serie::from)
                    .map_err(Into::into);
            }
            return Ok(crate::Serie::from(rows));
        }
    }
    let opened = if container {
        open_container(handle, &options)?
    } else {
        Opened::Reader(crate::iobase::leaf_reader(handle, &options)?)
    };
    let reader = match opened {
        // The table pushes the clauses into its scan plan and wraps the
        // selector and the limit itself: the reader is complete.
        Opened::Table(table) => table.read(&options)?,
        Opened::Reader(reader) => {
            options.limit_arrow_reader(options.apply_arrow_expressions(reader)?)?
        }
    };
    landed_options(reader, &options).map(crate::Serie::from)
}
