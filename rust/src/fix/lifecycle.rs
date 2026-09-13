//! One order's life, read across the messages that told it.
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
//! | `puuid` | the chain's version-7 UUID, dated by its first message and derived from its instrument and first identifier |
//!
//! # The chain is the identifiers, joined
//!
//! A message joins a chain through any identifier it carries - `OrigClOrdID`
//! first, because a replace names the order it replaces, then `ClOrdID`,
//! `OrderID`, `SecondaryClOrdID` and `SecondaryOrderID` - and every
//! identifier it carries joins the chain in turn, so a replace's new
//! `ClOrdID` reaches the chain the old one opened. A message carrying an
//! identifier no chain holds opens one, dated by its own impact clock.
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

use std::collections::HashMap;
use std::sync::Arc;

use smol_str::SmolStr;

use crate::path::Path;
use crate::txhash::unix_from_scalar;
use crate::types::{Code, State, Uuid};
use crate::xxhash::{Xxh3, Xxh128, xxh3};
use crate::{Error, Result, Scalar, TimeUnit};

use super::msg::FixMsg;
use super::registry::FixRegistry;
use super::{
    INSTUUID_TAG_NAME, ISINCODE_TAG_NAME, MICCODE_TAG_NAME, PUUID_TAG_NAME, STATE_TAG_NAME,
    UUID_TAG_NAME,
};

/// The tags an order is known by, in the order a message is joined on them.
///
/// `OrigClOrdID` first: a replace or a cancel names the order it acts on
/// there, and the new `ClOrdID` it carries is not yet anyone's.
const CHAIN_TAGS: [i32; 5] = [41, 11, 37, 526, 198];

/// The clocks closest to the market impact, most exact first.
const IMPACT_TAGS: [i32; 2] = [60, 52];

/// The separator between the parts an instrument's identity digests.
///
/// A unit separator rather than nothing, so `XNAS` beside `ESVUFR` and
/// `XNASE` beside `SVUFR` are two identities.
const PART_SEPARATOR: u8 = 0x1F;

/// The state a stream of messages has reached, one chain per order alive.
///
/// Built once per stream and fed every message in order through
/// [`fill`](Self::fill); [`FixCodec::lifecycle`](super::FixCodec::lifecycle)
/// does exactly that over an iterator, and the batch reader does it when
/// asked.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl::{FixCodec, FixLifecycle, FixRegistry, UUID_TAG_NAME, PUUID_TAG_NAME};
///
/// # fn main() -> yggdryl::Result<()> {
/// let registry = Arc::new(FixRegistry::new());
/// let reader = FixCodec::new(Arc::clone(&registry));
/// let mut life = FixLifecycle::new(registry);
///
/// // The order, then its acknowledgement under the venue's own identifier.
/// let order = life.fill(
///     reader
///         .parse_line(
///             b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|",
///         )?
///         .next()
///         .expect("one message")?,
/// )?;
/// let ack = life.fill(
///     reader
///         .parse_line(
///             b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.250|10=0|",
///         )?
///         .next()
///         .expect("one message")?,
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
///             b"8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|60=20260102-10:15:31.000|10=0|",
///         )?
///         .next()
///         .expect("one message")?,
/// )?;
/// assert_eq!(life.alive(), 0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct FixLifecycle {
    /// Whether the registry holds the three columns, decided once: a stamp
    /// lands through the message's own writer, which resolves the field
    /// again, and this only says whether there is one to resolve.
    stamps: bool,
    /// Every chain ever opened, by index; a closed one is a hole reused.
    chains: Vec<Option<Chain>>,
    /// Which chain each live identifier belongs to.
    keys: HashMap<SmolStr, usize>,
    /// The holes a closed chain left, so the vector stays the size of the
    /// widest moment rather than the whole capture.
    free: Vec<usize>,
}

/// One order still alive.
#[derive(Debug)]
struct Chain {
    /// The identity every message of the chain carries.
    persistent: Uuid,
    /// The identifiers that reach it, forgotten together when it closes.
    keys: Vec<SmolStr>,
}

impl FixLifecycle {
    /// A stream with no order alive yet.
    ///
    /// The three columns are the registry's own crate fields, so a registry
    /// without them - which none built by this crate is - stamps nothing.
    #[must_use]
    pub fn new(registry: Arc<FixRegistry>) -> Self {
        let stamps = [INSTUUID_TAG_NAME.0, UUID_TAG_NAME.0, PUUID_TAG_NAME.0]
            .into_iter()
            .all(|tag| registry.get_field_by_tag(tag).is_some());
        Self {
            stamps,
            chains: Vec::new(),
            keys: HashMap::new(),
            free: Vec::new(),
        }
    }

    /// How many orders are alive: opened by a message and not yet closed by
    /// a terminal state.
    #[must_use]
    pub fn alive(&self) -> usize {
        self.chains.len() - self.free.len()
    }

    /// Forgets every chain, as a new session or a new day would.
    pub fn clear(&mut self) {
        self.chains.clear();
        self.keys.clear();
        self.free.clear();
    }

    /// Stamps one message with its three identities and moves the chain it
    /// belongs to along.
    ///
    /// A stated value is never overwritten: a message already carrying an
    /// `uuid` keeps it, and one carrying a `puuid` joins nothing new,
    /// which is what makes a second pass over a stamped stream a no-op. The
    /// entries are untouched: the stamps land through [`FixMsg::set_many`],
    /// each typed by the dictionary's own column.
    ///
    /// # Errors
    ///
    /// Returns a located refusal for an impact instant outside the version-7
    /// range or a value the message's field refuses. No chain or identifier
    /// changes on failure.
    pub fn fill(&mut self, mut message: FixMsg) -> Result<FixMsg> {
        if !self.stamps {
            return Ok(message);
        }
        let stated = |tag: i32| message.get_by_tag(tag).is_some_and(|held| !held.is_null());
        let impact = impact_unix(&message);
        let keys = chain_keys(&message);
        let has_instrument = stated(INSTUUID_TAG_NAME.0);
        let has_uuid = stated(UUID_TAG_NAME.0);
        let has_persistent = stated(PUUID_TAG_NAME.0);
        let existing = if has_persistent {
            None
        } else {
            keys.iter().find_map(|key| self.keys.get(key).copied())
        };
        let needs_instrument =
            !has_instrument || (!has_persistent && existing.is_none() && !keys.is_empty());
        let instrument = if needs_instrument {
            instrument_digest(&message)
        } else {
            None
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
        let uuid = if has_uuid {
            None
        } else {
            Some(time_uuid(
                xxh3(&message.digest().to_be_bytes()),
                UUID_TAG_NAME.1,
            )?)
        };
        let persistent = if has_persistent {
            None
        } else if let Some(at) = existing {
            self.chains[at].as_ref().map(|held| held.persistent)
        } else {
            keys.first()
                .map(|first| time_uuid(persistent_digest(instrument, first), PUUID_TAG_NAME.1))
                .transpose()?
        };
        let instuuid = instrument.filter(|_| !has_instrument).map(Uuid::from_v8);
        message.set_many(
            [
                instuuid.map(|held| (INSTUUID_TAG_NAME.0, Scalar::Uuid(held))),
                uuid.map(|held| (UUID_TAG_NAME.0, Scalar::Uuid(held))),
                persistent.map(|held| (PUUID_TAG_NAME.0, Scalar::Uuid(held))),
            ]
            .into_iter()
            .flatten(),
        )?;
        // The message's value contract is the last fallible step. The
        // already-resolved join publishes only after all stamps landed.
        let chain = persistent.map(|held| self.join(existing, &keys, held));
        if is_terminal(&message) {
            // A stamped stream read again closes the chain its identifiers
            // reach, exactly as the first pass did.
            let closing = chain.or_else(|| keys.iter().find_map(|key| self.keys.get(key).copied()));
            if let Some(at) = closing {
                self.close(at);
            }
        }
        Ok(message)
    }

    /// The chain the identifiers reach, opened where none does.
    ///
    /// Every identifier the message carries then reaches the chain, so an
    /// identifier introduced by a replace joins the order it replaces.
    fn join(&mut self, existing: Option<usize>, keys: &[SmolStr], persistent: Uuid) -> usize {
        let at = match existing {
            Some(at) => at,
            None => {
                let chain = Chain {
                    persistent,
                    keys: Vec::with_capacity(keys.len()),
                };
                match self.free.pop() {
                    Some(hole) => {
                        self.chains[hole] = Some(chain);
                        hole
                    }
                    None => {
                        self.chains.push(Some(chain));
                        self.chains.len() - 1
                    }
                }
            }
        };
        if let Some(chain) = self.chains[at].as_mut() {
            for key in keys {
                if !self.keys.contains_key(key) {
                    self.keys.insert(key.clone(), at);
                    chain.keys.push(key.clone());
                }
            }
        }
        at
    }

    /// Forgets one chain and every identifier that reached it.
    fn close(&mut self, at: usize) {
        if let Some(chain) = self.chains[at].take() {
            for key in chain.keys {
                self.keys.remove(&key);
            }
            self.free.push(at);
        }
    }
}

/// The identifiers a message carries, in the order a chain is joined on.
fn chain_keys(message: &FixMsg) -> Vec<SmolStr> {
    let mut keys: Vec<SmolStr> = Vec::with_capacity(CHAIN_TAGS.len());
    for tag in CHAIN_TAGS {
        let Some(text) = message.get_by_tag(tag).and_then(Scalar::as_str) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        // Two tags spelling one identifier - an order acknowledged under the
        // client's own - is one key, not a chain joined to itself.
        if !keys.iter().any(|held| held == text) {
            keys.push(SmolStr::new(text));
        }
    }
    keys
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

/// The chain's payload: the raw instrument digest, separator and first key.
fn persistent_digest(instrument: Option<u128>, first: &str) -> u64 {
    let mut digest = Xxh3::new();
    if let Some(held) = instrument {
        digest.write_bytes(&held.to_be_bytes());
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
