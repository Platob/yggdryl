//! The one public door the crates this core is split into reach its
//! crate-private items through.
//!
//! Nothing here is API, and nothing here is for a caller outside the
//! workspace: an item is listed because a crate split off the core - the
//! market crate, the FIX crate, the media crates - needs it, and an item no
//! split crate reaches is not listed. Each reaches the core by one route:
//!
//! - a `pub use` of an item raised to `pub` inside a module the crate root
//!   does not publish, so the raise publishes nothing else;
//! - an `#[inline]` forwarder for an item of a published module, which stays
//!   as private as it was - a crate-private associated item forwarded as a
//!   free function taking its receiver first, named `<type>_<item>`;
//! - a definition moved here, where raising it inside a published module
//!   would publish it: [`InstantSequence`] and [`Staged`];
//! - an exported macro, `#[doc(hidden)]` at the crate root and re-exported
//!   here, whose expansion names what it needs through this module.
//!
//! A proven landing never crosses: the one landing door here,
//! [`land_unproven_batch`], takes nothing on trust and reads every leaf its
//! layout does not prove.

use std::borrow::Cow;
use std::fmt;
use std::hash::Hasher;

use arrow_array::RecordBatch;
use arrow_schema::Schema;
use smol_str::SmolStr;

use crate::graph::{Element, Event};
use crate::text::{TextBytes, TextEntries};
use crate::xxhash::Xxh3;
use crate::{
    Cfi, Charset, DataType, DateTime64, Field, IOBase, Isin, Listing, MarketDescriptor, Metadata,
    Mic, MimeType, Result, Scalar, Serie, State, Str, StructType, Time32, Time64, TimeUnit,
    Timezone, Uuid,
};

// ------------------------------------------------------------------------
// Both crates: what the market crate and the FIX crate each reach.
// ------------------------------------------------------------------------

/// `boolean::bool_of`, for the market and FIX crates: a boolean as itself, a
/// string leaf read through the one boolean table, anything else `None`.
#[inline]
pub fn bool_of(value: &Scalar) -> Option<bool> {
    crate::boolean::bool_of(value)
}

/// `code::folded_spelling`, for the market and FIX crates: one spelling
/// folded the way every name in the crate folds.
#[inline]
pub fn folded_spelling(spelling: &str) -> SmolStr {
    crate::code::folded_spelling(spelling)
}

/// `code::is_null_like`, for the market and FIX crates: whether `text` is a
/// spelling that states no value (`null`, `none`, `n/a`, the empty text).
#[inline]
pub fn is_null_like(text: &str) -> bool {
    crate::code::is_null_like(text)
}

/// `graph::element::right_is_reference`, for the market and FIX crates:
/// whether the right of two statements of one event is the reference, by
/// their `sendunix`, then their `transunix`.
#[inline]
pub fn right_is_reference(
    left_sendunix: Option<i64>,
    left_transunix: i64,
    right_sendunix: Option<i64>,
    right_transunix: i64,
) -> bool {
    crate::graph::element::right_is_reference(
        left_sendunix,
        left_transunix,
        right_sendunix,
        right_transunix,
    )
}

/// The crate's one fold, for the market and FIX crates: whether two
/// spellings are one name.
pub use crate::parser::folds_equal;

/// The allocation-free value path every recursive walk reports a failure
/// under, for the market and FIX crates.
pub use crate::path::{Path, Segment};

/// The deduplicated data warning, for the market and FIX crates: [`warn`]
/// is what the exported `warned!` expands to.
pub use crate::logging::warning::warn;

/// `warned!(what, subject, "format", args..)`: one deduplicated warning at
/// the calling module, for the market and FIX crates.
#[doc(hidden)]
pub use crate::warned;

/// `Isin::from_proven`, for the market and FIX crates: a number a landed
/// `isin` column holds, adopted as it stands.
#[inline]
pub fn isin_from_proven(text: &str) -> Isin {
    Isin::from_proven(text)
}

/// `Mic::from_market`, for the market and FIX crates: the market `text`
/// names, an ISO 10383 MIC as it is, else the one a Reuters exchange
/// mnemonic resolves to.
#[inline]
pub fn mic_from_market(text: &str) -> Option<Mic> {
    Mic::from_market(text)
}

/// `Scalar::try_build_sequence`, for the market and FIX crates: a
/// known-width sequence built in place, the whole run at once.
///
/// # Errors
///
/// Returns the first refusal `build` answers.
#[inline]
pub fn scalar_try_build_sequence(
    len: usize,
    build: impl FnOnce(&mut [Scalar]) -> Result<()>,
) -> Result<Scalar> {
    Scalar::try_build_sequence(len, build)
}

/// `StructType::from_unique_fields`, for the market and FIX crates: the
/// struct over children each valid and named once by construction, checked
/// by nothing but a debug assertion.
#[inline]
pub fn struct_type_from_unique_fields(fields: Vec<Field>) -> StructType {
    StructType::from_unique_fields(fields)
}

/// How many distinct contents one instant places inline before a run
/// spills its codes into a map: a message and what its parse split off.
const INLINE_PLACES: usize = 4;

/// How many contents past the inline ones a run remembers: a run is the
/// events of one nanosecond, so only a stream stamping thousands of events
/// with one clock - every undated message under one default sending time -
/// reaches it, and a content past it takes the next place without being
/// remembered, a later statement of it taking a place of its own. One
/// megabyte of held state at most.
const SPILLED_PLACES: usize = 1 << 16;

/// The place each event of a stream takes among the events of its instant,
/// in the order the stream hands them over: the one rule [`Event::get_seqnum`]
/// states, held once per stream by every door that places what it yields -
/// the market crate's walk and the FIX crate's parse doors.
///
/// A run is the events a stream hands over at one `transunix`, one after
/// another, and the next instant starts a run of its own at zero. A door
/// places either by order - every event the next place of its run, which is
/// what a parse hands over - or by content - the first content of a run
/// place zero, each content after it the next, and a statement of a content
/// the run already placed that content's place, which is what a walk reads,
/// so two statements of one event are one identity. Its state is the one
/// run it is in: the instant, how many it placed, the codes it placed by
/// content - inline to four, so a run of a message and its splits costs no
/// allocation, and past them to 65,536 in a map - and the identities its
/// last four placings moved, so a message split off one, handed over right
/// after it, names the identity its source was placed under.
#[derive(Debug, Default)]
pub struct InstantSequence {
    /// The instant the current run stands at, none before the first event.
    unix: Option<i64>,
    /// How many places the current run has given.
    placed: u64,
    /// The first contents of the run, in place order.
    inline: [u64; INLINE_PLACES],
    /// The run's contents past the inline ones, each to its place.
    spilled: std::collections::HashMap<u64, u64>,
    /// Each identity the run's last placings moved, beside the one it moved
    /// to, oldest first: at most [`INLINE_PLACES`], because a message a
    /// parse split off another is handed over right after it, so held
    /// inline and never allocated.
    moved: [(Uuid, Uuid); INLINE_PLACES],
    /// How many of `moved`, from the first, the current run holds.
    moves: usize,
}

impl InstantSequence {
    /// The place an event at `unix` takes after what this stream handed
    /// over before it, counted into the run: by content `code` where one is
    /// given - a content the run placed takes its place again - else by
    /// order.
    pub fn place(&mut self, unix: i64, code: Option<u64>) -> u64 {
        let seqnum = self.find(unix, code);
        self.record(unix, code, seqnum);
        seqnum
    }

    /// Places `event` by order after what this stream handed over before
    /// it - the next place of its instant's run - first naming each source a
    /// placing of this run moved by the identity it moved to: a message a
    /// parse split off another follows it at its instant, and names it.
    pub fn place_naming_sources<E: Event + ?Sized>(&mut self, event: &mut E) {
        let unix = event.get_transunix();
        let moved = &self.moved[..self.moves];
        if self.unix == Some(unix) && !moved.is_empty() {
            let named = event.get_srcuuids();
            if named
                .iter()
                .any(|source| moved.iter().any(|(from, _)| from == source))
            {
                // The newest move from a source is the one this message
                // names: twins at one instant share one identity before
                // their places, and each split message follows its own.
                let renamed = named
                    .iter()
                    .map(|source| {
                        moved
                            .iter()
                            .rev()
                            .find(|(from, _)| from == source)
                            .map_or(*source, |(_, to)| *to)
                    })
                    .collect();
                event.set_srcuuids(renamed);
            }
        }
        let seqnum = self.find(unix, None);
        let before = event.get_uuid();
        restate(event, seqnum);
        self.record(unix, None, seqnum);
        let after = event.get_uuid();
        if after != before {
            if self.moves == INLINE_PLACES {
                self.moved.copy_within(1.., 0);
                self.moves -= 1;
            }
            self.moved[self.moves] = (before, after);
            self.moves += 1;
        }
    }

    /// The place an event of content `code` takes at `unix`, counting
    /// nothing: by content, the one its content already took in the run;
    /// else the next.
    fn find(&self, unix: i64, code: Option<u64>) -> u64 {
        if self.unix != Some(unix) {
            return 0;
        }
        let Some(code) = code else {
            return self.placed;
        };
        let inline =
            usize::try_from(self.placed).map_or(INLINE_PLACES, |placed| placed.min(INLINE_PLACES));
        if let Some(at) = self.inline[..inline].iter().position(|held| *held == code) {
            return at as u64;
        }
        self.spilled.get(&code).copied().unwrap_or(self.placed)
    }

    /// Counts an event placed at `seqnum` at `unix` into the run - its
    /// content `code` where it was placed by content - a new instant
    /// starting a run of its own.
    fn record(&mut self, unix: i64, code: Option<u64>, seqnum: u64) {
        if self.unix != Some(unix) {
            self.unix = Some(unix);
            self.placed = 0;
            self.spilled.clear();
            self.moves = 0;
        }
        if seqnum != self.placed {
            return;
        }
        if let Some(code) = code {
            match usize::try_from(seqnum) {
                Ok(index) if index < INLINE_PLACES => self.inline[index] = code,
                _ if self.spilled.len() < SPILLED_PLACES => {
                    self.spilled.insert(code, seqnum);
                }
                _ => {}
            }
        }
        self.placed = seqnum.saturating_add(1);
    }
}

/// Restates `event`'s place where it moves.
fn restate<E: Event + ?Sized>(event: &mut E, seqnum: u64) {
    if event.get_seqnum() != seqnum {
        event.set_seqnum(seqnum);
    }
}

// ------------------------------------------------------------------------
// Market: what the market crate alone reaches.
// ------------------------------------------------------------------------

/// The row-to-batch reader's default rows per batch and its reader over
/// native rows, for the market crate.
pub use crate::arrow::rows::{DEFAULT_BATCH_ROW_SIZE, reader};

/// `arrow::size::ROW_OVERHEAD`, for the market crate: the bytes every row
/// costs before its payload, as the memory estimator charges it.
pub const ROW_OVERHEAD: usize = crate::arrow::size::ROW_OVERHEAD;

/// `bbg::BBG_WIDTH`, for the market crate: the most bytes a Bloomberg
/// identifier may be.
pub const BBG_WIDTH: usize = crate::bbg::BBG_WIDTH;

/// `ccy::CCY_WIDTH`, for the market crate: the most bytes a currency code
/// may be.
pub const CCY_WIDTH: usize = crate::ccy::CCY_WIDTH;

/// `dti::DTI_WIDTH`, for the market crate: the exact width of a digital
/// token identifier.
pub const DTI_WIDTH: usize = crate::dti::DTI_WIDTH;

/// `elf::ELF_WIDTH`, for the market crate: the exact width of an entity
/// legal form code.
pub const ELF_WIDTH: usize = crate::elf::ELF_WIDTH;

/// `fisn::FISN_WIDTH`, for the market crate: the most bytes a financial
/// instrument short name may be.
pub const FISN_WIDTH: usize = crate::fisn::FISN_WIDTH;

/// `lei::LEI_WIDTH`, for the market crate: the exact width of a legal entity
/// identifier.
pub const LEI_WIDTH: usize = crate::lei::LEI_WIDTH;

/// `ric::RIC_WIDTH`, for the market crate: the most bytes a Reuters
/// instrument code may be.
pub const RIC_WIDTH: usize = crate::ric::RIC_WIDTH;

/// `fisn::similarity`, for the market crate: how alike two short names are,
/// `1` where both are empty.
#[inline]
pub fn similarity(left: &str, right: &str) -> f64 {
    crate::fisn::similarity(left, right)
}

/// `fisn::below_threshold`, for the market crate: whether two texts of
/// `left` and `right` bytes cannot be `threshold` similar, by their lengths
/// alone.
#[inline]
pub fn below_threshold(left: usize, right: usize, threshold: f64) -> bool {
    crate::fisn::below_threshold(left, right, threshold)
}

/// `graph::element::canonicalize_uuids`, for the market crate: an owned
/// UUID list sorted and deduplicated in its own buffer.
#[inline]
pub fn canonicalize_uuids(values: &mut Vec<Uuid>) {
    crate::graph::element::canonicalize_uuids(values);
}

/// `graph::element::crosshash`, for the market crate: the XXH3-64 digest of
/// one cross code's bytes.
#[inline]
pub fn crosshash(crosscode: &str) -> u64 {
    crate::graph::element::crosshash(crosscode)
}

/// `graph::element::merge_element`, for the market crate: the facts an
/// element takes from another statement of itself; whether any moved.
#[inline]
pub fn merge_element<E: Element + ?Sized>(this: &mut E, other: &E) -> bool {
    crate::graph::element::merge_element(this, other)
}

/// `graph::element::merge_event_element`, for the market crate: the element
/// facts of two event statements, the reference leading; whether any moved.
#[inline]
pub fn merge_event_element<E: Element + ?Sized>(
    this: &mut E,
    other: &E,
    other_is_reference: bool,
) -> bool {
    crate::graph::element::merge_event_element(this, other, other_is_reference)
}

/// `graph::element::restate_event`, for the market crate: the facts an event
/// takes from restating `live`, another statement of the same event.
#[inline]
pub fn restate_event<E: Event + ?Sized>(this: &mut E, live: &E) {
    crate::graph::element::restate_event(this, live);
}

/// `graph::element::follow_element`, for the market crate: the facts an
/// element takes from following `previous`; whether any moved.
#[inline]
pub fn follow_element<E: Element + ?Sized>(this: &mut E, previous: &E) -> bool {
    crate::graph::element::follow_element(this, previous)
}

/// `graph::element::moved`, for the market crate: records `next` through
/// `set` where it differs from `current`, answering whether it did.
#[inline]
pub fn moved<T: PartialEq>(current: T, next: T, set: impl FnOnce(T)) -> bool {
    crate::graph::element::moved(current, next, set)
}

/// `graph::element::earliest`, for the market crate: the earlier of two
/// optional instants, or whichever is stated.
#[inline]
pub fn earliest(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    crate::graph::element::earliest(left, right)
}

/// `graph::element::latest`, for the market crate: the later of two optional
/// instants, or whichever is stated.
#[inline]
pub fn latest(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    crate::graph::element::latest(left, right)
}

/// `graph::element::fold_event_instants`, for the market crate: the earliest
/// `sendunix` two statements of one event know; whether it moved.
#[inline]
pub fn fold_event_instants<E: Event + ?Sized>(this: &mut E, other: &E) -> bool {
    crate::graph::element::fold_event_instants(this, other)
}

/// `graph::element::follow_timed`, for the market crate: the timed facts an
/// event takes from following `previous`; whether any moved.
#[inline]
pub fn follow_timed<E: Event>(this: &mut E, previous: &E) -> bool {
    crate::graph::element::follow_timed(this, previous)
}

/// `graph::element::merge_timed`, for the market crate: the timed facts an
/// event takes from another statement of itself; whether any moved.
#[inline]
pub fn merge_timed<E: Event>(this: &mut E, other: &E, other_is_reference: bool) -> bool {
    crate::graph::element::merge_timed(this, other, other_is_reference)
}

/// `graph::element::stated`, for the market crate: the fact the selected
/// statement states, else the other's, else nothing.
#[inline]
pub fn stated<T>(this: Option<T>, other: Option<T>, later: bool) -> Option<T> {
    crate::graph::element::stated(this, other, later)
}

/// Facts staged on the stack and written to a digest a chunk at a time, for
/// the market crate's operation, book and market digests.
///
/// A fact is four writes, and an element states dozens - its names, its
/// market, a side's every live entry: staged, a digest costs a write per
/// chunk instead, and the state reads the same bytes in the same order, so
/// the code is the one the element digest's own fact feed answers.
/// What is still staged is written when the stage is dropped.
pub struct Staged<'state> {
    state: &'state mut Xxh3,
    held: [u8; 512],
    len: usize,
}

impl<'state> Staged<'state> {
    /// A stage over `state`, nothing staged yet.
    pub fn new(state: &'state mut Xxh3) -> Self {
        Self {
            state,
            held: [0; 512],
            len: 0,
        }
    }

    /// One named fact, staged: the name, the bytes, each closed by a byte
    /// no name or value holds, so two facts never read as one.
    pub fn feed(&mut self, name: &str, bytes: &[u8]) {
        self.write(name.as_bytes());
        self.write(&[0]);
        self.write(bytes);
        self.write(&[0]);
    }

    /// Raw bytes, staged: a run longer than the stage is written through.
    pub fn write(&mut self, bytes: &[u8]) {
        if self.len + bytes.len() > self.held.len() {
            self.flush();
            if bytes.len() > self.held.len() {
                self.state.write(bytes);
                return;
            }
        }
        self.held[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }

    fn flush(&mut self) {
        if self.len > 0 {
            self.state.write(&self.held[..self.len]);
            self.len = 0;
        }
    }
}

impl Drop for Staged<'_> {
    fn drop(&mut self) {
        self.flush();
    }
}

/// A stream of batches as the serie it is, for the market crate's registry
/// store.
pub use crate::iomedia::arrow_serie;

/// `media::format::locate`, for the market crate's registry store: the table
/// a container handle addresses, if any claimed format locates one.
///
/// # Errors
///
/// Returns a format's refusal of a metadata document it found but could not
/// read.
#[inline]
pub fn locate<H: IOBase + ?Sized>(
    handle: &H,
) -> Result<Option<Box<dyn crate::media::LocatedTable>>> {
    crate::media::format::locate(handle)
}

/// `media::partition::record_parts`, for the market crate's registry store:
/// the leaves beneath `folder` that hold `encoding`.
///
/// # Errors
///
/// Returns the listing's failure.
#[inline]
pub fn record_parts(folder: &(impl IOBase + ?Sized), encoding: MimeType) -> Result<Listing> {
    crate::media::partition::record_parts(folder, encoding)
}

/// One captured text read under the datatype its capture was typed with,
/// for the market crate's book service.
pub use crate::text::arrow::parse_capture;

/// The enum leaf builder, for the market crate's kinds: `enum_leaf!` writes
/// a kind's enum, its descriptor and its typed value.
#[doc(hidden)]
pub use crate::enum_leaf;

/// The spelling patterns an `enum_leaf!` kind reads a member's words by,
/// for the macro's expansion in the market crate.
pub use crate::enums::patterns::Patterns;

/// The field marker builder, for the market crate's kinds:
/// `define_field_types!` writes a kind's marker and field alias.
#[doc(hidden)]
pub use crate::define_field_types;

/// `MarketDescriptor::adopt_code`, for the market crate's kinds: a member's
/// own code adopted as the value it is, unread - what an `enum_leaf!` kind's
/// typed member crosses into a scalar through.
#[inline]
pub fn adopt_market_code(kind: &'static MarketDescriptor, code: u16) -> Scalar {
    kind.adopt_code(code)
}

/// `Cfi::from_proven`, for the market crate's registry: a code a landed
/// `cfi` column holds, adopted as it stands.
#[inline]
pub fn cfi_from_proven(text: &str) -> Cfi {
    Cfi::from_proven(text)
}

/// `Mic::from_proven`, for the market crate's registry: a code a landed
/// `mic` column holds, adopted as it stands.
#[inline]
pub fn mic_from_proven(text: &str) -> Mic {
    Mic::from_proven(text)
}

// ------------------------------------------------------------------------
// FIX: what the FIX crate alone reaches.
// ------------------------------------------------------------------------

/// The row-to-batch reader over rows the producer proved canonical and cuts
/// itself, and the row it reads beside its cut, for the FIX crate's batch
/// reader.
pub use crate::arrow::rows::{Closing, canonical_closing_reader};

/// `DateTime64::from_fix_text`, for the FIX crate: a datetime as FIX spells
/// one, a zoneless reading a wall clock in `naive`.
///
/// # Errors
///
/// The general reading's refusal when `text` is no FIX datetime; an error
/// when a wall clock does not exist in `naive`'s rules.
#[inline]
pub fn datetime64_from_fix_text(text: &str, naive: Timezone) -> Result<DateTime64> {
    DateTime64::from_fix_text(text, naive)
}

/// `DateTime64::from_fix_clock`, for the FIX crate: a `TZTimeOnly` as FIX
/// spells one, a clock and no date on the epoch day.
///
/// # Errors
///
/// Returns a parse error for a dated value or text that is no clock, and an
/// error when a wall clock does not exist in `naive`'s rules.
#[inline]
pub fn datetime64_from_fix_clock(text: &str, naive: Timezone) -> Result<DateTime64> {
    DateTime64::from_fix_clock(text, naive)
}

/// `Time32::from_fix_text`, for the FIX crate: a time of day as FIX spells
/// one, at 32 bits.
///
/// # Errors
///
/// The ISO clock reader's refusal, a stated zone included.
#[inline]
pub fn time32_from_fix_text(text: &str) -> Result<Time32> {
    Time32::from_fix_text(text)
}

/// `Time64::from_fix_text`, for the FIX crate: a time of day as FIX spells
/// one, at 64 bits.
///
/// # Errors
///
/// The ISO clock reader's refusal, a stated zone included.
#[inline]
pub fn time64_from_fix_text(text: &str) -> Result<Time64> {
    Time64::from_fix_text(text)
}

/// `enums::read_enum_spelling`, for the FIX crate: the member of the enum
/// leaf `dtype` one spelling names, as the scalar it is.
///
/// # Errors
///
/// The leaf's own refusal naming the spelling, or one naming `dtype` where
/// it is not an enum leaf.
#[inline]
pub fn read_enum_spelling(dtype: &DataType, spelling: &str) -> Result<Scalar> {
    crate::enums::read_enum_spelling(dtype, spelling)
}

/// `graph::element::feed_event_facts`, for the FIX crate's message digest:
/// what an event digest feeds less the cross code, its state and its
/// predecessor's identity.
#[inline]
pub fn feed_event_facts<E: Event + ?Sized>(state: &mut Xxh3, this: &E) {
    crate::graph::element::feed_event_facts(state, this);
}

/// `graph::element_column::whole_u64`, for the FIX crate: the `uint64` one
/// cell states through the datatype's own value door.
#[inline]
pub fn whole_u64(value: &Scalar) -> Option<u64> {
    crate::graph::element_column::whole_u64(value)
}

/// `graph::element_column::digest_u64`, for the FIX crate: the digest one
/// cell states, an `int64` cell's bits included.
#[inline]
pub fn digest_u64(value: &Scalar) -> Option<u64> {
    crate::graph::element_column::digest_u64(value)
}

/// The shallow line scan the FIX codec frames a log line by: the escaped
/// separators, the scan and its message type, the document behind a
/// prefix, the located frame, the trim, the payload and the JSON span.
pub use crate::mime_type::line::{
    Located, SOH_MARKERS, document_behind_prefix, inspect, json_span, payload_at, trim_ascii,
};

/// The crate's one fold - the folded characters, their digest and their
/// owned text - for the FIX crate's dictionary.
pub use crate::parser::{fold_digest, folded, normalized};

/// `Scalar::try_sequence`, for the FIX crate: a known-width sequence built
/// in its final shared storage, one value per index.
///
/// # Errors
///
/// Returns the first refusal `at` answers.
#[inline]
pub fn scalar_try_sequence(len: usize, at: impl FnMut(usize) -> Result<Scalar>) -> Result<Scalar> {
    Scalar::try_sequence(len, at)
}

/// `Scalar::try_fill_sequence`, for the FIX crate: a known-width sequence
/// built by writing each slot where it is stored.
///
/// # Errors
///
/// Returns the first refusal `fill` answers.
#[inline]
pub fn scalar_try_fill_sequence(
    len: usize,
    fill: impl FnMut(usize, &mut Scalar) -> Result<()>,
) -> Result<Scalar> {
    Scalar::try_fill_sequence(len, fill)
}

/// `Scalar::leaf_display`, for the FIX crate: the width leaf's own
/// [`fmt::Display`], for a variant that holds one.
#[inline]
pub fn scalar_leaf_display(value: &Scalar) -> Option<&dyn fmt::Display> {
    value.leaf_display()
}

/// `Scalar::from_decimal_text`, for the FIX crate: `text` read as a decimal
/// at the scale `dtype` declares.
///
/// # Errors
///
/// Returns a parse error for text that is not a decimal, and a record error
/// for one the declared scale or width cannot hold.
#[inline]
pub fn scalar_from_decimal_text(dtype: &DataType, text: &str) -> Result<Scalar> {
    Scalar::from_decimal_text(dtype, text)
}

/// The boxed field every level of one root lands under, resolved once, for
/// the FIX crate's batch reader.
pub use crate::serie::arrow::Resolved;

/// `serie::land_batch` with nothing taken on trust, for the FIX crate's
/// batch reader: one batch landed as the record column of `root`, every
/// leaf whose layout is not its datatype's whole contract read once.
///
/// # Errors
///
/// Returns the landing's refusal, naming the batch row and the path below
/// it.
#[inline]
pub fn land_unproven_batch(root: &Resolved, batch: RecordBatch) -> crate::arrow::Result<Serie> {
    crate::serie::land_batch(root, batch, &crate::serie::Proof::Unproven)
}

/// `string::str_from_value`, for the FIX crate: the canonical text a value
/// spells, `None` for a kind that spells none.
#[inline]
pub fn str_from_value(value: &Scalar) -> Option<Result<Str>> {
    crate::string::str_from_value(value)
}

/// `timezone::civil_from_days`, for the FIX crate: the civil date a count
/// of days since the epoch falls on, its year saturated.
#[inline]
pub const fn civil_from_days(days: i64) -> (i32, u32, u32) {
    crate::timezone::civil_from_days(days)
}

/// `uri::percent_decode`, for the FIX crate: one component's percent escapes
/// decoded into the text they stand for, borrowed where there are none.
///
/// # Errors
///
/// Returns a parse error naming `target` for a malformed escape or bytes
/// that are not UTF-8.
#[inline]
pub fn percent_decode<'a>(value: &'a str, target: &'static str) -> Result<Cow<'a, str>> {
    crate::uri::percent_decode(value, target)
}

/// `xml::declared_charset`, for the FIX crate: the charset an XML
/// declaration names, if it names one.
///
/// # Errors
///
/// Returns a parse error naming the charset vocabulary when the declaration
/// names one the crate has no table for.
#[inline]
pub fn declared_charset(head: &[u8]) -> Result<Option<Charset>> {
    crate::xml::declared_charset(head)
}

/// `xxhash::write_named_bytes`, for the FIX crate: the canonical record
/// framing of an already sorted, borrowed named row.
#[inline]
pub fn write_named_bytes<'a>(
    sink: &mut impl Hasher,
    cells: impl ExactSizeIterator<Item = (&'a str, &'a Scalar)>,
    depth: usize,
) {
    crate::xxhash::write_named_bytes(sink, cells, depth);
}

/// `Cfi::UNCLASSIFIED`, for the FIX crate: the code that classifies
/// nothing, every position unknown.
pub const CFI_UNCLASSIFIED: &str = Cfi::UNCLASSIFIED;

/// `Metadata::shares_storage_with`, for the FIX crate: whether two metadata
/// values share one backing map.
#[inline]
pub fn metadata_shares_storage_with(metadata: &Metadata, other: &Metadata) -> bool {
    metadata.shares_storage_with(other)
}

/// `Metadata::storage_address`, for the FIX crate: the address of the
/// backing map, what two values sharing storage share.
#[inline]
pub fn metadata_storage_address(metadata: &Metadata) -> usize {
    metadata.storage_address()
}

/// `StructType::from_checked_fields`, for the FIX crate: the struct over
/// children each already valid, only their names checked.
///
/// # Errors
///
/// Returns an error when two children share a name.
#[inline]
pub fn struct_type_from_checked_fields(fields: Vec<Field>) -> Result<StructType> {
    StructType::from_checked_fields(fields)
}

/// `StructType::shares_storage_with`, for the FIX crate: whether two
/// structs share one children storage.
#[inline]
pub fn struct_type_shares_storage_with(fields: &StructType, other: &StructType) -> bool {
    fields.shares_storage_with(other)
}

/// `StructType::storage_address`, for the FIX crate: the address of the
/// children's storage, zero for no children.
#[inline]
pub fn struct_type_storage_address(fields: &StructType) -> usize {
    fields.storage_address()
}

/// `TextEntries::from_bytes_direct_located`, for the FIX codec: a line's
/// entries beside what the scan located, in one scan.
#[inline]
pub fn text_entries_from_bytes_direct_located(body: &TextBytes) -> (Option<TextEntries>, Located) {
    TextEntries::from_bytes_direct_located(body)
}

/// `Field::new_with_metadata`, for the FIX crate's dictionary: a field
/// around a metadata snapshot that is already valid.
#[inline]
pub fn field_new_with_metadata(
    name: impl Into<SmolStr>,
    dtype: DataType,
    nullable: bool,
    metadata: Metadata,
) -> Field {
    Field::new_with_metadata(name, dtype, nullable, metadata)
}

/// `state::state_from_wire_code`, for the FIX crate's status reader: the
/// state one `OrdStatus(39)` or `ExecType(150)` wire code names.
#[inline]
pub fn state_from_wire_code(code: &str) -> Option<State> {
    crate::state::state_from_wire_code(code)
}

/// The protocol-view builder, for the FIX crate's `FixField` and
/// `FixFieldMut`: `protocol_field_types!` declares one scheme's two views.
#[doc(hidden)]
pub use crate::protocol_field_types;

// ------------------------------------------------------------------------
// Shared with the media crates: what the market or FIX crate reaches that
// a media folder reaches too.
// ------------------------------------------------------------------------

/// `integer::integer_from_text_as`, for the market, FIX and media crates:
/// an integer read out of its canonical spelling at one native width, a
/// magnitude `T` cannot hold `None`.
#[inline]
pub fn integer_from_text_as<T: TryFrom<i128> + TryFrom<u128>>(text: &str) -> Option<T> {
    crate::integer::integer_from_text_as(text)
}

/// `http::server::normalize_path`, for the market crate's book service and
/// the media crates: the canonical spelling of a mount prefix or a route
/// path.
///
/// # Errors
///
/// Returns a parse error for a query, a fragment or a control byte in it.
#[cfg(feature = "http")]
#[inline]
pub fn normalize_path(path: &str) -> Result<String> {
    crate::http::server::normalize_path(path)
}

/// `temporal::format_timestamp`, for the market crate's book service and
/// the media crates: a zoned instant spelled as its local reading plus its
/// offset.
#[inline]
pub fn format_timestamp(count: i64, unit: TimeUnit, zone: &Timezone) -> Option<SmolStr> {
    crate::temporal::format_timestamp(count, unit, zone)
}

/// `arrow::field_from_arrow_schema`, for the FIX and media crates: the
/// record root an Arrow schema forms, its dictionary-ID sidecar taken off.
///
/// # Errors
///
/// Returns an error when the Arrow fields cannot form a non-null record
/// root, or when the dictionary-ID sidecar is malformed or conflicts with
/// the schema.
#[inline]
pub fn field_from_arrow_schema(name: &str, schema: &Schema) -> crate::arrow::Result<Field> {
    crate::arrow::field_from_arrow_schema(name, schema)
}

/// `boolean::BOOLEAN_SPELLINGS`, for the FIX and media crates: what every
/// refusal of a boolean spelling names.
pub const BOOLEAN_SPELLINGS: &str = crate::boolean::BOOLEAN_SPELLINGS;

/// `boolean::bool_from_text`, for the FIX and media crates: a boolean read
/// out of text through the one table every flag in the crate reads.
#[inline]
pub fn bool_from_text(text: &str) -> Option<bool> {
    crate::boolean::bool_from_text(text)
}

/// The bounded interpolation every error message crosses - the byte
/// budget, the text and display elisions, and the expected-got sentence -
/// for the market, FIX and media crates.
pub use crate::text::display::{
    ERROR_TEXT_LIMIT, Elided, ElidedDisplay, elide_display, elide_to, expected_got,
};

/// The row-to-batch reader over a fallible stream of rows, for the market
/// and media crates.
pub use crate::arrow::rows::result_reader;

/// The stable XXH3-64 of a structural hash, for the FIX and media crates.
pub use crate::hashing::stable::stable_hash_of;

/// The one ordered map over persistent stream workers, for the FIX and
/// media crates.
pub use crate::parallel::{Ordered, ordered};

/// A natural text value interpreted under one field and validated, for the
/// FIX and media crates.
pub use crate::text::typed::with_field;

/// The six byte leaves as one pattern over `DataType`, for the FIX and
/// media crates.
#[doc(hidden)]
pub use crate::bytes_dtypes;

/// The eighteen string leaves and the six byte leaves as patterns over
/// `Scalar`, for the FIX and media crates.
#[doc(hidden)]
pub use crate::{bytes_scalars, string_scalars};

/// `Serie::is_string_storage`, for the FIX and media crates: whether a serie
/// is one of the text storage layouts, whichever string leaf it declares.
#[inline]
pub fn serie_is_string_storage(serie: &Serie) -> bool {
    serie.is_string_storage()
}

/// `Serie::is_byte_storage`, for the FIX and media crates: whether a serie
/// is one of the byte storage layouts.
#[inline]
pub fn serie_is_byte_storage(serie: &Serie) -> bool {
    serie.is_byte_storage()
}

/// `Serie::value_bytes`, for the FIX and media crates: the bytes one text
/// or byte cell holds, borrowed where they lie.
#[inline]
pub fn serie_value_bytes(serie: &Serie, index: usize) -> Option<&[u8]> {
    serie.value_bytes(index)
}
