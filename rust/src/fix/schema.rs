//! The columns every message has, whatever it happened to carry.
//!
//! A message read into a per-message schema is a message no two of which can
//! be put in one table. This is the other shape: one fixed set of columns,
//! decided once, the same for a Logon and a market-data snapshot - so a
//! capture is a table, a partition is a file, and a reader written against it
//! keeps working when the next line carries a field it has never seen.
//!
//! # Columns are named by their folded names
//!
//! `msgtype`, never `35` and never `msg_type`. A column is spelled the way
//! the dictionary spells the field's canonical name - ASCII case folded once,
//! on the way in - so a row reads the way a message reads, in every binding
//! and every catalog, and a reader spelling `row["msgseqnum"]` finds the
//! sequence number without a dictionary in hand. The tag is still the
//! identity: each column carries its field's `fix:tag`, its lineage and its
//! code set, and the row is filled by that tag rather than by the spelling,
//! so a venue that renames a field between versions changes nothing about
//! where its value lands.
//!
//! # What is in it
//!
//! The standard header and trailer, because every message has them; the
//! fields a financial consumer actually reads, because they are what a table
//! is queried by; the three repeating groups worth persisting whole; the five
//! facts this crate derives; and then, last, everything else.
//!
//! # Nothing is lost at the end
//!
//! `nofixentries` closes every row with the whole arrival record: every pair, in arrival order,
//! untranslated. It is what makes a row lossless - the fixed columns are a
//! reading of the message and the entries are the message, so the wire is
//! rebuilt from them and never from the columns.
//!
//! Unresolved keys have tag zero in that same record; their original key,
//! value and children remain in place. No second projection duplicates them.

use std::cell::RefCell;
use std::sync::Arc;

use smol_str::SmolStr;

use crate::types::nested::Fields;
use crate::{DataType, Field, Result};

use super::FixRegistry;

/// The standard header, in the order FIX 4.4 declares it.
///
/// Every message carries it, so every row has these columns whether or not a
/// given message filled them.
pub const HEADER_TAGS: [i32; 24] = [
    8, 9, 35, 49, 56, 115, 128, 90, 91, 34, 50, 142, 57, 143, 116, 144, 129, 145, 43, 97, 52, 122,
    212, 213,
];

/// The standard trailer, in the order FIX 4.0 declares it.
pub const TRAILER_TAGS: [i32; 3] = [93, 89, 10];

/// The fields a financial consumer reads, in the order FIX writes them.
///
/// Not every FIX field, and deliberately: a column per tag makes the schema
/// depend on the dictionary's size rather than on what anyone queries. These
/// are the identifiers, the instrument, the sides and lanes, the prices, the
/// quantities, the currencies, the times and the statuses - what a book, a
/// blotter, a quote feed and a monitor all read.
///
/// Ordered as a message orders them - identity, then instrument, then the
/// order, then the quote's two lanes, then the times, then the outcome -
/// rather than by tag number, so a row reads the way the message it came
/// from reads.
pub const BODY_TAGS: [i32; 49] = [
    // Who the message is about: the order's own chain, its parents, and the
    // reports and quotes that answer it.
    1, 11, 41, 526, 37, 198, 17, 1003, 131, 117, 693, // The instrument.
    55, 48, 22, 167, 762, 207, 461, 541, 460, // The order.
    54, 40, 44, 38, 53, 854, 15, 120,
    // The quote's two lanes, which carry no side of their own.
    132, 133, 134, 135, // What was done.
    31, 32, 6, 14, 151, // When.
    60, 64, 75, 126, // How it went.
    39, 150, 297, 301, 368, 103, 102, 58,
];

/// The repeating groups persisted whole rather than lifted flat.
///
/// A group is the one shape a scalar column cannot hold, and these three are
/// the ones a consumer actually reads back: who was on the trade, what the
/// instrument's other identifiers were, and when each regulatory clock ran.
pub const GROUP_TAGS: [i32; 3] = [453, 454, 768];

/// The column holding the arrival record.
///
/// This arrival-record column carries no `fix:tag` or `fix:counter`: it belongs
/// to the fixed row, outside the registry and its dictionary shards.
pub const ENTRIES_COLUMN: &str = "nofixentries";

/// What one arrival record is called wherever it is materialized.
const ENTRY_COMPONENT: &str = "fixentry";

/// One row's columns, in order, as tags.
///
/// Header, body, groups, then the crate's own derived fields. The trailer
/// sits before the arrival list because it is still the message; the list
/// records the message rather than a projection of its fields.
#[must_use]
pub fn fix_schema_tags() -> Vec<i32> {
    let crated = super::fix_crate_fields().unwrap_or_default();
    let mut tags = Vec::with_capacity(
        HEADER_TAGS.len()
            + BODY_TAGS.len()
            + GROUP_TAGS.len()
            + TRAILER_TAGS.len()
            + crated.len()
            + 1,
    );
    tags.extend_from_slice(&HEADER_TAGS);
    tags.extend_from_slice(&BODY_TAGS);
    tags.extend_from_slice(&GROUP_TAGS);
    tags.extend_from_slice(&TRAILER_TAGS);
    for field in crated {
        if let Ok(Some(tag)) = field.as_fix().tag() {
            tags.push(tag);
        }
    }
    // `MsgDirection` is FIX's own and already sits in the header's dialect,
    // but no message carries it on the wire - it is read from the line - so
    // it is appended here rather than expected among the header's tags.
    tags.push(super::MSGDIRECTION_TAG_NAME.0);
    tags
}

/// BeginString and the partition supplement the identity owner's replay bundle.
fn is_required(tag: i32) -> bool {
    tag == 8 || tag == super::TIMEPARTITION_TAG_NAME.0 || super::identity::is_mandatory(tag)
}

/// The fixed root every message answers as.
///
/// Built from the dictionary, so each column carries its field's real type,
/// lineage and code set - and built without reading a single message, so two
/// captures that share a dictionary share a schema exactly.
///
/// A tag the dictionary does not have is skipped rather than invented: a
/// column with no field behind it could not be typed, and a dictionary
/// missing `Symbol` is a dictionary this was not meant for.
///
/// # Errors
///
/// Returns the schema grammar's refusal when the columns do not make a
/// struct, or when this crate's own fields do not build.
pub fn fix_schema(registry: &FixRegistry, name: impl Into<SmolStr>) -> Result<Field> {
    let tags = fix_schema_tags();
    let mut fields: Vec<Field> = Vec::with_capacity(tags.len() + 1);
    for tag in tags {
        if let Some(held) = registry.get_field_by_tag(tag) {
            let mut held = held.clone();
            // The scalar's canonical name is folded; its display preserves
            // the dictionary's spelling independently of that identity.
            if held.display().is_none() {
                let spelling = held.name().to_owned();
                held.set_display(spelling)?;
            }
            held.set_nullable(!is_required(tag));
            if !fields
                .iter()
                .any(|known| crate::types::folds_equal(known.name(), held.name()))
            {
                fields.push(held);
            }
        }
        // A native Map group owns its counter; no scalar has to precede it.
        if let Some(group) = registry.get_group_by_tag(tag) {
            let mut group = group.clone();
            group.set_nullable(true);
            fields.push(group);
        }
    }
    fields.push(entries_field()?);
    let schema = DataType::from_fields(fields)?.required_field(name);
    column_plan(&schema, registry)?
        .identity
        .require_bundle(&schema)?;
    Ok(schema)
}

/// The fixed schema behind a capture's own columns.
///
/// A capture is read from somewhere, and where it was read from is what a
/// monitor orders and joins on: the object's URL, the line number in it, the
/// clock the line was stamped with, the thread that wrote it. None of that is
/// FIX and all of it leads the row. A bulk configuration produces one output
/// row per configuration it named, repeating these source values for each
/// message, and no row at all where it named none.
///
/// A carried column whose name a FIX column already takes - under the fold
/// every name here resolves by, so `sessionId` and `sessionid` are one name -
/// is dropped rather than renamed or duplicated: the FIX column is the one a
/// reader spelling it means, and two columns of one name is not a schema.
/// What that column stated is not lost: the row fills the FIX column from
/// it, which is the whole point of naming a capture after a field.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// # use yggdryl::holder::local::Folder;
/// # use yggdryl::{DataType, FixRegistry, fix_schema, fix_schema_carrying};
/// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
/// # let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
/// let capture = DataType::from_fields([
///     DataType::utf8().required_field("url"),
///     DataType::Int64.required_field("rownum"),
///     DataType::utf8().required_field("body"),
/// ])?
/// .required_field("line");
///
/// let read = fix_schema(&registry, "fix")?;
/// let held = fix_schema_carrying(&capture, &read)?;
///
/// // The capture leads, and the fixed columns follow it.
/// assert_eq!(held.fields()[0].name(), "url");
/// assert_eq!(held.index_of("msgtype"), read.index_of("msgtype").map(|at| at + 3));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the schema grammar's refusal when the two halves do not make one
/// struct.
pub fn fix_schema_carrying(carrier: &Field, read: &Field) -> Result<Field> {
    let mut fields: Vec<Field> = carried(carrier, read)
        .into_iter()
        .filter_map(|at| carrier.fields().get(at).cloned())
        .collect();
    fields.extend(read.fields().iter().cloned());
    Ok(DataType::from_fields(fields)?.required_field(read.name()))
}

/// Where each of a capture's own columns sits, in the order they lead the row.
///
/// The one rule for which of them survive the FIX columns' claim on a name,
/// so a reader filling the row by position and the schema it fills are the
/// same reading of one capture.
pub(super) fn carried(carrier: &Field, read: &Field) -> Vec<usize> {
    carrier
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, held)| {
            !read
                .fields()
                .iter()
                .any(|column| crate::types::folds_equal(column.name(), held.name()))
        })
        .map(|(at, _)| at)
        .collect()
}

/// Where the column carrying one tag sits in a schema, if it does.
///
/// A column answers for a tag through the field it was built from, never
/// through its spelling: the fixed row names columns by their folded names,
/// and a caller-declared root may still spell one by its tag's digits, so
/// both readings are made and the tag on the field wins.
#[must_use]
pub fn fix_column_of(schema: &Field, tag: i32) -> Option<usize> {
    schema
        .fields()
        .iter()
        .position(|column| column_tag(column) == Some(tag))
}

/// The tag one column answers for: the one its field declares, else the one
/// its name spells.
fn column_tag(column: &Field) -> Option<i32> {
    column
        .as_fix()
        .tag()
        .ok()
        .flatten()
        .or_else(|| super::field::parse_tag(column.name()))
}

/// Each column's tag, read once for a whole schema.
///
/// A row is filled by tag, and a tag is a metadata read - so a batch of a
/// million rows reads the schema once and fills every row through this.
#[must_use]
pub fn fix_column_tags(schema: &Field) -> Vec<Option<i32>> {
    schema.fields().iter().map(column_tag).collect()
}

/// What one column answers for, read off its field once per schema.
///
/// A column declaring a group's `fix:counter` answers with that group; any
/// other column answers for the tag its field carries; one carrying neither
/// is a capture's own. Both are metadata reads, and a row is filled through
/// this so a batch of a million rows reads the schema once.
#[derive(Clone, Copy)]
pub(super) struct Column {
    pub(super) tag: Option<i32>,
    counter: Option<i32>,
}

/// One schema's resolved projections and canonical content order.
pub(super) struct Columns {
    columns: Box<[Column]>,
    pub(super) identity: super::identity::Plan,
}

pub(super) type ColumnPlan = Arc<Columns>;

impl std::ops::Deref for Columns {
    type Target = [Column];
    fn deref(&self) -> &[Column] {
        &self.columns
    }
}

pub(super) fn column_plan(schema: &Field, registry: &FixRegistry) -> Result<ColumnPlan> {
    let DataType::Struct(fields) = schema.dtype() else {
        return Err(super::identity::refused(
            schema.name(),
            "a Struct field",
            schema.dtype(),
        ));
    };
    let mut columns = Vec::with_capacity(fields.len());
    for column in fields.iter() {
        columns.push(Column {
            tag: super::identity::resolve_tag(column, registry)?,
            counter: column.as_fix().counter()?,
        });
    }
    let columns = columns.into_boxed_slice();
    let identity =
        super::identity::Plan::new(schema, columns.iter().map(|column| column.tag), registry)?;
    Ok(Arc::new(Columns { columns, identity }))
}

thread_local! {
    // One retained schema and registry: aliases belong to the resolving registry.
    static LAST_COLUMN_PLAN: RefCell<Option<(Fields, Arc<FixRegistry>, ColumnPlan)>> = const { RefCell::new(None) };
}

/// Pointer identity is fast; equal reconstructed layouts need a structural comparison.
pub(super) fn column_plan_of(schema: &Field, registry: &Arc<FixRegistry>) -> Result<ColumnPlan> {
    let DataType::Struct(columns) = schema.dtype() else {
        return column_plan(schema, registry);
    };
    LAST_COLUMN_PLAN.with(|held| {
        let mut held = held.borrow_mut();
        if let Some((known, resolver, plan)) = held.as_ref() {
            if Arc::ptr_eq(resolver, registry)
                && (known.shares_storage_with(columns) || known == columns)
            {
                return Ok(Arc::clone(plan));
            }
        }
        let plan = column_plan(schema, registry)?;
        *held = Some((columns.clone(), Arc::clone(registry), Arc::clone(&plan)));
        Ok(plan)
    })
}

/// How many `fixentry` structs any root-to-leaf path materializes.
///
/// The column's List is level 0; the `fixentry` it contains is level 1; a
/// child of that entry is level 2; a child of that entry is level 3; there is
/// no level-4 struct. "Three levels deep" means three `fixentry` structs on
/// any root-to-leaf path, and everything deeper folds into the binary leaf.
///
/// A materialization depth, never a semantic nesting limit: a message thirty
/// levels deep is read whole and folds, and adding a fourth level is changing
/// this constant, not rewriting shapes by hand.
const ENTRY_DEPTH: usize = 3;

/// The single counter-named list of arrival records.
fn entries_field() -> Result<Field> {
    let mut field = DataType::list(entry_item(1)?).nullable_field(ENTRIES_COLUMN);
    field.set_display("NoFixEntries")?;
    field.set_description("Every pair the message carried, in arrival order and untranslated.")?;
    Ok(field)
}

/// One `fixentry` struct at one materialization level.
///
/// The fourth member is `nofixentries` at every level and the meaning is
/// invariant; the type alone says where materialization stops - a list of
/// deeper occurrences above [`ENTRY_DEPTH`], the binary leaf at it. Every
/// occurrence, both inner lists and the leaf are non-null: an empty child
/// list means no children, an empty leaf means nothing was truncated, and
/// neither needs a validity bitmap to say so.
fn entry_item(level: usize) -> Result<Field> {
    let tail = if level < ENTRY_DEPTH {
        DataType::list(entry_item(level + 1)?).required_field(ENTRIES_COLUMN)
    } else {
        DataType::binary().required_field(ENTRIES_COLUMN)
    };
    Ok(DataType::from_fields([
        DataType::Int32.nullable_field("tag"),
        DataType::utf8().nullable_field("key"),
        DataType::utf8().nullable_field("value"),
        tail,
    ])?
    .required_field(ENTRY_COMPONENT))
}

/// One entry as the row value its materialization level takes.
///
/// Descendants past [`ENTRY_DEPTH`] fold into the leaf here and only here:
/// the builder never folds, and the Rust tree is never truncated. The leaf is
/// the UTF-8 bytes of the crate's own JSON over the truncated subtree, so one
/// serializer and one parser answer for it.
fn entry_scalar(entry: &super::FixEntry, level: usize) -> Result<crate::Scalar> {
    let tail = if level < ENTRY_DEPTH {
        let children: Result<Vec<crate::Scalar>> = entry
            .children()
            .iter()
            .map(|held| entry_scalar(held, level + 1))
            .collect();
        crate::Scalar::from_sequence(children?)
    } else if entry.children().is_empty() {
        crate::Scalar::from(&[] as &[u8])
    } else {
        let folded: Vec<crate::Scalar> = entry.children().iter().map(folded_scalar).collect();
        let rendered = crate::into_json_scalar(&crate::Scalar::from_sequence(folded))?;
        crate::Scalar::from(rendered.as_bytes())
    };
    // The key and the value are the ranges of the line the entry names, read
    // as the text a `utf8` column holds - lossily where a data field's bytes
    // are not text, which is the one place a row cannot say what arrived. It
    // says that it cannot: the decode leaves the replacement character, and
    // `anomalies()` reads it as the `Lossy` it is, on the parsed message and
    // on one rebuilt from this row alike.
    Ok(crate::Scalar::from_sequence([
        crate::Scalar::from(entry.tag()),
        crate::Scalar::from(entry.key_text()),
        crate::Scalar::from(entry.value_text()),
        tail,
    ]))
}

/// One truncated entry as the value the leaf's JSON stores.
///
/// The same four members in the same order, children as a plain array, so a
/// reader walks the decoded value exactly as it walks the materialized
/// levels. Untyped on the way back in, because no finite field describes an
/// unbounded subtree - and every member is an integer or UTF-8, so an untyped
/// decode loses nothing.
fn folded_scalar(entry: &super::FixEntry) -> crate::Scalar {
    crate::Scalar::from_sequence([
        crate::Scalar::from(entry.tag()),
        crate::Scalar::from(entry.key_text()),
        crate::Scalar::from(entry.value_text()),
        crate::Scalar::from_sequence(
            entry
                .children()
                .iter()
                .map(folded_scalar)
                .collect::<Vec<_>>(),
        ),
    ])
}

/// The members one repeating-group field declares, or None for anything else.
///
/// The row projection, restatement and builder share the catalog's reading
/// of a group's occurrence, including a Map's entries Struct.
pub(super) fn item_fields(field: &Field) -> Option<&[Field]> {
    let item = super::catalog::occurrence_of(field)?;
    item.dtype().as_fields()
}

/// One arrival entry read back out of the row value its level holds.
///
/// The inverse of [`entry_scalar`], level by level: the four members in the
/// order it wrote them, the children walked as the materialized List where
/// the level holds one and as the leaf's JSON - decoded through the crate's
/// one parser - where the level folded them. A leaf that cannot be decoded is
/// a refusal rather than a hole, because a message rebuilt with a pair
/// missing is a different message. A null tag reads as `0`, the tag of a
/// key that named no field, and a null key or value as empty text. The key
/// and the value are copied here, because a row is where a message stops being
/// a range of a line: the column holds the text, and the entry rebuilt from it
/// owns a page of its own.
///
/// # Errors
///
/// Returns [`crate::Error::InvalidRecord`] at the arrival path for a value
/// that is not an entry, including a folded leaf that does not decode.
fn entry_from_scalar(
    pair: &crate::Scalar,
    path: &crate::path::Path<'_>,
) -> Result<super::FixEntry> {
    let held = pair
        .as_sequence()
        .ok_or_else(|| entry_error(path, "a four-member arrival entry", pair.kind()))?;
    let [tag, key, value, tail] = held else {
        return Err(entry_error(path, "four arrival members", held.len()));
    };
    let tag = if tag.is_null() {
        0
    } else {
        tag.as_i128()
            .and_then(|value| i32::try_from(value).ok())
            .filter(|value| *value >= 0)
            .ok_or_else(|| {
                entry_error(
                    &path.field("tag"),
                    "a nonnegative i32 tag or null",
                    format_args!("{tag:?}"),
                )
            })?
    };
    let text = |value: &crate::Scalar, name| {
        if value.is_null() {
            Ok(crate::media::text::TextBytes::new())
        } else {
            let text = value
                .as_str()
                .ok_or_else(|| entry_error(&path.field(name), "text or null", value.kind()))?;
            crate::media::text::TextBytes::from_bytes(text)
        }
    };
    let children = entries_from_scalar(tail, &path.field(ENTRIES_COLUMN))?;
    let entry = super::FixEntry::new(tag, text(key, "key")?, text(value, "value")?);
    Ok(entry.with_children(children))
}

fn entry_error(
    path: &crate::path::Path<'_>,
    expected: impl std::fmt::Display,
    actual: impl std::fmt::Display,
) -> crate::Error {
    crate::Error::InvalidRecord {
        path: path.render().into(),
        reason: crate::text::expected_got(expected, crate::text::elide_display(&actual)),
    }
}

/// Decode one folded subtree, then walk the same entry shape as materialized rows.
fn entries_from_scalar(
    value: &crate::Scalar,
    path: &crate::path::Path<'_>,
) -> Result<Vec<super::FixEntry>> {
    let decoded;
    let value = if let Some(bytes) = value.as_bytes() {
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        decoded = crate::from_json_scalar(bytes)
            .map_err(|error| entry_error(path, "a JSON sequence of arrival entries", error))?;
        &decoded
    } else {
        value
    };
    value
        .as_sequence()
        .ok_or_else(|| entry_error(path, "a sequence of arrival entries", value.kind()))?
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            entry_from_scalar(entry, &path.child(crate::path::Segment::Index(index)))
        })
        .collect()
}

impl super::FixMsg {
    /// The message a fixed row holds: the inverse of [`Self::into_row`].
    ///
    /// The root is `schema` and the value is `row`, checked and canonicalized
    /// exactly as [`Self::with_registry`] checks one, so the columns are the
    /// message's children under the names the schema gave them and every
    /// lookup reaches them by tag as it reaches a parsed message's. The
    /// branches are the schema's own `fix:branches`. The entries are rebuilt from
    /// the [`ENTRIES_COLUMN`] - every level the row materialized, and the
    /// leaf the deepest level folded into decoded through the crate's own
    /// JSON reader - so [`Self::into_bytes`] re-emits the line the row was
    /// read from, and a row without that column has no entries. Nothing is
    /// parsed again: this is what makes a batch of rows a stream of messages
    /// at the cost of the values it already holds.
    ///
    /// The round trip is byte for byte over every capture this crate is
    /// tested against, and it is exact for an entry whose bytes are text -
    /// which is every entry a log wrote. It cannot be for one whose bytes are
    /// not: the row spells a key and a value as `utf8` because a column a
    /// reader can read is what a row is for, and a `data` field carrying
    /// bytes no text holds reaches that column as the decode of them. A
    /// message read from a line keeps the bytes and re-emits them; the same
    /// message read back out of a row re-emits the decode, and
    /// [`Self::anomalies`] reports the `Lossy` that says so on both.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::holder::local::Folder;
    /// # use yggdryl::{FixCodec, FixMsg, FixRegistry, fix_schema};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    /// let schema = fix_schema(&registry, "fix")?;
    /// let reader = FixCodec::new(Arc::clone(&registry));
    /// let line = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|9999=x|10=0|";
    /// let order = reader.parse_fix_line(line)?;
    ///
    /// let row = order.into_row(&schema)?;
    /// let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row)?;
    ///
    /// // The same message, reached the same way and re-emitted byte for byte.
    /// assert_eq!(held.by_tag(55)?, order.by_tag(55)?);
    /// assert_eq!(held.entries(), order.entries());
    /// assert_eq!(held.into_bytes(b'|'), line);
    /// // And the row it came from is the row it makes.
    /// assert_eq!(held.into_row(&schema)?, row);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the refusal [`Self::with_registry`] raises when the row does
    /// not fit the schema, or [`crate::Error::InvalidRecord`] at the arrival
    /// path when the entries column holds something that is not an arrival
    /// entry, a folded leaf that does not decode included.
    pub fn from_row(
        registry: Arc<FixRegistry>,
        schema: &Field,
        row: &crate::Scalar,
    ) -> Result<Self> {
        let plan = column_plan_of(schema, &registry)?;
        plan.identity.require_values(schema, row)?;
        let value = schema.canonicalize_value(row.clone())?;
        let mut built = Self::settled_parts(
            registry,
            schema.clone(),
            value,
            Vec::new(),
            super::identity::Assertions::STATED,
        )?;
        let entries = schema
            .index_of(ENTRIES_COLUMN)
            .and_then(|at| built.as_value().get(at))
            .and_then(crate::Scalar::as_sequence)
            .map(|held| {
                let root = crate::path::Path::root();
                let path = root.field(ENTRIES_COLUMN);
                held.iter()
                    .enumerate()
                    .map(|(index, entry)| {
                        entry_from_scalar(entry, &path.child(crate::path::Segment::Index(index)))
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        built.set_entries(entries);
        Ok(built)
    }

    /// This message as the fixed row a table holds.
    ///
    /// The columns are the schema's own, in its own order, and each is filled
    /// by its scalar tag or logical group's `fix:counter`. A message carrying
    /// nothing at a column answers null there rather than shifting its neighbours, which
    /// is what makes two rows of one capture comparable at all. A column no
    /// tag or group counter names is a capture's own: it takes the child of
    /// the same name where the message has one - which is how a row read
    /// back through [`Self::from_row`] returns to its schema whole - and is
    /// left null otherwise, which a required column refuses. The capture reader
    /// supplies its prefix before validation, not after projection.
    ///
    /// The arrival record closes the row under [`ENTRIES_COLUMN`], so the row
    /// stays lossless whatever the columns made of it. Keys no dictionary
    /// explained have tag zero in that same record, with their raw text intact.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::holder::local::Folder;
    /// # use yggdryl::{FixCodec, FixRegistry, fix_schema};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    /// let schema = fix_schema(&registry, "fix")?;
    /// let reader = FixCodec::new(Arc::clone(&registry));
    /// let order = reader.parse_fix_line(b"8=FIX.4.4|35=D|55=AAPL|54=1|9999=x|10=0|")?;
    ///
    /// let row = order.into_row(&schema)?;
    /// let held = row.as_sequence().expect("a row");
    /// // The columns are the folded names, so `msgtype` is where the type is.
    /// let at = schema.index_of("msgtype").expect("the msgtype column");
    /// assert_eq!(held[at].as_str(), Some("D"));
    /// // An unresolved numeric key stays in the one arrival record as tag zero.
    /// let entries = held.last().and_then(yggdryl::Scalar::as_sequence).unwrap();
    /// let entry = entries.iter().find(|entry| entry.get(1).and_then(yggdryl::Scalar::as_str) == Some("9999")).unwrap();
    /// assert_eq!(entry.get(0).and_then(yggdryl::Scalar::as_i128), Some(0));
    /// # Ok(())
    /// # }
    /// ```
    /// A value a column will not hold is that column's null rather than a
    /// refusal - a five-byte MIC under a four-byte column, an identifier
    /// whose check digit does not close - because the arrival record carries
    /// what arrived and a capture of ten million rows must not end on one of
    /// them. A column that cannot be null keeps the refusal, which is what
    /// separates an unreadable value from a broken contract.
    ///
    /// # Errors
    ///
    /// Refuses a missing or mistyped mandatory replay holder, a value a
    /// column that cannot be null will not hold, or an unrenderable truncated
    /// arrival subtree.
    pub fn into_row(&self, schema: &Field) -> Result<crate::Scalar> {
        let plan = column_plan_of(schema, self.registry())?;
        let mut values = self.row_values(schema, &plan, Vec::new())?;
        plan.identity
            .finalize(schema, &mut values, super::identity::Assertions::default())?;
        Ok(crate::Scalar::from_sequence(values))
    }

    /// The fixed row as the values it is made of, before they are wrapped.
    ///
    /// What [`Self::into_row`] answers, still open: a reader carrying its own
    /// columns in front of the tags writes them into the slots the schema
    /// left for them and wraps the row once, rather than unwrapping a row to
    /// wrap it again. `plan` is [`column_plan`] over the same schema, read
    /// once by the caller rather than once per row.
    pub(super) fn row_values(
        &self,
        schema: &Field,
        plan: &Columns,
        front: Vec<crate::Scalar>,
    ) -> Result<Vec<crate::Scalar>> {
        plan.identity.require_bundle(schema)?;
        let columns = schema.fields();
        if front.len() > columns.len() {
            return Err(super::identity::refused(
                schema.name(),
                columns.len(),
                front.len(),
            ));
        }
        let mut front = front.into_iter();
        let mut values: Vec<crate::Scalar> = Vec::with_capacity(columns.len());
        // The crate columns' derivations and the working row they read,
        // gathered off this message once for the three of them, and only
        // when one is asked for.
        let mut derived: Option<(Arc<super::enrich::Derivations>, Vec<crate::Scalar>)> = None;
        for (column, planned) in columns.iter().zip(plan.iter()) {
            if let Some(value) = front.next() {
                values.push(fitted(column, value)?);
                continue;
            }
            let value = match column.name() {
                ENTRIES_COLUMN => crate::Scalar::from_sequence(
                    self.entries()
                        .iter()
                        .map(|entry| entry_scalar(entry, 1))
                        .collect::<Result<Vec<_>>>()?,
                ),
                // A column declaring a group's `fix:counter` answers with
                // that group, read from the message's own occurrences. Any
                // other column answers for the tag its field carries; one that
                // carries none is a capture's own column, and only a child
                // spelled as it is - what a row read back through `from_row`
                // holds - answers for it.
                _ => {
                    if let Some(counter) = planned.counter {
                        let value = self
                            .index_of_group(counter)
                            .and_then(|index| self.as_value().get(index))
                            .cloned()
                            .unwrap_or(crate::Scalar::Null);
                        self.regrouped(counter, column, value)
                    } else {
                        match planned.tag {
                            Some(tag) => {
                                self.regrouped(tag, column, self.column_value(tag, &mut derived)?)
                            }
                            None => self
                                .index_of_name(column.name())
                                .and_then(|at| self.as_value().get(at))
                                .cloned()
                                .unwrap_or(crate::Scalar::Null),
                        }
                    }
                }
            };
            values.push(fitted(column, value)?);
        }
        Ok(values)
    }

    /// One group's value, laid out the way the fixed column declares it.
    ///
    /// A message's own group holds the members that occurrence stated, in the
    /// order it stated them; the fixed column declares the dictionary's
    /// members. Placing them by name is what lets an occurrence a bridge
    /// packed into one member land in the same column as one that spelled
    /// every member out. An unstated member becomes null; Field::scalar then
    /// enforces the declared occurrence's types and nullability. The
    /// [enriching pass](super::enrich) lays a group out the same way for
    /// the working row its derivations read, so a member named in a term
    /// stands where the registry's definition puts it.
    pub(super) fn regrouped(
        &self,
        tag: i32,
        declared: &Field,
        held: crate::Scalar,
    ) -> crate::Scalar {
        let Some(members) = item_fields(declared) else {
            return held;
        };
        let Some(occurrences) = held.as_sequence() else {
            return held;
        };
        // The message's own member names, in the order its values sit in.
        let spelled: Vec<&str> = self
            .index_of_group(tag)
            .and_then(|at| self.as_field().get_field_at(at))
            .and_then(item_fields)
            .map(|fields| fields.iter().map(Field::name).collect())
            .unwrap_or_default();
        let rows: Vec<crate::Scalar> = occurrences
            .iter()
            .map(|occurrence| {
                let Some(stated) = occurrence.as_sequence() else {
                    return occurrence.clone();
                };
                let row: Vec<crate::Scalar> = members
                    .iter()
                    .map(|member| {
                        spelled
                            .iter()
                            .position(|name| crate::types::folds_equal(name, member.name()))
                            .and_then(|at| stated.get(at))
                            .cloned()
                            .unwrap_or(crate::Scalar::Null)
                    })
                    .collect();
                crate::Scalar::from_sequence(row)
            })
            .collect();
        crate::Scalar::from_sequence(rows)
    }

    /// One column's value, derived where the message does not carry it.
    ///
    /// Three sources, in this order. What the message actually said, always,
    /// because a stated value is never overridden. Then the lift's own
    /// enrichment, which fills a column the message did not state but
    /// forced: a buy order at a price is a party willing to pay it, so a bid
    /// lane it never wrote is still true of it, and a one-sided quote implies
    /// the side it never wrote either. Then the derived facts this crate
    /// computes - the version, the ticker and the partition here, and every
    /// crate column whose field declares a `fix:derivation` through the one
    /// evaluator the [enriching pass](super::enrich) runs, so `isincode`,
    /// `miccode` and `state` fill a row of an unenriched message exactly as
    /// the pass would fill the message (decision 38).
    ///
    /// Enrichment fills and never overwrites, so a column a venue did state
    /// is that venue's answer whatever the derivation would have said.
    ///
    /// `derived` is the row's one gather for the crate columns: built on
    /// the first crate column asked for and read by the ones after it.
    ///
    /// # Errors
    ///
    /// Returns the registry's refusal of its own derivations, naming the
    /// field whose `fix:derivation` does not compile: a dictionary whose
    /// rules do not compile fills no row, exactly as it enriches no message.
    fn column_value(
        &self,
        tag: i32,
        derived: &mut Option<(Arc<super::enrich::Derivations>, Vec<crate::Scalar>)>,
    ) -> Result<crate::Scalar> {
        // A stated value wins - a stated null is a value that would not
        // type, and the derivation still answers for it.
        if let Some(held) = self.get_by_tag(tag).filter(|held| !held.is_null()) {
            return Ok(held.clone());
        }
        if let Some(facet) = DERIVED_FACETS
            .iter()
            .find_map(|(held, facet)| (*held == tag).then_some(*facet))
        {
            if let Some(held) = self.lifted(facet) {
                return Ok(held.clone());
            }
        }
        // The columns every row fills, derived here for a message built from
        // a schema and a value exactly as the builder stamps one it parsed.
        // A tuple's half is not a pattern, so the crate's tags are matched
        // by name.
        let is = |held: (i32, &str)| held.0 == tag;
        if tag == 8 {
            return Ok(crate::Scalar::from(format!(
                "FIX.{}",
                self.registry()
                    .newest()
                    .map_or_else(|| "4.4".to_owned(), |held| held.version().to_string())
            )));
        }
        Ok(if is(super::VERSION_TAG_NAME) {
            self.version().map_or(crate::Scalar::Null, |held| {
                crate::Scalar::from(held.to_string())
            })
        } else if is(super::SYMBOLTICKER_TAG_NAME) {
            self.symbol_ticker()
        } else if is(super::TIMEPARTITION_TAG_NAME) {
            partition_of(self.updatedat())
        } else if super::is_crate_tag(tag) {
            // The registry compiles every derivation once, and a refused
            // compile refuses the row as it refuses the pass; a derivation
            // that answers nothing is a null column, as on the message.
            let (derivations, row) = match derived {
                Some(held) => held,
                None => {
                    let compiled = self.registry().derivations()?;
                    let row = compiled.crate_row(self);
                    derived.insert((compiled, row))
                }
            };
            derivations.fill(tag, row).unwrap_or(crate::Scalar::Null)
        } else {
            crate::Scalar::Null
        })
    }
}

/// One column's value as that column holds it, leaf by leaf, best effort.
///
/// A capture is written by systems that disagree with the dictionary about
/// what a field is: a five-byte MIC where the standard says four, an
/// identifier whose check digit does not close, a quantity spelled as a
/// word. A table of ten million rows must not end on one of them. A leaf the
/// column refuses is that leaf's null, which is the honest answer for a value
/// nothing could read as the field it landed under, and nothing is lost by
/// it, because the arrival record beside it carries what arrived verbatim.
///
/// A nested column keeps everything that does read. The whole value is tried
/// first, so an ordinary row costs one call and nothing else; only when that
/// refuses is the value taken apart and put back together member by member,
/// so one unreadable `PartyID` costs that member, not the party around it and
/// not the parties beside it. An occurrence that cannot be formed at all,
/// because a member can hold neither its value nor a null, is dropped from
/// the list rather than taking the list with it.
///
/// Every null this puts in a value's place is reported through `log` at warn
/// level, naming the column and what the value could not be read as: a null
/// nobody can tell from a stated one is how a capture quietly loses a field,
/// and the log is where an operator sees that a venue and a dictionary
/// disagree.
///
/// Only a column that cannot be null keeps the refusal, and the two kinds of
/// failure stay apart because of it: an unreadable value is a null, and a
/// required column holding nothing is a broken contract that names itself.
/// The identity bundle is exactly such a contract, and
/// [`Plan::finalize`](super::identity::Plan) is where it is stated.
fn fitted(column: &Field, value: crate::Scalar) -> Result<crate::Scalar> {
    // Kept for the retry only where a retry has members to work on, and a
    // `Scalar`'s clone is a refcount rather than a copy of what it names.
    let retry = column.dtype().is_nested().then(|| value.clone());
    let refusal = match column.scalar(value) {
        Ok(held) => return Ok(held),
        Err(refusal) => refusal,
    };
    if let Some(value) = retry {
        if let Some(held) = refit(column, value) {
            log::warn!(
                "FIX column {}: kept what reads and nulled the rest ({refusal})",
                column.name()
            );
            return Ok(held);
        }
    }
    // The null is asked of the column rather than assumed, so a column that
    // refuses one answers with the refusal the value earned.
    match column.scalar(crate::Scalar::Null) {
        Ok(null) => {
            log::warn!("FIX column {}: null, {refusal}", column.name());
            Ok(null)
        }
        Err(_) => Err(refusal),
    }
}

/// One value rebuilt under one field with every leaf that will not fit nulled.
///
/// `None` where the field can hold neither the value nor a null in its place,
/// which is what drops one occurrence of a group rather than the group around
/// it. Reached only from [`fitted`]'s refusal path, so no row that reads pays
/// for it.
fn refit(field: &Field, value: crate::Scalar) -> Option<crate::Scalar> {
    let rebuilt = match field.dtype() {
        DataType::Struct(members) => value.as_sequence().map(|stated| {
            // A member the value never reached is the null the column would
            // have held anyway; one it reached is refitted in place.
            let held: Option<Vec<crate::Scalar>> = members
                .iter()
                .enumerate()
                .map(|(at, member)| match stated.get(at) {
                    Some(value) => refit(member, value.clone()),
                    None => member.scalar(crate::Scalar::Null).ok(),
                })
                .collect();
            held.map(crate::Scalar::from_sequence)
        }),
        DataType::List(item)
        | DataType::LargeList(item)
        | DataType::ListView(item)
        | DataType::LargeListView(item)
        | DataType::FixedSizeList(item, _) => value.as_sequence().map(|stated| {
            Some(crate::Scalar::from_sequence(
                stated
                    .iter()
                    .filter_map(|held| refit(item, held.clone()))
                    .collect::<Vec<_>>(),
            ))
        }),
        DataType::Map(map) => value.as_mapping().map(|stated| {
            // A pair whose key will not read names nothing, so it is left
            // out; one whose value will not read keeps its name and loses
            // the value, which is what every other column does.
            let entries = map.entries().dtype().as_fields()?;
            let [key, held] = entries else {
                return None;
            };
            crate::Scalar::from_mapping(stated.iter().filter_map(|(name, value)| {
                Some((refit(key, name.clone())?, refit(held, value.clone())?))
            }))
            .ok()
        }),
        // A leaf has no members to keep, so it is the value or the null.
        _ => None,
    };
    match rebuilt.flatten() {
        Some(held) => field.scalar(held).ok(),
        None => field
            .scalar(value)
            .ok()
            .or_else(|| field.scalar(crate::Scalar::Null).ok()),
    }
}

/// The columns a lift can fill that no message states.
///
/// Each is a tag whose facet derives rather than reads: the two quote lanes
/// and their sizes come from an order's stated price, quantity and side, and
/// a side comes back from a one-sided quote's single lane. The rules
/// themselves live with the lift table, because they are what a facet *is* -
/// this only says which column each of them lands in.
const DERIVED_FACETS: [(i32, &str); 5] = [
    (132, "bidpx"),
    (133, "askpx"),
    (134, "bidsize"),
    (135, "asksize"),
    (54, "side"),
];

/// The instrument identifiers a ticker is built from, in preference order.
///
/// `Symbol(55)` first, because a venue that states one means it. Then the
/// identifier and its scheme, because an ISIN with `SecurityIDSource` is a
/// stronger identity than a ticker even though it reads worse. `SecurityID`
/// alone is last, because an identifier whose scheme is unstated could be
/// anything.
const TICKER_SOURCES: [i32; 2] = [55, 48];

/// Exact layout shared by FIX event, creation, grid and previous clocks.
pub(super) const CLOCK_DATATYPE: DataType = DataType::DateTime64 {
    unit: crate::TimeUnit::Nanosecond,
    timezone: crate::Timezone::UTC,
};

impl super::FixMsg {
    /// One instrument symbol that is the same across venues.
    ///
    /// Two venues name one instrument three ways, and a table keyed on
    /// whichever they happened to send cannot be joined to itself. This picks
    /// the strongest identity the message actually carries and renders it one
    /// way: the exchange qualifies it where one is stated, and the scheme
    /// qualifies an identifier that is not a plain ticker - so `AAPL` at
    /// XNAS and `US0378331005` under ISIN are both readable and neither
    /// collides with the other venue's `AAPL`.
    ///
    /// Derived on every call and stored nowhere, like every other lift.
    #[must_use]
    pub fn symbol_ticker(&self) -> crate::Scalar {
        let Some((tag, held)) = TICKER_SOURCES
            .iter()
            .find_map(|tag| Some((*tag, self.get_by_tag(*tag)?)))
            .filter(|(_, held)| !held.is_null())
        else {
            return crate::Scalar::Null;
        };
        let Some(base) = held.as_str() else {
            return crate::Scalar::Null;
        };
        let mut rendered = String::with_capacity(base.len() + 16);
        // The scheme leads an identifier that is not a plain ticker, because
        // `US0378331005` says nothing about being an ISIN and two schemes
        // number differently.
        if tag != 55 {
            if let Some(scheme) = self.get_by_tag(22).and_then(crate::Scalar::as_str) {
                rendered.push_str(scheme);
                rendered.push(':');
            }
        }
        rendered.push_str(base);
        // The exchange qualifies it where the message states one: one ticker
        // at two venues is two instruments.
        if let Some(venue) = self.get_by_tag(207).and_then(crate::Scalar::as_str) {
            rendered.push('@');
            rendered.push_str(venue);
        }
        crate::Scalar::from(rendered)
    }
}

impl super::FixMsg {
    /// The partition [`Self::updatedat`] falls in: that instant floored to
    /// the hour, the crate's one partition width
    /// ([`DEFAULT_PARTITION_SECONDS`](super::DEFAULT_PARTITION_SECONDS)),
    /// as the same nanosecond UTC clock.
    ///
    /// Floor division rather than truncation, so a clock before the epoch
    /// lands in the hour that contains it rather than the one after. This is
    /// the value the `timepartition` column carries, and the value the
    /// column's own `transform:expression` computes when a batch arrives
    /// without it.
    #[must_use]
    pub fn time_partition(&self) -> crate::Scalar {
        partition_of(self.updatedat())
    }
}

/// The hour one market clock falls in, as an instant of the clock's layout.
///
/// Floor division rather than truncation, so a clock before the epoch lands
/// in the hour that contains it rather than the one after - and floored from
/// the clock's own nanoseconds, so a clock stated to the microsecond still
/// has a partition rather than a null for not being a whole second.
fn partition_of(clock: &crate::Scalar) -> crate::Scalar {
    let Some(nanoseconds) = clock.temporal_count_at(crate::TimeUnit::Nanosecond) else {
        return crate::Scalar::Null;
    };
    let width = super::DEFAULT_PARTITION_SECONDS * 1_000_000_000;
    let floored = nanoseconds.div_euclid(width) * width;
    crate::Scalar::datetime64(floored, crate::TimeUnit::Nanosecond, crate::Timezone::UTC)
        .unwrap_or(crate::Scalar::Null)
}
