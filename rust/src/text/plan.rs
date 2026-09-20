//! The compiled column plan one text read is answered by.
//!
//! Which columns exist, in what order, under which names, at which datatypes,
//! whether a capture is consumed by a column the line already states - is
//! decided here, once, before a byte is read. The per-row path then reads
//! each planned column off the line's own reading of it, which resolves on
//! the first ask and once: a batch asks every row for every column of the
//! plan, the sixteen event columns it opens with included, and a projection
//! reads fewer of them afterwards; a line handed on as a line resolves only
//! what is asked of it.
//!
//! The schema and the plan are one derivation: [`TextOptions::source_field`] is
//! built from the plan, so a column cannot exist in one and not the other.

use std::collections::BTreeMap;

use smol_str::{SmolStr, format_smolstr};

use crate::graph::EventColumn;
use crate::{DataType, Error, Field, FieldPath, Result, StructType};

use super::options::{MIMETYPE_COLUMN, MTIME_COLUMN, TextOptions, mtime_dtype};

/// What fills one emitted column.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TextSource {
    /// One of the sixteen columns the line is stated in as an event.
    Event(EventColumn),
    /// The object the line was read from.
    Url,
    /// The physical line number, offset by the configured first value.
    Rownum,
    /// When the record was written.
    Timestamp,
    /// What the line was classified as.
    BodyType,
    /// The line itself.
    Body,
    /// How many bytes went over the retained limit.
    DroppedByteSize,
    /// One row-header capture, by position in the expression.
    Capture(usize),
    /// One entry, by resolved path.
    Entry(FieldPath),
}

/// One emitted column, fully resolved.
#[derive(Clone, Debug)]
pub(crate) struct TextColumn {
    /// The emitted name, after renaming.
    pub(crate) name: SmolStr,
    /// What fills it.
    pub(crate) source: TextSource,
    /// The datatype it is built at.
    pub(crate) dtype: DataType,
    /// Whether it may hold a null.
    pub(crate) nullable: bool,
    /// What the column holds, for the catalog.
    pub(crate) description: Option<&'static str>,
}

/// The complete column plan for one configuration.
#[derive(Clone, Debug)]
pub(crate) struct TextPlan {
    columns: Vec<TextColumn>,
}

impl TextPlan {
    /// Compile the plan one configuration answers with.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a rename naming no column, two
    /// columns renamed onto one name, or a lifted path colliding with a column.
    /// The options are left exactly as they were.
    pub(crate) fn compile(options: &TextOptions) -> Result<Self> {
        let mut columns =
            Vec::with_capacity(EventColumn::ALL.len() + 8 + options.capture_names().len());
        // The event the line is, in the sixteen columns every graph event
        // is stated in - the same a FIX row opens with - so a message's
        // `srcuuids` joins the line's `curruuid` here, and a line read back
        // keeps the identity a message named.
        for column in EventColumn::ALL {
            push_named(
                &mut columns,
                TextSource::Event(column),
                SmolStr::new_static(column.name()),
                column.datatype()?,
                column.nullable(),
                Some(column.description()),
            );
        }
        push(
            &mut columns,
            TextSource::Url,
            "sourceurl",
            DataType::url(),
            true,
            "The URL of the object this line was read from.",
        );
        if options.start_rownum.is_some() {
            push(
                &mut columns,
                TextSource::Rownum,
                "rownum",
                DataType::Int64,
                false,
                "The physical line number within that object.",
            );
        }
        if options.parse_mtime {
            push(
                &mut columns,
                TextSource::Timestamp,
                MTIME_COLUMN,
                mtime_dtype(),
                true,
                "When the record was written: its own captured timestamp, or the handle's modification time when it declares none.",
            );
        }
        if options.parse_mimetype {
            push(
                &mut columns,
                TextSource::BodyType,
                MIMETYPE_COLUMN,
                DataType::utf8(),
                false,
                "What the line was classified as.",
            );
        }
        push(
            &mut columns,
            TextSource::Body,
            "body",
            DataType::utf8(),
            false,
            "The line itself, as text: the row header included, the edges stripped, the byte limit applied.",
        );
        if options.max_record_byte_size().is_some() {
            push(
                &mut columns,
                TextSource::DroppedByteSize,
                "dropped_byte_size",
                DataType::UInt64,
                true,
                "How many bytes of this record went over the retained limit.",
            );
        }
        for (index, name) in options.capture_names().enumerate() {
            if options.consumes_capture(index) {
                continue;
            }
            let dtype = options.capture_dtype(index);
            push_named(
                &mut columns,
                TextSource::Capture(index),
                SmolStr::new(name),
                dtype,
                true,
                None,
            );
        }
        for path in options.lift_paths() {
            // A lifted column takes the path's alias where it writes one, and
            // the last segment's own name otherwise. `55 as symbol` therefore
            // names its column in the same breath that selects it, and the
            // rename map stays for columns that are already there.
            let name = path.column_name().map(SmolStr::new).ok_or_else(|| {
                Error::InvalidRecord {
                    path: SmolStr::new_static("$.lift_names"),
                    reason: format_smolstr!(
                        "expected a lifted path ending in a name, or one aliased with `as`, got {path}"
                    ),
                }
            })?;
            push_named(
                &mut columns,
                TextSource::Entry(path.clone()),
                name,
                DataType::utf8(),
                true,
                None,
            );
        }
        rename(&mut columns, options.rename_columns())?;
        refuse_duplicates(&columns)?;
        Ok(Self { columns })
    }

    /// The emitted columns, in order.
    pub(crate) fn columns(&self) -> &[TextColumn] {
        &self.columns
    }

    /// The root field this plan answers.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the columns do not make a
    /// struct.
    pub(crate) fn field(&self, name: SmolStr) -> Result<Field> {
        let fields = self
            .columns
            .iter()
            .map(|column| {
                let mut field = column
                    .dtype
                    .clone()
                    .named_field(column.name.clone(), column.nullable);
                if let Some(description) = column.description {
                    field.set_description(description)?;
                }
                Ok(field)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(DataType::from(StructType::from_fields(fields)?).required_field(name))
    }
}

/// Add one fixed column under its default name.
fn push(
    columns: &mut Vec<TextColumn>,
    source: TextSource,
    name: &'static str,
    dtype: DataType,
    nullable: bool,
    description: &'static str,
) {
    push_named(
        columns,
        source,
        SmolStr::new_static(name),
        dtype,
        nullable,
        Some(description),
    );
}

/// Add one column already named.
fn push_named(
    columns: &mut Vec<TextColumn>,
    source: TextSource,
    name: SmolStr,
    dtype: DataType,
    nullable: bool,
    description: Option<&'static str>,
) {
    columns.push(TextColumn {
        name,
        source,
        dtype,
        nullable,
        description,
    });
}

/// Apply the rename map, refusing a key that names no column.
///
/// A rename decides what a column is called and never whether one exists, so a
/// key naming nothing is a mistake rather than a request to add something.
fn rename(columns: &mut [TextColumn], renames: &BTreeMap<SmolStr, SmolStr>) -> Result<()> {
    for (from, to) in renames {
        let Some(column) = columns.iter_mut().find(|column| &column.name == from) else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.rename_columns"),
                reason: format_smolstr!(
                    "expected a column name to rename, got {from:?}; lift an entry with lift_names instead"
                ),
            });
        };
        column.name = to.clone();
    }
    Ok(())
}

/// Refuse two columns emitting one name.
fn refuse_duplicates(columns: &[TextColumn]) -> Result<()> {
    for (index, column) in columns.iter().enumerate() {
        if columns[..index]
            .iter()
            .any(|earlier| earlier.name == column.name)
        {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.rename_columns"),
                reason: format_smolstr!(
                    "expected one column per name, got {:?} twice",
                    column.name
                ),
            });
        }
    }
    Ok(())
}
