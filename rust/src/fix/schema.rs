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
//! `fixentries` closes every row with the residual record: a sorted
//! `map<utf8, utf8>` of every field the message states that no column
//! represents. A scalar a column successfully represents is stated once, in
//! that column; unprojected, conflicting and partly represented content stays
//! in the map.
//!
//! Each key is the field's tag and its dictionary name, `tag:name` - `58:text`,
//! `453:parties` - the two its [identity](super::FixId) is made of. A scalar's
//! value is its canonical wire text; a group or a component is the JSON of
//! what it holds, rendered by the crate's one JSON codec - a group the array
//! of its occurrences, a component or an occurrence the object of its
//! members, keyed `tag:name` the same way, leaves as text, all the way down.
//! A key stated more than once at one level is one key whose value is the
//! JSON array of its occurrences, in arrival order. A scalar whose own text
//! opens the way JSON does - `[`, `{` or `"` - is written as its JSON string,
//! so a value opening that way is JSON and every other value is text. The
//! typed entries are the message's in memory; the map is only how a row holds
//! them, and a message read back from a row parses it into those entries
//! through the dictionary again.
//!
//! A key the dictionary does not resolve is no field and has no `tag:name`:
//! it never enters `fixentries`. It lands in the row's `metadata` under the
//! key as the message spelled it, its value the raw text - a repeated key
//! the JSON array of its values in arrival order, a nested one its JSON -
//! beside the namespaced keys a bridge states. Reading a row back restores
//! it as the entry of tag zero a parse holds it as, so the message read back
//! re-emits it and digests as the parse did; the row's `metadata` is where
//! an unmapped key lives, the message's residual is where it is read.

use std::borrow::Cow;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Weak};

use crate::graph::Market;

use smol_str::SmolStr;

use crate::StructType;
use crate::{DataType, Field, Result};

use super::FixRegistry;
use smallvec::{SmallVec, smallvec};

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
/// are the identifiers, the instrument, the sides, the quotes, the prices, the
/// quantities, the currencies, the times and the statuses - what a book, a
/// blotter, a quote feed and a monitor all read.
///
/// Ordered as a message orders them - identity, then instrument, then the
/// order, then the quote's bid and offer, then the times, then the outcome -
/// rather than by tag number, so a row reads the way the message it came
/// from reads.
///
/// `Price(44)`, `OrderQty(38)` and `Quantity(53)` are columns of the ladder
/// like the rest, each exact and stated once. What a message is *about* is
/// what [`get_price`](crate::graph::Market::get_price) and
/// [`get_quantity`](crate::graph::Market::get_quantity) read off them, and no
/// column of this crate's restates either, because a row carrying both
/// would carry one fact twice.
pub const BODY_TAGS: [i32; 56] = [
    // Who the message is about: the order's own chain, its parents, and the
    // reports and quotes that answer it.
    1, 11, 41, 526, 37, 198, 17, 1003, 131, 117, 693,
    // The instrument, and what the market says about trading it.
    55, 48, 22, 167, 762, 207, 461, 541, 460, 326, 340, 965, // The order.
    54, 40, 59, 854, 15, 120, // The quote's bid and offer, which carry no side of their own.
    132, 133, 134, 135, 188, 189, 190,
    191, // What was done, and the FX parts of the last price.
    31, 32, 6, 14, 151, 194, 195, // When.
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

/// The column holding the residual record: a sorted `map<utf8, utf8>` from
/// each field's `tag:name` to what it stated, as
/// [`FixMsg::into_row`](crate::FixMsg::into_row) writes it.
///
/// The one column of the fixed row the registry does not define: it holds
/// whatever fields a message states, so no finite definition describes it,
/// and a map's length is its own count.
pub const FIXENTRIES_COLUMN: &str = "fixentries";

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
/// before the bid and offer that quote it.
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
        CONVERSATIONID_TAG_NAME as CONVERSATIONID, CREAUNIX_TAG_NAME as CREAUNIX,
        CROSSCODE_TAG_NAME as CROSSCODE, CROSSHASHCODE_TAG_NAME as CROSSHASHCODE,
        CROSSUUID_TAG_NAME as CROSSUUID, CURRHASHCODE_TAG_NAME as HASHCODE,
        CURRUNIX_TAG_NAME as UNIX, CURRUUID_TAG_NAME as CURRUUID, EXECUNIX_TAG_NAME as EXECUNIX,
        EXPRUNIX_TAG_NAME as EXPRUNIX, METADATA_TAG_NAME as METADATA,
        MSGCTXID_TAG_NAME as MSGCTXID, MSGDIRECTION_TAG_NAME as MSGDIRECTION,
        MSGORIGINATOR_TAG_NAME as MSGORIGINATOR, MSGPLUGINID_TAG_NAME as MSGPLUGINID,
        MSGSESSEVENTID_TAG_NAME as MSGSESSEVENTID, MSGSESSIONID_TAG_NAME as MSGSESSIONID,
        PREVUNIX_TAG_NAME as PREVUNIX, PREVUUID_TAG_NAME as PREVUUID,
        RECDUNIX_TAG_NAME as RECDUNIX, SEQNUM_TAG_NAME as SEQNUM, SNAPUNIX_TAG_NAME as SNAPUNIX,
        SRCUUIDS_TAG_NAME as SRCUUIDS, STATE_TAG_NAME as STATE,
    };
    let crated = super::fix_crate_fields().unwrap_or_default();
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
    // known, then the instants that instant is read against - created,
    // followed, snapped, expiring - and the clocks the protocol states.
    band(
        &mut tags,
        &[
            UNIX.0, EXECUNIX.0, RECDUNIX.0, CREAUNIX.0, PREVUNIX.0, SNAPUNIX.0, EXPRUNIX.0, 52,
            122, 60, 64, 75, 126, 62, 432,
        ],
    );
    // Which event: its own identity, the chain it stands in, what it
    // follows and what it was read from. A join reads these and
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
            SRCUUIDS.0,
        ],
    );
    // Which message, over which session: what the frame says it is, who sent
    // it to whom, which bridge handled it and which of its plugins it came
    // from, the session event it delivered the message as and the
    // conversation that exchange belongs to. Not where this capture read
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
            MSGORIGINATOR.0,
            MSGCTXID.0,
            MSGSESSIONID.0,
            MSGSESSEVENTID.0,
            CONVERSATIONID.0,
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
            super::FOREXCODE_TAG_NAME.0,
            super::BLOOMBERGCODE_TAG_NAME.0,
            super::FIGICODE_TAG_NAME.0,
            super::MICCODE_TAG_NAME.0,
            super::STRIKEPX_TAG_NAME.0,
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
    // the ones it was read off - what it moved from, its last executed price and quantity,
    // where it has got to - then what those are counted and denominated in,
    // how the order was written, and last the quote's bid and offer.
    //
    // Each number is FIX's own and appears once: `Price(44)`, `OrderQty(38)`
    // and `Quantity(53)` are columns like the rest of the ladder, and what a
    // message is *about* is what [`Market::get_price`] reads off them
    // rather than a column restating one of them.
    band(
        &mut tags,
        &[
            54, 44, 140, 31, 6, 38, 53, 32, 14, 151, 996, 15, 120, 854, 40, 59, 132, 134, 133, 135,
            194, 195, 188, 189, 190, 191,
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
            *tag != METADATA.0
                && !super::identity::is_capture_tag(*tag)
                && !super::crated::is_unprojected_tag(*tag)
        })
        .collect();
    band(&mut tags, &rest);
    // Last, what the message carried outside its fields: the bridge's own
    // keys and every key no dictionary resolved. The residual record closes
    // the row after it.
    band(&mut tags, &[METADATA.0]);
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
/// contains itself. Written by [`FixRegistry::commit`] as
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
/// # use yggdryl::local::LocalFolder;
/// # use yggdryl::{DataType, FixRegistry, fix_schema, fix_schema_carrying};
/// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
/// # let registry = FixRegistry::from_handle(&LocalFolder::new(root)?)?;
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
    /// Whether the column is the crate's own residual record, declared
    /// exactly as [`entries_field`] declares it: only then does a column
    /// representing an entry take it out of the record. A caller's own
    /// `fixentries` column of another shape keeps every entry.
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
/// per registry they were planned against, or per metadata a root of that
/// shape carries - past which the oldest is forgotten. These are plans,
/// not retained registries.
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
    // The shape names the bucket and never the metadata's storage: a door
    // builds its fixed root per reader, each column's metadata edited into
    // storage of its own, and a digest of those addresses filed every
    // reader's root as a new shape and retained it. What the bucket holds
    // is verified by content below, so a root of one shape under other
    // metadata is another entry of the bucket, never a plan reused wrongly.
    let shape = shape_digest(schema);
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

/// Whether `value`, held by the child `field` of the level `fields` and
/// `values` make, is a list holding nothing that no counter of that level
/// states a count for: no entry, the group absent.
///
/// A group holding no occurrence is stated by its count alone - the parse
/// holds `NoPartySubIDs(802)=0` as the counter stating zero beside the
/// empty list, which is the group's entry - while a table may store a null
/// list as an empty one: PyIceberg reads a null list of structs back as
/// `[]`, its counter still null.
pub(super) fn is_unstated_group(
    registry: &FixRegistry,
    field: &Field,
    value: &crate::Scalar,
    fields: &[Field],
    values: &[crate::Scalar],
) -> bool {
    matches!(field.dtype(), DataType::Serie(_) | DataType::LargeSerie(_))
        && value.as_serie().is_some_and(crate::Serie::is_empty)
        && !tag_and_counter(registry, field).1.is_some_and(|counter| {
            fields.iter().zip(values).any(|(held, stated)| {
                !stated.is_null()
                    && !held.dtype().is_nested()
                    && tag_and_counter(registry, held).0 == Some(counter)
            })
        })
}

/// A digest of one root's shape: its children's names, datatype
/// identifiers and nullability, nested children included, and never their
/// metadata, which is verified by content wherever the digest is believed.
///
/// Two roots of one shape digest alike, so a table keyed by the digest
/// finds what was read against a root of this shape; two of different
/// shapes may collide, so what the table holds says whether it is the
/// same shape - the schema equality a caller compares.
pub(super) fn shape_digest(root: &Field) -> u64 {
    // The dictionary's own fold rather than a keyed hash: a root is a
    // hundred children, each a few writes, read once per message, and
    // what the digest names is verified before it is believed.
    let mut state = super::registry::Mix::default();
    digest_shape(root.fields(), &mut state);
    state.finish()
}

fn digest_shape(fields: &[Field], state: &mut super::registry::Mix) {
    state.write_usize(fields.len());
    for field in fields {
        state.write(field.name().as_bytes());
        field.dtype().id().hash(state);
        state.write_u8(u8::from(field.is_nullable()));
        match field.dtype() {
            DataType::Struct(children) => digest_shape(children.as_fields(), state),
            DataType::Serie(item) | DataType::LargeSerie(item) => {
                digest_shape(std::slice::from_ref(&**item), state);
            }
            _ => {}
        }
    }
}

/// The residual record's column: a sorted map from each entry's `tag:name`
/// to the text it states.
fn entries_field() -> Result<Field> {
    let mut field = DataType::map_of(DataType::utf8(), DataType::utf8(), true)?
        .nullable_field(FIXENTRIES_COLUMN);
    field.set_display("FixEntries")?;
    field.set_description(
        "Content no other column represents, keyed by each field's tag:name: a scalar's wire \
         text, a group or a component as the JSON of what it holds, keyed the same way.",
    )?;
    Ok(field)
}

/// The key one residual entry is filed under: its tag and its name,
/// `55:symbol`, the two its identity is made of.
fn entry_key(entry: &super::FixEntry) -> SmolStr {
    smol_str::format_smolstr!("{}:{}", entry.tag(), entry.name())
}

/// Whether a stated text opens the way JSON does, and so is written as its
/// JSON string where a map value holds it.
fn opens_json(text: &str) -> bool {
    matches!(text.as_bytes().first(), Some(b'[' | b'{' | b'"'))
}

/// Whether the entries under `entry` are its occurrences - a group's, a
/// repeated field's - rather than a component's members.
///
/// Read the way [`child_from_entry`] reads them back: a count heads
/// occurrences; one key stated more than once is occurrences, since a
/// component names each member once; and a single entry under another is an
/// occurrence exactly where the dictionary holds neither a component nor a
/// map under the name above it.
fn states_occurrences(registry: &FixRegistry, entry: &super::FixEntry) -> bool {
    let nested = entry.entries();
    let Some(first) = nested.first() else {
        return false;
    };
    if entry.is_stated() {
        return true;
    }
    if nested.len() > 1 {
        return nested
            .iter()
            .all(|held| held.tag() == first.tag() && held.name() == first.name());
    }
    registry
        .get_field_by_name(entry.name())
        .or_else(|| (entry.tag() != 0).then(|| registry.get_field_by_tag(entry.tag()))?)
        .is_some_and(|known| {
            !matches!(
                known.dtype(),
                DataType::Struct(_) | DataType::Map(_) | DataType::SortedMap(_)
            )
        })
}

/// One entry as the JSON value it states: a scalar as its text, an entry
/// heading occurrences as the array of them, one heading members as the
/// object of them under `key`, and one heading nothing as `{}`.
fn entry_json(
    registry: &FixRegistry,
    entry: &super::FixEntry,
    key: fn(&super::FixEntry) -> SmolStr,
) -> Result<crate::Scalar> {
    let nested = entry.entries();
    if nested.is_empty() {
        return match entry.held_value() {
            Some(value) => Ok(crate::Scalar::from(value.clone())),
            None => crate::Scalar::from_struct(std::iter::empty::<(SmolStr, crate::Scalar)>()),
        };
    }
    if states_occurrences(registry, entry) {
        return crate::Scalar::try_sequence(nested.len(), |index| {
            occurrence_json(registry, &nested[index], key)
        });
    }
    members_json(registry, nested, key)
}

/// One occurrence as the JSON value it states: a scalar as its text, and one
/// heading entries as the object of its members - or, where one key is
/// stated more than once under it, the array of those. An occurrence is
/// never asked of the dictionary: its name is its group's, not a field's.
fn occurrence_json(
    registry: &FixRegistry,
    entry: &super::FixEntry,
    key: fn(&super::FixEntry) -> SmolStr,
) -> Result<crate::Scalar> {
    let nested = entry.entries();
    let repeated = nested.len() > 1
        && nested
            .iter()
            .all(|held| held.tag() == nested[0].tag() && held.name() == nested[0].name());
    if nested.is_empty() {
        entry_json(registry, entry, key)
    } else if repeated {
        crate::Scalar::try_sequence(nested.len(), |index| {
            occurrence_json(registry, &nested[index], key)
        })
    } else {
        members_json(registry, nested, key)
    }
}

/// One level of members as the JSON object of them: each under `key`, a key
/// stated more than once the array of its values in arrival order.
fn members_json(
    registry: &FixRegistry,
    entries: &[super::FixEntry],
    key: fn(&super::FixEntry) -> SmolStr,
) -> Result<crate::Scalar> {
    let mut keyed: std::collections::BTreeMap<SmolStr, Vec<&super::FixEntry>> =
        std::collections::BTreeMap::new();
    for entry in entries {
        keyed.entry(key(entry)).or_default().push(entry);
    }
    let mut members = Vec::with_capacity(keyed.len());
    for (name, held) in keyed {
        let value = match held.as_slice() {
            [entry] => entry_json(registry, entry, key)?,
            many => crate::Scalar::try_sequence(many.len(), |index| {
                entry_json(registry, many[index], key)
            })?,
        };
        members.push((name, value));
    }
    crate::Scalar::from_struct(members)
}

/// The residual record as the column holds it: one key per `tag:name`, a
/// stated scalar's text as it is - its JSON string where the text opens the
/// way JSON does - and anything else as its JSON; a key stated more than once
/// the JSON array of its values, in arrival order.
fn entries_map<'entry>(
    registry: &FixRegistry,
    entries: impl Iterator<Item = &'entry super::FixEntry>,
) -> Result<crate::Scalar> {
    let mut keyed: std::collections::BTreeMap<SmolStr, Vec<&super::FixEntry>> =
        std::collections::BTreeMap::new();
    for entry in entries {
        keyed.entry(entry_key(entry)).or_default().push(entry);
    }
    let mut pairs = Vec::with_capacity(keyed.len());
    for (key, held) in keyed {
        let text = keyed_text(registry, &held, entry_key)?;
        pairs.push((crate::Scalar::from(key), crate::Scalar::from(text)));
    }
    crate::Scalar::from_mapping(pairs)
}

/// The text the entries filed under one key state as a map value: a stated
/// scalar's text as it is - its JSON string where the text opens the way
/// JSON does - anything else as its JSON, members keyed by `key`; a key
/// stated more than once the JSON array of its values, in arrival order.
fn keyed_text(
    registry: &FixRegistry,
    held: &[&super::FixEntry],
    key: fn(&super::FixEntry) -> SmolStr,
) -> Result<SmolStr> {
    let json = match held {
        [entry]
            if entry.entries().is_empty()
                && entry.value().is_some_and(|text| !opens_json(text)) =>
        {
            return Ok(entry.held_value().cloned().unwrap_or_default());
        }
        [entry] => entry_json(registry, entry, key)?,
        many => {
            crate::Scalar::try_sequence(many.len(), |index| entry_json(registry, many[index], key))?
        }
    };
    Ok(SmolStr::from(crate::into_json_scalar(&json)?))
}

/// A key no dictionary resolved, read back: a member is filed under its own
/// name, and a name is all it is.
fn unresolved_key<'key>(key: &'key str, _: &crate::path::Path<'_>) -> Result<(i32, &'key str)> {
    Ok((0, key))
}

/// The bridge statements of a row's `metadata` cell, the keys no dictionary
/// resolved moved onto `unresolved` as the tag-zero entries a parse holds
/// them as: a key under a namespace (`TECH.CLIENTID`) is a bridge's and
/// stays, every other is one [`FixMsg::row_metadata`] wrote out of the
/// residual - its text as stated, or, where it opens the way JSON does, the
/// entries its JSON holds. The cell answers null once nothing stays.
///
/// # Errors
///
/// Returns [`crate::Error::InvalidRecord`] at `$.metadata.<key>` for JSON
/// that does not decode.
fn split_unresolved(
    value: &crate::Scalar,
    unresolved: &mut Vec<super::FixEntry>,
) -> Result<crate::Scalar> {
    let root = crate::path::Path::root();
    let path = root.field("metadata");
    let Some(entries) = value.as_mapping() else {
        return Ok(value.clone());
    };
    if entries
        .iter()
        .all(|(key, _)| key.as_str().is_none_or(|key| key.contains('.')))
    {
        return Ok(value.clone());
    }
    let mut kept = Vec::with_capacity(entries.len());
    for (key, held) in entries {
        match key.as_str() {
            Some(name) if !name.contains('.') => {
                let here = path.field(name);
                let Some(text) = held.as_str() else {
                    unresolved.push(super::FixEntry::new(0, name, None));
                    continue;
                };
                unresolved.push(if opens_json(text) {
                    let decoded = crate::from_json_scalar(text.as_bytes()).map_err(|error| {
                        entry_error(&here, "the JSON of a key no dictionary resolved", error)
                    })?;
                    entry_from_json(0, name, &decoded, &here, unresolved_key)?
                } else {
                    super::FixEntry::new(0, name, Some(SmolStr::new(text)))
                });
            }
            _ => kept.push((key.clone(), held.clone())),
        }
    }
    if kept.is_empty() {
        return Ok(crate::Scalar::Null);
    }
    crate::Scalar::from_mapping(kept)
}

/// The members one repeating-group field declares, or None for anything else.
///
/// The row projection, restatement and builder share the catalog's reading
/// of a group's occurrence, including a Map's entries Struct.
pub(super) fn item_fields(field: &Field) -> Option<&[Field]> {
    let item = super::catalog::occurrence_of(field)?;
    item.dtype().as_fields()
}

/// One `tag:name` key read back: the tag, zero only for a member no
/// dictionary named inside a field one did, and the name after it.
fn parse_key<'key>(key: &'key str, path: &crate::path::Path<'_>) -> Result<(i32, &'key str)> {
    key.split_once(':')
        .and_then(|(tag, name)| {
            let tag = if tag == "0" {
                0
            } else {
                super::field::parse_tag(tag)?
            };
            (!name.is_empty()).then_some((tag, name))
        })
        .ok_or_else(|| entry_error(path, "a tag:name key", key))
}

/// The text one JSON leaf states: a string as it is, a number or a boolean as
/// its JSON spelling.
fn leaf_text(value: &crate::Scalar, path: &crate::path::Path<'_>) -> Result<SmolStr> {
    if let Some(text) = value.as_str() {
        return Ok(SmolStr::new(text));
    }
    if value.as_mapping().is_some() {
        return Err(entry_error(path, "text", value.kind()));
    }
    crate::into_json_scalar(value).map(SmolStr::from)
}

/// How a JSON member key reads back as a tag and a name: `tag:name` in the
/// `fixentries` column, the name alone in the metadata.
type KeyReader = for<'key> fn(&'key str, &crate::path::Path<'_>) -> Result<(i32, &'key str)>;

/// One entry read back out of the JSON it was written as: the inverse of
/// [`entry_json`] under `tag:name` keys. An object is members, each key its
/// member's `tag:name`; an array is occurrences, an object among them an
/// occurrence heading its members and anything else one stated under the
/// entry's own key; text is a stated scalar.
fn entry_from_json(
    tag: i32,
    name: &str,
    value: &crate::Scalar,
    path: &crate::path::Path<'_>,
    key: KeyReader,
) -> Result<super::FixEntry> {
    if let Some(members) = value.as_struct() {
        let mut nested = Vec::with_capacity(members.len());
        for (spelled, held) in members {
            let here = path.field(spelled);
            let (tag, name) = key(spelled, &here)?;
            nested.push(entry_from_json(tag, name, held, &here, key)?);
        }
        return Ok(super::FixEntry::new(tag, name, None).with_entries(nested));
    }
    if let Some(occurrences) = value.as_sequence() {
        let mut nested = Vec::with_capacity(occurrences.len());
        for (index, held) in occurrences.iter().enumerate() {
            if held.is_null() {
                continue;
            }
            let here = path.child(crate::path::Segment::Index(index));
            let own = if held.as_struct().is_some() { 0 } else { tag };
            nested.push(entry_from_json(own, name, held, &here, key)?);
        }
        return Ok(super::FixEntry::new(tag, name, None).with_entries(nested));
    }
    if value.is_null() {
        return Ok(super::FixEntry::new(tag, name, None));
    }
    Ok(super::FixEntry::new(
        tag,
        name,
        Some(leaf_text(value, path)?),
    ))
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

/// The residual entries one `fixentries` cell holds, in key order: the
/// inverse of [`entries_map`]. A value opening the way JSON does is read as
/// JSON, every other one as the text it states. A key is refused unless it is
/// a resolved field's `tag:name`, since a key no dictionary resolved lives in
/// `metadata`, and so is JSON that does not decode: a message rebuilt with a
/// pair missing is a different message. The text is copied here, because a
/// row is where a message stops being a range of a line.
///
/// # Errors
///
/// Returns [`crate::Error::InvalidRecord`] at the entry's path for a cell
/// that is not a map of text, a key that is not `tag:name` with a positive
/// tag, or a value that does not decode.
fn entries_from_map(
    value: &crate::Scalar,
    path: &crate::path::Path<'_>,
) -> Result<Vec<super::FixEntry>> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let pairs = value
        .as_mapping()
        .ok_or_else(|| entry_error(path, "a map of residual entries", value.kind()))?;
    let mut entries = Vec::with_capacity(pairs.len());
    for (key, text) in pairs {
        let key = key
            .as_str()
            .ok_or_else(|| entry_error(path, "a text key", key.kind()))?;
        let here = path.field(key);
        let (tag, name) = parse_key(key, &here)?;
        if tag == 0 {
            return Err(entry_error(&here, "a resolved field's tag:name", key));
        }
        if text.is_null() {
            continue;
        }
        let text = text
            .as_str()
            .ok_or_else(|| entry_error(&here, "text", text.kind()))?;
        entries.push(if opens_json(text) {
            let decoded = crate::from_json_scalar(text.as_bytes())
                .map_err(|error| entry_error(&here, "the JSON of a residual entry", error))?;
            entry_from_json(tag, name, &decoded, &here, parse_key)?
        } else {
            super::FixEntry::new(tag, name, Some(SmolStr::new(text)))
        });
    }
    Ok(entries)
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
                let count = value.as_serie().map_or(0, crate::Serie::len);
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

/// [`tag_and_counter`] of each of `fields`, in their order.
fn member_facts(registry: &FixRegistry, fields: &[Field]) -> Vec<(Option<i32>, Option<i32>)> {
    fields
        .iter()
        .map(|field| tag_and_counter(registry, field))
        .collect()
}

/// The one declared member an entry owns. A group column owns its counter's
/// entry before the scalar counter beside it; that scalar is the same fact,
/// checked separately against the occurrence count.
///
/// `facts` is [`tag_and_counter`] of each of `fields`, resolved once by the
/// caller for every entry it asks about.
fn covered_member_index(
    fields: &[Field],
    facts: &[(Option<i32>, Option<i32>)],
    entry: &super::FixEntry,
) -> Option<usize> {
    let unique = |counter: bool| {
        let mut found =
            fields
                .iter()
                .zip(facts)
                .enumerate()
                .filter(|(_, (field, (tag, held_counter)))| {
                    if entry.tag() == 0 {
                        !counter && crate::folds_equal(field.name(), entry.name())
                    } else if counter {
                        *held_counter == Some(entry.tag())
                    } else {
                        held_counter.is_none() && *tag == Some(entry.tag())
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
                && covers_members(
                    registry,
                    field.fields(),
                    &member_facts(registry, field.fields()),
                    value,
                    entry.entries(),
                )
        }
        DataType::Serie(item) | DataType::LargeSerie(item) => {
            let Some(occurrences) = value.as_serie() else {
                return false;
            };
            if entry.value().and_then(|count| count.parse::<usize>().ok())
                != Some(occurrences.len())
                || entry.entries().len() != occurrences.len()
            {
                return false;
            }
            // Every occurrence is laid out on the one item, so its members
            // are resolved once for the whole group.
            let facts = match item.dtype() {
                DataType::Struct(_) => member_facts(registry, item.fields()),
                _ => Vec::new(),
            };
            entry
                .entries()
                .iter()
                .zip(occurrences.iter())
                .all(|(occurrence, value)| match item.dtype() {
                    DataType::Struct(_) => {
                        occurrence.value().is_none()
                            && covers_identity(registry, item, occurrence)
                            && covers_members(
                                registry,
                                item.fields(),
                                &facts,
                                &value,
                                occurrence.entries(),
                            )
                    }
                    _ => covers_entry(registry, item, &value, occurrence),
                })
        }
        DataType::Map(_)
        | DataType::SortedMap(_)
        | DataType::SerieView(_)
        | DataType::FixedSizeSerie(..)
        | DataType::LargeSerieView(_) => false,
        _ => {
            entry.entries().is_empty()
                && super::entry::wire_text_under(registry, field, value).as_deref() == entry.value()
        }
    }
}

/// Whether every stated member is represented once in a fitted Struct value.
///
/// `facts` is [`member_facts`] of `fields`, and each entry's owner is
/// resolved once: every check below reads them by position rather than
/// scanning the members again per entry.
fn covers_members(
    registry: &FixRegistry,
    fields: &[Field],
    facts: &[(Option<i32>, Option<i32>)],
    value: &crate::Scalar,
    entries: &[super::FixEntry],
) -> bool {
    let Some(values) = value.as_sequence() else {
        return false;
    };
    if fields.len() != values.len() {
        return false;
    }
    // One bit per member, on the stack for a level of up to 128 of them:
    // this runs once per occurrence of every group a row states.
    let mut owned: SmallVec<[u64; 2]> = smallvec![0; fields.len().div_ceil(64)];
    let bit = |index: usize| (index / 64, 1_u64 << (index % 64));
    for entry in entries {
        let Some(index) = covered_member_index(fields, facts, entry) else {
            return false;
        };
        let (word, mask) = bit(index);
        if owned[word] & mask != 0 {
            return false;
        }
        owned[word] |= mask;
        // An empty list beside no count stated for it represents no entry.
        if is_unstated_group(registry, &fields[index], &values[index], fields, values)
            || !covers_entry(registry, &fields[index], &values[index], entry)
        {
            return false;
        }
    }
    values
        .iter()
        .zip(facts)
        .enumerate()
        .filter(|(_, (value, _))| !value.is_null())
        .all(|(index, (value, (tag, counter)))| {
            let (word, mask) = bit(index);
            if owned[word] & mask != 0 {
                return true;
            }
            // A group's scalar counter is represented by the same entry
            // as its Serie. It is covered only when both fitted values say
            // the same occurrence count.
            let Some(tag) = tag.filter(|_| counter.is_none()) else {
                return false;
            };
            let mut groups = facts
                .iter()
                .enumerate()
                .filter(|(_, (_, held))| *held == Some(tag));
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
                && covered_member_index(fields, facts, owner) == Some(group_index)
                && values[group_index].as_serie().is_some_and(|occurrences| {
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

/// One group entry as the serie it states: an occurrence per entry under it,
/// each holding the members that occurrence stated, laid out on the union of
/// every occurrence's members in their consistent relative order.
///
/// `item` is the occurrence the dictionary declares, where it declares one:
/// a member it names types through its own field, and the serie keeps the
/// group's storage. A group no dictionary declares - what a bridge packs
/// under a counter's own name - takes its occurrence name from the entries
/// and lands as a plain `Serie`, which is what the builder made of it.
fn group_from_entry(
    registry: &FixRegistry,
    entry: &super::FixEntry,
    known: &Field,
    item: Option<&Field>,
) -> Result<(Field, crate::Scalar)> {
    let declared = item
        .and_then(|item| item.dtype().as_fields())
        .unwrap_or_default();
    let facts = member_facts(registry, declared);
    let mut union: Vec<Field> = Vec::new();
    let mut stated: Vec<Vec<(SmolStr, crate::Scalar)>> = Vec::with_capacity(entry.entries().len());
    for occurrence in entry.entries() {
        let mut fields = Vec::with_capacity(occurrence.entries().len());
        let mut values = Vec::with_capacity(occurrence.entries().len());
        for member in occurrence.entries() {
            let slot = covered_member_index(declared, &facts, member)
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
    let mut union = ordered_group_union(union, &stated, entry.name())?;
    // A declared group's members stand in the order its dictionary declares
    // them, whatever order the record listed them in: the residual map is
    // keyed in sort order, and a wire re-emitted from the group opens each
    // occurrence on its delimiter. A member the dictionary does not declare
    // follows, in the order the occurrences settled.
    if !declared.is_empty() {
        union.sort_by_key(|field| {
            let (tag, _) = tag_and_counter(registry, field);
            declared
                .iter()
                .zip(&facts)
                .position(|(held, (held_tag, _))| {
                    crate::folds_equal(held.name(), field.name())
                        || (tag.is_some() && *held_tag == tag)
                })
                .unwrap_or(usize::MAX)
        });
    }
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
        DataType::LargeSerie(_) => DataType::large_serie(occurrence),
        map_dtype @ (DataType::Map(_) | DataType::SortedMap(_)) => {
            let map = &map_dtype
                .as_mapping()
                .expect("the variant was just matched");
            DataType::map(occurrence, map.keys_sorted())?
        }
        _ => DataType::serie(occurrence),
    };
    // A group the dictionary does not declare stands under the counter's own
    // name and states no counter of its own: the builder left the count to
    // the occurrences beside it, and a counter here would put a second child
    // of that name in the row.
    let field = Field::new_with_metadata(known.name(), dtype, true, known.as_metadata().clone());
    Ok((field, crate::Scalar::from_sequence(rows)))
}

/// A scalar stated at indexed positions as the Serie the builder made of it.
/// If every spelling types, the dictionary's scalar remains the item; if one
/// does not, every raw spelling stays under UTF-8 so no sibling changes type
/// or disappears on a later write.
fn scalar_serie_from_entry(
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
        DataType::serie(item),
        true,
        known.as_metadata().clone(),
    );
    Ok((field, crate::Scalar::from_sequence(values)))
}

/// An unexplained nested entry as the shape its children state. Repeated
/// tagless occurrence names identify a Serie; otherwise the children are the
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
            return scalar_serie_from_entry(registry, entry, &known);
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

/// The scalar field `name` reaches only as one of its `FIX:names`: never
/// its canonical name, which a residual entry spells only where it is that
/// field. An untagged residual entry spelled so is an alias that did not
/// fill the field - its winner took the tag.
fn alias_lost_to<'registry>(
    registry: &'registry FixRegistry,
    name: &str,
) -> Option<&'registry Field> {
    let reached = registry.get_field_by_name(name)?;
    (!reached.dtype().is_nested()
        && !crate::folds_equal(reached.name(), name)
        && reached
            .as_fix()
            .names()
            .any(|alias| crate::folds_equal(alias, name)))
    .then_some(reached)
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
    if declared.is_none() && entry.tag() == 0 {
        if let Some(lost_to) = alias_lost_to(registry, entry.name()) {
            // The builder's demoted alias, rebuilt as it built one: a row
            // carries no metadata, and without the mark the name would
            // reach the field it did not fill and fold into it again.
            let mut field = DataType::utf8().nullable_field(entry.held_name().clone());
            // A plain key and value: the one refusal a metadata insert has
            // is a shape no spelling here takes.
            let _ = field.insert_metadata(super::field::ALIAS_OF, lost_to.name());
            let value = entry
                .value()
                .map_or(crate::Scalar::Null, crate::Scalar::from);
            return Ok((field, value));
        }
    }
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
    // shape. It rebuilds as the serie it is rather than as the scalar the
    // name reaches, which would answer null and lose the occurrences.
    if !known.dtype().is_nested() && !entry.entries().is_empty() {
        if entry
            .entries()
            .iter()
            .all(|occurrence| occurrence.entries().is_empty())
        {
            return scalar_serie_from_entry(registry, entry, known);
        }
        return group_from_entry(registry, entry, known, None);
    }
    match known.dtype() {
        DataType::Serie(_)
        | DataType::LargeSerie(_)
        | DataType::Map(_)
        | DataType::SortedMap(_) => {
            let Some(item) = super::catalog::occurrence_of(known) else {
                return Ok((known.clone(), crate::Scalar::Null));
            };
            group_from_entry(registry, entry, known, Some(item))
        }
        DataType::Struct(_) => {
            let mut fields: Vec<Field> = Vec::with_capacity(entry.entries().len());
            let mut values: Vec<crate::Scalar> = Vec::with_capacity(entry.entries().len());
            let facts = member_facts(registry, known.fields());
            for member in entry.entries() {
                let slot = covered_member_index(known.fields(), &facts, member)
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
    /// # use yggdryl::local::LocalFolder;
    /// # use yggdryl::graph::Element;
    /// # use yggdryl::{FixCodec, FixMsg, FixRegistry, fix_schema};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
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
    /// [`crate::Error::InvalidRecord`] at the entry's path when the entries
    /// column holds a key that is not a resolved field's `tag:name` or a value
    /// whose JSON does not decode.
    pub fn from_row(
        registry: Arc<FixRegistry>,
        schema: &Field,
        row: &crate::Scalar,
    ) -> Result<Self> {
        let value = schema.canonicalize_value(row.clone())?;
        Self::rebuilt(registry, schema, &value)
    }

    /// [`Self::from_row`] for a row of a record column landed under
    /// `schema`: the landing proved every row against the schema - a null
    /// where the schema requires a value, a text past the bound a column
    /// states, a code no registry holds - and the column answers it in the
    /// schema's own canonical form, so it is rebuilt as it stands, neither
    /// validated nor canonicalized a second time.
    pub(super) fn from_landed_row(
        registry: Arc<FixRegistry>,
        schema: &Field,
        row: &crate::Scalar,
    ) -> Result<Self> {
        Self::rebuilt(registry, schema, row)
    }

    /// The message a canonical row of `schema` holds.
    fn rebuilt(registry: Arc<FixRegistry>, schema: &Field, value: &crate::Scalar) -> Result<Self> {
        let plan = column_plan_of(schema, &registry)?;
        let mut members: Vec<Field> = Vec::with_capacity(schema.fields().len());
        let mut values: Vec<crate::Scalar> = Vec::with_capacity(schema.fields().len());
        let mut residual: Option<Vec<super::FixEntry>> = None;
        let mut unresolved: Vec<super::FixEntry> = Vec::new();
        let mut projected: Vec<(i32, Field, crate::Scalar)> = Vec::new();
        let mut carried: Vec<(smol_str::SmolStr, crate::Scalar)> = Vec::new();
        let held = value.as_sequence().unwrap_or_default();
        for ((column, planned), value) in schema.fields().iter().zip(plan.iter()).zip(held) {
            if column.name() == FIXENTRIES_COLUMN {
                let root = crate::path::Path::root();
                residual = Some(entries_from_map(value, &root.field(FIXENTRIES_COLUMN))?);
                continue;
            }
            match planned.tag {
                // The row's metadata holds two kinds of key: a bridge's own
                // namespaced statement, which the message keeps as its
                // metadata, and a key no dictionary resolved, which a parse
                // holds as a residual entry of tag zero and so is restored
                // as one - the wire, the digest and a read by name then
                // answer what the line's did.
                Some(tag) if tag == super::METADATA_TAG_NAME.0 => {
                    members.push(column.clone());
                    values.push(split_unresolved(value, &mut unresolved)?);
                }
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
                // A list holding nothing beside no count stated for it is
                // the group absent, at the root as inside an occurrence: a
                // table may read a null list of structs back as `[]`, and
                // the group pushed from it would state its counter as zero.
                Some(tag) => {
                    if !value.is_null()
                        && !is_unstated_group(&registry, column, value, schema.fields(), held)
                    {
                        projected.push((
                            planned.counter.unwrap_or(tag),
                            column.clone(),
                            value.clone(),
                        ));
                    }
                }
                None if planned.counter.is_some() => {
                    if !value.is_null()
                        && !is_unstated_group(&registry, column, value, schema.fields(), held)
                    {
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
        let mut residual = residual.unwrap_or_default();
        residual.extend(unresolved);
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
            // A tag several residual entries state answers its column with
            // the first of them, and the map lists them by name, not in the
            // order they arrived: the one the column states stands first
            // among them again, so the row it was read from is the row it
            // writes.
            for (tag, field, value) in &projected {
                if field.dtype().is_nested()
                    || residual.iter().filter(|entry| entry.tag() == *tag).count() < 2
                {
                    continue;
                }
                let owners: SmallVec<[usize; 4]> = members
                    .iter()
                    .enumerate()
                    .filter(|(_, member)| {
                        !member.dtype().is_nested()
                            && tag_and_counter(&registry, member).0 == Some(*tag)
                    })
                    .map(|(at, _)| at)
                    .collect();
                let stated = super::entry::wire_text_under(&registry, field, value);
                if let Some(at) = owners.iter().copied().find(|at| {
                    super::entry::wire_text_under(&registry, &members[*at], &values[*at]) == stated
                }) {
                    if let Some(first) = owners.first().copied() {
                        members.swap(first, at);
                        values.swap(first, at);
                    }
                }
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
    /// column is omitted from it. Unprojected, conflicting, partially
    /// represented and unreadable content remains there whole, keyed by its
    /// `tag:name`. A key no dictionary resolved is no field: it is stated in
    /// the `metadata` column under its own spelling, its text intact.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::local::LocalFolder;
    /// # use yggdryl::{FixCodec, FixRegistry, fix_schema};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    /// let schema = fix_schema(&registry, "fix")?;
    /// let reader = FixCodec::new(Arc::clone(&registry));
    /// let order = reader.parse_fix_line(b"8=FIX.4.4|35=D|55=AAPL|54=1|9999=x|10=0|")?;
    ///
    /// let row = order.into_row(&schema)?;
    /// let held = row.as_sequence().expect("a row");
    /// // The columns are the folded names, so `msgtype` is where the type is.
    /// let at = schema.index_of("msgtype").expect("the msgtype column");
    /// assert_eq!(held[at].as_str(), Some("D"));
    /// // A key no dictionary resolved is stated in the metadata under its own
    /// // spelling, with its value as it arrived, and never in the record.
    /// let at = schema.index_of("metadata").expect("the metadata column");
    /// let metadata = held[at].as_mapping().expect("a map");
    /// assert!(metadata.iter().any(|(key, value)| key.as_str() == Some("9999") && value.as_str() == Some("x")));
    /// let at = schema.index_of("fixentries").expect("the residual record");
    /// let record = held[at].as_mapping().expect("a map");
    /// assert!(record.iter().all(|(key, _)| !key.as_str().unwrap().starts_with("0:")));
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
    /// Refuses a missing or mistyped mandatory replay holder, or a value a
    /// column that cannot be null will not hold.
    pub fn into_row(&self, schema: &Field) -> Result<crate::Scalar> {
        let plan = column_plan_of(schema, self.registry())?;
        let columns = schema.fields();
        let has_residual_columns = columns
            .iter()
            .any(|column| column.name() == FIXENTRIES_COLUMN);
        let fitted_cell = |index: usize| -> Result<crate::Scalar> {
            let column = &columns[index];
            let planned = &plan[index];
            if column.name() == FIXENTRIES_COLUMN {
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
                (Some(tag), _) if tag == super::METADATA_TAG_NAME.0 => self.row_metadata()?,
                (Some(tag), _) if super::identity::is_typed_tag(tag) => self.column_value(tag),
                // The one the crate tags - `sourceurl` - is carried.
                (Some(tag), _) if super::identity::is_capture_tag(tag) => {
                    self.carried_cell(column.name())
                }
                (_, Some(counter)) => {
                    let value = self
                        .index_of_group(counter)
                        .and_then(|index| self.as_value().get(index))
                        .map(Cow::into_owned)
                        .unwrap_or(crate::Scalar::Null);
                    self.regrouped(counter, column, value)
                }
                (Some(tag), None) => self.regrouped(tag, column, self.column_value(tag)),
                (None, None) => match self.carried_cell(column.name()) {
                    crate::Scalar::Null => self
                        .index_of_name(column.name())
                        .and_then(|at| self.as_value().get(at))
                        .map(Cow::into_owned)
                        .unwrap_or(crate::Scalar::Null),
                    carried => carried,
                },
            };
            fitted(column, value)
        };

        if !has_residual_columns {
            return crate::Scalar::try_sequence(columns.len(), fitted_cell);
        }
        // The row is written where it is stored: every fitted cell first,
        // then the counters and the record decided over them.
        crate::Scalar::try_build_sequence(columns.len(), |values| {
            for (index, slot) in values.iter_mut().enumerate() {
                *slot = fitted_cell(index)?;
            }
            // A group occurrence none of the fixed Serie's members can represent
            // belongs wholly to the residual record. Its scalar counter must stay
            // there with it: projecting the count beside a null Serie would claim
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
                        // A group holding no occurrence is stated by its
                        // count alone: an empty list with no counter column
                        // stating it represents no entry, so a stated zero
                        // stays in the record.
                        if !source_field.dtype().is_nested()
                            || is_unstated_group(
                                self.registry(),
                                column,
                                &values[index],
                                columns,
                                values,
                            )
                            || !covers_entry(self.registry(), column, &values[index], entry)
                        {
                            continue;
                        }
                    } else if source_field.dtype().is_nested()
                        || !crate::folds_equal(source_field.name(), entry.name())
                        || !entry.entries().is_empty()
                        || *source_value != values[index]
                        || !covers_entry(self.registry(), column, &values[index], entry)
                    {
                        continue;
                    }
                    represented.push(entry_index);
                }
                represented.sort_unstable();
                represented.dedup();
            }
            // The residual record is every entry no column represents, but
            // a key no dictionary resolved: that is no field, and the
            // metadata states it.
            let record = columns
                .iter()
                .any(|column| column.name() == FIXENTRIES_COLUMN)
                .then(|| {
                    entries_map(
                        self.registry(),
                        entries
                            .iter()
                            .enumerate()
                            .filter(|(index, entry)| {
                                entry.tag() != 0 && represented.binary_search(index).is_err()
                            })
                            .map(|(_, entry)| entry),
                    )
                })
                .transpose()?;
            for (index, column) in columns.iter().enumerate() {
                if column.name() == FIXENTRIES_COLUMN {
                    values[index] = fitted(column, record.clone().unwrap_or(crate::Scalar::Null))?;
                }
            }
            Ok(())
        })
    }

    /// The `metadata` cell a row states: the message's own map, then every
    /// key no dictionary resolved - a root entry of tag zero - under its own
    /// spelling, as [`keyed_text`] renders it with members under their own
    /// names, a key stated more than once the JSON array of its values in
    /// arrival order. Where one spelling names both, the message's own value
    /// stands.
    fn row_metadata(&self) -> Result<crate::Scalar> {
        // By name, arrival order kept among one name's occurrences: on the
        // stack for every message a desk writes.
        let mut unresolved: SmallVec<[&super::FixEntry; 64]> = self
            .entries()
            .iter()
            .filter(|entry| entry.tag() == 0)
            .collect();
        // Every entry is borrowed from the one slice, so its address is its
        // arrival: an unstable sort on the two is the stable one, with no
        // buffer of its own.
        unresolved.sort_unstable_by(|left, right| {
            left.held_name()
                .cmp(right.held_name())
                .then_with(|| std::ptr::from_ref(*left).cmp(&std::ptr::from_ref(*right)))
        });
        let mut held: Option<std::collections::BTreeMap<SmolStr, SmolStr>> = None;
        for entries in unresolved.chunk_by(|left, right| left.held_name() == right.held_name()) {
            let text = keyed_text(self.registry(), entries, |held| held.held_name().clone())?;
            held.get_or_insert_with(|| self.metadata().clone())
                .entry(entries[0].held_name().clone())
                .or_insert(text);
        }
        match held {
            None => Ok(self.column_value(super::METADATA_TAG_NAME.0)),
            Some(metadata) => crate::Scalar::from_mapping(
                metadata
                    .into_iter()
                    .map(|(key, value)| (crate::Scalar::from(key), crate::Scalar::from(value))),
            ),
        }
    }

    /// One group's value, laid out the way the fixed column declares it.
    ///
    /// A message's own group holds the members that occurrence stated, in the
    /// order it stated them; the fixed column declares the dictionary's
    /// members. Placing them by name is what lets an occurrence a bridge
    /// packed into one member land in the same column as one that spelled
    /// every member out. An unstated member becomes null; Field::scalar then
    /// enforces the declared occurrence's types and nullability.
    pub(super) fn regrouped(
        &self,
        tag: i32,
        declared: &Field,
        held: crate::Scalar,
    ) -> crate::Scalar {
        let Some(members) = item_fields(declared) else {
            return held;
        };
        let Some(occurrences) = held.as_serie() else {
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
                return occurrence.into_owned();
            };
            crate::Scalar::from_sequence(placed.iter().map(|(at, group_at)| {
                let explicit = at.and_then(|at| stated.get(at));
                if let Some(value) = explicit.filter(|value| !value.is_null()) {
                    return value.clone();
                }
                group_at
                    .and_then(|at| stated.get(at))
                    .and_then(crate::Scalar::as_serie)
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
    /// because a stated value is never overridden. Then the count of a group
    /// whose counter the message did not state. Then the facts this crate
    /// computes for a message built from a schema and a value - `BeginString`
    /// and the classification `CFICode(461)` - which the
    /// [enriching pass](super::enrich) would otherwise have stated.
    ///
    /// Enrichment fills and never overwrites, so a column a venue did state
    /// is that venue's answer whatever the derivation would have said.
    fn column_value(&self, tag: i32) -> crate::Scalar {
        // A sending clock the message never stated is intake's stand-in for
        // one rather than a fact of the message: it is what the instant and
        // the creation were settled from, and each of those has a column of
        // its own. Stating it here would make the row read back as a message
        // that stated a clock, and that message re-emits a `52=` its line
        // never carried.
        if tag == 52 && !self.header().stated_sendingtime() {
            return crate::Scalar::Null;
        }
        // A stated value wins.
        if let Some(held) = self.get_by_tag(tag).filter(|held| !held.is_null()) {
            return held;
        }
        // A group is the statement its counter counts. Where no scalar
        // counter was stated, derive the column from the occurrences; an
        // explicit non-null counter returned above remains authoritative even
        // when it conflicts.
        if let Some(count) = self
            .index_of_group(tag)
            .and_then(|index| self.as_value().get(index))
            .and_then(|held| held.as_serie().map(crate::Serie::len))
        {
            return crate::Scalar::from(i32::try_from(count).unwrap_or(i32::MAX));
        }
        // The version a message that states none is said to be read at,
        // derived here for a message built from a schema and a value exactly
        // as the builder stamps one it parsed.
        if tag == 8 {
            let version = super::build::default_version();
            return crate::Scalar::from(format!("FIX.{version}"));
        }
        if tag == super::cfi::CFICODE_TAG {
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
                .map(crate::Scalar::Cfi)
                .or_else(|| self.classification().map(crate::Scalar::from))
                .unwrap_or(crate::Scalar::Null)
        } else {
            crate::Scalar::Null
        }
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
/// the serie rather than taking the serie with it.
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
            crate::warning::warned!(
                "FIX column kept what reads and nulled the rest",
                column.name(),
                "{refusal}"
            );
            return Ok(held);
        }
    }
    // The null is asked of the column rather than assumed, so a column that
    // refuses one answers with the refusal the value earned.
    match column.scalar(crate::Scalar::Null) {
        Ok(null) => {
            crate::warning::warned!(
                "FIX column value unreadable, stored as null",
                column.name(),
                "{refusal}"
            );
            Ok(null)
        }
        Err(_) => Err(refusal),
    }
}

/// A price or a quantity this crate settled, narrowed to the column that
/// holds it.
///
/// This crate keeps a price and a quantity exact - `decimal128(38, 18)` -
/// because a price is money and a float is not; FIX's own `Price` and `Qty`
/// fields the dictionary types as `float64`, because that is what their text
/// always was. A number this crate settled therefore meets a float column,
/// and the narrowing is made here, once, rather than at each writer: the
/// alternative is a null, which loses the fact.
/// A settled number is the crate's fixed `decimal` leaf, and the dictionary
/// types the column it lands in as `decimal128(38, 18)`: the fact is restated
/// as that column's own value, so one tag answers one type whether it is
/// read off the holder, off the row, or out of an Arrow column. Nothing else
/// narrows - a decimal column takes any other decimal whole.
pub(super) fn narrowed(column: &Field, value: crate::Scalar) -> crate::Scalar {
    let fixed = matches!(
        value,
        crate::Scalar::Decimal(_) | crate::Scalar::BigDecimal(_)
    );
    let decimal = fixed
        || matches!(
            value,
            crate::Scalar::Decimal32(_)
                | crate::Scalar::Decimal64(_)
                | crate::Scalar::Decimal128(_)
                | crate::Scalar::Decimal256(_)
        );
    match column.dtype() {
        DataType::Float64 | DataType::Float32 if decimal => crate::Decimal::from_scalar(&value)
            .map_or(value, |held| crate::Scalar::from(held.to_f64())),
        DataType::Decimal32 { .. }
        | DataType::Decimal64 { .. }
        | DataType::Decimal128 { .. }
        | DataType::Decimal256 { .. }
            if fixed =>
        {
            // The column's own leaf where the number fits it; where it does
            // not, the row's write is what refuses, located.
            column.dtype().scalar(value.clone()).unwrap_or(value)
        }
        _ => value,
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
        DataType::Struct(members) => value.sequence_rows().map(|stated| {
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
        DataType::Serie(item)
        | DataType::LargeSerie(item)
        | DataType::SerieView(item)
        | DataType::LargeSerieView(item)
        | DataType::FixedSizeSerie(item, _) => value.as_serie().map(|stated| {
            Some(crate::Scalar::from_sequence(
                stated
                    .iter()
                    .filter_map(|held| refit(item, held.into_owned()))
                    .collect::<Vec<_>>(),
            ))
        }),
        map_dtype @ (DataType::Map(_) | DataType::SortedMap(_)) => {
            let map = &map_dtype
                .as_mapping()
                .expect("the variant was just matched");
            value.as_mapping().map(|stated| {
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
            })
        }
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
pub(super) const CLOCK_DATATYPE: DataType = DataType::DateTime64 {
    unit: crate::TimeUnit::Nanosecond,
    timezone: crate::Timezone::UTC,
};

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

    /// The digest of a row's shape.
    #[must_use]
    pub fn shape_digest(root: &Field) -> u64 {
        super::shape_digest(root)
    }
}
