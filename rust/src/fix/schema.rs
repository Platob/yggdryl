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
//! Two lists close every row.
//!
//! `entries` is the whole arrival record: every pair, in arrival order,
//! untranslated. It is what makes a row lossless - the fixed columns are a
//! reading of the message and the entries are the message, so the wire is
//! rebuilt from them and never from the columns.
//!
//! `unmapped` is a *view* over that record rather than the rest of it: the
//! pairs no dictionary explained, in the order they arrived. It holds nothing
//! `entries` does not, and it exists because a venue onboarding a new field
//! should find it by reading one column instead of filtering a million rows.
//! On a well-known dialect it is empty on every row and costs a validity bit.

use smol_str::SmolStr;

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
/// A counter-named list, exactly as the dictionary spells every repeating
/// group: the crate publishes that contract for FIX groups and follows it for
/// its own columns. It carries no `fix:tag` - it is in no field set, no
/// registry and no dictionary shard, and exists only in the fixed row.
pub const ENTRIES_COLUMN: &str = "nofixentries";

/// The column holding what no dictionary explained.
pub const UNMAPPED_COLUMN: &str = "nounmappedfixentries";

/// What one arrival record is called wherever it is materialized.
///
/// Both columns name their occurrence this. The generic rule would derive
/// `UnmappedFixEntry` from the second counter, but the crate owns this
/// component and one component must not answer to two names: an unexplained
/// entry is the same kind of thing as an explained one, seen through a
/// narrower window.
const ENTRY_COMPONENT: &str = "fixentry";

/// One row's columns, in order, as tags.
///
/// Header, body, groups, then the crate's own derived fields. The trailer
/// sits before the two lists because it is still the message; the lists are
/// not columns of the message, they are the message.
#[must_use]
pub fn fix_schema_tags() -> Vec<i32> {
    let mut tags = Vec::with_capacity(HEADER_TAGS.len() + BODY_TAGS.len() + 16);
    tags.extend_from_slice(&HEADER_TAGS);
    tags.extend_from_slice(&BODY_TAGS);
    tags.extend_from_slice(&GROUP_TAGS);
    tags.extend_from_slice(&TRAILER_TAGS);
    for field in super::fix_crate_fields().unwrap_or_default() {
        if let Ok(Some(tag)) = field.as_fix().tag() {
            tags.push(tag);
        }
    }
    // `MsgDirection` is FIX's own and already sits in the header's dialect,
    // but no message carries it on the wire - it is read from the line - so
    // it is appended here rather than expected among the header's tags.
    tags.push(super::MSGDIRECTION_TAG);
    tags
}

/// The columns every message fills, so they are declared non-null.
///
/// `BeginString` because a message is built with one whatever its line
/// carried; the digest because every arrival record has one; the clock and
/// its partition because a message is stamped when it is built - by the row
/// that carried it, by its own clock, or by the epoch.
const REQUIRED_TAGS: [i32; 4] = [
    8,
    super::MSGHASH_TAG,
    super::TIMESTAMP_TAG,
    super::UNIXPARTITION_TAG,
];

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
    let mut fields: Vec<Field> = Vec::with_capacity(fix_schema_tags().len() + 2);
    for tag in fix_schema_tags() {
        // The standard branch answers first, and the crate's own fields are
        // on it: every registry holds them, so every tag here resolves.
        let Some(held) = registry.get_field_by_tag(tag) else {
            continue;
        };
        let mut held = held.clone();
        // Named as the dictionary names the field, which is already folded;
        // the display spelling rides on the field so a renderer can show
        // `MsgType` over the column `msgtype`.
        let spelling = held.name().to_owned();
        if held.as_metadata().get("display").is_none() {
            let carried: Vec<(String, String)> = held
                .as_metadata()
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .chain([("display".to_owned(), spelling)])
                .collect();
            let _ = held.set_metadata(carried);
        }
        held.set_nullable(!REQUIRED_TAGS.contains(&tag));
        if !fields
            .iter()
            .any(|known| crate::types::folds_equal(known.name(), held.name()))
        {
            fields.push(held);
        }
    }
    fields.push(entries_field(
        ENTRIES_COLUMN,
        "NoFixEntries",
        "Every pair the message carried, in arrival order and untranslated.",
    )?);
    fields.push(entries_field(
        UNMAPPED_COLUMN,
        "NoUnmappedFixEntries",
        "The pairs no dictionary explained, a view over the arrival record.",
    )?);
    Ok(DataType::from_fields(fields)?.required_field(name))
}

/// The fixed schema behind a capture's own columns.
///
/// A capture is read from somewhere, and where it was read from is what a
/// monitor orders and joins on: the object's URL, the line number in it, the
/// clock the line was stamped with, the thread that wrote it. None of that is
/// FIX and all of it is the row, so it leads the row - and because a line in
/// is a row out, carrying it is a slice rather than a join.
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
///     DataType::Utf8.required_field("url"),
///     DataType::Int64.required_field("rownum"),
///     DataType::Binary.required_field("body"),
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

/// A counter-named list of arrival records, typed once for both columns.
///
/// One function types both, so the two cannot drift: they are the same
/// component seen through different windows. The unexplained column carries
/// the same recursive type and is never populated below level 1, because it
/// is a view of unexplained entries rather than a second copy of the tree.
fn entries_field(name: &str, display: &str, description: &str) -> Result<Field> {
    let mut field = DataType::list(entry_item(1)?).nullable_field(name);
    field.set_display(display)?;
    field.set_description(description)?;
    Ok(field)
}

/// One `fixentry` struct at one materialization level.
///
/// The fifth member is `nofixentries` at every level and the meaning is
/// invariant; the type alone says where materialization stops - a list of
/// deeper occurrences above [`ENTRY_DEPTH`], the binary leaf at it. Every
/// occurrence, both inner lists and the leaf are non-null: an empty child
/// list means no children, an empty leaf means nothing was truncated, and
/// neither needs a validity bitmap to say so.
fn entry_item(level: usize) -> Result<Field> {
    let tail = if level < ENTRY_DEPTH {
        DataType::list(entry_item(level + 1)?).required_field(ENTRIES_COLUMN)
    } else {
        DataType::Binary.required_field(ENTRIES_COLUMN)
    };
    Ok(DataType::from_fields([
        DataType::Int32.nullable_field("tag"),
        DataType::Int32.required_field("branch"),
        DataType::Utf8.nullable_field("key"),
        DataType::Utf8.nullable_field("value"),
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
    // The key and the value are the entry's own text, shared rather than
    // copied: a row is materialized once per message and an entry's text is
    // what it is wherever it is read.
    Ok(crate::Scalar::from_sequence([
        crate::Scalar::from(entry.tag()),
        crate::Scalar::from(entry.branch()),
        crate::Scalar::from(entry.key_shared()),
        crate::Scalar::from(entry.value_shared()),
        tail,
    ]))
}

/// One truncated entry as the value the leaf's JSON stores.
///
/// The same five members in the same order, children as a plain array, so a
/// reader walks the decoded value exactly as it walks the materialized
/// levels. Untyped on the way back in, because no finite field describes an
/// unbounded subtree - and every member is an integer or UTF-8, so an untyped
/// decode loses nothing.
fn folded_scalar(entry: &super::FixEntry) -> crate::Scalar {
    crate::Scalar::from_sequence([
        crate::Scalar::from(entry.tag()),
        crate::Scalar::from(entry.branch()),
        crate::Scalar::from(entry.key_shared()),
        crate::Scalar::from(entry.value_shared()),
        crate::Scalar::from_sequence(
            entry
                .children()
                .iter()
                .map(folded_scalar)
                .collect::<Vec<_>>(),
        ),
    ])
}

/// One value as the column holds it.
///
/// A value the message holds was typed under the column's own field - the
/// dictionary's, which the fixed row renamed - so it is already the stored
/// form and passes untouched. A value this module derived, or one the
/// message holds in another kind, goes through the column's contract: a
/// digest becomes the fixed-width bytes its column declares, a clock read
/// out of text becomes an instant, and a kind the column cannot hold is
/// null rather than a refusal the row cannot survive. This is what lets a
/// reader take every row of a capture as canonical without walking it.
fn fitted(column: &Field, value: crate::Scalar) -> crate::Scalar {
    if value.is_null() || value.id() == column.dtype().id() {
        return value;
    }
    column.scalar(value).unwrap_or(crate::Scalar::Null)
}

/// One clock source as the instant it states, whatever it is typed as.
///
/// A narrow dictionary types a clock field as text, and the derived clock
/// column is declared as an instant however narrow the dictionary is. FIX's
/// own spelling is read here, so a stated timestamp stays a timestamp instead
/// of becoming a refusal the row cannot survive.
pub(super) fn as_instant(held: crate::Scalar) -> crate::Scalar {
    let Some(text) = held.as_str() else {
        return held;
    };
    // Only a dated reading. `TZTimeOnly` is an instant on the epoch day,
    // which is a legal value of that datatype and never a moment a capture
    // happened at, so a clock column takes nothing from one.
    if super::build::fix_date(text).is_none() {
        return crate::Scalar::Null;
    }
    super::build::wire_spelling(&CLOCK_DATATYPE, text).unwrap_or(crate::Scalar::Null)
}

/// The members one repeating-group field declares, or None for anything else.
fn item_fields(field: &Field) -> Option<&[Field]> {
    let item = match field.dtype() {
        DataType::List(item) | DataType::LargeList(item) => item.as_ref(),
        _ => return None,
    };
    item.dtype().as_fields()
}

impl super::FixMsg {
    /// This message as the fixed row a table holds.
    ///
    /// The columns are the schema's own, in its own order, and each is filled
    /// by the tag its name spells - so a message that carried nothing at a
    /// column answers null there rather than shifting its neighbours, which
    /// is what makes two rows of one capture comparable at all. A column no
    /// tag names is a capture's own and is left null for whoever read the
    /// capture to fill.
    ///
    /// The arrival record closes the row under [`ENTRIES_COLUMN`], so the row
    /// stays lossless whatever the columns made of it, and [`UNMAPPED_COLUMN`]
    /// holds the part of that record no dictionary explained - a view over
    /// the first rather than the rest of it.
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
    /// let order = reader.transform_line(b"8=FIX.4.4|35=D|55=AAPL|54=1|9999=x|10=0|", false)?;
    ///
    /// let row = order.to_row(&schema)?;
    /// let held = row.as_sequence().expect("a row");
    /// // The columns are the folded names, so `msgtype` is where the type is.
    /// let at = schema.index_of("msgtype").expect("the msgtype column");
    /// assert_eq!(held[at].as_str(), Some("D"));
    /// // A tag no dictionary explains is still there, in its own column.
    /// let unmapped = held.last().and_then(yggdryl::Scalar::as_sequence);
    /// assert_eq!(unmapped.map(<[yggdryl::Scalar]>::len), Some(1));
    /// # Ok(())
    /// # }
    /// ```
    /// # Errors
    ///
    /// Returns the JSON writer's failure rendering a truncated subtree into
    /// the arrival column's leaf - the one fallible step, because every other
    /// member is already a value.
    pub fn to_row(&self, schema: &Field) -> Result<crate::Scalar> {
        self.row_values(schema, &fix_column_tags(schema))
            .map(crate::Scalar::from_sequence)
    }

    /// The fixed row as the values it is made of, before they are wrapped.
    ///
    /// What [`Self::to_row`] answers, still open: a reader carrying its own
    /// columns in front of the tags writes them into the slots the schema
    /// left for them and wraps the row once, rather than unwrapping a row to
    /// wrap it again. `tags` is [`fix_column_tags`] over the same schema,
    /// read once by the caller rather than once per row.
    pub(super) fn row_values(
        &self,
        schema: &Field,
        tags: &[Option<i32>],
    ) -> Result<Vec<crate::Scalar>> {
        let columns = schema.fields();
        // The arrival record answers both closing columns and is walked once,
        // because the second is a view over the first rather than a second
        // record. A schema holding neither pays nothing for either.
        let (mut known, mut unknown) = (None, None);
        if columns
            .iter()
            .any(|held| matches!(held.name(), ENTRIES_COLUMN | UNMAPPED_COLUMN))
        {
            let (record, unexplained) = self.divided()?;
            known = Some(record);
            unknown = Some(unexplained);
        }
        // The market clock answers two columns, so it is read once for both.
        let mut clock: Option<crate::Scalar> = None;
        let mut values: Vec<crate::Scalar> = Vec::with_capacity(columns.len());
        for (column, tag) in columns.iter().zip(tags) {
            values.push(match column.name() {
                ENTRIES_COLUMN => known.take().unwrap_or(crate::Scalar::Null),
                UNMAPPED_COLUMN => unknown.take().unwrap_or(crate::Scalar::Null),
                // A column answers for the tag its field carries; one that
                // carries none is a capture's own column and nothing in the
                // message answers for it.
                _ => match *tag {
                    Some(tag) => {
                        let held = self.regrouped(tag, column, self.column_value(tag, &mut clock));
                        fitted(column, held)
                    }
                    None => crate::Scalar::Null,
                },
            });
        }
        Ok(values)
    }

    /// One group's value, laid out the way the fixed column declares it.
    ///
    /// A message's own group holds the members that occurrence stated, in the
    /// order it stated them; the fixed column declares the dictionary's
    /// members. Placing them by name is what lets an occurrence a bridge
    /// packed into one member land in the same column as one that spelled
    /// every member out - and a member an occurrence never stated is null,
    /// which is what keeps a row's content from failing its batch.
    fn regrouped(&self, tag: i32, declared: &Field, held: crate::Scalar) -> crate::Scalar {
        let Some(members) = item_fields(declared) else {
            return held;
        };
        let Some(occurrences) = held.as_sequence() else {
            return held;
        };
        // The message's own member names, in the order its values sit in.
        let spelled: Vec<&str> = self
            .index_of_tag(tag)
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
                            .position(|name| name.eq_ignore_ascii_case(member.name()))
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
    /// because a stated value is never overridden. Then the derived facts
    /// this crate computes - the digest, the version, the ticker, the clock,
    /// the partition. Then the lift's own enrichment, which fills a column
    /// the message did not state but forced: a buy order at a price is a
    /// party willing to pay it, so a bid lane it never wrote is still true of
    /// it, and a one-sided quote implies the side it never wrote either.
    ///
    /// Enrichment fills and never overwrites, so a column a venue did state
    /// is that venue's answer whatever the derivation would have said.
    ///
    /// `clock` is the market clock once it has been read: two columns derive
    /// from it, and the row reads it for the first and keeps it for the
    /// second.
    fn column_value(&self, tag: i32, clock: &mut Option<crate::Scalar>) -> crate::Scalar {
        // A stated value wins - a stated null is a value that would not
        // type, and the derivation still answers for it.
        if let Some(held) = self.get_by_tag(tag).filter(|held| !held.is_null()) {
            return held.clone();
        }
        if let Some(facet) = DERIVED_FACETS
            .iter()
            .find_map(|(held, facet)| (*held == tag).then_some(*facet))
        {
            if let Some(held) = self.lifted(facet) {
                return held.clone();
            }
        }
        // The columns every row fills, derived here for a message built from
        // a schema and a value exactly as the builder stamps one it parsed.
        match tag {
            8 => crate::Scalar::from(format!(
                "FIX.{}",
                self.registry()
                    .newest()
                    .map_or_else(|| "4.4".to_owned(), |held| held.version().to_string())
            )),
            super::MSGHASH_TAG => crate::Scalar::from(self.digest().to_be_bytes().to_vec()),
            super::VERSION_TAG => self.version().map_or(crate::Scalar::Null, |held| {
                crate::Scalar::from(held.to_string())
            }),
            super::SYMBOLTICKER_TAG => self.symbol_ticker(),
            super::TIMESTAMP_TAG => clock.get_or_insert_with(|| self.stamped_clock()).clone(),
            super::UNIXPARTITION_TAG => partition_of(
                clock.get_or_insert_with(|| self.stamped_clock()),
                super::DEFAULT_PARTITION_SECONDS,
            ),
            super::ISINCODE_TAG => self.isin_code(),
            super::MICCODE_TAG => self.mic_code(),
            super::STATE_TAG => self.state(),
            _ => crate::Scalar::Null,
        }
    }

    /// The clock the row is dated by, never null: the stamp, else the epoch.
    fn stamped_clock(&self) -> crate::Scalar {
        let held = self.market_timestamp();
        if held.is_null() { epoch() } else { held }
    }

    /// The instrument's ISIN: `SecurityID(48)` under an ISIN source, else the
    /// `SecurityAltID(455)` whose source says ISIN.
    ///
    /// The message's own `isincode` answered before this was asked, so this
    /// reads the standard tags a venue states one in.
    fn isin_code(&self) -> crate::Scalar {
        if self.get_by_tag(22).and_then(crate::Scalar::as_str) == Some("4") {
            if let Some(held) = self.get_by_tag(48).filter(|held| !held.is_null()) {
                return held.clone();
            }
        }
        self.group_member_where(454, 455, 456, "4")
            .unwrap_or(crate::Scalar::Null)
    }

    /// The market the message names, as the MIC it states first: the
    /// exchange the instrument is listed on, the destination it was routed to,
    /// or the market it last traded on.
    fn mic_code(&self) -> crate::Scalar {
        [207, 100, 30]
            .into_iter()
            .find_map(|tag| self.get_by_tag(tag).filter(|held| !held.is_null()).cloned())
            .unwrap_or(crate::Scalar::Null)
    }

    /// The order's state: `OrdStatus(39)`, else `ExecType(150)`, both typed
    /// as the crate's one lifecycle vocabulary.
    fn state(&self) -> crate::Scalar {
        [39, 150]
            .into_iter()
            .find_map(|tag| self.get_by_tag(tag).filter(|held| !held.is_null()).cloned())
            .unwrap_or(crate::Scalar::Null)
    }

    /// One member of the first occurrence of a group whose other member
    /// states `wanted`: the `455` beside a `456` of `4`, say.
    fn group_member_where(
        &self,
        group: i32,
        member: i32,
        by: i32,
        wanted: &str,
    ) -> Option<crate::Scalar> {
        let at = self.index_of_tag(group)?;
        let declared = item_fields(self.as_field().get_field_at(at)?)?;
        let position = |tag: i32| {
            declared
                .iter()
                .position(|field| field.as_fix().tag().ok().flatten() == Some(tag))
        };
        let (member, by) = (position(member)?, position(by)?);
        self.as_value()
            .get(at)?
            .as_sequence()?
            .iter()
            .filter_map(crate::Scalar::as_sequence)
            .find(|occurrence| occurrence.get(by).and_then(crate::Scalar::as_str) == Some(wanted))
            .and_then(|occurrence| occurrence.get(member).cloned())
            .filter(|held| !held.is_null())
    }

    /// Whether this message's dictionary has a field at one tag.
    fn explains(&self, tag: i32) -> bool {
        tag != 0 && self.registry().get_field_by_tag(tag).is_some()
    }

    /// The arrival record, and the part of it nothing explained.
    ///
    /// The record carries the tree, folded past the materialization depth.
    /// The unexplained view stays flat: every entry the dictionary does not
    /// explain, found pre-order at any depth, appears once at level 1 with
    /// empty children and nothing folded, because it is a view of unexplained
    /// entries and not a second recursive copy.
    ///
    /// # Errors
    ///
    /// Returns the JSON writer's failure rendering a truncated subtree.
    fn divided(&self) -> Result<(crate::Scalar, crate::Scalar)> {
        let mut known = Vec::new();
        let mut unknown = Vec::new();
        for entry in self.entries() {
            known.push(entry_scalar(entry, 1)?);
        }
        self.unexplained(self.entries(), &mut unknown);
        Ok((
            crate::Scalar::from_sequence(known),
            crate::Scalar::from_sequence(unknown),
        ))
    }

    /// Collects every entry nothing explains, pre-order, flat.
    fn unexplained(&self, entries: &[super::FixEntry], out: &mut Vec<crate::Scalar>) {
        for entry in entries {
            // Explained means the dictionary has the field, not that the key
            // parsed: `9999=x` names a tag and still names nothing, and a
            // venue onboarding it wants to find it in exactly one column.
            if !self.explains(entry.tag()) {
                out.push(crate::Scalar::from_sequence([
                    crate::Scalar::from(entry.tag()),
                    crate::Scalar::from(entry.branch()),
                    crate::Scalar::from(entry.key_shared()),
                    crate::Scalar::from(entry.value_shared()),
                    crate::Scalar::from_sequence(Vec::new()),
                ]));
            }
            self.unexplained(entry.children(), out);
        }
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

/// The timestamps a market clock is read from, in decreasing exactness.
///
/// `TransactTime` is when the venue acted, `TrdRegTimestamp` when a
/// regulatory clock ran, `SendingTime` when the frame left, and
/// `OrigSendingTime` when the original left on a resend. A capture with no
/// time is not orderable at all, so this falls rather than refuses - and
/// which one answered stays visible, because the columns they came from are
/// in the row beside it.
pub(super) const CLOCK_SOURCES: [i32; 4] = [60, 769, 52, 122];

/// What the derived clock column is declared as, and therefore what a clock
/// read out of a text-typed field has to become.
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

    /// The timestamp a capture is ordered and partitioned by.
    ///
    /// A message the codec built carries it as a child of its own, stamped
    /// when it was built - by the row that carried it, else by the first
    /// clock source it answers in decreasing exactness (`TransactTime`,
    /// `TrdRegTimestamp`, `SendingTime`, `OrigSendingTime`), else by the
    /// epoch - so this answers that child. A message built from a schema and
    /// a value carries no such child, and this reads the clock sources
    /// directly for it. A group's timestamp is read from its first
    /// occurrence, because a regulatory clock that ran several times still
    /// ran first once.
    #[must_use]
    pub fn market_timestamp(&self) -> crate::Scalar {
        if let Some(stamped) = self.get_by_tag(super::TIMESTAMP_TAG) {
            if !stamped.is_null() {
                return stamped.clone();
            }
        }
        wire_clock(|tag| self.get_by_tag(tag).cloned())
    }
}

/// The first clock source a message answers, read through `held`.
///
/// Shared by the built message and the builder that stamps it, so the two
/// can never disagree about which clock a message keeps.
pub(super) fn wire_clock(held: impl Fn(i32) -> Option<crate::Scalar>) -> crate::Scalar {
    for tag in CLOCK_SOURCES {
        let Some(held) = held(tag) else {
            continue;
        };
        if held.is_null() {
            continue;
        }
        if let Some(occurrences) = held.as_sequence() {
            if let Some(first) = occurrences.iter().find(|held| !held.is_null()) {
                return as_instant(first.clone());
            }
            continue;
        }
        return as_instant(held);
    }
    crate::Scalar::Null
}

/// The instant a message with no clock at all is stamped with.
///
/// The epoch: where a row nobody dated sorts first and visibly, rather than
/// among the rows of whatever day it happened to be read on.
pub(super) fn epoch() -> crate::Scalar {
    CLOCK_DATATYPE
        .scalar(crate::Scalar::from(0_i64))
        .unwrap_or(crate::Scalar::Null)
}

impl super::FixMsg {
    /// The partition [`Self::market_timestamp`] falls in, in whole seconds.
    ///
    /// Floor division rather than truncation, so a timestamp before the epoch
    /// lands in the partition that contains it rather than the one after.
    #[must_use]
    pub fn unix_partition(&self, seconds: i64) -> crate::Scalar {
        partition_of(&self.market_timestamp(), seconds)
    }
}

/// The partition one market clock falls in, in whole seconds.
///
/// Floor division rather than truncation, so a timestamp before the epoch
/// lands in the partition that contains it rather than the one after - and
/// floored from the clock's own nanoseconds, so a clock stated to the
/// microsecond still has a partition rather than a null for not being a
/// whole second.
fn partition_of(clock: &crate::Scalar, seconds: i64) -> crate::Scalar {
    if seconds <= 0 {
        return crate::Scalar::Null;
    }
    let Some(nanoseconds) = clock.temporal_count_at(crate::TimeUnit::Nanosecond) else {
        return crate::Scalar::Null;
    };
    let width = i128::from(seconds) * 1_000_000_000;
    let floored = i128::from(nanoseconds).div_euclid(width) * i128::from(seconds);
    i64::try_from(floored).map_or(crate::Scalar::Null, crate::Scalar::from)
}
