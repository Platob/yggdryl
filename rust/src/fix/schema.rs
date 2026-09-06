//! The columns every message has, whatever it happened to carry.
//!
//! A message read into a per-message schema is a message no two of which can
//! be put in one table. This is the other shape: one fixed set of columns,
//! decided once, the same for a Logon and a market-data snapshot - so a
//! capture is a table, a partition is a file, and a reader written against it
//! keeps working when the next line carries a field it has never seen.
//!
//! # Columns are named by tag
//!
//! `35`, not `msgtype`. A tag is the one name a field has in every version
//! and every dialect: tag 32 is `LastShares` in 4.2 and `LastQty` in a newest
//! one, and a column named either of those is a column that changes meaning
//! when a venue upgrades. The tag never moves, so the column never does, and
//! a reader spelling it `FixMsg["35"]` gets the same answer forever.
//!
//! The names are still reachable - the field under each column carries its
//! `fix:tag`, its lineage and its code set - so a caller that wants
//! `msgtype` asks the dictionary and gets `35`.
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
pub const ENTRIES_COLUMN: &str = "entries";

/// The column holding what no dictionary explained.
pub const UNMAPPED_COLUMN: &str = "unmapped";

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
    let crate_fields = super::fix_crate_fields()?;
    let mut fields: Vec<Field> = Vec::with_capacity(fix_schema_tags().len() + 2);
    for tag in fix_schema_tags() {
        let held = registry.get_field_by_tag(tag).cloned().or_else(|| {
            crate_fields
                .iter()
                .find(|field| field.as_fix().tag().ok().flatten() == Some(tag))
                .cloned()
        });
        let Some(mut held) = held else {
            continue;
        };
        // Named by its tag, because that is the one name a field has in every
        // version and dialect. The spelling it had stays on the field, so a
        // renderer can still show `msgtype` over column `35`.
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
        held.set_name(rendered(tag));
        held.set_nullable(true);
        if !fields.iter().any(|known| known.name() == held.name()) {
            fields.push(held);
        }
    }
    fields.push(entries_field(ENTRIES_COLUMN)?);
    fields.push(entries_field(UNMAPPED_COLUMN)?);
    Ok(DataType::from_fields(fields)?.required_field(name))
}

/// One tag as the column name it takes.
#[must_use]
pub fn rendered(tag: i32) -> String {
    tag.to_string()
}

/// A list-of-struct column holding arrival records.
fn entries_field(name: &str) -> Result<Field> {
    let item = DataType::from_fields([
        DataType::Int32.nullable_field("tag"),
        DataType::Utf8.nullable_field("branch"),
        DataType::Utf8.nullable_field("key"),
        DataType::Utf8.nullable_field("value"),
    ])?
    .required_field("item");
    Ok(DataType::list(item).nullable_field(name))
}

/// Where each schema tag sits, resolved once against one dictionary.
///
/// A row projection asks for the same tags in the same order for every
/// message in a capture, and each ask through the ordinary tiers is a hash,
/// a verification and a branch walk. Resolving them once turns the per-row
/// cost into an indexed read - which is the whole reason a fixed schema is
/// worth having.
///
/// Held beside a dictionary rather than inside it: a projection is a reader's
/// concern, and a dictionary that carried one would have to invalidate it on
/// every edit.
pub struct FixProjection {
    /// The root the projection fills.
    field: Field,
    /// Each column's tag, in column order.
    tags: Vec<i32>,
    /// Each column's field, in column order.
    columns: Vec<Field>,
}

impl FixProjection {
    /// Resolves every schema column against one dictionary.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the columns do not make a
    /// struct.
    pub fn new(registry: &FixRegistry, name: impl Into<SmolStr>) -> Result<Self> {
        let field = fix_schema(registry, name)?;
        let columns: Vec<Field> = field
            .dtype()
            .as_fields()
            .map(<[Field]>::to_vec)
            .unwrap_or_default();
        let tags = columns
            .iter()
            .map(|held| held.as_fix().tag().ok().flatten().unwrap_or(0))
            .collect();
        Ok(Self {
            field,
            tags,
            columns,
        })
    }

    /// The root this projection fills.
    #[must_use]
    pub const fn field(&self) -> &Field {
        &self.field
    }

    /// The tag each column carries, in column order.
    #[must_use]
    pub fn tags(&self) -> &[i32] {
        &self.tags
    }

    /// The field behind one column, by position.
    #[must_use]
    pub fn column(&self, at: usize) -> Option<&Field> {
        self.columns.get(at)
    }

    /// How many columns carry a value rather than the arrival record.
    ///
    /// The two closing lists are columns like any other and carry no tag, so
    /// a row fills these and then appends them - rather than counting the
    /// lists twice, which is what a bare `tags().len()` would do.
    #[must_use]
    pub fn value_columns(&self) -> usize {
        self.columns
            .iter()
            .position(|held| held.name() == ENTRIES_COLUMN)
            .unwrap_or(self.columns.len())
    }

    /// Where one tag's column sits, without a dictionary lookup.
    ///
    /// A linear scan over a schema of this size beats a map: the columns are
    /// under a hundred, they are contiguous in cache, and the answer is
    /// usually in the first twenty because a header is read on every row.
    #[must_use]
    pub fn position_of(&self, tag: i32) -> Option<usize> {
        self.tags.iter().position(|held| *held == tag)
    }
}

impl super::FixMsg {
    /// This message as the fixed row a table holds.
    ///
    /// Every column is filled from the message's own values by tag, so a
    /// message that carried nothing at a column answers null there rather
    /// than shifting its neighbours - which is what makes two rows of one
    /// capture comparable at all.
    ///
    /// The first closing list is the whole arrival record, so the row stays
    /// lossless whatever the columns made of it; the second is the part of
    /// that record no dictionary explained, which is a view over the first
    /// rather than the rest of it.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::holder::local::Folder;
    /// # use yggdryl::{FixProjection, FixReader, FixRegistry};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    /// let projection = FixProjection::new(&registry, "fix")?;
    /// let reader = FixReader::new(Arc::clone(&registry));
    /// let order = reader.text("8=FIX.4.4|35=D|55=AAPL|54=1|9999=x|10=0|")?;
    ///
    /// let row = order.to_row(&projection);
    /// let held = row.as_sequence().expect("a row");
    /// // The columns are the tags, so `35` is where the message type is.
    /// let at = projection.position_of(35).expect("the msgtype column");
    /// assert_eq!(held[at].as_str(), Some("D"));
    /// // A tag no dictionary explains is still there, in its own column.
    /// let unmapped = held.last().and_then(yggdryl::Scalar::as_sequence);
    /// assert_eq!(unmapped.map(<[yggdryl::Scalar]>::len), Some(1));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn to_row(&self, projection: &FixProjection) -> crate::Scalar {
        let tags = projection.tags();
        let carried = projection.value_columns();
        let mut values: Vec<crate::Scalar> = Vec::with_capacity(tags.len());
        for (at, tag) in tags.iter().take(carried).enumerate() {
            let value = projection
                .column(at)
                .map_or(crate::Scalar::Null, |field| self.column_value(*tag, field));
            values.push(value);
        }
        let (known, unknown) = self.divided();
        values.push(known);
        values.push(unknown);
        crate::Scalar::from_sequence(values)
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
    fn column_value(&self, tag: i32, target: &Field) -> crate::Scalar {
        if let Some(index) = self.index_of_tag(tag) {
            let source = self.as_field().fields().get(index);
            let value = self.as_value().get(index);
            if let (Some(source), Some(value)) = (source, value) {
                return aligned_value(source, value, target);
            }
        }
        if let Some(held) = self.get_by_tag(tag) {
            return target.scalar(held.clone()).unwrap_or(crate::Scalar::Null);
        }
        if let Some(facet) = DERIVED_FACETS
            .iter()
            .find_map(|(held, facet)| (*held == tag).then_some(*facet))
        {
            if let Some(held) = self.lifted(facet) {
                return target.scalar(held.clone()).unwrap_or(crate::Scalar::Null);
            }
        }
        let derived = match tag {
            super::MSGHASH_TAG => crate::Scalar::from(self.digest().to_be_bytes().to_vec()),
            super::VERSION_TAG => self.version().map_or(crate::Scalar::Null, |held| {
                crate::Scalar::from(held.to_string())
            }),
            super::SYMBOLTICKER_TAG => self.symbol_ticker(),
            super::TIMESTAMP_TAG => self.market_timestamp(),
            super::UNIXPARTITION_TAG => self.unix_partition(super::DEFAULT_PARTITION_SECONDS),
            _ => crate::Scalar::Null,
        };
        target.scalar(derived).unwrap_or(crate::Scalar::Null)
    }

    /// Whether this message's dictionary has a field at one tag.
    fn explains(&self, tag: i32) -> bool {
        tag != 0 && self.registry().get_field_by_tag(tag).is_some()
    }

    /// The arrival record, and the part of it nothing explained.
    fn divided(&self) -> (crate::Scalar, crate::Scalar) {
        let mut known = Vec::new();
        let mut unknown = Vec::new();
        for entry in self.entries() {
            let held = crate::Scalar::from_sequence([
                crate::Scalar::from(entry.tag()),
                entry
                    .branch()
                    .map_or(crate::Scalar::Null, crate::Scalar::from),
                crate::Scalar::from(entry.key()),
                crate::Scalar::from(entry.value()),
            ]);
            // Explained means the dictionary has the field, not that the key
            // parsed: `9999=x` names a tag and still names nothing, and a
            // venue onboarding it wants to find it in exactly one column.
            if !self.explains(entry.tag()) {
                unknown.push(held.clone());
            }
            known.push(held);
        }
        (
            crate::Scalar::from_sequence(known),
            crate::Scalar::from_sequence(unknown),
        )
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
const CLOCK_SOURCES: [ClockSource; 4] = [
    ClockSource::Flat(60),
    ClockSource::Group {
        group: 768,
        member: 769,
    },
    ClockSource::Flat(52),
    ClockSource::Flat(122),
];

/// One place a market clock can live in a resolved message.
enum ClockSource {
    Flat(i32),
    Group { group: i32, member: i32 },
}

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
        let Some((tag, held)) = TICKER_SOURCES.iter().find_map(|tag| {
            let held = self.get_by_tag(*tag)?;
            (!held.is_null()).then_some((*tag, held))
        }) else {
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
    /// The first of [`CLOCK_SOURCES`] the message answers. A group's
    /// timestamp is read from its first occurrence, because a regulatory
    /// clock that ran several times still ran first once.
    #[must_use]
    pub fn market_timestamp(&self) -> crate::Scalar {
        for source in CLOCK_SOURCES {
            let timestamp = match source {
                ClockSource::Flat(tag) => self
                    .get_by_tag(tag)
                    .map_or(crate::Scalar::Null, first_timestamp),
                ClockSource::Group { group, member } => self.group_timestamp(group, member),
            };
            if !timestamp.is_null() {
                return timestamp;
            }
        }
        crate::Scalar::Null
    }

    /// The first valid timestamp member of one repeating group.
    fn group_timestamp(&self, group: i32, member: i32) -> crate::Scalar {
        let Some(at) = self.index_of_tag(group) else {
            return crate::Scalar::Null;
        };
        let Some(field) = self.as_field().fields().get(at) else {
            return crate::Scalar::Null;
        };
        let DataType::List(item) = field.dtype() else {
            return crate::Scalar::Null;
        };
        let Some(member_at) = item.dtype().as_fields().and_then(|fields| {
            fields
                .iter()
                .position(|field| field.as_fix().tag().ok().flatten() == Some(member))
        }) else {
            return crate::Scalar::Null;
        };
        let Some(occurrences) = self.as_value().get(at).and_then(crate::Scalar::as_sequence) else {
            return crate::Scalar::Null;
        };
        for occurrence in occurrences {
            let Some(value) = occurrence
                .as_sequence()
                .and_then(|values| values.get(member_at))
            else {
                continue;
            };
            let timestamp = first_timestamp(value);
            if !timestamp.is_null() {
                return timestamp;
            }
        }
        crate::Scalar::Null
    }

    /// The partition [`Self::market_timestamp`] falls in, in whole seconds.
    ///
    /// Floor division rather than truncation, so a timestamp before the epoch
    /// lands in the partition that contains it rather than the one after.
    #[must_use]
    pub fn unix_partition(&self, seconds: i64) -> crate::Scalar {
        if seconds <= 0 {
            return crate::Scalar::Null;
        }
        let held = self.market_timestamp();
        let Some((count, unit, _)) = held.as_datetime64() else {
            return crate::Scalar::Null;
        };
        // A partition contains the whole fractional instant. Floor at the
        // source unit before flooring to the requested interval, including
        // before the epoch; exact temporal restatement would reject every
        // ordinary FIX timestamp with a fraction.
        let scale = match unit {
            crate::TimeUnit::Second => 1,
            crate::TimeUnit::Millisecond => 1_000,
            crate::TimeUnit::Microsecond => 1_000_000,
            crate::TimeUnit::Nanosecond => 1_000_000_000,
            _ => return crate::Scalar::Null,
        };
        let epoch = count.div_euclid(scale);
        crate::Scalar::from(epoch.div_euclid(seconds) * seconds)
    }
}

/// One message value under the fixed column that will materialize it.
///
/// Repeating-group messages carry only the members that arrived, while the
/// fixed batch schema carries the dictionary's complete item. Aligning by FIX
/// identity fills the absent members with null instead of shifting a short
/// occurrence into the wrong columns. Any value that still cannot satisfy the
/// target becomes null, keeping row content from failing the capture stream.
fn aligned_value(source: &Field, value: &crate::Scalar, target: &Field) -> crate::Scalar {
    if value.is_null() {
        return crate::Scalar::Null;
    }
    let candidate = match (source.dtype(), target.dtype()) {
        (DataType::List(source_item), DataType::List(target_item)) => {
            let Some(values) = value.as_sequence() else {
                return crate::Scalar::Null;
            };
            crate::Scalar::from_sequence(
                values
                    .iter()
                    .map(|held| aligned_value(source_item, held, target_item))
                    .collect::<Vec<_>>(),
            )
        }
        (DataType::Struct(source_fields), DataType::Struct(target_fields)) => {
            let Some(values) = value.as_sequence() else {
                return crate::Scalar::Null;
            };
            crate::Scalar::from_sequence(
                target_fields
                    .iter()
                    .map(|target_field| {
                        source_fields
                            .iter()
                            .position(|source_field| same_fix_field(source_field, target_field))
                            .and_then(|at| {
                                Some(aligned_value(
                                    source_fields.get(at)?,
                                    values.get(at)?,
                                    target_field,
                                ))
                            })
                            .unwrap_or(crate::Scalar::Null)
                    })
                    .collect::<Vec<_>>(),
            )
        }
        _ => value.clone(),
    };
    conformed_value(candidate, target)
}

/// One candidate coerced through both the generic and FIX text contracts.
fn conformed_value(value: crate::Scalar, target: &Field) -> crate::Scalar {
    if let Ok(value) = target.scalar(value.clone()) {
        return value;
    }
    let Some(text) = value.as_str() else {
        return crate::Scalar::Null;
    };
    let Some(candidate) = super::build::wire_spelling(target.dtype(), text) else {
        return crate::Scalar::Null;
    };
    crate::text::prepare_text(candidate, target)
        .and_then(|value| target.scalar(value))
        .unwrap_or(crate::Scalar::Null)
}

/// Whether two nested fields name the same FIX value.
fn same_fix_field(left: &Field, right: &Field) -> bool {
    match (
        left.as_fix().id().ok().flatten(),
        right.as_fix().id().ok().flatten(),
    ) {
        (Some(left), Some(right)) => left == right,
        _ => crate::types::folds_equal(left.name(), right.name()),
    }
}

/// Restate one candidate as the crate's canonical UTC capture timestamp.
fn canonical_timestamp(value: &crate::Scalar) -> crate::Scalar {
    let Some(field) = super::fix_crate_fields().ok().and_then(|fields| {
        fields
            .iter()
            .find(|field| field.as_fix().tag().ok().flatten() == Some(super::TIMESTAMP_TAG))
    }) else {
        return crate::Scalar::Null;
    };
    if let Ok(timestamp) = field.scalar(value.clone()) {
        return timestamp;
    }
    // FIX permits nine fractional digits even though the canonical capture
    // clock is microseconds. A custom dictionary can therefore materialize a
    // nanosecond source exactly; derive the canonical clock by dropping only
    // its sub-microsecond remainder. `div_euclid` matches truncating the
    // printed fraction for instants before the epoch too.
    if let Some((count, crate::TimeUnit::Nanosecond, _)) = value.as_datetime64() {
        if let DataType::DateTime64 {
            unit: crate::TimeUnit::Microsecond,
            timezone,
        } = field.dtype()
        {
            return crate::Scalar::datetime64(
                count.div_euclid(1_000),
                crate::TimeUnit::Microsecond,
                *timezone,
            )
            .unwrap_or(crate::Scalar::Null);
        }
    }
    let Some(text) = value.as_str() else {
        return crate::Scalar::Null;
    };
    let Some(candidate) = super::build::wire_spelling(field.dtype(), text) else {
        return crate::Scalar::Null;
    };
    crate::text::prepare_text(candidate, field)
        .and_then(|prepared| field.scalar(prepared))
        .unwrap_or(crate::Scalar::Null)
}

/// The first valid clock in either a scalar value or repeated occurrences.
fn first_timestamp(value: &crate::Scalar) -> crate::Scalar {
    if value.is_null() {
        return crate::Scalar::Null;
    }
    if let Some(occurrences) = value.as_sequence() {
        for occurrence in occurrences {
            let timestamp = canonical_timestamp(occurrence);
            if !timestamp.is_null() {
                return timestamp;
            }
        }
        return crate::Scalar::Null;
    }
    canonical_timestamp(value)
}

impl FixProjection {
    /// Wraps a root that is already the fixed schema.
    ///
    /// The columns and their tags are read back off the field itself, so a
    /// caller that built the root once does not build it twice.
    #[must_use]
    pub fn from_field(field: Field) -> Self {
        let columns: Vec<Field> = field
            .dtype()
            .as_fields()
            .map(<[Field]>::to_vec)
            .unwrap_or_default();
        let tags = columns
            .iter()
            .map(|held| held.as_fix().tag().ok().flatten().unwrap_or(0))
            .collect();
        Self {
            field,
            tags,
            columns,
        }
    }
}
