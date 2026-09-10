//! The compiled column plan one text read is answered by.
//!
//! Everything the old decoder decided per row - which columns exist, in what
//! order, under which names, at which datatypes, whether a capture is consumed
//! by the timestamp - is decided here, once, before a byte is read. The per-row
//! path then moves bytes under a plan that is already resolved, which is what
//! `AGENTS.md` means by resolution being a boundary event.
//!
//! The schema and the plan are one derivation: [`TextOptions::source_field`] is
//! built from the plan, so a column cannot exist in one and not the other.

use std::collections::BTreeMap;

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Field, FieldPath, Result};

use super::options::{MIMETYPE_COLUMN, MTIME_COLUMN, TextOptions, mtime_dtype};

/// What fills one emitted column.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TextSource {
    /// The object the line was read from.
    Url,
    /// The physical line number, offset by the configured first value.
    Rownum,
    /// When the record was written.
    Timestamp,
    /// Which way the line moved.
    Direction,
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

impl TextSource {
    /// Whether filling this column needs the line's entry tree.
    const fn reads_entries(&self) -> bool {
        matches!(self, Self::Entry(_))
    }

    /// Whether filling this column needs the line classified.
    const fn reads_classification(&self) -> bool {
        matches!(self, Self::BodyType)
    }
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
    reads_entries: bool,
    reads_classification: bool,
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
        let mut columns = Vec::with_capacity(8 + options.capture_names().len());
        push(
            &mut columns,
            TextSource::Url,
            "url",
            DataType::Url,
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
        if options.parse_direction {
            push(
                &mut columns,
                TextSource::Direction,
                "direction",
                DataType::MsgDirection,
                true,
                "Which way the line moved, read from the verb in front of it.",
            );
        }
        if options.parse_mimetype {
            push(
                &mut columns,
                TextSource::BodyType,
                MIMETYPE_COLUMN,
                DataType::Utf8,
                false,
                "What the line was classified as.",
            );
        }
        push(
            &mut columns,
            TextSource::Body,
            "body",
            DataType::Binary,
            false,
            "The line itself, with whatever was read off its front removed.",
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
                DataType::Binary,
                true,
                None,
            );
        }
        rename(&mut columns, options.rename_columns())?;
        refuse_duplicates(&columns)?;
        Ok(Self {
            reads_entries: columns.iter().any(|column| column.source.reads_entries()),
            reads_classification: columns
                .iter()
                .any(|column| column.source.reads_classification()),
            columns,
        })
    }

    /// The emitted columns, in order.
    pub(crate) fn columns(&self) -> &[TextColumn] {
        &self.columns
    }

    /// Whether any column reads the entry tree.
    ///
    /// The one question the decoder asks before every line: materializing a
    /// tree is the only thing on the decode path that allocates, so a read
    /// whose columns never touch one must not build it.
    pub(crate) const fn reads_entries(&self) -> bool {
        self.reads_entries
    }

    /// Whether any column needs the line classified.
    pub(crate) const fn reads_classification(&self) -> bool {
        self.reads_classification
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
        Ok(DataType::from_fields(fields)?.required_field(name))
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
