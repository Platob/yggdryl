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
//! identity: each column carries its field's `FIX:tag` and its code set, and
//! the row is filled by that tag rather than by the spelling,
//! so a venue that renames a field between versions changes nothing about
//! where its value lands.
//!
//! # What is in it
//!
//! The standard header and trailer, because every message has them; the
//! fields a financial consumer actually reads, because they are what a table
//! is queried by; the four repeating groups worth persisting whole; the five
//! facts this crate derives; and then, last, everything else.
//!
//! # Values not represented by columns stay at the end
//!
//! `fixentries` closes every row with the residual arrival record, under the
//! `nofixentries` that counts it. A scalar a column successfully represents is
//! stated once, in that column. Unknown, unprojected, conflicting and partly
//! represented nested content remains in the record, in arrival order.
//!
//! Each residual entry carries the resolved tag, its dictionary name, its
//! canonical wire value and the entries nested below it. A message read back
//! from a row combines its columns with that residual, and a consumer can
//! group or filter the residual by name without a dictionary of its own.
//!
//! Unresolved keys have tag zero in that same record; their original key,
//! value and children remain in place, and `tagname` spells the key itself
//! rather than nothing. No second projection duplicates them.

use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Weak};

use crate::graph::MarketElement;

use smol_str::SmolStr;

use crate::{DataType, Field, Result};
use crate::{DateTimeType, StructType};

use super::FixRegistry;
use crate::sequence::SequenceType;

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
///
/// `Price(44)`, `OrderQty(38)` and `Quantity(53)` are columns of the ladder
/// like the rest, each exact and stated once. What a message is *about* is
/// what [`get_px`](crate::graph::MarketElement::get_px) and
/// [`get_qty`](crate::graph::MarketElement::get_qty) read off them, and no
/// column of this crate's restates either, because a row carrying both
/// would carry one fact twice.
pub const BODY_TAGS: [i32; 50] = [
    // Who the message is about: the order's own chain, its parents, and the
    // reports and quotes that answer it.
    1, 11, 41, 526, 37, 198, 17, 1003, 131, 117, 693,
    // The instrument, and what the market says about trading it.
    55, 48, 22, 167, 762, 207, 461, 541, 460, 326, 340, 965, // The order.
    54, 40, 59, 854, 15, 120, // The quote's two lanes, which carry no side of their own.
    132, 133, 134, 135, // What was done.
    31, 32, 6, 14, 151, // When.
    60, 64, 75, 126, // How it went.
    39, 150, 297, 301, 368, 103, 102, 58,
];

/// The repeating groups persisted whole rather than lifted flat.
///
/// A group is the one shape a scalar column cannot hold, and these four are
/// the ones a consumer actually reads back: who was on the trade, what the
/// instrument's other identifiers were, which regulatory identifiers the
/// trade carries, and when each regulatory clock ran.
pub const GROUP_TAGS: [i32; 4] = [453, 454, 768, 1907];

/// The group holding the arrival record.
///
/// Named as a group is named, because it is one: a List of `fixentry`
/// occurrences counted by [`NOFIXENTRIES_TAG_NAME`](super::NOFIXENTRIES_TAG_NAME),
/// exactly as `Parties` holds `Party` occurrences counted by `NoPartyIDs`. A
/// group spelled after its own counter was the one place this crate named a
/// thing after the thing beside it.
///
/// It is the one group the registry does not define. A `fixentry` contains
/// `fixentries`, and a catalog definition that referenced itself would be the
/// cyclic reference the store refuses to load; the shape is bounded to three
/// levels of occurrence instead, here, where the bound can be read.
pub const FIXENTRIES_COLUMN: &str = "fixentries";

/// What one arrival record is called wherever it is materialized.
const ENTRY_COMPONENT: &str = "fixentry";

/// The counter column's name, as a pattern a row fill matches on.
const NOFIXENTRIES_COLUMN: &str = super::crated::NOFIXENTRIES_TAG_NAME.1;

/// One row's columns, in order, as tags.
///
/// Ordered the way a reader thinks about a message rather than the way a
/// wire writes one, in nine bands: **when** it happened, **which** event it
/// is, **what message** carried it and over which session, **which
/// instrument** it is about, **which order** it belongs to, **what values**
/// it states, **how it went**, the **groups** kept whole, and last the
/// **frame** - the standard header and trailer fields nothing above claimed,
/// then the arrival record that closes the row.
///
/// A table is read by time, joined by identity and grouped by instrument, so
/// those three come first and in that order; a consumer scanning columns left
/// to right meets each band whole instead of meeting a clock, an identifier
/// and a price interleaved by tag number. Within a band the order is the one
/// the band's own subject implies: the instant a thing happened before the
/// instants it derives, the identity before what it descends from, the price
/// before the lanes that quote it.
///
/// Every band is a list of tags this crate names, and what no band names
/// still lands in the row: the standard header, trailer and body lists close
/// it, then any crate column a band left out. A tag named twice takes its
/// first place, so moving a column between bands is one edit and never a
/// duplicate.
///
/// This is the shape a capture lands in: projected, typed message columns
/// followed by the residual arrival record. Rebuilding combines both, so a
/// narrow projection chooses only its columns without losing the rest of the
/// message. Capture columns remain separate in front through
/// [`fix_schema_carrying`](super::fix_schema_carrying).
#[must_use]
pub fn fix_schema_tags() -> Vec<i32> {
    use super::crated::{
        CREAUNIX_TAG_NAME as CREAUNIX, CROSSCODE_TAG_NAME as CROSSCODE,
        CROSSHASHCODE_TAG_NAME as CROSSHASHCODE, CROSSUUID_TAG_NAME as CROSSUUID,
        CURRHASHCODE_TAG_NAME as HASHCODE, CURRUNIX_TAG_NAME as UNIX,
        CURRUUID_TAG_NAME as CURRUUID, EXECUNIX_TAG_NAME as EXECUNIX,
        EXPRTIME_TAG_NAME as EXPRTIME, IDENTIFIERS_TAG_NAME as IDENTIFIERS,
        METADATA_TAG_NAME as METADATA, MSGCTXID_TAG_NAME as MSGCTXID,
        MSGDIRECTION_TAG_NAME as MSGDIRECTION, MSGPLUGINID_TAG_NAME as MSGPLUGINID,
        MSGSESSIONID_TAG_NAME as MSGSESSIONID, PARENTUUIDS_TAG_NAME as PARENTUUIDS,
        PREVUNIX_TAG_NAME as PREVUNIX, PREVUUID_TAG_NAME as PREVUUID,
        RECDUNIX_TAG_NAME as RECDUNIX, REFRECDUNIX_TAG_NAME as REFRECDUNIX,
        SEQNUM_TAG_NAME as SEQNUM, SNAPUNIX_TAG_NAME as SNAPUNIX, SRCUUIDS_TAG_NAME as SRCUUIDS,
        STATE_TAG_NAME as STATE,
    };
    let crated = super::fix_crate_fields().unwrap_or_default();
    let counter = super::crated::NOFIXENTRIES_TAG_NAME.0;
    let mut tags: Vec<i32> = Vec::with_capacity(
        HEADER_TAGS.len() + BODY_TAGS.len() + GROUP_TAGS.len() + TRAILER_TAGS.len() + crated.len(),
    );
    let band = |tags: &mut Vec<i32>, held: &[i32]| {
        for tag in held {
            if !tags.contains(tag) {
                tags.push(*tag);
            }
        }
    };
    // When it happened: the settled instant, the execution and recording where
    // stated, the recording clock of the merge reference, then the instants
    // that instant is read against - created, followed, snapped, expiring -
    // and the clocks the protocol states.
    band(
        &mut tags,
        &[
            UNIX.0,
            EXECUNIX.0,
            RECDUNIX.0,
            REFRECDUNIX.0,
            CREAUNIX.0,
            PREVUNIX.0,
            SNAPUNIX.0,
            EXPRTIME.0,
            52,
            122,
            60,
            64,
            75,
            126,
            62,
            432,
        ],
    );
    // Which event: its own identity, the chain it stands in, what it
    // descends from and what it was read from. A join reads these and
    // nothing else.
    band(
        &mut tags,
        &[
            CURRUUID.0,
            CROSSUUID.0,
            CROSSCODE.0,
            HASHCODE.0,
            CROSSHASHCODE.0,
            PREVUUID.0,
            SEQNUM.0,
            PARENTUUIDS.0,
            SRCUUIDS.0,
            IDENTIFIERS.0,
        ],
    );
    // Which message, over which session: what the frame says it is, who sent
    // it to whom, and which bridge handled it. Not where this capture read
    // it: that is the reader's statement about the line and not the
    // message's about itself, so it travels as one of the capture's own
    // columns, beside the body and the row number, and no column of this row
    // restates it.
    band(
        &mut tags,
        &[
            8,
            35,
            super::MSGCAT_TAG_NAME.0,
            34,
            49,
            56,
            43,
            MSGDIRECTION.0,
            MSGPLUGINID.0,
            MSGCTXID.0,
            MSGSESSIONID.0,
        ],
    );
    // Which instrument: what the venue calls it and the ticker that settled
    // to, the identifiers this crate resolved for it, then what the market
    // said about trading it and the answer those add up to.
    band(
        &mut tags,
        &[
            55,
            48,
            22,
            super::ISINCODE_TAG_NAME.0,
            super::CUSIPCODE_TAG_NAME.0,
            super::SEDOLCODE_TAG_NAME.0,
            super::BLOOMBERGCODE_TAG_NAME.0,
            super::FIGICODE_TAG_NAME.0,
            super::MICCODE_TAG_NAME.0,
            167,
            762,
            207,
            100,
            30,
            461,
            541,
            460,
            326,
            340,
            965,
        ],
    );
    // Which order: the chain of identifiers a message and its answers share.
    band(
        &mut tags,
        &[1, 11, 41, 526, 37, 198, 17, 1003, 131, 117, 262, 693],
    );
    // What it states: the side it takes, then one ladder of prices and one
    // of quantities, each from the number the message is about down through
    // the ones it was read off - what it moved from, what it last traded,
    // where it has got to - then what those are counted and denominated in,
    // how the order was written, and last the quote's two lanes.
    //
    // Each number is FIX's own and appears once: `Price(44)`, `OrderQty(38)`
    // and `Quantity(53)` are columns like the rest of the ladder, and what a
    // message is *about* is what [`MarketElement::get_px`] reads off them
    // rather than a column restating one of them.
    band(
        &mut tags,
        &[
            54, 44, 140, 31, 6, 38, 53, 32, 14, 151, 996, 15, 120, 854, 40, 59, 132, 134, 133, 135,
        ],
    );
    // How it went: the ranked state the message reached - read off
    // `OrdStatus` and `ExecType`, the furthest its chain knows once walked -
    // then the protocol's own statuses and reasons, and whatever the venue
    // said in words.
    band(&mut tags, &[STATE.0, 39, 150, 297, 301, 368, 103, 102, 58]);
    // The groups kept whole, which no scalar column can hold.
    band(&mut tags, &GROUP_TAGS);
    // The frame: every standard header, body and trailer field no band above
    // claimed, in the order those lists state them.
    band(&mut tags, &HEADER_TAGS);
    band(&mut tags, &BODY_TAGS);
    band(&mut tags, &TRAILER_TAGS);
    // And every crate column no band named, so a column added to this crate
    // lands in the row without being listed twice - except the capture's
    // own, which is no column of this row at all: whoever read the line
    // states it beside the row, and a tail that swept it back in would put
    // the reader's word among the message's.
    let rest: Vec<i32> = crated
        .iter()
        .filter_map(|field| field.as_fix().tag().ok().flatten())
        .filter(|tag| {
            *tag != counter && *tag != METADATA.0 && !super::identity::is_capture_tag(*tag)
        })
        .collect();
    band(&mut tags, &rest);
    // Last, what the message carried outside its fields: the bridge's own
    // keys, then the arrival record's counter, which closes the row with the
    // group it counts.
    band(&mut tags, &[METADATA.0, counter]);
    tags
}

/// The columns every row states: `BeginString`, which the builder fills
/// where a line stated none, and the crate's own columns every message
/// settles - its instant, its creation, its codes and its identity. The
/// state it reached is stated on every row a message writes and admits a
/// null all the same: a state has no neutral member, so no default fills
/// the column a row leaves empty.
fn is_required(tag: i32) -> bool {
    tag == 8
        || [
            super::CURRUNIX_TAG_NAME.0,
            super::CREAUNIX_TAG_NAME.0,
            super::CURRHASHCODE_TAG_NAME.0,
            super::CROSSHASHCODE_TAG_NAME.0,
            super::CURRUUID_TAG_NAME.0,
            super::CROSSUUID_TAG_NAME.0,
        ]
        .contains(&tag)
}

/// The fixed root every message answers as.
///
/// Built from the dictionary, so each column carries its field's real type
/// and code set - and built without reading a single message, so two
/// captures that share a dictionary share a schema exactly.
///
/// A parse lands here, filled with what each message implies, a
/// [lifecycle](super::FixCodec::lifecycle_arrow_reader) walk states what
/// each message follows, and a
/// [format](super::FixCodec::format_arrow_reader) answers the same rows
/// under whatever message field a consumer reads by.
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
    rooted(registry, fix_schema_tags(), name)
}

/// The fixed row as a store dumps it: the columns of [`fix_schema`] under
/// the name and tag of [`FIXMSG_TAG_NAME`](super::FIXMSG_TAG_NAME), each
/// column a reference to the scalar or group the registry holds under it -
/// by name and by tag, so a reader resolves it by identity - and the
/// arrival record inline, because no definition can reference a shape that
/// contains itself. Written by [`FixRegistry::write_into`] as
/// `components/fixmsg.json` and read past by every reader, since this
/// crate is the row's one owner.
///
/// # Errors
///
/// Returns what [`fix_schema`] refuses, or the metadata grammar's refusal
/// of a reference.
pub(super) fn fixmsg_definition(registry: &FixRegistry) -> Result<Field> {
    let (tag, name) = super::FIXMSG_TAG_NAME;
    let schema = fix_schema(registry, name)?;
    let mut members = Vec::with_capacity(schema.fields().len());
    for column in schema.fields() {
        let mut member = column.clone();
        if column.name() != FIXENTRIES_COLUMN {
            let group = column
                .as_fix()
                .counter()?
                .and_then(|counter| registry.get_field_by_counter(counter))
                .filter(|group| crate::folds_equal(group.name(), column.name()));
            let scalar = column
                .as_fix()
                .tag()?
                .and_then(|tag| registry.get_scalar_by_tag(tag))
                .filter(|scalar| crate::folds_equal(scalar.name(), column.name()));
            if let Some(group) = group {
                member.as_fix_mut().set_group(group.name())?;
            } else if let Some(scalar) = scalar {
                member.as_fix_mut().set_field_ref(scalar.name())?;
            }
        }
        members.push(member);
    }
    let mut root = Field::new_with_metadata(
        schema.name(),
        DataType::from(StructType::from_fields(members)?),
        schema.is_nullable(),
        schema.as_metadata().clone(),
    );
    root.as_fix_mut().set_tag(tag)?;
    root.set_display("FixMsg")?;
    root.set_description(
        "The fixed row every message answers as: the crate's own columns, the standard header, \
         the fields a financial consumer reads, the groups persisted whole, the trailer, and the \
         arrival record that closes it.",
    )?;
    Ok(root)
}

/// One root over one tag list, closed by the arrival record.
pub(super) fn rooted(
    registry: &FixRegistry,
    tags: Vec<i32>,
    name: impl Into<SmolStr>,
) -> Result<Field> {
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
                .any(|known| crate::folds_equal(known.name(), held.name()))
            {
                fields.push(held);
            }
        }
        // A crate column the registry files under neither door. A definition
        // is categorized by its shape - a Map is a group, a Struct a
        // component - and a component is reached by no tag lookup. The
        // crate's own listing is where it is, and a column of this crate's
        // belongs in this crate's row whatever the catalog calls it.
        if super::is_crate_tag(tag)
            && registry.get_field_by_tag(tag).is_none()
            && registry.get_group_by_tag(tag).is_none()
        {
            if let Some(held) = super::fix_crate_fields()
                .unwrap_or_default()
                .iter()
                .find(|field| field.as_fix().tag().ok().flatten() == Some(tag))
            {
                let mut held = held.clone();
                held.set_nullable(!is_required(tag));
                if !fields
                    .iter()
                    .any(|known| crate::folds_equal(known.name(), held.name()))
                {
                    fields.push(held);
                }
            }
        }
        // A native Map group owns its counter; no scalar has to precede it.
        if let Some(group) = registry.get_group_by_tag(tag) {
            let mut group = group.clone();
            group.set_nullable(true);
            if !fields
                .iter()
                .any(|known| crate::folds_equal(known.name(), group.name()))
            {
                fields.push(group);
            }
        }
    }
    fields.push(entries_field()?);
    let schema = DataType::from(StructType::from_fields(fields)?).required_field(name);
    column_plan(&schema, registry)?;
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
/// # A carried column is nullable, whatever the capture declared
///
/// A capture's own column is the *reading's* statement, and no message holds
/// one: only a pass that has the source row in hand can state it. So a pass
/// that has not - the message halves,
/// [`messages`](super::FixCodec::messages) into
/// [`arrow_reader`](super::FixCodec::arrow_reader), with anything in between -
/// writes null there, and a column the capture declared non-nullable would
/// turn that into a refusal per row. It is relaxed here instead, so the
/// schema promises only what something can keep. The one-pass doors -
/// [`parse_text_arrow_reader`](super::FixCodec::parse_text_arrow_reader),
/// [`lifecycle_arrow_reader`](super::FixCodec::lifecycle_arrow_reader),
/// [`format_arrow_reader`](super::FixCodec::format_arrow_reader) - state
/// every one of them and never write that null.
///
/// ```
/// use yggdryl::StructType;
/// # fn main() -> yggdryl::Result<()> {
/// # use yggdryl::local::Folder;
/// # use yggdryl::{DataType, FixRegistry, fix_schema, fix_schema_carrying};
/// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
/// # let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
/// let capture = DataType::from(StructType::from_fields([
///     DataType::utf8().required_field("url"),
///     DataType::Int64.required_field("rownum"),
///     DataType::utf8().required_field("body"),
/// ])?)
/// .required_field("line");
///
/// let read = fix_schema(&registry, "fix")?;
/// let held = fix_schema_carrying(&capture, &read)?;
///
/// // The capture leads, and the fixed columns follow it.
/// assert_eq!(held.fields()[0].name(), "url");
/// assert_eq!(held.index_of("msgtype"), read.index_of("msgtype").map(|at| at + 3));
/// // And it leads it nullable: only a pass holding the source row can
/// // state where a line came from.
/// assert!(held.fields()[0].is_nullable());
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
        .map(|mut held| {
            held.set_nullable(true);
            held
        })
        .collect();
    fields.extend(read.fields().iter().cloned());
    Ok(DataType::from(StructType::from_fields(fields)?).required_field(read.name()))
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
                .any(|column| crate::folds_equal(column.name(), held.name()))
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
/// A column declaring a group's `FIX:counter` answers with that group; any
/// other column answers for the tag its field carries; one carrying neither
/// is a capture's own. Both are metadata reads, and a row is filled through
/// this so a batch of a million rows reads the schema once.
#[derive(Clone, Copy)]
pub(super) struct Column {
    pub(super) tag: Option<i32>,
    pub(super) counter: Option<i32>,
    /// Whether the column is the crate's own arrival record, declared
    /// exactly as [`entries_field`] declares it: [`entry_scalar`] writes
    /// what [`entry_item`] declares, leaf for leaf, so the value it builds
    /// is canonical under the column as built and is not walked twice
    /// more to prove it. A caller's own `fixentries` column of another
    /// shape is fitted as every column is.
    pub(super) entries: bool,
    /// Whether another column of the plan carries the same tag. A holder
    /// keeps one fact per tag, so a row stating one tag twice is left where
    /// it stands rather than lifted, and which columns those are is a fact
    /// of the shape, read once per shape rather than once per typed tag
    /// per message. A root rebuilt out of a planned root's children keeps
    /// every column of a shared tag, so the flag carries over with the
    /// column.
    pub(super) shared: bool,
}

/// One schema's resolved projections: one allocation holding the columns
/// and their count, which a root rebuilt out of a planned root's children
/// makes of the columns it keeps with [`Arc::from`].
pub(super) type ColumnPlan = Arc<[Column]>;

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
        let tag = super::identity::resolve_tag(column, registry)?;
        if let Some(tag) = tag {
            super::identity::validate_field(column, tag)?;
        }
        let counter = match registry.facts_of(column) {
            Some(facts) => facts.counter,
            None if column.as_metadata().is_empty() => None,
            None => column.as_fix().counter()?,
        };
        let entries = column.name() == FIXENTRIES_COLUMN
            && entries_field().is_ok_and(|declared| declared.dtype() == column.dtype());
        columns.push(Column {
            tag,
            counter,
            entries,
            shared: false,
        });
    }
    let mut tags: Vec<i32> = columns.iter().filter_map(|column| column.tag).collect();
    tags.sort_unstable();
    for column in &mut columns {
        column.shared = column.tag.is_some_and(|tag| {
            let at = tags.partition_point(|held| *held < tag);
            tags.get(at + 1) == Some(&tag)
        });
    }
    Ok(Arc::from(columns))
}

/// One schema's plan as this thread read it, beside the schema and a weak
/// identity for the registry it was read against: aliases belong to the
/// resolving registry, but a plan does not keep it alive.
type PlannedSchema = (StructType, Weak<FixRegistry>, ColumnPlan);

/// The plans one thread has read, by the shape of the schema each is for.
type ColumnPlans = super::registry::FixMap<u64, Vec<PlannedSchema>>;

thread_local! {
    /// Every schema this thread has planned, by its shape: a capture
    /// states a few shapes a million times each, and a plan is one metadata
    /// read per column.
    static COLUMN_PLANS: RefCell<ColumnPlans> = RefCell::new(ColumnPlans::default());
}

/// How many shapes one thread remembers plans for; past it a plan is still
/// read and simply not kept.
const REMEMBERED_COLUMN_PLANS: usize = 4_096;

/// How many schemas of one shape one thread remembers plans for - one
/// per registry they were planned against - past which the oldest is
/// forgotten. These are plans, not retained registries.
const REMEMBERED_SCHEMAS_PER_SHAPE: usize = 4;

/// The schemas this thread planned last, by the address of each one's
/// children and of the registry it was planned against: a door filling a
/// million rows under one schema holds the one `Field` for all of them, so
/// the plan is found by the address before the shape is digested at all.
/// A weak registry identity keeps its allocation address reserved, so a
/// dropped resolver's address is not reused while this plan is remembered.
type PlannedByAddress = Vec<((usize, usize), PlannedSchema)>;

thread_local! {
    static PLANNED_BY_ADDRESS: RefCell<PlannedByAddress> = const { RefCell::new(Vec::new()) };
}

/// How many schemas one thread remembers by address; a builder makes a
/// fresh root per message, and this many are what a stream reads under.
const REMEMBERED_BY_ADDRESS: usize = 16;

/// The plan one schema has, read once per shape per thread.
///
/// The schema's own address answers first, for the schema a door holds
/// across every row; else the shape digest names a bucket, and pointer
/// identity, else a structural comparison, says which remembered schema is
/// this one.
pub(super) fn column_plan_of(schema: &Field, registry: &Arc<FixRegistry>) -> Result<ColumnPlan> {
    let DataType::Struct(columns) = schema.dtype() else {
        return column_plan(schema, registry);
    };
    let address = (
        columns.storage_address(),
        Arc::as_ptr(registry).cast::<()>() as usize,
    );
    let planned = PLANNED_BY_ADDRESS.with(|held| {
        let mut held = held.borrow_mut();
        let at = held.iter().position(|(held, _)| *held == address)?;
        let (_, (known, resolver, plan)) = &held[at];
        if !(std::ptr::eq(resolver.as_ptr(), Arc::as_ptr(registry))
            && known.shares_storage_with(columns))
        {
            return None;
        }
        let plan = Arc::clone(plan);
        // The schema a door holds across its rows stays in front of the
        // roots a builder makes once each.
        held.swap(0, at);
        Some(plan)
    });
    if let Some(plan) = planned {
        return Ok(plan);
    }
    let shape = shape_digest(schema, true);
    let plan = COLUMN_PLANS.with(|held| -> Result<ColumnPlan> {
        if let Some(planned) = held.borrow().get(&shape) {
            for (known, resolver, plan) in planned {
                if std::ptr::eq(resolver.as_ptr(), Arc::as_ptr(registry))
                    && (known.shares_storage_with(columns) || known == columns)
                {
                    return Ok(Arc::clone(plan));
                }
            }
        }
        let plan = column_plan(schema, registry)?;
        let mut held = held.borrow_mut();
        if held.len() < REMEMBERED_COLUMN_PLANS || held.contains_key(&shape) {
            let planned = held.entry(shape).or_default();
            if planned.len() >= REMEMBERED_SCHEMAS_PER_SHAPE {
                planned.remove(0);
            }
            planned.push((columns.clone(), Arc::downgrade(registry), Arc::clone(&plan)));
        }
        Ok(plan)
    })?;
    PLANNED_BY_ADDRESS.with(|held| {
        let mut held = held.borrow_mut();
        if held.capacity() == 0 {
            held.reserve_exact(REMEMBERED_BY_ADDRESS);
        }
        if held.len() >= REMEMBERED_BY_ADDRESS {
            held.pop();
        }
        held.insert(
            0,
            (
                address,
                (columns.clone(), Arc::downgrade(registry), Arc::clone(&plan)),
            ),
        );
    });
    Ok(plan)
}

/// The tag one field carries and the group it counts: off the registry's
/// index where the field is the dictionary's own or stated under it, off
/// the field's metadata otherwise.
pub(super) fn tag_and_counter(registry: &FixRegistry, field: &Field) -> (Option<i32>, Option<i32>) {
    // A field stating nothing states no tag and counts nothing: a key the
    // dictionary does not explain is most of every bridge row.
    if field.as_metadata().is_empty() {
        return (None, None);
    }
    match registry.facts_of(field) {
        Some(facts) => (facts.tag, facts.counter),
        None => {
            let view = field.as_fix();
            (view.tag().ok().flatten(), view.counter().ok().flatten())
        }
    }
}

/// A digest of one root's shape: its children's names, datatype
/// identifiers and nullability, nested children included, and each
/// child's metadata storage where `metadata` says so.
///
/// Two roots of one shape digest alike, so a table keyed by the digest
/// finds what was read against a root of this shape; two of different
/// shapes may collide, so what the table holds says whether it is the
/// same shape - [`same_shape`], or the schema equality a caller compares.
pub(super) fn shape_digest(root: &Field, metadata: bool) -> u64 {
    // The dictionary's own fold rather than a keyed hash: a root is a
    // hundred children, each a few writes, read once per message, and
    // what the digest names is verified before it is believed.
    let mut state = super::registry::Mix::default();
    digest_shape(root.fields(), metadata, &mut state);
    state.finish()
}

fn digest_shape(fields: &[Field], metadata: bool, state: &mut super::registry::Mix) {
    state.write_usize(fields.len());
    for field in fields {
        state.write(field.name().as_bytes());
        field.dtype().id().hash(state);
        state.write_u8(u8::from(field.is_nullable()));
        if metadata {
            state.write_usize(field.as_metadata().storage_address());
        }
        match field.dtype() {
            DataType::Struct(children) => digest_shape(children.as_fields(), metadata, state),
            DataType::Sequence(SequenceType::List(item) | SequenceType::LargeList(item)) => {
                digest_shape(std::slice::from_ref(&**item), metadata, state);
            }
            _ => {}
        }
    }
}

/// Whether two roots are one shape to a term bound against either: the
/// same children by name, datatype and nullability, in the same order.
pub(super) fn same_shape(left: &Field, right: &Field) -> bool {
    let (left, right) = (left.fields(), right.fields());
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.name() == right.name()
                && left.dtype() == right.dtype()
                && left.is_nullable() == right.is_nullable()
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

/// The one group of arrival records, under the counter that counts them.
fn entries_field() -> Result<Field> {
    let mut field = DataType::list(entry_item(1)?).nullable_field(FIXENTRIES_COLUMN);
    field.set_display("FixEntries")?;
    field.set_description(
        "Content not represented by another column, in arrival order, beside what the dictionary made of it.",
    )?;
    // A group says which counter counts it, the way every other group in this
    // crate says it. It does not name its occurrence as a component: a
    // `fixentry` contains `fixentries`, so a catalog reference to it would be
    // the cycle the store refuses to load. The occurrence is declared inline
    // instead, which is also why `ENTRY_DEPTH` bounds it here.
    field
        .as_fix_mut()
        .set_counter(super::NOFIXENTRIES_TAG_NAME.0)?;
    Ok(field)
}

/// The resolved canonical tag of the field an entry states, `0` where no
/// dictionary explains it.
const TAG_COLUMN: (&str, &str) = ("tag", "Tag");

/// The dictionary's canonical name for that tag, else the key as it
/// arrived. Never null.
const NAME_COLUMN: (&str, &str) = ("name", "Name");

/// The value as the wire spells it; null for an entry that only heads
/// others.
const VALUE_COLUMN: (&str, &str) = ("value", "Value");

/// One `fixentry` struct at one materialization level.
///
/// The fourth member is `fixentries` at every level and the meaning is
/// invariant; the type alone says where materialization stops - a list of
/// deeper entries above [`ENTRY_DEPTH`], the folded text leaf at it. Every
/// entry and every inner list is non-null: an empty list means nothing
/// nested, and it needs no validity bitmap to say so. The leaf is nullable
/// instead, because "nothing was truncated" is an absence and a column that
/// spells it as the empty string cannot be told from one that folded an empty
/// subtree.
fn entry_item(level: usize) -> Result<Field> {
    let tail = if level < ENTRY_DEPTH {
        DataType::list(entry_item(level + 1)?).required_field(FIXENTRIES_COLUMN)
    } else {
        DataType::utf8().nullable_field(FIXENTRIES_COLUMN)
    };
    let named = |dtype: DataType, (name, display): (&str, &str), required: bool| -> Result<Field> {
        let mut field = if required {
            dtype.required_field(name)
        } else {
            dtype.nullable_field(name)
        };
        field.set_display(display)?;
        Ok(field)
    };
    Ok(DataType::from(StructType::from_fields([
        named(DataType::Int32, TAG_COLUMN, true)?,
        named(DataType::utf8(), NAME_COLUMN, true)?,
        named(DataType::utf8(), VALUE_COLUMN, false)?,
        tail,
    ])?)
    .required_field(ENTRY_COMPONENT))
}

/// One entry as the row value its materialization level takes.
///
/// Descendants past [`ENTRY_DEPTH`] fold into the leaf here and only here:
/// the builder never folds, and the Rust tree is never truncated. The leaf is
/// the crate's own JSON over the truncated subtree, as the text a `utf8`
/// column holds, so one serializer and one parser answer for it; an entry
/// that truncated nothing answers null there rather than an empty string,
/// which is what tells "nothing was folded" from "an empty subtree was".
fn entry_scalar(entry: &super::FixEntry, level: usize) -> Result<crate::Scalar> {
    let tail = if level < ENTRY_DEPTH {
        let nested = entry.entries();
        crate::Scalar::try_sequence(nested.len(), |index| {
            entry_scalar(&nested[index], level + 1)
        })?
    } else if entry.entries().is_empty() {
        crate::Scalar::Null
    } else {
        let folded = crate::Scalar::from_sequence(entry.entries().iter().map(folded_scalar));
        let rendered = crate::into_json_scalar(&folded)?;
        crate::Scalar::from(rendered)
    };
    Ok(crate::Scalar::from_sequence([
        crate::Scalar::from(entry.tag()),
        crate::Scalar::from(entry.held_name().clone()),
        entry
            .held_value()
            .cloned()
            .map_or(crate::Scalar::Null, crate::Scalar::from),
        tail,
    ]))
}

/// One truncated entry as the value the leaf's JSON stores.
///
/// The same four members in the same order, the nested entries as a plain
/// array, so a reader walks the decoded value exactly as it walks the
/// materialized levels. Untyped on the way back in, because no finite field
/// describes an unbounded subtree - and every member is an integer or
/// UTF-8, so an untyped decode loses nothing.
fn folded_scalar(entry: &super::FixEntry) -> crate::Scalar {
    crate::Scalar::from_sequence([
        crate::Scalar::from(entry.tag()),
        crate::Scalar::from(entry.held_name().clone()),
        entry
            .held_value()
            .cloned()
            .map_or(crate::Scalar::Null, crate::Scalar::from),
        crate::Scalar::from_sequence(entry.entries().iter().map(folded_scalar)),
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
/// The inverse of [`entry_scalar`], level by level: the five members in the
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
/// `tagname` is read past rather than read: it is what a dictionary made of
/// the tag beside it, so rebuilding the entry from it would give the arrival
/// record two owners of one fact. The message this returns carries the tag
/// and the arrival key, and its [registry](super::FixMsg::registry) answers
/// the name again whenever it is asked for.
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
        .ok_or_else(|| entry_error(path, "a four-member entry", pair.kind()))?;
    let [tag, name, value, tail] = held else {
        return Err(entry_error(path, "four entry members", held.len()));
    };
    let tag = if tag.is_null() {
        0
    } else {
        tag.as_i128()
            .and_then(|value| i32::try_from(value).ok())
            .filter(|value| *value >= 0)
            .ok_or_else(|| {
                entry_error(
                    &path.field(TAG_COLUMN.0),
                    "a nonnegative i32 tag or null",
                    format_args!("{tag:?}"),
                )
            })?
    };
    let name = name
        .as_str()
        .ok_or_else(|| entry_error(&path.field(NAME_COLUMN.0), "text", name.kind()))?;
    let value = if value.is_null() {
        None
    } else {
        Some(smol_str::SmolStr::new(value.as_str().ok_or_else(|| {
            entry_error(&path.field(VALUE_COLUMN.0), "text or null", value.kind())
        })?))
    };
    let nested = entries_from_scalar(tail, &path.field(FIXENTRIES_COLUMN))?;
    Ok(super::FixEntry::new(tag, name, value).with_entries(nested))
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
///
/// The leaf is text and nullable: a null or an empty one folded nothing, and
/// anything else is the crate's own JSON over the subtree it folded.
fn entries_from_scalar(
    value: &crate::Scalar,
    path: &crate::path::Path<'_>,
) -> Result<Vec<super::FixEntry>> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let decoded;
    let value = if let Some(text) = value.as_str() {
        if text.is_empty() {
            return Ok(Vec::new());
        }
        decoded = crate::from_json_scalar(text.as_bytes())
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

/// The content row an arrival record rebuilds: one child per entry, typed
/// through the dictionary as the builder types a pair, in the order the
/// record holds them. Two entries one fold names keep the first.
fn content_from_entries(
    registry: &FixRegistry,
    entries: &[super::FixEntry],
) -> Result<(Vec<Field>, Vec<crate::Scalar>)> {
    let mut fields: Vec<Field> = Vec::with_capacity(entries.len());
    let mut values: Vec<crate::Scalar> = Vec::with_capacity(entries.len());
    for entry in entries {
        let (mut field, value) = child_from_entry(registry, entry, None)?;
        preserve_contended_name(entries, entry, &mut field);
        push_child(registry, &mut fields, &mut values, field, value);
    }
    Ok((fields, values))
}

/// Keeps distinct names when several residual entries contend for one tag.
/// The registry still supplies their datatype and metadata; the row name is
/// the fact that keeps both children addressable instead of collapsing them.
fn preserve_contended_name(
    entries: &[super::FixEntry],
    entry: &super::FixEntry,
    field: &mut Field,
) {
    if entry.tag() != 0
        && entries
            .iter()
            .filter(|held| held.tag() == entry.tag())
            .count()
            > 1
        && !crate::folds_equal(field.name(), entry.name())
    {
        field.set_name(entry.held_name().clone());
    }
}

/// Adds one rebuilt child to a level, as the builder adds one: a group
/// arrives behind its counter's own child, which the group entry's count
/// fills, and two children one fold names keep the first.
fn push_child(
    registry: &FixRegistry,
    fields: &mut Vec<Field>,
    values: &mut Vec<crate::Scalar>,
    field: Field,
    value: crate::Scalar,
) {
    let taken = |fields: &[Field], name: &str| {
        fields
            .iter()
            .any(|held| crate::folds_equal(held.name(), name))
    };
    if let Some(counter) = field.as_fix().counter().ok().flatten() {
        if let Some(scalar) = registry.get_scalar_by_tag(counter) {
            if !taken(fields, scalar.name()) {
                let count = value.as_sequence().map_or(0, <[crate::Scalar]>::len);
                let count = super::build::typed_spelling(registry, scalar, &count.to_string());
                let mut scalar = scalar.clone();
                scalar.set_nullable(count.is_null());
                fields.push(scalar);
                values.push(count);
            }
        }
    }
    if taken(fields, field.name()) {
        return;
    }
    fields.push(field);
    values.push(value);
}

/// Whether one output child names an entry for residual coverage. A group is
/// identified by its counter, while an ordinary child is identified by its
/// tag; a tagless bridge child keeps its folded name.
fn covers_identity(registry: &FixRegistry, field: &Field, entry: &super::FixEntry) -> bool {
    if entry.tag() == 0 {
        return crate::folds_equal(field.name(), entry.name());
    }
    let (tag, counter) = tag_and_counter(registry, field);
    tag == Some(entry.tag()) || counter == Some(entry.tag())
}

/// The one declared member an entry owns. A group column owns its counter's
/// entry before the scalar counter beside it; that scalar is the same fact,
/// checked separately against the occurrence count.
fn covered_member_index(
    registry: &FixRegistry,
    fields: &[Field],
    entry: &super::FixEntry,
) -> Option<usize> {
    let unique = |counter: bool| {
        let mut found = fields.iter().enumerate().filter(|(_, field)| {
            let (tag, held_counter) = tag_and_counter(registry, field);
            if entry.tag() == 0 {
                !counter && crate::folds_equal(field.name(), entry.name())
            } else if counter {
                held_counter == Some(entry.tag())
            } else {
                held_counter.is_none() && tag == Some(entry.tag())
            }
        });
        let (index, _) = found.next()?;
        found.next().is_none().then_some(index)
    };
    unique(true).or_else(|| unique(false))
}

/// Whether a final fitted column value represents one entry whole.
///
/// Coverage is deliberately all or nothing. A group or component stays in
/// the residual when any stated descendant is absent, ambiguous, null or
/// changed by fitting; no subtree is partly removed.
fn covers_entry(
    registry: &FixRegistry,
    field: &Field,
    value: &crate::Scalar,
    entry: &super::FixEntry,
) -> bool {
    if value.is_null() || !covers_identity(registry, field, entry) {
        return false;
    }
    match field.dtype() {
        DataType::Struct(_) => {
            entry.value().is_none()
                && covers_members(registry, field.fields(), value, entry.entries())
        }
        DataType::Sequence(SequenceType::List(item) | SequenceType::LargeList(item)) => {
            let Some(occurrences) = value.as_sequence() else {
                return false;
            };
            if entry.value().and_then(|count| count.parse::<usize>().ok())
                != Some(occurrences.len())
                || entry.entries().len() != occurrences.len()
            {
                return false;
            }
            entry
                .entries()
                .iter()
                .zip(occurrences)
                .all(|(occurrence, value)| match item.dtype() {
                    DataType::Struct(_) => {
                        occurrence.value().is_none()
                            && covers_identity(registry, item, occurrence)
                            && covers_members(registry, item.fields(), value, occurrence.entries())
                    }
                    _ => covers_entry(registry, item, value, occurrence),
                })
        }
        DataType::Mapping(_) | DataType::Sequence(_) => false,
        _ => {
            entry.entries().is_empty()
                && super::entry::wire_text_under(registry, field, value).as_deref() == entry.value()
        }
    }
}

/// Whether every stated member is represented once in a fitted Struct value.
fn covers_members(
    registry: &FixRegistry,
    fields: &[Field],
    value: &crate::Scalar,
    entries: &[super::FixEntry],
) -> bool {
    let Some(values) = value.as_sequence() else {
        return false;
    };
    if fields.len() != values.len() {
        return false;
    }
    let stated = entries.iter().enumerate().all(|(at, entry)| {
        let Some(index) = covered_member_index(registry, fields, entry) else {
            return false;
        };
        if entries[..at]
            .iter()
            .any(|held| covered_member_index(registry, fields, held) == Some(index))
        {
            return false;
        }
        values
            .get(index)
            .is_some_and(|value| covers_entry(registry, &fields[index], value, entry))
    });
    stated
        && fields
            .iter()
            .zip(values)
            .enumerate()
            .filter(|(_, (_, value))| !value.is_null())
            .all(|(index, (field, value))| {
                if entries
                    .iter()
                    .any(|entry| covered_member_index(registry, fields, entry) == Some(index))
                {
                    return true;
                }
                // A group's scalar counter is represented by the same entry
                // as its List. It is covered only when both fitted values say
                // the same occurrence count.
                let (tag, counter) = tag_and_counter(registry, field);
                let Some(tag) = tag.filter(|_| counter.is_none()) else {
                    return false;
                };
                let mut groups = fields
                    .iter()
                    .enumerate()
                    .filter(|(_, held)| tag_and_counter(registry, held).1 == Some(tag));
                let Some((group_index, _)) = groups.next() else {
                    return false;
                };
                if groups.next().is_some() {
                    return false;
                }
                let mut owners = entries.iter().filter(|entry| entry.tag() == tag);
                let Some(owner) = owners.next() else {
                    return false;
                };
                owners.next().is_none()
                    && covered_member_index(registry, fields, owner) == Some(group_index)
                    && values[group_index]
                        .as_sequence()
                        .is_some_and(|occurrences| {
                            value.as_i128() == i128::try_from(occurrences.len()).ok()
                        })
            })
}

/// Settle the one member order every occurrence agrees with. Encounter order
/// breaks ties between unrelated members; adjacent members in an occurrence
/// are ordering constraints, and contradictory constraints refuse the row.
fn ordered_group_union(
    union: Vec<Field>,
    stated: &[Vec<(SmolStr, crate::Scalar)>],
    group: &str,
) -> Result<Vec<Field>> {
    let refusal = |reason: String| {
        let root = crate::path::Path::root();
        let entries = root.field(FIXENTRIES_COLUMN);
        let group = entries.field(group);
        crate::Error::InvalidRecord {
            path: group.render().into(),
            reason: reason.into(),
        }
    };
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for occurrence in stated {
        let mut previous = None;
        for (name, _) in occurrence {
            let Some(index) = union
                .iter()
                .position(|field| crate::folds_equal(field.name(), name))
            else {
                return Err(refusal(format!(
                    "member {name} is absent from its group schema"
                )));
            };
            if let Some(before) = previous {
                if before != index {
                    edges.push((before, index));
                }
            }
            previous = Some(index);
        }
    }
    edges.sort_unstable();
    edges.dedup();
    if edges.iter().all(|(before, after)| before < after) {
        return Ok(union);
    }

    let mut incoming = vec![0_usize; union.len()];
    for (_, after) in &edges {
        incoming[*after] += 1;
    }
    let mut selected = vec![false; union.len()];
    let mut order = Vec::with_capacity(union.len());
    while order.len() < union.len() {
        let Some(next) = (0..union.len()).find(|index| !selected[*index] && incoming[*index] == 0)
        else {
            let unresolved = union
                .iter()
                .enumerate()
                .filter(|(index, _)| !selected[*index])
                .map(|(_, field)| field.name())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(refusal(format!(
                "contradictory member order among {unresolved}"
            )));
        };
        selected[next] = true;
        order.push(next);
        for (before, after) in &edges {
            if *before == next {
                incoming[*after] -= 1;
            }
        }
    }
    let mut fields: Vec<Option<Field>> = union.into_iter().map(Some).collect();
    Ok(order
        .into_iter()
        .map(|index| fields[index].take().expect("each member is selected once"))
        .collect())
}

/// One group entry as the list it states: an occurrence per entry under it,
/// each holding the members that occurrence stated, laid out on the union of
/// every occurrence's members in their consistent relative order.
///
/// `item` is the occurrence the dictionary declares, where it declares one:
/// a member it names types through its own field, and the list keeps the
/// group's storage. A group no dictionary declares - what a bridge packs
/// under a counter's own name - takes its occurrence name from the entries
/// and lands as a plain `List`, which is what the builder made of it.
fn group_from_entry(
    registry: &FixRegistry,
    entry: &super::FixEntry,
    known: &Field,
    item: Option<&Field>,
) -> Result<(Field, crate::Scalar)> {
    let declared = item
        .and_then(|item| item.dtype().as_fields())
        .unwrap_or_default();
    let mut union: Vec<Field> = Vec::new();
    let mut stated: Vec<Vec<(SmolStr, crate::Scalar)>> = Vec::with_capacity(entry.entries().len());
    for occurrence in entry.entries() {
        let mut fields = Vec::with_capacity(occurrence.entries().len());
        let mut values = Vec::with_capacity(occurrence.entries().len());
        for member in occurrence.entries() {
            let slot = covered_member_index(registry, declared, member)
                .and_then(|index| declared.get(index));
            let (mut field, value) = child_from_entry(registry, member, slot)?;
            preserve_contended_name(occurrence.entries(), member, &mut field);
            push_child(registry, &mut fields, &mut values, field, value);
        }
        let mut members = Vec::with_capacity(fields.len());
        for (field, value) in fields.into_iter().zip(values) {
            match union
                .iter_mut()
                .find(|held| crate::folds_equal(held.name(), field.name()))
            {
                // Two occurrences state one member differently - a nested
                // group one of them left out a level of - so the slot is
                // what both fit in, which is what the members' own contract
                // answers for the pair.
                Some(slot) => {
                    if slot.dtype() != field.dtype() {
                        let mut widened = slot.merge_with(&field, true)?;
                        widened.set_nullable(true);
                        *slot = widened;
                    }
                }
                None => {
                    let mut slot = field.clone();
                    slot.set_nullable(true);
                    union.push(slot);
                }
            }
            members.push((SmolStr::new(field.name()), value));
        }
        stated.push(members);
    }
    let union = ordered_group_union(union, &stated, entry.name())?;
    // Each occurrence is stated by name rather than by position, so the
    // members land in the slots the union settled on whatever order the
    // occurrence stated them in: the root's own contract places a record.
    let rows: Vec<crate::Scalar> = stated
        .into_iter()
        .map(crate::Scalar::from_struct)
        .collect::<Result<Vec<_>>>()?;
    let name = item.map_or_else(
        || {
            entry
                .entries()
                .first()
                .map_or_else(|| known.name().to_owned(), |held| held.name().to_owned())
        },
        |item| item.name().to_owned(),
    );
    let occurrence = DataType::from(StructType::from_fields(union)?).required_field(name);
    let dtype = match known.dtype() {
        DataType::Sequence(SequenceType::LargeList(_)) => DataType::large_list(occurrence),
        DataType::Mapping(map) => DataType::map(occurrence, map.keys_sorted())?,
        _ => DataType::list(occurrence),
    };
    // A group the dictionary does not declare stands under the counter's own
    // name and states no counter of its own: the builder left the count to
    // the occurrences beside it, and a counter here would put a second child
    // of that name in the row.
    let field = Field::new_with_metadata(known.name(), dtype, true, known.as_metadata().clone());
    Ok((field, crate::Scalar::from_sequence(rows)))
}

/// A scalar stated at indexed positions as the List the builder made of it.
/// If every spelling types, the dictionary's scalar remains the item; if one
/// does not, every raw spelling stays under UTF-8 so no sibling changes type
/// or disappears on a later write.
fn scalar_list_from_entry(
    registry: &FixRegistry,
    entry: &super::FixEntry,
    known: &Field,
) -> Result<(Field, crate::Scalar)> {
    let typed: Vec<crate::Scalar> = entry
        .entries()
        .iter()
        .map(|occurrence| {
            occurrence.value().map_or(crate::Scalar::Null, |text| {
                super::build::typed_spelling_remembered(registry, known, text)
            })
        })
        .collect();
    let raw = entry
        .entries()
        .iter()
        .zip(&typed)
        .any(|(occurrence, value)| occurrence.value().is_some() && value.is_null());
    let mut item = if raw {
        Field::new_with_metadata(
            entry
                .entries()
                .first()
                .map_or(entry.name(), |held| held.name()),
            DataType::utf8(),
            true,
            known.as_metadata().clone(),
        )
    } else {
        known.clone()
    };
    if let Some(occurrence) = entry.entries().first() {
        item.set_name(occurrence.held_name().clone());
    }
    let values = if raw {
        entry
            .entries()
            .iter()
            .map(|occurrence| {
                occurrence
                    .value()
                    .map_or(crate::Scalar::Null, crate::Scalar::from)
            })
            .collect()
    } else {
        typed
    };
    let field = Field::new_with_metadata(
        entry.held_name().clone(),
        DataType::list(item),
        true,
        known.as_metadata().clone(),
    );
    Ok((field, crate::Scalar::from_sequence(values)))
}

/// An unexplained nested entry as the shape its children state. Repeated
/// tagless occurrence names identify a List; otherwise the children are the
/// members of one Struct component.
fn unknown_nested_from_entry(
    registry: &FixRegistry,
    entry: &super::FixEntry,
) -> Result<(Field, crate::Scalar)> {
    let first = entry.entries().first();
    let repeated = entry.entries().len() > 1
        && first.is_some_and(|first| {
            first.tag() == 0
                && entry
                    .entries()
                    .iter()
                    .all(|held| held.tag() == 0 && crate::folds_equal(held.name(), first.name()))
        });
    if repeated {
        let mut known = DataType::utf8().nullable_field(entry.held_name().clone());
        if entry.tag() != 0 {
            known.as_fix_mut().set_tag(entry.tag())?;
        }
        if entry
            .entries()
            .iter()
            .all(|occurrence| occurrence.entries().is_empty())
        {
            return scalar_list_from_entry(registry, entry, &known);
        }
        return group_from_entry(registry, entry, &known, None);
    }
    let mut fields: Vec<Field> = Vec::with_capacity(entry.entries().len());
    let mut values: Vec<crate::Scalar> = Vec::with_capacity(entry.entries().len());
    for member in entry.entries() {
        let (mut field, value) = child_from_entry(registry, member, None)?;
        preserve_contended_name(entry.entries(), member, &mut field);
        push_child(registry, &mut fields, &mut values, field, value);
    }
    let named: Vec<(SmolStr, crate::Scalar)> = fields
        .iter()
        .map(|field| SmolStr::new(field.name()))
        .zip(values)
        .collect();
    let mut field =
        DataType::from(StructType::from_fields(fields)?).nullable_field(entry.held_name().clone());
    if entry.tag() != 0 {
        field.as_fix_mut().set_tag(entry.tag())?;
    }
    Ok((field, crate::Scalar::from_struct(named)?))
}

/// One entry as the child it states and the value under it.
///
/// `declared` is the field the enclosing level declares for it - a
/// component's member, a group's occurrence member - where one does; else
/// the dictionary answers by tag, then by name; an entry neither explains
/// is the `utf8` child the builder keeps an unexplained key as. A group's
/// occurrences hold the members they stated, in the order they stated
/// them, as the builder lays them out; a component holds its stated
/// members; a scalar types its wire spelling under its field. A spelling the
/// field will not hold stays as UTF-8 under the same name and metadata, so an
/// unrelated later write cannot erase residual input.
fn child_from_entry(
    registry: &FixRegistry,
    entry: &super::FixEntry,
    declared: Option<&Field>,
) -> Result<(Field, crate::Scalar)> {
    // A declared nested member is what the level declares; a declared
    // scalar is the dictionary's own field, as the builder states one,
    // rather than the reference the level declares it through. Then the
    // canonical name before the tag: a counter's tag names the counter and
    // the group it heads, and the entry's name says which of the two it is.
    let known = declared
        .and_then(|held| {
            if held.dtype().is_nested() {
                return Some(held);
            }
            held.as_fix()
                .tag()
                .ok()
                .flatten()
                .and_then(|tag| registry.get_scalar_by_tag(tag))
                .or(Some(held))
        })
        .or_else(|| registry.get_field_by_name(entry.name()))
        .or_else(|| (entry.tag() != 0).then(|| registry.get_field_by_tag(entry.tag()))?);
    let Some(known) = known else {
        if !entry.entries().is_empty() {
            return unknown_nested_from_entry(registry, entry);
        }
        let mut field = DataType::utf8().nullable_field(entry.name());
        if entry.tag() != 0 {
            field.as_fix_mut().set_tag(entry.tag())?;
        }
        let value = entry
            .value()
            .map_or(crate::Scalar::Null, crate::Scalar::from);
        return Ok((field, value));
    };
    // A scalar the entries state occurrences under is a group no dictionary
    // declares - a bridge packs `NOTRADINGSESSIONS[0]=...` under the
    // counter's own name - and the entries are the only statement of its
    // shape. It rebuilds as the list it is rather than as the scalar the
    // name reaches, which would answer null and lose the occurrences.
    if !known.dtype().is_nested() && !entry.entries().is_empty() {
        if entry
            .entries()
            .iter()
            .all(|occurrence| occurrence.entries().is_empty())
        {
            return scalar_list_from_entry(registry, entry, known);
        }
        return group_from_entry(registry, entry, known, None);
    }
    match known.dtype() {
        DataType::Sequence(SequenceType::List(_))
        | DataType::Sequence(SequenceType::LargeList(_))
        | DataType::Mapping(_) => {
            let Some(item) = super::catalog::occurrence_of(known) else {
                return Ok((known.clone(), crate::Scalar::Null));
            };
            group_from_entry(registry, entry, known, Some(item))
        }
        DataType::Struct(_) => {
            let mut fields: Vec<Field> = Vec::with_capacity(entry.entries().len());
            let mut values: Vec<crate::Scalar> = Vec::with_capacity(entry.entries().len());
            for member in entry.entries() {
                let slot = covered_member_index(registry, known.fields(), member)
                    .and_then(|index| known.fields().get(index));
                let (mut field, value) = child_from_entry(registry, member, slot)?;
                preserve_contended_name(entry.entries(), member, &mut field);
                push_child(registry, &mut fields, &mut values, field, value);
            }
            let named: Vec<(SmolStr, crate::Scalar)> = fields
                .iter()
                .map(|field| SmolStr::new(field.name()))
                .zip(values)
                .collect();
            let field = Field::new_with_metadata(
                known.name(),
                DataType::from(StructType::from_fields(fields)?),
                false,
                known.as_metadata().clone(),
            );
            Ok((field, crate::Scalar::from_struct(named)?))
        }
        _ => {
            let value = entry.value().map_or(crate::Scalar::Null, |text| {
                super::build::typed_spelling_remembered(registry, known, text)
            });
            if value.is_null() {
                if let Some(text) = entry.value() {
                    let field = Field::new_with_metadata(
                        entry.held_name().clone(),
                        DataType::utf8(),
                        true,
                        known.as_metadata().clone(),
                    );
                    return Ok((field, crate::Scalar::from(text)));
                }
            }
            let mut field = known.clone();
            field.set_nullable(value.is_null());
            Ok((field, value))
        }
    }
}

impl super::FixMsg {
    /// The message a fixed row holds: the inverse of [`Self::into_row`].
    ///
    /// The root is `schema` and the value is `row`, checked and canonicalized
    /// exactly as [`Self::with_registry`] checks one. The typed facts - the
    /// header, the event, the capture, the text and the metadata - are read
    /// off the columns that hold them. Ordinary tagged columns rebuild the
    /// content they represent, and [`FIXENTRIES_COLUMN`] supplies everything
    /// that no column represented. A residual entry owns its tag where both
    /// are present, so an unreadable or partial value is never replaced by a
    /// column's narrower reading.
    ///
    /// # The capture's own columns are carried
    ///
    /// A column no tag and no counter names is the capture's own - the body
    /// the line was cut from, its place in the object, its media type, what
    /// a bound dropped - and so is the one the crate does tag,
    /// [`sourceurl`](crate::SOURCEURL_TAG_NAME). All of them are carried:
    /// each cell holding a value is read into the message under its
    /// column's name, as [`Self::carried`] answers it, and none of them is
    /// content - a message is what parsing one line answered, and what a
    /// *reader* said about that line is not it - so none can reach a
    /// digest, an entry, or a `body=` at a counterparty, and no rule has to
    /// keep telling one apart from a key no dictionary explains.
    ///
    /// [`Self::into_row`] states each again at the column of its name,
    /// which is how a row read back here and written again keeps what it
    /// said for itself, and how a walk answers messages in their own order
    /// with each row's cells still beside the message they were read with.
    ///
    /// Nothing is parsed again: this is what makes a batch of rows a stream
    /// of messages at the cost of the values it already holds. A row without
    /// the entries column still rebuilds the content its tagged columns state;
    /// content projected out of both places is absent.
    /// Residual values use canonical wire text: non-text binary data retains
    /// its decoded spelling there, while a binary column can preserve bytes.
    ///
    /// A fixed row is a semantic representation rather than an arrival-order
    /// transcript. Reading it back may order represented content by its
    /// columns and render its canonical wire spelling. A complete row keeps
    /// the event identity it recorded instead of deriving a different one
    /// from that ordering; a row without the identity columns is settled from
    /// the facts it does carry.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::local::Folder;
    /// # use yggdryl::graph::Element;
    /// # use yggdryl::{FixCodec, FixMsg, FixRegistry, fix_schema};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    /// let schema = fix_schema(&registry, "fix")?;
    /// let reader = FixCodec::new(Arc::clone(&registry));
    /// let line = b"8=FIX.4.4|35=D|52=20240102-10:15:30|54=1|11=A1|55=AAPL|9999=x|10=0|";
    /// let order = reader.parse_fix_line(line)?;
    ///
    /// let row = order.into_row(&schema)?;
    /// let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row)?;
    ///
    /// // The same facts are reached through the semantic row.
    /// assert_eq!(held.by_tag(55)?, order.by_tag(55)?);
    /// assert_eq!(held.get_currhashcode(), order.get_currhashcode());
    /// // And the row it came from is its fixed point.
    /// assert_eq!(held.into_row(&schema)?, row);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the schema's refusal when the row does not fit, or
    /// [`crate::Error::InvalidRecord`] at the arrival path when the entries
    /// column holds something that is not an arrival entry, a folded leaf that
    /// does not decode included.
    pub fn from_row(
        registry: Arc<FixRegistry>,
        schema: &Field,
        row: &crate::Scalar,
    ) -> Result<Self> {
        let value = schema.canonicalize_value(row.clone())?;
        Self::rebuilt(registry, schema, &value)
    }

    /// [`Self::from_row`] for a row read straight out of an Arrow batch
    /// under `schema`: the array reader answered it in the schema's own
    /// canonical form, so it is validated against the schema - a null
    /// where the schema requires a value, a text past the bound a column
    /// states, which Arrow does not police below the root - and rebuilt as
    /// it stands rather than canonicalized a second time.
    pub(super) fn from_arrow_row(
        registry: Arc<FixRegistry>,
        schema: &Field,
        row: &crate::Scalar,
    ) -> Result<Self> {
        crate::value::validate_row(schema, row)?;
        Self::rebuilt(registry, schema, row)
    }

    /// The message a canonical row of `schema` holds.
    fn rebuilt(registry: Arc<FixRegistry>, schema: &Field, value: &crate::Scalar) -> Result<Self> {
        let plan = column_plan_of(schema, &registry)?;
        let mut members: Vec<Field> = Vec::with_capacity(schema.fields().len());
        let mut values: Vec<crate::Scalar> = Vec::with_capacity(schema.fields().len());
        let mut residual: Option<Vec<super::FixEntry>> = None;
        let mut projected: Vec<(i32, Field, crate::Scalar)> = Vec::new();
        let mut carried: Vec<(smol_str::SmolStr, crate::Scalar)> = Vec::new();
        let held = value.as_sequence().unwrap_or_default();
        for ((column, planned), value) in schema.fields().iter().zip(plan.iter()).zip(held) {
            if column.name() == FIXENTRIES_COLUMN {
                let root = crate::path::Path::root();
                residual = Some(entries_from_scalar(value, &root.field(FIXENTRIES_COLUMN))?);
                continue;
            }
            if column.name() == NOFIXENTRIES_COLUMN {
                continue;
            }
            match planned.tag {
                Some(tag) if super::identity::is_typed_tag(tag) => {
                    members.push(column.clone());
                    values.push(value.clone());
                }
                // The one the crate tags - `sourceurl` - is the capture's
                // own, carried under its name as the untagged ones below.
                Some(tag) if super::identity::is_capture_tag(tag) => {
                    if !value.is_null() {
                        carried.push((smol_str::SmolStr::new(column.name()), value.clone()));
                    }
                }
                Some(tag) => {
                    if !value.is_null() {
                        projected.push((
                            planned.counter.unwrap_or(tag),
                            column.clone(),
                            value.clone(),
                        ));
                    }
                }
                None if planned.counter.is_some() => {
                    if !value.is_null() {
                        projected.push((
                            planned.counter.expect("the guarded counter"),
                            column.clone(),
                            value.clone(),
                        ));
                    }
                }
                // A key spelled under a namespace is a bridge's own
                // statement and the message's to keep: it is carried over
                // so the assembly below lands it in the metadata, exactly
                // as a parsed line's is.
                None if column.name().contains('.') => {
                    members.push(column.clone());
                    values.push(value.clone());
                }
                // Any other column no tag and no counter names is the
                // capture's own: the body the line was cut from, its place
                // in the object, its media type, what a bound dropped. Read
                // into the message as the cells it carries, and never as a
                // child - a column that became content would be an entry, a
                // digest input, a `body=` at a counterparty - so the row
                // written again states each at its column and the message
                // is the message the line parsed into.
                None => {
                    if !value.is_null() {
                        carried.push((smol_str::SmolStr::new(column.name()), value.clone()));
                    }
                }
            }
        }
        let residual = residual.unwrap_or_default();
        if !residual.is_empty() {
            let (fields, held) = content_from_entries(&registry, &residual)?;
            for (field, value) in fields.into_iter().zip(held) {
                if members
                    .iter()
                    .any(|known| crate::folds_equal(known.name(), field.name()))
                {
                    continue;
                }
                members.push(field);
                values.push(value);
            }
        }
        // A residual entry is the authoritative statement of its tag. Every
        // other non-null projected value is content the row stated only in
        // its column, so rebuild it through the same child insertion rule as
        // an entry.
        for (tag, field, value) in projected {
            if let Some(owner) = residual.iter().find(|entry| entry.tag() == tag) {
                // A group entry owns the occurrences, while its separate
                // scalar counter column owns an explicitly stated count (or
                // the derived count a semantic row records). Keep that scalar
                // beside the group so a conflicting explicit count is not
                // silently replaced by the occurrence length.
                if !owner.entries().is_empty() && !field.dtype().is_nested() {
                    if let Some(index) = members.iter().position(|member| {
                        !member.dtype().is_nested()
                            && tag_and_counter(&registry, member).0 == Some(tag)
                    }) {
                        members[index] = field;
                        values[index] = value;
                    } else {
                        push_child(&registry, &mut members, &mut values, field, value);
                    }
                }
                continue;
            }
            push_child(&registry, &mut members, &mut values, field, value);
        }
        // The columns are the schema's, validated when it was built, and
        // the content is the dictionary's fields, each kept only where no
        // column folds to its name: named once by construction, so the
        // root is built as it stands rather than validated once per row.
        let root = Field::new_with_metadata(
            schema.name(),
            DataType::from(StructType::from_unique_fields(members)),
            schema.is_nullable(),
            schema.as_metadata().clone(),
        );
        let retains_identity = [
            super::CURRUNIX_TAG_NAME.0,
            super::CREAUNIX_TAG_NAME.0,
            super::CURRHASHCODE_TAG_NAME.0,
            super::CROSSHASHCODE_TAG_NAME.0,
            super::CURRUUID_TAG_NAME.0,
            super::CROSSUUID_TAG_NAME.0,
        ]
        .into_iter()
        .all(|tag| {
            plan.iter().zip(held).any(|(column, value)| {
                column.tag == Some(tag) && !column.shared && !value.is_null()
            })
        });
        let mut message = Self::from_rebuilt_row(
            registry,
            root,
            crate::Scalar::from_sequence(values),
            retains_identity,
        )?;
        message.set_carried(carried);
        Ok(message)
    }

    /// This message as the fixed row a table holds.
    ///
    /// The columns are the schema's own, in its own order, and each is filled
    /// by its scalar tag or logical group's `FIX:counter`. A message carrying
    /// nothing at a column answers null there rather than shifting its neighbours, which
    /// is what makes two rows of one capture comparable at all.
    ///
    /// [The capture's own columns](Self::carried) - the one the crate tags,
    /// `sourceurl`, and every column no tag and no counter names - answer the
    /// cell the message carries under the column's name, and null where it
    /// carries none: a message holds no fact for any of them, and what it
    /// carries is what whoever read its row stated. Nothing derives one:
    /// where a line was read from is whoever read it to say.
    ///
    /// The residual record closes the row under [`FIXENTRIES_COLUMN`]. A
    /// scalar or complete group successfully represented by one unambiguous
    /// column is omitted from it. Unknown, unprojected, conflicting, partially
    /// represented and unreadable content remains there whole; keys no
    /// dictionary explained keep tag zero and their text intact.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::local::Folder;
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
    /// // An unresolved numeric key stays in the one arrival record as tag zero,
    /// // named after itself rather than nulled, with its value as it arrived.
    /// let entries = held.last().and_then(yggdryl::Scalar::as_sequence).unwrap();
    /// let entry = entries.iter().find(|entry| entry.get(1).and_then(yggdryl::Scalar::as_str) == Some("9999")).unwrap();
    /// assert_eq!(entry.get(0).and_then(yggdryl::Scalar::as_i128), Some(0));
    /// assert_eq!(entry.get(2).and_then(yggdryl::Scalar::as_str), Some("x"));
    /// # Ok(())
    /// # }
    /// ```
    /// A value a column will not hold is that column's null rather than a
    /// refusal - a five-byte MIC under a four-byte column, an identifier
    /// whose check digit does not close - and the residual keeps what the
    /// column could not represent. A column that cannot be null keeps the
    /// refusal, which separates an unreadable value from a broken contract.
    ///
    /// # Errors
    ///
    /// Refuses a missing or mistyped mandatory replay holder, a value a
    /// column that cannot be null will not hold, or an unrenderable truncated
    /// arrival subtree.
    pub fn into_row(&self, schema: &Field) -> Result<crate::Scalar> {
        let plan = column_plan_of(schema, self.registry())?;
        let columns = schema.fields();
        // The crate columns' derivations and the working row they read,
        // gathered off this message once for the three of them, and only
        // when one is asked for.
        let mut derived: Option<(Arc<super::enrich::Derivations>, Vec<crate::Scalar>)> = None;
        let has_residual_columns = columns
            .iter()
            .any(|column| matches!(column.name(), FIXENTRIES_COLUMN | NOFIXENTRIES_COLUMN));
        let mut fitted_cell = |index: usize| -> Result<crate::Scalar> {
            let column = &columns[index];
            let planned = &plan[index];
            if matches!(column.name(), FIXENTRIES_COLUMN | NOFIXENTRIES_COLUMN) {
                return Ok(crate::Scalar::Null);
            }
            let value = match (planned.tag, planned.counter) {
                // A column declaring a group's `FIX:counter` answers with
                // that group, read from the message's own occurrences. Any
                // other column answers for the tag its field carries; one
                // that carries none is a capture's own column, which the
                // message answers from the cell it carries under that name,
                // else from a content child spelled exactly as it is.
                // A typed fact is its holder's, whatever shape its
                // column takes: the identifiers Map is the event's.
                (Some(tag), _) if super::identity::is_typed_tag(tag) => {
                    self.column_value(tag, &mut derived)?
                }
                // The one the crate tags - `sourceurl` - is carried.
                (Some(tag), _) if super::identity::is_capture_tag(tag) => {
                    self.carried_cell(column.name())
                }
                (_, Some(counter)) => {
                    let value = self
                        .index_of_group(counter)
                        .and_then(|index| self.as_value().get(index))
                        .cloned()
                        .unwrap_or(crate::Scalar::Null);
                    self.regrouped(counter, column, value)
                }
                (Some(tag), None) => {
                    self.regrouped(tag, column, self.column_value(tag, &mut derived)?)
                }
                (None, None) => match self.carried_cell(column.name()) {
                    crate::Scalar::Null => self
                        .index_of_name(column.name())
                        .and_then(|at| self.as_value().get(at))
                        .cloned()
                        .unwrap_or(crate::Scalar::Null),
                    carried => carried,
                },
            };
            fitted(column, value)
        };

        if !has_residual_columns {
            return crate::Scalar::try_sequence(columns.len(), fitted_cell);
        }
        let mut values = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            values.push(fitted_cell(index)?);
        }
        // A group occurrence none of the fixed List's members can represent
        // belongs wholly to the residual record. Its scalar counter must stay
        // there with it: projecting the count beside a null List would claim
        // that the fixed group represented occurrences it cannot describe.
        // A bare scalar counter with no group still stands as stated.
        for (group_index, group) in plan.iter().enumerate() {
            let Some(counter) = group.counter else {
                continue;
            };
            if !values[group_index].is_null() || self.index_of_group(counter).is_none() {
                continue;
            }
            if let Some(counter_index) = plan
                .iter()
                .position(|held| held.tag == Some(counter) && held.counter.is_none())
            {
                values[counter_index] = crate::Scalar::Null;
            }
        }
        let entries = self.entries();
        let prunes = plan.iter().any(|column| column.entries);
        let mut represented = Vec::with_capacity(if prunes {
            columns.len().min(entries.len())
        } else {
            0
        });
        if prunes {
            for (index, (column, planned)) in columns.iter().zip(plan.iter()).enumerate() {
                let Some(tag) = planned.counter.or(planned.tag) else {
                    continue;
                };
                if planned.shared || values[index].is_null() {
                    continue;
                }
                let source = match planned.counter {
                    Some(counter) => self.index_of_group(counter),
                    None => self.unique_index_of_tag(tag),
                };
                let Some(source) = source else {
                    continue;
                };
                let Some(source_field) = self.as_field().get_field_at(source) else {
                    continue;
                };
                let Some(source_value) = self.as_value().get(source) else {
                    continue;
                };
                let mut matching = entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| entry.tag() == tag);
                let Some((entry_index, entry)) = matching.next() else {
                    continue;
                };
                if matching.next().is_some() {
                    continue;
                }
                if column.dtype().is_nested() {
                    if planned.counter.is_some()
                        && plan
                            .iter()
                            .filter(|held| held.counter == planned.counter)
                            .count()
                            != 1
                    {
                        continue;
                    }
                    if !source_field.dtype().is_nested()
                        || !covers_entry(self.registry(), column, &values[index], entry)
                    {
                        continue;
                    }
                } else if source_field.dtype().is_nested()
                    || !crate::folds_equal(source_field.name(), entry.name())
                    || !entry.entries().is_empty()
                    || source_value != &values[index]
                    || !covers_entry(self.registry(), column, &values[index], entry)
                {
                    continue;
                }
                represented.push(entry_index);
            }
            represented.sort_unstable();
            represented.dedup();
        }
        let mut residual = Vec::with_capacity(entries.len() - represented.len());
        for (index, entry) in entries.iter().enumerate() {
            if represented.binary_search(&index).is_err() {
                residual.push(entry);
            }
        }
        let record = columns
            .iter()
            .any(|column| column.name() == FIXENTRIES_COLUMN)
            .then(|| {
                crate::Scalar::try_sequence(residual.len(), |index| {
                    entry_scalar(residual[index], 1)
                })
            })
            .transpose()?;
        let count = crate::Scalar::from(i32::try_from(residual.len()).unwrap_or(i32::MAX));
        for (index, (column, planned)) in columns.iter().zip(plan.iter()).enumerate() {
            values[index] = match column.name() {
                FIXENTRIES_COLUMN => {
                    let record = record.clone().unwrap_or(crate::Scalar::Null);
                    if planned.entries {
                        record
                    } else {
                        fitted(column, record)?
                    }
                }
                NOFIXENTRIES_COLUMN => fitted(column, count.clone())?,
                _ => continue,
            };
        }
        Ok(crate::Scalar::from_sequence(values))
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
        let spelled = self
            .index_of_group(tag)
            .and_then(|at| self.as_field().get_field_at(at))
            .and_then(item_fields)
            .unwrap_or_default();
        // Where each declared member stands among the message's own, a fact
        // of the two schemas alone and so read once for every occurrence.
        let placed: Vec<(Option<usize>, Option<usize>)> = members
            .iter()
            .map(|member| {
                let at = spelled
                    .iter()
                    .position(|field| crate::folds_equal(field.name(), member.name()));
                // A group counter may be absent from the message's physical
                // member row while its group is present. Resolve that
                // relationship once for the schemas, then state the count
                // only where no explicit non-null counter value exists.
                let group_at = if member.dtype().is_nested() {
                    None
                } else {
                    let (Some(tag), None) = tag_and_counter(self.registry(), member) else {
                        return (at, None);
                    };
                    let mut groups = spelled.iter().enumerate().filter_map(|(index, field)| {
                        (tag_and_counter(self.registry(), field).1 == Some(tag)).then_some(index)
                    });
                    let first = groups.next();
                    if first.is_some() && groups.next().is_none() {
                        first
                    } else {
                        None
                    }
                };
                (at, group_at)
            })
            .collect();
        if !occurrences.is_empty()
            && !placed
                .iter()
                .any(|(at, group_at)| at.is_some() || group_at.is_some())
        {
            return crate::Scalar::Null;
        }
        crate::Scalar::from_sequence(occurrences.iter().map(|occurrence| {
            let Some(stated) = occurrence.as_sequence() else {
                return occurrence.clone();
            };
            crate::Scalar::from_sequence(placed.iter().map(|(at, group_at)| {
                let explicit = at.and_then(|at| stated.get(at));
                if let Some(value) = explicit.filter(|value| !value.is_null()) {
                    return value.clone();
                }
                group_at
                    .and_then(|at| stated.get(at))
                    .and_then(crate::Scalar::as_sequence)
                    .map(|occurrences| {
                        crate::Scalar::from(i32::try_from(occurrences.len()).unwrap_or(i32::MAX))
                    })
                    .unwrap_or(crate::Scalar::Null)
            }))
        }))
    }

    /// One column's value, derived where the message does not carry it.
    ///
    /// Three sources, in this order. What the message actually said, always,
    /// because a stated value is never overridden. Then the lift's own
    /// enrichment, which fills a column the message did not state but
    /// forced: a buy order at a price is a party willing to pay it, so a bid
    /// lane it never wrote is still true of it, and a one-sided quote implies
    /// the side it never wrote either. Then the derived facts this crate
    /// computes - `BeginString` here, and every crate column whose field
    /// declares a `FIX:derivation` through the one evaluator the
    /// [enriching pass](super::enrich) runs, so `isincode`, `miccode` and
    /// `state` fill a row of an unenriched message exactly as the pass
    /// would fill the message.
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
    /// field whose `FIX:derivation` does not compile: a dictionary whose
    /// rules do not compile fills no row, exactly as it enriches no message.
    fn column_value(
        &self,
        tag: i32,
        derived: &mut Option<(Arc<super::enrich::Derivations>, Vec<crate::Scalar>)>,
    ) -> Result<crate::Scalar> {
        // A sending clock the message never stated is intake's stand-in for
        // one rather than a fact of the message: it is what the instant and
        // the creation were settled from, and each of those has a column of
        // its own. Stating it here would make the row read back as a message
        // that stated a clock, and that message re-emits a `52=` its line
        // never carried.
        if tag == 52 && !self.header().stated_sendingtime() {
            return Ok(crate::Scalar::Null);
        }
        // A stated value wins - a stated null is a value that would not
        // type, and the derivation still answers for it.
        if let Some(held) = self.get_by_tag(tag).filter(|held| !held.is_null()) {
            return Ok(held);
        }
        // A group is the statement its counter counts. Where no scalar
        // counter was stated, derive the column from the occurrences; an
        // explicit non-null counter returned above remains authoritative even
        // when it conflicts.
        if let Some(count) = self
            .index_of_group(tag)
            .and_then(|index| self.as_value().get(index))
            .and_then(crate::Scalar::as_sequence)
            .map(<[crate::Scalar]>::len)
        {
            return Ok(crate::Scalar::from(
                i32::try_from(count).unwrap_or(i32::MAX),
            ));
        }
        // The version a message that states none is said to be read at,
        // derived here for a message built from a schema and a value exactly
        // as the builder stamps one it parsed.
        if tag == 8 {
            let version = super::build::default_version();
            return Ok(crate::Scalar::from(format!("FIX.{version}")));
        }
        Ok(if tag == super::cfi::CFICODE_TAG {
            // FIX's own tag rather than a column of this crate's: 461 is
            // already where a message states its classification, so filling
            // it to the maximum the message licenses is the whole job and a
            // second column would be a second owner of one fact.
            //
            // Only reached when the message stated nothing there - a stated
            // value returned above, because a stated value is never
            // overwritten. `classification` merges a partial stated code with
            // what the rest of the message says; this is the other half of it,
            // where there was nothing stated to merge with.
            self.get_cficode()
                .cloned()
                .map(crate::Scalar::CfiCode)
                .or_else(|| self.classification().map(crate::Scalar::from))
                .unwrap_or(crate::Scalar::Null)
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
/// A required crate column is exactly such a contract.
fn fitted(column: &Field, value: crate::Scalar) -> Result<crate::Scalar> {
    let value = narrowed(column, value);
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

/// A price or a quantity this crate settled, narrowed to the column that
/// holds it.
///
/// This crate keeps a price and a quantity exact - `decimal128(38, 18)` -
/// because a price is money and a float is not; FIX's own `BidPx`, `BidSize`,
/// `OfferPx` and `OfferSize` are `Price` and `Qty` fields, which the
/// dictionary types as `float64` because that is what their text always was.
/// A lane the message never wrote and this crate filled therefore meets a
/// float column, and the narrowing is made here, once, rather than at each
/// lane: the alternative is a null, which loses the fact the fill was for.
/// Nothing else narrows - a decimal column takes the decimal whole.
pub(super) fn narrowed(column: &Field, value: crate::Scalar) -> crate::Scalar {
    let float = matches!(column.dtype(), DataType::Float64 | DataType::Float32);
    let decimal = matches!(
        value,
        crate::Scalar::Decimal32(_)
            | crate::Scalar::Decimal64(_)
            | crate::Scalar::Decimal128(_)
            | crate::Scalar::Decimal256(_)
    );
    if !float || !decimal {
        return value;
    }
    crate::Decimal18::from_scalar(&value).map_or(value, |held| crate::Scalar::from(held.to_f64()))
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
        DataType::Sequence(SequenceType::List(item))
        | DataType::Sequence(SequenceType::LargeList(item))
        | DataType::Sequence(SequenceType::ListView(item))
        | DataType::Sequence(SequenceType::LargeListView(item))
        | DataType::Sequence(SequenceType::FixedSizeList(item, _)) => {
            value.as_sequence().map(|stated| {
                Some(crate::Scalar::from_sequence(
                    stated
                        .iter()
                        .filter_map(|held| refit(item, held.clone()))
                        .collect::<Vec<_>>(),
                ))
            })
        }
        DataType::Mapping(map) => value.as_mapping().map(|stated| {
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

/// Exact layout shared by FIX event, creation, grid and previous clocks.
pub(super) const CLOCK_DATATYPE: DataType = DataType::DateTime(DateTimeType::DateTime64 {
    unit: crate::TimeUnit::Nanosecond,
    timezone: crate::Timezone::UTC,
});

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/schema.rs`, `rust/tests/fix/codec.rs`,
    //! `rust/tests/fix/store.rs` and `rust/tests/fix/mod_.rs` pin and a caller
    //! cannot reach.
    //!
    //! The stable row is public; the clock layout every seeded FIX clock is
    //! typed by, the member order a group settles on, and the shape digest two
    //! readings are compared by are all steps inside it.
    use smol_str::SmolStr;

    use crate::{DataType, Field, Result, Scalar};

    /// The exact layout shared by the FIX event, creation, grid and previous
    /// clocks.
    #[must_use]
    pub const fn clock_datatype() -> DataType {
        super::CLOCK_DATATYPE
    }

    /// Settle the one member order every stated occurrence agrees with.
    ///
    /// # Errors
    ///
    /// Returns a typed failure naming the group where two occurrences state
    /// contradictory orders.
    pub fn ordered_group_union(
        union: Vec<Field>,
        stated: &[Vec<(String, Scalar)>],
        group: &str,
    ) -> Result<Vec<Field>> {
        let stated: Vec<Vec<(SmolStr, Scalar)>> = stated
            .iter()
            .map(|occurrence| {
                occurrence
                    .iter()
                    .map(|(name, value)| (SmolStr::new(name), value.clone()))
                    .collect()
            })
            .collect();
        super::ordered_group_union(union, &stated, group)
    }

    /// The digest of a row's shape, with or without the documents it carries.
    #[must_use]
    pub fn shape_digest(root: &Field, metadata: bool) -> u64 {
        super::shape_digest(root, metadata)
    }
}
