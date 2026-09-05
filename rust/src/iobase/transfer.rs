//! Arrow record transfer through [`IOBase`](super::IOBase).

use super::IOBase;
use crate::media::RecordOptions;
use crate::{Error, Result};

/// The default append implementation after an encoding-specific boundary has
/// validated its option variant.
///
/// Stateful media override the trait method only to perform that validation
/// before the source reader is pulled, then call this shared implementation.
#[cfg(feature = "arrow")]
pub(crate) fn append_arrow_reader_default(
    handle: &mut (impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    use crate::media::IORecordOptions;

    options.require_write_mode(crate::IOMode::Append)?;
    let commit_row_size = options.require_commit_row_size()?;
    options.require_write_limits()?;
    if options.write_limit_is_zero() {
        return Ok(());
    }
    // An empty append is a true no-op: discover it before asking the handle
    // whether it is a table or folder, so no location probe or listing runs.
    let Some(batches) = non_empty_arrow_reader(batches)? else {
        return Ok(());
    };
    let container = handle.is_container();
    if container {
        #[cfg(feature = "iceberg")]
        if let Some(mut table) = crate::media::iceberg::located(handle)? {
            return table.append_arrow_reader(batches, options);
        }
        return append_arrow_reader_folder(handle, batches, options, commit_row_size);
    }
    let (batches, delegated, target) = prepare_leaf_arrow_write(handle, batches, options)?;
    let Some(batches) = non_empty_arrow_reader(batches)? else {
        return Ok(());
    };
    if commit_row_size.is_some() {
        for commit in options.commit_arrow_readers(batches)? {
            match &target {
                Some(target) => append_leaf_onto(handle, commit?, &delegated, target)?,
                None => append_leaf(handle, commit?, &delegated)?,
            }
        }
        return Ok(());
    }
    match target {
        Some(target) => append_leaf_onto(handle, batches, &delegated, &target),
        None => append_leaf(handle, batches, &delegated),
    }
}

/// The default merge implementation after an encoding-specific boundary has
/// validated its option variant.
#[cfg(feature = "arrow")]
pub(crate) fn merge_arrow_reader_default(
    handle: &mut (impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    use crate::media::IORecordOptions;

    options.require_write_mode(crate::IOMode::Merge)?;
    let commit_row_size = options.require_commit_row_size()?;
    options.require_write_limits()?;
    // Key and limit intent is deterministic and has already been validated;
    // only then may an empty merge end without touching its destination.
    let Some(batches) = non_empty_arrow_reader(batches)? else {
        return Ok(());
    };
    let container = handle.is_container();
    if container {
        #[cfg(feature = "iceberg")]
        if let Some(mut table) = crate::media::iceberg::located(handle)? {
            return table.merge_arrow_reader(batches, options);
        }
        return merge_arrow_reader_folder(handle, batches, options, commit_row_size);
    }
    let (batches, delegated, target) = prepare_leaf_arrow_write(handle, batches, options)?;
    let Some(batches) = non_empty_arrow_reader(batches)? else {
        return Ok(());
    };
    if commit_row_size.is_some() {
        for commit in options.commit_arrow_readers(batches)? {
            match &target {
                Some(target) => merge_leaf_onto(
                    handle,
                    commit?,
                    &delegated,
                    options.merge_by_names(),
                    target,
                )?,
                None => merge_leaf(handle, commit?, &delegated, options.merge_by_names())?,
            }
        }
        return Ok(());
    }
    match target {
        Some(target) => merge_leaf_onto(
            handle,
            batches,
            &delegated,
            options.merge_by_names(),
            &target,
        ),
        None => merge_leaf(handle, batches, &delegated, options.merge_by_names()),
    }
}

/// The common overwrite implementation for byte and folder handles.
///
/// [`crate::IOMedia::overwrite_arrow_reader`] is required so a media or table format
/// can make publication one native operation. Implementations whose only
/// publication primitive is the byte surface call this function; it performs
/// all generic shaping and reaches exactly one encoding writer.
///
/// # Errors
///
/// Returns a field, cast, listing, encoding, or write failure. A non-empty
/// match key is refused because overwrite never guesses merge intent.
#[cfg(feature = "arrow")]
#[doc(hidden)]
pub fn overwrite_arrow_reader_default(
    handle: &mut (impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    overwrite_arrow_reader_default_with_field(handle, batches, options).map(|_| ())
}

/// Run the default overwrite and return the logical field actually published.
///
/// Stateful media use this to refresh an already-open metadata cache without
/// rereading the encoded value. The field is resolved by the same shaping pass
/// that consumes `batches`: declared-field casting and selection happen once,
/// then an existing stored field completes the result. `None` is reserved for
/// a table-format redirection whose own commit owns its metadata cache.
#[cfg(feature = "arrow")]
pub(crate) fn overwrite_arrow_reader_default_with_field(
    handle: &mut (impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<Option<crate::Field>> {
    use crate::media::IORecordOptions;

    options.require_write_mode(crate::IOMode::Overwrite)?;
    let commit_row_size = options.require_commit_row_size()?;
    let container = handle.is_container();
    if container {
        #[cfg(feature = "iceberg")]
        if let Some(mut table) = crate::media::iceberg::located(handle)? {
            table.overwrite_arrow_reader(batches, options)?;
            return Ok(None);
        }
        return overwrite_arrow_reader_folder(handle, batches, options, commit_row_size).map(Some);
    }
    let (batches, delegated, target) = prepare_leaf_arrow_write(handle, batches, options)?;
    let schema = batches.schema();
    let published = target
        .clone()
        .or(Some(crate::arrow::field_from_arrow_schema(
            delegated.name(),
            schema.as_ref(),
        )?));
    if commit_row_size.is_some() {
        let mut commits = options.commit_arrow_readers(batches)?;
        let Some(first) = commits.next() else {
            // Overwrite is the one intent for which an empty input still
            // publishes its shaped schema and clears the prior rows.
            handle.overwrite_prepared_arrow_reader(
                crate::arrow::batch_reader(schema, []),
                &delegated,
            )?;
            return Ok(published);
        };
        handle.overwrite_prepared_arrow_reader(first?, &delegated)?;
        // Replacing every cadence would retain only the last one. Once the
        // first prefix is visible, later overwrite cadences are appends.
        for commit in commits {
            match &target {
                Some(target) => append_leaf_onto(handle, commit?, &delegated, target)?,
                None => append_leaf(handle, commit?, &delegated)?,
            }
        }
        return Ok(published);
    }
    handle.overwrite_prepared_arrow_reader(batches, &delegated)?;
    Ok(published)
}

/// Append through one folder routing plan shared by every publication cadence.
#[cfg(feature = "arrow")]
fn append_arrow_reader_folder(
    folder: &(impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
    commit_row_size: Option<usize>,
) -> Result<()> {
    let mut writer = crate::media::partition::FolderWriter::new(folder, options)?;
    let (batches, delegated, declared) = prepare_arrow_write(batches, options)?;
    let Some(batches) = non_empty_arrow_reader(batches)? else {
        return Ok(());
    };
    writer.set_options(routing_options(delegated, declared))?;
    if commit_row_size.is_some() {
        for commit in options.commit_arrow_readers(batches)? {
            writer.append(folder, commit?)?;
        }
        return Ok(());
    }
    writer.append(folder, batches)
}

/// Merge through one folder routing plan shared by every publication cadence.
#[cfg(feature = "arrow")]
fn merge_arrow_reader_folder(
    folder: &(impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
    commit_row_size: Option<usize>,
) -> Result<()> {
    // Layout resolves before shaping or mutation because it decides whether
    // at least one merge key remains inside each leaf. The top-level no-op
    // peek has retained the first row-bearing batch without advancing past it.
    let mut writer = crate::media::partition::FolderWriter::new(folder, options)?;
    let (batches, delegated, declared) = prepare_arrow_write(batches, options)?;
    let Some(batches) = non_empty_arrow_reader(batches)? else {
        return Ok(());
    };
    writer.set_options(routing_options(delegated, declared))?;
    if commit_row_size.is_some() {
        for commit in options.commit_arrow_readers(batches)? {
            writer.merge(folder, commit?)?;
        }
        return Ok(());
    }
    writer.merge(folder, batches)
}

/// Overwrite through one folder routing plan shared by every publication cadence.
#[cfg(feature = "arrow")]
fn overwrite_arrow_reader_folder(
    folder: &(impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
    commit_row_size: Option<usize>,
) -> Result<crate::Field> {
    use crate::media::IORecordOptions;

    let mut writer = crate::media::partition::FolderWriter::new(folder, options)?;
    let (batches, delegated, declared) = prepare_arrow_write(batches, options)?;
    let schema = batches.schema();
    let published = crate::arrow::field_from_arrow_schema(delegated.name(), schema.as_ref())?;
    writer.set_options(routing_options(delegated, declared))?;
    if commit_row_size.is_none() {
        writer.overwrite(folder, batches)?;
        return Ok(published);
    }

    let mut commits = options.commit_arrow_readers(batches)?;
    let Some(first) = commits.next() else {
        writer.overwrite(folder, crate::arrow::batch_reader(schema, []))?;
        return Ok(published);
    };
    writer.overwrite(folder, first?)?;
    // Only the first cadence replaces the addressed tree. Every later prefix
    // extends the same top-level overwrite using the routing plan above.
    for commit in commits {
        writer.append(folder, commit?)?;
    }
    Ok(published)
}

/// Shape one incoming write stream and return options safe for delegation.
///
/// The declared field and selection are applied before the limits. The field
/// is then *taken* from the clone, and every other consumed shaping option is
/// cleared - including `commit_row_size` - so a default append or merge can
/// publish through an implementor's required overwrite hook without applying
/// an incoming-only transform to the stored rows, splitting recursively, or
/// casting the incoming rows twice.
#[cfg(feature = "arrow")]
pub(crate) fn prepare_arrow_write(
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<(
    crate::arrow::BatchReader,
    RecordOptions,
    Option<crate::Field>,
)> {
    prepare_arrow_write_onto(batches, options, None)
}

/// Shape one incoming stream and safely complete it onto a stored field once.
///
/// Table formats use this seam before splitting publication cadences. Their
/// native commit may defensively inspect the exact shape again, but every
/// declared cast, selection, limit, and safe stored-field completion has
/// already happened here over the one streaming reader.
#[cfg(feature = "arrow")]
pub(crate) fn prepare_arrow_write_onto(
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
    existing: Option<&crate::Field>,
) -> Result<(
    crate::arrow::BatchReader,
    RecordOptions,
    Option<crate::Field>,
)> {
    use crate::media::IORecordOptions;

    let batches = options.cast_arrow_reader(batches, existing)?;
    let batches = options.limit_arrow_reader(batches)?;
    let mut delegated = options.clone();
    let declared = delegated.take_field();
    delegated.set_select_by_names(Vec::new());
    delegated.set_max_row_size(None);
    delegated.set_max_byte_size(None);
    delegated.set_commit_row_size(None);
    Ok((batches, delegated, declared))
}

/// Shape one leaf stream onto a target resolved exactly once for the write.
///
/// A stored field completes the cast before global limits are applied. A
/// missing leaf takes the shaped reader's field as its target; text remains
/// schema-less and uses its native append implementation. The returned
/// options have every incoming-only transform removed and are safe for the
/// prepared publication hook.
#[cfg(feature = "arrow")]
fn prepare_leaf_arrow_write(
    handle: &(impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<(
    crate::arrow::BatchReader,
    RecordOptions,
    Option<crate::Field>,
)> {
    use crate::media::IORecordOptions;

    let stored = if matches!(options, RecordOptions::Text(_)) {
        None
    } else {
        stored_field(handle, options)?
    };
    let (batches, delegated, _) = prepare_arrow_write_onto(batches, options, stored.as_ref())?;
    let target = match stored {
        Some(stored) => Some(stored),
        None if matches!(options, RecordOptions::Text(_)) => None,
        None => Some(crate::arrow::field_from_arrow_schema(
            delegated.name(),
            batches.schema().as_ref(),
        )?),
    };
    Ok((batches, delegated, target))
}

/// One resumable, cadence-bounded Arrow write used by asynchronous bindings.
///
/// The session owns option shaping, global row/byte counters, the incomplete
/// cadence, and any folder or table routing plan. It deliberately does not
/// borrow the destination: a binding may leave Rust while awaiting its next
/// source chunk, then temporarily pass the same handle back to [`push`](Self::push)
/// or [`finish`](Self::finish). Complete cadences publish synchronously before
/// either method returns; [`abort`](Self::abort) drops only the unpublished
/// remainder.
///
/// This is hidden because it is a narrow runtime bridge, not another write
/// operation. Its mode is the same public [`crate::IOMode`] accepted by the
/// generic media entry points.
#[cfg(feature = "arrow")]
#[doc(hidden)]
pub struct ArrowWriteSession {
    mode: crate::IOMode,
    options: RecordOptions,
    delegated: RecordOptions,
    declared: Option<crate::Field>,
    limit: crate::media::WriteLimitState,
    commit_row_size: usize,
    input_schema: Option<arrow_schema::SchemaRef>,
    shaped_schema: Option<arrow_schema::SchemaRef>,
    buffer: Option<crate::media::CommitBuffer>,
    target: Option<ArrowWriteTarget>,
    published: bool,
    input_complete: bool,
    terminal: bool,
}

/// Destination state that must stay stable while an async source is awaited.
#[cfg(feature = "arrow")]
enum ArrowWriteTarget {
    /// A schema-bearing leaf after its one stable target is resolved.
    Leaf { stored: crate::Field },
    /// A missing schema-bearing leaf before the input schema is shaped.
    EmptyLeaf,
    /// Text lines have no stored field; append remains their native operation.
    TextLeaf,
    Folder {
        writer: Box<crate::media::partition::FolderWriter>,
    },
    #[cfg(feature = "iceberg")]
    Iceberg {
        located: Box<crate::media::iceberg::Located>,
        stored: crate::Field,
    },
}

#[cfg(feature = "arrow")]
impl ArrowWriteSession {
    /// Start an overwrite session without touching a destination or source.
    pub fn overwrite(options: &RecordOptions) -> Result<Self> {
        Self::new(crate::IOMode::Overwrite, options)
    }

    /// Start an append session without touching a destination or source.
    pub fn append(options: &RecordOptions) -> Result<Self> {
        Self::new(crate::IOMode::Append, options)
    }

    /// Start a merge session without touching a destination or source.
    pub fn merge(options: &RecordOptions) -> Result<Self> {
        Self::new(crate::IOMode::Merge, options)
    }

    /// Start a session for one explicit write mode.
    pub fn new(mode: crate::IOMode, options: &RecordOptions) -> Result<Self> {
        use crate::media::IORecordOptions;

        options.require_write_mode(mode)?;
        let commit_row_size =
            options
                .require_commit_row_size()?
                .ok_or_else(|| Error::InvalidRecord {
                    path: smol_str::SmolStr::new_static("$.commit_row_size"),
                    reason: crate::text::expected_got(
                        "a non-zero commit_row_size for a resumable write session",
                        "an unset commit_row_size",
                    ),
                })?;
        options.require_write_limits()?;
        let mut delegated = options.clone();
        let declared = delegated.take_field();
        delegated.set_select_by_names(Vec::new());
        delegated.set_max_row_size(None);
        delegated.set_max_byte_size(None);
        delegated.set_commit_row_size(None);
        Ok(Self {
            mode,
            options: options.clone(),
            delegated,
            declared,
            limit: crate::media::WriteLimitState::new(
                options.max_row_size(),
                options.max_byte_size(),
            ),
            commit_row_size,
            input_schema: None,
            shaped_schema: None,
            buffer: None,
            target: None,
            published: false,
            input_complete: false,
            terminal: false,
        })
    }

    /// Admit one Arrow chunk, publishing every complete cadence before return.
    ///
    /// The boolean is `true` while the binding should request another chunk.
    /// `false` means a global row or byte limit completed the logical input;
    /// no later source item may be inspected.
    pub fn push(
        &mut self,
        handle: &mut (impl IOBase + ?Sized),
        mut batches: crate::arrow::BatchReader,
    ) -> Result<bool> {
        use crate::media::IORecordOptions as _;
        use arrow_array::RecordBatchReader as _;

        self.require_live()?;
        if self.input_complete {
            return Ok(false);
        }
        let input_schema = batches.schema();
        if let Some(expected) = &self.input_schema {
            if expected.as_ref() != input_schema.as_ref() {
                let expected = format!("{expected:?}");
                let got = format!("{input_schema:?}");
                self.abort();
                return Err(Error::InvalidRecord {
                    path: smol_str::SmolStr::new_static("$"),
                    reason: crate::text::expected_got(
                        format_args!("the first asynchronous chunk schema {expected}"),
                        format_args!("a later chunk schema {got}"),
                    ),
                });
            }
        } else {
            self.input_schema = Some(std::sync::Arc::clone(&input_schema));
        }
        if let Err(error) = self.ensure_shaped_schema(handle, input_schema) {
            self.abort();
            return Err(error);
        }

        while !self.limit.satisfied() {
            let batch = match batches.next() {
                Some(Ok(batch)) => batch,
                Some(Err(error)) => {
                    self.abort();
                    return Err(crate::arrow::from_reader_error(error).into());
                }
                None => break,
            };
            let batch = match self.options.cast_arrow_batch(batch, self.target_field()) {
                Ok(batch) => batch,
                Err(error) => {
                    self.abort();
                    return Err(error);
                }
            };
            let Some(batch) = self.limit.apply(batch) else {
                break;
            };
            if batch.num_rows() != 0 {
                if let Some(reader) = self
                    .buffer
                    .as_mut()
                    .expect("a shaped session owns a commit buffer")
                    .push(batch)
                {
                    if let Err(error) = self.publish(handle, reader) {
                        self.abort();
                        return Err(error);
                    }
                }
                if let Err(error) = self.publish_ready(handle) {
                    self.abort();
                    return Err(error);
                }
            }
            if self.limit.satisfied() {
                if let Err(error) = self.complete_input(handle) {
                    self.abort();
                    return Err(error);
                }
                return Ok(false);
            }
        }
        if self.limit.satisfied() {
            if let Err(error) = self.complete_input(handle) {
                self.abort();
                return Err(error);
            }
            return Ok(false);
        }
        Ok(true)
    }

    /// Publish the final incomplete cadence and complete the session.
    pub fn finish(&mut self, handle: &mut (impl IOBase + ?Sized)) -> Result<()> {
        use crate::media::IORecordOptions as _;

        self.require_live()?;
        if !self.input_complete {
            if let Err(error) = self.complete_input(handle) {
                self.abort();
                return Err(error);
            }
        }
        if self.mode == crate::IOMode::Overwrite && !self.published {
            if self.shaped_schema.is_none() {
                let field = match self.options.require_field() {
                    Ok(field) => field.clone(),
                    Err(error) => {
                        self.abort();
                        return Err(error);
                    }
                };
                let schema = match field.into_arrow_schema() {
                    Ok(schema) => schema,
                    Err(error) => {
                        self.abort();
                        return Err(error.into());
                    }
                };
                self.input_schema = Some(std::sync::Arc::clone(&schema));
                if let Err(error) = self.ensure_shaped_schema(handle, schema) {
                    self.abort();
                    return Err(error);
                }
            }
            let schema = std::sync::Arc::clone(
                self.shaped_schema
                    .as_ref()
                    .expect("an empty overwrite has a shaped schema"),
            );
            if let Err(error) = self.publish(
                handle,
                crate::arrow::batch_reader(schema, std::iter::empty()),
            ) {
                self.abort();
                return Err(error);
            }
        }
        self.terminal = true;
        Ok(())
    }

    /// Drop the unpublished partial cadence while retaining prior commits.
    pub fn abort(&mut self) {
        if let Some(buffer) = &mut self.buffer {
            buffer.clear();
        }
        self.terminal = true;
    }

    fn require_live(&self) -> Result<()> {
        if self.terminal {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: smol_str::SmolStr::new_static(
                    "an Arrow write session cannot be reused after finish, abort, or failure",
                ),
            });
        }
        Ok(())
    }

    fn ensure_target(&mut self, handle: &(impl IOBase + ?Sized)) -> Result<()> {
        if self.target.is_some() {
            return Ok(());
        }
        if handle.is_container() {
            #[cfg(feature = "iceberg")]
            if let Some(located) = crate::media::iceberg::located(handle)? {
                let stored = located.stored_field()?;
                self.target = Some(ArrowWriteTarget::Iceberg {
                    located: Box::new(located),
                    stored,
                });
                return Ok(());
            }
            let mut writer = crate::media::partition::FolderWriter::new(handle, &self.options)?;
            writer.set_options(routing_options(
                self.delegated.clone(),
                self.declared.clone(),
            ))?;
            self.target = Some(ArrowWriteTarget::Folder {
                writer: Box::new(writer),
            });
        } else if matches!(self.delegated, RecordOptions::Text(_)) {
            self.target = Some(ArrowWriteTarget::TextLeaf);
        } else {
            self.target = Some(match stored_field(handle, &self.delegated)? {
                Some(stored) => ArrowWriteTarget::Leaf { stored },
                None => ArrowWriteTarget::EmptyLeaf,
            });
        }
        Ok(())
    }

    fn target_field(&self) -> Option<&crate::Field> {
        match self.target.as_ref() {
            Some(ArrowWriteTarget::Leaf { stored }) => Some(stored),
            Some(ArrowWriteTarget::EmptyLeaf)
            | Some(ArrowWriteTarget::TextLeaf)
            | Some(ArrowWriteTarget::Folder { .. })
            | None => None,
            #[cfg(feature = "iceberg")]
            Some(ArrowWriteTarget::Iceberg { stored, .. }) => Some(stored),
        }
    }

    fn ensure_shaped_schema(
        &mut self,
        handle: &(impl IOBase + ?Sized),
        input_schema: arrow_schema::SchemaRef,
    ) -> Result<()> {
        self.ensure_target(handle)?;
        if self.shaped_schema.is_some() {
            return Ok(());
        }
        use crate::media::IORecordOptions as _;
        let empty = arrow_array::RecordBatch::new_empty(input_schema);
        let shaped = self.options.cast_arrow_batch(empty, self.target_field())?;
        let schema = shaped.schema();
        // A missing leaf acquires the first shaped schema as this session's
        // target. Unlike an ordinary one-shot call, resumed cadences must not
        // re-plan against a resource another handle changed between awaits.
        if matches!(self.target, Some(ArrowWriteTarget::EmptyLeaf)) {
            self.target = Some(ArrowWriteTarget::Leaf {
                stored: crate::arrow::field_from_arrow_schema(
                    self.delegated.name(),
                    schema.as_ref(),
                )?,
            });
        }
        self.buffer = Some(crate::media::CommitBuffer::new(
            std::sync::Arc::clone(&schema),
            self.commit_row_size,
        ));
        self.shaped_schema = Some(schema);
        Ok(())
    }

    fn publish_ready(&mut self, handle: &mut (impl IOBase + ?Sized)) -> Result<()> {
        loop {
            let ready = self
                .buffer
                .as_mut()
                .and_then(crate::media::CommitBuffer::next_ready);
            let Some(reader) = ready else { return Ok(()) };
            self.publish(handle, reader)?;
        }
    }

    fn complete_input(&mut self, handle: &mut (impl IOBase + ?Sized)) -> Result<()> {
        if self.input_complete {
            return Ok(());
        }
        self.publish_ready(handle)?;
        let remainder = self
            .buffer
            .as_mut()
            .and_then(crate::media::CommitBuffer::finish);
        if let Some(reader) = remainder {
            self.publish(handle, reader)?;
        }
        self.input_complete = true;
        Ok(())
    }

    fn publish(
        &mut self,
        handle: &mut (impl IOBase + ?Sized),
        batches: crate::arrow::BatchReader,
    ) -> Result<()> {
        use crate::media::IORecordOptions as _;

        let mode = match (self.mode, self.published) {
            (crate::IOMode::Overwrite, true) => crate::IOMode::Append,
            (mode, _) => mode,
        };
        match self
            .target
            .as_mut()
            .expect("a publishing session has resolved its target")
        {
            ArrowWriteTarget::Leaf { stored } => match mode {
                crate::IOMode::Overwrite => {
                    handle.overwrite_prepared_arrow_reader(batches, &self.delegated)?
                }
                crate::IOMode::Append => {
                    append_leaf_onto(handle, batches, &self.delegated, stored)?
                }
                crate::IOMode::Merge => merge_leaf_onto(
                    handle,
                    batches,
                    &self.delegated,
                    self.delegated.merge_by_names(),
                    stored,
                )?,
                crate::IOMode::ReadOnly | crate::IOMode::Random => {
                    return Err(crate::Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$.mode"),
                        reason: smol_str::SmolStr::new_static(
                            "write mode readonly or random is not supported for this operation",
                        ),
                    });
                }
            },
            ArrowWriteTarget::TextLeaf => match mode {
                crate::IOMode::Overwrite => {
                    handle.overwrite_prepared_arrow_reader(batches, &self.delegated)?
                }
                crate::IOMode::Append => append_leaf(handle, batches, &self.delegated)?,
                crate::IOMode::Merge => merge_leaf(
                    handle,
                    batches,
                    &self.delegated,
                    self.delegated.merge_by_names(),
                )?,
                crate::IOMode::ReadOnly | crate::IOMode::Random => {
                    return Err(crate::Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$.mode"),
                        reason: smol_str::SmolStr::new_static(
                            "write mode readonly or random is not supported for this operation",
                        ),
                    });
                }
            },
            ArrowWriteTarget::EmptyLeaf => {
                unreachable!("a shaped session has resolved an empty leaf field")
            }
            ArrowWriteTarget::Folder { writer } => match mode {
                crate::IOMode::Overwrite => writer.overwrite(handle, batches)?,
                crate::IOMode::Append => writer.append(handle, batches)?,
                crate::IOMode::Merge => writer.merge(handle, batches)?,
                crate::IOMode::ReadOnly | crate::IOMode::Random => {
                    return Err(crate::Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$.mode"),
                        reason: smol_str::SmolStr::new_static(
                            "write mode readonly or random is not supported for this operation",
                        ),
                    });
                }
            },
            #[cfg(feature = "iceberg")]
            ArrowWriteTarget::Iceberg { located, .. } => match mode {
                crate::IOMode::Overwrite => {
                    located.overwrite_prepared(batches, self.delegated.safe())?
                }
                crate::IOMode::Append => located.append_prepared(batches)?,
                crate::IOMode::Merge => located.merge_prepared(
                    batches,
                    self.delegated.merge_by_names(),
                    self.delegated.safe(),
                )?,
                crate::IOMode::ReadOnly | crate::IOMode::Random => {
                    return Err(crate::Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$.mode"),
                        reason: smol_str::SmolStr::new_static(
                            "write mode readonly or random is not supported for this operation",
                        ),
                    });
                }
            },
        }
        self.published = true;
        Ok(())
    }
}

/// Peek until the first row-bearing batch without losing it or its schema.
///
/// Append and merge use this before touching a handle. A reader that ends (or
/// yields only zero-row batches) is a true no-op, while a first real batch is
/// returned ahead of the untouched remainder. At most that one batch is held.
#[cfg(feature = "arrow")]
pub(crate) fn non_empty_arrow_reader(
    mut batches: crate::arrow::BatchReader,
) -> Result<Option<crate::arrow::BatchReader>> {
    use arrow_array::RecordBatchReader as _;

    let schema = batches.schema();
    loop {
        let Some(batch) = batches.next() else {
            return Ok(None);
        };
        let batch = batch.map_err(crate::arrow::from_reader_error)?;
        if batch.num_rows() == 0 {
            continue;
        }
        return Ok(Some(Box::new(PrefixedBatchReader {
            schema,
            first: Some(batch),
            rest: batches,
        })));
    }
}

/// One peeked batch followed by the source it came from.
#[cfg(feature = "arrow")]
struct PrefixedBatchReader {
    schema: arrow_schema::SchemaRef,
    first: Option<arrow_array::RecordBatch>,
    rest: crate::arrow::BatchReader,
}

#[cfg(feature = "arrow")]
impl Iterator for PrefixedBatchReader {
    type Item = std::result::Result<arrow_array::RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.first.take().map(Ok).or_else(|| self.rest.next())
    }
}

#[cfg(feature = "arrow")]
impl arrow_array::RecordBatchReader for PrefixedBatchReader {
    fn schema(&self) -> arrow_schema::SchemaRef {
        std::sync::Arc::clone(&self.schema)
    }
}

/// Restore a declared field only for partition-layout discovery.
#[cfg(feature = "arrow")]
fn routing_options(mut delegated: RecordOptions, declared: Option<crate::Field>) -> RecordOptions {
    use crate::media::IORecordOptions;

    if let Some(field) = declared {
        delegated.set_field(field);
    }
    delegated
}

/// Decode one leaf, pushing the declared schema down and casting what returns.
///
/// This is the only place a record read reaches an encoding.
#[cfg(feature = "arrow")]
/// Narrow a reader to the columns the options select, in the order they name.
///
/// An empty selection is the reader as it stands - the common case pays one
/// slice borrow and nothing else. A non-empty one builds a target root holding
/// exactly the named columns of the reader's own schema, resolved ASCII
/// case-insensitively the way every cast matches names, and casts each batch
/// onto it - which is a projection, because the columns keep their datatypes.
/// A name the schema does not have is an error listing what is there, because
/// a selection is a claim about the rows rather than a wish.
#[cfg(feature = "arrow")]
pub(crate) fn select_reader(
    reader: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<crate::arrow::BatchReader> {
    use crate::media::IORecordOptions;

    let names = options.select_by_names();
    if names.is_empty() {
        return Ok(reader);
    }
    let root = crate::arrow::field_from_arrow_schema(options.name(), reader.schema().as_ref())?;
    match crate::arrow::selected_root(&root, names, options.name())? {
        Some(target) => Ok(crate::arrow::cast_reader(reader, &target, options.safe())?),
        None => Ok(reader),
    }
}

#[cfg(feature = "arrow")]
pub(crate) fn leaf_reader(
    handle: &(impl IOBase + ?Sized),
    options: &RecordOptions,
) -> Result<crate::arrow::BatchReader> {
    use crate::media::IORecordOptions;

    let declared = options.field();
    let declared = declared.as_ref();
    let reader = match options {
        RecordOptions::Ipc(ipc) => crate::media::ipc::read_batch_reader(handle, declared, ipc)?,
        #[cfg(feature = "parquet")]
        RecordOptions::Parquet(parquet) => {
            crate::media::parquet::read_batch_reader(handle, declared, parquet)?
        }
        RecordOptions::Avro(avro) => crate::media::avro::read_batch_reader(handle, declared, avro)?,
        RecordOptions::Text(text) => crate::media::text::arrow::read_arrow_reader(handle, text)?,
    };
    match declared {
        Some(field) => Ok(crate::arrow::cast_reader(reader, field, options.safe())?),
        None => Ok(reader),
    }
}

/// Count one encoded leaf from format metadata without decoding row arrays.
#[cfg(feature = "arrow")]
pub(crate) fn leaf_row_size(
    handle: &(impl IOBase + ?Sized),
    options: &RecordOptions,
) -> Result<u64> {
    match options {
        RecordOptions::Ipc(ipc) => crate::media::ipc::row_size(handle, ipc),
        #[cfg(feature = "parquet")]
        RecordOptions::Parquet(parquet) => crate::media::parquet::row_size(handle, parquet),
        RecordOptions::Avro(avro) => crate::media::avro::row_size(handle, avro),
        RecordOptions::Text(text) => crate::media::text::arrow::row_size(handle, text),
    }
}

/// Read one encoded leaf's canonical Struct field from format metadata.
///
/// Unlike asking a batch reader for its schema, each binary encoding reaches
/// its header or footer directly, so discovering a large Avro container's
/// width never fetches or decodes its block payloads.
#[cfg(feature = "arrow")]
pub(crate) fn leaf_field(
    handle: &(impl IOBase + ?Sized),
    options: &RecordOptions,
) -> Result<crate::Field> {
    use crate::media::IORecordOptions;

    if let Some(field) = options.field() {
        return Ok(field.clone());
    }
    match options {
        RecordOptions::Ipc(ipc) => Ok(crate::media::ipc::read_field(handle, ipc)?),
        #[cfg(feature = "parquet")]
        RecordOptions::Parquet(parquet) => Ok(crate::media::parquet::read_field(handle, parquet)?),
        RecordOptions::Avro(avro) => Ok(crate::media::avro::read_field(handle, avro)?),
        RecordOptions::Text(text) => text.source_field(),
    }
}

/// Encode one leaf's complete contents.
///
/// This is the only place a record write reaches an encoding. Nothing reaches
/// the handle until the last batch has been encoded, so a failure leaves the
/// resource exactly as it was.
#[cfg(feature = "arrow")]
pub(crate) fn leaf_writer(
    handle: &mut (impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    match options {
        RecordOptions::Ipc(ipc) => crate::media::ipc::overwrite_arrow_reader(handle, batches, ipc)?,
        #[cfg(feature = "parquet")]
        RecordOptions::Parquet(parquet) => {
            crate::media::parquet::overwrite_arrow_reader(handle, batches, parquet)?;
        }
        RecordOptions::Avro(avro) => {
            crate::media::avro::overwrite_arrow_reader(handle, batches, avro)?
        }
        RecordOptions::Text(text) => {
            crate::media::text::arrow::write_arrow_reader(handle, batches, text)?;
        }
    }
    Ok(())
}

/// Read the root Field a leaf's own bytes declare, if it holds any.
///
/// The declared schema is deliberately not consulted: this asks what is stored,
/// which is the only thing that can say whether a write is filling a resource
/// that already has a shape or giving one to a resource that has none.
#[cfg(feature = "arrow")]
pub(crate) fn stored_field(
    handle: &(impl IOBase + ?Sized),
    options: &RecordOptions,
) -> Result<Option<crate::Field>> {
    use crate::media::IORecordOptions;

    if handle.is_empty() {
        return Ok(None);
    }
    // Text lines store no record shape of their own: any row shape writes,
    // rendered line by line, so there is nothing to complete a cast onto.
    if matches!(options, RecordOptions::Text(_)) {
        return Ok(None);
    }
    let mut probe = RecordOptions::for_mime_type(&options.mime_type())?;
    probe.set_name(smol_str::SmolStr::new(options.name()));
    Ok(Some(leaf_field(handle, &probe)?))
}

/// Merge `incoming` into a leaf's rows on the options' match key.
#[cfg(feature = "arrow")]
fn merge_leaf(
    handle: &mut (impl IOBase + ?Sized),
    incoming: crate::arrow::BatchReader,
    options: &RecordOptions,
    merge_by_names: &[String],
) -> Result<()> {
    // A text line has no row identity: re-parsing the resource yields
    // projection rows, not the rows a caller wrote, so a key match would
    // silently compare against the wrong thing. Refused rather than guessed.
    if matches!(options, RecordOptions::Text(_)) {
        return Err(Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$.merge_by_names"),
            reason: crate::text::expected_got(
                "a record encoding with row identity to merge by (Arrow IPC, Parquet, Avro)",
                "text lines, which have none - use overwrite or append",
            ),
        });
    }
    let target = target_field(handle, &incoming, options)?;
    merge_leaf_onto(handle, incoming, options, merge_by_names, &target)
}

/// Merge an already-shaped cadence under one target fixed for the operation.
#[cfg(feature = "arrow")]
fn merge_leaf_onto(
    handle: &mut (impl IOBase + ?Sized),
    incoming: crate::arrow::BatchReader,
    options: &RecordOptions,
    merge_by_names: &[String],
    target: &crate::Field,
) -> Result<()> {
    use crate::media::IORecordOptions;

    // The stored side is read as the target so both sides of the match agree
    // column for column before a single key is compared.
    let mut rewrite = options.clone();
    rewrite.set_field(target.clone());
    let stored = leaf_reader(handle, &rewrite)?;
    let merged =
        crate::media::merge::merged(stored, incoming, target, merge_by_names, options.safe())?;
    // The merged contents are the whole new value. The cloned options already
    // had its declared field popped by `prepare_arrow_write`; clear the key as
    // well so the required overwrite hook sees exactly one publication and
    // cannot recursively merge the result against itself.
    rewrite.take_field();
    rewrite.set_merge_by_names(Vec::new());
    handle.overwrite_prepared_arrow_reader(merged, &rewrite)
}

/// Add `incoming` after a leaf's current rows.
#[cfg(feature = "arrow")]
fn append_leaf(
    handle: &mut (impl IOBase + ?Sized),
    incoming: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    // Text lines append natively: rows render after the current last line,
    // with no reason to re-parse what is already there.
    if let RecordOptions::Text(text) = options {
        return crate::media::text::arrow::append_arrow_reader(handle, incoming, text);
    }
    let target = target_field(handle, &incoming, options)?;
    append_leaf_onto(handle, incoming, options, &target)
}

/// Append an already-shaped cadence under one target fixed for the operation.
#[cfg(feature = "arrow")]
fn append_leaf_onto(
    handle: &mut (impl IOBase + ?Sized),
    incoming: crate::arrow::BatchReader,
    options: &RecordOptions,
    target: &crate::Field,
) -> Result<()> {
    use crate::media::IORecordOptions;

    let mut rewrite = options.clone();
    rewrite.set_field(target.clone());
    let current = if handle.is_empty() {
        // Per the laziness contract, a resource that holds nothing is skipped
        // rather than decoded.
        crate::arrow::batch_reader(crate::arrow::arrow_schema_from_field(target)?, [])
    } else {
        leaf_reader(handle, &rewrite)?
    };
    let appended = crate::arrow::appended(current, incoming, target, options.safe())?;
    rewrite.take_field();
    rewrite.set_merge_by_names(Vec::new());
    handle.overwrite_prepared_arrow_reader(appended, &rewrite)
}

/// Resolve the root Field a merge or an append produces.
///
/// The declared schema wins, then what the resource already stores, then the
/// shape the incoming reader arrived with - which is the only answer left when
/// nothing has been declared and nothing has been stored.
#[cfg(feature = "arrow")]
fn target_field(
    handle: &(impl IOBase + ?Sized),
    incoming: &crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<crate::Field> {
    use crate::media::IORecordOptions;

    if let Some(field) = options.field() {
        return Ok(field.clone());
    }
    if let Some(field) = stored_field(handle, options)? {
        return Ok(field);
    }
    Ok(crate::arrow::field_from_arrow_schema(
        options.name(),
        incoming.schema().as_ref(),
    )?)
}
