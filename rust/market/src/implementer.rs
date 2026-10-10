//! The one door the FIX crate reaches this crate's crate-private items
//! through, as `yggdryl::implementer` is the core's: nothing here is API,
//! and an item is listed because `yggdryl-fix` needs it. Each reaches this
//! crate by one route:
//!
//! - a `pub use` of an item raised to `pub` inside a module the crate root
//!   does not publish, so the raise publishes nothing else;
//! - an `#[inline]` forwarder for an item of a published module, which stays
//!   as private as it was - a crate-private associated item forwarded as a
//!   free function taking its receiver first, named `<type>_<item>`;
//! - an exported macro, `#[doc(hidden)]` at the crate root and re-exported
//!   here, whose expansion names the core through `::yggdryl`.

use std::cmp::Ordering;

use smol_str::SmolStr;
use yggdryl::graph::{Element, Event};

use crate::graph::iterator::sealed::Walked;
use crate::graph::{
    EventIterator, MarketData, Operation, OperationEvent, OperationKind, SnapshotEvent,
};
use crate::identifier::IDENTIFIER_VALUE_WIDTH;
use crate::securityid::SymbolCode;
use crate::{IdKey, IdSource, IdType, Identifiers, Instruments, MarketDataKind, Side};
use yggdryl::Result;

/// `graph::facts::OperationEventFacts`, raised to `pub` in a module the root does not publish.
pub use crate::graph::facts::OperationEventFacts;
/// `idtype::names_another_instrument`, raised to `pub` in a module the root does not publish.
pub use crate::idtype::names_another_instrument;
/// `instrument::EconomicMemo`, raised to `pub` in a module the root does not publish.
pub use crate::instrument::EconomicMemo;
/// `instrument::InstrumentTable`, raised to `pub` in a module the root does not publish.
pub use crate::instrument::InstrumentTable;
/// `instrument::Learned`, raised to `pub` in a module the root does not publish.
pub use crate::instrument::Learned;
/// `instrument::warn_full`, raised to `pub` in a module the root does not publish.
pub use crate::instrument::warn_full;
/// `instrument::Stated`, `instrument::Body` and `instrument::Production`, raised to `pub` in a module the root does not publish.
pub use crate::instrument::{Body, Production, Stated};
pub use crate::{delegate_market, delegate_operation};

/// `identifier::WORD_PAIR_WIDTH`: the most bytes two words spell with one
/// byte between them.
pub const WORD_PAIR_WIDTH: usize = crate::identifier::WORD_PAIR_WIDTH;

/// `identifier::fold_into`: `word` folded into `buffer`, as every
/// identifier word folds.
///
/// # Errors
///
/// What the fold expected, where `word` folds to nothing, past the
/// buffer's bytes, or holds a byte no word does.
#[inline]
pub fn fold_into<'buffer>(
    word: &str,
    buffer: &'buffer mut [u8],
) -> std::result::Result<&'buffer str, &'static str> {
    crate::identifier::fold_into(word, buffer)
}

/// `identifier::is_word`: whether a type or a source folds from `word`.
#[inline]
pub fn is_word(word: &str) -> bool {
    crate::identifier::is_word(word)
}

/// `identifier::folded_len`: how many bytes `word` folds to, or none where
/// it holds a byte no word does.
#[inline]
pub fn folded_len(word: &str) -> Option<usize> {
    crate::identifier::folded_len(word)
}

/// `Identifiers::held_at`: where the identifier keyed `key` and holding
/// `value` is.
#[inline]
pub fn identifiers_held_at(identifiers: &Identifiers, key: &IdKey, value: &str) -> Option<usize> {
    identifiers.held_at(key, value)
}

/// `IdKey::value_into`: `value` canonicalized under the key's type and
/// source into `buffer`.
///
/// # Errors
///
/// A value the type refuses, and one the source refuses, located on the
/// key.
#[inline]
pub fn id_key_value_into<'value>(
    key: &IdKey,
    value: &'value str,
    buffer: &'value mut [u8; IDENTIFIER_VALUE_WIDTH],
) -> Result<&'value str> {
    key.value_into(value, buffer)
}

/// `IdKey::infer`: the key a name no key spells reads as, its identifier
/// name one of `names`.
#[inline]
pub fn id_key_infer<'name>(
    key: &str,
    names: impl IntoIterator<Item = &'name str>,
) -> Option<IdKey> {
    IdKey::infer(key, names)
}

/// `IdSource::from_namespace`: the source a folded namespace names.
///
/// # Errors
///
/// A namespace no word holds.
#[inline]
pub fn id_source_from_namespace(folded: &str) -> Result<Option<IdSource>> {
    IdSource::from_namespace(folded)
}

/// `IdType::value_into`: `value` canonicalized under the type into
/// `buffer`.
///
/// # Errors
///
/// A value the type refuses, located on the type.
#[inline]
pub fn id_type_value_into<'value>(
    kind: &IdType,
    value: &'value str,
    buffer: &'value mut [u8; IDENTIFIER_VALUE_WIDTH],
) -> Result<&'value str> {
    kind.value_into(value, buffer)
}

/// `IdType::identifier_names`: every spelling an identifier name ends a
/// key with.
#[inline]
pub fn id_type_identifier_names<'name>() -> impl Iterator<Item = &'name str> + Clone {
    IdType::identifier_names()
}

/// `IdType::underlying_security`: the security type a folded name states
/// of an instrument's underlying.
#[inline]
pub fn id_type_underlying_security(folded: &str) -> Option<IdType> {
    IdType::underlying_security(folded)
}

/// `SymbolCode::identifier`: the security identifier a symbol names, as its
/// type and code.
#[inline]
pub fn symbol_code_identifier(symbol: &SymbolCode) -> Option<(IdType, &str)> {
    symbol.identifier()
}

/// `MarketDataKind::stored_side`: the side a kind's stored cross code
/// carries.
#[inline]
pub const fn market_data_kind_stored_side(kind: MarketDataKind, side: Side) -> Side {
    kind.stored_side(side)
}

/// `Instruments::as_table`: the table as it stands, shared.
#[inline]
pub fn instruments_as_table(registry: &Instruments) -> &InstrumentTable {
    registry.as_table()
}

/// `Instruments::learn_stating`: what the collection learns of an event,
/// with what only its message spells laid over its facts.
#[inline]
pub fn instruments_learn_stating<E: crate::graph::Market + Event + ?Sized>(
    instruments: &mut Instruments,
    event: &E,
    stated: &Stated<'_>,
) -> Learned {
    instruments.learn_stating(event, stated)
}

/// `instrument::spell_code`: the cross code an element's own facts spell,
/// with what its message spells beside them and the underlying's code where
/// one is known - what a FIX parse writes as `instcode` where the code is a
/// function of the message alone, and the key it mints a pair's number from.
///
/// # Errors
///
/// A code past [`MAX_CODE_WIDTH`](crate::MAX_CODE_WIDTH), named.
#[inline]
pub fn instrument_spell_code<'s, E: crate::graph::Market + ?Sized>(
    element: &E,
    body: Option<&Body>,
    underlying: Option<&str>,
    slot: &'s mut [u8; crate::MAX_CODE_WIDTH],
) -> Result<Option<(&'s str, Production)>> {
    crate::instrument::spell_code(element, body, underlying, slot)
}

/// `Instruments::fill_stating`: the collection's fill with what only the
/// element's message spells laid over its facts.
#[inline]
pub fn instruments_fill_stating<E: crate::graph::Market + Element + ?Sized>(
    instruments: &Instruments,
    element: &mut E,
    stated: &Stated<'_>,
) -> bool {
    instruments.fill_stating(element, stated)
}

/// `graph::iterator::order`: the elements' own order as a sort reads it.
#[inline]
pub fn order<E: Element>(left: &E, right: &E) -> Ordering {
    crate::graph::iterator::order(left, right)
}

/// `EventIterator::with_placing`: the walk placing every source element it
/// reads by content among what it handed over at that instant.
#[inline]
#[must_use]
pub fn event_iterator_with_placing<E: Walked, I: Iterator<Item = E>>(
    walk: EventIterator<E, I>,
    placing: bool,
) -> EventIterator<E, I> {
    walk.with_placing(placing)
}

/// `graph::market::base_crosscode`: a cross code without its stored
/// prefix.
#[inline]
pub fn base_crosscode(code: &str) -> &str {
    crate::graph::market::base_crosscode(code)
}

/// `graph::market::merge_operation_event`: the facts an operation on the
/// market takes from another statement of itself; whether any moved.
#[inline]
pub fn merge_operation_event<E: Event + Operation>(
    this: &mut E,
    other: &E,
    other_is_reference: bool,
) -> bool {
    crate::graph::market::merge_operation_event(this, other, other_is_reference)
}

/// `graph::market::restating_operation`: an operation on the market
/// restated over the live statement it follows.
#[inline]
pub fn restating_operation<E: Event + Operation>(this: E, live: &E) -> E {
    crate::graph::market::restating_operation(this, live)
}

/// `MarketData::as_event`: the value as an event, where it is one of the
/// dated leaves.
#[inline]
pub fn market_data_as_event(data: &MarketData) -> Option<&dyn Event> {
    data.as_event()
}

/// `OperationEvent::from_facts`: an operation over facts already held, not
/// yet finalized.
#[inline]
pub fn operation_event_from_facts<K: OperationKind>(
    data: OperationEventFacts,
) -> OperationEvent<K> {
    OperationEvent::<K>::from_facts(data)
}

/// `SnapshotEvent::from_facts` over the event half of `facts`: a snapshot
/// control states no operation of its own, so the operation facts drop.
#[inline]
pub fn snapshot_event_from_facts(
    facts: OperationEventFacts,
    scope: Option<SmolStr>,
) -> SnapshotEvent {
    SnapshotEvent::from_facts(facts.into_event(), scope)
}
