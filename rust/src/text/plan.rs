//! The compiled column plan one text read is answered by.
//!
//! Which columns exist, in what order, under which names, at which datatypes,
//! whether a capture is consumed by a fact the line already states - is
//! decided here, once, before a byte is read. The per-row path then reads
//! each planned column off the line's own reading of it, which resolves on
//! the first ask and once: a batch asks every row for every column of the
//! plan, the nineteen event columns it opens with included, and a projection
//! reads fewer of them afterwards; a line handed on as a line resolves only
//! what is asked of it.
//!
//! The schema and the plan are one derivation: [`TextOptions::source_field`] is
//! built from the plan, so a column cannot exist in one and not the other.

use std::collections::BTreeMap;

use smol_str::{SmolStr, format_smolstr};

use crate::graph::EventColumn;
use crate::{DataType, Error, Field, Result, StructType};

use super::options::{MIMETYPE_COLUMN, TextOptions};

/// What fills one emitted column.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TextSource {
    /// One of the nineteen columns the line is stated in as an event.
    Event(EventColumn),
    /// What the line was classified as.
    BodyType,
    /// The line itself, past its row header.
    Body,
    /// How many bytes went over the retained limit.
    DroppedByteSize,
    /// One row-header capture, by position in the expression.
    Capture(usize),
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
    ///
    /// The one place the answer is decided, and load-bearing in both
    /// directions: the row builder refuses a null this says cannot land,
    /// and a batch read back refuses a null cell under the same name. A
    /// column is nullable here exactly where the line can state nothing for
    /// it.
    pub(crate) nullable: bool,
    /// The column's own spelling, for a catalog that shows one; `None`
    /// where the name a caller wrote is the spelling.
    pub(crate) display: Option<&'static str>,
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
    /// Returns [`Error::InvalidRecord`] for a rename naming no column or two
    /// columns renamed onto one name. The options are left exactly as they
    /// were.
    pub(crate) fn compile(options: &TextOptions) -> Result<Self> {
        options.require_retained_body()?;
        let mut columns =
            Vec::with_capacity(EventColumn::ALL.len() + 3 + options.capture_names().len());
        // The event the line is, in the nineteen columns every graph event
        // is stated in - the same a FIX row opens with - so a message's
        // `srcuuids` joins the line's `curruuid` here, and a line read back
        // keeps the identity a message named.
        for column in EventColumn::ALL {
            push_named(
                &mut columns,
                TextSource::Event(column),
                SmolStr::new_static(column.name()),
                Some(column.display()),
                column.datatype()?,
                column.nullable(),
                Some(column.description()),
            );
        }
        if options.parse_mimetype {
            push(
                &mut columns,
                TextSource::BodyType,
                (MIMETYPE_COLUMN, "MimeType"),
                DataType::utf8(),
                false,
                "What the line was classified as.",
            );
        }
        push(
            &mut columns,
            TextSource::Body,
            ("body", "Body"),
            DataType::utf8(),
            false,
            "The line past its row header, as text: the edges stripped, the byte limit applied; never empty, because a line with no body is no line.",
        );
        if options.max_record_byte_size().is_some() {
            push(
                &mut columns,
                TextSource::DroppedByteSize,
                ("dropped_byte_size", "DroppedByteSize"),
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
            // Nullable, and named by the expression rather than by this
            // crate: a capture the header declared but did not match on a
            // line is the null its column holds, and the name the caller
            // wrote is the spelling a catalog shows.
            push_named(
                &mut columns,
                TextSource::Capture(index),
                SmolStr::new(name),
                None,
                dtype,
                true,
                Some(
                    "One row-header capture, read at the datatype its syntax matches; empty on every line the header declared it for and did not match.",
                ),
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
                // The spelling a catalog shows, beside what the column
                // holds: the nineteen event columns carry the display their
                // own enum states, so a line's row and a message's row name
                // one fact one way.
                if let Some(display) = column.display {
                    field.set_display(display)?;
                }
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
    (name, display): (&'static str, &'static str),
    dtype: DataType,
    nullable: bool,
    description: &'static str,
) {
    push_named(
        columns,
        source,
        SmolStr::new_static(name),
        Some(display),
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
    display: Option<&'static str>,
    dtype: DataType,
    nullable: bool,
    description: Option<&'static str>,
) {
    columns.push(TextColumn {
        name,
        source,
        dtype,
        nullable,
        display,
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
                    "expected a column name to rename, got {from:?}; only a row-header capture adds one"
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
