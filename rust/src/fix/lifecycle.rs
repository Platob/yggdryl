//! One event's life, read across the messages that told it.
//!
//! A message says what happened; it does not say which order it happened
//! to, beyond the identifiers a venue chose. An order is created under one
//! `ClOrdID`, acknowledged under an `OrderID`, replaced under a new
//! `ClOrdID` that names the old one as `OrigClOrdID`, and every consumer
//! then rebuilds the chain from those references independently - which is
//! how two systems come to disagree about how many orders there were.
//!
//! This module reads a stream once, in order, and stamps every message with
//! three identities the stream implies:
//!
//! | column | holds |
//! | --- | --- |
//! | `instuuid` | a version-8 UUID over the xxh128 digest of market, classification, ISIN - else symbol - and currency |
//! | `uuid` | a version-7 UUID ordered by the impact clock, with an xxh3 payload derived from what the message said |
//! | `puuid` | the chain's UUID; generated values are version 7 over its first clock, instrument scope and identifier |
//!
//! # The chain is the identifiers, joined
//!
//! A stated live `puuid` joins directly. Otherwise identifiers from stated
//! `altids`, or the registered component's compiled declaration, are looked
//! up under the effective `instuuid`. Canonical member-name order decides
//! which matching chain wins. Unowned identifiers attach to it; identifiers
//! belonging to another live chain are never stolen. A foreign stated
//! `puuid` is corrected when these identifiers already name a chain.
//! An absent instrument is its own scope, and an empty stated Map supplies
//! no identifiers. No order-only tag list or nested-group promotion applies.
//!
//! # A chain ends when its state does
//!
//! The [`State`](crate::types::State) a message reports ranks its
//! lifecycle, and a rank past the live ones - filled, done for day,
//! cancelled, rejected, expired - closes the chain: its identifiers are
//! forgotten, so a venue reusing a `ClOrdID` tomorrow opens a new chain
//! rather than joining yesterday's. What is held is therefore the orders
//! still alive, and [`FixLifecycle::alive`] says how many.
//!
//! # The impact clock
//!
//! `TransactTime(60)` is when the venue says it happened; `SendingTime(52)`
//! when the message left; the row's own `timestamp` when the capture saw
//! it. The first stated orders `uuid` and `puuid` at microsecond precision.
//! The exact instant remains the clock's fact, not a UUID accessor. These
//! deterministic hashes are not cryptographic or guaranteed collision-free.
//!
//! # Nothing here is an entry
//!
//! The three columns are stamped on the row alone. The entries are what
//! arrived, and a message re-emitted after this pass is the received line
//! byte for byte.

use std::collections::{HashMap, hash_map::Entry};
use std::sync::Arc;

use smol_str::SmolStr;

use crate::path::{Path, Segment};
use crate::txhash::unix_from_scalar;
use crate::types::{Code, State, Uuid};
use crate::xxhash::{Xxh3, Xxh128, xxh3};
use crate::{DataType, Error, Result, Scalar, TimeUnit};

use super::msg::FixMsg;
use super::registry::FixRegistry;
use super::{
    ALTIDS_TAG_NAME, INSTUUID_TAG_NAME, ISINCODE_TAG_NAME, MICCODE_TAG_NAME, PUUID_TAG_NAME,
    STATE_TAG_NAME, UUID_TAG_NAME,
};

/// The clocks closest to the market impact, most exact first.
const IMPACT_TAGS: [i32; 2] = [60, 52];

/// The separator between the parts an instrument's identity digests.
///
/// A unit separator rather than nothing, so `XNAS` beside `ESVUFR` and
/// `XNASE` beside `SVUFR` are two identities.
const PART_SEPARATOR: u8 = 0x1F;

/// The state a stream of messages has reached, one chain per live event.
///
/// Built once per stream and fed every message in order through
/// [`fill`](Self::fill); [`FixCodec::lifecycle`](super::FixCodec::lifecycle)
/// does exactly that over an iterator, and the batch reader does it when
/// asked.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl::{FixCodec, FixLifecycle, FixRegistry, Scalar, ALTIDS_TAG_NAME, UUID_TAG_NAME, PUUID_TAG_NAME};
///
/// # fn main() -> yggdryl::Result<()> {
/// let registry = Arc::new(FixRegistry::new());
/// let reader = FixCodec::new(Arc::clone(&registry));
/// let mut life = FixLifecycle::new(registry);
///
/// // Explicit altids work even without registered message definitions.
/// // The order, then its acknowledgement under the venue's own identifier.
/// let order = life.fill(
///     reader
///         .parse_line(
///             b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|",
///         )?
///         .next()
///         .expect("one message")?
///         .with_value(ALTIDS_TAG_NAME.0, Scalar::from_mapping([
///             (Scalar::from("clordid"), Scalar::from("A1")),
///         ])?)?,
/// )?;
/// let ack = life.fill(
///     reader
///         .parse_line(
///             b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.250|10=0|",
///         )?
///         .next()
///         .expect("one message")?
///         .with_value(ALTIDS_TAG_NAME.0, Scalar::from_mapping([
///             (Scalar::from("clordid"), Scalar::from("A1")),
///             (Scalar::from("orderid"), Scalar::from("O1")),
///         ])?)?,
/// )?;
/// // One chain, with two UUIDs ordered by the impact clock.
/// assert_eq!(order.by_tag(PUUID_TAG_NAME.0)?, ack.by_tag(PUUID_TAG_NAME.0)?);
/// assert!(order.by_tag(UUID_TAG_NAME.0)? < ack.by_tag(UUID_TAG_NAME.0)?);
/// assert_eq!(life.alive(), 1);
///
/// // The fill closes the chain, and the venue's identifier is forgotten.
/// life.fill(
///     reader
///         .parse_line(
///             b"8=FIX.4.4|35=8|37=O1|150=F|39=2|55=AAPL|14=100|151=0|60=20260102-10:15:31.000|10=0|",
///         )?
///         .next()
///         .expect("one message")?
///         .with_value(ALTIDS_TAG_NAME.0, Scalar::from_mapping([
///             (Scalar::from("orderid"), Scalar::from("O1")),
///         ])?)?,
/// )?;
/// assert_eq!(life.alive(), 0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct FixLifecycle {
    /// The owner of compiled message identifier selection.
    registry: Arc<FixRegistry>,
    /// Whether the registry holds the three columns, decided once: a stamp
    /// lands through the message's own writer, which resolves the field
    /// again, and this only says whether there is one to resolve.
    stamps: bool,
    /// Live chains only; capacity is reused after a wider live set closes.
    chains: HashMap<Uuid, Chain>,
    /// Which chain each live identifier belongs to.
    keys: HashMap<(Option<Uuid>, SmolStr), Uuid>,
}

/// One event still alive.
#[derive(Debug)]
struct Chain {
    /// Every attached scope is forgotten together when this chain closes.
    keys: Vec<(Option<Uuid>, SmolStr)>,
}

impl FixLifecycle {
    /// A stream with no event alive yet.
    ///
    /// The three columns are the registry's own crate fields, so a registry
    /// without them - which none built by this crate is - stamps nothing.
    #[must_use]
    pub fn new(registry: Arc<FixRegistry>) -> Self {
        let stamps = [INSTUUID_TAG_NAME.0, UUID_TAG_NAME.0, PUUID_TAG_NAME.0]
            .into_iter()
            .all(|tag| registry.get_field_by_tag(tag).is_some());
        Self {
            registry,
            stamps,
            chains: HashMap::new(),
            keys: HashMap::new(),
        }
    }

    /// How many events are alive: opened by a message and not yet closed by
    /// a terminal state.
    #[must_use]
    pub fn alive(&self) -> usize {
        self.chains.len()
    }

    /// Forgets every chain, as a new session or a new day would.
    pub fn clear(&mut self) {
        self.chains.clear();
        self.keys.clear();
    }

    /// Stamps one message with its three identities and moves the chain it
    /// belongs to along.
    ///
    /// A live stated `puuid` joins directly. Otherwise the first identifier
    /// reaching a live chain under the effective instrument wins, correcting
    /// even a foreign stated `puuid`. Occupied identifiers are never stolen
    /// from another chain. Stated `uuid` and `instuuid` remain untouched.
    /// Replay rebuilds the same live state from the carried identities.
    /// Entries are untouched; all stamps land through one [`FixMsg::set_many`].
    ///
    /// # Errors
    ///
    /// Returns a located refusal for an impact instant outside the version-7
    /// range, malformed stated UUID/identifier values, a generated UUID
    /// collision, or a value the message's field refuses. No chain or
    /// identifier changes on failure.
    pub fn fill(&mut self, mut message: FixMsg) -> Result<FixMsg> {
        if !self.stamps {
            return Ok(message);
        }
        let stated_instrument = stated_uuid(&message, INSTUUID_TAG_NAME)?;
        let stated_id = stated_uuid(&message, UUID_TAG_NAME)?;
        let stated_persistent = stated_uuid(&message, PUUID_TAG_NAME)?;
        let instrument =
            stated_instrument.or_else(|| instrument_digest(&message).map(Uuid::from_v8));
        let keys = self.chain_keys(&message, instrument)?;
        let existing = stated_persistent
            .filter(|held| self.chains.contains_key(held))
            .or_else(|| keys.iter().find_map(|key| self.keys.get(key).copied()));
        let impact = if stated_id.is_none()
            || (existing.is_none() && stated_persistent.is_none() && !keys.is_empty())
        {
            impact_unix(&message)
        } else {
            0
        };
        let time_uuid = |payload, name| {
            Uuid::from_v7(impact, payload).map_err(|error| match error {
                Error::InvalidRecord { reason, .. } => Error::InvalidRecord {
                    path: Path::root().field(name).render().into(),
                    reason,
                },
                other => other,
            })
        };
        let uuid = if stated_id.is_some() {
            None
        } else {
            Some(time_uuid(
                xxh3(&message.digest().to_be_bytes()),
                UUID_TAG_NAME.1,
            )?)
        };
        let persistent = if let Some(held) = existing.or(stated_persistent) {
            Some(held)
        } else {
            keys.first()
                .map(|(_, first)| time_uuid(persistent_digest(instrument, first), PUUID_TAG_NAME.1))
                .transpose()?
        };
        if existing.is_none()
            && stated_persistent.is_none()
            && persistent.is_some_and(|held| self.chains.contains_key(&held))
        {
            return Err(Error::InvalidRecord {
                path: Path::root().field(PUUID_TAG_NAME.1).render().into(),
                reason: "expected a new chain UUID, got a generated UUID owned by an unrelated live chain".into(),
            });
        }
        message.set_many_with(
            [
                instrument
                    .filter(|_| stated_instrument.is_none())
                    .map(|held| (INSTUUID_TAG_NAME.0, Scalar::Uuid(held))),
                uuid.map(|held| (UUID_TAG_NAME.0, Scalar::Uuid(held))),
                persistent
                    .filter(|held| Some(*held) != stated_persistent)
                    .map(|held| (PUUID_TAG_NAME.0, Scalar::Uuid(held))),
            ]
            .into_iter()
            .flatten(),
            |field| {
                if matches!(field.dtype(), DataType::Uuid) {
                    Ok(())
                } else {
                    Err(Error::InvalidRecord {
                        path: Path::root().field(field.name()).render().into(),
                        reason: crate::text::expected_got(
                            DataType::Uuid,
                            crate::text::elide_display(field.dtype()),
                        ),
                    })
                }
            },
        )?;
        // The message's value contract is the last fallible step. The
        // already-resolved join publishes only after all stamps landed.
        if let Some(persistent) = persistent {
            if is_terminal(&message) {
                self.close(persistent);
            } else {
                self.join(persistent, keys);
            }
        }
        Ok(message)
    }

    /// Attach only unowned keys, retaining the incoming buffer for a new chain.
    fn join(&mut self, persistent: Uuid, mut keys: Vec<(Option<Uuid>, SmolStr)>) {
        keys.retain(|key| match self.keys.entry(key.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(persistent);
                true
            }
            Entry::Occupied(_) => false,
        });
        match self.chains.entry(persistent) {
            Entry::Vacant(entry) => {
                // Duplicate occurrences cannot leave a large backing buffer
                // on a chain holding only a few distinct identifiers.
                keys.shrink_to_fit();
                entry.insert(Chain { keys });
            }
            Entry::Occupied(mut entry) => entry.get_mut().keys.extend(keys),
        }
    }

    /// Forgets one chain and every identifier that reached it.
    fn close(&mut self, persistent: Uuid) {
        if let Some(chain) = self.chains.remove(&persistent) {
            for key in chain.keys {
                self.keys.remove(&key);
            }
        }
    }

    /// Resolve the stated map, or the same compiled projection enrichment uses.
    fn chain_keys(
        &self,
        message: &FixMsg,
        instrument: Option<Uuid>,
    ) -> Result<Vec<(Option<Uuid>, SmolStr)>> {
        let derived;
        let held = if let Some(held) = message
            .get_by_tag(ALTIDS_TAG_NAME.0)
            .filter(|held| !held.is_null())
        {
            held
        } else {
            let code = message
                .get_by_tag(35)
                .and_then(Scalar::as_str)
                .unwrap_or_default();
            let Some(component) = self.registry.get_msgtype(code) else {
                return Ok(Vec::new());
            };
            derived = component.identifier_mapping(message)?;
            &derived
        };
        let root = Path::root();
        let path = root.field(ALTIDS_TAG_NAME.1);
        let entries = held.as_mapping().ok_or_else(|| Error::InvalidRecord {
            path: path.render().into(),
            reason: crate::text::expected_got(
                "a sorted identifier Map",
                crate::text::elide_display(&format_args!("{held:?}")),
            ),
        })?;
        let mut keys: Vec<(Option<Uuid>, SmolStr)> = Vec::with_capacity(entries.len());
        let mut previous = None;
        for (index, (name, value)) in entries.iter().enumerate() {
            let Scalar::String(name) = name else {
                return Err(Error::InvalidRecord {
                    path: path.child(Segment::MapKey(index)).render().into(),
                    reason: crate::text::expected_got(
                        "a UTF-8 identifier name",
                        crate::text::elide_display(&format_args!("{name:?}")),
                    ),
                });
            };
            let name = name.as_str();
            if previous.is_some_and(|previous| previous >= name) {
                return Err(Error::InvalidRecord {
                    path: path.child(Segment::MapKey(index)).render().into(),
                    reason: crate::text::expected_got(
                        "unique identifier names in ascending order",
                        crate::text::elide_display(&name),
                    ),
                });
            }
            previous = Some(name);
            if value.is_null() {
                continue;
            }
            let Scalar::String(value) = value else {
                return Err(Error::InvalidRecord {
                    path: path.child(Segment::MapValue(index)).render().into(),
                    reason: crate::text::expected_got(
                        "a UTF-8 identifier or null",
                        crate::text::elide_display(&format_args!("{value:?}")),
                    ),
                });
            };
            // The index admits each key once when the join publishes; no
            // quadratic duplicate scan is needed while reading occurrences.
            if !value.as_str().is_empty() {
                keys.push((instrument, value.storage().clone()));
            }
        }
        Ok(keys)
    }
}

/// Native UUIDs are already packed; this adds no version/variant policy.
fn stated_uuid(message: &FixMsg, (tag, name): (i32, &str)) -> Result<Option<Uuid>> {
    match message.get_by_tag(tag).filter(|held| !held.is_null()) {
        None => Ok(None),
        Some(Scalar::Uuid(held)) => Ok(Some(*held)),
        Some(held) => Err(Error::InvalidRecord {
            path: Path::root().field(name).render().into(),
            reason: crate::text::expected_got(
                "a native UUID or null",
                crate::text::elide_display(&format_args!("{held:?}")),
            ),
        }),
    }
}

/// The instant closest to the market impact, in microseconds since the epoch.
///
/// The venue's transaction time, else the sending time, else the clock the
/// row is dated by - which every built message carries - else the epoch.
fn impact_unix(message: &FixMsg) -> i64 {
    let micros = |held: &Scalar| {
        (!held.is_null())
            .then(|| unix_from_scalar(held, TimeUnit::Microsecond).ok())
            .flatten()
    };
    IMPACT_TAGS
        .into_iter()
        .filter_map(|tag| message.get_by_tag(tag))
        .find_map(micros)
        .or_else(|| micros(&message.market_timestamp()))
        .unwrap_or(0)
}

/// The instrument's identity: the xxh128 digest of its market, its
/// classification, its ISIN - else its symbol - and its currency, or nothing
/// where the message names none of them.
fn instrument_digest(message: &FixMsg) -> Option<u128> {
    let first = |tags: &[i32]| -> Option<String> {
        tags.iter().find_map(|tag| {
            message
                .get_by_tag(*tag)
                .filter(|held| !held.is_null())
                .and_then(Scalar::as_str)
                .map(str::trim)
                .filter(|held| !held.is_empty())
                .map(str::to_ascii_uppercase)
        })
    };
    let market = first(&[MICCODE_TAG_NAME.0, 207, 100, 30]);
    let classification = first(&[461]);
    let isin = first(&[ISINCODE_TAG_NAME.0]).or_else(|| {
        (message.get_by_tag(22).and_then(Scalar::as_str) == Some("4"))
            .then(|| first(&[48]))
            .flatten()
    });
    let instrument = isin.or_else(|| first(&[55]));
    let currency = first(&[15]);
    if [&market, &classification, &instrument, &currency]
        .iter()
        .all(|part| part.is_none())
    {
        return None;
    }
    let mut digest = Xxh128::new();
    for part in [market, classification, instrument, currency] {
        if let Some(held) = part {
            digest.write_bytes(held.as_bytes());
        }
        digest.write_bytes(&[PART_SEPARATOR]);
    }
    Some(digest.as_u128())
}

/// The chain's payload: the effective scope, separator and first identifier.
fn persistent_digest(instrument: Option<Uuid>, first: &str) -> u64 {
    let mut digest = Xxh3::new();
    digest.write_bytes(&[u8::from(instrument.is_some())]);
    if let Some(held) = instrument {
        digest.write_bytes(&held.into_bytes());
    }
    digest.write_bytes(&[PART_SEPARATOR]);
    digest.write_bytes(first.as_bytes());
    digest.as_u64()
}

/// Whether the state the message reports ends the order's life.
///
/// The crate's own `state`, else `OrdStatus`, else `ExecType`, read as the
/// one lifecycle vocabulary; a message stating none is not an ending.
fn is_terminal(message: &FixMsg) -> bool {
    [STATE_TAG_NAME.0, 39, 150]
        .into_iter()
        .filter_map(|tag| message.get_by_tag(tag))
        .find(|held| !held.is_null())
        .and_then(|held| match held {
            Scalar::Code(Code::State(state)) => Some(state.clone()),
            other => other.as_str().and_then(State::from_spelling),
        })
        .is_some_and(|state| !state.is_live())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stated_identifier_keys_share_long_strings() {
        let registry = Arc::new(FixRegistry::new());
        let field = registry
            .get_group_by_counter(ALTIDS_TAG_NAME.0)
            .unwrap()
            .clone();
        let identifier = Scalar::from("identifier".repeat(128));
        let message = FixMsg::with_registry(
            Arc::clone(&registry),
            DataType::from_fields([field])
                .unwrap()
                .required_field("event"),
            Scalar::from_sequence([Scalar::from_mapping([(
                Scalar::from("id"),
                identifier.clone(),
            )])
            .unwrap()]),
        )
        .unwrap();
        let life = FixLifecycle::new(registry);
        let keys = life.chain_keys(&message, None).unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(
            keys[0].1.as_str().as_ptr(),
            identifier.as_str().unwrap().as_ptr()
        );
    }

    #[test]
    fn duplicate_occurrences_do_not_enlarge_a_live_chains_key_buffer() {
        for count in [1, 64, 1024] {
            let mut life = FixLifecycle::new(Arc::new(FixRegistry::new()));
            let persistent = Uuid::new(1);
            life.join(persistent, vec![(None, SmolStr::new("same")); count]);
            let chain = &life.chains[&persistent];
            assert_eq!(chain.keys.len(), 1);
            assert_eq!(chain.keys.capacity(), 1);
            assert_eq!(life.keys.len(), 1);
            life.close(persistent);
            assert!(life.chains.is_empty());
            assert!(life.keys.is_empty());
        }
    }
}
